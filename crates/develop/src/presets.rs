//! Presets and copy/paste subsets.
//!
//! A preset is a *partial* settings object (JSON) — only the groups it includes. Applying merges it
//! into the photo's settings; `amount` (0..200 %) interpolates numeric values between the current
//! settings and the preset's. Copy/paste/sync use the same group selection ([`SettingsGroup`]).
//!
//! The rendering process ([`crate::ProcessVersion`]) is in no group: copy, paste, sync and presets
//! made from a photo never carry it, so the photos they change keep their own. A partial that names
//! `process` itself (Apply Settings JSON) sets it like any other field.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

use crate::DevelopSettings;

/// Groups shown in "Choose settings to copy" / "Create preset" (Lightroom's checklist).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsGroup {
    Profile,
    Treatment,
    WhiteBalance,
    Light,
    ToneCurve,
    Color,
    ColorMixer,
    ColorGrading,
    Effects,
    Vignette,
    Grain,
    Detail,
    Optics,
    Geometry,
    Crop,
    Masks,
    Spots,
    RedEye,
    LensBlur,
    Calibration,
    /// Retouch: Smooth Skin's sliders and which skin it works on, and the other retouch sections
    /// (teeth, eyes, lips, hair…) with the person each is for; never the skin and parts found on
    /// the photo (each photo's own are found for it). (Its id is still `smoothSkin`.)
    SmoothSkin,
}

impl SettingsGroup {
    pub const ALL: [SettingsGroup; 21] = [
        SettingsGroup::Profile,
        SettingsGroup::Treatment,
        SettingsGroup::WhiteBalance,
        SettingsGroup::Light,
        SettingsGroup::ToneCurve,
        SettingsGroup::Color,
        SettingsGroup::ColorMixer,
        SettingsGroup::ColorGrading,
        SettingsGroup::Effects,
        SettingsGroup::Vignette,
        SettingsGroup::Grain,
        SettingsGroup::Detail,
        SettingsGroup::Optics,
        SettingsGroup::Geometry,
        SettingsGroup::Crop,
        SettingsGroup::Masks,
        SettingsGroup::Spots,
        SettingsGroup::RedEye,
        SettingsGroup::LensBlur,
        SettingsGroup::Calibration,
        SettingsGroup::SmoothSkin,
    ];

    /// Groups a new preset includes by default: the copy defaults without White Balance (each
    /// photo keeps its own unless the photographer ticks it; client note 8b, the user's decision).
    pub fn default_preset() -> Vec<SettingsGroup> {
        SettingsGroup::default_copy().into_iter().filter(|g| *g != SettingsGroup::WhiteBalance).collect()
    }

    /// Groups included by default when copying (Lightroom excludes crop, masks and spots by default).
    pub fn default_copy() -> Vec<SettingsGroup> {
        SettingsGroup::ALL
            .into_iter()
            .filter(|g| {
                !matches!(g, SettingsGroup::Crop | SettingsGroup::Masks | SettingsGroup::Spots | SettingsGroup::RedEye | SettingsGroup::SmoothSkin)
            })
            .collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsGroup::Profile => "Profile",
            SettingsGroup::Treatment => "Treatment",
            SettingsGroup::WhiteBalance => "White Balance",
            SettingsGroup::Light => "Light",
            SettingsGroup::ToneCurve => "Tone Curve",
            SettingsGroup::Color => "Color",
            SettingsGroup::ColorMixer => "Color Mixer",
            SettingsGroup::ColorGrading => "Color Grading",
            SettingsGroup::Effects => "Effects",
            SettingsGroup::Vignette => "Vignette",
            SettingsGroup::Grain => "Grain",
            SettingsGroup::Detail => "Detail",
            SettingsGroup::Optics => "Optics",
            SettingsGroup::Geometry => "Geometry",
            SettingsGroup::Crop => "Crop",
            SettingsGroup::Masks => "Masking",
            SettingsGroup::Spots => "Remove",
            SettingsGroup::RedEye => "Red Eye",
            SettingsGroup::LensBlur => "Lens Blur",
            SettingsGroup::Calibration => "Calibration",
            SettingsGroup::SmoothSkin => "Retouch",
        }
    }

    /// Top-level JSON keys of `DevelopSettings` that belong to the group.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            SettingsGroup::Profile => &["profile"],
            SettingsGroup::Treatment => &["treatment"],
            SettingsGroup::WhiteBalance => &["wb"],
            SettingsGroup::Light => &["light", "hdr"],
            SettingsGroup::ToneCurve => &["curve"],
            SettingsGroup::Color => &["color"],
            SettingsGroup::ColorMixer => &["mixer", "bw_mix", "point_colors"],
            SettingsGroup::ColorGrading => &["grading"],
            SettingsGroup::Effects => &["effects"],
            SettingsGroup::Vignette => &["vignette"],
            SettingsGroup::Grain => &["grain"],
            SettingsGroup::Detail => &["detail", "enhance"],
            SettingsGroup::Optics => &["optics"],
            SettingsGroup::Geometry => &["geometry"],
            SettingsGroup::Crop => &["crop", "orientation"],
            SettingsGroup::Masks => &["masks"],
            SettingsGroup::Spots => &["spots"],
            SettingsGroup::RedEye => &["red_eye"],
            SettingsGroup::LensBlur => &["lens_blur"],
            // (the camera profile is the camera calibration)
            SettingsGroup::Calibration => &["calibration", "camera_profile"],
            SettingsGroup::SmoothSkin => &["skin", "retouch"],
        }
    }
}

