//! The app-wide store: presets, profiles and effects shared by every library, the merge of each
//! library into it (once, deduplicated, the library left as it was), "Store with this library",
//! and profile ids the store had to change.

use std::path::{Path, PathBuf};

use lightcraft_raw::dcp::{DcpProfile, write_dcp};
use lightcraft_raw::{ColorData, Mat3};
use serde_json::json;

use crate::Session;
use crate::cmd::mask_effects::MaskEffect;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-store-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn dcp(name: &str, gain: f64) -> Vec<u8> {
    let m = Mat3([[gain, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let xyz_to_cam = m.mul(&Mat3([[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]]));
    write_dcp(&DcpProfile {
        name: name.into(),
        camera_model: "LightCraft Test Camera".into(),
        color: ColorData { illuminant: [21, 0], color_matrix: [Some(xyz_to_cam), None], ..Default::default() },
    })
    .unwrap()
}

fn cube(title: &str, lift: f32) -> String {
    let mut c = format!("TITLE \"{title}\"\nLUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                c.push_str(&format!("{} {g} {b}\n", (r as f32 + lift).min(1.0)));
            }
        }
    }
    c
}

/// A library (no store) with a preset, a camera profile, a LUT and an effect.
fn make_library(dir: &Path, files: &Path, preset: &str, dcp_name: &str, gain: f64) {
    std::fs::create_dir_all(files).unwrap();
    let mut s = Session::new();
    s.open_library(dir, true).unwrap();
    s.execute("preset.create", &json!({"name": preset})).unwrap();
    let f = files.join(format!("{dcp_name}.dcp"));
    std::fs::write(&f, dcp(dcp_name, gain)).unwrap();
    let l = files.join("Warm.cube");
    std::fs::write(&l, cube("Warm", 0.1)).unwrap();
    s.execute("profile.import", &json!({"paths": [f.to_string_lossy(), l.to_string_lossy()]})).unwrap();
    s.mask_effects.push(MaskEffect { name: format!("{preset} Skin"), ..Default::default() });
    s.save_prefs().unwrap();
    s.close_library().unwrap();
}

fn with_store(store: &Path) -> Session {
    Session::new().with_app_store(store)
}

#[test]
fn libraries_are_merged_once_and_share_everything() {
    let root = temp_dir("merge");
    let (a, b, c, store) = (root.join("A"), root.join("B"), root.join("C"), root.join("store"));
    make_library(&a, &root.join("fa"), "Alex Warm", "Body 71", 1.0);
    make_library(&b, &root.join("fb"), "Sam Cool", "Body 74", 0.9);
    let a_presets = std::fs::read(a.join("presets.json")).unwrap();
    let a_prefs = std::fs::read(a.join("prefs.json")).unwrap();

    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    assert!(s.uses_app_store());
    s.open_library(&b, false).unwrap();
    let info = s.execute("store.info", &json!({})).unwrap();
    assert_eq!(info["merged"].as_array().unwrap().len(), 2, "{info}");
    let users: Vec<&str> = s.presets.iter().filter(|p| !p.builtin).map(|p| p.name.as_str()).collect();
    assert!(users.contains(&"Alex Warm") && users.contains(&"Sam Cool"), "{users:?}");
    assert_eq!(s.dcp_profiles.len(), 2);
    // the same LUT in both libraries is kept once
    assert_eq!(s.lut_profiles.len(), 1);
    assert_eq!(s.mask_effects.len(), 2);
    // the profile files are the store's copies
    for e in &s.dcp_profiles {
        assert!(Path::new(&e.file).starts_with(store.join("Profiles")), "{}", e.file);
        assert!(Path::new(&e.file).is_file());
    }
    // a preset made now goes to the store, and library A's files stay as they were
    s.open_library(&a, false).unwrap();
    s.execute("preset.create", &json!({"name": "Made Later"})).unwrap();
    s.close_library().unwrap();
    assert_eq!(std::fs::read(a.join("presets.json")).unwrap(), a_presets, "library presets untouched");
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(a.join("prefs.json")).unwrap()).unwrap();
    let before: serde_json::Value = serde_json::from_slice(&a_prefs).unwrap();
    for k in ["dcpProfiles", "lutProfiles", "maskEffects"] {
        assert_eq!(after[k], before[k], "{k} kept as the library had it");
    }
    assert!(after.get("profiles").is_none(), "the Profile Manager's choices are the store's");
    drop(s);

    // a new library on another day sees everything (a fresh process)
    let mut s = with_store(&store);
    s.open_library(&c, false).unwrap();
    let names: Vec<&str> = s.presets.iter().filter(|p| !p.builtin).map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Made Later") && names.contains(&"Sam Cool"), "{names:?}");
    assert_eq!(s.dcp_profiles.len(), 2);
    assert!(s.media.camera_profiles.len() == 2, "camera profiles registered with the decoder");
    // opening A again merges nothing new
    s.open_library(&a, false).unwrap();
    assert_eq!(s.dcp_profiles.len(), 2);
    assert_eq!(s.presets.iter().filter(|p| p.name == "Alex Warm").count(), 1);
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn libraries_not_open_are_merged_by_command_and_only_read() {
    let root = temp_dir("cmd");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    make_library(&a, &root.join("fa"), "Alex Warm", "Body 71", 1.0);
    make_library(&b, &root.join("fb"), "Sam Cool", "Body 74", 0.9);
    let listing = |d: &Path| {
        let mut v: Vec<(String, u64)> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.metadata().unwrap().len()))
            .collect();
        v.sort();
        v
    };
    let b_before = listing(&b);
    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    let r = s
        .execute("store.mergeLibraries", &json!({"paths": [a.to_string_lossy(), b.to_string_lossy(), root.join("nope").to_string_lossy()]}))
        .unwrap();
    let merged = r["merged"].as_array().unwrap();
    assert_eq!(merged[0]["skipped"], true, "A was merged when it opened");
    assert_eq!(merged[1]["presets"], 1);
    assert!(!merged[2]["problems"].as_array().unwrap().is_empty());
    assert_eq!(listing(&b), b_before, "B is only read");
    // again: nothing
    let r = s.execute("store.mergeLibraries", &json!({"paths": [b.to_string_lossy()]})).unwrap();
    assert_eq!(r["merged"][0]["skipped"], true);
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_profile_id_taken_by_another_file_moves_the_librarys_photos() {
    let root = temp_dir("remap");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    // two libraries, a camera profile of the same name (same id) but different content
    make_library(&a, &root.join("fa"), "Alex", "House", 1.0);
    make_library(&b, &root.join("fb"), "Sam", "House", 0.7);
    let id = {
        let mut s = Session::new();
        s.open_library(&b, false).unwrap();
        let id = s.dcp_profiles[0].id.clone();
        s.execute("develop.cameraProfile", &json!({"id": id})).unwrap();
        s.close_library().unwrap();
        id
    };
    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    s.open_library(&b, false).unwrap();
    assert_eq!(s.dcp_profiles.len(), 2);
    let moved = s.dcp_profiles.iter().find(|e| e.id != id).unwrap().id.clone();
    let active = s.active().unwrap();
    assert_eq!(s.catalog.photo(active).unwrap().develop.calibration_id(), Some(moved.as_str()), "B's photo follows its own profile");
    s.close_library().unwrap();
    drop(s);
    // only once
    let mut s = with_store(&store);
    s.open_library(&b, false).unwrap();
    assert_eq!(s.catalog.photo(active).unwrap().develop.calibration_id(), Some(moved.as_str()));
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn store_with_this_library_and_back() {
    let root = temp_dir("own");
    let (a, c, store) = (root.join("A"), root.join("C"), root.join("store"));
    make_library(&a, &root.join("fa"), "Alex Warm", "Body 71", 1.0);
    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    s.open_library(&c, true).unwrap();
    let r = s.execute("store.useLibrary", &json!({"keep": true})).unwrap();
    assert_eq!(r["changed"], true, "{r}");
    assert!(!s.uses_app_store());
    // C now has its own complete copy
    assert!(c.join("presets.json").is_file());
    assert!(c.join("Profiles").read_dir().unwrap().count() >= 2);
    s.execute("preset.create", &json!({"name": "Only Here"})).unwrap();
    s.close_library().unwrap();
    drop(s);
    let store_presets = std::fs::read_to_string(store.join("presets.json")).unwrap();
    assert!(!store_presets.contains("Only Here"));
    // reopened: still its own
    let mut s = with_store(&store);
    s.open_library(&c, false).unwrap();
    assert!(!s.uses_app_store());
    assert!(s.presets.iter().any(|p| p.name == "Only Here"));
    // back to the shared store: its own join it
    s.execute("store.useLibrary", &json!({"keep": false})).unwrap();
    assert!(s.uses_app_store());
    assert!(s.presets.iter().any(|p| p.name == "Only Here") && s.presets.iter().any(|p| p.name == "Alex Warm"));
    s.close_library().unwrap();
    let store_presets = std::fs::read_to_string(store.join("presets.json")).unwrap();
    assert!(store_presets.contains("Only Here"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_damaged_store_file_is_kept_aside_and_never_crashes() {
    let root = temp_dir("damaged");
    let store = root.join("store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("shared.json"), b"{ not json").unwrap();
    std::fs::write(store.join("presets.json"), [0xff, 0xfe, 0x00]).unwrap();
    let mut s = with_store(&store);
    s.open_library(root.join("L"), true).unwrap();
    let w = s.take_library_warnings();
    assert!(w.iter().any(|w| w.contains("shared.json")), "{w:?}");
    assert!(std::fs::read_dir(&store).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("shared.json.corrupt-")));
    s.execute("preset.create", &json!({"name": "After"})).unwrap();
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn import_presets_and_preset_choices_are_shared() {
    let root = temp_dir("importpresets");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    // a library of before the store, with an import preset of its own
    {
        let mut s = Session::new();
        s.open_library(&a, true).unwrap();
        s.execute("import.savePreset", &json!({"name": "Card", "settings": {"mode": "copy"}})).unwrap();
        s.close_library().unwrap();
    }
    let mut s = with_store(&store);
    s.open_library(&a, true).unwrap();
    assert!(s.import_presets.iter().any(|p| p.name == "Card"), "merged into the store");
    let builtin = s.presets.iter().find(|p| p.builtin).unwrap().id.clone();
    s.execute("preset.hide", &json!({"id": builtin})).unwrap();
    s.close_library().unwrap();
    drop(s);
    let mut s = with_store(&store);
    s.open_library(&b, true).unwrap();
    assert!(s.import_presets.iter().any(|p| p.name == "Card"), "a new library has it");
    assert!(s.preset_state.hidden.contains(&builtin));
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// Note 52: the user's way of working is the app's, not a library's. Export presets, a metadata
/// preset, the slideshow's settings, Hold View and the sort made with library A are there in a
/// new library B, and after a restart.
#[test]
fn settings_made_in_library_a_are_there_in_library_b() {
    let root = temp_dir("habits");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    let mut s = with_store(&store);
    s.open_library(&a, true).unwrap();
    s.execute("export.savePreset", &json!({"name": "Client Web 2048", "params": {"format": "jpeg", "longEdge": 2048, "quality": 85}})).unwrap();
    s.execute("metadata.savePreset", &json!({"name": "Studio", "fields": {"creator": "Studio Photographer"}})).unwrap();
    s.execute("slideshow.playback", &json!({"slideDuration": 7.0, "repeat": true})).unwrap();
    s.execute("library.holdView", &json!({"on": true})).unwrap();
    s.execute("library.sort", &json!({"key": "fileName", "ascending": true})).unwrap();
    let slideshow = s.slideshow.clone();
    s.close_library().unwrap();

    let check = |s: &mut Session, when: &str| {
        let names: Vec<String> = s
            .execute("export.presets", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect();
        assert!(names.iter().any(|n| n == "Client Web 2048"), "{when}: {names:?}");
        assert!(s.metadata_presets.iter().any(|m| m.name == "Studio"), "{when}");
        assert_eq!(s.slideshow, slideshow, "{when}");
        assert!(s.hold_view, "{when}");
        assert_eq!((s.sort.key, s.sort.ascending), (lightcraft_catalog::SortKey::FileName, true), "{when}");
    };
    s.open_library(&b, true).unwrap();
    check(&mut s, "library B");
    s.close_library().unwrap();
    drop(s);
    // a restart, straight into library B
    let mut s = with_store(&store);
    s.open_library(&b, false).unwrap();
    check(&mut s, "after a restart");
    // a change made in B is A's too
    s.execute("export.deletePreset", &json!({"name": "Client Web 2048"})).unwrap();
    s.close_library().unwrap();
    s.open_library(&a, false).unwrap();
    assert!(!s.export_presets.iter().any(|p| p.name == "Client Web 2048"), "deleted everywhere");
    let _ = std::fs::remove_dir_all(&root);
}

/// The migration: a library whose export presets an older build kept with it, merged into the store
/// by that build (before the store held them), gets them merged once on open: nothing lost, nothing
/// twice, a same-named different preset kept as "name 2", the library's files left as they were.
#[test]
fn export_presets_kept_with_an_older_library_join_the_store_once() {
    let root = temp_dir("habits-migrate");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    for (dir, quality) in [(&a, 80), (&b, 60)] {
        let mut s = Session::new();
        s.open_library(dir, true).unwrap();
        s.execute("export.savePreset", &json!({"name": "Proof", "params": {"format": "jpeg", "quality": quality}})).unwrap();
        s.execute("export.savePreset", &json!({"name": "Album", "params": {"format": "tiff"}})).unwrap();
        s.save_prefs().unwrap();
        s.close_library().unwrap();
    }
    let a_prefs = std::fs::read(a.join("prefs.json")).unwrap();
    // the store as an older build left it: both libraries merged, without their export presets
    std::fs::create_dir_all(&store).unwrap();
    let merged = |d: &Path| json!({"library": d.display().to_string(), "at": "2026-10-01T10:00:00"});
    std::fs::write(store.join("shared.json"), serde_json::to_vec(&json!({"version": 1, "merged": [merged(&a), merged(&b)]})).unwrap()).unwrap();

    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    let names = |s: &Session| -> Vec<String> { s.export_presets.iter().map(|p| p.name.clone()).collect() };
    assert_eq!(names(&s), ["Proof", "Album"]);
    s.open_library(&b, false).unwrap();
    assert_eq!(names(&s), ["Proof", "Album", "Proof 2"], "B's Proof differs: kept as Proof 2; the same Album once");
    assert_eq!(s.export_presets.iter().find(|p| p.name == "Proof 2").unwrap().params["quality"], 60);
    s.open_library(&a, false).unwrap();
    s.open_library(&b, false).unwrap();
    assert_eq!(names(&s).len(), 3, "merged once");
    s.close_library().unwrap();
    assert_eq!(std::fs::read(a.join("prefs.json")).unwrap(), a_prefs, "the library's copy stays as it was, for older builds");
    let shared: serde_json::Value = serde_json::from_slice(&std::fs::read(store.join("shared.json")).unwrap()).unwrap();
    assert_eq!(shared["exportPresets"].as_array().unwrap().len(), 3);
    assert!(shared["merged"].as_array().unwrap().iter().all(|m| m["habits"] == true));
    let _ = std::fs::remove_dir_all(&root);
}

/// Notes 48 and 52 together: the app-wide background preparation settings live in the install
/// store (library B sees what was set in A, also after a restart), while a library's own settings
/// stay in that library's `prefs.json`, win only while it is open, and are not frozen like the
/// store's copies there (a change to them is written).
#[test]
fn background_prep_is_app_wide_with_a_per_library_override() {
    let root = temp_dir("prep");
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    let mut s = with_store(&store);
    s.open_library(&a, true).unwrap();
    s.execute("prefs.setBackgroundPrep", &json!({"settings": {"library": {"ahead": 10}}})).unwrap();
    s.execute("prefs.setBackgroundPrep", &json!({"library": true, "settings": {"culling": {"ahead": 0}}})).unwrap();
    assert_eq!(s.prep_settings().culling.ahead, 0, "A's own");
    s.close_library().unwrap();
    let prefs: serde_json::Value = serde_json::from_slice(&std::fs::read(a.join("prefs.json")).unwrap()).unwrap();
    assert_eq!(prefs["prep"]["culling"]["ahead"], 0, "A's override is in A's prefs.json: {prefs}");

    s.open_library(&b, true).unwrap();
    assert_eq!(s.prep_settings().library.ahead, 10, "the app-wide change reached B");
    assert_eq!(s.prep_settings().culling.ahead, 5, "A's own settings stay A's");
    s.close_library().unwrap();
    drop(s);

    let mut s = with_store(&store);
    s.open_library(&a, false).unwrap();
    assert_eq!(s.prep_settings().culling.ahead, 0, "A's own after a restart");
    s.execute("prefs.setBackgroundPrep", &json!({"library": true, "settings": {"culling": {"ahead": 3}}})).unwrap();
    s.close_library().unwrap();
    let prefs: serde_json::Value = serde_json::from_slice(&std::fs::read(a.join("prefs.json")).unwrap()).unwrap();
    assert_eq!(prefs["prep"]["culling"]["ahead"], 3, "a change to the override is written, not frozen");
    s.open_library(&b, false).unwrap();
    assert_eq!(s.prep_settings().library.ahead, 10, "app-wide after a restart");
    let _ = std::fs::remove_dir_all(root);
}
