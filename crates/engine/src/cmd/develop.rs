//! Develop commands. All operate on the active photo unless `ids` are given (sync).

use lightcraft_catalog::{Op, PhotoId, Version};
use lightcraft_develop::{DevelopSettings, Preset, ProcessVersion, Section, SettingsGroup, SettingsPick, Treatment, Upright, WbMode, controls};
use lightcraft_geom::{CropGeometry, CropHandle, Point, Rect, crop_fit_angle, drag_crop};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, f64_or, f64_req, has_active, has_clipboard, has_selection, index_req, ok, str_param};
use crate::{Result, Session, media::SourceLevel};

fn active(s: &Session, c: &str) -> Result<PhotoId> {
    s.active().ok_or_else(|| bad(c, "no active photo"))
}

/// Modify the active photo's settings with `f` and commit (or preview during an interaction).
fn edit(s: &mut Session, c: &str, label: &str, f: impl FnOnce(&mut DevelopSettings) -> Result<()>) -> Result<Value> {
    let id = active(s, c)?;
    let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
    f(&mut d)?;
    s.set_develop(id, d, label)?;
    ok()
}

/// `d` (photo with source `info`, crop frame `dims`) with control `ctl` moved by `delta` from its
/// own value: white balance on the photo's own scale (As Shot read as the file's white first),
/// clamped to the control's range. Quick Develop and the live multi-photo edit use it, so every
/// photo moves by the same amount whatever its starting point.
fn nudge(info: &lightcraft_pipeline::SourceInfo, dims: (f64, f64), d: &DevelopSettings, ctl: &str, delta: f64, default: f64) -> DevelopSettings {
    let mut nd = d.clone();
    let wb = ctl == "wb.temp" || ctl == "wb.tint";
    if wb {
        lightcraft_pipeline::local::normalize_wb(&mut nd, info);
        if nd.wb.mode == WbMode::AsShot {
            nd.wb.temp = info.as_shot_temp;
            nd.wb.tint = info.as_shot_tint;
        }
    }
    let before = nd.crop.geometry;
    let v = controls::get(&nd, ctl).unwrap_or(default);
    controls::set(&mut nd, ctl, v + delta);
    refit_crop_angle(ctl == CROP_ANGLE, before, &mut nd, dims);
    if wb {
        nd.wb.mode = WbMode::Custom;
    }
    nd
}

/// `d` (photo with source `oinfo`, crop frame `dims`) with control `ctl` set to `value`, the value
/// the active photo (source `ainfo`) now has (Auto Sync and the live multi-photo edit: the same
/// value on every photo, note 18). White balance on the photo's own scale: the same Kelvin / tint
/// when both are on the same scale (an old relative value converted first, never read as Kelvin);
/// across scales (a JPEG next to a raw) there is no same number, and the photo moves by `delta`,
/// as much as the active photo did. A crop angle is fitted to the photo's own crop.
fn same_value(
    ainfo: &lightcraft_pipeline::SourceInfo,
    oinfo: &lightcraft_pipeline::SourceInfo,
    dims: (f64, f64),
    d: &DevelopSettings,
    ctl: &str,
    value: f64,
    delta: f64,
) -> DevelopSettings {
    let wb = ctl == "wb.temp" || ctl == "wb.tint";
    if wb && crate::sync_same::kelvin(ainfo) != crate::sync_same::kelvin(oinfo) {
        let default = controls::find(ctl).map_or(0.0, |c| c.default);
        return if delta.is_finite() { nudge(oinfo, dims, d, ctl, delta, default) } else { d.clone() };
    }
    let mut nd = d.clone();
    if wb {
        lightcraft_pipeline::local::normalize_wb(&mut nd, oinfo);
        // the white in effect (As Shot: the file's own; a light preset: its numbers) is where the other slider stays
        let (t, tint) = lightcraft_pipeline::local::effective_wb(oinfo, d);
        nd.wb.temp = t;
        nd.wb.tint = tint;
    }
    let before = nd.crop.geometry;
    controls::set(&mut nd, ctl, value);
    refit_crop_angle(ctl == CROP_ANGLE, before, &mut nd, dims);
    if wb {
        nd.wb.mode = WbMode::Custom;
    }
    nd
}

/// The value control `ctl` has on a photo as its sliders show it: white balance as the Kelvin and
/// tint in effect (As Shot reads the file's own), everything else as stored. What a live
/// multi-photo edit measures the active photo's move from.
fn own_value(info: &lightcraft_pipeline::SourceInfo, d: &DevelopSettings, ctl: &str) -> f64 {
    if ctl == "wb.temp" || ctl == "wb.tint" {
        let (temp, tint) = lightcraft_pipeline::local::effective_wb(info, d);
        return if ctl == "wb.temp" { temp } else { tint };
    }
    controls::get(d, ctl).unwrap_or_else(|| controls::find(ctl).map_or(0.0, |c| c.default))
}

fn section_param(p: &Value, c: &str) -> Result<Section> {
    let v = p.get("section").cloned().ok_or_else(|| bad(c, "missing `section`"))?;
    serde_json::from_value(v).map_err(|e| bad(c, e.to_string()))
}

fn groups_param(p: &Value, c: &str) -> Result<Option<Vec<SettingsGroup>>> {
    match p.get("groups") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| bad(c, format!("invalid `groups`: {e}"))),
    }
}

/// `groups` (whole groups) and `settings` (single settings of a group: `light.exposure`,
/// `detail.sharpening`…; `develop.settingItems` lists them) as one pick; `None` when neither is given.
fn pick_param(p: &Value, c: &str) -> Result<Option<SettingsPick>> {
    let groups = groups_param(p, c)?;
    let settings: Option<Vec<String>> = match p.get("settings") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| bad(c, format!("invalid `settings`: {e}")))?),
    };
    if groups.is_none() && settings.is_none() {
        return Ok(None);
    }
    let settings = settings.unwrap_or_default();
    if let Some(unknown) = settings.iter().find(|id| lightcraft_develop::presets::find_item(id).is_none()) {
        return Err(bad(c, format!("unknown setting `{unknown}` (see develop.settingItems)")));
    }
    Ok(Some(SettingsPick { groups: groups.unwrap_or_default(), settings }.normalized()))
}

/// Remember the dialogs' choices app-wide (saved with the store; a failed save is logged, the
/// command itself went through).
fn remember_dialogs(s: &mut Session) {
    if let Err(e) = s.save_prefs() {
        log::error!("dialogs: remembering the choices: {e}");
    }
}

pub(crate) fn crop_dims(s: &Session, id: PhotoId) -> (f64, f64) {
    let p = s.catalog.photo(id);
    let (w, h) = p.map(|p| (p.width.max(1) as f64, p.height.max(1) as f64)).unwrap_or((3.0, 2.0));
    let o = p.map(|p| p.develop.orientation).unwrap_or_default();
    if o.swaps_axes() { (h, w) } else { (w, h) }
}

/// After a generic control write (`develop.set`, `develop.adjust`, …) that touched `crop.angle`:
/// refit the crop as `crop.straighten` does, from the crop it had `before`, so the box never takes in
/// canvas outside the photo (issue #742).
fn refit_crop_angle(touched: bool, before: CropGeometry, d: &mut DevelopSettings, (w, h): (f64, f64)) {
    if touched {
        d.crop.geometry = before.with_angle(d.crop.geometry.angle, w, h);
    }
}

const CROP_ANGLE: &str = "crop.angle";

/// `v` as a list of numbers; `None` unless it is an array of nothing but numbers.
fn numbers(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(Value::as_f64).collect()
}

/// A width × height lock stored as hundredths. Rejects non-finite, non-positive and
/// extreme (beyond 20:1) shapes, which would give an empty or unusable crop.
fn aspect_tuple(c: &str, x: f64, y: f64) -> Result<(u32, u32)> {
    let ratio = x / y;
    if !(x.is_finite()
        && y.is_finite()
        && x > 0.0
        && y > 0.0
        && ratio.is_finite()
        && (1.0 / lightcraft_geom::MAX_RATIO..=lightcraft_geom::MAX_RATIO).contains(&ratio))
    {
        return Err(bad(c, "aspect needs two positive numbers with a ratio between 1:20 and 20:1"));
    }
    let to = |v: f64| (v * 100.0).round();
    let (a, b) = (to(x), to(y));
    if a < 1.0 || b < 1.0 || a > f64::from(u32::MAX) || b > f64::from(u32::MAX) {
        return Err(bad(c, "aspect numbers are out of range"));
    }
    Ok((a as u32, b as u32))
}

/// How a new aspect ratio is turned (issue #2253).
#[derive(Clone, Copy, PartialEq)]
enum Turn {
    /// As the crop is now: a named ratio ("2x3", "original"…) lands landscape on a landscape crop
    /// and portrait on a portrait one (after X), as in Lightroom. A square crop follows the image.
    LikeCrop,
    /// As given: a custom `[w, h]` taller than wide is a portrait crop.
    AsGiven,
}

