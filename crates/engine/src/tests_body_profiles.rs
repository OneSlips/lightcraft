//! Camera profiles per body (LR-PROF-CREATE): names with the body's serial, replace / keep both /
//! undo, auto-pick of a body's own profile at import and by `profile.applyBodyDefaults`, adopting a
//! sibling body's profile, and the picker's view of profiles by body. Synthetic DNGs only.

use std::path::PathBuf;

use lightcraft_camprof::ChartCorners;
use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::Session;

const W: usize = 360;
const H: usize = 240;

fn corners() -> ChartCorners {
    ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0)
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-bodyprof-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A chart DNG from body `serial` under `light`, written to `dir/name`.
fn chart(dir: &std::path::Path, name: &str, light: &str, serial: Option<&str>) -> String {
    let p = dir.join(name);
    std::fs::write(&p, crate::chart_profiling::demo_chart_dng_of_body(W, H, &corners(), light, true, serial).unwrap()).unwrap();
    p.to_string_lossy().to_string()
}

fn library(tag: &str) -> (Session, PathBuf) {
    let dir = temp_dir(tag);
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    (s, dir)
}

fn import(s: &mut Session, path: &str) -> PhotoId {
    let r = s.execute("library.import", &json!({"paths": [path]})).unwrap();
    PhotoId(r["imported"][0].as_u64().unwrap_or_else(|| panic!("{r}")))
}

/// The camera profile a photo develops with, else its look (`lc.color`: LightCraft's own colour).
fn profile_of(s: &Session, id: PhotoId) -> String {
    let d = &s.catalog.photo(id).unwrap().develop;
    d.calibration_id().unwrap_or(&d.profile.id).to_string()
}

