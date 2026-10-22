//! File naming (client notes 35, 38): naming presets shared by Rename, Export and Import and kept
//! app-wide, `{text:Label}` typed at each use, and numbers that follow the order on screen.

use serde_json::json;

use crate::Session;

/// Note 38: Rename numbers the photos in the order the view shows them, not the order they were
/// selected in.
#[test]
fn rename_numbers_in_the_order_on_screen() {
    let mut s = Session::with_demo();
    s.execute("library.sort", &json!({"key": "fileName", "ascending": true})).unwrap();
    let vis = s.visible_cloned();
    let pick = [vis[4], vis[1], vis[3]];
    // selected back to front
    s.execute("library.select", &json!({"ids": pick.iter().map(|p| p.0).collect::<Vec<_>>()})).unwrap();
    let plans = s.execute("photo.renamePreview", &json!({"template": "Shot-{seq}"})).unwrap();
    let by_id: Vec<(u64, String)> =
        plans.as_array().unwrap().iter().map(|p| (p["id"].as_u64().unwrap(), p["to"].as_str().unwrap().to_string())).collect();
    let ids: Vec<u64> = by_id.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![vis[1].0, vis[3].0, vis[4].0], "numbered in view order");
    assert!(by_id[0].1.starts_with("Shot-1") && by_id[2].1.starts_with("Shot-3"), "{by_id:?}");
    // ids given: their order is kept
    let given = s.execute("photo.renamePreview", &json!({"template": "Shot-{seq}", "ids": [vis[4].0, vis[1].0]})).unwrap();
    assert_eq!(given[0]["id"], vis[4].0);
}

/// Note 35: `{text:…}` is asked each time (a rename without it is refused, saying what to type).
#[test]
fn rename_asks_for_the_text_tags() {
    let mut s = Session::with_demo();
    let first = s.visible_cloned()[0];
    s.execute("library.select", &json!({"ids": [first.0]})).unwrap();
    let t = "{seq:1}_{text:Couple initials}_Studio";
    let err = s.execute("photo.rename", &json!({"template": t})).unwrap_err().to_string();
    assert!(err.contains("Couple initials"), "{err}");
    let preview = s.execute("photo.renamePreview", &json!({"template": t, "texts": {"Couple initials": "R+A"}})).unwrap();
    assert!(preview[0]["to"].as_str().unwrap().starts_with("1_R+A_Studio"), "{preview}");
    let r = s.execute("photo.rename", &json!({"template": t, "texts": {"Couple initials": "R+A"}})).unwrap();
    assert_eq!(r["renamed"], 1);
    assert!(s.catalog.photo(first).unwrap().file_name.starts_with("1_R+A_Studio"));
}

/// Note 35: the export naming's texts are typed at export and never stored in an export preset.
#[test]
fn export_naming_takes_texts_and_presets_stay_fixed() {
    use crate::export::ExportOptions;
    let naming = "{seq:1}_{text:Couple initials}_Wedding";
    let err = ExportOptions::from_params(&json!({"naming": naming})).unwrap_err().to_string();
    assert!(err.contains("Couple initials"), "{err}");
    assert!(ExportOptions::from_params(&json!({"naming": naming, "texts": "R+A"})).is_err());
    let o = ExportOptions::from_params(&json!({"naming": naming, "texts": {"Couple initials": "R+A"}, "format": "jpeg"})).unwrap();
    let photo = crate::rename::sample_photo();
    assert_eq!(o.file_name_for(&photo, 1), "1_R+A_Wedding.jpg");
    let mut s = Session::with_demo();
    s.execute("export.savePreset", &json!({"name": "Delivery", "params": {"format": "jpeg", "naming": naming, "texts": {"Couple initials": "R+A"}}}))
        .unwrap();
    let p = s.export_presets.iter().find(|p| p.name == "Delivery").unwrap();
    assert!(p.params.get("texts").is_none(), "{}", p.params);
    assert_eq!(p.params["naming"], naming);
}

/// Note 35: naming presets are shared by every library and survive a restart; LightCraft's own
/// can't be replaced or deleted.
#[test]
fn naming_presets_are_kept_app_wide() {
    let root = std::env::temp_dir().join(format!("lc-naming-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (a, b, store) = (root.join("A"), root.join("B"), root.join("store"));
    let mut s = Session::new().with_app_store(&store);
    s.open_library(&a, true).unwrap();
    let list = s.execute("naming.savePreset", &json!({"name": "Studio delivery", "template": "{seq:1}_{text:Couple initials}_Studio"})).unwrap();
    assert!(list.as_array().unwrap().iter().any(|p| p["name"] == "Studio delivery" && p["builtin"] == false));
    assert!(s.execute("naming.savePreset", &json!({"name": "original NAME", "template": "{name}"})).is_err(), "a built-in name");
    assert!(s.execute("naming.savePreset", &json!({"name": "x", "template": "  "})).is_err());
    assert!(s.execute("naming.deletePreset", &json!({"name": "Original name"})).is_err());
    s.close_library().unwrap();
    s.open_library(&b, true).unwrap();
    assert!(s.naming_presets.iter().any(|p| p.name == "Studio delivery"), "in another library");
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new().with_app_store(&store);
    assert!(s.naming_presets.iter().any(|p| p.name == "Studio delivery"), "after a restart");
    s.execute("naming.deletePreset", &json!({"name": "studio DELIVERY"})).unwrap();
    assert!(s.naming_presets.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// Note 38: photos are put in order inside a collection (Lightroom's custom order); the view
/// follows it, Rename numbers by it, and Undo puts the old order back.
#[test]
fn collections_keep_a_custom_order_that_rename_follows() {
    let mut s = Session::with_demo();
    let vis = s.visible_cloned();
    let four: Vec<u64> = vis.iter().take(4).map(|p| p.0).collect();
    let a = s.execute("album.create", &json!({"name": "Family"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": a, "ids": four})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": a})).unwrap();
    s.execute("library.sort", &json!({"key": "custom", "ascending": true})).unwrap();
    let order = |s: &mut Session| s.visible_cloned().iter().map(|p| p.0).collect::<Vec<_>>();
    assert_eq!(order(&mut s), four);
    // the last two, picked back to front, go before the first
    s.execute("library.select", &json!({"ids": [four[3], four[2]]})).unwrap();
    let r = s.execute("album.movePhotos", &json!({"before": four[0]})).unwrap();
    assert_eq!(r["moved"], 2);
    assert_eq!(order(&mut s), vec![four[2], four[3], four[0], four[1]]);
    // to the end
    s.execute("album.movePhotos", &json!({"ids": [four[2]], "before": null})).unwrap();
    assert_eq!(order(&mut s), vec![four[3], four[0], four[1], four[2]]);
    // Rename numbers in that order
    s.execute("library.selectAll", &json!({})).unwrap();
    let plans = s.execute("photo.renamePreview", &json!({"template": "F-{seq}"})).unwrap();
    let ids: Vec<u64> = plans.as_array().unwrap().iter().map(|p| p["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, vec![four[3], four[0], four[1], four[2]]);
    // Undo, and hostile params
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(order(&mut s), vec![four[2], four[3], four[0], four[1]]);
    assert!(s.execute("album.movePhotos", &json!({"ids": [four[0]], "before": 999_999})).is_err());
    assert!(s.execute("album.movePhotos", &json!({"ids": [four[0]], "before": "x"})).is_err());
    s.execute("library.source", &json!({"kind": "all"})).unwrap();
    assert!(s.execute("album.movePhotos", &json!({"ids": [four[0]]})).is_err(), "no collection shown");
}
