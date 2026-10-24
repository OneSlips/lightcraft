//! The whole masking workflow as a photographer uses it, headless: settings before the first mask,
//! one press of a tool making one mask, switching tools, effect presets before and after, carried-over
//! settings and Reset Settings on New Mask, Add / Subtract with every kind, and when the overlay
//! shows.

use std::time::Duration;

use lightcraft_develop::{MaskOp, MaskShape};
use lightcraft_pipeline::Overlay;
use serde_json::json;

use crate::headless::Headless;
use crate::panels::detail::view_overlay;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn masking() -> Headless {
    masking_sized([1300.0, 900.0])
}

fn masking_sized(size: [f32; 2]) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, size, 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    exec(&mut h, "panel.masking", json!({}));
    h.step();
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
    h.step();
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

/// A drag on the photo from `a` to `b` (normalized).
fn drag(h: &mut Headless, a: (f64, f64), b: (f64, f64)) {
    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let r = h.request(
        "ui.pointer",
        json!({"events": [{"kind": "down", "x": a.0, "y": a.1}, {"kind": "drag", "x": mid.0, "y": mid.1}, {"kind": "drag", "x": b.0, "y": b.1}, {"kind": "up", "x": b.0, "y": b.1}]}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    h.step();
}

fn tap(h: &mut Headless, at: (f64, f64)) {
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": at.0, "y": at.1}, {"kind": "up", "x": at.0, "y": at.1}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
}

fn radial_centres(h: &Headless) -> Vec<(f64, f64)> {
    develop(h)
        .masks
        .iter()
        .flat_map(|m| m.components.iter())
        .filter_map(|c| match c.shape {
            MaskShape::Radial { center, .. } => Some((center.x, center.y)),
            _ => None,
        })
        .collect()
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 0.03 && (a.1 - b.1).abs() < 0.03
}

fn selected_adjust(h: &Headless) -> lightcraft_develop::LocalAdjustments {
    let d = develop(h);
    d.masks.iter().find(|m| Some(m.id) == h.app.session.active_mask).map(|m| m.adjust).expect("a selected mask")
}

/// Settings before the first mask: the sliders and the effect show with no mask, and the first
/// mask drawn takes them; it then shows its effect, not the overlay.
#[test]
fn settings_set_before_drawing_go_to_the_next_mask() {
    let mut h = masking();
    assert!(develop(&h).masks.is_empty());
    assert!(has(&h, "maskNextSettings"), "the panel says the settings are for the next mask");
    assert!(has(&h, "slider:exposure"), "the sliders show with no mask");
    assert!(has(&h, "check:maskResetNew"));
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 0.8}}));
    assert_eq!(h.app.session.mask_next.exposure, 0.8);
    assert!(develop(&h).masks.is_empty(), "no mask is made by setting a slider");
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.3, 0.3), (0.42, 0.4));
    let d = develop(&h);
    assert_eq!(d.masks.len(), 1);
    assert_eq!(d.masks[0].adjust.exposure, 0.8, "the new mask has the settings set beforehand");
    assert!(near(radial_centres(&h)[0], (0.3, 0.3)), "drawn where dragged: {:?}", radial_centres(&h));
    // a mask with settings shows its effect: no overlay by itself
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None);
}

