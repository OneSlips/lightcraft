//! Photo ▸ Create Camera Profile from Color Chart… (LR-PROF-CREATE, LR-PROF-DUAL): the chart photo
//! with the patch grid found on it (corners can be dragged), the light, rendering and name, an
//! optional second chart photo under another light for a dual-illuminant profile, and after
//! building, each light's per-patch ΔE report. Decoding, finding and fitting run on worker threads
//! with a row in the activity stack ([`crate::tasks::spawn`]); the profile lands among the camera
//! profiles through [`lightcraft_engine::chart_profiling::install`].

use std::sync::Arc;

use lightcraft_catalog::PhotoId;
use lightcraft_engine::chart_profiling::{self as cp, Prepared, Request, camprof};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::Dialog;

/// Activity-stack labels (translated by the stack).
const PREPARING: &str = "Reading the color chart photo";
const BUILDING: &str = "Creating camera profile";

/// The dialog's choices (`Dialog::ChartProfile`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChartProfileDialog {
    /// Profile name (empty: named after the light).
    pub name: String,
    /// Each photo's light: `auto` or one of [`camprof::ILLUMINANTS`].
    pub lights: [String; 2],
    pub natural: bool,
    /// Each photo's chart corners (normalised to its frame) and quarter turns, once known.
    pub corners: [Option<[[f64; 2]; 4]>; 2],
    pub turns: [u8; 2],
    /// Which photo the picture shows (0 or 1).
    pub slot: usize,
    /// Which light's report is shown (index into the report's calibrations).
    pub report_tab: usize,
}

impl Default for ChartProfileDialog {
    fn default() -> Self {
        ChartProfileDialog {
            name: String::new(),
            lights: ["auto".into(), "auto".into()],
            natural: false,
            corners: [None, None],
            turns: [0, 0],
            slot: 0,
            report_tab: 0,
        }
    }
}

/// One chart photo in the dialog.
pub struct Shot {
    pub photo: Option<PhotoId>,
    pub path: String,
    pub prepared: Option<Arc<Prepared>>,
    pub texture: Option<egui::TextureHandle>,
    pub error: Option<String>,
}

/// The dialog's photos and results (kept on the app; the choices live in the dialog).
#[derive(Default)]
pub struct ChartProfileState {
    pub shots: [Option<Shot>; 2],
    /// The last profile built here: `profile.createFromChart`'s answer.
    pub report: Option<Value>,
    pub error: Option<String>,
    /// The corner being dragged.
    drag: Option<usize>,
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Decode a photo for `slot` on a worker thread; the dialog shows it when it is ready.
fn load(app: &mut LightcraftApp, slot: usize, photo: Option<PhotoId>, path: String) -> Result<Value, String> {
    let Some(entry) = app.chart_profile.shots.get_mut(slot) else { return Err("no such chart slot".into()) };
    *entry = Some(Shot { photo, path: path.clone(), prepared: None, texture: None, error: None });
    app.chart_profile.report = None;
    let (p, shown) = (path.clone(), file_name(&path));
    crate::tasks::spawn(
        app,
        PREPARING,
        Some("cameraProfile"),
        move || cp::prepare_path(&p),
        move |app, ctx, result| {
            let Some(Some(shot)) = app.chart_profile.shots.get_mut(slot) else { return };
            if shot.path != path {
                return; // replaced meanwhile
            }
            match result {
                Ok(prep) => {
                    let img = &prep.preview;
                    let color = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.as_bytes());
                    shot.texture = Some(ctx.load_texture(format!("chart-profile-{slot}"), color, egui::TextureOptions::LINEAR));
                    let located = prep.located.map(|l| (cp::normalized_corners(&prep.frame, &l.corners), l.turns));
                    shot.prepared = Some(Arc::new(prep));
                    if let Some(Dialog::ChartProfile { opts }) = &mut app.ui.dialog
                        && let Some((corners, turns)) = located
                    {
                        opts.corners[slot] = Some(corners);
                        opts.turns[slot] = turns;
                    }
                }
                Err(e) => shot.error = Some(e),
            }
        },
    )?;
    Ok(json!({"slot": slot, "path": shown}))
}

