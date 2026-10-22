# What's new in LightCraft

## 2026-10-22

### Export
- **A name typed in Save preset is saved when you press Export**, with a note saying so; the small Save button
  is no longer needed.

### Masking
- **Remembered objects ("Wedding dress") work in every library**, like presets: they are kept with LightCraft's
  settings instead of one library. Objects a library remembered before are added once, the first time it is open.

### Develop
- **A mask's Whites and Blacks adapt to what is under the mask**, as in Lightroom: Whites works up to the
  brightest tones of that mask and Blacks down to its darkest, so Whites on a white dress pushes the dress's own
  highlights and Blacks on a dark suit its own shadows. The tones are measured once on the whole photo, so the
  preview, a zoomed view and the export look the same.

### Library
- **Delete removes photos from the library, and the files show up in Import again.** As Lightroom's Remove from
  Catalog: the confirmation asks "Remove the selected photo from the library?", the files and their XMP sidecars
  stay on disk, and browsing the folder in Import lists them as not imported, ready to import (their edits come
  back from the sidecar). There is no Recently Deleted step any more; photos already there stay until you restore
  or remove them. The note after a delete has an **Undo** button.
- **Selecting works as in Lightroom.** Shift-click selects a range and Ctrl/⌘-click adds or removes a photo without
  changing the photo in the loupe; a click on a photo inside the selection shows it and keeps the selection. Select
  All (Ctrl/⌘+A) keeps up with photos still arriving from an import.
- **Selected photos are easy to see**: in the grid and the filmstrip the active photo is the brightest, the other
  selected photos are lighter than the rest, and a pick's glow sits behind the thumbnail instead of over it.
- **The filmstrip says "Photo 116 of 123 · 6 selected"** instead of "116 / 123".
- **Auto Sync stays as you left it**, in every library and after a restart, and has a switch at the right end of
  the bottom bar.
- **Space can zoom to the eyes** even with the eye strip off: View ▸ Space Zooms To chooses the camera's focus
  point (else the eyes) or the eyes first.
- **Assisted Culling can reject soft eyes too**, and asks whether only the main subjects' eyes count.
- **Import's Places leave out the folders Windows hides**, such as AppData.
- **Capture Time sort goes through the day across all your cameras.** A collection sorted by capture time used to
  show the photos in the order they were added (one card after the other), and Recently Added kept each import in
  file-name order whatever the sort; both now follow the sort you chose. The sort compares the full capture time,
  sub-seconds included; a camera that records its time zone and one that doesn't are compared by the clock time
  they show. The option is now called **Capture Time**; a collection's own order is **Custom Order**.
- **Your export presets and the rest of your way of working are the same in every library**: export presets
  and the last export, print templates, metadata, filter, curve, colour-label and keyword-set presets, slideshow
  settings, Hold View, the sort and the folded preset groups now live in LightCraft's settings folder, not in the
  library. The first time a library is opened, its own are copied in (nothing lost, nothing twice; a preset with
  the same name and other settings is kept as "name 2"). The library's copy stays for older builds.
- **Albums are now Collections**, as in Lightroom: the Library sidebar's **Collections**, **New Collection…**,
  **New Smart Collection…**, the target collection (**B** adds to it) and every menu and message that named albums.
  Nothing about them changed but the name; agents keep using the `album.*` commands.

### Presets
- **Create Camera Profile from Color Chart remembers each photo's light and the look** you chose last time.
- **New Preset, Copy Settings, Paste Selected and Sync Settings pick single settings**, as in Lightroom: open a group
  with its arrow to tick Exposure but not Contrast, Sharpening but not Noise Reduction, the camera profile but not the
  Calibration sliders. A group with only some settings ticked shows a dash.
- **White Balance is no longer saved in a new preset unless you tick it**, so a look applied to a whole wedding
  keeps each photo's own white balance.
- **New Preset offers the groups you have** in a list, with New Group… for another; typing a name that exists
  already (in any case) puts the preset in that group instead of making a second one.
- **Update with Current Settings…** (right-click a preset) shows what the preset holds, ticked, so you can see and
  change what is saved. The preset keeps its name, group, place and marks, and the message after it has **Undo**.
- **Copy Settings, Sync Settings and New Preset remember their ticks** (and New Preset its group) in every library
  and after a restart. Edit ▸ Sync Settings… opens Lightroom's Synchronize Settings choice.
- **A preset clicked in the Presets panel follows Auto Sync, like a slider**: with Auto Sync off it goes on the
  active photo only; with it on, on every selected photo (one undo). Right-click ▸ **Apply to N Selected Photos**
  and Quick Develop's Saved Preset still put it on the whole selection.
- The presets marked to come first when importing are now the **At Import** group (was "For Import").

### Remove
- The Remove panel has Lightroom's tools: **Remove**, **Heal**, **Clone** and **Generative**. Content-Aware is part
  of Remove: a painted Remove stroke is rebuilt from what surrounds it, as Content-Aware did. Spots made with
  Content-Aware before show as Remove.

### Retouch
- **Retouch fetches its face models by itself**: the first time a Retouch slider needs them, the MediaPipe face
  models (Apache-2.0) download in the background (a row in the activity stack) and the photo is retouched once
  they are there. No dialog.
- An older install's face model gets its small-face detector in the background once a session, and the AI masks
  use it as soon as it arrives.

### AI Denoise
- **LightCraft's own AI Denoise model** (MIT OR Apache-2.0, trained on public-domain raws) is offered in
  Settings ▸ AI Denoise, downloaded from radfilms.com.au. `LIGHTCRAFT_DENOISE_MODEL_URL` points it elsewhere.

## 2026-10-19

