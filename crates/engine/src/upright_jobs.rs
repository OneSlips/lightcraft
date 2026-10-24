//! Upright analysed on each photo, as Lightroom does (test notes 2026-10-24 note 4): an Upright mode
//! (Auto, Level, Vertical, Full) that reaches a photo without its own correction — Auto Sync, Sync
//! Settings, Paste, a preset, settings read from a file — is a recipe, and the photo's own pixels
//! decide the correction. After every committed edit, the photos whose settings name such a mode but
//! hold no correction are analysed: with [`UprightJobs::background`] (the desktop app) on a worker
//! thread, one activity row for the batch, the results merged into the photos' current settings as
//! they arrive ([`Session::upright_poll`], via [`Session::merge_background`]: an edit made
//! meanwhile is kept, and no undo step of its own); without it (CLI, MCP, tests) at once.
//!
//! Until its correction arrives a photo renders with one worked out for the render
//! ([`lightcraft_pipeline::upright::resolve`]), so nothing waits on the analysis.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::{DevelopSettings, Upright};
use lightcraft_geom::Homography;

use crate::Session;
use crate::activity::{Cancel, TaskGuard};

/// Whether settings `d` name an automatic Upright mode but hold no correction for it.
pub(crate) fn needs_analysis(d: &DevelopSettings) -> bool {
    matches!(d.geometry.upright, Upright::Auto | Upright::Level | Upright::Vertical | Upright::Full)
        && d.geometry.upright_transform.is_none()
        && d.section_enabled("geometry")
}

/// Whether committed op `op` leaves some photo needing [`needs_analysis`] (cheap: no session).
pub(crate) fn needs_any(op: &Op) -> bool {
    let mut set = Vec::new();
    developed(op, &mut set, 0);
    set.iter().any(|(_, d)| needs_analysis(d))
}

/// What a worker found for a photo.
struct Found {
    id: PhotoId,
    mode: Upright,
    result: Result<Homography, String>,
}

/// The Upright analyses in flight.
#[derive(Default)]
pub struct UprightJobs {
    /// Analyse on a worker thread (the desktop app); else at once.
    pub background: bool,
    /// Photos queued or being analysed, and for which mode.
    pending: HashMap<PhotoId, Upright>,
    rx: Option<Receiver<Found>>,
    tx: Option<Sender<Found>>,
    /// The batch's activity row, while photos are pending.
    guard: Option<TaskGuard>,
    cancel: Option<Arc<AtomicBool>>,
}

impl UprightJobs {
    /// Photos still waiting for their analysis.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// Photos whose settings `op` sets, with those settings (batches nested at most 16 deep).
fn developed(op: &Op, out: &mut Vec<(PhotoId, Arc<DevelopSettings>)>, depth: usize) {
    match op {
        Op::SetDevelop { id, settings, .. } => out.push((*id, settings.clone())),
        Op::Batch { ops } if depth < 16 => ops.iter().for_each(|o| developed(o, out, depth + 1)),
        _ => {}
    }
}

/// `t` into settings `d` when they still ask for mode `mode` without a correction.
fn fill(d: &mut DevelopSettings, mode: Upright, t: Homography) -> bool {
    if d.geometry.upright != mode || d.geometry.upright_transform.is_some() {
        return false;
    }
    d.geometry.upright_transform = Some(t);
    true
}

impl Session {
    /// After `op` was committed: analyse the photos it left with an Upright mode and no correction.
    pub(crate) fn upright_after(&mut self, op: &Op) {
        let mut set = Vec::new();
        developed(op, &mut set, 0);
        let todo: Vec<(PhotoId, Upright)> = set
            .into_iter()
            .filter(|(id, d)| needs_analysis(d) && self.upright_jobs.pending.get(id) != Some(&d.geometry.upright))
            .map(|(id, d)| (id, d.geometry.upright))
            .collect();
        if todo.is_empty() {
            return;
        }
        if self.upright_jobs.background && cfg!(not(target_arch = "wasm32")) {
            self.upright_queue(todo);
        } else {
            for (id, mode) in todo {
                let result = self.upright_for(id, mode).and_then(|t| t.ok_or_else(|| "no correction for this mode".to_string()));
                self.upright_land(Found { id, mode, result });
            }
        }
    }

