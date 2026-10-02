use agent_video_workbench::policy::{self, ProtectedRange};
use agentcut_core::{
    Asset, AssetKind, ClipData, Extensions, Fingerprint, ItemPayload, MediaMetadata, Project,
    RationalRate, RationalTime, TimeRange, TimelineItem,
};

fn time(value: i64, rate: u32) -> RationalTime {
    RationalTime::new(value, RationalRate::frames(rate).unwrap())
}

fn fixture() -> Project {
    let mut p = Project::new("protected", 360, 640, RationalRate::frames(30).unwrap());
    p.assets.push(Asset {
        id: "source".into(),
        kind: AssetKind::Video,
        uri: "originals/hash".into(),
        proxy_uri: None,
        label: "source".into(),
        fingerprint: Fingerprint::unprobed(),
        metadata: MediaMetadata::unprobed(),
        tags: Vec::new(),
        extensions: Extensions::new(),
    });
    for (id, start, duration) in [("b", 45, 15), ("a", 30, 15)] {
        p.sequences[0].tracks[0].items.push(TimelineItem {
            id: id.into(),
            name: id.into(),
            enabled: true,
            start: time(0, 30),
            duration: time(duration, 30),
            linked_group_id: None,
            payload: ItemPayload::Clip(ClipData::new(
                "source",
                TimeRange::new(time(start, 30), time(duration, 30)).unwrap(),
            )),
            effects: Vec::new(),
            keyframes: Vec::new(),
            extensions: Extensions::new(),
        });
    }
    p
}

#[test]
fn exact_cross_rate_union_accepts_reordering_but_rejects_source_gap() {
    let mut p = fixture();
    policy::add(
        &mut p,
        ProtectedRange {
            id: "demo".into(),
            asset_id: "source".into(),
            sequence_id: "seq_main".into(),
            start: time(48000, 48000),
            end: time(96000, 48000),
        },
    )
    .unwrap();
    let mut reordered = p.clone();
    reordered.sequences[0].tracks[0].items.reverse();
    policy::validate(&p, &reordered).unwrap();
    if let ItemPayload::Clip(clip) = &mut reordered.sequences[0].tracks[0].items[0].payload {
        clip.source_range.duration = time(14, 30);
    }
    assert!(policy::validate(&p, &reordered).is_err());
    let mut removed = p.clone();
    removed.extensions.clear();
    assert!(policy::validate(&p, &removed).is_err());
    let mut muted = p.clone();
    muted.sequences[0].tracks[0].enabled = false;
    assert!(policy::validate(&p, &muted).is_err());
}
