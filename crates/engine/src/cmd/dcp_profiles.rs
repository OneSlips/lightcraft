//! Imported camera profiles: the user's own DNG Camera Profiles (`.dcp`), chosen like any other
//! profile (`develop.profile {id: "dcp:…"}`). Unlike the built-in looks and LUT profiles, which
//! work on developed colour, a camera profile replaces the raw file's colour model when it is
//! decoded (its matrices and hue/saturation map; see [`lightcraft_raw::dcp`]), so the photo's
//! source is decoded again when it changes. Other photos (JPEG, TIFF…) ignore it.
//!
//! A library on disk keeps a copy of each file in its `Profiles/` folder; the list lives in the
//! library's prefs and the profiles are parsed again when the library opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_raw::dcp::DcpProfile;
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

/// Profile id prefix of imported camera profiles.
pub const DCP_PREFIX: &str = "dcp:";
/// Profile browser group of imported camera profiles.
pub const DCP_GROUP: &str = "Camera Profiles";

/// One imported camera profile.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DcpEntry {
    /// `dcp:<slug>`.
    pub id: String,
    /// `ProfileName`, else the file name.
    pub name: String,
    /// The camera it was made for (`UniqueCameraModel`).
    #[serde(default)]
    pub camera_model: String,
    /// The `.dcp` file (the library's copy when there is a library on disk).
    pub file: String,
    /// The camera bodies (serial numbers) the profile is for: the one it was measured on, and those
    /// it was adopted for. Empty: every body of the model. Not in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<String>,
    /// The bodies whose default this profile was made explicitly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_defaults: Vec<String>,
    /// (Old entries: the one body; moved into `bodies` when the library opens.)
    #[serde(default, skip_serializing)]
    pub serial: Option<String>,
}

/// Is `id` an imported camera profile id?
pub fn is_dcp_id(id: &str) -> bool {
    id.starts_with(DCP_PREFIX)
}

/// Where imported profile files are kept: the app store's `Profiles/`, else the library's.
fn profiles_dir(s: &Session) -> Option<PathBuf> {
    s.profiles_dir()
}

/// Parse every imported camera profile again and hand them to the decoder (missing or damaged
/// files are skipped: photos using them develop with the file's own colour).
pub fn register_all(s: &mut Session) {
    for e in s.dcp_profiles.iter_mut() {
        migrate(e);
    }
    let loaded: Vec<(String, Arc<DcpProfile>)> = s
        .dcp_profiles
        .iter()
        .filter_map(|p| std::fs::read(&p.file).ok().and_then(|b| lightcraft_raw::dcp::parse_dcp(&b).ok()).map(|d| (p.id.clone(), Arc::new(d))))
        .collect();
    s.media.camera_profiles = loaded.into_iter().collect();
}

/// `.dcp` files under `paths` (folders searched).
pub fn dcp_files(paths: &[String]) -> Vec<PathBuf> {
    let is_dcp = |f: &Path| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("dcp"));
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            out.extend(crate::walk::files_in(path, None, crate::walk::Limits::default(), is_dcp).files);
        } else if is_dcp(path) {
            out.push(path.to_path_buf());
        }
    }
    out
}

/// Import the `.dcp` files under `paths` → (imported `[{id, name, cameraModel}]`, failed `[[file, error]]`).
/// A profile already imported (same name, camera and bytes) is not added twice.
pub fn import_files(s: &mut Session, paths: &[String]) -> Result<(Vec<Value>, Vec<Value>)> {
    const C: &str = "profile.import";
    let dir = profiles_dir(s);
    if let Some(d) = &dir {
        std::fs::create_dir_all(d).map_err(|e| bad(C, format!("{}: {e}", d.display())))?;
    }
    let (mut imported, mut failed) = (Vec::new(), Vec::new());
    for f in dcp_files(paths) {
        let shown = f.display().to_string();
        let bytes = match std::fs::read(&f) {
            Ok(b) => b,
            Err(e) => {
                failed.push(json!([shown, e.to_string()]));
                continue;
            }
        };
        let profile = match lightcraft_raw::dcp::parse_dcp(&bytes) {
            Ok(p) => p,
            Err(e) => {
                failed.push(json!([shown, e.to_string()]));
                continue;
            }
        };
        let stem = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Camera Profile".into());
        let name = if profile.name.trim().is_empty() { stem.clone() } else { profile.name.trim().to_string() };
        // the same profile imported again: keep the one there is
        let same = s
            .dcp_profiles
            .iter()
            .find(|p| p.name == name && p.camera_model == profile.camera_model && std::fs::read(&p.file).is_ok_and(|b| b == bytes));
        if let Some(p) = same {
            imported.push(json!({"id": p.id, "name": p.name, "cameraModel": p.camera_model, "existing": true}));
            continue;
        }
        // (the file name keeps profiles with the same ProfileName apart, e.g. per-body copies)
        let slug = crate::presets::slug(&format!("{}-{stem}", profile.camera_model));
        let mut id = format!("{DCP_PREFIX}{slug}");
        let mut n = 2;
        while s.dcp_profiles.iter().any(|x| x.id == id) {
            id = format!("{DCP_PREFIX}{slug}-{n}");
            n += 1;
        }
        let file = match &dir {
            Some(d) => {
                let to = d.join(format!("{}.dcp", id.trim_start_matches(DCP_PREFIX)));
                std::fs::write(&to, &bytes).map_err(|e| bad(C, format!("{}: {e}", to.display())))?;
                to.to_string_lossy().to_string()
            }
            // no library on disk: remember the original (the in-memory session ends with the app)
            None => f.to_string_lossy().to_string(),
        };
        s.media.camera_profiles.insert(id.clone(), Arc::new(profile.clone()));
        s.dcp_profiles.push(DcpEntry {
            id: id.clone(),
            name: name.clone(),
            camera_model: profile.camera_model.clone(),
            file,
            bodies: Vec::new(),
            body_defaults: Vec::new(),
            serial: None,
        });
        imported.push(json!({"id": id, "name": name, "cameraModel": profile.camera_model}));
    }
    Ok((imported, failed))
}

