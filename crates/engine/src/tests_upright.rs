//! Upright per photo (test notes 2026-10-24 notes 4, 5, 6 and 16): a mode that reaches other photos is
//! analysed on each, in the background in the app; a second click analyses afresh; Constrain Crop's fit grows back
//! when the warp goes and keeps the photo's own shape.

use std::sync::Arc;

use lightcraft_catalog::PhotoId;
use lightcraft_develop::Upright;
use serde_json::json;

use crate::Session;
use crate::media::SourceLevel;

/// A photo of white posts on a dark wall, the camera rolled by `roll` degrees, as photo `id`'s source.
fn posts(s: &mut Session, id: PhotoId, roll: f64) {
    let (w, h) = (600usize, 400usize);
    let (sn, cs) = roll.to_radians().sin_cos();
    let img = lightcraft_raster::Rgb32f::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f64 - 300.0, y as f64 - 200.0);
        // the scene, turned back
        let u = dx * cs + dy * sn;
        let on = (u + 1000.0).rem_euclid(110.0) < 22.0;
        if on { [0.9; 3] } else { [0.08; 3] }
    });
    let img = Arc::new(img);
    s.media.insert_with_info(id, SourceLevel::Thumb, img.clone(), Default::default());
    s.media.insert_with_info(id, SourceLevel::Preview, img, Default::default());
}

/// Three demo photos, each with posts rolled differently, all selected (the first active) with Auto Sync.
fn three(background: bool) -> (Session, [PhotoId; 3]) {
    let mut s = Session::with_demo();
    s.upright_jobs.background = background;
    let ids: Vec<PhotoId> = s.visible_cloned().into_iter().take(3).collect();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    for (id, roll) in [(a, 2.0), (b, -3.0), (c, 1.0)] {
        posts(&mut s, id, roll);
    }
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    (s, [a, b, c])
}

fn transform(s: &Session, id: PhotoId) -> Option<lightcraft_geom::Homography> {
    s.develop_of(id).unwrap().geometry.upright_transform
}

/// Note 4a: Auto Sync carries the MODE; every photo is analysed on its own pixels, off the UI thread with one
/// activity row, the results merged into the settings as they arrive; one undo step for all.
#[test]
fn auto_sync_analyses_upright_on_each_photo_in_the_background() {
    let (mut s, [a, b, c]) = three(true);
    let undo = s.undo.len();
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    assert_eq!(s.undo.len(), undo + 1, "one undo step");
    assert!(transform(&s, a).is_some(), "the active photo is analysed at once");
    for id in [b, c] {
        assert_eq!(s.develop_of(id).unwrap().geometry.upright, Upright::Vertical, "the mode travels");
    }
    // (the worker may be quick, but the results only land when polled)
    assert!(transform(&s, b).is_none() && transform(&s, c).is_none(), "not the active photo's correction");
    assert_eq!(s.upright_jobs.pending(), 2);
    assert!(s.activity.list().iter().any(|t| t.kind == "upright"), "an activity row while it runs");
    let t0 = std::time::Instant::now();
    while s.upright_jobs.pending() > 0 {
        s.upright_poll();
        assert!(t0.elapsed().as_secs() < 60, "the analyses finish");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!s.activity.list().iter().any(|t| t.kind == "upright"), "the row goes when done");
    let (ta, tb, tc) = (transform(&s, a).unwrap(), transform(&s, b).unwrap(), transform(&s, c).unwrap());
    assert_eq!(Some(tb), s.upright_for(b, Upright::Vertical).unwrap(), "b's own correction");
    assert_eq!(Some(tc), s.upright_for(c, Upright::Vertical).unwrap(), "c's own correction");
    assert!(ta != tb && tb != tc, "each photo its own");
    assert_eq!(s.undo.len(), undo + 1, "the results are no undo step of their own");
    s.execute("edit.undo", &json!({})).unwrap();
    for id in [a, b, c] {
        assert_eq!(s.develop_of(id).unwrap().geometry.upright, Upright::Off, "one undo takes all back");
    }
}

