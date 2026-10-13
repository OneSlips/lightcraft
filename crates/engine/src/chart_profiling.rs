//! Camera profiling from a ColorChecker photograph (LR-PROF-CREATE): the chart is found on the raw
//! file, a DNG Camera Profile is fitted to it ([`lightcraft_camprof`]) and lands in the user's camera
//! profiles ([`crate::cmd::dcp_profiles`]), so it is chosen like an imported `.dcp` right away.
//!
//! The work is split so the UI can run it off its thread: [`prepare`] decodes the file and looks for
//! the chart, [`build`] fits the profile (both pure, `Send`), [`install`] files the result in the
//! session. The `profile.findChart` and `profile.createFromChart` commands do all of it in one call.
//!
//! **Same data as the render.** The chart is measured on the camera RGB LightCraft's own raw decoder
//! develops ([`frame_of`]): what the profile is later applied to, so the fit sees the signal it
//! will render. RADcamProfiles' own raw readers are not used.
//!
//! **Coordinates.** Chart corners are in the *frame*: the raw file's developed picture (default
//! crop) as the sensor recorded it (not rotated by its orientation tag), normalised to 0–1 across and down, in screen
//! order (top left, top right, bottom right, bottom left). `turns` (0–3) says which corner patch
//! A01 (dark skin) is in: top left, top right, bottom right, bottom left. [`Prepared::display`] is
//! the photo as LightCraft develops it, oriented; [`Prepared::to_display`] / [`Prepared::from_display`]
//! map between the two.

use std::sync::Arc;

pub use lightcraft_camprof as camprof;

use lightcraft_camprof::{BuildOptions, Built, ChartCorners, Located, RawFrame};
use lightcraft_raster::Rgba8;
use serde_json::{Value, json};

/// The longest edge of the developed picture [`prepare`] makes for looking at.
pub const PREVIEW_EDGE: usize = 3000;

/// What a chart photo says about the light it was taken under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightEstimate {
    /// Colour temperature, K.
    pub cct: f64,
    /// The nearest calibration illuminant ([`lightcraft_camprof::ILLUMINANTS`]).
    pub illuminant: &'static str,
    /// Measured on the chart's greys through the file's colour matrix (`true`), or the colour
    /// temperature the camera recorded for its white balance (`false`).
    pub from_chart: bool,
}

/// A decoded chart photograph.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// Camera-native RGB at half the sensor's resolution: what the chart is measured on.
    pub frame: Arc<RawFrame>,
    /// `UniqueCameraModel` for the profile.
    pub camera_model: String,
    /// The photo developed as LightCraft does by default (as-shot white balance, the file's colour,
    /// default tone; no lens correction, so it lines up with the frame), oriented, display sRGB, at
    /// most [`PREVIEW_EDGE`] long.
    pub display: Rgba8,
    /// The file's orientation: `display` is the frame turned by it.
    pub orientation: lightcraft_geom::Orientation,
    /// The colour temperature the camera recorded for its as-shot white balance, when it records
    /// one (Canon `ColorTempAsShot`, Sony white-balance presets).
    pub recorded_kelvin: Option<f64>,
    /// The camera body's serial number (EXIF `BodySerialNumber`), when the file has it.
    pub serial: Option<String>,
    /// The capture date, `YYYY-MM-DD`, when the file has it.
    pub captured: Option<String>,
    /// Where the chart was found, if it was.
    pub located: Option<Located>,
    /// The file's own colour model, to tell which light the chart was under.
    pub color: lightcraft_raw::ColorData,
}

