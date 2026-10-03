//! OpenTimelineIO cut/track subset with explicit losses and exact native times.
use crate::{
    Error, Result,
    store::{Outcome, Store},
    studio, workflow,
};
use agentcut_core::{
    AssetKind, ClipData, ItemPayload, Project, RationalRate, RationalTime, Sequence, TimeRange,
    TimelineItem, Track, TrackType,
};
use serde_json::{Value, json};

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn otio_time(t: RationalTime) -> Value {
    json!({"OTIO_SCHEMA":"RationalTime.1","value":t.value,"rate":t.rate.as_f64()})
}
fn range(start: RationalTime, duration: RationalTime) -> Value {
    json!({"OTIO_SCHEMA":"TimeRange.1","start_time":otio_time(start),"duration":otio_time(duration)})
}
pub fn export(project: &Project, sequence: &str) -> Result<Value> {
    let seq = project.require_sequence(sequence)?;
    let mut tracks = Vec::new();
    let mut losses = Vec::new();
    for track in &seq.tracks {
        if !matches!(track.track_type, TrackType::Video | TrackType::Audio) {
            for item in &track.items {
                losses.push(json!({"entityId":item.id,"feature":"caption/text/graphics","policy":"omitted; timing gaps preserved in other tracks"}));
            }
            continue;
        }
        let mut items: Vec<_> = track.items.iter().filter(|i| i.enabled).collect();
        items.sort_by(|a, b| {
            a.start
                .checked_cmp(b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut cursor = RationalTime::zero(seq.frame_rate);
        let mut children = Vec::new();
        for item in items {
            let Some(clip) = item.payload.as_clip() else {
                losses.push(
                    json!({"entityId":item.id,"feature":"non-clip payload","policy":"omitted"}),
                );
                continue;
            };
            if !clip.video.speed.is_normal() {
                return Err(invalid(
                    "OTIO subset requires normal-speed clips; speed/freeze cannot be silently flattened",
                ));
            }
            if item.start.checked_cmp(cursor)?.is_lt() {
                return Err(invalid(
                    "OTIO subset requires non-overlapping items within each track",
                ));
            }
            if item.start.checked_cmp(cursor)?.is_gt() {
                children.push(json!({"OTIO_SCHEMA":"Gap.1","name":"Timeline gap","source_range":range(RationalTime::zero(seq.frame_rate),item.start.checked_sub(cursor)?),"metadata":{}}));
            }
            let asset = project.require_asset(&clip.asset_id)?;
            if !matches!(asset.kind, AssetKind::Video | AssetKind::Audio) {
                return Err(invalid("OTIO cut subset requires video/audio media"));
            }
            if !item.effects.is_empty()
                || !item.keyframes.is_empty()
                || !clip.video.crop.is_none()
                || clip.transform != agentcut_core::Transform2d::identity()
                || clip.audio.gain_db != 0.0
            {
                losses.push(json!({"entityId":item.id,"feature":"effects, animation, crop, transform and audio finishing","policy":"omitted; originals and timing preserved"}));
            }
            children.push(json!({"OTIO_SCHEMA":"Clip.1","name":item.name,"source_range":range(clip.source_range.start,clip.source_range.duration),"media_reference":{"OTIO_SCHEMA":"ExternalReference.1","target_url":asset.uri,"available_range":asset.metadata.duration.map(|d|range(RationalTime::zero(d.rate),d)),"metadata":{"avwSha256":asset.fingerprint.sha256}},"effects":[],"markers":[],"metadata":{"avwExactSourceRange":clip.source_range,"avwExactTimelineDuration":item.duration,"avwAudioEnabled":clip.audio.enabled,"avwItemId":item.id}}));
            cursor = item.end_exclusive()?;
        }
        tracks.push(json!({"OTIO_SCHEMA":"Track.1","name":track.name,"kind":if track.track_type==TrackType::Audio {"Audio"}else{"Video"},"children":children,"source_range":null,"effects":[],"markers":[],"metadata":{"avwEnabled":track.enabled,"avwMuted":track.muted,"avwSolo":track.solo}}));
    }
    if !seq.transitions.is_empty() || !seq.buses.is_empty() {
        losses.push(
            json!({"entityId":sequence,"feature":"transitions/audio buses","policy":"omitted"}),
        );
    }
    Ok(
        json!({"document":{"OTIO_SCHEMA":"Timeline.1","name":seq.name,"global_start_time":otio_time(RationalTime::zero(seq.frame_rate)),"tracks":{"OTIO_SCHEMA":"Stack.1","name":"Tracks","children":tracks,"effects":[],"markers":[],"source_range":null,"metadata":{}},"metadata":{"avwInterchangeVersion":1,"avwCanvas":seq.canvas,"avwFrameRate":seq.frame_rate,"avwMarkers":seq.markers,"avwSourceRevision":project.revision,"avwLossReport":losses}},"lossReport":losses,"supportedSubset":"OpenTimelineIO Timeline.1/Track.1/Clip.1/Gap.1; normal-speed cuts, tracks, exact source bindings and marker metadata"}),
    )
}
fn read_time(value: &Value) -> Result<RationalTime> {
    let frames = value["value"]
        .as_i64()
        .ok_or_else(|| invalid("OTIO time value must be an exact integer"))?;
    let rate = value["rate"]
        .as_f64()
        .filter(|r| r.is_finite() && *r > 0.0 && *r <= 1000.0)
        .ok_or_else(|| invalid("OTIO time rate invalid"))?;
    let scaled = (rate * 1_000_000.0).round() as u32;
    let divisor = gcd(scaled, 1_000_000);
    Ok(RationalTime::new(
        frames,
        RationalRate {
            numerator: scaled / divisor,
            denominator: 1_000_000 / divisor,
        },
    ))
}
fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Store {
    pub fn interchange_import(
        &mut self,
        document: &Value,
        id: &str,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        workflow::id(id)?;
        self.change(key,expected,json!({"kind":"interchange.import","document":document,"sequenceId":id,"expectedRevision":expected}),dry_run,|project|{
            if document["OTIO_SCHEMA"]!="Timeline.1" || document["tracks"]["OTIO_SCHEMA"]!="Stack.1"{return Err(invalid("expected supported OTIO Timeline.1 and Stack.1"));}
            let rate=if let Some(value)=document["metadata"].get("avwFrameRate"){serde_json::from_value(value.clone())?}else{read_time(&document["global_start_time"])?.rate};
            let canvas=if let Some(value)=document["metadata"].get("avwCanvas"){serde_json::from_value::<agentcut_core::Canvas>(value.clone())?}else{project.require_sequence("seq_main")?.canvas};
            if canvas.width>4096 || canvas.height>4096{return Err(invalid("OTIO canvas exceeds worker limit"));}
            let mut seq=Sequence::new(id,document["name"].as_str().unwrap_or(id),canvas.width,canvas.height,rate,48_000);seq.tracks.clear();seq.canvas=canvas;
            if let Some(markers)=document["metadata"].get("avwMarkers"){seq.markers=serde_json::from_value(markers.clone())?;for marker in &mut seq.markers{marker.id=format!("{id}_{}",marker.id);}}
            let tracks=document["tracks"]["children"].as_array().ok_or_else(||invalid("OTIO tracks missing"))?;if tracks.len()>100{return Err(invalid("OTIO exceeds 100 tracks"));}
            for (n,track) in tracks.iter().enumerate(){if track["OTIO_SCHEMA"]!="Track.1"{return Err(invalid("unsupported OTIO stack child"));}let kind=match track["kind"].as_str(){Some("Video")=>TrackType::Video,Some("Audio")=>TrackType::Audio,_=>return Err(invalid("unsupported OTIO track kind"))};let mut out=Track::empty(format!("{id}_track{n}"),track["name"].as_str().unwrap_or("Track"),kind,n as u32);out.enabled=track["metadata"]["avwEnabled"].as_bool().unwrap_or(true);out.muted=track["metadata"]["avwMuted"].as_bool().unwrap_or(false);out.solo=track["metadata"]["avwSolo"].as_bool().unwrap_or(false);let mut at=RationalTime::zero(rate);
                let items=track["children"].as_array().ok_or_else(||invalid("OTIO track children missing"))?;if items.len()>5000{return Err(invalid("OTIO exceeds 5000 items per track"));}
                for (i,item) in items.iter().enumerate(){let duration=read_time(&item["source_range"]["duration"])?;
                    match item["OTIO_SCHEMA"].as_str(){Some("Gap.1")=>{at=at.checked_add(duration)?;continue;},Some("Clip.1")=>{},_=>return Err(invalid("unsupported OTIO item; no silent import loss"))}
                    if item["effects"].as_array().is_some_and(|e|!e.is_empty()) || item["markers"].as_array().is_some_and(|e|!e.is_empty()){return Err(invalid("OTIO clip effects/markers are outside the import subset"));}
                    let sha=item["media_reference"]["metadata"]["avwSha256"].as_str().ok_or_else(||invalid("OTIO media requires avwSha256 for exact local relinking"))?;
                    let asset=project.assets.iter().find(|a|a.fingerprint.sha256.as_deref()==Some(sha)).ok_or_else(||invalid("OTIO media is missing; import/relink its original hash first"))?;
                    let source=if let Some(value)=item["metadata"].get("avwExactSourceRange"){serde_json::from_value(value.clone())?}else{TimeRange::new(read_time(&item["source_range"]["start_time"])?,duration)?};
                    let output_duration=if let Some(value)=item["metadata"].get("avwExactTimelineDuration"){serde_json::from_value(value.clone())?}else{duration};
                    let mut clip=ClipData::new(&asset.id,source);clip.audio.enabled=item["metadata"]["avwAudioEnabled"].as_bool().unwrap_or(true);
                    out.items.push(TimelineItem{id:format!("{id}_clip{n}_{i}"),name:item["name"].as_str().unwrap_or("Clip").into(),enabled:true,start:at,duration:output_duration,linked_group_id:None,payload:ItemPayload::Clip(clip),effects:Vec::new(),keyframes:Vec::new(),extensions:Default::default()});at=at.checked_add(output_duration)?;
                }seq.tracks.push(out);
            }
            seq.extensions.insert("avw.interchange".into(),json!({"format":"OpenTimelineIO cut subset","lossReport":document["metadata"]["avwLossReport"]}));let mut next=project.clone();next.sequences.push(seq);studio::validate(&next)?;Ok(next)
        })
    }
}
