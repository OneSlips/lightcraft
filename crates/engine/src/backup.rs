//! Library backups on the desktop: a copy of the catalog, the library's settings and LightCraft's
//! own settings (the app store of presets and profiles included), never photos or caches.
//!
//! ```text
//! <destination>/<library name>/2026-10-16 183012/
//!   library/    catalog.snap + empty catalog.log (written from memory: always consistent),
//!               presets.json, prefs.json, view.json, people.json, …, Profiles/
//!   settings/   ui.json, control-surfaces.json, backup.json, Library Store/ (shared presets,
//!               profiles), camera-profiles/
//!   backup.json (what this is: library, time, files)
//! ```
//!
//! Plain folders: a backup can be opened as a library as it is (File ▸ Open Library), copied or
//! looked into. Restoring copies `library/` to a new folder beside the current library and opens it;
//! nothing is overwritten. The settings (`backup.json` in the settings folder) say when to back up:
//! every time LightCraft exits, once a day, once a week, or never; where to; and how many to keep.
//! API keys and downloaded models are never copied.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::{CommandSpec, always, bad, cmd};
use crate::{EngineError, Result, Session};

/// The manifest inside every backup.
pub const MANIFEST: &str = "backup.json";
/// The backup settings file in the settings folder.
pub const SETTINGS_FILE: &str = "backup.json";
/// Library files never copied (the catalog is written from memory; the lock).
const SKIP_LIBRARY: [&str; 3] = ["catalog.lock", "catalog.snap", "catalog.log"];
/// The library's folders that are copied (everything else in it, caches, photos copied into the
/// library or folders the user keeps there, is not).
const LIBRARY_DIRS: [&str; 1] = ["Profiles"];
/// Settings-folder entries copied (others are models, logs, downloaded data and API keys).
const SETTINGS_ENTRIES: [&str; 5] = ["ui.json", "control-surfaces.json", SETTINGS_FILE, crate::app_store::DIR_NAME, "camera-profiles"];
/// Largest single file copied (a settings file this big isn't one).
const MAX_FILE: u64 = 2 << 30;
/// Most files copied from one folder tree.
const MAX_FILES: usize = 20_000;

/// When to back up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Frequency {
    /// Every time LightCraft exits.
    EveryExit,
    /// When LightCraft exits, once a day.
    #[default]
    Daily,
    /// When LightCraft exits, once a week.
    Weekly,
    /// Only when asked (Back Up Library Now).
    Never,
}

impl Frequency {
    pub fn parse(s: &str) -> Option<Frequency> {
        Some(match s {
            "everyExit" | "always" => Frequency::EveryExit,
            "daily" => Frequency::Daily,
            "weekly" => Frequency::Weekly,
            "never" => Frequency::Never,
            _ => return None,
        })
    }
    fn every(self) -> Option<u64> {
        match self {
            Frequency::EveryExit => Some(0),
            // a little under a day / week: the same time tomorrow counts
            Frequency::Daily => Some(20 * 3600),
            Frequency::Weekly => Some(6 * 86_400 + 20 * 3600),
            Frequency::Never => None,
        }
    }
}

/// The backup settings (app-wide).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackupSettings {
    pub frequency: Frequency,
    /// Destination folder (`None`: a `Backups` folder in the library).
    pub dir: Option<String>,
    /// Backups kept per library (older ones are deleted; 0 = keep all).
    pub keep: u32,
    /// Check the backup by opening its catalog before older ones are pruned.
    pub verify: bool,
    /// Last backup per library folder (Unix seconds).
    pub last: std::collections::BTreeMap<String, u64>,
    /// What the library and the shared settings were at its last backup, per library folder
    /// ([`Session::backup_mark`]): a close with nothing changed since needs no backup.
    pub marks: std::collections::BTreeMap<String, String>,
}

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings { frequency: Frequency::Daily, dir: None, keep: 10, verify: true, last: Default::default(), marks: Default::default() }
    }
}

/// Backups as the session knows them: settings and where they are saved.
#[derive(Default)]
pub struct Backups {
    pub settings: BackupSettings,
    /// The settings file (`None`: not saved; backups need a host that turns them on).
    pub file: Option<PathBuf>,
    /// The settings folder copied into each backup.
    pub settings_dir: Option<PathBuf>,
    /// What the last backup did (shown in Settings).
    pub last_result: Option<Value>,
    /// A backup running in the background: its result once done.
    running: Option<Slot>,
    /// The mark of the backup being made (library folder, mark), kept once it succeeds.
    pending_mark: Option<(String, String)>,
}