/// Camera RGB of `raw` at half resolution, taken exactly where LightCraft develops a raw file and
/// where a camera profile is applied ([`crate::files`]): black and white levels, linearisation,
/// `OpcodeList1/2`, the default crop, `OpcodeList3` without the lens corrections (the pipeline
/// applies those itself), before white balance and colour matrix. Bayer mosaics take the render
/// path's 2 × 2 binning (block means of each colour); other mosaics are demosaiced as for a full-size
/// render and halved.
fn frame_of(raw: &lightcraft_raw::RawImage) -> Result<RawFrame, String> {
    let mut raw = raw.clone();
    raw.opcodes.list3.retain(|op| !op.is_lens_correction());
    let rgb = match raw.develop_binned_masked(2, f32::INFINITY).map_err(|e| e.to_string())? {
        Some((img, _)) => img,
        None => {
            if raw.cpp == 1 && raw.cfa.is_none() {
                return Err("a monochrome sensor has no colour to profile".into());
            }
            let full = raw.develop(lightcraft_raw::Method::Ahd).map_err(|e| e.to_string())?;
            let (w, h) = (full.width / 2, full.height / 2);
            let px = |x: usize, y: usize| full.data.get(y * full.width + x).copied().unwrap_or([0.0; 3]);
            let mut data = Vec::with_capacity(w.saturating_mul(h));
            for y in 0..h {
                for x in 0..w {
                    let q = [px(2 * x, 2 * y), px(2 * x + 1, 2 * y), px(2 * x, 2 * y + 1), px(2 * x + 1, 2 * y + 1)];
                    data.push(std::array::from_fn(|c| q.iter().map(|p| p[c]).sum::<f32>() / 4.0));
                }
            }
            lightcraft_raster::Rgb32f { width: w, height: h, data }
        }
    };
    if rgb.width < 8 || rgb.height < 8 {
        return Err("the photo is too small to profile from".into());
    }
    RawFrame::from_rgb(rgb.width, rgb.height, rgb.data).map_err(|e| e.to_string())
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
/// looking at, oriented: the stand-in when the photo can't be developed for display.
fn preview_of(frame: &RawFrame, neutral: Option<[f64; 3]>, orientation: lightcraft_geom::Orientation) -> Rgba8 {
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
    Rgba8 { width: w, height: h, data }.into_oriented(orientation)
}

/// The photo as LightCraft develops it by default, for looking at (see [`Prepared::display`]).
fn developed(bytes: &[u8]) -> Result<Rgba8, String> {
    let (img, info) = crate::files::load_bytes(bytes, PREVIEW_EDGE)?;
    let mut settings = lightcraft_develop::DevelopSettings::for_raw(info.as_shot_temp, info.as_shot_tint);
    settings.optics.lens_profile = false;
    let out = lightcraft_pipeline::render(&img, &info, &settings, &lightcraft_pipeline::RenderRequest::fit(PREVIEW_EDGE, PREVIEW_EDGE));
    Ok(out.image)
}

/// Decode a raw chart photograph and look for the chart.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, String> {
    if lightcraft_raw::probe(bytes).is_none() {
        return Err("not a raw file: a camera profile is made from a raw photo of the chart".into());
    }
    let raw = lightcraft_raw::decode(bytes).map_err(|e| e.to_string())?;
    let frame = frame_of(&raw)?;
    let camera_model = camera_model_of(&raw, bytes);
    let orientation = raw.orientation;
    let display = match developed(bytes) {
        Ok(img) => img,
        Err(e) => {
            log::warn!("camera profile: the chart photo could not be developed for display ({e}); showing it as shot");
            let neutral = raw.color.as_shot_neutral.or_else(|| raw.wb_multipliers.map(|m| std::array::from_fn(|c| 1.0 / f64::from(m[c]).max(1e-6))));
            preview_of(&frame, neutral, orientation)
        }
    };
    let recorded_kelvin = recorded_kelvin(&raw);
    let located = lightcraft_camprof::find_chart(&frame);
    let meta = if raw.metadata.serial_number.is_some() && raw.metadata.capture_time.is_some() { None } else { Some(lightcraft_meta::extract(bytes)) };
    let serial = raw
        .metadata
        .serial_number
        .clone()
        .or_else(|| meta.as_ref().and_then(|m| m.serial_number.clone()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let captured = raw
        .metadata
        .capture_time
        .or_else(|| meta.as_ref().and_then(|m| m.capture_time))
        .map(|t| format!("{:04}-{:02}-{:02}", t.year, t.month, t.day));
    Ok(Prepared { serial, captured, frame: Arc::new(frame), camera_model, display, orientation, recorded_kelvin, located, color: raw.color })
}

impl Prepared {
    /// A normalised frame point on [`Self::display`] (normalised).
    pub fn to_display(&self, p: [f64; 2]) -> [f64; 2] {
        let (x, y) = self.orientation.map(p[0], p[1], 1.0, 1.0);
        [x, y]
    }

    /// A normalised point of [`Self::display`] in the frame (normalised).
    pub fn from_display(&self, p: [f64; 2]) -> [f64; 2] {
        let (x, y) = self.orientation.inverse().map(p[0], p[1], 1.0, 1.0);
        [x, y]
    }
}

/// What a quick look at a photo finds (for choosing a chart photo from the catalog): the chart's
/// pattern score, and the light, when a chart is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartScan {
    pub score: f64,
    pub light: Option<LightEstimate>,
}

/// Look for a ColorChecker in a raw file without developing it for display: `None` when there is
/// none (or the file isn't a raw photo).
pub fn scan_path(path: &str) -> Option<ChartScan> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > 2 << 30 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    lightcraft_raw::probe(&bytes)?;
    let raw = lightcraft_raw::decode(&bytes).ok()?;
    let frame = frame_of(&raw).ok()?;
    let located = lightcraft_camprof::find_chart(&frame)?;
    let prep = Prepared {
        frame: Arc::new(frame),
        camera_model: String::new(),
        display: Rgba8::new(1, 1),
        orientation: raw.orientation,
        recorded_kelvin: recorded_kelvin(&raw),
        serial: None,
        captured: None,
        located: Some(located),
        color: raw.color,
    };
    Some(ChartScan { score: located.score.unwrap_or(0.0), light: estimate_light(&prep, &located) })
}

