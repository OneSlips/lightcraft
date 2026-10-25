//! Test notes 2026-10-25, note 12: "everyone's clothes minus the bride's" on several photos. Where
//! the bride can't be told apart with confidence (her look-alike daughter beside her, a portrait
//! framed closer than the photo she was named on), nothing is guessed: her part of the mask is left
//! out there, the photographer is told which photos in plain words, and one click on her face
//! there names her (another example of how she looks) and computes her part again.

use std::sync::Arc;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::recipe::Who;
use lightcraft_develop::{Mask, MaskComponent, MaskOp, MaskShape, SegMask};
use serde_json::json;

use crate::Session;
use crate::segment::worker::{FoundFace, Outcome, Payload};
use crate::segment::{AutoKind, Polled, Tag};
use crate::smart_masks::Part;

fn bride() -> Who {
    Who::Role("Bride".into())
}

fn people(op: MaskOp, who: Option<Who>, seg: Option<SegMask>) -> MaskComponent {
    MaskComponent { name: None, anchor: None, op, invert: false, shape: MaskShape::People { person: 0, parts: vec!["Clothes".into()], who, seg } }
}

/// Photo `id` with "Everyone's clothes minus the bride's" (everyone's computed, hers not yet).
fn everyone_minus_the_bride(s: &mut Session, id: PhotoId) -> u32 {
    let mut d = (*s.develop_of(id).unwrap()).clone();
    let m = Mask {
        id: 7,
        name: "Pyjamas".into(),
        components: vec![people(MaskOp::Add, None, Some(SegMask::from_logits(8, &[4.0; 64]))), people(MaskOp::Subtract, Some(bride()), None)],
        ..Default::default()
    };
    d.masks = vec![m];
    s.commit("setup", Op::SetDevelop { id, settings: Arc::new(d), label: "setup".into(), edited: None }).unwrap();
    7
}

/// Photo `id`'s faces as the face model would find them (no model in tests).
fn found(s: &mut Session, id: PhotoId, rects: &[[f64; 4]]) {
    let key = s.segment_input_key(id).unwrap();
    let faces = rects.iter().map(|r| FoundFace { rect: *r, score: 0.9, signature: Default::default(), landmarks: true }).collect();
    s.segmenter.smart.faces.insert(id, (key, faces));
}

fn outcome(photo: PhotoId, mask: u32, batch: u64, result: Option<SegMask>, payload: Payload) -> Outcome {
    let part = Part::Seg(AutoKind::People { person: 0, parts: vec!["Clothes".into()], who: Some(bride()) });
    Outcome {
        tag: Tag::Recipe { photo, mask, comp: 1, part, batch, key: None },
        result: Ok(result),
        superseded: false,
        faces: None,
        payload,
        note: None,
        notice: None,
    }
}

fn her_part(s: &Session, id: PhotoId) -> MaskShape {
    s.develop_of(id).unwrap().masks[0].components[1].shape.clone()
}

#[test]
fn unsure_photos_leave_her_out_say_which_and_one_click_points_at_her() {
    let mut s = Session::with_demo();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    for id in [a, b, c] {
        s.execute("people.setRole", &json!({"role": "Bride", "person": "Alex", "scope": {"ids": [id.0]}})).unwrap();
    }
    let mask = everyone_minus_the_bride(&mut s, a);
    everyone_minus_the_bride(&mut s, b);
    everyone_minus_the_bride(&mut s, c);
    // the batch: a and b not sure (look-alikes, a close portrait), c found
    let batch = s.open_test_batch(&[a, b, c], 3);
    let mut polled = Polled::default();
    s.apply_recipe_outcome(outcome(a, mask, batch, None, Payload::Unsure), &mut polled);
    s.apply_recipe_outcome(outcome(b, mask, batch, None, Payload::Unsure), &mut polled);
    assert!(polled.messages.is_empty(), "said once, when the batch ends: {:?}", polled.messages);
    s.apply_recipe_outcome(outcome(c, mask, batch, Some(SegMask::from_logits(8, &[4.0; 64])), Payload::None), &mut polled);
    let names = |id: PhotoId| s.catalog.photo(id).unwrap().file_name.clone();
    let said: Vec<&String> = polled.messages.iter().filter(|m| m.contains("Bride wasn't found with confidence on 2 photos")).collect();
    assert_eq!(said.len(), 1, "{:?}", polled.messages);
    assert!(said[0].contains(&names(a)) && said[0].contains(&names(b)) && !said[0].contains(&names(c)), "{}", said[0]);
    assert!(!polled.messages.iter().any(|m| m.contains("wasn't recognised")), "not the old per-photo note: {:?}", polled.messages);
    // her part is settled as nothing there: the subtraction is left out, not guessed
    match her_part(&s, a) {
        MaskShape::People { seg: Some(seg), .. } => assert!(crate::smart_masks::seg_box(&seg).is_none(), "nothing subtracted"),
        other => panic!("settled: {other:?}"),
    }
    let r = s.execute("people.toPoint", &json!({"id": a.0})).unwrap();
    assert_eq!(r["people"], json!([{"who": {"role": "Bride"}, "label": "Bride"}]), "{r}");
    assert_eq!(s.execute("people.toPoint", &json!({"id": c.0})).unwrap()["people"], json!([]));

    // one click on her face on photo a: named, one more example, her part computed again
    found(&mut s, a, &[[0.20, 0.20, 0.30, 0.35], [0.45, 0.22, 0.55, 0.37]]);
    let undo_len = s.undo.len();
    let r = s.execute("people.pointAt", &json!({"id": a.0, "face": 1, "who": {"role": "Bride"}})).unwrap();
    assert_eq!((r["named"].as_u64(), r["name"].as_str(), r["recomputed"].as_u64()), (Some(1), Some("Alex"), Some(1)), "{r}");
    assert!(s.catalog.photo(a).unwrap().meta.regions.iter().any(|g| g.name.as_deref() == Some("Alex")), "her face is named");
    let examples = s.people_store().person("Alex").map(|p| p.examples.clone()).unwrap_or_default();
    assert!(examples.iter().any(|e| e.photo == a.0), "the confirmed pick is an example now");
    assert!(matches!(her_part(&s, a), MaskShape::People { seg: None, .. }), "her part waits to be computed again");
    assert_eq!(s.execute("people.toPoint", &json!({"id": a.0})).unwrap()["people"], json!([]));
    assert_eq!(s.undo.len(), undo_len + 1, "one undo step");
    // photo b is still to point at; a later result that finds her clears it
    assert_eq!(s.execute("people.toPoint", &json!({"id": b.0})).unwrap()["people"].as_array().map(Vec::len), Some(1));
    s.apply_recipe_outcome(outcome(b, mask, 0, Some(SegMask::from_logits(8, &[4.0; 64])), Payload::None), &mut polled);
    assert_eq!(s.execute("people.toPoint", &json!({"id": b.0})).unwrap()["people"], json!([]));
}