/// The subset of `s` covering `groups`, as a partial JSON object.
pub fn extract_groups(s: &DevelopSettings, groups: &[SettingsGroup]) -> Value {
    let full = s.to_json();
    let mut out = Map::new();
    for g in groups {
        for k in g.keys() {
            if let Some(v) = full.get(*k) {
                out.insert((*k).to_string(), v.clone());
            }
        }
        // (not written while off: copying it off still switches it off where it is pasted)
        if *g == SettingsGroup::SmoothSkin && !out.contains_key("skin") {
            out.insert("skin".into(), serde_json::to_value(crate::SkinSmoothing::default()).unwrap_or(Value::Null));
        }
        if *g == SettingsGroup::SmoothSkin && !out.contains_key("retouch") {
            out.insert("retouch".into(), serde_json::to_value(crate::Retouch::default()).unwrap_or(Value::Null));
        }
    }
    Value::Object(out)
}

/// One setting inside a [`SettingsGroup`] that can be ticked on its own in the New Preset, Update
/// Preset and Copy Settings trees (Lightroom's per-setting checklist): Exposure inside Light, the
/// Sharpening sliders inside Detail… `id` is `<group id>.<name>`; `paths` are the dot paths of the
/// settings JSON it covers (a group's items together cover every key of the group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingItem {
    pub id: &'static str,
    pub label: &'static str,
    pub paths: &'static [&'static str],
}

const fn item(id: &'static str, label: &'static str, paths: &'static [&'static str]) -> SettingItem {
    SettingItem { id, label, paths }
}

const LIGHT_ITEMS: &[SettingItem] = &[
    item("light.exposure", "Exposure", &["light.exposure"]),
    item("light.contrast", "Contrast", &["light.contrast"]),
    item("light.highlights", "Highlights", &["light.highlights"]),
    item("light.shadows", "Shadows", &["light.shadows"]),
    item("light.whites", "Whites", &["light.whites"]),
    item("light.blacks", "Blacks", &["light.blacks"]),
    item("light.hdr", "HDR Output", &["hdr"]),
];
const CURVE_ITEMS: &[SettingItem] = &[
    item(
        "toneCurve.parametric",
        "Parametric Curve",
        &["curve.highlights", "curve.lights", "curve.darks", "curve.shadows", "curve.split_shadows", "curve.split_mid", "curve.split_highlights"],
    ),
    item("toneCurve.point", "Point Curve", &["curve.master", "curve.red", "curve.green", "curve.blue"]),
    item("toneCurve.refineSaturation", "Refine Saturation", &["curve.refine_saturation"]),
];
const COLOR_ITEMS: &[SettingItem] =
    &[item("color.vibrance", "Vibrance", &["color.vibrance"]), item("color.saturation", "Saturation", &["color.saturation"])];
