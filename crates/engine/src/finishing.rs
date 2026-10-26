//! Finishing an import, unattended: after the photos are in, one queue gets them ready for final
//! edits — AI masks of the presets computed, Smooth Skin's skin found, previews built — while the
//! app stays usable. One row in the activity stack shows the whole queue (overall progress and
//! the time left); the work itself runs where it always does (the mask and skin workers, the
//! preview build). The plan is kept in the library (`import-finishing.json`) until it is done,
//! so a restart picks it up where it stopped. When everything is done the session holds a note
//! for the user ("569 photos ready").
//!
//! Driven by [`Session::finishing_tick`] (every frame in the app; in a loop for the CLI and tests).

use std::collections::HashSet;
use web_time::Instant;

use lightcraft_catalog::PhotoId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Session;
use crate::activity::{Cancel, TaskGuard};

/// The library file that keeps an unfinished plan across restarts.
pub const FILE: &str = "import-finishing.json";

/// What an import asked to be finished.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FinishPlan {
    pub ids: Vec<u64>,
    /// `none` | `standard` | `full` (1:1).
    pub previews: String,
    /// When the import started it (ISO 8601).
    pub started: String,
}

impl FinishPlan {
    fn preview_size(&self) -> Option<&'static str> {
        match self.previews.as_str() {
            "standard" => Some("standard"),
            "full" | "1:1" => Some("full"),
            _ => None,
        }
    }
}

/// How many photos are at work at once: one analysed while the one before it has its previews
/// built (the workers stay busy), and never more, so the photos near the one on screen are done
/// before any photo further away is touched (note 5 of 2026-10-25).
const AT_ONCE: usize = 2;

/// The neighbours each way that are finished, all of them, before a photo further away is started.
pub const NEAR: usize = 5;

/// Where a photo at work is.
enum Step {
    /// Its AI masks, Smooth Skin's skin and Retouch parts are being worked out (their workers run them).
    Analysing,
    /// Its previews are being built (on a thread of their own; set when done).
    Previewing(std::sync::Arc<std::sync::atomic::AtomicBool>),
}

/// What a finished queue did, for the note: only what really happened is said (an import with a
/// preset without AI masks says nothing about masks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinishSummary {
    /// Photos of the import.
    pub photos: usize,
    /// Photos whose presets had AI masks, and how many masks were computed in all.
    pub mask_photos: usize,
    pub masks: usize,
    /// Photos whose Smooth Skin found their skin.
    pub skin: usize,
    /// Previews were built.
    pub previews: bool,
}

impl FinishSummary {
    /// Anything to tell: masks, skin or previews.
    pub fn did_something(&self) -> bool {
        self.masks > 0 || self.skin > 0 || self.previews
    }

    /// The note in English (the app says it in the UI language).
    pub fn note(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.masks > 0 {
            parts.push(format!("{} on {}", plural(self.masks, "AI mask", "AI masks"), plural(self.mask_photos, "photo", "photos")));
        }
        if self.skin > 0 {
            parts.push(format!("Smooth Skin on {}", plural(self.skin, "photo", "photos")));
        }
        if self.previews {
            parts.push("previews built".to_string());
        }
        let head = format!("{} ready", plural(self.photos, "photo", "photos"));
        if parts.is_empty() { head } else { format!("{head}: {}", parts.join(", ")) }
    }
}

/// A finishing queue at work: photo by photo, each one masks → skin → previews, the photo on
/// screen first, then its neighbours both ways, then outward, re-aimed as the user moves (note 5 of
/// 2026-10-25: "the photo I'm on is ready first").
pub struct Finishing {
    pub plan: FinishPlan,
    /// Of the plan's photos, those left over from a queue that was at work when this import came
    /// (an earlier import, or one a restart resumed): said in the row ("544 new + 456 from earlier").
    pub earlier: usize,
    summary: FinishSummary,
    /// Not started yet.
    waiting: Vec<PhotoId>,
    /// At work (at most [`AT_ONCE`]), in the order they were started.
    at_work: Vec<(PhotoId, Step)>,
    /// Photos analysed, previews built (or there already), photos done: of the plan's.
    analysed: usize,
    built: usize,
    skipped: usize,
    done: usize,
    total: usize,
    /// The order photos were started in (tests, `library.finishProgress` → `order`, the first few).
    pub started_order: Vec<PhotoId>,
    /// Stops the preview threads with the row's ✕.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    started: Instant,
    guard: TaskGuard,
}

impl std::fmt::Debug for Finishing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finishing").field("plan", &self.plan).field("done", &self.done).field("total", &self.total).finish()
    }
}