/// Open the dialog on the active photo (a raw photo of a ColorChecker); with exactly two photos
/// selected, the other becomes the second chart.
pub fn open(app: &mut LightcraftApp) -> Result<Value, String> {
    let mut ids = app.session.targets(&json!({}));
    if let Some(active) = app.session.active()
        && let Some(i) = ids.iter().position(|x| *x == active)
    {
        ids.swap(0, i);
    }
    let path_of = |app: &LightcraftApp, id: PhotoId| match app.session.catalog.photo(id).map(|p| &p.source) {
        Some(lightcraft_catalog::Source::File { path }) => Some(path.clone()),
        _ => None,
    };
    let first = ids.first().copied().ok_or_else(|| crate::i18n::tr("Select a raw photo of a ColorChecker first").to_string())?;
    let path = path_of(app, first).ok_or_else(|| crate::i18n::tr("The photo has no file to profile from").to_string())?;
    app.chart_profile = ChartProfileState::default();
    app.ui.dialog = Some(Dialog::ChartProfile { opts: ChartProfileDialog::default() });
    load(app, 0, Some(first), path)?;
    // exactly two photos selected: the other is the second chart (another light)
    if ids.len() == 2
        && let Some(&second) = ids.get(1)
        && let Some(p) = path_of(app, second)
    {
        load(app, 1, Some(second), p)?;
    }
    Ok(json!({"photos": ids.len().min(2)}))
}

/// `cameraProfile.second {path?}`: the second chart photo, under another light (asks for the file
/// when no path is given).
pub fn choose_second(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if !matches!(app.ui.dialog, Some(Dialog::ChartProfile { .. })) {
        return Err("the camera profile dialog isn't open".into());
    }
    if let Some(path) = p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        if let Some(Dialog::ChartProfile { opts }) = &mut app.ui.dialog {
            opts.corners[1] = None;
            opts.slot = 1;
        }
        return load(app, 1, None, path.to_string());
    }
    let request = crate::pick::PickRequest::file(crate::i18n::tr("Second Color Chart Photo"), crate::i18n::tr("Raw photos"), RAW_EXTENSIONS);
    match crate::pick::ask(app, "cameraProfile.second", p, "path", request, |svc| svc.pick_files.as_mut().map(|f| f())) {
        crate::pick::Picked::Now(paths) => match paths.first() {
            Some(path) => choose_second(app, &json!({"path": path})),
            None => Ok(Value::Null),
        },
        _ => Ok(Value::Null),
    }
}

/// Raw files the second-chart picker offers.
const RAW_EXTENSIONS: &[&str] = &[
    "dng", "arw", "cr2", "cr3", "nef", "nrw", "raf", "rw2", "orf", "pef", "srw", "x3f", "3fr", "iiq", "erf", "kdc", "mos", "mef", "raw", "rwl",
    "srf", "sr2",
];

/// `cameraProfile.removeSecond`: back to a single-light profile.
pub fn remove_second(app: &mut LightcraftApp) -> Result<Value, String> {
    app.chart_profile.shots[1] = None;
    app.chart_profile.report = None;
    if let Some(Dialog::ChartProfile { opts }) = &mut app.ui.dialog {
        opts.corners[1] = None;
        opts.slot = 0;
        opts.report_tab = 0;
    }
    Ok(Value::Null)
}

/// `cameraProfile.find {slot?}`: look for the chart again; `cameraProfile.turn {slot?}`: patch A01
/// a corner further round; `cameraProfile.align {slot?}`: move the corners onto the chart.
pub fn adjust(app: &mut LightcraftApp, command: &str, p: &Value) -> Result<Value, String> {
    let Some(Dialog::ChartProfile { opts }) = &mut app.ui.dialog else { return Err("the camera profile dialog isn't open".into()) };
    let slot = p.get("slot").and_then(Value::as_u64).map_or(opts.slot, |s| s.min(1) as usize);
    adjust_in(&mut app.chart_profile, opts, command, slot)
}

fn adjust_in(state: &mut ChartProfileState, opts: &mut ChartProfileDialog, command: &str, slot: usize) -> Result<Value, String> {
    let prep = state.shots.get(slot).and_then(|s| s.as_ref()).and_then(|s| s.prepared.clone()).ok_or("the chart photo isn't read yet")?;
    match command {
        "cameraProfile.turn" => opts.turns[slot] = (opts.turns[slot] + 1) % 4,
        "cameraProfile.find" => {
            let found =
                camprof::find_chart(&prep.frame).ok_or_else(|| crate::i18n::tr("No ColorChecker found: drag the corners onto it").to_string())?;
            opts.corners[slot] = Some(cp::normalized_corners(&prep.frame, &found.corners));
            opts.turns[slot] = found.turns;
        }
        _ => {
            let corners = opts.corners[slot].and_then(|c| cp::corners_from_normalized(&prep.frame, c)).ok_or("no corners to align")?;
            let refined = camprof::refine(&prep.frame, &corners, opts.turns[slot])
                .ok_or_else(|| crate::i18n::tr("The corners are too far from the chart to align: drag them closer").to_string())?;
            opts.corners[slot] = Some(cp::normalized_corners(&prep.frame, &refined.corners));
        }
    }
    state.report = None;
    Ok(json!({"slot": slot, "corners": opts.corners[slot], "turns": opts.turns[slot]}))
}

