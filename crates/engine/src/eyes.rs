//! Eye focus for culling: each person's sharper eye, cut out at 100 % (one pixel of the photo per
//! pixel), with a measure of how sharp it is — what Capture One's Cull view offers as Face Focus.
//!
//! The picture is the camera's full-size embedded JPEG of a raw (what culling shows anyway, and a
//! fraction of a raw decode), or the file itself for other images. Faces are found with MediaPipe
//! BlazeFace on a downscaled copy and their eye contours with FaceMesh on a full-resolution crop
//! around each face (the Face Landmarker model, Apache-2.0, downloaded on demand: `ModelId::Face`).
//! Without the model, or in a build without AI masks, [`EyeScanner`] answers with an error the UI
//! turns into "Get the AI Model…".
//!
//! **Focus measure** ([`focus_score`]): the high-frequency energy of the eye region (the RMS of a
//! Laplacian over the eye contour's box, lashes and lid edges included), less the photo's noise at
//! that scale (the median absolute Laplacian of a patch of cheek below the eye, which has no detail
//! of its own), divided by the eye's own contrast (the standard deviation of its luminance), so a
//! dark eye in shade and a bright one in sun score alike. Mapped to 0–100 by [`sharpness`].
//!
//! Nothing here blocks the UI: [`EyeScanner`] runs on its own thread, newest wish first, and the
//! caller keeps the results (they are small: a few crops of 160 × 100 pixels).

use std::sync::Arc;

use lightcraft_catalog::PhotoId;
use lightcraft_raster::Rgba8;

/// What changes the results (bump when the measure or the crops change, so caches drop them).
pub const EYES_VERSION: u64 = 1;
/// At most this many people get an eye thumbnail (the largest faces).
pub const MAX_PEOPLE: usize = 8;
/// The eye crop's size in photo pixels (shown at 100 %).
pub const CROP_W: usize = 160;
pub const CROP_H: usize = 100;
/// An eye whose contour is less than this share of the other's width is turned away (its
/// lashes and lid edge seen side on): the other one is the eye to judge.
const HIDDEN_EYE: f32 = 0.5;
/// Long edge of the copy faces are looked for in.
#[cfg_attr(not(feature = "sam"), allow(dead_code))]
const DETECT_EDGE: usize = 1600;
/// How sharp an eye looks, for a dot in the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Focus {
    Sharp,
    Soft,
    Blurry,
}

/// [`sharpness`] at or above this is [`Focus::Sharp`]; below [`SOFT_FROM`], [`Focus::Blurry`].
/// Set by eye on 120 eyes of a wedding's EOS R5 C frames (camera JPEGs): crisp lashes and iris
/// rims read 42–74, eyes just behind the focal plane 30–42, people out of the depth of field
/// 0–29. Smooth young skin reads a little lower than lined skin at the same focus.
pub const SHARP_FROM: f32 = 42.0;
pub const SOFT_FROM: f32 = 30.0;

impl Focus {
    pub fn of(sharpness: f32) -> Focus {
        if !sharpness.is_finite() || sharpness < SOFT_FROM {
            Focus::Blurry
        } else if sharpness < SHARP_FROM {
            Focus::Soft
        } else {
            Focus::Sharp
        }
    }
}

/// Whether an eye is open, from [`openness`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Lids {
    Open,
    /// Half closed: a blink caught midway, heavy lids (a laughing squint usually stays open).
    Half,
    /// A blink.
    Closed,
    /// FaceMesh gave no lids to measure.
    Unknown,
}

/// [`openness`] below this is [`Lids::Closed`]; below [`HALF_BELOW`], [`Lids::Half`]. Set on
/// 400 eyes of a wedding's frames: blinks and lowered lids read 0.07–0.29 (FaceMesh draws closed
/// lids slightly apart), laughing squints and heavy lids 0.33–0.42, open eyes from 0.36.
pub const CLOSED_BELOW: f32 = 0.30;
pub const HALF_BELOW: f32 = 0.36;

impl Lids {
    pub fn of(openness: Option<f32>) -> Lids {
        match openness {
            Some(o) if o.is_finite() && o < CLOSED_BELOW => Lids::Closed,
            Some(o) if o.is_finite() && o < HALF_BELOW => Lids::Half,
            Some(o) if o.is_finite() => Lids::Open,
            _ => Lids::Unknown,
        }
    }

    /// Better first: open, unknown, half, closed.
    fn rank(self) -> u8 {
        match self {
            Lids::Open => 3,
            Lids::Unknown => 2,
            Lids::Half => 1,
            Lids::Closed => 0,
        }
    }
}

/// FaceMesh's lid points per eye (the image's left eye first, as `lightcraft_segment`'s `EYES`):
/// the corners, then two upper-lid points above two lower-lid points (the six-point eye aspect
/// ratio's points).
pub const LIDS: [([usize; 2], [usize; 2], [usize; 2]); 2] = [([33, 133], [160, 158], [144, 153]), ([263, 362], [387, 385], [373, 380])];
/// FaceMesh's iris points per eye: centre, then four on the rim.
pub const IRIS_POINTS: [(usize, [usize; 4]); 2] = [(468, [469, 470, 471, 472]), (473, [474, 475, 476, 477])];

