//! Opening a library made by an earlier LightCraft, as Lightroom Classic upgrades a catalog: the original is never
//! changed. LightCraft makes a copy next to it, named with a `_V<format>` suffix (`Wedding Library_V10`), upgrades
//! the copy to this build's catalog format and opens that.
//!
//! [`Session::open_library`] refuses an older library with [`EngineError::LibraryNeedsUpgrade`] (nothing is read
//! beyond the catalog's header, nothing is written), which says in plain words what will happen; a frontend asks the
//! person and then calls [`Session::open_library_upgraded`] (agents: `app.openLibrary {path, upgrade: true}`, the
//! CLI's `--upgrade`). When the copy already exists it is opened instead of copying again.
//!
//! The copy is made in a hidden sibling folder (`.<name>_V10.partial`) and renamed into place only when every file
//! is there, so a failed copy (a full disk, no permission) leaves nothing that looks like a library; the partial
//! folder is removed and the error says the original is untouched. The photos a library keeps in its own
//! `Originals` folder are not copied: the catalog refers to them where they are.

use std::path::{Path, PathBuf};

use lightcraft_catalog::journal::{LOG, SNAPSHOT, VERSION};

use lightcraft_catalog::journal::LoadReport;

use crate::{EngineError, Result, Session};

/// The folders and files never copied: the lock (the copy is a new library), the photos kept in the library.
const SKIP: [&str; 3] = [lightcraft_catalog::lock::LOCK, lightcraft_catalog::lock::OWNER, "Originals"];
/// How deep a library's own folders go at most (thumbnail and preview caches are two or three levels).
const MAX_DEPTH: usize = 16;

/// What opening an older library takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upgrade {
    /// The catalog format the library is in (0: the earliest builds, a log without a snapshot).
    pub from: u32,
    /// Where the upgraded copy goes (or is): `<name>_V<format>` next to the library.
    pub copy: PathBuf,
    /// That copy exists already (from an earlier upgrade): it is opened instead of copying again.
    pub copy_exists: bool,
}

/// The catalog format a library folder's files are in, read from the head of its snapshot only: `None` when the
/// folder holds no catalog (a new library) or the head doesn't say (it is then left to the normal open, which
/// reports damage itself); `Some(0)` for a log without a snapshot.
pub fn stored_version(dir: &Path) -> Option<u32> {
    let snap = dir.join(SNAPSHOT);
    if !snap.is_file() {
        return dir.join(LOG).is_file().then_some(0);
    }
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(&snap).and_then(|mut f| f.read(&mut head)).ok()?;
    let head = std::str::from_utf8(head.get(..n)?).ok().or_else(|| std::str::from_utf8(head.get(..n.saturating_sub(3))?).ok())?;
    let at = head.find("\"version\":")?.saturating_add("\"version\":".len());
    let digits: String = head.get(at..)?.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The upgraded copy's place for the library at `dir`: `<name>_V<VERSION>` beside it.
pub fn copy_path(dir: &Path) -> Option<PathBuf> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    Some(dir.with_file_name(format!("{name}_V{VERSION}")))
}

/// What opening the library at `dir` takes: `None` when it opens as it is (this build's format, a newer one the
/// normal open refuses, or no catalog yet).
pub fn needed(dir: &Path) -> Option<Upgrade> {
    let from = stored_version(dir).filter(|v| *v < VERSION)?;
    let copy = copy_path(dir)?;
    let copy_exists = crate::library::is_library(&copy);
    Some(Upgrade { from, copy, copy_exists })
}

/// Copy the library at `from` into a new folder `to` (which must not exist), through a hidden partial folder renamed
/// at the end. On failure nothing is left at `to` and the partial folder is removed.
pub fn copy_library(from: &Path, to: &Path) -> std::result::Result<(), String> {
    if to.exists() {
        return Err(format!("{} already exists", to.display()));
    }
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("the copy has no name")?;
    let partial = to.with_file_name(format!(".{name}.partial"));
    // a partial copy left by a crash is never reused
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|e| format!("can't clear {}: {e}", partial.display()))?;
    }
    let copied =
        copy_dir(from, &partial, 0).and_then(|()| std::fs::rename(&partial, to).map_err(|e| format!("can't name the copy {}: {e}", to.display())));
    if copied.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    copied
}