const MIXER_ITEMS: &[SettingItem] = &[
    item("colorMixer.mixer", "Color Mixer", &["mixer"]),
    item("colorMixer.bw", "B&W Mix", &["bw_mix"]),
    item("colorMixer.pointColor", "Point Color", &["point_colors"]),
];
const EFFECTS_ITEMS: &[SettingItem] = &[
    item("effects.texture", "Texture", &["effects.texture"]),
    item("effects.clarity", "Clarity", &["effects.clarity"]),
    item("effects.dehaze", "Dehaze", &["effects.dehaze"]),
];
const DETAIL_ITEMS: &[SettingItem] = &[
    item("detail.sharpening", "Sharpening", &["detail.sharpen_amount", "detail.sharpen_radius", "detail.sharpen_detail", "detail.sharpen_masking"]),
    item("detail.noise", "Noise Reduction", &["detail.nr_luminance", "detail.nr_detail", "detail.nr_contrast"]),
    item("detail.colorNoise", "Color Noise Reduction", &["detail.nr_color", "detail.nr_color_detail", "detail.nr_color_smoothness"]),
    item("detail.enhance", "AI Denoise and Enhance", &["enhance"]),
];
const OPTICS_ITEMS: &[SettingItem] = &[
    item("optics.lensProfile", "Lens Profile Corrections", &["optics.lens_profile", "optics.profile_distortion", "optics.profile_vignetting"]),
    item("optics.chromatic", "Chromatic Aberration", &["optics.remove_ca", "optics.ca_red", "optics.ca_blue"]),
    item(
        "optics.defringe",
        "Defringe",
        &[
            "optics.defringe_purple_amount",
            "optics.defringe_purple_hue_lo",
            "optics.defringe_purple_hue_hi",
            "optics.defringe_green_amount",
            "optics.defringe_green_hue_lo",
            "optics.defringe_green_hue_hi",
        ],
    ),
    item("optics.distortion", "Manual Distortion", &["optics.distortion"]),
    item("optics.vignetting", "Manual Vignetting", &["optics.vignetting", "optics.vignetting_midpoint"]),
];
const GEOMETRY_ITEMS: &[SettingItem] = &[
    item("geometry.upright", "Upright", &["geometry.upright", "geometry.guides", "geometry.upright_transform"]),
    item(
        "geometry.transform",
        "Transform Sliders",
        &[
            "geometry.vertical",
            "geometry.horizontal",
            "geometry.rotate",
            "geometry.aspect",
            "geometry.scale",
            "geometry.offset_x",
            "geometry.offset_y",
        ],
    ),
    item("geometry.constrainCrop", "Constrain Crop", &["geometry.constrain_crop"]),
];
const CROP_ITEMS: &[SettingItem] =
    &[item("crop.crop", "Crop and Straighten", &["crop"]), item("crop.orientation", "Rotation and Flip", &["orientation"])];
const CALIBRATION_ITEMS: &[SettingItem] =
    &[item("calibration.cameraProfile", "Camera Profile", &["camera_profile"]), item("calibration.sliders", "Calibration Sliders", &["calibration"])];
const RETOUCH_ITEMS: &[SettingItem] =
    &[item("smoothSkin.skin", "Smooth Skin", &["skin"]), item("smoothSkin.parts", "Teeth, Eyes, Lips and Hair", &["retouch"])];

impl SettingsGroup {
    /// The settings that can be ticked one by one (empty: the group is one setting).
    pub fn items(self) -> &'static [SettingItem] {
        match self {
            SettingsGroup::Light => LIGHT_ITEMS,
            SettingsGroup::ToneCurve => CURVE_ITEMS,
            SettingsGroup::Color => COLOR_ITEMS,
            SettingsGroup::ColorMixer => MIXER_ITEMS,
            SettingsGroup::Effects => EFFECTS_ITEMS,
            SettingsGroup::Detail => DETAIL_ITEMS,
            SettingsGroup::Optics => OPTICS_ITEMS,
            SettingsGroup::Geometry => GEOMETRY_ITEMS,
            SettingsGroup::Crop => CROP_ITEMS,
            SettingsGroup::Calibration => CALIBRATION_ITEMS,
            SettingsGroup::SmoothSkin => RETOUCH_ITEMS,
            _ => &[],
        }
    }

    /// The group's id (`light`, `whiteBalance`…), as commands take it.
    pub fn id(self) -> &'static str {
        match self {
            SettingsGroup::Profile => "profile",
            SettingsGroup::Treatment => "treatment",
            SettingsGroup::WhiteBalance => "whiteBalance",
            SettingsGroup::Light => "light",
            SettingsGroup::ToneCurve => "toneCurve",
            SettingsGroup::Color => "color",
            SettingsGroup::ColorMixer => "colorMixer",
            SettingsGroup::ColorGrading => "colorGrading",
            SettingsGroup::Effects => "effects",
            SettingsGroup::Vignette => "vignette",
            SettingsGroup::Grain => "grain",
            SettingsGroup::Detail => "detail",
            SettingsGroup::Optics => "optics",
            SettingsGroup::Geometry => "geometry",
            SettingsGroup::Crop => "crop",
            SettingsGroup::Masks => "masks",
            SettingsGroup::Spots => "spots",
            SettingsGroup::RedEye => "redEye",
            SettingsGroup::LensBlur => "lensBlur",
            SettingsGroup::Calibration => "calibration",
            SettingsGroup::SmoothSkin => "smoothSkin",
        }
    }

    /// The group with id `id`.
    pub fn from_id(id: &str) -> Option<SettingsGroup> {
        SettingsGroup::ALL.into_iter().find(|g| g.id() == id)
    }
}

