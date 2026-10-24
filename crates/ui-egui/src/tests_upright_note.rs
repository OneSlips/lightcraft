//! Upright that finds nothing to correct says why, whatever ran it, under the photo rather than over it (test
//! notes 2026-10-24 notes 4 and 15).

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::ViewMode;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

#[test]
fn upright_says_why_under_the_photo_from_any_path() {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    // the demo's natural scenes have no verticals: find one Vertical refuses
    let ids = h.app.session.visible_cloned();
    let mut found = None;
    for id in ids.iter().take(6) {
        h.request("engine.execute", json!({"command": "library.select", "params": {"ids": [id.0], "active": id.0}}), T);
        h.app.ui.view = ViewMode::Detail;
        // as a console key runs it: the command alone, through the app's dispatch
        let r = h.app.run("geometry.upright", json!({"mode": "vertical"})).unwrap();
        if r["note"].is_string() {
            found = Some((*id, r["note"].as_str().unwrap_or_default().to_string()));
            break;
        }
    }
    let (id, note) = found.expect("a demo photo without verticals");
    assert!(!note.contains("Guided"), "brief, no routine push to Guided: {note}");
    let shown = h.app.ui.photo_note.clone().expect("the note is shown");
    assert_eq!(shown.photo, id.0);
    assert!(h.app.ui.toast.is_none(), "not as a toast over the photo");
    // drawn in the shot info strip's place, under the photo
    let widget = |h: &Headless, name: &str| h.app.widgets.iter().find(|(w, _)| w == name).map(|(_, r)| *r);
    h.step_until(Duration::from_secs(60), |h| h.app.widgets.iter().any(|(w, _)| w == "canvas:image"));
    assert!(h.app.ui.photo_note.is_some(), "still shown while the photo comes up");
    let strip = widget(&h, "loupe:shotInfo").expect("the strip is drawn");
    let img = widget(&h, "canvas:image").expect("the photo is drawn");
    assert!(strip.top() >= img.bottom() - 1.0, "under the photo: {strip:?} vs {img:?}");
    // a mode that finds something clears it
    h.app.run("geometry.upright", json!({"mode": "off"})).unwrap();
    assert!(h.app.ui.photo_note.is_none());
}