#[test]
fn pointing_needs_a_face_and_someone_to_be() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    found(&mut s, id, &[[0.2, 0.2, 0.3, 0.35]]);
    for p in [
        json!({"id": id.0, "face": 0}),
        json!({"id": id.0, "who": {"name": "Alex"}}),
        json!({"id": id.0, "face": 0, "who": {"nobody": 1}}),
        json!({"id": id.0, "face": 9, "who": {"name": "Alex"}}),
        json!({"id": id.0, "face": -1, "who": {"name": "Alex"}}),
        // a role nobody has here
        json!({"id": id.0, "face": 0, "who": {"role": "Groom"}}),
        json!({"id": 999_999, "face": 0, "who": {"name": "Alex"}}),
    ] {
        assert!(s.execute("people.pointAt", &p).is_err(), "{p}");
    }
    // a name works without a mask: the face is named, nothing to compute
    let r = s.execute("people.pointAt", &json!({"id": id.0, "face": 0, "who": {"name": "Alex"}})).unwrap();
    assert_eq!((r["named"].as_u64(), r["recomputed"].as_u64()), (Some(1), Some(0)), "{r}");
}

/// A working (tiny) recognition model, for tests: its embedding follows a crop's colours.
pub(crate) fn tiny_embedder(dim: u32) -> lightcraft_faces::runtime::Embedder {
    use lightcraft_faces::manifest::{InputSpec, Licence, ModelManifest, OutputSpec, Role, Thresholds};
    let p = std::env::temp_dir().join(format!("lc-tiny-embedder-{dim}-{}.onnx", std::process::id()));
    std::fs::write(&p, lightcraft_faces::synthetic::tiny_embedder_model(u64::from(dim))).unwrap();
    let m = ModelManifest {
        id: "tiny".into(),
        name: "Tiny".into(),
        version: "1".into(),
        role: Role::Embedder,
        licence: Licence::default(),
        source: None,
        sha256: None,
        size_bytes: None,
        provenance: String::new(),
        input: InputSpec::default(),
        output: OutputSpec::Embedding { dim },
        thresholds: Thresholds::default(),
        speed: None,
    };
    let e = lightcraft_faces::runtime::Embedder::load(&p, &m).unwrap();
    let _ = std::fs::remove_file(&p);
    e
}

/// Note 12, part 1: with a recognition model in use (Settings ▸ Faces), the bride's named faces'
/// embeddings from the People index (this shoot's first) go with the request, so the worker
/// matches her by face first; without one nothing changes.
#[test]
fn her_named_faces_go_with_the_request_when_a_recognition_model_is_in_use() {
    use crate::smart_masks::Resolved;
    use lightcraft_meta::{Region, RegionKind};
    let mut s = Session::with_demo();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let (a, b) = (ids[0], ids[1]);
    s.execute("people.setRole", &json!({"role": "Bride", "person": "Alex", "scope": {"ids": [a.0, b.0]}})).unwrap();
    let r = lightcraft_geom::Rect { x0: 0.4, y0: 0.2, x1: 0.6, y1: 0.5 };
    let mut meta = s.catalog.photo(b).unwrap().meta.clone();
    meta.regions = vec![Region { rect: r, kind: RegionKind::Face, name: Some("Alex".into()), description: None }];
    s.commit("setup", Op::SetMeta { id: b, meta: Box::new(meta) }).unwrap();
    // no recognition model in use: nothing to find her by on photo a
    assert!(matches!(s.resolve_who(a, &bride()), Resolved::Missing(_)));
    assert!(s.face_identity(Vec::new()).is_none());
    // a model in use, her face on b embedded by it
    s.faces.index.reset("tiny@1", 16, None);
    s.faces.index.insert(b.0, &r, vec![0.5; 16]);
    s.faces.embedder = Some(("tiny@1".into(), Arc::new(tiny_embedder(16))));
    let Resolved::Person { rect: None, faces, .. } = s.resolve_who(a, &bride()) else { panic!("found by her named face") };
    assert_eq!(faces.len(), 1);
    let ident = s.face_identity(faces).expect("an identity for the worker");
    assert_eq!((ident.tag.as_str(), ident.known.len(), ident.threshold), ("tiny@1", 1, 0.45));
    // on b itself she is named: her box decides there, no embeddings needed
    let Resolved::Person { rect: Some(_), faces, .. } = s.resolve_who(b, &bride()) else { panic!("named here") };
    assert!(faces.is_empty());
}
