//! Shared application boundary for CLI requests and MCP tools.
use crate::{Error, Result, inspect, jobs, media, store::Store, workflow};
use agentcut_core::OperationBatch;
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const EDITS: &[&str] = &[
    "project.rename",
    "sequence.add",
    "sequence.set",
    "track.add",
    "track.set",
    "clip.add",
    "item.set",
    "item.move",
    "item.remove",
    "text.add",
    "caption.add",
    "item.trim",
    "item.split",
    "item.duplicate",
    "track.reorder",
    "track.remove",
    "effect.add",
    "effect.set",
    "effect.remove",
    "keyframe.set",
    "keyframe.remove",
    "bus.add",
    "bus.set",
    "bus.remove",
    "transition.add",
    "transition.remove",
    "marker.add",
    "marker.remove",
];
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "command",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Request {
    Doctor {},
    Capabilities {},
    Schema {},
    AgentGuide {},
    Create {
        project: PathBuf,
        name: String,
    },
    Status {
        project: PathBuf,
    },
    Resume {
        project: PathBuf,
    },
    History {
        project: PathBuf,
        #[serde(default)]
        after: u64,
        #[serde(default)]
        through: Option<u64>,
        #[serde(default = "limit")]
        limit: u32,
    },
    Diff {
        project: PathBuf,
        from: u64,
        to: u64,
    },
    RequestOutcome {
        project: PathBuf,
        key: String,
    },
    Import {
        project: PathBuf,
        source: PathBuf,
        id: String,
        expected_revision: u64,
        key: String,
    },
    Apply {
        project: PathBuf,
        batch: Value,
        #[serde(default)]
        dry_run: bool,
    },
    Restore {
        project: PathBuf,
        revision: u64,
        expected_revision: u64,
        key: String,
    },
    Unprotect {
        project: PathBuf,
        id: String,
        expected_revision: u64,
        key: String,
    },
    Protect {
        project: PathBuf,
        range: Value,
        expected_revision: u64,
        key: String,
    },
    Compose {
        project: PathBuf,
        edit: workflow::Compose,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        dry_run: bool,
    },
    TranscribeStart {
        project: PathBuf,
        asset_id: String,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        no_launch: bool,
    },
    TranscriptImport {
        project: PathBuf,
        transcript: workflow::Transcript,
        expected_revision: u64,
        key: String,
    },
    TranscriptSearch {
        project: PathBuf,
        query: String,
        #[serde(default)]
        asset_id: Option<String>,
        #[serde(default)]
        offset: u32,
        #[serde(default = "limit")]
        limit: u32,
    },
    Inspect {
        project: PathBuf,
        asset_id: String,
        #[serde(default)]
        kind: inspect::Kind,
        #[serde(default)]
        start_ms: i64,
        #[serde(default)]
        end_ms: Option<i64>,
    },
    Render {
        project: PathBuf,
        #[serde(default = "sequence")]
        sequence: String,
    },
    RenderStart {
        project: PathBuf,
        #[serde(default = "sequence")]
        sequence: String,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        no_launch: bool,
        #[serde(default)]
        priority: i32,
    },
    Imports {
        project: PathBuf,
    },
    Jobs {
        project: PathBuf,
    },
    JobStatus {
        project: PathBuf,
        id: String,
    },
    JobCancel {
        project: PathBuf,
        id: String,
    },
    JobRetry {
        project: PathBuf,
        id: String,
        #[serde(default)]
        no_launch: bool,
    },
    Artifacts {
        project: PathBuf,
    },
    Artifact {
        project: PathBuf,
        id: String,
        #[serde(default)]
        sheet: bool,
    },
    Backup {
        project: PathBuf,
        destination: PathBuf,
    },
    AnalyzeStart {
        project: PathBuf,
        task: crate::analysis::Task,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        no_launch: bool,
        #[serde(default)]
        priority: i32,
    },
    LibraryExport {
        project: PathBuf,
        destination: PathBuf,
    },
    LibraryImport {
        project: PathBuf,
        source: PathBuf,
        prefix: String,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        dry_run: bool,
    },
    ImportUrl {
        project: PathBuf,
        url: String,
        id: String,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default = "download_bytes")]
        max_bytes: u64,
    },
    InterchangeExport {
        project: PathBuf,
        sequence: String,
    },
    InterchangeImport {
        project: PathBuf,
        document: Value,
        id: String,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        dry_run: bool,
    },
    Catalog {
        workspace: PathBuf,
        #[serde(default)]
        query: String,
        #[serde(default)]
        offset: u32,
        #[serde(default = "limit")]
        limit: u32,
    },
    Maintain {
        workspace: PathBuf,
        #[serde(default = "grace")]
        grace_seconds: u64,
        #[serde(default = "yes")]
        dry_run: bool,
    },
    VerifyProject {
        project: PathBuf,
    },
    Relink {
        project: PathBuf,
        source: PathBuf,
        sha256: String,
    },
    CacheGc {
        project: PathBuf,
        #[serde(default = "grace")]
        grace_seconds: u64,
        #[serde(default = "yes")]
        dry_run: bool,
    },
    BackupRestore {
        source: PathBuf,
        destination: PathBuf,
    },
    BatchStart {
        project: PathBuf,
        sequences: Vec<String>,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        no_launch: bool,
        #[serde(default)]
        priority: i32,
    },
    BatchStatus {
        project: PathBuf,
        id: String,
    },
    Delivery {
        project: PathBuf,
        id: String,
        destination: PathBuf,
    },
    Studio {
        project: PathBuf,
        edit: crate::studio::Edit,
        expected_revision: u64,
        key: String,
        #[serde(default)]
        dry_run: bool,
    },
    StudioState {
        project: PathBuf,
        #[serde(default = "decision_prefix")]
        prefix: String,
        #[serde(default)]
        offset: u32,
        #[serde(default = "limit")]
        limit: u32,
    },
    Reviews {
        project: PathBuf,
        sequence: String,
    },
    Describe {
        capability: String,
    },
}
fn decision_prefix() -> String {
    "avw.".into()
}
fn download_bytes() -> u64 {
    8 * 1024 * 1024 * 1024
}
fn yes() -> bool {
    true
}
fn grace() -> u64 {
    86400
}
fn sequence() -> String {
    "seq_main".into()
}
fn limit() -> u32 {
    50
}
pub struct Service {
    pub root: Option<PathBuf>,
    pub backend: FfmpegBackend,
    pub executable: PathBuf,
    pub asr: Option<crate::asr::Config>,
    pub downloads: crate::transfer::Policy,
    pub provider: Option<crate::analysis::Provider>,
}
impl Service {
    fn path(&self, path: &Path) -> Result<PathBuf> {
        let raw = if let Some(root) = &self.root {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        } else {
            path.to_path_buf()
        };
        let resolved = if raw.exists() {
            raw.canonicalize()?
        } else {
            let parent = raw
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            parent.canonicalize()?.join(
                raw.file_name()
                    .ok_or_else(|| Error::Invalid("path has no filename".into()))?,
            )
        };
        if let Some(root) = &self.root
            && !resolved.starts_with(root.canonicalize()?)
        {
            return Err(Error::Invalid(
                "path is outside the authorized workspace".into(),
            ));
        }
        Ok(resolved)
    }
    fn launch(&self, root: &Path) -> Result<()> {
        jobs::launch(root, &self.executable)
    }
    pub fn execute(&self, request: Request) -> Result<Value> {
        match request {
            Request::AnalyzeStart {
                project,
                task,
                expected_revision,
                key,
                no_launch,
                priority,
            } => {
                let root = self.path(&project)?;
                let provider = if matches!(task, crate::analysis::Task::Provider { .. }) {
                    self.provider.clone()
                } else {
                    None
                };
                let input = jobs::RenderInput {
                    sequence: "seq_main".into(),
                    priority,
                    expected_revision,
                    ffmpeg: self.backend.ffmpeg_path().into(),
                    ffprobe: self.backend.ffprobe_path().into(),
                    asr: None,
                    analysis: Some(crate::analysis::Input { task, provider }),
                };
                let job = Store::open(&root)?.enqueue(&key, &input)?;
                if !no_launch && job.state == "queued" {
                    self.launch(&root)?;
                }
                Ok(serde_json::to_value(job)?)
            }
            Request::LibraryExport {
                project,
                destination,
            } => crate::library::export(&self.path(&project)?, &self.path(&destination)?),
            Request::LibraryImport {
                project,
                source,
                prefix,
                expected_revision,
                key,
                dry_run,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.library_import(
                    &self.path(&source)?,
                    &prefix,
                    expected_revision,
                    &key,
                    dry_run,
                )?,
            )?),
            Request::ImportUrl {
                project,
                url,
                id,
                expected_revision,
                key,
                sha256,
                max_bytes,
            } => crate::transfer::import(
                &self.path(&project)?,
                crate::transfer::Input {
                    url: &url,
                    id: &id,
                    expected: expected_revision,
                    key: &key,
                    sha256: sha256.as_deref(),
                    max_bytes,
                },
                &self.downloads,
                &self.backend,
            ),
            Request::InterchangeExport { project, sequence } => crate::interchange::export(
                &Store::open(&self.path(&project)?)?.project()?,
                &sequence,
            ),
            Request::InterchangeImport {
                project,
                document,
                id,
                expected_revision,
                key,
                dry_run,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.interchange_import(
                    &document,
                    &id,
                    expected_revision,
                    &key,
                    dry_run,
                )?,
            )?),
            Request::Catalog {
                workspace,
                query,
                offset,
                limit,
            } => crate::storage::catalog(&self.path(&workspace)?, &query, offset, limit),
            Request::VerifyProject { project } => crate::storage::verify(&self.path(&project)?),
            Request::Relink {
                project,
                source,
                sha256,
            } => crate::storage::relink(&self.path(&project)?, &self.path(&source)?, &sha256),
            Request::CacheGc {
                project,
                grace_seconds,
                dry_run,
            } => crate::storage::gc(&self.path(&project)?, grace_seconds, dry_run),
            Request::BackupRestore {
                source,
                destination,
            } => media::backup(&self.path(&source)?, &self.path(&destination)?),
            Request::BatchStart {
                project,
                sequences,
                expected_revision,
                key,
                no_launch,
                priority,
            } => {
                let root = self.path(&project)?;
                let inputs: Vec<_> = sequences
                    .into_iter()
                    .map(|sequence| jobs::RenderInput {
                        sequence,
                        priority,
                        expected_revision,
                        ffmpeg: self.backend.ffmpeg_path().into(),
                        ffprobe: self.backend.ffprobe_path().into(),
                        asr: None,
                        analysis: None,
                    })
                    .collect();
                let batch = Store::open(&root)?.enqueue_batch(&key, &inputs)?;
                if !no_launch {
                    self.launch(&root)?;
                }
                Ok(serde_json::to_value(batch)?)
            }
            Request::BatchStatus { project, id } => {
                Store::open(&self.path(&project)?)?.batch_status(&id)
            }
            Request::Delivery {
                project,
                id,
                destination,
            } => crate::delivery::package(&self.path(&project)?, &id, &self.path(&destination)?),
            Request::Studio {
                project,
                edit,
                expected_revision,
                key,
                dry_run,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.studio(
                    &edit,
                    expected_revision,
                    &key,
                    dry_run,
                )?,
            )?),
            Request::Maintain {
                workspace,
                grace_seconds,
                dry_run,
            } => crate::storage::maintain(&self.path(&workspace)?, grace_seconds, dry_run),
            Request::StudioState {
                project,
                prefix,
                offset,
                limit,
            } => {
                if !(1..=100).contains(&limit) {
                    return Err(Error::Invalid("decision limit requires 1..100".into()));
                }
                let p = Store::open(&self.path(&project)?)?.project()?;
                let mut all = p
                    .extensions
                    .into_iter()
                    .filter(|(key, _)| key.starts_with(&prefix))
                    .collect::<Vec<_>>();
                all.sort_by(|a, b| a.0.cmp(&b.0));
                let total = all.len();
                let mut decisions = serde_json::Map::new();
                for (key, value) in all.into_iter().skip(offset as usize).take(limit as usize) {
                    let bytes = serde_json::to_vec(&value)?.len();
                    decisions.insert(key,if bytes<=65536 {value} else {json!({"omitted":true,"bytes":bytes,"assetId":value["assetId"],"language":value["language"],"provider":value["provider"],"detail":"use bounded transcript-search or an explicitly requested snapshot"})});
                }
                Ok(
                    json!({"revision":p.revision,"decisions":decisions,"nextOffset":if offset as usize+decisions.len()<total {Some(offset+limit)}else{None}}),
                )
            }
            Request::Reviews { project, sequence } => crate::studio::remap_reviews(
                &Store::open(&self.path(&project)?)?.project()?,
                &sequence,
            ),
            Request::Schema {} => Ok(serde_json::to_value(schemars::schema_for!(Request))?),
            Request::Capabilities {} => Ok(
                json!({"apiVersion":"1","version":env!("CARGO_PKG_VERSION"),"commands":["maintain","analyze-start","library-export","library-import","import-url","interchange-export","interchange-import","catalog","verify-project","relink","cache-gc","backup-restore","batch-start","batch-status","delivery","studio","studio-state","reviews","resume","import","imports","transcribe-start","transcript-import","transcript-search","compose","apply","restore","protect","unprotect","inspect","render-start","job-status","job-cancel","job-retry","artifact","backup"],"editOperations":EDITS,"media":{"output":"SDR H.264/AAC","source":"local SDR/PQ/HLG video/audio/image/font","hdr":"PQ/HLG tone mapped per source to Rec.709; Dolby Vision compatible profile 8 base layer only","asr":{"provider":"local whisper.cpp","configured":self.asr.is_some(),"machineTextRequiresReview":true}},"analysisProviderConfigured":self.provider.is_some(),"renderAnimation":"opacity; linear/step position and constant-viewport crop; other channels refused", "downloadHosts":self.downloads.hosts,"limits":{"requestBytes":8388608,"analysisRangeMs":300000,"outputDurationSeconds":3600,"canvasPixelsPerAxis":4096,"parallelRendersPerProject":1},"transports":["CLI JSON","MCP stdio"],"supportedPlatform":"Linux; local filesystem with locking"}),
            ),
            Request::AgentGuide {} => Ok(json!({"guide":include_str!("../AGENT_GUIDE.md")})),
            Request::Doctor {} => doctor(&self.backend),
            Request::Describe { capability } => {
                if !EDITS.contains(&capability.as_str()) {
                    return Err(Error::Invalid(
                        "operation is outside the supported editing surface".into(),
                    ));
                }
                operation_description(&capability)
            }
            Request::Create { project, name } => {
                let path = self.path(&project)?;
                Ok(serde_json::to_value(
                    Store::create(&path, &name)?.project()?,
                )?)
            }
            Request::Status { project } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.project()?,
            )?),
            Request::Resume { project } => {
                let store = Store::open(&self.path(&project)?)?;
                let p = store.project()?;
                let jobs = job_list(&store, 10)?;
                Ok(
                    json!({"projectId":p.project_id,"name":p.name,"revision":p.revision,"outputs":p.sequences.iter().map(|s|json!({"id":s.id,"name":s.name,"width":s.canvas.width,"height":s.canvas.height,"frameRate":s.frame_rate,"trackCount":s.tracks.len()})).collect::<Vec<_>>(),"assets":p.assets.iter().take(100).map(|a|json!({"id":a.id,"kind":a.kind,"sha256":a.fingerprint.sha256,"duration":a.metadata.duration})).collect::<Vec<_>>(),"assetsTruncated":p.assets.len()>100,"protectedRanges":crate::policy::ranges(&p)?,"recentHistory":store.history_page(p.revision.saturating_sub(5),p.revision,5)?,"jobs":jobs}),
                )
            }
            Request::History {
                project,
                after,
                through,
                limit,
            } => {
                check_limit(limit)?;
                let store = Store::open(&self.path(&project)?)?;
                let revision = through.unwrap_or(store.project()?.revision);
                let mut items = store.history_page(after, revision, limit + 1)?;
                let more = items.len() > limit as usize;
                items.truncate(limit as usize);
                let next = if more {
                    items.last().and_then(|v| v["revision"].as_u64())
                } else {
                    None
                };
                Ok(json!({"revision":revision,"items":items,"nextAfter":next}))
            }
            Request::Diff { project, from, to } => {
                let store = Store::open(&self.path(&project)?)?;
                semantic_diff(
                    &serde_json::to_value(store.revision(from)?)?,
                    &serde_json::to_value(store.revision(to)?)?,
                )
            }
            Request::RequestOutcome { project, key } => {
                Ok(json!({"outcome":Store::open(&self.path(&project)?)?.request_outcome(&key)?}))
            }
            Request::Import {
                project,
                source,
                id,
                expected_revision,
                key,
            } => Ok(serde_json::to_value(media::import(
                &self.path(&project)?,
                &self.path(&source)?,
                &id,
                expected_revision,
                &key,
                &self.backend,
            )?)?),
            Request::Apply {
                project,
                batch,
                dry_run,
            } => {
                let batch: OperationBatch = crate::json::parse(&serde_json::to_vec(&batch)?)?;
                if batch.operations.len() > 5000 {
                    return Err(Error::Invalid("batch exceeds 5000 operations".into()));
                }
                for op in &batch.operations {
                    if !EDITS.contains(&op.op.as_str()) {
                        return Err(Error::Invalid(format!(
                            "unsupported operation {}; managed assets must be imported",
                            op.op
                        )));
                    }
                }
                Ok(serde_json::to_value(
                    Store::open(&self.path(&project)?)?.apply(&batch, dry_run)?,
                )?)
            }
            Request::Restore {
                project,
                revision,
                expected_revision,
                key,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.restore(revision, expected_revision, &key)?,
            )?),
            Request::Unprotect {
                project,
                id,
                expected_revision,
                key,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.unprotect(&id, expected_revision, &key)?,
            )?),
            Request::Protect {
                project,
                range,
                expected_revision,
                key,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.protect(
                    serde_json::from_value(range)?,
                    expected_revision,
                    &key,
                )?,
            )?),
            Request::Compose {
                project,
                edit,
                expected_revision,
                key,
                dry_run,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.compose(
                    &edit,
                    expected_revision,
                    &key,
                    dry_run,
                )?,
            )?),
            Request::TranscribeStart {
                project,
                asset_id,
                expected_revision,
                key,
                no_launch,
            } => {
                let config=self.asr.clone().ok_or_else(||Error::Invalid("local ASR is not configured; launch with --whisper, --whisper-model and --whisper-model-sha256, or import a reviewed transcript".into()))?;
                let root = self.path(&project)?;
                let mut store = Store::open(&root)?;
                store.project()?.require_asset(&asset_id)?;
                let job = store.enqueue(
                    &key,
                    &jobs::RenderInput {
                        sequence: "seq_main".into(),
                        priority: 0,
                        expected_revision,
                        ffmpeg: self.backend.ffmpeg_path().into(),
                        ffprobe: self.backend.ffprobe_path().into(),
                        asr: Some(crate::asr::Input { asset_id, config }),
                        analysis: None,
                    },
                )?;
                if !no_launch && job.state == "queued" {
                    self.launch(&root)?;
                }
                Ok(serde_json::to_value(job)?)
            }
            Request::TranscriptImport {
                project,
                transcript,
                expected_revision,
                key,
            } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.transcript_import(
                    transcript,
                    expected_revision,
                    &key,
                )?,
            )?),
            Request::TranscriptSearch {
                project,
                query,
                asset_id,
                offset,
                limit,
            } => {
                check_limit(limit)?;
                let p = Store::open(&self.path(&project)?)?.project()?;
                let transcript = workflow::transcripts(&p)?;
                let query = query.to_lowercase();
                let hits: Vec<_> = transcript
                    .iter()
                    .filter(|t| asset_id.as_ref().is_none_or(|id| *id == t.asset_id))
                    .flat_map(|t| {
                        t.cues
                            .iter()
                            .filter(|c| c.text.to_lowercase().contains(&query))
                            .map(|c| json!({"assetId":t.asset_id,"cue":c}))
                    })
                    .skip(offset as usize)
                    .take(limit as usize + 1)
                    .collect();
                let more = hits.len() > limit as usize;
                let items: Vec<_> = hits.into_iter().take(limit as usize).collect();
                Ok(
                    json!({"revision":p.revision,"items":items,"nextOffset":if more{Some(offset+limit)}else{None}}),
                )
            }
            Request::Inspect {
                project,
                asset_id,
                kind,
                start_ms,
                end_ms,
            } => inspect::inspect(
                &self.path(&project)?,
                &asset_id,
                kind,
                start_ms,
                end_ms,
                &self.backend,
            ),
            Request::Render { project, sequence } => {
                media::render(&self.path(&project)?, &sequence, &self.backend)
            }
            Request::RenderStart {
                project,
                sequence,
                expected_revision,
                key,
                no_launch,
                priority,
            } => {
                let root = self.path(&project)?;
                let job = Store::open(&root)?.enqueue(
                    &key,
                    &jobs::RenderInput {
                        sequence,
                        priority,
                        expected_revision,
                        ffmpeg: self.backend.ffmpeg_path().into(),
                        ffprobe: self.backend.ffprobe_path().into(),
                        asr: None,
                        analysis: None,
                    },
                )?;
                if !no_launch && job.state == "queued" {
                    self.launch(&root)?;
                }
                Ok(serde_json::to_value(job)?)
            }
            Request::Imports { project } => {
                Ok(json!(Store::open(&self.path(&project)?)?.imports()?))
            }
            Request::Jobs { project } => job_list(&Store::open(&self.path(&project)?)?, 100),
            Request::JobStatus { project, id } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.job(&id)?,
            )?),
            Request::JobCancel { project, id } => Ok(serde_json::to_value(
                Store::open(&self.path(&project)?)?.cancel_job(&id)?,
            )?),
            Request::JobRetry {
                project,
                id,
                no_launch,
            } => {
                let root = self.path(&project)?;
                let job = Store::open(&root)?.retry_job(&id)?;
                if !no_launch {
                    self.launch(&root)?;
                }
                Ok(serde_json::to_value(job)?)
            }
            Request::Artifacts { project } => {
                let jobs = job_list(&Store::open(&self.path(&project)?)?, 100)?;
                Ok(
                    json!({"artifacts":jobs.as_array().ok_or_else(||Error::Invalid("invalid job list".into()))?.iter().filter(|j|j["state"]=="succeeded").map(|j|&j["result"]).collect::<Vec<_>>(),"limit":100}),
                )
            }
            Request::Artifact { project, id, sheet } => {
                let root = self.path(&project)?;
                let store = Store::open(&root)?;
                let job = store.job(&id)?;
                if job.state != "succeeded" {
                    return Err(Error::Invalid("job has no verified artifact".into()));
                }
                let result = job
                    .result
                    .ok_or_else(|| Error::Invalid("artifact result is missing".into()))?;
                if result["mimeType"] == "application/json" {
                    let path =
                        self.path(Path::new(result["path"].as_str().ok_or_else(|| {
                            Error::Invalid("transcript artifact path missing".into())
                        })?))?;
                    let hash = media::hash_file(&path)?;
                    if result["sha256"] != hash {
                        return Err(Error::Invalid("transcript artifact bytes changed".into()));
                    }
                    let report: Value = crate::json::read(&path)?;
                    let mut attachments = Vec::new();
                    if let Some(files) = report["files"].as_array() {
                        for file in files {
                            let name = file["name"].as_str().ok_or_else(|| {
                                Error::Invalid("analysis attachment name missing".into())
                            })?;
                            if Path::new(name).components().count() != 1 {
                                return Err(Error::Invalid(
                                    "invalid analysis attachment path".into(),
                                ));
                            }
                            let attachment = self.path(&path.with_file_name(name))?;
                            if media::hash_file(&attachment)? != file["sha256"] {
                                return Err(Error::Invalid(
                                    "analysis attachment bytes changed".into(),
                                ));
                            }
                            attachments.push(json!({"path":attachment,"sha256":file["sha256"],"bytes":std::fs::metadata(&attachment)?.len()}));
                        }
                    }
                    return Ok(
                        json!({"jobId":id,"path":path,"revision":job.revision,"sha256":hash,"bytes":std::fs::metadata(path)?.len(),"mimeType":"application/json","verified":true,"needsTextReview":result.get("assetId").is_some(),"attachments":attachments}),
                    );
                }
                let manifest_path = result["manifest"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("artifact manifest is missing".into()))?;
                let manifest: Value = crate::json::read(&self.path(Path::new(manifest_path))?)?;
                let path = if sheet {
                    Path::new(manifest_path).with_file_name("sheet.png")
                } else {
                    PathBuf::from(
                        result["path"]
                            .as_str()
                            .ok_or_else(|| Error::Invalid("artifact path is missing".into()))?,
                    )
                };
                let path = self.path(&path)?;
                let hash = media::hash_file(&path)?;
                if (if sheet {
                    &manifest["contactSheet"]["sha256"]
                } else {
                    &manifest["verification"]["sha256"]
                }) != &hash
                {
                    return Err(Error::Invalid(
                        "artifact bytes changed since verification".into(),
                    ));
                }
                Ok(
                    json!({"jobId":id,"path":path,"revision":job.revision,"sha256":hash,"bytes":std::fs::metadata(path)?.len(),"mimeType":if sheet{"image/png"}else{"video/mp4"},"verified":true,"manifest":manifest_path}),
                )
            }
            Request::Backup {
                project,
                destination,
            } => media::backup(&self.path(&project)?, &self.path(&destination)?),
        }
    }
}
fn check_limit(limit: u32) -> Result<()> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Invalid("limit must be 1..100".into()));
    }
    Ok(())
}
fn job_list(store: &Store, limit: u32) -> Result<Value> {
    let mut stmt = store
        .conn
        .prepare("SELECT id FROM jobs ORDER BY rowid DESC LIMIT ?1")?;
    let ids = stmt.query_map([limit], |r| r.get::<_, String>(0))?;
    let mut jobs = Vec::new();
    for id in ids {
        jobs.push(store.job(&id?)?);
    }
    Ok(serde_json::to_value(jobs)?)
}
pub fn envelope(result: Result<Value>) -> Value {
    match result {
        Ok(value) => json!({"ok":true,"apiVersion":"1","result":value}),
        Err(error) => {
            let details = match &error {
                Error::Conflict { expected, current } => {
                    json!({"expectedRevision":expected,"currentRevision":current})
                }
                _ => json!({}),
            };
            json!({"ok":false,"apiVersion":"1","error":{"code":error.code(),"message":error.to_string(),"details":details,"retryable":matches!(error,Error::Timeout|Error::Execution{..})}})
        }
    }
}
fn semantic_diff(from: &Value, to: &Value) -> Result<Value> {
    fn entities(p: &Value) -> std::collections::BTreeMap<String, Value> {
        let mut out = std::collections::BTreeMap::new();
        for field in ["assets", "sequences"] {
            if let Some(items) = p[field].as_array() {
                for item in items {
                    if let Some(id) = item["id"].as_str() {
                        out.insert(id.into(), item.clone());
                    }
                    if let Some(tracks) = item["tracks"].as_array() {
                        for track in tracks {
                            if let Some(id) = track["id"].as_str() {
                                out.insert(id.into(), track.clone());
                            }
                            if let Some(items) = track["items"].as_array() {
                                for item in items {
                                    if let Some(id) = item["id"].as_str() {
                                        out.insert(id.into(), item.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
    let a = entities(from);
    let b = entities(to);
    let created: Vec<_> = b.keys().filter(|id| !a.contains_key(*id)).collect();
    let removed: Vec<_> = a.keys().filter(|id| !b.contains_key(*id)).collect();
    let changed: Vec<_> = a
        .keys()
        .filter(|id| b.contains_key(*id) && a.get(*id) != b.get(*id))
        .collect();
    Ok(
        json!({"fromRevision":from["revision"],"toRevision":to["revision"],"name":{"from":from["name"],"to":to["name"]},"createdIds":created,"removedIds":removed,"changedIds":changed}),
    )
}
fn operation_description(operation: &str) -> Result<Value> {
    let time = json!({"value":30,"rate":{"numerator":30,"denominator":1}});
    let params = match operation {
        "project.rename" => json!({"name":"New name"}),
        "sequence.add" => {
            json!({"id":"output","name":"Output","width":1080,"height":1920,"frameRate":{"numerator":30,"denominator":1}})
        }
        "sequence.set" => json!({"name":"Updated output"}),
        "track.add" => json!({"id":"video","sequence":"output","type":"video"}),
        "track.set" => json!({"enabled":true,"muted":false}),
        "clip.add" => {
            json!({"id":"clip","asset":"source","track":"video","at":time,"sourceIn":time,"duration":time,"fit":"cover"})
        }
        "item.set" => json!({"property":"text.style.fontSize","value":48}),
        "item.move" => json!({"to":time,"collision":"reject"}),
        "item.remove" => json!({"ripple":"none"}),
        "text.add" | "caption.add" => {
            json!({"id":"caption","track":"captions","at":time,"duration":time,"text":"Literal caption"})
        }
        "item.trim" => json!({"sourceIn":time}),
        "item.split" => json!({"at":time,"leftId":"left","rightId":"right"}),
        "item.duplicate" => json!({"id":"copy","at":time}),
        "track.reorder" => json!({"position":1}),
        "effect.add" => json!({"id":"gain","effect":"audio.gain","params":{"gainDb":-3}}),
        "effect.set" => json!({"enabled":false}),
        "keyframe.set" => {
            json!({"property":"transform.position","at":time,"value":{"x":180,"y":320},"interpolation":"linear"})
        }
        "keyframe.remove" => json!({"property":"transform.position"}),
        "bus.add" => json!({"id":"dialogue","sequence":"output","gainDb":0}),
        "bus.set" => json!({"gainDb":-3}),
        "transition.add" => {
            json!({"id":"dissolve","sequence":"output","left":"left","right":"right","type":"video.transition.crossfade","duration":time,"handlePolicy":"reject"})
        }
        "marker.add" => json!({"id":"note","sequence":"output","at":time,"name":"Review"}),
        "track.remove" | "effect.remove" | "bus.remove" | "transition.remove" | "marker.remove" => {
            json!({})
        }
        _ => return Err(Error::Invalid("unsupported operation".into())),
    };
    let target_required = matches!(
        operation,
        "sequence.set"
            | "track.set"
            | "item.set"
            | "item.move"
            | "item.remove"
            | "item.trim"
            | "item.split"
            | "item.duplicate"
            | "track.reorder"
            | "track.remove"
            | "effect.add"
            | "effect.set"
            | "effect.remove"
            | "keyframe.set"
            | "keyframe.remove"
            | "bus.set"
            | "bus.remove"
            | "transition.remove"
            | "marker.remove"
    );
    let mut example = json!({"id":"edit-1","op":operation,"params":params});
    if target_required {
        example["target"] = json!("existing-entity-id");
    }
    let properties: Value = if operation == "item.set" {
        serde_json::to_value(agentcut_core::capabilities::property_registry())?
    } else {
        json!([])
    };
    Ok(
        json!({"operation":operation,"targetRequired":target_required,"example":example,"itemProperties":properties,"validation":"The pinned domain engine validates parameters, IDs, time ranges, collisions and media references. Use apply dryRun against the current revision before committing unfamiliar edits.","coreRevision":"20ecdffc9d770bc280b294ee00414cafe4ce36ed","effects":agentcut_core::capabilities::registry().iter().filter(|c| c.id.starts_with("audio.") || c.id.starts_with("video.") || c.id.starts_with("transition.")).collect::<Vec<_>>()}),
    )
}

fn doctor(backend: &FfmpegBackend) -> Result<Value> {
    let version = crate::process::run(
        Command::new(backend.ffmpeg_path()).arg("-version"),
        Duration::from_secs(10),
        &mut crate::process::Uncontrolled,
    )?;
    crate::process::run(
        Command::new(backend.ffprobe_path()).arg("-version"),
        Duration::from_secs(10),
        &mut crate::process::Uncontrolled,
    )?;
    let filters = crate::process::run(
        Command::new(backend.ffmpeg_path()).args(["-filters"]),
        Duration::from_secs(10),
        &mut crate::process::Uncontrolled,
    )?;
    let filters = String::from_utf8_lossy(&filters.stdout);
    for required in [
        "drawtext",
        "zscale",
        "tonemap",
        "loudnorm",
        "sidechaincompress",
        "afade",
    ] {
        if !filters
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(required))
        {
            return Err(Error::Invalid(format!(
                "configured FFmpeg lacks required filter {required}"
            )));
        }
    }
    let directory = std::env::temp_dir().join(format!("avw-doctor-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory)?;
    let path = directory.join("test.mp4");
    let outcome = (|| {
        crate::process::run(
            Command::new(backend.ffmpeg_path())
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=64x64:r=30:d=0.2",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=0.2",
                    "-vf",
                    "drawtext=text=AVW:fontsize=12",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                ])
                .arg(&path),
            Duration::from_secs(30),
            &mut crate::process::Uncontrolled,
        )?;
        crate::process::run(
            Command::new(backend.ffmpeg_path())
                .args(["-v", "error", "-xerror", "-i"])
                .arg(&path)
                .args(["-f", "null", "-"]),
            Duration::from_secs(30),
            &mut crate::process::Uncontrolled,
        )?;
        Ok(
            json!({"ready":true,"ffmpeg":String::from_utf8_lossy(&version.stdout).lines().next(),"checks":["ffprobe","H.264/AAC encode","drawtext captions","required HDR/audio filters","full output decode"],"optionalGpuRequired":false}),
        )
    })();
    let _ = std::fs::remove_dir_all(directory);
    outcome
}

pub(crate) fn changed_ids(
    from: &agentcut_core::Project,
    to: &agentcut_core::Project,
) -> Result<Vec<String>> {
    let diff = semantic_diff(&serde_json::to_value(from)?, &serde_json::to_value(to)?)?;
    Ok(serde_json::from_value(diff["changedIds"].clone())?)
}
