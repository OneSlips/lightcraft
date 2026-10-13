//! Camera profiles at import (LR-PROF-IMPORT-APPLY): which camera profile each imported raw
//! develops with, decided per camera body.
//!
//! - **Automatic** (the default): each body's own profile ([`dp::body_default_for`]), else
//!   LightCraft's own colour, as [`dp::auto_pick`] does for every import.
//! - **Per body** (`{"perBody": {bodyKey: profileId | "builtin"}}`): this import only; the bodies not
//!   named stay automatic.
//! - **One profile for all photos** (`{"all": profileId | "builtin"}`): every raw of the profile's
//!   camera (a profile made for another camera model is never put on a photo).
//!
//! A Lightroom XMP sidecar (or a raw's embedded XMP) with develop settings or a camera profile wins
//! over all of these: the photo keeps what it says ("kept from sidecar"). Photos already in the
//! catalog are never changed (duplicates are skipped by the import). A choice made here is an
//! explicit one: [`dp::auto_pick`] never replaces it later. [`plan`] is the review's summary, built
//! from the scan's candidates without reading any file again.

use std::collections::BTreeMap;

use lightcraft_catalog::Photo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Session;
use crate::cmd::bad;
use crate::cmd::dcp_profiles as dp;
use crate::import::ImportCandidate;

/// LightCraft's own colour, as a profile choice.
pub const BUILTIN: &str = "builtin";

/// `cameraProfile` of `library.import` / `import.profilePlan`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ProfileChoice {
    #[default]
    Auto,
    /// One profile (or [`BUILTIN`]) for every raw.
    All(String),
    /// body key → profile id (or [`BUILTIN`]); other bodies are automatic.
    PerBody(BTreeMap<String, String>),
}

impl ProfileChoice {
    /// Parse and check `cameraProfile` (`"auto"`, `{"all": id}` or `{"perBody": {key: id}}`).
    pub fn parse(s: &Session, v: Option<&Value>, c: &str) -> crate::Result<Self> {
        let check = |id: &str| -> crate::Result<String> {
            let id = id.trim();
            if id == BUILTIN || s.dcp_profiles.iter().any(|e| e.id == id) {
                Ok(id.to_string())
            } else {
                Err(bad(c, format!("unknown camera profile `{id}` (a profile id from profiles.camera, or `builtin`)")))
            }
        };
        match v {
            None | Some(Value::Null) => Ok(ProfileChoice::Auto),
            Some(Value::String(x)) if x == "auto" => Ok(ProfileChoice::Auto),
            Some(Value::Object(o)) => {
                if let Some(id) = o.get("all") {
                    let id = id.as_str().ok_or_else(|| bad(c, "`cameraProfile.all` is a profile id or `builtin`"))?;
                    return Ok(ProfileChoice::All(check(id)?));
                }
                let Some(m) = o.get("perBody").and_then(Value::as_object) else {
                    return Err(bad(c, "`cameraProfile` is \"auto\", {\"all\": profileId} or {\"perBody\": {bodyKey: profileId | \"builtin\"}}"));
                };
                let mut out = BTreeMap::new();
                for (k, id) in m.iter().take(10_000) {
                    let id = id.as_str().ok_or_else(|| bad(c, format!("`cameraProfile.perBody.{k}` is a profile id or `builtin`")))?;
                    let k = k.trim();
                    if !k.is_empty() {
                        out.insert(k.to_string(), check(id)?);
                    }
                }
                Ok(ProfileChoice::PerBody(out))
            }
            Some(_) => Err(bad(c, "`cameraProfile` is \"auto\", {\"all\": profileId} or {\"perBody\": {bodyKey: profileId | \"builtin\"}}")),
        }
    }

    /// The profile chosen for photos of body `body` (`None`: automatic).
    pub fn for_body(&self, body: Option<&str>) -> Option<&str> {
        match self {
            ProfileChoice::Auto => None,
            ProfileChoice::All(id) => Some(id),
            ProfileChoice::PerBody(m) => body.and_then(|b| m.get(b.trim())).map(String::as_str),
        }
    }

