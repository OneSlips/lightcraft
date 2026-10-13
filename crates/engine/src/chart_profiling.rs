//! Camera profiling from a ColorChecker photograph (LR-PROF-CREATE): the chart is found on the raw
//! file, a DNG Camera Profile is fitted to it ([`lightcraft_camprof`]) and lands in the user's camera
//! profiles ([`crate::cmd::dcp_profiles`]), so it is chosen like an imported `.dcp` right away.
//!
//! The work is split so the UI can run it off its thread: [`prepare`] decodes the file and looks for
//! the chart, [`build`] fits the profile (both pure, `Send`), [`install`] files the result in the
//! session. The `profile.findChart` and `profile.createFromChart` commands do all of it in one call.
//!
//! **Coordinates.** Chart corners are in the *frame*: the raw file's active area as the sensor
//! recorded it (not rotated by its orientation tag), normalised to 0–1 across and down, in screen
//! order (top left, top right, bottom right, bottom left). `turns` (0–3) says which corner patch
//! A01 (dark skin) is in: top left, top right, bottom right, bottom left.

use std::sync::Arc;

pub use lightcraft_camprof as camprof;

use lightcraft_camprof::{BuildOptions, Built, ChartCorners, Located, RawFrame};
use lightcraft_raster::Rgba8;
use serde_json::{Value, json};

/// The longest edge of the preview [`prepare`] makes.
pub const PREVIEW_EDGE: usize = 1600;

/// A decoded chart photograph.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// Camera-native RGB at half the sensor's resolution: what the chart is measured on.
    pub frame: Arc<RawFrame>,
    /// `UniqueCameraModel` for the profile.
    pub camera_model: String,
    /// The frame for looking at (white balanced, display sRGB), at most [`PREVIEW_EDGE`] long.
    pub preview: Rgba8,
    /// Where the chart was found, if it was.
    pub located: Option<Located>,
}

/// Camera RGB of `raw` at half resolution (one pixel per 2 × 2 colour-filter unit; other mosaics
/// are demosaiced and halved), black removed, 1.0 the white level.
fn frame_of(raw: &lightcraft_raw::RawImage) -> Result<RawFrame, String> {
    let n = raw.normalized().map_err(|e| e.to_string())?;
    let (w, h) = (n.width / 2, n.height / 2);
    if w < 8 || h < 8 {
        return Err("the photo is too small to profile from".into());
    }
    let mut data = Vec::with_capacity(w.saturating_mul(h));
    let at = |x: usize, y: usize, c: usize| n.data.get((y * n.width + x) * n.cpp + c).copied().unwrap_or(0.0);
    match (&n.cfa, n.cpp) {
        (Some(cfa), 1) if cfa.width == 2 && cfa.height == 2 => {
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0f32; 3];
                    let mut count = [0.0f32; 3];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let c = usize::from(cfa.color_at(2 * x + dx, 2 * y + dy)).min(2);
                        sum[c] += at(2 * x + dx, 2 * y + dy, 0);
                        count[c] += 1.0;
                    }
                    data.push(std::array::from_fn(|c| if count[c] > 0.0 { sum[c] / count[c] } else { 0.0 }));
                }
            }
        }
        (Some(_), 1) => {
            // X-Trans and other mosaics: demosaic, then halve
            let rgb = lightcraft_raw::demosaic(&n, lightcraft_raw::Method::Bilinear);
            let px = |x: usize, y: usize| rgb.data.get(y * rgb.width + x).copied().unwrap_or([0.0; 3]);
            for y in 0..h {
                for x in 0..w {
                    let q = [px(2 * x, 2 * y), px(2 * x + 1, 2 * y), px(2 * x, 2 * y + 1), px(2 * x + 1, 2 * y + 1)];
                    data.push(std::array::from_fn(|c| q.iter().map(|p| p[c]).sum::<f32>() / 4.0));
                }
            }
        }
        (None, 3) => {
            for y in 0..h {
                for x in 0..w {
                    data.push(std::array::from_fn(|c| {
                        (at(2 * x, 2 * y, c) + at(2 * x + 1, 2 * y, c) + at(2 * x, 2 * y + 1, c) + at(2 * x + 1, 2 * y + 1, c)) / 4.0
                    }));
                }
            }
        }
        _ => return Err("a monochrome sensor has no colour to profile".into()),
    }
    RawFrame::from_rgb(w, h, data).map_err(|e| e.to_string())
}

