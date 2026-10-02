use crate::{Error, Result};
use agentcut_core::{OperationBatch, Project, RationalRate, apply_batch};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub project_id: uuid::Uuid,
    pub previous_revision: u64,
    pub revision: u64,
}

impl Store {
    fn connect(path: &Path) -> Result<Self> {
        let conn = Connection::open(path.join("project.sqlite"))?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { conn })
    }

    pub fn create(path: &Path, name: &str) -> Result<Self> {
        // Refuse to initialize over any existing project or unrelated directory.
        std::fs::create_dir(path)?;
        for dir in ["originals", "analysis", "cache", "renders", "exports"] {
            std::fs::create_dir(path.join(dir))?;
        }
        let mut store = Self::connect(path)?;
        let tx = store.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE revisions(id INTEGER PRIMARY KEY, parent INTEGER REFERENCES revisions(id), snapshot TEXT NOT NULL, request TEXT NOT NULL);
             CREATE TABLE head(singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL REFERENCES revisions(id));
             CREATE TABLE requests(key TEXT PRIMARY KEY, hash TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TABLE jobs(id TEXT PRIMARY KEY, revision INTEGER NOT NULL REFERENCES revisions(id), state TEXT NOT NULL, result TEXT, error TEXT);
             PRAGMA user_version=2;",
        )?;
        let project = Project::new(
            "",
            360,
            640,
            RationalRate {
                numerator: 30,
                denominator: 1,
            },
        );
        let project = Project {
            name: name.into(),
            ..project
        };
        tx.execute(
            "INSERT INTO revisions VALUES(0,NULL,?1,'{\"kind\":\"create\"}')",
            [serde_json::to_string(&project)?],
        )?;
        tx.execute("INSERT INTO head VALUES(1,0)", [])?;
        tx.commit()?;
        Ok(store)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if path.join("backup.incomplete").exists() {
            return Err(Error::Invalid("backup is incomplete".into()));
        }
        Self::open_backup(path)
    }

    pub(crate) fn open_backup(path: &Path) -> Result<Self> {
        if !path.join("project.sqlite").is_file() {
            return Err(Error::Invalid("project.sqlite is missing".into()));
        }
        let mut store = Self::connect(path)?;
        let version: u32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 1 {
            // Keep a consistent pre-migration copy. Never overwrite a user's backup.
            let backup = path.join(format!("schema1-{}.sqlite", uuid::Uuid::new_v4()));
            store.backup(&backup)?;
            let tx = store
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, revision INTEGER NOT NULL REFERENCES revisions(id), state TEXT NOT NULL, result TEXT, error TEXT); PRAGMA user_version=2;")?;
            tx.commit()?;
        } else if version != 2 {
            return Err(Error::Invalid(format!(
                "unsupported store schema {version}"
            )));
        }
        store.project()?;
        Ok(store)
    }

    pub fn project(&self) -> Result<Project> {
        let text: String = self.conn.query_row(
            "SELECT snapshot FROM revisions JOIN head ON revisions.id=head.revision",
            [],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn revision(&self, id: u64) -> Result<Project> {
        let text: String = self.conn.query_row(
            "SELECT snapshot FROM revisions WHERE id=?1",
            [sql_revision(id)?],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn history(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,parent,request FROM revisions ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (revision, parent, request) = row?;
            result.push(json!({"revision":revision,"parent":parent,"request":serde_json::from_str::<Value>(&request)?}));
        }
        Ok(result)
    }

    pub fn apply(&mut self, batch: &OperationBatch, dry_run: bool) -> Result<Outcome> {
        let request = serde_json::to_value(batch)?;
        self.change(
            &batch.idempotency_key,
            batch.base_revision,
            request,
            dry_run,
            |p| Ok(apply_batch(p, batch)?.project),
        )
    }

    /// Undo/redo are explicit restores; they append history instead of rewriting it.
    pub fn protect(
        &mut self,
        range: crate::policy::ProtectedRange,
        expected: u64,
        key: &str,
    ) -> Result<Outcome> {
        let request = json!({"kind":"protect","range":range,"expectedRevision":expected});
        self.change(key, expected, request, false, |current| {
            let mut next = current.clone();
            crate::policy::add(&mut next, range)?;
            Ok(next)
        })
    }

    pub fn restore(&mut self, revision: u64, expected: u64, key: &str) -> Result<Outcome> {
        let target = self.revision(revision)?;
        self.change(
            key,
            expected,
            json!({"kind":"restore","target":revision,"expectedRevision":expected}),
            false,
            |current| {
                if target.project_id != current.project_id {
                    return Err(Error::Invalid("wrong project".into()));
                }
                let mut target = target;
                crate::policy::preserve(current, &mut target);
                Ok(target)
            },
        )
    }

    fn change(
        &mut self,
        key: &str,
        expected: u64,
        request: Value,
        dry_run: bool,
        change: impl FnOnce(&Project) -> Result<Project>,
    ) -> Result<Outcome> {
        if key.trim().is_empty() {
            return Err(Error::Invalid("idempotency key is empty".into()));
        }
        let text = serde_json::to_string(&request)?;
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT hash,outcome FROM requests WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_hash, outcome)) = prior {
            if old_hash != hash {
                return Err(Error::KeyConflict);
            }
            return Ok(serde_json::from_str(&outcome)?);
        }
        let snapshot: String = tx.query_row(
            "SELECT snapshot FROM revisions JOIN head ON revisions.id=head.revision",
            [],
            |r| r.get(0),
        )?;
        let current: Project = serde_json::from_str(&snapshot)?;
        if current.revision != expected {
            return Err(Error::Conflict {
                expected,
                current: current.revision,
            });
        }
        let mut next = change(&current)?;
        crate::policy::validate(&current, &next)?;
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("revision overflow".into()))?;
        let outcome = Outcome {
            project_id: current.project_id,
            previous_revision: current.revision,
            revision: next.revision,
        };
        if !dry_run {
            tx.execute(
                "INSERT INTO revisions VALUES(?1,?2,?3,?4)",
                params![
                    sql_revision(next.revision)?,
                    sql_revision(current.revision)?,
                    serde_json::to_string(&next)?,
                    text
                ],
            )?;
            tx.execute(
                "UPDATE head SET revision=?1 WHERE singleton=1",
                [sql_revision(next.revision)?],
            )?;
            tx.execute(
                "INSERT INTO requests VALUES(?1,?2,?3)",
                params![key, hash, serde_json::to_string(&outcome)?],
            )?;
            tx.commit()?;
        }
        Ok(outcome)
    }

    pub(crate) fn mark_backup_jobs_unavailable(&self) -> Result<()> {
        self.conn.execute("UPDATE jobs SET state='unavailable',error='Derived artifacts omitted from portable backup'", [])?;
        Ok(())
    }

    pub fn start_job(&self, id: &str, revision: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO jobs(id,revision,state) VALUES(?1,?2,'running')",
            params![id, sql_revision(revision)?],
        )?;
        Ok(())
    }

    pub fn finish_job(&self, id: &str, result: &Value) -> Result<()> {
        let changed = self.conn.execute("UPDATE jobs SET state='succeeded',result=?1,error=NULL WHERE id=?2 AND state='running'", params![serde_json::to_string(result)?, id])?;
        if changed != 1 {
            return Err(Error::Invalid("job is missing or no longer running".into()));
        }
        Ok(())
    }

    pub fn fail_job(&self, id: &str, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state='failed',error=?1 WHERE id=?2 AND state='running'",
            params![error, id],
        )?;
        Ok(())
    }

    pub fn jobs(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,revision,state,result,error FROM jobs ORDER BY rowid")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, revision, state, result, error) = row?;
            let result = result
                .map(|value| serde_json::from_str::<Value>(&value))
                .transpose()?;
            Ok(json!({"id":id,"revision":revision,"state":state,"result":result,"error":error}))
        })
        .collect()
    }

    pub fn backup(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(Error::Invalid("backup destination exists".into()));
        }
        self.conn.backup("main", destination, None)?;
        Ok(())
    }
}

fn sql_revision(revision: u64) -> Result<i64> {
    i64::try_from(revision).map_err(|_| Error::Invalid("revision exceeds SQLite range".into()))
}
