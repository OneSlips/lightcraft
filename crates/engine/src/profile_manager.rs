//! The Profile Manager: every profile in one list (LightCraft's built-in looks, the user's LUT
//! profiles and camera profiles), with the choices the user made about them ([`ProfileState`]):
//! hidden (out of the menus and the browser, kept here), removed (a built-in look deleted from this
//! install; "Restore built-in looks" brings them back), renamed, regrouped, reordered, favourites.
//! The menus and the profile browser list [`Session::shown_profiles`]; a photo that uses a hidden or
//! removed profile keeps rendering with it.
//!
//! Commands: `profiles.manager`, `profile.hide`, `profile.remove`, `profile.restoreBuiltins`,
//! `profile.rename`, `profile.setGroup`, `profile.reorder`, `profiles.bodies`.

use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::dcp_profiles::{self as dp, DCP_GROUP};
use crate::cmd::{CommandSpec, always, bad, cmd, str_param};
use crate::presets::PROFILES;
use crate::{Result, Session};

/// Where a profile comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfileKind {
    /// A look built into LightCraft.
    Builtin,
    /// A `.cube` LUT the user imported.
    Lut,
    /// A camera profile (`.dcp`) the user imported or made.
    Camera,
}

/// One profile as the menus and the manager list it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileItem {
    pub id: String,
    /// Its name (the user's, when renamed).
    pub name: String,
    /// Its name before renaming.
    pub original_name: String,
    pub group: String,
    /// The group is the user's (not translated like a built-in group).
    pub custom_group: bool,
    pub kind: ProfileKind,
    pub hidden: bool,
    pub favorite: bool,
}

impl ProfileItem {
    /// The name is LightCraft's own (translated for display).
    pub fn builtin_name(&self) -> bool {
        self.kind == ProfileKind::Builtin && self.name == self.original_name
    }
    /// The group is LightCraft's own (translated for display).
    pub fn builtin_group(&self) -> bool {
        self.kind == ProfileKind::Builtin && !self.custom_group
    }
}

impl Session {
    /// Every profile not removed from this install, hidden ones included, in the user's order (then
    /// built-in looks, LUT profiles, camera profiles).
    pub fn all_profiles(&self) -> Vec<ProfileItem> {
        let st = &self.profile_state;
        let item = |id: &str, name: &str, group: &str, kind: ProfileKind| {
            let custom = st.groups.get(id).filter(|g| !g.trim().is_empty());
            ProfileItem {
                id: id.to_string(),
                name: st.names.get(id).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| name.to_string()),
                original_name: name.to_string(),
                group: custom.cloned().unwrap_or_else(|| group.to_string()),
                custom_group: custom.is_some(),
                kind,
                hidden: st.hidden.iter().any(|h| h == id),
                favorite: self.profile_favorites.iter().any(|f| f == id),
            }
        };
        // the user's own profiles first (their camera profiles and LUT folders), then LightCraft's looks
        let mut out: Vec<ProfileItem> = self.dcp_profiles.iter().map(|p| item(&p.id, &p.name, DCP_GROUP, ProfileKind::Camera)).collect();
        out.extend(self.lut_profiles.iter().map(|p| item(&p.id, &p.name, &p.group, ProfileKind::Lut)));
        out.extend(PROFILES.iter().filter(|p| !st.removed.iter().any(|r| r == p.id)).map(|p| item(p.id, p.name, p.group, ProfileKind::Builtin)));
        let natural = Session::profile_group_order(&out);
        let group_pos = |g: &str| {
            st.group_order
                .iter()
                .position(|o| o == g)
                .unwrap_or_else(|| st.group_order.len() + natural.iter().position(|n| n == g).unwrap_or(natural.len()))
        };
        let pos = |id: &str| st.order.iter().position(|o| o == id).unwrap_or(usize::MAX);
        // stable: within a group, those not ordered keep their usual order after the ordered ones
        out.sort_by_key(|p| (group_pos(&p.group), pos(&p.id)));
        out
    }

    /// Every group in the manager's order: those with profiles, and empty ones made in the manager.
    pub fn profile_groups_all(&self) -> Vec<String> {
        let mut out = Session::profile_group_order(&self.all_profiles());
        for (i, g) in self.profile_state.group_order.iter().enumerate() {
            if !out.contains(g) {
                out.insert(i.min(out.len()), g.clone());
            }
        }
        out
    }

    /// The profiles the menus and the browser offer: [`Session::all_profiles`] without the hidden.
    pub fn shown_profiles(&self) -> Vec<ProfileItem> {
        self.all_profiles().into_iter().filter(|p| !p.hidden).collect()
    }

    /// Groups of `items` in the order they first appear.
    pub fn profile_group_order(items: &[ProfileItem]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in items {
            if !out.contains(&p.group) {
                out.push(p.group.clone());
            }
        }
        out
    }

    /// `id` is offered in the menus (not hidden, not removed).
    pub fn profile_shown(&self, id: &str) -> bool {
        let st = &self.profile_state;
        !st.hidden.iter().any(|h| h == id) && !st.removed.iter().any(|r| r == id)
    }
}

