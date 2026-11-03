//! Subtitle and delivery files are derived from the same frozen timeline.
use crate::{
    Error, Result, media,
    process::{self, Control},
};
use agentcut_core::{ItemPayload, Project, RationalRate, RationalTime, RoundingMode, Sequence};
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

// SRT and WebVTT represent timestamps in whole milliseconds. Preserve the
// exporter's existing nearest-millisecond convention (positive half values
// round up), but calculate it exactly so fractional frame rates and large
// timestamps cannot be changed by a float conversion.
fn sidecar_milliseconds(time: RationalTime, item_id: &str) -> Result<i64> {
    if time.value < 0 || time.rate.numerator == 0 || time.rate.denominator == 0 {
        return Err(Error::Invalid(format!(
            "caption {item_id} has an invalid sidecar timestamp"
        )));
    }
    // Even the largest i64 value multiplied by u32::MAX and 1000 fits i128.
    let numerator = i128::from(time.value) * i128::from(time.rate.denominator) * 1000;
    let denominator = i128::from(time.rate.numerator);
    let rounded =
        numerator / denominator + i128::from((numerator % denominator) * 2 >= denominator);
    i64::try_from(rounded).map_err(|_| {
        Error::Invalid(format!(
            "caption {item_id} exceeds the sidecar timestamp range"
        ))
    })
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
// Generate one durable identity for each page-local draft.  Never fall back
// to time or Math.random: the importer uses this ID to make retries safe.
function newReviewDraftId() {
  try {
    if (!globalThis.crypto) return null;
    if (typeof globalThis.crypto.randomUUID === 'function') {
      try { return globalThis.crypto.randomUUID(); } catch (_) { /* Try getRandomValues. */ }
    }
    if (typeof globalThis.crypto.getRandomValues !== 'function') return null;
    const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    const hex = [...bytes].map(b => b.toString(16).padStart(2, '0')).join('');
    return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
  } catch (_) { return null; }
}
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
  const rangeTimingKnown = section.dataset.reviewLimitMs !== ''
    && Number.isSafeInteger(reviewLimitMs) && reviewLimitMs > 0;
  const composer = section.querySelector('[data-feedback-draft]');
  const draftNotice = section.querySelector('[data-feedback-draft-unavailable]');
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
  // Notes are local until explicitly copied or downloaded.  Text is always
  // written through textContent/value and never interpreted as HTML.
  const draftId = newReviewDraftId();
  const uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
  if (uuidV4.test(draftId || '') && typeof TextEncoder === 'function'
      && /^[0-9a-f]{64}$/.test(section.dataset.reviewSha256 || '')
      && section.dataset.reviewProject && Number.isSafeInteger(Number(section.dataset.reviewRevision))) {
    const draft = { schemaVersion: 1, projectId: section.dataset.reviewProject,
      jobId: section.dataset.reviewJob, artifactRevision: Number(section.dataset.reviewRevision),
      artifactSha256: section.dataset.reviewSha256, draftId, items: [] };
    let nextId = 1;
    const actor = composer.querySelector('[data-draft-actor]');
    const comment = composer.querySelector('[data-draft-text]');
    const list = composer.querySelector('[data-draft-list]');
    const serialized = composer.querySelector('[data-draft-json]');
    const draftStatus = composer.querySelector('[role=status]');
    const copy = composer.querySelector('[data-draft-copy]');
    const download = composer.querySelector('[data-draft-download]');
    const pointButton = composer.querySelector('[data-draft-add-point]');
    const rangeButton = composer.querySelector('[data-draft-add-range]');
    const choice = composer.querySelector('[data-draft-decision-choice]');
    const decisionLabel = composer.querySelector('[data-draft-decision]');
    const clearDecision = composer.querySelector('[data-draft-clear-decision]');
    pointButton.disabled = !rangeTimingKnown;
    rangeButton.disabled = !rangeTimingKnown;
    const utf8 = new TextEncoder();
    const hasDraftContent = () => draft.items.length > 0 || draft.decision !== undefined;
    const validActor = () => {
      if (actor.value.trim() && utf8.encode(actor.value).length <= 200) return true;
      draftStatus.textContent = 'Enter a name of at most 200 UTF-8 bytes.';
      actor.focus();
      return false;
    };
    const updateDraft = () => {
      list.replaceChildren();
      for (const note of draft.items) {
        const entry = document.createElement('li');
        const label = document.createElement('span');
        label.textContent = `${formatTime(note.atMs)}–${formatTime(note.endMs)} · ${note.actor}: ${note.text} `;
        entry.append(label);
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.textContent = 'Remove';
        remove.setAttribute('aria-label', `Remove feedback ${note.id}`);
        remove.addEventListener('click', () => {
          draft.items = draft.items.filter(item => item.id !== note.id);
          updateDraft();
          draftStatus.textContent = 'Feedback removed from this page-local draft.';
        });
        entry.append(remove);
        list.append(entry);
      }
      decisionLabel.textContent = draft.decision
        ? `Draft decision: ${draft.decision.status === 'approved' ? 'Approved' : 'Changes requested'} by ${draft.decision.actor}`
        : 'Draft decision: None';
      clearDecision.disabled = !draft.decision;
      serialized.value = hasDraftContent() ? JSON.stringify(draft, null, 2) : '';
      copy.disabled = !hasDraftContent();
      download.disabled = !hasDraftContent();
    };
    const addNote = (at, end) => {
      if (draft.items.length >= 100) {
        draftStatus.textContent = 'A draft can contain at most 100 feedback notes.';
      } else if (!validActor()) {
        return;
      } else if (!comment.value.trim() || utf8.encode(comment.value).length > 4000) {
        draftStatus.textContent = 'Enter feedback of at most 4000 UTF-8 bytes.';
        comment.focus();
      } else if (!Number.isSafeInteger(at) || !Number.isSafeInteger(end)
                 || at < 0 || at >= end || end > reviewLimitMs) {
        draftStatus.textContent = 'Choose a valid time or marked range within this output.';
      } else {
        draft.items.push({ id: `note-${nextId++}`, atMs: at, endMs: end,
          actor: actor.value, text: comment.value });
        comment.value = '';
        updateDraft();
        draftStatus.textContent = 'Feedback added. Copy or download the draft before leaving this page.';
      }
    };
    pointButton.addEventListener('click', () => {
      if (!rangeTimingKnown) return;
      video.pause();
      const ms = position();
      if (ms === null || ms >= reviewLimitMs) {
        draftStatus.textContent = 'Load the video and choose a time before the output ends.';
        return;
      }
      // A point is represented as a one-millisecond half-open interval.
      addNote(ms, ms + 1);
    });
    rangeButton.addEventListener('click', () => {
      if (!rangeTimingKnown) return;
      if (rangeStart === null || rangeEnd === null) {
        draftStatus.textContent = 'Mark a range above before adding range feedback.';
        return;
      }
      addNote(rangeStart, rangeEnd);
    });
    composer.querySelector('[data-draft-set-decision]').addEventListener('click', () => {
      if (!['approved', 'changes-requested'].includes(choice.value)) {
        draftStatus.textContent = 'Choose a version decision before setting it.';
        choice.focus();
      } else if (validActor()) {
        draft.decision = { status: choice.value, actor: actor.value };
        updateDraft();
        draftStatus.textContent = 'Decision added to this local draft. Copy or download it to share.';
      }
    });
    clearDecision.addEventListener('click', () => {
      delete draft.decision;
      choice.value = '';
      updateDraft();
      draftStatus.textContent = 'Decision removed from this local draft.';
    });
    composer.querySelector('[data-draft-new]').addEventListener('click', () => {
      if ((hasDraftContent() || comment.value.trim()) && !globalThis.confirm('Start a new draft? Copy or download this draft first; its unsaved content will be cleared.')) return;
      const nextDraftId = newReviewDraftId();
      if (!uuidV4.test(nextDraftId || '') || nextDraftId === draft.draftId) {
        draftStatus.textContent = 'Secure draft creation is unavailable in this browser.';
        return;
      }
      draft.draftId = nextDraftId;
      draft.items = [];
      delete draft.decision;
      nextId = 1;
      choice.value = '';
      comment.value = '';
      updateDraft();
      draftStatus.textContent = 'New draft started for this version.';
    });
    copy.addEventListener('click', async () => {
      if (!hasDraftContent()) return;
      try {
        if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('Clipboard unavailable');
        await navigator.clipboard.writeText(serialized.value);
        draftStatus.textContent = 'Feedback draft copied.';
      } catch (_) {
        serialized.focus();
        serialized.select();
        draftStatus.textContent = 'Feedback draft selected. Copy it with your usual keyboard shortcut.';
      }
    });
    download.addEventListener('click', () => {
      if (!hasDraftContent()) return;
      try {
        const url = URL.createObjectURL(new Blob([serialized.value + '\n'], {type: 'application/json'}));
        const link = document.createElement('a');
        link.href = url;
        link.download = `feedback-${section.dataset.reviewJob}.json`;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        draftStatus.textContent = 'Feedback draft download started.';
      } catch (_) {
        serialized.focus();
        serialized.select();
        draftStatus.textContent = 'Download unavailable. Copy the selected feedback draft instead.';
      }
    });
    updateDraft();
    composer.hidden = false;
  } else {
    draftNotice.hidden = false;
  }
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
  if (rangeTimingKnown) rangeField.hidden = false;
}

