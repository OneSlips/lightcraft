//! The retouch of people's parts: each section changes only its own part, as its sliders say, the
//! same at any size; nothing without a part found, or with its eye off.

use lightcraft_develop::{DevelopSettings, PartRequest, RetouchAnalysis, RetouchMask, RetouchPart, SegMask, SkinFace};
use lightcraft_geom::Orientation;
use lightcraft_raster::Rgb32f;

use crate::geometry::Frame;
use crate::retouch;

/// Settings `s` with part `part` (of everyone) found where `inside(x, y)` (normalized) on a 256 grid,
/// one face at the centre with its eyes 0.2 of the long edge apart.
pub(crate) fn with_part(mut s: DevelopSettings, part: RetouchPart, inside: impl Fn(f64, f64) -> bool) -> DevelopSettings {
    let side = 256;
    let logits: Vec<f32> = (0..side * side)
        .map(|i| {
            let (x, y) = (((i % side) as f64 + 0.5) / side as f64, ((i / side) as f64 + 0.5) / side as f64);
            if inside(x, y) { 12.0 } else { -12.0 }
        })
        .collect();
    let mask = RetouchMask { part, person: 0, seg: Some(SegMask::from_logits(side, &logits)) };
    let mut a = s.retouch_analysis.take().unwrap_or(RetouchAnalysis { orientation: Orientation::Normal, masks: vec![], faces: vec![] });
    a.masks.retain(|m| m.part != part);
    a.masks.push(mask);
    a.faces = vec![SkinFace { x: 0.5, y: 0.5, eyes: 0.2 }];
    s.retouch_analysis = Some(a);
    s
}

pub(crate) fn run(img: &mut Rgb32f, s: &DevelopSettings) {
    let frame = Frame::new(img.width, img.height, s, true);
    let ppl = frame.px_per_long(img.width);
    retouch::apply(img, s, &frame, ppl);
}

pub(crate) fn lum(p: [f32; 3]) -> f32 {
    0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2]
}

/// log2 of red over green, and of green over blue: how warm and how yellow a pixel is.
pub(crate) fn warmth(p: [f32; 3]) -> (f32, f32) {
    ((p[0] / p[1]).log2(), (p[1] / p[2]).log2())
}

/// A disc of radius 0.15 at (0.35, 0.5): the part.
pub(crate) fn disc(x: f64, y: f64) -> bool {
    (x - 0.35).hypot(y - 0.5) < 0.15
}

#[test]
fn whiten_takes_the_yellow_out_of_the_teeth_and_nothing_else() {
    // yellowish teeth on the left, the same colour elsewhere (lips, skin, a wall: never touched)
    let teeth = [0.62f32, 0.55, 0.36];
    let before = Rgb32f::from_fn(600, 400, |_, _| teeth);
    let mut s = DevelopSettings::default();
    s.retouch.teeth.whiten = 100.0;
    s.retouch.teeth.brighten = 50.0;
    let s = with_part(s, RetouchPart::Teeth, disc);
    let mut after = before.clone();
    run(&mut after, &s);
    let (i, o) = (after.data[200 * 600 + 210], after.data[200 * 600 + 500]);
    let (w0, y0) = warmth(teeth);
    let (w1, y1) = warmth(i);
    assert!(y1 < 0.5 * y0 && w1 < 0.6 * w0, "less yellow: {y0} → {y1}, {w0} → {w1}");
    assert!(lum(i) > lum(teeth) * 1.15, "brighter: {} → {}", lum(teeth), lum(i));
    assert_eq!(o, teeth, "outside the teeth nothing changes");
    // a gentle Whiten alone barely brightens
    let mut s2 = s.clone();
    s2.retouch.teeth = lightcraft_develop::RetouchTeeth { whiten: 30.0, ..Default::default() };
    let mut gentle = before.clone();
    run(&mut gentle, &s2);
    let g = gentle.data[200 * 600 + 210];
    assert!(lum(g) / lum(teeth) < 1.05 && warmth(g).1 < y0, "{g:?}");
}

#[test]
fn nothing_changes_while_off_without_a_part_or_with_its_eye_off() {
    let px = [0.62f32, 0.55, 0.36];
    let before = Rgb32f::from_fn(300, 200, |_, _| px);
    let mut on = DevelopSettings::default();
    on.retouch.teeth.whiten = 80.0;
    let found = with_part(on.clone(), RetouchPart::Teeth, disc);
    let mut eye_off = found.clone();
    eye_off.set_section_enabled("retouch.teeth", false);
    let mut all_off = found.clone();
    all_off.set_section_enabled("retouch", false);
    let mut turned = found.clone();
    turned.orientation = Orientation::Rotate90;
    let mut not_there = on.clone();
    not_there.retouch_analysis = Some(RetouchAnalysis {
        orientation: Orientation::Normal,
        masks: vec![RetouchMask { part: RetouchPart::Teeth, person: 0, seg: None }],
        faces: vec![],
    });
    let mut other_person = found.clone();
    other_person.retouch.teeth.person = 2;
    for (what, s) in [
        ("off", with_part(DevelopSettings::default(), RetouchPart::Teeth, disc)),
        ("not looked for", on),
        ("eye", eye_off),
        ("Retouch's eye", all_off),
        ("turned since", turned),
        ("none found", not_there),
        ("another person", other_person),
    ] {
        let mut img = before.clone();
        run(&mut img, &s);
        assert_eq!(img.data, before.data, "{what}");
        assert!(retouch::key(&s).is_none() && !retouch::active(&s), "{what}");
    }
    assert!(retouch::key(&found).is_some() && retouch::active(&found));
    let _ = PartRequest { part: RetouchPart::Teeth, person: 0 };
}

#[test]
fn a_preview_and_an_export_change_the_part_alike() {
    // the same photo at two sizes: the change at the same normalized point matches
    let px = [0.62f32, 0.55, 0.36];
    let mut s = DevelopSettings::default();
    s.retouch.teeth.whiten = 70.0;
    s.retouch.teeth.brighten = 40.0;
    let s = with_part(s, RetouchPart::Teeth, disc);
    let at = |w: usize| {
        let mut img = Rgb32f::from_fn(w, w * 2 / 3, |_, _| px);
        run(&mut img, &s);
        img.data[(w / 3) * w + (w * 35 / 100)]
    };
    let (small, big) = (at(300), at(1800));
    for k in 0..3 {
        assert!((small[k] - big[k]).abs() < 0.01, "{small:?} vs {big:?}");
    }
}
