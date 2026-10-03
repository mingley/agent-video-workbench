//! Controlled sidechain processing and measured, reversible two-pass finishing.
use crate::{
    Error, Result,
    process::{self, Control},
};
use agentcut_render::{FfmpegBackend, compile::RenderPlan, ir::RenderIr};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, process::Command, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ducking {
    pub dialogue_items: Vec<String>,
    pub music_items: Vec<String>,
    pub threshold: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
}
pub fn validate(duck: &Ducking) -> Result<()> {
    if duck.dialogue_items.is_empty()
        || duck.music_items.is_empty()
        || duck.dialogue_items.len() > 100
        || duck.music_items.len() > 100
        || !(0.001..=1.0).contains(&duck.threshold)
        || !(1.0..=20.0).contains(&duck.ratio)
        || !(1.0..=2000.0).contains(&duck.attack_ms)
        || !(1.0..=9000.0).contains(&duck.release_ms)
    {
        return Err(Error::Invalid(
            "invalid sidechain items or compressor settings".into(),
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    if duck
        .dialogue_items
        .iter()
        .chain(&duck.music_items)
        .any(|id| !ids.insert(id))
    {
        return Err(Error::Invalid("sidechain item IDs must be distinct".into()));
    }
    Ok(())
}
pub fn adapt(
    plan: &mut RenderPlan,
    ir: &RenderIr,
    sequence: &agentcut_core::Sequence,
) -> Result<()> {
    let Some(value) = sequence.extensions.get("avw.ducking") else {
        return Ok(());
    };
    let duck: Ducking = serde_json::from_value(value.clone())?;
    validate(&duck)?;
    let position = plan
        .args
        .iter()
        .position(|a| a == "-filter_complex")
        .ok_or_else(|| Error::Invalid("audio graph missing".into()))?
        + 1;
    let graph = plan
        .args
        .get_mut(position)
        .ok_or_else(|| Error::Invalid("audio graph missing".into()))?;
    let find = |id: &String| {
        ir.audio_elements
            .iter()
            .position(|e| &e.item_id == id)
            .ok_or_else(|| Error::Invalid(format!("sidechain item {id} has no active audio")))
    };
    let dialogue: Vec<_> = duck
        .dialogue_items
        .iter()
        .map(find)
        .collect::<Result<_>>()?;
    let music: Vec<_> = duck.music_items.iter().map(find).collect::<Result<_>>()?;
    let mut chains: Vec<String> = graph.split(';').map(str::to_owned).collect();
    // Find producer suffixes only; consumer labels remain intact. User text
    // reaches side files, so it cannot introduce a filter separator here.
    for (i, n) in dialogue.iter().enumerate() {
        let suffix = format!("[a{n}]");
        let producer = chains
            .iter_mut()
            .find(|s| s.ends_with(&suffix))
            .ok_or_else(|| Error::Invalid("dialogue producer missing".into()))?;
        producer.truncate(producer.len() - suffix.len());
        producer.push_str(&format!("[avw_d{i}];[avw_d{i}]asplit=2[a{n}][avw_sc{i}]"));
    }
    for (i, n) in music.iter().enumerate() {
        let suffix = format!("[a{n}]");
        let producer = chains
            .iter_mut()
            .find(|s| s.ends_with(&suffix))
            .ok_or_else(|| Error::Invalid("music producer missing".into()))?;
        producer.truncate(producer.len() - suffix.len());
        producer.push_str(&format!("[avw_m{i}]"));
    }
    let inputs = (0..dialogue.len())
        .map(|i| format!("[avw_sc{i}]"))
        .collect::<String>();
    chains.push(format!(
        "{inputs}amix=inputs={}:normalize=0:dropout_transition=0[avw_sc]",
        dialogue.len()
    ));
    let branches = (0..music.len())
        .map(|i| format!("[avw_key{i}]"))
        .collect::<String>();
    chains.push(format!("[avw_sc]asplit={}{branches}", music.len()));
    for (i, n) in music.iter().enumerate() {
        chains.push(format!("[avw_m{i}][avw_key{i}]sidechaincompress=threshold={}:ratio={}:attack={}:release={}[a{n}]",duck.threshold,duck.ratio,duck.attack_ms,duck.release_ms));
    }
    *graph = chains.join(";");
    Ok(())
}
fn measurement(
    path: &Path,
    backend: &FfmpegBackend,
    target: f64,
    peak: f64,
    control: &mut dyn Control,
) -> Result<Value> {
    let output = process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-nostdin", "-v", "info", "-i"])
            .arg(path)
            .args([
                "-vn",
                "-af",
                &format!("loudnorm=I={target}:TP={peak}:LRA=11:print_format=json"),
                "-f",
                "null",
                "-",
            ]),
        Duration::from_secs(4 * 3600),
        control,
    )?;
    let start = output
        .stderr
        .rfind('{')
        .ok_or_else(|| Error::Invalid("loudness measurement missing".into()))?;
    let end = output.stderr[start..]
        .find('}')
        .ok_or_else(|| Error::Invalid("loudness measurement incomplete".into()))?
        + start
        + 1;
    crate::json::parse(&output.stderr.as_bytes()[start..end])
}
fn number(value: &Value, key: &str) -> Option<f64> {
    value[key]
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}
pub fn finish(
    path: &Path,
    backend: &FfmpegBackend,
    sequence: &agentcut_core::Sequence,
    control: &mut dyn Control,
) -> Result<Value> {
    let Some(profile) = sequence.extensions.get("avw.profile") else {
        return Ok(json!({"status":"not_run","reason":"no pinned loudness target"}));
    };
    let Some(target) = profile["loudnessLufs"].as_f64() else {
        return Ok(json!({"status":"not_run","reason":"no pinned loudness target"}));
    };
    let peak = profile["truePeakDb"].as_f64().unwrap_or(-1.0);
    let measured = measurement(path, backend, target, peak, control)?;
    let Some(input_i) = number(&measured, "input_i") else {
        return Ok(
            json!({"status":"not_run","reason":"silent output has no finite integrated loudness","measurement":measured}),
        );
    };
    let mut values = Vec::new();
    for key in ["input_tp", "input_lra", "input_thresh", "target_offset"] {
        values.push(
            number(&measured, key)
                .ok_or_else(|| Error::Invalid("nonfinite loudness measurement".into()))?,
        );
    }
    let staged = path.with_extension("normalized.mp4");
    let filter = format!(
        "loudnorm=I={target}:TP={peak}:LRA=11:measured_I={input_i}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
        values[0], values[1], values[2], values[3]
    );
    let result = (|| {
        process::run(
            Command::new(backend.ffmpeg_path())
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(path)
                .args([
                    "-map", "0:v:0", "-map", "0:a:0", "-c:v", "copy", "-af", &filter, "-ar",
                    "48000", "-c:a", "aac", "-b:a", "192k",
                ])
                .arg(&staged),
            Duration::from_secs(4 * 3600),
            control,
        )?;
        let verified = measurement(&staged, backend, target, peak, control)?;
        let actual = number(&verified, "input_i")
            .ok_or_else(|| Error::Invalid("finished audio is not measurable".into()))?;
        let actual_peak = number(&verified, "input_tp")
            .ok_or_else(|| Error::Invalid("finished peak is not measurable".into()))?;
        if (actual - target).abs() > 1.0 || actual_peak > peak + 0.3 {
            return Err(Error::Invalid(format!(
                "audio QC failed: {actual} LUFS, {actual_peak} dBTP"
            )));
        }
        std::fs::File::open(&staged)?.sync_all()?;
        std::fs::rename(&staged, path)?;
        Ok(
            json!({"status":"passed","method":"FFmpeg loudnorm two-pass","targetLufs":target,"targetTruePeakDb":peak,"before":measured,"after":verified,"integratedToleranceLu":1.0,"peakToleranceDb":0.3}),
        )
    })();
    if staged.exists() {
        std::fs::remove_file(staged)?;
    }
    result
}
