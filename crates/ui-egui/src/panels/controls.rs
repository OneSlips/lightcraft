//! Settings ▸ Controls: control surfaces.
//!
//! The page is built around the device itself, as Logi Options+ shows it: pick a surface at the
//! top (MX Creative Keypad, Dialpad, Actions Ring, …, each with its connected / offline state),
//! see a drawing of it with every key and dial showing its icon and name, click one and set what
//! it does in the panel beside it (actions in categories with a search box, a value's own dial
//! speed). The rest sits in sections under the device bar: pages and the pages that follow the
//! app, dial feel, the readout and the keys' look, and profiles, pairing and the plugin install.
//! Everything is saved to `control-surfaces.json` (`crate::surface`).
//!
//! Portions © RAD Films - Roman Alurkoff (from TactileBridge), MIT: the settings design
//! (TactileBridge's SettingsForm: global and per-value dial feel, workflow presets, bridge
//! timing, OSD, plugin colours, tool contexts, Auto Sync, tool switching).

use egui::{Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_surface as ls;
use ls::settings::OsdPosition;
use ls::{Binding, PageGroup};
use serde_json::{Value, json};

use super::settings::{check, choices, heading, hint, row};
use crate::LightcraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::register;

// ------------------------------------------------------------------------------------- devices

/// A surface in the device bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drawing {
    Keypad,
    Dialpad,
    /// Set up in its maker's app with LightCraft's actions.
    Elsewhere,
    /// The Loupedeck+: knobs and buttons without pages, Loupedeck's own numbers.
    LoupedeckPlus,
    /// Support is still to come.
    Later,
}

/// (id, name, drawing, controls) of the surfaces in the device bar, in order.
const DEVICES: &[(&str, &str, Drawing)] = &[
    ("keypad", "MX Creative Keypad", Drawing::Keypad),
    ("dialpad", "MX Creative Dialpad", Drawing::Dialpad),
    ("ring", "Actions Ring", Drawing::Elsewhere),
    ("loupedeck-plus", "Loupedeck+", Drawing::LoupedeckPlus),
    ("loupedeck", "Loupedeck Live / CT", Drawing::Later),
    ("midi", "MIDI controllers", Drawing::Later),
    ("tangent", "Tangent panels", Drawing::Later),
];

const KEYPAD: [&str; 9] = ["key.1", "key.2", "key.3", "key.4", "key.5", "key.6", "key.7", "key.8", "key.9"];
const DIALPAD: [&str; 6] = ["dial", "roller", "pad.1", "pad.2", "pad.3", "pad.4"];

/// The device bar entry a control belongs to.
fn device_of(control: &str) -> &'static str {
    if KEYPAD.contains(&control) {
        "keypad"
    } else if control.starts_with("ld.") {
        "loupedeck-plus"
    } else {
        "dialpad"
    }
}

fn current_device(app: &LightcraftApp) -> (&'static str, &'static str, Drawing) {
    let id = app.surface.editor.device.as_str();
    DEVICES.iter().copied().find(|d| d.0 == id).unwrap_or(DEVICES[0])
}

/// Whether the MX Creative Console's plugin is connected now.
fn console_connected(app: &LightcraftApp) -> bool {
    app.surface.connected().iter().any(|(_, d)| *d == ls::devices::MX_CREATIVE_CONSOLE.name)
}

/// (connected, status text) of a device bar entry.
fn device_status(app: &LightcraftApp, drawing: Drawing) -> (bool, &'static str) {
    match drawing {
        Drawing::Keypad | Drawing::Dialpad | Drawing::LoupedeckPlus => {
            if !app.surface.settings().enabled {
                (false, "Off")
            } else if console_connected(app) {
                (true, "Connected")
            } else {
                (false, "Offline")
            }
        }
        Drawing::Elsewhere => {
            let installed = app.surface.plugin.status.as_ref().is_some_and(|s| s.installed.is_some());
            (installed && console_connected(app), if installed { "In Logi Options+" } else { "Offline" })
        }
        Drawing::Later => (false, "Coming later"),
    }
}

// ------------------------------------------------------------------------------------- widgets

fn slider(ui: &mut egui::Ui, id: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, decimals: usize, suffix: &str) -> bool {
    let r = ui.add(egui::Slider::new(v, range).fixed_decimals(decimals).suffix(suffix));
    register(ui.ctx(), format!("slider:{id}"), r.rect);
    r.changed()
}

fn slider_u32(ui: &mut egui::Ui, id: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>, suffix: &str) -> bool {
    let r = ui.add(egui::Slider::new(v, range).suffix(suffix));
    register(ui.ctx(), format!("slider:{id}"), r.rect);
    r.changed()
}

fn button(ui: &mut egui::Ui, id: &str, label: &str) -> bool {
    let r = ui.button(tr(label));
    register(ui.ctx(), format!("button:{id}"), r.rect);
    r.clicked()
}

fn button_enabled(ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(tr(label)));
    register(ui.ctx(), format!("button:{id}"), r.rect);
    r.clicked()
}

fn warn_color() -> Color32 {
    Color32::from_rgb(232, 168, 56)
}

fn ok_color() -> Color32 {
    Color32::from_rgb(92, 196, 120)
}

/// A status dot and its text.
fn status(ui: &mut egui::Ui, t: &Tokens, on: bool, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, if on { ok_color() } else { t.text_disabled });
    ui.label(RichText::new(tr(text)).size(11.0).color(if on { t.text } else { t.text_dim }));
}

// ------------------------------------------------------------------------------------- the page

/// Sections under the device bar: (id, label).
const SECTIONS: &[(&str, &str)] = &[
    ("layout", "Keys and Dials"),
    ("pages", "Pages and Tools"),
    ("feel", "Dial Feel"),
    ("readout", "Readout and Look"),
    ("setup", "Profiles and Setup"),
];

pub fn settings_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    if let Some(c) = app.surface.runtime.take_learned() {
        app.surface.editor.device = device_of(&c).to_string();
        app.surface.editor.section = "layout".into();
        app.surface.editor.control = Some(c);
    }
    crate::surface_plugin::refresh(app, false);
    top_bar(app, ui, t);
    banners(app, ui, t);
    device_bar(app, ui, t);
    ui.separator();
    let section = if app.surface.editor.section.is_empty() { "layout".to_string() } else { app.surface.editor.section.clone() };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (id, label) in SECTIONS {
            if crate::widgets::text_button(ui, &format!("surfaceSection-{id}"), label, section == *id).clicked() {
                app.surface.editor.section = id.to_string();
            }
        }
    });
    ui.add_space(4.0);
    let max_h = (ui.ctx().content_rect().height() * 0.62).max(220.0);
    match section.as_str() {
        "pages" => {
            egui::ScrollArea::vertical().id_salt("surfacePages").max_height(max_h).show(ui, |ui| pages_section(app, ui, t));
        }
        "feel" => {
            egui::ScrollArea::vertical().id_salt("surfaceFeel").max_height(max_h).show(ui, |ui| feel(app, ui, t));
        }
        "readout" => {
            egui::ScrollArea::vertical().id_salt("surfaceReadout").max_height(max_h).show(ui, |ui| timing_and_readout(app, ui, t));
        }
        "setup" => {
            egui::ScrollArea::vertical().id_salt("surfaceSetup").max_height(max_h).show(ui, |ui| {
                plugin_section(app, ui, t);
                profiles(app, ui, t);
                connection(app, ui, t);
            });
        }
        _ => layout_section(app, ui, t, max_h),
    }
}

