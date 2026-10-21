//! The Import tab's panels: the sources on the left (This PC with every drive and camera card,
//! places, favourites, recent folders), the bar over the culling grid and loupe (where you are,
//! the folder, Culling saved, and the one big Import button with Picks / Selected / All), and the
//! settings on the right (import preset, destination, camera profiles, the preset stack,
//! keywords, previews to build, what browsing writes).

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
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

/// The right panel in the Import tab: the import's settings, laid out for a narrow panel: each
/// setting's name above its control, every control as wide as the panel, the buttons of a row one
/// size, long text wrapped. Nothing is ever cut off at the panel's edge.
pub fn settings(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    // the width every control takes: the panel's, less the margins and the scroll bar's room
    let w = settings_width(ui.available_width());
    egui::ScrollArea::vertical().id_salt("import-settings").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 22, top: 10, bottom: 16 }).show(ui, |ui| {
            ui.set_max_width(w);
            ui.spacing_mut().item_spacing.y = 6.0;
            let t = Tokens::get(ui.ctx());
            ui.label(egui::RichText::new(crate::i18n::tr("Import settings")).font(t.semibold(15.0)).color(t.text));
            preset_row(app, ui, w);
            ui.separator();
            let mut s = app.ui.import_settings.clone();
            transfer(app, ui, &mut s.opts, w);
            album(app, ui, &mut s.opts, w);
            camera_profiles(app, ui, &mut s.opts, w);
            look(app, ui, &mut s.opts, w);
            preset_stack(app, ui, &mut s.opts, w);
            metadata(app, ui, &mut s.opts, w);
            caption(ui, "Keywords");
            let r = crate::text_field::TextField::singleline("field:importKeywords", &mut s.opts.keywords)
                .hint(crate::i18n::tr("comma, separated"))
                .width(w)
                .show(ui)
                .response;
            register(ui.ctx(), "field:importKeywords", r.rect);
            ui.separator();
            caption(ui, "Previews to build");
            let keys = ["none", "standard", "full"];
            let cur = keys.iter().position(|k| *k == s.previews);
            if let Some(i) = crate::widgets::segmented(ui, "importPreviews", &[("None", "none"), ("Standard", "standard"), ("1:1", "full")], cur, 3)
                && let Some(k) = keys.get(i)
            {
                s.previews = (*k).to_string();
            }
            help(
                ui,
                "After the import, LightCraft gets the photos ready by itself: the presets' AI masks, Smooth Skin, then these previews. You can keep working meanwhile.",
            );
            ui.separator();
            caption(ui, "While browsing");
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
            help(ui, "Nothing is written into the folder you browse unless you choose to. Culling is saved in the library either way.");
            store(app, s);
        });
    });
}

/// The width the settings' controls take in a panel `available` points wide (its margins and the
/// scroll bar's room left out; never so small that nothing fits).
pub(crate) fn settings_width(available: f32) -> f32 {
    let w = (available - 16.0 - 22.0 - 6.0).floor();
    if w.is_finite() { w.max(120.0) } else { 120.0 }
}

/// Keep the settings the panel left (an import preset no longer describes changed ones).
fn store(app: &mut LightcraftApp, mut s: crate::import_tab::ImportSettings) {
    s.opts = crate::import_tab::settings_only(s.opts);
    if s == app.ui.import_settings {
        return;
    }
    if s.opts != app.ui.import_settings.opts || s.previews != app.ui.import_settings.previews {
        s.preset_name.clear();
    }
    app.ui.import_settings = s;
}

/// A setting's name, above its control.
fn caption(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(text)).color(t.text_label)).wrap());
}

/// A small explanation under a control, wrapped to the panel.
fn help(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(text)).color(t.text_dim).small()).wrap());
}

/// A drop-down as wide as the panel; a long choice is cut with an ellipsis, never the panel.
fn combo(id: &str, w: f32) -> egui::ComboBox {
    crate::widgets::combo_box(id).width(w).truncate()
}

