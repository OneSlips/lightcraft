//! The app-wide store: camera profiles (`.dcp`), LUT profiles, develop presets, local adjustment
//! (mask effect) presets, import defaults, body profile defaults and the Profile Manager's choices,
//! shared by every library on this computer, like the other app's presets folder.
//!
//! ```text
//! <settings folder>/Library Store/
//!   shared.json    (profiles list, body defaults, mask effects, import defaults, Profile Manager)
//!   presets.json   (develop presets + favourites, profile favourites and recents: as a library's)
//!   Profiles/      (the profile files: .dcp, .cube)
//! ```
//!
//! A host turns it on ([`Session::with_default_app_store`]: the desktop app, and the CLI when it opens
//! a library); a bare [`Session`] keeps everything with its library, so tests stay hermetic. A library
//! can still keep its own ("Store with this library", `store.useLibrary`): its `prefs.json` /
//! `presets.json` / `Profiles/` then hold them, as before.
//!
//! **Migration.** Each library is merged into the store once: when it is opened, and at start-up for
//! the libraries the app knows (`store.mergeLibraries`). Its profiles are copied into
//! `Profiles/` (the library's own copies stay), its presets, effects and body defaults are added
//! (the same one twice is kept once), and nothing in the library is changed by the merge. A profile id
//! the store already uses for a different file gets a new id; photos of that library are moved to it
//! when the library is opened ([`Merged::remap`]).
//!
//! While a library uses the store, its own copies are left as they were read: `prefs.json` keeps
//! its profile and effect lists, and `presets.json` is not written.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use lightcraft_catalog::{FsStore, Store};
use lightcraft_develop::Preset;
use serde::{Deserialize, Serialize};

use crate::cmd::dcp_profiles::DcpEntry;
use crate::cmd::lut_profiles::LutProfile;
use crate::cmd::mask_effects::MaskEffect;
use crate::import::ImportDefaults;
use crate::{EngineError, Result, Session};

/// The store's folder in LightCraft's settings folder.
pub const DIR_NAME: &str = "Library Store";
/// Profiles, effects, body defaults, import defaults, Profile Manager choices, merged libraries.
pub const SHARED: &str = "shared.json";
/// Develop presets (same format as a library's `presets.json`).
pub const PRESETS: &str = "presets.json";
/// The profile files.
pub const PROFILES_DIR: &str = "Profiles";
/// Largest `presets.json` read when a library is merged (a 11 000-preset collection is ~40 MB).
const MAX_PRESETS_FILE: u64 = 512 << 20;
/// Largest `prefs.json` / `shared.json` read.
const MAX_SMALL_FILE: u64 = 64 << 20;
/// Largest profile file copied (a DCP with a big look table is a few MB).
const MAX_PROFILE_FILE: u64 = 64 << 20;

/// The Profile Manager's choices: profiles hidden from the menus, built-in looks removed from this
/// install, new names, groups and order. Kept in the store (or with a library that keeps its own).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileState {
    /// Out of the menus and the browser, kept in the manager.
    pub hidden: Vec<String>,
    /// Built-in looks deleted from this install ("Restore built-in looks" brings them back).
    pub removed: Vec<String>,
    /// New names by profile id.
    pub names: BTreeMap<String, String>,
    /// Groups chosen by profile id.
    pub groups: BTreeMap<String, String>,
    /// Profiles in the order the user put them (the others follow in their usual order).
    pub order: Vec<String>,
    /// Groups in the order the user put them, empty groups made in the manager included (the
    /// others follow: the user's own first, then LightCraft's).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_order: Vec<String>,
}

impl ProfileState {
    pub fn is_default(&self) -> bool {
        *self == ProfileState::default()
    }

    /// Add what `other` has and this lacks.
    fn merge(&mut self, other: ProfileState, remap: &BTreeMap<String, String>) {
        let map = |id: String| remap.get(&id).cloned().unwrap_or(id);
        for id in other.hidden.into_iter().map(map) {
            if !self.hidden.contains(&id) {
                self.hidden.push(id);
            }
        }
        for id in other.removed {
            if !self.removed.contains(&id) {
                self.removed.push(id);
            }
        }
        for (id, name) in other.names {
            self.names.entry(map(id)).or_insert(name);
        }
        for (id, group) in other.groups {
            self.groups.entry(map(id)).or_insert(group);
        }
        for id in other.order.into_iter().map(map) {
            if !self.order.contains(&id) {
                self.order.push(id);
            }
        }
        for g in other.group_order {
            if !self.group_order.contains(&g) {
                self.group_order.push(g);
            }
        }
    }
}

