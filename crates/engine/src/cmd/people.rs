//! `people.*`: who is on a photo, by name and by role, for masks and presets ("the Bride's
//! dress", "the Groom's suit"). Faces are found with MediaPipe's BlazeFace and FaceMesh (open
//! models) on the AI worker; names live on the photos' Face regions (as upstream's People view
//! keeps them); roles and appearance examples in the library's `people.json`
//! ([`crate::people_store`]). Nothing leaves the computer; `people.forget` / `people.forgetAll`
//! delete the biometric data.
#![cfg_attr(not(feature = "sam"), allow(dead_code, unused_variables))]

use lightcraft_catalog::{Op, PhotoId};
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, str_param};
use crate::people_store::Scope;
use crate::{Result, Session};

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// The photo a command is about: `id`, else the active photo.
fn photo_param(s: &Session, p: &Value, c: &str) -> Result<PhotoId> {
    let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active()).ok_or_else(|| bad(c, "no photo (give `id`)"))?;
    s.catalog.photo(id).ok_or_else(|| bad(c, format!("no photo {}", id.0)))?;
    Ok(id)
}

fn name_param(p: &Value, key: &str, c: &str) -> Result<String> {
    let n = str_param(p, key).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, format!("missing `{key}`")))?;
    if n.chars().count() > 200 {
        return Err(bad(c, format!("`{key}` is too long")));
    }
    Ok(n.to_string())
}

fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let union = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0) + (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// The photo's Face region that is this face (overlapping it, or around its centre).
fn region_of(meta: &lightcraft_catalog::Meta, rect: [f64; 4]) -> Option<usize> {
    let c = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    meta.regions
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == lightcraft_meta::RegionKind::Face)
        .map(|(i, r)| {
            let b = [r.rect.x0, r.rect.y0, r.rect.x1, r.rect.y1];
            let inside = (b[0]..=b[2]).contains(&c[0]) && (b[1]..=b[3]).contains(&c[1]);
            (i, iou(b, rect).max(if inside { 0.3 } else { 0.0 }))
        })
        .filter(|(_, s)| *s >= 0.3)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// A scope from `scope`: `"folder"`/`"current"` (the photo's folder, the default), `{folder}`,
/// `{album}`, `{from, to}`, `{ids}`.
fn scope_param(s: &Session, p: &Value, photo: Option<PhotoId>, c: &str) -> Result<Scope> {
    let v = p.get("scope").cloned().unwrap_or(Value::Null);
    let folder_of = |id: PhotoId| s.photo_context(id).folder;
    Ok(match &v {
        Value::Null => Scope::Folder(photo.and_then(folder_of).ok_or_else(|| bad(c, "the photo has no folder: give `scope`"))?),
        Value::String(k) if matches!(k.as_str(), "folder" | "current") => {
            Scope::Folder(photo.and_then(folder_of).ok_or_else(|| bad(c, "the photo has no folder: give `scope`"))?)
        }
        Value::Object(o) => {
            if let Some(f) = o.get("folder").and_then(Value::as_str) {
                Scope::Folder(crate::people_store::norm_folder(f))
            } else if let Some(a) = o.get("album").and_then(Value::as_u64) {
                Scope::Album(a)
            } else if let (Some(f), Some(t)) = (o.get("from").and_then(Value::as_str), o.get("to").and_then(Value::as_str)) {
                let d = |x: &str| x.get(..10).unwrap_or(x).to_string();
                Scope::Dates { from: d(f), to: d(t) }
            } else if let Some(ids) = o.get("ids").and_then(Value::as_array) {
                Scope::Photos(ids.iter().filter_map(Value::as_u64).take(100_000).collect())
            } else {
                return Err(bad(c, "`scope` must be \"folder\", {folder}, {album}, {from, to} or {ids}"));
            }
        }
        _ => return Err(bad(c, "`scope` must be \"folder\", {folder}, {album}, {from, to} or {ids}")),
    })
}

fn scope_json(sc: &Scope) -> Value {
    serde_json::to_value(sc).unwrap_or(Value::Null)
}

/// Rename (or clear, `to: None`) every Face region named `from` across the catalog: the ops.
fn rename_ops(s: &Session, from: &str, to: Option<&str>) -> Vec<Op> {
    s.catalog
        .photos()
        .filter(|p| p.meta.regions.iter().any(|r| r.name.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(from.trim()))))
        .map(|p| {
            let mut meta = p.meta.clone();
            for r in &mut meta.regions {
                if r.kind == lightcraft_meta::RegionKind::Face && r.name.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(from.trim())) {
                    r.name = to.map(str::to_string);
                }
            }
            Op::SetMeta { id: p.id, meta: Box::new(meta) }
        })
        .collect()
}

