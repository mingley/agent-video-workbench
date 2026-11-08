//! Local whisper.cpp process adapter. Analysis never mutates project history;
//! attaching reviewed cues is a separate atomic transcript-import request.
#[path = "asr_clock.rs"]
mod asr_clock;

use crate::{
    Error, Result, media,
    process::{self, Control},
    workflow::{Cue, Transcript},
};
use agentcut_core::{Asset, Project};
use agentcut_render::FfmpegBackend;
use asr_clock::{AudioClock, MAX_AUDIT_BYTES, MAX_WAV_BYTES, audit_audio_clock, probe_audio_clock};
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

struct CacheContext<'a> {
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
        Ok(_) => Err(Error::Invalid(
            "ASR cache entries must be regular files".into(),
        )),
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
        return Err(Error::Invalid(
            "cached ASR manifest identity changed".into(),
        ));
    }
    if manifest["assetId"].as_str() != Some(context.input.asset_id.as_str()) {
        return Err(Error::Invalid(
            "cached ASR manifest asset ID changed".into(),
        ));
    }
    let transcript: Transcript = crate::json::read(cache)
        .map_err(|_| Error::Invalid("cached ASR transcript is malformed".into()))?;
    if transcript.asset_id.as_str() != context.input.asset_id.as_str() {
        return Err(Error::Invalid(
            "cached ASR transcript asset ID changed".into(),
        ));
    }
    Ok(Some(
        json!({"path":cache,"manifest":manifest_path,"sha256":hash,"mimeType":"application/json","assetId":transcript.asset_id,"cacheHit":true,"cueCount":transcript.cues.len()}),
    ))
}

// Reject a decoded stream that cannot fit within the declared source clock,
// even if the provider would return no cues. The one-second allowance mirrors
// the existing provider-end tolerance for codec padding.
fn validate_audio_range(clock: &AudioClock, duration: f64) -> Result<()> {
    let invalid = || Error::Invalid("audio clock lies outside the source timeline".into());
    if !duration.is_finite() || duration <= 0.0 || clock.offset_ticks < 0 || clock.samples <= 0 {
        return Err(invalid());
    }
    let duration_samples = (duration * 16_000.0).ceil() as i128;
    let source_end = clock
        .offset_ticks
        .checked_add(clock.samples)
        .ok_or_else(invalid)?;
    if clock.offset_ticks >= duration_samples
        || source_end > duration_samples.checked_add(16_000).ok_or_else(invalid)?
    {
        return Err(invalid());
    }
    Ok(())
}