/// The camera name a profile for `raw` carries: the file's own `UniqueCameraModel` when it is a
/// DNG that states one, else Adobe-style `Maker Model`.
fn camera_model_of(raw: &lightcraft_raw::RawImage, bytes: &[u8]) -> String {
    if raw.format == lightcraft_raw::RawFormat::Dng
        && let Some(name) = unique_camera_model(bytes)
    {
        return name;
    }
    let make = raw.metadata.make.clone().unwrap_or_default();
    let model = raw.metadata.model.clone().unwrap_or_default();
    let name = lightcraft_camprof::camera_name(&make, &model);
    if name.trim().is_empty() { "Unknown Camera".into() } else { name }
}

fn unique_camera_model(bytes: &[u8]) -> Option<String> {
    let opts = lightcraft_tiff::ParseOptions { follow_children: false, max_ifds: 2, ..Default::default() };
    let tiff = lightcraft_tiff::Tiff::parse_with(bytes, &opts).ok()?;
    let name = tiff.ifds.first()?.string(lightcraft_tiff::tags::UNIQUE_CAMERA_MODEL)?;
    let name = name.trim_matches(char::from(0)).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// The frame white balanced (by the file's as-shot neutral, else its grey world) and exposed for
/// looking at, at most [`PREVIEW_EDGE`] long. Not a rendering of the camera's colour: only for
/// placing the chart.
fn preview_of(frame: &RawFrame, neutral: Option<[f64; 3]>) -> Rgba8 {
    let scale = (frame.width.max(frame.height) as f64 / PREVIEW_EDGE as f64).max(1.0);
    let (w, h) = (((frame.width as f64 / scale) as usize).max(1), ((frame.height as f64 / scale) as usize).max(1));
    let small = frame.preview(w, h);
    let neutral = neutral.filter(|n| n.iter().all(|v| v.is_finite() && *v > 0.0)).unwrap_or_else(|| {
        let mut sum = [0.0f64; 3];
        for p in &small {
            for c in 0..3 {
                sum[c] += p.v[c];
            }
        }
        let g = sum[1].max(1e-9);
        [sum[0].max(1e-9) / g, 1.0, sum[2].max(1e-9) / g]
    });
    let balanced: Vec<[f64; 3]> = small.iter().map(|p| std::array::from_fn(|c| p.v[c] / neutral[c].max(1e-6))).collect();
    let mut lum: Vec<f64> = balanced.iter().map(|p| p[1]).filter(|v| v.is_finite()).collect();
    lum.sort_by(f64::total_cmp);
    let bright = lum.get(lum.len().saturating_sub(1) * 99 / 100).copied().unwrap_or(1.0).max(1e-6);
    let gain = 0.9 / bright;
    let data = balanced
        .iter()
        .map(|p| {
            let e = |v: f64| lightcraft_color::transfer::encode_srgb8((v * gain).clamp(0.0, 1.0) as f32);
            [e(p[0]), e(p[1]), e(p[2]), 255]
        })
        .collect();
    Rgba8 { width: w, height: h, data }
}

/// Decode a raw chart photograph and look for the chart.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, String> {
    if lightcraft_raw::probe(bytes).is_none() {
        return Err("not a raw file: a camera profile is made from a raw photo of the chart".into());
    }
    let raw = lightcraft_raw::decode(bytes).map_err(|e| e.to_string())?;
    let frame = frame_of(&raw)?;
    let camera_model = camera_model_of(&raw, bytes);
    let neutral = raw.color.as_shot_neutral.or_else(|| raw.wb_multipliers.map(|m| std::array::from_fn(|c| 1.0 / f64::from(m[c]).max(1e-6))));
    let preview = preview_of(&frame, neutral);
    let located = lightcraft_camprof::find_chart(&frame);
    Ok(Prepared { frame: Arc::new(frame), camera_model, preview, located })
}