/// What to do when a profile with the same name for the same camera is already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnExists {
    /// Save nothing; say which profile is there (the UI then asks).
    Ask,
    /// Write over it in place: same id, so the photos using it follow the new version (undoable).
    Replace,
    /// Save beside it as "… 2", "… 3".
    KeepBoth,
    /// An error naming the existing profile and these choices.
    Fail,
}

impl OnExists {
    pub fn parse(v: Option<&str>) -> Option<OnExists> {
        Some(match v.unwrap_or("fail") {
            "ask" => OnExists::Ask,
            "replace" => OnExists::Replace,
            "keepBoth" => OnExists::KeepBoth,
            "fail" => OnExists::Fail,
            _ => return None,
        })
    }
}

/// The profile named `name` for `camera_model`, if there is one.
pub fn existing<'a>(s: &'a Session, name: &str, camera_model: &str) -> Option<&'a DcpEntry> {
    s.dcp_profiles.iter().find(|p| p.name == name.trim() && p.camera_model == camera_model)
}

/// How many photos develop with profile `id`.
pub fn used_by(s: &Session, id: &str) -> usize {
    s.catalog.photos().filter(|p| p.develop.calibration_id() == Some(id)).count()
}

/// Choose profile `id` in `d`, the two layers apart: a camera profile (`dcp:…`) sets the camera
/// calibration and keeps the look (a camera profile with a look of its own, a tone curve or look
/// table, comes with it: "As profile"); `""` goes back to LightCraft's own camera colour; any other
/// id sets the look and keeps the calibration. Edits from before the two were separate (a camera
/// profile as the look) keep their camera profile as the calibration.
pub fn choose(s: &Session, d: &mut lightcraft_develop::DevelopSettings, id: &str) {
    let legacy = d.camera_profile.is_empty() && is_dcp_id(&d.profile.id);
    if is_dcp_id(id) {
        let own_look = s.media.camera_profiles.get(id).is_some_and(|p| p.color.profile.tone_curve.is_some() || p.color.profile.look_table.is_some());
        d.camera_profile = id.to_string();
        if own_look {
            d.profile.id = id.to_string();
        } else if is_dcp_id(&d.profile.id) {
            d.profile.id = lightcraft_develop::Profile::default().id;
        }
    } else if id.is_empty() || id == lightcraft_develop::BUILTIN_CAMERA_COLOR {
        d.camera_profile = lightcraft_develop::BUILTIN_CAMERA_COLOR.to_string();
        if is_dcp_id(&d.profile.id) {
            d.profile.id = lightcraft_develop::Profile::default().id;
        }
    } else {
        if legacy {
            d.camera_profile = d.profile.id.clone();
        }
        d.profile.id = id.to_string();
    }
}

/// The first free "name 2", "name 3"… for `camera_model`.
pub fn free_name(s: &Session, name: &str, camera_model: &str) -> String {
    let base = name.trim();
    (2..).map(|n| format!("{base} {n}")).find(|n| existing(s, n, camera_model).is_none()).unwrap_or_else(|| base.to_string())
}

/// What [`add_profile`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum Added {
    /// A new profile.
    New(DcpEntry),
    /// An existing one written over in place (same id).
    Replaced(DcpEntry),
    /// Nothing saved: a profile of that name is there ([`OnExists::Ask`]); how many photos use it.
    Exists(DcpEntry, usize),
}

/// A camera profile's file and entry as they were before a Replace: what its undo puts back.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSwap {
    pub entry: DcpEntry,
    pub bytes: Vec<u8>,
}

/// Put `swap` in place (its file, its entry, the decoder's copy; the photos using it are decoded
/// again) → the swap that undoes it.
pub(crate) fn apply_swap(s: &mut Session, swap: &ProfileSwap) -> Result<ProfileSwap> {
    const C: &str = "profile.undoReplace";
    let profile = lightcraft_raw::dcp::parse_dcp(&swap.bytes).map_err(|e| bad(C, e.to_string()))?;
    let i = s.dcp_profiles.iter().position(|p| p.id == swap.entry.id).ok_or_else(|| bad(C, format!("no camera profile `{}`", swap.entry.id)))?;
    let now = s.dcp_profiles.get(i).cloned().ok_or_else(|| bad(C, "no such profile"))?;
    let now_bytes = std::fs::read(&now.file).map_err(|e| bad(C, format!("{}: {e}", now.file)))?;
    std::fs::write(&swap.entry.file, &swap.bytes).map_err(|e| bad(C, format!("{}: {e}", swap.entry.file)))?;
    if let Some(slot) = s.dcp_profiles.get_mut(i) {
        *slot = swap.entry.clone();
    }
    profile_changed(s, &swap.entry.id, profile);
    s.save_prefs()?;
    Ok(ProfileSwap { entry: now, bytes: now_bytes })
}

/// A camera profile's content changed under its id: the decoder gets the new one, and the photos
/// using it are decoded and rendered again.
fn profile_changed(s: &mut Session, id: &str, profile: DcpProfile) {
    s.media.camera_profiles.insert(id.to_string(), Arc::new(profile));
    *s.media.camera_profile_versions.entry(id.to_string()).or_insert(0) += 1;
    let users: Vec<lightcraft_catalog::PhotoId> = s.catalog.photos().filter(|p| p.develop.profile.id == id).map(|p| p.id).collect();
    for u in users {
        s.media.forget(u);
    }
}

