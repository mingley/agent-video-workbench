//! A single-owner persistent queue. An OS lock fences crashed workers; generation
//! checks fence stale database writes. Each retry writes a fresh artifact attempt.
use crate::{Error, Result, media, process::Control, store::Store};
use agentcut_render::FfmpegBackend;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderInput {
    pub sequence: String,
    pub expected_revision: u64,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub revision: i64,
    pub state: String,
    pub attempt: i64,
    pub generation: i64,
    pub cancel_requested: bool,
    pub updated: i64,
    pub input: Option<RenderInput>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

fn now() -> Result<i64> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid("system clock is before Unix epoch".into()))?;
    i64::try_from(time.as_millis()).map_err(|_| Error::Invalid("system clock overflow".into()))
}

impl Store {
    pub fn job(&self, id: &str) -> Result<Job> {
        let row = self.conn.query_row("SELECT id,revision,state,attempt,generation,cancel_requested,updated,input,result,error FROM jobs WHERE id=?1",[id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,bool>(5)?,r.get::<_,i64>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,Option<String>>(9)?)))?;
        Ok(Job {
            id: row.0,
            revision: row.1,
            state: row.2,
            attempt: row.3,
            generation: row.4,
            cancel_requested: row.5,
            updated: row.6,
            input: row.7.map(|s| serde_json::from_str(&s)).transpose()?,
            result: row.8.map(|s| serde_json::from_str(&s)).transpose()?,
            error: row.9,
        })
    }

    pub fn enqueue(&mut self, key: &str, input: &RenderInput) -> Result<Job> {
        if key.trim().is_empty() || key.len() > 256 {
            return Err(Error::Invalid("job key must contain 1..256 bytes".into()));
        }
        let encoded = serde_json::to_string(input)?;
        let hash = format!("{:x}", Sha256::digest(encoded.as_bytes()));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT hash,job_id FROM job_requests WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old, id)) = previous {
            if old != hash {
                return Err(Error::KeyConflict);
            }
            drop(tx);
            return self.job(&id);
        }
        let current: i64 = tx.query_row("SELECT revision FROM head", [], |r| r.get(0))?;
        let current_revision =
            u64::try_from(current).map_err(|_| Error::Invalid("invalid head revision".into()))?;
        if current_revision != input.expected_revision {
            return Err(Error::Conflict {
                expected: input.expected_revision,
                current: current_revision,
            });
        }
        let text: String = tx.query_row(
            "SELECT snapshot FROM revisions WHERE id=?1",
            [current],
            |r| r.get(0),
        )?;
        let project: agentcut_core::Project = serde_json::from_str(&text)?;
        let seq = project.require_sequence(&input.sequence)?;
        if seq.canvas.width > 4096 || seq.canvas.height > 4096 {
            return Err(Error::Invalid(
                "canvas exceeds the 4096-pixel worker limit".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO jobs(id,revision,state,input,updated) VALUES(?1,?2,'queued',?3,?4)",
            params![id, current, encoded, now()?],
        )?;
        tx.execute(
            "INSERT INTO job_requests VALUES(?1,?2,?3)",
            params![key, hash, id],
        )?;
        tx.commit()?;
        self.job(&id)
    }

    pub fn cancel_job(&self, id: &str) -> Result<Job> {
        self.conn.execute("UPDATE jobs SET cancel_requested=1,state=CASE WHEN state='queued' THEN 'cancelled' ELSE state END,updated=?1 WHERE id=?2 AND state IN ('queued','running','verifying')",params![now()?,id])?;
        self.job(id)
    }

    pub fn retry_job(&self, id: &str) -> Result<Job> {
        let changed = self.conn.execute("UPDATE jobs SET state='queued',cancel_requested=0,error=NULL,updated=?1 WHERE id=?2 AND input IS NOT NULL AND state IN ('failed','cancelled','interrupted')",params![now()?,id])?;
        if changed != 1 {
            return Err(Error::Invalid(
                "only failed, cancelled or interrupted jobs can be retried".into(),
            ));
        }
        self.job(id)
    }

    fn recover_queue(&self) -> Result<()> {
        self.conn.execute("UPDATE jobs SET state=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE 'interrupted' END,error='Worker exited before committing the artifact; retry starts a fresh attempt',generation=generation+1,updated=?1 WHERE state IN ('running','verifying')",[now()?])?;
        Ok(())
    }

    fn claim(&mut self) -> Result<Option<Job>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Option<String> = tx.query_row("SELECT id FROM jobs WHERE state='queued' AND input IS NOT NULL ORDER BY rowid LIMIT 1",[],|r|r.get(0)).optional()?;
        let Some(id) = id else { return Ok(None) };
        tx.execute("UPDATE jobs SET state='running',attempt=attempt+1,generation=generation+1,updated=?1 WHERE id=?2 AND state='queued'",params![now()?,id])?;
        tx.commit()?;
        self.job(&id).map(Some)
    }
}

