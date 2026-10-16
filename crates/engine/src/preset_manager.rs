//! The Presets Manager: every develop preset (the user's, often thousands in Lightroom groups, and
//! LightCraft's built-in ones with the Smart Masks), with the choices the user made about them
//! ([`PresetState`]): hidden (out of the Presets panel, kept here), removed (a built-in preset taken
//! off this install; "Restore built-in presets" brings them back), built-in presets renamed or moved
//! to another group, and the order of the groups. User presets are renamed, regrouped, reordered and
//! deleted in the list itself. Whole groups: hide, delete (or move their presets elsewhere), rename,
//! new, move.
//!
//! Commands: `presets.manager`, `preset.hide`, `preset.remove`, `preset.restoreBuiltins`,
//! `preset.reorder`, `preset.groupHide`, `preset.groupRemove`, `preset.groupRename`, `preset.groupNew`,
//! `preset.groupMove`; with `preset.rename`, `preset.move`, `preset.favorite`, `preset.delete`.

use std::collections::{BTreeMap, HashMap};

use lightcraft_develop::Preset;
use serde_json::{Value, json};

use crate::app_store::PresetState;
use crate::cmd::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

/// Built-in presets as LightCraft names and groups them (id → (name, group)).
fn builtin_names() -> &'static HashMap<String, (String, String)> {
    static MAP: std::sync::OnceLock<HashMap<String, (String, String)>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| crate::presets::builtin().into_iter().map(|p| (p.id, (p.name, p.group))).collect())
}

/// Give built-in presets the names and groups the user chose (and back their own where none is).
pub fn apply_builtin_overrides(list: &mut [Preset], st: &PresetState) {
    let base = builtin_names();
    for p in list.iter_mut().filter(|p| p.builtin) {
        let Some((name, group)) = base.get(&p.id) else { continue };
        p.name = st.names.get(&p.id).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| name.clone());
        p.group = st.groups.get(&p.id).filter(|g| !g.trim().is_empty()).cloned().unwrap_or_else(|| group.clone());
    }
}

impl Session {
    /// A preset the Presets panel and the pickers offer (not hidden, not removed).
    pub fn preset_shown(&self, p: &Preset) -> bool {
        !self.preset_state.hidden.contains(&p.id) && !(p.builtin && self.preset_state.removed.contains(&p.id))
    }

    /// A preset is in the manager (built-in presets removed from this install are not).
    pub fn preset_listed(&self, p: &Preset) -> bool {
        !(p.builtin && self.preset_state.removed.contains(&p.id))
    }

    /// Every group in the manager's order: the user's order first, then the rest by name; empty
    /// groups made in the manager included.
    pub fn preset_groups(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for p in self.presets.iter().filter(|p| self.preset_listed(p)) {
            if seen.insert(p.group.as_str()) {
                names.push(p.group.clone());
            }
        }
        names.sort();
        let order = &self.preset_state.group_order;
        let mut out: Vec<String> = order.clone();
        out.extend(names.into_iter().filter(|g| !order.contains(g)));
        out
    }

    /// The place of group `g` in [`Session::preset_groups`] order (for sorting).
    pub fn preset_group_rank(&self) -> BTreeMap<String, usize> {
        self.preset_groups().into_iter().enumerate().map(|(i, g)| (g, i)).collect()
    }

    fn preset_state_changed(&mut self) -> Result<()> {
        self.preset_state_gen = self.preset_state_gen.wrapping_add(1);
        self.save_prefs()
    }

    /// Built-in presets take their user names and groups again (after the choices changed).
    fn reapply_builtin_overrides(&mut self) {
        let st = self.preset_state.clone();
        if self.presets.iter().any(|p| {
            p.builtin
                && builtin_names()
                    .get(&p.id)
                    .is_some_and(|(n, g)| st.names.get(&p.id).unwrap_or(n) != &p.name || st.groups.get(&p.id).unwrap_or(g) != &p.group)
        }) {
            apply_builtin_overrides(&mut self.presets, &st);
        }
    }
}

fn id_param(s: &Session, p: &Value, c: &str) -> Result<(String, bool)> {
    let id = str_param(p, "id").ok_or_else(|| bad(c, "missing `id`"))?.to_string();
    let builtin = s.presets.iter().find(|x| x.id == id).map(|x| x.builtin).ok_or_else(|| bad(c, format!("no preset `{id}`")))?;
    Ok((id, builtin))
}

fn group_param(s: &Session, p: &Value, c: &str) -> Result<String> {
    let g = str_param(p, "group").map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(c, "missing `group`"))?.to_string();
    if !s.preset_groups().contains(&g) {
        return Err(bad(c, format!("no preset group `{g}`")));
    }
    Ok(g)
}

