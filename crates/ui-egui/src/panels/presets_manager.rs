//! Settings ▸ Presets: the Presets Manager. Develop presets (the user's, often thousands in
//! Lightroom groups, and LightCraft's own with the Smart Masks), local adjustment presets and import
//! presets, each in one searchable list.
//!
//! Per preset: favourite, rename, move to a group, move up / down (presets of your own), hide and
//! delete. Per group: collapse, rename, move up / down, hide, delete (or move its presets elsewhere),
//! and New Group…. Import presets (files, folders, Lightroom preset folders, .zip; dropping them on
//! the window too) and export them (LightCraft `.lcpreset`, Lightroom `.xmp`). Search by name and
//! group, and "with AI masks only". Drawn as the managers' table ([`super::manager_table`]); a
//! group's Delete asks at the group. Only the rows in view are built, so a collection of 11 000+
//! presets scrolls smoothly; the rows are rebuilt only when the list, the choices or the search
//! change.

use std::sync::Arc;

use serde_json::{Value, json};

use super::manager_table::{self as table, Col, GroupAction, GroupHead, confirm_group_delete, group_header, new_group_row};
use super::settings::{check, choices, hint};
use crate::LightcraftApp;
use crate::i18n::{tr, tr_format};
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// Which presets the manager shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Develop,
    Effects,
    Import,
}

/// The manager's own state between frames.
#[derive(Clone, Default)]
struct State {
    kind: Kind,
    query: String,
    ai_only: bool,
    /// A row being renamed (id, text) and a group being renamed (group, text).
    rename: Option<(String, String)>,
    group_rename: Option<(String, String)>,
    group_new: Option<String>,
    /// A preset whose Delete asks first (by id).
    confirm: Option<String>,
    /// A group whose Delete asks first, and where its own presets would go.
    group_confirm: Option<String>,
    group_move_to: Option<String>,
}

fn state_id() -> egui::Id {
    egui::Id::new("presetsManager")
}

/// One row of the list.
#[derive(Clone)]
enum Row {
    /// A group: its name, shown name, members, all hidden.
    Group { name: String, shown: String, count: usize, hidden: bool },
    /// Asking before this group is deleted (under its header).
    Confirm(String),
    /// A preset (develop: index into the session's presets; effects: index into its effects), and
    /// its place in its group.
    Item(usize, usize),
}

pub fn settings_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut st: State = ui.data(|m| m.get_temp(state_id())).unwrap_or_default();
    ui.horizontal(|ui| {
        choices(
            ui,
            "presetsKind",
            &[(Kind::Develop, "Develop Presets"), (Kind::Effects, "Local Adjustment Presets"), (Kind::Import, "Import Presets")],
            &mut st.kind,
        );
    });
    ui.add_space(4.0);
    match st.kind {
        Kind::Develop => develop(app, ui, t, &mut st),
        Kind::Effects => effects(app, ui, t, &mut st),
        Kind::Import => import_presets(app, ui, t, &mut st),
    }
    ui.data_mut(|m| m.insert_temp(state_id(), st));
}

/// The search field and "with AI masks only"; true when either changed.
fn search_row(ui: &mut egui::Ui, st: &mut State, ai: bool) -> bool {
    let before = (st.query.clone(), st.ai_only);
    ui.horizontal(|ui| {
        crate::text_field::TextField::singleline("field:presetsSearch", &mut st.query).hint(tr("Search by name or group")).width(260.0).show(ui);
        if ai {
            check(ui, "presetsAiOnly", &mut st.ai_only, "With AI masks only");
        }
    });
    before != (st.query.clone(), st.ai_only)
}

