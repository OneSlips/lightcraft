//! Sharp zoomed views (issue #323). The loupe renders the whole frame in one texture, which can't
//! be as large as a 1:1 or 8:1 view of a big photo. Past that size it also renders just the window
//! of the frame that is on screen, at the zoom scale, and draws it over the whole-frame render.
//!
//! This module is the geometry: when a window is needed and which one. Windows are snapped to a
//! grid and carry a margin, so panning by a few pixels re-uses the render and the spatial stages
//! (clarity, dehaze…) have the context they read around the visible edge.

use lightcraft_engine::pipeline::PixelWindow;

/// Windows start and end on multiples of this many pixels of the zoomed frame.
pub const SNAP: usize = 256;
/// The most context rendered beyond the visible pixels (a multiple of [`SNAP`]).
/// 4.5 % of an 11:1 view of a 24 MP frame, snapped to [`SNAP`]. The old 768 px cap left Dehaze
/// tens of codes off the full-frame render from 4:1 up (#1146).
pub const MAX_MARGIN: usize = 3072;
/// The most a window may span on either axis: a bound on its texture whatever the window size.
pub const MAX_SPAN: usize = if cfg!(target_arch = "wasm32") { 3072 } else { 6144 };

/// A window render the loupe asked for: which photo, the size of the zoomed frame it is a window
/// of, and the window. The texture that comes back is drawn at this place of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionView {
    pub photo: lightcraft_catalog::PhotoId,
    /// The Before side of a Before/After view (the photo without its edits).
    pub before: bool,
    pub key: u64,
    pub full: (usize, usize),
    pub window: PixelWindow,
    /// Hash of the render state it was rendered with (develop settings plus any window-safe overlay).
    pub settings: u64,
}

impl RegionView {
    /// Whether this window's pixels still belong in a loupe showing `photo` with `settings` in a
    /// frame of `full` pixels (they are drawn until the next window arrives, if so).
    ///
    /// While a slider is dragged (`drafting`) a window made for an earlier value of it still
    /// belongs: drafts follow the drag, and waiting for an exact match would show none.
    pub fn is_current(&self, photo: lightcraft_catalog::PhotoId, full: (usize, usize), settings: u64, drafting: bool) -> bool {
        self.photo == photo && self.full == full && (drafting || self.settings == settings)
    }
}

/// Context to render around the visible pixels of a frame `full_long` pixels along: enough for the
/// wide stages (base, clarity and dehaze planes have sigmas up to 0.02 of it) up to a cap.
pub fn margin_for(full_long: usize) -> usize {
    let want = (full_long as f64 * 0.045).min(MAX_MARGIN as f64) as usize;
    want.div_ceil(SNAP).max(1) * SNAP
}

/// The widest a window may be on a host whose GPU textures are at most `texture_side` px: a
/// window is one texture, and a texture over the limit makes some backends (egui_glow, i.e. the
/// browser build) panic. A multiple of [`SNAP`], at least one grid step, at most [`MAX_SPAN`].
pub fn max_span(texture_side: usize) -> usize {
    (texture_side.min(MAX_SPAN) / SNAP * SNAP).max(SNAP)
}

/// A window render is worth it only when the whole-frame render is this much smaller than the
/// zoomed frame: a little softness (a Retina fit view of 2800 px from the 2560 px preview) is not
/// worth decoding the original for.
pub const WINDOW_RATIO: f32 = 1.25;

/// How big the loupe's view of the photo is, and what the host allows.
#[derive(Clone, Copy, Debug)]
pub struct ViewSizes {
    /// Long edge of the photo as drawn (physical px): the zoomed frame.
    pub drawn_long: f32,
    /// Long edge of the canvas it is drawn in (physical px).
    pub canvas_long: f32,
    /// The photo's own long edge as shown (after crop and rotation).
    pub native_long: usize,
    /// The largest texture the GPU takes.
    pub texture_side: usize,
    /// 1.0, or less while a slider is dragged (a draft of the whole-frame render; see
    /// [`DraftSize`]).
    pub draft_scale: f32,
    /// A slider is being dragged: the whole-frame draft is never larger than the canvas.
    pub drafting: bool,
    /// Whether a window can be drawn over the whole-frame render (not with a diagnostic overlay
    /// that needs whole-frame context, soft proofing, or a read too far for one window). Mask
    /// overlays are window-safe.
    pub windows: bool,
}

/// What the loupe renders for a view: the whole frame at about canvas size, and, once the view is
/// zoomed past what that holds, the window on screen at no more than 100 %.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoupePlan {
    /// Long edge of the whole-frame render ([`Slot::Main`](crate::render::Slot::Main)): never more
    /// than the canvas shows, the user's preview limit, the texture side or the preview source level.
    pub main_edge: usize,
    /// Long edge of the zoomed frame the window is cut from (never above the photo's own pixels:
    /// beyond 100 % the GPU magnifies the window), or `None`: the whole-frame render is enough.
    pub window_edge: Option<usize>,
}

