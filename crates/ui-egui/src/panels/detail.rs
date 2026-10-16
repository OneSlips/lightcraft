//! The Detail (loupe) view: the developed photo, before/after, zoom & pan, the filmstrip, and the
//! on-canvas tools (crop, brush/gradient masks, remove spots, white-balance picker).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, MaskShape};
use lightcraft_geom::{Affine, Point};
use lightcraft_pipeline::geometry::Frame;
use serde_json::json;

use crate::LightcraftApp;
use crate::render::Slot;
use crate::state::{BeforeAfter, RightPanel, ToolOverlay, Zoom};
use crate::theme::Tokens;
use crate::widgets::register;

/// An in-progress on-canvas gesture.
#[derive(Clone, Debug)]
pub enum Gesture {
    Brush {
        points: Vec<Point>,
        /// Pen pressure (0..1) at each point; empty when no pen pressure was reported (a mouse) or the
        /// brush's Pen Pressure switch is off.
        pressure: Vec<f32>,
        /// Auto Mask's live stroke: when (app time) the stroke last went to the photo and how many
        /// points it had then; `None` until it first does (its undo step is open meanwhile).
        live: Option<(f64, usize)>,
    },
    Spot {
        points: Vec<Point>,
    },
    /// Remove tool: dragging spot `spot`'s target (or its source).
    SpotMove {
        spot: usize,
        source: bool,
    },
    CropHandle {
        handle: u8,
        start: lightcraft_geom::Rect,
        angle: f64,
    },
    /// Straighten tool: a line drawn along something that should be level (or plumb).
    StraightenLine {
        a: Pos2,
    },
    CropRotate {
        start_angle: f64,
        a0: f32,
    },
    Pan {
        start: (f32, f32),
        at: Pos2,
    },
    /// Dragging component `comp` of `mask`: its pin (`handle` 0) or a linear gradient's start (1)
    /// or end (2).
    MaskHandle {
        mask: u32,
        comp: usize,
        handle: u8,
        original_shape: MaskShape,
    },
    /// Guided Upright: a guide being drawn from `a` (normalized transformed coordinates).
    Guide {
        a: Point,
    },
    /// Red Eye: an ellipse being dragged from corner `a` (normalized).
    Eye {
        a: Point,
    },
    /// Targeted adjustment drag from `at` (normalized); `acc` = vertical travel not yet applied.
    Targeted {
        at: Point,
        acc: f32,
    },
    /// Colour / Luminance Range: an area being dragged from `a` (normalized) to sample.
    RangeArea {
        a: Point,
    },
    /// Colour Range: pin `index` of component `comp` being dragged to another place.
    ColorPin {
        comp: usize,
        index: usize,
    },
}

/// Screen ↔ normalized-image mapping for the displayed frame.
#[derive(Clone, Copy)]
pub struct CanvasMap {
    rect: Rect,
    to_norm: Affine,
    from_norm: Affine,
    k: f32,
    /// The photo is drawn turned by this much (radians, clockwise on screen) about the centre of
    /// `rect`: the crop tool turns the photo under a level crop frame.
    turn: f32,
}

impl CanvasMap {
    fn new(frame: &Frame, rect: Rect) -> CanvasMap {
        let k = 8.0f32;
        let (w, h) = ((rect.width() * k).round().max(1.0) as usize, (rect.height() * k).round().max(1.0) as usize);
        let to_norm = frame.out_to_norm(w, h);
        CanvasMap { rect, to_norm, from_norm: to_norm.inverse().unwrap_or(Affine::IDENTITY), k, turn: 0.0 }
    }
    /// The same mapping with the photo drawn turned by `turn` radians about its rectangle's centre.
    fn turned(self, turn: f32) -> CanvasMap {
        CanvasMap { turn: if turn.is_finite() { turn } else { 0.0 }, ..self }
    }
    fn spin(&self, p: Pos2, turn: f32) -> Pos2 {
        if turn == 0.0 {
            return p;
        }
        let c = self.rect.center();
        let (s, co) = turn.sin_cos();
        let v = p - c;
        c + vec2(v.x * co - v.y * s, v.x * s + v.y * co)
    }
    pub fn norm(&self, p: Pos2) -> Point {
        let p = self.spin(p, -self.turn);
        self.to_norm.apply(Point::new(((p.x - self.rect.left()) * self.k) as f64, ((p.y - self.rect.top()) * self.k) as f64))
    }
    pub fn screen(&self, n: Point) -> Pos2 {
        let q = self.from_norm.apply(n);
        self.spin(pos2(self.rect.left() + q.x as f32 / self.k, self.rect.top() + q.y as f32 / self.k), self.turn)
    }
}

/// The crop tool turns the photo under a level crop frame: how far (radians, clockwise on
/// screen) to turn the uncropped photo drawn in `img` so the crop box `d` asks for is level.
fn crop_view_turn(d: &DevelopSettings, frame: &Frame, img: Rect) -> f32 {
    let map = CanvasMap::new(frame, img);
    let q = frame_crop_quad(d, frame);
    let (a, b) = (map.screen(q[0]), map.screen(q[1]));
    let along = b - a;
    if along.length() < 1e-3 {
        return 0.0;
    }
    // the top edge, left to right on screen (a horizontally flipped photo shows it right to left)
    let along = if frame.flip_h { -along } else { along };
    -along.y.atan2(along.x)
}

/// `img` shrunk about its centre so the photo drawn in it, turned by `turn`, still fits `area`
/// (the fitted view; a zoomed one keeps its size).
fn fit_turned(img: Rect, area: Rect, turn: f32) -> Rect {
    let (s, c) = turn.sin_cos();
    let (w, h) = (img.width(), img.height());
    let (bw, bh) = (w * c.abs() + h * s.abs(), w * s.abs() + h * c.abs());
    let k = (area.width() / bw.max(1.0)).min(area.height() / bh.max(1.0)).min(1.0);
    if !k.is_finite() || k <= 0.0 {
        return img;
    }
    Rect::from_center_size(img.center(), img.size() * k)
}

/// Draw texture `tex` into `r` turned by `turn` radians about the centre of `r`.
fn image_turned(p: &egui::Painter, tex: egui::TextureId, r: Rect, turn: f32) {
    let (s, co) = turn.sin_cos();
    let c = r.center();
    let at = |q: Pos2| {
        let v = q - c;
        c + vec2(v.x * co - v.y * s, v.x * s + v.y * co)
    };
    let mut mesh = egui::epaint::Mesh::with_texture(tex);
    let corners =
        [(r.left_top(), pos2(0.0, 0.0)), (r.right_top(), pos2(1.0, 0.0)), (r.right_bottom(), pos2(1.0, 1.0)), (r.left_bottom(), pos2(0.0, 1.0))];
    for (q, uv) in corners {
        mesh.vertices.push(egui::epaint::Vertex { pos: at(q), uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    p.add(mesh);
}

pub(crate) fn fit_rect(area: Rect, aspect: f32, zoom: Zoom, img_px: [usize; 2], ppp: f32, pan: (f32, f32)) -> Rect {
    let zoom = match zoom {
        Zoom::Percent(p) if !p.is_finite() || p <= 0.0 => Zoom::Fit,
        _ => zoom,
    };
    let (aw, ah) = (area.width(), area.height());
    let (w, h) = match zoom {
        Zoom::Fit => {
            if aw / ah > aspect {
                (ah * aspect, ah)
            } else {
                (aw, aw / aspect)
            }
        }
        Zoom::Fill => {
            if aw / ah > aspect {
                (aw, aw / aspect)
            } else {
                (ah * aspect, ah)
            }
        }
        Zoom::Percent(p) => {
            // the photo's own pixels at p% (its width as shown: cropped and rotated)
            let w = img_px[0] as f32 * p / 100.0 / ppp;
            (w, w / aspect)
        }
    };
    let pan = bounded_pan(area, vec2(w, h), pan);
    let c = pos2(area.center().x - (pan.0 - 0.5) * w, area.center().y - (pan.1 - 0.5) * h);
    let rect = Rect::from_center_size(c, vec2(w, h));
    // At 100 % the texture maps texel for texel to screen pixels, so its corner must sit on a whole
    // physical pixel: half a pixel off and the GPU's bilinear filter blurs the view (an odd-width
    // canvas on a 2x display does this). Other zooms resample anyway, and a pinch must not jump.
    let one_to_one = ppp.is_finite() && ppp > 0.0 && (w * ppp - img_px[0] as f32).abs() < 0.5;
    if matches!(zoom, Zoom::Percent(_)) && one_to_one {
        let snap = |v: f32| (v * ppp).round() / ppp;
        return Rect::from_min_size(pos2(snap(rect.min.x), snap(rect.min.y)), rect.size());
    }
    rect
}

/// Fit a decoded image inside the frame calculated from catalog metadata. Unsupported RAWs can
/// only show their embedded camera JPEG, whose crop and aspect ratio can differ from that metadata.
pub(crate) fn fit_texture_rect(area: Rect, size: [usize; 2]) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let scale = (area.width() / w).min(area.height() / h);
    Rect::from_center_size(area.center(), vec2(w * scale, h * scale))
}

/// Keep the viewport inside the photo; axes smaller than the viewport stay centred.
fn bounded_pan(area: Rect, size: egui::Vec2, pan: (f32, f32)) -> (f32, f32) {
    let bound = |visible: f32, image: f32, centre: f32| {
        let centre = if centre.is_finite() { centre } else { 0.5 };
        if image > visible.max(0.0) {
            let half = visible.max(0.0) / image / 2.0;
            centre.clamp(half, 1.0 - half)
        } else {
            0.5
        }
    };
    (bound(area.width(), size.x, pan.0), bound(area.height(), size.y, pan.1))
}

pub(crate) fn pan_image(app: &mut LightcraftApp, area: Rect, img: Rect, delta: egui::Vec2) {
    let centre = area.center() - img.min - delta;
    let pan = bounded_pan(area, img.size(), (centre.x / img.width().max(1.0), centre.y / img.height().max(1.0)));
    let _ = app.run("view.navigate", json!({"pan": pan}));
}

/// Net vertical notches (up is positive) of unmodified line/page-unit wheel events: a real mouse wheel, not a trackpad.
fn wheel_notches(events: &[egui::Event]) -> f32 {
    events
        .iter()
        .filter_map(|e| match e {
            egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page, delta, modifiers, .. }
                if *modifiers == egui::Modifiers::NONE && delta.y.is_finite() =>
            {
                Some(delta.y)
            }
            _ => None,
        })
        .sum()
}

/// Native pinch (also modifier-wheel zoom) and two-finger scroll, scoped to this image view.
pub(crate) fn navigate_gesture(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, area: Rect, img: Rect, native: [usize; 2]) -> bool {
    if !resp.hovered() {
        return false;
    }
    let (mut factor, mut delta, anchor) =
        ui.input(|i| (i.zoom_delta(), i.translation_delta(), i.multi_touch().map(|t| t.center_pos).or(i.pointer.hover_pos())));
    // A notched mouse wheel (line/page units, no modifier) zooms around the pointer, through egui's smoothed wheel
    // delta like Ctrl+wheel (so it glides instead of jumping); trackpads report points and still pan.
    let wheel_id = egui::Id::new("loupe-wheel-zoom");
    if ui.input(|i| wheel_notches(&i.events)) != 0.0 {
        ui.data_mut(|d| d.insert_temp(wheel_id, true));
    }
    if ui.data(|d| d.get_temp::<bool>(wheel_id)).unwrap_or(false) {
        let smooth = ui.input(|i| i.smooth_scroll_delta.y);
        let speed = ui.ctx().options(|o| o.input_options.scroll_zoom_speed);
        if smooth == 0.0 {
            ui.data_mut(|d| d.remove_temp::<bool>(wheel_id));
        } else {
            factor *= (speed * smooth).exp();
            delta = egui::Vec2::ZERO;
        }
    }
    if !factor.is_finite() || factor <= 0.0 || !delta.is_finite() || (factor == 1.0 && delta == egui::Vec2::ZERO) {
        return false;
    }
    let Some(anchor) = anchor.filter(|p| area.contains(*p)) else { return false };
    let ppp = ui.ctx().pixels_per_point();
    let aspect = img.width() / img.height().max(1.0);
    let fit = fit_rect(area, aspect, Zoom::Fit, native, ppp, (0.5, 0.5));
    let fit_pct = fit.width() * ppp * 100.0 / native[0].max(1) as f32;
    let mut zoom = app.ui.zoom;
    let size = if factor != 1.0 {
        // Start at the displayed scale (including Fit/Fill and an interrupted click animation).
        let pct = (img.width() * ppp * 100.0 / native[0].max(1) as f32 * factor).clamp(fit_pct.min(800.0), 800.0);
        zoom = if pct <= fit_pct { Zoom::Fit } else { Zoom::Percent(pct) };
        fit_rect(area, aspect, zoom, native, ppp, (0.5, 0.5)).size()
    } else {
        img.size()
    };
    // The image point under the pointer remains there as its size changes, then follows the pan.
    let point = (anchor - img.min) / img.size();
    let offset = area.center() - anchor - delta;
    let pan = bounded_pan(area, size, (point.x + offset.x / size.x.max(1.0), point.y + offset.y / size.y.max(1.0)));
    let _ = app.run("view.navigate", json!({"zoom": zoom, "pan": pan}));
    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    ui.ctx().request_repaint();
    true
}

/// The largest texture the GPU behind `ctx` takes (egui reports it; 2048 until it does, the
/// smallest limit WebGL devices have).
///
/// The value egui keeps, not the frame's raw input: a native host (egui-winit) reports the limit
/// in the first frame's input only, so the raw value is `None` from the second frame on
/// (issue #652: every window render was cut to 2048 px, with blurry strips beside it).
pub(crate) fn texture_side(ctx: &egui::Context) -> usize {
    ctx.input(|i| i.max_texture_side)
}

/// The photo's own pixels as it is shown: its size after the crop and the user's rotation (what
/// 100 % zoom and the render size limit are measured against), at least 1 × 1.
pub(crate) fn output_px(frame: &Frame) -> [usize; 2] {
    let (w, h) = frame.native_size();
    let px = |v: f64| if v.is_finite() { v.round().clamp(1.0, 1e9) as usize } else { 1 };
    [px(w), px(h)]
}

/// The camera's focus point of `photo` in the displayed frame (its crop, rotation and flips), as the
/// normalized box over the drawn image: `None` when the photo has none or it is cropped away.
pub(crate) fn focus_box(photo: &lightcraft_catalog::Photo) -> Option<lightcraft_geom::Rect> {
    let f = photo.meta.focus.as_ref()?;
    let d = &photo.develop;
    let frame = Frame::with_lens(photo.width.max(1) as usize, photo.height.max(1) as usize, d, true, photo.embedded_lens.as_ref());
    let [w, h] = output_px(&frame);
    let to_out = frame.out_to_norm(w, h).inverse()?;
    // stored on the upright photo: the user's Rotate / Flip on top, then the crop
    let r = d.orientation.map_norm_rect(f.rect);
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)].map(|c| {
        let q = to_out.apply(c);
        (q.x / w as f64, q.y / h as f64)
    });
    let xs = corners.map(|c| c.0);
    let ys = corners.map(|c| c.1);
    let out = lightcraft_geom::Rect {
        x0: xs.iter().copied().fold(f64::INFINITY, f64::min),
        y0: ys.iter().copied().fold(f64::INFINITY, f64::min),
        x1: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        y1: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    };
    if ![out.x0, out.y0, out.x1, out.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    // its centre must be in the frame; the box may reach past the edge (it is clipped when drawn)
    let (cx, cy) = ((out.x0 + out.x1) / 2.0, (out.y0 + out.y1) / 2.0);
    ((0.0..=1.0).contains(&cx) && (0.0..=1.0).contains(&cy)).then_some(out)
}

