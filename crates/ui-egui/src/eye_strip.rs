//! The eye focus strip (`view.eyeFocus`): across the top of the loupe, each person's sharper eye at
//! 100 %, left to right as they stand, with a dot saying how sharp it is (see
//! [`lightcraft_engine::eyes`]). A click shows that eye at 100 % in the loupe.
//!
//! The eyes are found on the eye finder's own thread for the photo on screen first, then for the
//! next ones in the direction of travel, and kept per photo, so stepping never waits for them:
//! the strip appears when its photo's result is there.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_engine::eyes::{EyeReport, EyeScanner, Focus, Lids};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// Results kept (a few crops each: about 50 kB a photo).
const KEEP: usize = 64;
/// Photos after the one on screen whose eyes are found ahead.
pub const AHEAD: usize = 3;

/// What the strip keeps between frames.
#[derive(Default)]
pub struct EyeStrip {
    scanner: Option<EyeScanner>,
    /// Result per job key, and when it was last wanted.
    results: HashMap<u64, (Result<Arc<EyeReport>, String>, u64)>,
    tick: u64,
    /// The keys last handed to the scanner, in order.
    wanted: Vec<u64>,
    /// The crops of the photo on screen, as textures.
    textures: Option<(u64, Vec<egui::TextureHandle>)>,
}

impl std::fmt::Debug for EyeStrip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EyeStrip").field("results", &self.results.len()).field("wanted", &self.wanted).finish()
    }
}

/// What the strip shows for a photo.
#[derive(Debug, PartialEq)]
pub enum Shown {
    Pending,
    Failed(String),
    NoFaces,
    /// Faces, but none with an eye that can be judged.
    NoEyes(usize),
    Eyes(usize),
}

impl EyeStrip {
    /// The state for `ui.inspect`.
    pub fn to_json(&self, key: Option<u64>) -> serde_json::Value {
        let r = key.and_then(|k| self.results.get(&k)).map(|(r, _)| r);
        serde_json::json!({
            "pending": key.is_some_and(|k| !self.results.contains_key(&k)),
            "error": r.and_then(|r| r.as_ref().err()),
            "faces": r.and_then(|r| r.as_ref().ok()).map(|r| r.faces),
            "ms": r.and_then(|r| r.as_ref().ok()).map(|r| r.ms),
            "eyes": r.and_then(|r| r.as_ref().ok()).map(|r| r.eyes.iter().map(|e| serde_json::json!({
                "center": e.center, "sharpness": e.sharpness, "focus": e.focus, "bothEyes": e.both_eyes,
                "lids": e.lids, "openness": e.openness, "good": e.good(),
            })).collect::<Vec<_>>()),
            "blinks": r.and_then(|r| r.as_ref().ok()).map(|r| r.blinks()),
            "cached": self.results.len(),
        })
    }

    /// Take in finished results and ask for `jobs` (the photo on screen first). `model` is where
    /// the model is installed.
    fn update(&mut self, model: std::path::PathBuf, jobs: Vec<lightcraft_engine::eyes::EyeJob>) {
        if self.scanner.as_ref().is_none_or(|s| s.model_dir() != model) {
            self.scanner = EyeScanner::start(model);
            self.wanted.clear();
        }
        self.tick += 1;
        if let Some(s) = &self.scanner {
            for (key, _, r) in s.finished() {
                self.results.insert(key, (r, self.tick));
            }
        }
        for j in &jobs {
            if let Some(e) = self.results.get_mut(&j.key) {
                e.1 = self.tick;
            }
        }
        let todo: Vec<_> = jobs.into_iter().filter(|j| !self.results.contains_key(&j.key)).collect();
        let keys: Vec<u64> = todo.iter().map(|j| j.key).collect();
        if keys != self.wanted
            && let Some(s) = &self.scanner
        {
            s.want(todo);
            self.wanted = keys;
        }
        self.trim();
    }

    /// Keep the [`KEEP`] results wanted last.
    fn trim(&mut self) {
        if self.results.len() > KEEP {
            let mut ages: Vec<(u64, u64)> = self.results.iter().map(|(k, v)| (v.1, *k)).collect();
            ages.sort_unstable();
            for (_, k) in ages.into_iter().take(self.results.len() - KEEP) {
                self.results.remove(&k);
            }
        }
    }

    fn shown(&self, key: u64) -> Shown {
        match self.results.get(&key).map(|(r, _)| r) {
            None => Shown::Pending,
            Some(Err(e)) => Shown::Failed(e.clone()),
            Some(Ok(r)) if r.eyes.is_empty() && r.faces == 0 => Shown::NoFaces,
            Some(Ok(r)) if r.eyes.is_empty() => Shown::NoEyes(r.no_eyes.max(1)),
            Some(Ok(r)) => Shown::Eyes(r.eyes.len()),
        }
    }
}

