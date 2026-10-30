//! Subtitle and delivery files are derived from the same frozen timeline.
use crate::{
    Error, Result, media,
    process::{self, Control},
};
use agentcut_core::{ItemPayload, Project, RationalRate, RoundingMode, Sequence};
use agentcut_render::FfmpegBackend;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    time::Duration,
};

fn timestamp(ms: i64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn editing_notes(manifest: &Value) -> String {
    let record = &manifest["editPreflight"]["editPlan"];
    let Some(objective) = record["brief"]["objective"].as_str() else {
        return String::new();
    };
    let mut html = format!("<p data-edit-objective>{}</p>", html_text(objective));
    if record["status"] == "stale" {
        html.push_str("<p>This edit or its review context has changed since these notes. Review this version again.</p>");
    }
    if let Some(decisions) = record["decisions"].as_array() {
        html.push_str("<details><summary>Why these cuts</summary><ol>");
        for decision in decisions.iter().take(20) {
            if let Some(reason) = decision["reason"].as_str() {
                html.push_str(&format!("<li data-cut-reason>{}</li>", html_text(reason)));
            }
        }
        html.push_str("</ol></details>");
        if decisions.len() > 20 {
            html.push_str("<p>Further cut notes are included in the manifest.</p>");
        }
    }
    html
}

// The reference can be selected and copied. User-supplied titles are read
// from textContent and never interpreted as markup.
const REVIEW_SCRIPT: &str = r#"<script>
for (const section of document.querySelectorAll('[data-review-job]')) {
  const video = section.querySelector('video');
  const fieldset = section.querySelector('[data-feedback-reference]');
  const reference = fieldset.querySelector('textarea');
  const status = fieldset.querySelector('[role=status]');
  const rangeField = section.querySelector('[data-feedback-range]');
  const rangeReference = rangeField.querySelector('textarea');
  const rangeStatus = rangeField.querySelector('[role=status]');
  const rangeCopy = rangeField.querySelector('[data-range-copy]');
  const reviewLimitMs = Number(section.dataset.reviewLimitMs);
  let rangeStart = null;
  let rangeEnd = null;
  const formatTime = ms => {
    const hours = Math.floor(ms / 3600000);
    const minutes = Math.floor(ms / 60000) % 60;
    const seconds = Math.floor(ms / 1000) % 60;
    return String(hours).padStart(2, '0') + ':' + String(minutes).padStart(2, '0') + ':'
      + String(seconds).padStart(2, '0') + '.' + String(ms % 1000).padStart(3, '0');
  };
  const update = () => {
    const ms = Number.isFinite(video.currentTime) ? Math.max(0, Math.round(video.currentTime * 1000)) : 0;
    const title = section.querySelector('h2').textContent.replace(/\s+/g, ' ').trim() || 'Video';
    fieldset.querySelector('[data-feedback-time]').textContent = formatTime(ms);
    reference.value = 'Video: ' + title
      + '\nRender job ID: ' + section.dataset.reviewJob
      + '\nRendered revision: ' + section.dataset.reviewRevision
      + '\nPlayback time: ' + formatTime(ms) + ' (' + ms + ' ms)';
  };
  const updateRange = () => {
    rangeField.querySelector('[data-range-start]').textContent = rangeStart === null ? 'Not set' : formatTime(rangeStart);
    rangeField.querySelector('[data-range-end]').textContent = rangeEnd === null ? 'Not set' : formatTime(rangeEnd);
    const ready = rangeStart !== null && rangeEnd !== null;
    rangeCopy.disabled = !ready;
    const title = section.querySelector('h2').textContent.replace(/\s+/g, ' ').trim() || 'Video';
    rangeReference.value = ready ? 'Video: ' + title
      + '\nRender job ID: ' + section.dataset.reviewJob
      + '\nRendered revision: ' + section.dataset.reviewRevision
      + '\nRange start: ' + formatTime(rangeStart) + ' (atMs: ' + rangeStart + ')'
      + '\nRange end: ' + formatTime(rangeEnd) + ' (endMs: ' + rangeEnd + ')'
      : 'Set a start and an end to create a range reference.';
  };
  const position = () => video.readyState >= 1 && Number.isFinite(video.currentTime)
    ? Math.max(0, Math.round(video.currentTime * 1000)) : null;
  rangeField.querySelector('[data-range-mark-start]').addEventListener('click', () => {
    video.pause();
    const ms = position();
    rangeStart = null;
    rangeEnd = null;
    if (ms === null) {
      rangeStatus.textContent = 'Load the video before setting a start.';
    } else if (!Number.isSafeInteger(reviewLimitMs) || ms >= reviewLimitMs) {
      rangeStatus.textContent = 'Choose a start before the end of the reviewed output.';
    } else {
      rangeStart = ms;
      rangeStatus.textContent = 'Start set. Seek to a later point and set the end.';
    }
    updateRange();
  });
  rangeField.querySelector('[data-range-mark-end]').addEventListener('click', () => {
    video.pause();
    const ms = position();
    rangeEnd = null;
    if (rangeStart === null) {
      rangeStatus.textContent = 'Set a range start first.';
    } else if (ms === null) {
      rangeStatus.textContent = 'Load the video before setting an end.';
    } else if (ms <= rangeStart) {
      rangeStatus.textContent = 'Choose an end after the start.';
    } else if (!Number.isSafeInteger(reviewLimitMs) || ms > reviewLimitMs) {
      rangeStatus.textContent = 'Choose an end within the reviewed output.';
    } else {
      rangeEnd = ms;
      rangeStatus.textContent = 'Range selected. Copy the reference for your agent.';
    }
    updateRange();
  });
  rangeField.querySelector('[data-range-clear]').addEventListener('click', () => {
    rangeStart = null;
    rangeEnd = null;
    rangeStatus.textContent = 'Range cleared.';
    updateRange();
  });
  rangeCopy.addEventListener('click', async () => {
    if (rangeStart === null || rangeEnd === null) return;
    try {
      if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(rangeReference.value);
      rangeStatus.textContent = 'Range reference copied.';
    } catch (_) {
      rangeReference.focus();
      rangeReference.select();
      rangeStatus.textContent = 'Range reference selected. Copy it with your usual keyboard shortcut.';
    }
  });
  video.addEventListener('timeupdate', update);
  video.addEventListener('seeked', update);
  fieldset.querySelector('button').addEventListener('click', async () => {
    video.pause();
    update();
    try {
      if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(reference.value);
      status.textContent = 'Feedback reference copied.';
    } catch (_) {
      reference.focus();
      reference.select();
      status.textContent = 'Reference selected. Copy it with your usual keyboard shortcut.';
    }
  });
  update();
  updateRange();
  fieldset.hidden = false;
  rangeField.hidden = false;
}
</script>"#;

pub fn sidecars(sequence: &Sequence) -> Result<(String, String, Value)> {
    let mut cues = Vec::new();
    // The renderer applies solo separately to visual and audio tracks, and
    // clears disabled solos before normalization. Match the burned captions.
    let solo = sequence
        .tracks
        .iter()
        .any(|t| t.enabled && t.solo && t.track_type.is_visual());
    for item in sequence
        .tracks
        .iter()
        .filter(|t| t.enabled && t.track_type.is_visual() && (!solo || t.solo))
        .flat_map(|t| &t.items)
        .filter(|i| i.enabled)
    {
        if let ItemPayload::Caption(caption) = &item.payload {
            let start = (item.start.as_seconds_f64() * 1000.0).round() as i64;
            let end = (item.end_exclusive()?.as_seconds_f64() * 1000.0).round() as i64;
            cues.push((
                start,
                end,
                caption.text.clone(),
                item.id.clone(),
                item.extensions.get("avw.sourceCue").cloned(),
            ));
        }
    }
    cues.sort_by_key(|c| (c.0, c.1, c.3.clone()));
    let mut srt = String::new();
    let mut vtt = "WEBVTT\n\n".to_owned();
    let mut mapping = Vec::new();
    for (index, (start, end, text, id, binding)) in cues.iter().enumerate() {
        // A blank line terminates a subtitle cue. Keep authored words while
        // removing empty visual lines so one cue cannot inject another cue.
        let text = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        srt.push_str(&format!(
            "{}\n{} --> {}\n{text}\n\n",
            index + 1,
            timestamp(*start, ','),
            timestamp(*end, ',')
        ));
        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        vtt.push_str(&format!(
            "{}\n{} --> {}\n{escaped}\n\n",
            index + 1,
            timestamp(*start, '.'),
            timestamp(*end, '.')
        ));
        mapping.push(json!({"itemId":id,"startMs":start,"endMs":end,"sourceCue":binding}));
    }
    Ok((
        srt,
        vtt,
        json!({"cues":mapping,"lossReport":["Font, outline, positioning and other visual styles are retained in burned captions and omitted from plain SRT/VTT"]}),
    ))
}
pub fn extras(
    path: &Path,
    directory: &Path,
    sequence: &Sequence,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<Value> {
    let (srt, vtt, mapping) = sidecars(sequence)?;
    let mut files = Vec::new();
    for (name, bytes, mime) in [
        ("captions.srt", srt.as_bytes(), "application/x-subrip"),
        ("captions.vtt", vtt.as_bytes(), "text/vtt"),
    ] {
        let target = directory.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        files.push(json!({"path":name,"sha256":media::hash_file_controlled(&target,control)?,"mimeType":mime}));
    }
    let cover = directory.join("cover.png");
    process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
            .arg(path)
            .args([
                "-vf",
                "scale=1280:1280:force_original_aspect_ratio=decrease",
                "-frames:v",
                "1",
            ])
            .arg(&cover),
        Duration::from_secs(60),
        control,
    )?;
    File::open(&cover)?.sync_all()?;
    files.push(json!({"path":"cover.png","sha256":media::hash_file_controlled(&cover,control)?,"mimeType":"image/png","outputFrame":0}));
    Ok(json!({"files":files,"captions":mapping,"editorialReview":"pending"}))
}
pub fn package(root: &Path, id: &str, destination: &Path) -> Result<Value> {
    let store = crate::store::Store::open(root)?;
    let job = store.job(id)?;
    let ids = if job.state == "batch" {
        job.result
            .as_ref()
            .and_then(|r| r["jobIds"].as_array())
            .ok_or_else(|| Error::Invalid("batch items missing".into()))?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::Invalid("invalid batch job ID".into()))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![id.to_owned()]
    };
    std::fs::create_dir(destination)?;
    std::fs::write(
        destination.join("delivery.incomplete"),
        b"Package not complete",
    )?;
    let mut items = Vec::new();
    let mut html = String::from(
        "<!doctype html><html lang=en><meta charset=utf-8>\
         <meta name=viewport content='width=device-width, initial-scale=1'>\
         <title>Video review</title><style>\
         body{font-family:system-ui,sans-serif;margin:1rem auto;padding:0 1rem;max-width:60rem}\
         section{margin:0 0 2rem}video{display:block;width:360px;max-width:100%;height:auto}\
         a,h2,p,li{overflow-wrap:anywhere}\
         fieldset{box-sizing:border-box;max-width:36rem;margin:1rem 0}\
         textarea{box-sizing:border-box;width:100%;resize:vertical;font:inherit}\
         button{min-height:2.75rem} [role=status]{min-height:1.5em}\
         </style><body><main><h1>Video review</h1>",
    );
    for id in ids {
        let item = store.job(&id)?;
        if item.state != "succeeded" {
            items.push(json!({"jobId":id,"state":item.state,"error":item.error}));
            continue;
        }
        let result = item
            .result
            .ok_or_else(|| Error::Invalid("job result missing".into()))?;
        let manifest_path = Path::new(
            result["manifest"]
                .as_str()
                .ok_or_else(|| Error::Invalid("video manifest missing".into()))?,
        );
        if !manifest_path.starts_with(root) {
            return Err(Error::Invalid("artifact manifest outside project".into()));
        }
        let manifest: Value = crate::json::read(manifest_path)?;
        let source = manifest_path
            .parent()
            .ok_or_else(|| Error::Invalid("manifest parent missing".into()))?;
        let folder = destination.join(&id);
        std::fs::create_dir(&folder)?;
        let mut files = vec![
            json!({"path":"video.mp4","sha256":manifest["verification"]["sha256"]}),
            json!({"path":"sheet.png","sha256":manifest["contactSheet"]["sha256"]}),
        ];
        files.extend(
            manifest["delivery"]["files"]
                .as_array()
                .cloned()
                .ok_or_else(|| {
                    Error::Invalid(
                        "artifact predates delivery files; re-render captured revision".into(),
                    )
                })?,
        );
        for file in &files {
            let name = file["path"]
                .as_str()
                .ok_or_else(|| Error::Invalid("delivery filename missing".into()))?;
            if Path::new(name).components().count() != 1 {
                return Err(Error::Invalid("invalid delivery filename".into()));
            }
            let input = source.join(name);
            if media::hash_file(&input)? != file["sha256"] {
                return Err(Error::Invalid("delivery artifact bytes changed".into()));
            }
            let output = folder.join(name);
            std::fs::copy(&input, &output)?;
            File::open(&output)?.sync_all()?;
            if media::hash_file(&output)? != file["sha256"] {
                return Err(Error::Invalid("delivery copy verification failed".into()));
            }
        }
        std::fs::copy(manifest_path, folder.join("manifest.json"))?;
        File::open(folder.join("manifest.json"))?.sync_all()?;
        // IDs are UUIDs from the queue, not user supplied markup.
        let preview = if manifest["delivery"]["reviewVideo"] == "review-sdr.mp4" {
            "review-sdr.mp4"
        } else {
            "video.mp4"
        };
        let label = if preview == "review-sdr.mp4" {
            "<p>SDR review preview. Download the source-preserving master for color evaluation on a compatible display.</p>"
        } else {
            ""
        };
        let frozen: Project = serde_json::from_value(manifest["snapshot"].clone())?;
        let sequence_id = manifest["sequenceId"]
            .as_str()
            .ok_or_else(|| Error::Invalid("delivery sequence missing".into()))?;
        let sequence = frozen.require_sequence(sequence_id)?;
        let review_limit_ms = sequence
            .content_duration()?
            .rescale_to(RationalRate::frames(1000)?, RoundingMode::Floor)?
            .value
            .max(0);
        let title = &sequence.name;
        let notes = editing_notes(&manifest);
        html.push_str(&format!("<section data-review-job='{id}' data-review-revision='{revision}' data-review-limit-ms='{review_limit_ms}'><h2 id='title-{id}'>{title}</h2><video controls width=360 aria-labelledby='title-{id}' src='{id}/{preview}'></video>{label}{notes}<fieldset data-feedback-reference hidden><legend>Share feedback at this moment</legend><p>Pause at the moment you want to discuss, then copy this reference for your agent.</p><p>Playback time: <span data-feedback-time></span></p><label for='feedback-{id}'>Feedback reference</label><textarea id='feedback-{id}' rows=6 readonly></textarea><button type=button>Copy feedback reference</button><p role=status aria-live=polite></p></fieldset><details data-feedback-range hidden><summary>Share feedback for a range</summary><fieldset><legend>Mark a range</legend><p>Pause at the beginning and set the start, then seek to the end and set it.</p><p>Start: <span data-range-start>Not set</span><br>End: <span data-range-end>Not set</span></p><button type=button data-range-mark-start>Set range start</button> <button type=button data-range-mark-end>Set range end</button> <button type=button data-range-clear>Clear range</button><p><label for='range-{id}'>Range reference</label></p><textarea id='range-{id}' rows=6 readonly></textarea><button type=button data-range-copy disabled>Copy range reference</button><p role=status aria-live=polite></p></fieldset></details><noscript><p>For feedback, include render job ID {id}, rendered revision {revision}, and the playback time. For a range, include its start and end times.</p></noscript><p><a href='{id}/video.mp4'>Master</a> · <a href='{id}/captions.vtt'>Captions</a> · <a href='{id}/manifest.json'>Manifest</a></p></section>",title=html_text(title),revision=item.revision));
        items.push(json!({"jobId":id,"state":"succeeded","revision":item.revision,"sequenceId":manifest["sequenceId"],"files":files,"review":"pending","editPlan":manifest["editPreflight"]["editPlan"]}));
    }
    html.push_str("</main>");
    html.push_str(REVIEW_SCRIPT);
    html.push_str("</body></html>");
    let report = json!({"schemaVersion":1,"batchId":job.id,"frozenRevision":job.revision,"items":items,"reviews":store.project()?.extensions.get("avw.reviews"),"editorialReview":"pending"});
    for (name, bytes) in [
        ("manifest.json", serde_json::to_vec_pretty(&report)?),
        ("index.html", html.into_bytes()),
    ] {
        let mut file = File::create(destination.join(name))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::remove_file(destination.join("delivery.incomplete"))?;
    File::open(destination)?.sync_all()?;
    Ok(json!({"path":destination,"manifest":report}))
}