/// Note 4a: Paste (and presets, Sync) carry Upright as a recipe too: the photo pasted on gets its own
/// correction, never the copied photo's; without a worker (CLI, MCP) at once.
#[test]
fn pasted_upright_is_analysed_on_the_photo_it_lands_on() {
    let (mut s, [a, b, _]) = three(false);
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("library.select", &json!({"ids": [a.0], "active": a.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["geometry"]})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0], "active": b.0})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    let gb = s.develop_of(b).unwrap().geometry.clone();
    assert_eq!(gb.upright, Upright::Vertical);
    assert_eq!(gb.upright_transform, s.upright_for(b, Upright::Vertical).unwrap(), "b's own correction");
    assert_ne!(gb.upright_transform, transform(&s, a), "not a's");
}

/// Decision 2026-10-25 (item 12): Guided Upright is drawn on one photo's lines, so syncing it is skipped: Auto
/// Sync, Paste and presets leave the other photos with the Upright they have (mode, correction, guides); the
/// Transform sliders still travel.
#[test]
fn guided_upright_is_not_synced_the_other_photos_keep_their_own() {
    let (mut s, [a, b, c]) = three(false);
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0], "active": b.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    let gb = s.develop_of(b).unwrap().geometry.clone();
    assert!(gb.upright_transform.is_some());
    // Auto Sync: Guided on a, b keeps Vertical with its own correction, c stays Off
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    s.execute("geometry.guides", &json!({"guides": [[0.2, 0.1, 0.25, 0.9], [0.8, 0.1, 0.75, 0.9]]})).unwrap();
    s.execute("develop.set", &json!({"control": "geometry.rotate", "value": 2.0})).unwrap();
    assert_eq!(s.develop_of(a).unwrap().geometry.upright, Upright::Guided);
    let g = s.develop_of(b).unwrap().geometry.clone();
    assert_eq!((g.upright, g.upright_transform, g.guides.len()), (Upright::Vertical, gb.upright_transform, 0), "b keeps its own");
    assert_eq!(g.rotate, 2.0, "the Transform sliders still travel");
    assert_eq!(s.develop_of(c).unwrap().geometry.upright, Upright::Off, "c keeps Off");
    // Copy / Paste (and presets: the same recipe rule) of a Guided photo
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("library.select", &json!({"ids": [a.0], "active": a.0})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["geometry"]})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0], "active": b.0})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    let g = s.develop_of(b).unwrap().geometry.clone();
    assert_eq!((g.upright, g.upright_transform, g.guides.len()), (Upright::Vertical, gb.upright_transform, 0), "paste keeps b's own");
    // a mode that isn't Guided still travels
    s.execute("library.select", &json!({"ids": [a.0], "active": a.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "level"})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["geometry"]})).unwrap();
    s.execute("library.select", &json!({"ids": [c.0], "active": c.0})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    assert_eq!(s.develop_of(c).unwrap().geometry.upright, Upright::Level);
}

