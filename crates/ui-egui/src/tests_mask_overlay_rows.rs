//! Which hover shows a mask's overlay (test notes 2026-10-31 note 7): the mask's own row (name,
//! eye, menu) shows none, so its eye can be used to judge the effect; a component row under it shows
//! the whole mask's overlay; the pin shows it (dragging the pin hides it, see
//! `tests_mask_workflow`); a mask without adjustments keeps its overlay whatever is hovered; O
//! still shows and hides it.

use std::time::Duration;

use lightcraft_pipeline::Overlay;
use serde_json::json;

use crate::headless::Headless;
use crate::panels::detail::view_overlay;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn masking() -> Headless {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    ask(&mut h, "ui.set", json!({"view": "detail"}));
    exec(&mut h, "panel.masking", json!({}));
    h
}

fn ask(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    // two frames: the hover of this frame and the last both count
    h.step();
    h.step();
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) {
    ask(h, "engine.execute", json!({"command": command, "params": params}));
}

fn shown(h: &Headless) -> bool {
    let id = h.app.session.active().expect("a photo");
    let d = h.app.session.develop_of(id).unwrap_or_default();
    matches!(view_overlay(&h.app, &d), Overlay::Mask { .. })
}

fn visible(h: &Headless, mid: u32) -> bool {
    let id = h.app.session.active().expect("a photo");
    h.app.session.develop_of(id).unwrap_or_default().masks.iter().find(|m| m.id == mid).expect("the mask").visible
}

/// A radial mask at the middle, given an adjustment (so it doesn't show by itself).
fn adjusted_radial(h: &mut Headless) -> u32 {
    exec(h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    exec(h, "mask.adjust", json!({"values": {"exposure": 1.0}}));
    h.app.session.active_mask.expect("the new mask is selected")
}

/// The pointer somewhere on the canvas away from every pin and row.
fn away(h: &mut Headless) {
    let c = h.app.canvas_rect.expect("the loupe");
    ask(h, "ui.move", json!({"x": c.left() + 6.0, "y": c.top() + 6.0}));
}

#[test]
fn the_mask_row_shows_no_overlay_and_its_eye_judges_the_effect() {
    let mut h = masking();
    let mid = adjusted_radial(&mut h);
    away(&mut h);
    assert!(!shown(&h), "adjusted: no overlay by itself");
    ask(&mut h, "ui.hoverWidget", json!({"id": format!("mask:{mid}")}));
    assert!(!shown(&h), "the pointer on the mask's own row shows no overlay");
    ask(&mut h, "ui.hoverWidget", json!({"id": format!("maskVisible:{mid}")}));
    assert!(!shown(&h), "nor on its eye");
    ask(&mut h, "ui.clickWidget", json!({"id": format!("maskVisible:{mid}")}));
    assert!(!visible(&h, mid), "the eye switched the effect off");
    assert!(!shown(&h), "with no overlay over the photo");
    ask(&mut h, "ui.clickWidget", json!({"id": format!("maskVisible:{mid}")}));
    assert!(visible(&h, mid), "and on again");
    assert!(!shown(&h));
}

#[test]
fn a_component_row_shows_the_whole_mask() {
    let mut h = masking();
    let mid = adjusted_radial(&mut h);
    exec(&mut h, "mask.addComponent", json!({"op": "subtract", "kind": "linear", "start": [0.5, 0.0], "end": [0.5, 0.3]}));
    assert_eq!(h.app.session.active_mask, Some(mid));
    for i in 0..2 {
        ask(&mut h, "ui.hoverWidget", json!({"id": format!("component:{i}")}));
        assert!(shown(&h), "the pointer on component {i} shows the mask");
        assert_eq!(h.app.ui.mask_hover.or(h.app.ui.mask_hover_prev), Some(mid), "the whole mask, not the one component");
    }
    away(&mut h);
    assert!(!shown(&h), "leaving the row hides it again");
}

#[test]
fn the_pin_shows_the_overlay() {
    let mut h = masking();
    adjusted_radial(&mut h);
    away(&mut h);
    let img = h.app.image_rect.expect("the photo on screen");
    ask(&mut h, "ui.move", json!({"x": img.center().x, "y": img.center().y}));
    assert!(shown(&h), "the pointer on the pin shows the overlay");
    away(&mut h);
    assert!(!shown(&h));
}

#[test]
fn a_mask_without_adjustments_keeps_its_overlay() {
    let mut h = masking();
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    let mid = h.app.session.active_mask.expect("selected");
    away(&mut h);
    assert!(shown(&h), "no adjustment yet: the overlay shows what it selects");
    ask(&mut h, "ui.hoverWidget", json!({"id": format!("mask:{mid}")}));
    assert!(shown(&h), "also with the pointer on its own row");
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 0.5}}));
    ask(&mut h, "ui.hoverWidget", json!({"id": format!("mask:{mid}")}));
    assert!(!shown(&h), "an adjustment given: its row shows no overlay any more");
}

#[test]
fn o_still_shows_and_hides_the_overlay() {
    let mut h = masking();
    let mid = adjusted_radial(&mut h);
    ask(&mut h, "ui.hoverWidget", json!({"id": format!("mask:{mid}")}));
    exec(&mut h, "view.maskOverlay", json!({"show": true}));
    assert!(shown(&h), "O shows it, even over its own row");
    exec(&mut h, "view.maskOverlay", json!({"show": false}));
    assert!(!shown(&h), "and hides it");
}
