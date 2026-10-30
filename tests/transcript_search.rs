use agent_video_workbench::{
    media,
    service::{Request, Service},
    store::Store,
    workflow::{Cue, Transcript},
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use std::process::Command;

fn cue(id: &str, start_ms: i64, end_ms: i64, text: &str) -> Cue {
    Cue {
        id: id.into(),
        start_ms,
        end_ms,
        text: text.into(),
    }
}

fn transcript(asset_id: &str, cues: Vec<Cue>) -> Transcript {
    Transcript {
        asset_id: asset_id.into(),
        language: "en".into(),
        provider: "reviewed".into(),
        cues,
    }
}

fn setup() -> (tempfile::TempDir, Store, Service) {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("project"), "Phrase search").unwrap();
    let project = store.project().unwrap();
    let operations: Vec<_> = ["a", "b", "c", "d"].into_iter().map(|id| {
        let video = (id != "b").then(|| json!({
            "size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},
            "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
            "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"
        }));
        let duration = (id != "c").then(|| json!({"value":600,"rate":{"numerator":1,"denominator":1}}));
        let fingerprint = (id != "d").then(|| json!({"strategy":"sha256","sizeBytes":0,"sha256":"0".repeat(64)}));
        let mut params = json!({"id":id,"uri":format!("originals/{id}"),"kind":"video",
            "metadata":{"status":"available","duration":duration,"video":video}});
        if let Some(fp) = fingerprint { params["fingerprint"] = fp; }
        json!({"id":format!("add-{id}"),"op":"asset.add","params":params})
    }).collect();
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":project.project_id,"baseRevision":0,"idempotencyKey":"seed-search","operations":operations})).unwrap();
    store.apply(&batch, false).unwrap();
    store
        .transcript_import(
            transcript(
                "a",
                vec![
                    cue("late_end", 20000, 20500, "the packaging"),
                    cue("first_end", 1600, 1900, "the packaging"),
                    cue("second_start", 3000, 3500, "we changed"),
                    cue("first_start", 1000, 1500, "We changed"),
                    cue("late_start", 10000, 10500, "we changed"),
                    cue("second_end", 3600, 3900, "the packaging"),
                ],
            ),
            1,
            "transcript-a",
        )
        .unwrap();
    store
        .transcript_import(
            transcript("b", vec![cue("only_end", 0, 500, "the packaging")]),
            2,
            "transcript-b",
        )
        .unwrap();
    store
        .transcript_import(
            transcript(
                "c",
                vec![cue("whole", 1000, 2000, "WE CHANGED\nTHE PACKAGING")],
            ),
            3,
            "transcript-c",
        )
        .unwrap();
    let service = Service {
        root: Some(tmp.path().into()),
        backend: media::backend("ffmpeg".into(), "ffprobe".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    (tmp, store, service)
}

fn search(service: &Service, request: Value) -> agent_video_workbench::Result<Value> {
    service.execute(serde_json::from_value::<Request>(request).unwrap())
}

#[test]
fn phrase_search_groups_source_cues_and_paginates_at_a_pinned_revision() {
    let (_tmp, mut store, service) = setup();
    let base = json!({"command":"transcript-search","project":"project", "query":"  WE changed\nTHE   packaging  ",
        "acrossCues":true,"expectedRevision":4,"limit":1});
    let first = search(&service, base.clone()).unwrap();
    assert_eq!(first["revision"], 4);
    assert_eq!(first["mode"], "across-cues");
    assert_eq!(first["items"][0]["assetId"], "a");
    assert_eq!(
        first["items"][0]["matchedCueIds"],
        json!(["first_start", "first_end"])
    );
    assert_eq!(
        (
            first["items"][0]["sourceStartMs"].as_i64(),
            first["items"][0]["sourceEndMs"].as_i64()
        ),
        (Some(1000), Some(1900))
    );
    assert_eq!(first["items"][0]["contextAfter"]["id"], "second_start");
    assert_eq!(first["items"][0]["transcript"]["language"], "en");
    assert_eq!(
        first["items"][0]["transcript"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let preview = &first["items"][0]["previewRequest"];
    assert_eq!(preview["project"], "project");
    assert_eq!(preview["expectedRevision"], 4);
    assert_eq!(
        (
            preview["task"]["startMs"].as_i64(),
            preview["task"]["endMs"].as_i64()
        ),
        (Some(0), Some(2900))
    );
    assert!(
        preview["key"]
            .as_str()
            .unwrap()
            .starts_with("avw-preview-v1-")
    );
    assert!(first["items"][0]["previewUnavailableReason"].is_null());
    let queued = service
        .execute(serde_json::from_value::<Request>(preview.clone()).unwrap())
        .unwrap();
    assert_eq!(queued["state"], "queued");
    let replay = service
        .execute(serde_json::from_value::<Request>(preview.clone()).unwrap())
        .unwrap();
    assert_eq!(replay["id"], queued["id"]);
    assert_eq!(first["nextOffset"], 1);
    let mut second_request = base.clone();
    second_request["offset"] = json!(1);
    let second = search(&service, second_request).unwrap();
    assert_eq!(
        second["items"][0]["matchedCueIds"],
        json!(["second_start", "second_end"])
    );
    assert_eq!(second["nextOffset"], 2);
    let mut third_request = base.clone();
    third_request["offset"] = json!(2);
    let third = search(&service, third_request).unwrap();
    assert_eq!(third["items"][0]["assetId"], "c");
    assert_eq!(third["items"][0]["matchedCueIds"], json!(["whole"]));
    assert!(third["items"][0]["previewRequest"].is_null());
    assert_eq!(
        third["items"][0]["previewUnavailableReason"],
        "source-duration-unavailable"
    );
    assert!(third["nextOffset"].is_null());
    let restricted = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":"changed the packaging",
        "acrossCues":true,"assetId":"b"}),
    )
    .unwrap();
    assert!(restricted["items"].as_array().unwrap().is_empty());
    // Original mode keeps its existing per-cue substring and output shape.
    let legacy = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":"changed"}),
    )
    .unwrap();
    assert!(
        legacy["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("cue").is_some())
    );
    assert!(legacy.get("mode").is_none());
    let old_phrase = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":"changed the packaging"}),
    )
    .unwrap();
    assert!(old_phrase["items"].as_array().unwrap().is_empty());
    let previous = first["items"][0]["transcript"]["sha256"].clone();
    store
        .transcript_import(
            transcript("a", vec![cue("corrected", 1000, 1900, "Changed packaging")]),
            4,
            "replace-transcript",
        )
        .unwrap();
    let after_edit = service
        .execute(serde_json::from_value::<Request>(preview.clone()).unwrap())
        .unwrap();
    assert_eq!(after_edit["id"], queued["id"]);
    assert_eq!(
        search(&service, base).unwrap_err().code(),
        "E_REVISION_CONFLICT"
    );
    let fresh = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":"changed packaging",
        "acrossCues":true,"assetId":"a","expectedRevision":5}),
    )
    .unwrap();
    assert_ne!(fresh["items"][0]["transcript"]["sha256"], previous);
    assert_eq!(fresh["items"][0]["matchedCueIds"], json!(["corrected"]));
}

