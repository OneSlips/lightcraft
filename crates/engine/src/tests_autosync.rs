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
