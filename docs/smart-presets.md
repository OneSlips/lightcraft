# Smart presets: AI masks in presets

A LightCraft preset can carry masks. An AI mask in a preset is stored as a **recipe**: what to
select and how, never the pixels a model found on the photo the preset was made from. Applying the
preset computes the selection again on each photo. The mask can select:

- the sky, the subject or the background;
- the Bride's clothes or Sam's hair;
- the wedding dress you taught LightCraft to recognise;
- an area bounded by a gradient that starts at the couple's feet.

Lightroom Classic treats AI masks in presets the same way, and its presets with AI masks import as
recipes too.

## How applying works (as in Lightroom Classic)

There is one kind of preset. Applying any preset:

- **sets the global settings the preset contains**, and only those: a preset with only masks
  changes nothing else;
- **adds the masks it carries**, with their adjustments, to the photo's own masks;
- **never removes or replaces** a mask the photo already has.

Presets therefore stack. Apply your main look, then *Recover Blue Suit*, then *Bring Sky Down*, and
each adds its masks on top of what is there.

What a preset is depends on what it holds:

- **Mask-only.** One or more masks with their local adjustments, and nothing else. *Bring Sky
  Down* is an example.
- **Combined.** Global settings (your studio look) plus several AI masks. *Portrait + Wedding
  Dress* is an example.

Presets carrying AI masks show a small **AI** mark in the Presets panel.

Groups work as in Lightroom: a preset belongs to one group. Imported Lightroom presets keep their
`crs:Group`, and a preset without one takes the name of its folder.

### Applying the same preset again

The masks added by the most recent preset application stay **provisional**: they are tagged with
the preset that added them. Two cases follow.

- **The same preset, clicked again right away.** Its masks are replaced, recomputed, and get the
  preset's adjustments again. They don't double.
- **Another preset applied in between, or you edited one of those masks.** The masks are yours:
  they no longer belong to any preset. Clicking the first preset again adds a fresh set and leaves
  yours alone.

LightCraft can't check Lightroom Classic's own behaviour (it is never driven), and Adobe's
documentation doesn't describe it. This rule is LightCraft's choice. It avoids silent doubling on a
double click, and it never takes away a mask you may have changed.

To get a second copy on purpose, use **Duplicate Mask**. The tag is stored with the photo's edit,
so it survives a restart.

### Undo

Every application is **one undo step**, AI masks included. Masks computed in the background
complete the step that added them, with no extra steps of their own.

- **Undo while the masks are still computing.** The computation is cancelled.
- **Redo.** The masks come back with what was already computed, and anything still missing is
  computed again.

## AI masks are computed per photo, in the background

These all bring masks that may need computing: applying a preset (to one photo or to a selection),
**Paste Settings**, **Sync Settings**, Auto Sync, and **Import with a develop preset**. In the app
the masks are then computed on the AI worker in the background:

- one **Updating AI masks** row in the activity stack, with a ✕ to cancel;
- the photo shows the approximate mask meanwhile;
- import never waits: the photos appear first and their masks follow.

From the CLI, MCP or the control channel, the command waits and returns
`{queued, updated, approximate, notes}`. `masks.recompute {ids?, force?}` recomputes what is
missing (`force`: everything) on any photos.

If a model isn't installed, the mask stays approximate and one note names the model to get. If a
person, role or remembered object isn't on the photo, that part selects nothing and a note says so,
for example "Jack wasn't recognised on this photo".

### Hover preview

Rest the pointer on a preset to preview it on the photo without applying it, as in Lightroom.
Moving off restores the view exactly.

- **Global settings** preview at once.
- **AI masks** preview straight away when the photo already has the same selection, either from an
  earlier preview or from one of its own masks with the same recipe.
- **Otherwise**, after about 150 ms of resting, the masks are computed in the background and join
  the preview as they become ready. Moving off cancels what is still queued.
- **Clicking** reuses whatever the preview already computed.

## What a recipe can say

| Part | Example | Stored as |
|---|---|---|
| Subject, Sky, Background | Bring Sky Down | `{"kind": "sky"}` |
| Landscape class | water, vegetation… | `{"kind": "landscape", "class": "water"}` |
| People, everyone or by position | Lightroom's "Person 1 - Face Skin" | `{"kind": "people", "person": 0, "parts": ["Face Skin"]}` |
| People **by role** | the Groom's suit | `{"kind": "people", "parts": ["Clothes"], "who": {"role": "Groom"}}` |
| People **by name** | Sam's hair | `"who": {"name": "Sam"}` |
| A **remembered object** | the wedding dress | `{"kind": "concept", "name": "Wedding dress"}` |
| Gradients | linear, radial | normalised coordinates |
| A gradient **relative to a selection** | from the bottom of the couple | `"anchor": {"to": "people", "shape": {"kind": "linear", …}}` |
| Ranges | luminance, colour, depth | as in any mask |