/// Where Toggle Zoom centres the view on the photo's focus point (`app.ui.pan`), if it has one in frame.
pub(crate) fn focus_pan(app: &LightcraftApp, id: PhotoId) -> Option<(f32, f32)> {
    if let Some(r) = focus_box(app.session.catalog.photo(id)?) {
        return Some((((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32));
    }
    // no point recorded: the main subject's sharpest open eye, else the largest face (when the
    // eye finder has looked at the photo)
    let key = app.session.eye_job(id)?.key;
    let [x, y] = app.eye_strip.report(key)?.zoom_target()?;
    Some((x, y))
}

/// Draw the camera's focus box over a photo drawn in `img` (View ▸ Show Focus Point): green corner
/// brackets with a dark keyline, the way cameras show their AF frame, never smaller than 14 pt.
/// `key` names it for tests and the control channel (`canvas:focusPoint`, `compare:focusPoint:<id>`).
pub(crate) fn draw_focus_point(p: &egui::Painter, img: Rect, photo: &lightcraft_catalog::Photo, key: &str) {
    let Some(r) = focus_box(photo) else { return };
    let at = |x: f64, y: f64| pos2(img.left() + x as f32 * img.width(), img.top() + y as f32 * img.height());
    let b = Rect::from_two_pos(at(r.x0, r.y0), at(r.x1, r.y1));
    let b = Rect::from_center_size(b.center(), b.size().max(vec2(14.0, 14.0)));
    let arm = (b.width().min(b.height()) * 0.3).max(5.0);
    let green = Color32::from_rgb(90, 230, 110);
    for (c, dx, dy) in [(b.left_top(), 1.0, 1.0), (b.right_top(), -1.0, 1.0), (b.left_bottom(), 1.0, -1.0), (b.right_bottom(), -1.0, -1.0)] {
        let pts = vec![c + vec2(dx * arm, 0.0), c, c + vec2(0.0, dy * arm)];
        p.add(egui::Shape::line(pts.clone(), Stroke::new(4.0, Color32::from_black_alpha(170))));
        p.add(egui::Shape::line(pts, Stroke::new(2.0, green)));
    }
    let c = b.center();
    p.circle_filled(c, 2.5, Color32::from_black_alpha(170));
    p.circle_filled(c, 1.5, green);
    register(p.ctx(), key, b);
}

/// Ease the loupe rect toward `target` while a click-zoom animation runs; otherwise follow it exactly.
fn animated_rect(ctx: &egui::Context, anim: &mut bool, target: Rect) -> Rect {
    let t = if *anim { 0.22 } else { 0.0 };
    let id = egui::Id::new("loupe_anim");
    let v = |k: &str, x: f32| ctx.animate_value_with_time(id.with(k), x, t);
    let (c, s) = (target.center(), target.size());
    let r = Rect::from_center_size(pos2(v("cx", c.x), v("cy", c.y)), vec2(v("w", s.x), v("h", s.y)));
    if *anim && (r.center() - c).abs().max_elem() < 0.5 && (r.size() - s).abs().max_elem() < 0.5 {
        *anim = false;
    }
    r
}

/// Whether an overlay can be rendered on a region tile without changing what the tile means.
/// Mask overlays (one mask, or every visible mask at once) are evaluated from the same mask
/// alphas as the full render; the other diagnostic overlays either need whole-frame context or
/// deliberately replace the loupe.
pub(crate) fn overlay_supports_window(overlay: lightcraft_pipeline::Overlay) -> bool {
    use lightcraft_pipeline::Overlay;
    matches!(overlay, Overlay::None | Overlay::Mask { .. } | Overlay::AllMasks { .. })
}

/// What a window render of the loupe needs to know about the view it belongs to.
struct WindowCtx {
    id: PhotoId,
    overlay: lightcraft_pipeline::Overlay,
    /// Long edge of the frame the windows are cut from (`None`: no windows).
    frame_edge: Option<usize>,
    drawn_long: f32,
    aspect: f32,
    ppp: f32,
    texture_side: usize,
    crop_tool: bool,
    interacting: bool,
    /// Hash of the photo's develop settings.
    look: u64,
    /// A pinch or two-finger scroll is running: keep the windows there are.
    holding: bool,
}

/// One side of the loupe that gets a window render.
#[derive(Clone, Copy)]
struct WindowView {
    slot: Slot,
    /// The Before side (the photo without its edits).
    before: bool,
    /// The rect the photo is drawn in.
    img: Rect,
    /// The rect it is requested for (the drawn rect without a click-zoom animation).
    target: Rect,
    /// The area whose pixels are on screen.
    visible: Rect,
}

/// Ask for the window of `v` that holds what is on screen; remember it so its texture can be
/// drawn where it belongs. `None`: no window is wanted (or possible) for this view.
fn request_window(app: &mut LightcraftApp, c: &WindowCtx, v: &WindowView) -> Option<crate::region::RegionView> {
    let frame_edge = c.frame_edge?;
    // while a pinch or scroll runs the windows there are keep being drawn, magnified: no new one
    // per frame (zooming out would ask for ever wider ones)
    if c.holding {
        return if v.before { app.region_before_view } else { app.region_view }.filter(|w| w.photo == c.id);
    }
    let (fw, fh) =
        if c.aspect >= 1.0 { (frame_edge as f32, frame_edge as f32 / c.aspect) } else { (frame_edge as f32 * c.aspect, frame_edge as f32) };
    let (fw, fh) = (fw.round().max(1.0) as usize, fh.round().max(1.0) as usize);
    // what is on screen, in pixels of the window's frame (the drawn frame, scaled)
    let to_px = c.ppp * frame_edge as f32 / c.drawn_long.max(1.0);
    let visible = (
        to_px * (v.visible.left() - v.target.left()),
        to_px * (v.visible.top() - v.target.top()),
        to_px * (v.visible.right() - v.target.left()),
        to_px * (v.visible.bottom() - v.target.top()),
    );
    let win = crate::region::window_for(fw, fh, visible, crate::region::max_span(c.texture_side))?;
    // a window waits for the original another window job is decoding (see `defer_window`)
    let original_held = app.session.media.has_source(c.id, lightcraft_engine::SourceLevel::for_size(fw.max(fh)));
    let window_pending = app.renderer.is_pending(Slot::Region) || app.renderer.is_pending(Slot::RegionBefore);
    if crate::region::defer_window(window_pending, original_held) {
        return if v.before { app.region_before_view } else { app.region_view }.filter(|w| w.photo == c.id);
    }
    let job = if v.before {
        app.session.region_job_before(c.id, fw, fh, win, !c.crop_tool)
    } else {
        app.session.region_job(c.id, fw, fh, win, !c.crop_tool)
    };
    // refused: what the window reads (a spot's source, an Auto Mask stroke) is too far for one
    // render, so the whole frame is rendered at the drawn size instead (see `region::plan`)
    let Some(job) = job else {
        app.window_refused = Some((c.id, c.look, frame_edge));
        return None;
    };
    // a drag renders drafts of the window, as it does of the whole frame. The Before tile stays
    // plain; the After tile carries the same mask overlay as the whole-frame render.
    let job = if c.interacting { job.draft() } else { job };
    let overlay = if v.before { lightcraft_pipeline::Overlay::None } else { c.overlay };
    let job = if v.before { job } else { job.with_hidden_grading(app.ui.grading_hidden_for(c.id)) };
    let job = job.with_overlay(overlay);
    // keyed as the renderer keys the texture (a display profile changes the key)
    let job = app.renderer.job_for(v.slot, job);
    // The overlay is part of the tile's identity too, so changing mask view/colour never leaves a
    // stale region tile on screen while its replacement renders.
    let look = job.settings.hash64() ^ overlay.key().rotate_left(17);
    let view = crate::region::RegionView { photo: c.id, before: v.before, key: job.key, full: (fw, fh), window: win, settings: look };
    // windows of other photos, looks and zooms of this side can't be drawn any more
    app.region_tiles.retain(|(before, _), t| *before != v.before || t.is_current(c.id, (fw, fh), look, c.interacting));
    app.region_tiles.insert((v.before, job.key), view);
    app.renderer.request(v.slot, job, 99);
    Some(view)
}

/// Draw `v`'s window texture over the whole-frame render, if it is the one asked for (or, in a
/// drag, an earlier draft of it).
fn draw_window(p: &egui::Painter, app: &LightcraftApp, c: &WindowCtx, v: &WindowView, wanted: &crate::region::RegionView) -> bool {
    let Some(tex) = app.renderer.textures.get(&v.slot).filter(|t| t.photo == c.id) else { return false };
    let Some(tile) = app.region_tiles.get(&(v.before, tex.key)).filter(|t| t.is_current(c.id, wanted.full, wanted.settings, c.interacting)) else {
        return false;
    };
    let (fw, fh) = (tile.full.0 as f32, tile.full.1 as f32);
    let at = v.img.min + vec2(tile.window.x as f32 / fw * v.img.width(), tile.window.y as f32 / fh * v.img.height());
    let dst = Rect::from_min_size(at, vec2(tile.window.w as f32 / fw * v.img.width(), tile.window.h as f32 / fh * v.img.height()));
    p.image(tex.id(), dst, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    true
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.max_rect();
    let fullscreen = app.ui.fullscreen;
    let show_film = app.ui.filmstrip && !fullscreen;
    let film_h = if show_film { crate::state::FILM_HEIGHT.clamp(app.ui.film_height) } else { 0.0 };
    let canvas = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - film_h));
    app.canvas_rect = Some(canvas);
    if show_film {
        filmstrip(app, ui, Rect::from_min_max(pos2(full.left(), canvas.bottom()), full.max));
    }
    let Some(id) = app.session.active() else {
        if super::empty_library::shown(&app.session) {
            super::empty_library::show(app, ui, canvas);
        } else {
            super::empty_message(ui, canvas, "No photo selected", "Choose a photo in the grid or filmstrip");
        }
        return;
    };
    let Some(photo) = app.session.catalog.photo(id).cloned() else { return };
    let d = (*photo.develop).clone();
    // the full-screen preview shows the photo only: no tool overlays
    let right = if fullscreen { RightPanel::None } else { app.ui.right };
    let crop_tool = right == RightPanel::Crop;
    let frame = Frame::with_lens(photo.width.max(1) as usize, photo.height.max(1) as usize, &d, !crop_tool, photo.embedded_lens.as_ref());
    let aspect = frame.aspect() as f32;
    let ppp = ui.ctx().pixels_per_point();
    let area = canvas.shrink(if fullscreen {
        0.0
    } else if crop_tool {
        48.0
    } else {
        24.0
    });
    let native = output_px(&frame);
    let texture_side = texture_side(ui.ctx());
    // two views (before, after): side by side or stacked
    let split = matches!(app.ui.before_after, BeforeAfter::SideBySide | BeforeAfter::TopBottom);
    let split_view = matches!(app.ui.before_after, BeforeAfter::Split | BeforeAfter::SplitTopBottom);
    let areas: Vec<Rect> = match app.ui.before_after {
        BeforeAfter::SideBySide => {
            let half = area.width() / 2.0 - 6.0;
            vec![
                Rect::from_min_size(area.min, vec2(half, area.height())),
                Rect::from_min_size(pos2(area.center().x + 6.0, area.top()), vec2(half, area.height())),
            ]
        }
        BeforeAfter::TopBottom => {
            let half = area.height() / 2.0 - 14.0;
            vec![
                Rect::from_min_size(area.min, vec2(area.width(), half)),
                Rect::from_min_size(pos2(area.left(), area.center().y + 14.0), vec2(area.width(), half)),
            ]
        }
        _ => vec![area],
    };
    let main_area = *areas.last().unwrap_or(&area);
    let visible_texture_aspect = app
        .renderer
        .textures
        .get(&Slot::Main)
        .filter(|t| t.photo == id)
        .or_else(|| app.renderer.textures.get(&Slot::Preview).filter(|t| t.photo == id))
        .or_else(|| app.renderer.thumb(id))
        .map(|tex| tex.size[0].max(1) as f32 / tex.size[1].max(1) as f32);
    // Follow the texture only where it really differs (an unsupported raw's embedded JPEG with
    // another crop): a texture's whole-pixel size is a little off the exact aspect, and the crop
    // tool's overlays need the frame's own geometry.
    let display_aspect = visible_texture_aspect.filter(|a| !crop_tool && (a / aspect - 1.0).abs() > 0.02).unwrap_or(aspect);
    let resp = ui.interact(canvas, egui::Id::new("loupe"), Sense::click_and_drag());
    crate::widgets::announce(&resp, crate::i18n::tr("Photo"));
    let mut target_rect = fit_rect(main_area, display_aspect, app.ui.zoom, native, ppp, app.ui.pan);
    // the crop tool: the photo turns under a level crop frame (as Lightroom Classic and Capture
    // One do), smaller so it still fits
    let crop_turn = if crop_tool && !split && !split_view { crop_view_turn(&d, &frame, target_rect) } else { 0.0 };
    if crop_turn != 0.0 && matches!(app.ui.zoom, Zoom::Fit) {
        target_rect = fit_turned(target_rect, main_area, crop_turn);
    }
    let mut img_rect = animated_rect(ui.ctx(), &mut app.ui.zoom_anim, target_rect);
    let in_pane = |pane: Rect, image: Rect| image.translate(pane.center() - main_area.center());
    let mut navigating = false;
    for &view_area in &areas {
        let view_img = in_pane(view_area, img_rect);
        if navigate_gesture(app, ui, &resp, view_area, view_img, native) {
            navigating = true;
            target_rect = fit_rect(main_area, display_aspect, app.ui.zoom, native, ppp, app.ui.pan);
            img_rect = target_rect;
            break;
        }
    }
    let before_img = in_pane(areas[0], img_rect);
    let before_target = in_pane(areas[0], target_rect);
    app.image_rect = Some(img_rect);
    // request renders: the loupe at display resolution (drafts during drags)
    let look = d.hash64();
    let now = ui.input(|i| i.time);
    // an edit in progress (a slider held, a control surface's gesture): drafts while the photo
    // changes, the full render once it has been still for a moment (`region::DraftSize`, PERF-EDIT)
    let drag_open = app.session.interaction.is_some();
    let full_ms = app.renderer.last_full_main_ms;
    let interacting = app.draft_size.drafting(drag_open, id, look, now, full_ms);
    if interacting && let Some(left) = app.draft_size.refine_in(now, full_ms) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let overlay = view_overlay(app, &d);
    // a drag drafts at full size where the GPU draws the view (by the GPU's own rule, `handles`;
    // `ready` first: `available` would wait for a device still being created), else as big as
    // the measured cost allows (`region::DragScale`, #330)
    let gpu = interacting
        && lightcraft_engine::gpu::handles(lightcraft_pipeline::OutputDepth::U8, app.ui.soft_proof, overlay)
        && lightcraft_engine::gpu::ready()
        && lightcraft_engine::gpu::available();
    let shown = app.renderer.textures.get(&Slot::Main).filter(|t| t.photo == id && t.quick.is_none()).map(|t| (t.key, t.ms));
    let drag = app.session.interaction.as_ref().filter(|_| interacting).map(|i| (i.photo, i.label.as_str()));
    let draft = app.drag_scale.scale(drag, gpu, shown.map(|(key, _)| key));
    // render at the final size: a click-zoom animation only changes how the result is drawn
    let native_long = native[0].max(native[1]);
    let drawn_long = target_rect.width().max(target_rect.height()) * ppp;
    // the whole frame at about canvas size; the zoomed part of it is a window render (below)
    // Diagnostic views that need whole-frame context, soft proofing, or reads too far for one
    // window still render the whole frame at the size it is drawn. A mask overlay is window-safe.
    let sizes = crate::region::ViewSizes {
        drawn_long,
        canvas_long: canvas.width().max(canvas.height()) * ppp,
        native_long,
        texture_side,
        draft,
        windows: !app.ui.soft_proof && overlay_supports_window(overlay),
    };
    let mut plan = crate::region::plan(&app.ui.settings, sizes);
    if plan.window_edge.is_some_and(|edge| app.window_refused == Some((id, look, edge))) {
        plan = crate::region::plan(&app.ui.settings, crate::region::ViewSizes { windows: false, ..sizes });
    }
    // time the full-size drafts where they are what shows: under a window the whole-frame draft
    // is small anyway, and the frame's time is the window's upload
    if let Some((key, ms)) = shown.filter(|_| plan.window_edge.is_none()) {
        app.drag_scale.observe(key, ms, app.perf.host_frame_ms);
    }
    // a pinch changes the zoom every frame: keep the sizes of before it until it is quiet
    let plan = app.size_hold.apply(id, now, navigating, plan);
    if app.size_hold.holding(now) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(crate::region::HOLD_SECS));
    }
    let want = plan.main_edge;
    let (rw, rh) = if aspect >= 1.0 { (want, (want as f32 / aspect) as usize) } else { ((want as f32 * aspect) as usize, want) };
    // hover and before renders are stand-ins: the preview size is plenty and keeps them cheap
    let se = app.ui.settings.stand_in_edge(want, texture_side);
    let (sw, sh) = if aspect >= 1.0 { (se, (se as f32 / aspect) as usize) } else { ((se as f32 * aspect) as usize, se) };
    // stepping fast (an arrow held down): the render waits until the photo stays, the camera JPEG
    // or thumbnail shows meanwhile, and a render queued for a photo already left is dropped
    let settling = !interacting && app.loupe_timing.settling(id, now);
    if settling {
        if app.renderer.textures.get(&Slot::Main).is_none_or(|t| t.photo != id) {
            app.renderer.drop_queued(Slot::Main);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(crate::loupe_timing::SETTLE_SECS / 2.0));
    }
    if !settling && let Some(job) = app.session.loupe_job(id, rw.max(8), rh.max(8), !crop_tool) {
        let job = if interacting { job.draft() } else { job };
        let job = job.with_overlay(overlay).with_proof(app.ui.soft_proof.then_some(app.ui.proof));
        let job = job.with_hidden_grading(app.ui.grading_hidden_for(id));
        app.renderer.request(Slot::Main, job, 100);
        // a drag's draft that renders in a few ms (GPU) is drawn in this frame, not the next (zoomed
        // in, what changes on screen is the window's draft)
        #[cfg(not(target_arch = "wasm32"))]
        if interacting && plan.window_edge.is_none() {
            let frame = ui.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 30.0);
            // (a frame time that is not a number waits for nothing)
            let budget = std::time::Duration::try_from_secs_f32(frame * 0.4).unwrap_or_default();
            app.renderer.wait_for_draft(ui.ctx(), &mut app.session, budget);
        }
    }
    // hovering a preset or profile: the photo with that look, shown instead of the loupe render
    // once it is ready (nothing is committed)
    let hover_key = match app.hover_preview.clone() {
        Some(h) if !interacting => app.session.preview_job(id, sw.max(8), sh.max(8), !crop_tool, &h.settings).map(|job| {
            let key = job.key;
            app.renderer.request(Slot::Hover, job, 105);
            (key, h.label)
        }),
        _ => None,
    };
    // once this photo is on screen: prepare its neighbours in filmstrip order (source decoded and
    // kept, view render cached) so stepping to them is instant
    // (not while an edit is in progress, even paused: the neighbours' decodes would hold up its drafts)
    if !drag_open && !app.renderer.is_pending(Slot::Main) && app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.photo == id) {
        let ids = app.session.visible_cloned();
        if let Some(i) = ids.iter().position(|p| *p == id) {
            let next = ids.get(i + 1).copied();
            let prev = i.checked_sub(1).and_then(|j| ids.get(j)).copied();
            for (n, nid) in [next, prev].into_iter().enumerate() {
                let Some(nid) = nid else { continue };
                let Some(np) = app.session.catalog.photo(nid).cloned() else { continue };
                let nf = Frame::with_lens(np.width.max(1) as usize, np.height.max(1) as usize, &np.develop, !crop_tool, np.embedded_lens.as_ref());
                let na = nf.aspect() as f32;
                let nr = fit_rect(main_area, na, app.ui.zoom, output_px(&nf), ppp, app.ui.pan);
                let nw =
                    app.ui.settings.prefetch_edge(nr.width().max(nr.height()) * ppp, output_px(&nf).into_iter().max().unwrap_or(1), texture_side);
                let (w, h) = if na >= 1.0 { (nw, (nw as f32 / na) as usize) } else { ((nw as f32 * na) as usize, nw) };
                if let Some(job) = app.session.loupe_job(nid, w.max(8), h.max(8), !crop_tool) {
                    app.renderer.prefetch(Slot::Prefetch(n as u8), job, PREFETCH_PRIORITY);
                }
            }
        }
    }
    // until the loupe has this photo: show its cached render / embedded preview / a thumbnail
    if app.renderer.textures.get(&Slot::Main).is_none_or(|t| t.photo != id)
        && let Some(q) = app.session.quick_view_job(id, want.max(8), !crop_tool)
    {
        app.renderer.request_quick(Slot::Preview, q, 110);
    }
    if !interacting {
        look_ahead(app, id, want.max(8), !crop_tool);
    }
    let show_before = app.ui.before_after == BeforeAfter::Original || ui.input(|i| i.key_down(egui::Key::Backslash));
    if (split || show_before || split_view)
        && let Some(job) = app.session.render_job(id, sw.max(8), sh.max(8), true, !crop_tool)
    {
        app.renderer.request(Slot::Before, job, 90);
    }
    // zoomed past what the whole-frame render holds: also render the part on screen, at no more
    // than 100 % (the GPU magnifies beyond that), for each side that is shown. Mask overlays use
    // the same window; diagnostic overlays that need whole-frame context do not.
    let windowable_view =
        hover_key.is_none() && !app.ui.soft_proof && overlay_supports_window(overlay) && crate::region::same_aspect(display_aspect, aspect);
    let window_ctx = WindowCtx {
        id,
        overlay,
        // (a turned photo in the crop tool is drawn whole: windows are level)
        frame_edge: plan.window_edge.filter(|_| windowable_view && crop_turn == 0.0),
        drawn_long,
        aspect,
        ppp,
        texture_side,
        crop_tool,
        interacting,
        look,
        holding: navigating || app.size_hold.holding(now),
    };
    // (slot, before side?, rect to draw in, rect it is requested for, area whose pixels show)
    let mut window_views: Vec<WindowView> = Vec::new();
    if window_ctx.frame_edge.is_some() {
        let after = WindowView { slot: Slot::Region, before: false, img: img_rect, target: target_rect, visible: main_area };
        if split {
            window_views.push(WindowView { slot: Slot::RegionBefore, before: true, img: before_img, target: before_target, visible: areas[0] });
            window_views.push(after);
        } else if show_before {
            window_views.push(WindowView { slot: Slot::RegionBefore, before: true, img: img_rect, target: target_rect, visible: canvas });
        } else if split_view {
            window_views.push(WindowView { slot: Slot::RegionBefore, before: true, img: img_rect, target: target_rect, visible: canvas });
            window_views.push(WindowView { visible: canvas, ..after });
        } else {
            window_views.push(WindowView { visible: canvas, ..after });
        }
    }
    let mut windows: Vec<(WindowView, crate::region::RegionView)> = Vec::new();
    for v in window_views {
        if let Some(view) = request_window(app, &window_ctx, &v) {
            windows.push((v, view));
        }
    }
    // the renderer polled before these requests: without a frame to come, a render asked for
    // when nothing else repaints (the refine of a still photo in an open interaction) would sit
    // finished until the next input
    app.renderer.keep_polling(ui.ctx());
    for (slot, before) in [(Slot::Region, false), (Slot::RegionBefore, true)] {
        let shown = windows.iter().find(|(v, _)| v.before == before).map(|(_, view)| *view);
        if before {
            app.region_before_view = shown;
        } else {
            app.region_view = shown;
        }
        if shown.is_none() {
            app.region_tiles.retain(|(b, _), _| *b != before);
            app.renderer.release(slot);
        }
    }
    // zoomed in on an unedited raw: its camera JPEG at full size under the window render, so a
    // focus check doesn't wait for the raw (PERF-CULL-5; Capture One's Cull view shows the
    // embedded JPEG at 100 % too). Released when the view is not zoomed any more (180 MB).
    let camera_full = !windows.is_empty()
        && hover_key.is_none()
        && !split
        && !show_before
        && !split_view
        && !interacting
        && app.session.embedded_thumb_is_final(&photo);
    if camera_full {
        if let Some(q) = app.session.embedded_full_job(id, native_long.min(texture_side)) {
            app.renderer.request_quick(Slot::QuickFull, q, 98);
        }
    } else if app.renderer.textures.contains_key(&Slot::QuickFull) || app.renderer.is_pending(Slot::QuickFull) {
        app.renderer.forget(Slot::QuickFull);
    }
    let p = ui.painter_at(canvas);
    // what a view slot shows: its own render of this photo, else the stand-ins (no blank frame
    // between photos, and the full render replaces them in place)
    let draw = |slot: Slot, r: Rect, clip: Rect| -> &'static str {
        let mine = |s: Slot| app.renderer.textures.get(&s).filter(|t| t.photo == id);
        let (tex, what) = if let Some(t) = mine(slot) {
            (t, "render")
        } else if let Some(t) = mine(Slot::Preview) {
            (t, t.quick.map(quick_name).unwrap_or("preview"))
        } else if let Some(t) = app.renderer.thumb(id) {
            (t, "thumb")
        } else {
            return "none";
        };
        let image_rect = fit_texture_rect(r, tex.size);
        if crop_turn != 0.0 {
            image_turned(&p.with_clip_rect(clip), tex.id(), image_rect, crop_turn);
        } else {
            p.with_clip_rect(clip).image(tex.id(), image_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        what
    };
    // soft proofing: a paper-white surround and the proof's name, as Lightroom shows it
    if app.ui.soft_proof && !fullscreen {
        p.rect_filled(canvas, 0.0, Color32::from_gray(238));
        let label = crate::i18n::tr_format!("Proof Preview · {}", app.ui.proof.space.label());
        p.text(pos2(canvas.right() - 16.0, canvas.top() + 14.0), Align2::RIGHT_CENTER, label, t.font(12.5), Color32::from_gray(60));
    }
    let shown;
    if split {
        // each view stays in its pane: zoomed in, a photo reaches past it (and would cover the
        // other view's margins with its whole-frame render, magnified and blurred)
        let br = before_img;
        draw(Slot::Before, br, areas[0]);
        shown = draw(Slot::Main, img_rect, main_area);
        // the labels under each photo, or under its pane when the photo reaches past it
        let label_at = |r: Rect, pane: Rect| pos2(r.left().max(pane.left()), r.bottom().min(pane.bottom()) + 14.0);
        p.text(label_at(br, areas[0]), Align2::LEFT_CENTER, crate::i18n::tr("Before"), t.font(12.0), t.text_dim);
        p.text(label_at(img_rect, main_area), Align2::LEFT_CENTER, crate::i18n::tr("After"), t.font(12.0), t.text_dim);
    } else if show_before {
        shown = draw(Slot::Before, img_rect, canvas);
        p.text(pos2(img_rect.left() + 8.0, img_rect.top() + 14.0), Align2::LEFT_CENTER, crate::i18n::tr("Before"), t.font(12.0), t.text);
    } else if let Some((_key, label)) = &hover_key
        && let Some(tex) = app.renderer.textures.get(&Slot::Hover).filter(|t| t.photo == id)
    {
        let hover_rect = fit_texture_rect(img_rect, tex.size);
        p.image(tex.id(), hover_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let g = p.layout_no_wrap(label.clone(), t.font(12.0), Color32::WHITE);
        let bg = Rect::from_min_size(hover_rect.left_top() + vec2(8.0, 8.0), g.size() + vec2(16.0, 8.0));
        p.rect_filled(bg, 4.0, Color32::from_black_alpha(160));
        p.galley(bg.min + vec2(8.0, 4.0), g, Color32::WHITE);
        shown = "hover";
    } else {
        shown = draw(Slot::Main, img_rect, canvas);
        if shown == "none" {
            match app.renderer.failure_of(Slot::Main, id) {
                Some(e) => {
                    p.text(
                        canvas.center() - vec2(0.0, 10.0),
                        Align2::CENTER_CENTER,
                        crate::i18n::tr("This photo can't be opened"),
                        t.semibold(14.0),
                        t.text,
                    );
                    p.text(canvas.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, e, t.font(12.0), t.text_dim);
                }
                None => {
                    p.text(canvas.center(), Align2::CENTER_CENTER, crate::i18n::tr("Rendering…"), t.font(13.0), t.text_dim);
                }
            }
        }
    }
    app.loupe_shown = Some((id, shown));
    app.loupe_timing.observe(id, shown, now);
    // the camera's own JPEG is on screen, not LightCraft's rendering of the raw: say so, so colour
    // and tone are not judged on it (PERF-CULL-4; Lightroom Classic badges its embedded previews)
    if shown == "embedded" || (shown == "thumb" && app.session.embedded_thumb_is_final(&photo)) {
        camera_preview_badge(ui, canvas, img_rect.intersect(canvas));
    }
    // a wipe shows the Before whole-frame render on its side of the line…
    let line = crate::region::wipe_line(img_rect, canvas);
    if app.ui.before_after == BeforeAfter::Split
        && let Some(tex) = app.renderer.textures.get(&Slot::Before).filter(|t| t.photo == id)
    {
        let before_rect = fit_texture_rect(img_rect, tex.size);
        let u = ((line.x - before_rect.left()) / before_rect.width().max(1.0)).clamp(0.0, 1.0);
        let left = Rect::from_min_max(before_rect.min, pos2(before_rect.left() + u * before_rect.width(), before_rect.bottom()));
        p.image(tex.id(), left, Rect::from_min_max(pos2(0.0, 0.0), pos2(u, 1.0)), Color32::WHITE);
    }
    if app.ui.before_after == BeforeAfter::SplitTopBottom
        && let Some(tex) = app.renderer.textures.get(&Slot::Before).filter(|t| t.photo == id)
    {
        let before_rect = fit_texture_rect(img_rect, tex.size);
        let v = ((line.y - before_rect.top()) / before_rect.height().max(1.0)).clamp(0.0, 1.0);
        let top = Rect::from_min_max(before_rect.min, pos2(before_rect.right(), before_rect.top() + v * before_rect.height()));
        p.image(tex.id(), top, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, v)), Color32::WHITE);
    }
    // the camera JPEG at full size, magnified like the whole-frame render, until the window is there
    let mut camera_full_drawn = false;
    if camera_full
        && let Some(tex) = app.renderer.textures.get(&Slot::QuickFull).filter(|t| t.photo == id)
        && crate::region::same_aspect(tex.size[0].max(1) as f32 / tex.size[1].max(1) as f32, display_aspect)
    {
        p.image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        camera_full_drawn = true;
    }
    // …and the window renders go over the whole-frame ones, all of them, where they belong in the
    // frame (clipped to their pane, or their side of the line)
    let mut window_drawn = false;
    for (v, view) in &windows {
        // (holding `\` in a wipe shows the Before everywhere: it is the Before view alone then)
        let mode = if show_before { BeforeAfter::Original } else { app.ui.before_after };
        let clip = crate::region::window_clip(mode, v.before, v.visible, canvas, img_rect);
        window_drawn |= draw_window(&p.with_clip_rect(clip), app, &window_ctx, v, view);
    }
    if camera_full_drawn && !window_drawn {
        app.loupe_shown = Some((id, "embedded"));
        camera_preview_badge(ui, canvas, img_rect.intersect(canvas));
    }
    // each person's sharper eye at 100 %, found for this photo and the next ones (never waited for)
    if app.eye_focus_on() {
        let ids = app.session.visible_cloned();
        let upcoming = ids.iter().position(|p| *p == id).map(|i| app.look_ahead.targets(&ids, i)).unwrap_or_default();
        crate::eye_strip::show(app, ui, canvas, id, &upcoming);
    }
    // the line of a wipe over both
    if app.ui.before_after == BeforeAfter::Split {
        p.line_segment([pos2(line.x, img_rect.top()), pos2(line.x, img_rect.bottom())], Stroke::new(1.5, Color32::WHITE));
    }
    if app.ui.before_after == BeforeAfter::SplitTopBottom {
        p.line_segment([pos2(img_rect.left(), line.y), pos2(img_rect.right(), line.y)], Stroke::new(1.5, Color32::WHITE));
    }
    if app.ui.show_clipping && !show_before && !fullscreen {
        clipping_overlay(app, &p, img_rect);
    }
    // View ▸ Loupe Overlay ▸ Layout Image, over the photo (the After view in a before/after)
    super::layout_overlay::draw(app, &p, img_rect);
    register(ui.ctx(), "canvas:image", img_rect);
    let map = CanvasMap::new(&frame, img_rect).turned(crop_turn);
    info_overlay(app, &p, canvas, &photo);
    if app.ui.shot_info {
        shot_info_strip(ui, canvas, img_rect, &shot_info(&photo.meta, native));
    }
    if !fullscreen {
        filter_pill(app, ui, canvas);
    }
    if app.ui.face_boxes {
        // a name box left open on another photo is dropped
        if app.ui.name_edit.as_ref().is_some_and(|e| e.photo != id.0) {
            app.ui.name_edit = None;
        }
        let hints = super::faces::hints_for(app, id.0);
        let people = if app.ui.name_edit.is_some() { app.caches.person_names(&app.session.catalog) } else { Default::default() };
        // while the name box is open and recognition is not set up, the box offers the next step
        let setup = if app.ui.name_edit.is_some() { super::faces::setup(app, ui.ctx()) } else { super::faces::Setup::Running };
        match region_overlay(ui, &p, &map, &photo, d.orientation, hints.as_deref(), &people, &mut app.ui.name_edit, setup) {
            Some(RegionEdit::SetUp) => {
                app.ui.name_edit = None;
                super::faces::take_step(app, ui.ctx(), setup);
            }
            Some(RegionEdit::Remove(index)) => {
                let _ = app.run("photo.removeRegion", json!({"id": id.0, "index": index}));
            }
            Some(RegionEdit::Name(index, name)) => {
                app.ui.name_edit = None;
                let _ = app.run("faces.setName", json!({"id": id.0, "index": index, "name": name}));
            }
            Some(RegionEdit::Resize(index, r)) => {
                let _ = app.run("photo.setRegion", json!({"id": id.0, "index": index, "rect": {"x0": r.x0, "y0": r.y0, "x1": r.x1, "y1": r.y1}}));
            }
            None => {}
        }
    }
    if app.ui.settings.focus_point && !crop_tool {
        draw_focus_point(&p.with_clip_rect(img_rect.intersect(canvas)), img_rect, &photo, "canvas:focusPoint");
    }
    // a fine grid while a transform (geometry) slider is dragged, to judge verticals
    if app.ui.dragging_control.as_deref().is_some_and(|c| c.starts_with("geometry.")) {
        let n = 12;
        let stroke = Stroke::new(1.0, Color32::from_white_alpha(70));
        for i in 1..n {
            let fx = img_rect.left() + img_rect.width() * i as f32 / n as f32;
            p.line_segment([pos2(fx, img_rect.top()), pos2(fx, img_rect.bottom())], stroke);
        }
        let rows = ((n as f32) * img_rect.height() / img_rect.width()).round().max(2.0) as usize;
        for i in 1..rows {
            let fy = img_rect.top() + img_rect.height() * i as f32 / rows as f32;
            p.line_segment([pos2(img_rect.left(), fy), pos2(img_rect.right(), fy)], stroke);
        }
    }
    // Cursor selection must also run on gesture frames, when tool input is suppressed.
    if !matches!(right, RightPanel::Crop | RightPanel::Masking | RightPanel::Remove | RightPanel::RedEye) {
        general_cursor(app, ui, &resp, img_rect, main_area);
    }
    if !navigating {
        match right {
            RightPanel::Crop => crop_overlay(app, ui, &resp, &map, &frame, &d, id),
            RightPanel::Masking => mask_overlay(app, ui, &resp, &map, &d),
            RightPanel::Remove => {
                remove_overlay(app, ui, &resp, &map, &d);
                // a spot being generated: a spinner over it until the result arrives
                crate::panels::genfill::canvas(app, ui, &ui.painter_at(app.canvas_rect.unwrap_or(map.rect)), &|q| map.screen(q), &d);
            }
            RightPanel::RedEye => eye_overlay(app, ui, &resp, &map, &d),
            _ => {
                // side by side / stacked: Before shows the same frame, so a click in its view (white
                // balance, colour samples, targeted adjustment) means the same point of the photo.
                // Zoomed in, a view's image reaches past its pane: what counts is the pane clicked.
                // A click-zoom there centres both views on the point clicked (#1742).
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
                let (map, img, pane) = match (split, areas.first(), pointer) {
                    (true, Some(before), Some(q)) if before.contains(q) => (CanvasMap::new(&frame, before_img), before_img, *before),
                    _ => (map, img_rect, main_area),
                };
                general_interaction(app, ui, &resp, &map, img, pane, native, aspect)
            }
        }
    }
    // the overlay switched on to stay (O) overrides showing it only when useful: say so on the photo
    if right == RightPanel::Masking && app.ui.mask_overlay && !adjusting(app) && !fullscreen {
        overlay_on_hint(ui, canvas, app.ui.mask_overlay_color);
    }
    // drawn and hit-tested above the loupe and its tools: clicks on it pan
    let nav = navigator(app, ui, canvas, img_rect, id);
    // an AI mask on its way: what is being found, on the photo, with Cancel (no stand-in overlay)
    if right == RightPanel::Masking
        && let Some(m) = app.session.active_mask.and_then(|mid| d.masks.iter().find(|m| m.id == mid))
        && let Some((comp, what)) = ai_computing(app, m)
    {
        let over = img_rect.intersect(canvas);
        if ai_progress(ui, if over.is_positive() { over } else { canvas }, &what) {
            // the result of a mask that changed is dropped when it arrives
            let r = if m.components.len() <= 1 {
                app.run("mask.delete", json!({"id": m.id}))
            } else {
                app.run("mask.component", json!({"id": m.id, "component": comp, "action": "delete"}))
            };
            if let Err(e) = r {
                app.toast(ui.ctx(), e);
            }
        }
    }
    // what is on screen isn't the render on its way (a stand-in, the last render after an edit,
    // a magnified frame under its window): say so once that has lasted a moment
    let whole: &[Slot] = if show_before && !split {
        &[Slot::Before]
    } else if split || split_view {
        &[Slot::Main, Slot::Before]
    } else {
        &[Slot::Main]
    };
    let coming = !matches!(shown, "none" | "hover")
        && whole.iter().chain(windows.iter().map(|(v, _)| &v.slot)).any(|s| app.renderer.is_pending(*s) && !app.renderer.retrying(*s));
    let keys =
        [Slot::Main, Slot::Before, Slot::Region, Slot::RegionBefore].map(|s| app.renderer.textures.get(&s).filter(|t| t.photo == id).map(|t| t.key));
    let rendering = ui.ctx().data_mut(|d| rendering_overdue(d.get_temp_mut_or(egui::Id::new("loupe-rendering"), None), keys, coming, now));
    // the render failed while a stand-in is up: what is on screen is all there will be (also while
    // a library change retries it)
    let failed = (!rendering
        && (!show_before || split)
        && !matches!(shown, "render" | "none" | "hover")
        && (!app.renderer.is_pending(Slot::Main) || app.renderer.retrying(Slot::Main)))
    .then(|| app.renderer.failure_of(Slot::Main, id))
    .flatten();
    if rendering || failed.is_some() {
        // over the photo (the After in two views); in the crop tool's wide margin, under it
        let over = if crop_tool { canvas } else { img_rect.intersect(if split { main_area } else { canvas }) };
        let over = if over.is_positive() { over } else { canvas };
        match failed {
            Some(e) => failed_pill(ui, app, &photo, canvas, over, nav, "loupe:renderFailed", e),
            None => {
                loupe_pill(ui, canvas, over, nav, "loupe:rendering", crate::i18n::tr("Rendering…"), false);
            }
        }
    }
    if let Some(why) = photo.preview_only.as_deref().filter(|_| !fullscreen) {
        preview_only_pill(ui, canvas, why);
    }
    resp.context_menu(|ui| {
        crate::widgets::menu_button(ui, crate::i18n::tr("Zoom"), |ui| {
            for (label, cmd) in [("Fit", "view.zoomFit"), ("100%", "view.zoom100"), ("Zoom In", "view.zoomIn"), ("Zoom Out", "view.zoomOut")] {
                if ui.button(crate::i18n::tr(label)).clicked() {
                    let _ = app.run(cmd, json!({}));
                }
            }
        });
        ui.separator();
        super::grid::context_menu(app, ui, id);
    });
}

