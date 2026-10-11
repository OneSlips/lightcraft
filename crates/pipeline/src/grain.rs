//! Film grain: a stochastic Boolean model of silver/dye grains, filtered by the output pixel.
//!
//! Implemented from the published model of Newson, Delon & Galerne, "Realistic Film Grain
//! Rendering" (IPOL 2017): grains are discs whose centres form a Poisson process; their density
//! follows the local tone so that a point is covered with probability `p` (the expected grain
//! coverage, `λ·E[πr²] = −ln(1 − p)`), and what a pixel shows is the coverage averaged over its
//! footprint. Here every grain is a disc softened by that footprint (the analytic stand-in for the
//! paper's Monte-Carlo Gaussian filter), and the union of grains is evaluated exactly as
//! `1 − Π(1 − aᵢ)`, so the mean is `p` at any scale and the variance has a closed form:
//! `Var = (1 − p)²·(exp(λ·E[∫a²]) − 1)` (the Poisson probability generating functional).
//!
//! Resolution independence: grain geometry lives in normalized frame coordinates (a fraction of the
//! uncropped frame's long edge, like a grain on a 35 mm negative) and only the softening — one
//! output pixel — depends on the render size. A small render therefore shows what the export
//! shows downscaled: the same mean, the same (lower) variance, a near-white spectrum. Once grains
//! are much smaller than a pixel (grid thumbnails, fit-to-window previews of fine grain) the field
//! is replaced by per-pixel Gaussian noise with that exact variance, cross-faded over a band so
//! the switch never shows.
//!
//! Shared with the GPU kernel (`crates/gpu/src/wgsl/finish.wgsl`): keep both in step.

use lightcraft_develop::Grain;

/// Hash constants (shared with the GPU kernel): PCG-style permutation and per-layer seed salts.
pub const GRAIN_HASH: [u32; 4] = [0x2c9277b5, 0xac564b05, 0x9e37_79b9, 0x85eb_ca6b];

/// Coverage range: grains always overlap into irregular clumps and pores (a sparse field would
/// read as isolated dots, a saturated one as pores in black): light tones are covered less.
pub const P_MIN: f32 = 0.2;
pub const P_MAX: f32 = 0.8;

/// Grain amplitude in the deepest shadows and brightest highlights relative to the midtones.
pub const TONE_FLOOR: f32 = 0.35;

/// Grains a cell may hold (a bound for the loop, far above any reachable mean).
pub const MAX_PER_CELL: u32 = 48;

/// The softening one output pixel applies, as the edge width of a grain (pixels): the width at
/// which a grain much smaller than a pixel spreads over the noise-equivalent area of a one-pixel
/// box (`(∫a)² / ∫a² = 1 px²`), so a small render has the variance of a box-downscaled export.
pub const PIXEL_SOFTNESS: f64 = 0.389_327_8;

/// Grain units are mean grain radii; the reference resolution for the Amount calibration is a
/// 6000 px long edge (a 24 MP export).
const REF_PX_PER_LONG: f64 = 6000.0;

/// Explicit grains while the softening is below this many radii, Gaussian noise above the second
/// bound, cross-faded between.
const EXPLICIT_UPTO: f64 = 1.0;
const GAUSSIAN_FROM: f64 = 1.8;

/// Opponent colour directions for colour grain (orthogonal to neutral grey).
pub const CHROMA_DIRS: [[f32; 3]; 2] = [[0.707_106_77, -0.707_106_77, 0.0], [0.408_248_3, 0.408_248_3, -0.816_496_6]];

/// One render's grain parameters (see the module docs). Lengths are in grain units (mean radius 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainParams {
    /// Output scale: brightness change per unit of coverage deviation.
    pub amp: f32,
    /// Normalized long-edge units → grain units (`1 / mean radius`).
    pub inv_r: f32,
    /// Edge softness (grain units): film softness and the output pixel's footprint.
    pub s: f32,
    /// Log-normal-like spread of grain radii.
    pub disp: f32,
    /// `π·E[r²]·E[o]`: mean grain area times mean density (coverage per grain).
    pub area: f32,
    /// Largest distance at which a grain still covers a point.
    pub support: f32,
    /// Side of a Poisson cell (the support: a point sees at most 3 × 3 cells).
    pub cell: f32,
    /// `E[∫a²] / E[∫a]` for this softness (the variance formula's exponent per unit coverage).
    pub kappa: f32,
    /// Cross-fade weights of the explicit field and of the Gaussian noise (`cos² + sin² = 1`).
    pub w_field: f32,
    pub w_noise: f32,
    /// Colour (chroma) grain, 0..1.
    pub color: f32,
    /// Tone response exponent: > 1 moves grain into the shadows, < 1 into the highlights.
    pub gamma: f32,
    pub seed: u32,
    /// Output pixels per normalized long edge (for the per-pixel noise).
    pub px_per_long: f32,
}

