//! Auto Sync carries the active photo's VALUE to every selected photo (Lightroom Classic; test
//! notes 2026-10-24 note 18, the user's decision "same value for all synced values"): a slider,
//! a console dial, a key or the on-screen display sets each photo to exactly the number the
//! active photo now has. Where a setting is a mode rather than a number, the mode travels and
//! each photo applies it to itself: White Balance As Shot / Auto / a light preset gives each photo
//! its own white; an Upright mode is analysed on each photo; a crop keeps its shape and angle and
//! is fitted to each photo's frame. Quick Develop's +/- buttons stay relative (that is what they
//! mean).
//!
//! White balance is set on each photo's own scale: a raw with a Kelvin scale gets the same Kelvin
//! and tint (its old relative-scale value converted first, never read as Kelvin, which turned
//! photos blue: test notes 2026-10-15); a photo on another scale than the active one (a JPEG
//! next to a raw) has no "same" number and moves by as much as the active photo instead.

use std::collections::HashMap;

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{Crop, DevelopSettings, Upright, WbMode};
use lightcraft_geom::{CropGeometry, Homography, crop_fit_angle};
use lightcraft_pipeline::SourceInfo;

use crate::Session;

/// White balance on this source is edited in Kelvin (a raw with a measured camera scale), not on
/// the relative scale (6500 K / 0 = as shot).
pub(crate) fn kelvin(info: &SourceInfo) -> bool {
    info.raw && !info.relative_wb
}

/// The temperature and tint mode `mode` stands for on a source with `info` (As Shot and the light
/// presets; `None` for Auto and Custom, which need the pixels or the numbers).
pub(crate) fn wb_of_mode(info: &SourceInfo, mode: WbMode) -> Option<(f64, f64)> {
    match mode {
        WbMode::AsShot => Some((info.as_shot_temp, info.as_shot_tint)),
        m => m.preset().map(|(t, ti)| {
            // presets are Kelvin on raws with that scale, relative to daylight (6500 K = as shot) elsewhere
            if kelvin(info) { (t, ti) } else { (6500.0 * t / 5500.0, ti) }
        }),
    }
}

/// What each photo works out for itself before Auto Sync carries an edit to it: its own Auto
/// white balance. (Its own Upright correction is worked out after the edit, in the background in
/// the app: [`crate::upright_jobs`].)
#[derive(Default)]
pub(crate) struct OwnModes {
    pub wb: HashMap<PhotoId, (f64, f64)>,
}

/// `synced.wb` for a photo (source `oinfo`, settings `own` before the sync) when the active
/// photo's white balance went from `old` to `new` (source `ainfo`). `auto`: the photo's own Auto
/// white balance, when `new` is Auto.
pub(crate) fn synced_wb(
    ainfo: &SourceInfo,
    old: &DevelopSettings,
    new: &DevelopSettings,
    oinfo: &SourceInfo,
    own: &DevelopSettings,
    auto: Option<(f64, f64)>,
    synced: &mut DevelopSettings,
) {
    use lightcraft_pipeline::local::{effective_wb, normalize_wb};
    let mut d = own.clone();
    normalize_wb(&mut d, oinfo);
    let mode = new.wb.mode;
    let resolved = match mode {
        WbMode::Auto => auto,
        WbMode::Custom => None,
        m => wb_of_mode(oinfo, m),
    };
    match (mode, resolved) {
        (WbMode::Custom, _) => {
            let (ot, oti) = effective_wb(oinfo, own);
            let (at0, ati0) = effective_wb(ainfo, old);
            let (at, ati) = effective_wb(ainfo, new);
            let same_scale = kelvin(ainfo) == kelvin(oinfo);
            // only what moved: a Temp drag leaves each photo's own tint
            let pick = |own: f64, was: f64, now: f64| {
                if (now - was).abs() < 1e-9 {
                    own
                } else if same_scale {
                    now
                } else {
                    own + (now - was)
                }
            };
            d.wb.temp = pick(ot, at0, at);
            d.wb.tint = pick(oti, ati0, ati);
            d.wb.mode = WbMode::Custom;
        }
        (m, Some((t, ti))) => {
            d.wb.temp = t;
            d.wb.tint = ti;
            d.wb.mode = m;
        }
        // Auto couldn't be worked out on this photo (its pixels didn't load): it keeps its own
        (_, None) => return,
    }
    let (t, ti) = (d.wb.temp, d.wb.tint);
    lightcraft_develop::controls::set(&mut d, "wb.temp", t);
    lightcraft_develop::controls::set(&mut d, "wb.tint", ti);
    synced.wb = d.wb;
}