/// The setting item with id `id` and its group.
pub fn find_item(id: &str) -> Option<(SettingsGroup, &'static SettingItem)> {
    SettingsGroup::ALL.into_iter().find_map(|g| g.items().iter().find(|i| i.id == id).map(|i| (g, i)))
}

/// What to copy, save in a preset or sync: whole groups, plus single settings of groups that are only
/// partly ticked ([`SettingItem`] ids). Remembered by the dialogs that choose it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsPick {
    pub groups: Vec<SettingsGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<String>,
}

impl SettingsPick {
    /// Whole groups only.
    pub fn of_groups(groups: Vec<SettingsGroup>) -> SettingsPick {
        SettingsPick { groups, settings: Vec::new() }.normalized()
    }

    /// A list of group and setting ids (`light`, `detail.sharpening`) as the dialogs keep them: a group
    /// whose every setting is ticked becomes the whole group. Unknown ids are left out.
    pub fn from_ids<S: AsRef<str>>(ids: &[S]) -> SettingsPick {
        let mut pick = SettingsPick::default();
        for id in ids {
            let id = id.as_ref();
            if let Some(g) = SettingsGroup::from_id(id) {
                if !pick.groups.contains(&g) {
                    pick.groups.push(g);
                }
            } else if find_item(id).is_some() && !pick.settings.iter().any(|s| s == id) {
                pick.settings.push(id.to_string());
            }
        }
        pick.normalized()
    }

