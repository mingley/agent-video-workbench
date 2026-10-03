use crate::{Error, Result};
use agentcut_core::{OperationBatch, Project, RationalRate, apply_batch};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) root: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub project_id: uuid::Uuid,
    pub previous_revision: u64,
    pub revision: u64,
    #[serde(default)]
    pub created_ids: Vec<String>,
    #[serde(default)]
    pub changed_ids: Vec<String>,
}

impl Store {
    fn connect(path: &Path) -> Result<Self> {
        let conn = Connection::open(path.join("project.sqlite"))?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self {
            conn,
            root: path.canonicalize()?,
        })
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
             ALTER TABLE jobs ADD COLUMN input TEXT;
             ALTER TABLE jobs ADD COLUMN attempt INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE jobs ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE jobs ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
             CREATE TABLE job_requests(key TEXT PRIMARY KEY, hash TEXT NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id));
             CREATE TABLE imports(id TEXT PRIMARY KEY,asset_id TEXT NOT NULL,staged TEXT NOT NULL,state TEXT NOT NULL,error TEXT);
             PRAGMA user_version=4;",
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
        if (1..=3).contains(&version) {
            let backup = path.join(format!("schema{version}-{}.sqlite", uuid::Uuid::new_v4()));
            store.backup(&backup)?;
            let tx = store
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if actual == 1 {
                tx.execute_batch("CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, revision INTEGER NOT NULL REFERENCES revisions(id), state TEXT NOT NULL, result TEXT, error TEXT);")?;
            }
            if actual == 1 || actual == 2 {
                tx.execute_batch("ALTER TABLE jobs ADD COLUMN input TEXT;
                    ALTER TABLE jobs ADD COLUMN attempt INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE jobs ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE jobs ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
                    CREATE TABLE job_requests(key TEXT PRIMARY KEY,hash TEXT NOT NULL,job_id TEXT NOT NULL REFERENCES jobs(id));
                    PRAGMA user_version=3;")?;
            } else if actual != 3 && actual != 4 {
                return Err(Error::Invalid(format!("unsupported store schema {actual}")));
            }
            if actual <= 3 {
                tx.execute_batch("CREATE TABLE imports(id TEXT PRIMARY KEY,asset_id TEXT NOT NULL,staged TEXT NOT NULL,state TEXT NOT NULL,error TEXT); PRAGMA user_version=4;")?;
            }
            tx.commit()?;
        } else if version != 4 {
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

    pub fn unprotect(&mut self, id: &str, expected: u64, key: &str) -> Result<Outcome> {
        self.change(
            key,
            expected,
            json!({"kind":"protection.remove","id":id,"expectedRevision":expected}),
            false,
            |current| {
                let mut next = current.clone();
                crate::policy::remove(&mut next, id)?;
                Ok(next)
            },
        )
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

    pub(crate) fn change(
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
        if request["kind"] == "protection.remove" {
            crate::policy::validate(&next, &next)?;
        } else {
            crate::policy::validate(&current, &next)?;
        }
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("revision overflow".into()))?;
        let outcome = Outcome {
            project_id: current.project_id,
            previous_revision: current.revision,
            revision: next.revision,
            created_ids: {
                let previous: std::collections::BTreeSet<_> =
                    current.entity_ids().into_iter().collect();
                next.entity_ids()
                    .into_iter()
                    .filter(|id| !previous.contains(id))
                    .map(str::to_owned)
                    .collect()
            },
            changed_ids: crate::service::changed_ids(&current, &next)?,
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

    pub(crate) fn begin_import(&self, id: &str, asset: &str, staged: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO imports VALUES(?1,?2,?3,'copying',NULL)",
            params![id, asset, staged],
        )?;
        Ok(())
    }
    pub(crate) fn import_state(&self, id: &str, state: &str, error: Option<String>) -> Result<()> {
        self.conn.execute(
            "UPDATE imports SET state=?1,error=?2 WHERE id=?3",
            params![state, error, id],
        )?;
        Ok(())
    }
    pub(crate) fn recover_imports(&self, root: &Path) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id,staged FROM imports WHERE state IN ('copying','verifying','ready')",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, staged) = row?;
            let path = Path::new(&staged);
            if path.parent() != Some(Path::new("cache"))
                || !path.file_name().is_some_and(|name| {
                    name.to_string_lossy().starts_with(&format!("import-{id}."))
                })
            {
                return Err(Error::Invalid(
                    "invalid staged import path in database".into(),
                ));
            }
            match std::fs::remove_file(root.join(path)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            self.import_state(&id,"interrupted",Some("Import owner exited; original/head were preserved. Retry with the same request key.".into()))?;
        }
        Ok(())
    }
    pub fn imports(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,asset_id,state,error FROM imports ORDER BY rowid DESC LIMIT 100")?;
        let rows=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"assetId":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?})))?;
        rows.map(|r| r.map_err(Error::from)).collect()
    }

    pub fn request_outcome(&self, key: &str) -> Result<Option<Outcome>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT outcome FROM requests WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }

    pub fn history_page(&self, after: u64, through: u64, limit: u32) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,parent,request FROM revisions WHERE id>?1 AND id<=?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![sql_revision(after)?, sql_revision(through)?, limit],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        rows.map(|row|{let (revision,parent,text)=row?;let request:Value=serde_json::from_str(&text)?;Ok(json!({"revision":revision,"parent":parent,"description":request.get("description"),"kind":request.get("kind"),"operationCount":request.get("operations").and_then(|v|v.as_array()).map(Vec::len)}))}).collect()
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
