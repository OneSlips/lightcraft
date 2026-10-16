//! The Import tab's import: browsed (Local) photos join the library with what the import sets (a
//! stack of presets, keywords, camera profiles), in place or copied / moved, and their culling
//! (flags, ratings, labels) and edits go with them.

use std::path::{Path, PathBuf};

use lightcraft_catalog::{Flag, PhotoId, Source};
use serde_json::json;

use crate::{Selection, Session};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-import-tab-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(p: &Path, seed: u8) {
    let img = lightcraft_raster::Rgba8::from_fn(16, 12, |x, y| [(x * 9) as u8, (y * 11) as u8, seed, 255]);
    let b =
        crate::export::encode_image(&img, &crate::export::ExportOptions { format: crate::export::ExportFormat::Png, ..Default::default() }).unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

/// Two presets: one sets exposure, the next contrast.
fn presets(s: &mut Session) {
    for (id, name, settings) in
        [("t.bright", "Bright", json!({"light": {"exposure": 0.5}})), ("t.punch", "Punch", json!({"light": {"contrast": 20.0}}))]
    {
        s.presets.push(lightcraft_develop::Preset {
            id: id.into(),
            name: name.into(),
            group: "Test".into(),
            settings,
            favorite: false,
            builtin: false,
        });
    }
}

/// A session with a library, browsing a folder of three photos (in `root/card`); the first is
/// picked and rated, the second rejected with a label.
fn browsing(tag: &str) -> (Session, PathBuf, Vec<PhotoId>) {
    let root = temp_dir(tag);
    let card = root.join("card");
    for (i, n) in ["a.png", "b.png", "c.png"].iter().enumerate() {
        png(&card.join(n), i as u8 * 40);
    }
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    presets(&mut s);
    s.execute("library.browse", &json!({"path": card.to_string_lossy()})).unwrap();
    let mut ids: Vec<(String, PhotoId)> = s.catalog.photos().filter(|p| p.local).map(|p| (p.file_name.clone(), p.id)).collect();
    ids.sort();
    let ids: Vec<PhotoId> = ids.into_iter().map(|(_, id)| id).collect();
    assert_eq!(ids.len(), 3);
    s.selection = Selection::single(ids[0]);
    s.execute("photo.flag", &json!({"flag": "pick"})).unwrap();
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    s.selection = Selection::single(ids[1]);
    s.execute("photo.flag", &json!({"flag": "reject"})).unwrap();
    s.execute("photo.label", &json!({"label": "red"})).unwrap();
    (s, root, ids)
}

fn path_of(s: &Session, id: PhotoId) -> String {
    match &s.catalog.photo(id).unwrap().source {
        Source::File { path } => path.clone(),
        Source::Demo { .. } => String::new(),
    }
}

#[test]
fn importing_browsed_photos_in_place_applies_the_preset_stack_and_keeps_the_culling() {
    let (mut s, _root, ids) = browsing("in-place");
    let paths: Vec<String> = ids.iter().map(|id| path_of(&s, *id)).collect();
    let n0 = s.catalog.len();
    let r = s.execute("library.import", &json!({"paths": paths, "presets": ["t.bright", "t.punch"], "keywords": ["Wedding"]})).unwrap();
    assert_eq!(r["imported"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(s.catalog.len(), n0, "the browsed records join the library: no new photos");
    let a = s.catalog.photo(ids[0]).unwrap();
    assert!(!a.local, "in the library now");
    assert_eq!((a.flag, a.rating), (Flag::Pick, 4), "the culling stays");
    assert_eq!(a.develop.light.exposure, 0.5, "the first preset of the stack");
    assert_eq!(a.develop.light.contrast, 20.0, "the second preset of the stack, on top");
    let steps: Vec<&str> = a.history.iter().map(|h| h.label.as_str()).collect();
    assert!(steps.ends_with(&["Preset: Bright", "Preset: Punch"]), "one History entry each, in order: {steps:?}");
    assert!(a.meta.keywords.iter().any(|k| k == "Wedding"), "{:?}", a.meta.keywords);
    let b = s.catalog.photo(ids[1]).unwrap();
    assert_eq!((b.flag, b.label), (Flag::Reject, Some(lightcraft_catalog::ColorLabel::Red)));
    // one undo step takes it all back out of the library, culling intact
    s.execute("edit.undo", &json!({})).unwrap();
    let a = s.catalog.photo(ids[0]).unwrap();
    assert!(a.local && a.develop.light.exposure == 0.0 && a.flag == Flag::Pick, "undo: browsed again, unedited, still picked");
}

#[test]
fn a_preset_and_a_stack_apply_in_order_and_an_unknown_one_refuses() {
    let (mut s, _root, ids) = browsing("order");
    let p = vec![path_of(&s, ids[2])];
    let e = s.execute("library.import", &json!({"paths": p, "presets": ["t.nope"]})).unwrap_err().to_string();
    assert!(e.contains("t.nope"), "{e}");
    assert!(s.catalog.photo(ids[2]).unwrap().local, "nothing imported");
    s.execute("library.import", &json!({"paths": p, "preset": "t.punch", "presets": ["t.bright"]})).unwrap();
    let c = s.catalog.photo(ids[2]).unwrap();
    let steps: Vec<&str> = c.history.iter().map(|h| h.label.as_str()).collect();
    assert!(steps.ends_with(&["Preset: Punch", "Preset: Bright"]), "{steps:?}");
}

#[test]
fn copying_browsed_photos_takes_their_records_along_to_the_copies() {
    let (mut s, root, ids) = browsing("copy");
    let paths: Vec<String> = ids.iter().map(|id| path_of(&s, *id)).collect();
    let dest = root.join("photos");
    let n0 = s.catalog.len();
    let r = s
        .execute(
            "library.import",
            &json!({"paths": paths, "mode": "copy", "destination": dest.to_string_lossy(), "organize": "flat", "presets": ["t.bright"]}),
        )
        .unwrap();
    assert_eq!(r["imported"].as_array().unwrap().len(), 3, "{r}");
    assert!(r["duplicates"].as_array().is_none_or(Vec::is_empty), "a browsed photo is not a duplicate of its own copy: {r}");
    assert_eq!(s.catalog.len(), n0, "no second record of the same photo");
    let a = s.catalog.photo(ids[0]).unwrap();
    assert!(!a.local);
    assert_eq!((a.flag, a.rating), (Flag::Pick, 4), "the culling goes with the copy");
    assert_eq!(a.develop.light.exposure, 0.5);
    let at = path_of(&s, ids[0]);
    assert!(Path::new(&at).starts_with(&dest) && Path::new(&at).exists(), "the record points at the copy: {at}");
    assert!(paths.iter().all(|p| Path::new(p).exists()), "copying leaves the card as it was");
}

#[test]
fn moving_browsed_photos_moves_their_files_and_records() {
    let (mut s, root, ids) = browsing("move");
    let paths: Vec<String> = ids.iter().map(|id| path_of(&s, *id)).collect();
    let dest = root.join("photos");
    s.execute("library.import", &json!({"paths": [paths[0]], "mode": "move", "destination": dest.to_string_lossy(), "organize": "flat"})).unwrap();
    let a = s.catalog.photo(ids[0]).unwrap();
    assert!(!a.local && a.flag == Flag::Pick);
    let at = path_of(&s, ids[0]);
    assert!(Path::new(&at).starts_with(&dest) && Path::new(&at).exists(), "{at}");
    assert!(!Path::new(&paths[0]).exists(), "moved: the original is gone from the card");
}

/// Edits and culling while browsing stay in LightCraft: with auto-write on, nothing is written
/// into the browsed folder until the photos are imported (unless `writeBrowsed` asks for it).
#[test]
fn browsing_writes_no_sidecars_until_import() {
    let (mut s, _root, ids) = browsing("xmp");
    s.execute("library.xmpPreferences", &json!({"autoWrite": true})).unwrap();
    let side = |s: &Session, id: PhotoId| crate::sidecar::sidecar_path(&path_of(s, id), Default::default());
    s.selection = Selection::single(ids[2]);
    s.execute("photo.rate", &json!({"rating": 3})).unwrap();
    assert!(!side(&s, ids[2]).exists(), "a browsed photo's culling is not written into the folder");
    s.execute("library.import", &json!({"paths": [path_of(&s, ids[2])]})).unwrap();
    s.selection = Selection::single(ids[2]);
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    assert!(side(&s, ids[2]).exists(), "imported: auto-write writes its sidecar");
    // asked for: browsed photos too
    s.execute("library.xmpPreferences", &json!({"writeBrowsed": true})).unwrap();
    s.selection = Selection::single(ids[0]);
    s.execute("photo.rate", &json!({"rating": 2})).unwrap();
    assert!(side(&s, ids[0]).exists());
}

/// No delete in Import: Delete on a browsed photo is refused with what to do instead, and Delete
/// Rejected Files from Disk needs the count repeated.
#[test]
fn browsed_photos_are_never_deleted_and_rejected_files_need_the_count() {
    let (mut s, _root, ids) = browsing("nodelete");
    s.selection = Selection::single(ids[0]);
    let e = s.execute("photo.delete", &json!({})).unwrap_err().to_string();
    assert!(e.contains("reject"), "{e}");
    assert!(!s.catalog.photo(ids[0]).unwrap().deleted, "nothing went to Recently Deleted");
    let e = s.execute("photo.deletePermanently", &json!({"ids": [ids[1].0]})).unwrap_err().to_string();
    assert!(e.contains("reject"), "{e}");
    assert!(s.catalog.photo(ids[1]).is_some());
    // one rejected photo (b): the wrong count is refused, nothing on disk changes
    let file = path_of(&s, ids[1]);
    let e = s.execute("import.deleteRejectedFiles", &json!({"confirm": 2})).unwrap_err().to_string();
    assert!(e.contains('1'), "{e}");
    let e = s.execute("import.deleteRejectedFiles", &json!({})).unwrap_err().to_string();
    assert!(e.contains("confirm"), "{e}");
    assert!(Path::new(&file).exists());
    // a library photo is never touched by it
    s.execute("library.import", &json!({"paths": [file]})).unwrap();
    let r = s.execute("import.deleteRejectedFiles", &json!({"confirm": 0})).unwrap();
    assert_eq!(r["deleted"], 0, "{r}");
    assert!(Path::new(&file).exists());
}

/// `import.targets` counts what the big button would take; `import.run` imports it and queues
/// the finishing.
#[test]
fn import_run_takes_the_picks_and_finishes_them() {
    let (mut s, _root, ids) = browsing("run");
    s.selection = lightcraft_engine_selection(&[ids[1], ids[2]]);
    let t = s.execute("import.targets", &json!({"what": "picks"})).unwrap();
    assert_eq!((t["picks"].as_u64(), t["selected"].as_u64(), t["all"].as_u64()), (Some(1), Some(2), Some(3)), "{t}");
    assert_eq!(t["paths"].as_array().unwrap().len(), 1);
    let r = s.execute("import.run", &json!({"what": "picks", "presets": ["t.bright"], "previews": "none"})).unwrap();
    assert_eq!(r["imported"].as_array().unwrap().len(), 1, "{r}");
    assert!(!s.catalog.photo(ids[0]).unwrap().local && s.catalog.photo(ids[1]).unwrap().local, "only the pick");
    // nothing to compute here: the queue is done at once, with its note
    let mut note = None;
    for _ in 0..100 {
        note = s.finishing_tick().or(note);
        if s.finishing.is_none() {
            break;
        }
    }
    assert!(s.finishing.is_none());
    // (a preset without AI masks, no Smooth Skin, no previews: the note says no more than that)
    assert_eq!(s.finishing_note.as_deref(), Some("1 photo ready"), "{note:?}");
    assert!(s.finishing_done.is_some_and(|d| !d.did_something() && d.masks == 0), "{:?}", s.finishing_done);
    let e = s.execute("import.run", &json!({"what": "picks"})).unwrap_err().to_string();
    assert!(e.contains("no picks"), "{e}");
}

fn lightcraft_engine_selection(ids: &[PhotoId]) -> Selection {
    Selection { ids: ids.to_vec(), active: ids.last().copied() }
}

/// An unfinished queue is kept in the library and picked up when it opens again.
#[test]
fn an_unfinished_finishing_queue_resumes_after_a_restart() {
    let (mut s, root, ids) = browsing("resume");
    let paths: Vec<String> = ids.iter().map(|id| path_of(&s, *id)).collect();
    s.execute("library.import", &json!({"paths": paths})).unwrap();
    // a preview build already running keeps the queue waiting at its previews
    s.preview_build = Some(std::sync::Arc::new(crate::cmd::previews::PreviewBuild { total: 1, ..Default::default() }));
    s.finish_import(crate::finishing::FinishPlan { ids: ids.iter().map(|i| i.0).collect(), previews: "standard".into(), started: String::new() });
    assert!(s.finishing.is_some(), "waiting for the previews");
    s.persist().unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    assert!(s.finish_pending.as_ref().is_some_and(|p| p.ids.len() == 3 && p.previews == "standard"), "the plan was kept");
    s.finishing_tick();
    assert!(s.finishing.is_some() || s.finishing_note.is_some(), "resumed");
    // done: the plan is cleared from the library
    let r = s.execute("library.finishImport", &json!({"ids": [ids[0].0], "previews": "none", "wait": true, "timeout": 60})).unwrap();
    assert_eq!(r["running"], false, "{r}");
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    assert!(s.finish_pending.is_none(), "nothing left to resume");
}

/// Import presets keep the Import tab's settings by name, with the library.
#[test]
fn import_presets_are_saved_with_the_library() {
    let (mut s, root, _ids) = browsing("presets");
    s.execute("import.savePreset", &json!({"name": "Wedding", "settings": {"mode": "copy", "presets": ["t.bright"], "previews": "full"}})).unwrap();
    s.execute("import.savePreset", &json!({"name": "wedding", "settings": {"mode": "add"}})).unwrap();
    assert_eq!(s.import_presets.len(), 1, "same name (any case): replaced");
    assert!(s.execute("import.savePreset", &json!({"name": " ", "settings": {}})).is_err());
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    let list = s.execute("import.presets", &json!({})).unwrap();
    assert_eq!(list[0]["settings"]["mode"], "add", "{list}");
    s.execute("import.deletePreset", &json!({"name": "WEDDING"})).unwrap();
    assert!(s.import_presets.is_empty());
}

/// LR-IMP-NOTICE: the finishing queue counts the AI masks the presets really bring: none for a
/// preset without masks, each mask of each photo for one with them.
#[test]
fn the_finishing_counts_only_real_ai_masks() {
    use lightcraft_develop::{Mask, MaskComponent, MaskOp, MaskShape};
    let (mut s, _root, ids) = browsing("notice");
    let sky = Mask {
        name: "Sky".into(),
        components: vec![MaskComponent { name: None, anchor: None, op: MaskOp::Add, invert: false, shape: MaskShape::Sky { seg: None } }],
        ..Default::default()
    };
    s.presets.push(lightcraft_develop::Preset {
        id: "t.sky".into(),
        name: "Sky".into(),
        group: "Test".into(),
        settings: json!({"masks": [serde_json::to_value(&sky).unwrap()]}),
        favorite: false,
        builtin: false,
    });
    s.selection = lightcraft_engine_selection(&[ids[0]]);
    // (no model here: the queue is done at once; while it runs, `import.status` has the counts too)
    let summary = |s: &mut Session| {
        for _ in 0..100 {
            s.finishing_tick();
            if s.finishing.is_none() {
                break;
            }
        }
        s.finishing_done.unwrap()
    };
    s.execute("import.run", &json!({"what": "selected", "presets": ["t.bright"]})).unwrap();
    let done = summary(&mut s);
    assert_eq!((done.masks, done.did_something()), (0, false), "{done:?}");
    s.selection = lightcraft_engine_selection(&[ids[1], ids[2]]);
    s.execute("import.run", &json!({"what": "selected", "presets": ["t.bright", "t.sky"]})).unwrap();
    let done = summary(&mut s);
    assert_eq!((done.masks, done.mask_photos, done.photos), (2, 2, 2), "{done:?}");
    assert_eq!(done.note(), "2 photos ready: 2 AI masks on 2 photos");
}
