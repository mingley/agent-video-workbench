use agent_video_workbench::{
    discovery::{self, EvidenceQuery, HighlightBrief, ProviderSearch},
    media,
    service::{Request, Service},
    store::Store,
    workflow::{Cue, Transcript},
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Store, Service) {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("project"), "Discovery").unwrap();
    let project = store.project().unwrap();
    let operations: Vec<_> = [("source", "a"), ("silent", "b")].into_iter().map(|(id,hex)| {
        json!({"id":format!("add-{id}"),"op":"asset.add","params":{"id":id,"uri":format!("originals/{id}"),"kind":"video",
            "fingerprint":{"strategy":"sha256","sizeBytes":0,"sha256":hex.repeat(64)},
            "metadata":{"status":"available","duration":{"value":120,"rate":{"numerator":1,"denominator":1}},
                "video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},"pixelFormat":"yuv420p",
                    "sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
                    "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}}})
    }).collect();
    let batch: OperationBatch = serde_json::from_value(
        json!({"schemaVersion":"1.0.0","projectId":project.project_id,
        "baseRevision":0,"idempotencyKey":"seed-discovery","operations":operations}),
    )
    .unwrap();
    store.apply(&batch, false).unwrap();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "fixture".into(),
                cues: vec![
                    Cue {
                        id: "intro".into(),
                        start_ms: 10_000,
                        end_ms: 11_000,
                        text: "Opening explanation".into(),
                    },
                    Cue {
                        id: "boundary".into(),
                        start_ms: 29_500,
                        end_ms: 30_500,
                        text: "Across the boundary".into(),
                    },
                    Cue {
                        id: "answer".into(),
                        start_ms: 45_000,
                        end_ms: 46_000,
                        text: "A useful answer".into(),
                    },
                    Cue {
                        id: "closing".into(),
                        start_ms: 90_100,
                        end_ms: 91_200,
                        text: "The closing thought".into(),
                    },
                ],
            },
            1,
            "import-discovery-transcript",
        )
        .unwrap();
    let service = Service {
        root: Some(temp.path().into()),
        backend: media::backend("ffmpeg".into(), "ffprobe".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    (temp, store, service)
}

fn query(value: Value) -> EvidenceQuery {
    serde_json::from_value(value).unwrap()
}
fn brief(value: Value) -> HighlightBrief {
    serde_json::from_value(value).unwrap()
}

#[test]
fn evidence_pages_cover_transcript_and_visual_context_with_scoped_replayable_proxies() {
    let (_temp, store, service) = setup();
    let project = store.project().unwrap();
    let first_query = query(json!({"assetId":"source","windowMs":30000,"limit":2}));
    let first = discovery::evidence(
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        &first_query,
    )
    .unwrap();
    assert_eq!(first["revision"], 2);
    assert_eq!(first["sourceSha256"], "a".repeat(64));
    assert_eq!(first["transcriptStatus"], "available");
    assert_eq!(first["totalWindows"], 4);
    assert_eq!(first["nextOffset"], 2);
    assert_eq!(first["items"][0]["cueExcerptsTotal"], 2);
    assert_eq!(first["items"][0]["cueExcerpts"][0]["id"], "intro");
    assert_eq!(first["items"][1]["cueExcerpts"][0]["id"], "boundary");
    assert_eq!(first["items"][1]["cueExcerpts"][1]["id"], "answer");
    assert_eq!(
        (
            first["items"][1]["sourceStartMs"].as_i64(),
            first["items"][1]["sourceEndMs"].as_i64()
        ),
        (Some(30000), Some(60000))
    );
    let mut next = first_query.clone();
    next.offset = 2;
    let page =
        discovery::evidence(&project, "project".as_ref(), &service.backend, 2, &next).unwrap();
    assert!(page["nextOffset"].is_null());
    assert_eq!(page["items"][0]["cueExcerptsTotal"], 0);
    assert_eq!(page["items"][1]["cueExcerpts"][0]["id"], "closing");
    let proxy = first["items"][0]["previewRequest"].clone();
    assert!(
        proxy["key"]
            .as_str()
            .unwrap()
            .starts_with("avw-preview-v1-")
    );
    let job = service
        .execute(serde_json::from_value::<Request>(proxy.clone()).unwrap())
        .unwrap();
    let replay = service
        .execute(serde_json::from_value::<Request>(proxy).unwrap())
        .unwrap();
    assert_eq!(job["id"], replay["id"]);
    assert_eq!(store.project().unwrap(), project);

    let no_transcript = discovery::evidence(
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        &query(json!({"assetId":"silent","startMs":5000,"endMs":6000,"windowMs":1000})),
    )
    .unwrap();
    assert_eq!(no_transcript["transcriptStatus"], "unavailable");
    assert_eq!(no_transcript["sourceSha256"], "b".repeat(64));
    assert!(no_transcript["items"][0]["previewRequest"].is_object());
}

#[test]
fn proposals_bind_sources_and_cues_and_produce_reviewable_plan_requests() {
    let (_temp, store, service) = setup();
    let project = store.project().unwrap();
    let page = discovery::evidence(
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        &query(json!({"assetId":"source","windowMs":30000})),
    )
    .unwrap();
    let transcript_sha = page["transcript"]["sha256"].as_str().unwrap();
    let input = json!({"objective":"Find an opening that explains the product","targetDurationMs":30000,
        "candidates":[{"id":"intro-cut","outputId":"intro-output","assetId":"source","sourceSha256":"a".repeat(64),
            "startMs":9000,"endMs":12000,"title":"Opening idea","reason":"The explanation starts here","confidence":0.7,
            "evidence":[{"kind":"transcript-cue","transcriptSha256":transcript_sha,"cueId":"intro"},
                {"kind":"observation","modality":"video","description":"Caller saw the product"}]}]});
    let result = discovery::prepare(
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        &brief(input),
    )
    .unwrap();
    assert_eq!(result["interpretation"], "caller-supplied");
    assert_eq!(result["items"][0]["sourceSha256"], "a".repeat(64));
    assert_eq!(result["items"][0]["evidence"][0]["cue"]["id"], "intro");
    assert_eq!(
        result["items"][0]["evidence"][1]["verification"],
        "caller-asserted"
    );
    assert_eq!(result["items"][0]["callerReportedConfidence"], 0.7);
    assert_eq!(result["items"][0]["reviewRequired"], true);
    assert_eq!(
        result["items"][0]["previewRequest"]["task"]["startMs"],
        8000
    );
    assert_eq!(result["items"][0]["previewRequest"]["task"]["endMs"], 13000);
    let plan = result["items"][0]["planRequest"].clone();
    assert_eq!(plan["command"], "plan-edit");
    assert_eq!(plan["plan"]["edit"]["cuts"][0]["id"], "intro-cut");
    assert_eq!(
        plan["plan"]["decisions"][0]["reason"],
        "The explanation starts here"
    );
    let checked = service
        .execute(serde_json::from_value::<Request>(plan).unwrap())
        .unwrap();
    assert_eq!(checked["revision"], 2);
    assert_eq!(store.project().unwrap(), project);
}

#[test]
fn refuses_stale_mismatched_unbounded_or_ungrounded_proposals() {
    let (_temp, store, service) = setup();
    let project = store.project().unwrap();
    let page = discovery::evidence(
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        &query(json!({"assetId":"source"})),
    )
    .unwrap();
    let transcript_sha = page["transcript"]["sha256"].as_str().unwrap();
    let base = json!({"objective":"Find a moment","candidates":[{"id":"cut","outputId":"output","assetId":"source",
        "sourceSha256":"a".repeat(64),"startMs":9000,"endMs":12000,"title":"Idea","reason":"Candidate",
        "evidence":[{"kind":"transcript-cue","transcriptSha256":transcript_sha,"cueId":"intro"}]}]});
    assert!(
        discovery::prepare(
            &project,
            "project".as_ref(),
            &service.backend,
            1,
            &brief(base.clone())
        )
        .unwrap_err()
        .to_string()
        .contains("revision conflict")
    );
    for change in 0..8 {
        let mut bad = base.clone();
        match change {
            0 => bad["candidates"][0]["sourceSha256"] = json!("b".repeat(64)),
            1 => bad["candidates"][0]["evidence"][0]["transcriptSha256"] = json!("0".repeat(64)),
            2 => bad["candidates"][0]["evidence"][0]["cueId"] = json!("answer"),
            3 => bad["candidates"][0]["endMs"] = json!(120001),
            4 => bad["candidates"][0]["confidence"] = json!(1.1),
            5 => bad["candidates"][0]["evidence"] = json!([]),
            6 => bad["candidates"][0]["id"] = json!("output"),
            7 => {
                bad["candidates"][0]["assetId"] = json!("silent");
                bad["candidates"][0]["sourceSha256"] = json!("b".repeat(64));
            }
            _ => unreachable!(),
        }
        assert!(
            discovery::prepare(
                &project,
                "project".as_ref(),
                &service.backend,
                2,
                &brief(bad)
            )
            .is_err(),
            "accepted change {change}"
        );
    }
    for value in [
        json!({"assetId":"source","endMs":120001}),
        json!({"assetId":"source","windowMs":999}),
        json!({"assetId":"source","limit":21}),
        json!({"assetId":"source","offset":100}),
    ] {
        assert!(
            discovery::evidence(
                &project,
                "project".as_ref(),
                &service.backend,
                2,
                &query(value)
            )
            .is_err()
        );
    }
}

#[test]
fn dense_windows_report_truncation_and_changed_transcripts_invalidate_evidence() {
    let (_temp, mut store, service) = setup();
    let before = store.project().unwrap();
    let old = discovery::evidence(
        &before,
        "project".as_ref(),
        &service.backend,
        2,
        &query(json!({"assetId":"source"})),
    )
    .unwrap();
    let old_sha = old["transcript"]["sha256"].as_str().unwrap();
    let cues = (0..25)
        .rev()
        .map(|index| Cue {
            id: format!("cue_{index}"),
            start_ms: index * 100,
            end_ms: index * 100 + 50,
            text: format!("Cue {index}"),
        })
        .collect();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "new-fixture".into(),
                cues,
            },
            2,
            "change-discovery-transcript",
        )
        .unwrap();
    let project = store.project().unwrap();
    let page = discovery::evidence(
        &project,
        "project".as_ref(),
        &service.backend,
        3,
        &query(json!({"assetId":"source","windowMs":30000})),
    )
    .unwrap();
    assert_eq!(page["items"][0]["cueExcerptsTotal"], 25);
    assert_eq!(page["items"][0]["cueExcerptsTruncated"], true);
    assert_eq!(
        page["items"][0]["cueExcerpts"].as_array().unwrap().len(),
        20
    );
    assert_eq!(page["items"][0]["cueExcerpts"][0]["id"], "cue_0");
    assert_ne!(page["transcript"]["sha256"], old_sha);
    let stale = brief(
        json!({"objective":"Find a moment","candidates":[{"id":"cut","outputId":"output","assetId":"source",
        "sourceSha256":"a".repeat(64),"startMs":9000,"endMs":12000,"title":"Idea","reason":"Candidate",
        "evidence":[{"kind":"transcript-cue","transcriptSha256":old_sha,"cueId":"intro"}]}]}),
    );
    assert!(
        discovery::prepare(&project, "project".as_ref(), &service.backend, 3, &stale)
            .unwrap_err()
            .to_string()
            .contains("transcript hash is stale")
    );
    assert_eq!(store.project().unwrap(), project);
}

#[test]
fn impossible_plan_keeps_the_source_preview_and_explains_the_failure() {
    let (_temp, store, service) = setup();
    let project = store.project().unwrap();
    let proposal = brief(
        json!({"objective":"Find a moment","candidates":[{"id":"cut","outputId":"seq_main","assetId":"source",
        "sourceSha256":"a".repeat(64),"startMs":9000,"endMs":12000,"title":"Idea","reason":"Candidate",
        "evidence":[{"kind":"observation","modality":"video","description":"The agent saw a product"}]}]}),
    );
    let result =
        discovery::prepare(&project, "project".as_ref(), &service.backend, 2, &proposal).unwrap();
    assert!(result["items"][0]["planRequest"].is_null());
    assert!(result["items"][0]["planningUnavailableReason"].is_string());
    assert!(result["items"][0]["previewRequest"].is_object());
    assert_eq!(
        result["items"][0]["evidence"][0]["verification"],
        "caller-asserted"
    );
    assert_eq!(store.project().unwrap(), project);
}

fn provider_query() -> ProviderSearch {
    serde_json::from_value(json!({"assetId":"source","sourceSha256":"a".repeat(64),
        "query":"Find the product","modality":"visual","startMs":0,"endMs":30000,"maxResults":2,"targetDurationMs":30000}))
    .unwrap()
}

fn provider_output(candidates: Value) -> Value {
    json!({"schemaVersion":1,"query":"Find the product","candidates":candidates})
}

fn provider_candidate() -> Value {
    json!({"id":"candidate-1","startMs":9000,"endMs":12000,"title":"Product moment",
        "reason":"The provider reports the product here","confidence":0.75,
        "evidence":[{"kind":"observation","modality":"video","description":"Provider identified an object"}]})
}

fn complete_provider_fixture(
    root: &std::path::Path,
    store: &Store,
    job_id: &str,
    output: Value,
) -> (std::path::PathBuf, Value) {
    let job = store.job(job_id).unwrap();
    let analysis = job.input.unwrap().analysis.unwrap();
    let analysis_key = "f".repeat(64);
    let folder = root.join("analysis").join(&analysis_key);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("result.json");
    let report = json!({"schemaVersion":1,"sourceSha256":"a".repeat(64),"task":analysis.task,
        "analysisKey":analysis_key,"data":{"schemaVersion":1,"kind":"analysis","sourceSha256":"a".repeat(64),"data":output},"files":[]});
    std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    let result = json!({"path":path,"sha256":media::hash_file(&path).unwrap(),"mimeType":"application/json",
        "analysisKey":analysis_key,"needsReview":true});
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), job_id],
    )
    .unwrap();
    (path, report)
}

