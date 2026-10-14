//! Portrait retouching in the Edit and Remove panels: Smooth Skin (its own section, beyond
//! Lightroom) and Find Blemishes (a button in Remove). Both run `skin.*` / `spot.findBlemishes`
//! commands; the open MediaPipe models they need are offered for download when missing.

use egui::RichText;
use lightcraft_develop::{DevelopSettings, SkinArea, SkinSmoothing};
use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::{register, segmented, text_button};

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

/// The Smooth Skin section's body.
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
    for c in ["skin.amount", "skin.texture", "skin.evenTone", "skin.redness"] {
        super::edit::control(app, ui, d, c, true);
    }
    super::edit::sub_title(ui, "Skin");
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
    let people = [("Everyone", "0"), ("1", "1"), ("2", "2"), ("3", "3"), ("4", "4")];
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