/// The strip's colour for a focus.
pub fn focus_colour(f: Focus) -> Color32 {
    match f {
        Focus::Sharp => Color32::from_rgb(0x3f, 0xb9, 0x50),
        Focus::Soft => Color32::from_rgb(0xe3, 0xa0, 0x08),
        Focus::Blurry => Color32::from_rgb(0xf8, 0x51, 0x49),
    }
}

/// The closed-eye marker's word, `None` for an open eye (or one whose lids couldn't be measured).
fn lids_word(l: Lids) -> Option<&'static str> {
    match l {
        Lids::Closed => Some(crate::i18n::tr("Closed")),
        Lids::Half => Some(crate::i18n::tr("Half closed")),
        Lids::Open | Lids::Unknown => None,
    }
}

/// A small closed eye (a lid's curve with lashes) at `c`, `w` wide.
fn closed_eye_icon(p: &egui::Painter, c: egui::Pos2, w: f32, colour: Color32) {
    let pts: Vec<egui::Pos2> = (0..=8)
        .map(|i| {
            let t = i as f32 / 8.0;
            pos2(c.x - w / 2.0 + t * w, c.y - (std::f32::consts::PI * t).sin() * w * 0.22 + w * 0.08)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.4, colour)));
    for t in [0.25f32, 0.5, 0.75] {
        let x = c.x - w / 2.0 + t * w;
        let y = c.y - (std::f32::consts::PI * t).sin() * w * 0.22 + w * 0.08;
        p.line_segment([pos2(x, y), pos2(x, y + w * 0.2)], Stroke::new(1.2, colour));
    }
}

fn focus_word(f: Focus) -> &'static str {
    match f {
        Focus::Sharp => crate::i18n::tr("Sharp"),
        Focus::Soft => crate::i18n::tr("Soft"),
        Focus::Blurry => crate::i18n::tr("Blurry"),
    }
}

/// Lay out `n` thumbnails `w × h` (points) with `gap` between them in `width`: their size, scaled
/// down (never up) so they fit.
pub fn fit_thumbs(n: usize, w: f32, h: f32, gap: f32, width: f32) -> (f32, f32) {
    if n == 0 || !(w > 0.0 && h > 0.0) || !width.is_finite() {
        return (w.max(0.0), h.max(0.0));
    }
    let need = n as f32 * w + (n as f32 + 1.0) * gap;
    let s = ((width - (n as f32 + 1.0) * gap).max(1.0) / (n as f32 * w)).min(1.0);
    if need <= width { (w, h) } else { (w * s, h * s) }
}

/// Draw the strip across the top of `canvas` for photo `id` (`upcoming`: the photos likely next).
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, id: PhotoId, upcoming: &[PhotoId]) {
    let t = Tokens::get(ui.ctx());
    let Some(model) = app.session.eye_model_dir() else {
        needs_model(app, ui, canvas);
        return;
    };
    let Some(job) = app.session.eye_job(id) else { return };
    let key = job.key;
    let mut jobs = vec![job];
    jobs.extend(upcoming.iter().take(AHEAD).filter_map(|p| app.session.eye_job(*p)));
    app.eye_strip.update(model, jobs);
    app.eye_key = Some(key);
    let shown = app.eye_strip.shown(key);
    let pill = |ui: &egui::Ui, text: String| {
        let p = ui.painter_at(canvas);
        let g = p.layout_no_wrap(text, t.font(12.0), Color32::from_gray(225));
        let r = Rect::from_center_size(pos2(canvas.center().x, canvas.top() + 20.0), g.size() + vec2(20.0, 8.0));
        p.rect_filled(r, 10.0, Color32::from_black_alpha(170));
        p.galley(r.min + vec2(10.0, 4.0), g, Color32::from_gray(225));
        register(ui.ctx(), "eyes:status", r);
    };
    match shown {
        Shown::Pending => {
            pill(ui, crate::i18n::tr("Finding eyes…").to_string());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        }
        Shown::Failed(e) => pill(ui, crate::i18n::tr_format!("Eye focus: {}", e)),
        Shown::NoFaces => pill(ui, crate::i18n::tr("No faces").to_string()),
        Shown::NoEyes(_) => pill(ui, crate::i18n::tr("No eye to judge (faces turned away)").to_string()),
        Shown::Eyes(_) => thumbs(app, ui, canvas, key),
    }
}