/// One press of a tool = one mask: after one radial the tool is put away; a drag or a click
/// elsewhere makes no other mask (a drag on the mask moves it); pressing the tool again draws the
/// next one, where it is dragged. The same for Linear; Brush strokes paint the one mask its tile
/// made, and the tile again starts a new one.
#[test]
fn one_press_of_a_tool_makes_one_mask() {
    let mut h = masking();
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.25, 0.3), (0.33, 0.36));
    assert_eq!(develop(&h).masks.len(), 1);
    assert_eq!(h.app.ui.tool, "", "the tool is put away after its mask");
    assert!(h.app.ui.mask_new.is_none());
    // drags and clicks away from it make no copies
    drag(&mut h, (0.7, 0.7), (0.78, 0.76));
    tap(&mut h, (0.6, 0.2));
    // with no tool armed a click zooms (note 23 of 2026-10-24): back to Fit for the next tap
    assert!(matches!(h.app.ui.zoom, crate::state::Zoom::Percent(_)), "a click with no tool zooms in");
    h.app.ui.zoom = crate::state::Zoom::Fit;
    h.settle(Duration::from_secs(60));
    tap(&mut h, (0.1, 0.8));
    h.app.ui.zoom = crate::state::Zoom::Fit;
    h.settle(Duration::from_secs(60));
    assert_eq!(develop(&h).masks.len(), 1, "no mask from clicks or drags elsewhere");
    assert_eq!(develop(&h).masks[0].components.len(), 1, "and no part added to it");
    assert!(near(radial_centres(&h)[0], (0.25, 0.3)), "{:?}", radial_centres(&h));
    // a drag inside it moves it
    drag(&mut h, (0.25, 0.3), (0.35, 0.4));
    let c = radial_centres(&h);
    assert_eq!(c.len(), 1);
    assert!(near(c[0], (0.35, 0.4)), "moved: {c:?}");
    // the tile pressed again: the next one, where dragged, and the tool is put away again
    click(&mut h, "maskNew:radial");
    assert_eq!(h.app.ui.tool, "radial");
    drag(&mut h, (0.7, 0.7), (0.78, 0.76));
    let c = radial_centres(&h);
    assert_eq!(c.len(), 2);
    assert!(near(c[0], (0.35, 0.4)) && near(c[1], (0.7, 0.7)), "each where it was dragged: {c:?}");
    assert_eq!(h.app.ui.tool, "");
    drag(&mut h, (0.5, 0.15), (0.56, 0.2));
    assert_eq!(radial_centres(&h).len(), 2, "still no third");
    // Linear: one per press too
    click(&mut h, "maskNew:linear");
    drag(&mut h, (0.1, 0.1), (0.1, 0.3));
    drag(&mut h, (0.8, 0.5), (0.9, 0.5));
    click(&mut h, "maskNew:linear");
    drag(&mut h, (0.8, 0.5), (0.9, 0.5));
    let linears: Vec<_> = develop(&h)
        .masks
        .iter()
        .flat_map(|m| m.components.iter())
        .filter_map(|c| match c.shape {
            MaskShape::Linear { start, .. } => Some((start.x, start.y)),
            _ => None,
        })
        .collect();
    assert_eq!(linears.len(), 2, "{linears:?}");
    assert!(near(linears[0], (0.1, 0.1)) && near(linears[1], (0.8, 0.5)), "{linears:?}");
    // a click (a radial is placed by one) after its mask: still one
    click(&mut h, "maskNew:radial");
    tap(&mut h, (0.5, 0.5));
    let n = develop(&h).masks.len();
    tap(&mut h, (0.2, 0.6));
    assert_eq!(develop(&h).masks.len(), n, "a click places one radial, the next click none");
    // (that click, with no tool armed, zoomed in: note 23 of 2026-10-24) back to Fit
    h.app.ui.zoom = crate::state::Zoom::Fit;
    h.settle(Duration::from_secs(60));
    // Brush: strokes go into its mask; the tile again starts a new one
    click(&mut h, "maskNew:brush");
    drag(&mut h, (0.2, 0.8), (0.3, 0.8));
    let n = develop(&h).masks.len();
    drag(&mut h, (0.2, 0.85), (0.3, 0.85));
    assert_eq!(develop(&h).masks.len(), n, "a second stroke paints the same mask");
    click(&mut h, "maskNew:brush");
    drag(&mut h, (0.6, 0.85), (0.7, 0.85));
    assert_eq!(develop(&h).masks.len(), n + 1, "the tile again: a new brush mask");
    // the tile pressed twice before drawing puts the tool away
    click(&mut h, "maskNew:radial");
    click(&mut h, "maskNew:radial");
    assert_eq!(h.app.ui.tool, "", "{}", h.app.ui.tool);
}