fn now_secs() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `secs` as `YYYY-MM-DD HHMMSS UTC`, for folder names (sorting by name sorts by time).
fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Copy `from` (a file or folder) to `to`; folders recursively, at most [`MAX_FILES`] files.
fn copy_tree(from: &Path, to: &Path, skip: &[&str], depth: u32, count: &mut usize, bytes: &mut u64) -> std::io::Result<()> {
    if depth > 16 {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        if meta.len() > MAX_FILE || *count >= MAX_FILES {
            return Ok(());
        }
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        *bytes = bytes.saturating_add(std::fs::copy(from, to)?);
        *count += 1;
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if skip.contains(&name.as_str()) || name.contains(".corrupt-") || name.starts_with(".write-test") || name.ends_with(".tmp") {
            continue;
        }
        copy_tree(&e.path(), &to.join(&name), &[], depth + 1, count, bytes)?;
    }
    Ok(())
}

/// The backups in `dir` (folders with a manifest), newest first.
pub fn list_in(dir: &Path) -> Vec<(PathBuf, Value)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, Value)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let m = std::fs::read(p.join(MANIFEST)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())?;
            Some((p, m))
        })
        .collect();
    out.sort_by(|a, b| b.1["at"].as_u64().unwrap_or(0).cmp(&a.1["at"].as_u64().unwrap_or(0)).then_with(|| b.0.cmp(&a.0)));
    out
}

/// Where a background backup leaves its result.
type Slot = std::sync::Arc<std::sync::Mutex<Option<std::result::Result<Value, String>>>>;

/// Everything one backup needs, to run on any thread.
struct Job {
    lib_dir: PathBuf,
    root: PathBuf,
    seq: u64,
    catalog: lightcraft_catalog::Catalog,
    settings_dir: Option<PathBuf>,
    keep: u32,
    verify: bool,
    secs: u64,
}

impl Job {
    fn run(self) -> std::result::Result<Value, String> {
        let t0 = web_time::Instant::now();
        let mut dest = self.root.join(stamp(self.secs));
        let mut n = 2;
        while dest.exists() && n < 100 {
            dest = self.root.join(format!("{} ({n})", stamp(self.secs)));
            n += 1;
        }
        let lib_to = dest.join("library");
        std::fs::create_dir_all(&lib_to).map_err(|e| format!("{}: {e}", lib_to.display()))?;
        let (files, bytes) = match self.write(&dest) {
            Ok(v) => v,
            Err(e) => {
                // a half-written backup is not left looking like a backup
                let _ = std::fs::remove_dir_all(&dest);
                return Err(e);
            }
        };
        if self.verify
            && let Err(e) = verify(&lib_to, self.catalog.len())
        {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(format!("the backup didn't check out ({e}); it was removed and older backups are kept"));
        }
        let pruned = prune(&self.root, self.keep);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        log::info!("backup: {} ({files} files, {bytes} bytes, {ms:.0} ms)", dest.display());
        Ok(json!({
            "path": dest.display().to_string(),
            "library": self.lib_dir.to_string_lossy(),
            "at": self.secs,
            "files": files,
            "bytes": bytes,
            "ms": ms.round(),
            "verified": self.verify,
            "pruned": pruned,
        }))
    }