/// The dialog's Create Profile: fit on a worker thread, then file the profile and show the report.
pub fn start_build(app: &mut LightcraftApp, opts: &ChartProfileDialog) -> Result<Value, String> {
    if app.tasks.is_running(BUILDING) {
        return Err(crate::i18n::tr("A camera profile is being built").to_string());
    }
    let shot = |i: usize| app.chart_profile.shots.get(i).and_then(|s| s.as_ref());
    let first = shot(0).and_then(|s| s.prepared.clone()).ok_or_else(|| crate::i18n::tr("The chart photo isn't read yet").to_string())?;
    let second =
        shot(1).map(|s| s.prepared.clone().ok_or_else(|| crate::i18n::tr("The second chart photo isn't read yet").to_string())).transpose()?;
    let request = |i: usize| Request { corners: opts.corners[i], turns: Some(opts.turns[i]), refine: false, illuminant: opts.lights[i].clone() };
    let (req_a, req_b) = (request(0), request(1));
    let options = camprof::BuildOptions {
        illuminant: String::new(),
        natural: opts.natural,
        correction_table: true,
        camera_model: String::new(),
        name: opts.name.clone(),
    };
    let apply: Vec<PhotoId> = (0..2).filter_map(|i| shot(i).and_then(|s| s.photo)).collect();
    app.chart_profile.error = None;
    crate::tasks::spawn(
        app,
        BUILDING,
        Some("cameraProfile"),
        move || {
            let out = cp::build((&first, &req_a), second.as_deref().map(|p| (p, &req_b)), &options)?;
            let preps: Vec<&Prepared> = std::iter::once(first.as_ref()).chain(second.as_deref()).collect();
            let report = cp::report_json(&preps, &out);
            Ok::<_, String>((out.dcp, report))
        },
        move |app, ctx, result| match result.and_then(|(dcp, mut report)| {
            let installed = cp::install(&mut app.session, &dcp, &apply).map_err(|e| e.to_string())?;
            if let (Some(o), Some(i)) = (report.as_object_mut(), installed.as_object()) {
                o.extend(i.clone());
            }
            Ok(report)
        }) {
            Ok(report) => {
                let name = report["name"].as_str().unwrap_or("").to_string();
                app.toast(ctx, crate::i18n::tr_format!("Camera profile “{name}” created", name = name));
                app.chart_profile.report = Some(report);
            }
            Err(e) => app.chart_profile.error = Some(e),
        },
    )?;
    Ok(json!({"started": true}))
}

/// Whether Create Profile can run now.
pub fn can_build(app: &LightcraftApp, opts: &ChartProfileDialog) -> bool {
    let ready = |i: usize| app.chart_profile.shots.get(i).and_then(|s| s.as_ref()).is_some_and(|s| s.prepared.is_some()) && opts.corners[i].is_some();
    !app.tasks.is_running(BUILDING) && ready(0) && (app.chart_profile.shots[1].is_none() || ready(1))
}

/// A Lab (D50) colour as display sRGB.
fn swatch(lab: &Value) -> egui::Color32 {
    let v = |i: usize| lab.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    let xyz = camprof::color::lab_to_xyz(camprof::Vec3::new(v(0), v(1), v(2)), camprof::color::D50);
    let d65 = camprof::color::white_map(camprof::color::Cat::Bradford, camprof::color::D50, camprof::color::D65) * xyz;
    let rgb = lightcraft_color::SRGB.from_xyz().apply([d65[0], d65[1], d65[2]]);
    let e = |c: f64| lightcraft_color::transfer::encode_srgb8(c.clamp(0.0, 1.0) as f32);
    egui::Color32::from_rgb(e(rgb[0]), e(rgb[1]), e(rgb[2]))
}

const LIGHTS: &[(&str, &str)] =
    &[("auto", "Auto (from the chart)"), ("A", "Tungsten (A)"), ("D50", "D50"), ("D55", "Daylight (D55)"), ("D65", "D65"), ("D75", "Shade (D75)")];

