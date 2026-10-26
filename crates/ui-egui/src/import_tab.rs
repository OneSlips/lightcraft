//! The two workspace tabs, Import and Library (the user's decision): **Import** browses a folder
//! or card without adding it (the Local view of a folder), for culling with the fast grid, loupe
//! and eye strip, with the sources on the left and the import settings on the right; **Library**
//! is the catalog, where everything imported is edited.
//!
//! The Import tab's settings ([`ImportSettings`]) are kept between sessions; one big button
//! imports the picks, the selection or all of the folder's photos not in the library yet
//! ([`start`]), switches to the Library tab where they come in, and once they are in a queue
//! finishes them unattended (masks, skin, previews: `library.finishImport`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;
use lightcraft_engine::LibrarySource;
pub use lightcraft_engine::cmd::import_tab::Take;

/// The workspace tab on show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    Import,
    #[default]
    Library,
}

/// The Import tab's settings: what the big button takes, and what the import does to the photos
/// (the import review's options, with the preset stack, kept between sessions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportSettings {
    /// `picks` | `selected` | `all`: what the big button imports.
    pub take: String,
    /// `none` | `standard` | `full`: previews built after the import.
    pub previews: String,
    /// The import preset these settings came from ("" = none).
    pub preset_name: String,
    /// Destination, renaming, album, camera profiles, look, presets, keywords (never candidates:
    /// the folder's photos are the Import tab's).
    pub opts: crate::import::ImportDialog,
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings { take: "picks".into(), previews: "standard".into(), preset_name: String::new(), opts: Default::default() }
    }
}

impl ImportSettings {
    pub fn take(&self) -> Take {
        Take::parse(&self.take).unwrap_or(Take::Picks)
    }

    /// The `library.import` parameters (without `paths`).
    pub fn params(&self) -> Result<Value, String> {
        self.opts.params()
    }

    /// These settings from an import preset's (unknown fields keep their defaults).
    pub fn from_preset(name: &str, v: &Value) -> ImportSettings {
        let opts: crate::import::ImportDialog = serde_json::from_value(v.clone()).unwrap_or_default();
        let previews = v.get("previews").and_then(Value::as_str).unwrap_or("standard").to_string();
        ImportSettings { take: "picks".into(), previews, preset_name: name.to_string(), opts: settings_only(opts) }
    }

    /// What an import preset keeps of these settings (not what to take, nor any photo).
    pub fn to_preset(&self) -> Value {
        let mut v = serde_json::to_value(settings_only(self.opts.clone())).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            for k in ["candidates", "checked", "trashed", "sources", "onDeleted"] {
                o.remove(k);
            }
            o.insert("previews".into(), json!(self.previews));
        }
        v
    }
}

/// The options of `d` without the files a review scanned.
pub fn settings_only(mut d: crate::import::ImportDialog) -> crate::import::ImportDialog {
    d.candidates.clear();
    d.checked.clear();
    d.trashed.clear();
    d.sources.clear();
    d
}

/// The open library's name, as the Import button names it ("Library" for one in memory).
pub fn library_name(app: &LightcraftApp) -> String {
    crate::panels::settings::open_library_name(app).unwrap_or_else(|| crate::i18n::tr("Library").to_string())
}

/// Whether the Import tab is on show.
pub fn importing(app: &LightcraftApp) -> bool {
    app.ui.tab == Tab::Import
}