    /// The pick as ids, one per tickable box: a group with settings lists each of them (the tree's leaves).
    pub fn to_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for g in SettingsGroup::ALL {
            if self.groups.contains(&g) {
                if g.items().is_empty() {
                    out.push(g.id().to_string());
                } else {
                    out.extend(g.items().iter().map(|i| i.id.to_string()));
                }
            } else {
                out.extend(g.items().iter().filter(|i| self.settings.iter().any(|s| s == i.id)).map(|i| i.id.to_string()));
            }
        }
        out
    }

    /// Single settings of a group that is whole anyway are dropped; a group whose settings are all
    /// there becomes whole; unknown setting ids are dropped.
    pub fn normalized(mut self) -> SettingsPick {
        let groups = self.groups.clone();
        self.settings.retain(|s| find_item(s).is_some_and(|(g, _)| !groups.contains(&g)));
        for g in SettingsGroup::ALL {
            let items = g.items();
            if !items.is_empty() && !self.groups.contains(&g) && items.iter().all(|i| self.settings.iter().any(|s| s == i.id)) {
                self.groups.push(g);
                self.settings.retain(|s| !items.iter().any(|i| i.id == s));
            }
        }
        // in the groups' own order, once each
        self.groups = SettingsGroup::ALL.into_iter().filter(|g| self.groups.contains(g)).collect();
        let mut seen = Vec::new();
        self.settings.retain(|s| {
            let new = !seen.contains(s);
            seen.push(s.clone());
            new
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.settings.is_empty()
    }

    /// The pick includes all or part of `g`.
    pub fn touches(&self, g: SettingsGroup) -> bool {
        self.groups.contains(&g) || g.items().iter().any(|i| self.settings.iter().any(|s| s == i.id))
    }

    /// The pick includes setting (or group) `id`.
    pub fn has(&self, id: &str) -> bool {
        if let Some(g) = SettingsGroup::from_id(id) {
            return self.groups.contains(&g);
        }
        self.settings.iter().any(|s| s == id) || find_item(id).is_some_and(|(g, _)| self.groups.contains(&g))
    }

    /// Only what both picks hold (Paste Selected: the ticked part of what was copied).
    pub fn intersect(&self, other: &SettingsPick) -> SettingsPick {
        let ids: Vec<String> = self.to_ids().into_iter().filter(|id| other.has(id)).collect();
        SettingsPick::from_ids(&ids)
    }

    /// The part of `s` the pick covers, as a partial JSON object.
    pub fn extract(&self, s: &DevelopSettings) -> Value {
        let mut out = extract_groups(s, &self.groups);
        if self.settings.is_empty() {
            return out;
        }
        let full = s.to_json();
        let Some(o) = out.as_object_mut() else { return out };
        for id in &self.settings {
            let Some((_, it)) = find_item(id) else { continue };
            for path in it.paths {
                copy_path(&full, o, path);
            }
            // (left out of the JSON while off, as the whole group knows: copying it off switches it off)
            for (key, default) in [
                ("skin", serde_json::to_value(crate::SkinSmoothing::default())),
                ("retouch", serde_json::to_value(crate::Retouch::default())),
                ("hdr", serde_json::to_value(crate::Hdr::default())),
            ] {
                if it.paths.contains(&key) && !o.contains_key(key) {
                    o.insert(key.into(), default.unwrap_or(Value::Null));
                }
            }
        }
        out
    }

    /// What the partial settings `partial` (a preset's) hold, as a pick: a key of a group with several
    /// settings counts for the settings it has.
    pub fn of_partial(partial: &Value) -> SettingsPick {
        let Some(o) = partial.as_object() else { return SettingsPick::default() };
        let mut ids: Vec<String> = Vec::new();
        for g in SettingsGroup::ALL {
            if g.items().is_empty() {
                if g.keys().iter().any(|k| o.contains_key(*k)) {
                    ids.push(g.id().to_string());
                }
                continue;
            }
            for it in g.items() {
                if it.paths.iter().any(|p| path_in(partial, p)) {
                    ids.push(it.id.to_string());
                }
            }
        }
        SettingsPick::from_ids(&ids)
    }
}

/// `path` (dot separated) is in `v`.
fn path_in(v: &Value, path: &str) -> bool {
    let mut cur = v;
    for k in path.split('.') {
        match cur.get(k) {
            Some(n) => cur = n,
            None => return false,
        }
    }
    true
}

/// Copy the value at `path` in `from` into `to` (creating the objects on the way).
fn copy_path(from: &Value, to: &mut Map<String, Value>, path: &str) {
    let mut parts = path.split('.').peekable();
    let mut src = from;
    let mut dst = to;
    while let Some(k) = parts.next() {
        let Some(v) = src.get(k) else { return };
        if parts.peek().is_none() {
            dst.insert(k.to_string(), v.clone());
            return;
        }
        src = v;
        let entry = dst.entry(k.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Some(next) = entry.as_object_mut() else { return };
        dst = next;
    }
}

/// Recursively merge `patch` into `base` (objects merge; everything else replaces).
pub fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Interpolate numbers in `patch` towards/away from `base` by `t` (1 = patch as-is); non-numeric
/// values are taken from the patch when `t >= 0.5`.
fn scale_patch(base: &Value, patch: &Value, t: f64) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out = Map::new();
            for (k, pv) in p {
                let bv = b.get(k).unwrap_or(&Value::Null);
                out.insert(k.clone(), scale_patch(bv, pv, t));
            }
            Value::Object(out)
        }
        // Integer fields (the grain seed, ids) are discrete like strings: an interpolated seed would
        // not deserialize as an integer, and the whole preset would silently not apply.
        (Value::Number(b), Value::Number(p)) if b.is_f64() || p.is_f64() => {
            let (b, p) = (b.as_f64().unwrap_or(0.0), p.as_f64().unwrap_or(0.0));
            serde_json::Number::from_f64(b + (p - b) * t).map(Value::Number).unwrap_or(Value::Null)
        }
        (b, p) => {
            if t >= 0.5 {
                p.clone()
            } else {
                b.clone()
            }
        }
    }
}

/// Apply a partial settings object with an amount (1.0 = 100 %). Out-of-range values are clamped
/// through the control specs, as every merge does ([`DevelopSettings::merged`]).
pub fn apply_partial(s: &DevelopSettings, partial: &Value, amount: f64) -> DevelopSettings {
    let patch = if (amount - 1.0).abs() < 1e-9 { Cow::Borrowed(partial) } else { Cow::Owned(scale_patch(&s.to_json(), partial, amount)) };
    // (a white balance without a scale keeps its old meaning: `DevelopSettings::merged`)
    s.merged(&patch).unwrap_or_else(|_| s.clone())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub group: String,
    /// Partial settings.
    pub settings: Value,
    #[serde(default)]
    pub favorite: bool,
    /// Built-in presets ship with the app and can't be deleted.
    #[serde(default)]
    pub builtin: bool,
}