Components combine with **add, subtract and intersect**. Each component and each mask can be
inverted.

- **Anchored gradients.** The anchored gradient's points are in the box of the selection it
  follows: 0,0 is its top left and 1,1 its bottom right. `"to"` is `"subject"`, `"people"` or
  `{"who": {…}}`. The box is found on each photo when its masks are computed. Until then the
  gradient spans the whole photo.
- **Roles.** A role resolves to whoever has it in that photo's shoot (see
  [People and roles](people-and-roles.md)). With nobody set for the role, the mask covers everyone
  and a note says so.

From commands:

```
mask.add {"kind": "people", "who": {"role": "Groom"}, "parts": ["Clothes"]}
mask.add {"kind": "concept", "concept": "Wedding dress"}
mask.add {"kind": "linear", "start": {"x": 0.5, "y": 1}, "end": {"x": 0.5, "y": 0.55}, "anchor": "people"}
preset.create {"name": "Recover Blue Suit", "groups": ["masks"]}
preset.apply {"id": "…", "ids": [1, 2, 3]}
```

`preset.create` and `preset.update` store recipes: the selections of the photo they were made from
are dropped.

## Built-in smart presets

These ship in the **Smart Masks** group. Their values are LightCraft's own, written in code (they
are not asset files).

| Preset | Mask | Adjustments |
|---|---|---|
| **Sun Kissed** | Background, minus a linear gradient anchored to the people (from their feet up to half their height) | warmer (+18), a little brighter, colour 40° at 25, softer highlights, less dehaze: golden-hour backlight behind the couple |
| **Recover Blue Suit** | the Groom's clothes ∩ a colour range of suit blues (navy, royal, steel, grey-blue) | Saturation +30, a little brighter, open shadows: brings back the blue most wedding looks mute |
| **Bring Sky Down** | Sky | Exposure −0.6, Highlights −35, Dehaze +12 |
| **Brighten Dress** | the Bride's clothes ∩ the bright luminances | Exposure +0.3, Whites +10, Shadows +15, slightly cooler and cleaner |
| **Darken Background** | Background | Exposure −0.5, Clarity −10, Saturation −10 |

Recover Blue Suit and Brighten Dress need a Groom and a Bride in the shoot. Set them with the People
picker's **Roles…** or with `people.setRole`. Without one, they apply to everyone's clothes.

## Lightroom presets with AI masks

**File ▸ Import Presets…** (`preset.import`) reads Lightroom Classic `.xmp` presets. Lightroom's AI
masks map to recipes:

| Lightroom (`MaskSubType` / `MaskSubCategoryID`) | LightCraft |
|---|---|
| 1 | Subject |
| 2 | Sky |
| 0 / 22 | Background |
| 3 / 2 Face Skin, 3 Iris and Pupil, 4 Body Skin, 5 Hair, 6 Lips, 8 Eye Sclera, 11 Clothes, 12 Teeth | People with that part. Other parts are recognised by the mask's name ("Person 1 - Eyebrows"). |
| Landscape | Landscape, by the mask's name (Water, Vegetation, Mountains, Architecture, Natural / Artificial Ground) |
| 4 Objects | not carried over: Lightroom stores a selection brushed on the photo the preset was made on |

- **People.** Lightroom names the person after the photo the preset was saved on ("Person 1"), and
  detects people again on each photo. LightCraft maps it to **everyone**. To make the mask follow
  the Bride or the Groom, re-create it with the People picker and update the preset.
- **Combine modes.** Lightroom's "intersect with" (stored as subtracting the inverted component)
  keeps its meaning.
- **Reported, not dropped.** The import report lists anything that isn't carried over:
  - local Point Color;
  - local Curves;
  - local Grain;
  - Objects masks.

**Export** (`preset.exportXmp {ids | id, folder}`) writes Lightroom-style `.xmp` presets:

- Subject, Sky, Background and People parts become Lightroom AI masks.
- Gradients become linear and radial gradients.
- Luminance ranges become luminance range masks.
- Mask adjustments become `Local*` sliders.
- What Lightroom has no equivalent for is listed in the result:
  - roles and names (Lightroom selects every person);
  - remembered objects, Landscape, colour ranges and brushes;
  - anchored gradients (exported fixed on the photo);
  - the global settings.
- The complete LightCraft preset is also written in LightCraft's own namespace
  (`lc:PresetSettings`). Lightroom ignores it, and importing the file back into LightCraft restores
  the preset exactly.

## Remembered objects

See [People and roles](people-and-roles.md#remembered-objects).
