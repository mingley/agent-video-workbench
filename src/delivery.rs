//! Subtitle and delivery files are derived from the same frozen timeline.
use crate::{
    Error, Result, media,
    process::{self, Control},
};
use agentcut_core::{ItemPayload, Sequence};
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
pub fn sidecars(sequence: &Sequence) -> Result<(String, String, Value)> {
    let mut cues = Vec::new();
    let solo = sequence.tracks.iter().any(|t| t.enabled && t.solo);
    for item in sequence
        .tracks
        .iter()
        .filter(|t| t.enabled && (!solo || t.solo))
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
         a{overflow-wrap:anywhere}\
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
        html.push_str(&format!("<section><video controls width=360 src='{id}/{preview}'></video>{label}<p><a href='{id}/video.mp4'>Master</a> · <a href='{id}/captions.vtt'>Captions</a> · <a href='{id}/manifest.json'>Manifest</a></p></section>"));
        items.push(json!({"jobId":id,"state":"succeeded","revision":item.revision,"sequenceId":manifest["sequenceId"],"files":files,"review":"pending"}));
    }
    html.push_str("</main></body></html>");
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
