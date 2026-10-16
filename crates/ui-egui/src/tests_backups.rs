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

/// LR-BACKUP: quitting with a backup due asks first, as Lightroom Classic does: Back Up and Quit, or
/// Skip This Time. After a backup, quitting with nothing changed asks nothing (and backs up
/// nothing); a change asks again. `app.quit` over the control channel never waits for an answer.
#[test]
fn quitting_asks_to_back_up_only_when_something_changed() {
    let dir = std::env::temp_dir().join(format!("lc-ui-backup-quit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = lightcraft_engine::Session::new().with_backups(&dir.join("settings"));
    s.open_library(dir.join("Lib"), true).unwrap();
    s.execute("library.backupSettings", &json!({"frequency": "everyExit"})).unwrap();
    let mut h = app(s);
    h.step();
    // a new library: no backup yet, so it asks
    assert!(!crate::panels::notices::may_close(&mut h.app));
    assert!(matches!(h.app.quit_prompt, Some(crate::QuitPrompt::Backup(_))), "{:?}", h.app.quit_prompt);
    h.step();
    h.step();
    click(&mut h, "button:quitSkipBackup");
    assert_eq!(h.app.backup_on_quit, crate::BackupOnQuit::Skip);
    assert!(h.app.quit_prompt.is_none() && !h.app.backup_at_exit(), "skipped this time");
    // backed up: nothing changed since, so quitting asks nothing and backs up nothing
    h.app.backup_on_quit = crate::BackupOnQuit::Ask;
    h.app.session.back_up_library(false).unwrap();
    assert!(!h.app.session.backup_changed());
    assert!(crate::panels::notices::may_close(&mut h.app), "nothing to ask");
    assert!(h.app.quit_prompt.is_none() && !h.app.backup_at_exit());
    // a change: it asks again; Back Up and Quit backs up as LightCraft exits
    h.app.backup_on_quit = crate::BackupOnQuit::Ask;
    h.app.session.execute("album.create", &json!({"name": "Alex"})).unwrap();
    let _ = h.app.session.persist();
    assert!(h.app.session.backup_changed());
    assert!(!crate::panels::notices::may_close(&mut h.app));
    h.step();
    h.step();
    click(&mut h, "button:quitBackUp");
    assert_eq!(h.app.backup_on_quit, crate::BackupOnQuit::Now);
    assert!(h.app.backup_at_exit());
    // the control channel's quit doesn't wait for an answer
    h.app.backup_on_quit = crate::BackupOnQuit::Auto;
    assert!(crate::panels::notices::may_close(&mut h.app) && h.app.quit_prompt.is_none());
    let _ = h.app.session.close_library();
    let _ = std::fs::remove_dir_all(&dir);
}
