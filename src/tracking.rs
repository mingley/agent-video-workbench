//! CPU template tracking of an explicitly selected region, with editable holds.
use crate::{
    Error, Result,
    analysis::Task,
    process::{self, Control},
};
use agentcut_core::Asset;
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Point {
    pub source_ms: i64,
    pub center_x: f64,
    pub center_y: f64,
    pub confidence: f64,
    pub held: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub algorithm: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub source_sha256: String,
    pub region: Region,
    pub points: Vec<Point>,
    pub findings: Vec<String>,
    pub fallback: String,
}
pub fn validate(
    region: &Region,
    start: i64,
    end: i64,
    sample: i64,
    confidence: f64,
    motion: f64,
) -> Result<()> {
    if ![region.x, region.y, region.width, region.height]
        .iter()
        .all(|v| v.is_finite())
        || region.x < 0.0
        || region.y < 0.0
        || region.width < 0.02
        || region.height < 0.02
        || region.width > 0.6
        || region.height > 0.6
        || region.x + region.width > 1.0
        || region.y + region.height > 1.0
        || start < 0
        || end <= start
        || !(100..=5000).contains(&sample)
        || (end - start) / sample > 120
        || !(0.0..=1.0).contains(&confidence)
        || !(0.01..=0.5).contains(&motion)
    {
        return Err(Error::Invalid("tracking requires an in-frame region, 100..5000 ms samples, <=120 intervals, confidence 0..1 and motion fraction .01.. .5".into()));
    }
    Ok(())
}
fn frame(
    path: &Path,
    asset: &Asset,
    at: i64,
    width: usize,
    height: usize,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<Vec<u8>> {
    let color = crate::color::source_filter(asset)?;
    let filter = format!(
        "{}scale={width}:{height},format=gray",
        if color.is_empty() {
            color
        } else {
            format!("{color},")
        }
    );
    let output = process::run(
        Command::new(backend.ffmpeg_path())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-noautorotate",
                "-ss",
                &format!("{}", at as f64 / 1000.0),
                "-i",
            ])
            .arg(path)
            .args([
                "-vf",
                &filter,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "-",
            ]),
        Duration::from_secs(60),
        control,
    )?;
    if output.stdout.len() != width * height {
        return Err(Error::Invalid(
            "tracking frame dimensions differ from recipe".into(),
        ));
    }
    Ok(output.stdout)
}
fn patch(frame: &[u8], width: usize, x: usize, y: usize, pw: usize, ph: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(256);
    for row in 0..16 {
        for col in 0..16 {
            out.push(f64::from(
                frame[(y + row * (ph - 1) / 15) * width + x + col * (pw - 1) / 15],
            ));
        }
    }
    out
}
fn centered(values: &[f64]) -> (Vec<f64>, f64) {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let values: Vec<_> = values.iter().map(|v| v - mean).collect();
    let energy = values.iter().map(|v| v * v).sum::<f64>();
    (values, energy)
}
pub fn track(
    path: &Path,
    asset: &Asset,
    task: &Task,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<Proposal> {
    let Task::Track {
        start_ms,
        end_ms,
        region,
        sample_ms,
        min_confidence,
        max_motion_fraction,
        ..
    } = task
    else {
        return Err(Error::Invalid("tracking task required".into()));
    };
    validate(
        region,
        *start_ms,
        *end_ms,
        *sample_ms,
        *min_confidence,
        *max_motion_fraction,
    )?;
    let video = asset
        .metadata
        .video
        .as_ref()
        .ok_or_else(|| Error::Invalid("tracking requires video".into()))?;
    let size = crate::color::upright_size(video);
    let factor = (640.0 / f64::from(size.width))
        .min(640.0 / f64::from(size.height))
        .min(1.0);
    let width = (f64::from(size.width) * factor).round().max(16.0) as usize;
    let height = (f64::from(size.height) * factor).round().max(16.0) as usize;
    let pw = (region.width * width as f64).round().max(16.0) as usize;
    let ph = (region.height * height as f64).round().max(16.0) as usize;
    let mut x = (region.x * width as f64).round() as usize;
    let mut y = (region.y * height as f64).round() as usize;
    if x + pw > width || y + ph > height {
        return Err(Error::Invalid(
            "sampled tracking region extends outside frame".into(),
        ));
    }
    let first = frame(path, asset, *start_ms, width, height, backend, control)?;
    let (template, energy) = centered(&patch(&first, width, x, y, pw, ph));
    if energy < 256.0 {
        return Err(Error::Invalid("selected region has insufficient texture; use manual framing/keyframes or another provider".into()));
    }
    let mut points = Vec::new();
    let mut findings = Vec::new();
    let radius = (width.max(height) as f64 * max_motion_fraction).ceil() as usize;
    let mut at = *start_ms;
    while at < *end_ms {
        control.check()?;
        let mut best = (1.0, x, y);
        if at != *start_ms {
            let image = frame(path, asset, at, width, height, backend, control)?;
            best = (-1.0, x, y);
            let left = x.saturating_sub(radius);
            let right = (x + radius).min(width - pw);
            let top = y.saturating_sub(radius);
            let bottom = (y + radius).min(height - ph);
            for candidate_y in (top..=bottom).step_by(2) {
                control.check()?;
                for candidate_x in (left..=right).step_by(2) {
                    let (pixels, e) =
                        centered(&patch(&image, width, candidate_x, candidate_y, pw, ph));
                    let score = if e > 0.0 {
                        pixels
                            .iter()
                            .zip(&template)
                            .map(|(a, b)| a * b)
                            .sum::<f64>()
                            / (e * energy).sqrt()
                    } else {
                        0.0
                    };
                    let distance = candidate_x.abs_diff(x) + candidate_y.abs_diff(y);
                    let old_distance = best.1.abs_diff(x) + best.2.abs_diff(y);
                    if score > best.0 + 0.000001
                        || ((score - best.0).abs() < 0.000001 && distance < old_distance)
                    {
                        best = (score, candidate_x, candidate_y);
                    }
                }
            }
        }
        let held = best.0 < *min_confidence;
        if held {
            findings.push(format!(
                "source {at} ms: confidence below threshold; previous accepted position held"
            ));
        } else {
            (x, y) = (best.1, best.2);
        }
        points.push(Point {
            source_ms: at,
            center_x: (x as f64 + pw as f64 / 2.0) / width as f64,
            center_y: (y as f64 + ph as f64 / 2.0) / height as f64,
            confidence: best.0.clamp(0.0, 1.0),
            held,
        });
        at = at
            .checked_add(*sample_ms)
            .ok_or_else(|| Error::Invalid("tracking time overflow".into()))?;
    }
    Ok(Proposal {
        algorithm: "avw.gray-ncc-template.v1".into(),
        start_ms: *start_ms,
        end_ms: *end_ms,
        source_sha256: asset
            .fingerprint
            .sha256
            .clone()
            .ok_or_else(|| Error::Invalid("tracking source hash missing".into()))?,
        region: region.clone(),
        points,
        findings,
        fallback: "hold previous accepted region; review required".into(),
    })
}
