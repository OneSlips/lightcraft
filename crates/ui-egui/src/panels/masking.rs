//! The Masking panel: create masks, list them, edit components and local adjustments.

use egui::{Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{ControlSpec, LocalAdjustments, MaskShape, Section, Track};
use serde_json::json;

use super::edit::apply_slider_out;
use super::right::header;
use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{divider, one_line, register, slider, text_button};

const fn spec(id: &'static str, label: &'static str, min: f64, max: f64, step: f64, decimals: u8, track: Track) -> ControlSpec {
    ControlSpec { id, label, section: Section::Light, min, max, default: 0.0, step, decimals, track }
}

/// Local adjustment sliders (key = `LocalAdjustments` field name).
pub const LOCAL: &[ControlSpec] = &[
    spec("temp", "Temp", -100.0, 100.0, 1.0, 0, Track::Temp),
    spec("tint", "Tint", -100.0, 100.0, 1.0, 0, Track::Tint),
    spec("exposure", "Exposure", -4.0, 4.0, 0.01, 2, Track::Centered),
    spec("contrast", "Contrast", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("highlights", "Highlights", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("shadows", "Shadows", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("whites", "Whites", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("blacks", "Blacks", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("texture", "Texture", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("clarity", "Clarity", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("dehaze", "Dehaze", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("hue", "Hue", -100.0, 100.0, 1.0, 0, Track::Rainbow),
    spec("saturation", "Saturation", -100.0, 100.0, 1.0, 0, Track::Gradient { from: "#7a7a7a", to: "#e04a3a" }),
    spec("sharpness", "Sharpness", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("noise", "Noise", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("moire", "Moiré", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("defringe", "Defringe", -100.0, 100.0, 1.0, 0, Track::Centered),
];

pub fn local_get(a: &LocalAdjustments, key: &str) -> f64 {
    serde_json::to_value(a).ok().and_then(|v| v.get(key).and_then(|x| x.as_f64())).unwrap_or(0.0)
}

const TILE: f32 = 52.0;
const TILE_GAP: f32 = 5.0;
/// Height of a Create New Mask tile.
const TILE_H: f32 = 44.0;
/// Room left and right of a tile's label.
const TILE_PAD: f32 = 3.0;

/// Columns, tile width and label size of the Create New Mask grid in `width`, `label(size)` being
/// the widest label at a font size: four tiles of up to 52 pt (at least 48), else three, else two —
/// the first that fits the labels at 10.5 pt, or at 9.5 pt before a column is dropped. A tile grows
/// past 52 pt for a long (translated) label. In a panel too narrow even for that, two tiles whose
/// labels are cut short.
fn tile_layout(width: f32, label: impl Fn(f32) -> f32) -> (usize, f32, f32) {
    let room = |n: usize| ((width - (n as f32 - 1.0) * TILE_GAP) / n as f32).floor();
    for n in [4, 3, 2] {
        let w = room(n);
        if n == 4 && w < 48.0 {
            continue;
        }
        for size in [10.5, 9.5] {
            let need = (label(size) + 2.0 * TILE_PAD).ceil();
            if need <= w {
                return (n, w.min(TILE.max(need)), size);
            }
        }
    }
    (2, room(2).max(24.0), 9.5)
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    let d = app.session.develop_of(id).unwrap_or_default();
    header(ui, "Masking");
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 0, bottom: 10 }).show(ui, |ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Create New Mask")).color(t.text_dim));
        ui.add_space(6.0);
        let tiles = MASK_KINDS;
        // four 52 pt tiles a row when they and their labels fit, else fewer (wider for a long label)
        let widest = |size: f32| {
            let font = t.font(size);
            tiles
                .iter()
                .map(|(_, l, _)| ui.painter().layout_no_wrap(crate::i18n::tr(l).to_string(), font.clone(), t.text_dim).size().x)
                .fold(0.0, f32::max)
        };
        let (cols, tile, size) = tile_layout(ui.available_width(), widest);
        egui::Grid::new("mask-tiles").spacing(vec2(TILE_GAP, TILE_GAP)).show(ui, |ui| {
            for (i, (kind, label, icon)) in tiles.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(tile, TILE_H), Sense::click());
                crate::widgets::announce(&resp, crate::i18n::tr(label));
                register(ui.ctx(), format!("maskNew:{kind}"), r);
                // an armed tool (waiting for the photo), or an open picker, shows pressed
                let armed = tile_active(app, kind);
                ui.painter().rect_filled(
                    r,
                    4.0,
                    if armed {
                        t.tool_active
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        t.inset
                    },
                );
                paint(ui.painter(), Rect::from_center_size(r.center() - vec2(0.0, 6.0), vec2(17.0, 17.0)), *icon, t.text_label);
                // (cut short only in a panel too narrow for the label: the full name on hover)
                let label = crate::i18n::tr(label);
                let g = one_line(ui.painter(), label, t.font(size), t.text_dim, tile - 2.0 * TILE_PAD);
                let elided = g.elided;
                let at = pos2(r.center().x, r.bottom() - 8.0) - g.size() / 2.0;
                register(ui.ctx(), format!("maskNewLabel:{kind}"), Rect::from_min_size(at, g.size()));
                ui.painter().galley(at, g, t.text_dim);
                let resp = if elided { resp.on_hover_text(label) } else { resp };
                if resp.clicked() {
                    start_kind(app, ui.ctx(), kind, "new");
                }
                if i % cols == cols - 1 {
                    ui.end_row();
                }
            }
        });
        describe_field(app, ui, true);
        // (a picker opened from Add / Subtract / Intersect shows under the selected mask instead)
        if app.ui.mask_pick_op.is_none() {
            pickers(app, ui);
        }
        ai_notes(app, ui);
        let seg = &app.session.segmenter;
        let status = if seg.analyzing() {
            Some("Analyzing the photo for AI masks…")
        } else if seg.busy() {
            Some("Selecting…")
        } else if seg.detail_busy() || app.ui.detail_due.is_some() {
            Some("Refining the mask's detail…")
        } else {
            None
        };
        if let Some(status) = status {
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(status)).color(t.text_dim)).wrap());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        // the model download, while its dialog is closed
        let download = seg.download_status();
        if download.running && app.ui.dialog.is_none() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let pct = if download.total > 0 { download.done as f64 / download.total as f64 } else { 0.0 };
                let r = ui.add(
                    egui::ProgressBar::new(pct as f32).desired_width(ui.available_width() - 70.0).text(format!("SAM 3: {} %", (pct * 100.0).floor())),
                );
                register(ui.ctx(), "maskSamProgress", r.rect);
                if text_button(ui, "maskSamDetails", crate::i18n::tr("Details"), false).clicked() {
                    app.offer_sam_download(None);
                }
            });
        }
    });
    divider(ui);
    // the masks as a tree: each mask, the selected one's components, Add / Subtract / Intersect
    egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
        super::mask_tree::show(app, ui, id, &d);
    });
    // Reset Settings on New Mask follows the remembered choice (also after a restart)
    if app.session.mask_reset_new != app.ui.mask_reset_new {
        let _ = app.run("mask.next", json!({"resetNew": app.ui.mask_reset_new}));
    }
    let selected = app.session.active_mask.and_then(|mid| d.masks.iter().find(|m| m.id == mid).cloned());
    if app.ui.tool == "brush" {
        divider(ui);
        brush_settings(app, ui);
    }
    divider(ui);
    // no mask selected: the effect and sliders still show and work, for the next new mask (set
    // the look first, then draw)
    let shown = match &selected {
        Some(m) => m.clone(),
        None => {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 4, bottom: 0 }).show(ui, |ui| {
                let r =
                    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr("Settings for the next mask")).color(t.text_dim).size(11.5)).wrap());
                register(ui.ctx(), "maskNextSettings", r.rect);
            });
            // (id 0: no mask; the effect menu then sets the next mask's settings)
            lightcraft_develop::Mask { id: 0, adjust: app.session.mask_next, ..Default::default() }
        }
    };
    super::mask_effects::effect_row(app, ui, &shown);
    reset_new_row(app, ui);
    // how strongly the mask's adjustments apply (Lightroom's Amount, Capture One's layer opacity)
    let amt = ControlSpec {
        id: "amount",
        label: "Amount",
        section: Section::Light,
        min: 0.0,
        max: 200.0,
        default: 100.0,
        step: 1.0,
        decimals: 0,
        track: Track::Plain,
    };
    // (with no mask selected, mask.adjust sets the next mask's settings)
    let out = slider(ui, &amt, shown.adjust.amount, true, None);
    apply_slider_out(app, &amt, out, |app, v| app.run("mask.adjust", json!({"values": {"amount": v}})));
    for s in LOCAL {
        let v = local_get(&shown.adjust, s.id);
        let out = slider(ui, s, v, true, None);
        apply_slider_out(app, s, out, |app, v| app.run("mask.adjust", json!({"values": {s.id: v}})));
    }
    let Some(m) = selected else {
        ui.add_space(30.0);
        return;
    };
    let refine = ControlSpec {
        id: "refine",
        label: "Refine Edges",
        section: Section::Light,
        min: 0.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Track::Plain,
    };
    let out = slider(ui, &refine, m.refine, true, None);
    apply_slider_out(app, &refine, out, |app, v| app.run("mask.refine", json!({"value": v})));
    // further sections of the mask's adjustments: its own Curve and Point Color
    mask_sections(app, ui, id, &m);
    ui.add_space(30.0);
    let _ = Stroke::NONE;
}

