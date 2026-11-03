//! Review caller-supplied caption translations against a frozen sequence.
//!
//! This module does not translate, identify a language, or certify caption
//! accuracy. It returns an editable existing `studio::Variant` request.
use crate::{Error, Result, store::Store, studio::Edit, workflow};
use agentcut_core::{ItemPayload, Project, Sequence};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const MAX_CAPTIONS: usize = 3_000;
const MAX_TRANSLATION_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionTranslation {
    pub caption_id: String,
    pub source_text_sha256: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationProposal {
    pub schema_version: u32,
    pub project_id: String,
    pub source_revision: u64,
    pub sequence: String,
    pub sequence_sha256: String,
    pub output_id: String,
    pub output_name: String,
    pub target_language: String,
    pub provider: String,
    pub provider_version: String,
    pub translations: Vec<CaptionTranslation>,
}

/// Optional, attributable input for a variant operation. The digest binds the
/// entire explicit proposal; it is not evidence of translation quality.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationProvenance {
    pub schema_version: u32,
    pub proposal_sha256: String,
    pub proposal: TranslationProposal,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptionRow {
    caption_id: String,
    text: String,
    text_sha256: String,
    start: agentcut_core::RationalTime,
    duration: agentcut_core::RationalTime,
    visible: bool,
    source_cue: Option<Value>,
    source_asset_sha256: Option<String>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn value_digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(digest(&serde_json::to_vec(value)?))
}
fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn label(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

// This is a conservative syntactic subset, not full BCP 47 validation or
// proof that supplied caption text is in the named language.
fn language(value: &str) -> bool {
    if value.len() > 35 {
        return false;
    }
    let mut parts = value.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    (2..=8).contains(&primary.len())
        && primary.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|c| c.is_ascii_alphanumeric())
        })
}

fn rows(project: &Project, sequence: &Sequence) -> Result<Vec<CaptionRow>> {
    let solo = sequence
        .tracks
        .iter()
        .any(|track| track.enabled && track.solo && track.track_type.is_visual());
    let mut found = Vec::new();
    let mut ids = BTreeSet::new();
    for track in &sequence.tracks {
        for item in &track.items {
            let ItemPayload::Caption(caption) = &item.payload else {
                continue;
            };
            if found.len() >= MAX_CAPTIONS {
                return Err(invalid(format!(
                    "source sequence exceeds {MAX_CAPTIONS} captions"
                )));
            }
            if !ids.insert(item.id.as_str()) {
                return Err(invalid("source sequence has ambiguous caption IDs"));
            }
            let binding = item.extensions.get("avw.sourceCue").cloned();
            let source_hash = binding
                .as_ref()
                .and_then(|value| value["assetId"].as_str())
                .and_then(|id| project.assets.iter().find(|asset| asset.id == id))
                .and_then(|asset| asset.fingerprint.sha256.clone());
            found.push(CaptionRow {
                caption_id: item.id.clone(),
                text: caption.text.clone(),
                text_sha256: digest(caption.text.as_bytes()),
                start: item.start,
                duration: item.duration,
                visible: track.enabled
                    && track.track_type.is_visual()
                    && (!solo || track.solo)
                    && item.enabled,
                source_cue: binding,
                source_asset_sha256: source_hash,
            });
        }
    }
    Ok(found)
}

/// Read bounded source-caption context from a revision-pinned snapshot.
pub fn context(
    project: &Project,
    sequence: &str,
    expected_revision: u64,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    workflow::id(sequence)?;
    if project.revision != expected_revision {
        return Err(Error::Conflict {
            expected: expected_revision,
            current: project.revision,
        });
    }
    if limit == 0 || limit > 100 {
        return Err(invalid("caption context limit must be 1..100"));
    }
    let sequence = project.require_sequence(sequence)?;
    let captions = rows(project, sequence)?;
    let start = (offset as usize).min(captions.len());
    let end = start.saturating_add(limit as usize).min(captions.len());
    Ok(
        json!({"schemaVersion":1,"projectId":project.project_id,"sourceRevision":project.revision,
        "sequence":sequence.id,"sequenceSha256":value_digest(sequence)?,"canvas":sequence.canvas,
        "captionCount":captions.len(),"visibleCaptionCount":captions.iter().filter(|row|row.visible).count(),
        "items":&captions[start..end],"nextOffset":if end<captions.len(){Some(end)}else{None},
        "guidance":"Supply reviewed translations for every caption, including hidden captions; the existing variant operation requires complete coverage. Source hashes are stored project identities, not newly verified bytes."}),
    )
}