/// How far eye `k` is open in FaceMesh landmarks `lm` (photo pixels): the mean gap between the
/// upper and lower lid over the iris's diameter (about 0.8-1.1 wide open, under 0.2 in a blink).
/// The iris is the yardstick because FaceMesh places it at its full size whether the lids hide it
/// or not, and it doesn't shrink when the head turns the way the eye's width does; without a
/// plausible iris, the gap over a third of the eye's width (an eye is about three irises wide).
pub fn openness(lm: &[[f32; 2]], k: usize) -> Option<f32> {
    let d = |a: usize, b: usize| -> Option<f32> {
        let (p, q) = (lm.get(a)?, lm.get(b)?);
        let v = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        v.is_finite().then_some(v)
    };
    let (corners, upper, lower) = LIDS.get(k)?;
    let gap = (d(upper[0], lower[0])? + d(upper[1], lower[1])?) / 2.0;
    let width = d(corners[0], corners[1])?;
    let iris = IRIS_POINTS.get(k).and_then(|(c, rim)| {
        let r = rim.iter().map(|&p| d(*c, p)).collect::<Option<Vec<f32>>>()?;
        Some(r.iter().sum::<f32>() / 2.0)
    });
    let yardstick = iris.filter(|v| *v > 0.5 && *v < width * 0.8).unwrap_or(width / 3.0);
    (yardstick > 0.0).then(|| gap / yardstick).filter(|v| v.is_finite())
}

/// How much the iris shows in eye `k`: the luminance just above the iris's centre (its upper
/// half, where an open eye shows the iris and a closed one its lid) over the luminance of the skin
/// below the eye (two iris diameters under it). Well under 1 when the iris shows (it is darker than
/// skin, light irises too), near 1 or above when the lid covers it. `None` without landmarks or
/// pixels to compare.
pub fn iris_showing(img: &Rgba8, lm: &[[f32; 2]], k: usize) -> Option<f32> {
    let (c, rim) = IRIS_POINTS.get(k)?;
    let c = *lm.get(*c)?;
    let r = rim.iter().filter_map(|p| lm.get(*p)).map(|p| ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt()).sum::<f32>() / 4.0;
    if !(r.is_finite() && r >= 2.0 && c.iter().all(|v| v.is_finite())) {
        return None;
    }
    let mean = |reg: Region| {
        let mut sum = 0.0f32;
        let mut n = 0.0f32;
        for y in reg.y..reg.y + reg.h {
            for x in reg.x..reg.x + reg.w {
                if let Some(p) = img.data.get(y * img.width + x) {
                    sum += luma(*p);
                    n += 1.0;
                }
            }
        }
        (n > 0.0).then(|| sum / n)
    };
    let iris = mean(Region::around(c[0], c[1] - r * 0.35, r * 0.9, r * 0.5, img.width, img.height)?)?;
    let skin = mean(Region::around(c[0], c[1] + r * 4.0, r * 2.0, r, img.width, img.height)?)?;
    (skin > 0.02).then(|| iris / skin).filter(|v| v.is_finite())
}

/// The lids' verdict from their gap ([`openness`]) and how much iris shows ([`iris_showing`]):
/// FaceMesh tends to draw a closed eye a little open (gaps of 0.3–0.4 on closed lids are common),
/// so a middling gap counts as closed when the iris doesn't show either, and as open when it does
/// (a laughing squint).
pub fn lids_from(openness: Option<f32>, iris: Option<f32>) -> Lids {
    match (Lids::of(openness), iris.filter(|v| v.is_finite())) {
        (Lids::Half, Some(i)) if i >= IRIS_HIDDEN => Lids::Closed,
        (Lids::Half, Some(i)) if i < IRIS_SHOWS => Lids::Open,
        (Lids::Open, Some(i)) if i >= IRIS_HIDDEN && openness.is_some_and(|o| o < WIDE_OPEN) => Lids::Half,
        (l, _) => l,
    }
}

/// [`iris_showing`] at or above this: no iris to be seen (lid skin where the iris would be).
pub const IRIS_HIDDEN: f32 = 0.85;
/// Below this: the iris clearly shows.
pub const IRIS_SHOWS: f32 = 0.40;
/// An eye this open ([`openness`]) is open whatever the iris check says (a pale iris in shade).
pub const WIDE_OPEN: f32 = 0.45;

/// Of two measured eyes, the one to show: the more open (open, unknown, half, closed), then the
/// sharper. Both closed: the sharper, still marked closed.
pub fn pick_eye<T>(a: Option<(f32, T)>, b: Option<(f32, T)>, lids: impl Fn(&(f32, T)) -> Lids) -> Option<(f32, T)> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (ra, rb) = (lids(&a).rank(), lids(&b).rank());
            Some(if rb > ra || (rb == ra && b.0 > a.0) { b } else { a })
        }
        (a, b) => a.or(b),
    }
}

/// One person's sharper eye.
#[derive(Clone, Debug)]
pub struct EyeShot {
    /// The face's box, normalised to the picture (x0, y0, x1, y1).
    pub face: [f32; 4],
    /// The eye's centre, normalised to the picture.
    pub center: [f32; 2],
    /// The crop at 100 % ([`CROP_W`] × [`CROP_H`], smaller at the picture's edges).
    pub crop: Arc<Rgba8>,
    /// 0–100 (see [`sharpness`]).
    pub sharpness: f32,
    pub focus: Focus,
    /// Whether the other eye was measured too (a face turned away shows one).
    pub both_eyes: bool,
    /// How far the eye is open: the lid gap over the iris's diameter (see [`openness`]).
    pub openness: Option<f32>,
    /// How much of the iris shows (see [`iris_showing`]; lower shows more).
    pub iris: Option<f32>,
    pub lids: Lids,
}

impl EyeShot {
    /// What the culler wants: sharp and open.
    pub fn good(&self) -> bool {
        self.focus == Focus::Sharp && self.lids == Lids::Open
    }
}

/// The eyes of one photo, left to right as the people stand.
#[derive(Clone, Debug)]
pub struct EyeReport {
    pub photo: PhotoId,
    pub eyes: Vec<EyeShot>,
    /// Faces found (before the [`MAX_PEOPLE`] cap).
    pub faces: usize,
    /// Of the people shown, those whose eyes couldn't be placed (a strong profile, a face buried
    /// in a shoulder): BlazeFace finds them, FaceMesh doesn't take them, and BlazeFace's own eye
    /// points on such faces land on ears and fingers as often as on eyes, so they get no crop.
    pub no_eyes: usize,
    /// The picture's size (pixels).
    pub size: (usize, usize),
    pub ms: f64,
}

