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

- Every part has plain sliders that start at 0 (nothing changes) and are gentle at their ends: the aim
  is a natural retouch.
- Every part works on **everyone** or on **one person** (1 to 4, counted from the left by their face),
  each part on its own: whiten one person's teeth, soften everyone's skin.
- A **dot** after a part's name says it changes the photo; its **eye** switches it off and on for a
  before/after. The Retouch section's own eye (in its header) switches off all of it.
- The first time a part needs them, LightCraft offers the two models it uses (MediaPipe people, 16 MB,
  and face, 3.8 MB, Apache-2.0).

## Presets, copy and paste

The sliders and the person of every part are develop settings, in the **Retouch** settings group:
they are copied, pasted, synced and saved in presets when that group is chosen (it is not chosen by
default, like Masking). What the models found on a photo is the photo's own: never copied, found again
for each photo the settings land on.
