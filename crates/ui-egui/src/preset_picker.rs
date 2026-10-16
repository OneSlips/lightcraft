//! The preset picker: one list for choosing a develop preset wherever one is chosen — the Import
//! panel's preset stack and the import review, Quick Develop's Saved Preset (applied to every
//! selected photo), and the default presets in Settings. The Presets panel lists the same rows.
//!
//! - **Search** over every preset's name and group.
//! - **Favorites** and **For Import** (presets marked for the Import panel) come first, then the
//!   groups in the Presets Manager's order (the user's own before LightCraft's), each folding.
//!   Choosing for an import shows Favorites and For Import alone until "All Presets" is chosen
//!   (when both are empty, everything shows).
//! - Hidden presets and built-in presets removed from this computer never show.
//! - Resting on a preset shows it on a photo (the one in view, else the first of the folder) in the
//!   picker, and in the loupe when one is open, as the profile menu does.
//! - Long names left-aligned, cut short with … (the full name on hover); the star and the
//!   For Import mark toggle in place.
//!
//! Only the rows in view are built: 11 000 presets open and scroll in a frame. The rows are kept
//! until the list, the manager's choices, the search or a group's fold change.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};
use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::LightcraftApp;
use crate::i18n::tr;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::register;

/// The Favorites group's key (not a group name a preset can have).
pub const FAVORITES: &str = "\u{1}favorites";
/// The For Import group's key.
pub const FOR_IMPORT: &str = "\u{1}forImport";

/// Height of a row.
const ROW_H: f32 = 24.0;
/// Width of the list.
const LIST_W: f32 = 340.0;
/// Long edge of the preview.
const PREVIEW_EDGE: usize = 256;

/// Which presets a picker offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    /// Every preset.
    #[default]
    All,
    /// Favorites and For Import only (the Import panel's start).
    ForImport,
}

/// One row of a picker's list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A group's header: key (a group's name, [`FAVORITES`] or [`FOR_IMPORT`]), name as shown,
    /// presets in it, open.
    Header { key: String, shown: String, count: usize, open: bool },
    /// A preset: its index in the session's presets.
    Preset(usize),
}

/// Whether group `key` is open by default: Favorites and For Import are; groups fold until opened.
fn default_open(key: &str) -> bool {
    key == FAVORITES || key == FOR_IMPORT
}

/// A group as the pickers and the Presets panel list it.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// A group's name, [`FAVORITES`] or [`FOR_IMPORT`].
    pub key: String,
    /// The name as shown (a built-in group's in the UI language).
    pub shown: String,
    /// Only LightCraft's presets in it.
    pub builtin: bool,
    /// Indices into the session's presets, in list order.
    pub items: Vec<usize>,
}

impl Group {
    /// Favorites or For Import (not a group a preset belongs to).
    pub fn pseudo(&self) -> bool {
        self.key == FAVORITES || self.key == FOR_IMPORT
    }
}

/// The groups of shown presets (hidden ones and built-in ones taken off this computer left out):
/// with a `query`, every group with a preset whose name or group has it, in the manager's order;
/// else Favorites and For Import, then (in [`View::All`], or when both are empty) every group in
/// the manager's order (the user's own before LightCraft's).
pub fn groups(session: &lightcraft_engine::Session, query: &str, view: View) -> Vec<Group> {
    let q = query.trim().to_lowercase();
    let shown: Vec<(usize, &lightcraft_develop::Preset)> = session.presets.iter().enumerate().filter(|(_, p)| session.preset_shown(p)).collect();
    let mut out = Vec::new();
    let by_group = |pick: &dyn Fn(&lightcraft_develop::Preset) -> bool| {
        let mut m: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, p) in &shown {
            if pick(p) {
                m.entry(p.group.as_str()).or_default().push(*i);
            }
        }
        m
    };
    let in_order = |out: &mut Vec<Group>, mut m: BTreeMap<&str, Vec<usize>>| {
        for g in session.preset_groups() {
            if let Some(items) = m.remove(g.as_str()) {
                let builtin = items.iter().all(|i| session.presets.get(*i).is_some_and(|p| p.builtin));
                out.push(Group { shown: crate::i18n::builtin_label(&g, builtin).to_string(), key: g, builtin, items });
            }
        }
    };
    if !q.is_empty() {
        let m = by_group(&|p: &lightcraft_develop::Preset| {
            p.name.to_lowercase().contains(&q)
                || crate::i18n::builtin_label(&p.name, p.builtin).to_lowercase().contains(&q)
                || p.group.to_lowercase().contains(&q)
        });
        in_order(&mut out, m);
        return out;
    }
    let favorites: Vec<usize> = shown.iter().filter(|(_, p)| p.favorite).map(|(i, _)| *i).collect();
    let for_import: Vec<usize> = shown.iter().filter(|(_, p)| session.preset_for_import(p)).map(|(i, _)| *i).collect();
    let all = view == View::All || (favorites.is_empty() && for_import.is_empty());
    for (key, name, items) in [(FAVORITES, "Favorites", favorites), (FOR_IMPORT, "For Import", for_import)] {
        if !items.is_empty() {
            out.push(Group { key: key.to_string(), shown: tr(name).to_string(), builtin: false, items });
        }
    }
    if all {
        in_order(&mut out, by_group(&|_| true));
    }
    out
}

