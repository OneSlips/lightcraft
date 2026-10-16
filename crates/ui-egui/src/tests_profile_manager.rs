//! Settings ▸ Profiles (the Profile Manager), driven like a user: hide and delete a built-in look
//! (the menus follow), restore it; the browser's and the menu's ways in.

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

fn shown(h: &Headless, id: &str) -> bool {
    h.app.session.shown_profiles().iter().any(|p| p.id == id)
}

#[test]
fn hide_delete_and_restore_a_look_from_the_profile_manager() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.app.ui.dialog = Some(Dialog::Settings { tab: "profiles".into() });
    h.step();
    h.step();
    assert!(widgets(&mut h, "profileRow:").contains("profileRow:lc.color"), "the profiles are listed");
    // Hide: out of the menu, still in the manager, marked
    click(&mut h, "icon:profileHide:lc.vivid");
    assert!(!shown(&h, "lc.vivid"));
    assert!(!h.app.session.profile_menu().to_string().contains("\"lc.vivid\""));
    assert!(widgets(&mut h, "profileRow:").contains("profileRow:lc.vivid"), "a hidden profile stays in the manager");
    click(&mut h, "icon:profileHide:lc.vivid");
    assert!(shown(&h, "lc.vivid"), "one click shows it again");
    // Delete asks first, then the look is gone from this install until restored
    click(&mut h, "icon:profileDelete:lc.portrait");
    assert!(shown(&h, "lc.portrait"), "nothing happens before the confirmation");
    click(&mut h, "button:profileDeleteConfirm:lc.portrait");
    assert!(!h.app.session.all_profiles().iter().any(|p| p.id == "lc.portrait"));
    click(&mut h, "button:profilesRestoreBuiltins");
    assert!(shown(&h, "lc.portrait"));
    // the profile browser's Manage… and the menu's Manage Profiles… open this tab
    h.app.ui.dialog = None;
    h.app.run("app.settings", json!({"tab": "profiles"})).unwrap();
    assert_eq!(h.app.ui.dialog, Some(Dialog::Settings { tab: "profiles".into() }));
}

#[test]
fn profile_groups_hide_and_delete_from_their_header() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.app.ui.dialog = Some(Dialog::Settings { tab: "profiles".into() });
    h.step();
    h.step();
    // (the table's rows are roomier: fold the first group to bring Film into view)
    click(&mut h, "icon:profileGroupFold:Basic");
    click(&mut h, "icon:profileGroupHide:Film");
    assert!(!h.app.session.shown_profiles().iter().any(|p| p.group == "Film"));
    click(&mut h, "icon:profileGroupDelete:Film");
    click(&mut h, "button:groupDeleteConfirm:Film");
    assert!(!h.app.session.profile_groups_all().contains(&"Film".to_string()));
}