    /// Give photo `f.id` what its analysis found.
    fn upright_land(&mut self, f: Found) {
        match f.result {
            Ok(t) => {
                let Some(photo) = self.catalog.photo(f.id).cloned() else { return };
                let (sw, sh) = (photo.width.max(1) as usize, photo.height.max(1) as usize);
                let lens = photo.embedded_lens;
                let mode = f.mode;
                // the correction, and the crop Constrain Crop made fitted to it (a drawn crop is left)
                let patch = move |d: &mut DevelopSettings| {
                    let auto_crop = d.geometry.constrain_crop.then(|| crate::auto_fitted_crop(d, sw, sh, lens.as_ref())).flatten();
                    if !fill(d, mode, t) {
                        return false;
                    }
                    if let Some(whole) = auto_crop {
                        let mut at = d.clone();
                        at.crop.geometry = whole;
                        d.crop.geometry.rect = lightcraft_pipeline::geometry::Frame::with_lens(sw, sh, &at, true, lens.as_ref()).crop.rect;
                    }
                    true
                };
                self.merge_background(f.id, "Upright", false, &patch);
            }
            Err(e) => log::warn!("Upright on photo {}: {e}", f.id.0),
        }
    }

    /// Analyse `todo` on a worker thread (one activity row for all of them).
    #[cfg(not(target_arch = "wasm32"))]
    fn upright_queue(&mut self, todo: Vec<(PhotoId, Upright)>) {
        let mut jobs = Vec::new();
        for (id, mode) in todo {
            let Some(p) = self.catalog.photo(id).cloned() else { continue };
            let d = self.develop_of(id).unwrap_or_default();
            let src = self.media.source_ref(&p, crate::media::SourceLevel::Preview);
            let info = self.source_info(id);
            jobs.push((id, mode, src, info, d));
        }
        if jobs.is_empty() {
            return;
        }
        let uj = &mut self.upright_jobs;
        if uj.tx.is_none() {
            let (tx, rx) = channel();
            uj.tx = Some(tx);
            uj.rx = Some(rx);
        }
        let Some(tx) = uj.tx.clone() else { return };
        for (id, mode, ..) in &jobs {
            uj.pending.insert(*id, *mode);
        }
        let cancel = uj.cancel.get_or_insert_with(|| Arc::new(AtomicBool::new(false))).clone();
        let total = uj.pending.len() as u64;
        let guard = uj.guard.get_or_insert_with(|| self.activity.start_batch("upright", "Upright", Cancel::Flag(cancel.clone())));
        guard.progress(guard.done(), total);
        let spawned = std::thread::Builder::new().name("lc-upright".into()).spawn(move || {
            for (id, mode, src, info, d) in jobs {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(Found { id, mode, result: Err("cancelled".into()) });
                    continue;
                }
                // background work: what is on screen first
                let _ = crate::prep::wait_for_pictures(&|| cancel.load(Ordering::Relaxed), std::time::Duration::from_secs(30));
                let result = src.load_source().map(|r| {
                    let a = lightcraft_pipeline::upright::analyze_source(&r.image, &info, &d);
                    lightcraft_pipeline::upright::solve(mode, &a)
                });
                if tx.send(Found { id, mode, result }).is_err() {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Upright: no worker thread ({e}); analysing now");
            let ids: Vec<(PhotoId, Upright)> = self.upright_jobs.pending.drain().collect();
            self.upright_jobs.guard = None;
            for (id, mode) in ids {
                let result = self.upright_for(id, mode).and_then(|t| t.ok_or_else(|| "no correction for this mode".to_string()));
                self.upright_land(Found { id, mode, result });
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn upright_queue(&mut self, todo: Vec<(PhotoId, Upright)>) {
        for (id, mode) in todo {
            let result = self.upright_for(id, mode).and_then(|t| t.ok_or_else(|| "no correction for this mode".to_string()));
            self.upright_land(Found { id, mode, result });
        }
    }

    /// Apply the background analyses that finished (the desktop app calls this every frame) → whether
    /// a photo's settings changed.
    pub fn upright_poll(&mut self) -> bool {
        let mut found = Vec::new();
        if let Some(rx) = &self.upright_jobs.rx {
            while let Ok(f) = rx.try_recv() {
                found.push(f);
            }
        }
        let changed = !found.is_empty();
        for f in found {
            if self.upright_jobs.pending.get(&f.id) == Some(&f.mode) {
                self.upright_jobs.pending.remove(&f.id);
            }
            if let Some(g) = &self.upright_jobs.guard {
                match &f.result {
                    Ok(_) => g.item_done(),
                    Err(e) if e == "cancelled" => {}
                    Err(e) => g.item_failed(e),
                }
            }
            self.upright_land(f);
        }
        if self.upright_jobs.pending.is_empty() {
            self.upright_jobs.guard = None;
            self.upright_jobs.cancel = None;
        }
        changed
    }
}
