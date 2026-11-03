//! Explicit, reversible per-item spectral noise reduction for composed output.
//!
//! Settings live on the timeline item, not the original media. The filter is
//! deliberately opt-in and does not claim to isolate speech or improve every
//! recording. Source-preserving assemblies and nested timelines are refused.
use crate::{
    Error, Result, assembly,
    process::{self, Control},
    store::{Outcome, Store},
    workflow,
};
use agentcut_core::{ItemPayload, Project, RationalTime, TrackType};
use agentcut_render::{FfmpegBackend, compile::RenderPlan, ir::RenderIr};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, process::Command, time::Duration};

pub const KEY: &str = "avw.audioCleanup.v1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub noise_reduction_db: u8,
    pub noise_floor_db: i8,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CleanupEdit {
    Set {
        sequence: String,
        item_id: String,
        settings: Settings,
    },
    Remove {
        sequence: String,
        item_id: String,
    },
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(1..=18).contains(&self.noise_reduction_db)
            || !(-80..=-25).contains(&self.noise_floor_db)
        {
            return Err(invalid(
                "audio cleanup requires noiseReductionDb 1..18 and noiseFloorDb -80..-25",
            ));
        }
        Ok(())
    }
}

/// Determine whether a selected sequence (including nested descendants) has
/// cleanup or nesting. Iterative traversal and a set bound cyclic input.
fn scope(project: &Project, sequence: &str) -> Result<(bool, bool)> {
    let mut pending = vec![sequence.to_owned()];
    let mut seen = HashSet::new();
    let mut nested = false;
    let mut configured = false;
    while let Some(sequence_id) = pending.pop() {
        if !seen.insert(sequence_id.clone()) {
            continue;
        }
        let current = project.require_sequence(&sequence_id)?;
        for item in current.tracks.iter().flat_map(|track| &track.items) {
            configured |= item.extensions.contains_key(KEY);
            if let ItemPayload::Sequence(child) = &item.payload {
                nested = true;
                pending.push(child.sequence_id.clone());
            }
        }
    }
    Ok((nested, configured))
}

/// Refuse paths where source-preserving or nested rendering would bypass the
/// configured item filter. This check runs on the frozen input before encoding.
pub fn validate_scope(project: &Project, sequence: &str, assembled: bool) -> Result<()> {
    let (nested, configured) = scope(project, sequence)?;
    if assembled && configured {
        return Err(invalid(
            "source-preserving assemblies cannot apply audio cleanup; create a composed output or remove cleanup",
        ));
    }
    if nested && configured {
        return Err(invalid(
            "audio cleanup with nested sequences is unsupported; flatten the output or remove cleanup",
        ));
    }
    Ok(())
}

/// Update one item's explicit cleanup settings as a normal project revision.
pub fn edit(
    root: &Path,
    edit: &CleanupEdit,
    expected: u64,
    key: &str,
    dry_run: bool,
) -> Result<Outcome> {
    let (sequence_id, item_id) = match edit {
        CleanupEdit::Set {
            sequence,
            item_id,
            settings,
        } => {
            settings.validate()?;
            (sequence, item_id)
        }
        CleanupEdit::Remove { sequence, item_id } => (sequence, item_id),
    };
    workflow::id(sequence_id)?;
    workflow::id(item_id)?;
    if key.trim().is_empty() || !(8..=200).contains(&key.len()) {
        return Err(invalid("idempotency key requires 8..200 nonblank bytes"));
    }
    let mut store = Store::open(root)?;
    store.change(key, expected, json!({"kind":"audio.cleanup","edit":edit,"expectedRevision":expected}), dry_run, |project| {
        let sequence = project.require_sequence(sequence_id)?;
        let (track, item) = sequence.tracks.iter().find_map(|track| {
            track.items.iter().find(|item| &item.id == item_id).map(|item| (track, item))
        }).ok_or_else(|| invalid("audio cleanup item is not in the selected sequence"))?;
        if track.locked { return Err(invalid("audio cleanup cannot change an item on a locked track")); }
        match edit {
            CleanupEdit::Set { settings, .. } => {
                if assembly::policy(sequence)?.is_some() {
                    return Err(invalid("source-preserving assemblies cannot apply audio cleanup; create a composed output"));
                }
                if scope(project, sequence_id)?.0 {
                    return Err(invalid("audio cleanup with nested sequences is unsupported; flatten the output first"));
                }
                let ItemPayload::Clip(clip) = &item.payload else {
                    return Err(invalid("audio cleanup requires a clip with an audio stream"));
                };
                let solo = sequence.tracks.iter().any(|candidate| candidate.enabled && candidate.solo
                    && if track.track_type == TrackType::Audio { candidate.track_type == TrackType::Audio }
                    else { candidate.track_type.is_visual() });
                if !track.enabled || !item.enabled || !clip.audio.enabled || track.track_type == TrackType::Audio && track.muted
                    || solo && !track.solo || !matches!(track.track_type, TrackType::Audio | TrackType::Video) {
                    return Err(invalid("audio cleanup target is disabled, muted or excluded by solo"));
                }
                if !clip.video.speed.is_normal() {
                    return Err(invalid("audio cleanup requires normal-speed clips"));
                }
                let asset = project.require_asset(&clip.asset_id)?;
                if asset.metadata.status != agentcut_core::MediaStatus::Available
                    || asset.metadata.audio.is_none() {
                    return Err(invalid("audio cleanup requires a probed source audio stream"));
                }
                let zero = RationalTime::zero(sequence.frame_rate);
                if !item.start.checked_cmp(sequence.effective_duration()?)?.is_lt()
                    || !item.end_exclusive()?.checked_cmp(zero)?.is_gt() {
                    return Err(invalid("audio cleanup target is outside the output duration"));
                }
                let mut next = project.clone();
                let target = next.require_sequence_mut(sequence_id)?.tracks.iter_mut()
                    .flat_map(|track| &mut track.items).find(|item| &item.id == item_id)
                    .ok_or_else(|| invalid("audio cleanup item disappeared"))?;
                target.extensions.insert(KEY.into(), serde_json::to_value(settings)?);
                Ok(next)
            }
            CleanupEdit::Remove { .. } => {
                if !item.extensions.contains_key(KEY) { return Err(invalid("audio cleanup is not configured on this item")); }
                let mut next = project.clone();
                let target = next.require_sequence_mut(sequence_id)?.tracks.iter_mut()
                    .flat_map(|track| &mut track.items).find(|item| &item.id == item_id)
                    .ok_or_else(|| invalid("audio cleanup item disappeared"))?;
                target.extensions.remove(KEY);
                Ok(next)
            }
        }
    })
}