    fn write(&self, dest: &Path) -> std::result::Result<(usize, u64), String> {
        let lib_to = dest.join("library");
        let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        // the catalog, from memory
        {
            let mut store = lightcraft_catalog::FsStore::open(&lib_to).map_err(|e| err(&lib_to, e))?;
            lightcraft_catalog::Journal::export(&mut store, self.seq, &self.catalog).map_err(|e| format!("catalog: {e}"))?;
        }
        let (mut files, mut bytes) = (2usize, 0u64);
        for name in [lightcraft_catalog::journal::SNAPSHOT, lightcraft_catalog::journal::LOG] {
            bytes += std::fs::metadata(lib_to.join(name)).map(|m| m.len()).unwrap_or(0);
        }
        // the library's own settings files and profiles
        for e in std::fs::read_dir(&self.lib_dir).map_err(|e| err(&self.lib_dir, e))?.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            let is_file = e.file_type().is_ok_and(|t| t.is_file());
            let wanted = if is_file { !SKIP_LIBRARY.contains(&name.as_str()) } else { LIBRARY_DIRS.contains(&name.as_str()) };
            if wanted && !name.contains(".corrupt-") && !name.ends_with(".tmp") {
                copy_tree(&e.path(), &lib_to.join(&name), &[], 1, &mut files, &mut bytes).map_err(|x| err(&e.path(), x))?;
            }
        }
        // LightCraft's settings (the app store of presets and profiles included)
        if let Some(sd) = &self.settings_dir {
            for name in SETTINGS_ENTRIES {
                let from = sd.join(name);
                if from.exists() {
                    copy_tree(&from, &dest.join("settings").join(name), &[], 0, &mut files, &mut bytes).map_err(|e| err(&from, e))?;
                }
            }
        }
        let manifest = json!({
            "format": "lightcraft.backup",
            "version": 1,
            "library": self.lib_dir.display().to_string(),
            "at": self.secs,
            "photos": self.catalog.len(),
            "files": files,
            "bytes": bytes,
            "app": env!("CARGO_PKG_VERSION"),
        });
        std::fs::write(dest.join(MANIFEST), serde_json::to_vec_pretty(&manifest).unwrap_or_default()).map_err(|e| err(dest, e))?;
        Ok((files, bytes))
    }
}

/// Delete the oldest backups in `root` beyond `keep` (0 = keep all) -> how many went.
fn prune(root: &Path, keep: u32) -> usize {
    if keep == 0 {
        return 0;
    }
    let mut gone = 0;
    for (p, m) in list_in(root).into_iter().skip(keep as usize) {
        // only folders this feature wrote
        if m["format"] == "lightcraft.backup" && std::fs::remove_dir_all(&p).is_ok() {
            gone += 1;
        }
    }
    gone
}

impl Session {
    /// Backups configured from `<config>/backup.json`, with LightCraft's settings folder copied into
    /// each one. Not with `LIGHTCRAFT_NO_PREFS`.
    pub fn with_default_backups(mut self) -> Self {
        if std::env::var_os("LIGHTCRAFT_NO_PREFS").is_some() {
            return self;
        }
        if let Some(d) = crate::config::config_dir() {
            self = self.with_backups(&d);
        }
        self
    }

    /// Backups configured from `<settings_dir>/backup.json` (read when present; a damaged file
    /// gives the defaults).
    pub fn with_backups(mut self, settings_dir: &Path) -> Self {
        let file = settings_dir.join(SETTINGS_FILE);
        if let Ok(b) = std::fs::read(&file) {
            match serde_json::from_slice(&b) {
                Ok(s) => self.backups.settings = s,
                Err(e) => log::warn!("backup: {}: {e}; the defaults are used", file.display()),
            }
        }
        self.backups.file = Some(file);
        self.backups.settings_dir = Some(settings_dir.to_path_buf());
        self
    }

