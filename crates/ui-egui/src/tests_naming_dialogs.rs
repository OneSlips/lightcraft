//! Rename and Export naming (client notes 35, 38): naming presets, `{text:…}` fields, + Number.

use std::time::Duration;

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::headless::Headless;
use crate::state::Dialog;

const T: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_secs(120);

fn app() -> Headless {
    let services = crate::Services { png: None, ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    Headless::new(app, [1400.0, 1000.0], 1.0)
}

fn exec(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

fn has_widget(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

/// A naming preset with `{text:…}` asks for the text in Rename; the files are named with it, in
/// the order shown.
#[test]
fn rename_with_a_text_preset() {
    let mut h = app();
    exec(&mut h, "naming.savePreset", json!({"name": "Delivery", "template": "{seq:1}_{text:Couple initials}_Studio"}));
    let vis: Vec<u64> = h.app.session.visible_cloned().iter().take(3).map(|p| p.0).collect();
    exec(&mut h, "library.select", json!({"ids": [vis[2], vis[0]]}));
    exec(&mut h, "dialog.rename", json!({"template": "{seq:1}_{text:Couple initials}_Studio"}));
    h.settle(SETTLE);
    assert!(has_widget(&h, "field:renameText0"), "a field for the couple's initials");
    assert!(has_widget(&h, "combo:renameNamingPreset"));
    if let Some(Dialog::Rename { texts, .. }) = &mut h.app.ui.dialog {
        texts.insert("Couple initials".into(), "R+A".into());
    }
    h.settle(SETTLE);
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    let name = |id: u64| h.app.session.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().file_name.clone();
    assert!(name(vis[0]).starts_with("1_R+A_Studio"), "{}", name(vis[0]));
    assert!(name(vis[2]).starts_with("2_R+A_Studio"), "the first shown is 1: {}", name(vis[2]));
}

/// + Number puts `{seq}` in the template; it is off once the template numbers the photos.
#[test]
fn number_chip_adds_seq() {
    let mut h = app();
    let vis: Vec<u64> = h.app.session.visible_cloned().iter().take(2).map(|p| p.0).collect();
    exec(&mut h, "library.select", json!({"ids": vis}));
    exec(&mut h, "dialog.rename", json!({"template": "Wedding"}));
    h.settle(SETTLE);
    let r = h.request("ui.clickWidget", json!({"id": "button:renameTemplateNumber"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    match &h.app.ui.dialog {
        Some(Dialog::Rename { template, .. }) => assert_eq!(template, "Wedding_{seq}"),
        d => panic!("dialog: {d:?}"),
    }
}

/// Export asks for the naming's texts too and keeps them out of a saved preset.
#[test]
fn export_dialog_asks_for_texts() {
    let mut h = app();
    exec(&mut h, "dialog.export", json!({}));
    if let Some(Dialog::Export { opts, .. }) = &mut h.app.ui.dialog {
        opts.naming = "{seq:1}_{text:Couple initials}_Wedding".into();
    }
    h.settle(SETTLE);
    assert!(has_widget(&h, "field:exportText0"));
    assert!(has_widget(&h, "button:exportNamingNumber"));
    assert!(has_widget(&h, "combo:exportNamingPreset"));
    h.request("ui.clickWidget", json!({"id": "button:dialogCancel"}), T);
}

/// Note 38: in a collection, dragging a photo onto another's left half puts it before that one.
#[test]
fn drag_to_reorder_a_collection() {
    let mut h = app();
    let four: Vec<u64> = h.app.session.visible_cloned().iter().take(4).map(|p| p.0).collect();
    let a = exec(&mut h, "album.create", json!({"name": "Family"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": four}));
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    exec(&mut h, "library.sort", json!({"key": "custom", "ascending": true}));
    exec(&mut h, "view.photoGrid", json!({}));
    h.settle(SETTLE);
    let rect = |h: &Headless, id: u64| {
        h.app.widgets.iter().find(|(w, _)| *w == format!("thumb:{id}")).map(|(_, r)| *r).unwrap_or_else(|| panic!("thumb {id}"))
    };
    let from = rect(&h, four[3]).center();
    let to = rect(&h, four[0]);
    let r = h.request("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.left() + 6.0, "toY": to.center().y, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    let order: Vec<u64> = h.app.session.visible_cloned().iter().map(|p| p.0).collect();
    assert_eq!(order, vec![four[3], four[0], four[1], four[2]]);
    assert!(h.app.ui.dragging_photos.is_none());
}