/// The crop a photo with frame `odims` (oriented width, height) gets when the active photo (frame
/// `adims`) has crop `new`: the same aspect lock, flips and angle; on a frame of the same shape the
/// same rectangle, on another shape the largest crop of the same shape (an uncropped photo: its own
/// frame) at that angle, centred.
pub(crate) fn synced_crop(new: &Crop, adims: (f64, f64), odims: (f64, f64)) -> Crop {
    let (aw, ah) = (adims.0.max(1.0), adims.1.max(1.0));
    let (ow, oh) = (odims.0.max(1.0), odims.1.max(1.0));
    let (ar, or) = (aw / ah, ow / oh);
    if (ar - or).abs() <= 0.005 * ar {
        return *new;
    }
    // a portrait photo takes a landscape shape turned
    let turned = (ar > 1.0) != (or > 1.0);
    let aspect = new.aspect.map(|(x, y)| if turned { (y, x) } else { (x, y) });
    let angle = new.geometry.angle;
    let full = crop_fit_angle(aw, ah, angle, None).rect;
    let r = new.geometry.rect;
    let close = |a: f64, b: f64| (a - b).abs() < 1e-3;
    let uncropped = new.aspect.is_none() && close(r.x0, full.x0) && close(r.y0, full.y0) && close(r.x1, full.x1) && close(r.y1, full.y1);
    let shape = if uncropped {
        None
    } else {
        let px = new.geometry.rect_px(aw, ah);
        let s = aspect.map(|(x, y)| f64::from(x) / f64::from(y.max(1))).or(Some(px.width() / px.height()));
        s.filter(|s| s.is_finite() && *s > 0.0).map(|s| if turned && aspect.is_none() { 1.0 / s } else { s })
    };
    let geometry: CropGeometry = crop_fit_angle(ow, oh, angle, shape);
    Crop { geometry, aspect, flip_h: new.flip_h, flip_v: new.flip_v }
}

impl Session {
    /// What the other selected photos must work out on themselves before Auto Sync carries the
    /// active photo's edit (`id`, to `new`) over: their own Auto white balance. Empty when Auto
    /// Sync won't run or the edit doesn't change such a mode.
    pub(crate) fn own_modes(&mut self, id: PhotoId, new: &DevelopSettings, label: &str) -> OwnModes {
        let mut own = OwnModes::default();
        if !self.auto_sync
            || self.active() != Some(id)
            || self.selection.ids.len() < 2
            || label.starts_with("History:")
            || label.starts_with("Restore ")
        {
            return own;
        }
        let Some(old) = self.develop_of(id) else { return own };
        let auto_wb = new.wb.mode == WbMode::Auto && old.wb != new.wb;
        if !auto_wb {
            return own;
        }
        let others: Vec<PhotoId> = self.selection.ids.iter().copied().filter(|x| *x != id).collect();
        for x in others {
            match self.source_now(x, crate::media::SourceLevel::Thumb) {
                Ok(src) => {
                    let info = self.source_info(x);
                    own.wb.insert(x, lightcraft_pipeline::auto::auto_wb(&src, &info));
                }
                Err(e) => log::warn!("Auto Sync: Auto white balance on photo {}: {e}", x.0),
            }
        }
        own
    }

    /// The Upright correction mode `mode` finds on photo `id` (its own pixels and lens settings);
    /// `None` for Off and Guided.
    pub(crate) fn upright_for(&mut self, id: PhotoId, mode: Upright) -> Result<Option<Homography>, String> {
        if matches!(mode, Upright::Off | Upright::Guided) {
            return Ok(None);
        }
        let d = self.develop_of(id).unwrap_or_default();
        let src = self.source_now(id, crate::media::SourceLevel::Preview)?;
        let info = self.source_info(id);
        let a = lightcraft_pipeline::upright::analyze_source(&src, &info, &d);
        Ok(Some(lightcraft_pipeline::upright::solve(mode, &a)))
    }
}

/// The last History batch id handed out (process-wide, so two sessions never share one).
static LAST_BATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A new History batch id: milliseconds since 1970, above every id handed out before (unique across
/// sessions and restarts, as long as the clock doesn't run back further than the ids went).
fn next_batch_id() -> u64 {
    let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    use std::sync::atomic::Ordering::SeqCst;
    let mut last = LAST_BATCH.load(SeqCst);
    loop {
        let id = now.max(last.saturating_add(1));
        match LAST_BATCH.compare_exchange_weak(last, id, SeqCst, SeqCst) {
            Ok(_) => return id,
            Err(seen) => last = seen,
        }
    }
}

/// Photos whose History `op` pushes a step on that has no batch yet (batches nested at most 16 deep).
fn unbatched_steps(op: &lightcraft_catalog::Op, out: &mut std::collections::HashSet<PhotoId>, depth: usize) {
    match op {
        lightcraft_catalog::Op::PushHistory { id, step } if step.batch.is_none() => {
            out.insert(*id);
        }
        lightcraft_catalog::Op::Batch { ops } if depth < 16 => ops.iter().for_each(|o| unbatched_steps(o, out, depth + 1)),
        _ => {}
    }
}

fn set_batch(op: &mut lightcraft_catalog::Op, batch: u64, depth: usize) {
    match op {
        lightcraft_catalog::Op::PushHistory { step, .. } if step.batch.is_none() => step.batch = Some(batch),
        lightcraft_catalog::Op::Batch { ops } if depth < 16 => ops.iter_mut().for_each(|o| set_batch(o, batch, depth + 1)),
        _ => {}
    }
}

impl Session {
    /// One edit that adds History steps on several photos (Auto Sync, Sync Settings, a preset or
    /// Quick Develop on a selection, a multi-photo drag): the same batch id on each of its steps
    /// (groundwork for note 22, the whole selection back to a step).
    pub(crate) fn mark_batch(&mut self, op: &mut lightcraft_catalog::Op) {
        let mut photos = std::collections::HashSet::new();
        unbatched_steps(op, &mut photos, 0);
        if photos.len() >= 2 {
            set_batch(op, next_batch_id(), 0);
        }
    }
}