/// The Presets Manager's choices: presets and built-in presets hidden or removed from this install,
/// built-in presets renamed or moved to another group, and the order of the groups. User presets
/// carry their own name and group.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PresetState {
    /// Out of the Presets panel and the pickers, kept in the manager.
    pub hidden: Vec<String>,
    /// Built-in presets deleted from this install ("Restore built-in presets" brings them back).
    pub removed: Vec<String>,
    /// New names of built-in presets.
    pub names: BTreeMap<String, String>,
    /// Groups of built-in presets moved elsewhere.
    pub groups: BTreeMap<String, String>,
    /// Groups in the order the user put them, empty groups made in the manager included.
    pub group_order: Vec<String>,
    /// Presets marked At Import: the Import panel's picker shows them first (with Favorites).
    pub for_import: Vec<String>,
}

impl PresetState {
    pub fn is_default(&self) -> bool {
        *self == PresetState::default()
    }

    fn merge(&mut self, other: PresetState) {
        for (mine, theirs) in [
            (&mut self.hidden, other.hidden),
            (&mut self.removed, other.removed),
            (&mut self.group_order, other.group_order),
            (&mut self.for_import, other.for_import),
        ] {
            for x in theirs {
                if !mine.contains(&x) {
                    mine.push(x);
                }
            }
        }
        for (k, v) in other.names {
            self.names.entry(k).or_insert(v);
        }
        for (k, v) in other.groups {
            self.groups.entry(k).or_insert(v);
        }
    }
}

/// A library merged into the store.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Merged {
    /// The library folder, as it was named.
    pub library: String,
    /// When (ISO 8601).
    pub at: String,
    /// What it added.
    pub presets: usize,
    pub profiles: usize,
    pub effects: usize,
    /// Profile ids of the library that the store gave a new id (another profile had it): photos of
    /// the library are moved to the new id when it is opened.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub remap: BTreeMap<String, String>,
    /// The remap was applied to the library's photos.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remapped: bool,
}

/// `shared.json`.
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SharedFile {
    version: u32,
    lut_profiles: Vec<LutProfile>,
    dcp_profiles: Vec<DcpEntry>,
    mask_effects: Vec<MaskEffect>,
    mask_tool_effects: BTreeMap<String, String>,
    import: ImportDefaults,
    profiles: ProfileState,
    merged: Vec<Merged>,
    /// Import presets (named sets of import settings).
    import_presets: Vec<crate::cmd::import_tab::ImportPreset>,
    /// The Presets Manager's choices.
    preset_state: PresetState,
}

const SHARED_VERSION: u32 = 1;

/// A library's (or the store's) `presets.json`.
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PresetsFile {
    pub user: Vec<Preset>,
    pub favorites: Vec<String>,
    /// Favourite profiles, and the recently applied ones (newest first).
    pub profile_favorites: Vec<String>,
    pub profile_recent: Vec<String>,
}

/// The parts of a library's `prefs.json` the store holds.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LibraryShared {
    pub lut_profiles: Vec<LutProfile>,
    pub dcp_profiles: Vec<DcpEntry>,
    pub mask_effects: Vec<MaskEffect>,
    pub mask_tool_effects: BTreeMap<String, String>,
    pub import: ImportDefaults,
    pub profiles: ProfileState,
    pub store_with_library: bool,
    pub import_presets: Vec<crate::cmd::import_tab::ImportPreset>,
    pub preset_state: PresetState,
}

/// The keys of a library's `prefs.json` that the store holds while the library uses it: written back
/// as they were read (the library's own copy stays as it was).
pub(crate) const SHARED_PREFS_KEYS: [&str; 7] =
    ["lutProfiles", "dcpProfiles", "maskEffects", "maskToolEffects", "profiles", "importPresets", "presetState"];

/// The store on disk.
pub struct AppStore {
    /// Its folder.
    pub dir: PathBuf,
    files: Box<dyn Store>,
    /// `shared.json` as last written or read (it is written again only when it changes).
    shared_written: Vec<u8>,
    /// The session holds the store's content (read once; a library keeping its own replaces it).
    pub(crate) in_session: bool,
    /// Libraries merged so far.
    pub merged: Vec<Merged>,
    /// Files that couldn't be read when the store was loaded: never overwritten this session.
    blocked: Vec<&'static str>,
    /// Problems to show once (damaged files, profiles that couldn't be copied).
    pub warnings: Vec<String>,
    warnings_reported: usize,
}

