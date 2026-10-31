//! A mask's own Color Mixer (beyond Lightroom: Capture One has one per layer): the commands, and the
//! places a mask's settings go — the sidecar's and the catalog's settings, a LightCraft XMP preset,
//! copy and paste, and Auto Sync onto the selected linked copies.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, Hsl, Mask, Mixer};
use serde_json::json;

use crate::Session;

fn dev(s: &Session, id: PhotoId) -> DevelopSettings {
    (*s.develop_of(id).unwrap()).clone()
}

/// The demo photo with one radial mask (selected) in its middle.
fn with_mask() -> (Session, PhotoId, u32) {
    let mut s = Session::with_demo();
    s.execute("mask.add", &json!({"kind": "radial", "center": {"x": 0.5, "y": 0.5}, "rx": 0.3, "ry": 0.3})).unwrap();
    let photo = s.active().unwrap();
    let id = dev(&s, photo).masks[0].id;
    (s, photo, id)
}

/// A mixer with every band moved (each slider a different value).
fn every_band() -> Mixer {
    let mut m = Mixer::default();
    for i in 0..8 {
        let k = i as f64;
        *m.band_mut(i).unwrap() = Hsl { hue: 10.0 + k, sat: -20.0 - k, lum: 30.0 + k };
    }
    m
}

#[test]
fn mask_mixer_command_sets_validates_renders_and_undoes() {
    let (mut s, photo, id) = with_mask();
    let before = s.render_now(photo, 256, 256).unwrap().image;
    let r = s
        .execute("mask.mixer", &json!({"id": id, "values": {"red.sat": 80, "orange.sat": 100, "yellow.sat": 100, "green.lum": -60, "blue.hue": 250}}))
        .unwrap();
    assert_eq!(r["mixer"]["blue.hue"], 100.0, "clamped to the slider's range");
    let m = &dev(&s, photo).masks[0];
    assert_eq!((m.mixer.red.sat, m.mixer.green.lum, m.mixer.blue.hue), (80.0, -60.0, 100.0));
    assert!(dev(&s, photo).mixer.is_neutral(), "the global mixer is untouched");
    let after = s.render_now(photo, 256, 256).unwrap().image;
    let at = |img: &lightcraft_raster::Rgba8, x: usize, y: usize| img.data[y * img.width + x];
    let inside = (after.width / 4..after.width * 3 / 4).filter(|x| at(&after, *x, after.height / 2) != at(&before, *x, before.height / 2)).count();
    assert!(inside > 0, "pixels inside the mask change");
    assert_eq!(at(&after, 2, 2), at(&before, 2, 2), "the corner (outside the mask) does not");

    // hostile input: nothing changes and the answer says why
    let was = dev(&s, photo);
    for bad in [
        json!({}),
        json!({"values": {"red.hue": "a lot"}}),
        json!({"values": {"teal.hue": 10}}),
        json!({"values": {"red.chroma": 10}}),
        json!({"values": {"red": 10}}),
        json!({"values": [1, 2]}),
        json!({"id": 999, "values": {"red.hue": 10}}),
        json!({"values": {"red.hue": null}}),
        json!({"values": {"red.hue": 10, "nope": 1}}),
    ] {
        assert!(s.execute("mask.mixer", &bad).is_err(), "{bad} is refused");
    }
    assert_eq!(dev(&s, photo), was, "a refused call changes nothing");

    // reset, then undo brings it back
    s.execute("mask.mixer", &json!({"reset": true})).unwrap();
    assert!(dev(&s, photo).masks[0].mixer.is_neutral());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(dev(&s, photo).masks[0].mixer.red.sat, 80.0);
    // a reset with values: everything else back to 0
    s.execute("mask.mixer", &json!({"reset": true, "values": {"aqua.lum": 5}})).unwrap();
    let mut want = Mixer::default();
    want.aqua.lum = 5.0;
    assert_eq!(dev(&s, photo).masks[0].mixer, want);
}

#[test]
fn mask_mixer_targeted_moves_the_bands_of_the_colour_under_the_point() {
    let (mut s, photo, _) = with_mask();
    let r = s.execute("mask.mixerTargeted", &json!({"target": "sat", "x": 0.5, "y": 0.5, "delta": 20})).unwrap();
    let changed = r.as_object().unwrap();
    assert!(!changed.is_empty() && changed.keys().all(|k| k.ends_with(".sat")), "{r}");
    let m = dev(&s, photo).masks[0].mixer;
    assert!(m.bands().iter().any(|b| b.sat > 0.0) && m.bands().iter().all(|b| b.hue == 0.0 && b.lum == 0.0));
    assert!(dev(&s, photo).mixer.is_neutral(), "the global mixer is untouched");
    for bad in [
        json!({"target": "vibrance", "x": 0.5, "y": 0.5, "delta": 1}),
        json!({"target": "hue", "x": 0.5, "delta": 1}),
        json!({"target": "hue", "x": 0.5, "y": 0.5}),
        json!({"id": 999, "target": "hue", "x": 0.5, "y": 0.5, "delta": 1}),
    ] {
        assert!(s.execute("mask.mixerTargeted", &bad).is_err(), "{bad} is refused");
    }
}

