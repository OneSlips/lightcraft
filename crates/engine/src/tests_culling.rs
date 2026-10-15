//! The culling filters and counts (the bottom bar): stars, flags and colour labels.

use serde_json::json;

use crate::Session;

fn demo() -> Session {
    Session::with_demo()
}

/// The quick culling filters: unrated, any of several flags, labels with "no label"; the tally
/// counts the view without them.
#[test]
fn quick_filters_and_the_tally() {
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    let total = ids.len();
    assert!(total >= 6);
    s.execute("photo.flag", &json!({"ids": [ids[0], ids[1]], "flag": "pick"})).unwrap();
    s.execute("photo.flag", &json!({"ids": [ids[2]], "flag": "reject"})).unwrap();
    s.execute("photo.rate", &json!({"ids": [ids[0]], "rating": 3})).unwrap();
    s.execute("photo.rate", &json!({"ids": [ids[3]], "rating": 5})).unwrap();
    s.execute("photo.label", &json!({"ids": [ids[4]], "label": "green"})).unwrap();
    let base = s.tally();
    let t = s.execute("library.tally", &json!({})).unwrap();
    assert_eq!(t["picks"], json!(base.picks));
    s.execute("library.filter", &json!({"flags": ["pick", "none"]})).unwrap();
    assert_eq!(s.visible_cloned().len(), total - base.rejects);
    assert_eq!(s.tally(), base, "the counts are of the view without the quick filters");
    s.execute("library.clearFilter", &json!({})).unwrap();
    s.execute("library.filter", &json!({"unrated": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.stars[0]);
    s.execute("library.filter", &json!({"rating": 3, "ratingOp": "atLeast"})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.stars[0] + base.at_least(3), "unrated or three stars and more");
    s.execute("library.clearFilter", &json!({})).unwrap();
    s.execute("library.filter", &json!({"labels": ["green"], "unlabeled": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.label(lightcraft_catalog::ColorLabel::Green) + base.unlabeled);
    s.execute("library.filter", &json!({"labels": [], "unlabeled": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.unlabeled);
    let chips = crate::filter_chips(&s.filter, &s.catalog);
    assert!(chips.iter().any(|c| c.label.contains("no label")), "{chips:?}");
}

/// A stand-in Recycle Bin: each file is renamed `<name>.binned` beside itself.
fn fake_bin(paths: &[std::path::PathBuf]) -> std::result::Result<(), String> {
    for p in paths {
        std::fs::rename(p, format!("{}.binned", p.display())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn library_of_jpegs(tag: &str, n: usize) -> (Session, std::path::PathBuf, Vec<std::path::PathBuf>) {
    let dir = std::env::temp_dir().join(format!("lc-engine-rejects-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<std::path::PathBuf> = (0..n).map(|i| dir.join(format!("IMG_{i}.jpg"))).collect();
    for (i, p) in paths.iter().enumerate() {
        // each its own picture (the same bytes would be one photo imported once)
        let img = lightcraft_raster::Rgba8::from_fn(24, 16, |x, y| [(x * 9) as u8, (y * 13) as u8, (i * 40) as u8, 255]);
        std::fs::write(p, crate::export::encode_image(&img, &crate::export::ExportOptions::default()).unwrap()).unwrap();
    }
    let mut s = Session::new().with_fs();
    s.recycle = fake_bin;
    let list: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    s.execute("library.import", &json!({"paths": list})).unwrap();
    (s, dir, paths)
}

/// Delete Rejected Photos: every reject of the view goes in one step (whatever the flag filter
/// shows), to Recently Deleted (undoable, files kept), or with its files and own sidecar to the
/// Recycle Bin.
#[test]
fn delete_rejected_photos_keeps_or_recycles_the_files() {
    let (mut s, dir, paths) = library_of_jpegs("keep", 4);
    let ids: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    assert_eq!(ids.len(), 4);
    s.execute("photo.flag", &json!({"ids": [ids[0], ids[2]], "flag": "reject"})).unwrap();
    // only the picks shown: the rejects are still the view's to delete
    s.execute("library.filter", &json!({"flags": ["pick"]})).unwrap();
    let r = s.execute("library.deleteRejected", &json!({})).unwrap();
    assert_eq!(r["removed"], 2);
    s.execute("library.clearFilter", &json!({})).unwrap();
    assert_eq!(s.visible_cloned().len(), 2);
    assert!(paths.iter().all(|p| p.exists()), "the files stay");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.visible_cloned().len(), 4, "one undo step brings them back");
    assert_eq!(s.execute("library.deleteRejected", &json!({"files": "bin"})).unwrap_err().to_string().contains("keep or recycle"), true);
    let _ = std::fs::remove_dir_all(&dir);

    let (mut s, dir, paths) = library_of_jpegs("recycle", 3);
    let ids: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    let rejected = s.catalog.photo(lightcraft_catalog::PhotoId(ids[1])).and_then(|p| match &p.source {
        lightcraft_catalog::Source::File { path } => Some(std::path::PathBuf::from(path)),
        _ => None,
    });
    let rejected = rejected.unwrap();
    std::fs::write(rejected.with_extension("xmp"), b"sidecar").unwrap();
    s.execute("photo.flag", &json!({"ids": [ids[1]], "flag": "reject"})).unwrap();
    let r = s.execute("library.deleteRejected", &json!({"files": "recycle"})).unwrap();
    assert_eq!(r["removed"], 1);
    assert_eq!(r["recycled"].as_array().map(Vec::len), Some(2), "the photo and its own sidecar: {r}");
    assert!(!rejected.exists() && std::path::Path::new(&format!("{}.binned", rejected.display())).exists());
    assert!(!rejected.with_extension("xmp").exists());
    assert_eq!(s.visible_cloned().len(), 2);
    assert_eq!(paths.iter().filter(|p| p.exists()).count(), 2, "the others stay");
    assert_eq!(s.execute("library.deleteRejected", &json!({})).unwrap()["removed"], 0, "nothing left to delete");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sidecar shared by a raw + JPEG pair stays when only one of them goes.
#[test]
fn a_shared_sidecar_stays_with_the_photo_that_stays() {
    let dir = std::env::temp_dir().join(format!("lc-engine-shared-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (raw, jpg, xmp) = (dir.join("IMG_1.CR3"), dir.join("IMG_1.JPG"), dir.join("IMG_1.xmp"));
    for p in [&raw, &jpg, &xmp] {
        std::fs::write(p, b"x").unwrap();
    }
    let naming = crate::sidecar::SidecarNaming::Stem;
    assert!(crate::recycle::own_sidecars(std::slice::from_ref(&raw), naming).is_empty(), "the JPEG still needs it");
    assert_eq!(crate::recycle::own_sidecars(&[raw.clone(), jpg.clone()], naming), vec![xmp.clone()]);
    let _ = std::fs::remove_dir_all(&dir);
}
