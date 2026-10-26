//! A persistent library: a directory holding the catalog (op log + snapshot), user presets, the
//! last view state and the preview cache.
//!
//! ```text
//! LightCraft Library/
//!   catalog.snap   catalog.log      (lightcraft-catalog journal)
//!   presets.json   view.json        (user presets + favourites; last source/sort/selection)
//!   prefs.json     (library preferences: XMP sidecars, cache size, auto import; and, read by older
//!                   builds or kept by a library that keeps its own, the app store's settings)
//!   thumbs/        (rendered thumbnail cache, safe to delete)
//!   Originals/     (photos imported with "copy into library")
//! ```
//!
//! The files live in [`Store`]s: a directory ([`FsStore`]) natively, or any other implementation
//! via [`Session::open_library_in`] (the browser build keeps them in OPFS or IndexedDB).
//!
//! Every top-level [`Session::execute`] persists the ops it produced (fsynced) before returning,
//! so a crash loses at most the command in flight. When that write fails the command returns
//! [`EngineError::NotSaved`]: its change stays applied in memory and queued, and every later save
//! retries the queue ([`Session::unsaved`] reports it meanwhile), so nothing is lost once the
//! disk is writable again. The log is compacted into a snapshot when it
//! grows (see [`lightcraft_catalog::SnapshotPolicy`]; written by a worker thread on native, see
//! [`lightcraft_catalog::journal`]) and on [`Session::close_library`].

use std::path::{Path, PathBuf};

use lightcraft_catalog::{FsStore, Journal, LibraryLock, LoadReport, Store};
use serde::{Deserialize, Serialize};

use crate::app_store::PresetsFile;
use crate::{EngineError, LibrarySource, Result, Selection, Session};

/// Library directory name inside the user's Pictures folder.
pub const DEFAULT_NAME: &str = "LightCraft Library";

/// The library folder `path` names. A library is a folder; naming one of its own files
/// (`catalog.snap`, `catalog.log`, `catalog.lock`) means the folder that holds it. Any other
/// existing file is an error that says so, instead of the OS's "File exists" from creating the
/// folder.
pub fn resolve_dir(path: &Path) -> Result<PathBuf> {
    if !path.is_file() {
        return Ok(path.to_path_buf());
    }
    let own = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| [lightcraft_catalog::journal::SNAPSHOT, lightcraft_catalog::journal::LOG, lightcraft_catalog::lock::LOCK].contains(&n));
    if !own {
        return Err(EngineError::Other(format!("{} is a file; a library is a folder", path.display())));
    }
    Ok(match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir.to_path_buf(),
        None => PathBuf::from("."),
    })
}

/// Whether `dir` already holds a library (its catalog files).
pub fn is_library(dir: &Path) -> bool {
    dir.join(lightcraft_catalog::journal::SNAPSHOT).is_file() || dir.join(lightcraft_catalog::journal::LOG).is_file()
}

/// The default library location: `$LIGHTCRAFT_LIBRARY` if set, else `~/Pictures/LightCraft Library`
/// (`%USERPROFILE%\Pictures\LightCraft Library` on Windows).
pub fn default_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LIGHTCRAFT_LIBRARY").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") }?;
    Some(PathBuf::from(home).join("Pictures").join(DEFAULT_NAME))
}

pub struct Library {
    /// The library directory, or a descriptive pseudo-path when the library isn't on disk.
    pub dir: PathBuf,
    /// The library is a real directory: photos can be copied into `Originals/` and thumbnails are
    /// cached in `thumbs/`.
    pub on_disk: bool,
    journal: Journal,
    /// presets.json, view.json, prefs.json.
    pub(crate) files: Box<dyn Store>,
    /// What happened when the catalog was loaded.
    pub report: LoadReport,
    /// Last persistence error (shown by the UI; ops stay pending and are retried).
    pub last_error: Option<String>,
    /// Why the last append of queued ops failed; `None` once a save succeeds. While set, changes
    /// exist only in memory ([`Session::unsaved`]).
    pub unsaved_error: Option<String>,
    /// Why the last write of presets.json failed; `None` once it is written. While set, the preset
    /// changes exist only in memory and count as unsaved ([`Session::unsaved`], issue #2266).
    presets_error: Option<String>,
    /// When the frame loop may retry a failed append ([`Session::persist_if_dirty`] backs off).
    retry_at: Option<web_time::Instant>,
    /// What forgetting untouched Local records did when the library opened.
    pub forgot_local: Option<lightcraft_catalog::ForgetPlan>,
    /// What presets.json holds (see [`PresetsSaved`]).
    presets_saved: PresetsSaved,
    view_written: Vec<u8>,
    /// Settings files that were unreadable or damaged when the library opened (`library.info` →
    /// `settingsWarnings`; shown by the UI once, see [`Session::take_library_warnings`]).
    pub settings_warnings: Vec<String>,
    warnings_reported: usize,
    /// Settings files that couldn't be read or set aside: never overwritten this session.
    blocked: Vec<&'static str>,
    /// The library keeps its own presets, profiles and effects (its `prefs.json` says so, or there
    /// is no app store): they are read from and saved to its files. Otherwise the app store holds
    /// them ([`crate::app_store`]).
    pub own_store: bool,
    /// The library asked to keep its own ("Store with this library"; `prefs.json` →
    /// `storeWithLibrary`).
    pub store_with_library: bool,
    /// While the app store holds them: the library's own shared-settings keys of `prefs.json` as
    /// read, written back unchanged.
    frozen: serde_json::Map<String, serde_json::Value>,
    /// Keeps other processes out of this library while it's open (last: released after the
    /// journal's background snapshot has landed).
    lock: Option<LibraryLock>,
}

/// Where a library's files live (see [`Session::open_library_in`]).
pub struct LibraryStores {
    /// Shown as the library's location (`library.info`).
    pub dir: PathBuf,
    /// The catalog journal (`catalog.snap`, `catalog.log`).
    pub catalog: Box<dyn Store>,
    /// presets.json, view.json, prefs.json.
    pub files: Box<dyn Store>,
    /// `dir` is a real directory (enables `Originals/` copies and the `thumbs/` disk cache).
    pub on_disk: bool,
}

/// The state presets.json was last written (or read) from. Saving checks the preset list's
/// generation and the small profile lists before serializing anything: with a 10 000-preset
/// collection presets.json is ~40 MB, and serializing it after every command made each slider
/// tick take seconds.
#[derive(Default)]
pub(crate) struct PresetsSaved {
    generation: u64,
    profile_favorites: Vec<String>,
    profile_recent: Vec<String>,
    /// Hash of the bytes written: a list borrowed mutably but left as it was isn't rewritten.
    pub(crate) hash: u64,
}

