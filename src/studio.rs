//! Durable creator decisions, expanded and validated within one revision.
use crate::{
    Error, Result, media,
    store::{Outcome, Store},
    workflow,
};
use agentcut_core::{
    Asset, AssetKind, ClipData, ItemPayload, OperationBatch, Project, RationalRate, RationalTime,
    Sequence, TimelineItem, Track, TrackType, TransitionAlignment,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub version: u32,
    pub font_asset_id: String,
    pub font_size: u32,
    pub color: String,
    #[serde(default = "cover")]
    pub fit: String,
    #[serde(default)]
    pub loudness_lufs: Option<f64>,
    #[serde(default)]
    pub true_peak_db: Option<f64>,
}
fn preserve() -> bool {
    true
}
fn cover() -> String {
    "cover".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub version: u32,
    pub name: String,
    pub profile_id: String,
    pub profile_version: u32,
    pub slots: Vec<Slot>,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub min_duration_ms: i64,
    pub max_duration_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Edit {
    Retention {
        enabled: bool,
        grace_seconds: u64,
    },
    Grade {
        item_id: String,
        effect_id: String,
        brightness: f64,
        exposure: f64,
        contrast: f64,
        saturation: f64,
    },
    ReframeApply {
        item_id: String,
        proposal: crate::tracking::Proposal,
    },
    AudioDuck {
        sequence: String,
        settings: crate::audio::Ducking,
    },
    ProfilePut {
        profile: Profile,
    },
    ProfileApply {
        sequence: String,
        profile_id: String,
        version: u32,
    },
    TemplatePut {
        template: Template,
    },
    TemplateInstantiate {
        template_id: String,
        version: u32,
        output_id: String,
        name: String,
        bindings: Vec<workflow::Cut>,
    },
    Variant {
        source_revision: u64,
        sequence: String,
        id: String,
        name: String,
        width: u32,
        height: u32,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        captions: BTreeMap<String, String>,
    },
    Omit {
        id: String,
        item_id: String,
        reason: String,
    },
    RestoreOmission {
        id: String,
        at_ms: i64,
    },
    Layer {
        sequence: String,
        id: String,
        asset_id: String,
        at_ms: i64,
        source_start_ms: i64,
        duration_ms: i64,
        #[serde(default = "cover")]
        fit: String,
    },
    ReplaceLayer {
        item_id: String,
        asset_id: String,
        source_start_ms: i64,
    },
    TranscriptCorrect {
        asset_id: String,
        cues: BTreeMap<String, String>,
        #[serde(default)]
        sequences: Vec<String>,
    },
    TranscriptSelect {
        asset_id: String,
        analysis_version: String,
        #[serde(default = "preserve")]
        preserve_corrections: bool,
        #[serde(default)]
        sequences: Vec<String>,
    },
    ReviewAdd {
        id: String,
        job_id: String,
        at_ms: i64,
        end_ms: i64,
        actor: String,
        text: String,
    },
    ReviewResolve {
        id: String,
        state: ReviewState,
        note: String,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    Open,
    Addressed,
    Dismissed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Omission {
    id: String,
    sequence: String,
    start: RationalTime,
    duration: RationalTime,
    reason: String,
    items: Vec<(String, TimelineItem)>,
    restored: bool,
}

pub(crate) fn valid_color(color: &str) -> bool {
    (color.len() == 7 || color.len() == 9)
        && color.starts_with('#')
        && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn validate_canvas(width: u32, height: u32) -> Result<()> {
    if !(2..=4096).contains(&width)
        || !(2..=4096).contains(&height)
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
    {
        return Err(invalid("canvas requires even dimensions in 2..4096"));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn profile_key(id: &str, version: u32) -> String {
    format!("avw.profile.{id}.{version}")
}
fn template_key(id: &str, version: u32) -> String {
    format!("avw.template.{id}.{version}")
}
pub(crate) fn validate(project: &Project) -> Result<()> {
    let issues = agentcut_core::validate_project(project);
    if agentcut_core::has_errors(&issues) {
        return Err(Error::Invalid(serde_json::to_string(&issues)?));
    }
    Ok(())
}
fn batch(project: &Project, ops: Vec<Value>) -> Result<Project> {
    let ops: Vec<_> = ops
        .into_iter()
        .enumerate()
        .map(|(i, mut op)| {
            op["id"] = json!(format!("studio-{i}"));
            op
        })
        .collect();
    let batch: OperationBatch = serde_json::from_value(
        json!({"schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":project.revision,"idempotencyKey":"internal-studio-expansion","operations":ops}),
    )?;
    Ok(agentcut_core::apply_batch(project, &batch)?.project)
}
fn property(id: &str, property: &str, value: Value) -> Value {
    json!({"op":"item.set","target":id,"params":{"property":property,"value":value}})
}
fn profile(project: &Project, id: &str, version: u32) -> Result<Profile> {
    serde_json::from_value(
        project
            .extensions
            .get(&profile_key(id, version))
            .ok_or_else(|| invalid("profile version missing"))?
            .clone(),
    )
    .map_err(Error::from)
}
fn apply_profile(project: &Project, sequence: &str, profile: &Profile) -> Result<Project> {
    let mut ops = Vec::new();
    for item in project
        .require_sequence(sequence)?
        .tracks
        .iter()
        .flat_map(|t| &t.items)
    {
        match &item.payload {
            ItemPayload::Caption(_) => {
                for (path, value) in [
                    ("text.style.fontAssetId", json!(profile.font_asset_id)),
                    ("text.style.fontSize", json!(profile.font_size)),
                    ("text.style.color", json!(profile.color)),
                ] {
                    ops.push(property(&item.id, path, value));
                }
            }
            ItemPayload::Clip(_) => ops.push(property(&item.id, "video.fit", json!(profile.fit))),
            _ => {}
        }
    }
    let mut next = if ops.is_empty() {
        project.clone()
    } else {
        batch(project, ops)?
    };
    next.require_sequence_mut(sequence)?
        .extensions
        .insert("avw.profile".into(), serde_json::to_value(profile)?);
    Ok(next)
}
fn shift(sequence: &mut Sequence, at: RationalTime, delta: RationalTime) -> Result<()> {
    for item in sequence.tracks.iter_mut().flat_map(|t| &mut t.items) {
        if item.start.checked_cmp(at)?.is_ge() {
            item.start = item.start.checked_add(delta)?;
        } else if item.end_exclusive()?.checked_cmp(at)?.is_gt() {
            return Err(invalid(
                "ripple boundary crosses another layer; split that layer or choose an explicit timeline edit",
            ));
        }
    }
    // Marker times are shifted by the same exact delta.
    for marker in &mut sequence.markers {
        if marker.time.checked_cmp(at)?.is_ge() {
            marker.time = marker.time.checked_add(delta)?;
        }
    }
    sequence.duration = None;
    Ok(())
}
fn collect_ids(value: &Value, map: &mut BTreeMap<String, String>, prefix: &str) {
    match value {
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                map.insert(id.into(), format!("{prefix}_{id}"));
            }
            for v in object.values() {
                collect_ids(v, map, prefix);
            }
        }
        Value::Array(array) => {
            for v in array {
                collect_ids(v, map, prefix);
            }
        }
        _ => {}
    }
}
pub(crate) fn rewrite(value: &mut Value, map: &BTreeMap<String, String>) {
    match value {
        Value::String(v) => {
            if let Some(new) = map.get(v) {
                *v = new.clone();
            }
        }
        Value::Object(v) => {
            for x in v.values_mut() {
                rewrite(x, map);
            }
        }
        Value::Array(v) => {
            for x in v {
                rewrite(x, map);
            }
        }
        _ => {}
    }
}
fn clone_sequence(sequence: &Sequence, id: &str) -> Result<Sequence> {
    let mut value = serde_json::to_value(sequence)?;
    let mut map = BTreeMap::new();
    collect_ids(&value, &mut map, id);
    map.insert(sequence.id.clone(), id.into());
    rewrite(&mut value, &map);
    Ok(serde_json::from_value(value)?)
}

/// Preserve exact source/output coordinates when frame and millisecond clocks
/// differ. Never round a feedback anchor onto the current timeline.
fn exact_time(a: RationalTime, b: RationalTime, subtract: bool) -> Result<RationalTime> {
    let (a_num, b_num) = (
        a.rate.normalized()?.numerator,
        b.rate.normalized()?.numerator,
    );
    let (mut divisor, mut remainder) = (a_num, b_num);
    while remainder != 0 {
        (divisor, remainder) = (remainder, divisor % remainder);
    }
    let ticks = (a_num / divisor)
        .checked_mul(b_num)
        .ok_or_else(|| invalid("review times require an unsupported common clock"))?;
    let rate = RationalRate::frames(ticks)?;
    let (a, b) = (a.rescale_exact(rate)?, b.rescale_exact(rate)?);
    Ok(if subtract {
        a.checked_sub(b)?
    } else {
        a.checked_add(b)?
    })
}

fn active_tracks(sequence: &Sequence) -> impl Iterator<Item = &Track> {
    let video_solo = sequence
        .tracks
        .iter()
        .any(|t| t.enabled && t.solo && t.track_type.is_visual());
    let audio_solo = sequence
        .tracks
        .iter()
        .any(|t| t.enabled && t.solo && t.track_type == TrackType::Audio);
    sequence.tracks.iter().filter(move |track| {
        track.enabled
            && track.track_type != TrackType::Data
            && !(track.track_type == TrackType::Audio
                && (track.muted || (audio_solo && !track.solo)))
            && !(track.track_type.is_visual() && video_solo && !track.solo)
    })
}

/// The renderer expands transition handles beyond the nominal clip intervals.
/// We cannot currently trace those extra source frames precisely.
fn touches_transition(sequence: &Sequence, at: RationalTime, end: RationalTime) -> Result<bool> {
    use agentcut_core::RoundingMode;
    for transition in &sequence.transitions {
        let left = sequence
            .item(&transition.left_item_id)
            .ok_or_else(|| invalid("transition item missing"))?;
        let cut = left
            .end_exclusive()?
            .rescale_to(sequence.frame_rate, RoundingMode::Nearest)?;
        let duration = transition
            .duration
            .rescale_to(sequence.frame_rate, RoundingMode::Nearest)?;
        let start = match transition.alignment {
            TransitionAlignment::Center => {
                cut.checked_sub(duration.scaled(1, 2, RoundingMode::Floor)?)?
            }
            TransitionAlignment::StartAtCut => cut,
            TransitionAlignment::EndAtCut => cut.checked_sub(duration)?,
        };
        let finish = start.checked_add(duration)?;
        if start.checked_cmp(end)?.is_lt() && finish.checked_cmp(at)?.is_gt() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewSourceRange {
    asset_id: String,
    item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_kind: Option<ReviewMediaKind>,
    start: RationalTime,
    end: RationalTime,
    output_start: RationalTime,
    output_end: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReviewMediaKind {
    Video,
    Audio,
}

// A clip on a visual track can also contribute its embedded audio. Explicit
// audio tracks contribute audio only. Keep these identities independent so a
// surviving soundtrack cannot stand in for a removed picture, or vice versa.
fn contributes(track: &Track, clip: &ClipData, asset: &Asset, kind: ReviewMediaKind) -> bool {
    match kind {
        ReviewMediaKind::Video => {
            track.track_type == TrackType::Video && asset.kind == AssetKind::Video
        }
        ReviewMediaKind::Audio => {
            clip.audio.enabled
                && (track.track_type == TrackType::Audio
                    || (track.track_type == TrackType::Video && asset.metadata.audio.is_some()))
        }
    }
}

struct CapturedReviewSources {
    points: Vec<Value>,
    ranges: Vec<ReviewSourceRange>,
    coverage: &'static str,
    limitations: BTreeSet<&'static str>,
}

// Fraction components fit in i128: i64 value × two u32 rate components.
fn compare_time(a: RationalTime, b: RationalTime) -> Ordering {
    (i128::from(a.value) * i128::from(a.rate.denominator) * i128::from(b.rate.numerator))
        .cmp(&(i128::from(b.value) * i128::from(b.rate.denominator) * i128::from(a.rate.numerator)))
}

/// Whether a union covers a whole interval and whether it contains overlap.
fn coverage(
    start: RationalTime,
    end: RationalTime,
    intervals: &mut [(RationalTime, RationalTime)],
) -> Result<(bool, bool)> {
    intervals.sort_by(|a, b| compare_time(a.0, b.0));
    let mut cursor = start;
    let mut complete = true;
    let mut ambiguous = false;
    for &(from, to) in intervals.iter() {
        if from.checked_cmp(cursor)?.is_gt() {
            complete = false;
        } else if from.checked_cmp(cursor)?.is_lt() {
            ambiguous = true;
        }
        if to.checked_cmp(cursor)?.is_gt() {
            cursor = to;
        }
    }
    Ok((complete && cursor.checked_cmp(end)?.is_ge(), ambiguous))
}

fn capture_review_sources(
    project: &Project,
    sequence: &Sequence,
    at_ms: i64,
    end_ms: i64,
) -> Result<CapturedReviewSources> {
    let (at, end) = (workflow::time(at_ms), workflow::time(end_ms));
    let mut points = Vec::new();
    let mut ranges = Vec::new();
    let mut timeline = Vec::new();
    let mut limitations = BTreeSet::new();
    if touches_transition(sequence, at, end)? {
        limitations.insert("transition-handles");
    }
    for (track, item) in active_tracks(sequence)
        .flat_map(|track| track.items.iter().map(move |item| (track, item)))
        .filter(|(_, item)| item.enabled)
    {
        let from = item.start.max_of(at)?;
        let to = item.end_exclusive()?.min_of(end)?;
        if from.checked_cmp(to)?.is_ge() {
            continue;
        }
        let ItemPayload::Clip(clip) = &item.payload else {
            if matches!(item.payload, ItemPayload::Sequence(_)) {
                limitations.insert("nested-sequence");
            }
            continue;
        };
        if track.track_type == TrackType::Audio && !clip.audio.enabled {
            continue;
        }
        if !clip.video.speed.is_normal() {
            limitations.insert("non-normal-speed");
            continue;
        }
        if item.start.checked_cmp(at)?.is_le() && item.end_exclusive()?.checked_cmp(at)?.is_gt() {
            points.push(json!({"assetId":clip.asset_id,"start":exact_time(clip.source_range.start,exact_time(at,item.start,true)?,false)?,"itemId":item.id}));
        }
        let asset = project.require_asset(&clip.asset_id)?;
        if !matches!(asset.kind, AssetKind::Video | AssetKind::Audio)
            || !clip.source_range.duration.same_instant(item.duration)?
        {
            limitations.insert("non-temporal-or-mismatched-source");
            continue;
        }
        let mut contributed = false;
        for kind in [ReviewMediaKind::Video, ReviewMediaKind::Audio] {
            if !contributes(track, clip, asset, kind) {
                continue;
            }
            ranges.push(ReviewSourceRange {
                asset_id: clip.asset_id.clone(),
                item_id: item.id.clone(),
                media_kind: Some(kind),
                start: exact_time(
                    clip.source_range.start,
                    exact_time(from, item.start, true)?,
                    false,
                )?,
                end: exact_time(
                    clip.source_range.start,
                    exact_time(to, item.start, true)?,
                    false,
                )?,
                output_start: from,
                output_end: to,
            });
            contributed = true;
        }
        if contributed {
            timeline.push((from, to));
        }
    }
    let (complete, _) = coverage(at, end, &mut timeline)?;
    let status = if ranges.is_empty() {
        "unmapped"
    } else if complete && limitations.is_empty() {
        "complete"
    } else {
        "partial"
    };
    Ok(CapturedReviewSources {
        points,
        ranges,
        coverage: status,
        limitations,
    })
}

fn map_review_range(
    project: &Project,
    sequence: &Sequence,
    range: &ReviewSourceRange,
) -> Result<Value> {
    range.start.rate.normalized()?;
    range.end.rate.normalized()?;
    if !range.start.checked_cmp(range.end)?.is_lt() {
        return Err(invalid("invalid review source range"));
    }
    let mut placements = Vec::new();
    let mut source_intervals = Vec::new();
    let mut offsets = Vec::new();
    let mut limitations = BTreeSet::new();
    if range.media_kind.is_none() {
        limitations.insert("media-kind-unknown");
    }
    for (track, item) in active_tracks(sequence)
        .flat_map(|track| track.items.iter().map(move |item| (track, item)))
        .filter(|(_, item)| item.enabled)
    {
        let ItemPayload::Clip(clip) = &item.payload else {
            if matches!(item.payload, ItemPayload::Sequence(_)) {
                limitations.insert("nested-sequence");
            }
            continue;
        };
        if (track.track_type == TrackType::Audio && !clip.audio.enabled)
            || clip.asset_id != range.asset_id
        {
            continue;
        }
        let asset = project.require_asset(&clip.asset_id)?;
        let kinds = [ReviewMediaKind::Video, ReviewMediaKind::Audio];
        if !kinds.into_iter().any(|kind| {
            range.media_kind.is_none_or(|saved| saved == kind)
                && contributes(track, clip, asset, kind)
        }) {
            continue;
        }
        let from = clip.source_range.start.max_of(range.start)?;
        let to = clip.source_range.end_exclusive()?.min_of(range.end)?;
        if !from.checked_cmp(to)?.is_lt() {
            continue;
        }
        if !clip.video.speed.is_normal() {
            limitations.insert("non-normal-speed");
            continue;
        }
        if !matches!(asset.kind, AssetKind::Video | AssetKind::Audio)
            || !clip.source_range.duration.same_instant(item.duration)?
        {
            limitations.insert("non-temporal-or-mismatched-source");
            continue;
        }
        let output_start = exact_time(
            item.start,
            exact_time(from, clip.source_range.start, true)?,
            false,
        )?;
        let output_end = exact_time(
            item.start,
            exact_time(to, clip.source_range.start, true)?,
            false,
        )?;
        if touches_transition(sequence, output_start, output_end)? {
            limitations.insert("transition-handles");
        }
        placements.push(json!({"itemId":item.id,"sourceStart":from,"sourceEnd":to,"start":output_start,"end":output_end,
            "atMs":output_start.rescale_to(workflow::time(0).rate,agentcut_core::RoundingMode::Nearest)?.value,
            "endMs":output_end.rescale_to(workflow::time(0).rate,agentcut_core::RoundingMode::Nearest)?.value}));
        source_intervals.push((from, to));
        offsets.push((from, to, exact_time(output_start, from, true)?));
    }
    let (complete, _) = coverage(range.start, range.end, &mut source_intervals)?;
    // Overlapping audio and video can refer to the same source at the same
    // output time; only differing source-to-output offsets are ambiguous.
    offsets.sort_by(|a, b| compare_time(a.0, b.0));
    let mut active: Option<(RationalTime, RationalTime)> = None;
    let mut ambiguous = false;
    for (from, to, offset) in offsets {
        if let Some((last_end, last_offset)) = active {
            if from.checked_cmp(last_end)?.is_lt() && !offset.same_instant(last_offset)? {
                ambiguous = true;
            }
            if to.checked_cmp(last_end)?.is_gt() {
                active = Some((to, offset));
            }
        } else {
            active = Some((to, offset));
        }
    }
    let status = if placements.is_empty() {
        "removed_or_unmapped"
    } else if ambiguous {
        "ambiguous"
    } else if complete && limitations.is_empty() {
        "resolved"
    } else {
        "partial"
    };
    Ok(json!({"source":range,"mapping":status,"placements":placements,"limitations":limitations}))
}

// Multiple source segments may all survive but move apart or change order.
// They preserve the reviewed interval only when every piece moves by the same
// exact output-time offset, including pieces created by later clip splits.
fn range_arrangement(mappings: &[Value]) -> Result<bool> {
    let mut offset: Option<RationalTime> = None;
    for mapping in mappings {
        let source: ReviewSourceRange = serde_json::from_value(mapping["source"].clone())?;
        for placement in mapping["placements"]
            .as_array()
            .ok_or_else(|| invalid("review placements missing"))?
        {
            let source_start: RationalTime =
                serde_json::from_value(placement["sourceStart"].clone())?;
            let output_start: RationalTime = serde_json::from_value(placement["start"].clone())?;
            let original_start = exact_time(
                source.output_start,
                exact_time(source_start, source.start, true)?,
                false,
            )?;
            let shift = exact_time(output_start, original_start, true)?;
            if let Some(previous) = offset {
                if !previous.same_instant(shift)? {
                    return Ok(false);
                }
            } else {
                offset = Some(shift);
            }
        }
    }
    Ok(true)
}

pub fn remap_reviews(project: &Project, sequence: &str) -> Result<Value> {
    let sequence = project.require_sequence(sequence)?;
    let reviews = project
        .extensions
        .get("avw.reviews")
        .cloned()
        .unwrap_or(json!([]));
    let mut result = Vec::new();
    for review in reviews
        .as_array()
        .ok_or_else(|| invalid("invalid reviews"))?
    {
        let mut placements = Vec::new();
        if let Some(refs) = review["sources"].as_array() {
            for source in refs {
                let start: RationalTime = serde_json::from_value(source["start"].clone())?;
                for (track, item) in active_tracks(sequence)
                    .flat_map(|track| track.items.iter().map(move |item| (track, item)))
                    .filter(|(_, item)| item.enabled)
                {
                    if let ItemPayload::Clip(clip) = &item.payload
                        && !(track.track_type == TrackType::Audio && !clip.audio.enabled)
                        && source["assetId"] == clip.asset_id
                        && clip.video.speed.is_normal()
                        && clip.source_range.start.checked_cmp(start)?.is_le()
                        && clip
                            .source_range
                            .end_exclusive()?
                            .checked_cmp(start)?
                            .is_gt()
                    {
                        let at = exact_time(
                            item.start,
                            exact_time(start, clip.source_range.start, true)?,
                            false,
                        )?;
                        placements.push(json!({"itemId":item.id,"atMs":at.rescale_to(workflow::time(0).rate,agentcut_core::RoundingMode::Nearest)?.value}));
                    }
                }
            }
        }
        let mut range_mappings = Vec::new();
        let mut arrangement = "unknown";
        let mapping = if let Some(value) = review.get("sourceRanges") {
            let ranges: Vec<ReviewSourceRange> = serde_json::from_value(value.clone())?;
            for range in &ranges {
                range_mappings.push(map_review_range(project, sequence, range)?);
            }
            if range_mappings.iter().any(|m| m["mapping"] == "ambiguous") {
                "ambiguous"
            } else if range_mappings
                .iter()
                .all(|m| m["mapping"] == "removed_or_unmapped")
            {
                "removed_or_unmapped"
            } else if review["sourceRangeCoverage"] != "complete"
                || range_mappings.iter().any(|m| m["mapping"] != "resolved")
            {
                "partial"
            } else if range_arrangement(&range_mappings)? {
                arrangement = "preserved";
                "resolved"
            } else {
                arrangement = "changed";
                "rearranged"
            }
        } else {
            match placements.len() {
                0 => "removed_or_unmapped",
                1 => "resolved",
                _ => "ambiguous",
            }
        };
        result.push(json!({"review":review,"placements":placements,"mapping":mapping,
            "mappingBasis":if review.get("sourceRanges").is_some(){"source-range"}else{"legacy-point"},
            "arrangement":arrangement,"rangeMappings":range_mappings}));
    }
    Ok(json!({"revision":project.revision,"sequenceId":sequence.id,"items":result}))
}
fn reflow_captions(previous: &Project, next: &mut Project, root: &Path) -> Result<()> {
    let assets = next
        .assets
        .iter()
        .map(|a| (a.id.clone(), a.clone()))
        .collect::<BTreeMap<_, _>>();
    for sequence in &mut next.sequences {
        for item in sequence.tracks.iter_mut().flat_map(|t| &mut t.items) {
            if let Some((old_sequence, _, old)) = previous.find_item(&item.id)
                && old_sequence.canvas == sequence.canvas
                && old.payload == item.payload
            {
                continue;
            }
            let ItemPayload::Caption(caption) = &mut item.payload else {
                continue;
            };
            let font = assets
                .get(
                    caption
                        .style
                        .font_asset_id
                        .as_deref()
                        .ok_or_else(|| invalid("changed captions require an imported font"))?,
                )
                .ok_or_else(|| invalid("caption font missing"))?;
            media::verify_asset(root, font)?;
            let metrics = agentcut_render::probe::font_metrics(&root.join(&font.uri))
                .ok_or_else(|| invalid("caption font metrics missing"))?;
            let unwrapped = caption
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            caption.text =
                workflow::wrap_caption(&unwrapped, sequence.canvas.width as f64 * 0.8, |line| {
                    metrics.run_width(line, caption.style.font_size, caption.style.letter_spacing)
                })?;
            if caption.text.lines().count() as f64
                * caption.style.font_size
                * caption.style.line_height
                > sequence.canvas.height as f64 * 0.3
            {
                return Err(invalid(
                    "caption exceeds safe height; reduce font size or split the source cue",
                ));
            }
        }
    }
    Ok(())
}
impl Store {
    pub fn studio(
        &mut self,
        edit: &Edit,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        let root = self.root.clone();
        let origin = if let Edit::Variant {
            source_revision, ..
        } = edit
        {
            Some(self.revision(*source_revision)?)
        } else {
            None
        };
        let review = if let Edit::ReviewAdd { job_id, .. } = edit {
            let job = self.job(job_id)?;
            if job.state != "succeeded" {
                return Err(invalid("review requires a completed artifact"));
            }
            let result = job.result.ok_or_else(|| invalid("artifact missing"))?;
            let manifest: Value = crate::json::read(Path::new(
                result["manifest"]
                    .as_str()
                    .ok_or_else(|| invalid("video manifest missing"))?,
            ))?;
            if manifest["verification"]["sha256"]
                != media::hash_file(Path::new(
                    result["path"]
                        .as_str()
                        .ok_or_else(|| invalid("artifact path missing"))?,
                ))?
            {
                return Err(invalid("reviewed artifact bytes changed"));
            }
            Some(manifest)
        } else {
            None
        };
        self.change(key,expected,json!({"kind":"studio.edit","edit":edit,"expectedRevision":expected}),dry_run,|project| {
            let mut next=project.clone();
            match edit {
                Edit::Retention{enabled,grace_seconds}=>{if !(60..=31_536_000).contains(grace_seconds){return Err(invalid("retention grace requires 60 seconds..1 year"));}next.extensions.insert("avw.retention".into(),json!({"enabled":enabled,"graceSeconds":grace_seconds}));},
                Edit::Grade{item_id,effect_id,brightness,exposure,contrast,saturation} => {
                    workflow::id(effect_id)?;let params=json!({"brightness":brightness,"exposure":exposure,"contrast":contrast,"saturation":saturation});
                    next=batch(project,vec![json!({"op":"effect.add","target":item_id,"params":{"id":effect_id,"effect":"video.color.basic","params":params}})])?;
                    next.find_item_mut(item_id).ok_or_else(||invalid("graded item missing"))?.extensions.insert("avw.grade.v1".into(),json!({"effectId":effect_id,"parameters":params,"workingSpace":"display-referred Rec.709 SDR after source tone mapping","version":1}));
                },
                Edit::ReframeApply{item_id,proposal} => {
                    let (sequence,_,item)=project.find_item(item_id).ok_or_else(||invalid("reframed item missing"))?;let clip=item.payload.as_clip().ok_or_else(||invalid("reframing requires video clip"))?;if !clip.video.speed.is_normal(){return Err(invalid("tracked framing requires normal-speed footage"));}
                    let asset=project.require_asset(&clip.asset_id)?;if asset.fingerprint.sha256.as_deref()!=Some(&proposal.source_sha256){return Err(invalid("tracking proposal belongs to a different original"));}
                    if proposal.points.is_empty() || proposal.points.len()>121 || proposal.points.windows(2).any(|p|p[0].source_ms>=p[1].source_ms){return Err(invalid("proposal needs 1..121 ordered points"));}
                    let source_start=(clip.source_range.start.as_seconds_f64()*1000.0).round() as i64;let source_end=(clip.source_range.end_exclusive()?.as_seconds_f64()*1000.0).round() as i64;if proposal.start_ms>source_start || proposal.end_ms<source_end {return Err(invalid("tracking coverage must include the complete selected clip"));}
                    let size=crate::color::upright_size(asset.metadata.video.as_ref().ok_or_else(||invalid("reframing requires video"))?);let source_aspect=size.width as f64/size.height as f64;let output_aspect=sequence.canvas.width as f64/sequence.canvas.height as f64;let viewport_width=(output_aspect/source_aspect).min(1.0);let viewport_height=(source_aspect/output_aspect).min(1.0);
                    if proposal.region.width>viewport_width || proposal.region.height>viewport_height{return Err(invalid("selected region exceeds the output crop; choose contain or a wider canvas"));}
                    let first=proposal.points.iter().rev().find(|p|p.source_ms<=source_start).ok_or_else(||invalid("tracking has no initial framing point"))?;let mut points=vec![(source_start,first)];points.extend(proposal.points.iter().filter(|p|p.source_ms>source_start && p.source_ms<source_end).map(|p|(p.source_ms,p)));
                    let mut ops=vec![property(item_id,"video.fit",json!("cover"))];for (n,(at,p)) in points.iter().enumerate(){if !p.center_x.is_finite() || !p.center_y.is_finite() || !(0.0..=1.0).contains(&p.confidence){return Err(invalid("invalid tracking coordinates/confidence"));}
                        let left=if viewport_width==1.0{0.0}else{p.center_x-viewport_width/2.0};let top=if viewport_height==1.0{0.0}else{p.center_y-viewport_height/2.0};let right=1.0-left-viewport_width;let bottom=1.0-top-viewport_height;if [left,top,right,bottom].iter().any(|v|*v< -0.000001){return Err(invalid("proposed crop extends outside source; review or edit the proposal instead of silent clamping"));}
                        for (side,value) in [("left",left),("right",right),("top",top),("bottom",bottom)] {let path=format!("video.crop.{side}");if n==0{ops.push(property(item_id,&path,json!(value)));}ops.push(json!({"op":"keyframe.set","target":item_id,"params":{"property":path,"at":workflow::time(at-source_start),"value":value,"interpolation":if p.held {"step"}else{"linear"}}}));}
                    }
                    next=batch(project,ops)?;next.find_item_mut(item_id).ok_or_else(||invalid("reframed item missing"))?.extensions.insert("avw.reframe.v1".into(),serde_json::to_value(proposal)?);
                },
                Edit::AudioDuck {sequence,settings} => {crate::audio::validate(settings)?; for id in settings.dialogue_items.iter().chain(&settings.music_items) {if !project.require_sequence(sequence)?.tracks.iter().flat_map(|t| &t.items).any(|i| &i.id==id && i.payload.as_clip().is_some_and(|c|c.audio.enabled)){return Err(invalid("ducking requires active audio clip IDs in the selected sequence"));}}next.require_sequence_mut(sequence)?.extensions.insert("avw.ducking".into(),serde_json::to_value(settings)?);},
                Edit::ProfilePut{profile:p}=> {
                    workflow::id(&p.id)?;if !valid_color(&p.color){return Err(invalid("profile color requires #RRGGBB or #RRGGBBAA"));}
                    if p.version==0 || p.font_size==0 || p.font_size>200{return Err(invalid("profile version and font size must be positive; font size <=200"));}
                    if p.loudness_lufs.is_some_and(|v| !(-30.0..=-5.0).contains(&v)) || p.true_peak_db.is_some_and(|v| !(-9.0..=-0.1).contains(&v)) {return Err(invalid("audio target outside supported range"));}
                    let font=project.require_asset(&p.font_asset_id)?;if font.kind!=agentcut_core::AssetKind::Font{return Err(invalid("profile requires an imported font"));} media::verify_asset(&root,font)?;
                    if !matches!(p.fit.as_str(),"cover"|"contain"){return Err(invalid("profile fit must be contain or cover"));}
                    let name=profile_key(&p.id,p.version);if next.extensions.contains_key(&name){return Err(invalid("profile versions are immutable; create a new version"));}
                    next.extensions.insert(name,serde_json::to_value(p)?);
                },
                Edit::ProfileApply{sequence,profile_id,version}=>next=apply_profile(project,sequence,&profile(project,profile_id,*version)?)?,
                Edit::TemplatePut{template:t}=>{
                    workflow::id(&t.id)?;validate_canvas(t.width,t.height)?;profile(project,&t.profile_id,t.profile_version)?;
                    if t.version==0 || t.slots.is_empty() || t.slots.len()>100{return Err(invalid("template requires positive version and 1..100 slots"));}
                    let mut ids=std::collections::BTreeSet::new();for slot in &t.slots{workflow::id(&slot.id)?;if !ids.insert(&slot.id) || slot.min_duration_ms<1 || slot.max_duration_ms<slot.min_duration_ms || slot.max_duration_ms>3_600_000{return Err(invalid("invalid template slot constraints"));}}
                    let name=template_key(&t.id,t.version);if next.extensions.contains_key(&name){return Err(invalid("template versions are immutable"));}next.extensions.insert(name,serde_json::to_value(t)?);
                },
                Edit::TemplateInstantiate{template_id,version,output_id,name,bindings}=>{
                    let t: Template=serde_json::from_value(project.extensions.get(&template_key(template_id,*version)).ok_or_else(||invalid("template version missing"))?.clone())?;
                    if bindings.len()!=t.slots.len(){return Err(invalid("template binding count does not match slots"));}
                    let p=profile(project,&t.profile_id,t.profile_version)?;
                    let mut cuts=Vec::new();for slot in &t.slots {let matches:Vec<_>=bindings.iter().filter(|c| c.id==slot.id).collect();if matches.len()!=1{return Err(invalid("every slot requires exactly one binding"));}let cut=matches[0];let duration=cut.end_ms.checked_sub(cut.start_ms).ok_or_else(||invalid("duration overflow"))?;if duration<slot.min_duration_ms || duration>slot.max_duration_ms{return Err(invalid("binding duration violates template constraint"));}cuts.push(cut.clone());}
                    let compose=workflow::Compose{output_id:output_id.clone(),name:name.clone(),font_asset_id:p.font_asset_id.clone(),cuts,width:t.width,height:t.height,font_size:p.font_size};
                    next=workflow::compose_project(project,&root,&compose,expected,key)?;next=apply_profile(&next,output_id,&p)?;next.require_sequence_mut(output_id)?.extensions.insert("avw.template".into(),serde_json::to_value(t)?);
                },
                Edit::Variant{sequence,id,name,width,height,language,captions,..}=>{
                    workflow::id(id)?;if *width==0 || *height==0 || *width>4096 || *height>4096 || !width.is_multiple_of(2) || !height.is_multiple_of(2){return Err(invalid("variant canvas requires even dimensions in 2..4096"));}
                    let source=origin.as_ref().ok_or_else(||invalid("source revision missing"))?;
                    let original=source.require_sequence(sequence)?;
                    for asset in &source.assets {if let Some(current)=next.assets.iter().find(|a| a.id==asset.id){if current.fingerprint!=asset.fingerprint{return Err(invalid("variant asset identity conflicts with current project"));}}else{next.assets.push(asset.clone());}}
                    let mut copy=clone_sequence(original,id)?;copy.name=name.clone();let sx=*width as f64/original.canvas.width as f64;let sy=*height as f64/original.canvas.height as f64;copy.canvas.width= *width;copy.canvas.height= *height;
                    for (old_track,new_track) in original.tracks.iter().zip(&mut copy.tracks){for (old,item) in old_track.items.iter().zip(&mut new_track.items){match &mut item.payload{
                        ItemPayload::Caption(c)=>{if let Some(text)=captions.get(&old.id){if text.trim().is_empty() || text.len()>2000{return Err(invalid("translated caption requires 1..2000 bytes"));}c.text=text.clone();}},
                        ItemPayload::Text(t)=>{t.text_box.width*=sx;t.text_box.height*=sy;t.transform.position.x*=sx;t.transform.position.y*=sy;},
                        ItemPayload::Clip(c)=>{c.transform.position.x*=sx;c.transform.position.y*=sy;},_=>{}
                    }}}
                    let actual=original.tracks.iter().flat_map(|t| &t.items).filter(|i| matches!(i.payload,ItemPayload::Caption(_))).count();if language.is_some() && captions.len()!=actual{return Err(invalid("language variants require an explicit translation for every caption"));}
                    if captions.keys().any(|id| !original.tracks.iter().flat_map(|t| &t.items).any(|i| &i.id==id && matches!(i.payload,ItemPayload::Caption(_)))){return Err(invalid("translation targets an unknown caption"));}
                    copy.extensions.insert("avw.variant".into(),json!({"sequenceId":sequence,"sourceRevision":source.revision,"language":language,"translationReview":"required","alignment":"one imported translated segment per source caption"}));next.sequences.push(copy);
                },
                Edit::Omit{id,item_id,reason}=>{
                    workflow::id(id)?;let name=format!("avw.omission.{id}");if next.extensions.contains_key(&name){return Err(invalid("omission ID already exists"));}
                    let (seq,track,item)=project.find_item(item_id).ok_or_else(||invalid("omitted item missing"))?;if !matches!(item.payload,ItemPayload::Clip(_)){return Err(invalid("omit requires a source clip; split partial ranges first"));}
                    if seq.transitions.iter().any(|t| t.left_item_id==*item_id || t.right_item_id==*item_id){return Err(invalid("remove dependent transition before omitting clip"));}
                    let mut saved=vec![(track.id.clone(),item.clone())];let start=item.start;let end=item.end_exclusive()?;
                    for other_track in &seq.tracks {for other in &other_track.items {if other.id!=*item_id && ((item.linked_group_id.is_some() && item.linked_group_id==other.linked_group_id) || (matches!(other.payload,ItemPayload::Caption(_)) && other.start.checked_cmp(start)?.is_ge() && other.end_exclusive()?.checked_cmp(end)?.is_le())){saved.push((other_track.id.clone(),other.clone()));}}}
                    let changed=next.require_sequence_mut(&seq.id)?;for t in &mut changed.tracks{t.items.retain(|i| !saved.iter().any(|(_,s)|s.id==i.id));}shift(changed,end,RationalTime::zero(item.duration.rate).checked_sub(item.duration)?)?;
                    next.extensions.insert(name,serde_json::to_value(Omission{id:id.clone(),sequence:seq.id.clone(),start,duration:item.duration,reason:reason.clone(),items:saved,restored:false})?);
                },
                Edit::RestoreOmission{id,at_ms}=>{
                    if *at_ms<0{return Err(invalid("restore destination must be nonnegative"));}
                    let name=format!("avw.omission.{id}");let mut omission: Omission=serde_json::from_value(next.extensions.get(&name).ok_or_else(||invalid("omission missing"))?.clone())?;if omission.restored{return Err(invalid("omission has already been restored"));}
                    let at=workflow::time(*at_ms).rescale_to(omission.start.rate,agentcut_core::RoundingMode::Nearest)?;
                    let seq=next.require_sequence_mut(&omission.sequence)?;let style=seq.tracks.iter().flat_map(|t| &t.items).find_map(|i|if let ItemPayload::Caption(c)=&i.payload{Some(c.style.clone())}else{None});
                    shift(seq,at,omission.duration)?;
                    for (track,mut item) in omission.items.clone(){item.start=at.checked_add(item.start.checked_sub(omission.start)?)?;item.id=format!("restored_{id}_{}",item.id);if let ItemPayload::Caption(c)=&mut item.payload && let Some(style)=&style{c.style=style.clone();}seq.tracks.iter_mut().find(|t| t.id==track).ok_or_else(||invalid("omission track was removed; explicit interchange needed"))?.items.push(item);}
                    for t in &mut seq.tracks{t.sort_items()?;}omission.restored=true;next.extensions.insert(name,serde_json::to_value(omission)?);
                },
                Edit::Layer{sequence,id,asset_id,at_ms,source_start_ms,duration_ms,fit}=>{
                    workflow::id(id)?;if *at_ms<0 || *source_start_ms<0 || *duration_ms<=0 || *duration_ms>3_600_000{return Err(invalid("invalid visual layer interval"));}
                    if !project.require_asset(asset_id)?.kind.is_visual(){return Err(invalid("layer requires a visual asset"));}
                    let track=format!("{id}_layer");next=batch(project,vec![json!({"op":"track.add","params":{"id":track,"sequence":sequence,"type":"video"}}),json!({"op":"clip.add","params":{"id":id,"asset":asset_id,"track":track,"at":workflow::time(*at_ms),"sourceIn":workflow::time(*source_start_ms),"duration":workflow::time(*duration_ms),"fit":fit}}),property(id,"audio.enabled",json!(false))])?;
                    next.find_item_mut(id).ok_or_else(||invalid("layer missing"))?.extensions.insert("avw.visualOnly".into(),json!(true));
                },
                Edit::ReplaceLayer{item_id,asset_id,source_start_ms}=>{
                    if *source_start_ms<0 || !project.require_asset(asset_id)?.kind.is_visual(){return Err(invalid("invalid replacement visual source"));}let item=next.find_item_mut(item_id).ok_or_else(||invalid("layer missing"))?;if item.extensions.get("avw.visualOnly")!=Some(&json!(true)){return Err(invalid("replace-layer targets only an explicitly visual-only layer"));}let clip=item.payload.as_clip_mut().ok_or_else(||invalid("layer is not a clip"))?;clip.asset_id=asset_id.clone();clip.source_range.start=workflow::time(*source_start_ms);clip.audio.enabled=false;
                },
                Edit::TranscriptCorrect{asset_id,cues,sequences}=>{
                    let mut transcripts=workflow::transcripts(project)?;let transcript=transcripts.iter_mut().find(|t| &t.asset_id==asset_id).ok_or_else(||invalid("transcript missing"))?;
                    let original=serde_json::to_value(&*transcript)?;let mut changes=Vec::new();
                    for (id,text) in cues{if text.trim().is_empty() || text.len()>2000{return Err(invalid("corrected cue requires 1..2000 bytes"));}let cue=transcript.cues.iter_mut().find(|c| &c.id==id).ok_or_else(||invalid("corrected cue missing"))?;changes.push(json!({"cueId":id,"originalText":cue.text,"correctedText":text}));cue.text=text.clone();}
                    for sequence in sequences {let seq=next.require_sequence_mut(sequence)?;for item in seq.tracks.iter_mut().flat_map(|t| &mut t.items){if let ItemPayload::Caption(c)=&mut item.payload && let Some(binding)=item.extensions.get("avw.sourceCue") && binding["assetId"]==*asset_id && let Some(text)=binding["cueId"].as_str().and_then(|id|cues.get(id)){c.text=text.clone();}}}
                    next.extensions.insert(workflow::TRANSCRIPTS.into(),serde_json::to_value(transcripts)?);let name=format!("avw.corrections.{asset_id}");let mut corrections=next.extensions.get(&name).cloned().unwrap_or(json!([]));corrections.as_array_mut().ok_or_else(||invalid("invalid correction history"))?.push(json!({"analysis":original,"changes":changes,"revision":expected+1}));next.extensions.insert(name,corrections);
                },
                Edit::TranscriptSelect{asset_id,analysis_version,preserve_corrections,sequences}=>{
                    if analysis_version.len()!=64 || !analysis_version.bytes().all(|b|b.is_ascii_hexdigit()){return Err(invalid("analysisVersion requires a SHA-256 transcript version"));}
                    let mut selected:workflow::Transcript=serde_json::from_value(project.extensions.get(&format!("avw.analysis.transcript.{analysis_version}")).ok_or_else(||invalid("analysis version missing"))?.clone())?;
                    if selected.asset_id!=*asset_id{return Err(invalid("analysis source does not match selected asset"));}
                    let mut list=workflow::transcripts(project)?;let previous=list.iter().find(|t|t.asset_id==*asset_id).ok_or_else(||invalid("selected source has no attached transcript"))?;
                    let correction_key=format!("avw.corrections.{asset_id}");
                    if *preserve_corrections && let Some(history)=project.extensions.get(&correction_key).and_then(Value::as_array){
                        for change in history.iter().filter_map(|h|h["changes"].as_array()).flatten(){
                            let cue_id=change["cueId"].as_str().ok_or_else(||invalid("correction cue missing"))?;
                            let old=previous.cues.iter().find(|c|c.id==cue_id).ok_or_else(||invalid("reviewed cue missing"))?;
                            let cue=selected.cues.iter_mut().find(|c|c.id==cue_id && c.start_ms==old.start_ms && c.end_ms==old.end_ms).ok_or_else(||invalid("correction cannot align to new analysis; review explicitly with preserveCorrections false"))?;
                            cue.text=old.text.clone();
                        }
                    }
                    for id in sequences {for item in next.require_sequence_mut(id)?.tracks.iter_mut().flat_map(|t|&mut t.items){
                        if let ItemPayload::Caption(c)=&mut item.payload && let Some(binding)=item.extensions.get_mut("avw.sourceCue") && binding["assetId"]==*asset_id{
                            let cue=selected.cues.iter().find(|c|Some(c.id.as_str())==binding["cueId"].as_str()).ok_or_else(||invalid("bound caption cue missing in new analysis; recompose selected output"))?;
                            let start=binding["sourceStartMs"].as_i64().ok_or_else(||invalid("caption source start missing"))?;let end=binding["sourceEndMs"].as_i64().ok_or_else(||invalid("caption source end missing"))?;
                            if start<cue.start_ms || end>cue.end_ms{return Err(invalid("new cue timing does not cover existing caption; recompose selected output"));}
                            c.text=cue.text.clone();binding["analysisVersion"]=json!(analysis_version);binding["language"]=json!(selected.language);
                        }
                    }}
                    list.retain(|t|t.asset_id!=*asset_id);list.push(selected);next.extensions.insert(workflow::TRANSCRIPTS.into(),serde_json::to_value(list)?);
                    next.extensions.entry(format!("avw.transcriptSelections.{asset_id}")).or_insert(json!([])).as_array_mut().ok_or_else(||invalid("invalid transcript selection history"))?.push(json!({"analysisVersion":analysis_version,"preserveCorrections":preserve_corrections,"sequences":sequences,"revision":expected+1}));
                },
                Edit::ReviewAdd{id,job_id,at_ms,end_ms,actor,text}=>{
                    workflow::id(id)?;if *at_ms<0 || end_ms<=at_ms || actor.trim().is_empty() || text.trim().is_empty() || text.len()>4000{return Err(invalid("review requires an interval, actor and 1..4000 bytes of text"));}
                    let manifest=review.as_ref().ok_or_else(||invalid("review manifest missing"))?;let snapshot:Project=serde_json::from_value(manifest["snapshot"].clone())?;let sequence=manifest["sequenceId"].as_str().ok_or_else(||invalid("review sequence missing"))?;let seq=snapshot.require_sequence(sequence)?;
                    let duration=crate::delivery::review_duration(seq,manifest).map_err(|_|invalid("reviewed output timing is unavailable or inconsistent; re-render this output before adding a range"))?;
                    if workflow::time(*end_ms).checked_cmp(duration)?.is_gt(){return Err(invalid("review extends beyond artifact duration"));}
                    let sources=capture_review_sources(&snapshot,seq,*at_ms,*end_ms)?;
                    let reviews=next.extensions.entry("avw.reviews".into()).or_insert(json!([])).as_array_mut().ok_or_else(||invalid("invalid reviews"))?;if reviews.iter().any(|r|r["id"]==*id){return Err(invalid("review ID exists"));}reviews.push(json!({"id":id,"jobId":job_id,"artifactSha256":manifest["verification"]["sha256"],"artifactRevision":snapshot.revision,"sequenceId":sequence,"atMs":at_ms,"endMs":end_ms,"actor":actor,"text":text,"sources":sources.points,"sourceRanges":sources.ranges,"sourceRangeCoverage":sources.coverage,"sourceRangeLimitations":sources.limitations,"state":"open"}));
                },
                Edit::ReviewResolve{id,state,note}=>{let reviews=next.extensions.get_mut("avw.reviews").and_then(Value::as_array_mut).ok_or_else(||invalid("reviews missing"))?;let review=reviews.iter_mut().find(|r|r["id"]==*id).ok_or_else(||invalid("review missing"))?;review["state"]=serde_json::to_value(state)?;review["resolution"]=json!({"revision":expected+1,"note":note});}
            }
            reflow_captions(project,&mut next,&root)?;validate(&next)?;Ok(next)
        })
    }
}