fn copy_dir(from: &Path, to: &Path, depth: usize) -> std::result::Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{} is nested too deep to copy", from.display()));
    }
    std::fs::create_dir(to).map_err(|e| format!("can't create {}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("can't read {}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("can't read {}: {e}", from.display()))?;
        let name = entry.file_name();
        if depth == 0 && SKIP.iter().any(|s| name.as_os_str() == *s) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| format!("can't read {}: {e}", entry.path().display()))?;
        let target = to.join(&name);
        if kind.is_dir() {
            copy_dir(&entry.path(), &target, depth + 1)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|e| format!("can't copy {}: {e}", entry.path().display()))?;
        }
        // (links are not followed: a library's own files are plain)
    }
    Ok(())
}

impl Session {
    /// Open the library at `dir` made by an earlier LightCraft through its upgraded copy (see the module docs): the
    /// copy is made unless it exists, then opened (and upgraded) as this session's library. The original is never
    /// written. Returns the folder opened. A library that needs no upgrade is simply opened.
    pub fn open_library_upgraded(&mut self, dir: impl AsRef<Path>, seed_demo: bool) -> Result<(PathBuf, &LoadReport)> {
        let dir = crate::library::resolve_dir(dir.as_ref())?;
        let Some(up) = needed(&dir) else {
            self.open_library(&dir, seed_demo)?;
            return Ok((dir, self.loaded_library_report()?));
        };
        if !up.copy_exists {
            // another program writing to the original while it is copied would make a torn copy
            if dir.join(lightcraft_catalog::lock::LOCK).is_file() {
                drop(
                    lightcraft_catalog::lock::LibraryLock::acquire(&dir, "LightCraft (upgrade copy)")
                        .map_err(|e| EngineError::LibraryInUse(e.to_string()))?,
                );
            }
            copy_library(&dir, &up.copy).map_err(|e| {
                EngineError::Other(format!("Couldn't make the upgraded copy of the library: {e}. The original library is untouched."))
            })?;
            log::info!("library {}: format {} copied to {} for this LightCraft", dir.display(), up.from, up.copy.display());
        }
        self.open_library_checked(&up.copy, false, false)?;
        Ok((up.copy, self.loaded_library_report()?))
    }