fn set_aspect(d: &mut DevelopSettings, w: f64, h: f64, aspect: Option<(u32, u32)>, turn: Turn) {
    let r = d.crop.geometry.rect_px(w, h);
    let landscape = if (r.width() - r.height()).abs() > 1e-6 * r.width().max(r.height()) { r.width() > r.height() } else { w >= h };
    let aspect = aspect.map(|(aw, ah)| if turn == Turn::LikeCrop && (aw >= ah) != landscape { (ah, aw) } else { (aw, ah) });
    d.crop.aspect = aspect;
    let a = aspect.map(|(aw, ah)| aw as f64 / ah as f64);
    let angle = d.crop.geometry.angle;
    d.crop.geometry = crop_fit_angle(w, h, angle, a);
}

/// Photo `id` is on an older rendering process than [`ProcessVersion::LATEST`].
fn outdated(s: &Session, id: PhotoId) -> bool {
    s.catalog.photo(id).is_some_and(|p| p.develop.process.is_outdated())
}

fn already_current() -> String {
    format!("already on the current process (version {})", ProcessVersion::LATEST.0)
}

/// Update to Current Process applies to the selection (the selected photos, else the active
/// photo): one of them is on an older rendering process. Calls that name their photos are
/// checked by the command instead (`explicit_targets`).
fn has_outdated_process(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let sel = &s.selection;
    let targets = if sel.ids.is_empty() { sel.active.as_slice() } else { sel.ids.as_slice() };
    if targets.iter().any(|id| outdated(s, *id)) { Ok(()) } else { Err(already_current()) }
}

/// The photos a call names (`ids`, or `id`), each of which must be in the library; `None` when it
/// names none (the selection applies).
fn named_photos(s: &Session, p: &Value, c: &str) -> Result<Option<Vec<PhotoId>>> {
    if !super::names_photos(p) {
        return Ok(None);
    }
    let given = match p.get("ids").filter(|v| !v.is_null()) {
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(bad(c, "`ids` must be a list of photo ids")),
        None => p.get("id").cloned().into_iter().collect(),
    };
    if given.is_empty() {
        return Err(bad(c, "`ids` names no photos"));
    }
    let ids = given
        .iter()
        .map(|v| match v.as_u64().map(PhotoId) {
            Some(id) if s.catalog.photo(id).is_some() => Ok(id),
            _ => Err(bad(c, format!("no such photo {v}"))),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(ids))
}

/// The user preset named by `p.id` (built-ins can't be changed).
fn user_preset<'a>(s: &'a mut Session, p: &Value, c: &str) -> Result<&'a mut Preset> {
    let pid = str_param(p, "id").ok_or_else(|| bad(c, "missing id"))?;
    let pr = s.presets.iter_mut().find(|x| x.id == pid).ok_or_else(|| bad(c, format!("unknown preset `{pid}`")))?;
    if pr.builtin {
        return Err(bad(c, "built-in presets can't be changed"));
    }
    Ok(pr)
}

