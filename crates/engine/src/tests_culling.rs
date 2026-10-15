//! The culling filters and counts (the bottom bar): stars, flags and colour labels.

use serde_json::json;

use crate::Session;

fn demo() -> Session {
    Session::with_demo()
}

/// The quick culling filters: unrated, any of several flags, labels with "no label"; the tally
/// counts the view without them.
#[test]
fn quick_filters_and_the_tally() {
    let mut s = demo();
    let ids: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    let total = ids.len();
    assert!(total >= 6);
    s.execute("photo.flag", &json!({"ids": [ids[0], ids[1]], "flag": "pick"})).unwrap();
    s.execute("photo.flag", &json!({"ids": [ids[2]], "flag": "reject"})).unwrap();
    s.execute("photo.rate", &json!({"ids": [ids[0]], "rating": 3})).unwrap();
    s.execute("photo.rate", &json!({"ids": [ids[3]], "rating": 5})).unwrap();
    s.execute("photo.label", &json!({"ids": [ids[4]], "label": "green"})).unwrap();
    let base = s.tally();
    let t = s.execute("library.tally", &json!({})).unwrap();
    assert_eq!(t["picks"], json!(base.picks));
    s.execute("library.filter", &json!({"flags": ["pick", "none"]})).unwrap();
    assert_eq!(s.visible_cloned().len(), total - base.rejects);
    assert_eq!(s.tally(), base, "the counts are of the view without the quick filters");
    s.execute("library.clearFilter", &json!({})).unwrap();
    s.execute("library.filter", &json!({"unrated": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.stars[0]);
    s.execute("library.filter", &json!({"rating": 3, "ratingOp": "atLeast"})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.stars[0] + base.at_least(3), "unrated or three stars and more");
    s.execute("library.clearFilter", &json!({})).unwrap();
    s.execute("library.filter", &json!({"labels": ["green"], "unlabeled": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.label(lightcraft_catalog::ColorLabel::Green) + base.unlabeled);
    s.execute("library.filter", &json!({"labels": [], "unlabeled": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), base.unlabeled);
    let chips = crate::filter_chips(&s.filter, &s.catalog);
    assert!(chips.iter().any(|c| c.label.contains("no label")), "{chips:?}");
}