#[cfg(feature = "sam")]
mod faces {
    use super::*;
    use crate::segment::worker::{FoundFace, Kind, Payload};
    use lightcraft_segment::identity::{self, Signature};

    /// Suggestions need this score (the appearance model's probability of the same person).
    pub const SUGGEST: f32 = 0.6;

    /// The faces on photo `id`: from the cache, else computed (waiting) or queued (the app:
    /// `None` until they are found; the UI asks again when `segment_poll` reports a change).
    pub fn faces_on(s: &mut Session, id: PhotoId, c: &str) -> Result<Option<Vec<FoundFace>>> {
        let key = s.segment_input_key(id).ok_or_else(|| bad(c, "no such photo"))?;
        if let Some((k, f)) = s.segmenter.smart.faces.get(&id)
            && *k == key
        {
            return Ok(Some(f.clone()));
        }
        let face = s.segmenter.require_model("face").map_err(crate::EngineError::Other)?;
        let people = s.segmenter.require_model("people").ok();
        let sam2 = s.segmenter.require_model("sam2").ok();
        let kind = Kind::Faces { face, people, sam2 };
        let tag = crate::segment::Tag::Faces { photo: id, key };
        if s.segmenter.background {
            if s.segmenter.smart.faces_pending.insert(id) {
                s.segment_queue_kind(id, kind, tag).map_err(crate::EngineError::Other)?;
            }
            return Ok(None);
        }
        let o = s.segment_wait_outcome(id, kind, tag).map_err(crate::EngineError::Other)?;
        o.result.map_err(crate::EngineError::Other)?;
        let faces = match o.payload {
            Payload::Faces(f) => f,
            _ => Vec::new(),
        };
        s.segmenter.smart.faces.insert(id, (key, faces.clone()));
        Ok(Some(faces))
    }

    /// Everyone named, with their remembered appearance as it is in photo `id`'s shoot: the
    /// examples from its folder (appearance — clothes above all — only holds within a shoot), or,
    /// when nobody was named there yet, every example.
    pub fn known(s: &mut Session, id: PhotoId) -> Vec<(String, Vec<Signature>)> {
        let folder = s.photo_context(id).folder;
        let store = s.people_store().clone();
        let parse = |e: &crate::people_store::Example| serde_json::from_value::<Signature>(e.signature.clone()).ok();
        let here: Vec<(String, Vec<Signature>)> = store
            .people
            .iter()
            .map(|p| {
                let sigs =
                    p.examples.iter().filter(|e| folder.is_some() && s.photo_context(PhotoId(e.photo)).folder == folder).filter_map(parse).collect();
                (p.name.clone(), sigs)
            })
            .filter(|(_, v): &(String, Vec<Signature>)| !v.is_empty())
            .collect();
        if !here.is_empty() {
            return here;
        }
        store
            .people
            .iter()
            .map(|p| (p.name.clone(), p.examples.iter().filter_map(parse).collect::<Vec<_>>()))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }

    /// Who the unnamed faces of a photo are: each person at most once, the most alike first.
    /// `(face index, index into known, score)`.
    pub fn assign(faces: &[FoundFace], skip: &[usize], known: &[(String, Vec<Signature>)], threshold: f32) -> Vec<(usize, usize, f32)> {
        let person_score = |sig: &Signature, sigs: &[Signature]| {
            let mut v: Vec<f32> = sigs.iter().map(|q| identity::similarity(sig, q)).collect();
            v.sort_by(|a, b| b.total_cmp(a));
            let top: Vec<f32> = v.into_iter().take(2).collect();
            if top.is_empty() { 0.0 } else { top.iter().sum::<f32>() / top.len() as f32 }
        };
        let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
        for (i, f) in faces.iter().enumerate().filter(|(i, _)| !skip.contains(i)) {
            for (k, (_, sigs)) in known.iter().enumerate() {
                let sc = person_score(&f.signature, sigs);
                if sc >= threshold {
                    pairs.push((i, k, sc));
                }
            }
        }
        pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
        let mut out: Vec<(usize, usize, f32)> = Vec::new();
        for (i, k, sc) in pairs {
            if out.iter().any(|(fi, fk, _)| *fi == i || *fk == k) {
                continue;
            }
            out.push((i, k, sc));
        }
        out
    }

