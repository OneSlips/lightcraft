//! A mask's own Curve and Point Color: the sections under the selected mask's sliders in the Masking
//! panel (Lightroom's Curve and Point Color sections of a mask). They reuse the Edit panel's curve
//! graph and Point Color controls, and edit the mask through `mask.curve` and
//! `mask.pickPointColor` / `mask.setPointColor` / `mask.deletePointColor`.

use egui::{Sense, Stroke, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{ControlSpec, MAX_POINT_COLORS, Mask, PointColor, Section, Track};
use serde_json::json;

use super::edit::{CurveTarget, apply_slider_out, curve_graph, oklch_color, sub_title};
use crate::LightcraftApp;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{divider, register, section_header, slider, text_button};

/// The on-canvas tool that samples a colour for the selected mask's Point Color.
pub const PICK_TOOL: &str = "maskPointColor";

const fn spec(id: &'static str, label: &'static str, min: f64, max: f64, default: f64, track: Track) -> ControlSpec {
    ControlSpec { id, label, section: Section::PointColor, min, max, default, step: 1.0, decimals: 0, track }
}

/// A sample's sliders (`maskPointColor.<the mask.setPointColor field>`), as the global Point Color's.
const POINT: [ControlSpec; 8] = [
    spec("maskPointColor.hueShift", "Hue", -100.0, 100.0, 0.0, Track::Rainbow),
    spec("maskPointColor.satShift", "Saturation", -100.0, 100.0, 0.0, Track::Gradient { from: "#7a7a7a", to: "#e04a3a" }),
    spec("maskPointColor.lumShift", "Luminance", -100.0, 100.0, 0.0, Track::Gradient { from: "#202020", to: "#f0f0f0" }),
    spec("maskPointColor.variance", "Variance", -100.0, 100.0, 0.0, Track::Centered),
    spec("maskPointColor.range", "Range", 0.0, 100.0, 50.0, Track::Plain),
    spec("maskPointColor.hueRange", "Hue Range", 0.0, 100.0, 50.0, Track::Rainbow),
    spec("maskPointColor.satRange", "Saturation Range", 0.0, 100.0, 50.0, Track::Gradient { from: "#7a7a7a", to: "#e04a3a" }),
    spec("maskPointColor.lumRange", "Luminance Range", 0.0, 100.0, 50.0, Track::Gradient { from: "#202020", to: "#f0f0f0" }),
];

const REFINE: ControlSpec = spec("maskCurve.refineSaturation", "Refine Saturation", 0.0, 100.0, 100.0, Track::Plain);

/// The `mask.setPointColor` field of a slider (`maskPointColor.hueShift` → `hueShift`).
fn field(s: &ControlSpec) -> &'static str {
    s.id.strip_prefix("maskPointColor.").unwrap_or(s.id)
}

fn value(p: &PointColor, key: &str) -> f64 {
    match key {
        "hueShift" => p.hue_shift,
        "satShift" => p.sat_shift,
        "lumShift" => p.lum_shift,
        "variance" => p.variance,
        "range" => p.range,
        "hueRange" => p.hue_range,
        "satRange" => p.sat_range,
        _ => p.lum_range,
    }
}

/// A collapsible section header for the mask (open unless the user closed it). True when open.
pub(crate) fn mask_section(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let open = !app.ui.mask_sections_closed.iter().any(|s| s == id);
    let (resp, _) = section_header(ui, id, title, open, None);
    if resp.clicked() {
        if open {
            app.ui.mask_sections_closed.push(id.to_string());
        } else {
            app.ui.mask_sections_closed.retain(|s| s != id);
        }
    }
    open
}

/// The selected mask's Curve and Point Color sections.
pub(crate) fn sections(app: &mut LightcraftApp, ui: &mut egui::Ui, photo: PhotoId, m: &Mask) {
    divider(ui);
    if mask_section(app, ui, "maskCurve", "Curve") {
        let _ = curve_graph(app, ui, photo, &m.curve, CurveTarget::Mask(m.id));
        let out = slider(ui, &REFINE, m.curve.refine_saturation, true, Some("Refine Sat."));
        apply_slider_out(app, &REFINE, out, |app, v| app.run("mask.curve", json!({"id": m.id, "refineSaturation": v})));
    }
    divider(ui);
    if mask_section(app, ui, "maskPointColor", "Point Color") {
        point_color(app, ui, m);
    }
}

/// Swatches of the mask's samples (+ the eyedropper), the selected sample's shifts and range.
fn point_color(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &Mask) {
    let t = Tokens::get(ui.ctx());
    let n = m.point_colors.len();
    if app.ui.mask_point_color >= n && n > 0 {
        app.ui.mask_point_color = n - 1;
    }
    let sel = app.ui.mask_point_color;
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 8, bottom: 4 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let active = app.ui.tool == PICK_TOOL;
            if crate::widgets::icon_button(
                ui,
                "maskPointColorPicker",
                Icon::Picker,
                vec2(26.0, 26.0),
                active,
                n < MAX_POINT_COLORS,
                crate::i18n::tr("Sample a colour on the photo"),
            )
            .clicked()
            {
                app.ui.tool = if active { String::new() } else { PICK_TOOL.into() };
            }
            for (i, p) in m.point_colors.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                register(ui.ctx(), format!("maskPointColor:{i}"), r);
                ui.painter().rect_filled(r.shrink(2.0), 3.0, oklch_color(p.lum, p.chroma, p.hue));
                if i == sel {
                    ui.painter().rect_stroke(r, 4.0, Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    app.ui.mask_point_color = i;
                }
            }
        });
    });
    let Some(p) = m.point_colors.get(sel).copied() else {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 2, bottom: 8 }).show(ui, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Pick a colour on the photo with the eyedropper.")).size(12.0).color(t.text_dim));
        });
        return;
    };
    let row = |app: &mut LightcraftApp, ui: &mut egui::Ui, s: &ControlSpec| {
        let out = slider(ui, s, value(&p, field(s)), true, None);
        apply_slider_out(app, s, out, |app, v| app.run("mask.setPointColor", json!({"id": m.id, "index": sel, "values": {field(s): v}})));
    };
    for s in &POINT[..4] {
        row(app, ui, s);
    }
    sub_title(ui, crate::i18n::tr("Range"));
    for s in &POINT[4..] {
        row(app, ui, s);
    }
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 4, bottom: 8 }).show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if text_button(ui, "maskPointColorDelete", crate::i18n::tr("Delete"), false).clicked() {
                let _ = app.run("mask.deletePointColor", json!({"id": m.id, "index": sel}));
            }
        });
    });
}