#[test]
fn names_carry_the_body_and_files_are_named_after_them() {
    let (mut s, dir) = library("names");
    let p71 = chart(&dir, "A71.dng", "D65", Some("012345678971"));
    let r = s.execute("profile.createFromChart", &json!({"path": p71, "illuminant": "D65", "apply": false})).unwrap();
    let name = r["name"].as_str().unwrap();
    assert!(name.starts_with("LightCraft Chart Camera 71 · D65 · Photo · "), "{name}");
    let file = r["file"].as_str().unwrap();
    assert!(file.contains("LightCraft_Chart_Camera_71_D65_Photo_") && file.ends_with(".dcp"), "{file}");
    assert_eq!(r["bodies"], json!(["012345678971"]));
    // the ProfileName tag is the whole name; UniqueCameraModel stays the camera
    let dcp = lightcraft_raw::dcp::parse_dcp(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!((dcp.name.as_str(), dcp.camera_model.as_str()), (name, "LightCraft Chart Camera"));
    // no serial: no body in the name; a one-character serial: that character
    let none = chart(&dir, "N.dng", "D65", None);
    let r = s.execute("profile.createFromChart", &json!({"path": none, "illuminant": "D65", "apply": false, "rendering": "reproduction"})).unwrap();
    assert!(r["name"].as_str().unwrap().starts_with("LightCraft Chart Camera · D65 · Repro · "), "{}", r["name"]);
    assert_eq!(r["bodies"], json!([]));
    let short = chart(&dir, "S.dng", "A", Some(" 7 "));
    let r = s.execute("profile.createFromChart", &json!({"path": short, "illuminant": "A", "apply": false})).unwrap();
    assert!(r["name"].as_str().unwrap().starts_with("LightCraft Chart Camera 7 · StdA · Photo"), "{}", r["name"]);
    assert_eq!(crate::chart_profiling::body_suffix(Some("  ")), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_bodies_are_noticed() {
    let dir = temp_dir("bodies");
    let a = crate::chart_profiling::prepare(&std::fs::read(chart(&dir, "A.dng", "A", Some("SN71"))).unwrap()).unwrap();
    let b = crate::chart_profiling::prepare(&std::fs::read(chart(&dir, "B.dng", "D65", Some("SN74"))).unwrap()).unwrap();
    let c = crate::chart_profiling::prepare(&std::fs::read(chart(&dir, "C.dng", "D65", Some("SN71"))).unwrap()).unwrap();
    assert_eq!(crate::chart_profiling::bodies_differ(&a, &b), Some(("71".into(), "74".into())));
    assert_eq!(crate::chart_profiling::bodies_differ(&a, &c), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_keeps_the_id_photos_follow_and_undo_restores() {
    let (mut s, dir) = library("replace");
    let day = chart(&dir, "DAY.dng", "D65", Some("SN71"));
    let id = import(&mut s, &day);
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    let first = s.execute("profile.createFromChart", &json!({"name": "Studio", "illuminant": "D65"})).unwrap();
    let pid = first["id"].as_str().unwrap().to_string();
    assert_eq!(profile_of(&s, id), pid);
    let old_bytes = std::fs::read(first["file"].as_str().unwrap()).unwrap();
    let ask = s.execute("profile.createFromChart", &json!({"name": "Studio", "illuminant": "D50", "onExists": "ask"})).unwrap();
    assert_eq!(ask["exists"]["id"], pid.as_str());
    assert_eq!(ask["exists"]["usedBy"], 1);
    assert_eq!(ask["keepBothName"], "Studio 2");
    assert_eq!(s.dcp_profiles.len(), 1, "ask saves nothing");
    // replace: same id and file, new content; the photo is decoded with the new version
    let job = s.render_job(id, 48, 48, false, true).unwrap();
    let loaded = job.run().loaded.unwrap();
    s.media.insert(id, crate::media::SourceLevel::Thumb, loaded.image.clone());
    let r = s.execute("profile.createFromChart", &json!({"name": "Studio", "illuminant": "D50", "onExists": "replace"})).unwrap();
    assert_eq!((r["id"].as_str(), r["replaced"].as_bool()), (Some(pid.as_str()), Some(true)));
    assert_eq!(s.dcp_profiles.len(), 1);
    let new_bytes = std::fs::read(&s.dcp_profiles[0].file).unwrap();
    assert_ne!(new_bytes, old_bytes);
    assert_eq!(s.media.camera_profiles[&pid].color.illuminant[0], 23, "D50 now");
    match &s.render_job(id, 48, 48, false, true).unwrap().source {
        crate::media::SourceRef::File { camera_profile: Some((made_with, p)), .. } => {
            assert_ne!(made_with, &pid, "a decode made with the old version doesn't stand in");
            assert_eq!(p.color.illuminant[0], 23);
        }
        other => panic!("decoded again with the new profile, not {}", matches!(other, crate::media::SourceRef::Loaded(_))),
    }
    // undo puts the old file and profile back; redo the new
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(std::fs::read(&s.dcp_profiles[0].file).unwrap(), old_bytes);
    assert_eq!(s.media.camera_profiles[&pid].color.illuminant[0], 21);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(std::fs::read(&s.dcp_profiles[0].file).unwrap(), new_bytes);
    // keepBoth: a second entry
    let r = s.execute("profile.createFromChart", &json!({"name": "Studio", "illuminant": "D65", "onExists": "keepBoth", "apply": false})).unwrap();
    assert_eq!(r["name"], "Studio 2");
    assert_eq!(s.dcp_profiles.len(), 2);
    assert!(s.execute("profile.createFromChart", &json!({"name": "Studio", "onExists": "sometimes"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bodys_own_profile_is_picked_at_import_and_never_over_a_choice() {
    let (mut s, dir) = library("autopick");
    let made = chart(&dir, "MADE71.dng", "D65", Some("SN71"));
    let r = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "D65", "apply": false})).unwrap();
    let pid = r["id"].as_str().unwrap().to_string();
    // a photo from the same body gets it; another body or no serial gets the default
    let same = import(&mut s, &chart(&dir, "SAME71.dng", "D65", Some("SN71")));
    let other = import(&mut s, &chart(&dir, "OTHER74.dng", "D65", Some("SN74")));
    let none = import(&mut s, &chart(&dir, "NONE.dng", "D65", None));
    assert_eq!(profile_of(&s, same), pid);
    assert_eq!(profile_of(&s, other), "lc.color");
    assert_eq!(profile_of(&s, none), "lc.color");
    // a look keeps the body's profile; LightCraft's own colour chosen by the user stays, also
    // through applyBodyDefaults
    s.execute("library.select", &json!({"ids": [same.0]})).unwrap();
    s.execute("develop.profile", &json!({"id": "lc.vivid"})).unwrap();
    assert_eq!(profile_of(&s, same), pid);
    s.execute("develop.cameraProfile", &json!({"id": ""})).unwrap();
    let r = s.execute("profile.applyBodyDefaults", &json!({"ids": [same.0, other.0, none.0]})).unwrap();
    assert_eq!(r["changed"], json!([]));
    assert_eq!(r["skipped"], 3);
    assert_eq!(profile_of(&s, same), "lc.vivid");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apply_body_defaults_to_photos_imported_before_is_one_undo_step() {
    let (mut s, dir) = library("apply");
    let a = import(&mut s, &chart(&dir, "A71.dng", "D65", Some("SN71")));
    let b = import(&mut s, &chart(&dir, "B71.dng", "A", Some("SN71")));
    let c = import(&mut s, &chart(&dir, "C74.dng", "D65", Some("SN74")));
    let made = chart(&dir, "MADE71.dng", "D65", Some("SN71"));
    let pid = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "D65", "apply": false})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(profile_of(&s, a), "lc.color", "existing photos are never changed silently");
    let r = s.execute("profile.applyBodyDefaults", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    assert_eq!(r["changed"], json!([a.0, b.0]));
    assert_eq!(r["skipped"], 1);
    assert_eq!((profile_of(&s, a), profile_of(&s, b), profile_of(&s, c)), (pid.clone(), pid.clone(), "lc.color".into()));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((profile_of(&s, a), profile_of(&s, b)), ("lc.color".into(), "lc.color".into()), "one step");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn priority_explicit_default_then_dual_then_newest() {
    let (mut s, dir) = library("priority");
    let mk = |s: &mut Session, name: &str, extra: Value| {
        let mut p = json!({"path": chart(&dir, &format!("{name}.dng"), "D65", Some("SN71")), "illuminant": "D65", "apply": false, "name": name});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        s.execute("profile.createFromChart", &p).unwrap()["id"].as_str().unwrap().to_string()
    };
    let older = mk(&mut s, "older", json!({}));
    let dual = mk(&mut s, "dual", json!({"second": {"path": chart(&dir, "dualA.dng", "A", Some("SN71")), "illuminant": "A"}}));
    let newest = mk(&mut s, "newest", json!({}));
    let camera = "LightCraft Chart Camera";
    assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, camera, "SN71").as_deref(), Some(dual.as_str()), "dual over newest");
    s.execute("profile.setBodyDefault", &json!({"profile": older, "serial": "SN71"})).unwrap();
    assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, camera, "SN71").as_deref(), Some(older.as_str()), "explicit default wins");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, camera, "SN71").as_deref(), Some(dual.as_str()));
    s.execute("profile.deleteCamera", &json!({"id": dual})).unwrap();
    assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, camera, "SN71").as_deref(), Some(newest.as_str()), "else the newest");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn adopting_and_releasing_bodies() {
    let (mut s, dir) = library("adopt");
    let pid = s
        .execute("profile.createFromChart", &json!({"path": chart(&dir, "M71.dng", "D65", Some("SN71")), "illuminant": "D65", "apply": false}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let p74 = import(&mut s, &chart(&dir, "P74.dng", "D65", Some("SN74")));
    // the picker's view from body 74: another body's profile
    let v = s.execute("profiles.forPhoto", &json!({"id": p74.0})).unwrap();
    assert_eq!(v["profiles"][0]["relation"], "otherBody", "{v}");
    let r = s.execute("profile.adoptForBody", &json!({"profile": pid, "photo": p74.0})).unwrap();
    assert_eq!(r["bodies"], json!(["SN71", "SN74"]));
    assert_eq!(s.execute("profiles.forPhoto", &json!({"id": p74.0})).unwrap()["profiles"][0]["relation"], "body");
    // a new photo from body 74 now takes it at import
    let q74 = import(&mut s, &chart(&dir, "Q74.dng", "D50", Some("SN74")));
    assert_eq!(profile_of(&s, q74), pid);
    // release, refused for the last body, undo
    s.execute("profile.releaseFromBody", &json!({"profile": pid, "serial": "SN74"})).unwrap();
    assert_eq!(s.dcp_profiles[0].bodies, ["SN71"]);
    let e = s.execute("profile.releaseFromBody", &json!({"profile": pid, "serial": "SN71"})).unwrap_err().to_string();
    assert!(e.contains("only body"), "{e}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.dcp_profiles[0].bodies, ["SN71", "SN74"], "undo puts the body back");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.dcp_profiles[0].bodies, ["SN71"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_single_serial_entries_migrate() {
    let mut e: crate::cmd::dcp_profiles::DcpEntry =
        serde_json::from_value(json!({"id": "dcp:x", "name": "X", "cameraModel": "C", "file": "x.dcp", "serial": " SN71 "})).unwrap();
    crate::cmd::dcp_profiles::migrate(&mut e);
    assert_eq!(e.bodies, ["SN71"]);
    assert!(serde_json::to_value(&e).unwrap().get("serial").is_none(), "written as bodies only");
    let mut e: crate::cmd::dcp_profiles::DcpEntry = serde_json::from_value(json!({"id": "dcp:y", "name": "Y", "file": "y.dcp"})).unwrap();
    crate::cmd::dcp_profiles::migrate(&mut e);
    assert!(e.bodies.is_empty(), "model-wide");
}