    /// The JSON of photo `id`'s faces: box, the person named there, their roles in this shoot, a
    /// suggestion for unnamed faces.
    pub fn describe(s: &mut Session, id: PhotoId, faces: &[FoundFace]) -> Vec<Value> {
        let meta = s.catalog.photo(id).map(|p| p.meta.clone()).unwrap_or_default();
        let ctx = s.photo_context(id);
        let known = known(s, id);
        let named: Vec<usize> = (0..faces.len())
            .filter(|i| faces.get(*i).is_some_and(|f| region_of(&meta, f.rect).and_then(|r| meta.regions.get(r)).is_some_and(|r| r.name.is_some())))
            .collect();
        let picks = assign(faces, &named, &known, SUGGEST);
        faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let region = region_of(&meta, f.rect);
                let person = region.and_then(|r| meta.regions.get(r)).and_then(|r| r.name.clone()).filter(|n| !n.trim().is_empty());
                let mut v = json!({"index": i, "rect": f.rect, "score": f.score, "profile": !f.landmarks, "region": region});
                if let Some(n) = &person {
                    v["person"] = json!(n);
                    v["roles"] = json!(s.people_store().person_roles(n, &ctx));
                } else if let Some((_, k, score)) = picks.iter().find(|p| p.0 == i)
                    && let Some((name, _)) = known.get(*k)
                {
                    v["suggestion"] = json!({"name": name, "score": score, "roles": s.people_store().person_roles(name, &ctx)});
                }
                v
            })
            .collect()
    }

    /// Name faces (`[(photo, face index)]`) as `name`: their Face regions (made when missing) and
    /// the appearance examples. → (faces named, photos).
    pub fn name(s: &mut Session, faces: &[(PhotoId, usize)], name: &str, c: &str) -> Result<(usize, usize)> {
        let mut by_photo: std::collections::BTreeMap<u64, Vec<usize>> = Default::default();
        for (p, i) in faces {
            by_photo.entry(p.0).or_default().push(*i);
        }
        let (mut ops, mut named, mut examples) = (Vec::new(), 0usize, Vec::new());
        for (photo, idx) in by_photo {
            let id = PhotoId(photo);
            let Some(found) = faces_on(s, id, c)? else { return Err(bad(c, "the faces are still being found: try again in a moment")) };
            let Some(p) = s.catalog.photo(id) else { continue };
            let mut meta = p.meta.clone();
            for i in idx {
                let Some(f) = found.get(i) else { return Err(bad(c, format!("photo {photo} has no face {i}"))) };
                match region_of(&meta, f.rect) {
                    Some(r) => {
                        if let Some(reg) = meta.regions.get_mut(r) {
                            reg.name = Some(name.to_string());
                            if reg.description.as_deref().is_some_and(|d| d.starts_with(crate::cmd::face_detect::MARK)) {
                                reg.description = None;
                            }
                        }
                    }
                    None => meta.regions.push(lightcraft_meta::Region {
                        rect: lightcraft_geom::Rect { x0: f.rect[0], y0: f.rect[1], x1: f.rect[2], y1: f.rect[3] },
                        kind: lightcraft_meta::RegionKind::Face,
                        name: Some(name.to_string()),
                        description: None,
                    }),
                }
                named += 1;
                examples.push(crate::people_store::Example {
                    photo,
                    rect: f.rect,
                    signature: serde_json::to_value(&f.signature).unwrap_or(Value::Null),
                });
            }
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
        let photos = ops.len();
        if ops.is_empty() {
            return Err(bad(c, "no faces to name"));
        }
        s.commit(if named == 1 { "Name Face" } else { "Name Faces" }, Op::Batch { ops })?;
        s.skip_auto_write = true;
        s.with_people(|st| examples.into_iter().for_each(|e| st.add_example(name, e))).map_err(crate::EngineError::Other)?;
        Ok((named, photos))
    }
}