/// File a camera profile made in LightCraft (`profile.createFromChart`) among the imported ones: its
/// `.dcp` (named after the profile, file-safe: [`lightcraft_camprof::file_stem`]) goes into the
/// library's `Profiles/` folder (the temp folder without a library on disk). `serial` is the camera
/// body's serial number, kept with the entry (not in the file). A profile of the same name for the
/// same camera is handled as `on_exists` says.
pub fn add_profile(s: &mut Session, mut profile: DcpProfile, bodies: Vec<String>, on_exists: OnExists, c: &str) -> Result<Added> {
    let bodies: Vec<String> = bodies.iter().map(|b| norm_serial(b)).filter(|b| !b.is_empty()).collect();
    let name = if profile.name.trim().is_empty() { "Camera Profile".to_string() } else { profile.name.trim().to_string() };
    profile.name = name.clone();
    if let Some(there) = existing(s, &name, &profile.camera_model).cloned() {
        match on_exists {
            OnExists::Ask => return Ok(Added::Exists(there.clone(), used_by(s, &there.id))),
            OnExists::Fail => {
                return Err(bad(
                    c,
                    format!(
                        "a camera profile “{name}” for {} is already there ({}, used by {} photos): pass onExists: \"replace\" to write over it (the photos follow; undoable) or \"keepBoth\" to save this one as “{}”, or give another name",
                        profile.camera_model,
                        there.id,
                        used_by(s, &there.id),
                        free_name(s, &name, &profile.camera_model)
                    ),
                ));
            }
            OnExists::KeepBoth => profile.name = free_name(s, &name, &profile.camera_model),
            OnExists::Replace => {
                let bytes = lightcraft_raw::dcp::write_dcp(&profile).map_err(|e| bad(c, e.to_string()))?;
                let old = std::fs::read(&there.file).map_err(|e| bad(c, format!("{}: {e}", there.file)))?;
                std::fs::write(&there.file, &bytes).map_err(|e| bad(c, format!("{}: {e}", there.file)))?;
                let mut entry = there.clone();
                if !entry.bodies.is_empty() {
                    for b in bodies.iter().filter(|b| !entry.bodies.contains(b)).collect::<Vec<_>>() {
                        entry.bodies.push(b.clone());
                    }
                }
                if let Some(slot) = s.dcp_profiles.iter_mut().find(|p| p.id == there.id) {
                    *slot = entry.clone();
                }
                profile_changed(s, &there.id, profile);
                s.commit_profile_swap(&format!("Replace Camera Profile “{name}”"), ProfileSwap { entry: there, bytes: old })?;
                return Ok(Added::Replaced(entry));
            }
        }
    }
    let name = profile.name.clone();
    let bytes = lightcraft_raw::dcp::write_dcp(&profile).map_err(|e| bad(c, e.to_string()))?;
    let slug = crate::presets::slug(&format!("{}-{name}", profile.camera_model));
    let mut id = format!("{DCP_PREFIX}{slug}");
    let mut n = 2;
    while s.dcp_profiles.iter().any(|x| x.id == id) {
        id = format!("{DCP_PREFIX}{slug}-{n}");
        n += 1;
    }
    let dir = profiles_dir(s).unwrap_or_else(|| std::env::temp_dir().join("lightcraft-camera-profiles"));
    std::fs::create_dir_all(&dir).map_err(|e| bad(c, format!("{}: {e}", dir.display())))?;
    // the file is named after the profile (renaming the profile later doesn't rename it)
    let stem = lightcraft_camprof::file_stem(&name);
    let taken = |p: &std::path::Path| p.exists() || s.dcp_profiles.iter().any(|x| std::path::Path::new(&x.file) == p);
    let mut to = dir.join(format!("{stem}.dcp"));
    let mut n = 2;
    while taken(&to) && n < 10_000 {
        to = dir.join(format!("{stem}_{n}.dcp"));
        n += 1;
    }
    std::fs::write(&to, &bytes).map_err(|e| bad(c, format!("{}: {e}", to.display())))?;
    let entry = DcpEntry {
        id: id.clone(),
        name,
        camera_model: profile.camera_model.clone(),
        file: to.to_string_lossy().to_string(),
        bodies,
        body_defaults: Vec::new(),
        serial: None,
    };
    s.media.camera_profiles.insert(id, Arc::new(profile));
    s.dcp_profiles.push(entry.clone());
    Ok(Added::New(entry))
}

/// The chart photograph a command works on: `path`, else the target photo's file.
fn chart_source(s: &Session, p: &Value, c: &str) -> Result<(Option<lightcraft_catalog::PhotoId>, String)> {
    if let Some(path) = str_param(p, "path") {
        return Ok((None, path.to_string()));
    }
    let id = s.targets(p)?.into_iter().next().ok_or_else(|| bad(c, "no photo: select the chart photo or give `id` or `path`"))?;
    let ph = s.catalog.photo(id).ok_or_else(|| bad(c, format!("no photo {}", id.0)))?;
    match &ph.source {
        lightcraft_catalog::Source::File { path } => Ok((Some(id), path.clone())),
        _ => Err(bad(c, "the photo has no file to profile from (a demo photo?)")),
    }
}

fn corners_param(p: &Value, c: &str) -> Result<Option<[[f64; 2]; 4]>> {
    let Some(v) = p.get("corners").filter(|v| !v.is_null()) else { return Ok(None) };
    let pts: Vec<[f64; 2]> = serde_json::from_value(v.clone()).map_err(|_| bad(c, "`corners` must be four [x, y] points (0–1)"))?;
    let pts: [[f64; 2]; 4] = pts.try_into().map_err(|_| bad(c, "`corners` must be four [x, y] points (0–1)"))?;
    Ok(Some(pts))
}

