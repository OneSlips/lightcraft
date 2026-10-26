//! Lens profiles looked up again when the lens database appears or changes. A photo's lens data is
//! stored when it is imported ([`Photo::embedded_lens`]), so photos imported before the database was there say "no
//! profile" until something reads their file again. When the database arrives (the first-run download) or is updated,
//! and when "Enable Lens Corrections" is ticked on a photo without a profile, every library raw that has no profile
//! but whose lens the database knows by name is probed again and gets what Reload from Disk would give it: the
//! profile, and the correction switched on where its settings are still what import gave it (an edited photo keeps
//! its settings).
//!
//! Split like [`super::recheck`] so the app probes on a worker thread: [`missing`] lists what to probe (no
//! file-system calls), [`probe`] reads the files without the session, [`apply`] applies the results as LightCraft's
//! own bookkeeping (journaled, not an undo step). Agents and tests use `library.refreshLensProfiles`.

use lightcraft_catalog::{Catalog, Op, Photo, PhotoId};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd};
use crate::media::{FileProbe, ProbeInfo};
use crate::{Result, Session};

/// A photo to probe again, and its file when listed (a result for a photo moved meanwhile is dropped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub id: PhotoId,
    pub path: String,
}

/// Library raws with a file, no lens profile and a lens the database `db` knows by name (`ids`: only these; virtual
/// copies follow the photo they were copied from). No file-system calls.
pub fn missing(cat: &Catalog, db: &lightcraft_lensdb::LensDb, ids: Option<&[PhotoId]>) -> Vec<Missing> {
    // a photo and its virtual copies list the photo once, in library order
    let mut seen = std::collections::HashSet::new();
    cat.photos()
        .filter(|p| ids.is_none_or(|ids| ids.contains(&p.id) || p.copy_of.is_some_and(|c| ids.contains(&c))))
        .map(|p| p.copy_of.and_then(|c| cat.photo(c)).map_or(&**p, |orig| &**orig))
        .filter(|p| p.develops_raw() && p.embedded_lens.is_none() && !p.meta.lens.trim().is_empty())
        .filter(|p| crate::lens_profiles::profile_name(db, &p.meta.camera, &p.meta.lens).is_some())
        .filter_map(|p| crate::cmd::missing::checked_path(p).map(|path| Missing { id: p.id, path: path.to_string() }))
        .filter(|m| seen.insert(m.id))
        .collect()
}

/// Probe each listed photo's file as Reload would. Doesn't touch the session.
pub fn probe(probe: &FileProbe, list: Vec<Missing>) -> Vec<(Missing, std::result::Result<ProbeInfo, String>)> {
    let paths: Vec<String> = list.iter().map(|x| x.path.clone()).collect();
    list.into_iter().zip(crate::import::probe_with(probe, &paths)).collect()
}

/// Apply [`probe`]'s results: each photo and its virtual copies get the lens data their file gives now, in one batch
/// that is journaled but isn't an undo step. Returns the photos that gained a profile.
pub fn apply(s: &mut Session, results: Vec<(Missing, std::result::Result<ProbeInfo, String>)>) -> Result<Vec<PhotoId>> {
    let mut ops = Vec::new();
    let mut found = Vec::new();
    for (item, info) in results {
        let Ok(info) = info else { continue };
        if info.embedded_lens.is_none() {
            continue;
        }
        // the photo moved or left the library while its file was read: the result is stale, for its copies too
        if s.catalog.photo(item.id).and_then(|p| crate::cmd::missing::checked_path(p)) != Some(item.path.as_str()) {
            continue;
        }
        let family: Vec<PhotoId> = s.catalog.photos().filter(|p| p.id == item.id || p.copy_of == Some(item.id)).map(|p| p.id).collect();
        for id in family {
            if let Some(p) = s.catalog.photo(id) {
                let mine = crate::cmd::convert::lens_ops(id, p, &info);
                if !mine.is_empty() {
                    ops.extend(mine);
                    found.push(id);
                }
            }
        }
    }
    if !ops.is_empty() {
        s.apply_system(Op::Batch { ops })?;
    }
    for id in &found {
        s.media.forget(*id);
    }
    Ok(found)
}