impl Finishing {
    fn wants_previews(&self) -> bool {
        self.plan.preview_size().is_some()
    }

    /// Overall progress, 0..=1: each photo's analysis and its previews (all of it without previews).
    fn fraction(&self) -> f64 {
        let n = self.total.max(1) as f64;
        if self.wants_previews() { (self.analysed + self.built) as f64 / (2.0 * n) } else { self.analysed as f64 / n }
    }

    /// Seconds left, from the pace so far (`None` until there is a pace to go by).
    fn eta(&self) -> Option<u64> {
        let spent = self.started.elapsed().as_secs_f64();
        let f = self.fraction();
        (f > 0.02 && f < 1.0 && spent > 2.0).then(|| (spent / f * (1.0 - f)).round() as u64)
    }

    fn stage(&self) -> &'static str {
        if self.done >= self.total {
            "done"
        } else if !self.waiting.is_empty() || self.at_work.iter().any(|(_, s)| matches!(s, Step::Analysing)) {
            "masks"
        } else {
            "previews"
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "running": self.done < self.total,
            "stage": self.stage(),
            "total": self.total,
            "done": self.done,
            "masks": self.summary.masks,
            "maskPhotos": self.summary.mask_photos,
            "skin": self.summary.skin,
            "analysing": self.total.saturating_sub(self.analysed),
            "previews": self.plan.previews,
            "previewsDone": self.wants_previews().then_some(self.built),
            "previewsSkipped": self.wants_previews().then_some(self.skipped),
            "earlier": self.earlier,
            "progress": (self.fraction() * 1000.0).round() / 1000.0,
            "eta": self.eta(),
            "order": self.started_order.iter().take(64).map(|p| p.0).collect::<Vec<_>>(),
        })
    }

    /// The row's detail: the photos done, the time left, and where the photos come from when
    /// leftovers of an earlier queue were taken along.
    pub fn detail(&self) -> String {
        let mut parts = vec![format!("Photo {} of {}", (self.done + 1).min(self.total), self.total)];
        if let Some(eta) = self.eta() {
            parts.push(format!("about {} left", duration_text(eta)));
        }
        if self.earlier > 0 {
            parts.push(format!("{} new + {} from earlier", self.total.saturating_sub(self.earlier), self.earlier));
        }
        parts.join(" · ")
    }
}

/// "1 h 5 min", "12 min", "40 s".
pub fn duration_text(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{} h {} min", s / 3600, (s % 3600) / 60),
        s if s >= 60 => format!("{} min", s.div_ceil(60)),
        s => format!("{s} s"),
    }
}