impl Preset {
    pub fn from_settings(id: &str, name: &str, group: &str, s: &DevelopSettings, groups: &[SettingsGroup]) -> Preset {
        Preset { id: id.into(), name: name.into(), group: group.into(), settings: extract_groups(s, groups), favorite: false, builtin: false }
    }
    /// Apply at `amount` (1.0 = 100 %) to a photo whose image is `aspect` = width / height
    /// (oriented), as Lightroom Classic does: the preset's settings are set (only those it has;
    /// everything else stays) and its masks are **added** to the photo's own, their effect scaled
    /// by the amount. Masks are recipes: their AI selections are computed for each photo
    /// afterwards ([`crate::Mask::needs_compute`]), never copied from the photo the preset was
    /// made on.
    ///
    /// The masks the latest preset application added are provisional (tagged with that preset's
    /// id): applying the same preset again right away replaces them (no doubling); applying any
    /// other preset first makes them the photographer's own (untagged), so the next application
    /// of the first preset adds a fresh set. Editing a provisional mask makes it the
    /// photographer's own too ([`settle_provisional`]).
    pub fn apply(&self, s: &DevelopSettings, amount: f64, aspect: f64) -> DevelopSettings {
        let masks: Vec<crate::Mask> = self
            .settings
            .get("masks")
            .and_then(Value::as_array)
            .map(|m| m.iter().filter_map(|m| serde_json::from_value::<crate::Mask>(m.clone()).ok()).collect())
            .unwrap_or_default();
        let mut rest = self.settings.clone();
        if let Some(o) = rest.as_object_mut() {
            o.remove("masks");
        }
        let mut out = apply_partial(s, &rest, amount);
        // the same preset again: its own (still provisional) masks make room for the new ones
        let again = out.masks.iter().any(|m| m.preset.as_deref() == Some(self.id.as_str()));
        if again {
            out.masks.retain(|m| m.preset.as_deref() != Some(self.id.as_str()));
        }
        // whatever an earlier application left is the photographer's now
        for m in &mut out.masks {
            m.preset = None;
        }
        let mut next = s.masks.iter().chain(&out.masks).map(|m| m.id).max().unwrap_or(0);
        // named after the preset: "Sun Kissed", or "Dress + Portrait - Clothes" for one of several
        let short = crate::recipe::short_preset_name(&self.name);
        let several = masks.len() > 1;
        for mut m in masks {
            let own = if crate::recipe::generic_name(&m.name) { crate::recipe::content_name(&m) } else { m.name.trim().to_string() };
            m.name = match (short.is_empty(), several) {
                (true, _) => own,
                (false, true) => format!("{short} - {own}"),
                (false, false) => short.clone(),
            };
            next = next.saturating_add(1);
            m.id = next;
            m.preset = Some(self.id.clone());
            m.strip_ai(aspect);
            m.adjust.amount = (m.adjust.amount * amount).clamp(0.0, 200.0);
            out.masks.push(m);
        }
        out
    }

    /// Whether it carries masks with AI selections (shown with a badge; applying it computes them).
    pub fn has_ai_masks(&self) -> bool {
        self.settings
            .get("masks")
            .and_then(Value::as_array)
            .is_some_and(|m| m.iter().filter_map(|m| serde_json::from_value::<crate::Mask>(m.clone()).ok()).any(|m| m.uses_ai()))
    }

    /// Whether it has masks and no other settings (a "mask-only" preset such as Bring Sky Down).
    pub fn mask_only(&self) -> bool {
        self.settings.as_object().is_some_and(|o| o.contains_key("masks") && o.len() == 1)
    }
}

/// `partial`'s masks and Upright as recipes: no segmentation computed for one photo, no Object clicks (they
/// point at that photo's pixels: the object is picked again on each photo), no provisional tag, no Upright
/// correction found on one photo, no Guided guides (what presets, copy and paste carry).
pub fn to_recipes(partial: &mut Value) {
    // Upright is a recipe too (test notes 2026-10-24 note 4): the mode travels, each photo finds its own
    // correction; Guided's guides were drawn on one photo's lines and stay there
    if let Some(g) = partial.get_mut("geometry").and_then(Value::as_object_mut) {
        if g.contains_key("upright_transform") {
            g.insert("upright_transform".into(), Value::Null);
        }
        if g.contains_key("guides") {
            g.insert("guides".into(), Value::Array(Vec::new()));
        }
    }
    let Some(masks) = partial.get_mut("masks").and_then(Value::as_array_mut) else { return };
    for m in masks.iter_mut() {
        if let Ok(mut mask) = serde_json::from_value::<crate::Mask>(m.clone()) {
            mask.for_other_photo(crate::recipe::RECIPE_ASPECT);
            mask.preset = None;
            if let Ok(v) = serde_json::to_value(mask) {
                *m = v;
            }
        }
    }
}