/// Why a photo has no lens profile, in words for the Optics panel: `None` when it has one.
pub fn no_profile_reason(p: &Photo) -> Option<String> {
    if p.embedded_lens.is_some() {
        return None;
    }
    let lens = p.meta.lens.trim();
    Some(if lens.is_empty() { "This file names no lens.".to_string() } else { format!("No lens profile found for {lens}.") })
}

fn refresh(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.refreshLensProfiles";
    let fp = super::recheck::file_probe(s).ok_or_else(|| bad(C, "this session has no file access"))?;
    let ids: Option<Vec<PhotoId>> = match p.get("ids") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.iter().map(|v| v.as_u64().map(PhotoId).ok_or_else(|| bad(C, "ids are photo ids"))).collect::<Result<_>>()?),
        Some(_) => return Err(bad(C, "ids is a list of photo ids")),
    };
    let Some(db) = crate::lens_profiles::database() else {
        return Ok(json!({"checked": 0, "found": [], "database": false}));
    };
    let listed = missing(&s.catalog, &db, ids.as_deref());
    let n = listed.len();
    let found = apply(s, probe(&fp, listed))?;
    Ok(json!({"checked": n, "found": found.iter().map(|i| i.0).collect::<Vec<_>>(), "database": true}))
}

fn download(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "lens.downloadDatabase";
    let dir = crate::lensfun_download::default_dir().ok_or_else(|| bad(C, "this session has no data folder"))?;
    let activity = s.activity.clone();
    s.lensfun.start(&dir, crate::lensfun_download::mirrors(), crate::lensfun_download::FILES, &activity).map_err(|e| bad(C, e))?;
    Ok(json!({"started": true, "dir": dir.to_string_lossy()}))
}

fn status(s: &mut Session, _: &Value) -> Result<Value> {
    use crate::lensfun_download::State;
    let state = match s.lensfun.state() {
        State::Idle => json!({"state": "idle"}),
        State::Running { bytes, total } => json!({"state": "running", "bytes": bytes, "total": total}),
        State::Done { files } => json!({"state": "done", "files": files}),
        State::Failed(e) => json!({"state": "failed", "error": e}),
        State::Cancelled => json!({"state": "cancelled"}),
    };
    let dir = crate::lens_profiles::dir();
    let lenses = crate::lens_profiles::database().map_or(0, |db| db.lenses.len());
    Ok(json!({"download": state, "dir": dir.map(|d| d.to_string_lossy().to_string()), "lenses": lenses}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.refreshLensProfiles",
            "Look Up Lens Profiles Again",
            [],
            None,
            "{ids?: [photo ids]} — library raws without a lens profile whose lens the lens database knows are read again and get their profile (the correction is switched on where the photo's settings are still as imported); the app does this by itself when the database arrives or changes and when Enable Lens Corrections is ticked; not an undo step → {checked, found: [ids], database: bool}",
            always,
            refresh
        ),
        cmd!(
            "lens.downloadDatabase",
            "Download the Lens Database",
            [],
            None,
            "{} — fetch lensfun's lens database (CC BY-SA 3.0, pinned to one commit, checked file by file) into the app's data folder in the background, with an activity row; the desktop app does this by itself on first run. LIGHTCRAFT_LENSFUN_URL lists other places to fetch the same files from → {started, dir}",
            always,
            download
        ),
        cmd!(
            "lens.databaseStatus",
            "Lens Database Status",
            [],
            None,
            "{} → {download: {state: idle|running|done|failed|cancelled, …}, dir, lenses (in the database in use)}",
            always,
            status
        ),
    ]
}
