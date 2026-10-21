# Retouch

*Beyond Lightroom.* The Edit panel's **Retouch** section (below Detail) gathers the portrait retouch in
one place, a part of the person at a time. It works on the parts the open MediaPipe models find (the
same ones the People masks use: Selfie Multiclass for skin and hair, Face Landmarker for the faces and
their features), on every photo by itself.

## Using it

Open **Edit ▸ Retouch** (or **Window ▸ Edit Sections ▸ Retouch**). Each part folds open on a click:

| Part | What it does |
| --- | --- |
| **Skin** | Smooth Skin (Amount, Texture, Even Tone, Redness, face or body skin) and **Reduce Shine**; see [smooth-skin.md](smooth-skin.md). |
| **Blemishes** | **Find Blemishes** heals the small dark or red spots on the faces' skin (and with **Include body skin**, on shoulders, arms and back). It is also in the Remove panel. |
| **Teeth** | **Whiten** takes the yellow out (red over green and green over blue moved towards neutral) with a touch more brightness; **Brighten** lifts them up to about half a stop. Only the teeth showing between the lips (the bright, unsaturated pixels inside the mouth opening) change. |

- Every part has plain sliders that start at 0 (nothing changes) and are gentle at their ends: the aim
  is a natural retouch.
- Every part works on **everyone** or on **one person** (1 to 4, counted from the left by their face),
  each part on its own: whiten one person's teeth, soften everyone's skin.
- A **dot** after a part's name says it changes the photo; its **eye** switches it off and on for a
  before/after. The Retouch section's own eye (in its header) switches off all of it.
- The first time a part needs them, LightCraft offers the two models it uses (MediaPipe people, 16 MB,
  and face, 3.8 MB, Apache-2.0).

## How it works

Each part is a segmentation the People masks would make (`Teeth`, …), of everyone or of the chosen
person, found with the photo's look before any retouch (the models see the people as they are). They are
kept in the photo's settings (`retouch_analysis`, with the orientation they were found for), so rendering
and exporting never need the models: a part is found once, when a part first asks for it (a slider moves,
settings are pasted, the person changes or the photo is turned), in the background with an activity row.

The retouch runs on the scene-linear image right after Smooth Skin, before exposure, tone and colour, so
those ride on top of it. Its changes are made in stops of luminance and in log colour ratios (`R/G`,
`B/G`), blended in by the part's soft coverage: nothing outside a part changes, and a preview and a
full-size export change it alike. Both renderers run the same code.

## Commands

- `retouch.set {section, person?, …sliders}`: `teeth {whiten, brighten}`; one undo step.
- `retouch.off {section?}`: a section (or all of them but Skin) back to 0, keeping its person.
- `retouch.find`: look for the parts again; `retouch.status`: where each section stands.
- Every slider and person is a develop control too (`retouch.teethWhiten`, `retouch.teethBrighten`,
  `retouch.teethPerson`, …) for `develop.set`, the console, presets and the CLI's `render --set`.

## Presets, copy and paste

The sliders and the person of every part are develop settings, in the **Retouch** settings group:
they are copied, pasted, synced and saved in presets when that group is chosen (it is not chosen by
default, like Masking). What the models found on a photo is the photo's own: never copied, found again
for each photo the settings land on.
