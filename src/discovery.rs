//! Source-bound evidence pages and reviewable, agent-supplied highlight proposals.
//!
//! This module does not interpret speech or video. The external agent (or its
//! provider) finds moments; AVW checks their identities, supplies source
//! context and constructs requests for the existing editorial workflow.
use crate::{Error, Result, analysis, editorial, jobs, media, service, store::Store, workflow};
use agentcut_core::{Asset, Project, RoundingMode};
use agentcut_render::FfmpegBackend;
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    path::{Path, PathBuf},
};

const MAX_SOURCE_MS: i64 = 86_400_000;
const MAX_WINDOW_MS: i64 = 300_000;
const MAX_WINDOWS: u32 = 20;
const MAX_CUES_PER_WINDOW: usize = 20;
const MAX_CANDIDATES: usize = 20;
const SEARCH_TASK: &str = "moment-search-v1";
pub const PROVIDER_KEY_PREFIX: &str = "avw-search-v1-";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn default_window() -> u32 {
    30_000
}
fn default_limit() -> u32 {
    5
}
fn default_context() -> u32 {
    1_000
}
fn default_search_limit() -> u32 {
    5
}

/// A source-time range divided into pages for an external agent to inspect.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceQuery {
    pub asset_id: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default = "default_window")]
    pub window_ms: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// Distinguish text verified against the active transcript from an agent's own
/// interpretation of video or audio, which AVW cannot independently verify.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Evidence {
    TranscriptCue {
        transcript_sha256: String,
        cue_id: String,
    },
    Observation {
        modality: Modality,
        description: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Modality {
    Video,
    Audio,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub output_id: String,
    pub asset_id: String,
    pub source_sha256: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
    pub reason: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HighlightBrief {
    pub objective: String,
    #[serde(default)]
    pub target_duration_ms: Option<u32>,
    #[serde(default = "default_context")]
    pub context_ms: u32,
    pub candidates: Vec<Candidate>,
}

/// Search a source using an explicitly configured external provider. The
/// provider receives the original source path under the existing contract.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSearch {
    pub asset_id: String,
    pub source_sha256: String,
    pub query: String,
    #[serde(default)]
    pub modality: SearchModality,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default = "default_search_limit")]
    pub max_results: u32,
    #[serde(default)]
    pub target_duration_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SearchModality {
    Speech,
    Visual,
    #[default]
    Multimodal,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchParameters {
    schema_version: u32,
    source_sha256: String,
    query: String,
    modality: SearchModality,
    start_ms: i64,
    end_ms: i64,
    max_results: u32,
    #[serde(default)]
    target_duration_ms: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCandidate {
    id: String,
    start_ms: i64,
    end_ms: i64,
    title: String,
    reason: String,
    #[serde(default)]
    confidence: Option<f64>,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderResults {
    schema_version: u32,
    query: String,
    candidates: Vec<ProviderCandidate>,
}

fn check_revision(project: &Project, expected: u64) -> Result<()> {
    if project.revision != expected {
        return Err(Error::Conflict {
            expected,
            current: project.revision,
        });
    }
    Ok(())
}

fn source<'a>(project: &'a Project, id: &str) -> Result<(&'a Asset, &'a str, i64)> {
    workflow::id(id)?;
    let asset = project.require_asset(id)?;
    if asset.metadata.video.is_none() {
        return Err(invalid("discovery requires a video source"));
    }
    let sha = asset
        .fingerprint
        .sha256
        .as_deref()
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("discovery source has no SHA-256 identity"))?;
    let duration = asset
        .metadata
        .duration
        .ok_or_else(|| invalid("discovery source duration is unknown"))?
        .rescale_to(workflow::time(0).rate, RoundingMode::Floor)?
        .value;
    if duration <= 0 {
        return Err(invalid("discovery source has no positive duration"));
    }
    Ok((asset, sha, duration))
}

fn active_transcript<'a>(
    transcripts: &'a [workflow::Transcript],
    asset_id: &str,
) -> Result<Option<(&'a workflow::Transcript, String)>> {
    let mut matches = transcripts.iter().filter(|t| t.asset_id == asset_id);
    let Some(transcript) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(invalid(
            "source has multiple active transcripts; select one before discovery",
        ));
    }
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, transcript)?;
    let sha = format!("{:x}", writer.0.finalize());
    Ok(Some((transcript, sha)))
}

