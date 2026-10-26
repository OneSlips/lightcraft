//! Lens profiles stay per lens. Presets, Copy / Paste and Sync carry "Enable Lens Corrections" and
//! its amounts, never a profile: each photo keeps the one found for its own lens. And photos imported before the lens
//! database was there get their profile when it arrives (`cmd::lens_refresh`).

use std::sync::Arc;

use lightcraft_catalog::{MediaKind, Op, Photo, PhotoId, Source};
use lightcraft_develop::{EmbeddedLens, EmbeddedWarp, LensSource};
use serde_json::json;

use crate::Session;

fn lens(k: f64) -> EmbeddedLens {
    EmbeddedLens {
        warp: Some(EmbeddedWarp { planes: [[1.0, k, 0.0, 0.0, 0.0, 0.0]; 3], radius: 0.6, ..Default::default() }),
        source: LensSource::Database,
        ..Default::default()
    }
}

fn lens_of(s: &Session, id: PhotoId) -> Option<EmbeddedLens> {
    s.catalog.photo(id).and_then(|p| p.embedded_lens)
}

/// Two demo photos "taken with different lenses".
fn two_lenses() -> (Session, PhotoId, PhotoId) {
    let mut s = Session::with_demo();
    let ids: Vec<PhotoId> = s.visible().iter().copied().take(2).collect();
    let (a, b) = (ids[0], ids[1]);
    s.catalog.apply(Op::SetEmbeddedLens { id: a, lens: Some(Box::new(lens(-0.05))) }).unwrap();
    s.catalog.apply(Op::SetEmbeddedLens { id: b, lens: Some(Box::new(lens(0.03))) }).unwrap();
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("develop.merge", &json!({"settings": {"optics": {"lens_profile": true}}, "label": "Lens Corrections"})).unwrap();
    s.execute("develop.set", &json!({"values": {"optics.profileDistortion": 60}})).unwrap();
    (s, a, b)
}

fn carried(s: &Session, id: PhotoId) {
    let d = s.develop_of(id).unwrap();
    assert!(d.optics.lens_profile, "the switch travels");
    assert_eq!(d.optics.profile_distortion, 60.0, "the amount travels");
}

#[test]
fn copy_paste_carries_the_switch_and_amounts_never_the_profile() {
    let (mut s, a, b) = two_lenses();
    s.execute("develop.copy", &json!({})).unwrap();
    s.execute("develop.paste", &json!({"ids": [b.0]})).unwrap();
    carried(&s, b);
    assert_eq!(lens_of(&s, b), Some(lens(0.03)), "b keeps its own lens's profile");
    assert_eq!(lens_of(&s, a), Some(lens(-0.05)));
}

