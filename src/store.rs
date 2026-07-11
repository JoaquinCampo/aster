use crate::{
    domain::{Artifact, AuditEvent, Checkpoint, Operation, OperationState, Task, TaskState},
    effects::OperationAuthorization,
};
use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
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
          CREATE TABLE IF NOT EXISTS checkpoints(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, attempt INTEGER NOT NULL, operation_id TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS checkpoints_owner ON checkpoints(task_id,attempt,operation_id);
          CREATE TABLE IF NOT EXISTS artifacts(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, attempt INTEGER NOT NULL, operation_id TEXT NOT NULL, digest TEXT NOT NULL, body TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS artifacts_owner ON artifacts(task_id,attempt,operation_id);
          CREATE TABLE IF NOT EXISTS effect_authorizations(id TEXT PRIMARY KEY, operation_id TEXT NOT NULL, task_id TEXT NOT NULL, body TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS routing_outcomes(policy_revision INTEGER NOT NULL, role TEXT NOT NULL, model TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(policy_revision,role,model));
          CREATE TABLE IF NOT EXISTS routing_recommendations(id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL);
          INSERT OR IGNORE INTO schema_migrations VALUES(1, datetime('now'));
          INSERT OR IGNORE INTO schema_migrations VALUES(2, datetime('now'));
          INSERT OR IGNORE INTO schema_migrations VALUES(3, datetime('now'));" )?;
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
    fn validate_owner(&self, task_id: Uuid, attempt: u32, operation_id: Uuid) -> Result<()> {
        let owner: Option<(String, u32)> = self
            .conn
            .query_row(
                "SELECT task_id,attempt FROM operations WHERE id=?1",
                [operation_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if owner != Some((task_id.to_string(), attempt)) {
            bail!("operation ownership mismatch")
        }
        Ok(())
    }
    pub fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()> {
        self.validate_owner(
            checkpoint.task_id,
            checkpoint.attempt,
            checkpoint.operation_id,
        )?;
        let digest = format!("sha256:{:x}", Sha256::digest(checkpoint.payload.as_bytes()));
        if checkpoint.digest != digest {
            bail!("checkpoint digest mismatch")
        }
        self.conn.execute("INSERT INTO checkpoints(id,task_id,attempt,operation_id,phase,body) VALUES(?1,?2,?3,?4,?5,?6)", params![checkpoint.id.to_string(), checkpoint.task_id.to_string(), checkpoint.attempt, checkpoint.operation_id.to_string(), checkpoint.phase, serde_json::to_string(checkpoint)?])?;
        Ok(())
    }
    pub fn checkpoints_for(&self, task_id: Uuid) -> Result<Vec<Checkpoint>> {
        let mut statement = self
            .conn
            .prepare("SELECT body FROM checkpoints WHERE task_id=?1 ORDER BY attempt,rowid")?;
        let bodies = statement
            .query_map([task_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bodies
            .into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
            .collect()
    }
    pub fn save_artifact(&self, artifact: &Artifact) -> Result<()> {
        self.validate_owner(artifact.task_id, artifact.attempt, artifact.operation_id)?;
        let digest = format!("sha256:{:x}", Sha256::digest(&artifact.content));
        if artifact.digest != digest {
            bail!("artifact digest mismatch")
        }
        self.conn.execute("INSERT INTO artifacts(id,task_id,attempt,operation_id,digest,body) VALUES(?1,?2,?3,?4,?5,?6)", params![artifact.id.to_string(), artifact.task_id.to_string(), artifact.attempt, artifact.operation_id.to_string(), artifact.digest, serde_json::to_string(artifact)?])?;
        Ok(())
    }
    pub fn artifacts_for(&self, task_id: Uuid) -> Result<Vec<Artifact>> {
        let mut statement = self
            .conn
            .prepare("SELECT body FROM artifacts WHERE task_id=?1 ORDER BY attempt,rowid")?;
        let bodies = statement
            .query_map([task_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bodies
            .into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
            .collect()
    }
    pub fn dependency_artifacts(&self, task: &Task) -> Result<Vec<Artifact>> {
        let mut artifacts = Vec::new();
        for dependency in &task.dependencies {
            artifacts.extend(self.artifacts_for(*dependency)?);
        }
        Ok(artifacts)
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
    pub fn save_routing_outcome(
        &self,
        outcome: &crate::routing_policy::OutcomeAggregate,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO routing_outcomes(policy_revision,role,model,body) VALUES(?1,?2,?3,?4) ON CONFLICT(policy_revision,role,model) DO UPDATE SET body=excluded.body",
            params![outcome.policy_revision, outcome.role.to_string(), outcome.model, serde_json::to_string(outcome)?])?;
        Ok(())
    }
    pub fn routing_outcomes(
        &self,
        revision: u64,
    ) -> Result<Vec<crate::routing_policy::OutcomeAggregate>> {
        let mut statement = self.conn.prepare(
            "SELECT body FROM routing_outcomes WHERE policy_revision=?1 ORDER BY role,model",
        )?;
        let bodies = statement
            .query_map([revision], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bodies
            .into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
            .collect()
    }
    pub fn save_routing_recommendation(
        &self,
        recommendation: &crate::routing_policy::PolicyRecommendation,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO routing_recommendations(body) VALUES(?1)",
            [serde_json::to_string(recommendation)?],
        )?;
        Ok(())
    }
    pub fn routing_recommendations(
        &self,
    ) -> Result<Vec<crate::routing_policy::PolicyRecommendation>> {
        let mut statement = self
            .conn
            .prepare("SELECT body FROM routing_recommendations ORDER BY id")?;
        let bodies = statement
            .query_map([], |row| row.get::<_, String>(0))?
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
    /// Irreversibly erase user-controlled task payloads while preserving only lifecycle
    /// metadata and immutable audit history. Audit events never contain the prompt,
    /// transcript, checkpoint, artifact, or secret payload itself.
    pub fn delete_task_payloads(&self, task_id: Uuid) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut task = load_tx(&tx, task_id)?;
        task.prompt.clear();
        task.output = None;
        task.verification = None;
        task.failure_reason = None;
        tx.execute(
            "UPDATE tasks SET body=?2 WHERE id=?1",
            params![task_id.to_string(), serde_json::to_string(&task)?],
        )?;
        tx.execute(
            "DELETE FROM checkpoints WHERE task_id=?1",
            [task_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM artifacts WHERE task_id=?1",
            [task_id.to_string()],
        )?;
        insert_event(
            &tx,
            task_id,
            "payload.deleted",
            "payload_ref=deleted; classes=prompt,transcript,verification,diagnostic,checkpoint,artifact",
        )?;
        tx.commit()?;
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        Ok(())
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
