//! Reviewed source-word proposals bound to original media and corrected text.
//!
//! This module does not perform speech recognition or infer word timing. It
//! validates and stores a caller-supplied proposal without rewriting captions.
use crate::{
    Error, Result, media,
    store::{Outcome, Store},
    workflow,
};
use agentcut_core::{AssetKind, Project, RationalTime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const PREFIX: &str = "avw.wordAlignment.v1.";
const MAX_WORDS: usize = 30_000;

/// A text span and its proposed half-open source-time interval.
/// Text offsets are UTF-8 bytes into the selected, corrected cue text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlignedWord {
    pub id: String,
    pub cue_id: String,
    pub text: String,
    pub text_start_byte: u32,
    pub text_end_byte: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlignmentProposal {
    pub schema_version: u32,
    pub asset_id: String,
    pub source_sha256: String,
    pub transcript_sha256: String,
    pub provider: String,
    pub provider_version: String,
    pub words: Vec<AlignedWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlignmentReview {
    pub reviewer: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptedAlignment {
    proposal: AlignmentProposal,
    review: AlignmentReview,
    accepted_revision: u64,
    source_duration: RationalTime,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn field(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// Identity of the exact selected transcript, including corrected cue text.
pub fn transcript_sha256(transcript: &workflow::Transcript) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(transcript)?)
    ))
}

fn transcript(project: &Project, asset_id: &str) -> Result<workflow::Transcript> {
    let mut matching = workflow::transcripts(project)?
        .into_iter()
        .filter(|item| item.asset_id == asset_id);
    let selected = matching
        .next()
        .ok_or_else(|| invalid("selected transcript is unavailable"))?;
    if matching.next().is_some() {
        return Err(invalid("selected transcript is ambiguous"));
    }
    Ok(selected)
}

fn validate_shape(proposal: &AlignmentProposal) -> Result<()> {
    workflow::id(&proposal.asset_id)?;
    if proposal.schema_version != 1
        || !sha256(&proposal.source_sha256)
        || !sha256(&proposal.transcript_sha256)
        || !field(&proposal.provider, 120)
        || !field(&proposal.provider_version, 120)
    {
        return Err(invalid(
            "alignment requires schemaVersion 1, lowercase source/transcript SHA-256 and provider identity/version",
        ));
    }
    if proposal.words.is_empty() || proposal.words.len() > MAX_WORDS {
        return Err(invalid(format!("alignment requires 1..={MAX_WORDS} words")));
    }
    let mut ids = BTreeSet::new();
    for word in &proposal.words {
        workflow::id(&word.id)?;
        workflow::id(&word.cue_id)?;
        if !ids.insert(&word.id) {
            return Err(invalid(format!("duplicate alignment word ID {}", word.id)));
        }
        if word.text.is_empty()
            || word.text.len() > 2000
            || word.text.chars().any(char::is_whitespace)
            || word.text_start_byte >= word.text_end_byte
            || word.start_ms < 0
            || word.end_ms <= word.start_ms
            || word.end_ms > 86_400_000
        {
            return Err(invalid(format!(
                "word {} requires non-whitespace text and a positive text/source interval",
                word.id
            )));
        }
        if word
            .speaker
            .as_ref()
            .is_some_and(|speaker| !field(speaker, 120))
        {
            return Err(invalid(format!(
                "word {} has an invalid speaker label",
                word.id
            )));
        }
        if word
            .confidence
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(invalid(format!(
                "word {} confidence must be between zero and one",
                word.id
            )));
        }
    }
    Ok(())
}

fn validate_review(review: &AlignmentReview) -> Result<()> {
    if !field(&review.reviewer, 120) || !field(&review.note, 2000) {
        return Err(invalid(
            "alignment acceptance requires a reviewer and a 1..2000 byte review note",
        ));
    }
    Ok(())
}

