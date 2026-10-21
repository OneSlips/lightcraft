//! Unit tests of the on-canvas mask and spot tools: gradient handles, the radial feather ring,
//! the live Auto Mask stroke's timing, the one-outline spots, the crop view's turn.

use egui::{Pos2, Rect, pos2, vec2};
use lightcraft_develop::{DevelopSettings, MaskShape};
use lightcraft_geom::Point;
use lightcraft_pipeline::geometry::Frame;

use super::*;

/// A 400 × 300 photo drawn at 400 × 300 points from (100, 50).
fn map() -> CanvasMap {
    let frame = Frame::new(400, 300, &DevelopSettings::default(), true);
    CanvasMap::new(&frame, Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 300.0)))
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
}

#[test]
fn turning_a_linear_gradient_keeps_its_middle_and_length() {
    let m = map();
    let shape = MaskShape::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 0.5) };
    // from right of the middle to below it: a quarter turn clockwise on screen
    let (press, at) = (Point::new(0.75, 0.5), Point::new(0.5, 0.5 + 100.0 / 300.0));
    let MaskShape::Linear { start, end } = moved_shape(&shape, ROTATE, Point::new(at.x - press.x, at.y - press.y), at, &m, false) else { panic!() };
    let (a, b) = (m.screen(start), m.screen(end));
    assert!((a.lerp(b, 0.5) - pos2(300.0, 200.0)).length() < 0.1, "the middle stays");
    assert!((a.distance(b) - 200.0).abs() < 0.1, "the length stays");
    assert!((a.x - b.x).abs() < 0.1 && a.y < b.y, "now vertical, start above: {a:?} {b:?}");
}

#[test]
fn dragging_a_linear_line_widens_the_fade_without_turning_it() {
    let m = map();
    let shape = MaskShape::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 0.5) };
    // the start line dragged left and down: only the leftward part counts
    let (press, at) = (Point::new(0.25, 0.2), Point::new(0.1, 0.4));
    let MaskShape::Linear { start, end } = moved_shape(&shape, LINE_START, Point::new(at.x - press.x, at.y - press.y), at, &m, false) else {
        panic!()
    };
    assert!(close(start, Point::new(0.1, 0.5)), "{start:?}");
    assert!(close(end, Point::new(0.75, 0.5)), "the end stays");
    let MaskShape::Linear { end, .. } = moved_shape(&shape, LINE_END, Point::new(0.05, 0.3), Point::new(0.8, 0.8), &m, false) else { panic!() };
    assert!(close(end, Point::new(0.8, 0.5)), "{end:?}");
}

#[test]
fn which_part_of_a_gradient_a_press_grabs() {
    let m = map();
    let linear = MaskShape::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 0.5) };
    // the lines at x = 200 (start), 300 (middle), 400 (end) on screen
    assert_eq!(gradient_part(&linear, pos2(300.0, 120.0), &m), Some(ROTATE));
    assert_eq!(gradient_part(&linear, pos2(202.0, 300.0), &m), Some(LINE_START));
    assert_eq!(gradient_part(&linear, pos2(398.0, 60.0), &m), Some(LINE_END));
    assert_eq!(gradient_part(&linear, pos2(250.0, 200.0), &m), Some(0), "between the lines: moves");
    assert_eq!(gradient_part(&linear, pos2(480.0, 200.0), &m), None, "beyond the end line: not grabbed (a drag there pans)");
    assert_eq!(gradient_part(&linear, pos2(120.0, 200.0), &m), None, "before the start line too");
    let radial = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.25, ry: 0.25, angle: 0.0, feather: 50.0, invert: false };
    // radius 100 points round (300, 200)
    assert_eq!(gradient_part(&radial, pos2(300.0, 150.0), &m), Some(0), "inside: moves");
    assert_eq!(gradient_part(&radial, pos2(408.0, 200.0), &m), Some(ROTATE), "just outside: turns");
    assert_eq!(gradient_part(&radial, pos2(480.0, 200.0), &m), None, "far outside: nothing");
}

#[test]
fn the_feather_ring_sets_the_feather_and_the_slider_moves_the_ring() {
    let m = map();
    let radial = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.25, ry: 0.25, angle: 0.0, feather: 50.0, invert: false };
    // the ring at half the radius; dragged out to 80 % of it: feather 20
    let at = Point::new(0.5 + 0.8 * 100.0 / 400.0, 0.5);
    let MaskShape::Radial { feather, rx, .. } = moved_shape(&radial, 6, Point::new(0.0, 0.0), at, &m, false) else { panic!() };
    assert!((feather - 20.0).abs() < 0.6 && rx == 0.25, "{feather}");
    // the ring is drawn where the feather says
    assert!((radial_inner(20.0) - 0.8).abs() < 1e-9);
    assert!((radial_inner(f64::NAN) - 0.5).abs() < 1e-9, "a hostile value draws a ring anyway");
    let q = radial_point(&m, Point::new(0.5, 0.5), 0.25, 0.25, 0.0, radial_inner(20.0), 0.0);
    assert!((q - pos2(380.0, 200.0)).length() < 0.1, "{q:?}");
}

