//! The Import and Library workspace tabs: browsing a folder is the Import tab (its sources on the
//! left, its settings on the right, the bar with the big Import button over the grid and the
//! loupe); the big button imports the picks into the library, switches to the Library tab and
//! queues the finishing; no delete for browsed photos.

use std::time::Duration;

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::headless::Headless;
use crate::import_tab::Tab;
use crate::state::{RightPanel, ViewMode};

const T: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_secs(120);

fn app() -> Headless {
    let services = crate::Services { png: None, ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = ViewMode::PhotoGrid;
    Headless::new(app, [1500.0, 900.0], 1.0)
}

fn exec(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command} {params}: {r}");
    h.settle(SETTLE);
    r
}

fn run(h: &mut Headless, command: &str, params: Value) -> Result<Value, String> {
    let r = h.app.run(command, params);
    h.settle(SETTLE);
    r
}

fn folder(name: &str, n: u8) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-ui-importtab-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let o = lightcraft_engine::export::ExportOptions { format: lightcraft_engine::export::ExportFormat::Png, ..Default::default() };
    for i in 0..n {
        let img = lightcraft_raster::Rgba8::from_fn(24, 16, |x, y| [(x * 9) as u8, (y * 12) as u8, 40 * i, 255]);
        std::fs::write(dir.join(format!("p{i}.png")), lightcraft_engine::export::encode_image(&img, &o).unwrap()).unwrap();
    }
    dir
}

