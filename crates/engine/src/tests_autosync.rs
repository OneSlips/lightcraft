//! Auto Sync and live multi-photo edits: every photo gets the active photo's value (note 18).

use serde_json::json;

use crate::Session;

fn demo() -> Session {
    Session::with_demo()
}

/// Give photo `id` a decoded raw source (thumbnail and preview) with as-shot white `(temp, tint)`;
/// `legacy`: a camera whose settings without a scale were written on the relative scale.
fn raw_as_shot(s: &mut Session, id: lightcraft_catalog::PhotoId, temp: f64, tint: f64, legacy: bool, warm: f32) {
    use crate::media::SourceLevel;
    let mut img = lightcraft_raster::Rgb32f::new(48, 32);
    for (i, px) in img.data.iter_mut().enumerate() {
        let v = 0.1 + 0.6 * ((i % 48) as f32 / 48.0);
        *px = [v * warm, v, v / warm];
    }
    let img = std::sync::Arc::new(img);
    let info =
        lightcraft_pipeline::SourceInfo { raw: true, legacy_relative_wb: legacy, as_shot_temp: temp, as_shot_tint: tint, ..Default::default() };
    s.media.insert_with_info(id, SourceLevel::Thumb, img.clone(), info.clone());
    s.media.insert_with_info(id, SourceLevel::Preview, img, info);
}

fn pid(n: u64) -> lightcraft_catalog::PhotoId {
    lightcraft_catalog::PhotoId(n)
}

/// The white balance photo `id` renders with (Kelvin, tint).
fn wb(s: &Session, id: lightcraft_catalog::PhotoId) -> (f64, f64) {
    lightcraft_pipeline::local::effective_wb(&s.source_info(id), &s.develop_of(id).unwrap())
}

fn exp(s: &Session, id: lightcraft_catalog::PhotoId) -> f64 {
    s.develop_of(id).unwrap().light.exposure
}

/// Three demo photos a (active), b, c selected with Auto Sync on; b at Exposure +1, c at +0.5.
fn three() -> (Session, [lightcraft_catalog::PhotoId; 3]) {
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(3).map(|p| p.0).collect();
    let (a, b, c) = (pid(ids[0]), pid(ids[1]), pid(ids[2]));
    for (id, e) in [(b, 1.0), (c, 0.5)] {
        s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
        s.execute("develop.set", &json!({"control": "light.exposure", "value": e})).unwrap();
    }
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    (s, [a, b, c])
}

/// Note 18 (the user's decision, 24 Oct): Auto Sync sets every selected photo to the active
/// photo's value, as Lightroom Classic does: live during a slider drag, from the console's dials
/// (`develop.adjust` in a multi-photo interaction), for a typed value, a key or a reset; one undo
/// step for all; clamped like the active photo. The on-screen display counts the photos that
/// actually changed (one already at the value isn't). Quick Develop's +/- stay relative.
#[test]
fn auto_sync_sets_every_photo_to_the_active_photos_value() {
    let (mut s, [a, b, c]) = three();
    // a slider drag: live
    s.execute("develop.beginInteraction", &json!({"label": "Exposure", "ids": [b.0, c.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.25})).unwrap();
    assert!((exp(&s, b) - 0.25).abs() < 1e-9 && (exp(&s, c) - 0.25).abs() < 1e-9, "b {} c {}", exp(&s, b), exp(&s, c));
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.5})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    assert!([a, b, c].iter().all(|id| (exp(&s, *id) - 0.5).abs() < 1e-9));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(exp(&s, a).abs() < 1e-9 && (exp(&s, b) - 1.0).abs() < 1e-9 && (exp(&s, c) - 0.5).abs() < 1e-9, "one undo step for all");
    // the console's dials: relative ticks on the active photo, its value on the others
    s.execute("develop.beginInteraction", &json!({"label": "Exposure", "ids": [b.0, c.0]})).unwrap();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.5})).unwrap();
    assert!((exp(&s, b) - 0.5).abs() < 1e-9, "b takes a's value, not +0.5 from its own: {}", exp(&s, b));
    // c was at +0.5 already: two photos changed, not three
    assert_eq!(s.last_change.as_ref().map(|c| c.photos), Some(2), "{:?}", s.last_change);
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 9.0})).unwrap();
    assert!([a, b, c].iter().all(|id| (exp(&s, *id) - 5.0).abs() < 1e-9), "clamped alike");
    s.execute("develop.endInteraction", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    // a typed value or a key outside a drag: the same, in one step
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.7})).unwrap();
    assert!([a, b, c].iter().all(|id| (exp(&s, *id) - 0.7).abs() < 1e-9));
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.1})).unwrap();
    assert!([a, b, c].iter().all(|id| (exp(&s, *id) - 0.8).abs() < 1e-9), "a key nudge: the active photo's new value");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((exp(&s, b) - 1.0).abs() < 1e-9 && (exp(&s, c) - 0.5).abs() < 1e-9);
    // Quick Develop's +/- buttons stay relative: each photo from its own value
    s.execute("develop.quickAdjust", &json!({"control": "light.exposure", "delta": 0.5})).unwrap();
    assert!((exp(&s, a) - 0.5).abs() < 1e-9 && (exp(&s, b) - 1.5).abs() < 1e-9 && (exp(&s, c) - 1.0).abs() < 1e-9);
}

