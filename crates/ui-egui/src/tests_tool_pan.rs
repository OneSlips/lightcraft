//! Panning a zoomed photo while a tool panel is open (item 27 of the evening retest): with no
//! tool armed a drag pans in the Masking panel, as in the loupe; with a tool armed, Space+drag
//! pans (and the tool gets nothing); the same Space+drag in Remove and Crop.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn zoomed(panel: &str) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    exec(&mut h, panel);
    exec(&mut h, "view.zoom100");
    h.settle(SETTLE);
    assert!(crate::panels::detail::zoomed_in(&h.app), "zoomed in past the pane");
    h
}

fn exec(h: &mut Headless, command: &str) {
    let r = h.request("engine.execute", json!({"command": command, "params": {}}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
}

/// A drag across the middle of the loupe, by (-200, -120) points.
fn drag(h: &mut Headless) {
    let c = h.app.canvas_rect.expect("the loupe").center();
    let r = h.request("ui.drag", json!({"x": c.x, "y": c.y, "toX": c.x - 200.0, "toY": c.y - 120.0, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    for _ in 0..4 {
        h.step();
    }
}

fn space(h: &mut Headless, down: bool) {
    h.app.synthetic.push(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: down,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

fn masks(h: &Headless) -> usize {
    let id = h.app.session.active().expect("a photo");
    h.app.session.develop_of(id).map_or(0, |d| d.masks.len())
}

fn moved(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() > 0.01 || (a.1 - b.1).abs() > 0.01
}

#[test]
fn a_drag_pans_in_the_masking_panel_with_no_tool_armed() {
    let mut h = zoomed("panel.masking");
    assert!(h.app.ui.tool.is_empty() && h.app.ui.mask_new.is_none());
    let pan = h.app.ui.pan;
    drag(&mut h);
    assert!(moved(pan, h.app.ui.pan), "the drag panned: {pan:?} -> {:?}", h.app.ui.pan);
    assert!(h.app.gesture.is_none(), "the pan ends with the drag");
    assert_eq!(masks(&h), 0, "and made no mask");
}

#[test]
fn space_drag_pans_with_a_mask_tool_armed() {
    let mut h = zoomed("panel.masking");
    click(&mut h, "maskNew:brush");
    assert_eq!(h.app.ui.tool, "brush");
    let pan = h.app.ui.pan;
    space(&mut h, true);
    drag(&mut h);
    space(&mut h, false);
    assert!(moved(pan, h.app.ui.pan), "Space+drag panned: {pan:?} -> {:?}", h.app.ui.pan);
    assert_eq!(masks(&h), 0, "the brush painted nothing");
    assert_eq!(h.app.ui.tool, "brush", "and is still armed");
    assert!(crate::panels::detail::zoomed_in(&h.app), "Space didn't zoom out from under the hand");
    // without Space the brush paints
    drag(&mut h);
    assert_eq!(masks(&h), 1, "a plain drag paints with the brush");
    // a gradient armed: Space+drag pans too, and draws nothing
    click(&mut h, "maskNew:radial");
    let pan = h.app.ui.pan;
    space(&mut h, true);
    drag(&mut h);
    space(&mut h, false);
    assert!(moved(pan, h.app.ui.pan));
    assert_eq!(masks(&h), 1, "no radial drawn");
}

#[test]
fn space_drag_pans_in_remove_and_crop() {
    for panel in ["panel.remove", "panel.crop"] {
        let mut h = zoomed(panel);
        let id = h.app.session.active().expect("a photo");
        let before = h.app.session.develop_of(id).expect("settings");
        let pan = h.app.ui.pan;
        space(&mut h, true);
        drag(&mut h);
        space(&mut h, false);
        assert!(moved(pan, h.app.ui.pan), "{panel}: Space+drag panned: {pan:?} -> {:?}", h.app.ui.pan);
        let after = h.app.session.develop_of(id).expect("settings");
        assert_eq!(after.spots.len(), before.spots.len(), "{panel}: no spot");
        assert_eq!(after.crop, before.crop, "{panel}: the crop didn't move");
    }
}
