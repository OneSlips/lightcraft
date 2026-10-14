//! Settings ▸ Controls: control surfaces — pairing, profiles, the layout editor (pages, what each
//! key, button and dial does, learn mode), pages that follow the app, dial feel, timing, the
//! on-screen readout and the surface's look. Everything is saved to `control-surfaces.json`
//! (`crate::surface`).
//!
//! Portions © RAD Films - Roman Alurkoff (from TactileBridge), MIT: the settings design
//! (TactileBridge's SettingsForm: global and per-value dial feel, workflow presets, bridge
//! timing, OSD, plugin colours, tool contexts, Auto Sync, tool switching).

use egui::RichText;
use lightcraft_surface as ls;
use ls::settings::OsdPosition;
use ls::{Binding, Modifier, PageGroup};
use serde_json::{Value, json};

use super::settings::{check, choices, heading, hint, row};
use crate::LightcraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::register;

/// What kind of binding the editor shows for a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    None,
    Value,
    Command,
    Fine,
    Coarse,
    PageNext,
    PagePrevious,
    Reset,
    ApplyToSelected,
}

impl Kind {
    const ALL: [(Kind, &'static str); 9] = [
        (Kind::None, "Nothing"),
        (Kind::Value, "Value (dial)"),
        (Kind::Command, "Command"),
        (Kind::Fine, "Hold for fine steps"),
        (Kind::Coarse, "Hold for big steps"),
        (Kind::PageNext, "Next page"),
        (Kind::PagePrevious, "Previous page"),
        (Kind::Reset, "Reset the dial's value"),
        (Kind::ApplyToSelected, "Apply to all selected (toggle)"),
    ];

    fn of(b: Option<&Binding>) -> Kind {
        match b {
            None => Kind::None,
            Some(Binding::Value { .. }) => Kind::Value,
            Some(Binding::Command { .. }) => Kind::Command,
            Some(Binding::Modifier { modifier: Modifier::Fine }) => Kind::Fine,
            Some(Binding::Modifier { modifier: Modifier::Coarse }) => Kind::Coarse,
            Some(Binding::PageNext { .. }) => Kind::PageNext,
            Some(Binding::PagePrevious { .. }) => Kind::PagePrevious,
            Some(Binding::Reset) => Kind::Reset,
            Some(Binding::ApplyToSelected) => Kind::ApplyToSelected,
            Some(_) => Kind::Command,
        }
    }

    fn binding(self, control: &str) -> Option<Binding> {
        let dials = ls::devices::group_of(control) == PageGroup::Dials;
        Some(match self {
            Kind::None => return None,
            Kind::Value => Binding::value("light.exposure"),
            Kind::Command => Binding::command("library.next"),
            Kind::Fine => Binding::Modifier { modifier: Modifier::Fine },
            Kind::Coarse => Binding::Modifier { modifier: Modifier::Coarse },
            Kind::PageNext => Binding::PageNext { group: Some(if dials { PageGroup::Dials } else { PageGroup::Keys }) },
            Kind::PagePrevious => Binding::PagePrevious { group: Some(if dials { PageGroup::Dials } else { PageGroup::Keys }) },
            Kind::Reset => Binding::Reset,
            Kind::ApplyToSelected => Binding::ApplyToSelected,
        })
    }
}

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

pub fn settings_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    if let Some(c) = app.surface.runtime.take_learned() {
        app.surface.editor.control = Some(c);
    }
    let max_h = (ui.ctx().content_rect().height() * 0.7).max(240.0);
    egui::ScrollArea::vertical().id_salt("surfaceSettings").max_height(max_h).show(ui, |ui| {
        connection(app, ui, t);
        profiles(app, ui, t);
        layout(app, ui, t);
        contexts(app, ui, t);
        feel(app, ui, t);
        timing_and_readout(app, ui, t);
    });
}

// ------------------------------------------------------------------------------------- pairing

