//! Smart presets: presets behave as in Lightroom Classic (settings set, masks added, never
//! removed), the same preset applied again right away replaces its own masks, another preset
//! in between makes them the photographer's, AI masks travel as recipes and are computed per
//! photo, and undo steps back through it all.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::recipe::Who;
use lightcraft_develop::{Mask, MaskComponent, MaskOp, MaskShape, Preset, SegMask};
use serde_json::{Value, json};

use crate::Session;

fn comp(shape: MaskShape) -> MaskComponent {
    MaskComponent { name: None, anchor: None, op: MaskOp::Add, invert: false, shape }
}

fn seg() -> SegMask {
    SegMask::from_logits(8, &[4.0; 64])
}

/// A mask-only preset ("Bring sky down") and a combined one (a look plus a People mask).
fn presets(s: &mut Session) -> (String, String) {
    let sky = Mask { name: "Sky".into(), components: vec![comp(MaskShape::Sky { seg: Some(seg()) })], ..Default::default() };
    let mut sky_adj = sky.clone();
    sky_adj.adjust.exposure = -0.6;
    let p = Preset {
        id: "user.test.sky".into(),
        name: "Bring sky down".into(),
        group: "Smart".into(),
        settings: json!({"masks": [serde_json::to_value(&sky_adj).unwrap()]}),
        favorite: false,
        builtin: false,
    };
    let suit = Mask {
        name: "Suit".into(),
        components: vec![comp(MaskShape::People { person: 0, parts: vec!["Clothes".into()], who: Some(Who::Role("Groom".into())), seg: None })],
        ..Default::default()
    };
    let q = Preset {
        id: "user.test.look".into(),
        name: "Studio look".into(),
        group: "Smart".into(),
        settings: json!({"light": {"contrast": 25.0}, "masks": [serde_json::to_value(&suit).unwrap()]}),
        favorite: false,
        builtin: false,
    };
    s.presets.push(p);
    s.presets.push(q);
    ("user.test.sky".into(), "user.test.look".into())
}

fn masks(s: &Session) -> Vec<Mask> {
    s.develop_of(s.active().unwrap()).unwrap().masks.clone()
}

fn apply(s: &mut Session, id: &str) -> Value {
    s.execute("preset.apply", &json!({"id": id})).unwrap()
}

#[test]
fn the_same_preset_twice_replaces_its_masks() {
    let mut s = Session::with_demo();
    let (sky, _) = presets(&mut s);
    apply(&mut s, &sky);
    let once = masks(&s);
    assert_eq!(once.len(), 1);
    assert_eq!(once[0].preset.as_deref(), Some(sky.as_str()), "provisional: tagged with its preset");
    assert!(!once[0].components[0].shape.has_seg(), "a recipe: the preset's photo's sky isn't copied");
    apply(&mut s, &sky);
    let twice = masks(&s);
    assert_eq!(twice.len(), 1, "no doubling");
    assert_eq!(twice[0].adjust.exposure, -0.6);
}

#[test]
fn another_preset_in_between_makes_the_masks_the_photographers() {
    let mut s = Session::with_demo();
    let (sky, look) = presets(&mut s);
    apply(&mut s, &sky);
    apply(&mut s, &look);
    let m = masks(&s);
    assert_eq!(m.len(), 2, "masks are added, never replaced");
    assert_eq!(m[0].preset, None, "the sky mask is the photographer's now");
    assert_eq!(m[1].preset.as_deref(), Some(look.as_str()));
    assert_eq!(s.develop_of(s.active().unwrap()).unwrap().light.contrast, 25.0, "the look's settings are set");
    apply(&mut s, &sky);
    let m = masks(&s);
    assert_eq!(m.iter().filter(|m| m.name == "Bring sky down").count(), 2, "P, Q, P: P's masks are added again (named after it)");
    assert_eq!(m.iter().filter(|m| m.preset.is_some()).count(), 1, "only the latest application is provisional");
    let ids: std::collections::HashSet<u32> = m.iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), m.len(), "ids stay unique");
}

#[test]
fn editing_a_provisional_mask_makes_it_permanent() {
    let mut s = Session::with_demo();
    let (sky, _) = presets(&mut s);
    apply(&mut s, &sky);
    let id = masks(&s)[0].id;
    s.execute("mask.adjust", &json!({"id": id, "values": {"exposure": -1.0}})).unwrap();
    assert_eq!(masks(&s)[0].preset, None);
    apply(&mut s, &sky);
    assert_eq!(masks(&s).len(), 2, "the edited mask is kept; the preset adds a fresh one");
}

