//! The Import tab's panels: the sources on the left (This PC with every drive and camera card,
//! places, favourites, recent folders), the bar over the culling grid and loupe (where you are,
//! the folder, Culling saved, and the one big Import button with Picks / Selected / All), and the
//! settings on the right (import preset, destination, camera profiles, the preset stack,
//! keywords, previews to build, what browsing writes).

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::Icon;
use crate::import_tab::Take;
use crate::theme::Tokens;
use crate::widgets::register;
use lightcraft_engine::LibrarySource;

/// A drive or volume to import from: (label, path, holds a camera card's DCIM folder).
type Drive = (String, String, bool);

/// The drives and volumes of this computer, read on a worker thread ([`super::left::fs_cached`]):
/// on Windows every drive letter that answers (local, USB, cards, mapped network drives); elsewhere
/// the root and the mounted volumes.
fn list_drives(_: &str) -> Vec<Drive> {
    let mut v: Vec<Drive> = Vec::new();
    let card = |p: &str| std::path::Path::new(p).join("DCIM").is_dir();
    if cfg!(windows) {
        for l in b'A'..=b'Z' {
            let p = format!("{}:\\", l as char);
            if std::fs::metadata(&p).is_ok() {
                v.push((format!("{}:", l as char), p.clone(), card(&p)));
            }
        }
    } else {
        v.push(("/".into(), "/".into(), false));
        let user = std::env::var("USER").unwrap_or_default();
        for base in ["/Volumes".to_string(), format!("/media/{user}"), format!("/run/media/{user}"), "/mnt".to_string()] {
            let Ok(rd) = std::fs::read_dir(&base) else { continue };
            let mut found: Vec<Drive> = rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
                .map(|e| {
                    let p = e.path().to_string_lossy().to_string();
                    (e.file_name().to_string_lossy().to_string(), p.clone(), card(&p))
                })
                .collect();
            found.sort();
            v.extend(found);
        }
    }
    v
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(text), t.semibold(12.0), t.text_dim);
}

/// The left panel in the Import tab: where to import from.
pub fn sources(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
        register(ui.ctx(), "sidebarTitle", hr);
        ui.painter().text(pos2(hr.left() + 18.0, hr.center().y - 7.0), Align2::LEFT_CENTER, crate::i18n::tr("Import from"), t.semibold(15.0), t.text);
        ui.painter().text(
            pos2(hr.left() + 18.0, hr.center().y + 10.0),
            Align2::LEFT_CENTER,
            crate::i18n::tr("A drive, card or folder"),
            t.font(11.0),
            t.text_dim,
        );
        let current = app.session.browse.clone().filter(|_| app.session.source == LibrarySource::Folder).map(|b| b.path);
        egui::ScrollArea::both().id_salt("import-sources").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let wide = viewport.width().max(super::left::content_width(ui.ctx()));
            ui.set_min_width(wide);
            ui.set_max_width(wide);
            ui.data_mut(|d| {
                d.insert_temp(egui::Id::new("left-content-width-next"), 0.0f32);
                d.insert_temp(egui::Id::new("left-visible-right"), ui.cursor().left() + viewport.max.x);
            });
            // This PC: every drive, cards first
            heading(ui, "This PC");
            let mut drives = super::left::fs_cached(ui, "import-drives", "", 5.0, list_drives).unwrap_or_default();
            drives.sort_by_key(|(_, _, card)| !*card);
            if drives.is_empty() {
                ui.label(egui::RichText::new(crate::i18n::tr("Looking for drives…")).color(t.text_dim));
            }
            for (label, path, card) in &drives {
                let (name, icon) =
                    if *card { (crate::i18n::tr_format!("Camera card ({label})", label = label), Icon::Card) } else { (label.clone(), Icon::Drive) };
                // a card opens at its DCIM folder
                let open_at = if *card { std::path::Path::new(path).join("DCIM").to_string_lossy().to_string() } else { path.clone() };
                super::left::folder_tree(app, ui, "drives", &name, &open_at, 0.0, current.as_deref(), false, false, icon);
            }
            // the usual places, the favourite folders (Add to Favourites, Browse Folder…) and the
            // folder being browsed, as Local lists them
            heading(ui, "Places");
            super::left::local_section(app, ui);
            // recent: the folders browsed last, newest first
            let mut recent: Vec<(String, String)> =
                app.session.catalog.browsed_folders().iter().map(|(folder, at)| (at.clone(), folder.clone())).collect();
            recent.sort_by(|a, b| b.cmp(a));
            let recent: Vec<String> = recent
                .into_iter()
                .map(|(_, f)| f)
                .filter(|f| super::left::fs_cached(ui, "is-dir", f, 10.0, |p| std::path::Path::new(p).is_dir()) == Some(true))
                .take(8)
                .collect();
            if !recent.is_empty() {
                heading(ui, "Recent");
                for f in &recent {
                    let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
                    let sel = current.as_deref().is_some_and(|c| super::left::same_folder(c, f));
                    let r = super::left::row(app, ui, &format!("recent:{f}"), Icon::Clock, &name, None, sel, 0.0).on_hover_text(f);
                    if r.clicked()
                        && let Err(e) = app.run("library.browse", json!({"path": f}))
                    {
                        app.toast(ui.ctx(), e);
                    }
                }
            }
            ui.add_space(8.0);
            if super::left::row(app, ui, "import:files", Icon::Photos, crate::i18n::tr("Import Individual Files…"), None, false, 0.0)
                .on_hover_text(crate::i18n::tr("Pick single files to import, with the import review"))
                .clicked()
                && let Err(e) = app.run("file.addPhotos", json!({}))
            {
                app.toast(ui.ctx(), e);
            }
            ui.add_space(16.0);
            let next = ui.data(|d| d.get_temp::<f32>(egui::Id::new("left-content-width-next"))).unwrap_or(0.0);
            if (next - super::left::content_width(ui.ctx())).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(egui::Id::new("left-content-width"), next));
                ui.ctx().request_repaint();
            }
        });
    });
}

