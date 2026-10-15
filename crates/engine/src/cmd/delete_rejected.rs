//! The one way to delete files from disk (DECIDED by the user: no delete in Import):
//! `import.deleteRejectedFiles` moves the files of the browsed folder's rejected photos (and
//! their XMP sidecars) to the Recycle Bin, after a confirmation that repeats how many.

use std::path::Path;

use lightcraft_catalog::{Flag, Op, PhotoId, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd};
use crate::{LibrarySource, Result, Session};

impl Session {
    /// The rejected browsed (not imported) photos of the folder view: what Delete Rejected Files
    /// from Disk would remove.
    pub fn rejected_browsed(&mut self) -> Vec<PhotoId> {
        if self.source != LibrarySource::Folder {
            return Vec::new();
        }
        let ids = self.visible().to_vec();
        ids.into_iter().filter(|id| self.catalog.photo(*id).is_some_and(|p| p.local && !p.deleted && p.flag == Flag::Reject)).collect()
    }
}

/// Move `path` to the Recycle Bin (Trash). Never deletes outright: where there is no bin to move
/// it to, it fails and the file stays.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn to_recycle_bin(paths: &[String]) -> std::result::Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        // Windows' own Recycle Bin, through the shell (one process for all the files; paths on stdin)
        use std::io::Write;
        let script = "Add-Type -AssemblyName Microsoft.VisualBasic; $failed = 0; foreach ($p in [Console]::In.ReadToEnd().Split(\"`n\")) { $p = $p.Trim(); if ($p) { try { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } catch { $failed++; [Console]::Error.WriteLine($p + ': ' + $_.Exception.Message) } } }; exit $failed";
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not reach the Recycle Bin: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(paths.join("\n").as_bytes()).map_err(|e| format!("could not reach the Recycle Bin: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("could not reach the Recycle Bin: {e}"))?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }
    #[cfg(not(windows))]
    {
        // the freedesktop trash (gio), else the macOS Trash in the home folder
        let gio = std::process::Command::new("gio").arg("trash").args(paths).output();
        if gio.as_ref().is_ok_and(|o| o.status.success()) {
            return Ok(());
        }
        let home = std::env::var("HOME").map_err(|_| "no Trash to move the files to".to_string())?;
        let trash = Path::new(&home).join(".Trash");
        if !trash.is_dir() {
            return Err(gio.map_or_else(|e| e.to_string(), |o| String::from_utf8_lossy(&o.stderr).trim().to_string()));
        }
        for p in paths {
            let name = Path::new(p).file_name().ok_or_else(|| format!("{p}: not a file"))?;
            let mut dst = trash.join(name);
            let mut n = 1;
            while dst.exists() && n < 1000 {
                dst = trash.join(format!("{} {n}", name.to_string_lossy()));
                n += 1;
            }
            std::fs::rename(p, &dst).map_err(|e| format!("{p}: {e}"))?;
        }
        Ok(())
    }
}

/// Delete Rejected Files from Disk: the rejected browsed photos of the folder view (and their
/// XMP sidecars) go to the Recycle Bin, and their records leave LightCraft. Refused unless
/// `confirm` repeats how many files that is (the dialog has the user type it).
fn delete_rejected(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "import.deleteRejectedFiles";
    let ids = s.rejected_browsed();
    let n = ids.len();
    let confirm = p.get("confirm").and_then(Value::as_u64);
    if confirm != Some(n as u64) {
        return Err(bad(
            C,
            format!("{n} rejected file{} would go to the Recycle Bin: repeat the count as `confirm` to go ahead", if n == 1 { "" } else { "s" }),
        ));
    }
    if n == 0 {
        return Ok(json!({"deleted": 0}));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ids;
        Err(bad(C, "files can't be deleted from the browser"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let naming = s.xmp.naming;
        let mut files: Vec<(PhotoId, Vec<String>)> = Vec::new();
        for id in &ids {
            let Some(Source::File { path }) = s.catalog.photo(*id).map(|p| p.source.clone()) else { continue };
            if !Path::new(&path).is_file() {
                continue;
            }
            let mut all = vec![path.clone()];
            if let Some(sc) = crate::sidecar::find_sidecar(&path, naming) {
                // a sidecar shared with another photo of the same name (a raw + JPEG pair) stays
                let stem = |q: &str| Path::new(q).with_extension("").to_string_lossy().to_lowercase();
                let shared = s
                    .catalog
                    .photos()
                    .any(|o| o.id != *id && !ids.contains(&o.id) && matches!(&o.source, Source::File { path: op } if stem(op) == stem(&path)));
                if !shared {
                    all.push(sc.to_string_lossy().to_string());
                }
            }
            files.push((*id, all));
        }
        let mut gone: Vec<PhotoId> = Vec::new();
        let mut failed: Vec<Value> = Vec::new();
        // a batch at a time: one failure doesn't hold back the rest
        for chunk in files.chunks(50) {
            let paths: Vec<String> = chunk.iter().flat_map(|(_, f)| f.iter().cloned()).collect();
            match to_recycle_bin(&paths) {
                Ok(()) => gone.extend(chunk.iter().map(|(id, _)| *id)),
                Err(e) => {
                    // which made it: those whose file is no longer there
                    for (id, f) in chunk {
                        if f.first().is_some_and(|x| !Path::new(x).exists()) {
                            gone.push(*id);
                        } else {
                            failed.push(json!({"path": f.first(), "error": e}));
                        }
                    }
                }
            }
        }
        if !gone.is_empty() {
            // the records go too (not an undo step: the files are in the Recycle Bin, not in LightCraft)
            let op = Op::Batch { ops: gone.iter().map(|id| Op::RemovePhoto { id: *id }).collect() };
            s.catalog.apply(op.clone()).map_err(|e| bad(C, e.to_string()))?;
            s.pending_log.push(op);
            s.selection.ids.retain(|id| !gone.contains(id));
            if s.selection.active.is_some_and(|a| gone.contains(&a)) {
                s.selection.active = s.selection.ids.last().copied();
            }
            s.undo.retain(|e| {
                let mut touches = false;
                e.op.photo_ids(&mut |id| touches |= gone.contains(&id));
                !touches
            });
            s.redo.clear();
            s.refresh_view();
        }
        log::info!("{C}: {} file(s) moved to the Recycle Bin, {} failed", gone.len(), failed.len());
        Ok(json!({"deleted": gone.len(), "failed": failed}))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "import.deleteRejectedFiles",
        "Delete Rejected Files from Disk…",
        [],
        None,
        "{confirm: n (must equal the number of rejected browsed files)} — Import only: the rejected photos of the browsed folder, and their XMP sidecars, are moved to the Recycle Bin (Trash) and leave LightCraft; never library photos; not undoable in LightCraft (restore them from the Recycle Bin) → {deleted, failed: [{path, error}]}",
        always,
        delete_rejected
    )]
}
