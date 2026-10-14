//! How long the loupe takes to show each photo it is stepped to, measured on the frames it draws.
//!
//! Culling is stepping through photos, so the number that matters is the time from the step (the
//! first frame that draws the new photo) to the first frame that shows something of it, to the
//! first frame with a full-size picture (the camera's embedded JPEG or a cached render) and to the
//! first frame with LightCraft's own render. `ui.inspect` → `perf.loupeSteps` reports the latest
//! steps; `perf-scratch`-style drivers turn them into medians and p95 without the control
//! channel's own round trip in the numbers.

use std::collections::VecDeque;

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

/// Steps kept for `ui.inspect`.
const KEEP: usize = 256;

/// One photo's way onto the screen (times in ms since the frame it was stepped to).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepTiming {
    pub photo: PhotoId,
    /// First frame that drew anything of the photo (often the grid thumbnail, magnified).
    pub any_ms: Option<f64>,
    /// What that was.
    pub any_source: &'static str,
    /// First frame with a full-size stand-in or better: the camera's JPEG, a cached render, the render.
    pub full_ms: Option<f64>,
    pub full_source: &'static str,
    /// First frame with LightCraft's own render of the photo (`render` or `cached`).
    pub sharp_ms: Option<f64>,
    /// How long the photo stayed in the loupe (ms): a step taken before `sharp_ms` was skipped past.
    pub dwell_ms: f64,
}

impl StepTiming {
    fn to_json(self) -> Value {
        json!({
            "photo": self.photo.0,
            "anyMs": self.any_ms,
            "anySource": self.any_source,
            "fullMs": self.full_ms,
            "fullSource": self.full_source,
            "sharpMs": self.sharp_ms,
            "dwellMs": self.dwell_ms,
        })
    }
}

/// The loupe's step timings.
#[derive(Debug, Default)]
pub struct LoupeTiming {
    current: Option<(f64, StepTiming)>,
    done: VecDeque<StepTiming>,
}

/// Sources that show the photo at full size: the camera's JPEG, a cached or a fresh render.
fn full_size(source: &str) -> bool {
    matches!(source, "embedded" | "cached" | "render" | "preview")
}

impl LoupeTiming {
    /// The loupe drew `photo` from `source` (what [`crate::panels::detail`] reports in
    /// `loupe_shown`) in the frame that started at `now` (s).
    pub fn observe(&mut self, photo: PhotoId, source: &'static str, now: f64) {
        if self.current.as_ref().is_none_or(|(_, t)| t.photo != photo) {
            self.finish(now);
            let t = StepTiming { photo, any_ms: None, any_source: "none", full_ms: None, full_source: "none", sharp_ms: None, dwell_ms: 0.0 };
            self.current = Some((now, t));
        }
        let Some((since, t)) = self.current.as_mut() else { return };
        let ms = ((now - *since) * 1000.0).max(0.0);
        if !ms.is_finite() {
            return;
        }
        t.dwell_ms = ms;
        if source != "none" && t.any_ms.is_none() {
            t.any_ms = Some(ms);
            t.any_source = source;
        }
        if full_size(source) && t.full_ms.is_none() {
            t.full_ms = Some(ms);
            t.full_source = source;
        }
        if matches!(source, "render" | "cached") && t.sharp_ms.is_none() {
            t.sharp_ms = Some(ms);
        }
    }

    /// The loupe is not showing a photo any more (the grid is up).
    pub fn left(&mut self, now: f64) {
        self.finish(now);
    }

    fn finish(&mut self, now: f64) {
        if let Some((since, mut t)) = self.current.take() {
            let dwell = (now - since) * 1000.0;
            if dwell.is_finite() {
                t.dwell_ms = dwell.max(t.dwell_ms);
            }
            if self.done.len() >= KEEP {
                self.done.pop_front();
            }
            self.done.push_back(t);
        }
    }

    /// The finished steps, oldest first, and the photo on screen now.
    pub fn to_json(&self) -> Value {
        json!({
            "steps": self.done.iter().map(|t| t.to_json()).collect::<Vec<_>>(),
            "current": self.current.as_ref().map(|(_, t)| t.to_json()),
        })
    }

    #[cfg(test)]
    fn steps(&self) -> Vec<StepTiming> {
        self.done.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_records_when_each_kind_of_picture_first_showed() {
        let mut l = LoupeTiming::default();
        l.observe(PhotoId(1), "none", 10.0);
        l.observe(PhotoId(1), "thumb", 10.016);
        l.observe(PhotoId(1), "embedded", 10.040);
        l.observe(PhotoId(1), "embedded", 10.056);
        l.observe(PhotoId(1), "render", 11.0);
        l.observe(PhotoId(2), "thumb", 12.0);
        let s = l.steps();
        assert_eq!(s.len(), 1);
        let t = s[0];
        assert_eq!(t.photo, PhotoId(1));
        assert_eq!((t.any_source, t.full_source), ("thumb", "embedded"));
        assert!((t.any_ms.unwrap() - 16.0).abs() < 0.01);
        assert!((t.full_ms.unwrap() - 40.0).abs() < 0.01);
        assert!((t.sharp_ms.unwrap() - 1000.0).abs() < 0.01);
        assert!((t.dwell_ms - 2000.0).abs() < 0.01);
        assert_eq!(l.to_json()["current"]["photo"], 2);
    }

    #[test]
    fn a_photo_stepped_past_has_no_sharp_time_and_the_list_is_bounded() {
        let mut l = LoupeTiming::default();
        for i in 0..(KEEP as u64 + 10) {
            l.observe(PhotoId(i), "thumb", i as f64 * 0.033);
        }
        l.left(1000.0);
        let s = l.steps();
        assert_eq!(s.len(), KEEP);
        assert!(s.iter().all(|t| t.sharp_ms.is_none() && t.full_ms.is_none()));
        // a non-finite clock never poisons the record
        l.observe(PhotoId(9999), "render", f64::NAN);
        l.observe(PhotoId(9999), "render", f64::INFINITY);
        assert!(l.to_json().is_object());
    }
}