#[test]
fn sync_and_presets_keep_each_photos_own_profile() {
    let (mut s, a, b) = two_lenses();
    s.execute("library.select", &json!({"ids": [a.0, b.0], "active": a.0})).unwrap();
    s.execute("develop.sync", &json!({})).unwrap();
    carried(&s, b);
    assert_eq!(lens_of(&s, b), Some(lens(0.03)));
    // a preset made on a's settings, applied to b
    let pid = s.execute("preset.create", &json!({"name": "Optics", "groups": ["optics"]})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("develop.merge", &json!({"settings": {"optics": {"lens_profile": false}}, "label": "Lens Corrections"})).unwrap();
    s.execute("preset.apply", &json!({"id": pid})).unwrap();
    carried(&s, b);
    assert_eq!(lens_of(&s, b), Some(lens(0.03)), "a preset never brings another lens's profile");
    assert_eq!(lens_of(&s, a), Some(lens(-0.05)));
}

/// A raw in a library folder, as an older import left it: no lens data.
fn raw(id: u64, lens_name: &str, edited: bool) -> Photo {
    let mut p =
        Photo::new(PhotoId(id), Source::File { path: format!("/photos/{id}.cr3") }, &format!("{id}.cr3"), "CR3", 600, 400, "2026-10-01T00:00:00");
    p.kind = MediaKind::Raw;
    p.meta.camera = "Testcam TC-1".into();
    p.meta.lens = lens_name.into();
    if edited {
        let mut d = (*p.develop).clone();
        d.light.exposure = 0.5;
        p.develop = Arc::new(d);
        p.edited = Some("2026-10-02T00:00:00".into());
    }
    p
}

const DB: &str = r#"<lensdatabase>
  <camera><maker>Testcam</maker><model>TC-1</model><mount>T</mount><cropfactor>1</cropfactor></camera>
  <lens><maker>Testcam</maker><model>Testcam 50mm f/1.8</model><mount>T</mount><cropfactor>1</cropfactor>
    <calibration><distortion model="poly3" focal="50" k1="-0.01" /></calibration></lens></lensdatabase>"#;

/// When the database arrives, the library's raws whose lens it knows get their profile; untouched photos get the
/// correction switched on as import would, edited ones keep their settings; lenses it doesn't know are not probed.
#[test]
fn photos_get_their_profile_when_the_database_arrives() {
    let mut s = Session::new();
    for p in [raw(1, "50mm F1.8", false), raw(2, "50mm F1.8", true), raw(3, "300mm F4", false), raw(4, "", false)] {
        s.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    let db = lightcraft_lensdb::LensDb::parse(DB).unwrap();
    let listed = crate::cmd::lens_refresh::missing(&s.catalog, &db, None);
    assert_eq!(listed.iter().map(|m| m.id.0).collect::<Vec<_>>(), [1, 2], "only lenses the database knows");
    assert_eq!(crate::cmd::lens_refresh::missing(&s.catalog, &db, Some(&[PhotoId(2)])).len(), 1, "or only the photos asked for");
    // what the files give when read again
    let results = listed.into_iter().map(|m| (m, Ok(crate::media::ProbeInfo { embedded_lens: Some(lens(-0.01)), ..Default::default() }))).collect();
    let found = crate::cmd::lens_refresh::apply(&mut s, results).unwrap();
    assert_eq!(found, [PhotoId(1), PhotoId(2)]);
    let p = |id: u64| s.catalog.photo(PhotoId(id)).unwrap().clone();
    assert!(p(1).embedded_lens.is_some() && p(1).develop.optics.lens_profile, "untouched: on, as import would");
    assert!(p(2).embedded_lens.is_some() && !p(2).develop.optics.lens_profile, "edited: its settings stay");
    assert!(p(3).embedded_lens.is_none());
    assert!(s.undo.is_empty(), "bookkeeping, not an undo step");
    // the Optics panel's words
    assert_eq!(crate::cmd::lens_refresh::no_profile_reason(&p(3)).as_deref(), Some("No lens profile found for 300mm F4."));
    assert_eq!(crate::cmd::lens_refresh::no_profile_reason(&p(4)).as_deref(), Some("This file names no lens."));
    assert_eq!(crate::cmd::lens_refresh::no_profile_reason(&p(1)), None);
    // a result read from a file the photo no longer has (moved meanwhile) is dropped, for its virtual copies too
    let mut copy = raw(5, "300mm F4", false);
    copy.copy_of = Some(PhotoId(3));
    s.catalog.apply(Op::AddPhoto { photo: Box::new(copy) }).unwrap();
    let stale = crate::cmd::lens_refresh::Missing { id: PhotoId(3), path: "/elsewhere/3.cr3".into() };
    let ok = crate::media::ProbeInfo { embedded_lens: Some(lens(-0.01)), ..Default::default() };
    assert!(crate::cmd::lens_refresh::apply(&mut s, vec![(stale, Ok(ok))]).unwrap().is_empty());
    assert!(s.catalog.photo(PhotoId(3)).unwrap().embedded_lens.is_none(), "a stale probe changes nothing");
    assert!(s.catalog.photo(PhotoId(5)).unwrap().embedded_lens.is_none(), "nor the copies");
    // without file access the command says so instead of clearing anything
    assert!(s.execute("library.refreshLensProfiles", &json!({})).is_err());
    assert!(s.execute("library.refreshLensProfiles", &json!({"ids": "x"})).is_err());
}
