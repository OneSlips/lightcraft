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
}

/// Is `id` an imported camera profile id?
pub fn is_dcp_id(id: &str) -> bool {
    id.starts_with(DCP_PREFIX)
}

fn profiles_dir(s: &Session) -> Option<PathBuf> {
    s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join("Profiles"))
}

/// Parse every imported camera profile again and hand them to the decoder (missing or damaged
/// files are skipped: photos using them develop with the file's own colour).
pub fn register_all(s: &mut Session) {
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
        s.dcp_profiles.push(DcpEntry { id: id.clone(), name: name.clone(), camera_model: profile.camera_model.clone(), file });
        imported.push(json!({"id": id, "name": name, "cameraModel": profile.camera_model}));
    }
    Ok((imported, failed))
}

/// File a camera profile made in LightCraft (`profile.createFromChart`) among the imported ones: its
/// `.dcp` goes into the library's `Profiles/` folder (the temp folder without a library on disk). A
/// profile with the same name and camera as one there is named "… 2", "… 3": an existing profile is
/// never rewritten under photos that use it.
pub fn add_profile(s: &mut Session, mut profile: DcpProfile, c: &str) -> Result<DcpEntry> {
    let base = if profile.name.trim().is_empty() { "Camera Profile".to_string() } else { profile.name.trim().to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while s.dcp_profiles.iter().any(|p| p.name == name && p.camera_model == profile.camera_model) {
        name = format!("{base} {n}");
        n += 1;
    }
    profile.name = name.clone();
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
    let to = dir.join(format!("{}.dcp", id.trim_start_matches(DCP_PREFIX)));
    std::fs::write(&to, &bytes).map_err(|e| bad(c, format!("{}: {e}", to.display())))?;
    let entry = DcpEntry { id: id.clone(), name, camera_model: profile.camera_model.clone(), file: to.to_string_lossy().to_string() };
    s.media.camera_profiles.insert(id, Arc::new(profile));
    s.dcp_profiles.push(entry.clone());
    Ok(entry)
}

/// The chart photograph a command works on: `path`, else the target photo's file.
fn chart_source(s: &Session, p: &Value, c: &str) -> Result<(Option<lightcraft_catalog::PhotoId>, String)> {
    if let Some(path) = str_param(p, "path") {
        return Ok((None, path.to_string()));
    }
    let id = s.targets(p).into_iter().next().ok_or_else(|| bad(c, "no photo: select the chart photo or give `id` or `path`"))?;
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
    let found = prep
        .located
        .map(|l| json!({"corners": crate::chart_profiling::normalized_corners(&prep.frame, &l.corners), "turns": l.turns, "score": l.score}));
    Ok(json!({
        "photo": id.map(|i| i.0),
        "path": path,
        "cameraModel": prep.camera_model,
        "frame": [prep.frame.width, prep.frame.height],
        "found": found.is_some(),
        "chart": found,
    }))
}

fn create_from_chart(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.createFromChart";
    let (id, path) = chart_source(s, p, C)?;
    let corners = corners_param(p, C)?;
    let turns = p.get("turns").and_then(Value::as_u64).map(|t| (t % 4) as u8);
    let natural = match str_param(p, "rendering").unwrap_or("accurate") {
        "accurate" => false,
        "natural" => true,
        other => return Err(bad(C, format!("unknown rendering `{other}` (accurate or natural)"))),
    };
    let options = lightcraft_camprof::BuildOptions {
        illuminant: str_param(p, "illuminant").unwrap_or("D55").to_string(),
        natural,
        correction_table: p.get("correctionTable").and_then(Value::as_bool).unwrap_or(true),
        camera_model: str_param(p, "cameraModel").unwrap_or("").to_string(),
        name: str_param(p, "name").unwrap_or("").to_string(),
    };
    let req = crate::chart_profiling::Request { corners, turns, refine: p.get("refine").and_then(Value::as_bool).unwrap_or(false), options };
    let task = s.activity.start("cameraProfile", "Creating camera profile", crate::activity::Cancel::No);
    task.detail(&Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let prep = crate::chart_profiling::prepare_path(&path).map_err(|e| bad(C, e))?;
    let built = crate::chart_profiling::build(&prep, &req).map_err(|e| bad(C, e))?;
    drop(task);
    let apply = p.get("apply").and_then(Value::as_bool).unwrap_or(true);
    let installed = crate::chart_profiling::install(s, &built, id.filter(|_| apply))?;
    let mut out = crate::chart_profiling::report_json(&prep, &built);
    if let (Some(o), Some(i)) = (out.as_object_mut(), installed.as_object()) {
        for (k, v) in i {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok(out)
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
            "{id? | path?: a raw photo of a ColorChecker 24} → {photo, path, cameraModel, frame: [w, h], found, chart: {corners: [[x, y] ×4], turns, score}}: corners are 0–1 in the raw frame as the sensor recorded it (not rotated), top left, top right, bottom right, bottom left; turns 0–3 = patch A01 at the top left, top right, bottom right, bottom left",
            always,
            find_chart
        ),
        cmd!(
            "profile.createFromChart",
            "Create Camera Profile from Color Chart",
            [],
            None,
            "{id? | path?, corners?: [[x, y] ×4] (0–1, as profile.findChart; found automatically when omitted), turns?: 0–3 (decided by fitting when omitted), refine?: bool, illuminant?: D50 | D55 (default) | D65 | D75 | A, rendering?: accurate (default) | natural, correctionTable?: bool (default true), name?, cameraModel? (default: the file's), apply?: bool (default true: the photo develops with the new profile)} — fits a DNG camera profile (ColorMatrix, ForwardMatrix, HueSatMap) to a ColorChecker 24 and adds it to the camera profiles (dcp: id) → {id, name, cameraModel, file, applied, meanDeltaE, maxDeltaE, matrixMeanDeltaE, patchesUsed, patches: [{id, name, deltaE, matrixDeltaE, referenceLab, renderedLab}], corners, turns, notes} (ΔE = CIEDE2000)",
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
