//! Per-setting picks for presets, copy, paste and sync (client notes 8, 11, 13): single settings of a
//! group, White Balance only when ticked, one group per name, Update Preset with an undo, and every
//! choice remembered app-wide.

use std::path::PathBuf;

use lightcraft_develop::{SettingsGroup, WbMode};
use serde_json::json;

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-picks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Client note 8a: a preset of Light without Exposure leaves each photo's exposure as it was.
#[test]
fn preset_with_single_settings_of_a_group() {
    let mut s = Session::with_demo();
    let vis = s.visible_cloned();
    s.execute("library.select", &json!({"ids": [vis[0].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 1.2, "light.contrast": 25, "light.shadows": 40, "effects.clarity": 15}})).unwrap();
    let r = s
        .execute("preset.create", &json!({"name": "Contrast only", "groups": ["effects"], "settings": ["light.contrast", "light.shadows"]}))
        .unwrap();
    let pid = r["id"].as_str().unwrap().to_string();
    let p = s.presets.iter().find(|p| p.id == pid).unwrap().clone();
    assert_eq!(p.settings["light"], json!({"contrast": 25.0, "shadows": 40.0}), "{}", p.settings);
    assert!(p.settings.get("effects").is_some());
    s.execute("library.select", &json!({"ids": [vis[3].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": -0.5}})).unwrap();
    s.execute("preset.apply", &json!({"id": pid})).unwrap();
    let d = s.develop_of(vis[3]).unwrap();
    assert_eq!((d.light.exposure, d.light.contrast, d.light.shadows, d.effects.clarity), (-0.5, 25.0, 40.0, 15.0));
    // the ticks the preset holds, for Update Preset's dialog
    let held = s.execute("preset.settings", &json!({"id": pid})).unwrap();
    assert_eq!(held["settings"], json!(["light.contrast", "light.shadows"]), "{held}");
    assert_eq!(held["groups"], json!(["effects"]));
    // an unknown setting is refused, nothing made
    let n = s.presets.len();
    assert!(s.execute("preset.create", &json!({"name": "X", "settings": ["light.nope"]})).is_err());
    assert!(s.execute("preset.create", &json!({"name": "X", "settings": "light.contrast"})).is_err());
    assert!(s.execute("preset.create", &json!({"name": "X", "groups": [], "settings": []})).is_err(), "nothing ticked");
    assert_eq!(s.presets.len(), n);
}

/// Client note 8b: a new preset leaves White Balance out unless it is ticked, so applying it keeps
/// every photo's own white balance (as shot or set by hand).
#[test]
fn new_preset_leaves_white_balance_alone() {
    let mut s = Session::with_demo();
    let vis = s.visible_cloned();
    let defaults = s.execute("develop.settingItems", &json!({})).unwrap();
    assert!(!defaults["newPreset"]["groups"].as_array().unwrap().contains(&json!("whiteBalance")), "{defaults}");
    assert!(defaults["copy"]["groups"].as_array().unwrap().contains(&json!("whiteBalance")), "{defaults}");
    s.execute("develop.set", &json!({"values": {"wb.temp": 3000, "wb.tint": 20, "light.exposure": 0.3}})).unwrap();
    let pid = s.execute("preset.create", &json!({"name": "Warm look"})).unwrap()["id"].as_str().unwrap().to_string();
    let p = s.presets.iter().find(|p| p.id == pid).unwrap();
    assert!(p.settings.get("wb").is_none(), "{}", p.settings);
    // photos with their own as-shot and custom white balance
    s.execute("library.select", &json!({"ids": [vis[1].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"wb.temp": 7200, "wb.tint": -5}})).unwrap();
    let before: Vec<_> = [vis[1], vis[2]].iter().map(|id| s.develop_of(*id).unwrap().wb).collect();
    assert_eq!(before[1].mode, WbMode::AsShot);
    s.execute("library.select", &json!({"ids": [vis[1].0, vis[2].0]})).unwrap();
    s.execute("preset.apply", &json!({"id": pid})).unwrap();
    for (id, wb) in [vis[1], vis[2]].iter().zip(&before) {
        let d = s.develop_of(*id).unwrap();
        assert_eq!(d.wb, *wb, "white balance changed by a preset without it");
        assert_eq!(d.light.exposure, 0.3);
    }
    // ticked, it travels
    s.execute("library.select", &json!({"ids": [vis[0].0]})).unwrap();
    let with = s.execute("preset.create", &json!({"name": "Warm WB", "groups": ["whiteBalance"]})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(s.presets.iter().find(|p| p.id == with).unwrap().settings.get("wb").is_some());
}

/// Client note 8c: a group typed like an existing one (case, spaces) is that group.
#[test]
fn a_typed_group_is_the_existing_one() {
    let mut s = Session::with_demo();
    let a = s.execute("preset.create", &json!({"name": "One", "group": "Wedding Looks"})).unwrap();
    let b = s.execute("preset.create", &json!({"name": "Two", "group": "  wedding looks "})).unwrap();
    let c = s.execute("preset.create", &json!({"name": "Two", "group": "WEDDING LOOKS"})).unwrap();
    assert_eq!(a["group"], "Wedding Looks");
    assert_eq!(b["group"], "Wedding Looks");
    assert_eq!(c["group"], "Wedding Looks");
    assert_ne!(b["id"], c["id"], "the same name twice gets its own id");
    let groups: std::collections::BTreeSet<&str> = s.presets.iter().filter(|p| !p.builtin).map(|p| p.group.as_str()).collect();
    assert_eq!(groups.into_iter().collect::<Vec<_>>(), vec!["Wedding Looks"]);
    // remembered for the next New Preset
    assert_eq!(s.dialogs.preset_group, "Wedding Looks");
}

/// Client note 11: Update with Current Settings keeps name, group, place and marks; Undo puts the old
/// settings back.
#[test]
fn update_preset_keeps_its_place_and_can_be_undone() {
    let mut s = Session::with_demo();
    s.execute("develop.set", &json!({"values": {"light.exposure": 0.5, "light.contrast": 10, "color.vibrance": 5}})).unwrap();
    let pid =
        s.execute("preset.create", &json!({"name": "Bright", "group": "Mine", "settings": ["light.exposure", "color.vibrance"]})).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    s.execute("preset.create", &json!({"name": "After", "group": "Mine", "groups": ["light"]})).unwrap();
    s.execute("preset.favorite", &json!({"id": pid, "favorite": true})).unwrap();
    let place = s.presets.iter().position(|p| p.id == pid).unwrap();
    let old = s.presets.iter().find(|p| p.id == pid).unwrap().clone();
    s.execute("develop.set", &json!({"values": {"light.exposure": 1.5, "light.contrast": 60, "color.vibrance": 30}})).unwrap();
    // default: the settings it held, with the current values
    let r = s.execute("preset.update", &json!({"id": pid})).unwrap();
    assert_eq!(r["settings"], json!(["light.exposure", "color.vibrance"]), "{r}");
    let p = s.presets.iter().find(|p| p.id == pid).unwrap().clone();
    assert_eq!(p.settings, json!({"light": {"exposure": 1.5}, "color": {"vibrance": 30.0}}));
    assert_eq!((p.name.as_str(), p.group.as_str(), p.favorite), ("Bright", "Mine", true));
    assert_eq!(s.presets.iter().position(|p| p.id == pid), Some(place));
    s.execute("preset.undoUpdate", &json!({})).unwrap();
    assert_eq!(*s.presets.iter().find(|p| p.id == pid).unwrap(), old);
    assert!(s.execute("preset.undoUpdate", &json!({})).is_err(), "once");
    // the ticks given replace what it holds
    s.execute("preset.update", &json!({"id": pid, "settings": ["light.contrast"]})).unwrap();
    assert_eq!(s.presets.iter().find(|p| p.id == pid).unwrap().settings, json!({"light": {"contrast": 60.0}}));
    assert!(s.execute("preset.update", &json!({"id": "lc.moody"})).is_err(), "built-in presets stay");
}

/// Client note 13: Copy Settings' single settings, and Paste Selected of part of them.
#[test]
fn copy_and_paste_single_settings() {
    let mut s = Session::with_demo();
    let vis = s.visible_cloned();
    s.execute("library.select", &json!({"ids": [vis[0].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 1.0, "light.whites": 30, "detail.sharpenAmount": 90, "detail.nrLuminance": 35}}))
        .unwrap();
    let r = s.execute("develop.copy", &json!({"settings": ["light.whites", "detail.sharpening", "detail.noise"]})).unwrap();
    assert_eq!(r["settings"], json!(["light.whites", "detail.sharpening", "detail.noise"]), "{r}");
    s.execute("library.select", &json!({"ids": [vis[1].0]})).unwrap();
    s.execute("develop.paste", &json!({"settings": ["detail.sharpening", "light.exposure"]})).unwrap();
    let d = s.develop_of(vis[1]).unwrap();
    assert_eq!((d.light.exposure, d.light.whites, d.detail.sharpen_amount, d.detail.nr_luminance), (0.0, 0.0, 90.0, 0.0));
    s.execute("develop.paste", &json!({})).unwrap();
    let d = s.develop_of(vis[1]).unwrap();
    assert_eq!((d.light.exposure, d.light.whites, d.detail.nr_luminance), (0.0, 30.0, 35.0));
    // Sync remembers its own choice
    s.execute("library.select", &json!({"ids": [vis[0].0, vis[2].0], "active": vis[0].0})).unwrap();
    s.execute("develop.sync", &json!({"settings": ["light.exposure"]})).unwrap();
    assert_eq!(s.develop_of(vis[2]).unwrap().light.exposure, 1.0);
    assert_eq!(s.develop_of(vis[2]).unwrap().light.whites, 0.0);
    assert_eq!(s.dialogs.sync.settings, vec!["light.exposure".to_string()]);
    assert!(s.dialogs.copy.has("detail.noise"));
}

/// Client note 13: the ticks of Copy, Sync and New Preset and the other dialogs' answers are the same
/// in every library and after a restart (the app store), and a damaged entry can't break anything.
#[test]
fn dialog_choices_are_remembered_app_wide() {
    let root = temp_dir("remember");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    let mut s = Session::new().with_app_store(&store);
    s.open_library(&a, true).unwrap();
    s.dialogs.copy = lightcraft_develop::SettingsPick::from_ids(&["light.exposure", "toneCurve"]);
    s.execute("develop.settingItems", &json!({})).unwrap();
    s.execute("dialog.remember", &json!({"dialog": "cameraProfile", "choices": {"light": "daylight", "type": "dcp"}})).unwrap();
    s.save_prefs().unwrap();
    s.close_library().unwrap();
    s.open_library(&b, true).unwrap();
    assert!(s.dialogs.copy.has("light.exposure") && s.dialogs.copy.has("toneCurve") && !s.dialogs.copy.has("light.contrast"));
    assert_eq!(s.execute("dialog.recall", &json!({"dialog": "cameraProfile"})).unwrap()["type"], "dcp");
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new().with_app_store(&store);
    assert!(s.dialogs.copy.has("light.exposure"), "after a restart");
    assert_eq!(s.execute("dialog.recall", &json!({"dialog": "nothing"})).unwrap(), serde_json::Value::Null);
    assert!(s.execute("dialog.remember", &json!({"dialog": "big", "choices": "x".repeat(70_000)})).is_err());
    s.execute("dialog.remember", &json!({"dialog": "cameraProfile", "choices": null})).unwrap();
    assert_eq!(s.execute("dialog.recall", &json!({"dialog": "cameraProfile"})).unwrap(), serde_json::Value::Null);
    drop(s);
    // a hand-edited store: unknown ids and groups are dropped, nothing panics
    let shared = store.join(crate::app_store::SHARED);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&shared).unwrap()).unwrap();
    v["dialogs"] = json!({"copy": {"groups": ["light"], "settings": ["light.exposure", "bogus", 7]}, "newPreset": {"settings": ["light.nope"]}, "presetGroup": "  "});
    std::fs::write(&shared, serde_json::to_vec(&v).unwrap()).unwrap();
    let s = Session::new().with_app_store(&store);
    // (a field that can't be read leaves the whole dialog memory at its defaults)
    assert!(s.dialogs.copy.groups.contains(&SettingsGroup::Light) || s.dialogs.copy == crate::app_store::DialogMemory::default().copy);
    assert!(!s.dialogs.preset_group.trim().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