fn connection(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Control Surfaces");
    let mut on = app.surface.settings().enabled;
    if check(ui, "surface.enabled", &mut on, "Let control surfaces drive LightCraft (Logitech MX Creative Console and others)") {
        app.surface.runtime.settings_mut().enabled = on;
        app.surface.changed();
    }
    hint(
        ui,
        t,
        "Surfaces connect through their plugin (for the MX Creative Console: the LightCraft plugin for Logi Options+). A new surface asks to pair: check that it shows the same code, then approve it here.",
    );
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
                ui.label(RichText::new(e.clone()).size(11.0).color(egui::Color32::from_rgb(232, 168, 56)));
            }
            _ => {}
        }
    });
    if let Some(e) = app.surface.load_error.clone().or(app.surface.save_error.clone()) {
        ui.label(RichText::new(e).color(egui::Color32::from_rgb(232, 168, 56)));
    }
    let connected = app.surface.connected();
    if connected.is_empty() {
        hint(ui, t, "No surface is connected.");
    } else {
        for (client, device) in connected {
            ui.label(RichText::new(format!("● {client} — {device}")).color(t.text));
        }
    }
    let waiting: Vec<_> = app.surface.pairing.waiting().cloned().collect();
    for w in waiting {
        ui.horizontal(|ui| {
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

// ------------------------------------------------------------------------------------- profiles

fn profiles(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Profile");
    let names: Vec<String> = app.surface.settings().profiles.iter().map(|p| p.name.clone()).collect();
    let current = app.surface.settings().profile.clone();
    row(ui, t, "Profile", |ui| {
        let r = egui::ComboBox::from_id_salt("surfaceProfile").selected_text(&current).show_ui(ui, |ui| {
            for n in &names {
                if ui.selectable_label(*n == current, n).clicked() && *n != current {
                    let _ = app.run("surface.profile", json!({"profile": n}));
                    app.surface.editor.page = None;
                    app.surface.editor.control = None;
                }
            }
        });
        register(ui.ctx(), "combo:surfaceProfile", r.response.rect);
    });
    if let Some(desc) = app.surface.settings().active_profile().map(|p| p.description.clone()).filter(|d| !d.is_empty()) {
        hint(ui, t, &desc);
    }
    ui.horizontal_wrapped(|ui| {
        crate::text_field::TextField::singleline("field:surfaceNewName", &mut app.surface.editor.new_name).hint(tr("Name")).width(160.0).show(ui);
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

// ------------------------------------------------------------------------------------- layout

const BASE: &str = "\u{0}base";

fn layout(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Layout");
    let Some(profile) = app.surface.settings().active_profile().cloned() else { return };
    let page_key = app.surface.editor.page.clone().unwrap_or_else(|| profile.pages.first().map(|p| p.name.clone()).unwrap_or(BASE.into()));
    let page_label = |k: &str| if k == BASE { tr("Every page (base)").to_string() } else { k.to_string() };
    row(ui, t, "Page", |ui| {
        let r = egui::ComboBox::from_id_salt("surfacePage").selected_text(page_label(&page_key)).show_ui(ui, |ui| {
            if ui.selectable_label(page_key == BASE, page_label(BASE)).clicked() {
                app.surface.editor.page = Some(BASE.into());
            }
            for p in &profile.pages {
                let auto = if p.auto { format!("  · {}", tr("tool page")) } else { String::new() };
                if ui.selectable_label(page_key == p.name, format!("{}{auto}", p.name)).clicked() {
                    app.surface.editor.page = Some(p.name.clone());
                }
            }
        });
        register(ui.ctx(), "combo:surfacePage", r.response.rect);
    });
    let page = profile.page(&page_key).cloned();
    ui.horizontal_wrapped(|ui| {
        if let Some(page) = &page {
            let mut keys = page.in_group(PageGroup::Keys);
            let mut dials = page.in_group(PageGroup::Dials);
            let mut auto = page.auto;
            let a = check(ui, "surface.pageKeys", &mut keys, "Keys");
            let b = check(ui, "surface.pageDials", &mut dials, "Dials");
            let c = check(ui, "surface.pageAuto", &mut auto, "Tool page (shown by its context only)");
            if a || b || c {
                edit_page(app, &page.name, |p| {
                    p.groups = match (keys, dials) {
                        (true, false) => vec![PageGroup::Keys],
                        (false, true) => vec![PageGroup::Dials],
                        _ => Vec::new(),
                    };
                    p.auto = auto;
                });
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if button(ui, "surfaceAddPage", "Add page") {
            let name = app.surface.editor.new_name.trim().to_string();
            let name = if name.is_empty() { format!("Page {}", profile.pages.len() + 1) } else { name };
            edit_profile(app, |p| p.pages.push(ls::Page { name: name.clone(), groups: vec![PageGroup::Keys], ..Default::default() }));
            app.surface.editor.page = Some(name);
            app.surface.editor.new_name.clear();
        }
        if page.is_some() && button(ui, "surfaceDeletePage", "Delete page") {
            let name = page_key.clone();
            edit_profile(app, |p| p.pages.retain(|q| q.name != name));
            app.surface.editor.page = None;
        }
        if page.is_some() && button(ui, "surfaceRenamePage", "Rename page") {
            let new = app.surface.editor.new_name.trim().to_string();
            let old = page_key.clone();
            if !new.is_empty() && profile.page(&new).is_none() {
                edit_profile(app, |p| {
                    if let Some(q) = p.pages.iter_mut().find(|q| q.name == old) {
                        q.name = new.clone();
                    }
                    for list in p.contexts.values_mut() {
                        for n in list.iter_mut().filter(|n| **n == old) {
                            *n = new.clone();
                        }
                    }
                });
                app.surface.editor.page = Some(new);
                app.surface.editor.new_name.clear();
            }
        }
    });
    ui.horizontal(|ui| {
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
    // the device: 3 × 3 keys, then the dialpad
    let bindings = |c: &str| -> Option<Binding> {
        if page_key == BASE { profile.base.get(c).cloned() } else { page.as_ref().and_then(|p| p.bindings.get(c).cloned()) }
    };
    let selected = app.surface.editor.control.clone();
    let mut clicked = None;
    let cell = |ui: &mut egui::Ui, control: &str, clicked: &mut Option<String>| {
        let b = bindings(control);
        let text = b.as_ref().map_or_else(|| "—".to_string(), |b| short_label(app, b));
        let r = ui.add_sized([96.0, 44.0], egui::Button::new(RichText::new(text).size(11.0)).selected(selected.as_deref() == Some(control)));
        register(ui.ctx(), format!("button:surfaceControl-{control}"), r.rect);
        if r.clicked() {
            *clicked = Some(control.to_string());
        }
        r.on_hover_text(control);
    };
    for row_keys in [["key.1", "key.2", "key.3"], ["key.4", "key.5", "key.6"], ["key.7", "key.8", "key.9"]] {
        ui.horizontal(|ui| {
            for k in row_keys {
                cell(ui, k, &mut clicked);
            }
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for c in ["dial", "roller"] {
            cell(ui, c, &mut clicked);
        }
    });
    ui.horizontal(|ui| {
        for c in ["pad.1", "pad.2", "pad.3", "pad.4"] {
            cell(ui, c, &mut clicked);
        }
    });
    if let Some(c) = clicked {
        app.surface.editor.control = Some(c);
        app.surface.editor.params_text.clear();
    }
    if let Some(control) = app.surface.editor.control.clone() {
        binding_editor(app, ui, t, &page_key, &control, bindings(&control));
    }
}

fn short_label(_app: &LightcraftApp, b: &Binding) -> String {
    match b {
        Binding::Command { command, params } => {
            let s = ls::icons::command_label(command, params);
            tr(&s).to_string()
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

fn binding_editor(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, page: &str, control: &str, current: Option<Binding>) {
    let label = ls::devices::find_control(control).map_or(control.to_string(), |c| tr(c.label).to_string());
    ui.label(RichText::new(label).font(t.semibold(12.5)).color(t.text));
    let kind = Kind::of(current.as_ref());
    let mut new_kind = kind;
    row(ui, t, "Does", |ui| {
        let r = egui::ComboBox::from_id_salt("surfaceBindingKind")
            .selected_text(tr(Kind::ALL.iter().find(|k| k.0 == kind).map_or("", |k| k.1)))
            .show_ui(ui, |ui| {
                for (k, l) in Kind::ALL {
                    ui.selectable_value(&mut new_kind, k, tr(l));
                }
            });
        register(ui.ctx(), "combo:surfaceBindingKind", r.response.rect);
    });
    if new_kind != kind {
        set_binding(app, page, control, new_kind.binding(control));
        app.surface.editor.params_text.clear();
        return;
    }
    let filter_box = |app: &mut LightcraftApp, ui: &mut egui::Ui| {
        crate::text_field::TextField::singleline("field:surfaceFilter", &mut app.surface.editor.filter).hint(tr("Search")).width(220.0).show(ui);
    };
    match current {
        Some(Binding::Value { target }) => {
            let spec = ls::targets::target_spec(&target);
            ui.label(
                RichText::new(crate::i18n::tr_format!(
                    "Moves: {name}",
                    name = spec.as_ref().map_or(target.clone(), |s| format!("{} ({})", tr(s.label), tr(s.group)))
                ))
                .color(t.text_label),
            );
            filter_box(app, ui);
            let q = app.surface.editor.filter.to_lowercase();
            egui::ScrollArea::vertical().id_salt("surfaceTargets").max_height(160.0).show(ui, |ui| {
                for s in ls::targets::all_targets() {
                    let text = format!("{} — {}", tr(s.group), tr(s.label));
                    if !q.is_empty() && !text.to_lowercase().contains(&q) && !s.id.to_lowercase().contains(&q) {
                        continue;
                    }
                    let r = ui.selectable_label(s.id == target, text);
                    register(ui.ctx(), format!("item:surfaceTarget-{}", s.id), r.rect);
                    if r.clicked() {
                        set_binding(app, page, control, Some(Binding::value(&s.id)));
                    }
                }
            });
            if !ls::devices::is_encoder(control) {
                hint(ui, t, "Press this key to put the value on the dial; press it again to give the dial back.");
            }
        }
        Some(Binding::Command { command, params }) => {
            ui.label(RichText::new(crate::i18n::tr_format!("Runs: {name}", name = command.clone())).color(t.text_label));
            filter_box(app, ui);
            let q = app.surface.editor.filter.to_lowercase();
            let mut all: Vec<(String, String)> = crate::menus::ui_commands().map(|c| (c.0.to_string(), tr(c.1).to_string())).collect();
            all.extend(app.session.commands().into_iter().map(|c| (c.id.to_string(), tr(c.label).to_string())));
            all.sort_by(|a, b| a.1.cmp(&b.1));
            all.dedup_by(|a, b| a.0 == b.0);
            egui::ScrollArea::vertical().id_salt("surfaceCommands").max_height(160.0).show(ui, |ui| {
                for (id, label) in all {
                    if !q.is_empty() && !label.to_lowercase().contains(&q) && !id.to_lowercase().contains(&q) {
                        continue;
                    }
                    let r = ui.selectable_label(id == command, format!("{label} ({id})"));
                    register(ui.ctx(), format!("item:surfaceCommand-{id}"), r.rect);
                    if r.clicked() {
                        set_binding(app, page, control, Some(Binding::command(&id)));
                        app.surface.editor.params_text.clear();
                    }
                }
            });
            if app.surface.editor.params_text.is_empty() {
                app.surface.editor.params_text = if params.is_null() { "{}".into() } else { params.to_string() };
            }
            row(ui, t, "Parameters", |ui| {
                let r = crate::text_field::TextField::singleline("field:surfaceParams", &mut app.surface.editor.params_text).width(260.0).show(ui);
                let _ = r;
            });
            match serde_json::from_str::<Value>(&app.surface.editor.params_text) {
                Ok(v) if v.is_object() || v.is_null() => {
                    let v = if v.as_object().is_some_and(|o| o.is_empty()) { Value::Null } else { v };
                    if v != params {
                        set_binding(app, page, control, Some(Binding::command_with(&command, v)));
                    }
                }
                _ => hint(ui, t, "Parameters are a JSON object, e.g. {\"rating\": 5}"),
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------------------------- contexts

fn contexts(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, "Pages That Follow the App");
    let mut follow = app.surface.settings().follow_context;
    if check(ui, "surface.followContext", &mut follow, "Switch pages with what LightCraft shows (Crop, Masking, a dialog…)") {
        app.surface.runtime.settings_mut().follow_context = follow;
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
    // one value's own tuning
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

// ------------------------------------------------------------------------------------- timing

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
        let r = crate::text_field::TextField::singleline("field:surfaceOsdColor", &mut s.osd.background).width(90.0).show(ui);
        let _ = r;
    });
    heading(ui, t, "Surface");
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