fn preview(
    project: &Project,
    project_path: &Path,
    backend: &FfmpegBackend,
    asset_id: &str,
    start: i64,
    end: i64,
) -> Result<Value> {
    let task = analysis::Task::Proxy {
        asset_id: asset_id.to_owned(),
        start_ms: start,
        end_ms: end,
        width: 1280,
        height: 1280,
    };
    let key = service::preview_job_key(backend, task.clone(), project.revision, project)?;
    Ok(
        json!({"command":"analyze-start","project":project_path,"task":task,
        "expectedRevision":project.revision,"key":key,"noLaunch":true,"priority":0}),
    )
}

fn validate_range(start: i64, end: i64, source_end: i64, maximum: i64) -> Result<()> {
    if start < 0 || end <= start || end > source_end || end > MAX_SOURCE_MS || end - start > maximum
    {
        return Err(invalid(
            "discovery range must be nonempty, in the source and within its duration limit",
        ));
    }
    Ok(())
}

fn validate_search_parameters(
    project: &Project,
    asset_id: &str,
    parameters: &SearchParameters,
) -> Result<()> {
    let (_, source_sha, source_end) = source(project, asset_id)?;
    if parameters.schema_version != 1
        || parameters.source_sha256 != source_sha
        || !text_valid(&parameters.query, 2_000)
        || !(1..=20).contains(&parameters.max_results)
        || parameters
            .target_duration_ms
            .is_some_and(|v| v == 0 || v > 3_600_000)
    {
        return Err(invalid(
            "provider search requires schemaVersion 1, matching source hash, a query, maxResults 1..20 and optional targetDurationMs 1..3600000",
        ));
    }
    validate_range(
        parameters.start_ms,
        parameters.end_ms,
        source_end,
        MAX_SOURCE_MS,
    )
}

/// Bind a reserved provider-search key to the entire immutable job input. The
/// queue rechecks this key so copied requests cannot change provider/backend.
pub fn provider_input_key(input: &jobs::RenderInput, project: &Project) -> Result<String> {
    check_revision(project, input.expected_revision)?;
    if input.sequence != "seq_main" || input.priority != 0 || input.asr.is_some() {
        return Err(invalid(
            "generated provider search requires the original search job input",
        ));
    }
    let analysis::Input { task, provider } = input
        .analysis
        .as_ref()
        .ok_or_else(|| invalid("provider search analysis input is missing"))?;
    let provider = provider
        .as_ref()
        .ok_or_else(|| invalid("external analysis provider is not configured"))?;
    if provider.sha256.len() != 64 || !provider.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(
            "provider search executable has no SHA-256 identity",
        ));
    }
    let analysis::Task::Provider {
        asset_id,
        task,
        parameters,
    } = task
    else {
        return Err(invalid(
            "generated provider search key requires a provider task",
        ));
    };
    if task != SEARCH_TASK {
        return Err(invalid(
            "generated provider search key requires moment-search-v1",
        ));
    }
    let parameters: SearchParameters = serde_json::from_value(parameters.clone())?;
    validate_search_parameters(project, asset_id, &parameters)?;
    let (_, source_sha, _) = source(project, asset_id)?;
    Ok(format!(
        "{PROVIDER_KEY_PREFIX}{:x}",
        Sha256::digest(serde_json::to_vec(&json!({
            "input":input,"projectId":project.project_id,"sourceSha256":source_sha
        }))?)
    ))
}

