//! Compare the source coordinates of two frozen, verified plain-cut renders.
use crate::{Error, Result, assembly, media, store::Store};
use agentcut_core::{ItemPayload, Project, RationalRate, RationalTime};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MAX_PLACEMENTS: usize = 4_000;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn scoped_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
    .canonicalize()?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(invalid(
            "comparison artifact is outside the project or is not a file",
        ));
    }
    Ok(resolved)
}

#[derive(Clone)]
pub(crate) struct Frozen {
    pub(crate) snapshot: Project,
    pub(crate) sequence_id: String,
    pub(crate) identity: Value,
    pub(crate) manifest: Value,
}

pub(crate) fn artifact(store: &Store, root: &Path, id: &str) -> Result<Frozen> {
    let job = store.job(id)?;
    if job.state != "succeeded" {
        return Err(invalid(format!("render job {id} is not succeeded")));
    }
    let result = job
        .result
        .as_ref()
        .ok_or_else(|| invalid("render result missing"))?;
    let revision = u64::try_from(job.revision).map_err(|_| invalid("invalid job revision"))?;
    let snapshot = store.revision(revision)?;
    let manifest_path = scoped_file(
        root,
        Path::new(
            result["manifest"]
                .as_str()
                .ok_or_else(|| invalid("job has no render manifest"))?,
        ),
    )?;
    let manifest: Value = crate::json::read(&manifest_path)?;
    let sequence_id = manifest["sequenceId"]
        .as_str()
        .ok_or_else(|| invalid("manifest sequence missing"))?;
    let artifact_id = result["artifactId"]
        .as_str()
        .ok_or_else(|| invalid("render artifact ID missing"))?;
    if manifest["projectId"] != json!(snapshot.project_id)
        || manifest["revision"] != revision
        || result["revision"] != revision
        || manifest["artifactId"] != artifact_id
        || manifest["snapshot"] != serde_json::to_value(&snapshot)?
    {
        return Err(invalid(
            "render manifest does not match its frozen project revision",
        ));
    }
    snapshot.require_sequence(sequence_id)?;
    let sequence_binding = if let Some(input) = &job.input {
        if input.sequence != sequence_id
            || input.analysis.is_some()
            || input.asr.is_some()
            || input.expected_revision != revision
            || artifact_id != format!("{}-{}", job.id, job.attempt)
        {
            return Err(invalid(
                "render manifest does not match its frozen job input",
            ));
        }
        "job-input"
    } else {
        if artifact_id != job.id {
            return Err(invalid("render artifact ID does not match its job"));
        }
        "manifest-only"
    };
    let master = scoped_file(
        root,
        Path::new(
            result["path"]
                .as_str()
                .ok_or_else(|| invalid("render path missing"))?,
        ),
    )?;
    let parent = manifest_path
        .parent()
        .ok_or_else(|| invalid("manifest directory missing"))?;
    if scoped_file(root, &parent.join("video.mp4"))? != master || manifest["output"] != "video.mp4"
    {
        return Err(invalid("render path and manifest disagree"));
    }
    let master_sha = media::hash_file(&master)?;
    if manifest["verification"]["sha256"] != master_sha {
        return Err(invalid("rendered master bytes changed"));
    }
    let declared_review = &manifest["delivery"]["reviewVideo"];
    if !declared_review.is_null()
        && declared_review != "video.mp4"
        && declared_review != "review-sdr.mp4"
    {
        return Err(invalid("render declares an unsupported review preview"));
    }
    let (review_path, review_sha, role) = if *declared_review == "review-sdr.mp4" {
        let expected = manifest["delivery"]["files"]
            .as_array()
            .and_then(|files| files.iter().find(|file| file["path"] == "review-sdr.mp4"))
            .and_then(|file| file["sha256"].as_str())
            .ok_or_else(|| invalid("review preview verification missing"))?;
        let path = scoped_file(root, &parent.join("review-sdr.mp4"))?;
        let sha = media::hash_file(&path)?;
        if sha != expected {
            return Err(invalid("review preview bytes changed"));
        }
        (path, sha, "sdr-review-preview")
    } else {
        (master.clone(), master_sha.clone(), "master")
    };
    let sequence = snapshot.require_sequence(sequence_id)?;
    let identity = json!({"jobId":id,"artifactId":artifact_id,"revision":revision,"sequenceId":sequence_id,
        "name":sequence.name,"sequenceBinding":sequence_binding,"canvas":sequence.canvas,"frameRate":sequence.frame_rate,
        "outputColorFromManifest":manifest["outputColor"],
        "master":{"path":master,"sha256":master_sha,"bytes":std::fs::metadata(&master)?.len()},
        "review":{"path":review_path,"sha256":review_sha,"bytes":std::fs::metadata(&review_path)?.len(),"role":role},
        "snapshotMatchesProjectHistory":true,
        "integrity":"Video and review bytes match hashes in the manifest; the manifest itself is not independently authenticated"});
    Ok(Frozen {
        snapshot,
        sequence_id: sequence_id.to_owned(),
        identity,
        manifest: manifest.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceId {
    asset_id: String,
    sha256: String,
}

#[derive(Debug, Clone)]
struct Clip {
    item_id: String,
    start: RationalTime,
    end: RationalTime,
    output: RationalTime,
}

#[derive(Debug, Clone)]
struct Placement {
    item_id: String,
    start: RationalTime,
    end: RationalTime,
}

fn compare_time(a: RationalTime, b: RationalTime) -> Ordering {
    (i128::from(a.value) * i128::from(a.rate.denominator) * i128::from(b.rate.numerator))
        .cmp(&(i128::from(b.value) * i128::from(b.rate.denominator) * i128::from(a.rate.numerator)))
}

fn exact(a: RationalTime, b: RationalTime, subtract: bool) -> Result<RationalTime> {
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
        .ok_or_else(|| invalid("comparison times require an unsupported common clock"))?;
    let rate = RationalRate::frames(ticks)?;
    let (a, b) = (a.rescale_exact(rate)?, b.rescale_exact(rate)?);
    Ok(if subtract {
        a.checked_sub(b)?
    } else {
        a.checked_add(b)?
    })
}

fn clips(frozen: &Frozen) -> Result<std::result::Result<BTreeMap<SourceId, Vec<Clip>>, String>> {
    let sequence = frozen.snapshot.require_sequence(&frozen.sequence_id)?;
    if assembly::policy(sequence)?.is_none() {
        return Ok(Err(
            "only source-preserving plain-cut assemblies can be compared".into(),
        ));
    }
    let items = match assembly::clips(sequence) {
        Ok(items) => items,
        Err(Error::Invalid(message)) => return Ok(Err(message)),
        Err(error) => return Err(error),
    };
    let mut sources: BTreeMap<SourceId, Vec<Clip>> = BTreeMap::new();
    for item in items {
        let ItemPayload::Clip(clip) = &item.payload else {
            unreachable!()
        };
        let asset = frozen.snapshot.require_asset(&clip.asset_id)?;
        let hash = asset
            .fingerprint
            .sha256
            .as_ref()
            .filter(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| invalid(format!("source {} lacks a SHA-256 identity", asset.id)))?;
        let start = clip.source_range.start;
        let end = clip.source_range.end_exclusive()?;
        start.rate.normalized()?;
        end.rate.normalized()?;
        item.start.rate.normalized()?;
        sources
            .entry(SourceId {
                asset_id: asset.id.clone(),
                sha256: hash.clone(),
            })
            .or_default()
            .push(Clip {
                item_id: item.id.clone(),
                start,
                end,
                output: item.start,
            });
    }
    Ok(Ok(sources))
}

fn placements(clips: &[Clip], start: RationalTime, end: RationalTime) -> Result<Vec<Placement>> {
    let mut result = Vec::new();
    for clip in clips {
        if clip.start.checked_cmp(start)?.is_le() && clip.end.checked_cmp(end)?.is_ge() {
            let output_start = exact(clip.output, exact(start, clip.start, true)?, false)?;
            let output_end = exact(clip.output, exact(end, clip.start, true)?, false)?;
            result.push(Placement {
                item_id: clip.item_id.clone(),
                start: output_start,
                end: output_end,
            });
        }
    }
    result.sort_by(|a, b| compare_time(a.start, b.start).then(a.item_id.cmp(&b.item_id)));
    Ok(result)
}

#[derive(Debug)]
struct Slice {
    source: SourceId,
    start: RationalTime,
    end: RationalTime,
    baseline: Vec<Placement>,
    candidate: Vec<Placement>,
    reordered: bool,
}

fn render_placements(items: &[Placement]) -> Vec<Value> {
    items
        .iter()
        .map(|p| json!({"itemId":p.item_id,"outputStart":p.start,"outputEnd":p.end}))
        .collect()
}

/// Compare frozen outputs without mutating history, jobs, or the original media.
pub fn compare_outputs(root: &Path, baseline: &str, candidate: &str) -> Result<Value> {
    let root = root.canonicalize()?;
    let store = Store::open(&root)?;
    let left = artifact(&store, &root, baseline)?;
    let right = if baseline == candidate {
        left.clone()
    } else {
        artifact(&store, &root, candidate)?
    };
    let mut limitations = Vec::new();
    if left.identity["sequenceBinding"] != "job-input" {
        limitations.push("baseline: synchronous render does not bind its sequence in the job record; re-render with render-start".to_string());
    }
    if right.identity["sequenceBinding"] != "job-input" {
        limitations.push("candidate: synchronous render does not bind its sequence in the job record; re-render with render-start".to_string());
    }
    if !limitations.is_empty() {
        return Ok(
            json!({"status":"unsupported","baseline":left.identity,"candidate":right.identity,"limitations":limitations}),
        );
    }
    let left_clips = clips(&left)?
        .map_err(|error| limitations.push(format!("baseline: {error}")))
        .ok();
    let right_clips = clips(&right)?
        .map_err(|error| limitations.push(format!("candidate: {error}")))
        .ok();
    let (Some(left_clips), Some(right_clips)) = (left_clips, right_clips) else {
        return Ok(
            json!({"status":"unsupported","baseline":left.identity,"candidate":right.identity,"limitations":limitations,
            "guidance":"Only normal-speed source-preserving plain cuts are mapped. Review layered, styled, transitioned or speed-altered outputs directly."}),
        );
    };
    let mut groups: BTreeMap<SourceId, (Vec<Clip>, Vec<Clip>)> = BTreeMap::new();
    for (source, clips) in left_clips {
        groups.entry(source).or_default().0 = clips;
    }
    for (source, clips) in right_clips {
        groups.entry(source).or_default().1 = clips;
    }
    let mut slices = Vec::new();
    let mut total_placements = 0usize;
    for (source, (a, b)) in groups {
        let mut boundaries: Vec<_> = a.iter().chain(&b).flat_map(|c| [c.start, c.end]).collect();
        boundaries.sort_by(|a, b| compare_time(*a, *b));
        boundaries.dedup_by(|a, b| compare_time(*a, *b) == Ordering::Equal);
        for pair in boundaries.windows(2) {
            let baseline = placements(&a, pair[0], pair[1])?;
            let candidate = placements(&b, pair[0], pair[1])?;
            if baseline.is_empty() && candidate.is_empty() {
                continue;
            }
            total_placements += baseline.len() + candidate.len();
            if total_placements > MAX_PLACEMENTS {
                return Err(invalid(
                    "comparison exceeds 4000 interval placements; compare shorter outputs",
                ));
            }
            slices.push(Slice {
                source: source.clone(),
                start: pair[0],
                end: pair[1],
                baseline,
                candidate,
                reordered: false,
            });
        }
    }
    for i in 0..slices.len() {
        for j in i + 1..slices.len() {
            let (a, b) = (&slices[i], &slices[j]);
            if a.baseline.len() != 1
                || a.candidate.len() != 1
                || b.baseline.len() != 1
                || b.candidate.len() != 1
            {
                continue;
            }
            let left_order = compare_time(a.baseline[0].start, b.baseline[0].start);
            let right_order = compare_time(a.candidate[0].start, b.candidate[0].start);
            if left_order != Ordering::Equal
                && right_order != Ordering::Equal
                && left_order != right_order
            {
                slices[i].reordered = true;
                slices[j].reordered = true;
            }
        }
    }
    let mut retained = Vec::new();
    let mut added = Vec::new();
    let mut omitted = Vec::new();
    let ambiguous_retained = slices.iter().any(|slice| {
        !slice.baseline.is_empty()
            && !slice.candidate.is_empty()
            && (slice.baseline.len() > 1 || slice.candidate.len() > 1)
    });
    for slice in &slices {
        let repeated = slice.baseline.len() > 1 || slice.candidate.len() > 1;
        let value = json!({"assetId":slice.source.asset_id,"sourceSha256":slice.source.sha256,
            "sourceStart":slice.start,"sourceEnd":slice.end,
            "baselinePlacements":render_placements(&slice.baseline),"candidatePlacements":render_placements(&slice.candidate),
            "baselineCount":slice.baseline.len(),"candidateCount":slice.candidate.len(),
            "repeatedInBaseline":slice.baseline.len()>1,"repeatedInCandidate":slice.candidate.len()>1,
            "mapping":if slice.baseline.is_empty() || slice.candidate.is_empty() {"unpaired"} else if repeated {"ambiguous"} else {"unique"},
            "reordered":if repeated || slice.baseline.is_empty() || slice.candidate.is_empty() || (ambiguous_retained && !slice.reordered) {None} else {Some(slice.reordered)}});
        if slice.baseline.is_empty() {
            added.push(value);
        } else if slice.candidate.is_empty() {
            omitted.push(value);
        } else {
            retained.push(value);
        }
    }
    Ok(
        json!({"status":"complete","scope":{"kind":"source-video-ranges","sourceIdentity":"asset-id-and-sha256",
        "notCompared":["audio","color","quality","effects","framing","cadence","pixel-content","appearance"],
        "guidance":"Identical source coverage does not imply identical media. Byte-identical assets with different IDs remain distinct."},
        "baseline":left.identity,"candidate":right.identity,
        "retained":retained,"added":added,"omitted":omitted,
        "limits":{"maxCutsPerOutput":100,"maxIntervalPlacements":MAX_PLACEMENTS},
        "guidance":"Relative order is assessed only for uniquely placed retained intervals. If any retained material repeats, an unproven reorder is unknown. Time mappings do not judge meaning, color, sound or appearance."}),
    )
}
