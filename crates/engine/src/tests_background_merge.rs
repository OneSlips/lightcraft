//! Background results merge into the photo's current settings, never replace them (test notes
//! 2026-10-24 note 18): edits made while an AI mask computes are kept on every photo, and the
//! result reaches the undo and History steps made meanwhile.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, Mask, MaskComponent, MaskOp, MaskShape, SegMask};
use serde_json::json;

use crate::Session;

fn sky_recipe() -> Mask {
    Mask {
        id: 7,
        name: "Sky".into(),
        components: vec![MaskComponent { name: None, anchor: None, op: MaskOp::Add, invert: false, shape: MaskShape::Sky { seg: None } }],
        ..Default::default()
    }
}

fn has_seg(s: &Session, id: PhotoId) -> bool {
    s.develop_of(id).is_some_and(|d| d.masks.iter().any(|m| m.id == 7 && m.components.iter().all(|c| c.shape.has_seg())))
}

fn exp(s: &Session, id: PhotoId) -> f64 {
    s.develop_of(id).map_or(f64::NAN, |d| d.light.exposure)
}

/// The latest History step holds what the photo has.
fn history_matches(s: &Session, id: PhotoId) -> bool {
    let p = s.catalog.photo(id).unwrap();
    p.history.last().is_some_and(|st| *st.settings == *p.develop)
}

/// Three demo photos selected with Auto Sync on, each with a Sky mask still to compute (as a
/// preset or a synced AI mask leaves them), as one undo step with a History step on each.
fn three_with_recipes() -> (Session, [PhotoId; 3]) {
    let mut s = Session::with_demo();
    let ids: Vec<PhotoId> = s.visible_cloned().into_iter().take(3).collect();
    let ops = ids
        .iter()
        .map(|id| {
            let d = DevelopSettings { masks: vec![sky_recipe()], ..(*s.develop_of(*id).unwrap()).clone() };
            s.develop_op(*id, d, "Preset: Sky").unwrap()
        })
        .collect();
    s.commit("Preset: Sky", lightcraft_catalog::Op::Batch { ops }).unwrap();
    s.execute("library.select", &json!({"ids": [ids[0].0, ids[1].0, ids[2].0], "active": ids[0].0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    (s, [ids[0], ids[1], ids[2]])
}

/// The worker's answer for photo `id`'s Sky mask, asked for the recipe `key`.
#[cfg(feature = "sam")]
fn deliver(s: &mut Session, id: PhotoId, key: Option<u64>) -> bool {
    use crate::segment::{AutoKind, Polled, Tag, worker::Outcome};
    use crate::smart_masks::Part;
    let tag = Tag::Recipe { photo: id, mask: 7, comp: 0, part: Part::Seg(AutoKind::Sky), batch: 4242, key };
    let mut polled = Polled::default();
    let seg = SegMask::from_logits(8, &[4.0; 64]);
    s.apply_recipe_outcome(
        Outcome { tag, result: Ok(Some(seg)), superseded: false, faces: None, payload: Default::default(), note: None, notice: None },
        &mut polled,
    )
}

#[cfg(feature = "sam")]
fn sky_key() -> u64 {
    crate::smart_masks::part_key(&sky_recipe().components[0], &crate::smart_masks::Part::Seg(crate::segment::AutoKind::Sky))
}

/// Note 18's regression: AI masks computing on several photos, an Exposure edit synced to all of
/// them before the results come in. Every photo keeps the edit and gets its selection; History
/// matches the settings; undoing the edit keeps the selection (it reached the undo step), redo
/// brings the edit back with it.
#[cfg(feature = "sam")]
#[test]
fn an_edit_synced_while_ai_masks_compute_is_kept_on_every_photo() {
    let (mut s, ids) = three_with_recipes();
    // the edit, before any result
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.8})).unwrap();
    assert!(ids.iter().all(|id| (exp(&s, *id) - 0.8).abs() < 1e-9));
    // a second edit, live, while results come in (a drag over the first two results)
    s.execute("develop.beginInteraction", &json!({"label": "Contrast", "ids": [ids[1].0, ids[2].0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.contrast", "value": 20.0})).unwrap();
    assert!(deliver(&mut s, ids[1], Some(sky_key())));
    assert!(deliver(&mut s, ids[0], Some(sky_key())));
    s.execute("develop.set", &json!({"control": "light.contrast", "value": 25.0})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    assert!(deliver(&mut s, ids[2], Some(sky_key())));
    for id in ids {
        let d = s.develop_of(id).unwrap();
        assert!((d.light.exposure - 0.8).abs() < 1e-9 && (d.light.contrast - 25.0).abs() < 1e-9, "photo {} kept its edits: {d:?}", id.0);
        assert!(has_seg(&s, id), "photo {} has its selection", id.0);
        assert!(history_matches(&s, id), "photo {}: History matches the settings", id.0);
    }
    // undo the contrast drag and the exposure edit: the selections stay
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    for id in ids {
        assert!(exp(&s, id).abs() < 1e-9, "the edit undone on photo {}", id.0);
        assert!(has_seg(&s, id), "photo {}: undo keeps the selection that came in later", id.0);
    }
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(ids.iter().all(|id| (exp(&s, *id) - 0.8).abs() < 1e-9 && has_seg(&s, *id)));
}

/// A drag cancelled after a result came in keeps the result; a result for a recipe that changed
/// meanwhile (another kind of mask on that component) goes nowhere.
#[cfg(feature = "sam")]
#[test]
fn a_cancelled_drag_keeps_a_result_and_a_changed_recipe_takes_none() {
    let (mut s, ids) = three_with_recipes();
    s.execute("develop.beginInteraction", &json!({"label": "Exposure", "ids": [ids[1].0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
    assert!(deliver(&mut s, ids[1], Some(sky_key())));
    s.execute("develop.cancelInteraction", &json!({})).unwrap();
    assert!(exp(&s, ids[1]).abs() < 1e-9 && has_seg(&s, ids[1]), "the drag cancelled, the selection kept");
    // photo 3's mask becomes a Background mask before the Sky result arrives
    let mut d = (*s.develop_of(ids[2]).unwrap()).clone();
    if let Some(c) = d.masks.get_mut(0).and_then(|m| m.components.get_mut(0)) {
        c.shape = MaskShape::Background { seg: None };
    }
    s.catalog
        .apply(lightcraft_catalog::Op::SetDevelop { id: ids[2], settings: std::sync::Arc::new(d.clone()), label: "x".into(), edited: None })
        .unwrap();
    assert!(!deliver(&mut s, ids[2], Some(sky_key())), "not into another recipe");
    assert_eq!(*s.develop_of(ids[2]).unwrap(), d);
}

/// Without the AI models the setup still holds: the edit lands on every photo and History
/// matches (the part every build runs).
#[test]
fn an_edit_synced_onto_photos_with_masks_to_compute_lands_everywhere() {
    let (mut s, ids) = three_with_recipes();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.8})).unwrap();
    for id in ids {
        assert!((exp(&s, id) - 0.8).abs() < 1e-9);
        assert!(history_matches(&s, id));
        assert!(s.develop_of(id).is_some_and(|d| d.masks.len() == 1), "the mask kept");
    }
}
