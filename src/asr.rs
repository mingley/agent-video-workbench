//! Local whisper.cpp process adapter. Analysis never mutates project history;
//! attaching reviewed cues is a separate atomic transcript-import request.
use crate::{
    Error, Result, media,
    process::{self, Control},
    workflow::{Cue, Transcript},
};
use agentcut_core::{Asset, Project};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub program: PathBuf,
    pub program_sha256: String,
    pub model: PathBuf,
    pub model_sha256: String,
    pub language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub asset_id: String,
    pub config: Config,
}

struct CacheContext<'a> {
    root: &'a Path,
    asset: &'a Asset,
    input: &'a Input,
    analysis_dir: PathBuf,
    scratch_dir: PathBuf,
}

const INCOMPLETE_CACHE_MESSAGE: &str = "ASR cache is incomplete or cannot be verified; restore its missing companion from a verified backup, or collect it only if no successful job references it";

fn incomplete_cache() -> Error {
    Error::Invalid(INCOMPLETE_CACHE_MESSAGE.into())
}

fn cache_file_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(Error::Invalid("ASR cache entries must be regular files".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn publish_new(source: &Path, destination: &Path) -> Result<()> {
    match std::fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(Error::Invalid(
            "ASR cache changed during publication; existing bytes were preserved".into(),
        )),
        Err(error) => Err(error.into()),
    }
}

fn cached(
    context: &CacheContext<'_>,
    cache: &Path,
    manifest_path: &Path,
    version: u64,
    control: &mut dyn Control,
) -> Result<Option<Value>> {
    let data_exists = cache_file_exists(cache)?;
    let manifest_exists = cache_file_exists(manifest_path)?;
    if data_exists != manifest_exists {
        return Err(incomplete_cache());
    }
    if !data_exists {
        return Ok(None);
    }
    let manifest: Value = crate::json::read(manifest_path)
        .map_err(|_| Error::Invalid("cached ASR manifest is malformed".into()))?;
    let hash = media::hash_file_controlled(cache, control)?;
    if manifest["sha256"].as_str() != Some(hash.as_str()) {
        return Err(Error::Invalid("cached transcript bytes changed".into()));
    }
    if manifest["sourceSha256"] != serde_json::to_value(&context.asset.fingerprint.sha256)?
        || manifest["provider"] != serde_json::to_value(&context.input.config)?
        || manifest["adapterVersion"].as_u64() != Some(version)
    {
        return Err(Error::Invalid("cached ASR manifest identity changed".into()));
    }
    if version == 3 && manifest["assetId"].as_str() != Some(context.input.asset_id.as_str()) {
        return Err(Error::Invalid("cached ASR manifest asset ID changed".into()));
    }
    let transcript: Transcript = crate::json::read(cache)
        .map_err(|_| Error::Invalid("cached ASR transcript is malformed".into()))?;
    if transcript.asset_id.as_str() != context.input.asset_id.as_str() {
        if version == 2 {
            return Ok(None);
        }
        return Err(Error::Invalid("cached ASR transcript asset ID changed".into()));
    }
    Ok(Some(
        json!({"path":cache,"manifest":manifest_path,"sha256":hash,"mimeType":"application/json","assetId":transcript.asset_id,"cacheHit":true,"cueCount":transcript.cues.len()}),
    ))
}

// Repair only a missing v2 sidecar backed by unchanged, historically successful
// jobs. Never rewrite the transcript; conflicting or missing evidence is refused.
fn repair_legacy_manifest(
    context: &CacheContext<'_>,
    cache: &Path,
    manifest_path: &Path,
    control: &mut dyn Control,
) -> Result<()> {
    let transcript: Transcript = crate::json::read(cache).map_err(|_| incomplete_cache())?;
    if transcript.asset_id.as_str() != context.input.asset_id.as_str() {
        return Err(incomplete_cache());
    }
    let hash = media::hash_file_controlled(cache, control)?;
    let source_sha = context
        .asset
        .fingerprint
        .sha256
        .as_deref()
        .filter(|sha| !sha.is_empty())
        .ok_or_else(incomplete_cache)?;
    let store = crate::store::Store::open(context.root)?;
    let mut found = false;
    for row in store.jobs()? {
        control.check()?;
        if row["state"] != "succeeded"
            || !row["result"]["path"]
                .as_str()
                .is_some_and(|path| Path::new(path) == cache)
        {
            continue;
        }
        found = true;
        if row["result"]["sha256"].as_str() != Some(hash.as_str())
            || row["result"]["assetId"].as_str() != Some(transcript.asset_id.as_str())
            || row["result"]["mimeType"] != "application/json"
        {
            return Err(incomplete_cache());
        }
        let id = row["id"].as_str().ok_or_else(incomplete_cache)?;
        let job = store.job(id)?;
        let prior = job
            .input
            .as_ref()
            .and_then(|input| input.asr.as_ref())
            .ok_or_else(incomplete_cache)?;
        if prior.asset_id.as_str() != transcript.asset_id.as_str()
            || serde_json::to_value(&prior.config)? != serde_json::to_value(&context.input.config)?
        {
            return Err(incomplete_cache());
        }
        let revision = u64::try_from(job.revision).map_err(|_| incomplete_cache())?;
        let project = store.revision(revision)?;
        let source = project
            .require_asset(&prior.asset_id)
            .map_err(|_| incomplete_cache())?;
        if source.fingerprint.sha256.as_deref() != Some(source_sha) {
            return Err(incomplete_cache());
        }
    }
    if !found {
        return Err(incomplete_cache());
    }
    let staging_dir = context
        .scratch_dir
        .join(format!("asr-repair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging_dir)?;
    let result = (|| -> Result<()> {
        let staged = staging_dir.join("manifest.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        file.write_all(&serde_json::to_vec(
            &json!({"sha256":hash,"sourceSha256":context.asset.fingerprint.sha256,"provider":context.input.config,"adapterVersion":2}),
        )?)?;
        file.sync_all()?;
        control.check()?;
        publish_new(&staged, manifest_path)?;
        File::open(&context.analysis_dir)?.sync_all()?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging_dir);
    result
}

pub fn analyze(
    root: &Path,
    project: &Project,
    input: &Input,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<Value> {
    control.check()?;
    if media::hash_file_controlled(&input.config.model, control)? != input.config.model_sha256 {
        return Err(Error::Invalid(
            "ASR model checksum does not match the configured SHA-256".into(),
        ));
    }
    if media::hash_file_controlled(&input.config.program, control)? != input.config.program_sha256 {
        return Err(Error::Invalid(
            "ASR executable changed since the job was queued".into(),
        ));
    }
    let asset = project.require_asset(&input.asset_id)?;
    media::verify_asset_controlled(root, asset, control)?;
    if asset.metadata.audio.is_none() {
        return Err(Error::Invalid("ASR requires a source audio stream".into()));
    }
    let duration = asset
        .metadata
        .duration
        .ok_or_else(|| Error::Invalid("source duration is unknown".into()))?
        .as_seconds_f64();
    if duration > 3600.0 {
        return Err(Error::Invalid("ASR input exceeds one hour".into()));
    }
    if crate::jobs::free_space(root)? < 256 * 1024 * 1024 {
        return Err(Error::Invalid("insufficient ASR scratch space".into()));
    }
    let context = CacheContext {
        root,
        asset,
        input,
        analysis_dir: media::project_directory(root, "analysis")?,
        scratch_dir: media::project_directory(root, "cache")?,
    };
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"asset":asset.fingerprint.sha256,"assetId":input.asset_id,"config":input.config,"version":3})
        )?)
    );
    let cache = context.analysis_dir.join(format!("asr-{key}.json"));
    let cache_manifest = cache.with_extension("manifest.json");
    if let Some(hit) = cached(&context, &cache, &cache_manifest, 3, control)? {
        return Ok(hit);
    }

    // Reuse the old source-only key only when its transcript belongs to this
    // asset. Identical bytes imported under another asset ID need a v3 entry.
    let legacy_key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"asset":asset.fingerprint.sha256,"config":input.config,"version":2})
        )?)
    );
    let legacy = context.analysis_dir.join(format!("asr-{legacy_key}.json"));
    let legacy_manifest = legacy.with_extension("manifest.json");
    let legacy_data_exists = cache_file_exists(&legacy)?;
    let legacy_manifest_exists = cache_file_exists(&legacy_manifest)?;
    let use_legacy = match (legacy_data_exists, legacy_manifest_exists) {
        (true, false) => match repair_legacy_manifest(&context, &legacy, &legacy_manifest, control) {
            Ok(()) => true,
            // A legacy entry with insufficient or conflicting evidence stays
            // untouched; the independent v3 cache can still serve this asset.
            Err(Error::Invalid(message)) if message == INCOMPLETE_CACHE_MESSAGE => false,
            Err(error) => return Err(error),
        },
        (false, true) => false,
        _ => true,
    };
    if use_legacy && let Some(hit) = cached(&context, &legacy, &legacy_manifest, 2, control)? {
        return Ok(hit);
    }
    let scratch = context
        .scratch_dir
        .join(format!("asr-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&scratch)?;
    let result = (|| {
        let audio = scratch.join("source.wav");
        process::run(
            Command::new(backend.ffmpeg_path())
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-protocol_whitelist",
                    "file",
                    "-i",
                ])
                .arg(root.join(&asset.uri))
                .args(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
                .arg(&audio),
            Duration::from_secs(3600),
            control,
        )?;
        let prefix = scratch.join("transcript");
        process::run(
            Command::new(&input.config.program)
                .arg("-m")
                .arg(&input.config.model)
                .arg("-f")
                .arg(&audio)
                .args(["-l", &input.config.language, "-t", "2", "-oj", "-of"])
                .arg(&prefix),
            Duration::from_secs(4 * 3600),
            control,
        )?;
        let raw: Value = crate::json::read(&prefix.with_extension("json"))?;
        let segments = raw["transcription"]
            .as_array()
            .ok_or_else(|| Error::Invalid("ASR provider omitted transcription segments".into()))?;
        if segments.len() > 20_000 {
            return Err(Error::Invalid("ASR exceeded 20000 cues".into()));
        }
        let mut cues = Vec::new();
        for (n, segment) in segments.iter().enumerate() {
            let start = segment["offsets"]["from"]
                .as_i64()
                .ok_or_else(|| Error::Invalid("ASR cue has no start offset".into()))?;
            let raw_end = segment["offsets"]["to"]
                .as_i64()
                .ok_or_else(|| Error::Invalid("ASR cue has no end offset".into()))?;
            let text = segment["text"]
                .as_str()
                .ok_or_else(|| Error::Invalid("ASR cue has no text".into()))?
                .trim();
            let end = raw_end.min((duration * 1000.0).floor() as i64);
            if start < 0
                || end <= start
                || raw_end as f64 > duration * 1000.0 + 1000.0
                || text.len() > 2000
            {
                return Err(Error::Invalid(
                    "ASR provider returned an invalid cue".into(),
                ));
            }
            if !text.is_empty() {
                cues.push(Cue {
                    id: format!("asr_{n:06}"),
                    start_ms: start,
                    end_ms: end,
                    text: text.into(),
                });
            }
        }
        let transcript = Transcript {
            asset_id: input.asset_id.clone(),
            language: input.config.language.clone(),
            provider: format!(
                "whisper.cpp binary SHA256={} model SHA256={}; machine transcript requires review",
                input.config.program_sha256, input.config.model_sha256
            ),
            cues,
        };
        control.stage("verifying")?;
        let staged = scratch.join("normalized.json");
        std::fs::write(&staged, serde_json::to_vec_pretty(&transcript)?)?;
        File::open(&staged)?.sync_all()?;
        let hash = media::hash_file_controlled(&staged, control)?;
        let staged_manifest = scratch.join("manifest.json");
        std::fs::write(
            &staged_manifest,
            serde_json::to_vec(
                &json!({"sha256":hash,"sourceSha256":asset.fingerprint.sha256,"assetId":input.asset_id,"provider":input.config,"adapterVersion":3}),
            )?,
        )?;
        File::open(&staged_manifest)?.sync_all()?;
        control.check()?;
        publish_new(&staged, &cache)?;
        publish_new(&staged_manifest, &cache_manifest)?;
        File::open(&context.analysis_dir)?.sync_all()?;
        Ok(
            json!({"path":cache,"manifest":cache_manifest,"sha256":hash,"mimeType":"application/json","assetId":input.asset_id,"cacheHit":false,"cueCount":transcript.cues.len()}),
        )
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}