    /// As params (`"auto"`, `{"all": id}`, `{"perBody": {…}}`).
    pub fn to_json(&self) -> Value {
        match self {
            ProfileChoice::Auto => json!("auto"),
            ProfileChoice::All(id) => json!({"all": id}),
            ProfileChoice::PerBody(m) => json!({"perBody": m}),
        }
    }
}

/// A photo's body key from its metadata and the name of the file it was imported from (its
/// serial, else the file name's key: [`crate::chart_profiling::body_key`]).
pub fn body_of(serial: &str, source_path: &str) -> Option<String> {
    let name = std::path::Path::new(source_path).file_name().and_then(|n| n.to_str());
    crate::chart_profiling::body_key(Some(serial), name)
}

/// What a photo of `camera` and `body` gets: a profile id (`None`: LightCraft's own colour) and how
/// (`auto`, `chosen` for a per-body choice, `all`).
pub fn resolve(s: &Session, choice: &ProfileChoice, camera: &str, body: Option<&str>) -> (Option<String>, &'static str) {
    let how = if matches!(choice, ProfileChoice::All(_)) { "all" } else { "chosen" };
    match choice.for_body(body) {
        Some(BUILTIN) => (None, how),
        Some(id) => match s.dcp_profiles.iter().find(|e| e.id == id) {
            // (a profile made for another camera model never goes on a photo)
            Some(e) if e.camera_model.trim().is_empty() || dp::same_camera(e, camera) => (Some(id.to_string()), how),
            _ => (auto(s, camera, body), "auto"),
        },
        None => (auto(s, camera, body), "auto"),
    }
}

fn auto(s: &Session, camera: &str, body: Option<&str>) -> Option<String> {
    body.and_then(|b| dp::body_default_for(s, camera, b))
}

/// What happened to one imported photo's camera profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    /// Not a raw: camera profiles don't apply.
    NotRaw,
    /// Its XMP sidecar set its develop settings or camera profile.
    Sidecar,
    /// It develops with this profile (`None`: LightCraft's own colour).
    Profile(Option<String>),
}

/// Give freshly imported photo `p` (already through [`crate::import::apply_import_defaults`],
/// which made the automatic choice, and its sidecar) the profile `choice` names for its body.
pub fn apply(s: &Session, p: &mut Photo, choice: &ProfileChoice, body: Option<&str>, from_sidecar: bool) -> Applied {
    if !p.develops_raw() {
        return Applied::NotRaw;
    }
    if from_sidecar {
        return Applied::Sidecar;
    }
    let (profile, how) = resolve(s, choice, &p.meta.camera, body);
    if how != "auto" {
        let mut d = (*p.develop).clone();
        dp::choose(s, &mut d, profile.as_deref().unwrap_or(lightcraft_develop::BUILTIN_CAMERA_COLOR));
        p.develop = std::sync::Arc::new(d);
    }
    let cal = p.develop.calibration_id().filter(|c| dp::is_dcp_id(c)).map(str::to_string);
    Applied::Profile(cal)
}

/// Whether a sidecar sets what [`apply`] leaves alone: develop settings or a camera profile.
pub fn sidecar_sets(sc: &crate::sidecar::SidecarData) -> bool {
    sc.develop.is_some() || sc.camera_profile.is_some()
}

/// The counts an import reports: photos per profile (`builtin` for LightCraft's own colour) with
/// the bodies they came from, and the photos whose sidecar decided.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileCounts {
    pub profiles: BTreeMap<String, ProfileCount>,
    pub kept_from_sidecar: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileCount {
    pub name: String,
    pub photos: usize,
    pub bodies: Vec<String>,
}

