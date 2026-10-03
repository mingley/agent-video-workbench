//! Portable profile/template libraries contain fonts and validated data only.
use crate::{
    Error, Result, media,
    store::{Outcome, Store},
    studio::{Profile, Template},
    workflow,
};
use agentcut_core::Asset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Library {
    version: u32,
    profiles: Vec<Profile>,
    templates: Vec<Template>,
    fonts: Vec<Asset>,
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
pub fn export(root: &Path, destination: &Path) -> Result<Value> {
    let p = Store::open(root)?.project()?;
    let mut library = Library {
        version: 1,
        profiles: Vec::new(),
        templates: Vec::new(),
        fonts: Vec::new(),
    };
    for (name, value) in &p.extensions {
        if name.starts_with("avw.profile.") {
            library
                .profiles
                .push(serde_json::from_value(value.clone())?);
        }
        if name.starts_with("avw.template.") {
            library
                .templates
                .push(serde_json::from_value(value.clone())?);
        }
    }
    if library.profiles.is_empty() {
        return Err(invalid("project has no saved profiles"));
    }
    for profile in &library.profiles {
        if !library.fonts.iter().any(|a| a.id == profile.font_asset_id) {
            let font = p.require_asset(&profile.font_asset_id)?;
            media::verify_asset(root, font)?;
            library.fonts.push(font.clone());
        }
    }
    std::fs::create_dir(destination)?;
    std::fs::write(
        destination.join("library.incomplete"),
        b"Do not import incomplete library",
    )?;
    std::fs::create_dir(destination.join("originals"))?;
    for font in &library.fonts {
        let output = destination.join(&font.uri);
        if !output.exists() {
            std::fs::copy(root.join(&font.uri), &output)?;
            File::open(output)?.sync_all()?;
        }
        media::verify_asset(destination, font)?;
    }
    let mut file = File::create(destination.join("library.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&library)?)?;
    file.sync_all()?;
    std::fs::remove_file(destination.join("library.incomplete"))?;
    File::open(destination)?.sync_all()?;
    Ok(
        json!({"path":destination,"profileVersions":library.profiles.len(),"templateVersions":library.templates.len(),"fontObjects":library.fonts.len(),"privateFootageIncluded":false}),
    )
}
impl Store {
    pub fn library_import(
        &mut self,
        source: &Path,
        prefix: &str,
        expected: u64,
        key: &str,
        dry_run: bool,
    ) -> Result<Outcome> {
        workflow::id(prefix)?;
        if source.join("library.incomplete").exists() {
            return Err(invalid("library is incomplete"));
        }
        let library: Library = crate::json::read(&source.join("library.json"))?;
        if library.version != 1
            || library.fonts.len() > 100
            || library.profiles.len() > 1000
            || library.templates.len() > 1000
        {
            return Err(invalid("unsupported or oversized library"));
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("import.lock"))?;
        lock.try_lock()
            .map_err(|_| invalid("project import is active"))?;
        for font in &library.fonts {
            if font.kind != agentcut_core::AssetKind::Font
                || font.fingerprint.size_bytes > 32 * 1024 * 1024
            {
                return Err(invalid("library assets must be fonts <=32 MiB"));
            }
            media::verify_asset(source, font)?;
            if agentcut_render::probe::font_metrics(&source.join(&font.uri)).is_none() {
                return Err(invalid("library font has no readable metrics"));
            }
            if !dry_run {
                let output = self.root.join(&font.uri);
                if !output.exists() {
                    let temporary = self
                        .root
                        .join("cache")
                        .join(format!("library-{}", uuid::Uuid::new_v4()));
                    std::fs::copy(source.join(&font.uri), &temporary)?;
                    File::open(&temporary)?.sync_all()?;
                    if media::hash_file(&temporary)?
                        != font.fingerprint.sha256.as_deref().unwrap_or("")
                    {
                        std::fs::remove_file(temporary)?;
                        return Err(invalid("library font changed during copy"));
                    }
                    std::fs::hard_link(&temporary, &output)?;
                    std::fs::remove_file(temporary)?;
                }
                media::verify_asset(&self.root, font)?;
            }
        }
        if !dry_run {
            File::open(self.root.join("originals"))?.sync_all()?;
        }
        self.change(key,expected,json!({"kind":"library.import","prefix":prefix,"library":library,"expectedRevision":expected}),dry_run,|project|{
            let mut next=project.clone();let mut fonts=BTreeMap::new();for font in &library.fonts{let id=if let Some(existing)=next.assets.iter().find(|a|a.kind==agentcut_core::AssetKind::Font && a.fingerprint.sha256==font.fingerprint.sha256){existing.id.clone()}else{let mut copy=font.clone();copy.id=format!("{prefix}_{}",font.id);workflow::id(&copy.id)?;let id=copy.id.clone();next.assets.push(copy);id};fonts.insert(font.id.clone(),id);}
            for p in &library.profiles{let mut p=p.clone();p.id=format!("{prefix}_{}",p.id);workflow::id(&p.id)?;if p.version==0 || !(1..=200).contains(&p.font_size) || !matches!(p.fit.as_str(),"cover"|"contain") || p.loudness_lufs.is_some_and(|v|!(-30.0..=-5.0).contains(&v)) || p.true_peak_db.is_some_and(|v|!(-9.0..=-0.1).contains(&v)){return Err(invalid("invalid imported profile settings"));}p.font_asset_id=fonts.get(&p.font_asset_id).ok_or_else(||invalid("library profile font missing"))?.clone();let name=format!("avw.profile.{}.{}",p.id,p.version);if next.extensions.contains_key(&name){return Err(invalid("profile version already exists"));}next.extensions.insert(name,serde_json::to_value(p)?);}
            for t in &library.templates{let mut t=t.clone();t.id=format!("{prefix}_{}",t.id);t.profile_id=format!("{prefix}_{}",t.profile_id);workflow::id(&t.id)?;if !next.extensions.contains_key(&format!("avw.profile.{}.{}",t.profile_id,t.profile_version)) || t.version==0 || t.slots.is_empty() || t.slots.len()>100{return Err(invalid("invalid imported template or missing profile version"));}let name=format!("avw.template.{}.{}",t.id,t.version);if next.extensions.contains_key(&name){return Err(invalid("template version already exists"));}next.extensions.insert(name,serde_json::to_value(t)?);}
            crate::studio::validate(&next)?;Ok(next)
        })
    }
}