/// Transfer (in place / copy / move), and for a copy or move the destination, folders, renaming.
fn transfer(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    let t = Tokens::get(ui.ctx());
    caption(ui, "Transfer");
    let cur = Some(match (d.copy, d.move_files) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    });
    // three to a row while they fit, else one below the other (one size either way)
    let per_row = if w >= 200.0 { 3 } else { 1 };
    if let Some(i) = crate::widgets::segmented(ui, "importMode", &[("In place", "add"), ("Copy", "copy"), ("Move", "move")], cur, per_row) {
        d.copy = i > 0;
        d.move_files = i == 2;
    }
    help(
        ui,
        match (d.copy, d.move_files) {
            (true, false) => "Copy: the files are copied to the folder below and the library uses the copies; the originals are left as they are.",
            (true, true) => {
                "Move: the files are moved to the folder below; each original (and its XMP sidecar) is removed from the source only after its copy is verified. Duplicates and files that fail stay where they are."
            }
            _ => "Add in place: the library references the files where they are; nothing is copied or moved.",
        },
    );
    if !d.copy {
        return;
    }
    caption(ui, if d.move_files { "Move to" } else { "Copy to" });
    let shown = if d.destination.trim().is_empty() { crate::i18n::tr("Library Originals").to_string() } else { d.destination.clone() };
    let r = ui.add(egui::Label::new(egui::RichText::new(shown).color(t.text)).wrap());
    register(ui.ctx(), "label:importDestination", r.rect);
    let mut items: Vec<(&str, &str)> = Vec::new();
    if app.services.pick_folder.is_some() {
        items.push(("Choose…", "choose"));
    }
    if !d.destination.is_empty() {
        items.push(("Library", "library"));
    }
    if !items.is_empty() {
        let n = items.len();
        match crate::widgets::segmented(ui, "importDest", &items, None, n).and_then(|i| items.get(i)).map(|x| x.1) {
            Some("choose") => {
                if let Some(f) = app.services.pick_folder.as_mut().and_then(|f| f()) {
                    d.destination = f;
                }
            }
            Some("library") => d.destination.clear(),
            _ => {}
        }
    }
    caption(ui, "Folders");
    let opts =
        [("date", "By day (YYYY/YYYY-MM-DD)"), ("month", "By month (YYYY/YYYY-MM)"), ("flat", "Into one folder"), ("custom", "Custom template…")];
    let key = if d.organize.is_empty() { "date".to_string() } else { d.organize.clone() };
    let cur = opts.iter().find(|o| o.0 == key).map_or(opts[0].1, |o| o.1);
    let r = combo("import-organize", w).selected_text(crate::i18n::tr(cur)).show_ui(ui, |ui| {
        for (k, label) in opts {
            let r = ui.selectable_label(key == k, crate::i18n::tr(label));
            register(ui.ctx(), format!("button:importOrganize-{k}"), r.rect);
            if r.clicked() {
                d.organize = k.to_string();
                if k == "custom" && d.folder_template.trim().is_empty() {
                    d.folder_template = crate::import::DEFAULT_FOLDER_TEMPLATE.into();
                }
            }
        }
    });
    register(ui.ctx(), "combo:importOrganize", r.response.rect);
    if d.organize == "custom" {
        let r = crate::text_field::TextField::singleline("field:importFolderTemplate", &mut d.folder_template)
            .hint(crate::import::DEFAULT_FOLDER_TEMPLATE)
            .width(w)
            .show(ui)
            .response;
        register(ui.ctx(), "field:importFolderTemplate", r.rect);
        if let Some(e) = lightcraft_engine::rename::folder_template_error(&d.folder_template) {
            ui.add(egui::Label::new(egui::RichText::new(e).color(t.caution)).wrap());
        }
    }
    caption(ui, "Rename");
    let r = crate::text_field::TextField::singleline("field:importRename", &mut d.rename)
        .hint(crate::i18n::tr("keep names — or e.g. {date}_{seq:3}"))
        .width(w)
        .show(ui)
        .response;
    register(ui.ctx(), "field:importRename", r.rect);
    if !d.move_files {
        let r = ui.checkbox(&mut d.dng, crate::i18n::tr("Copy as DNG"));
        register(ui.ctx(), "check:importDng", r.rect);
    }
}

fn album(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    caption(ui, "Collection");
    let mut albums: Vec<(u64, String)> =
        app.session.catalog.albums().filter(|a| !a.folder && !a.is_smart()).map(|a| (a.id.0, a.name.clone())).collect();
    albums.sort_by_key(|(_, n)| n.to_lowercase());
    let cur = match d.album {
        Some(a) => albums.iter().find(|x| x.0 == a).map(|x| x.1.clone()).unwrap_or_default(),
        None if !d.new_album.is_empty() => crate::i18n::tr("New collection").into(),
        None => crate::i18n::tr("None").into(),
    };
    let r = combo("import-album", w).selected_text(cur).show_ui(ui, |ui| {
        if ui.selectable_label(d.album.is_none() && d.new_album.is_empty(), crate::i18n::tr("None")).clicked() {
            d.album = None;
            d.new_album.clear();
        }
        if ui.selectable_label(d.album.is_none() && !d.new_album.is_empty(), crate::i18n::tr("New collection…")).clicked() {
            d.album = None;
            if d.new_album.is_empty() {
                d.new_album = crate::i18n::tr("Imported Photos").into();
            }
        }
        for (id, name) in &albums {
            if ui.selectable_label(d.album == Some(*id), name).clicked() {
                d.album = Some(*id);
                d.new_album.clear();
            }
        }
    });
    register(ui.ctx(), "combo:importAlbum", r.response.rect);
    if d.album.is_none() && !d.new_album.is_empty() {
        let r = crate::text_field::TextField::singleline("field:importAlbumName", &mut d.new_album).width(w).show(ui).response;
        register(ui.ctx(), "field:importAlbumName", r.rect);
    }
}