/// Return an executable queued-analysis request. This function never starts a
/// provider or transfers media; invoking the returned request does so.
pub fn provider_search_request(
    project: &Project,
    project_path: &Path,
    backend: &FfmpegBackend,
    provider: Option<&analysis::Provider>,
    expected: u64,
    search: &ProviderSearch,
) -> Result<Value> {
    check_revision(project, expected)?;
    let provider =
        provider.ok_or_else(|| invalid("external analysis provider is not configured"))?;
    let (_, source_sha, source_end) = source(project, &search.asset_id)?;
    let end = search.end_ms.unwrap_or(source_end.min(MAX_SOURCE_MS));
    let params = SearchParameters {
        schema_version: 1,
        source_sha256: search.source_sha256.clone(),
        query: search.query.clone(),
        modality: search.modality,
        start_ms: search.start_ms,
        end_ms: end,
        max_results: search.max_results,
        target_duration_ms: search.target_duration_ms,
    };
    validate_search_parameters(project, &search.asset_id, &params)?;
    let task = analysis::Task::Provider {
        asset_id: search.asset_id.clone(),
        task: SEARCH_TASK.into(),
        parameters: serde_json::to_value(&params)?,
    };
    let analysis = analysis::Input {
        task: task.clone(),
        provider: Some(provider.clone()),
    };
    analysis::validate(&analysis, project)?;
    let input = jobs::RenderInput {
        sequence: "seq_main".into(),
        priority: 0,
        expected_revision: expected,
        ffmpeg: backend.ffmpeg_path().into(),
        ffprobe: backend.ffprobe_path().into(),
        asr: None,
        analysis: Some(analysis),
    };
    let key = provider_input_key(&input, project)?;
    Ok(
        json!({"revision":project.revision,"assetId":search.asset_id,"sourceSha256":source_sha,
        "scopeStartMs":search.start_ms,"scopeEndMs":end,"scopeLimitedBy24HourMaximum":search.end_ms.is_none() && source_end>MAX_SOURCE_MS,
        "providerSha256":provider.sha256,"analysisRequest":{"command":"analyze-start","project":project_path,
            "expectedRevision":expected,"key":key,"task":task,"noLaunch":true,"priority":0},
        "providerReceives":"original source path, source metadata and search parameters",
        "guidance":"No provider runs until you explicitly submit analysisRequest. The configured executable controls any subsequent network transfer; provider results require review."}),
    )
}

fn provider_result_path(root: &Path, result: &Value) -> Result<PathBuf> {
    let key = result["analysisKey"]
        .as_str()
        .filter(|key| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("provider analysis key is invalid"))?;
    let root = root.canonicalize()?;
    // Analysis workers publish into this exact directory. Comparing a
    // canonical actual path against the unsymlinked expected path also
    // refuses a symlink that redirects the cache to another project file.
    let expected = root.join("analysis").join(key).join("result.json");
    let raw = Path::new(
        result["path"]
            .as_str()
            .ok_or_else(|| invalid("provider result path missing"))?,
    );
    let actual = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    }
    .canonicalize()?;
    if expected != actual || !actual.is_file() {
        return Err(invalid(
            "provider result is outside its project analysis directory",
        ));
    }
    Ok(actual)
}