fn light_label(v: &str) -> String {
    crate::i18n::tr(LIGHTS.iter().find(|(k, _)| *k == v).map_or(v, |(_, l)| l)).to_string()
}

fn light_combo(ui: &mut egui::Ui, id: &str, value: &mut String) -> bool {
    let mut changed = false;
    let r = egui::ComboBox::from_id_salt(id).selected_text(light_label(value)).width(200.0).show_ui(ui, |ui| {
        for (k, _) in LIGHTS {
            if ui.selectable_label(value == k, light_label(k)).clicked() && value != k {
                *value = k.to_string();
                changed = true;
            }
        }
    });
    crate::widgets::register(ui.ctx(), format!("combo:{id}"), r.response.rect);
    changed
}

/// The picture: the photo with the patch grid and draggable corners.
fn picture(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut ChartProfileDialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let size = egui::vec2(600.0, 400.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 4.0, t.canvas);
    crate::widgets::register(ui.ctx(), "chartProfile.picture", rect);
    let slot = opts.slot.min(1);
    let Some(shot) = app.chart_profile.shots.get(slot).and_then(|s| s.as_ref()) else { return };
    let (Some(tex), Some(prep)) = (&shot.texture, &shot.prepared) else {
        let text = shot.error.clone().unwrap_or_else(|| crate::i18n::tr_format!("Reading {name}…", name = file_name(&shot.path)));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, t.font(13.0), t.text);
        return;
    };
    let [w, h] = tex.size();
    let s = (size.x / w as f32).min(size.y / h as f32);
    let img = egui::Rect::from_center_size(rect.center(), egui::vec2(w as f32 * s, h as f32 * s));
    ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    let to_screen = |p: [f64; 2]| egui::pos2(img.min.x + p[0] as f32 * img.width(), img.min.y + p[1] as f32 * img.height());
    let from_screen = |q: egui::Pos2| [f64::from((q.x - img.min.x) / img.width()), f64::from((q.y - img.min.y) / img.height())];
    // dragging a corner
    if response.drag_started()
        && let (Some(at), Some(c)) = (response.interact_pointer_pos(), opts.corners[slot])
    {
        app.chart_profile.drag =
            (0..4).map(|i| (i, to_screen(c[i]).distance(at))).filter(|(_, d)| *d < 16.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
    }
    if let (Some(i), Some(at)) = (app.chart_profile.drag, response.interact_pointer_pos())
        && response.dragged()
        && let Some(c) = &mut opts.corners[slot]
    {
        c[i] = from_screen(at).map(|v| v.clamp(0.0, 1.0));
        app.chart_profile.report = None;
    }
    if response.drag_stopped() {
        app.chart_profile.drag = None;
    }
    let Some(corners) = opts.corners[slot] else {
        ui.painter().text(
            img.center_bottom() - egui::vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            crate::i18n::tr("No ColorChecker found: use Find Chart, or place the corners"),
            t.font(12.0),
            t.text,
        );
        return;
    };
    // the patch grid: the squares sampled, A01 marked
    let Some(px) = cp::corners_from_normalized(&prep.frame, corners) else { return };
    let chart = camprof::chart();
    let turns = opts.turns[slot] % 4;
    let layout = if turns % 2 == 1 { camprof::reference::Layout { columns: chart.layout.rows, rows: chart.layout.columns } } else { chart.layout };
    let (fw, fh) = (prep.frame.width.max(1) as f64, prep.frame.height.max(1) as f64);
    let extent = px.patch_extent(layout, camprof::SAMPLE_FRACTION);
    let green = egui::Color32::from_rgb(80, 220, 120);
    for i in 0..layout.patches() {
        let c = px.patch_centre(layout, i);
        let a = to_screen([(c[0] - extent[0]) / fw, (c[1] - extent[1]) / fh]);
        let b = to_screen([(c[0] + extent[0]) / fw, (c[1] + extent[1]) / fh]);
        ui.painter().rect_stroke(egui::Rect::from_two_pos(a, b), 0.0, egui::Stroke::new(1.5, green), egui::StrokeKind::Middle);
        let (fr, fc) = (i / layout.columns.max(1), i % layout.columns.max(1));
        if camprof::reference::chart_cell(turns, fr, fc, chart.layout.rows, chart.layout.columns) == (0, 0) {
            let at = to_screen([c[0] / fw, c[1] / fh]);
            ui.painter().text(at, egui::Align2::CENTER_CENTER, "A01", t.semibold(11.0), egui::Color32::WHITE);
        }
    }
    let ring = corners.map(to_screen);
    for i in 0..4 {
        ui.painter().line_segment([ring[i], ring[(i + 1) % 4]], egui::Stroke::new(1.0, green.gamma_multiply(0.6)));
        ui.painter().circle(ring[i], 6.0, egui::Color32::from_black_alpha(140), egui::Stroke::new(2.0, egui::Color32::WHITE));
        crate::widgets::register(ui.ctx(), format!("chartProfile.corner{i}"), egui::Rect::from_center_size(ring[i], egui::vec2(12.0, 12.0)));
    }
}