/// [`prepare`] for a file (refused above 2 GB).
pub fn prepare_path(path: &str) -> Result<Prepared, String> {
    let len = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?.len();
    if len > 2 << 30 {
        return Err(format!("{path}: larger than 2 GB"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    prepare(&bytes)
}

/// Corners in frame pixels from normalised ones (`None` when one isn't a finite number).
pub fn corners_from_normalized(frame: &RawFrame, c: [[f64; 2]; 4]) -> Option<ChartCorners> {
    let (w, h) = (frame.width as f64, frame.height as f64);
    if c.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let p = |q: [f64; 2]| [q[0].clamp(-1.0, 2.0) * w, q[1].clamp(-1.0, 2.0) * h];
    Some(ChartCorners { top_left: p(c[0]), top_right: p(c[1]), bottom_right: p(c[2]), bottom_left: p(c[3]) })
}

/// Corners normalised to the frame.
pub fn normalized_corners(frame: &RawFrame, c: &ChartCorners) -> [[f64; 2]; 4] {
    let (w, h) = (frame.width.max(1) as f64, frame.height.max(1) as f64);
    c.points().map(|p| [p[0] / w, p[1] / h])
}

/// What to build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// Chart corners (normalised, see the module notes); `None`: where [`prepare`] found the chart.
    pub corners: Option<[[f64; 2]; 4]>,
    /// Which corner A01 is in; `None` with corners: decided by fitting.
    pub turns: Option<u8>,
    /// Move the corners onto the chart first.
    pub refine: bool,
    /// Illuminant, rendering, name (the camera model is filled in from the file when empty).
    pub options: BuildOptions,
}

/// Fit a profile to the chart in `prep`.
pub fn build(prep: &Prepared, req: &Request) -> Result<Built, String> {
    let frame = prep.frame.as_ref();
    let located = match req.corners {
        Some(c) => {
            let corners = corners_from_normalized(frame, c).ok_or("the chart corners must be numbers")?;
            if !corners.is_usable() {
                return Err("the chart corners don't make a usable grid".into());
            }
            let corners = if req.refine { lightcraft_camprof::refine(frame, &corners, 0).map(|l| l.corners).unwrap_or(corners) } else { corners };
            let turns = match req.turns {
                Some(t) => t % 4,
                None => lightcraft_camprof::orient(frame, &corners),
            };
            Located { corners, turns, score: None }
        }
        None => prep.located.ok_or("no ColorChecker found in the photo: place its corners by hand")?,
    };
    let mut options = req.options.clone();
    if options.camera_model.trim().is_empty() {
        options.camera_model = prep.camera_model.clone();
    }
    lightcraft_camprof::build(frame, &located, &options).map_err(|e| e.to_string())
}

/// The fit report as JSON (`ΔE` is CIEDE2000; corners normalised).
pub fn report_json(prep: &Prepared, built: &Built) -> Value {
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    json!({
        "cameraModel": built.dcp.camera_model,
        "name": built.dcp.name,
        "meanDeltaE": r3(built.mean_delta_e),
        "maxDeltaE": r3(built.max_delta_e),
        "matrixMeanDeltaE": r3(built.matrix_mean_delta_e),
        "patchesRead": built.patches_read,
        "patchesUsed": built.patches_used,
        "neutralIds": built.neutral_ids,
        "corners": normalized_corners(&prep.frame, &built.located.corners),
        "turns": built.located.turns,
        "notes": built.notes,
        "patches": built.patches.iter().map(|p| json!({
            "id": p.id,
            "name": p.name,
            "deltaE": r3(p.delta_e),
            "matrixDeltaE": r3(p.matrix_delta_e),
            "referenceLab": p.reference_lab.map(r3),
            "renderedLab": p.rendered_lab.map(r3),
        })).collect::<Vec<_>>(),
    })
}

/// File `built` among the user's camera profiles; with `apply_to`, that photo develops with it.
/// → `{id, name, cameraModel, file, applied}`.
pub fn install(s: &mut crate::Session, built: &Built, apply_to: Option<lightcraft_catalog::PhotoId>) -> crate::Result<Value> {
    let entry = crate::cmd::dcp_profiles::add_profile(s, built.dcp.clone(), "profile.createFromChart")?;
    let mut applied = Value::Null;
    if let Some(id) = apply_to
        && s.catalog.photo(id).is_some()
    {
        let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
        d.profile.id = entry.id.clone();
        s.set_develop(id, d, "Profile")?;
        s.note_profile_used(&entry.id);
        applied = json!(id.0);
    }
    s.save_prefs()?;
    Ok(json!({"id": entry.id, "name": entry.name, "cameraModel": entry.camera_model, "file": entry.file, "applied": applied}))
}

/// A ColorChecker photographed by a made-up camera, as a DNG (an RGGB mosaic `2 × width` by
/// `2 × height`), for tests and demonstrations: painted procedurally
/// ([`lightcraft_camprof::synthetic`]), no photograph involved. The chart grid lies at `corners`
/// (in half-resolution pixels), lit by D55; the file's own colour matrix is a naive one (sRGB
/// primaries), as an unprofiled camera's might be.
pub fn demo_chart_dng(width: usize, height: usize, corners: &ChartCorners) -> Result<Vec<u8>, String> {
    use lightcraft_raw::{BlackLevel, Cfa, ColorData, DngWriteOptions, Mat3, Metadata, OpcodeLists, Orientation, RawData, RawFormat, RawImage, Rect};
    if width < 8 || height < 8 || width > 8000 || height > 8000 {
        return Err("the demo chart photo must be 8 to 8000 pixels each way".into());
    }
    let d55 = lightcraft_camprof::illuminants::white_point_by_name("1931_2", "D55").map_err(|e| e.to_string())?;
    let rgb = lightcraft_camprof::synthetic::chart_frame(width, height, corners, 0, d55, 0.5);
    let (w, h) = (2 * width, 2 * height);
    let (black, white) = (256.0f32, 16000.0f32);
    let mut data = vec![0u16; w * h];
    let s = |v: f32| (black + v.clamp(0.0, 1.0) * (white - black)).round() as u16;
    for (i, [r, g, b]) in rgb.iter().enumerate() {
        let (x, y) = (i % width, i / width);
        for (dx, dy, v) in [(0, 0, *r), (1, 0, *g), (0, 1, *g), (1, 1, *b)] {
            if let Some(d) = data.get_mut((2 * y + dy) * w + 2 * x + dx) {
                *d = s(v);
            }
        }
    }
    let n = lightcraft_camprof::synthetic::camera_signal(d55);
    let neutral = [n[0] / n[1].max(1e-9), 1.0, n[2] / n[1].max(1e-9)];
    let srgb_from_xyz = Mat3([[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]]);
    let raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Cfa::bayer("RGGB"),
        bits: 14,
        black: BlackLevel::uniform(black),
        white: vec![white],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Normal,
        color: ColorData { illuminant: [21, 0], color_matrix: [Some(srgb_from_xyz), None], as_shot_neutral: Some(neutral), ..Default::default() },
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        metadata: Metadata { make: Some("LightCraft".into()), model: Some("Chart Camera".into()), ..Default::default() },
    };
    lightcraft_raw::write_dng(&raw, &DngWriteOptions::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_corners_are_errors() {
        let frame = RawFrame::from_rgb(40, 30, vec![[0.2; 3]; 1200]).unwrap();
        let prep = Prepared { frame: Arc::new(frame), camera_model: "X".into(), preview: Rgba8::new(1, 1), located: None };
        for corners in [[[f64::NAN, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], [[0.5; 2]; 4], [[1e308, -1e308], [0.0; 2], [0.0; 2], [0.0; 2]]] {
            let req = Request { corners: Some(corners), turns: Some(9), ..Default::default() };
            assert!(build(&prep, &req).is_err());
        }
        assert!(build(&prep, &Request::default()).unwrap_err().contains("no ColorChecker"));
        assert!(prepare(b"not a raw file").is_err());
        assert!(prepare(&[]).is_err());
    }
}
