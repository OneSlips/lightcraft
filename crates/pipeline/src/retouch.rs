//! Retouch of the parts of people (beyond Lightroom): teeth, eyes, lips, hair… Each part found on
//! the photo ([`RetouchAnalysis`]) is changed by its section's sliders ([`Retouch`]), blended in by
//! the part's coverage, so nothing outside it changes. It runs on the scene-linear image right
//! after Smooth Skin (before exposure, tone and colour), on the CPU for both renderers, so a preview
//! and an export match.
//!
//! The changes are made in log luminance and log colour ratios (`R/G`, `B/G`; neutral is 0), the
//! same terms Smooth Skin works in: a brightness change is a change of stops, a whiter tooth has
//! less yellow (red over green and green over blue moved towards neutral).

use lightcraft_develop::{DevelopSettings, PartRequest, Retouch, RetouchAnalysis, RetouchPart};
use lightcraft_geom::Point;
use lightcraft_raster::{Plane, Rgb32f, par_rows};

use crate::geometry::Frame;
use crate::masks::SegGrid;
use crate::skin::{fast_exp2, shifted, to_lab};

/// Pixels with less coverage than this are left alone.
const MIN_COVERAGE: f32 = 1.0 / 256.0;
/// Cells whose logit is above this bound the work (sigmoid ≈ 0.02).
const MIN_LOGIT: f32 = -4.0;

/// The parts this render retouches, each with its stored segmentation (none when off, its eye is
/// off, or nothing was found for it).
fn parts(s: &DevelopSettings) -> Vec<(PartRequest, &lightcraft_develop::SegMask)> {
    let Some(a) = analysis(s) else { return Vec::new() };
    let wanted = s.retouch.wanted(|k| s.retouch_section_on(&format!("retouch.{k}")));
    wanted.into_iter().filter_map(|r| Some((r, a.mask(r)?.seg.as_ref()?))).collect()
}

/// The stored analysis, when it is for the photo as it is turned now.
fn analysis(s: &DevelopSettings) -> Option<&RetouchAnalysis> {
    if !s.section_enabled("retouch") || s.retouch.is_default() {
        return None;
    }
    s.retouch_analysis.as_ref().filter(|a| a.orientation == s.orientation)
}

/// Whether the retouch changes this render (it runs on the CPU, after Smooth Skin).
pub fn active(s: &DevelopSettings) -> bool {
    !parts(s).is_empty()
}

/// What the retouch's result depends on (`None` when it does nothing): a stage-cache key.
pub fn key(s: &DevelopSettings) -> Option<u64> {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    let parts = parts(s);
    if parts.is_empty() {
        return None;
    }
    let a = analysis(s)?;
    let faces: Vec<[u64; 3]> = a.faces.iter().map(|f| [f.x.to_bits(), f.y.to_bits(), f.eyes.to_bits()]).collect();
    let segs: Vec<(PartRequest, u32, &str, Option<[u64; 4]>)> =
        parts.iter().map(|(r, m)| (*r, m.side, m.data.as_str(), m.rect.map(|q| q.map(f64::to_bits)))).collect();
    Some(BuildHasherDefault::<DefaultHasher>::default().hash_one((format!("{:?}", s.retouch), segs, faces)))
}

/// How far (output pixels) the retouch reads around a pixel it changes: a windowed render must
/// hold this much more around itself to match the whole render.
pub fn reach(_s: &DevelopSettings, _ppl: f64) -> f64 {
    0.0
}

/// A part's change of one pixel's log luminance and log colour ratios (`v`).
type Shift = [f32; 3];

/// The change section settings `r` make to part `part` at a pixel `v` (log luminance, log `R/G`,
/// log `B/G`); `None` when it changes nothing.
fn shift(part: RetouchPart, r: &Retouch, v: [f32; 3]) -> Shift {
    let f = |x: f64| if x.is_finite() { (x / 100.0).clamp(-1.0, 1.0) as f32 } else { 0.0 };
    let [_, a, b] = v;
    match part {
        RetouchPart::Teeth => {
            let (w, br) = (f(r.teeth.whiten).max(0.0), f(r.teeth.brighten).max(0.0));
            // yellow is red over green and blue under green: both moved towards neutral, and a
            // whiter tooth reads a touch brighter
            [0.45 * br + 0.12 * w, -0.6 * w * a.max(0.0), -0.75 * w * b.min(0.0)]
        }
    }
}