#[test]
fn turning_a_radial_gradient_follows_the_pointer_round_its_centre() {
    let m = map();
    let radial = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.25, ry: 0.1, angle: 10.0, feather: 50.0, invert: false };
    // a quarter turn clockwise from anywhere round the centre: +90°, from where it was
    let (press, at) = (Point::new(0.75, 0.5), Point::new(0.5, 0.5 + 100.0 / 300.0));
    for handle in [5, ROTATE] {
        let MaskShape::Radial { angle, .. } = moved_shape(&radial, handle, Point::new(at.x - press.x, at.y - press.y), at, &m, false) else {
            panic!()
        };
        assert!((angle - 100.0).abs() < 0.01, "{handle}: {angle}");
    }
}

#[test]
fn resize_cursors_follow_the_direction() {
    use egui::CursorIcon::*;
    assert_eq!(resize_cursor(vec2(1.0, 0.0)), ResizeHorizontal);
    assert_eq!(resize_cursor(vec2(-1.0, 0.05)), ResizeHorizontal);
    assert_eq!(resize_cursor(vec2(0.0, 1.0)), ResizeVertical);
    assert_eq!(resize_cursor(vec2(1.0, 1.0)), ResizeNwSe);
    assert_eq!(resize_cursor(vec2(1.0, -1.0)), ResizeNeSw);
    let _ = resize_cursor(vec2(f32::NAN, 0.0));
}

#[test]
fn the_live_auto_mask_stroke_goes_out_throttled() {
    assert!(!live_due(None, 1, 0.0), "a single point is not a stroke yet");
    assert!(live_due(None, 2, 0.0));
    assert!(!live_due(Some((1.0, 10)), 12, 1.05), "too soon");
    assert!(live_due(Some((1.0, 10)), 12, 1.0 + LIVE_AUTO_MASK_EVERY));
    assert!(!live_due(Some((1.0, 10)), 10, 5.0), "nothing new");
    assert!(live_due(Some((1.0, 10)), 11, f64::NAN), "a broken clock never stalls it");
}

#[test]
fn a_painted_spot_has_one_outline_not_a_ring_per_dab() {
    // two dabs far apart: two whole circles
    let arcs = union_arcs(&[pos2(0.0, 0.0), pos2(100.0, 0.0)], 10.0);
    assert_eq!(arcs.len(), 2);
    // a stroke of 200 samples along a line: its outline is two long sides and two round ends, and
    // no point of it lies inside the painted area
    let line: Vec<Pos2> = (0..200).map(|i| pos2(i as f32, 0.0)).collect();
    let arcs = union_arcs(&line, 10.0);
    let points: usize = arcs.iter().map(Vec::len).sum();
    assert!(points < 1200, "{points} outline points for 200 samples");
    for q in arcs.iter().flatten() {
        let inside = line.iter().filter(|c| c.distance(*q) < 9.0).count();
        assert_eq!(inside, 0, "{q:?} is inside the stroke");
    }
    // hostile input draws nothing and never panics
    assert!(union_arcs(&[pos2(f32::NAN, 0.0)], 10.0).is_empty());
    assert!(union_arcs(&line, f32::INFINITY).is_empty());
    assert!(union_arcs(&line, 0.0).is_empty());
}

/// The outline of a long painted stroke is drawn every frame while painting: it is found through a
/// grid, not by testing every dab against every other (4,096 dabs took ~16 M distance tests a frame).
#[test]
fn a_long_stroke_outline_is_quick() {
    // a scribble of 4,000 dabs going back and forth
    let stroke: Vec<Pos2> = (0..4000).map(|i| pos2((i % 400) as f32 * 3.0, (i / 400) as f32 * 12.0)).collect();
    let t = std::time::Instant::now();
    let arcs = union_arcs(&stroke, 8.0);
    let took = t.elapsed();
    assert!(!arcs.is_empty());
    for q in arcs.iter().flatten() {
        assert!(stroke.iter().all(|c| c.distance(*q) >= 7.4), "{q:?} is inside the stroke");
    }
    assert!(took < std::time::Duration::from_millis(400), "{took:?}");
}

#[test]
fn the_crop_view_turns_the_photo_level_and_fits_it() {
    let mut d = DevelopSettings::default();
    d.crop.geometry.angle = 10.0;
    let frame = Frame::new(400, 300, &d, false);
    let img = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let turn = crop_view_turn(&d, &frame, img);
    assert!((turn.to_degrees().abs() - 10.0).abs() < 0.01, "{}", turn.to_degrees());
    // turned that way, the box is level
    let map = CanvasMap::new(&frame, img).turned(turn);
    let q = frame_crop_quad(&d, &frame);
    let (a, b) = (map.screen(q[0]), map.screen(q[1]));
    assert!((a.y - b.y).abs() < 0.01, "{a:?} {b:?}");
    // and the turned photo fits the area it was fitted to
    let fitted = fit_turned(img, img, turn);
    let (s, c) = turn.sin_cos();
    assert!(fitted.width() * c.abs() + fitted.height() * s.abs() <= 400.0 + 0.01);
    assert!(fitted.width() * s.abs() + fitted.height() * c.abs() <= 300.0 + 0.01);
    // a level crop doesn't turn anything
    assert_eq!(crop_view_turn(&DevelopSettings::default(), &Frame::new(400, 300, &DevelopSettings::default(), false), img), 0.0);
    // the mapping turns back exactly
    let p = pos2(123.0, 77.0);
    assert!((map.screen(map.norm(p)) - p).length() < 0.01);
}