/// One light's per-patch report: the chart (reference above, profile below) and each patch's ΔE.
fn report_grid(ui: &mut egui::Ui, cal: &Value) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let patches = cal["patches"].as_array().cloned().unwrap_or_default();
    let cell = egui::vec2(40.0, 52.0);
    egui::Grid::new("chartProfileReport").spacing(egui::vec2(3.0, 3.0)).show(ui, |ui| {
        for (i, p) in patches.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::hover());
            let sw = egui::Rect::from_min_size(rect.min, egui::vec2(cell.x, 32.0));
            let (top, bottom) = (
                egui::Rect::from_min_max(sw.min, egui::pos2(sw.max.x, sw.center().y)),
                egui::Rect::from_min_max(egui::pos2(sw.min.x, sw.center().y), sw.max),
            );
            ui.painter().rect_filled(top, 0.0, swatch(&p["referenceLab"]));
            ui.painter().rect_filled(bottom, 0.0, swatch(&p["renderedLab"]));
            let de = p["deltaE"].as_f64().unwrap_or(f64::NAN);
            let colour = if de < 1.0 {
                t.text_dim
            } else if de < 2.5 {
                t.text
            } else {
                t.caution
            };
            ui.painter().text(egui::pos2(rect.center().x, rect.max.y - 9.0), egui::Align2::CENTER_CENTER, format!("{de:.2}"), t.font(10.5), colour);
            resp.on_hover_text(format!("{} {}: ΔE {de:.2}", p["id"].as_str().unwrap_or(""), p["name"].as_str().unwrap_or("")));
            if (i + 1) % 6 == 0 {
                ui.end_row();
            }
        }
    });
}

