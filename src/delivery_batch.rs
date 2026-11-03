//! Read-only, per-output delivery checks for a frozen render batch.
//!
//! Each requested brief is evaluated independently against the corresponding
//! frozen child. No child failure hides a successful sibling. Metadata is
//! supplied by the caller and is not a platform acceptance or editorial claim.
use crate::{Error, Result, delivery_check, jobs::Job, store::Store};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

const MAX_OUTPUTS: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputBrief {
    pub sequence_id: String,
    pub brief: delivery_check::DeliveryBrief,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ContentMetadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchBrief {
    pub schema_version: u32,
    pub outputs: Vec<OutputBrief>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn valid_single_line(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

impl BatchBrief {
    /// Validate the whole request before opening the project or hashing files.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("batch delivery brief schemaVersion must be 1"));
        }
        if self.outputs.is_empty() || self.outputs.len() > MAX_OUTPUTS {
            return Err(invalid("batch delivery brief requires 1..32 output briefs"));
        }
        let mut ids = BTreeSet::new();
        for output in &self.outputs {
            // Sequences can be created through the workflow's ID grammar or
            // the wider AgentCut entity-ID grammar used by raw operations.
            if crate::workflow::id(&output.sequence_id).is_err()
                && !agentcut_core::is_entity_id(&output.sequence_id)
            {
                return Err(invalid("invalid batch delivery sequence ID"));
            }
            if !ids.insert(&output.sequence_id) {
                return Err(invalid("batch delivery contains duplicate sequence briefs"));
            }
            output.brief.validate()?;
            if let Some(metadata) = &output.metadata {
                if metadata
                    .title
                    .as_ref()
                    .is_some_and(|s| !valid_single_line(s, 1000))
                {
                    return Err(invalid(
                        "content title must be 1..1000 bytes of single-line text",
                    ));
                }
                if metadata.description.as_ref().is_some_and(|s| {
                    s.len() > 16_000
                        || s.chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                }) {
                    return Err(invalid(
                        "content description exceeds 16000 bytes or contains controls",
                    ));
                }
                if metadata.tags.len() > 100
                    || metadata.tags.iter().any(|s| !valid_single_line(s, 128))
                {
                    return Err(invalid(
                        "content tags require up to 100 single-line values of 1..128 bytes",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Check one frozen batch using caller-supplied briefs, without modifying jobs,
/// project revisions, source files or the delivery package.
pub fn check(
    root: &Path,
    batch_id: &str,
    request: &BatchBrief,
    backend: &FfmpegBackend,
) -> Result<Value> {
    request.validate()?;
    let root = root.canonicalize()?;
    let store = Store::open(&root)?;
    let batch = store.job(batch_id)?;
    if batch.state != "batch" {
        return Err(invalid("delivery batch check requires a render batch job"));
    }
    let revision = u64::try_from(batch.revision).map_err(|_| invalid("invalid batch revision"))?;
    let snapshot = store.revision(revision)?;
    if snapshot.revision != revision {
        return Err(invalid("batch revision does not match project history"));
    }
    let result = batch
        .result
        .as_ref()
        .ok_or_else(|| invalid("batch result missing"))?;
    if result["kind"] != "render-batch" {
        return Err(invalid("batch result is not a render batch"));
    }
    let ids = result["jobIds"]
        .as_array()
        .ok_or_else(|| invalid("batch children missing"))?;
    if ids.is_empty() || ids.len() > MAX_OUTPUTS {
        return Err(invalid("batch must contain 1..32 child jobs"));
    }
    let mut id_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids.iter().filter_map(Value::as_str) {
        *id_counts.entry(id).or_default() += 1;
    }
    let jobs: Vec<Option<Job>> = ids
        .iter()
        .map(|value| value.as_str().and_then(|id| store.job(id).ok()))
        .collect();
    let mut sequence_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for job in jobs.iter().flatten() {
        if let Some(input) = &job.input {
            *sequence_counts.entry(&input.sequence).or_default() += 1;
        }
    }
    let briefs: BTreeMap<&str, &OutputBrief> = request
        .outputs
        .iter()
        .map(|entry| (entry.sequence_id.as_str(), entry))
        .collect();
    let mut matched = BTreeSet::new();
    let mut items = Vec::with_capacity(ids.len());
    for (id_value, job) in ids.iter().zip(jobs.iter()) {
        let Some(id) = id_value.as_str().filter(|id| !id.is_empty()) else {
            items.push(json!({"jobId":null,"status":"unknown","reason":"invalid child job ID"}));
            continue;
        };
        let Some(job) = job else {
            items.push(json!({"jobId":id,"status":"unknown","reason":"child job is unavailable"}));
            continue;
        };
        let sequence = job.input.as_ref().map(|input| input.sequence.as_str());
        let trusted = id_counts.get(id) == Some(&1)
            && job.id == id
            && job.revision == batch.revision
            && job.input.as_ref().is_some_and(|input| {
                input.expected_revision == revision
                    && input.asr.is_none()
                    && input.analysis.is_none()
                    && sequence_counts.get(input.sequence.as_str()) == Some(&1)
                    && snapshot.require_sequence(&input.sequence).is_ok()
            });
        if !trusted {
            items.push(json!({"jobId":id,"state":job.state,"status":"unknown",
                "reason":"child job does not uniquely match the batch frozen revision and sequence"}));
            continue;
        }
        let sequence = sequence.expect("trusted input has a sequence");
        let entry = briefs.get(sequence).copied();
        if entry.is_some() {
            matched.insert(sequence);
        }
        let requested = entry.map(|item| &item.brief);
        let metadata = entry.and_then(|item| item.metadata.as_ref());
        let base = |status: &str, reason: &str| {
            json!({"jobId":id,"sequenceId":sequence,
            "revision":revision,"state":job.state,"status":status,"reason":reason,
            "brief":requested,"metadata":metadata,
            "metadataSource":if metadata.is_some() { Some("caller-supplied") } else { None }})
        };
        if matches!(job.state.as_str(), "failed" | "cancelled") {
            let mut item = base("fail", "render job did not succeed");
            item["jobError"] = json!(job.error);
            items.push(item);
            continue;
        }
        if job.state != "succeeded" {
            items.push(base(
                "unknown",
                "render job has no completed verified artifact",
            ));
            continue;
        }
        let Some(brief) = requested else {
            items.push(base(
                "unknown",
                "no delivery brief was supplied for this output",
            ));
            continue;
        };
        match delivery_check::check(&root, id, brief, backend) {
            Ok(report)
                if report["artifact"]["jobId"] == id
                    && report["artifact"]["sequenceId"] == sequence
                    && report["artifact"]["revision"] == revision =>
            {
                let status = report["status"].as_str().unwrap_or("unknown");
                let mut item = base(status, "brief checked against frozen render");
                item["report"] = report;
                items.push(item);
            }
            Ok(_) => items.push(base(
                "unknown",
                "verified artifact identity does not match the batch child",
            )),
            Err(error) => {
                let mut item = base("unknown", "artifact could not be verified for this brief");
                item["verificationError"] =
                    json!({"code":error.code(),"message":error.to_string()});
                items.push(item);
            }
        }
    }
    let unmatched: Vec<_> = request
        .outputs
        .iter()
        .filter(|entry| !matched.contains(entry.sequence_id.as_str()))
        .collect();
    let status = if items.iter().any(|item| item["status"] == "fail") {
        "fail"
    } else if !unmatched.is_empty() || items.iter().any(|item| item["status"] != "pass") {
        "unknown"
    } else {
        "pass"
    };
    Ok(
        json!({"schemaVersion":1,"batchId":batch.id,"projectId":snapshot.project_id,
        "frozenRevision":revision,"status":status,"brief":request,"items":items,"unmatchedBriefs":unmatched,
        "limitations":["Reports describe independent observations; rerun before delivery if jobs or artifacts change",
            "Supplied titles, descriptions and tags are caller-authored and not checked for suitability",
            "A passing check does not establish upload acceptance, caption accuracy or editorial approval",
            "Render manifests are not independently authenticated; this report is not automatically embedded in a delivery package"]}),
    )
}

fn html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn create_private_staging(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    // Supply the mode to mkdir itself, so no wider-permission directory exists
    // before the first manifest or media file is copied. The process umask can
    // make these owner permissions more restrictive, never grant extra access.
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_staging(_path: &Path) -> Result<()> {
    Err(invalid("private batch staging requires a Unix filesystem"))
}

// Every hash recorded by the brief must also match the independently copied
// package; a changed sibling must not inherit stale passing evidence.
fn verify_packaged(item: &Value, packaged: &Value, nested: &Path) -> Result<()> {
    let id = item["jobId"]
        .as_str()
        .ok_or_else(|| invalid("batch output job ID missing"))?;
    let report = &item["report"];
    let expected_sha = report["artifact"]["sha256"]
        .as_str()
        .ok_or_else(|| invalid("batch output verification hash missing"))?;
    let entries = packaged["manifest"]["items"]
        .as_array()
        .filter(|entries| entries.len() == 1)
        .ok_or_else(|| invalid("packaged output identity missing"))?;
    let entry = &entries[0];
    if entry["jobId"] != id
        || entry["state"] != "succeeded"
        || entry["revision"] != item["revision"]
        || entry["sequenceId"] != item["sequenceId"]
        || report["artifact"]["jobId"] != id
        || report["artifact"]["revision"] != item["revision"]
        || report["artifact"]["sequenceId"] != item["sequenceId"]
    {
        return Err(invalid(
            "packaged output identity does not match the checked batch child",
        ));
    }
    let files = entry["files"]
        .as_array()
        .ok_or_else(|| invalid("packaged output file list missing"))?;
    let verify_file = |name: &str, hash: &str| -> Result<()> {
        let mut matches = files.iter().filter(|file| file["path"] == name);
        let file = matches
            .next()
            .ok_or_else(|| invalid("checked package file is missing"))?;
        if matches.next().is_some()
            || file["sha256"] != hash
            || crate::media::hash_file(&nested.join(id).join(name))? != hash
        {
            return Err(invalid(
                "checked report and packaged file hashes do not match",
            ));
        }
        Ok(())
    };
    verify_file("video.mp4", expected_sha)?;
    let checks = report["checks"]
        .as_array()
        .ok_or_else(|| invalid("batch output checks missing"))?;
    for check in checks
        .iter()
        .filter(|check| check["constraint"] == "requiredSidecar")
    {
        let name = match check["requested"].as_str() {
            Some("srt") => "captions.srt",
            Some("vtt") => "captions.vtt",
            _ => return Err(invalid("unsupported checked sidecar")),
        };
        match check["actual"]["sha256"].as_str() {
            Some(hash) => verify_file(name, hash)?,
            None if check["status"] == "pass" => {
                return Err(invalid("passing sidecar check lacks a hash"));
            }
            None => {}
        }
    }
    Ok(())
}

// The general delivery package deliberately retains the full project snapshot
// and existing reviews. A checked batch is intended for external review, so
// copy only explicitly selected media and derive fresh, minimal manifests.
fn share_safe_output(
    item: &Value,
    packaged: &Value,
    nested: &Path,
    public_title: &str,
    project_id: &Value,
) -> Result<()> {
    const MEDIA: &[&str] = &[
        "video.mp4",
        "sheet.png",
        "captions.srt",
        "captions.vtt",
        "cover.png",
        "review-sdr.mp4",
    ];
    let id = item["jobId"]
        .as_str()
        .ok_or_else(|| invalid("batch output job ID missing"))?;
    let folder = nested.join(id);
    let original: Value = crate::json::read(&folder.join("manifest.json"))?;
    let preview = match &original["delivery"]["reviewVideo"] {
        Value::Null => "video.mp4",
        Value::String(name) if name == "video.mp4" => "video.mp4",
        Value::String(name) if name == "review-sdr.mp4" => "review-sdr.mp4",
        _ => return Err(invalid("unsupported review video in batch package")),
    };
    let listed = packaged["manifest"]["items"][0]["files"]
        .as_array()
        .ok_or_else(|| invalid("packaged output file list missing"))?;
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for file in listed {
        let name = file["path"]
            .as_str()
            .ok_or_else(|| invalid("packaged filename missing"))?;
        if !MEDIA.contains(&name) {
            continue;
        }
        if !seen.insert(name) {
            return Err(invalid("duplicate shareable package filename"));
        }
        let hash = file["sha256"]
            .as_str()
            .ok_or_else(|| invalid("packaged file hash missing"))?;
        let path = folder.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || crate::media::hash_file(&path)? != hash {
            return Err(invalid(
                "shareable package file changed during verification",
            ));
        }
        let mime = match name {
            "video.mp4" | "review-sdr.mp4" => "video/mp4",
            "sheet.png" | "cover.png" => "image/png",
            "captions.srt" => "application/x-subrip",
            "captions.vtt" => "text/vtt",
            _ => unreachable!(),
        };
        files.push(json!({"path":name,"sha256":hash,"bytes":metadata.len(),"mimeType":mime}));
    }
    if !seen.contains("video.mp4") || !seen.contains("sheet.png") || !seen.contains(preview) {
        return Err(invalid("required shareable package media is missing"));
    }
    // Only the selected, known media types and the derived manifest survive.
    // Unknown files are deliberately omitted, even if an older manifest lists
    // them, because they can contain unrelated project data.
    for entry in fs::read_dir(&folder)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "manifest.json" || name.to_str().is_some_and(|name| MEDIA.contains(&name)) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_file() || kind.is_symlink() {
            fs::remove_file(entry.path())?;
        } else {
            return Err(invalid("unexpected directory in shareable output"));
        }
    }
    let id_marker = format!("<section data-review-job='{id}' ");
    let title_marker = format!("<h2 id='title-{id}'>");
    let html_path = nested.join("index.html");
    let html = fs::read_to_string(&html_path)?;
    if html.matches(&id_marker).count() != 1
        || html.matches("<section data-review-job=").count() != 1
        || html.matches(&title_marker).count() != 1
    {
        return Err(invalid("review page layout cannot be safely minimized"));
    }
    let title_start = html
        .find(&title_marker)
        .ok_or_else(|| invalid("review title missing"))?
        + title_marker.len();
    let title_end = title_start
        + html[title_start..]
            .find("</h2>")
            .ok_or_else(|| invalid("review title is malformed"))?;
    let html = format!(
        "{}{}{}",
        &html[..title_start],
        html_text(public_title),
        &html[title_end..]
    );
    let section_start = html
        .find(&id_marker)
        .ok_or_else(|| invalid("review section missing"))?;
    let video_end = section_start
        + html[section_start..]
            .find("</video>")
            .ok_or_else(|| invalid("review video element missing"))?
        + "</video>".len();
    let controls = video_end
        + html[video_end..]
            .find("<fieldset data-feedback-reference hidden>")
            .ok_or_else(|| invalid("review controls missing"))?;
    let section_end = video_end
        + html[video_end..]
            .find("</section>")
            .ok_or_else(|| invalid("review section is malformed"))?;
    if controls >= section_end {
        return Err(invalid("review controls lie outside their output"));
    }
    // Remove the entire edit-objective/cut-reason segment, retaining only the
    // fixed SDR-preview warning when applicable. Feedback controls and frozen
    // identity attributes remain intact for review-import.
    let label = if preview == "review-sdr.mp4" {
        "<p>SDR review preview. Download the source-preserving master for color evaluation on a compatible display.</p>"
    } else {
        ""
    };
    let html = format!("{}{label}{}", &html[..video_end], &html[controls..]);
    let artifact = &item["report"]["artifact"];
    let shared_output = json!({"schemaVersion":1,"format":"avw-share-render","projectId":project_id,
        "jobId":id,"artifactId":artifact["artifactId"],"revision":item["revision"],"sequenceId":item["sequenceId"],
        "output":"video.mp4","verification":{"sha256":artifact["sha256"],"bytes":artifact["bytes"]},
        "reviewVideo":preview,"files":files,
        "notice":"Project snapshots, edit notes and saved reviews are omitted; selected media and captions are shared intact."});
    let shared_review = json!({"schemaVersion":1,"format":"avw-share-review","batchId":id,"projectId":project_id,
        "frozenRevision":item["revision"],"items":[{"jobId":id,"state":"succeeded","revision":item["revision"],
            "sequenceId":item["sequenceId"],"files":files,"review":"pending"}],"editorialReview":"pending",
        "notice":"Project snapshots, edit notes and saved reviews are omitted; selected media and captions are shared intact."});
    // These paths were newly created inside a private, unpublished staging
    // directory. Replace their contents and sync before publishing the parent.
    for (path, data) in [
        (
            folder.join("manifest.json"),
            serde_json::to_vec_pretty(&shared_output)?,
        ),
        (
            nested.join("manifest.json"),
            serde_json::to_vec_pretty(&shared_review)?,
        ),
        (html_path, html.into_bytes()),
    ] {
        let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    File::open(&folder)?.sync_all()?;
    File::open(nested)?.sync_all()?;
    Ok(())
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn publish_no_replace(parent: &File, staging: &Path, destination: &Path) -> Result<()> {
    use nix::fcntl::{RenameFlags, renameat2};
    renameat2(
        parent,
        staging,
        parent,
        destination,
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| Error::Io(std::io::Error::from_raw_os_error(error as i32)))?;
    Ok(())
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn publish_no_replace(_parent: &File, _staging: &Path, _destination: &Path) -> Result<()> {
    Err(invalid(
        "atomic batch delivery publication requires Linux GNU",
    ))
}

/// Publish a checked batch delivery to a new destination. Each verified output
/// is packaged independently so a bad sibling does not hide a good one.
/// Caller metadata and checks are included in the root manifest. A unique
/// staging directory carries an incomplete marker until the root files are
/// durable; publication refuses to replace an existing destination.
pub fn package(
    root: &Path,
    batch_id: &str,
    destination: &Path,
    request: &BatchBrief,
    backend: &FfmpegBackend,
) -> Result<Value> {
    let mut report = check(root, batch_id, request, backend)?;
    let root = root.canonicalize()?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let target = destination
        .file_name()
        .ok_or_else(|| invalid("batch delivery destination has no filename"))?;
    let parent_file = File::open(&parent)?;
    let staging_name = format!(".avw-batch-{}.incomplete", uuid::Uuid::new_v4());
    let staging = parent.join(&staging_name);
    create_private_staging(&staging)?;
    let result = (|| -> Result<Value> {
        write_new(
            &staging.join("delivery.incomplete"),
            b"Checked batch delivery is incomplete",
        )?;
        let outputs = staging.join("outputs");
        fs::create_dir(&outputs)?;
        let mut html = String::from(
            "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content='width=device-width, initial-scale=1'><title>Batch video review</title><style>body{font-family:system-ui,sans-serif;margin:1rem auto;padding:0 1rem;max-width:60rem}section{margin:0 0 2rem}a,h2,p{overflow-wrap:anywhere}</style><body><main><h1>Batch video review</h1><p>Review every output and its delivery checks before sharing.</p>",
        );
        let project_id = report["projectId"].clone();
        let items = report["items"]
            .as_array_mut()
            .ok_or_else(|| invalid("batch report items missing"))?;
        let mut count = 0_usize;
        for (index, item) in items.iter_mut().enumerate() {
            let id = item["jobId"].as_str().unwrap_or("").to_owned();
            let safe_id = uuid::Uuid::parse_str(&id).is_ok_and(|uuid| uuid.to_string() == id);
            let title = item["metadata"]["title"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Output {}", index + 1));
            let check_status = item["status"].as_str().unwrap_or("unknown").to_owned();
            let mut link = None;
            if safe_id && item["report"].is_object() {
                let nested = outputs.join(&id);
                let child_result =
                    crate::delivery::package(&root, &id, &nested).and_then(|packaged| {
                        verify_packaged(item, &packaged, &nested)?;
                        share_safe_output(item, &packaged, &nested, &title, &project_id)
                    });
                match child_result {
                    Ok(()) => {
                        count += 1;
                        let review = format!("outputs/{id}/index.html");
                        let master = format!("outputs/{id}/{id}/video.mp4");
                        item["package"] =
                            json!({"status":"packaged","reviewPath":review,"masterPath":master});
                        link = Some(review);
                    }
                    Err(error) => {
                        if nested.exists() {
                            fs::remove_dir_all(&nested)?;
                        }
                        item["package"] = json!({"status":"unavailable","reason":"packaging or verification failed","errorCode":error.code()});
                    }
                }
            } else {
                item["package"] = json!({"status":"unavailable","reason":"no uniquely identified verified artifact to package"});
            }
            // Single-output reports contain private local paths. Keep only
            // package-relative paths in the embedded check report.
            if let Some(report) = item.get_mut("report").and_then(Value::as_object_mut)
                && let Some(artifact) = report.get_mut("artifact").and_then(Value::as_object_mut)
            {
                artifact.remove("path");
                artifact.remove("manifest");
            }
            if let Some(object) = item.as_object_mut() {
                object.remove("jobError");
            }
            if let Some(error) = item
                .get_mut("verificationError")
                .and_then(Value::as_object_mut)
            {
                error.remove("message");
            }
            html.push_str(&format!(
                "<section><h2>{}</h2><p>Delivery checks: {}</p>",
                html_text(&title),
                html_text(&check_status)
            ));
            if let Some(link) = link {
                html.push_str(&format!(
                    "<p><a href=\"{}\">Review this output</a></p>",
                    html_text(&link)
                ));
            } else {
                html.push_str("<p>This output could not be packaged.</p>");
            }
            html.push_str("</section>");
        }
        html.push_str("</main></body></html>");
        let item_count = items.len();
        let complete = count == item_count
            && report["unmatchedBriefs"]
                .as_array()
                .is_some_and(Vec::is_empty);
        if let Some(limits) = report["limitations"].as_array_mut() {
            for limitation in limits {
                if limitation
                    .as_str()
                    .is_some_and(|s| s.contains("not automatically embedded"))
                {
                    *limitation = json!("Render manifests are not independently authenticated");
                }
            }
        }
        report["package"] = json!({"format":"avw-checked-batch-delivery","version":1,
            "state":if complete {"complete"} else {"partial"},"packagedOutputs":count,
            "notes":["A complete package can contain failed or unknown delivery checks; review each output",
                "Caller-supplied metadata and checks are not platform acceptance or editorial approval",
                "Project snapshots, saved reviews, edit objectives and cut notes are omitted; review pages accept new feedback",
                "Selected media, captions, images and caller-authored metadata are shared intact and may contain sensitive content",
                "The published package directory remains owner-only; grant access deliberately if another OS user must serve it",
                "A crash can leave an owner-only local .avw-batch-*.incomplete staging directory containing private intermediate data; do not share it"]});
        File::open(&outputs)?.sync_all()?;
        write_new(
            &staging.join("manifest.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        write_new(&staging.join("index.html"), html.as_bytes())?;
        fs::remove_file(staging.join("delivery.incomplete"))?;
        File::open(&staging)?.sync_all()?;
        publish_no_replace(&parent_file, Path::new(&staging_name), Path::new(target))?;
        parent_file.sync_all()?;
        Ok(json!({"path":parent.join(target),"manifest":report}))
    })();
    if result.is_err() && staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    result
}

#[cfg(test)]
mod package_tests {
    use super::*;

    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn staging_is_owner_only_from_creation_through_publication() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("private.incomplete");
        let destination = temp.path().join("published");
        create_private_staging(&staging).unwrap();
        let initial = fs::metadata(&staging).unwrap().permissions().mode() & 0o777;
        assert_eq!(initial & 0o077, 0, "staging grants group/other access");
        fs::write(staging.join("private-intermediate"), b"fixture").unwrap();
        let parent = File::open(temp.path()).unwrap();
        publish_no_replace(
            &parent,
            Path::new("private.incomplete"),
            Path::new("published"),
        )
        .unwrap();
        let published = fs::metadata(&destination).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            published, initial,
            "publication changed directory permissions"
        );
        assert_eq!(
            published & 0o077,
            0,
            "published directory grants group/other access"
        );
    }

    #[test]
    fn copied_master_and_checked_sidecar_must_match_recorded_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let nested = temp.path().join("package");
        let output = nested.join(&id);
        fs::create_dir_all(&output).unwrap();
        let video = output.join("video.mp4");
        let caption = output.join("captions.srt");
        fs::write(&video, b"master bytes").unwrap();
        fs::write(&caption, b"caption bytes").unwrap();
        let video_hash = crate::media::hash_file(&video).unwrap();
        let caption_hash = crate::media::hash_file(&caption).unwrap();
        let item = json!({"jobId":id,"revision":1,"sequenceId":"square",
            "report":{"artifact":{"jobId":id,"revision":1,"sequenceId":"square","sha256":video_hash},
                "checks":[{"constraint":"requiredSidecar","requested":"srt","status":"pass","actual":{"sha256":caption_hash}}]}});
        let packaged = json!({"manifest":{"items":[{"jobId":id,"state":"succeeded","revision":1,
            "sequenceId":"square","files":[{"path":"video.mp4","sha256":video_hash},
                {"path":"captions.srt","sha256":caption_hash}]}]}});
        verify_packaged(&item, &packaged, &nested).unwrap();
        fs::write(&caption, b"changed").unwrap();
        assert!(verify_packaged(&item, &packaged, &nested).is_err());
        fs::write(&caption, b"caption bytes").unwrap();
        fs::write(&video, b"changed").unwrap();
        assert!(verify_packaged(&item, &packaged, &nested).is_err());
    }
}