fn recorded_kelvin(raw: &lightcraft_raw::RawImage) -> Option<f64> {
    raw.color
        .as_shot_kelvin
        .or_else(|| raw.color.kelvin_presets.as_ref()?.temp_tint(raw.wb_multipliers?).map(|(t, _)| t))
        .filter(|k| k.is_finite() && (1500.0..=15000.0).contains(k))
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

/// The light a chart photo was taken under: the chart's mid greys (D02–D05) taken to a white point
/// through the file's own colour model (DNG 6.3) when the file has one, else the colour temperature
/// the camera recorded for its white balance. `None` when neither is known: the light must be chosen.
pub fn estimate_light(prep: &Prepared, located: &Located) -> Option<LightEstimate> {
    measured_light(prep, located)
        .map(|cct| LightEstimate { cct, illuminant: lightcraft_camprof::nearest_illuminant(cct), from_chart: true })
        .or_else(|| prep.recorded_kelvin.map(|cct| LightEstimate { cct, illuminant: lightcraft_camprof::nearest_illuminant(cct), from_chart: false }))
}

fn measured_light(prep: &Prepared, located: &Located) -> Option<f64> {
    if !lightcraft_raw::color::has_matrix(&prep.color) {
        return None;
    }
    let chart = lightcraft_camprof::chart();
    let m =
        lightcraft_camprof::reference::measure_rotated(&prep.frame, chart, &located.corners, lightcraft_camprof::SAMPLE_FRACTION, located.turns % 4);
    let mut sum = [0.0f64; 3];
    for (id, rgb) in m.ids.iter().zip(&m.camera_rgb) {
        if ["D02", "D03", "D04", "D05"].contains(&id.as_str()) {
            for c in 0..3 {
                sum[c] += rgb[c];
            }
        }
    }
    let peak = sum.iter().copied().fold(0.0, f64::max);
    if peak.is_nan() || peak <= 0.0 || sum.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let xy = lightcraft_raw::color::neutral_to_xy(&prep.color, sum.map(|v| v / peak));
    let cct = lightcraft_color::cct::xy_to_temp_tint(xy).0;
    (cct.is_finite() && cct > 1000.0 && cct < 25000.0).then_some(cct)
}

/// What to build from one chart photo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// Chart corners (normalised, see the module notes); `None`: where [`prepare`] found the chart.
    pub corners: Option<[[f64; 2]; 4]>,
    /// Which corner A01 is in; `None` with corners: decided by fitting.
    pub turns: Option<u8>,
    /// Move the corners onto the chart first.
    pub refine: bool,
    /// The light ([`lightcraft_camprof::ILLUMINANTS`]); empty or `auto`: [`estimate_light`] (an error
    /// when there is none: the light must be chosen).
    pub illuminant: String,
}

