//! Headless tests of the brush (TEST-NOTES 2026-10-28): ⌥ / Alt tracked through a stroke (note 19), the
//! eraser's own settings (note 23) and a Size that is a size on the screen (note 27).

use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2};
use lightcraft_develop::{BrushStroke, MaskShape};
use serde_json::json;

use crate::headless::Headless;
use crate::panels::detail::{BrushRun, brush_runs};
use crate::state::{Zoom, brush_core_points};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
const ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: false, command: false };

fn detail() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    for (c, p) in [("ui.set", json!({"view": "detail"})), ("engine.execute", json!({"command": "panel.masking"}))] {
        let r = h.request(c, p, T);
        assert_eq!(r["ok"], true, "{r}");
    }
    h
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

/// The photo position (fractions of the photo on screen) on the screen.
fn at(h: &Headless, x: f32, y: f32) -> Pos2 {
    let r = h.app.image_rect.expect("a photo on screen");
    egui::pos2(r.left() + x * r.width(), r.top() + y * r.height())
}

/// One frame with `events`.
fn frame(h: &mut Headless, events: Vec<Event>) {
    h.app.synthetic.extend(events);
    h.step();
}

/// One gesture, a frame per step: `Some((x, y))` moves the pointer there (the first press), `None` toggles ⌥ /
/// Alt; `alt_at_release` is the key's state when the button goes up.
fn gesture(h: &mut Headless, steps: &[Option<(f32, f32)>]) {
    let mut alt = false;
    let mut last = None;
    for s in steps {
        match s {
            Some((x, y)) => {
                let p = at(h, *x, *y);
                let mods = if alt { ALT } else { Modifiers::NONE };
                if last.is_none() {
                    frame(
                        h,
                        vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: mods }],
                    );
                } else {
                    frame(h, vec![Event::PointerMoved(p)]);
                }
                last = Some(p);
            }
            None => {
                alt = !alt;
                frame(h, vec![Event::ModifiersChanged(if alt { ALT } else { Modifiers::NONE })]);
            }
        }
    }
    let p = last.expect("a press");
    let mods = if alt { ALT } else { Modifiers::NONE };
    frame(h, vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: mods }]);
    frame(h, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    h.step();
}

fn strokes(h: &Headless) -> Vec<BrushStroke> {
    let id = h.app.session.active().expect("active photo");
    let d = h.app.session.develop_of(id).unwrap_or_default();
    match d.masks.first().and_then(|m| m.components.iter().find(|c| matches!(c.shape, MaskShape::Brush { .. }))).map(|c| &c.shape) {
        Some(MaskShape::Brush { strokes }) => strokes.clone(),
        _ => Vec::new(),
    }
}

/// Each run continues where the one before it ended.
fn assert_continuous(s: &[BrushStroke]) {
    for w in s.windows(2) {
        assert_eq!(w[1].points.first(), w[0].points.last(), "{s:?}");
    }
}

// Note 19: the runs of a gesture, unit by unit
#[test]
fn a_gesture_cuts_into_runs_of_one_mode() {
    let p = |x: f64| lightcraft_geom::Point::new(x, 0.5);
    let pts: Vec<_> = (0..6).map(|i| p(i as f64 / 10.0)).collect();
    let runs = brush_runs(&pts, &[], &[false, false, true, true, false, true]);
    let want = vec![
        BrushRun { points: vec![p(0.0), p(0.1)], pressure: vec![], erase: false },
        BrushRun { points: vec![p(0.1), p(0.2), p(0.3)], pressure: vec![], erase: true },
        BrushRun { points: vec![p(0.3), p(0.4)], pressure: vec![], erase: false },
        BrushRun { points: vec![p(0.4), p(0.5)], pressure: vec![], erase: true },
    ];
    assert_eq!(runs, want);
    // pen pressure follows its points; a short or missing mode list takes the last mode
    let runs = brush_runs(&pts[..3], &[0.2, 0.4, 0.6], &[true]);
    assert_eq!(runs, vec![BrushRun { points: pts[..3].to_vec(), pressure: vec![0.2, 0.4, 0.6], erase: true }]);
    assert!(brush_runs(&[], &[], &[]).is_empty());
}