/// The photos of the folder view as import candidates, for the camera-profile plan (cached per
/// view generation: thousands of photos).
fn candidates(app: &mut LightcraftApp, ctx: &egui::Context) -> Vec<lightcraft_engine::import::ImportCandidate> {
    let (generation, ids) = app.session.visible_shared();
    let key = egui::Id::new("import-tab-candidates");
    if let Some((g, c)) = ctx.data(|d| d.get_temp::<(u64, std::sync::Arc<Vec<lightcraft_engine::import::ImportCandidate>>)>(key))
        && g == generation
    {
        return (*c).clone();
    }
    let c: Vec<lightcraft_engine::import::ImportCandidate> = ids
        .iter()
        .filter_map(|id| app.session.catalog.photo(*id))
        .filter(|p| p.local)
        .map(|p| {
            let path = match &p.source {
                lightcraft_catalog::Source::File { path } => path.clone(),
                lightcraft_catalog::Source::Demo { .. } => p.file_name.clone(),
            };
            lightcraft_engine::import::ImportCandidate {
                body: lightcraft_engine::import_profiles::body_of(&p.meta.serial, &path),
                camera: p.meta.camera.clone(),
                name: p.file_name.clone(),
                format: p.format.clone(),
                kind: p.kind,
                path,
                ..Default::default()
            }
        })
        .collect();
    ctx.data_mut(|d| d.insert_temp(key, (generation, std::sync::Arc::new(c.clone()))));
    c
}

