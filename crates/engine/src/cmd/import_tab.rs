//! The Import tab's commands: what an import of the browsed folder would take (picks, the
//! selection or all, with counts), import presets (named sets of import settings) and finishing an
//! import unattended (`library.finishImport`).

use lightcraft_catalog::{Flag, PhotoId, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{LibrarySource, Result, Session};

/// A named set of import settings: the `library.import` parameters it sets (mode, destination,
/// organize, rename, presets, keywords, cameraProfile, look, metadataPreset) plus `previews`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportPreset {
    pub name: String,
    pub settings: Value,
}

/// Which browsed photos an import takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    Picks,
    Selected,
    All,
}

impl Take {
    pub fn parse(s: &str) -> Option<Take> {
        match s {
            "picks" => Some(Take::Picks),
            "selected" | "selection" => Some(Take::Selected),
            "all" => Some(Take::All),
            _ => None,
        }
    }
}

/// The browsed photos not in the library yet, among those the folder view shows (in its order).
fn browsed_in_view(s: &mut Session) -> Vec<PhotoId> {
    if s.source != LibrarySource::Folder {
        return Vec::new();
    }
    let ids = s.visible().to_vec();
    ids.into_iter().filter(|id| s.catalog.photo(*id).is_some_and(|p| p.local && !p.deleted)).collect()
}

impl Session {
    /// The browsed photos an import would take: (picks, selected, all).
    pub fn import_targets(&mut self) -> (Vec<PhotoId>, Vec<PhotoId>, Vec<PhotoId>) {
        let all = browsed_in_view(self);
        let picks = all.iter().copied().filter(|id| self.catalog.photo(*id).is_some_and(|p| p.flag == Flag::Pick)).collect();
        let selected = all.iter().copied().filter(|id| self.selection.ids.contains(id)).collect();
        (picks, selected, all)
    }

    /// The files of `take` ([`Session::import_targets`]).
    pub fn import_paths(&mut self, take: Take) -> Vec<String> {
        let (picks, selected, all) = self.import_targets();
        let ids = match take {
            Take::Picks => picks,
            Take::Selected => selected,
            Take::All => all,
        };
        ids.iter()
            .filter_map(|id| match &self.catalog.photo(*id)?.source {
                Source::File { path } => Some(path.clone()),
                Source::Demo { .. } => None,
            })
            .collect()
    }
}

fn targets(s: &mut Session, p: &Value) -> Result<Value> {
    let (picks, selected, all) = s.import_targets();
    let mut out = json!({"picks": picks.len(), "selected": selected.len(), "all": all.len()});
    if let Some(w) = str_param(p, "what") {
        let take = Take::parse(w).ok_or_else(|| bad("import.targets", format!("unknown what `{w}` (picks|selected|all)")))?;
        out["paths"] = json!(s.import_paths(take));
    }
    Ok(out)
}

/// `library.import` for the browsed folder: the photos `what` names, with the given settings.
fn import_browsed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "import.run";
    let w = str_param(p, "what").unwrap_or("all");
    let take = Take::parse(w).ok_or_else(|| bad(C, format!("unknown what `{w}` (picks|selected|all)")))?;
    if s.source != LibrarySource::Folder {
        return Err(bad(C, "browse a folder first (library.browse)"));
    }
    let paths = s.import_paths(take);
    if paths.is_empty() {
        return Err(bad(
            C,
            match take {
                Take::Picks => "no picks to import: flag photos as picks (P) first",
                Take::Selected => "no browsed photos selected",
                Take::All => "nothing to import: every photo here is in the library already",
            },
        ));
    }
    let mut params = p.clone();
    if let Some(o) = params.as_object_mut() {
        o.remove("what");
        o.remove("previews");
    }
    params["paths"] = json!(paths);
    let mut r = s.execute("library.import", &params)?;
    let ids: Vec<u64> = ["imported", "restored"].iter().flat_map(|k| r[*k].as_array().into_iter().flatten().filter_map(Value::as_u64)).collect();
    let previews = str_param(p, "previews").unwrap_or("none").to_string();
    r["finishing"] = s.finish_import(crate::finishing::FinishPlan { ids, previews, started: String::new() });
    Ok(r)
}