/// Note 18 and the old blue photos (test notes 2026-10-15): white balance syncs as the same Kelvin
/// and tint on each photo's own scale, whatever the path (drag, dial, typed value), for photos
/// with another as-shot white and one still holding an old relative-scale value (which, read as
/// Kelvin, turned photos blue). A Temp move leaves each photo's own tint. Modes go on as modes:
/// As Shot gives each photo its own as-shot white, a light preset its numbers, Auto each photo's
/// own from its pixels.
#[test]
fn white_balance_syncs_the_same_kelvin_on_each_photos_own_scale_and_modes_per_photo() {
    use lightcraft_develop::{WbMode, WbScale, WhiteBalance};
    let (mut s, [a, b, c]) = three();
    raw_as_shot(&mut s, a, 5200.0, 4.0, false, 1.0);
    raw_as_shot(&mut s, b, 3300.0, -12.0, false, 1.4);
    raw_as_shot(&mut s, c, 6100.0, 8.0, true, 0.7);
    // c holds an old relative-scale white: 7000 K there meant "a little warmer than as shot"
    let mut d = (*s.develop_of(c).unwrap()).clone();
    d.wb = WhiteBalance { mode: WbMode::Custom, temp: 7000.0, tint: 0.0, scale: WbScale::Legacy };
    s.catalog.apply(lightcraft_catalog::Op::SetDevelop { id: c, settings: std::sync::Arc::new(d), label: "x".into(), edited: None }).unwrap();
    let (b_tint, c_tint) = (wb(&s, b).1, wb(&s, c).1);
    let close = |x: (f64, f64), y: (f64, f64)| (x.0 - y.0).abs() < 1.0 && (x.1 - y.1).abs() < 0.5;
    // a typed Temp
    s.execute("develop.set", &json!({"control": "wb.temp", "value": 5600.0})).unwrap();
    assert!((wb(&s, a).0 - 5600.0).abs() < 1.0);
    assert!(close(wb(&s, b), (5600.0, b_tint)), "b: {:?}", wb(&s, b));
    assert!(close(wb(&s, c), (5600.0, c_tint)), "c (old relative value): {:?}", wb(&s, c));
    s.execute("edit.undo", &json!({})).unwrap();
    // a drag and the dials
    s.execute("develop.beginInteraction", &json!({"label": "Temp", "ids": [b.0, c.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "wb.temp", "value": 4800.0})).unwrap();
    s.execute("develop.adjust", &json!({"control": "wb.tint", "delta": 6.0})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    let want = wb(&s, a);
    assert!((want.0 - 4800.0).abs() < 1.0 && (want.1 - 10.0).abs() < 0.5, "a: {want:?}");
    assert!(close(wb(&s, b), want) && close(wb(&s, c), want), "b {:?} c {:?} a {want:?}", wb(&s, b), wb(&s, c));
    // As Shot: each photo's own
    s.execute("develop.wb", &json!({"mode": "asShot"})).unwrap();
    assert!(
        close(wb(&s, a), (5200.0, 4.0)) && close(wb(&s, b), (3300.0, -12.0)) && close(wb(&s, c), (6100.0, 8.0)),
        "{:?} {:?} {:?}",
        wb(&s, a),
        wb(&s, b),
        wb(&s, c)
    );
    assert!([a, b, c].iter().all(|id| s.develop_of(*id).unwrap().wb.mode == WbMode::AsShot));
    // a light preset: its numbers on every raw
    s.execute("develop.wb", &json!({"mode": "tungsten"})).unwrap();
    assert!([a, b, c].iter().all(|id| close(wb(&s, *id), (2850.0, 0.0))));
    // Auto: each photo's own, from its own pixels
    s.execute("develop.wb", &json!({"mode": "auto"})).unwrap();
    for id in [b, c] {
        let src = s.source_now(id, crate::media::SourceLevel::Thumb).unwrap();
        let own = lightcraft_pipeline::auto::auto_wb(&src, &s.source_info(id));
        assert!(close(wb(&s, id), own), "photo {}: {:?} vs its own Auto {own:?}", id.0, wb(&s, id));
        assert_eq!(s.develop_of(id).unwrap().wb.mode, WbMode::Auto);
    }
    assert!(!close(wb(&s, a), wb(&s, b)), "not a's Auto copied over");
}

/// Note 18 / note 4: a crop goes on as its shape and angle, fitted to each photo's frame (a
/// portrait photo takes the locked ratio turned); an Upright mode is analysed on each photo, never
/// the active photo's correction copied over.
#[test]
fn crop_and_upright_sync_as_modes_fitted_to_each_photo() {
    let (mut s, [a, b, c]) = three();
    raw_as_shot(&mut s, a, 5200.0, 0.0, false, 1.0);
    raw_as_shot(&mut s, b, 5200.0, 0.0, false, 1.3);
    let dims = |s: &Session, id| crate::cmd::develop::crop_dims(s, id);
    let (aw, ah) = dims(&s, a);
    let wide = aw > ah;
    // b is turned the way a is, c the other way
    if (dims(&s, b).0 > dims(&s, b).1) != wide {
        s.execute("photo.rotateRight", &json!({"ids": [b.0]})).unwrap();
    }
    if (dims(&s, c).0 > dims(&s, c).1) == wide {
        s.execute("photo.rotateRight", &json!({"ids": [c.0]})).unwrap();
    }
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    let (lock, turned) = if wide { ((300, 200), (200, 300)) } else { ((200, 300), (300, 200)) };
    let ratio = |l: (u32, u32)| f64::from(l.0) / f64::from(l.1);
    let mut d = (*s.develop_of(a).unwrap()).clone();
    d.crop.aspect = Some(lock);
    d.crop.geometry = lightcraft_geom::crop_fit_angle(aw, ah, 3.0, Some(ratio(lock)));
    s.set_develop(a, d, "Crop").unwrap();
    let shape = |s: &Session, id| {
        let (w, h) = dims(s, id);
        let r = s.develop_of(id).unwrap().crop.geometry.rect_px(w, h);
        r.width() / r.height()
    };
    let cb = s.develop_of(b).unwrap().crop;
    assert_eq!(cb.aspect, Some(lock));
    assert!((cb.geometry.angle - 3.0).abs() < 1e-9);
    let cc = s.develop_of(c).unwrap().crop;
    assert_eq!(cc.aspect, Some(turned), "the ratio turned for the photo turned the other way");
    assert!((cc.geometry.angle - 3.0).abs() < 1e-9);
    assert!((shape(&s, c) - ratio(turned)).abs() < 0.01, "c's crop has the turned shape on its own frame: {}", shape(&s, c));
    let (cw, ch) = dims(&s, c);
    let fit = lightcraft_geom::crop_fit_angle(cw, ch, 3.0, Some(ratio(turned)));
    assert!((cc.geometry.rect.x0 - fit.rect.x0).abs() < 1e-6 && (cc.geometry.rect.y1 - fit.rect.y1).abs() < 1e-6, "the largest that fits");
    // Upright Vertical: each photo's own analysis
    s.execute("geometry.upright", &json!({"mode": "vertical"})).unwrap();
    let own = s.upright_for(b, lightcraft_develop::Upright::Vertical).unwrap();
    let gb = s.develop_of(b).unwrap().geometry.clone();
    assert_eq!(gb.upright, lightcraft_develop::Upright::Vertical);
    assert_eq!(gb.upright_transform, own, "b's own correction");
    s.execute("geometry.upright", &json!({"mode": "off"})).unwrap();
    assert!([a, b, c].iter().all(|id| s.develop_of(*id).unwrap().geometry.upright_transform.is_none()));
}

/// Note 51: Develop's Presets panel (`followAutoSync`) follows the slider rule. Auto Sync off: the
/// active photo only; on: every selected photo, one undo step restoring all of them. Without the
/// flag (Quick Develop, "Apply to Selected Photos", agents) the preset goes on the selection.
#[test]
fn a_develop_preset_follows_auto_sync_like_a_slider() {
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(3).map(|p| p.0).collect();
    let all: Vec<lightcraft_catalog::PhotoId> = ids.iter().copied().map(lightcraft_catalog::PhotoId).collect();
    let before: Vec<_> = all.iter().map(|id| s.develop_of(*id).unwrap()).collect();
    let changed = |s: &Session| -> Vec<bool> { all.iter().zip(&before).map(|(id, b)| s.develop_of(*id).unwrap() != *b).collect() };
    s.execute("library.select", &json!({"ids": ids, "active": ids[1]})).unwrap();
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    let pid = "lc.golden-hour";

    let r = s.execute("preset.apply", &json!({"id": pid, "followAutoSync": true})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!(changed(&s), [false, true, false], "Auto Sync off: the active photo only");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(changed(&s), [false, false, false]);

    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    let r = s.execute("preset.apply", &json!({"id": pid, "followAutoSync": true})).unwrap();
    assert_eq!(r["changed"], 3);
    assert_eq!(changed(&s), [true, true, true], "Auto Sync on: every selected photo");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(changed(&s), [false, false, false], "one undo restores every photo it touched");

    // on purpose, on the whole selection (Quick Develop, the right-click item), Auto Sync or not
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    let r = s.execute("preset.apply", &json!({"id": pid})).unwrap();
    assert_eq!(r["changed"], 3);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(changed(&s), [false, false, false]);
    // explicit ids still win
    s.execute("preset.apply", &json!({"id": pid, "ids": [ids[2]], "followAutoSync": true})).unwrap();
    assert_eq!(changed(&s), [false, false, true]);
}

/// A stored segmentation for `mask.add {seg}` (no model needed), all `v`.
fn seg(v: f32) -> serde_json::Value {
    serde_json::to_value(lightcraft_develop::SegMask::from_logits(4, &[v; 16])).unwrap()
}

fn masks(s: &Session, id: lightcraft_catalog::PhotoId) -> Vec<lightcraft_develop::Mask> {
    s.develop_of(id).unwrap().masks.clone()
}

/// Note 1: with Auto Sync on, an AI mask made on the active photo goes to the other selected photos
/// as a recipe, to be computed from each photo's own pixels: never the active photo's bitmap, for
/// every AI kind (an Object's clicks stay on the photo they were made on). The other photos keep
/// their own masks (merged, not replaced); a later edit of the mask carries only what changed and
/// leaves each photo's own computed selection; deleting it deletes each copy; one undo step each.
#[test]
fn auto_sync_carries_ai_masks_as_recipes_and_keeps_each_photos_own_masks() {
    use lightcraft_develop::MaskShape;
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(3).map(|p| p.0).collect();
    let [a, b, c] = [0, 1, 2].map(|i| lightcraft_catalog::PhotoId(ids[i]));
    // b has a hand-drawn mask of its own
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "linear", "start": [0.5, 0.0], "end": [0.5, 0.4], "name": "B's own"})).unwrap();
    s.execute("mask.adjust", &json!({"values": {"exposure": -1.0}})).unwrap();
    let b_own = masks(&s, b);
    assert_eq!(b_own.len(), 1);
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();

    // every AI kind, made on a with its segmentation
    let kinds = [
        json!({"kind": "subject"}),
        json!({"kind": "sky"}),
        json!({"kind": "background"}),
        json!({"kind": "people", "parts": ["Clothes"]}),
        json!({"kind": "landscape", "class": "water"}),
        json!({"kind": "concept", "concept": "Wedding dress"}),
        json!({"kind": "object", "points": [[0.4, 0.5]]}),
    ];
    for k in &kinds {
        let mut p = k.clone();
        p["seg"] = seg(2.0);
        s.execute("mask.add", &p).unwrap();
    }
    let on_a = masks(&s, a);
    assert_eq!(on_a.len(), kinds.len());
    assert!(on_a.iter().all(|m| m.components[0].shape.has_seg()), "a keeps the selections made on it");
    for (photo, before) in [(b, b_own.len()), (c, 0)] {
        let got = masks(&s, photo);
        assert_eq!(got.len(), before + kinds.len(), "merged into the photo's own masks: {got:#?}");
        if photo == b {
            assert_eq!(got[0], b_own[0], "b's own mask stays as it was");
        }
        for m in &got[before..] {
            let shape = &m.components[0].shape;
            assert!(shape.is_ai() && !shape.has_seg(), "a recipe, never a's bitmap: {shape:?}");
            assert!(m.needs_compute(), "computed on this photo");
            if let MaskShape::Object { hint, exclude, .. } = shape {
                assert!(hint.is_empty() && exclude.is_empty(), "an Object's clicks stay on a: picked again here");
            }
        }
        let ids: std::collections::HashSet<u32> = got.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), got.len(), "no two masks share an id");
    }

    // b's Subject gets its own selection (as computing it on b would)
    let mut d = (*s.develop_of(b).unwrap()).clone();
    let own = lightcraft_develop::SegMask::from_logits(4, &[-3.0; 16]);
    if let MaskShape::Subject { seg } = &mut d.masks[1].components[0].shape {
        *seg = Some(own.clone());
    }
    s.amend_develop(b, d);

    // an edit of the Subject mask on a: only the changed setting travels; b keeps its own pixels
    let subject = on_a[0].id;
    s.execute("mask.select", &json!({"id": subject})).unwrap();
    s.execute("mask.adjust", &json!({"values": {"exposure": 0.7}})).unwrap();
    let got = masks(&s, b);
    assert_eq!(got[0], b_own[0], "b's own mask is untouched by an edit of another");
    assert!((got[1].adjust.exposure - 0.7).abs() < 1e-9, "the change travels: {:?}", got[1].adjust);
    assert_eq!(got[1].components[0].shape, MaskShape::Subject { seg: Some(own.clone()) }, "b keeps the Subject computed on it");
    assert!((masks(&s, c)[0].adjust.exposure - 0.7).abs() < 1e-9);
    // recomputing a's selection carries nothing
    let before_b = masks(&s, b);
    let mut d = (*s.develop_of(a).unwrap()).clone();
    if let MaskShape::Subject { seg } = &mut d.masks[0].components[0].shape {
        *seg = Some(lightcraft_develop::SegMask::from_logits(4, &[5.0; 16]));
    }
    s.set_develop(a, d, "Update AI Mask").unwrap();
    assert_eq!(masks(&s, b), before_b, "a's own recompute stays on a");

    // a hand-drawn mask syncs as it is
    s.execute("mask.add", &json!({"kind": "radial", "center": [0.3, 0.6], "rx": 0.1, "ry": 0.1})).unwrap();
    let radial_a = masks(&s, a).last().unwrap().components.clone();
    assert_eq!(masks(&s, b).last().unwrap().components, radial_a);

    // deleting the Subject mask on a deletes each photo's copy, nothing else; one undo restores all
    let (nb, nc) = (masks(&s, b).len(), masks(&s, c).len());
    s.execute("mask.delete", &json!({"id": subject})).unwrap();
    let got = masks(&s, b);
    assert_eq!(got.len(), nb - 1);
    assert_eq!(got[0], b_own[0]);
    assert!(!got.iter().any(|m| matches!(m.components[0].shape, MaskShape::Subject { .. })));
    assert_eq!(masks(&s, c).len(), nc - 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(masks(&s, b).len(), nb);
    assert_eq!(masks(&s, c).len(), nc);
}