impl PresetsSaved {
    pub(crate) fn state_of(s: &Session) -> PresetsSaved {
        PresetsSaved {
            generation: s.presets.generation(),
            profile_favorites: s.profile_favorites.clone(),
            profile_recent: s.profile_recent.clone(),
            hash: 0,
        }
    }

    /// A state that differs from every session's (the file is saved again).
    pub(crate) fn stale() -> PresetsSaved {
        PresetsSaved { generation: 0, profile_favorites: Vec::new(), profile_recent: Vec::new(), hash: 0 }
    }

    /// `s` changed its presets or profile lists since this state.
    pub(crate) fn differs(&self, s: &Session) -> bool {
        self.generation != s.presets.generation() || self.profile_favorites != s.profile_favorites || self.profile_recent != s.profile_recent
    }
}

pub(crate) fn bytes_hash(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

#[cfg(test)]
thread_local! {
    static PRESETS_SERIALIZED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread serialized the preset list (tests).
#[cfg(test)]
pub(crate) fn presets_serialized() -> usize {
    PRESETS_SERIALIZED.with(|c| c.get())
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ViewFile {
    source: LibrarySource,
    browse: Option<crate::Browse>,
    /// The folder a `libraryFolder` source shows.
    library_folder: Option<String>,
    // No filter: a library opens unfiltered. A date, keyword or person left over from the last session
    // would silently hide photos, with only a small badge to say so.
    sort: lightcraft_catalog::Sort,
    selection: Selection,
}

impl Library {
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    /// A small file of the library's own (`people.json`, `concepts.json`): `None` when missing.
    pub(crate) fn read_file(&mut self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.files.read(name)
    }
    /// Replace a small file of the library's own atomically.
    pub(crate) fn write_file(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        self.files.write_atomic(name, data)
    }
    #[cfg(test)]
    pub(crate) fn journal_mut(&mut self) -> &mut Journal {
        &mut self.journal
    }
    /// presets.json is saved at the next persist, whatever it holds.
    pub(crate) fn reset_presets_saved(&mut self) {
        self.presets_saved = PresetsSaved::default();
    }
    /// The shared-settings keys of the library's prefs.json as they are on disk now.
    pub(crate) fn freeze_shared(&mut self) -> serde_json::Map<String, serde_json::Value> {
        read_frozen(self.files.as_mut())
    }
    pub(crate) fn set_frozen(&mut self, frozen: serde_json::Map<String, serde_json::Value>) {
        self.frozen = frozen;
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }
    pub fn originals_dir(&self) -> PathBuf {
        self.dir.join("Originals")
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PrefsFile {
    xmp: crate::sidecar::XmpPrefs,
    /// Export presets, the last export, print / metadata / filter / curve / label / keyword-set
    /// presets, slideshow settings, Hold View: the app store's while the library uses it (note 52;
    /// kept here as read), the library's own otherwise.
    #[serde(flatten)]
    habits: crate::app_store::Habits,
    /// Import presets (the Import tab's settings, by name).
    import_presets: Vec<crate::cmd::import_tab::ImportPreset>,
    /// Mask effect presets.
    mask_effects: Vec<crate::cmd::mask_effects::MaskEffect>,
    /// The effect new masks of each kind start with.
    mask_tool_effects: std::collections::BTreeMap<String, String>,
    /// Imported LUT profiles.
    lut_profiles: Vec<crate::cmd::lut_profiles::LutProfile>,
    /// Imported camera profiles (`.dcp`).
    dcp_profiles: Vec<crate::cmd::dcp_profiles::DcpEntry>,
    /// Recent keywords (the keywords of this library's photos).
    recent_keywords: Vec<String>,
    /// Develop defaults for imported photos.
    import: crate::import::ImportDefaults,
    /// Thumbnail disk cache budget (MB, 0 = default).
    cache_mb: u32,
    /// This library's own background preparation settings (`None`: the app-wide ones).
    #[serde(skip_serializing_if = "Option::is_none")]
    prep: Option<crate::prep::PrepSettings>,
    /// Folder for smart previews (default: `Smart Previews` in the library).
    smart_previews_dir: Option<String>,
    /// Days after which untouched Local records of unbrowsed folders are forgotten (missing =
    /// the default, 0 = never).
    forget_local_days: Option<u32>,
    // A library-level override of an app-wide setting (note 48: background preparation for this
    // library) is a field here, read on open and winning over the app's while the library is open.
    /// The Profile Manager's choices (while the library keeps its own; else in the app store).
    #[serde(skip_serializing_if = "crate::app_store::ProfileState::is_default")]
    profiles: crate::app_store::ProfileState,
    /// The Presets Manager's choices (while the library keeps its own; else in the app store).
    #[serde(skip_serializing_if = "crate::app_store::PresetState::is_default")]
    preset_state: crate::app_store::PresetState,
    /// Keep presets, profiles and effects with this library instead of the app store.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    store_with_library: bool,
}

pub(crate) fn presets_json(s: &Session) -> String {
    #[cfg(test)]
    PRESETS_SERIALIZED.with(|c| c.set(c.get() + 1));
    // (borrowed: the 10 000+ presets of an imported collection are not copied to be written)
    crate::presets_save::Snapshot::of(s).json()
}

/// Finish a background compaction that is done (cheap otherwise). A failed one lost nothing (the
/// log is kept whole) and is retried later; it is reported like other persistence errors.
fn poll_compaction(lib: &mut Library) {
    if let Err(e) = lib.journal.poll() {
        log::error!("library: compaction: {e}");
        lib.last_error = Some(format!("compaction: {e}"));
    }
}

/// How long the frame loop waits before retrying a failed append (commands retry at once).
const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);

/// `a` and `b` name the same directory.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// This program, for the lock owner note ("LightCraft", "lightcraft-cli").
fn program_name() -> String {
    let exe = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()));
    match exe.as_deref() {
        Some("lightcraft") | None => "LightCraft".into(),
        Some(other) => other.to_string(),
    }
}

/// The keys of prefs.json the app store holds, as `store` has them.
fn read_frozen(store: &mut dyn Store) -> serde_json::Map<String, serde_json::Value> {
    let mut frozen = serde_json::Map::new();
    if let Ok(Some(raw)) = store.read("prefs.json")
        && let Ok(serde_json::Value::Object(map)) = serde_json::from_slice::<serde_json::Value>(&raw)
    {
        for k in crate::app_store::SHARED_PREFS_KEYS.iter().chain(crate::app_store::HABITS_PREFS_KEYS.iter()).chain(["import"].iter()) {
            if let Some(v) = map.get(*k) {
                frozen.insert((*k).to_string(), v.clone());
            }
        }
    }
    frozen
}

fn json_or_null<T: Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

/// Problems reading the settings files (`presets.json`, `prefs.json`, `view.json`) when the library
/// opened (issue #103). A missing file is not a problem (defaults). A file that doesn't parse is
/// kept as `<name>.corrupt-<unix time>` before the defaults are used (so the next save can't
/// lose it); a file that can't be read at all (locked by another program, I/O error) is never
/// written this session, so its content survives until the library is opened again.
#[derive(Default)]
struct SettingsLoad {
    warnings: Vec<String>,
    /// Files not to overwrite this session.
    blocked: Vec<&'static str>,
}

impl SettingsLoad {
    fn read<T: serde::de::DeserializeOwned>(&mut self, store: &mut dyn Store, name: &'static str) -> Option<T> {
        let bytes = match store.read(name) {
            Ok(b) => b?,
            Err(e) => {
                log::error!("library: {name}: {e}");
                self.blocked.push(name);
                self.warnings.push(format!(
                    "{name} couldn't be read ({e}). LightCraft uses the defaults for now and won't overwrite the file; reopen the library to try again."
                ));
                return None;
            }
        };
        let err = match serde_json::from_slice(&bytes) {
            Ok(v) => return Some(v),
            Err(e) => e,
        };
        let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let keep = format!("{name}.corrupt-{secs}");
        log::error!("library: {name} is damaged: {err}");
        match store.write_atomic(&keep, &bytes) {
            Ok(()) => self.warnings.push(format!("{name} is damaged ({err}). It was kept as {keep}, and the defaults are used.")),
            Err(w) => {
                self.blocked.push(name);
                self.warnings.push(format!(
                    "{name} is damaged ({err}) and couldn't be set aside ({w}). LightCraft uses the defaults and won't overwrite the file."
                ));
            }
        }
        None
    }
}

impl Session {
    /// Take over what the thumbnail cache wrote under its name of before (`.jpg` files, which
    /// photo galleries took for the person's photos: issues #610, #1066), once for a cache
    /// folder that was not made or tidied by this build yet. On a thread of its own, with a row
    /// in the activity stack that can stop it: a large cache is some 100 000 files to rename.
    fn tidy_thumbnail_cache(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            let cache = self.media.rendered.clone();
            if !cache.disk().is_some_and(|d| d.needs_tidying()) {
                return;
            }
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let task = self.activity.start("cache", "Tidying the thumbnail cache", crate::activity::Cancel::Flag(stop.clone()));
            let spawned = std::thread::Builder::new().name("lc-thumbs-tidy".into()).spawn(move || {
                // the row stays while this runs (it is dropped with the closure, however that ends)
                let _row = task;
                if let Some(disk) = cache.disk() {
                    // (also given up when the library is closed or another is opened: the cache
                    // is retired then, and it is gone on with when this library opens again)
                    let stopped = || stop.load(Ordering::Relaxed) || cache.generation() == u64::MAX;
                    let taken = disk.take_over_old(&stopped, &|done, of| _row.progress(done as u64, of as u64));
                    log::info!("thumbnail cache: {taken} thumbnails taken over under their new name in {}", disk.dir().display());
                }
            });
            // (not started: the thumbnails are still found one by one, and it is tried at the next open)
            if let Err(e) = spawned {
                log::warn!("thumbnail cache: could not be tidied: {e}");
            }
        }
    }

    /// Check, without closing the library open now, that [`Session::open_library`] can be expected
    /// to open `dir`: not a Lightroom catalog, not some other file, a readable folder (or one not
    /// made yet), not locked by another program. Returns the library folder ([`resolve_dir`]).
    /// Nothing is created; a library folder that exists gets its lock taken and released again.
    /// Opening can still fail afterwards (a damaged catalog, a disk going away).
    pub fn check_library(&self, dir: impl AsRef<Path>) -> Result<PathBuf> {
        let dir = dir.as_ref().to_path_buf();
        if let Some(place) = crate::lightroom_folder::lightroom_catalog_at(&dir) {
            return Err(EngineError::LightroomCatalog { path: dir, catalog: place.catalog });
        }
        let dir = resolve_dir(&dir)?;
        if !dir.exists() {
            return Ok(dir);
        }
        if !dir.is_dir() {
            return Err(EngineError::Other(format!("{} is not a folder", dir.display())));
        }
        std::fs::read_dir(&dir).map_err(|e| EngineError::Other(format!("can't open library {}: {e}", dir.display())))?;
        // the library open in this session keeps its lock when reopened; any other must be free
        let current = self.library.as_ref().is_some_and(|l| l.on_disk && same_dir(&l.dir, &dir));
        if !current && is_library(&dir) {
            drop(LibraryLock::acquire(&dir, &program_name()).map_err(|e| EngineError::LibraryInUse(e.to_string()))?);
        }
        Ok(dir)
    }

    /// Open (or create) the library at `dir` into this session, replacing its catalog. With
    /// `seed_demo`, a newly created library starts with the procedural demo photos.
    ///
    /// The library is locked for this session ([`lightcraft_catalog::lock`]): if another process
    /// has it open, this fails with [`EngineError::LibraryInUse`] and nothing is read or changed.
    /// Reopening the library this session already has open keeps its lock.
    ///
    /// A Lightroom Classic catalog folder, one of its `.lrdata` / `.lrcat-data` folders or an
    /// `.lrcat` file is refused with [`EngineError::LightroomCatalog`] before anything is
    /// created ([`crate::lightroom_folder`]); a LightCraft library always opens, whatever else
    /// its folder holds.
    pub fn open_library(&mut self, dir: impl AsRef<Path>, seed_demo: bool) -> Result<&LoadReport> {
        self.open_library_checked(dir.as_ref(), seed_demo, true)
    }

    /// [`Session::open_library`]; `refuse_older`: a library in an earlier catalog format is refused
    /// (the upgraded copy itself is opened with `false`, which upgrades it in place).
    pub(crate) fn open_library_checked(&mut self, dir: &Path, seed_demo: bool, refuse_older: bool) -> Result<&LoadReport> {
        let dir = dir.to_path_buf();
        if let Some(place) = crate::lightroom_folder::lightroom_catalog_at(&dir) {
            return Err(EngineError::LightroomCatalog { path: dir, catalog: place.catalog });
        }
        let dir = resolve_dir(&dir)?;
        // made by an earlier LightCraft: only ever opened through an upgraded copy (`library_upgrade`)
        if refuse_older && let Some(up) = crate::library_upgrade::needed(&dir) {
            return Err(EngineError::LibraryNeedsUpgrade { path: dir, copy: up.copy, copy_exists: up.copy_exists });
        }
        let open = || FsStore::open(&dir).map_err(|e| EngineError::Other(format!("can't open library {}: {e}", dir.display())));
        let stores = LibraryStores { catalog: Box::new(open()?), files: Box::new(open()?), on_disk: true, dir: dir.clone() };
        // the library open in this session (same directory): hand its lock over
        let reused = self.library.as_mut().filter(|l| same_dir(&l.dir, &dir)).and_then(|l| l.lock.take());
        let reusing = reused.is_some();
        let mut lock = match reused {
            Some(l) => Some(l),
            None => Some(LibraryLock::acquire(&dir, &program_name()).map_err(|e| EngineError::LibraryInUse(e.to_string()))?),
        };
        if let Err(e) = self.open_stores(stores, seed_demo, &mut lock) {
            if reusing && let Some(lib) = self.library.as_mut() {
                lib.lock = lock.take();
            }
            return Err(e);
        }
        self.loaded_report()
    }

    /// Open (or create) a library whose files live in `stores` (e.g. browser storage), replacing
    /// this session's catalog. Not locked: the host guards against a second opener (see
    /// [`Session::open_library`] for directories).
    pub fn open_library_in(&mut self, stores: LibraryStores, seed_demo: bool) -> Result<&LoadReport> {
        self.open_stores(stores, seed_demo, &mut None)?;
        self.loaded_report()
    }

    fn loaded_report(&self) -> Result<&LoadReport> {
        match &self.library {
            Some(lib) => Ok(&lib.report),
            None => Err(EngineError::Other("library closed while opening".into())),
        }
    }

    /// Open `stores`; the new library takes `lock` once nothing can fail any more.
    fn open_stores(&mut self, stores: LibraryStores, seed_demo: bool, lock: &mut Option<LibraryLock>) -> Result<()> {
        let LibraryStores { dir, catalog, mut files, on_disk } = stores;
        self.media.smart_dir = on_disk.then(|| crate::smart::dir(&dir));
        // the current library may be these same files: let its background snapshot land first
        if let Some(old) = self.library.as_mut()
            && let Err(e) = old.journal.wait()
        {
            log::error!("library: {e}");
        }
        // Photo ids belong to one library; denoise work must not carry into the next.
        self.denoise.library_changed();
        self.media.denoise.clear();
        // what was learned about the old library's faces is saved, and none of it carries over (photo ids are per library)
        #[cfg(not(target_arch = "wasm32"))]
        self.faces.library_changed();
        let (mut journal, mut catalog, report) = Journal::open(catalog)?;
        // A loaded catalog counts revisions from 0, like every other one. Caches (sidebar counts,
        // keyword tree, the grid's list…) are keyed on the revision, so give each library loaded
        // in this process its own range: otherwise an empty library opened after one that was
        // never edited this session would look unchanged and show the old library's numbers.
        static LOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        catalog.revision = LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed).wrapping_add(1) << 32;
        self.catalog = catalog;
        // (nothing of the list the last library showed is this one's)
        self.refresh_view();
        self.library_generation = crate::LIBRARY_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for entry in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            self.pending_sidecar_repairs.append(&mut entry.retry_sidecars);
        }
        self.undo.clear();
        self.redo.clear();
        self.interaction = None;
        self.pending_log.clear();
        self.selection = Selection::default();
        self.source = LibrarySource::All;
        self.library_folder = None;
        if report.created && seed_demo {
            crate::demo::load(self);
            journal.snapshot(&self.catalog)?;
        }
        let mut settings = SettingsLoad::default();
        // preferences
        let prefs = settings.read::<PrefsFile>(files.as_mut(), "prefs.json").unwrap_or_default();
        self.xmp = prefs.xmp;
        self.recent_keywords = prefs.recent_keywords;
        self.cache_mb = prefs.cache_mb;
        self.prep.library = prefs.prep.map(crate::prep::PrepSettings::sanitized);
        self.forget_local_days = prefs.forget_local_days.unwrap_or(lightcraft_catalog::DEFAULT_FORGET_DAYS);
        self.refresh_view();
        self.smart_previews_dir = prefs.smart_previews_dir.filter(|_| on_disk).map(PathBuf::from);
        if let Some(d) = &self.smart_previews_dir {
            self.media.smart_dir = Some(d.clone());
        }
        // presets, profiles, effects, import defaults: the app store's, or the library's own
        let store_with_library = prefs.store_with_library;
        let own_store = store_with_library || self.app_store.is_none();
        let mut frozen = serde_json::Map::new();
        let mut merged_now = None;
        if own_store {
            // (the sort comes from view.json below)
            self.set_habits(crate::app_store::Habits { sort: None, ..prefs.habits });
            self.mask_effects = prefs.mask_effects;
            self.mask_tool_effects = prefs.mask_tool_effects;
            self.lut_profiles = prefs.lut_profiles;
            crate::cmd::lut_profiles::register_all(self);
            self.dcp_profiles = prefs.dcp_profiles;
            crate::cmd::dcp_profiles::register_all(self);
            self.import_defaults = prefs.import;
            self.profile_state = prefs.profiles;
            self.import_presets = prefs.import_presets;
            self.preset_state = prefs.preset_state;
            let f = settings.read::<PresetsFile>(files.as_mut(), "presets.json").unwrap_or_default();
            self.set_presets_from(f);
            if let Some(st) = self.app_store.as_mut() {
                // the session holds this library's now
                st.in_session = false;
            }
        } else {
            self.load_app_store();
            // what the store holds is kept in the library's prefs.json as it was read
            frozen = read_frozen(files.as_mut());
            // auto import stays with the library
            self.import_defaults.auto_folder = prefs.import.auto_folder.clone();
            self.import_defaults.auto_copy = prefs.import.auto_copy;
            self.import_defaults.auto_album = prefs.import.auto_album.clone();
            // first time with the store: its presets and profiles join the store (read only)
            if self.app_store.as_ref().is_some_and(|st| !st.has_merged(&dir)) && on_disk {
                let presets = settings.read::<PresetsFile>(files.as_mut(), "presets.json");
                let shared = crate::app_store::LibraryShared {
                    lut_profiles: prefs.lut_profiles,
                    dcp_profiles: prefs.dcp_profiles,
                    mask_effects: prefs.mask_effects,
                    mask_tool_effects: prefs.mask_tool_effects,
                    import: prefs.import,
                    profiles: prefs.profiles,
                    store_with_library: false,
                    import_presets: prefs.import_presets,
                    preset_state: prefs.preset_state,
                    habits: prefs.habits,
                };
                merged_now = Some(self.merge_library_content(&dir, crate::app_store::LibraryContent { shared, presets }));
            } else if on_disk {
                // merged by a build that left export presets and templates with the library: once more for them
                let n = self.merge_library_habits(&dir, prefs.habits);
                if n > 0 {
                    log::info!("store: {n} export presets and templates of the library joined the shared store");
                }
            }
        }
        // the sort is the app's once the store keeps one (else the library's, below)
        let store_sort = (!own_store && self.app_store.as_ref().is_some_and(|st| st.sort_known)).then_some(self.sort);
        // view state
        if let Some(v) = settings.read::<ViewFile>(files.as_mut(), "view.json") {
            self.source = v.source;
            self.browse = v.browse;
            self.library_folder = v.library_folder.filter(|f| !f.trim().is_empty());
            if self.source == LibrarySource::LibraryFolder {
                // a folder that is gone (or none): everything, not an empty grid
                let f = lightcraft_catalog::Filter { library_folder: self.library_folder.clone(), ..Default::default() };
                let any = self.library_folder.is_some() && !self.catalog.query(&f, &lightcraft_catalog::Sort::default()).is_empty();
                if !any {
                    self.source = LibrarySource::All;
                    self.library_folder = None;
                }
            }
            self.sort = store_sort.unwrap_or(v.sort);
            self.selection = v.selection;
            self.selection.ids.retain(|id| self.catalog.photo(*id).is_some());
            self.selection.active = self.selection.active.filter(|id| self.catalog.photo(*id).is_some());
        }
        // not on Missing Photos: working that view out checks every original (minutes on a
        // sleeping NAS), and the app checks them in the background once it runs
        if self.selection.active.is_none()
            && self.source != LibrarySource::Missing
            && let Some(first) = self.visible_cloned().first()
        {
            self.selection = Selection::single(*first);
        }
        // presets.json is as read: saved again only after the list changes (hash 0 = unknown bytes)
        let presets_saved = PresetsSaved::state_of(self);
        // photo ids are per library: drop decoded sources of the previous one
        self.media.clear_sources();
        if on_disk {
            self.media.attach_disk_cache(&dir.join("thumbs"), self.cache_bytes_in(&dir.join("thumbs")));
            self.keep_edited_previews_last();
            self.tidy_thumbnail_cache();
        }
        let view_written = self.view_json();
        self.library = Some(Library {
            dir,
            on_disk,
            journal,
            files,
            report,
            last_error: None,
            unsaved_error: None,
            presets_error: None,
            retry_at: None,
            forgot_local: None,
            presets_saved,
            view_written,
            settings_warnings: unlocked_warning(lock.as_ref()).into_iter().chain(settings.warnings).collect(),
            warnings_reported: 0,
            blocked: settings.blocked,
            own_store,
            store_with_library,
            frozen,
            lock: lock.take(),
        });
        if !own_store {
            if let Some(r) = merged_now {
                log::info!("store: library merged into the shared store: {} presets, {} profiles, {} effects", r.presets, r.profiles, r.effects);
                crate::profile_refs::refresh(self);
            }
            self.apply_store_remap();
            if let Err(e) = self.save_app_store().and_then(|()| self.save_shared_presets()) {
                log::error!("store: {e}");
            }
        }
        // forget untouched Local records of folders not browsed for a while (journaled at once)
        if self.forget_local_days > 0 {
            let plan = self.forget_local(false, None);
            if !plan.evict.is_empty() {
                self.compact_soon();
            } else if let Err(e) = self.persist() {
                log::error!("library: {e}");
            }
            if let Some(lib) = self.library.as_mut() {
                lib.forgot_local = Some(plan);
            }
        }
        // keywords read whole from a list a line long (before they were split) become one each
        if self.split_multiline_keywords() > 0
            && let Err(e) = self.persist()
        {
            log::error!("library: {e}");
        }
        // smart previews written by an older camera-look fit are brought up to date in the background
        #[cfg(not(target_arch = "wasm32"))]
        if on_disk {
            crate::cmd::previews::refresh_stale_smart_previews(self);
        }
        // an import still being finished when the app closed picks up where it stopped
        self.load_finishing();
        self.library_identity = std::sync::Arc::new(());
        Ok(())
    }

    /// Photos whose keywords hold line breaks (a camera preset's whole keyword list read as one
    /// keyword by an earlier version) get one keyword a line, journaled, not an undo step. Returns
    /// how many photos changed.
    pub fn split_multiline_keywords(&mut self) -> usize {
        let fix: Vec<(lightcraft_catalog::PhotoId, lightcraft_catalog::Meta)> = self
            .catalog
            .photos()
            .filter(|p| p.meta.keywords.iter().any(|k| k.contains(['\r', '\n', '\u{2028}', '\u{2029}'])))
            .map(|p| {
                let mut meta = p.meta.clone();
                meta.keywords = lightcraft_catalog::keywords::from_file(&meta.keywords, &[]);
                (p.id, meta)
            })
            .collect();
        let n = fix.len();
        for (id, meta) in fix {
            let op = lightcraft_catalog::Op::SetMeta { id, meta: Box::new(meta) };
            match self.catalog.apply(op.clone()) {
                Ok(_) => self.pending_log.push(op),
                Err(e) => log::warn!("split keywords: {e}"),
            }
        }
        if n > 0 {
            log::info!("split the line-separated keywords of {n} photo(s) into one keyword a line");
        }
        n
    }

    /// Write pending ops to the log (fsynced), compact when due, and save changed presets.
    /// Called after every top-level command; cheap when nothing changed. Fails (with
    /// [`EngineError::NotSaved`]) only when the queued ops couldn't be written; they stay queued.
    pub fn persist(&mut self) -> Result<()> {
        let t0 = web_time::Instant::now();
        let r = self.persist_inner();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ms > 20.0 && crate::command_profiling() {
            eprintln!("persist: {ms:.1} ms ({:?})", self.library.as_ref().map(|l| l.journal.stats()));
        }
        r
    }

    fn persist_inner(&mut self) -> Result<()> {
        let Some(lib) = self.library.as_mut() else { return Ok(()) };
        if !self.pending_log.is_empty() {
            if let Err(e) = lib.journal.append(&self.pending_log) {
                // the ops stay queued (and applied in memory): the next persist retries them
                let reason = e.to_string();
                log::error!("library: {} change(s) not written to disk: {reason}", self.pending_log.len());
                lib.last_error = Some(reason.clone());
                lib.unsaved_error = Some(reason.clone());
                lib.retry_at = Some(web_time::Instant::now() + RETRY_BACKOFF);
                return Err(EngineError::NotSaved(reason));
            }
            if lib.unsaved_error.take().is_some() {
                log::info!("library: queued changes written to disk");
            }
            lib.retry_at = None;
            self.pending_log.clear();
            lib.last_error = None;
        }
        poll_compaction(lib);
        // Never snapshot mid-interaction: the catalog then holds an uncommitted preview value.
        // The snapshot is written by a worker thread (natively); appends go on meanwhile.
        // A failed compaction loses nothing (the log is kept whole and compacted later): it is
        // reported (`library.info` → `lastError`), not returned.
        if lib.journal.wants_snapshot()
            && self.interaction.is_none()
            && let Err(e) = lib.journal.snapshot_in_background(&self.catalog)
        {
            log::error!("library: compaction: {e}");
            lib.last_error = Some(format!("compaction: {e}"));
        }
        if self.uses_app_store() {
            // the presets live in the app store: the library's presets.json stays as it was
            if let Err(e) = self.save_shared_presets() {
                log::error!("store: presets: {e}");
            }
            return Ok(());
        }
        if let Some(e) = self.presets_save_failed()
            && let Some(lib) = self.library.as_mut()
        {
            log::warn!("library: saving the presets again after: {e}");
            lib.presets_saved = PresetsSaved::stale();
        }
        let due = self.library.as_ref().is_some_and(|l| l.presets_saved.differs(self) && !l.blocked.contains(&"presets.json"));
        if !due {
            return Ok(());
        }
        // off the UI thread when it can be (`presets_save`): the bytes are made and compared there
        if let Some(lib) = self.library.take() {
            let known = lib.presets_saved.hash;
            let handed = self.save_presets_in_background(lib.files.as_ref(), known);
            self.library = Some(lib);
            if handed {
                let mut state = PresetsSaved::state_of(self);
                state.hash = known;
                if let Some(lib) = self.library.as_mut() {
                    lib.presets_saved = state;
                }
                return Ok(());
            }
        }
        let presets = presets_json(self);
        let mut state = PresetsSaved::state_of(self);
        state.hash = bytes_hash(presets.as_bytes());
        let Some(lib) = self.library.as_mut() else { return Ok(()) };
        if state.hash == lib.presets_saved.hash {
            lib.presets_saved = state;
            lib.presets_error = None;
        } else if let Err(e) = lib.files.write_atomic("presets.json", presets.as_bytes()) {
            // the state stays stale: retried by the next command and the frame loop (backing off);
            // meanwhile the presets count as unsaved, so the UI warns and a backup refuses
            log::error!("library: presets: {e}");
            lib.presets_error = Some(format!("presets: {e}"));
            lib.retry_at = Some(web_time::Instant::now() + RETRY_BACKOFF);
        } else {
            if lib.presets_error.take().is_some() {
                log::info!("library: presets written to disk");
            }
            lib.presets_saved = state;
        }
        Ok(())
    }

    /// Persist if commands left ops pending (cheap; frontends call it once per frame for state
    /// changed outside [`Session::execute`]).
    pub fn persist_if_dirty(&mut self) {
        let backing_off = self.library.as_ref().and_then(|l| l.retry_at).is_some_and(|t| web_time::Instant::now() < t);
        let presets_unsaved = self.library.as_ref().is_some_and(|l| l.presets_error.is_some());
        if self.library.is_some() && (!self.pending_log.is_empty() || presets_unsaved) && !backing_off {
            let _ = self.persist();
        } else if let Some(lib) = self.library.as_mut() {
            poll_compaction(lib);
        }
    }

    /// Changes applied in memory but not yet written to disk because saving failed: the number of
    /// queued ops (plus one for presets.json) and why (`None` when everything is saved, or no library
    /// is open). Retried by every command and by [`Session::persist_if_dirty`].
    pub fn unsaved(&self) -> Option<(usize, &str)> {
        let lib = self.library.as_ref()?;
        let ops = lib.unsaved_error.as_deref().filter(|_| !self.pending_log.is_empty());
        let presets = lib.presets_error.as_deref();
        let n = ops.map_or(0, |_| self.pending_log.len()) + usize::from(presets.is_some());
        ops.or(presets).map(|why| (n, why))
    }

    /// Flush everything and write a snapshot (on quit). Ends an open interaction first.
    ///
    /// The snapshot is written even when appending the queued ops fails: it is a fresh file
    /// holding everything in memory, so it saves those ops too (the log handle may be the only
    /// thing that's broken). Fails only if nothing could be saved; the ops then stay queued.
    pub fn close_library(&mut self) -> Result<()> {
        if self.library.is_none() {
            return Ok(());
        }
        let _ = self.end_interaction();
        let persisted = self.persist();
        // a presets.json save on its way finishes first (it may be this library's)
        self.flush_presets();
        self.save_view();
        let unlogged = if persisted.is_err() { self.pending_log.len() as u64 } else { 0 };
        let Some(lib) = self.library.as_mut() else { return Ok(()) };
        let before = lib.journal.seq();
        let snapshot = lib.journal.snapshot_with_unlogged(&self.catalog, unlogged);
        if unlogged > 0 && lib.journal.seq() == before + unlogged {
            // the snapshot holds the queued ops: they are saved, never append them again
            log::info!("library: {unlogged} queued change(s) saved in the closing snapshot");
            self.pending_log.clear();
            lib.unsaved_error = None;
            lib.retry_at = None;
            lib.last_error = None;
        }
        let result = match (persisted, snapshot) {
            (_, Ok(())) => Ok(()),
            // neither the log nor the snapshot took the queued ops
            (Err(EngineError::NotSaved(e)), Err(s)) if lib.journal.seq() == before => Err(EngineError::NotSaved(format!("{e}; snapshot: {s}"))),
            (_, Err(s)) => Err(s.into()),
        };
        if result.is_ok() {
            self.library_identity = std::sync::Arc::new(());
        }
        result
    }

    fn view_json(&self) -> Vec<u8> {
        let view = ViewFile {
            source: self.source,
            browse: self.browse.clone(),
            // only while it is shown: a leftover would be a stale choice nobody made
            library_folder: self.library_folder.clone().filter(|_| self.source == LibrarySource::LibraryFolder),
            sort: self.sort,
            selection: self.selection.clone(),
        };
        serde_json::to_vec_pretty(&view).unwrap_or_default()
    }

    /// Save the view state (source, sort, selection) if it changed since it was last
    /// written. Native hosts get this from [`Session::close_library`]; the browser host calls it
    /// periodically, since a tab can be closed without notice.
    pub fn save_view(&mut self) {
        if self.library.is_none() {
            return;
        }
        // the sort is the app store's (note 52)
        if let Err(e) = self.save_app_store() {
            log::error!("store: {e}");
        }
        let v = self.view_json();
        let Some(lib) = self.library.as_mut() else { return };
        if v != lib.view_written && !lib.blocked.contains(&"view.json") {
            match lib.files.write_atomic("view.json", &v) {
                Ok(()) => lib.view_written = v,
                Err(e) => log::error!("library: view: {e}"),
            }
        }
    }

    /// Save the library preferences (no-op for in-memory sessions).
    pub fn save_prefs(&mut self) -> Result<()> {
        let v = serde_json::to_value(PrefsFile {
            xmp: self.xmp,
            habits: crate::app_store::Habits { sort: None, ..self.habits() },
            import_presets: self.import_presets.clone(),
            mask_effects: self.mask_effects.clone(),
            mask_tool_effects: self.mask_tool_effects.clone(),
            lut_profiles: self.lut_profiles.clone(),
            dcp_profiles: self.dcp_profiles.clone(),
            recent_keywords: self.recent_keywords.clone(),
            import: self.import_defaults.clone(),
            cache_mb: self.cache_mb,
            prep: self.prep.library,
            smart_previews_dir: self.smart_previews_dir.as_ref().map(|d| d.to_string_lossy().to_string()),
            forget_local_days: Some(self.forget_local_days),
            profiles: self.profile_state.clone(),
            preset_state: self.preset_state.clone(),
            store_with_library: self.library.as_ref().is_some_and(|l| l.store_with_library),
        });
        // while the app store holds them, the library's own copies are written back as read
        let store = self.save_app_store();
        let v = match (v, self.library.as_ref()) {
            (Ok(serde_json::Value::Object(mut map)), Some(lib)) if !lib.own_store => {
                for k in crate::app_store::SHARED_PREFS_KEYS.iter().chain(crate::app_store::HABITS_PREFS_KEYS.iter()) {
                    match lib.frozen.get(*k) {
                        Some(old) => map.insert(k.to_string(), old.clone()),
                        None => map.remove(*k),
                    };
                }
                let mut import = lib.frozen.get("import").cloned().unwrap_or_else(|| json_or_null(&crate::import::ImportDefaults::default()));
                if let serde_json::Value::Object(im) = &mut import {
                    im.insert("autoFolder".into(), json_or_null(&self.import_defaults.auto_folder));
                    im.insert("autoCopy".into(), serde_json::Value::Bool(self.import_defaults.auto_copy));
                    im.insert("autoAlbum".into(), json_or_null(&self.import_defaults.auto_album));
                }
                map.insert("import".into(), import);
                serde_json::to_vec_pretty(&serde_json::Value::Object(map)).unwrap_or_default()
            }
            (Ok(v), _) => serde_json::to_vec_pretty(&v).unwrap_or_default(),
            (Err(_), _) => Vec::new(),
        };
        let Some(lib) = self.library.as_mut() else { return store };
        if v.is_empty() {
            return Err(EngineError::Other("prefs: couldn't be serialized".into()));
        }
        if lib.blocked.contains(&"prefs.json") {
            return Err(EngineError::Other(
                "prefs: prefs.json couldn't be read when the library opened, so it isn't overwritten; reopen the library to save preferences".into(),
            ));
        }
        lib.files.write_atomic("prefs.json", &v).map_err(|e| EngineError::Other(format!("prefs: {e}")))?;
        store
    }

    /// Warnings about the open library not handed out yet: its settings files, and a lock that
    /// couldn't be taken (the UI shows each once; the CLI and MCP print them on stderr).
    pub fn take_library_warnings(&mut self) -> Vec<String> {
        let mut out = self.app_store.as_mut().map(crate::app_store::AppStore::take_warnings).unwrap_or_default();
        let Some(lib) = self.library.as_mut() else { return out };
        out.extend(lib.settings_warnings.get(lib.warnings_reported..).unwrap_or_default().iter().cloned());
        lib.warnings_reported = lib.settings_warnings.len();
        out
    }

    /// The thumbnail disk cache budget in bytes ([`Session::cache_mb`], else the default, which
    /// grows with the library: [`crate::media::default_disk_cache_bytes`]).
    pub fn cache_bytes(&self) -> u64 {
        match (self.cache_mb, self.media.rendered.disk()) {
            (0, Some(d)) => self.cache_bytes_in(d.dir()),
            (0, None) => crate::media::default_disk_cache_bytes(self.catalog.len()),
            (mb, _) => u64::from(mb) << 20,
        }
    }

    /// [`Self::cache_bytes`] for a cache in `dir`: Automatic is a share of its drive's free space
    /// ([`crate::media::automatic_disk_cache_bytes`]; looked up at most once a minute).
    pub fn cache_bytes_in(&self, dir: &std::path::Path) -> u64 {
        if self.cache_mb != 0 {
            return u64::from(self.cache_mb) << 20;
        }
        let used = self.media.rendered.disk().filter(|d| d.dir() == dir).and_then(|d| d.known_size()).unwrap_or(0);
        crate::media::automatic_disk_cache_bytes(self.catalog.len(), self.cache_drive_free(dir), used)
    }

    /// What Automatic would make the preview cache now (Settings shows it), whatever is chosen.
    pub fn automatic_cache_bytes(&self) -> u64 {
        match self.media.rendered.disk() {
            Some(d) => {
                let used = d.known_size().unwrap_or(0);
                crate::media::automatic_disk_cache_bytes(self.catalog.len(), self.cache_drive_free(d.dir()), used)
            }
            None => crate::media::default_disk_cache_bytes(self.catalog.len()),
        }
    }

    /// The free space of the drive holding `dir` (`None`: unknown), looked up again after a minute.
    fn cache_drive_free(&self, dir: &std::path::Path) -> Option<u64> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut known = self.cache_drive_free.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((at, d, free)) = known.as_ref()
                && d == dir
                && at.elapsed() < std::time::Duration::from_secs(60)
            {
                return *free;
            }
            let free = lightcraft_sysmem::free_disk_space(dir);
            *known = Some((web_time::Instant::now(), dir.to_path_buf(), free));
            free
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = dir;
            None
        }
    }

    /// The edited photos' latest previews (their last loupe view and grid thumbnail) are the last
    /// pictures a full disk cache lets go: the only pictures of their edits (an unedited photo has
    /// its camera JPEG). Notes 3/11/31 of 2026-10-24: "every rendered edit is also lost".
    pub fn keep_edited_previews_last(&self) {
        let Some(d) = self.media.rendered.disk() else { return };
        let keys = self
            .catalog
            .photos()
            .filter(|p| !crate::import::looks_as_shot(p))
            .flat_map(|p| [crate::media::latest_view_key(p, true), crate::media::latest_thumb_key(p)]);
        d.keep_last(keys);
    }

    /// Bring the disk cache's budget up to date with the library (photos were added since it was
    /// set): cheap, nothing is read or written. Called before previews are built.
    pub fn fit_disk_cache(&mut self) {
        let want = self.cache_bytes();
        if let Some(d) = self.media.rendered.disk()
            && d.budget() != want
        {
            d.set_budget(want);
        }
    }

    /// Change the thumbnail disk cache budget (MB, 0 = default) in place: the cached thumbnails
    /// stay (the next write trims to the new size), and so do the textures shown from them.
    /// Saved with the library preferences.
    pub fn set_cache_mb(&mut self, mb: u32) -> Result<()> {
        self.cache_mb = mb;
        if let Some(lib) = self.library.as_ref().filter(|l| l.on_disk) {
            let dir = lib.thumbs_dir();
            self.media.set_disk_cache_bytes(&dir, self.cache_bytes());
        }
        self.save_prefs()
    }

    /// Save, then start compacting in the background (natively; synchronously elsewhere): after
    /// a change that shrinks the catalog a lot, so the smaller snapshot replaces the old one.
    /// Errors are logged and reported like other persistence errors (nothing is lost).
    pub fn compact_soon(&mut self) {
        if let Err(e) = self.persist() {
            log::error!("library: {e}");
            return;
        }
        if self.interaction.is_some() {
            return;
        }
        let Some(lib) = self.library.as_mut() else { return };
        if let Err(e) = lib.journal.snapshot_in_background(&self.catalog) {
            log::error!("library: compaction: {e}");
            lib.last_error = Some(format!("compaction: {e}"));
        }
    }

    /// Compact the log into a snapshot now.
    pub fn compact_library(&mut self) -> Result<()> {
        self.persist()?;
        if self.interaction.is_some() {
            return Err(EngineError::Other("can't compact during an interaction".into()));
        }
        if let Some(lib) = self.library.as_mut() {
            lib.journal.snapshot(&self.catalog)?;
        }
        Ok(())
    }
}

