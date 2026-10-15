# People, roles and remembered objects

Masks and presets can be about a particular person ("the Bride's dress", "Jack's hair") or a
particular thing ("the wedding dress", hanging or worn). This page explains how LightCraft knows who
and what, where that knowledge is kept, and how to delete it.

## Names and roles

- **A name** is a person across your whole catalog: Alex, Jack. Names are the same names the
  People view shows: the name on a face region of a photo, whether it was read from XMP, named in
  the People view or named in the Masking panel.
- **A role** belongs to one shoot: the Bride, the Groom, the Best Man, the Mother of the Bride… The
  Bride in one wedding folder is Alex; in another she is Chloe. A role applies within a **scope**:
  - a folder (the default: the photo's folder and its sub-folders);
  - an album;
  - a date range;
  - an explicit list of photos.

  When scopes overlap, the most specific one wins: photos, then album, then the deepest folder,
  then dates.

A preset that says "the Groom's clothes" finds, on each photo, whoever is the Groom in that photo's
shoot. A mask that says "Alex's hair" finds Alex.

### In the app

- **Masking panel ▸ People.** The picker shows the faces found on the photo as thumbnails, each
  with its name and role, or a suggestion ("Alex?") for an unnamed face that looks like someone
  you named.
  - Click a face to choose that person.
  - Type a **Name** and optionally a **Role** (chips offer Bride, Groom and the other usual roles,
    plus names you used before), then **Save Name**.
  - **Find on other photos by** chooses whether the mask follows the person's name or their role.
    Pick the role for masks you will save in a preset.
- **Roles…** (in the People picker and the People view) opens **People and Roles**. It shows who
  has which role in this photo's folder. You can set a role, remove one, and **Forget All Face
  Data**.
- **People view.** Right-click a person for **Rename or Merge…** (typing an existing name merges
  the two) and **Forget This Person**.

### Commands (CLI, MCP, control channel)

| Command | What it does |
|---|---|
| `people.faces {id?}` | The faces on a photo: box, the person named there, their roles here, a suggestion. In the app it returns `pending` until the faces are found. |
| `people.name {id?, face, name, role?}` or `{faces: [{photo, face}], name}` | Names faces (Face regions, one undo step) and remembers how they look. `role` also binds the role in the photo's folder. |
| `people.setRole {role, person, scope?}` | Binds a role. `scope` is `"folder"` (the default), `{folder}`, `{album}`, `{from, to}` or `{ids}`. |
| `people.roles {id?}` | The roles that apply to a photo, and all roles. |
| `people.clearRole {role, scope?}` | Removes a role (in a scope, or everywhere). |
| `people.list` | Everyone named or remembered, with their photo counts and roles. |
| `people.recognize {ids?, apply?}` | Suggests names for the unnamed faces on the photos. `apply` names the confident ones (one undo step). |
| `people.cluster {ids?}` | Groups the unnamed faces of a shoot by look, so a whole group can be named at once. |
| `people.rename {from, to}` | Renames (or merges) a person on every photo. |
| `people.forget {person, names?}` and `people.forgetAll {names?}` | Delete face data (see below). |

## How people are recognised

### The licence finding

No open face-recognition model is clean enough to ship.

- **Face detection and landmarks** use Google's MediaPipe BlazeFace and FaceMesh. Their weights are
  Apache-2.0, trained on Google's own consented data (Tier A).
- **Face identity** models (an embedding per face) all fail that test:
  - **SFace** (upstream's default) has weights labelled Apache-2.0 by OpenCV Zoo. Its training data
    is undocumented, and its repository names CASIA-WebFace, VGGFace2 and MS1MV2, all
    non-commercial or withdrawn datasets (**Tier B**).
  - **AuraFace** has an Apache-2.0 tag. It was trained on "a commercial dataset" whose sources,
    consent and licence are not stated (**Tier C at best**: a maintainer decision).
  - **YuNet**, upstream's face detector, was trained on WIDER FACE (CC BY-NC-ND), so it is **Tier
    B**, and this feature doesn't use it.

So LightCraft recognises people by **appearance**, with the Tier A models only. Each face gets a
signature made of three things:

- the colours of the person's **clothes and hair**, from Selfie Multiclass inside the part of the
  photo that belongs to that face;
- **SAM 2.1's image features of their body** (cut, fabric, beard, hair style), when SAM 2.1 is
  installed;
- a coarse **colour layout** of the face.

A small model, fitted on pairs of faces from three weddings, weighs the three. The face's
proportions (FaceMesh landmarks) and skin colour were measured too, and hardly told people apart
there.

### What this means in practice

- **Within one shoot** this tells the bride (a white dress) from the groom (a dark suit) and from
  the guests well.
- **Across shoots** clothes change, so it doesn't recognise a person from one wedding to another.
  Suggestions only use the people named in the same folder. Name a face or two per shoot, or bind
  roles per folder.
- **Measured.** On 60 photos of three weddings, with three faces named per person, about 60 % of
  the suggestions were right and half of the faces got one. It was weakest when bride and groom
  wore the same colour (a black dress and a black suit).
- **Matching only suggests.** Nothing is named without you, unless you run
  `people.recognize {apply: true}`.
- **A face-recognition model** you installed yourself (Settings ▸ Faces, with its licence shown
  there) can add an identity embedding to the signature. This feature never downloads one.

## Remembered objects

Teach LightCraft a thing once or a few times and it finds it on other photos: a wedding dress worn
or hanging, the groom's suit, the bouquet, the cake.

1. **Select the thing** on a photo with any mask:
   - an **Object** click (SAM 2.1);
   - a brush;
   - People with the clothes of a person;
   - Describe.
2. **Masking panel ▸ Remembered ▸ Remember Selected Mask As…**, then a name ("Wedding dress"). Or
   use `concepts.teach {name, mask?}`.
3. **Teach it a few times, on different photos.** For example:
   - worn, and hanging on a door;
   - in daylight and in tungsten light;
   - a white dress and a black one, if both should count.

   Each photo adds an example (up to 12).
4. **Use it.** Click it under **Remembered** to make a mask, or put it in a recipe
   (`{"kind": "concept", "name": "Wedding dress"}`), so presets carry it.

How it works:

- SAM 2.1 (Apache-2.0) encodes the photo, and the selection's features become prototypes, along
  with features of what isn't the thing.
- On another photo, the cells that look most like the thing become clicks.
- SAM 2.1 draws the mask from those clicks, and it is kept only where the likeness map agrees.

SAM's features describe **appearance** (white satin with folds, a dark tailored jacket) better than
**identity** (this exact dress). Teach more examples when something similar is picked up by mistake.

Commands: `concepts.list`, `concepts.teach {name, id?, mask?}`, `concepts.rename {from, to}`,
`concepts.forget {name}`, and `mask.add {"kind": "concept", "concept": "Wedding dress"}`.

## Where the data is kept, and deleting it (privacy, GDPR)

All of it stays in your library folder, on your computer:

- `people.json` holds roles and appearance signatures (up to 64 faces per person);
- `concepts.json` holds remembered objects;
- face names stay on the photos' Face regions.

Nothing is uploaded, nothing is written into sidecars, XMP or exported files, and nothing is
needed by renders.

Face signatures are **biometric data** of the people you photograph. Under the GDPR (article 9) and
similar laws, processing them to identify people needs a lawful basis, normally your clients'
**explicit consent**. Keep them only as long as you need them.

| Action | What is deleted |
|---|---|
| **Forget This Person** (People view, right-click a person) or `people.forget {person}` | That person's appearance signatures and roles. With **Forget and Remove Their Name** (`names: true`), also their name on every photo (one undo step). |
| **Forget All Face Data…** (People and Roles window, click twice) or `people.forgetAll` | `people.json`, upstream's face-recognition caches (`face-embeddings-*.bin`, `face-scanned.bin`), and the faces found this session. `names: true` also removes every person's name from the photos. |

Remembered objects (`concepts.json`) hold no face data. Delete them with `concepts.forget`.
