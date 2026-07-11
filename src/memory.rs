use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryScope {
    Turn,
    Task,
    Session,
    UserPreference,
    ProjectKnowledge,
    ArchitectureDecision,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub scope: MemoryScope,
    pub key: String,
    pub value: String,
    pub provenance: String,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
pub struct MemoryStore {
    conn: Connection,
}
impl MemoryStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS memories(id TEXT PRIMARY KEY,scope TEXT NOT NULL,key TEXT NOT NULL,value TEXT,provenance TEXT NOT NULL,digest TEXT NOT NULL,created TEXT NOT NULL,deleted TEXT); CREATE UNIQUE INDEX IF NOT EXISTS memory_dedup ON memories(scope,digest) WHERE deleted IS NULL; CREATE TABLE IF NOT EXISTS memory_tombstones(id TEXT PRIMARY KEY,scope TEXT NOT NULL,digest TEXT NOT NULL,deleted TEXT NOT NULL);")?;
        Ok(Self { conn })
    }
    pub fn add(
        &self,
        scope: MemoryScope,
        key: &str,
        value: &str,
        provenance: &str,
    ) -> Result<Uuid> {
        let digest = digest(value);
        let existing = self.conn.query_row(
            "SELECT id FROM memories WHERE scope=?1 AND digest=?2 AND deleted IS NULL",
            params![scope_s(&scope), digest],
            |r| r.get::<_, String>(0),
        );
        if let Ok(id) = existing {
            return Ok(Uuid::parse_str(&id)?);
        };
        let id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO memories VALUES(?1,?2,?3,?4,?5,?6,?7,NULL)",
            params![
                id.to_string(),
                scope_s(&scope),
                key,
                value,
                provenance,
                digest,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(id)
    }
    pub fn active(&self) -> Result<Vec<Memory>> {
        let mut s=self.conn.prepare("SELECT id,scope,key,value,provenance,created,deleted FROM memories WHERE deleted IS NULL ORDER BY rowid")?;
        let rows = s.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = vec![];
        for r in rows {
            let (id, scope, key, value, provenance, created) = r?;
            out.push(Memory {
                id: Uuid::parse_str(&id)?,
                scope: parse_scope(&scope)?,
                key,
                value,
                provenance,
                created_at: DateTime::parse_from_rfc3339(&created)?.with_timezone(&Utc),
                deleted_at: None,
            })
        }
        Ok(out)
    }
    pub fn contradictions(
        &self,
        scope: &MemoryScope,
        key: &str,
        value: &str,
    ) -> Result<Vec<Memory>> {
        Ok(self
            .active()?
            .into_iter()
            .filter(|m| &m.scope == scope && m.key == key && m.value != value)
            .collect())
    }
    pub fn amend(&self, id: Uuid, value: &str, provenance: &str) -> Result<Uuid> {
        let old = self
            .active()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| anyhow::anyhow!("memory not found"))?;
        self.delete(id)?;
        self.add(old.scope, &old.key, value, provenance)
    }
    pub fn delete(&self, id: Uuid) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let row = tx.query_row(
            "SELECT scope,digest FROM memories WHERE id=?1 AND deleted IS NULL",
            [id.to_string()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO memory_tombstones VALUES(?1,?2,?3,?4)",
            params![id.to_string(), row.0, row.1, now],
        )?;
        tx.execute(
            "UPDATE memories SET value=NULL,deleted=?2 WHERE id=?1",
            params![id.to_string(), now],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn tombstone_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM memory_tombstones", [], |r| r.get(0))?)
    }
}
fn digest(v: &str) -> String {
    format!("{:x}", Sha256::digest(v.trim().to_lowercase()))
}
fn scope_s(s: &MemoryScope) -> String {
    serde_json::to_string(s).unwrap()
}
fn parse_scope(s: &str) -> Result<MemoryScope> {
    serde_json::from_str(s).map_err(Into::into)
}