// Provider offsets describe positions in the continuous extracted WAV.
// Only after validating those against audited WAV samples may they be shifted
// into the source timeline, whose zero is the container origin.
fn map_cue_to_source(
    start_ms: i64,
    end_ms: i64,
    clock: &AudioClock,
    duration: f64,
) -> Result<(i64, i64)> {
    let invalid = || Error::Invalid("ASR cue is outside the verified audio clock".into());
    if start_ms < 0 || end_ms <= start_ms || clock.samples <= 0 || clock.offset_ticks < 0 {
        return Err(invalid());
    }
    let start = i128::from(start_ms).checked_mul(16).ok_or_else(invalid)?;
    let end = i128::from(end_ms).checked_mul(16).ok_or_else(invalid)?;
    let limit = clock.samples.checked_add(16_000).ok_or_else(invalid)?;
    if start >= clock.samples || end > limit {
        return Err(invalid());
    }
    let source_start = clock.offset_ticks.checked_add(start).ok_or_else(invalid)?;
    let source_end = clock
        .offset_ticks
        .checked_add(end.min(clock.samples))
        .ok_or_else(invalid)?;
    let duration_samples = (duration * 16_000.0).floor() as i128;
    if source_start >= duration_samples
        || source_end > duration_samples.checked_add(16_000).ok_or_else(invalid)?
    {
        return Err(invalid());
    }
    let start = source_start.checked_add(8).ok_or_else(invalid)? / 16;
    let end = source_end
        .min(duration_samples)
        .checked_add(8)
        .ok_or_else(invalid)?
        / 16;
    let duration_ms = (duration * 1000.0).floor() as i128;
    let end = end.min(duration_ms);
    if end <= start {
        return Err(invalid());
    }
    Ok((
        i64::try_from(start).map_err(|_| invalid())?,
        i64::try_from(end).map_err(|_| invalid())?,
    ))
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
    if !duration.is_finite() || duration <= 0.0 {
        return Err(Error::Invalid("source duration is invalid".into()));
    }
    if duration > 3600.0 {
        return Err(Error::Invalid("ASR input exceeds one hour".into()));
    }
    if crate::jobs::free_space(root)? < 256 * 1024 * 1024 {
        return Err(Error::Invalid("insufficient ASR scratch space".into()));
    }
    let context = CacheContext {
        asset,
        input,
        analysis_dir: media::project_directory(root, "analysis")?,
        scratch_dir: media::project_directory(root, "cache")?,
    };
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"asset":asset.fingerprint.sha256,"assetId":input.asset_id,"config":input.config,"version":4})
        )?)
    );
    let cache = context.analysis_dir.join(format!("asr-{key}.json"));
    let cache_manifest = cache.with_extension("manifest.json");
    if let Some(hit) = cached(&context, &cache, &cache_manifest, 4, control)? {
        return Ok(hit);
    }

    let scratch = context
        .scratch_dir
        .join(format!("asr-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&scratch)?;
    let result = (|| {
        let audio = scratch.join("source.wav");
        let audit = scratch.join("audio.framecrc");
        let source = root.join(&asset.uri);
        let probe = probe_audio_clock(&source, backend, control)?;
        let filters = format!(
            "[0:{}]asplit=2[src][first];[first]asettb=1/16000,aselect='eq(n,0)',ashowinfo@asr_clock,anullsink;[src]aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono,asplit=2[w][audit]",
            probe.stream_index
        );
        // Decode once: both the model WAV and frame-level clock audit come
        // from the same post-resample PCM frames. The audit is size-limited,
        // read as a stream, and checked before the model is invoked.
        let decoded = process::run(
            Command::new(backend.ffmpeg_path())
                .args([
                    "-nostdin",
                    "-hide_banner",
                    "-nostats",
                    "-v",
                    "info",
                    "-xerror",
                    "-copyts",
                    "-protocol_whitelist",
                    "file",
                    "-i",
                ])
                .arg(&source)
                .args(["-filter_complex", &filters])
                .args(["-map", "[w]", "-c:a", "pcm_s16le", "-fs"])
                .arg(MAX_WAV_BYTES.to_string())
                .args(["-f", "wav", "-n"])
                .arg(&audio)
                .args([
                    "-map",
                    "[audit]",
                    "-c:a",
                    "pcm_s16le",
                    "-avoid_negative_ts",
                    "disabled",
                    "-fs",
                ])
                .arg(MAX_AUDIT_BYTES.to_string())
                .args(["-f", "framecrc", "-n"])
                .arg(&audit),
            Duration::from_secs(3600),
            control,
        )?;
        let clock = audit_audio_clock(&audio, &audit, &decoded.stderr, &probe, backend, control)?;
        validate_audio_range(&clock, duration)?;
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
            if text.len() > 2000 {
                return Err(Error::Invalid(
                    "ASR provider returned an invalid cue".into(),
                ));
            }
            let (start, end) = map_cue_to_source(start, raw_end, &clock, duration)?;
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
                &json!({"sha256":hash,"sourceSha256":asset.fingerprint.sha256,"assetId":input.asset_id,"provider":input.config,"adapterVersion":4}),
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

#[cfg(test)]
mod clock_mapping_tests {
    use super::*;

    fn clock() -> AudioClock {
        AudioClock {
            offset_ticks: 16_000,
            samples: 48_000,
        }
    }

    #[test]
    fn maps_wav_cues_to_delayed_source_timeline() {
        assert_eq!(
            map_cue_to_source(250, 750, &clock(), 4.0).unwrap(),
            (1_250, 1_750)
        );
        assert_eq!(
            map_cue_to_source(2_500, 3_500, &clock(), 4.0).unwrap(),
            (3_500, 4_000)
        );
    }

    #[test]
    fn rejects_invalid_or_out_of_audio_ranges() {
        assert!(map_cue_to_source(-1, 10, &clock(), 4.0).is_err());
        assert!(map_cue_to_source(100, 100, &clock(), 4.0).is_err());
        assert!(map_cue_to_source(3_000, 3_001, &clock(), 4.0).is_err());
        assert!(map_cue_to_source(2_500, 4_001, &clock(), 4.0).is_err());
        assert!(map_cue_to_source(2_500, 2_900, &clock(), 3.0).is_err());
    }

    #[test]
    fn rejects_negative_or_overflowing_source_offsets() {
        let before_origin = AudioClock {
            offset_ticks: -1,
            samples: 48_000,
        };
        assert!(map_cue_to_source(0, 500, &before_origin, 4.0).is_err());
        let overflow = AudioClock {
            offset_ticks: i128::MAX,
            samples: 48_000,
        };
        assert!(map_cue_to_source(1, 500, &overflow, 4.0).is_err());
        assert!(map_cue_to_source(1, i64::MAX, &clock(), 4.0).is_err());
    }
    #[test]
    fn bounds_source_clock_before_transcription() {
        assert!(validate_audio_range(&clock(), 4.0).is_ok());
        assert!(validate_audio_range(&clock(), 2.0).is_err());
        assert!(validate_audio_range(&clock(), f64::NAN).is_err());
        assert!(
            validate_audio_range(
                &AudioClock {
                    offset_ticks: -1,
                    samples: 100
                },
                1.0
            )
            .is_err()
        );
        assert!(
            validate_audio_range(
                &AudioClock {
                    offset_ticks: 1,
                    samples: i128::MAX
                },
                1.0
            )
            .is_err()
        );
    }
}
