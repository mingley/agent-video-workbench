//! Scoped ranged downloads. Credentials and signed URLs never enter history.
use crate::{
    Error, Result, media,
    process::{self, Uncontrolled},
};
use agentcut_render::FfmpegBackend;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::Command,
    time::Duration,
};
use url::Url;

#[derive(Default)]
pub struct Policy {
    pub hosts: Vec<String>,
    pub allow_loopback_http: bool,
}
fn error(message: &str) -> Error {
    Error::Invalid(format!("transfer: {message}"))
}
impl Policy {
    pub fn check(&self, url: &Url) -> Result<()> {
        let host = url.host_str().ok_or_else(|| error("URL host missing"))?;
        let dev = self.allow_loopback_http && matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        if !self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            return Err(error("host is outside configured download scope"));
        }
        if url.scheme() != "https" && !(dev && url.scheme() == "http") {
            return Err(error("HTTPS is required"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(error(
                "URL credentials are prohibited; use an authorized temporary media URL",
            ));
        }
        if !dev {
            if matches!(host, "localhost" | "metadata.google.internal")
                || host.ends_with(".local")
                || host.ends_with(".internal")
                || host.ends_with(".localhost")
            {
                return Err(error("private infrastructure is outside download scope"));
            }
            if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
                let private = match ip {
                    std::net::IpAddr::V4(v) => {
                        v.is_private()
                            || v.is_loopback()
                            || v.is_link_local()
                            || v.is_unspecified()
                            || v.is_broadcast()
                            || v.is_documentation()
                    }
                    std::net::IpAddr::V6(v) => {
                        v.is_loopback()
                            || v.is_unspecified()
                            || v.is_unique_local()
                            || v.is_unicast_link_local()
                    }
                };
                if private {
                    return Err(error("private IP is outside download scope"));
                }
            }
        }
        Ok(())
    }
}
fn headers(path: &Path) -> Result<(u16, BTreeMap<String, String>)> {
    let mut bytes = Vec::new();
    File::open(path)?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(error("response headers exceed 64 KiB"));
    }
    let text = String::from_utf8(bytes).map_err(|_| error("invalid response headers"))?;
    let mut status = 0;
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with("HTTP/") {
            status = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| error("invalid HTTP status"))?;
            values.clear();
        } else if let Some((name, value)) = line.split_once(':') {
            values.insert(name.to_ascii_lowercase(), value.trim().into());
        }
    }
    Ok((status, values))
}
fn curl(url: &Url, head: &Path) -> Command {
    let mut command = Command::new("curl");
    command
        .args([
            "--silent",
            "--show-error",
            "--connect-timeout",
            "15",
            "--max-time",
            "1800",
            "--proto",
            "=https,http",
            "--dump-header",
        ])
        .arg(head)
        .args(["--url", url.as_str()]);
    command
}
fn probe(url: &mut Url, policy: &Policy, head: &Path) -> Result<BTreeMap<String, String>> {
    for _ in 0..6 {
        policy.check(url)?;
        process::run(
            curl(url, head).arg("--head"),
            Duration::from_secs(60),
            &mut Uncontrolled,
        )
        .map_err(|_| {
            error("metadata request failed or expired; retry with a valid authorized URL")
        })?;
        let (status, values) = headers(head)?;
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = values
                .get("location")
                .ok_or_else(|| error("redirect has no location"))?;
            *url = url
                .join(location)
                .map_err(|_| error("invalid redirect URL"))?;
            continue;
        }
        match status {
            200 => return Ok(values),
            401 | 403 => return Err(error("authorization failed or temporary URL expired")),
            404 => return Err(error("remote object missing")),
            429 => return Err(error("remote quota/rate limit")),
            _ => {
                return Err(error(
                    "remote source must support HEAD with length metadata",
                ));
            }
        }
    }
    Err(error("too many redirects"))
}
pub struct Input<'a> {
    pub url: &'a str,
    pub id: &'a str,
    pub expected: u64,
    pub key: &'a str,
    pub sha256: Option<&'a str>,
    pub max_bytes: u64,
}
pub fn import(
    root: &Path,
    input: Input<'_>,
    policy: &Policy,
    backend: &FfmpegBackend,
) -> Result<Value> {
    if input.max_bytes == 0 || input.max_bytes > 32 * 1024 * 1024 * 1024 {
        return Err(error("maxBytes must be in 1..32 GiB"));
    }
    let mut url = Url::parse(input.url).map_err(|_| error("invalid URL"))?;
    policy.check(&url)?;
    if input
        .sha256
        .is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(error("supplied SHA-256 is invalid"));
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("transfer.lock"))?;
    lock.try_lock()
        .map_err(|_| error("another transfer owns this project"))?;
    let identity = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"key":input.key,"id":input.id,"revision":input.expected})
        )?)
    );
    let part = root.join("cache").join(format!("download-{identity}.part"));
    let meta = part.with_extension("json");
    let head = part.with_extension("headers");
    let probe_head = root
        .join("cache")
        .join(format!("probe-headers-{}", uuid::Uuid::new_v4()));
    let segment = part.with_extension("segment");
    let result = (|| {
        let abandoned = if head.exists() {
            headers(&head).ok()
        } else {
            None
        };
        let fields = probe(&mut url, policy, &probe_head)?;
        if fields
            .get("content-type")
            .is_some_and(|t| t.starts_with("text/html"))
        {
            return Err(error("sharing page is not media bytes"));
        }
        let length = fields
            .get("content-length")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| error("remote length is required for space budgeting"))?;
        if length == 0
            || length > input.max_bytes
            || length > crate::jobs::free_space(root)?.saturating_sub(128 * 1024 * 1024) / 2
        {
            return Err(error("remote object exceeds configured byte/space budget"));
        }
        let etag = fields
            .get("etag")
            .filter(|s| s.starts_with('"') && s.ends_with('"') && !s.contains(['\r', '\n']))
            .cloned();
        let url_hash = format!("{:x}", Sha256::digest(url.as_str().as_bytes()));
        let current =
            json!({"urlHash":url_hash,"etag":etag,"length":length,"suppliedSha256":input.sha256});
        let prior = if meta.exists() {
            Some(crate::json::read::<Value>(&meta)?)
        } else {
            None
        };
        let can_resume = etag.is_some() && prior.as_ref() == Some(&current);
        let mut offset = if can_resume && part.is_file() {
            std::fs::metadata(&part)?.len()
        } else {
            0
        };
        if offset > length {
            offset = 0;
        }
        if can_resume
            && segment.is_file()
            && let Some((status, response)) = abandoned
            && response.get("etag") == etag.as_ref()
        {
            if offset > 0
                && status == 206
                && response.get("content-range").is_some_and(|v| {
                    v.starts_with(&format!("bytes {offset}-")) && v.ends_with(&format!("/{length}"))
                })
            {
                let mut target = OpenOptions::new().append(true).open(&part)?;
                std::io::copy(&mut File::open(&segment)?, &mut target)?;
                target.sync_all()?;
                offset = std::fs::metadata(&part)?.len();
            } else if offset == 0 && status == 200 {
                std::fs::rename(&segment, &part)?;
                File::open(&part)?.sync_all()?;
                offset = std::fs::metadata(&part)?.len();
            }
        }
        if segment.exists() {
            std::fs::remove_file(&segment)?;
        }
        if offset > length {
            offset = 0;
        }
        if offset == 0 && part.exists() {
            std::fs::remove_file(&part)?;
        }
        let mut metadata = File::create(&meta)?;
        metadata.write_all(&serde_json::to_vec(&current)?)?;
        metadata.sync_all()?;
        if offset < length {
            let mut command = curl(&url, &head);
            command
                .args(["--max-filesize", &input.max_bytes.to_string(), "--output"])
                .arg(&segment);
            if offset > 0 {
                command.args([
                    "--range",
                    &format!("{offset}-"),
                    "--header",
                    &format!(
                        "If-Range: {}",
                        etag.as_ref()
                            .ok_or_else(|| error("resume validator missing"))?
                    ),
                ]);
            }
            let transport =
                process::run(&mut command, Duration::from_secs(1810), &mut Uncontrolled);
            let (status, response) = headers(&head)?;
            if !matches!(status, 200 | 206) {
                return Err(error("download authorization, quota or server failure"));
            }
            if etag.is_some() && response.get("etag") != etag.as_ref() {
                std::fs::remove_file(&meta)?;
                return Err(error(
                    "remote object changed during transfer; retry starts a new attempt",
                ));
            }
            if offset > 0 && status == 206 {
                let expected = format!("bytes {offset}-");
                if !response
                    .get("content-range")
                    .is_some_and(|v| v.starts_with(&expected) && v.ends_with(&format!("/{length}")))
                {
                    return Err(error("invalid ranged response"));
                }
                let mut target = OpenOptions::new().append(true).open(&part)?;
                std::io::copy(&mut File::open(&segment)?, &mut target)?;
                target.sync_all()?;
            } else {
                offset = 0;
                std::fs::rename(&segment, &part)?;
                File::open(&part)?.sync_all()?;
            }
            if transport.is_err() {
                return Err(error(
                    "download interrupted; validated partial bytes retained for retry",
                ));
            }
        }
        if std::fs::metadata(&part)?.len() != length {
            return Err(error(
                "incomplete transfer; retry resumes with the same validator",
            ));
        }
        let hash = media::hash_file(&part)?;
        if input
            .sha256
            .is_some_and(|expected| !hash.eq_ignore_ascii_case(expected))
        {
            std::fs::remove_file(&part)?;
            std::fs::remove_file(&meta)?;
            return Err(error("supplied checksum mismatch"));
        }
        let outcome = media::import(root, &part, input.id, input.expected, input.key, backend)?;
        std::fs::remove_file(&part)?;
        std::fs::remove_file(&meta)?;
        Ok(
            json!({"outcome":outcome,"transfer":{"bytes":length,"sha256":hash,"resumedFromBytes":offset,"validator":"strong ETag when available","originality":"unknown; file format cannot prove camera originality"}}),
        )
    })();
    for path in [head, probe_head, segment] {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    result
}
