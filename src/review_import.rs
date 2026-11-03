//! Atomically import creator-authored feedback from an offline review page.
//!
//! The draft is untrusted input. It identifies one frozen render and contains
//! only author-selected notes. Source anchors are derived again from verified
//! project history, never taken from the draft.
use crate::{Error, Result, compare, delivery, store::Store, studio, workflow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};
use uuid::Uuid;

const MAX_ITEMS: usize = 100;
const MAX_ACTOR_BYTES: usize = 200;
const MAX_TEXT_BYTES: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDraft {
    pub schema_version: u32,
    pub draft_id: String,
    pub project_id: String,
    pub job_id: String,
    pub artifact_revision: u64,
    pub artifact_sha256: String,
    pub items: Vec<ReviewDraftItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ReviewDraftDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDraftItem {
    pub id: String,
    pub at_ms: i64,
    pub end_ms: i64,
    pub actor: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDraftDecision {
    pub status: ReviewDecisionStatus,
    pub actor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewDecisionStatus {
    Approved,
    ChangesRequested,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

impl ReviewDraft {
    /// Validate bounded fields without touching the project or its artifacts.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("review draft schemaVersion must be 1"));
        }
        if !canonical_uuid(&self.draft_id) || !canonical_uuid(&self.project_id) {
            return Err(invalid(
                "draftId and projectId must be canonical lowercase UUIDs",
            ));
        }
        workflow::id(&self.job_id)?;
        if self.artifact_sha256.len() != 64
            || !self
                .artifact_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("artifactSha256 must be a lowercase SHA-256 digest"));
        }
        if self.items.len() > MAX_ITEMS || (self.items.is_empty() && self.decision.is_none()) {
            return Err(invalid(format!(
                "review draft requires a decision or 1..{MAX_ITEMS} items"
            )));
        }
        if let Some(decision) = &self.decision
            && (decision.actor.trim().is_empty() || decision.actor.len() > MAX_ACTOR_BYTES)
        {
            return Err(invalid(format!(
                "review decision actor requires 1..{MAX_ACTOR_BYTES} nonblank bytes"
            )));
        }
        let mut ids = HashSet::new();
        for item in &self.items {
            workflow::id(&item.id)?;
            if !ids.insert(&item.id) {
                return Err(invalid(format!(
                    "duplicate review draft item ID {}",
                    item.id
                )));
            }
            if item.at_ms < 0 || item.end_ms <= item.at_ms {
                return Err(invalid(format!(
                    "review item {} needs a positive interval",
                    item.id
                )));
            }
            if item.actor.trim().is_empty() || item.actor.len() > MAX_ACTOR_BYTES {
                return Err(invalid(format!(
                    "review item {} actor requires 1..{MAX_ACTOR_BYTES} nonblank bytes",
                    item.id
                )));
            }
            if item.text.trim().is_empty() || item.text.len() > MAX_TEXT_BYTES {
                return Err(invalid(format!(
                    "review item {} text requires 1..{MAX_TEXT_BYTES} nonblank bytes",
                    item.id
                )));
            }
        }
        Ok(())
    }
}

fn review_id(draft: &ReviewDraft, item: &ReviewDraftItem) -> Result<String> {
    // A draft's item ID is local to that draft. Bind its globally stable ID
    // to the project and render as well, so two pages cannot collide merely
    // because both called a note `note-1`. The whole digest avoids truncation.
    let identity =
        serde_json::to_vec(&[&draft.project_id, &draft.job_id, &draft.draft_id, &item.id])?;
    Ok(format!("review_{:x}", Sha256::digest(identity)))
}

fn decision_id(draft: &ReviewDraft) -> Result<String> {
    let identity = serde_json::to_vec(&[&draft.project_id, &draft.job_id, &draft.draft_id])?;
    Ok(format!("decision_{:x}", Sha256::digest(identity)))
}

