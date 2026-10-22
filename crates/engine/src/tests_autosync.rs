//! Auto Sync and live multi-photo edits: each photo moves from its own value.

use serde_json::json;

use crate::Session;

fn demo() -> Session {
    Session::with_demo()
}

/// Auto Sync from a slider moves every selected photo by as much as the active one, from its own
/// value, live during the drag and as one undo step; never the active photo's numbers copied over
/// (which turned photos with another as-shot white balance blue).
#[test]
fn auto_sync_moves_each_photo_from_its_own_value() {
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(2).map(|p| p.0).collect();
    let (a, b) = (lightcraft_catalog::PhotoId(ids[0]), lightcraft_catalog::PhotoId(ids[1]));
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
    s.execute("develop.set", &json!({"control": "wb.temp", "value": 7000.0})).unwrap();
    s.execute("library.select", &json!({"ids": [a.0, b.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    let exp = |s: &Session, id| s.develop_of(id).unwrap().light.exposure;
    let temp = |s: &Session, id| {
        let info = s.source_info(id);
        lightcraft_pipeline::local::effective_wb(&info, &s.develop_of(id).unwrap()).0
    };
    let (a_temp, b_temp) = (temp(&s, a), temp(&s, b));
    // a drag: live, each from its own value
    s.execute("develop.beginInteraction", &json!({"label": "Exposure", "ids": [b.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.25})).unwrap();
    assert!((exp(&s, b) - 1.25).abs() < 1e-9, "b moves during the drag: {}", exp(&s, b));
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.5})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    assert!((exp(&s, a) - 0.5).abs() < 1e-9 && (exp(&s, b) - 1.5).abs() < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(exp(&s, a).abs() < 1e-9 && (exp(&s, b) - 1.0).abs() < 1e-9, "one undo step for both");
    // a typed value outside a drag: the same, on each photo's own white-balance scale
    s.execute("develop.set", &json!({"control": "wb.temp", "value": a_temp + 300.0})).unwrap();
    assert!((temp(&s, a) - (a_temp + 300.0)).abs() < 1.0);
    assert!((temp(&s, b) - (b_temp + 300.0)).abs() < 1.0, "b keeps its own white balance: {} vs {}", temp(&s, b), b_temp + 300.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((temp(&s, b) - b_temp).abs() < 1.0);
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