/// The "needs the model" strip, with its button.
fn needs_model(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect) {
    let t = Tokens::get(ui.ctx());
    let text = crate::i18n::tr("Eye focus needs the face AI model");
    let g = ui.painter().layout_no_wrap(text.to_string(), t.font(12.0), Color32::from_gray(225));
    let r = Rect::from_center_size(pos2(canvas.center().x, canvas.top() + 22.0), vec2(g.size().x + 200.0, 30.0));
    ui.painter_at(canvas).rect_filled(r, 8.0, Color32::from_black_alpha(170));
    ui.painter_at(canvas).galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, Color32::from_gray(225));
    let b = Rect::from_min_max(pos2(r.right() - 176.0, r.top() + 4.0), pos2(r.right() - 6.0, r.bottom() - 4.0));
    register(ui.ctx(), "eyes:getModel", b);
    if ui.put(b, egui::Button::new(crate::i18n::tr("Get the AI Model…")).small()).clicked() {
        app.offer_ai_model("face", None);
    }
}

fn thumbs(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, key: u64) {
    let t = Tokens::get(ui.ctx());
    let Some(Ok(report)) = app.eye_strip.results.get(&key).map(|(r, _)| r.clone()) else { return };
    if app.eye_strip.textures.as_ref().is_none_or(|(k, _)| *k != key) {
        let tex = report
            .eyes
            .iter()
            .enumerate()
            .map(|(i, e)| ui.ctx().load_texture(format!("eye-{key}-{i}"), crate::render::color_image(&e.crop), egui::TextureOptions::NEAREST))
            .collect();
        app.eye_strip.textures = Some((key, tex));
    }
    let Some((_, tex)) = app.eye_strip.textures.clone() else { return };
    let ppp = ui.ctx().pixels_per_point().max(0.5);
    let n = report.eyes.len();
    let (gap, label_h) = (8.0, 15.0);
    // one photo pixel per screen pixel, unless that doesn't fit
    let (w, h) =
        fit_thumbs(n, lightcraft_engine::eyes::CROP_W as f32 / ppp, lightcraft_engine::eyes::CROP_H as f32 / ppp, gap, canvas.width() - 24.0);
    // what the thumbnails don't show, beside them inside the band
    let mut notes = Vec::new();
    if report.blinks() > 0 {
        notes.push(crate::i18n::tr_format!("{} with eyes closed", report.blinks()));
    }
    if report.no_eyes > 0 {
        notes.push(crate::i18n::tr_format!("+{} turned away", report.no_eyes));
    }
    let notes: Vec<_> = notes.into_iter().map(|n| ui.painter().layout_no_wrap(n, t.font(10.5), Color32::from_gray(215))).collect();
    let notes_w = notes.iter().map(|g| g.size().x).fold(0.0f32, f32::max);
    let band_w = n as f32 * w + (n as f32 + 1.0) * gap + if notes.is_empty() { 0.0 } else { notes_w + gap };
    let band = Rect::from_min_size(pos2(canvas.center().x - band_w / 2.0, canvas.top() + 6.0), vec2(band_w, h + label_h + gap * 1.5));
    let p = ui.painter_at(canvas);
    p.rect_filled(band, 8.0, Color32::from_black_alpha(175));
    register(ui.ctx(), "eyes:strip", band);
    let lines = notes.len() as f32;
    for (k, g) in notes.into_iter().enumerate() {
        let y = band.center().y - (lines - 1.0) * 8.0 + k as f32 * 16.0 - g.size().y / 2.0;
        p.galley(pos2(band.right() - gap - notes_w, y), g, Color32::from_gray(215));
    }
    for (i, (e, tx)) in report.eyes.iter().zip(&tex).enumerate() {
        let r = Rect::from_min_size(pos2(band.left() + gap + i as f32 * (w + gap), band.top() + gap), vec2(w, h));
        // the crop is shown whole (smaller at the picture's edges: centred in its place)
        let [cw, ch] = tx.size();
        let s = (w / (cw as f32 / ppp)).min(h / (ch as f32 / ppp)).min(1.0);
        let img = Rect::from_center_size(r.center(), vec2(cw as f32 / ppp * s, ch as f32 / ppp * s));
        p.image(tx.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let c = focus_colour(e.focus);
        p.rect_stroke(r, 3.0, Stroke::new(2.0, c), StrokeKind::Outside);
        p.circle_filled(pos2(r.right() - 7.0, r.top() + 7.0), 4.5, c);
        p.circle_stroke(pos2(r.right() - 7.0, r.top() + 7.0), 4.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
        p.text(
            pos2(r.center().x, r.bottom() + 9.0),
            Align2::CENTER_CENTER,
            format!("{} {:.0}", focus_word(e.focus), e.sharpness),
            t.font(10.5),
            Color32::from_gray(220),
        );
        // closed or half closed: a marker of its own, apart from the sharpness colour
        if let Some(word) = lids_word(e.lids) {
            let g = p.layout_no_wrap(word.to_string(), t.semibold(9.5), Color32::WHITE);
            let pill = Rect::from_min_size(pos2(r.left() + 3.0, r.bottom() - 17.0), vec2(g.size().x + 26.0, 14.0));
            let fill = if e.lids == Lids::Closed {
                Color32::from_rgba_unmultiplied(120, 40, 160, 230)
            } else {
                Color32::from_rgba_unmultiplied(90, 60, 140, 210)
            };
            p.rect_filled(pill, 7.0, fill);
            closed_eye_icon(&p, pos2(pill.left() + 10.0, pill.center().y), 11.0, Color32::WHITE);
            p.galley(pos2(pill.left() + 19.0, pill.center().y - g.size().y / 2.0), g, Color32::WHITE);
            register(ui.ctx(), format!("eyes:closed:{i}"), pill);
        }
        // the person's verdict: sharp and open is a keeper
        let v = pos2(r.left() + 8.0, r.top() + 8.0);
        if e.good() {
            p.circle_filled(v, 6.0, Color32::from_black_alpha(150));
            p.line_segment([v + vec2(-3.0, 0.0), v + vec2(-1.0, 2.5)], Stroke::new(1.6, focus_colour(Focus::Sharp)));
            p.line_segment([v + vec2(-1.0, 2.5), v + vec2(3.5, -2.5)], Stroke::new(1.6, focus_colour(Focus::Sharp)));
        }
        let id = format!("eyes:thumb:{i}");
        register(ui.ctx(), &id, r);
        let resp = ui.interact(r, egui::Id::new(("eye-thumb", i)), Sense::click()).on_hover_text(crate::i18n::tr("Click to see this eye at 100 %"));
        if resp.clicked() {
            zoom_to(app, e.center);
        }
    }
}

/// Show the point `center` (normalised to the picture) at 100 % in the loupe.
pub fn zoom_to(app: &mut LightcraftApp, center: [f32; 2]) {
    if center.iter().all(|v| v.is_finite()) {
        app.ui.zoom = crate::state::Zoom::Percent(100.0);
        app.ui.pan = (center[0].clamp(0.0, 1.0), center[1].clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_shrink_to_fit_and_never_grow() {
        assert_eq!(fit_thumbs(3, 160.0, 100.0, 8.0, 2000.0), (160.0, 100.0));
        let (w, h) = fit_thumbs(8, 160.0, 100.0, 8.0, 800.0);
        assert!(w < 160.0 && (w / h - 1.6).abs() < 0.01);
        assert!(8.0 * w + 9.0 * 8.0 <= 800.5);
        assert_eq!(fit_thumbs(0, 160.0, 100.0, 8.0, 10.0), (160.0, 100.0));
        let (w, _) = fit_thumbs(4, 160.0, 100.0, 8.0, f32::NAN);
        assert_eq!(w, 160.0);
    }

    #[test]
    fn a_click_zooms_to_the_eye_and_junk_is_ignored() {
        let mut app = crate::LightcraftApp::new(lightcraft_engine::Session::new(), crate::Services::default());
        zoom_to(&mut app, [0.25, 0.75]);
        assert_eq!((app.ui.zoom, app.ui.pan), (crate::state::Zoom::Percent(100.0), (0.25, 0.75)));
        zoom_to(&mut app, [f32::NAN, 0.1]);
        assert_eq!(app.ui.pan, (0.25, 0.75));
    }

    #[test]
    fn results_are_kept_per_photo_and_bounded() {
        let mut s = EyeStrip::default();
        assert_eq!(s.shown(1), Shown::Pending);
        for k in 0..(KEEP as u64 + 10) {
            s.tick += 1;
            let r = EyeReport { photo: PhotoId(k), eyes: Vec::new(), faces: (k % 2) as usize, no_eyes: (k % 2) as usize, size: (1, 1), ms: 0.0 };
            s.results.insert(k, (Ok(Arc::new(r)), s.tick));
        }
        s.trim();
        assert_eq!(s.results.len(), KEEP);
        assert!(!s.results.contains_key(&0), "the oldest go first");
        assert_eq!(s.shown(KEEP as u64 + 8), Shown::NoFaces);
        assert_eq!(s.shown(KEEP as u64 + 9), Shown::NoEyes(1));
        s.results.insert(7, (Err("no model".into()), s.tick));
        assert_eq!(s.shown(7), Shown::Failed("no model".into()));
    }
}
