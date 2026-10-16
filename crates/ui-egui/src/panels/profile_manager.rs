//! Settings ▸ Profiles: the Profile Manager. Every profile in one list — LightCraft's built-in looks,
//! the user's LUT profiles and camera profiles — grouped as the menus show them, each with a
//! favourite star, Rename, Group, Move up / down, Hide / Show and Delete; Import Profiles… and Import
//! Folder… (dropping files or a folder on the window imports them too); Restore Built-in Looks; the
//! default camera profile of each camera body; and where all of it is kept (shared by every library,
//! or with this one). Opened from Settings and from the profile browser's Manage button.

use egui::RichText;
use serde_json::{Value, json};

use super::settings::{check, heading, hint, row};
use crate::LightcraftApp;
use crate::i18n::{tr, tr_format};
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// What a row is doing: renaming, choosing a new group, or asking before a delete (by profile id).
#[derive(Clone, Default)]
struct Editing {
    rename: Option<(String, String)>,
    new_group: Option<(String, String)>,
    confirm_delete: Option<String>,
    /// A group being renamed (group, new name), and the name of a new group being typed.
    group_rename: Option<(String, String)>,
    group_new: Option<String>,
    /// A group whose Delete asks first, and where its own profiles would go instead.
    group_confirm: Option<String>,
    group_move_to: Option<String>,
}

fn editing_id() -> egui::Id {
    egui::Id::new("profileManagerEditing")
}

pub fn settings_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let list = app.session.execute("profiles.manager", &json!({})).unwrap_or(Value::Null);
    let mut ed: Editing = ui.data(|m| m.get_temp(editing_id())).unwrap_or_default();
    store_section(app, ui, t, &list);

    heading(ui, t, "Profiles");
    hint(
        ui,
        t,
        "Hide takes a profile out of the menus and the browser; it stays here, one click to show it again. Delete removes your own profile files, and takes a built-in look off this install (Restore Built-in Looks brings them back). Photos that use a hidden or deleted built-in look keep it.",
    );
    ui.horizontal_wrapped(|ui| {
        let can_files = app.services.pick_preset_files.is_some();
        let r = ui.add_enabled(can_files, egui::Button::new(tr("Import Profiles…")));
        register(ui.ctx(), "button:profilesImportFiles", r.rect);
        if r.clicked()
            && let Some(paths) = app.services.pick_preset_files.as_mut().map(|f| f())
        {
            import(app, paths);
        }
        let r = ui.add_enabled(app.services.pick_folder.is_some(), egui::Button::new(tr("Import Folder…")));
        register(ui.ctx(), "button:profilesImportFolder", r.rect);
        if r.clicked()
            && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
        {
            import(app, vec![dir]);
        }
        let removed = list["removedBuiltins"].as_u64().unwrap_or(0);
        if removed > 0 {
            let r = ui.button(tr_format!("Restore Built-in Looks ({n})", n = removed));
            register(ui.ctx(), "button:profilesRestoreBuiltins", r.rect);
            if r.clicked() {
                let _ = app.run("profile.restoreBuiltins", json!({}));
            }
        }
    });
    hint(ui, t, "DNG camera profiles (.dcp) and 3D LUTs (.cube), single files, folders or .zip packs. You can also drop them on this window.");
    ui.add_space(4.0);

    let profiles: Vec<Value> = list["profiles"].as_array().cloned().unwrap_or_default();
    let groups: Vec<String> = list["groups"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut action: Option<(&'static str, Value)> = None;
    new_group_row(ui, t, "profileGroupNew", &mut ed.group_new, |name| action = Some(("profile.groupNew", json!({"name": name}))));
    for (gi, g) in groups.iter().enumerate() {
        let members: Vec<&Value> = profiles.iter().filter(|p| p["group"].as_str() == Some(g.as_str())).collect();
        let builtin_group = !members.is_empty() && members.iter().all(|p| p["kind"] == "builtin" && p["customGroup"] != true);
        let own = members.iter().filter(|p| p["kind"] != "builtin").count();
        let head = GroupHead {
            key: "profileGroup",
            name: g,
            shown: crate::i18n::builtin_label(g, builtin_group).to_string(),
            count: members.len(),
            all_hidden: !members.is_empty() && members.iter().all(|p| p["hidden"] == true),
            prev: gi.checked_sub(1).and_then(|j| groups.get(j)).map(String::as_str),
            next: groups.get(gi + 1).map(String::as_str),
            default_open: true,
        };
        ui.add_space(6.0);
        let (open, asked) = group_header(ui, t, &head, &mut ed.group_rename);
        match asked {
            Some(GroupAction::Hide(hidden)) => action = Some(("profile.groupHide", json!({"group": g, "hidden": hidden}))),
            Some(GroupAction::Rename(name)) => action = Some(("profile.groupRename", json!({"group": g, "name": name}))),
            Some(GroupAction::Move { before, other }) => {
                let key = if before { "before" } else { "after" };
                action = Some(("profile.groupMove", json!({"group": g, key: other})));
            }
            Some(GroupAction::Delete) => {
                ed.group_confirm = Some(g.clone());
                ed.group_move_to = None;
            }
            None => {}
        }
        if ed.group_confirm.as_deref() == Some(g.as_str()) {
            let others: Vec<String> = groups.iter().filter(|o| *o != g).cloned().collect();
            let counts = (members.len() - own, own);
            if let Some(params) = confirm_group_delete(ui, t, g, &head.shown, counts, &others, &mut ed.group_move_to, &mut ed.group_confirm) {
                action = Some(("profile.groupRemove", params));
            }
        }
        if !open {
            continue;
        }
        for (i, p) in members.iter().enumerate() {
            let prev = i.checked_sub(1).and_then(|j| members.get(j)).and_then(|q| q["id"].as_str());
            let next = members.get(i + 1).and_then(|q| q["id"].as_str());
            profile_row(ui, t, p, &groups, prev, next, &mut ed, &mut action);
        }
    }
    ui.data_mut(|m| m.insert_temp(editing_id(), ed));
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
    bodies_section(app, ui, t);
}

/// A group's header row as the managers (profiles, presets) draw it.
pub(super) struct GroupHead<'a> {
    /// Widget id prefix (`{key}Fold:{name}`, `{key}Hide:{name}`…).
    pub key: &'a str,
    pub name: &'a str,
    /// The name as shown (a built-in group's in the UI language).
    pub shown: String,
    pub count: usize,
    pub all_hidden: bool,
    pub prev: Option<&'a str>,
    pub next: Option<&'a str>,
    /// Open until the user folds it.
    pub default_open: bool,
}