fn finite(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

/// Mean grain radius as a fraction of the frame's long edge for the Size slider (0..100): about
/// 4 µm on a 35 mm frame at 0, 10 µm at the default 25, 48 µm at 100.
pub fn radius_for_size(size: f64) -> f64 {
    let t = finite(size, 0.0, 100.0, 25.0) / 100.0;
    0.000_12 + t.powf(1.4) * 0.001_2
}

/// `∫(ρ+wτ)·F(τ)² dτ` moments of the edge profile `F` over τ ∈ [−1, 1]: (∫F², ∫τF²).
fn edge_moments() -> (f64, f64) {
    const N: usize = 256;
    let (mut i0, mut i1) = (0.0, 0.0);
    for k in 0..N {
        let tau = -1.0 + (k as f64 + 0.5) * 2.0 / N as f64;
        let f = edge(tau);
        i0 += f * f;
        i1 += tau * f * f;
    }
    (i0 * 2.0 / N as f64, i1 * 2.0 / N as f64)
}

/// The edge profile: 1 inside, 0 outside, a smoothstep across τ ∈ [−1, 1].
#[inline]
fn edge(tau: f64) -> f64 {
    if tau <= -1.0 {
        1.0
    } else if tau >= 1.0 {
        0.0
    } else {
        (1.0 - tau) * (1.0 - tau) * (2.0 + tau) * 0.25
    }
}

/// `(π·E[r²], E[∫a²] / π·E[r²])` for radius spread `disp` and softness `s` (grain units), over the
/// radius distribution the renderer draws: `r = exp(disp·z)`, `z = 2(u₁+u₂+u₃) − 3`.
fn area_and_kappa(disp: f64, s: f64) -> (f64, f64) {
    const N: usize = 12;
    let (i0, i1) = edge_moments();
    let (mut ar, mut a2) = (0.0, 0.0);
    for a in 0..N {
        for b in 0..N {
            for c in 0..N {
                let z = 2.0 * ((a + b + c) as f64 + 1.5) / N as f64 - 3.0;
                let r = (disp * z).exp();
                let rho2 = r * r + s * s;
                let rho = rho2.sqrt();
                let h = r * r / (rho2 + s * s / 5.0);
                // ∫a² = h²·(π(ρ−w)² + 2πw(ρ·I0 + w·I1)), w = s
                let j = std::f64::consts::PI * (rho - s) * (rho - s) + 2.0 * std::f64::consts::PI * s * (rho * i0 + s * i1);
                // grain densities 1/4, 1/2, 3/4, 1 equally often: E[o] = 5/8, E[o²] = 15/32
                ar += std::f64::consts::PI * r * r * 0.625;
                a2 += h * h * j * 0.468_75;
            }
        }
    }
    let n = (N * N * N) as f64;
    let area = ar / n;
    (area, if area > 0.0 { (a2 / n / area).clamp(1e-6, 1.0) } else { 1.0 })
}

/// Standard deviation of the coverage at `p` for exponent `kappa` (fade correction off).
fn std_base(p: f64, kappa: f64) -> f64 {
    let q = 1.0 - p;
    q * (q.powf(-kappa) - 1.0).max(0.0).sqrt()
}

impl GrainParams {
    /// Parameters for a render with `px_per_long` output pixels per long edge of the uncropped
    /// frame; `None` when there is no grain. Out-of-range or non-finite settings are clamped.
    pub fn new(g: &Grain, px_per_long: f64) -> Option<GrainParams> {
        let amount = finite(g.amount, 0.0, 100.0, 0.0) / 100.0;
        let ppl = finite(px_per_long, 1.0, 1e7, 1.0);
        if amount <= 0.0 {
            return None;
        }
        let size = finite(g.size, 0.0, 100.0, 25.0);
        let rough = finite(g.roughness, 0.0, 100.0, 50.0) / 100.0;
        let r_norm = radius_for_size(size);
        let disp = 0.35 * rough;
        let film_soft = 0.7 - 0.4 * rough;
        let soft_at = |ppl: f64| (film_soft * film_soft + (PIXEL_SOFTNESS / (r_norm * ppl)).powi(2)).sqrt();
        let s = soft_at(ppl);
        let (area, kappa) = area_and_kappa(disp, s);
        let (_, kappa_ref) = area_and_kappa(disp, soft_at(REF_PX_PER_LONG));
        // Amount 100 ≈ 0.06 (8-bit: 15 levels) standard deviation in mid-grey at the reference
        // resolution, a little more for coarse grain
        let target = amount * 0.055 * (0.8 + 0.4 * size / 100.0);
        let amp = target / std_base(0.5, kappa_ref).max(1e-4);
        let t = ((s - EXPLICIT_UPTO) / (GAUSSIAN_FROM - EXPLICIT_UPTO)).clamp(0.0, 1.0);
        let (w_field, w_noise) = if t <= 0.0 {
            (1.0, 0.0)
        } else if t >= 1.0 {
            (0.0, 1.0)
        } else {
            let a = t * std::f64::consts::FRAC_PI_2;
            (a.cos(), a.sin())
        };
        let r_max = (3.0 * disp).exp();
        let support = (r_max * r_max + s * s).sqrt() + s;
        Some(GrainParams {
            amp: amp as f32,
            inv_r: (1.0 / r_norm) as f32,
            s: s as f32,
            disp: disp as f32,
            area: area as f32,
            support: support as f32,
            cell: support as f32,
            kappa: kappa as f32,
            w_field: w_field as f32,
            w_noise: w_noise as f32,
            color: (finite(g.color, 0.0, 100.0, 0.0) / 100.0) as f32,
            gamma: 2f64.powf(-finite(g.response, -100.0, 100.0, 0.0) / 50.0) as f32,
            seed: g.seed,
            px_per_long: ppl as f32,
        })
    }

    /// Expected coverage for display-encoded luminance `lum` (grain is density: dark tones are
    /// covered more).
    #[inline]
    pub fn coverage(&self, lum: f32) -> f32 {
        let l = if lum.is_finite() { lum.clamp(0.0, 1.0) } else { 0.5 };
        P_MIN + (P_MAX - P_MIN) * self.shade(l)
    }

    /// `(1 − l)^γ` for luminance `l` in 0..1 (no `powf` at the default response).
    #[inline]
    fn shade(&self, l: f32) -> f32 {
        let d = (1.0 - l).max(1e-7);
        if self.gamma == 1.0 { d } else { d.powf(self.gamma) }
    }

    /// Amplitude weight for display-encoded luminance `lum`: 1 in the midtones (moved by the
    /// response), [`TONE_FLOOR`] at black and white.
    #[inline]
    pub fn tone_weight(&self, lum: f32) -> f32 {
        let l = if lum.is_finite() { lum.clamp(0.0, 1.0) } else { 0.5 };
        let m = 1.0 - self.shade(l);
        TONE_FLOOR + (1.0 - TONE_FLOOR) * 4.0 * m * (1.0 - m)
    }

    /// Expected grains per Poisson cell at coverage `p`.
    #[inline]
    fn per_cell(&self, p: f32) -> f32 {
        -(1.0 - p).ln() / self.area * self.cell * self.cell
    }

    /// Standard deviation of the coverage the explicit field has at `p` (the Gaussian noise's
    /// amplitude): `(1 − p)·√(exp(κ·μ') − 1)`, where `μ'` counts fading-in grains by `f²`.
    #[inline]
    pub fn coverage_std(&self, p: f32) -> f32 {
        let t = self.per_cell(p);
        let fw = t.min(1.0);
        let mu = -(1.0 - p).ln() * (1.0 - fw / (6.0 * t.max(1e-12)));
        (1.0 - p) * ((self.kappa * mu).exp() - 1.0).max(0.0).sqrt()
    }

    /// Grain coverage at grain-unit position (x, y) for expected coverage `p` and layer seed.
    pub fn field(&self, x: f32, y: f32, p: f32, seed: u32) -> f32 {
        let t = self.per_cell(p);
        // grains fade in over the last `fw` of the cell's mark range, so the field changes
        // continuously with tone (no grain pops in or out); the shift keeps the mean count at t
        let fw = t.clamp(1e-6, 1.0);
        let end = t + 0.5 * fw;
        let (r, c, s) = (self.support, self.cell, self.s);
        let (i0, j0) = (((x - r) / c).floor() as i32, ((y - r) / c).floor() as i32);
        let (i1, j1) = ((((x + r) / c).floor() as i32).min(i0.saturating_add(6)), (((y + r) / c).floor() as i32).min(j0.saturating_add(6)));
        // a cell holds grains while the running product of their uniforms stays above
        // exp(−end) (Poisson arrivals in mark order, `mₖ = −ln Π uᵢ`); full weight above exp(fw − end)
        let (e_end, e_full) = ((-end).exp(), (fw - end).exp());
        let salt = pcg(seed);
        let mut keep = 1.0f32;
        for j in j0..=j1 {
            let (cy0, cy1) = (j as f32 * c, (j as f32 + 1.0) * c);
            let dy = y - y.clamp(cy0, cy1);
            for i in i0..=i1 {
                let (cx0, cx1) = (i as f32 * c, (i as f32 + 1.0) * c);
                let dx = x - x.clamp(cx0, cx1);
                if dx * dx + dy * dy >= r * r {
                    continue;
                }
                let base = cell_hash(i, j, salt);
                let mut prod = 1.0f32;
                for k in 0..MAX_PER_CELL {
                    let h = pcg(base.wrapping_add(k));
                    prod *= unit(h);
                    if prod <= e_end {
                        break;
                    }
                    let pos = pcg(h ^ GRAIN_HASH[2]);
                    let gx = (i as f32 + unit16(pos)) * c;
                    let gy = (j as f32 + unit16(pos >> 16)) * c;
                    let d2 = (x - gx) * (x - gx) + (y - gy) * (y - gy);
                    if d2 >= r * r {
                        continue;
                    }
                    let rad = pcg(h ^ GRAIN_HASH[3]);
                    let z = 2.0 * (unit10(rad) + unit10(rad >> 10) + unit10(rad >> 20)) - 3.0;
                    let ri = (self.disp * z).exp();
                    let rho2 = ri * ri + s * s;
                    let tau = (d2.sqrt() - rho2.sqrt()) / s;
                    if tau >= 1.0 {
                        continue;
                    }
                    let prof = if tau <= -1.0 { 1.0 } else { (1.0 - tau) * (1.0 - tau) * (2.0 + tau) * 0.25 };
                    let f = if prod >= e_full { 1.0 } else { ((end + prod.ln()) / fw).clamp(0.0, 1.0) };
                    keep *= 1.0 - f * density(rad) * ri * ri / (rho2 + s * s * 0.2) * prof;
                }
            }
        }
        1.0 - keep
    }

    /// [`GrainParams::field`] for a band of output rows: `xs`, `ys` are the grain-unit positions of
    /// `rows × cols` pixels, row-major, on an affine grid (as an output window is), `ps` their
    /// expected coverages; the coverage goes to `out`. Visits every cell near the band once and
    /// splats its grains onto the pixels they reach, testing each grain against each pixel
    /// exactly as `field` does (same grains, same weights; only the product order differs).
    pub fn field_band(&self, xs: &[f32], ys: &[f32], cols: usize, ps: &[f32], seed: u32, out: &mut [f32]) {
        let len = xs.len().min(ys.len()).min(ps.len()).min(out.len());
        let rows = len.checked_div(cols).unwrap_or(0);
        let n = rows * cols;
        let (r, c, s) = (self.support, self.cell, self.s);
        let corner = |row: usize, k: usize| (xs.get(row * cols + k).copied(), ys.get(row * cols + k).copied());
        let ((Some(x00), Some(y00)), (Some(x01), Some(y01)), (Some(x10), Some(y10))) =
            (corner(0, 0), corner(0, cols.wrapping_sub(1)), corner(rows.wrapping_sub(1), 0))
        else {
            return;
        };
        let finite = [x00, y00, x01, y01, x10, y10, r, c, s].iter().all(|v| v.is_finite()) && c > 0.0 && s > 0.0;
        let span = ((x01 - x00).abs().max((y01 - y00).abs()).max((x10 - x00).abs()).max((y10 - y00).abs()) / c.max(1e-6)) as usize;
        if !finite || span > 16 * (cols + rows) + 64 {
            // not a well-formed window (never for a render): pixel by pixel
            for (k, o) in out.iter_mut().enumerate().take(n) {
                *o = self.field(xs[k], ys[k], ps[k], seed);
            }
            return;
        }
        // along a row (bx, by) per pixel; from row to row (dx, dy)
        let (bx, by) = ((x01 - x00) / (cols - 1).max(1) as f32, (y01 - y00) / (cols - 1).max(1) as f32);
        let (dx, dy) = ((x10 - x00) / (rows - 1).max(1) as f32, (y10 - y00) / (rows - 1).max(1) as f32);
        let bb = bx * bx + by * by;
        let inv_b = if bb > 1e-24 { 1.0 / bb.sqrt() } else { 0.0 };
        // per-pixel thresholds as in `field`
        let mut ends = Vec::with_capacity(n);
        let mut e_min = 1.0f32;
        for &p in &ps[..n] {
            let t = self.per_cell(p);
            let fw = t.clamp(1e-6, 1.0);
            let end = t + 0.5 * fw;
            let e_end = (-end).exp();
            e_min = e_min.min(e_end);
            ends.push((end, fw, e_end, (fw - end).exp()));
        }
        let keep = &mut out[..n];
        keep.fill(1.0);
        let salt = pcg(seed);
        let last = (cols - 1) as f32;
        // pixel index range (clamped, padded by one) for a parameter interval along a row
        let krange = |a: f32, b: f32| -> Option<(usize, usize)> {
            let (lo, hi) = (a.min(b).floor() - 1.0, a.max(b).ceil() + 1.0);
            if hi.is_nan() || lo.is_nan() || hi < 0.0 || lo > last {
                return None;
            }
            Some((lo.max(0.0) as usize, (hi.min(last) as usize).min(cols - 1)))
        };
        // the band is the parallelogram of its pixel centres; cells within the support of it
        let quad = [(x00, y00), (x01, y01), (x10 + (x01 - x00), y10 + (y01 - y00)), (x10, y10)];
        let (qx0, qx1) = quad.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), q| (a.min(q.0), b.max(q.0)));
        let (i0, i1) = (((qx0 - r) / c).floor() as i32, ((qx1 + r) / c).floor() as i32);
        for i in i0..=i1 {
            // the parallelogram's Y extent within this column of cells (grown by the support):
            // the extremes lie on its edges clipped to the column
            let (cx0, cx1) = (i as f32 * c - r, (i as f32 + 1.0) * c + r);
            let (mut y_lo, mut y_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for e in 0..4 {
                let ((ax, ay), (bx2, by2)) = (quad[e], quad[(e + 1) % 4]);
                let ex = bx2 - ax;
                let (t0, t1) = if ex.abs() > 1e-12 {
                    let (u, v) = ((cx0 - ax) / ex, (cx1 - ax) / ex);
                    (u.min(v).max(0.0), u.max(v).min(1.0))
                } else if ax >= cx0 && ax <= cx1 {
                    (0.0, 1.0)
                } else {
                    continue;
                };
                if t0 > t1 {
                    continue;
                }
                for t in [t0, t1] {
                    let yt = ay + t * (by2 - ay);
                    y_lo = y_lo.min(yt);
                    y_hi = y_hi.max(yt);
                }
            }
            if y_lo.is_nan() || y_hi.is_nan() || y_lo > y_hi {
                continue;
            }
            let (j0, j1) = (((y_lo - r) / c).floor() as i32, ((y_hi + r) / c).floor() as i32);
            for j in j0..=j1 {
                let base = cell_hash(i, j, salt);
                let mut prod = 1.0f32;
                for k in 0..MAX_PER_CELL {
                    let h = pcg(base.wrapping_add(k));
                    prod *= unit(h);
                    if prod <= e_min {
                        break;
                    }
                    let pos = pcg(h ^ GRAIN_HASH[2]);
                    let gx = (i as f32 + unit16(pos)) * c;
                    let gy = (j as f32 + unit16(pos >> 16)) * c;
                    let mut radius: Option<(f32, f32, f32)> = None;
                    let mut ln_prod = None;
                    for row in 0..rows {
                        let (ddx, ddy) = (gx - (x00 + row as f32 * dx), gy - (y00 + row as f32 * dy));
                        // distance from the row's line (cross product: no cancellation far along it)
                        let perp = if inv_b > 0.0 { (ddx * by - ddy * bx) * inv_b } else { (ddx * ddx + ddy * ddy).sqrt() };
                        if perp.abs() >= r {
                            continue;
                        }
                        let (rho, height, reach) = *radius.get_or_insert_with(|| {
                            let rad = pcg(h ^ GRAIN_HASH[3]);
                            let z = 2.0 * (unit10(rad) + unit10(rad >> 10) + unit10(rad >> 20)) - 3.0;
                            let ri = (self.disp * z).exp();
                            let rho2 = ri * ri + s * s;
                            let rho = rho2.sqrt();
                            (rho, density(rad) * ri * ri / (rho2 + s * s * 0.2), rho + s)
                        });
                        if perp.abs() >= reach {
                            continue;
                        }
                        // the pixels within reach: the chord of the grain's disc along the row
                        let near = if inv_b > 0.0 {
                            let kc = (ddx * bx + ddy * by) / bb;
                            let half = (reach * reach - perp * perp).max(0.0).sqrt() * inv_b;
                            krange(kc - half, kc + half)
                        } else {
                            Some((0, cols - 1))
                        };
                        let Some((pa, pb)) = near else { continue };
                        for q in row * cols + pa..=row * cols + pb {
                            let (Some(&(end, fw, e_end, e_full)), Some(&x), Some(&y)) = (ends.get(q), xs.get(q), ys.get(q)) else { continue };
                            if prod <= e_end {
                                continue;
                            }
                            let d2 = (x - gx) * (x - gx) + (y - gy) * (y - gy);
                            if d2 >= r * r {
                                continue;
                            }
                            let tau = (d2.sqrt() - rho) / s;
                            if tau >= 1.0 {
                                continue;
                            }
                            let prof = if tau <= -1.0 { 1.0 } else { (1.0 - tau) * (1.0 - tau) * (2.0 + tau) * 0.25 };
                            let f = if prod >= e_full { 1.0 } else { ((end + *ln_prod.get_or_insert_with(|| prod.ln())) / fw).clamp(0.0, 1.0) };
                            if let Some(kq) = keep.get_mut(q) {
                                *kq *= 1.0 - f * height * prof;
                            }
                        }
                    }
                }
            }
        }
        for v in keep.iter_mut() {
            *v = 1.0 - *v;
        }
    }

    /// [`GrainParams::apply`] for a band of output rows: adds grain to the display-encoded colours
    /// `e` of `rows × cols` pixels (row-major) at normalized positions `gx`, `gy` (an affine grid).
    pub fn apply_band(&self, gx: &[f32], gy: &[f32], cols: usize, e: &mut [[f32; 3]]) {
        let n = gx.len().min(gy.len()).min(e.len());
        let lums: Vec<f32> = e[..n].iter().map(|c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).collect();
        let ps: Vec<f32> = lums.iter().map(|l| self.coverage(*l)).collect();
        let amps: Vec<f32> = lums.iter().map(|l| self.amp * self.tone_weight(*l)).collect();
        let (xs, ys): (Vec<f32>, Vec<f32>) = gx[..n].iter().zip(&gy[..n]).map(|(x, y)| (x * self.inv_r, y * self.inv_r)).unzip();
        let mut v = vec![0.0f32; n];
        // luminance layer, then the two chroma layers (opponent directions)
        let layers: &[(u32, Option<usize>)] =
            if self.color > 0.0 { &[(0, None), (GRAIN_HASH[2], Some(0)), (GRAIN_HASH[3], Some(1))] } else { &[(0, None)] };
        for &(salt, dir) in layers {
            let seed = self.seed ^ salt;
            if self.w_field > 0.0 {
                self.field_band(&xs, &ys, cols, &ps, seed, &mut v);
            }
            for k in 0..n {
                let (x, y, p) = (gx[k], gy[k], ps[k]);
                if !(x.is_finite() && y.is_finite()) {
                    continue;
                }
                let mut d = 0.0;
                if self.w_field > 0.0 {
                    d += self.w_field * (p - v[k]);
                }
                if self.w_noise > 0.0 {
                    let (ix, iy) = ((x * self.px_per_long).floor() as i32, (y * self.px_per_long).floor() as i32);
                    d -= self.w_noise * self.coverage_std(p) * gauss(ix, iy, seed);
                }
                let c = &mut e[k];
                let (amt, dirs) = match dir {
                    None => (amps[k] * d, [1.0; 3]),
                    Some(m) => (amps[k] * self.color * d, CHROMA_DIRS[m]),
                };
                *c = [c[0] + amt * dirs[0], c[1] + amt * dirs[1], c[2] + amt * dirs[2]];
            }
        }
    }

    /// Brightness deviation of one grain layer at normalized position (gx, gy).
    #[inline]
    fn layer(&self, gx: f32, gy: f32, p: f32, seed: u32) -> f32 {
        let mut d = 0.0;
        if self.w_field > 0.0 {
            d += self.w_field * (p - self.field(gx * self.inv_r, gy * self.inv_r, p, seed));
        }
        if self.w_noise > 0.0 {
            let (ix, iy) = ((gx * self.px_per_long).floor() as i32, (gy * self.px_per_long).floor() as i32);
            d -= self.w_noise * self.coverage_std(p) * gauss(ix, iy, seed);
        }
        d
    }

    /// What grain adds to each display-encoded channel at normalized position (gx, gy) (fractions
    /// of the uncropped frame's long edge) for a pixel of display-encoded luminance `lum`.
    pub fn apply(&self, gx: f32, gy: f32, lum: f32) -> [f32; 3] {
        if !(gx.is_finite() && gy.is_finite()) {
            return [0.0; 3];
        }
        let p = self.coverage(lum);
        let amp = self.amp * self.tone_weight(lum);
        let l = amp * self.layer(gx, gy, p, self.seed);
        if self.color <= 0.0 {
            return [l; 3];
        }
        let k = amp * self.color;
        let c1 = k * self.layer(gx, gy, p, self.seed ^ GRAIN_HASH[2]);
        let c2 = k * self.layer(gx, gy, p, self.seed ^ GRAIN_HASH[3]);
        std::array::from_fn(|ch| l + c1 * CHROMA_DIRS[0][ch] + c2 * CHROMA_DIRS[1][ch])
    }
}

