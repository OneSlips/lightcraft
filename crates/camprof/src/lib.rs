//! Camera profiling for LightCraft: photograph a ColorChecker, and build a DNG Camera Profile
//! (`.dcp`) for that camera from it.
//!
//! This is the profiling engine of RADcamProfiles (© RAD Films - Roman Alurkoff, MIT; relicensed
//! here as MIT OR Apache-2.0 by its author), embedded without its raw readers and container codecs:
//! LightCraft's own decoders build the [`frame::RawFrame`] the chart is measured on, and the
//! profile is written by [`lightcraft_raw::dcp::write_dcp`]. `NOTICE` and `assets/ATTRIBUTION.md` say what came over and
//! where its data comes from.
//!
//! # Pipeline
//!
//! 1. [`find_chart`] looks for the chart by its colours (or [`refine`] moves hand-placed corners
//!    onto it).
//! 2. [`build`] measures the patches ([`reference::measure_rotated`]), fits a forward matrix, a
//!    colour matrix and a correction table ([`profile::fit`]), and expresses them as a DCP
//!    ([`dcp::build`]): `ColorMatrix1`, `ForwardMatrix1` and a `ProfileHueSatMap`.
//! 3. The [`Built`] report says how closely the profile reproduces each patch (CIEDE2000).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// The numerics below are a port: keep their long-hand style.
// (`!(x > 0.0)` is deliberate: it is also true for NaN.)
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::explicit_counter_loop)]

pub mod align;
pub mod color;
mod data;
pub mod dcp;
pub mod detect;
pub mod evaluate;
pub mod frame;
pub mod illuminants;
pub mod linalg;
pub mod lut;
pub mod matopt;
pub mod observer;
pub mod profile;
pub mod reference;
pub mod simplex;
pub mod spectrum;
pub mod synthetic;
pub mod target;

pub use frame::RawFrame;
pub use linalg::{Mat3, Vec3};
pub use reference::ChartCorners;

use lightcraft_raw::dcp::DcpProfile;

/// Generator name, for profile descriptions.
pub const GENERATOR: &str = "LightCraft camera profiling (RADcamProfiles engine)";

/// How much of each patch's cell is sampled (the rest is margin against misplacement).
pub const SAMPLE_FRACTION: f64 = 0.6;

/// The calibration illuminants a chart photograph can be declared to be lit by.
pub const ILLUMINANTS: &[&str] = &["D50", "D55", "D65", "D75", "A"];

/// Why a profile could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CamprofError {
    /// The chart could not be found or the corners don't lie on it.
    NoChart(String),
    /// An option is out of range (an unknown illuminant, empty name…).
    Options(String),
    /// The measurement could not be fitted.
    Fit(String),
}

impl core::fmt::Display for CamprofError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoChart(m) | Self::Options(m) | Self::Fit(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CamprofError {}

/// Where the chart lies in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Located {
    /// Outer corners of the patch grid, in frame pixels, screen order (top left, top right, bottom
    /// right, bottom left).
    pub corners: ChartCorners,
    /// Quarter turns 0 to 3: patch A01 at the top left, top right, bottom right or bottom left.
    pub turns: u8,
    /// How closely the cells' colours follow the chart's (−1 to 1), when found by them.
    pub score: Option<f64>,
}

/// The ColorChecker 24 this crate profiles from.
pub fn chart() -> &'static reference::Chart {
    &reference::COLORCHECKER_CLASSIC
}

/// Look for the ColorChecker in `frame` by its colours (any position, size and quarter turn).
pub fn find_chart(frame: &RawFrame) -> Option<Located> {
    let found = detect::find_by_pattern(frame, chart())?;
    Some(Located { corners: found.corners, turns: found.quarter_turns % 4, score: Some(found.score) })
}

/// Move corners placed by hand onto the chart (`turns` is kept: the refinement cannot tell a grid
/// from its half turn). `None` when they are too far from a chart.
pub fn refine(frame: &RawFrame, corners: &ChartCorners, turns: u8) -> Option<Located> {
    if !corners.is_usable() {
        return None;
    }
    let placement = align::align(frame, chart(), corners, SAMPLE_FRACTION)?;
    Some(Located { corners: placement.corners, turns: turns % 4, score: None })
}

/// How to build a profile.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildOptions {
    /// The light the chart was photographed under ([`ILLUMINANTS`]).
    pub illuminant: String,
    /// Keep the camera's own lightness for each patch and take only hue and saturation from the
    /// chart ("Natural"), instead of reproducing the chart exactly ("Accurate").
    pub natural: bool,
    /// Fit a correction table (a `HueSatMap`) on top of the matrix.
    pub correction_table: bool,
    /// `UniqueCameraModel`: the camera, as raw converters name it.
    pub camera_model: String,
    /// `ProfileName`.
    pub name: String,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self { illuminant: "D55".into(), natural: false, correction_table: true, camera_model: String::new(), name: String::new() }
    }
}