/// The loupe's render plan. The cost of a render follows the canvas, not the zoom: the whole-frame
/// render is canvas-sized, and the window holds only the pixels on screen at no more than 100 %.
pub fn plan(settings: &crate::state::AppSettings, v: ViewSizes) -> LoupePlan {
    let canvas_long = if v.canvas_long.is_finite() { v.canvas_long.max(8.0) } else { 8.0 };
    let drawn_long = if v.drawn_long.is_finite() { v.drawn_long.max(8.0) } else { canvas_long };
    let scale = if v.draft_scale.is_finite() { v.draft_scale.clamp(0.1, 1.0) } else { 1.0 };
    if !v.windows {
        // the whole frame at the size it is drawn (soft proof or a whole-frame diagnostic has no
        // window); while a slider drags, a canvas-sized draft as with windows: a drag at 1:1 must
        // not render the photo's every pixel each frame
        let long = if v.drafting || scale < 1.0 { drawn_long.min(canvas_long) } else { drawn_long };
        let main_edge = settings.loupe_edge(long * scale, v.native_long, v.texture_side);
        return LoupePlan { main_edge, window_edge: None };
    }
    // a user who chose a size above the preview source level (3840, 5120) means it; otherwise the
    // original is not decoded for the whole-frame render
    let source_cap = if settings.preview_limit == 0 { lightcraft_engine::SourceLevel::Preview.max_edge() } else { settings.preview_limit as usize };
    let edge_at = |scale: f32| settings.loupe_edge(drawn_long.min(canvas_long) * scale, v.native_long, v.texture_side).min(source_cap);
    let main_edge = edge_at(scale);
    let frame_edge = settings.window_frame_edge(drawn_long, v.native_long);
    // a user who capped the preview size has chosen speed over sharpness at fit
    let fit = drawn_long <= canvas_long * 1.05;
    // (against the undrafted whole-frame render: a drag must not switch the window off and on)
    let window_edge = (frame_edge as f32 > edge_at(1.0) as f32 * WINDOW_RATIO && !(fit && settings.preview_limit != 0)).then_some(frame_edge);
    LoupePlan { main_edge, window_edge }
}

/// What a slider drag's whole-frame drafts may cost (ms): one frame at 60 Hz (the budget in
/// `CLAUDE.md`, "slider update ≤ 16 ms draft").
pub const DRAFT_BUDGET_MS: f64 = 16.0;

/// The draft scales a drag may use, sharpest first. A drag keeps the one it started with (each
/// size is a new geometry: every plane of the stage cache would be redone), stepping down only
/// when its renders keep running over budget.
pub const DRAFT_SCALES: [f32; 5] = [1.0, 0.8, 0.6, 0.45, 0.35];

/// The scale of the first drafts on the CPU, before anything has been measured (on the GPU a
/// full-size draft takes a few milliseconds: it starts at 1.0).
pub const FIRST_DRAFT_SCALE_CPU: f32 = 0.6;

/// How long the photo must stay unchanged in an open interaction before the view renders at full
/// quality (s), at least: a slider held still, or a control surface whose gesture stays open for a
/// second and more after the last turn of a knob, shows the sharp render instead of the last
/// draft. The time the last full render took is added (up to [`REFINE_MAX_SECS`]): a slow CPU
/// render started at every pause of the pointer would hold up the drafts behind it.
pub const REFINE_AFTER_SECS: f64 = 0.15;
/// The most the refine delay grows to.
pub const REFINE_MAX_SECS: f64 = 1.0;

/// Draft renders over twice the budget in a row that make a drag step down to the next scale.
const OVER_BUDGET_STEPS: u32 = 3;

/// The part of a render's time that does not grow with its size (ms): dispatch, readback,
/// texture upload. Left out of the cost per megapixel, or small drafts (a zoomed view's
/// whole-frame render) would make a fit view's drafts look dearer than they are.
const DRAFT_OVERHEAD_MS: f64 = 2.0;

/// How sharp a slider drag's drafts are, and when they end.
///
/// Drafts used to be a fixed 60 % of the view, and the view stayed a draft until the interaction
/// ended: at Fit a drag was visibly soft, and with a control surface (whose gesture ends about
/// 1.5 s after the last turn) it stayed soft for seconds after the photo had stopped changing.
/// Now the scale is the sharpest that the measured draft cost allows within [`DRAFT_BUDGET_MS`]
/// (a full-size draft on the GPU), and the view refines to full quality once the photo has been
/// still for the refine delay ([`REFINE_AFTER_SECS`]), interaction open or not.
#[derive(Clone, Debug, Default)]
pub struct DraftSize {
    /// The photo and look seen last, and since when (s; `-inf`: not changed in an interaction).
    seen: Option<(lightcraft_catalog::PhotoId, u64, f64)>,
    /// Measured cost of drafts: ms per megapixel (a running average), and the photo and
    /// renderer (GPU?) it was measured for: another photo (other masks) or renderer starts over.
    ms_per_mp: Option<f64>,
    cost_of: Option<(lightcraft_catalog::PhotoId, bool)>,
    /// The scale of the drag in progress.
    held: Option<f32>,
    /// Draft renders over budget in a row (in this drag).
    over: u32,
    /// When the newest draft taken into account reached the screen ([`crate::render::LoupeRender::at_ms`]).
    last_draft_at: f64,
}

impl DraftSize {
    /// Whether the view renders drafts this frame: an interaction is `open` and the photo's `look`
    /// changed within the refine delay (`full_ms`: the last full render's time). `now` is in s.
    pub fn drafting(&mut self, open: bool, photo: lightcraft_catalog::PhotoId, look: u64, now: f64, full_ms: f64) -> bool {
        let changed = !matches!(self.seen, Some((p, l, _)) if p == photo && l == look);
        if changed {
            // (a look first seen outside an interaction is not an edit in progress)
            self.seen = Some((photo, look, if open { now } else { f64::NEG_INFINITY }));
        }
        if !open {
            self.held = None;
            self.over = 0;
            return false;
        }
        let since = self.seen.map_or(f64::NEG_INFINITY, |(_, _, t)| t);
        now - since < Self::refine_after(full_ms)
    }