    fn save_backup_settings(&mut self) -> Result<()> {
        let Some(f) = self.backups.file.clone() else { return Ok(()) };
        let bytes = serde_json::to_vec_pretty(&self.backups.settings).map_err(|e| EngineError::Other(e.to_string()))?;
        if let Some(p) = f.parent() {
            std::fs::create_dir_all(p).map_err(|e| EngineError::Other(format!("{}: {e}", p.display())))?;
        }
        let tmp = f.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &f)).map_err(|e| EngineError::Other(format!("{}: {e}", f.display())))
    }

    /// Where the open library's backups go: `<destination>/<library name>`.
    pub fn backup_dir(&self) -> Option<PathBuf> {
        let lib = self.library.as_ref().filter(|l| l.on_disk)?;
        let name = lib.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "LightCraft Library".into());
        Some(match self.backups.settings.dir.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(d) => PathBuf::from(d).join(name),
            None => lib.dir.join("Backups"),
        })
    }

    /// What the open library and the shared settings are now: the catalog's journal position and
    /// the contents of the library's settings files and of the shared presets and profiles (not the
    /// view, which changes by just looking). Equal marks: nothing to back up.
    pub fn backup_mark(&self) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let lib = self.library.as_ref().filter(|l| l.on_disk)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        lib.journal().seq().hash(&mut h);
        let mut skip: Vec<&str> = SKIP_LIBRARY.to_vec();
        skip.extend(["view.json", crate::finishing::FILE]);
        hash_files(&lib.dir, &skip, &mut h);
        hash_files(&lib.dir.join("Profiles"), &[], &mut h);
        if let Some(sd) = &self.backups.settings_dir {
            let store = sd.join(crate::app_store::DIR_NAME);
            hash_files(&store, &[], &mut h);
            hash_files(&store.join("Profiles"), &[], &mut h);
            hash_files(&sd.join("camera-profiles"), &[], &mut h);
        }
        Some(format!("{:016x}", h.finish()))
    }

    /// Something changed since this library's last backup (or it has none yet).
    pub fn backup_changed(&self) -> bool {
        let Some(lib) = self.library.as_ref().filter(|l| l.on_disk) else { return false };
        if !self.pending_log.is_empty() {
            return true;
        }
        let key = lib.dir.to_string_lossy().to_string();
        match (self.backups.settings.marks.get(&key), self.backup_mark()) {
            (Some(was), Some(now)) => *was != now,
            _ => true,
        }
    }

    /// A backup is due when LightCraft exits (by the frequency and the last backup of this library).
    pub fn backup_due(&self) -> bool {
        let Some(lib) = self.library.as_ref().filter(|l| l.on_disk) else { return false };
        if self.backups.file.is_none() {
            return false;
        }
        let Some(every) = self.backups.settings.frequency.every() else { return false };
        let last = self.backups.settings.last.get(&lib.dir.to_string_lossy().to_string()).copied().unwrap_or(0);
        now_secs().saturating_sub(last) >= every
    }

    /// Back up the open library → `{path, files, bytes, ms, verified, pruned}`. The library is saved
    /// first and the catalog is written from a copy of what is in memory (consistent while the
    /// library is open). With `background`, the copying runs on a thread of its own with a row in
    /// the activity stack, and this returns `{running: true}` at once ([`Session::poll_backup`]
    /// takes the result).
    pub fn back_up_library(&mut self, background: bool) -> Result<Value> {
        if self.backup_running() {
            return Err(EngineError::Other("a backup is already running".into()));
        }
        let lib = self.library.as_ref().filter(|l| l.on_disk).ok_or_else(|| EngineError::Other("no library on disk is open".into()))?;
        let lib_dir = lib.dir.clone();
        let seq = lib.journal().seq();
        let root = self.backup_dir().ok_or_else(|| EngineError::Other("no backup folder".into()))?;
        if root.starts_with(lib_dir.join("thumbs")) {
            return Err(EngineError::Other("the backup folder can't be inside the thumbnail cache".into()));
        }
        // everything on disk is current
        let _ = self.persist();
        self.save_view();
        self.backups.pending_mark = self.backup_mark().map(|m| (lib_dir.to_string_lossy().to_string(), m));
        let job = Job {
            lib_dir,
            root,
            seq,
            catalog: self.catalog.clone(),
            settings_dir: self.backups.settings_dir.clone(),
            keep: self.backups.settings.keep,
            verify: self.backups.settings.verify,
            secs: now_secs(),
        };
        if !background {
            let r = job.run().map_err(EngineError::Other);
            return self.finish_backup(r);
        }
        let slot: Slot = Default::default();
        let out = slot.clone();
        let task = self.activity.start("backup", "Backing up the library", crate::activity::Cancel::No);
        let spawned = std::thread::Builder::new().name("lc-backup".into()).spawn(move || {
            let _row = task;
            let r = job.run();
            *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);
        });
        if let Err(e) = spawned {
            return Err(EngineError::Other(format!("can't start the backup: {e}")));
        }
        self.backups.running = Some(slot);
        Ok(json!({"running": true}))
    }

    /// A background backup that finished: its result (once); the settings remember it.
    pub fn poll_backup(&mut self) -> Option<Result<Value>> {
        let slot = self.backups.running.as_ref()?;
        let done = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
        self.backups.running = None;
        Some(self.finish_backup(done.map_err(EngineError::Other)))
    }

    /// A background backup is running.
    pub fn backup_running(&self) -> bool {
        self.backups.running.as_ref().is_some_and(|r| r.lock().map(|g| g.is_none()).unwrap_or(false))
    }

    /// Wait for a background backup (quitting) → its result, if one ran.
    pub fn wait_backup(&mut self, limit: std::time::Duration) -> Option<Result<Value>> {
        let t0 = std::time::Instant::now();
        while self.backup_running() && t0.elapsed() < limit {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.poll_backup()
    }

    fn finish_backup(&mut self, r: Result<Value>) -> Result<Value> {
        match r {
            Ok(v) => {
                if let (Some(lib), Some(at)) = (v["library"].as_str(), v["at"].as_u64()) {
                    self.backups.settings.last.insert(lib.to_string(), at);
                }
                if let Some((lib, mark)) = self.backups.pending_mark.take() {
                    self.backups.settings.marks.insert(lib, mark);
                }
                if let Err(e) = self.save_backup_settings() {
                    log::warn!("backup: {e}");
                }
                self.backups.last_result = Some(v.clone());
                Ok(v)
            }
            Err(e) => {
                log::error!("backup: {e}");
                self.backups.last_result = Some(json!({"error": e.to_string()}));
                Err(e)
            }
        }
    }

    /// The open library's backups, newest first.
    pub fn list_backups(&self) -> Vec<Value> {
        let Some(root) = self.backup_dir() else { return Vec::new() };
        list_in(&root)
            .into_iter()
            .map(|(p, m)| json!({"path": p.display().to_string(), "name": p.file_name().map(|n| n.to_string_lossy().to_string()), "at": m["at"], "photos": m["photos"], "bytes": m["bytes"]}))
            .collect()
    }
}

