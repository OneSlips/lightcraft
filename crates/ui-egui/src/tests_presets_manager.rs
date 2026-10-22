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

/// The rectangle `[x, y, w, h]` of widget `id` on screen.
fn rect(h: &mut Headless, id: &str) -> Option<[f64; 4]> {
    let r = h.request("ui.widgets", json!({ "filter": id }), T);
    let w = r["result"].as_array()?.iter().find(|w| w["id"] == id)?.clone();
    let v: Vec<f64> = w["rect"].as_array()?.iter().filter_map(|x| x.as_f64()).collect();
    Some([*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?])
}

/// LR-PRE-MANAGE: a group's Delete asks at the group, right under its header (not at the top of
/// the list, out of view), and the actions sit in the same column on every row.
#[test]
fn a_groups_delete_asks_at_the_group_and_actions_line_up() {
    let mut h = app(400);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "presets".into() });
    h.step();
    h.step();
    // the lowest group header on screen
    let heads = h.request("ui.widgets", json!({ "filter": "presetGroupHead:" }), T);
    let mut heads: Vec<(f64, String)> = heads["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["rect"][1].as_f64().unwrap(), w["id"].as_str().unwrap().trim_start_matches("presetGroupHead:").to_string()))
        .collect();
    heads.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(heads.len() > 5, "{heads:?}");
    let (_, low) = heads[heads.len() / 2].clone();
    let (_, high) = heads[1].clone();
    // the actions form columns: the same x on two groups' rows
    let a = rect(&mut h, &format!("icon:presetGroupDelete:{high}")).unwrap();
    let b = rect(&mut h, &format!("icon:presetGroupDelete:{low}")).unwrap();
    assert_eq!((a[0], a[2]), (b[0], b[2]), "Delete in one column");
    let a = rect(&mut h, &format!("icon:presetGroupHide:{high}")).unwrap();
    let b = rect(&mut h, &format!("icon:presetGroupHide:{low}")).unwrap();
    assert_eq!(a[0], b[0], "Hide in one column");
    let head = rect(&mut h, &format!("presetGroupHead:{low}")).unwrap();
    click(&mut h, &format!("icon:presetGroupDelete:{low}"));
    let ask =
        rect(&mut h, &format!("label:groupDeleteAsk:{low}")).unwrap_or_else(|| panic!("the question shows: {}", widgets(&mut h, "groupDelete")));
    assert!(ask[1] > head[1] && ask[1] - (head[1] + head[3]) < 40.0, "under the header: head {head:?}, ask {ask:?}");
    assert!(rect(&mut h, &format!("button:groupDeleteConfirm:{low}")).is_some_and(|r| r[1] + r[3] < 1000.0), "in view");
    click(&mut h, &format!("button:groupDeleteConfirm:{low}"));
    assert!(!h.app.session.presets.iter().any(|p| p.group == low));
    // one preset's Delete asks in its own row
    click(&mut h, &format!("icon:presetGroupFold:{high}"));
    let id = h.app.session.presets.iter().find(|p| p.group == high).unwrap().id.clone();
    click(&mut h, &format!("icon:presetDelete:{id}"));
    assert!(rect(&mut h, &format!("button:preset:{id}DeleteConfirm")).is_some());
    click(&mut h, &format!("button:preset:{id}DeleteConfirm"));
    assert!(!h.app.session.presets.iter().any(|p| p.id == id));
}

/// LR-PRESET-PICKER: a preset picker (Settings ▸ Import's raw default) opens with search, groups in
/// the manager's order and no hidden presets; a search finds one across 400 presets and picking it
/// sets it; At Import marks from the manager.
#[test]
fn the_shared_preset_picker_searches_and_picks() {
    let mut h = app(400);
    h.app.session.execute("preset.hide", &json!({"id": "user.m5"})).unwrap();
    h.app.ui.dialog = Some(Dialog::Settings { tab: "import".into() });
    h.step();
    h.step();
    click(&mut h, "presetPicker:settingsRawPreset");
    let groups = widgets(&mut h, "presetPickerGroup:settingsRawPreset:");
    assert!(groups.contains("presetPickerGroup:settingsRawPreset:Collection 0"), "{groups}");
    // a search opens what it finds; the hidden preset isn't there
    crate::preset_picker::set_query(&h.view.ctx, "settingsRawPreset", "look 5");
    h.step();
    h.step();
    let rows = widgets(&mut h, "presetPickerRow:settingsRawPreset:");
    assert!(rows.contains("user.m50\"") && !rows.contains("user.m5\""), "{rows}");
    click(&mut h, "presetPickerRow:settingsRawPreset:user.m50");
    assert_eq!(h.app.session.import_defaults.raw_preset.as_deref(), Some("user.m50"));
    // At Import from the manager's column
    h.app.ui.dialog = Some(Dialog::Settings { tab: "presets".into() });
    h.step();
    h.step();
    click(&mut h, "icon:presetGroupFold:Collection 1");
    click(&mut h, "icon:presetForImport:user.m1");
    let p = h.app.session.presets.iter().find(|p| p.id == "user.m1").unwrap().clone();
    assert!(h.app.session.preset_for_import(&p));
}

fn wrect(h: &mut Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

/// Client note 9: Ctrl-click two presets and drag them onto another group's header: they move into
/// it; drag a preset onto another's lower half: it goes after it.
#[test]
fn presets_are_dragged_into_groups_and_places() {
    let mut h = app(42);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "presets".into() });
    h.step();
    h.step();
    // Collection 0 holds m0 and m400 after the fold opens; Collection 1 holds m1 and m41
    click(&mut h, "icon:presetGroupFold:Collection 0");
    click(&mut h, "icon:presetGroupFold:Collection 1");
    let a = wrect(&mut h, "presetRow:user.m0").center();
    h.request("ui.click", json!({"x": a.x, "y": a.y}), T);
    h.step();
    let b = wrect(&mut h, "presetRow:user.m40").center();
    h.request("ui.click", json!({"x": b.x, "y": b.y, "cmd": true}), T);
    h.step();
    let to = wrect(&mut h, "presetGroupHead:Collection 10").center();
    let r = h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": to.x, "toY": to.y, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    let group = |h: &Headless, id: &str| h.app.session.presets.iter().find(|p| p.id == id).unwrap().group.clone();
    assert_eq!(group(&h, "user.m0"), "Collection 10");
    assert_eq!(group(&h, "user.m40"), "Collection 10");
    // m41 dropped on m1's lower half: after it, in Collection 1
    let from = wrect(&mut h, "presetRow:user.m41").center();
    let m1 = wrect(&mut h, "presetRow:user.m1");
    h.request("ui.drag", json!({"x": from.x, "y": from.y, "toX": m1.center().x, "toY": m1.bottom() - 4.0, "steps": 12}), T);
    h.step();
    let order: Vec<&str> = h.app.session.presets.iter().filter(|p| p.group == "Collection 1").map(|p| p.id.as_str()).collect();
    assert_eq!(order, vec!["user.m1", "user.m41"]);
}
