//! Linked masks (test notes 2026-10-28 #12): the copies of a mask applied to several photos in one
//! go (Auto Sync, a preset on a selection, Paste, Sync Settings) stay one group, and an edit of
//! one of them goes to the others, whatever is selected and whether Auto Sync is on.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{Mask, MaskShape};
use lightcraft_geom::Point;
use serde_json::json;

use crate::Session;

fn masks(s: &Session, id: PhotoId) -> Vec<Mask> {
    s.develop_of(id).unwrap().masks.clone()
}

fn three() -> (Session, [PhotoId; 3]) {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.visible_cloned().iter().take(3).map(|p| p.0).collect();
    (s, [0, 1, 2].map(|i| PhotoId(ids[i])))
}

/// The gradient drag the photo does: an interaction, `mask.update`s, its end.
fn drag(s: &mut Session, mask: u32, comp: usize, start: [f64; 2], end: [f64; 2]) {
    s.execute("develop.beginInteraction", &json!({"label": "Edit Mask"})).unwrap();
    for t in [0.5, 1.0] {
        let shape = MaskShape::Linear { start: Point::new(start[0] * t, start[1] * t), end: Point::new(end[0] * t, end[1] * t) };
        s.execute("mask.update", &json!({"id": mask, "component": comp, "shape": shape})).unwrap();
    }
    s.execute("develop.endInteraction", &json!({})).unwrap();
}

/// The failing case of note 12: a smart mask (an AI selection minus a Linear gradient anchored to
/// the people box) applied with Auto Sync on to a selection. Dragging the gradient on one photo
/// puts it in exactly the same place (start, end) on every linked photo, not relative to each
/// photo's own people box, and from then on the gradient is placed by hand, not through the box.
/// The copies keep their own AI selection. It works with Auto Sync off and only one photo selected
/// too: the link is stored with the masks.
#[test]
fn a_gradient_dragged_on_one_linked_photo_goes_to_the_same_place_on_all() {
    let (mut s, [a, b, c]) = three();
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    s.execute("mask.add", &json!({"kind": "background", "seg": lightcraft_develop::SegMask::from_logits(4, &[2.0; 16]), "name": "Sun Kissed"}))
        .unwrap();
    let mid = masks(&s, a)[0].id;
    s.execute("mask.select", &json!({"id": mid})).unwrap();
    s.execute("mask.addComponent", &json!({"kind": "linear", "op": "subtract", "start": [0.5, 1.0], "end": [0.5, 0.55], "anchor": "people"}))
        .unwrap();
    let link = masks(&s, a)[0].link.clone();
    assert!(!link.is_empty(), "a mask Auto Sync carries is linked");
    for id in [b, c] {
        let m = masks(&s, id);
        assert_eq!(m.len(), 1, "one copy each: {m:#?}");
        assert_eq!(m[0].link, link, "the copies share the link");
        assert_eq!(m[0].components.len(), 2);
    }
    // each photo found its own people box: its anchored gradient sits differently
    for (id, bx) in [(a, [0.1, 0.1, 0.5, 0.9]), (b, [0.4, 0.2, 0.9, 0.8]), (c, [0.0, 0.5, 0.3, 1.0])] {
        let mut d = (*s.develop_of(id).unwrap()).clone();
        let comp = &mut d.masks[0].components[1];
        let anchor = comp.anchor.as_mut().expect("anchored");
        anchor.bounds = Some(bx);
        comp.shape = anchor.placed(1.5).unwrap();
        s.amend_develop(id, d);
    }

    // with Auto Sync on (the user's case): the drag reaches every selected photo exactly
    drag(&mut s, mid, 1, [0.552, 0.549], [0.261, 0.259]);
    let want = (Point::new(0.552, 0.549), Point::new(0.261, 0.259));
    for id in [a, b, c] {
        let m = &masks(&s, id)[0];
        let comp = &m.components[1];
        assert_eq!(
            match &comp.shape {
                MaskShape::Linear { start, end } => (*start, *end),
                o => panic!("{o:?}"),
            },
            want,
            "the master's exact place on every photo"
        );
        assert!(comp.anchor.is_none(), "placed by hand: no longer through the people box");
        assert!(m.components[0].shape.is_ai(), "the AI part stays");
    }
    assert!(masks(&s, a)[0].components[0].shape.has_seg(), "a keeps its own selection");

    // Auto Sync off, only b selected: an edit of b's copy still reaches a and c
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    let bid = masks(&s, b)[0].id;
    s.execute("mask.select", &json!({"id": bid})).unwrap();
    drag(&mut s, bid, 1, [0.3, 0.9], [0.7, 0.2]);
    s.execute("mask.adjust", &json!({"id": bid, "values": {"exposure": 0.8}})).unwrap();
    for id in [a, b, c] {
        let m = &masks(&s, id)[0];
        let MaskShape::Linear { start, end } = m.components[1].shape else { panic!() };
        assert_eq!((start, end), (Point::new(0.3, 0.9), Point::new(0.7, 0.2)), "linked without Auto Sync");
        assert!((m.adjust.exposure - 0.8).abs() < 1e-9, "sliders too: {:?}", m.adjust);
    }
    // one undo step takes the edit back on all of them
    s.execute("edit.undo", &json!({})).unwrap();
    for id in [a, b, c] {
        assert!(masks(&s, id)[0].adjust.exposure.abs() < 1e-9, "undone on every photo");
    }
}

