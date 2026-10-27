//! Test notes 2026-10-27, notes 5, 8, 9 and 10, headless: one masking mode at a time (a tool armed
//! for a new mask, or a selected mask edited; never both), one component edited on the photo at a
//! time (a Brush component arms the brush on itself, so strokes and erasing go into it), no red
//! stroke while painting a mask that has settings, and Space + click zooming whatever tool is armed.

use std::time::Duration;

use egui::{Event, Key, Modifiers};
use lightcraft_develop::{MaskOp, MaskShape};
use serde_json::json;

use crate::headless::Headless;
use crate::state::Zoom;
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

/// Strokes per component of mask `mid` (-1: not a Brush).
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

fn linear_start(h: &Headless, mid: u32) -> (f64, f64) {
    let d = develop(h);
    let m = d.masks.iter().find(|m| m.id == mid).expect("the gradient mask");
    let start = m.components.iter().find_map(|c| match &c.shape {
        MaskShape::Linear { start, .. } => Some((start.x, start.y)),
        _ => None,
    });
    start.expect("a linear gradient")
}

/// Note 9: a selected mask and an armed tool never at once. Selecting a mask puts the tool away
/// (a drag then moves the gradient, never paints); arming a tool deselects the mask; leaving
/// Masking and coming back never finds a tool armed; the brush that made a mask stops painting
/// once another mask is selected.
#[test]
fn a_selected_mask_and_an_armed_tool_are_never_at_once() {
    let mut h = masking();
    exec(&mut h, "mask.add", json!({"kind": "linear", "start": [0.5, 0.3], "end": [0.5, 0.6]}));
    let gradient = h.app.session.active_mask.expect("the gradient is selected");
    // Brush armed: the gradient is deselected
    click(&mut h, "maskNew:brush");
    assert_eq!(h.app.ui.tool, "brush");
    assert_eq!(h.app.session.active_mask, None, "arming a tool for a new mask deselects the mask");
    // the gradient's row clicked: the brush is put away, the gradient stays selected
    click(&mut h, &format!("mask:{gradient}"));
    h.step();
    assert_eq!(h.app.session.active_mask, Some(gradient));
    assert!(h.app.ui.tool.is_empty() && h.app.ui.mask_new.is_none(), "selecting a mask disarms the tool: {:?}", h.app.ui.tool);
    // a drag moves the gradient and paints nothing
    let before = linear_start(&h, gradient);
    drag(&mut h, (0.5, 0.45), (0.6, 0.55));
    assert_eq!(develop(&h).masks.len(), 1, "no brush mask was made");
    assert_ne!(linear_start(&h, gradient), before, "the drag moved the gradient");
    // a brush mask made by painting, then the gradient selected: the brush doesn't follow it
    click(&mut h, "maskNew:brush");
    drag(&mut h, (0.15, 0.8), (0.3, 0.8));
    let brush = h.app.session.active_mask.expect("the new brush mask");
    assert_ne!(brush, gradient);
    assert_eq!(h.app.ui.tool, "brush", "the brush keeps painting the mask it made");
    assert_eq!(strokes(&h, brush), vec![1]);
    let pin = widgets(&h, &format!("maskPin:{gradient}:"));
    assert!(!pin.is_empty(), "the gradient's pin shows: {pin:?}");
    click(&mut h, &format!("mask:{gradient}"));
    h.step();
    assert_eq!(h.app.session.active_mask, Some(gradient));
    assert!(h.app.ui.tool.is_empty(), "the brush was put away");
    let before = linear_start(&h, gradient);
    drag(&mut h, (0.5, 0.45), (0.55, 0.5));
    assert_eq!(strokes(&h, brush), vec![1], "the brush painted nothing more");
    assert_eq!(strokes(&h, gradient), vec![-1], "nor into the gradient");
    assert_ne!(linear_start(&h, gradient), before);
    // leaving Masking with a tool armed and coming back: nothing armed
    click(&mut h, "maskNew:radial");
    assert_eq!(h.app.ui.tool, "radial");
    exec(&mut h, "panel.masking", json!({}));
    assert_ne!(h.app.ui.right, crate::state::RightPanel::Masking, "Masking was left");
    exec(&mut h, "panel.masking", json!({}));
    assert_eq!(h.app.ui.right, crate::state::RightPanel::Masking);
    assert!(h.app.ui.tool.is_empty() && h.app.ui.mask_new.is_none(), "coming back never restores a tool: {:?}", h.app.ui.tool);
    // another photo: the brush armed on a component is put away
    exec(&mut h, "tool.brush", json!({"new": true}));
    exec(&mut h, "library.next", json!({}));
    h.step();
    assert!(h.app.ui.tool.is_empty() && h.app.ui.brush_on.is_none(), "another photo disarms: {:?}", h.app.ui.tool);
}