/// Reset Settings on New Mask (Lightroom's Reset Sliders Automatically): off, a new mask starts
/// with the last mask's settings; on, at zero. Remembered.
fn reset_new_row(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 0, bottom: 4 }).show(ui, |ui| {
        let mut on = app.ui.mask_reset_new;
        let r = ui
            .checkbox(&mut on, crate::i18n::tr("Reset settings on new mask"))
            .on_hover_text(crate::i18n::tr("Off: a new mask starts with the last mask's settings. On: every new mask starts at zero."));
        register(ui.ctx(), "check:maskResetNew", r.rect);
        if r.changed() {
            app.ui.mask_reset_new = on;
            let _ = app.run("mask.next", json!({"resetNew": on}));
        }
    });
}

/// Sections of a mask's adjustments below its sliders: the mask's own Curve and Point Color, each a
/// collapsible section like the Edit panel's (`mask_local`).
fn mask_sections(app: &mut LightcraftApp, ui: &mut egui::Ui, photo: PhotoId, m: &lightcraft_develop::Mask) {
    super::mask_local::sections(app, ui, photo, m);
}

/// Translate fixed action labels, but never mutate or translate user-entered mask names.
fn menu_label(source: &str) -> &str {
    crate::i18n::tr(source)
}

/// The right-click menu of a mask in the Masks list.
pub(crate) fn mask_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, id: u32, name: &str, visible: bool, index: usize, count: usize) {
    let mut run = |ui: &mut egui::Ui, label: &str, enabled: bool, cmd: &str, p: serde_json::Value| {
        if ui.add_enabled(enabled, egui::Button::new(menu_label(label))).clicked() {
            let _ = app.run(cmd, p);
            ui.close();
        }
    };
    run(ui, "Duplicate Mask", true, "mask.duplicate", json!({"id": id}));
    run(ui, "Duplicate and Invert Mask", true, "mask.duplicate", json!({"id": id, "invert": true}));
    run(ui, "Invert Mask", true, "mask.invert", json!({"id": id}));
    run(ui, if visible { "Hide Mask" } else { "Show Mask" }, true, "mask.visible", json!({"id": id}));
    ui.separator();
    run(ui, "Move Up", index > 0, "mask.move", json!({"id": id, "delta": -1}));
    run(ui, "Move Down", index + 1 < count, "mask.move", json!({"id": id, "delta": 1}));
    ui.separator();
    if ui.button(crate::i18n::tr("Rename…")).clicked() {
        app.ui.renaming_mask = Some((id, name.to_string()));
        ui.close();
    }
    if ui.button(crate::i18n::tr("Delete Mask")).clicked() {
        let _ = app.run("mask.delete", json!({"id": id}));
        ui.close();
    }
}

