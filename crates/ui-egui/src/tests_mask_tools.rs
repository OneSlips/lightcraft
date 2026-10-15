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

/// The user's standard: while a slider (or a dial) adjusts, the overlay, pins and tool graphics
/// hide, even an overlay switched on with O; afterwards they are back as they were.
#[test]
fn overlays_hide_while_adjusting_and_come_back_as_they_were() {
    let mut h = detail("panel.masking");
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5], "rx": 0.2, "ry": 0.2}));
    exec(&mut h, "view.maskOverlay", json!({"show": true}));
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }));
    // a slider held: an effect slider hides everything…
    h.app.ui.dragging_control = Some("exposure".into());
    assert!(crate::panels::detail::adjusting(&h.app));
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None, "the effect shows, not the overlay");
    h.step();
    assert!(widgets(&h, "canvas:overlayOn").is_empty(), "no overlay hint while adjusting");
    // …one that shapes the mask (a range's Refine, a radial's Feather) keeps it
    h.app.ui.dragging_control = Some("feather".into());
    assert!(!crate::panels::detail::adjusting(&h.app));
    // released: the overlay switched on with O is back
    h.app.ui.dragging_control = None;
    assert!(h.app.ui.mask_overlay);
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }));
    h.step();
    assert_eq!(widgets(&h, "canvas:overlayOn"), vec!["canvas:overlayOn".to_string()], "the canvas says the overlay is on (O)");
    // the same in the Remove tool: Visualize Spots hides while adjusting
    exec(&mut h, "panel.remove", json!({}));
    h.app.ui.visualize_spots = true;
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Spots(_)));
    h.app.ui.dragging_control = Some("light.exposure".into());
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None);
    h.app.ui.dragging_control = None;
}