/// The develop presets' rows, rebuilt only when the list, the choices, the search, a group's open
/// state or the group asking before a delete changed.
fn develop_rows(ui: &egui::Ui, app: &LightcraftApp, st: &State) -> Arc<Vec<Row>> {
    let s = &app.session;
    let groups = s.preset_groups();
    let open: Vec<bool> = groups.iter().map(|g| table::group_open(ui.ctx(), "presetGroup", g, false)).collect();
    let key: RowsKey = (s.presets.generation(), s.preset_state_gen, st.query.clone(), st.ai_only, open.clone(), st.group_confirm.clone());
    let id = egui::Id::new("presetsManagerRows");
    if let Some((k, rows)) = ui.data(|m| m.get_temp::<(RowsKey, Arc<Vec<Row>>)>(id))
        && k == key
    {
        return rows;
    }
    let q = st.query.trim().to_lowercase();
    let searching = !q.is_empty() || st.ai_only;
    let mut rows = Vec::new();
    for (g, is_open) in groups.iter().zip(open) {
        let members: Vec<usize> = s.presets.iter().enumerate().filter(|(_, p)| &p.group == g && s.preset_listed(p)).map(|(i, _)| i).collect();
        let hits: Vec<usize> = members
            .iter()
            .copied()
            .filter(|i| {
                s.presets.get(*i).is_some_and(|p| {
                    (q.is_empty() || p.name.to_lowercase().contains(&q) || p.group.to_lowercase().contains(&q)) && (!st.ai_only || p.has_ai_masks())
                })
            })
            .collect();
        if searching && hits.is_empty() && st.group_confirm.as_deref() != Some(g.as_str()) {
            continue;
        }
        let builtin = members.iter().filter(|i| s.presets.get(**i).is_some_and(|p| p.builtin)).count();
        let hidden = !members.is_empty() && members.iter().all(|i| s.presets.get(*i).is_some_and(|p| !s.preset_shown(p)));
        let shown = crate::i18n::builtin_label(g, builtin > 0 && builtin == members.len()).to_string();
        rows.push(Row::Group { name: g.clone(), shown, count: members.len(), hidden });
        // asking before the group goes: right under its header, where it was asked
        if st.group_confirm.as_deref() == Some(g.as_str()) {
            rows.push(Row::Confirm(g.clone()));
        }
        // a search opens every group it finds something in
        if is_open || searching {
            rows.extend(hits.into_iter().enumerate().map(|(n, i)| Row::Item(i, n)));
        }
    }
    let rows = Arc::new(rows);
    ui.ctx().data_mut(|m| m.insert_temp(id, (key, rows.clone())));
    rows
}

/// What the develop rows were built from.
type RowsKey = (u64, u64, String, bool, Vec<bool>, Option<String>);

/// The heights of `rows` (a confirmation is taller).
fn heights(ctx: &egui::Context, rows: &[Row]) -> Vec<f32> {
    let confirm = table::confirm_h(ctx);
    rows.iter().map(|r| if matches!(r, Row::Confirm(_)) { confirm } else { table::ROW_H }).collect()
}

/// The columns of the develop presets' table.
const DEVELOP_COLS: table::Columns = table::Columns { info_w: 130.0, for_import: true };

