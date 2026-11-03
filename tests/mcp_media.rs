use agent_video_workbench::{mcp, media, service::Service, store::Store};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn service(root: &Path, ffmpeg: &Path) -> Service {
    Service {
        root: Some(root.to_path_buf()),
        backend: media::backend(ffmpeg.to_path_buf(), "unused-ffprobe".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    }
}

fn call(service: &Service, protocol: &str, args: Value) -> Value {
    let request = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":protocol}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"avw-media","arguments":args}}),
    );
    let mut output = Vec::new();
    mcp::serve(service, &mut std::io::Cursor::new(request), &mut output).unwrap();
    serde_json::from_str(String::from_utf8(output).unwrap().lines().nth(1).unwrap()).unwrap()
}

fn render_fixture(project: &Path, png: &[u8]) -> (Store, std::path::PathBuf, std::path::PathBuf) {
    let store = Store::create(project, "MCP image").unwrap();
    let dir = project.join("renders/test-render");
    fs::create_dir(&dir).unwrap();
    let sheet = dir.join("sheet.png");
    fs::write(&sheet, png).unwrap();
    let video = dir.join("video.mp4");
    fs::write(&video, b"placeholder").unwrap();
    let manifest = dir.join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({"contactSheet":{"sha256":media::hash_file(&sheet).unwrap()}}))
            .unwrap(),
    )
    .unwrap();
    store.start_job("render", 0).unwrap();
    store
        .finish_job("render", &json!({"path":video,"manifest":manifest}))
        .unwrap();
    (store, sheet, manifest)
}

#[test]
fn image_output_is_explicit_verified_bounded_and_workspace_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let project = workspace.join("project");
    let png = b"\x89PNG\r\n\x1a\nhello";
    let (_store, sheet, manifest) = render_fixture(&project, png);
    let service = service(&workspace, Path::new("unused-ffmpeg"));
    let args = json!({"kind":"contact-sheet","project":"project","jobId":"render"});
    let response = call(&service, "2025-11-25", args.clone());
    assert_eq!(response["result"]["structuredContent"]["ok"], true);
    assert_eq!(
        response["result"]["structuredContent"]["result"]["bytes"],
        png.len()
    );
    assert_eq!(response["result"]["content"][1]["type"], "image");
    assert_eq!(response["result"]["content"][1]["mimeType"], "image/png");
    assert_eq!(
        response["result"]["content"][1]["data"],
        "iVBORw0KGgpoZWxsbw=="
    );

    fs::write(&sheet, b"tampered").unwrap();
    let changed = call(&service, "2025-11-25", args.clone());
    assert_eq!(changed["result"]["isError"], true);
    assert_eq!(changed["result"]["content"].as_array().unwrap().len(), 1);

    let mut large = vec![0u8; 2 * 1024 * 1024 + 1];
    large[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    fs::write(&sheet, large).unwrap();
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({"contactSheet":{"sha256":media::hash_file(&sheet).unwrap()}}))
            .unwrap(),
    )
    .unwrap();
    let too_large = call(&service, "2025-11-25", args.clone());
    assert_eq!(too_large["result"]["isError"], true);
    assert!(
        too_large["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("2 MiB")
    );

    let outside = temp.path().join("outside.png");
    fs::write(&outside, png).unwrap();
    fs::remove_file(&sheet).unwrap();
    std::os::unix::fs::symlink(&outside, &sheet).unwrap();
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({"contactSheet":{"sha256":media::hash_file(&outside).unwrap()}}))
            .unwrap(),
    )
    .unwrap();
    let escape = call(&service, "2025-11-25", args.clone());
    assert_eq!(escape["result"]["isError"], true);
    let arbitrary_path = call(
        &service,
        "2025-11-25",
        json!({"kind":"contact-sheet","project":"project","jobId":"render","path":"/etc/passwd"}),
    );
    assert_eq!(arbitrary_path["result"]["isError"], true);
    let outside_project = call(
        &service,
        "2025-11-25",
        json!({"kind":"contact-sheet","project":"..","jobId":"render"}),
    );
    assert_eq!(outside_project["result"]["isError"], true);
}

