use agent_video_workbench::{
    store::Store,
    studio::Edit,
    translation::{self, CaptionTranslation, TranslationProposal, TranslationProvenance},
};
use agentcut_core::{ItemPayload, OperationBatch};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// A generated metric-only font fixture supports the variant dry-run. Real
// caption rendering and glyph coverage are exercised by media evaluations.
fn metric_fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 220];
    let write16 = |bytes: &mut [u8], at: usize, value: u16| {
        bytes[at..at + 2].copy_from_slice(&value.to_be_bytes())
    };
    let write32 = |bytes: &mut [u8], at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_be_bytes())
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
    write16(&mut bytes, 98, 1000);
    write16(&mut bytes, 116, 800);
    write16(&mut bytes, 118, (-200_i16) as u16);
    write16(&mut bytes, 146, 1);
    write16(&mut bytes, 152, 500);
    write16(&mut bytes, 162, 1);
    write16(&mut bytes, 164, 3);
    write16(&mut bytes, 166, 10);
    write32(&mut bytes, 168, 12);
    write16(&mut bytes, 172, 12);
    bytes
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    store: Store,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Translation proposals").unwrap();
    let font = metric_fixture();
    let hash = format!("{:x}", Sha256::digest(&font));
    std::fs::write(root.join("originals").join(&hash), &font).unwrap();
    let project = store.project().unwrap();
    let time = |value| json!({"value":value,"rate":{"numerator":30,"denominator":1}});
    let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":project.project_id,
        "baseRevision":0,"idempotencyKey":"translation-fixture","operations":[
            {"id":"font","op":"asset.add","params":{"id":"font","uri":format!("originals/{hash}"),
                "kind":"font","fingerprint":{"strategy":"sha256","sizeBytes":font.len(),"sha256":hash}}},
            {"id":"visible-track","op":"track.add","params":{"id":"visible","sequence":"seq_main","type":"caption"}},
            {"id":"hidden-track","op":"track.add","params":{"id":"hidden","sequence":"seq_main","type":"caption"}},
            {"id":"caption-one","op":"caption.add","params":{"id":"c1","track":"visible","at":time(0),"duration":time(30),"text":"Hello world"}},
            {"id":"caption-two","op":"caption.add","params":{"id":"c2","track":"hidden","at":time(30),"duration":time(30),"text":"Hidden text"}},
            {"id":"font-one","op":"item.set","target":"c1","params":{"property":"text.style.fontAssetId","value":"font"}},
            {"id":"font-two","op":"item.set","target":"c2","params":{"property":"text.style.fontAssetId","value":"font"}},
            {"id":"hide","op":"track.set","target":"hidden","params":{"enabled":false}}
        ]})).unwrap();
    store.apply(&batch, false).unwrap();
    Fixture {
        _temp: temp,
        root,
        store,
    }
}

fn source_context(f: &Fixture) -> Value {
    translation::context(&f.store.project().unwrap(), "seq_main", 1, 0, 100).unwrap()
}

fn proposal(f: &Fixture) -> TranslationProposal {
    let context = source_context(f);
    let rows = context["items"].as_array().unwrap();
    TranslationProposal {
        schema_version: 1,
        project_id: context["projectId"].as_str().unwrap().into(),
        source_revision: 1,
        sequence: "seq_main".into(),
        sequence_sha256: context["sequenceSha256"].as_str().unwrap().into(),
        output_id: "spanish".into(),
        output_name: "Spanish captions".into(),
        target_language: "es".into(),
        provider: "manual-review".into(),
        provider_version: "1".into(),
        translations: rows
            .iter()
            .map(|row| CaptionTranslation {
                caption_id: row["captionId"].as_str().unwrap().into(),
                source_text_sha256: row["textSha256"].as_str().unwrap().into(),
                text: if row["captionId"] == "c1" {
                    "Hola mundo"
                } else {
                    "Texto oculto"
                }
                .into(),
            })
            .collect(),
    }
}

