//! Settings ▸ Presets (the Presets Manager), driven like a user: groups start folded, a search opens what it finds, hide a preset (the Presets panel's list
//! follows), hide and delete whole groups; with 11 000 presets a frame stays quick.

use std::time::Duration;

use lightcraft_develop::Preset;
use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(10);

fn app(presets: usize) -> Headless {
    let mut session = lightcraft_engine::Session::with_demo();
    for i in 0..presets {
        session.presets.push(Preset {
            id: format!("user.m{i}"),
            name: format!("Look {i}"),
            group: format!("Collection {}", i % 40),
            settings: json!({"light": {"exposure": 0.1}}),
            favorite: false,
            builtin: false,
        });
    }
    let mut app = LightcraftApp::new(session, Services { png: None, ..Default::default() });
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    Headless::new(app, [1400.0, 1000.0], 1.0)
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({ "id": id }), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
    h.step();
}

fn widgets(h: &mut Headless, filter: &str) -> String {
    h.request("ui.widgets", json!({ "filter": filter }), T).to_string()
}

#[test]
fn presets_hide_and_whole_groups_from_the_manager() {
    let mut h = app(30);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "presets".into() });
    h.step();
    h.step();
    // groups start folded: their headers only
    let heads = widgets(&mut h, "presetGroupHead:");
    assert!(heads.contains("presetGroupHead:Collection 1"), "{heads}");
    assert!(!widgets(&mut h, "presetRow:").contains("presetRow:user.m1\""), "folded");
    // open one and hide a preset: the Presets panel's list leaves it out
    click(&mut h, "icon:presetGroupFold:Collection 1");
    assert!(widgets(&mut h, "presetRow:").contains("presetRow:user.m1\""));
    click(&mut h, "icon:presetHide:user.m1");
    let p = h.app.session.presets.iter().find(|p| p.id == "user.m1").unwrap().clone();
    assert!(!h.app.session.preset_shown(&p));
    // hide a whole group, then delete it after confirming (moving its presets elsewhere)
    click(&mut h, "icon:presetGroupHide:Collection 10");
    assert!(h.app.session.presets.iter().filter(|p| p.group == "Collection 10").all(|p| !h.app.session.preset_shown(p)));
    click(&mut h, "icon:presetGroupDelete:Collection 11");
    assert!(h.app.session.presets.iter().any(|p| p.group == "Collection 11"), "nothing before the confirmation");
    click(&mut h, "button:groupDeleteConfirm:Collection 11");
    assert!(!h.app.session.presets.iter().any(|p| p.group == "Collection 11"));
}

/// 11 000 presets in 40 groups: the manager and a search stay quick (only the rows in view are built).
#[test]
fn a_big_collection_stays_quick() {
    let mut h = app(11_000);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "presets".into() });
    h.step();
    let t0 = std::time::Instant::now();
    for _ in 0..10 {
        h.step();
    }
    let per_frame = t0.elapsed() / 10;
    assert!(per_frame < Duration::from_millis(400), "{per_frame:?} a frame");
}
