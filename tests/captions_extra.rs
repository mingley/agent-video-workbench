use agent_video_workbench::{
    media,
    service::{Request, Service, envelope},
    store::Store,
    workflow::{Compose, Cue, Transcript},
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    process::{Command, Stdio},
};

// A generated, minimal table fixture is sufficient for the width reader in
// compose. Actual rendered captions use a real font in the media regression.
fn metric_fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 220];
    let write16 = |bytes: &mut [u8], at: usize, value: u16| {
        bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
    };
    let write32 = |bytes: &mut [u8], at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    };
    write16(&mut bytes, 4, 4);
    for (index, (tag, offset)) in [
        (b"head", 80),
        (b"hhea", 112),
        (b"hmtx", 152),
        (b"cmap", 160),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = 12 + index * 16;
        bytes[entry..entry + 4].copy_from_slice(tag);
        write32(&mut bytes, entry + 8, offset);
    }
    write16(&mut bytes, 80 + 18, 1000); // units per em
    write16(&mut bytes, 112 + 4, 800); // ascent
    write16(&mut bytes, 112 + 6, (-200_i16) as u16); // descent
    write16(&mut bytes, 112 + 34, 1); // one width metric
    write16(&mut bytes, 152, 500);
    write16(&mut bytes, 160 + 2, 1); // one Unicode cmap record
    write16(&mut bytes, 160 + 4, 3);
    write16(&mut bytes, 160 + 6, 10);
    write32(&mut bytes, 160 + 8, 12);
    write16(&mut bytes, 172, 12); // empty format 12 map, fallback width
    bytes
}

fn cue(id: &str, start_ms: i64, end_ms: i64) -> Cue {
    Cue {
        id: id.into(),
        start_ms,
        end_ms,
        text: format!("Text for {id}"),
    }
}

fn setup(cues: Vec<Cue>) -> (tempfile::TempDir, Store, Service) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Caption timing").unwrap();
    let font = metric_fixture();
    let hash = format!("{:x}", Sha256::digest(&font));
    std::fs::write(root.join("originals").join(&hash), &font).unwrap();
    let snapshot = store.project().unwrap();
    let seed: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0","projectId":snapshot.project_id,"baseRevision":0,
        "idempotencyKey":"seed-caption-timing","operations":[
            {"id":"font","op":"asset.add","params":{"id":"font","uri":format!("originals/{hash}"),
                "kind":"font","fingerprint":{"strategy":"sha256","sizeBytes":font.len(),"sha256":hash}}},
            {"id":"video","op":"asset.add","params":{"id":"source","uri":"originals/source","kind":"video",
                "metadata":{"status":"available","duration":{"value":10,"rate":{"numerator":1,"denominator":1}},
                    "video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},
                    "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0}}}}
        ]
    })).unwrap();
    store.apply(&seed, false).unwrap();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "reviewed".into(),
                cues,
            },
            1,
            "seed-cues",
        )
        .unwrap();
    let service = Service {
        root: Some(temp.path().into()),
        backend: media::backend("unused".into(), "unused".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    (temp, store, service)
}

fn edit() -> Compose {
    serde_json::from_value(
        json!({"outputId":"candidate","name":"Candidate","fontAssetId":"font",
        "width":360,"height":640,"fontSize":20,
        "cuts":[{"id":"take","assetId":"source","startMs":0,"endMs":1000}]}),
    )
    .unwrap()
}

#[test]
fn clipped_cue_refusal_is_atomic_and_equivalent_across_transports() {
    let (temp, mut store, service) =
        setup(vec![cue("short", 200, 220), cue("boundary", 990, 1020)]);
    let before = store.project().unwrap();
    let history = store.history().unwrap();
    let request = json!({"command":"compose","project":"project","expectedRevision":2,
        "key":"same-compose-key","edit":edit()});
    let direct =
        envelope(service.execute(serde_json::from_value::<Request>(request.clone()).unwrap()));
    assert_eq!(direct["ok"], false);
    assert_eq!(direct["error"]["code"], "E_INVALID_REQUEST");
    let message = direct["error"]["message"].as_str().unwrap();
    for part in [
        "boundary",
        "source",
        "take",
        "990..1000ms",
        "zero output frames",
    ] {
        assert!(message.contains(part), "{message}");
    }
    let mut dry_run = request.clone();
    dry_run["dryRun"] = json!(true);
    let dry_result = envelope(service.execute(serde_json::from_value::<Request>(dry_run).unwrap()));
    assert_eq!(dry_result["error"]["message"], direct["error"]["message"]);
    let messages = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"avw","arguments":request}})
    );
    let mut output = Vec::new();
    agent_video_workbench::mcp::serve(&service, &mut std::io::Cursor::new(messages), &mut output)
        .unwrap();
    let reply: Value =
        serde_json::from_str(String::from_utf8(output).unwrap().lines().nth(1).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"], direct);
    let mut cli = Command::new(env!("CARGO_BIN_EXE_avw"))
        .arg("--workspace")
        .arg(temp.path())
        .arg("request")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    cli.stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&request).unwrap().as_bytes())
        .unwrap();
    let result = cli.wait_with_output().unwrap();
    assert!(!result.status.success());
    let cli_response: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(cli_response["error"]["code"], direct["error"]["code"]);
    assert_eq!(cli_response["error"]["message"], direct["error"]["message"]);
    assert_eq!(store.project().unwrap(), before);
    assert_eq!(store.history().unwrap(), history);
    assert!(store.request_outcome("same-compose-key").unwrap().is_none());

    // A reviewed timing correction can safely reuse a key that never committed.
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "correction".into(),
                cues: vec![cue("short", 200, 220), cue("boundary", 900, 1000)],
            },
            2,
            "correct-cues",
        )
        .unwrap();
    store
        .compose(&edit(), 3, "same-compose-key", false)
        .unwrap();
    let project = store.project().unwrap();
    let captions: Vec<_> = project
        .require_sequence("candidate")
        .unwrap()
        .tracks
        .iter()
        .filter(|track| track.track_type == agentcut_core::TrackType::Caption)
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(captions.len(), 2);
    let short = captions
        .iter()
        .find(|item| item.extensions["avw.sourceCue"]["cueId"] == "short")
        .unwrap();
    assert_eq!(short.duration.value, 1);
    assert_eq!(short.extensions["avw.sourceCue"]["sourceStartMs"], 200);
    assert_eq!(short.extensions["avw.sourceCue"]["sourceEndMs"], 220);
}