/// Note 1 for Copy / Paste and Sync Settings: an Object mask arrives without the clicks made on
/// the other photo (the object is picked again on each photo), and no AI bitmap travels.
#[test]
fn pasted_object_masks_leave_their_clicks_behind() {
    use lightcraft_develop::MaskShape;
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(2).map(|p| p.0).collect();
    let (a, b) = (lightcraft_catalog::PhotoId(ids[0]), lightcraft_catalog::PhotoId(ids[1]));
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "object", "points": [[0.4, 0.5]], "exclude": [[0.1, 0.1]], "seg": seg(2.0)})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    let got = masks(&s, b);
    assert_eq!(got.len(), 1);
    let MaskShape::Object { hint, exclude, seg, .. } = &got[0].components[0].shape else { panic!("an Object: {got:?}") };
    assert!(hint.is_empty() && exclude.is_empty() && seg.is_none(), "no clicks or pixels from a");
}

/// Note 2: Recalculate All AI Masks (Photo menu, `masks.recalculate`) drops whatever AI selection
/// the selected photos hold (a copied, stale or approximate one) back to the recipe, to be computed
/// from each photo's own pixels; hand-drawn masks are left as they are. Recalculate AI Mask on an
/// Object without clicks on this photo says to pick it first.
#[test]
fn recalculate_all_ai_masks_replaces_every_ai_selection_on_the_selected_photos() {
    use lightcraft_develop::MaskShape;
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(2).map(|p| p.0).collect();
    let (a, b) = (lightcraft_catalog::PhotoId(ids[0]), lightcraft_catalog::PhotoId(ids[1]));
    for id in [a, b] {
        s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
        s.execute("mask.add", &json!({"kind": "subject", "seg": seg(2.0)})).unwrap();
        s.execute("mask.add", &json!({"kind": "radial", "center": [0.3, 0.6], "rx": 0.1, "ry": 0.1})).unwrap();
    }
    let radial = masks(&s, b)[1].clone();
    assert!(masks(&s, b)[0].components[0].shape.has_seg());
    s.execute("library.select", &json!({"ids": [a.0, b.0], "active": a.0})).unwrap();
    let r = s.execute("masks.recalculate", &json!({})).unwrap();
    assert!(r.get("queued").is_some(), "{r}");
    for id in [a, b] {
        let got = masks(&s, id);
        assert_eq!(got[0].components[0].shape, MaskShape::Subject { seg: None }, "back to the recipe on {id:?}");
        assert!(got[0].needs_compute());
        assert_eq!(got[1].components, radial.components, "hand-drawn masks stay");
    }
    // an Object pasted from another photo has no clicks here: pick it first
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "object", "points": [[0.4, 0.5]], "seg": seg(2.0)})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    let object = masks(&s, b).iter().find(|m| matches!(m.components[0].shape, MaskShape::Object { .. })).map(|m| m.id).unwrap();
    let e = s.execute("mask.recompute", &json!({"id": object})).unwrap_err().to_string();
    assert!(e.contains("pick the object"), "{e}");
    // the command needs a selection and caps its size
    assert!(s.execute("masks.recalculate", &json!({"ids": (0..50_001).collect::<Vec<u64>>()})).is_err());
}

