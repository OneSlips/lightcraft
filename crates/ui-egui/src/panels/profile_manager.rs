//! Settings ▸ Profiles: the Profile Manager. Every profile in one list — LightCraft's built-in looks,
//! the user's LUT profiles and camera profiles — grouped as the menus show them, each with a
//! favourite star, Rename, Group, Move up / down, Hide / Show and Delete; Import Profiles… and Import
//! Folder… (dropping files or a folder on the window imports them too); Restore Built-in Looks; the
//! default camera profile of each camera body; and where all of it is kept (shared by every library,
//! or with this one). Opened from Settings and from the profile browser's Manage button. Drawn as
//! the managers' table ([`super::manager_table`]), like the Presets Manager.

use egui::RichText;
use serde_json::{Value, json};

use super::manager_table::{self as table, Col, GroupAction, GroupHead, confirm_group_delete, group_header, new_group_row};
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
    /// Profiles selected for a drag (ids) and the one a Shift-click counts from (client note 9).
    selected: Vec<String>,
    anchor: Option<String>,
}

/// The profile list's key (its group headers' widgets, and its drag and drop).
const KEY: &str = "profileGroup";

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
    hint(
        ui,
        t,
        "Drag profiles to put them in order, or onto a group to move them there (Ctrl-click or Shift-click picks several; right-click for Move to Top or Bottom). Drag a group by its header.",
    );
    ui.add_space(4.0);

    let profiles: Vec<Value> = list["profiles"].as_array().cloned().unwrap_or_default();
    let groups: Vec<String> = list["groups"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut action: Option<(&'static str, Value)> = None;
    // the rows as listed (open groups), for Shift-click
    let order: Vec<String> = groups
        .iter()
        .filter(|g| table::group_open(ui.ctx(), KEY, g, true))
        .flat_map(|g| profiles.iter().filter(move |p| p["group"].as_str() == Some(g.as_str())).filter_map(|p| p["id"].as_str().map(str::to_string)))
        .collect();
    new_group_row(ui, t, "profileGroupNew", &mut ed.group_new, |name| action = Some(("profile.groupNew", json!({"name": name}))));
    let before = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
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
        ui.add_space(4.0);
        let (open, asked) = group_header(ui, t, &PROFILE_COLS, &head, &mut ed.group_rename);
        match asked {
            Some(GroupAction::DropItems(ids)) => action = Some(("profile.place", json!({"ids": ids, "group": g}))),
            Some(GroupAction::DropGroup { group, before }) => {
                let key = if before { "before" } else { "after" };
                action = Some(("profile.groupMove", json!({"group": group, key: g})));
            }
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
            profile_row(ui, t, p, i, &groups, (prev, next), &order, &mut ed, &mut action);
        }
    }
    table::drag_tick(ui.ctx(), t, KEY);
    ui.spacing_mut().item_spacing.y = before;
    ui.data_mut(|m| m.insert_temp(editing_id(), ed));
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
    bodies_section(app, ui, t);
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

/// The columns of the profiles' table.
const PROFILE_COLS: table::Columns = table::Columns { info_w: 250.0, for_import: false };

#[allow(clippy::too_many_arguments)]
fn profile_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    p: &Value,
    n: usize,
    groups: &[String],
    (prev, next): (Option<&str>, Option<&str>),
    order: &[String],
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
    let (rect, resp, rid) = table::row(ui, table::ROW_H, ("profile", &id));
    table::paint_item(ui, t, rect, n, resp.contains_pointer());
    // selecting, dragging and dropping (client note 9)
    let selected = ed.selected.contains(&id);
    if selected {
        table::paint_selected(ui, t, rect);
    }
    if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        table::select_click(&mut ed.selected, &mut ed.anchor, &id, order, m);
    }
    if resp.drag_started() && ed.rename.is_none() {
        let ids = if selected { ed.selected.clone() } else { vec![id.clone()] };
        table::drag_start(ui.ctx(), KEY, table::Dragged::Items(ids));
    }
    let group = p["group"].as_str().unwrap_or_default().to_string();
    if let Some((table::Dragged::Items(ids), at, true)) = table::drop_target(ui, t, KEY, rect, false)
        && !ids.contains(&id)
    {
        let key = if at == table::DropAt::Before { "before" } else { "after" };
        *action = Some(("profile.place", json!({"ids": ids, "group": group, key: id})));
    }
    resp.context_menu(|ui| {
        let ids = if selected { ed.selected.clone() } else { vec![id.clone()] };
        if ui.button(tr("Move to Top of Group")).clicked() {
            *action = Some(("profile.place", json!({"ids": ids, "group": group, "position": "top"})));
            ui.close();
        }
        if ui.button(tr("Move to Bottom of Group")).clicked() {
            *action = Some(("profile.place", json!({"ids": ids, "group": group, "position": "bottom"})));
            ui.close();
        }
    });
    let rr = table::RowRects::new(rect, rid, &PROFILE_COLS, table::ITEM_NAME_LEFT);
    let fav = rr.in_lead(ui, table::item_star(rect), |ui| {
        icon_button(ui, &format!("profileFav:{id}"), if favorite { Icon::StarFilled } else { Icon::Star }, sz, favorite, true, "Favorite")
    });
    if fav.clicked() {
        *action = Some(("profile.favorite", json!({"id": id})));
    }
    if let Some((rid, text)) = ed.rename.as_mut().filter(|(rid, _)| rid == &id) {
        let mut done = false;
        rr.in_name(ui, |ui| {
            let r = crate::text_field::TextField::singleline(&format!("field:profileRename:{rid}"), text)
                .width(rr.name.width().min(340.0))
                .select_on_focus(true)
                .show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                *action = Some(("profile.rename", json!({"id": id, "name": text.clone()})));
                done = true;
            } else if r.cancelled() {
                done = true;
            }
        });
        if done {
            ed.rename = None;
        }
    } else {
        table::text_in(ui, rr.name, &shown_name, t.font(13.0), if hidden { t.text_disabled } else { t.text });
        register(ui.ctx(), format!("profileRow:{id}"), rr.name);
    }
    let what = match kind {
        "camera" => p["cameraModel"].as_str().map(|m| format!("{} · {m}", tr("Camera profile"))).unwrap_or_else(|| tr("Camera profile").to_string()),
        "lut" => tr("LUT profile").to_string(),
        _ => tr("Built-in look").to_string(),
    };
    let mut info = what;
    // the bodies it is the default of (…71, …74)
    let defaults: Vec<String> =
        p["bodyDefaults"].as_array().into_iter().flatten().filter_map(Value::as_str).map(lightcraft_engine::cmd::dcp_profiles::body_label).collect();
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
    table::info_in(ui, t, rr.info, &info);
    if rr.in_col(ui, Col::Rename, |ui| icon_button(ui, &format!("profileRename:{id}"), Icon::Pencil, sz, false, true, "Rename")).clicked() {
        ed.rename = Some((id.clone(), name.to_string()));
    }
    let r = rr.in_col(ui, Col::Group, |ui| {
        crate::widgets::menu_button(ui, tr("Group"), |ui| {
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
        })
    });
    register(ui.ctx(), format!("profileGroup:{id}"), r.response.rect);
    if rr.in_col(ui, Col::Up, |ui| icon_button(ui, &format!("profileUp:{id}"), Icon::ChevronUp, sz, false, prev.is_some(), "Move Up")).clicked()
        && let Some(prev) = prev
    {
        *action = Some(("profile.reorder", json!({"id": id, "before": prev})));
    }
    if rr
        .in_col(ui, Col::Down, |ui| icon_button(ui, &format!("profileDown:{id}"), Icon::ChevronDown, sz, false, next.is_some(), "Move Down"))
        .clicked()
        && let Some(next) = next
    {
        *action = Some(("profile.reorder", json!({"id": id, "after": next})));
    }
    let (icon, tip) = if hidden { (Icon::EyeOff, "Show in the menus") } else { (Icon::Eye, "Hide from the menus") };
    if rr.in_col(ui, Col::Hide, |ui| icon_button(ui, &format!("profileHide:{id}"), icon, sz, hidden, true, tip)).clicked() {
        *action = Some(("profile.hide", json!({"id": id, "hidden": !hidden})));
    }
    if rr.in_col(ui, Col::Delete, |ui| icon_button(ui, &format!("profileDelete:{id}"), Icon::Trash, sz, false, true, "Delete")).clicked() {
        ed.confirm_delete = Some(id.clone());
    }
    // a new group's name
    let mut group_done = false;
    if let Some((gid, text)) = ed.new_group.as_mut().filter(|(gid, _)| gid == &id) {
        ui.horizontal(|ui| {
            ui.add_space(table::ITEM_NAME_LEFT);
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
    // asking before a delete: right under the row
    if ed.confirm_delete.as_deref() == Some(id.as_str()) {
        egui::Frame::NONE
            .fill(t.inset)
            .stroke(egui::Stroke::new(1.0, t.caution.gamma_multiply(0.7)))
            .corner_radius(4.0)
            .inner_margin(egui::Margin { left: 30, right: 8, top: 6, bottom: 6 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
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