### Hidden Local locations come back one at a time
- With several locations removed from Local, "Show N hidden locations" now lists them, so each comes back on its own
  (Restore All is there too). The row used to bring every hidden location back at once (issue #2114).

## 2026-10-18

### Fill Location from GPS: no more neighbouring town in City
- The offline place names list towns of 15,000 people or more, so a photo taken in a smaller one got the nearest larger
  town in City, sometimes another municipality (issue #2242). The Fill Location tooltip and the command help now say so,
  and `metadata.reverseGeocode` takes `maxDistance` (metres): past it, City stays empty (State and Country still fill)
  and the reply names the town it found as `nearestCity`.

### A preset that couldn't be saved is no longer lost silently
- When presets.json can't be written (a full disk, or the browser's storage quota), the new or changed preset now
  counts as unsaved, like other changes: the top bar warns, saving is retried, and quitting asks first (issue #2266).
  Before, the failure was only logged, and the preset could be lost without any warning.

### Portrait crop ratios stay portrait
- A crop ratio no longer flips to the photo's orientation (issue #2253): after X turns the crop portrait, picking
  another ratio (4 × 5, 2 × 3…) keeps it portrait, as in Lightroom, and a Custom ratio taller than wide gives a
  portrait crop.

## 2026-10-17

### Develop
- **The Edit panel comes back after Masking, Crop, Remove or Red Eye**: close the tool and the
  sliders are there again, at the same width and scroll position, and the photo doesn't move. If
  the panel was hidden before, it stays hidden.
- **Color Mixer: HSL**, laid out as in Lightroom Classic: **Mixer | Point Color** tabs, then
  **Adjust: HSL / Color**. HSL shows **Hue, Saturation, Luminance** or **All** (the three stacked,
  each with its heading): eight colour sliders each, with coloured tracks, and a targeted
  adjustment per group. HSL is the default and your choice is remembered. Point Color is now the
  Color Mixer's second tab.
- **A mask's Whites and Blacks work on bright subjects**: Whites moves the brightest tones under
  the mask (the highlights of a white dress, not only what was already near clipping), Blacks the
  darkest. Both are stronger than before.
- **Mask sliders in groups**: thin lines between Temp / Tint, the tone sliders, Texture / Clarity /
  Dehaze, Hue / Saturation and the detail sliders.
- **Panning while masking**: zoomed in with the Masking panel open and no tool armed, drag the photo
  to move around. With a tool armed (a brush, a gradient, Remove, Crop, a picker), hold **Space**
  and drag. Space no longer zooms out while a tool is in use on a zoomed photo.
- **An eyedropper pointer** for the white balance selector, Point Color and the Color /
  Luminance Range pickers; its tip is the pixel picked.
- **Constrain Crop is on for new photos**: the crop stays inside the photo after a rotation or a
  lens correction. **Settings ▸ Import ▸ Constrain crop by default** turns that off. Photos
  already in the library, and Lightroom edits, keep theirs.

### A note over the photo for every change
- Whatever you change, with a key, the mouse, a menu, a slider or the console, a short note over the photo says
  what changed: **Exposure +0.33** (the running value while you drag), **Picked**, **3 stars on 12 photos**,
  **Profile X applied to 350 photos**, **Undo: Exposure**. With Auto Sync or several photos selected it says how many
  photos changed.
- **Settings ▸ Interface ▸ On-Screen Overlay**: show changes **Always**, for the **Control surface only**, or **Off**;
  its size, opacity, how long it stays, where it sits and its colour. The console's readout uses the same look.

### Settings ▸ Controls
- **The Dialpad is drawn as the device**: buttons 1 and 2 top left, the roller top right, the big dial in the middle,
  buttons 3 and 4 at the bottom corners. The Keypad's page buttons sit at its lower corners.
- **The Actions Ring** has its eight slots in LightCraft, with useful actions to start with. **Drag an action** from
  the list onto any slot, key, button or dial.
- **Touch to find**: while Settings ▸ Controls is open, touch a knob or press a button on the device and it is
  selected in the editor. The photo doesn't change meanwhile.
- **Loupedeck+**: drawn in zones as on the console (colour knobs with their mode, tone knobs, the Control Dial with
  its buttons, the rest of the buttons). With the new plugin it works without assigning anything in the Logi or
  Loupedeck software.

### Face Skin masks
- **The eyes, brows and lips are left out again** on real faces, smiling and squinting ones too: the
  whole eye (lids and lashes) is cut out, however far it is open.
- **Face Skin reaches the hairline**: a high forehead is no longer faded out towards the top.

### Dress and clothing masks
- **A remembered dress (or suit) no longer takes the skin with it**: the neckline, arms and hands of
  the person wearing it, and their hair, are left out; People ▸ Clothes too.
- **Crisp edges, no halo**: the outline follows the dress's own edge instead of a soft glow around
  it (the model's coarse mask is now refined on the photo).

### Import keeps each body's camera profile
- Photos imported from the Import tab now always get **their camera body's profile**, also when the
  body's profile was set after the folder was opened (in the Import panel, say), and with a preset
  stack. Before, such a photo could arrive with plain "Color".
- A preset never takes the body's profile away. To let a preset's own camera profile win, tick
  **Presets may change the camera profile** under the Import panel's presets.

### Import notices say what really happened
- When an import is ready, the notice names only what was done: **AI masks with their counts**
  ("AI masks: 24 on 12 photos") only when a preset had some, Smooth Skin when it ran, previews when
  they were built. An import with nothing more to do shows no notice.

### Backup when quitting: asks first
- With a backup due (Settings ▸ Backups), quitting asks **Back Up and Quit** or **Skip This Time**,
  as Lightroom Classic does. When nothing changed since the last backup (you only looked at
  photos), it doesn't ask and doesn't back up.

### Presets Manager: a proper table
- **Deleting a group asks right under the group**, not at the top of the list where you couldn't see
  it. A preset's Delete asks in its own row.
- The list reads as a table: each group's header is a **shaded band** with its count in a muted
  column, the presets of an open group hang off an **indent guide** with faint alternate shading,
  the row under the pointer lights up, and **every action sits in a fixed column at the right**, the
  same for groups and presets (no more zig-zag after names of different lengths).
- The **Profile Manager** has the same table look; its confirmations stay next to what they are
  about.

### One preset picker everywhere
- Choosing a preset in the **Import panel**, the import review, **Quick Develop** (new: **Saved
  Preset** applies one to all the selected photos) and the default presets in Settings now opens
  the same picker:
  - a **search** field;
  - **Favorites** and **At Import** first: mark the presets you use on import with the import
    arrow at the right of a row (or in the Presets Manager's new column, or the Presets panel's
    right-click menu). When importing, the picker shows just those until you choose **All Presets**;
  - then **groups that fold**, in the Presets Manager's order: your own groups ("001-…", "002-…")
    before LightCraft's;
  - long names left-aligned, cut short with … (the full name on hover);
  - rest the pointer on a preset to **see it on a photo** of the folder, beside the list.
- Hidden presets and built-in presets you deleted no longer show in the Import panel, nor do looks
  you hid or deleted in the Profile Manager.
- The Presets panel has the same search and the Favorites and At Import groups at the top.

### Retouch panel
- **Edit ▸ Retouch** (where Smooth Skin was, below Detail) gathers the portrait retouch in one place,
  a part of the person at a time. Each part folds open, has plain sliders, works on everyone or on
  one person, and has its own **eye** for a before/after; the Retouch section's eye switches all of
  it off. A dot marks the parts that change the photo.
- **Skin**: Smooth Skin as before (Amount, Texture, Even Tone, Redness, face or body, a person),
  plus **Reduce Shine**, which tones down shiny spots on the forehead, nose and cheeks towards the
  skin around them.
- **Blemishes**: Find Blemishes (faces, and body skin if you like), also still in the Remove panel.
- **Teeth**: **Whiten** takes the yellow out, **Brighten** lifts them a little; only the teeth showing
  change. The teeth are found by themselves, in the background, the first time you move a slider.
- **Window ▸ Edit Sections ▸ Retouch** opens it.

### AI Denoise
- Settings ▸ AI Denoise no longer says a model is being trained or promises a download: it says
  plainly that this version has no AI Denoise model and none can be downloaded from here yet, and
  that Luminance and Color under Detail reduce noise meanwhile. A LightCraft denoise model you have
  (a folder with a .onnx file and denoise-model.json) installs with **Install from file…**.

## 2026-10-16

### Presets Manager
- **Settings ▸ Presets** (or **Manage Presets…** in the Presets panel's menu) lists every develop
  preset, yours (all 11 000 of a Lightroom collection, in their groups) and LightCraft's with the
  Smart Masks, plus your local adjustment presets and import presets.
- Per preset: **favourite, rename, move to a group, move up or down, hide** (out of the Presets
  panel, kept here) and **delete** (a built-in one is taken off this computer; **Restore Built-in
  Presets** brings it back).
- Per group: **fold, rename, move up or down, hide, delete** (or move its presets to another group
  first), and **New Group…**.
- **Search** by name or group, and **With AI masks only**. Groups start folded; a search opens what
  it finds. Scrolling stays smooth with 11 000+ presets.
- **Import** presets, folders, Lightroom preset folders and .zip packs (or drop them on the window),
  and **export** them for LightCraft or for Lightroom (.xmp). Import presets are now shared by all
  your libraries too.

### Profile Manager: whole groups
- Each group has a header row: **fold**, **rename**, **move up or down**, **hide** the whole group,
  or **delete** it (LightCraft's looks are taken off this computer and can be restored; your own
  profiles are deleted with their files, or moved to another group, after asking with the counts).
  **New Group…** makes an empty group to move profiles into.
- Your own groups come first: your camera profiles (with the bodies they are the default of) and
  LUT folders are at the top, LightCraft's looks below.

### Bottom bar
- **Every button on the bottom bar is the same size.**

### Scroll bars
- The grid's and the filmstrip's **scroll bars are slimmer and their handle never hides**. The tick strip of picks and rejects above the filmstrip is gone; the position ("1,263 / 2,233") stays.

### Flags, take two
- The marks under the thumbnails are **smaller**, in a slim row.
- A **white line** along the bottom of a picked cell and a **red line** under a rejected one; picked cells are slightly lighter, rejected ones darker.

### Keywords
- **No more ghost text in the Keywords list.** Some cameras write their whole keyword list as one entry, a keyword a line; LightCraft now reads it as one keyword a line, and repairs libraries that already hold such a keyword when they open.

### Show Alternatives
- When a pick doesn't work out while you edit, Photo ▸ **Show Alternatives** (or right-click) shows the frames shot around it, straight from its folder, imported or not. Select one or more and **Import Selected**: they join the library **with the same edit**, and you are back where you were.

### Delete Rejected Photos
- Photo ▸ **Delete Rejected Photos…** takes all the rejects of the view in one go. It asks: remove them from the library (Edit ▸ Undo brings them back), or remove them and **move the files to the Recycle Bin**, with their XMP sidecars.

### The bottom bar
- **A flat bottom bar** like its left side: icons and thin dividers, no islands.
- **Filters with live counts** beside the stars, flags and colour labels you set: Picks, Rejects and Unflagged (combine them: Picks + Unflagged hides the rejects), a rating menu (unrated, 3 stars or more, exactly 5…), each colour label and No label. The Picks count follows every P, so you can stop when you reach your target.
- **More filters** (the … button, or View ▸ More Filters…, Shift+F) has the rest: type, edits, camera, lens, keyword, Save as Smart Album, Clear All Filters and filter presets. The filter bar over the grid and the funnel by the search field are gone.
- Where you are: **"1,234 / 2,233"** (and how many are selected) at the start of the middle.

### Auto Sync
- **Auto Sync now edits all the selected photos live** while you drag a slider, and **each photo moves from its own value**: white balance on each photo's own scale, so photos with another as-shot white balance no longer turn blue.

### Live thumbnails
- In the grid, **the thumbnails of the photos you are editing follow the slider** while you drag it, not only when you let go.

### Finding your place
- **Scroll bars you can see and grab** on the grid and the filmstrip. While the grid scrolls, a label by the bar says where you are ("1,234 / 2,233" and the date).
- The **filmstrip** shows "1,234 / 2,233" and, along its top edge, a tick for every pick (white) and reject (red) through the shoot.
- **Resizing the window keeps your place**: the photos at the top of the grid stay at the top.

### Eye focus strip
- **On while you import, off in the library**: two switches, one for each side; the eye button on the bottom bar (and Shift+T) switches the one where you are.
- **Bigger type** under the eye crops, and **big groups wrap** into rows; when they don't all fit, the closed and soft eyes come first.

### Culling with Caps Lock
- **Caps Lock turns Auto Advance on**: with it on, every pick, reject, rating or label moves to the next photo, as in Lightroom. Photo ▸ Auto Advance still switches it on for good.

### Colour labels
- **Pressing a colour key again takes the label off**, as does choosing the photo's own label in the menu or on the bottom bar's colour chips. Photo ▸ Set Color Label ▸ **No Label** clears it too.

### Culling: flags you can see
- **One flag everywhere.** Picked is a solid white flag, rejected a red flag with a cross, unflagged an empty outline: the same on thumbnails, in the filmstrip, on the bottom bar, in the filters and the menus.
- **Ratings, flags and labels under the thumbnails**, big enough to read at a glance, in the grid and the filmstrip (Settings ▸ Interface ▸ Grid; on). Rejects are no longer greyed out.

### Mask tools
- **Set the look before you draw**: with no mask selected, the Masking panel still shows the
  effect presets and every slider, marked "Settings for the next mask"; the next mask you make
  starts with them.
- **A new mask starts with the last mask's settings**, as in Lightroom Classic. Tick **Reset
  settings on new mask** (remembered) to start every new mask at zero instead.
- **One press of a tool makes one mask**: after you draw a Radial or Linear its tool is put away,
  and clicks or drags on the photo then edit or move that mask; they never make copies. Press the
  tool again for the next one. Pressing a tool that hasn't drawn anything yet puts it away.
- **A new gradient appears where you drag it**: a second Radial landed on the first one's spot.
- **The overlay stays away from masks with settings**: a new mask that starts with settings (the
  last mask's, or a preset's) shows its effect, not the red overlay. The overlay shows by itself
  only for a mask without settings yet, while you draw a mask, and while the pointer is on its row
  or pin; **O** shows it whenever you want.
- **Add, Subtract and Intersect offer every kind of mask** that Create New Mask does, in the same
  order and with the same icons: also Background, People, Remembered, Landscape and Color. A Radial
  minus the Background is two clicks. People, Remembered and Landscape open their picker under the
  mask they combine with. The mask tiles have their own icons now.
- **While you move a slider or turn a dial, the overlay, pins and tool lines step aside** so you
  see the effect itself, even an overlay you switched on with **O**; let go and they come back as
  they were. The same for Remove spots, the crop guides and the red-eye circles.
- The **overlay button** has its own icon (a frame with a tinted area, no longer the Masking tool's)
  and lights up in the overlay's colour while the overlay is switched on to stay; the photo then
  says "Overlay on · O". Its settings menu has a **Show overlay** switch, and choosing how the
  overlay looks there no longer switches it on for good.
- **Gradients show what a drag will do**: the pointer becomes a move arrow over a gradient's body
  or pin, a resize arrow over its edges, lines and handles, and a turning arrow just outside a
  Radial or on a Linear's middle line.
- A **Linear** gradient has its **middle line** (the 50 % point) with the pin on it, as in
  Lightroom Classic: drag it to turn the gradient; drag the outer lines to widen or narrow the fade
  (they stay parallel).
- A **Radial** gradient has an **inner feather ring**: drag its handle in or out to set how soft
  the edge is. A **Feather** slider under the radial does the same; both stay in step.
- **Auto Mask follows the edges while you paint**: the stroke goes to the photo every eighth of a
  second as it grows, so the edge-aware result shows live, not only after you let go. The whole
  stroke is still one undo step.
- The **brush cursor has a small crosshair at its centre**: that is where Auto Mask samples the
  colour and texture to follow, so you can aim it.
- **Color Range picks like Lightroom Classic**: click to pick a colour, **⇧-click to add more** (up
  to five), **⌥-click to remove** one, or **drag over an area** to pick its main colours at once.
  Each picked place has a pin (a dashed box for an area) that you can drag to pick somewhere else.
  The picked colours show as swatches by the Pick button.
- **Luminance Range has a Pick button too**: click the photo to select the tones round the one
  there, or drag over an area to select its tones; ⇧ widens the range.
- **Remove spots no longer hide what they fix**: a painted spot has one thin outline round the
  whole painted area (and one round its source), instead of a ring for every dab; while painting
  you see only that outline, not a white fill. Hover a spot to see its outline and both pins, and
  drag either pin to move the area or choose another source.
- **AI masks no longer show a stand-in while they compute**: Subject, Sky, Background, People,
  Landscape, Object, Describe and Remembered masks show no overlay until the model's result is
  there. Instead the photo says what is being found ("Finding the subject…") with a spinner and a
  **Cancel** button. A simpler method is used only when the AI model isn't installed, and the
  panel says so.
- **Crop shows the crop you get**: with Constrain Crop (on with Upright), the crop box is limited
  to the photo's valid area while you crop, so no white wedges show that disappear when you leave.
- **Rotating turns the photo, not the box**: the crop box stays level, the photo turns under it
  (smaller so it still fits), with a fine grid while you drag.

### Masking panel
- **Describe works without SAM 3.** Type what to select (`white dress`, `car, road`) and LightCraft
  finds every instance with an open model, OWLv2, and cuts each one out with SAM 2.1 (the model
  behind Object). Both download on first use, about 620 MB and 156 MB; Meta's gated SAM 3 is no
  longer needed, and is still used when you have installed it. The first description on a photo
  takes a few seconds while the photo is analyzed; more descriptions on the same photo are quick.

### People masks
- **Entire Person** includes the eyes, brows, mouth, hair and what the person holds, and has no
  more square holes in a lace dress.
- People are found on ordinary couple shots where the faces are small, and each person's mask stays
  on their own outline. Clothes and people edges follow the photo's own edges more closely.

### Canon focus points
- **Show Focus Point** and **Zoom to Focus Point** work with Canon raws (CR3 and CR2): the box the
  camera focused with, including the face it tracked. Photos already in your library get it in the
  background.
- Without a recorded point, Space zooms to the main subject's sharpest open eye instead of the
  middle of the photo.

### Assisted Culling by the eyes
- **Assisted Culling** can reject photos where a main subject has closed or blurry eyes (two new
  boxes in the dialog). It runs in the background, leaves picked photos alone, and says what it
  rejected; one Undo puts them back.

### Faces and eyes while culling
- **Groups**: the eye strip now finds the small faces of big groups (a second, long-range face
  finder): about 62 of 65 people on a 45-megapixel family group, where it found none before. The
  strip takes up to 120 people.
- **Fair sharpness**: an eye is measured at a scale that follows its size and compared with the
  sharpest eye in the photo, so a wide-open 85 mm portrait of five people in focus reads green, not
  red. Missed focus stays red.
- **Closed eyes on faces turned away** (a kiss on the cheek, a 3/4 view) are marked Closed; a closed
  eye is never called "Blurry". Faces in 3/4 view are found and judged more often.
- **Sunglasses**: a dark lens over the eyes reads "Eyes not visible" in grey instead of a green
  frame.

### Presets and profiles in every library
- **Your profiles and presets are shared by all your libraries**, as in Lightroom Classic: camera
  profiles (`.dcp`), LUT profiles, develop presets, local adjustment presets, import settings and
  the default profile of each camera body now live in LightCraft's settings folder. Start a new
  library for the next wedding and everything is there.
- **Nothing to move by hand.** The first time LightCraft opens a library (and the libraries in
  Open Recent Library), its presets and profiles join the shared ones; the same preset or profile
  is kept once. The library's own copies stay where they were, untouched.
- Want one library to keep its own? Settings ▸ Profiles ▸ **Store presets and profiles with this
  library instead**.

### Profile Manager
- **Settings ▸ Profiles** (or **Manage…** in the profile browser, **Manage Profiles…** in the
  profile menu) lists every profile: LightCraft's built-in looks, your LUTs and your camera
  profiles.
- **Hide** a profile to take it out of the menus and the browser (one click shows it again), or
  **Delete** it: your own files are deleted after asking, a built-in look is taken off this
  computer and **Restore Built-in Looks** brings it back. Photos that use a hidden or deleted
  built-in look keep it.
- **Rename**, move to a **group** (or a new one), **move up or down**, star as a favourite.
- **Import Profiles…** and **Import Folder…**, or drop `.dcp`, `.cube`, folders and `.zip` packs
  on the window.
- **Camera Bodies**: choose the camera profile each body's raws get (…71, …74), also one of your
  own profiles made for the whole camera model, and apply it to the photos in view.

### Library backups
- Settings ▸ **Backups**: back up when LightCraft exits **every time, once a day, once a week or
  never**, to a folder you choose (another drive or a network folder), keeping the last 3 to 50.
  Each backup can be checked by opening its catalog.
- **File ▸ Back Up Library Now** runs in the background; **Library Backups…** lists them, and
  **Restore…** opens a backup as a new library beside the current one: nothing is overwritten.
- A backup holds the catalog, the library's settings and LightCraft's settings with your presets
  and profiles. Never the photos or the thumbnail cache, so it takes well under a second.

### Colour
- **Canon EOS R5 C** raws start from a colour profile fitted to nearly 400 of the camera's own
  photos: skin keeps its warmth instead of turning olive in grass-green scenes. `lightcraft-cli
  calibrate` now reads CR3 and CR2 (and RW2, ORF, PEF, SRW, ERF) too.
- Culling a card in place uses each body's default camera profile, so the switch from the camera
  preview to LightCraft's own render isn't jarring.

### AI Denoise
- When no AI Denoise model is available, Settings ▸ AI Denoise says so plainly: LightCraft's own
  model is still being trained, there is nothing to download or install yet, and Luminance and
  Color under Detail reduce noise meanwhile.

### Import and Library tabs
- **Two tabs at the top right: Import and Library.** You always know where you are: Import is a
  folder or card you are looking at (nothing is in your library yet), Library is everything you
  imported, to edit and export.
- **The Import tab** lists every drive of the computer (network drives and camera cards too: a
  card opens at its DCIM folder), the usual places, your favourite and recent folders on the
  left; the photos in the middle, with the same fast grid, loupe and eye strip for culling; and
  the import settings on the right.
- **One big button: "Import 312 into <your library>"**, over the grid and the loupe alike, with
  **Picks / Selected / All** and how many each is. It imports exactly that, never everything by
  surprise, then shows the Library tab while the photos come in.
- **Stack presets on import**: a colour preset, then a retouch preset, then one with AI masks,
  applied in that order (move them up or down). Destination (in place, copy or move, with folder
  templates and renaming), camera profile per body, keywords and metadata are there too, and
  **import presets** keep the whole setup by name.
- **Unattended finishing**: after the import LightCraft computes the presets' AI masks, finds the
  skin for Smooth Skin and builds the previews you chose (none, standard or 1:1), in one row of
  the activity stack with the time left. Go for lunch: it keeps going while you browse, picks up
  again after a restart, and says "N photos ready" when it is done.
- **Copied or moved photos keep their culling**: flags, stars, labels and edits made while
  browsing go with the photos into the library.
- **Browsing writes nothing into the folder** (a client's card or archive) until you import,
  even with automatic XMP writing on; Import ▸ While browsing can change that. **Culling saved**
  shows that your flags and stars are kept, imported or not.
- **No delete in Import.** Delete on a photo you are browsing does nothing but say "reject it
  (X) instead". To remove files, **Photo ▸ Delete Rejected Files from Disk…** moves the rejected
  files to the Recycle Bin after you type how many there are.
- "My Photos" is now called **Library** everywhere.

### Generative Fill
- **The upload works.** Sending a crop to OpenAI failed with "secure connection failed: failed
  to write whole buffer"; large uploads now go out whole. Tested with a real OpenAI account on
  wedding photos.
- **Painting only marks the area.** With the Generative tool, painting no longer runs a heal or a
  content-aware fill that looked like a poor AI result: the photo stays as it is until the
  generated result arrives, with a spinner over the area while it is on its way.
- **Large areas stay sharper.** Instead of one crop at 20–30 % of the photo's resolution, a large
  area is sent as a few tiles at about half resolution or more (**Sharper result for a large
  area**, with the number of requests and the cost); untick it for one cheaper, softer crop.
- **Variations** is 1 unless you choose more, and LightCraft remembers it; the price per
  variation shows before you generate.
- **Only the services you set up** appear in the Remove panel.
- **A short, calm first-use dialog**: what is sent, its size and the cost, Generate or Cancel; the
  data policy and watermarks are behind Details.
- **Failures are clear**: the message stays in the Remove panel with Retry.
- **Which service for what**: Settings ▸ Generative Fill has a card per service (best at, weak
  spots, size and price, training on uploads, marks added) and a recommendation.

### Deleting photos
- Deleting photos now says plainly what happened: they went to **Recently Deleted**, the files
  stay on disk, and **Undo** brings them back.

### Control surfaces: safer keys, Erase while held
- **Erase** works while its key is held, like Alt with the brush, and lights up only then. New
  Brush runs once and never stays lit. A key lit because its value is on the dial, because it is
  held, or because a mode is on now looks different for each.
- **Delete, Reset and other keys that undo work need a short hold** (0.7 s) on the console; a
  quick press only shows "Hold the key". The built-in layouts have no Delete key any more.
- The brush page has **Brush Size** and **Feather** keys; Mask Above / Below are now **Previous
  Mask** / **Next Mask**.

### Control surfaces: icons of their own
- Invert Mask, Erase, New Brush, Previous / Next Mask, Mask On/Off, the zoom, selection, rotate
  and Reset keys have their own icons instead of sharing one, drawn in the style of the icon pack.

### Loupedeck+
- The **Loupedeck+** works with LightCraft through the same plugin as the MX Creative Console.
  With no setup each knob and button does what its label says. Settings ▸ Controls ▸ Loupedeck+
  shows every control and lets you change it, and **imports your own Loupedeck profile**: the
  profiles on this computer are listed, one click each. Only the layout comes over; the report
  lists what didn't (macros, presets LightCraft doesn't have).

## 2026-10-15

### Settings window
- **Settings keeps its size.** Switching tabs no longer makes the window grow, shrink or jump:
  a long tab scrolls inside it. Drag its edge or corner to make it the size you like; LightCraft
  remembers that size, and the tab you used last, for the next time.
- **One row of tabs** across the top, the chosen one underlined. When the window is too narrow
  for all of them (or the names are long, as in Russian), the rest are under **More** at the
  end of the row; they never wrap onto a second row.
- **Ctrl+Tab** and **Ctrl+Shift+Tab** (or Ctrl+Page Down / Page Up) go to the next or previous
  tab; with a tab focused, the left and right arrow keys move between tabs.
- Every tab lines up the same way: section titles with a thin rule under them, names of settings
  in one column and their controls beside them. Long names wrap in their column, and rows of
  buttons wrap instead of running off the edge.

### Masking panel
- The **mask overlay** shows only while it helps: for a mask you have just made, while you paint or
  draw, and while the pointer rests on a mask's name or pin. Adjust the mask and you see its effect;
  presets never leave the overlay on. **O** shows or hides it; its colour and opacity are in the
  small menu by the Masks title.
- Masks are a **tree with thumbnails**: each mask, and under the selected one its parts, with Add,
  Subtract, Intersect and Invert. The **eye** switches a mask's effect on and off; the **bin**
  deletes it. **Amount** (the strength of the mask's effect) is the first slider.
- **Tools wait for the photo**: Brush, Linear, Radial, Color and Object make their mask with your
  first stroke, drag or click on the photo; clicking the tool again puts it away. No more empty
  masks.
- Masks are **named after where they come from**: the preset ("Sun Kissed"), the person ("Groom
  Jamie - Clothes"), or what they select ("Radial Gradient − Linear Gradient"). Rename one and your
  name stays.
- **Brighten Dress** and the other People masks for one person follow that person's outline, so an
  arm around the bride or the groom's cuff on her waist stays out, and their edges follow the dress.

### Constrain Crop tightens the crop you see
- Geometry ▸ Constrain Crop now changes the crop itself, as in Lightroom (issue #1927). Ticking it, or moving a
  Geometry slider, Upright or a lens correction while it is ticked, shrinks the crop about its centre until the blank
  corners left by the correction are gone, and the Crop tool shows the tighter crop. Before, it only applied when the
  photo was shown outside the Crop tool, so in the Crop panel, where the checkbox is, nothing seemed to change. A crop
  you draw by hand is kept as drawn, and unticking keeps the tighter crop.

### Web: no backup that misses unsaved changes
- In the browser version, File ▸ Back Up Library… no longer reports success while a save is failing (storage full or
  cleared): the backup copies the stored library, so it left out the changes that couldn't be saved (issue #1508). It
  now refuses to start and says how many changes are unsaved and why. LightCraft retries the save with every change;
  back up once nothing is left unsaved.

### Web: an unreadable library is not opened
- In the browser version, a library file that exists but can't be read at start-up is no longer taken for a missing
  one (issue #1507). LightCraft used to open the library without it, showing an incomplete catalog that the next save
  would write over the stored copy. It now leaves the library unopened, saves nothing and says which file couldn't be
  read; reload the page to try again.

## 2026-10-14

### Model downloads time out on a name that doesn't resolve
- Model downloads (AI denoise, faces, SAM) give up on a server name that doesn't resolve within the connect timeout,
  and Cancel stops a download that is still looking the name up (issue #1532). The lookup used to block with no time
  limit, and each of a server's addresses then got the whole connect timeout of its own.

### Lightroom radial masks on the right side
- Radial masks brought over from Lightroom Classic (catalog import, XMP sidecars, edits inside DNGs and presets) now
  apply their effect inside the circle, as Lightroom does, instead of outside it (issue #1815). A mask inverted in
  Lightroom stays inverted, and older Radial Filters keep their Effect Outside / Invert Mask setting. Photos imported
  before this keep the masks they got: Photo ▸ Read Metadata from File reads a sidecar's again.

### Delete in an album removes from the album only
- In an album, the Delete key now takes the selected photos out of the album, as in Lightroom. It used to move them
  to Recently Deleted, so they also left their folder's view (issue #1805). They stay in the library and in their
  folder; Edit ▸ Undo puts them back in the album. Photo ▸ Remove from Album does the same and now shows the key.
  To delete photos from the library from an album, use Photo ▸ Delete Photo or right-click ▸ Delete Photo. In a smart
  album, Delete changes nothing: its rules decide what it shows.

### Local folders select a photo
- Showing a Local folder now makes its first photo the active one, as albums and the other sources do (issue
  #1447), or keeps the photo you had when the folder holds it. Before, the photo from the previous view stayed active:
  Photo Grid marked nothing, and Detail, the histogram and the photo info showed a photo that isn't in the folder.

### Nikon Z 6II, Z 8 and D5300 colour profiles
- Raws from the Nikon Z 6II, Z 8 and D5300 now start from a colour profile built from a few hundred photos from each camera, the same way the
  Sony and Fujifilm profiles were made, instead of fitting each photo's colour on its own. Colours that one photo
  shows only a little of come out closer to the camera's JPEG.

### Closer camera colour for lens-corrected raws
- Raws from cameras that correct their own JPEG for lens distortion (Panasonic and Fujifilm with that setting on, the
  Sony A7R IVA) now start from colours closer to the camera's. The starting look was being fitted against a JPEG that
  didn't line up with the raw near the edges. Photos you already imported pick this up when their previews are rebuilt.

### HDR merges of moving lights, and many brackets at once
- A star, the moon or a light that moves between the photos of a bracket (a night bracket on a tripod) no longer
  leaves a black hole where the middle photo had it clipped. With Deghost, the brightest photo's copy of it in a
  dark sky is now removed too.
- Photo ▸ Photo Merge ▸ HDR for Each Bracket Set merges every bracket among the selected photos with the last HDR
  settings, one after the other: each stack, and photos taken less than a second apart at different exposures.
  From the command line: `lightcraft-cli merge hdr-batch FOLDER`.
- Merging uses about a quarter less memory: three 67 MP raws need 3.8 GB instead of 4.8 GB.

### XMP sidecars catch up after a network share comes back
- With automatic XMP writing on, an edit to a photo whose folder can't be written (a network share that dropped
  out, a folder that went read-only) used to be written only to the library, with a line in the log, so the sidecar
  stayed behind the edit. LightCraft now says once that the sidecars are waiting, tries again every few seconds once
  the files are reachable, and says when they're written. `library.retrySidecars` does it on request.

### Reliability
- A damaged or crafted raw can no longer make LightCraft set aside gigabytes of memory before finding out the file is
  bad (issue #1512): a compressed PEF whose data is too short for the size it declares, an uncompressed TIFF-based
  raw whose tile claims far more samples than it holds, and an ARW6 with a broken tile are refused first. All 74
  files in the test corpus still decode.

### Reliability
- Thumbnails and previews are written to the cache on disk by one background writer instead of a new thread for
  each image (issue #1522). When renders come in faster than the disk takes them, at most 128 MB of images wait;
  past that a write is skipped (the image stays in memory and is written by a later render), so a slow disk can't
  pile up threads and memory.

### Reliability
- Saving the catalog and resuming a model download no longer write through a link left at the temporary file's
  name (`catalog.snap.tmp`, `model.onnx.part`): the link is removed and a new file made in its place, so the file it
  pointed to is never changed (issue #1502).

### Reliability
- A photo whose file was replaced (Reload Changed Files after an outside edit) no longer keeps the faces recognition
  made from the old pixels: they are made again from the new file, and a scan still busy with the old file when it
  changed doesn't put its faces on the new one (issue #1524).

### Settings stays put when you change tabs
- The Settings window keeps its top edge and width when you switch tabs, so the tab row no longer jumps around
  under the pointer. It used to be centred, so every tab of a different height moved it.

### Layout image over the loupe
- View ▸ Loupe Overlay ▸ Choose Layout Image… draws a PNG with transparency over the photo in the loupe, such as
  a magazine cover's masthead and cover lines, so you can frame a shot for the page it is made for. It fits inside
  the photo, follows zoom and pan, and ⌥⌘O shows or hides it. The same menu sets its opacity and a matte that
  darkens the photo outside it. The layout stays chosen across launches.

### Smaller JPEG exports
- JPEG exports are 12–14 % smaller at the same quality, with exactly the same pixels (issue #1373): each file now
  carries Huffman tables made for its own picture instead of the standard example tables. Every viewer opens them
  as before.

### Reliability
- A photo whose Defringe amount was stored far past the sliders' 20 (in a damaged catalog or sidecar) no longer
  crashes LightCraft each time it renders: the amount acts as 20.

### Lens names for Nikon DSLR raws
- Photos from Nikon DSLRs like the D700, D750, D80 or D5300 now show which lens they were taken with, for example
  "24.0-70.0 mm f/2.8". Those cameras don't write a lens name, so the lens field was empty and you couldn't filter or
  search by lens. LightCraft now reads the lens range Nikon keeps in its own maker note, the way Lightroom shows a
  lens it doesn't have a name for. Newly imported photos show it.

### Import when a card is connected
- Settings → Import → Cameras and Cards has an option to open the import review as soon as a camera or memory card
  is connected, the same review File → Import from Device opens. It's off by default. A card that's already in when
  LightCraft starts doesn't ask, and if a dialog is open or an import is running it waits until that's done.

### Local folders
- Photos deleted (or moved) by another app no longer stay in a Local folder, failing to load and logging an error
  each time you look (issue #1392): the next time you browse the folder, in a later session, they're left out. A photo you had rated, flagged or
  edited stays, marked missing, so its changes aren't lost; photos in My Photos are unchanged.

### Full-resolution panoramas
- Panorama and HDR Panorama merges keep the resolution of their photos instead of stopping at 40 megapixels
  (issue #1455), up to 512 MP and 65,000 pixels on the long edge, as in Lightroom. When the full size needs more
  memory than the computer has, the panorama is made as large as fits (five 51 MP photos: up to 166 MP with
  16 GB, 355 MP with 32 GB) and the message says so. The merge also frees each photo's pixels once they are
  projected, so it needs less memory than before at any size (an 81 MP stitch: 4.4 GB instead of 6.2 GB).

### Nikon D600, D610 and D5000
- Raws from the Nikon D600, D610 and D5000 no longer show a bright line down the right edge (issue #1627). Their
  sensor data runs a few columns of filler past the picture, which stayed in the image; these bodies now develop
  at the camera JPEG's size (6016 × 4016 for the D600 and D610, 4288 × 2848 for the D5000, as the D90 already
  did). On the D600 and D610 the white level is also measured inside the picture, so the filler can't pass for
  clipped highlights.

### More Canon EOS R6 Mark II C-RAW files decode
- Some EOS R6 Mark II C-RAW files (about 1 in 50 in one photographer's library) opened as their preview with
  "Canon CRX adaptive QP value 172" or "175". They now decode from the sensor data. Photos imported before this
  switch over with Photo ▸ Reload from Disk.

### Nikon raws without coloured edge bars
- Raws from older Nikon bodies no longer show a thin coloured or dark bar along one edge (the right edge, or the top
  of a portrait photo): masked sensor columns were being developed as part of the picture (issues #976, #914,
  #1236). The D7000, D5100, D800, D800E, Df, D4, D3, D3S, D700, D300, D300S, D600, D610, D810, D4S, D5600,
  Coolpix A, D3000, D40X, D60, D80, D2Xs, D200, D3200, D3100, D5000, D40, D50, D70, D70s and Nikon 1 V1 now show
  the area the camera itself frames, a few pixels smaller than before.

### An empty library leads to Import
- A library with no photos now opens on the Photo Grid with the sidebar (Square Grid stays as it is), instead of a
  Detail view that pointed at an empty grid. The grid and Detail view offer Import Photos… and, in the desktop app,
  Import Lightroom Catalog… as buttons, and say that photos (and, in the desktop app, folders) can be dropped into
  the window. The shortcut they show is the one Import Photos… has now.
- The top bar has an Import button, beside Export, in every library.

### Reliability

### One kind of button
- Dialogs' buttons and About's Discord link are drawn by one button component: primary (filled with the accent),
  secondary, danger (filled red) and ghost, in three sizes, with an icon or without, and an icon alone (named for
  screen readers). Each darkens a little when hovered and more when pressed, shows a ring where the keyboard is, and
  its text reads at 4.5:1 or more on its fill in every theme. Dialog buttons are 30 points tall, like the welcome's.

### A dialog's confirm button says it's the next step
- In dialogs, the confirm button is on the right with Cancel to its left, and filled with the accent when it makes or
  brings in something (Import N Photos, Merge, Save, Create, Download…). Delete and Remove stay plain, so a destructive
  choice is never the eye-catching default.

### Straight brush strokes with ⇧-click
- With the brush, click once, then ⇧-click somewhere else: the brush paints a straight line between the two, and
  further ⇧-clicks carry on from the last point, as in Lightroom. Change the brush size, feather or flow before the
  ⇧-click and the line tapers from the old values to the new ones. The whole line is one undo step.
  `mask.brushStroke {connect: true}` does the same for agents.

### Hard-edged Heal and Clone spots
- A spot with Feather at 0 no longer draws black pixels exactly on its radius. This also fixes spots at 0% Opacity
  changing pixels along their edge. The hard edge now leaves pixels on the radius unchanged.

### Smooth Skin and Find Blemishes
- **Smooth Skin** (Edit panel, beyond Lightroom): evens out skin tone and texture while keeping the pores. Amount,
  Texture, Even Tone and Redness; the face's skin, the body's, or both; everyone or one person. It starts subtle,
  never touches eyes, brows, lips or beards, and a preview looks like the export.
- **Find Blemishes** (Remove panel) now uses the same open models: it looks only on the skin they find, cuts out
  the eyes, brows, nostrils and lips by the face's landmarks, and can include the shoulders, arms and back.
- **Detect Faces** and the face scan use the open MediaPipe face model too. YuNet is no longer offered: its training
  data doesn't allow commercial use. If you installed it, Settings ▸ Faces lists it so you can remove it.

### Smart AI-mask presets
- Presets remember their **AI masks as recipes**: applying one finds the sky, the subject, the
  background or the people again on each photo, in the background, with one undo step per preset.
- Presets work as in Lightroom Classic: their settings are set and their masks are **added**, so
  they stack (a look, then Recover Blue Suit, then Bring Sky Down). Clicking the same preset again
  right away replaces its own masks instead of doubling them.
- **People by name and role**: the People picker shows the faces on the photo; name them and make
  them the Bride or the Groom of the shoot, and masks and presets follow them on other photos.
- **Remembered objects**: select a wedding dress (or anything) and Remember Selected Mask As…;
  LightCraft finds it on other photos.
- New **Smart Masks** presets: Sun Kissed, Recover Blue Suit, Bring Sky Down, Brighten Dress, Darken
  Background. Lightroom presets with AI masks import with their masks.
- Resting on a preset previews its AI masks too. Face data stays on your computer; People and Roles
  can forget it.

### AI masks with openly licensed models

### Better Content-Aware fill, and Generative fill with your own account
- **Content-Aware** now continues straight edges and thin lines through the area you paint — a
  horizon, a door frame, a power line — instead of breaking them, never copies from other spots or
  from faces, and takes in a little more than you painted so soft edges and the start of a shadow go
  too.
- **Select, then remove**: make a mask (an **Object** selection with one click, a brushed area,
  people) and click **Fill Selected Mask** or **Generate into Selected Mask** in Remove.
- **Generative** (new tool in Remove): fills the area with an image model you choose — OpenAI,
  Stability AI, Black Forest Labs or fal.ai — using **your own API key and account** (Settings ▸
  Generative Fill). It is off until you add a key; nothing is sent until you click **Generate**, and
  the first time a dialog shows exactly the crop that will be uploaded, what the service does with it
  and what it costs. Only a crop around the spot is sent, never the whole photo or its metadata.
- The result goes back into the photo's own raw-level pixels, before white balance: change white
  balance, exposure, colour or noise reduction afterwards and the filled area follows like the rest
  of the photo, with the photo's own grain. Pick between up to 4 variations.
- See docs/generative-fill.md for the details and the limits (8-bit highlights, softer patches when
  a very large area has to be downscaled).

### AI masks with open models
- **Subject**, **Background**, **Sky**, **People** and **Landscape** masks now use AI models instead of
  guesses, and **Object** works without Meta's SAM 3: each model is downloaded once, when you first
  need it, after a dialog that shows its size and licence. Without a model the mask is made the old
  way and the Masking panel says it is approximate, with a button to get the model.
- **People**: pick everyone or one person and the parts — face skin, body skin, hair, clothes,
  eyebrows, eye whites, irises, lips, teeth.
- **Landscape**: sky, water, vegetation, mountains, architecture, natural or artificial ground.
- **Update** recomputes a mask made before its model was installed.
- Everything runs on your computer, in LightCraft's own code; nothing is uploaded.

### Hot pixels and sensor dust, remembered per camera
- Hot, dead and stuck pixels are fixed on raw photos automatically, before the photo is developed: the clear single
  outliers each photo shows, and every pixel a camera has shown in photos of different scenes. LightCraft remembers
  each camera body (by serial number, else the first two letters of its file names) and keeps learning as you browse,
  so faint hot pixels that one photo can't prove are fixed once three photos agree. Stars, catchlights and glints on
  water or sand are left alone. Detail ▸ Sensor ▸ *Fix hot and dead pixels* turns it off for a photo.
- A dark frame (a raw with the lens cap on) gives a camera's hot pixels at once: `camera.pixelMap.calibrate`.
- **Remove Dust** (was Find Dust Spots, in the Remove panel) works on a selection too, in the background. Its spots
  are outlined amber in the Remove tool, and each one is healed or evened out, whichever matches its surroundings
  better. It also remembers each camera's dust — where it sits on the sensor, how big it looks at each aperture, and
  when it moved — and adds that dust to later photos where it shows on smooth areas. Settings ▸ Import can run it after
  every import. See [sensor-defects.md](sensor-defects.md).

### Control surfaces: easier to set up
- Settings, Controls now shows your MX Creative Keypad and Dialpad as drawings, every key and dial with its icon and
  name. Click one and pick what it does from a list of actions in categories, with icons and a search box. Switch
  between the Keypad, the Dialpad and the Actions Ring at the top; each shows whether it is connected.
- LightCraft installs its Logi Options+ plugin for you: when Logi Options+ is on the computer, Settings, Controls
  offers "Install the plugin" (and later Update or Remove). Logi Options+ asks you to confirm.
- In Logi Options+ the console's keys now show LightCraft's icons instead of warning triangles, and every LightCraft
  action and slider is listed there by category, ready to drag onto a key, a dial or the Actions Ring.
- The readout in the window shows the icon of what you turned or pressed.


### Control surfaces: the Logitech MX Creative Console
- Turn dials and press keys on a Logitech MX Creative Console to edit: the dial moves Exposure, Temp, a mask's
  sliders or the brush size, accelerating when you turn fast, and each turn is one undo step. With "Dials change
  every selected photo" a turn edits all of them at once.
- The keypad follows what you do: Crop puts crop keys on it and Straighten on the dial, Masking its mask tools, a
  dialog OK and Cancel. A readout in the window shows the value as you turn.
- Settings, Controls holds it all: six ready profiles (Default, Wedding, Portrait, Fashion, Landscape, Real Estate),
  a layout editor where you touch a key on the console to change what it does, dial speed per slider, and pairing:
  a new console shows a code you approve once.
- Coming from TactileBridge? One click brings your dial tuning, readout, colours and tool layouts over.
- It is TactileBridge's engine, contributed by its author; the console connects through the LightCraft plugin for
  Logi Options+.

### Where the camera focused
- **View ▸ Show Focus Point** (⌥F) draws the autofocus frame the camera recorded over the photo, in Detail, Compare
  and Survey, following rotation and crops; the Info panel says how it focused (`AF-C · Wide · Animal Eye Tracking`).
  Lightroom has no such view.
- **Z zooms to it**: Toggle Zoom centres on the focus point, so checking that the subject is sharp is one key
  (View ▸ Zoom to Focus Point turns this off). Photos without a point zoom as before.
- Sony α1 (ILCE-1) and A7S III (ILCE-7SM3) raws for now; manual-focus shots and Wide shots that only report the frame
  centre show nothing. More cameras will follow. Details: [focus-points.md](focus-points.md).
- **Photos already in your library get theirs in the background**: LightCraft reads just the few kilobytes of each
  file that hold the focus point (about 100 photos a second from a spinning disk), with a *Reading focus points* row
  in the activity stack you can stop. Photo ▸ Read Focus Points runs it by hand.

### Cleaner HDR merges
- Bright windows in an HDR made with Photo ▸ Photo Merge ▸ HDR no longer take a magenta tinge from the brighter
  photos along the areas they clipped: right next to a clipped area, a photo's colour isn't used. It showed most when
  the highlights were darkened or saturated.
- The same photos give the same HDR whatever order they are selected in; before, a different order could change a
  few pixels.

### HDR merges keep the white balance
- An HDR merged from raws that LightCraft colours from the camera's JPEG (CR2, NEF, ARW…) now opens at the same
  As Shot temperature and tint as its photos, and changing the white balance no longer turns it green. Sony ARW
  merges used to open at about 7000 K / +115 where the photos show 4000–5300 K / +8. HDRs merged before this keep
  their old white balance; merge them again.

## 2026-10-13

### Edits keep up in a large library
- A small edit (a keyword, a rating, a flag, a slider) no longer has the grid's list worked out again from every
  photo: only the photos the edit touched are asked about again and put at their place in the order. In a library of
  67,000 photos that step went from 37 ms to under 1 ms. Still worked out whole: an edit to more than 256 photos, a
  change to an album, a stack or the keyword list, an album shown in its own order, Recently Added, Missing Photos,
  and a view that asks about "the last N days".
- With no filter set, the count of the source is taken from the list instead of from a second pass over the library.
- The side panels keep a tally an edit cannot have changed: a rating or a slider changes none of the folders,
  keywords, dates, people and counts; a keyword leaves the folders, the dates and the counts as they are. With both
  side panels open, the frame after a keyword went from about 156 ms to about 54 ms in that library (2 ms with the
  panels closed, from 71 ms).
- The count beside each smart album in the left panel follows an edit the same way: the album asks only the photos
  the edit touched, where it asked every photo of the library after each edit (about 25 ms an album in that library).
  With that, the frame after a keyword is at about 29 ms there. New rules, a change to an album or the keyword list,
  an edit to more than 256 photos, and each new minute for an album that asks about "the last N days" are still
  counted from every photo.
- Still tallied from every photo on each edit: the filter bar's lists of cameras, lenses and keywords.

### A view can hold its photos until you refresh it
- View ▸ Hold View Until Refreshed (off unless you turn it on; kept with the library). With it on, an edit that makes
  a photo stop matching the view keeps it in the view: a keyword given in a smart album of photos without keywords,
  a flag taken off in Picks, a rating that falls under the filter bar's. With it off, as before, the photo leaves the
  grid at once and every photo after it moves up. View ▸ Refresh View (also in the sort menu while the hold is on),
  Reshuffle, another source, filter or sort, or new rules for the smart album shows what matches now.
- Photos that start matching still appear at once, and deleting a photo or taking it out of the album or folder
  shown still takes it out of the view.

### The window opens at the size you left it
- LightCraft's window opens at the size it had when you last used it, and zoomed (macOS) / maximized if it was: it
  used to open at 1600 × 1000 every time. Double-click the top bar to zoom the window, quit, and it is zoomed when
  you come back; un-zoom it and it goes back to the size it had before.
- Full screen is not remembered (the app opens as a window), and neither is where on the desktop the window was:
  the system places it.
- On macOS the system does not tell LightCraft when a window is zoomed from the Window menu or resized after a
  zoom, so it goes by the window's size, and when in doubt takes the window for a window of the size it has: it
  comes back as large as you left it, but not as a zoomed window. That is so after a zoom from the Window menu
  with the Dock at a side of the screen, or of a window already as wide as the screen. Zoomed by a double-click
  on the top bar it comes back zoomed. The other way round, a window you drag out until it is as wide as the
  screen and nearly as tall counts as zoomed.
- For scripts: a size set with `ui.resize` is remembered like any other, so a script that needs a size sets it.

### HDR merges keep the lens correction
- An HDR made with Photo ▸ Photo Merge ▸ HDR now keeps the lens correction the photos carry (Sony's, Panasonic's
  and Leica's, and the one some DNGs carry, e.g. from drones): Optics ▸ Enable Lens Corrections straightens it as it
  does the photos it was made from. HDRs merged before this don't have it; merge them again.

### Portable mode
- A file named `portable.txt` next to the LightCraft program makes it keep its settings, logs and downloaded
  models in a `LightCraftData` folder beside it instead of the per-user settings folder (issue #1200), as
  PhotoCraft does. If that folder can't be written, the usual folder is used and the log says why.

### Reliability
- A photo whose Defringe amount was stored far past the sliders' 20 (in a damaged catalog or sidecar) no longer
  crashes LightCraft each time it renders: the amount acts as 20.

- Importing a preset zip that unpacks to far more than any preset (a "zip bomb") is refused with a message naming
  the file, instead of asking for gigabytes of memory, which could end LightCraft.

- A crop of no size (from an agent or a sidecar) no longer gives the Detail view a photo of no size to lay out
  (its width over its height was 0 / 0, which stopped debug builds); it shows as a single pixel's colour.

- A crop a hair wide (or of no size), set by an agent or read from a sidecar, no longer crashes or hangs the
  render with Clarity, Dehaze, Texture or Highlights / Shadows on: their blurs are held to the picture's size.
- A crop of no size with healing spots or masks no longer crashes the render: samples outside the picture read its
  edge.

- A library whose catalog file was damaged (or edited by hand) in a way that still reads no longer crashes
  LightCraft when it shows a stack, or hangs it when a folder is moved: on opening, records are put back under
  their own ids, stacks keep only photos that exist (at least two), folders inside themselves move to the top,
  and new photos never reuse an id in use. What was repaired is written to the log.

### Nikon lens corrections
- Raws from the Nikon Z 6II, Z 8 and Z f now open with the camera's own distortion correction, as Lightroom applies a
  built-in profile: straight lines near the edges stay straight and the framing matches the camera's JPEG. It works
  with Z lenses and with F-mount lenses on the FTZ adapter, follows the in-body stabilisation's shift of the optical
  centre, and can be turned off under Lens Corrections. Vignetting correction from the file comes later.

### Thumbnail cache stays in its folder
- The thumbnail cache no longer follows a shard folder (`thumbs/ab/`) that is a symlink or junction to somewhere
  else (issue #1504): trimming or clearing the cache can't delete files outside it, and thumbnails are never read
  from or written through such a link.

### Synchronize Folder asks a network share less
- A Synchronize Folder scan no longer asks the disk about every photo of the library in the folder (is its file
  there, has it a sidecar): it has just listed the folder, and reads the answers off the listing. On a network share
  each of those questions is a round trip. Measured on a hard disk over SMB, for folders of 1,600 to 2,400 photos
  that had not changed: 4.6 to 7.1 s of questions after the listing (about 3 ms a photo), now under 0.05 s. The
  listing itself took 4 to 5 s and is as it was.
- The disk is still asked wherever a listing can't be sure what a file system would say: about a name that is
  there in another case, a link, a name Windows reads another way (`aux.jpg`, a `~`, a `:`, a dot or a space at the
  end), a folder the scan did not walk to its end, and, in a folder that holds any name with a character outside
  ASCII (an umlaut, a curly apostrophe, a dash), about every name the folder does not list. Names it does list are
  answered there too. In such a folder the scan saves less, and its checks now start after the listing where they
  ran beside it.
- A photo the folder lists is there for the scan, also where its file could not be opened by its path (a folder
  that may be listed but not searched, a path longer than the system takes): such photos used to be listed as
  missing. And a share that drops after the scan has listed a folder no longer turns that folder's photos into
  missing ones.

### Large libraries: Synchronize Folder, import and sidecars start sooner
- Opening Synchronize Folder, an import review or an import no longer makes a table of every photo in the library
  before it looks at the first file. In a library of 3.5 million photos that held the window for about 6 s before
  a Synchronize scan began (now under 1 s), 2.6 s before an import review (now 0.05 s) and 1.7 s before an import
  (now 0.05 s). The scan looks up only the files it finds and the contents it reads, on its own thread.
- Writing one photo's XMP sidecar (Save Metadata to File, or automatically after an edit) no longer works out who
  owns every shared sidecar name in the library: 2.4 s a write in that library, now under 1 s.
- Nothing else changes: the same files are found new, already there, moved or missing, and sidecars get the same
  names.

### Faces load faster in the People view
- Faces on unedited raws are cut from the camera's preview inside the file instead of developing the raw (issue
  #1440): a Sony α1 face reads 8 MB instead of 70 MB, an A7S III face about 1 MB instead of 24 MB, several times
  faster on an external hard disk. A face whose preview is too small, and any edited photo, is still developed from
  the raw, so it keeps its look.
- Raws whose preview is in Adobe RGB (Sony α1 among others) no longer read the whole file for their grid thumbnail:
  only the header and the preview are read, as for other raws.
- Faces are kept in the thumbnail cache at a few fixed sizes, so moving the thumbnail slider no longer renders them
  all again.

- A raw file whose black level is stored as "not a number" (or infinity), or a floating-point DNG with such
  samples, no longer crashes LightCraft at 1:1 or on export: the bad values read as black.

- Exporting a photo more than 65,535 times as long as it is wide (a one-pixel strip), or opening the Export dialog's
  watermark preview for it, no longer crashes LightCraft; the export is capped at the encoders' 65,535 pixels.

- Dragging a Tone Curve point that sits very close to its neighbours (two clicks side by side, or a curve from a
  sidecar, a preset or an agent) no longer crashes LightCraft; the point stays between them. Curve points sent to
  `develop.curve` are kept in order, and a sidecar curve with a "NaN" or "inf" point is ignored.

- A demo photo whose stored size was damaged (a width of 4294967295) is rendered at its own size instead of
  one that overflows.

### DNG camera identity
- DNG export and Convert to DNG no longer duplicate the camera make when the model already
  contains it: `Canon EOS R6` stays `Canon EOS R6`, rather than `Canon Canon EOS R6` (the camera
  identity subissue in #840). The separate Make and Model tags keep their original values.
  This does not change colour matrices, camera calibration or RAW noise rendering.

### Building from source
- The English and Simplified Chinese build guides now describe command-local job and release
  profile overrides for memory-constrained builders (issue #1196), including why packaging
  recipes can override inherited settings. Default release profiles and application behavior
  are unchanged; the guidance does not guarantee a build will fit every 16 GB machine.

### Lightroom catalog import
- A Lightroom Classic catalog whose changes are all still in its `.lrcat-wal` file (a new catalog Lightroom has
  open and hasn't written back yet) imports instead of failing with "unsupported SQLite schema format".

- An export enlarged far past the photo's size (20000 typed for 2000 with Don't Enlarge off) no longer ends
  LightCraft: an enlargement stops at 512 megapixels, as in Lightroom, and one that needs more memory than the
  computer has is refused with the size it would take, before it starts.

### Camera profiles: per body, and looks on top
- LightCraft's profiles (Color, Neutral, Vivid, the creative ones…) are now looks that sit on the photo's camera
  profile instead of replacing it: pick Neutral on your own chart-made profile and you get Neutral on your camera's
  measured colour. The profile row shows the look, with "on Canon EOS R5 C 71 · Daylight · Photo" under it (long
  names are shortened to fit the panel; hover for the whole name). Camera Profiles ▸ LightCraft Camera Color goes
  back to the built-in colour.
- A profile made from a chart remembers the camera body it was shot with. Raws from that body imported later
  develop with it on their own (never over a profile you picked); Photo ▸ Apply Camera Body Profiles does it for
  photos already in the library. The Camera Profiles menu shows your body's profiles; Browse Other Camera Profiles
  shows the ones made on your other bodies, and Adopt for This Body makes one of them this body's too. A profile
  made from a photo is always that body's own, also in libraries imported before body serials were kept (LightCraft
  reads the serial from the file); one of the body's profiles that isn't its default offers "Make default for this
  body".
- Hovering a camera profile in the profile menu or browser now previews its colours.
- The Import dialog has a Camera profile section. Automatic lists each camera body on the card ("Canon EOS R5 C 71
  → its profile (412 photos)"); Choose… gives a body another profile for this import, and "Adopt for this body"
  keeps it. One profile for all photos does what it says. A Look puts one of LightCraft's looks on top. Photos with
  Lightroom settings in their XMP sidecar keep them, and the finished import says which profile went where.

### Create Camera Profile from Color Chart, reworked
- The chart photo shows developed and the right way up; zoom with the wheel, move with a right-drag or Space-drag,
  double-click to fit, and a magnifier shows the corner you are placing.
- Say which light the chart was photographed under: Daylight / Flash or Tungsten (the second chart starts on the
  other). The chart's greys show what they measure, for information; the camera's own white balance is never taken
  for the light.
- Photographic (colours match the chart, lightness as the camera recorded it) and Reproduction (everything
  matches the chart exactly) replace Accurate and Natural.
- Profiles are named "Canon EOS R5 C 71 · Daylight · Photo · 2026-10-13" (camera, body, light, mode, date; a camera that records no serial number is told apart by the first two characters of its file names, as in "Sony ILCE-9 SA") until you type
  another name. Creating one again asks whether to replace it (photos using it follow) or keep both.
- The second chart is picked from the library, photos with a chart first.


### Camera profiles from a color chart
- Photo ▸ Create Camera Profile from Color Chart… makes a DNG camera profile for your own camera from a raw photo of
  a ColorChecker 24. The chart is found on its own (drag its corners if it isn't); choose the light it was shot
  under (or let LightCraft tell from the chart's greys), Accurate or Natural, and a name. The new profile is listed
  under Camera Profiles like an imported `.dcp`, the chart photo uses it right away, and a report shows how close
  each patch comes (ΔE).
- Add a second chart photo under another light, such as tungsten, for a dual-illuminant profile: each light gets its
  own calibration, and photos in between are rendered by their white balance between the two.
- The profiling engine is RADcamProfiles, contributed by its author.

### Confirm matching faces together
- On a person's page, the ↑ control on a suggested face confirms it and all shown suggestions with equal or higher
  similarity as one undo step. Hover or focus that face and press A for the same action; hidden suggestions stay hidden.

### Noise reduction looks the same at every size
- Luminance and colour noise reduction no longer look much stronger in grid thumbnails and the filmstrip than in
  the loupe (issue #960). Their radii are now set in the original photo's pixels, so a thumbnail, the fitted loupe
  and a smaller export look like the full-size result scaled down; 1:1 views and full-size exports are unchanged.
  Like in Lightroom, judge noise reduction at 1:1: at small sizes the noise it removes is mostly averaged away.
  Cached thumbnails are redrawn.
- When the graphics driver resets the GPU while LightCraft is exporting or rendering (issue #1337, seen on Windows
  with NVIDIA), LightCraft still stops, because its window can't keep drawing on the lost device, but it now says
  "LightCraft stopped unexpectedly" and why, instead of "could not open its window", and the next start renders
  on the CPU and explains how to turn the GPU back on.

### Resetting a slider
- Resting the pointer on a slider's name or its thumb now says that a double-click resets it ("Exposure — double-click
  to reset it"), in every language (issue #1436).
- A double-click on the thumb is a single step in History and Undo. Its first click no longer nudges the value to
  where the pointer landed, and a single click on the thumb leaves the value alone.

### Slider drags on large raws
- Dragging a slider no longer re-renders the edited photo's filmstrip thumbnail on every frame; it follows once you
  let go. On a 64 MP raw the first of those renders decoded the whole file again, which held up the loupe's
  previews for up to a second.
- A thumbnail of the photo open in Detail is made from its already decoded preview instead of decoding the file
  again.

### Dehaze while zoomed in
- With Dehaze on, panning a photo zoomed to 1:1 or moving a slider while zoomed no longer samples the whole
  original again to find the haze colour. On a 64 MP raw (Sony A7R VI) that took ~0.2 s of CPU per pan and per
  slider step; a slider step now takes ~5 ms on the GPU.

### Thumbnails of raw files appear sooner
- Grid thumbnails and the first picture in the loupe of an unedited raw now come from the smallest JPEG the camera
  stored that is big enough, not always from the largest one. On a Sony A7R VI file a 512 px thumbnail decodes
  0.3 MB of JPEG instead of 5.6 MB and is ready several times sooner; Nikon and Panasonic raws with a mid-sized
  JPEG get faster too.

## 2026-10-12

### Canon CR2 highlights
- Blown highlights in CR2 files from the Canon EOS 50D, 60D, 450D (Rebel XSi), 500D (Rebel T1i), 550D (Rebel T2i),
  1200D (Rebel T5) and 100D (Rebel SL1) now count as clipped (issue #1272). They used to stop at 81–98% of white, so
  highlight recovery missed them and, darkened, they took a colour cast instead of going neutral grey.

## 2026-10-12

### Pentax raws without black borders
- Raws from the Pentax KP, KF, K-3 Mark III (and Monochrome), K-70, K-1, K-1 Mark II, K-S1, K-S2 and 645Z no longer
  show a black (or, on the 645Z, white) band along one or more edges (issue #1271). The sensor's masked border is
  left out, so these photos come out a little smaller, closer to the camera's own JPEG. Crops and masks on photos
  edited earlier may shift by a few pixels.

## 2026-10-12

### Camera-look deep shadows

- The starting look fitted from a raw's camera JPEG now uses its darkest tones too (issue #1307).
  Photos with deep shadows no longer have to extend the first measured tone in a straight line to black.
  Existing smart previews are refreshed when their originals are online, and cached renders are redrawn.

### Slider drags follow the pointer more closely
- With a GPU, a slider drag's preview is drawn in the same frame as the slider moves instead of one frame later,
  when the preview renders in a few milliseconds (it waits at most 40 % of a frame for it; slower renders, such as
  on the CPU, are not waited for).

## 2026-10-12

### Kelvin white balance for more Sony cameras
- Sony ILCA-99M2, ILCA-77M2, SLT-A33, DSC-RX10, ILCE-3500 and ILCE-7S raws now show white balance in Kelvin too,
  read from the white-balance presets the camera stores in every ARW: a photo shot at 3400 K on the camera reads
  3396 K, one on the Shade preset 7493 K. As for #730, only the numbers change: edits made on the old relative
  scale keep their look, and As Shot shows the real temperature once the file is opened.

## 2026-10-12

### Bold text in other scripts
- Arabic names and other Arabic text in headings, chosen tabs and other bold places are now drawn bold, as Latin text
  beside them is; they used to stay at regular weight.

## 2026-10-12

### Older raw cameras
- Canon PowerShot G9 raws open with the camera's white balance (White Balance ▸ As Shot); it used to start from a
  neutral default.

## 2026-10-12

### Older raw cameras
- Olympus E-10 raws open with the camera's white balance (White Balance ▸ As Shot) and without the grey haze in
  the shadows: the black level recorded in the file is now subtracted.

### Told when there is no GPU
- When GPU rendering is on but no graphics processor can be found, LightCraft now says so once at start-up instead
  of quietly rendering everything on the CPU, much more slowly. Help ▸ System Info says why. On NixOS this happens
  when a `cargo` build runs outside `nix develop` (no Vulkan loader); start it from there.

## 2026-10-12

### Leaf and Hasselblad raws
- The Hasselblad Lunar's raws (a Sony NEX-7 inside) now develop from their sensor data with every editing tool,
  instead of opening as their embedded preview. Photos imported earlier switch over with Photo ▸ Reload from Disk.
- An uncompressed Leaf Aptus 22 MOS no longer opens as a dark, colourless picture of its sensor mosaic: it imports
  as a raw and shows its embedded preview (preview only, with the reason), as the compressed MOS files do.

## 2026-10-12

### Zoomed 64 MP raws stay decoded
- Panning a photo zoomed to 1:1, or moving a slider while zoomed, no longer decodes the whole raw again each time on
  high-resolution cameras such as the Sony A7R VI (64 MP). Their decoded original (~800 MB) is bigger than the
  default cache share (768 MB), so it was evicted as soon as it was stored. The original of the photo shown in Detail
  now stays, like its preview, until another photo or view is shown.
- Zooming in with Before/After side by side decodes the original once: the Before side used to decode its own copy
  beside the After's, and a window asked for again while the first was still decoding (a pan, the layout settling)
  started yet another decode.
- A zoomed view keeps the GPU copy of the original while it is on screen: pausing for a few seconds between pans no
  longer frees it, which cost a re-upload of about a second on the next pan (800 MB for a 64 MP raw). It is freed
  once you zoom out or move on.
- The kept original no longer pushes everything else out of the cache: on a 64 MP raw it filled the cache share
  alone, so the photo's preview and thumbnail sources and its neighbours' were decoded from the raw again on every
  edit at 1:1, every zoom out and every step to the next photo.

## 2026-10-11

### Keywords panel
- The titles **Keywords** and **Keyword List** in the Keywords panel fold their sections shut and open again, so
  the list of every keyword is one click away however many keywords the selected photos have. What is folded is
  remembered.
- Settings ▸ Interface ▸ Keywords of the selection ▸ **Grouped by parent** lays the selected photos' keywords out by
  the keyword they are inside: "Places › Italy" once, on a line of its own, with Rome, Venice and the others side by
  side under it. A long list stops after eight lines with "+N more", and the suggestions say only the keyword's own
  name. Whole names, one chip per keyword, stays the default.
- A selection without keywords now says "No keywords yet" where its chips would be.
- Typing in **Add keyword** offers the library's matching keywords right under the box. ↓ and Return (or a click)
  give the offered keyword and keep the cursor in the box for the next one; Return alone still adds what you typed.
  A name the library doesn't have is marked as a new keyword before you press Return, so a misspelling no longer
  becomes a keyword unnoticed. The suggestions further down stay put while you type.
- With several photos selected, the Keywords panel says how many photos its keywords belong to. The × on a keyword
  is a larger target that lights up on its own, and taking a keyword off several photos at once now says which
  keyword left how many photos, and that Edit ▸ Undo puts it back.

### HDR and panorama merges of CR2 keep their colour
- Photo Merge of Canon CR2 (and other raws whose colour LightCraft fits from the camera's own JPEG, such as NEF and
  ARW) gives the merged DNG the colour each photo has when opened alone (issue #965). An HDR merge of CR2s used to come
  out green and dark.

### "Rendering…" on the photo
- The loupe says "Rendering…" at the bottom of the photo when what it shows isn't the finished photo yet: the
  camera's embedded JPEG while a raw decodes, the previous render while an edit renders, or the magnified frame
  while the 1:1 detail is on its way. It appears once that has lasted half a second, so quick renders never show it.
- If LightCraft can't open a photo (its original is on a drive that isn't connected, or the file is damaged) but has
  its embedded JPEG or a thumbnail to show, the loupe now says so at the bottom of the photo ("Original offline · no
  smart preview" or "This photo can't be opened", the reason on hover) instead of showing that preview as if it were
  the photo.

### `--import <folder>` takes HEIC
- `lightcraft-cli run --import <folder>` (and the MCP tools that take folders) now picks up HEIC, HEIF and Nikon NRW
  files, the same files as `library.import` (issue #1080). They used to be skipped without a word.
- Compare and Reference View panes say "Rendering…" (or that the photo can't be opened) the same way, so a
  magnified preview isn't taken for the photo when checking focus at 1:1.

### Lens corrections
- Sony A7R VI (ILCE-7RM6) raws are corrected for lens distortion from the table the camera records in each file,
  under Optics ▸ Enable Lens Corrections, on by default for newly imported photos. On 33 photos shot with the camera's
  correction on (24–70 mm, 100 mm macro, 200–600 mm) the render lines up with the camera's JPEG within 0.68 px at
  1440 px at the corners, where it was up to 17 px off before. The camera's lateral chromatic aberration table is read
  too, but not applied by default: our demosaic already hides most of that colour fringing, and the table on top made
  it slightly worse. The vignetting table is read but not applied: the camera's own JPEGs don't apply it either. This
  covers lossless, Compressed and Compressed HQ (ARW 6.0) ARWs alike. The A7R IVA (ILCE-7RM4A) has its own distortion
  correction, checked against Sony's exports; other Sony bodies record the same tables, but they stay uncorrected until
  they are checked the same way. Photos imported earlier get the correction with Photo ▸ Reload from Disk (when still
  at their import settings). See `docs/sony-ilce7rm6-lens-corrections.md`.

### The thumbnail cache is no longer taken for your photos
- LightCraft's cached thumbnails no longer show up in Windows Photos / Gallery or other photo apps as pictures of
  yours (issues #610, #1066). They were ordinary `.jpg` files in the library's `thumbs` folder, which is usually
  inside Pictures, so every thumbnail (one for each photo, size and state of its edits) was listed as a photo.
  They are now kept under a name that is no picture's. The ones already there are taken over, not rendered again:
  the first time a library opens after this update, "Tidying the thumbnail cache" shows in the activity stack
  while they are renamed (it can be stopped, and goes on at the next open).
- Opening a library no longer rewrites the date of every thumbnail it shows, which made them all look new at each
  launch. Reading a thumbnail writes nothing (but for one that went unused for a week, whose date is brought
  forward); what was read is remembered for the session, so a full cache still lets go of what is not in use first.
- A `thumbs` folder that holds nothing but the cache says that it is one (`CACHEDIR.TAG`), so backup programs that
  go by that can leave it out. It is safe to delete while LightCraft is closed, as before. A `thumbs` folder that
  also holds files of yours (a library opened on a photo folder that had one) is not tagged, and your files in it
  are left as they are. Note that files there named exactly like the cache's own (`ab/ab…` with 32 hex digits and
  `.jpg`) have always been taken for the cache's, and are renamed with it.
- If you go back to an older version, it renders its thumbnails again as `.jpg`. This version then takes those
  over one by one as they are used; the rest stay until the cache makes room.

### Nikon raws without coloured edge bars
- Raws from older Nikon bodies no longer show a thin coloured or dark bar along one edge (the right edge, or the top
  of a portrait photo): masked sensor columns were being developed as part of the picture (issues #976, #914). The
  D7000, D5100, D800, D800E, Df, D4, D3, D3S, D700, D300, D300S, D600, D610, D810, D4S, D5600, Coolpix A, D3000, D40X,
  D60, D80 and D2Xs now show the area the camera itself frames, a few pixels smaller than before.

### JPEG XL export
- Export as JPEG XL (issue #247): Format ▸ JPEG XL in the Export dialog, `format: "jxl"` for agents and the CLI
  (`render in.raw -o out.jxl`). Quality as for JPEG, or Lossless; Effort (Fast / Normal) trades time for a
  smaller file; Progressive files show a coarse version first. 8- or 16-bit, sRGB, Display P3 or Rec. 2020 (lossless
  files also Adobe RGB and ProPhoto), metadata as for JPEG. With HDR output, photos edited in HDR are written as
  16-bit Rec. 2020 PQ. Not in the web version yet.
- Lossy JPEG XL files in Display P3 or Rec. 2020 now open in their real colours; saturated colours used to come in
  noticeably duller.

### Export next to the originals
- The Export dialog has Export to: Specific folder (as before) or Same folder as original photo, like Lightroom
  Classic (issue #956). With the second, each photo is exported into the folder its original is in, plus the
  Subfolder if you give one (say `Exports`), so a batch from several folders lands next to each set of originals.
  "If file exists" works per folder as usual, and an original is never written over.
- A photo that has no folder on disk (a demo photo, or one in the web version's browser storage) or whose folder
  is missing (moved, drive unplugged) is not exported; it gets its own error naming it.
- The choice is saved in export presets and Export with Previous. Agents pass
  `destinationMode: "sameAsOriginal"` (or `"specificFolder"`) to `app.export`; `dir` is then not needed.

### Open Library knows a Lightroom catalog
- Choosing a Lightroom Classic catalog folder in File → Open Library… (or one of its `Previews.lrdata` /
  `.lrcat-data` folders, or the `.lrcat`) no longer makes an empty LightCraft library inside it (issue #891). The
  library open now stays, and LightCraft says "This is a Lightroom catalog, not a LightCraft library" and offers
  **Import Lightroom Catalog…** with that catalog, or Cancel. `lightcraft-cli --library` refuses those folders too.

### Canon EOS R5 Mark II
- Canon EOS R5 Mark II C-RAW files that opened as "preview only" (more than half of them, depending on the
  scene) now develop from their sensor data with every editing tool, like the camera's other raws. Photos
  imported earlier switch over with Photo ▸ Reload from Disk.

### Switching libraries
- File ▸ New Library… asks for a name, then where to put it, and opens the new library (issue #902).
- File ▸ Open Recent Library lists the last 8 libraries; ones that are gone (moved, drive disconnected) are greyed
  out. Clear Recent Libraries empties the list.
- The open library's name shows under My Photos at the top of the left panel and in the window title.
- `lightcraft-cli --library` given a library's own file (`catalog.snap`) opens its folder instead of failing with
  "File exists".
- Agents use `app.newLibrary {path, name}`, `app.recentLibraries` and `app.clearRecentLibraries`.

### Edit in PhotoCraft
- Right-click a photo ▸ Edit In ▸ Edit in PhotoCraft (or Photo ▸ Edit In, ⇧⌘E) renders a copy with your edits, stacks it
  with the original and opens it in PhotoCraft. LightCraft finds PhotoCraft where its installer put it (Windows,
  macOS, Linux packages, AppImage, Flatpak), and says so if it isn't installed, before making a copy.
- The copy is a 16-bit PSD by default, which PhotoCraft saves straight back to with File ▸ Save; when you come back to
  LightCraft the copy shows what you saved.
- Settings ▸ General ▸ External Editing: the format (PSD or TIFF), colour space and bit depth for PhotoCraft and for
  the other editor (Edit in Other Editor, now ⌥⌘E), and where PhotoCraft is if you use a portable copy.
- Export can write PSD too (flat, 8- or 16-bit).

### Faster GPU exports
- Exports with noise reduction, Texture, Clarity, Dehaze or Highlights and Shadows can be much faster on the GPU,
  depending on the graphics card. On a Windows laptop with an AMD Radeon 780M, a full-size export of a 24 MP raw
  with those edits went from about 3.7 s to 1.9 s. Photos look the same as before.

### Reliability
- Upgrading from a Windows installer older than 0.6 (such as 0.5.0) keeps LightCraft in the folder it was installed
  in, instead of moving it back to Program Files (issue #399). If that folder has been deleted since, the upgrade
  installs to Program Files.
- A photo that runs into a bug in LightCraft while it renders (in a raw decoder or the editing pipeline) now fails
  like a photo that can't be opened, with the reason, instead of rendering forever; its thumbnail stops waiting too.
  Each such bug used to take one of the render threads with it, so after a few of them no photo rendered until
  LightCraft was restarted. The bug is still written to the log (Help ▸ Open Log Folder) for a report.

### Library sorting
- Photos that tie on the sort key (equal ratings, one import batch, a burst shot within the same second) now sort by
  file name, then id, instead of import order. Recently Added keeps a batch in file name order, and Auto-Stack orders a
  burst by file name, so the first file is the cover.

### More of the rendering on the GPU
- LUT profiles, masks with Refine Edges and soft proofing render on the GPU (they used to send the whole render to the
  CPU), and so do spot removal, defringe and the histogram (they used to bounce the image through the CPU mid-render).
  Results match the CPU within 1 level in 8 bits.
- Grid, filmstrip, preset and profile thumbnails render on the GPU once it is ready (about a quarter faster on an RTX
  5070, and the CPU stays free for decoding raws).
- While a slider is dragged, the loupe is drawn straight from the GPU: rendering and drawing share the window's
  graphics device, so drafts are never copied to memory and back (`LIGHTCRAFT_GPU_WINDOW=0` turns this off).

### Large preset collections
- A library holding thousands of presets (a Lightroom collection imported whole: 11 000 presets, a 42 MB
  presets.json) stays responsive. Every command used to re-serialize the whole list to see whether it had changed
  (~0.6 s per slider tick); now the list is saved only after it changes, and a slider tick takes ~2 ms.
- The Presets panel builds only the rows in view (and their thumbnails) instead of every preset on every frame; the
  preset pickers in Import and Settings ▸ Import scroll the same way.
- Importing a large collection and opening its library no longer compare every preset with every other.

### Slideshow export: PDF, JPEG and video
- View ▸ Export Slideshow… (or the Export buttons at the end of Settings ▸ Slideshow) writes the slideshow out, like
  Lightroom Classic's Export PDF, Export JPEG and Export Video: the photos in view as their slides, with the layout,
  overlays and the intro and ending screens, in view order.
- **PDF:** one page per slide at 1024, 1920 or 3840 pixels on the long side, at a JPEG quality you choose. Each page
  carries the slide duration and fade, so a PDF viewer's presentation mode plays it as a slideshow, and "Automatically
  show full screen" asks the viewer to open it that way.
- **JPEG:** numbered files (`Holiday-001.jpg`, `Holiday-002.jpg`…) in a new folder.
- **Video:** AV1 video in an MP4 (or WebM) file at 720p or 1080p, 24 or 30 frames per second: each slide stays up for
  its duration and the fades are rendered frame by frame (crossfades, or fades through the fade colour). The video has
  no sound yet. It is AV1 rather than Lightroom's H.264, which has no open encoder we can ship; current browsers,
  VLC and recent phones and TVs play it.
- The export runs in the background with its row in the activity stack; ✕ stops it and leaves no half-written file
  behind. A photo that can't be read is skipped and named in the message at the end. Agents use `slideshow.export`
  (`slideshow.exportStatus` for the result).

### Slideshow templates
- Settings ▸ Slideshow now opens with a template browser, like Lightroom Classic's Template Browser. A template holds
  everything about how a slideshow looks and plays: the playback timing and fades, the layout, options and backdrop,
  the overlays and the intro and ending screens. Click one to apply it; point at one to see it on the selected photo in
  the preview beside it without changing anything.
- Five built-in templates of our own: Plain Black (the photo on black), Gallery Wall (a light wall, a white mat and a
  soft shadow, with the caption and rating under the photo), Edge to Edge (photos cropped to fill the screen), Cinema
  Wide (16:9 with black bars, the title in the corner, fades through black and a closing card) and Shooting Data (the
  exposure, ISO, focal length, camera and lens under each photo on a dark backdrop).
- Save the current settings as a template of your own; right-click it to update it with the current settings, rename,
  export or delete it, and import templates others share with you. Your templates are `.json` files in the settings
  folder's `slideshow-templates` folder, shared by all your libraries.
- Applying a template can't be undone with Edit ▸ Undo (slideshow settings are library preferences), so the settings it
  replaced are listed as "Previous settings" until you quit: click that to go back.
- A template file that can't be used (damaged, from a newer LightCraft, too large) is listed as skipped instead of
  stopping the others, and a template whose identity plate image or background photo isn't there still applies, with a
  note saying what is missing. Agents use `slideshow.templates.*`, and `slideshow.preview {template}` renders a template
  without applying it.

### Slideshow intro and ending screens
- A slideshow can now open with an intro screen and close with an ending screen, like Lightroom Classic's Titles
  panel. Each is switched on on its own and has a background colour and, centred on it, your identity plate (the one
  set under Overlays: its text or image) or a line of text of its own, in the plate's colour or one you pick, at a
  scale. Each stays up for its own duration (by default the slide duration) and fades in and out like a slide.
- The intro plays once before the first photo; the ending after the last photo when Repeat is off (while repeating
  the show runs until you stop it, so the ending doesn't come up). Random order never moves them, ←/→ step across them,
  and the control bar says Intro or End while one is up (the photo count only counts photos).
- Set them in Settings ▸ Slideshow ▸ Titles; the preview there switches between the photo's slide, the intro and the
  ending. They are saved with the library. Agents use `slideshow.titles`, and `slideshow.preview {title: "intro"}`
  renders a screen as a PNG.

### Slideshow overlays
- Slides can now carry Lightroom Classic's Slideshow overlays: an identity plate (a line of text in a colour, or an
  image file such as a PNG logo with transparency, at a size and opacity, optionally drawn behind the photo), the
  photo's rating as stars, and any number of text overlays (up to 16) built from tokens: `{caption}`, `{title}`,
  `{file}`, `{date}`, `{exposure}` ("1/250 s at f/8"), `{iso}`, `{focal}`, `{camera}`, `{lens}`, `{rating}` and the
  other file-naming tokens, with a size, colour, opacity and an optional soft shadow (opacity, offset, radius, angle).
  Each sits at one of nine anchors of the slide or of the photo, moved by an offset, and never leaves the slide.
- The export watermark can be drawn on every slide's photo too ("Use export watermark" takes the one you last
  exported with).
- Set them in Settings ▸ Slideshow ▸ Overlays; the slide preview there and View ▸ Slideshow show them. Sizes are a
  share of the slide, so slides look the same on any screen. They are saved with the library. Agents use
  `slideshow.overlays`; `slideshow.preview` now also returns where each overlay landed.

### Slideshow layout, options and backdrop
- Slides can now be laid out like Lightroom Classic's Slideshow Layout, Options and Backdrop panels: margins (all four
  linked, or each on its own), Zoom to Fill Frame (crop the photo to fill its cell), a stroke border (colour, width),
  a cast shadow (opacity, offset, radius, angle), and a backdrop colour with an optional colour wash (a gradient from a
  second colour, at an angle and opacity) and a background image (a photo from the library, at an opacity).
- Set them in Settings ▸ Slideshow, which now shows the selected photo as a slide as you change them (with the margin
  guides, at the screen's aspect or 16:9 / 4:3). Sizes are a share of the slide, so a slide looks the same on any
  screen. They are saved with the library.
- View ▸ Slideshow plays the composed slides, prepared ahead off the UI thread like before, so fades and the playback
  controls work as they did. With the default layout (no margins, border, shadow or backdrop) it plays the photos as
  before. Agents use `slideshow.layout`, and `slideshow.preview` returns one composed slide as a PNG.

### Slideshow playback
- View ▸ Slideshow (⌥⌘↩) now plays with Lightroom Classic's Playback settings: how long each slide stays up, a fade
  between slides (a crossfade, or through black, white or any colour; 0 cuts), random order (every photo once per
  pass, a new order each time round), repeat (off: the slideshow ends after the last photo) and preparing previews in
  advance (the next two slides render while the current one is up, so a fade never waits or flashes).
- Change them in Settings ▸ Slideshow (View ▸ Slideshow Options…); they are saved with the library. Agents use
  `slideshow.playback`; `view.slideshow` takes any of them for one run (`interval` still sets the slide duration).
- While it plays, moving the pointer shows a small control bar: previous, play / pause, next, the slide number and
  end. Space pauses, ← / → step (a cut), Esc ends. A slide's time starts once it is shown sharp.

## 2026-10-10

### Dialogs fit the window
- A dialog taller than the window no longer runs off it (issue #781): on a 1366×768 screen the Export dialog's title
  and its Cancel / OK buttons were off screen, so it couldn't be confirmed. Now its options scroll and the title and
  buttons stay visible. Long lists in dialogs (What's New, All Metadata, Keyboard Shortcuts) get shorter to fit.
  Dialogs that fit look as before.

### Previews match exports on Mac screens
- On macOS, photos are shown through the profile of the screen the window is on while no monitor profile is chosen in
  Settings ▸ Display (issue #787). On P3 screens previews used to look more saturated than the exported files in
  Preview or Photos. Moving the window to another screen is followed; **Use Screen Profile** goes back to it after
  choosing a profile file.

### Sidebar sections
- Arrange the left sidebar your way: drag Albums, Local, By Date, Folders or Keywords by its header to move it (a line
  shows where it will go), or right-click a header for Move Up / Move Down.
- Hide the sections you don't use: View ▸ Sidebar Sections, or right-click a header ▸ Hide This Section. Right-click
  the My Photos title to bring one back, or choose Reset Sidebar Sections. The arrangement is remembered.
- In a long section, the header stays at the top of the sidebar while you scroll through its rows, so you always see
  which section you are in and can fold it from there.
- Sections are easier to tell apart: a hairline between them, the fold chevron in the same column as the rows'
  triangles, a highlight under the pointer. A folded section shows how many rows it holds, counts of 0 are fainter,
  and a section with nothing in it says so instead of disappearing.
- Recently Deleted is listed with All Photos, Recently Added and Picks under My Photos.
- Keywords has a + in its header, like Albums: Create Keyword Tag without opening the Keyword List.
- Agents: `view.sidebarSection`, `view.sidebarMoveSection`, `view.sidebarReset`; `ui.set` takes `sidebar`.

### Kelvin white balance for raws whose camera has colour matrices
- Raws from the 52 camera models whose colour matrices LightCraft has measured (Sony ILCE-7M3, ILCE-7M4, ILCE-7RM3,
  Nikon D850, Canon EOS 5D Mark III and others) now show white balance in Kelvin, as a DNG does and as Lightroom
  does: As Shot reads the file's white (an ILCE-7M3 sample: 4268 K / +7), the Temp slider runs 2000–50000 K and the
  Daylight, Cloudy, Shade, Tungsten, Fluorescent and Flash presets set real temperatures (issue #730). The matrices
  only convert between the camera's multipliers and Kelvin; the colour look is what it was.
- Nothing already edited changes: a Custom white balance set on the old relative scale renders pixel for pixel as
  before, and becomes the Kelvin value with the same look the next time it is edited. A photo imported earlier shows
  its real As Shot temperature once the file is opened (or Reload reads it).
- Lightroom's `crs:Temperature` for these raws is read as the Kelvin it is (sidecars, Lightroom catalogs, presets);
  raws of other cameras keep the as-shot shift of issue #510. The two Kelvin scales come from different matrices and
  differ by about 7–10 % on the ILCE-7M3; which one is off is still open.

### Fujifilm lens corrections
- Fujifilm RAFs from 59 camera models now get the lens correction the camera stores in the file: distortion (with the
  camera's own zoom to fill the frame) and vignetting, as with Panasonic and Sony files. The Optics profile controls
  adjust or switch them off, and DNG export keeps them. Lateral chromatic aberration data is not applied: our
  demosaic already lines the colour planes up. Details and measurements: `docs/fuji-lens-corrections.md`.

### Appearance
- Settings ▸ Interface ▸ Appearance Mode: Sync with System, Dark Mode or Light Mode, above a light
  and a dark theme card, each previewing its theme (dark: Charcoal, the look so far, or Midnight;
  light: Silver or Paper). New and existing installs stay dark; following the system is opt-in.
- The top bar's appearance button cycles Auto, Light, Dark (a monitor, sun or moon), and View ▸
  Appearance lists the modes and themes. On Linux, Auto follows the desktop portal's colour scheme
  as it changes, without polling.

### Add to Apple Photos (macOS)
- The Export dialog has **Add to Apple Photos**, with an optional album. The files are exported to your folder as
  usual and then added to Photos, into a top-level album of that name (made when it's missing). The toast says how
  many Photos added, or why it added none. Export with Previous and export presets repeat it.
- The first time, macOS asks whether LightCraft may control Photos. Photos may ask about duplicates; files it skips
  show as a partial import.
- Agents: `app.export {addToPhotos, photosAlbum}`, `export.addToPhotos {paths}` for files already on disk, and
  `export.photosImports`. See [Add to Apple Photos](apple-photos.md).

### Activity stack
- Long-running tasks show in one place, top left under the top bar, as in Lightroom Classic (issue #345): imports and
  folder scans, exports (and the contact sheet PDF), Synchronize Folder, Build / Smart Previews, Lightroom catalog import, Photo Merge, model downloads, the face scan,
  AI Denoise and Find Missing Photos. Each row has the task's name, a progress bar, the count and what it is working
  on; ✕ stops the tasks that can stop. Three rows show, then "+N more". The separate export panel and the progress
  windows and toasts are gone.
- Quitting while a task that can be stopped is running asks first ("Quit Anyway" stops it).
- Agents list and stop tasks with `activity.list` and `activity.cancel`; `ui.inspect` reports them as `activity`.

### Importing a deleted photo again
- Importing a file whose photo is in Recently Deleted brings the photo back, with its edits, rating and albums
  (issue #706; before, the import skipped it as "already in the library" while the grid showed it deleted). The
  import review checks such files, marks them "Will be restored" and counts them as "in Recently Deleted"; the
  choice next to that (Restore them / Import as new / Leave them) now starts on Restore. Files dropped on the window
  restore the photo too and say "Restored 1 photo from Recently Deleted". A deleted photo whose file was moved is
  relinked to the file it is imported from. Browsing a folder in Local, Auto Import and a Lightroom catalog import
  never restore anything. Agents: `library.import` → `onDeleted` defaults to `restore` (`skip` keeps the old
  behaviour, `fresh` imports a clean copy and deletes the old record).

### Formats
- File extensions match whatever their letter case: `IMG_0001.JPG` and `DSC_0002.NEF` as Nikon writes them, `.Jpg`
  after a rename, an `.XMP` sidecar. Adding photos or a folder, Browse Folder (Local), drag-and-drop, the CLI and the
  sidecar lookup always did; the Add Photos dialog on Linux did not until 0.5.0 (GTK and the desktop portal match
  `*.jpg` case-sensitively, so `.JPG` and `.NEF` files were hidden — issues #342 and #705).

### Reliability
- Closing a library releases its lock explicitly (issue #680). On Linux and macOS the lock used to go with the file
  handle, and a program being started by another thread at that moment held a copy of the handle until it was fully
  launched, so reopening the library straight away could be refused with "This library is already open in another
  LightCraft program" although nothing had it open. Nothing in the shipped app is known to hit the window today; it
  made a lock test flaky, and the fix closes the window for good.

## 2026-10-09

### HDR PNG export
- With **HDR output** on, a photo edited in HDR can now be exported as a **16-bit PNG** with the PQ curve in Rec. 2020,
  like the HDR AVIF: the same brightness, with SDR white at 203 cd/m², and content light levels recorded.
- The colour is described by the PNG `cICP` chunk, not an ICC profile. HDR-aware viewers (current Chromium-based
  browsers, for example) show it in HDR; **viewers that don't read `cICP` show the PQ values as if they were sRGB**,
  so the picture looks flat and washed out there. That's how PQ PNG works, not a broken file: for a picture that
  looks right everywhere, export a JPEG with HDR output on (an SDR image plus a gain map).

### Smart album rules
- The rule editor's field menu is grouped: Rating, Pick Flag, Color Label and Any Searchable Text at the top, then
  submenus for Source, File, Date, Keywords & People, Description, Camera Info, Location, Size, Develop and Assisted
  Culling.
- New fields: Keyword Count ("2 or more", "exactly 2"…), People and People Count (named faces), Shutter Speed (written
  as 1/250), File Extension, Video Duration, Copy Name, In a Stack, Alt Text, City, State / Province, Country, Long
  Edge, Short Edge, Aspect Ratio (landscape / portrait / square), Cropped and Treatment (in color / black & white).
  Size rules measure the photo as shown: Megapixels now counts the cropped photo, like Long Edge, Short Edge and
  Aspect Ratio (a 24 MP photo cropped to a square is 16 MP).
- Choice values read as words in every language: "Public Domain", "Picked", "Black & White" rather than ids like
  `publicDomain` (rules and agents keep using the ids; `album.ruleFields` adds `choiceLabels`).
- Rule values are checked: a rating of 9, a date like 2026-13 or "banana", a "between" whose dates are the wrong
  way round, "in the last 0 days", a colour label that doesn't exist, "contains" with nothing to look for, an
  album that is gone, or an empty group is reported with its rule ("rule 2.1: no rating 0–5 is 9")
  instead of quietly matching nothing or everything. New date rules start at the current year.
- A smart album can include or exclude another smart album: "Keywords contain travel" and "Album isn't Excluded Photos"
  leaves out whatever Excluded Photos matches, as its rules change. The album picker shows your albums as in the sidebar
  (folders open on click) or finds them as you type part of a name or folder (↑ / ↓ and Enter to pick); albums that
  can't be picked, such as one that would make an album include itself, are greyed with the reason. Esc with a dropdown or calendar open in a dialog
  closes just that.
- Date rules have a calendar button: pick a day, a month or a whole year (Year / Month / Day), in your language; the
  two dates of a "between" can't be picked out of order; Esc closes just the calendar. It picks dates, not times (a
  time is typed, and picking a day replaces one). Typing still works, and "2026-10-01 10:00" (a space for the
  T) now matches.
- Rule problems are shown in your language and name the field the way the menu does ("Title: needs something to look
  for").
- A smart album whose rules no longer check (an album they test was deleted) is marked ⚠ in the sidebar; its tooltip
  says what to fix, and `albums.list` reports `problems`.
- Editing a smart album's name and rules is one step: both or neither, and one undo takes both back
  (`album.setRules` takes `name`).
- The rule editor marks each rule that needs fixing right under it and keeps OK disabled until they are fixed. An
  Album rule is "is" or "isn't" an album, picked with the album picker.
- Yes/no fields (Has Edits, Cropped, Has GPS…) read Yes / No instead of true / false. A rule sent as `"false"` now
  means no (it used to count as yes), and a value that is neither yes nor no is refused.
- Any Searchable Text also finds the state / province, alt text, people and a virtual copy's name.

### Keyword lists
- File ▸ Import Keywords… and Export Keywords… read and write keyword list files as Lightroom Classic does (a keyword
  a line, a tab per level, synonyms in braces, keywords left out of export in brackets). Capture One, darktable, Adobe
  Bridge and Photo Supreme (its Formatted Vocabulary File) use the same files, so keyword lists move between them and
  LightCraft. Importing adds the keywords you don't have, in one undo step; a file it can't read says which line, or
  to save it as UTF-8. Exporting warns about keywords Capture One won't import, and names any keyword the format can't
  hold (such as a name in brackets), which it leaves out so the file always reads back.

### Keyword sets
- Keyword Set ▸ Edit Set… edits a set's nine keywords in place, one field per ⌥ key, and renames it (or saves the edits as a new
  set). An empty field leaves its key empty instead of moving the keywords after it; editing Recent Keywords and naming
  them saves a new set.

### Keywording box
- The chips at the top of the Keywords panel are the keywords of every selected photo, not only the active one's; a
  keyword only some of them have is marked with an asterisk (hover: how many). Only the × removes a keyword, from
  every selected photo; a chip's menu acts on the selection (add to all, remove, show photos, edit) and no longer
  deletes a keyword from the whole library.
- Switch to & Containing to see the keywords with the keywords containing them, or to Will Export to see what
  exported files will carry for the selection.
- The keyword set's buttons and the suggestions follow the selection too; the typing boxes get the Cut / Copy / Paste
  menu, and Return in the painter's box starts painting.

### Keyword List
- The Keywords panel ends with a Keyword List, as in Lightroom Classic: every keyword with its photo count, those no
  photo has yet too. Filter it, tick a keyword to give it to the selected photos (a dash: only some have it), or click
  the arrow to see its photos.
- Create keywords with + (inside the one you picked, with synonyms, and given to the selection if you like), edit one
  by double-clicking it, delete one with − after a confirmation. Right-click for more: create inside, Put New Keywords
  Inside This Keyword, Purge Unused Keywords.
- Drag a keyword onto another to nest it, or onto the list's title to bring it back to the top level; photos dragged
  from the grid onto a keyword get it.
- Right-click ▸ Put New Keywords Inside This Keyword: new keywords, made with + or typed in the box above, go inside
  it. The list is there with no photo selected too, to set up keywords first.
- Keyword tag options decide what exported files carry: Include on Export, Export Containing Keywords, Export
  Synonyms. Exports now write keyword names to `dc:subject` and their paths to `lr:hierarchicalSubject`, and keywords
  imported from files (or read from sidecars) keep the hierarchy `lr:hierarchicalSubject` gives them.

### Text fields
- Right-click Search Photos, a slider's typed value, the Info panel's fields, the name field of New Album, Rename
  Album, New Smart Album, Rename Keyword, Merge Keywords and similar dialogs, or the name field under unnamed faces for
  Cut, Copy, Paste and Select All. Esc in them gives back the text from before you started typing (in the Info panel it
  no longer saves it). Return in those dialogs now confirms them, and their name opens selected. More fields will
  follow.
- On Windows and Linux, Ctrl+C outside a text field copies edit settings and Ctrl+V pastes them (⇧Ctrl+V: Paste
  Selected Settings). Ctrl+V only reaches LightCraft while the system clipboard holds text; otherwise use Edit ▸ Paste Edit
  Settings.

### Dialogs
- System Info, All Metadata and What's New end with one Close button, like About, instead of Cancel and OK
  (issue #782).

### Synchronize Folder
- Right-click a folder (or a disk other than the startup disk) in the sidebar's Folders section ▸ Synchronize
  Folder…. The folder and the folders inside it are scanned in the background for photos added on disk, photos whose
  file is gone, photos whose file was renamed or moved within the folder, and XMP sidecars changed by other apps; then
  choose what to do: import the new photos and relink the moved ones (the defaults), move the missing ones to Recently
  Deleted, read the metadata updates. A folder on a disk that isn't connected is reported as such. Synchronizing
  runs in the background with a row in the activity stack (✕ stops it); everything it does is one undo step, and no file on disk
  is touched. Agents use `folder.scanChanges` and `folder.synchronize`.

### Folder colour labels
- Right-click a folder (or a disk other than the startup disk) in the sidebar's Folders section ▸ Set Color Label ▸ a colour or None. The row
  shows the label's dot before its photo count. Agents use `folder.label` (`path`, `label`); `library.folders`
  reports each row's `label`.
- A label follows its folder when it is renamed or moved in LightCraft, together with the labels of the folders
  inside it; undo and redo carry them back and forth.
- The catalog format is now version 4 (`Catalog.folder_records`, `Op::SetFolderRecord`). Once this version has
  opened a library, older versions refuse it, as with every format change.

### Monitor profiles
- Settings ▸ Display ▸ Choose Profile… shows photos through your monitor's ICC profile (matrix/TRC or LUT-based), so
  colours are right on wide-gamut and calibrated displays instead of oversaturated. The Detail view renders straight
  into the display's own gamut; thumbnails and other previews are converted to it. Histograms, the preview caches
  and exports don't change. Agents use `app.displayProfile`. See [display profiles](display-profiles.md).

### Highlights and Shadows (process 2)
- Highlights and Shadows keep the fine detail of the tones they move (issue #632): pulling Highlights down on a bright
  sky no longer flattens the twigs, lines and texture in it, and lifting Shadows keeps the texture of dark areas, much
  as Exposure would; the edges between bright and dark regions stay as sharp as before. This is rendering process 2:
  new photos (and Reset) get it, photos you already edited keep their look until you choose Photo ▸ Update to Current
  Process. See [process versions](process-versions.md).

### Masking
- Drag anywhere inside the selected radial gradient's ellipse to move it, including rotated
  ellipses and radial components in compound masks. Edge and rotation handles keep their existing
  functions, and one Undo restores the entire drag.

### Crop rotation
- Outside the crop box, where dragging rotates the photo, the pointer is a curved double arrow, and the angle is shown
  next to it while you rotate (issue #534). For an exact angle, type it in the new Angle field under Straighten and press Return
  (Esc keeps the old angle; a decimal comma works too), or click the Straighten value; hovering any slider's value now says it can be typed.

## 2026-10-08

### Albums tree
- Album folders fold with the same disclosure triangle as Local, By Date, Folders and Keywords.
- Right-click a folder ▸ New ▸ Create Album… / Create Smart Album… / Create Smart Album from Filter… / Create Folder…
  makes it inside that folder (`dialog.newAlbum`, `dialog.newFolder`, `dialog.smartAlbum`, `dialog.newSmartAlbum` take `parent`).
- Drag an album or folder onto a folder to move it there (a closed folder opens after 0.6 s under the pointer; a
  "Albums" header takes it back to the top level; Esc cancels; holding a dragged album or photos near the top or bottom edge of the sidebar scrolls it). Drop on the top or bottom half of another album,
  or the edge of another folder, to place it before or after it: that folder is then ordered by hand. Right-click ▸
  Sort Contents A–Z (or ＋ ▸ Sort Albums A–Z at the top level) goes back to by name. To put a folder after an open
  folder, fold that folder first (the bottom of an open folder means "inside"). Agents use `album.reorder`
  (`id`, `parent?`, `before?`) and `album.sort`.
- The catalog format is now version 3 (`Album.order`, `Op::SetAlbumOrder`). Once this version has opened a library,
  older versions refuse it, as with every format change.

### AI RAW denoise
- Detail has a per-photo AI Denoise switch and Amount. Bayer RAW photos use a disposable cache, with matching previews and exports and pure-Rust CPU/GPU inference.
- Models are installed separately after accepting their terms; no weights are bundled. See [setup and limitations](denoise.md).

### Library keyboard culling
- Colour labels tint thumbnail surrounds in Square Grid and the Detail filmstrip, and a translucent footer along the bottom of the photo in Photo Grid. Label confirmations use a matching pale colour.
- Setting or clearing a colour label shows a brief bottom confirmation, like rating a photo; custom label names appear in the message too.
- On macOS, ratings `0–5`, labels `6–9` and pick/unflag `P/U` now reach the app even when shown in the native menu (issue #283; adapted from PR #261).
- `Shift+6–9` labels and advances. `Shift+P` picks and advances in Photo Grid and Square Grid; it opens Presets in other views. With Auto Advance on, Shift still moves only once.
- Help → Keyboard Shortcuts includes the number-key bindings. See [library shortcuts](library-shortcuts.md).

### Keyboard shortcuts
- Shortcuts are editable: Help ▸ Keyboard Shortcuts (⌘/) lists every command with a search box; click a shortcut and
  press the new keys (Esc cancels), × removes it, ↺ restores the original, Reset All undoes every change. A key that
  belonged to another command moves to the new one. Menus show the new keys; agents use `app.setShortcut`.
- Return presses OK in Choose Edit Settings to Copy (⇧⌘C) and Paste Selected Settings (⇧⌘V), and Esc cancels them
  (issue #735).

### Formats
- HEIC / HEIF photos (iPhone and Mac) open now: the optional `lightcraft-heif` crate (heic-rs, pure Rust) behind
  codecs' `heif` feature — 8- and 10-bit, alpha, grid tiles, the container's rotation/mirror/crop, ICC, EXIF and XMP.
  Off by default (HEVC patents are the distributor's call, same as PhotoCraft); official builds pass `--features heif`.
- HEIC colours now match libheif to within one code value (they were up to ~10 off): the HEVC stream's own
  full-range/matrix signalling is honoured (iPhone photos are full range), chroma is upsampled like libheif, grids
  take their tiles' ICC profile, and `imir` mirrors the way libheif writes it. Importing a HEIC reads its size from
  the container instead of decoding it; small previews use the file's embedded thumbnail. The release packages
  (macOS, Windows, Linux, FreeBSD, Nix) now really are built with HEIC support; a build without it reports
  "HEIC/HEIF support isn't included in this build" for each `.heic`/`.heif` at import.

### Editing
- The Tint slider works the right way round (issues #188, #321): left adds green, right adds magenta, as its track
  shows and as in Lightroom, and Tint values in Lightroom XMP sidecars now render as they do there. A custom Tint
  saved in an earlier version now shifts the other way; set it again (or re-run Auto / the white-balance picker).
- White balance on DNGs (and other raws with a colour matrix) re-develops the photo for the new white through the
  camera's own colour matrices, as Lightroom does, instead of shifting the colours of the as-shot rendering: a grey
  lit by the chosen white comes out grey and saturated colours move as the camera records them. The eyedropper and
  Auto use the same model. Custom white balances on these photos render slightly differently than before.
- Crop (issue #295): a Lock toggle keeps the aspect ratio on every handle, Custom takes your own ratio (Apply), and
  dragging a handle into the image edge stops there instead of pushing the crop out of shape.

### Library and views
- Trackpads: pinch to zoom around the pointer and scroll with two fingers to pan the photo; panning keeps the photo
  inside the view. A plain mouse wheel over a zoomed photo pans it too.
- A Folders section in the sidebar lists the folders your photos were imported from; choose one to see its photos.
- Select All and multi-selection show every selected photo in the grid and filmstrip, not only the active one
  (issues #187, #298). Importing files that are in Recently Deleted asks whether to leave them there, restore them
  (with their edits) or import them as new; the trash view's Photo menu has Empty Recently Deleted.
- The Import Photos review opens bigger and can be resized; its photo grid fills it (issue #337). Shift-click checks
  or unchecks a range of photos (issue #338).
- When a folder holds several file types, the Import Photos review has a toggle per type (`ARW · 120`, `JPG · 120`):
  import only the raws and leave the JPEGs beside them (issue #344).

### Languages
- The interface is available in Spanish (issue #371), German and Russian (Edit ▸ Language), alongside English,
  Chinese (Simplified and Traditional), Japanese and Brazilian Portuguese.

### Editing
- Type an exact value into any slider (issue #322): click the number next to its name, type (`1.5`, `-20`, `5600`)
  and press Return; Esc keeps the old value.

### Editing
- The eye on the Light, Color and Detail section headers now hides their adjustments, as it already did for Effects,
  Optics, Geometry and Calibration (issue #316).

### Library
- Choosing a date under By Date or a keyword under Keywords shows those photos from All Photos, as their counts
  promise, instead of filtering whatever album or folder was open, which often showed nothing (issue #341).
)
)

## 2026-10-07

### RAW decoding
- Olympus ORF raws get the starting look fitted to the camera's own JPEG too, instead of opening flat and grey (the
  E-1, E-400 and XZ-2 corpus files: all accepted), with white balance relative to the as-shot look as for the other
  formats. On an E-1 photo the starting render moved from ΔE00 11.3 to 8.3 against Lightroom's. Photos already
  imported pick it up when re-rendered.
- Samsung SRW files without compression now open as raws: NX5, NX10, NX11, NX20, NX200, NX210, NX1000, NX1100, EX1
  and WB2000. The compressed ones (NX1, NX30, NX300, NX500, NX2000, NX3000, NX3300, NX mini) still open from their
  camera JPEG. Photos already imported pick the change up on Reload.
- Raws from 52 camera models (22 Canon, 11 Nikon, 11 Sony, 3 Fujifilm and 5 others, plus their other names) whose
  camera JPEG can't be fitted no longer open with the neutral fallback: they start from colour matrices fitted to the
  camera's measured spectral sensitivities, from the Academy Software Foundation's rawtoaces-data. A camera profile
  and the fit to the photo's own JPEG still come first, so photos that had a fitted look keep it. See
  [camera preview colour](camera-preview-colour.md#spectral-camera-matrices-and-the-order-of-precedence).
- Canon CR2 raws no longer open too dark with blocked-up shadows: the black level was measured over border columns
  that light already reaches (left of the image area Canon records), 4 % of the range too high on the EOS 6D. It is
  now measured on the masked columns alone (8 corpus bodies, EOS 40D to 5DS R). On a 6D photo the starting render
  moved from ΔE00 7.6 to 5.2 against Lightroom's.
- Canon CR2 and Pentax PEF raws get the same starting look fitted to the camera's own JPEG as ARW, NEF, RW2, RAF
  and CR3, instead of opening flat and desaturated (issue #310). Photos already imported pick it up when re-rendered.
- JPEG XL compressed DNGs (DNG 1.7) now open: lossless tiles decode sample for sample (checked on synthetic files);
  lossy tiles decode too, keeping the raw values above 1.0 instead of clipping them (checked on one real file). A tile that cannot be decoded makes the file open from its embedded
  preview, with the reason, instead of showing a black tile. A JPEG XL preview stored in the DNG is used like an
  embedded JPEG.
- Apple ProRAW's gain table map (its local tone mapping, `ProfileGainTableMap`) is now read and kept when a photo is
  exported or converted to DNG. It is not applied by default: Lightroom Classic renders ProRAW without it. To see a
  ProRAW the way the iPhone renders it, turn on Profile ▸ Camera local tone mapping (shown for photos that carry the
  map); the option is per photo, so presets and Copy Settings carry it.
- iPhone ProRAW and other DNGs that carry their own segmentation mattes (DNG semantic masks) use them for the Select
  Sky, Subject and Background masks instead of our heuristics, so the sky is selected where the camera found it
  (checked on one CC0 iPhone 12 Pro ProRAW). Photos without mattes are unchanged.
- Panasonic and Leica raws (RW2, RWL) are now corrected for lens distortion the way the camera corrects its own JPEG
  (issue #256): the correction the camera records in the file is applied under Lens Corrections ▸ Enable Profile
  Corrections, on by default for newly imported photos, with the same framing as the camera's JPEG. At 12 mm the
  12–32 mm kit zoom was off by about 5 % of the image width at the corners before. Files shot with the correction off
  are unchanged; photos imported before this change get it when imported again.
- Canon CR3 raws now develop from their sensor data: lossless RAW and C-RAW, checked sample for sample on the EOS
  M50, R100, R8 and RP (on the RP also in 1.6× crop mode). CR3 files the decoder can't read yet still open from their embedded JPEG, as before.
- Canon EOS R7 C-RAW files develop from their sensor data too, instead of opening from their embedded JPEG. For R7
  C-RAWs imported earlier, Photo ▸ Reload from Disk picks up the raw data.
- Canon CRW, Minolta MRW, Sigma X3F, Kodak KDC, Leaf MOS and Epson ERF files that LightCraft can't decode yet
  now import as "preview only" with their embedded JPEG instead of failing. A raw whose data is damaged but whose
  preview is intact does the same.
- Raw files whose raw data sits in a private block of a TIFF (Phase One / Leaf IIQ, Canon EOS-1D / 1Ds and Kodak DCS
  TIFFs) are no longer opened as a thumbnail-sized ordinary image. They are recognised as raws LightCraft can't decode
  yet and import as "preview only", with the reason.
- Sony ILCE-7M4 downsized lossless ARWs now decode subsampled YCbCr tiles into linear RGB,
  preserving RAW editing & full-resolution export instead of using embedded JPEG previews.
- Sony A7R II (and other) raws whose camera JPEG is lens-corrected no longer open grey and too dark (issue #232): the
  starting look is fitted to the camera JPEG away from edges when the misaligned edges spoil the fit on all pixels.

### Lightroom Classic catalogs
- File → Import Lightroom Catalog… opens `.lrcat` directly, with originals referenced in place.
  Ratings, flags, labels, keywords, collections/sets, virtual copies and supported edits migrate;
  existing LightCraft edits are preserved by default. Source settings/history are archived, unsupported
  fields are reported, and the original Lightroom database stays read-only. Rendering is approximate.

## 2026-10-05

### Reliability
- The Windows installer asks where to install LightCraft (Program Files by default; upgrades keep the folder you
  chose) and ends on a page saying it was installed, with a "Launch LightCraft" box. It used to finish without a word,
  so a successful install looked like nothing had happened (issues #18, #399). Silent installs (`/qn`, `/passive`)
  show no dialogs and take `INSTALLFOLDER=...`.
- If the desktop app can't open its window (for example when no graphics device can be used), it now says so in a
  message box that names the log file, instead of quitting without a trace (issue #260).
- On Linux and FreeBSD, file dialogs (Import Photos…, Open Library, Export, presets, keyword lists…) are children of
  the LightCraft window. The xdg-desktop-portal picker used to open as a separate top-level window on Wayland
  ("Failed to associate portal window with parent window" in the portal's log): tiling compositors put it beside the
  app, and with LightCraft fullscreen it was hidden altogether, so Import Photos… looked like a hang until the
  dialog was found and closed.
- On macOS, single-key shortcuts that appear in the menu bar now work: E, C, H, M, ⇧P, I, K, D, ratings 0–5,
  labels 6–9, P / U and the rest did nothing, because macOS only passes ⌘ / ⌃ combinations and function keys
  to the menu bar and the app ignored those keys, assuming the menu bar would handle them. Keys outside the menus
  (Space, G, X, ⌫) and ⌘ shortcuts were not affected.
- `--memory` sessions keep their promise to save nothing (issues #164, #169): UI changes made in one no longer
  land in `ui.json` (where they replaced the saved settings), and the GPU crash sentinel no longer creates the
  settings folder there. The same goes for the temporary session offered when the library can't be opened.
- The desktop app keeps a log file: `logs/lightcraft.log` in its settings folder (Linux `~/.config/lightcraft/logs/`),
  with the logs of the two previous runs beside it, so warnings and crashes of a run started from a desktop menu or the
  Dock can be attached to a bug report. `LIGHTCRAFT_LOG` works as before; `RUST_LOG` takes env_logger-style
  directives. See README → Quick start → Logs.
- Help → Open Log Folder shows that log file in the file manager (Finder, Explorer, or the folder on Linux), so it
  can be attached to a report without hunting for the settings folder (issue #260).
- `lightcraft-cli` logs warnings on stderr too (issue #168); `LIGHTCRAFT_LOG` or `RUST_LOG` picks another level.
- LightCraft no longer crashes at launch on Windows PCs whose Vulkan driver is broken (issue #136, e.g. some Intel UHD
  630 drivers): on Windows the window and GPU rendering use DirectX 12 only and never load the Vulkan driver unless
  asked to. `LIGHTCRAFT_GPU_BACKEND=dx12 | vulkan | metal | off` (or wgpu's `WGPU_BACKEND`, which GPU rendering
  ignored before) chooses the graphics backend; `off` renders on the CPU. The GPU now starts after the window
  is up, and only when Settings ▸ Performance ▸ Use the GPU for rendering is on; if LightCraft ever dies while
  starting the GPU, the next launch starts with GPU rendering off and says how to turn it back on.
- The Windows app no longer quits at launch with "Parent device is lost" when another program's `dxcompiler.dll` is
  on the DLL search path without its `dxil.dll` (issue #471): DirectX 12 shaders now always compile with the
  compiler built into Windows (FXC). `WGPU_DX12_COMPILER=dxc` uses a `dxcompiler.dll` instead.
- Exports and renders never write over a photo's original (issue #93): exporting into the photo's own folder with
  the same name and "Overwrite" (or Export with Previous repeating it), an exact output path from the control
  channel or MCP, a merge preview path or `lightcraft-cli render IMG.jpg -o IMG.jpg` is refused with a clear
  message, and the original is left byte for byte. Ordinary earlier exports are still overwritten when asked.
  Exported files are written to a temp file and then renamed into place, so a full disk or an unplugged drive
  never leaves a truncated file; the XMP sidecar of an "Original" export follows the "If file exists" choice too.
- Convert to DNG, Copy as DNG, Photo Merge and smart previews no longer write straight to the final file (issue
  #106): a DNG is checked against the raw data, written to a temp file, synced and read back before it gets its
  name (never replacing a file), and only then is the photo relinked or the raw copy removed — a failed write
  leaves no DNG and keeps the raw. Smart previews are written the same way; a damaged one (cut short by a crash or
  a full drive) no longer counts as built and Build Smart Previews replaces it.
- Import ▸ Copy verifies every copy, like Move (issue #96): each file is written as a new file, synced to disk and
  checked against the content read from the card. A copy that fails or differs is removed and reported as a failed
  import — so "import complete" means the copies are good before you format the card — and a name that is taken
  gets -1, -2… instead of being replaced.
- Faster exports and card imports, with the same protection for what can't be recreated (issue #134): exports,
  renders and screenshots are still written to a temp file and renamed into place, but no longer forced to disk
  one by one — they can always be exported again, and on a USB drive or a NAS that per-file sync dominated a large
  export. The catalog, XMP sidecars, settings, DNGs, merges, smart previews and the copies an import makes are still
  synced. Import ▸ Copy now checks each copy against the content hash taken while scanning the card instead of
  reading the card a third time (Move still compares byte for byte before it deletes a source); a file that changed
  on the card after Review Import is reported instead of being imported with stale details. Checking an export path
  against the library's originals no longer scans the whole library when nothing is at that path.
- Saving metadata to an XMP sidecar another application wrote no longer replaces it (issue #92): LightCraft merges its
  fields in and keeps the rest — e.g. that application's develop settings and edit history — byte for byte. A
  sidecar that isn't valid XMP is copied to `<name>.xmp.bak-<time>` first. With the default stem naming, a raw and a
  JPEG with the same name (`IMG_0001.CR3` + `IMG_0001.JPG`) no longer share one sidecar: the raw keeps `IMG_0001.xmp`,
  the JPEG uses `IMG_0001.JPG.xmp`.
- A save that fails part-way (a full disk, a network share that drops) no longer looks like a damaged catalog
  afterwards (issue #101): the partial write is cut off before LightCraft retries, so the next launch replays every
  change. Catalogs already holding such a fragment load in full. Quitting while the catalog log can't be written
  still saves your queued changes in the closing snapshot.
- The catalog has a format version (issue #102). Opening a library from an older LightCraft upgrades it; a library
  written by a newer LightCraft is refused with "this library was written by a newer version of LightCraft" and left
  untouched — older versions no longer read part of it as a damaged log. Once this version has opened a library,
  LightCraft 0.2.0 and older refuse it ("unsupported format … v2").
- The control port (`--control`) closes a connection as soon as it receives anything that isn't a JSON request
  (issue #94): an HTTP request from a web page can no longer carry a command in its body. Lines are capped at
  4 MiB and connections at 16.
- A sleeping NAS, a dropped network share or a USB drive spinning up no longer freezes the window (issue #104):
  whether originals are there is checked on a worker thread (grid thumbnails, the Info panel, the photo menu, Missing
  Photos); importing (also by drag and drop), Find Missing Photos, Build / Discard Smart Previews, the Rename preview,
  the Local folder tree and Auto Import read the disk on worker threads too. The import progress window has Cancel.
- Browser version (experimental), keeping a library safe (issue #107): File ▸ Back Up Library… downloads the catalog
  and every imported photo as one zip, and File ▸ Restore Library from Backup… brings it back (the current library
  is kept). A failed save (storage full) now shows the unsaved warning and is retried, a photo that can't be stored
  isn't added, a second tab shows a message instead of overwriting the first, `?reset` asks first, and the page says
  when the browser may evict the library. Hosting: the sample cache headers no longer mark the (unhashed) files
  immutable, and HOSTING.md describes the actual build.
- Canon CR2 photos from the EOS 7D, 50D, 60D, 550D, 600D, 1200D, 1300D, 5D Mark II and 1D Mark IV (and other
  models whose sensor starts on a green-blue row) no longer come out magenta (issue #85): the colour-filter
  layout is read from each file instead of assumed.
- Exports are never black because of the GPU (issue #78): a GPU render that runs out of device
  memory, exceeds the GPU's buffer limits, hits a driver error or reset, or comes back
  incomplete is redone on the CPU — the file is the same image either way. Work is sent to the GPU
  in short pieces so slow integrated GPUs aren't reset by their watchdog. `ui.inspect` → `perf`
  (`gpuReason`, `gpuFallback`), Help ▸ System Info and Settings ▸ Performance say why the GPU isn't
  used (e.g. a skipped software adapter such as llvmpipe) and why the last render fell back.
- The thumbnail cache only ever counts and deletes its own files (issue #98): a library opened on a folder that
  already has a `thumbs/` folder of other pictures no longer loses them when the cache is trimmed or cleared.
- Settings files are never quietly reset (issue #103): a damaged `prefs.json`, `presets.json` or `view.json` is kept
  as `<name>.corrupt-<time>` and you're told; one that can't be read (e.g. locked by another program) is left alone
  for the session instead of being overwritten with defaults. The app settings (`ui.json`, which remembers your
  library) are written atomically and saved as soon as you open another library, not only at quit. Quitting while
  changes couldn't be saved tries once more, then asks: Try Saving Again, Quit Anyway or Cancel.

## 2026-10-02

### Presets and profiles
- Import presets from other editors: XMP presets, classic `.lrtemplate` files, "DNG presets" from mobile apps and `.zip`
  bundles of any of these — whole folders at once, grouped by pack. Masks inside presets come along.
- Luminar looks: `.lmp` files and `.mplumpack` collections import as presets (grouped by collection); the sliders
  with a counterpart here come along, the rest is listed.
- 23 new built-in presets: Portrait, Landscape, Urban, Food, Seasons, Vintage and B&W toners.
- Importing XMP presets no longer lists bookkeeping fields (`Cluster`, `SortName`, `SupportsAmount2`, the as-shot
  white, empty Point Color slots…) as settings that couldn't be carried over.
- A preset whose lens-profile switch is off no longer turns off the lens corrections built into a DNG (iPhone ProRAW
  and other files with embedded distortion / vignetting corrections), matching what the preset does elsewhere.
- XMP presets apply their red / green / blue curves only when they also have the master curve and all three channel
  curves, as Lightroom does: a preset with just one channel curve (or no master curve) leaves the channels alone.
- Imported `.cube` LUT profiles appear in the Profile menu and the profile browser, grouped by their folder, and stay
  favourites across restarts (issue #328).

### Library
- Photos in Recently Deleted can be restored from the app: right-click ▸ Restore (or Delete Permanently), also in the
  Photo menu. The filmstrip has the photo context menu too. Adding a file again that is in Recently Deleted no
  longer just says "duplicate skipped": it opens the side panel on Recently Deleted with the photo selected and says
  how to restore it or delete it permanently and import it afresh. (For a fresh start on a photo, Reset Edits,
  Cmd+Shift+R, keeps the photo and clears its edits.)
- Canon CR3 files show their full-size embedded JPEG (e.g. 6960 × 4640 on an EOS R6 Mark III) instead of the
  1620 × 1080 preview, and import with their metadata: capture time, camera, lens, exposure, GPS and XMP. Their raw
  data is not decoded yet, so they stay preview-only. For CR3s imported earlier, Photo ▸ Reload from Disk (now in
  the Photo menu and the photo context menu) picks up the full-size preview and fills in the camera metadata they
  were missing, without touching anything already set.
- Rename Photos never overwrites another photo when only the letter case changes (issue #95): on case-sensitive
  volumes (Linux, case-sensitive APFS) `img_1.JPG` next to `IMG_1.JPG` is a different photo and the renamed one gets
  `img_1-1.JPG`; on case-insensitive volumes the case change still goes through.
- Rename Photos reports files it could not move back after a failure (issue #105), e.g. when a network share drops
  mid-batch: the error lists them (old → new) and the library points at their new names (an undoable partial rename),
  so none shows as missing. Renaming one of a raw + JPEG pair copies their shared `IMG_0001.xmp` instead of taking
  it away from the other (issue #92). Find Missing Photos also finds renamed files by their content, prefers a content match
  over a same-name same-size look-alike, and skips (and reports) photos it can't tell apart instead of guessing.
- Rename Folder… and Move Folder To… (Local) can be undone and redone (issue #97): the folder moves back on disk with
  its photos and XMP sidecars, and the photos point at it again. If something now occupies the old place, the undo
  is refused and nothing is overwritten.
- Smart albums with a rule editor: match all / any / none, nested groups, 26 fields.
- Quick Collection and target album (B in the grid), keyword sets (⌥1–⌥9), colour-label sets.
- Colour-label filter with several labels at once; expandable folder tree in Local.
- Import: copy to any folder, by day / by month / one folder, rename on import, metadata preset, Copy as DNG.
- Import ▸ Move: photos go into the destination (with the same folders and renaming as Copy, e.g.
  `Photos/2026/20260114/20260114_001.jpg`) together with their XMP sidecars; each original leaves the card only after its
  copy is verified and in the library. Duplicates and files that fail stay where they were.
- Watched-folder auto import; Convert to DNG; Duplicate; Build Standard / 1:1 / Smart Previews.
- Export file names use the Rename Photos tokens ({title}, {seq:2}, {date:%Y-%m-%d}…), plus new {num}, {folder}, {lens}, {iso}, {rating}, {creator}.
- Copyright status, rights usage terms and copyright info URL in Info, metadata presets and exports.
- Auto-Tag from Tracklog: GPS locations for your photos from a GPX track log, matched by capture time.
- A change that can't be saved to disk (full or unplugged drive) is no longer silent: the command reports
  "saved in memory but not written to disk", the top bar shows a warning, and LightCraft keeps retrying until the
  save goes through.
- Smaller, faster catalogs: photos you only looked at in Local (never added, rated or edited) are forgotten once
  their folder has not been browsed for 30 days — your files and sidecars stay, and browsing the folder shows them
  again. Change the period (or turn it off) in Settings → Performance.
- A library is open in one program at a time (issue #99): opening a library that LightCraft or `lightcraft-cli` already
  has open — on this computer or another one sharing the folder — says who has it ("already open in LightCraft (process
  123 on studio-mac)") instead of letting both write and silently drop each other's edits. A crash never leaves the
  library locked: the lock is the operating system's and goes away with the program.
- If your library can't be opened at launch (open in another program, unreadable, on a drive that isn't connected,
  written by a newer LightCraft), LightCraft says so and why, and offers Try Again, Choose Another Library…, Continue
  Without Saving and Quit (issue #100). It no longer quietly starts a demo session that looked like a reset library and
  lost everything at quit; a temporary session shows a banner the whole time and never writes to your library.

### Editing
- Optional remote SAM 3: keep the native editor local and run Object, Describe, and detail
  inference on a Mac through SSH. Saved masks still render and export offline.
  See [remote Metal inference](ai-masks.md#remote-metal-inference).
- AI masks with SAM 3 (Object and Describe in the Masking panel): click an object to select it (⌥-click leaves a
  part out), or type what to select ("sky", "the red car", "car, road"); both combine with other masks, have an
  Edge setting, and get a sharper zoomed-in pass in the background. The model runs inside LightCraft in pure Rust
  and never freezes the window. It is optional and not part of LightCraft (Meta's SAM License, about 3.4 GB): the
  first time you use an AI mask, LightCraft asks before downloading it, shows the progress, can cancel and resume,
  and checks the file before using it. Masks keep their selection, so they render and export without the model.
- Auto Sync: edits apply to every selected photo. Auto B&W mix. Automatic versions.
- Colour-range masks: click the photo to sample. Luminance ranges: range bar, smoothness, luminance map.
- ⌘-drag to straighten, ⇧G Guided Upright, a grid while transforming.
- Nikon NEFs start from a colour and tone look fitted to the camera's own JPEG, as Sony ARWs do, instead of a muted,
  greenish neutral rendering (issue #150); white balance is adjusted relative to the as-shot look. 12-bit NEFs
  (e.g. D750, D780, D850, D7500, Z 50) no longer render nearly black or with crushed shadows: their black level was
  read in the wrong units.

### Viewing and sharing
- Slideshow, second window, All Metadata, System Info.
- Edit in External Editor (⇧⌘E): a 16-bit TIFF copy, stacked, refreshed when you come back.
- Lossy DNG files and Smart Previews open as raw photos.
- Compressed Nikon NEFs (lossless and lossy compressed, 12- and 14-bit — e.g. D3200, D5100, D7000, D750, D850, Z 50)
  now develop from the raw data instead of the camera's embedded JPEG, so a B&W or other picture style set in the
  camera no longer gets baked in. Photos already imported as "preview only" switch over on Reload. (Files that
  Nikon splits into two differently compressed halves still use the preview for now.)
- Panasonic and Leica raws (RW2, RWL and the older RAW files, DMC-LX1 to DC-S1R II) develop from the raw data in
  every format the cameras write. Most bodies opened as preview only before: GH1–GH5, the G, GX, GF, GM, FZ, LX and
  TZ/ZS series, Leica D-Lux, V-Lux and C-Lux, and the GH6, GH7, G9 II, S5 II and S9 generation. They start from a
  colour and tone look fitted to the camera's own JPEG, as Nikon and Sony raws do, framed in the aspect ratio set in
  the camera; mapped-out sensor defects are filled in. `.rwl` and `.raw` files are imported too. Photos already
  imported as "preview only" switch over on Reload.
- Sony ARWs from before about 2017 (RX100, RX100 II–V, RX10, NEX, SLT, ILCE-6000, A7 / A7 II / A7R II and their
  siblings) no longer open bright green (issue #148): their as-shot white balance and black level are read from the
  file (Sony stores them only in scrambled maker-note data on these bodies), the few columns of padding at the right
  edge are cropped away, and "12-bit uncompressed" files are no longer clipped. The RX100 series renders much closer
  to the camera's own JPEG.