/// Change version `p.index` of the active photo (`f` gets it and the current settings).
fn edit_version(s: &mut Session, p: &Value, c: &str, label: &str, f: impl FnOnce(&mut Version, std::sync::Arc<DevelopSettings>)) -> Result<Value> {
    let id = active(s, c)?;
    let i = index_req(p, "index", c)?;
    let ph = s.catalog.photo(id).ok_or_else(|| bad(c, "no photo"))?;
    let mut versions = ph.versions.clone();
    let v = versions.get_mut(i).ok_or_else(|| bad(c, "no such version"))?;
    f(v, ph.develop.clone());
    s.commit(label, Op::SetVersions { id, versions })?;
    ok()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "develop.set",
            "Set Develop Values",
            [],
            None,
            "{control?: controlId, value?: number, values?: {controlId: number}, ids?: [photo]} — see develop.controls; `crop.angle` turns the crop as crop.straighten does (kept if it still fits, else the largest of its aspect inside the photo)",
            has_active,
            |s, p| {
                let mut vals: Vec<(String, f64)> = Vec::new();
                if let (Some(c), Some(v)) = (str_param(p, "control"), p.get("value").and_then(Value::as_f64)) {
                    vals.push((c.to_string(), v));
                }
                if let Some(o) = p.get("values").and_then(Value::as_object) {
                    for (k, v) in o {
                        vals.push((k.clone(), v.as_f64().ok_or_else(|| bad("develop.set", format!("`{k}` must be a number")))?));
                    }
                }
                if vals.is_empty() {
                    return Err(bad("develop.set", "give `control`+`value` or `values`"));
                }
                for (k, _) in &vals {
                    if controls::find(k).is_none() {
                        return Err(bad("develop.set", format!("unknown control `{k}`")));
                    }
                }
                let label = if vals.len() == 1 { controls::find(&vals[0].0).map(|c| c.label).unwrap_or("Edit").to_string() } else { "Edit".into() };
                let angle = vals.iter().any(|(k, _)| k == CROP_ANGLE);
                let apply = |d: &mut DevelopSettings, info: &lightcraft_pipeline::SourceInfo, dims: (f64, f64)| {
                    let before = d.crop.geometry;
                    if vals.iter().any(|(k, _)| k == "wb.temp" || k == "wb.tint") {
                        lightcraft_pipeline::local::normalize_wb(d, info);
                        if d.wb.mode == WbMode::AsShot {
                            d.wb.temp = info.as_shot_temp;
                            d.wb.tint = info.as_shot_tint;
                        }
                    }
                    for (k, v) in &vals {
                        controls::set(d, k, *v);
                        if k == "wb.temp" || k == "wb.tint" {
                            d.wb.mode = WbMode::Custom;
                        }
                    }
                    refit_crop_angle(angle, before, d, dims);
                };
                if let Some(ids) = p.get("ids").and_then(Value::as_array) {
                    let ids: Vec<PhotoId> = ids.iter().filter_map(Value::as_u64).map(PhotoId).collect();
                    let ops = ids
                        .iter()
                        .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                        .filter_map(|(id, d)| {
                            let mut d = (*d).clone();
                            let info = s.source_info(id);
                            apply(&mut d, &info, crop_dims(s, id));
                            s.develop_op(id, d, &label)
                        })
                        .collect();
                    s.commit(&label, Op::Batch { ops })?;
                    return ok();
                }
                let id = active(s, "develop.set")?;
                let info = s.source_info(id);
                let dims = crop_dims(s, id);
                // a multi-photo interaction (a slider dragged with Auto Sync on, the console's dials):
                // the others get the active photo's new value, live (`same_value`: white balance on
                // each photo's own scale). Outside a drag (a typed value, a reset, a key) Auto Sync
                // does the same in the edit's own undo step (`Session::set_develop`).
                let others = s.interaction_others();
                let before: Vec<(String, f64)> = if others.is_empty() {
                    Vec::new()
                } else {
                    let d = s.develop_of(id).unwrap_or_default();
                    vals.iter().map(|(k, _)| (k.clone(), own_value(&info, &d, k))).collect()
                };
                edit(s, "develop.set", &label, |d| {
                    apply(d, &info, dims);
                    Ok(())
                })?;
                if !others.is_empty() {
                    let now = s.develop_of(id).unwrap_or_default();
                    let moved: Vec<(String, f64, f64)> = vals
                        .iter()
                        .zip(&before)
                        .map(|((k, _), (_, was))| (k.clone(), own_value(&info, &now, k), *was))
                        .filter(|(_, v, was)| v.is_finite() && (v - was).abs() > 0.0)
                        .collect();
                    for other in &others {
                        let Some(mut d) = s.develop_of(*other).map(|d| (*d).clone()) else { continue };
                        let (oinfo, odims) = (s.source_info(*other), crop_dims(s, *other));
                        for (k, v, was) in &moved {
                            d = same_value(&info, &oinfo, odims, &d, k, *v, v - was);
                        }
                        s.set_develop_silent(*other, d, &label)?;
                    }
                }
                ok()
            }
        ),
        cmd!("develop.adjust", "Nudge Develop Value", [], None, "{control, delta}", has_active, |s, p| {
            let c = str_param(p, "control").ok_or_else(|| bad("develop.adjust", "missing control"))?.to_string();
            let spec = controls::find(&c).ok_or_else(|| bad("develop.adjust", "unknown control"))?;
            let delta = f64_req(p, "delta", "develop.adjust")?;
            let id = active(s, "develop.adjust")?;
            let (info, dims) = (s.source_info(id), crop_dims(s, id));
            edit(s, "develop.adjust", spec.label, |d| {
                *d = nudge(&info, dims, d, &c, delta, spec.default);
                Ok(())
            })?;
            // a multi-photo interaction (develop.beginInteraction {ids}: the console's dials, a
            // drag with Auto Sync on): the other photos get the active photo's new value, live
            let others = s.interaction_others();
            if !others.is_empty() {
                let now = s.develop_of(id).unwrap_or_default();
                let v = own_value(&info, &now, &c);
                for other in others {
                    let Some(d) = s.develop_of(other) else { continue };
                    let nd = same_value(&info, &s.source_info(other), crop_dims(s, other), &d, &c, v, delta);
                    s.set_develop_silent(other, nd, spec.label)?;
                }
            }
            ok()
        }),
        cmd!("develop.merge", "Apply Settings JSON", [], None, "{settings: partial DevelopSettings JSON, label?}", has_active, |s, p| {
            let partial = p.get("settings").cloned().ok_or_else(|| bad("develop.merge", "missing settings"))?;
            let label = str_param(p, "label").unwrap_or("Edit").to_string();
            edit(s, "develop.merge", &label, |d| {
                *d = d.merged(&partial).map_err(|e| bad("develop.merge", e.to_string()))?;
                Ok(())
            })
        }),
        cmd!("develop.reset", "Reset Edits", ["Photo"], Some("Cmd+Shift+R"), "{ids?}", has_selection, |s, p| {
            let ids = s.targets(p)?;
            let ops = ids
                .iter()
                .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                .filter_map(|(id, _d)| {
                    // back to the photo's import defaults when a default preset gave it its look
                    let look = s.catalog.photo(id).and_then(|p| p.import_look.clone());
                    let fresh = match look {
                        Some(l) => (*l).clone(),
                        None => {
                            // the photo's built-in defaults (raws: capture sharpening and colour noise
                            // reduction, embedded lens corrections), as at import, with the white balance
                            // of its current as-shot reference
                            let info = s.source_info(id);
                            let defaults = s.catalog.photo(id).map(|p| p.camera_defaults()).unwrap_or_default();
                            let mut fresh = DevelopSettings {
                                wb: lightcraft_develop::WhiteBalance {
                                    mode: WbMode::AsShot,
                                    temp: info.as_shot_temp,
                                    tint: info.as_shot_tint,
                                    ..Default::default()
                                },
                                ..defaults
                            };
                            lightcraft_pipeline::local::normalize_wb(&mut fresh, &info);
                            fresh
                        }
                    };
                    // a fresh start renders with the current process (a section or slider reset doesn't change it)
                    s.develop_op(id, DevelopSettings { process: ProcessVersion::LATEST, ..fresh }, "Reset")
                })
                .collect();
            s.commit("Reset", Op::Batch { ops })?;
            ok()
        }),
        CommandSpec {
            explicit_targets: true,
            ..cmd!(
                "develop.updateProcess",
                "Update to Current Process",
                ["Photo"],
                None,
                "{ids?}: moves the target photos (`ids`, else the selection) that are on an older rendering process to the current one, as one undo step (their look can change; the sliders keep their values; see docs/process-versions.md). Enabled while a selected photo is on an older process; named photos are checked themselves (each must exist, one must be on an older process) → {changed, process}",
                has_outdated_process,
                |s, p| {
                    const C: &str = "develop.updateProcess";
                    const LABEL: &str = "Update to Current Process";
                    let targets = match named_photos(s, p, C)? {
                        Some(ids) if !ids.iter().any(|id| outdated(s, *id)) => return Err(crate::EngineError::Disabled(C.into(), already_current())),
                        Some(ids) => ids,
                        None => s.targets(p)?,
                    };
                    let ops: Vec<Op> = targets
                        .into_iter()
                        .filter_map(|id| s.develop_of(id).map(|d| (id, d)))
                        .filter(|(_, d)| d.process.is_outdated())
                        .filter_map(|(id, d)| s.develop_op(id, DevelopSettings { process: ProcessVersion::LATEST, ..(*d).clone() }, LABEL))
                        .collect();
                    let n = ops.len();
                    if n > 0 {
                        s.commit(LABEL, Op::Batch { ops })?;
                    }
                    Ok(json!({"changed": n, "process": ProcessVersion::LATEST}))
                }
            )
        },
        cmd!(
            "develop.resetSection",
            "Reset Section",
            [],
            None,
            "{section: light|curve|color|mixer|grading|effects|vignette|grain|detail|optics|geometry}",
            has_active,
            |s, p| {
                let sec = section_param(p, "develop.resetSection")?;
                edit(s, "develop.resetSection", &format!("Reset {}", sec.label()), |d| {
                    d.reset_section(sec);
                    Ok(())
                })
            }
        ),
        cmd!("develop.resetControl", "Reset Control", [], None, "{control}", has_active, |s, p| {
            let c = str_param(p, "control").ok_or_else(|| bad("develop.resetControl", "missing control"))?.to_string();
            let spec = controls::find(&c).ok_or_else(|| bad("develop.resetControl", "unknown control"))?;
            let dims = crop_dims(s, active(s, "develop.resetControl")?);
            edit(s, "develop.resetControl", spec.label, |d| {
                let before = d.crop.geometry;
                controls::set(d, &c, spec.default);
                refit_crop_angle(c == CROP_ANGLE, before, d, dims);
                Ok(())
            })
        }),
        cmd!("develop.sectionEnabled", "Toggle Section", [], None, "{section: string, enabled: bool}", has_active, |s, p| {
            let sec = str_param(p, "section").ok_or_else(|| bad("develop.sectionEnabled", "missing section"))?.to_string();
            let on = bool_or(p, "enabled", true);
            edit(s, "develop.sectionEnabled", "Toggle Section", |d| {
                d.set_section_enabled(&sec, on);
                Ok(())
            })
        }),
        cmd!(
            "develop.autoBwMix",
            "Auto B&W Mix",
            [],
            None,
            "{} — sets the black & white mix from the photo's colours (and switches to B&W)",
            has_active,
            |s, _| {
                let id = active(s, "develop.autoBwMix")?;
                let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.autoBwMix", e))?;
                let info = s.source_info(id);
                let d = s.develop_of(id).unwrap_or_default();
                let m = lightcraft_pipeline::auto::auto_bw_mix(&src, &info, &d);
                edit(s, "develop.autoBwMix", "Auto B&W Mix", |d| {
                    d.treatment = Treatment::Bw;
                    let b = &mut d.bw_mix;
                    [b.red, b.orange, b.yellow, b.green, b.aqua, b.blue, b.purple, b.magenta] = m;
                    Ok(())
                })?;
                Ok(json!({"mix": m}))
            }
        ),
        cmd!("develop.auto", "Auto Settings", ["Photo"], Some("Shift+A"), "{}", has_active, |s, _| {
            let id = active(s, "develop.auto")?;
            let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.auto", e))?;
            let info = s.source_info(id);
            let d = s.develop_of(id).unwrap_or_default();
            let a = lightcraft_pipeline::auto::auto_tone(&src, &info, &d);
            edit(s, "develop.auto", "Auto", |d| {
                d.light.exposure = a.exposure;
                d.light.contrast = a.contrast;
                d.light.highlights = a.highlights;
                d.light.shadows = a.shadows;
                d.light.whites = a.whites;
                d.light.blacks = a.blacks;
                d.color.vibrance = a.vibrance;
                d.color.saturation = a.saturation;
                Ok(())
            })?;
            Ok(serde_json::to_value(a).unwrap_or_default())
        }),
        cmd!(
            "develop.wb",
            "White Balance",
            [],
            None,
            "{mode: asShot|auto|daylight|cloudy|shade|tungsten|fluorescent|flash|custom, temp?, tint?}",
            has_active,
            |s, p| {
                let mode: WbMode =
                    serde_json::from_value(p.get("mode").cloned().unwrap_or(json!("custom"))).map_err(|e| bad("develop.wb", e.to_string()))?;
                let id = active(s, "develop.wb")?;
                let info = s.source_info(id);
                let (mut t, mut tint) = match mode {
                    WbMode::AsShot => (info.as_shot_temp, info.as_shot_tint),
                    WbMode::Auto => {
                        let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.wb", e))?;
                        lightcraft_pipeline::auto::auto_wb(&src, &info)
                    }
                    m => m
                        .preset()
                        .map(|(t, ti)| {
                            // presets are Kelvin on raws with that scale, relative to daylight (6500 K = as shot) elsewhere
                            if info.raw && !info.relative_wb { (t, ti) } else { (6500.0 * t / 5500.0, ti) }
                        })
                        .unwrap_or((info.as_shot_temp, info.as_shot_tint)),
                };
                t = f64_or(p, "temp", t);
                tint = f64_or(p, "tint", tint);
                // The resolved temperature/tint is stored, so rendering never depends on the mode; the
                // mode is kept for the UI's WB dropdown.
                edit(s, "develop.wb", "White Balance", |d| {
                    lightcraft_pipeline::local::normalize_wb(d, &info);
                    d.wb.mode = mode;
                    d.wb.temp = t.clamp(2000.0, 50000.0);
                    d.wb.tint = tint.clamp(-150.0, 150.0);
                    Ok(())
                })?;
                Ok(json!({"temp": t, "tint": tint}))
            }
        ),
        cmd!("develop.wbPick", "White Balance Selector", [], Some("W"), "{x, y} normalized image coords of a neutral point", has_active, |s, p| {
            let (x, y) = (f64_req(p, "x", "develop.wbPick")?, f64_req(p, "y", "develop.wbPick")?);
            let id = active(s, "develop.wbPick")?;
            let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.wbPick", e))?;
            let info = s.source_info(id);
            let d = s.develop_of(id).unwrap_or_default();
            // Sample a small neighbourhood in the oriented source.
            let o = src.oriented(d.orientation);
            let (cx, cy) = ((x.clamp(0.0, 1.0) * o.width as f64) as isize, (y.clamp(0.0, 1.0) * o.height as f64) as isize);
            let mut acc = [0.0f32; 3];
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let c = o.get_clamped(cx + dx, cy + dy);
                    for i in 0..3 {
                        acc[i] += c[i] / 25.0;
                    }
                }
            }
            let patch = lightcraft_raster::Rgb32f::filled(4, 4, acc);
            let (t, tint) = lightcraft_pipeline::auto::auto_wb(&patch, &info);
            edit(s, "develop.wbPick", "White Balance", |d| {
                lightcraft_pipeline::local::normalize_wb(d, &info);
                d.wb.mode = WbMode::Custom;
                d.wb.temp = t;
                d.wb.tint = tint;
                Ok(())
            })?;
            Ok(json!({"temp": t, "tint": tint}))
        }),
        cmd!("develop.treatment", "Black & White", ["Photo"], Some("V"), "{bw?: bool} (toggles when omitted)", has_active, |s, p| {
            edit(s, "develop.treatment", "Treatment", |d| {
                let bw = p.get("bw").and_then(Value::as_bool).unwrap_or(d.treatment != Treatment::Bw);
                d.treatment = if bw { Treatment::Bw } else { Treatment::Color };
                Ok(())
            })
        }),
        cmd!("develop.profile", "Set Profile", [], None, "{id: profile id (see profiles.list), amount?: 0..200}", has_active, |s, p| {
            let id = str_param(p, "id").ok_or_else(|| bad("develop.profile", "missing id"))?.to_string();
            if !crate::presets::PROFILES.iter().any(|x| x.id == id)
                && !s.lut_profiles.iter().any(|x| x.id == id)
                && !s.dcp_profiles.iter().any(|x| x.id == id)
            {
                return Err(bad("develop.profile", format!("unknown profile `{id}`")));
            }
            let mut d = (*s.develop_of(active(s, "develop.profile")?).unwrap_or_default()).clone();
            crate::cmd::dcp_profiles::choose(s, &mut d, &id);
            edit(s, "develop.profile", "Profile", |e| {
                e.profile.id = d.profile.id.clone();
                e.camera_profile = d.camera_profile.clone();
                e.profile.amount = f64_or(p, "amount", e.profile.amount).clamp(0.0, 200.0);
                Ok(())
            })?;
            s.note_profile_used(&id);
            ok()
        }),
        cmd!(
            "develop.cameraProfile",
            "Set Camera Calibration",
            [],
            None,
            "{id: dcp:… | \"\" (LightCraft's own camera colour)} — the camera calibration only; the look (develop.profile) stays",
            has_active,
            |s, p| {
                let id = str_param(p, "id").unwrap_or("").to_string();
                if !id.is_empty() && !s.dcp_profiles.iter().any(|x| x.id == id) {
                    return Err(bad("develop.cameraProfile", format!("unknown camera profile `{id}`")));
                }
                let mut d = (*s.develop_of(active(s, "develop.cameraProfile")?).unwrap_or_default()).clone();
                crate::cmd::dcp_profiles::choose(s, &mut d, &id);
                edit(s, "develop.cameraProfile", "Camera Calibration", |e| {
                    e.profile.id = d.profile.id.clone();
                    e.camera_profile = d.camera_profile.clone();
                    Ok(())
                })?;
                ok()
            }
        ),
        cmd!("profile.favorite", "Favorite Profile", [], None, "{id, favorite?: bool} (toggles when omitted)", always, |s, p| {
            let id = str_param(p, "id").ok_or_else(|| bad("profile.favorite", "missing id"))?.to_string();
            if crate::presets::profile(&id).is_none() && !s.lut_profiles.iter().any(|x| x.id == id) {
                return Err(bad("profile.favorite", format!("unknown profile `{id}`")));
            }
            let on = s.profile_favorites.contains(&id);
            let want = bool_or(p, "favorite", !on);
            if want && !on {
                s.profile_favorites.push(id);
            } else if !want {
                s.profile_favorites.retain(|f| *f != id);
            }
            Ok(json!({"favorite": want}))
        }),
        cmd!("develop.curve", "Set Point Curve", [], None, "{channel: master|red|green|blue, points: [[x,y],...] in 0..1}", has_active, |s, p| {
            let ch = str_param(p, "channel").unwrap_or("master").to_string();
            let mut pts: Vec<Point> = p
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("develop.curve", "missing points"))?
                .iter()
                .filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?.clamp(0.0, 1.0), q.get(1)?.as_f64()?.clamp(0.0, 1.0))))
                .collect();
            // stored in the order the curve runs, as the editor draws and drags them
            pts.sort_by(|a, b| a.x.total_cmp(&b.x));
            edit(s, "develop.curve", "Tone Curve", |d| {
                match ch.as_str() {
                    "master" | "rgb" => d.curve.master = pts,
                    "red" => d.curve.red = pts,
                    "green" => d.curve.green = pts,
                    "blue" => d.curve.blue = pts,
                    other => return Err(bad("develop.curve", format!("unknown channel `{other}`"))),
                }
                Ok(())
            })
        }),
        // ---- crop & rotate
        cmd!("crop.set", "Set Crop", [], None, "{rect?: [x0,y0,x1,y1] normalized in the straightened frame, angle?: degrees}", has_active, |s, p| {
            let id = active(s, "crop.set")?;
            let (w, h) = crop_dims(s, id);
            edit(s, "crop.set", "Crop", |d| {
                if let Some(a) = p.get("rect").and_then(Value::as_array) {
                    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                    if v.len() != 4 {
                        return Err(bad("crop.set", "rect needs 4 numbers"));
                    }
                    d.crop.geometry.rect = Rect::new(v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3]));
                }
                let angle = p.get("angle").and_then(Value::as_f64).map(|a| a.clamp(-45.0, 45.0));
                d.crop.geometry = match (p.get("rect"), angle) {
                    // a new angle alone turns the crop as Straighten does (issue #742)
                    (None, Some(a)) => d.crop.geometry.with_angle(a, w, h),
                    (_, a) => CropGeometry { angle: a.unwrap_or(d.crop.geometry.angle), ..d.crop.geometry }.constrained(w, h),
                };
                Ok(())
            })
        }),
        cmd!(
            "crop.drag",
            "Drag Crop Handle",
            [],
            None,
            "{handle: topLeft|topRight|bottomRight|bottomLeft|top|right|bottom|left|move | 0..8, from: [x,y], to: [x,y] (normalized, straightened frame), start?: [x0,y0,x1,y1] (default: the current crop), keepRatio?: bool (default false)} — drags a crop handle; stops at the image edge, keeps the opposite edges, honours the aspect lock; keepRatio holds the frame's current proportions for this drag even with the lock off (the UI sends it while Shift is held)",
            has_active,
            |s, p| {
                let id = active(s, "crop.drag")?;
                let (w, h) = crop_dims(s, id);
                let handle = match p.get("handle") {
                    Some(Value::String(n)) => CropHandle::from_name(n),
                    Some(v) => v.as_u64().and_then(|i| u8::try_from(i).ok()).and_then(CropHandle::from_index),
                    None => None,
                }
                .ok_or_else(|| bad("crop.drag", "unknown handle"))?;
                let point = |k: &str| -> Result<Point> {
                    match numbers(p.get(k)).as_deref() {
                        Some([x, y]) if x.is_finite() && y.is_finite() => Ok(Point::new(*x, *y)),
                        _ => Err(bad("crop.drag", format!("`{k}` needs [x, y]"))),
                    }
                };
                let (from, to) = (point("from")?, point("to")?);
                let given = match (p.get("start"), numbers(p.get("start")).as_deref()) {
                    (None | Some(Value::Null), _) => None,
                    (_, Some(&[x0, y0, x1, y1])) if [x0, y0, x1, y1].iter().all(|v| v.is_finite()) => {
                        Some(Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)))
                    }
                    _ => return Err(bad("crop.drag", "`start` needs 4 numbers")),
                };
                let keep_ratio = bool_or(p, "keepRatio", false);
                edit(s, "crop.drag", "Crop", |d| {
                    let start = CropGeometry { rect: given.unwrap_or(d.crop.geometry.rect), angle: d.crop.geometry.angle };
                    // A locked crop keeps the shape it has: the lock stores a width:height tuple whose
                    // orientation is reinterpreted per image, so the frame itself is the truth.
                    // `keepRatio` (Shift held) asks for the same thing for one drag without touching the lock.
                    let ratio = (d.crop.aspect.is_some() || keep_ratio).then(|| start.rect_px(w, h).aspect()).filter(|a| a.is_finite() && *a > 0.0);
                    d.crop.geometry = drag_crop(start, handle, from, to, w, h, ratio);
                    Ok(())
                })
            }
        ),
        cmd!(
            "crop.straighten",
            "Straighten",
            [],
            None,
            "{angle: degrees -45..45} — keeps the crop if it still fits, else shrinks it about its centre (the largest crop of its aspect stays the largest)",
            has_active,
            |s, p| {
                let id = active(s, "crop.straighten")?;
                let (w, h) = crop_dims(s, id);
                let angle = f64_req(p, "angle", "crop.straighten")?.clamp(-45.0, 45.0);
                edit(s, "crop.straighten", "Straighten", |d| {
                    // keep the current crop if it still fits, else shrink it about its centre until it does
                    d.crop.geometry = d.crop.geometry.with_angle(angle, w, h);
                    Ok(())
                })
            }
        ),
        cmd!(
            "crop.autoStraighten",
            "Auto Straighten",
            [],
            None,
            "{} — level the horizon (or plumb lines) with the crop angle",
            has_active,
            |s, _| {
                let id = active(s, "crop.autoStraighten")?;
                let src = s.source_now(id, SourceLevel::Preview).map_err(|e| bad("crop.autoStraighten", e))?;
                let info = s.source_info(id);
                let d = s.develop_of(id).unwrap_or_default();
                let Some(level) = lightcraft_pipeline::upright::level_degrees(&src, &info, &d) else {
                    return Ok(json!({"changed": false, "reason": "no dominant horizontal or vertical lines"}));
                };
                // Same convention as the crop angle (verified: a horizon descending 6° to the right
                // comes out level with angle = level ≈ −6°).
                let angle = level.clamp(-45.0, 45.0);
                s.execute("crop.straighten", &json!({"angle": (angle * 100.0).round() / 100.0}))?;
                Ok(json!({"changed": true, "angle": angle}))
            }
        ),
        cmd!(
            "crop.aspect",
            "Crop Aspect",
            [],
            None,
            "{aspect: \"original\"|\"free\"|\"current\" (lock the crop's present shape)|\"toggle\" (lock ↔ free)|\"1x1\"|\"4x5\"|\"8.5x11\"|\"5x7\"|\"2x3\"|\"4x3\"|\"16x9\"|\"16x10\"|[w,h], orientation?: \"crop\"|\"given\"} — a named ratio takes the crop's present orientation (landscape, or portrait after X); [w,h] is used as given (taller than wide → portrait); `orientation` overrides either",
            has_active,
            |s, p| {
                let id = active(s, "crop.aspect")?;
                let (w, h) = crop_dims(s, id);
                // checked first, whatever the aspect: a bad value is an error on every path
                let turn = match p.get("orientation") {
                    Some(Value::String(o)) if o == "crop" => Turn::LikeCrop,
                    Some(Value::String(o)) if o == "given" => Turn::AsGiven,
                    Some(_) => return Err(bad("crop.aspect", "orientation is \"crop\" or \"given\"")),
                    None if matches!(p.get("aspect"), Some(Value::Array(_))) => Turn::AsGiven,
                    None => Turn::LikeCrop,
                };
                let locked = s.develop_of(id).is_some_and(|d| d.crop.aspect.is_some());
                let lock_current = match p.get("aspect").and_then(Value::as_str) {
                    Some("current") => true,
                    Some("toggle") => !locked,
                    _ => false,
                };
                if lock_current {
                    // lock to the rectangle as it is: no refit
                    return edit(s, "crop.aspect", "Lock Crop Aspect", |d| {
                        let r = d.crop.geometry.rect_px(w, h);
                        d.crop.aspect = Some(((r.width() * 100.0).round().max(1.0) as u32, (r.height() * 100.0).round().max(1.0) as u32));
                        Ok(())
                    });
                }
                let aspect = match p.get("aspect") {
                    Some(Value::String(a)) if a == "toggle" => None,
                    Some(Value::String(a)) if a == "free" => None,
                    Some(Value::String(a)) if a == "original" => Some(((w * 100.0) as u32, (h * 100.0) as u32)),
                    Some(Value::String(a)) => {
                        let (x, y) = a.split_once('x').ok_or_else(|| bad("crop.aspect", "use WxH"))?;
                        let (x, y): (f64, f64) =
                            (x.parse().map_err(|_| bad("crop.aspect", "bad number"))?, y.parse().map_err(|_| bad("crop.aspect", "bad number"))?);
                        Some(aspect_tuple("crop.aspect", x, y)?)
                    }
                    Some(v @ Value::Array(_)) => match numbers(Some(v)).as_deref() {
                        Some(&[x, y]) => Some(aspect_tuple("crop.aspect", x, y)?),
                        _ => return Err(bad("crop.aspect", "use [w, h] with two numbers")),
                    },
                    _ => return Err(bad("crop.aspect", "missing aspect")),
                };
                edit(s, "crop.aspect", "Crop Aspect", |d| {
                    if aspect.is_none() {
                        d.crop.aspect = None;
                    } else {
                        set_aspect(d, w, h, aspect, turn);
                    }
                    Ok(())
                })
            }
        ),
        cmd!(
            "crop.rotateAspect",
            "Rotate Crop Aspect",
            [],
            Some("X"),
            "{lock?: [w,h]} — swap the crop between landscape and portrait, keeping its lock and ratio (3:2 → 2:3); `lock` is the ratio the crop tool shows locked on a crop that has none yet (an untouched photo), so the swap keeps it locked",
            has_active,
            |s, p| {
                let id = active(s, "crop.rotateAspect")?;
                let (w, h) = crop_dims(s, id);
                let lock = match p.get("lock") {
                    None | Some(Value::Null) => None,
                    Some(v @ Value::Array(_)) => match numbers(Some(v)).as_deref() {
                        Some(&[x, y]) => Some(aspect_tuple("crop.rotateAspect", x, y)?),
                        _ => return Err(bad("crop.rotateAspect", "lock is [w, h] with two numbers")),
                    },
                    Some(_) => return Err(bad("crop.rotateAspect", "lock is [w, h] with two numbers")),
                };
                edit(s, "crop.rotateAspect", "Rotate Crop", |d| {
                    // client note 3c: the tool showed an untouched photo's crop locked (to the photo's
                    // own shape or the remembered ratio) while the crop itself had no lock, so the swap
                    // left it free and the ratio became Custom. Lock it first, as the tool shows it.
                    if d.crop.aspect.is_none()
                        && let Some(l) = lock
                    {
                        set_aspect(d, w, h, Some(l), Turn::LikeCrop);
                    }
                    let r = d.crop.geometry.rect_px(w, h);
                    // a degenerate crop has no aspect to rotate: fall back to the image's own
                    let a = Some(r.height() / r.width()).filter(|a| a.is_finite() && *a > 0.0);
                    d.crop.aspect = d.crop.aspect.map(|(x, y)| (y, x));
                    let angle = d.crop.geometry.angle;
                    d.crop.geometry = crop_fit_angle(w, h, angle, a);
                    Ok(())
                })
            }
        ),
        cmd!("crop.reset", "Reset Crop", [], Some("Cmd+Alt+R"), "{}", has_active, |s, _| {
            edit(s, "crop.reset", "Reset Crop", |d| {
                d.crop = Default::default();
                Ok(())
            })
        }),
        // ---- HDR editing
        cmd!(
            "develop.hdr",
            "HDR",
            [],
            None,
            "{enabled?: bool (default: toggle), maxEv?: number 0..5 (headroom limit, stops above SDR white)} — HDR editing: highlights above SDR white for HDR exports and displays; every SDR render uses the SDR rendition (the hdr.sdr* controls)",
            has_active,
            |s, p| {
                let id = active(s, "develop.hdr")?;
                let cur = s.develop_of(id).unwrap_or_default().hdr;
                let enabled = bool_or(p, "enabled", !cur.enabled);
                let max_ev = match p.get("maxEv") {
                    None => None,
                    Some(v) => Some(v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("develop.hdr", "`maxEv` must be a number"))?),
                };
                let label = if enabled { "HDR On" } else { "HDR Off" };
                edit(s, "develop.hdr", label, |d| {
                    d.hdr.enabled = enabled;
                    if let Some(m) = max_ev {
                        d.hdr.max_ev = m.clamp(0.0, lightcraft_develop::Hdr::MAX_EV_LIMIT);
                    }
                    Ok(())
                })?;
                Ok(json!({"enabled": enabled, "maxEv": s.develop_of(id).map(|d| d.hdr.max_ev)}))
            }
        ),
        // ---- geometry: Upright
        cmd!(
            "geometry.upright",
            "Upright",
            [],
            None,
            "{mode: off|auto|guided|level|vertical|full, again?: bool} — analyses the photo (line segments → vanishing points) and stores the correction; run again to update after lens changes. `again` (a second click on the mode that is on) starts it over: the Transform sliders back to 0, a crop Constrain Crop made fitted again, Guided's guides cleared → {mode, transform, note (why the mode found nothing to correct, when it didn't)}",
            has_active,
            |s, p| {
                let mode: Upright =
                    serde_json::from_value(p.get("mode").cloned().unwrap_or(json!("auto"))).map_err(|e| bad("geometry.upright", e.to_string()))?;
                let id = active(s, "geometry.upright")?;
                let d = s.develop_of(id).unwrap_or_default();
                let (transform, note) = match mode {
                    Upright::Off | Upright::Guided => (None, None),
                    m => {
                        let src = s.source_now(id, SourceLevel::Preview).map_err(|e| bad("geometry.upright", e))?;
                        let info = s.source_info(id);
                        let a = lightcraft_pipeline::upright::analyze_source(&src, &info, &d);
                        (Some(lightcraft_pipeline::upright::solve(m, &a)), lightcraft_pipeline::upright::shortfall(m, &a))
                    }
                };
                // test notes 2026-10-24 note 6: a second click on the mode starts over, without hunting for resets
                let again = bool_or(p, "again", false);
                edit(s, "geometry.upright", "Upright", |d| {
                    d.geometry.upright = mode;
                    d.geometry.upright_transform = transform;
                    if again {
                        let keep = d.geometry.clone();
                        d.geometry = lightcraft_develop::Geometry {
                            upright: keep.upright,
                            upright_transform: keep.upright_transform,
                            constrain_crop: keep.constrain_crop,
                            guides: if mode == Upright::Guided { Vec::new() } else { keep.guides },
                            ..Default::default()
                        };
                    }
                    Ok(())
                })?;
                // client note 4: say why when the mode can't do what it says, instead of nothing happening
                let note = note.map(|why| {
                    let what = match mode {
                        Upright::Level => "No level correction",
                        Upright::Vertical => "No vertical correction",
                        Upright::Full => "No full correction",
                        _ => "No Upright correction",
                    };
                    format!("{what}: {why}.")
                });
                Ok(json!({"mode": mode, "transform": transform, "note": note}))
            }
        ),
        cmd!(
            "geometry.guides",
            "Guided Upright Guides",
            [],
            None,
            "{guides: [[x0,y0,x1,y1], …] (≤ 4, normalized coords of the lens-corrected image), add?: bool (append one guide, dropping the oldest beyond 4)} — switches Upright to Guided",
            has_active,
            |s, p| {
                let parse = |v: &Value| -> Option<(Point, Point)> {
                    let a: Vec<f64> = v.as_array()?.iter().filter_map(Value::as_f64).collect();
                    (a.len() == 4 && a.iter().all(|x| x.is_finite())).then(|| (Point::new(a[0], a[1]), Point::new(a[2], a[3])))
                };
                let guides: Vec<(Point, Point)> =
                    p.get("guides").and_then(Value::as_array).map(|a| a.iter().filter_map(parse).collect()).unwrap_or_default();
                let add = bool_or(p, "add", false);
                edit(s, "geometry.guides", "Guided Upright", |d| {
                    if add {
                        d.geometry.guides.extend(guides);
                    } else {
                        d.geometry.guides = guides;
                    }
                    let n = d.geometry.guides.len();
                    if n > 4 {
                        d.geometry.guides.drain(..n - 4);
                    }
                    d.geometry.upright = Upright::Guided;
                    d.geometry.upright_transform = None;
                    Ok(())
                })
            }
        ),
        // ---- copy / paste / sync
        cmd!(
            "develop.copy",
            "Copy Edit Settings",
            ["Edit"],
            Some("Cmd+C"),
            "{groups?: [settingsGroup], settings?: [settingItem] (single settings of a group, develop.settingItems)} (default: what was ticked last; at first all but crop, masks, remove) — the choice is remembered app-wide",
            has_active,
            |s, p| {
                let c = "develop.copy";
                let id = active(s, c)?;
                if let Some(pick) = pick_param(p, c)?
                    && pick != s.dialogs.copy
                {
                    s.dialogs.copy = pick;
                    remember_dialogs(s);
                }
                let d = s.develop_of(id).unwrap_or_default();
                // masks travel as recipes: each photo they are pasted on computes its own selections
                let mut clip = s.dialogs.copy.extract(&d);
                lightcraft_develop::presets::to_recipes(&mut clip);
                s.clipboard = Some(clip);
                Ok(json!({"groups": s.dialogs.copy.groups, "settings": s.dialogs.copy.settings}))
            }
        ),
        cmd!(
            "develop.pastePrevious",
            "Paste Settings from Previous",
            ["Edit"],
            Some("Cmd+Alt+V"),
            "{ids?, groups?: [settingsGroup], settings?: [settingItem] (default: the copy choice)} — from the photo that was active before this one",
            has_active,
            |s, p| {
                let c = "develop.pastePrevious";
                let prev =
                    s.previous_active.filter(|id| s.catalog.photo(*id).is_some_and(|ph| !ph.deleted)).ok_or_else(|| bad(c, "no previous photo"))?;
                let pick = pick_param(p, c)?.unwrap_or_else(|| s.dialogs.copy.clone());
                let mut partial = pick.extract(&s.develop_of(prev).unwrap_or_default());
                lightcraft_develop::presets::to_recipes(&mut partial);
                let ids: Vec<PhotoId> = s.targets(p)?.into_iter().filter(|id| *id != prev).collect();
                let ops = ids
                    .iter()
                    .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                    .filter_map(|(id, d)| s.develop_op(id, lightcraft_develop::apply_partial(&d, &partial, 1.0), "Paste from Previous"))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit("Paste from Previous", Op::Batch { ops })?;
                let masks = if partial.get("masks").is_some() { s.compute_masks(&ids, false) } else { Value::Null };
                Ok(json!({"changed": n, "from": prev.0, "masks": masks}))
            }
        ),
        cmd!(
            "develop.paste",
            "Paste Edit Settings",
            ["Edit"],
            Some("Cmd+V"),
            "{ids?, groups?: [settingsGroup], settings?: [settingItem] (paste only these of the copied settings)}",
            has_clipboard,
            |s, p| {
                let c = "develop.paste";
                let mut clip = s.clipboard.clone().unwrap_or_default();
                if let Some(only) = pick_param(p, c)? {
                    // the clipboard holds what was copied: rebuild it with just the chosen part of it
                    let only = only.intersect(&s.dialogs.copy);
                    let masks = clip.get("masks").cloned();
                    let full = lightcraft_develop::apply_partial(&DevelopSettings::default(), &clip, 1.0);
                    clip = only.extract(&full);
                    // (the copied masks go as they were copied: recipes)
                    if let (Some(m), Some(o)) = (masks, clip.as_object_mut())
                        && o.contains_key("masks")
                    {
                        o.insert("masks".into(), m);
                    }
                }
                lightcraft_develop::presets::to_recipes(&mut clip);
                let ids = s.targets(p)?;
                let ops = ids
                    .iter()
                    .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                    .filter_map(|(id, d)| s.develop_op(id, lightcraft_develop::apply_partial(&d, &clip, 1.0), "Paste Settings"))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit("Paste Settings", Op::Batch { ops })?;
                let masks = if clip.get("masks").is_some() { s.compute_masks(&ids, false) } else { Value::Null };
                Ok(json!({"changed": n, "masks": masks}))
            }
        ),
        cmd!(
            "develop.sync",
            "Sync Settings",
            [],
            None,
            "{groups?, settings?: [settingItem]} (default: what Sync ticked last) — copy the active photo's settings to all selected photos; the choice is remembered app-wide",
            has_selection,
            |s, p| {
                let c = "develop.sync";
                let id = active(s, c)?;
                if let Some(pick) = pick_param(p, c)?
                    && pick != s.dialogs.sync
                {
                    s.dialogs.sync = pick;
                    remember_dialogs(s);
                }
                let d = s.develop_of(id).unwrap_or_default();
                let mut partial = s.dialogs.sync.extract(&d);
                lightcraft_develop::presets::to_recipes(&mut partial);
                let targets: Vec<PhotoId> = s.selection.ids.iter().copied().filter(|x| *x != id).collect();
                let ops = targets
                    .iter()
                    .filter_map(|x| s.develop_of(*x).map(|d| (*x, d)))
                    .filter_map(|(x, dd)| s.develop_op(x, lightcraft_develop::apply_partial(&dd, &partial, 1.0), "Sync Settings"))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit("Sync Settings", Op::Batch { ops })?;
                let masks = if partial.get("masks").is_some() { s.compute_masks(&targets, false) } else { Value::Null };
                Ok(json!({"changed": n, "masks": masks}))
            }
        ),
        cmd!(
            "develop.settingItems",
            "List Settings to Copy",
            [],
            None,
            "{} → {groups: [{group, label, settings: [{id, label}]}], copy, sync, newPreset, presetGroup}: the settings groups and the single settings inside them that copy, sync and presets can pick, and what Copy, Sync and New Preset tick now",
            always,
            |s, _| {
                let groups: Vec<Value> = SettingsGroup::ALL
                    .iter()
                    .map(|g| {
                        json!({
                            "group": g.id(),
                            "label": g.label(),
                            "settings": g.items().iter().map(|i| json!({"id": i.id, "label": i.label})).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                Ok(json!({
                    "groups": groups,
                    "copy": s.dialogs.copy,
                    "sync": s.dialogs.sync,
                    "newPreset": s.dialogs.new_preset,
                    "presetGroup": s.dialogs.preset_group,
                }))
            }
        ),
        cmd!(
            "develop.quickAdjust",
            "Quick Develop",
            [],
            None,
            "{control: develop control id, delta, ids?} — add `delta` to the control on every target photo (each from its own value; Quick Develop) in one undo step → {changed}",
            has_selection,
            |s, p| {
                let c = "develop.quickAdjust";
                let ctl = str_param(p, "control").ok_or_else(|| bad(c, "missing control"))?.to_string();
                let spec = controls::find(&ctl).ok_or_else(|| bad(c, format!("unknown control `{ctl}`")))?;
                let delta = f64_req(p, "delta", c)?;
                let label = format!("Quick Develop: {}", spec.label);
                let ops: Vec<Op> = s
                    .targets(p)?
                    .into_iter()
                    .filter_map(|id| s.develop_of(id).map(|d| (id, d)))
                    .filter_map(|(id, d)| {
                        // each photo on its own scale: Kelvin (an old relative value converted first) or relative
                        let nd = nudge(&s.source_info(id), crop_dims(s, id), &d, &ctl, delta, spec.default);
                        s.develop_op(id, nd, &label)
                    })
                    .collect();
                let n = ops.len();
                if n > 0 {
                    s.commit(&label, Op::Batch { ops })?;
                }
                Ok(json!({"changed": n}))
            }
        ),
        cmd!(
            "develop.matchExposure",
            "Match Total Exposures",
            ["Photo"],
            None,
            "{ids?} — set each selected photo's Exposure so its total exposure (shutter × ISO ÷ aperture², plus the slider) equals the active photo's; photos without exposure data are skipped → {changed, skipped}",
            has_selection,
            |s, p| {
                let c = "develop.matchExposure";
                let reference = active(s, c)?;
                // log2 of the light the photo gathered (shutter s × ISO / f-number²)
                let gathered = |s: &Session, id: PhotoId| -> Option<f64> {
                    let m = &s.catalog.photo(id)?.meta;
                    let t = match m.shutter.split_once('/') {
                        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?,
                        None => m.shutter.trim().trim_end_matches('s').trim().parse::<f64>().ok()?,
                    };
                    let (n, iso) = (m.aperture? as f64, m.iso? as f64);
                    (t > 0.0 && n > 0.0 && iso > 0.0).then(|| (t * iso / (n * n)).log2())
                };
                let r_ev = gathered(s, reference).ok_or_else(|| bad(c, "the active photo has no shutter / aperture / ISO"))?;
                let r_slider = s.develop_of(reference).map(|d| d.light.exposure).unwrap_or(0.0);
                let ids: Vec<PhotoId> = s.targets(p)?.into_iter().filter(|x| *x != reference).collect();
                let (mut ops, mut skipped) = (Vec::new(), Vec::new());
                for id in ids {
                    let (Some(ev), Some(d)) = (gathered(s, id), s.develop_of(id)) else {
                        skipped.push(id.0);
                        continue;
                    };
                    let mut nd = (*d).clone();
                    nd.light.exposure = (r_slider + r_ev - ev).clamp(-5.0, 5.0);
                    ops.extend(s.develop_op(id, nd, "Match Total Exposures"));
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Match Total Exposures", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n, "skipped": skipped}))
            }
        ),
        cmd!(
            "develop.autoSync",
            "Auto Sync",
            ["Edit"],
            Some("Cmd+Alt+Shift+A"),
            "{on?: bool} (toggles when omitted) — edits to the active photo also change the other selected photos (only the settings that changed; not spot removal / red eye). Remembered app-wide (the install's store), whichever library is open → {on}",
            always,
            |s, p| {
                let on = match p.get("on") {
                    None | Some(Value::Null) => !s.auto_sync,
                    Some(Value::Bool(on)) => *on,
                    Some(_) => return Err(bad("develop.autoSync", "on must be true or false")),
                };
                if on != s.auto_sync {
                    let was = s.auto_sync;
                    s.auto_sync = on;
                    if let Err(e) = s.save_prefs() {
                        // not kept: not changed either, so asking again does what was asked
                        s.auto_sync = was;
                        return Err(e);
                    }
                }
                Ok(json!({"on": s.auto_sync}))
            }
        ),
        // ---- presets
        cmd!(
            "preset.apply",
            "Apply Preset",
            [],
            None,
            "{id: presetId, amount?: 0..200 (percent), ids?, followAutoSync?: bool (true, as Develop's Presets panel: the photos a slider changes, the active photo, or every selected photo with Auto Sync on; default: the selection)} — as Lightroom Classic: sets the settings the preset has and adds its masks (never removes the photo's own); its AI masks (Subject, Sky, People by name or role, remembered objects, anchored gradients) are then computed on each photo, in the app in the background (one undo step with the apply). The same preset again right away replaces its own masks instead of doubling them → {changed, masks: {queued, updated, approximate, notes}}",
            has_selection,
            |s, p| {
                let pid = str_param(p, "id").ok_or_else(|| bad("preset.apply", "missing id"))?;
                let preset = s.presets.iter().find(|x| x.id == pid).cloned().ok_or_else(|| bad("preset.apply", format!("unknown preset `{pid}`")))?;
                let amount = f64_or(p, "amount", 100.0).clamp(0.0, 200.0) / 100.0;
                // `id` names the preset here: the photos are `ids`, else the selection, or (Develop's
                // Presets panel) the photos a slider would change (note 51)
                let ids = if p.get("ids").is_none_or(Value::is_null) && bool_or(p, "followAutoSync", false) {
                    s.slider_targets()
                } else {
                    s.targets(&json!({"ids": p.get("ids")}))?
                };
                let label = format!("Preset: {}", preset.name);
                let ops = ids
                    .iter()
                    .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                    .filter_map(|(id, d)| s.develop_op(id, preset.apply(&d, amount, s.photo_aspect(id)), &label))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit(&label, Op::Batch { ops })?;
                let masks = if preset.settings.get("masks").is_some() { s.compute_masks(&ids, false) } else { Value::Null };
                Ok(json!({"changed": n, "masks": masks}))
            }
        ),
        cmd!(
            "preset.create",
            "Create Preset",
            ["Photo"],
            None,
            "{name, group?, groups?: [settingsGroup], settings?: [settingItem]} (default: what New Preset ticked last; White Balance only once ticked) — a group named like an existing one (any case, spaces trimmed) is that group; the choice is remembered app-wide → {id, group}",
            has_active,
            |s, p| {
                let c = "preset.create";
                let id = active(s, c)?;
                let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing name"))?.to_string();
                let typed = str_param(p, "group").map(str::trim).filter(|g| !g.is_empty()).unwrap_or(crate::app_store::DEFAULT_PRESET_GROUP);
                let group = s.existing_preset_group(typed);
                let pick = match pick_param(p, c)? {
                    Some(pick) => pick,
                    None => s.dialogs.new_preset.clone(),
                };
                if pick.is_empty() {
                    return Err(bad(c, "tick at least one setting to save in the preset"));
                }
                if pick != s.dialogs.new_preset || group != s.dialogs.preset_group {
                    s.dialogs.new_preset = pick.clone();
                    s.dialogs.preset_group = group.clone();
                    remember_dialogs(s);
                }
                let d = s.develop_of(id).unwrap_or_default();
                let base = format!("user.{}", crate::presets::slug(&format!("{group}-{name}")));
                let mut pid = base.clone();
                let mut n = 2u32;
                while s.presets.iter().any(|x| x.id == pid) && n < 100_000 {
                    pid = format!("{base}-{n}");
                    n += 1;
                }
                let mut preset = Preset { settings: pick.extract(&d), ..Preset::from_settings(&pid, &name, &group, &d, &[]) };
                // masks are kept as recipes (what to select), never this photo's selections
                lightcraft_develop::presets::to_recipes(&mut preset.settings);
                s.presets.push(preset);
                Ok(json!({"id": pid, "group": group}))
            }
        ),
        cmd!("preset.delete", "Delete Preset", [], None, "{id}", always, |s, p| {
            let pid = str_param(p, "id").ok_or_else(|| bad("preset.delete", "missing id"))?;
            let i = s.presets.iter().position(|x| x.id == pid).ok_or_else(|| bad("preset.delete", "unknown preset"))?;
            if s.presets[i].builtin {
                return Err(bad("preset.delete", "built-in presets can't be deleted"));
            }
            s.presets.remove(i);
            ok()
        }),
        cmd!("preset.favorite", "Favorite Preset", [], None, "{id, favorite?: bool}", always, |s, p| {
            let pid = str_param(p, "id").ok_or_else(|| bad("preset.favorite", "missing id"))?;
            let pr = s.presets.iter_mut().find(|x| x.id == pid).ok_or_else(|| bad("preset.favorite", "unknown preset"))?;
            pr.favorite = bool_or(p, "favorite", !pr.favorite);
            ok()
        }),
        cmd!("preset.rename", "Rename Preset", [], None, "{id, name}", always, |s, p| {
            let c = "preset.rename";
            let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing name"))?.to_string();
            user_preset(s, p, c)?.name = name;
            ok()
        }),
        cmd!("preset.move", "Move Preset to Group", [], None, "{id, group}", always, |s, p| {
            let c = "preset.move";
            let group = str_param(p, "group").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing group"))?;
            // (a group named like one there is is that group: never two "Wedding" groups)
            let group = s.existing_preset_group(group);
            user_preset(s, p, c)?.group = group;
            ok()
        }),
        cmd!(
            "preset.update",
            "Update Preset with Current Settings",
            [],
            None,
            "{id, groups?: [settingsGroup], settings?: [settingItem] (default: the settings the preset already has)} — from the active photo; keeps the preset's name, group, place, favourite and At Import mark; preset.undoUpdate puts the old settings back → {id, groups, settings}",
            has_active,
            |s, p| {
                let c = "preset.update";
                let id = active(s, c)?;
                let current = s.develop_of(id).unwrap_or_default();
                let pick = pick_param(p, c)?;
                let pr = user_preset(s, p, c)?;
                let pick = pick.unwrap_or_else(|| SettingsPick::of_partial(&pr.settings));
                if pick.is_empty() {
                    return Err(bad(c, "tick at least one setting to keep in the preset"));
                }
                let mut settings = pick.extract(&current);
                lightcraft_develop::presets::to_recipes(&mut settings);
                let before = std::mem::replace(&mut pr.settings, settings);
                let pid = pr.id.clone();
                s.preset_undo = Some((pid.clone(), before));
                Ok(json!({"id": pid, "groups": pick.groups, "settings": pick.settings}))
            }
        ),
        cmd!(
            "preset.undoUpdate",
            "Undo Update Preset",
            [],
            None,
            "{} — put back the settings the last Update with Current Settings replaced",
            |s| if s.preset_undo.is_some() { Ok(()) } else { Err("no preset update to undo".into()) },
            |s, _| {
                let c = "preset.undoUpdate";
                let (pid, before) = s.preset_undo.take().ok_or_else(|| bad(c, "no preset update to undo"))?;
                let pr = s.presets.iter_mut().find(|x| x.id == pid).ok_or_else(|| bad(c, "the preset was deleted"))?;
                pr.settings = before;
                Ok(json!({"id": pid}))
            }
        ),
        cmd!(
            "preset.settings",
            "Preset Settings",
            [],
            None,
            "{id} → {groups, settings, ids}: what the preset holds, as the per-setting ticks (Update Preset's dialog starts from it)",
            always,
            |s, p| {
                let c = "preset.settings";
                let pid = str_param(p, "id").ok_or_else(|| bad(c, "missing id"))?;
                let pr = s.presets.iter().find(|x| x.id == pid).ok_or_else(|| bad(c, format!("unknown preset `{pid}`")))?;
                let pick = SettingsPick::of_partial(&pr.settings);
                Ok(json!({"groups": pick.groups, "settings": pick.settings, "ids": pick.to_ids()}))
            }
        ),
        // ---- versions & history
        cmd!("version.create", "Create Version", ["Photo"], Some("Cmd+Shift+S"), "{name?}", has_active, |s, p| {
            let id = active(s, "version.create")?;
            let ph = s.catalog.photo(id).cloned().ok_or_else(|| bad("version.create", "no photo"))?;
            let mut versions = ph.versions.clone();
            let name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("Version {}", versions.len() + 1));
            versions.push(Version { name, created: (s.clock)(), settings: ph.develop.clone(), auto: false });
            s.commit("Create Version", Op::SetVersions { id, versions })?;
            ok()
        }),
        cmd!("version.restore", "Restore Version", [], None, "{index}", has_active, |s, p| {
            let id = active(s, "version.restore")?;
            let i = index_req(p, "index", "version.restore")?;
            let v = s.catalog.photo(id).and_then(|ph| ph.versions.get(i).cloned()).ok_or_else(|| bad("version.restore", "no such version"))?;
            s.set_develop(id, (*v.settings).clone(), &format!("Restore {}", v.name))?;
            ok()
        }),
        cmd!("version.delete", "Delete Version", [], None, "{index}", has_active, |s, p| {
            let id = active(s, "version.delete")?;
            let i = index_req(p, "index", "version.delete")?;
            let mut versions = s.catalog.photo(id).map(|ph| ph.versions.clone()).unwrap_or_default();
            if i >= versions.len() {
                return Err(bad("version.delete", "no such version"));
            }
            versions.remove(i);
            s.commit("Delete Version", Op::SetVersions { id, versions })?;
            ok()
        }),
        cmd!("version.rename", "Rename Version", [], None, "{index, name}", has_active, |s, p| {
            let c = "version.rename";
            let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing name"))?.to_string();
            edit_version(s, p, c, "Rename Version", |v, _| v.name = name)
        }),
        cmd!("version.update", "Update Version with Current Settings", [], None, "{index}", has_active, |s, p| {
            edit_version(s, p, "version.update", "Update Version", |v, current| v.settings = current)
        }),
        cmd!("history.clear", "Clear History", [], None, "{} — keeps the current settings as the only step", has_active, |s, _| {
            let id = active(s, "history.clear")?;
            let d = s.develop_of(id).unwrap_or_default();
            let history = vec![lightcraft_catalog::HistoryStep::new("Cleared History", d)];
            s.commit("Clear History", Op::Batch { ops: vec![Op::SetHistory { id, history }, Op::SetHistoryCursor { id, cursor: None }] })?;
            ok()
        }),
        cmd!(
            "history.restore",
            "Go to History Step",
            [],
            None,
            "{index} — loads that step's settings and moves the History cursor there; later steps stay listed until a new edit",
            has_active,
            |s, p| {
                let c = "history.restore";
                let id = active(s, c)?;
                let i = index_req(p, "index", c)?;
                go_to_history_step(s, c, id, i)
            }
        ),
        cmd!("history.previous", "Previous History Step", [], None, "{} — steps the History cursor back one step", has_history_before, |s, _| {
            let c = "history.previous";
            let id = active(s, c)?;
            let i = s
                .catalog
                .photo(id)
                .and_then(|ph| ph.history_index())
                .and_then(|i| i.checked_sub(1))
                .ok_or_else(|| bad(c, "at the first History step"))?;
            go_to_history_step(s, c, id, i)
        }),
        cmd!("history.next", "Next History Step", [], None, "{} — steps the History cursor forward one step", has_history_after, |s, _| {
            let c = "history.next";
            let id = active(s, c)?;
            let ph = s.catalog.photo(id).ok_or_else(|| bad(c, "no photo"))?;
            let i = ph
                .history_index()
                .map(|i| i.saturating_add(1))
                .filter(|i| *i < ph.history.len())
                .ok_or_else(|| bad(c, "at the latest History step"))?;
            go_to_history_step(s, c, id, i)
        }),
        // ---- interactions (slider drags, brush strokes)
        cmd!(
            "develop.beginInteraction",
            "Begin Interaction",
            [],
            None,
            "{label, ids?} — ids: more photos develop.adjust moves along, live (a multi-photo edit; one undo step)",
            has_active,
            |s, p| {
                let ids: Vec<PhotoId> = p
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_u64).take(100_000).map(PhotoId).collect())
                    .unwrap_or_default();
                s.begin_interaction_with(str_param(p, "label").unwrap_or("Edit"), &ids)?;
                ok()
            }
        ),
        cmd!("develop.endInteraction", "End Interaction", [], None, "{}", always, |s, _| {
            s.end_interaction()?;
            ok()
        }),
        cmd!("develop.cancelInteraction", "Cancel Interaction", [], None, "{}", always, |s, _| {
            s.cancel_interaction()?;
            ok()
        }),
    ]
}