/// How many photos of the open library use profile `id` (as the look or as the camera profile).
fn used_by(s: &Session, id: &str) -> usize {
    s.catalog.photos().filter(|p| p.develop.profile.id == id || p.develop.calibration_id() == Some(id)).count()
}

fn manager(s: &mut Session, _: &Value) -> Result<Value> {
    let items: Vec<Value> = s
        .all_profiles()
        .into_iter()
        .map(|p| {
            let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
            v["usedBy"] = json!(used_by(s, &p.id));
            match p.kind {
                ProfileKind::Camera => {
                    if let Some(e) = s.dcp_profiles.iter().find(|e| e.id == p.id) {
                        v["cameraModel"] = json!(e.camera_model);
                        v["bodies"] = json!(e.bodies);
                        v["bodyDefaults"] = json!(e.body_defaults);
                        v["file"] = json!(e.file);
                        v["missing"] = json!(!std::path::Path::new(&e.file).is_file());
                    }
                }
                ProfileKind::Lut => {
                    if let Some(e) = s.lut_profiles.iter().find(|e| e.id == p.id) {
                        v["file"] = json!(e.file);
                        v["missing"] = json!(!std::path::Path::new(&e.file).is_file());
                    }
                }
                ProfileKind::Builtin => {}
            }
            v
        })
        .collect();
    Ok(json!({
        "profiles": items,
        "removedBuiltins": s.profile_state.removed.len(),
        "groups": s.profile_groups_all(),
        "store": if s.uses_app_store() { "shared" } else { "library" },
    }))
}

fn id_param(s: &Session, p: &Value, c: &str) -> Result<String> {
    let id = str_param(p, "id").ok_or_else(|| bad(c, "missing `id`"))?.to_string();
    if !s.profile_known(&id) {
        return Err(bad(c, format!("no profile `{id}`")));
    }
    Ok(id)
}

/// Save the Profile Manager's choices where they live (the app store, or the library).
fn save(s: &mut Session) -> Result<()> {
    s.save_prefs()
}