#[test]
fn provider_search_is_explicit_source_bound_and_returns_reviewable_proposals() {
    let (temp, store, mut service) = setup();
    let root = temp.path().join("project");
    let project = store.project().unwrap();
    let provider = agent_video_workbench::analysis::Provider {
        program: temp.path().join("unexecuted-provider"),
        sha256: "c".repeat(64),
    };
    assert!(
        discovery::provider_search_request(
            &project,
            "project".as_ref(),
            &service.backend,
            None,
            2,
            &provider_query()
        )
        .is_err()
    );
    service.provider = Some(provider.clone());
    let search = discovery::provider_search_request(
        &project,
        "project".as_ref(),
        &service.backend,
        Some(&provider),
        2,
        &provider_query(),
    )
    .unwrap();
    assert_eq!(
        search["providerReceives"],
        "original source path, source metadata and search parameters"
    );
    assert!(!provider.program.exists());
    let request = search["analysisRequest"].clone();
    assert!(
        request["key"]
            .as_str()
            .unwrap()
            .starts_with(discovery::PROVIDER_KEY_PREFIX)
    );
    assert_eq!(request["task"]["task"], "moment-search-v1");
    let mut changed_query = request.clone();
    changed_query["task"]["parameters"]["query"] = json!("different query");
    assert!(
        service
            .execute(serde_json::from_value::<Request>(changed_query).unwrap())
            .unwrap_err()
            .to_string()
            .contains("generated search key")
    );
    let mut another_provider = provider.clone();
    another_provider.program = temp.path().join("other-provider");
    service.provider = Some(another_provider);
    assert!(
        service
            .execute(serde_json::from_value::<Request>(request.clone()).unwrap())
            .unwrap_err()
            .to_string()
            .contains("generated search key")
    );
    service.provider = Some(provider.clone());
    service.backend = media::backend("ffmpeg".into(), "other-ffprobe".into());
    assert!(
        service
            .execute(serde_json::from_value::<Request>(request.clone()).unwrap())
            .unwrap_err()
            .to_string()
            .contains("generated search key")
    );
    service.backend = media::backend("ffmpeg".into(), "ffprobe".into());
    let job = service
        .execute(serde_json::from_value::<Request>(request.clone()).unwrap())
        .unwrap();
    let repeated = service
        .execute(serde_json::from_value::<Request>(request).unwrap())
        .unwrap();
    assert_eq!(job["id"], repeated["id"]);
    let job_id = job["id"].as_str().unwrap();
    let input = store.job(job_id).unwrap().input.unwrap();
    let key = discovery::provider_input_key(&input, &project).unwrap();
    let mut different = input.clone();
    different.ffprobe = "different-backend".into();
    assert_ne!(
        key,
        discovery::provider_input_key(&different, &project).unwrap()
    );
    different = input.clone();
    different
        .analysis
        .as_mut()
        .unwrap()
        .provider
        .as_mut()
        .unwrap()
        .sha256 = "d".repeat(64);
    assert_ne!(
        key,
        discovery::provider_input_key(&different, &project).unwrap()
    );
    different = input;
    different.analysis.as_mut().unwrap().provider = None;
    assert!(discovery::provider_input_key(&different, &project).is_err());
    complete_provider_fixture(
        &root,
        &store,
        job_id,
        provider_output(json!([provider_candidate()])),
    );
    let report = discovery::provider_search_results(
        &root,
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        job_id,
        "short",
    )
    .unwrap();
    assert_eq!(report["interpretation"], "provider-supplied");
    assert_eq!(report["provider"]["programSha256"], "c".repeat(64));
    assert_eq!(report["provider"]["jobId"], job_id);
    assert_eq!(report["items"][0]["providerCandidateId"], "candidate-1");
    assert_eq!(report["provider"]["targetDurationMs"], 30000);
    assert_eq!(
        report["items"][0]["planningStatus"]["targetDurationDeltaMs"],
        -27000.0
    );
    assert_eq!(report["items"][0]["providerReportedConfidence"], 0.75);
    assert_eq!(
        report["items"][0]["evidence"][0]["verification"],
        "provider-asserted"
    );
    assert_eq!(report["items"][0]["planRequest"]["command"], "plan-edit");
    assert_eq!(
        report["items"][0]["previewRequest"]["command"],
        "analyze-start"
    );
    assert_eq!(store.project().unwrap(), project);
}