impl Session {
    /// Start finishing `plan` (replacing a queue at work: its photos are taken along). Saved in the
    /// library so a restart resumes it.
    pub fn finish_import(&mut self, mut plan: FinishPlan) -> Value {
        let mut earlier = 0;
        if let Some(old) = self.finishing.take() {
            // its preview builds are stopped: the merged queue builds them all, once (the photos
            // built already are skipped)
            old.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let new: HashSet<u64> = plan.ids.iter().copied().collect();
            for id in old.plan.ids {
                if !new.contains(&id) && self.catalog.photo(PhotoId(id)).is_some() {
                    plan.ids.push(id);
                    earlier += 1;
                }
            }
        }
        plan.ids.retain(|id| self.catalog.photo(PhotoId(*id)).is_some());
        if plan.started.is_empty() {
            plan.started = (self.clock)();
        }
        self.finish_pending = None;
        if plan.ids.is_empty() {
            return json!({"running": false, "total": 0});
        }
        self.save_finishing(Some(&plan));
        let guard = self.activity.start("finishing", "Getting photos ready", Cancel::Yes);
        let list: Vec<PhotoId> = plan.ids.iter().map(|i| PhotoId(*i)).collect();
        let mut summary = FinishSummary { photos: list.len(), ..Default::default() };
        for id in &list {
            if let Some(d) = self.develop_of(*id) {
                let n = d.masks.iter().filter(|m| m.needs_compute()).count();
                if n > 0 {
                    summary.mask_photos += 1;
                    summary.masks += n;
                }
            }
        }
        let total = list.len();
        self.finishing = Some(Finishing {
            plan,
            earlier,
            summary,
            waiting: list,
            at_work: Vec::new(),
            analysed: 0,
            built: 0,
            skipped: 0,
            done: 0,
            total,
            started_order: Vec::new(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            started: Instant::now(),
            guard,
        });
        self.finishing_tick();
        self.finishing.as_ref().map_or_else(|| json!({"running": false}), Finishing::json)
    }

    /// Whether photo `id`'s masks, skin or retouch parts are still being worked out.
    fn still_analysing(&self, id: PhotoId) -> bool {
        self.segmenter.skin_pending(id) || self.segmenter.retouch_pending(id) || {
            let one: HashSet<PhotoId> = std::iter::once(id).collect();
            self.segmenter.smart.busy_with(&one)
        }
    }

    /// The next photo of `waiting` to start: the one on screen, then its neighbours in the order the
    /// photos are shown, both ways (forward first), then outward; with no photo on screen, from the
    /// first photo of the order. Photos not shown come last, in the plan's order.
    fn next_to_finish(&mut self, waiting: &[PhotoId], at_work: &[(PhotoId, Step)]) -> Option<usize> {
        let shown = self.visible_cloned();
        let at: std::collections::HashMap<PhotoId, usize> = shown.iter().enumerate().map(|(i, p)| (*p, i)).collect();
        let focus = self.active().and_then(|a| at.get(&a).copied()).unwrap_or(0);
        let rank = |p: &PhotoId| match at.get(p) {
            // (behind the photo on screen ranks just after the same distance ahead)
            Some(&i) => (if i >= focus { (i - focus) * 2 } else { (focus - i) * 2 + 1 }) as u64,
            None => u64::MAX,
        };
        let near = |p: &PhotoId| at.get(p).is_some_and(|&i| i.abs_diff(focus) <= NEAR);
        let (k, next) = waiting.iter().enumerate().min_by_key(|(k, p)| (rank(p), *k))?;
        // a photo further away waits until the near ones at work are done
        (near(next) || !at_work.iter().any(|(p, _)| near(p))).then_some(k)
    }

    /// Start photo `id`: its masks, skin and retouch parts go to their workers as background work
    /// (the user's own requests go first, note 6 of 2026-10-25).
    fn start_finishing(&mut self, id: PhotoId, summary: &mut FinishSummary) {
        self.segmenter.low_priority = true;
        if self.develop_of(id).is_some_and(|d| d.masks.iter().any(|m| m.needs_compute())) {
            let _ = self.compute_masks(&[id], false);
        }
        match self.skin_ensure(id, false) {
            Ok(true) => summary.skin += 1,
            Ok(false) => {}
            Err(e) => log::info!("finishing import: Smooth Skin on {}: {e}", id.0),
        }
        if let Err(e) = self.retouch_ensure(id, false) {
            log::info!("finishing import: Retouch on {}: {e}", id.0);
        }
        self.segmenter.low_priority = false;
    }

    /// Build photo `id`'s previews on a thread of its own (the photo on screen first: it waits while
    /// that is made). `None`: nothing to build (there already, or no such photo); else the flag set
    /// when it is done.
    fn build_previews_of(
        &mut self,
        id: PhotoId,
        size: &str,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        let edge = (size != "full").then(|| crate::cmd::previews::standard_edge(self));
        let jobs = crate::cmd::previews::jobs_for(self, id, edge);
        if jobs.is_empty() || crate::cmd::previews::already_built(&jobs) {
            return None;
        }
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (done, cancel) = (flag.clone(), cancel.clone());
        let work = move || {
            if !crate::prep::wait_for_foreground(&cancel) {
                let build = || crate::cmd::previews::run_photo(jobs);
                if !crate::guard::catch("building previews", || crate::memory::in_background(build)).unwrap_or(false) {
                    log::info!("finishing import: previews of {} not built", id.0);
                }
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let flag2 = flag.clone();
            let spawned = std::thread::Builder::new().name("finishing-previews".into()).spawn(work);
            if let Err(e) = spawned {
                log::warn!("finishing import: previews of {}: {e}", id.0);
                flag2.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        #[cfg(target_arch = "wasm32")]
        work();
        Some(flag)
    }

    /// Advance the finishing queue (cheap; call every frame). Returns the note for the user when
    /// it has just finished ("569 photos ready…"), also kept in [`Session::finishing_note`].
    pub fn finishing_tick(&mut self) -> Option<String> {
        if self.finishing.is_none()
            && let Some(plan) = self.finish_pending.take()
        {
            log::info!("resuming the unfinished import of {} photo(s)", plan.ids.len());
            self.finish_import(plan);
        }
        let mut f = self.finishing.take()?;
        if f.guard.is_cancelled() {
            f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.save_finishing(None);
            return None;
        }
        let size = f.plan.preview_size();
        // the photos at work move on: analysed → previews → done
        let mut at_work = std::mem::take(&mut f.at_work);
        let mut k = 0;
        while k < at_work.len() {
            let Some((id, step)) = at_work.get(k) else { break };
            let id = *id;
            let next = match step {
                Step::Analysing if self.still_analysing(id) => None,
                Step::Analysing => {
                    f.analysed += 1;
                    match size.and_then(|sz| self.build_previews_of(id, sz, &f.cancel)) {
                        Some(flag) => Some(Some(Step::Previewing(flag))),
                        None => {
                            if size.is_some() {
                                f.built += 1;
                                f.skipped += 1;
                            }
                            Some(None)
                        }
                    }
                }
                Step::Previewing(flag) if flag.load(std::sync::atomic::Ordering::Relaxed) => {
                    f.built += 1;
                    Some(None)
                }
                Step::Previewing(_) => None,
            };
            match next {
                None => k += 1,
                Some(Some(step)) => {
                    if let Some(slot) = at_work.get_mut(k) {
                        slot.1 = step;
                    }
                    k += 1;
                }
                Some(None) => {
                    at_work.remove(k);
                    f.done += 1;
                }
            }
        }
        // the next photos, nearest the one on screen first
        let mut summary = f.summary;
        while at_work.len() < AT_ONCE && !f.waiting.is_empty() {
            let Some(k) = self.next_to_finish(&f.waiting, &at_work) else { break };
            let id = f.waiting.remove(k);
            if self.catalog.photo(id).is_none() {
                // (removed meanwhile)
                f.analysed += 1;
                f.built += usize::from(size.is_some());
                f.done += 1;
                continue;
            }
            f.started_order.push(id);
            self.start_finishing(id, &mut summary);
            at_work.push((id, Step::Analysing));
        }
        f.summary = summary;
        f.at_work = at_work;
        let frac = f.fraction();
        f.guard.set_unit(crate::activity::Unit::Percent);
        f.guard.progress((frac * 100.0).round() as u64, 100);
        f.guard.detail(&f.detail());
        if f.done < f.total {
            self.finishing = Some(f);
            return None;
        }
        self.save_finishing(None);
        let mut summary = f.summary;
        summary.photos = f.total;
        summary.previews = size.is_some();
        let note = summary.note();
        log::info!("finishing import: {note} in {}", duration_text(f.started.elapsed().as_secs()));
        self.finishing_note = Some(note.clone());
        self.finishing_done = Some(summary);
        Some(note)
    }

    /// Keep (`Some`) or clear the plan in the library, for a restart.
    fn save_finishing(&mut self, plan: Option<&FinishPlan>) {
        let Some(lib) = self.library.as_mut() else { return };
        // (cleared by an empty plan: the store has no remove)
        let empty = FinishPlan::default();
        let r = lib.files.write_atomic(FILE, &serde_json::to_vec_pretty(plan.unwrap_or(&empty)).unwrap_or_default());
        if let Err(e) = r {
            log::warn!("finishing import: {FILE}: {e}");
        }
    }

    /// When a library opens: an unfinished plan it kept, to resume with the next tick.
    pub(crate) fn load_finishing(&mut self) {
        self.finishing = None;
        self.finish_pending = None;
        let Some(lib) = self.library.as_mut() else { return };
        let Ok(Some(bytes)) = lib.files.read(FILE) else { return };
        match serde_json::from_slice::<FinishPlan>(&bytes) {
            Ok(plan) if !plan.ids.is_empty() => self.finish_pending = Some(plan),
            Ok(_) => {}
            Err(e) => log::warn!("finishing import: {FILE} unreadable, left alone: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FinishSummary;

    /// LR-IMP-NOTICE: the note names masks only when there were some, with the counts.
    #[test]
    fn the_note_says_only_what_was_done() {
        let none = FinishSummary { photos: 1, ..Default::default() };
        assert!(!none.did_something());
        assert_eq!(none.note(), "1 photo ready");
        let masks = FinishSummary { photos: 12, mask_photos: 12, masks: 24, ..Default::default() };
        assert!(masks.did_something());
        assert_eq!(masks.note(), "12 photos ready: 24 AI masks on 12 photos");
        let all = FinishSummary { photos: 2, mask_photos: 1, masks: 1, skin: 2, previews: true };
        assert_eq!(all.note(), "2 photos ready: 1 AI mask on 1 photo, Smooth Skin on 2 photos, previews built");
        let previews = FinishSummary { photos: 3, previews: true, ..Default::default() };
        assert_eq!(previews.note(), "3 photos ready: previews built");
    }
}