/// What a group header asked for.
pub(super) enum GroupAction {
    Hide(bool),
    Rename(String),
    Move { before: bool, other: String },
    Delete,
}

/// A group's header: collapse / expand, its name and count, Rename, Move up / down, Hide / Show and
/// Delete. Returns whether it is open, and what was asked.
pub(super) fn group_header(ui: &mut egui::Ui, t: &Tokens, h: &GroupHead, renaming: &mut Option<(String, String)>) -> (bool, Option<GroupAction>) {
    let open_id = egui::Id::new((h.key, "open", h.name));
    let open: bool = ui.data(|m| m.get_temp(open_id)).unwrap_or(h.default_open);
    let mut out = None;
    let sz = egui::vec2(24.0, 24.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let fold = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        if icon_button(ui, &format!("{}Fold:{}", h.key, h.name), fold, sz, false, true, if open { "Collapse" } else { "Expand" }).clicked() {
            ui.data_mut(|m| m.insert_temp(open_id, !open));
        }
        let mut done = None;
        if let Some((rg, text)) = renaming.as_mut().filter(|(rg, _)| rg == h.name) {
            let r =
                crate::text_field::TextField::singleline(&format!("field:{}Rename:{rg}", h.key), text).width(240.0).select_on_focus(true).show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                done = Some(Some(text.trim().to_string()).filter(|n| !n.is_empty() && n != h.name));
            } else if r.cancelled() {
                done = Some(None);
            }
        } else {
            let color = if h.all_hidden { t.text_disabled } else { t.text_label };
            let r = ui.label(RichText::new(&h.shown).font(t.semibold(13.0)).color(color));
            register(ui.ctx(), format!("{}Head:{}", h.key, h.name), r.rect);
            ui.label(RichText::new(h.count.to_string()).size(11.0).color(t.text_dim));
        }
        if let Some(name) = done {
            *renaming = None;
            if let Some(n) = name {
                out = Some(GroupAction::Rename(n));
            }
        }
        ui.add_space(8.0);
        if icon_button(ui, &format!("{}Rename:{}", h.key, h.name), Icon::Pencil, sz, false, true, "Rename Group").clicked() {
            *renaming = Some((h.name.to_string(), h.name.to_string()));
        }
        if icon_button(ui, &format!("{}Up:{}", h.key, h.name), Icon::ChevronUp, sz, false, h.prev.is_some(), "Move Group Up").clicked()
            && let Some(o) = h.prev
        {
            out = Some(GroupAction::Move { before: true, other: o.to_string() });
        }
        if icon_button(ui, &format!("{}Down:{}", h.key, h.name), Icon::ChevronDown, sz, false, h.next.is_some(), "Move Group Down").clicked()
            && let Some(o) = h.next
        {
            out = Some(GroupAction::Move { before: false, other: o.to_string() });
        }
        let (icon, tip) = if h.all_hidden { (Icon::EyeOff, "Show the group in the menus") } else { (Icon::Eye, "Hide the group from the menus") };
        if icon_button(ui, &format!("{}Hide:{}", h.key, h.name), icon, sz, h.all_hidden, h.count > 0, tip).clicked() {
            out = Some(GroupAction::Hide(!h.all_hidden));
        }
        if icon_button(ui, &format!("{}Delete:{}", h.key, h.name), Icon::Trash, sz, false, true, "Delete Group").clicked() {
            out = Some(GroupAction::Delete);
        }
    });
    (open, out)
}