/// Note 22's groundwork: one edit on several photos (Auto Sync, a multi-photo drag, Sync
/// Settings) gives each photo's History step the same batch id, a new one per edit; one photo's
/// own edit has none; steps saved before the id existed read as having none, and a step without
/// one is saved as before.
#[test]
fn an_edit_on_several_photos_shares_one_history_batch_id() {
    let (mut s, [a, b, c]) = three();
    let last = |s: &Session, id| s.catalog.photo(id).unwrap().history.last().cloned().unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.3})).unwrap();
    let first = last(&s, a).batch;
    assert!(first.is_some());
    assert!([b, c].iter().all(|id| last(&s, *id).batch == first), "the same id on every photo's step");
    s.execute("develop.beginInteraction", &json!({"label": "Contrast", "ids": [b.0, c.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.contrast", "value": 10.0})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    let second = last(&s, a).batch;
    assert!(second.is_some() && second != first, "a new id per edit");
    assert!([b, c].iter().all(|id| last(&s, *id).batch == second));
    let listed = s.execute("history.list", &json!({"id": b.0})).unwrap();
    assert_eq!(listed["batches"].as_array().and_then(|v| v.last()).and_then(|v| v.as_u64()), second);
    // one photo's own edit
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.1})).unwrap();
    assert_eq!(last(&s, a).batch, None);
    // older catalogs: a step without the field reads, and one without an id writes no field
    let old: lightcraft_catalog::HistoryStep = serde_json::from_value(json!({"label": "Exposure", "settings": {}})).unwrap();
    assert_eq!(old.batch, None);
    assert!(serde_json::to_value(&old).unwrap().get("batch").is_none());
}