/// On / off, the profile, and where the plugin stands, in one line.
fn top_bar(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal_wrapped(|ui| {
        let mut on = app.surface.settings().enabled;
        if check(ui, "surface.enabled", &mut on, "Use control surfaces") {
            let s = app.surface.runtime.settings_mut();
            s.enabled = on;
            // off stays off; on lets LightCraft set up an installed plugin by itself again
            s.off_by_user = !on;
            app.surface.changed();
            if on {
                crate::surface_plugin::auto_setup(app);
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new(tr("Profile")).color(t.text_label));
        let names: Vec<String> = app.surface.settings().profiles.iter().map(|p| p.name.clone()).collect();
        let current = app.surface.settings().profile.clone();
        let r = egui::ComboBox::from_id_salt("surfaceProfile").selected_text(&current).width(140.0).show_ui(ui, |ui| {
            for n in &names {
                if ui.selectable_label(*n == current, n).clicked() && *n != current {
                    let _ = app.run("surface.profile", json!({"profile": n}));
                    app.surface.editor.page = None;
                }
            }
        });
        register(ui.ctx(), "combo:surfaceProfile", r.response.rect);
    });
}

/// Pairing requests and a missing plugin: shown above everything until dealt with.
fn banners(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let waiting: Vec<_> = app.surface.pairing.waiting().cloned().collect();
    for w in waiting {
        egui::Frame::new().fill(t.inset).stroke(Stroke::new(1.0, t.accent)).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(crate::i18n::tr_format!("{name} asks to pair. Code: {code}", name = w.name.clone(), code = w.code.clone()))
                        .color(t.text),
                );
                if button(ui, &format!("surfaceApprove-{}", w.client), "Approve") {
                    let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let mut s = app.surface.settings().clone();
                    if app.surface.pairing.approve(&w.request, &mut s, now).is_ok() {
                        app.surface.runtime.settings_mut().paired = s.paired;
                        app.surface.changed();
                    }
                }
                if button(ui, &format!("surfaceDeny-{}", w.client), "Deny") {
                    app.surface.pairing.deny(&w.request);
                }
            });
            hint(ui, t, "Check that the keypad shows the same code.");
        });
    }
    let st = app.surface.plugin.status.clone();
    if let Some(s) = st.filter(|s| s.host.is_some() && (s.installed.is_none() || s.update_available()) && s.package.is_some())
        && app.surface.plugin.job.is_none()
    {
        ui.horizontal_wrapped(|ui| {
            let text = if s.installed.is_none() {
                "Logi Options+ is on this computer, but the LightCraft plugin isn't installed yet."
            } else {
                "A newer LightCraft plugin for Logi Options+ comes with this LightCraft."
            };
            ui.label(RichText::new(tr(text)).color(warn_color()));
            let label = if s.installed.is_none() { "Install the plugin" } else { "Update the plugin" };
            if button(ui, "surfaceInstallPluginBanner", label) {
                let r = crate::surface_plugin::start(app, true);
                if let Err(e) = r {
                    app.surface.plugin.message = Some(Err(e));
                }
            }
        });
    }
}

fn device_bar(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (current, _, _) = current_device(app);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (id, name, drawing) in DEVICES {
            let (on, _) = device_status(app, *drawing);
            let selected = *id == current;
            let fill = if selected { t.pressed } else { t.button };
            // one allocated chip (so the bar wraps on a narrow window): a status dot and the name
            let color = if *drawing == Drawing::Later { t.text_dim } else { t.text };
            let galley = ui.painter().layout_no_wrap(tr(name).to_string(), FontId::proportional(11.5), color);
            let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 30.0, 24.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.0, if selected { t.accent } else { t.button_border }),
                StrokeKind::Inside,
            );
            let dot = match drawing {
                Drawing::Later => t.text_disabled,
                _ if on => ok_color(),
                _ => t.text_dim,
            };
            ui.painter().circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, dot);
            ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, color);
            let r = ui.interact(rect, ui.id().with(("surfaceDevice", *id)), Sense::click());
            register(ui.ctx(), format!("button:surfaceDevice-{id}"), r.rect);
            let (_, text) = device_status(app, *drawing);
            let r = r.on_hover_text(tr(text));
            if r.clicked() {
                app.surface.editor.device = id.to_string();
                app.surface.editor.section = "layout".into();
                if app.surface.editor.control.as_deref().is_some_and(|c| device_of(c) != *id) {
                    app.surface.editor.control = None;
                }
                app.surface.editor.page = None;
            }
        }
    });
}

// ------------------------------------------------------------------------------------- layout

const BASE: &str = "\u{0}base";

/// The page group a device's pages belong to.
fn group_of(drawing: Drawing) -> PageGroup {
    if drawing == Drawing::Dialpad { PageGroup::Dials } else { PageGroup::Keys }
}

fn layout_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, max_h: f32) {
    let (_, name, drawing) = current_device(app);
    match drawing {
        Drawing::Elsewhere => {
            heading(ui, t, name);
            hint(
                ui,
                t,
                "The Actions Ring is set up in Logi Options+: open LightCraft's actions there and drag them onto the ring. Every LightCraft action and value is listed there in its category, with its icon.",
            );
            return;
        }
        Drawing::Later => {
            heading(ui, t, name);
            hint(ui, t, "Support for this surface comes later. Its keys and dials will be set up here, like the MX Creative Console's.");
            return;
        }
        Drawing::LoupedeckPlus => {
            loupedeck_section(app, ui, t, max_h);
            return;
        }
        _ => {}
    }
    let Some(profile) = app.surface.settings().active_profile().cloned() else { return };
    let group = group_of(drawing);
    // the pages this device shows: its own pages, then every page (base); tool pages in a menu
    let own: Vec<&ls::Page> = profile.pages.iter().filter(|p| !p.auto && p.in_group(group)).collect();
    let tools: Vec<&ls::Page> = profile.pages.iter().filter(|p| p.auto).collect();
    let page_key = app
        .surface
        .editor
        .page
        .clone()
        // a page of this device's group (a keypad page doesn't carry the dial)
        .filter(|k| k == BASE || profile.page(k).is_some_and(|p| p.auto || p.in_group(group)))
        .unwrap_or_else(|| own.first().map(|p| p.name.clone()).unwrap_or(BASE.into()));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(tr("Page")).color(t.text_label));
        for (i, p) in own.iter().enumerate() {
            if crate::widgets::text_button(ui, &format!("surfacePage-{i}"), &p.name, page_key == p.name).clicked() {
                app.surface.editor.page = Some(p.name.clone());
            }
        }
        if crate::widgets::text_button(ui, "surfacePage-base", "All pages", page_key == BASE)
            .on_hover_text(tr("What a control does on every page that leaves it empty"))
            .clicked()
        {
            app.surface.editor.page = Some(BASE.into());
        }
        if !tools.is_empty() {
            let is_tool = tools.iter().any(|p| p.name == page_key);
            let text = if is_tool { page_key.clone() } else { tr("Tool pages").to_string() };
            let r = egui::ComboBox::from_id_salt("surfaceToolPage").selected_text(text).show_ui(ui, |ui| {
                for p in &tools {
                    if ui.selectable_label(p.name == page_key, &p.name).clicked() {
                        app.surface.editor.page = Some(p.name.clone());
                    }
                }
            });
            register(ui.ctx(), "combo:surfaceToolPage", r.response.rect);
        }
    });
    ui.add_space(4.0);
    // the device shows the page being edited (live), so the drawing and the hardware match
    if page_key != BASE {
        app.surface.runtime.show_page(&page_key);
    }
    let page = profile.page(&page_key).cloned();
    let binding = |c: &str| -> Option<Binding> {
        if page_key == BASE { profile.base.get(c).cloned() } else { page.as_ref().and_then(|p| p.bindings.get(c).cloned()) }
    };
    // (control, its binding, whether that comes from All pages)
    let of = |c: &str| -> (String, Option<Binding>, bool) {
        match binding(c) {
            Some(b) => (c.to_string(), Some(b), false),
            None if page_key != BASE => (c.to_string(), profile.base.get(c).cloned(), profile.base.contains_key(c)),
            None => (c.to_string(), None, false),
        }
    };
    let views: Vec<(String, Option<Binding>, bool)> = match drawing {
        Drawing::Keypad => KEYPAD.iter().map(|c| of(c)).collect(),
        _ => DIALPAD.iter().map(|c| of(c)).collect(),
    };
    let wide = ui.available_width() >= 640.0;
    let draw_w = if wide { 300.0 } else { ui.available_width().min(300.0) };
    let mut clicked: Option<String> = None;
    let mut step: i32 = 0;
    let mut device = |app: &LightcraftApp, ui: &mut egui::Ui| {
        let labels: Vec<View> = views
            .iter()
            .map(|(c, b, inherited)| {
                (c.clone(), *inherited, b.as_ref().map(|b| short_label(app, b)).unwrap_or_default(), b.as_ref().and_then(ls::icons::icon_for))
            })
            .collect();
        let selected = app.surface.editor.control.clone();
        match drawing {
            Drawing::Keypad => draw_keypad(ui, t, draw_w, &labels, selected.as_deref(), &mut clicked, &mut step),
            _ => draw_dialpad(ui, t, draw_w, &labels, selected.as_deref(), &mut clicked),
        }
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(draw_w);
                device(app, ui);
            });
            ui.add_space(10.0);
            ui.vertical(|ui| editor_panel(app, ui, t, &page_key, max_h));
        });
    } else {
        device(app, ui);
        ui.add_space(6.0);
        editor_panel(app, ui, t, &page_key, max_h);
    }
    if let Some(c) = clicked {
        app.surface.editor.control = Some(c);
        app.surface.editor.params_text.clear();
    }
    if step != 0 && !own.is_empty() {
        let i = own.iter().position(|p| p.name == page_key).map_or(0, |i| i as i32);
        let n = own.len() as i32;
        let next = (i + step).rem_euclid(n) as usize;
        if let Some(p) = own.get(next) {
            app.surface.editor.page = Some(p.name.clone());
        }
    }
}