/// "New Group…" and its name field; `create` gets the name typed.
pub(super) fn new_group_row(ui: &mut egui::Ui, t: &Tokens, key: &str, typing: &mut Option<String>, create: impl FnOnce(String)) {
    ui.horizontal(|ui| match typing.as_mut() {
        Some(text) => {
            ui.label(RichText::new(tr("New group:")).color(t.text_label));
            let r = crate::text_field::TextField::singleline(&format!("field:{key}"), text).width(240.0).show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                let name = text.trim().to_string();
                *typing = None;
                if !name.is_empty() {
                    create(name);
                }
            } else if r.cancelled() {
                *typing = None;
            }
        }
        None => {
            let r = ui.button(tr("New Group…"));
            register(ui.ctx(), format!("button:{key}"), r.rect);
            if r.clicked() {
                *typing = Some(String::new());
            }
        }
    });
}

/// Asking before a group is deleted: what happens to its built-in and own items (`counts`), and
/// (with own items) a group to move them to instead. Returns the `groupRemove` parameters once
/// confirmed.
#[allow(clippy::too_many_arguments)]
pub(super) fn confirm_group_delete(
    ui: &mut egui::Ui,
    t: &Tokens,
    group: &str,
    shown: &str,
    counts: (usize, usize),
    others: &[String],
    move_to: &mut Option<String>,
    confirm: &mut Option<String>,
) -> Option<Value> {
    let (builtin, own) = counts;
    let mut out = None;
    egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        let mut text = tr_format!("Delete the group “{name}”?", name = shown);
        if builtin > 0 {
            text.push(' ');
            text.push_str(&tr_format!("{n} built-in items are taken off this computer (Restore brings them back).", n = builtin));
        }
        if own > 0 {
            text.push(' ');
            text.push_str(&tr_format!("{n} of your own: delete them, or move them to another group.", n = own));
        }
        ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
        ui.horizontal(|ui| {
            if own > 0 && !others.is_empty() {
                let label = move_to.clone().unwrap_or_else(|| tr("Move to…").to_string());
                let r = crate::widgets::combo_box(("groupMoveTo", group)).width(200.0).selected_text(label).show_ui(ui, |ui| {
                    for o in others {
                        if ui.selectable_label(move_to.as_deref() == Some(o.as_str()), o.as_str()).clicked() {
                            *move_to = Some(o.clone());
                        }
                    }
                });
                register(ui.ctx(), format!("combo:groupMoveTo:{group}"), r.response.rect);
                if let Some(to) = move_to.clone() {
                    let r = ui.button(tr("Move Them and Delete the Group"));
                    register(ui.ctx(), format!("button:groupMove:{group}"), r.rect);
                    if r.clicked() {
                        out = Some(json!({"group": group, "moveTo": to}));
                        *confirm = None;
                    }
                }
            }
            let r = ui.button(if own > 0 { tr("Delete Everything") } else { tr("Delete") });
            register(ui.ctx(), format!("button:groupDeleteConfirm:{group}"), r.rect);
            if r.clicked() {
                out = Some(json!({"group": group}));
                *confirm = None;
            }
            let r = ui.button(tr("Cancel"));
            register(ui.ctx(), format!("button:groupDeleteCancel:{group}"), r.rect);
            if r.clicked() {
                *confirm = None;
            }
        });
    });
    out
}