/// Camera profiles: each body of the folder with the profile it will develop with. Picking one
/// there makes it that body's default (kept with the profiles: every later import and library
/// photo of the body gets it). The Profile Manager has the rest.
fn camera_profiles(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    use lightcraft_engine::import_profiles;
    let t = Tokens::get(ui.ctx());
    caption(ui, "Camera profiles");
    let all = d.profile_mode == "all";
    let cur = if all { "One profile for all photos" } else { "Each body's own profile" };
    let r = combo("import-camera-profile", w).selected_text(crate::i18n::tr(cur)).show_ui(ui, |ui| {
        if ui.selectable_label(!all, crate::i18n::tr("Each body's own profile")).clicked() {
            d.profile_mode.clear();
        }
        if ui.selectable_label(all, crate::i18n::tr("One profile for all photos")).clicked() {
            d.profile_mode = "all".into();
            if d.profile_all.is_empty() {
                d.profile_all = import_profiles::BUILTIN.into();
            }
        }
    });
    register(ui.ctx(), "combo:importCameraProfile", r.response.rect);
    if all {
        let name = if d.profile_all == import_profiles::BUILTIN {
            crate::i18n::tr("LightCraft colour").to_string()
        } else {
            import_profiles::profile_name(&app.session, Some(&d.profile_all))
        };
        let r = combo("import-camera-profile-all", w).selected_text(name).height(300.0).show_ui(ui, |ui| {
            if ui.selectable_label(d.profile_all == import_profiles::BUILTIN, crate::i18n::tr("LightCraft colour")).clicked() {
                d.profile_all = import_profiles::BUILTIN.into();
            }
            for e in &app.session.dcp_profiles {
                if ui.selectable_label(d.profile_all == e.id, &e.name).clicked() {
                    d.profile_all = e.id.clone();
                }
            }
        });
        register(ui.ctx(), "combo:importCameraProfileAll", r.response.rect);
        return;
    }
    // (the review's per-import overrides are not offered here: the body's default is)
    d.profile_per_body.clear();
    d.adopt.clear();
    let cands = candidates(app, ui.ctx());
    let refs: Vec<&lightcraft_engine::import::ImportCandidate> = cands.iter().collect();
    let rows = import_profiles::plan(&app.session, &refs, &import_profiles::ProfileChoice::Auto);
    let mut unset = false;
    for (i, row) in rows.iter().enumerate() {
        let who = if row.body_label.is_empty() { row.camera.clone() } else { format!("{} · {}", row.camera, row.body_label) };
        let who = if who.trim().is_empty() { crate::i18n::tr("Unknown camera").to_string() } else { who };
        let text = crate::i18n::tr_format!("{who}: {n} photos", who = who, n = crate::i18n::group_digits(row.photos));
        let r = ui.add(egui::Label::new(egui::RichText::new(text).color(t.text)).wrap());
        register(ui.ctx(), format!("label:importProfileRow-{i}"), r.rect);
        let Some(body) = row.body.clone() else {
            help(ui, "These files don't say which body took them: they develop with the camera's default.");
            continue;
        };
        let current = match &row.profile {
            Some(_) => row.profile_name.clone(),
            None => {
                unset = true;
                crate::i18n::tr("LightCraft colour (no profile set)").to_string()
            }
        };
        let mut pick: Option<String> = None;
        let r = combo(&format!("import-body-{i}"), w).selected_text(current).height(300.0).show_ui(ui, |ui| {
            let (mine, model, others) = import_profiles::choices(&app.session, &row.camera, Some(&body));
            let mut entry = |ui: &mut egui::Ui, id: &str, label: &str| {
                if ui.selectable_label(row.profile.as_deref() == Some(id), label).clicked() {
                    pick = Some(id.to_string());
                }
            };
            if !mine.is_empty() {
                ui.label(egui::RichText::new(crate::i18n::tr("This body")).small().color(t.text_dim));
                for (id, name) in &mine {
                    entry(ui, id, name);
                }
            }
            if !model.is_empty() {
                ui.separator();
                for (id, name) in &model {
                    entry(ui, id, name);
                }
            }
            if !others.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(crate::i18n::tr("Other bodies")).small().color(t.text_dim));
                for (id, name, bodies) in &others {
                    entry(ui, id, &format!("{name} ({bodies})"));
                }
            }
            if mine.is_empty() && model.is_empty() && others.is_empty() {
                ui.label(
                    egui::RichText::new(crate::i18n::tr("No camera profiles yet: import or create them in the Profile Manager")).color(t.text_dim),
                );
            }
        });
        register(ui.ctx(), format!("combo:importBodyProfile-{i}"), r.response.rect);
        r.response.on_hover_text(crate::i18n::tr("The profile this body's raws develop with: it becomes the body's default"));
        if let Some(id) = pick
            && let Err(e) = app.run("profile.adoptForBody", json!({"profile": id, "serial": body, "makeDefault": true}))
        {
            app.toast(ui.ctx(), e);
        }
    }
    if unset {
        help(ui, "A body without a profile of its own develops with LightCraft colour. Pick one above: it becomes that body's default.");
    }
    if crate::widgets::segmented(ui, "importProfileManager", &[("Profile Manager…", "open")], None, 1).is_some()
        && let Err(e) = app.run("app.settings", json!({"tab": "profiles"}))
    {
        app.toast(ui.ctx(), e);
    }
}

