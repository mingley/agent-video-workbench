//! Reviewable crop paths from externally observed subject boxes.
//!
//! The observer, not this module, identifies people or objects. This module
//! verifies the frozen source, computes a crop that fits supplied boxes at
//! sampled times, and emits an editable existing reframe operation.
use crate::{Error, Result, assembly, media, tracking, workflow};
use agentcut_core::{Project, RoundingMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubjectSample {
    pub source_ms: i64,
    /// Boxes are normalized to the upright source after sample-aspect correction.
    pub regions: Vec<tracking::Region>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReframeRequest {
    pub schema_version: u32,
    pub item_id: String,
    pub source_sha256: String,
    /// Identity supplied by the observer; it is not independently authenticated.
    pub observer: String,
    /// Fraction of each output edge to keep clear around observed subjects.
    #[serde(default)]
    pub safe_inset: f64,
    #[serde(default = "default_confidence")]
    pub min_confidence: f64,
    #[serde(default = "default_gap")]
    pub max_gap_ms: u32,
    pub samples: Vec<SubjectSample>,
}

fn default_confidence() -> f64 {
    0.5
}
fn default_gap() -> u32 {
    2000
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn valid_region(r: &tracking::Region) -> bool {
    [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
        && r.x >= 0.0
        && r.y >= 0.0
        && r.width > 0.0
        && r.height > 0.0
        && r.x + r.width <= 1.0
        && r.y + r.height <= 1.0
}

/// Calculate crop centers without modifying the project or inferring subjects.
pub fn propose(
    root: &Path,
    project: &Project,
    expected: u64,
    request: &ReframeRequest,
) -> Result<Value> {
    if project.revision != expected {
        return Err(Error::Conflict {
            expected,
            current: project.revision,
        });
    }
    if request.schema_version != 1
        || request.observer.trim().is_empty()
        || request.observer.len() > 200
        || request.observer.chars().any(char::is_control)
        || !request.safe_inset.is_finite()
        || !(0.0..=0.25).contains(&request.safe_inset)
        || !request.min_confidence.is_finite()
        || !(0.0..=1.0).contains(&request.min_confidence)
        || !(100..=5000).contains(&request.max_gap_ms)
        || request.samples.is_empty()
        || request.samples.len() > 121
        || request
            .samples
            .windows(2)
            .any(|p| p[0].source_ms >= p[1].source_ms)
    {
        return Err(invalid(
            "reframe observations require schemaVersion 1, an observer, 1..121 ordered samples, safeInset 0..0.25, confidence 0..1 and maxGapMs 100..5000",
        ));
    }
    workflow::id(&request.item_id)?;
    let (sequence, track, item) = project
        .find_item(&request.item_id)
        .ok_or_else(|| invalid("reframe item not found"))?;
    if !track.track_type.is_visual()
        || !item.enabled
        || !track.enabled
        || track.locked
        || (sequence
            .tracks
            .iter()
            .any(|t| t.enabled && t.solo && t.track_type.is_visual())
            && !track.solo)
    {
        return Err(invalid(
            "reframing requires an enabled, unlocked item on an enabled visual track that is not excluded by solo",
        ));
    }
    let clip = item
        .payload
        .as_clip()
        .ok_or_else(|| invalid("reframing requires a video clip"))?;
    if !clip.video.speed.is_normal() {
        return Err(invalid("reframing requires normal-speed footage"));
    }
    if sequence.canvas.pixel_aspect_ratio.numerator == 0
        || sequence.canvas.pixel_aspect_ratio.numerator
            != sequence.canvas.pixel_aspect_ratio.denominator
        || !clip.transform.is_identity_placement(sequence.canvas)
        || !clip.video.crop.is_none()
        || clip.video.corner_radius != 0.0
        || clip.opacity != 1.0
        || clip.blend_mode != agentcut_core::BlendMode::Normal
        || !item.keyframes.is_empty()
        || item.effects.iter().any(|effect| effect.enabled)
        || track.effects.iter().any(|effect| effect.enabled)
        || sequence.transitions.iter().any(|transition| {
            transition.left_item_id == item.id || transition.right_item_id == item.id
        })
    {
        return Err(invalid(
            "subject-fit proposals require square-pixel canvas, centered untransformed uncropped opaque video, and no active item/track effects, keyframes or touching transitions",
        ));
    }
    if assembly::policy(sequence)?.is_some() {
        return Err(invalid(
            "source-preserving assemblies do not support cropping; create an independent composed output",
        ));
    }
    let asset = project.require_asset(&clip.asset_id)?;
    if asset.fingerprint.sha256.as_deref() != Some(request.source_sha256.as_str()) {
        return Err(invalid(
            "reframe observations do not match the selected source hash",
        ));
    }
    media::verify_asset(root, asset)?;
    let metadata = asset
        .metadata
        .video
        .as_ref()
        .ok_or_else(|| invalid("reframing requires video metadata"))?;
    if metadata.size.width == 0
        || metadata.size.height == 0
        || metadata.sample_aspect_ratio.numerator == 0
        || metadata.sample_aspect_ratio.denominator == 0
        || metadata.rotation_degrees.rem_euclid(90) != 0
        || sequence.canvas.width == 0
        || sequence.canvas.height == 0
    {
        return Err(invalid(
            "reframing requires known upright video and canvas geometry",
        ));
    }
    let source_start = clip.source_range.start;
    let source_end = clip.source_range.end_exclusive()?;
    let source_duration = asset
        .metadata
        .duration
        .ok_or_else(|| invalid("reframing requires a known source duration"))?;
    if !source_end.checked_cmp(source_duration)?.is_le() {
        return Err(invalid(
            "reframe clip extends beyond the known source duration",
        ));
    }
    let clock = workflow::time(0).rate;
    let start_ms = source_start.rescale_to(clock, RoundingMode::Floor)?.value;
    let end_ms = source_end.rescale_to(clock, RoundingMode::Ceil)?.value;
    if start_ms < 0 || end_ms <= start_ms || request.samples[0].source_ms != start_ms {
        return Err(invalid(
            "first subject sample must be at the floored source start of the clip",
        ));
    }
    for sample in &request.samples {
        if sample.source_ms < start_ms
            || !workflow::time(sample.source_ms)
                .checked_cmp(source_end)?
                .is_lt()
            || !sample.confidence.is_finite()
            || !(0.0..=1.0).contains(&sample.confidence)
            || sample.regions.len() > 8
            || sample.regions.iter().any(|r| !valid_region(r))
        {
            return Err(invalid(
                "subject samples require in-range times, 0..8 valid upright regions and confidence 0..1",
            ));
        }
    }
    let upright = crate::color::upright_size(metadata);
    let source_aspect = f64::from(upright.width) / f64::from(upright.height);
    let output_aspect = f64::from(sequence.canvas.width) / f64::from(sequence.canvas.height);
    let viewport_width = (output_aspect / source_aspect).min(1.0);
    let viewport_height = (source_aspect / output_aspect).min(1.0);
    let mut issues = Vec::new();
    let max_gap = i64::from(request.max_gap_ms);
    if request
        .samples
        .windows(2)
        .any(|p| p[1].source_ms - p[0].source_ms > max_gap)
        || end_ms.saturating_sub(request.samples.last().expect("nonempty").source_ms) > max_gap
    {
        issues.push(json!({"code":"sample-gap","message":"Subject observations do not cover the clip at the requested sampling interval."}));
    }
    let mut points = Vec::new();
    let mut first_region = None;
    let mut observations = Vec::new();
    for sample in &request.samples {
        if sample.regions.is_empty() || sample.confidence < request.min_confidence {
            issues.push(
                json!({"code":"subject-uncertain","sourceMs":sample.source_ms,
                "message":"Subject observations are missing or below the requested confidence."}),
            );
            observations.push(json!({"sourceMs":sample.source_ms,"status":"unknown"}));
            continue;
        }
        let left = sample.regions.iter().map(|r| r.x).fold(1.0_f64, f64::min);
        let top = sample.regions.iter().map(|r| r.y).fold(1.0_f64, f64::min);
        let right = sample
            .regions
            .iter()
            .map(|r| r.x + r.width)
            .fold(0.0_f64, f64::max);
        let bottom = sample
            .regions
            .iter()
            .map(|r| r.y + r.height)
            .fold(0.0_f64, f64::max);
        let bounds = tracking::Region {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        };
        let axis = |lo: f64, hi: f64, viewport: f64, previous: f64| {
            let minimum = (viewport / 2.0).max(hi - viewport * (0.5 - request.safe_inset));
            let maximum = (1.0 - viewport / 2.0).min(lo + viewport * (0.5 - request.safe_inset));
            (minimum <= maximum + 1e-10).then(|| previous.clamp(minimum, maximum.max(minimum)))
        };
        let desired_x = points
            .last()
            .map_or((left + right) / 2.0, |p: &tracking::Point| p.center_x);
        let desired_y = points
            .last()
            .map_or((top + bottom) / 2.0, |p: &tracking::Point| p.center_y);
        let center = axis(left, right, viewport_width, desired_x).zip(axis(
            top,
            bottom,
            viewport_height,
            desired_y,
        ));
        if let Some((center_x, center_y)) = center {
            if first_region.is_none() {
                first_region = Some(bounds.clone());
            }
            points.push(tracking::Point {
                source_ms: sample.source_ms,
                center_x,
                center_y,
                confidence: sample.confidence,
                held: false,
            });
            observations.push(
                json!({"sourceMs":sample.source_ms,"status":"fits","subjectBounds":bounds,
                "cropCenter":{"x":center_x,"y":center_y}}),
            );
        } else {
            issues.push(json!({"code":"subjects-do-not-fit","sourceMs":sample.source_ms,
                "message":"Observed subjects cannot fit in this crop and inset; use a wider format or contain."}));
            observations.push(
                json!({"sourceMs":sample.source_ms,"status":"does-not-fit","subjectBounds":bounds}),
            );
        }
    }
    let proposal = if issues.is_empty() {
        Some(tracking::Proposal { algorithm:"avw.observed-subjects.v1".into(), start_ms, end_ms,
            source_sha256:request.source_sha256.clone(), region:first_region.expect("all samples fit"), points,
            findings:vec![format!("Subject observations supplied by {}; requested safe inset {}. Observations are not independently verified.", request.observer, request.safe_inset),
                "Frames between observations and overlay occlusion are unverified.".into()],
            fallback:"Review the full video and adjust the editable crop; use contain or a wider format if subjects leave frame.".into() })
    } else {
        None
    };
    let edit = proposal
        .as_ref()
        .map(|p| json!({"action":"reframe-apply","itemId":request.item_id,"proposal":p}));
    Ok(
        json!({"schemaVersion":1,"revision":project.revision,"sequenceId":sequence.id,"itemId":item.id,
        "assetId":asset.id,"sourceSha256":request.source_sha256,"observer":request.observer,
        "sourceStart":source_start,"sourceEnd":source_end,"viewport":{"width":viewport_width,"height":viewport_height},
        "safeInset":request.safe_inset,"samples":observations,"issues":issues,"proposal":proposal,"edit":edit,
        "status":if edit.is_some(){"review-required"}else{"needs-review"},
        "limitations":["Subject identity and confidence are supplied observations, not independently verified detections.",
          "Only sampled source instants are checked; interpolated motion, occlusion and text/platform overlays need playback review.",
          "This is an editable proposal, not an automatic project edit or an accessibility guarantee."]}),
    )
}
