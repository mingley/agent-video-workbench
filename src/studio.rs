//! Durable creator decisions, expanded and validated within one revision.
use crate::{
    Error, Result, media,
    store::{Outcome, Store},
    workflow,
};
use agentcut_core::{ItemPayload, OperationBatch, Project, RationalTime, Sequence, TimelineItem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub version: u32,
    pub font_asset_id: String,
    pub font_size: u32,
    pub color: String,
    #[serde(default = "cover")]
    pub fit: String,
    #[serde(default)]
    pub loudness_lufs: Option<f64>,
    #[serde(default)]
    pub true_peak_db: Option<f64>,
}
fn cover() -> String {
    "cover".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub version: u32,
    pub name: String,
    pub profile_id: String,
    pub profile_version: u32,
    pub slots: Vec<Slot>,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub min_duration_ms: i64,
    pub max_duration_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Edit {
    AudioDuck {
        sequence: String,
        settings: crate::audio::Ducking,
    },
    ProfilePut {
        profile: Profile,
    },
    ProfileApply {
        sequence: String,
        profile_id: String,
        version: u32,
    },
    TemplatePut {
        template: Template,
    },
    TemplateInstantiate {
        template_id: String,
        version: u32,
        output_id: String,
        name: String,
        bindings: Vec<workflow::Cut>,
    },
    Variant {
        source_revision: u64,
        sequence: String,
        id: String,
        name: String,
        width: u32,
        height: u32,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        captions: BTreeMap<String, String>,
    },
    Omit {
        id: String,
        item_id: String,
        reason: String,
    },
    RestoreOmission {
        id: String,
        at_ms: i64,
    },
    Layer {
        sequence: String,
        id: String,
        asset_id: String,
        at_ms: i64,
        source_start_ms: i64,
        duration_ms: i64,
        #[serde(default = "cover")]
        fit: String,
    },
    ReplaceLayer {
        item_id: String,
        asset_id: String,
        source_start_ms: i64,
    },
    TranscriptCorrect {
        asset_id: String,
        cues: BTreeMap<String, String>,
        #[serde(default)]
        sequences: Vec<String>,
    },
    ReviewAdd {
        id: String,
        job_id: String,
        at_ms: i64,
        end_ms: i64,
        actor: String,
        text: String,
    },
    ReviewResolve {
        id: String,
        state: ReviewState,
        note: String,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    Open,
    Addressed,
    Dismissed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Omission {
    id: String,
    sequence: String,
    start: RationalTime,
    duration: RationalTime,
    reason: String,
    items: Vec<(String, TimelineItem)>,
    restored: bool,
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn profile_key(id: &str, version: u32) -> String {
    format!("avw.profile.{id}.{version}")
}
fn template_key(id: &str, version: u32) -> String {
    format!("avw.template.{id}.{version}")
}
pub(crate) fn validate(project: &Project) -> Result<()> {
    let issues = agentcut_core::validate_project(project);
    if agentcut_core::has_errors(&issues) {
        return Err(Error::Invalid(serde_json::to_string(&issues)?));
    }
    Ok(())
}
fn batch(project: &Project, ops: Vec<Value>) -> Result<Project> {
    let ops: Vec<_> = ops
        .into_iter()
        .enumerate()
        .map(|(i, mut op)| {
            op["id"] = json!(format!("studio-{i}"));
            op
        })
        .collect();
    let batch: OperationBatch = serde_json::from_value(
        json!({"schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":project.revision,"idempotencyKey":"internal-studio-expansion","operations":ops}),
    )?;
    Ok(agentcut_core::apply_batch(project, &batch)?.project)
}
fn property(id: &str, property: &str, value: Value) -> Value {
    json!({"op":"item.set","target":id,"params":{"property":property,"value":value}})
}
fn profile(project: &Project, id: &str, version: u32) -> Result<Profile> {
    serde_json::from_value(
        project
            .extensions
            .get(&profile_key(id, version))
            .ok_or_else(|| invalid("profile version missing"))?
            .clone(),
    )
    .map_err(Error::from)
}
fn apply_profile(project: &Project, sequence: &str, profile: &Profile) -> Result<Project> {
    let mut ops = Vec::new();
    for item in project
        .require_sequence(sequence)?
        .tracks
        .iter()
        .flat_map(|t| &t.items)
    {
        match &item.payload {
            ItemPayload::Caption(_) => {
                for (path, value) in [
                    ("text.style.fontAssetId", json!(profile.font_asset_id)),
                    ("text.style.fontSize", json!(profile.font_size)),
                    ("text.style.color", json!(profile.color)),
                ] {
                    ops.push(property(&item.id, path, value));
                }
            }
            ItemPayload::Clip(_) => ops.push(property(&item.id, "video.fit", json!(profile.fit))),
            _ => {}
        }
    }
    let mut next = if ops.is_empty() {
        project.clone()
    } else {
        batch(project, ops)?
    };
    next.require_sequence_mut(sequence)?
        .extensions
        .insert("avw.profile".into(), serde_json::to_value(profile)?);
    Ok(next)
}
fn shift(sequence: &mut Sequence, at: RationalTime, delta: RationalTime) -> Result<()> {
    for item in sequence.tracks.iter_mut().flat_map(|t| &mut t.items) {
        if item.start.checked_cmp(at)?.is_ge() {
            item.start = item.start.checked_add(delta)?;
        } else if item.end_exclusive()?.checked_cmp(at)?.is_gt() {
            return Err(invalid(
                "ripple boundary crosses another layer; split that layer or choose an explicit timeline edit",
            ));
        }
    }
    // Marker times are shifted by the same exact delta.
    for marker in &mut sequence.markers {
        if marker.time.checked_cmp(at)?.is_ge() {
            marker.time = marker.time.checked_add(delta)?;
        }
    }
    sequence.duration = None;
    Ok(())
}
fn collect_ids(value: &Value, map: &mut BTreeMap<String, String>, prefix: &str) {
    match value {
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                map.insert(id.into(), format!("{prefix}_{id}"));
            }
            for v in object.values() {
                collect_ids(v, map, prefix);
            }
        }
        Value::Array(array) => {
            for v in array {
                collect_ids(v, map, prefix);
            }
        }
        _ => {}
    }
}
pub(crate) fn rewrite(value: &mut Value, map: &BTreeMap<String, String>) {
    match value {
        Value::String(v) => {
            if let Some(new) = map.get(v) {
                *v = new.clone();
            }
        }
        Value::Object(v) => {
            for x in v.values_mut() {
                rewrite(x, map);
            }
        }
        Value::Array(v) => {
            for x in v {
                rewrite(x, map);
            }
        }
        _ => {}
    }
}
fn clone_sequence(sequence: &Sequence, id: &str) -> Result<Sequence> {
    let mut value = serde_json::to_value(sequence)?;
    let mut map = BTreeMap::new();
    collect_ids(&value, &mut map, id);
    map.insert(sequence.id.clone(), id.into());
    rewrite(&mut value, &map);
    Ok(serde_json::from_value(value)?)
}
pub fn remap_reviews(project: &Project, sequence: &str) -> Result<Value> {
    let reviews = project
        .extensions
        .get("avw.reviews")
        .cloned()
        .unwrap_or(json!([]));
    let mut result = Vec::new();
    for review in reviews
        .as_array()
        .ok_or_else(|| invalid("invalid reviews"))?
    {
        let mut placements = Vec::new();
        if let Some(refs) = review["sources"].as_array() {
            for source in refs {
                let start: RationalTime = serde_json::from_value(source["start"].clone())?;
                for item in project
                    .require_sequence(sequence)?
                    .tracks
                    .iter()
                    .flat_map(|t| &t.items)
                    .filter(|i| i.enabled)
                {
                    if let ItemPayload::Clip(clip) = &item.payload
                        && source["assetId"] == clip.asset_id
                        && clip.video.speed.is_normal()
                        && clip.source_range.start.checked_cmp(start)?.is_le()
                        && clip
                            .source_range
                            .end_exclusive()?
                            .checked_cmp(start)?
                            .is_gt()
                    {
                        placements.push(json!({"itemId":item.id,"atMs":(item.start.checked_add(start.checked_sub(clip.source_range.start)?)?.as_seconds_f64()*1000.0).round() as i64}));
                    }
                }
            }
        }
        result.push(json!({"review":review,"placements":placements,"mapping":match placements.len(){0=>"removed_or_unmapped",1=>"resolved",_=>"ambiguous"}}));
    }
    Ok(json!({"revision":project.revision,"sequenceId":sequence,"items":result}))
}
impl Store {
    pub fn studio(
        &mut self,
        edit: &Edit,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        let root = self.root.clone();
        let origin = if let Edit::Variant {
            source_revision, ..
        } = edit
        {
            Some(self.revision(*source_revision)?)
        } else {
            None
        };
        let review = if let Edit::ReviewAdd { job_id, .. } = edit {
            let job = self.job(job_id)?;
            if job.state != "succeeded" {
                return Err(invalid("review requires a completed artifact"));
            }
            let result = job.result.ok_or_else(|| invalid("artifact missing"))?;
            let manifest: Value = crate::json::read(Path::new(
                result["manifest"]
                    .as_str()
                    .ok_or_else(|| invalid("video manifest missing"))?,
            ))?;
            if manifest["verification"]["sha256"]
                != media::hash_file(Path::new(
                    result["path"]
                        .as_str()
                        .ok_or_else(|| invalid("artifact path missing"))?,
                ))?
            {
                return Err(invalid("reviewed artifact bytes changed"));
            }
            Some(manifest)
        } else {
            None
        };
        self.change(key,expected,json!({"kind":"studio.edit","edit":edit,"expectedRevision":expected}),dry_run,|project| {
            let mut next=project.clone();
            match edit {
                Edit::AudioDuck {sequence,settings} => {crate::audio::validate(settings)?; for id in settings.dialogue_items.iter().chain(&settings.music_items) {if !project.require_sequence(sequence)?.tracks.iter().flat_map(|t| &t.items).any(|i| &i.id==id && i.payload.as_clip().is_some_and(|c|c.audio.enabled)){return Err(invalid("ducking requires active audio clip IDs in the selected sequence"));}}next.require_sequence_mut(sequence)?.extensions.insert("avw.ducking".into(),serde_json::to_value(settings)?);},
                Edit::ProfilePut{profile:p}=> {
                    workflow::id(&p.id)?;if p.version==0 || p.font_size==0 || p.font_size>200{return Err(invalid("profile version and font size must be positive; font size <=200"));}
                    if p.loudness_lufs.is_some_and(|v| !(-30.0..=-5.0).contains(&v)) || p.true_peak_db.is_some_and(|v| !(-9.0..=-0.1).contains(&v)) {return Err(invalid("audio target outside supported range"));}
                    let font=project.require_asset(&p.font_asset_id)?;if font.kind!=agentcut_core::AssetKind::Font{return Err(invalid("profile requires an imported font"));} media::verify_asset(&root,font)?;
                    if !matches!(p.fit.as_str(),"cover"|"contain"){return Err(invalid("profile fit must be contain or cover"));}
                    let name=profile_key(&p.id,p.version);if next.extensions.contains_key(&name){return Err(invalid("profile versions are immutable; create a new version"));}
                    next.extensions.insert(name,serde_json::to_value(p)?);
                },
                Edit::ProfileApply{sequence,profile_id,version}=>next=apply_profile(project,sequence,&profile(project,profile_id,*version)?)?,
                Edit::TemplatePut{template:t}=>{
                    workflow::id(&t.id)?;profile(project,&t.profile_id,t.profile_version)?;
                    if t.version==0 || t.slots.is_empty() || t.slots.len()>100{return Err(invalid("template requires positive version and 1..100 slots"));}
                    let mut ids=std::collections::BTreeSet::new();for slot in &t.slots{workflow::id(&slot.id)?;if !ids.insert(&slot.id) || slot.min_duration_ms<1 || slot.max_duration_ms<slot.min_duration_ms || slot.max_duration_ms>3_600_000{return Err(invalid("invalid template slot constraints"));}}
                    let name=template_key(&t.id,t.version);if next.extensions.contains_key(&name){return Err(invalid("template versions are immutable"));}next.extensions.insert(name,serde_json::to_value(t)?);
                },
                Edit::TemplateInstantiate{template_id,version,output_id,name,bindings}=>{
                    let t: Template=serde_json::from_value(project.extensions.get(&template_key(template_id,*version)).ok_or_else(||invalid("template version missing"))?.clone())?;
                    if bindings.len()!=t.slots.len(){return Err(invalid("template binding count does not match slots"));}
                    let p=profile(project,&t.profile_id,t.profile_version)?;
                    let mut cuts=Vec::new();for slot in &t.slots {let matches:Vec<_>=bindings.iter().filter(|c| c.id==slot.id).collect();if matches.len()!=1{return Err(invalid("every slot requires exactly one binding"));}let cut=matches[0];let duration=cut.end_ms.checked_sub(cut.start_ms).ok_or_else(||invalid("duration overflow"))?;if duration<slot.min_duration_ms || duration>slot.max_duration_ms{return Err(invalid("binding duration violates template constraint"));}cuts.push(cut.clone());}
                    let compose=workflow::Compose{output_id:output_id.clone(),name:name.clone(),font_asset_id:p.font_asset_id.clone(),cuts,width:t.width,height:t.height,font_size:p.font_size};
                    next=workflow::compose_project(project,&root,&compose,expected,key)?;next=apply_profile(&next,output_id,&p)?;next.require_sequence_mut(output_id)?.extensions.insert("avw.template".into(),serde_json::to_value(t)?);
                },
                Edit::Variant{sequence,id,name,width,height,language,captions,..}=>{
                    workflow::id(id)?;if *width==0 || *height==0 || *width>4096 || *height>4096 || !width.is_multiple_of(2) || !height.is_multiple_of(2){return Err(invalid("variant canvas requires even dimensions in 2..4096"));}
                    let source=origin.as_ref().ok_or_else(||invalid("source revision missing"))?;
                    let original=source.require_sequence(sequence)?;
                    for asset in &source.assets {if let Some(current)=next.assets.iter().find(|a| a.id==asset.id){if current.fingerprint!=asset.fingerprint{return Err(invalid("variant asset identity conflicts with current project"));}}else{next.assets.push(asset.clone());}}
                    let mut copy=clone_sequence(original,id)?;copy.name=name.clone();let sx=*width as f64/original.canvas.width as f64;let sy=*height as f64/original.canvas.height as f64;copy.canvas.width= *width;copy.canvas.height= *height;
                    for (old_track,new_track) in original.tracks.iter().zip(&mut copy.tracks){for (old,item) in old_track.items.iter().zip(&mut new_track.items){match &mut item.payload{
                        ItemPayload::Caption(c)=>{if let Some(text)=captions.get(&old.id){if text.trim().is_empty() || text.len()>2000{return Err(invalid("translated caption requires 1..2000 bytes"));}c.text=text.clone();}},
                        ItemPayload::Text(t)=>{t.text_box.width*=sx;t.text_box.height*=sy;t.transform.position.x*=sx;t.transform.position.y*=sy;},
                        ItemPayload::Clip(c)=>{c.transform.position.x*=sx;c.transform.position.y*=sy;},_=>{}
                    }}}
                    let actual=original.tracks.iter().flat_map(|t| &t.items).filter(|i| matches!(i.payload,ItemPayload::Caption(_))).count();if language.is_some() && captions.len()!=actual{return Err(invalid("language variants require an explicit translation for every caption"));}
                    if captions.keys().any(|id| !original.tracks.iter().flat_map(|t| &t.items).any(|i| &i.id==id && matches!(i.payload,ItemPayload::Caption(_)))){return Err(invalid("translation targets an unknown caption"));}
                    copy.extensions.insert("avw.variant".into(),json!({"sequenceId":sequence,"sourceRevision":source.revision,"language":language,"translationReview":"required","alignment":"one imported translated segment per source caption"}));next.sequences.push(copy);
                },
                Edit::Omit{id,item_id,reason}=>{
                    workflow::id(id)?;let name=format!("avw.omission.{id}");if next.extensions.contains_key(&name){return Err(invalid("omission ID already exists"));}
                    let (seq,track,item)=project.find_item(item_id).ok_or_else(||invalid("omitted item missing"))?;if !matches!(item.payload,ItemPayload::Clip(_)){return Err(invalid("omit requires a source clip; split partial ranges first"));}
                    if seq.transitions.iter().any(|t| t.left_item_id==*item_id || t.right_item_id==*item_id){return Err(invalid("remove dependent transition before omitting clip"));}
                    let mut saved=vec![(track.id.clone(),item.clone())];let start=item.start;let end=item.end_exclusive()?;
                    for other_track in &seq.tracks {for other in &other_track.items {if other.id!=*item_id && ((item.linked_group_id.is_some() && item.linked_group_id==other.linked_group_id) || (matches!(other.payload,ItemPayload::Caption(_)) && other.start.checked_cmp(start)?.is_ge() && other.end_exclusive()?.checked_cmp(end)?.is_le())){saved.push((other_track.id.clone(),other.clone()));}}}
                    let changed=next.require_sequence_mut(&seq.id)?;for t in &mut changed.tracks{t.items.retain(|i| !saved.iter().any(|(_,s)|s.id==i.id));}shift(changed,end,RationalTime::zero(item.duration.rate).checked_sub(item.duration)?)?;
                    next.extensions.insert(name,serde_json::to_value(Omission{id:id.clone(),sequence:seq.id.clone(),start,duration:item.duration,reason:reason.clone(),items:saved,restored:false})?);
                },
                Edit::RestoreOmission{id,at_ms}=>{
                    if *at_ms<0{return Err(invalid("restore destination must be nonnegative"));}
                    let name=format!("avw.omission.{id}");let mut omission: Omission=serde_json::from_value(next.extensions.get(&name).ok_or_else(||invalid("omission missing"))?.clone())?;if omission.restored{return Err(invalid("omission has already been restored"));}
                    let at=workflow::time(*at_ms).rescale_to(omission.start.rate,agentcut_core::RoundingMode::Nearest)?;
                    let seq=next.require_sequence_mut(&omission.sequence)?;let style=seq.tracks.iter().flat_map(|t| &t.items).find_map(|i|if let ItemPayload::Caption(c)=&i.payload{Some(c.style.clone())}else{None});
                    shift(seq,at,omission.duration)?;
                    for (track,mut item) in omission.items.clone(){item.start=at.checked_add(item.start.checked_sub(omission.start)?)?;item.id=format!("restored_{id}_{}",item.id);if let ItemPayload::Caption(c)=&mut item.payload && let Some(style)=&style{c.style=style.clone();}seq.tracks.iter_mut().find(|t| t.id==track).ok_or_else(||invalid("omission track was removed; explicit interchange needed"))?.items.push(item);}
                    for t in &mut seq.tracks{t.sort_items()?;}omission.restored=true;next.extensions.insert(name,serde_json::to_value(omission)?);
                },
                Edit::Layer{sequence,id,asset_id,at_ms,source_start_ms,duration_ms,fit}=>{
                    workflow::id(id)?;if *at_ms<0 || *source_start_ms<0 || *duration_ms<=0 || *duration_ms>3_600_000{return Err(invalid("invalid visual layer interval"));}
                    if !project.require_asset(asset_id)?.kind.is_visual(){return Err(invalid("layer requires a visual asset"));}
                    let track=format!("{id}_layer");next=batch(project,vec![json!({"op":"track.add","params":{"id":track,"sequence":sequence,"type":"video"}}),json!({"op":"clip.add","params":{"id":id,"asset":asset_id,"track":track,"at":workflow::time(*at_ms),"sourceIn":workflow::time(*source_start_ms),"duration":workflow::time(*duration_ms),"fit":fit}}),property(id,"audio.enabled",json!(false))])?;
                    next.find_item_mut(id).ok_or_else(||invalid("layer missing"))?.extensions.insert("avw.visualOnly".into(),json!(true));
                },
                Edit::ReplaceLayer{item_id,asset_id,source_start_ms}=>{
                    if *source_start_ms<0 || !project.require_asset(asset_id)?.kind.is_visual(){return Err(invalid("invalid replacement visual source"));}let item=next.find_item_mut(item_id).ok_or_else(||invalid("layer missing"))?;if item.extensions.get("avw.visualOnly")!=Some(&json!(true)){return Err(invalid("replace-layer targets only an explicitly visual-only layer"));}let clip=item.payload.as_clip_mut().ok_or_else(||invalid("layer is not a clip"))?;clip.asset_id=asset_id.clone();clip.source_range.start=workflow::time(*source_start_ms);clip.audio.enabled=false;
                },
                Edit::TranscriptCorrect{asset_id,cues,sequences}=>{
                    let mut transcripts=workflow::transcripts(project)?;let transcript=transcripts.iter_mut().find(|t| &t.asset_id==asset_id).ok_or_else(||invalid("transcript missing"))?;
                    let original=serde_json::to_value(&*transcript)?;let mut changes=Vec::new();
                    for (id,text) in cues{if text.trim().is_empty() || text.len()>2000{return Err(invalid("corrected cue requires 1..2000 bytes"));}let cue=transcript.cues.iter_mut().find(|c| &c.id==id).ok_or_else(||invalid("corrected cue missing"))?;changes.push(json!({"cueId":id,"originalText":cue.text,"correctedText":text}));cue.text=text.clone();}
                    for sequence in sequences {let seq=next.require_sequence_mut(sequence)?;for item in seq.tracks.iter_mut().flat_map(|t| &mut t.items){if let ItemPayload::Caption(c)=&mut item.payload && let Some(binding)=item.extensions.get("avw.sourceCue") && binding["assetId"]==*asset_id && let Some(text)=binding["cueId"].as_str().and_then(|id|cues.get(id)){c.text=text.clone();}}}
                    next.extensions.insert(workflow::TRANSCRIPTS.into(),serde_json::to_value(transcripts)?);let name=format!("avw.corrections.{asset_id}");let mut corrections=next.extensions.get(&name).cloned().unwrap_or(json!([]));corrections.as_array_mut().ok_or_else(||invalid("invalid correction history"))?.push(json!({"analysis":original,"changes":changes,"revision":expected+1}));next.extensions.insert(name,corrections);
                },
                Edit::ReviewAdd{id,job_id,at_ms,end_ms,actor,text}=>{
                    workflow::id(id)?;if *at_ms<0 || end_ms<=at_ms || actor.trim().is_empty() || text.trim().is_empty() || text.len()>4000{return Err(invalid("review requires an interval, actor and 1..4000 bytes of text"));}
                    let manifest=review.as_ref().ok_or_else(||invalid("review manifest missing"))?;let snapshot:Project=serde_json::from_value(manifest["snapshot"].clone())?;let sequence=manifest["sequenceId"].as_str().ok_or_else(||invalid("review sequence missing"))?;let seq=snapshot.require_sequence(sequence)?;if *end_ms as f64>seq.content_duration()?.as_seconds_f64()*1000.0+1.0{return Err(invalid("review extends beyond artifact duration"));}
                    let mut sources=Vec::new();for item in seq.tracks.iter().flat_map(|t| &t.items).filter(|i|i.enabled){let start=workflow::time(*at_ms);if item.start.checked_cmp(start)?.is_le() && item.end_exclusive()?.checked_cmp(start)?.is_gt() && let ItemPayload::Clip(c)=&item.payload && c.video.speed.is_normal(){sources.push(json!({"assetId":c.asset_id,"start":c.source_range.start.checked_add(start.checked_sub(item.start)?)?,"itemId":item.id}));}}
                    let reviews=next.extensions.entry("avw.reviews".into()).or_insert(json!([])).as_array_mut().ok_or_else(||invalid("invalid reviews"))?;if reviews.iter().any(|r|r["id"]==*id){return Err(invalid("review ID exists"));}reviews.push(json!({"id":id,"jobId":job_id,"artifactSha256":manifest["verification"]["sha256"],"artifactRevision":snapshot.revision,"sequenceId":sequence,"atMs":at_ms,"endMs":end_ms,"actor":actor,"text":text,"sources":sources,"state":"open"}));
                },
                Edit::ReviewResolve{id,state,note}=>{let reviews=next.extensions.get_mut("avw.reviews").and_then(Value::as_array_mut).ok_or_else(||invalid("reviews missing"))?;let review=reviews.iter_mut().find(|r|r["id"]==*id).ok_or_else(||invalid("review missing"))?;review["state"]=serde_json::to_value(state)?;review["resolution"]=json!({"revision":expected+1,"note":note});}
            }
            validate(&next)?;Ok(next)
        })
    }
}