fn hide(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.hide";
    let id = id_param(s, p, C)?;
    let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(!s.profile_state.hidden.contains(&id));
    s.profile_state.hidden.retain(|h| h != &id);
    if hidden {
        s.profile_state.hidden.push(id.clone());
    }
    save(s)?;
    Ok(json!({"id": id, "hidden": hidden}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.remove";
    let id = id_param(s, p, C)?;
    let used = used_by(s, &id);
    if crate::presets::profile(&id).is_some() {
        // a built-in look: gone from this install's menus; photos using it keep rendering with it
        if !s.profile_state.removed.contains(&id) {
            s.profile_state.removed.push(id.clone());
        }
        s.profile_state.hidden.retain(|h| h != &id);
        s.profile_favorites.retain(|f| f != &id);
        s.profile_recent.retain(|f| f != &id);
        save(s)?;
        return Ok(json!({"removed": id, "builtin": true, "usedBy": used}));
    }
    // the user's own file: deleted (the store's or library's copy; a file elsewhere stays)
    s.execute("profile.deleteImported", &json!({"id": id}))?;
    s.profile_state.hidden.retain(|h| h != &id);
    s.profile_state.names.remove(&id);
    s.profile_state.groups.remove(&id);
    s.profile_state.order.retain(|o| o != &id);
    s.profile_favorites.retain(|f| f != &id);
    s.profile_recent.retain(|f| f != &id);
    save(s)?;
    Ok(json!({"removed": id, "builtin": false, "usedBy": used}))
}

fn restore_builtins(s: &mut Session, _: &Value) -> Result<Value> {
    let restored = std::mem::take(&mut s.profile_state.removed);
    save(s)?;
    Ok(json!({"restored": restored}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.rename";
    let id = id_param(s, p, C)?;
    let name = str_param(p, "name").map(str::trim).unwrap_or("").to_string();
    if name.chars().count() > 200 {
        return Err(bad(C, "the name is too long (200 characters at most)"));
    }
    let original = s.all_profiles().into_iter().find(|x| x.id == id).map(|x| x.original_name).unwrap_or_default();
    if name.is_empty() || name == original {
        s.profile_state.names.remove(&id);
    } else {
        s.profile_state.names.insert(id.clone(), name.clone());
    }
    save(s)?;
    let shown = s.profile_info(&id).map(|(n, _)| n.to_string()).unwrap_or_default();
    Ok(json!({"id": id, "name": shown}))
}

fn set_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.setGroup";
    let id = id_param(s, p, C)?;
    let group = str_param(p, "group").map(str::trim).unwrap_or("").to_string();
    if group.chars().count() > 120 {
        return Err(bad(C, "the group name is too long (120 characters at most)"));
    }
    if group.is_empty() {
        s.profile_state.groups.remove(&id);
    } else {
        s.profile_state.groups.insert(id.clone(), group.clone());
    }
    save(s)?;
    let shown = s.profile_info(&id).map(|(_, g)| g.to_string()).unwrap_or_default();
    Ok(json!({"id": id, "group": shown}))
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.reorder";
    if let Some(ids) = p.get("ids").and_then(Value::as_array) {
        let ids: Vec<String> = ids.iter().filter_map(Value::as_str).filter(|id| s.profile_known(id)).map(str::to_string).collect();
        // the given ones first, in that order; the rest of the earlier order after them
        let mut order = ids.clone();
        order.extend(s.profile_state.order.iter().filter(|o| !ids.contains(o)).cloned());
        s.profile_state.order = order;
    } else {
        // {id, before?: id | after?: id}: move one profile next to another
        let id = id_param(s, p, C)?;
        let all: Vec<String> = s.all_profiles().into_iter().map(|x| x.id).collect();
        let mut order: Vec<String> = all.into_iter().filter(|x| x != &id).collect();
        let at = if let Some(b) = str_param(p, "before") {
            order.iter().position(|x| x == b).ok_or_else(|| bad(C, format!("no profile `{b}`")))?
        } else if let Some(a) = str_param(p, "after") {
            order.iter().position(|x| x == a).map(|i| i + 1).ok_or_else(|| bad(C, format!("no profile `{a}`")))?
        } else {
            return Err(bad(C, "give {ids: [...]} or {id, before | after}"));
        };
        order.insert(at.min(order.len()), id);
        s.profile_state.order = order;
    }
    if p.get("reset").and_then(Value::as_bool) == Some(true) {
        s.profile_state.order.clear();
    }
    save(s)?;
    Ok(json!({"order": s.all_profiles().into_iter().map(|x| x.id).collect::<Vec<_>>()}))
}

fn group_param(s: &Session, p: &Value, c: &str) -> Result<String> {
    let g = str_param(p, "group").map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(c, "missing `group`"))?.to_string();
    if !s.profile_groups_all().contains(&g) {
        return Err(bad(c, format!("no profile group `{g}`")));
    }
    Ok(g)
}

fn new_group_name(p: &Value, key: &str, c: &str) -> Result<String> {
    let name = str_param(p, key).map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(c, format!("missing `{key}`")))?.to_string();
    if name.chars().count() > 120 {
        return Err(bad(c, "the group name is too long (120 characters at most)"));
    }
    Ok(name)
}

/// The profiles of group `g` (hidden ones included).
fn members(s: &Session, g: &str) -> Vec<ProfileItem> {
    s.all_profiles().into_iter().filter(|p| p.group == g).collect()
}

/// Put profile `id` in group `group`: its own group again when that is the one it comes from.
fn set_group_of(s: &mut Session, item: &ProfileItem, group: &str) {
    let natural = if item.custom_group {
        let mut st = s.profile_state.clone();
        st.groups.remove(&item.id);
        let saved = std::mem::replace(&mut s.profile_state, st);
        let g = s.all_profiles().into_iter().find(|x| x.id == item.id).map(|x| x.group).unwrap_or_default();
        s.profile_state = saved;
        g
    } else {
        item.group.clone()
    };
    if natural == group {
        s.profile_state.groups.remove(&item.id);
    } else {
        s.profile_state.groups.insert(item.id.clone(), group.to_string());
    }
}

fn group_hide(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.groupHide";
    let g = group_param(s, p, C)?;
    let list = members(s, &g);
    let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(!list.iter().all(|x| x.hidden));
    for it in &list {
        s.profile_state.hidden.retain(|h| h != &it.id);
        if hidden {
            s.profile_state.hidden.push(it.id.clone());
        }
    }
    save(s)?;
    Ok(json!({"group": g, "hidden": hidden, "profiles": list.len()}))
}

fn group_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.groupRemove";
    let g = group_param(s, p, C)?;
    let move_to = str_param(p, "moveTo").map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    if move_to.as_deref() == Some(g.as_str()) {
        return Err(bad(C, "moveTo is the group being removed"));
    }
    let list = members(s, &g);
    let (mut removed, mut deleted, mut moved) = (Vec::new(), Vec::new(), Vec::new());
    for it in &list {
        if it.kind == ProfileKind::Builtin {
            // a built-in look: off this install (Restore Built-in Looks brings it back)
            s.execute("profile.remove", &json!({"id": it.id}))?;
            removed.push(it.id.clone());
        } else if let Some(to) = &move_to {
            set_group_of(s, it, to);
            moved.push(it.id.clone());
        } else {
            s.execute("profile.remove", &json!({"id": it.id}))?;
            deleted.push(it.id.clone());
        }
    }
    s.profile_state.group_order.retain(|o| o != &g);
    if let Some(to) = &move_to
        && !s.profile_groups_all().contains(to)
    {
        s.profile_state.group_order.push(to.clone());
    }
    save(s)?;
    Ok(json!({"group": g, "removed": removed, "deleted": deleted, "moved": moved}))
}