/// Where the chart of `prep` lies for `req`.
pub fn locate(prep: &Prepared, req: &Request) -> Result<Located, String> {
    let frame = prep.frame.as_ref();
    match req.corners {
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
            Ok(Located { corners, turns, score: None })
        }
        None => prep.located.ok_or_else(|| "no ColorChecker found in the photo: place its corners by hand".into()),
    }
}

/// One calibration of a built profile, and the photo it came from.
#[derive(Clone, Debug)]
pub struct Calibration {
    pub built: Built,
    /// 0: the first photo, 1: the second.
    pub shot: usize,
    /// The light as estimated from the photo, when it could be.
    pub estimated: Option<LightEstimate>,
}

/// A profile from one chart photo, or a dual-illuminant one from two under different lights.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub dcp: lightcraft_raw::dcp::DcpProfile,
    /// The lower colour temperature first.
    pub calibrations: Vec<Calibration>,
}

/// The light a request asks for: its own, else the estimate.
fn light_of(prep: &Prepared, req: &Request, located: &Located) -> Result<(String, Option<LightEstimate>), String> {
    let estimated = estimate_light(prep, located);
    let chosen = req.illuminant.trim();
    if chosen.is_empty() || chosen.eq_ignore_ascii_case("auto") {
        let e = estimated.ok_or("this camera doesn't record the light it was used under, and the file has no colour matrix to tell it from the chart: choose the light (A, D50, D55, D65 or D75)")?;
        Ok((e.illuminant.to_string(), estimated))
    } else {
        Ok((chosen.to_string(), estimated))
    }
}

/// The last two characters of a body serial number, as the name of a profile tells bodies apart
/// (`Canon EOS R5 C 71`); `None` without a serial.
pub fn body_suffix(serial: Option<&str>) -> Option<String> {
    let s: Vec<char> = serial?.trim().chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return None;
    }
    Some(s[s.len().saturating_sub(2)..].iter().collect())
}

/// The camera as a profile name says it: the model, then the body's serial suffix.
pub fn camera_label(prep: &Prepared) -> String {
    match body_suffix(prep.serial.as_deref()) {
        Some(b) => format!("{} {b}", prep.camera_model),
        None => prep.camera_model.clone(),
    }
}

/// The name a profile gets when none is given: `<camera> <body> · <light(s)> · <HSL|HS> · <date>`
/// ([`lightcraft_camprof::default_name`]); the date is the first photo's capture date, else today.
pub fn default_name(first: &Prepared, lights: &[&str], hue_saturation_only: bool) -> String {
    let mut lights: Vec<&str> = lights.to_vec();
    lights.sort_by(|a, b| lightcraft_camprof::illuminant_cct(a).unwrap_or(0.0).total_cmp(&lightcraft_camprof::illuminant_cct(b).unwrap_or(0.0)));
    let today = crate::import::system_clock();
    let date = first.captured.clone().unwrap_or_else(|| today.get(..10).unwrap_or("").to_string());
    lightcraft_camprof::default_name(&camera_label(first), &lights.join(" + "), hue_saturation_only, &date)
}