fn look(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    caption(ui, "Look");
    let cur = if d.look.is_empty() {
        crate::i18n::tr("None").to_string()
    } else {
        let name = app.session.profile_info(&d.look).map(|(n, _)| n).unwrap_or(&d.look);
        crate::i18n::profile_label(&d.look, name).to_string()
    };
    // LightCraft's looks the Profile Manager still shows (not hidden, not taken off this computer)
    let shown: std::collections::HashSet<String> = app.session.shown_profiles().into_iter().map(|p| p.id).collect();
    let r = combo("import-look", w).selected_text(cur).height(300.0).show_ui(ui, |ui| {
        if ui.selectable_label(d.look.is_empty(), crate::i18n::tr("None")).clicked() {
            d.look.clear();
        }
        for p in lightcraft_engine::presets::PROFILES.iter().filter(|p| p.id != lightcraft_develop::BUILTIN_CAMERA_COLOR && shown.contains(p.id)) {
            let name = app.session.profile_info(p.id).map_or(p.name, |(n, _)| n);
            if ui.selectable_label(d.look == p.id, crate::i18n::profile_label(p.id, name)).clicked() {
                d.look = p.id.to_string();
            }
        }
    });
    register(ui.ctx(), "combo:importLook", r.response.rect);
}

/// The presets applied on import, in order: each row moves up or down or goes; Add puts one at the end.
fn preset_stack(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    if !d.preset.is_empty() {
        d.presets.insert(0, std::mem::take(&mut d.preset));
    }
    caption(ui, "Presets");
    let t = Tokens::get(ui.ctx());
    let n = d.presets.len();
    let mut action: Option<(usize, i32)> = None;
    for i in 0..n {
        let id = d.presets.get(i).cloned().unwrap_or_default();
        let name = app
            .session
            .presets
            .iter()
            .find(|p| p.id == id)
            .map(|p| crate::i18n::builtin_label(&p.name, p.builtin).to_string())
            .unwrap_or_else(|| crate::i18n::tr("(missing preset)").to_string());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let sz = vec2(24.0, 24.0);
            if crate::widgets::icon_button(ui, &format!("importPresetUp-{i}"), Icon::ChevronUp, sz, false, i > 0, crate::i18n::tr("Apply earlier"))
                .clicked()
            {
                action = Some((i, -1));
            }
            if crate::widgets::icon_button(
                ui,
                &format!("importPresetDown-{i}"),
                Icon::ChevronDown,
                sz,
                false,
                i + 1 < n,
                crate::i18n::tr("Apply later"),
            )
            .clicked()
            {
                action = Some((i, 1));
            }
            if crate::widgets::icon_button(
                ui,
                &format!("importPresetRemove-{i}"),
                Icon::Close,
                sz,
                false,
                true,
                crate::i18n::tr("Remove from the stack"),
            )
            .clicked()
            {
                action = Some((i, 0));
            }
            let r = ui.add(egui::Label::new(egui::RichText::new(format!("{}. {name}", i + 1)).color(t.text)).truncate());
            register(ui.ctx(), format!("label:importPreset-{i}"), r.rect);
        });
    }
    match action {
        Some((i, 0)) if i < d.presets.len() => {
            d.presets.remove(i);
        }
        Some((i, -1)) if i > 0 && i < d.presets.len() => d.presets.swap(i, i - 1),
        Some((i, 1)) if i + 1 < d.presets.len() => d.presets.swap(i, i + 1),
        _ => {}
    }
    let label = if d.presets.is_empty() { crate::i18n::tr("Add a preset…") } else { crate::i18n::tr("Add another preset…") };
    // (the shared picker: Favorites and At Import first, search, groups, a preview on the photo)
    if let Some(crate::preset_picker::Picked::Preset(pid)) =
        crate::preset_picker::Picker::new("importPresetAdd").for_import(true).button(app, ui, label, w)
    {
        d.presets.push(pid);
    }
    if d.presets.len() > 1 {
        help(ui, "Applied in this order: each sets its settings and adds its masks.");
    }
    if !d.presets.is_empty() {
        let r = ui.checkbox(&mut d.preset_profile, crate::i18n::tr("Presets may change the camera profile"));
        register(ui.ctx(), "check:importPresetProfile", r.rect);
        help(ui, "Off: each raw keeps its camera body's profile, whatever profile a preset names.");
    }
}

