//! MCP stdio adapter. Tool execution delegates to Service; no separate mutation path.
use crate::{
    Error, Result,
    service::{Request, Service, envelope},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
const PROTOCOL: &str = "2025-11-25";
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_AUDIO_MS: u32 = 15_000;

/// Transport convenience for media which the existing service has inspected
/// or verified. The caller cannot supply an arbitrary file path.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediaRequest {
    Frame {
        project: PathBuf,
        asset_id: String,
        at_ms: i64,
    },
    ContactSheet {
        project: PathBuf,
        job_id: String,
    },
    Storyboard {
        project: PathBuf,
        job_id: String,
    },
    AudioProxy {
        project: PathBuf,
        job_id: String,
        start_ms: u32,
        duration_ms: u32,
    },
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

// MCP uses RFC 4648 base64 for image and audio content. Keeping the encoder
// local avoids adding another runtime dependency solely for this transport.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(char::from(TABLE[((word >> 18) & 63) as usize]));
        out.push(char::from(TABLE[((word >> 12) & 63) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[((word >> 6) & 63) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[(word & 63) as usize])
        } else {
            '='
        });
    }
    out
}

fn scoped_file(service: &Service, path: &Path) -> Result<PathBuf> {
    let root = service
        .root
        .as_ref()
        .ok_or_else(|| invalid("MCP media requires an authorized workspace"))?
        .canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(invalid(
            "verified media is outside the authorized workspace",
        ));
    }
    Ok(path)
}

fn image_content(service: &Service, report: &Value) -> Result<(Value, Value)> {
    if report["mimeType"] != "image/png" {
        return Err(invalid("requested evidence is not a PNG image"));
    }
    let expected = report["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("image hash is unavailable"))?;
    let path = scoped_file(
        service,
        Path::new(
            report["path"]
                .as_str()
                .ok_or_else(|| invalid("image path is unavailable"))?,
        ),
    )?;
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("image is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(invalid("image exceeds the 2 MiB MCP media limit"));
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid("verified image is not a PNG"));
    }
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(invalid("image bytes changed after verification"));
    }
    let content = json!({"type":"image","mimeType":"image/png","data":base64(&bytes)});
    let metadata = json!({"mimeType":"image/png","bytes":bytes.len(),"sha256":expected});
    Ok((metadata, content))
}

fn proxy_artifact(service: &Service, project: &Path, id: &str) -> Result<Value> {
    service.execute(Request::Artifact {
        project: project.to_path_buf(),
        id: id.into(),
        sheet: false,
        preview: false,
    })
}

fn proxy_attachment(artifact: &Value) -> Result<&Value> {
    if artifact["verified"] != true || artifact["mimeType"] != "application/json" {
        return Err(invalid(
            "audio requires a verified source-proxy analysis job",
        ));
    }
    let files = artifact["attachments"]
        .as_array()
        .ok_or_else(|| invalid("proxy attachments are unavailable"))?;
    let mut candidates = files.iter().filter(|f| {
        f["path"]
            .as_str()
            .and_then(|p| Path::new(p).file_name())
            .is_some_and(|name| name == "proxy.mp4")
    });
    let file = candidates
        .next()
        .ok_or_else(|| invalid("verified proxy.mp4 attachment is unavailable"))?;
    if candidates.next().is_some() {
        return Err(invalid("proxy attachments are ambiguous"));
    }
    Ok(file)
}

// Return a conventional, seekable 16-bit mono WAV header for bounded PCM.
fn pcm_wav(pcm: &[u8]) -> Result<Vec<u8>> {
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
        return Err(invalid("source proxy produced no complete audio samples"));
    }
    let size = u32::try_from(pcm.len()).map_err(|_| invalid("audio excerpt is too large"))?;
    let riff = size
        .checked_add(36)
        .ok_or_else(|| invalid("audio excerpt is too large"))?;
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&32_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&size.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

