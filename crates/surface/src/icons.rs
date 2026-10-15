//! The icon pack shown on LCD keys and in the on-screen readout, and short key labels.
//!
//! Portions © RAD Films - Roman Alurkoff (from TactileBridge), MIT: the icons are TactileBridge's
//! hand-drawn pack (`assets/surface-icons`, renamed to kebab-case), the key labels its
//! `deploy_captureone_plugin.ps1` labels.
//!
//! The SVGs are compiled into native builds only (the web build has no surfaces).

use serde_json::Value;

use crate::binding::{Binding, Modifier};
use crate::tactilebridge::{ADJUSTMENTS, COMMANDS, binding_of};

#[cfg(not(target_arch = "wasm32"))]
#[path = "icon_table.rs"]
mod table;

#[cfg(not(target_arch = "wasm32"))]
fn icons() -> &'static [(&'static str, &'static str)] {
    table::ICONS
}

#[cfg(target_arch = "wasm32")]
fn icons() -> &'static [(&'static str, &'static str)] {
    &[]
}

/// The SVG of an icon.
pub fn svg(name: &str) -> Option<&'static str> {
    let all = icons();
    all.binary_search_by(|(n, _)| (*n).cmp(name)).ok().and_then(|i| all.get(i)).map(|(_, s)| *s)
}

/// Every icon name.
pub fn names() -> impl Iterator<Item = &'static str> {
    icons().iter().map(|(n, _)| *n)
}

/// `Rate5Stars` → `rate-5-stars`, `BWRed` → `bw-red` (the icon file names).
pub fn kebab(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let next = chars.get(i + 1).copied();
        let boundary = match prev {
            None => false,
            Some(p) => {
                (c.is_ascii_uppercase() && (p.is_ascii_lowercase() || p.is_ascii_digit()))
                    || (c.is_ascii_uppercase() && p.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase()))
                    || (c.is_ascii_digit() && p.is_ascii_alphabetic())
            }
        };
        if boundary {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The TactileBridge key a binding came from (the first that maps to it).
fn tb_key(b: &Binding) -> Option<&'static str> {
    let keys: &[&'static str] = if matches!(b, Binding::Value { .. }) { ADJUSTMENTS } else { COMMANDS };
    keys.iter().copied().find(|k| binding_of(k).as_ref() == Some(b))
}

/// The icon of a binding, if the pack has one.
pub fn icon_for(b: &Binding) -> Option<&'static str> {
    let name: String = match b {
        Binding::Value { target } => {
            if let Some(k) = tb_key(b) {
                kebab(k)
            } else if let Some(field) = target.strip_prefix("mask.") {
                // a mask slider shows its global slider's icon
                let global = match field {
                    "exposure" | "contrast" | "highlights" | "shadows" | "whites" | "blacks" => format!("light.{field}"),
                    "temp" => "wb.temp".into(),
                    "tint" => "wb.tint".into(),
                    "clarity" | "texture" => format!("effects.{field}"),
                    "saturation" => "color.saturation".into(),
                    "amount" => return svg_name("layer-opacity"),
                    _ => return None,
                };
                return icon_for(&Binding::value(&global));
            } else {
                match target.as_str() {
                    "ui.brushFeather" => "brush-hardness".into(),
                    "ui.removeFeather" | "ui.removeOpacity" => "spot-radius".into(),
                    "crop.angle" => "straighten-tool".into(),
                    "color.vibrance" => "saturation".into(),
                    "effects.dehaze" => "clarity".into(),
                    _ => return None,
                }
            }
        }
        Binding::Command { .. } => match tb_key(b) {
            Some(k) => kebab(k),
            None => match b {
                Binding::Command { command, .. } => match command.as_str() {
                    "photo.flag" | "photo.pick" => "pick-variant".into(),
                    "photo.reject" => "filter-rejected".into(),
                    "tool.done" => "pick-variant".into(),
                    "tool.brush" => "draw-mask-tool".into(),
                    "crop.autoStraighten" => "straighten-tool".into(),
                    "crop.reset" => "reset-adjustments".into(),
                    "view.cropOverlayOrientation" => "switch-crop-orientation".into(),
                    "view.maskOverlay" => "toggle-overlay".into(),
                    "view.visualizeSpots" => "toggle-focus-mask".into(),
                    "spot.delete" => "remove-layer".into(),
                    "mask.visible" => "toggle-layer-on-off".into(),
                    "mask.add" => "new-empty-layer".into(),
                    "surface.dialogConfirm" => "pick-variant".into(),
                    "surface.dialogCancel" => "deselect-all".into(),
                    _ => return None,
                },
                _ => return None,
            },
        },
        Binding::PageNext { .. } => "next-set".into(),
        Binding::PagePrevious { .. } => "previous-set".into(),
        Binding::Page { .. } | Binding::Profile { .. } | Binding::ProfileNext => "toggle-tools".into(),
        Binding::Reset => "reset-adjustments".into(),
        Binding::ApplyToSelected => "auto-sync".into(),
        Binding::FollowContext => "toggle-tools".into(),
        Binding::Modifier { modifier: Modifier::Fine } => "decrease-zoom".into(),
        Binding::Modifier { modifier: Modifier::Coarse } => "increase-zoom".into(),
    };
    svg_name(&name)
}

fn svg_name(name: &str) -> Option<&'static str> {
    // the static name, so callers can keep it
    let all = icons();
    all.binary_search_by(|(n, _)| (*n).cmp(name)).ok().and_then(|i| all.get(i)).map(|(n, _)| *n)
}

