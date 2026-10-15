//! Delete Rejected Photos (`library.deleteRejected`): the rejects of the view leave the library in
//! one go, and on request their files go to the system's Recycle Bin (Trash), with the XMP
//! sidecars that are theirs alone.

use std::path::{Path, PathBuf};

use lightcraft_catalog::{Filter, Flag, Op, PhotoId, Source};

use crate::{EngineError, Result, Session};

/// Moves files to the Recycle Bin. The session's is the system's ([`system_bin`]); tests give it
/// one that keeps the files in a folder of their own.
pub type RecycleFn = fn(&[PathBuf]) -> std::result::Result<(), String>;

/// The system's Recycle Bin (Windows), Trash (macOS, the freedesktop.org trash on Linux and the
/// BSDs). The web build has none.
pub fn system_bin(paths: &[PathBuf]) -> std::result::Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if paths.is_empty() {
            return Ok(());
        }
        trash::delete_all(paths).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = paths;
        Err("there is no Recycle Bin in the browser".into())
    }
}

/// What to do with the rejects' files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Files {
    /// Leave them on disk: the photos go to Recently Deleted (one undo step).
    Keep,
    /// Move them (and their own sidecars) to the Recycle Bin, and forget the photos.
    Recycle,
}

impl Session {
    /// The rejected photos of the view: its source and filters, without the stars / flags / labels
    /// filter (a view of only the picks still has rejects to delete), stacks included.
    pub fn rejects_in_view(&mut self) -> Vec<PhotoId> {
        let base = self.filter.without_quick();
        let f = Filter { flag: Some(Flag::Reject), ..self.view_filter(self.source, &base) };
        self.catalog.query(&f, &self.sort)
    }

    /// Delete the rejects of the view (see [`Files`]). Returns (photos removed, files recycled).
    pub fn delete_rejected(&mut self, files: Files) -> Result<(usize, Vec<String>)> {
        let ids = self.rejects_in_view();
        if ids.is_empty() {
            return Ok((0, Vec::new()));
        }
        let before = self.visible_cloned();
        match files {
            Files::Keep => {
                let mut ops: Vec<Op> = ids.iter().map(|id| Op::SetDeleted { id: *id, deleted: true }).collect();
                ops.extend(self.catalog.remove_from_stacks_ops(&ids));
                self.commit("Delete Rejected Photos", Op::Batch { ops })?;
                self.select_after_removal(&before);
                Ok((ids.len(), Vec::new()))
            }
            Files::Recycle => {
                let originals: Vec<PathBuf> = ids
                    .iter()
                    .filter_map(|id| match self.catalog.photo(*id).map(|p| &p.source) {
                        Some(Source::File { path }) => Some(PathBuf::from(path)),
                        _ => None,
                    })
                    .filter(|p| p.exists())
                    .collect();
                let mut paths = originals.clone();
                paths.extend(own_sidecars(&originals, self.xmp.naming));
                (self.recycle)(&paths).map_err(|e| EngineError::Other(format!("could not move the files to the Recycle Bin: {e}")))?;
                let op = self.catalog.delete_photos_permanently_ops(&ids);
                self.commit("Delete Rejected Photos", op)?;
                self.select_after_removal(&before);
                Ok((ids.len(), paths.iter().map(|p| p.to_string_lossy().into_owned()).collect()))
            }
        }
    }

    /// After photos left the view: the selection keeps what is still shown, and the photo after
    /// the active one (else before it) becomes active.
    fn select_after_removal(&mut self, before: &[PhotoId]) {
        let at = before.iter().position(|id| Some(*id) == self.selection.active);
        let remaining: std::collections::HashSet<PhotoId> = self.visible_cloned().into_iter().collect();
        self.selection.ids.retain(|id| remaining.contains(id));
        self.selection.active = self.selection.active.filter(|id| remaining.contains(id)).or_else(|| self.selection.ids.first().copied());
        if self.selection.active.is_none()
            && let Some(i) = at
        {
            let next = before.iter().skip(i).chain(before.iter().take(i).rev()).find(|id| remaining.contains(id)).copied();
            if let Some(n) = next {
                self.selection = crate::Selection::single(n);
            }
        }
    }
}

/// The XMP sidecars of `originals` that belong to them alone: `IMG_1.CR3.xmp`, and `IMG_1.xmp`
/// unless another file named `IMG_1.*` stays beside it (a raw + JPEG pair shares it).
pub fn own_sidecars(originals: &[PathBuf], naming: crate::sidecar::SidecarNaming) -> Vec<PathBuf> {
    let going: std::collections::HashSet<&Path> = originals.iter().map(PathBuf::as_path).collect();
    let mut out: Vec<PathBuf> = Vec::new();
    for o in originals {
        let Some(s) = o.to_str().and_then(|s| crate::sidecar::find_sidecar(s, naming)) else { continue };
        let shared = s.file_stem() == o.file_stem() && others_with_stem(o, &going);
        if !shared && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Whether a file other than an XMP sidecar and other than those `going` shares `o`'s name stem
/// in its folder.
fn others_with_stem(o: &Path, going: &std::collections::HashSet<&Path>) -> bool {
    let (Some(dir), Some(stem)) = (o.parent(), o.file_stem()) else { return false };
    let Ok(entries) = std::fs::read_dir(dir) else { return true };
    entries.flatten().map(|e| e.path()).any(|p| {
        p.file_stem() == Some(stem)
            && !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
            && !going.contains(p.as_path())
            && p.as_path() != o
    })
}