struct Prepared {
    captions: BTreeMap<String, String>,
    visible_count: usize,
    total_count: usize,
}

fn validate(project: &Project, proposal: &TranslationProposal) -> Result<Prepared> {
    if proposal.schema_version != 1 || proposal.project_id != project.project_id.to_string() {
        return Err(invalid(
            "translation proposal requires schemaVersion 1 and the matching project ID",
        ));
    }
    if proposal.source_revision != project.revision {
        return Err(Error::Conflict {
            expected: proposal.source_revision,
            current: project.revision,
        });
    }
    workflow::id(&proposal.sequence)?;
    workflow::id(&proposal.output_id)?;
    if !label(&proposal.output_name, 200)
        || !label(&proposal.provider, 120)
        || !label(&proposal.provider_version, 120)
        || !language(&proposal.target_language)
    {
        return Err(invalid(
            "translation output name, provider/version or target language is invalid",
        ));
    }
    let sequence = project.require_sequence(&proposal.sequence)?;
    if !sha256(&proposal.sequence_sha256) || proposal.sequence_sha256 != value_digest(sequence)? {
        return Err(invalid(
            "translation proposal sequence SHA-256 is stale or invalid",
        ));
    }
    let captions = rows(project, sequence)?;
    if captions.is_empty() || captions.len() > MAX_CAPTIONS {
        return Err(invalid(format!(
            "translation requires 1..={MAX_CAPTIONS} source captions"
        )));
    }
    let visible_count = captions.iter().filter(|row| row.visible).count();
    if visible_count == 0 {
        return Err(invalid(
            "source sequence has no visible captions to translate",
        ));
    }
    let source: BTreeMap<_, _> = captions
        .iter()
        .map(|caption| (caption.caption_id.as_str(), caption))
        .collect();
    let mut translated = BTreeMap::new();
    let mut total_bytes = 0_usize;
    for item in &proposal.translations {
        workflow::id(&item.caption_id)?;
        let caption = source.get(item.caption_id.as_str()).ok_or_else(|| {
            invalid(format!(
                "translation refers to unknown caption {}",
                item.caption_id
            ))
        })?;
        if !sha256(&item.source_text_sha256) || item.source_text_sha256 != caption.text_sha256 {
            return Err(invalid(format!(
                "source text for caption {} is stale or invalid",
                item.caption_id
            )));
        }
        if item.text.trim().is_empty()
            || item.text.len() > 2000
            || item
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(invalid(format!(
                "translated caption {} requires 1..2000 bytes of text",
                item.caption_id
            )));
        }
        total_bytes = total_bytes
            .checked_add(item.text.len())
            .ok_or_else(|| invalid("translation text size overflow"))?;
        if total_bytes > MAX_TRANSLATION_BYTES {
            return Err(invalid("translation text exceeds the 1 MiB proposal limit"));
        }
        if translated
            .insert(item.caption_id.clone(), item.text.clone())
            .is_some()
        {
            return Err(invalid(format!(
                "duplicate translation for caption {}",
                item.caption_id
            )));
        }
    }
    if translated.len() != captions.len() {
        let missing: Vec<_> = captions
            .iter()
            .filter(|caption| !translated.contains_key(&caption.caption_id))
            .take(8)
            .map(|caption| caption.caption_id.as_str())
            .collect();
        return Err(invalid(format!(
            "translations required for every caption, including hidden captions; missing {}",
            missing.join(", ")
        )));
    }
    Ok(Prepared {
        captions: translated,
        visible_count,
        total_count: captions.len(),
    })
}