/// The rows of a picker: [`groups`], each header followed by its presets when open (Favorites and
/// For Import are open until folded, groups fold until opened; a search opens all it finds).
/// `folds`: groups the user opened or closed.
pub fn rows(session: &lightcraft_engine::Session, query: &str, view: View, folds: &BTreeMap<String, bool>) -> Vec<Row> {
    let searching = !query.trim().is_empty();
    let mut out = Vec::new();
    for g in groups(session, query, view) {
        let open = searching || folds.get(&g.key).copied().unwrap_or_else(|| default_open(&g.key));
        out.push(Row::Header { key: g.key, shown: g.shown, count: g.items.len(), open });
        if open {
            out.extend(g.items.into_iter().map(Row::Preset));
        }
    }
    out
}

/// What a picker remembers between frames.
#[derive(Clone, Default)]
struct State {
    query: String,
    /// The view chosen (`None`: the picker's default).
    view: Option<View>,
    folds: BTreeMap<String, bool>,
    /// Put the keyboard in the search field (just opened).
    focus: bool,
    /// The preset under the pointer last (the preview stays while the pointer is in the picker).
    hovered: Option<String>,
}

fn state_id(id: &str) -> egui::Id {
    egui::Id::new(("presetPicker", id))
}

/// A choice besides the presets ("None", "LightCraft Default"…), listed first.
pub struct Extra<'a> {
    pub key: &'a str,
    pub label: String,
    pub selected: bool,
}

/// What was picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Picked {
    Preset(String),
    Extra(String),
}

/// A picker's options.
pub struct Picker<'a> {
    /// Widget ids (`presetPicker:{id}`…) and remembered state.
    pub id: &'a str,
    /// The preset chosen now (highlighted).
    pub selected: Option<&'a str>,
    /// Choosing for an import: starts on Favorites and For Import.
    pub for_import: bool,
    pub extras: Vec<Extra<'a>>,
}

impl<'a> Picker<'a> {
    pub fn new(id: &'a str) -> Self {
        Picker { id, selected: None, for_import: false, extras: Vec::new() }
    }

    pub fn selected(mut self, id: Option<&'a str>) -> Self {
        self.selected = id;
        self
    }

    pub fn for_import(mut self, on: bool) -> Self {
        self.for_import = on;
        self
    }

    pub fn extra(mut self, key: &'a str, label: impl Into<String>, selected: bool) -> Self {
        self.extras.push(Extra { key, label: label.into(), selected });
        self
    }

