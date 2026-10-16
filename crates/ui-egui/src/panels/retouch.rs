//! Portrait retouching (beyond Lightroom): the Edit panel's **Retouch** section, one sub-section per
//! part of a person (Skin, Blemishes, …), each with its own eye for a before/after; and Find
//! Blemishes in the Remove panel too. They run `skin.*`, `retouch.*` and `spot.findBlemishes`
//! commands; the open MediaPipe models they need are offered for download when missing.

use egui::{Align2, RichText, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, SkinArea, SkinSmoothing};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::i18n::tr;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{register, segmented, text_button};

/// The Retouch section's body: its sub-sections, folded or open.
pub fn panel(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings, id: PhotoId) {
    if sub_header(app, ui, d, "skin", "Skin", d.skin.active(), true) {
        smooth_skin(app, ui, d, id);
    }
    if sub_header(app, ui, d, "blemishes", "Blemishes", false, false) {
        padded(ui, |ui| {
            note(ui, "Heals small dark or red spots on the skin of the faces (and, if you like, of the body).");
            ui.add_space(4.0);
            find_blemishes(app, ui);
        });
        ui.add_space(4.0);
    }
    for (key, title, intro, missing) in PARTS {
        if sub_header(app, ui, d, key, title, d.retouch.section_active(key), true) {
            part_body(app, ui, d, id, key, intro, missing);
        }
    }
    ui.add_space(4.0);
}

/// The sections beyond Skin and Blemishes: (key, title, what it does, what to say when its part
/// isn't in the photo).
const PARTS: &[(&str, &str, &str, &str)] =
    &[("teeth", "Teeth", "Whiter, brighter teeth: the yellow is taken out gently.", "No teeth were found in this photo.")];

/// Where a section stands on this photo, in words (`None`: its parts were found, or it is off).
fn part_status(app: &LightcraftApp, d: &DevelopSettings, id: PhotoId, key: &str, missing: &str) -> Option<String> {
    if !d.retouch.section_active(key) {
        return None;
    }
    let seg = &app.session.segmenter;
    let a = d.retouch_analysis.as_ref().filter(|a| a.orientation == d.orientation);
    let parts = d.retouch.section_parts(key);
    let found: Vec<Option<bool>> = parts.iter().map(|r| a.and_then(|a| a.mask(*r)).map(|m| m.seg.is_some())).collect();
    if found.contains(&Some(true)) {
        return None;
    }
    if found.iter().all(Option::is_some) {
        let person = parts.first().map_or(0, |r| r.person);
        return Some(if person > 0 { tr("This person wasn't found in this photo.").to_string() } else { tr(missing).to_string() });
    }
    Some(if seg.retouch_pending(id) {
        tr("Finding faces…").to_string()
    } else if !(seg.model_installed("people") && seg.model_installed("face")) {
        tr("Needs the MediaPipe people and face models (one-time downloads, Apache-2.0).").to_string()
    } else {
        tr("The faces haven't been looked at yet.").to_string()
    })
}

/// A section's body: what it does or where it stands, its sliders, the person, Turn Off.
fn part_body(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings, id: PhotoId, key: &str, intro: &str, missing: &str) {
    let status = part_status(app, d, id, key, missing);
    padded(ui, |ui| match &status {
        Some(s) => {
            note(ui, s);
            if !app.session.segmenter.retouch_pending(id) {
                let model = ["people", "face"].into_iter().find(|m| !app.session.segmenter.model_installed(m));
                let (label, wid) = match model {
                    Some(_) => ("Download…", format!("retouchDownload-{key}")),
                    None => ("Find Again", format!("retouchFind-{key}")),
                };
                if text_button(ui, &wid, label, false).clicked() {
                    match model {
                        Some(m) => app.offer_ai_model(m, None),
                        None => {
                            let r = app.run("retouch.find", json!({}));
                            report(app, ui.ctx(), r);
                        }
                    }
                }
            }
        }
        None => note(ui, intro),
    });
    if app.session.segmenter.retouch_pending(id) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    for (sec, _, control) in lightcraft_develop::retouch::SLIDERS {
        if *sec != key {
            continue;
        }
        let before = d.retouch.section_active(key);
        super::edit::control(app, ui, d, control, true);
        // the first move of a slider needs the models: offer them when they are missing
        let now = app.session.develop_of(id).is_some_and(|n| n.retouch.section_active(key));
        if !before && now && lightcraft_engine::segment::Segmenter::AVAILABLE {
            let missing_model = ["people", "face"].into_iter().find(|m| !app.session.segmenter.model_installed(m));
            if let Some(m) = missing_model {
                app.offer_ai_model(m, None);
            }
        }
    }
    super::edit::sub_title(ui, "Person");
    let people = [("All", "0"), ("1", "1"), ("2", "2"), ("3", "3"), ("4", "4")];
    let person = lightcraft_develop::controls::get(d, &lightcraft_develop::retouch::person_control(key)).unwrap_or(0.0).clamp(0.0, 4.0) as usize;
    let mut chosen = None;
    padded(ui, |ui| chosen = segmented(ui, &format!("retouchPerson-{key}"), &people, Some(person), 5));
    if let Some(i) = chosen {
        let r = app.run("retouch.set", json!({"section": key, "person": i}));
        report(app, ui.ctx(), r);
    }
    padded(ui, |ui| {
        note(ui, "People are counted from the left, by their face.");
        if d.retouch.section_active(key) {
            ui.add_space(4.0);
            let r = ui.button(tr("Turn Off"));
            register(ui.ctx(), format!("button:retouchOff-{key}"), r.rect);
            if r.clicked() {
                let r = app.run("retouch.off", json!({"section": key}));
                report(app, ui.ctx(), r);
            }
        }
    });
    ui.add_space(6.0);
}