fn group_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.groupRename";
    let g = group_param(s, p, C)?;
    let name = new_group_name(p, "name", C)?;
    for it in members(s, &g) {
        set_group_of(s, &it, &name);
    }
    // its place in the order is kept
    let mut order = if s.profile_state.group_order.is_empty() { s.profile_groups_all() } else { s.profile_state.group_order.clone() };
    match order.iter().position(|o| o == &g) {
        Some(i) => {
            if let Some(slot) = order.get_mut(i) {
                *slot = name.clone();
            }
        }
        None => order.push(name.clone()),
    }
    let mut seen = Vec::new();
    order.retain(|o| {
        let first = !seen.contains(o);
        seen.push(o.clone());
        first
    });
    s.profile_state.group_order = order;
    save(s)?;
    Ok(json!({"group": name}))
}

fn group_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.groupNew";
    let name = new_group_name(p, "name", C)?;
    if s.profile_groups_all().contains(&name) {
        return Err(bad(C, format!("there is a group `{name}` already")));
    }
    if s.profile_state.group_order.is_empty() {
        s.profile_state.group_order = s.profile_groups_all();
    }
    // first, where it is seen at once
    s.profile_state.group_order.insert(0, name.clone());
    save(s)?;
    Ok(json!({"group": name}))
}

fn group_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.groupMove";
    let g = group_param(s, p, C)?;
    let mut order: Vec<String> = s.profile_groups_all().into_iter().filter(|x| x != &g).collect();
    let at = if let Some(b) = str_param(p, "before") {
        order.iter().position(|x| x == b).ok_or_else(|| bad(C, format!("no group `{b}`")))?
    } else if let Some(a) = str_param(p, "after") {
        order.iter().position(|x| x == a).map(|i| i + 1).ok_or_else(|| bad(C, format!("no group `{a}`")))?
    } else {
        return Err(bad(C, "give {group, before | after: group}"));
    };
    order.insert(at.min(order.len()), g);
    s.profile_state.group_order = order.clone();
    save(s)?;
    Ok(json!({"groups": order}))
}