#[test]
fn complete_proposal_is_dry_run_then_atomically_applied_with_provenance() {
    let mut f = fixture();
    let before = f.store.project().unwrap();
    let history = f.store.history().unwrap();
    let context = source_context(&f);
    assert_eq!(context["sourceRevision"], 1);
    assert_eq!(context["captionCount"], 2);
    assert_eq!(context["visibleCaptionCount"], 1);
    assert_eq!(context["items"][0]["text"], "Hello world");
    assert_eq!(context["items"][0]["visible"], true);
    assert_eq!(context["items"][1]["visible"], false);
    assert!(context["items"][0]["sourceCue"].is_null());
    let first = translation::context(&before, "seq_main", 1, 0, 1).unwrap();
    assert_eq!(first["nextOffset"], 1);
    let second = translation::context(&before, "seq_main", 1, 1, 1).unwrap();
    assert!(second["nextOffset"].is_null());
    assert!(translation::context(&before, "seq_main", 0, 1, 1).is_err());
    assert!(translation::context(&before, "seq_main", 1, 0, 101).is_err());
    let proposed = proposal(&f);
    let report = translation::plan(
        &f.root,
        std::path::Path::new("project"),
        &proposed,
        1,
        "translate-es",
    )
    .unwrap();
    assert_eq!(report["status"], "ready");
    assert_eq!(report["captionCount"], 2);
    assert_eq!(report["visibleCaptionCount"], 1);
    assert_eq!(report["providerProvenanceIncluded"], true);
    assert_eq!(report["proposal"], serde_json::to_value(&proposed).unwrap());
    assert_eq!(report["variantDryRun"]["revision"], 2);
    assert_eq!(report["studioRequest"]["dryRun"], true);
    assert_eq!(f.store.project().unwrap(), before);
    assert_eq!(f.store.history().unwrap(), history);
    let edit: Edit = serde_json::from_value(report["studioRequest"]["edit"].clone()).unwrap();
    let applied = f.store.studio(&edit, 1, "translate-es", false).unwrap();
    assert_eq!(applied.revision, 2);
    let after = f.store.project().unwrap();
    assert_eq!(
        after.require_sequence("seq_main").unwrap(),
        before.require_sequence("seq_main").unwrap()
    );
    let variant = after.require_sequence("spanish").unwrap();
    let texts: Vec<_> = variant
        .tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter_map(|i| match &i.payload {
            ItemPayload::Caption(c) => Some(c.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Hola mundo", "Texto oculto"]);
    assert_eq!(variant.extensions["avw.variant"]["sourceRevision"], 1);
    assert_eq!(variant.extensions["avw.variant"]["language"], "es");
    assert_eq!(
        variant.extensions["avw.translationProvenance"]["proposal"]["provider"],
        "manual-review"
    );
    assert_eq!(
        variant.extensions["avw.translationProvenance"]["proposalSha256"],
        report["proposalSha256"]
    );
    assert_eq!(
        f.store.studio(&edit, 1, "translate-es", false).unwrap(),
        applied
    );
}

#[test]
fn invalid_missing_hidden_stale_and_forged_translations_are_refused_atomically() {
    let mut f = fixture();
    let before = f.store.project().unwrap();
    let good = proposal(&f);
    let mut bad = Vec::new();
    let mut missing = good.clone();
    missing.translations.pop();
    bad.push(missing);
    let mut duplicate = good.clone();
    duplicate
        .translations
        .push(duplicate.translations[0].clone());
    bad.push(duplicate);
    let mut unknown = good.clone();
    unknown.translations[0].caption_id = "unknown".into();
    bad.push(unknown);
    let mut stale_text = good.clone();
    stale_text.translations[0].source_text_sha256 = "0".repeat(64);
    bad.push(stale_text);
    let mut stale_seq = good.clone();
    stale_seq.sequence_sha256 = "1".repeat(64);
    bad.push(stale_seq);
    let mut wrong_project = good.clone();
    wrong_project.project_id = "other".into();
    bad.push(wrong_project);
    let mut empty = good.clone();
    empty.translations[0].text = "  ".into();
    bad.push(empty);
    let mut bad_lang = good.clone();
    bad_lang.target_language = "en_US".into();
    bad.push(bad_lang);
    let mut empty_provider = good.clone();
    empty_provider.provider = "  ".into();
    bad.push(empty_provider);
    let mut oversized = good.clone();
    oversized.translations[0].text = "a".repeat(2001);
    bad.push(oversized);
    for (index, p) in bad.iter().enumerate() {
        assert!(
            translation::plan(
                &f.root,
                std::path::Path::new("project"),
                p,
                1,
                "translate-es"
            )
            .is_err(),
            "invalid case {index}"
        );
        assert_eq!(f.store.project().unwrap(), before);
    }
    assert!(
        translation::plan(&f.root, std::path::Path::new("project"), &good, 1, "short").is_err()
    );
    let report = translation::plan(
        &f.root,
        std::path::Path::new("project"),
        &good,
        1,
        "translate-es",
    )
    .unwrap();
    let mut wrong_caption = report["studioRequest"]["edit"].clone();
    wrong_caption["captions"]["c1"] = json!("Tampered");
    let edit: Edit = serde_json::from_value(wrong_caption).unwrap();
    assert!(
        f.store
            .studio(&edit, 1, "translate-es", false)
            .unwrap_err()
            .to_string()
            .contains("provenance")
    );
    let mut wrong_proposal = report["studioRequest"]["edit"].clone();
    wrong_proposal["translationProvenance"]["proposal"]["provider"] = json!("other");
    let edit: Edit = serde_json::from_value(wrong_proposal).unwrap();
    assert!(
        f.store
            .studio(&edit, 1, "translate-es", false)
            .unwrap_err()
            .to_string()
            .contains("digest")
    );
    assert_eq!(f.store.project().unwrap(), before);
    let valid: Edit = serde_json::from_value(report["studioRequest"]["edit"].clone()).unwrap();
    assert!(f.store.studio(&valid, 1, "translate-es", false).is_ok());
}

#[test]
fn provenance_is_bound_to_the_source_snapshot_and_exact_variant() {
    let f = fixture();
    let before = f.store.project().unwrap();
    let p = proposal(&f);
    let report = translation::plan(
        &f.root,
        std::path::Path::new("project"),
        &p,
        1,
        "translate-es",
    )
    .unwrap();
    let edit: Edit = serde_json::from_value(report["studioRequest"]["edit"].clone()).unwrap();
    let provenance: TranslationProvenance =
        serde_json::from_value(report["studioRequest"]["edit"]["translationProvenance"].clone())
            .unwrap();
    translation::validate_variant_provenance(&before, &edit, &provenance).unwrap();
    let mut changed = before.clone();
    let caption = changed.find_item_mut("c1").unwrap();
    if let ItemPayload::Caption(text) = &mut caption.payload {
        text.text = "Later correction".into();
    }
    assert!(
        translation::validate_variant_provenance(&changed, &edit, &provenance)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn legacy_variant_serialization_and_idempotent_replay_stay_compatible() {
    let mut f = fixture();
    let legacy = json!({"action":"variant","sourceRevision":1,"sequence":"seq_main","id":"legacy",
        "name":"Legacy","width":360,"height":640,"language":null,"captions":{}});
    let edit: Edit = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&edit).unwrap(), legacy);
    let outcome = f.store.studio(&edit, 1, "legacy-variant", false).unwrap();
    assert_eq!(
        f.store.studio(&edit, 1, "legacy-variant", false).unwrap(),
        outcome
    );
    assert!(
        !f.store
            .project()
            .unwrap()
            .require_sequence("legacy")
            .unwrap()
            .extensions
            .contains_key("avw.translationProvenance")
    );
}