    /// How long the photo must be still before a full render, when the last one took `full_ms`.
    pub fn refine_after(full_ms: f64) -> f64 {
        let full = if full_ms.is_finite() { full_ms.max(0.0) / 1e3 } else { 0.0 };
        (REFINE_AFTER_SECS + full).min(REFINE_MAX_SECS)
    }

    /// In how many seconds from `now` the view refines (a repaint is due then); `None`: no
    /// refine is waiting.
    pub fn refine_in(&self, now: f64, full_ms: f64) -> Option<f64> {
        let (_, _, since) = self.seen?;
        let left = since + Self::refine_after(full_ms) - now;
        (left.is_finite() && left > 0.0).then_some(left)
    }

    /// The scale of this drag's drafts of `photo`, for a view whose full render has `full_px`
    /// pixels (`gpu`: views render on the GPU). Chosen once per drag.
    pub fn scale(&mut self, photo: lightcraft_catalog::PhotoId, full_px: usize, gpu: bool) -> f32 {
        if let Some(s) = self.held {
            return s;
        }
        if self.cost_of.is_some_and(|c| c != (photo, gpu)) {
            self.ms_per_mp = None;
        }
        self.cost_of = Some((photo, gpu));
        let s = match self.ms_per_mp {
            None if gpu => 1.0,
            None => FIRST_DRAFT_SCALE_CPU,
            Some(c) => {
                let mp = full_px as f64 / 1e6;
                let fits = |s: &&f32| DRAFT_OVERHEAD_MS + c * mp * f64::from(**s) * f64::from(**s) <= DRAFT_BUDGET_MS;
                DRAFT_SCALES.iter().find(fits).or(DRAFT_SCALES.last()).copied().unwrap_or(FIRST_DRAFT_SCALE_CPU)
            }
        };
        self.held = Some(s);
        s
    }

    /// Learn from a whole-frame draft that reached the screen at `at_ms`: `pixels` rendered in
    /// `render_ms`. Each draft counts once.
    pub fn observe(&mut self, at_ms: f64, pixels: usize, render_ms: f64) {
        if at_ms <= self.last_draft_at || pixels == 0 || !render_ms.is_finite() || render_ms < 0.0 {
            return;
        }
        self.last_draft_at = at_ms;
        let cost = (render_ms - DRAFT_OVERHEAD_MS).max(0.0) / (pixels as f64 / 1e6);
        // (one slow render, a plane redone for another slider, moves the average only so far)
        self.ms_per_mp = Some(match self.ms_per_mp {
            Some(c) => 0.7 * c + 0.3 * cost.min(3.0 * c.max(0.1)),
            None => cost,
        });
        self.over = if render_ms > 2.0 * DRAFT_BUDGET_MS { self.over + 1 } else { 0 };
        if self.over >= OVER_BUDGET_STEPS
            && let Some(held) = self.held
        {
            self.over = 0;
            self.held = Some(DRAFT_SCALES.iter().copied().find(|s| *s < held).unwrap_or(held));
        }
    }

    /// The measured draft cost (ms per megapixel), for `ui.inspect`.
    pub fn ms_per_mp(&self) -> Option<f64> {
        self.ms_per_mp
    }

    /// The scale of the drag in progress, for `ui.inspect`.
    pub fn held(&self) -> Option<f32> {
        self.held
    }
}

/// How long the render sizes stay as they were after a pinch or two-finger scroll stops (s).
pub const HOLD_SECS: f64 = 0.25;

/// Holds the loupe's render sizes while a gesture runs (a continuous pinch changes the zoom every
/// frame; re-planning each time would be a stream of renders of every size in between). The view
/// keeps being drawn from the renders it has, magnified by the GPU, and is re-planned once the
/// gesture has been quiet for [`HOLD_SECS`].
#[derive(Clone, Debug, Default)]
pub struct SizeHold {
    photo: Option<lightcraft_catalog::PhotoId>,
    plan: Option<LoupePlan>,
    until: f64,
}

impl SizeHold {
    /// The plan to render this frame: `fresh` normally; the plan from before the gesture while
    /// one is running (`gesturing`) and for [`HOLD_SECS`] after. `now` is in seconds.
    pub fn apply(&mut self, photo: lightcraft_catalog::PhotoId, now: f64, gesturing: bool, fresh: LoupePlan) -> LoupePlan {
        if self.photo != Some(photo) {
            *self = SizeHold { photo: Some(photo), plan: Some(fresh), until: f64::NEG_INFINITY };
            return fresh;
        }
        if gesturing {
            self.until = now + HOLD_SECS;
        }
        match self.plan {
            Some(held) if gesturing || now < self.until => held,
            _ => {
                self.plan = Some(fresh);
                fresh
            }
        }
    }

    /// Whether sizes are being held at `now` (the caller asks for a repaint when it ends).
    pub fn holding(&self, now: f64) -> bool {
        now < self.until
    }
}

/// Where the line of a wipe crosses: the middle of the part of the photo that is on screen. At
/// Fit that is the photo's middle; zoomed in, the photo's own middle is mostly off screen, and a
/// line there left the whole view to one side (#1351).
pub fn wipe_line(img: egui::Rect, canvas: egui::Rect) -> egui::Pos2 {
    let shown = img.intersect(canvas);
    if shown.is_positive() { shown.center() } else { img.center() }
}