struct JobControl<'a> {
    store: &'a Store,
    job: &'a Job,
    stop: Arc<AtomicBool>,
    heartbeat: Instant,
}
impl Control for JobControl<'_> {
    fn check(&mut self) -> Result<()> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let (cancel, generation): (bool, i64) = self.store.conn.query_row(
            "SELECT cancel_requested,generation FROM jobs WHERE id=?1",
            [&self.job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if cancel || generation != self.job.generation {
            return Err(Error::Cancelled);
        }
        if self.heartbeat.elapsed() > Duration::from_secs(1) {
            self.store.conn.execute(
                "UPDATE jobs SET updated=?1 WHERE id=?2 AND generation=?3",
                params![now()?, self.job.id, self.job.generation],
            )?;
            self.heartbeat = Instant::now();
        }
        Ok(())
    }
    fn stage(&mut self, stage: &str) -> Result<()> {
        self.check()?;
        self.store.conn.execute(
            "UPDATE jobs SET state=?1,updated=?2 WHERE id=?3 AND generation=?4",
            params![stage, now()?, self.job.id, self.job.generation],
        )?;
        Ok(())
    }
}

pub fn worker(root: &Path, stop: Arc<AtomicBool>, idle_seconds: u64) -> Result<Value> {
    let root = root.canonicalize()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("worker.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(json!({"worker":"already-running"}));
    }
    let mut store = Store::open(&root)?;
    // Lock acquisition proves the previous execution owner no longer holds it.
    store.recover_queue()?;
    let mut idle = Instant::now();
    let mut completed = 0;
    while !stop.load(Ordering::Relaxed) {
        let Some(job) = store.claim()? else {
            if idle.elapsed() >= Duration::from_secs(idle_seconds) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        idle = Instant::now();
        let input = job
            .input
            .as_ref()
            .ok_or_else(|| Error::Invalid("job input is missing".into()))?;
        let project = store.revision(
            u64::try_from(job.revision)
                .map_err(|_| Error::Invalid("invalid job revision".into()))?,
        )?;
        let backend = FfmpegBackend::new(&input.ffmpeg, &input.ffprobe);
        let artifact_id = format!("{}-{}", job.id, job.attempt);
        let mut control = JobControl {
            store: &store,
            job: &job,
            stop: stop.clone(),
            heartbeat: Instant::now(),
        };
        let result = media::render_attempt(
            &root,
            &input.sequence,
            &backend,
            &project,
            &artifact_id,
            &mut control,
        );
        let (state, result, error) = match result {
            Ok(value) => ("succeeded", Some(serde_json::to_string(&value)?), None),
            Err(Error::Cancelled) if stop.load(Ordering::Relaxed) => (
                "interrupted",
                None,
                Some("Worker stopped; retry is safe".to_owned()),
            ),
            Err(Error::Cancelled) => ("cancelled", None, Some("Job cancelled".to_owned())),
            Err(error) => ("failed", None, Some(error.to_string())),
        };
        let updated = store.conn.execute("UPDATE jobs SET state=?1,result=?2,error=?3,updated=?4 WHERE id=?5 AND generation=?6 AND state IN ('running','verifying')",params![state,result,error,now()?,job.id,job.generation])?;
        if updated != 1 {
            return Err(Error::Invalid("job execution generation was fenced".into()));
        }
        completed += 1;
    }
    drop(lock);
    Ok(json!({"worker":"stopped","processed":completed}))
}

pub fn launch(root: &Path, executable: &Path) -> Result<()> {
    Command::new(executable)
        .arg("worker")
        .arg(root.canonicalize()?)
        .arg("--idle-seconds")
        .arg("10")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub fn free_space(root: &Path) -> Result<u64> {
    let stats = nix::sys::statvfs::statvfs(root).map_err(std::io::Error::from)?;
    stats
        .blocks_available()
        .checked_mul(stats.fragment_size())
        .ok_or_else(|| Error::Invalid("filesystem size overflow".into()))
}
