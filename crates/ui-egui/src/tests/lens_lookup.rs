//! Enable Lens Corrections looks up again only the photos it switched on (and selected photos that already had
//! corrections on): with Auto Sync off the other selected photos are left alone.

use lightcraft_engine::Session;
use serde_json::json;

#[test]
fn ticking_lens_corrections_looks_up_only_the_photos_it_changed() {
    let mut s = Session::with_demo();
    s.execute("library.selectAll", &json!({})).unwrap();
    let active = s.selection.active.unwrap();
    assert!(s.selection.ids.len() > 2);
    let lp = |s: &Session| s.selection.ids.iter().filter(|id| s.catalog.photo(**id).unwrap().develop.optics.lens_profile).count();
    assert_eq!(lp(&s), 0);
    // Auto Sync off: only the active photo is changed, so only it is looked up
    s.execute("develop.merge", &json!({"settings": {"optics": {"lens_profile": true}}})).unwrap();
    assert_eq!(crate::lens_db::to_look_up(&s, active), vec![active]);
    // Auto Sync on: every selected photo now has corrections on, and each is looked up once
    let mut s = Session::with_demo();
    s.execute("library.selectAll", &json!({})).unwrap();
    let active = s.selection.active.unwrap();
    s.auto_sync = true;
    s.execute("develop.merge", &json!({"settings": {"optics": {"lens_profile": true}}})).unwrap();
    assert_eq!(lp(&s), s.selection.ids.len());
    let ids = crate::lens_db::to_look_up(&s, active);
    assert_eq!(ids.len(), s.selection.ids.len());
    assert_eq!(ids.first(), Some(&active));
}

/// A lookup's results are dropped when the lens database was reloaded while the files were read (the probe may have
/// used the old one); the next frame looks the photos up again.
#[test]
fn a_database_reloaded_during_a_lookup_makes_its_results_stale() {
    let app = crate::LightcraftApp::new(Session::with_demo(), crate::Services { png: None, ..Default::default() });
    let key = crate::lens_db::lookup_key(&app);
    assert_eq!(crate::lens_db::lookup_key(&app), key);
    lightcraft_engine::lens_profiles::reload();
    assert_ne!(crate::lens_db::lookup_key(&app), key);
}