impl EyeReport {
    /// People whose shown eye is closed or half closed (a cull filter could use it later).
    pub fn blinks(&self) -> usize {
        self.eyes.iter().filter(|e| matches!(e.lids, Lids::Closed | Lids::Half)).count()
    }
}

/// Where a photo's picture comes from.
#[derive(Clone)]
pub enum EyeSource {
    /// A raw's embedded camera JPEG, through the preview loader at full size.
    Embedded(String, crate::media::PreviewLoader),
    /// An image file, decoded whole.
    File(String),
}

/// A photo to look at.
#[derive(Clone)]
pub struct EyeJob {
    pub key: u64,
    pub photo: PhotoId,
    pub source: EyeSource,
}

/// Luminance (0–1, display-referred) of an sRGB pixel.
fn luma(p: [u8; 4]) -> f32 {
    (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
}

/// A rectangle of pixels, inside the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Region {
    /// `(cx, cy)` ± half of `w × h`, clipped to an `iw × ih` picture; `None` when nothing is left.
    pub fn around(cx: f32, cy: f32, w: f32, h: f32, iw: usize, ih: usize) -> Option<Region> {
        if !(cx.is_finite() && cy.is_finite() && w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
            return None;
        }
        let x0 = (cx - w / 2.0).round().clamp(0.0, iw as f32) as usize;
        let y0 = (cy - h / 2.0).round().clamp(0.0, ih as f32) as usize;
        let x1 = (cx + w / 2.0).round().clamp(0.0, iw as f32) as usize;
        let y1 = (cy + h / 2.0).round().clamp(0.0, ih as f32) as usize;
        (x1 > x0 + 2 && y1 > y0 + 2).then_some(Region { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

/// The luminance inside `r`, averaged over 2 × 2 pixels: (values, width, height). A defocused eye
/// has lost its detail at this scale too, while the photo's grain (mostly finer) weighs a quarter
/// as much.
fn luma_half(img: &Rgba8, r: Region) -> (Vec<f32>, usize, usize) {
    let (gw, gh) = (r.w / 2, r.h / 2);
    let at = |x: usize, y: usize| img.data.get(y * img.width + x).copied().map(luma).unwrap_or(0.0);
    let mut v = Vec::with_capacity(gw * gh);
    for gy in 0..gh {
        for gx in 0..gw {
            let (x, y) = (r.x + gx * 2, r.y + gy * 2);
            v.push((at(x, y) + at(x + 1, y) + at(x, y + 1) + at(x + 1, y + 1)) / 4.0);
        }
    }
    (v, gw, gh)
}

/// The 4-neighbour Laplacian of a `w × h` grid (its border left out).
fn laplacian(g: &[f32], w: usize, h: usize) -> Vec<f32> {
    let at = |x: usize, y: usize| g.get(y * w + x).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(w.saturating_sub(2) * h.saturating_sub(2));
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            out.push(4.0 * at(x, y) - at(x - 1, y) - at(x + 1, y) - at(x, y - 1) - at(x, y + 1));
        }
    }
    out
}

/// The noise at the measure's scale in `r` (a region without detail of its own): the median
/// absolute Laplacian, scaled to a standard deviation.
pub fn noise_level(img: &Rgba8, r: Region) -> f32 {
    let (g, w, h) = luma_half(img, r);
    let mut v: Vec<f32> = laplacian(&g, w, h).into_iter().map(f32::abs).filter(|v| v.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0) * 1.4826
}

/// How sharp the detail in `r` is: RMS Laplacian (of the 2 × 2-averaged luminance) with
/// `noise`'s energy taken out, over the region's contrast (its luminance's standard deviation).
/// Unitless; `0.0` for an empty or flat region.
pub fn focus_score(img: &Rgba8, r: Region, noise: f32) -> f32 {
    let (g, w, h) = luma_half(img, r);
    let lap = laplacian(&g, w, h);
    if lap.is_empty() || g.len() < 2 {
        return 0.0;
    }
    let rms = (lap.iter().map(|v| v * v).sum::<f32>() / lap.len() as f32).sqrt();
    let n = g.len() as f32;
    let mean = g.iter().sum::<f32>() / n;
    let std = (g.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n).max(0.0).sqrt();
    let noise = if noise.is_finite() { noise.max(0.0) } else { 0.0 };
    // the noise's own energy taken out (in quadrature: the two add as variances)
    let s = (rms * rms - noise * noise).max(0.0).sqrt() / (std + 0.02);
    if s.is_finite() { s } else { 0.0 }
}

/// [`focus_score`] on 0–100: 50 at a score of [`HALF_SCORE`] (a crisp eye scores well above it,
/// a missed one well below).
pub fn sharpness(score: f32) -> f32 {
    if !score.is_finite() || score <= 0.0 {
        return 0.0;
    }
    100.0 * (1.0 - (-score * std::f32::consts::LN_2 / HALF_SCORE).exp())
}

/// The [`focus_score`] that reads 50 on [`sharpness`].
pub const HALF_SCORE: f32 = 0.6;

/// The eyes worth judging of two measured ones `(score, (centre, width))`: both, unless one is
/// much narrower than the other ([`HIDDEN_EYE`]: turned away from the camera).
pub fn visible_eyes<C>(a: Option<(f32, (C, f32))>, b: Option<(f32, (C, f32))>) -> (Option<(f32, (C, f32))>, Option<(f32, (C, f32))>) {
    match (a, b) {
        (Some(a), Some(b)) if a.1.1 < b.1.1 * HIDDEN_EYE => (None, Some(b)),
        (Some(a), Some(b)) if b.1.1 < a.1.1 * HIDDEN_EYE => (Some(a), None),
        other => other,
    }
}

/// Of two measured eyes (score, anything), the sharper.
pub fn sharper<T>(a: Option<(f32, T)>, b: Option<(f32, T)>) -> Option<(f32, T)> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if b.0 > a.0 { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// The faces to show: the [`MAX_PEOPLE`] largest (by box area), then left to right.
pub fn people_order(boxes: &[[f32; 4]]) -> Vec<usize> {
    let area = |b: &[f32; 4]| ((b[2] - b[0]) * (b[3] - b[1])).max(0.0);
    let mut idx: Vec<usize> = (0..boxes.len()).filter(|&i| boxes.get(i).is_some_and(|b| b.iter().all(|v| v.is_finite()))).collect();
    idx.sort_by(|&a, &b| boxes.get(b).map(area).unwrap_or(0.0).total_cmp(&boxes.get(a).map(area).unwrap_or(0.0)));
    idx.truncate(MAX_PEOPLE);
    idx.sort_by(|&a, &b| {
        let cx = |i: usize| boxes.get(i).map(|b| (b[0] + b[2]) / 2.0).unwrap_or(0.0);
        cx(a).total_cmp(&cx(b))
    });
    idx
}

/// The 100 % crop centred on `(cx, cy)` ([`CROP_W`] × [`CROP_H`], kept inside the picture).
pub fn crop_at(img: &Rgba8, cx: f32, cy: f32) -> Option<Rgba8> {
    let w = CROP_W.min(img.width);
    let h = CROP_H.min(img.height);
    if w == 0 || h == 0 || !cx.is_finite() || !cy.is_finite() {
        return None;
    }
    let x0 = (cx - w as f32 / 2.0).round().clamp(0.0, (img.width - w) as f32) as usize;
    let y0 = (cy - h as f32 / 2.0).round().clamp(0.0, (img.height - h) as f32) as usize;
    let mut out = Rgba8::new(w, h);
    for (y, row) in out.data.chunks_mut(w).enumerate() {
        let start = (y0 + y) * img.width + x0;
        if let Some(src) = img.data.get(start..start + w) {
            row.copy_from_slice(src);
        }
    }
    Some(out)
}

/// One eye's measurement: (score, centre, width) from its outline points (photo pixels), with a patch of
/// cheek `below` it for the noise. `None` when the outline is degenerate.
pub fn measure_eye(img: &Rgba8, outline: &[[f32; 2]]) -> Option<(f32, [f32; 2], f32)> {
    let xs = outline.iter().map(|p| p[0]).filter(|v| v.is_finite());
    let ys = outline.iter().map(|p| p[1]).filter(|v| v.is_finite());
    let (x0, x1) = xs.clone().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    let (y0, y1) = ys.clone().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    if !(x1 > x0 && y1 >= y0) {
        return None;
    }
    let ew = (x1 - x0).max(4.0);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // the eye with its lids, lashes, creases and the brow above it: a laughing or blinking eye has
    // little detail of its own, and the brow hairs and lid creases are in the same focal plane;
    // at least 12 pixels each way
    let eye = Region::around(cx, cy - ew * 0.2, (ew * 1.5).max(12.0), (ew * 1.2).max(12.0), img.width, img.height)?;
    // the grain at the measure's scale, from the region itself: its median absolute Laplacian
    // (the edges that make an eye sharp are a minority of its pixels; a patch of cheek "beside"
    // it was a laughing crease or a tooth as often as plain skin)
    let noise = noise_level(img, eye);
    Some((focus_score(img, eye, noise), [cx, cy], ew))
}

/// Look at `img` (the photo at full size) with `model`: each person's sharper eye.
#[cfg(feature = "sam")]
pub fn analyze(model: &lightcraft_segment::people::FaceModel, img: &Rgba8, photo: PhotoId) -> Result<EyeReport, String> {
    use lightcraft_segment::mediapipe::{Detection, EYES, IRISES};
    let t0 = web_time::Instant::now();
    let (w, h) = (img.width, img.height);
    if w < 16 || h < 16 {
        return Ok(EyeReport { photo, eyes: Vec::new(), faces: 0, no_eyes: 0, size: (w, h), ms: 0.0 });
    }
    // faces in a smaller copy
    let s = (DETECT_EDGE as f32 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * s).round() as usize).max(1), ((h as f32 * s).round() as usize).max(1));
    let small = rgb_resized(img, sw, sh);
    let found = model.detect(&small, sw, sh).map_err(|e| e.to_string())?;
    let k = 1.0 / s;
    let boxes: Vec<[f32; 4]> = found.iter().map(|d| [d.x0 * k, d.y0 * k, d.x1 * k, d.y1 * k]).collect();
    let mut eyes = Vec::new();
    let mut no_eyes = 0;
    for i in people_order(&boxes) {
        let Some(d) = found.get(i) else { continue };
        let full = Detection {
            x0: d.x0 * k,
            y0: d.y0 * k,
            x1: d.x1 * k,
            y1: d.y1 * k,
            score: d.score,
            keypoints: d.keypoints.map(|p| [p[0] * k, p[1] * k]),
        };
        // FaceMesh on a full-resolution crop around the face (the eye contours are a few pixels
        // wide in the small copy)
        let landmarks = face_crop(img, &full).and_then(|(rgb, cw, ch, ox, oy)| {
            let local = Detection {
                x0: full.x0 - ox,
                y0: full.y0 - oy,
                x1: full.x1 - ox,
                y1: full.y1 - oy,
                score: full.score,
                keypoints: full.keypoints.map(|p| [p[0] - ox, p[1] - oy]),
            };
            model.landmarks(&rgb, cw, ch, &local).ok().flatten().map(|l| l.into_iter().map(|p| [p[0] + ox, p[1] + oy]).collect::<Vec<_>>())
        });
        // measured over the contour, shown centred on the iris (the lashes and the pupil's rim are
        // where a missed focus shows first)
        let Some(l) = &landmarks else {
            no_eyes += 1;
            continue;
        };
        let outline = |e: &[usize; 16]| e.iter().filter_map(|&j| l.get(j).copied()).collect::<Vec<_>>();
        let [a, b] = [0, 1].map(|k| {
            let m = EYES.get(k).and_then(|e| measure_eye(img, &outline(e)))?;
            let iris = IRISES.get(k).and_then(|(c, _)| l.get(*c)).copied().filter(|p| p.iter().all(|v| v.is_finite()));
            Some((m.0, ((iris.unwrap_or(m.1), (openness(l, k), iris_showing(img, l, k))), m.2)))
        });
        let (a, b) = visible_eyes(a, b);
        let both = a.is_some() && b.is_some();
        // the open eye if there is one, then the sharper
        let Some((score, ((c, (open, iris)), _))) = pick_eye(a, b, |e| lids_from(e.1.0.1.0, e.1.0.1.1)) else {
            no_eyes += 1;
            continue;
        };
        let Some(crop) = crop_at(img, c[0], c[1]) else { continue };
        let sh = sharpness(score);
        eyes.push(EyeShot {
            face: [full.x0 / w as f32, full.y0 / h as f32, full.x1 / w as f32, full.y1 / h as f32],
            center: [c[0] / w as f32, c[1] / h as f32],
            crop: Arc::new(crop),
            sharpness: sh,
            focus: Focus::of(sh),
            both_eyes: both,
            openness: open,
            iris,
            lids: lids_from(open, iris),
        });
    }
    Ok(EyeReport { photo, eyes, faces: found.len(), no_eyes, size: (w, h), ms: t0.elapsed().as_secs_f64() * 1000.0 })
}

/// `img` box-averaged to `w × h`, as packed RGB.
#[cfg_attr(not(feature = "sam"), allow(dead_code))]
fn rgb_resized(img: &Rgba8, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    let (sx, sy) = (img.width as f32 / w as f32, img.height as f32 / h as f32);
    for y in 0..h {
        let (y0, y1) = ((y as f32 * sy) as usize, (((y + 1) as f32 * sy) as usize).max((y as f32 * sy) as usize + 1).min(img.height));
        for x in 0..w {
            let (x0, x1) = ((x as f32 * sx) as usize, (((x + 1) as f32 * sx) as usize).max((x as f32 * sx) as usize + 1).min(img.width));
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            for yy in y0..y1 {
                for p in img.data.get(yy * img.width + x0..yy * img.width + x1).unwrap_or(&[]) {
                    acc[0] += u32::from(p[0]);
                    acc[1] += u32::from(p[1]);
                    acc[2] += u32::from(p[2]);
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend(acc.map(|v| (v / n) as u8));
        }
    }
    out
}

/// A full-resolution RGB crop around a face (its box 1.8 × as wide and tall), with its offset:
/// (rgb, w, h, x offset, y offset).
#[cfg(feature = "sam")]
fn face_crop(img: &Rgba8, d: &lightcraft_segment::mediapipe::Detection) -> Option<(Vec<u8>, usize, usize, f32, f32)> {
    let c = d.center();
    let side = d.width().max(d.height()) * 1.8;
    let r = Region::around(c[0], c[1], side, side, img.width, img.height)?;
    let mut rgb = Vec::with_capacity(r.w * r.h * 3);
    for y in r.y..r.y + r.h {
        for p in img.data.get(y * img.width + r.x..y * img.width + r.x + r.w)? {
            rgb.extend_from_slice(&p[..3]);
        }
    }
    Some((rgb, r.w, r.h, r.x as f32, r.y as f32))
}

/// The picture of a job, at full size (sRGB 8-bit, upright).
pub fn load(source: &EyeSource) -> Option<Rgba8> {
    match source {
        EyeSource::Embedded(path, loader) => loader(path, usize::MAX),
        EyeSource::File(path) => {
            let bytes = std::fs::read(path).ok()?;
            let d = lightcraft_codecs::decode(&bytes, Default::default()).ok()?;
            let o = lightcraft_geom::Orientation::from_exif(d.orientation);
            Some(d.to_srgb8().oriented(o))
        }
    }
}

impl crate::Session {
    /// The eye finder's job for photo `id`: a raw's camera JPEG, else the image file itself.
    /// Results depend on the picture only, so the key is the photo's content (not its edits).
    pub fn eye_job(&self, id: PhotoId) -> Option<EyeJob> {
        let p = self.catalog.photo(id)?;
        let lightcraft_catalog::Source::File { path } = &p.source else { return None };
        let key = lightcraft_preview::Hasher128::new().str(&crate::media::content_key(p)).str("eyes").u64(EYES_VERSION).finish().0 as u64;
        let source = match (&p.kind, &self.media.preview_loader) {
            (lightcraft_catalog::MediaKind::Raw, Some(l)) => EyeSource::Embedded(path.clone(), l.clone()),
            (lightcraft_catalog::MediaKind::Image, _) => EyeSource::File(path.clone()),
            _ => return None,
        };
        Some(EyeJob { key, photo: id, source })
    }

    /// Where the eye finder's model lives, when it is installed (`None`: it needs downloading,
    /// or this build has no AI masks).
    pub fn eye_model_dir(&self) -> Option<std::path::PathBuf> {
        self.segmenter.model_installed("face").then(|| self.segmenter.model_dir_for("face")).flatten()
    }
}

/// What came back for a job.
pub type EyeResult = (u64, PhotoId, Result<Arc<EyeReport>, String>);

/// The eye finder's thread: give it the photos wanted, most wanted first ([`EyeScanner::want`]);
/// it looks at them one at a time and hands back results ([`EyeScanner::finished`]). The model is
/// loaded on the first job and kept.
pub struct EyeScanner {
    shared: Arc<Shared>,
    rx: std::sync::mpsc::Receiver<EyeResult>,
}

struct Shared {
    queue: std::sync::Mutex<(std::collections::VecDeque<EyeJob>, bool)>,
    cv: std::sync::Condvar,
    model_dir: std::path::PathBuf,
}

impl EyeScanner {
    /// A scanner using the Face Landmarker model in `model_dir`. `None` where there are no threads.
    pub fn start(model_dir: std::path::PathBuf) -> Option<EyeScanner> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = model_dir;
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let shared = Arc::new(Shared { queue: Default::default(), cv: Default::default(), model_dir });
            let (tx, rx) = std::sync::mpsc::channel();
            let s = shared.clone();
            std::thread::Builder::new().name("lightcraft-eyes".into()).spawn(move || run(&s, &tx)).ok()?;
            Some(EyeScanner { shared, rx })
        }
    }

    /// The photos to look at, most wanted first; replaces the ones not started yet.
    pub fn want(&self, jobs: Vec<EyeJob>) {
        let mut q = self.shared.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        q.0 = jobs.into();
        self.shared.cv.notify_one();
    }

    /// Results since the last call.
    pub fn finished(&self) -> Vec<EyeResult> {
        self.rx.try_iter().collect()
    }

    /// The model folder it was started with.
    pub fn model_dir(&self) -> &std::path::Path {
        &self.shared.model_dir
    }
}

impl Drop for EyeScanner {
    fn drop(&mut self) {
        let mut q = self.shared.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        q.0.clear();
        q.1 = true;
        self.shared.cv.notify_all();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run(s: &Shared, tx: &std::sync::mpsc::Sender<EyeResult>) {
    #[cfg(feature = "sam")]
    let mut model: Option<lightcraft_segment::people::FaceModel> = None;
    loop {
        let job = {
            let mut q = s.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if q.1 {
                    return;
                }
                if let Some(j) = q.0.pop_front() {
                    break j;
                }
                q = s.cv.wait(q).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let result = crate::guard::catch("finding eyes", || -> Result<Arc<EyeReport>, String> {
            #[cfg(feature = "sam")]
            {
                if model.is_none() {
                    model = Some(lightcraft_segment::people::FaceModel::load(&s.model_dir).map_err(|e| e.to_string())?);
                }
                let m = model.as_ref().ok_or("no face model")?;
                let img = crate::memory::in_background(|| load(&job.source)).ok_or("the picture could not be read")?;
                let r = analyze(m, &img, job.photo).map(Arc::new);
                drop(img);
                crate::memory::release();
                r
            }
            #[cfg(not(feature = "sam"))]
            {
                Err("eye focus needs a build with AI masks".to_string())
            }
        })
        .and_then(|r| r);
        if tx.send((job.key, job.photo, result)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic eye: a dark iris disc with a bright catch-light on mid-grey skin, blurred by
    /// `blur` box passes, with `noise` added (deterministic).
    fn eye(blur: usize, noise: f32) -> Rgba8 {
        let (w, h) = (200usize, 140usize);
        let mut v: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let d = ((x - 100.0).powi(2) + (y - 50.0).powi(2)).sqrt();
                let lid = (y - 50.0).abs() > 22.0;
                if (x - 112.0).powi(2) + (y - 44.0).powi(2) < 9.0 {
                    0.95
                } else if d < 18.0 && !lid {
                    0.12
                } else if (x - 100.0).abs() < 40.0 && !lid {
                    0.8
                } else {
                    0.55
                }
            })
            .collect();
        for _ in 0..blur {
            let src = v.clone();
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    let mut s = 0.0;
                    for dy in 0..3 {
                        for dx in 0..3 {
                            s += src[(y + dy - 1) * w + x + dx - 1];
                        }
                    }
                    v[y * w + x] = s / 9.0;
                }
            }
        }
        let mut seed = 12345u32;
        let data = v
            .into_iter()
            .map(|l| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let n = ((seed >> 16) as f32 / 65535.0 - 0.5) * 2.0 * noise;
                let b = ((l + n).clamp(0.0, 1.0) * 255.0) as u8;
                [b, b, b, 255]
            })
            .collect();
        Rgba8 { width: w, height: h, data }
    }

    fn outline() -> Vec<[f32; 2]> {
        vec![[60.0, 50.0], [100.0, 30.0], [140.0, 50.0], [100.0, 70.0]]
    }

    /// A sharp eye scores higher than the same eye blurred, with or without noise, and noise alone
    /// doesn't make a blurred eye look sharp.
    #[test]
    fn a_sharp_eye_scores_above_a_blurred_one() {
        for noise in [0.0, 0.015] {
            let s = |b| measure_eye(&eye(b, noise), &outline()).unwrap().0;
            let (sharp, soft, blurry) = (s(0), s(3), s(10));
            assert!(sharp > soft && soft > blurry, "noise {noise}: {sharp} {soft} {blurry}");
            assert_eq!(Focus::of(sharpness(sharp)), Focus::Sharp, "noise {noise}: {sharp} -> {}", sharpness(sharp));
            assert_eq!(Focus::of(sharpness(blurry)), Focus::Blurry, "noise {noise}: {blurry} -> {}", sharpness(blurry));
        }
        // the measure follows the detail, not the brightness: a darker copy scores alike
        let bright = eye(0, 0.0);
        let dark = Rgba8 { data: bright.data.iter().map(|p| [p[0] / 3, p[1] / 3, p[2] / 3, 255]).collect(), ..bright.clone() };
        let (a, b) = (measure_eye(&bright, &outline()).unwrap().0, measure_eye(&dark, &outline()).unwrap().0);
        assert!((a / b - 1.0).abs() < 0.35, "{a} vs {b}");
    }

    #[test]
    fn the_sharper_eye_is_chosen() {
        assert_eq!(sharper(Some((0.4, "left")), Some((0.9, "right"))).map(|e| e.1), Some("right"));
        assert_eq!(sharper(Some((0.9, "left")), Some((0.4, "right"))).map(|e| e.1), Some("left"));
        assert_eq!(sharper(None, Some((0.1, "right"))).map(|e| e.1), Some("right"));
        assert_eq!(sharper::<&str>(None, None), None);
        // an open eye wins over a sharper closed one; both closed: the sharper
        let lid = |e: &(f32, (&str, Lids))| e.1.1;
        assert_eq!(pick_eye(Some((0.9, ("closed", Lids::Closed))), Some((0.4, ("open", Lids::Open))), lid).map(|e| e.1.0), Some("open"));
        assert_eq!(pick_eye(Some((0.9, ("a", Lids::Closed))), Some((0.4, ("b", Lids::Closed))), lid).map(|e| e.1.0), Some("a"));
        assert_eq!(pick_eye(Some((0.2, ("a", Lids::Open))), Some((0.4, ("b", Lids::Open))), lid).map(|e| e.1.0), Some("b"));
        // a far eye seen side on (a third of the near one's width) is not judged, however textured
        let (near, far) = (Some((0.3, ("near", 90.0))), Some((0.9, ("far", 30.0))));
        let (a, b) = visible_eyes(near, far);
        assert_eq!(sharper(a, b).map(|e| e.1.0), Some("near"));
        let (a, b) = visible_eyes(Some((0.3, ("l", 80.0))), Some((0.9, ("r", 70.0))));
        assert_eq!(sharper(a, b).map(|e| e.1.0), Some("r"), "both visible: the sharper");
        // on pictures: a sharp right eye beside a blurred left one
        let (sharp, blurred) = (eye(0, 0.01), eye(8, 0.01));
        let pick = sharper(measure_eye(&blurred, &outline()).map(|m| (m.0, "blurred")), measure_eye(&sharp, &outline()).map(|m| (m.0, "sharp")));
        assert_eq!(pick.map(|p| p.1), Some("sharp"));
    }

    /// Synthetic FaceMesh eyes: corners 60 px apart, an iris 20 px across, lids `gap` px apart.
    fn lids(gap: f32, iris: bool) -> Vec<[f32; 2]> {
        let mut lm = vec![[f32::NAN, f32::NAN]; 478];
        for (k, ox) in [(0usize, 100.0f32), (1, 300.0)] {
            let (corners, upper, lower) = LIDS[k];
            lm[corners[0]] = [ox, 100.0];
            lm[corners[1]] = [ox + 60.0, 100.0];
            lm[upper[0]] = [ox + 22.0, 100.0 - gap / 2.0];
            lm[upper[1]] = [ox + 38.0, 100.0 - gap / 2.0];
            lm[lower[0]] = [ox + 22.0, 100.0 + gap / 2.0];
            lm[lower[1]] = [ox + 38.0, 100.0 + gap / 2.0];
            if iris {
                let (c, rim) = IRIS_POINTS[k];
                lm[c] = [ox + 30.0, 100.0];
                for (i, p) in rim.iter().enumerate() {
                    let a = i as f32 * std::f32::consts::FRAC_PI_2;
                    lm[*p] = [ox + 30.0 + 10.0 * a.cos(), 100.0 + 10.0 * a.sin()];
                }
            }
        }
        lm
    }

    /// Open, half-closed and closed eyes from their lids; the iris is the yardstick, the eye's
    /// width (a third of it) without one; broken landmarks give no answer.
    #[test]
    fn lids_tell_open_half_and_closed_eyes() {
        for iris in [true, false] {
            assert_eq!(Lids::of(openness(&lids(18.0, iris), 0)), Lids::Open, "iris {iris}");
            assert_eq!(Lids::of(openness(&lids(6.8, iris), 1)), Lids::Half, "iris {iris}");
            assert_eq!(Lids::of(openness(&lids(1.0, iris), 0)), Lids::Closed, "iris {iris}");
        }
        assert!((openness(&lids(20.0, true), 0).unwrap() - 1.0).abs() < 1e-4);
        assert_eq!(openness(&lids(18.0, true)[..100], 0), None);
        assert_eq!(openness(&lids(18.0, true), 5), None);
        assert_eq!(Lids::of(None), Lids::Unknown);
        assert_eq!(Lids::of(Some(f32::NAN)), Lids::Unknown);
        // the iris settles a middling gap: shown, a squint (open); hidden, a lowered lid (closed)
        assert_eq!(lids_from(Some(0.33), Some(0.2)), Lids::Open);
        assert_eq!(lids_from(Some(0.33), Some(0.95)), Lids::Closed);
        assert_eq!(lids_from(Some(0.33), None), Lids::Half);
        assert_eq!(lids_from(Some(0.40), Some(0.95)), Lids::Half);
        assert_eq!(lids_from(Some(0.80), Some(0.95)), Lids::Open, "wide open whatever the iris check says");
        assert_eq!(lids_from(Some(0.10), Some(0.1)), Lids::Closed, "dark lashes over a closed lid");
    }

    /// The largest eight faces, then left to right as the people stand.
    #[test]
    fn people_are_the_largest_faces_left_to_right() {
        let b = |x: f32, s: f32| [x, 0.0, x + s, s];
        let boxes = vec![b(500.0, 50.0), b(100.0, 80.0), b(300.0, 10.0), b(200.0, 120.0)];
        assert_eq!(people_order(&boxes), vec![1, 3, 2, 0]);
        let many: Vec<[f32; 4]> = (0..12).map(|i| b(i as f32 * 100.0, 10.0 + i as f32)).collect();
        let o = people_order(&many);
        assert_eq!(o.len(), MAX_PEOPLE);
        assert_eq!(o, (4..12).collect::<Vec<_>>(), "the smallest faces are left out, the rest left to right");
        assert!(people_order(&[[f32::NAN, 0.0, 1.0, 1.0]]).is_empty());
    }

    /// Crops stay inside the picture; hostile input never panics.
    #[test]
    fn crops_and_measures_survive_edges_and_junk() {
        let img = eye(0, 0.0);
        let c = crop_at(&img, 0.0, 0.0).unwrap();
        assert_eq!((c.width, c.height), (CROP_W, CROP_H));
        assert_eq!(c.data[0], img.data[0]);
        let tiny = Rgba8::new(3, 2);
        assert_eq!(crop_at(&tiny, 1.0, 1.0).map(|c| (c.width, c.height)), Some((3, 2)));
        assert!(crop_at(&img, f32::NAN, 1.0).is_none());
        assert!(measure_eye(&img, &[]).is_none());
        assert!(measure_eye(&img, &[[f32::NAN, 1.0], [f32::INFINITY, 2.0]]).is_none());
        assert!(measure_eye(&img, &[[-500.0, -500.0], [-400.0, -480.0]]).is_none());
        assert_eq!(sharpness(f32::NAN), 0.0);
        assert_eq!(Focus::of(f32::NAN), Focus::Blurry);
    }

    /// The real check on a photographer's own frames: `LC_EYES_DIR` (a folder of raws, read only),
    /// `LC_EYES_MODELS` (the folder holding `mediapipe-face-landmarker`), `LC_EYES_OUT` (where the
    /// crops are written). Ignored by default.
    #[cfg(feature = "sam")]
    #[test]
    #[ignore]
    fn real_frames() {
        let (Ok(dir), Ok(models), Ok(out)) = (std::env::var("LC_EYES_DIR"), std::env::var("LC_EYES_MODELS"), std::env::var("LC_EYES_OUT")) else {
            eprintln!("set LC_EYES_DIR, LC_EYES_MODELS and LC_EYES_OUT");
            return;
        };
        let n: usize = std::env::var("LC_EYES_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let skip: usize = std::env::var("LC_EYES_SKIP").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let model = lightcraft_segment::people::FaceModel::load(&std::path::Path::new(&models).join("mediapipe-face-landmarker")).unwrap();
        let loader = crate::files::fs_preview_loader();
        std::fs::create_dir_all(&out).unwrap();
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| crate::import::is_supported(p)).collect();
        files.sort();
        for f in files.iter().skip(skip).take(n) {
            let path = f.to_string_lossy().to_string();
            let t = web_time::Instant::now();
            let Some(img) = load(&EyeSource::Embedded(path.clone(), loader.clone())) else { continue };
            let load_ms = t.elapsed().as_secs_f64() * 1000.0;
            let r = analyze(&model, &img, PhotoId(0)).unwrap();
            if std::env::var("LC_EYES_DEBUG").is_ok() {
                let s = (DETECT_EDGE as f32 / img.width.max(img.height) as f32).min(1.0);
                let (sw, sh) = ((img.width as f32 * s) as usize, (img.height as f32 * s) as usize);
                let small = rgb_resized(&img, sw, sh);
                for d in model.detect(&small, sw, sh).unwrap() {
                    let full = lightcraft_segment::mediapipe::Detection {
                        x0: d.x0 / s,
                        y0: d.y0 / s,
                        x1: d.x1 / s,
                        y1: d.y1 / s,
                        score: d.score,
                        keypoints: d.keypoints.map(|p| [p[0] / s, p[1] / s]),
                    };
                    let lm_small = model.landmarks(&small, sw, sh, &d).unwrap().is_some();
                    let lm_crop = face_crop(&img, &full).map(|(rgb, cw, ch, ox, oy)| {
                        let local = lightcraft_segment::mediapipe::Detection {
                            x0: full.x0 - ox,
                            y0: full.y0 - oy,
                            x1: full.x1 - ox,
                            y1: full.y1 - oy,
                            score: full.score,
                            keypoints: full.keypoints.map(|p| [p[0] - ox, p[1] - oy]),
                        };
                        model.landmarks(&rgb, cw, ch, &local).unwrap().is_some()
                    });
                    println!(
                        "   face {:.0},{:.0} {:.0}x{:.0} score {:.2} mesh small {lm_small} crop {lm_crop:?}",
                        full.x0,
                        full.y0,
                        full.width(),
                        full.height(),
                        d.score
                    );
                    if let Some((rgb, cw, ch, _, _)) = face_crop(&img, &full) {
                        let data = rgb.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2], 255]).collect();
                        let face = Rgba8 { width: cw, height: ch, data };
                        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&face), &Default::default()).unwrap();
                        std::fs::write(
                            std::path::Path::new(&out).join(format!("{}-face-{:.0}.png", f.file_stem().unwrap().to_string_lossy(), full.x0)),
                            png,
                        )
                        .unwrap();
                    }
                }
            }
            let stem = f.file_stem().unwrap().to_string_lossy().to_string();
            let mut line =
                format!("{stem}: {}x{} faces {} (no eyes {}) load {load_ms:.0} ms analyze {:.0} ms |", r.size.0, r.size.1, r.faces, r.no_eyes, r.ms);
            for (i, e) in r.eyes.iter().enumerate() {
                line += &format!(
                    " #{i} {:.0} {:?} {:?} o{:.2} i{:.2}{}",
                    e.sharpness,
                    e.focus,
                    e.lids,
                    e.openness.unwrap_or(-1.0),
                    e.iris.unwrap_or(-1.0),
                    if e.both_eyes { "" } else { " one-eye" }
                );
                if std::env::var("LC_EYES_ONLY_BLINKS").is_ok() && e.lids == Lids::Open {
                    continue;
                }
                let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&e.crop), &Default::default()).unwrap();
                std::fs::write(
                    std::path::Path::new(&out).join(format!(
                        "{stem}-{i}-{:.0}-{:?}-{:?}-o{:.2}-i{:.2}.png",
                        e.sharpness,
                        e.focus,
                        e.lids,
                        e.openness.unwrap_or(-1.0),
                        e.iris.unwrap_or(-1.0)
                    )),
                    png,
                )
                .unwrap();
            }
            println!("{line}");
        }
    }
}