/// Two chart photos from different bodies of the camera: their serial suffixes.
pub fn bodies_differ(a: &Prepared, b: &Prepared) -> Option<(String, String)> {
    match (body_suffix(a.serial.as_deref()), body_suffix(b.serial.as_deref())) {
        (Some(x), Some(y)) if a.serial != b.serial => Some((x, y)),
        _ => None,
    }
}

/// Fit a profile to the chart in `first` (and, for a dual-illuminant profile, `second`).
/// `options.illuminant` is ignored: each photo's light comes from its [`Request`].
pub fn build(first: (&Prepared, &Request), second: Option<(&Prepared, &Request)>, options: &BuildOptions) -> Result<Outcome, String> {
    let mut options = options.clone();
    if options.camera_model.trim().is_empty() {
        options.camera_model = first.0.camera_model.clone();
    }
    let la = locate(first.0, first.1)?;
    let (ia, ea) = light_of(first.0, first.1, &la)?;
    let named = !options.name.trim().is_empty();
    if !named && second.is_none() {
        options.name = default_name(first.0, &[&ia], options.natural);
    }
    let Some((prep_b, req_b)) = second else {
        let built = lightcraft_camprof::build(&first.0.frame, &la, &BuildOptions { illuminant: ia, ..options }).map_err(|e| e.to_string())?;
        return Ok(Outcome { dcp: built.dcp.clone(), calibrations: vec![Calibration { built, shot: 0, estimated: ea }] });
    };
    if prep_b.camera_model != first.0.camera_model {
        return Err(format!("the two chart photos are from different cameras ({} and {})", first.0.camera_model, prep_b.camera_model));
    }
    let lb = locate(prep_b, req_b)?;
    let (ib, eb) = light_of(prep_b, req_b, &lb)?;
    if !named {
        options.name = default_name(first.0, &[&ia, &ib], options.natural);
    }
    let dual = lightcraft_camprof::build_dual(
        &lightcraft_camprof::Shot { frame: &first.0.frame, located: la, illuminant: &ia },
        &lightcraft_camprof::Shot { frame: &prep_b.frame, located: lb, illuminant: &ib },
        &options,
    )
    .map_err(|e| e.to_string())?;
    // (build_dual puts the lower colour temperature first)
    let first_is_low = lightcraft_camprof::illuminant_cct(&ia).unwrap_or(0.0) <= lightcraft_camprof::illuminant_cct(&ib).unwrap_or(0.0);
    let [low, high] = dual.calibrations;
    let calibrations = if first_is_low {
        vec![Calibration { built: low, shot: 0, estimated: ea }, Calibration { built: high, shot: 1, estimated: eb }]
    } else {
        vec![Calibration { built: low, shot: 1, estimated: eb }, Calibration { built: high, shot: 0, estimated: ea }]
    };
    Ok(Outcome { dcp: dual.dcp, calibrations })
}