/// "Overlay on · O" at the canvas' bottom left, a swatch of the overlay's colour before it: the
/// mask overlay was switched on to stay (widget `canvas:overlayOn`).
fn overlay_on_hint(ui: &egui::Ui, canvas: Rect, color: [u8; 3]) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let g = p.layout_no_wrap(crate::i18n::tr("Overlay on · O").to_string(), t.font(12.0), Color32::WHITE);
    let bg = Rect::from_min_size(pos2(canvas.left() + 10.0, canvas.bottom() - 18.0 - g.size().y), g.size() + vec2(30.0, 8.0));
    p.rect_filled(bg, bg.height() / 2.0, Color32::from_black_alpha(150));
    let [r, gg, b] = color;
    p.circle_filled(pos2(bg.left() + 12.0, bg.center().y), 4.5, Color32::from_rgb(r, gg, b));
    p.galley(pos2(bg.left() + 22.0, bg.top() + 4.0), g, Color32::WHITE);
    register(ui.ctx(), "canvas:overlayOn", bg);
}

/// The info overlay at the canvas' top left (`view.infoOverlay`): file name with the capture date
/// and size, or with the camera and exposure.
fn info_overlay(app: &LightcraftApp, p: &egui::Painter, canvas: Rect, photo: &lightcraft_catalog::Photo) {
    use crate::state::InfoOverlay;
    let lines: Vec<String> = match app.ui.info_overlay {
        InfoOverlay::Off => return,
        InfoOverlay::Basic => {
            let date = photo.captured.as_deref().unwrap_or(&photo.imported);
            vec![photo.file_name.clone(), format!("{} · {} × {}", pretty_date(date), photo.width, photo.height)]
        }
        InfoOverlay::Exposure => {
            let m = &photo.meta;
            let mut exp = Vec::new();
            if !m.shutter.is_empty() {
                exp.push(format!("{} s", m.shutter));
            }
            if let Some(a) = m.aperture {
                exp.push(format!("f/{a:.1}"));
            }
            if let Some(i) = m.iso {
                exp.push(format!("ISO {i}"));
            }
            if let Some(f) = m.focal_mm {
                exp.push(format!("{f:.0} mm"));
            }
            let camera = [m.camera.as_str(), m.lens.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
            let mut v = vec![photo.file_name.clone()];
            v.push(if exp.is_empty() { "No exposure information".into() } else { exp.join("  ") });
            if !camera.is_empty() {
                v.push(camera);
            }
            v
        }
    };
    let t = Tokens::get(p.ctx());
    let mut y = canvas.top() + 12.0;
    for (i, l) in lines.iter().enumerate() {
        let font = if i == 0 { t.semibold(15.0) } else { t.font(12.5) };
        // laid out uncoloured: `galley` only fills placeholder text with its colour, so a white
        // layout made the shadow a second white copy (#1690)
        let g = p.layout_no_wrap(l.clone(), font, Color32::PLACEHOLDER);
        let at = pos2(canvas.left() + 14.0, y);
        // a soft shadow keeps the text readable on bright photos
        p.galley(at + vec2(1.0, 1.0), g.clone(), Color32::from_black_alpha(200));
        p.galley(at, g.clone(), Color32::WHITE);
        y += g.size().y + 3.0;
    }
    register(p.ctx(), "canvas:infoOverlay", Rect::from_min_max(canvas.min, pos2(canvas.left() + 320.0, y)));
}

/// What the user did to a face box this frame.
enum RegionEdit {
    /// The × was clicked.
    Remove(usize),
    /// A handle drag ended: the region's new box (normalized, upright frame).
    Resize(usize, lightcraft_geom::Rect),
    /// A name was typed or picked for the face.
    Name(usize, String),
    /// The name box's offer to set up face recognition was pressed.
    SetUp,
}

/// Handles of a box: (x, y) as fractions of its width and height, and the cursor they show.
const REGION_HANDLES: [(f32, f32, egui::CursorIcon); 8] = [
    (0.0, 0.0, egui::CursorIcon::ResizeNwSe),
    (0.5, 0.0, egui::CursorIcon::ResizeVertical),
    (1.0, 0.0, egui::CursorIcon::ResizeNeSw),
    (1.0, 0.5, egui::CursorIcon::ResizeHorizontal),
    (1.0, 1.0, egui::CursorIcon::ResizeNwSe),
    (0.5, 1.0, egui::CursorIcon::ResizeVertical),
    (0.0, 1.0, egui::CursorIcon::ResizeNeSw),
    (0.0, 0.5, egui::CursorIcon::ResizeHorizontal),
];

/// Face/pet/focus regions read from XMP (MWG-RS), drawn as boxes over the photo. Hovering a box shows
/// a × in its corner and eight resize handles. A drag previews the new box live and is reported once,
/// on release (one undo step); the × reports the region to remove. Both are catalog-only edits:
/// LightCraft doesn't write regions to XMP. Regions are stored on the upright (EXIF-oriented) photo;
/// `orient` is the user's Rotate / Flip on top of it, which the loupe's normalized frame includes.
fn region_overlay(
    ui: &egui::Ui,
    p: &egui::Painter,
    map: &CanvasMap,
    photo: &lightcraft_catalog::Photo,
    orient: lightcraft_geom::Orientation,
    hints: Option<&super::faces::Hints>,
    people: &[String],
    editing: &mut Option<crate::state::NameEdit>,
    setup: super::faces::Setup,
) -> Option<RegionEdit> {
    let t = Tokens::get(p.ctx());
    let clip = p.clip_rect();
    let pointer = ui.input(|i| i.pointer.hover_pos());
    // the box being dragged: (region index, its box so far)
    let drag_key = egui::Id::new("region-drag");
    let live: Option<(usize, lightcraft_geom::Rect)> = ui.data(|d| d.get_temp(drag_key));
    let mut edit = None;
    for (index, r) in photo.meta.regions.iter().enumerate() {
        let dragging = live.filter(|(i, _)| *i == index);
        let norm = orient.map_norm_rect(dragging.map_or(r.rect, |(_, n)| n));
        let rect = Rect::from_two_pos(map.screen(Point::new(norm.x0, norm.y0)), map.screen(Point::new(norm.x1, norm.y1)));
        // white with a black keyline just outside it, so the box shows on any background
        p.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_black_alpha(190)), StrokeKind::Outside);
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Outside);
        if dragging.is_some() || (live.is_none() && pointer.is_some_and(|h| rect.expand(8.0).contains(h))) {
            for (h, (fx, fy, cursor)) in REGION_HANDLES.iter().enumerate() {
                let c = pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height());
                let resp = ui.interact(Rect::from_center_size(c, vec2(14.0, 14.0)), egui::Id::new(("region-handle", index, h)), Sense::drag());
                crate::widgets::announce(&resp, crate::i18n::tr("Resize region"));
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(*cursor);
                }
                let sq = Rect::from_center_size(c, vec2(7.0, 7.0));
                p.rect_filled(sq, 0.0, if resp.hovered() || resp.dragged() { Color32::from_rgb(120, 190, 255) } else { Color32::WHITE });
                p.rect_stroke(sq, 0.0, Stroke::new(1.0, Color32::from_black_alpha(220)), StrokeKind::Outside);
                if resp.dragged()
                    && let Some(pp) = resp.interact_pointer_pos()
                {
                    // move the dragged edge(s) to the pointer, never closer than 12 px to the opposite one
                    let mut n = rect;
                    if *fx == 0.0 {
                        n.min.x = pp.x.min(n.max.x - 12.0);
                    } else if *fx == 1.0 {
                        n.max.x = pp.x.max(n.min.x + 12.0);
                    }
                    if *fy == 0.0 {
                        n.min.y = pp.y.min(n.max.y - 12.0);
                    } else if *fy == 1.0 {
                        n.max.y = pp.y.max(n.min.y + 12.0);
                    }
                    let (a, b) = (map.norm(n.min), map.norm(n.max));
                    let shown = lightcraft_geom::Rect {
                        x0: a.x.min(b.x).clamp(0.0, 1.0),
                        y0: a.y.min(b.y).clamp(0.0, 1.0),
                        x1: a.x.max(b.x).clamp(0.0, 1.0),
                        y1: a.y.max(b.y).clamp(0.0, 1.0),
                    };
                    // back to the upright frame the region is stored in
                    let new = orient.inverse().map_norm_rect(shown);
                    ui.data_mut(|d| d.insert_temp(drag_key, (index, new)));
                    ui.ctx().request_repaint();
                }
                if resp.drag_stopped() {
                    if let Some((i, new)) = ui.data(|d| d.get_temp::<(usize, lightcraft_geom::Rect)>(drag_key)) {
                        edit = Some(RegionEdit::Resize(i, new));
                    }
                    ui.data_mut(|d| d.remove_temp::<(usize, lightcraft_geom::Rect)>(drag_key));
                }
            }
        }
        if dragging.is_none() && live.is_none() && pointer.is_some_and(|h| rect.expand(3.0).contains(h)) {
            let xr = Rect::from_center_size(pos2(rect.right() - 13.0, rect.top() + 13.0), vec2(16.0, 16.0));
            let resp = ui.interact(xr, egui::Id::new(("region-x", index)), Sense::click());
            crate::widgets::announce(&resp, crate::i18n::tr("Remove region"));
            register(ui.ctx(), format!("regionRemove:{index}"), xr);
            let fill = Color32::from_black_alpha(if resp.hovered() { 235 } else { 190 });
            p.rect_filled(xr, 2.0, fill);
            p.rect_stroke(xr, 2.0, Stroke::new(1.0, Color32::from_white_alpha(120)), StrokeKind::Inside);
            let (c, m) = (xr.center(), 3.5);
            let cross = Stroke::new(1.4, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(210) });
            p.line_segment([c - vec2(m, m), c + vec2(m, m)], cross);
            p.line_segment([c - vec2(m, -m), c + vec2(m, -m)], cross);
            if resp.on_hover_text(crate::i18n::tr("Remove this face box (undo with Edit ▸ Undo)")).clicked() {
                edit = Some(RegionEdit::Remove(index));
            }
        }
        // what the label says: the name, a guess to confirm ("Jane Doe?"), or on hover an invitation to name the face
        let is_face = r.kind == lightcraft_meta::RegionKind::Face;
        let hint = hints.and_then(|h| h.by_index.get(&index));
        let hovered = pointer.is_some_and(|h| rect.expand(3.0).contains(h));
        let editing_this = editing.as_ref().is_some_and(|e| e.photo == photo.id.0 && e.index == index);
        let (text, text_color, fill_alpha, invitation) = match (&r.name, hint.and_then(|h| h.suggestion.as_ref())) {
            (Some(n), _) => (n.clone(), Color32::from_gray(225), 235, false),
            (None, Some((n, _))) if is_face => (format!("{n}?"), Color32::from_gray(185), 200, false),
            (None, None) if is_face && live.is_none() => ("Add name".to_string(), Color32::from_gray(150), 190, true),
            _ => continue,
        };
        // the name in a dark label with a caret, centred above the box (below it when there is no room)
        let name = &text;
        let g = p.layout_no_wrap(name.clone(), t.font(13.0), text_color);
        let (pad, caret) = (vec2(14.0, 7.0), 5.0);
        let size = g.size() + pad * 2.0;
        let above = rect.top() - caret - size.y >= clip.top();
        let (top, tip, base) = if above {
            (rect.top() - caret - size.y, rect.top() - 1.0, rect.top() - caret - 1.0)
        } else {
            (rect.bottom() + caret, rect.bottom() + 1.0, rect.bottom() + caret + 1.0)
        };
        let left = (rect.center().x - size.x / 2.0).clamp(clip.left(), (clip.right() - size.x).max(clip.left()));
        let label = Rect::from_min_size(pos2(left, top), size);
        // the invitation shows while the pointer is on the box *or on the label* (and the gap between), so it can be reached
        if invitation && !(hovered || editing_this || pointer.is_some_and(|h| rect.union(label).expand(3.0).contains(h))) {
            continue;
        }
        let fill = Color32::from_rgba_unmultiplied(56, 56, 56, fill_alpha);
        p.rect_filled(label, 3.0, fill);
        p.rect_stroke(label, 3.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Inside);
        let cx = rect.center().x.clamp(label.left() + caret + 4.0, label.right() - caret - 4.0);
        p.add(egui::Shape::convex_polygon(vec![pos2(cx - caret, base), pos2(cx + caret, base), pos2(cx, tip)], fill, Stroke::NONE));
        p.galley(label.min + pad, g, text_color);
        if is_face && live.is_none() {
            // a click on the label opens the name box, filled with the name or the guess
            let resp = ui.interact(label, egui::Id::new(("region-label", index)), Sense::click());
            crate::widgets::announce(&resp, crate::i18n::tr("Click to name this face"));
            register(ui.ctx(), format!("regionLabel:{index}"), label);
            if resp.on_hover_text(crate::i18n::tr("Click to name this face")).clicked() {
                let start = r.name.clone().or_else(|| hint.and_then(|h| h.suggestion.as_ref().map(|(n, _)| n.clone()))).unwrap_or_default();
                *editing = Some(crate::state::NameEdit { photo: photo.id.0, index, text: start, fresh: true });
            }
        }
        if editing_this && let Some(e) = editing.as_mut() {
            let candidates = hint.map(|h| h.candidates.as_slice()).unwrap_or(&[]);
            match super::faces::name_editor(ui.ctx(), pos2(label.left(), rect.bottom() + 8.0), e, candidates, people, setup) {
                super::faces::Editor::Submit(name) => edit = Some(RegionEdit::Name(index, name)),
                super::faces::Editor::Setup => edit = Some(RegionEdit::SetUp),
                super::faces::Editor::Cancel => *editing = None,
                super::faces::Editor::Open => {}
            }
        }
    }
    edit
}

/// A small pill at the loupe's top left naming the active filters: the filmstrip and Next / Previous
/// follow them, so a filter must never be invisible here. A click clears them all. It sits in the
/// canvas margin, so the photo does not move.
fn filter_pill(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect) {
    let chips = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog);
    if chips.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let label =
        chips.iter().map(|c| crate::panels::chips::display_label(c, &app.session.filter, &app.session.catalog)).collect::<Vec<_>>().join("  ·  ");
    let g = p.layout_no_wrap(crate::i18n::tr_format!("Filtered: {label}", label = label), t.font(12.0), Color32::from_gray(225));
    // the same inset from the left as from the top, so the pill sits in the corner (#1645)
    let inset = 3.0;
    let w = (g.size().x + 40.0).min((canvas.width() - 2.0 * inset).max(60.0));
    let r = Rect::from_min_size(canvas.min + vec2(inset, inset), vec2(w, 20.0));
    let resp = ui.interact(r, egui::Id::new("loupe-filter-pill"), Sense::click());
    crate::widgets::announce(&resp, crate::i18n::tr("Filters"));
    register(ui.ctx(), "loupe:filters", r);
    p.rect_filled(r, 10.0, Color32::from_black_alpha(if resp.hovered() { 225 } else { 185 }));
    let text_clip = Rect::from_min_max(r.min, pos2(r.right() - 24.0, r.max.y));
    p.with_clip_rect(text_clip).galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, Color32::from_gray(225));
    let (c, m) = (pos2(r.right() - 13.0, r.center().y), 3.0);
    let cross = Stroke::new(1.3, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(190) });
    p.line_segment([c - vec2(m, m), c + vec2(m, m)], cross);
    p.line_segment([c - vec2(m, -m), c + vec2(m, -m)], cross);
    if resp.on_hover_text(crate::i18n::tr("Click to clear these filters")).clicked() {
        let _ = app.run("library.clearFilter", json!({}));
    }
}

/// A raw shown from its embedded JPEG (issue #10): a pill at the canvas' top centre saying so;
/// hovering it explains (registered as `notice:previewOnly:loupe`).
/// A small "Camera preview" label at the top right of the photo while the loupe shows the camera's
/// embedded JPEG (widget `loupe:cameraPreview`), with what that means on hover.
fn camera_preview_badge(ui: &egui::Ui, canvas: Rect, over: Rect) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let text = crate::i18n::tr("Camera preview");
    let g = p.layout_no_wrap(text.to_string(), t.font(11.5), Color32::from_gray(225));
    let size = vec2(g.size().x + 16.0, 18.0);
    let over = if over.is_positive() { over } else { canvas };
    let r = Rect::from_min_size(pos2(over.right() - size.x - 8.0, over.top() + 8.0), size);
    p.rect_filled(r, 9.0, Color32::from_black_alpha(165));
    p.galley(pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, Color32::from_gray(225));
    register(ui.ctx(), "loupe:cameraPreview", r);
    let tip = crate::i18n::tr("The camera's own JPEG, shown until LightCraft has rendered the raw. Judge colour and tone on the rendered photo.");
    let resp = ui.interact(r, egui::Id::new("camera-preview-badge"), Sense::hover()).on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{text}: {tip}")));
}

fn preview_only_pill(ui: &mut egui::Ui, canvas: Rect, reason: &str) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let text = crate::i18n::tr_format!("Preview only — editing the camera's embedded JPEG ({})", crate::widgets::preview_only_variant(reason));
    let g = p.layout_no_wrap(text, t.font(12.0), Color32::WHITE);
    let size = vec2(g.size().x + 40.0, 26.0);
    let r = Rect::from_min_size(pos2(canvas.center().x - size.x / 2.0, canvas.top() + 12.0), size);
    p.rect_filled(r, 13.0, Color32::from_black_alpha(185));
    crate::icons::paint(&p, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(15.0, 15.0)), crate::icons::Icon::Info, t.caution);
    p.galley(pos2(r.left() + 29.0, r.center().y - g.size().y / 2.0), g, Color32::WHITE);
    register(ui.ctx(), "notice:previewOnly:loupe", r);
    ui.interact(r, egui::Id::new("preview-only-pill"), Sense::hover()).on_hover_text(crate::widgets::preview_only_explanation(reason));
}

/// The shot info strip's parts, most important first (the strip drops them from the end when it
/// is narrow): aperture, shutter, ISO, focal length shot, the lens (its model, else its focal
/// range), the picture's size in pixels as shown (after the crop, oriented). Only what the photo's
/// metadata already holds: nothing is read from the file.
pub(crate) fn shot_info(m: &lightcraft_catalog::Meta, size: [usize; 2]) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(a) = m.aperture.filter(|a| a.is_finite() && *a > 0.0) {
        v.push(if a >= 10.0 { format!("f/{a:.0}") } else { format!("f/{a:.1}") });
    }
    if !m.shutter.trim().is_empty() {
        v.push(format!("{} s", m.shutter.trim()));
    }
    if let Some(i) = m.iso {
        v.push(format!("ISO {i}"));
    }
    if let Some(f) = m.focal_mm.filter(|f| f.is_finite() && *f > 0.0) {
        v.push(format!("{f:.0} mm"));
    }
    let lens = m.lens.trim();
    if !lens.is_empty() {
        v.push(lens.to_string());
    } else if let Some((a, b)) = m.lens_range.filter(|(a, b)| a.is_finite() && b.is_finite() && *a > 0.0) {
        // (the focal length shot comes first; this is the lens's range, in brackets)
        v.push(if (b - a).abs() < 0.5 { format!("({a:.0} mm)") } else { format!("({a:.0}–{b:.0} mm)") });
    }
    if size[0] > 1 && size[1] > 1 {
        v.push(format!("{} × {}", size[0], size[1]));
    }
    v
}

/// The shot info strip: one line under the photo (inside its bottom edge when there is no room
/// below it, as in full screen), its parts dropped from the end when it doesn't fit (widget
/// `loupe:shotInfo`).
fn shot_info_strip(ui: &egui::Ui, canvas: Rect, img: Rect, parts: &[String]) {
    if parts.is_empty() || !img.is_positive() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let font = t.font(11.5);
    let h = 18.0;
    let below = img.bottom() + 3.0 + h <= canvas.bottom();
    let top = if below { img.bottom() + 3.0 } else { img.bottom() - h - 6.0 };
    let max_w = (img.width().min(canvas.width()) - 16.0).max(40.0);
    let colour = if below { t.text_dim } else { Color32::from_gray(230) };
    let width = |s: &str| p.layout_no_wrap(s.to_string(), font.clone(), colour).size().x;
    let mut shown: Vec<String> = parts.to_vec();
    // least important last: the size, then the lens give way first; the last part left is cut
    // short with an ellipsis rather than dropped
    while shown.len() > 1 && strip_width(&shown.iter().map(|s| width(s)).collect::<Vec<_>>()) > max_w {
        shown.pop();
    }
    if shown.len() == 1
        && let Some(only) = shown.first_mut()
    {
        let mut chars: Vec<char> = only.chars().collect();
        while width(only) > max_w && chars.len() > 1 {
            chars.pop();
            *only = chars.iter().collect::<String>().trim_end().to_string() + "…";
        }
    }
    if shown.is_empty() {
        return;
    }
    let galleys: Vec<_> = shown.into_iter().map(|s| p.layout_no_wrap(s, font.clone(), colour)).collect();
    let total = strip_width(&galleys.iter().map(|g| g.size().x).collect::<Vec<_>>());
    let r = Rect::from_center_size(pos2(img.center().x, top + h / 2.0), vec2(total + 2.0 * STRIP_PAD, h));
    if !below {
        p.rect_filled(r, 9.0, Color32::from_black_alpha(140));
    }
    let mut x = r.left() + STRIP_PAD;
    let n = galleys.len();
    for (i, g) in galleys.into_iter().enumerate() {
        let w = g.size().x;
        p.galley(pos2(x, r.center().y - g.size().y / 2.0), g, colour);
        x += w;
        if i + 1 < n {
            // a faint dot between the items, in the middle of a wide gap
            p.circle_filled(pos2(x + STRIP_GAP / 2.0, r.center().y), 1.3, colour.gamma_multiply(0.55));
            x += STRIP_GAP;
        }
    }
    register(ui.ctx(), "loupe:shotInfo", r);
}

/// Space between two items of the shot info strip (a dot sits in its middle), and around them.
const STRIP_GAP: f32 = 22.0;
const STRIP_PAD: f32 = 10.0;

/// Width of the shot info strip's items (their widths) with the gaps between them.
pub(crate) fn strip_width(widths: &[f32]) -> f32 {
    widths.iter().sum::<f32>() + STRIP_GAP * widths.len().saturating_sub(1) as f32
}

/// `2026-09-30T12:00:00` → `2026-09-30 12:00`.
fn pretty_date(iso: &str) -> String {
    let d = iso.replacen('T', " ", 1);
    d.get(..16).map(str::to_string).unwrap_or(d)
}

/// Size of the Navigator mini map (points).
const NAV_W: f32 = 180.0;