#[test]
fn the_mask_mixer_survives_the_sidecar_and_the_catalog() {
    use crate::sidecar::{DevelopPatch, parse_sidecar, sidecar_packet};
    let (mut s, photo, id) = with_mask();
    let mut d = dev(&s, photo);
    d.masks[0].mixer = every_band();
    s.set_develop(photo, d, "test").unwrap();
    let p = s.catalog.photo(photo).unwrap().clone();
    let packet = sidecar_packet(&p, &s.catalog);
    let Some(DevelopPatch::Full(back)) = parse_sidecar(&packet, crate::crs::Target::Rendered).unwrap().develop else { panic!("no lc:settings") };
    let m = back.masks.iter().find(|m| m.id == id).unwrap();
    assert_eq!(m.mixer, every_band(), "every slider comes back from the sidecar");
    // the catalog keeps a photo's settings as their JSON
    let cat: lightcraft_catalog::Photo = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(cat.develop.masks[0].mixer, every_band());
    // a sidecar from before (or a mask without one) reads as neutral
    let mut plain = (*p.develop).clone();
    plain.masks[0].mixer = Mixer::default();
    let json = plain.to_json();
    assert!(json["masks"][0].get("mixer").is_none(), "a neutral local mixer isn't written");
    assert!(json.get("mixer").is_some(), "(the global one is)");
    let old: Mask = serde_json::from_str(r#"{"id": 3, "name": "Old", "visible": true, "invert": false, "components": [], "adjust": {}}"#).unwrap();
    assert!(old.mixer.is_neutral());
}

#[test]
fn the_mask_mixer_goes_with_presets_and_copy_paste() {
    let (mut s, photo, _) = with_mask();
    s.execute("mask.mixer", &json!({"values": {"orange.hue": -40, "blue.sat": -70}})).unwrap();
    // a LightCraft XMP preset carries it (Lightroom can't: the export says so) and reads it back
    let r = s.execute("preset.create", &json!({"name": "Local Mix", "groups": ["masks"]})).unwrap();
    let pid = r["id"].as_str().unwrap().to_string();
    let preset = s.presets.iter().find(|p| p.id == pid).unwrap().clone();
    let (xmp, notes) = crate::preset_xmp::to_xmp(&preset);
    assert!(notes.iter().any(|n| n.contains("Color Mixer")), "{notes:?}");
    let back = crate::preset_import::read_presets("Local Mix.xmp", xmp.as_bytes(), None).unwrap();
    let masks: Vec<Mask> = serde_json::from_value(back[0].preset.settings["masks"].clone()).unwrap();
    assert_eq!((masks[0].mixer.orange.hue, masks[0].mixer.blue.sat), (-40.0, -70.0));
    // applied to another photo: the mask arrives with its mixer
    let other = s.visible_cloned().into_iter().find(|i| *i != photo).unwrap();
    s.execute("library.select", &json!({"ids": [other.0], "active": other.0})).unwrap();
    s.execute("preset.apply", &json!({"id": pid})).unwrap();
    assert_eq!(dev(&s, other).masks.last().unwrap().mixer.orange.hue, -40.0);
    // copy / paste with the masks
    let third = s.visible_cloned().into_iter().find(|i| *i != photo && *i != other).unwrap();
    s.execute("library.select", &json!({"ids": [photo.0], "active": photo.0})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("develop.paste", &json!({"ids": [third.0]})).unwrap();
    assert_eq!(dev(&s, third).masks.last().unwrap().mixer.blue.sat, -70.0);
}

/// With Auto Sync on, a mixer slider on one photo reaches the linked copies on the other selected
/// photos (each slider that changed, at the same value; the copy's other sliders stay its own); a
/// copy on a photo that isn't selected stays as it was.
#[test]
fn auto_sync_carries_the_mask_mixer_to_the_selected_linked_copies() {
    let mut s = Session::with_demo();
    let ids: Vec<PhotoId> = s.visible_cloned().into_iter().take(3).collect();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let select = |s: &mut Session, ids: &[PhotoId]| {
        s.execute("library.select", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "active": ids[0].0})).unwrap();
    };
    select(&mut s, &[a, b, c]);
    s.execute("develop.autoSync", &json!({"on": true})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial"})).unwrap();
    let mid = dev(&s, a).masks[0].id;
    // b's copy has a slider of its own
    let mut d = dev(&s, b);
    d.masks[0].mixer.aqua.lum = 25.0;
    s.amend_develop(b, d);
    select(&mut s, &[a, b]);
    s.execute("mask.mixer", &json!({"id": mid, "values": {"red.hue": 30}})).unwrap();
    assert_eq!(dev(&s, b).masks[0].mixer.red.hue, 30.0, "the selected copy takes the slider");
    assert_eq!(dev(&s, b).masks[0].mixer.aqua.lum, 25.0, "and keeps its own other sliders");
    assert!(dev(&s, c).masks[0].mixer.is_neutral(), "a copy on a photo that isn't selected stays");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(dev(&s, b).masks[0].mixer.red.hue, 0.0, "one undo step for both");
}