#[test]
fn undo_steps_back_one_application_at_a_time() {
    let mut s = Session::with_demo();
    let (sky, look) = presets(&mut s);
    let original = (*s.develop_of(s.active().unwrap()).unwrap()).clone();
    apply(&mut s, &sky);
    let after_p = (*s.develop_of(s.active().unwrap()).unwrap()).clone();
    apply(&mut s, &look);
    let after_q = (*s.develop_of(s.active().unwrap()).unwrap()).clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.develop_of(s.active().unwrap()).unwrap(), after_p, "undo: back to after P (P's mask provisional again)");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.develop_of(s.active().unwrap()).unwrap(), original);
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.develop_of(s.active().unwrap()).unwrap(), after_q);
}

#[test]
fn copy_and_paste_carry_recipes_not_another_photos_selection() {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).take(2).collect();
    s.execute("library.select", &json!({"ids": [ids[0]], "active": ids[0]})).unwrap();
    let mut d = (*s.develop_of(PhotoId(ids[0])).unwrap()).clone();
    d.masks.push(Mask { id: 1, name: "Sky".into(), components: vec![comp(MaskShape::Sky { seg: Some(seg()) })], ..Default::default() });
    s.set_develop(PhotoId(ids[0]), d, "Sky").unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    let r = s.execute("develop.paste", &json!({"ids": [ids[1]]})).unwrap();
    let pasted = s.develop_of(PhotoId(ids[1])).unwrap();
    assert_eq!(pasted.masks.len(), 1);
    assert!(!pasted.masks[0].components[0].shape.has_seg(), "computed on the target, not copied");
    assert!(s.develop_of(PhotoId(ids[0])).unwrap().masks[0].components[0].shape.has_seg(), "the source keeps its own");
    assert!(r.get("masks").is_some());
}

#[test]
fn presets_made_from_a_photo_keep_recipes() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    let mut d = (*s.develop_of(id).unwrap()).clone();
    d.masks.push(Mask { id: 1, name: "Sky".into(), components: vec![comp(MaskShape::Sky { seg: Some(seg()) })], ..Default::default() });
    s.set_develop(id, d, "Sky").unwrap();
    let r = s.execute("preset.create", &json!({"name": "Sky pull", "groups": ["masks"]})).unwrap();
    let pid = r["id"].as_str().unwrap();
    let p = s.presets.iter().find(|p| p.id == pid).unwrap();
    assert!(p.settings.pointer("/masks/0/components/0/shape/seg").is_none(), "{}", p.settings);
    assert!(p.has_ai_masks() && p.mask_only());
}

#[test]
fn roles_resolve_per_folder_and_masks_target_them() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    // demo photos have no folder: bind the role to the photo itself
    s.execute("people.setRole", &json!({"role": "Groom", "person": "Jack", "scope": {"ids": [id.0]}})).unwrap();
    let r = s.execute("people.roles", &json!({})).unwrap();
    assert_eq!(r["here"][0], json!({"role": "Groom", "person": "Jack"}));
    let r = s.execute("mask.add", &json!({"kind": "people", "who": {"role": "Groom"}, "parts": ["Clothes"], "name": "Suit"})).unwrap();
    let m = masks(&s);
    assert!(matches!(&m[0].components[0].shape, MaskShape::People { who: Some(Who::Role(g)), .. } if g == "Groom"), "{r}");
    // without a face model: nothing computed, nothing crashes; the result says why
    assert!(s.execute("mask.add", &json!({"kind": "people", "who": {"nobody": 1}})).is_err());
    s.execute("people.clearRole", &json!({"role": "groom"})).unwrap();
    assert_eq!(s.execute("people.roles", &json!({})).unwrap()["roles"], json!([]));
}

#[test]
fn anchored_gradients_start_on_the_whole_photo_and_need_their_box() {
    let mut s = Session::with_demo();
    s.execute("mask.add", &json!({"kind": "linear", "start": {"x": 0.5, "y": 1.0}, "end": {"x": 0.5, "y": 0.6}, "anchor": "people"})).unwrap();
    let m = masks(&s);
    let c = &m[0].components[0];
    assert!(c.anchor.as_ref().is_some_and(|a| a.bounds.is_none()));
    assert!(m[0].needs_compute());
    assert!(s.execute("mask.add", &json!({"kind": "sky", "anchor": "people"})).is_err(), "only gradients anchor");
    let r = s.execute("masks.recompute", &json!({})).unwrap();
    assert_eq!(r["queued"], 0, "no model in tests: {r}");
}