    /// A button `width` wide showing `text`; clicking it opens the picker below it. Returns what
    /// was picked (the picker closes then).
    pub fn button(self, app: &mut LightcraftApp, ui: &mut Ui, text: &str, width: f32) -> Option<Picked> {
        let t = Tokens::get(ui.ctx());
        let button = ui.add_sized(vec2(width, ui.spacing().interact_size.y), egui::Button::new(text).truncate().right_text(" "));
        paint(
            ui.painter(),
            Rect::from_center_size(pos2(button.rect.right() - 10.0, button.rect.center().y), vec2(16.0, 16.0)),
            Icon::ChevronDown,
            t.icon,
        );
        register(ui.ctx(), format!("presetPicker:{}", self.id), button.rect);
        let key = state_id(self.id);
        if button.clicked() {
            ui.data_mut(|d| {
                let mut st: State = d.get_temp(key).unwrap_or_default();
                st.focus = true;
                st.query.clear();
                st.hovered = None;
                d.insert_temp(key, st);
            });
        }
        let mut picked = None;
        egui::Popup::from_toggle_button_response(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            picked = self.body(app, ui);
            if picked.is_some() {
                ui.close();
            }
        });
        picked
    }

    /// The picker itself (in a popup or a panel): search, view, list and preview.
    pub fn body(&self, app: &mut LightcraftApp, ui: &mut Ui) -> Option<Picked> {
        let t = Tokens::get(ui.ctx());
        let key = state_id(self.id);
        let mut st: State = ui.data(|d| d.get_temp(key)).unwrap_or_default();
        let mut picked = None;
        let mut hovered_now: Option<String> = None;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(LIST_W);
                let r = crate::text_field::TextField::singleline(&format!("field:presetPickerSearch:{}", self.id), &mut st.query)
                    .hint(tr("Search presets"))
                    .width(LIST_W)
                    .show(ui);
                if st.focus {
                    r.response.request_focus();
                    st.focus = false;
                }
                let mut view = st.view.unwrap_or(if self.for_import { View::ForImport } else { View::All });
                // nothing marked yet: everything shows, and how to mark them is said
                let marked = app.session.presets.iter().any(|p| app.session.preset_shown(p) && (p.favorite || app.session.preset_for_import(p)));
                if !marked {
                    view = View::All;
                }
                if self.for_import && st.query.trim().is_empty() {
                    let cur = Some(if view == View::ForImport { 0 } else { 1 });
                    let items = [("Favorites and For Import", "import"), ("All Presets", "all")];
                    if let Some(i) = crate::widgets::segmented(ui, &format!("presetPickerView:{}", self.id), &items, cur, 2) {
                        st.view = Some(if i == 0 { View::ForImport } else { View::All });
                    }
                    if !marked {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tr("Mark the presets you use on import with the star or the import arrow at the right of a row: they show first here."))
                                    .size(11.0)
                                    .color(t.text_dim),
                            )
                            .wrap(),
                        );
                    }
                }
                for e in &self.extras {
                    let r = ui.add_sized(vec2(LIST_W, ROW_H), egui::Button::selectable(e.selected, e.label.as_str()).right_text(""));
                    register(ui.ctx(), format!("presetPickerExtra:{}:{}", self.id, e.key), r.rect);
                    if r.clicked() {
                        picked = Some(Picked::Extra(e.key.to_string()));
                    }
                }
                let list = self.cached_rows(app, ui.ctx(), &st, view);
                if list.is_empty() {
                    ui.label(egui::RichText::new(tr(if st.query.trim().is_empty() { "No presets" } else { "No preset matches" })).color(t.text_dim));
                }
                egui::ScrollArea::vertical().id_salt(("presetPickerList", self.id)).max_height(380.0).auto_shrink([false, true]).show_rows(
                    ui,
                    ROW_H,
                    list.len(),
                    |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for row in list.get(range).unwrap_or_default() {
                            match row {
                                Row::Header { key, shown, count, open } => {
                                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                                    register(ui.ctx(), format!("presetPickerGroup:{}:{key}", self.id), r);
                                    ui.painter().rect_filled(r.shrink2(vec2(0.0, 1.0)), 3.0, if resp.hovered() { t.hover } else { t.inset });
                                    paint(
                                        ui.painter(),
                                        Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(14.0, 14.0)),
                                        if *open { Icon::ChevronDown } else { Icon::ChevronRight },
                                        t.text_label,
                                    );
                                    let icon = match key.as_str() {
                                        FAVORITES => Some(Icon::StarFilled),
                                        FOR_IMPORT => Some(Icon::ImportArrow),
                                        _ => None,
                                    };
                                    let mut x = r.left() + 24.0;
                                    if let Some(i) = icon {
                                        paint(ui.painter(), Rect::from_center_size(pos2(x + 7.0, r.center().y), vec2(14.0, 14.0)), i, t.text_label);
                                        x += 20.0;
                                    }
                                    let count_text = crate::i18n::group_digits(*count);
                                    let cg = ui.painter().layout_no_wrap(count_text, t.font(11.0), t.text_dim);
                                    let name_w = (r.right() - 10.0 - cg.size().x - 8.0 - x).max(20.0);
                                    let g = crate::widgets::one_line(ui.painter(), shown, t.semibold(12.5), t.text_label, name_w);
                                    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, t.text_label);
                                    ui.painter().galley(pos2(r.right() - 10.0 - cg.size().x, r.center().y - cg.size().y / 2.0), cg, t.text_dim);
                                    if resp.clicked() {
                                        st.folds.insert(key.clone(), !open);
                                    }
                                }
                                Row::Preset(ix) => {
                                    let Some(p) = app.session.presets.get(*ix).cloned() else { continue };
                                    if let Some(a) = preset_row(app, ui, &t, self, &p, &mut hovered_now) {
                                        picked = Some(a);
                                    }
                                }
                            }
                        }
                    },
                );
            });
            // resting on a preset: it on the photo
            if ui.ui_contains_pointer() && hovered_now.is_some() {
                st.hovered = hovered_now.clone();
            }
            // (the preview's place is kept, so the list doesn't jump when it shows)
            match st.hovered.clone() {
                Some(pid) => preview(app, ui, &t, &pid),
                None => {
                    ui.vertical(|ui| {
                        ui.set_width(PREVIEW_EDGE as f32);
                        let (r, _) = ui.allocate_exact_size(vec2(PREVIEW_EDGE as f32, PREVIEW_EDGE as f32 * 2.0 / 3.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, t.inset);
                        let g = ui.painter().layout(tr("Rest the pointer on a preset to see it on the photo").to_string(), t.font(11.5), t.text_dim, r.width() - 24.0);
                        ui.painter().galley(r.center() - g.size() / 2.0, g, t.text_dim);
                    });
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(key, st));
        picked
    }

    /// The rows for `st`, built again only when something they come from changed.
    fn cached_rows(&self, app: &LightcraftApp, ctx: &egui::Context, st: &State, view: View) -> Arc<Vec<Row>> {
        type Key = (u64, u64, String, View, Vec<(String, bool)>);
        let s = &app.session;
        let k: Key = (s.presets.generation(), s.preset_state_gen, st.query.clone(), view, st.folds.iter().map(|(a, b)| (a.clone(), *b)).collect());
        let id = egui::Id::new(("presetPickerRows", self.id));
        if let Some((have, list)) = ctx.data(|d| d.get_temp::<(Key, Arc<Vec<Row>>)>(id))
            && have == k
        {
            return list;
        }
        let list = Arc::new(rows(s, &st.query, view, &st.folds));
        ctx.data_mut(|d| d.insert_temp(id, (k, list.clone())));
        list
    }
}