/// A crop of `frac` of the largest of shape `shape` at `angle` on a `w × h` frame, centred at the
/// normalized point `c` (moved inside), with `aspect` locked or free.
fn crop_on(
    w: f64,
    h: f64,
    shape: f64,
    frac: f64,
    c: (f64, f64),
    angle: f64,
    aspect: Option<(u32, u32)>,
    flips: (bool, bool),
) -> lightcraft_develop::Crop {
    let f = lightcraft_geom::crop_fit_angle(w, h, angle, Some(shape)).rect_px(w, h);
    let r = lightcraft_geom::Rect::from_center(lightcraft_geom::Point::new(c.0 * w, c.1 * h), f.width() * frac, f.height() * frac);
    let g = lightcraft_geom::CropGeometry { rect: r.scale(1.0 / w, 1.0 / h), angle }.constrained(w, h);
    lightcraft_develop::Crop { geometry: g, aspect, flip_h: flips.0, flip_v: flips.1 }
}

/// Test notes 2026-10-25 note 15: a crop synced from a vertical to a landscape (and back) keeps its
/// tightness and position: 80 % of the largest crop of its shape on the vertical is 80 % of the
/// largest of the turned shape on the landscape, its centre at the same normalized point, moved in
/// only as far as the frame needs. It used to be the landscape's largest crop, i.e. no change.
#[test]
fn a_synced_crop_keeps_its_tightness_and_position_across_orientations() {
    let (v, l) = ((4000.0, 6000.0), (6000.0, 4000.0));
    for aspect in [Some((2, 3)), None] {
        let src = crop_on(v.0, v.1, 2.0 / 3.0, 0.8, (0.5, 1.0 / 3.0), 0.0, aspect, (false, false));
        let out = crate::sync_same::synced_crop(&src, v, l);
        let r = out.geometry.rect_px(l.0, l.1);
        let largest = lightcraft_geom::crop_fit_angle(l.0, l.1, 0.0, Some(1.5)).rect_px(l.0, l.1);
        assert!((r.width() / r.height() - 1.5).abs() < 1e-6, "{aspect:?}: turned to 3:2, got {}", r.width() / r.height());
        assert!((r.width() / largest.width() - 0.8).abs() < 1e-6, "{aspect:?}: 80 % of the largest, got {}", r.width() / largest.width());
        assert!((r.center().x / l.0 - 0.5).abs() < 1e-6, "{aspect:?}: centred across");
        // the vertical's crop sits near its top: so does the landscape's, at the same normalized
        // centre, which still fits
        let sc = src.geometry.rect.center();
        assert!((r.center().y / l.1 - sc.y).abs() < 1e-6, "{aspect:?}: the same height in the frame, {} vs {}", r.center().y / l.1, sc.y);
        assert!(r.y0 >= -1e-6 && r.y0 < 0.01 * l.1, "{aspect:?}: near the top, y0 {}", r.y0);
        assert!(out.geometry.is_within_image(l.0, l.1));
        if aspect.is_some() {
            assert_eq!(out.aspect, Some((3, 2)));
        }
    }
    // a centre that would put the crop off the frame moves in only as far as it must
    let src = crop_on(v.0, v.1, 2.0 / 3.0, 0.9, (0.5, 0.45), 0.0, Some((2, 3)), (false, false));
    let mut low = src;
    let (w, h) = (low.geometry.rect.width(), low.geometry.rect.height());
    low.geometry.rect = lightcraft_geom::Rect::new(0.5 - w / 2.0, 1.0 - h, 0.5 + w / 2.0, 1.0);
    let out = crate::sync_same::synced_crop(&low, v, l);
    let r = out.geometry.rect_px(l.0, l.1);
    assert!((r.y1 - l.1).abs() < 1e-3, "down against the bottom edge: {r:?}");
    assert!(out.geometry.is_within_image(l.0, l.1));
    // a crop with room to keep its place keeps it exactly, normalized
    let src = crop_on(l.0, l.1, 1.5, 0.5, (0.3, 0.6), 0.0, Some((3, 2)), (false, false));
    let out = crate::sync_same::synced_crop(&src, l, v);
    let r = out.geometry.rect_px(v.0, v.1);
    assert!((r.center().x / v.0 - 0.3).abs() < 1e-6 && (r.center().y / v.1 - 0.6).abs() < 1e-6, "{:?}", r.center());
    assert_eq!(out.aspect, Some((2, 3)));
    // same shape: the very rectangle
    let same = crate::sync_same::synced_crop(&src, l, (3000.0, 2000.0));
    assert_eq!(same, src);
}

