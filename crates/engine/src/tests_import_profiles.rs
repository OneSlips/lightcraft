//! Camera profiles at import (LR-PROF-IMPORT-APPLY): the review's per-body summary, automatic
//! body profiles, per-body and all-photos choices, adopting during import, sidecars winning,
//! duplicates left alone and a look on top. Synthetic DNGs only.

use std::path::{Path, PathBuf};

use lightcraft_camprof::ChartCorners;
use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-importprof-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A chart DNG from body `serial` (none: known by its file name) under `light`.
fn raw(dir: &Path, name: &str, light: &str, serial: Option<&str>) -> String {
    let p = dir.join(name);
    let corners = ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0);
    std::fs::write(&p, crate::chart_profiling::demo_chart_dng_of_body(360, 240, &corners, light, true, serial).unwrap()).unwrap();
    p.to_string_lossy().to_string()
}

fn library(tag: &str) -> (Session, PathBuf) {
    let dir = temp_dir(tag);
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    (s, dir)
}

/// A camera profile made from a chart of body `serial` (not applied to anything).
fn profile(s: &mut Session, dir: &Path, name: &str, light: &str, serial: Option<&str>) -> String {
    let path = raw(dir, name, light, serial);
    s.execute("profile.createFromChart", &json!({"path": path, "illuminant": light, "apply": false})).unwrap()["id"].as_str().unwrap().to_string()
}

fn ids(r: &Value) -> Vec<PhotoId> {
    r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect()
}

/// The camera profile a photo develops with ("" = LightCraft's colour).
fn calibration(s: &Session, id: PhotoId) -> String {
    s.catalog.photo(id).unwrap().develop.calibration_id().filter(|c| c.starts_with("dcp:")).unwrap_or("").to_string()
}

fn sidecar(raw_path: &str) {
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+0.50"/></rdf:RDF></x:xmpmeta>"#;
    std::fs::write(Path::new(raw_path).with_extension("xmp"), x).unwrap();
}