/// Hash the names and contents of the files directly in `dir` (not `skip`), in name order. Each
/// file at most [`MAX_FILE`]; unreadable ones count by name.
fn hash_files(dir: &Path, skip: &[&str], h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .filter(|(n, _)| !skip.contains(&n.as_str()) && !n.ends_with(".tmp"))
        .take(MAX_FILES)
        .collect();
    files.sort();
    for (name, path) in files {
        name.hash(h);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_FILE)
            && let Ok(bytes) = std::fs::read(&path)
        {
            bytes.hash(h);
        }
    }
}

fn has_library(s: &Session) -> std::result::Result<(), String> {
    if s.library.as_ref().is_some_and(|l| l.on_disk) { Ok(()) } else { Err("no library on disk is open".into()) }
}

fn settings_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.backupSettings";
    let mut next = s.backups.settings.clone();
    if let Some(f) = p.get("frequency").and_then(Value::as_str) {
        next.frequency = Frequency::parse(f).ok_or_else(|| bad(C, "frequency: everyExit | daily | weekly | never"))?;
    }
    if let Some(d) = p.get("dir") {
        next.dir = d.as_str().map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
        if let Some(d) = &next.dir
            && !Path::new(d).is_absolute()
        {
            return Err(bad(C, "dir: give a full folder path"));
        }
    }
    if let Some(k) = p.get("keep") {
        next.keep = k.as_u64().filter(|k| *k <= 1000).ok_or_else(|| bad(C, "keep: 0 (all) to 1000"))? as u32;
    }
    if let Some(v) = p.get("verify").and_then(Value::as_bool) {
        next.verify = v;
    }
    s.backups.settings = next;
    s.save_backup_settings()?;
    Ok(settings_json(s))
}

fn settings_json(s: &Session) -> Value {
    let st = &s.backups.settings;
    json!({
        "frequency": st.frequency,
        "dir": st.dir,
        "keep": st.keep,
        "verify": st.verify,
        "folder": s.backup_dir().map(|d| d.display().to_string()),
        "last": s.library.as_ref().and_then(|l| st.last.get(&l.dir.to_string_lossy().to_string())),
        "due": s.backup_due(),
        "available": s.backups.file.is_some(),
    })
}