impl AppStore {
    /// The store in `dir` (created when missing).
    pub fn open(dir: &Path) -> std::io::Result<AppStore> {
        let files = FsStore::open(dir)?;
        Ok(AppStore {
            dir: dir.to_path_buf(),
            files: Box::new(files),
            shared_written: Vec::new(),
            in_session: false,
            merged: Vec::new(),
            blocked: Vec::new(),
            warnings: Vec::new(),
            warnings_reported: 0,
        })
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.dir.join(PROFILES_DIR)
    }

    /// `library` (a folder) was merged already.
    pub fn has_merged(&self, library: &Path) -> bool {
        self.merged.iter().any(|m| same_path(Path::new(&m.library), library))
    }

    /// A store file, parsed; `None` when missing. A damaged file is kept as `<name>.corrupt-<time>`
    /// and the defaults are used; a file that can't be read is never written this session.
    fn read<T: serde::de::DeserializeOwned>(&mut self, name: &'static str) -> Option<T> {
        let bytes = match self.files.read(name) {
            Ok(b) => b?,
            Err(e) => {
                log::error!("store: {name}: {e}");
                self.blocked.push(name);
                self.warnings.push(format!("The shared {name} couldn't be read ({e}). LightCraft won't overwrite it this session."));
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(err) => {
                let keep = format!("{name}.corrupt-{}", unix_secs());
                match self.files.write_atomic(&keep, &bytes) {
                    Ok(()) => self.warnings.push(format!("The shared {name} is damaged ({err}). It was kept as {keep}.")),
                    Err(w) => {
                        self.blocked.push(name);
                        self.warnings.push(format!("The shared {name} is damaged ({err}) and couldn't be set aside ({w}); it isn't overwritten."));
                    }
                }
                None
            }
        }
    }

    fn write(&mut self, name: &'static str, bytes: &[u8]) -> Result<()> {
        if self.blocked.contains(&name) {
            return Err(EngineError::Other(format!("the shared {name} couldn't be read when LightCraft started, so it isn't overwritten")));
        }
        self.files.write_atomic(name, bytes).map_err(|e| EngineError::Other(format!("shared {name}: {e}")))
    }

    pub(crate) fn blocks(&self, name: &str) -> bool {
        self.blocked.contains(&name)
    }

    /// The store's files (a background save takes a handle of its own from it).
    pub(crate) fn store(&self) -> &dyn Store {
        self.files.as_ref()
    }

    pub(crate) fn write_presets(&mut self, bytes: &[u8]) -> Result<()> {
        self.write(PRESETS, bytes)
    }

    /// Warnings not handed out yet.
    pub fn take_warnings(&mut self) -> Vec<String> {
        let new = self.warnings.get(self.warnings_reported..).map(<[String]>::to_vec).unwrap_or_default();
        self.warnings_reported = self.warnings.len();
        new
    }
}

fn unix_secs() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `a` and `b` name the same folder (case and separators as the system sees them).
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            let norm = |p: &Path| {
                let s = p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_string();
                if cfg!(windows) { s.to_lowercase() } else { s }
            };
            norm(a) == norm(b)
        }
    }
}

/// Read a file of at most `max` bytes (`None` when missing).
fn read_capped(path: &Path, max: u64) -> std::result::Result<Option<Vec<u8>>, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > max => Err(format!("{}: {} bytes is too large", path.display(), m.len())),
        Ok(_) => std::fs::read(path).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// What one merge added.
#[derive(Default, Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub library: String,
    pub presets: usize,
    pub profiles: usize,
    pub effects: usize,
    /// Problems (a profile file that couldn't be read…); nothing is lost: the library keeps its copy.
    pub problems: Vec<String>,
    /// Already merged before: nothing done.
    pub skipped: bool,
}

/// The store-held content a library carries, read from its folder (never written).
pub(crate) struct LibraryContent {
    pub shared: LibraryShared,
    pub presets: Option<PresetsFile>,
}

/// Read a library folder's `prefs.json` and `presets.json` (read only).
pub(crate) fn read_library(dir: &Path) -> std::result::Result<LibraryContent, String> {
    let shared = match read_capped(&dir.join("prefs.json"), MAX_SMALL_FILE)? {
        Some(b) => serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", dir.join("prefs.json").display()))?,
        None => LibraryShared::default(),
    };
    let presets = match read_capped(&dir.join("presets.json"), MAX_PRESETS_FILE)? {
        Some(b) => Some(serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", dir.join("presets.json").display()))?),
        None => None,
    };
    Ok(LibraryContent { shared, presets })
}

/// Same content (bytes) as the file at `path`.
fn file_is(path: &str, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|b| b == bytes)
}

