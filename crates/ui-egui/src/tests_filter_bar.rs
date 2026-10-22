//! The bottom bar's quick filters (flags and colour labels) clicked on, off, cleared and on again:
//! each click changes the view, switching the last one off shows every photo again, and Clear
//! Filters leaves the bar working (notes item 18: Red on → Red off emptied the view, and after
//! View ▸ Clear Filters the buttons did nothing).

use std::time::Duration;

use lightcraft_catalog::{ColorLabel, Filter, Flag, Op, Photo, PhotoId, Source};
use serde_json::json;

use crate::headless::Headless;
use crate::state::ViewMode;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(120);
const N: u64 = 12;

/// `N` photos (missing files: no thumbnails to wait for): 1 red, 2 yellow, 3 picks, 2 rejects.
fn app(source: &str, view: ViewMode, width: f32, hold: bool) -> Headless {
    let mut s = lightcraft_engine::Session::new();
    for i in 1..=N {
        let path = format!("/lightcraft-filter-test/IMG_{i:03}.jpg");
        let mut p = Photo::new(PhotoId(i), Source::File { path }, &format!("IMG_{i:03}.jpg"), "JPEG", 600, 400, "2026-10-08T10:00:00");
        p.label = match i {
            1 => Some(ColorLabel::Red),
            2 | 3 => Some(ColorLabel::Yellow),
            _ => None,
        };
        p.flag = match i {
            4..=6 => Flag::Pick,
            7 | 8 => Flag::Reject,
            _ => Flag::None,
        };
        s.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    s.execute("library.source", &json!({"kind": source})).unwrap();
    if hold {
        s.execute("library.holdView", &json!({"on": true})).unwrap();
    }
    let mut h = Headless::new(LightcraftApp::new(s, Services { png: None, ..Default::default() }), [width, 900.0], 1.0);
    h.app.ui.view = view;
    // the bar's label filters give way to a right panel on a narrower window: room for all of them
    h.app.ui.right = crate::state::RightPanel::None;
    h.settle(SETTLE);
    h
}

fn rect(h: &Headless, id: &str) -> egui::Rect {
    match h.app.widgets.iter().find(|(w, _)| w == id) {
        Some((_, r)) => *r,
        None => panic!("{id} is on the bar: {:?}", h.app.widgets.iter().map(|(w, _)| w).collect::<Vec<_>>()),
    }
}

/// Click `id` where it is, and check it is still there afterwards: a button that moves under the
/// pointer while counts change sends the next click to its neighbour.
fn click(h: &mut Headless, id: &str) {
    let before = rect(h, id);
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
    let after = rect(h, id);
    assert!((before.left() - after.left()).abs() < 0.5 && (before.width() - after.width()).abs() < 0.5, "{id} moved from {before:?} to {after:?}");
}

fn shown(h: &mut Headless) -> usize {
    h.app.session.visible_cloned().len()
}

/// Every quick filter: on (the view narrows to it), off (every photo again, and no filter left
/// behind), Clear Filters, on again (it still narrows the view), off.
#[test]
fn every_bar_filter_toggles_on_off_clear_and_on_again() {
    let cases: [(&str, usize); 6] =
        [("filter:label-red", 1), ("filter:label-yellow", 2), ("filter:label-none", 9), ("filter:pick", 3), ("filter:reject", 2), ("filter:none", 7)];
    for (source, view, width, hold) in [
        ("all", ViewMode::PhotoGrid, 2200.0, false),
        ("recentlyAdded", ViewMode::PhotoGrid, 2200.0, false),
        ("all", ViewMode::Detail, 2200.0, false),
        // (the bar also holds the Auto Sync switch and "Photo n of N": the labels need a little more room)
        ("recentlyAdded", ViewMode::Detail, 1700.0, false),
        ("all", ViewMode::PhotoGrid, 2200.0, true),
        ("recentlyAdded", ViewMode::Detail, 2200.0, true),
    ] {
        let mut h = app(source, view, width, hold);
        let source = format!("{source} {view:?} {width} hold {hold}");
        let source = source.as_str();
        assert_eq!(shown(&mut h), N as usize, "{source}: every photo to start with");
        for (id, n) in cases {
            click(&mut h, id);
            assert_eq!(shown(&mut h), n, "{source}: {id} on");
            click(&mut h, id);
            assert_eq!(shown(&mut h), N as usize, "{source}: {id} off shows every photo again");
            assert_eq!(h.app.session.filter, Filter::default(), "{source}: {id} off leaves no filter behind");
            click(&mut h, id);
            assert_eq!(shown(&mut h), n, "{source}: {id} on again");
            let r = h.request("ui.menu.invoke", json!({"id": "library.clearFilter"}), T);
            assert_eq!(r["ok"], true, "{r}");
            h.settle(SETTLE);
            assert_eq!(shown(&mut h), N as usize, "{source}: Clear Filters after {id}");
            click(&mut h, id);
            assert_eq!(shown(&mut h), n, "{source}: {id} still works after Clear Filters");
            click(&mut h, id);
            assert_eq!(shown(&mut h), N as usize, "{source}: {id} off after Clear Filters");
        }
    }
}

/// On a bar too narrow for every part, a quick filter that is on stays on the bar (it may lose its
/// counts): the filter hiding photos is always where it can be seen and switched off.
#[test]
fn an_active_filter_never_gives_way() {
    for (patch, id) in [
        (json!({"labels": ["red"]}), "filter:label-red"),
        (json!({"unlabeled": true}), "filter:label-none"),
        (json!({"rating": 3}), "filter:stars"),
        (json!({"flags": ["reject"]}), "filter:reject"),
    ] {
        let mut h = app("all", ViewMode::Detail, 1500.0, false);
        h.app.ui.right = crate::state::RightPanel::Edit;
        h.settle(SETTLE);
        let r = h.request("engine.execute", json!({"command": "library.filter", "params": patch}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.settle(SETTLE);
        assert!(h.app.widgets.iter().any(|(w, _)| w == id), "{id} stays on a narrow bar while it filters");
        // the empty view offers the way out right there
        let r = h.request("engine.execute", json!({"command": "library.filter", "params": {"text": "no photo is called this"}}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.app.ui.view = ViewMode::PhotoGrid;
        h.settle(SETTLE);
        click_away(&mut h, "button:emptyClearFilters");
        assert_eq!(h.app.session.filter, Filter::default(), "the empty grid's button clears every filter");
        assert_eq!(shown(&mut h), N as usize);
    }
}

fn click_away(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}