impl ProfileCounts {
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.kept_from_sidecar == 0
    }

    pub fn add(&mut self, s: &Session, applied: &Applied, body: Option<&str>) {
        match applied {
            Applied::NotRaw => {}
            Applied::Sidecar => self.kept_from_sidecar += 1,
            Applied::Profile(id) => {
                let key = id.clone().unwrap_or_else(|| BUILTIN.to_string());
                let e = self.profiles.entry(key).or_insert_with(|| ProfileCount { name: profile_name(s, id.as_deref()), ..Default::default() });
                e.photos += 1;
                if let Some(b) = body
                    && !e.bodies.iter().any(|x| x == b)
                {
                    e.bodies.push(b.to_string());
                }
            }
        }
    }

    /// Add another batch's counts (the app commits an import in batches).
    pub fn merge(&mut self, other: &ProfileCounts) {
        self.kept_from_sidecar += other.kept_from_sidecar;
        for (k, v) in &other.profiles {
            let e = self.profiles.entry(k.clone()).or_insert_with(|| ProfileCount { name: v.name.clone(), ..Default::default() });
            e.photos += v.photos;
            for b in &v.bodies {
                if !e.bodies.contains(b) {
                    e.bodies.push(b.clone());
                }
            }
        }
    }
}

/// A camera profile's name (`None`: LightCraft's own colour, named by the UI).
pub fn profile_name(s: &Session, id: Option<&str>) -> String {
    match id {
        Some(id) => s.dcp_profiles.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string()),
        None => String::new(),
    }
}

/// One row of the review's summary: the raws of one camera body about to be imported.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRow {
    pub camera: String,
    /// Its key (serial, else `file:XX`); `None`: the files say nothing about the body.
    pub body: Option<String>,
    /// As the UI shows it (`…71`, `SA`).
    pub body_label: String,
    /// The profile it will develop with (`None`: LightCraft's own colour).
    pub profile: Option<String>,
    pub profile_name: String,
    /// `auto`, `chosen` (per body) or `all`.
    pub how: &'static str,
    /// The profile is the body's own (listed for it).
    pub own: bool,
    /// Raws of the body to import, of which `from_sidecar` keep their sidecar's settings.
    pub photos: usize,
    pub from_sidecar: usize,
}

/// The summary for `candidates` (only new raws count: duplicates, unreadable files and other kinds
/// are left out), one row per camera and body, most photos first.
pub fn plan(s: &Session, candidates: &[&ImportCandidate], choice: &ProfileChoice) -> Vec<PlanRow> {
    let mut rows: BTreeMap<(String, Option<String>), PlanRow> = BTreeMap::new();
    for c in candidates {
        if c.kind != lightcraft_catalog::MediaKind::Raw || c.duplicate.is_some() || c.error.is_some() || c.preview_only.is_some() {
            continue;
        }
        let key = (c.camera.trim().to_string(), c.body.clone());
        let row = rows.entry(key).or_insert_with(|| {
            let (profile, how) = resolve(s, choice, &c.camera, c.body.as_deref());
            let own = match (&profile, &c.body) {
                (Some(id), Some(b)) => s.dcp_profiles.iter().any(|e| &e.id == id && e.bodies.iter().any(|x| x == b)),
                _ => false,
            };
            PlanRow {
                camera: c.camera.trim().to_string(),
                body_label: c.body.as_deref().map(dp::body_label).unwrap_or_default(),
                body: c.body.clone(),
                profile_name: profile_name(s, profile.as_deref()),
                profile,
                how,
                own,
                ..Default::default()
            }
        });
        row.photos += 1;
        if c.sidecar_develop {
            row.from_sidecar += 1;
        }
    }
    let mut rows: Vec<PlanRow> = rows.into_values().collect();
    rows.sort_by(|a, b| b.photos.cmp(&a.photos).then_with(|| a.camera.cmp(&b.camera)).then_with(|| a.body.cmp(&b.body)));
    rows
}

/// `import.profilePlan`'s answer for rows.
pub fn plan_json(rows: &[PlanRow]) -> Value {
    let photos: usize = rows.iter().map(|r| r.photos).sum();
    let kept: usize = rows.iter().map(|r| r.from_sidecar).sum();
    json!({"rows": rows, "photos": photos, "keptFromSidecar": kept})
}

/// The camera profiles a photo of `camera` and `body` can be given at import, for the review's
/// Choose… menu: (this body's own, model-wide, other bodies'), each `(id, name)`.
pub type Choices = (Vec<(String, String)>, Vec<(String, String)>, Vec<(String, String, String)>);

