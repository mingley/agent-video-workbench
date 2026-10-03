//! Local whisper.cpp process adapter. Analysis never mutates project history;
//! attaching reviewed cues is a separate atomic transcript-import request.
use crate::{
    Error, Result, media,
    process::{self, Control},
    workflow::{Cue, Transcript},
};
use agentcut_core::Project;
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
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
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"asset":asset.fingerprint.sha256,"config":input.config,"version":2})
        )?)
    );
    let cache = root.join("analysis").join(format!("asr-{key}.json"));
    let cache_manifest = cache.with_extension("manifest.json");
    if cache.exists() && cache_manifest.exists() {
        let manifest: Value = crate::json::read(&cache_manifest)?;
        let hash = media::hash_file_controlled(&cache, control)?;
        if manifest["sha256"] != hash {
            return Err(Error::Invalid("cached transcript bytes changed".into()));
        }
        let transcript: Transcript = crate::json::read(&cache)?;
        return Ok(
            json!({"path":cache,"sha256":hash,"mimeType":"application/json","assetId":transcript.asset_id,"cacheHit":true,"cueCount":transcript.cues.len()}),
        );
    }
    let scratch = root
        .join("cache")
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
                &json!({"sha256":hash,"sourceSha256":asset.fingerprint.sha256,"provider":input.config,"adapterVersion":2}),
            )?,
        )?;
        File::open(&staged_manifest)?.sync_all()?;
        control.check()?;
        std::fs::rename(&staged, &cache)?;
        std::fs::rename(&staged_manifest, &cache_manifest)?;
        File::open(root.join("analysis"))?.sync_all()?;
        Ok(
            json!({"path":cache,"sha256":hash,"mimeType":"application/json","assetId":input.asset_id,"cacheHit":false,"cueCount":transcript.cues.len()}),
        )
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}