/// TactileBridge's short labels for keys whose split name is too long for a key.
fn tb_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "Zoom100" => "Zoom 100%",
        "Zoom200" => "Zoom 200%",
        "Zoom400" => "Zoom 400%",
        "ZoomToFit" => "Zoom To Fit",
        "ToggleBeforeAfter" => "Before / After",
        "ToggleExposureWarn" => "Clipping",
        "ToggleLayerOnOff" => "Mask On/Off",
        "ToggleSelectionPoints" => "Mask Pins",
        "ResetAdjustments" => "Reset",
        "ApplyAdjustments" => "Paste",
        "CopyAdjustments" => "Copy",
        "CopyAndApplyAdj" => "Sync",
        "AutoAdjust" => "Auto",
        "WBPickerTool" => "WB Picker",
        "SpotRemovalTool" => "Remove",
        "LinearGradientTool" => "Linear Grad",
        "RadialGradientTool" => "Radial Grad",
        "DrawMaskTool" => "Brush",
        "EraseMaskTool" => "Erase",
        "CropTool" => "Crop",
        "MoveToTrash" | "Delete" => "Delete",
        "ClearRating" => "No Rating",
        "Rate1Star" => "1 Star",
        "Rate2Stars" => "2 Stars",
        "Rate3Stars" => "3 Stars",
        "Rate4Stars" => "4 Stars",
        "Rate5Stars" => "5 Stars",
        "ColorTagNone" => "No Label",
        "ColorTagRed" => "Red",
        "ColorTagYellow" => "Yellow",
        "ColorTagGreen" => "Green",
        "ColorTagBlue" => "Blue",
        "ColorTagPurple" => "Purple",
        "PreviousImage" => "Previous",
        "NextImage" => "Next",
        "BWEnable" => "B&W",
        "NewEmptyLayer" => "New Mask",
        "SelectLayerUp" => "Mask Above",
        "SelectLayerDown" => "Mask Below",
        "Process" => "Export",
        "EditAllSelected" => "Auto Sync",
        "DirectColorEditor" => "Color",
        "LoupeTool" => "Zoom 1:1",
        "SwitchCropOrientation" => "Crop Orient.",
        "AutoSync" => "Apply to All",
        _ => return None,
    })
}

/// `RotateLeft` → `Rotate Left`.
fn split_words(key: &str) -> String {
    kebab(key)
        .split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A short English key label for a command (the host prefers the registry's label when this
/// only repeats the id).
pub fn command_label(command: &str, params: &Value) -> String {
    let b = Binding::Command { command: command.to_string(), params: params.clone() };
    if let Some(k) = tb_key(&b) {
        return tb_label(k).map_or_else(|| split_words(k), str::to_string);
    }
    match command {
        "tool.done" => "Done".into(),
        "surface.dialogConfirm" => "OK".into(),
        "surface.dialogCancel" => "Cancel".into(),
        "surface.maskPrevious" => "Mask Above".into(),
        "surface.maskNext" => "Mask Below".into(),
        "mask.add" => match params.get("kind").and_then(Value::as_str) {
            Some("subject") => "Subject".into(),
            Some("sky") => "Sky".into(),
            Some("luminanceRange") => "Luminance".into(),
            Some(k) => split_words(k),
            None => "New Mask".into(),
        },
        "tool.brush" if params.get("op").and_then(Value::as_str) == Some("subtract") => "Erase".into(),
        "tool.brush" if params.get("new").and_then(Value::as_bool) == Some(true) => "New Brush".into(),
        _ => command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kebab_matches_the_file_names() {
        assert_eq!(kebab("Rate5Stars"), "rate-5-stars");
        assert_eq!(kebab("BWRed"), "bw-red");
        assert_eq!(kebab("NRLuminance"), "nr-luminance");
        assert_eq!(kebab("Zoom100"), "zoom-100");
        assert_eq!(kebab("CEBlueHue"), "ce-blue-hue");
        assert_eq!(kebab("SnapTo90"), "snap-to-90");
        assert_eq!(kebab("ISOUp"), "iso-up");
        assert_eq!(kebab(""), "");
    }

    #[test]
    fn the_pack_is_embedded_sorted_and_found() {
        let all: Vec<_> = names().collect();
        assert!(all.len() > 300, "{}", all.len());
        assert!(all.windows(2).all(|w| w[0] < w[1]), "sorted for binary search");
        assert!(svg("exposure").is_some_and(|s| s.contains("<svg")));
        assert!(svg("keystone-tool").is_none(), "flagged icons are left out");
        assert_eq!(icon_for(&Binding::value("light.exposure")), Some("exposure"));
        assert_eq!(icon_for(&Binding::value("mask.exposure")), Some("exposure"));
        assert_eq!(icon_for(&Binding::value("wb.temp")), Some("kelvin"));
        assert_eq!(icon_for(&Binding::command_with("photo.rate", json!({"rating": 5}))), Some("rate-5-stars"));
        assert_eq!(icon_for(&Binding::command("edit.undo")), Some("undo"));
        assert_eq!(icon_for(&Binding::ApplyToSelected), Some("auto-sync"));
        // every built-in key either has an icon or a label
        for p in crate::defaults::profiles() {
            for b in p.pages.iter().flat_map(|pg| pg.bindings.values()) {
                assert!(icon_for(b).is_some() || !b.describe().is_empty());
            }
        }
    }

    #[test]
    fn labels() {
        assert_eq!(command_label("photo.rate", &json!({"rating": 5})), "5 Stars");
        assert_eq!(command_label("photo.rotateLeft", &Value::Null), "Rotate Left");
        assert_eq!(command_label("mask.add", &json!({"kind": "sky"})), "Sky");
        assert_eq!(command_label("app.whatever", &Value::Null), "app.whatever");
    }
}