/// One preset's row: its name (left-aligned, cut short), an AI badge, and the star and For Import
/// marks at the right. Returns the pick when the row was clicked.
fn preset_row(
    app: &mut LightcraftApp,
    ui: &mut Ui,
    t: &Tokens,
    picker: &Picker,
    p: &lightcraft_develop::Preset,
    hovered: &mut Option<String>,
) -> Option<Picked> {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let name = crate::i18n::builtin_label(&p.name, p.builtin).to_string();
    register(ui.ctx(), format!("presetPickerRow:{}:{}", picker.id, p.id), r);
    let selected = picker.selected == Some(p.id.as_str());
    if selected {
        ui.painter().rect_filled(r, 2.0, t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 2.0, t.hover);
    }
    // the marks at the right: star, For Import
    let for_import = app.session.preset_for_import(p);
    let star = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(20.0, 20.0));
    let imp = Rect::from_center_size(pos2(r.right() - 36.0, r.center().y), vec2(20.0, 20.0));
    let mut out = None;
    let star_r = ui.interact(star, ui.id().with(("presetPickerStar", &p.id)), Sense::click());
    let imp_r = ui.interact(imp, ui.id().with(("presetPickerImport", &p.id)), Sense::click());
    register(ui.ctx(), format!("presetPickerStar:{}:{}", picker.id, p.id), star);
    register(ui.ctx(), format!("presetPickerForImport:{}:{}", picker.id, p.id), imp);
    let dim = |on: bool, hover: bool| {
        if on {
            t.text_label
        } else if hover {
            t.text_dim
        } else {
            t.text_dim.gamma_multiply(0.45)
        }
    };
    paint(
        ui.painter(),
        star.shrink(3.0),
        if p.favorite { Icon::StarFilled } else { Icon::Star },
        if p.favorite { t.star } else { dim(false, star_r.hovered()) },
    );
    paint(ui.painter(), imp.shrink(3.0), Icon::ImportArrow, if for_import { t.accent } else { dim(false, imp_r.hovered()) });
    let star_r = star_r.on_hover_text(tr(if p.favorite { "Remove from Favorites" } else { "Add to Favorites" }));
    let imp_r = imp_r.on_hover_text(tr(if for_import { "Take out of For Import" } else { "Add to For Import (shown first when importing)" }));
    let mut right = imp.left() - 4.0;
    if p.has_ai_masks() {
        let g = ui.painter().layout_no_wrap(tr("AI").to_string(), t.font(10.5), t.text_dim);
        ui.painter().galley(pos2(right - g.size().x, r.center().y - g.size().y / 2.0), g.clone(), t.text_dim);
        right -= g.size().x + 6.0;
    }
    let left = r.left() + 10.0;
    let g = crate::widgets::one_line(ui.painter(), &name, t.font(13.0), t.text, (right - left - 4.0).max(20.0));
    let elided = g.elided;
    ui.painter().galley(pos2(left, r.center().y - g.size().y / 2.0), g, t.text);
    if star_r.clicked() {
        let _ = app.run("preset.favorite", json!({"id": p.id}));
    } else if imp_r.clicked() {
        let _ = app.run("preset.forImport", json!({"id": p.id, "on": !for_import}));
    } else if resp.clicked() {
        out = Some(Picked::Preset(p.id.clone()));
    }
    let resp = if elided { resp.on_hover_text(name) } else { resp };
    if resp.hovered() || star_r.hovered() || imp_r.hovered() {
        *hovered = Some(p.id.clone());
    }
    out
}