/// The right panel in the Import tab: the import's settings.
pub fn settings(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::ScrollArea::vertical().id_salt("import-settings").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 20, top: 10, bottom: 16 }).show(ui, |ui| {
            // everything wraps to the panel's width (less the scroll bar's room)
            ui.set_max_width((ui.available_width() - 12.0).max(120.0));
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(egui::RichText::new(crate::i18n::tr("Import settings")).font(t.semibold(15.0)).color(t.text));
            preset_row(app, ui);
            ui.separator();
            // the review's options, on the folder's photos (their bodies for the camera profiles)
            let mut d = app.ui.import_settings.opts.clone();
            d.candidates = candidates(app, ui.ctx());
            d.checked = vec![true; d.candidates.len()];
            crate::import::options(app, ui, &mut d);
            let d = crate::import_tab::settings_only(d);
            if d != app.ui.import_settings.opts {
                app.ui.import_settings.opts = d;
                app.ui.import_settings.preset_name.clear();
            }
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::tr("Previews to build")).color(t.text_label));
            let keys = ["none", "standard", "full"];
            let cur = keys.iter().position(|k| *k == app.ui.import_settings.previews);
            if let Some(i) = crate::widgets::segmented(ui, "importPreviews", &[("None", "none"), ("Standard", "standard"), ("1:1", "full")], cur, 3)
                && let Some(k) = keys.get(i)
            {
                app.ui.import_settings.previews = (*k).to_string();
            }
            ui.label(
                egui::RichText::new(crate::i18n::tr(
                    "After the import, LightCraft gets the photos ready by itself: the presets' AI masks, Smooth Skin, then these previews. You can keep working meanwhile.",
                ))
                .color(t.text_dim)
                .small(),
            );
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::tr("While browsing")).color(t.text_label));
            let mut write = app.session.xmp.write_browsed;
            let r = ui.radio_value(&mut write, false, crate::i18n::tr("Keep culling and edits in LightCraft until import"));
            register(ui.ctx(), "radio:browseKeep", r.rect);
            let r = ui.radio_value(&mut write, true, crate::i18n::tr("Also write them into the folder's XMP files"));
            register(ui.ctx(), "radio:browseWrite", r.rect);
            if write != app.session.xmp.write_browsed
                && let Err(e) = app.run("library.xmpPreferences", json!({"writeBrowsed": write}))
            {
                app.toast(ui.ctx(), e);
            }
            ui.label(
                egui::RichText::new(crate::i18n::tr("Nothing is written into the folder you browse unless you choose to. Culling is saved in the library either way."))
                    .color(t.text_dim)
                    .small(),
            );
        });
    });
}