/// A sub-section's header: a chevron, its title, a dot while it changes the photo, and (`eye`) its
/// eye, which switches it off for a before/after. Returns whether it is open.
fn sub_header(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings, key: &str, title: &str, active: bool, eye: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let flyout = format!("retouch.{key}");
    let open = app.ui.flyout_open(&flyout);
    let title = tr(title);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title.to_string()));
    register(ui.ctx(), format!("retouch:{key}"), r);
    let p = ui.painter();
    if ui.rect_contains_pointer(r) {
        p.rect_filled(r, 0.0, t.chrome.gamma_multiply(1.06));
    }
    let chev = egui::Rect::from_center_size(pos2(r.left() + 30.0, r.center().y), vec2(12.0, 12.0));
    paint(p, chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_label);
    let galley = p.text(pos2(r.left() + 42.0, r.center().y), Align2::LEFT_CENTER, title, t.semibold(13.0), t.text);
    let section = section_id(key);
    let on = d.section_enabled(&section);
    if active {
        // a dot: this part is being retouched
        p.circle_filled(pos2(galley.right() + 8.0, r.center().y), 3.0, if on { t.accent } else { t.text_disabled });
    }
    if eye && (active || !on) {
        let er = egui::Rect::from_center_size(pos2(r.right() - 34.0, r.center().y), vec2(16.0, 16.0));
        let e = ui.interact(er, ui.id().with(("retouchEye", key)), Sense::click());
        crate::widgets::announce_selected(&e, tr("Show / hide"), on);
        register(ui.ctx(), format!("retouchEye:{key}"), er);
        paint(ui.painter(), er, Icon::Eye, if on { t.icon } else { t.text_disabled });
        if !on {
            ui.painter().line_segment([er.left_bottom(), er.right_top()], Stroke::new(1.5, t.text_disabled));
        }
        if e.clicked() {
            let _ = app.run("develop.sectionEnabled", json!({"section": section, "enabled": !on}));
            return open;
        }
    }
    if resp.clicked() {
        app.ui.toggle_flyout(&flyout);
    }
    open
}

/// The develop section (its eye) of a Retouch sub-section: `skin` for Smooth Skin, `retouch.<key>`
/// for the others.
pub fn section_id(key: &str) -> String {
    if key == "skin" { key.to_string() } else { format!("retouch.{key}") }
}

fn padded(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 4, bottom: 4 }).show(ui, add);
}

fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(RichText::new(tr(text)).size(11.0).color(t.text_dim)).wrap());
}

/// A command's error, or the `error` its reply carries: when it is a missing open model, the dialog
/// that offers its download; otherwise a toast.
fn report(app: &mut LightcraftApp, ctx: &egui::Context, r: Result<Value, String>) {
    let e = match r {
        Ok(v) => match v["error"].as_str() {
            Some(e) => e.to_string(),
            None => return,
        },
        Err(e) => e,
    };
    match lightcraft_engine::segment::missing_model_in(&e) {
        Some(key) => app.offer_ai_model(key, None),
        None => app.toast_error(ctx, e),
    }
}

/// Where Smooth Skin stands on this photo, in words (`None`: it is working on the skin found).
fn status(app: &LightcraftApp, d: &DevelopSettings, id: PhotoId) -> Option<String> {
    let found = d.skin_analysis.as_ref().filter(|a| a.is_for(&d.skin, d.orientation));
    let seg = &app.session.segmenter;
    match found {
        Some(a) if a.seg.logits().is_some_and(|l| l.iter().any(|v| *v > 0.0)) => None,
        Some(_) if d.skin.person > 0 => Some(tr("This person wasn't found in this photo.").to_string()),
        Some(_) => Some(tr("No skin was found in this photo.").to_string()),
        None if seg.skin_pending(id) => Some(tr("Finding the skin…").to_string()),
        None if !(seg.model_installed("people") && seg.model_installed("face")) => {
            Some(tr("Needs the MediaPipe people and face models (one-time downloads, Apache-2.0).").to_string())
        }
        None => Some(tr("The skin hasn't been found yet.").to_string()),
    }
}