/// The Navigator: while zoomed in, a mini map of the photo at the canvas' bottom right with the
/// visible region outlined; click or drag on it to pan. Returns the area it covers.
fn navigator(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, img: Rect, id: PhotoId) -> Option<Rect> {
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    if !app.ui.navigator || !zoomed || app.ui.fullscreen {
        return None;
    }
    let tex = app.renderer.thumb(id).or_else(|| app.renderer.textures.get(&Slot::Main).filter(|t| t.photo == id));
    let aspect = img.width() / img.height().max(1.0);
    let (w, h) = if aspect >= 1.0 { (NAV_W, NAV_W / aspect) } else { (NAV_W * aspect, NAV_W) };
    let frame = Rect::from_min_size(pos2(canvas.right() - w - 16.0, canvas.bottom() - h - 16.0), vec2(w, h));
    let p = ui.painter_at(canvas);
    p.rect_filled(frame.expand(5.0), 4.0, Color32::from_black_alpha(190));
    match tex {
        Some(t) => p.image(t.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE),
        None => p.rect_filled(frame, 0.0, Color32::from_gray(40)),
    };
    // the visible part of the photo, in normalized image coordinates
    let vis = canvas.intersect(img);
    let n = |q: Pos2| pos2((q.x - img.left()) / img.width(), (q.y - img.top()) / img.height());
    let (a, b) = (n(vis.min), n(vis.max));
    let to_nav = |q: Pos2| pos2(frame.left() + q.x * frame.width(), frame.top() + q.y * frame.height());
    let view = Rect::from_min_max(to_nav(a), to_nav(b));
    p.rect_stroke(view, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
    p.rect_stroke(view.expand(1.5), 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
    register(ui.ctx(), "canvas:navigator", frame);
    let resp = ui.interact(frame.expand(5.0), egui::Id::new("navigator"), Sense::click_and_drag());
    crate::widgets::announce(&resp, crate::i18n::tr("Navigator"));
    if (resp.clicked() || resp.dragged())
        && let Some(q) = resp.interact_pointer_pos()
    {
        // centre the view on the point under the pointer
        let u = ((q.x - frame.left()) / frame.width()).clamp(0.0, 1.0);
        let v = ((q.y - frame.top()) / frame.height()).clamp(0.0, 1.0);
        app.ui.pan = (u, v);
    }
    Some(frame.expand(5.0))
}

/// Neighbour prefetch: below on-screen thumbnails, above background thumbnail refreshes.
const PREFETCH_PRIORITY: u32 = 4;

/// How many photos ahead (in the direction of travel) the loupe makes its stand-in for, and how
/// many behind (a step back is common when culling).
const AHEAD: usize = 5;
const BEHIND: usize = 1;
/// The look-ahead's priority: below the loupe's own render (100), above everything else.
const AHEAD_PRIORITY: u32 = 96;
/// Thumbnails the loupe keeps while stepping (as many as the grid keeps), and how many photos on
/// either side of the open one stay when it trims them.
const MAX_LOUPE_THUMBS: usize = 600;
const KEEP_AROUND: usize = 64;
/// The thumbnail size the look-ahead asks for (the filmstrip's at 2x).
const AHEAD_THUMB_EDGE: usize = 256;

/// What [`look_ahead`] last worked out (see `LightcraftApp::look_ahead`).
#[derive(Debug, Default)]
pub struct LookAhead {
    /// The last photo the loupe showed, for the direction of travel.
    last: Option<PhotoId>,
    /// +1 forward (the default), -1 backward.
    dir: i8,
    /// (photo, direction, catalog revision, edge) the requests were made for.
    made_for: Option<(PhotoId, i8, u64, usize)>,
}

impl LookAhead {
    /// The photos to prepare around `id` at position `i` of `ids`: [`AHEAD`] in the direction of
    /// travel, nearest first, then [`BEHIND`] the other way.
    fn targets(&self, ids: &[PhotoId], i: usize) -> Vec<PhotoId> {
        let dir: isize = if self.dir < 0 { -1 } else { 1 };
        let at = |k: isize| i.checked_add_signed(k).and_then(|j| ids.get(j)).copied();
        let ahead = (1..=AHEAD as isize).filter_map(|k| at(dir * k));
        let behind = (1..=BEHIND as isize).filter_map(|k| at(-dir * k));
        ahead.chain(behind).collect()
    }
}

/// Culling: make the loupe's stand-ins (the camera JPEG of an unedited raw, or a cached render) of
/// the next photos in the direction of travel, so that a step shows the next one at once instead
/// of after reading and decoding it (see [`Slot::QuickAhead`]).
fn look_ahead(app: &mut LightcraftApp, id: PhotoId, edge: usize, crop: bool) {
    let rev = app.session.catalog.revision;
    if app.look_ahead.last != Some(id) {
        let ids = app.session.visible_cloned();
        let pos = |p: PhotoId| ids.iter().position(|q| *q == p);
        if let (Some(prev), Some(i)) = (app.look_ahead.last, pos(id))
            && let Some(j) = pos(prev)
        {
            app.look_ahead.dir = if j > i { -1 } else { 1 };
        }
        app.look_ahead.last = Some(id);
    }
    let want = (id, app.look_ahead.dir, rev, edge);
    if app.look_ahead.made_for == Some(want) {
        return;
    }
    app.look_ahead.made_for = Some(want);
    let ids = app.session.visible_cloned();
    let Some(i) = ids.iter().position(|p| *p == id) else { return };
    let targets = app.look_ahead.targets(&ids, i);
    // stepping through a whole wedding in the loupe would keep every thumbnail it ever drew: keep
    // the ones around the photo (what the filmstrip can show) once there are many
    if app.renderer.thumb_textures() > MAX_LOUPE_THUMBS {
        let keep = ids.iter().skip(i.saturating_sub(KEEP_AROUND)).take(2 * KEEP_AROUND + 1).copied().collect();
        app.renderer.evict_thumbs(&keep, MAX_LOUPE_THUMBS);
    }
    // their thumbnails first (a small embedded JPEG each, a few ms): something to show at once
    // even when a step outruns the full-size previews, and in full screen, where no filmstrip
    // asks for them
    for &nid in &targets {
        super::grid::request_thumb(app, nid, AHEAD_THUMB_EDGE, AHEAD_PRIORITY + 2);
    }
    let jobs = targets.into_iter().filter_map(|nid| app.session.quick_view_job(nid, edge, crop)).collect();
    app.renderer.look_ahead(jobs, AHEAD_PRIORITY);
}

/// How long a render may be on its way, with nothing new on screen, before the loupe says
/// "Rendering…" (s): as long as the activity stack waits before it shows a task.
const RENDERING_AFTER_SECS: f64 = 0.5;

/// What [`rendering_overdue`] keeps between frames while a render is coming: the texture keys on
/// screen, since when, and whether the pill is up.
type RenderingWait = Option<([Option<u64>; 4], f64, bool)>;

/// Whether the loupe says "Rendering…": a render has been `coming` for [`RENDERING_AFTER_SECS`] with
/// no new texture `keys` on screen. The wait starts when the render starts coming (an idle app draws
/// no frames, so the last one may be long ago), and once up the pill stays until nothing is coming:
/// drafts slower than the wait don't make it blink.
pub(crate) fn rendering_overdue(wait: &mut RenderingWait, keys: [Option<u64>; 4], coming: bool, now: f64) -> bool {
    if !coming {
        *wait = None;
        return false;
    }
    let (seen, since, up) = wait.get_or_insert((keys, now, false));
    if !*up && *seen != keys {
        (*seen, *since) = (keys, now);
    }
    *up |= now - *since > RENDERING_AFTER_SECS;
    *up
}

/// A pill over the photo, 12 pt above the bottom of `over` or just above the Navigator when that
/// is in the way, registered as `id`; `caution` adds the preview-only pill's caution icon. The
/// loupe's "Rendering…" says the pixels on screen aren't final, so it isn't an activity row (see
/// docs/background-tasks.md); "This photo can't be opened" says the stand-in on screen is all
/// there will be.
pub(crate) fn loupe_pill(ui: &egui::Ui, canvas: Rect, over: Rect, navigator: Option<Rect>, id: &str, text: &str, caution: bool) -> Rect {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let g = p.layout_no_wrap(text.to_string(), t.font(12.0), Color32::from_gray(225));
    let inset = if caution { 29.0 } else { 10.0 };
    let size = vec2(g.size().x + inset + 10.0, 20.0);
    let mut r = Rect::from_center_size(pos2(over.center().x, over.bottom() - 12.0 - size.y / 2.0), size);
    if let Some(nav) = navigator.filter(|n| n.expand(8.0).intersects(r)) {
        r = Rect::from_min_size(pos2(nav.right() - size.x, nav.top() - 8.0 - size.y), size);
    }
    p.rect_filled(r, 10.0, Color32::from_black_alpha(185));
    if caution {
        crate::icons::paint(&p, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(13.0, 13.0)), crate::icons::Icon::Info, t.caution);
    }
    p.galley(pos2(r.left() + inset, r.center().y - g.size().y / 2.0), g, Color32::from_gray(225));
    register(ui.ctx(), id, r);
    r
}

/// The caution pill saying a render failed behind the stand-in on screen (see [`loupe_pill`]):
/// "Original offline · no smart preview" when that is why, else "This photo can't be opened", with
/// the error on hover and for screen readers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn failed_pill(
    ui: &egui::Ui,
    app: &LightcraftApp,
    photo: &lightcraft_catalog::Photo,
    canvas: Rect,
    over: Rect,
    navigator: Option<Rect>,
    id: &str,
    error: &str,
) {
    let offline = matches!(&photo.source, lightcraft_catalog::Source::File { path } if app.session.media.availability.is_offline(path));
    let text = crate::i18n::tr(if offline { "Original offline · no smart preview" } else { "This photo can't be opened" });
    let r = loupe_pill(ui, canvas, over, navigator, id, text, true);
    let resp = ui.interact(r, egui::Id::new(("render-failed", id)), Sense::hover()).on_hover_text(error);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{text}: {error}")));
}

/// The selected mask `m`'s AI selection still being computed by its model: the component and what
/// is being found ("Finding the subject…"). Meanwhile no overlay shows for the mask: a stand-in
/// would look like the result.
pub(crate) fn ai_computing(app: &LightcraftApp, m: &lightcraft_develop::Mask) -> Option<(usize, String)> {
    use lightcraft_engine::segment::AutoKind;
    let seg = &app.session.segmenter;
    if !seg.busy() {
        return None;
    }
    m.components.iter().enumerate().find_map(|(k, c)| {
        let what = match &c.shape {
            MaskShape::Prompt { text, seg: None, .. } => crate::i18n::tr_format!("Finding “{text}”…", text = text.trim()),
            MaskShape::Object { seg: None, hint, .. } if !hint.is_empty() => crate::i18n::tr("Selecting the object…").to_string(),
            s => {
                let kind = AutoKind::of(s).filter(|k| k.seg_of(s).is_some_and(Option::is_none) && seg.auto_model(k).is_ok())?;
                crate::i18n::tr(match kind {
                    AutoKind::Subject => "Finding the subject…",
                    AutoKind::Background => "Finding the background…",
                    AutoKind::Sky => "Finding the sky…",
                    AutoKind::People { .. } => "Finding people…",
                    AutoKind::Landscape { .. } => "Finding the landscape…",
                    AutoKind::Concept { .. } => "Finding the remembered object…",
                })
                .to_string()
            }
        };
        Some((k, what))
    })
}

/// The AI mask's progress in the middle of the photo: a spinner, what is being found, and Cancel
/// (widgets `canvas:aiProgress`, `canvas:aiCancel`). True when Cancel was clicked.
fn ai_progress(ui: &mut egui::Ui, over: Rect, what: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(over);
    let g = p.layout_no_wrap(what.to_string(), t.font(13.0), Color32::WHITE);
    let cancel_w = 64.0;
    let size = vec2(34.0 + g.size().x + 12.0 + cancel_w + 10.0, 36.0);
    let bg = Rect::from_center_size(over.center(), size);
    p.rect_filled(bg, 8.0, Color32::from_black_alpha(185));
    // the spinner: an arc going round
    let c = pos2(bg.left() + 18.0, bg.center().y);
    let time = ui.input(|i| i.time) as f32;
    let a0 = time * 5.0;
    let arc: Vec<Pos2> = (0..=20).map(|k| c + egui::Vec2::angled(a0 + k as f32 / 20.0 * 4.4) * 7.0).collect();
    p.add(egui::Shape::line(arc, Stroke::new(2.0, Color32::WHITE)));
    p.galley(pos2(bg.left() + 34.0, bg.center().y - g.size().y / 2.0), g, Color32::WHITE);
    register(ui.ctx(), "canvas:aiProgress", bg);
    let br = Rect::from_min_size(pos2(bg.right() - cancel_w - 8.0, bg.top() + 6.0), vec2(cancel_w, bg.height() - 12.0));
    let b = ui.interact(br, egui::Id::new("canvas-ai-cancel"), Sense::click());
    crate::widgets::announce(&b, crate::i18n::tr("Cancel"));
    register(ui.ctx(), "canvas:aiCancel", br);
    p.rect_filled(br, 4.0, if b.hovered() { Color32::from_white_alpha(60) } else { Color32::from_white_alpha(30) });
    p.text(br.center(), Align2::CENTER_CENTER, crate::i18n::tr("Cancel"), t.font(12.5), Color32::WHITE);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    b.clicked()
}

fn quick_name(q: lightcraft_engine::media::QuickSource) -> &'static str {
    use lightcraft_engine::media::QuickSource;
    match q {
        QuickSource::Cached => "cached",
        QuickSource::Embedded => "embedded",
        QuickSource::Small => "small",
    }
}

/// Targeted adjustment tool: dragging up/down on the photo raises/lowers the tone-curve region or
/// the colour-mixer bands under the press point (`develop.targeted`, one call per whole step, all
/// in one interaction = one undo step).
fn targeted_drag(app: &mut LightcraftApp, resp: &egui::Response, map: &CanvasMap, target: &str) {
    if resp.drag_started()
        && let Some(q) = resp.interact_pointer_pos()
    {
        let _ = app.run("develop.beginInteraction", json!({"label": "Targeted Adjustment"}));
        app.gesture = Some(Gesture::Targeted { at: map.norm(q), acc: 0.0 });
    }
    if resp.dragged()
        && let Some(Gesture::Targeted { at, acc }) = &mut app.gesture
    {
        *acc -= resp.drag_delta().y * 0.5;
        let step = acc.trunc();
        if step != 0.0 {
            *acc -= step;
            let at = *at;
            let mut p = json!({"target": target, "x": at.x, "y": at.y, "delta": step});
            if target == "curve" && app.ui.curve_channel != "parametric" {
                p["channel"] = json!(app.ui.curve_channel);
            }
            let _ = app.run("develop.targeted", p);
        }
    }
    if resp.drag_stopped() && matches!(app.gesture, Some(Gesture::Targeted { .. })) {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
}

/// Whether a slider or a dial is adjusting the photo right now. Then the mask overlay, the pins and
/// the tool graphics (masks, spots, crop guides) hide so the effect itself can be judged, even an
/// overlay switched on with O; they come back as they were when the adjusting stops (the user's
/// standard behaviour). Sliders that shape the tool's own graphics (a range's Refine, a spot's
/// Size, the crop angle, a transform with its fine grid) keep them.
pub(crate) fn adjusting(app: &LightcraftApp) -> bool {
    let slider = app.ui.dragging_control.as_deref().is_some_and(|c| !shapes_graphics(c));
    slider || app.surface.adjusting()
}

/// Slider ids that change what the tool graphics show rather than the photo's look.
fn shapes_graphics(id: &str) -> bool {
    matches!(id, "refine" | "smoothness" | "edge" | "feather" | "ui.removeSize" | "ui.removeFeather" | "crop.angle")
        || id.starts_with("geometry.")
        || id.starts_with("crop.")
}

/// Mask tools that set a mask on the photo: the overlay shows while one is in use.
const MASK_TOOLS: [&str; 6] = ["brush", "linear", "radial", "colorRange", "object", "lumRange"];

/// The mask the overlay shows, if any (as Lightroom Classic): the one whose row or pin the pointer
/// is on; else the selected mask when the overlay is on (O), when it is fresh (no adjustment yet,
/// unless O hid it) or while a mask tool is in use. A preset's or an adjusted mask never shows by
/// itself, so the effect can be judged.
pub(crate) fn overlay_mask<'a>(app: &LightcraftApp, d: &'a DevelopSettings) -> Option<&'a lightcraft_develop::Mask> {
    if let Some(m) = app.ui.mask_hover.or(app.ui.mask_hover_prev).and_then(|id| d.masks.iter().find(|m| m.id == id)) {
        return Some(m);
    }
    let m = app.session.active_mask.and_then(|id| d.masks.iter().find(|m| m.id == id))?;
    overlay_shown(app, m).then_some(m)
}

/// Whether the selected mask `m`'s overlay shows (without hovering).
pub(crate) fn overlay_shown(app: &LightcraftApp, m: &lightcraft_develop::Mask) -> bool {
    if app.ui.mask_overlay {
        return true;
    }
    if app.ui.mask_overlay_hidden == Some(m.id) {
        return false;
    }
    let fresh = m.adjust == lightcraft_develop::LocalAdjustments::default() && m.preset.is_none();
    fresh || MASK_TOOLS.contains(&app.ui.tool.as_str())
}

/// The diagnostic overlay the loupe shows (the selected mask, Point Color's visualized range,
/// Visualize Spots).
pub(crate) fn view_overlay(app: &LightcraftApp, d: &DevelopSettings) -> lightcraft_pipeline::Overlay {
    use lightcraft_pipeline::{MaskView, Overlay};
    if app.ui.fullscreen {
        return Overlay::None;
    }
    // a slider or dial at work: the effect shows, not the overlay (it returns afterwards)
    let shown = !adjusting(app);
    // Show every visible mask's region at once (Ctrl+Shift+O / Cmd+Shift+O), ahead of the selected-mask overlay
    // so the shortcut is not shadowed by the always-on single-mask overlay.
    if shown && app.ui.right == RightPanel::Masking && app.ui.mask_overlay_all && d.masks.iter().any(|m| m.visible && !m.components.is_empty()) {
        return Overlay::AllMasks {
            view: MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default(),
            color: app.ui.mask_overlay_color,
            opacity: app.ui.mask_overlay_opacity.clamp(10.0, 100.0).round() as u8,
        };
    }
    if shown
        && app.ui.right == RightPanel::Masking
        && let Some(m) = overlay_mask(app, d)
        && !m.components.is_empty()
        && ai_computing(app, m).is_none()
    {
        return Overlay::Mask {
            id: m.id.min(u16::MAX as u32) as u16,
            view: MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default(),
            color: app.ui.mask_overlay_color,
            opacity: app.ui.mask_overlay_opacity.clamp(10.0, 100.0).round() as u8,
        };
    }
    if shown && app.ui.right == RightPanel::Remove && app.ui.visualize_spots {
        return Overlay::Spots(app.ui.spots_threshold.clamp(0.0, 100.0).round() as u8);
    }
    let edit = app.ui.right == RightPanel::Edit;
    if edit && app.ui.point_color_visualize && app.ui.flyout_open("pointColor") && app.ui.point_color < d.point_colors.len() {
        return Overlay::PointColorRange(app.ui.point_color as u8);
    }
    if edit && app.ui.hdr_visualize && d.hdr.enabled {
        return Overlay::HdrRange;
    }
    Overlay::None
}

fn clipping_overlay(app: &LightcraftApp, p: &egui::Painter, r: Rect) {
    // Highlight clipped regions using the histogram's extremes isn't spatial; show a subtle frame hint.
    if let Some(h) = app.renderer.textures.get(&Slot::Main).and_then(|t| t.histogram.as_ref()) {
        let (lo, hi) = h.clipping();
        if hi > 0.003 {
            p.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 60, 60)), StrokeKind::Outside);
        }
        if lo > 0.003 {
            p.rect_stroke(r.expand(3.0), 0.0, Stroke::new(2.0, Color32::from_rgb(60, 120, 255)), StrokeKind::Outside);
        }
    }
}

fn general_cursor(app: &LightcraftApp, ui: &egui::Ui, resp: &egui::Response, img: Rect, canvas: Rect) {
    // A panel or overlay under the pointer keeps its own cursor.
    if !resp.dragged() && !resp.hovered() {
        return;
    }
    let cursor = match app.ui.tool.as_str() {
        "wbPicker" | "colorRange" | "pointColor" => egui::CursorIcon::Crosshair,
        tool if tool.starts_with("tat:") => egui::CursorIcon::ResizeVertical,
        _ if img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0 => {
            if resp.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            }
        }
        _ => return,
    };
    ui.ctx().set_cursor_icon(cursor);
}

fn general_interaction(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    map: &CanvasMap,
    img: Rect,
    canvas: Rect,
    native: [usize; 2],
    aspect: f32,
) {
    if app.ui.tool == "wbPicker" {
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let _ = app.run("develop.wbPick", json!({"x": n.x, "y": n.y}));
            app.ui.tool.clear();
        }
        return;
    }
    if let Some(target) = app.ui.tool.strip_prefix("tat:").map(str::to_string) {
        targeted_drag(app, resp, map, &target);
        return;
    }
    if app.ui.tool == "colorRange" {
        color_range_pick(app, ui, resp, map);
        return;
    }
    if app.ui.tool == "pointColor" {
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            if let Ok(r) = app.run("pointColor.pick", json!({"x": n.x, "y": n.y})) {
                app.ui.point_color = r["index"].as_u64().unwrap_or(0) as usize;
            }
            app.ui.tool.clear();
        }
        return;
    }
    // click toggles Fit ↔ the chosen click-zoom ratio at the clicked point; drag pans when zoomed
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    if resp.double_clicked() || (resp.clicked() && !zoomed) {
        if let Some(q) = resp.interact_pointer_pos() {
            let u = ((q.x - img.left()) / img.width()).clamp(0.0, 1.0);
            let v = ((q.y - img.top()) / img.height()).clamp(0.0, 1.0);
            app.ui.pan = (u, v);
        }
        app.ui.zoom = if matches!(app.ui.zoom, Zoom::Fit) { Zoom::Percent(app.ui.click_zoom as f32) } else { Zoom::Fit };
        app.ui.zoom_anim = true;
    } else if resp.clicked() && zoomed {
        app.ui.zoom = Zoom::Fit;
        app.ui.zoom_anim = true;
    }
    if zoomed && resp.dragged() {
        pan_image(app, canvas, img, resp.drag_delta());
    }
    let _ = (native, aspect);
}

// ------------------------------------------------------------------------ crop

