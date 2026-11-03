//! Local first-run diagnostics. Host behavior is never inferred from local tests.
use crate::{
    Error, Result, jobs,
    service::{Request, Service},
};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

fn workspace_path(service: &Service, workspace: &Path) -> Result<PathBuf> {
    let raw = if workspace.is_absolute() {
        workspace.to_path_buf()
    } else if let Some(root) = &service.root {
        root.join(workspace)
    } else {
        workspace.to_path_buf()
    };
    let resolved = raw.canonicalize().map_err(|error| {
        Error::Invalid(format!("workspace must be an existing directory: {error}"))
    })?;
    if !resolved.is_dir() {
        return Err(Error::Invalid(
            "workspace must be an existing directory".into(),
        ));
    }
    if let Some(root) = &service.root
        && !resolved.starts_with(root.canonicalize()?)
    {
        return Err(Error::Invalid(
            "workspace is outside the authorized service root".into(),
        ));
    }
    Ok(resolved)
}

/// Create and remove only a unique file owned by this check. Always attempt
/// cleanup after a successful create, including if writing or syncing fails.
fn writable_probe(workspace: &Path) -> Value {
    let path = workspace.join(format!(".avw-readiness-{}", uuid::Uuid::new_v4()));
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) => return json!({"status":"fail","stage":"create","reason":error.to_string()}),
    };
    let written = file
        .write_all(b"AVW host readiness probe\n")
        .and_then(|()| file.sync_all());
    drop(file);
    let removed = fs::remove_file(&path);
    match (written, removed) {
        (Ok(()), Ok(())) => json!({"status":"pass","checks":["create","write","sync","remove"]}),
        (Err(write), Ok(())) => {
            json!({"status":"fail","stage":"write-or-sync","reason":write.to_string(),"cleanup":"removed"})
        }
        (Ok(()), Err(remove)) => {
            json!({"status":"fail","stage":"remove","reason":remove.to_string(),"cleanupPath":path})
        }
        (Err(write), Err(remove)) => {
            json!({"status":"fail","stage":"write-or-sync-and-remove","reason":write.to_string(),"cleanupReason":remove.to_string(),"cleanupPath":path})
        }
    }
}

/// Run the same media doctor and capability discovery exposed by the service,
/// alongside a scoped filesystem probe. No files are uploaded or published.
pub fn check(service: &Service, workspace: &Path) -> Result<Value> {
    let workspace = workspace_path(service, workspace)?;
    let writable = writable_probe(&workspace);
    let space = match jobs::free_space(&workspace) {
        Ok(bytes) => json!({"status":"measured","availableBytesDecimal":bytes.to_string(),
            "availableBytes":if bytes<=9_007_199_254_740_991 {Some(bytes)}else{None},
            "method":"filesystem-reported blocks available to unprivileged users; quotas and reservations may differ"}),
        Err(error) => json!({"status":"unknown","code":error.code(),"reason":error.to_string()}),
    };
    let media = match service.execute(Request::Doctor {}) {
        Ok(result) if result["ready"] == true => json!({"status":"pass","doctor":result}),
        Ok(result) => json!({"status":"unknown","doctor":result}),
        Err(error) => json!({"status":"fail","code":error.code(),"reason":error.to_string()}),
    };
    let capabilities = service.execute(Request::Capabilities {})?;
    let status = if writable["status"] == "fail" || media["status"] == "fail" {
        "needs-attention"
    } else if writable["status"] != "pass"
        || media["status"] != "pass"
        || space["status"] != "measured"
    {
        "incomplete"
    } else {
        "local-checks-passed"
    };
    Ok(
        json!({"status":status,"workspace":{"path":workspace,"writable":writable,"freeSpace":space,
            "persistence":{"status":"unknown","reason":"Requires a host restart or recreation test."},
            "filesystemLocking":{"status":"unknown","reason":"Create/remove does not test cross-process locking."}},
        "media":media,"capabilities":capabilities,
        "host":{"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "modelImageConsumption":{"status":"unknown","reason":"Requires an actual image-content trial in the chosen host."},
            "modelAudioConsumption":{"status":"unknown","reason":"Requires an actual audio-content trial in the chosen host."},
            "accountCompatibility":{"status":"unknown","reason":"This local diagnostic does not exercise a hosted account or file delivery."}},
        "guidance":"Local checks do not establish persistent storage, media consumption by the model, resource guarantees, host compatibility or creator approval."}),
    )
}