/// The Loupedeck+: every knob and button as Loupedeck numbers them, what each does (its label's
/// own job until set here or imported), Loupedeck's profiles found on this computer to import,
/// and the editor beside them. The console has no pages: its controls hold on every page.
fn loupedeck_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, max_h: f32) {
    heading(ui, t, "Loupedeck+");
    hint(
        ui,
        t,
        "The Loupedeck+ works through the LightCraft plugin. Every knob and button does what its label says until you set it here, or bring over your own Loupedeck profile (only the layout comes along).",
    );
    ui.horizontal_wrapped(|ui| {
        for (i, (name, path)) in crate::surface::loupedeck_profiles().into_iter().enumerate() {
            let r = ui.button(crate::i18n::tr_format!("Import “{name}”", name = name.clone()));
            register(ui.ctx(), format!("button:surfaceImportLoupedeck-{i}"), r.rect);
            if r.on_hover_text(path.to_string_lossy()).clicked() {
                let r = app_run_import(app, &path.to_string_lossy());
                report(app, r);
                app.surface.editor.device = "loupedeck-plus".into();
            }
        }
        if app.services.pick_surface_file.is_some() && button(ui, "surfaceImportLoupedeckFile", "Import a Loupedeck profile…") {
            let files = app.services.pick_surface_file.as_mut().map(|f| f()).unwrap_or_default();
            for f in files {
                let r = app_run_import(app, &f);
                report(app, r);
                app.surface.editor.device = "loupedeck-plus".into();
            }
        }
    });
    if let Some(m) = app.surface.editor.message.clone() {
        ui.label(RichText::new(m).color(t.text_label));
    }
    let Some(profile) = app.surface.settings().active_profile().cloned() else { return };
    let selected = app.surface.editor.control.clone();
    let mut clicked: Option<String> = None;
    let sections: [(&str, &dyn Fn(&str) -> bool); 6] = [
        ("Knobs", &|c: &str| ls::devices::is_encoder(c) && c.strip_prefix("ld.r").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| n > 32)),
        ("P1–P8 (Hue)", &|c: &str| c.strip_prefix("ld.r").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (1..=8).contains(&n))),
        ("P1–P8 (Sat)", &|c: &str| c.strip_prefix("ld.r").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (9..=16).contains(&n))),
        ("P1–P8 (Lum)", &|c: &str| c.strip_prefix("ld.r").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (17..=24).contains(&n))),
        ("P1–P8 (B&W)", &|c: &str| c.strip_prefix("ld.r").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (25..=32).contains(&n))),
        ("Buttons", &|c: &str| c.starts_with("ld.b")),
    ];
    let mut grid = |app: &LightcraftApp, ui: &mut egui::Ui| {
        for (title, keep) in &sections {
            ui.label(RichText::new(tr(title)).font(t.semibold(12.0)).color(t.text_label));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for c in ls::loupedeck::CONTROLS.iter().filter(|c| keep(c.id)) {
                    let own = profile.base.get(c.id).cloned();
                    let inherited = own.is_none();
                    let b = own.or_else(|| ls::loupedeck::default_binding(c.id));
                    let label = b.as_ref().map(|b| short_label(app, b)).unwrap_or_default();
                    let icon = b.as_ref().and_then(ls::icons::icon_for);
                    let (cell, _) = ui.allocate_exact_size(vec2(64.0, 80.0), Sense::hover());
                    let face = Rect::from_min_size(cell.min, vec2(64.0, 64.0));
                    if draw_control(
                        ui,
                        t,
                        face,
                        c.id,
                        (&label, icon, inherited),
                        selected.as_deref() == Some(c.id),
                        c.kind == ls::ControlKind::Encoder,
                    ) {
                        clicked = Some(c.id.to_string());
                    }
                    ui.painter().text(
                        pos2(cell.center().x, cell.bottom() - 7.0),
                        egui::Align2::CENTER_CENTER,
                        tr(c.label),
                        FontId::proportional(9.5),
                        t.text_dim,
                    );
                }
            });
            ui.add_space(4.0);
        }
    };
    let wide = ui.available_width() >= 760.0;
    if wide {
        ui.horizontal_top(|ui| {
            let w = (ui.available_width() - 310.0).max(300.0);
            ui.vertical(|ui| {
                ui.set_width(w);
                egui::ScrollArea::vertical().id_salt("surfaceLoupedeck").max_height(max_h).show(ui, |ui| grid(app, ui));
            });
            ui.add_space(10.0);
            ui.vertical(|ui| editor_panel(app, ui, t, BASE, max_h));
        });
    } else {
        egui::ScrollArea::vertical().id_salt("surfaceLoupedeck").max_height(max_h * 0.6).show(ui, |ui| grid(app, ui));
        ui.add_space(6.0);
        editor_panel(app, ui, t, BASE, max_h);
    }
    if let Some(c) = clicked {
        app.surface.editor.control = Some(c);
        app.surface.editor.params_text.clear();
    }
}

