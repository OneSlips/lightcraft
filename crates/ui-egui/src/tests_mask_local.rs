//! Headless tests of a mask's Curve and Point Color sections in the Masking panel.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

/// The Masking panel on the demo photo with one radial mask, in a window tall enough to show
/// the mask's sections without scrolling.
fn with_mask() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1300.0, 3000.0], 1.0);
    for (method, params) in [
        ("ui.set", json!({"view": "detail"})),
        ("engine.execute", json!({"command": "panel.masking"})),
        ("engine.execute", json!({"command": "mask.add", "params": {"kind": "radial", "center": {"x": 0.5, "y": 0.5}, "rx": 0.3, "ry": 0.3}})),
    ] {
        let r = h.request(method, params, T);
        assert_eq!(r["ok"], true, "{method}: {r}");
    }
    h.settle(SETTLE);
    h
}

fn develop(h: &Headless) -> lightcraft_develop::DevelopSettings {
    let id = h.app.session.active().expect("active photo");
    (*h.app.session.develop_of(id).unwrap_or_default()).clone()
}

fn click(h: &mut Headless, id: &str, at: Option<(f64, f64)>) {
    let mut p = json!({"id": id});
    if let Some((fx, fy)) = at {
        p["fx"] = json!(fx);
        p["fy"] = json!(fy);
    }
    let r = h.request("ui.clickWidget", p, T);
    assert_eq!(r["ok"], true, "{id}: {r}");
}

#[test]
fn a_masks_curve_section_edits_its_own_curve() {
    let mut h = with_mask();
    // no parametric curve in a mask: its channels are RGB, red, green, blue
    let w = h.request("ui.hoverWidget", json!({"id": "maskCurveChannel:parametric"}), T);
    assert_eq!(w["ok"], false, "{w}");
    // double-clicking a channel straightens it (and only it)
    for ch in ["master", "green"] {
        let r = h.request(
            "engine.execute",
            json!({"command": "mask.curve", "params": {"channel": ch, "points": [[0.0, 0.0], [0.3, 0.2], [1.0, 1.0]]}}),
            T,
        );
        assert_eq!(r["ok"], true, "{r}");
    }
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": "maskCurveChannel:green", "count": 2}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let c = develop(&h).masks[0].curve.clone();
    assert!(c.green.is_empty() && c.master.len() == 3, "{c:?}");
    assert_eq!(h.app.ui.mask_curve_channel, "green");
    // a click above the diagonal adds a point to the mask's red curve
    click(&mut h, "maskCurveChannel:red", None);
    assert_eq!(h.app.ui.mask_curve_channel, "red");
    click(&mut h, "maskCurve", Some((0.5, 0.3)));
    h.settle(SETTLE);
    let d = develop(&h);
    assert_eq!(d.masks[0].curve.red.len(), 3, "{:?}", d.masks[0].curve);
    assert!(d.masks[0].curve.red[1].y > 0.6);
    assert!(d.curve.red.is_empty() && d.curve.master.is_empty(), "the photo's own curve is untouched");
}

#[test]
fn a_masks_point_color_section_samples_and_adjusts() {
    let mut h = with_mask();
    click(&mut h, "icon:maskPointColorPicker", None);
    assert_eq!(h.app.ui.tool, crate::panels::mask_local::PICK_TOOL);
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": 0.5, "y": 0.5}, {"kind": "up", "x": 0.5, "y": 0.5}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(develop(&h).masks[0].point_colors.len(), 1, "the click sampled a colour");
    assert!(h.app.ui.tool.is_empty(), "the eyedropper puts itself away");
    // the sample's swatch and sliders show; a slider edits the mask's sample
    for id in ["maskPointColor:0", "slider:maskPointColor.satShift", "slider:maskPointColor.lumRange"] {
        let r = h.request("ui.hoverWidget", json!({"id": id}), T);
        assert_eq!(r["ok"], true, "{id}: {r}");
    }
    click(&mut h, "slider:maskPointColor.satShift", Some((0.05, 0.5)));
    h.settle(SETTLE);
    let d = develop(&h);
    assert!(d.masks[0].point_colors[0].sat_shift < -50.0, "{:?}", d.masks[0].point_colors[0]);
    assert!(d.point_colors.is_empty(), "the photo's own Point Color is untouched");
    click(&mut h, "button:maskPointColorDelete", None);
    h.settle(SETTLE);
    assert!(develop(&h).masks[0].point_colors.is_empty());
}