/// The photo a picker previews on: the one in view, else the first of the folder or library.
fn preview_photo(app: &mut LightcraftApp) -> Option<PhotoId> {
    app.session.active().or_else(|| app.session.visible_cloned().first().copied())
}

/// Preset `pid` on the preview photo, beside the list (and in the loupe when one is open).
fn preview(app: &mut LightcraftApp, ui: &mut Ui, t: &Tokens, pid: &str) {
    let Some(p) = app.session.presets.iter().find(|p| p.id == pid).cloned() else { return };
    ui.vertical(|ui| {
        ui.set_width(PREVIEW_EDGE as f32);
        let Some(id) = preview_photo(app) else {
            ui.label(egui::RichText::new(tr("No photo to show the preset on")).color(t.text_dim));
            return;
        };
        let Some(current) = app.session.develop_of(id).map(|d| (*d).clone()) else { return };
        let aspect = app.session.photo_aspect(id);
        let mut look = p.apply(&current, 1.0, aspect);
        if p.settings.get("masks").is_some() {
            look = app.session.preview_preset(id, look, false).0;
        }
        let a = (aspect as f32).clamp(0.2, 5.0);
        let size = if a >= 1.0 { vec2(PREVIEW_EDGE as f32, PREVIEW_EDGE as f32 / a) } else { vec2(PREVIEW_EDGE as f32 * a, PREVIEW_EDGE as f32) };
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        register(ui.ctx(), "presetPickerPreview", r);
        ui.painter().rect_filled(r, 2.0, t.canvas);
        match app.session.variant_job(id, &look, PREVIEW_EDGE).and_then(|job| app.renderer.variant(job)) {
            Some(tex) => {
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().text(r.center(), Align2::CENTER_CENTER, tr("Rendering…"), t.font(11.0), t.text_dim);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
            }
        }
        let name = crate::i18n::builtin_label(&p.name, p.builtin).to_string();
        ui.add(egui::Label::new(egui::RichText::new(&name).color(t.text)).truncate());
        let group = crate::i18n::builtin_label(&p.group, p.builtin).to_string();
        ui.add(egui::Label::new(egui::RichText::new(group).size(11.0).color(t.text_dim)).truncate());
        app.hover_preview = Some(crate::HoverPreview { label: crate::i18n::tr_format!("Preset: {name}", name = name), settings: look });
    });
}

/// Type `query` into picker `id`'s search (tests).
#[cfg(test)]
pub(crate) fn set_query(ctx: &egui::Context, id: &str, query: &str) {
    let key = state_id(id);
    ctx.data_mut(|d| {
        let mut st: State = d.get_temp(key).unwrap_or_default();
        st.query = query.to_string();
        d.insert_temp(key, st);
    });
}