/// After an edit of `before` into `after` (not a preset application): when the photographer
/// changed one of the masks the latest preset application added, all of that application's masks
/// become theirs (untagged), so applying that preset again adds rather than replaces.
pub fn settle_provisional(before: &DevelopSettings, after: &mut DevelopSettings) {
    let touched = after.masks.iter().filter(|m| m.preset.is_some()).any(|m| {
        before.masks.iter().find(|b| b.id == m.id).is_some_and(|b| {
            b.preset != m.preset
                || !b.same_recipe(m)
                || b.adjust != m.adjust
                || b.name != m.name
                || b.invert != m.invert
                || b.visible != m.visible
                || b.refine != m.refine
        })
    });
    if touched {
        for m in &mut after.masks {
            m.preset = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_and_apply_roundtrip() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.effects.clarity = 30.0;
        a.crop.geometry.angle = 5.0;
        let p = Preset::from_settings("p", "P", "User", &a, &SettingsGroup::default_copy());
        assert!(p.settings.get("crop").is_none());
        let b = p.apply(&DevelopSettings::default(), 1.0, 1.5);
        assert_eq!(b.light.exposure, 1.0);
        assert_eq!(b.effects.clarity, 30.0);
        assert_eq!(b.crop.geometry.angle, 0.0);
    }

    #[test]
    fn amount_scales_numbers() {
        let base = DevelopSettings::default();
        let partial = json!({"light": {"exposure": 1.0, "contrast": 40}});
        let half = apply_partial(&base, &partial, 0.5);
        assert!((half.light.exposure - 0.5).abs() < 1e-9);
        assert!((half.light.contrast - 20.0).abs() < 1e-9);
        let more = apply_partial(&base, &partial, 2.0);
        assert!((more.light.contrast - 80.0).abs() < 1e-9);
        // clamped through specs
        let lots = apply_partial(&base, &json!({"light": {"exposure": 4.0}}), 2.0);
        assert_eq!(lots.light.exposure, 5.0);
    }

    /// A preset with the Grain group (its seed is an integer) used to do nothing at any amount
    /// other than 100 %: the interpolated seed failed to deserialize and the preset was dropped.
    #[test]
    fn amount_keeps_integer_fields_whole() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.grain.amount = 40.0;
        a.grain.seed = 12345;
        let p = Preset::from_settings("p", "P", "User", &a, &SettingsGroup::default_copy());
        assert!(p.settings.pointer("/grain/seed").is_some_and(Value::is_u64), "the default copy groups include the grain seed");
        let base = DevelopSettings::default();
        for (amount, seed) in [(0.3, 0), (0.5, 12345), (1.5, 12345), (2.0, 12345)] {
            let out = p.apply(&base, amount, 1.5);
            assert!((out.light.exposure - amount.min(5.0)).abs() < 1e-9, "{amount}: exposure {}", out.light.exposure);
            assert!((out.grain.amount - (40.0 * amount).min(100.0)).abs() < 1e-9, "{amount}: grain {}", out.grain.amount);
            assert_eq!(out.grain.seed, seed, "{amount}: seed");
        }
    }

    /// Every group with single settings is covered by them: all ticked = the whole group.
    #[test]
    fn group_items_cover_every_key() {
        let mut s = DevelopSettings::default();
        s.hdr.enabled = true;
        s.camera_profile = "dcp:x".into();
        s.skin.amount = 10.0;
        let full = s.to_json();
        for g in SettingsGroup::ALL {
            if g.items().is_empty() {
                continue;
            }
            let whole = extract_groups(&s, &[g]);
            let pick = SettingsPick { groups: vec![], settings: g.items().iter().map(|i| i.id.to_string()).collect() };
            assert_eq!(pick.clone().normalized().groups, vec![g], "{g:?}");
            let by_items = SettingsPick { groups: vec![], settings: pick.settings }.extract(&s);
            assert_eq!(whole, by_items, "{g:?}: the settings miss a key of the group");
            for it in g.items() {
                assert!(it.id.starts_with(&format!("{}.", g.id())), "{}", it.id);
                for p in it.paths {
                    assert!(path_in(&full, p) || ["camera_profile", "retouch"].contains(p), "{}: {p} isn't in the settings", it.id);
                }
            }
        }
        for g in SettingsGroup::ALL {
            assert_eq!(serde_json::to_value(g).unwrap(), json!(g.id()));
            assert_eq!(SettingsGroup::from_id(g.id()), Some(g));
        }
    }

    /// Client note 8a: Light without Exposure leaves each photo's exposure alone.
    #[test]
    fn single_settings_travel_alone() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.5;
        a.light.contrast = 30.0;
        a.detail.sharpen_amount = 80.0;
        a.detail.nr_luminance = 40.0;
        let pick = SettingsPick::from_ids(&["light.contrast", "detail.sharpening"]);
        assert_eq!(pick.settings, vec!["light.contrast".to_string(), "detail.sharpening".to_string()]);
        let p = Preset { settings: pick.extract(&a), ..Preset::from_settings("p", "P", "User", &a, &[]) };
        assert_eq!(
            p.settings,
            json!({"light": {"contrast": 30.0}, "detail": {"sharpen_amount": 80.0, "sharpen_radius": 1.0, "sharpen_detail": 25.0, "sharpen_masking": 0.0}})
        );
        let mut b = DevelopSettings::default();
        b.light.exposure = -0.7;
        b.detail.nr_luminance = 12.0;
        let out = p.apply(&b, 1.0, 1.5);
        assert_eq!((out.light.exposure, out.light.contrast), (-0.7, 30.0));
        assert_eq!((out.detail.sharpen_amount, out.detail.nr_luminance), (80.0, 12.0));
        // what the preset holds, as ticks again
        assert_eq!(SettingsPick::of_partial(&p.settings), pick);
        assert_eq!(pick.to_ids(), vec!["light.contrast".to_string(), "detail.sharpening".to_string()]);
        // a whole group lists every setting as a tick
        let whole = SettingsPick::from_ids(&["effects", "whiteBalance", "nonsense", "light.nope"]);
        assert_eq!(whole.groups, vec![SettingsGroup::WhiteBalance, SettingsGroup::Effects]);
        assert!(whole.settings.is_empty());
        assert_eq!(whole.to_ids(), vec!["whiteBalance", "effects.texture", "effects.clarity", "effects.dehaze"]);
        assert!(whole.has("effects.clarity") && whole.has("whiteBalance") && !whole.has("light.exposure"));
        assert_eq!(whole.intersect(&SettingsPick::from_ids(&["effects.clarity", "light"])), SettingsPick::from_ids(&["effects.clarity"]));
    }

    /// Client note 8b: a preset saved without White Balance never touches a photo's own (as shot or
    /// set by hand), and New Preset leaves it out unless it is ticked.
    #[test]
    fn preset_without_white_balance_keeps_each_photos_own() {
        assert!(!SettingsGroup::default_preset().contains(&SettingsGroup::WhiteBalance));
        assert!(SettingsGroup::default_copy().contains(&SettingsGroup::WhiteBalance));
        let mut src = DevelopSettings::for_raw(3200.0, 12.0);
        src.wb.mode = crate::WbMode::Custom;
        src.wb.scale = crate::WbScale::Kelvin;
        src.light.exposure = 0.4;
        src.calibration.red_hue = 10.0;
        for pick in [SettingsPick::of_groups(SettingsGroup::default_preset()), SettingsPick::from_ids(&["light", "color", "calibration"])] {
            let p = Preset { settings: pick.extract(&src), ..Preset::from_settings("p", "P", "User", &src, &[]) };
            assert!(p.settings.get("wb").is_none(), "{:?}", p.settings);
            for (temp, tint, mode, scale) in [
                (5600.0, 4.0, crate::WbMode::AsShot, crate::WbScale::Kelvin),
                (6500.0, 0.0, crate::WbMode::AsShot, crate::WbScale::Legacy),
                (4100.0, -8.0, crate::WbMode::Custom, crate::WbScale::Kelvin),
            ] {
                let mut photo = DevelopSettings::for_raw(temp, tint);
                photo.wb.mode = mode;
                photo.wb.scale = scale;
                let out = p.apply(&photo, 1.0, 1.5);
                assert_eq!(out.wb, photo.wb);
                assert_eq!(out.light.exposure, 0.4);
                let half = p.apply(&photo, 0.5, 1.5);
                assert_eq!(half.wb, photo.wb);
            }
        }
    }

    #[test]
    fn deep_merge_objects() {
        let mut a = json!({"x": {"a": 1, "b": 2}, "y": [1]});
        deep_merge(&mut a, &json!({"x": {"b": 3}, "y": [2, 3]}));
        assert_eq!(a, json!({"x": {"a": 1, "b": 3}, "y": [2, 3]}));
    }
}