#[test]
fn adjacent_or_overlapping_cues_are_never_silently_lost() {
    for (name, cues) in [
        (
            "adjacent",
            vec![cue("first", 100, 120), cue("collapsed", 120, 125)],
        ),
        (
            "overlapping",
            vec![cue("first", 100, 200), cue("collapsed", 110, 115)],
        ),
        ("wholly short", vec![cue("collapsed", 200, 210)]),
    ] {
        let (_temp, mut store, _service) = setup(cues);
        let before = store.project().unwrap();
        let error = store
            .compose(&edit(), 2, "reject-collapsed", false)
            .unwrap_err();
        assert!(error.to_string().contains("collapsed"), "{name}: {error}");
        assert_eq!(store.project().unwrap(), before, "{name}");
        assert!(store.request_outcome("reject-collapsed").unwrap().is_none());
    }
}

#[test]
fn cut_start_fragments_refuse_but_nonintersecting_cues_do_not() {
    let (_temp, mut store, _service) = setup(vec![cue("clipped_start", 900, 1020)]);
    let before = store.project().unwrap();
    let mut after_cue_starts = edit();
    after_cue_starts.cuts[0].start_ms = 1010;
    after_cue_starts.cuts[0].end_ms = 2000;
    let error = store
        .compose(&after_cue_starts, 2, "clipped-start", false)
        .unwrap_err()
        .to_string();
    for part in [
        "clipped_start",
        "source",
        "take",
        "1010..1020ms",
        "zero output frames",
    ] {
        assert!(error.contains(part), "{error}");
    }
    assert_eq!(store.project().unwrap(), before);

    let (_temp, mut store, _service) = setup(vec![cue("outside", 1000, 1050)]);
    store.compose(&edit(), 2, "no-intersection", false).unwrap();
    let project = store.project().unwrap();
    let count: usize = project
        .require_sequence("candidate")
        .unwrap()
        .tracks
        .iter()
        .filter(|track| track.track_type == agentcut_core::TrackType::Caption)
        .map(|track| track.items.len())
        .sum();
    assert_eq!(count, 0);
}

#[test]
fn renderable_adjacent_cues_remain_and_overlapping_cues_refuse_atomically() {
    let (_temp, mut store, _service) = setup(vec![cue("first", 100, 200), cue("second", 200, 300)]);
    store
        .compose(&edit(), 2, "adjacent-compose", false)
        .unwrap();
    let project = store.project().unwrap();
    let captions: Vec<_> = project
        .require_sequence("candidate")
        .unwrap()
        .tracks
        .iter()
        .filter(|track| track.track_type == agentcut_core::TrackType::Caption)
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(captions.len(), 2);
    assert!(
        captions
            .iter()
            .any(|item| item.extensions["avw.sourceCue"]["cueId"] == "first")
    );
    assert!(
        captions
            .iter()
            .any(|item| item.extensions["avw.sourceCue"]["cueId"] == "second")
    );

    let (_temp, mut store, _service) = setup(vec![cue("first", 100, 200), cue("second", 150, 250)]);
    let before = store.project().unwrap();
    let error = store
        .compose(&edit(), 2, "overlap-compose", false)
        .unwrap_err();
    assert!(error.to_string().contains("overlap"), "{error}");
    assert_eq!(store.project().unwrap(), before);
    assert!(store.request_outcome("overlap-compose").unwrap().is_none());
}

#[test]
fn successful_older_compose_requests_replay_after_cues_change() {
    let (_temp, mut store, _service) = setup(vec![cue("boundary", 900, 1000)]);
    let outcome = store
        .compose(&edit(), 2, "original-compose", false)
        .unwrap();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "later-correction".into(),
                cues: vec![cue("boundary", 990, 1020)],
            },
            3,
            "change-cue-after-compose",
        )
        .unwrap();
    let before = store.project().unwrap();
    assert_eq!(
        store
            .compose(&edit(), 2, "original-compose", false)
            .unwrap(),
        outcome
    );
    assert_eq!(store.project().unwrap(), before);
    let mut new_edit = edit();
    new_edit.output_id = "revised".into();
    assert!(
        store
            .compose(&new_edit, 4, "new-compose", false)
            .unwrap_err()
            .to_string()
            .contains("zero output frames")
    );
    assert_eq!(store.project().unwrap(), before);
}
