//! Test notes 2026-10-31 note 3: brushing at 100 % with Auto Sync on three photos. The stroke is
//! painted live on the photo being brushed; the other photos get it once, when the button goes
//! up, and their renders are background work that never takes the view's turn.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Event, PointerButton, Pos2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::MaskShape;
use serde_json::json;

use crate::headless::Headless;
use crate::render::Slot;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
}

fn strokes(h: &Headless, id: PhotoId) -> usize {
    let d = h.app.session.develop_of(id).unwrap_or_default();
    d.masks
        .iter()
        .flat_map(|m| m.components.iter())
        .map(|c| match &c.shape {
            MaskShape::Brush { strokes } => strokes.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn a_long_stroke_with_auto_sync_reaches_the_other_photos_once_when_it_ends() {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let ids: Vec<PhotoId> = h.app.session.visible_cloned().iter().take(3).copied().collect();
    assert_eq!(ids.len(), 3, "the demo library has three photos to brush");
    exec(&mut h, "library.select", json!({"ids": ids.iter().map(|p| p.0).collect::<Vec<_>>(), "active": ids[0].0}));
    exec(&mut h, "develop.autoSync", json!({"on": true}));
    exec(&mut h, "panel.masking", json!({}));
    exec(&mut h, "tool.brush", json!({"new": true}));
    exec(&mut h, "view.navigate", json!({"zoom": {"percent": 100}, "pan": [0.5, 0.5]}));
    h.settle(SETTLE);
    assert!(h.app.image_rect.zip(h.app.canvas_rect).is_some_and(|(i, c)| i.width() > c.width()), "the photo is zoomed in past the view");

    let c = h.app.canvas_rect.expect("the canvas").shrink(80.0);
    let point = |i: usize, n: usize| -> Pos2 {
        let t = i as f32 / n as f32;
        egui::pos2(c.left() + t * c.width(), c.center().y + 60.0 * (t * 12.0).sin())
    };
    h.app.renderer.submitted.clear();
    let n = 90;
    let p0 = point(0, n);
    h.app.synthetic.extend([
        Event::PointerMoved(p0),
        Event::PointerButton { pos: p0, button: PointerButton::Primary, pressed: true, modifiers: Default::default() },
    ]);
    h.step();
    let mut ms = Vec::new();
    for i in 1..=n {
        h.app.synthetic.push(Event::PointerMoved(point(i, n)));
        let t0 = Instant::now();
        h.step();
        ms.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    // while the button is held, the other photos are left alone: no edit, no render
    for id in &ids[1..] {
        assert_eq!(strokes(&h, *id), 0, "photo {id:?} gets nothing mid-stroke");
    }
    let mid: Vec<_> = h.app.renderer.submitted.iter().filter(|(_, p)| *p != ids[0]).collect();
    assert!(mid.is_empty(), "no render of another photo mid-stroke: {mid:?}");

    let end = point(n, n);
    h.app.synthetic.push(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    h.settle(SETTLE);
    for id in &ids {
        assert_eq!(strokes(&h, *id), 1, "photo {id:?} has the stroke, once");
    }
    // the others' renders came after the stroke, as background work, each once
    let mut per: HashMap<(Slot, PhotoId), usize> = HashMap::new();
    for (s, p) in h.app.renderer.submitted.iter().filter(|(_, p)| *p != ids[0]) {
        *per.entry((*s, *p)).or_default() += 1;
    }
    for ((s, p), k) in &per {
        assert!(matches!(s, Slot::Thumb(_) | Slot::ThumbQuick(_) | Slot::Prefetch(_)), "{p:?} is rendered in the background, not as {s:?}");
        assert_eq!(*k, 1, "{p:?} is rendered once for the stroke ({s:?}): {per:?}");
    }
    ms.sort_by(f64::total_cmp);
    let median = ms.get(ms.len() / 2).copied().unwrap_or(0.0);
    let max = ms.last().copied().unwrap_or(0.0);
    eprintln!("note 3: stroke frames median {median:.1} ms, max {max:.1} ms; renders of the others after it {per:?}");
    assert!(median <= 16.0, "stroke frame median {median} ms");
}