/// The fit report as JSON (`ΔE` is CIEDE2000; corners normalised to each photo's frame). The
/// top-level ΔE figures cover every calibration (the mean of their means, the largest maximum);
/// `calibrations` has each light's own.
pub fn report_json(preps: &[&Prepared], out: &Outcome) -> Value {
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    let calibrations: Vec<Value> = out
        .calibrations
        .iter()
        .map(|c| {
            let b = &c.built;
            let corners = preps.get(c.shot).map(|p| normalized_corners(&p.frame, &b.located.corners));
            json!({
                "shot": c.shot,
                "illuminant": b.illuminant,
                "cct": lightcraft_camprof::illuminant_cct(&b.illuminant),
                "estimatedCct": c.estimated.map(|e| e.cct.round()),
                "estimatedIlluminant": c.estimated.map(|e| e.illuminant),
                "estimatedFrom": c.estimated.map(|e| if e.from_chart { "chart" } else { "camera" }),
                "meanDeltaE": r3(b.mean_delta_e),
                "maxDeltaE": r3(b.max_delta_e),
                "matrixMeanDeltaE": r3(b.matrix_mean_delta_e),
                "patchesRead": b.patches_read,
                "patchesUsed": b.patches_used,
                "neutralIds": b.neutral_ids,
                "corners": corners,
                "turns": b.located.turns,
                "notes": b.notes,
                "patches": b.patches.iter().map(|p| json!({
                    "id": p.id,
                    "name": p.name,
                    "deltaE": r3(p.delta_e),
                    "matrixDeltaE": r3(p.matrix_delta_e),
                    "referenceLab": p.reference_lab.map(r3),
                    "renderedLab": p.rendered_lab.map(r3),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let n = out.calibrations.len().max(1) as f64;
    let mean = out.calibrations.iter().map(|c| c.built.mean_delta_e).sum::<f64>() / n;
    let max = out.calibrations.iter().map(|c| c.built.max_delta_e).fold(0.0, f64::max);
    let matrix = out.calibrations.iter().map(|c| c.built.matrix_mean_delta_e).sum::<f64>() / n;
    let primary = calibrations.iter().find(|c| c["shot"] == 0).cloned().unwrap_or(Value::Null);
    json!({
        "cameraModel": out.dcp.camera_model,
        "name": out.dcp.name,
        "dual": out.calibrations.len() == 2,
        "meanDeltaE": r3(mean),
        "maxDeltaE": r3(max),
        "matrixMeanDeltaE": r3(matrix),
        "patchesUsed": primary["patchesUsed"],
        "corners": primary["corners"],
        "turns": primary["turns"],
        "illuminant": primary["illuminant"],
        "patches": primary["patches"],
        "calibrations": calibrations,
    })
}

/// File `dcp` among the user's camera profiles (`serial`: the body's, kept with the entry); the
/// photos in `apply_to` develop with it. A profile of the same name for the camera is handled as
/// `on_exists` says. → `{id, name, cameraModel, file, applied: [photo ids], replaced}`, or, with
/// [`OnExists::Ask`] and a profile there, `{exists: {id, name, usedBy}, keepBothName}` (nothing saved).
pub fn install(
    s: &mut crate::Session,
    dcp: &lightcraft_raw::dcp::DcpProfile,
    serial: Option<String>,
    on_exists: crate::cmd::dcp_profiles::OnExists,
    apply_to: &[lightcraft_catalog::PhotoId],
) -> crate::Result<Value> {
    use crate::cmd::dcp_profiles::Added;
    let (entry, replaced) = match crate::cmd::dcp_profiles::add_profile(s, dcp.clone(), serial, on_exists, "profile.createFromChart")? {
        Added::New(e) => (e, false),
        Added::Replaced(e) => (e, true),
        Added::Exists(e, used) => {
            let keep = crate::cmd::dcp_profiles::free_name(s, &e.name, &e.camera_model);
            return Ok(json!({"exists": {"id": e.id, "name": e.name, "usedBy": used}, "keepBothName": keep}));
        }
    };
    let mut applied = Vec::new();
    for &id in apply_to {
        if s.catalog.photo(id).is_none() || applied.contains(&id.0) {
            continue;
        }
        let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
        if d.calibration_id() == Some(entry.id.as_str()) {
            continue;
        }
        crate::cmd::dcp_profiles::choose(s, &mut d, &entry.id);
        s.set_develop(id, d, "Profile")?;
        applied.push(id.0);
    }
    if !applied.is_empty() {
        s.note_profile_used(&entry.id);
    }
    s.save_prefs()?;
    Ok(
        json!({"id": entry.id, "name": entry.name, "cameraModel": entry.camera_model, "file": entry.file, "bodies": entry.bodies, "applied": applied, "replaced": replaced}),
    )
}

/// A ColorChecker photographed by a made-up camera, as a DNG (an RGGB mosaic `2 × width` by
/// `2 × height`), for tests and demonstrations: painted procedurally
/// ([`lightcraft_camprof::synthetic`]), no photograph involved. The chart grid lies at `corners`
/// (in half-resolution pixels), lit by `light` (an illuminant of
/// [`lightcraft_camprof::ILLUMINANTS`]). The file's own colour matrix is the made-up camera's linear
/// response when `true_matrix`, else a naive one (sRGB primaries), as an unprofiled camera's might be.
pub fn demo_chart_dng(width: usize, height: usize, corners: &ChartCorners, light: &str, true_matrix: bool) -> Result<Vec<u8>, String> {
    demo_chart_dng_of_body(width, height, corners, light, true_matrix, None)
}

/// [`demo_chart_dng`] from the camera body with serial number `serial` (DNG `CameraSerialNumber`).
pub fn demo_chart_dng_of_body(
    width: usize,
    height: usize,
    corners: &ChartCorners,
    light: &str,
    true_matrix: bool,
    serial: Option<&str>,
) -> Result<Vec<u8>, String> {
    use lightcraft_raw::{BlackLevel, Cfa, ColorData, DngWriteOptions, Mat3, Metadata, OpcodeLists, Orientation, RawData, RawFormat, RawImage, Rect};
    if width < 8 || height < 8 || width > 8000 || height > 8000 {
        return Err("the demo chart photo must be 8 to 8000 pixels each way".into());
    }
    let tag = lightcraft_camprof::illuminants::ExifLightSource::from_name(light).ok_or_else(|| format!("unknown light `{light}`"))?;
    let lit = lightcraft_camprof::illuminants::white_point_by_name("1931_2", light).map_err(|e| e.to_string())?;
    let rgb = lightcraft_camprof::synthetic::chart_frame(width, height, corners, 0, lit, 0.5);
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
    let n = lightcraft_camprof::synthetic::camera_signal(lit);
    let neutral = [n[0] / n[1].max(1e-9), 1.0, n[2] / n[1].max(1e-9)];
    let matrix = if true_matrix {
        Mat3(lightcraft_camprof::synthetic::TEST_CAMERA)
    } else {
        Mat3([[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]])
    };
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
        color: ColorData {
            illuminant: [u16::from(tag.tag()), 0],
            color_matrix: [Some(matrix), None],
            as_shot_neutral: Some(neutral),
            ..Default::default()
        },
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        metadata: Metadata {
            make: Some("LightCraft".into()),
            model: Some("Chart Camera".into()),
            serial_number: serial.map(str::to_string),
            ..Default::default()
        },
    };
    lightcraft_raw::write_dng(&raw, &DngWriteOptions::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_corners_are_errors() {
        let frame = RawFrame::from_rgb(40, 30, vec![[0.2; 3]; 1200]).unwrap();
        let prep = Prepared {
            frame: Arc::new(frame),
            camera_model: "X".into(),
            display: Rgba8::new(1, 1),
            orientation: Default::default(),
            recorded_kelvin: None,
            serial: None,
            captured: None,
            located: None,
            color: Default::default(),
        };
        for corners in [[[f64::NAN, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], [[0.5; 2]; 4], [[1e308, -1e308], [0.0; 2], [0.0; 2], [0.0; 2]]] {
            let req = Request { corners: Some(corners), turns: Some(9), ..Default::default() };
            assert!(build((&prep, &req), None, &Default::default()).is_err());
        }
        assert!(build((&prep, &Request::default()), None, &Default::default()).unwrap_err().contains("no ColorChecker"));
        assert!(prepare(b"not a raw file").is_err());
        assert!(prepare(&[]).is_err());
    }
}
