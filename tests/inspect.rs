use agent_video_workbench::{inspect, media, store::Store};
use agentcut_core::OperationBatch;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn shared_source_evidence_keeps_the_requested_asset_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Shared source evidence").unwrap();
    let source = b"source footage";
    let other = b"different source footage";
    let source_hash = format!("{:x}", Sha256::digest(source));
    let other_hash = format!("{:x}", Sha256::digest(other));
    fs::write(root.join("originals").join(&source_hash), source).unwrap();
    fs::write(root.join("originals").join(&other_hash), other).unwrap();

    let asset = |id: &str, hash: &str, size: usize| {
        json!({"id":format!("add_{id}"),"op":"asset.add","params":{
            "id":id,"uri":format!("originals/{hash}"),"kind":"video",
            "fingerprint":{"strategy":"sha256","sizeBytes":size,"sha256":hash},
            "metadata":{"status":"available","duration":{"value":10,"rate":{"numerator":1,"denominator":1}},
                "video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},
                    "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
                    "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}
        }})
    };
    let project = store.project().unwrap();
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":project.project_id,"baseRevision":0,"idempotencyKey":"add-shared-sources",
        "operations":[asset("source_a",&source_hash,source.len()),asset("source_b",&source_hash,source.len()),
                      asset("source_c",&other_hash,other.len())]})).unwrap();
    store.apply(&batch, false).unwrap();

    // The process fixture exercises actual cache files and source verification
    // without making the Rust suite depend on a host FFmpeg installation.
    let ffmpeg = tmp.path().join("ffmpeg-fixture");
    fs::write(
        &ffmpeg,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo ffmpeg-fixture; exit 0; fi\n\
         case \" $* \" in\n  *' -frames:v '*) for last do :; done; printf image > \"$last\" ;;\n\
         *) echo 'silence_start: 0.4' >&2; echo 'silence_end: 1.9' >&2; \
            echo 'silence_start: -0.2' >&2; echo 'pts_time: 0.5' >&2; \
            echo 'lavfi.scene_score=0.8' >&2; echo 'pts_time: 1.1' >&2; \
            echo 'lavfi.scene_score=0.9' >&2 ;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
    let backend = media::backend(ffmpeg, "unused".into());

    for kind in [
        inspect::Kind::Frame,
        inspect::Kind::Silence,
        inspect::Kind::Scenes,
    ] {
        let first = inspect::inspect(&root, "source_a", kind, 0, Some(1000), &backend).unwrap();
        let alias = inspect::inspect(&root, "source_b", kind, 0, Some(1000), &backend).unwrap();
        let repeated = inspect::inspect(&root, "source_a", kind, 0, Some(1000), &backend).unwrap();
        assert_eq!(first["assetId"], "source_a");
        assert_eq!(first["cacheHit"], false);
        assert_eq!(alias["assetId"], "source_b");
        assert_eq!(alias["cacheHit"], matches!(kind, inspect::Kind::Frame));
        assert_eq!(alias["sourceHash"], first["sourceHash"]);
        assert_eq!(repeated["assetId"], "source_a");
        assert_eq!(repeated["cacheHit"], matches!(kind, inspect::Kind::Frame));
        if matches!(kind, inspect::Kind::Frame) {
            assert_eq!(alias["path"], first["path"]);
            assert_eq!(alias["sha256"], first["sha256"]);
        } else {
            assert_eq!(alias["evidence"], first["evidence"]);
        }

        let different_time =
            inspect::inspect(&root, "source_b", kind, 1000, Some(2000), &backend).unwrap();
        let different_source =
            inspect::inspect(&root, "source_c", kind, 0, Some(1000), &backend).unwrap();
        assert_eq!(different_time["cacheHit"], false);
        assert_eq!(different_source["cacheHit"], false);
        assert_eq!(different_source["assetId"], "source_c");
        assert_ne!(different_source["sourceHash"], first["sourceHash"]);
    }
    // A v2 cache can contain observations after the requested end. It must
    // never be reused by the bounded detector or reported as current evidence.
    let legacy_key = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!({
                "source":source_hash,"kind":"silence","startMs":3000,"endMs":4000,
                "tool":"ffmpeg-fixture\n","version":2
            }))
            .unwrap()
        )
    );
    fs::write(
        root.join("analysis").join(format!("{legacy_key}.json")),
        serde_json::to_vec(
            &json!({"assetId":"wrong","evidence":[{"sourceTimeMs":9000}],"cacheHit":false}),
        )
        .unwrap(),
    )
    .unwrap();
    let bounded = inspect::inspect(
        &root,
        "source_a",
        inspect::Kind::Silence,
        3000,
        Some(4000),
        &backend,
    )
    .unwrap();
    assert_eq!(bounded["cacheHit"], false);
    assert_eq!(bounded["timing"]["mapping"], "unknown");
    assert_eq!(bounded["coverage"]["leadingCoverage"], "not-measured");
    assert_eq!(bounded["coverage"]["trailingCoverage"], "not-measured");
    assert_eq!(bounded["analysisStatus"], "unavailable");
    assert_eq!(
        bounded["unavailableReason"],
        "source-timestamp-mapping-unavailable"
    );
    assert!(bounded["evidence"].as_array().unwrap().is_empty());
    assert_eq!(
        inspect::inspect(
            &root,
            "source_a",
            inspect::Kind::Silence,
            3000,
            Some(4000),
            &backend
        )
        .unwrap()["cacheHit"],
        false
    );
    let scenes = inspect::inspect(
        &root,
        "source_a",
        inspect::Kind::Scenes,
        3000,
        Some(4000),
        &backend,
    )
    .unwrap();
    assert_eq!(scenes["analysisStatus"], "unavailable");
    assert!(scenes["evidence"].as_array().unwrap().is_empty());
    assert_eq!(store.project().unwrap().revision, 1);
}