fn develop(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, st: &mut State) {
    hint(
        ui,
        t,
        "Every develop preset, yours and LightCraft's (the Smart Masks too), shared by all your libraries. Hide takes a preset out of the Presets panel; it stays here. Delete removes your own presets, and takes a built-in one off this computer (Restore Built-in Presets brings them back).",
    );
    let removed = app.session.preset_state.removed.len();
    let mut action: Option<(&'static str, Value)> = None;
    ui.horizontal_wrapped(|ui| {
        let r = ui.button(tr("Import Presets…"));
        register(ui.ctx(), "button:presetsImport", r.rect);
        if r.clicked() {
            action = Some(("file.importPresets", json!({})));
        }
        let r = ui.add_enabled(app.services.pick_folder.is_some(), egui::Button::new(tr("Import Folder…")));
        register(ui.ctx(), "button:presetsImportFolder", r.rect);
        if r.clicked()
            && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
        {
            action = Some(("file.importPresets", json!({"paths": [dir]})));
        }
        let any_user = app.session.presets.iter().any(|p| !p.builtin);
        let r = ui.add_enabled(any_user, egui::Button::new(tr("Export…")));
        register(ui.ctx(), "button:presetsExport", r.rect);
        if r.clicked() {
            action = Some(("file.exportPresets", json!({})));
        }
        let r = ui.add_enabled(any_user && app.services.pick_folder.is_some(), egui::Button::new(tr("Export for Lightroom…")));
        register(ui.ctx(), "button:presetsExportXmp", r.rect);
        if r.clicked()
            && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
        {
            let ids: Vec<String> = app.session.presets.iter().filter(|p| !p.builtin).map(|p| p.id.clone()).collect();
            action = Some(("preset.exportXmp", json!({"ids": ids, "folder": dir})));
        }
        if removed > 0 {
            let r = ui.button(tr_format!("Restore Built-in Presets ({n})", n = removed));
            register(ui.ctx(), "button:presetsRestoreBuiltins", r.rect);
            if r.clicked() {
                action = Some(("preset.restoreBuiltins", json!({})));
            }
        }
    });
    hint(
        ui,
        t,
        "Lightroom .xmp and .lrtemplate presets, .lcpreset files, folders (Lightroom's preset folders keep their groups) and .zip packs. You can also drop them on this window.",
    );
    search_row(ui, st, true);
    new_group_row(ui, t, "presetGroupNew", &mut st.group_new, |name| action = Some(("preset.groupNew", json!({"name": name}))));
    let rows = develop_rows(ui, app, st);
    let groups: Vec<String> = rows.iter().filter_map(|r| if let Row::Group { name, .. } = r { Some(name.clone()) } else { None }).collect();
    let all_groups = app.session.preset_groups();
    // the rest of the window (the tab scrolls only for the controls above)
    let height = (ui.clip_rect().bottom() - ui.cursor().top() - 8.0).max(360.0);
    let hs = heights(ui.ctx(), &rows);
    table::virtual_list(ui, "presetsManagerList", &hs, height, |ui, i| {
        let Some(row) = rows.get(i) else { return };
        let prev_group = |name: &str| groups.iter().position(|g| g == name).and_then(|j| j.checked_sub(1)).and_then(|j| groups.get(j));
        let next_group = |name: &str| groups.iter().position(|g| g == name).and_then(|j| groups.get(j + 1));
        match row {
            Row::Group { name, shown, count, hidden } => {
                let head = GroupHead {
                    key: "presetGroup",
                    name,
                    shown: shown.clone(),
                    count: *count,
                    all_hidden: *hidden,
                    prev: prev_group(name).map(String::as_str),
                    next: next_group(name).map(String::as_str),
                    default_open: false,
                };
                match group_header(ui, t, &DEVELOP_COLS, &head, &mut st.group_rename).1 {
                    Some(GroupAction::Hide(h)) => action = Some(("preset.groupHide", json!({"group": name, "hidden": h}))),
                    Some(GroupAction::Rename(n)) => action = Some(("preset.groupRename", json!({"group": name, "name": n}))),
                    Some(GroupAction::Move { before, other }) => {
                        let key = if before { "before" } else { "after" };
                        action = Some(("preset.groupMove", json!({"group": name, key: other})));
                    }
                    Some(GroupAction::Delete) => {
                        st.group_confirm = Some(name.clone());
                        st.group_move_to = None;
                    }
                    None => {}
                }
            }
            Row::Confirm(g) => {
                let members: Vec<&lightcraft_develop::Preset> =
                    app.session.presets.iter().filter(|p| &p.group == g && app.session.preset_listed(p)).collect();
                let builtin = members.iter().filter(|p| p.builtin).count();
                let others: Vec<String> = all_groups.iter().filter(|o| *o != g).cloned().collect();
                let shown = crate::i18n::builtin_label(g, builtin > 0 && builtin == members.len()).to_string();
                let counts = (builtin, members.len() - builtin);
                if let Some(params) = confirm_group_delete(ui, t, g, &shown, counts, &others, &mut st.group_move_to, &mut st.group_confirm) {
                    action = Some(("preset.groupRemove", params));
                }
            }
            Row::Item(ix, n) => {
                if let Some(a) = develop_row(app, ui, t, st, (*ix, *n), &rows, i, &all_groups) {
                    action = Some(a);
                }
            }
        }
    });
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
}

/// The neighbour of row `i` among the presets of the same group (for Move Up / Down).
fn neighbour(app: &LightcraftApp, rows: &[Row], i: usize, up: bool) -> Option<String> {
    let j = if up { i.checked_sub(1)? } else { i + 1 };
    match rows.get(j)? {
        Row::Item(k, _) => app.session.presets.get(*k).filter(|p| !p.builtin).map(|p| p.id.clone()),
        Row::Group { .. } | Row::Confirm(_) => None,
    }
}

/// One preset's row: favourite, name, details, then Rename, Group, Move Up / Down, Hide and Delete
/// in their columns. `(ix, n)`: the preset's index, and its place in its group.
#[allow(clippy::too_many_arguments)]
fn develop_row(
    app: &LightcraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    st: &mut State,
    (ix, n): (usize, usize),
    rows: &[Row],
    i: usize,
    groups: &[String],
) -> Option<(&'static str, Value)> {
    let p = app.session.presets.get(ix)?;
    let (id, builtin) = (p.id.clone(), p.builtin);
    let hidden = !app.session.preset_shown(p);
    let name = if builtin && !app.session.preset_state.names.contains_key(&id) { tr(&p.name).to_string() } else { p.name.clone() };
    let sz = egui::vec2(24.0, 24.0);
    let mut out = None;
    // asking before a delete takes the row
    if st.confirm.as_deref() == Some(id.as_str()) {
        let text =
            if builtin { tr_format!("Take “{name}” off this computer?", name = name) } else { tr_format!("Delete “{name}”?", name = name) };
        match table::confirm_item(ui, t, &DEVELOP_COLS, &format!("preset:{id}"), &text) {
            Some(true) => {
                st.confirm = None;
                return Some(("preset.remove", json!({"id": id})));
            }
            Some(false) => st.confirm = None,
            None => {}
        }
        return None;
    }
    let (rect, resp, rid) = table::row(ui, table::ROW_H, ("preset", &id));
    table::paint_item(ui, t, rect, n, resp.contains_pointer());
    let rr = table::RowRects::new(rect, rid, &DEVELOP_COLS, table::ITEM_NAME_LEFT);
    let fav = rr.in_lead(ui, table::item_star(rect), |ui| {
        icon_button(ui, &format!("presetFav:{id}"), if p.favorite { Icon::StarFilled } else { Icon::Star }, sz, p.favorite, true, "Favorite")
    });
    if fav.clicked() {
        out = Some(("preset.favorite", json!({"id": id})));
    }
    let mut renamed = None;
    if let Some((rid, text)) = st.rename.as_mut().filter(|(rid, _)| rid == &id) {
        rr.in_name(ui, |ui| {
            let r = crate::text_field::TextField::singleline(&format!("field:presetRename:{rid}"), text)
                .width(rr.name.width().min(380.0))
                .select_on_focus(true)
                .show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                renamed = Some(Some(text.trim().to_string()));
            } else if r.cancelled() {
                renamed = Some(None);
            }
        });
    } else {
        table::text_in(ui, rr.name, &name, t.font(13.0), if hidden { t.text_disabled } else { t.text });
        register(ui.ctx(), format!("presetRow:{id}"), rr.name);
    }
    if let Some(n) = renamed {
        st.rename = None;
        if let Some(n) = n.filter(|n| !n.is_empty()) {
            out =
                Some(if builtin { ("preset.renameBuiltin", json!({"id": id, "name": n})) } else { ("preset.rename", json!({"id": id, "name": n})) });
        }
    }
    let mut info: Vec<String> = Vec::new();
    if p.settings.get("masks").is_some() && p.has_ai_masks() {
        info.push(tr("AI masks").to_string());
    }
    if builtin {
        info.push(tr("Built-in").to_string());
    }
    if hidden {
        info.push(tr("Hidden").to_string());
    }
    table::info_in(ui, t, rr.info, &info.join(" · "));
    let fi = app.session.preset_for_import(p);
    let tip = if fi { "Take out of For Import" } else { "Add to For Import (shown first when importing)" };
    if rr.in_col(ui, Col::ForImport, |ui| icon_button(ui, &format!("presetForImport:{id}"), Icon::ImportArrow, sz, fi, true, tip)).clicked() {
        out = Some(("preset.forImport", json!({"id": id, "on": !fi})));
    }
    if rr.in_col(ui, Col::Rename, |ui| icon_button(ui, &format!("presetRename:{id}"), Icon::Pencil, sz, false, true, "Rename")).clicked() {
        st.rename = Some((id.clone(), p.name.clone()));
    }
    let r = rr.in_col(ui, Col::Group, |ui| {
        crate::widgets::menu_button(ui, tr("Group"), |ui| {
            ui.set_min_width(200.0);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for g in groups {
                    if ui.selectable_label(*g == p.group, g.as_str()).clicked() {
                        out = Some(("preset.setGroup", json!({"id": id, "group": g})));
                        ui.close();
                    }
                }
            });
        })
    });
    register(ui.ctx(), format!("presetGroupMenu:{id}"), r.response.rect);
    let up = if builtin { None } else { neighbour(app, rows, i, true) };
    let down = if builtin { None } else { neighbour(app, rows, i, false) };
    if rr.in_col(ui, Col::Up, |ui| icon_button(ui, &format!("presetUp:{id}"), Icon::ChevronUp, sz, false, up.is_some(), "Move Up")).clicked()
        && let Some(o) = up
    {
        out = Some(("preset.reorder", json!({"id": id, "before": o})));
    }
    if rr
        .in_col(ui, Col::Down, |ui| icon_button(ui, &format!("presetDown:{id}"), Icon::ChevronDown, sz, false, down.is_some(), "Move Down"))
        .clicked()
        && let Some(o) = down
    {
        out = Some(("preset.reorder", json!({"id": id, "after": o})));
    }
    let (icon, tip) = if hidden { (Icon::EyeOff, "Show in the Presets panel") } else { (Icon::Eye, "Hide from the Presets panel") };
    if rr.in_col(ui, Col::Hide, |ui| icon_button(ui, &format!("presetHide:{id}"), icon, sz, hidden, true, tip)).clicked() {
        out = Some(("preset.hide", json!({"id": id, "hidden": !hidden})));
    }
    if rr.in_col(ui, Col::Delete, |ui| icon_button(ui, &format!("presetDelete:{id}"), Icon::Trash, sz, false, true, "Delete")).clicked() {
        st.confirm = Some(id.clone());
    }
    out
}