/// One control face: a rounded key with its icon and name.
#[allow(clippy::too_many_arguments)]
fn draw_control(ui: &mut egui::Ui, t: &Tokens, rect: Rect, control: &str, view: (&str, Option<&str>, bool), selected: bool, round: bool) -> bool {
    let (label, icon, inherited) = view;
    let id = ui.id().with(("surfaceControl", control));
    let r = ui.interact(rect, id, Sense::click());
    register(ui.ctx(), format!("button:surfaceControl-{control}"), rect);
    let p = ui.painter();
    let fill = if r.hovered() { Color32::from_gray(42) } else { Color32::from_gray(26) };
    let stroke = if selected { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, Color32::from_gray(70)) };
    if round {
        p.circle_filled(rect.center(), rect.width() / 2.0, fill);
        p.circle_stroke(rect.center(), rect.width() / 2.0, stroke);
    } else {
        p.rect_filled(rect, CornerRadius::same(8), fill);
        p.rect_stroke(rect, CornerRadius::same(8), stroke, StrokeKind::Inside);
    }
    let text_h = if label.is_empty() { 0.0 } else { (rect.height() * 0.3).min(28.0) };
    let icon_side = ((rect.height() - text_h) * 0.62).min(rect.width() * 0.62);
    let mut drew = false;
    if let Some(name) = icon {
        let ir = Rect::from_center_size(pos2(rect.center().x, rect.top() + (rect.height() - text_h) / 2.0 + 2.0), vec2(icon_side, icon_side));
        let tint = if inherited { Color32::from_gray(120) } else { Color32::WHITE };
        drew = crate::surface_icons::paint(ui, name, ir, tint);
    }
    let text = if label.is_empty() { "—".to_string() } else { label.to_string() };
    let font = FontId::proportional(if drew { 10.0 } else { 11.0 });
    let galley = ui.painter().layout(text, font, if label.is_empty() || inherited { t.text_dim } else { t.text }, rect.width() - 8.0);
    let y = if drew { rect.bottom() - text_h + (text_h - galley.size().y).max(0.0) / 2.0 - 2.0 } else { rect.center().y - galley.size().y / 2.0 };
    let pos = pos2(rect.center().x - galley.size().x / 2.0, y);
    ui.painter().galley(pos, galley, t.text);
    let tip = if label.is_empty() {
        tr("Nothing — click to set").to_string()
    } else if inherited {
        format!("{label} ({})", tr("from All pages"))
    } else {
        label.to_string()
    };
    let r = r.on_hover_text(tip);
    r.clicked()
}

/// (control, inherited from All pages, label, icon).
type View = (String, bool, String, Option<&'static str>);

fn draw_keypad(ui: &mut egui::Ui, t: &Tokens, w: f32, keys: &[View], selected: Option<&str>, clicked: &mut Option<String>, step: &mut i32) {
    let pad = 14.0;
    let gap = 8.0;
    let key = ((w - 2.0 * pad - 2.0 * gap) / 3.0).max(40.0);
    let body_h = pad * 2.0 + key * 3.0 + gap * 2.0 + gap + 30.0;
    let (body, _) = ui.allocate_exact_size(vec2(w, body_h), Sense::hover());
    ui.painter().rect_filled(body, CornerRadius::same(18), Color32::from_gray(52));
    ui.painter().rect_stroke(body, CornerRadius::same(18), Stroke::new(1.0, Color32::from_gray(80)), StrokeKind::Inside);
    for (i, (c, inherited, label, icon)) in keys.iter().enumerate() {
        let (row, col) = ((i / 3) as f32, (i % 3) as f32);
        let r = Rect::from_min_size(pos2(body.left() + pad + col * (key + gap), body.top() + pad + row * (key + gap)), vec2(key, key));
        if draw_control(ui, t, r, c, (label, *icon, *inherited), selected == Some(c.as_str()), false) {
            *clicked = Some(c.clone());
        }
    }
    // the page buttons under the keys
    let y = body.top() + pad + 3.0 * key + 3.0 * gap;
    for (i, (s, glyph)) in [(-1, "‹"), (1, "›")].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(body.left() + pad + i as f32 * (key + gap), y), vec2(key, 26.0));
        let resp = ui.interact(r, ui.id().with(("surfacePageButton", i)), Sense::click());
        register(ui.ctx(), format!("button:surfaceKeypadPage-{}", if s < 0 { "previous" } else { "next" }), r);
        ui.painter().rect_filled(r, CornerRadius::same(6), if resp.hovered() { Color32::from_gray(42) } else { Color32::from_gray(30) });
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(16.0), t.text);
        if resp.on_hover_text(tr(if s < 0 { "Previous page" } else { "Next page" })).clicked() {
            *step = s;
        }
    }
}

fn draw_dialpad(ui: &mut egui::Ui, t: &Tokens, w: f32, controls: &[View], selected: Option<&str>, clicked: &mut Option<String>) {
    let pad = 14.0;
    let gap = 8.0;
    let dial = (w * 0.5).min(150.0);
    let btn_w = (w - 2.0 * pad - 3.0 * gap) / 4.0;
    let btn_h = 48.0;
    let body_h = pad * 2.0 + dial + gap * 2.0 + btn_h;
    let (body, _) = ui.allocate_exact_size(vec2(w, body_h), Sense::hover());
    ui.painter().rect_filled(body, CornerRadius::same(18), Color32::from_gray(52));
    ui.painter().rect_stroke(body, CornerRadius::same(18), Stroke::new(1.0, Color32::from_gray(80)), StrokeKind::Inside);
    let find = |id: &str| controls.iter().find(|v| v.0 == id);
    // the roller on the left, the dial on the right
    if let Some((c, inherited, label, icon)) = find("roller") {
        let r = Rect::from_min_size(pos2(body.left() + pad, body.top() + pad), vec2((w - 2.0 * pad - dial - gap * 2.0).max(60.0), dial));
        if draw_control(ui, t, r, c, (label, *icon, *inherited), selected == Some(c.as_str()), false) {
            *clicked = Some(c.clone());
        }
    }
    if let Some((c, inherited, label, icon)) = find("dial") {
        let r = Rect::from_min_size(pos2(body.right() - pad - dial, body.top() + pad), vec2(dial, dial));
        if draw_control(ui, t, r, c, (label, *icon, *inherited), selected == Some(c.as_str()), true) {
            *clicked = Some(c.clone());
        }
    }
    for i in 0..4 {
        let id = format!("pad.{}", i + 1);
        if let Some((c, inherited, label, icon)) = find(&id) {
            let r = Rect::from_min_size(pos2(body.left() + pad + i as f32 * (btn_w + gap), body.top() + pad + dial + gap * 2.0), vec2(btn_w, btn_h));
            if draw_control(ui, t, r, c, (label, *icon, *inherited), selected == Some(c.as_str()), false) {
                *clicked = Some(c.clone());
            }
        }
    }
}

fn short_label(_app: &LightcraftApp, b: &Binding) -> String {
    match b {
        Binding::Command { command, params } => {
            let s = ls::icons::command_label(command, params);
            if s == *command {
                // the registry's own name
                crate::menus::ui_commands().find(|c| c.0 == command).map_or_else(|| s.clone(), |c| tr(c.1).to_string())
            } else {
                tr(&s).to_string()
            }
        }
        other => tr(&other.describe()).to_string(),
    }
}

fn edit_profile(app: &mut LightcraftApp, f: impl FnOnce(&mut ls::Profile)) {
    let s = app.surface.runtime.settings_mut();
    let cur = s.profile.clone();
    if let Some(p) = s.profiles.iter_mut().find(|p| p.name == cur) {
        f(p);
    }
    app.surface.changed();
}

fn edit_page(app: &mut LightcraftApp, page: &str, f: impl FnOnce(&mut ls::Page)) {
    edit_profile(app, |p| {
        if let Some(q) = p.pages.iter_mut().find(|q| q.name == page) {
            f(q);
        }
    });
}