/// Overlay colours offered as swatches (the colour of the selected one is used for the tint).
pub const OVERLAY_COLORS: [[u8; 3]; 5] = [[230, 30, 40], [40, 200, 70], [40, 110, 240], [250, 210, 30], [255, 255, 255]];

/// Controls for a range component: the selected range (two handles over a dark → light bar),
/// Smoothness, and Show Luminance Map; colour ranges get Refine.
pub(crate) fn range_controls(app: &mut LightcraftApp, ui: &mut egui::Ui, comp: usize, shape: &MaskShape) {
    let t = Tokens::get(ui.ctx());
    let update = |app: &mut LightcraftApp, shape: MaskShape| {
        let _ = app.run("mask.update", json!({"component": comp, "shape": shape}));
    };
    match shape {
        MaskShape::LuminanceRange { lo, hi, lo_feather, hi_feather } => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::i18n::tr("Select Luminance Range")).color(t.text_dim).size(11.5));
                let picking = app.ui.tool == "lumRange";
                if text_button(ui, &format!("lumPick{comp}"), crate::i18n::tr("Pick"), picking)
                    .on_hover_text(crate::i18n::tr("Click the photo to pick its tone, or drag over an area to pick its tones; ⇧ widens the range"))
                    .clicked()
                {
                    app.ui.tool = if picking { String::new() } else { "lumRange".into() };
                }
            });
            let (lo, hi) = (*lo, *hi);
            let w = ui.available_width().min(240.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click_and_drag());
            crate::widgets::announce(&resp, crate::i18n::tr("Select Luminance Range"));
            register(ui.ctx(), format!("lumRange:{comp}"), rect);
            let p = ui.painter();
            // dark → light ramp, the selected span outlined
            let n = 24;
            for k in 0..n {
                let a = k as f32 / n as f32;
                let r = Rect::from_min_max(
                    pos2(rect.left() + a * w, rect.top() + 4.0),
                    pos2(rect.left() + (a + 1.0 / n as f32) * w + 0.5, rect.bottom() - 4.0),
                );
                let g = (a * 255.0) as u8;
                p.rect_filled(r, 0.0, egui::Color32::from_gray(g));
            }
            let x = |v: f64| rect.left() + v.clamp(0.0, 1.0) as f32 * w;
            p.rect_stroke(
                Rect::from_min_max(pos2(x(lo), rect.top() + 2.0), pos2(x(hi), rect.bottom() - 2.0)),
                2.0,
                Stroke::new(1.5, t.accent),
                egui::StrokeKind::Middle,
            );
            for v in [lo, hi] {
                p.circle_filled(pos2(x(v), rect.center().y), 5.0, egui::Color32::WHITE);
                p.circle_stroke(pos2(x(v), rect.center().y), 5.0, Stroke::new(1.0, egui::Color32::from_gray(40)));
            }
            // Choose at the press position, then keep that handle for the entire drag. Testing
            // proximity on every frame can move the opposite endpoint after a fast/crossing drag.
            let handle_id = resp.id.with("luminance-handle");
            let nearer_low = |px: f32| (px - x(lo)).abs() <= (px - x(hi)).abs();
            if resp.drag_started() {
                if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                    ui.data_mut(|d| d.insert_temp(handle_id, nearer_low(origin.x)));
                }
                let _ = app.run("develop.beginInteraction", json!({"label": "Luminance Range"}));
            }
            if (resp.dragged() || resp.clicked())
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let v = (((pos.x - rect.left()) / w) as f64).clamp(0.0, 1.0);
                let low =
                    if resp.dragged() { ui.data(|d| d.get_temp::<bool>(handle_id)).unwrap_or_else(|| nearer_low(pos.x)) } else { nearer_low(pos.x) };
                let (nlo, nhi) = if low { (v.min((hi - 0.01).max(0.0)), hi) } else { (lo, v.max((lo + 0.01).min(1.0))) };
                update(app, MaskShape::LuminanceRange { lo: nlo, hi: nhi, lo_feather: *lo_feather, hi_feather: *hi_feather });
            }
            if resp.drag_stopped() {
                ui.data_mut(|d| d.remove::<bool>(handle_id));
                let _ = app.run("develop.endInteraction", json!({}));
            }
            // Smoothness: both falloffs at once
            let smooth = ControlSpec {
                id: "smoothness",
                label: "Smoothness",
                section: Section::Light,
                min: 0.0,
                max: 100.0,
                default: 20.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let cur = ((lo_feather + hi_feather) / 2.0 / 0.5 * 100.0).clamp(0.0, 100.0);
            let out = slider(ui, &smooth, cur, true, None);
            apply_slider_out(app, &smooth, out, |app, v| {
                let f = v / 100.0 * 0.5;
                app.run("mask.update", json!({"component": comp, "shape": MaskShape::LuminanceRange { lo, hi, lo_feather: f, hi_feather: f }}))
            });
            let mut map = app.ui.mask_overlay && app.ui.mask_overlay_mode == "colorOnBw";
            let r = ui.checkbox(&mut map, crate::i18n::tr("Show Luminance Map"));
            register(ui.ctx(), format!("check:lumMap{comp}"), r.rect);
            if r.changed() {
                // the luminance map: the photo in black & white with the selected range tinted
                if map {
                    app.ui.luminance_map_restore = Some((app.ui.mask_overlay, app.ui.mask_overlay_mode.clone()));
                    app.ui.mask_overlay = true;
                    app.ui.mask_overlay_mode = "colorOnBw".into();
                } else {
                    let (on, mode) = app.ui.luminance_map_restore.take().unwrap_or((false, "color".into()));
                    app.ui.mask_overlay = on;
                    app.ui.mask_overlay_mode = if mode == "colorOnBw" { "color".into() } else { mode };
                }
            }
        }
        MaskShape::Radial { feather, .. } => {
            // the softness of the edge, as the inner ring on the photo shows it (both stay in step)
            let spec = ControlSpec {
                id: "feather",
                label: "Feather",
                section: Section::Light,
                min: 0.0,
                max: 100.0,
                default: 50.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let out = slider(ui, &spec, *feather, true, None);
            let base = shape.clone();
            apply_slider_out(app, &spec, out, |app, v| {
                let mut s = base.clone();
                if let MaskShape::Radial { feather, .. } = &mut s {
                    *feather = v.clamp(0.0, 100.0);
                }
                app.run("mask.update", json!({"component": comp, "shape": s}))
            });
        }
        MaskShape::Object { edge, .. } | MaskShape::Prompt { edge, .. } => {
            // Edge: how crisp the selection's border is (−100 hard … 0 as computed … 100 soft)
            let spec = ControlSpec {
                id: "edge",
                label: "Edge",
                section: Section::Light,
                min: -100.0,
                max: 100.0,
                default: 0.0,
                step: 1.0,
                decimals: 0,
                track: Track::Centered,
            };
            let out = slider(ui, &spec, *edge, true, None);
            let base = shape.clone();
            apply_slider_out(app, &spec, out, |app, v| {
                let mut s = base.clone();
                if let MaskShape::Object { edge, .. } | MaskShape::Prompt { edge, .. } = &mut s {
                    *edge = v.clamp(-100.0, 100.0);
                }
                app.run("mask.update", json!({"component": comp, "shape": s}))
            });
            ui.label(egui::RichText::new(crate::i18n::tr("− harder border · + softer border")).color(t.text_dim).size(11.0));
        }
        MaskShape::ColorRange { samples, refine, areas } => {
            ui.horizontal(|ui| {
                // the picked colours, as swatches
                for (i, s) in samples.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, super::range_pick::oklab_color(*s));
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
                    register(ui.ctx(), format!("colorSwatch:{comp}:{i}"), r);
                }
                if samples.is_empty() {
                    ui.label(egui::RichText::new(crate::i18n::tr("No colour picked")).color(t.text_dim).size(11.5));
                }
                let picking = app.ui.tool == "colorRange";
                if text_button(ui, &format!("colorPick{comp}"), crate::i18n::tr("Pick"), picking)
                    .on_hover_text(crate::i18n::tr(
                        "Click or drag over the photo to pick colours; ⇧ adds more (up to 5), ⌥-click removes one, drag a pin to move it",
                    ))
                    .clicked()
                {
                    app.ui.tool = if picking { String::new() } else { "colorRange".into() };
                }
            });
            let spec = ControlSpec {
                id: "refine",
                label: "Refine",
                section: Section::Color,
                min: 0.0,
                max: 100.0,
                default: 50.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let out = slider(ui, &spec, *refine, true, None);
            let samples = samples.clone();
            apply_slider_out(app, &spec, out, |app, v| {
                app.run(
                    "mask.update",
                    json!({"component": comp, "shape": MaskShape::ColorRange { samples: samples.clone(), refine: v, areas: areas.clone() }}),
                )
            });
        }
        _ => {}
    }
}