fn validate_bound(root: &Path, project: &Project, proposal: &AlignmentProposal) -> Result<Value> {
    validate_shape(proposal)?;
    let asset = project.require_asset(&proposal.asset_id)?;
    if !matches!(asset.kind, AssetKind::Video | AssetKind::Audio) {
        return Err(invalid("alignment requires a video or audio source"));
    }
    if asset.fingerprint.sha256.as_deref() != Some(&proposal.source_sha256) {
        return Err(invalid(
            "alignment source SHA-256 does not match the selected asset",
        ));
    }
    media::verify_asset(root, asset)?;
    let duration = asset
        .metadata
        .duration
        .ok_or_else(|| invalid("source duration is unavailable; alignment cannot be bounded"))?;
    let selected = transcript(project, &proposal.asset_id)?;
    if transcript_sha256(&selected)? != proposal.transcript_sha256 {
        return Err(invalid(
            "alignment transcript SHA-256 is stale; regenerate against the selected corrected transcript",
        ));
    }
    let cues: BTreeMap<_, _> = selected
        .cues
        .iter()
        .map(|cue| (cue.id.as_str(), cue))
        .collect();
    if cues.len() != selected.cues.len() {
        return Err(invalid("selected transcript cue IDs are ambiguous"));
    }
    let mut by_cue: BTreeMap<&str, Vec<&AlignedWord>> = BTreeMap::new();
    let mut previous_global_end = None;
    for word in &proposal.words {
        let cue = cues.get(word.cue_id.as_str()).ok_or_else(|| {
            invalid(format!(
                "alignment word {} refers to a missing cue",
                word.id
            ))
        })?;
        if word.start_ms < cue.start_ms
            || word.end_ms > cue.end_ms
            || workflow::time(word.end_ms).checked_cmp(duration)?.is_gt()
        {
            return Err(invalid(format!(
                "alignment word {} is outside its cue or source",
                word.id
            )));
        }
        if previous_global_end.is_some_and(|end| word.start_ms < end) {
            return Err(invalid(
                "word source intervals must be ordered and non-overlapping",
            ));
        }
        previous_global_end = Some(word.end_ms);
        by_cue.entry(&word.cue_id).or_default().push(word);
    }
    for (cue_id, words) in &by_cue {
        let cue = cues[cue_id];
        let mut previous_byte = 0_usize;
        let mut previous_end = None;
        for word in words {
            let start = word.text_start_byte as usize;
            let end = word.text_end_byte as usize;
            let gap = cue.text.get(previous_byte..start).ok_or_else(|| {
                invalid(format!(
                    "word {} text offsets overlap or split a UTF-8 character",
                    word.id
                ))
            })?;
            let text = cue.text.get(start..end).ok_or_else(|| {
                invalid(format!(
                    "word {} text offsets are invalid UTF-8 boundaries",
                    word.id
                ))
            })?;
            if !gap.chars().all(char::is_whitespace) || text != word.text {
                return Err(invalid(format!(
                    "word {} does not preserve the selected corrected cue text",
                    word.id
                )));
            }
            if previous_end.is_some_and(|end_ms| word.start_ms < end_ms) {
                return Err(invalid(format!(
                    "word {} is out of source-time order",
                    word.id
                )));
            }
            previous_byte = end;
            previous_end = Some(word.end_ms);
        }
        if !cue
            .text
            .get(previous_byte..)
            .is_some_and(|remaining| remaining.chars().all(char::is_whitespace))
        {
            return Err(invalid(format!(
                "alignment omits corrected text from cue {cue_id}"
            )));
        }
    }
    Ok(
        json!({"projectRevision":project.revision,"assetId":proposal.asset_id,
        "sourceSha256":proposal.source_sha256,"sourceBytesVerified":true,"sourceDuration":duration,
        "transcriptSha256":proposal.transcript_sha256,"transcriptLanguage":selected.language,
        "provider":proposal.provider,"providerVersion":proposal.provider_version,
        "wordCount":proposal.words.len(),"selectedCueCount":by_cue.len(),
        "unalignedCueCount":selected.cues.len()-by_cue.len(),
        "limitations":["Caller-supplied timings and speaker labels are proposals; validation does not establish speech accuracy", "Only complete selected cues are aligned; simultaneous or overlapping words are unsupported", "Text offsets are UTF-8 bytes, not character indices"]}),
    )
}

/// Validate a proposal without changing project history.
pub fn preview(
    root: &Path,
    project: &Project,
    proposal: &AlignmentProposal,
    expected_revision: u64,
) -> Result<Value> {
    if project.revision != expected_revision {
        return Err(Error::Conflict {
            expected: expected_revision,
            current: project.revision,
        });
    }
    validate_bound(root, project, proposal)
}