    fn loaded_library_report(&self) -> Result<&LoadReport> {
        self.library.as_ref().map(|l| &l.report).ok_or_else(|| EngineError::Other("library closed while opening".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-upgrade-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A library with a photo and an edit, then rewritten as format 9 (what the previous builds wrote).
    fn format9_library(root: &Path) -> PathBuf {
        let lib = root.join("Wedding Library");
        let mut s = Session::new().with_fs();
        s.open_library(&lib, true).unwrap();
        let id = s.catalog.photos().next().unwrap().id;
        s.selection = crate::Selection { ids: vec![id], active: Some(id) };
        s.execute("develop.set", &json!({"values": {"light.exposure": 0.8}})).unwrap();
        s.close_library().unwrap();
        drop(s);
        // fold the log into the snapshot and mark it format 9 (the fields format 10 adds are all optional)
        let snap = std::fs::read_to_string(lib.join(SNAPSHOT)).unwrap();
        let head = format!("\"version\":{VERSION}");
        assert!(snap.contains(&head), "{}", &snap[..80.min(snap.len())]);
        std::fs::write(lib.join(SNAPSHOT), snap.replacen(&head, "\"version\":9", 1)).unwrap();
        lib
    }

    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = [SNAPSHOT, LOG, "prefs.json", "presets.json"]
            .iter()
            .filter_map(|n| std::fs::read(dir.join(n)).ok().map(|b| (n.to_string(), b)))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn an_older_library_opens_through_an_upgraded_copy_and_stays_untouched() {
        let root = scratch("copy");
        let lib = format9_library(&root);
        assert_eq!(stored_version(&lib), Some(9));
        let before = files(&lib);
        // the plain open refuses, in words, and writes nothing
        let mut s = Session::new().with_fs();
        let e = s.open_library(&lib, false).err().unwrap();
        assert_eq!(e.kind(), crate::ErrorKind::LibraryNeedsUpgrade);
        let text = e.to_string();
        assert!(text.contains("Wedding Library_V10") && text.contains("untouched") && !text.contains("v9"), "{text}");
        assert_eq!(files(&lib), before);
        // accepted: the copy is made, upgraded and opened
        let (opened, _) = s.open_library_upgraded(&lib, false).unwrap();
        assert_eq!(opened, root.join(format!("Wedding Library_V{VERSION}")));
        assert_eq!(s.catalog.len(), lightcraft_scenes::demo_library().len(), "the photos came along");
        assert!(s.catalog.photos().any(|p| p.develop.light.exposure == 0.8), "and the edits");
        s.close_library().unwrap();
        drop(s);
        assert_eq!(files(&lib), before, "the original's catalog files are byte for byte as they were");
        assert_eq!(stored_version(&opened), Some(VERSION), "the copy is in this build's format");
        assert!(!root.join(format!(".Wedding Library_V{VERSION}.partial")).exists());
        assert!(needed(&opened).is_none());
        // again: the existing copy is offered and opened, not copied a second time
        let up = needed(&lib).unwrap();
        assert!(up.copy_exists);
        std::fs::write(opened.join("marker.txt"), b"kept").unwrap();
        let mut s = Session::new().with_fs();
        let e = s.open_library(&lib, false).err().unwrap().to_string();
        assert!(e.contains("already"), "{e}");
        let (again, _) = s.open_library_upgraded(&lib, false).unwrap();
        assert_eq!(again, opened);
        assert!(opened.join("marker.txt").is_file(), "the existing copy was reused, not replaced");
        drop(s);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_copy_leaves_the_original_and_nothing_to_open() {
        let root = scratch("fail");
        let lib = format9_library(&root);
        let before = files(&lib);
        // a folder nested deeper than a library's own: the copy fails part-way
        let mut deep = lib.join("thumbs");
        for i in 0..(MAX_DEPTH + 2) {
            deep = deep.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        let mut s = Session::new().with_fs();
        let e = s.open_library_upgraded(&lib, false).err().unwrap().to_string();
        assert!(e.contains("untouched"), "{e}");
        assert!(s.library.is_none(), "nothing opened");
        let copy = copy_path(&lib).unwrap();
        assert!(!copy.exists() && !root.join(format!(".Wedding Library_V{VERSION}.partial")).exists(), "nothing half-written left");
        assert_eq!(files(&lib), before);
        drop(s);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn versions_are_read_from_the_head_only() {
        let root = scratch("head");
        assert_eq!(stored_version(&root), None, "no catalog: a new library");
        std::fs::write(root.join(LOG), b"").unwrap();
        assert_eq!(stored_version(&root), Some(0), "a log without a snapshot: the earliest builds");
        std::fs::write(root.join(SNAPSHOT), b"{\"format\":\"lightcraft-catalog\",\"version\": 7,\"seq\":0}").unwrap();
        assert_eq!(stored_version(&root), Some(7));
        std::fs::write(root.join(SNAPSHOT), b"garbage").unwrap();
        assert_eq!(stored_version(&root), None, "left to the normal open, which reports it");
        assert_eq!(copy_path(Path::new("/a/My Lib")), Some(PathBuf::from(format!("/a/My Lib_V{VERSION}"))));
        let _ = std::fs::remove_dir_all(&root);
    }
}