// Note 19: paint, ⌥ down, ⌥ up, ⌥ down, the button let go with ⌥ still held: four runs, paint / erase / paint /
// erase, each continuing where the last ended (one undo step); before, the whole stroke took the mode ⌥ had
// at the release
#[test]
fn alt_is_tracked_through_the_stroke() {
    for live in [false, true] {
        let mut h = detail();
        exec(&mut h, "tool.brush", json!({"new": true}));
        // a dab first (the mask exists from then on)
        gesture(&mut h, &[Some((0.8, 0.2))]);
        if live {
            // a mask with settings shows its effect while painted: the stroke goes to the photo as it grows
            exec(&mut h, "mask.adjust", json!({"values": {"exposure": -1.0}}));
        }
        let s: Vec<Option<(f32, f32)>> = vec![
            Some((0.2, 0.5)),
            Some((0.25, 0.5)),
            Some((0.3, 0.5)),
            None,
            Some((0.35, 0.5)),
            Some((0.4, 0.5)),
            None,
            Some((0.45, 0.5)),
            Some((0.5, 0.5)),
            None,
            Some((0.55, 0.5)),
            Some((0.6, 0.5)),
        ];
        gesture(&mut h, &s);
        let got = strokes(&h);
        assert_eq!(got.iter().map(|s| s.erase).collect::<Vec<_>>(), [false, false, true, false, true], "live {live}: {got:?}");
        assert_continuous(&got[1..]);
        assert!(!h.app.ui.brush_erase, "the brush mode is unchanged");
        exec(&mut h, "edit.undo", json!({}));
        assert_eq!(strokes(&h).len(), 1, "one undo step takes the whole gesture back");
        h.settle(SETTLE);
    }
}

// Note 19: ⌥ let go a frame before the button changes nothing when the pointer didn't move in between, and only
// the last bit when it did; a short ⌥ stroke over paint still erases; with Erase picked, ⌥ paints
#[test]
fn letting_go_of_alt_before_the_button_changes_only_the_last_bit() {
    let mut h = detail();
    exec(&mut h, "tool.brush", json!({"new": true}));
    gesture(&mut h, &[Some((0.2, 0.5)), Some((0.4, 0.5))]);
    // ⌥ down, a stroke, ⌥ up, then the button
    gesture(&mut h, &[None, Some((0.25, 0.5)), Some((0.3, 0.5)), Some((0.35, 0.5)), None]);
    let got = strokes(&h);
    assert_eq!(got.iter().map(|s| s.erase).collect::<Vec<_>>(), [false, true], "{got:?}");
    // ⌥ up, one more step, then the button: only that step paints
    gesture(&mut h, &[None, Some((0.25, 0.6)), Some((0.3, 0.6)), Some((0.35, 0.6)), None, Some((0.36, 0.6))]);
    let got = strokes(&h);
    assert_eq!(got.iter().map(|s| s.erase).collect::<Vec<_>>(), [false, true, true, false], "{got:?}");
    assert_eq!(got[3].points.len(), 2, "the last step: {:?}", got[3].points);
    // Erase picked: ⌥ toggles to painting the same way
    h.app.ui.brush_erase = true;
    gesture(&mut h, &[Some((0.5, 0.3)), Some((0.55, 0.3)), None, Some((0.6, 0.3)), Some((0.65, 0.3))]);
    let got = strokes(&h);
    assert_eq!(got[4..].iter().map(|s| s.erase).collect::<Vec<_>>(), [true, false], "{got:?}");
    assert_continuous(&got[4..]);
    h.settle(SETTLE);
}