/// The camera bodies LightCraft knows (the open library's photos, and those the camera profiles name)
/// with the profile each develops with by default.
fn bodies(s: &mut Session, _: &Value) -> Result<Value> {
    use std::collections::BTreeMap;
    // body → (camera, photos in this library)
    let mut seen: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for p in s.catalog.photos() {
        if !p.develops_raw() {
            continue;
        }
        if let Some(b) = dp::catalog_body(p) {
            let e = seen.entry(b).or_insert_with(|| (p.meta.camera.clone(), 0));
            e.1 += 1;
        }
    }
    for e in &s.dcp_profiles {
        for b in e.bodies.iter().chain(e.body_defaults.iter()) {
            seen.entry(b.clone()).or_insert_with(|| (e.camera_model.clone(), 0));
        }
    }
    let rows: Vec<Value> = seen
        .into_iter()
        .map(|(body, (camera, photos))| {
            let default = dp::body_default_for(s, &camera, &body);
            let name = default.as_deref().and_then(|d| s.profile_info(d)).map(|(n, _)| n.to_string());
            let profiles: Vec<Value> = s
                .dcp_profiles
                .iter()
                .filter(|e| dp::same_camera(e, &camera) || camera.is_empty())
                .map(|e| json!({"id": e.id, "name": e.name, "relation": dp::relation(e, Some(&body))}))
                .collect();
            json!({"body": body, "label": dp::body_label(&body), "camera": crate::import_profiles::display_camera(&camera), "photos": photos, "profile": default, "profileName": name, "profiles": profiles})
        })
        .collect();
    Ok(json!({"bodies": rows}))
}