fn crop_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, frame: &Frame, d: &DevelopSettings, id: PhotoId) {
    if app.ui.tool == "guidedUpright" {
        guided_overlay(app, ui, resp, map, frame, d);
        return;
    }
    if app.ui.tool == "straighten" {
        straighten_overlay(app, ui, resp, false);
        return;
    }
    // hold ⌘ and drag: draw a straighten line without leaving the crop tool
    let cmd_drag = resp.drag_started() && ui.input(|i| i.modifiers.command);
    if cmd_drag || matches!(app.gesture, Some(Gesture::StraightenLine { .. })) {
        straighten_overlay(app, ui, resp, true);
        return;
    }
    if resp.hovered() && ui.input(|i| i.modifiers.command) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    // the crop as it will be: with Constrain Crop, inside the photo's valid area (no empty
    // wedges after Upright or a lens correction), so the box here is the crop applied
    let shown = shown_crop(d, frame);
    let d = &DevelopSettings { crop: lightcraft_develop::Crop { geometry: shown, ..d.crop }, ..d.clone() };
    let quad = frame_crop_quad(d, frame);
    let pts: Vec<Pos2> = quad.iter().map(|q| map.screen(*q)).collect();
    let p = ui.painter();
    // darken outside the crop (the box's corners in screen order: a flipped photo shows its
    // top-left corner elsewhere), over the whole canvas: the photo may be turned
    let quad = flipped_corners(&pts, frame.flip_h, frame.flip_v);
    if pts.len() == 4 {
        p.add(outside_crop_mesh(app.canvas_rect.unwrap_or(map.rect).union(map.rect), &quad, Color32::from_black_alpha(150)));
    }
    p.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
    // overlay guides and handles (hidden while a slider or dial adjusts the photo)
    let mut p = p.clone();
    if adjusting(app) {
        p.set_invisible();
    }
    let lerp = |a: Pos2, b: Pos2, t: f32| a + (b - a) * t;
    let guide = Stroke::new(0.8, Color32::from_white_alpha(150));
    // (u, v) across / down the (rotated) crop frame
    let at = |(u, v): (f32, f32)| pts[0] + (pts[1] - pts[0]) * u + (pts[3] - pts[0]) * v;
    let (cw, ch) = (pts[0].distance(pts[1]), pts[0].distance(pts[3]));
    for line in super::crop_overlay::paths(app.ui.crop_overlay, app.ui.crop_overlay_orient, cw, ch) {
        p.add(egui::Shape::line(line.into_iter().map(at).collect(), guide));
    }
    // turning the photo: a fine grid in the box, to line things up with
    if matches!(app.gesture, Some(Gesture::CropRotate { .. })) {
        let fine = Stroke::new(0.6, Color32::from_white_alpha(70));
        for k in 1..12 {
            let u = k as f32 / 12.0;
            p.line_segment([at((u, 0.0)), at((u, 1.0))], fine);
            p.line_segment([at((0.0, u)), at((1.0, u))], fine);
        }
    }
    // handles: 0..3 corners, 4..7 edges (top, right, bottom, left)
    let handles: Vec<Pos2> = (0..8).map(|i| if i < 4 { pts[i] } else { lerp(pts[i - 4], pts[(i - 3) % 4], 0.5) }).collect();
    for (i, h) in handles.iter().enumerate() {
        register(ui.ctx(), format!("cropHandle:{i}"), Rect::from_center_size(*h, vec2(14.0, 14.0)));
        let s = if i < 4 { 12.0 } else { 9.0 };
        p.rect_filled(Rect::from_center_size(*h, vec2(s, 3.0)), 0.0, Color32::WHITE);
        p.rect_filled(Rect::from_center_size(*h, vec2(3.0, s)), 0.0, Color32::WHITE);
    }
    // the keyboard: each handle (and the box itself) a stand-in Tab reaches and the arrows move
    if crop_keys(app, ui, map, &handles, &pts, d, frame) {
        return;
    }
    let inside = |q: Pos2| {
        let n = map.norm(q);
        let s = to_straight(n, d.crop.geometry.angle, frame);
        d.crop.geometry.rect.contains(Point::new(s.x.clamp(-1.0, 2.0), s.y))
    };
    // a crop drag cut short (the tool closed before the release) leaves its gesture behind: with no
    // button held it is over
    if matches!(app.gesture, Some(Gesture::CropRotate { .. } | Gesture::CropHandle { .. }))
        && !resp.drag_stopped()
        && !ui.input(|i| i.pointer.any_down())
    {
        app.gesture = None;
    }
    // while rotating, the pointer may leave the canvas: it still shows rotation and the angle
    let rotating = matches!(app.gesture, Some(Gesture::CropRotate { .. }));
    let pointer = resp.hover_pos().or_else(|| if rotating { ui.input(|i| i.pointer.latest_pos()) } else { None });
    // with ⌘ held a drag draws a level line (the straighten crosshair, set above), not a rotation
    let straightening = !rotating && ui.input(|i| i.modifiers.command);
    if let Some(hq) = pointer.filter(|_| !straightening) {
        // a move or resize drag keeps its pointer wherever it goes, even past the box
        let held = match app.gesture {
            Some(Gesture::CropHandle { handle, .. }) => Some(usize::from(handle)),
            _ => None,
        };
        let near = held.or_else(|| handles.iter().position(|h| h.distance(hq) < 12.0)).filter(|_| !rotating);
        let cursor = match near {
            Some(0 | 2) => egui::CursorIcon::ResizeNwSe,
            Some(1 | 3) => egui::CursorIcon::ResizeNeSw,
            Some(4 | 6) => egui::CursorIcon::ResizeVertical,
            Some(5 | 7) => egui::CursorIcon::ResizeHorizontal,
            // 8: the whole box
            Some(_) => egui::CursorIcon::Move,
            None if inside(hq) && !rotating => egui::CursorIcon::Move,
            // a drag here rotates: no system cursor shows that, so draw a curved double arrow
            None => {
                rotate_cursor(ui, hq, rotate_cursor_turn(&pts, hq, inside(hq)));
                egui::CursorIcon::None
            }
        };
        ui.ctx().set_cursor_icon(cursor);
    }
    // the angle next to the pointer, drawn once this frame's rotation is applied (below)
    let readout_at = pointer.filter(|_| rotating);
    let mut shown_angle = d.crop.geometry.angle;
    // double-click inside the crop box applies the crop (same as Return / Done)
    if resp.double_clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && inside(q)
        && !handles.iter().any(|h| h.distance(q) < 12.0)
    {
        let _ = app.run("tool.done", json!({}));
        return;
    }
    if resp.drag_started()
        && let Some(q) = resp.interact_pointer_pos()
    {
        if let Err(e) = app.run("develop.beginInteraction", json!({"label": "Crop"})).and_then(|_| app.begin_crop().map(|_| serde_json::Value::Null))
        {
            app.ui.status = e;
            let _ = app.session.cancel_interaction();
            return;
        }
        // (from the crop as shown: a constrained box starts where it is drawn)
        let g = shown_crop(&app.session.develop_of(id).unwrap_or_default(), frame);
        app.gesture = Some(match handles.iter().position(|h| h.distance(q) < 12.0) {
            Some(h) => Gesture::CropHandle { handle: h as u8, start: g.rect, angle: g.angle },
            None if inside(q) => Gesture::CropHandle { handle: 8, start: g.rect, angle: g.angle },
            None => {
                let c = map.screen(Point::new(0.5, 0.5));
                Gesture::CropRotate { start_angle: g.angle, a0: (q - c).angle() }
            }
        });
    }
    if resp.dragged()
        && let Some(q) = resp.interact_pointer_pos()
    {
        match app.gesture.clone() {
            Some(Gesture::CropHandle { handle, start, angle }) => {
                // The crop model decides what the drag does (anchor, aspect lock, image bounds): see
                // `lightcraft_geom::drag_crop`; the UI only reports where the pointer started and is.
                let n = to_straight(map.norm(q), angle, frame);
                let orig = ui.input(|i| i.pointer.press_origin()).map(|q0| to_straight(map.norm(q0), angle, frame)).unwrap_or(n);
                // Shift keeps the current proportions for this drag only; the Lock toggle is untouched.
                let keep_ratio = ui.input(|i| i.modifiers.shift);
                let _ = app.run(
                    "crop.drag",
                    json!({"handle": handle, "from": [orig.x, orig.y], "to": [n.x, n.y], "start": [start.x0, start.y0, start.x1, start.y1], "keepRatio": keep_ratio}),
                );
            }
            Some(Gesture::CropRotate { start_angle, a0 }) => {
                // the photo turns with the pointer under the level box: the box turns the other
                // way on the photo (measured from where the drag began, as the view turns along)
                let c = map.rect.center();
                let a = (q - c).angle();
                let turn = (a - a0).to_degrees() as f64;
                let turn = if turn > 180.0 {
                    turn - 360.0
                } else if turn < -180.0 {
                    turn + 360.0
                } else {
                    turn
                };
                let ang = (start_angle - turn).clamp(-45.0, 45.0);
                shown_angle = (ang * 100.0).round() / 100.0;
                let _ = app.run("crop.straighten", json!({"angle": shown_angle}));
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
    if let Some(at) = readout_at {
        // the canvas (the loupe's own rect), not the photo: rotating happens in the margin around it
        angle_readout(ui, at, shown_angle, resp.rect);
    }
    let _ = id;
}

/// The crop handles' names for screen readers, in `crop.drag`'s numbering: 0..3 corners, 4..7
/// edges (top, right, bottom, left), 8 the whole box.
const CROP_HANDLE_NAMES: [&str; 9] = [
    "Crop top-left corner",
    "Crop top-right corner",
    "Crop bottom-right corner",
    "Crop bottom-left corner",
    "Crop top edge",
    "Crop right edge",
    "Crop bottom edge",
    "Crop left edge",
    "Crop box",
];

/// Keyboard stand-ins for the crop handles: an arrow press drags the focused one by a step (one
/// undo step each), as a pointer drag from its place would. Return applies the crop. True when the
/// frame is over (the crop was applied).
fn crop_keys(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    map: &CanvasMap,
    handles: &[Pos2],
    pts: &[Pos2],
    d: &DevelopSettings,
    frame: &Frame,
) -> bool {
    let r = d.crop.geometry.rect;
    let centre = if pts.len() == 4 { pts.iter().fold(Pos2::ZERO, |acc, q| acc + q.to_vec2() / 4.0) } else { return false };
    let pct = |v: f64| if v.is_finite() { (v * 100.0).round() as i64 } else { 0 };
    let value = format!("left {}%, top {}%, right {}%, bottom {}%", pct(r.x0), pct(r.y0), pct(r.x1), pct(r.y1));
    for (i, name) in CROP_HANDLE_NAMES.iter().enumerate() {
        let Some(at) = handles.get(i).copied().or_else(|| (i == 8).then_some(centre)) else { continue };
        let keys = crate::canvas_keys::handle(ui, &format!("crop:{i}"), crate::i18n::tr(name), value.clone(), at);
        if keys.activated {
            let _ = app.run("tool.done", json!({}));
            return true;
        }
        let Some((dx, dy)) = keys.delta else { continue };
        // the drag runs in the straightened crop frame, as a pointer drag from the handle would;
        // the arrows follow the screen
        let angle = d.crop.geometry.angle;
        let to_q = at + vec2(dx as f32 * map.rect.width(), dy as f32 * map.rect.height());
        let (from, to) = (to_straight(map.norm(at), angle, frame), to_straight(map.norm(to_q), angle, frame));
        let started = app.run("develop.beginInteraction", json!({"label": "Crop"})).and_then(|_| app.begin_crop().map(|_| serde_json::Value::Null));
        if let Err(e) = started {
            app.ui.status = e;
            let _ = app.session.cancel_interaction();
            return false;
        }
        let _ = app.run(
            "crop.drag",
            json!({"handle": i, "from": [from.x, from.y], "to": [to.x, to.y], "start": [r.x0, r.y0, r.x1, r.y1], "keepRatio": false}),
        );
        let _ = app.run("develop.endInteraction", json!({}));
    }
    false
}

/// The crop angle as the rotation readout shows it: like the Straighten value, in degrees.
pub(crate) fn crop_angle_label(angle: f64) -> String {
    let shown =
        lightcraft_develop::controls::find("crop.angle").map_or_else(|| format!("{angle:.2}"), |spec| crate::widgets::shown_value(spec, angle));
    // the slider's format turns a rounded −0.00 into "0": every zero reads "0.00", the angle's
    // usual two decimals
    let shown = if shown == "0" { "0.00".to_string() } else { shown };
    format!("{shown}°")
}

/// Where the angle readout of `size` goes for a pointer `at`: below right of it, flipped to the
/// left / above where that would leave `bounds`, then kept inside them.
pub(crate) fn readout_rect(at: Pos2, size: egui::Vec2, bounds: Rect) -> Rect {
    // without a real pointer or bounds (NaN, an empty rect) there is nothing to keep it in; and
    // f32::clamp panics on NaN, so only finite numbers get there
    if !at.is_finite() || !bounds.is_finite() || !bounds.is_positive() {
        let at = if at.is_finite() { at } else { bounds.min.max(Pos2::ZERO) };
        let at = if at.is_finite() { at } else { Pos2::ZERO };
        return Rect::from_min_size(at + vec2(18.0, 14.0), size);
    }
    let x = if at.x + 18.0 + size.x > bounds.right() { at.x - 18.0 - size.x } else { at.x + 18.0 };
    let y = if at.y + 14.0 + size.y > bounds.bottom() { at.y - 14.0 - size.y } else { at.y + 14.0 };
    let x = x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()));
    let y = y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    Rect::from_min_size(pos2(x, y), size)
}

/// On top of everything (the tooltip layer), so neither the photo nor a panel covers it.
fn top_painter(ui: &egui::Ui, name: &'static str) -> egui::Painter {
    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(name)))
}

/// The rotation pointer: a curved double arrow on a dark disc, so it reads over any photo,
/// turned `turn` radians clockwise ([`rotate_cursor_turn`]).
fn rotate_cursor(ui: &egui::Ui, at: Pos2, turn: f32) {
    let p = top_painter(ui, "crop-rotate-cursor");
    let r = Rect::from_center_size(at, vec2(22.0, 22.0));
    p.circle_filled(at, 12.0, Color32::from_black_alpha(150));
    crate::icons::paint_turned(&p, r.shrink(2.0), crate::icons::Icon::RotateDrag, Color32::WHITE, turn);
    register(ui.ctx(), "cropRotateCursor", r);
}

/// How far to turn the rotation pointer (radians, clockwise) so its arc, drawn bulging up, bulges
/// away from the crop box `pts` (its four corners on screen, maybe rotated): up above the top edge,
/// right beside the right edge, down below it, diagonally off a corner. Measured from the nearest
/// point of the box's outline; `inside` (a rotate drag that has come back over the box) faces it
/// the other way, still away from that edge.
fn rotate_cursor_turn(pts: &[Pos2], q: Pos2, inside: bool) -> f32 {
    let nearest = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let ab = b - a;
            let t = if ab.length_sq() > 0.0 { ((q - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
            a + ab * t
        })
        .min_by(|a, b| a.distance_sq(q).total_cmp(&b.distance_sq(q)));
    let Some(nearest) = nearest else { return 0.0 };
    let away = if inside { nearest - q } else { q - nearest };
    if away.length() < 0.5 {
        return 0.0;
    }
    away.y.atan2(away.x) + std::f32::consts::FRAC_PI_2
}

/// While rotating: the angle, next to the pointer, kept inside `bounds` (the canvas): on the
/// pointer's other side when it would run past an edge.
fn angle_readout(ui: &egui::Ui, at: Pos2, angle: f64, bounds: Rect) {
    let t = Tokens::get(ui.ctx());
    let p = top_painter(ui, "crop-angle-readout");
    let galley = p.layout_no_wrap(crop_angle_label(angle), t.font(12.5), Color32::WHITE);
    let rect = readout_rect(at, galley.size() + vec2(12.0, 6.0), bounds);
    p.rect_filled(rect, 4.0, Color32::from_black_alpha(170));
    p.galley(rect.min + vec2(6.0, 3.0), galley, Color32::WHITE);
    register(ui.ctx(), "cropAngleReadout", rect);
}

/// Guided Upright: draw up to four guides along lines that should be vertical or horizontal. Guides are
/// stored in lens-corrected (pre-perspective) coordinates, so they stay attached to the image as it warps.
fn guided_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, frame: &Frame, d: &DevelopSettings) {
    let p = ui.painter_at(map.rect.expand(8.0));
    let col = Color32::from_rgb(255, 196, 40);
    let draw = |a: Pos2, b: Pos2| {
        p.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(120)));
        p.line_segment([a, b], Stroke::new(1.5, col));
        for q in [a, b] {
            p.circle_filled(q, 4.0, col);
            p.circle_stroke(q, 4.0, Stroke::new(1.0, Color32::BLACK));
        }
    };
    for (i, (a, b)) in d.geometry.guides.iter().enumerate() {
        let (sa, sb) = (map.screen(frame.corrected_to_transformed(*a)), map.screen(frame.corrected_to_transformed(*b)));
        register(ui.ctx(), format!("uprightGuide:{i}"), Rect::from_two_pos(sa, sb));
        draw(sa, sb);
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if resp.drag_started()
        && let Some(q) = resp.interact_pointer_pos()
    {
        app.gesture = Some(Gesture::Guide { a: map.norm(q) });
    }
    if let (Some(Gesture::Guide { a }), Some(q)) = (&app.gesture, resp.interact_pointer_pos()) {
        draw(map.screen(*a), q);
    }
    if resp.drag_stopped()
        && let (Some(Gesture::Guide { a }), Some(q)) = (app.gesture.clone(), resp.interact_pointer_pos())
    {
        app.gesture = None;
        let b = map.norm(q);
        if a.dist(b) > 0.02 {
            let (ca, cb) = (frame.transformed_to_corrected(a), frame.transformed_to_corrected(b));
            let _ = app.run("geometry.guides", json!({"guides": [[ca.x, ca.y, cb.x, cb.y]], "add": true}));
        }
    }
}

/// Normalized oriented coords → the straightened (rotated) frame the crop rect lives in.
fn to_straight(n: Point, angle: f64, frame: &Frame) -> Point {
    let (w, h) = (frame.ow, frame.oh);
    let px = Point::new(n.x * w, n.y * h);
    let r = Affine::rotate_about(angle.to_radians(), Point::new(w / 2.0, h / 2.0)).apply(px);
    Point::new(r.x / w, r.y / h)
}

/// A crop box's corners (top-left, top-right, bottom-right, bottom-left of the unflipped photo)
/// in the order they appear on screen when the photo is shown flipped.
fn flipped_corners(pts: &[Pos2], flip_h: bool, flip_v: bool) -> [Pos2; 4] {
    let at = |i: usize| pts.get(i).copied().unwrap_or(Pos2::ZERO);
    let order = match (flip_h, flip_v) {
        (false, false) => [0, 1, 2, 3],
        (true, false) => [1, 0, 3, 2],
        (false, true) => [3, 2, 1, 0],
        (true, true) => [2, 3, 0, 1],
    };
    order.map(at)
}

/// The dimming outside the crop box: the photo's rectangle minus the crop quad, as one mesh of
/// non-overlapping convex pieces. The quad is a rotated rectangle, so its opposite sides are
/// parallel and the plane outside it splits into four disjoint half-strips: outside edge `i`
/// and inside edge `i + 1`. Each is clipped to the photo and fanned into triangles.
///
/// Issue #731: joining photo corner `i` to crop corner `i` with a quad per side only works while
/// the two rectangles are aligned. Rotated, those quads cross themselves and each other, which
/// showed as dimmed wedges inside the crop and doubly dark ones outside, growing with the angle.
/// A degenerate quad (a point, a line, a non-finite corner) dims nothing.
fn outside_crop_mesh(img: Rect, quad: &[Pos2; 4], color: Color32) -> egui::epaint::Mesh {
    let mut mesh = egui::epaint::Mesh::default();
    if !quad.iter().all(|q| q.x.is_finite() && q.y.is_finite()) || !img.is_finite() {
        return mesh;
    }
    let centre = pos2((quad[0].x + quad[1].x + quad[2].x + quad[3].x) / 4.0, (quad[0].y + quad[1].y + quad[2].y + quad[3].y) / 4.0);
    // signed distance from edge i's line, positive towards the crop's centre
    let side = |i: usize, q: Pos2| -> Option<f32> {
        let (a, b) = (quad[i % 4], quad[(i + 1) % 4]);
        let n = vec2(a.y - b.y, b.x - a.x);
        let orient = n.dot(centre - a);
        // an edge the centre lies on: the box is flat
        (orient.abs() > 1e-6 * n.length().max(1.0)).then(|| n.dot(q - a) * orient.signum())
    };
    // Sutherland–Hodgman: the part of a convex polygon where `keep` is at or above zero
    let clip = |poly: &[Pos2], keep: &dyn Fn(Pos2) -> Option<f32>| -> Option<Vec<Pos2>> {
        let mut out = Vec::with_capacity(poly.len() + 2);
        for (k, &a) in poly.iter().enumerate() {
            let b = *poly.get((k + 1) % poly.len())?;
            let (da, db) = (keep(a)?, keep(b)?);
            if da >= 0.0 {
                out.push(a);
            }
            if (da < 0.0) != (db < 0.0) && (da - db).abs() > f32::EPSILON {
                let t = da / (da - db);
                out.push(a + (b - a) * t);
            }
        }
        Some(out)
    };
    let photo = [img.left_top(), img.right_top(), img.right_bottom(), img.left_bottom()];
    for i in 0..4 {
        let outside = |q: Pos2| side(i, q).map(|d| -d);
        let inside_next = |q: Pos2| side(i + 1, q);
        let Some(strip) = clip(&photo, &outside).and_then(|poly| clip(&poly, &inside_next)) else { return egui::epaint::Mesh::default() };
        if strip.len() < 3 {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend(strip.iter().map(|&pos| egui::epaint::Vertex { pos, uv: Pos2::ZERO, color }));
        for k in 1..(strip.len() as u32).saturating_sub(1) {
            mesh.indices.extend([base, base + k, base + k + 1]);
        }
    }
    mesh
}

/// The crop the photo gets: with Constrain Crop, the box shrunk into the warped photo's valid area
/// (what the render does), else the box as stored.
fn shown_crop(d: &DevelopSettings, frame: &Frame) -> lightcraft_geom::CropGeometry {
    if d.geometry.constrain_crop { frame.constrained_crop(d.crop.geometry) } else { d.crop.geometry }
}

fn frame_crop_quad(d: &DevelopSettings, frame: &Frame) -> [Point; 4] {
    let f = Frame { crop: d.crop.geometry, ..frame.clone() };
    f.crop_quad_norm()
}

// ------------------------------------------------------------------------ masks

/// Where a mask component's pin sits (normalized), for shapes that have a place on the photo.
pub(crate) fn component_pin(shape: &MaskShape) -> Option<Point> {
    match shape {
        MaskShape::Radial { center, .. } => Some(*center),
        MaskShape::Linear { start, end } => Some(Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0)),
        MaskShape::Brush { strokes } => strokes.iter().find(|s| !s.erase).and_then(|s| s.points.first()).copied(),
        _ => None,
    }
}

/// `shape` dragged from `at - dn` to `at` (normalized) by `handle`: 0 moves it; a radial
/// gradient's 1..4 resize it, 5 and [`ROTATE`] turn it, 6 sets the feather (its inner ring), 9
/// draws a new one; a linear gradient's 1 / 2 set its start / end, [`ROTATE`] turns it about its
/// middle, [`LINE_START`] / [`LINE_END`] slide the start / end line along it.
fn moved_shape(shape: &MaskShape, handle: u8, dn: Point, at: Point, map: &CanvasMap, alt: bool) -> MaskShape {
    let mv = |p: Point| Point::new(p.x + dn.x, p.y + dn.y);
    let press = Point::new(at.x - dn.x, at.y - dn.y);
    match shape.clone() {
        MaskShape::Radial { center, rx, ry, angle, feather, invert } => match handle {
            1 | 3 => {
                let l = frame_long_norm(map);
                let dx = (at.x - center.x) / l.0;
                let dy = (at.y - center.y) / l.1;
                let (s, co) = angle.to_radians().sin_cos();
                let new_rx = (dx * co + dy * s).abs().max(0.01);
                let mut new_ry = ry;
                if alt {
                    new_ry = ry * (new_rx / rx.max(0.01));
                }
                if (new_rx - new_ry).abs() / new_rx.max(new_ry) < 0.05 {
                    new_ry = new_rx;
                }
                MaskShape::Radial { center, rx: new_rx, ry: new_ry, angle, feather, invert }
            }
            2 | 4 => {
                let l = frame_long_norm(map);
                let dx = (at.x - center.x) / l.0;
                let dy = (at.y - center.y) / l.1;
                let (s, co) = angle.to_radians().sin_cos();
                let new_ry = (dx * (-s) + dy * co).abs().max(0.01);
                let mut new_rx = rx;
                if alt {
                    new_rx = rx * (new_ry / ry.max(0.01));
                }
                if (new_rx - new_ry).abs() / new_rx.max(new_ry) < 0.05 {
                    new_rx = new_ry;
                }
                MaskShape::Radial { center, rx: new_rx, ry: new_ry, angle, feather, invert }
            }
            // drawing a new ellipse: both radii follow the pointer
            9 => {
                let l = frame_long_norm(map);
                let rx = ((at.x - center.x) / l.0).abs().max(0.01);
                let ry = ((at.y - center.y) / l.1).abs().max(0.01);
                MaskShape::Radial { center, rx, ry, angle, feather, invert }
            }
            // turning: by as much as the pointer went round the centre (on screen)
            5 | ROTATE => {
                let turn = screen_angle(map, at, center) - screen_angle(map, press, center);
                let mut new_angle = angle + f64::from(turn).to_degrees();
                if !new_angle.is_finite() {
                    new_angle = angle;
                }
                MaskShape::Radial { center, rx, ry, angle: (new_angle + 180.0).rem_euclid(360.0) - 180.0, feather, invert }
            }
            // the feather ring: inside it the effect is full, so its size sets the feather
            6 => {
                let r = radial_r(shape, at, map).unwrap_or(1.0 - feather / 100.0);
                let feather = ((1.0 - r) * 100.0).clamp(0.0, 100.0).round();
                MaskShape::Radial { center, rx, ry, angle, feather, invert }
            }
            _ => MaskShape::Radial { center: mv(center), rx, ry, angle, feather, invert },
        },
        MaskShape::Linear { start, end } => match handle {
            1 => MaskShape::Linear { start: at, end },
            2 => MaskShape::Linear { start, end: at },
            ROTATE => {
                let mid = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
                let turn = screen_angle(map, at, mid) - screen_angle(map, press, mid);
                if !turn.is_finite() {
                    return MaskShape::Linear { start, end };
                }
                MaskShape::Linear { start: turned(map, start, mid, turn), end: turned(map, end, mid, turn) }
            }
            LINE_START | LINE_END => {
                // only along the gradient: the line stays parallel, the fade gets wider or narrower
                let (a, b) = (map.screen(start), map.screen(end));
                let len = a.distance(b);
                if len.is_nan() || len < 1.0 {
                    return MaskShape::Linear { start, end };
                }
                let dir = (b - a) / len;
                let shift = (map.screen(at) - map.screen(press)).dot(dir);
                if handle == LINE_START {
                    MaskShape::Linear { start: map.norm(a + dir * shift), end }
                } else {
                    MaskShape::Linear { start, end: map.norm(b + dir * shift) }
                }
            }
            _ => MaskShape::Linear { start: mv(start), end: mv(end) },
        },
        MaskShape::Brush { mut strokes } => {
            for st in &mut strokes {
                st.points.iter_mut().for_each(|p| *p = mv(*p));
            }
            MaskShape::Brush { strokes }
        }
        s => s,
    }
}

/// Where the radial gradient's feather handle sits on its inner ring (degrees, in the ellipse's own
/// frame: up and to the right of the centre).
const FEATHER_HANDLE_DEG: f64 = -45.0;

/// The inner (feather) ring's size relative to the outer edge: the effect is full inside it and
/// fades to nothing at the edge (`feather` 0..100, as the pipeline draws it).
fn radial_inner(feather: f64) -> f64 {
    if feather.is_finite() { (1.0 - feather / 100.0).clamp(0.0, 1.0) } else { 0.5 }
}

/// The point at `deg` on the ellipse `scale` times the radial gradient's, on screen.
fn radial_point(map: &CanvasMap, center: Point, rx: f64, ry: f64, angle: f64, scale: f64, deg: f64) -> Pos2 {
    let l = frame_long_norm(map);
    let (s, co) = angle.to_radians().sin_cos();
    let a = deg.to_radians();
    let (x, y) = (scale * rx * a.cos(), scale * ry * a.sin());
    map.screen(Point::new(center.x + (x * co - y * s) * l.0, center.y + (x * s + y * co) * l.1))
}

/// The radial gradient's ellipse scaled by `scale`, as a closed line on screen.
fn radial_ring(map: &CanvasMap, center: Point, rx: f64, ry: f64, angle: f64, scale: f64) -> Vec<Pos2> {
    (0..72).map(|i| radial_point(map, center, rx, ry, angle, scale, i as f64 * 5.0)).collect()
}

/// How far `at` (normalized) is from the radial gradient's centre, 1 on its edge.
fn radial_r(shape: &MaskShape, at: Point, map: &CanvasMap) -> Option<f64> {
    let MaskShape::Radial { center, rx, ry, angle, .. } = shape else { return None };
    if !rx.is_finite() || !ry.is_finite() || *rx <= 0.0 || *ry <= 0.0 {
        return None;
    }
    let l = frame_long_norm(map);
    let dx = (at.x - center.x) / l.0;
    let dy = (at.y - center.y) / l.1;
    let (s, co) = angle.to_radians().sin_cos();
    let x = (dx * co + dy * s) / rx;
    let y = (-dx * s + dy * co) / ry;
    let r = (x * x + y * y).sqrt();
    r.is_finite().then_some(r)
}

/// `p` turned by `turn` radians about `c`, on screen (so the photo's aspect doesn't skew it).
fn turned(map: &CanvasMap, p: Point, c: Point, turn: f32) -> Point {
    let (pq, cq) = (map.screen(p), map.screen(c));
    let v = pq - cq;
    let (s, co) = turn.sin_cos();
    map.norm(cq + vec2(v.x * co - v.y * s, v.x * s + v.y * co))
}

/// The angle (radians, on screen) of `p` seen from `c`.
fn screen_angle(map: &CanvasMap, p: Point, c: Point) -> f32 {
    (map.screen(p) - map.screen(c)).angle()
}

