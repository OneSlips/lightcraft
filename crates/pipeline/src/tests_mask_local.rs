//! Pixel tests of a mask's local Curve and Point Color: under a mask covering the whole photo they
//! render exactly as the global ones; under a partial mask they stay inside it.

use lightcraft_develop::{DevelopSettings, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, PointColor, ToneCurve};
use lightcraft_geom::Point;
use lightcraft_raster::{Rgb32f, Rgba8};

use crate::{RenderRequest, SourceInfo, render};

const W: usize = 192;
const H: usize = 96;

/// Colours across the hue circle (x) and from dark to bright (y).
fn colours() -> Rgb32f {
    Rgb32f::from_fn(W, H, |x, y| {
        let h = x as f32 / W as f32 * std::f32::consts::TAU;
        let v = 0.03 + 0.5 * y as f32 / H as f32;
        [v * (1.0 + 0.6 * h.cos()), v * (1.0 + 0.6 * (h - 2.1).cos()), v * (1.0 + 0.6 * (h + 2.1).cos())]
    })
}

/// A mask from `start` (full) to `end` (none) of a linear gradient, with `curve` and `points`.
fn masked(start: f64, end: f64, curve: ToneCurve, points: Vec<PointColor>, amount: f64) -> DevelopSettings {
    let shape = MaskShape::Linear { start: Point::new(start, 0.5), end: Point::new(end, 0.5) };
    let mut s = DevelopSettings::default();
    s.masks.push(Mask {
        id: 1,
        components: vec![MaskComponent { anchor: None, name: None, op: MaskOp::Add, invert: false, shape }],
        adjust: LocalAdjustments { amount, ..Default::default() },
        curve,
        point_colors: points,
        ..Default::default()
    });
    s
}

fn shot(src: &Rgb32f, s: &DevelopSettings) -> Rgba8 {
    render(src, &SourceInfo::default(), s, &RenderRequest::fit(W, H)).image
}

fn s_curve() -> ToneCurve {
    let p = |x: f64, y: f64| Point::new(x, y);
    ToneCurve {
        master: vec![p(0.0, 0.0), p(0.25, 0.18), p(0.75, 0.84), p(1.0, 1.0)],
        red: vec![p(0.0, 0.05), p(0.5, 0.58), p(1.0, 1.0)],
        refine_saturation: 60.0,
        ..Default::default()
    }
}

fn orange_sample() -> PointColor {
    PointColor { lum: 0.6, chroma: 0.12, hue: 55.0, hue_shift: 40.0, sat_shift: -50.0, lum_shift: 30.0, variance: -20.0, ..Default::default() }
}

/// Columns `x0..x1` of two renders, as (identical, differing) pixel counts.
fn compare(a: &Rgba8, b: &Rgba8, x0: usize, x1: usize) -> (usize, usize) {
    let mut same = 0;
    for y in 0..H {
        for x in x0..x1 {
            same += usize::from(a.data[y * W + x] == b.data[y * W + x]);
        }
    }
    (same, (x1 - x0) * H - same)
}

#[test]
fn a_full_mask_curve_renders_as_the_global_curve() {
    let src = colours();
    let global = DevelopSettings { curve: s_curve(), ..Default::default() };
    let local = masked(2.0, 2.1, s_curve(), Vec::new(), 100.0);
    let (g, l) = (shot(&src, &global), shot(&src, &local));
    assert_eq!(g.data, l.data, "same curve, same maths");
    assert_ne!(g.data, shot(&src, &DevelopSettings::default()).data, "the curve does something");
}

#[test]
fn a_full_mask_point_color_renders_as_the_global_point_color() {
    let src = colours();
    let global = DevelopSettings { point_colors: vec![orange_sample()], ..Default::default() };
    let local = masked(2.0, 2.1, ToneCurve::default(), vec![orange_sample()], 100.0);
    let (g, l) = (shot(&src, &global), shot(&src, &local));
    let (_, differ) = compare(&g, &l, 0, W);
    assert_eq!(differ, 0, "same samples, same maths");
    let (_, changed) = compare(&g, &shot(&src, &DevelopSettings::default()), 0, W);
    assert!(changed > W * H / 20, "the sample changes its colours ({changed} px)");
}