#[test]
fn phrase_limits_unicode_overlap_gap_and_input_validation() {
    let (_tmp, mut store, service) = setup();
    let mut cues = vec![
        cue("unicode_first", 0, 400, "À  CAFÉ"),
        cue("unicode_second", 450, 900, "NOIR"),
        cue("overlap_a", 1500, 2000, "a a"),
        cue("overlap_b", 2100, 2500, "a"),
        cue("simultaneous_a", 12000, 13000, "danger alpha"),
        cue("simultaneous_b", 12500, 13500, "bravo"),
        cue("enclosing", 15000, 24000, "outer"),
        cue("nested", 16000, 17000, "inner"),
        cue("much_later", 23000, 23500, "later"),
    ];
    for (index, word) in [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ]
    .iter()
    .enumerate()
    {
        cues.push(cue(
            &format!("word{index}"),
            3000 + index as i64 * 300,
            3200 + index as i64 * 300,
            word,
        ));
    }
    store
        .transcript_import(transcript("a", cues), 4, "edge-transcript")
        .unwrap();
    let unicode = search(&service, json!({"command":"transcript-search","project":"project","query":"café noir","acrossCues":true,"assetId":"a"})).unwrap();
    assert_eq!(
        unicode["items"][0]["matchedCueIds"],
        json!(["unicode_first", "unicode_second"])
    );
    let overlap = search(&service, json!({"command":"transcript-search","project":"project","query":"a a","acrossCues":true,"assetId":"a"})).unwrap();
    assert!(
        overlap["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["matchedCueIds"] == json!(["overlap_a", "overlap_b"]))
    );
    let eight = search(&service, json!({"command":"transcript-search","project":"project","query":"one two three four five six seven eight","acrossCues":true,"assetId":"a"})).unwrap();
    assert_eq!(
        eight["items"][0]["matchedCueIds"].as_array().unwrap().len(),
        8
    );
    let nine = search(&service, json!({"command":"transcript-search","project":"project","query":"one two three four five six seven eight nine","acrossCues":true,"assetId":"a"})).unwrap();
    assert!(nine["items"].as_array().unwrap().is_empty());
    let no_gap = search(&service, json!({"command":"transcript-search","project":"project","query":"café noir","acrossCues":true,"maxGapMs":0,"assetId":"a"})).unwrap();
    assert!(no_gap["items"].as_array().unwrap().is_empty());
    for phrase in ["alpha bravo", "outer inner", "outer inner later"] {
        let separated = search(
            &service,
            json!({"command":"transcript-search","project":"project",
            "query":phrase,"acrossCues":true,"maxGapMs":0,"assetId":"a"}),
        )
        .unwrap();
        assert!(separated["items"].as_array().unwrap().is_empty());
    }
    for request in [
        json!({"query":"  ","acrossCues":true}),
        json!({"query":"x".repeat(257),"acrossCues":true}),
        json!({"query":"test","acrossCues":true,"maxGapMs":5001}),
        json!({"query":"test","maxGapMs":0}),
        json!({"query":"test","acrossCues":true,"limit":101}),
        json!({"query":"test","acrossCues":true,"offset":1}),
    ] {
        let mut request = request;
        request["command"] = json!("transcript-search");
        request["project"] = json!("project");
        assert_eq!(
            search(&service, request).unwrap_err().code(),
            "E_INVALID_REQUEST"
        );
    }
}

#[test]
fn phrase_search_is_identical_in_cli_json_named_cli_and_mcp() {
    let (tmp, _store, service) = setup();
    let request = json!({"command":"transcript-search","project":"project","query":"changed the packaging",
        "acrossCues":true,"maxGapMs":500,"expectedRevision":4,"limit":2});
    let direct = search(&service, request.clone()).unwrap();
    let request_file = tmp.path().join("request.json");
    std::fs::write(&request_file, serde_json::to_vec(&request).unwrap()).unwrap();
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_avw"))
            .args(["--workspace", tmp.path().to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    };
    assert_eq!(cli(&["request", request_file.to_str().unwrap()]), direct);
    assert_eq!(
        cli(&[
            "transcript-search",
            "project",
            "changed the packaging",
            "--across-cues",
            "--max-gap-ms",
            "500",
            "--expected-revision",
            "4",
            "--limit",
            "2"
        ]),
        direct
    );
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"avw","arguments":request}})
    );
    let mut output = Vec::new();
    agent_video_workbench::mcp::serve(&service, &mut std::io::Cursor::new(input), &mut output)
        .unwrap();
    let reply: Value =
        serde_json::from_str(String::from_utf8(output).unwrap().lines().nth(1).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["result"], direct);
}