#[test]
fn recipe_results_complete_the_step_and_undo_cancels_the_rest() {
    let mut s = Session::with_demo();
    let (sky, _) = presets(&mut s);
    apply(&mut s, &sky);
    let photo = s.active().unwrap();
    let mask = masks(&s)[0].id;
    let undo_len = s.undo.len();
    #[cfg(feature = "sam")]
    {
        use crate::segment::{AutoKind, Polled, Tag, worker::Outcome};
        use crate::smart_masks::Part;
        let tag = Tag::Recipe { photo, mask, comp: 0, part: Part::Seg(AutoKind::Sky), batch: 99 };
        let mut polled = Polled::default();
        assert!(s.apply_recipe_outcome(
            Outcome { tag, result: Ok(Some(seg())), superseded: false, faces: None, payload: Default::default(), note: None },
            &mut polled
        ));
        assert!(masks(&s)[0].components[0].shape.has_seg(), "computed");
        assert_eq!(s.undo.len(), undo_len, "no undo step of its own");
        assert_eq!(masks(&s)[0].preset.as_deref(), Some(sky.as_str()), "still provisional");
        // undo takes the preset and its computed masks back together
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(masks(&s).is_empty());
        // redo brings back the computed mask, no model needed
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(masks(&s)[0].components[0].shape.has_seg());
        // a late result for a mask that is gone changes nothing
        let tag = Tag::Recipe { photo, mask: 4242, comp: 0, part: Part::Seg(AutoKind::Sky), batch: 99 };
        assert!(!s.apply_recipe_outcome(
            Outcome { tag, result: Ok(Some(seg())), superseded: false, faces: None, payload: Default::default(), note: None },
            &mut polled
        ));
    }
    #[cfg(not(feature = "sam"))]
    let _ = (photo, mask, undo_len);
}

#[test]
fn forgetting_deletes_the_face_data() {
    let dir = std::env::temp_dir().join(format!("lc-smart-forget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    s.open_library(&dir, true).unwrap();
    let id = s.active().or_else(|| s.catalog.photos().next().map(|p| p.id)).unwrap();
    s.execute("people.setRole", &json!({"role": "Bride", "person": "Alex", "scope": {"ids": [id.0]}})).unwrap();
    assert!(dir.join("people.json").exists());
    std::fs::write(dir.join("face-embeddings-test.bin"), b"LCFE").unwrap();
    std::fs::write(dir.join("face-scanned.bin"), b"LCFS").unwrap();
    let r = s.execute("people.forget", &json!({"person": "alex"})).unwrap();
    assert_eq!(r["forgotten"], true);
    let r = s.execute("people.forgetAll", &json!({})).unwrap();
    assert!(r["deleted"].as_array().unwrap().iter().any(|f| f == "face-embeddings-test.bin"), "{r}");
    assert!(!dir.join("face-embeddings-test.bin").exists() && !dir.join("face-scanned.bin").exists());
    // a damaged store is reported, not a crash, and isn't overwritten
    s.close_library().unwrap();
    drop(s);
    std::fs::write(dir.join("people.json"), b"{broken").unwrap();
    let mut t = Session::new();
    t.open_library(&dir, false).unwrap();
    assert!(t.execute("people.setRole", &json!({"role": "Bride", "person": "X", "scope": {"ids": [1]}})).is_err());
    assert_eq!(std::fs::read(dir.join("people.json")).unwrap(), b"{broken");
    drop(t);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_parameters_are_errors() {
    let mut s = Session::with_demo();
    for (c, p) in [
        ("people.setRole", json!({"role": "", "person": "x"})),
        ("people.setRole", json!({"role": "Bride", "person": "x", "scope": 7})),
        ("people.rename", json!({"from": "a"})),
        ("concepts.teach", json!({"name": "Dress", "mask": 999})),
        ("mask.add", json!({"kind": "concept"})),
        ("preset.apply", json!({"id": "nope"})),
    ] {
        assert!(s.execute(c, &p).is_err(), "{c} {p}");
    }
    assert!(s.execute("concepts.list", &json!({})).unwrap()["concepts"].as_array().unwrap().is_empty());
}

#[test]
fn smart_presets_export_as_lightroom_xmp_and_import_back_unchanged() {
    let mut s = Session::with_demo();
    let dir = std::env::temp_dir().join(format!("lc-smart-xmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let r = s.execute("preset.exportXmp", &json!({"id": "lc.smart.sunKissed", "folder": dir.to_string_lossy()})).unwrap();
    let path = r["written"][0]["path"].as_str().unwrap().to_string();
    assert!(r["written"][0]["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("follows the people")), "{r}");
    let bytes = std::fs::read(&path).unwrap();
    let back = crate::preset_import::read_presets(&path, &bytes, None).unwrap().remove(0);
    let original = s.presets.iter().find(|p| p.id == "lc.smart.sunKissed").unwrap().settings.clone();
    assert_eq!(back.preset.settings, original, "the LightCraft recipe survives the round trip");
    assert_eq!(back.preset.group, "Smart Masks");
    assert!(back.unmapped.is_empty());
    let parsed = crate::presets::parse_preset_file(&path, &bytes).unwrap();
    assert_eq!(parsed[0].settings, original);
    // the same preset imported again is recognised as already there
    let r = s.execute("preset.import", &json!({"paths": [path]})).unwrap();
    assert_eq!(r["imported"].as_array().map_or(0, Vec::len) + r["skipped"].as_u64().unwrap_or(0) as usize, 1, "{r}");
    assert!(s.execute("preset.exportXmp", &json!({"folder": dir.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