fn metadata(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut crate::import::ImportDialog, w: f32) {
    if app.session.metadata_presets.is_empty() {
        return;
    }
    caption(ui, "Metadata");
    let cur = if d.metadata_preset.is_empty() { crate::i18n::tr("None").to_string() } else { d.metadata_preset.clone() };
    let r = combo("import-metadata", w).selected_text(cur).show_ui(ui, |ui| {
        if ui.selectable_label(d.metadata_preset.is_empty(), crate::i18n::tr("None")).clicked() {
            d.metadata_preset.clear();
        }
        for m in &app.session.metadata_presets {
            if ui.selectable_label(d.metadata_preset == m.name, &m.name).clicked() {
                d.metadata_preset = m.name.clone();
            }
        }
    });
    register(ui.ctx(), "combo:importMetadata", r.response.rect);
}

/// Import preset: the one these settings came from (or none), as wide as the panel; Save… and
/// Delete beside each other, one size.
fn preset_row(app: &mut LightcraftApp, ui: &mut egui::Ui, w: f32) {
    let cur = if app.ui.import_settings.preset_name.is_empty() {
        crate::i18n::tr("Import preset: none").to_string()
    } else {
        crate::i18n::tr_format!("Import preset: {name}", name = app.ui.import_settings.preset_name)
    };
    let names: Vec<String> = app.session.import_presets.iter().map(|p| p.name.clone()).collect();
    let mut chosen = None;
    let combo_r = combo("import-preset-pick", w).selected_text(cur).show_ui(ui, |ui| {
        if names.is_empty() {
            ui.label(crate::i18n::tr("No import presets yet: set things up, then Save"));
        }
        for n in &names {
            if ui.selectable_label(*n == app.ui.import_settings.preset_name, n).clicked() {
                chosen = Some(n.clone());
            }
        }
    });
    register(ui.ctx(), "combo:importPreset", combo_r.response.rect);
    if let Some(n) = chosen
        && let Err(e) = app.run("import.usePreset", json!({"name": n}))
    {
        app.toast(ui.ctx(), e);
    }
    let named = !app.ui.import_settings.preset_name.is_empty();
    let items: Vec<(&str, &str)> = if named { vec![("Save…", "save"), ("Delete", "delete")] } else { vec![("Save…", "save")] };
    let n = items.len();
    match crate::widgets::segmented(ui, "importPresetAction", &items, None, n).and_then(|i| items.get(i)).map(|x| x.1) {
        Some("save") => {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: crate::i18n::tr("Save Import Preset").into(),
                hint: "Name, e.g. Wedding".into(),
                value: app.ui.import_settings.preset_name.clone(),
                command: "import.saveSettings".into(),
                params: json!({}),
                key: "name".into(),
            });
        }
        Some("delete") => {
            let name = app.ui.import_settings.preset_name.clone();
            match app.run("import.deletePreset", json!({"name": name})) {
                Ok(_) => app.ui.import_settings.preset_name.clear(),
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        _ => {}
    }
}

/// The counts the big button offers: (picks, selected, all not imported yet).
fn counts(app: &mut LightcraftApp) -> (usize, usize, usize) {
    let (p, s, a) = app.session.import_targets();
    (p.len(), s.len(), a.len())
}

/// `text` cut to `max` points with an ellipsis at the end.
fn elide_end(text: &str, max: f32, width: impl Fn(&str) -> f32) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        n -= 1;
        let s: String = chars.iter().take(n).collect::<String>() + "…";
        if width(&s) <= max {
            return s;
        }
    }
    "…".to_string()
}

