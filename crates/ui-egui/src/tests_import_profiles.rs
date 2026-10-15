//! LR-PROF-IMPORT-APPLY in the import review: a card with two bodies (one known by its serial,
//! one by its file names) and a Lightroom sidecar shows a row per body; a per-body choice with
//! "Adopt for this body" imports with it, and the toast says which profile went where.
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

fn raw(dir: &std::path::Path, name: &str, light: &str, serial: Option<&str>) -> String {
    let corners = lightcraft_engine::chart_profiling::camprof::ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0);
    let p = dir.join(name);
    std::fs::write(&p, lightcraft_engine::chart_profiling::demo_chart_dng_of_body(360, 240, &corners, light, true, serial).unwrap()).unwrap();
    p.to_string_lossy().to_string()
}

fn review(h: &mut Headless) -> &mut crate::import::ImportDialog {
    match &mut h.app.ui.dialog {
        Some(Dialog::Import { opts }) => opts,
        other => panic!("the import review is open: {other:?}"),
    }
}

#[test]
fn the_review_shows_each_body_and_imports_with_the_choices() {
    let dir = std::env::temp_dir().join(format!("lc-ui-importprof-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    let mut session = lightcraft_engine::Session::new().with_fs();
    let chart = raw(&dir, "M71.dng", "D55", Some("SN71"));
    let p71 = session.execute("profile.createFromChart", &json!({"path": chart, "illuminant": "D55", "apply": false})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    raw(&card, "A1.dng", "D65", Some("SN71"));
    raw(&card, "A2.dng", "D50", Some("SN71"));
    raw(&card, "_SA0001.dng", "A", None);
    let lr = raw(&card, "_SA0002.dng", "D75", None);
    std::fs::write(
        std::path::Path::new(&lr).with_extension("xmp"),
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+0.50"/></rdf:RDF></x:xmpmeta>"#,
    )
    .unwrap();
    let app = LightcraftApp::new(session, Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    crate::import::open(&mut h.app, vec![card.to_string_lossy().to_string()]).unwrap();
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    h.settle(T);
    assert!(has(&h, "combo:importCameraProfile"));
    // a row per body: 71 (its own profile, automatically) and SA (none of its own)
    let rows = {
        let d = review(&mut h).clone();
        lightcraft_engine::import_profiles::plan(&h.app.session, &d.to_import(), &lightcraft_engine::import_profiles::ProfileChoice::Auto)
    };
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!((rows[0].body.as_deref(), rows[0].profile.as_deref(), rows[0].photos), (Some("SN71"), Some(p71.as_str()), 2));
    assert_eq!((rows[1].body.as_deref(), rows[1].profile.as_deref(), rows[1].from_sidecar), (Some("file:SA"), None, 1));
    assert!(has(&h, "label:importProfileRow-0") && has(&h, "label:importProfileRow-1"));
    assert!(has(&h, "button:importProfileChoose-1"));
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:importProfileChoose-1"}), T)["ok"], true, "Choose… opens the menu");
    h.settle(T);
    // SA takes 71's profile for this import, and adopts it
    review(&mut h).profile_per_body.insert("file:SA".into(), p71.clone());
    h.settle(T);
    assert!(has(&h, "check:importAdopt-1"), "another body's profile can be adopted");
    assert_eq!(h.request("ui.clickWidget", json!({"id": "check:importAdopt-1"}), T)["ok"], true);
    h.settle(T);
    assert_eq!(review(&mut h).adopt, ["file:SA"]);
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    assert!(h.step_until(T, |h| h.app.import.is_none() && h.app.session.catalog.len() == 4));
    // (read the timed toast as soon as the import is done: settling first could outlast it on a busy machine)
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    h.settle(T);
    let cal = |h: &Headless, name: &str| {
        let p = h.app.session.catalog.photos().find(|p| p.file_name == name).unwrap();
        p.develop.calibration_id().filter(|c| c.starts_with("dcp:")).unwrap_or("").to_string()
    };
    assert_eq!(cal(&h, "A1.dng"), p71);
    assert_eq!(cal(&h, "_SA0001.dng"), p71, "the per-body choice");
    assert_eq!(cal(&h, "_SA0002.dng"), "", "the sidecar's settings win");
    assert_eq!(h.app.session.dcp_profiles[0].bodies, ["SN71", "file:SA"], "adopted");
    assert!(toast.contains("Camera profiles:") && toast.contains("(3)"), "{toast}");
    assert!(toast.contains("1 kept from sidecar") && toast.contains("adopted for SA"), "{toast}");
    let _ = std::fs::remove_dir_all(&dir);
}