/// Validate a completed provider search, then convert its source-bound
/// suggestions into reviewable previews and plan requests.
pub fn provider_search_results(
    root: &Path,
    project: &Project,
    project_path: &Path,
    backend: &FfmpegBackend,
    expected: u64,
    job_id: &str,
    output_prefix: &str,
) -> Result<Value> {
    check_revision(project, expected)?;
    workflow::id(output_prefix)?;
    if output_prefix.len() > 64
        || !output_prefix
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return Err(invalid(
            "output prefix must start with a letter and be at most 64 bytes",
        ));
    }
    let store = Store::open(root)?;
    if store.project()? != *project {
        return Err(invalid("provider search project snapshot changed"));
    }
    let job = store.job(job_id)?;
    if job.state != "succeeded" || u64::try_from(job.revision).ok() != Some(expected) {
        return Err(invalid(
            "provider search job is not succeeded at the requested revision",
        ));
    }
    let input = job
        .input
        .as_ref()
        .ok_or_else(|| invalid("provider job input missing"))?;
    let generated_key = provider_input_key(input, project)?;
    let recorded_job: Option<String> = store
        .conn
        .query_row(
            "SELECT job_id FROM job_requests WHERE key=?1",
            [&generated_key],
            |row| row.get(0),
        )
        .optional()?;
    if recorded_job.as_deref() != Some(job_id) {
        return Err(invalid(
            "provider search job is not bound to its generated request key",
        ));
    }
    let analysis = input
        .analysis
        .as_ref()
        .ok_or_else(|| invalid("provider analysis input missing"))?;
    let provider = analysis
        .provider
        .as_ref()
        .ok_or_else(|| invalid("provider identity missing"))?;
    let analysis::Task::Provider {
        asset_id,
        task,
        parameters,
    } = &analysis.task
    else {
        return Err(invalid("job is not a provider search"));
    };
    if task != SEARCH_TASK {
        return Err(invalid("job is not a moment-search-v1 provider search"));
    }
    let parameters: SearchParameters = serde_json::from_value(parameters.clone())?;
    validate_search_parameters(project, asset_id, &parameters)?;
    let result = job
        .result
        .as_ref()
        .ok_or_else(|| invalid("provider result missing"))?;
    if result["mimeType"] != "application/json" {
        return Err(invalid("provider result has wrong media type"));
    }
    let expected_sha = result["sha256"]
        .as_str()
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("provider result SHA-256 missing"))?;
    let path = provider_result_path(root, result)?;
    if media::hash_file(&path)? != expected_sha {
        return Err(invalid("provider result bytes changed"));
    }
    let report: Value = crate::json::read(&path)?;
    if media::hash_file(&path)? != expected_sha {
        return Err(invalid("provider result bytes changed during reading"));
    }
    if report["schemaVersion"] != 1
        || report["sourceSha256"] != parameters.source_sha256
        || report["task"] != serde_json::to_value(&analysis.task)?
        || report["analysisKey"] != result["analysisKey"]
        || report["data"]["schemaVersion"] != 1
        || report["data"]["kind"] != "analysis"
        || report["data"]["sourceSha256"] != parameters.source_sha256
    {
        return Err(invalid(
            "provider result does not match its frozen search and source",
        ));
    }
    let found: ProviderResults = serde_json::from_value(report["data"]["data"].clone())?;
    if found.schema_version != 1
        || found.query != parameters.query
        || found.candidates.len() > parameters.max_results as usize
    {
        return Err(invalid(
            "provider search output does not match its query, version or result limit",
        ));
    }
    let identity = json!({"jobId":job_id,"jobKey":generated_key,"programSha256":provider.sha256,
        "resultSha256":expected_sha,"sourceSha256":parameters.source_sha256,"assetId":asset_id,
        "query":parameters.query,"modality":parameters.modality,"scopeStartMs":parameters.start_ms,"scopeEndMs":parameters.end_ms,
        "targetDurationMs":parameters.target_duration_ms});
    if found.candidates.is_empty() {
        return Ok(
            json!({"revision":expected,"status":"no-proposals","interpretation":"provider-supplied","provider":identity,
            "items":[],"guidance":"The provider returned no proposals. This does not establish that matching moments are absent."}),
        );
    }
    let mut provider_ids = BTreeSet::new();
    let mut candidates = Vec::with_capacity(found.candidates.len());
    for (index, candidate) in found.candidates.iter().enumerate() {
        workflow::id(&candidate.id)?;
        if !provider_ids.insert(&candidate.id)
            || candidate.start_ms < parameters.start_ms
            || candidate.end_ms > parameters.end_ms
        {
            return Err(invalid(
                "provider candidate IDs must be unique and ranges must stay within the requested source scope",
            ));
        }
        candidates.push(Candidate {
            id: format!("{output_prefix}-cut-{}", index + 1),
            output_id: format!("{output_prefix}-{}", index + 1),
            asset_id: asset_id.clone(),
            source_sha256: parameters.source_sha256.clone(),
            start_ms: candidate.start_ms,
            end_ms: candidate.end_ms,
            title: candidate.title.clone(),
            reason: candidate.reason.clone(),
            confidence: candidate.confidence,
            evidence: candidate.evidence.clone(),
        });
    }
    let brief = HighlightBrief {
        objective: parameters.query.clone(),
        target_duration_ms: parameters.target_duration_ms,
        context_ms: default_context(),
        candidates,
    };
    let mut prepared = prepare(project, project_path, backend, expected, &brief)?;
    prepared["interpretation"] = json!("provider-supplied");
    prepared["provider"] = identity;
    if let Some(items) = prepared["items"].as_array_mut() {
        for (item, candidate) in items.iter_mut().zip(found.candidates) {
            item["providerCandidateId"] = json!(candidate.id);
            if let Some(map) = item.as_object_mut() {
                let confidence = map
                    .remove("callerReportedConfidence")
                    .unwrap_or(Value::Null);
                map.insert("providerReportedConfidence".into(), confidence);
            }
            if let Some(evidence) = item["evidence"].as_array_mut() {
                for item in evidence {
                    if item["verification"] == "caller-asserted" {
                        item["verification"] = json!("provider-asserted");
                    }
                }
            }
        }
    }
    prepared["guidance"] = json!(
        "These are provider suggestions, not verified facts or proof of quality. Review the original media and speech boundaries before applying a plan."
    );
    Ok(prepared)
}