/// The right-click / "…" menu of one component of the selected mask.
pub(crate) fn component_row_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, mask: u32, k: usize, label: &str, count: usize) {
    let mut run = |ui: &mut egui::Ui, text: &str, p: serde_json::Value| {
        if ui.button(menu_label(text)).clicked() {
            let mut p = p;
            p["id"] = json!(mask);
            p["component"] = json!(k);
            let _ = app.run("mask.component", p);
            ui.close();
        }
    };
    run(ui, "Invert", json!({"action": "invert"}));
    run(ui, &crate::i18n::tr_format!("Duplicate \"{label}\"", label = label), json!({"action": "duplicate"}));
    if k > 0 {
        crate::widgets::menu_button(ui, crate::i18n::tr("Mode"), |ui| {
            for (op, text) in [("add", "Add"), ("subtract", "Subtract"), ("intersect", "Intersect")] {
                run(ui, text, json!({"action": "op", "op": op}));
            }
        });
    }
    crate::widgets::menu_button(ui, crate::i18n::tr("Intersect with"), |ui| component_menu(app, ui, "intersect"));
    crate::widgets::menu_button(ui, crate::i18n::tr("Subtract"), |ui| component_menu(app, ui, "subtract"));
    ui.separator();
    if ui.button(crate::i18n::tr("Rename…")).clicked() {
        app.ui.renaming_component = Some((mask, k, label.to_string()));
        ui.close();
    }
    let del = if count == 1 {
        crate::i18n::tr_format!("Delete \"{label}\" (and the mask)", label = label)
    } else {
        crate::i18n::tr_format!("Delete \"{label}\"", label = label)
    };
    if ui.button(del).clicked() {
        let _ = app.run("mask.component", json!({"id": mask, "component": k, "action": "delete"}));
        ui.close();
    }
}