/// A file name for `slug` + `ext` in `dir` that isn't taken by a different file.
fn free_file(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> PathBuf {
    let stem = if stem.trim().is_empty() { "profile" } else { stem };
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2u32;
    while path.exists() && !file_is(&path.to_string_lossy(), bytes) {
        path = dir.join(format!("{stem}-{n}.{ext}"));
        n = n.saturating_add(1);
        if n > 10_000 {
            break;
        }
    }
    path
}

/// A free id: `base`, else `base-2`, `base-3`…
fn free_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut id = base.to_string();
    let mut n = 2u32;
    while taken(&id) && n < 100_000 {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

impl Session {
    /// Keep profiles, presets and the other shared settings in LightCraft's settings folder
    /// (`<config>/Library Store`), shared by every library. Not with `LIGHTCRAFT_NO_PREFS` (tests,
    /// scripts) or without a settings folder.
    pub fn with_default_app_store(self) -> Self {
        if std::env::var_os("LIGHTCRAFT_NO_PREFS").is_some() {
            return self;
        }
        match crate::config::config_dir() {
            Some(d) => self.with_app_store(&d.join(DIR_NAME)),
            None => self,
        }
    }

    /// [`Session::with_default_app_store`] on a session that exists already.
    pub fn use_default_app_store(&mut self) {
        let s = std::mem::take(self);
        *self = s.with_default_app_store();
    }

    /// Keep the shared settings in `dir` ([`AppStore`]). A folder that can't be made leaves every
    /// library keeping its own (logged).
    pub fn with_app_store(mut self, dir: &Path) -> Self {
        match AppStore::open(dir) {
            Ok(st) => self.app_store = Some(st),
            Err(e) => log::error!("store: {}: {e}; libraries keep their own presets and profiles", dir.display()),
        }
        if self.uses_app_store() {
            self.load_app_store();
        }
        self
    }

    /// The shared settings live in the app store now (not in the open library).
    pub fn uses_app_store(&self) -> bool {
        self.app_store.is_some() && !self.library.as_ref().is_some_and(|l| l.own_store)
    }

    /// Where the profile files of the shared settings go: the store's `Profiles/`, or the library's
    /// when it keeps its own (`None`: neither on disk).
    pub fn profiles_dir(&self) -> Option<PathBuf> {
        if self.uses_app_store() {
            return self.app_store.as_ref().map(AppStore::profiles_dir);
        }
        self.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join("Profiles"))
    }

    /// Load the store's content into the session (once; again after a library kept its own).
    pub(crate) fn load_app_store(&mut self) {
        let Some(st) = self.app_store.as_mut() else { return };
        if st.in_session {
            return;
        }
        let shared: SharedFile = st.read(SHARED).unwrap_or_default();
        let presets: Option<PresetsFile> = st.read(PRESETS);
        st.merged = shared.merged;
        st.in_session = true;
        st.shared_written = Vec::new();
        self.lut_profiles = shared.lut_profiles;
        crate::cmd::lut_profiles::register_all(self);
        self.dcp_profiles = shared.dcp_profiles;
        crate::cmd::dcp_profiles::register_all(self);
        self.mask_effects = shared.mask_effects;
        self.mask_tool_effects = shared.mask_tool_effects;
        let (auto_folder, auto_copy, auto_album) =
            (self.import_defaults.auto_folder.take(), self.import_defaults.auto_copy, self.import_defaults.auto_album.take());
        self.import_defaults = shared.import;
        self.import_defaults.auto_folder = auto_folder;
        self.import_defaults.auto_copy = auto_copy;
        self.import_defaults.auto_album = auto_album;
        self.profile_state = shared.profiles;
        self.import_presets = shared.import_presets;
        self.preset_state = shared.preset_state;
        self.set_presets_from(presets.unwrap_or_default());
        self.shared_presets_saved = crate::library::PresetsSaved::state_of(self);
        // what was just read is on disk already
        let bytes = self.shared_json();
        if let Some(st) = self.app_store.as_mut() {
            st.shared_written = bytes;
        }
    }

    /// The built-in presets plus `f`'s, and its favourites and profile lists.
    pub(crate) fn set_presets_from(&mut self, f: PresetsFile) {
        let mut list = crate::presets::builtin();
        for p in &mut list {
            p.favorite = f.favorites.contains(&p.id);
        }
        let mut ids: HashSet<String> = list.iter().map(|p| p.id.clone()).collect();
        for u in f.user {
            if ids.insert(u.id.clone()) {
                list.push(u);
            }
        }
        crate::preset_manager::apply_builtin_overrides(&mut list, &self.preset_state);
        self.presets = crate::presets::PresetList::new(list);
        let favorites: Vec<String> = f.profile_favorites.into_iter().filter(|id| self.profile_known(id)).collect();
        let recent: Vec<String> = f.profile_recent.into_iter().filter(|id| self.profile_known(id)).take(crate::presets::RECENT_PROFILES).collect();
        self.profile_favorites = favorites;
        self.profile_recent = recent;
    }

    /// `id` names a profile the session has (built-in or imported).
    pub(crate) fn profile_known(&self, id: &str) -> bool {
        crate::presets::profile(id).is_some() || self.lut_profiles.iter().any(|p| p.id == id) || self.dcp_profiles.iter().any(|p| p.id == id)
    }

    fn shared_json(&self) -> Vec<u8> {
        let merged = self.app_store.as_ref().map(|s| s.merged.clone()).unwrap_or_default();
        let f = SharedFile {
            version: SHARED_VERSION,
            lut_profiles: self.lut_profiles.clone(),
            dcp_profiles: self.dcp_profiles.clone(),
            mask_effects: self.mask_effects.clone(),
            mask_tool_effects: self.mask_tool_effects.clone(),
            import: ImportDefaults { auto_folder: None, auto_copy: false, auto_album: None, ..self.import_defaults.clone() },
            profiles: self.profile_state.clone(),
            merged,
            import_presets: self.import_presets.clone(),
            preset_state: self.preset_state.clone(),
        };
        serde_json::to_vec_pretty(&f).unwrap_or_default()
    }

    /// Write `shared.json` when the session's shared settings changed (no-op without the store, or
    /// while the library keeps its own).
    pub(crate) fn save_app_store(&mut self) -> Result<()> {
        if !self.uses_app_store() || !self.app_store.as_ref().is_some_and(|s| s.in_session) {
            return Ok(());
        }
        let bytes = self.shared_json();
        let Some(st) = self.app_store.as_mut() else { return Ok(()) };
        if bytes == st.shared_written {
            return Ok(());
        }
        st.write(SHARED, &bytes)?;
        st.shared_written = bytes;
        Ok(())
    }

    /// Merge library folder `dir` into the store (once per library; `force` merges again). The
    /// session must hold the store's content. Nothing in the library is written.
    pub(crate) fn merge_library_content(&mut self, dir: &Path, content: LibraryContent) -> MergeReport {
        let mut report = MergeReport { library: dir.display().to_string(), ..Default::default() };
        let Some(profiles_dir) = self.app_store.as_ref().map(AppStore::profiles_dir) else {
            return report;
        };
        if let Err(e) = std::fs::create_dir_all(&profiles_dir) {
            report.problems.push(format!("{}: {e}", profiles_dir.display()));
        }
        let LibraryContent { shared, presets } = content;
        let mut remap: BTreeMap<String, String> = BTreeMap::new();
        // camera profiles: the same file once; a name taken by another file gets a new id
        let mut dcp_changed = false;
        for mut e in shared.dcp_profiles {
            crate::cmd::dcp_profiles::migrate(&mut e);
            let bytes = match read_capped(Path::new(&e.file), MAX_PROFILE_FILE) {
                Ok(Some(b)) => Some(b),
                Ok(None) => {
                    report.problems.push(format!("{}: the profile file is missing; kept as listed", e.file));
                    None
                }
                Err(err) => {
                    report.problems.push(err);
                    None
                }
            };
            let same = self.dcp_profiles.iter_mut().find(|p| {
                p.name == e.name && p.camera_model == e.camera_model && bytes.as_ref().is_some_and(|b| file_is(&p.file, b))
                    || (p.id == e.id && bytes.is_none() && p.file == e.file)
            });
            if let Some(p) = same {
                for b in e.bodies {
                    if !p.bodies.contains(&b) {
                        p.bodies.push(b);
                        dcp_changed = true;
                    }
                }
                for b in e.body_defaults {
                    if !p.body_defaults.contains(&b) {
                        p.body_defaults.push(b);
                        dcp_changed = true;
                    }
                }
                if p.id != e.id {
                    remap.insert(e.id.clone(), p.id.clone());
                }
                continue;
            }
            let id = free_id(&e.id, |id| self.dcp_profiles.iter().any(|p| p.id == id));
            if id != e.id {
                remap.insert(e.id.clone(), id.clone());
            }
            if let Some(b) = &bytes {
                let to = free_file(&profiles_dir, id.trim_start_matches(crate::cmd::dcp_profiles::DCP_PREFIX), "dcp", b);
                match std::fs::write(&to, b) {
                    Ok(()) => e.file = to.to_string_lossy().to_string(),
                    Err(err) => report.problems.push(format!("{}: {err}; the library's copy is used", to.display())),
                }
            }
            e.id = id;
            self.dcp_profiles.push(e);
            report.profiles += 1;
            dcp_changed = true;
        }
        // LUT profiles
        let mut lut_changed = false;
        for mut l in shared.lut_profiles {
            let bytes = match read_capped(Path::new(&l.file), MAX_PROFILE_FILE) {
                Ok(b) => b,
                Err(err) => {
                    report.problems.push(err);
                    None
                }
            };
            if bytes.is_none() {
                report.problems.push(format!("{}: the profile file is missing; kept as listed", l.file));
            }
            let same = self.lut_profiles.iter().find(|p| {
                p.name == l.name && p.group == l.group && bytes.as_ref().is_some_and(|b| file_is(&p.file, b)) || (p.id == l.id && p.file == l.file)
            });
            if let Some(p) = same {
                if p.id != l.id {
                    remap.insert(l.id.clone(), p.id.clone());
                }
                continue;
            }
            let id = free_id(&l.id, |id| self.lut_profiles.iter().any(|p| p.id == id));
            if id != l.id {
                remap.insert(l.id.clone(), id.clone());
            }
            if let Some(b) = &bytes {
                let to = free_file(&profiles_dir, id.trim_start_matches("lut:"), "cube", b);
                match std::fs::write(&to, b) {
                    Ok(()) => l.file = to.to_string_lossy().to_string(),
                    Err(err) => report.problems.push(format!("{}: {err}; the library's copy is used", to.display())),
                }
            }
            l.id = id;
            self.lut_profiles.push(l);
            report.profiles += 1;
            lut_changed = true;
        }
        if dcp_changed {
            crate::cmd::dcp_profiles::register_all(self);
        }
        if lut_changed {
            crate::cmd::lut_profiles::register_all(self);
        }
        // local adjustment presets (mask effects): by name and folder
        for mut fx in shared.mask_effects {
            if let Some(have) = self.mask_effects.iter_mut().find(|m| m.name == fx.name && m.group == fx.group) {
                if have.adjust == fx.adjust && have.fields == fx.fields && have.with_amount == fx.with_amount && have.brush == fx.brush {
                    have.favorite |= fx.favorite;
                    continue;
                }
                let base = fx.name.clone();
                let mut n = 2;
                while self.mask_effects.iter().any(|m| m.name == fx.name && m.group == fx.group) && n < 10_000 {
                    fx.name = format!("{base} {n}");
                    n += 1;
                }
            }
            self.mask_effects.push(fx);
            report.effects += 1;
        }
        for (k, v) in shared.mask_tool_effects {
            self.mask_tool_effects.entry(k).or_insert(v);
        }
        // import defaults: a library's own when the store has none yet
        let plain = |d: &ImportDefaults| ImportDefaults { auto_folder: None, auto_copy: false, auto_album: None, ..d.clone() };
        if plain(&self.import_defaults) == ImportDefaults::default() && plain(&shared.import) != ImportDefaults::default() {
            let (f, c, a) = (self.import_defaults.auto_folder.take(), self.import_defaults.auto_copy, self.import_defaults.auto_album.take());
            self.import_defaults = plain(&shared.import);
            self.import_defaults.auto_folder = f;
            self.import_defaults.auto_copy = c;
            self.import_defaults.auto_album = a;
        }
        self.profile_state.merge(shared.profiles, &remap);
        self.preset_state.merge(shared.preset_state);
        // import presets: by name; the same name with other settings is kept as "name 2"
        for mut ip in shared.import_presets {
            if let Some(have) = self.import_presets.iter().find(|x| x.name.eq_ignore_ascii_case(&ip.name)) {
                if have.settings == ip.settings {
                    continue;
                }
                let base = ip.name.clone();
                let mut n = 2;
                while self.import_presets.iter().any(|x| x.name.eq_ignore_ascii_case(&ip.name)) && n < 10_000 {
                    ip.name = format!("{base} {n}");
                    n += 1;
                }
            }
            self.import_presets.push(ip);
        }
        // develop presets: the same preset (name, group, settings) once; a taken id gets a new one
        if let Some(f) = presets {
            report.presets = self.merge_presets(f, &remap);
        }
        if let Some(st) = self.app_store.as_mut() {
            st.merged.retain(|m| !same_path(Path::new(&m.library), dir));
            st.merged.push(Merged {
                library: dir.display().to_string(),
                at: (self.clock)(),
                presets: report.presets,
                profiles: report.profiles,
                effects: report.effects,
                remap,
                remapped: false,
            });
            for p in &report.problems {
                log::warn!("store: merging {}: {p}", dir.display());
            }
        }
        report
    }

    /// Add a library's presets → how many were new.
    fn merge_presets(&mut self, f: PresetsFile, remap: &BTreeMap<String, String>) -> usize {
        let mut ids: HashSet<String> = self.presets.iter().map(|q| q.id.clone()).collect();
        let mut by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (i, q) in self.presets.iter().enumerate() {
            by_key.entry((q.name.clone(), q.group.clone())).or_default().push(i);
        }
        let mut added = 0;
        let mut fav_same: Vec<usize> = Vec::new();
        let mut new: Vec<Preset> = Vec::new();
        for mut p in f.user {
            if p.builtin {
                continue;
            }
            let key = (p.name.clone(), p.group.clone());
            if let Some(i) =
                by_key.get(&key).and_then(|ix| ix.iter().copied().find(|&i| self.presets.get(i).is_some_and(|q| q.settings == p.settings)))
            {
                if p.favorite {
                    fav_same.push(i);
                }
                continue;
            }
            if p.id.trim().is_empty() || p.id.starts_with("lc.") || ids.contains(&p.id) {
                let base = format!("user.{}", crate::presets::slug(&format!("{}-{}", p.group, p.name)));
                p.id = free_id(&base, |id| ids.contains(id));
            }
            ids.insert(p.id.clone());
            by_key.entry(key).or_default().push(self.presets.len() + new.len());
            new.push(p);
            added += 1;
        }
        let builtin_favs: HashSet<String> = f.favorites.into_iter().collect();
        let touches =
            !new.is_empty() || !fav_same.is_empty() || self.presets.iter().any(|p| p.builtin && !p.favorite && builtin_favs.contains(&p.id));
        if touches {
            // (one generation bump for the whole merge)
            let list = &mut self.presets;
            for i in fav_same {
                if let Some(p) = list.get_mut(i) {
                    p.favorite = true;
                }
            }
            for p in list.iter_mut().filter(|p| p.builtin && builtin_favs.contains(&p.id)) {
                p.favorite = true;
            }
            list.extend(new);
        }
        let map = |id: String| remap.get(&id).cloned().unwrap_or(id);
        for id in f.profile_favorites.into_iter().map(map) {
            if self.profile_known(&id) && !self.profile_favorites.contains(&id) {
                self.profile_favorites.push(id);
            }
        }
        for id in f.profile_recent.into_iter().map(map) {
            if self.profile_known(&id) && !self.profile_recent.contains(&id) && self.profile_recent.len() < crate::presets::RECENT_PROFILES {
                self.profile_recent.push(id);
            }
        }
        added
    }

    /// Merge the library folders `dirs` into the store, those not merged yet (`force`: again).
    /// Reads them only. Saves the store.
    pub fn merge_libraries(&mut self, dirs: &[PathBuf], force: bool) -> Result<Vec<MergeReport>> {
        if self.app_store.is_none() {
            return Err(EngineError::Other("there is no shared store (LightCraft's settings folder isn't available)".into()));
        }
        if !self.uses_app_store() {
            return Err(EngineError::Other("the open library keeps its own presets and profiles; switch it to the shared store first".into()));
        }
        self.load_app_store();
        let mut out = Vec::new();
        for dir in dirs {
            if !force && self.app_store.as_ref().is_some_and(|s| s.has_merged(dir)) {
                out.push(MergeReport { library: dir.display().to_string(), skipped: true, ..Default::default() });
                continue;
            }
            if !crate::library::is_library(dir) {
                out.push(MergeReport { library: dir.display().to_string(), problems: vec!["not a LightCraft library".into()], ..Default::default() });
                continue;
            }
            match read_library(dir) {
                Ok(content) => {
                    let r = self.merge_library_content(dir, content);
                    out.push(r);
                }
                Err(e) => out.push(MergeReport { library: dir.display().to_string(), problems: vec![e], ..Default::default() }),
            }
        }
        // the open library's own photos follow ids the store changed
        self.apply_store_remap();
        crate::profile_refs::refresh(self);
        self.save_app_store()?;
        self.save_shared_presets()?;
        Ok(out)
    }

    /// The open library's photos that name a profile id the store changed when it was merged move to
    /// the new id (once; not an edit: no undo step, saved with the library).
    pub(crate) fn apply_store_remap(&mut self) {
        let Some(dir) = self.library.as_ref().filter(|_| self.uses_app_store()).map(|l| l.dir.clone()) else { return };
        let Some(m) = self.app_store.as_ref().and_then(|s| s.merged.iter().find(|m| same_path(Path::new(&m.library), &dir)).cloned()) else {
            return;
        };
        if m.remapped || m.remap.is_empty() {
            return;
        }
        let changes: Vec<(lightcraft_catalog::PhotoId, lightcraft_develop::DevelopSettings, Option<String>)> = self
            .catalog
            .photos()
            .filter_map(|p| {
                let mut d = (*p.develop).clone();
                let mut changed = false;
                if let Some(to) = m.remap.get(&d.camera_profile) {
                    d.camera_profile = to.clone();
                    changed = true;
                }
                if let Some(to) = m.remap.get(&d.profile.id) {
                    d.profile.id = to.clone();
                    changed = true;
                }
                changed.then(|| (p.id, d, p.edited.clone()))
            })
            .collect();
        for (id, d, edited) in changes {
            let op = lightcraft_catalog::Op::SetDevelop { id, settings: std::sync::Arc::new(d), label: "Shared profile".into(), edited };
            if let Err(e) = self.record_quietly(op) {
                log::error!("store: moving photo {} to its profile's new id: {e}", id.0);
            }
        }
        if let Some(st) = self.app_store.as_mut()
            && let Some(rec) = st.merged.iter_mut().find(|r| same_path(Path::new(&r.library), &dir))
        {
            rec.remapped = true;
        }
    }

    /// Save the develop presets to the store when they changed.
    pub(crate) fn save_shared_presets(&mut self) -> Result<()> {
        if !self.uses_app_store() {
            return Ok(());
        }
        // a background save that failed: it is done again now
        if let Some(e) = self.presets_save_failed() {
            log::warn!("store: saving the presets again after: {e}");
            self.shared_presets_saved = crate::library::PresetsSaved::stale();
        }
        if !self.shared_presets_saved.differs(self) || self.app_store.as_ref().is_some_and(|s| s.blocks(PRESETS)) {
            return Ok(());
        }
        // off the UI thread when it can be (`presets_save`): the bytes are made and compared there
        if let Some(st) = self.app_store.take() {
            let known = self.shared_presets_saved.hash;
            let handed = self.save_presets_in_background(st.store(), known);
            self.app_store = Some(st);
            if handed {
                let mut state = crate::library::PresetsSaved::state_of(self);
                state.hash = known;
                self.shared_presets_saved = state;
                return Ok(());
            }
        }
        let json = crate::library::presets_json(self);
        let mut state = crate::library::PresetsSaved::state_of(self);
        state.hash = crate::library::bytes_hash(json.as_bytes());
        if state.hash == self.shared_presets_saved.hash {
            self.shared_presets_saved = state;
            return Ok(());
        }
        let Some(st) = self.app_store.as_mut() else { return Ok(()) };
        st.write_presets(json.as_bytes())?;
        self.shared_presets_saved = state;
        Ok(())
    }

    /// `store.info`: where the shared settings live and what they hold.
    pub fn store_info(&self) -> serde_json::Value {
        let st = self.app_store.as_ref();
        serde_json::json!({
            "available": st.is_some(),
            "dir": st.map(|s| s.dir.display().to_string()),
            "inUse": self.uses_app_store(),
            "libraryKeepsOwn": self.library.as_ref().is_some_and(|l| l.own_store),
            "presets": self.presets.iter().filter(|p| !p.builtin).count(),
            "cameraProfiles": self.dcp_profiles.len(),
            "lutProfiles": self.lut_profiles.len(),
            "effects": self.mask_effects.len(),
            "merged": st.map(|s| s.merged.clone()).unwrap_or_default(),
        })
    }
}
