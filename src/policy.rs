//! Source protection uses exact source times, independent of timeline placement.
use crate::{Error, Result};
use agentcut_core::{ItemPayload, Project, RationalTime, TrackType};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

const KEY: &str = "avw.protectedSourceRanges.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtectedRange {
    pub id: String,
    pub asset_id: String,
    pub sequence_id: String,
    pub start: RationalTime,
    pub end: RationalTime,
}

pub fn ranges(project: &Project) -> Result<Vec<ProtectedRange>> {
    project
        .extensions
        .get(KEY)
        .map(|value| serde_json::from_value(value.clone()).map_err(Error::from))
        .unwrap_or_else(|| Ok(Vec::new()))
}

pub fn add(project: &mut Project, range: ProtectedRange) -> Result<()> {
    range.start.rate.normalized()?;
    range.end.rate.normalized()?;
    if range.id.trim().is_empty()
        || range.start.is_negative()
        || range.start.checked_cmp(range.end)? != Ordering::Less
    {
        return Err(Error::Invalid(
            "protection needs an ID and a positive source interval".into(),
        ));
    }
    project.require_asset(&range.asset_id)?;
    project.require_sequence(&range.sequence_id)?;
    let mut list = ranges(project)?;
    if list.iter().any(|existing| existing.id == range.id) {
        return Err(Error::Invalid("protection ID already exists".into()));
    }
    list.push(range);
    project
        .extensions
        .insert(KEY.into(), serde_json::to_value(list)?);
    validate(project, project)
}

pub fn preserve(current: &Project, restored: &mut Project) {
    if let Some(value) = current.extensions.get(KEY) {
        restored.extensions.insert(KEY.into(), value.clone());
    }
}

pub fn validate(current: &Project, next: &Project) -> Result<()> {
    let previous = ranges(current)?;
    let protected = ranges(next)?;
    for old in previous {
        if !protected.contains(&old) {
            return Err(Error::Invalid(format!(
                "protected range {} cannot be removed through an edit",
                old.id
            )));
        }
    }
    for range in protected {
        range.start.rate.normalized()?;
        range.end.rate.normalized()?;
        if range.start.is_negative() || range.start.checked_cmp(range.end)? != Ordering::Less {
            return Err(Error::Invalid("invalid protected interval".into()));
        }
        let sequence = next.require_sequence(&range.sequence_id)?;
        let mut intervals = Vec::new();
        for track in &sequence.tracks {
            if !track.enabled || track.track_type != TrackType::Video {
                continue;
            }
            for item in &track.items {
                if !item.enabled {
                    continue;
                }
                if let ItemPayload::Clip(clip) = &item.payload
                    && clip.asset_id == range.asset_id
                    && clip.opacity > 0.0
                {
                    let start = clip.source_range.start;
                    let end = clip.source_range.end_exclusive()?;
                    intervals.push((start, end));
                }
            }
        }
        let mut covered = range.start;
        loop {
            let previous = covered;
            for &(start, end) in &intervals {
                if start.checked_cmp(covered)? != Ordering::Greater
                    && end.checked_cmp(covered)? == Ordering::Greater
                {
                    covered = end;
                }
            }
            if covered.same_instant(previous)? || covered.checked_cmp(range.end)? != Ordering::Less
            {
                break;
            }
        }
        if covered.checked_cmp(range.end)? == Ordering::Less {
            return Err(Error::Invalid(format!(
                "edit removes protected source range {}",
                range.id
            )));
        }
    }
    Ok(())
}
