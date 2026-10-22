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

/// `partial`'s masks as recipes: no segmentation computed for one photo, no Object clicks (they
/// point at that photo's pixels: the object is picked again on each photo), no provisional tag
/// (what presets, copy and paste carry).
pub fn to_recipes(partial: &mut Value) {
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

    #[test]
    fn deep_merge_objects() {
        let mut a = json!({"x": {"a": 1, "b": 2}, "y": [1]});
        deep_merge(&mut a, &json!({"x": {"b": 3}, "y": [2, 3]}));
        assert_eq!(a, json!({"x": {"a": 1, "b": 3}, "y": [2, 3]}));
    }
}