fn name_param(p: &Value, key: &str, c: &str) -> Result<String> {
    let name = str_param(p, key).map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(c, format!("missing `{key}`")))?.to_string();
    if name.chars().count() > 200 {
        return Err(bad(c, "the name is too long (200 characters at most)"));
    }
    Ok(name)
}

/// Move preset `id` to `group` (a built-in one through the choices).
fn set_group(s: &mut Session, id: &str, builtin: bool, group: &str) {
    if builtin {
        let own = builtin_names().get(id).map(|(_, g)| g.clone()).unwrap_or_default();
        if own == group {
            s.preset_state.groups.remove(id);
        } else {
            s.preset_state.groups.insert(id.to_string(), group.to_string());
        }
    } else if let Some(pr) = s.presets.iter_mut().find(|x| x.id == id) {
        pr.group = group.to_string();
    }
}

fn manager(s: &mut Session, p: &Value) -> Result<Value> {
    let query = str_param(p, "query").map(str::to_lowercase).unwrap_or_default();
    let only_group = str_param(p, "group").map(str::to_string);
    let ai = p.get("aiMasks").and_then(Value::as_bool) == Some(true);
    let offset = p.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(500).min(20_000) as usize;
    let rank = s.preset_group_rank();
    let mut rows: Vec<(usize, usize, &Preset)> = s
        .presets
        .iter()
        .enumerate()
        .filter(|(_, x)| s.preset_listed(x))
        .filter(|(_, x)| only_group.as_ref().is_none_or(|g| &x.group == g))
        .filter(|(_, x)| query.is_empty() || x.name.to_lowercase().contains(&query) || x.group.to_lowercase().contains(&query))
        .filter(|(_, x)| !ai || x.has_ai_masks())
        .map(|(i, x)| (rank.get(&x.group).copied().unwrap_or(usize::MAX), i, x))
        .collect();
    rows.sort_by_key(|(r, i, _)| (*r, *i));
    let total = rows.len();
    let presets: Vec<Value> = rows
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(_, _, x)| {
            json!({"id": x.id, "name": x.name, "group": x.group, "builtin": x.builtin, "favorite": x.favorite,
                "hidden": s.preset_state.hidden.contains(&x.id), "aiMasks": x.has_ai_masks()})
        })
        .collect();
    let groups: Vec<Value> = s
        .preset_groups()
        .into_iter()
        .map(|g| {
            let members: Vec<&Preset> = s.presets.iter().filter(|x| x.group == g && s.preset_listed(x)).collect();
            json!({"name": g, "count": members.len(), "builtin": members.iter().filter(|x| x.builtin).count(),
                "hidden": !members.is_empty() && members.iter().all(|x| s.preset_state.hidden.contains(&x.id))})
        })
        .collect();
    Ok(json!({"groups": groups, "presets": presets, "total": total, "removedBuiltins": s.preset_state.removed.len(),
        "store": if s.uses_app_store() { "shared" } else { "library" }}))
}

fn hide(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, _) = id_param(s, p, "preset.hide")?;
    let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(!s.preset_state.hidden.contains(&id));
    s.preset_state.hidden.retain(|h| h != &id);
    if hidden {
        s.preset_state.hidden.push(id.clone());
    }
    s.preset_state_changed()?;
    Ok(json!({"id": id, "hidden": hidden}))
}

fn remove_one(s: &mut Session, id: &str, builtin: bool) {
    if builtin {
        if !s.preset_state.removed.iter().any(|r| r == id) {
            s.preset_state.removed.push(id.to_string());
        }
        if let Some(pr) = s.presets.iter_mut().find(|x| x.id == id) {
            pr.favorite = false;
        }
    } else {
        s.presets.retain(|x| x.id != id);
    }
    s.preset_state.hidden.retain(|h| h != id);
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, builtin) = id_param(s, p, "preset.remove")?;
    remove_one(s, &id, builtin);
    s.preset_state_changed()?;
    Ok(json!({"removed": id, "builtin": builtin}))
}

fn restore_builtins(s: &mut Session, _: &Value) -> Result<Value> {
    let restored = std::mem::take(&mut s.preset_state.removed);
    s.preset_state_changed()?;
    Ok(json!({"restored": restored}))
}

