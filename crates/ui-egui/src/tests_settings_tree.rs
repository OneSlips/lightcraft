//! The per-setting tree of New Preset, Update Preset, Copy and Sync (client notes 8, 11, 13).

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
    let mut h = Headless::new(app, [1400.0, 1000.0], 1.0);
    let first = h.app.session.visible_cloned()[0].0;
    exec(&mut h, "library.select", json!({"ids": [first]}));
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 1.0, "light.contrast": 20, "effects.clarity": 10}}));
    h
}

fn exec(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}

fn ticks(h: &Headless) -> Vec<String> {
    match &h.app.ui.dialog {
        Some(
            Dialog::CreatePreset { groups, .. }
            | Dialog::UpdatePreset { groups, .. }
            | Dialog::CopySettings { groups }
            | Dialog::SyncSettings { groups },
        ) => groups.clone(),
        d => panic!("dialog: {d:?}"),
    }
}

/// New Preset: open Light, untick Exposure (Light shows a partial state), save; the preset leaves
/// exposure out and White Balance is unticked unless chosen.
#[test]
fn new_preset_without_exposure() {
    let mut h = app();
    h.app.run("dialog.createPreset", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(!ticks(&h).iter().any(|t| t == "whiteBalance"), "White Balance starts unticked");
    assert!(ticks(&h).iter().any(|t| t == "light.exposure"));
    click(&mut h, "presetInclude:open:light");
    click(&mut h, "presetInclude:light.exposure");
    assert!(!ticks(&h).iter().any(|t| t == "light.exposure") && ticks(&h).iter().any(|t| t == "light.contrast"));
    if let Some(Dialog::CreatePreset { name, .. }) = &mut h.app.ui.dialog {
        *name = "No exposure".into();
    }
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    let p = h.app.session.presets.iter().find(|p| p.name == "No exposure").expect("created").clone();
    assert!(p.settings["light"].get("exposure").is_none() && p.settings["light"]["contrast"] == 20.0, "{}", p.settings);
    assert!(p.settings.get("wb").is_none());
    // the next New Preset starts the same way
    h.app.run("dialog.createPreset", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(!ticks(&h).iter().any(|t| t == "light.exposure"), "remembered");
    // a partly ticked group's box ticks all of it
    click(&mut h, "presetInclude:light");
    assert!(ticks(&h).iter().any(|t| t == "light.exposure"));
    click(&mut h, "button:dialogCancel");
}

/// Update Preset: pre-ticked with what the preset holds; confirming updates it and offers Undo.
#[test]
fn update_preset_dialog() {
    let mut h = app();
    let pid = exec(&mut h, "preset.create", json!({"name": "Mine", "settings": ["light.contrast", "effects.clarity"]}))["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    exec(&mut h, "develop.set", json!({"values": {"light.contrast": 55, "effects.clarity": 33, "light.exposure": -1}}));
    h.app.run("dialog.updatePreset", json!({"id": pid})).unwrap();
    h.settle(SETTLE);
    assert_eq!(ticks(&h), vec!["light.contrast", "effects.clarity"]);
    assert!(h.app.run("dialog.updatePreset", json!({"id": "lc.moody"})).is_err(), "built-in presets stay");
    h.app.ui.dialog =
        Some(Dialog::UpdatePreset { id: pid.clone(), name: "Mine".into(), groups: vec!["light.contrast".into(), "effects.clarity".into()] });
    h.settle(SETTLE);
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    let p = h.app.session.presets.iter().find(|p| p.id == pid).unwrap().clone();
    assert_eq!(p.settings, json!({"light": {"contrast": 55.0}, "effects": {"clarity": 33.0}}));
    let action = h.app.ui.toast_action.clone().expect("Undo on the toast");
    assert_eq!(action.command, "preset.undoUpdate");
    h.app.run(&action.command, json!({})).unwrap();
    let p = h.app.session.presets.iter().find(|p| p.id == pid).unwrap().clone();
    assert_eq!(p.settings, json!({"light": {"contrast": 20.0}, "effects": {"clarity": 10.0}}));
}

/// Copy and Sync open with what they ticked last.
#[test]
fn copy_and_sync_remember_their_ticks() {
    let mut h = app();
    h.app.run("dialog.copySettings", json!({})).unwrap();
    h.settle(SETTLE);
    click(&mut h, "copyGroup:none");
    click(&mut h, "copyGroup:open:light");
    click(&mut h, "copyGroup:light.exposure");
    assert_eq!(ticks(&h), vec!["light.exposure"]);
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    assert_eq!(h.app.session.clipboard.clone().unwrap(), json!({"light": {"exposure": 1.0}}));
    h.app.run("dialog.copySettings", json!({})).unwrap();
    h.settle(SETTLE);
    assert_eq!(ticks(&h), vec!["light.exposure"]);
    click(&mut h, "button:dialogCancel");
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(2).map(|p| p.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids, "active": ids[0]}));
    h.app.run("dialog.syncSettings", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(ticks(&h).iter().any(|t| t == "whiteBalance"), "Sync starts with the copy defaults");
}