#[test]
fn the_plan_groups_raws_by_serial_and_by_file_name() {
    let (mut s, dir) = library("plan");
    let p71 = profile(&mut s, &dir, "M71.dng", "D55", Some("SN71"));
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    raw(&card, "A1.dng", "D65", Some("SN71"));
    raw(&card, "A2.dng", "D50", Some("SN71"));
    raw(&card, "_SA0001.dng", "A", None);
    let sa2 = raw(&card, "_SA0002.dng", "D75", None);
    sidecar(&sa2);
    let r = s.execute("import.profilePlan", &json!({"paths": [card.to_string_lossy()]})).unwrap();
    assert_eq!((r["photos"].as_u64(), r["keptFromSidecar"].as_u64()), (Some(4), Some(1)), "{r}");
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{r}");
    assert_eq!((rows[0]["body"].as_str(), rows[0]["photos"].as_u64(), rows[0]["profile"].as_str()), (Some("SN71"), Some(2), Some(p71.as_str())));
    assert_eq!((rows[0]["how"].as_str(), rows[0]["own"].as_bool()), (Some("auto"), Some(true)));
    assert_eq!((rows[1]["body"].as_str(), rows[1]["bodyLabel"].as_str(), rows[1]["profile"].clone()), (Some("file:SA"), Some("SA"), Value::Null));
    assert_eq!(rows[1]["fromSidecar"], 1);
    // a choice for the file-named body shows in its row
    let r = s.execute("import.profilePlan", &json!({"paths": [card.to_string_lossy()], "cameraProfile": {"perBody": {"file:SA": p71}}})).unwrap();
    let sa = r["rows"].as_array().unwrap().iter().find(|row| row["body"] == "file:SA").unwrap().clone();
    assert_eq!((sa["profile"].as_str(), sa["how"].as_str(), sa["own"].as_bool()), (Some(p71.as_str()), Some("chosen"), Some(false)));
    let e = s.execute("import.profilePlan", &json!({"paths": [card.to_string_lossy()], "cameraProfile": {"all": "dcp:nope"}})).unwrap_err();
    assert!(e.to_string().contains("unknown camera profile"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn automatic_per_body_and_all_choices_and_the_report() {
    let (mut s, dir) = library("choices");
    let p71 = profile(&mut s, &dir, "M71.dng", "D55", Some("SN71"));
    let p74 = profile(&mut s, &dir, "M74.dng", "A", Some("SN74"));
    // automatic: each body's own, else LightCraft's colour
    let a = raw(&dir, "a71.dng", "D65", Some("SN71"));
    let b = raw(&dir, "b74.dng", "D50", Some("SN74"));
    let c = raw(&dir, "_SA1.dng", "D75", None);
    let r = s.execute("library.import", &json!({"paths": [a, b, c]})).unwrap();
    let got = ids(&r);
    assert_eq!(got.iter().map(|id| calibration(&s, *id)).collect::<Vec<_>>(), [p71.clone(), p74.clone(), String::new()]);
    let counts = &r["cameraProfiles"]["profiles"];
    assert_eq!((counts[&p71]["photos"].as_u64(), counts["builtin"]["photos"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(counts[&p71]["bodies"], json!(["SN71"]));
    // per body: the file-named body takes 71's profile, 74 LightCraft's colour; 71 stays automatic
    let a = raw(&dir, "a71b.dng", "A", Some("SN71"));
    let b = raw(&dir, "b74b.dng", "D55", Some("SN74"));
    let c = raw(&dir, "_SA2.dng", "D65", None);
    let r = s.execute("library.import", &json!({"paths": [a, b, c], "cameraProfile": {"perBody": {"file:SA": p71, "SN74": "builtin"}}})).unwrap();
    let got = ids(&r);
    assert_eq!(got.iter().map(|id| calibration(&s, *id)).collect::<Vec<_>>(), [p71.clone(), String::new(), p71.clone()]);
    // an explicit choice: applying body defaults later leaves it
    s.execute("profile.applyBodyDefaults", &json!({})).unwrap();
    assert_eq!(calibration(&s, got[1]), "", "LightCraft's colour chosen at import stays");
    // one for all
    let a = raw(&dir, "a71c.dng", "D75", Some("SN71"));
    let b = raw(&dir, "b74c.dng", "D65", Some("SN74"));
    let r = s.execute("library.import", &json!({"paths": [a, b], "cameraProfile": {"all": p74}})).unwrap();
    assert!(ids(&r).iter().all(|id| calibration(&s, *id) == p74));
    assert_eq!(r["cameraProfiles"]["profiles"][&p74]["photos"], 2);
    // bad params
    let e = s.execute("library.import", &json!({"paths": [c], "cameraProfile": {"perBody": {"SN71": 5}}})).unwrap_err();
    assert!(e.to_string().contains("profile id"), "{e}");
    let e = s.execute("library.import", &json!({"paths": [c], "cameraProfile": "sometimes"})).unwrap_err();
    assert!(e.to_string().contains("cameraProfile"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn adopting_during_import_makes_it_the_bodys_own() {
    let (mut s, dir) = library("adopt");
    let p71 = profile(&mut s, &dir, "M71.dng", "D55", Some("SN71"));
    let b = raw(&dir, "b74.dng", "D50", Some("SN74"));
    let e = s.execute("library.import", &json!({"paths": [b], "adopt": ["SN74"]})).unwrap_err();
    assert!(e.to_string().contains("needs a camera profile chosen"), "{e}");
    let r = s.execute("library.import", &json!({"paths": [b], "cameraProfile": {"perBody": {"SN74": p71}}, "adopt": ["SN74"]})).unwrap();
    assert_eq!(r["adopted"], json!([{"body": "SN74", "profile": p71}]));
    assert_eq!(s.dcp_profiles[0].bodies, ["SN71", "SN74"]);
    // a later photo of body 74 now takes it on its own
    let r = s.execute("library.import", &json!({"paths": [raw(&dir, "c74.dng", "D65", Some("SN74"))]})).unwrap();
    assert_eq!(calibration(&s, ids(&r)[0]), p71);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sidecar_wins_and_duplicates_are_left_alone() {
    let (mut s, dir) = library("sidecar");
    let p71 = profile(&mut s, &dir, "M71.dng", "D55", Some("SN71"));
    let a = raw(&dir, "a71.dng", "D65", Some("SN71"));
    sidecar(&a);
    let b = raw(&dir, "b71.dng", "D50", Some("SN71"));
    let r = s.execute("library.import", &json!({"paths": [a, b]})).unwrap();
    let got = ids(&r);
    assert_eq!(calibration(&s, got[0]), "", "Lightroom's settings were made on its colour, not on the body's profile");
    assert_eq!(s.catalog.photo(got[0]).unwrap().develop.light.exposure, 0.5);
    assert_eq!(calibration(&s, got[1]), p71);
    assert_eq!(r["cameraProfiles"]["keptFromSidecar"], 1, "{r}");
    // even an explicit choice for all photos leaves the sidecar's
    let c = raw(&dir, "c71.dng", "A", Some("SN71"));
    sidecar(&c);
    let r = s.execute("library.import", &json!({"paths": [c], "cameraProfile": {"all": p71}})).unwrap();
    assert_eq!(calibration(&s, ids(&r)[0]), "");
    // duplicates: nothing about the photo already there changes
    let before = s.catalog.photo(got[1]).unwrap().develop.clone();
    let r = s.execute("library.import", &json!({"paths": [b], "cameraProfile": {"all": "builtin"}, "look": "lc.vivid"})).unwrap();
    assert!(ids(&r).is_empty() && r["duplicates"].as_array().unwrap().len() == 1, "{r}");
    assert_eq!(s.catalog.photo(got[1]).unwrap().develop, before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_look_and_a_preset_go_on_top_of_the_calibration() {
    let (mut s, dir) = library("look");
    let p71 = profile(&mut s, &dir, "M71.dng", "D55", Some("SN71"));
    s.presets.push(lightcraft_develop::Preset {
        id: "test.bright".into(),
        name: "Bright".into(),
        group: "Test".into(),
        settings: json!({"light": {"exposure": 0.7}}),
        favorite: false,
        builtin: false,
    });
    let a = raw(&dir, "a71.dng", "D65", Some("SN71"));
    let r = s.execute("library.import", &json!({"paths": [a], "look": "lc.vivid", "preset": "test.bright"})).unwrap();
    let id = ids(&r)[0];
    let d = &s.catalog.photo(id).unwrap().develop;
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), ("lc.vivid", p71.as_str()), "the look on the body's profile");
    assert_eq!(d.light.exposure, 0.7, "the preset too");
    let e = s.execute("library.import", &json!({"paths": [a], "look": p71})).unwrap_err();
    assert!(e.to_string().contains("cameraProfile"), "{e}");
    let e = s.execute("library.import", &json!({"paths": [a], "look": "lc.nope"})).unwrap_err();
    assert!(e.to_string().contains("unknown look"), "{e}");
    // the import is one undo step: undo removes the photo, look and all
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.photo(id).is_none_or(|p| p.deleted) || !s.catalog.photos().any(|p| p.id == id));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cameras_are_named_without_the_make_twice() {
    use crate::import_profiles::display_camera;
    assert_eq!(display_camera("Canon Canon EOS R5 C"), "Canon EOS R5 C");
    assert_eq!(display_camera("NIKON CORPORATION NIKON Z 6"), "NIKON Z 6");
    assert_eq!(display_camera("SONY ILCE-9"), "Sony ILCE-9");
    assert_eq!(display_camera(" "), "");
}

/// Duplicates are photos already in this catalog: a folder of new raws (with identical other files
/// beside them, as a library's thumbnail folder has) has none, and its rows add up to its raws.
#[test]
fn duplicates_are_only_raws_already_in_the_catalog() {
    let (mut s, dir) = library("dups");
    let card = dir.join("card");
    let thumbs = card.join("thumbs");
    std::fs::create_dir_all(&thumbs).unwrap();
    let mut png = Vec::new();
    let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[90, 3, 9, 255]; 64] };
    png.extend(lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap());
    for i in 0..4 {
        std::fs::write(thumbs.join(format!("t{i}.png")), &png).unwrap();
    }
    let first = raw(&card, "A1.dng", "D65", Some("SN71"));
    raw(&card, "A2.dng", "D50", Some("SN71"));
    raw(&card, "_SA1.dng", "A", None);
    let plan = |s: &mut Session| s.execute("import.profilePlan", &json!({"paths": [card.to_string_lossy()]})).unwrap();
    let r = plan(&mut s);
    assert_eq!((r["photos"].as_u64(), r["duplicates"].as_u64(), r["copiesInSource"].as_u64()), (Some(3), Some(0), Some(0)), "{r}");
    let rows: u64 = r["rows"].as_array().unwrap().iter().filter_map(|row| row["photos"].as_u64()).sum();
    assert_eq!(rows, 3);
    assert_eq!(r["rows"][0]["camera"], "LightCraft Chart Camera");
    // one of them imported: now it is a duplicate, and no longer among the photos
    s.execute("library.import", &json!({"paths": [first]})).unwrap();
    let r = plan(&mut s);
    assert_eq!((r["photos"].as_u64(), r["duplicates"].as_u64()), (Some(2), Some(1)), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}
