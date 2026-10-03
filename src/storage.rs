//! Project discovery, verified repair and conservative derived-cache collection.
use crate::{Error, Result, media, store::Store};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::Path,
    time::{Duration, SystemTime},
};

fn objects(store: &Store) -> Result<BTreeMap<String, agentcut_core::Asset>> {
    let mut objects = BTreeMap::new();
    for row in store.history()? {
        for asset in store
            .revision(
                row["revision"]
                    .as_u64()
                    .ok_or_else(|| Error::Invalid("invalid revision".into()))?,
            )?
            .assets
        {
            let hash = asset
                .fingerprint
                .sha256
                .clone()
                .ok_or_else(|| Error::Invalid("asset missing hash".into()))?;
            objects.entry(hash).or_insert(asset);
        }
    }
    Ok(objects)
}
pub fn verify(root: &Path) -> Result<Value> {
    let store = Store::open(root)?;
    let mut files = Vec::new();
    for (hash, asset) in objects(&store)? {
        let path = root.join(&asset.uri);
        let status = if !path.exists() {
            "missing"
        } else if media::verify_asset(root, &asset).is_ok() {
            "verified"
        } else {
            "corrupt"
        };
        files.push(json!({"sha256":hash,"assetId":asset.id,"path":asset.uri,"bytes":asset.fingerprint.size_bytes,"state":status}));
    }
    Ok(
        json!({"projectId":store.project()?.project_id,"revision":store.project()?.revision,"complete":files.iter().all(|f|f["state"]=="verified"),"objects":files}),
    )
}
pub fn relink(root: &Path, source: &Path, hash: &str) -> Result<Value> {
    let store = Store::open(root)?;
    let all = objects(&store)?;
    let asset = all.get(hash).ok_or_else(|| {
        Error::Invalid("hash is not referenced by retained project history".into())
    })?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("import.lock"))?;
    lock.try_lock()
        .map_err(|_| Error::Invalid("project import is active".into()))?;
    if !std::fs::metadata(source)?.is_file() || media::hash_file(source)? != hash {
        return Err(Error::Invalid(
            "relink source does not match the required object".into(),
        ));
    }
    let staged = root
        .join("cache")
        .join(format!("relink-{}", uuid::Uuid::new_v4()));
    std::fs::copy(source, &staged)?;
    File::open(&staged)?.sync_all()?;
    if media::hash_file(&staged)? != hash {
        std::fs::remove_file(&staged)?;
        return Err(Error::Invalid(
            "relink source changed during copying".into(),
        ));
    }
    let destination = root.join(&asset.uri);
    if destination.exists() {
        if media::verify_asset(root, asset).is_ok() {
            std::fs::remove_file(staged)?;
            return Ok(json!({"sha256":hash,"state":"already-verified"}));
        }
        // Keep corrupt bytes available for diagnosis rather than overwriting them.
        std::fs::rename(
            &destination,
            root.join("cache")
                .join(format!("quarantine-{}", uuid::Uuid::new_v4())),
        )?;
    }
    std::fs::hard_link(&staged, &destination)?;
    std::fs::remove_file(staged)?;
    File::open(root.join("originals"))?.sync_all()?;
    media::verify_asset(root, asset)?;
    Ok(json!({"sha256":hash,"state":"relinked","revisionUnchanged":true}))
}
fn old(path: &Path, grace: Duration) -> Result<bool> {
    Ok(SystemTime::now()
        .duration_since(std::fs::symlink_metadata(path)?.modified()?)
        .unwrap_or_default()
        >= grace)
}
fn size(path: &Path) -> Result<u64> {
    let info = std::fs::symlink_metadata(path)?;
    if info.file_type().is_symlink() {
        return Err(Error::Invalid("cache collection refuses symlinks".into()));
    }
    if info.is_file() {
        return Ok(info.len());
    }
    let mut bytes = 0;
    for entry in std::fs::read_dir(path)? {
        bytes += size(&entry?.path())?;
    }
    Ok(bytes)
}
pub fn gc(root: &Path, grace_seconds: u64, dry_run: bool) -> Result<Value> {
    if grace_seconds < 60 {
        return Err(Error::Invalid(
            "cache grace must be at least 60 seconds".into(),
        ));
    }
    let worker = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("worker.lock"))?;
    worker
        .try_lock()
        .map_err(|_| Error::Invalid("active worker leases prevent collection".into()))?;
    let ingest = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("import.lock"))?;
    ingest
        .try_lock()
        .map_err(|_| Error::Invalid("active import prevents collection".into()))?;
    let transfer_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("transfer.lock"))?;
    transfer_lock
        .try_lock()
        .map_err(|_| Error::Invalid("active transfer prevents collection".into()))?;
    let analysis_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("analysis.lock"))?;
    analysis_lock
        .try_lock()
        .map_err(|_| Error::Invalid("source inspection leases prevent collection".into()))?;
    let store = Store::open(root)?;
    let jobs = store.jobs()?;
    let grace = Duration::from_secs(grace_seconds);
    let mut candidates = Vec::new();
    for name in ["cache", "analysis", "renders"] {
        let directory = root.join(name);
        if !std::fs::symlink_metadata(&directory)?.is_dir() {
            return Err(Error::Invalid(
                "managed cache directory is not a real directory".into(),
            ));
        }
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            let filename = path
                .file_name()
                .ok_or_else(|| Error::Invalid("cache filename missing".into()))?
                .to_string_lossy();
            if jobs.iter().filter(|j| j["state"] == "succeeded").any(|j| {
                ["path", "manifest"].iter().any(|key| {
                    j["result"][key]
                        .as_str()
                        .is_some_and(|p| Path::new(p).starts_with(&path))
                })
            }) {
                continue;
            }
            if filename.starts_with("quarantine-") {
                continue;
            }
            // Retain every successful final and every attempt belonging to queued
            // or recoverable jobs. Only terminal failed/cancelled scratch is evicted.
            if name == "renders"
                && !jobs.iter().any(|j| {
                    matches!(
                        j["state"].as_str(),
                        Some("failed" | "cancelled" | "unavailable")
                    ) && j["id"]
                        .as_str()
                        .is_some_and(|id| filename == id || filename.starts_with(&format!("{id}-")))
                })
            {
                continue;
            }
            if !old(&path, grace)? {
                continue;
            }
            let bytes = size(&path)?;
            candidates.push(json!({"path":path.strip_prefix(root).map_err(|_|Error::Invalid("cache path escaped root".into()))?,"bytes":bytes,"reason":"reproducible analysis or terminal attempt scratch; no retained original is eligible"}));
            if !dry_run {
                if std::fs::symlink_metadata(&path)?.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
            }
        }
    }
    Ok(
        json!({"dryRun":dry_run,"candidates":candidates,"retained":"all originals and history, successful artifacts, quarantine bytes, queued/retryable render attempts","bytes":candidates.iter().filter_map(|v|v["bytes"].as_u64()).sum::<u64>()}),
    )
}
pub fn catalog(root: &Path, query: &str, offset: u32, limit: u32) -> Result<Value> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Invalid("catalog limit must be 1..100".into()));
    }
    let root = root.canonicalize()?;
    let mut pending = vec![(root.clone(), 0)];
    let mut visited = 0;
    let mut projects = Vec::new();
    while let Some((path, depth)) = pending.pop() {
        visited += 1;
        if visited > 10_000 {
            return Err(Error::Invalid(
                "catalog scan exceeds 10000 directories; select a narrower workspace".into(),
            ));
        }
        if path.join("project.sqlite").is_file() {
            let result = (|| {
                let conn = rusqlite::Connection::open_with_flags(
                    path.join("project.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                let text: String = conn.query_row(
                    "SELECT snapshot FROM revisions JOIN head ON revisions.id=head.revision",
                    [],
                    |r| r.get(0),
                )?;
                let p: agentcut_core::Project = serde_json::from_str(&text)?;
                Ok::<_, Error>(
                    json!({"projectId":p.project_id,"name":p.name,"revision":p.revision,"sequences":p.sequences.iter().map(|s|json!({"id":s.id,"name":s.name})).collect::<Vec<_>>(),"path":path,"state":"available"}),
                )
            })();
            let item = result.unwrap_or_else(
                |e| json!({"path":path,"state":"unavailable","error":e.to_string()}),
            );
            if item
                .to_string()
                .to_lowercase()
                .contains(&query.to_lowercase())
            {
                projects.push(item);
            }
            continue;
        }
        if depth < 3 {
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir()
                    && !entry.file_name().to_string_lossy().starts_with('.')
                {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
    projects.sort_by_key(|p| p["path"].as_str().unwrap_or("").to_owned());
    let total = projects.len();
    let items: Vec<_> = projects
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    Ok(
        json!({"items":items,"nextOffset":if offset as usize+items.len()<total {Some(offset+limit)}else{None},"authority":"each project's SQLite database; catalog is rebuilt on demand"}),
    )
}

/// A scheduler entry point; busy projects are reported and retried next run.
pub fn maintain(workspace: &Path, grace_seconds: u64, dry_run: bool) -> Result<Value> {
    if grace_seconds < 60 {
        return Err(Error::Invalid(
            "cache grace must be at least 60 seconds".into(),
        ));
    }
    let mut offset = 0;
    let mut results = Vec::new();
    loop {
        let page = catalog(workspace, "", offset, 100)?;
        for item in page["items"]
            .as_array()
            .ok_or_else(|| Error::Invalid("invalid catalog page".into()))?
        {
            let path = Path::new(
                item["path"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("catalog path missing".into()))?,
            );
            let result = if item["state"] == "available" {
                gc(path, grace_seconds, dry_run)
                    .map(|v| json!({"path":path,"state":"collected","result":v}))
                    .unwrap_or_else(
                        |e| json!({"path":path,"state":"deferred","error":e.to_string()}),
                    )
            } else {
                item.clone()
            };
            results.push(result);
        }
        let Some(next) = page["nextOffset"].as_u64() else {
            break;
        };
        offset =
            u32::try_from(next).map_err(|_| Error::Invalid("catalog offset overflow".into()))?;
    }
    Ok(
        json!({"dryRun":dry_run,"projects":results,"deferredProjects":results.iter().filter(|r|r["state"]!="collected").count()}),
    )
}