/// The warning for a library opened without its one-program-at-a-time lock (issue #171): its
/// file system can't lock `catalog.lock` (some network shares). It opens anyway, as documented —
/// refusing would lock the user out — but the user must learn that a second program could open it.
fn unlocked_warning(lock: Option<&LibraryLock>) -> Option<String> {
    lock.filter(|l| !l.held()).map(|_| {
        "This library could not be locked (its catalog.lock file can't be locked where it is stored, e.g. on some network \
         shares), so it is open without protection against a second program: use it in one LightCraft app or command at \
         a time, or changes made in one of them can be lost."
            .to_string()
    })
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    /// `--library <dir>/catalog.snap` failed with the OS's "File exists" (creating a folder over
    /// the file); it opens the library that file belongs to.
    #[test]
    fn a_librarys_own_file_opens_its_folder_and_any_other_file_is_a_clear_error() {
        let dir = std::env::temp_dir().join(format!("lc-resolve-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new().with_fs();
        s.open_library(&dir, false).unwrap();
        s.close_library().unwrap();
        drop(s);
        assert!(is_library(&dir));
        let snap = dir.join(lightcraft_catalog::journal::SNAPSHOT);
        assert_eq!(resolve_dir(&snap).unwrap(), dir);
        let mut s = Session::new().with_fs();
        s.open_library(&snap, false).unwrap();
        assert_eq!(s.library.as_ref().map(|l| l.dir.clone()), Some(dir.clone()));
        drop(s);

        let other = dir.join("notes.txt");
        std::fs::write(&other, b"x").unwrap();
        let e = Session::new().with_fs().open_library(&other, false).err().unwrap().to_string();
        assert!(e.contains("is a file"), "{e}");
        assert!(!e.contains("File exists"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `check_library` refuses what `open_library` would, without touching the open library.
    #[test]
    fn check_library_refuses_locked_libraries_files_and_lightroom_catalogs() {
        let dir = std::env::temp_dir().join(format!("lc-check-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&b).unwrap();
        let mut s = Session::new().with_fs();
        s.open_library(&b, false).unwrap();
        s.close_library().unwrap();
        s.open_library(&a, false).unwrap();
        assert_eq!(s.check_library(&a).unwrap(), a, "the open library keeps its lock");
        assert_eq!(s.check_library(&b).unwrap(), b);
        assert_eq!(s.check_library(dir.join("new")).unwrap(), dir.join("new"));
        assert!(!dir.join("new").exists(), "nothing created");
        let held = LibraryLock::acquire(&b, "other").unwrap();
        assert!(matches!(s.check_library(&b), Err(EngineError::LibraryInUse(_))));
        drop(held);
        assert!(s.check_library(&b).is_ok(), "the probe released its lock");
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert!(s.check_library(dir.join("notes.txt")).is_err());
        std::fs::write(dir.join("Cat.lrcat"), b"x").unwrap();
        assert!(matches!(s.check_library(dir.join("Cat.lrcat")), Err(EngineError::LightroomCatalog { .. })));
        assert!(s.library.as_ref().is_some_and(|l| l.dir == a));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