fn restore_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.restoreBackup";
    let path = p.get("path").and_then(Value::as_str).filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(C, "missing `path` (a backup folder)"))?;
    let beside = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone()).ok_or_else(|| bad(C, "no library on disk is open"))?;
    let with_settings = p.get("settings").and_then(Value::as_bool) == Some(true);
    let settings_dir = s.backups.settings_dir.clone();
    restore(Path::new(path), &beside, settings_dir.as_deref(), with_settings).map_err(|e| bad(C, e))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.backUpNow",
            "Back Up Library Now",
            ["File"],
            None,
            "{background?: bool (on a thread, with a row in the activity stack: answers {running: true})} — copies the catalog (written from memory), the library's settings and profiles, and LightCraft's settings with the shared presets and profiles into a new folder in the backup destination; never photos, caches or API keys; checks the copy when Check backups is on and deletes the oldest beyond the number kept → {path, files, bytes, ms, verified, pruned}",
            has_library,
            |s, p| s.back_up_library(p.get("background").and_then(Value::as_bool) == Some(true))
        ),
        cmd!(
            query "library.backups",
            "Library Backups",
            [],
            None,
            "{} → {backups: [{path, name, at (Unix seconds), photos, bytes}] newest first, settings: {frequency, dir, keep, verify, folder, last, due, available}}",
            always,
            |s, _| Ok(json!({"backups": s.list_backups(), "settings": settings_json(s)}))
        ),
        cmd!(
            "library.backupSettings",
            "Library Backup Settings",
            [],
            None,
            "{frequency?: everyExit | daily | weekly | never (backed up when LightCraft exits), dir?: folder (\"\" = a Backups folder in the library), keep?: 0 (all) – 1000, verify?: bool} → the settings (app-wide, saved in the settings folder)",
            always,
            settings_cmd
        ),
        cmd!(
            "library.restoreBackup",
            "Restore Library from Backup",
            [],
            None,
            "{path: a backup folder, settings?: bool (also put back the shared presets and profiles; the current ones are kept beside)} — copies the backup's library to a new folder beside the open library (nothing is overwritten) → {library: the new folder (open it with app.openLibrary), files, bytes, settingsRestored}",
            has_library,
            restore_cmd
        ),
    ]
}

/// Open the backup's catalog and check it holds `photos` photos.
fn verify(lib: &Path, photos: usize) -> std::result::Result<(), String> {
    let store = lightcraft_catalog::FsStore::open(lib).map_err(|e| e.to_string())?;
    let (_, catalog, _) = lightcraft_catalog::Journal::open(Box::new(store)).map_err(|e| e.to_string())?;
    if catalog.len() != photos {
        return Err(format!("{} photos in the backup, {photos} in the library", catalog.len()));
    }
    Ok(())
}

