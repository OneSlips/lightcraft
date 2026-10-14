# Smooth Skin

*Beyond Lightroom.* A portrait retouch in the Edit panel: it evens out skin tone and texture while keeping
the pores, on the skin the open MediaPipe models find (never the eyes, brows, lips, teeth, facial hair, hair
or background).

## Using it

**Edit ▸ Smooth Skin ▸ Smooth Skin** switches it on with subtle settings (Amount 35, Texture 0, Even Tone 25,
Redness 20). The first time, LightCraft offers the two models it needs (MediaPipe people, 16 MB, and face,
3.8 MB, Apache-2.0; the same ones the People masks use). The skin is found in the background in a second or
two; the panel says when it is looking, and when there is no skin (or no such person) in the photo.

| Control | What it does |
| --- | --- |
| **Amount** (0–100) | Softens the middle band: uneven texture, small bumps, fine lines. Small swings go; strong edges (the side of the nose, a jawline, a smile line) stay. |
| **Texture** (−100–100) | The fine band: pores and fine texture. 0 keeps them exactly; below softens them (it can look plastic), above brings them out. |
| **Even Tone** (0–100) | Evens out blotches of tone and colour. The light and shape of the face (its shading, a warm and a cool side) are kept. |
| **Redness** (0–100) | Takes patches that are redder than the skin around them (a pimple's halo, a flushed patch) back towards it. |
| **Skin**: Face / Body / Both | Which skin: the face's, the body's (shoulders, arms, back, hands), or both. |
| **Person**: All / 1–4 | Everyone, or one person, counted from the left by their face (as the People mask counts). |

The section's eye switches it off for a before/after; **Turn Off** sets it back to zero.

## How it works

It runs in the develop pipeline after white balance, spot removal and noise reduction, and before exposure,
tone and colour (`crates/pipeline/src/skin.rs`), so exposure, contrast and colour grading ride on top of it,
and heal spots (Find Blemishes) are smoothed with the skin around them.

The image is split, in log luminance and two log colour ratios (red/green, blue/green), into four bands
measured in **each face's eye distance** *d*, so a preview and a full-size export look alike:

- **fine**, below 0.01 *d*: pores and fine texture, kept;
- **middle**, 0.01–0.05 *d*: softened by Amount. The softening fades out as the band's own contrast grows, so
  real edges stay;
- **low**, 0.05–0.22 *d*: evened by Even Tone (luminance and colour) and Redness (colour, towards the skin
  around it), with the same protection for strong contrast;
- **base**, above 0.22 *d*: the face's shape and light, untouched.

Each band is a normalized convolution over the skin alone (`blur(x·m) / blur(m)`), so hair, eyes and lips never
bleed into the skin's bands, and the result is blended in by the skin's coverage: nothing outside it changes.
Faces of very different sizes (someone close, someone behind) are worked separately, each pixel by the face it
belongs to. The wide bands are computed on reduced copies; on a 2.5 MP preview the stage takes a few tens of
milliseconds on a desktop CPU, and only Smooth Skin is redone while one of its sliders is dragged. The GPU
renderer runs the same code after its noise reduction, so both match.

The skin and faces found are kept in the photo's settings (`skin_analysis`, with the area, person and
orientation they were found for), so rendering and exporting never need the models. They are the photo's own
data, not an edit: no undo step of their own, never copied by Copy / Paste / Sync / presets (the sliders are,
in the **Smooth Skin** group, which is not copied by default), and found again when you change the skin or
person, rotate the photo, or paste Smooth Skin onto another photo. They are written to the XMP sidecar in
LightCraft's namespace (`lc:settings`) with everything else; Lightroom ignores them.

## Commands

`skin.smooth {off?}`, `skin.set {amount?, texture?, evenTone?, redness?, area?: face|body|both, person?: 0–4}`,
`skin.find` (look for the skin again), `skin.status`; the sliders are also develop controls (`skin.amount`,
`skin.texture`, `skin.evenTone`, `skin.redness`) for `develop.set`, presets and the CLI's `render --set`. In the
CLI and MCP the skin is found before the command returns.

## Limits

- The skin comes from Selfie Multiclass, which sees the photo at 256 × 256: on a small face in a big group photo
  its edge is soft, and the skin near hair and the background is smoothed less.
- A face FaceMesh doesn't place landmarks on (a strong profile) is measured by its box, and face skin there may
  include the eye and mouth.
- Facial hair is left out where it is clearly darker and more textured than the cheeks; light stubble may be
  smoothed a little (its hairs are fine-band texture, which is kept).