/// Faces on the photos `ids` (computing or queueing them): `(ready, pending photos)`.
#[cfg(feature = "sam")]
fn faces_of(s: &mut Session, ids: &[PhotoId], c: &str) -> Result<(Vec<(PhotoId, Vec<crate::segment::worker::FoundFace>)>, usize)> {
    let (mut ready, mut pending) = (Vec::new(), 0usize);
    for id in ids {
        match faces::faces_on(s, *id, c)? {
            Some(f) => ready.push((*id, f)),
            None => pending += 1,
        }
    }
    Ok((ready, pending))
}

impl Session {
    /// The faces on photo `id` as `people.faces` describes them, for the Masking panel's People
    /// picker (no journal entry): `None` while they are being found (in the background).
    pub fn faces_here(&mut self, id: PhotoId) -> std::result::Result<Option<Vec<Value>>, String> {
        #[cfg(feature = "sam")]
        {
            match faces::faces_on(self, id, "people.faces").map_err(|e| e.to_string())? {
                Some(f) => Ok(Some(faces::describe(self, id, &f))),
                None => Ok(None),
            }
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = id;
            Err("finding faces needs the AI models".into())
        }
    }

    /// Names (people named on photos or remembered) and roles, for autocompletion.
    pub fn people_names(&mut self) -> (Vec<String>, Vec<String>) {
        let mut names: Vec<String> = self.catalog.people().into_iter().map(|p| p.name).collect();
        let store = self.people_store().clone();
        for p in &store.people {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&p.name)) {
                names.push(p.name.clone());
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        let mut roles: Vec<String> = store.roles.iter().map(|b| b.role.clone()).collect();
        for r in ["Bride", "Groom"] {
            roles.push(r.into());
        }
        roles.sort_by_key(|r| r.to_lowercase());
        roles.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        (names, roles)
    }
}