/// Import profile files or folders and say what happened.
fn import(app: &mut LightcraftApp, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    match app.session.execute("profile.import", &json!({"paths": paths})) {
        Ok(v) => {
            let n = v["imported"].as_array().map_or(0, Vec::len);
            let failed = v["failed"].as_array().map_or(0, Vec::len);
            app.ui.status = if failed > 0 {
                tr_format!("Imported {n} profiles; {failed} couldn't be read", n = n, failed = failed)
            } else {
                tr_format!("Imported {n} profiles", n = n)
            };
        }
        Err(e) => app.ui.status = e.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    p: &Value,
    groups: &[String],
    prev: Option<&str>,
    next: Option<&str>,
    ed: &mut Editing,
    action: &mut Option<(&'static str, Value)>,
) {
    let id = p["id"].as_str().unwrap_or_default().to_string();
    let name = p["name"].as_str().unwrap_or_default();
    let kind = p["kind"].as_str().unwrap_or_default();
    let builtin = kind == "builtin";
    let hidden = p["hidden"] == true;
    let favorite = p["favorite"] == true;
    let used = p["usedBy"].as_u64().unwrap_or(0);
    let shown_name = if builtin && p["name"] == p["originalName"] { tr(name).to_string() } else { name.to_string() };
    let sz = egui::vec2(24.0, 24.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(ui, &format!("profileFav:{id}"), if favorite { Icon::StarFilled } else { Icon::Star }, sz, favorite, true, "Favorite")
            .clicked()
        {
            *action = Some(("profile.favorite", json!({"id": id})));
        }
        let name_w = (ui.available_width() - 390.0).clamp(140.0, 340.0);
        if let Some((rid, text)) = ed.rename.as_mut().filter(|(rid, _)| rid == &id) {
            let r =
                crate::text_field::TextField::singleline(&format!("field:profileRename:{rid}"), text).width(name_w).select_on_focus(true).show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                *action = Some(("profile.rename", json!({"id": id, "name": text.clone()})));
                ed.rename = None;
            } else if r.cancelled() {
                ed.rename = None;
            }
        } else {
            let color = if hidden { t.text_disabled } else { t.text };
            let (r, _) = ui.allocate_exact_size(egui::vec2(name_w, 22.0), egui::Sense::hover());
            let g = crate::widgets::one_line(ui.painter(), &shown_name, t.font(13.0), color, name_w - 4.0);
            ui.painter().galley(egui::pos2(r.left(), r.center().y - g.size().y / 2.0), g, color);
            register(ui.ctx(), format!("profileRow:{id}"), r);
        }
        let what = match kind {
            "camera" => {
                p["cameraModel"].as_str().map(|m| format!("{} · {m}", tr("Camera profile"))).unwrap_or_else(|| tr("Camera profile").to_string())
            }
            "lut" => tr("LUT profile").to_string(),
            _ => tr("Built-in look").to_string(),
        };
        let mut info = what;
        // the bodies it is the default of (…71, …74)
        let defaults: Vec<String> = p["bodyDefaults"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(lightcraft_engine::cmd::dcp_profiles::body_label)
            .collect();
        if !defaults.is_empty() {
            info.push_str(&format!(" · {}", tr_format!("default for {bodies}", bodies = defaults.join(", "))));
        }
        if used > 0 {
            info.push_str(&format!(" · {}", tr_format!("{n} photos here", n = used)));
        }
        if hidden {
            info.push_str(&format!(" · {}", tr("Hidden")));
        }
        if p["missing"] == true {
            info.push_str(&format!(" · {}", tr("file missing")));
        }
        ui.add_sized(egui::vec2(230.0, 22.0), egui::Label::new(RichText::new(info).size(11.0).color(t.text_dim)).truncate());
        if icon_button(ui, &format!("profileRename:{id}"), Icon::Pencil, sz, false, true, "Rename").clicked() {
            ed.rename = Some((id.clone(), name.to_string()));
        }
        let r = crate::widgets::menu_button(ui, tr("Group"), |ui| {
            ui.set_min_width(180.0);
            for g in groups {
                if ui.button(g.as_str()).clicked() {
                    *action = Some(("profile.setGroup", json!({"id": id, "group": g})));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tr("New Group…")).clicked() {
                ed.new_group = Some((id.clone(), String::new()));
                ui.close();
            }
            if p["customGroup"] == true && ui.button(tr("Back to Its Own Group")).clicked() {
                *action = Some(("profile.setGroup", json!({"id": id, "group": ""})));
                ui.close();
            }
        });
        register(ui.ctx(), format!("profileGroup:{id}"), r.response.rect);
        if icon_button(ui, &format!("profileUp:{id}"), Icon::ChevronUp, sz, false, prev.is_some(), "Move Up").clicked()
            && let Some(prev) = prev
        {
            *action = Some(("profile.reorder", json!({"id": id, "before": prev})));
        }
        if icon_button(ui, &format!("profileDown:{id}"), Icon::ChevronDown, sz, false, next.is_some(), "Move Down").clicked()
            && let Some(next) = next
        {
            *action = Some(("profile.reorder", json!({"id": id, "after": next})));
        }
        let (icon, tip) = if hidden { (Icon::EyeOff, "Show in the menus") } else { (Icon::Eye, "Hide from the menus") };
        if icon_button(ui, &format!("profileHide:{id}"), icon, sz, hidden, true, tip).clicked() {
            *action = Some(("profile.hide", json!({"id": id, "hidden": !hidden})));
        }
        if icon_button(ui, &format!("profileDelete:{id}"), Icon::Trash, sz, false, true, "Delete").clicked() {
            ed.confirm_delete = Some(id.clone());
        }
    });
    // a new group's name
    let mut group_done = false;
    if let Some((gid, text)) = ed.new_group.as_mut().filter(|(gid, _)| gid == &id) {
        ui.horizontal(|ui| {
            ui.add_space(30.0);
            ui.label(RichText::new(tr("New group:")).color(t.text_label));
            let r = crate::text_field::TextField::singleline(&format!("field:profileNewGroup:{gid}"), text).width(220.0).show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() && !text.trim().is_empty() {
                *action = Some(("profile.setGroup", json!({"id": id, "group": text.trim()})));
                group_done = true;
            } else if r.cancelled() {
                group_done = true;
            }
        });
    }
    if group_done {
        ed.new_group = None;
    }
    // asking before a delete
    if ed.confirm_delete.as_deref() == Some(id.as_str()) {
        egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
            let text = if builtin {
                tr_format!("Remove “{name}” from LightCraft on this computer? Restore Built-in Looks brings it back.", name = shown_name)
            } else if used > 0 {
                tr_format!(
                    "Delete “{name}” and its file? {n} photos here use it; camera profiles fall back to LightCraft's camera colour.",
                    name = shown_name,
                    n = used
                )
            } else {
                tr_format!("Delete “{name}” and its file?", name = shown_name)
            };
            ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
            ui.horizontal(|ui| {
                let r = ui.button(tr("Delete"));
                register(ui.ctx(), format!("button:profileDeleteConfirm:{id}"), r.rect);
                if r.clicked() {
                    *action = Some(("profile.remove", json!({"id": id})));
                    ed.confirm_delete = None;
                }
                let r = ui.button(tr("Cancel"));
                register(ui.ctx(), format!("button:profileDeleteCancel:{id}"), r.rect);
                if r.clicked() {
                    ed.confirm_delete = None;
                }
            });
        });
    }
}