/// The double-arrow cursor that resizes along `dir` (on screen).
fn resize_cursor(dir: egui::Vec2) -> egui::CursorIcon {
    let deg = dir.y.atan2(dir.x).to_degrees().rem_euclid(180.0);
    match deg {
        d if !(22.5..157.5).contains(&d) => egui::CursorIcon::ResizeHorizontal,
        d if d < 67.5 => egui::CursorIcon::ResizeNwSe,
        d if d < 112.5 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

/// What a press at `q` on the selected gradient `shape` grabs when no handle is under it: a
/// handle number for [`moved_shape`] (0 moves it), or `None` (nothing: the press is elsewhere).
/// Radial: inside moves, just outside the edge rotates. Linear: the middle line rotates, the
/// start and end lines move that line only (the fade's width), anywhere else moves the gradient.
fn gradient_part(shape: &MaskShape, q: Pos2, map: &CanvasMap) -> Option<u8> {
    match shape {
        MaskShape::Radial { center, .. } => {
            let r = radial_r(shape, map.norm(q), map)?;
            if r <= 1.0 {
                return Some(0);
            }
            // a band outside the edge, about 16 points wide on screen
            let edge = map.screen(*center).distance(q) / r as f32;
            (r <= 1.0 + f64::from(16.0 / edge.max(1.0))).then_some(ROTATE)
        }
        MaskShape::Linear { start, end } => {
            let (a, b) = (map.screen(*start), map.screen(*end));
            let len = a.distance(b);
            if len.is_nan() || len < 1.0 {
                return Some(0);
            }
            let dir = (b - a) / len;
            // distance along the gradient from the start line, in points
            let t = (q - a).dot(dir);
            if (t - len / 2.0).abs() < 6.0 {
                Some(ROTATE)
            } else if t.abs() < 6.0 {
                Some(LINE_START)
            } else if (t - len).abs() < 6.0 {
                Some(LINE_END)
            } else {
                Some(0)
            }
        }
        _ => None,
    }
}

/// [`moved_shape`]'s handles beyond the ones drawn: turning a gradient about its centre, and
/// moving a linear gradient's start or end line along it (the width of the fade, not its angle).
const ROTATE: u8 = 7;
const LINE_START: u8 = 11;
const LINE_END: u8 = 12;

/// The selected mask `m`'s gradient part under `q` when no handle is: (component, handle). A
/// linear gradient's lines first, then a radial gradient's body or the band round it, then the
/// first linear gradient as a whole (anywhere on the photo moves it).
fn gradient_grab(m: &lightcraft_develop::Mask, q: Pos2, map: &CanvasMap) -> Option<(usize, u8)> {
    let linear = |c: &&lightcraft_develop::MaskComponent| matches!(c.shape, MaskShape::Linear { .. });
    for (ci, c) in m.components.iter().enumerate().filter(|(_, c)| linear(c)) {
        if let Some(h) = gradient_part(&c.shape, q, map).filter(|h| *h != 0) {
            return Some((ci, h));
        }
    }
    for (ci, c) in m.components.iter().enumerate().filter(|(_, c)| matches!(c.shape, MaskShape::Radial { .. })) {
        if let Some(h) = gradient_part(&c.shape, q, map) {
            return Some((ci, h));
        }
    }
    m.components.iter().position(|c| linear(&c)).map(|ci| (ci, 0))
}

/// The system cursor for dragging `handle` of `shape` from `q`; `None` for a turn (no system
/// cursor shows that: the canvas draws its own).
fn gradient_cursor(shape: &MaskShape, handle: u8, q: Pos2, map: &CanvasMap) -> Option<egui::CursorIcon> {
    match (shape, handle) {
        (_, 0) => Some(egui::CursorIcon::Move),
        (MaskShape::Radial { .. }, 5 | ROTATE) | (MaskShape::Linear { .. }, ROTATE) => None,
        (MaskShape::Radial { center, rx, ry, angle, .. }, h) => {
            let deg = match h {
                1 | 3 => 0.0,
                2 | 4 => 90.0,
                _ => FEATHER_HANDLE_DEG,
            };
            let c = map.screen(*center);
            let dir = radial_point(map, *center, *rx, *ry, *angle, 1.0, deg) - c;
            let dir = if dir.length() > 0.5 { dir } else { q - c };
            Some(resize_cursor(dir))
        }
        (MaskShape::Linear { start, end }, _) => Some(resize_cursor(map.screen(*end) - map.screen(*start))),
        _ => Some(egui::CursorIcon::Move),
    }
}

/// Colour Range eyedropper: a click samples the colour under it for the selected mask's
/// colour range, at normalized frame coordinates (crop, rotation and zoom aware);
/// ⇧-click adds a sample (up to 5).
fn color_range_pick(app: &mut LightcraftApp, ui: &egui::Ui, resp: &egui::Response, map: &CanvasMap) {
    if resp.clicked()
        && let Some(q) = resp.interact_pointer_pos()
    {
        let n = map.norm(q);
        let add = ui.input(|i| i.modifiers.shift);
        if let Err(e) = app.run("mask.sampleColor", json!({"x": n.x, "y": n.y, "add": add})) {
            app.toast(ui.ctx(), e);
        }
    }
}

/// The Masking tool on the photo: outlines and handles of the selected mask, a pin per mask
/// component (click selects its mask, drag moves the component), and brush painting. The mask
/// itself shows as a rendered overlay ([`view_overlay`]).
fn mask_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    let long = (map.rect.width().max(map.rect.height())) as f64;
    let active = app.session.active_mask;
    let clip = app.canvas_rect.unwrap_or(map.rect);
    let mut painter = ui.painter_at(clip);
    // a slider or dial at work: no lines, handles or pins over the effect
    if adjusting(app) {
        painter.set_invisible();
    }
    let p = &painter;
    // (mask, component, handle, screen position) of everything that can be grabbed
    let mut grips: Vec<(u32, usize, u8, Pos2)> = Vec::new();
    // pins follow Show Pins and the Tool Overlay mode (Shift+H); Auto hides them while brushing
    let (mode, live) = (app.ui.tool_overlay, pins_live(app, resp, map));
    let mut pins = p.clone();
    if mode == ToolOverlay::Auto && matches!(app.gesture, Some(Gesture::Brush { .. })) {
        pins.set_invisible();
    }
    for m in &d.masks {
        let sel = Some(m.id) == active;
        for (ci, c) in m.components.iter().enumerate() {
            match &c.shape {
                MaskShape::Radial { center, rx, ry, angle, feather, .. } if sel => {
                    // the outer edge (where the effect ends) and, inside it, the feather ring (where
                    // it is full): thin, so the photo shows between them
                    p.add(egui::Shape::closed_line(
                        radial_ring(map, *center, *rx, *ry, *angle, 1.0),
                        Stroke::new(1.0, Color32::from_white_alpha(230)),
                    ));
                    let inner = radial_inner(*feather);
                    p.add(egui::Shape::dashed_line(
                        &radial_ring(map, *center, *rx, *ry, *angle, inner),
                        Stroke::new(1.0, Color32::from_white_alpha(170)),
                        4.0,
                        3.0,
                    ));
                    let fq = radial_point(map, *center, *rx, *ry, *angle, inner, FEATHER_HANDLE_DEG);
                    p.circle_filled(fq, 4.0, Color32::from_black_alpha(90));
                    p.circle_stroke(fq, 4.0, Stroke::new(1.5, Color32::WHITE));
                    register(ui.ctx(), format!("maskHandle:{}:{ci}:6", m.id), Rect::from_center_size(fq, vec2(14.0, 14.0)));
                    grips.push((m.id, ci, 6, fq));

                    let l = frame_long_norm(map);
                    for (i, a_deg) in [0.0_f64, 90.0_f64, 180.0_f64, 270.0_f64].into_iter().enumerate() {
                        let a_local = a_deg.to_radians();
                        let (s, co) = angle.to_radians().sin_cos();
                        let (x, y) = (rx * a_local.cos(), ry * a_local.sin());
                        let q = map.screen(Point::new(center.x + (x * co - y * s) * l.0, center.y + (x * s + y * co) * l.1));
                        p.circle_stroke(q, 5.0, Stroke::new(1.5, Color32::WHITE));
                        register(ui.ctx(), format!("maskHandle:{}:{ci}:{}", m.id, i + 1), Rect::from_center_size(q, vec2(14.0, 14.0)));
                        grips.push((m.id, ci, i as u8 + 1, q));
                    }

                    let (s, co) = angle.to_radians().sin_cos();
                    let rot_r = rx + 0.08;
                    let rot_q = map.screen(Point::new(center.x + (rot_r * co) * l.0, center.y + (rot_r * s) * l.1));
                    let h1_q = map.screen(Point::new(center.x + (rx * co) * l.0, center.y + (rx * s) * l.1));
                    p.line_segment([h1_q, rot_q], Stroke::new(1.0, Color32::from_white_alpha(200)));
                    p.circle_stroke(rot_q, 4.0, Stroke::new(1.5, Color32::WHITE));
                    register(ui.ctx(), format!("maskHandle:{}:{ci}:5", m.id), Rect::from_center_size(rot_q, vec2(14.0, 14.0)));
                    grips.push((m.id, ci, 5, rot_q));
                }
                MaskShape::Linear { start, end } if sel => {
                    let (a, b) = (map.screen(*start), map.screen(*end));
                    let dir = (b - a).normalized();
                    let perp = vec2(-dir.y, dir.x) * 3000.0;
                    // start (full effect), the middle (50 %, with the pin) and the end (no effect)
                    let mid = a.lerp(b, 0.5);
                    for (q, alpha) in [(a, 210), (mid, 150), (b, 110)] {
                        p.line_segment([q - perp, q + perp], Stroke::new(1.0, Color32::from_white_alpha(alpha)));
                    }
                    for (i, q) in [a, b].iter().enumerate() {
                        p.circle_stroke(*q, 5.0, Stroke::new(1.5, Color32::WHITE));
                        register(ui.ctx(), format!("maskHandle:{}:{ci}:{i}", m.id), Rect::from_center_size(*q, vec2(14.0, 14.0)));
                        grips.push((m.id, ci, i as u8 + 1, *q));
                    }
                }
                _ => {}
            }
            if app.ui.mask_pins
                && mode.shows(sel, live)
                && let Some(at) = component_pin(&c.shape)
            {
                let q = map.screen(at);
                pin(&pins, q, sel);
                register(ui.ctx(), format!("maskPin:{}:{ci}", m.id), Rect::from_center_size(q, vec2(14.0, 14.0)));
                grips.push((m.id, ci, 0, q));
            }
        }
    }
    // a mask tool armed in the panel: the first click or drag here makes its mask
    if app.ui.mask_new.as_ref().is_some_and(|(k, _)| *k != app.ui.tool) {
        app.ui.mask_new = None;
    }
    if let Some((kind, op)) = app.ui.mask_new.clone()
        && (resp.drag_started() || resp.clicked())
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let n0 = map.norm(q);
        let drag = resp.drag_started();
        let params = match kind.as_str() {
            "linear" if drag => Some(json!({"kind": "linear", "start": n0, "end": n0})),
            "radial" if drag => Some(json!({"kind": "radial", "center": n0, "rx": 0.01, "ry": 0.01})),
            "radial" => Some(json!({"kind": "radial", "center": n0})),
            "brush" | "colorRange" | "object" => Some(json!({"kind": kind})),
            _ => None,
        };
        if let Some(mut params) = params {
            let shaped = matches!(kind.as_str(), "linear" | "radial") && drag;
            if shaped {
                let _ = app.run("develop.beginInteraction", json!({"label": "New Mask"}));
            }
            let r = if op == "new" {
                app.run("mask.add", params)
            } else {
                params["op"] = json!(op);
                app.run("mask.addComponent", params)
            };
            match r {
                Ok(_) => {
                    app.ui.mask_new = None;
                    if shaped {
                        // drawing: the new gradient's end (or the ellipse's size) follows the pointer
                        let made = app.session.active().and_then(|id| app.session.develop_of(id)).and_then(|d| {
                            let m = d.masks.iter().find(|m| Some(m.id) == app.session.active_mask)?;
                            let k = m.components.len().checked_sub(1)?;
                            Some((m.id, k, m.components.get(k)?.shape.clone()))
                        });
                        if let Some((mask, comp, original_shape)) = made {
                            let handle = if kind == "linear" { 2 } else { 9 };
                            app.gesture = Some(Gesture::MaskHandle { mask, comp, handle, original_shape });
                        }
                        return;
                    }
                }
                Err(e) => {
                    if shaped {
                        let _ = app.run("develop.endInteraction", json!({}));
                    }
                    app.toast(ui.ctx(), e);
                    return;
                }
            }
        }
    }
    // a gradient armed: the first drag here draws it
    if app.ui.mask_new.as_ref().is_some_and(|(k, _)| k == "linear" || k == "radial") && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    // the pointer on a pin shows that mask's overlay (as Lightroom Classic)
    if let Some(h) = resp.hover_pos()
        && let Some((mid, ..)) = grips.iter().find(|(_, _, handle, q)| *handle == 0 && q.distance(h) < 9.0)
    {
        app.ui.mask_hover = Some(*mid);
    }
    mask_keys(app, ui, map, d, &grips);
    // Object selections (SAM 3) of the selected mask: each click, green to include, red to exclude
    // (clicks still on their way to the model included)
    let pending = app.session.segmenter.pending_clicks().cloned();
    for m in d.masks.iter().filter(|m| Some(m.id) == active) {
        for (k, c) in m.components.iter().enumerate() {
            if let MaskShape::Object { hint, exclude, .. } = &c.shape {
                let (hint, exclude) = match pending.as_ref().filter(|q| (q.mask, q.comp) == (m.id, k)) {
                    Some(q) => (&q.hint, &q.exclude),
                    None => (hint, exclude),
                };
                for (pts, col) in [(hint, Color32::from_rgb(40, 200, 90)), (exclude, Color32::from_rgb(230, 60, 60))] {
                    for q in pts {
                        let q = map.screen(*q);
                        p.circle_filled(q, 5.0, col);
                        p.circle_stroke(q, 5.0, Stroke::new(1.5, Color32::WHITE));
                    }
                }
            }
        }
    }
    // Colour Range "Pick": the Masking panel owns the loupe's clicks (general_interaction
    // never runs while it is open), so the eyedropper is handled here too.
    if app.ui.tool == "colorRange" {
        super::range_pick::color_tool(app, ui, resp, map, d, p);
        return;
    }
    // the Luminance Range picker
    if app.ui.tool == "lumRange" {
        super::range_pick::luminance_tool(app, ui, resp, map, p);
        return;
    }
    // the selected mask's Point Color eyedropper: a click samples the colour there
    if app.ui.tool == super::mask_local::PICK_TOOL {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            if let Ok(r) = app.run("mask.pickPointColor", json!({"x": n.x, "y": n.y})) {
                app.ui.mask_point_color = r["index"].as_u64().unwrap_or(0) as usize;
            }
            app.ui.tool.clear();
        }
        return;
    }
    // Object tool: a click includes what's under it, ⌥-click leaves it out
    if app.ui.tool == "object" {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let exclude = ui.input(|i| i.modifiers.alt);
            match app.run_typed("mask.objectPoint", json!({"x": n.x, "y": n.y, "exclude": exclude})) {
                Ok(_) => {
                    if let Some(mid) = app.session.active_mask {
                        app.ui.detail_due = Some((ui.input(|i| i.time) + 1.0, mid));
                    }
                }
                Err(e) => app.ai_error(ui.ctx(), e, Some(("object", "new"))),
            }
        }
        return;
    }
    // brush tool
    if app.ui.tool == "brush" {
        // an Auto Mask stroke cut short (no release seen): what went to the photo stays, in one step
        if matches!(app.gesture, Some(Gesture::Brush { live: Some(_), .. }))
            && !resp.drag_stopped()
            && !resp.clicked()
            && !ui.input(|i| i.pointer.any_down())
        {
            app.gesture = None;
            let _ = app.run("develop.endInteraction", json!({}));
        }
        let r = (app.ui.brush_size as f64 * long) as f32;
        if let Some(h) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            p.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
            p.circle_stroke(h, r * (1.0 - app.ui.brush_feather / 100.0).max(0.05), Stroke::new(1.0, Color32::from_white_alpha(120)));
            // the centre, where Auto Mask samples the colour and texture to follow
            brush_crosshair(p, h);
        }
        if let Some(q) = resp.interact_pointer_pos()
            && (resp.drag_started() || resp.dragged() || resp.clicked())
        {
            let n = map.norm(q);
            let force = if app.ui.brush_pressure { ui.input(pen_force) } else { None };
            match &mut app.gesture {
                // a held, unmoving pointer records nothing: coverage builds by passes, not time (#518)
                Some(Gesture::Brush { points, .. }) if points.last() == Some(&n) => {}
                Some(Gesture::Brush { points, pressure, .. }) => {
                    record_pressure(pressure, points.len(), force);
                    points.push(n);
                }
                _ => {
                    let mut pressure = Vec::new();
                    record_pressure(&mut pressure, 0, force);
                    app.gesture = Some(Gesture::Brush { points: vec![n], pressure, live: None });
                }
            }
        }
        // Auto Mask: the stroke so far goes to the photo every moment while painting, so the
        // edge-aware result shows live (one undo step for the whole stroke)
        let now = ui.input(|i| i.time);
        if app.ui.brush_auto_mask
            && let Some(Gesture::Brush { points, pressure, live }) = &app.gesture
            && live_due(*live, points.len(), now)
        {
            let (pts, pressure, first) = (points.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>(), pressure.clone(), live.is_none());
            if first {
                let _ = app.run("develop.beginInteraction", json!({"label": "Brush"}));
            }
            let erase = app.ui.brush_erase != ui.input(|i| i.modifiers.alt);
            let _ = app.run(
                "mask.brushStroke",
                json!({"points": pts, "size": app.ui.brush_size, "feather": app.ui.brush_feather, "flow": app.ui.brush_flow,
                       "erase": erase, "autoMask": true, "pressure": pressure, "continue": !first}),
            );
            if let Some(Gesture::Brush { live, .. }) = &mut app.gesture {
                *live = Some((now, pts.len()));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(LIVE_AUTO_MASK_EVERY));
        }
        // the stroke being painted (the render shows it once it's committed), with the committed
        // stroke's dab spacing, feather and flow (issue #517); with Auto Mask live the render shows
        // the real, edge-aware stroke instead
        if let Some(Gesture::Brush { points, pressure, live: None }) = &app.gesture {
            let screen: Vec<Pos2> = points.iter().map(|q| map.screen(*q)).collect();
            let opacity = if app.ui.mask_overlay && app.ui.mask_overlay_opacity > 0.0 { app.ui.mask_overlay_opacity / 100.0 } else { 110.0 / 255.0 };
            let params = crate::brush_live::LiveParams {
                clip,
                scale: ui.ctx().pixels_per_point(),
                r,
                hard: r * (1.0 - (app.ui.brush_feather / 100.0).clamp(0.0, 1.0)),
                flow: (app.ui.brush_flow / 100.0).clamp(0.0, 1.0),
                density: 1.0,
                color: app.ui.mask_overlay_color,
                opacity: opacity.clamp(0.0, 1.0),
            };
            crate::brush_live::paint(&mut app.brush_live, ui.ctx(), p, params, &screen, pressure);
        } else {
            app.brush_live = None;
        }
        if (resp.drag_stopped() || resp.clicked())
            && let Some(Gesture::Brush { points, pressure, live }) = app.gesture.take()
        {
            app.brush_live = None;
            let pts: Vec<[f64; 2]> = points.iter().map(|q| [q.x, q.y]).collect();
            // holding ⌥ (Alt) paints with the other mode: Erase while adding, Add while erasing
            let erase = app.ui.brush_erase != resp.ctx.input(|i| i.modifiers.alt);
            let _ = app.run(
                "mask.brushStroke",
                json!({"points": pts, "size": app.ui.brush_size, "feather": app.ui.brush_feather, "flow": app.ui.brush_flow,
                       "erase": erase, "autoMask": app.ui.brush_auto_mask, "pressure": pressure, "continue": live.is_some()}),
            );
            if live.is_some() {
                let _ = app.run("develop.endInteraction", json!({}));
            }
        }
        return;
    }
    // the grip under the pointer: the selected mask's first, then the nearest
    let hit = |q: Pos2| {
        grips
            .iter()
            .filter(|g| g.3.distance(q) < 10.0)
            .min_by(|a, b| {
                (Some(a.0) != active, a.3.distance(q)).partial_cmp(&(Some(b.0) != active, b.3.distance(q))).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    };
    // what a press here grabs: a handle or pin, else a part of the selected mask's gradients
    let grab = |q: Pos2| {
        hit(q).map(|(m, c, h, _)| (m, c, h)).or_else(|| {
            let m = d.masks.iter().find(|m| Some(m.id) == active)?;
            gradient_grab(m, q, map).map(|(ci, h)| (m.id, ci, h))
        })
    };
    // the pointer says what a drag will do: move, resize (the edge, a line, the feather ring) or
    // turn the gradient
    let held = match &app.gesture {
        Some(Gesture::MaskHandle { mask, comp, handle, .. }) => Some((*mask, *comp, *handle)),
        _ => None,
    };
    if let Some(h) = resp.hover_pos().or_else(|| held.and_then(|_| ui.input(|i| i.pointer.latest_pos())))
        && let Some((mid, ci, handle)) = held.or_else(|| grab(h))
        && let Some(shape) = d.masks.iter().find(|m| m.id == mid).and_then(|m| m.components.get(ci)).map(|c| &c.shape)
        && !adjusting(app)
        && app.ui.mask_new.is_none()
    {
        match gradient_cursor(shape, handle, h, map) {
            Some(cursor) => ui.ctx().set_cursor_icon(cursor),
            None => {
                let c = match shape {
                    MaskShape::Radial { center, .. } => map.screen(*center),
                    MaskShape::Linear { start, end } => map.screen(*start).lerp(map.screen(*end), 0.5),
                    _ => h,
                };
                let away = h - c;
                let turn = if away.length() > 0.5 { away.y.atan2(away.x) + std::f32::consts::FRAC_PI_2 } else { 0.0 };
                rotate_cursor(ui, h, turn);
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            }
        }
    }
    if resp.clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && let Some((mid, ..)) = hit(q)
        && Some(mid) != active
    {
        let _ = app.run("mask.select", json!({"id": mid}));
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let grip = grab(q);
        if let Some((mask, comp, handle)) = grip
            && let Some(original_shape) = d.masks.iter().find(|m| m.id == mask).and_then(|m| m.components.get(comp)).map(|c| c.shape.clone())
        {
            if Some(mask) != active {
                let _ = app.run("mask.select", json!({"id": mask}));
            }
            let _ = app.run("develop.beginInteraction", json!({"label": "Edit Mask"}));
            app.gesture = Some(Gesture::MaskHandle { mask, comp, handle, original_shape });
        }
    }
    if resp.dragged()
        && let (Some(Gesture::MaskHandle { mask, comp, handle, original_shape }), Some(q)) = (app.gesture.clone(), resp.interact_pointer_pos())
    {
        let n = map.norm(q);
        let press_origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(q);
        let n0 = map.norm(press_origin);
        let dn = Point::new(n.x - n0.x, n.y - n0.y);
        let alt = ui.input(|i| i.modifiers.alt);
        let new_shape = moved_shape(&original_shape, handle, dn, n, map, alt);
        let _ = app.run("mask.update", json!({"id": mask, "component": comp, "shape": new_shape}));
    }
    if resp.drag_stopped() && matches!(app.gesture, Some(Gesture::MaskHandle { .. })) {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
}

/// How often (seconds) a stroke being painted with Auto Mask goes to the photo.
const LIVE_AUTO_MASK_EVERY: f64 = 0.12;

/// Whether the stroke of `len` points should go to the photo now: the first time once it has two
/// points, then every [`LIVE_AUTO_MASK_EVERY`] when it grew.
fn live_due(live: Option<(f64, usize)>, len: usize, now: f64) -> bool {
    match live {
        None => len >= 2,
        Some((at, sent)) => len > sent && (now - at >= LIVE_AUTO_MASK_EVERY || now < at || now.is_nan()),
    }
}

/// A small crosshair at the brush's centre (dark under light, so it reads on any photo).
fn brush_crosshair(p: &egui::Painter, c: Pos2) {
    for (w, col) in [(3.0, Color32::from_black_alpha(140)), (1.0, Color32::WHITE)] {
        let s = Stroke::new(w, col);
        p.line_segment([c - vec2(6.0, 0.0), c - vec2(2.0, 0.0)], s);
        p.line_segment([c + vec2(2.0, 0.0), c + vec2(6.0, 0.0)], s);
        p.line_segment([c - vec2(0.0, 6.0), c - vec2(0.0, 2.0)], s);
        p.line_segment([c + vec2(0.0, 2.0), c + vec2(0.0, 6.0)], s);
    }
}

/// A mask handle's name for screen readers: `handle` 0 is the component's pin, 1..5 a radial
/// gradient's right / bottom / left / top edge and rotation, 1..2 a linear gradient's start and end.
fn mask_handle_name(mask: &str, shape: &MaskShape, handle: u8) -> String {
    let part = match (shape, handle) {
        (_, 0) => "pin",
        (MaskShape::Radial { .. }, 1) => "right edge",
        (MaskShape::Radial { .. }, 2) => "bottom edge",
        (MaskShape::Radial { .. }, 3) => "left edge",
        (MaskShape::Radial { .. }, 4) => "top edge",
        (MaskShape::Radial { .. }, 6) => "feather",
        (MaskShape::Radial { .. }, _) => "rotation",
        (MaskShape::Linear { .. }, 1) => "start",
        (MaskShape::Linear { .. }, _) => "end",
        _ => "handle",
    };
    format!("{mask} {}", crate::i18n::tr(part))
}

/// Keyboard stand-ins for the mask handles and pins: an arrow press moves the focused one by a
/// step (one undo step), as dragging it would; Return on a pin selects its mask.
fn mask_keys(app: &mut LightcraftApp, ui: &mut egui::Ui, map: &CanvasMap, d: &DevelopSettings, grips: &[(u32, usize, u8, Pos2)]) {
    for &(mid, ci, handle, q) in grips {
        let Some(m) = d.masks.iter().find(|m| m.id == mid) else { continue };
        let Some(shape) = m.components.get(ci).map(|c| &c.shape) else { continue };
        let at = map.norm(q);
        let mask_name = if m.name.trim().is_empty() { format!("{} {mid}", crate::i18n::tr("Mask")) } else { m.name.clone() };
        let name = mask_handle_name(&mask_name, shape, handle);
        let keys = crate::canvas_keys::handle(ui, &format!("mask:{mid}:{ci}:{handle}"), &name, crate::canvas_keys::position_text(at.x, at.y), q);
        if keys.activated && app.session.active_mask != Some(mid) {
            let _ = app.run("mask.select", json!({"id": mid}));
        }
        let Some((dx, dy)) = keys.delta else { continue };
        if app.session.active_mask != Some(mid) {
            let _ = app.run("mask.select", json!({"id": mid}));
        }
        let to = Point::new(at.x + dx, at.y + dy);
        let new_shape = moved_shape(shape, handle, Point::new(dx, dy), to, map, false);
        let _ = app.run("develop.beginInteraction", json!({"label": "Edit Mask"}));
        let _ = app.run("mask.update", json!({"id": mid, "component": ci, "shape": new_shape}));
        let _ = app.run("develop.endInteraction", json!({}));
    }
}

/// Long-edge units → normalized units for x and y.
fn frame_long_norm(map: &CanvasMap) -> (f64, f64) {
    // derive from the mapping: 1 normalized unit in x and y in screen points
    let o = map.screen(Point::new(0.0, 0.0));
    let x = map.screen(Point::new(1.0, 0.0)).distance(o) as f64;
    let y = map.screen(Point::new(0.0, 1.0)).distance(o) as f64;
    let l = x.max(y);
    (l / x.max(1e-9), l / y.max(1e-9))
}

/// Auto's condition for the Tool Overlay (Shift+H): the pointer is over the photo, or a pin is
/// being dragged.
fn pins_live(app: &LightcraftApp, resp: &egui::Response, map: &CanvasMap) -> bool {
    resp.hover_pos().is_some_and(|q| map.rect.contains(q))
        || matches!(app.gesture, Some(Gesture::SpotMove { .. } | Gesture::MaskHandle { .. }))
        // a handle moved with the keyboard stays shown while it has the focus
        || crate::canvas_keys::has_focus(&resp.ctx)
}

fn pin(p: &egui::Painter, c: Pos2, sel: bool) {
    p.circle_filled(c, 6.0, if sel { Color32::from_rgb(1, 101, 221) } else { Color32::from_gray(200) });
    p.circle_stroke(c, 6.0, Stroke::new(1.5, Color32::WHITE));
}

// ------------------------------------------------------------------------ remove

/// The Remove tool on the photo: every spot's outline and pin (the selected one with its source),
/// click a pin to select its spot, drag a target or source to move it, paint elsewhere to add one.
/// The Tool Overlay mode (Shift+H) says which spots show; a hidden spot can't be grabbed, so a
/// click there adds a new one.
fn remove_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    let long = (map.rect.width().max(map.rect.height())) as f64;
    let active = app.session.active_spot.filter(|i| *i < d.spots.len());
    let (mode, live) = (app.ui.tool_overlay, pins_live(app, resp, map));
    // Auto also hides them while a new spot is painted
    let painting = mode == ToolOverlay::Auto && matches!(app.gesture, Some(Gesture::Spot { .. }));
    let mut p = ui.painter_at(app.canvas_rect.unwrap_or(map.rect));
    // a slider or dial at work: the spots' outlines and pins hide over the result
    if adjusting(app) {
        p.set_invisible();
    }
    let mut pins = p.clone();
    if painting {
        pins.set_invisible();
    }
    // (spot, is source, screen centre, radius) of everything that can be grabbed
    let mut grips: Vec<(usize, bool, Pos2, f32)> = Vec::new();
    // the spot under the pointer (its painted area or its source): it shows thin, with both pins
    let hot =
        resp.hover_pos().filter(|_| !painting && matches!(mode, ToolOverlay::Auto | ToolOverlay::Always)).and_then(|h| spot_at(d, map, long, h));
    for (i, s) in d.spots.iter().enumerate() {
        let r = (s.size * long) as f32;
        let sel = Some(i) == active;
        let hot = hot == Some(i);
        if !mode.shows(sel, live) && !hot {
            continue;
        }
        let p = &pins;
        // Remove Dust's spots: amber (the camera's known dust) or yellow (found in this photo alone); with Visualize
        // Spots on they stand out more
        let col = match s.auto {
            Some(a) => {
                let alpha = if sel || app.ui.visualize_spots { 255 } else { 190 };
                if a.confirmed { Color32::from_rgba_unmultiplied(255, 166, 0, alpha) } else { Color32::from_rgba_unmultiplied(255, 228, 92, alpha) }
            }
            None => Color32::from_white_alpha(if sel || hot { 230 } else { 150 }),
        };
        let width = if s.auto.is_some() && app.ui.visualize_spots { 2.0 } else { 1.0 };
        // one outline round the whole painted area (never a ring per dab, which hides the result)
        let target: Vec<Pos2> = s.points.iter().map(|q| map.screen(*q)).collect();
        union_outline(p, &target, r, Stroke::new(width, col));
        let Some(&t) = s.points.first() else { continue };
        let tq = map.screen(t);
        // the source (where the pixels come from): its outline, an arrow to the target, its pin
        if let Some(o) = s.source_offset.filter(|_| sel || hot) {
            let sq = map.screen(Point::new(t.x + o.x, t.y + o.y));
            let source: Vec<Pos2> = s.points.iter().map(|q| map.screen(Point::new(q.x + o.x, q.y + o.y))).collect();
            union_outline(p, &source, r, Stroke::new(1.0, Color32::from_white_alpha(150)));
            let v = tq - sq;
            if v.length() > 2.0 * r + 12.0 {
                let dir = v.normalized();
                p.arrow(sq + dir * r, v - dir * 2.0 * r, Stroke::new(1.0, Color32::from_white_alpha(170)));
            }
            p.circle_filled(sq, 5.0, Color32::from_black_alpha(110));
            p.circle_stroke(sq, 5.0, Stroke::new(1.5, Color32::WHITE));
            register(ui.ctx(), format!("spotSource:{i}"), Rect::from_center_size(sq, vec2(14.0, 14.0)));
            grips.push((i, true, sq, r));
        }
        match s.auto {
            // Remove Dust's spots: an amber (known dust) or yellow (this photo alone) pin
            Some(a) if !sel => {
                p.circle_filled(tq, 6.0, if a.confirmed { Color32::from_rgb(255, 166, 0) } else { Color32::from_rgb(240, 214, 80) });
                p.circle_stroke(tq, 6.0, Stroke::new(1.5, Color32::WHITE));
            }
            _ => pin(p, tq, sel),
        }
        register(ui.ctx(), format!("spotPin:{i}"), Rect::from_center_size(tq, vec2(14.0, 14.0)));
        grips.push((i, false, tq, r));
    }
    if spot_keys(app, ui, map, d, &grips) {
        return;
    }
    let r = (app.ui.remove_size as f64 * long) as f32;
    // a target or source under `q`: the selected spot's first, then the nearest pin
    let hit = |q: Pos2| {
        grips
            .iter()
            .filter(|g| g.2.distance(q) < g.3.max(8.0))
            .min_by(|a, b| {
                (Some(a.0) != active, a.2.distance(q)).partial_cmp(&(Some(b.0) != active, b.2.distance(q))).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    };
    let hover = resp.hover_pos().and_then(hit);
    if let Some(h) = resp.hover_pos() {
        if hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            p.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
            p.circle_stroke(h, r * (1.0 - app.ui.remove_feather / 100.0).max(0.05), Stroke::new(1.0, Color32::from_white_alpha(110)));
        }
    }
    let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
    if resp.drag_started()
        && let Some((spot, source, ..)) = press.and_then(hit)
    {
        if Some(spot) != active {
            let _ = app.run("spot.select", json!({"index": spot}));
        }
        let _ = app.run("develop.beginInteraction", json!({"label": "Edit Spot"}));
        app.gesture = Some(Gesture::SpotMove { spot, source });
    }
    if let Some(Gesture::SpotMove { spot, source }) = app.gesture.clone() {
        if resp.dragged()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let (n, n0) = (map.norm(q), map.norm(q - resp.drag_delta()));
            let dn = [n.x - n0.x, n.y - n0.y];
            if dn != [0.0, 0.0] {
                let _ = app.run("spot.update", json!({"index": spot, if source { "moveSource" } else { "move" }: dn}));
            }
        }
        if resp.drag_stopped() {
            app.gesture = None;
            let _ = app.run("develop.endInteraction", json!({}));
        }
        return;
    }
    if resp.clicked()
        && let Some((spot, ..)) = press.and_then(hit)
    {
        let _ = app.run("spot.select", json!({"index": spot}));
        return;
    }
    if let Some(q) = resp.interact_pointer_pos()
        && (resp.drag_started() || resp.dragged() || resp.clicked())
    {
        let n = map.norm(q);
        match &mut app.gesture {
            Some(Gesture::Spot { points }) => points.push(n),
            _ => app.gesture = Some(Gesture::Spot { points: vec![n] }),
        }
    }
    // the spot being painted: its outline only, so what it covers stays visible
    if let Some(Gesture::Spot { points }) = &app.gesture {
        let screen: Vec<Pos2> = points.iter().map(|q| map.screen(*q)).collect();
        union_outline(&p, &screen, r, Stroke::new(1.0, Color32::from_white_alpha(220)));
    }
    if (resp.drag_stopped() || resp.clicked())
        && let Some(Gesture::Spot { points }) = app.gesture.take()
    {
        let mode = match app.ui.tool.as_str() {
            "heal" => "heal",
            "clone" => "clone",
            "fill" => "fill",
            "generate" => "generate",
            _ => "remove",
        };
        let pts: Vec<[f64; 2]> = points.iter().map(|q| [q.x, q.y]).collect();
        let _ = app.run(
            "spot.add",
            json!({"mode": mode, "points": pts, "size": app.ui.remove_size, "feather": app.ui.remove_feather, "opacity": app.ui.remove_opacity}),
        );
    }
}

/// The spot whose painted area (or, for the selected one, source) is under `h`: the last drawn
/// first, as it is on top.
fn spot_at(d: &DevelopSettings, map: &CanvasMap, long: f64, h: Pos2) -> Option<usize> {
    d.spots.iter().enumerate().rev().find_map(|(i, s)| {
        let r = ((s.size * long) as f32).max(4.0);
        let o = s.source_offset.unwrap_or(Point::new(0.0, 0.0));
        let inside = s.points.iter().any(|q| map.screen(*q).distance(h) <= r)
            || (s.source_offset.is_some() && s.points.iter().any(|q| map.screen(Point::new(q.x + o.x, q.y + o.y)).distance(h) <= r));
        inside.then_some(i)
    })
}

/// The outline of the union of circles of radius `r` round `centres` (a painted stroke), drawn
/// thin ([`union_arcs`]).
pub(crate) fn union_outline(p: &egui::Painter, centres: &[Pos2], r: f32, stroke: Stroke) {
    for arc in union_arcs(centres, r) {
        p.add(egui::Shape::line(arc, stroke));
    }
}

/// The outline of the union of circles of radius `r` round `centres`: each circle's arcs that no
/// other circle covers. Centres closer than a quarter radius to the last one kept are skipped (a
/// stroke records many), and each circle is only tested against its neighbours.
pub(crate) fn union_arcs(centres: &[Pos2], r: f32) -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    if !(r.is_finite() && r > 0.5) {
        return out;
    }
    let mut kept: Vec<Pos2> = Vec::with_capacity(centres.len().min(4096));
    for c in centres.iter().filter(|c| c.x.is_finite() && c.y.is_finite()) {
        if kept.last().is_none_or(|l| l.distance(*c) >= r * 0.25) && kept.len() < 4096 {
            kept.push(*c);
        }
    }
    const N: usize = 40;
    let ring: Vec<egui::Vec2> = (0..=N).map(|k| egui::Vec2::angled(k as f32 / N as f32 * std::f32::consts::TAU) * r).collect();
    for (i, c) in kept.iter().enumerate() {
        let near: Vec<Pos2> = kept.iter().enumerate().filter(|(j, o)| *j != i && o.distance(*c) < 2.0 * r).map(|(_, o)| *o).collect();
        let open = |q: Pos2| near.iter().all(|o| o.distance(q) >= r - 0.5);
        let mut run: Vec<Pos2> = Vec::new();
        for v in &ring {
            let q = *c + *v;
            if open(q) {
                run.push(q);
            } else if run.len() > 1 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
        }
        if run.len() > 1 {
            out.push(run);
        }
    }
    out
}