/// The columns of the local adjustment presets' table.
const EFFECT_COLS: table::Columns = table::Columns { info_w: 0.0, for_import: false };

/// Local adjustment (mask effect) presets: by their Effect menu groups.
fn effects(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, st: &mut State) {
    hint(ui, t, "The presets of the Masking panel's Effect menu, shared by all your libraries. Hide takes one out of the menu; it stays here.");
    let mut action: Option<(&'static str, Value)> = None;
    ui.horizontal_wrapped(|ui| {
        let r = ui.button(tr("Import Presets…"));
        register(ui.ctx(), "button:effectsImport", r.rect);
        if r.clicked() {
            action = Some(("file.importPresets", json!({})));
        }
        let r = ui.add_enabled(!app.session.mask_effects.is_empty(), egui::Button::new(tr("Export…")));
        register(ui.ctx(), "button:effectsExport", r.rect);
        if r.clicked() {
            action = Some(("file.exportMaskEffects", json!({})));
        }
    });
    search_row(ui, st, false);
    let q = st.query.trim().to_lowercase();
    let fx = &app.session.mask_effects;
    let order = lightcraft_engine::cmd::mask_effects::menu_order(fx);
    let group_names: Vec<String> = order.iter().map(|(g, _)| g.clone()).filter(|g| !g.is_empty()).collect();
    let mut rows: Vec<Row> = Vec::new();
    for (g, ix) in &order {
        let hits: Vec<usize> = ix
            .iter()
            .copied()
            .filter(|i| fx.get(*i).is_some_and(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || e.group.to_lowercase().contains(&q)))
            .collect();
        if hits.is_empty() && !q.is_empty() {
            continue;
        }
        let shown = if g.is_empty() { tr("No group").to_string() } else { g.clone() };
        let hidden = ix.iter().all(|i| fx.get(*i).is_some_and(|e| e.hidden));
        rows.push(Row::Group { name: g.clone(), shown, count: ix.len(), hidden });
        if st.group_confirm.as_deref() == Some(g.as_str()) {
            rows.push(Row::Confirm(g.clone()));
        }
        if table::group_open(ui.ctx(), "effectGroup", g, true) || !q.is_empty() {
            rows.extend(hits.into_iter().enumerate().map(|(n, i)| Row::Item(i, n)));
        }
    }
    // the rest of the window (the tab scrolls only for the controls above)
    let height = (ui.clip_rect().bottom() - ui.cursor().top() - 8.0).max(360.0);
    let hs = heights(ui.ctx(), &rows);
    table::virtual_list(ui, "effectsManagerList", &hs, height, |ui, i| {
        let Some(row) = rows.get(i) else { return };
        match row {
            Row::Group { name, shown, count, hidden } => {
                let head = GroupHead {
                    key: "effectGroup",
                    name,
                    shown: shown.clone(),
                    count: *count,
                    all_hidden: *hidden,
                    prev: None,
                    next: None,
                    default_open: true,
                };
                let names: Vec<String> = fx.iter().filter(|e| &e.group == name).map(|e| e.name.clone()).collect();
                match group_header(ui, t, &EFFECT_COLS, &head, &mut st.group_rename).1 {
                    Some(GroupAction::Hide(h)) => action = Some(("mask.hideEffect", json!({"names": names, "hidden": h}))),
                    Some(GroupAction::Rename(n)) if !name.is_empty() => action = Some(("mask.renameEffectGroup", json!({"group": name, "to": n}))),
                    Some(GroupAction::Delete) if !name.is_empty() => {
                        st.group_confirm = Some(name.clone());
                        st.group_move_to = None;
                    }
                    _ => {}
                }
            }
            Row::Confirm(g) => {
                let names: Vec<String> = fx.iter().filter(|e| &e.group == g).map(|e| e.name.clone()).collect();
                let others: Vec<String> = group_names.iter().filter(|o| *o != g).cloned().collect();
                if let Some(params) = confirm_group_delete(ui, t, g, g, (0, names.len()), &others, &mut st.group_move_to, &mut st.group_confirm) {
                    action = Some(match params.get("moveTo").and_then(Value::as_str) {
                        Some(to) => ("mask.setEffectGroup", json!({"names": names, "group": to})),
                        None => ("mask.deleteEffect", json!({"names": names})),
                    });
                }
            }
            Row::Item(ix, n) => {
                if let Some(a) = effect_row(ui, t, st, fx, (*ix, *n), &group_names) {
                    action = Some(a);
                }
            }
        }
    });
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
}

/// One local adjustment preset's row. `(ix, n)`: its index, and its place in its group.
fn effect_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    st: &mut State,
    fx: &[lightcraft_engine::cmd::mask_effects::MaskEffect],
    (ix, n): (usize, usize),
    group_names: &[String],
) -> Option<(&'static str, Value)> {
    let e = fx.get(ix)?;
    let sz = egui::vec2(24.0, 24.0);
    let key = e.name.clone();
    let mut out = None;
    if st.confirm.as_deref() == Some(key.as_str()) {
        match table::confirm_item(ui, t, &EFFECT_COLS, &format!("effect:{key}"), &tr_format!("Delete “{name}”?", name = key)) {
            Some(true) => {
                st.confirm = None;
                return Some(("mask.deleteEffect", json!({"name": key})));
            }
            Some(false) => st.confirm = None,
            None => {}
        }
        return None;
    }
    let (rect, resp, rid) = table::row(ui, table::ROW_H, ("effect", &key));
    table::paint_item(ui, t, rect, n, resp.contains_pointer());
    let rr = table::RowRects::new(rect, rid, &EFFECT_COLS, table::ITEM_NAME_LEFT);
    let fav = rr.in_lead(ui, table::item_star(rect), |ui| {
        icon_button(ui, &format!("effectFav:{key}"), if e.favorite { Icon::StarFilled } else { Icon::Star }, sz, e.favorite, true, "Favorite")
    });
    if fav.clicked() {
        out = Some(("mask.favoriteEffect", json!({"name": key})));
    }
    let mut renamed = None;
    if let Some((rid, text)) = st.rename.as_mut().filter(|(rid, _)| rid == &key) {
        rr.in_name(ui, |ui| {
            let r = crate::text_field::TextField::singleline(&format!("field:effectRename:{rid}"), text)
                .width(rr.name.width().min(380.0))
                .select_on_focus(true)
                .show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                renamed = Some(Some(text.trim().to_string()));
            } else if r.cancelled() {
                renamed = Some(None);
            }
        });
    } else {
        table::text_in(ui, rr.name, &key, t.font(13.0), if e.hidden { t.text_disabled } else { t.text });
        register(ui.ctx(), format!("effectRow:{key}"), rr.name);
    }
    if let Some(n) = renamed {
        st.rename = None;
        if let Some(n) = n.filter(|n| !n.is_empty() && *n != key) {
            out = Some(("mask.renameEffect", json!({"name": key, "to": n})));
        }
    }
    if rr.in_col(ui, Col::Rename, |ui| icon_button(ui, &format!("effectRename:{key}"), Icon::Pencil, sz, false, true, "Rename")).clicked() {
        st.rename = Some((key.clone(), key.clone()));
    }
    let r = rr.in_col(ui, Col::Group, |ui| {
        crate::widgets::menu_button(ui, tr("Group"), |ui| {
            ui.set_min_width(180.0);
            if ui.selectable_label(e.group.is_empty(), tr("No group")).clicked() {
                out = Some(("mask.setEffectGroup", json!({"name": key, "group": ""})));
                ui.close();
            }
            for g in group_names {
                if ui.selectable_label(*g == e.group, g.as_str()).clicked() {
                    out = Some(("mask.setEffectGroup", json!({"name": key, "group": g})));
                    ui.close();
                }
            }
        })
    });
    register(ui.ctx(), format!("effectGroupMenu:{key}"), r.response.rect);
    let (icon, tip) = if e.hidden { (Icon::EyeOff, "Show in the Effect menu") } else { (Icon::Eye, "Hide from the Effect menu") };
    if rr.in_col(ui, Col::Hide, |ui| icon_button(ui, &format!("effectHide:{key}"), icon, sz, e.hidden, true, tip)).clicked() {
        out = Some(("mask.hideEffect", json!({"name": key, "hidden": !e.hidden})));
    }
    if rr.in_col(ui, Col::Delete, |ui| icon_button(ui, &format!("effectDelete:{key}"), Icon::Trash, sz, false, true, "Delete")).clicked() {
        st.confirm = Some(key.clone());
    }
    out
}