/// Note 10: a Linear + Brush mask edits one component at a time. The Linear row shows only the
/// gradient's handles (a drag moves it); the Brush row hides them and arms the brush on that
/// component (a drag adds a stroke to it, not a new mask; ⌥ erases from it); a brush-only mask and
/// a Subtract brush take strokes the same way; the B key arms the selected mask's Brush.
#[test]
fn one_component_is_edited_at_a_time_and_a_brush_component_takes_new_strokes() {
    let mut h = masking();
    exec(&mut h, "mask.add", json!({"kind": "linear", "start": [0.5, 0.3], "end": [0.5, 0.6]}));
    let mid = h.app.session.active_mask.expect("selected");
    exec(&mut h, "mask.addComponent", json!({"kind": "brush"}));
    exec(&mut h, "mask.brushStroke", json!({"points": [[0.2, 0.2], [0.25, 0.2]]}));
    assert_eq!(strokes(&h, mid), vec![-1, 1]);
    pointer(&mut h, json!([{"kind": "move", "x": 0.9, "y": 0.9}]));
    // the mask as a whole: one component's controls (the last one's), never two
    let handles = widgets(&h, &format!("maskHandle:{mid}:0:"));
    assert!(handles.is_empty(), "the gradient's handles hide while the brush is the active component: {handles:?}");
    assert!(h.app.ui.tool.is_empty(), "selecting the mask arms nothing");
    // the Linear row: its handles only, and a drag moves it
    click(&mut h, "component:0");
    pointer(&mut h, json!([{"kind": "move", "x": 0.9, "y": 0.9}]));
    assert!(h.app.ui.tool.is_empty());
    assert!(!widgets(&h, &format!("maskHandle:{mid}:0:")).is_empty(), "the gradient's handles show");
    assert!(widgets(&h, &format!("maskPin:{mid}:1")).is_empty(), "the brush's pin hides");
    let before = linear_start(&h, mid);
    drag(&mut h, (0.5, 0.45), (0.55, 0.5));
    assert_ne!(linear_start(&h, mid), before, "the drag moved the gradient");
    assert_eq!(strokes(&h, mid), vec![-1, 1]);
    // the Brush row: the brush armed on it; the gradient's handles hide; a drag adds a stroke to it
    click(&mut h, "component:1");
    pointer(&mut h, json!([{"kind": "move", "x": 0.9, "y": 0.9}]));
    assert_eq!(h.app.ui.tool, "brush");
    assert!(h.app.ui.mask_new.is_none(), "not a new mask");
    assert!(widgets(&h, &format!("maskHandle:{mid}:0:")).is_empty());
    drag(&mut h, (0.7, 0.8), (0.85, 0.8));
    assert_eq!(develop(&h).masks.len(), 1, "no new mask");
    assert_eq!(strokes(&h, mid), vec![-1, 2], "the stroke went into the Brush component");
    // ⌥ erases from it
    let r = h.request(
        "ui.pointer",
        json!({"events": [{"kind": "down", "x": 0.7, "y": 0.8}, {"kind": "drag", "x": 0.75, "y": 0.8}, {"kind": "drag", "x": 0.8, "y": 0.8}, {"kind": "up", "x": 0.8, "y": 0.8}], "alt": true}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    let d = develop(&h);
    let Some(MaskShape::Brush { strokes: s }) = d.masks[0].components.get(1).map(|c| &c.shape) else { panic!("a brush") };
    assert_eq!(s.len(), 3);
    assert!(s.last().is_some_and(|s| s.erase), "the ⌥ stroke erases");
    // a Subtract brush: its strokes go into it and it keeps subtracting
    exec(&mut h, "mask.addComponent", json!({"kind": "brush", "op": "subtract"}));
    click(&mut h, "component:2");
    assert_eq!(h.app.ui.brush_on.map(|b| b.2), Some(2));
    drag(&mut h, (0.1, 0.9), (0.2, 0.9));
    assert_eq!(strokes(&h, mid), vec![-1, 3, 1]);
    assert_eq!(develop(&h).masks[0].components[2].op, MaskOp::Subtract);
    // a brush-only mask: selected (the brush put away), then its Brush row arms the brush on it
    click(&mut h, "maskNew:brush");
    drag(&mut h, (0.3, 0.1), (0.4, 0.1));
    let only = h.app.session.active_mask.expect("the brush-only mask");
    click(&mut h, &format!("mask:{mid}"));
    click(&mut h, &format!("mask:{only}"));
    assert!(h.app.ui.tool.is_empty(), "selecting the mask row arms nothing");
    click(&mut h, "component:0");
    assert_eq!(h.app.ui.tool, "brush");
    drag(&mut h, (0.6, 0.1), (0.7, 0.1));
    assert_eq!(strokes(&h, only), vec![2], "the brush-only mask took the stroke");
    assert_eq!(develop(&h).masks.len(), 2);
    // the B key (tool.brush) on a selected brush mask arms its Brush too
    click(&mut h, &format!("mask:{only}"));
    assert!(h.app.ui.tool.is_empty());
    exec(&mut h, "tool.brush", json!({}));
    assert_eq!(h.app.ui.brush_on.map(|b| (b.1, b.2)), Some((only, 0)));
    // hostile arguments are errors, never a panic
    assert!(h.app.run("mask.editComponent", json!({"id": only, "component": 9})).is_err());
    assert!(h.app.run("mask.editComponent", json!({"id": 999, "component": 0})).is_err());
    assert!(h.app.run("mask.editComponent", json!({"component": -1})).is_err());
}

/// Note 5: painting with Auto Mask off on a mask that has settings shows the effect as it is
/// painted (the stroke goes to the photo live), never the red stroke; a mask without settings
/// still shows the red stroke while it is painted.
#[test]
fn painting_a_mask_with_settings_shows_its_effect_not_red() {
    let mut h = masking();
    exec(&mut h, "mask.next", json!({"resetNew": true}));
    h.app.ui.mask_reset_new = true;
    click(&mut h, "maskNew:brush");
    assert!(!h.app.ui.brush_auto_mask);
    // no settings: the red stroke while painting
    pointer(&mut h, json!([{"kind": "down", "x": 0.2, "y": 0.5}, {"kind": "drag", "x": 0.25, "y": 0.5}, {"kind": "drag", "x": 0.3, "y": 0.5}]));
    assert!(h.app.brush_live.is_some(), "a mask without settings shows the red stroke");
    pointer(&mut h, json!([{"kind": "up", "x": 0.3, "y": 0.5}]));
    let mid = h.app.session.active_mask.expect("the brush mask");
    assert_eq!(strokes(&h, mid), vec![1]);
    // with settings: no red; the stroke is on the photo while painted, one undo step
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1.0}}));
    assert_eq!(h.app.ui.tool, "brush", "the brush stays armed on its mask");
    pointer(&mut h, json!([{"kind": "down", "x": 0.2, "y": 0.7}, {"kind": "drag", "x": 0.25, "y": 0.7}, {"kind": "drag", "x": 0.3, "y": 0.7}]));
    assert!(h.app.brush_live.is_none(), "no red stroke on a mask with settings");
    assert_eq!(crate::panels::detail::view_overlay(&h.app, &develop(&h)), lightcraft_pipeline::Overlay::None, "no overlay either");
    assert_eq!(strokes(&h, mid), vec![2], "the stroke shows its effect on the photo while painted");
    assert!(h.app.session.interaction.is_some());
    pointer(&mut h, json!([{"kind": "up", "x": 0.3, "y": 0.7}]));
    h.step();
    assert_eq!(strokes(&h, mid), vec![2]);
    assert!(h.app.session.interaction.is_none());
    exec(&mut h, "edit.undo", json!({}));
    assert_eq!(strokes(&h, mid), vec![1], "one undo removes the whole stroke");
    // O shows the overlay on purpose: the red stroke again
    exec(&mut h, "view.maskOverlay", json!({"show": true}));
    pointer(&mut h, json!([{"kind": "down", "x": 0.2, "y": 0.3}, {"kind": "drag", "x": 0.25, "y": 0.3}, {"kind": "drag", "x": 0.3, "y": 0.3}]));
    assert!(h.app.brush_live.is_some(), "O: the overlay colour while painting");
    pointer(&mut h, json!([{"kind": "up", "x": 0.3, "y": 0.3}]));
}

