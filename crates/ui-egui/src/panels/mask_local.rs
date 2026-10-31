//! A mask's own Curve, Color Mixer and Point Color: the sections under the selected mask's sliders in
//! the Masking panel (Lightroom's Curve and Point Color sections of a mask, and a Color Mixer its masks
//! don't have). They reuse the Edit panel's curve graph, Color Mixer sliders and Point Color controls,
//! and edit the mask through `mask.curve`, `mask.mixer` / `mask.mixerTargeted` and
//! `mask.pickPointColor` / `mask.setPointColor` / `mask.deletePointColor`.

use egui::{Sense, Stroke, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{ControlSpec, MAX_POINT_COLORS, MIXER_BANDS, Mask, Mixer, PointColor, Section, Track};
use serde_json::json;

use super::edit::{BAND_NAMES, CurveTarget, apply_slider_out, curve_graph, group_tab, oklch_color, sub_title};
use crate::LightcraftApp;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{BAND_COLORS, divider, hex, register, section_header, slider, text_button};

/// The on-canvas tool that samples a colour for the selected mask's Point Color.
pub const PICK_TOOL: &str = "maskPointColor";

/// The on-canvas targeted adjustment of the selected mask's Color Mixer: `maskMixer:<hue|sat|lum>`.
pub const MIXER_TOOL: &str = "maskMixer:";

/// The Color Mixer sliders of a mask (`maskMixer.<band>.<hue|sat|lum>`, the `mask.mixer` key after the
/// prefix), in `MIXER_BANDS` order, each band's hue, saturation and luminance.
const MIXER_IDS: [&str; 24] = [
    "maskMixer.red.hue",
    "maskMixer.red.sat",
    "maskMixer.red.lum",
    "maskMixer.orange.hue",
    "maskMixer.orange.sat",
    "maskMixer.orange.lum",
    "maskMixer.yellow.hue",
    "maskMixer.yellow.sat",
    "maskMixer.yellow.lum",
    "maskMixer.green.hue",
    "maskMixer.green.sat",
    "maskMixer.green.lum",
    "maskMixer.aqua.hue",
    "maskMixer.aqua.sat",
    "maskMixer.aqua.lum",
    "maskMixer.blue.hue",
    "maskMixer.blue.sat",
    "maskMixer.blue.lum",
    "maskMixer.purple.hue",
    "maskMixer.purple.sat",
    "maskMixer.purple.lum",
    "maskMixer.magenta.hue",
    "maskMixer.magenta.sat",
    "maskMixer.magenta.lum",
];

/// The mask mixer's tabs: one slider for each colour (Hue | Saturation | Luminance), or one colour's
/// three (Color), as the global Color Mixer's HSL groups and its Color mode.
const MIXER_TABS: [(&str, &str); 4] = [("Hue", "hue"), ("Saturation", "sat"), ("Luminance", "lum"), ("Color", "color")];

/// The slider of band `b` (index into `MIXER_BANDS`) and attribute `k` (0 hue, 1 sat, 2 lum): the global
/// mixer's (range, coloured track) under the mask's id.
fn mixer_spec(b: usize, k: usize) -> Option<ControlSpec> {
    let id = *MIXER_IDS.get(b * 3 + k)?;
    let key = id.strip_prefix("maskMixer.")?;
    let global = lightcraft_develop::controls::find(&format!("mixer.{key}"))?;
    Some(ControlSpec { id, ..*global })
}

/// The `mask.mixer` key of a slider (`maskMixer.red.hue` → `red.hue`).
fn mixer_key(s: &ControlSpec) -> &'static str {
    s.id.strip_prefix("maskMixer.").unwrap_or(s.id)
}

fn mixer_value(m: &Mixer, b: usize, k: usize) -> f64 {
    m.bands().get(b).map_or(0.0, |h| match k {
        0 => h.hue,
        1 => h.sat,
        _ => h.lum,
    })
}