/// Import preset: choose one, save the settings as one, delete one.
fn preset_row(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = if app.ui.import_settings.preset_name.is_empty() {
            crate::i18n::tr("Import preset: none").to_string()
        } else {
            crate::i18n::tr_format!("Import preset: {name}", name = app.ui.import_settings.preset_name)
        };
        let names: Vec<String> = app.session.import_presets.iter().map(|p| p.name.clone()).collect();
        let mut chosen = None;
        let combo = crate::widgets::combo_box("import-preset-pick")
            .selected_text(cur)
            .width((ui.available_width() - 70.0).max(80.0))
            .truncate()
            .show_ui(ui, |ui| {
                if names.is_empty() {
                    ui.label(crate::i18n::tr("No import presets yet: set things up, then Save"));
                }
                for n in &names {
                    if ui.selectable_label(*n == app.ui.import_settings.preset_name, n).clicked() {
                        chosen = Some(n.clone());
                    }
                }
            });
        register(ui.ctx(), "combo:importPreset", combo.response.rect);
        if let Some(n) = chosen
            && let Err(e) = app.run("import.usePreset", json!({"name": n}))
        {
            app.toast(ui.ctx(), e);
        }
        if crate::widgets::text_button(ui, "importPresetSave", crate::i18n::tr("Save…"), false)
            .on_hover_text(crate::i18n::tr("Keep these settings as an import preset"))
            .clicked()
        {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: crate::i18n::tr("Save Import Preset").into(),
                hint: "Name, e.g. Wedding".into(),
                value: app.ui.import_settings.preset_name.clone(),
                command: "import.saveSettings".into(),
                params: json!({}),
                key: "name".into(),
            });
        }
    });
    if !app.ui.import_settings.preset_name.is_empty() {
        let name = app.ui.import_settings.preset_name.clone();
        if crate::widgets::text_button(ui, "importPresetDelete", crate::i18n::tr("Delete this preset"), false).clicked() {
            match app.run("import.deletePreset", json!({"name": name})) {
                Ok(_) => app.ui.import_settings.preset_name.clear(),
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
    }
}

/// The counts the big button offers: (picks, selected, all).
fn counts(app: &mut LightcraftApp) -> (usize, usize, usize) {
    let (p, s, a) = app.session.import_targets();
    (p.len(), s.len(), a.len())
}

/// The bar over the grid and the loupe in the Import tab: where you are and the Import button.
pub fn bar(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(b) = app.session.browse.clone().filter(|_| app.session.source == LibrarySource::Folder) else { return };
    let (picks, selected, all) = counts(app);
    let library = crate::import_tab::library_name(app);
    let folder = std::path::Path::new(&b.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| b.path.clone());
    let frame = egui::Frame::NONE.fill(t.canvas).inner_margin(egui::Margin { left: 16, right: 12, top: 8, bottom: 8 });
    let resp = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // where you are: browsing, not imported
            let badge = crate::i18n::tr("IMPORT");
            let g = ui.painter().layout_no_wrap(badge.to_string(), t.semibold(10.5), t.canvas);
            let (r, _) = ui.allocate_exact_size(g.size() + vec2(12.0, 6.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, t.accent);
            ui.painter().galley(r.center() - g.size() / 2.0, g, t.canvas);
            register(ui.ctx(), "badge:import", r);
            let title = crate::i18n::tr_format!("Browsing {folder}: not imported", folder = folder);
            let r = ui.add(egui::Label::new(egui::RichText::new(title).font(t.semibold(14.0)).color(t.text))).on_hover_text(&b.path);
            register(ui.ctx(), "label:importWhere", r.rect);
            ui.add_space(12.0);
            // the path, each part opening that folder
            let parts = super::grid::crumbs(&b.path).len().max(1) as f32;
            let w = ui.available_width().min(150.0 * parts).max(0.0);
            ui.allocate_ui_with_layout(vec2(w, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                super::grid::folder_breadcrumbs(app, ui, &b.path);
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let mut sub = b.subfolders;
            let c = ui.checkbox(&mut sub, crate::i18n::tr("Include subfolders"));
            register(ui.ctx(), "check:includeSubfolders", c.rect);
            if c.changed() {
                let _ = app.run("library.browse", json!({"path": b.path, "subfolders": sub}));
            }
            // culling is kept in the library as it is done: say so
            let (text, colour) = if app.session.unsaved().is_some() {
                (crate::i18n::tr("Culling not saved yet: retrying").to_string(), t.caution)
            } else {
                (format!("✓ {}", crate::i18n::tr("Culling saved")), t.text_dim)
            };
            let r = ui.label(egui::RichText::new(text).color(colour)).on_hover_text(crate::i18n::tr(
                "Flags, stars and labels on these photos are kept in the library at once, imported or not, and come back whenever you browse this folder again.",
            ));
            register(ui.ctx(), "label:cullingSaved", r.rect);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let take = app.ui.import_settings.take();
                let n = match take {
                    Take::Picks => picks,
                    Take::Selected => selected,
                    Take::All => all,
                };
                let label = crate::i18n::tr_format!("Import {n} into {library}", n = n, library = library);
                let busy = app.import.as_ref().is_some_and(|t| !t.browse());
                let go = crate::button::Button::new(&label)
                    .icon(Icon::ImportArrow)
                    .primary()
                    .large()
                    .enabled(n > 0 && !busy)
                    .widget("button:importNow")
                    .tooltip(crate::i18n::tr("Import these photos into the library, with the settings on the right"))
                    .show(ui);
                if go.clicked()
                    && let Err(e) = app.run("import.start", json!({}))
                {
                    app.toast(ui.ctx(), e);
                }
                let items = [
                    (crate::i18n::tr_format!("Picks ({n})", n = picks), "picks"),
                    (crate::i18n::tr_format!("Selected ({n})", n = selected), "selected"),
                    (crate::i18n::tr_format!("All ({n})", n = all), "all"),
                ];
                // right to left: All, Selected, Picks
                for (text, key) in items.iter().rev() {
                    let on = app.ui.import_settings.take == *key;
                    if crate::widgets::text_button(ui, &format!("importTake-{key}"), text, on).clicked() {
                        app.ui.import_settings.take = (*key).to_string();
                    }
                }
            });
        });
    });
    register(ui.ctx(), "grid:folderHeader", resp.response.rect);
}

/// The Import tab before a folder is chosen.
pub fn choose_source(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let r = ui.max_rect();
    ui.painter().rect_filled(r, 0.0, t.grid_bg);
    super::empty_message(
        ui,
        r.translate(vec2(0.0, -30.0)),
        "Choose what to import",
        "Pick a drive, camera card or folder on the left to see its photos and cull them",
    );
    if app.services.pick_folder.is_some() {
        let size = vec2(180.0, 36.0);
        let br = Rect::from_center_size(r.center() + vec2(0.0, 40.0), size);
        let resp = crate::button::Button::new(crate::i18n::tr("Browse Folder…")).primary().large().widget("button:importBrowse").show_at(ui, br);
        if resp.clicked() {
            let picked = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(path) = picked
                && let Err(e) = app.run("library.browse", json!({"path": path}))
            {
                app.toast(ui.ctx(), e);
            }
        }
    }
    let _ = StrokeKind::Inside;
}
