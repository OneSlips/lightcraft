//! Test notes 2026-10-28, notes 26 and 28, headless: the mask row activates the mask's top
//! component (a Brush arms the brush on it, a gradient shows its handles), pins stay draggable
//! with the brush armed, and the selected mask (and its component) stays selected when the
//! filmstrip selection grows or the active photo moves to a photo with its linked copy.

use std::time::Duration;

use lightcraft_develop::MaskShape;
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn masking() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("engine.execute", json!({"command": "panel.masking"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
    r["result"].clone()
}

fn develop(h: &Headless) -> lightcraft_develop::DevelopSettings {
    let id = h.app.session.active().expect("active photo");
    (*h.app.session.develop_of(id).unwrap_or_default()).clone()
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
}

fn pointer(h: &mut Headless, events: serde_json::Value) {
    let r = h.request("ui.pointer", json!({"events": events}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
}

fn drag(h: &mut Headless, from: (f64, f64), to: (f64, f64)) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    pointer(
        h,
        json!([{"kind": "down", "x": from.0, "y": from.1}, {"kind": "drag", "x": mid.0, "y": mid.1},
               {"kind": "drag", "x": to.0, "y": to.1}, {"kind": "up", "x": to.0, "y": to.1}]),
    );
    h.step();
}

fn widgets(h: &Headless, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = h.app.widgets.iter().filter(|(id, _)| id.starts_with(prefix)).map(|(id, _)| id.clone()).collect();
    v.sort();
    v.dedup();
    v
}

fn strokes(h: &Headless, mid: u32) -> Vec<i32> {
    develop(h)
        .masks
        .iter()
        .find(|m| m.id == mid)
        .map(|m| {
            m.components
                .iter()
                .map(|c| match &c.shape {
                    MaskShape::Brush { strokes } => strokes.len() as i32,
                    _ => -1,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The first stroke point of mask `mid`'s component `k` (where its pin is dragged from).
fn first_point(h: &Headless, mid: u32, k: usize) -> (f64, f64) {
    let d = develop(h);
    match d.masks.iter().find(|m| m.id == mid).and_then(|m| m.components.get(k)).map(|c| &c.shape) {
        Some(MaskShape::Brush { strokes }) => strokes.first().and_then(|s| s.points.first()).map(|p| (p.x, p.y)).expect("a stroke"),
        other => panic!("not a brush: {other:?}"),
    }
}

/// Note 28 (decision): clicking the mask row activates its top component. A brush-only mask arms
/// the brush on it (a drag adds a stroke, no new mask); a drag starting on its pin moves the pin
/// instead. A Linear + Brush mask selects the Linear (handles, no brush); its Brush row then arms
/// the brush on the Brush.
#[test]
fn the_mask_row_activates_its_top_component() {
    let mut h = masking();
    click(&mut h, "maskNew:brush");
    drag(&mut h, (0.3, 0.3), (0.4, 0.3));
    let brush = h.app.session.active_mask.expect("the brush mask");
    exec(&mut h, "mask.add", json!({"kind": "linear", "start": [0.5, 0.3], "end": [0.5, 0.6]}));
    let lin = h.app.session.active_mask.expect("the gradient");
    exec(&mut h, "mask.addComponent", json!({"kind": "brush"}));
    assert_ne!(brush, lin);

    // the brush mask's row: the brush armed on its component 0
    click(&mut h, &format!("mask:{brush}"));
    assert_eq!(h.app.session.active_mask, Some(brush));
    assert_eq!(h.app.ui.tool, "brush", "the row arms the brush on the mask's Brush");
    assert_eq!(h.app.ui.brush_on.map(|b| (b.1, b.2)), Some((brush, 0)));
    assert!(h.app.ui.mask_new.is_none(), "not for a new mask");
    pointer(&mut h, json!([{"kind": "move", "x": 0.7, "y": 0.75}]));
    assert!(!widgets(&h, "brushCursor:").is_empty(), "the brush cursor shows");
    drag(&mut h, (0.7, 0.75), (0.8, 0.75));
    assert_eq!(develop(&h).masks.len(), 2, "no new mask");
    assert_eq!(strokes(&h, brush), vec![2], "the stroke went into the selected brush mask");
    // its pin: a drag from it moves the strokes, paints nothing
    let pins = widgets(&h, &format!("maskPin:{brush}:0"));
    assert!(!pins.is_empty(), "the pin shows with the brush armed");
    let pin = h.app.widgets.iter().find(|(id, _)| *id == pins[0]).map(|(_, r)| *r).expect("the pin's place");
    let (img, before) = (h.app.image_rect.expect("the photo on screen"), first_point(&h, brush, 0));
    let at = ((pin.center().x - img.left()) / img.width(), (pin.center().y - img.top()) / img.height());
    drag(&mut h, (at.0 as f64, at.1 as f64), (at.0 as f64 + 0.05, at.1 as f64 + 0.05));
    assert_eq!(strokes(&h, brush), vec![2], "dragging the pin painted nothing");
    assert_ne!(first_point(&h, brush, 0), before, "the pin moved the strokes");

    // the Linear + Brush mask's row: the Linear (its top) is active, no brush
    click(&mut h, &format!("mask:{lin}"));
    assert_eq!(h.app.session.active_mask, Some(lin));
    assert!(h.app.ui.tool.is_empty() && h.app.ui.brush_on.is_none(), "a gradient on top arms nothing: {:?}", h.app.ui.tool);
    pointer(&mut h, json!([{"kind": "move", "x": 0.9, "y": 0.9}]));
    assert!(!widgets(&h, &format!("maskHandle:{lin}:0:")).is_empty(), "its handles show");
    click(&mut h, "component:1");
    assert_eq!(h.app.ui.brush_on.map(|b| (b.1, b.2)), Some((lin, 1)), "the Brush row arms the brush on it");
    // a new-mask tool still deselects the mask (note 9 stays for the tool buttons)
    click(&mut h, "maskNew:radial");
    assert_eq!(h.app.session.active_mask, None);
}

/// Note 26: a ⇧-click range (or ⌘-click) that keeps the active photo keeps the selected mask and
/// component; moving the active photo to one with the linked copy selects that copy (its matching
/// component too); a photo without it has none selected.
#[test]
fn the_selected_mask_survives_range_selection_and_follows_its_linked_copy() {
    let mut h = masking();
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(4).map(|p| p.0).collect();
    let (a, b, c, e) = (ids[0], ids[1], ids[2], ids[3]);
    // b has a mask of its own first: its copy of a's mask gets another id
    exec(&mut h, "library.select", json!({"ids": [b]}));
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    exec(&mut h, "library.select", json!({"ids": [a, b, c], "active": a}));
    exec(&mut h, "develop.autoSync", json!({"on": true}));
    exec(&mut h, "mask.add", json!({"kind": "linear", "start": [0.5, 0.3], "end": [0.5, 0.6]}));
    exec(&mut h, "mask.addComponent", json!({"kind": "brush"}));
    exec(&mut h, "develop.autoSync", json!({"on": false}));
    let on_a = h.app.session.active_mask.expect("selected");
    let link = develop(&h).masks[0].link.clone();
    assert!(!link.is_empty());
    click(&mut h, &format!("mask:{on_a}"));
    click(&mut h, "component:1");
    assert_eq!(h.app.ui.brush_on.map(|x| x.2), Some(1));

    // a range from a (a stays active): the mask and its Brush stay
    exec(&mut h, "library.select", json!({"ids": [a]}));
    exec(&mut h, "library.select", json!({"ids": [c], "mode": "range"}));
    assert_eq!(h.app.session.active().map(|p| p.0), Some(a));
    assert_eq!(h.app.session.active_mask, Some(on_a), "a range keeps the selected mask");
    assert_eq!(h.app.ui.mask_component.map(|x| x.2), Some(1));
    exec(&mut h, "library.select", json!({"ids": [e], "mode": "toggle"}));
    assert_eq!(h.app.session.active_mask, Some(on_a), "a ⌘-click that keeps the active photo keeps it too");

    // the active photo moves to b: b's linked copy is selected, with its Brush
    exec(&mut h, "library.select", json!({"ids": [b], "mode": "click"}));
    h.step();
    let on_b = develop(&h).masks.iter().find(|m| m.link == link).map(|m| m.id).expect("b's copy");
    assert_eq!(h.app.session.active().map(|p| p.0), Some(b));
    assert_ne!(on_b, on_a, "another id on b");
    assert_eq!(h.app.session.active_mask, Some(on_b), "the linked copy stays selected");
    assert_eq!(h.app.ui.mask_component.map(|x| (x.1, x.2)), Some((on_b, 1)), "and its matching component");
    // a photo without the mask: none selected (never another mask)
    exec(&mut h, "library.select", json!({"ids": [e], "mode": "replace"}));
    h.step();
    assert_eq!(h.app.session.active_mask, None);
}
