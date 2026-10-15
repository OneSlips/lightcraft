//! The app-wide store of presets, profiles and effects ([`crate::app_store`]): where they live,
//! merging libraries into it, and keeping them with one library instead.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd};
use crate::app_store::{LibraryContent, LibraryShared, PresetsFile};
use crate::{Result, Session};

fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "store.mergeLibraries";
    let paths: Vec<PathBuf> = p
        .get("paths")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter(|x| !x.trim().is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default();
    let force = p.get("force").and_then(Value::as_bool) == Some(true);
    if s.app_store.is_none() {
        return Err(bad(C, "there is no shared store: LightCraft's settings folder isn't available"));
    }
    if !s.uses_app_store() {
        // nothing to do while the open library keeps its own: not an error at start-up
        return Ok(json!({"merged": [], "libraryKeepsOwn": true}));
    }
    let reports = s.merge_libraries(&paths, force)?;
    Ok(json!({"merged": reports}))
}

/// The open library's photos name profiles by id: copy the profile files the session uses into the
/// library's `Profiles/` so it is complete on its own.
fn copy_profiles_into(s: &mut Session, dir: &Path) -> Vec<String> {
    let to_dir = dir.join("Profiles");
    let mut problems = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&to_dir) {
        problems.push(format!("{}: {e}", to_dir.display()));
        return problems;
    }
    let mut copy = |file: &mut String| {
        let from = PathBuf::from(file.as_str());
        if from.starts_with(&to_dir) {
            return;
        }
        let Some(name) = from.file_name() else { return };
        let to = to_dir.join(name);
        if to.exists() {
            if std::fs::read(&to).ok() == std::fs::read(&from).ok() {
                *file = to.to_string_lossy().to_string();
            }
            // a different file of that name: the entry keeps pointing at the store's copy
            return;
        }
        match std::fs::copy(&from, &to) {
            Ok(_) => *file = to.to_string_lossy().to_string(),
            Err(e) => problems.push(format!("{}: {e}", from.display())),
        }
    };
    for e in &mut s.dcp_profiles {
        copy(&mut e.file);
    }
    for l in &mut s.lut_profiles {
        copy(&mut l.file);
    }
    problems
}

fn use_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "store.useLibrary";
    let keep = p.get("keep").and_then(Value::as_bool).ok_or_else(|| bad(C, "give {keep: true | false}"))?;
    let Some(lib) = s.library.as_ref() else { return Err(bad(C, "no library is open")) };
    if !lib.on_disk {
        return Err(bad(C, "the library isn't on disk"));
    }
    let dir = lib.dir.clone();
    if s.app_store.is_none() {
        return Err(bad(C, "there is no shared store: LightCraft's settings folder isn't available"));
    }
    let problems;
    if keep {
        if lib.own_store {
            return Ok(json!({"keep": true, "changed": false}));
        }
        // the library takes a copy of what the store holds now
        problems = copy_profiles_into(s, &dir);
        if let Some(lib) = s.library.as_mut() {
            lib.own_store = true;
            lib.store_with_library = true;
            lib.reset_presets_saved();
        }
        if let Some(st) = s.app_store.as_mut() {
            st.in_session = false;
        }
        s.save_prefs()?;
        s.persist()?;
    } else {
        if !lib.own_store || !lib.store_with_library {
            return Ok(json!({"keep": false, "changed": false}));
        }
        // the library's own join the store
        let content = LibraryContent {
            shared: LibraryShared {
                lut_profiles: s.lut_profiles.clone(),
                dcp_profiles: s.dcp_profiles.clone(),
                mask_effects: s.mask_effects.clone(),
                mask_tool_effects: s.mask_tool_effects.clone(),
                import: s.import_defaults.clone(),
                profiles: s.profile_state.clone(),
                store_with_library: false,
            },
            presets: Some(PresetsFile {
                user: s.presets.iter().filter(|p| !p.builtin).cloned().collect(),
                favorites: s.presets.iter().filter(|p| p.builtin && p.favorite).map(|p| p.id.clone()).collect(),
                profile_favorites: s.profile_favorites.clone(),
                profile_recent: s.profile_recent.clone(),
            }),
        };
        let auto = (s.import_defaults.auto_folder.clone(), s.import_defaults.auto_copy, s.import_defaults.auto_album.clone());
        // what the library holds is written once more, then kept as it is
        s.save_prefs()?;
        s.persist()?;
        let frozen = s.library.as_mut().map(|l| l.freeze_shared()).unwrap_or_default();
        if let Some(lib) = s.library.as_mut() {
            lib.own_store = false;
            lib.store_with_library = false;
            lib.set_frozen(frozen);
        }
        s.load_app_store();
        s.import_defaults.auto_folder = auto.0;
        s.import_defaults.auto_copy = auto.1;
        s.import_defaults.auto_album = auto.2;
        let r = s.merge_library_content(&dir, content);
        problems = r.problems;
        s.apply_store_remap();
        crate::profile_refs::refresh(s);
        s.save_prefs()?;
        s.save_shared_presets()?;
    }
    s.refresh_view();
    Ok(json!({"keep": keep, "changed": true, "problems": problems}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "store.info",
            "Shared Presets and Profiles",
            [],
            None,
            "{} → {available, dir, inUse (the open library uses the shared store), libraryKeepsOwn, presets, cameraProfiles, lutProfiles, effects, merged: [{library, at, presets, profiles, effects}]}: the store of presets, profiles, local adjustment presets, import defaults and camera body defaults that every library shares",
            always,
            |s, _| Ok(s.store_info())
        ),
        cmd!(
            "store.mergeLibraries",
            "Add Libraries to Shared Presets and Profiles",
            [],
            None,
            "{paths: [library folders], force?: bool} — each library's presets, profiles (files copied), local adjustment presets and body defaults join the shared store, once per library unless `force` (the same item is kept once); the libraries are only read → {merged: [{library, presets, profiles, effects, problems, skipped}]}",
            always,
            merge
        ),
        cmd!(
            "store.useLibrary",
            "Store Presets and Profiles with This Library",
            [],
            None,
            "{keep: bool} — true: the open library keeps its own presets, profiles and effects (it takes a copy of the shared ones, profile files included); false: it uses the shared store again (its own join it) → {keep, changed, problems}",
            always,
            use_library
        ),
    ]
}
