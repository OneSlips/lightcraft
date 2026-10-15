//! Settings ▸ Backups, driven like a user: how often, back up now (in the background), the backup listed.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(10);

fn app(session: lightcraft_engine::Session) -> Headless {
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

/// Widgets on screen whose id contains `filter`, as text.
fn widgets(h: &mut Headless, filter: &str) -> String {
    h.request("ui.widgets", json!({ "filter": filter }), T).to_string()
}

#[test]
fn back_up_from_the_backups_tab_and_see_it_listed() {
    let dir = std::env::temp_dir().join(format!("lc-ui-backups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = lightcraft_engine::Session::new().with_backups(&dir.join("settings"));
    s.open_library(dir.join("Lib"), true).unwrap();
    let mut h = app(s);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "backups".into() });
    h.step();
    h.step();
    click(&mut h, "button:backupFrequency-0");
    assert_eq!(h.app.session.backups.settings.frequency, lightcraft_engine::backup::Frequency::EveryExit);
    click(&mut h, "button:backUpNow");
    // it runs in the background, with a row in the activity stack
    assert!(h.step_until(Duration::from_secs(60), |h| !h.app.session.backup_running() && h.app.session.backups.last_result.is_some()));
    h.step();
    h.step();
    let listed = h.app.session.execute("library.backups", &json!({})).unwrap();
    assert_eq!(listed["backups"].as_array().unwrap().len(), 1, "{listed}");
    assert!(widgets(&mut h, "backupRestore:").contains("button:backupRestore:"), "the backup is listed with Restore");
    let _ = h.app.session.close_library();
    let _ = std::fs::remove_dir_all(&dir);
}
