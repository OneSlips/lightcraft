//! The shot info strip (f-stop · shutter · ISO · focal length · lens · size) stays in the loupe when zoomed in: a
//! slim pill over the bottom edge of the photo on screen, left out while an on-photo tool is in use (test notes
//! 2026-10-24 notes 20 and 15).

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{RightPanel, ViewMode, Zoom};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

#[test]
fn the_shot_info_strip_stays_when_zoomed_in_and_hides_for_tools() {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let id = h.app.session.visible_cloned()[0];
    h.request("engine.execute", json!({"command": "library.select", "params": {"ids": [id.0]}}), T);
    h.app.ui.view = ViewMode::Detail;
    h.app.ui.right = RightPanel::Edit;
    h.app.ui.shot_info = true;
    h.step_until(Duration::from_secs(60), |h| h.app.widgets.iter().any(|(w, _)| w == "canvas:image"));
    h.settle(T);
    let fit = widget(&h, "loupe:shotInfo").expect("the strip at Fit");
    let img = widget(&h, "canvas:image").expect("the photo");
    assert!(fit.top() >= img.bottom() - 1.0, "at Fit, under the photo: {fit:?} vs {img:?}");
    h.app.ui.zoom = Zoom::Percent(100.0);
    h.settle(T);
    let canvas = h.app.canvas_rect.expect("the canvas");
    let img = widget(&h, "canvas:image").expect("the photo");
    assert!(img.height() > canvas.height(), "zoomed in: {img:?} in {canvas:?}");
    let strip = widget(&h, "loupe:shotInfo").expect("the strip is still there zoomed in");
    assert!(canvas.contains_rect(strip), "on screen: {strip:?} in {canvas:?}");
    assert!(strip.bottom() > canvas.bottom() - 40.0, "along the bottom edge: {strip:?} in {canvas:?}");
    // a tool on the pixels: nothing over them
    h.app.ui.tool = "brush".into();
    h.app.ui.right = RightPanel::Masking;
    h.step();
    h.step();
    assert!(widget(&h, "loupe:shotInfo").is_none(), "hidden while a tool is armed");
    // back when the tool is put away
    h.app.ui.tool.clear();
    h.app.ui.right = RightPanel::Edit;
    h.step();
    h.step();
    assert!(widget(&h, "loupe:shotInfo").is_some(), "back once the tool is put away");
}
