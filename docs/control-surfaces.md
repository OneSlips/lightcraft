# Control surfaces

> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-14 · **Change:** major (created: the control-surface engine ported from TactileBridge, the surface protocol, Settings ▸ Controls, the Logitech MX Creative Console) · **Target:** Adobe Lightroom Classic 15.5 / Lightroom 9.6

Hardware dials, keys and buttons that drive LightCraft: the **Logitech MX Creative Console** (Keypad + Dialpad) first,
MIDI controllers and Tangent panels later. LightCraft owns the whole engine — layouts, what each control does, dial
speed and acceleration, pages that follow the tool you use, one undo step per dial turn, the on-screen readout — and a
thin plugin on the device's side only forwards ticks and presses and draws what LightCraft sends.

The engine is a port of **TactileBridge**, its author's own (never released) Capture One bridge for the MX Creative
Console, contributed under the MIT licence: Portions © RAD Films - Roman Alurkoff (from TactileBridge), MIT. Its
code, settings design, layouts, tuning values and hand-drawn icon pack came over; nothing specific to Capture One did
(no Capture One control lists, no reading of Capture One's files, no Tangent hub impersonation, no screen scraping for
the active tool, no simulated keystrokes): LightCraft knows its own state.

Code: [`crates/surface`](../crates/surface) (`lightcraft-surface`, L2, no UI), `crates/ui-egui/src/surface.rs` (the
app side), `crates/ui-egui/src/panels/controls.rs` (Settings ▸ Controls), `crates/ui-egui/src/panels/surface_osd.rs`
(the readout), `apps/lightcraft/src/control_server.rs` (the transport). The MX Creative Console plugin lives in its own
repository (`lightcraft-logi`, a Logi Actions SDK plugin in C#).

## Using it

1. **Settings ▸ Controls ▸ Let control surfaces drive LightCraft.** LightCraft then listens on `127.0.0.1:7981` (the
   **Port** field; loopback only) for surfaces. Off by default.
2. Install the surface's plugin (MX Creative Console: the LightCraft plugin for Logi Options+, see its README).
3. The plugin asks to pair: LightCraft shows "… asks to control LightCraft" and lists the request in Settings ▸
   Controls with a six-digit code; the keypad shows the same code. Check they match and click **Approve**. The plugin
   keeps the token it gets; **Forget** under *Paired* revokes it.
4. Pick a **profile** (Default, Wedding, Portrait, Fashion, Landscape, Real Estate, or your own).

What happens then:

- **Dials** move develop values (Exposure, Temp, a mask's sliders, the brush size, the zoom…). Ticks are gathered and
  applied once per *Apply movement every* (16 ms); turning fast accelerates on TactileBridge's curve. A turn is **one
  undo step**: it ends when the dial rests for *Turn ends after* (800 ms), when another dial starts, when a key runs a
  command, or when the photo changes.
- **Apply to Selected** (a key, or *Dials change every selected photo*) carries each turn to every selected photo,
  in that same single undo step (LightCraft's Auto Sync at the end of the gesture). This is TactileBridge's Auto Sync
  without the copy-and-paste round trip.
- **Keys** run commands (any engine or UI command, with parameters), switch pages or profiles, hold a modifier
  (*fine* ¼× / *big* 4× steps while held), reset the dial's value, or — bound to a value — put that value on the dial
  until pressed again (TactileBridge's adjustment dial toggle). A dial press resets its value.
- **Pages.** A profile's pages bind controls; keys and dials page separately (the Keypad and the Dialpad), and
  controls a page leaves free fall back to the profile's base bindings (the Dialpad's buttons). The MX Creative
  Keypad's own page buttons show key pages 1–3; the Dialpad's fourth button turns the dial pages in the built-in
  profiles.
- **Pages follow the app** (TactileBridge's tool switching): Crop puts the crop page on the keypad and Straighten on
  the dial; Masking shows mask tools, a selected mask its sliders, its brush the brush size and feather; Remove the
  spot size; a dialog OK / Cancel. Leaving the tool brings back the page you chose. Any page can be mapped to any
  context in Settings ▸ Controls ▸ *Pages that follow the app*.
- **The on-screen readout** (TactileBridge's OSD) shows the value being turned (name, value, a bar) or the command a
  key ran, at the top or bottom of the photo, with its opacity, colour and display time.
- **The surface** shows each key's icon (TactileBridge's hand-drawn pack), label and live value; a lit border marks
  the value on the dial, a held modifier and toggles that are on.

## Settings ▸ Controls

| Section | What it holds |
|---|---|
| Control Surfaces | on / off, port, connected surfaces, pairing requests (Approve / Deny), paired surfaces (Forget) |
| Profile | the profile in use, Duplicate / Rename / Delete, Add the built-in profiles, **Bring over my TactileBridge setup** (shown when `%LOCALAPPDATA%\TactileBridge\settings.json` exists), Import… (`settings.json`, `.lp5`, `.lcsurface`), Export… (`.lcsurface`), the last import's report |
| Layout | the page (or *Every page (base)*), its groups (Keys / Dials) and *Tool page*, Add / Rename / Delete page; the device (3 × 3 keys, dial, roller, four buttons): click a control, or **Learn** and touch it on the surface, then choose what it does (a value, a command with JSON parameters, a modifier, next / previous page, reset, apply to selected) |
| Pages That Follow the App | on / off; one page per context |
| Dial Feel | global speed, acceleration, fine and big step factors, invert; per value: speed, acceleration, slow / fast turning (floor / ceiling), fast / slow thresholds, invert; TactileBridge's presets for every value (Wedding, Real Estate, Fashion, Landscape) |
| Editing | apply to every selected photo, when a turn ends, how often movement is applied, steady readout while turning |
| On-Screen Readout | on / off, also for key presses, opacity, display time, position (top edge, top, bottom, bottom edge), background colour |
| Surface | show values on the device, haptics, key background and text colours |

Every action of the editor is also a command (`cmd:surface.profile`, `cmd:surface.page`, `cmd:surface.pageNext`,
`cmd:surface.pagePrevious`, `cmd:surface.applyToSelected`, `cmd:surface.followContext`, `cmd:surface.import`,
`cmd:surface.export`, `cmd:surface.settings` (read or replace the whole settings), `cmd:surface.state`,
`cmd:surface.input` (the input a surface sends)), plus the commands keys need: `cmd:surface.maskPrevious`,
`cmd:surface.maskNext`, `cmd:surface.dialogConfirm`, `cmd:surface.dialogCancel`.

## Settings file

`control-surfaces.json` in LightCraft's settings folder (next to `ui.json`; `LIGHTCRAFT_CONFIG_DIR` moves it). Written
atomically half a second after a change. A damaged file, or one from a newer LightCraft, is reported (Settings ▸
Controls and a notice), kept as it is, and LightCraft runs on the defaults until you change something. Every number is
clamped into range on reading.

```jsonc
{
  "version": 1,                 // format version
  "enabled": true,              // surfaces may connect (the listener runs)
  "port": 7981,
  "profile": "Wedding",         // the profile in use
  "followContext": true,        // pages follow the app
  "applyToSelected": false,     // turns go to every selected photo
  "motion": {"speed": 1.0, "acceleration": 1.0, "invert": false, "fine": 0.25, "coarse": 4.0},
  "tuning": {                   // per value; absent = defaults
    "wb.temp": {"speed": 0.3, "acceleration": 1.0, "accelerationMin": 1.0, "accelerationMax": 10.0,
                "fastMs": 20, "slowMs": 200, "invert": false}
  },
  "transport": {"flushMs": 16, "echoSuppression": true, "echoSuppressionMs": 100, "gestureEndMs": 800},
  "osd": {"enabled": true, "opacity": 0.9, "background": "#202020", "displayMs": 1800,
          "position": "bottom", "showCommands": true},
  "look": {"background": "#000000", "foreground": "#FFFFFF", "showValues": true, "haptics": true},
  "profiles": [{
    "name": "Wedding",
    "description": "…",
    "base": {"pad.1": {"kind": "command", "command": "develop.copy"},
             "pad.4": {"kind": "pageNext", "group": "dials"}},
    "pages": [
      {"name": "Cull", "groups": ["keys"], "bindings": {
         "key.1": {"kind": "command", "command": "library.previous"},
         "key.4": {"kind": "command", "command": "photo.rate", "params": {"rating": 1}}}},
      {"name": "Exposure/WB", "groups": ["dials"], "bindings": {
         "dial": {"kind": "value", "target": "light.exposure"}, "roller": {"kind": "value", "target": "wb.temp"}}},
      {"name": "Crop", "auto": true, "bindings": {"dial": {"kind": "value", "target": "crop.angle"}}}
    ],
    "contexts": {"default": ["Cull", "Exposure/WB"], "crop": ["Crop"]}
  }],
  "paired": [{"client": "lightcraft-logi", "name": "LightCraft plugin for Logi Options+",
              "tokenSha256": "…64 hex digits…", "approvedAt": 1792000000}]
}
```

- **Bindings** (`kind`): `value {target}`, `command {command, params?}`, `modifier {modifier: fine|coarse}`,
  `pageNext {group?: keys|dials}`, `pagePrevious {group?}`, `page {page}`, `profile {profile}`, `profileNext`, `reset`,
  `applyToSelected`, `followContext`.
- **Targets**: develop control ids (`develop.controls` lists them; indexed ones such as `pointColor.0.hueShift` too);
  `mask.<field>` for the selected mask (`exposure`, `contrast`, `highlights`, `shadows`, `whites`, `blacks`, `temp`,
  `tint`, `texture`, `clarity`, `dehaze`, `hue`, `saturation`, `sharpness`, `noise`, `moire`, `defringe`, `amount`);
  `ui.brushSize`, `ui.brushFeather`, `ui.brushFlow`, `ui.removeSize`, `ui.removeFeather`, `ui.removeOpacity`,
  `ui.zoom` (tool and view values, no undo step).
- **Pages**: `groups` empty = keys and dials; `auto: true` = a tool page, shown only by its context (paging by hand
  skips it).
- **Contexts**, most specific first; a context falls back to its parents, then `default`:

| Context | When |
|---|---|
| `dialog.<kind>` | a dialog is open (`dialog.export`, `dialog.import`, `dialog.settings`, …; the dialog's `kind` as `ui.inspect` shows it) |
| `library` | the photo grid, Survey or People (or no photo open) |
| `develop`, `develop.<section>` | Detail; the Edit panel's section opened last (`light`, `color`, `effects`, `detail`, `optics`, …) |
| `crop`, `remove`, `redeye` | those tools |
| `masking`, `masking.mask`, `masking.mask.brush`, `masking.mask.gradient` | Masking; a mask selected; its brush or gradient tool |

A **layout file** (`.lcsurface`) is one profile as JSON (the `profiles[]` element above).

## The surface protocol

Version **1**. JSON lines on a loopback TCP port, the same framing and rules as the
[control protocol](control-protocol.md): one request object per line, `{"id", "method", "params"}`, one reply per
line, `{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`. The control-surface port (Settings ▸ Controls,
`7981`) serves only `surface.*` methods; an app started with `--control PORT` serves them there too, beside everything
else. Between replies, a subscribed connection receives **events**: lines with an `event` field and no `id`. A line is
never split between a reply and an event.

**Pairing.** The control port has no authentication, so surfaces pair: only a paired connection may subscribe or send
input. `surface.pair` creates a request with a six-digit code; the user approves it in Settings ▸ Controls; polling
`surface.pair {request}` then returns a 64-hex-digit token, once. Every later connection presents it in
`surface.hello`. LightCraft stores only its SHA-256; re-pairing the same client replaces its token; Forget revokes it
and tells connected surfaces (`surface.unpaired`). Requests expire after 10 minutes; at most four wait at once.
In-process requests (headless scripts, tests) count as paired.

| Method | Params | Result | Paired? |
|---|---|---|---|
| `surface.hello` | `{protocol: 1, client, name?, token?}` | `{protocol, app, version, paired, enabled, devices: [{id, name, controls: [{id, kind, label}]}]}`; a valid token pairs this connection | no |
| `surface.pair` | `{client, name}` → new or open request; `{request}` → poll | `{status: "pending", request, code}` / `{status: "approved", token}` (once) / `{status: "denied"}` | no |
| `surface.icons` | `{names?: [..]}` (all when omitted) | `{name: svg}` | no |
| `surface.subscribe` | `{device?: "mx-creative-console"}` | `{layout}`; then `surface.layout` events whenever it changes | yes |
| `surface.unsubscribe` | — | — | yes |
| `surface.layout` | `{device?}` | the layout now | yes |
| `surface.input` | `{events: [{control, ticks}` \| `{control, press: true\|false, page?}]}` (at most 256) | `{effects}` | yes |

Errors: an unpaired connection gets *this surface isn't paired …*; switched off: *control surfaces are switched off*;
a newer client protocol: *this LightCraft speaks surface protocol 1*. Junk input is refused with an error, never
acted on: `ticks` must be a number (clamped to ±1000 per event), `page` 1–64, control ids are trimmed to 64 characters.

**Events**: `{"event": "surface.layout", "layout": {…}}` (a subscriber's device layout changed: a value, a page, the
context, a toggle — sent at most once per frame, and held back while the turned value hasn't caught up with the dial),
`{"event": "surface.unpaired"}` (the user forgot this surface; the connection should pair again).

**Layout** (`lightcraft_surface::layout::DeviceLayout`):

```jsonc
{
  "device": "mx-creative-console", "profile": "Default", "context": "crop",
  "keyPage": "Crop", "dialPage": "Crop",
  "keys":    [{"control": "key.1", "binding": {"kind": "command", "command": "crop.rotateAspect"},
               "label": "Crop Orient.", "icon": "switch-crop-orientation", "value": null, "active": false}, …],
  "keyPages": [{"name": "Page 1", "keys": [ … nine keys … ]}, … at least three …],
  "buttons": [ … pad.1–pad.4 … ],
  "dials":   [{"control": "dial", "binding": {"kind": "value", "target": "crop.angle"}, "label": "Straighten",
               "icon": "straighten-tool", "value": "+1.25", "position": 0.51}, …],
  "focus": null, "applyToSelected": false, "followContext": true,
  "background": "#000000", "foreground": "#FFFFFF", "showValues": true, "haptics": true
}
```

Labels and values come translated and formatted; `icon` names an SVG from `surface.icons`. `keyPages[n-1]` is what a
keypad with its own page buttons shows on its page *n* (the context's page on every one while a tool page is active);
such a keypad sends `page` with its presses.

**Devices and controls.** `mx-creative-console`: `key.1`–`key.9` (Keypad LCD keys, row by row), `dial`, `roller`,
`pad.1`–`pad.4` (Dialpad buttons). A dial press is `{control: "dial", press: true}`; it runs `dial.press` if a page
binds it, else resets the dial's value. Logi's device types: `Loupedeck70` = MX Creative Keypad, `Loupedeck71` = MX
Creative Dialpad, `Loupedeck72` = the Actions Ring (TactileBridge's `…70` / `…71` / `…72` profile variants).

## Coming next: MIDI and Tangent

The engine is device-neutral: controls are string ids, a device is a list of them with their kind (encoder, button,
LCD key) and page group. What each still needs:

- **MIDI**: an input backend that fits the pure-Rust rule (the common `midir` crate links ALSA's C library on Linux); a `midi` device whose controls are `midi.cc.<channel>.<n>` (relative encoders: two's-complement and
  binary-offset modes; absolute faders as values) and `midi.note.<channel>.<n>` (buttons); learn mode already reports
  any control id; feedback to motor faders / LED rings from the pushed layout (`position`).
- **Tangent** (Wave, Element, Ripple): written from Tangent's **public** developer documentation only (the TUBE /
  Hub API); a `tangent` device mapping panel controls to ids, the same layouts and protocol. TactileBridge's Tangent
  hub impersonation for Capture One is not part of LightCraft and is not used.
- Both reuse pairing (an in-process client counts as local), profiles, contexts, motion and the readout as they are.

## From TactileBridge

Settings ▸ Controls ▸ **Bring over my TactileBridge setup** (or `surface.import {path}` with its `settings.json`)
brings over the dial feel (global and per value), the readout, the key colours, haptics and LED switches, automatic
switching, Auto Sync and every tool context, as a new profile named *TactileBridge* (made active; your other profiles
stay). Import… takes TactileBridge's Logi profiles (`.lp5`) too. The report lists what came over and what was left
out, and why. Tuning is kept as it was; LightCraft's slider units differ from Capture One's in places (Contrast is
±100 per 1, Temp 50 K per tick at speed 1), so a value may want its speed retuned.

### Every TactileBridge feature and setting

✅ ported · 🔁 done natively by LightCraft · ✕ dropped (Capture One–specific or not needed)

| TactileBridge | LightCraft | |
|---|---|---|
| Dial acceleration curve (`GetAcceleration`: floor, ceiling, fast / slow thresholds, quadratic ramp) | `lightcraft_surface::motion` | ✅ |
| Global speed, acceleration, invert (`Controls.*`) | `motion.speed`, `motion.acceleration`, `motion.invert`; Settings ▸ Controls ▸ Dial Feel | ✅ |
| Per-adjustment tuning (`AdjustmentProfiles`: speed, acceleration, min, max, fast ms, slow ms, invert) | `tuning.<target>`; Dial Feel ▸ Value | ✅ (keys mapped to LightCraft targets) |
| Workflow presets (Wedding, Real Estate, Fashion, Landscape) | Dial Feel ▸ Presets for every value | ✅ (Landscape's 18× ceiling is 15×, as TactileBridge's own clamp made it) |
| Echo suppression (`EnableEchoSuppression`, `EchoSuppressionMs`) | `transport.echoSuppression(Ms)`: pushes held while the turned value catches up | ✅ |
| Flush interval (`Bridge.FlushIntervalMs`, defined but unused in TactileBridge) | `transport.flushMs`: ticks coalesced, applied once per interval | ✅ (implemented) |
| Idle reconcile, state retention, periodic value poll, coalescing diagnostics (`Bridge.*`) | — | 🔁 LightCraft's values are authoritative and pushed on change: nothing to poll or reconcile |
| On-screen display (`OsdOverlay`: pill, title, value, bar, icon; enabled, opacity, colour, display time, 4 positions, fade) | `panels::surface_osd` in the window; `osd.*` | ✅ (no icon in the readout; the keys show the icons) |
| Plugin colours (`PluginColors`) | `look.background`, `look.foreground`, pushed live | ✅ (no plugin redeploy needed) |
| Haptic feedback switch | `look.haptics`: the plugin raises Logi plugin events (key pressed, value reset, value at its limit) Options+ can map to haptics | ✅ (TactileBridge had the switch only) |
| LED value indicator | `look.showValues` | ✅ |
| Tool contexts (`ToolContext.Contexts`: 9 keys + up to 4 encoders per Capture One tool) | profile pages with `auto: true`, mapped to contexts | ✅ (Crop → `crop`; Draw / Erase Mask → `masking.mask.brush`; Draw Healing Mask → `remove`; Direct Color Editor → `develop.color`; Import / Export / Process → `dialog.import` / `dialog.export`) |
| Automatic tool switching (`ToolStateAutomation`: UI Automation + BitBlt scan for the orange tool highlight; dialog detection by window title) | contexts read from LightCraft's own state | 🔁 (`followContext`) |
| Adjustment dial toggle on a pad key (focus, lit rounded border) | a key bound to a value; the plugin draws the lit border | ✅ |
| Modifier keys (keystrokes with Ctrl / Shift sent to Capture One) | commands run directly; new *hold for fine / big steps* modifiers | 🔁 / ✅ |
| Auto Sync (debounced Copy + Apply after the dial rests; `IdleTimeoutMs`, `CopyApplyDelayMs`; the AutoSync key) | Apply to Selected: the gesture's change goes to every selected photo in its one undo step; `gestureEndMs` | 🔁 (copy-apply delay dropped) |
| Dial press resets the parameter | dial press resets its value (or runs `dial.press`) | ✅ |
| Display smoothing on the dial tile | — | ✕ the plugin shows LightCraft's value as pushed |
| Kelvin conversion (`KelvinConvert`) | LightCraft's own Kelvin scale | 🔁 |
| Curated profiles (Default, Wedding, Portrait, Fashion, Landscape, Real Estate; `…70` Keypad, `…71` Dialpad, `…72` Actions Ring) | built-in profiles (`lightcraft_surface::defaults`): key pages from `70`, dial pages and buttons from `71` (`72` repeats them) | ✅ (the Dialpad's fourth button turns the dial pages; its TactileBridge action is on a key page, except Fashion's *Layer on/off*, which the Brush and Mask tool pages have) |
| Default Options+ profiles shipped by the plugin | the LightCraft plugin ships one Keypad and one Dialpad profile of generic keys | ✅ (layouts live in LightCraft, so nothing to assign in Options+) |
| Icon pack (`actionicons` = `actionsymbols` = `customicons`, 336 SVGs) | `assets/surface-icons` (332, kebab-case names) | ✅ (3 Keystone icons held back for the author to confirm; the duplicate `Clarity.min.svg` left out) |
| Icon map (`CaptureOneIconMap`) | `lightcraft_surface::icons::icon_for` | ✅ |
| Settings window (`SettingsForm`: Global, Per Adjustment with filter, Bridge, Hardware, OSD, Plugin Colors, Tool Context, Tool Switching tabs) | Settings ▸ Controls | ✅ (the Redeploy Plugin button is gone: changes apply live) |
| Capture One actions and parameters (`CaptureOneCommand`, `CaptureOneAdjustment`) | LightCraft commands and targets, table below | ✅ / ✕ per action |
| Color Editor values via UI Automation (`CE*`) | colour mixer targets (`mixer.<band>.<hue/sat/lum>`) | 🔁 |
| Mode buttons switching the dial (SPEC 7.1) | `pageNext {group: dials}` on a button | ✅ |
| Tangent TIPC hub impersonation (`TipcBridge`, `TipcProtocol`, `TipcConnection`, `CaptureOneConfig`) | — | ✕ Capture One–specific |
| Capture One control list (`ControlDatabase`, `controls.xml`), inspection tools (`_inspect`, `temp_inspect*`, `reflect_lr.ps1`, `inspect_sdk*`) | — | ✕ never ported |
| Keyboard simulation (`KeyboardSimulator`) | — | ✕ commands instead |
| Named-pipe bridge (`PipeServer`, `BridgeClient`, `PipeProtocol`) | the surface protocol on the control channel | 🔁 |
| Tray app, hub window, telemetry, logs (`Program`, `HubWindow`, `BridgeTelemetry`, `Log`) | LightCraft's own log and Settings ▸ Controls status | 🔁 |
| Production plans (installer, auto-update, crash reporting, licensing, code signing, Windows service) | — | ✕ LightCraft has its own release process |
| Branding and design references (`media/`, `design-mockups/`) | — | ✕ not shipped |

### Every TactileBridge action

Generated from `lightcraft_surface::tactilebridge::mapping_table()` (its tests check that every target exists and
every dropped action says why).

**Dial values**

| TactileBridge | LightCraft | Note |
|---|---|---|
| `Kelvin` | `wb.temp` |  |
| `Tint` | `wb.tint` |  |
| `Exposure` | `light.exposure` |  |
| `Contrast` | `light.contrast` |  |
| `Brightness` | — | Lightroom and LightCraft have no Brightness slider (use Exposure, Whites or the Tone Curve) |
| `Saturation` | `color.saturation` |  |
| `Highlight` | `light.highlights` |  |
| `Shadow` | `light.shadows` |  |
| `White` | `light.whites` |  |
| `Black` | `light.blacks` |  |
| `MasterToneX` | `grading.global.hue` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `MasterToneY` | `grading.global.sat` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `ShadowToneX` | `grading.shadows.hue` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `ShadowToneY` | `grading.shadows.sat` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `ShadowLightness` | `grading.shadows.lum` |  |
| `MidtoneToneX` | `grading.midtones.hue` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `MidtoneToneY` | `grading.midtones.sat` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `MidtoneLightness` | `grading.midtones.lum` |  |
| `HighlightToneX` | `grading.highlights.hue` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `HighlightToneY` | `grading.highlights.sat` | Color Grading uses hue and saturation where Capture One's Color Balance uses x / y |
| `HighlightLightness` | `grading.highlights.lum` |  |
| `BWRed` | `bw.red` |  |
| `BWYellow` | `bw.yellow` |  |
| `BWGreen` | `bw.green` |  |
| `BWCyan` | `bw.aqua` | Cyan is called Aqua in the B&W mixer |
| `BWBlue` | `bw.blue` |  |
| `BWMagenta` | `bw.magenta` |  |
| `BWHighlightsHue` | `grading.highlights.hue` | B&W split toning is done with Color Grading |
| `BWHighlightsSat` | `grading.highlights.sat` | B&W split toning is done with Color Grading |
| `BWShadowsHue` | `grading.shadows.hue` | B&W split toning is done with Color Grading |
| `BWShadowsSat` | `grading.shadows.sat` | B&W split toning is done with Color Grading |
| `Clarity` | `effects.clarity` |  |
| `Structure` | `effects.texture` | Texture is the closest to Capture One's Structure |
| `Vignetting` | `vignette.amount` | post-crop vignette |
| `Distortion` | `optics.distortion` |  |
| `SharpnessFalloff` | — | LightCraft has no sharpness falloff correction |
| `LightFalloff` | `optics.vignetting` | lens vignetting (light falloff) correction |
| `Defringe` | `optics.defringePurple` | purple fringe amount |
| `UniformLight` | — | a Capture One feature LightCraft does not have |
| `SharpeningAmount` | `detail.sharpenAmount` |  |
| `SharpeningRadius` | `detail.sharpenRadius` |  |
| `SharpeningThreshold` | `detail.sharpenMasking` | Masking is the closest to a sharpening threshold |
| `SharpeningHalo` | — | LightCraft has no halo suppression slider |
| `NRLuminance` | `detail.nrLuminance` |  |
| `NRDetail` | `detail.nrDetail` |  |
| `NRColor` | `detail.nrColor` |  |
| `NRSinglePixel` | — | LightCraft has no single-pixel noise slider |
| `GrainImpact` | `grain.amount` |  |
| `GrainGranularity` | `grain.size` |  |
| `MoireAmount` | `mask.moire` | moiré is a mask adjustment in LightCraft (as in Lightroom) |
| `MoirePattern` | — | LightCraft's moiré reduction has no pattern size |
| `LayerOpacity` | `mask.amount` | a mask's Amount |
| `SpotRadius` | `ui.removeSize` | the Remove brush size |
| `BrushSize` | `ui.brushSize` |  |
| `BrushHardness` | `ui.brushFeather` | Feather (softness) — turn the other way for harder |
| `BrushOpacity` | — | LightCraft brushes have Size, Feather and Flow |
| `BrushFlow` | `ui.brushFlow` |  |
| `Zoom` | `ui.zoom` |  |
| `OverlayOpacity` | — | a Capture One feature LightCraft does not have |
| `OverlayScale` | — | a Capture One feature LightCraft does not have |
| `OverlayHorizontal` | — | a Capture One feature LightCraft does not have |
| `OverlayVertical` | — | a Capture One feature LightCraft does not have |
| `PenSize` | — | LightCraft has no annotations |
| `CERedHue` | `mixer.red.hue` |  |
| `CERedSat` | `mixer.red.sat` |  |
| `CERedLight` | `mixer.red.lum` |  |
| `CEOrangeHue` | `mixer.orange.hue` |  |
| `CEOrangeSat` | `mixer.orange.sat` |  |
| `CEOrangeLight` | `mixer.orange.lum` |  |
| `CEYellowHue` | `mixer.yellow.hue` |  |
| `CEYellowSat` | `mixer.yellow.sat` |  |
| `CEYellowLight` | `mixer.yellow.lum` |  |
| `CEGreenHue` | `mixer.green.hue` |  |
| `CEGreenSat` | `mixer.green.sat` |  |
| `CEGreenLight` | `mixer.green.lum` |  |
| `CECyanHue` | `mixer.aqua.hue` | Cyan is called Aqua in the colour mixer |
| `CECyanSat` | `mixer.aqua.sat` | Cyan is called Aqua in the colour mixer |
| `CECyanLight` | `mixer.aqua.lum` | Cyan is called Aqua in the colour mixer |
| `CEBlueHue` | `mixer.blue.hue` |  |
| `CEBlueSat` | `mixer.blue.sat` |  |
| `CEBlueLight` | `mixer.blue.lum` |  |
| `CEPurpleHue` | `mixer.purple.hue` |  |
| `CEPurpleSat` | `mixer.purple.sat` |  |
| `CEPurpleLight` | `mixer.purple.lum` |  |
| `CEMagentaHue` | `mixer.magenta.hue` |  |
| `CEMagentaSat` | `mixer.magenta.sat` |  |
| `CEMagentaLight` | `mixer.magenta.lum` |  |

**Keys**

| TactileBridge | LightCraft | Note |
|---|---|---|
| `ClearRating` | `cmd:photo.rate` `{"rating":0}` |  |
| `Rate1Star` | `cmd:photo.rate` `{"rating":1}` |  |
| `Rate2Stars` | `cmd:photo.rate` `{"rating":2}` |  |
| `Rate3Stars` | `cmd:photo.rate` `{"rating":3}` |  |
| `Rate4Stars` | `cmd:photo.rate` `{"rating":4}` |  |
| `Rate5Stars` | `cmd:photo.rate` `{"rating":5}` |  |
| `ColorTagNone` | `cmd:photo.label` `{"label":"none"}` |  |
| `ColorTagRed` | `cmd:photo.label` `{"label":"red"}` |  |
| `ColorTagOrange` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `ColorTagYellow` | `cmd:photo.label` `{"label":"yellow"}` |  |
| `ColorTagGreen` | `cmd:photo.label` `{"label":"green"}` |  |
| `ColorTagBlue` | `cmd:photo.label` `{"label":"blue"}` |  |
| `ColorTagPink` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `ColorTagPurple` | `cmd:photo.label` `{"label":"purple"}` |  |
| `PreviousImage` | `cmd:library.previous` |  |
| `NextImage` | `cmd:library.next` |  |
| `FirstImage` | — | no LightCraft command jumps to the first or last photo yet |
| `LastImage` | — | no LightCraft command jumps to the first or last photo yet |
| `SelectAll` | `cmd:library.selectAll` |  |
| `DeselectAll` | `cmd:library.selectNone` |  |
| `InvertSelection` | — | no LightCraft command inverts the selection yet |
| `SelectPrimaryOnly` | — | a Capture One feature LightCraft does not have |
| `DeselectPrimary` | — | a Capture One feature LightCraft does not have |
| `SelectSameColorTag` | — | a Capture One feature LightCraft does not have |
| `SelectSameSequenceId` | — | a Capture One feature LightCraft does not have |
| `SelectSameVariantPos` | — | a Capture One feature LightCraft does not have |
| `PreviousSet` | — | a Capture One feature LightCraft does not have |
| `NextSet` | — | a Capture One feature LightCraft does not have |
| `PreviousCollection` | — | a Capture One feature LightCraft does not have |
| `NextCollection` | — | a Capture One feature LightCraft does not have |
| `Undo` | `cmd:edit.undo` |  |
| `Redo` | `cmd:edit.redo` |  |
| `ResetAdjustments` | `cmd:develop.reset` |  |
| `CopyAdjustments` | `cmd:develop.copy` |  |
| `ApplyAdjustments` | `cmd:develop.paste` |  |
| `CopyAndApplyAdj` | `cmd:develop.sync` | copies the active photo's settings to the other selected photos |
| `AutoAdjust` | `cmd:develop.auto` |  |
| `RotateLeft` | `cmd:photo.rotateLeft` |  |
| `RotateRight` | `cmd:photo.rotateRight` |  |
| `SnapTo90` | `cmd:develop.resetControl` `{"control":"crop.angle"}` | straightens back to 0° |
| `NewEmptyLayer` | `cmd:mask.add` `{"kind":"brush"}` | a new brush mask |
| `NewFilledLayer` | `cmd:mask.add` `{"center":[0.5,0.5],"feather":0.0,"kind":"radial","rx":1.0,"ry":1.0}` | a new mask over the whole photo |
| `NewCloneLayer` | `cmd:panel.remove` | the Remove tool (heal and clone) |
| `NewHealLayer` | `cmd:panel.remove` | the Remove tool (heal and clone) |
| `RemoveLayer` | `cmd:mask.delete` | deletes the selected mask |
| `SelectLayerUp` | `cmd:surface.maskPrevious` | selects the mask above |
| `SelectLayerDown` | `cmd:surface.maskNext` | selects the mask below |
| `SelectBackground` | `cmd:mask.select` `{"id":null}` | deselects the mask |
| `ToggleLayerOnOff` | `cmd:mask.visible` | shows or hides the selected mask |
| `InvertMask` | `cmd:mask.invert` |  |
| `FillMask` | — | a Capture One feature LightCraft does not have |
| `ClearMask` | — | a Capture One feature LightCraft does not have |
| `RasterizeMask` | — | a Capture One feature LightCraft does not have |
| `RefineMask` | — | Refine runs on a mask's own component in LightCraft |
| `FeatherMask` | — | a Capture One feature LightCraft does not have |
| `LumaRange` | `cmd:mask.add` `{"kind":"luminanceRange"}` | a new luminance range mask |
| `ToggleSelectionPoints` | `cmd:view.maskPins` | mask pins |
| `ZoomIn` | `cmd:view.zoomIn` |  |
| `ZoomOut` | `cmd:view.zoomOut` |  |
| `ZoomToFit` | `cmd:view.zoomFit` |  |
| `Zoom100` | `cmd:view.zoom100` |  |
| `Zoom200` | `cmd:view.navigate` `{"zoom":{"percent":200}}` |  |
| `Zoom400` | `cmd:view.navigate` `{"zoom":{"percent":400}}` |  |
| `IncreaseZoom` | `cmd:view.zoomIn` |  |
| `DecreaseZoom` | `cmd:view.zoomOut` |  |
| `ToggleMultiView` | `cmd:view.survey` | Survey view |
| `ToggleExposureWarn` | `cmd:view.clipping` | clipping warnings |
| `ToggleFocusMask` | — | LightCraft has no focus mask yet |
| `ToggleOverlay` | — | a Capture One feature LightCraft does not have |
| `ToggleBeforeAfter` | `cmd:view.beforeAfter` |  |
| `ShowGuides` | `cmd:view.cropOverlay` | cycles the crop overlay |
| `ShowGrid` | `cmd:view.cropOverlay` | cycles the crop overlay |
| `RecipeProofing` | `cmd:view.softProof` | soft proofing |
| `FullScreen` | `cmd:view.fullScreenPreview` |  |
| `SlideShow` | `cmd:view.slideshow` |  |
| `ShowHideViewer` | — | a Capture One feature LightCraft does not have |
| `ShowHideBrowser` | `cmd:view.filmstrip` | the filmstrip |
| `ToggleTools` | `cmd:panel.edit` | opens or closes the Edit panel |
| `SelectTool` | `cmd:tool.none` | no tool: click and drag pans |
| `MoveTool` | `cmd:tool.none` | no tool: click and drag pans |
| `PanTool` | `cmd:tool.none` | no tool: click and drag pans |
| `ZoomInTool` | `cmd:view.zoomIn` |  |
| `ZoomOutTool` | `cmd:view.zoomOut` |  |
| `CropTool` | `cmd:panel.crop` | Crop & Rotate (straighten there) |
| `SwitchCropOrientation` | `cmd:crop.rotateAspect` |  |
| `StraightenTool` | `cmd:panel.crop` | Crop & Rotate (straighten there) |
| `RotateFreehandTool` | `cmd:panel.crop` | Crop & Rotate (straighten there) |
| `SpotRemovalTool` | `cmd:panel.remove` | the Remove tool (heal and clone) |
| `WBPickerTool` | `cmd:tool.wbPicker` |  |
| `ApplyNormalization` | — | a Capture One feature LightCraft does not have |
| `PickNormTarget` | — | a Capture One feature LightCraft does not have |
| `PickHighlightLevel` | — | LightCraft's targeted adjustment works on the photo with the pointer |
| `PickShadowLevel` | — | LightCraft's targeted adjustment works on the photo with the pointer |
| `PickCurvePoint` | — | LightCraft's targeted adjustment works on the photo with the pointer |
| `PickLumaCurve` | — | LightCraft's targeted adjustment works on the photo with the pointer |
| `PickColorCorrection` | — | LightCraft's targeted adjustment works on the photo with the pointer |
| `DirectColorEditor` | `cmd:section.color` | the Color section (Point Color is there) |
| `PickSkinColor` | — | a Capture One feature LightCraft does not have |
| `AddColorReadout` | — | a Capture One feature LightCraft does not have |
| `DeleteColorReadout` | — | a Capture One feature LightCraft does not have |
| `OverlayTool` | — | a Capture One feature LightCraft does not have |
| `WatermarkTool` | — | a Capture One feature LightCraft does not have |
| `FocusMeterTool` | — | a Capture One feature LightCraft does not have |
| `PickFocusPoint` | — | a Capture One feature LightCraft does not have |
| `LoupeTool` | `cmd:view.zoomToggle` | zooms to 100 % and back |
| `KeystoneTool` | `cmd:tool.guidedUpright` | Guided Upright |
| `KeystoneVertical` | `cmd:geometry.upright` `{"mode":"vertical"}` | Upright: Vertical |
| `KeystoneHorizontal` | `cmd:geometry.upright` `{"mode":"level"}` | Upright: Level |
| `DrawMaskTool` | `cmd:tool.brush` |  |
| `DrawHealTool` | `cmd:panel.remove` | the Remove tool (heal and clone) |
| `DrawCloneTool` | `cmd:panel.remove` | the Remove tool (heal and clone) |
| `EraseMaskTool` | `cmd:tool.brush` `{"op":"subtract"}` | a subtracting brush on the selected mask |
| `LinearGradientTool` | `cmd:tool.linear` |  |
| `RadialGradientTool` | `cmd:tool.radial` |  |
| `Process` | `cmd:dialog.export` | the Export dialog |
| `ExportOriginals` | `cmd:dialog.export` | the Export dialog |
| `ExportVariants` | `cmd:dialog.export` | the Export dialog |
| `ImportImages` | `cmd:file.addPhotos` |  |
| `Print` | `cmd:dialog.contactSheet` | LightCraft prints contact sheets as PDF |
| `EditAllSelected` | `cmd:develop.autoSync` | Auto Sync: edits change every selected photo |
| `Rename` | `cmd:dialog.rename` |  |
| `NewVariant` | `cmd:photo.virtualCopy` | a virtual copy |
| `CloneVariant` | `cmd:photo.virtualCopy` | a virtual copy |
| `Delete` | `cmd:photo.delete` | moves to Recently Deleted |
| `MoveToTrash` | `cmd:photo.delete` | moves to Recently Deleted |
| `RegeneratePreviews` | `cmd:library.buildPreviews` |  |
| `SetCompareVariant` | `cmd:photo.setReference` | the reference photo |
| `ClearCompareVariant` | — | a Capture One feature LightCraft does not have |
| `PickVariant` | `cmd:photo.pick` | flags as Pick |
| `PromoteVariant` | `cmd:stack.moveUp` | one place up in the stack |
| `DemoteVariant` | `cmd:stack.moveDown` | one place down in the stack |
| `ShowInLibrary` | `cmd:view.back` | back to the grid |
| `ShowInFileSystem` | `cmd:app.showInFinder` |  |
| `Capture` | — | tethered capture is not in LightCraft yet |
| `CompositionMode` | — | tethered capture is not in LightCraft yet |
| `LiveView` | — | tethered capture is not in LightCraft yet |
| `AutoFocus` | — | tethered capture is not in LightCraft yet |
| `AutoFocusFine` | — | tethered capture is not in LightCraft yet |
| `ShutterSpeedUp` | — | tethered capture is not in LightCraft yet |
| `ShutterSpeedDown` | — | tethered capture is not in LightCraft yet |
| `ApertureUp` | — | tethered capture is not in LightCraft yet |
| `ApertureDown` | — | tethered capture is not in LightCraft yet |
| `ExposureBiasUp` | — | tethered capture is not in LightCraft yet |
| `ExposureBiasDown` | — | tethered capture is not in LightCraft yet |
| `ISOUp` | — | tethered capture is not in LightCraft yet |
| `ISODown` | — | tethered capture is not in LightCraft yet |
| `BWEnable` | `cmd:develop.treatment` |  |
| `ToggleGrading` | — | Capture One's grading switch has no single LightCraft counterpart |
| `SelectPen` | — | LightCraft has no annotations |
| `SelectEraser` | — | LightCraft has no annotations |
| `PenBlack` | — | LightCraft has no annotations |
| `PenWhite` | — | LightCraft has no annotations |
| `PenRed` | — | LightCraft has no annotations |
| `PenYellow` | — | LightCraft has no annotations |
| `PenGreen` | — | LightCraft has no annotations |
| `PenBlue` | — | LightCraft has no annotations |
| `ToggleSelectImage` | — | these sent keystrokes to Capture One's import window; LightCraft's import dialog takes the keyboard directly |
| `NextGroup` | — | these sent keystrokes to Capture One's import window; LightCraft's import dialog takes the keyboard directly |
| `PreviousGroup` | — | these sent keystrokes to Capture One's import window; LightCraft's import dialog takes the keyboard directly |
| `NextImageImport` | — | these sent keystrokes to Capture One's import window; LightCraft's import dialog takes the keyboard directly |
| `PreviousImageImport` | — | these sent keystrokes to Capture One's import window; LightCraft's import dialog takes the keyboard directly |
| `FilterRatingNone` | `cmd:library.filter` `{"rating":0,"ratingOp":"exactly"}` | shows unrated photos |
| `FilterRating1Star` | `cmd:library.filter` `{"rating":1,"ratingOp":"atLeast"}` |  |
| `FilterRating2Stars` | `cmd:library.filter` `{"rating":2,"ratingOp":"atLeast"}` |  |
| `FilterRating3Stars` | `cmd:library.filter` `{"rating":3,"ratingOp":"atLeast"}` |  |
| `FilterRating4Stars` | `cmd:library.filter` `{"rating":4,"ratingOp":"atLeast"}` |  |
| `FilterRating5Stars` | `cmd:library.filter` `{"rating":5,"ratingOp":"atLeast"}` |  |
| `FilterTagNone` | `cmd:library.filter` `{"label":null}` |  |
| `FilterTagRed` | `cmd:library.filter` `{"label":"red"}` |  |
| `FilterTagOrange` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `FilterTagYellow` | `cmd:library.filter` `{"label":"yellow"}` |  |
| `FilterTagGreen` | `cmd:library.filter` `{"label":"green"}` |  |
| `FilterTagBlue` | `cmd:library.filter` `{"label":"blue"}` |  |
| `FilterTagPink` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `FilterTagPurple` | `cmd:library.filter` `{"label":"purple"}` |  |
| `SelectUnrated` | `cmd:library.selectBy` `{"rating":0,"ratingOp":"eq"}` |  |
| `Select1Star` | `cmd:library.selectBy` `{"rating":1,"ratingOp":"eq"}` |  |
| `Select2Stars` | `cmd:library.selectBy` `{"rating":2,"ratingOp":"eq"}` |  |
| `Select3Stars` | `cmd:library.selectBy` `{"rating":3,"ratingOp":"eq"}` |  |
| `Select4Stars` | `cmd:library.selectBy` `{"rating":4,"ratingOp":"eq"}` |  |
| `Select5Stars` | `cmd:library.selectBy` `{"rating":5,"ratingOp":"eq"}` |  |
| `SelectTagNone` | `cmd:library.selectBy` `{"label":"none"}` |  |
| `SelectTagRed` | `cmd:library.selectBy` `{"label":"red"}` |  |
| `SelectTagOrange` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `SelectTagYellow` | `cmd:library.selectBy` `{"label":"yellow"}` |  |
| `SelectTagGreen` | `cmd:library.selectBy` `{"label":"green"}` |  |
| `SelectTagBlue` | `cmd:library.selectBy` `{"label":"blue"}` |  |
| `SelectTagPink` | — | LightCraft's colour labels are red, yellow, green, blue and purple (as Lightroom's) |
| `SelectTagPurple` | `cmd:library.selectBy` `{"label":"purple"}` |  |
| `AutoSync` | Apply to Selected | dials change every selected photo |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-14 | major | Created: the control-surface engine (ported from TactileBridge), Settings ▸ Controls, the surface protocol v1, the MX Creative Console |