/// Where a window of one side of a Before/After view may be drawn: its own pane in side by side
/// and stacked views (a window carries margin beyond what its pane shows, which would otherwise
/// spill into the neighbour), its side of the line in a wipe, else the canvas.
pub fn window_clip(mode: crate::state::BeforeAfter, before_side: bool, pane: egui::Rect, canvas: egui::Rect, img: egui::Rect) -> egui::Rect {
    use crate::state::BeforeAfter;
    let (min, max) = (canvas.min, canvas.max);
    let line = wipe_line(img, canvas);
    let clip = match (mode, before_side) {
        (BeforeAfter::Split, true) => egui::Rect::from_min_max(min, egui::pos2(line.x, max.y)),
        (BeforeAfter::SplitTopBottom, true) => egui::Rect::from_min_max(min, egui::pos2(max.x, line.y)),
        (BeforeAfter::Split, false) => egui::Rect::from_min_max(egui::pos2(line.x, min.y), max),
        (BeforeAfter::SplitTopBottom, false) => egui::Rect::from_min_max(egui::pos2(min.x, line.y), max),
        (BeforeAfter::SideBySide | BeforeAfter::TopBottom, _) => pane,
        _ => canvas,
    };
    clip.intersect(canvas)
}

/// Whether a window waits for the one already being rendered. All windows are cut from the same
/// decoded original; asked for while it isn't held yet (the Before and After together, in either
/// order, or a new window while the first is still decoding), each worker decodes its own copy
/// (two or three full-size decodes and uploads, all but one thrown away). The waiting window
/// follows once the original is in.
pub fn defer_window(window_pending: bool, original_held: bool) -> bool {
    window_pending && !original_held
}

/// Whether the picture on screen has the frame's aspect (to the one pixel a texture's whole-pixel
/// size is off by): a window is a part of the frame, so it can only be placed over a picture that
/// is the frame (an unsupported raw's embedded JPEG may be cropped differently).
pub fn same_aspect(shown: f32, frame: f32) -> bool {
    shown.is_finite() && frame.is_finite() && frame > 0.0 && (shown / frame - 1.0).abs() <= 0.02
}