/// One patch of the fit report.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchReport {
    /// `A01`…`D06`.
    pub id: String,
    /// "Dark Skin"…
    pub name: String,
    /// CIEDE2000 through the whole profile.
    pub delta_e: f64,
    /// CIEDE2000 through the matrix alone.
    pub matrix_delta_e: f64,
    /// The chart's value, CIELAB D50.
    pub reference_lab: [f64; 3],
    /// What the profile renders, CIELAB D50.
    pub rendered_lab: [f64; 3],
}

/// A built profile and how well it fits the chart.
#[derive(Clone, Debug)]
pub struct Built {
    /// The DCP.
    pub dcp: DcpProfile,
    /// Where the chart was measured.
    pub located: Located,
    /// Mean and largest CIEDE2000 over the patches used.
    pub mean_delta_e: f64,
    pub max_delta_e: f64,
    /// Mean CIEDE2000 of the matrix alone.
    pub matrix_mean_delta_e: f64,
    /// Patches measured and used (clipped or too-dark patches are left out).
    pub patches_read: usize,
    pub patches_used: usize,
    /// The grey patches the white balance was taken from.
    pub neutral_ids: Vec<String>,
    /// Per patch.
    pub patches: Vec<PatchReport>,
    /// Notes worth showing.
    pub notes: Vec<String>,
}

/// Measure the chart at `located` in `frame` and build a profile from it.
pub fn build(frame: &RawFrame, located: &Located, opts: &BuildOptions) -> Result<Built, CamprofError> {
    let chart = chart();
    if !located.corners.is_usable() {
        return Err(CamprofError::NoChart("the chart corners don't make a usable grid".into()));
    }
    let illuminant = opts.illuminant.trim();
    if !ILLUMINANTS.iter().any(|i| i.eq_ignore_ascii_case(illuminant)) {
        return Err(CamprofError::Options(format!("unknown illuminant `{illuminant}` (one of {})", ILLUMINANTS.join(", "))));
    }
    let illuminant = illuminant.to_ascii_uppercase();
    if opts.camera_model.trim().is_empty() {
        return Err(CamprofError::Options("the camera's name (UniqueCameraModel) is empty".into()));
    }
    let white = illuminants::white_point_by_name("1931_2", &illuminant).map_err(|e| CamprofError::Options(e.to_string()))?;
    let turns = located.turns % 4;
    let measurement = reference::measure_rotated(frame, chart, &located.corners, SAMPLE_FRACTION, turns);
    if measurement.sample_counts.contains(&0) {
        return Err(CamprofError::NoChart("part of the chart grid lies outside the picture".into()));
    }
    let patch_set = measurement.to_patch_set(white);
    let fit_options = profile::FitOptions {
        illuminant_name: Some(illuminant.clone()),
        camera_name: Some(opts.camera_model.trim().to_string()),
        make_lut: opts.correction_table,
        lut: profile::LutFitOptions { chart_lightness: if opts.natural { profile::NATURAL } else { profile::ACCURATE }, ..Default::default() },
        ..profile::FitOptions::default()
    };
    let (prof, report) = profile::fit(&patch_set, &fit_options, &mut |_| {}).map_err(|e| CamprofError::Fit(e.to_string()))?;
    let name = if opts.name.trim().is_empty() {
        format!("{illuminant}{}", if opts.natural { " Natural" } else { "" })
    } else {
        opts.name.trim().to_string()
    };
    let dcp = dcp::build(&prof, opts.camera_model.trim(), &name).map_err(|e| CamprofError::Fit(e.to_string()))?;

    let d50 = color::D50;
    let mut patches = Vec::with_capacity(prof.patch_ids.len());
    for (i, id) in prof.patch_ids.iter().enumerate() {
        let (Some(reference), Some(cam)) = (prof.reference_xyz.get(i), prof.camera_rgb.get(i)) else { continue };
        let rendered = prof.render(*cam);
        let name = chart.patches.iter().find(|p| p.id == id.as_str()).map(|p| p.name.to_string()).unwrap_or_else(|| id.clone());
        let lab = |xyz: Vec3| {
            let l = color::xyz_to_lab(xyz, d50);
            [l[0], l[1], l[2]]
        };
        patches.push(PatchReport {
            id: id.clone(),
            name,
            delta_e: prof.lut_patch_errors.get(i).copied().unwrap_or(f64::NAN),
            matrix_delta_e: prof.matrix_patch_errors.get(i).copied().unwrap_or(f64::NAN),
            reference_lab: lab(*reference),
            rendered_lab: lab(rendered),
        });
    }
    let errors: Vec<f64> = patches.iter().map(|p| p.delta_e).filter(|d| d.is_finite()).collect();
    let max_delta_e = errors.iter().copied().fold(0.0, f64::max);
    let mean_delta_e = if errors.is_empty() { report.lut_mean_de } else { errors.iter().sum::<f64>() / errors.len() as f64 };
    Ok(Built {
        dcp,
        located: Located { turns, ..*located },
        mean_delta_e,
        max_delta_e,
        matrix_mean_delta_e: report.matrix_mean_de,
        patches_read: report.patches_read,
        patches_used: report.patches_used,
        neutral_ids: report.neutral_ids,
        patches,
        notes: report.notes,
    })
}