fn set_binding(app: &mut LightcraftApp, page: &str, control: &str, b: Option<Binding>) {
    edit_profile(app, |p| {
        let map = if page == BASE { Some(&mut p.base) } else { p.pages.iter_mut().find(|q| q.name == page).map(|q| &mut q.bindings) };
        if let Some(map) = map {
            match b {
                Some(b) => {
                    map.insert(control.to_string(), b);
                }
                None => {
                    map.remove(control);
                }
            }
        }
    });
}

/// The panel beside the drawing: what the selected control does, and the action picker.
fn editor_panel(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, page: &str, max_h: f32) {
    ui.horizontal_wrapped(|ui| {
        let learning = app.surface.runtime.learning();
        if button(ui, "surfaceLearn", if learning { "Cancel learning" } else { "Learn: touch a control" }) {
            if learning {
                app.surface.runtime.cancel_learn();
            } else {
                app.surface.runtime.arm_learn();
            }
        }
        if learning {
            hint(ui, t, "Turn a dial or press a key on the surface…");
        }
    });
    let Some(control) = app.surface.editor.control.clone().filter(|c| device_of(c) == current_device(app).0) else {
        ui.add_space(6.0);
        hint(ui, t, "Click a key, button or dial on the drawing (or use Learn) to choose what it does.");
        return;
    };
    let current = {
        let Some(p) = app.surface.settings().active_profile() else { return };
        if page == BASE { p.base.get(&control).cloned() } else { p.page(page).and_then(|q| q.bindings.get(&control).cloned()) }
    };
    let name = ls::devices::find_control(&control).map_or(control.clone(), |c| tr(c.label).to_string());
    let page_name = if page == BASE { tr("All pages").to_string() } else { page.to_string() };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(6), Color32::from_gray(26));
        if let Some(icon) = current.as_ref().and_then(ls::icons::icon_for) {
            crate::surface_icons::paint(ui, icon, r.shrink(4.0), Color32::WHITE);
        }
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("{name} · {page_name}")).size(11.0).color(t.text_dim));
            let what = current.as_ref().map_or_else(|| tr("Nothing").to_string(), |b| short_label(app, b));
            ui.label(RichText::new(what).font(t.semibold(13.0)).color(t.text));
        });
    });
    ui.horizontal_wrapped(|ui| {
        if button_enabled(ui, "surfaceClear", "Clear", current.is_some()) {
            set_binding(app, page, &control, None);
        }
        if let Some(Binding::Value { .. }) = &current
            && !ls::devices::is_encoder(&control)
        {
            hint(ui, t, "Press this key to put the value on the dial; press it again to give the dial back.");
        }
    });
    // a value's own dial feel
    if let Some(Binding::Value { target }) = &current {
        let mut tu = app.surface.settings().tuning_of(target);
        let mut c = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Its speed")).color(t.text_label));
            c |= slider(ui, "surface.valueSpeed", &mut tu.speed, 0.1..=5.0, 2, "×");
            ui.label(RichText::new(tr("Its acceleration")).color(t.text_label));
            c |= slider(ui, "surface.valueAccel", &mut tu.acceleration, 0.1..=4.0, 2, "×");
        });
        if c {
            app.surface.runtime.settings_mut().tuning.insert(target.clone(), tu.sanitized());
            app.surface.changed();
        }
    }
    if let Some(Binding::Command { command, params }) = &current {
        egui::CollapsingHeader::new(tr("Command parameters (advanced)")).id_salt("surfaceParams").show(ui, |ui| {
            if app.surface.editor.params_text.is_empty() {
                app.surface.editor.params_text = if params.is_null() { "{}".into() } else { params.to_string() };
            }
            crate::text_field::TextField::singleline("field:surfaceParams", &mut app.surface.editor.params_text).width(240.0).show(ui);
            match serde_json::from_str::<Value>(&app.surface.editor.params_text) {
                Ok(v) if v.is_object() || v.is_null() => {
                    let v = if v.as_object().is_some_and(|o| o.is_empty()) { Value::Null } else { v };
                    if v != *params {
                        set_binding(app, page, &control, Some(Binding::command_with(command, v)));
                    }
                }
                _ => hint(ui, t, "Parameters are a JSON object, e.g. {\"rating\": 5}"),
            }
        });
    }
    ui.add_space(4.0);
    crate::text_field::TextField::singleline("field:surfaceActionSearch", &mut app.surface.editor.filter)
        .hint(tr("Search actions"))
        .width(240.0)
        .show(ui);
    let picked = action_picker(app, ui, t, &control, current.as_ref(), max_h - 120.0);
    if let Some(b) = picked {
        set_binding(app, page, &control, Some(b));
        app.surface.editor.params_text.clear();
    }
}

/// Every action in categories, filtered by the search box; the chosen one.
fn action_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, control: &str, current: Option<&Binding>, height: f32) -> Option<Binding> {
    let q = app.surface.editor.filter.trim().to_lowercase();
    let encoder = ls::devices::is_encoder(control);
    // (group, label, icon, key, binding)
    let mut items: Vec<(String, String, Option<&'static str>, String, Binding)> = Vec::new();
    // keys: what a key does once first, then the values it can put on the dial
    let mut order: Vec<&ls::catalog::Entry> = ls::catalog::cached().iter().collect();
    if !encoder {
        order.sort_by_key(|e| e.value);
    }
    for e in order {
        // a dial turns values (and pages); keys do anything
        if encoder && !e.value && !matches!(e.binding, Binding::PageNext { .. } | Binding::PagePrevious { .. }) {
            continue;
        }
        let label = match &e.binding {
            Binding::Command { .. } => short_label(app, &e.binding),
            _ => tr(&e.label).to_string(),
        };
        items.push((tr(e.group).to_string(), label, e.icon, e.key.clone(), e.binding.clone()));
    }
    if !encoder && let Some(p) = app.surface.settings().active_profile() {
        let g = tr("Pages and Profiles").to_string();
        for pg in p.pages.iter().filter(|p| !p.auto) {
            let b = Binding::Page { page: pg.name.clone() };
            items.push((
                g.clone(),
                crate::i18n::tr_format!("Show page {name}", name = pg.name.clone()),
                Some("toggle-tools"),
                ls::catalog::key_of(&b),
                b,
            ));
        }
        for pr in &app.surface.settings().profiles {
            let b = Binding::Profile { profile: pr.name.clone() };
            items.push((
                g.clone(),
                crate::i18n::tr_format!("Switch to profile {name}", name = pr.name.clone()),
                Some("toggle-tools"),
                ls::catalog::key_of(&b),
                b,
            ));
        }
    }
    if !encoder {
        // every other command of the app, for power users
        let g = tr("All commands").to_string();
        let mut all: Vec<(String, String)> = crate::menus::ui_commands().map(|c| (c.0.to_string(), tr(c.1).to_string())).collect();
        all.extend(app.session.commands().into_iter().map(|c| (c.id.to_string(), tr(c.label).to_string())));
        all.sort_by(|a, b| a.1.cmp(&b.1));
        all.dedup_by(|a, b| a.0 == b.0);
        for (id, label) in all {
            let b = Binding::command(&id);
            let key = ls::catalog::key_of(&b);
            if ls::catalog::of(&b).is_some() {
                continue;
            }
            items.push((g.clone(), label, None, key, b));
        }
    }
    let matches = |label: &str, key: &str, group: &str| {
        q.is_empty() || label.to_lowercase().contains(&q) || key.to_lowercase().contains(&q) || group.to_lowercase().contains(&q)
    };
    let mut groups: Vec<String> = Vec::new();
    for it in &items {
        if !groups.contains(&it.0) {
            groups.push(it.0.clone());
        }
    }
    let current_key = current.map(ls::catalog::key_of);
    let mut picked = None;
    egui::ScrollArea::vertical().id_salt("surfaceActions").max_height(height.max(140.0)).auto_shrink([false, true]).show(ui, |ui| {
        let mut shown = 0usize;
        for g in &groups {
            let rows: Vec<_> = items.iter().filter(|it| &it.0 == g && matches(&it.1, &it.3, &it.0)).collect();
            if rows.is_empty() {
                continue;
            }
            let has_current = rows.iter().any(|it| Some(&it.3) == current_key.as_ref());
            let open = if q.is_empty() { None } else { Some(true) };
            let mut h = egui::CollapsingHeader::new(RichText::new(format!("{g}  ({})", rows.len())).color(t.text))
                .id_salt(("surfaceGroup", g.as_str()))
                .default_open(has_current);
            if let Some(o) = open {
                h = h.open(Some(o));
            }
            let r = h.show(ui, |ui| {
                for (_, label, icon, key, b) in rows.iter().map(|x| (&x.0, &x.1, x.2, &x.3, &x.4)) {
                    shown += 1;
                    if shown > 600 {
                        break;
                    }
                    let selected = Some(key) == current_key.as_ref();
                    let resp = ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        if let Some(name) = icon {
                            crate::surface_icons::paint(ui, name, ir, Color32::WHITE);
                        }
                        ui.add(egui::Button::selectable(selected, label.as_str()))
                    });
                    register(ui.ctx(), format!("item:surfaceAction-{key}"), resp.inner.rect);
                    if resp.inner.clicked() {
                        picked = Some(b.clone());
                    }
                }
            });
            register(ui.ctx(), format!("group:surfaceActions-{g}"), r.header_response.rect);
        }
        if shown == 0 {
            hint(ui, t, "No action matches.");
        }
    });
    picked
}