fn rename_builtin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.renameBuiltin";
    let (id, builtin) = id_param(s, p, C)?;
    if !builtin {
        return Err(bad(C, "a preset of your own is renamed with preset.rename"));
    }
    let name = str_param(p, "name").map(str::trim).unwrap_or("").to_string();
    let own = builtin_names().get(&id).map(|(n, _)| n.clone()).unwrap_or_default();
    if name.is_empty() || name == own {
        s.preset_state.names.remove(&id);
    } else {
        s.preset_state.names.insert(id.clone(), name);
    }
    s.reapply_builtin_overrides();
    s.preset_state_changed()?;
    let shown = s.presets.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
    Ok(json!({"id": id, "name": shown}))
}

fn move_to_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.setGroup";
    let (id, builtin) = id_param(s, p, C)?;
    let group = name_param(p, "group", C)?;
    set_group(s, &id, builtin, &group);
    s.reapply_builtin_overrides();
    s.preset_state_changed()?;
    Ok(json!({"id": id, "group": group}))
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.reorder";
    let (id, builtin) = id_param(s, p, C)?;
    if builtin {
        return Err(bad(C, "built-in presets keep their order"));
    }
    let other = str_param(p, "before").map(|b| (b, true)).or_else(|| str_param(p, "after").map(|a| (a, false)));
    let Some((other, before)) = other else { return Err(bad(C, "give {id, before | after: id}")) };
    let from = s.presets.iter().position(|x| x.id == id).ok_or_else(|| bad(C, "no such preset"))?;
    let moved = s.presets.remove(from);
    let to = s.presets.iter().position(|x| x.id == other).ok_or_else(|| bad(C, format!("no preset `{other}`")))?;
    let at = if before { to } else { to + 1 };
    let len = s.presets.len();
    s.presets.insert(at.min(len), moved);
    Ok(json!({"id": id}))
}

fn group_hide(s: &mut Session, p: &Value) -> Result<Value> {
    let g = group_param(s, p, "preset.groupHide")?;
    let ids: Vec<String> = s.presets.iter().filter(|x| x.group == g && s.preset_listed(x)).map(|x| x.id.clone()).collect();
    let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(!ids.iter().all(|i| s.preset_state.hidden.contains(i)));
    s.preset_state.hidden.retain(|h| !ids.contains(h));
    if hidden {
        s.preset_state.hidden.extend(ids.iter().cloned());
    }
    s.preset_state_changed()?;
    Ok(json!({"group": g, "hidden": hidden, "presets": ids.len()}))
}

fn group_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.groupRemove";
    let g = group_param(s, p, C)?;
    let move_to = str_param(p, "moveTo").map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    if move_to.as_deref() == Some(g.as_str()) {
        return Err(bad(C, "moveTo is the group being removed"));
    }
    let members: Vec<(String, bool)> = s.presets.iter().filter(|x| x.group == g && s.preset_listed(x)).map(|x| (x.id.clone(), x.builtin)).collect();
    let (mut removed, mut deleted, mut moved) = (0usize, 0usize, 0usize);
    let mut gone: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (id, builtin) in &members {
        if *builtin {
            if !s.preset_state.removed.contains(id) {
                s.preset_state.removed.push(id.clone());
            }
            removed += 1;
        } else if let Some(to) = &move_to {
            set_group(s, id, false, to);
            moved += 1;
        } else {
            gone.insert(id.clone());
            deleted += 1;
        }
        s.preset_state.hidden.retain(|h| h != id);
    }
    if !gone.is_empty() {
        // one pass (a group can hold thousands)
        s.presets.retain(|x| !gone.contains(&x.id));
    }
    s.preset_state.group_order.retain(|o| o != &g);
    if let Some(to) = &move_to
        && !s.preset_state.group_order.is_empty()
        && !s.preset_state.group_order.contains(to)
    {
        s.preset_state.group_order.push(to.clone());
    }
    s.preset_state_changed()?;
    Ok(json!({"group": g, "removed": removed, "deleted": deleted, "moved": moved}))
}

fn group_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.groupRename";
    let g = group_param(s, p, C)?;
    let name = name_param(p, "name", C)?;
    let members: Vec<(String, bool)> = s.presets.iter().filter(|x| x.group == g).map(|x| (x.id.clone(), x.builtin)).collect();
    let builtin: Vec<&(String, bool)> = members.iter().filter(|(_, b)| *b).collect();
    for (id, _) in &builtin {
        set_group(s, id, true, &name);
    }
    if members.iter().any(|(_, b)| !*b) {
        // (one pass over the list)
        for pr in s.presets.iter_mut().filter(|x| !x.builtin && x.group == g) {
            pr.group = name.clone();
        }
    }
    for o in s.preset_state.group_order.iter_mut().filter(|o| **o == g) {
        *o = name.clone();
    }
    s.reapply_builtin_overrides();
    s.preset_state_changed()?;
    Ok(json!({"group": name, "presets": members.len()}))
}

