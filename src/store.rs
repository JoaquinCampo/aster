use crate::domain::{AuditEvent, Task};
use anyhow::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, body TEXT NOT NULL); CREATE TABLE IF NOT EXISTS audit(seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL, task_id TEXT NOT NULL, body TEXT NOT NULL);")?;
        Ok(Self { conn })
    }
    pub fn save_task(&self, task: &Task) -> Result<()> {
        self.conn.execute("INSERT INTO tasks(id, body) VALUES(?1, ?2) ON CONFLICT(id) DO UPDATE SET body=excluded.body", params![task.id.to_string(), serde_json::to_string(task)?])?;
        Ok(())
    }
    pub fn tasks(&self) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare("SELECT body FROM tasks ORDER BY rowid")?;
        Ok(stmt
            .query_map([], |r| {
                let body: String = r.get(0)?;
                Ok(body)
            })?
            .filter_map(Result::ok)
            .filter_map(|v| serde_json::from_str(&v).ok())
            .collect())
    }
    pub fn append(&self, event: &AuditEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit(id, task_id, body) VALUES(?1, ?2, ?3)",
            params![
                event.id.to_string(),
                event.task_id.to_string(),
                serde_json::to_string(event)?
            ],
        )?;
        Ok(())
    }
    pub fn audit_for(&self, task_id: Uuid) -> Result<Vec<AuditEvent>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body FROM audit WHERE task_id=?1 ORDER BY seq")?;
        Ok(stmt
            .query_map([task_id.to_string()], |r| {
                let body: String = r.get(0)?;
                Ok(body)
            })?
            .filter_map(Result::ok)
            .filter_map(|v| serde_json::from_str(&v).ok())
            .collect())
    }
}
