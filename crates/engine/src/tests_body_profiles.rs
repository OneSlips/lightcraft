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
    assert!(r["name"].as_str().unwrap().starts_with("LightCraft Chart Camera 7 · Tungsten · Photo"), "{}", r["name"]);
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

#[test]
fn a_body_without_a_serial_is_known_by_its_file_names() {
    use crate::chart_profiling::body_key;
    assert_eq!(body_key(Some(" 0123 "), Some("_SA94454.ARW")).as_deref(), Some("0123"), "a real serial wins");
    assert_eq!(body_key(None, Some("_SA94454.ARW")).as_deref(), Some("file:SA"));
    assert_eq!(body_key(Some(""), Some("__71_4940.CR3")).as_deref(), Some("file:71"));
    assert_eq!(body_key(None, Some("_S.ARW")), None, "fewer than two characters");
    assert_eq!(body_key(None, Some("_-x123.ARW")), None, "not letters or digits");
    assert_eq!(body_key(None, None), None);
    assert_eq!(crate::cmd::dcp_profiles::body_label("file:SA"), "SA");
    assert_eq!(crate::cmd::dcp_profiles::body_label("012371"), "…71");

    // named after it, stored as file:SA, and picked for photos of the same camera with that key
    let (mut s, dir) = library("filebody");
    let made = chart(&dir, "_SA94454.dng", "D55", None);
    let r = s.execute("profile.createFromChart", &json!({"path": made, "apply": false})).unwrap();
    assert!(r["name"].as_str().unwrap().starts_with("LightCraft Chart Camera SA · Daylight · Photo · "), "{}", r["name"]);
    assert_eq!(r["bodies"], json!(["file:SA"]));
    let pid = r["id"].as_str().unwrap().to_string();
    let same = import(&mut s, &chart(&dir, "_SA90001.dng", "D65", None));
    let other = import(&mut s, &chart(&dir, "_SB90002.dng", "D50", None));
    // a real serial wins over the file name: this one is another body
    let serial = import(&mut s, &chart(&dir, "_SA90003.dng", "A", Some("REAL01")));
    assert_eq!(profile_of(&s, same), pid);
    assert_eq!(profile_of(&s, other), "lc.color");
    assert_eq!(profile_of(&s, serial), "lc.color");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lights_by_their_words_and_auto_no_longer() {
    let (mut s, dir) = library("lights");
    let p = chart(&dir, "L.dng", "D55", None);
    let r = s.execute("profile.createFromChart", &json!({"path": p, "apply": false, "name": "a"})).unwrap();
    assert_eq!(r["illuminant"], "D55", "Daylight / Flash by default");
    assert_eq!(r["warnings"], json!([]));
    let r = s.execute("profile.createFromChart", &json!({"path": p, "apply": false, "name": "b", "illuminant": "auto"})).unwrap();
    assert_eq!(r["illuminant"], "D55");
    assert!(r["warnings"][0].as_str().unwrap().contains("no longer supported"), "{r}");
    for (word, std) in [("daylight", "D55"), ("Flash", "D55"), ("tungsten", "A")] {
        let r = s.execute("profile.createFromChart", &json!({"path": p, "apply": false, "name": word, "illuminant": word})).unwrap();
        assert_eq!(r["illuminant"], std, "{word}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The catalog forgets photo `id`'s serial, as a library made before serials were kept has it.
fn forget_serial(s: &mut Session, id: PhotoId) {
    let mut meta = s.catalog.photo(id).unwrap().meta.clone();
    meta.serial.clear();
    s.catalog.apply(lightcraft_catalog::Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

#[test]
fn a_profile_made_from_a_photo_of_an_old_library_is_that_bodys_own() {
    use crate::cmd::dcp_profiles as dp;
    let (mut s, dir) = library("oldlib");
    let id = import(&mut s, &chart(&dir, "_71_4940.dng", "D65", Some("012345678971")));
    forget_serial(&mut s, id);
    assert_eq!(dp::catalog_body(s.catalog.photo(id).unwrap()).as_deref(), Some("file:71"));
    let r = s.execute("profile.createFromChart", &json!({"id": id.0, "illuminant": "D65"})).unwrap();
    // the profile is the body's, by its serial
    assert_eq!(r["bodies"], json!(["012345678971"]), "{r}");
    // the catalog learnt the photo's serial; the photo's profile is its body's own: no Adopt
    assert_eq!(s.catalog.photo(id).unwrap().meta.serial, "012345678971");
    let (relation, _, is_default) = dp::photo_profile_status(&s, id).unwrap();
    assert_eq!((relation, is_default), ("body", true));
    assert_eq!(dp::hidden_for_photo(&s, Some(id)), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_profile_listing_only_the_serial_matches_once_the_serial_is_read() {
    use crate::cmd::dcp_profiles as dp;
    let (mut s, dir) = library("readserial");
    let id = import(&mut s, &chart(&dir, "_71_1.dng", "D65", Some("SN71")));
    s.execute("profile.createFromChart", &json!({"id": id.0, "illuminant": "D65"})).unwrap();
    // a profile from an earlier build: the serial only
    for e in &mut s.dcp_profiles {
        e.bodies = vec!["SN71".into()];
    }
    forget_serial(&mut s, id);
    assert_eq!(dp::photo_profile_status(&s, id).unwrap().0, "otherBody");
    let path = dp::photo_file(&s, id).unwrap();
    let serial = dp::read_serial(&path).unwrap();
    assert!(dp::backfill_serial(&mut s, id, &serial));
    assert!(!dp::backfill_serial(&mut s, id, "OTHER"), "a known serial is never replaced");
    assert_eq!(dp::photo_profile_status(&s, id).unwrap().0, "body");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Culling a card in place (Browse) develops each body's raws with the body's default camera profile,
/// also a model-wide one the user made the body's default (the user's own `.dcp`, kept in the shared
/// store): the render that follows the camera preview is the body's look, not LightCraft's plain colour.
#[test]
fn browsing_develops_each_body_with_its_default_profile() {
    let dir = temp_dir("browse-default");
    let mut s = Session::new().with_fs().with_app_store(&dir.join("store"));
    s.open_library(dir.join("lib"), false).unwrap();
    // a model-wide profile of the camera (the user's own .dcp, no bodies)
    let made = chart(&dir, "MADE.dng", "D65", Some("SN71"));
    let r = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "D65", "apply": false})).unwrap();
    let file = r["file"].as_str().unwrap().to_string();
    let copy = dir.join("Wedding.dcp");
    std::fs::copy(&file, &copy).unwrap();
    s.execute("profile.deleteCamera", &json!({"id": r["id"]})).unwrap();
    let r = s.execute("profile.import", &json!({"paths": [copy.to_string_lossy()]})).unwrap();
    let pid = r["imported"][0]["id"].as_str().unwrap().to_string();
    assert!(s.dcp_profiles.iter().all(|e| e.bodies.is_empty()));
    s.execute("profile.setBodyDefault", &json!({"profile": pid, "serial": "SN74"})).unwrap();
    // a card of two bodies, looked at in place
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    chart(&card, "A74.dng", "D65", Some("SN74"));
    chart(&card, "B71.dng", "D65", Some("SN71"));
    s.execute("library.browse", &json!({"path": card.to_string_lossy()})).unwrap();
    let of = |s: &Session, name: &str| {
        let p = s.catalog.photos().find(|p| p.file_name == name).unwrap();
        profile_of(s, p.id)
    };
    assert_eq!(of(&s, "A74.dng"), pid);
    assert_eq!(of(&s, "B71.dng"), "lc.color", "a body without a default keeps LightCraft's colour");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A user preset `id` with `settings`.
fn add_preset(s: &mut Session, id: &str, settings: Value) {
    s.presets.push(lightcraft_develop::Preset { id: id.into(), name: id.into(), group: "Studio".into(), settings, favorite: false, builtin: false });
}

/// LR-IMP-PROFILE: the Import tab imports browsed photos. A body whose default profile was set after
/// the card was browsed (in the Import panel, say) still gets it at import, with a preset stack or
/// without; a preset that names another camera profile or LightCraft's standard colour doesn't take
/// it away unless the import says so (`presetProfile`).
#[test]
fn import_gives_each_body_its_profile_with_and_without_a_preset() {
    let dir = temp_dir("import-body");
    let mut s = Session::new().with_fs().with_app_store(&dir.join("store"));
    s.open_library(dir.join("lib"), false).unwrap();
    let made = chart(&dir, "MADE74.dng", "D65", Some("SN74"));
    let pid = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "D65", "apply": false})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let other = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "A", "apply": false, "name": "Other"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // model-wide profiles (the user's own .dcp files): the body has no default yet while the card
    // is browsed
    for e in &mut s.dcp_profiles {
        e.bodies.clear();
        e.body_defaults.clear();
    }
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    for n in ["A74.dng", "B74.dng", "C74.dng", "D74.dng"] {
        chart(&card, n, "D65", Some("SN74"));
    }
    s.execute("library.browse", &json!({"path": card.to_string_lossy()})).unwrap();
    let id_of = |s: &Session, name: &str| s.catalog.photos().find(|p| p.file_name == name).unwrap().id;
    assert_eq!(profile_of(&s, id_of(&s, "A74.dng")), "lc.color", "no default while browsing");
    // now the body gets its profile (the Import panel's body row)
    s.execute("profile.setBodyDefault", &json!({"profile": pid, "serial": "SN74"})).unwrap();
    add_preset(&mut s, "u.studio", json!({"light": {"exposure": 0.3}, "treatment": "color"}));
    // a preset of LightCraft's own that carries a camera profile and the standard look
    add_preset(&mut s, "u.withProfile", json!({"camera_profile": other, "profile": {"id": "lc.color", "amount": 100.0}}));
    let import = |s: &mut Session, name: &str, extra: Value| {
        let id = id_of(s, name);
        s.selection = crate::Selection::single(id);
        let mut p = json!({"what": "selected", "previews": "none"});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        let r = s.execute("import.run", &p).unwrap();
        assert_eq!(r["imported"].as_array().map(Vec::len), Some(1), "{r}");
        id
    };
    let plain = import(&mut s, "A74.dng", json!({}));
    assert_eq!(profile_of(&s, plain), pid, "without a preset");
    let with = import(&mut s, "B74.dng", json!({"presets": ["u.studio"]}));
    assert_eq!(profile_of(&s, with), pid, "with a preset");
    assert_eq!(s.catalog.photo(with).unwrap().develop.light.exposure, 0.3, "the preset applied");
    let kept = import(&mut s, "C74.dng", json!({"presets": ["u.studio", "u.withProfile"]}));
    assert_eq!(profile_of(&s, kept), pid, "a preset's own profile doesn't win by itself");
    let chosen = import(&mut s, "D74.dng", json!({"presets": ["u.withProfile"], "presetProfile": true}));
    assert_eq!(profile_of(&s, chosen), other, "it wins when the import says so");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A browsed photo picked up its body's profile automatically; the body's default changed before
/// the import: it imports with the new one.
#[test]
fn a_browsed_photo_takes_its_bodys_new_default_at_import() {
    let dir = temp_dir("import-body-new");
    let mut s = Session::new().with_fs().with_app_store(&dir.join("store"));
    s.open_library(dir.join("lib"), false).unwrap();
    let made = chart(&dir, "MADE71.dng", "D65", Some("SN71"));
    let first = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "D65", "apply": false})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    chart(&card, "A71.dng", "D65", Some("SN71"));
    s.execute("library.browse", &json!({"path": card.to_string_lossy()})).unwrap();
    let id = s.catalog.photos().find(|p| p.file_name == "A71.dng").unwrap().id;
    assert_eq!(profile_of(&s, id), first);
    let second = s.execute("profile.createFromChart", &json!({"path": made, "illuminant": "A", "apply": false, "name": "Second"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("profile.setBodyDefault", &json!({"profile": second, "serial": "SN71"})).unwrap();
    s.selection = crate::Selection::single(id);
    s.execute("import.run", &json!({"what": "selected"})).unwrap();
    assert_eq!(profile_of(&s, id), second);
    let _ = std::fs::remove_dir_all(&dir);
}