/// Carried-over settings vs Reset Settings on New Mask; effect presets before and after a mask.
#[test]
fn new_masks_carry_the_last_settings_unless_reset_is_on() {
    let mut h = masking();
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.3, 0.3), (0.4, 0.4));
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1.2, "clarity": 30}}));
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.75, 0.7), (0.82, 0.76));
    let a = selected_adjust(&h);
    assert_eq!((a.exposure, a.clarity), (1.2, 30.0), "the next mask starts with the last mask's settings");
    // Reset settings on new mask: the next one starts at zero (and it is remembered)
    click(&mut h, "check:maskResetNew");
    assert!(h.app.ui.mask_reset_new && h.app.session.mask_reset_new);
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.15, 0.75), (0.22, 0.8));
    assert_eq!(selected_adjust(&h), lightcraft_develop::LocalAdjustments::default());
    // …except for settings set on purpose with no mask selected
    exec(&mut h, "mask.select", json!({"id": null}));
    h.step();
    exec(&mut h, "mask.adjust", json!({"values": {"shadows": 40}}));
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.5, 0.15), (0.55, 0.2));
    assert_eq!(selected_adjust(&h).shadows, 40.0);
    // effect presets: one saved, applied with no mask (for the next one), then to a mask
    exec(&mut h, "mask.adjust", json!({"values": {"temp": 25}}));
    exec(&mut h, "mask.saveEffect", json!({"name": "Warm"}));
    exec(&mut h, "mask.select", json!({"id": null}));
    let r = exec(&mut h, "mask.applyEffect", json!({"name": "Warm"}));
    assert_eq!(r["masks"], json!([]), "no mask: the effect is for the next one");
    click(&mut h, "maskNew:linear");
    drag(&mut h, (0.1, 0.4), (0.1, 0.6));
    assert_eq!(selected_adjust(&h).temp, 25.0);
    exec(&mut h, "mask.adjust", json!({"values": {"temp": 0}}));
    exec(&mut h, "mask.applyEffect", json!({"name": "Warm"}));
    assert_eq!(selected_adjust(&h).temp, 25.0, "and to the selected mask");
    click(&mut h, "check:maskResetNew");
    assert!(!h.app.session.mask_reset_new);
}

/// Note 5 (the repeat offender): moving or reshaping a mask that has settings never flashes the
/// overlay, not even with the pointer on the pin being dragged; the effect shows instead. Once the
/// drag ends, hovering the pin shows it again. A mask without settings shows its overlay while it
/// is dragged (nothing else to see yet). A local curve counts as a setting.
#[test]
fn dragging_a_mask_with_settings_never_shows_the_overlay() {
    let mut h = masking();
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.3, 0.3), (0.4, 0.4));
    let mid = h.app.session.active_mask.expect("selected");
    let press = |h: &mut Headless, at: (f64, f64), to: (f64, f64)| {
        let r = h.request(
            "ui.pointer",
            json!({"events": [{"kind": "down", "x": at.0, "y": at.1}, {"kind": "drag", "x": (at.0 + to.0) / 2.0, "y": (at.1 + to.1) / 2.0}, {"kind": "drag", "x": to.0, "y": to.1}]}),
            T,
        );
        assert_eq!(r["ok"], true, "{r}");
        h.step();
    };
    let release = |h: &mut Headless, at: (f64, f64)| {
        let r = h.request("ui.pointer", json!({"events": [{"kind": "up", "x": at.0, "y": at.1}]}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.step();
    };
    // no settings: dragging it shows what it selects
    let centre = radial_centres(&h)[0];
    let to = (centre.0 + 0.05, centre.1 + 0.05);
    press(&mut h, centre, to);
    assert!(crate::panels::detail::mask_gesture(&h.app), "the pin is being dragged: {:?}", h.app.gesture);
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }), "no settings: shown while dragged");
    release(&mut h, to);
    // with settings: never while dragged, even with the pointer on the pin
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1}}));
    let centre = radial_centres(&h)[0];
    let to = (centre.0 - 0.05, centre.1);
    press(&mut h, centre, to);
    assert!(crate::panels::detail::mask_gesture(&h.app), "the pin is being dragged: {:?}", h.app.gesture);
    h.app.ui.mask_hover = Some(mid);
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None, "settings: no overlay while dragged, even over the pin");
    release(&mut h, to);
    assert!(!crate::panels::detail::mask_gesture(&h.app));
    assert!(!near(radial_centres(&h)[0], centre), "the drag moved it");
    h.app.ui.mask_hover = Some(mid);
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }), "hovering the pin after the drag shows it");
    h.app.ui.mask_hover = None;
    h.app.ui.mask_hover_prev = None;
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None);
    // a local curve alone is a setting too
    let mut m = develop(&h).masks[0].clone();
    m.adjust = lightcraft_develop::LocalAdjustments::default();
    assert!(!crate::panels::detail::mask_has_settings(&m));
    m.curve.master = vec![lightcraft_geom::Point::new(0.0, 0.0), lightcraft_geom::Point::new(0.5, 0.6), lightcraft_geom::Point::new(1.0, 1.0)];
    assert!(crate::panels::detail::mask_has_settings(&m), "a local curve is a setting");
}