/// Apply the retouch to `img` (the output-resolution, scene-linear image `frame` draws, before
/// exposure), `ppl` output pixels per unit of the long edge. Does nothing when nothing was found
/// or the stored parts are damaged.
pub fn apply(img: &mut Rgb32f, s: &DevelopSettings, frame: &Frame, ppl: f64) {
    let (w, h) = (img.width, img.height);
    if w < 2 || h < 2 || !(ppl.is_finite() && ppl > 0.0) {
        return;
    }
    for (req, seg) in parts(s) {
        let Some(grid) = SegGrid::new(seg) else { continue };
        let Some((x0, y0, cover)) = crate::timed("retouch coverage", || coverage(&grid, frame, w, h)) else { continue };
        let r = s.retouch;
        let cw = cover.width;
        let rows = img.data.get_mut(y0 * w..(y0 + cover.height) * w).unwrap_or(&mut []);
        crate::timed("retouch", || {
            par_rows(rows, w, |y, row| {
                for x in 0..cw {
                    let m = cover.data.get(y * cw + x).copied().unwrap_or(0.0);
                    if m < MIN_COVERAGE {
                        continue;
                    }
                    let Some(px) = row.get(x0 + x).copied() else { continue };
                    let Some(v) = to_lab(px) else { continue };
                    let [dl, da, db] = shift(req.part, &r, v);
                    let out = shifted(px, dl, da, db);
                    if let Some(o) = row.get_mut(x0 + x) {
                        *o = std::array::from_fn(|k| px[k] + m * (out[k] - px[k]));
                    }
                }
            })
        });
    }
}

/// The coverage of `grid` over the output pixels it touches: the box's top-left corner and the
/// coverage there (`None` when it covers nothing of the output).
fn coverage(grid: &SegGrid, frame: &Frame, w: usize, h: usize) -> Option<(usize, usize, Plane)> {
    let b = grid.bounds_above(MIN_LOGIT)?;
    let to_out = frame.norm_to_out(w, h);
    let corners = [Point::new(b[0], b[1]), Point::new(b[2], b[1]), Point::new(b[0], b[3]), Point::new(b[2], b[3])].map(|c| to_out.apply(c));
    let (bx0, bx1) = corners.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c.x), hi.max(c.x)));
    let (by0, by1) = corners.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c.y), hi.max(c.y)));
    let cx = |v: f64| if v.is_finite() { v.clamp(0.0, w as f64) as usize } else { 0 };
    let cy = |v: f64| if v.is_finite() { v.clamp(0.0, h as f64) as usize } else { 0 };
    let (x0, x1, y0, y1) = (cx(bx0.floor() - 1.0), cx(bx1.ceil() + 1.0), cy(by0.floor() - 1.0), cy(by1.ceil() + 1.0));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let to_norm = frame.out_to_norm(w, h);
    let (mw, mh) = (x1 - x0, y1 - y0);
    let mut cover = Plane::new(mw, mh);
    par_rows(&mut cover.data, mw, |y, row| {
        let p0 = to_norm.apply(Point::new(x0 as f64 + 0.5, (y0 + y) as f64 + 0.5));
        let p1 = to_norm.apply(Point::new(x0 as f64 + 1.5, (y0 + y) as f64 + 0.5));
        let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
        for (x, v) in row.iter_mut().enumerate() {
            let n = Point::new(p0.x + dx * x as f64, p0.y + dy * x as f64);
            *v = if grid.contains(n) { 1.0 / (1.0 + fast_exp2(-grid.logit(n) * std::f32::consts::LOG2_E)) } else { 0.0 };
        }
    });
    Some((x0, y0, cover))
}