/// Atomically add all notes to project history, or add none on failure.
///
/// Repeating the same key and request returns the recorded outcome even if the
/// original artifact subsequently becomes unavailable. A changed request with
/// the same key conflicts; a new key cannot overwrite previously imported IDs.
pub fn import(
    root: &Path,
    draft: &ReviewDraft,
    expected: u64,
    key: &str,
    dry_run: bool,
) -> Result<Value> {
    draft.validate()?;
    if key.trim().is_empty() || !(8..=200).contains(&key.len()) {
        return Err(invalid("idempotency key requires 8..200 nonblank bytes"));
    }
    let root = root.canonicalize()?;
    let mut store = Store::open(&root)?;
    // A separate read connection is opened before Store::change starts its
    // write transaction; verification then happens inside the change closure,
    // after replay/conflict checks, without borrowing the writer recursively.
    let verifier = Store::open(&root)?;
    let ids = draft
        .items
        .iter()
        .map(|item| review_id(draft, item))
        .collect::<Result<Vec<_>>>()?;
    let decision_id = draft
        .decision
        .as_ref()
        .map(|_| decision_id(draft))
        .transpose()?;
    let request = json!({"kind":"review.import","draft":draft,"expectedRevision":expected});
    let outcome = store.change(key, expected, request, dry_run, |current| {
        if current.project_id.to_string() != draft.project_id {
            return Err(invalid("review draft belongs to a different project"));
        }
        let frozen = compare::artifact(&verifier, &root, &draft.job_id)?;
        if frozen.identity["revision"] != draft.artifact_revision
            || frozen.identity["master"]["sha256"] != draft.artifact_sha256
            || frozen.snapshot.project_id != current.project_id
        {
            return Err(invalid("review draft does not match the frozen render identity"));
        }
        let sequence = frozen.snapshot.require_sequence(&frozen.sequence_id)?;
        let mut next = current.clone();
        if !draft.items.is_empty() {
            let duration = delivery::review_duration(sequence, &frozen.manifest).map_err(|_| {
                invalid("reviewed output timing is unavailable or inconsistent; re-render this output before importing feedback")
            })?;
            let reviews = next.extensions.entry("avw.reviews".into()).or_insert(json!([]))
                .as_array_mut().ok_or_else(|| invalid("invalid reviews"))?;
            for (item, id) in draft.items.iter().zip(&ids) {
                if workflow::time(item.end_ms).checked_cmp(duration)?.is_gt() {
                    return Err(invalid(format!("review item {} extends beyond artifact duration", item.id)));
                }
                if reviews.iter().any(|review| review["id"] == *id) {
                    return Err(invalid(format!("review item {} was already imported; use a new draft for a new note", item.id)));
                }
                let mapping = studio::capture_review_json(&frozen.snapshot, sequence, item.at_ms, item.end_ms)?;
                reviews.push(json!({"id":id,"jobId":draft.job_id,"artifactSha256":draft.artifact_sha256,
                    "artifactRevision":draft.artifact_revision,"sequenceId":frozen.sequence_id,
                    "atMs":item.at_ms,"endMs":item.end_ms,"actor":item.actor,"text":item.text,
                    "sources":mapping["sources"],"sourceRanges":mapping["sourceRanges"],
                    "sourceRangeCoverage":mapping["sourceRangeCoverage"],"sourceRangeLimitations":mapping["sourceRangeLimitations"],
                    "draftId":draft.draft_id,"draftItemId":item.id,"state":"open"}));
            }
        }
        if let (Some(decision), Some(id)) = (&draft.decision, &decision_id) {
            let decisions = next.extensions.entry("avw.reviewDecisions".into()).or_insert(json!([]))
                .as_array_mut().ok_or_else(|| invalid("invalid review decisions"))?;
            if decisions.iter().any(|record| record["id"] == *id) {
                return Err(invalid("a decision from this draft was already imported; use a new draft to record another decision"));
            }
            let recorded_revision = current.revision.checked_add(1).ok_or_else(|| invalid("revision overflow"))?;
            decisions.push(json!({"id":id,"draftId":draft.draft_id,"jobId":draft.job_id,
                "artifactSha256":draft.artifact_sha256,"artifactRevision":draft.artifact_revision,
                "sequenceId":frozen.sequence_id,"status":decision.status,"actor":decision.actor,
                "recordedRevision":recorded_revision}));
        }
        // Detect changes to the master or its declared review preview while
        // mapping the notes. The manifest remains locally trusted as in compare.
        let after = compare::artifact(&verifier, &root, &draft.job_id)?;
        if after.identity != frozen.identity || after.manifest != frozen.manifest {
            return Err(invalid("reviewed artifact changed during feedback import"));
        }
        Ok(next)
    })?;
    let mut result = json!({"draftId":draft.draft_id,"jobId":draft.job_id,
        "items":draft.items.iter().zip(&ids).map(|(item,id)|json!({"draftItemId":item.id,"reviewId":id})).collect::<Vec<_>>(),
        "outcome":outcome});
    if let (Some(decision), Some(id)) = (&draft.decision, &decision_id) {
        result["decision"] = json!({"decisionId":id,"status":decision.status,"actor":decision.actor,
            "jobId":draft.job_id,"artifactRevision":draft.artifact_revision,"artifactSha256":draft.artifact_sha256});
    }
    Ok(result)
}