fn widget(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

#[test]
fn the_tabs_follow_browsing_and_the_library() {
    let mut h = app();
    h.settle(SETTLE);
    assert_eq!(h.app.ui.tab, Tab::Library);
    assert!(widget(&h, "tab:import") && widget(&h, "tab:library"), "both tabs at the top");
    // the Import tab with nothing chosen yet: the sources, and a way to choose
    run(&mut h, "view.importTab", json!({})).unwrap();
    assert_eq!(h.app.ui.tab, Tab::Import);
    assert!(h.app.ui.left_panel, "the sources are shown");
    assert_eq!(h.app.ui.right, RightPanel::Import);
    assert!(widget(&h, "panel:importSettings"), "the settings on the right");
    // browsing a folder: the bar with the big button over the grid
    let dir = folder("tabs", 3);
    run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
    assert_eq!(h.app.ui.tab, Tab::Import);
    assert!(widget(&h, "button:importNow") && widget(&h, "label:cullingSaved"), "{:?}", h.app.widgets.iter().map(|w| &w.0).collect::<Vec<_>>());
    // …and over the loupe
    h.request("ui.set", json!({"view": "detail"}), T);
    h.settle(SETTLE);
    assert!(widget(&h, "button:importNow"), "the Import button is there in the loupe too");
    // a library view is the Library tab
    exec(&mut h, "library.source", json!({"kind": "all"}));
    assert_eq!(h.app.ui.tab, Tab::Library);
    assert_ne!(h.app.ui.right, RightPanel::Import);
    // and back: the folder browsed last
    run(&mut h, "view.importTab", json!({})).unwrap();
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Folder);
    run(&mut h, "view.libraryTab", json!({})).unwrap();
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::All, "back to the view it had");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_big_button_imports_the_picks_and_shows_the_library() {
    let mut h = app();
    h.settle(SETTLE);
    let dir = folder("button", 3);
    run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
    let vis = h.app.session.visible_cloned();
    assert_eq!(vis.len(), 3);
    // no picks yet: the button says what to do
    let e = run(&mut h, "import.start", json!({})).unwrap_err();
    assert!(e.contains("P first"), "{e}");
    exec(&mut h, "photo.flag", json!({"ids": [vis[1].0], "flag": "pick"}));
    h.app.ui.import_settings.previews = "none".into();
    let r = run(&mut h, "import.start", json!({})).unwrap();
    assert_eq!(r["importing"], 1, "{r}");
    assert_eq!(h.app.ui.tab, Tab::Library, "the Library tab, where the photos come in");
    for _ in 0..200 {
        if h.app.import.is_none() {
            break;
        }
        h.settle(Duration::from_millis(200));
    }
    assert!(h.app.import.is_none(), "the import finished");
    let s = &mut h.app.session;
    assert!(!s.catalog.photo(vis[1]).unwrap().local, "the pick is in the library");
    assert!(s.catalog.photo(vis[0]).unwrap().local && s.catalog.photo(vis[2]).unwrap().local, "the others are not");
    assert!(s.visible_cloned().contains(&vis[1]), "and on show");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_does_nothing_to_browsed_photos() {
    let mut h = app();
    h.settle(SETTLE);
    h.app.ui.settings.confirm_delete = true;
    let dir = folder("nodelete", 2);
    run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
    let vis = h.app.session.visible_cloned();
    exec(&mut h, "library.select", json!({"ids": [vis[0].0]}));
    // the Delete key: no confirmation dialog, nothing deleted, the reason told
    assert!(!crate::menus::confirm_delete(&mut h.app), "no dialog for browsed photos");
    assert!(run(&mut h, "photo.delete", json!({})).unwrap_err().contains("reject"));
    assert!(!h.app.session.catalog.photo(vis[0]).unwrap().deleted);
    // Delete Rejected Files needs rejected photos, then the typed count
    assert!(run(&mut h, "dialog.deleteRejected", json!({})).is_err());
    exec(&mut h, "photo.flag", json!({"ids": [vis[0].0], "flag": "reject"}));
    run(&mut h, "dialog.deleteRejected", json!({})).unwrap();
    assert_eq!(h.app.ui.dialog, Some(crate::state::Dialog::DeleteRejected { count: 1, typed: String::new() }));
    assert!(dir.join("p0.png").exists(), "nothing happens before the count is typed");
    let _ = std::fs::remove_dir_all(&dir);
}

fn rect(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

/// The bar over the grid is one row: the badge, the breadcrumb, Include subfolders, Culling saved,
/// Picks / Selected / not imported and the Import button share one line, the buttons one height.
#[test]
fn the_browse_bar_is_one_row_of_one_height() {
    let mut h = app();
    h.settle(SETTLE);
    let dir = folder("bar", 3);
    run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
    let bar = rect(&h, "grid:folderHeader");
    assert!(bar.height() <= 46.0, "one row: {bar:?}");
    let ids =
        ["badge:import", "check:includeSubfolders", "label:cullingSaved", "button:importTake-picks", "button:importTake-all", "button:importNow"];
    for id in ids {
        let r = rect(&h, id);
        assert!((r.center().y - bar.center().y).abs() < 1.0, "{id} is on the bar's line: {r:?} in {bar:?}");
        assert!(bar.contains_rect(r), "{id} inside the bar");
    }
    let hs: Vec<f32> =
        ["check:includeSubfolders", "button:importTake-picks", "button:importTake-selected", "button:importTake-all", "button:importNow"]
            .iter()
            .map(|id| rect(&h, id).height())
            .collect();
    assert!(hs.iter().all(|x| (x - hs[0]).abs() < 0.5), "one height: {hs:?}");
    let seg: Vec<f32> =
        ["button:importTake-picks", "button:importTake-selected", "button:importTake-all"].iter().map(|id| rect(&h, id).width()).collect();
    assert!(seg.iter().all(|x| (x - seg[0]).abs() < 0.5), "one segment size: {seg:?}");
    // the last folder is always in the breadcrumb
    let parts = crate::panels::grid::crumbs(&dir.to_string_lossy()).len();
    assert!(h.app.widgets.iter().any(|(w, _)| *w == format!("crumb:{}", parts - 1)));
    // nothing to import: the button is off (and says why on hover)
    assert!(h.app.ui.import_settings.take == "picks");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The settings panel never cuts anything off: at a narrow width every control ends inside it.
#[test]
fn the_import_settings_fit_a_narrow_panel() {
    for width in [240.0, 300.0] {
        let mut h = app();
        h.settle(SETTLE);
        let dir = folder("narrow", 2);
        run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
        run(
            &mut h,
            "import.configure",
            json!({"copy": true, "organize": "custom", "folderTemplate": "{date:%Y}/{date:%Y%m%d}", "presets": ["lc.warm-glow"]}),
        )
        .unwrap();
        h.request("ui.set", json!({"rightWidth": width}), T);
        h.settle(SETTLE);
        h.settle(SETTLE);
        let panel = rect(&h, "panel:importSettings");
        let inside: Vec<(String, egui::Rect)> = h
            .app
            .widgets
            .iter()
            .filter(|(w, _)| {
                [
                    "button:importMode-",
                    "button:importPreviews-",
                    "button:importPresetAction-",
                    "button:importDest-",
                    "combo:import",
                    "radio:browse",
                    "field:import",
                    "label:importPreset-",
                ]
                .iter()
                .any(|p| w.starts_with(p))
            })
            .cloned()
            .collect();
        assert!(
            inside.iter().any(|(w, _)| w == "button:importMode-move") && inside.iter().any(|(w, _)| w == "button:importPreviews-full"),
            "{inside:?}"
        );
        for (id, r) in &inside {
            assert!(r.right() <= panel.right() + 0.5, "{width}: {id} ends at {} past the panel's {}", r.right(), panel.right());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Test notes 2026-10-25 note 3 (the user's decision): browsed photos show only in the Import
/// tab, as in Lightroom. A browsed folder the app reopens on with the Library tab saved (the view
/// at quit) opens in the Import tab, with its bar and Import button; the Library tab's views never
/// list a browsed photo. It used to reopen in the Library tab, where nothing said "not imported".
#[test]
fn a_browsed_folder_never_shows_in_the_library_tab() {
    let mut h = app();
    h.settle(SETTLE);
    let dir = folder("reopen", 3);
    run(&mut h, "library.browse", json!({"path": dir.to_string_lossy()})).unwrap();
    let browsed = h.app.session.visible_cloned();
    assert_eq!(browsed.len(), 3);
    exec(&mut h, "photo.flag", json!({"ids": [browsed[0].0], "flag": "pick"}));
    // the app reopens: the session is back on the folder, the saved UI on the Library tab
    h.app.ui.tab = Tab::Library;
    h.app.ui.right = RightPanel::Edit;
    h.settle(SETTLE);
    assert_eq!(h.app.ui.tab, Tab::Import, "a browsed folder is the Import tab");
    assert_eq!(h.app.ui.right, RightPanel::Import);
    assert!(widget(&h, "button:importNow"), "with the Import button");
    // a tool at work keeps its panel
    h.app.ui.tab = Tab::Library;
    h.app.ui.right = RightPanel::Crop;
    h.settle(SETTLE);
    assert_eq!(h.app.ui.tab, Tab::Import);
    assert_eq!(h.app.ui.right, RightPanel::Crop);
    // the Library tab's views: imported photos only
    run(&mut h, "view.libraryTab", json!({})).unwrap();
    assert_eq!(h.app.ui.tab, Tab::Library);
    for kind in ["all", "recentlyAdded", "picks"] {
        let _ = run(&mut h, "library.source", json!({"kind": kind}));
        assert_eq!(h.app.ui.tab, Tab::Library, "{kind}");
        let shown = h.app.session.visible_cloned();
        assert!(browsed.iter().all(|id| !shown.contains(id)), "{kind}: no browsed photo in the Library tab");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
