use crate::{
    domain::{AuditEvent, Operation, OperationState, Task, TaskState},
    effects::OperationAuthorization,
};
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
          CREATE TABLE IF NOT EXISTS effect_authorizations(id TEXT PRIMARY KEY, operation_id TEXT NOT NULL, task_id TEXT NOT NULL, body TEXT NOT NULL);
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
    pub fn operation(&self, id: Uuid) -> Result<Option<Operation>> {
        let body: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM operations WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(body.map(|v| serde_json::from_str(&v)).transpose()?)
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
    pub fn authorize_effect(&self, auth: &OperationAuthorization) -> Result<()> {
        self.conn.execute(
            "INSERT INTO effect_authorizations(id,operation_id,task_id,body) VALUES(?1,?2,?3,?4)",
            params![
                auth.id.to_string(),
                auth.operation_id.to_string(),
                auth.task_id.to_string(),
                serde_json::to_string(auth)?
            ],
        )?;
        Ok(())
    }
    pub fn effect_authorizations(&self, operation_id: Uuid) -> Result<Vec<OperationAuthorization>> {
        let mut statement = self.conn.prepare(
            "SELECT body FROM effect_authorizations WHERE operation_id=?1 ORDER BY rowid",
        )?;
        let bodies = statement
            .query_map([operation_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bodies
            .into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
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
        let tx = self.conn.unchecked_transaction()?;
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
    pub fn create_task(&self, task: &Task, events: &[(&str, &str)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO tasks(id,body) VALUES(?1,?2)",
            params![task.id.to_string(), serde_json::to_string(task)?],
        )?;
        for (kind, detail) in events {
            insert_event(&tx, task.id, kind, detail)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_task_with_event(&self, task: &Task, kind: &str, detail: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![task.id.to_string(), serde_json::to_string(task)?],
        )?;
        insert_event(&tx, task.id, kind, detail)?;
        tx.commit()?;
        Ok(())
    }
    pub fn start_operation(&self, task: &Task, op: &Operation) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![task.id.to_string(), serde_json::to_string(task)?],
        )?;
        tx.execute(
            "INSERT INTO operations(id,task_id,attempt,body) VALUES(?1,?2,?3,?4)",
            params![
                op.id.to_string(),
                op.task_id.to_string(),
                op.attempt,
                serde_json::to_string(op)?
            ],
        )?;
        insert_event(&tx, task.id, "operation.intent", &op.id.to_string())?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_operation(&self, task: &Task, op: &Operation) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![task.id.to_string(), serde_json::to_string(task)?],
        )?;
        tx.execute(
            "UPDATE operations SET body=?2 WHERE id=?1",
            params![op.id.to_string(), serde_json::to_string(op)?],
        )?;
        insert_event(
            &tx,
            task.id,
            "operation.completed",
            &format!("{}: {:?}", op.id, task.state),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn reconcile_operation(
        &mut self,
        task_id: Uuid,
        operation_id: Uuid,
        outcome: OperationState,
    ) -> Result<Task> {
        if !matches!(
            outcome,
            OperationState::Succeeded
                | OperationState::Failed
                | OperationState::TimedOut
                | OperationState::Cancelled
        ) {
            bail!("invalid reconciliation outcome")
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut task = load_tx(&tx, task_id)?;
        if task.state != TaskState::OutcomeUnknown {
            bail!("task is not outcome-unknown")
        }
        let body: String = tx.query_row(
            "SELECT body FROM operations WHERE id=?1 AND task_id=?2",
            params![operation_id.to_string(), task_id.to_string()],
            |r| r.get(0),
        )?;
        let mut op: Operation = serde_json::from_str(&body)?;
        if op.state != OperationState::OutcomeUnknown {
            bail!("operation is not outcome-unknown")
        }
        op.state = outcome;
        op.completed_at = Some(Utc::now());
        task.state = match outcome {
            OperationState::Succeeded => TaskState::Succeeded,
            OperationState::Failed => TaskState::Failed,
            OperationState::TimedOut => TaskState::TimedOut,
            OperationState::Cancelled => TaskState::Cancelled,
            _ => unreachable!(),
        };
        task.updated_at = Utc::now();
        tx.execute(
            "UPDATE operations SET body=?2 WHERE id=?1",
            params![operation_id.to_string(), serde_json::to_string(&op)?],
        )?;
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![task_id.to_string(), serde_json::to_string(&task)?],
        )?;
        insert_event(
            &tx,
            task_id,
            "operation.reconciled",
            &format!("operation_id={operation_id}; outcome={outcome:?}"),
        )?;
        tx.commit()?;
        Ok(task)
    }
    pub fn recover(&mut self) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let bodies = {
            let mut s = tx.prepare("SELECT body FROM tasks")?;
            s.query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut count = 0;
        for body in bodies {
            let mut task: Task = serde_json::from_str(&body)?;
            if !matches!(
                task.state,
                TaskState::Running | TaskState::Pausing | TaskState::Cancelling
            ) {
                continue;
            }
            task.state = TaskState::OutcomeUnknown;
            task.failure_reason = Some(
                "process exited while operation was in flight; reconciliation required".into(),
            );
            task.updated_at = Utc::now();
            tx.execute(
                "UPDATE tasks SET body=?2 WHERE id=?1",
                params![task.id.to_string(), serde_json::to_string(&task)?],
            )?;
            let ops = {
                let mut s = tx.prepare("SELECT id,body FROM operations WHERE task_id=?1")?;
                s.query_map([task.id.to_string()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (id, body) in ops {
                let mut op: Operation = serde_json::from_str(&body)?;
                if matches!(
                    op.state,
                    OperationState::IntentRecorded | OperationState::Running
                ) {
                    op.state = OperationState::OutcomeUnknown;
                    tx.execute(
                        "UPDATE operations SET body=?2 WHERE id=?1",
                        params![id, serde_json::to_string(&op)?],
                    )?;
                }
            }
            insert_event(
                &tx,
                task.id,
                "recovery.outcome_unknown",
                "in-flight operation requires reconciliation",
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }
}
fn insert_event(tx: &Transaction<'_>, task_id: Uuid, kind: &str, detail: &str) -> Result<()> {
    let event = AuditEvent {
        id: Uuid::new_v4(),
        task_id,
        kind: kind.into(),
        detail: detail.into(),
        at: Utc::now(),
    };
    tx.execute(
        "INSERT INTO audit(id,task_id,body) VALUES(?1,?2,?3)",
        params![
            event.id.to_string(),
            task_id.to_string(),
            serde_json::to_string(&event)?
        ],
    )?;
    Ok(())
}

fn load_tx(tx: &Transaction<'_>, id: Uuid) -> Result<Task> {
    let b: String = tx.query_row(
        "SELECT body FROM tasks WHERE id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&b)?)
}
