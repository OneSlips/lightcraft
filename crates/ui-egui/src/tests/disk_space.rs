//! The low-disk notice: once per session, when the drive holding the preview cache runs low, with a
//! button to Settings ▸ Performance (where the cache's size limit is).

use std::time::Duration;

use lightcraft_engine::disk_space::DiskSpace;
use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const GB: u64 = 1 << 30;

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

#[test]
fn a_low_drive_is_told_once_with_a_way_to_the_performance_settings() {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
    app.disk_space_notice = true;
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    h.step();
    assert!(h.app.notices.is_empty(), "no notice before the drive is low");
    // 12 GB free of a 500 GB drive: under 20 GB and under 10 %
    assert!(h.app.session.note_disk_space(DiskSpace { available: 12 * GB, total: 500 * GB }));
    h.step();
    h.step();
    assert_eq!(h.app.notices.len(), 1, "{:?}", h.app.notices);
    let text = h.app.notices[0].clone();
    assert!(text.contains("12.0 GB free of 500 GB") && text.contains("Settings → Performance"), "{text}");
    let inspect = h.request("ui.inspect", json!({}), T);
    assert_eq!(inspect["result"]["notices"][0], text.as_str(), "{inspect}");
    assert!(has(&h, "button:noticeSettings") && has(&h, "button:noticeOk"), "the notice offers the settings");
    let r = h.request("ui.clickWidget", json!({"id": "button:noticeSettings"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.app.notices.is_empty(), "the button dismisses the notice");
    assert_eq!(h.app.ui.dialog, Some(Dialog::Settings { tab: "performance".into() }), "and opens Settings ▸ Performance");
    // lower still, later in the same session: not again
    assert!(!h.app.session.note_disk_space(DiskSpace { available: 2 * GB, total: 500 * GB }));
    h.step();
    h.step();
    assert!(h.app.notices.is_empty(), "once per session");
}

#[test]
fn without_the_desktop_flag_no_notice_is_shown() {
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default()), [1300.0, 900.0], 1.0);
    assert!(h.app.session.note_disk_space(DiskSpace { available: GB, total: 500 * GB }));
    h.step();
    assert!(h.app.notices.is_empty(), "headless sessions and tests don't show it");
}