#[test]
fn audio_uses_only_verified_proxy_and_reports_actual_returned_samples() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let project = workspace.join("project");
    let store = Store::create(&project, "MCP audio").unwrap();
    let dir = project.join("analysis/proxy-job");
    fs::create_dir(&dir).unwrap();
    let proxy = dir.join("proxy.mp4");
    fs::write(&proxy, b"bounded proxy fixture").unwrap();
    let report = dir.join("result.json");
    fs::write(&report, serde_json::to_vec(&json!({
        "schemaVersion":1,"sourceSha256":"source-identity",
        "task":{"kind":"proxy","assetId":"source","startMs":1200,"endMs":5200,"width":320,"height":180},
        "data":{"file":"proxy.mp4"},"files":[{"name":"proxy.mp4","sha256":media::hash_file(&proxy).unwrap()}]
    })).unwrap()).unwrap();
    store.start_job("proxy", 0).unwrap();
    store.finish_job("proxy", &json!({"path":report,"sha256":media::hash_file(&report).unwrap(),"mimeType":"application/json"})).unwrap();
    let ffmpeg = temp.path().join("ffmpeg-fixture");
    fs::write(&ffmpeg, "#!/bin/sh\nprintf '\\001\\000\\002\\000'\n").unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
    let service = service(&workspace, &ffmpeg);
    let args = json!({"kind":"audio-proxy","project":"project","jobId":"proxy","startMs":500,"durationMs":1000});
    let result = call(&service, "2025-11-25", args.clone());
    assert_eq!(result["result"]["structuredContent"]["ok"], true);
    let metadata = &result["result"]["structuredContent"]["result"];
    assert_eq!(metadata["requestedSourceStartMs"], 1700);
    assert_eq!(metadata["requestedSourceEndMs"], 2700);
    assert_eq!(metadata["sampleCount"], 2);
    assert_eq!(metadata["returnedFullRequestedDuration"], false);
    assert_eq!(result["result"]["content"][1]["type"], "audio");
    assert_eq!(result["result"]["content"][1]["mimeType"], "audio/wav");
    assert!(
        result["result"]["content"][1]["data"]
            .as_str()
            .unwrap()
            .starts_with("UklGR")
    );

    // A proxy changed by the backend during decoding must never be returned,
    // even though the pre-decode artifact check succeeded.
    fs::write(
        &ffmpeg,
        "#!/bin/sh\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = -i ]; then printf changed > \"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '\\001\\000\\002\\000'\n",
    )
    .unwrap();
    let changed_during_extraction = call(&service, "2025-11-25", args.clone());
    assert_eq!(changed_during_extraction["result"]["isError"], true);
    assert_eq!(
        changed_during_extraction["result"]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::write(&proxy, b"bounded proxy fixture").unwrap();
    fs::write(&ffmpeg, "#!/bin/sh\nprintf '\\001\\000\\002\\000'\n").unwrap();

    let unsupported = call(&service, "2024-11-05", args.clone());
    assert_eq!(unsupported["result"]["isError"], true);
    assert!(
        unsupported["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("2025-03-26")
    );
    let too_long = call(
        &service,
        "2025-11-25",
        json!({"kind":"audio-proxy","project":"project","jobId":"proxy","startMs":0,"durationMs":15001}),
    );
    assert_eq!(too_long["result"]["isError"], true);
    let outside = call(
        &service,
        "2025-11-25",
        json!({"kind":"audio-proxy","project":"project","jobId":"proxy","startMs":3500,"durationMs":1000}),
    );
    assert_eq!(outside["result"]["isError"], true);
    fs::write(&proxy, b"tampered").unwrap();
    let changed = call(&service, "2025-11-25", args);
    assert_eq!(changed["result"]["isError"], true);
    assert_eq!(changed["result"]["content"].as_array().unwrap().len(), 1);
}