impl Store {
    /// Explicitly accept a validated proposal and review in the project history.
    pub fn accept_word_alignment(
        &mut self,
        proposal: &AlignmentProposal,
        review: &AlignmentReview,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        validate_shape(proposal)?;
        validate_review(review)?;
        if key.trim().is_empty() || !(8..=200).contains(&key.len()) {
            return Err(invalid("idempotency key requires 8..200 nonblank bytes"));
        }
        let root = self.root.clone();
        let request = json!({"kind":"word-alignment.accept","proposal":proposal,"review":review,"expectedRevision":expected});
        self.change(key, expected, request, dry_run, |current| {
            validate_bound(&root, current, proposal)?;
            let mut next = current.clone();
            let accepted_revision = expected
                .checked_add(1)
                .ok_or_else(|| invalid("revision overflow"))?;
            let source_duration = current
                .require_asset(&proposal.asset_id)?
                .metadata
                .duration
                .ok_or_else(|| {
                    invalid("source duration is unavailable; alignment cannot be bounded")
                })?;
            let accepted = AcceptedAlignment {
                proposal: proposal.clone(),
                review: review.clone(),
                accepted_revision,
                source_duration,
            };
            next.extensions.insert(
                format!("{PREFIX}{}", proposal.asset_id),
                serde_json::to_value(accepted)?,
            );
            Ok(next)
        })
    }
}

/// Retrieve a bounded page of an accepted alignment, with freshness evidence.
pub fn get(
    root: &Path,
    project: &Project,
    asset_id: &str,
    expected_revision: Option<u64>,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    workflow::id(asset_id)?;
    if limit == 0 || limit > 100 {
        return Err(invalid("alignment page limit must be 1..100"));
    }
    if offset > 0 && expected_revision.is_none() {
        return Err(invalid("alignment continuation requires expectedRevision"));
    }
    if let Some(expected) = expected_revision
        && expected != project.revision
    {
        return Err(Error::Conflict {
            expected,
            current: project.revision,
        });
    }
    let stored = project
        .extensions
        .get(&format!("{PREFIX}{asset_id}"))
        .ok_or_else(|| invalid("accepted alignment is unavailable"))?;
    let accepted: AcceptedAlignment = serde_json::from_value(stored.clone())?;
    if accepted.proposal.asset_id != asset_id {
        return Err(invalid("accepted alignment identity is invalid"));
    }
    let mut reasons = Vec::new();
    let asset = project.assets.iter().find(|asset| asset.id == asset_id);
    let identity_matches = asset.and_then(|asset| asset.fingerprint.sha256.as_deref())
        == Some(accepted.proposal.source_sha256.as_str());
    if !identity_matches {
        reasons.push("source-identity-changed");
    }
    let duration_matches = asset
        .and_then(|asset| asset.metadata.duration)
        .and_then(|duration| duration.same_instant(accepted.source_duration).ok())
        .unwrap_or(false);
    if !duration_matches {
        reasons.push("source-duration-changed");
    }
    let current_transcript_sha = transcript(project, asset_id)
        .ok()
        .map(|t| transcript_sha256(&t))
        .transpose()?;
    if current_transcript_sha.as_deref() != Some(accepted.proposal.transcript_sha256.as_str()) {
        reasons.push("selected-transcript-changed");
    }
    let verified =
        identity_matches && asset.is_some_and(|asset| media::verify_asset(root, asset).is_ok());
    let state = if !reasons.is_empty() {
        "stale"
    } else if !verified {
        "unavailable"
    } else {
        "current"
    };
    let start = (offset as usize).min(accepted.proposal.words.len());
    let end = start
        .saturating_add(limit as usize)
        .min(accepted.proposal.words.len());
    let next = (end < accepted.proposal.words.len()).then_some(end);
    Ok(
        json!({"status":state,"revision":project.revision,"assetId":asset_id,
        "acceptedRevision":accepted.accepted_revision,"sourceSha256":accepted.proposal.source_sha256,"sourceDuration":accepted.source_duration,
        "sourceBytesVerified":verified,"transcriptSha256":accepted.proposal.transcript_sha256,
        "provider":accepted.proposal.provider,"providerVersion":accepted.proposal.provider_version,
        "review":accepted.review,"wordCount":accepted.proposal.words.len(),"items":&accepted.proposal.words[start..end],
        "nextOffset":next,"staleReasons":reasons,
        "guidance":"Only current, byte-verified alignments match the selected text and source; caller-reviewed timing remains a proposal and still requires source playback and accuracy review"}),
    )
}
