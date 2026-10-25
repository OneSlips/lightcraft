//! The bottom bar's More filters popover: kind, edited state and camera / lens / keyword pickers
//! (the stars, flags and colour labels are on the bar itself) — every control drives
//! `library.filter`. "Save as Smart Album…" keeps the current view as a live album; "Clear All
//! Filters" resets the filter.

use egui::{Sense, Stroke, StrokeKind, vec2};
use lightcraft_catalog::MediaKind;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// Colour-label swatches (UI colours, our own choice).
pub use crate::theme::label_color;

fn filter(app: &mut LightcraftApp, patch: Value) {
    let _ = app.run("library.filter", patch);
}

/// A dropdown showing `current`; `items` are (label, filter patch, selected).
fn picker(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, current: &str, active: bool, items: Vec<(String, Value, bool)>) {
    let t = Tokens::get(ui.ctx());
    let r = crate::widgets::dropdown(
        ui,
        &format!("filter-{id}"),
        if active && matches!(id, "camera" | "lens" | "keyword") { current } else { crate::i18n::tr(current) },
        t.font(12.5),
        if active { t.text } else { t.text_label },
    );
    if active {
        ui.painter().rect_stroke(r.rect.expand(2.0), 4.0, Stroke::new(1.0, t.accent.gamma_multiply(0.7)), StrokeKind::Outside);
    }
    let mut chosen = None;
    egui::Popup::menu(&r).show(|ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (label, patch, sel) in &items {
                if ui
                    .selectable_label(
                        *sel,
                        if matches!(id, "camera" | "lens" | "keyword") && patch.get(id).is_some_and(|value| !value.is_null()) {
                            label.as_str()
                        } else {
                            crate::i18n::tr(label)
                        },
                    )
                    .clicked()
                {
                    chosen = Some(patch.clone());
                }
            }
        });
    });
    if let Some(p) = chosen {
        filter(app, p);
    }
}

/// The bottom bar's More filters popover: the rarer filters (type, edited, camera, lens, keyword),
/// Save as Smart Album…, Clear and the filter presets. The stars, flags and colour
/// labels are on the bar itself.
pub fn more_filters(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    register(ui.ctx(), "filterbar", ui.max_rect());
    let f = app.session.filter.clone();
    ui.spacing_mut().item_spacing.y = 6.0;
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 20.0), Sense::hover());
            ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, crate::i18n::tr(label), t.font(12.0), t.text_dim);
            add(ui);
        });
    };
    // kind + edited
    let kind_label = match f.kind {
        None => "All types",
        Some(MediaKind::Image) => "Photos",
        Some(MediaKind::Raw) => "Raw",
        Some(MediaKind::Video) => "Videos",
    };
    let kind_label = match f.merged.as_deref() {
        Some("hdr") => "HDR",
        Some("panorama") => "Panoramas",
        Some("hdrPanorama") => "HDR Panoramas",
        Some(_) => "Merged",
        None => kind_label,
    };
    row(ui, "Type", &mut |ui| {
        picker(
            app,
            ui,
            "kind",
            kind_label,
            f.kind.is_some() || f.merged.is_some(),
            [
                ("All types", Value::Null, None),
                ("Photos", json!("image"), Some(MediaKind::Image)),
                ("Raw", json!("raw"), Some(MediaKind::Raw)),
                ("Videos", json!("video"), Some(MediaKind::Video)),
            ]
            .into_iter()
            .map(|(l, v, k)| (l.to_string(), json!({"kind": v, "merged": null}), f.kind == k && f.merged.is_none()))
            .chain(
                [("HDR", "hdr"), ("Panoramas", "panorama"), ("HDR Panoramas", "hdrPanorama")]
                    .into_iter()
                    .map(|(l, m)| (l.to_string(), json!({"kind": null, "merged": m}), f.merged.as_deref() == Some(m))),
            )
            .collect(),
        )
    });
    let edited_label = match f.edited {
        None => "Edited or not",
        Some(true) => "Edited",
        Some(false) => "Unedited",
    };
    row(ui, "Edits", &mut |ui| {
        picker(
            app,
            ui,
            "edited",
            edited_label,
            f.edited.is_some(),
            [("Edited or not", Value::Null, None), ("Edited", json!(true), Some(true)), ("Unedited", json!(false), Some(false))]
                .into_iter()
                .map(|(l, v, e)| (l.to_string(), json!({"edited": v}), f.edited == e))
                .collect(),
        )
    });
    // metadata pickers
    let values = app.caches.filter_values(&app.session.catalog);
    let (cameras, lenses, keywords) = (values.cameras.clone(), values.lenses.clone(), values.keywords.clone());
    for (id, key, all_label, current, values) in [
        ("camera", "camera", "Camera", f.camera.clone(), cameras),
        ("lens", "lens", "Lens", f.lens.clone(), lenses),
        ("keyword", "keyword", "Keyword", f.keyword.clone(), keywords),
    ] {
        let mut items = vec![(format!("Any {}", all_label.to_lowercase()), json!({key: Value::Null}), current.is_none())];
        items.extend(values.into_iter().map(|v| (v.clone(), json!({key: v.clone()}), current.as_deref() == Some(v.as_str()))));
        let shown = current.clone().unwrap_or_else(|| format!("Any {}", all_label.to_lowercase()));
        let shown = if shown.chars().count() > 26 { format!("{}…", shown.chars().take(25).collect::<String>()) } else { shown };
        row(ui, all_label, &mut |ui| picker(app, ui, id, &shown, current.is_some(), items.clone()));
    }
    ui.separator();
    // save / clear / presets
    let filtering = app.session.filter != Default::default();
    ui.horizontal(|ui| {
        let save = ui.add_enabled(filtering, egui::Button::new(egui::RichText::new(crate::i18n::tr("Save as Smart Collection…")).font(t.font(12.5))));
        register(ui.ctx(), "button:filterSave", save.rect);
        if save.clicked() {
            let _ = app.run("dialog.newSmartAlbum", json!({}));
            ui.close();
        }
        let clear = ui.add_enabled(filtering, egui::Button::new(egui::RichText::new(crate::i18n::tr("Clear All Filters")).font(t.font(12.5))));
        register(ui.ctx(), "button:filterClear", clear.rect);
        if clear.clicked() {
            app.ui.search.clear();
            let _ = app.run("library.clearFilter", json!({}));
        }
    });
    // filter presets: apply one, save the current filter, delete
    let presets = crate::widgets::dropdown(ui, "filterPresets", crate::i18n::tr("Filter Presets"), t.font(12.5), t.text);
    register(ui.ctx(), "button:filterPresets", presets.rect);
    egui::Popup::menu(&presets).show(|ui| {
        let names: Vec<String> = app.session.filter_presets.iter().map(|f| f.name.clone()).collect();
        for n in &names {
            let r = ui.button(n);
            if r.clicked() {
                let _ = app.run("filter.applyPreset", json!({"name": n}));
            }
            crate::widgets::context_menu(&r, |ui| {
                if ui.button(crate::i18n::tr("Delete Preset")).clicked() {
                    let _ = app.run("filter.deletePreset", json!({"name": n}));
                }
            });
        }
        if names.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::tr("No filter presets yet")).color(t.text_dim));
        }
        ui.separator();
        if ui.add_enabled(filtering, egui::Button::new(crate::i18n::tr("Save Current Filter as Preset…"))).clicked() {
            crate::panels::dialogs::prompt(app, "Save Filter Preset", "Preset name", "", "filter.savePreset", json!({}), "name");
        }
    });
}