/// The dialog body.
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut ChartProfileDialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let has_second = app.chart_profile.shots[1].is_some();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            if has_second {
                ui.horizontal(|ui| {
                    for (i, label) in [(0, "First Chart"), (1, "Second Chart")] {
                        if crate::widgets::text_button(ui, &format!("chartProfileSlot-{i}"), label, opts.slot == i).clicked() {
                            opts.slot = i;
                        }
                    }
                });
            }
            opts.slot = if has_second { opts.slot.min(1) } else { 0 };
            picture(app, ui, opts);
            let slot = opts.slot;
            if let Some(shot) = app.chart_profile.shots[slot].as_ref() {
                let mut caption = file_name(&shot.path);
                if let Some(prep) = &shot.prepared {
                    caption = format!("{caption} · {}", prep.camera_model);
                    if let Some(c) = opts.corners[slot].and_then(|c| cp::corners_from_normalized(&prep.frame, c))
                        && let Some((cct, light)) = cp::estimate_light(prep, &camprof::Located { corners: c, turns: opts.turns[slot], score: None })
                    {
                        caption = format!(
                            "{caption} · {}",
                            crate::i18n::tr_format!("light measured {cct:.0} K, nearest {light}", cct = cct, light = light)
                        );
                    }
                }
                ui.label(egui::RichText::new(caption).color(t.text_dim));
            }
            ui.horizontal(|ui| {
                for (id, label, command) in [
                    ("chartProfileFind", "Find Chart", "cameraProfile.find"),
                    ("chartProfileAlign", "Align to Chart", "cameraProfile.align"),
                    ("chartProfileTurn", "Turn A01", "cameraProfile.turn"),
                ] {
                    let r = ui.button(crate::i18n::tr(label));
                    crate::widgets::register(ui.ctx(), format!("button:{id}"), r.rect);
                    if r.clicked() {
                        app.chart_profile.error = adjust_in(&mut app.chart_profile, opts, command, slot).err();
                    }
                }
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(250.0);
            ui.label(egui::RichText::new(crate::i18n::tr("Profile name")).color(t.text_label));
            crate::text_field::TextField::singleline("chartProfileName", &mut opts.name)
                .hint(crate::i18n::tr("Named after the light"))
                .label(crate::i18n::tr("Profile name"))
                .width(240.0)
                .show(ui);
            ui.label(egui::RichText::new(crate::i18n::tr(if has_second { "Light of the first chart" } else { "Light" })).color(t.text_label));
            if light_combo(ui, "chartProfileLight0", &mut opts.lights[0]) {
                app.chart_profile.report = None;
            }
            ui.label(egui::RichText::new(crate::i18n::tr("Rendering")).color(t.text_label));
            ui.horizontal(|ui| {
                for (natural, label) in [(false, "Accurate"), (true, "Natural")] {
                    if crate::widgets::text_button(ui, &format!("chartProfileRendering-{natural}"), label, opts.natural == natural).clicked() {
                        opts.natural = natural;
                        app.chart_profile.report = None;
                    }
                }
            });
            ui.label(
                egui::RichText::new(crate::i18n::tr(if opts.natural {
                    "Hue and saturation from the chart, lightness from the camera."
                } else {
                    "Reproduces the chart's colors."
                }))
                .color(t.text_dim),
            );
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::tr("Second chart (other light)")).color(t.text_label));
            match app.chart_profile.shots[1].as_ref().map(|s| file_name(&s.path)) {
                None => {
                    ui.label(
                        egui::RichText::new(crate::i18n::tr(
                            "Optional: a chart photo under another light, such as tungsten, makes a dual-illuminant profile.",
                        ))
                        .color(t.text_dim),
                    );
                    let r = ui.button(crate::i18n::tr("Add Second Chart…"));
                    crate::widgets::register(ui.ctx(), "button:chartProfileSecond", r.rect);
                    if r.clicked() {
                        // the file dialog answers through `cameraProfile.second {path}`
                        app.chart_profile.error = choose_second(app, &json!({})).err();
                    }
                }
                Some(name) => {
                    ui.label(egui::RichText::new(name).color(t.text));
                    if light_combo(ui, "chartProfileLight1", &mut opts.lights[1]) {
                        app.chart_profile.report = None;
                    }
                    let r = ui.button(crate::i18n::tr("Remove Second Chart"));
                    crate::widgets::register(ui.ctx(), "button:chartProfileRemoveSecond", r.rect);
                    if r.clicked() {
                        app.chart_profile.shots[1] = None;
                        app.chart_profile.report = None;
                        opts.corners[1] = None;
                        opts.slot = 0;
                        opts.report_tab = 0;
                    }
                }
            }
            ui.separator();
            if app.tasks.is_running(BUILDING) {
                ui.label(egui::RichText::new(crate::i18n::tr("Building the profile…")).color(t.text_dim));
            }
            if let Some(e) = &app.chart_profile.error {
                ui.label(egui::RichText::new(e.as_str()).color(t.caution));
            }
            if let Some(report) = app.chart_profile.report.clone() {
                ui.label(
                    egui::RichText::new(crate::i18n::tr_format!("“{name}” added to Camera Profiles", name = report["name"].as_str().unwrap_or("")))
                        .color(t.text),
                );
                let cals = report["calibrations"].as_array().cloned().unwrap_or_default();
                if cals.len() > 1 {
                    ui.horizontal(|ui| {
                        for (i, c) in cals.iter().enumerate() {
                            let label = c["illuminant"].as_str().unwrap_or("").to_string();
                            if crate::widgets::text_button(ui, &format!("labelSet-chartProfileReport-{i}"), &label, opts.report_tab == i).clicked() {
                                opts.report_tab = i;
                            }
                        }
                    });
                }
                let tab = opts.report_tab.min(cals.len().saturating_sub(1));
                if let Some(cal) = cals.get(tab) {
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!(
                            "{light}: mean ΔE {mean:.2}, largest {max:.2} ({n} patches)",
                            light = cal["illuminant"].as_str().unwrap_or(""),
                            mean = cal["meanDeltaE"].as_f64().unwrap_or(0.0),
                            max = cal["maxDeltaE"].as_f64().unwrap_or(0.0),
                            n = cal["patchesUsed"].as_u64().unwrap_or(0)
                        ))
                        .color(t.text),
                    );
                    report_grid(ui, cal);
                    ui.label(egui::RichText::new(crate::i18n::tr("Above: the chart. Below: the profile. ΔE is CIEDE2000.")).color(t.text_dim));
                }
            }
        });
    });
}