/// PCG output permutation (RXS-M-XS) of an LCG step.
#[inline]
pub fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

/// Hash of cell (i, j) for a layer `salt` (`pcg(seed)`).
#[inline]
fn cell_hash(i: i32, j: i32, salt: u32) -> u32 {
    pcg((i as u32).wrapping_mul(GRAIN_HASH[0]) ^ (j as u32).wrapping_mul(GRAIN_HASH[1]) ^ salt)
}

/// A uniform in (0, 1) from the top 24 bits.
#[inline]
fn unit(v: u32) -> f32 {
    ((v >> 8) as f32 + 0.5) * (1.0 / 16_777_216.0)
}

/// A grain's optical density (1/4, 1/2, 3/4 or 1) from the top bits of its radius hash: grains
/// at different depths of the emulsion overlap into continuous tones, not two flat levels.
#[inline]
fn density(rad: u32) -> f32 {
    0.25 + 0.25 * (rad >> 30) as f32
}

/// A uniform in (0, 1) from the low 16 bits.
#[inline]
fn unit16(v: u32) -> f32 {
    ((v & 0xffff) as f32 + 0.5) * (1.0 / 65_536.0)
}

/// A uniform in (0, 1) from the low 10 bits.
#[inline]
fn unit10(v: u32) -> f32 {
    ((v & 0x3ff) as f32 + 0.5) * (1.0 / 1024.0)
}