/// Note 6 and decision 2026-10-25 (item 11): a second click on the mode that is on analyses it afresh and
/// (Guided) clears the guides, one undo step; the Transform sliders the photographer set stay.
#[test]
fn a_second_click_on_the_upright_mode_re_analyses_and_keeps_the_sliders() {
    let (mut s, [a, ..]) = three(false);
    s.execute("library.select", &json!({"ids": [a.0], "active": a.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    s.execute("develop.set", &json!({"control": "geometry.rotate", "value": 4.0})).unwrap();
    s.execute("develop.set", &json!({"control": "geometry.vertical", "value": 20.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "vertical", "again": true})).unwrap();
    let g = s.develop_of(a).unwrap().geometry.clone();
    assert_eq!((g.rotate, g.vertical), (4.0, 20.0), "the Transform sliders are kept");
    assert_eq!(g.upright, Upright::Vertical);
    assert_eq!(g.upright_transform, s.upright_for(a, Upright::Vertical).unwrap());
    // Guided: the guides go
    s.execute("geometry.guides", &json!({"guides": [[0.2, 0.1, 0.25, 0.9], [0.8, 0.1, 0.75, 0.9]]})).unwrap();
    assert_eq!(s.develop_of(a).unwrap().geometry.guides.len(), 2);
    let undo = s.undo.len();
    s.execute("geometry.upright", &json!({"mode": "guided", "again": true})).unwrap();
    let g = s.develop_of(a).unwrap().geometry.clone();
    assert_eq!(g.upright, Upright::Guided);
    assert!(g.guides.is_empty(), "a second click on Guided clears the guides");
    assert_eq!((g.rotate, g.vertical), (4.0, 20.0), "and keeps the sliders");
    assert_eq!(s.undo.len(), undo + 1, "one undo step");
    // a plain run (the Update button, an agent) keeps the sliders
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    s.execute("develop.set", &json!({"control": "geometry.rotate", "value": 3.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    assert_eq!(s.develop_of(a).unwrap().geometry.rotate, 3.0);
}

/// Notes 5 and 16: under Constrain Crop, Upright shrinks an untouched crop to fit the warp and the crop tool
/// still shows it untouched (locked to the remembered ratio, the photo's own shape kept); Upright Off makes it
/// the whole photo again (it left a strip), and so does a Reset with Upright on followed by Off.
#[test]
fn constrain_crop_grows_back_when_upright_goes_and_keeps_the_shape() {
    let (mut s, [a, ..]) = three(false);
    s.execute("library.select", &json!({"ids": [a.0], "active": a.0})).unwrap();
    s.execute("develop.merge", &json!({"settings": {"geometry": {"constrain_crop": true}}})).unwrap();
    let full = lightcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0);
    let rect = |s: &Session| s.develop_of(a).unwrap().crop.geometry.rect;
    let (w, h) = crate::cmd::develop::crop_dims(&s, a);
    let shape = |s: &Session| {
        let r = s.develop_of(a).unwrap().crop.geometry.rect_px(w, h);
        r.width() / r.height()
    };
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    assert!(rect(&s).x0 > 0.001, "fitted inside the warped photo: {:?}", rect(&s));
    assert!((shape(&s) - w / h).abs() < 0.01 * w / h, "the photo's own shape: {}", shape(&s));
    assert!(s.crop_untouched(a), "the tool still shows the crop as untouched (locked to Original)");
    s.execute("geometry.upright", &json!({"mode": "off"})).unwrap();
    let r = rect(&s);
    assert!(
        (r.x0 - full.x0).abs() < 1e-6 && (r.x1 - full.x1).abs() < 1e-6 && (r.y0 - full.y0).abs() < 1e-6 && (r.y1 - full.y1).abs() < 1e-6,
        "{r:?}"
    );
    // Reset with Upright on, then Off
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    s.execute("crop.reset", &json!({})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "off"})).unwrap();
    let r = rect(&s);
    assert!((r.x0).abs() < 1e-6 && (r.x1 - 1.0).abs() < 1e-6 && (r.y0).abs() < 1e-6 && (r.y1 - 1.0).abs() < 1e-6, "{r:?}");
    // a crop drawn by the photographer only shrinks, and comes back as drawn
    s.execute("crop.aspect", &json!({"aspect": "3x2"})).unwrap();
    s.execute("crop.set", &json!({"rect": [0.2, 0.2, 0.7, 0.7]})).ok();
    let drawn = rect(&s);
    let locked = s.develop_of(a).unwrap().crop.aspect;
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    assert_eq!(s.develop_of(a).unwrap().crop.aspect, locked, "the lock stays");
    s.execute("geometry.upright", &json!({"mode": "off"})).unwrap();
    assert!(rect(&s).width() <= drawn.width() + 1e-9, "a drawn crop doesn't grow past what was drawn");
}
