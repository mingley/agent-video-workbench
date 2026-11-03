use agent_video_workbench::{
    media,
    smart_reframe::{self, ReframeRequest},
    store::Store,
    studio,
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn fixture() -> (tempfile::TempDir, Store, Value) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Observed subject").unwrap();
    let bytes = b"subject fixture";
    let sha = format!("{:x}", Sha256::digest(bytes));
    let source = root.join("originals").join(&sha);
    std::fs::write(&source, bytes).unwrap();
    assert_eq!(media::hash_file(&source).unwrap(), sha);
    let project = store.project().unwrap();
    let time = |value| json!({"value":value,"rate":{"numerator":30,"denominator":1}});
    let operations = json!({"schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":0,
        "idempotencyKey":"reframe-fixture","operations":[
        {"id":"source","op":"asset.add","params":{"id":"source","uri":format!("originals/{sha}"),"kind":"video",
            "fingerprint":{"strategy":"sha256","sizeBytes":15,"sha256":sha},
            "metadata":{"status":"available","duration":time(90),"video":{
                "size":{"width":1920,"height":1080},"frameRate":{"numerator":30,"denominator":1},
                "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
                "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}}},
        {"id":"sequence","op":"sequence.add","params":{"id":"vertical","name":"Vertical","width":1080,"height":1920,"frameRate":{"numerator":30,"denominator":1}}},
        {"id":"track","op":"track.add","params":{"id":"vertical_video","sequence":"vertical","type":"video"}},
        {"id":"clip","op":"clip.add","params":{"id":"take","asset":"source","track":"vertical_video",
            "at":time(0),"sourceIn":time(0),"duration":time(60),"fit":"contain"}}
    ]});
    store
        .apply(
            &serde_json::from_value::<OperationBatch>(operations).unwrap(),
            false,
        )
        .unwrap();
    let request = json!({"schemaVersion":1,"itemId":"take","sourceSha256":sha,"observer":"reviewed visual provider v1",
    "safeInset":0.05,"minConfidence":0.7,"maxGapMs":1000,"samples":[
        {"sourceMs":0,"confidence":0.9,"regions":[{"x":0.20,"y":0.25,"width":0.10,"height":0.25}]},
        {"sourceMs":1000,"confidence":0.8,"regions":[{"x":0.60,"y":0.25,"width":0.10,"height":0.25}]}
    ]});
    (temp, store, request)
}

fn propose(
    temp: &tempfile::TempDir,
    store: &Store,
    value: Value,
) -> agent_video_workbench::Result<Value> {
    let request: ReframeRequest = serde_json::from_value(value).unwrap();
    smart_reframe::propose(
        &temp.path().join("project"),
        &store.project().unwrap(),
        1,
        &request,
    )
}

#[test]
fn observed_subjects_produce_an_editable_reviewable_crop_path() {
    let (temp, mut store, request) = fixture();
    let before = store.project().unwrap();
    let result = propose(&temp, &store, request).unwrap();
    assert_eq!(result["status"], "review-required");
    assert_eq!(result["samples"].as_array().unwrap().len(), 2);
    assert!(
        result["samples"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["status"] == "fits")
    );
    assert_eq!(result["proposal"]["algorithm"], "avw.observed-subjects.v1");
    assert_eq!(store.project().unwrap(), before);
    let edit: studio::Edit = serde_json::from_value(result["edit"].clone()).unwrap();
    store
        .studio(&edit, 1, "accept-reviewed-crop", false)
        .unwrap();
    let updated = store.project().unwrap();
    assert_eq!(updated.revision, 2);
    assert!(
        updated
            .find_item("take")
            .unwrap()
            .2
            .extensions
            .contains_key("avw.reframe.v1")
    );
}

#[test]
fn missing_low_confidence_wide_or_sparse_observations_never_yield_an_edit() {
    let (temp, store, request) = fixture();
    for mode in ["missing", "low", "wide", "gap"] {
        let mut value = request.clone();
        match mode {
            "missing" => value["samples"][1]["regions"] = json!([]),
            "low" => value["samples"][1]["confidence"] = json!(0.2),
            "wide" => {
                value["samples"][1]["regions"] = json!([
                {"x":0.05,"y":0.25,"width":0.1,"height":0.25},
                {"x":0.85,"y":0.25,"width":0.1,"height":0.25}])
            }
            "gap" => value["maxGapMs"] = json!(500),
            _ => unreachable!(),
        }
        let result = propose(&temp, &store, value).unwrap();
        assert_eq!(result["status"], "needs-review", "{mode}");
        assert!(result["edit"].is_null(), "{mode}");
        assert!(!result["issues"].as_array().unwrap().is_empty(), "{mode}");
    }
    assert_eq!(store.project().unwrap().revision, 1);
}

#[test]
fn validates_source_revision_time_order_and_original_bytes() {
    let (temp, store, request) = fixture();
    let project = store.project().unwrap();
    let typed: ReframeRequest = serde_json::from_value(request.clone()).unwrap();
    assert!(smart_reframe::propose(&temp.path().join("project"), &project, 0, &typed).is_err());
    let mut wrong = request.clone();
    wrong["sourceSha256"] = json!("0".repeat(64));
    assert!(propose(&temp, &store, wrong).is_err());
    let mut bad = request.clone();
    bad["samples"][1]["sourceMs"] = json!(0);
    assert!(propose(&temp, &store, bad).is_err());
    let mut out = request.clone();
    out["samples"][1]["regions"][0]["x"] = json!(1.0);
    assert!(propose(&temp, &store, out).is_err());
    let original = temp
        .path()
        .join("project/originals")
        .join(request["sourceSha256"].as_str().unwrap());
    std::fs::write(original, b"changed").unwrap();
    assert!(propose(&temp, &store, request).is_err());
}

#[test]
fn rejects_existing_geometry_or_effects_that_invalidate_subject_fit() {
    let (temp, store, request) = fixture();
    let typed: ReframeRequest = serde_json::from_value(request).unwrap();
    for mode in [
        "position",
        "scale",
        "rotation",
        "crop",
        "keyframes",
        "effect",
        "pixel-aspect",
        "solo",
    ] {
        let mut project = store.project().unwrap();
        match mode {
            "position" | "scale" | "rotation" | "crop" => {
                let item = project.find_item_mut("take").unwrap();
                let agentcut_core::ItemPayload::Clip(clip) = &mut item.payload else {
                    unreachable!()
                };
                match mode {
                    "position" => clip.transform.position.x += 10.0,
                    "scale" => clip.transform.scale.x = 1.2,
                    "rotation" => clip.transform.rotation_degrees = 10.0,
                    "crop" => clip.video.crop.left = 0.1,
                    _ => unreachable!(),
                }
            }
            "keyframes" => project.find_item_mut("take").unwrap().keyframes.push(
                agentcut_core::KeyframeChannel::new(
                    "motion",
                    "transform.position.x",
                    agentcut_core::ValueType::Number,
                ),
            ),
            "effect" => project
                .find_item_mut("take")
                .unwrap()
                .effects
                .push(agentcut_core::Effect::new("effect", "video.color.basic", 1)),
            "pixel-aspect" => {
                project
                    .require_sequence_mut("vertical")
                    .unwrap()
                    .canvas
                    .pixel_aspect_ratio
                    .numerator = 2
            }
            "solo" => {
                let sequence = project.require_sequence_mut("vertical").unwrap();
                let mut other = agentcut_core::Track::empty(
                    "other",
                    "Other",
                    agentcut_core::TrackType::Video,
                    2,
                );
                other.solo = true;
                sequence.tracks.push(other);
            }
            _ => unreachable!(),
        }
        assert!(
            smart_reframe::propose(&temp.path().join("project"), &project, 1, &typed).is_err(),
            "{mode}"
        );
    }
}