/// How a link breaks: Unlink on one photo takes its copy out of the group (edits stop travelling
/// both ways); deleting a copy without Auto Sync deletes only that one, and the rest stay linked;
/// a mask of a photo that was never in the group is never touched.
#[test]
fn unlink_and_delete_break_the_link_for_one_copy_only() {
    let (mut s, [a, b, c]) = three();
    let d = s.visible_cloned().get(3).copied().expect("a fourth demo photo");
    // d has a mask of its own with the same id and name
    s.execute("library.select", &json!({"ids": [d.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial", "name": "Mask 1"})).unwrap();
    let d_own = masks(&s, d);
    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial", "name": "Mask 1"})).unwrap();
    s.execute("develop.autoSync", &json!({"on": false})).unwrap();
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    let mid = masks(&s, a)[0].id;
    s.execute("mask.adjust", &json!({"id": mid, "values": {"exposure": 0.5}})).unwrap();
    for id in [b, c] {
        assert!((masks(&s, id)[0].adjust.exposure - 0.5).abs() < 1e-9);
    }
    assert_eq!(masks(&s, d), d_own, "never in the group: untouched");

    // c leaves the group
    s.execute("library.select", &json!({"ids": [c.0]})).unwrap();
    let r = s.execute("mask.unlink", &json!({"id": masks(&s, c)[0].id})).unwrap();
    assert_eq!(r["unlinked"], json!(true));
    assert!(masks(&s, c)[0].link.is_empty());
    s.execute("mask.adjust", &json!({"id": masks(&s, c)[0].id, "values": {"contrast": 30}})).unwrap();
    assert_eq!(masks(&s, a)[0].adjust.contrast, 0.0, "an unlinked copy's edit stays on its photo");
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("mask.adjust", &json!({"id": mid, "values": {"exposure": 1.0}})).unwrap();
    assert!((masks(&s, b)[0].adjust.exposure - 1.0).abs() < 1e-9, "b is still linked");
    assert!((masks(&s, c)[0].adjust.exposure - 0.5).abs() < 1e-9, "c no longer follows");

    // deleting a's copy deletes only a's
    s.execute("mask.delete", &json!({"id": mid})).unwrap();
    assert!(masks(&s, a).is_empty());
    assert_eq!(masks(&s, b).len(), 1, "b keeps its copy");
    assert_eq!(masks(&s, c).len(), 1);
}

/// A preset on a selection, Paste onto a selection, Sync Settings and Paste from Previous link
/// the masks they copy (with the source photo's original for Paste and Sync); a preset on one
/// photo links nothing.
#[test]
fn presets_paste_and_sync_link_the_masks_they_copy() {
    let (mut s, [a, b, c]) = three();
    let settings = json!({"masks": [{"id": 1, "name": "Glow", "components": [{"shape": {"kind": "radial", "center": {"x": 0.5, "y": 0.5}, "rx": 0.2, "ry": 0.2, "angle": 0.0, "feather": 50.0, "invert": false}}], "adjust": {"exposure": 0.3}}]});
    s.presets.push(lightcraft_develop::Preset {
        id: "glow".into(),
        name: "Glow".into(),
        group: "Test".into(),
        settings,
        favorite: false,
        builtin: false,
    });
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("preset.apply", &json!({"id": "glow"})).unwrap();
    assert!(masks(&s, a)[0].link.is_empty(), "one photo: nothing to link");
    s.execute("mask.deleteAll", &json!({})).unwrap();

    s.execute("library.select", &json!({"ids": [a.0, b.0, c.0], "active": a.0})).unwrap();
    s.execute("preset.apply", &json!({"id": "glow"})).unwrap();
    let link = masks(&s, a)[0].link.clone();
    assert!(!link.is_empty());
    assert!([b, c].iter().all(|id| masks(&s, *id)[0].link == link), "a preset on a selection links its masks");
    for id in [a, b, c] {
        s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
        s.execute("mask.deleteAll", &json!({})).unwrap();
    }

    // Paste: copied from a, pasted onto b and c
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial"})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("library.select", &json!({"ids": [b.0, c.0], "active": b.0})).unwrap();
    s.execute("develop.paste", &json!({})).unwrap();
    let link = masks(&s, a)[0].link.clone();
    assert!(!link.is_empty(), "the original joins the group");
    assert!([b, c].iter().all(|id| masks(&s, *id)[0].link == link));
    s.execute("library.select", &json!({"ids": [c.0]})).unwrap();
    s.execute("mask.adjust", &json!({"id": masks(&s, c)[0].id, "values": {"shadows": 40}})).unwrap();
    assert_eq!(masks(&s, a)[0].adjust.shadows, 40.0, "a pasted copy's edit reaches the original");

    // Sync Settings from b to c (b's mask already linked: c's copy joins that group)
    s.execute("library.select", &json!({"ids": [b.0, c.0], "active": b.0})).unwrap();
    s.execute("develop.sync", &json!({"groups": ["masks"]})).unwrap();
    assert_eq!(masks(&s, c)[0].link, link);
}

/// Masks saved before links existed read as unlinked, and a link survives a save.
#[test]
fn old_masks_read_unlinked() {
    let m: Mask = serde_json::from_value(json!({"id": 1, "name": "Mask 1", "components": []})).unwrap();
    assert!(m.link.is_empty());
    assert!(serde_json::to_value(&m).unwrap().get("link").is_none(), "not written while unlinked");
    let linked = Mask { link: "00ff".into(), ..m };
    let back: Mask = serde_json::from_value(serde_json::to_value(&linked).unwrap()).unwrap();
    assert_eq!(back.link, "00ff");
}

/// Test notes 2026-10-28 #26: a ⇧-click range or ⌘-click that keeps the active photo keeps the
/// selected mask; a click that moves the active photo selects the linked copy there (another id),
/// and on a photo without it none.
#[test]
fn the_selected_mask_follows_the_active_photo_to_its_linked_copy() {
    let (mut s, [a, b, c]) = three();
    s.execute("library.select", &json!({"ids": [b.0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial"})).unwrap();
    s.execute("library.select", &json!({"ids": [a.0, b.0], "active": a.0})).unwrap();
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    s.execute("mask.add", &json!({"kind": "linear"})).unwrap();
    let on_a = s.active_mask.expect("selected");
    let link = masks(&s, a)[0].link.clone();
    let on_b = masks(&s, b).iter().find(|m| m.link == link).map(|m| m.id).expect("b's copy");
    assert_ne!(on_a, on_b);
    s.execute("library.select", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(s.active_mask, Some(on_a), "the same active photo keeps its mask");
    s.execute("library.select", &json!({"ids": [c.0], "mode": "range"})).unwrap();
    assert_eq!(s.active_mask, Some(on_a), "a range keeps it");
    s.execute("library.select", &json!({"ids": [c.0], "mode": "toggle"})).unwrap();
    assert_eq!(s.active_mask, Some(on_a), "a ⌘-click keeps it");
    s.execute("library.select", &json!({"ids": [b.0], "mode": "click"})).unwrap();
    assert_eq!(s.active(), Some(b));
    assert_eq!(s.active_mask, Some(on_b), "the linked copy on the new active photo");
    s.execute("library.select", &json!({"ids": [c.0], "mode": "click"})).unwrap();
    assert_eq!(s.active_mask, None, "a photo without it: none");
}