pub fn media_request(
    service: &Service,
    request: MediaRequest,
    protocol: &str,
) -> Result<(Value, Value)> {
    match request {
        MediaRequest::Frame {
            project,
            asset_id,
            at_ms,
        } => {
            let report = service.execute(Request::Inspect {
                project,
                asset_id: asset_id.clone(),
                kind: crate::inspect::Kind::Frame,
                start_ms: at_ms,
                end_ms: None,
            })?;
            let (mut metadata, content) = image_content(service, &report)?;
            metadata["kind"] = json!("frame");
            metadata["assetId"] = json!(asset_id);
            metadata["sourceSha256"] = report["sourceHash"].clone();
            metadata["requestedSourceTimeMs"] = json!(at_ms);
            metadata["guidance"] = json!(
                "The requested source seek is not an independently measured decoded-frame timestamp. Image is an SDR inspection preview."
            );
            Ok((metadata, content))
        }
        MediaRequest::ContactSheet { project, job_id } => {
            let report = service.execute(Request::Artifact {
                project,
                id: job_id.clone(),
                sheet: true,
                preview: false,
            })?;
            if report["verified"] != true || report["role"] != "contact-sheet" {
                return Err(invalid("job has no verified render contact sheet"));
            }
            let (mut metadata, content) = image_content(service, &report)?;
            metadata["kind"] = json!("contact-sheet");
            metadata["jobId"] = json!(job_id);
            metadata["revision"] = report["revision"].clone();
            metadata["guidance"] = json!(
                "Contact sheet is an SDR review aid; evaluate the master separately for color and appearance."
            );
            Ok((metadata, content))
        }
        MediaRequest::Storyboard { project, job_id } => {
            let artifact = proxy_artifact(service, &project, &job_id)?;
            if artifact["verified"] != true || artifact["mimeType"] != "application/json" {
                return Err(invalid("storyboard requires a verified analysis job"));
            }
            let path = scoped_file(
                service,
                Path::new(
                    artifact["path"]
                        .as_str()
                        .ok_or_else(|| invalid("storyboard report unavailable"))?,
                ),
            )?;
            let report: Value = crate::json::read(&path)?;
            if crate::media::hash_file(&path)? != artifact["sha256"]
                || report["task"]["kind"] != "storyboard"
            {
                return Err(invalid("analysis job is not an intact storyboard"));
            }
            let file = artifact["attachments"]
                .as_array()
                .and_then(|a| {
                    a.iter().find(|f| {
                        Path::new(f["path"].as_str().unwrap_or(""))
                            .file_name()
                            .is_some_and(|n| n == "storyboard.png")
                    })
                })
                .ok_or_else(|| invalid("storyboard image attachment unavailable"))?;
            let mut image_report = file.clone();
            image_report["mimeType"] = json!("image/png");
            let (mut metadata, content) = image_content(service, &image_report)?;
            metadata["revision"] = artifact["revision"].clone();
            metadata["guidance"] = json!(
                "Frozen sparse SDR source navigation image; pair tile timestamps with the image and inspect original media for details or HDR appearance."
            );
            metadata["kind"] = json!("storyboard");
            metadata["jobId"] = json!(job_id);
            metadata["sourceSha256"] = report["sourceSha256"].clone();
            metadata["storyboard"] = report["data"].clone();
            Ok((metadata, content))
        }
        MediaRequest::AudioProxy {
            project,
            job_id,
            start_ms,
            duration_ms,
        } => {
            if protocol == "2024-11-05" {
                return Err(invalid(
                    "audio content requires MCP 2025-03-26 or later; use the verified proxy artifact path on older clients",
                ));
            }
            if duration_ms == 0 || duration_ms > MAX_AUDIO_MS {
                return Err(invalid("audio duration must be 1..15000 ms"));
            }
            let end_ms = start_ms
                .checked_add(duration_ms)
                .ok_or_else(|| invalid("audio range overflow"))?;
            let artifact = proxy_artifact(service, &project, &job_id)?;
            let attachment = proxy_attachment(&artifact)?;
            let proxy = scoped_file(
                service,
                Path::new(
                    attachment["path"]
                        .as_str()
                        .ok_or_else(|| invalid("proxy path missing"))?,
                ),
            )?;
            let proxy_sha = attachment["sha256"]
                .as_str()
                .ok_or_else(|| invalid("proxy hash missing"))?;
            let report_path = scoped_file(
                service,
                Path::new(
                    artifact["path"]
                        .as_str()
                        .ok_or_else(|| invalid("analysis report missing"))?,
                ),
            )?;
            let report: Value = crate::json::read(&report_path)?;
            if crate::media::hash_file(&report_path)? != artifact["sha256"] {
                return Err(invalid("analysis report changed after verification"));
            }
            let task: crate::analysis::Task = serde_json::from_value(report["task"].clone())?;
            let (asset_id, source_start, source_end) = match task {
                crate::analysis::Task::Proxy {
                    asset_id,
                    start_ms,
                    end_ms,
                    ..
                } => (asset_id, start_ms, end_ms),
                crate::analysis::Task::BudgetPreview(q) => (q.asset_id, q.start_ms, q.end_ms),
                _ => return Err(invalid("audio requires a source-proxy analysis job")),
            };
            if source_start < 0
                || source_end <= source_start
                || i64::from(end_ms) > source_end - source_start
                || report["data"]["file"] != "proxy.mp4"
            {
                return Err(invalid(
                    "audio interval is outside the verified source proxy",
                ));
            }
            let pcm = crate::process::run(
                Command::new(service.backend.ffmpeg_path())
                    .args([
                        "-nostdin",
                        "-v",
                        "error",
                        "-protocol_whitelist",
                        "file",
                        "-ss",
                    ])
                    .arg(format!("{}.{:03}", start_ms / 1000, start_ms % 1000))
                    .arg("-i")
                    .arg(&proxy)
                    .args(["-t"])
                    .arg(format!("{}.{:03}", duration_ms / 1000, duration_ms % 1000))
                    .args([
                        "-map",
                        "0:a:0",
                        "-vn",
                        "-ac",
                        "1",
                        "-ar",
                        "16000",
                        "-c:a",
                        "pcm_s16le",
                        "-f",
                        "s16le",
                        "pipe:1",
                    ]),
                Duration::from_secs(60),
                &mut crate::process::Uncontrolled,
            )?
            .stdout;
            let expected_pcm_bytes = usize::try_from(duration_ms)
                .map_err(|_| invalid("audio duration is too large"))?
                .checked_mul(32)
                .ok_or_else(|| invalid("audio duration is too large"))?;
            if pcm.len() > expected_pcm_bytes {
                return Err(invalid("audio excerpt exceeds the requested duration"));
            }
            let checked = proxy_artifact(service, &project, &job_id)?;
            let checked_attachment = proxy_attachment(&checked)?;
            if checked["sha256"] != artifact["sha256"]
                || checked_attachment["sha256"] != proxy_sha
                || scoped_file(
                    service,
                    Path::new(
                        checked_attachment["path"]
                            .as_str()
                            .ok_or_else(|| invalid("proxy path missing"))?,
                    ),
                )? != proxy
            {
                return Err(invalid("source proxy changed during audio extraction"));
            }
            let wav = pcm_wav(&pcm)?;
            let sha = format!("{:x}", Sha256::digest(&wav));
            let metadata = json!({"kind":"audio-proxy","jobId":job_id,"assetId":asset_id,
                "sourceSha256":report["sourceSha256"],"proxySha256":proxy_sha,"sha256":sha,
                "mimeType":"audio/wav","bytes":wav.len(),"sampleRateHz":16000,"channels":1,
                "sampleCount":pcm.len()/2,"returnedFullRequestedDuration":pcm.len()==expected_pcm_bytes,
                "requestedProxyStartMs":start_ms,"requestedDurationMs":duration_ms,
                "requestedSourceStartMs":source_start + i64::from(start_ms),"requestedSourceEndMs":source_start + i64::from(end_ms),
                "guidance":"This is a mono listening preview from the verified proxy. Source times follow its requested mapping and are not independent audio or word alignment. Host audio support is required."});
            let content = json!({"type":"audio","mimeType":"audio/wav","data":base64(&wav)});
            Ok((metadata, content))
        }
    }
}
fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub fn serve(service: &Service, reader: &mut impl BufRead, writer: &mut impl Write) -> Result<()> {
    let mut initialized = false;
    let mut negotiated_protocol = PROTOCOL.to_owned();
    loop {
        let mut line = Vec::new();
        let count = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count > 8 * 1024 * 1024 {
            return Err(Error::Invalid("MCP message exceeds 8 MiB".into()));
        }
        let message: Value = match crate::json::parse(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(writer, "{}", error(&Value::Null, -32700, &e.to_string()))?;
                writer.flush()?;
                continue;
            }
        };
        let id = message.get("id").cloned();
        let null = Value::Null;
        let response = if message["jsonrpc"] != "2.0" || !message["method"].is_string() {
            Some(error(
                id.as_ref().unwrap_or(&null),
                -32600,
                "Invalid JSON-RPC request",
            ))
        } else {
            match message["method"].as_str().unwrap_or("") {
                "initialize" if id.is_some() => {
                    initialized = true;
                    let requested = message["params"]["protocolVersion"]
                        .as_str()
                        .unwrap_or(PROTOCOL);
                    let negotiated = if matches!(
                        requested,
                        "2025-11-25" | "2025-06-18" | "2025-03-26" | "2024-11-05"
                    ) {
                        requested
                    } else {
                        PROTOCOL
                    };
                    negotiated_protocol = negotiated.to_owned();
                    Some(reply(
                        id.as_ref().unwrap_or(&null),
                        json!({"protocolVersion":negotiated,"capabilities":{"tools":{"listChanged":false},"resources":{"listChanged":false}},"serverInfo":{"name":"agent-video-workbench","version":env!("CARGO_PKG_VERSION")},"instructions":"Use avw command=agent-guide first. Long renders return durable job handles; use render-start and job-status. All paths are restricted to the configured workspace."}),
                    ))
                }
                "notifications/initialized" | "notifications/cancelled" => None,
                _ if id.is_none() => None,
                _ if !initialized => Some(error(
                    id.as_ref().unwrap_or(&null),
                    -32002,
                    "Initialize before requesting tools",
                )),
                "ping" => Some(reply(id.as_ref().unwrap_or(&null), json!({}))),
                "tools/list" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"tools":[{"name":"avw","description":"Persistent video editing, source inspection, transcripts, revision history, background render jobs and verified artifacts. Call agent-guide for the complete workflow.","inputSchema":tool_schema()},
                    {"name":"avw-media","description":"Opt in to receive a verified source frame, source storyboard or contact sheet as PNG, or a short audio excerpt from a verified source-proxy job as WAV. Host image/audio support is not guaranteed. Requests use project/asset/job IDs, never arbitrary paths.","inputSchema":media_tool_schema()}]}),
                )),
                "tools/call" => {
                    let arguments = serde_json::to_vec(&message["params"]["arguments"])?;
                    let result: Result<(Value, Option<Value>)> =
                        match message["params"]["name"].as_str() {
                            Some("avw") => crate::json::parse::<Request>(&arguments)
                                .and_then(|r| service.execute(r))
                                .map(|result| (result, None)),
                            Some("avw-media") => crate::json::parse::<MediaRequest>(&arguments)
                                .and_then(|r| media_request(service, r, &negotiated_protocol))
                                .map(|(result, content)| (result, Some(content))),
                            _ => Err(Error::Invalid("unknown tool".into())),
                        };
                    let (result, extra) = match result {
                        Ok((value, content)) => (Ok(value), content),
                        Err(error) => (Err(error), None),
                    };
                    let envelope = envelope(result);
                    let failed = envelope["ok"] == false;
                    let mut content =
                        vec![json!({"type":"text","text":serde_json::to_string(&envelope)?})];
                    if let Some(extra) = extra {
                        content.push(extra);
                    }
                    Some(reply(
                        id.as_ref().unwrap_or(&null),
                        json!({"content":content,"structuredContent":envelope,"isError":failed}),
                    ))
                }
                "resources/list" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"resources":[{"uri":"avw://guide","name":"Agent workflow","mimeType":"text/markdown"}]}),
                )),
                "resources/read" if message["params"]["uri"] == "avw://guide" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"contents":[{"uri":"avw://guide","mimeType":"text/markdown","text":include_str!("../AGENT_GUIDE.md")}]}),
                )),
                _ => Some(error(
                    id.as_ref().unwrap_or(&null),
                    -32601,
                    "Method not found",
                )),
            }
        };
        if let Some(response) = response {
            writeln!(writer, "{response}")?;
            writer.flush()?;
        }
    }
}

fn tool_schema() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_else(|_| json!({}));
    schema["type"] = json!("object");
    schema
}

fn media_tool_schema() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(MediaRequest)).unwrap_or_else(|_| json!({}));
    schema["type"] = json!("object");
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_base64_encodes_standard_padding_and_binary_vectors() {
        for (input, expected) in [
            (&b""[..], ""),
            (&b"f"[..], "Zg=="),
            (&b"fo"[..], "Zm8="),
            (&b"foo"[..], "Zm9v"),
            (&b"foob"[..], "Zm9vYg=="),
            (&b"foobar"[..], "Zm9vYmFy"),
            (&[0, 255, 254, 253][..], "AP/+/Q=="),
        ] {
            assert_eq!(base64(input), expected);
        }
    }

    #[test]
    fn wav_header_describes_exact_returned_pcm_samples() {
        let wav = pcm_wav(&[1, 0, 2, 0]).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 4);
        assert_eq!(&wav[44..], &[1, 0, 2, 0]);
        assert!(pcm_wav(&[]).is_err());
        assert!(pcm_wav(&[1]).is_err());
    }
}
