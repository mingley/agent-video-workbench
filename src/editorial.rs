//! Source-context paper edits. Editorial review is explicit, bounded and durable.
use crate::{Error, Result, assembly, store::Store, workflow};
use agentcut_core::{ItemPayload, Project, RationalRate, RationalTime, RoundingMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const RECORD: &str = "avw.edit-plan.v1";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Pacing {
    #[default]
    Natural,
    Montage,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Brief {
    pub objective: String,
    #[serde(default)]
    pub pacing: Pacing,
    #[serde(default = "handles")]
    pub speech_handles_ms: u32,
    #[serde(default = "context")]
    pub context_ms: u32,
    #[serde(default)]
    pub target_duration_ms: Option<u32>,
}
fn handles() -> u32 {
    250
}
fn context() -> u32 {
    1000
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    SpeechBoundary,
    SpeechHandle,
    TightCut,
    ShortPause,
    ReorderedSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Review {
    pub risk: Risk,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decision {
    pub cut_id: String,
    pub reason: String,
    #[serde(default)]
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditPlan {
    pub edit: assembly::Assemble,
    pub brief: Brief,
    pub decisions: Vec<Decision>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn text_valid(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.len() <= max
}
fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn difference(left: RationalTime, right: RationalTime) -> Result<RationalTime> {
    let (mut divisor, mut remainder) = (left.rate.numerator, right.rate.numerator);
    while remainder != 0 {
        (divisor, remainder) = (remainder, divisor % remainder);
    }
    let ticks = (left.rate.numerator / divisor)
        .checked_mul(right.rate.numerator)
        .ok_or_else(|| invalid("source times require an unsupported common tick clock"))?;
    let rate = RationalRate::frames(ticks)?;
    Ok(left
        .rescale_exact(rate)?
        .checked_sub(right.rescale_exact(rate)?)?)
}

fn validate(plan: &EditPlan) -> Result<()> {
    if !text_valid(&plan.brief.objective, 2000)
        || plan.brief.speech_handles_ms > 2000
        || !(250..=5000).contains(&plan.brief.context_ms)
        || plan
            .brief
            .target_duration_ms
            .is_some_and(|d| d == 0 || d > 3_600_000)
    {
        return Err(invalid(
            "brief needs an objective (1..2000 bytes), speechHandlesMs 0..2000, contextMs 250..5000 and optional targetDurationMs 1..3600000",
        ));
    }
    let cuts: BTreeSet<_> = plan.edit.cuts.iter().map(|c| c.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    for decision in &plan.decisions {
        if !cuts.contains(decision.cut_id.as_str())
            || !seen.insert(decision.cut_id.as_str())
            || !text_valid(&decision.reason, 1000)
        {
            return Err(invalid(
                "every cut needs exactly one decision with its cutId and a reason (1..1000 bytes)",
            ));
        }
        let mut risks = BTreeSet::new();
        for review in &decision.reviews {
            if !risks.insert(review.risk) || !text_valid(&review.note, 1000) {
                return Err(invalid(
                    "risk reviews need distinct risk codes and specific notes (1..1000 bytes)",
                ));
            }
        }
    }
    if seen != cuts {
        return Err(invalid("every cut needs exactly one decision"));
    }
    Ok(())
}

fn token(project: &Project, plan: &EditPlan) -> Result<String> {
    hash(&json!({"version":1,"project":project,"plan":plan}))
}

/// Ignore unrelated project edits, but invalidate review after timing, policy,
/// source identity, or reviewed transcript changes. Never call preflight here.
fn content_hash(project: &Project, sequence_id: &str) -> Result<String> {
    let mut sequence = project.require_sequence(sequence_id)?.clone();
    sequence.extensions.remove(RECORD);
    let assets: BTreeSet<_> = sequence
        .tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter_map(|i| match &i.payload {
            ItemPayload::Clip(c) => Some(c.asset_id.as_str()),
            _ => None,
        })
        .collect();
    let sources: Vec<_> = project
        .assets
        .iter()
        .filter(|a| assets.contains(a.id.as_str()))
        .map(|a| json!({"id":a.id,"fingerprint":a.fingerprint,"metadata":a.metadata}))
        .collect();
    let transcripts: Vec<_> = workflow::transcripts(project)?
        .into_iter()
        .filter(|t| assets.contains(t.asset_id.as_str()))
        .collect();
    hash(&json!({"sequence":sequence,"sources":sources,"transcripts":transcripts}))
}

pub fn state(project: &Project, sequence_id: &str) -> Result<Value> {
    let sequence = project.require_sequence(sequence_id)?;
    let Some(record) = sequence.extensions.get(RECORD) else {
        return Ok(json!({"status":"unplanned"}));
    };
    Ok(
        json!({"status":if record["contentSha256"] == content_hash(project,sequence_id)? {"current"} else {"stale"},
        "baseRevision":record["baseRevision"],"planSha256":record["planSha256"],
        "brief":record["plan"]["brief"],"decisions":record["plan"]["decisions"],
        "guidance":"Recorded reasons and risk notes are editorial intent, not proof of human approval or artistic quality"}),
    )
}

fn preview(asset: &str, start: i64, end: i64) -> Value {
    json!({"command":"analyze-start","task":{"kind":"proxy","assetId":asset,"startMs":start,"endMs":end,"width":1280,"height":1280},
        "noLaunch":true})
}

fn overlaps(start: RationalTime, end: RationalTime, cue: &workflow::Cue) -> Result<bool> {
    Ok(workflow::time(cue.start_ms).checked_cmp(end)?.is_lt()
        && workflow::time(cue.end_ms).checked_cmp(start)?.is_gt())
}

/// Planning performs no media writes or project mutations. Preview descriptors
/// use the existing verified proxy jobs and include audio/source-time mappings.
pub fn plan(project: &Project, plan: &EditPlan, expected: u64) -> Result<Value> {
    if project.revision != expected {
        return Err(Error::Conflict {
            expected,
            current: project.revision,
        });
    }
    validate(plan)?;
    let draft = assembly::build(project, &plan.edit, expected, "editorial-plan-draft")?;
    crate::policy::validate(project, &draft)?;
    let sequence = draft.require_sequence(&plan.edit.output_id)?;
    let items = assembly::clips(sequence)?;
    let transcripts = workflow::transcripts(project)?;
    let decisions: BTreeMap<_, _> = plan
        .decisions
        .iter()
        .map(|d| (d.cut_id.as_str(), d))
        .collect();
    let mut cuts = Vec::new();
    let mut omitted = Vec::new();
    let mut coverage: BTreeMap<&str, Vec<(RationalTime, RationalTime)>> = BTreeMap::new();
    let mut prior: BTreeMap<&str, RationalTime> = BTreeMap::new();
    let mut last_asset = "";
    let mut blockers = 0;
    for (cut, item) in plan.edit.cuts.iter().zip(items) {
        let ItemPayload::Clip(clip) = &item.payload else {
            unreachable!()
        };
        let asset = project.require_asset(&cut.asset_id)?;
        let duration = asset
            .metadata
            .duration
            .ok_or_else(|| invalid("paper edits require known source duration"))?;
        let source_end = duration
            .rescale_to(workflow::time(0).rate, RoundingMode::Floor)?
            .value;
        let start = clip.source_range.start;
        let end = clip.source_range.end_exclusive()?;
        let decision = decisions[cut.id.as_str()];
        let mut risks = BTreeSet::new();
        let selected: Vec<_> = transcripts
            .iter()
            .filter(|t| t.asset_id == asset.id)
            .flat_map(|t| &t.cues)
            .collect();
        let mut boundary_cues = Vec::new();
        let mut context_cues = Vec::new();
        let mut suggested_start_ms = cut.start_ms;
        let mut suggested_end_ms = cut.end_ms;
        for cue in &selected {
            let cue_start = workflow::time(cue.start_ms);
            let cue_end = workflow::time(cue.end_ms);
            let mut inside = false;
            for boundary in [start, end] {
                inside |= boundary.checked_cmp(cue_start)?.is_gt()
                    && boundary.checked_cmp(cue_end)?.is_lt();
            }
            let handle = workflow::time(i64::from(plan.brief.speech_handles_ms));
            let near_start = start.value > 0
                && start.checked_cmp(cue_start)?.is_le()
                && cue_start.checked_sub(start)?.checked_cmp(handle)?.is_lt();
            let near_end = end.checked_cmp(duration)?.is_lt()
                && end.checked_cmp(cue_end)?.is_ge()
                && end.checked_sub(cue_end)?.checked_cmp(handle)?.is_lt();
            if inside {
                risks.insert(Risk::SpeechBoundary);
            }
            if near_start || near_end {
                risks.insert(Risk::SpeechHandle);
            }
            if start.checked_cmp(cue_start)?.is_gt() && start.checked_cmp(cue_end)?.is_lt()
                || near_start
            {
                suggested_start_ms = suggested_start_ms
                    .min((cue.start_ms - i64::from(plan.brief.speech_handles_ms)).max(0));
            }
            if end.checked_cmp(cue_start)?.is_gt() && end.checked_cmp(cue_end)?.is_lt() || near_end
            {
                suggested_end_ms = suggested_end_ms
                    .max((cue.end_ms + i64::from(plan.brief.speech_handles_ms)).min(source_end));
            }
            if (inside || near_start || near_end) && boundary_cues.len() < 12 {
                boundary_cues.push(*cue);
            }
            if overlaps(
                start.checked_sub(workflow::time(i64::from(plan.brief.context_ms)))?,
                end.checked_add(workflow::time(i64::from(plan.brief.context_ms)))?,
                cue,
            )? && context_cues.len() < 12
            {
                context_cues.push(*cue);
            }
        }
        if item.duration.checked_cmp(workflow::time(500))?.is_lt() {
            risks.insert(Risk::TightCut);
        }
        if let Some(previous) = prior.get(asset.id.as_str()) {
            if start.checked_cmp(*previous)?.is_lt() {
                risks.insert(Risk::ReorderedSource);
            }
            let gap = start.checked_sub(*previous)?;
            if last_asset == asset.id
                && gap.value > 0
                && gap.checked_cmp(workflow::time(250))?.is_lt()
            {
                risks.insert(Risk::ShortPause);
            }
        }
        prior.insert(&asset.id, end);
        last_asset = &asset.id;
        let findings: Vec<_> = risks.into_iter().map(|risk| {
            let review = decision.reviews.iter().find(|r| r.risk == risk);
            if review.is_none() { blockers+=1; }
            json!({"risk":risk,"status":if review.is_some(){"reviewed"}else{"needs-review"},"note":review.map(|r|&r.note)})
        }).collect();
        let windows: Vec<_> = [("in",start),("out",end)].into_iter().map(|(edge,boundary)| -> Result<Value> {
            let ms=boundary.as_seconds_f64()*1000.0;
            let context=workflow::time(i64::from(plan.brief.context_ms));
            let a = boundary.checked_sub(context)?.rescale_to(workflow::time(0).rate,RoundingMode::Floor)?.value.max(0);
            let b = boundary.checked_add(context)?.rescale_to(workflow::time(0).rate,RoundingMode::Ceil)?.value.min(source_end);
            Ok(json!({"edge":edge,"boundary":boundary,
                "boundaryOffsetMs":ms-a as f64,"sourceStartMs":a,"sourceEndMs":b,"previewRequest":preview(&asset.id,a,b)}))
        }).collect::<Result<_>>()?;
        coverage.entry(&asset.id).or_default().push((start, end));
        cuts.push(json!({"cutId":cut.id,"itemId":item.id,"assetId":asset.id,"sourceSha256":asset.fingerprint.sha256,
            "requestedRangeMs":[cut.start_ms,cut.end_ms],"sourceRange":clip.source_range,"outputStart":item.start,"outputDuration":item.duration,
            "reason":decision.reason,"findings":findings,"speechEvidence":if selected.is_empty(){"unavailable"}else{"reviewed-cue-timing"},
            "boundaryCues":boundary_cues,"contextCues":context_cues,"cueExcerptsLimited":true,"contextWindows":windows,
            "boundaryExpansionProposal":if suggested_start_ms!=cut.start_ms || suggested_end_ms!=cut.end_ms {json!({"startMs":suggested_start_ms,"endMs":suggested_end_ms,"requiresReview":true,"guidance":"May include another thought or overlap the next cut. Listen, adjust and re-plan; this proposal is never applied automatically"})} else {Value::Null},
            "guidance":"Listen across both boundaries; cue timestamps are not guaranteed word-level alignment. Do not automatically expand or trim from text alone"}));
    }
    // Union retained ranges per source: reordered/repeated material must not be
    // misreported as omitted. Include excluded opening/ending, even if silent.
    for (asset_id, mut ranges) in coverage {
        let asset = project.require_asset(asset_id)?;
        // Assembly uses one exact tick clock for all source starts.
        ranges.sort_by_key(|a| a.0.value);
        let mut cursor = workflow::time(0);
        for (start, end) in ranges.into_iter().chain(std::iter::once((
            asset.metadata.duration.expect("checked duration"),
            asset.metadata.duration.expect("checked duration"),
        ))) {
            if start.checked_cmp(cursor)?.is_gt() {
                let mut cues = Vec::new();
                for cue in transcripts
                    .iter()
                    .filter(|t| t.asset_id == asset_id)
                    .flat_map(|t| &t.cues)
                {
                    if overlaps(cursor, start, cue)? && cues.len() < 12 {
                        cues.push(cue);
                    }
                }
                omitted.push(json!({"assetId":asset_id,"sourceStart":cursor,"sourceEnd":start,"duration":difference(start,cursor)?,"cueExcerpts":cues,"cueExcerptsLimited":true,
                    "guidance":"Excluded material may contain silent action, reactions or speech; absence of transcript text does not mean dead air"}));
            }
            if end.checked_cmp(cursor)?.is_gt() {
                cursor = end;
            }
        }
    }
    let duration_ms = sequence.content_duration()?.as_seconds_f64() * 1000.0;
    Ok(
        json!({"revision":expected,"projectId":project.project_id,"planSha256":token(project,plan)?,"readyToApply":blockers==0,"unreviewedRisks":blockers,
        "brief":plan.brief,"outputId":plan.edit.output_id,"durationMs":duration_ms,"targetDurationDeltaMs":plan.brief.target_duration_ms.map(|t|duration_ms-f64::from(t)),
        "cuts":cuts,"omittedSourceRanges":omitted,"preflight":assembly::preflight(&draft,&plan.edit.output_id)?,
        "guidance":"Fill previewRequest project/expectedRevision/key, run its worker and retrieve the verified artifact. Preserve a separate continuous baseline. No media or history changed by planning"}),
    )
}

impl Store {
    pub fn apply_edit_plan(
        &mut self,
        plan: &EditPlan,
        expected: u64,
        key: &str,
        plan_sha256: &str,
        dry_run: bool,
    ) -> Result<crate::store::Outcome> {
        self.change(key,expected,json!({"kind":"edit-plan.apply","plan":plan,"expectedRevision":expected,"planSha256":plan_sha256}),dry_run,|project| {
            let report = crate::editorial::plan(project,plan,expected)?;
            if report["planSha256"] != plan_sha256 { return Err(invalid("planSha256 does not match this project revision and plan; run plan-edit again")); }
            if report["readyToApply"] != true { return Err(invalid("edit plan has unreviewed risks; adjust source ranges or supply a specific decision review note, then run plan-edit again")); }
            let mut next=assembly::build(project,&plan.edit,expected,key)?;
            let content=content_hash(&next,&plan.edit.output_id)?;
            next.require_sequence_mut(&plan.edit.output_id)?.extensions.insert(RECORD.into(),json!({"baseRevision":expected,"planSha256":plan_sha256,"contentSha256":content,"plan":plan}));
            Ok(next)
        })
    }
}