/// Show a tab (`view.importTab` / `view.libraryTab`). Import shows the folder browsed last (or the
/// sources to choose one from), with its panels; Library goes back to the view it had.
pub fn switch(app: &mut LightcraftApp, tab: Tab) -> Result<Value, String> {
    // the user's own choice of tab
    app.ui.import_moved = false;
    if app.ui.tab == tab {
        return Ok(json!({"tab": tab}));
    }
    app.ui.tab = tab;
    match tab {
        Tab::Import => {
            if app.session.source != LibrarySource::Folder {
                app.ui.library_source = Some(app.session.source);
            }
            // the sources are where an import starts
            app.ui.left_panel = true;
            app.ui.right = crate::state::RightPanel::Import;
            if let Some(b) = app.session.browse.clone()
                && std::path::Path::new(&b.path).is_dir()
            {
                let _ = crate::import::browse(app, &b.path, Some(b.subfolders));
            }
        }
        Tab::Library => {
            if app.ui.right == crate::state::RightPanel::Import {
                app.ui.right = crate::state::RightPanel::Edit;
            }
            if app.session.source == LibrarySource::Folder {
                let back = app.ui.library_source.take().filter(|s| *s != LibrarySource::Folder).unwrap_or(LibrarySource::All);
                app.session.source = back;
                app.session.keep_active_in_view();
            }
        }
    }
    Ok(json!({"tab": tab}))
}

/// Browsed photos show only in the Import tab, as in Lightroom (test notes 2026-10-25 note 3, the
/// user's decision): a browsed folder on show in the Library tab, however it got there (the view
/// the app or a library reopens on, a command, an agent), moves to the Import tab, so the Library
/// tab only ever shows imported photos. Checked every frame.
/// A view that moved there by itself (Show Alternatives, a library reopening on its browsed
/// folder) goes back to the Library tab once it shows the library again.
pub fn keep_in_step(app: &mut LightcraftApp) {
    let browsing = app.session.source == LibrarySource::Folder;
    if browsing && app.ui.tab != Tab::Import {
        app.ui.tab = Tab::Import;
        app.ui.import_moved = true;
        app.ui.left_panel = true;
        // the import settings, unless a tool is at work on the photo
        if matches!(app.ui.right, crate::state::RightPanel::Edit | crate::state::RightPanel::None | crate::state::RightPanel::Profiles) {
            app.ui.right = crate::state::RightPanel::Import;
        }
    } else if !browsing && app.ui.import_moved {
        app.ui.import_moved = false;
        if app.ui.tab == Tab::Import {
            app.ui.tab = Tab::Library;
            if app.ui.right == crate::state::RightPanel::Import {
                app.ui.right = crate::state::RightPanel::Edit;
            }
        }
    }
}

/// Keep the tab in step with the view after a command: browsing a folder is the Import tab, any
/// library view (All Photos, an album…) the Library tab.
pub fn follow(app: &mut LightcraftApp, id: &str) {
    match id {
        "library.browse" if app.session.source == LibrarySource::Folder && app.ui.tab != Tab::Import => {
            app.ui.tab = Tab::Import;
            app.ui.left_panel = true;
            // the import settings, unless a tool is at work on the photo
            if matches!(app.ui.right, crate::state::RightPanel::Edit | crate::state::RightPanel::None | crate::state::RightPanel::Profiles) {
                app.ui.right = crate::state::RightPanel::Import;
            }
        }
        "library.source" if app.session.source != LibrarySource::Folder && app.ui.tab != Tab::Library => {
            app.ui.tab = Tab::Library;
            if app.ui.right == crate::state::RightPanel::Import {
                app.ui.right = crate::state::RightPanel::Edit;
            }
        }
        _ => {}
    }
    keep_in_step(app);
}