/// Keyboard stand-ins for the Remove tool: each spot's target and source (an arrow press moves
/// it a step, one undo step; Return selects the spot), and a placement point that the arrows move
/// and Return turns into a new spot of the brush's size there. True when a spot was added.
fn spot_keys(app: &mut LightcraftApp, ui: &mut egui::Ui, map: &CanvasMap, d: &DevelopSettings, grips: &[(usize, bool, Pos2, f32)]) -> bool {
    for &(spot, source, q, _) in grips {
        let at = map.norm(q);
        let name =
            format!("{} {}{}", crate::i18n::tr("Spot"), spot + 1, if source { format!(" {}", crate::i18n::tr("source")) } else { String::new() });
        let size = d.spots.get(spot).map(|s| s.size).unwrap_or_default();
        let value = format!("{}, {} {:.0}%", crate::canvas_keys::position_text(at.x, at.y), crate::i18n::tr("size"), size * 100.0);
        let keys = crate::canvas_keys::handle(ui, &format!("spot:{spot}:{}", if source { "source" } else { "target" }), &name, value, q);
        if (keys.activated || keys.delta.is_some()) && app.session.active_spot != Some(spot) {
            let _ = app.run("spot.select", json!({"index": spot}));
        }
        if let Some((dx, dy)) = keys.delta {
            let _ = app.run("develop.beginInteraction", json!({"label": "Edit Spot"}));
            let _ = app.run("spot.update", json!({"index": spot, if source { "moveSource" } else { "move" }: [dx, dy]}));
            let _ = app.run("develop.endInteraction", json!({}));
        }
    }
    // where a new spot goes: the photo's centre at first, then wherever the arrows took it
    let key = egui::Id::new("lc-spot-placement");
    let place: (f64, f64) = ui.data(|m| m.get_temp(key)).unwrap_or((0.5, 0.5));
    let q = map.screen(Point::new(place.0, place.1));
    let keys = crate::canvas_keys::handle(
        ui,
        "spot:new",
        crate::i18n::tr("New spot (Return adds it here)"),
        crate::canvas_keys::position_text(place.0, place.1),
        q,
    );
    if let Some((dx, dy)) = keys.delta {
        let moved = ((place.0 + dx).clamp(0.0, 1.0), (place.1 + dy).clamp(0.0, 1.0));
        ui.data_mut(|m| m.insert_temp(key, moved));
    }
    if crate::canvas_keys::has_focus(ui.ctx()) && ui.memory(|m| m.has_focus(crate::canvas_keys::id("spot:new"))) {
        let r = (app.ui.remove_size as f64 * (map.rect.width().max(map.rect.height())) as f64) as f32;
        ui.painter().circle_stroke(q, r, Stroke::new(1.0, Color32::WHITE));
    }
    if keys.activated {
        let mode = match app.ui.tool.as_str() {
            "heal" => "heal",
            "clone" => "clone",
            "fill" => "fill",
            "generate" => "generate",
            _ => "remove",
        };
        let _ = app.run(
            "spot.add",
            json!({"mode": mode, "points": [[place.0, place.1]], "size": app.ui.remove_size, "feather": app.ui.remove_feather, "opacity": app.ui.remove_opacity}),
        );
        return true;
    }
    false
}

// ------------------------------------------------------------------------ red eye

/// Red Eye tool: drag an ellipse over an eye (a new correction), click one to select it.
fn eye_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    // screen points per long-edge unit
    let o = map.screen(Point::new(0.0, 0.0));
    let (sx, sy) = (map.screen(Point::new(1.0, 0.0)).distance(o), map.screen(Point::new(0.0, 1.0)).distance(o));
    let per_long = sx.max(sy);
    let mut p = ui.painter().clone();
    if adjusting(app) {
        p.set_invisible();
    }
    for (i, e) in d.red_eye.iter().enumerate() {
        let c = map.screen(e.center);
        let r = vec2(e.rx as f32 * per_long, e.ry as f32 * per_long);
        let sel = i == app.ui.eye;
        let col = if sel { Color32::WHITE } else { Color32::from_white_alpha(150) };
        p.add(egui::Shape::ellipse_stroke(c, r, Stroke::new(if sel { 1.5 } else { 1.0 }, col)));
        p.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], Stroke::new(1.0, col));
        p.line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], Stroke::new(1.0, col));
    }
    if resp.hover_pos().is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        // from where the button went down (a drag is recognized only after some movement)
        app.gesture = Some(Gesture::Eye { a: map.norm(q) });
    }
    if let Some(Gesture::Eye { a }) = app.gesture
        && let Some(q) = resp.interact_pointer_pos()
    {
        let (a, b) = (map.screen(a), q);
        p.add(egui::Shape::ellipse_stroke(a.lerp(b, 0.5), ((b - a) / 2.0).abs(), Stroke::new(1.0, Color32::WHITE)));
    }
    if resp.drag_stopped()
        && let Some(Gesture::Eye { a }) = app.gesture.take()
        && let Some(q) = resp.interact_pointer_pos()
    {
        let (sa, sb) = (map.screen(a), q);
        let half = ((sb - sa) / 2.0).abs() / per_long;
        if half.x > 0.002 && half.y > 0.002 {
            let c = map.norm(sa.lerp(sb, 0.5));
            let r = app.run("redeye.add", json!({"center": [c.x, c.y], "rx": half.x, "ry": half.y, "pet": app.ui.eye_pet}));
            if let Ok(v) = r {
                app.ui.eye = v["index"].as_u64().unwrap_or(0) as usize;
            }
        }
    } else if resp.clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && let Some(i) = d.red_eye.iter().position(|e| {
            let c = map.screen(e.center);
            let (dx, dy) = ((q.x - c.x) / (e.rx as f32 * per_long).max(4.0), (q.y - c.y) / (e.ry as f32 * per_long).max(4.0));
            dx * dx + dy * dy <= 1.0
        })
    {
        app.ui.eye = i;
    }
}

// ------------------------------------------------------------------------ filmstrip

/// A filmstrip cell's file-name label: names longer than 14 characters are cut to 13 and an
/// ellipsis. Counts characters, not bytes, so a CJK name is never cut inside a character (#266).
fn film_label(name: &str) -> String {
    if name.chars().nth(14).is_some() { format!("{}…", name.chars().take(13).collect::<String>()) } else { name.to_string() }
}

/// Where the filmstrip is in the shoot: "1,234 / 2,233" at its right (with the selection's size).
fn film_overview(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, ids: &[PhotoId], active: Option<PhotoId>) {
    let t = Tokens::get(ui.ctx());
    let n = ids.len();
    if n == 0 {
        return;
    }
    let p = ui.painter();
    let at = active.and_then(|a| ids.iter().position(|x| *x == a));
    let mut text = match at {
        Some(i) => format!("{} / {}", crate::i18n::group_digits(i + 1), crate::i18n::group_digits(n)),
        None => crate::i18n::group_digits(n),
    };
    let sel = app.session.selection.ids.len();
    if sel > 1 {
        text.push_str(&crate::i18n::tr_format!(" · {n} selected", n = crate::i18n::group_digits(sel)));
    }
    let g = p.layout_no_wrap(text, t.semibold(11.5), Color32::WHITE);
    let pill = Rect::from_min_size(pos2(r.right() - g.size().x - 22.0, r.top() + 7.0), g.size() + vec2(14.0, 4.0));
    p.rect_filled(pill, 4.0, Color32::from_black_alpha(190));
    p.galley(pill.min + vec2(7.0, 2.0), g, Color32::WHITE);
    register(ui.ctx(), "filmstrip:position", pill);
}

/// Width of a filmstrip cell for a strip `height` points high (the shape of its original
/// 120 × 142 cells).
pub(crate) fn film_cell_width(height: f32) -> f32 {
    let h = if height.is_finite() { height } else { crate::state::FILM_HEIGHT.default };
    (h * 120.0 / 142.0).round().max(60.0)
}

/// The thumbnail size (px) a filmstrip cell `cell_w` points wide asks for: what it shows at the
/// screen's density, from the thumbnail sizes the cache keeps (at most 512).
pub(crate) fn film_thumb_px(cell_w: f32, ppp: f32) -> usize {
    let px = cell_w * if ppp.is_finite() { ppp.clamp(1.0, 3.0) } else { 1.0 } * 2.0;
    (px.round() as usize).clamp(128, 512)
}

/// The filmstrip's top edge: dragged up or down, it resizes the strip (and its thumbnails) within
/// [`crate::state::FILM_HEIGHT`]; the height is kept with the UI settings (widget `filmstrip:resize`).
fn film_resize(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let edge = Rect::from_min_size(r.min - vec2(0.0, 2.0), vec2(r.width(), 7.0));
    let resp = ui.interact(edge, egui::Id::new("film-resize"), Sense::drag());
    crate::widgets::announce(&resp, crate::i18n::tr("Filmstrip height"));
    register(ui.ctx(), "filmstrip:resize", edge);
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.dragged() {
        let dy = resp.drag_delta().y;
        if dy.is_finite() && dy != 0.0 {
            app.ui.film_height = crate::state::FILM_HEIGHT.clamp(app.ui.film_height - dy);
        }
    }
    if resp.double_clicked() {
        app.ui.film_height = crate::state::FILM_HEIGHT.default;
    }
}

pub(crate) fn filmstrip(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, t.canvas);
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 4.0)), 0.0, Color32::from_gray(0x20));
    film_resize(app, ui, r);
    let ids = app.session.visible_cloned();
    let active = app.session.selection.active;
    // the cells keep the shape they had at the strip's original height (120 wide at 142 high)
    let cell_w = film_cell_width(r.height());
    let ppp = ui.ctx().pixels_per_point();
    let thumb_px = film_thumb_px(cell_w, ppp);
    if ids.is_empty() {
        let why = if app.session.filter != Default::default() { "No photos match the filters (View → Clear Filters)" } else { "No photos" };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, why, t.font(12.5), t.text_dim);
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(0.0, 4.0))));
    // a vertical mouse wheel scrolls the strip sideways (horizontal trackpad scrolls still work)
    child.style_mut().always_scroll_the_only_direction = true;
    // only a new active photo (or the strip appearing) scrolls it; see `grid::follow_active`
    let follow = super::grid::follow_active(child.ctx(), egui::Id::new("film-follow-active"), active, app.session.source);
    // a wheel notch scrolls a third of its width; see `grid::wheel_step`
    register(ui.ctx(), "filmstrip", child.max_rect());
    let wheel = super::grid::wheel_step(&child, egui::Id::new("film-wheel"), child.max_rect(), child.max_rect().width(), true);
    // a bar that can be seen and grabbed, as in the grid
    super::grid::visible_scroll_bar(&mut child);
    let film_area = egui::ScrollArea::horizontal().id_salt("filmstrip").auto_shrink([false, false]);
    // the handle always there: it says where the strip is in the view
    let film_area = film_area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
    film_area.show_viewport(&mut child, |ui, vp| {
        if wheel != egui::Vec2::ZERO {
            ui.scroll_with_delta_animation(wheel, egui::style::ScrollAnimation::none());
        }
        let (area, _) = ui.allocate_exact_size(vec2(ids.len() as f32 * cell_w, (ui.available_height() - 2.0).max(40.0)), Sense::hover());
        app.film_scroll = Some(vp.left());
        for (i, id) in ids.iter().enumerate() {
            let cr = Rect::from_min_size(pos2(area.left() + i as f32 * cell_w, area.top()), vec2(cell_w, area.height()));
            let local = Rect::from_min_size(pos2(i as f32 * cell_w, 0.0), cr.size());
            // centred when it is (partly) off screen; a click on a visible thumbnail leaves the strip alone
            if follow && Some(*id) == active && !(local.left() >= vp.left() && local.right() <= vp.right()) {
                ui.scroll_to_rect(cr, Some(egui::Align::Center));
            }
            if !local.intersects(vp.expand2(vec2(cell_w * 4.0, 0.0))) {
                continue;
            }
            let resp = ui.interact(cr, egui::Id::new(("film", id.0)), Sense::click());
            crate::widgets::announce(&resp, app.session.catalog.photo(*id).map(|p| p.file_name.clone()).unwrap_or_default());
            register(ui.ctx(), format!("film:{}", id.0), cr);
            let state = app.session.selection.state_of(*id);
            let sel = state == lightcraft_engine::SelectionState::Active;
            let p = ui.painter();
            let label = app.session.catalog.photo(*id).and_then(|ph| ph.label);
            let selected = state != lightcraft_engine::SelectionState::NotSelected;
            let flag = app.session.catalog.photo(*id).map(|ph| ph.flag).unwrap_or_default();
            let base = if selected {
                t.cell_selected
            } else if resp.hovered() {
                t.cell_selected.gamma_multiply(0.6)
            } else {
                super::grid::flag_tint(t.canvas, flag)
            };
            p.rect_filled(cr, 0.0, crate::theme::label_background(base, label, selected));
            super::grid::flag_line(p, &t, cr, flag);
            let names = app.ui.settings.film_names;
            if let Some(ph) = app.session.catalog.photo(*id).filter(|_| names) {
                let name = ph.file_name.rsplit_once('.').map(|(n, _)| n).unwrap_or(&ph.file_name);
                p.text(pos2(cr.left() + 8.0, cr.top() + 10.0), Align2::LEFT_CENTER, film_label(name), t.font(10.0), t.text_dim);
                p.text(pos2(cr.right() - 8.0, cr.top() + 10.0), Align2::RIGHT_CENTER, &ph.format, t.semibold(8.5), t.text_dim);
            }
            // the marks under the picture (Settings ▸ Interface), or over its bottom edge
            let under = app.ui.settings.badges_under && app.ui.settings.film_badges;
            let strip = if under { (cr.height() * 0.11).clamp(12.0, 16.0) } else { 0.0 };
            let img_area = Rect::from_min_max(cr.min + vec2(10.0, 22.0), cr.max - vec2(10.0, 8.0 + strip));
            if under && let Some(ph) = app.session.catalog.photo(*id) {
                let bar = Rect::from_min_max(pos2(cr.left() + 8.0, img_area.bottom() + 3.0), pos2(cr.right() - 8.0, img_area.bottom() + 3.0 + strip));
                super::grid::paint_badges(p, &t, bar, ph, false);
            }
            super::grid::request_thumb(app, *id, thumb_px, 8);
            if let Some(tex) = app.renderer.thumb(*id) {
                let [tw, th] = tex.size;
                let s = (img_area.width() / tw as f32).min(img_area.height() / th as f32);
                let fr = Rect::from_center_size(img_area.center(), vec2(tw as f32 * s, th as f32 * s));
                p.image(tex.id(), fr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if sel {
                    p.rect_stroke(fr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
                } else if state == lightcraft_engine::SelectionState::Selected {
                    p.rect_stroke(fr, 0.0, Stroke::new(1.5, Color32::from_gray(170)), StrokeKind::Outside);
                }
                if app.ui.settings.film_badges
                    && !under
                    && let Some(ph) = app.session.catalog.photo(*id)
                {
                    film_badges(p, &t, fr, ph);
                }
                if let Some(st) = app.session.catalog.stack_of(*id) {
                    let text =
                        if st.collapsed { st.photos.len().to_string() } else { format!("{}/{}", st.position(*id).unwrap_or(0) + 1, st.photos.len()) };
                    let g = p.layout_no_wrap(text, t.semibold(9.5), Color32::WHITE);
                    let br = Rect::from_min_size(fr.min + vec2(3.0, 3.0), vec2(g.size().x + 22.0, 15.0));
                    p.rect_filled(br, 7.5, Color32::from_black_alpha(170));
                    crate::icons::paint(p, Rect::from_min_size(br.min + vec2(4.0, 2.0), vec2(11.0, 11.0)), crate::icons::Icon::Stack, Color32::WHITE);
                    p.galley(pos2(br.min.x + 17.0, br.center().y - g.size().y / 2.0), g, Color32::WHITE);
                }
            }
            if resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                let compare = app.ui.view == crate::state::ViewMode::Compare;
                let r = if compare && !m.command && !m.shift {
                    // Compare: the clicked photo replaces the active side (or swaps with the other one)
                    app.run("compare.set", json!({"id": id.0}))
                } else if compare {
                    // ⌘/Ctrl- and Shift-clicks change the selection; the pair on screen stays (#187)
                    super::compare::select_with_modifier(app, *id, !m.command)
                } else {
                    let mode = if m.command {
                        "toggle"
                    } else if m.shift {
                        "range"
                    } else {
                        "replace"
                    };
                    app.run("library.select", json!({"ids": [id.0], "mode": mode}))
                };
                if compare && let Err(e) = r {
                    app.toast(ui.ctx(), e);
                }
            }
            // the same photo actions as the grid and loupe (Restore / Delete Permanently in Recently Deleted)
            resp.context_menu(|ui| super::grid::context_menu(app, ui, *id));
        }
    });
    film_overview(app, ui, r, &ids, active);
}

