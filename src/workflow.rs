//! Creator workflows expand into the same validated, atomic model operations.
use crate::{
    Error, Result,
    store::{Outcome, Store},
};
use agentcut_core::{OperationBatch, Project, RationalRate, RationalTime, RoundingMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cue {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transcript {
    pub asset_id: String,
    pub language: String,
    pub provider: String,
    pub cues: Vec<Cue>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cut {
    pub id: String,
    pub asset_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Compose {
    pub output_id: String,
    pub name: String,
    pub font_asset_id: String,
    pub cuts: Vec<Cut>,
    #[serde(default = "width")]
    pub width: u32,
    #[serde(default = "height")]
    pub height: u32,
    #[serde(default = "size")]
    pub font_size: u32,
}
fn width() -> u32 {
    1080
}
fn height() -> u32 {
    1920
}
fn size() -> u32 {
    48
}
fn id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Error::Invalid(
            "IDs require 1..80 ASCII letters, digits, underscores or hyphens".into(),
        ));
    }
    Ok(())
}
fn interval(start: i64, end: i64) -> Result<()> {
    if start < 0 || end <= start || end > 86_400_000 {
        return Err(Error::Invalid(
            "source interval must satisfy 0 <= startMs < endMs <= 86400000".into(),
        ));
    }
    Ok(())
}
fn time(ms: i64) -> RationalTime {
    RationalTime::new(
        ms,
        RationalRate {
            numerator: 1000,
            denominator: 1,
        },
    )
}
const TRANSCRIPTS: &str = "avw.transcripts.v1";

pub fn transcripts(project: &Project) -> Result<Vec<Transcript>> {
    project
        .extensions
        .get(TRANSCRIPTS)
        .map(|v| serde_json::from_value(v.clone()).map_err(Error::from))
        .unwrap_or_else(|| Ok(Vec::new()))
}
impl Store {
    pub fn transcript_import(
        &mut self,
        transcript: Transcript,
        expected: u64,
        key: &str,
    ) -> Result<Outcome> {
        id(&transcript.asset_id)?;
        if transcript.cues.len() > 20_000 {
            return Err(Error::Invalid("transcript exceeds 20000 cues".into()));
        }
        let mut ids = BTreeSet::new();
        for cue in &transcript.cues {
            id(&cue.id)?;
            interval(cue.start_ms, cue.end_ms)?;
            if !ids.insert(&cue.id) || cue.text.trim().is_empty() || cue.text.len() > 2000 {
                return Err(Error::Invalid(
                    "cue IDs must be unique; text requires 1..2000 bytes".into(),
                ));
            }
        }
        self.change(
            key,
            expected,
            json!({"kind":"transcript.import","transcript":transcript,"expectedRevision":expected}),
            false,
            |current| {
                let asset = current.require_asset(&transcript.asset_id)?;
                if let Some(duration) = asset.metadata.duration {
                    for cue in &transcript.cues {
                        if time(cue.end_ms).checked_cmp(duration)?.is_gt() {
                            return Err(Error::Invalid(
                                "transcript cue exceeds source duration".into(),
                            ));
                        }
                    }
                }
                let mut list = transcripts(current)?;
                list.retain(|t| t.asset_id != transcript.asset_id);
                list.push(transcript);
                let mut next = current.clone();
                next.extensions
                    .insert(TRANSCRIPTS.into(), serde_json::to_value(list)?);
                Ok(next)
            },
        )
    }

    pub fn compose(
        &mut self,
        compose: &Compose,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        id(&compose.output_id)?;
        if compose.cuts.is_empty()
            || compose.cuts.len() > 100
            || compose.width == 0
            || compose.height == 0
            || compose.width > 4096
            || compose.height > 4096
            || !compose.width.is_multiple_of(2)
            || !compose.height.is_multiple_of(2)
            || compose.font_size == 0
            || compose.font_size > 200
        {
            return Err(Error::Invalid(
                "compose requires 1..100 cuts, even canvas dimensions <=4096, and fontSize 1..200"
                    .into(),
            ));
        }
        // Build inside the revision transaction. A retry hashes original intent,
        // so imported transcript updates cannot change a committed request identity.
        self.change(key,expected,json!({"kind":"compose","compose":compose,"expectedRevision":expected}),dry_run,|project|{
            let font=project.require_asset(&compose.font_asset_id)?;
            if font.kind!=agentcut_core::AssetKind::Font{return Err(Error::Invalid("fontAssetId must name an imported font".into()));}
            let vtrack=format!("{}_video",compose.output_id);let ctrack=format!("{}_captions",compose.output_id);
            let mut ops=vec![json!({"op":"sequence.add","params":{"id":compose.output_id,"name":compose.name,"width":compose.width,"height":compose.height,"frameRate":{"numerator":30,"denominator":1}}}),
                json!({"op":"track.add","params":{"id":vtrack,"sequence":compose.output_id,"type":"video"}}),
                json!({"op":"track.add","params":{"id":ctrack,"sequence":compose.output_id,"type":"caption"}})];
            let transcripts=transcripts(project)?;let mut at=RationalTime::zero(RationalRate::frames(30)?);let mut cut_ids=BTreeSet::new();
            for cut in &compose.cuts {
                id(&cut.id)?;interval(cut.start_ms,cut.end_ms)?;
                if !cut_ids.insert(&cut.id){return Err(Error::Invalid("cut IDs must be unique".into()));}
                let source=project.require_asset(&cut.asset_id)?;
                if source.kind!=agentcut_core::AssetKind::Video{return Err(Error::Invalid("cuts require a video asset".into()));}
                let duration=time(cut.end_ms-cut.start_ms).rescale_to(at.rate,RoundingMode::Nearest)?;
                if duration.is_zero(){return Err(Error::Invalid("cut is shorter than one output frame".into()));}
                let clip_id=format!("{}_{}",compose.output_id,cut.id);
                ops.push(json!({"op":"clip.add","params":{"id":clip_id,"asset":cut.asset_id,"track":vtrack,"at":at,"sourceIn":time(cut.start_ms),"duration":duration,"fit":"cover"}}));
                for transcript in transcripts.iter().filter(|t|t.asset_id==cut.asset_id) {
                    for cue in &transcript.cues {
                        let start=cue.start_ms.max(cut.start_ms);let end=cue.end_ms.min(cut.end_ms);
                        if start>=end{continue;}
                        let local=time(start-cut.start_ms).rescale_to(at.rate,RoundingMode::Nearest)?;
                        let end=time(end-cut.start_ms).rescale_to(at.rate,RoundingMode::Nearest)?;
                        let cue_duration=end.checked_sub(local)?;
                        if cue_duration.is_zero(){continue;}
                        let cue_id=format!("{}_{}_{}",compose.output_id,cut.id,cue.id);
                        ops.push(json!({"op":"caption.add","params":{"id":cue_id,"track":ctrack,"at":at.checked_add(local)?,"duration":cue_duration,"text":cue.text}}));
                        ops.push(json!({"op":"item.set","target":cue_id,"params":{"property":"text.style.fontAssetId","value":compose.font_asset_id}}));
                        ops.push(json!({"op":"item.set","target":cue_id,"params":{"property":"text.style.fontSize","value":compose.font_size}}));
                    }
                }
                at=at.checked_add(duration)?;
            }
            if at.as_seconds_f64()>3600.0{return Err(Error::Invalid("output exceeds one hour".into()));}
            for (n,op) in ops.iter_mut().enumerate(){op["id"]=json!(format!("compose-{n}"));}
            let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":expected,"idempotencyKey":key,"description":format!("Compose {}",compose.name),"operations":ops}))?;
            let mut next=agentcut_core::apply_batch(project,&batch)?.project;
            next.extensions.insert(format!("avw.output.{}",compose.output_id),json!({"cuts":compose.cuts,"fontAssetId":compose.font_asset_id,"fontSize":compose.font_size,"captionMapping":"source cue intersections; cut-boundary cues may require review"}));
            Ok(next)
        })
    }
}
