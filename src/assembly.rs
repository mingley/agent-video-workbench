//! Restrained source edits, explicit delivery intent and inspectable preflight.
use crate::{Error, Result, color, store::Store, workflow};
use agentcut_core::{
    Asset, AssetKind, AudioClipSettings, BlendMode, ItemPayload, OperationBatch, Project,
    RationalRate, RationalTime, RoundingMode, Sequence, TimelineItem, VideoClipSettings,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "avw.assembly.v1";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryColor {
    #[default]
    Preserve,
    Sdr,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Quality {
    #[default]
    High,
    Lossless,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assemble {
    pub output_id: String,
    pub name: String,
    pub cuts: Vec<workflow::Cut>,
    #[serde(default)]
    pub color: DeliveryColor,
    #[serde(default)]
    pub quality: Quality,
    /// Explicit conformance for VFR or mismatched source rates.
    #[serde(default)]
    pub frame_rate: Option<FrameRate>,
    #[serde(default)]
    pub allow_tight_cuts: bool,
    #[serde(default)]
    pub allow_reorder: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub color: DeliveryColor,
    pub quality: Quality,
    pub explicit_frame_rate: bool,
    pub allow_tight_cuts: bool,
    pub allow_reorder: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutputColor {
    pub codec: String,
    pub pixel_format: String,
    pub primaries: String,
    pub transfer: String,
    pub matrix: String,
    pub hdr: bool,
}

impl OutputColor {
    pub fn sdr() -> Self {
        Self {
            codec: "h264".into(),
            pixel_format: "yuv420p".into(),
            primaries: "bt709".into(),
            transfer: "bt709".into(),
            matrix: "bt709".into(),
            hdr: false,
        }
    }
}

pub fn policy(sequence: &Sequence) -> Result<Option<Policy>> {
    sequence
        .extensions
        .get(POLICY)
        .map(|v| serde_json::from_value(v.clone()).map_err(Error::from))
        .transpose()
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

impl Store {
    pub fn assemble(
        &mut self,
        edit: &Assemble,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<crate::store::Outcome> {
        workflow::id(&edit.output_id)?;
        if edit.cuts.is_empty() || edit.cuts.len() > 100 {
            return Err(invalid("assemble requires 1..100 explicit source cuts"));
        }
        self.change(
            key,
            expected,
            json!({"kind":"assemble","edit":edit,"expectedRevision":expected}),
            dry_run,
            |project| {
                let asset = project.require_asset(&edit.cuts[0].asset_id)?;
                let video = asset
                    .metadata
                    .video
                    .as_ref()
                    .ok_or_else(|| invalid("assemble requires a video source"))?;
                let size = color::upright_size(video);
                let rate = edit.frame_rate.map(|r| RationalRate::new(r.numerator,r.denominator)).transpose()?.unwrap_or(video.frame_rate.normalized()?);
                if !(1.0..=120.0).contains(&rate.as_f64())
                    || size.width == 0
                    || size.height == 0
                    || size.width > 4096
                    || size.height > 4096
                    || !size.width.is_multiple_of(2)
                    || !size.height.is_multiple_of(2)
                {
                    return Err(invalid("source-matched canvas must be even and <=4096 per axis; frame rate must be 1..120"));
                }
                let track = format!("{}_video", edit.output_id);
                // Share an exact tick clock between milliseconds and output
                // frames. Quantize duration, but never move the requested source
                // start (and its audio) onto a rounded NTSC frame boundary.
                let (mut divisor, mut remainder) = (rate.numerator, 1000_u32);
                while remainder != 0 {
                    (divisor, remainder) = (remainder, divisor % remainder);
                }
                let ticks = (rate.numerator / divisor).checked_mul(1000)
                    .ok_or_else(|| invalid("frame rate requires an unsupported source tick clock"))?;
                let source_rate = RationalRate::frames(ticks)?;
                let mut ops = vec![
                    json!({"op":"sequence.add","params":{"id":edit.output_id,"name":edit.name,"width":size.width,"height":size.height,"frameRate":rate}}),
                    json!({"op":"track.add","params":{"id":track,"sequence":edit.output_id,"type":"video"}}),
                ];
                let mut at = RationalTime::zero(rate);
                let mut ids = BTreeSet::new();
                for cut in &edit.cuts {
                    workflow::id(&cut.id)?;
                    if !ids.insert(&cut.id)
                        || cut.start_ms < 0
                        || cut.end_ms <= cut.start_ms
                        || cut.end_ms > 86_400_000
                    {
                        return Err(invalid("cuts need unique IDs and 0 <= startMs < endMs <=86400000"));
                    }
                    let source = project.require_asset(&cut.asset_id)?;
                    if source.kind != AssetKind::Video {
                        return Err(invalid("assemble cuts require video assets"));
                    }
                    let duration = workflow::time(cut.end_ms - cut.start_ms)
                        .rescale_to(rate, RoundingMode::Nearest)?;
                    if duration.is_zero() {
                        return Err(invalid("source cut is shorter than an output frame"));
                    }
                    ops.push(json!({"op":"clip.add","params":{"id":format!("{}_{}",edit.output_id,cut.id),"asset":cut.asset_id,"track":track,"at":at,"sourceIn":workflow::time(cut.start_ms).rescale_exact(source_rate)?,"duration":duration,"fit":"contain"}}));
                    at = at.checked_add(duration)?;
                }
                if at.as_seconds_f64() > 3600.0 {
                    return Err(invalid("assembled output exceeds one hour"));
                }
                for (index, op) in ops.iter_mut().enumerate() {
                    op["id"] = json!(format!("assemble-{index}"));
                }
                let batch: OperationBatch = serde_json::from_value(json!({
                    "schemaVersion":"1.0.0","projectId":project.project_id,
                    "baseRevision":expected,"idempotencyKey":key,
                    "description":format!("Source-preserving assembly: {}", edit.name),
                    "operations":ops
                }))?;
                let mut next = agentcut_core::apply_batch(project, &batch)?.project;
                next.require_sequence_mut(&edit.output_id)?.extensions.insert(
                    POLICY.into(),
                    serde_json::to_value(Policy {
                        color: edit.color,
                        quality: edit.quality,
                        explicit_frame_rate: edit.frame_rate.is_some(),
                        allow_tight_cuts: edit.allow_tight_cuts,
                        allow_reorder: edit.allow_reorder,
                    })?,
                );
                // Refuse unsupported edits inside the revision transaction.
                preflight(&next, &edit.output_id)?;
                Ok(next)
            },
        )
    }
}

pub fn output_color(asset: &Asset, policy: &Policy) -> Result<OutputColor> {
    if color::capability(asset)["deliverySupported"] != true {
        return Err(invalid(format!(
            "source {} has no qualified color path",
            asset.id
        )));
    }
    if policy.color == DeliveryColor::Sdr {
        return Ok(OutputColor::sdr());
    }
    let video = asset
        .metadata
        .video
        .as_ref()
        .ok_or_else(|| invalid("video metadata missing"))?;
    let primaries = video.color_primaries.as_deref();
    let transfer = video.color_transfer.as_deref();
    let matrix = video.color_matrix.as_deref();
    let hdr = matches!(transfer, Some("smpte2084" | "arib-std-b67"));
    let ten_bit = video.pixel_format == "yuv420p10le";
    let supported = if hdr {
        primaries == Some("bt2020") && matrix == Some("bt2020nc") && ten_bit
    } else {
        primaries == Some("bt709")
            && transfer == Some("bt709")
            && matrix == Some("bt709")
            && (video.pixel_format == "yuv420p" || ten_bit)
    };
    if !supported || !matches!(video.color_range.as_deref(), Some("tv" | "limited")) {
        return Err(invalid(format!(
            "source {}: preservation needs tagged limited-range Rec.709 or 10-bit BT.2020 PQ/HLG 4:2:0; inspect metadata and explicitly choose color:sdr for a qualified conversion",
            asset.id
        )));
    }
    Ok(OutputColor {
        codec: if ten_bit { "hevc" } else { "h264" }.into(),
        pixel_format: video.pixel_format.clone(),
        primaries: primaries.unwrap_or_default().into(),
        transfer: transfer.unwrap_or_default().into(),
        matrix: matrix.unwrap_or_default().into(),
        hdr,
    })
}

pub(crate) fn clips(sequence: &Sequence) -> Result<Vec<&TimelineItem>> {
    let mut items = Vec::new();
    for track in &sequence.tracks {
        if track.enabled {
            if track.muted
                || track.solo
                || track.output.is_some()
                || track.effects.iter().any(|e| e.enabled)
            {
                return Err(invalid(
                    "source-preserving assembly does not support track routing/effects/mute/solo",
                ));
            }
            for item in track.items.iter().filter(|i| i.enabled) {
                if !matches!(&item.payload, ItemPayload::Clip(_))
                    || item.effects.iter().any(|e| e.enabled)
                    || !item.keyframes.is_empty()
                {
                    return Err(invalid(
                        "source-preserving assembly supports plain cuts only; captions, effects, nested sequences and keyframes need an explicitly composed SDR output",
                    ));
                }
                items.push(item);
            }
        }
    }
    let mut ordered = items
        .into_iter()
        .map(|item| Ok((item.start.rescale_exact(sequence.frame_rate)?.value, item)))
        .collect::<Result<Vec<_>>>()?;
    ordered.sort_by_key(|(at, _)| *at);
    let items: Vec<_> = ordered.into_iter().map(|(_, item)| item).collect();
    if items.is_empty()
        || items.len() > 100
        || !sequence.transitions.is_empty()
        || !sequence.buses.is_empty()
        || sequence.audio_sample_rate != 48000
        || sequence
            .extensions
            .get("avw.profile")
            .is_some_and(|v| !v["loudnessLufs"].is_null())
        || sequence.extensions.contains_key("avw.ducking")
    {
        return Err(invalid(
            "source-preserving assembly needs 1..100 plain cuts, 48kHz audio and no transitions/buses/loudness processing",
        ));
    }
    let mut at = RationalTime::zero(sequence.frame_rate);
    for item in &items {
        let ItemPayload::Clip(clip) = &item.payload else {
            unreachable!()
        };
        if !item.start.same_instant(at)?
            || !clip.transform.is_identity_placement(sequence.canvas)
            || clip.opacity != 1.0
            || clip.blend_mode != BlendMode::Normal
            || clip.video != VideoClipSettings::default()
            || clip.audio != AudioClipSettings::default()
            || !clip.source_range.duration.same_instant(item.duration)?
        {
            return Err(invalid(
                "source-preserving assembly refuses gaps, overlaps, crops, transforms, speed or audio changes; use plain contiguous timeline cuts",
            ));
        }
        at = item.end_exclusive()?;
    }
    if !sequence.content_duration()?.same_instant(at)? || at.as_seconds_f64() > 3600.0 {
        return Err(invalid("assembly duration does not match its plain cuts"));
    }
    Ok(items)
}

pub fn preflight(project: &Project, sequence_id: &str) -> Result<Value> {
    let sequence = project.require_sequence(sequence_id)?;
    let Some(policy) = policy(sequence)? else {
        let mut sources = Vec::new();
        let mut captions = 0;
        for item in sequence
            .tracks
            .iter()
            .filter(|t| t.enabled)
            .flat_map(|t| &t.items)
            .filter(|i| i.enabled)
        {
            match &item.payload {
                ItemPayload::Clip(clip) => {
                    let asset = project.require_asset(&clip.asset_id)?;
                    sources.push(json!({"itemId":item.id,"assetId":asset.id,"sourceRange":clip.source_range,"color":color::capability(asset),"fit":clip.video.fit,"crop":clip.video.crop,"speed":clip.video.speed,"sourceVideo":asset.metadata.video}));
                }
                ItemPayload::Caption(_) | ItemPayload::Text(_) => captions += 1,
                _ => {}
            }
        }
        return Ok(
            json!({"revision":project.revision,"sequenceId":sequence_id,"mode":"composed-sdr","outputColor":OutputColor::sdr(),"canvas":sequence.canvas,"frameRate":sequence.frame_rate,"captionOrTextItems":captions,"sources":sources,"editorialReview":"pending","guidance":["HDR sources are tone mapped for this output; an HDR master needs assemble color:preserve","Review crop, captions and cut timing against the original; silence is not permission to remove reaction pauses","Use assemble for a source-matched baseline without automatic captions or crop"],"renderSupport":"renderer validation is still required"}),
        );
    };
    let items = clips(sequence)?;
    let mut sources = Vec::new();
    let mut prior: BTreeMap<&str, RationalTime> = BTreeMap::new();
    let mut last_asset = "";
    let mut findings = Vec::new();
    let mut output: Option<OutputColor> = None;
    let mut mastering = None;
    let transcripts = workflow::transcripts(project)?;
    for item in &items {
        let ItemPayload::Clip(clip) = &item.payload else {
            unreachable!()
        };
        let asset = project.require_asset(&clip.asset_id)?;
        let video = asset
            .metadata
            .video
            .as_ref()
            .ok_or_else(|| invalid("source video metadata missing"))?;
        let next_color = output_color(asset, &policy)?;
        let metadata = crate::preserve::hdr_metadata(asset)?;
        if output.as_ref().is_some_and(|v| v != &next_color)
            || (policy.color == DeliveryColor::Preserve
                && mastering.as_ref().is_some_and(|v| v != &metadata))
        {
            return Err(invalid(
                "mixed HDR transfers/color formats/mastering metadata require an explicit SDR conversion or separate source-preserving outputs",
            ));
        }
        mastering = Some(metadata);
        output = Some(next_color);
        let size = color::upright_size(video);
        if size.width != sequence.canvas.width || size.height != sequence.canvas.height {
            return Err(invalid(
                "source-preserving assembly needs matching upright dimensions; create independent outputs for different framing",
            ));
        }
        if !policy.explicit_frame_rate
            && (video.variable_frame_rate == Some(true) || video.frame_rate != sequence.frame_rate)
        {
            return Err(invalid(
                "VFR or mixed source rates require an explicit frameRate; preflight records CFR conformance rather than promising original frame timing",
            ));
        }
        let start_time = clip.source_range.start;
        let end_time = clip.source_range.end_exclusive()?;
        let start = start_time.as_seconds_f64();
        let end = end_time.as_seconds_f64();
        let duration = item.duration.as_seconds_f64();
        if item.duration.checked_cmp(workflow::time(500))?.is_lt() && !policy.allow_tight_cuts {
            return Err(invalid(
                "a source cut is shorter than 500ms; preserve breathing room or explicitly set allowTightCuts after review",
            ));
        }
        if let Some(previous) = prior.get(asset.id.as_str()) {
            if start_time.checked_cmp(*previous)?.is_lt() && !policy.allow_reorder {
                return Err(invalid(
                    "source ranges are reordered or repeated; natural assembly needs chronological cuts unless allowReorder is explicit",
                ));
            }
            let gap = start_time.checked_sub(*previous)?;
            if last_asset == asset.id
                && gap.value > 0
                && gap.checked_cmp(workflow::time(250))?.is_lt()
                && !policy.allow_tight_cuts
            {
                return Err(invalid(
                    "removing a pause shorter than 250ms can clip breaths/reactions; keep a contiguous range or explicitly set allowTightCuts",
                ));
            }
        }
        if video.variable_frame_rate == Some(true) || video.frame_rate != sequence.frame_rate {
            findings.push(json!({"itemId":item.id,"finding":"explicit CFR conformance can duplicate/drop video frames; audio stays on the source clock"}));
        }
        if color::capability(asset)["usesCompatibleBaseLayer"] == true {
            findings.push(json!({"itemId":item.id,"finding":"Dolby Vision compatible base layer only; proprietary dynamic metadata is not preserved"}));
        }
        for transcript in transcripts.iter().filter(|t| t.asset_id == asset.id) {
            for cue in &transcript.cues {
                if [start_time, end_time].iter().try_fold(
                    false,
                    |hit, boundary| -> Result<bool> {
                        Ok(hit
                            || (boundary.checked_cmp(workflow::time(cue.start_ms))?.is_gt()
                                && boundary.checked_cmp(workflow::time(cue.end_ms))?.is_lt()))
                    },
                )? && findings.len() < 100
                {
                    findings.push(json!({"itemId":item.id,"cueId":cue.id,"finding":"cut intersects a reviewed speech cue; listen for clipped words and retain handles"}));
                }
            }
        }
        prior.insert(&asset.id, end_time);
        last_asset = &asset.id;
        sources.push(json!({"itemId":item.id,"assetId":asset.id,"sourceSha256":asset.fingerprint.sha256,"sourceStartSeconds":start,"sourceEndSeconds":end,"outputStartSeconds":item.start.as_seconds_f64(),"outputDurationSeconds":duration,"sourceVideo":video}));
    }
    Ok(
        json!({"revision":project.revision,"sequenceId":sequence_id,"mode":"source-preserving","policy":policy,"outputColor":output,"canvas":sequence.canvas,"frameRate":sequence.frame_rate,"sources":sources,"findings":findings,"findingsTruncated":findings.len()>=100,"editorialReview":"pending","masterVideoEncodingGenerations":1,"lossyIntermediate":false,"automaticCaptions":false,"automaticCrop":false,"guidance":"Compare with the original and listen across every cut; technical verification cannot judge a natural performance"}),
    )
}
