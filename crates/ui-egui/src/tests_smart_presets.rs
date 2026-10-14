//! Headless tests of smart presets in the app: hovering a preset with AI masks previews it without
//! changing the photo and restores the view exactly when the pointer leaves; clicking applies it
//! (twice: no doubling); the AI mark; the People picker and the Roles window open.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn app(panel: &str) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("engine.execute", json!({"command": panel}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h
}

#[test]
fn hovering_a_smart_preset_previews_it_and_leaving_restores_the_photo() {
    let mut h = app("panel.presets");
    let id = h.app.session.active().unwrap();
    let before = h.app.session.catalog.photo(id).unwrap().clone();
    let undo = h.app.session.undo.len();
    // scroll the list down to the Smart Masks group and rest on Bring Sky Down
    let r = h.request("ui.hoverWidget", json!({"id": "preset:lc.warm-glow"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let mut r = h.request("ui.hoverWidget", json!({"id": "preset:lc.smart.bringSkyDown"}), T);
    for _ in 0..60 {
        if r["ok"] == true {
            break;
        }
        h.request("ui.scroll", json!({"dy": -300}), T);
        h.step();
        r = h.request("ui.hoverWidget", json!({"id": "preset:lc.smart.bringSkyDown"}), T);
    }
    assert_eq!(r["ok"], true, "{r}");
    // (the list may still glide after the wheel: rest on it again once it is still)
    h.settle(SETTLE);
    let r = h.request("ui.hoverWidget", json!({"id": "preset:lc.smart.bringSkyDown"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h.step();
    let hover = h.app.hover_preview.clone().expect("a preview while resting on the preset");
    assert!(hover.label.contains("Bring Sky Down"), "{}", hover.label);
    // no model in tests: the mask isn't known, so the preview shows the preset without it rather
    // than a wrong approximation
    assert!(hover.settings.masks.is_empty());
    assert!(h.app.widgets.iter().any(|(w, _)| w == "presetAi:lc.smart.bringSkyDown"), "the AI mark");
    let after = h.app.session.catalog.photo(id).unwrap().clone();
    assert_eq!(after.develop, before.develop, "hovering changes nothing");
    assert_eq!(h.app.session.undo.len(), undo);
    // off the presets: the view is the photo's own again
    h.request("ui.move", json!({"x": 5, "y": 500}), T);
    h.settle(SETTLE);
    h.step();
    assert!(h.app.hover_preview.is_none());
    assert!(!h.app.session.preview_busy());
    // click: applied once; click again: replaced, not doubled
    for _ in 0..2 {
        let r = h.request("ui.clickWidget", json!({"id": "preset:lc.smart.bringSkyDown"}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.settle(SETTLE);
    }
    let d = h.app.session.develop_of(id).unwrap();
    assert_eq!(d.masks.iter().filter(|m| m.name == "Bring Sky Down").count(), 1, "{:?}", d.masks.iter().map(|m| &m.name).collect::<Vec<_>>());
    // one undo step back to the photo as it was
    let r = h.request("engine.execute", json!({"command": "edit.undo"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.session.develop_of(id).unwrap().masks.len(), before.develop.masks.len() + 1, "the first click's mask");
    let r = h.request("engine.execute", json!({"command": "edit.undo"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(*h.app.session.develop_of(id).unwrap(), *before.develop);
}

#[test]
fn the_people_picker_and_roles_window_open() {
    let mut h = app("panel.masking");
    let r = h.request("ui.clickWidget", json!({"id": "maskNew:people"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h.step();
    assert!(h.app.ui.people_pick.is_some());
    // without the face model: people counted from the left, and the Roles button
    assert!(h.app.widgets.iter().any(|(w, _)| w == "maskPeople:person:0"));
    assert!(h.app.widgets.iter().any(|(w, _)| w == "button:maskPeople:roles"));
    h.app.ui.roles_window = true;
    h.step();
    h.step();
    assert!(h.app.widgets.iter().any(|(w, _)| w == "rolesWindow:forgetAll"));
    // the Remembered tile lists what was taught (nothing yet) and offers Remember Selected Mask As…
    h.app.ui.roles_window = false;
    let r = h.request("ui.clickWidget", json!({"id": "maskNew:concept"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h.step();
    assert!(h.app.ui.concept_pick);
    assert!(h.app.widgets.iter().any(|(w, _)| w == "maskConcept:teach"));
}