/// Page chronological source windows and bounded transcript excerpts. No
/// keyword ranking, semantic inference or automatic editing is performed.
pub fn evidence(
    project: &Project,
    project_path: &Path,
    backend: &FfmpegBackend,
    expected: u64,
    query: &EvidenceQuery,
) -> Result<Value> {
    check_revision(project, expected)?;
    let (_, source_sha, source_end) = source(project, &query.asset_id)?;
    let end = query.end_ms.unwrap_or(source_end.min(MAX_SOURCE_MS));
    let window = i64::from(query.window_ms);
    if !(1_000..=MAX_WINDOW_MS).contains(&window) || !(1..=MAX_WINDOWS).contains(&query.limit) {
        return Err(invalid(
            "discovery windows require windowMs 1000..300000 and limit 1..20",
        ));
    }
    validate_range(query.start_ms, end, source_end, MAX_SOURCE_MS)?;
    let total = (end - query.start_ms + window - 1) / window;
    if i64::from(query.offset) > total {
        return Err(invalid("discovery offset exceeds the number of windows"));
    }
    let transcripts = workflow::transcripts(project)?;
    let active = active_transcript(&transcripts, &query.asset_id)?;
    let mut cues = active
        .as_ref()
        .map(|(t, _)| t.cues.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    cues.sort_by(|a, b| (a.start_ms, a.end_ms, &a.id).cmp(&(b.start_ms, b.end_ms, &b.id)));
    let mut windows = Vec::new();
    for index in
        i64::from(query.offset)..(i64::from(query.offset) + i64::from(query.limit)).min(total)
    {
        let start = query.start_ms + index * window;
        let end = (start + window).min(end);
        let matches: Vec<_> = cues
            .iter()
            .copied()
            .filter(|cue| cue.start_ms < end && cue.end_ms > start)
            .collect();
        windows.push(json!({"index":index,"sourceStartMs":start,"sourceEndMs":end,
            "cueExcerpts":matches.iter().take(MAX_CUES_PER_WINDOW).collect::<Vec<_>>(),
            "cueExcerptsTotal":matches.len(),"cueExcerptsTruncated":matches.len()>MAX_CUES_PER_WINDOW,
            "previewRequest":preview(project,project_path,backend,&query.asset_id,start,end)?}));
    }
    let next = i64::from(query.offset)
        + i64::try_from(windows.len()).map_err(|_| invalid("too many discovery windows"))?;
    Ok(
        json!({"revision":project.revision,"assetId":query.asset_id,"sourceSha256":source_sha,
        "transcript":active.as_ref().map(|(t,sha)|json!({"sha256":sha,"language":t.language,"provider":t.provider})),
        "transcriptStatus":if active.is_some(){"available"}else{"unavailable"},
        "scopeStartMs":query.start_ms,"scopeEndMs":end,"sourceDurationMsFloor":source_end,
        "sourceContinuesAfterScope":end<source_end,"scopeLimitedBy24HourMaximum":query.end_ms.is_none() && source_end>MAX_SOURCE_MS,
        "windowMs":window,"totalWindows":total,
        "items":windows,"nextOffset":if next<total {Some(next)}else{None},
        "guidance":"Chronological source evidence, not ranked highlights. Review the source preview and transcript; no captions, transcript or preview establishes complete or accurate content understanding.",
        "integrity":"The source hash is recorded in the project; proxy jobs verify the source bytes before use."}),
    )
}

fn text_valid(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum
}

/// Validate external suggestions and produce preview and editorial planning
/// requests without mutating the project or claiming the suggestions are true.
pub fn prepare(
    project: &Project,
    project_path: &Path,
    backend: &FfmpegBackend,
    expected: u64,
    brief: &HighlightBrief,
) -> Result<Value> {
    check_revision(project, expected)?;
    if !text_valid(&brief.objective, 2_000)
        || !(250..=5_000).contains(&brief.context_ms)
        || brief
            .target_duration_ms
            .is_some_and(|v| v == 0 || v > 3_600_000)
        || brief.candidates.is_empty()
        || brief.candidates.len() > MAX_CANDIDATES
    {
        return Err(invalid(
            "highlight brief requires an objective, 1..20 candidates, contextMs 250..5000 and optional targetDurationMs 1..3600000",
        ));
    }
    let mut ids = BTreeSet::new();
    let transcripts = workflow::transcripts(project)?;
    let mut active_cache = BTreeMap::new();
    let mut items = Vec::with_capacity(brief.candidates.len());
    for candidate in &brief.candidates {
        workflow::id(&candidate.id)?;
        workflow::id(&candidate.output_id)?;
        if !ids.insert(&candidate.id) || !ids.insert(&candidate.output_id) {
            return Err(invalid(
                "highlight candidates and output IDs must be distinct",
            ));
        }
        if !text_valid(&candidate.title, 200)
            || !text_valid(&candidate.reason, 1000)
            || candidate
                .confidence
                .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            || candidate.evidence.is_empty()
            || candidate.evidence.len() > 20
        {
            return Err(invalid(
                "each candidate needs title, reason, 1..20 evidence items and optional confidence in 0..1",
            ));
        }
        let (_, source_sha, source_end) = source(project, &candidate.asset_id)?;
        if candidate.source_sha256 != source_sha {
            return Err(invalid(format!(
                "candidate {} source hash does not match the project",
                candidate.id
            )));
        }
        validate_range(
            candidate.start_ms,
            candidate.end_ms,
            source_end,
            MAX_WINDOW_MS,
        )?;
        if candidate
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::TranscriptCue { .. }))
            && !active_cache.contains_key(&candidate.asset_id)
        {
            active_cache.insert(
                candidate.asset_id.clone(),
                active_transcript(&transcripts, &candidate.asset_id)?,
            );
        }
        let active = active_cache
            .get(&candidate.asset_id)
            .and_then(Option::as_ref);
        let mut references = Vec::new();
        let mut cue_ids = BTreeSet::new();
        for evidence in &candidate.evidence {
            match evidence {
                Evidence::TranscriptCue {
                    transcript_sha256,
                    cue_id,
                } => {
                    if !cue_ids.insert(cue_id) {
                        return Err(invalid("candidate repeats a transcript cue"));
                    }
                    let (transcript, sha) = active.ok_or_else(|| {
                        invalid("candidate names a cue but the source has no active transcript")
                    })?;
                    if sha != transcript_sha256 {
                        return Err(invalid("candidate transcript hash is stale"));
                    }
                    let cue = transcript
                        .cues
                        .iter()
                        .find(|cue| &cue.id == cue_id)
                        .ok_or_else(|| {
                            invalid("candidate cue is absent from the active source transcript")
                        })?;
                    if cue.start_ms >= candidate.end_ms || cue.end_ms <= candidate.start_ms {
                        return Err(invalid("candidate cue does not overlap its source range"));
                    }
                    references.push(json!({"kind":"transcript-cue","transcriptSha256":sha,"cue":cue,
                        "verification":"active-transcript-reference","limitation":"Cue text/timing require listening and editorial review."}));
                }
                Evidence::Observation {
                    modality,
                    description,
                } => {
                    if !text_valid(description, 1000) {
                        return Err(invalid("observation description requires 1..1000 bytes"));
                    }
                    references.push(json!({"kind":"observation","modality":modality,"description":description,
                        "verification":"caller-asserted","limitation":"AVW has not independently verified this observation."}));
                }
            }
        }
        let duration = candidate.end_ms - candidate.start_ms;
        let budget = MAX_WINDOW_MS - duration;
        let before = i64::from(brief.context_ms)
            .min(candidate.start_ms)
            .min(budget / 2);
        let after = i64::from(brief.context_ms)
            .min(source_end - candidate.end_ms)
            .min(budget - before);
        let before = i64::from(brief.context_ms)
            .min(candidate.start_ms)
            .min(budget - after);
        let preview = preview(
            project,
            project_path,
            backend,
            &candidate.asset_id,
            candidate.start_ms - before,
            candidate.end_ms + after,
        )?;
        let plan_value = json!({"edit":{"outputId":candidate.output_id,"name":candidate.title,
            "cuts":[{"id":candidate.id,"assetId":candidate.asset_id,"startMs":candidate.start_ms,"endMs":candidate.end_ms}]},
            "brief":{"objective":brief.objective,"contextMs":brief.context_ms,"targetDurationMs":brief.target_duration_ms},
            "decisions":[{"cutId":candidate.id,"reason":candidate.reason}]});
        let plan: editorial::EditPlan = serde_json::from_value(plan_value.clone())?;
        let (plan_request, plan_status, reason) = match editorial::plan(project, &plan, expected) {
            Ok(report) => (
                json!({"command":"plan-edit","project":project_path,"expectedRevision":expected,"plan":plan_value}),
                json!({"readyToApply":report["readyToApply"],"unreviewedRisks":report["unreviewedRisks"],
                    "targetDurationDeltaMs":report["targetDurationDeltaMs"]}),
                Value::Null,
            ),
            Err(error) => (Value::Null, Value::Null, json!(error.to_string())),
        };
        items.push(json!({"id":candidate.id,"outputId":candidate.output_id,"title":candidate.title,"reason":candidate.reason,
            "assetId":candidate.asset_id,"sourceSha256":source_sha,"sourceStartMs":candidate.start_ms,"sourceEndMs":candidate.end_ms,
            "durationMs":duration,"callerReportedConfidence":candidate.confidence,"evidence":references,
            "previewRequest":preview,"planRequest":plan_request,"planningStatus":plan_status,"planningUnavailableReason":reason,
            "reviewRequired":true}));
    }
    Ok(
        json!({"revision":project.revision,"objective":brief.objective,"targetDurationMs":brief.target_duration_ms,
        "items":items,"status":"proposals","interpretation":"caller-supplied",
        "guidance":"AVW validates references, not meaning or quality. Listen and watch source context, review speech boundaries, and send planRequest to plan-edit before applying any candidate."}),
    )
}