fn find_chart(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.findChart";
    let (id, path) = chart_source(s, p, C)?;
    let _task = s.activity.start("cameraProfile", "Finding the color chart", crate::activity::Cancel::No);
    let prep = crate::chart_profiling::prepare_path(&path).map_err(|e| bad(C, e))?;
    let found = prep.located.map(|l| {
        let light = crate::chart_profiling::estimate_light(&prep, &l);
        json!({
            "corners": crate::chart_profiling::normalized_corners(&prep.frame, &l.corners),
            "turns": l.turns,
            "score": l.score,
            "estimatedCct": light.map(|e| e.cct.round()),
            "estimatedIlluminant": light.map(|e| e.illuminant),
        })
    });
    Ok(json!({
        "photo": id.map(|i| i.0),
        "path": path,
        "cameraModel": prep.camera_model,
        "frame": [prep.frame.width, prep.frame.height],
        "found": found.is_some(),
        "chart": found,
    }))
}

/// One chart photo's part of a request: `{corners?, turns?, refine?, illuminant?}`.
fn chart_request(p: &Value, c: &str) -> Result<crate::chart_profiling::Request> {
    Ok(crate::chart_profiling::Request {
        corners: corners_param(p, c)?,
        turns: p.get("turns").and_then(Value::as_u64).map(|t| (t % 4) as u8),
        refine: p.get("refine").and_then(Value::as_bool).unwrap_or(false),
        illuminant: str_param(p, "illuminant").unwrap_or("").to_string(),
    })
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn create_from_chart(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.createFromChart";
    let (id, path) = chart_source(s, p, C)?;
    // (natural / hueSaturation and accurate / hueSaturationLightness: earlier names)
    let natural = match str_param(p, "rendering").unwrap_or("photographic") {
        "photographic" | "natural" | "hueSaturation" => true,
        "reproduction" | "accurate" | "hueSaturationLightness" => false,
        other => return Err(bad(C, format!("unknown rendering `{other}` (photographic or reproduction)"))),
    };
    let options = lightcraft_camprof::BuildOptions {
        illuminant: String::new(),
        natural,
        correction_table: p.get("correctionTable").and_then(Value::as_bool).unwrap_or(true),
        camera_model: str_param(p, "cameraModel").unwrap_or("").to_string(),
        name: str_param(p, "name").unwrap_or("").to_string(),
    };
    let first = chart_request(p, C)?;
    if OnExists::parse(str_param(p, "onExists")).is_none() {
        return Err(bad(C, "`onExists` is ask, replace, keepBoth or fail"));
    }
    // the second photo, under another light: named explicitly (never the selection)
    let second = match p.get("second").filter(|v| !v.is_null()) {
        None => None,
        Some(q) if q.is_object() => {
            let source = if let Some(path) = str_param(q, "path") {
                (None, path.to_string())
            } else if let Some(n) = q.get("id").and_then(Value::as_u64) {
                chart_source(s, &json!({"id": n}), C)?
            } else {
                return Err(bad(C, "`second` needs the other chart photo's `id` or `path`"));
            };
            Some((source, chart_request(q, C)?))
        }
        Some(_) => return Err(bad(C, "`second` must be an object: {id? | path?, corners?, turns?, illuminant?}")),
    };
    let task = s.activity.start("cameraProfile", "Creating camera profile", crate::activity::Cancel::No);
    task.detail(&file_name(&path));
    let prep_a = crate::chart_profiling::prepare_path(&path).map_err(|e| bad(C, e))?;
    let prep_b = match &second {
        Some(((_, path_b), _)) => {
            task.detail(&file_name(path_b));
            Some(crate::chart_profiling::prepare_path(path_b).map_err(|e| bad(C, e))?)
        }
        None => None,
    };
    let out = crate::chart_profiling::build((&prep_a, &first), prep_b.as_ref().zip(second.as_ref()).map(|(prep, (_, req))| (prep, req)), &options)
        .map_err(|e| bad(C, e))?;
    drop(task);
    let mut apply_to = Vec::new();
    if p.get("apply").and_then(Value::as_bool).unwrap_or(true) {
        apply_to.extend(id);
        apply_to.extend(second.as_ref().and_then(|((i, _), _)| *i));
    }
    let on_exists = OnExists::parse(str_param(p, "onExists")).ok_or_else(|| bad(C, "`onExists` is ask, replace, keepBoth or fail"))?;
    let installed = crate::chart_profiling::install(s, &out.dcp, prep_a.body.clone().into_iter().collect(), on_exists, &apply_to)?;
    // the chart photos' catalog entries learn their serial (libraries from before serials were kept)
    for (photo, prep) in [(id, Some(&prep_a)), (second.as_ref().and_then(|((i, _), _)| *i), prep_b.as_ref())] {
        if let (Some(photo), Some(serial)) = (photo, prep.and_then(|p| p.serial.clone())) {
            backfill_serial(s, photo, &serial);
        }
    }
    let preps: Vec<&crate::chart_profiling::Prepared> = std::iter::once(&prep_a).chain(prep_b.as_ref()).collect();
    let mut report = crate::chart_profiling::report_json(&preps, &out);
    if let (Some(o), Some(i)) = (report.as_object_mut(), installed.as_object()) {
        for (k, v) in i {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok(report)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.deleteCamera";
    let id = str_param(p, "id").ok_or_else(|| bad(C, "missing id"))?.to_string();
    let Some(i) = s.dcp_profiles.iter().position(|x| x.id == id) else {
        return Err(bad(C, format!("no imported camera profile `{id}`")));
    };
    let entry = s.dcp_profiles.remove(i);
    s.media.camera_profiles.remove(&id);
    // the library's copy goes; a file outside the library is the user's and stays
    if let Some(d) = profiles_dir(s)
        && Path::new(&entry.file).starts_with(&d)
    {
        let _ = std::fs::remove_file(&entry.file);
    }
    s.profile_favorites.retain(|f| f != &id);
    s.profile_recent.retain(|f| f != &id);
    s.save_prefs()?;
    Ok(json!({"deleted": id}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "profile.findChart",
            "Find Color Chart",
            [],
            None,
            "{id? | path?: a raw photo of a ColorChecker 24} → {photo, path, cameraModel, frame: [w, h], found, chart: {corners: [[x, y] ×4], turns, score, estimatedCct, estimatedIlluminant}}: estimatedCct is what the chart's greys measure through the file's colour matrix, for information: corners are 0–1 in the developed raw frame (default crop) as the sensor recorded it (not rotated), top left, top right, bottom right, bottom left; turns 0–3 = patch A01 at the top left, top right, bottom right, bottom left",
            always,
            find_chart
        ),
        cmd!(
            "profile.createFromChart",
            "Create Camera Profile from Color Chart",
            [],
            None,
            "{id? | path?, corners?: [[x, y] ×4] (0–1, as profile.findChart; found automatically when omitted), turns?: 0–3 (decided by fitting when omitted), refine?: bool, illuminant?: D55 (default; also daylight, flash) | A (also tungsten, StdA) | D50 | D65 | D75 (auto: no longer supported, read as D55 with a warning), second?: {id? | path?, corners?, turns?, refine?, illuminant?} (a second chart photo under another light, e.g. tungsten: a dual-illuminant profile; the same light twice is refused), rendering?: photographic (default: colours match the chart, each keeps the lightness the camera recorded; for people and everyday photos; aliases natural, hueSaturation) | reproduction (everything matches the chart exactly, lightness included; for artwork, products and copy work; aliases accurate, hueSaturationLightness), correctionTable?: bool (default true), name? (default: '<camera> <body serial's last 2> · <light(s), StdA + D65> · <Photo or Repro> · <capture date>'), onExists?: fail (default: an error naming the profile there) | replace (write over it, same id: the photos using it follow; undoable) | keepBoth (save as '… 2') | ask (save nothing, answer {exists: {id, name, usedBy}, keepBothName}), cameraModel? (default: the file's), apply?: bool (default true: the chart photos develop with the new profile)} — fits a DNG camera profile (ColorMatrix, ForwardMatrix, HueSatMap per light; CalibrationIlluminant1 the lower colour temperature) to a ColorChecker 24 and adds it to the camera profiles (dcp: id) → {id, name, cameraModel, serial, file, applied: [ids], replaced, dual, meanDeltaE, maxDeltaE, matrixMeanDeltaE, illuminant, corners, turns, patches, calibrations: [{shot, illuminant, cct, estimatedCct, estimatedIlluminant, meanDeltaE, maxDeltaE, matrixMeanDeltaE, patchesUsed, corners, turns, notes, patches: [{id, name, deltaE, matrixDeltaE, referenceLab, renderedLab}]}]} (ΔE = CIEDE2000)",
            always,
            create_from_chart
        ),
        cmd!(
            query "profiles.camera",
            "Imported Camera Profiles",
            [],
            None,
            "{} → {profiles: [{id, name, cameraModel, file}]}: the imported .dcp camera profiles (use with develop.profile)",
            always,
            |s, _| Ok(json!({"profiles": s.dcp_profiles}))
        ),
        cmd!(
            "profile.deleteCamera",
            "Delete Imported Camera Profile",
            [],
            None,
            "{id: dcp:…} — photos using it develop with their file's own colour again",
            always,
            delete
        ),
    ]
}

// ------------------------------------------------------------------------------------------------
// Camera bodies: a profile made on (or adopted for) a body's serial number is that body's default
// ------------------------------------------------------------------------------------------------

/// A serial number as it is compared: trimmed.
fn norm_serial(s: &str) -> String {
    s.trim().to_string()
}

/// How the UI names a body: `…71` (a serial number's last two characters), or `SA` for a body known
/// by its file names (`file:SA`, see [`crate::chart_profiling::body_key`]).
pub fn body_label(body: &str) -> String {
    let suffix = crate::chart_profiling::body_suffix(Some(body)).unwrap_or_default();
    if is_file_body(body) { suffix } else { format!("…{suffix}") }
}

/// A body known by its file names, not a serial number.
pub fn is_file_body(body: &str) -> bool {
    body.trim().starts_with("file:")
}

/// Give photo `id`'s catalog entry the body serial read from its file, when it has none (photos
/// imported before serials were kept). Not an edit: no undo step, but saved with the library.
pub fn backfill_serial(s: &mut Session, id: lightcraft_catalog::PhotoId, serial: &str) -> bool {
    let serial = norm_serial(serial);
    let Some(p) = s.catalog.photo(id) else { return false };
    if serial.is_empty() || !p.meta.serial.trim().is_empty() {
        return false;
    }
    let mut meta = p.meta.clone();
    meta.serial = serial;
    s.record_quietly(lightcraft_catalog::Op::SetMeta { id, meta: Box::new(meta) }).is_ok()
}

/// A photo's body as the catalog knows it (its serial, else its file name's key): cheap.
pub fn catalog_body(p: &lightcraft_catalog::Photo) -> Option<String> {
    crate::chart_profiling::body_key(Some(&p.meta.serial), Some(&p.file_name))
}

/// The keys a photo's body is known by in the catalog: its serial when the catalog has it, else its
/// file name's key ([`catalog_body`]). A photo whose serial isn't known yet (a library from before
/// serials were kept) learns it ([`backfill_serial`]) before its profile's body is judged.
pub fn catalog_keys(p: &lightcraft_catalog::Photo) -> Vec<String> {
    catalog_body(p).into_iter().collect()
}

/// [`relation`] for a photo known by several keys ([`catalog_keys`]): its own when any matches.
pub fn relation_keys(e: &DcpEntry, keys: &[String]) -> &'static str {
    match keys.iter().find(|k| e.bodies.contains(&norm_serial(k))) {
        Some(k) => relation(e, Some(k)),
        None => relation(e, keys.first().map(String::as_str)),
    }
}

/// [`body_default_for`] over every key of a photo's body (the first that has one).
fn body_default_keys(s: &Session, camera: &str, keys: &[String]) -> Option<String> {
    keys.iter().find_map(|k| body_default_for(s, camera, k))
}

/// The file a photo was imported from, when it is a file.
pub fn photo_file(s: &Session, id: lightcraft_catalog::PhotoId) -> Option<PathBuf> {
    match &s.catalog.photo(id)?.source {
        lightcraft_catalog::Source::File { path } => Some(PathBuf::from(path)),
        _ => None,
    }
}

/// The body serial number in a photo file's metadata (reads the file: off the UI thread).
pub fn read_serial(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    lightcraft_meta::extract(&bytes).serial_number.map(|s| norm_serial(&s)).filter(|s| !s.is_empty())
}

/// Whether profile `e` (made for a camera model named as raw converters name it) is for a photo
/// whose camera is `camera` (`Make Model` as the file has it).
pub fn same_camera(e: &DcpEntry, camera: &str) -> bool {
    let model = e.camera_model.split_once(' ').map_or(e.camera_model.as_str(), |(_, m)| m).trim().to_lowercase();
    let camera = camera.to_lowercase();
    !model.is_empty() && (camera.contains(&model) || e.camera_model.to_lowercase() == camera.trim())
}

/// Old entries kept one `serial`; now a profile has a list of bodies.
pub fn migrate(e: &mut DcpEntry) {
    if let Some(s) = e.serial.take().map(|s| norm_serial(&s)).filter(|s| !s.is_empty())
        && !e.bodies.contains(&s)
    {
        e.bodies.push(s);
    }
}

/// The profile body `serial` of `camera` develops with by default: among the profiles listing the
/// body, the one marked its default, else a dual-illuminant one, else the newest.
pub fn body_default_for(s: &Session, camera: &str, serial: &str) -> Option<String> {
    let serial = norm_serial(serial);
    if serial.is_empty() {
        return None;
    }
    // made the body's default explicitly: also a profile for every body of the model
    if let Some(e) = s.dcp_profiles.iter().rev().find(|e| e.body_defaults.contains(&serial) && same_camera(e, camera)) {
        return Some(e.id.clone());
    }
    let mine: Vec<&DcpEntry> = s.dcp_profiles.iter().filter(|e| e.bodies.contains(&serial) && same_camera(e, camera)).collect();
    let dual = |e: &&&DcpEntry| s.media.camera_profiles.get(&e.id).is_some_and(|p| p.color.color_matrix[1].is_some());
    mine.iter().rev().find(dual).or(mine.last()).map(|e| e.id.clone())
}

/// A photo's body: its serial number (the catalog's, else read from its file, for photos imported
/// before serials were kept), else its file name's key ([`crate::chart_profiling::body_key`]).
pub fn photo_serial(s: &Session, id: lightcraft_catalog::PhotoId) -> Option<String> {
    let p = s.catalog.photo(id)?;
    if !p.meta.serial.trim().is_empty() {
        return Some(norm_serial(&p.meta.serial));
    }
    let from_file = match &p.source {
        lightcraft_catalog::Source::File { path } => std::fs::read(path).ok().and_then(|b| lightcraft_meta::extract(&b).serial_number),
        _ => None,
    };
    crate::chart_profiling::body_key(from_file.as_deref(), Some(&p.file_name))
}

/// At import (and for `profile.applyBodyDefaults`): the photo's body default, when it still has the
/// default profile. Nothing else is ever replaced.
pub fn auto_pick(s: &Session, p: &mut lightcraft_catalog::Photo) -> bool {
    // (only a calibration never chosen: LightCraft's own colour chosen explicitly stays)
    if !p.develop.camera_profile.is_empty() || p.develop.look_is_camera_profile() || !p.develops_raw() {
        return false;
    }
    // (its serial number when the file has one, else its file name's key: a real serial wins)
    let Some(id) = body_default_keys(s, &p.meta.camera, &catalog_keys(p)) else { return false };
    let mut d = (*p.develop).clone();
    choose(s, &mut d, &id);
    p.develop = Arc::new(d);
    true
}

/// How profile `e` relates to a photo of body `serial`: its own (`body`), model-wide (`model`), or
/// made for other bodies (`otherBody`).
pub fn relation(e: &DcpEntry, serial: Option<&str>) -> &'static str {
    match serial.map(norm_serial).filter(|s| !s.is_empty()) {
        _ if e.bodies.is_empty() => "model",
        Some(s) if e.bodies.contains(&s) => "body",
        Some(_) => "otherBody",
        // a photo without a serial sees every profile of its model
        None => "model",
    }
}

/// Change entry `id` as one undoable step (the file is kept as it is).
fn edit_entry(s: &mut Session, c: &str, id: &str, label: &str, f: impl FnOnce(&mut DcpEntry) -> Result<()>) -> Result<DcpEntry> {
    let i = s.dcp_profiles.iter().position(|e| e.id == id).ok_or_else(|| bad(c, format!("no camera profile `{id}`")))?;
    let before = s.dcp_profiles.get(i).cloned().ok_or_else(|| bad(c, "no such profile"))?;
    let bytes = std::fs::read(&before.file).map_err(|e| bad(c, format!("{}: {e}", before.file)))?;
    let mut after = before.clone();
    f(&mut after)?;
    if let Some(slot) = s.dcp_profiles.get_mut(i) {
        *slot = after.clone();
    }
    s.save_prefs()?;
    s.commit_profile_swap(label, ProfileSwap { entry: before, bytes })?;
    Ok(after)
}

/// `{serial}` or `{photo}` (its body).
fn serial_param(s: &Session, p: &Value, c: &str) -> Result<String> {
    if let Some(v) = str_param(p, "serial").map(norm_serial).filter(|v| !v.is_empty()) {
        return Ok(v);
    }
    let id = p
        .get("photo")
        .and_then(Value::as_u64)
        .map(lightcraft_catalog::PhotoId)
        .or_else(|| s.active())
        .ok_or_else(|| bad(c, "give `serial` or `photo`"))?;
    photo_serial(s, id)
        .ok_or_else(|| bad(c, "the photo's file doesn't say which camera body took it (no serial number, and its file name gives no body)"))
}

/// After [`serial_param`] read a photo's serial from its file: the catalog keeps it, so the photo
/// matches what was just adopted or made default (libraries from before serials were kept).
fn remember_serial(s: &mut Session, p: &Value, serial: &str) {
    if str_param(p, "serial").is_some() || is_file_body(serial) {
        return;
    }
    if let Some(id) = p.get("photo").and_then(Value::as_u64).map(lightcraft_catalog::PhotoId).or_else(|| s.active()) {
        backfill_serial(s, id, serial);
    }
}

fn adopt(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.adoptForBody";
    let id = str_param(p, "profile").ok_or_else(|| bad(C, "missing `profile`"))?.to_string();
    let serial = serial_param(s, p, C)?;
    remember_serial(s, p, &serial);
    let make_default = p.get("makeDefault").and_then(Value::as_bool).unwrap_or(false);
    let label = format!("Adopt Camera Profile for Body {}", body_label(&serial));
    let e = edit_entry(s, C, &id, &label, |e| {
        if !e.bodies.contains(&serial) && !e.bodies.is_empty() {
            e.bodies.push(serial.clone());
        }
        if make_default && !e.body_defaults.contains(&serial) {
            e.body_defaults.push(serial.clone());
        }
        Ok(())
    })?;
    if make_default {
        // one default per body
        for other in s.dcp_profiles.iter_mut().filter(|o| o.id != id) {
            other.body_defaults.retain(|b| b != &serial);
        }
        s.save_prefs()?;
    }
    Ok(json!({"profile": e.id, "bodies": e.bodies, "bodyDefaults": e.body_defaults}))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.releaseFromBody";
    let id = str_param(p, "profile").ok_or_else(|| bad(C, "missing `profile`"))?.to_string();
    let serial = serial_param(s, p, C)?;
    let label = format!("Remove Camera Profile from Body {}", body_label(&serial));
    let e = edit_entry(s, C, &id, &label, |e| {
        if !e.bodies.contains(&serial) {
            return Err(bad(C, format!("the profile isn't for body {}", body_label(&serial))));
        }
        if e.bodies.len() == 1 {
            return Err(bad(C, format!("{} is the only body this profile is for: delete the profile instead", body_label(&serial))));
        }
        e.bodies.retain(|b| b != &serial);
        e.body_defaults.retain(|b| b != &serial);
        Ok(())
    })?;
    Ok(json!({"profile": e.id, "bodies": e.bodies}))
}

fn set_body_default(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.setBodyDefault";
    let id = str_param(p, "profile").ok_or_else(|| bad(C, "missing `profile`"))?.to_string();
    let serial = serial_param(s, p, C)?;
    remember_serial(s, p, &serial);
    let label = format!("Default Camera Profile for Body {}", body_label(&serial));
    let others: Vec<(usize, DcpEntry)> =
        s.dcp_profiles.iter().cloned().enumerate().filter(|(_, o)| o.id != id && o.body_defaults.contains(&serial)).collect();
    let e = edit_entry(s, C, &id, &label, |e| {
        if !e.bodies.is_empty() && !e.bodies.contains(&serial) {
            e.bodies.push(serial.clone());
        }
        if !e.body_defaults.contains(&serial) {
            e.body_defaults.push(serial.clone());
        }
        Ok(())
    })?;
    for (i, mut o) in others {
        o.body_defaults.retain(|b| b != &serial);
        if let Some(slot) = s.dcp_profiles.get_mut(i) {
            *slot = o;
        }
    }
    s.save_prefs()?;
    Ok(json!({"profile": e.id, "bodies": e.bodies, "bodyDefaults": e.body_defaults}))
}

/// `profile.applyBodyDefaults`: photos still on the default profile take their body's profile.
fn apply_body_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    let mut ids = s.targets(p)?;
    if ids.is_empty() {
        ids = s.visible_cloned();
    }
    let before = s.commits();
    let (mut changed, mut skipped) = (Vec::new(), 0usize);
    for id in ids {
        let Some(mut photo) = s.catalog.photo(id).map(|p| (**p).clone()) else { continue };
        if photo.meta.serial.trim().is_empty()
            && let Some(serial) = photo_serial(s, id).filter(|b| !is_file_body(b))
        {
            photo.meta.serial = serial;
        }
        if auto_pick(s, &mut photo) {
            s.set_develop(id, (*photo.develop).clone(), "Profile")?;
            changed.push(id.0);
        } else {
            skipped += 1;
        }
    }
    let n = usize::try_from(s.commits().wrapping_sub(before)).unwrap_or(0);
    s.merge_undo(n, "Apply Camera Body Profiles");
    Ok(json!({"changed": changed, "skipped": skipped}))
}