/// Restore backup `backup` (a backup folder) as a new library folder beside `beside` → the new folder.
/// Nothing is overwritten. With `settings`, the shared presets and profiles in the backup replace
/// the current ones, which are kept beside as `Library Store (before restore …)`.
pub fn restore(backup: &Path, beside: &Path, settings_dir: Option<&Path>, with_settings: bool) -> std::result::Result<Value, String> {
    let manifest: Value = std::fs::read(backup.join(MANIFEST))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("{}: not a LightCraft backup (no {MANIFEST})", backup.display()))?;
    if manifest["format"] != "lightcraft.backup" {
        return Err(format!("{}: not a LightCraft backup", backup.display()));
    }
    let from = backup.join("library");
    verify(&from, manifest["photos"].as_u64().unwrap_or(0) as usize)?;
    let parent = beside.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let name = beside.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "LightCraft Library".into());
    let when = backup.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut to = parent.join(format!("{name} (restored {when})"));
    let mut n = 2;
    while to.exists() && n < 1000 {
        to = parent.join(format!("{name} (restored {when}) {n}"));
        n += 1;
    }
    let (mut files, mut bytes) = (0usize, 0u64);
    copy_tree(&from, &to, &[], 0, &mut files, &mut bytes).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut settings_restored = false;
    if with_settings && let Some(sd) = settings_dir {
        let src = backup.join("settings").join(crate::app_store::DIR_NAME);
        if src.is_dir() {
            let cur = sd.join(crate::app_store::DIR_NAME);
            if cur.exists() {
                let aside = sd.join(format!("{} (before restore {})", crate::app_store::DIR_NAME, stamp(now_secs())));
                std::fs::rename(&cur, &aside).map_err(|e| format!("{}: {e}", cur.display()))?;
            }
            copy_tree(&src, &cur, &[], 0, &mut files, &mut bytes).map_err(|e| format!("{}: {e}", cur.display()))?;
            settings_restored = true;
        }
    }
    Ok(json!({"library": to.display().to_string(), "files": files, "bytes": bytes, "settingsRestored": settings_restored}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-backup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn stamps_are_dates() {
        assert_eq!(stamp(0), "1970-01-01 000000 UTC");
        assert_eq!(stamp(1_760_000_000), "2025-10-09 085320 UTC");
    }

    #[test]
    fn back_up_prune_and_restore() {
        let root = temp("cycle");
        let (lib, cfg, dest) = (root.join("Lib"), root.join("cfg"), root.join("dest"));
        std::fs::create_dir_all(cfg.join(crate::app_store::DIR_NAME)).unwrap();
        std::fs::write(cfg.join("ui.json"), b"{}").unwrap();
        std::fs::write(cfg.join("genfill-keys.json"), b"secret").unwrap();
        std::fs::create_dir_all(cfg.join("models")).unwrap();
        std::fs::write(cfg.join("models/big.onnx"), b"model").unwrap();
        let mut s = Session::new().with_app_store(&cfg.join(crate::app_store::DIR_NAME)).with_backups(&cfg);
        std::fs::create_dir_all(lib.join("RAWs")).unwrap();
        std::fs::write(lib.join("RAWs/a.CR3"), b"raw").unwrap();
        s.open_library(&lib, true).unwrap();
        s.execute("preset.create", &json!({"name": "Alex Warm"})).unwrap();
        s.execute("photo.rate", &json!({"rating": 4})).unwrap();
        s.execute("library.backupSettings", &json!({"dir": dest.to_string_lossy(), "keep": 2, "frequency": "everyExit"})).unwrap();
        assert!(s.backup_due());
        let r = s.execute("library.backUpNow", &json!({})).unwrap();
        let first = PathBuf::from(r["path"].as_str().unwrap());
        assert!(first.starts_with(dest.join("Lib")), "{r}");
        assert!(first.join("library/catalog.snap").is_file() && first.join("library/catalog.log").is_file());
        assert!(first.join("settings/ui.json").is_file());
        assert!(first.join("settings").join(crate::app_store::DIR_NAME).join("presets.json").is_file(), "the shared presets");
        assert!(!first.join("settings/genfill-keys.json").exists(), "never API keys");
        assert!(!first.join("settings/models").exists(), "never models");
        assert!(!first.join("library/thumbs").exists(), "never caches");
        assert!(!first.join("library/RAWs").exists(), "never photos kept in the library folder");
        assert!(!first.join("library/catalog.lock").exists());
        // keep 2
        for _ in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(1100));
            s.execute("library.backUpNow", &json!({})).unwrap();
        }
        // in the background: a row in the activity stack while it runs, the result once
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let r = s.execute("library.backUpNow", &json!({"background": true})).unwrap();
        assert_eq!(r["running"], true);
        let done = s.wait_backup(std::time::Duration::from_secs(30)).expect("a result").unwrap();
        assert!(done["path"].as_str().is_some());
        assert!(s.poll_backup().is_none(), "handed out once");
        let listed = s.execute("library.backups", &json!({})).unwrap();
        assert_eq!(listed["backups"].as_array().unwrap().len(), 2, "{listed}");
        // restore: a new library beside, with the photos' edits
        let newest = listed["backups"][0]["path"].as_str().unwrap().to_string();
        let r = restore(Path::new(&newest), &lib, Some(&cfg), false).unwrap();
        let restored = PathBuf::from(r["library"].as_str().unwrap());
        assert!(restored.starts_with(&root) && restored != lib);
        let mut t = Session::new();
        t.open_library(&restored, false).unwrap();
        assert_eq!(t.catalog.len(), s.catalog.len());
        let active = s.active().unwrap();
        assert_eq!(t.catalog.photo(active).unwrap().rating, 4);
        t.close_library().unwrap();
        s.close_library().unwrap();
        // the settings remember the destination
        let again = Session::new().with_backups(&cfg);
        assert_eq!(again.backups.settings.keep, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn not_a_backup_is_refused() {
        let root = temp("refuse");
        assert!(restore(&root, &root.join("Lib"), None, false).is_err());
        std::fs::write(root.join(MANIFEST), b"{\"format\": \"other\"}").unwrap();
        assert!(restore(&root, &root.join("Lib"), None, false).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