/// The window of a `full_w × full_h` frame to render so that `visible` — `(x0, y0, x1, y1)` in the
/// frame's pixels, as much of the frame as is on screen — is covered: expanded by a margin,
/// snapped outwards to the [`SNAP`] grid (margin: see [`margin_for`]) and clamped into the frame,
/// at most `max_span` ([`max_span`]) wide or high. `None`: nothing is visible.
pub fn window_for(full_w: usize, full_h: usize, visible: (f32, f32, f32, f32), max_span: usize) -> Option<PixelWindow> {
    if full_w == 0 || full_h == 0 || [visible.0, visible.1, visible.2, visible.3].iter().any(|v| !v.is_finite()) {
        return None;
    }
    let margin = margin_for(full_w.max(full_h));
    let axis = |lo: f32, hi: f32, full: usize| -> Option<(usize, usize)> {
        let (lo, hi) = (lo.max(0.0), hi.min(full as f32));
        if hi <= lo {
            return None;
        }
        let a = (lo as usize).saturating_sub(margin) / SNAP * SNAP;
        let b = ((hi.ceil() as usize).saturating_add(margin)).div_ceil(SNAP).saturating_mul(SNAP).min(full);
        // a window larger than MAX_SPAN keeps the visible part (centred on it), snapped
        if b - a > max_span {
            let mid = ((lo + hi) / 2.0) as usize;
            let a = mid.saturating_sub(max_span / 2) / SNAP * SNAP;
            return Some((a, (a + max_span).min(full)));
        }
        Some((a, b))
    };
    let (x0, x1) = axis(visible.0, visible.2, full_w)?;
    let (y0, y1) = axis(visible.1, visible.3, full_h)?;
    Some(PixelWindow { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppSettings;

    const BIG: usize = 16384;

    fn sizes(drawn: f32, canvas: f32, native: usize) -> ViewSizes {
        ViewSizes { drawn_long: drawn, canvas_long: canvas, native_long: native, texture_side: BIG, draft_scale: 1.0, drafting: false, windows: true }
    }

    fn auto() -> AppSettings {
        AppSettings::default()
    }

    // Given a fit view on a Retina Mac (about 2800 px), the preview is enough: no original decoded
    #[test]
    fn a_retina_fit_view_needs_no_window() {
        let p = plan(&auto(), sizes(2800.0, 2800.0, 6000));
        assert_eq!(p.main_edge, 2560, "the preview source level");
        assert_eq!(p.window_edge, None);
    }

    // Given a 4K display, the fit view is 1.5x the preview: the window makes it sharp
    #[test]
    fn a_large_fit_view_gets_a_window() {
        let p = plan(&auto(), sizes(3840.0, 3840.0, 6000));
        assert_eq!((p.main_edge, p.window_edge), (2560, Some(3840)));
    }

    // Given 1:1 on a 24 MP photo in a 2800 px canvas, the whole frame stays canvas-sized and the
    // window is cut from the photo's own pixels
    #[test]
    fn one_to_one_renders_the_canvas_and_a_native_window() {
        let p = plan(&auto(), sizes(6000.0, 2800.0, 6000));
        assert_eq!((p.main_edge, p.window_edge), (2560, Some(6000)));
    }

    // Given 800 %, the window is still at 100 % (the GPU magnifies it): the cost does not grow
    #[test]
    fn beyond_one_to_one_the_window_stays_at_native_resolution() {
        let p = plan(&auto(), sizes(48000.0, 2800.0, 6000));
        assert_eq!((p.main_edge, p.window_edge), (2560, Some(6000)));
        let q = plan(&auto(), sizes(480.0 * 100.0, 2800.0, 6000));
        assert_eq!(q.window_edge, Some(6000));
    }

    // Given a small photo zoomed in, the whole-frame render already holds all its pixels
    #[test]
    fn a_small_photo_needs_no_window() {
        let p = plan(&auto(), sizes(8000.0, 1400.0, 1000));
        assert_eq!((p.main_edge, p.window_edge), (1000, None));
    }

    // Given a slider drag at 400 % (drafts at 0.6 scale), the window plan does not change: it is
    // the whole-frame draft that gets cheaper
    #[test]
    fn a_drag_shrinks_the_whole_frame_draft_not_the_window() {
        let still = plan(&auto(), sizes(24000.0, 2800.0, 6000));
        let drag = plan(&auto(), ViewSizes { draft_scale: 0.6, ..sizes(24000.0, 2800.0, 6000) });
        assert!(drag.main_edge < still.main_edge, "{drag:?} vs {still:?}");
        assert_eq!(drag.window_edge, still.window_edge);
    }

    // Given no window (a whole-frame diagnostic or soft proofing), the view at rest renders the
    // whole frame at the size it is drawn, but a slider drag drafts at the canvas size like the
    // windowed path
    #[test]
    fn without_a_window_a_drag_drafts_at_the_canvas_size() {
        let no_window = |draft_scale: f32| ViewSizes { windows: false, draft_scale, drafting: draft_scale < 1.0, ..sizes(24000.0, 2800.0, 6000) };
        let still = plan(&auto(), no_window(1.0));
        let drag = plan(&auto(), no_window(0.6));
        assert_eq!(still.window_edge, None);
        assert!(still.main_edge > 2800, "at rest the zoomed frame: {still:?}");
        assert!(drag.main_edge <= 2800, "a drag drafts at most the canvas: {drag:?}");
    }

    // Given the user capped the preview size, the whole-frame render obeys it, and a fit view
    // stays on it (no original decoded); a zoomed view still gets its sharp window
    #[test]
    fn a_preview_limit_caps_the_whole_frame_render_and_spares_the_fit_view() {
        let s = AppSettings { preview_limit: 1600, ..Default::default() };
        let fit = plan(&s, sizes(2800.0, 2800.0, 6000));
        assert_eq!((fit.main_edge, fit.window_edge), (1600, None));
        let zoomed = plan(&s, sizes(6000.0, 2800.0, 6000));
        assert_eq!((zoomed.main_edge, zoomed.window_edge), (1600, Some(6000)));
    }

    // Given a GPU with 2048 px textures, the whole-frame render obeys it, the window frame need not
    // (a window is cut to the texture side separately)
    #[test]
    fn the_texture_side_caps_the_whole_frame_render_only() {
        let p = plan(&auto(), ViewSizes { texture_side: 2048, ..sizes(6000.0, 2800.0, 6000) });
        assert_eq!((p.main_edge, p.window_edge), (2048, Some(6000)));
    }

    // Given a picture that is not the frame (an embedded JPEG cropped differently), no window
    #[test]
    fn a_window_needs_a_picture_with_the_frames_aspect() {
        assert!(same_aspect(1.5, 1.5) && same_aspect(1.51, 1.5));
        assert!(!same_aspect(1.5, 1.0) && !same_aspect(f32::NAN, 1.0) && !same_aspect(1.0, 0.0));
    }

    // Given a drag at fit on a 4K or 5K display (where the fit view has a window), the window stays:
    // it is only the whole-frame draft that gets smaller
    #[test]
    fn a_drag_at_fit_on_a_large_display_keeps_the_window() {
        for drawn in [3300.0, 3840.0, 4096.0, 5120.0] {
            let still = plan(&auto(), sizes(drawn, drawn, 6000));
            let drag = plan(&auto(), ViewSizes { draft_scale: 0.6, ..sizes(drawn, drawn, 6000) });
            assert!(still.window_edge.is_some(), "{drawn}: {still:?}");
            assert_eq!(drag.window_edge, still.window_edge, "{drawn}");
        }
    }

    // Given a mode whose picture can't be a window (soft proofing, a whole-frame diagnostic, or a
    // spot read too far for one render), the whole-frame render is as big as the view
    #[test]
    fn without_windows_the_whole_frame_is_rendered_at_the_drawn_size() {
        let p = plan(&auto(), ViewSizes { windows: false, ..sizes(6000.0, 2800.0, 6000) });
        assert_eq!((p.main_edge, p.window_edge), (6000, None));
        let fit = plan(&auto(), ViewSizes { windows: false, ..sizes(2800.0, 2800.0, 6000) });
        assert_eq!((fit.main_edge, fit.window_edge), (2816, None));
        let deep = plan(&auto(), ViewSizes { windows: false, ..sizes(48000.0, 2800.0, 6000) });
        assert_eq!(deep.main_edge, 6000, "never above the photo's own pixels");
        let small = plan(&auto(), ViewSizes { windows: false, texture_side: 2048, ..sizes(6000.0, 2800.0, 6000) });
        assert_eq!(small.main_edge, 2048);
    }

    // Given a user who chose a preview size above the preview source level (3840 or 5120 on a big
    // display), the whole-frame render is that big
    #[test]
    fn a_large_preview_limit_is_honoured() {
        for limit in [3840u32, 5120] {
            let s = AppSettings { preview_limit: limit, ..Default::default() };
            let p = plan(&s, sizes(5120.0, 5120.0, 6000));
            assert_eq!(p.main_edge, limit as usize, "{limit}");
            assert_eq!(p.window_edge, None, "a fit view at the limit needs no window");
        }
        let s = AppSettings { preview_limit: 2560, ..Default::default() };
        assert_eq!(plan(&s, sizes(5120.0, 5120.0, 6000)).main_edge, 2560);
    }

    // Given a medium format photo wider than 8192 px, 1:1 is its own pixels
    #[test]
    fn a_window_frame_can_be_as_big_as_the_photo() {
        let p = plan(&auto(), sizes(40000.0, 2800.0, 11648));
        assert_eq!(p.window_edge, Some(11648));
    }

    // Given a drag at fit on a Retina Mac, the window does not switch on (it would flicker with the drag)
    #[test]
    fn a_drag_at_fit_does_not_switch_the_window_on() {
        let p = plan(&auto(), ViewSizes { draft_scale: 0.6, ..sizes(2800.0, 2800.0, 6000) });
        assert_eq!(p.window_edge, None);
    }

    // Given a pinch from fit to 400 %, the sizes of the first frame hold until the gesture is quiet
    #[test]
    fn a_pinch_holds_the_render_sizes() {
        use lightcraft_catalog::PhotoId;
        let (fit, zoomed) = (plan(&auto(), sizes(1400.0, 1400.0, 6000)), plan(&auto(), sizes(24000.0, 1400.0, 6000)));
        assert_ne!(fit, zoomed);
        let mut hold = SizeHold::default();
        let p = PhotoId(1);
        assert_eq!(hold.apply(p, 0.0, false, fit), fit);
        // the pinch runs for a second: every frame would plan something else
        for k in 1..60 {
            assert_eq!(hold.apply(p, k as f64 / 60.0, true, zoomed), fit, "frame {k}");
        }
        // it stopped at t = 59/60: still held for a moment (no flicker between two events)…
        assert!(hold.holding(1.1));
        assert_eq!(hold.apply(p, 1.1, false, zoomed), fit);
        // …then the view is planned for where it ended
        assert!(!hold.holding(59.0 / 60.0 + HOLD_SECS + 0.01));
        assert_eq!(hold.apply(p, 1.3, false, zoomed), zoomed);
        assert_eq!(hold.apply(p, 1.4, false, zoomed), zoomed);
    }

    // Given another photo, the held sizes are forgotten
    #[test]
    fn another_photo_is_planned_afresh() {
        use lightcraft_catalog::PhotoId;
        let (fit, zoomed) = (plan(&auto(), sizes(1400.0, 1400.0, 6000)), plan(&auto(), sizes(24000.0, 1400.0, 6000)));
        let mut hold = SizeHold::default();
        hold.apply(PhotoId(1), 0.0, false, fit);
        assert_eq!(hold.apply(PhotoId(2), 0.1, true, zoomed), zoomed);
    }

    // Given side by side, a window is drawn in its pane only; given a wipe, on its side of the line
    #[test]
    fn a_window_is_clipped_to_its_pane_or_its_side_of_the_wipe() {
        use crate::state::BeforeAfter::*;
        let canvas = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 600.0));
        let (left, right) = (
            egui::Rect::from_min_max(egui::pos2(24.0, 24.0), egui::pos2(494.0, 576.0)),
            egui::Rect::from_min_max(egui::pos2(506.0, 24.0), egui::pos2(976.0, 576.0)),
        );
        assert_eq!(window_clip(SideBySide, true, left, canvas, left), left);
        assert_eq!(window_clip(SideBySide, false, right, canvas, right), right);
        let img = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(900.0, 550.0));
        let before = window_clip(Split, true, canvas, canvas, img);
        let after = window_clip(Split, false, canvas, canvas, img);
        assert_eq!((before.max.x, after.min.x), (500.0, 500.0), "the line is the image's middle");
        assert_eq!(window_clip(SplitTopBottom, true, canvas, canvas, img).max.y, 300.0);
        assert_eq!(window_clip(SplitTopBottom, false, canvas, canvas, img).min.y, 300.0);
        // zoomed in, the photo's middle (x = 1600) is off screen: the line is the view's middle
        let zoomed = egui::Rect::from_min_max(egui::pos2(-200.0, -900.0), egui::pos2(3400.0, 1500.0));
        assert_eq!(window_clip(Split, false, canvas, canvas, zoomed).min.x, 500.0);
        assert_eq!(window_clip(SplitTopBottom, true, canvas, canvas, zoomed).max.y, 300.0);
        assert_eq!(window_clip(Off, false, canvas, canvas, img), canvas);
        assert_eq!(window_clip(Original, true, canvas, canvas, img), canvas);
    }

    // Given a window asked for while another is rendering and the original isn't decoded yet, it
    // waits for the decoded original instead of decoding another copy
    #[test]
    fn a_window_waits_for_the_original_another_is_decoding() {
        assert!(defer_window(true, false));
        assert!(!defer_window(true, true), "the original is in");
        assert!(!defer_window(false, false), "nothing is decoding it");
    }

    // Hostile numbers give a plan, never a panic
    #[test]
    fn hostile_sizes_give_a_usable_plan() {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -3.0, 0.0] {
            let p = plan(&auto(), ViewSizes { drawn_long: v, canvas_long: v, draft_scale: v, ..sizes(1.0, 1.0, 6000) });
            assert!((8..=2560).contains(&p.main_edge), "{v}: {p:?}");
        }
        let p = plan(&auto(), sizes(1000.0, 1000.0, 0));
        assert!(p.main_edge >= 8);
    }

    // Given a visible area in the middle of a big frame, the window covers it with a margin, on the grid
    #[test]
    fn the_window_covers_the_visible_area_with_a_margin_on_the_grid() {
        let w = window_for(48_000, 32_000, (20_000.0, 10_000.0, 21_400.0, 10_900.0), MAX_SPAN).unwrap();
        let m = margin_for(48_000);
        assert!(w.x + m <= 20_000 && w.x + w.w >= 21_400 + m, "{w:?}");
        assert!(w.y + m <= 10_000 && w.y + w.h >= 10_900 + m, "{w:?}");
        assert_eq!((w.x % SNAP, w.y % SNAP, w.w % SNAP, w.h % SNAP), (0, 0, 0, 0));
    }

    // Given a small pan, the same window is kept (no re-render); a big pan changes it
    #[test]
    fn small_pans_keep_the_window() {
        let a = window_for(48_000, 32_000, (20_000.0, 10_000.0, 21_400.0, 10_900.0), MAX_SPAN).unwrap();
        let b = window_for(48_000, 32_000, (20_040.0, 10_030.0, 21_440.0, 10_930.0), MAX_SPAN).unwrap();
        assert_eq!(a, b);
        let c = window_for(48_000, 32_000, (26_000.0, 10_000.0, 27_400.0, 10_900.0), MAX_SPAN).unwrap();
        assert_ne!(a, c);
    }

    // Given a frame smaller than the margin, or a view at its edge, the window is clamped into it
    #[test]
    fn the_window_stays_inside_the_frame() {
        let w = window_for(3000, 2000, (-500.0, -500.0, 900.0, 700.0), MAX_SPAN).unwrap();
        assert_eq!((w.x, w.y), (0, 0));
        let w = window_for(3000, 2000, (2500.0, 1500.0, 9000.0, 9000.0), MAX_SPAN).unwrap();
        assert!(w.x + w.w <= 3000 && w.y + w.h <= 2000, "{w:?}");
        assert_eq!((w.x + w.w, w.y + w.h), (3000, 2000));
    }

    // Given a deeper zoom, the margin grows with the wide stages' kernels (3σ of the base, clarity
    // and dehaze planes are 0.045 of the frame), up to a cap that bounds the window
    #[test]
    fn the_margin_grows_with_the_zoom_up_to_a_cap() {
        assert_eq!(margin_for(1000), SNAP, "never less than one grid step");
        assert!(margin_for(8000) > margin_for(3000));
        assert_eq!(margin_for(48_000), 2304, "4.5 % of an 8:1 frame, under the cap");
        assert_eq!(margin_for(80_000), MAX_MARGIN);
        assert_eq!(margin_for(0), SNAP);
        assert!(margin_for(usize::MAX) <= MAX_MARGIN);
        for l in [100, 3000, 8000, 48_000, 400_000] {
            assert_eq!(margin_for(l) % SNAP, 0);
        }
    }

    // Given a tile rendered for other settings, another photo or another zoom, it is not drawn
    #[test]
    fn a_tile_is_drawn_only_while_it_shows_what_the_loupe_shows() {
        use lightcraft_catalog::PhotoId;
        let v = RegionView {
            photo: PhotoId(1),
            before: false,
            key: 7,
            full: (6000, 4000),
            window: PixelWindow { x: 0, y: 0, w: 256, h: 256 },
            settings: 11,
        };
        assert!(v.is_current(PhotoId(1), (6000, 4000), 11, false));
        assert!(!v.is_current(PhotoId(2), (6000, 4000), 11, false), "another photo");
        assert!(!v.is_current(PhotoId(1), (6000, 4001), 11, false), "another zoom");
        assert!(!v.is_current(PhotoId(1), (6000, 4000), 12, false), "another look (an edit, an undo)");
        assert!(v.is_current(PhotoId(1), (6000, 4000), 12, true), "a drag's drafts follow the value");
        assert!(!v.is_current(PhotoId(2), (6000, 4000), 12, true), "…of this photo only");
    }

    // Given a GPU that allows 2048 px textures, no window is wider than that
    #[test]
    fn a_window_fits_the_texture_side() {
        for tex in [512, 2048, 4096] {
            let span = max_span(tex);
            assert!(span <= tex.max(SNAP) && span.is_multiple_of(SNAP), "{tex}: {span}");
            let w = window_for(48_000, 32_000, (20_000.0, 10_000.0, 21_400.0, 10_900.0), span).unwrap();
            assert!(w.w <= span && w.h <= span, "{tex}: {w:?}");
        }
        assert_eq!(max_span(100_000), MAX_SPAN);
        assert_eq!(max_span(0), SNAP);
    }

    // Given nothing visible, or hostile numbers, there is no window (never a panic)
    #[test]
    fn nothing_visible_or_hostile_input_is_no_window() {
        assert_eq!(window_for(1000, 1000, (2000.0, 0.0, 3000.0, 100.0), MAX_SPAN), None);
        assert_eq!(window_for(1000, 1000, (10.0, 10.0, 10.0, 90.0), MAX_SPAN), None);
        assert_eq!(window_for(0, 1000, (0.0, 0.0, 10.0, 10.0), MAX_SPAN), None);
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(window_for(1000, 1000, (v, 0.0, 10.0, 10.0), MAX_SPAN), None);
        }
        let w = window_for(usize::MAX / 2, usize::MAX / 2, (0.0, 0.0, 1e30, 1e30), MAX_SPAN).unwrap();
        assert!(w.w <= MAX_SPAN && w.h <= MAX_SPAN, "{w:?}");
    }

    // Given a view of an enormous area, the window is bounded
    #[test]
    fn a_window_is_bounded() {
        let w = window_for(100_000, 100_000, (0.0, 0.0, 90_000.0, 90_000.0), MAX_SPAN).unwrap();
        assert!(w.w <= MAX_SPAN && w.h <= MAX_SPAN);
    }

    // Given a slider drag, the view drafts while the photo changes and refines once it has been
    // still for the refine delay, though the interaction stays open (a control surface's gesture
    // ends 1.5 s after the last turn); nothing drafts outside an interaction
    #[test]
    fn drafts_while_the_photo_changes_and_refine_when_it_is_still() {
        use lightcraft_catalog::PhotoId;
        let mut d = DraftSize::default();
        let p = PhotoId(1);
        assert!(!d.drafting(false, p, 1, 0.0, 10.0), "no interaction");
        assert!(!d.drafting(true, p, 1, 0.1, 10.0), "an interaction that has not changed the photo yet");
        assert!(d.drafting(true, p, 2, 0.2, 10.0), "the first change");
        assert!(d.drafting(true, p, 2, 0.3, 10.0), "within the refine delay");
        let left = d.refine_in(0.3, 10.0).expect("a refine is due");
        assert!((left - (0.2 + DraftSize::refine_after(10.0) - 0.3)).abs() < 1e-9, "{left}");
        assert!(!d.drafting(true, p, 2, 0.2 + DraftSize::refine_after(10.0) + 0.01, 10.0), "still: the full render");
        assert!(d.drafting(true, p, 3, 2.0, 10.0), "changing again drafts again");
        assert!(!d.drafting(false, p, 3, 2.01, 10.0), "the interaction ended");
        assert!(!d.drafting(true, PhotoId(2), 9, 3.0, 10.0) || d.refine_in(3.0, 10.0).is_some());
    }

    // Given a slow full render (the CPU), the refine waits longer, but never more than a second
    #[test]
    fn the_refine_delay_follows_the_full_render_time() {
        assert!((DraftSize::refine_after(0.0) - REFINE_AFTER_SECS).abs() < 1e-9);
        assert!((DraftSize::refine_after(200.0) - (REFINE_AFTER_SECS + 0.2)).abs() < 1e-9);
        assert_eq!(DraftSize::refine_after(5000.0), REFINE_MAX_SECS);
        for v in [f64::NAN, f64::INFINITY, -1.0] {
            let r = DraftSize::refine_after(v);
            assert!((REFINE_AFTER_SECS..=REFINE_MAX_SECS).contains(&r), "{v}: {r}");
        }
    }

    // Given nothing measured yet, a GPU drag drafts at full size and a CPU one at the old 60 %;
    // once measured, the sharpest scale within the budget, held for the whole drag
    #[test]
    fn the_draft_scale_is_the_sharpest_the_measured_cost_allows() {
        const P: lightcraft_catalog::PhotoId = lightcraft_catalog::PhotoId(1);
        let px = 1280 * 1920; // 2.46 MP
        assert_eq!(DraftSize::default().scale(P, px, true), 1.0);
        assert_eq!(DraftSize::default().scale(P, px, false), FIRST_DRAFT_SCALE_CPU);
        // a GPU draft: 5 ms for 1.3 MP: a full-size draft fits the budget
        let mut cheap = DraftSize::default();
        cheap.observe(1.0, 768 * 1152, 5.0);
        assert_eq!(cheap.scale(P, px, false), 1.0);
        // a CPU draft with masks: 50 ms for 1.3 MP
        let mut dear = DraftSize::default();
        dear.observe(1.0, 768 * 1152, 50.0);
        let s = dear.scale(P, px, false);
        assert!(s < 0.6, "{s}");
        let per_mp = dear.ms_per_mp().unwrap_or(0.0);
        assert!(DRAFT_OVERHEAD_MS + per_mp * px as f64 / 1e6 * f64::from(s * s) <= DRAFT_BUDGET_MS || s == DRAFT_SCALES[DRAFT_SCALES.len() - 1]);
        // held for the drag, though the cost changes
        dear.observe(2.0, 768 * 1152, 1.0);
        assert_eq!(dear.scale(P, px, false), s);
        // a new drag starts from the new measurements
        dear.drafting(false, lightcraft_catalog::PhotoId(1), 1, 0.0, 0.0);
        assert!(dear.held().is_none());
        // another photo (other masks) starts over
        assert_eq!(dear.scale(lightcraft_catalog::PhotoId(2), px, true), 1.0);
    }

    // Given drafts that keep taking over twice the budget, the drag steps down one scale; one
    // slow draft (a plane redone) does not
    #[test]
    fn a_drag_over_budget_steps_down() {
        const P: lightcraft_catalog::PhotoId = lightcraft_catalog::PhotoId(1);
        let mut d = DraftSize::default();
        assert_eq!(d.scale(P, 1000 * 1000, true), 1.0);
        d.observe(1.0, 1000 * 1000, 80.0);
        d.observe(2.0, 1000 * 1000, 5.0);
        d.observe(3.0, 1000 * 1000, 80.0);
        assert_eq!(d.held(), Some(1.0));
        d.observe(4.0, 1000 * 1000, 80.0);
        d.observe(5.0, 1000 * 1000, 80.0);
        assert_eq!(d.held(), Some(DRAFT_SCALES[1]));
        // the same draft twice, and hostile numbers, change nothing
        let before = d.ms_per_mp();
        d.observe(5.0, 1000 * 1000, 999.0);
        d.observe(6.0, 0, 5.0);
        d.observe(7.0, 1000, f64::NAN);
        d.observe(8.0, 1000, -1.0);
        assert_eq!(d.ms_per_mp(), before);
    }
}