/// The Skin sub-section's body (Smooth Skin).
pub fn smooth_skin(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings, id: PhotoId) {
    let k = d.skin;
    if !k.active() {
        padded(ui, |ui| {
            note(ui, "Evens out skin tone and texture while keeping pores: the skin and faces are found by the open MediaPipe models.");
            ui.add_space(4.0);
            if text_button(ui, "skinSmooth", "Smooth Skin", false).on_hover_text(tr("Switch Smooth Skin on with subtle settings")).clicked() {
                let r = app.run("skin.smooth", json!({}));
                report(app, ui.ctx(), r);
            }
        });
        return;
    }
    if let Some(s) = status(app, d, id) {
        padded(ui, |ui| {
            note(ui, &s);
            if !app.session.segmenter.skin_pending(id) {
                let missing = ["people", "face"].into_iter().find(|m| !app.session.segmenter.model_installed(m));
                let (label, wid) = match missing {
                    Some(_) => ("Download…", "skinDownload"),
                    None => ("Find Skin", "skinFind"),
                };
                if text_button(ui, wid, label, false).clicked() {
                    match missing {
                        Some(m) => app.offer_ai_model(m, None),
                        None => {
                            let r = app.run("skin.find", json!({}));
                            report(app, ui.ctx(), r);
                        }
                    }
                }
            }
        });
    } else if app.session.segmenter.skin_pending(id) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    for c in ["skin.amount", "skin.texture", "skin.evenTone", "skin.redness", "skin.shine"] {
        super::edit::control(app, ui, d, c, true);
    }
    super::edit::sub_title(ui, "Face or Body");
    let areas = [(SkinArea::Face, "Face", "face"), (SkinArea::Body, "Body", "body"), (SkinArea::Both, "Both", "both")];
    let items: Vec<(&str, &str)> = areas.iter().map(|(_, l, k)| (*l, *k)).collect();
    let mut chosen = None;
    padded(ui, |ui| chosen = segmented(ui, "skinArea", &items, areas.iter().position(|(a, _, _)| *a == k.area), 3));
    if let Some(i) = chosen
        && let Some((_, _, name)) = areas.get(i)
    {
        let r = app.run("skin.set", json!({"area": name}));
        report(app, ui.ctx(), r);
    }
    super::edit::sub_title(ui, "Person");
    let people = [("All", "0"), ("1", "1"), ("2", "2"), ("3", "3"), ("4", "4")];
    let mut chosen = None;
    padded(ui, |ui| chosen = segmented(ui, "skinPerson", &people, Some(k.person.min(SkinSmoothing::MAX_PERSON) as usize), 5));
    if let Some(i) = chosen {
        let r = app.run("skin.set", json!({"person": i}));
        report(app, ui.ctx(), r);
    }
    padded(ui, |ui| {
        note(ui, "People are counted from the left, by their face.");
        ui.add_space(4.0);
        let r = ui.button(tr("Turn Off"));
        register(ui.ctx(), "button:skinOff", r.rect);
        if r.clicked() {
            let r = app.run("skin.smooth", json!({"off": true}));
            report(app, ui.ctx(), r);
        }
    });
    ui.add_space(6.0);
}

/// Find Blemishes in the Remove panel: the button and its "Include body skin" option.
pub fn find_blemishes(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if text_button(ui, "findBlemishes", "Find Blemishes", false)
            .on_hover_text(tr("Heal the small dark or red spots on the faces' skin; eyes, brows, nostrils and lips are left alone"))
            .clicked()
        {
            let body = app.ui.blemish_body;
            match app.run("spot.findBlemishes", json!({"body": body})) {
                Ok(v) if v["queued"] == true => app.toast(ui.ctx(), tr("Finding blemishes…")),
                Ok(v) => {
                    let n = v["added"].as_u64().unwrap_or(0);
                    let text = match n {
                        0 => tr("No blemishes found").to_string(),
                        1 => tr("Healed 1 blemish").to_string(),
                        n => crate::i18n::tr_format!("Healed {n} blemishes", n = n),
                    };
                    app.toast(ui.ctx(), text);
                }
                Err(e) => report(app, ui.ctx(), Err(e)),
            }
        }
    });
    let mut body = app.ui.blemish_body;
    let r = ui.checkbox(&mut body, tr("Include body skin"));
    register(ui.ctx(), "check:blemishBody", r.rect);
    r.clone().on_hover_text(tr("Also look on shoulders, arms and back"));
    if r.changed() {
        app.ui.blemish_body = body;
    }
}