#[test]
fn generated_plan_and_phrase_previews_are_bound_to_the_queuing_backend() {
    let (tmp, _store, service) = setup();
    let other = Service {
        root: Some(tmp.path().into()),
        backend: media::backend("other-ffmpeg".into(), "other-ffprobe".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    let requests = [
        json!({"command":"transcript-search","project":"project","query":"changed the packaging","acrossCues":true,"assetId":"a","expectedRevision":4}),
        json!({"command":"plan-edit","project":"project","expectedRevision":4,"plan":{
            "edit":{"outputId":"candidate","name":"Candidate","cuts":[{"id":"take","assetId":"a","startMs":0,"endMs":6000}]},
            "brief":{"objective":"Keep the reviewed delivery"},"decisions":[{"cutId":"take","reason":"Retain the thought"}]
        }}),
    ];
    for request in requests {
        let result_a = search(&service, request.clone()).unwrap();
        let result_b = search(&other, request).unwrap();
        let (preview_a, preview_b) = if result_a["mode"] == "across-cues" {
            (
                &result_a["items"][0]["previewRequest"],
                &result_b["items"][0]["previewRequest"],
            )
        } else {
            assert_eq!(result_a["planSha256"], result_b["planSha256"]);
            (
                &result_a["cuts"][0]["contextWindows"][0]["previewRequest"],
                &result_b["cuts"][0]["contextWindows"][0]["previewRequest"],
            )
        };
        assert_ne!(preview_a["key"], preview_b["key"]);
        let wrong = other
            .execute(serde_json::from_value::<Request>(preview_a.clone()).unwrap())
            .unwrap_err();
        assert_eq!(wrong.code(), "E_INVALID_REQUEST");
        assert!(wrong.to_string().contains("regenerate"));
        let job_a = service
            .execute(serde_json::from_value::<Request>(preview_a.clone()).unwrap())
            .unwrap();
        let repeat_a = service
            .execute(serde_json::from_value::<Request>(preview_a.clone()).unwrap())
            .unwrap();
        assert_eq!(job_a["id"], repeat_a["id"]);
        let job_b = other
            .execute(serde_json::from_value::<Request>(preview_b.clone()).unwrap())
            .unwrap();
        assert_ne!(job_a["id"], job_b["id"]);
    }
}

#[test]
fn previews_are_omitted_with_reasons_and_fit_the_full_match_in_five_minutes() {
    let (_tmp, mut store, service) = setup();
    let audio = search(&service, json!({"command":"transcript-search","project":"project","query":"packaging","acrossCues":true,"assetId":"b"})).unwrap();
    assert_eq!(
        audio["items"][0]["previewUnavailableReason"],
        "source-not-video"
    );
    store
        .transcript_import(
            transcript("d", vec![cue("nohash", 0, 1000, "nohash")]),
            4,
            "nohash-transcript",
        )
        .unwrap();
    let nohash = search(&service, json!({"command":"transcript-search","project":"project","query":"nohash","acrossCues":true,"assetId":"d"})).unwrap();
    assert_eq!(
        nohash["items"][0]["previewUnavailableReason"],
        "source-hash-unavailable"
    );
    for (revision, duration, expected) in [
        (5, 299_500, None),
        (6, 300_000, None),
        (7, 300_001, Some("match-exceeds-proxy-limit")),
    ] {
        store
            .transcript_import(
                transcript("a", vec![cue("long", 1000, 1000 + duration, "long match")]),
                revision,
                &format!("long-{duration}"),
            )
            .unwrap();
        let found = search(&service, json!({"command":"transcript-search","project":"project","query":"long match","acrossCues":true,"assetId":"a"})).unwrap();
        let item = &found["items"][0];
        if let Some(reason) = expected {
            assert!(item["previewRequest"].is_null());
            assert_eq!(item["previewUnavailableReason"], reason);
        } else {
            let task = &item["previewRequest"]["task"];
            let start = task["startMs"].as_i64().unwrap();
            let end = task["endMs"].as_i64().unwrap();
            assert!(start <= 1000 && end >= 1000 + duration && end - start <= 300_000);
            let job = service
                .execute(serde_json::from_value::<Request>(item["previewRequest"].clone()).unwrap())
                .unwrap();
            assert_eq!(job["state"], "queued");
        }
    }
}

#[test]
fn generated_previews_cannot_cross_into_an_unrelated_workspace_or_source() {
    let (_root_a, store_a, service_a) = setup();
    let (_root_b, _store_b, service_b) = setup();
    let request = json!({"command":"transcript-search","project":"project","query":"changed the packaging","acrossCues":true,"assetId":"a","expectedRevision":4});
    let found_a = search(&service_a, request.clone()).unwrap();
    let found_b = search(&service_b, request).unwrap();
    let preview_a = &found_a["items"][0]["previewRequest"];
    let preview_b = &found_b["items"][0]["previewRequest"];
    assert_ne!(preview_a["key"], preview_b["key"]);
    let error = service_b
        .execute(serde_json::from_value::<Request>(preview_a.clone()).unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "E_INVALID_REQUEST");
    assert!(error.to_string().contains("regenerate"));
    assert_eq!(
        service_b
            .execute(serde_json::from_value::<Request>(preview_b.clone()).unwrap())
            .unwrap()["state"],
        "queued"
    );
    let parsed = serde_json::from_value::<Request>(preview_a.clone()).unwrap();
    let Request::AnalyzeStart {
        task,
        expected_revision,
        ..
    } = parsed
    else {
        panic!("expected proxy request")
    };
    let input = agent_video_workbench::jobs::RenderInput {
        sequence: "seq_main".into(),
        priority: 0,
        expected_revision,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: Some(agent_video_workbench::analysis::Input {
            task,
            provider: None,
        }),
    };
    let original = store_a.project().unwrap();
    let original_key = agent_video_workbench::jobs::preview_input_key(&input, &original).unwrap();
    assert_eq!(original_key, preview_a["key"]);
    let mut different_source = original.clone();
    different_source
        .assets
        .iter_mut()
        .find(|asset| asset.id == "a")
        .unwrap()
        .fingerprint
        .sha256 = Some("f".repeat(64));
    assert_ne!(
        agent_video_workbench::jobs::preview_input_key(&input, &different_source).unwrap(),
        original_key
    );
}

#[test]
fn phrase_search_handles_the_full_twenty_thousand_cue_limit() {
    let (_tmp, mut store, service) = setup();
    let cues = (0..20_000)
        .map(|index| {
            let text = match index {
                19_998 => "noteworthy",
                19_999 => "finish",
                _ => "context",
            };
            cue(&format!("cue{index}"), index, index + 1, text)
        })
        .collect();
    store
        .transcript_import(transcript("a", cues), 4, "large-transcript")
        .unwrap();
    let result = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":"noteworthy finish",
        "acrossCues":true,"assetId":"a","expectedRevision":5}),
    )
    .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["items"][0]["matchedCueIds"],
        json!(["cue19998", "cue19999"])
    );
    assert!(result["nextOffset"].is_null());
}

#[test]
fn long_cues_and_maximum_phrase_keep_the_page_bounded() {
    let (_tmp, mut store, service) = setup();
    let query = "z".repeat(256);
    let mut cues: Vec<_> = (0..4000)
        .map(|index| cue(&format!("cue{index}"), index, index + 1, &"x".repeat(1900)))
        .collect();
    cues[3999].text = format!("{} {query}", "x".repeat(1600));
    let transcript = transcript("a", cues);
    assert!(serde_json::to_vec(&transcript).unwrap().len() < 8 * 1024 * 1024);
    store
        .transcript_import(transcript, 4, "large-cues")
        .unwrap();
    let result = search(
        &service,
        json!({"command":"transcript-search","project":"project","query":query,
        "acrossCues":true,"assetId":"a","expectedRevision":5,"limit":1}),
    )
    .unwrap();
    assert_eq!(result["items"][0]["matchedCueIds"], json!(["cue3999"]));
    assert!(result["nextOffset"].is_null());
    assert!(serde_json::to_vec(&result).unwrap().len() < 16 * 1024);
}