/// Whether an AI model Object or Describe needs is missing in a build that could use it: then
/// its download is offered (with `then` to start afterwards) instead of starting the mask. Both
/// run on openly licensed models (Object on SAM 2.1, Describe on OWLv2 and SAM 2.1); SAM 3 is an
/// optional extra, offered only when there is no folder for the open models.
fn needs_model(app: &mut LightcraftApp, kind: &str, op: &str) -> bool {
    let seg = &app.session.segmenter;
    let sam3 = seg.remote_endpoint().is_some() || seg.installed();
    if !lightcraft_engine::segment::Segmenter::AVAILABLE || sam3 {
        return false;
    }
    if seg.models_root.is_some() {
        let missing = match kind {
            "object" => (!seg.model_installed("sam2")).then_some("sam2"),
            // the finder first, then SAM 2.1 (each download offers the next when it is done)
            _ => seg.describe_needs().first().copied(),
        };
        let Some(model) = missing else { return false };
        app.offer_ai_model(model, Some((kind, op)));
        return true;
    }
    if seg.dir.is_some() {
        app.offer_sam_download(Some((kind, op)));
        return true;
    }
    false
}

/// Add an automatic AI mask (`params` for `mask.add`, or `mask.addComponent` with `op`): when its
/// model isn't installed it is made by the heuristic, and the panel says so ([`ai_notes`]).
pub(crate) fn add_auto(app: &mut LightcraftApp, ctx: &egui::Context, params: serde_json::Value, op: Option<&str>) {
    let r = match op {
        None => app.run("mask.add", params),
        Some(op) => {
            let mut p = params;
            p["op"] = json!(op);
            app.run("mask.addComponent", p)
        }
    };
    match r {
        Ok(r) => {
            if r.get("approximate").and_then(serde_json::Value::as_bool) == Some(true) {
                app.ui.ai_hint = r.get("model").and_then(serde_json::Value::as_str).map(str::to_string);
            }
        }
        Err(e) => app.ai_error(ctx, e, None),
    }
}

/// The People parts Lightroom offers, in its order (the names are stored with the mask).
pub(crate) const PEOPLE_PARTS: [&str; 10] =
    ["Entire Person", "Face Skin", "Body Skin", "Hair", "Clothes", "Eyebrows", "Eye Sclera", "Iris and Pupil", "Lips", "Teeth"];

/// Every kind of mask, in Create New Mask's order, with its label and icon. The tiles and the
/// Add / Subtract / Intersect menus offer all of them, in the same order, with the same icons.
pub(crate) const MASK_KINDS: [(&str, &str, Icon); 13] = [
    ("object", "Object", Icon::Object),
    ("prompt", "Describe", Icon::Describe),
    ("subject", "Subject", Icon::Subject),
    ("sky", "Sky", Icon::Sky),
    ("background", "Background", Icon::Background),
    ("people", "People", Icon::People),
    ("concept", "Remembered", Icon::Remembered),
    ("landscape", "Landscape", Icon::Landscape),
    ("brush", "Brush", Icon::Brush),
    ("linear", "Linear", Icon::Linear),
    ("radial", "Radial", Icon::Radial),
    ("luminanceRange", "Luminance", Icon::Luminance),
    ("colorRange", "Color", Icon::Picker),
];

