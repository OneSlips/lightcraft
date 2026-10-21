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

/// A finishing queue at work.
pub struct Finishing {
    pub plan: FinishPlan,
    /// Of the plan's photos, those left over from a queue that was at work when this import came
    /// (an earlier import, or one a restart resumed): said in the row ("544 new + 456 from earlier").
    pub earlier: usize,
    summary: FinishSummary,
    ids: HashSet<PhotoId>,
    stage: Stage,
    /// The photos still waiting for their masks or skin (last tick).
    analysing: usize,
    /// The preview build this queue started.
    previews: Option<std::sync::Arc<crate::cmd::previews::PreviewBuild>>,
    /// A preview build of someone else's ran when previews were due: wait for it.
    waited: bool,
    started: Instant,
    /// When the stage at work began (its pace gives the time left).
    stage_started: Instant,
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

    /// Seconds left, from the pace of the stage at work (`None` until there is a pace to go by).
    /// Previews are the last stage: their pace says what is left; the analysis says only its own
    /// part (previews can't be judged before one is built).
    fn eta(&self) -> Option<u64> {
        let spent = self.stage_started.elapsed().as_secs_f64();
        let n = self.ids.len().max(1) as f64;
        let f = match (self.stage, &self.previews) {
            (Stage::Analyse, _) => (self.ids.len().saturating_sub(self.analysing)) as f64 / n,
            (Stage::Previews, Some(b)) => b.done.load(std::sync::atomic::Ordering::Relaxed) as f64 / (b.total.max(1) as f64),
            _ => return None,
        };
        (f > 0.02 && f < 1.0 && spent > 2.0).then(|| (spent / f * (1.0 - f)).round() as u64)
    }

    pub fn json(&self) -> Value {
        json!({
            "running": self.stage != Stage::Done,
            "stage": self.stage.name(),
            "total": self.ids.len(),
            "masks": self.summary.masks,
            "maskPhotos": self.summary.mask_photos,
            "skin": self.summary.skin,
            "analysing": self.analysing,
            "previews": self.plan.previews,
            "previewsDone": self.previews.as_ref().map(|b| b.done.load(std::sync::atomic::Ordering::Relaxed)),
            "previewsSkipped": self.previews.as_ref().map(|b| b.skipped.load(std::sync::atomic::Ordering::Relaxed)),
            "earlier": self.earlier,
            "progress": (self.fraction() * 1000.0).round() / 1000.0,
            "eta": self.eta(),
        })
    }
}

impl Finishing {
    /// The row's detail: what is at work, how far it is, the time left, and where the photos come
    /// from when leftovers of an earlier queue were taken along.
    pub fn detail(&self) -> String {
        let n = self.ids.len();
        let stage = match (self.stage, &self.previews) {
            (Stage::Analyse, _) => format!("AI masks and skin {} of {n}", n.saturating_sub(self.analysing)),
            (_, Some(b)) => format!("Previews {} of {}", b.done.load(std::sync::atomic::Ordering::Relaxed).min(b.total), b.total),
            (_, None) => "Previews".to_string(),
        };
        let mut parts = vec![stage];
        if let Some(eta) = self.eta() {
            parts.push(format!("about {} left", duration_text(eta)));
        }
        if self.earlier > 0 {
            parts.push(format!("{} new + {} from earlier", n.saturating_sub(self.earlier), self.earlier));
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
            // its preview build is stopped: the merged queue builds them all, once (the photos it
            // built already are skipped)
            if let Some(b) = &old.previews {
                b.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
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
        let ids: HashSet<PhotoId> = plan.ids.iter().map(|i| PhotoId(*i)).collect();
        let mut f = Finishing {
            plan,
            earlier,
            ids,
            stage: Stage::Analyse,
            analysing: 0,
            summary: FinishSummary::default(),
            previews: None,
            waited: false,
            started: Instant::now(),
            stage_started: Instant::now(),
            guard,
        };
        // the masks of the presets and Smooth Skin's skin: queued now, their workers take it from here
        let list: Vec<PhotoId> = f.plan.ids.iter().map(|i| PhotoId(*i)).collect();
        let need: Vec<PhotoId> =
            list.iter().copied().filter(|id| self.develop_of(*id).is_some_and(|d| d.masks.iter().any(|m| m.needs_compute()))).collect();
        f.summary.photos = list.len();
        f.summary.mask_photos = need.len();
        f.summary.masks = need.iter().filter_map(|id| self.develop_of(*id)).map(|d| d.masks.iter().filter(|m| m.needs_compute()).count()).sum();
        if !need.is_empty() {
            let _ = self.compute_masks(&need, false);
        }
        for id in &list {
            match self.skin_ensure(*id, false) {
                Ok(true) => f.summary.skin += 1,
                Ok(false) => {}
                Err(e) => log::info!("finishing import: Smooth Skin on {}: {e}", id.0),
            }
            if let Err(e) = self.retouch_ensure(*id, false) {
                log::info!("finishing import: Retouch on {}: {e}", id.0);
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
                self.segmenter.skin_pending(**id) || self.segmenter.retouch_pending(**id) || {
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
                f.stage_started = Instant::now();
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
                        // one row for the whole queue (this one): the build has none of its own;
                        // photos whose previews are there already are skipped
                        match self.execute("library.buildPreviews", &json!({"ids": ids, "size": size, "row": false})) {
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
        // the whole queue in per cent (its stages differ in size), and in the detail the photos of
        // the stage at work: "Previews 160 of 544"
        f.guard.set_unit(crate::activity::Unit::Percent);
        f.guard.progress((frac * 100.0).round() as u64, 100);
        f.guard.detail(&f.detail());
        if f.stage != Stage::Done {
            self.finishing = Some(f);
            return None;
        }
        self.save_finishing(None);
        let mut summary = f.summary;
        summary.photos = f.ids.len();
        summary.previews = f.previews.is_some();
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