/// The big button: import what the settings take from the browsed folder, in the background,
/// then show the Library tab as the photos come in (`import.start`).
pub fn start(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.source != LibrarySource::Folder {
        return Err(crate::i18n::tr("Choose a folder or card to import from first").to_string());
    }
    crate::import::busy_synchronizing(app)?;
    if app.import.as_ref().is_some_and(|t| !t.browse()) || app.scan.as_ref().is_some_and(|t| !t.browse) {
        return Err(crate::i18n::tr("An import is running").to_string());
    }
    if let Some(take) = p.get("what").and_then(Value::as_str) {
        if Take::parse(take).is_none() {
            return Err(format!("unknown what `{take}` (picks|selected|all)"));
        }
        app.ui.import_settings.take = take.to_string();
    }
    let settings = app.ui.import_settings.clone();
    let take = settings.take();
    // a folder still being read: what is in so far is what there is
    let paths = app.session.import_paths(take);
    if paths.is_empty() {
        return Err(crate::i18n::tr(match take {
            Take::Picks => "No picks to import: flag the photos you want with P first",
            Take::Selected => "No photos selected to import",
            Take::All => "Every photo here is in the library already",
        })
        .to_string());
    }
    let mut params = settings.params()?;
    let previews = settings.previews.clone();
    // a new album named in the settings is made now (a later batch joins it)
    if settings.opts.album.is_none() && !settings.opts.new_album.trim().is_empty() {
        params["albumName"] = json!(settings.opts.new_album.trim());
    }
    // checked now, not halfway through: an unknown preset, profile or template refuses the import
    let mut check = params.clone();
    check["paths"] = json!(paths.first().cloned().into_iter().collect::<Vec<_>>());
    lightcraft_engine::cmd::library::import_params(&app.session, &check).map_err(|e| e.to_string())?;
    // a browse still reading the folder stops: its files are what is imported now
    if app.import.as_ref().is_some_and(crate::import::ImportTask::browse) {
        app.import = None;
    }
    if let Some(t) = app.scan.take() {
        t.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let n = paths.len();
    let undo0 = app.session.undo.len();
    let mut task = crate::import::ImportTask::new(paths, params, undo0, false);
    task.finish = Some(previews);
    app.import = Some(task);
    app.renderer.forget_imports();
    // the Library tab, where the photos come in (Recently Added shows them as they arrive)
    app.ui.tab = Tab::Library;
    app.ui.library_source = None;
    if app.ui.right == crate::state::RightPanel::Import {
        app.ui.right = crate::state::RightPanel::Edit;
    }
    app.session.source = LibrarySource::RecentlyAdded;
    app.session.refresh_view();
    Ok(json!({"importing": n, "into": library_name(app)}))
}

/// Apply an import preset to the Import tab's settings (`import.usePreset {name}`).
pub fn use_preset(app: &mut LightcraftApp, name: &str) -> Result<Value, String> {
    let Some(p) = app.session.import_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)).cloned() else {
        return Err(format!("no import preset `{name}`"));
    };
    let take = app.ui.import_settings.take.clone();
    app.ui.import_settings = ImportSettings::from_preset(&p.name, &p.settings);
    app.ui.import_settings.take = take;
    Ok(json!({"name": p.name}))
}

/// Change the Import tab's settings (`import.configure`): any of `take`, `previews`, and the
/// options of the import review (`copy`, `moveFiles`, `destination`, `organize`, `folderTemplate`,
/// `rename`, `presets`, `keywords`, `metadataPreset`, `look`, `profileMode`, `profileAll`,
/// `profilePerBody`, `album`, `newAlbum`), by name. Returns the settings.
pub fn configure(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let Some(given) = p.as_object() else { return Err("settings must be an object".into()) };
    let mut s = app.ui.import_settings.clone();
    let mut opts = serde_json::to_value(&s.opts).map_err(|e| e.to_string())?;
    for (k, v) in given {
        match k.as_str() {
            "take" => {
                let t = v.as_str().filter(|t| Take::parse(t).is_some()).ok_or("take: picks|selected|all")?;
                s.take = t.to_string();
            }
            "previews" => {
                let t = v.as_str().filter(|t| matches!(*t, "none" | "standard" | "full")).ok_or("previews: none|standard|full")?;
                s.previews = t.to_string();
            }
            "candidates" | "checked" | "trashed" | "sources" => return Err(format!("`{k}` is not a setting")),
            _ if opts.get(k).is_some() => opts[k] = v.clone(),
            _ => return Err(format!("unknown import setting `{k}`")),
        }
    }
    s.opts = serde_json::from_value(opts).map_err(|e| format!("import settings: {e}"))?;
    s.opts.params()?;
    s.preset_name.clear();
    app.ui.import_settings = s;
    serde_json::to_value(&app.ui.import_settings).map_err(|e| e.to_string())
}
