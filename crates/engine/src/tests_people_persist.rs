//! Note 15: people's names and roles belong to the library. Naming a face and giving it a role,
//! then closing the library (quitting) and opening it again, keeps both, and a recipe by role
//! (a smart preset's "Bride") still finds that person.

use std::path::PathBuf;

use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::Session;
use crate::segment::worker::FoundFace;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-people-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Photo `id`'s faces as the face model would find them (no model in tests).
fn found(s: &mut Session, id: PhotoId, rects: &[[f64; 4]]) {
    let key = s.segment_input_key(id).unwrap();
    let faces = rects.iter().map(|r| FoundFace { rect: *r, score: 0.9, signature: Default::default(), landmarks: true }).collect();
    s.segmenter.smart.faces.insert(id, (key, faces));
}

fn region_names(s: &Session, id: PhotoId) -> Vec<Option<String>> {
    s.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

#[test]
fn names_and_roles_survive_reopening_the_library() {
    let dir = temp_dir("reopen");
    let mut s = Session::new();
    s.open_library(&dir, true).unwrap();
    let id = s.visible_cloned()[0];
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    let (bride, groom) = ([0.30, 0.20, 0.40, 0.35], [0.60, 0.22, 0.70, 0.37]);
    found(&mut s, id, &[bride, groom]);
    s.execute("people.name", &json!({"id": id.0, "face": 0, "name": "Alex", "role": "Bride", "scope": {"ids": [id.0]}})).unwrap();
    s.execute("people.name", &json!({"id": id.0, "face": 1, "name": "Sam"})).unwrap();
    s.execute("people.setRole", &json!({"role": "Groom", "person": "Sam", "scope": {"ids": [id.0]}})).unwrap();
    let names = region_names(&s, id);
    assert!(names.contains(&Some("Alex".into())) && names.contains(&Some("Sam".into())), "{names:?}");
    s.close_library().unwrap();
    drop(s);

    // the next start: the same library, the faces found again (a little differently)
    let mut s = Session::new();
    s.open_library(&dir, false).unwrap();
    let names = region_names(&s, id);
    assert!(names.contains(&Some("Alex".into())) && names.contains(&Some("Sam".into())), "names kept: {names:?}");
    let r = s.execute("people.roles", &json!({"id": id.0})).unwrap();
    let here = r["here"].as_array().cloned().unwrap_or_default();
    assert!(here.contains(&json!({"role": "Bride", "person": "Alex"})), "roles kept: {r}");
    assert!(here.contains(&json!({"role": "Groom", "person": "Sam"})), "roles kept: {r}");
    found(&mut s, id, &[[0.31, 0.21, 0.41, 0.36], [0.61, 0.23, 0.71, 0.38]]);
    let faces = s.execute("people.faces", &json!({"id": id.0})).unwrap();
    assert_eq!(faces["faces"][0]["person"], "Alex", "{faces}");
    assert_eq!(faces["faces"][1]["person"], "Sam", "{faces}");
    // a recipe by role still finds the bride on this photo
    match s.resolve_who(id, &lightcraft_develop::recipe::Who::Role("Bride".into())) {
        crate::smart_masks::Resolved::Person { rect, .. } => assert_eq!(rect, Some(bride)),
        _ => panic!("the Bride is found by her role"),
    }
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_jpeg(path: &std::path::Path, seed: u8) {
    let (w, h) = (48usize, 32usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_jpeg(&lightcraft_codecs::EncodeImage::rgba8(&img), 90, Default::default(), &Default::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The Masking panel's way (Save Name with a role picked: the role for the photo's folder), on a
/// photo imported from disk with XMP auto-write on and an edit after the naming (its sidecar
/// written): the names and the folder's roles come back after a restart.
#[test]
fn names_and_folder_roles_of_imported_photos_survive_a_restart() {
    let src = temp_dir("src");
    write_jpeg(&src.join("first.jpg"), 90);
    write_jpeg(&src.join("second.jpg"), 140);
    let dir = temp_dir("lib");
    let mut s = Session::new().with_fs();
    s.open_library(&dir, false).unwrap();
    s.xmp.auto_write = true;
    s.execute("library.import", &json!({"paths": [src.to_string_lossy()], "mode": "add"})).unwrap();
    let ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    assert_eq!(ids.len(), 2);
    let (a, b) = (ids[0], ids[1]);
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    let face = [0.30, 0.20, 0.40, 0.35];
    found(&mut s, a, &[face]);
    s.execute("people.name", &json!({"id": a.0, "face": 0, "name": "Alex", "role": "Bride"})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 0.5}})).unwrap();
    s.close_library().unwrap();
    drop(s);

    let mut s = Session::new().with_fs();
    s.open_library(&dir, false).unwrap();
    assert_eq!(region_names(&s, a), [Some("Alex".to_string())], "the name is kept");
    for id in [a, b] {
        let r = s.execute("people.roles", &json!({"id": id.0})).unwrap();
        assert_eq!(r["here"], json!([{"role": "Bride", "person": "Alex"}]), "the folder's role is kept for photo {id:?}: {r}");
    }
    let (names, roles) = s.people_names();
    assert!(names.iter().any(|n| n == "Alex") && roles.iter().any(|r| r == "Bride"), "{names:?} {roles:?}");
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&src);
}