/// Choosing how the overlay looks in its settings doesn't switch it on for good (that left it
/// "stuck on"); ⇧O, cycling the look from the keyboard, still shows it; the pop-up has an explicit
/// on / off switch.
#[test]
fn the_overlay_look_menu_does_not_latch_the_overlay_on() {
    let mut h = detail("panel.masking");
    exec(&mut h, "mask.add", json!({"kind": "radial", "center": [0.5, 0.5]}));
    assert!(!h.app.ui.mask_overlay);
    exec(&mut h, "view.maskOverlayMode", json!({"mode": "whiteOnBlack", "show": false}));
    assert!(!h.app.ui.mask_overlay, "picking a look leaves the overlay as it was");
    exec(&mut h, "view.maskOverlayMode", json!({}));
    assert!(h.app.ui.mask_overlay, "⇧O cycles the look and shows it");
    // the settings pop-up's switch
    exec(&mut h, "view.maskOverlay", json!({"show": false}));
    click(&mut h, "icon:maskOverlaySettings");
    h.step();
    click(&mut h, "check:maskOverlayOn");
    assert!(h.app.ui.mask_overlay, "the pop-up's Show overlay switch turns it on");
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

/// Colour Range: a click replaces, ⇧-click adds, a drag samples an area (one pin for it), ⌥-click
/// removes a pin, a pin dragged samples elsewhere; the panel shows the colours as swatches.
#[test]
fn colour_range_picks_add_remove_and_move() {
    let mut h = detail("panel.masking");
    click(&mut h, "maskNew:colorRange");
    pointer(&mut h, json!([{"kind": "down", "x": 0.3, "y": 0.3}, {"kind": "up", "x": 0.3, "y": 0.3}]));
    let samples = |h: &Headless| match develop(h).masks.first().and_then(|m| m.components.first()).map(|c| c.shape.clone()) {
        Some(MaskShape::ColorRange { samples, areas, .. }) => (samples.len(), lightcraft_develop::color_range_pins(&areas).len()),
        other => panic!("{other:?}"),
    };
    assert_eq!(samples(&h), (1, 1), "a click picks one colour, with its pin");
    pointer(&mut h, json!([{"kind": "down", "x": 0.7, "y": 0.6}, {"kind": "up", "x": 0.7, "y": 0.6}]));
    assert_eq!(samples(&h), (1, 1), "a second click replaces it");
    pointer_with(&mut h, json!([{"kind": "down", "x": 0.2, "y": 0.8}, {"kind": "up", "x": 0.2, "y": 0.8}]), "shift");
    assert_eq!(samples(&h), (2, 2), "⇧-click adds");
    // a drag over an area: up to three of its colours, one pin
    pointer_with(
        &mut h,
        json!([{"kind": "down", "x": 0.05, "y": 0.05}, {"kind": "drag", "x": 0.3, "y": 0.2},
               {"kind": "drag", "x": 0.6, "y": 0.45}, {"kind": "up", "x": 0.6, "y": 0.45}]),
        "shift",
    );
    let (n, pins) = samples(&h);
    assert_eq!(pins, 3, "the area has one pin");
    assert!((3..=5).contains(&n), "{n} samples");
    h.step();
    assert_eq!(widgets(&h, "colorPin:").len(), 3, "a pin per picked place on the photo");
    assert_eq!(widgets(&h, "colorSwatch:0:").len(), n, "a swatch per colour in the panel");
    // ⌥-click on the first pin removes it
    pointer_with(&mut h, json!([{"kind": "down", "x": 0.7, "y": 0.6}, {"kind": "up", "x": 0.7, "y": 0.6}]), "alt");
    assert_eq!(samples(&h).1, 2);
    // the command moves a pin and samples there again
    let r = exec(&mut h, "mask.colorPin", json!({"index": 0, "x": 0.4, "y": 0.4}));
    assert_eq!(r["pins"], 2, "{r}");
    // hostile arguments are errors, never a panic
    assert!(h.app.run("mask.colorPin", json!({"index": 99, "remove": true})).is_err());
    assert!(h.app.run("mask.sampleColor", json!({"rect": [0.1, 0.1]})).is_err());
    assert!(h.app.run("mask.sampleColor", json!({"x": f64::MAX, "y": 0.5})).is_err());
}

/// Luminance Range: a click picks a band round the tone there, a drag the tones of the area, ⇧
/// widens what is there.
#[test]
fn luminance_range_picks_from_the_photo() {
    let mut h = detail("panel.masking");
    exec(&mut h, "mask.add", json!({"kind": "luminanceRange", "lo": 0.0, "hi": 1.0}));
    h.app.ui.tool = "lumRange".into();
    h.step();
    let range = |h: &Headless| match develop(h).masks[0].components[0].shape {
        MaskShape::LuminanceRange { lo, hi, .. } => (lo, hi),
        ref s => panic!("{s:?}"),
    };
    pointer(&mut h, json!([{"kind": "down", "x": 0.5, "y": 0.5}, {"kind": "up", "x": 0.5, "y": 0.5}]));
    let (lo, hi) = range(&h);
    assert!(hi - lo > 0.1 && hi - lo < 0.2, "a band round the tone: {lo}..{hi}");
    pointer(
        &mut h,
        json!([{"kind": "down", "x": 0.05, "y": 0.05}, {"kind": "drag", "x": 0.5, "y": 0.5}, {"kind": "drag", "x": 0.95, "y": 0.95}, {"kind": "up", "x": 0.95, "y": 0.95}]),
    );
    let (lo2, hi2) = range(&h);
    assert!(hi2 - lo2 > hi - lo, "the whole photo has more tones than a point: {lo2}..{hi2}");
    let r = exec(&mut h, "mask.sampleLuminance", json!({"x": 0.5, "y": 0.5, "add": true}));
    assert!(r["lo"].as_f64().unwrap() <= lo2 + 1e-9 && r["hi"].as_f64().unwrap() >= hi2 - 1e-9, "⇧ only widens: {r}");
}

/// Auto Mask while painting: the stroke goes to the photo as it grows (the render shows the real,
/// edge-aware coverage), replacing itself, and the whole stroke is one undo step.
#[test]
fn auto_mask_strokes_show_live_and_undo_in_one_step() {
    let mut h = detail("panel.masking");
    click(&mut h, "maskNew:brush");
    h.app.ui.brush_auto_mask = true;
    let strokes = |h: &Headless| match develop(h).masks.first().and_then(|m| m.components.first()).map(|c| c.shape.clone()) {
        Some(MaskShape::Brush { strokes }) => strokes.iter().map(|s| s.points.len()).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    pointer(&mut h, json!([{"kind": "down", "x": 0.2, "y": 0.5}, {"kind": "drag", "x": 0.25, "y": 0.5}, {"kind": "drag", "x": 0.3, "y": 0.5}]));
    let live = strokes(&h);
    assert_eq!(live.len(), 1, "the stroke is on the photo before the button is let go: {live:?}");
    assert!(h.app.session.interaction.is_some(), "its undo step is open");
    // later points replace it (one stroke, longer), once the throttle allows
    std::thread::sleep(Duration::from_millis(150));
    pointer(&mut h, json!([{"kind": "drag", "x": 0.4, "y": 0.5}, {"kind": "drag", "x": 0.5, "y": 0.5}, {"kind": "up", "x": 0.5, "y": 0.5}]));
    let done = strokes(&h);
    assert_eq!(done.len(), 1, "{done:?}");
    assert!(done[0] > live[0], "{done:?} after {live:?}");
    assert!(h.app.session.interaction.is_none(), "the step is closed on release");
    exec(&mut h, "edit.undo", json!({}));
    assert!(strokes(&h).is_empty(), "one undo removes the whole stroke");
}

/// A Remove spot painted with many dabs has one outline on the photo, not a ring per dab, and its
/// source pin shows on hover so both pins can be moved.
#[test]
fn a_painted_spot_shows_one_outline_and_both_pins() {
    let mut h = detail("panel.remove");
    let path: Vec<[f64; 2]> = (0..60).map(|i| [0.2 + i as f64 * 0.005, 0.5]).collect();
    exec(&mut h, "spot.add", json!({"mode": "heal", "points": path, "size": 0.02}));
    exec(&mut h, "spot.select", json!({"index": 0}));
    pointer(&mut h, json!([{"kind": "move", "x": 0.3, "y": 0.5}]));
    assert_eq!(widgets(&h, "spotPin:"), vec!["spotPin:0".to_string()]);
    assert_eq!(widgets(&h, "spotSource:"), vec!["spotSource:0".to_string()], "the source pin to move it");
}