// Note 23: the eraser keeps its own Size / Feather / Flow / Density, used while erasing (⌥ held or Erase
// picked) and changed by the keys then; both are remembered
#[test]
fn the_eraser_keeps_its_own_settings() {
    let mut h = detail();
    exec(&mut h, "tool.brush", json!({"new": true}));
    (h.app.ui.brush_size, h.app.ui.brush_feather, h.app.ui.brush_flow, h.app.ui.brush_density) = (20.0, 30.0, 70.0, 90.0);
    (h.app.ui.erase_size, h.app.ui.erase_feather, h.app.ui.erase_flow, h.app.ui.erase_density) = (40.0, 80.0, 50.0, 60.0);
    gesture(&mut h, &[Some((0.2, 0.5)), Some((0.3, 0.5)), None, Some((0.35, 0.5)), Some((0.4, 0.5))]);
    let got = strokes(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    let (paint, erase) = (&got[0], &got[1]);
    assert_eq!((paint.feather, paint.flow, paint.density), (30.0, 70.0, 90.0));
    assert_eq!((erase.feather, erase.flow, erase.density), (80.0, 50.0, 60.0));
    let ratio = erase.size / paint.size;
    let want = f64::from(brush_core_points(40.0) / brush_core_points(20.0));
    assert!((ratio - want).abs() < 1e-6, "{ratio} vs {want}");
    // ] with ⌥ held grows the eraser, not the brush
    h.app.ui.brush_alt = true;
    let _ = h.app.run("brush.larger", json!({}));
    assert_eq!((h.app.ui.brush_size, h.app.ui.erase_size), (20.0, 48.0));
    h.app.ui.brush_alt = false;
    let _ = h.app.run("brush.larger", json!({}));
    assert_eq!((h.app.ui.brush_size, h.app.ui.erase_size), (24.0, 48.0));
    // remembered across restarts
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_str::<crate::state::UiState>(&saved).unwrap().sanitized();
    assert_eq!((back.erase_size, back.erase_feather, back.erase_flow, back.erase_density), (48.0, 80.0, 50.0, 60.0));
    assert_eq!(back.brush_density, 90.0);
    h.settle(SETTLE);
}

// Note 27: Size is a size on the screen: the cursor is as big at 100 % as at Fit, and a stroke painted at 100 %
// stores a smaller radius in the photo than one painted at Fit, by the zoom ratio
#[test]
fn brush_size_is_a_screen_size() {
    let mut h = detail();
    exec(&mut h, "tool.brush", json!({"new": true}));
    h.app.ui.brush_size = 20.0;
    // after the first click its pin sits at the centre, and a pin wins over the brush within a few points (note 28),
    // so the cursor and the second stroke stay off it
    let cursor = |h: &mut Headless, x: f32| {
        let p = at(h, x, 0.5);
        frame(h, vec![Event::PointerMoved(p)]);
        h.step();
        h.app.widgets.iter().find(|w| w.0 == "brushCursor:paint").map(|w| w.1.width()).expect("the brush cursor")
    };
    let long_of = |h: &Headless| h.app.image_rect.map(|r| r.width().max(r.height())).expect("a photo on screen");
    let fit = cursor(&mut h, 0.5);
    let long_fit = long_of(&h);
    gesture(&mut h, &[Some((0.5, 0.5))]);
    h.app.ui.zoom = Zoom::Percent(100.0);
    for _ in 0..4 {
        h.step();
    }
    let at_100 = cursor(&mut h, 0.55);
    let long_100 = long_of(&h);
    assert!(long_100 > long_fit * 1.2, "100 % is closer than Fit here: {long_fit} vs {long_100}");
    assert!((fit - at_100).abs() < 0.5, "the cursor keeps its size on the screen: {fit} vs {at_100}");
    gesture(&mut h, &[Some((0.55, 0.5))]);
    let got = strokes(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    let ratio = got[0].size / got[1].size;
    let want = f64::from(long_100 / long_fit);
    assert!((ratio - want).abs() < 0.01 * want, "{ratio} vs {want}");
    h.settle(SETTLE);
}