/// Start a mask of `kind` (a [`MASK_KINDS`] key): a new mask (`op` "new") or a component the
/// selected mask adds, subtracts or intersects. Tools wait for the photo (the first click or drag
/// there makes it), pickers and the Describe field open, the automatic ones are made at once.
pub(crate) fn start_kind(app: &mut LightcraftApp, ctx: &egui::Context, kind: &str, op: &str) {
    let combine = (op != "new").then_some(op);
    match kind {
        // a second click on a tile puts its tool away (never an empty mask)
        "colorRange" | "brush" | "linear" | "radial" => arm(app, ctx, kind, op),
        "object" => {
            if !needs_model(app, "object", op) {
                arm(app, ctx, kind, op);
            }
        }
        "prompt" => {
            if app.ui.describe.as_ref().is_some_and(|(o, _)| o == op) {
                app.ui.describe = None;
            } else {
                start_describe(app, op);
            }
        }
        "people" | "concept" | "landscape" => {
            let open = match kind {
                "people" => app.ui.people_pick.is_some(),
                "concept" => app.ui.concept_pick,
                _ => app.ui.landscape_pick,
            };
            let same_op = app.ui.mask_pick_op.as_deref() == combine;
            // one picker at a time
            app.ui.people_pick = None;
            app.ui.concept_pick = false;
            app.ui.landscape_pick = false;
            if open && same_op {
                app.ui.mask_pick_op = None;
                return;
            }
            app.ui.mask_pick_op = combine.map(str::to_string);
            match kind {
                "people" => app.ui.people_pick = Some((0, Vec::new())),
                "concept" => app.ui.concept_pick = true,
                _ => app.ui.landscape_pick = true,
            }
        }
        "subject" | "sky" | "background" => add_auto(app, ctx, json!({"kind": kind}), combine),
        k => {
            let r = match combine {
                None => app.run("mask.add", json!({"kind": k})),
                Some(op) => app.run("mask.addComponent", json!({"op": op, "kind": k})),
            };
            if let Err(e) = r {
                app.toast(ctx, e);
            }
        }
    }
}

/// The People, Remembered and Landscape pickers (whichever is open).
pub(crate) fn pickers(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    super::smart_people::people_picker(app, ui);
    super::smart_people::concept_picker(app, ui);
    landscape_picker(app, ui);
}

/// The Landscape classes, under the tiles once the Landscape tile was clicked.
fn landscape_picker(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    if !app.ui.landscape_pick {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for (class, label) in [
            ("sky", "Sky"),
            ("water", "Water"),
            ("vegetation", "Vegetation"),
            ("mountains", "Mountains"),
            ("architecture", "Architecture"),
            ("naturalGround", "Natural Ground"),
            ("artificialGround", "Artificial Ground"),
        ] {
            let b = ui.button(crate::i18n::tr(label));
            register(ui.ctx(), format!("maskLandscape:{class}"), b.rect);
            if b.clicked() {
                app.ui.landscape_pick = false;
                let op = app.ui.mask_pick_op.take();
                add_auto(app, ui.ctx(), json!({"kind": "landscape", "class": class}), op.as_deref());
            }
        }
    });
}

/// Under the tiles: an AI mask made by the heuristic because its model is missing (with a button to
/// get it), a selected mask whose AI selections can now be computed by their model (Update), and
/// openly licensed model downloads while their dialog is closed.
fn ai_notes(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if let Some(model) = app.ui.ai_hint.clone() {
        if app.session.segmenter.model_installed(&model) {
            app.ui.ai_hint = None;
        } else {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::i18n::tr("This mask is approximate: its AI model isn't installed, so a simpler method made it."))
                        .color(t.text_dim),
                )
                .wrap(),
            );
            ui.horizontal_wrapped(|ui| {
                if text_button(ui, "maskAiGetModel", crate::i18n::tr("Get the AI Model…"), false).clicked() {
                    app.offer_ai_model(&model, None);
                }
                if text_button(ui, "maskAiHintDismiss", crate::i18n::tr("Not Now"), false).clicked() {
                    app.ui.ai_hint = None;
                }
            });
        }
    }
    // the selected mask has automatic selections without a model's segmentation, and the model is there now
    let stale = app.session.active().and_then(|id| app.session.develop_of(id)).is_some_and(|d| {
        d.masks.iter().find(|m| Some(m.id) == app.session.active_mask).is_some_and(|m| {
            m.components.iter().any(|c| {
                lightcraft_engine::segment::AutoKind::of(&c.shape)
                    .is_some_and(|k| k.seg_of(&c.shape).is_some_and(Option::is_none) && app.session.segmenter.auto_model(&k).is_ok())
            })
        })
    });
    if stale && !app.session.segmenter.busy() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("The AI model for this mask is installed.")).color(t.text_dim));
            if text_button(ui, "maskAiUpdate", crate::i18n::tr("Update"), false).clicked()
                && let Err(e) = app.run("mask.recompute", json!({}))
            {
                app.ai_error(ui.ctx(), e, None);
            }
        });
    }
    if app.ui.dialog.is_none() {
        for key in app.ui.ai_downloading.clone() {
            let d = app.session.segmenter.model_download_status(&key);
            if !d.running {
                continue;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let pct = if d.total > 0 { d.done as f64 / d.total as f64 } else { 0.0 };
                let title = app.session.segmenter.model_status().into_iter().find(|m| m.id == key).map(|m| m.title).unwrap_or_default();
                let r = ui.add(
                    egui::ProgressBar::new(pct as f32)
                        .desired_width(ui.available_width() - 70.0)
                        .text(format!("{title}: {} %", (pct * 100.0).floor())),
                );
                register(ui.ctx(), format!("maskAiProgress:{key}"), r.rect);
                if text_button(ui, &format!("maskAiDetails:{key}"), crate::i18n::tr("Details"), false).clicked() {
                    app.offer_ai_model(&key, None);
                }
            });
        }
    }
}