/// Check provenance and exact variant intent against its frozen source.
/// A caller/provider assertion is retained as an assertion, never as a
/// verification that the translation is correct or linguistically suitable.
pub fn validate_variant_provenance(
    source: &Project,
    variant: &Edit,
    provenance: &TranslationProvenance,
) -> Result<()> {
    if provenance.schema_version != 1
        || !sha256(&provenance.proposal_sha256)
        || provenance.proposal_sha256 != value_digest(&provenance.proposal)?
    {
        return Err(invalid("translation provenance digest is invalid"));
    }
    let prepared = validate(source, &provenance.proposal)?;
    let Edit::Variant {
        source_revision,
        sequence,
        id,
        name,
        width,
        height,
        language,
        captions,
        ..
    } = variant
    else {
        return Err(invalid("translation provenance requires a variant edit"));
    };
    let source_sequence = source.require_sequence(&provenance.proposal.sequence)?;
    if *source_revision != provenance.proposal.source_revision
        || sequence != &provenance.proposal.sequence
        || id != &provenance.proposal.output_id
        || name != &provenance.proposal.output_name
        || *width != source_sequence.canvas.width
        || *height != source_sequence.canvas.height
        || language.as_deref() != Some(provenance.proposal.target_language.as_str())
        || captions != &prepared.captions
    {
        return Err(invalid(
            "translation provenance does not match the requested variant",
        ));
    }
    Ok(())
}

/// Validate every provided translation, execute the existing variant dry-run,
/// and return its editable request and attributable proposal.
pub fn plan(
    root: &Path,
    project_path: &Path,
    proposal: &TranslationProposal,
    expected_revision: u64,
    key: &str,
) -> Result<Value> {
    if key.trim().is_empty() || !(8..=200).contains(&key.len()) {
        return Err(invalid("idempotency key requires 8..200 nonblank bytes"));
    }
    let mut store = Store::open(root)?;
    let project = store.project()?;
    if project.revision != expected_revision {
        return Err(Error::Conflict {
            expected: expected_revision,
            current: project.revision,
        });
    }
    let prepared = validate(&project, proposal)?;
    let sequence = project.require_sequence(&proposal.sequence)?;
    let provenance = TranslationProvenance {
        schema_version: 1,
        proposal_sha256: value_digest(proposal)?,
        proposal: proposal.clone(),
    };
    let mut edit = json!({"action":"variant","sourceRevision":proposal.source_revision,
        "sequence":proposal.sequence,"id":proposal.output_id,"name":proposal.output_name,
        "width":sequence.canvas.width,"height":sequence.canvas.height,"language":proposal.target_language,"captions":prepared.captions});
    edit["translationProvenance"] = serde_json::to_value(&provenance)?;
    let parsed: Edit = serde_json::from_value(edit.clone())?;
    validate_variant_provenance(&project, &parsed, &provenance)?;
    let outcome = store.studio(&parsed, expected_revision, key, true)?;
    let request = json!({"command":"studio","project":project_path,"expectedRevision":expected_revision,
        "key":key,"edit":edit,"dryRun":true});
    Ok(
        json!({"schemaVersion":1,"status":"ready","proposalSha256":provenance.proposal_sha256,
        "proposal":proposal,"sourceRevision":expected_revision,"visibleCaptionCount":prepared.visible_count,
        "captionCount":prepared.total_count,"variantDryRun":outcome,"studioRequest":request,
        "providerProvenanceIncluded":true,
        "guidance":"Submit the returned studio request with dryRun false to create the reviewed variant; its caller-supplied provenance is stored atomically if the edit is submitted unchanged.",
        "limitations":["No translation, language, speaker or transcription accuracy is established", "Font glyph coverage, shaping, RTL and visual appearance require render and human review", "The variant reflows caption whitespace using existing font metrics", "Source asset hashes in context are stored metadata, not fresh byte verification", "Provider provenance is caller-supplied and is not independently authenticated"]}),
    )
}
