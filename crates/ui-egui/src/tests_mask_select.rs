//! Test notes 2026-10-28, notes 26 and 28, headless: the mask row activates the mask's top
//! component (a Brush arms the brush on it, a gradient shows its handles), pins stay draggable
//! with the brush armed, and the selected mask (and its component) stays selected when the
//! filmstrip selection grows or the active photo moves to a photo with its linked copy.

use std::time::Duration;

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