/// Where profiles, presets and effects are kept, and "keep with this library".
fn store_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, list: &Value) {
    let info = app.session.execute("store.info", &json!({})).unwrap_or(Value::Null);
    if info["available"] != true {
        return;
    }
    heading(ui, t, "Shared by All Libraries");
    let shared = list["store"] == "shared";
    let text = if shared {
        tr_format!(
            "Your profiles, presets ({presets}), local adjustment presets ({effects}), import settings and camera body defaults are kept in LightCraft's settings folder, so every library has them.",
            presets = info["presets"].as_u64().unwrap_or(0),
            effects = info["effects"].as_u64().unwrap_or(0)
        )
    } else {
        tr("This library keeps its own profiles, presets, local adjustment presets and body defaults. Other libraries don't see them.").to_string()
    };
    hint(ui, t, &text);
    let mut own = !shared;
    if app.session.library.as_ref().is_some_and(|l| l.on_disk)
        && check(ui, "storeWithLibrary", &mut own, "Store presets and profiles with this library instead")
        && let Err(e) = app.run("store.useLibrary", json!({"keep": own}))
    {
        app.ui.status = e;
    }
}

/// The default camera profile of each camera body.
fn bodies_section(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let v = app.session.execute("profiles.bodies", &json!({})).unwrap_or(Value::Null);
    let bodies: Vec<Value> = v["bodies"].as_array().cloned().unwrap_or_default();
    heading(ui, t, "Camera Bodies");
    if bodies.is_empty() {
        hint(ui, t, "Bodies appear here once raw photos from them are in this library, or a camera profile names them.");
        return;
    }
    hint(
        ui,
        t,
        "The camera profile each body's raw photos get when they are imported. Photos already in the library keep theirs until you apply the defaults.",
    );
    let mut action: Option<(&'static str, Value)> = None;
    for b in &bodies {
        let body = b["body"].as_str().unwrap_or_default().to_string();
        let mut label = format!("{} · {}", b["label"].as_str().unwrap_or_default(), b["camera"].as_str().unwrap_or_default());
        let photos = b["photos"].as_u64().unwrap_or(0);
        if photos > 0 {
            label.push_str(&format!(" · {}", tr_format!("{n} photos here", n = photos)));
        }
        row(ui, t, &label, |ui| {
            let current = b["profile"].as_str().map(str::to_string);
            let builtin = tr("LightCraft camera colour").to_string();
            let text = b["profileName"].as_str().map(str::to_string).unwrap_or_else(|| builtin.clone());
            let r = crate::widgets::combo_box(("bodyDefault", body.as_str())).width(280.0).selected_text(text).show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), builtin.as_str()).clicked() {
                    action = Some(("profile.clearBodyDefault", json!({"serial": body})));
                }
                for p in b["profiles"].as_array().into_iter().flatten() {
                    let pid = p["id"].as_str().unwrap_or_default();
                    if ui.selectable_label(current.as_deref() == Some(pid), p["name"].as_str().unwrap_or_default()).clicked() {
                        action = Some(("profile.setBodyDefault", json!({"profile": pid, "serial": body})));
                    }
                }
            });
            register(ui.ctx(), format!("combo:bodyDefault:{body}"), r.response.rect);
        });
    }
    row(ui, t, "", |ui| {
        let r = ui.button(tr("Apply to Photos in View"));
        register(ui.ctx(), "button:applyBodyDefaults", r.rect);
        if r.clicked() {
            action = Some(("profile.applyBodyDefaults", json!({})));
        }
        hint(ui, t, "Only photos still on the default profile change; one undo step.");
    });
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
}

/// Dropped files or folders while the Profiles tab shows: profiles, not photos.
pub fn takes_drops(app: &LightcraftApp) -> bool {
    matches!(&app.ui.dialog, Some(crate::state::Dialog::Settings { tab }) if tab == "profiles")
}

/// Import dropped profile files or folders.
pub fn drop_import(app: &mut LightcraftApp, paths: Vec<String>) {
    import(app, paths);
}