fn save_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "import.savePreset";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "give the preset a name"))?.to_string();
    let settings = p.get("settings").cloned().filter(Value::is_object).ok_or_else(|| bad(C, "settings must be an object"))?;
    match s.import_presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
        Some(x) => x.settings = settings,
        None => s.import_presets.push(ImportPreset { name: name.clone(), settings }),
    }
    s.save_prefs()?;
    Ok(json!({"name": name, "presets": s.import_presets.len()}))
}

fn delete_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "import.deletePreset";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing name"))?;
    let n = s.import_presets.len();
    s.import_presets.retain(|x| !x.name.eq_ignore_ascii_case(name));
    if s.import_presets.len() == n {
        return Err(bad(C, format!("no import preset `{name}`")));
    }
    s.save_prefs()?;
    Ok(json!({"deleted": name}))
}

fn finish(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.finishImport";
    let previews = str_param(p, "previews").unwrap_or("none");
    if !matches!(previews, "none" | "standard" | "full" | "1:1") {
        return Err(bad(C, format!("unknown previews `{previews}` (none|standard|full)")));
    }
    let ids: Vec<u64> = if p.get("ids").is_some() || p.get("id").is_some() {
        s.targets(p)?.into_iter().map(|i| i.0).collect()
    } else {
        s.selection.ids.iter().map(|i| i.0).collect()
    };
    // (no threads to wait on in the browser: the app ticks it)
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false) && !cfg!(target_arch = "wasm32");
    let mut r = s.finish_import(crate::finishing::FinishPlan { ids, previews: previews.to_string(), started: String::new() });
    if wait {
        // (CLI, MCP, tests: tick until the queue is done)
        let until = web_time::Instant::now() + std::time::Duration::from_secs(p.get("timeout").and_then(Value::as_u64).unwrap_or(3600));
        while s.finishing.is_some() && web_time::Instant::now() < until {
            let _ = s.segment_poll();
            s.finishing_tick();
            if s.finishing.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        r = s.finishing.as_ref().map_or_else(|| json!({"running": false, "note": s.finishing_note}), crate::finishing::Finishing::json);
    }
    Ok(r)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "import.targets",
            "Import Targets",
            [],
            None,
            "{what?: picks|selected|all} — the browsed (not yet imported) photos of the folder view an import would take, counted: {picks, selected, all, paths? (of `what`)}",
            always,
            targets
        ),
        cmd!(
            "import.run",
            "Import",
            [],
            None,
            "{what?: picks|selected|all (default all), previews?: none|standard|full, …library.import settings (mode, destination, organize, rename, presets, keywords, cameraProfile, look, metadataPreset…)} — import the browsed folder's picks, selection or all its photos not in the library yet (their culling and edits go along), then finish them unattended (library.finishImport) → the library.import report + {finishing}",
            always,
            import_browsed
        ),
        cmd!(query "import.presets", "Import Presets", [], None, "{} → [{name, settings}]", always, |s, _| Ok(json!(s.import_presets))),
        cmd!(
            "import.savePreset",
            "Save Import Preset",
            [],
            None,
            "{name, settings: {mode, destination, organize, rename, presets, keywords, cameraProfile, look, metadataPreset, previews…}} — keep the Import tab's settings under a name (replaces one of that name; saved with the library)",
            always,
            save_preset
        ),
        cmd!("import.deletePreset", "Delete Import Preset", [], None, "{name}", always, delete_preset),
        cmd!(
            "library.finishImport",
            "Finish Import",
            [],
            None,
            "{ids? (default: the selection), previews?: none|standard|full, wait?: bool, timeout?: s} — get photos ready unattended: AI masks of their presets, Smooth Skin's skin, then previews; one activity row with progress and time left; kept in the library and resumed after a restart; ends with a note (library.finishProgress) → {running, stage, total, progress, eta}",
            always,
            finish
        ),
        cmd!(query "library.finishProgress", "Finish Import Progress", [], None, "{} → {running, stage: masks|previews|done, total, analysing, previewsDone, progress: 0..1, eta: s} | {running: false, note}", always, |s, _| {
            Ok(s.finishing.as_ref().map_or_else(|| json!({"running": false, "note": s.finishing_note, "pending": s.finish_pending.is_some()}), crate::finishing::Finishing::json))
        }),
    ]
}