/// Load History step `i` of photo `id` and put the cursor there, without adding a step: the later
/// steps stay listed (one undo step).
fn go_to_history_step(s: &mut Session, c: &str, id: PhotoId, i: usize) -> Result<Value> {
    let ph = s.catalog.photo(id).ok_or_else(|| bad(c, "no photo"))?;
    let st = ph.history.get(i).cloned().ok_or_else(|| bad(c, "no such step"))?;
    let cursor = (i.saturating_add(1) < ph.history.len()).then_some(i);
    let label = format!("History: {}", st.label);
    let edited = Some((s.clock)());
    s.commit(
        &label,
        Op::Batch { ops: vec![Op::SetDevelop { id, settings: st.settings, label: label.clone(), edited }, Op::SetHistoryCursor { id, cursor }] },
    )?;
    ok()
}

fn has_history_before(s: &Session) -> std::result::Result<(), String> {
    let at = s.active().and_then(|id| s.catalog.photo(id)).and_then(|p| p.history_index());
    if at.is_some_and(|i| i > 0) { Ok(()) } else { Err("at the first History step".into()) }
}

fn has_history_after(s: &Session) -> std::result::Result<(), String> {
    let photo = s.active().and_then(|id| s.catalog.photo(id));
    if photo.is_some_and(|p| p.history_index().is_some_and(|i| i.saturating_add(1) < p.history.len())) {
        Ok(())
    } else {
        Err("at the latest History step".into())
    }
}