/// Insert filters on exact compiled item branches. Return evidence even when a
/// previously configured item no longer contributes an active audio branch.
pub fn adapt(
    plan: &mut RenderPlan,
    ir: &RenderIr,
    project: &Project,
    sequence: &str,
) -> Result<Vec<Value>> {
    let sequence = project.require_sequence(sequence)?;
    let configured = sequence
        .tracks
        .iter()
        .flat_map(|track| &track.items)
        .filter_map(|item| item.extensions.get(KEY).map(|settings| (item, settings)))
        .collect::<Vec<_>>();
    if configured.is_empty() {
        return Ok(Vec::new());
    }
    let position = plan
        .args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .ok_or_else(|| invalid("audio cleanup graph missing"))?
        + 1;
    let graph = plan
        .args
        .get_mut(position)
        .ok_or_else(|| invalid("audio cleanup graph missing"))?;
    let mut chains = graph.split(';').map(str::to_owned).collect::<Vec<_>>();
    let mut report = Vec::new();
    for (item, raw) in configured {
        let settings: Settings = serde_json::from_value(raw.clone())?;
        settings.validate()?;
        let elements = ir
            .audio_elements
            .iter()
            .enumerate()
            .filter(|(_, element)| element.item_id == item.id)
            .collect::<Vec<_>>();
        if elements.is_empty() {
            report.push(json!({"itemId":item.id,"status":"skipped","reason":"item has no active audio branch in this frozen render","settings":settings}));
            continue;
        }
        if elements.len() != 1 {
            return Err(invalid(format!(
                "audio cleanup target {} has ambiguous audio branches",
                item.id
            )));
        }
        let (ordinal, element) = elements[0];
        if !element.speed.is_normal() {
            return Err(invalid(format!(
                "audio cleanup target {} is not normal speed",
                item.id
            )));
        }
        let suffix = format!("[a{ordinal}]");
        let producers = chains
            .iter()
            .filter(|chain| chain.ends_with(&suffix))
            .count();
        if producers != 1 {
            return Err(invalid(format!(
                "audio cleanup target {} has no unique compiled producer",
                item.id
            )));
        }
        let branch = chains
            .iter_mut()
            .find(|chain| chain.ends_with(&suffix))
            .expect("producer counted");
        branch.truncate(branch.len() - suffix.len());
        branch.push_str(&format!("[avw_cleanup_{ordinal}];[avw_cleanup_{ordinal}]afftdn=nr={}:nf={}:nt=w:tn=0[a{ordinal}]", settings.noise_reduction_db, settings.noise_floor_db));
        report.push(json!({"itemId":item.id,"status":"applied","settings":settings,"method":"FFmpeg afftdn white-noise spectral reduction","routing":"after clip/track gain, pan and fades, before bus routing and sidechain processing; downstream mute and gain still apply","review":"Listen for altered speech, music and transient sounds"}));
    }
    *graph = chains.join(";");
    Ok(report)
}

/// Only configured, active cleanup requires the optional FFmpeg filter.
pub fn require_filter(backend: &FfmpegBackend, control: &mut dyn Control) -> Result<()> {
    let output = process::run(
        Command::new(backend.ffmpeg_path()).args(["-hide_banner", "-filters"]),
        Duration::from_secs(10),
        control,
    )?;
    let available = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some("afftdn"));
    if !available {
        return Err(invalid(
            "configured FFmpeg lacks the afftdn filter required for audio cleanup",
        ));
    }
    Ok(())
}