/// The name of preset `id` as a picker's button shows it.
pub fn preset_label(app: &LightcraftApp, id: &str) -> String {
    app.session
        .presets
        .iter()
        .find(|p| p.id == id)
        .map(|p| crate::i18n::builtin_label(&p.name, p.builtin).to_string())
        .unwrap_or_else(|| crate::i18n::tr_format!("{pid} (missing)", pid = id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::Preset;

    fn session() -> lightcraft_engine::Session {
        let mut s = lightcraft_engine::Session::with_demo();
        for (i, g) in ["002-Film", "001-Studio", "001-Studio", "Zeta"].iter().enumerate() {
            s.presets.push(Preset {
                id: format!("user.p{i}"),
                name: format!("Look {i}"),
                group: g.to_string(),
                settings: json!({"light": {"exposure": 0.1}}),
                favorite: false,
                builtin: false,
            });
        }
        s
    }

    fn headers(rows: &[Row]) -> Vec<String> {
        rows.iter().filter_map(|r| if let Row::Header { key, .. } = r { Some(key.clone()) } else { None }).collect()
    }

    /// LR-PRESET-PICKER: the user's groups first in the manager's order, LightCraft's after; groups
    /// fold until opened; hidden and removed presets never show; a search opens what it finds.
    #[test]
    fn rows_follow_the_managers_order_and_choices() {
        let mut s = session();
        let folds = BTreeMap::new();
        let list = rows(&s, "", View::All, &folds);
        let h = headers(&list);
        assert_eq!(&h[..3], ["001-Studio", "002-Film", "Zeta"], "{h:?}");
        assert!(list.iter().all(|r| matches!(r, Row::Header { open: false, .. })), "folded");
        // hidden and removed: gone
        s.execute("preset.hide", &json!({"id": "user.p3"})).unwrap();
        let builtin = s.presets.iter().find(|p| p.builtin).unwrap().clone();
        s.execute("preset.remove", &json!({"id": builtin.id})).unwrap();
        let mut folds = BTreeMap::new();
        for k in headers(&rows(&s, "", View::All, &BTreeMap::new())) {
            folds.insert(k, true);
        }
        let list = rows(&s, "", View::All, &folds);
        let ids: Vec<&str> =
            list.iter().filter_map(|r| if let Row::Preset(i) = r { s.presets.get(*i).map(|p| p.id.as_str()) } else { None }).collect();
        assert!(!ids.contains(&"user.p3") && !ids.contains(&builtin.id.as_str()), "{ids:?}");
        assert!(!headers(&list).contains(&"Zeta".to_string()), "a group with nothing shown goes");
        // a search: open, across groups
        let list = rows(&s, "look 1", View::All, &BTreeMap::new());
        assert_eq!(list.len(), 2, "{list:?}");
        assert!(matches!(&list[0], Row::Header { key, open: true, .. } if key == "001-Studio"));
    }

    /// Favorites and For Import come first; for an import they are all there is until All Presets.
    #[test]
    fn favorites_and_for_import_first() {
        let mut s = session();
        // nothing marked yet: an import's picker shows everything
        assert!(headers(&rows(&s, "", View::ForImport, &BTreeMap::new())).contains(&"001-Studio".to_string()));
        s.execute("preset.favorite", &json!({"id": "user.p0"})).unwrap();
        s.execute("preset.forImport", &json!({"id": "user.p2", "on": true})).unwrap();
        let list = rows(&s, "", View::ForImport, &BTreeMap::new());
        assert_eq!(headers(&list), [FAVORITES, FOR_IMPORT]);
        assert_eq!(list.len(), 4, "both open: {list:?}");
        let all = rows(&s, "", View::All, &BTreeMap::new());
        assert_eq!(&headers(&all)[..3], [FAVORITES, FOR_IMPORT, "001-Studio"]);
    }

    /// 11 000 presets in 200 groups: the rows are built well within a frame's budget.
    #[test]
    fn eleven_thousand_presets_build_quickly() {
        let mut s = lightcraft_engine::Session::with_demo();
        for i in 0..11_000 {
            s.presets.push(Preset {
                id: format!("user.b{i}"),
                name: format!("Look {i}"),
                group: format!("Collection_{}", i % 200),
                settings: json!({"light": {"exposure": 0.1}}),
                favorite: i % 997 == 0,
                builtin: false,
            });
        }
        let t0 = std::time::Instant::now();
        let list = rows(&s, "", View::All, &BTreeMap::new());
        let search = rows(&s, "look 12", View::All, &BTreeMap::new());
        assert!(list.len() > 200 && search.len() > 10);
        // (debug build, shared machine: generous; the release build is ~10x faster)
        assert!(t0.elapsed() < std::time::Duration::from_millis(1500), "{:?}", t0.elapsed());
    }
}
