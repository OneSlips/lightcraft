//! The panel a tool takes the place of comes back when the tool closes (item 12 of the evening
//! retest): open Edit → Masking (or Crop) → close it → Edit again, at the same width and scroll
//! position, with the photo where it was; a hidden panel stays hidden.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::RightPanel;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn detail() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h
}

fn exec(h: &mut Headless, command: &str) {
    let r = h.request("engine.execute", json!({"command": command, "params": {}}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
    h.step();
}

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

/// The Edit panel's first slider on show (its id and where it is: it moves when the panel scrolls).
fn first_slider(h: &Headless) -> (String, egui::Rect) {
    let panel = widget(h, "panel:right_panel").expect("the right panel");
    h.app
        .widgets
        .iter()
        .filter(|(w, r)| w.starts_with("slider:") && panel.contains_rect(*r))
        .min_by(|a, b| a.1.top().total_cmp(&b.1.top()))
        .map(|(w, r)| (w.clone(), *r))
        .expect("a slider in the Edit panel")
}

#[test]
fn the_edit_panel_comes_back_after_masking_and_crop_where_it_was() {
    let mut h = detail();
    // scroll the Edit panel down a little (the wheel scrolls what the pointer is over)
    let panel = widget(&h, "panel:right_panel").expect("the right panel");
    h.request("ui.move", json!({"x": panel.center().x, "y": panel.center().y}), T);
    let r = h.request("ui.scroll", json!({"dy": -300.0}), T);
    assert_eq!(r["ok"], true, "{r}");
    for _ in 0..60 {
        h.step();
    }
    let (slider, at) = first_slider(&h);
    let photo = widget(&h, "canvas:image").expect("the photo");
    for tool in ["panel.masking", "panel.crop", "panel.remove", "panel.redeye"] {
        exec(&mut h, tool);
        assert!(h.app.ui.right.is_tool(), "{tool}: {:?}", h.app.ui.right);
        assert_eq!(widget(&h, "panel:right_panel"), Some(panel), "{tool}: the panel keeps its place and width");
        // the same button again closes the tool: Edit is back, as it was
        exec(&mut h, tool);
        assert_eq!(h.app.ui.right, RightPanel::Edit, "{tool}: the Edit panel comes back");
        assert_eq!(widget(&h, "panel:right_panel"), Some(panel), "{tool}: same place and width");
        assert_eq!(widget(&h, "canvas:image"), Some(photo), "{tool}: the photo didn't move");
        assert_eq!(widget(&h, &slider), Some(at), "{tool}: Edit kept its scroll position");
    }
    // Masking, then Crop from it, then Crop closed: exactly what was there before, Masking (note 21); then Edit
    exec(&mut h, "panel.masking");
    exec(&mut h, "panel.crop");
    exec(&mut h, "panel.crop");
    assert_eq!(h.app.ui.right, RightPanel::Masking);
    exec(&mut h, "panel.masking");
    assert_eq!(h.app.ui.right, RightPanel::Edit);
    // Return (Done) in a tool also goes back
    exec(&mut h, "panel.masking");
    exec(&mut h, "tool.done");
    assert_eq!(h.app.ui.right, RightPanel::Edit);
}

#[test]
fn a_hidden_panel_stays_hidden_after_a_tool() {
    let mut h = detail();
    exec(&mut h, "panel.close");
    assert_eq!(h.app.ui.right, RightPanel::None);
    let photo = widget(&h, "canvas:image").expect("the photo");
    for tool in ["panel.masking", "panel.crop"] {
        exec(&mut h, tool);
        assert!(h.app.ui.right.is_tool(), "{tool} shows");
        exec(&mut h, tool);
        assert_eq!(h.app.ui.right, RightPanel::None, "{tool}: nothing shows again");
        h.settle(SETTLE);
        assert_eq!(widget(&h, "canvas:image"), Some(photo), "{tool}: the photo is where it was");
    }
    exec(&mut h, "panel.masking");
    exec(&mut h, "tool.done");
    assert_eq!(h.app.ui.right, RightPanel::None, "Done: hidden as before");
    // a panel picked meanwhile is what comes back: Info → Masking → closed = Info
    exec(&mut h, "panel.info");
    exec(&mut h, "panel.masking");
    exec(&mut h, "panel.masking");
    assert_eq!(h.app.ui.right, RightPanel::Info);
}

/// "I" toggles the Info panel; pressing it again brings back exactly what the right column showed before, closed
/// included (test notes 2026-10-24 note 21: the Edit sliders stayed gone).
#[test]
fn info_toggled_off_brings_back_exactly_what_was_there() {
    let mut h = detail();
    assert_eq!(h.app.ui.right, RightPanel::Edit);
    // the key, as the user pressed it
    let key = |h: &mut Headless| {
        let r = h.request("ui.key", json!({"key": "I"}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.step();
        h.step();
    };
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::Info);
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::Edit, "Edit → I → I = Edit");
    exec(&mut h, "panel.masking");
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::Info);
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::Masking, "Masks → I → I = Masks");
    // and Masks still closes back to Edit
    exec(&mut h, "panel.masking");
    assert_eq!(h.app.ui.right, RightPanel::Edit);
    for panel in ["panel.keywords", "panel.versions", "panel.activity"] {
        exec(&mut h, panel);
        exec(&mut h, panel);
        assert_eq!(h.app.ui.right, RightPanel::Edit, "{panel} toggled off: Edit again");
    }
    exec(&mut h, "panel.close");
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::Info);
    key(&mut h);
    assert_eq!(h.app.ui.right, RightPanel::None, "closed → I → I = closed");
}

/// A `ui.set` that changes something else (here the panel width) keeps a pending toggle: I still puts Edit back
/// (CodeRabbit on #3031).
#[test]
fn a_ui_set_between_the_two_presses_keeps_the_toggle() {
    let mut h = detail();
    exec(&mut h, "panel.info");
    assert_eq!(h.app.ui.right, RightPanel::Info);
    let r = h.request("ui.set", json!({"rightWidth": 320.0}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    exec(&mut h, "panel.info");
    assert_eq!(h.app.ui.right, RightPanel::Edit, "Edit → I → ui.set → I = Edit");
}

/// A tool panel restored from the saved UI state (the panel before it unknown) closes to Edit, not to a hidden column.
#[test]
fn a_tool_restored_from_saved_state_closes_to_edit() {
    let mut ui: crate::state::UiState = serde_json::from_str(r#"{"right": "crop"}"#).unwrap();
    assert_eq!(ui.right, RightPanel::Crop);
    ui.note_right();
    assert_eq!(ui.leave_tool(), RightPanel::Edit);
}