#[test]
fn provider_results_reject_tampering_wrong_query_ranges_and_never_infer_absence() {
    let (temp, store, mut service) = setup();
    let root = temp.path().join("project");
    let project = store.project().unwrap();
    let provider = agent_video_workbench::analysis::Provider {
        program: temp.path().join("provider"),
        sha256: "c".repeat(64),
    };
    service.provider = Some(provider.clone());
    let request = discovery::provider_search_request(
        &project,
        "project".as_ref(),
        &service.backend,
        Some(&provider),
        2,
        &provider_query(),
    )
    .unwrap();
    let job = service
        .execute(serde_json::from_value::<Request>(request["analysisRequest"].clone()).unwrap())
        .unwrap();
    let job_id = job["id"].as_str().unwrap();
    let (path, _) = complete_provider_fixture(&root, &store, job_id, provider_output(json!([])));
    let empty = discovery::provider_search_results(
        &root,
        &project,
        "project".as_ref(),
        &service.backend,
        2,
        job_id,
        "short",
    )
    .unwrap();
    assert_eq!(empty["status"], "no-proposals");
    assert!(
        empty["guidance"]
            .as_str()
            .unwrap()
            .contains("does not establish")
    );
    let original_result = std::fs::read(&path).unwrap();
    let redirected = root.join("cache").join("other-result.json");
    std::fs::write(&redirected, &original_result).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&redirected, &path).unwrap();
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "short"
        )
        .unwrap_err()
        .to_string()
        .contains("outside its project analysis directory")
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, &original_result).unwrap();
    std::fs::write(&path, b"tampered").unwrap();
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "short"
        )
        .unwrap_err()
        .to_string()
        .contains("bytes changed")
    );

    let mut wrong_query = provider_output(json!([provider_candidate()]));
    wrong_query["query"] = json!("another query");
    complete_provider_fixture(&root, &store, job_id, wrong_query);
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "short"
        )
        .is_err()
    );
    let mut out_of_scope = provider_candidate();
    out_of_scope["endMs"] = json!(30001);
    complete_provider_fixture(
        &root,
        &store,
        job_id,
        provider_output(json!([out_of_scope])),
    );
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "short"
        )
        .is_err()
    );
    complete_provider_fixture(
        &root,
        &store,
        job_id,
        provider_output(json!([provider_candidate(), provider_candidate()])),
    );
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "short"
        )
        .is_err()
    );
    complete_provider_fixture(
        &root,
        &store,
        job_id,
        provider_output(json!([provider_candidate()])),
    );
    assert!(
        discovery::provider_search_results(
            &root,
            &project,
            "project".as_ref(),
            &service.backend,
            2,
            job_id,
            "9bad"
        )
        .is_err()
    );
    assert_eq!(store.project().unwrap(), project);
}