// ------------------------------------------------------------------------------------- pages

fn pages_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let Some(profile) = app.surface.settings().active_profile().cloned() else { return };
    heading(ui, t, "Pages");
    hint(ui, t, "Keypad pages and dial pages of this profile. A tool page appears by itself while its tool is open (see below).");
    for (i, p) in profile.pages.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&p.name).color(t.text)).on_hover_text(tr("Page"));
            let mut keys = p.in_group(PageGroup::Keys);
            let mut dials = p.in_group(PageGroup::Dials);
            let mut auto = p.auto;
            let a = check(ui, &format!("surface.pageKeys-{i}"), &mut keys, "Keys");
            let b = check(ui, &format!("surface.pageDials-{i}"), &mut dials, "Dials");
            let c = check(ui, &format!("surface.pageAuto-{i}"), &mut auto, "Tool page");
            if a || b || c {
                let name = p.name.clone();
                edit_page(app, &name, |q| {
                    q.groups = match (keys, dials) {
                        (true, false) => vec![PageGroup::Keys],
                        (false, true) => vec![PageGroup::Dials],
                        _ => Vec::new(),
                    };
                    q.auto = auto;
                });
            }
            if button(ui, &format!("surfaceRenamePage-{i}"), "Rename") {
                let new = app.surface.editor.new_name.trim().to_string();
                let old = p.name.clone();
                if !new.is_empty() && profile.page(&new).is_none() {
                    edit_profile(app, |pr| {
                        if let Some(q) = pr.pages.iter_mut().find(|q| q.name == old) {
                            q.name = new.clone();
                        }
                        for list in pr.contexts.values_mut() {
                            for n in list.iter_mut().filter(|n| **n == old) {
                                *n = new.clone();
                            }
                        }
                    });
                    app.surface.editor.new_name.clear();
                }
            }
            if button(ui, &format!("surfaceDeletePage-{i}"), "Delete") {
                let name = p.name.clone();
                edit_profile(app, |pr| pr.pages.retain(|q| q.name != name));
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        crate::text_field::TextField::singleline("field:surfaceNewName", &mut app.surface.editor.new_name).hint(tr("Name")).width(160.0).show(ui);
        if button(ui, "surfaceAddPage", "Add page") {
            let name = app.surface.editor.new_name.trim().to_string();
            let name = if name.is_empty() { format!("Page {}", profile.pages.len() + 1) } else { name };
            edit_profile(app, |p| p.pages.push(ls::Page { name: name.clone(), groups: vec![PageGroup::Keys], ..Default::default() }));
            app.surface.editor.new_name.clear();
        }
    });
    hint(ui, t, "Type a name, then Add page, or Rename on a page.");
    contexts(app, ui, t);
}

fn contexts(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Pages That Follow the App");
    let mut follow = app.surface.settings().follow_context;
    if check(ui, "surface.followContext", &mut follow, "Switch pages with what LightCraft shows (Crop, Masking…)") {
        app.surface.runtime.settings_mut().follow_context = follow;
        app.surface.changed();
    }
    let mut dialogs = app.surface.settings().dialog_pages;
    if check(ui, "surface.dialogPages", &mut dialogs, "Dialogs show their own page (OK, Cancel…)") {
        app.surface.runtime.settings_mut().dialog_pages = dialogs;
        app.surface.changed();
    }
    let Some(profile) = app.surface.settings().active_profile().cloned() else { return };
    ui.label(
        RichText::new(crate::i18n::tr_format!("Now: {context}", context = tr(ls::context::label(app.surface.runtime.context())).to_string()))
            .size(11.0)
            .color(t.text_dim),
    );
    for (key, label) in ls::context::KNOWN {
        let current = profile.contexts.get(*key).and_then(|l| l.first()).cloned();
        row(ui, t, label, |ui| {
            let r = egui::ComboBox::from_id_salt(format!("surfaceContext-{key}"))
                .selected_text(current.clone().unwrap_or_else(|| "—".into()))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "—").clicked() {
                        let k = key.to_string();
                        edit_profile(app, |p| {
                            p.contexts.remove(&k);
                        });
                    }
                    for pg in &profile.pages {
                        if ui.selectable_label(current.as_deref() == Some(&pg.name), &pg.name).clicked() {
                            let (k, n) = (key.to_string(), pg.name.clone());
                            edit_profile(app, |p| {
                                p.contexts.insert(k, vec![n]);
                            });
                        }
                    }
                });
            register(ui.ctx(), format!("combo:surfaceContext-{key}"), r.response.rect);
        });
    }
}

// ------------------------------------------------------------------------------------- setup

