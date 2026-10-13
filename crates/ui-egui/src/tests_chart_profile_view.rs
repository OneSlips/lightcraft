//! The chart dialog's view and choices: zoom, pan and the magnifier; the suggested name following
//! the light and mode until edited; "Profile created ✓" until something changes; a name clash
//! answered with Keep Both; the library picker (charts first); the Edit panel's body hints.
//! Procedural DNGs only.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(60);

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn opts(h: &Headless) -> crate::chart_profile::ChartProfileDialog {
    match &h.app.ui.dialog {
        Some(Dialog::ChartProfile { opts }) => opts.clone(),
        _ => panic!("the chart dialog is open"),
    }
}

fn write(dir: &std::path::Path, name: &str, light: &str, serial: Option<&str>, chart: bool) -> String {
    let corners = if chart {
        lightcraft_engine::chart_profiling::camprof::ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0)
    } else {
        lightcraft_engine::chart_profiling::camprof::ChartCorners::from_rect(5000.0, 5000.0, 180.0, 120.0)
    };
    let p = dir.join(name);
    std::fs::write(&p, lightcraft_engine::chart_profiling::demo_chart_dng_of_body(360, 240, &corners, light, true, serial).unwrap()).unwrap();
    p.to_string_lossy().to_string()
}

fn session_with(paths: &[String]) -> (Headless, Vec<u64>) {
    let app = LightcraftApp::new(lightcraft_engine::Session::new().with_fs(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    let r = h.app.session.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<u64> = r["imported"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
    (h, ids)
}

fn open_on(h: &mut Headless, id: u64) {
    h.app.session.execute("library.select", &json!({"ids": [id], "active": id})).unwrap();
    crate::menus::run_ui_command(&mut h.app, "dialog.cameraProfile", &json!({})).unwrap().unwrap();
    assert!(h.step_until(T, |h| h.app.chart_profile.shots[0].as_ref().is_some_and(|s| s.prepared.is_some())));
    h.settle(T);
}

#[test]
fn the_chart_view_zooms_pans_fits_and_magnifies_a_corner() {
    let dir = std::env::temp_dir().join(format!("lc-ui-chartview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, ids) = session_with(&[write(&dir, "C.dng", "D55", Some("SN71"), true)]);
    open_on(&mut h, ids[0]);
    // the developed photo, oriented: the texture is the display render
    let shot = h.app.chart_profile.shots[0].as_ref().unwrap();
    assert_eq!(shot.texture.as_ref().unwrap().size(), [720, 480], "developed at the photo's own size");
    assert_eq!(h.app.chart_profile.views[0].zoom, 1.0);
    // the wheel zooms about the pointer
    assert_eq!(h.request("ui.hoverWidget", json!({"id": "chartProfile.picture", "fx": 0.3, "fy": 0.4}), T)["ok"], true);
    assert_eq!(h.request("ui.scroll", json!({"dx": 0, "dy": 300}), T)["ok"], true);
    h.settle(T);
    let zoomed = h.app.chart_profile.views[0];
    assert!(zoomed.zoom > 1.5, "{zoomed:?}");
    // a right-drag pans
    assert_eq!(h.request("ui.dragWidget", json!({"id": "chartProfile.picture", "dx": 80, "dy": 0, "button": "right"}), T)["ok"], true);
    h.settle(T);
    assert_ne!(h.app.chart_profile.views[0].center, zoomed.center, "moved");
    // double-click fits; 100 % is one pixel per pixel
    assert_eq!(h.request("ui.clickWidget", json!({"id": "chartProfile.picture", "count": 2}), T)["ok"], true);
    h.settle(T);
    assert_eq!(h.app.chart_profile.views[0].zoom, 1.0);
    let r = crate::menus::run_ui_command(&mut h.app, "cameraProfile.zoom", &json!({"mode": "actual"})).unwrap().unwrap();
    assert!(r["zoom"].as_f64().unwrap() > 0.9, "{r}");
    crate::menus::run_ui_command(&mut h.app, "cameraProfile.zoom", &json!({"mode": "fit"})).unwrap().unwrap();
    // resting on a corner shows the magnifier
    h.settle(T);
    assert!(!has(&h, "chartProfile.magnifier"));
    assert_eq!(h.request("ui.hoverWidget", json!({"id": "chartProfile.corner2"}), T)["ok"], true);
    h.settle(T);
    assert!(has(&h, "chartProfile.magnifier"), "a magnifier at the corner");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_name_follows_light_and_mode_until_typed_and_a_clash_is_asked() {
    let dir = std::env::temp_dir().join(format!("lc-ui-chartname-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, ids) = session_with(&[write(&dir, "C.dng", "D55", Some("ABC71"), true)]);
    open_on(&mut h, ids[0]);
    let name = opts(&h).name;
    assert!(name.starts_with("LightCraft Chart Camera 71 · Daylight · Photo · "), "{name}");
    assert_eq!(h.request("ui.clickWidget", json!({"id": "radio:chartProfileRendering-reproduction"}), T)["ok"], true);
    h.settle(T);
    assert!(opts(&h).name.contains("· Repro ·"), "{}", opts(&h).name);
    // build: then "Profile created ✓" until something changes
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:dialogOk"}), T)["ok"], true);
    assert!(h.step_until(T, |h| h.app.chart_profile.report.is_some() || h.app.chart_profile.error.is_some()));
    assert_eq!(h.app.chart_profile.error, None);
    h.settle(T);
    let o = opts(&h);
    assert!(crate::chart_profile::is_built(&h.app, &o) && !crate::chart_profile::can_build(&h.app, &o));
    assert_eq!(h.app.session.dcp_profiles.len(), 1);
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:dialogOk"}), T)["ok"], true);
    h.settle(T);
    assert_eq!(h.app.session.dcp_profiles.len(), 1, "a second click makes nothing");
    // the same name again (after a change and back): asked, and Keep Both saves "… 2"
    h.app.chart_profile.built = None;
    let o = opts(&h);
    crate::chart_profile::start_build(&mut h.app, &o).unwrap();
    assert!(h.app.chart_profile.clash.is_some(), "a profile of that name is there");
    h.settle(T);
    assert!(has(&h, "button:chartProfileReplace") && has(&h, "button:chartProfileKeepBoth"));
    crate::menus::run_ui_command(&mut h.app, "cameraProfile.resolve", &json!({"choice": "keepBoth"})).unwrap().unwrap();
    assert!(h.step_until(T, |h| h.app.session.dcp_profiles.len() == 2));
    assert!(h.app.session.dcp_profiles[1].name.ends_with(" 2"), "{}", h.app.session.dcp_profiles[1].name);
    // typed: no longer follows the mode
    if let Some(Dialog::ChartProfile { opts }) = &mut h.app.ui.dialog {
        opts.name = "My Studio".into();
        opts.name_edited = true;
    }
    assert_eq!(h.request("ui.clickWidget", json!({"id": "radio:chartProfileRendering-photographic"}), T)["ok"], true);
    h.settle(T);
    assert_eq!(opts(&h).name, "My Studio");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_second_chart_picker_lists_charts_first_and_the_edit_panel_hints_bodies() {
    let dir = std::env::temp_dir().join(format!("lc-ui-chartpick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = [
        write(&dir, "FIRST.dng", "D65", Some("SN71"), true),
        write(&dir, "NOCHART.dng", "A", Some("SN71"), false),
        write(&dir, "TUNGSTEN.dng", "A", Some("SN71"), true),
    ];
    let (mut h, ids) = session_with(&paths);
    open_on(&mut h, ids[0]);
    let r = crate::menus::run_ui_command(&mut h.app, "cameraProfile.pickSecond", &json!({})).unwrap().unwrap();
    assert_eq!(r["candidates"], 2, "{r}");
    assert!(h.app.session.activity.list().iter().any(|t| t.kind == "cameraProfile"), "the chart search shows in the activity stack");
    assert!(h.step_until(T, |h| h.app.chart_profile.scans.len() == 2));
    let list = crate::chart_profile::candidates(&h.app, false, "", Some("D65"));
    let names: Vec<&str> = list.iter().map(|c| c.file_name.as_str()).collect();
    assert_eq!(names, ["TUNGSTEN.dng", "NOCHART.dng"], "charts first");
    h.settle(T);
    assert!(has(&h, &format!("chartProfilePick:{}", ids[2])));
    // picking fills the second slot
    let w = h.request("ui.widgets", json!({"filter": "chartProfilePick"}), T);
    let r = h.request("ui.clickWidget", json!({"id": format!("chartProfilePick:{}", ids[2])}), T);
    assert_eq!(r["ok"], true, "{r} {w}");
    for _ in 0..5 {
        h.step();
    }
    assert!(h.app.chart_profile.shots[1].is_some(), "clicked {r} {w}");
    h.step_until(Duration::from_secs(20), |h| h.app.chart_profile.shots[1].as_ref().is_some_and(|s| s.prepared.is_some() || s.error.is_some()));
    let st = h.app.chart_profile.shots[1].as_ref().map(|s| (s.path.clone(), s.prepared.is_some(), s.error.clone()));
    assert!(st.as_ref().is_some_and(|s| s.1), "{st:?} picker open: {} err {:?}", h.app.chart_profile.picker.is_some(), h.app.chart_profile.error);
    assert!(h.app.chart_profile.picker.is_none());
    // the Edit panel: a profile made on body 71 applied to a photo of body 74 offers adoption
    let p74 = write(&dir, "P74.dng", "D55", Some("SN74"), true);
    h.app.ui.dialog = None;
    let made = h.app.session.execute("profile.createFromChart", &json!({"path": paths[0], "illuminant": "D65", "apply": false})).unwrap();
    let r = h.app.session.execute("library.import", &json!({"paths": [p74]})).unwrap();
    let id74 = r["imported"][0].as_u64().unwrap();
    h.app.session.execute("library.select", &json!({"ids": [id74], "active": id74})).unwrap();
    let (mine, _) = lightcraft_engine::cmd::dcp_profiles::menu_for_photo(&h.app.session, Some(lightcraft_catalog::PhotoId(id74)), false);
    assert!(mine.is_empty(), "another body's profile isn't in the default view");
    let (_, theirs) = lightcraft_engine::cmd::dcp_profiles::menu_for_photo(&h.app.session, Some(lightcraft_catalog::PhotoId(id74)), true);
    assert_eq!(theirs.len(), 1, "Browse Other Camera Profiles reveals it");
    assert_eq!(theirs[0].2, "…71");
    h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    h.settle(T);
    assert!(!has(&h, "button:adoptForBody"), "not without another body's profile");
    h.app.session.execute("develop.cameraProfile", &json!({"id": made["id"]})).unwrap();
    h.settle(T);
    assert!(has(&h, "button:adoptForBody"), "a profile from another body: Adopt for This Body");
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:adoptForBody"}), T)["ok"], true);
    h.settle(T);
    assert_eq!(h.app.session.dcp_profiles[0].bodies, ["SN71", "SN74"]);
    h.settle(T);
    assert!(!has(&h, "button:adoptForBody"), "its own now");
    let _ = std::fs::remove_dir_all(&dir);
}