/// Camera profiles for a photo (`profiles.forPhoto {id?}`): each with how it relates to the photo's
/// body ([`relation`]), the photo's own and model-wide first.
fn for_photo(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("id").and_then(Value::as_u64).map(lightcraft_catalog::PhotoId).or_else(|| s.active());
    let serial = id.and_then(|i| photo_serial(s, i));
    let camera = id.and_then(|i| s.catalog.photo(i)).map(|p| p.meta.camera.clone()).unwrap_or_default();
    let default = serial.as_deref().and_then(|sn| body_default_for(s, &camera, sn));
    let list: Vec<Value> = s
        .dcp_profiles
        .iter()
        .filter(|e| camera.is_empty() || same_camera(e, &camera))
        .map(|e| {
            json!({"id": e.id, "name": e.name, "bodies": e.bodies, "relation": relation(e, serial.as_deref()),
                "bodyDefault": default.as_deref() == Some(e.id.as_str())})
        })
        .collect();
    Ok(json!({"serial": serial, "profiles": list}))
}

pub fn body_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "import.profilePlan",
            "Camera Profiles for Import",
            [],
            None,
            "{paths: [file or folder], cameraProfile?: as library.import} → {rows: [{camera (as people name it: Canon EOS R5 C), body (serial, else file:XX), bodyLabel, profile (null: LightCraft's colour), profileName, how: auto | chosen | all, own, photos, fromSidecar}], photos, keptFromSidecar, duplicates (raws already in this catalog, not counted in photos), copiesInSource (raws whose bytes repeat another file of the source)}: the camera profile each body's raws would get at import (new raws only)",
            always,
            crate::import_profiles::plan_command
        ),
        cmd!(
            query "profiles.forPhoto",
            "Camera Profiles for Photo",
            [],
            None,
            "{id?} → {serial, profiles: [{id, name, bodies, relation: body | model | otherBody, bodyDefault}]}: the photo's camera's profiles and how each relates to the photo's body",
            always,
            for_photo
        ),
        cmd!(
            "profile.applyBodyDefaults",
            "Apply Camera Body Profiles",
            ["Photo"],
            None,
            "{ids?} (default: the selection, else the photos in view) — photos still on the default profile take their camera body's own profile (made on or adopted for its serial number); one undo step → {changed: [ids], skipped}",
            always,
            apply_body_defaults
        ),
        cmd!(
            "profile.adoptForBody",
            "Adopt Camera Profile for Body",
            [],
            None,
            "{profile: dcp:…, serial? | photo? (default: the active photo's body), makeDefault?: bool} — the profile becomes one of the body's own (no file is copied); undoable",
            always,
            adopt
        ),
        cmd!(
            "profile.releaseFromBody",
            "Remove Camera Profile from Body",
            [],
            None,
            "{profile: dcp:…, serial? | photo?} — the profile is no longer the body's (refused for its last body); undoable",
            always,
            release
        ),
        cmd!(
            "profile.setBodyDefault",
            "Use Camera Profile as Body Default",
            [],
            None,
            "{profile: dcp:…, serial? | photo?} — the profile is the body's default (over a dual-illuminant or newer one); undoable",
            always,
            set_body_default
        ),
    ]
}