fn plugin_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Logi Options+ Plugin");
    hint(
        ui,
        t,
        "The MX Creative Console talks to LightCraft through the LightCraft plugin for Logi Options+. Installing hands the plugin to Logi Options+, which asks you to confirm.",
    );
    if app.services.surface_plugin.is_none() {
        hint(ui, t, "This LightCraft can't install it: open LightCraft.lplug4 from the LightCraft download to install it in Logi Options+.");
        return;
    }
    let s = app.surface.plugin.status.clone().unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        status(ui, t, s.host.is_some(), if s.host.is_some() { "Logi Options+ found" } else { "Logi Options+ not found" });
        ui.add_space(8.0);
        match &s.installed {
            Some(v) => status(ui, t, true, &crate::i18n::tr_format!("Plugin {version} installed", version = v.clone())),
            None => status(ui, t, false, "Plugin not installed"),
        }
        ui.add_space(8.0);
        status(ui, t, console_connected(app), if console_connected(app) { "Connected" } else { "Not connected" });
    });
    let busy = app.surface.plugin.job.as_ref().map(|j| j.installing);
    ui.horizontal_wrapped(|ui| {
        let label = if s.update_available() {
            "Update the plugin"
        } else if s.installed.is_some() {
            "Reinstall the plugin"
        } else {
            "Install the plugin"
        };
        if button_enabled(ui, "surfaceInstallPlugin", label, busy.is_none() && s.can_install())
            && let Err(e) = crate::surface_plugin::start(app, true)
        {
            app.surface.plugin.message = Some(Err(e));
        }
        if button_enabled(ui, "surfaceUninstallPlugin", "Remove the plugin", busy.is_none() && s.installed.is_some() && s.package.is_some())
            && let Err(e) = crate::surface_plugin::start(app, false)
        {
            app.surface.plugin.message = Some(Err(e));
        }
        match busy {
            Some(true) => hint(ui, t, "Installing…"),
            Some(false) => hint(ui, t, "Removing…"),
            None => {}
        }
    });
    if s.package.is_none() && s.host.is_some() {
        hint(
            ui,
            t,
            "No plugin package came with this LightCraft (LightCraft.lplug4 next to the program). Developers: build it with the plugin's scripts/package.ps1 and point LIGHTCRAFT_LOGI_PLUGIN at it.",
        );
    }
    match &app.surface.plugin.message {
        Some(Ok(m)) => {
            ui.label(RichText::new(tr(m)).color(t.text_label));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e.clone()).color(warn_color()));
        }
        None => {}
    }
}

fn connection(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Connection");
    row(ui, t, "Port", |ui| {
        let mut port = u32::from(app.surface.settings().port);
        let r = ui.add(egui::DragValue::new(&mut port).range(1024..=65535));
        register(ui.ctx(), "drag:surfacePort", r.rect);
        if r.changed() {
            app.surface.runtime.settings_mut().port = u16::try_from(port).unwrap_or(ls::settings::DEFAULT_PORT);
            app.surface.changed();
        }
        match (app.surface.listening, &app.surface.listen_error) {
            (Some(p), _) => {
                ui.label(RichText::new(crate::i18n::tr_format!("Listening on 127.0.0.1:{port}", port = p)).size(11.0).color(t.text_dim));
            }
            (None, Some(e)) => {
                ui.label(RichText::new(e.clone()).size(11.0).color(warn_color()));
            }
            _ => {}
        }
    });
    if let Some(e) = app.surface.load_error.clone().or(app.surface.save_error.clone()) {
        ui.label(RichText::new(e).color(warn_color()));
    }
    let connected = app.surface.connected();
    if connected.is_empty() {
        hint(ui, t, "No surface is connected.");
    } else {
        for (client, device) in connected {
            ui.horizontal(|ui| status(ui, t, true, &format!("{client} — {device}")));
        }
    }
    let paired: Vec<(String, String)> = app.surface.settings().paired.iter().map(|c| (c.client.clone(), c.name.clone())).collect();
    for (client, name) in paired {
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::i18n::tr_format!("Paired: {name}", name = name)).color(t.text_label));
            if button(ui, &format!("surfaceForget-{client}"), "Forget") {
                app.surface.unpair(&client);
            }
        });
    }
}

fn profiles(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Profiles");
    let names: Vec<String> = app.surface.settings().profiles.iter().map(|p| p.name.clone()).collect();
    if let Some(desc) = app.surface.settings().active_profile().map(|p| p.description.clone()).filter(|d| !d.is_empty()) {
        hint(ui, t, &desc);
    }
    ui.horizontal_wrapped(|ui| {
        crate::text_field::TextField::singleline("field:surfaceProfileName", &mut app.surface.editor.new_name).hint(tr("Name")).width(160.0).show(ui);
        if button(ui, "surfaceDuplicate", "Duplicate") {
            let name = app.surface.editor.new_name.trim().to_string();
            let s = app.surface.runtime.settings_mut();
            if let Some(mut p) = s.active_profile().cloned() {
                p.name = if name.is_empty() { format!("{} copy", p.name) } else { name };
                p.description.clear();
                s.profile = p.name.clone();
                s.profiles.push(p);
                app.surface.editor.new_name.clear();
                app.surface.changed();
            }
        }
        if button(ui, "surfaceRename", "Rename") {
            let name = app.surface.editor.new_name.trim().to_string();
            if !name.is_empty() && !names.contains(&name) {
                let s = app.surface.runtime.settings_mut();
                let cur = s.profile.clone();
                if let Some(p) = s.profiles.iter_mut().find(|p| p.name == cur) {
                    p.name = name.clone();
                }
                s.profile = name;
                app.surface.editor.new_name.clear();
                app.surface.changed();
            }
        }
        if names.len() > 1 && button(ui, "surfaceDeleteProfile", "Delete") {
            let s = app.surface.runtime.settings_mut();
            let cur = s.profile.clone();
            s.profiles.retain(|p| p.name != cur);
            app.surface.changed();
        }
        if button(ui, "surfaceRestore", "Add the built-in profiles") {
            let s = app.surface.runtime.settings_mut();
            for p in ls::defaults::profiles() {
                if !s.profiles.iter().any(|q| q.name == p.name) {
                    s.profiles.push(p);
                }
            }
            app.surface.changed();
        }
    });
    ui.horizontal_wrapped(|ui| {
        if let Some(path) = crate::surface::tactilebridge_settings_path()
            && button(ui, "surfaceImportTb", "Bring over my TactileBridge setup")
        {
            let r = app_run_import(app, &path.to_string_lossy());
            report(app, r);
        }
        if app.services.pick_surface_file.is_some() && button(ui, "surfaceImport", "Import…") {
            let files = app.services.pick_surface_file.as_mut().map(|f| f()).unwrap_or_default();
            for f in files {
                let r = app_run_import(app, &f);
                report(app, r);
            }
        }
        if app.services.save_surface_file.is_some() && button(ui, "surfaceExport", "Export…") {
            let name = format!("{}.lcsurface", app.surface.settings().profile);
            if let Some(path) = app.services.save_surface_file.as_mut().and_then(|f| f(&name)) {
                let r = app.run("surface.export", json!({"path": path}));
                app.surface.editor.message = Some(match r {
                    Ok(_) => crate::i18n::tr_format!("Saved {path}", path = path),
                    Err(e) => e,
                });
            }
        }
    });
    hint(
        ui,
        t,
        "Import takes a TactileBridge settings.json (dial feel, readout, colours, tool layouts), a Logi profile (.lp5) or a LightCraft layout file (.lcsurface).",
    );
    if let Some(m) = app.surface.editor.message.clone() {
        ui.label(RichText::new(m).color(t.text_label));
    }
    if let Some(r) = app.surface.last_report.clone() {
        egui::CollapsingHeader::new(crate::i18n::tr_format!(
            "Last import: {mapped} brought over, {dropped} left out",
            mapped = r.mapped.len(),
            dropped = r.dropped.len()
        ))
        .id_salt("surfaceReport")
        .show(ui, |ui| {
            for l in r.dropped.iter().take(200) {
                ui.label(RichText::new(format!("✕ {} — {}", l.from, l.note)).size(11.0).color(t.text_dim));
            }
            for l in r.mapped.iter().take(400) {
                let note = if l.note.is_empty() { String::new() } else { format!(" ({})", l.note) };
                ui.label(RichText::new(format!("✓ {} → {}{note}", l.from, l.to)).size(11.0).color(t.text_dim));
            }
        });
    }
}

fn app_run_import(app: &mut LightcraftApp, path: &str) -> Result<Value, String> {
    app.run("surface.import", json!({"path": path}))
}