/// Which parts of a path the compact breadcrumb shows within `room`: all when they fit; else the
/// first part (the drive), "…" for the middle, and as many of the last folders as fit (always the
/// last). `widths[i]`: part i's width; `sep`: a separator's; `dots`: the "…"'s. `None` is the "…".
pub(crate) fn compact_crumbs(widths: &[f32], sep: f32, dots: f32, room: f32) -> Vec<Option<usize>> {
    let n = widths.len();
    if n == 0 {
        return Vec::new();
    }
    let total = |idx: &[Option<usize>]| -> f32 {
        idx.iter().map(|i| i.and_then(|i| widths.get(i).copied()).unwrap_or(dots)).sum::<f32>() + sep * idx.len().saturating_sub(1) as f32
    };
    let all: Vec<Option<usize>> = (0..n).map(Some).collect();
    if total(&all) <= room || n == 1 {
        return all;
    }
    let mut best: Vec<Option<usize>> = vec![None, Some(n - 1)];
    for k in 1..n.saturating_sub(1) {
        let mut v = vec![Some(0), None];
        v.extend((n - k..n).map(Some));
        if total(&v) <= room {
            best = v;
        } else {
            break;
        }
    }
    best
}

/// The bar over the grid and the loupe in the Import tab, in one row: the IMPORT badge, the folder
/// as a compact breadcrumb, Include subfolders, Culling saved, Picks / Selected / not imported (one
/// segmented control) and the Import button, all of one height.
pub fn bar(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    const H: f32 = 30.0;
    let t = Tokens::get(ui.ctx());
    let Some(b) = app.session.browse.clone().filter(|_| app.session.source == LibrarySource::Folder) else { return };
    let (picks, selected, all) = counts(app);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), H + 14.0), Sense::hover());
    ui.painter().rect_filled(row, 0.0, t.canvas);
    register(ui.ctx(), "grid:folderHeader", row);
    let inner = row.shrink2(vec2(12.0, 7.0));
    let cy = inner.center().y;
    let gap = 8.0;
    let painter = ui.painter().clone();
    let measure = |s: &str, f: &egui::FontId| painter.layout_no_wrap(s.to_string(), f.clone(), t.text).size().x;

    // the Import button (the library's name cut to leave room for the rest)
    let take = app.ui.import_settings.take();
    let n = match take {
        Take::Picks => picks,
        Take::Selected => selected,
        Take::All => all,
    };
    let full_name = crate::import_tab::library_name(app);
    let library = elide_end(&full_name, (inner.width() * 0.2).max(80.0), |s| measure(s, &t.semibold(12.5)));
    let long_label = crate::i18n::tr_format!("Import {n} into {library}", n = crate::i18n::group_digits(n), library = library);
    let short_label = crate::i18n::tr_format!("Import {n}", n = crate::i18n::group_digits(n));
    let busy = app.import.as_ref().is_some_and(|t| !t.browse());
    let width_of = |label: &str| crate::button::Button::new(label).icon(Icon::ImportArrow).desired_size(ui).x;
    let (long_w, short_w) = (width_of(&long_label), width_of(&short_label));
    // what the row leaves the breadcrumb: Culling saved goes to its icon, then the button drops the
    // library's name (it is in the tooltip), before the folder is squeezed below a readable width
    let (saved_text, saved_colour) = if app.session.unsaved().is_some() {
        (crate::i18n::tr("Culling not saved yet: retrying").to_string(), t.caution)
    } else {
        (crate::i18n::tr("Culling saved").to_string(), t.text_dim)
    };
    let small = t.font(11.5);
    let saved_full = measure(&saved_text, &small) + 18.0;
    let seg_font = t.semibold(11.5);
    let picks_t = crate::i18n::tr_format!("Picks ({n})", n = crate::i18n::group_digits(picks));
    let selected_t = crate::i18n::tr_format!("Selected ({n})", n = crate::i18n::group_digits(selected));
    let all_t = crate::i18n::tr_format!("{n} not imported", n = crate::i18n::group_digits(all));
    let seg_w = [&picks_t, &selected_t, &all_t].iter().map(|s| measure(s, &seg_font)).fold(0.0f32, f32::max) + 20.0;
    let badge = crate::i18n::tr("IMPORT");
    let badge_w = measure(badge, &t.semibold(10.5)) + 12.0;
    let crumbs = super::grid::crumbs(&b.path);
    let crumbs_w: f32 = crumbs.iter().map(|(name, _)| measure(name, &t.font(13.0)) + 20.0).sum();
    let want = crumbs_w.min(180.0);
    let fixed = badge_w + 10.0 + H + gap * 1.5 + seg_w * 3.0 + gap * 1.5 + gap;
    let left_for = |saved: f32, button: f32| inner.width() - fixed - saved - gap * 1.5 - button;
    let (saved_full_on, long_on) = if left_for(saved_full, long_w) >= want {
        (true, true)
    } else if left_for(14.0, long_w) >= want {
        (false, true)
    } else {
        (false, false)
    };
    let label = if long_on { long_label } else { short_label };
    let button = crate::button::Button::new(&label).icon(Icon::ImportArrow).primary().enabled(n > 0 && !busy).widget("button:importNow");
    let bw = if long_on { long_w } else { short_w };
    let br = Rect::from_min_size(pos2(inner.right() - bw, cy - H / 2.0), vec2(bw, H));
    let go = button.show_at(ui, br);
    let go = go
        .on_hover_text(crate::i18n::tr_format!("Import these photos into {library}, with the settings on the right", library = full_name))
        .on_disabled_hover_text(crate::i18n::tr(if busy {
            "An import is running"
        } else {
            "Nothing to import: flag photos as picks (P), select some, or choose All"
        }));
    if go.clicked()
        && let Err(e) = app.run("import.start", json!({}))
    {
        app.toast(ui.ctx(), e);
    }
    let mut right = br.left() - gap;

    // Picks / Selected / not imported: one control, segments of one size
    let items = [
        (picks_t, "picks", crate::i18n::tr("Flagged as picks (P)")),
        (selected_t, "selected", crate::i18n::tr("The photos selected here")),
        (all_t, "all", crate::i18n::tr("Every photo here that isn't in the library yet")),
    ];
    let seg = Rect::from_min_size(pos2(right - seg_w * 3.0, cy - H / 2.0), vec2(seg_w * 3.0, H));
    ui.painter().rect(seg, 5.0, t.button, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    for (i, (text, key, tip)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(seg.left() + seg_w * i as f32, seg.top()), vec2(seg_w, H));
        let resp = ui.interact(r, egui::Id::new(("import-take", *key)), Sense::click());
        let on = app.ui.import_settings.take == *key;
        crate::widgets::announce_selected(&resp, text.clone(), on);
        register(ui.ctx(), format!("button:importTake-{key}"), r);
        if on {
            ui.painter().rect_filled(r.shrink(2.0), 4.0, t.pressed);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), 4.0, t.hover);
        }
        if i > 0 {
            ui.painter().line_segment([pos2(r.left(), r.top() + 6.0), pos2(r.left(), r.bottom() - 6.0)], Stroke::new(1.0, t.button_border));
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, text, seg_font.clone(), if on { t.text } else { t.text_label });
        if resp.on_hover_text(*tip).clicked() {
            app.ui.import_settings.take = (*key).to_string();
        }
    }
    right = seg.left() - gap * 1.5;

    // Culling saved, small (just its tick when the row is tight)
    let sw = if saved_full_on { saved_full } else { 14.0 };
    let sr = Rect::from_min_size(pos2(right - sw, cy - 9.0), vec2(sw, 18.0));
    paint(ui.painter(), Rect::from_min_size(sr.left_top() + vec2(0.0, 2.0), vec2(14.0, 14.0)), Icon::Check, saved_colour);
    if saved_full_on {
        ui.painter().text(pos2(sr.left() + 18.0, cy), Align2::LEFT_CENTER, &saved_text, small, saved_colour);
    }
    let about = crate::i18n::tr(
        "Flags, stars and labels on these photos are kept in the library at once, imported or not, and come back whenever you browse this folder again.",
    );
    ui.interact(sr, egui::Id::new("import-culling-saved"), Sense::hover()).on_hover_text(format!(
        "{saved_text}
{about}"
    ));
    register(ui.ctx(), "label:cullingSaved", sr);
    right = sr.left() - gap * 1.5;

    // Include subfolders: an icon toggle of the bar's height
    let toggle = Rect::from_min_size(pos2(right - H, cy - H / 2.0), vec2(H, H));
    let resp = ui.interact(toggle, egui::Id::new("import-subfolders"), Sense::click());
    crate::widgets::announce_selected(&resp, crate::i18n::tr("Include subfolders").to_string(), b.subfolders);
    register(ui.ctx(), "check:includeSubfolders", toggle);
    let fill = if b.subfolders {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        t.button
    };
    ui.painter().rect(toggle, 5.0, fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    paint(ui.painter(), toggle.shrink(7.0), Icon::Stack, if b.subfolders { t.text } else { t.icon });
    let tip = if b.subfolders { "Include subfolders: on" } else { "Include subfolders: off" };
    if resp.on_hover_text(crate::i18n::tr(tip)).clicked() {
        let _ = app.run("library.browse", json!({"path": b.path, "subfolders": !b.subfolders}));
    }
    right = toggle.left() - gap * 1.5;

    // the IMPORT badge, then the folder
    let g = ui.painter().layout_no_wrap(badge.to_string(), t.semibold(10.5), t.canvas);
    let badge_r = Rect::from_min_size(pos2(inner.left(), cy - 10.0), vec2(g.size().x + 12.0, 20.0));
    ui.painter().rect_filled(badge_r, 4.0, t.accent);
    ui.painter().galley(badge_r.center() - g.size() / 2.0, g, t.canvas);
    register(ui.ctx(), "badge:import", badge_r);
    let folder = std::path::Path::new(&b.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| b.path.clone());
    let where_tip = crate::i18n::tr_format!("Browsing {folder}: not imported", folder = folder);
    ui.interact(badge_r, egui::Id::new("import-badge"), Sense::hover()).on_hover_text(&where_tip);
    let left = badge_r.right() + 10.0;
    let room = (right - left).max(0.0);
    let crumb_font = t.font(13.0);
    let last_font = t.semibold(13.0);
    let widths: Vec<f32> =
        crumbs.iter().enumerate().map(|(i, (name, _))| measure(name, if i + 1 == crumbs.len() { &last_font } else { &crumb_font }) + 4.0).collect();
    let sep_w = 16.0;
    let dots_w = measure("…", &crumb_font) + 4.0;
    let mut x = left;
    let shown = compact_crumbs(&widths, sep_w, dots_w, room);
    // the folders "…" stands for: a click lists them, each one opens
    let hidden: Vec<usize> = (0..crumbs.len()).filter(|i| !shown.contains(&Some(*i))).collect();
    for (k, part) in shown.iter().enumerate() {
        if k > 0 {
            paint(ui.painter(), Rect::from_center_size(pos2(x + sep_w / 2.0, cy), vec2(12.0, 12.0)), Icon::ChevronRight, t.text_dim);
            x += sep_w;
        }
        match part.and_then(|i| crumbs.get(i).map(|c| (i, c))) {
            None => {
                let r = Rect::from_min_size(pos2(x, cy - 10.0), vec2(dots_w, 20.0));
                let resp = ui.interact(r, egui::Id::new("import-crumb-dots"), Sense::click());
                register(ui.ctx(), "crumb:more", r);
                if resp.hovered() {
                    ui.painter().rect_filled(r, 3.0, t.hover);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "…", crumb_font.clone(), t.text_label);
                let resp = resp.on_hover_text(&b.path);
                egui::Popup::menu(&resp).show(|ui| {
                    for i in &hidden {
                        if let Some((name, to)) = crumbs.get(*i) {
                            let item = ui.button(*name);
                            register(ui.ctx(), format!("crumb:{i}"), item.rect);
                            if item.on_hover_text(to).clicked() {
                                let _ = app.run("library.browse", json!({"path": to}));
                                ui.close();
                            }
                        }
                    }
                });
                x += dots_w;
            }
            Some((i, (name, to))) => {
                let last = i + 1 == crumbs.len();
                let ww = widths.get(i).copied().unwrap_or(40.0).min((left + room - x).max(0.0));
                let r = Rect::from_min_size(pos2(x, cy - 10.0), vec2(ww, 20.0));
                let resp = ui.interact(r, egui::Id::new(("import-crumb", i)), Sense::click());
                register(ui.ctx(), format!("crumb:{i}"), r);
                let f = if last { last_font.clone() } else { crumb_font.clone() };
                let text = elide_end(name, (ww - 4.0).max(0.0), |s| measure(s, &f));
                let colour = if last || resp.hovered() { t.text } else { t.text_label };
                ui.painter().with_clip_rect(r).text(pos2(r.left() + 2.0, cy), Align2::LEFT_CENTER, text, f, colour);
                let resp = resp.on_hover_text(if last { where_tip.clone() } else { to.clone() });
                if !last {
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        let _ = app.run("library.browse", json!({"path": to}));
                    }
                }
                x += ww;
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breadcrumb_keeps_the_last_folder_and_collapses_the_middle() {
        let w = [20.0, 120.0, 140.0, 60.0];
        assert_eq!(compact_crumbs(&w, 16.0, 12.0, 1000.0), vec![Some(0), Some(1), Some(2), Some(3)], "all fit");
        // the drive, "…", then as many of the last as fit
        assert_eq!(compact_crumbs(&w, 16.0, 12.0, 300.0), vec![Some(0), None, Some(2), Some(3)]);
        assert_eq!(compact_crumbs(&w, 16.0, 12.0, 150.0), vec![Some(0), None, Some(3)]);
        // no room even for that: "…" and the last folder (cut)
        assert_eq!(compact_crumbs(&w, 16.0, 12.0, 40.0), vec![None, Some(3)]);
        assert!(compact_crumbs(&[], 16.0, 12.0, 100.0).is_empty());
        assert_eq!(compact_crumbs(&[500.0], 16.0, 12.0, 100.0), vec![Some(0)]);
    }

    #[test]
    fn elided_text_ends_with_an_ellipsis_within_the_room() {
        let width = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(elide_end("Wedding", 100.0, width), "Wedding");
        assert_eq!(elide_end("2026-09-10 Library", 60.0, width), "2026-…");
        assert_eq!(settings_width(f32::NAN), 120.0);
        assert_eq!(settings_width(268.0), 224.0);
    }
}
