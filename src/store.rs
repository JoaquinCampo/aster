use crate::domain::{AuditEvent, Operation, OperationState, Task, TaskState};
use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, body TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS audit(seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL, task_id TEXT NOT NULL, body TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, attempt INTEGER NOT NULL, body TEXT NOT NULL);
          INSERT OR IGNORE INTO schema_migrations VALUES(1, datetime('now'));
          INSERT OR IGNORE INTO schema_migrations VALUES(2, datetime('now'));" )?;
        Ok(Self { conn })
    }
    pub fn save_task(&self, task: &Task) -> Result<()> {
        self.conn.execute("INSERT INTO tasks(id,body) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET body=excluded.body", params![task.id.to_string(),serde_json::to_string(task)?])?;
        Ok(())
    }
    pub fn task(&self, id: Uuid) -> Result<Option<Task>> {
        let body: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM tasks WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(body.map(|v| serde_json::from_str(&v)).transpose()?)
    }
    pub fn tasks(&self) -> Result<Vec<Task>> {
        self.rows("SELECT body FROM tasks ORDER BY rowid", [])
    }
    fn rows<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<Vec<Task>> {
        let mut s = self.conn.prepare(sql)?;
        let bodies = s
            .query_map(params, |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bodies
            .into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }
    pub fn append(&self, e: &AuditEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit(id,task_id,body) VALUES(?1,?2,?3)",
            params![
                e.id.to_string(),
                e.task_id.to_string(),
                serde_json::to_string(e)?
            ],
        )?;
        Ok(())
    }
    pub fn audit_for(&self, id: Uuid) -> Result<Vec<AuditEvent>> {
        let mut s = self
            .conn
            .prepare("SELECT body FROM audit WHERE task_id=?1 ORDER BY seq")?;
        let b = s
            .query_map([id.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        b.into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }
    pub fn create_operation(&self, op: &Operation) -> Result<()> {
        self.conn.execute(
            "INSERT INTO operations(id,task_id,attempt,body) VALUES(?1,?2,?3,?4)",
            params![
                op.id.to_string(),
                op.task_id.to_string(),
                op.attempt,
                serde_json::to_string(op)?
            ],
        )?;
        Ok(())
    }
    pub fn save_operation(&self, op: &Operation) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET body=?2 WHERE id=?1",
            params![op.id.to_string(), serde_json::to_string(op)?],
        )?;
        Ok(())
    }
    pub fn operations_for(&self, id: Uuid) -> Result<Vec<Operation>> {
        let mut s = self
            .conn
            .prepare("SELECT body FROM operations WHERE task_id=?1 ORDER BY attempt")?;
        let b = s
            .query_map([id.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        b.into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }
    pub fn transition(
        &mut self,
        id: Uuid,
        from: &[TaskState],
        to: TaskState,
        kind: &str,
        detail: &str,
    ) -> Result<Task> {
        let tx = self.conn.transaction()?;
        let mut task = load_tx(&tx, id)?;
        if !from.contains(&task.state) {
            bail!("invalid transition {:?} -> {:?}", task.state, to)
        }
        task.state = to;
        task.updated_at = Utc::now();
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![id.to_string(), serde_json::to_string(&task)?],
        )?;
        let e = AuditEvent {
            id: Uuid::new_v4(),
            task_id: id,
            kind: kind.into(),
            detail: detail.into(),
            at: Utc::now(),
        };
        tx.execute(
            "INSERT INTO audit(id,task_id,body) VALUES(?1,?2,?3)",
            params![e.id.to_string(), id.to_string(), serde_json::to_string(&e)?],
        )?;
        tx.commit()?;
        Ok(task)
    }
    pub fn recover(&mut self) -> Result<usize> {
        let ids: Vec<Uuid> = self
            .tasks()?
            .into_iter()
            .filter(|t| {
                matches!(
                    t.state,
                    TaskState::Running | TaskState::Pausing | TaskState::Cancelling
                )
            })
            .map(|t| t.id)
            .collect();
        for id in &ids {
            let mut t = self.task(*id)?.expect("listed task");
            t.state = TaskState::OutcomeUnknown;
            t.failure_reason = Some(
                "process exited while operation was in flight; reconciliation required".into(),
            );
            t.updated_at = Utc::now();
            self.save_task(&t)?;
            for mut op in self
                .operations_for(*id)?
                .into_iter()
                .filter(|o| o.state == OperationState::Running)
            {
                op.state = OperationState::OutcomeUnknown;
                self.save_operation(&op)?;
            }
            self.append(&AuditEvent {
                id: Uuid::new_v4(),
                task_id: *id,
                kind: "recovery.outcome_unknown".into(),
                detail: "in-flight operation requires reconciliation".into(),
                at: Utc::now(),
            })?;
        }
        Ok(ids.len())
    }
}
fn load_tx(tx: &Transaction<'_>, id: Uuid) -> Result<Task> {
    let b: String = tx.query_row(
        "SELECT body FROM tasks WHERE id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&b)?)
}