/// The mask's Color Mixer: Hue | Saturation | Luminance | Color tabs, the targeted adjustment of the
/// shown attribute, and the sliders, as the global Color Mixer's (`mask.mixer`).
fn mixer(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &Mask) {
    let t = Tokens::get(ui.ctx());
    let group = app.ui.mask_mixer_group.clone();
    egui::Frame::NONE.inner_margin(crate::widgets::row_margin(4, 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (label, key) in MIXER_TABS {
                if group_tab(ui, &format!("maskMixerGroup-{key}"), crate::i18n::tr(label), group == key).clicked() {
                    app.ui.mask_mixer_group = key.to_string();
                    if app.ui.tool.starts_with(MIXER_TOOL) {
                        app.ui.tool.clear();
                    }
                }
            }
        });
    });
    let k = match group.as_str() {
        "sat" => Some(1),
        "lum" => Some(2),
        "color" => None,
        _ => Some(0),
    };
    let row = |app: &mut LightcraftApp, ui: &mut egui::Ui, b: usize, k: usize, label: Option<&str>| {
        let Some(s) = mixer_spec(b, k) else { return };
        let out = slider(ui, &s, mixer_value(&m.mixer, b, k), true, label);
        apply_slider_out(app, &s, out, |app, v| app.run("mask.mixer", json!({"id": m.id, "values": {mixer_key(&s): v}})));
    };
    match k {
        Some(k) => {
            // the targeted adjustment of this attribute, then a slider per colour (labelled by the colour)
            egui::Frame::NONE.inner_margin(crate::widgets::row_margin(2, 0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let tool = format!("{MIXER_TOOL}{group}");
                    let active = app.ui.tool == tool;
                    if crate::widgets::icon_button(
                        ui,
                        &format!("maskMixerTargeted-{group}"),
                        Icon::Target,
                        vec2(24.0, 24.0),
                        active,
                        true,
                        crate::i18n::tr("Targeted adjustment: drag up/down on the photo to adjust the colours there"),
                    )
                    .clicked()
                    {
                        app.ui.tool = if active { String::new() } else { tool };
                    }
                    let any = !m.mixer.is_neutral();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if any && text_button(ui, "maskMixerReset", crate::i18n::tr("Reset"), false).clicked() {
                            let _ = app.run("mask.mixer", json!({"id": m.id, "reset": true}));
                        }
                    });
                });
            });
            for (b, name) in BAND_NAMES.iter().enumerate() {
                row(app, ui, b, k, Some(name));
            }
        }
        None => {
            // one colour at a time: a dot per colour, then its three sliders
            let sel = MIXER_BANDS.iter().position(|b| *b == app.ui.mask_mixer_band).unwrap_or(0);
            egui::Frame::NONE.inner_margin(crate::widgets::row_margin(4, 4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    for (i, b) in MIXER_BANDS.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                        crate::widgets::announce(&resp, crate::i18n::tr(BAND_NAMES.get(i).copied().unwrap_or(b)));
                        register(ui.ctx(), format!("maskMixerBand:{b}"), r);
                        ui.painter().circle_filled(r.center(), 9.0, hex(BAND_COLORS.get(i).copied().unwrap_or("#808080")));
                        if i == sel {
                            ui.painter().circle_stroke(r.center(), 10.5, Stroke::new(1.5, t.text));
                        }
                        if resp.clicked() {
                            app.ui.mask_mixer_band = b.to_string();
                        }
                    }
                });
            });
            for k in 0..3 {
                row(app, ui, sel, k, None);
            }
        }
    }
}

const fn spec(id: &'static str, label: &'static str, min: f64, max: f64, default: f64, track: Track) -> ControlSpec {
    ControlSpec { id, label, section: Section::PointColor, min, max, default, step: 1.0, decimals: 0, track, shade: lightcraft_develop::Shade::Plain }
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

/// The selected mask's Curve, Color Mixer and Point Color sections.
pub(crate) fn sections(app: &mut LightcraftApp, ui: &mut egui::Ui, photo: PhotoId, m: &Mask) {
    divider(ui);
    if mask_section(app, ui, "maskCurve", "Curve") {
        let _ = curve_graph(app, ui, photo, &m.curve, CurveTarget::Mask(m.id));
        let out = slider(ui, &REFINE, m.curve.refine_saturation, true, Some("Refine Sat."));
        apply_slider_out(app, &REFINE, out, |app, v| app.run("mask.curve", json!({"id": m.id, "refineSaturation": v})));
    }
    divider(ui);
    if mask_section(app, ui, "maskMixer", "Color Mixer") {
        mixer(app, ui, m);
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
    egui::Frame::NONE.inner_margin(crate::widgets::row_margin(4, 4)).show(ui, |ui| {
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
        egui::Frame::NONE.inner_margin(crate::widgets::row_margin(2, 4)).show(ui, |ui| {
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
    egui::Frame::NONE.inner_margin(crate::widgets::row_margin(4, 4)).show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if text_button(ui, "maskPointColorDelete", crate::i18n::tr("Delete"), false).clicked() {
                let _ = app.run("mask.deletePointColor", json!({"id": m.id, "index": sel}));
            }
        });
    });
}
