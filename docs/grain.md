# Film grain

The Effects panel's Grain (`ctl:grain.*`, `crates/pipeline/src/grain.rs`, GPU port in
`crates/gpu/src/wgsl/finish.wgsl`) renders a stochastic model of film grain rather than smoothed noise.

## Model

Implemented from the published Boolean model of Newson, Delon & Galerne, *Realistic Film Grain
Rendering* (IPOL 2017). No code was taken from any implementation.

- **Grains** are beads whose centres form a Poisson process on a fixed grid of cells (2.5 mean
  radii), so no slider and no render size reshuffles them. Their radii follow `exp(disp·z)` (z ≈
  unit normal, bounded to ±3). Their outlines are lumpy: the radius varies with angle by
  `L·m·(0.6 cos(2θ − φ₂) + 0.4 cos(3θ − φ₃))` with random phases and strength `m`. Each grain has an
  optical density of 1/4, 1/2, 3/4 or 1, so overlapping grains give continuous tones instead of two
  flat levels. Roughness sets the size spread `disp` (0–0.35), the lumpiness `L` (0–0.3) and the
  film softness (0.7 → 0.3 radii), so it reshapes the same grains: round, even and soft at 0;
  mixed, irregular and crisp at 100.
- **Density follows tone.** A pixel of display-encoded luminance `l` has expected grain coverage
  `p = 0.2 + 0.6·(1 − l)^γ`, where darker means denser. The grain density is set so that
  `λ·E[πr²]·E[o] = −ln(1 − p)`. Grains always overlap into irregular clumps and pores: a sparse field
  would read as isolated dots.
- **Each pixel shows the coverage averaged over its footprint.** Every grain is softened by an edge
  width `s` that combines film softness with one output pixel. The union is evaluated exactly as
  `1 − Π(1 − oᵢ·aᵢ)`, so the mean is `p` at any scale. The variance has a closed form (Poisson
  generating functional): `Var = (1 − p)²·(exp(λ·E[∫(o·a)²]) − 1)`.
- **Grains fade in with tone.** Within a cell, grains arrive in mark order and fade in over the last
  grain's worth of the mark range. The field therefore changes continuously as the tone changes:
  no grain pops in or out between neighbouring pixels or slider steps.
- **Output:** each channel gets `amp · w(l) · (p − coverage)` in display-encoded space, before the
  profile LUT. `w(l)` is 1 in the midtones and 0.35 at black and white, and the Response slider
  moves its peak.

## Resolution independence

Grain geometry is defined in normalized frame coordinates (fractions of the uncropped frame's long
edge, like grain on a negative). Only the softening (one output pixel) depends on the render size, so
a small render shows what the export shows downscaled:

- **Explicit grains** are used while the softening is up to 1 mean radius.
- **Per-pixel Gaussian noise** with the exact variance above is used from 1.8 radii, i.e. grains
  much smaller than a pixel (grid thumbnails, fit previews of fine grain).
- **Between the two**, the explicit and Gaussian parts cross-fade with variance-preserving weights.
- **Calibration:** the pixel softness (0.389 px) gives a sub-pixel grain the noise-equivalent area of
  a one-pixel box filter.
- **Tests:** `small_render_matches_a_downscaled_full_size_render` (grain level: mean, standard
  deviation, lag-1 correlation) and `small_pipeline_render_matches_downscaled_full_render` (whole
  pipeline: strength within 15 %).
- **On the demo scenes**, a 1500 px render's grain is within 3 % of the 6000 px export's grain
  box-downscaled to 1500 px. The old value noise was off by 11–12 %.

Amount is calibrated at a 6000 px long edge (a 24 MP export). Amount 100 gives a standard deviation
of about 0.06 (15 levels of 255) in mid-grey there, a little more for coarse grain.

## Controls

| Control | Range | Meaning |
|---|---|---|
| `grain.amount` | 0–100 | strength (XMP `crs:GrainAmount`) |
| `grain.size` | 0–100 | mean grain radius ≈ 4.3 µm (0), 5 µm (25), 8.9 µm (100) on a 35 mm frame: 0.7, 0.8 and 1.5 px on a 6000 px export (XMP `crs:GrainSize`) |
| `grain.roughness` | 0–100 | grain shape: 0 = round, even and soft beads; 100 = mixed sizes, lumpy outlines, crisp (XMP `crs:GrainFrequency`) |
| `grain.color` | 0–100 | colour grain: two more grain layers in opponent colour directions (neutral on average) |
| `grain.response` | −100–100 | where grain is strongest: shadows (−) … midtones (0) … highlights (+) |
| `grain.seed` | integer | the grain pattern (stable per photo) |

Settings saved before Colour and Response existed load with both at 0, which renders the default
model. LightCraft keeps both in its own settings; Camera Raw has no equivalents.

## Performance

- **GPU:** the per-pixel kernel evaluates the cells within a grain's reach of each pixel. On the
  24 MP test export it adds ≈ 20 ms to the finish pass (≈ 40 ms without grain).
- **CPU:** works on bands of 8 output rows. Each grain is generated once per band and splatted onto
  the pixels its bead reaches (`band_matches_per_pixel` checks the result equals the per-pixel
  evaluation). That is about 290 ns per pixel single-threaded at the default size (220 ns at
  Roughness 0; the lumpy outlines cost the rest).
- **CPU timings on 32 threads:** a 24 MP export's finish stage goes from ≈ 110 ms to ≈ 480 ms at
  Size 25, and ≈ 1.1 s with Color (three layers). A 1500 px render costs ≈ 15 ms more. The old value
  noise cost ≈ 35 ms at 24 MP. CPU-only exports with grain are therefore over the 1 s budget.

The CPU cost is above the old grain's: it is the price of evaluating real grains. Interactive
renders use the GPU path when an adapter exists.
