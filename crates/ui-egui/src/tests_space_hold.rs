//! Holding Space never repeats (test notes 2026-10-31 note 6): Space is a modifier like Shift, so
//! the system's auto-repeat presses while it is held are ignored. One press toggles the zoom at most
//! once, in the grid, the loupe and Develop, at fit and at 100 %; Space+drag still pans with a tool
//! armed; a text box still types a space for each repeat.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{ViewMode, Zoom};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
const REPEATS: usize = 9;

fn demo() -> Headless {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    h.settle(SETTLE);
    h
}

fn ask(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
}

fn space(h: &mut Headless, phase: &str) {
    ask(h, "ui.key", json!({"key": "space", "phase": phase}));
    h.step();
}

/// Space down, `REPEATS` auto-repeats (one a frame), up: the zoom after each frame.
fn hold_space(h: &mut Headless) -> Vec<Zoom> {
    let mut seen = vec![h.app.ui.zoom];
    space(h, "down");
    seen.push(h.app.ui.zoom);
    for _ in 0..REPEATS {
        space(h, "repeat");
        seen.push(h.app.ui.zoom);
    }
    space(h, "up");
    seen.push(h.app.ui.zoom);
    seen
}

fn changes(seen: &[Zoom]) -> usize {
    seen.windows(2).filter(|w| w[0] != w[1]).count()
}

#[test]
fn holding_space_toggles_the_zoom_at_most_once() {
    use crate::state::RightPanel;
    // (where, view, right panel, start at 100 %)
    for (name, view, right, at_100) in [
        ("grid", ViewMode::PhotoGrid, RightPanel::None, false),
        ("loupe at fit", ViewMode::Detail, RightPanel::None, false),
        ("loupe at 100 %", ViewMode::Detail, RightPanel::None, true),
        ("develop at fit", ViewMode::Detail, RightPanel::Edit, false),
        ("develop at 100 %", ViewMode::Detail, RightPanel::Edit, true),
    ] {
        let mut h = demo();
        h.app.ui.view = view;
        h.app.ui.right = right;
        if at_100 {
            ask(&mut h, "engine.execute", json!({"command": "view.zoom100"}));
        }
        h.settle(SETTLE);
        let seen = hold_space(&mut h);
        assert!(changes(&seen) <= 1, "{name}: one Space press changed the zoom {} times: {seen:?}", changes(&seen));
        // the loupe's Space press is Toggle Zoom: exactly once, on the press
        if view == ViewMode::Detail {
            assert_eq!(changes(&seen[..2]), 1, "{name}: the press toggled the zoom: {seen:?}");
        }
    }
}

#[test]
fn space_repeats_while_dragging_still_pan_with_a_tool_armed() {
    let mut h = demo();
    ask(&mut h, "ui.set", json!({"view": "detail", "right": "edit"}));
    ask(&mut h, "engine.execute", json!({"command": "panel.masking"}));
    ask(&mut h, "engine.execute", json!({"command": "view.zoom100"}));
    h.settle(SETTLE);
    ask(&mut h, "ui.clickWidget", json!({"id": "maskNew:brush"}));
    h.step();
    assert_eq!(h.app.ui.tool, "brush");
    let (zoom, pan) = (h.app.ui.zoom, h.app.ui.pan);
    space(&mut h, "down");
    for _ in 0..3 {
        space(&mut h, "repeat");
    }
    let c = h.app.canvas_rect.expect("the loupe").center();
    ask(&mut h, "ui.drag", json!({"x": c.x, "y": c.y, "toX": c.x - 200.0, "toY": c.y - 120.0, "steps": 12}));
    for _ in 0..4 {
        space(&mut h, "repeat");
    }
    space(&mut h, "up");
    assert_eq!(h.app.ui.zoom, zoom, "Space held with a tool armed doesn't zoom");
    assert!((h.app.ui.pan.0 - pan.0).abs() > 0.01 || (h.app.ui.pan.1 - pan.1).abs() > 0.01, "Space+drag panned");
    let id = h.app.session.active().expect("a photo");
    assert_eq!(h.app.session.develop_of(id).map_or(0, |d| d.masks.len()), 0, "the brush painted nothing");
}

#[test]
fn a_text_box_still_types_a_space_for_each_repeat() {
    let mut h = demo();
    h.app.ui.view = ViewMode::PhotoGrid;
    h.settle(SETTLE);
    ask(&mut h, "ui.clickWidget", json!({"id": "field:search"}));
    h.step();
    ask(&mut h, "ui.text", json!({"text": "a"}));
    h.step();
    let zoom = h.app.ui.zoom;
    space(&mut h, "down");
    ask(&mut h, "ui.text", json!({"text": " "}));
    h.step();
    for _ in 0..3 {
        space(&mut h, "repeat");
        ask(&mut h, "ui.text", json!({"text": " "}));
        h.step();
    }
    space(&mut h, "up");
    ask(&mut h, "ui.text", json!({"text": "b"}));
    h.settle(SETTLE);
    assert_eq!(h.app.ui.search, "a    b", "the held Space typed four spaces");
    assert_eq!(h.app.ui.zoom, zoom, "and toggled no zoom");
}

#[test]
fn ui_key_rejects_an_unknown_phase() {
    let mut h = demo();
    let r = h.request("ui.key", json!({"key": "space", "phase": "sideways"}), T);
    assert_ne!(r["ok"], true, "{r}");
}
