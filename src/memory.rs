use anyhow::{Result, bail};
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
    AuditHistory,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Memory {
    pub id: Uuid,
    pub scope: MemoryScope,
    pub key: String,
    pub value: String,
    pub provenance: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryExport {
    pub schema_version: u32,
    pub exported_at: DateTime<Utc>,
    pub memories: Vec<Memory>,
}
pub struct MemoryStore {
    conn: Connection,
    digest_key: String,
}
impl MemoryStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS memories(id TEXT PRIMARY KEY,scope TEXT NOT NULL,key TEXT NOT NULL,value TEXT,provenance TEXT NOT NULL,digest TEXT NOT NULL,created TEXT NOT NULL,deleted TEXT,expires TEXT); CREATE UNIQUE INDEX IF NOT EXISTS memory_dedup ON memories(scope,digest) WHERE deleted IS NULL; CREATE TABLE IF NOT EXISTS memory_tombstones(id TEXT PRIMARY KEY,scope TEXT NOT NULL,digest TEXT NOT NULL,deleted TEXT NOT NULL); CREATE TABLE IF NOT EXISTS memory_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN expires TEXT", []);
        let digest_key = conn
            .query_row(
                "SELECT value FROM memory_meta WHERE key='digest_key'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| {
                let k = Uuid::new_v4().to_string();
                conn.execute("INSERT INTO memory_meta VALUES('digest_key',?1)", [&k])
                    .expect("digest key");
                k
            });
        Ok(Self { conn, digest_key })
    }
    pub fn add(
        &self,
        scope: MemoryScope,
        key: &str,
        value: &str,
        provenance: &str,
    ) -> Result<Uuid> {
        self.add_expiring(scope, key, value, provenance, None)
    }
    pub fn add_expiring(
        &self,
        scope: MemoryScope,
        key: &str,
        value: &str,
        provenance: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Uuid> {
        let d = digest(&self.digest_key, value);
        if let Ok(id) = self.conn.query_row(
            "SELECT id FROM memories WHERE scope=?1 AND digest=?2 AND deleted IS NULL",
            params![scope_s(&scope), d],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(Uuid::parse_str(&id)?);
        }
        let id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO memories VALUES(?1,?2,?3,?4,?5,?6,?7,NULL,?8)",
            params![
                id.to_string(),
                scope_s(&scope),
                key,
                value,
                provenance,
                d,
                Utc::now().to_rfc3339(),
                expires_at.map(|v| v.to_rfc3339())
            ],
        )?;
        Ok(id)
    }
    pub fn active(&self) -> Result<Vec<Memory>> {
        self.query(None, None)
    }
    pub fn search(&self, query: &str, scope: Option<&MemoryScope>) -> Result<Vec<Memory>> {
        self.query(Some(query), scope)
    }
    fn query(&self, query: Option<&str>, scope: Option<&MemoryScope>) -> Result<Vec<Memory>> {
        let now = Utc::now();
        let mut s=self.conn.prepare("SELECT id,scope,key,value,provenance,created,deleted,expires FROM memories WHERE deleted IS NULL ORDER BY rowid")?;
        let rows = s.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, ss, key, value, provenance, created, expires) = row?;
            let m = Memory {
                id: Uuid::parse_str(&id)?,
                scope: parse_scope(&ss)?,
                key,
                value,
                provenance,
                created_at: DateTime::parse_from_rfc3339(&created)?.with_timezone(&Utc),
                expires_at: expires
                    .map(|x| DateTime::parse_from_rfc3339(&x).map(|d| d.with_timezone(&Utc)))
                    .transpose()?,
                deleted_at: None,
            };
            if m.expires_at.is_none_or(|e| e > now)
                && scope.is_none_or(|x| x == &m.scope)
                && query.is_none_or(|q| {
                    let q = q.to_lowercase();
                    m.key.to_lowercase().contains(&q)
                        || m.value.to_lowercase().contains(&q)
                        || m.provenance.to_lowercase().contains(&q)
                })
            {
                out.push(m)
            }
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
        self.add_expiring(old.scope, &old.key, value, provenance, old.expires_at)
    }
    pub fn merge(&self, ids: &[Uuid], key: &str, value: &str, provenance: &str) -> Result<Uuid> {
        if ids.len() < 2 {
            bail!("merge requires at least two memories")
        }
        let all = self.active()?;
        let selected: Vec<_> = all.iter().filter(|m| ids.contains(&m.id)).collect();
        if selected.len() != ids.len() {
            bail!("memory not found")
        }
        let scope = selected[0].scope.clone();
        if selected.iter().any(|m| m.scope != scope) {
            bail!("cannot merge different scopes")
        }
        for id in ids {
            self.delete(*id)?
        }
        self.add(scope, key, value, provenance)
    }
    pub fn expire(&self, now: DateTime<Utc>) -> Result<usize> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM memories WHERE deleted IS NULL AND expires IS NOT NULL AND expires<=?1",
        )?;
        let ids = statement
            .query_map([now.to_rfc3339()], |row| row.get::<_, String>(0))?
            .map(|id| Ok(Uuid::parse_str(&id?)?))
            .collect::<Result<Vec<_>>>()?;
        drop(statement);
        for id in &ids {
            self.delete(*id)?
        }
        Ok(ids.len())
    }
    pub fn export(&self) -> Result<MemoryExport> {
        Ok(MemoryExport {
            schema_version: 1,
            exported_at: Utc::now(),
            memories: self.active()?,
        })
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
fn digest(key: &str, v: &str) -> String {
    let mut d = Sha256::new();
    d.update(key.as_bytes());
    d.update([0]);
    d.update(v.trim().to_lowercase());
    format!("{:x}", d.finalize())
}
fn scope_s(s: &MemoryScope) -> String {
    serde_json::to_string(s).unwrap()
}
fn parse_scope(s: &str) -> Result<MemoryScope> {
    serde_json::from_str(s).map_err(Into::into)
}