/// Whether a tile shows pressed: its tool is in use, or its picker / field is open.
fn tile_active(app: &LightcraftApp, kind: &str) -> bool {
    match kind {
        "people" => app.ui.people_pick.is_some() && app.ui.mask_pick_op.is_none(),
        "concept" => app.ui.concept_pick && app.ui.mask_pick_op.is_none(),
        "landscape" => app.ui.landscape_pick && app.ui.mask_pick_op.is_none(),
        "prompt" => app.ui.describe.as_ref().is_some_and(|(op, _)| op == "new"),
        k => app.ui.tool == k,
    }
}

/// Arm mask tool `kind` (brush, linear, radial, colorRange, object) for a new mask (`op` "new")
/// or a component combined by `op`: nothing is made until the photo is clicked or dragged
/// ([`crate::panels::detail`]). The same tool again puts it away.
pub(crate) fn arm(app: &mut LightcraftApp, ctx: &egui::Context, kind: &str, op: &str) {
    // one press = one mask: a tile clicked again while its tool waits for the photo puts it away;
    // once the tool has made its mask (a gradient's tool is then put away, a brush keeps painting
    // that mask), the tile arms it for the next one (Add / Subtract menus always arm)
    if op == "new" && app.ui.tool == kind && app.ui.mask_new.as_ref().is_some_and(|(_, o)| o == op) {
        app.ui.tool.clear();
        app.ui.mask_new = None;
        return;
    }
    app.ui.tool = kind.into();
    app.ui.mask_new = Some((kind.into(), op.into()));
    app.ui.mask_overlay_hidden = None;
    if kind == "brush" {
        app.ui.brush_erase = false;
    }
    let hint = match kind {
        "linear" => "Drag on the photo to draw the gradient",
        "radial" => "Drag on the photo to draw the ellipse (a click places one)",
        "brush" => "Paint on the photo",
        "colorRange" => "Click the photo to pick a colour · ⇧-click adds more",
        _ => "Click the object to select it · ⌥-click leaves a part out",
    };
    app.toast(ctx, crate::i18n::tr(hint));
}

/// Start an AI mask of `kind` (object|prompt) combined by `op` (new|add|subtract|intersect),
/// after the model was installed.
pub(crate) fn begin_ai(app: &mut LightcraftApp, kind: &str, op: &str) -> Result<serde_json::Value, String> {
    match kind {
        "object" => {
            // armed: the first click on the photo makes the selection
            app.ui.tool = "object".into();
            app.ui.mask_new = Some(("object".into(), op.into()));
            Ok(serde_json::Value::Null)
        }
        _ => {
            app.ui.describe = Some((op.to_string(), String::new()));
            Ok(serde_json::Value::Null)
        }
    }
}

/// Open the Describe field (a new mask, or a component combined by `op`).
pub(crate) fn start_describe(app: &mut LightcraftApp, op: &str) {
    if needs_model(app, "prompt", op) {
        return;
    }
    app.ui.describe = Some((op.to_string(), String::new()));
}

/// The Describe field: type what to select ("sky", "the red car") and press Return.
pub(crate) fn describe_field(app: &mut LightcraftApp, ui: &mut egui::Ui, new_mask: bool) {
    let Some((op, mut text)) = app.ui.describe.clone() else { return };
    // a new mask's field sits under the tiles; one that combines, under the selected mask
    if (op == "new") != new_mask {
        return;
    }
    ui.add_space(8.0);
    let prompt = match op.as_str() {
        "subtract" => "Describe what to leave out",
        "intersect" => "Describe what to keep",
        _ => "Describe what to select",
    };
    ui.label(crate::i18n::tr(prompt));
    let mut submit = false;
    // the (translated) Select button is placed first, at the right; the field takes what is left
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if text_button(ui, "maskDescribeGo", crate::i18n::tr("Select"), false).clicked() {
            submit = true;
        }
        let r = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text(crate::i18n::tr("e.g. sky · the red car · car, road"))
                .desired_width(ui.available_width()),
        );
        register(ui.ctx(), "maskDescribe", r.rect);
        if !r.has_focus() && !r.lost_focus() && text.is_empty() {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.describe = None;
        return;
    }
    app.ui.describe = Some((op.clone(), text.clone()));
    if submit && !text.trim().is_empty() {
        let r = if op == "new" {
            app.run_typed("mask.add", json!({"kind": "prompt", "text": text}))
        } else {
            app.run_typed("mask.addComponent", json!({"op": op, "kind": "prompt", "text": text}))
        };
        match r {
            // the mask appears when the model has found it (a detail pass follows)
            Ok(_) => app.ui.describe = None,
            Err(e) => app.ai_error(ui.ctx(), e, Some(("prompt", op.as_str()))),
        }
    }
}

/// The Add / Subtract / Intersect menu: every kind of mask, as Create New Mask offers them
/// ([`MASK_KINDS`], same order and icons), combined with the selected mask by `op`.
pub(crate) fn component_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, op: &str) {
    for (kind, label, icon) in MASK_KINDS {
        // the menu's names say a little more than the tiles' (there is room)
        let label = match kind {
            "prompt" => "Describe…",
            "people" => "People…",
            "concept" => "Remembered…",
            "landscape" => "Landscape…",
            "linear" => "Linear Gradient",
            "radial" => "Radial Gradient",
            "luminanceRange" => "Luminance Range",
            "colorRange" => "Color Range",
            _ => label,
        };
        let b = menu_item(ui, icon, menu_label(label));
        register(ui.ctx(), format!("maskComp:{op}:{kind}"), b.rect);
        if b.clicked() {
            start_kind(app, ui.ctx(), kind, op);
            ui.close();
        }
    }
}

