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
use std::time::Instant;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// AI masks and Smooth Skin's skin, queued at once (their own workers run them).
    Analyse,
    /// Previews built (or none asked for).
    Previews,
    Done,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Analyse => "masks",
            Stage::Previews => "previews",
            Stage::Done => "done",
        }
    }
}

/// A finishing queue at work.
pub struct Finishing {
    pub plan: FinishPlan,
    ids: HashSet<PhotoId>,
    stage: Stage,
    /// The photos still waiting for their masks or skin (last tick).
    analysing: usize,
    /// The preview build this queue started.
    previews: Option<std::sync::Arc<crate::cmd::previews::PreviewBuild>>,
    /// A preview build of someone else's ran when previews were due: wait for it.
    waited: bool,
    started: Instant,
    guard: TaskGuard,
}

impl std::fmt::Debug for Finishing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finishing").field("plan", &self.plan).field("stage", &self.stage).finish()
    }
}

impl Finishing {
    /// Overall progress, 0..=1: the analysis is the first half (all of it without previews).
    fn fraction(&self) -> f64 {
        let n = self.ids.len().max(1) as f64;
        let analysed = (self.ids.len().saturating_sub(self.analysing)) as f64 / n;
        let built = match (&self.previews, self.stage) {
            (_, Stage::Done) => 1.0,
            (Some(b), _) => b.done.load(std::sync::atomic::Ordering::Relaxed) as f64 / (b.total.max(1) as f64),
            (None, _) => 0.0,
        };
        let analysed = if self.stage == Stage::Analyse { analysed } else { 1.0 };
        if self.plan.preview_size().is_some() { (analysed + built) / 2.0 } else { analysed }
    }

    /// Seconds left, from the pace so far (`None` until there is a pace to go by).
    fn eta(&self) -> Option<u64> {
        let f = self.fraction();
        let spent = self.started.elapsed().as_secs_f64();
        (f > 0.02 && f < 1.0 && spent > 2.0).then(|| (spent / f * (1.0 - f)).round() as u64)
    }

    pub fn json(&self) -> Value {
        json!({
            "running": self.stage != Stage::Done,
            "stage": self.stage.name(),
            "total": self.ids.len(),
            "analysing": self.analysing,
            "previews": self.plan.previews,
            "previewsDone": self.previews.as_ref().map(|b| b.done.load(std::sync::atomic::Ordering::Relaxed)),
            "progress": (self.fraction() * 1000.0).round() / 1000.0,
            "eta": self.eta(),
        })
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
        if let Some(old) = self.finishing.take() {
            for id in old.plan.ids {
                if !plan.ids.contains(&id) {
                    plan.ids.push(id);
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
        let ids: HashSet<PhotoId> = plan.ids.iter().map(|i| PhotoId(*i)).collect();
        let mut f = Finishing { plan, ids, stage: Stage::Analyse, analysing: 0, previews: None, waited: false, started: Instant::now(), guard };
        // the masks of the presets and Smooth Skin's skin: queued now, their workers take it from here
        let list: Vec<PhotoId> = f.plan.ids.iter().map(|i| PhotoId(*i)).collect();
        let need: Vec<PhotoId> =
            list.iter().copied().filter(|id| self.develop_of(*id).is_some_and(|d| d.masks.iter().any(|m| m.needs_compute()))).collect();
        if !need.is_empty() {
            let _ = self.compute_masks(&need, false);
        }
        for id in &list {
            if let Err(e) = self.skin_ensure(*id, false) {
                log::info!("finishing import: Smooth Skin on {}: {e}", id.0);
            }
        }
        f.analysing = self.still_analysing(&f.ids);
        self.finishing = Some(f);
        self.finishing_tick();
        self.finishing.as_ref().map_or_else(|| json!({"running": false}), Finishing::json)
    }

    /// The photos of `ids` whose masks or skin are still being worked out.
    fn still_analysing(&self, ids: &HashSet<PhotoId>) -> usize {
        ids.iter()
            .filter(|id| {
                self.segmenter.skin_pending(**id) || {
                    let one: HashSet<PhotoId> = std::iter::once(**id).collect();
                    self.segmenter.smart.busy_with(&one)
                }
            })
            .count()
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
            if let Some(b) = &f.previews {
                b.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.save_finishing(None);
            return None;
        }
        if f.stage == Stage::Analyse {
            f.analysing = self.still_analysing(&f.ids);
            if f.analysing == 0 {
                f.stage = Stage::Previews;
            }
        }
        if f.stage == Stage::Previews {
            match (f.plan.preview_size(), &f.previews) {
                (None, _) => f.stage = Stage::Done,
                (Some(size), None) => {
                    let busy = self.preview_build.as_ref().is_some_and(|b| !b.finished.load(std::sync::atomic::Ordering::Relaxed));
                    if busy {
                        f.waited = true;
                    } else {
                        let ids: Vec<u64> = f.plan.ids.clone();
                        match self.execute("library.buildPreviews", &json!({"ids": ids, "size": size})) {
                            Ok(_) => f.previews = self.preview_build.clone(),
                            Err(e) => {
                                log::warn!("finishing import: previews: {e}");
                                f.stage = Stage::Done;
                            }
                        }
                    }
                }
                (Some(_), Some(b)) => {
                    if b.finished.load(std::sync::atomic::Ordering::Relaxed) {
                        f.stage = Stage::Done;
                    }
                }
            }
        }
        let frac = f.fraction();
        f.guard.progress((frac * 1000.0) as u64, 1000);
        let detail = match (f.stage, f.eta()) {
            (Stage::Analyse, Some(eta)) => format!("AI masks and skin · about {} left", duration_text(eta)),
            (Stage::Analyse, None) => "AI masks and skin".to_string(),
            (_, Some(eta)) => format!("Previews · about {} left", duration_text(eta)),
            (_, None) => "Previews".to_string(),
        };
        f.guard.detail(&detail);
        if f.stage != Stage::Done {
            self.finishing = Some(f);
            return None;
        }
        self.save_finishing(None);
        let n = f.ids.len();
        let what = if f.plan.preview_size().is_some() { "masks, retouch and previews done" } else { "masks and retouch done" };
        let note = format!("{n} photo{} ready: {what}", if n == 1 { "" } else { "s" });
        log::info!("finishing import: {note} in {}", duration_text(f.started.elapsed().as_secs()));
        self.finishing_note = Some(note.clone());
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