/// A photo's camera profile seen from its body: `(relation, body serial, is the body's default)`
/// when the photo develops with a camera profile made in or adopted by LightCraft and its file has
/// a serial (from the catalog only: cheap enough for every frame).
pub fn photo_profile_status(s: &Session, photo: lightcraft_catalog::PhotoId) -> Option<(&'static str, String, bool)> {
    let p = s.catalog.photo(photo)?;
    let keys = catalog_keys(p);
    let serial = catalog_body(p)?;
    let e = s.dcp_profiles.iter().find(|e| Some(e.id.as_str()) == p.develop.calibration_id())?;
    let is_default = body_default_keys(s, &p.meta.camera, &keys).as_deref() == Some(e.id.as_str());
    Some((relation_keys(e, &keys), serial, is_default))
}

/// The camera profiles a photo's profile menu lists (`(id, name, body label)`): its body's own and
/// model-wide ones; with `others`, also those made for other bodies (with their bodies' labels).
pub fn menu_for_photo(
    s: &Session,
    photo: Option<lightcraft_catalog::PhotoId>,
    others: bool,
) -> (Vec<(String, String)>, Vec<(String, String, String)>) {
    let keys = photo.and_then(|id| s.catalog.photo(id)).map(|p| catalog_keys(p)).unwrap_or_default();
    let (mut mine, mut theirs) = (Vec::new(), Vec::new());
    for e in &s.dcp_profiles {
        match relation_keys(e, &keys) {
            "otherBody" => {
                if others {
                    let label = e.bodies.iter().map(|b| body_label(b)).collect::<Vec<_>>().join(", ");
                    theirs.push((e.id.clone(), e.name.clone(), label));
                }
            }
            _ => mine.push((e.id.clone(), e.name.clone())),
        }
    }
    (mine, theirs)
}

/// How many camera profiles a photo's menu hides (made for other bodies).
pub fn hidden_for_photo(s: &Session, photo: Option<lightcraft_catalog::PhotoId>) -> usize {
    let keys = photo.and_then(|id| s.catalog.photo(id)).map(|p| catalog_keys(p)).unwrap_or_default();
    s.dcp_profiles.iter().filter(|e| relation_keys(e, &keys) == "otherBody").count()
}

/// A Lightroom sidecar names camera profile `name` (`crs:CameraProfile`): when the user has a
/// camera profile of that name for the photo's camera, it becomes the photo's camera calibration
/// (its look stays). Nothing changes otherwise, or when the photo has a calibration already.
pub fn from_xmp_name(s: &Session, p: &mut lightcraft_catalog::Photo, name: &str) -> bool {
    if p.develop.calibration_id().is_some() {
        return false;
    }
    let Some(e) = s.dcp_profiles.iter().find(|e| e.name == name.trim() && same_camera(e, &p.meta.camera)) else { return false };
    let mut d = (*p.develop).clone();
    choose(s, &mut d, &e.id);
    p.develop = Arc::new(d);
    true
}