/// Let the zoom animation end.
fn settle(h: &mut Headless) {
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        h.step();
    }
}

fn space(h: &mut Headless, pressed: bool) {
    h.app.synthetic.push(Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    h.step();
}

/// Note 8: with a mask tool armed, Space + click zooms (in at the click from fit, out to fit when
/// zoomed in) and never paints or makes a mask; Space pressed alone doesn't zoom; let go, the tool
/// is back. The same in the Remove tool.
#[test]
fn space_click_zooms_with_a_tool_armed_and_never_paints() {
    let mut h = masking();
    // (the demo photo is small: a click zoom that makes it larger than the loupe)
    h.app.ui.click_zoom = 800;
    click(&mut h, "maskNew:brush");
    assert_eq!(h.app.ui.zoom, Zoom::Fit);
    space(&mut h, true);
    assert_eq!(h.app.ui.zoom, Zoom::Fit, "Space alone doesn't zoom while a tool is armed");
    pointer(&mut h, json!([{"kind": "down", "x": 0.3, "y": 0.3}, {"kind": "up", "x": 0.3, "y": 0.3}]));
    h.step();
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "Space + click zooms in: {:?}", h.app.ui.zoom);
    assert!(develop(&h).masks.is_empty(), "the click made no mask");
    settle(&mut h);
    assert!(crate::panels::detail::zoomed_in(&h.app), "zoomed in past the loupe");
    // the middle of the loupe, in the zoomed photo's coordinates
    let (img, canvas) = (h.app.image_rect.expect("photo"), h.app.canvas_rect.expect("loupe"));
    let (x, y) = ((canvas.center().x - img.left()) / img.width(), (canvas.center().y - img.top()) / img.height());
    pointer(&mut h, json!([{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]));
    h.step();
    assert_eq!(h.app.ui.zoom, Zoom::Fit, "Space + click zooms back out");
    assert!(develop(&h).masks.is_empty());
    space(&mut h, false);
    settle(&mut h);
    assert_eq!(h.app.ui.tool, "brush", "let go of Space: the brush is back");
    // the brush armed on an existing component: the same
    drag(&mut h, (0.2, 0.2), (0.3, 0.2));
    let mid = h.app.session.active_mask.expect("painted");
    space(&mut h, true);
    pointer(&mut h, json!([{"kind": "down", "x": 0.6, "y": 0.6}, {"kind": "up", "x": 0.6, "y": 0.6}]));
    h.step();
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)));
    assert_eq!(strokes(&h, mid), vec![1], "no stroke from the Space-click");
    space(&mut h, false);
    // the Remove tool: a Space-click adds no spot
    exec(&mut h, "view.zoomFit", json!({}));
    exec(&mut h, "panel.remove", json!({}));
    settle(&mut h);
    let spots = develop(&h).spots.len();
    space(&mut h, true);
    pointer(&mut h, json!([{"kind": "down", "x": 0.4, "y": 0.4}, {"kind": "up", "x": 0.4, "y": 0.4}]));
    h.step();
    space(&mut h, false);
    assert_eq!(develop(&h).spots.len(), spots, "no spot from the Space-click");
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)));
}