#[test]
fn local_curve_and_point_color_stay_inside_the_mask() {
    let src = colours();
    let plain = shot(&src, &DevelopSettings::default());
    // full left of x = 0.45, nothing right of 0.46
    let s = masked(0.45, 0.46, s_curve(), vec![orange_sample()], 100.0);
    let img = shot(&src, &s);
    let (_, outside) = compare(&img, &plain, (W as f64 * 0.47) as usize, W);
    assert_eq!(outside, 0, "nothing changes outside the mask");
    let (_, inside) = compare(&img, &plain, 0, (W as f64 * 0.44) as usize);
    assert!(inside > W * H / 4, "the mask's curve and colour apply inside ({inside} px)");
}

#[test]
fn half_amount_lands_between() {
    let src = colours();
    let plain = shot(&src, &DevelopSettings::default());
    let full = shot(&src, &masked(2.0, 2.1, s_curve(), Vec::new(), 100.0));
    let half = shot(&src, &masked(2.0, 2.1, s_curve(), Vec::new(), 50.0));
    let mut between = 0;
    let mut moved = 0;
    for ((p, f), h) in plain.data.iter().zip(&full.data).zip(&half.data) {
        for k in 0..3 {
            let (p, f, h) = (p[k] as i32, f[k] as i32, h[k] as i32);
            if (p - f).abs() > 6 {
                moved += 1;
                between += usize::from((h - p).signum() == (f - p).signum() && (h - p).abs() < (f - p).abs());
            }
        }
    }
    assert!(moved > 100 && between * 10 >= moved * 9, "{between} of {moved} channels between");
}

#[test]
fn black_and_white_ignores_local_point_color_as_it_does_global() {
    let src = colours();
    let mut bw = masked(2.0, 2.1, ToneCurve::default(), vec![orange_sample()], 100.0);
    bw.treatment = lightcraft_develop::Treatment::Bw;
    let plain = DevelopSettings { treatment: lightcraft_develop::Treatment::Bw, ..Default::default() };
    assert_eq!(shot(&src, &bw).data, shot(&src, &plain).data);
}

/// A bright subject (a white dress: scene values from its folds, a stop and a half under middle
/// grey, to near white, in rows) under a mask covering it.
fn dress() -> Rgb32f {
    Rgb32f::from_fn(W, H, |_, y| {
        let v = 0.06 + 0.74 * y as f32 / (H - 1) as f32;
        [v * 0.98, v, v * 1.02]
    })
}

fn tone_masked(adjust: LocalAdjustments) -> DevelopSettings {
    let mut s = masked(2.0, 2.1, ToneCurve::default(), Vec::new(), 100.0);
    s.masks[0].adjust = LocalAdjustments { amount: 100.0, ..adjust };
    s
}

/// The mean green value of rows `y0..y1`.
fn rows(img: &Rgba8, y0: usize, y1: usize) -> f64 {
    let px = img.data[y0 * W..y1 * W].iter();
    px.map(|p| p[1] as f64).sum::<f64>() / ((y1 - y0) * W) as f64
}

/// Item 25 (evening retest): a mask's Whites did nothing on a bright dress. They move its
/// brightest pixels clearly (more than its mid tones), and Blacks its darkest ones more than its
/// brightest.
#[test]
fn mask_whites_and_blacks_move_a_bright_subjects_tones() {
    let src = dress();
    let plain = shot(&src, &DevelopSettings::default());
    let whites = shot(&src, &tone_masked(LocalAdjustments { whites: 100.0, ..Default::default() }));
    let darker = shot(&src, &tone_masked(LocalAdjustments { whites: -100.0, ..Default::default() }));
    let (top, low) = ((H * 3 / 4, H * 7 / 8), (0, H / 8));
    let lift = rows(&whites, top.0, top.1) - rows(&plain, top.0, top.1);
    assert!(lift > 8.0, "Whites +100 brightens the dress's brightest tones ({lift:.1} levels)");
    let drop = rows(&plain, top.0, top.1) - rows(&darker, top.0, top.1);
    assert!(drop > 15.0, "Whites -100 pulls them down ({drop:.1} levels)");
    let low_lift = rows(&whites, low.0, low.1) - rows(&plain, low.0, low.1);
    assert!(low_lift < lift, "the brightest move more than the darker parts ({low_lift:.1} vs {lift:.1})");
    let blacks = shot(&src, &tone_masked(LocalAdjustments { blacks: -100.0, ..Default::default() }));
    let deep = rows(&plain, low.0, low.1) - rows(&blacks, low.0, low.1);
    assert!(deep > 6.0, "Blacks -100 deepens the dress's darkest tones ({deep:.1} levels)");
    let top_deep = rows(&plain, top.0, top.1) - rows(&blacks, top.0, top.1);
    assert!(top_deep < deep, "…more than its brightest ({top_deep:.1} vs {deep:.1})");
}
