//! The one-line slider row (test notes 9 and 25): its parts, its height, and real egui frames
//! for reset, typing, nudges and hostile values.
use super::{SliderOut, row_parts, slider, take_registry};
use crate::headless::HeadlessView;
use egui::{Event, Key, Modifiers, PointerButton, Rect, pos2, vec2};
use lightcraft_develop::{ControlSpec, controls};

struct Row {
    view: HeadlessView,
    spec: ControlSpec,
    value: f64,
    time: f64,
    width: f32,
}

impl Row {
    fn new(id: &str, value: f64) -> Self {
        let mut r = Row { view: HeadlessView::new(), spec: *controls::find(id).unwrap(), value, time: 0.0, width: 300.0 };
        for _ in 0..2 {
            r.frame(vec![]);
        }
        r
    }
    fn frame(&mut self, events: Vec<Event>) -> SliderOut {
        self.time += 0.02;
        let mut out = SliderOut::default();
        let (spec, value) = (self.spec, self.value);
        self.view.run(HeadlessView::raw_input(vec2(self.width, 100.0), 1.0, self.time, events), |ui| {
            out = slider(ui, &spec, value, true, None);
        });
        if let Some(v) = out.value {
            self.value = v;
        }
        out
    }
    fn rect(&self, part: &str) -> Rect {
        let name = format!("{part}:{}", self.spec.id);
        take_registry(&self.view.ctx).into_iter().find(|(id, _)| *id == name).map(|(_, r)| r).unwrap_or_else(|| panic!("no {name}"))
    }
    fn parts(&mut self) -> (Rect, Rect, Rect) {
        self.frame(vec![]);
        let registry = take_registry(&self.view.ctx);
        let find = |p: &str| registry.iter().find(|(id, _)| *id == format!("{p}:{}", self.spec.id)).unwrap().1;
        (find("sliderLabel"), find("slider"), find("sliderValue"))
    }
    fn click(&mut self, at: egui::Pos2, count: usize) -> Vec<SliderOut> {
        let mut outs = vec![self.frame(vec![Event::PointerMoved(at)])];
        for _ in 0..count {
            for pressed in [true, false] {
                outs.push(self.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]));
            }
        }
        outs.push(self.frame(vec![]));
        outs
    }
    fn key(&mut self, key: Key) -> SliderOut {
        self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }])
    }
}

#[test]
fn a_slider_is_one_row_of_the_dense_height() {
    let mut r = Row::new("light.exposure", 0.0);
    let (label, track, value) = r.parts();
    let h = crate::theme::Tokens::default().slider_row_h;
    assert!((18.0..=22.0).contains(&h), "row height {h}");
    for (name, part) in [("name", label), ("track", track), ("value", value)] {
        assert!((part.height() - h).abs() < 0.5, "{name} spans the row: {part:?}");
        assert!((part.top() - label.top()).abs() < 0.5, "{name} on the same line: {part:?}");
    }
    assert!(label.right() < track.left() && track.right() < value.left(), "name | track | value: {label:?} {track:?} {value:?}");
}

#[test]
fn row_parts_never_overlap_or_vanish_at_any_width() {
    for w in [0.0, 1.0, 10.0, 40.0, 120.0, 250.0, 270.0, 520.0, 4000.0] {
        for name in [0.0, 40.0, 1e9, f32::NAN, f32::INFINITY, -5.0] {
            let p = row_parts(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, 20.0)), name);
            assert!(p.track.width() >= 1.0, "{w}: a track to divide by: {p:?}");
            assert!(p.label.right() <= p.track.left(), "{w}: {p:?}");
            for r in [p.label, p.track, p.value] {
                assert!(r.min.x.is_finite() && r.max.x.is_finite() && r.width() >= 0.0, "{w}: {p:?}");
            }
        }
    }
}

#[test]
fn double_clicking_the_name_resets() {
    let mut r = Row::new("light.contrast", 40.0);
    let (label, _, _) = r.parts();
    let outs = r.click(label.left_center() + vec2(6.0, 0.0), 2);
    assert!(outs.iter().any(|o| o.reset), "a reset");
    assert_eq!(r.value, 0.0);
}

#[test]
fn dragging_the_name_moves_the_value_from_where_it_is() {
    let mut r = Row::new("light.contrast", 10.0);
    let (label, track, _) = r.parts();
    let from = label.center();
    r.frame(vec![Event::PointerMoved(from)]);
    r.frame(vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    // a quarter of the track to the right: a quarter of the range (200) up, from 10, not a jump
    let to = from + vec2(track.width() / 4.0, 0.0);
    for i in 1..=10 {
        r.frame(vec![Event::PointerMoved(from + (to - from) * (i as f32 / 10.0))]);
    }
    r.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert!((r.value - 60.0).abs() <= 3.0, "10 + 50: {}", r.value);
}

#[test]
fn typing_a_value_into_the_row_sets_it() {
    let mut r = Row::new("light.exposure", 0.0);
    let (_, _, value) = r.parts();
    r.click(value.center(), 1);
    r.frame(vec![Event::Text("1.5".into())]);
    let out = r.key(Key::Enter);
    assert_eq!(out.value, Some(1.5));
    assert_eq!(r.value, 1.5);
}

#[test]
fn arrow_keys_and_the_wheel_nudge_a_hovered_or_clicked_row() {
    let mut r = Row::new("light.contrast", 0.0);
    let (_, track, _) = r.parts();
    r.frame(vec![Event::PointerMoved(track.center())]);
    r.key(Key::ArrowUp);
    assert_eq!(r.value, 1.0, "↑ over the row");
    // a click arms the wheel; a notch down is a step down (the click itself set the value there)
    r.click(track.center(), 1);
    let before = r.value;
    r.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -120.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    for _ in 0..40 {
        r.frame(vec![]);
    }
    assert!(r.value < before, "the wheel nudged it down: {before} → {}", r.value);
}

#[test]
fn hostile_values_and_widths_never_panic() {
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300] {
        for w in [0.0f32, 3.0, 300.0] {
            let mut r = Row::new("wb.temp", 6500.0);
            r.width = w.max(1.0);
            r.value = v;
            let out = r.frame(vec![]);
            assert!(out.value.is_none_or(|x| x.is_finite()), "{v} at {w}: {out:?}");
            // a press and drag over whatever is there
            let at = pos2(w / 2.0, 10.0);
            r.frame(vec![Event::PointerMoved(at)]);
            r.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
            let out = r.frame(vec![Event::PointerMoved(at + vec2(30.0, 0.0))]);
            assert!(out.value.is_none_or(|x| x.is_finite()), "{v} at {w}: {out:?}");
            r.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        }
    }
    // the registry is still there after all that
    let r = Row::new("light.exposure", 0.0);
    let _ = r.rect("slider");
}
