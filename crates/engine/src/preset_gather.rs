//! Export presets (and the other named presets the app store holds: print templates, filter,
//! metadata, curve, keyword, label and naming presets) gathered from every library the user has,
//! never dropped (test notes 2026-10-24 note 29). The move into the app store (note 52) merged a
//! library once, when it was open; a library merged by an older build was only merged again for
//! these presets when it was opened again, and one whose own `prefs.json` was gone (restored from a
//! backup, rebuilt) never was, so its presets went missing.
//!
//! Gathering reads, once per library folder: its `prefs.json`, and in each of its backups
//! (`Backups/<time>/` in the library, or `<backup folder>/<library name>/<time>/`) the library's
//! `prefs.json` and the shared settings' `Library Store/shared.json`. The same preset is kept once
//! (by name and content); the same name with other content joins as "name 2". Nothing in the
//! libraries or their backups is written. The libraries come from the caller (the app's Recent
//! Libraries and the open one) and from every library the store merged before.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::app_store::{Habits, same_path};
use crate::{EngineError, Result, Session};

/// Largest settings file read.
const MAX_FILE: u64 = 64 << 20;
/// Most backups of one library looked into (newest first).
const MAX_BACKUPS: usize = 200;
/// Most libraries gathered from in one call.
const MAX_LIBRARIES: usize = 500;

/// What gathering found.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatherReport {
    /// Library folders looked into.
    pub libraries: usize,
    /// Settings files read.
    pub files: usize,
    /// Presets added, of every kind.
    pub count: usize,
    /// The export presets added (their names as they are in the store now).
    pub added: Vec<String>,
    /// Libraries gathered before (skipped).
    pub skipped: usize,
    /// What couldn't be read.
    pub problems: Vec<String>,
}

/// A settings file's named presets (`None`: no file there).
fn read_habits(path: &Path) -> std::result::Result<Option<Habits>, String> {
    let bytes = match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE => return Err(format!("{}: {} bytes is too large", path.display(), m.len())),
        Ok(_) => std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

/// The backups in `root` (`<time>/library`), newest first.
fn backups_in(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("library").is_dir()).collect();
    dirs.sort();
    dirs.reverse();
    dirs.truncate(MAX_BACKUPS);
    dirs
}

/// The settings files of library `dir` that can hold presets: its own, then its backups' (in the
/// library's `Backups`, and in `backup_root/<library name>` when backups go elsewhere).
pub(crate) fn sources(dir: &Path, backup_root: Option<&Path>) -> Vec<PathBuf> {
    let mut out = vec![dir.join("prefs.json")];
    let mut roots = vec![dir.join("Backups")];
    if let (Some(root), Some(name)) = (backup_root, dir.file_name()) {
        roots.push(root.join(name));
    }
    for root in roots {
        for b in backups_in(&root) {
            out.push(b.join("library").join("prefs.json"));
            out.push(b.join("settings").join(crate::app_store::DIR_NAME).join(crate::app_store::SHARED));
        }
    }
    out
}

impl Session {
    /// Gather the named presets of the library folders `dirs`, the open library and every library
    /// the store merged before, from their settings and backups ([`self`] module docs), once per
    /// library unless `force`. Saves the store. → what was found.
    pub fn gather_presets(&mut self, dirs: &[PathBuf], force: bool) -> Result<GatherReport> {
        if self.app_store.is_none() {
            return Err(EngineError::Other("there is no shared store (LightCraft's settings folder isn't available)".into()));
        }
        if !self.uses_app_store() {
            return Err(EngineError::Other("the open library keeps its own presets and profiles; switch it to the shared store first".into()));
        }
        self.load_app_store();
        let mut libraries: Vec<PathBuf> = Vec::new();
        let open = self.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone());
        let merged: Vec<PathBuf> = self.app_store.as_ref().map(|s| s.merged.iter().map(|m| PathBuf::from(&m.library)).collect()).unwrap_or_default();
        for d in dirs.iter().cloned().chain(open).chain(merged) {
            if libraries.len() >= MAX_LIBRARIES {
                break;
            }
            if !d.as_os_str().is_empty() && !libraries.iter().any(|l| same_path(l, &d)) {
                libraries.push(d);
            }
        }
        let backup_root = self.backups.settings.dir.as_deref().filter(|d| !d.trim().is_empty()).map(PathBuf::from);
        let mut report = GatherReport::default();
        for dir in libraries {
            let done = self.app_store.as_ref().is_some_and(|s| s.gathered.iter().any(|g| same_path(Path::new(g), &dir)));
            if done && !force {
                report.skipped += 1;
                continue;
            }
            // a drive that isn't there now (a disconnected disk): asked again next time
            if !dir.is_dir() {
                continue;
            }
            report.libraries += 1;
            let mut mine = self.habits();
            for f in sources(&dir, backup_root.as_deref()) {
                match read_habits(&f) {
                    Ok(Some(mut theirs)) => {
                        report.files += 1;
                        let before: Vec<String> = mine.export_presets.iter().map(|p| p.name.clone()).collect();
                        let n = mine.merge_lists(&mut theirs);
                        report.count += n;
                        if n > 0 {
                            report.added.extend(mine.export_presets.iter().map(|p| p.name.clone()).filter(|n| !before.contains(n)));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => report.problems.push(e),
                }
            }
            // (only the named presets change: the session's single settings stay as they are)
            let h = self.habits();
            let changed = mine != h;
            if changed {
                self.set_habits(Habits { sort: h.sort, ..mine });
            }
            if let Some(st) = self.app_store.as_mut() {
                st.gathered.retain(|g| !same_path(Path::new(g), &dir));
                st.gathered.push(dir.display().to_string());
            }
        }
        for p in &report.problems {
            log::warn!("store: gathering presets: {p}");
        }
        self.save_app_store()?;
        Ok(report)
    }
}