fn report(app: &mut LightcraftApp, r: Result<Value, String>) {
    app.surface.editor.page = None;
    app.surface.editor.control = None;
    app.surface.editor.message = Some(match r {
        Ok(v) => crate::i18n::tr_format!("Imported into the profile “{name}”", name = v["profile"].as_str().unwrap_or_default().to_string()),
        Err(e) => e,
    });
}

// ------------------------------------------------------------------------------------- feel

fn feel(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Dial Feel");
    let mut m = app.surface.settings().motion.clone();
    let mut changed = false;
    row(ui, t, "Speed", |ui| changed |= slider(ui, "surface.speed", &mut m.speed, 0.1..=5.0, 2, "×"));
    row(ui, t, "Acceleration", |ui| changed |= slider(ui, "surface.acceleration", &mut m.acceleration, 0.1..=4.0, 2, "×"));
    row(ui, t, "Fine steps", |ui| changed |= slider(ui, "surface.fine", &mut m.fine, 0.01..=1.0, 2, "×"));
    row(ui, t, "Big steps", |ui| changed |= slider(ui, "surface.coarse", &mut m.coarse, 1.0..=20.0, 1, "×"));
    changed |= check(ui, "surface.invert", &mut m.invert, "Turn every dial the other way");
    if changed {
        app.surface.runtime.settings_mut().motion = m;
        app.surface.changed();
    }
    heading(ui, t, "One Value");
    let target = app.surface.editor.tuning_target.clone();
    let label = ls::targets::target_spec(&target).map_or(target.clone(), |s| tr(s.label).to_string());
    row(ui, t, "Value", |ui| {
        let r = egui::ComboBox::from_id_salt("surfaceTuningTarget").selected_text(label).show_ui(ui, |ui| {
            for s in ls::targets::all_targets() {
                if ui.selectable_label(s.id == target, format!("{} — {}", tr(s.group), tr(s.label))).clicked() {
                    app.surface.editor.tuning_target = s.id.clone();
                }
            }
        });
        register(ui.ctx(), "combo:surfaceTuningTarget", r.response.rect);
    });
    let mut tu = app.surface.settings().tuning_of(&target);
    let mut c = false;
    row(ui, t, "Its speed", |ui| c |= slider(ui, "surface.tuneSpeed", &mut tu.speed, 0.1..=5.0, 2, "×"));
    row(ui, t, "Its acceleration", |ui| c |= slider(ui, "surface.tuneAccel", &mut tu.acceleration, 0.1..=4.0, 2, "×"));
    row(ui, t, "Slow turning", |ui| c |= slider(ui, "surface.tuneMin", &mut tu.acceleration_min, 0.1..=10.0, 1, "×"));
    row(ui, t, "Fast turning", |ui| c |= slider(ui, "surface.tuneMax", &mut tu.acceleration_max, 0.1..=15.0, 1, "×"));
    row(ui, t, "Fast below", |ui| c |= slider_u32(ui, "surface.tuneFast", &mut tu.fast_ms, 5..=250, " ms"));
    row(ui, t, "Slow above", |ui| c |= slider_u32(ui, "surface.tuneSlow", &mut tu.slow_ms, 20..=1200, " ms"));
    c |= check(ui, "surface.tuneInvert", &mut tu.invert, "Turn this value the other way");
    if c {
        app.surface.runtime.settings_mut().tuning.insert(target.clone(), tu.sanitized());
        app.surface.changed();
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Presets for every value:")).color(t.text_label));
        for (name, desc, tuning) in ls::settings::tuning_presets() {
            let r = ui.button(tr(name)).on_hover_text(tr(desc));
            register(ui.ctx(), format!("button:surfacePreset-{name}"), r.rect);
            if r.clicked() {
                let s = app.surface.runtime.settings_mut();
                for spec in ls::targets::all_targets() {
                    s.tuning.insert(spec.id, tuning.clone());
                }
                app.surface.changed();
            }
        }
        if button(ui, "surfaceTuningReset", "Reset all") {
            app.surface.runtime.settings_mut().tuning.clear();
            app.surface.changed();
        }
    });
    hint(ui, t, "A preset overwrites the tuning of every value.");
}

// ------------------------------------------------------------------------------------- readout

fn timing_and_readout(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut s = app.surface.settings().clone();
    let before = s.clone();
    heading(ui, t, "Editing");
    check(ui, "surface.applyToSelected", &mut s.apply_to_selected, "Dials change every selected photo (one undo step per turn)");
    row(ui, t, "Turn ends after", |ui| {
        slider_u32(ui, "surface.gestureEnd", &mut s.transport.gesture_end_ms, 200..=10_000, " ms");
    });
    row(ui, t, "Apply movement every", |ui| {
        slider_u32(ui, "surface.flush", &mut s.transport.flush_ms, 8..=100, " ms");
    });
    check(ui, "surface.echo", &mut s.transport.echo_suppression, "Keep the surface's readout steady while turning");
    heading(ui, t, "On-Screen Readout");
    check(ui, "surface.osd", &mut s.osd.enabled, "Show the value or command in the window");
    check(ui, "surface.osdCommands", &mut s.osd.show_commands, "Also for key presses");
    row(ui, t, "Opacity", |ui| {
        slider(ui, "surface.osdOpacity", &mut s.osd.opacity, 0.1..=1.0, 2, "");
    });
    row(ui, t, "Shown for", |ui| {
        slider_u32(ui, "surface.osdTime", &mut s.osd.display_ms, 500..=10_000, " ms");
    });
    row(ui, t, "Position", |ui| {
        choices(
            ui,
            "surfaceOsdPosition",
            &[
                (OsdPosition::TopEdge, "Top edge"),
                (OsdPosition::Top, "Top"),
                (OsdPosition::Bottom, "Bottom"),
                (OsdPosition::BottomEdge, "Bottom edge"),
            ],
            &mut s.osd.position,
        );
    });
    row(ui, t, "Background", |ui| {
        crate::text_field::TextField::singleline("field:surfaceOsdColor", &mut s.osd.background).width(90.0).show(ui);
    });
    heading(ui, t, "Keys");
    check(ui, "surface.showValues", &mut s.look.show_values, "Show values on the keys and the dial display");
    check(ui, "surface.haptics", &mut s.look.haptics, "Haptic feedback (where the device has it)");
    row(ui, t, "Key background", |ui| {
        crate::text_field::TextField::singleline("field:surfaceBackground", &mut s.look.background).width(90.0).show(ui);
    });
    row(ui, t, "Key text", |ui| {
        crate::text_field::TextField::singleline("field:surfaceForeground", &mut s.look.foreground).width(90.0).show(ui);
    });
    hint(ui, t, "Colours are #RRGGBB.");
    if s != before {
        // colours apply once they are whole (#RRGGBB)
        let colours_ok = [&s.osd.background, &s.look.background, &s.look.foreground].iter().all(|c| ls::settings::rgb(c).is_some());
        let only_colours_changed = {
            let mut a = s.clone();
            a.osd.background = before.osd.background.clone();
            a.look.background = before.look.background.clone();
            a.look.foreground = before.look.foreground.clone();
            a == before
        };
        if colours_ok || !only_colours_changed {
            if !colours_ok {
                s.osd.background = before.osd.background;
                s.look.background = before.look.background;
                s.look.foreground = before.look.foreground;
            }
            *app.surface.runtime.settings_mut() = s;
            app.surface.changed();
        } else {
            // keep the half-typed colour in the field without applying it
            let rt = app.surface.runtime.settings_mut();
            rt.osd.background = s.osd.background;
            rt.look.background = s.look.background;
            rt.look.foreground = s.look.foreground;
        }
    }
}