/// Test notes 2026-10-25 note 15, the user's warning (another raw developer gives a horizontal
/// photo a vertical crop when syncing from a vertical): whatever the crop, a landscape never gets a
/// portrait-shaped crop and a vertical never a landscape one, with the ratio locked or free, at a
/// straighten angle, with flips, from either orientation; and the crop always stays inside.
#[test]
fn a_synced_crop_never_gets_the_other_orientation() {
    let frames = [(4000.0, 6000.0), (6000.0, 4000.0), (3000.0, 4000.0), (5472.0, 3648.0)];
    for a in frames {
        for o in frames {
            for (shape, aspect) in [(2.0 / 3.0, Some((2, 3))), (1.5, Some((3, 2))), (0.8, None), (1.7, None), (1.0, Some((1, 1)))] {
                for angle in [0.0, 3.0, -7.5, 20.0] {
                    for flips in [(false, false), (true, false), (false, true), (true, true)] {
                        for (frac, c) in [(1.0, (0.5, 0.5)), (0.7, (0.2, 0.8)), (0.4, (0.9, 0.1))] {
                            let src = crop_on(a.0, a.1, shape, frac, c, angle, aspect, flips);
                            let out = crate::sync_same::synced_crop(&src, a, o);
                            let r = out.geometry.rect_px(o.0, o.1);
                            let s = r.width() / r.height();
                            let what = format!("{a:?} → {o:?}, shape {shape} {aspect:?}, {angle}°, {flips:?}, {frac} at {c:?}: {s}");
                            assert!(out.geometry.is_within_image(o.0, o.1), "inside: {what}");
                            assert_eq!((out.flip_h, out.flip_v), flips, "{what}");
                            assert_eq!(out.geometry.angle, angle, "{what}");
                            if (a.0 / a.1 - o.0 / o.1).abs() <= 0.005 * a.0 / a.1 {
                                continue;
                            }
                            let wide = o.0 > o.1;
                            if (shape - 1.0f64).abs() > 1e-3 {
                                assert_eq!(s > 1.0, wide, "the photo's own orientation: {what}");
                            }
                            if let Some((x, y)) = out.aspect.filter(|(x, y)| x != y) {
                                assert_eq!(x > y, wide, "the locked ratio turned too: {what}");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Test notes 2026-10-25 note 15 in the app: Auto Sync of a tighter crop from one photo to a photo
/// turned the other way (rotated) with Upright on both changes the other photo's crop to the same
/// tightness, in its own orientation; it used to give it its whole frame (no visible change).
#[test]
fn auto_sync_of_a_tight_crop_reaches_a_photo_turned_the_other_way_with_upright() {
    let (mut s, [a, b, _]) = three();
    raw_as_shot(&mut s, a, 5200.0, 0.0, false, 1.0);
    raw_as_shot(&mut s, b, 5200.0, 0.0, false, 1.0);
    let dims = |s: &Session, id| crate::cmd::develop::crop_dims(s, id);
    if (dims(&s, b).0 > dims(&s, b).1) == (dims(&s, a).0 > dims(&s, a).1) {
        s.execute("photo.rotateRight", &json!({"ids": [b.0]})).unwrap();
    }
    s.execute("library.select", &json!({"ids": [a.0, b.0], "active": a.0})).unwrap();
    s.execute("geometry.upright", &json!({"mode": "level"})).unwrap();
    let (aw, ah) = dims(&s, a);
    let shape = aw / ah;
    let lock = if aw > ah { (3, 2) } else { (2, 3) };
    let mut d = (*s.develop_of(a).unwrap()).clone();
    d.crop = crop_on(aw, ah, f64::from(lock.0) / f64::from(lock.1), 0.6, (0.5, 0.5), 2.0, Some(lock), (false, false));
    let _ = shape;
    s.set_develop(a, d, "Crop").unwrap();
    let (bw, bh) = dims(&s, b);
    let cb = s.develop_of(b).unwrap().crop;
    let r = cb.geometry.rect_px(bw, bh);
    assert_eq!(r.width() > r.height(), bw > bh, "b's own orientation: {r:?} on {bw}×{bh}");
    let largest = lightcraft_geom::crop_fit_angle(bw, bh, 2.0, Some(r.width() / r.height())).rect_px(bw, bh);
    assert!(r.width() / largest.width() < 0.7, "tighter than b's whole frame: {}", r.width() / largest.width());
}