/// The columns of the import presets' table.
const IMPORT_COLS: table::Columns = table::Columns { info_w: 0.0, for_import: false };

/// Import presets (named sets of import settings, saved in the Import tab).
fn import_presets(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, st: &mut State) {
    hint(ui, t, "The import presets saved in the Import tab, shared by all your libraries.");
    let names: Vec<String> = app.session.import_presets.iter().map(|p| p.name.clone()).collect();
    if names.is_empty() {
        hint(ui, t, "None yet. Save one from the Import tab.");
        return;
    }
    let mut action: Option<(&'static str, Value)> = None;
    let sz = egui::vec2(24.0, 24.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    for (n, name) in names.iter().enumerate() {
        if st.confirm.as_deref() == Some(name.as_str()) {
            match table::confirm_item(ui, t, &IMPORT_COLS, &format!("importPreset:{name}"), &tr_format!("Delete “{name}”?", name = name)) {
                Some(true) => {
                    st.confirm = None;
                    action = Some(("import.deletePreset", json!({"name": name})));
                }
                Some(false) => st.confirm = None,
                None => {}
            }
            continue;
        }
        let (rect, resp, rid) = table::row(ui, table::ROW_H, ("importPreset", name));
        table::paint_item(ui, t, rect, n, resp.contains_pointer());
        let rr = table::RowRects::new(rect, rid, &IMPORT_COLS, 24.0);
        let mut renamed = None;
        if let Some((rid, text)) = st.rename.as_mut().filter(|(rid, _)| rid == name) {
            rr.in_name(ui, |ui| {
                let r = crate::text_field::TextField::singleline(&format!("field:importPresetRename:{rid}"), text)
                    .width(rr.name.width().min(380.0))
                    .select_on_focus(true)
                    .show(ui);
                if !r.editing && r.ending.is_none() {
                    r.response.request_focus();
                }
                if r.committed() {
                    renamed = Some(Some(text.trim().to_string()));
                } else if r.cancelled() {
                    renamed = Some(None);
                }
            });
        } else {
            table::text_in(ui, rr.name, name, t.font(13.0), t.text);
            register(ui.ctx(), format!("importPresetRow:{name}"), rr.name);
        }
        if let Some(n) = renamed {
            st.rename = None;
            if let Some(n) = n.filter(|n| !n.is_empty() && n != name) {
                action = Some(("import.renamePreset", json!({"name": name, "to": n})));
            }
        }
        if rr.in_col(ui, Col::Rename, |ui| icon_button(ui, &format!("importPresetRename:{name}"), Icon::Pencil, sz, false, true, "Rename")).clicked()
        {
            st.rename = Some((name.clone(), name.clone()));
        }
        if rr.in_col(ui, Col::Delete, |ui| icon_button(ui, &format!("importPresetDelete:{name}"), Icon::Trash, sz, false, true, "Delete")).clicked() {
            st.confirm = Some(name.clone());
        }
    }
    if let Some((command, params)) = action
        && let Err(e) = app.run(command, params)
    {
        app.ui.status = e;
    }
}

/// Dropped files or folders while the Presets tab shows: presets.
pub fn takes_drops(app: &LightcraftApp) -> bool {
    matches!(&app.ui.dialog, Some(crate::state::Dialog::Settings { tab }) if tab == "presets")
}
