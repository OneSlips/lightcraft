# Film grain

The Effects panel's Grain (`ctl:grain.*`, `crates/pipeline/src/grain.rs`, GPU port in
`crates/gpu/src/wgsl/finish.wgsl`) renders a stochastic model of film grain rather than smoothed noise.

## Model

Implemented from the published Boolean model of Newson, Delon & Galerne, *Realistic Film Grain
Rendering* (IPOL 2017). No code was taken from any implementation.

- **Grains** are discs whose centres form a Poisson process. Their radii follow `exp(disp·z)` (z ≈
  unit normal, bounded to ±3), and each grain has an optical density of 1/4, 1/2, 3/4 or 1, so
  overlapping grains give continuous tones instead of two flat levels.
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
| `grain.size` | 0–100 | mean grain radius ≈ 4 µm (0), 10 µm (25), 48 µm (100) on a 35 mm frame (XMP `crs:GrainSize`) |
| `grain.roughness` | 0–100 | spread of grain sizes and crispness: 0 = even and soft, 100 = mixed and crisp (XMP `crs:GrainFrequency`) |
| `grain.color` | 0–100 | colour grain: two more grain layers in opponent colour directions (neutral on average) |
| `grain.response` | −100–100 | where grain is strongest: shadows (−) … midtones (0) … highlights (+) |
| `grain.seed` | integer | the grain pattern (stable per photo) |

Settings saved before Colour and Response existed load with both at 0, which renders the default
model. LightCraft keeps both in its own settings; Camera Raw has no equivalents.

## Performance

- **GPU:** the per-pixel kernel evaluates the at most 3 × 3 cells around each pixel. On the 24 MP
  test export, the finish pass with readback takes about 42 ms with grain vs 36 ms without.
- **CPU:** works on bands of 8 output rows. Each grain is generated once per band and splatted onto
  the pixels its disc reaches, so it costs about 95 ns per pixel single-threaded at the default size
  (`band_matches_per_pixel` checks the result equals the per-pixel evaluation).
- **CPU timings on 32 threads:** a 24 MP export's finish stage goes from ≈ 110 ms to ≈ 390 ms at
  Size 25. With Color it is ≈ 600 ms (three layers). A 1500 px render costs ≈ 50 ms more. The old
  value noise cost ≈ 35 ms at 24 MP.

The CPU cost is above the old grain's: it is the price of evaluating real grains. Interactive
renders use the GPU path when an adapter exists.