fn group_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.groupNew";
    let name = name_param(p, "name", C)?;
    if s.preset_groups().contains(&name) {
        return Err(bad(C, format!("there is a group `{name}` already")));
    }
    if s.preset_state.group_order.is_empty() {
        s.preset_state.group_order = s.preset_groups();
    }
    s.preset_state.group_order.insert(0, name.clone());
    s.preset_state_changed()?;
    Ok(json!({"group": name}))
}

fn group_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "preset.groupMove";
    let g = group_param(s, p, C)?;
    let mut order: Vec<String> = s.preset_groups().into_iter().filter(|x| x != &g).collect();
    let at = if let Some(b) = str_param(p, "before") {
        order.iter().position(|x| x == b).ok_or_else(|| bad(C, format!("no group `{b}`")))?
    } else if let Some(a) = str_param(p, "after") {
        order.iter().position(|x| x == a).map(|i| i + 1).ok_or_else(|| bad(C, format!("no group `{a}`")))?
    } else {
        return Err(bad(C, "give {group, before | after: group}"));
    };
    order.insert(at.min(order.len()), g);
    s.preset_state.group_order = order;
    s.preset_state_changed()?;
    Ok(json!({"groups": s.preset_groups()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "presets.manager",
            "Presets Manager List",
            [],
            None,
            "{query?: text in name or group, group?, aiMasks?: bool (only presets with AI masks), offset?, limit? (default 500)} → {groups: [{name, count, builtin, hidden}] in order (empty ones too), presets: [{id, name, group, builtin, favorite, hidden, aiMasks}], total, removedBuiltins, store}",
            always,
            manager
        ),
        cmd!(
            "preset.hide",
            "Hide Preset",
            [],
            None,
            "{id, hidden?: bool (default: toggle)} — out of the Presets panel and pickers, kept in the manager",
            always,
            hide
        ),
        cmd!(
            "preset.remove",
            "Delete Preset or Built-in Preset",
            [],
            None,
            "{id} — a preset of your own is deleted; a built-in one is taken off this install (preset.restoreBuiltins brings it back) → {removed, builtin}",
            always,
            remove
        ),
        cmd!(
            "preset.restoreBuiltins",
            "Restore Built-in Presets",
            [],
            None,
            "{} — every built-in preset removed is back → {restored: [ids]}",
            always,
            restore_builtins
        ),
        cmd!(
            "preset.renameBuiltin",
            "Rename Built-in Preset",
            [],
            None,
            "{id, name} — the name shown for a built-in preset (\"\" = its own again)",
            always,
            rename_builtin
        ),
        cmd!(
            "preset.setGroup",
            "Move Preset to Group",
            [],
            None,
            "{id, group} — any preset, built-in ones too (their own group when it is that one)",
            always,
            move_to_group
        ),
        cmd!(
            "preset.reorder",
            "Reorder Preset",
            [],
            None,
            "{id, before | after: id} — the place of a preset of your own in the list",
            always,
            reorder
        ),
        cmd!(
            "preset.groupHide",
            "Hide Preset Group",
            [],
            None,
            "{group, hidden?: bool (default: toggle)} — every preset of the group",
            always,
            group_hide
        ),
        cmd!(
            "preset.groupRemove",
            "Delete Preset Group",
            [],
            None,
            "{group, moveTo?: group} — its built-in presets are taken off this install (restorable); its own are deleted, or moved to `moveTo` → {group, removed, deleted, moved}",
            always,
            group_remove
        ),
        cmd!(
            "preset.groupRename",
            "Rename Preset Group",
            [],
            None,
            "{group, name} — every preset of the group, built-in ones too, in the same place",
            always,
            group_rename
        ),
        cmd!("preset.groupNew", "New Preset Group", [], None, "{name} — an empty group, first in the list", always, group_new),
        cmd!(
            "preset.groupMove",
            "Move Preset Group",
            [],
            None,
            "{group, before | after: group} — the order of the groups in the panel and manager",
            always,
            group_move
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::with_demo();
        for (i, g) in ["TestWedding", "TestWedding", "TestFilm", "TestFilm", "TestFilm"].iter().enumerate() {
            s.presets.push(Preset {
                id: format!("user.t{i}"),
                name: format!("P{i}"),
                group: g.to_string(),
                settings: json!({"light": {"exposure": 0.1}}),
                favorite: false,
                builtin: false,
            });
        }
        s
    }

    #[test]
    fn presets_hide_remove_rename_and_restore() {
        let mut s = session();
        let builtin = s.presets.iter().find(|p| p.builtin).unwrap().id.clone();
        s.execute("preset.hide", &json!({"id": "user.t0"})).unwrap();
        let p0 = s.presets.iter().find(|p| p.id == "user.t0").unwrap().clone();
        assert!(!s.preset_shown(&p0) && s.preset_listed(&p0));
        s.execute("preset.remove", &json!({"id": builtin})).unwrap();
        assert!(s.presets.iter().any(|p| p.id == builtin), "a built-in preset stays known");
        let b = s.presets.iter().find(|p| p.id == builtin).unwrap().clone();
        assert!(!s.preset_listed(&b));
        s.execute("preset.restoreBuiltins", &json!({})).unwrap();
        assert!(s.preset_listed(&b));
        s.execute("preset.renameBuiltin", &json!({"id": builtin, "name": "Mine"})).unwrap();
        assert_eq!(s.presets.iter().find(|p| p.id == builtin).unwrap().name, "Mine");
        s.execute("preset.setGroup", &json!({"id": builtin, "group": "TestWedding"})).unwrap();
        assert_eq!(s.presets.iter().find(|p| p.id == builtin).unwrap().group, "TestWedding");
        s.execute("preset.remove", &json!({"id": "user.t1"})).unwrap();
        assert!(!s.presets.iter().any(|p| p.id == "user.t1"), "own: deleted");
        // reorder own presets
        s.execute("preset.reorder", &json!({"id": "user.t4", "before": "user.t2"})).unwrap();
        let film: Vec<&str> = s.presets.iter().filter(|p| p.group == "TestFilm").map(|p| p.id.as_str()).collect();
        assert_eq!(film, ["user.t4", "user.t2", "user.t3"]);
        // search
        let r = s.execute("presets.manager", &json!({"query": "p3"})).unwrap();
        assert_eq!(r["total"], 1, "{r}");
        assert!(s.execute("preset.hide", &json!({"id": "nope"})).is_err());
    }

    #[test]
    fn whole_preset_groups() {
        let mut s = session();
        s.execute("preset.groupHide", &json!({"group": "TestFilm"})).unwrap();
        assert!(s.presets.iter().filter(|p| p.group == "TestFilm").all(|p| !s.preset_shown(p)));
        s.execute("preset.groupRename", &json!({"group": "TestFilm", "name": "Analog"})).unwrap();
        assert_eq!(s.presets.iter().filter(|p| p.group == "Analog").count(), 3);
        s.execute("preset.groupNew", &json!({"name": "Clients"})).unwrap();
        assert_eq!(s.preset_groups().first().map(String::as_str), Some("Clients"));
        s.execute("preset.groupMove", &json!({"group": "TestWedding", "before": "Clients"})).unwrap();
        assert_eq!(s.preset_groups().first().map(String::as_str), Some("TestWedding"));
        s.execute("preset.groupRemove", &json!({"group": "Analog", "moveTo": "Clients"})).unwrap();
        assert_eq!(s.presets.iter().filter(|p| p.group == "Clients").count(), 3, "moved, not deleted");
        s.execute("preset.groupRemove", &json!({"group": "Clients"})).unwrap();
        assert!(!s.presets.iter().any(|p| p.group == "Clients"));
        // a built-in group: taken off this install, restorable
        let g = s.presets.iter().find(|p| p.builtin).unwrap().group.clone();
        let r = s.execute("preset.groupRemove", &json!({"group": g})).unwrap();
        assert!(r["removed"].as_u64().unwrap() > 0);
        assert!(!s.preset_groups().contains(&g));
        s.execute("preset.restoreBuiltins", &json!({})).unwrap();
        assert!(s.preset_groups().contains(&g));
    }

    /// 11 000 presets: listing a search's first page stays quick.
    #[test]
    fn a_big_collection_lists_quickly() {
        let mut s = Session::with_demo();
        for i in 0..11_000 {
            s.presets.push(Preset {
                id: format!("user.b{i}"),
                name: format!("Look {i}"),
                group: format!("Collection_{}", i % 50),
                settings: json!({"light": {"exposure": 0.1}}),
                favorite: false,
                builtin: false,
            });
        }
        let t0 = std::time::Instant::now();
        let r = s.execute("presets.manager", &json!({"query": "look 1", "limit": 100})).unwrap();
        assert!(r["total"].as_u64().unwrap() > 1000);
        assert!(t0.elapsed() < std::time::Duration::from_secs(5), "{:?}", t0.elapsed());
    }
}