/// When the overlay shows: a fresh mask (no settings) shows what it selects; a mask with settings
/// (carried over or a preset) doesn't by itself, only while its row or pin is under the pointer
/// (and nothing is being adjusted); O shows it whatever the settings.
#[test]
fn the_overlay_shows_by_itself_only_for_a_mask_without_settings() {
    let mut h = masking();
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.3, 0.3), (0.4, 0.4));
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }), "fresh: shown");
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1}}));
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None, "adjusted: the effect shows");
    // the next one starts with those settings: no overlay by itself, even with its tool in use
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.7, 0.7), (0.8, 0.78));
    assert_eq!(develop(&h).masks.len(), 2);
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None, "carried settings: no overlay");
    // the pointer on its row shows it
    let mid = h.app.session.active_mask.expect("selected");
    h.app.ui.mask_hover = Some(mid);
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }), "hover: shown");
    h.app.ui.mask_hover = None;
    h.app.ui.mask_hover_prev = None;
    // O shows it whatever the settings, and hides it again
    exec(&mut h, "view.maskOverlay", json!({"show": true}));
    assert!(matches!(view_overlay(&h.app, &develop(&h)), Overlay::Mask { .. }));
    exec(&mut h, "view.maskOverlay", json!({"show": false}));
    assert_eq!(view_overlay(&h.app, &develop(&h)), Overlay::None);
}

/// Add and Subtract offer every kind and each one works on a radial (the tools on the photo, the
/// automatic ones at once, the pickers under the mask); nothing fails silently or panics.
#[test]
fn add_and_subtract_work_with_every_kind() {
    // (tall: the mask's parts and their controls push Add / Subtract down the panel)
    let mut h = masking_sized([1300.0, 2400.0]);
    click(&mut h, "maskNew:radial");
    drag(&mut h, (0.4, 0.4), (0.55, 0.5));
    let comps = |h: &Headless| develop(h).masks[0].components.len();
    for (op, button) in [("add", "button:maskAddComp"), ("subtract", "button:maskSubComp")] {
        for (kind, _, _) in crate::panels::masking::MASK_KINDS {
            let first = develop(&h).masks[0].id;
            exec(&mut h, "mask.select", json!({"id": first}));
            h.step();
            let before = comps(&h);
            click(&mut h, button);
            assert!(
                has(&h, &format!("maskComp:{op}:{kind}")),
                "{op} offers {kind}: {:?}",
                h.app.widgets.iter().filter(|(w, _)| w.starts_with("maskComp") || w.starts_with("button:mask")).collect::<Vec<_>>()
            );
            click(&mut h, &format!("maskComp:{op}:{kind}"));
            match kind {
                "brush" | "linear" | "radial" => drag(&mut h, (0.2, 0.2), (0.3, 0.3)),
                "colorRange" | "luminanceRange" => tap(&mut h, (0.5, 0.5)),
                "people" => click(&mut h, "maskPeople:create"),
                "landscape" => click(&mut h, "maskLandscape:sky"),
                // Object and Describe offer their model (no models here); Remembered has nothing remembered
                _ => {}
            }
            h.app.ui.dialog = None;
            h.app.ui.describe = None;
            h.app.ui.concept_pick = false;
            h.app.ui.mask_pick_op = None;
            h.app.ui.tool.clear();
            h.app.ui.mask_new = None;
            // a tap with no tool armed zooms (note 23 of 2026-10-24): the next kind starts at Fit
            h.app.ui.zoom = crate::state::Zoom::Fit;
            h.step();
            let after = comps(&h);
            let made = !matches!(kind, "object" | "prompt" | "concept");
            if made {
                assert_eq!(after, before + 1, "{op} {kind} adds a part");
                let c = develop(&h).masks[0].components.last().cloned().expect("a part");
                let want = if op == "add" { MaskOp::Add } else { MaskOp::Subtract };
                assert_eq!(c.op, want, "{op} {kind}");
            }
            assert_eq!(develop(&h).masks.len(), 1, "{op} {kind} combines with the mask, never a new one");
        }
    }
}
