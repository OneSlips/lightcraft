//! The reworked Masks panel: tools arm and wait for the photo (no empty masks), a second click
//! puts them away; the eye switches the mask's effect, O only the overlay; the overlay shows by
//! itself only for a fresh mask; Amount is the effect's strength; the bin deletes; the tree's
//! Add / Subtract combine.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn masking() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    assert_eq!(h.request("ui.set", json!({"view": "detail"}), T)["ok"], true);
    assert_eq!(h.request("engine.execute", json!({"command": "panel.masking"}), T)["ok"], true);
    h.settle(SETTLE);
    h
}

fn masks(h: &Headless) -> Vec<lightcraft_develop::Mask> {
    let id = h.app.session.active().expect("a photo");
    h.app.session.develop_of(id).expect("settings").masks.clone()
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}

fn drag(h: &mut Headless, from: (f64, f64), to: (f64, f64)) {
    let r = h.request(
        "ui.pointer",
        json!({"events": [{"kind": "down", "x": from.0, "y": from.1}, {"kind": "move", "x": (from.0 + to.0) / 2.0, "y": (from.1 + to.1) / 2.0}, {"kind": "move", "x": to.0, "y": to.1}, {"kind": "up", "x": to.0, "y": to.1}]}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.settle(SETTLE);
}

#[test]
fn tools_arm_and_wait_for_the_photo_and_a_second_click_disarms() {
    let mut h = masking();
    for tool in ["linear", "radial", "brush", "colorRange"] {
        click(&mut h, &format!("maskNew:{tool}"));
        assert_eq!(h.app.ui.tool, tool);
        click(&mut h, &format!("maskNew:{tool}"));
        assert!(h.app.ui.tool.is_empty(), "{tool}: a second click puts the tool away");
        assert!(masks(&h).is_empty(), "{tool}: arming makes no mask");
    }
    // armed linear: one drag makes one gradient, from the press to the release
    click(&mut h, "maskNew:linear");
    drag(&mut h, (0.5, 0.2), (0.5, 0.7));
    let m = masks(&h);
    assert_eq!(m.len(), 1, "one drag, one mask");
    let lightcraft_develop::MaskShape::Linear { start, end } = m[0].components[0].shape else { panic!("{:?}", m[0].components[0].shape) };
    assert!((start.y - 0.2).abs() < 0.05 && (end.y - 0.7).abs() < 0.05, "{start:?} {end:?}");
    assert_eq!(m[0].name, "Linear Gradient", "named after what it selects");
    // one undo step takes the drawn gradient away
    exec(&mut h, "edit.undo", json!({}));
    assert!(masks(&h).is_empty());
    exec(&mut h, "edit.redo", json!({}));
    // a radial: a click places one; clicking the tile twice made nothing
    click(&mut h, "maskNew:radial");
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": 0.3, "y": 0.4}, {"kind": "up", "x": 0.3, "y": 0.4}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(masks(&h).len(), 2);
}

#[test]
fn eye_switches_the_effect_o_the_overlay_and_amount_is_the_strength() {
    use lightcraft_pipeline::Overlay;
    let mut h = masking();
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    let d = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap();
    assert!(matches!(crate::panels::detail::view_overlay(&h.app, &d), Overlay::Mask { .. }), "a fresh mask shows its overlay");
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1.0}}));
    let d = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap();
    assert_eq!(crate::panels::detail::view_overlay(&h.app, &d), Overlay::None, "an adjusted mask shows its effect, not the overlay");
    // O: the overlay only
    let before = masks(&h);
    let r = h.request("ui.key", json!({"key": "o"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    let d = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap();
    assert!(matches!(crate::panels::detail::view_overlay(&h.app, &d), Overlay::Mask { .. }));
    assert_eq!(masks(&h), before, "O leaves the mask alone");
    h.request("ui.key", json!({"key": "o"}), T);
    h.settle(SETTLE);
    let d = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap();
    assert_eq!(crate::panels::detail::view_overlay(&h.app, &d), Overlay::None);
    // the eye: the mask's effect on / off (a quick before / after of that mask)
    let mid = masks(&h)[0].id;
    click(&mut h, &format!("maskVisible:{mid}"));
    assert!(!masks(&h)[0].visible, "the eye turns the effect off");
    click(&mut h, &format!("maskVisible:{mid}"));
    assert!(masks(&h)[0].visible);
    // the strength slider is the mask's Amount (its effect), not the overlay's opacity
    let overlay_opacity = h.app.ui.mask_overlay_opacity;
    exec(&mut h, "mask.adjust", json!({"values": {"amount": 40.0}}));
    assert_eq!(masks(&h)[0].adjust.amount, 40.0);
    assert_eq!(h.app.ui.mask_overlay_opacity, overlay_opacity);
    assert!(h.app.widgets.iter().any(|(w, _)| w == "slider:amount"), "Amount is in the panel");
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "slider:ui.maskOverlayOpacity"), "the overlay's opacity is in its popup");
}

#[test]
fn the_bin_deletes_and_the_tree_combines() {
    let mut h = masking();
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    let mid = masks(&h)[0].id;
    h.request("ui.hoverWidget", json!({"id": format!("mask:{mid}")}), T);
    h.settle(SETTLE);
    // the selected mask's components hang under it, with Add / Subtract to combine more
    assert!(h.app.widgets.iter().any(|(w, _)| w == "component:0"));
    assert!(h.app.widgets.iter().any(|(w, _)| w == "button:maskSubComp"));
    exec(&mut h, "mask.addComponent", json!({"op": "subtract", "kind": "linear", "start": [0.5, 0.0], "end": [0.5, 0.4]}));
    h.settle(SETTLE);
    assert!(h.app.widgets.iter().any(|(w, _)| w == "component:1"));
    assert_eq!(masks(&h)[0].name, "Radial Gradient − Linear Gradient", "the automatic name follows the components");
    exec(&mut h, "mask.component", json!({"component": 1, "action": "invert"}));
    assert!(masks(&h)[0].components[1].invert);
    // renamed by the photographer: the name stays
    exec(&mut h, "mask.rename", json!({"name": "Vignette"}));
    exec(&mut h, "mask.addComponent", json!({"op": "add", "kind": "sky"}));
    assert_eq!(masks(&h)[0].name, "Vignette");
    // the bin
    click(&mut h, &format!("maskDelete:{mid}"));
    assert!(masks(&h).is_empty());
}
