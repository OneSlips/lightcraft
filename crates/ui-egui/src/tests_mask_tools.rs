//! Headless tests of the mask tools' follow-ups: overlays and tool graphics hide while a slider or
//! dial adjusts (and come back as they were), the overlay button's own state, the complete Add /
//! Subtract / Intersect menus, the Colour and Luminance Range pickers, the live Auto Mask stroke,
//! and one outline per Remove spot.

use std::time::Duration;

use lightcraft_develop::MaskShape;
use lightcraft_pipeline::Overlay;
use serde_json::json;

use crate::headless::Headless;
use crate::panels::detail::view_overlay;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn detail(panel: &str) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("engine.execute", json!({"command": panel}), T);
    assert_eq!(r["ok"], true, "{r}");
    h
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

fn develop(h: &Headless) -> lightcraft_develop::DevelopSettings {
    let id = h.app.session.active().expect("active photo");
    (*h.app.session.develop_of(id).unwrap_or_default()).clone()
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
}

fn pointer(h: &mut Headless, events: serde_json::Value) {
    let r = h.request("ui.pointer", json!({"events": events}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
}

/// Pointer events with modifier `key` (`shift`, `alt`) held.
fn pointer_with(h: &mut Headless, events: serde_json::Value, key: &str) {
    let r = h.request("ui.pointer", json!({"events": events, key: true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
}

fn widgets(h: &Headless, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = h.app.widgets.iter().filter(|(id, _)| id.starts_with(prefix)).map(|(id, _)| id.clone()).collect();
    v.dedup();
    v
}

/// Every kind of mask Create New Mask offers is in Add, Subtract and Intersect too, in the same
/// order (Radial, then Subtract Background, was missing).
#[test]
fn add_subtract_and_intersect_offer_every_mask_kind() {
    let mut h = detail("panel.masking");
    h.step();
    let tiles: Vec<String> =
        widgets(&h, "maskNew:").into_iter().filter(|w| !w.starts_with("maskNewLabel")).map(|w| w.replace("maskNew:", "")).collect();
    assert_eq!(tiles.len(), 13, "{tiles:?}");
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    h.step();
    for (button, op) in [("button:maskAddComp", "add"), ("button:maskSubComp", "subtract"), ("button:maskIntComp", "intersect")] {
        click(&mut h, button);
        h.step();
        let menu: Vec<String> = widgets(&h, &format!("maskComp:{op}:")).into_iter().map(|w| w.replace(&format!("maskComp:{op}:"), "")).collect();
        assert_eq!(menu, tiles, "{op}");
        let r = h.request("ui.key", json!({"key": "Escape"}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.step();
    }
    // the user's case: a radial, minus the background
    let before = develop(&h).masks[0].components.len();
    click(&mut h, "button:maskSubComp");
    h.step();
    click(&mut h, "maskComp:subtract:background");
    let m = &develop(&h).masks[0];
    assert_eq!(m.components.len(), before + 1);
    assert!(matches!(m.components.last().map(|c| (&c.shape, c.op)), Some((MaskShape::Background { .. }, lightcraft_develop::MaskOp::Subtract))));
}