#[cfg_attr(feature = "sam", allow(dead_code))]
fn no_ai(c: &str) -> crate::EngineError {
    bad(c, "finding faces needs the AI models, which this build doesn't have (use the desktop app)")
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "people.faces",
            "Faces on This Photo",
            [],
            None,
            "{id?, signatures?: bool} — the faces MediaPipe finds on the photo (left to right): {faces: [{index, rect [x0,y0,x1,y1] normalized, person?, roles?, suggestion?: {name, score, roles}, profile}], pending?} — in the app `pending: true` until they are found (ask again)",
            always,
            |s, p| {
                let c = "people.faces";
                let id = photo_param(s, p, c)?;
                #[cfg(feature = "sam")]
                {
                    match faces::faces_on(s, id, c)? {
                        Some(f) => {
                            let mut faces = faces::describe(s, id, &f);
                            // (tools tuning recognition: the appearance signatures themselves)
                            if bool_or(p, "signatures", false) {
                                for (v, ff) in faces.iter_mut().zip(&f) {
                                    v["signature"] = serde_json::to_value(&ff.signature).unwrap_or(Value::Null);
                                }
                            }
                            Ok(json!({"id": id.0, "faces": faces}))
                        }
                        None => Ok(json!({"id": id.0, "faces": [], "pending": true})),
                    }
                }
                #[cfg(not(feature = "sam"))]
                {
                    let _ = id;
                    Err(no_ai(c))
                }
            }
        ),
        cmd!(
            "people.name",
            "Name Person",
            [],
            None,
            "{id?, face: index (from people.faces) | faces: [{photo, face}], name, role?: \"Bride\" (also make them that role in this photo's folder)} — names the faces (their Face regions, as the People view shows them) and remembers how they look, so masks and presets find them on other photos; one undo step → {named, photos}",
            always,
            |s, p| {
                let c = "people.name";
                let name = name_param(p, "name", c)?;
                #[cfg(feature = "sam")]
                {
                    let list: Vec<(PhotoId, usize)> = match p.get("faces").and_then(Value::as_array) {
                        Some(a) => a
                            .iter()
                            .take(5000)
                            .filter_map(|f| Some((PhotoId(f.get("photo")?.as_u64()?), usize::try_from(f.get("face")?.as_u64()?).ok()?)))
                            .collect(),
                        None => {
                            let id = photo_param(s, p, c)?;
                            let i = p
                                .get("face")
                                .and_then(Value::as_u64)
                                .and_then(|v| usize::try_from(v).ok())
                                .ok_or_else(|| bad(c, "missing `face`"))?;
                            vec![(id, i)]
                        }
                    };
                    let (named, photos) = faces::name(s, &list, &name, c)?;
                    if let Some(role) = str_param(p, "role").map(str::trim).filter(|r| !r.is_empty()) {
                        let first = list.first().map(|f| f.0);
                        let scope = scope_param(s, p, first, c)?;
                        s.with_people(|st| st.set_role(role, &name, scope)).map_err(crate::EngineError::Other)?.map_err(|e| bad(c, e))?;
                    }
                    Ok(json!({"named": named, "photos": photos, "name": name}))
                }
                #[cfg(not(feature = "sam"))]
                {
                    let _ = name;
                    Err(no_ai(c))
                }
            }
        ),
        cmd!(
            query "people.list",
            "People",
            [],
            None,
            "→ {people: [{name, photos, photo, face, examples, roles: [{role, scope}]}]}: everyone named (Face regions) or remembered",
            always,
            |s, _| {
                let named = s.catalog.people();
                let store = s.people_store().clone();
                let mut out: Vec<Value> = named
                    .iter()
                    .map(|n| {
                        let ex = store.person(&n.name).map_or(0, |d| d.examples.len());
                        let roles: Vec<Value> = store
                            .roles
                            .iter()
                            .filter(|b| b.person.eq_ignore_ascii_case(&n.name))
                            .map(|b| json!({"role": b.role, "scope": scope_json(&b.scope)}))
                            .collect();
                        json!({"name": n.name, "photos": n.count, "photo": n.photo.0, "face": [n.face.x0, n.face.y0, n.face.x1, n.face.y1], "examples": ex, "roles": roles})
                    })
                    .collect();
                for d in &store.people {
                    if !named.iter().any(|n| n.name.eq_ignore_ascii_case(&d.name)) {
                        out.push(json!({"name": d.name, "photos": 0, "examples": d.examples.len(), "roles": []}));
                    }
                }
                Ok(json!({"people": out}))
            }
        ),
        cmd!(
            "people.recognize",
            "Recognize People",
            [],
            None,
            "{ids?, apply?: bool (false), threshold?: 0..1} — find the faces on the photos and match them to the people you named (by appearance; within a shoot clothes and hair count most): {suggestions: [{photo, face, name, score}], named, pending} — `apply` names the confident ones (one undo step); in the app `pending` photos are still being analyzed (ask again)",
            always,
            |s, p| {
                let c = "people.recognize";
                let ids = s.targets(p);
                if ids.len() > 20_000 {
                    return Err(bad(c, "at most 20000 photos at a time"));
                }
                #[cfg(feature = "sam")]
                {
                    let threshold = p.get("threshold").and_then(Value::as_f64).map_or(faces::SUGGEST, |t| t.clamp(0.3, 0.99) as f32);
                    let (ready, pending) = faces_of(s, &ids, c)?;
                    if s.people_store().people.is_empty() {
                        return Err(bad(c, "name a few faces first (people.name): recognition matches against them"));
                    }
                    let mut suggestions = Vec::new();
                    let mut todo: Vec<(PhotoId, usize, String)> = Vec::new();
                    for (id, faces) in &ready {
                        let known = faces::known(s, *id);
                        let meta = s.catalog.photo(*id).map(|p| p.meta.clone()).unwrap_or_default();
                        let named: Vec<usize> = (0..faces.len())
                            .filter(|i| {
                                faces
                                    .get(*i)
                                    .is_some_and(|f| region_of(&meta, f.rect).and_then(|r| meta.regions.get(r)).is_some_and(|r| r.name.is_some()))
                            })
                            .collect();
                        // a person already named on the photo isn't suggested for another face
                        let here: Vec<String> = named
                            .iter()
                            .filter_map(|i| faces.get(*i))
                            .filter_map(|f| region_of(&meta, f.rect))
                            .filter_map(|r| meta.regions.get(r))
                            .filter_map(|r| r.name.clone())
                            .collect();
                        let known: Vec<(String, Vec<lightcraft_segment::identity::Signature>)> =
                            known.into_iter().filter(|(n, _)| !here.iter().any(|h| h.eq_ignore_ascii_case(n))).collect();
                        for (i, k, score) in faces::assign(faces, &named, &known, threshold) {
                            if let Some((name, _)) = known.get(k) {
                                suggestions.push(json!({"photo": id.0, "face": i, "name": name, "score": score}));
                                todo.push((*id, i, name.clone()));
                            }
                        }
                    }
                    let mut named = 0;
                    if bool_or(p, "apply", false) && pending == 0 && !todo.is_empty() {
                        // one undo step: group by name
                        let names: std::collections::BTreeSet<String> = todo.iter().map(|t| t.2.clone()).collect();
                        let before = s.commits();
                        for n in names {
                            let list: Vec<(PhotoId, usize)> = todo.iter().filter(|t| t.2 == n).map(|t| (t.0, t.1)).collect();
                            named += faces::name(s, &list, &n, c)?.0;
                        }
                        let steps = s.commits().wrapping_sub(before) as usize;
                        s.merge_undo(steps, "Recognize People");
                    }
                    Ok(json!({"suggestions": suggestions, "named": named, "pending": pending}))
                }
                #[cfg(not(feature = "sam"))]
                {
                    let _ = ids;
                    Err(no_ai(c))
                }
            }
        ),
        cmd!(
            query "people.cluster",
            "Group Unnamed Faces",
            [],
            None,
            "{ids?, threshold?: 0..1 (0.6)} — group the unnamed faces of the photos (a shoot) by appearance, for naming a group at once (people.name {faces}): {clusters: [{faces: [{photo, face, rect}]}], pending}",
            always,
            |s, p| {
                let c = "people.cluster";
                let ids = s.targets(p);
                #[cfg(feature = "sam")]
                {
                    let threshold = p.get("threshold").and_then(Value::as_f64).map_or(0.6, |t| t.clamp(0.2, 0.99) as f32);
                    let (ready, pending) = faces_of(s, &ids, c)?;
                    let mut refs = Vec::new();
                    let mut sigs = Vec::new();
                    for (id, faces) in &ready {
                        let meta = s.catalog.photo(*id).map(|p| p.meta.clone()).unwrap_or_default();
                        for (i, f) in faces.iter().enumerate() {
                            let named = region_of(&meta, f.rect).and_then(|r| meta.regions.get(r)).is_some_and(|r| r.name.is_some());
                            if !named && sigs.len() < lightcraft_segment::identity::MAX_CLUSTERED {
                                refs.push(json!({"photo": id.0, "face": i, "rect": f.rect}));
                                sigs.push(f.signature.clone());
                            }
                        }
                    }
                    let clusters: Vec<Value> = lightcraft_segment::identity::cluster(&sigs, threshold)
                        .into_iter()
                        .map(|cl| json!({"faces": cl.iter().filter_map(|i| refs.get(*i).cloned()).collect::<Vec<_>>()}))
                        .collect();
                    Ok(json!({"clusters": clusters, "pending": pending}))
                }
                #[cfg(not(feature = "sam"))]
                {
                    let _ = ids;
                    Err(no_ai(c))
                }
            }
        ),
        cmd!(
            "people.rename",
            "Rename Person",
            [],
            None,
            "{from, to} — rename a person on every photo (merging into `to` when that person exists), with their roles and remembered looks; one undo step → {photos}",
            always,
            |s, p| {
                let c = "people.rename";
                let (from, to) = (name_param(p, "from", c)?, name_param(p, "to", c)?);
                let ops = rename_ops(s, &from, Some(&to));
                let n = ops.len();
                if n > 0 {
                    s.commit(&format!("Rename {from}"), Op::Batch { ops })?;
                    s.skip_auto_write = true;
                }
                s.with_people(|st| st.rename(&from, &to)).map_err(crate::EngineError::Other)?;
                Ok(json!({"photos": n, "name": to}))
            }
        ),
        cmd!(
            "people.setRole",
            "Set Role",
            [],
            None,
            "{role: \"Bride\", person: \"Sam\", id?, scope?: \"folder\" (the photo's folder, default) | {folder} | {album} | {from, to} (dates) | {ids}} — who has the role in this shoot; masks and presets saying \"Bride\" find that person → {role, person, scope}",
            always,
            |s, p| {
                let c = "people.setRole";
                let (role, person) = (name_param(p, "role", c)?, name_param(p, "person", c)?);
                let photo = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active());
                let scope = scope_param(s, p, photo, c)?;
                let sj = scope_json(&scope);
                s.with_people(|st| st.set_role(&role, &person, scope)).map_err(crate::EngineError::Other)?.map_err(|e| bad(c, e))?;
                Ok(json!({"role": role, "person": person, "scope": sj}))
            }
        ),
        cmd!(
            query "people.roles",
            "Roles",
            [],
            None,
            "{id?} → {here: [{role, person}] (for the photo), roles: [{role, person, scope}] (all)}",
            always,
            |s, p| {
                let photo = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active());
                let ctx = photo.map(|id| s.photo_context(id));
                let store = s.people_store().clone();
                let here: Vec<Value> =
                    ctx.map(|c| store.roles_for(&c).into_iter().map(|(r, n)| json!({"role": r, "person": n})).collect()).unwrap_or_default();
                let all: Vec<Value> =
                    store.roles.iter().map(|b| json!({"role": b.role, "person": b.person, "scope": scope_json(&b.scope)})).collect();
                Ok(json!({"here": here, "roles": all}))
            }
        ),
        cmd!("people.clearRole", "Clear Role", [], None, "{role, scope?: as people.setRole (omitted: everywhere)} → {removed}", always, |s, p| {
            let c = "people.clearRole";
            let role = name_param(p, "role", c)?;
            let scope = if p.get("scope").is_some() { Some(scope_param(s, p, s.active(), c)?) } else { None };
            let n = s.with_people(|st| st.clear_role(&role, scope.as_ref())).map_err(crate::EngineError::Other)?;
            Ok(json!({"removed": n}))
        }),
        cmd!(
            "people.forget",
            "Forget This Person",
            [],
            None,
            "{person, names?: bool (false)} — delete what LightCraft remembers about how they look and their roles (biometric data); `names` also removes their name from every photo (one undo step) → {forgotten, photos}",
            always,
            |s, p| {
                let c = "people.forget";
                let person = name_param(p, "person", c)?;
                let forgotten = s.with_people(|st| st.forget(&person)).map_err(crate::EngineError::Other)?;
                let mut photos = 0;
                if bool_or(p, "names", false) {
                    let ops = rename_ops(s, &person, None);
                    photos = ops.len();
                    if photos > 0 {
                        s.commit(&format!("Forget {person}"), Op::Batch { ops })?;
                        s.skip_auto_write = true;
                    }
                }
                #[cfg(feature = "sam")]
                s.segmenter.smart.faces.clear();
                Ok(json!({"forgotten": forgotten, "photos": photos}))
            }
        ),
        cmd!(
            "people.forgetAll",
            "Forget All Face Data",
            [],
            None,
            "{names?: bool (false)} — delete every face signature, role and face-recognition embedding this library keeps (people.json, face-embeddings-*.bin, face-scanned.bin); `names` also removes every person's name from the photos (one undo step) → {deleted: [files], photos}",
            always,
            |s, p| {
                s.with_people(|st| *st = crate::people_store::PeopleStore::default()).map_err(crate::EngineError::Other)?;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    s.faces.index.forget_all();
                    s.faces.scanned.forget_all();
                }
                #[cfg(feature = "sam")]
                s.segmenter.smart.faces.clear();
                let mut deleted = vec![crate::people_store::FILE.to_string()];
                if let Some(dir) = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone())
                    && let Ok(rd) = std::fs::read_dir(&dir)
                {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().to_string();
                        if (n.starts_with("face-embeddings-") && n.ends_with(".bin")) || n == "face-scanned.bin" {
                            match std::fs::remove_file(e.path()) {
                                Ok(()) => deleted.push(n),
                                Err(err) => log::warn!("couldn't delete {n}: {err}"),
                            }
                        }
                    }
                }
                let mut photos = 0;
                if bool_or(p, "names", false) {
                    let ops: Vec<Op> = s
                        .catalog
                        .photos()
                        .filter(|ph| ph.meta.regions.iter().any(|r| r.kind == lightcraft_meta::RegionKind::Face && r.name.is_some()))
                        .map(|ph| {
                            let mut meta = ph.meta.clone();
                            for r in &mut meta.regions {
                                if r.kind == lightcraft_meta::RegionKind::Face {
                                    r.name = None;
                                }
                            }
                            Op::SetMeta { id: ph.id, meta: Box::new(meta) }
                        })
                        .collect();
                    photos = ops.len();
                    if photos > 0 {
                        s.commit("Forget All Names", Op::Batch { ops })?;
                        s.skip_auto_write = true;
                    }
                }
                Ok(json!({"deleted": deleted, "photos": photos}))
            }
        ),
    ]
}