pub fn choices(s: &Session, camera: &str, body: Option<&str>) -> Choices {
    let (mut mine, mut model, mut others) = (Vec::new(), Vec::new(), Vec::new());
    for e in s.dcp_profiles.iter().filter(|e| e.camera_model.trim().is_empty() || dp::same_camera(e, camera)) {
        match dp::relation(e, body) {
            "body" => mine.push((e.id.clone(), e.name.clone())),
            "otherBody" => {
                let label = e.bodies.iter().map(|b| dp::body_label(b)).collect::<Vec<_>>().join(", ");
                others.push((e.id.clone(), e.name.clone(), label));
            }
            _ => model.push((e.id.clone(), e.name.clone())),
        }
    }
    (mine, model, others)
}

/// Whether the XMP an import will read for this raw (its sidecar, else the XMP inside it) sets
/// develop settings or a camera profile: the review counts it as "kept from sidecar".
pub fn sidecar_decides(path: &str, info: &crate::media::ProbeInfo, naming: crate::sidecar::SidecarNaming) -> bool {
    if info.kind != lightcraft_catalog::MediaKind::Raw {
        return false;
    }
    let packet = crate::sidecar::find_sidecar(path, naming)
        .filter(|f| std::fs::metadata(f).is_ok_and(|m| m.len() <= 16 << 20))
        .and_then(|f| std::fs::read_to_string(f).ok())
        .or_else(|| info.xmp.clone());
    let target = crate::crs::Target::for_file(info.kind, &info.format, info.preview_only.is_some(), &info.meta.camera);
    packet.and_then(|x| crate::sidecar::parse_sidecar(&x, target).ok()).is_some_and(|sc| sidecar_sets(&sc))
}

/// After an import: the bodies in `adopt` take the profile chosen for them (`profile.adoptForBody`,
/// its own undo step each). Returns `[{body, profile}]`.
pub fn adopt_after_import(s: &mut Session, p: &Value) -> crate::Result<Vec<Value>> {
    let c = "library.import";
    let choice = ProfileChoice::parse(s, p.get("cameraProfile"), c)?;
    let mut out = Vec::new();
    for body in adopt_param(&choice, p, c)? {
        let Some(id) = choice.for_body(Some(&body)).map(str::to_string) else { continue };
        s.execute("profile.adoptForBody", &json!({"profile": id, "serial": body}))?;
        out.push(json!({"body": body, "profile": id}));
    }
    Ok(out)
}

/// `adopt`: body keys, each given a camera profile (not LightCraft's colour) by `cameraProfile.perBody`.
pub fn adopt_param(choice: &ProfileChoice, p: &Value, c: &str) -> crate::Result<Vec<String>> {
    let Some(list) = p.get("adopt").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = list.as_array().ok_or_else(|| bad(c, "`adopt` is a list of body keys"))?;
    let mut out = Vec::new();
    for b in list.iter().take(10_000) {
        let b = b.as_str().map(str::trim).filter(|b| !b.is_empty()).ok_or_else(|| bad(c, "`adopt` is a list of body keys"))?;
        match (choice, choice.for_body(Some(b))) {
            (ProfileChoice::PerBody(_), Some(id)) if id != BUILTIN => out.push(b.to_string()),
            _ => return Err(bad(c, format!("`adopt`: body `{b}` needs a camera profile chosen for it in `cameraProfile.perBody`"))),
        }
    }
    Ok(out)
}

/// `import.profilePlan`: scan `paths` as the import review does and summarise the camera profiles
/// their raws would get.
pub fn plan_command(s: &mut Session, p: &Value) -> crate::Result<Value> {
    let c = "import.profilePlan";
    let paths: Vec<String> =
        p.get("paths").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if paths.is_empty() {
        return Err(bad(c, "no paths"));
    }
    let choice = ProfileChoice::parse(s, p.get("cameraProfile"), c)?;
    let cands = crate::import::scan(s, &paths);
    let refs: Vec<&ImportCandidate> = cands.iter().collect();
    let rows = plan(s, &refs, &choice);
    let mut out = plan_json(&rows);
    out["duplicates"] = json!(cands.iter().filter(|c| c.duplicate.is_some()).count());
    Ok(out)
}
