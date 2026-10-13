//! LR-PROF-CREATE / LR-PROF-DUAL in the UI: Photo > Create Camera Profile from Color Chart… on two
//! chart photos under different lights (procedural DNGs, no photographs): both are read with a row
//! in the activity stack, their charts are found and drawn, and Create Profile files a
//! dual-illuminant profile and shows each light's report.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(60);

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

#[test]
fn two_chart_photos_make_a_dual_illuminant_profile_in_the_dialog() {
    let dir = std::env::temp_dir().join(format!("lc-ui-chartprof-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let corners = lightcraft_engine::chart_profiling::camprof::ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0);
    let mut paths = Vec::new();
    for light in ["A", "D65"] {
        let p = dir.join(format!("CHART_{light}.dng"));
        std::fs::write(&p, lightcraft_engine::chart_profiling::demo_chart_dng(360, 240, &corners, light, true).unwrap()).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let app = LightcraftApp::new(lightcraft_engine::Session::new(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    let r = h.app.session.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<u64> = r["imported"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
    assert_eq!(ids.len(), 2, "{r}");
    h.app.session.execute("library.select", &json!({"ids": ids, "active": ids[0]})).unwrap();

    crate::menus::run_ui_command(&mut h.app, "dialog.cameraProfile", &json!({})).unwrap().unwrap();
    assert!(matches!(h.app.ui.dialog, Some(Dialog::ChartProfile { .. })));
    assert!(h.app.session.activity.list().iter().any(|t| t.kind == "cameraProfile"), "reading the photos shows in the activity stack");
    assert!(h.step_until(T, |h| h.app.chart_profile.shots.iter().all(|s| s.as_ref().is_some_and(|s| s.prepared.is_some()))));
    h.settle(T);
    let Some(Dialog::ChartProfile { opts }) = h.app.ui.dialog.clone() else { panic!("the dialog stays open") };
    assert!(opts.corners.iter().all(Option::is_some), "both charts found: {opts:?}");
    assert!(has(&h, "chartProfile.picture") && has(&h, "chartProfile.corner0"), "the photo and its corners are drawn");
    assert!(h.app.session.activity.list().is_empty(), "the rows are gone");

    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:dialogOk"}), T)["ok"], true);
    assert!(h.step_until(T, |h| h.app.chart_profile.report.is_some() || h.app.chart_profile.error.is_some()));
    assert_eq!(h.app.chart_profile.error, None);
    let report = h.app.chart_profile.report.clone().unwrap();
    assert_eq!(report["dual"], true, "{report}");
    let cals = report["calibrations"].as_array().unwrap();
    assert_eq!((cals[0]["illuminant"].as_str(), cals[1]["illuminant"].as_str()), (Some("A"), Some("D65")));
    assert_eq!(h.app.session.dcp_profiles.len(), 1);
    let pid = report["id"].as_str().unwrap();
    for id in &ids {
        assert_eq!(h.app.session.catalog.photo(lightcraft_catalog::PhotoId(*id)).unwrap().develop.calibration_id(), Some(pid), "both chart photos use it");
    }
    h.settle(T);
    assert!(matches!(h.app.ui.dialog, Some(Dialog::ChartProfile { .. })), "the dialog stays to show the report");

    // back to one light; patch A01 turned a corner further
    crate::menus::run_ui_command(&mut h.app, "cameraProfile.removeSecond", &json!({})).unwrap().unwrap();
    crate::menus::run_ui_command(&mut h.app, "cameraProfile.turn", &json!({})).unwrap().unwrap();
    let Some(Dialog::ChartProfile { opts }) = h.app.ui.dialog.clone() else { panic!() };
    assert_eq!(opts.turns[0], 1);
    let _ = std::fs::remove_dir_all(&dir);
}