/// Rating, flag and edited badges along a filmstrip thumbnail's bottom edge (Settings → Interface).
fn film_badges(p: &egui::Painter, t: &Tokens, fr: Rect, ph: &lightcraft_catalog::Photo) {
    use lightcraft_catalog::Flag;
    if ph.rating == 0 && ph.flag == Flag::None && !ph.is_edited() && ph.label.is_none() {
        return;
    }
    // as big as the strip under the picture would draw them (at least 18 points: readable at a glance)
    let h = (fr.height() * 0.2).clamp(18.0, 24.0).min(fr.height());
    let bar = Rect::from_min_max(pos2(fr.left(), fr.bottom() - h), fr.right_bottom());
    p.rect_filled(bar, 0.0, Color32::from_black_alpha(140));
    super::grid::paint_badges(p, t, bar.shrink2(vec2(2.0, 0.0)), ph, true);
}

#[cfg(test)]
mod preview_geometry_tests {
    use super::{fit_rect, fit_texture_rect};
    use crate::state::Zoom;
    use egui::{Rect, pos2, vec2};

    #[test]
    fn portrait_preview_keeps_its_ratio_inside_a_landscape_frame() {
        let frame = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let image = fit_texture_rect(frame, [100, 200]);

        assert!((image.width() / image.height() - 0.5).abs() < 1e-6);
        assert_eq!(image.center(), frame.center());
        assert!(frame.contains_rect(image));
    }

    /// A 100 % view must put the photo's corner on a whole physical pixel, or the GPU's bilinear
    /// filter softens it (a 1279.5 pt canvas on a 2x display centred the photo half a pixel off).
    #[test]
    fn percent_zoom_lands_on_whole_physical_pixels() {
        let area = Rect::from_min_size(pos2(0.0, 42.0), vec2(1279.5, 726.0));
        for ppp in [1.0, 1.25, 2.0, 3.0] {
            for pan in [(0.5, 0.5), (0.3, 0.7), (0.4999, 0.5001)] {
                let image = fit_rect(area, 5152.0 / 7728.0, Zoom::Percent(100.0), [5152, 7728], ppp, pan);
                for v in [image.min.x, image.min.y] {
                    assert!((v * ppp - (v * ppp).round()).abs() < 1e-3, "{v} pt is not a whole pixel at {ppp}x, pan {pan:?}");
                }
                assert!((image.width() * ppp - 5152.0).abs() < 1e-3, "size stays the photo's own pixels");
            }
        }
    }

    #[test]
    fn portrait_preview_aspect_fits_detail_area_without_stretching() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let image = fit_rect(area, 0.5, Zoom::Fit, [4000, 6000], 1.0, (0.5, 0.5));

        assert!((image.width() / image.height() - 0.5).abs() < 1e-6);
        assert_eq!(image.size(), vec2(100.0, 200.0));
        assert_eq!(image.center(), area.center());
    }
}

/// Straighten tool: drag along a horizon (or a vertical) to set the crop angle; double-click = Auto.
/// The image is shown unrotated in the crop view, so the line's on-screen angle is its image angle.
/// `held`: drawn with ⌘ held in the crop tool, which stays active afterwards.
fn straighten_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, held: bool) {
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if !held && resp.double_clicked() {
        let _ = app.run("crop.autoStraighten", json!({}));
        app.ui.tool.clear();
        app.gesture = None;
        return;
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        app.gesture = Some(Gesture::StraightenLine { a: q });
    }
    let Some(Gesture::StraightenLine { a }) = app.gesture.clone() else { return };
    let Some(b) = resp.interact_pointer_pos().or(resp.hover_pos()) else { return };
    let p = ui.painter();
    p.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
    p.line_segment([a, b], Stroke::new(1.5, Color32::WHITE));
    if resp.drag_stopped() {
        app.gesture = None;
        let v = b - a;
        if v.length() > 8.0 {
            let mut deg = v.y.atan2(v.x).to_degrees() as f64;
            // fold to the nearest axis: a line closer to vertical straightens plumb lines
            while deg > 45.0 {
                deg -= 90.0;
            }
            while deg < -45.0 {
                deg += 90.0;
            }
            let _ = app.run("crop.straighten", json!({"angle": (-deg * 100.0).round() / 100.0}));
        }
        if !held {
            app.ui.tool.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    /// The rotation pointer's arc bulges away from the side of the crop box it is on, and follows
    /// a box that is itself rotated.
    #[test]
    fn rotate_cursor_faces_away_from_the_crop_box() {
        use egui::{Pos2, pos2};
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let near = |a: f32, b: f32| ((a - b + PI).rem_euclid(2.0 * PI) - PI).abs() < 1e-3;
        let pts = [pos2(100.0, 100.0), pos2(300.0, 100.0), pos2(300.0, 200.0), pos2(100.0, 200.0)];
        let turn = |x: f32, y: f32| super::rotate_cursor_turn(&pts, pos2(x, y), false);
        assert!(near(turn(200.0, 80.0), 0.0), "above: as drawn");
        assert!(near(turn(320.0, 150.0), FRAC_PI_2), "right");
        assert!(near(turn(200.0, 230.0), PI), "below");
        assert!(near(turn(70.0, 150.0), -FRAC_PI_2), "left");
        assert!(near(turn(290.0, 60.0), 0.0), "above the top edge near its end: still up");
        // off each corner: diagonally out from it
        assert!(near(turn(80.0, 80.0), -FRAC_PI_4), "top-left corner");
        assert!(near(turn(320.0, 80.0), FRAC_PI_4), "top-right corner");
        assert!(near(turn(320.0, 220.0), 3.0 * FRAC_PI_4), "bottom-right corner");
        assert!(near(turn(80.0, 220.0), -3.0 * FRAC_PI_4), "bottom-left corner");
        assert!(near(super::rotate_cursor_turn(&pts, pos2(200.0, 110.0), true), 0.0), "dragged back over the top edge: still up");
        // a box turned 90°: its top edge is now on the right
        let turned = [pts[3], pts[0], pts[1], pts[2]].map(|p| pos2(400.0 - p.y, p.x));
        assert!(near(super::rotate_cursor_turn(&turned, pos2(420.0, 200.0), false), FRAC_PI_2));
        // a box turned 30°: straight out from its top edge is 30° clockwise of up
        let c = pos2(200.0, 150.0);
        let rot = |p: Pos2| {
            let (s, co) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
            let d = p - c;
            pos2(c.x + d.x * co - d.y * s, c.y + d.x * s + d.y * co)
        };
        let tilted = pts.map(rot);
        assert!(near(super::rotate_cursor_turn(&tilted, rot(pos2(200.0, 60.0)), false), 30f32.to_radians()));
    }

    use super::{film_label, flipped_corners, outside_crop_mesh, texture_side};
    use egui::{Color32, Pos2, Rect, pos2};

    /// A rotated `w` × `h` box centred on `centre`.
    fn box_at(centre: Pos2, w: f32, h: f32, angle: f32) -> [Pos2; 4] {
        let (s, c) = angle.to_radians().sin_cos();
        [(-w, -h), (w, -h), (w, h), (-w, h)].map(|(x, y)| pos2(centre.x + (x * c - y * s) / 2.0, centre.y + (x * s + y * c) / 2.0))
    }

    /// Signed distances from a convex polygon's edges, positive inwards (its winding either way).
    fn sides(poly: &[Pos2], p: Pos2) -> Vec<f32> {
        let n = poly.len() as f32;
        let centre = pos2(poly.iter().map(|q| q.x).sum::<f32>() / n, poly.iter().map(|q| q.y).sum::<f32>() / n);
        (0..poly.len())
            .map(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let nrm = egui::vec2(a.y - b.y, b.x - a.x).normalized();
                nrm.dot(p - a) * nrm.dot(centre - a).signum()
            })
            .collect()
    }

    /// Issue #731: the dimming is the photo minus the crop box at any angle. Every point of the
    /// photo outside the box lies in exactly one of the mesh's triangles; no point inside the box
    /// or outside the photo lies in any. (Four quads joining photo and box corners crossed and
    /// overlapped once the box was rotated.)
    #[test]
    fn the_dimming_outside_a_rotated_crop_is_exactly_the_photo_minus_the_box() {
        let img = Rect::from_min_max(pos2(100.0, 50.0), pos2(500.0, 650.0));
        // boxes inside the photo, poking out of it, and bigger than it
        for (w, h) in [(240.0, 360.0), (420.0, 300.0), (700.0, 900.0)] {
            for angle in [0.0f32, 5.0, 15.0, 30.0, -45.0, 90.0, 137.0, 200.0] {
                let quad = box_at(img.center() + egui::vec2(30.0, -20.0), w, h, angle);
                let mesh = outside_crop_mesh(img, &quad, Color32::BLACK);
                let tris: Vec<[Pos2; 3]> = mesh
                    .indices
                    .chunks(3)
                    .map(|t| [mesh.vertices[t[0] as usize].pos, mesh.vertices[t[1] as usize].pos, mesh.vertices[t[2] as usize].pos])
                    .collect();
                let (mut inside, mut outside) = (0, 0);
                let mut y = img.top() - 20.0;
                while y < img.bottom() + 20.0 {
                    let mut x = img.left() - 20.0;
                    while x < img.right() + 20.0 {
                        let p = pos2(x, y);
                        let in_box = sides(&quad, p);
                        let in_img = sides(&[img.left_top(), img.right_top(), img.right_bottom(), img.left_bottom()], p);
                        // a pixel on an edge may fall either way: skip the boundaries themselves
                        if in_box.iter().chain(&in_img).any(|d| d.abs() < 1.0) {
                            x += 7.0;
                            continue;
                        }
                        let expected = if in_img.iter().all(|d| *d > 0.0) && !in_box.iter().all(|d| *d > 0.0) { 1 } else { 0 };
                        // a sample on the seam between two of a strip's fan triangles is in both
                        // or in neither, depending on rounding: count it both ways
                        let covered = |eps: f32| tris.iter().filter(|t| sides(*t, p).iter().all(|d| *d > eps)).count();
                        let (at_least, at_most) = (covered(0.01), covered(-0.01));
                        assert!(
                            at_least <= expected && expected <= at_most,
                            "{w}×{h} box at {angle}°: ({x}, {y}) lies in {at_least}..={at_most} triangle(s), not {expected}"
                        );
                        if expected == 1 {
                            outside += 1
                        } else if in_img.iter().all(|d| *d > 0.0) {
                            inside += 1
                        }
                        x += 7.0;
                    }
                    y += 7.0;
                }
                // (a box bigger than the photo swallows it whole at some angles)
                assert!(inside > 50 && (outside > 50 || w > img.width()), "{w}×{h} at {angle}°: {outside} dimmed, {inside} clear samples");
            }
        }
    }

    /// A flat or broken box dims nothing rather than something arbitrary (or panicking).
    #[test]
    fn a_degenerate_crop_box_dims_nothing() {
        let img = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let point = [pos2(10.0, 10.0); 4];
        let line = [pos2(10.0, 10.0), pos2(90.0, 10.0), pos2(90.0, 10.0), pos2(10.0, 10.0)];
        let nan = [pos2(f32::NAN, 10.0), pos2(90.0, 10.0), pos2(90.0, 50.0), pos2(10.0, 50.0)];
        for quad in [point, line, nan] {
            assert!(outside_crop_mesh(img, &quad, Color32::BLACK).indices.is_empty(), "{quad:?}");
        }
        assert!(outside_crop_mesh(Rect::NAN, &box_at(pos2(50.0, 50.0), 20.0, 20.0, 10.0), Color32::BLACK).indices.is_empty());
    }

    /// A raw's embedded JPEG shown while the raw decodes, or the last render while an edit renders,
    /// looked like the finished photo: the loupe says "Rendering…" once a render has been coming for
    /// a moment with nothing new on screen, and doesn't blink while slow drafts arrive.
    #[test]
    fn the_loupe_says_rendering_when_the_screen_falls_behind() {
        use super::{RENDERING_AFTER_SECS as WAIT, rendering_overdue};
        let key = |k| [Some(k), None, None, None];
        let mut wait = None;
        assert!(!rendering_overdue(&mut wait, key(1), false, 0.0), "nothing coming");
        // an edit after an idle spell: the wait starts now, not at the last frame drawn
        assert!(!rendering_overdue(&mut wait, key(1), true, 30.0), "a render just asked for");
        assert!(!rendering_overdue(&mut wait, key(1), true, 30.0 + WAIT * 0.8), "a quick render never shows it");
        assert!(rendering_overdue(&mut wait, key(1), true, 30.0 + WAIT * 1.2), "nothing new for the wait");
        assert!(rendering_overdue(&mut wait, key(2), true, 31.0 + WAIT), "once up, a slow draft doesn't blink it");
        assert!(!rendering_overdue(&mut wait, key(3), false, 32.0), "done");
        // a drag whose drafts keep up: always something coming, a new draft every 0.1 s
        for i in 1..30u32 {
            assert!(!rendering_overdue(&mut wait, key(u64::from(i)), true, 40.0 + f64::from(i) * 0.1), "draft {i}");
        }
    }

    /// Issue #652: a native host reports the GPU's texture limit in the first frame's input only.
    /// The loupe must still know it in every later frame (it read the raw input, found nothing and
    /// cut every window render to 2048 px).
    #[test]
    fn the_texture_limit_outlives_the_frame_it_was_reported_in() {
        let ctx = egui::Context::default();
        assert_eq!(texture_side(&ctx), 2048, "until the host says: the smallest limit there is");
        let mut seen = Vec::new();
        for reported in [Some(8192), None, None] {
            let raw = egui::RawInput { max_texture_side: reported, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| seen.push(texture_side(ui.ctx())));
            out.textures_delta.clear();
        }
        assert_eq!(seen, [8192, 8192, 8192]);
    }

    #[test]
    fn film_labels_cut_on_characters_not_bytes() {
        // the name from #266: byte 13 falls inside '限'
        assert_eq!(film_label("202407層三限定訂閱圖(4)"), "202407層三限定訂閱圖…");
        assert_eq!(film_label("写真"), "写真");
        // exactly 14 characters (42 bytes) is shown whole
        assert_eq!(film_label("一二三四五六七八九十一二三四"), "一二三四五六七八九十一二三四");
        assert_eq!(film_label("IMG_20240712_153012"), "IMG_20240712_…");
        assert_eq!(film_label("DSC_0001"), "DSC_0001");
        assert_eq!(film_label(""), "");
    }

    /// With the photo flipped, the crop box's corners reach the screen mirrored; the dimming
    /// outside the box pairs each photo corner with the box corner on the same side of the screen
    /// (it used to pair them in data order, so a flipped photo was dimmed inside the box).
    #[test]
    fn flipped_crop_box_corners_come_in_screen_order() {
        let screen = [pos2(10.0, 10.0), pos2(90.0, 10.0), pos2(90.0, 50.0), pos2(10.0, 50.0)];
        // data order (top-left, top-right, bottom-right, bottom-left) as each flip shows it
        let h = [screen[1], screen[0], screen[3], screen[2]];
        let v = [screen[3], screen[2], screen[1], screen[0]];
        let hv = [screen[2], screen[3], screen[0], screen[1]];
        assert_eq!(flipped_corners(&screen, false, false), screen);
        assert_eq!(flipped_corners(&h, true, false), screen);
        assert_eq!(flipped_corners(&v, false, true), screen);
        assert_eq!(flipped_corners(&hv, true, true), screen);
    }
}

#[cfg(test)]
mod wheel_zoom_tests {
    use super::*;

    fn wheel(unit: egui::MouseWheelUnit, y: f32, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::MouseWheel { unit, delta: egui::vec2(0.0, y), phase: egui::TouchPhase::Move, modifiers }
    }

    #[test]
    fn only_notched_unmodified_wheels_count() {
        let m = egui::Modifiers::NONE;
        assert_eq!(wheel_notches(&[wheel(egui::MouseWheelUnit::Line, 1.0, m), wheel(egui::MouseWheelUnit::Line, 2.0, m)]), 3.0);
        assert_eq!(wheel_notches(&[wheel(egui::MouseWheelUnit::Point, 40.0, m)]), 0.0);
        assert_eq!(wheel_notches(&[wheel(egui::MouseWheelUnit::Line, 1.0, egui::Modifiers::CTRL)]), 0.0);
        assert_eq!(wheel_notches(&[wheel(egui::MouseWheelUnit::Line, f32::NAN, m)]), 0.0);
    }
}

/// The pen pressure (0..1) reported this frame: the force of the latest touch or pen event that carries one (winit
/// reports a pen's pressure as a touch force, on the platforms that give it), `None` for a mouse.
pub(crate) fn pen_force(i: &egui::InputState) -> Option<f32> {
    i.events.iter().rev().find_map(|e| match e {
        egui::Event::Touch { force: Some(f), .. } if f.is_finite() => Some(f.clamp(0.0, 1.0)),
        _ => None,
    })
}

/// Record the pressure of the stroke's point `index`: `force` when the pen reported one this frame, else the last
/// known one. Points before the first reported force take it too; a stroke without any keeps no pressure list.
pub(crate) fn record_pressure(pressure: &mut Vec<f32>, index: usize, force: Option<f32>) {
    match (force, pressure.last().copied()) {
        (Some(f), _) if pressure.is_empty() => pressure.extend(std::iter::repeat_n(f, index + 1)),
        (Some(f), _) => pressure.push(f),
        (None, Some(last)) => pressure.push(last),
        (None, None) => {}
    }
}

#[cfg(test)]
mod pen_tests {
    use super::*;

    #[test]
    fn pen_pressure_is_recorded_per_point_and_absent_for_a_mouse() {
        let mut p = Vec::new();
        record_pressure(&mut p, 0, None);
        record_pressure(&mut p, 1, None);
        assert!(p.is_empty(), "a mouse: no pressure list");
        // the first force covers the points before it, a frame without one repeats the last
        record_pressure(&mut p, 2, Some(0.4));
        record_pressure(&mut p, 3, None);
        record_pressure(&mut p, 4, Some(0.9));
        assert_eq!(p, vec![0.4, 0.4, 0.4, 0.4, 0.9]);

        let mut i = egui::InputState::default();
        assert_eq!(pen_force(&i), None);
        let touch = |force| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase: egui::TouchPhase::Move,
            pos: egui::pos2(1.0, 1.0),
            force,
        };
        i.events = vec![touch(Some(0.3)), touch(Some(f32::NAN)), touch(None)];
        assert_eq!(pen_force(&i), Some(0.3), "the latest usable force");
        i.events = vec![touch(Some(4.0))];
        assert_eq!(pen_force(&i), Some(1.0));
    }
}

#[cfg(test)]
mod filter_pill_tests {
    use super::*;

    /// #1645: the filter pill sits in the loupe's top-left corner as far from the left edge as
    /// from the top one.
    #[test]
    fn the_filter_pill_is_inset_equally_from_both_edges() {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), crate::Services { png: None, ..Default::default() });
        app.session.filter.text = "717".into();
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(pos2(52.0, 82.0), vec2(800.0, 500.0));
        let raw = crate::headless::HeadlessView::raw_input(vec2(1000.0, 700.0), 1.0, 0.0, vec![]);
        let mut out = ctx.run_ui(raw, |ui| filter_pill(&mut app, ui, canvas));
        out.textures_delta.clear();
        let widgets = crate::widgets::take_registry(&ctx);
        let pill = widgets.iter().find(|(w, _)| w == "loupe:filters").map(|(_, r)| *r).expect("the pill shows with a search");
        let (left, top) = (pill.left() - canvas.left(), pill.top() - canvas.top());
        assert!(top > 0.0, "inside the canvas: {pill:?}");
        assert_eq!(left, top, "left and top insets: {pill:?} in {canvas:?}");
    }
}

#[cfg(test)]
mod shot_info_tests {
    use super::*;

    /// The strip shows aperture, shutter, ISO, focal length, lens and size, from the metadata the
    /// probe already read; a lens without a model name is named by its focal range.
    #[test]
    fn shot_info_reads_the_metadata_in_order() {
        let mut m = lightcraft_catalog::Meta {
            aperture: Some(2.8),
            shutter: "1/250".into(),
            iso: Some(1600),
            focal_mm: Some(50.4),
            lens: "RF24-70mm F2.8 L IS USM".into(),
            lens_range: Some((24.0, 70.0)),
            ..Default::default()
        };
        assert_eq!(shot_info(&m, [8192, 5464]), ["f/2.8", "1/250 s", "ISO 1600", "50 mm", "RF24-70mm F2.8 L IS USM", "8192 × 5464"]);
        m.lens.clear();
        assert_eq!(shot_info(&m, [8192, 5464])[4], "(24–70 mm)");
        m.lens_range = Some((85.0, 85.0));
        m.aperture = Some(11.0);
        assert_eq!(shot_info(&m, [100, 50])[..1], ["f/11"]);
        assert_eq!(shot_info(&m, [100, 50])[4], "(85 mm)");
        // nothing known: nothing shown; hostile numbers are left out
        let none = lightcraft_catalog::Meta { aperture: Some(f32::NAN), focal_mm: Some(-1.0), ..Default::default() };
        assert!(shot_info(&none, [1, 1]).is_empty());
    }

    /// The strip's items are spaced by a wide gap; one item has none.
    #[test]
    fn shot_info_items_are_spaced() {
        assert_eq!(strip_width(&[]), 0.0);
        assert_eq!(strip_width(&[40.0]), 40.0);
        assert_eq!(strip_width(&[40.0, 30.0, 20.0]), 90.0 + 2.0 * STRIP_GAP);
    }

    /// A resized filmstrip keeps its cells' shape and asks for thumbnails its cells can show.
    #[test]
    fn filmstrip_cells_follow_its_height() {
        assert_eq!(film_cell_width(142.0), 120.0);
        assert!(film_cell_width(360.0) > 300.0 && film_cell_width(96.0) < 90.0);
        assert_eq!(film_cell_width(f32::NAN), 120.0);
        assert_eq!(film_thumb_px(120.0, 1.0), 240);
        assert_eq!(film_thumb_px(300.0, 2.0), 512);
        assert_eq!(film_thumb_px(60.0, f32::NAN), 128);
        let s = crate::state::FILM_HEIGHT;
        assert_eq!((s.clamp(10.0), s.clamp(1e6), s.clamp(f32::NAN)), (s.min, s.max, s.default));
    }
}

#[cfg(test)]
mod info_overlay_tests {
    use super::*;

    /// #1690: the info overlay's shadow is a dark copy under the text. Drawn as a second white
    /// copy one point off, each line read doubled.
    #[test]
    fn the_info_overlay_shadow_is_dark() {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), crate::Services { png: None, ..Default::default() });
        app.ui.info_overlay = crate::state::InfoOverlay::Basic;
        let photo = app.session.catalog.photos().next().map(|p| (**p).clone()).expect("the demo catalog has photos");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let canvas = Rect::from_min_size(pos2(52.0, 82.0), vec2(800.0, 500.0));
        let raw = crate::headless::HeadlessView::raw_input(vec2(1000.0, 700.0), 1.0, 0.0, vec![]);
        let mut out = ctx.run_ui(raw, |ui| info_overlay(&app, ui.painter(), canvas, &photo));
        out.textures_delta.clear();
        // the colour a text shape paints its glyphs in, as epaint resolves it
        let ink = |s: &egui::epaint::TextShape| {
            let c = s.galley.job.sections.first().map_or(Color32::PLACEHOLDER, |sec| sec.format.color);
            s.override_text_color.unwrap_or(if c == Color32::PLACEHOLDER { s.fallback_color } else { c })
        };
        fn texts(shape: &egui::epaint::Shape, out: &mut Vec<egui::epaint::TextShape>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.clone()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut all));
        let copies: Vec<Color32> = all.iter().filter(|t| t.galley.job.text == photo.file_name).map(ink).collect();
        assert_eq!(copies.len(), 2, "the file name and its shadow: {copies:?}");
        assert_eq!(copies[1], Color32::WHITE, "the text on top");
        assert!(copies[0].r() < 64 && copies[0].a() > 0, "the shadow under it is dark: {:?}", copies[0]);
    }
}

#[cfg(test)]
#[path = "detail_tools_tests.rs"]
mod tools_tests;