/// Unit-variance, near-Gaussian noise for pixel (ix, iy): the sum of four uniforms.
#[inline]
fn gauss(ix: i32, iy: i32, seed: u32) -> f32 {
    // four 8-bit uniforms from one hash (their sum is smooth enough at a pixel's scale)
    let h = cell_hash(ix, iy, pcg(seed ^ 0x5bd1_e995));
    let sum = (h & 0xff) + ((h >> 8) & 0xff) + ((h >> 16) & 0xff) + (h >> 24);
    ((sum as f32 + 2.0) * (1.0 / 256.0) - 2.0) * 1.732_050_8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grain(amount: f64, size: f64, roughness: f64) -> Grain {
        Grain { amount, size, roughness, ..Default::default() }
    }

    /// Brightness deviations of the luminance layer over a `n × n` pixel patch for a render with
    /// `ppl` px per long edge, at constant luminance `lum`.
    fn patch(gp: &GrainParams, n: usize, lum: f32) -> Vec<f32> {
        let ppl = gp.px_per_long;
        (0..n * n).map(|i| gp.apply(((i % n) as f32 + 0.5) / ppl + 0.1003, ((i / n) as f32 + 0.5) / ppl + 0.1007, lum)[0]).collect()
    }

    fn stats(v: &[f32]) -> (f64, f64) {
        let n = v.len() as f64;
        let mean = v.iter().map(|x| *x as f64).sum::<f64>() / n;
        let var = v.iter().map(|x| (*x as f64 - mean).powi(2)).sum::<f64>() / n;
        (mean, var)
    }

    /// Correlation of horizontally adjacent pixels.
    fn lag1(v: &[f32], n: usize) -> f64 {
        let (m, var) = stats(v);
        let mut c = 0.0;
        for y in 0..n {
            for x in 0..n - 1 {
                c += (v[y * n + x] as f64 - m) * (v[y * n + x + 1] as f64 - m);
            }
        }
        c / (n * (n - 1)) as f64 / var.max(1e-30)
    }

    fn box_down(v: &[f32], n: usize, k: usize) -> Vec<f32> {
        let m = n / k;
        (0..m * m)
            .map(|i| {
                let (x, y) = (i % m, i / m);
                let mut s = 0.0;
                for dy in 0..k {
                    for dx in 0..k {
                        s += v[(y * k + dy) * n + x * k + dx];
                    }
                }
                s / (k * k) as f32
            })
            .collect()
    }

    #[test]
    fn field_mean_is_the_expected_coverage_and_variance_matches_theory() {
        for (size, rough) in [(25.0, 50.0), (80.0, 0.0), (60.0, 100.0)] {
            let gp = GrainParams::new(&grain(50.0, size, rough), 6000.0).unwrap();
            assert_eq!((gp.w_field, gp.w_noise), (1.0, 0.0), "explicit at full size");
            for p in [0.05f32, 0.3, 0.6, 0.88] {
                let n = 220;
                let v: Vec<f32> = (0..n * n).map(|i| gp.field((i % n) as f32 * 0.7, (i / n) as f32 * 0.7, p, 7)).collect();
                let (mean, var) = stats(&v);
                let sd = gp.coverage_std(p) as f64;
                assert!((mean - p as f64).abs() < 0.02 + 0.1 * sd, "size {size} p {p}: mean {mean}");
                assert!((var.sqrt() / sd - 1.0).abs() < 0.15, "size {size} p {p}: sd {} vs theory {sd}", var.sqrt());
            }
        }
    }

    /// A small render shows the grain a full-size export would show downscaled to that size: the
    /// same mean, a variance and a pixel-to-pixel correlation within tolerance, for both the
    /// explicit field and the Gaussian stand-in (fine grain on a small render).
    #[test]
    fn small_render_matches_a_downscaled_full_size_render() {
        for (size, k) in [(100.0, 4usize), (60.0, 3), (25.0, 4), (0.0, 3), (25.0, 8)] {
            let full = GrainParams::new(&grain(60.0, size, 50.0), 3000.0).unwrap();
            let small = GrainParams::new(&grain(60.0, size, 50.0), 3000.0 / k as f64).unwrap();
            assert_eq!(full.amp, small.amp, "Amount is calibrated independently of the render size");
            let n = 120;
            for lum in [0.25f32, 0.5, 0.8] {
                let big = patch(&full, n * k, lum);
                let down = box_down(&big, n * k, k);
                let sm = patch(&small, n, lum);
                let ((m0, v0), (m1, v1)) = (stats(&down), stats(&sm));
                let sd = v0.sqrt().max(v1.sqrt());
                assert!((m0 - m1).abs() < 0.25 * sd + 2e-4, "size {size} ÷{k} lum {lum}: mean {m0} vs {m1}");
                let ratio = (v1 / v0.max(1e-30)).sqrt();
                eprintln!(
                    "size {size} ÷{k} lum {lum}: sd ratio {ratio:.3}  lag1 {:.3} vs {:.3}  regimes {}/{}",
                    lag1(&down, n),
                    lag1(&sm, n),
                    full.w_field,
                    small.w_field
                );
                assert!((0.75..1.33).contains(&ratio), "size {size} ÷{k} lum {lum}: sd ratio {ratio} ({} vs {})", v1.sqrt(), v0.sqrt());
                let (c0, c1) = (lag1(&down, n), lag1(&sm, n));
                assert!((c0 - c1).abs() < 0.2, "size {size} ÷{k} lum {lum}: lag-1 correlation {c0} vs {c1}");
            }
        }
    }

    #[test]
    fn grain_is_strongest_in_the_midtones_and_response_moves_it() {
        let sd = |gp: &GrainParams, lum: f32| stats(&patch(gp, 160, lum)).1.sqrt();
        let gp = GrainParams::new(&grain(50.0, 30.0, 50.0), 6000.0).unwrap();
        let (dark, mid, light) = (sd(&gp, 0.05), sd(&gp, 0.5), sd(&gp, 0.95));
        assert!(mid > dark && mid > light && dark > 0.0 && light > 0.0, "{dark} {mid} {light}");
        let shadows = GrainParams::new(&Grain { response: -100.0, ..grain(50.0, 30.0, 50.0) }, 6000.0).unwrap();
        let highlights = GrainParams::new(&Grain { response: 100.0, ..grain(50.0, 30.0, 50.0) }, 6000.0).unwrap();
        assert!(sd(&shadows, 0.15) > sd(&shadows, 0.85), "negative response favours the shadows");
        assert!(sd(&highlights, 0.85) > sd(&highlights, 0.15), "positive response favours the highlights");
        // Amount 50 in mid-grey at the reference resolution: about 0.03 (8 levels of 255)
        assert!((0.02..0.045).contains(&mid), "mid-grey sd {mid}");
    }

    #[test]
    fn colour_grain_is_chroma_only_at_zero_and_coloured_above() {
        let mono = GrainParams::new(&grain(50.0, 30.0, 50.0), 2000.0).unwrap();
        let col = GrainParams::new(&Grain { color: 100.0, ..grain(50.0, 30.0, 50.0) }, 2000.0).unwrap();
        let (mut chroma, mut lum_diff) = (0.0f64, 0.0f64);
        for i in 0..4000 {
            let (x, y) = ((i % 63) as f32 / 2000.0, (i / 63) as f32 / 2000.0);
            let m = mono.apply(x, y, 0.5);
            assert!(m[0] == m[1] && m[1] == m[2]);
            let c = col.apply(x, y, 0.5);
            chroma += ((c[0] - c[1]).abs() + (c[1] - c[2]).abs()) as f64;
            // the chroma directions are neutral on average: the mean of the channels is the mono grain
            lum_diff += ((c[0] + c[1] + c[2]) / 3.0 - m[0]).abs() as f64;
        }
        assert!(chroma / 4000.0 > 0.01, "colour grain differs between channels: {}", chroma / 4000.0);
        assert!(lum_diff / 4000.0 < 1e-6, "{}", lum_diff / 4000.0);
    }

    #[test]
    fn the_field_changes_continuously_with_tone() {
        let gp = GrainParams::new(&grain(50.0, 40.0, 50.0), 6000.0).unwrap();
        let mut worst = 0.0f32;
        for i in 0..2000 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.11);
            for k in 0..100 {
                let p = 0.05 + k as f32 * 0.008;
                let jump = (gp.field(x, y, p + 1e-4, 3) - gp.field(x, y, p, 3)).abs();
                worst = worst.max(jump);
            }
        }
        assert!(worst < 0.02, "largest change for Δp = 1e-4: {worst}");
    }

    /// The band evaluation (CPU renders) gives what the per-pixel one (the GPU kernel's) gives,
    /// for straight, rotated, flipped and transposed bands, with and without colour grain.
    #[test]
    fn band_matches_per_pixel() {
        for (size, ppl, color) in [(25.0, 6000.0, 0.0), (90.0, 2000.0, 70.0), (0.0, 6000.0, 0.0), (25.0, 900.0, 30.0)] {
            let gp = GrainParams::new(&Grain { response: 30.0, color, ..grain(70.0, size, 80.0) }, ppl).unwrap();
            // (along a row, from row to row) per pixel, in output pixels
            for ((ax, ay), (rx, ry)) in
                [((1.0f32, 0.0f32), (0.0f32, 1.0f32)), ((0.97, 0.26), (-0.26, 0.97)), ((0.0, 1.0), (1.0, 0.0)), ((-0.6, 0.8), (0.8, 0.6))]
            {
                let (cols, rows) = (500, 7);
                let mut gx = Vec::new();
                let mut gy = Vec::new();
                for row in 0..rows {
                    for k in 0..cols {
                        let (u, v) = (k as f32 + 0.5, row as f32 + 0.5);
                        gx.push(0.2 + (u * ax + v * rx) / ppl as f32);
                        gy.push(0.3 + (u * ay + v * ry) / ppl as f32);
                    }
                }
                let mut px: Vec<[f32; 3]> = (0..cols * rows).map(|k| [0.1 + 0.8 * (k % 97) as f32 / 97.0; 3]).collect();
                let want: Vec<[f32; 3]> = px
                    .iter()
                    .enumerate()
                    .map(|(k, c)| {
                        let g = gp.apply(gx[k], gy[k], 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
                        [c[0] + g[0], c[1] + g[1], c[2] + g[2]]
                    })
                    .collect();
                gp.apply_band(&gx, &gy, cols, &mut px);
                for k in 0..cols * rows {
                    for ch in 0..3 {
                        assert!((px[k][ch] - want[k][ch]).abs() < 2e-5, "size {size} dir ({ax}, {ay}) px {k}: {:?} vs {:?}", px[k], want[k]);
                    }
                }
            }
        }
    }

    /// Settings saved before colour grain and the response existed (catalog, XMP, presets) load
    /// with both off and render the same grain as settings that name them at their defaults.
    #[test]
    fn settings_without_the_new_fields_are_back_compatible() {
        let old: lightcraft_develop::DevelopSettings =
            serde_json::from_value(serde_json::json!({"grain": {"amount": 20.0, "size": 30.0, "roughness": 60.0, "seed": 9}})).unwrap();
        assert_eq!((old.grain.color, old.grain.response), (0.0, 0.0));
        let explicit = Grain { amount: 20.0, size: 30.0, roughness: 60.0, seed: 9, color: 0.0, response: 0.0 };
        assert_eq!(old.grain, explicit);
        let (a, b) = (GrainParams::new(&old.grain, 3000.0).unwrap(), GrainParams::new(&explicit, 3000.0).unwrap());
        assert_eq!(a, b);
        assert_eq!((a.color, a.gamma), (0.0, 1.0), "no colour grain, midtone response");
    }

    /// Through the whole pipeline: a 1/4-size render of a flat grey photo shows the grain of the
    /// full-size render box-downscaled to that size (same strength within 15 %), for grain that
    /// stays explicit and for grain that turns into per-pixel noise at the small size.
    #[test]
    fn small_pipeline_render_matches_downscaled_full_render() {
        use crate::{RenderRequest, SourceInfo, render};
        use lightcraft_raster::Rgb32f;
        let src = Rgb32f::filled(1600, 1200, [0.18, 0.18, 0.18]);
        let info = SourceInfo::default();
        for size in [25.0, 90.0] {
            let s = lightcraft_develop::DevelopSettings { grain: grain(60.0, size, 50.0), ..Default::default() };
            let full = render(&src, &info, &s, &RenderRequest::fit(1600, 1200)).image;
            let small = render(&src, &info, &s, &RenderRequest::fit(400, 300)).image;
            assert_eq!((small.width, small.height), (400, 300));
            let lum = |p: &[u8; 4]| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0;
            let down: Vec<f32> = (0..400 * 300)
                .map(|i| {
                    let (x, y) = (i % 400, i / 400);
                    let mut acc = 0.0;
                    for dy in 0..4 {
                        for dx in 0..4 {
                            acc += lum(&full.data[(y * 4 + dy) * 1600 + x * 4 + dx]);
                        }
                    }
                    acc / 16.0
                })
                .collect();
            let sm: Vec<f32> = small.data.iter().map(lum).collect();
            let ((m0, v0), (m1, v1)) = (stats(&down), stats(&sm));
            let ratio = (v1 / v0).sqrt();
            assert!((m0 - m1).abs() < 0.5, "size {size}: mean {m0} vs {m1}");
            assert!((0.85..1.15).contains(&ratio), "size {size}: grain sd {} (small) vs {} (downscaled)", v1.sqrt(), v0.sqrt());
            assert!(v0.sqrt() > 0.3, "size {size}: visible grain ({})", v0.sqrt());
        }
    }

    /// Hostile settings (NaN, infinities, far out of range) and positions never panic and give
    /// finite output.
    #[test]
    fn extreme_values_never_panic() {
        let vals = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, 0.0, 100.0];
        for &a in &[f64::NAN, 1e300, 100.0, 1.0] {
            for &v in &vals {
                for ppl in [f64::NAN, 0.0, -5.0, 1.0, 1e12, 6000.0] {
                    let g = Grain { amount: a, size: v, roughness: v, seed: u32::MAX, color: v, response: v };
                    let Some(gp) = GrainParams::new(&g, ppl) else { continue };
                    for (x, y, l) in [(0.0, 0.0, 0.5), (f32::NAN, 1.0, f32::NAN), (f32::INFINITY, -1e30, 2.0), (1e9, -1e9, -1.0)] {
                        let o = gp.apply(x, y, l);
                        assert!(o.iter().all(|c| c.is_finite()), "{g:?} {ppl} {x} {y} {l} → {o:?}");
                    }
                }
            }
        }
        assert!(GrainParams::new(&grain(f64::NAN, 25.0, 50.0), 1000.0).is_none());
        assert!(GrainParams::new(&grain(-5.0, 25.0, 50.0), 1000.0).is_none());
    }
}