fn clear_body_default(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "profile.clearBodyDefault";
    let serial = str_param(p, "serial").map(str::trim).filter(|x| !x.is_empty()).ok_or_else(|| bad(C, "missing `serial`"))?.to_string();
    let mut cleared = Vec::new();
    for e in &mut s.dcp_profiles {
        if e.body_defaults.contains(&serial) {
            e.body_defaults.retain(|b| b != &serial);
            cleared.push(e.id.clone());
        }
    }
    save(s)?;
    let now = s.catalog.photos().find(|ph| dp::catalog_body(ph).as_deref() == Some(serial.as_str())).map(|ph| ph.meta.camera.clone());
    let fallback = now.and_then(|camera| dp::body_default_for(s, &camera, &serial));
    Ok(json!({"serial": serial, "cleared": cleared, "profile": fallback}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "profiles.manager",
            "Profile Manager List",
            [],
            None,
            "{} → {groups: [names in order, empty ones too], profiles: [{id, name, originalName, group, customGroup, kind: builtin | lut | camera, hidden, favorite, usedBy (photos of this library), cameraModel?, bodies?, bodyDefaults?, file?, missing?}], removedBuiltins, store: shared | library}: every profile not removed, hidden ones included, in the user's order",
            always,
            manager
        ),
        cmd!(
            "profile.hide",
            "Hide Profile",
            [],
            None,
            "{id, hidden?: bool (default: toggle)} — out of the profile menus and browser (kept in the Profile Manager); photos using it keep it",
            always,
            hide
        ),
        cmd!(
            "profile.remove",
            "Delete Profile",
            [],
            None,
            "{id} — a built-in look is removed from this install (profile.restoreBuiltins brings it back); the user's own LUT or camera profile is deleted with its file in the store (a file elsewhere stays). Photos using a built-in look keep it; photos using a deleted camera profile develop with LightCraft's camera colour → {removed, builtin, usedBy}",
            always,
            remove
        ),
        cmd!(
            "profile.restoreBuiltins",
            "Restore Built-in Looks",
            [],
            None,
            "{} — every built-in look removed with profile.remove is back → {restored: [ids]}",
            always,
            restore_builtins
        ),
        cmd!(
            "profile.rename",
            "Rename Profile",
            [],
            None,
            "{id, name} — the name shown everywhere (\"\" = its own name again); the file isn't changed",
            always,
            rename
        ),
        cmd!(
            "profile.setGroup",
            "Move Profile to Group",
            [],
            None,
            "{id, group} — the group it is listed under in the menus and browser (\"\" = its own group again)",
            always,
            set_group
        ),
        cmd!(
            "profile.reorder",
            "Reorder Profiles",
            [],
            None,
            "{ids: [...]} (these first, in this order) | {id, before | after: id} | {reset: true} — the order profiles are listed in",
            always,
            reorder
        ),
        cmd!(
            "profile.clearBodyDefault",
            "Clear Body Default Profile",
            [],
            None,
            "{serial} — no profile is chosen as the body's default any more (a profile made on the body still applies) → {serial, cleared: [ids], profile}",
            always,
            clear_body_default
        ),
        cmd!(
            "profile.groupHide",
            "Hide Profile Group",
            [],
            None,
            "{group, hidden?: bool (default: toggle)} — every profile of the group out of (or back in) the menus and browser",
            always,
            group_hide
        ),
        cmd!(
            "profile.groupRemove",
            "Delete Profile Group",
            [],
            None,
            "{group, moveTo?: group} — its built-in looks are removed from this install (Restore Built-in Looks brings them back); its own LUT and camera profiles are deleted with their files, or moved to `moveTo` → {group, removed, deleted, moved}",
            always,
            group_remove
        ),
        cmd!(
            "profile.groupRename",
            "Rename Profile Group",
            [],
            None,
            "{group, name} — every profile of the group moves to the new name, in the same place",
            always,
            group_rename
        ),
        cmd!(
            "profile.groupNew",
            "New Profile Group",
            [],
            None,
            "{name} — an empty group, first in the list (move profiles into it with profile.setGroup)",
            always,
            group_new
        ),
        cmd!(
            "profile.groupMove",
            "Move Profile Group",
            [],
            None,
            "{group, before | after: group} — the order of the groups in the menus, browser and manager",
            always,
            group_move
        ),
        cmd!(
            query "profiles.bodies",
            "Camera Bodies and Their Profiles",
            [],
            None,
            "{} → {bodies: [{body (serial, or file:XX), label (…71), camera, photos (raw photos of this library), profile (the body's default camera profile, null: LightCraft's colour), profileName, profiles: [{id, name, relation}]}]}",
            always,
            bodies
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_remove_rename_group_restore() {
        let mut s = Session::with_demo();
        let n = s.shown_profiles().len();
        s.execute("profile.hide", &json!({"id": "lc.vivid"})).unwrap();
        assert_eq!(s.shown_profiles().len(), n - 1);
        assert!(s.all_profiles().iter().any(|p| p.id == "lc.vivid" && p.hidden));
        s.execute("profile.hide", &json!({"id": "lc.vivid", "hidden": false})).unwrap();
        assert_eq!(s.shown_profiles().len(), n);
        let r = s.execute("profile.remove", &json!({"id": "lc.film.cool-fade"})).unwrap();
        assert_eq!(r["builtin"], true);
        assert!(!s.all_profiles().iter().any(|p| p.id == "lc.film.cool-fade"));
        // a photo can still use it: it is still known
        assert!(s.profile_info("lc.film.cool-fade").is_some());
        s.execute("profile.rename", &json!({"id": "lc.portrait", "name": "Skin"})).unwrap();
        assert_eq!(s.profile_info("lc.portrait").unwrap().0, "Skin");
        s.execute("profile.setGroup", &json!({"id": "lc.portrait", "group": "Weddings"})).unwrap();
        let it = s.all_profiles().into_iter().find(|p| p.id == "lc.portrait").unwrap();
        assert_eq!(it.group, "Weddings");
        assert!(!it.builtin_name() && !it.builtin_group());
        s.execute("profile.reorder", &json!({"id": "lc.vivid", "before": "lc.color"})).unwrap();
        assert_eq!(s.all_profiles().into_iter().find(|p| p.group == "Basic").unwrap().id, "lc.vivid", "first of its group");
        let menu = s.profile_menu();
        assert!(menu["groups"].as_array().unwrap().iter().any(|g| g["name"] == "Weddings"));
        s.execute("profile.restoreBuiltins", &json!({})).unwrap();
        assert!(s.all_profiles().iter().any(|p| p.id == "lc.film.cool-fade"));
        assert!(s.execute("profile.hide", &json!({"id": "nope"})).is_err());
    }

    /// A model-wide DNG camera profile (our own numbers).
    fn test_dcp(name: &str) -> Vec<u8> {
        use lightcraft_raw::{ColorData, Mat3};
        let xyz_to_cam = Mat3([[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]]);
        lightcraft_raw::dcp::write_dcp(&lightcraft_raw::dcp::DcpProfile {
            name: name.into(),
            camera_model: "LightCraft Test Camera".into(),
            color: ColorData { illuminant: [21, 0], color_matrix: [Some(xyz_to_cam), None], ..Default::default() },
        })
        .unwrap()
    }

    /// The Profile Manager's choices are kept with the shared store, so a new library has them.
    #[test]
    fn choices_live_in_the_shared_store() {
        let root = std::env::temp_dir().join(format!("lc-pm-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = Session::new().with_app_store(&root.join("store"));
        s.open_library(root.join("A"), true).unwrap();
        s.execute("profile.hide", &json!({"id": "lc.vivid"})).unwrap();
        s.execute("profile.rename", &json!({"id": "lc.portrait", "name": "Skin"})).unwrap();
        s.close_library().unwrap();
        drop(s);
        let mut s = Session::new().with_app_store(&root.join("store"));
        s.open_library(root.join("B"), true).unwrap();
        assert!(!s.profile_shown("lc.vivid"));
        assert_eq!(s.profile_info("lc.portrait").unwrap().0, "Skin");
        s.close_library().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_body_default_on_a_model_wide_profile_applies() {
        let root = std::env::temp_dir().join(format!("lc-pm-bodydefault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut s = Session::new().with_app_store(&root.join("store"));
        s.open_library(root.join("L"), false).unwrap();
        let f = root.join("p.dcp");
        std::fs::write(&f, test_dcp("Wedding")).unwrap();
        s.execute("profile.import", &json!({"paths": [f.to_string_lossy()]})).unwrap();
        let id = s.dcp_profiles[0].id.clone();
        assert!(s.dcp_profiles[0].bodies.is_empty(), "model-wide");
        s.execute("profile.setBodyDefault", &json!({"profile": id, "serial": "0123456071"})).unwrap();
        assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, "LightCraft Test Camera", "0123456071").as_deref(), Some(id.as_str()));
        let bodies = s.execute("profiles.bodies", &json!({})).unwrap();
        assert!(bodies["bodies"].as_array().unwrap().iter().any(|b| b["body"] == "0123456071" && b["profile"] == id.as_str()), "{bodies}");
        s.execute("profile.clearBodyDefault", &json!({"serial": "0123456071"})).unwrap();
        assert_eq!(crate::cmd::dcp_profiles::body_default_for(&s, "LightCraft Test Camera", "0123456071"), None);
        s.close_library().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn whole_groups_hide_rename_move_and_go() {
        let root = std::env::temp_dir().join(format!("lc-pm-groups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut s = Session::new().with_app_store(&root.join("store"));
        s.open_library(root.join("L"), true).unwrap();
        let f = root.join("p.dcp");
        std::fs::write(&f, test_dcp("Wedding")).unwrap();
        s.execute("profile.import", &json!({"paths": [f.to_string_lossy()]})).unwrap();
        let dcp = s.dcp_profiles[0].id.clone();
        // the user's own come first
        assert_eq!(s.profile_groups_all().first().map(String::as_str), Some(DCP_GROUP));
        // hide a whole group, and show it again
        s.execute("profile.groupHide", &json!({"group": "Film"})).unwrap();
        assert!(!s.shown_profiles().iter().any(|p| p.group == "Film"));
        s.execute("profile.groupHide", &json!({"group": "Film", "hidden": false})).unwrap();
        assert!(s.shown_profiles().iter().any(|p| p.group == "Film"));
        // rename keeps its place; new and move
        s.execute("profile.groupRename", &json!({"group": "Muted", "name": "Soft"})).unwrap();
        let groups = s.profile_groups_all();
        assert!(groups.contains(&"Soft".to_string()) && !groups.contains(&"Muted".to_string()));
        s.execute("profile.groupNew", &json!({"name": "Weddings"})).unwrap();
        assert_eq!(s.profile_groups_all().first().map(String::as_str), Some("Weddings"), "an empty group shows");
        s.execute("profile.groupMove", &json!({"group": "B&W", "before": "Weddings"})).unwrap();
        assert_eq!(s.profile_groups_all().first().map(String::as_str), Some("B&W"));
        // removing a group of own profiles can move them instead of deleting the files
        s.execute("profile.groupRemove", &json!({"group": DCP_GROUP, "moveTo": "Weddings"})).unwrap();
        assert!(s.dcp_profiles.iter().any(|e| e.id == dcp), "moved, not deleted");
        assert_eq!(s.all_profiles().into_iter().find(|p| p.id == dcp).unwrap().group, "Weddings");
        // built-in looks of a removed group come back with Restore
        s.execute("profile.groupRemove", &json!({"group": "Cinematic"})).unwrap();
        assert!(!s.profile_groups_all().contains(&"Cinematic".to_string()));
        s.execute("profile.restoreBuiltins", &json!({})).unwrap();
        assert!(s.profile_groups_all().contains(&"Cinematic".to_string()));
        // and own files are deleted when not moved
        s.execute("profile.groupRemove", &json!({"group": "Weddings"})).unwrap();
        assert!(s.dcp_profiles.is_empty());
        assert!(s.execute("profile.groupHide", &json!({"group": "nope"})).is_err());
        s.close_library().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
}
