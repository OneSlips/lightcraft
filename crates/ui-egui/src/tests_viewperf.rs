//! Test notes 2026-10-28 notes 14, 15, 16 and 29: what an Optics drag shows, how soon a slider's
//! handle follows the pointer, ← / → through a selection, and what a brush stroke on a synced
//! selection costs and sets off.

use std::time::{Duration, Instant};

use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn detail() -> Headless {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

/// Selects the first `n` photos of the view (the first active), with Auto Sync on for more than one.
fn select_synced(h: &mut Headless, n: usize) -> Vec<PhotoId> {
    let ids: Vec<PhotoId> = h.app.session.visible_cloned().into_iter().take(n).collect();
    assert_eq!(ids.len(), n, "the demo has {n} photos");
    exec(h, "library.select", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
    if n > 1 {
        exec(h, "develop.autoSync", json!({"on": true}));
    }
    ids
}

fn press(h: &mut Headless, pos: egui::Pos2, pressed: bool) {
    h.app.synthetic.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
}

// ---------------------------------------------------------------- note 14

/// The mean brightness (0..255) of the loupe's picture in the headless frame.
fn loupe_brightness(h: &Headless) -> f64 {
    let img = h.app.image_rect.expect("the photo on screen");
    let shot = h.paint();
    let [w, ht] = shot.size;
    let (x0, x1) = ((img.left().max(0.0) as usize).min(w), (img.right().max(0.0) as usize).min(w));
    let (y0, y1) = ((img.top().max(0.0) as usize).min(ht), (img.bottom().max(0.0) as usize).min(ht));
    let (mut sum, mut n) = (0.0, 0.0);
    for y in (y0..y1).step_by(4) {
        for x in (x0..x1).step_by(4) {
            let c = shot.pixels[y * w + x];
            sum += f64::from(c.r()) + f64::from(c.g()) + f64::from(c.b());
            n += 3.0;
        }
    }
    if n > 0.0 { sum / n } else { 0.0 }
}

/// Note 14: dragging an Optics slider (a lens correction changes the warp) keeps the photo on screen:
/// the last render is drawn until the next draft arrives, never a black loupe.
#[test]
fn an_optics_drag_never_shows_a_black_loupe() {
    let mut h = detail();
    h.settle(SETTLE);
    let rest = loupe_brightness(&h);
    assert!(rest > 60.0, "the demo photo is on screen: {rest}");
    for (ctl, v) in [("optics.distortion", 30.0), ("optics.vignetting", 50.0), ("optics.defringePurple", 10.0), ("optics.caRed", 20.0)] {
        exec(&mut h, "develop.beginInteraction", json!({"label": ctl}));
        for k in 1..=6 {
            exec(&mut h, "develop.set", json!({"control": ctl, "value": v * f64::from(k) / 6.0}));
            h.step();
            let shown = h.app.loupe_shown.map(|l| l.1);
            assert_ne!(shown, Some("none"), "{ctl} step {k}: the loupe draws a picture");
            let b = loupe_brightness(&h);
            assert!(b > rest * 0.6, "{ctl} step {k}: not black ({b} against {rest} at rest)");
        }
        exec(&mut h, "develop.endInteraction", json!({}));
        h.settle(SETTLE);
    }
}

// ---------------------------------------------------------------- note 15

/// Note 15: a press on a slider's handle and a slow drag (1 point a frame) moves the value from the
/// first frame the pointer moves, by the pointer's travel (no jump); the drag is one undo step; a
/// click on the handle without a move changes nothing.
#[test]
fn a_slider_follows_a_slow_drag_from_its_first_move() {
    for n in [1usize, 6] {
        let mut h = detail();
        let ids = select_synced(&mut h, n);
        h.settle(SETTLE);
        let id = "light.exposure";
        let track = |h: &Headless| h.app.widgets.iter().find(|(w, _)| w == &format!("slider:{id}")).map(|(_, r)| *r).expect("the Exposure slider");
        let rect = track(&h);
        let value = |h: &Headless, p: PhotoId| lightcraft_develop::controls::get(&h.app.session.develop_of(p).unwrap(), id).unwrap_or(0.0);
        let undo = h.app.session.undo.len();
        // the handle sits at the middle of the track (Exposure 0); grabbed a little off its centre
        let pos = rect.center() + egui::vec2(5.0, 0.0);
        h.app.synthetic.push(egui::Event::PointerMoved(pos));
        press(&mut h, pos, true);
        h.step();
        let mut moved_at = None;
        for k in 1..=10 {
            h.app.synthetic.push(egui::Event::PointerMoved(pos + egui::vec2(k as f32, 0.0)));
            h.step();
            if moved_at.is_none() && value(&h, ids[0]).abs() > 1e-9 {
                moved_at = Some(k);
            }
        }
        assert!(moved_at.is_some_and(|k| k <= 2), "n={n}: the value moves with the first points of travel: {moved_at:?}");
        // 10 points of a 10 EV track; grabbed 5 points off the handle's centre, a jump to the pointer would add 5 more
        let want = 10.0 / f64::from(rect.width()) * 10.0;
        let jump = 5.0 / f64::from(rect.width()) * 10.0;
        let got = value(&h, ids[0]);
        assert!((got - want).abs() < jump / 2.0, "n={n}: by the pointer's travel, no jump to the pointer: {got} vs {want}");
        if n > 1 {
            assert!((value(&h, ids[1]) - got).abs() < 1e-9, "Auto Sync carries it live");
        }
        press(&mut h, pos + egui::vec2(10.0, 0.0), false);
        h.step();
        h.step();
        assert_eq!(h.app.session.undo.len(), undo + 1, "n={n}: one undo step");
        assert!(h.app.session.interaction.is_none());
        // a click on the handle without a move leaves it where it is
        let at = track(&h);
        let before = value(&h, ids[0]);
        let handle = egui::pos2(at.left() + ((before + 5.0) / 10.0) as f32 * at.width(), at.center().y);
        h.app.synthetic.push(egui::Event::PointerMoved(handle));
        press(&mut h, handle, true);
        h.step();
        press(&mut h, handle, false);
        h.step();
        h.step();
        assert_eq!(value(&h, ids[0]), before, "n={n}: a click on the handle moves nothing");
    }
}

// ---------------------------------------------------------------- note 16

/// Note 16 in the app: → and ← (keys) step through the selection in Detail and Grid, keep it, and wrap.
#[test]
fn arrow_keys_step_through_the_selection_in_detail_and_grid() {
    for view in ["detail", "grid"] {
        let mut h = detail();
        let r = h.request("ui.set", json!({"view": if view == "grid" { "photoGrid" } else { view }}), T);
        assert_eq!(r["ok"], true, "{r}");
        let vis = h.app.session.visible_cloned();
        let picked = [vis[1], vis[4], vis[7]];
        exec(&mut h, "library.select", json!({"ids": picked.iter().map(|i| i.0).collect::<Vec<_>>()}));
        exec(&mut h, "develop.autoSync", json!({"on": true}));
        let mut seen = Vec::new();
        for key in ["right", "right", "right", "left"] {
            let r = h.request("ui.key", json!({"key": key}), T);
            assert_eq!(r["ok"], true, "{r}");
            seen.push(h.app.session.active().unwrap());
            assert_eq!(h.app.session.selection.ids.len(), 3, "{view}: the selection is kept");
        }
        assert_eq!(seen, vec![picked[1], picked[2], picked[0], picked[2]], "{view}: within the selection, wrapping round");
        assert!(h.app.session.auto_sync, "{view}: Auto Sync stays on");
    }
}

// ---------------------------------------------------------------- note 29

/// A horizontal stroke across the part of the photo on screen, `steps` pointer moves, one frame each:
/// the frame times (ms) and the renders set off for photos other than `active` while it lasted.
fn stroke(h: &mut Headless, active: PhotoId, steps: usize) -> (Vec<f64>, usize) {
    // (zoomed in, the photo is larger than the canvas)
    let img = h.app.image_rect.expect("the photo on screen").intersect(h.app.canvas_rect.expect("the canvas"));
    let at = |x: f32| egui::pos2(img.left() + x * img.width(), img.top() + 0.5 * img.height());
    h.app.renderer.submitted.clear();
    let mut frames = Vec::new();
    h.app.synthetic.push(egui::Event::PointerMoved(at(0.3)));
    press(h, at(0.3), true);
    h.step();
    for i in 1..=steps {
        h.app.synthetic.push(egui::Event::PointerMoved(at(0.3 + 0.4 * i as f32 / steps as f32)));
        let t0 = Instant::now();
        h.step();
        frames.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    press(h, at(0.7), false);
    h.step();
    h.step();
    let others: Vec<String> = h.app.renderer.submitted.iter().filter(|(_, p)| *p != active).map(|(s, _)| format!("{s:?}")).collect();
    if !others.is_empty() {
        eprintln!("renders of other photos while painting: {others:?}");
    }
    let others = others.len();
    (frames, others)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn p95(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() * 95 / 100).copied().unwrap_or(0.0)
}

fn brush_strokes(h: &Headless, id: PhotoId) -> usize {
    h.app
        .session
        .develop_of(id)
        .map(|d| {
            d.masks
                .iter()
                .flat_map(|m| m.components.iter())
                .map(|c| match &c.shape {
                    lightcraft_develop::MaskShape::Brush { strokes } => strokes.len(),
                    _ => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// A brush mask with an effect (its strokes go to the photo live) on `n` selected photos, at 100 %.
fn brushing(n: usize) -> (Headless, Vec<PhotoId>) {
    let mut h = detail();
    let ids = select_synced(&mut h, n);
    exec(&mut h, "panel.masking", json!({}));
    exec(&mut h, "tool.brush", json!({"new": true}));
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": 1.0}}));
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": 0.2, "y": 0.2}, {"kind": "up", "x": 0.2, "y": 0.2}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    exec(&mut h, "view.zoom100", json!({}));
    h.settle(SETTLE);
    assert_eq!(h.app.ui.tool, "brush");
    (h, ids)
}

/// Note 29: six photos selected with Auto Sync, brushing stroke after stroke at 100 %: while the
/// strokes go on only the photo on screen renders (the others' thumbnails and preparation wait), each
/// stroke reaches every photo once as it ends, in one undo step, and the others' thumbnails follow
/// once the strokes pause. Frames stay within the 16 ms budget.
#[test]
fn brushing_a_synced_selection_renders_only_the_photo_on_screen() {
    let (mut h, ids) = brushing(6);
    let undo = h.app.session.undo.len();
    let mut frames = Vec::new();
    for k in 0..4 {
        // the next stroke starts most of a second after the last (the others' thumbnails used to start
        // in that gap and run during the stroke)
        for _ in 0..if k == 0 { 0 } else { 50 } {
            h.step();
        }
        let (f, others) = stroke(&mut h, ids[0], if k == 0 { 200 } else { 60 });
        assert_eq!(others, 0, "stroke {k}: no render of the other photos while painting");
        frames.extend(f);
        for id in &ids {
            assert_eq!(brush_strokes(&h, *id), 2 + k, "stroke {k}: photo {id:?} got the stroke once, as it ended");
        }
    }
    assert_eq!(h.app.session.undo.len(), undo + 4, "one undo step a stroke, for all six photos");
    let (m, p) = (median(frames.clone()), p95(frames));
    eprintln!("note 29: six photos, frames median {m:.1} ms, p95 {p:.1} ms");
    assert!(m <= 16.0, "frame time median {m} ms");
    // the strokes pause: the others' thumbnails follow (in the background)
    h.app.renderer.submitted.clear();
    for _ in 0..120 {
        h.step();
    }
    h.settle(SETTLE);
    for id in &ids[1..] {
        assert!(
            h.app.renderer.submitted.iter().any(|(s, p)| p == id && matches!(s, crate::render::Slot::Thumb(_))),
            "photo {id:?}: its thumbnail follows once the strokes pause"
        );
    }
}

/// Measures a long stroke with 1, 2 and 6 photos selected (run with `--ignored --nocapture`).
#[test]
#[ignore = "measurement"]
fn measure_brush_stroke_cost() {
    for n in [1, 2, 6] {
        let (mut h, ids) = brushing(n);
        let (first, o1) = stroke(&mut h, ids[0], 200);
        let mut next = Vec::new();
        let mut others = o1;
        for _ in 0..3 {
            let (f, o) = stroke(&mut h, ids[0], 60);
            next.extend(f);
            others += o;
        }
        eprintln!(
            "MEASURE brush n={n}: first stroke median {:.1} / p95 {:.1} ms; next strokes median {:.1} / p95 {:.1} ms; renders of other photos while painting {others}",
            median(first.clone()),
            p95(first),
            median(next.clone()),
            p95(next)
        );
    }
}

/// Note 29: while the photographer paints (and for a moment after each stroke) the thumbnails of the
/// other photos an edit reached wait; then they follow.
#[test]
fn while_painting_the_other_photos_thumbnails_wait() {
    let mut h = detail();
    let ids = select_synced(&mut h, 3);
    h.settle(SETTLE);
    assert!(h.app.renderer.thumb(ids[1]).is_some(), "the filmstrip has its thumbnail");
    h.app.renderer.submitted.clear();
    h.app.paint_hold_until = h.time() + 0.9;
    exec(&mut h, "develop.set", json!({"ids": [ids[1].0], "values": {"light.exposure": 1.0}}));
    for _ in 0..90 {
        h.app.paint_hold_until = h.time() + 0.9;
        h.step();
    }
    let early: Vec<_> = h.app.renderer.submitted.iter().filter(|(_, p)| *p == ids[1]).collect();
    assert!(early.is_empty(), "nothing for the other photo while painting: {early:?}");
    for _ in 0..120 {
        h.step();
    }
    h.settle(SETTLE);
    assert!(
        h.app.renderer.submitted.iter().any(|(s, p)| *p == ids[1] && matches!(s, crate::render::Slot::Thumb(_))),
        "its thumbnail follows once the painting pauses"
    );
}