// Compare actual packaged review videos; all creator-supplied names remain
// literal text. Players are independent and do not imply equivalent media.
const compareSections = [...document.querySelectorAll('[data-review-job]')];
if (compareSections.length >= 2) {
  const panel = document.createElement('details');
  panel.dataset.reviewCompare = '';
  const summary = document.createElement('summary');
  summary.textContent = 'Compare two versions';
  panel.append(summary);
  const guidance = document.createElement('p');
  guidance.textContent = 'Choose two versions and use each player to review it. The players are independent; compare the picture, sound and timing yourself.';
  panel.append(guidance);
  const grid = document.createElement('div');
  grid.dataset.compareGrid = '';
  panel.append(grid);
  const players = [];
  for (const [side, initial] of [['a', 0], ['b', 1]]) {
    const sideLabel = `Version ${side.toUpperCase()}`;
    const container = document.createElement('div');
    container.dataset.compareSide = side;
    const label = document.createElement('label');
    label.htmlFor = `review-compare-${side}`;
    label.textContent = sideLabel;
    container.append(label);
    const select = document.createElement('select');
    select.id = label.htmlFor;
    select.dataset.compareSelect = side;
    for (const [index, section] of compareSections.entries()) {
      const option = document.createElement('option');
      option.value = String(index);
      option.textContent = section.querySelector('h2').textContent;
      select.append(option);
    }
    select.value = String(initial);
    container.append(select);
    const identity = document.createElement('p');
    identity.dataset.compareIdentity = side;
    container.append(identity);
    grid.append(container);
    const player = { side, select, identity, container, video: null };
    const changeVersion = () => {
      const selected = compareSections[Number(select.value)];
      if (!selected) return;
      const source = selected.querySelector('video').getAttribute('src');
      const role = source.endsWith('/review-sdr.mp4') ? 'SDR review preview' : 'Master';
      identity.textContent = `Render job ID: ${selected.dataset.reviewJob} · Revision: ${selected.dataset.reviewRevision} · ${role}`;
      if (player.video) {
        player.video.pause();
        player.video.setAttribute('aria-label', `${sideLabel} player: ${selected.querySelector('h2').textContent}`);
        player.video.setAttribute('src', source);
        player.video.load();
      }
    };
    select.addEventListener('change', changeVersion);
    player.changeVersion = changeVersion;
    players.push(player);
    changeVersion();
  }
  panel.addEventListener('toggle', () => {
    if (panel.open) {
      for (const player of players) {
        if (!player.video) {
          player.video = document.createElement('video');
          player.video.controls = true;
          player.video.preload = 'metadata';
          player.video.dataset.compareVideo = player.side;
          player.container.append(player.video);
          player.changeVersion();
        }
      }
    } else {
      for (const player of players) if (player.video) player.video.pause();
    }
  });
  compareSections[0].before(panel);
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
            if item.duration.value <= 0
                || item.duration.rate.numerator == 0
                || item.duration.rate.denominator == 0
            {
                return Err(Error::Invalid(format!(
                    "caption {} has an invalid sidecar duration",
                    item.id
                )));
            }
            let start = sidecar_milliseconds(item.start, &item.id)?;
            let end_time = item.end_exclusive().map_err(|error| {
                Error::Invalid(format!(
                    "caption {} has an invalid sidecar interval: {error}",
                    item.id
                ))
            })?;
            let end = sidecar_milliseconds(end_time, &item.id)?;
            if end <= start {
                return Err(Error::Invalid(format!(
                    "caption {} has no duration after sidecar millisecond rounding; review and adjust its timing",
                    item.id
                )));
            }
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

/// Bound feedback to the verified master and, when present, the review video.
/// Old or inconsistent probe records do not establish a safe range bound.
pub(crate) fn review_duration(sequence: &Sequence, manifest: &Value) -> Result<RationalTime> {
    let master =
        crate::chapters::duration_from_verified_stream(sequence, &manifest["verification"])?;
    match &manifest["delivery"]["reviewVideo"] {
        Value::Null => Ok(master),
        Value::String(name) if name == "video.mp4" => Ok(master),
        Value::String(name) if name == "review-sdr.mp4" => {
            let files = manifest["delivery"]["files"]
                .as_array()
                .ok_or_else(|| Error::Invalid("review video timing evidence is missing".into()))?;
            let mut matches = files.iter().filter(|file| file["path"] == "review-sdr.mp4");
            let preview = matches
                .next()
                .ok_or_else(|| Error::Invalid("review video timing evidence is missing".into()))?;
            if matches.next().is_some() {
                return Err(Error::Invalid(
                    "review video timing evidence is ambiguous".into(),
                ));
            }
            let preview =
                crate::chapters::duration_from_verified_stream(sequence, &preview["verification"])?;
            if preview.checked_cmp(master)?.is_lt() {
                Ok(preview)
            } else {
                Ok(master)
            }
        }
        _ => Err(Error::Invalid(
            "review video timing evidence is unavailable".into(),
        )),
    }
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
         input,select{box-sizing:border-box;max-width:100%;font:inherit}\
         [data-compare-grid]{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1rem}\
         [data-compare-grid] select,[data-compare-grid] video{display:block;width:100%;max-width:100%}\
         @media(max-width:640px){[data-compare-grid]{grid-template-columns:1fr}}\
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
        // The rendered stream can differ from content_duration when a sequence
        // declares an explicit duration. Never round a fractional millisecond
        // past the verified end; leave range controls unavailable if evidence
        // is missing, while keeping the review page and point references.
        let review_limit_ms = review_duration(sequence, &manifest)
            .and_then(|duration| {
                Ok(duration
                    .rescale_to(RationalRate::frames(1000)?, RoundingMode::Floor)?
                    .value)
            })
            .ok()
            .filter(|limit| *limit > 0);
        let range_notice = if review_limit_ms.is_none() {
            "<p data-range-unavailable>Range feedback is unavailable because the output timing could not be verified. Re-render this output to create a range reference.</p>"
        } else {
            ""
        };
        let no_script_range = if review_limit_ms.is_some() {
            " For a range, include its start and end times."
        } else {
            ""
        };
        let review_limit_ms = review_limit_ms
            .map(|limit| limit.to_string())
            .unwrap_or_default();
        let title = &sequence.name;
        let notes = editing_notes(&manifest);
        let project_id = html_text(&frozen.project_id.to_string());
        let artifact_sha = html_text(
            manifest["verification"]["sha256"]
                .as_str()
                .ok_or_else(|| Error::Invalid("delivery master hash missing".into()))?,
        );
        let draft_controls = format!(
            "<p data-feedback-draft-unavailable hidden>Feedback drafting is unavailable for this review. Copy a playback reference instead.</p><details data-feedback-draft hidden><summary>Review this version</summary><fieldset><legend>Review draft</legend><p>Optionally set a version decision or add timed feedback. A playback point is recorded as a one-millisecond interval.</p><p><label for='draft-actor-{id}'>Your name</label><br><input id='draft-actor-{id}' data-draft-actor value='Creator' maxlength=200></p><p><label for='draft-decision-{id}'>Version decision</label><br><select id='draft-decision-{id}' data-draft-decision-choice><option value=''>Choose a decision</option><option value='approved'>Approve this version</option><option value='changes-requested'>Request changes</option></select></p><button type=button data-draft-set-decision>Set version decision</button> <button type=button data-draft-clear-decision disabled>Clear decision</button><p data-draft-decision>Draft decision: None</p><p>Pause at a moment, or mark a range above, to add timed feedback.</p><p><label for='draft-text-{id}'>Feedback</label><textarea id='draft-text-{id}' data-draft-text rows=3 maxlength=4000></textarea></p><button type=button data-draft-add-point>Add feedback at playback time</button> <button type=button data-draft-add-range>Add feedback for marked range</button><p>This draft stays on this page until you copy or download it; nothing is sent automatically. To change an already imported decision, start a new draft.</p><ol data-draft-list aria-label='Feedback in this draft'></ol><p><label for='draft-json-{id}'>Feedback draft for your agent</label></p><textarea id='draft-json-{id}' data-draft-json rows=8 readonly></textarea><button type=button data-draft-copy disabled>Copy feedback draft</button> <button type=button data-draft-download disabled>Download feedback draft</button> <button type=button data-draft-new>Start new draft</button><p role=status aria-live=polite></p></fieldset></details>"
        );
        html.push_str(&format!("<section data-review-job='{id}' data-review-revision='{revision}' data-review-limit-ms='{review_limit_ms}' data-review-project='{project_id}' data-review-sha256='{artifact_sha}'><h2 id='title-{id}'>{title}</h2><video controls width=360 aria-labelledby='title-{id}' src='{id}/{preview}'></video>{label}{notes}<fieldset data-feedback-reference hidden><legend>Share feedback at this moment</legend><p>Pause at the moment you want to discuss, then copy this reference for your agent.</p><p>Playback time: <span data-feedback-time></span></p><label for='feedback-{id}'>Feedback reference</label><textarea id='feedback-{id}' rows=6 readonly></textarea><button type=button>Copy feedback reference</button><p role=status aria-live=polite></p></fieldset>{range_notice}<details data-feedback-range hidden><summary>Share feedback for a range</summary><fieldset><legend>Mark a range</legend><p>Pause at the beginning and set the start, then seek to the end and set it.</p><p>Start: <span data-range-start>Not set</span><br>End: <span data-range-end>Not set</span></p><button type=button data-range-mark-start>Set range start</button> <button type=button data-range-mark-end>Set range end</button> <button type=button data-range-clear>Clear range</button><p><label for='range-{id}'>Range reference</label></p><textarea id='range-{id}' rows=6 readonly></textarea><button type=button data-range-copy disabled>Copy range reference</button><p role=status aria-live=polite></p></fieldset></details>{draft_controls}<noscript><p>For feedback, include render job ID {id}, rendered revision {revision}, and the playback time.{no_script_range}</p></noscript><p><a href='{id}/video.mp4'>Master</a> · <a href='{id}/captions.vtt'>Captions</a> · <a href='{id}/manifest.json'>Manifest</a></p></section>",title=html_text(title),revision=item.revision));
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