/// A menu entry with an icon before its label.
fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let h = 22.0;
    // as wide as the label needs (a menu grows to what it is given)
    let text = ui.painter().layout_no_wrap(label.to_string(), t.font(12.5), t.text).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2((text + 40.0).max(170.0), h), Sense::click());
    crate::widgets::announce(&resp, label);
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let c = if resp.hovered() { t.text } else { t.text_label };
    paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), vec2(15.0, 15.0)), icon, c);
    ui.painter().text(pos2(r.left() + 28.0, r.center().y), egui::Align2::LEFT_CENTER, label, t.font(12.5), c);
    resp
}

fn brush_settings(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 6, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Brush")).font(t.semibold(13.0)));
            if text_button(ui, "brushAdd", crate::i18n::tr("Add"), !app.ui.brush_erase).clicked() {
                app.ui.brush_erase = false;
            }
            if text_button(ui, "brushErase", crate::i18n::tr("Erase"), app.ui.brush_erase).clicked() {
                app.ui.brush_erase = true;
            }
        });
        let mut auto = app.ui.brush_auto_mask;
        if ui
            .checkbox(&mut auto, crate::i18n::tr("Auto Mask"))
            .on_hover_text(crate::i18n::tr("Paint only areas like the one under the brush"))
            .changed()
        {
            app.ui.brush_auto_mask = auto;
        }
        let mut pressure = app.ui.brush_pressure;
        if ui
            .checkbox(&mut pressure, crate::i18n::tr("Pen Pressure"))
            .on_hover_text(crate::i18n::tr("A pen's pressure sets the size and flow of the brush"))
            .changed()
        {
            app.ui.brush_pressure = pressure;
        }
    });
    for (id, label, min, max, get) in [
        ("ui.brushSize", "Size", 1.0, 100.0, (app.ui.brush_size * 400.0) as f64),
        ("ui.brushFeather", "Feather", 0.0, 100.0, app.ui.brush_feather as f64),
        ("ui.brushFlow", "Flow", 1.0, 100.0, app.ui.brush_flow as f64),
    ] {
        let s = ControlSpec { id, label, section: Section::Light, min, max, default: min, step: 1.0, decimals: 0, track: Track::Plain };
        let out = slider(ui, &s, get, true, None);
        if let Some(v) = out.value {
            match id {
                "ui.brushSize" => app.ui.brush_size = (v / 400.0) as f32,
                "ui.brushFeather" => app.ui.brush_feather = v as f32,
                _ => app.ui.brush_flow = v as f32,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_context_actions_use_catalog_translations() {
        let saved = crate::i18n::language();
        crate::i18n::set_language(crate::i18n::Locale::Fr);
        for (english, translated) in [
            ("Duplicate Mask", "Dupliquer le masque"),
            ("Invert Mask", "Inverser le masque"),
            ("Hide Mask", "Masquer le masque"),
            ("Move Up", "Monter"),
            ("Brush", "Pinceau"),
            ("Luminance Range", "Plage de luminance"),
        ] {
            assert_eq!(menu_label(english), translated, "{english}");
        }
        // An arbitrary user name has no catalog entry and is left unchanged.
        assert_eq!(menu_label("My custom mask"), "My custom mask");
        crate::i18n::set_language(saved);
    }

    /// The grid keeps four 52 pt tiles when the labels fit, and gives a long (translated) label
    /// room by dropping a column rather than drawing it past its tile.
    #[test]
    fn tile_labels_get_the_room_they_need() {
        let short = |size: f32| size * 4.0; // "Luminance"-ish: 42 pt at 10.5
        let long = |size: f32| size * 5.6; // "Background"/"Hintergrund": 59 pt at 10.5, 53 at 9.5
        assert_eq!(tile_layout(224.0, short), (4, 52.0, 10.5));
        // too wide for four at either size: three tiles, wider than 52 so the label fits
        let (n, w, size) = tile_layout(224.0, long);
        assert_eq!((n, size), (3, 10.5));
        assert!(w >= long(10.5) + 2.0 * TILE_PAD && w <= (224.0 - 2.0 * TILE_GAP) / 3.0, "{w}");
        // a smaller label keeps a column: 9.5 pt fits where 10.5 doesn't
        let mid = |size: f32| size * 4.6; // 48.3 at 10.5, 43.7 at 9.5
        assert_eq!(tile_layout(224.0, mid), (4, 52.0, 9.5));
        // a wide panel: tiles stay 52 pt (or as wide as the label)
        assert_eq!(tile_layout(474.0, short).0, 4);
        assert_eq!(tile_layout(474.0, short).1, TILE);
        // far too narrow for any label: two tiles (labels cut short), never a negative width
        let (n, w, _) = tile_layout(60.0, long);
        assert_eq!(n, 2);
        assert!(w >= 24.0);
        assert_eq!(tile_layout(f32::NAN, long).0, 2);
    }
}
