//! The table the managers (Settings ▸ Profiles, Settings ▸ Presets) draw their lists as: one row
//! per group or item, every row the same height. A group's header is a shaded band; its open
//! items hang off an indent guide, alternate faintly and light up under the pointer. The actions
//! sit in fixed columns at the right edge, the same for groups and items, so they form columns
//! down the list whatever the names' lengths; the count and the details are a muted column just
//! left of them. Confirmations open in the list at the row they are about (never at the top).
//!
//! Long lists draw only the rows in view ([`rows_in_view`]), so 11 000 presets scroll smoothly.

use egui::{Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use serde_json::Value;

use crate::i18n::{tr, tr_format};
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// Height of a row (headers too).
pub const ROW_H: f32 = 30.0;
/// Height of a confirmation's row (its text and buttons).
pub const CONFIRM_H: f32 = 84.0;
/// Width of an icon action's column.
const ICON_W: f32 = 28.0;
/// Width of the Group menu's column.
const GROUP_W: f32 = 74.0;
/// Where an item's star sits (inside the indent guide).
const INDENT: f32 = 26.0;

/// An action column, left to right as they appear at the row's right edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    /// Use on import (presets only).
    ForImport,
    Rename,
    /// The Group menu (items only).
    Group,
    Up,
    Down,
    Hide,
    Delete,
}

/// The columns of one table.
#[derive(Clone, Copy, Debug)]
pub struct Columns {
    /// The muted details column (count for groups, kind / badges for items).
    pub info_w: f32,
    /// Has the Use on Import column.
    pub for_import: bool,
}

impl Columns {
    fn cols(&self) -> Vec<(Col, f32)> {
        let mut v = Vec::with_capacity(7);
        if self.for_import {
            v.push((Col::ForImport, ICON_W));
        }
        v.extend([(Col::Rename, ICON_W), (Col::Group, GROUP_W), (Col::Up, ICON_W), (Col::Down, ICON_W), (Col::Hide, ICON_W), (Col::Delete, ICON_W)]);
        v
    }

    /// Width of all the action columns.
    pub fn actions_w(&self) -> f32 {
        self.cols().iter().map(|(_, w)| w).sum::<f32>() + 6.0
    }
}

/// A row's parts: where its name goes, its details, and each action column.
pub struct RowRects {
    pub rect: Rect,
    /// The row's own id: its columns' widgets keep their ids from frame to frame whatever rows
    /// come before (a click lands on the widget it was aimed at).
    pub id: egui::Id,
    /// The name (after the fold arrow or the star).
    pub name: Rect,
    pub info: Rect,
    cols: Vec<(Col, Rect)>,
}

impl RowRects {
    pub fn new(rect: Rect, id: egui::Id, c: &Columns, name_left: f32) -> RowRects {
        let mut x = rect.right() - 6.0;
        let mut cols: Vec<(Col, Rect)> = Vec::new();
        for (col, w) in c.cols().into_iter().rev() {
            cols.push((col, Rect::from_min_max(pos2(x - w, rect.top()), pos2(x, rect.bottom()))));
            x -= w;
        }
        cols.reverse();
        let info = Rect::from_min_max(pos2((x - c.info_w - 8.0).max(rect.left() + name_left), rect.top()), pos2(x - 8.0, rect.bottom()));
        let name =
            Rect::from_min_max(pos2(rect.left() + name_left, rect.top()), pos2((info.left() - 8.0).max(rect.left() + name_left), rect.bottom()));
        RowRects { rect, id, name, info, cols }
    }

    /// The rectangle of column `col` (an empty one at the right edge when the table has no such column).
    pub fn col(&self, col: Col) -> Rect {
        self.cols.iter().find(|(c, _)| *c == col).map_or(Rect::from_min_size(self.rect.right_top(), vec2(0.0, 0.0)), |(_, r)| *r)
    }

    /// Run `f` in a child UI filling column `col`, its contents centred.
    pub fn in_col<R>(&self, ui: &mut Ui, col: Col, f: impl FnOnce(&mut Ui) -> R) -> R {
        let r = self.col(col);
        let b = UiBuilder::new().id_salt((self.id, col as u8)).max_rect(r).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight));
        ui.scope_builder(b, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            f(ui)
        })
        .inner
    }

    /// Run `f` in a child UI over the name's place, laid out left to right.
    pub fn in_name<R>(&self, ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
        let b = UiBuilder::new().id_salt((self.id, "name")).max_rect(self.name).layout(egui::Layout::left_to_right(egui::Align::Center));
        ui.scope_builder(b, f).inner
    }

    /// Run `f` in a child UI over `r` (an item's star, a group's fold arrow).
    pub fn in_lead<R>(&self, ui: &mut Ui, r: Rect, f: impl FnOnce(&mut Ui) -> R) -> R {
        ui.scope_builder(UiBuilder::new().id_salt((self.id, "lead")).max_rect(r), f).inner
    }
}

/// Allocate a full-width row `h` high (at the cursor) that senses clicks, with an id of its own
/// (`key`: what the row shows, unique in the list).
pub fn row(ui: &mut Ui, h: f32, key: impl std::hash::Hash + std::fmt::Debug) -> (Rect, egui::Response, egui::Id) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), h));
    let id = egui::Id::new(("managerRow", key));
    (rect, ui.interact(rect, id, Sense::click()), id)
}

/// Paint a group header's band.
pub fn paint_group(ui: &Ui, t: &Tokens, rect: Rect, hovered: bool) {
    let fill = if hovered { t.hover } else { t.inset };
    ui.painter().rect_filled(rect.shrink2(vec2(0.0, 1.0)), 3.0, fill);
}

/// Paint an item row: faint alternate shading (`n`: its place in the group), the hover highlight,
/// and the group's indent guide.
pub fn paint_item(ui: &Ui, t: &Tokens, rect: Rect, n: usize, hovered: bool) {
    if hovered {
        ui.painter().rect_filled(rect, 2.0, t.hover.gamma_multiply(0.8));
    } else if n % 2 == 1 {
        ui.painter().rect_filled(rect, 0.0, t.inset.gamma_multiply(0.45));
    }
    let x = rect.left() + 14.0;
    ui.painter().line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], egui::Stroke::new(1.0, t.divider));
    ui.painter().line_segment(
        [pos2(rect.left() + 20.0, rect.bottom()), pos2(rect.right(), rect.bottom())],
        egui::Stroke::new(0.5, t.divider.gamma_multiply(0.5)),
    );
}

/// Draw text left-aligned and cut short to fit `r`.
pub fn text_in(ui: &Ui, r: Rect, text: &str, font: egui::FontId, color: egui::Color32) {
    let g = crate::widgets::one_line(ui.painter(), text, font, color, r.width().max(8.0) - 2.0);
    ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, color);
}

/// Draw muted text right-aligned in `r` (the details column).
pub fn info_in(ui: &Ui, t: &Tokens, r: Rect, text: &str) {
    let g = crate::widgets::one_line(ui.painter(), text, t.font(11.0), t.text_dim, r.width().max(8.0) - 2.0);
    ui.painter().galley(pos2(r.right() - g.size().x, r.center().y - g.size().y / 2.0), g, t.text_dim);
}

/// A group's header row as the managers draw it.
pub struct GroupHead<'a> {
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
pub enum GroupAction {
    Hide(bool),
    Rename(String),
    Move { before: bool, other: String },
    Delete,
}

/// Whether group `name` (widgets keyed `key`) is open.
pub fn group_open(ctx: &egui::Context, key: &str, name: &str, default_open: bool) -> bool {
    ctx.data(|m| m.get_temp(egui::Id::new((key, "open", name)))).unwrap_or(default_open)
}

/// A group's header row: a shaded band with the fold arrow, its name and count; Rename, Move up /
/// down, Hide / Show and Delete in their columns. Clicking the band folds it. Returns whether it is
/// open, and what was asked.
pub fn group_header(ui: &mut Ui, t: &Tokens, c: &Columns, h: &GroupHead, renaming: &mut Option<(String, String)>) -> (bool, Option<GroupAction>) {
    let open_id = egui::Id::new((h.key, "open", h.name));
    let open: bool = ui.data(|m| m.get_temp(open_id)).unwrap_or(h.default_open);
    let mut out = None;
    let sz = vec2(24.0, 24.0);
    let (rect, resp, id) = row(ui, ROW_H, (h.key, "group", h.name));
    paint_group(ui, t, rect, resp.contains_pointer());
    let rr = RowRects::new(rect, id, c, 30.0);
    let fold = Rect::from_min_size(pos2(rect.left() + 3.0, rect.center().y - 12.0), sz);
    let fold_r = rr.in_lead(ui, fold, |ui| {
        icon_button(
            ui,
            &format!("{}Fold:{}", h.key, h.name),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            sz,
            false,
            true,
            if open { "Collapse" } else { "Expand" },
        )
    });
    let renaming_this = renaming.as_ref().is_some_and(|(rg, _)| rg == h.name);
    if fold_r.clicked() || (resp.clicked() && !renaming_this) {
        ui.data_mut(|m| m.insert_temp(open_id, !open));
    }
    let mut done = None;
    if let Some((rg, text)) = renaming.as_mut().filter(|(rg, _)| rg == h.name) {
        rr.in_name(ui, |ui| {
            let r = crate::text_field::TextField::singleline(&format!("field:{}Rename:{rg}", h.key), text)
                .width(rr.name.width().min(300.0))
                .select_on_focus(true)
                .show(ui);
            if !r.editing && r.ending.is_none() {
                r.response.request_focus();
            }
            if r.committed() {
                done = Some(Some(text.trim().to_string()).filter(|n| !n.is_empty() && n != h.name));
            } else if r.cancelled() {
                done = Some(None);
            }
        });
    } else {
        let color = if h.all_hidden { t.text_disabled } else { t.text_label };
        text_in(ui, rr.name, &h.shown, t.semibold(13.0), color);
        register(ui.ctx(), format!("{}Head:{}", h.key, h.name), rr.name);
        let count = crate::i18n::group_digits(h.count);
        let count = if h.all_hidden { format!("{} · {count}", tr("Hidden")) } else { count };
        info_in(ui, t, rr.info, &count);
    }
    if let Some(name) = done {
        *renaming = None;
        if let Some(n) = name {
            out = Some(GroupAction::Rename(n));
        }
    }
    if rr
        .in_col(ui, Col::Rename, |ui| icon_button(ui, &format!("{}Rename:{}", h.key, h.name), Icon::Pencil, sz, false, true, "Rename Group"))
        .clicked()
    {
        *renaming = Some((h.name.to_string(), h.name.to_string()));
    }
    if rr
        .in_col(ui, Col::Up, |ui| icon_button(ui, &format!("{}Up:{}", h.key, h.name), Icon::ChevronUp, sz, false, h.prev.is_some(), "Move Group Up"))
        .clicked()
        && let Some(o) = h.prev
    {
        out = Some(GroupAction::Move { before: true, other: o.to_string() });
    }
    if rr
        .in_col(ui, Col::Down, |ui| {
            icon_button(ui, &format!("{}Down:{}", h.key, h.name), Icon::ChevronDown, sz, false, h.next.is_some(), "Move Group Down")
        })
        .clicked()
        && let Some(o) = h.next
    {
        out = Some(GroupAction::Move { before: false, other: o.to_string() });
    }
    let (icon, tip) = if h.all_hidden { (Icon::EyeOff, "Show the group in the menus") } else { (Icon::Eye, "Hide the group from the menus") };
    if rr.in_col(ui, Col::Hide, |ui| icon_button(ui, &format!("{}Hide:{}", h.key, h.name), icon, sz, h.all_hidden, h.count > 0, tip)).clicked() {
        out = Some(GroupAction::Hide(!h.all_hidden));
    }
    if rr
        .in_col(ui, Col::Delete, |ui| icon_button(ui, &format!("{}Delete:{}", h.key, h.name), Icon::Trash, sz, false, true, "Delete Group"))
        .clicked()
    {
        out = Some(GroupAction::Delete);
    }
    (open, out)
}

/// "New Group…" and its name field; `create` gets the name typed.
pub fn new_group_row(ui: &mut Ui, t: &Tokens, key: &str, typing: &mut Option<String>, create: impl FnOnce(String)) {
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

/// Asking before a group is deleted, in a box at the group (under its header): what happens to its
/// built-in and own items (`counts`), and (with own items) a group to move them to instead. Returns
/// the `groupRemove` parameters once confirmed.
#[allow(clippy::too_many_arguments)]
pub fn confirm_group_delete(
    ui: &mut Ui,
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
    let framed = egui::Frame::NONE
        .fill(t.inset)
        .stroke(egui::Stroke::new(1.0, t.caution.gamma_multiply(0.7)))
        .corner_radius(4.0)
        .inner_margin(egui::Margin { left: 30, right: 8, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let mut text = tr_format!("Delete the group “{name}”?", name = shown);
            if builtin > 0 {
                text.push(' ');
                text.push_str(&tr_format!("{n} built-in items are taken off this computer (Restore brings them back).", n = builtin));
            }
            if own > 0 {
                text.push(' ');
                text.push_str(&tr_format!("{n} of your own: delete them, or move them to another group.", n = own));
            }
            let r = ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
            register(ui.ctx(), format!("label:groupDeleteAsk:{group}"), r.rect);
            ui.horizontal(|ui| {
                if own > 0 && !others.is_empty() {
                    let label = move_to.clone().unwrap_or_else(|| tr("Move to…").to_string());
                    let r = crate::widgets::combo_box(("groupMoveTo", group)).width(200.0).selected_text(label).height(300.0).show_ui(ui, |ui| {
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
                            out = Some(serde_json::json!({"group": group, "moveTo": to}));
                            *confirm = None;
                        }
                    }
                }
                let r = ui.button(if own > 0 { tr("Delete Everything") } else { tr("Delete") });
                register(ui.ctx(), format!("button:groupDeleteConfirm:{group}"), r.rect);
                if r.clicked() {
                    out = Some(serde_json::json!({"group": group}));
                    *confirm = None;
                }
                let r = ui.button(tr("Cancel"));
                register(ui.ctx(), format!("button:groupDeleteCancel:{group}"), r.rect);
                if r.clicked() {
                    *confirm = None;
                }
            });
        });
    // a virtual list makes the box's row as tall as it was drawn
    let h = framed.response.rect.height() + 4.0;
    ui.ctx().data_mut(|d| d.insert_temp(confirm_h_id(), h));
    out
}

fn confirm_h_id() -> egui::Id {
    egui::Id::new("managerTableConfirmH")
}

/// The height of a group's confirmation box (as drawn last, else a guess).
pub fn confirm_h(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(confirm_h_id())).filter(|h| h.is_finite() && *h > 20.0).unwrap_or(CONFIRM_H).min(400.0)
}

/// Asking before one item is deleted, in place of its row: the question at its name, Delete and
/// Cancel in the action columns' place. `Some(true)`: delete; `Some(false)`: cancelled.
pub fn confirm_item(ui: &mut Ui, t: &Tokens, c: &Columns, key: &str, question: &str) -> Option<bool> {
    let (rect, _, _) = row(ui, ROW_H, (key, "confirm"));
    ui.painter().rect_filled(rect, 3.0, t.inset);
    ui.painter().rect_stroke(rect.shrink(0.5), 3.0, egui::Stroke::new(1.0, t.caution.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let actions = Rect::from_min_max(pos2(rect.right() - c.actions_w().max(180.0), rect.top()), rect.right_bottom());
    let text = Rect::from_min_max(pos2(rect.left() + INDENT, rect.top()), pos2(actions.left() - 8.0, rect.bottom()));
    text_in(ui, text, question, t.font(13.0), t.text);
    let mut out = None;
    ui.scope_builder(
        UiBuilder::new()
            .id_salt((key, "confirmButtons"))
            .max_rect(actions.shrink2(vec2(4.0, 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let r = ui.button(tr("Cancel"));
            register(ui.ctx(), format!("button:{key}DeleteCancel"), r.rect);
            if r.clicked() {
                out = Some(false);
            }
            let r = ui.button(tr("Delete"));
            register(ui.ctx(), format!("button:{key}DeleteConfirm"), r.rect);
            if r.clicked() {
                out = Some(true);
            }
        },
    );
    out
}

/// The left edge of an item's star (the name follows it).
pub fn item_star(rect: Rect) -> Rect {
    Rect::from_min_size(pos2(rect.left() + INDENT - 4.0, rect.center().y - 12.0), vec2(24.0, 24.0))
}

/// Where an item's name starts.
pub const ITEM_NAME_LEFT: f32 = INDENT + 26.0;

/// The rows `first..end` of a list whose row `i` starts at `tops[i]` (with `tops[n]` its end)
/// that meet the band `y0..y1` of it.
pub fn rows_in_view(tops: &[f32], y0: f32, y1: f32) -> (usize, usize) {
    let n = tops.len().saturating_sub(1);
    let first = tops.partition_point(|t| *t <= y0).saturating_sub(1).min(n);
    let end = tops.partition_point(|t| *t < y1).min(n).max(first);
    (first, end)
}

/// A scrolling list of rows of different heights (`heights`), drawing only those in view: `draw`
/// gets each row's index and is laid out at its place. `max_height` caps the list's height.
pub fn virtual_list(ui: &mut Ui, id: &str, heights: &[f32], max_height: f32, mut draw: impl FnMut(&mut Ui, usize)) {
    let mut tops = Vec::with_capacity(heights.len() + 1);
    let mut y = 0.0;
    for h in heights {
        tops.push(y);
        y += h.max(1.0);
    }
    tops.push(y);
    let total = y;
    egui::ScrollArea::vertical().id_salt(id).max_height(max_height).auto_shrink([false, false]).show_viewport(ui, |ui, view| {
        ui.set_height(total);
        let (first, end) = rows_in_view(&tops, view.top(), view.bottom());
        let origin = ui.min_rect().top();
        for i in first..end {
            let (Some(top), Some(h)) = (tops.get(i), heights.get(i)) else { continue };
            let r = Rect::from_min_size(pos2(ui.min_rect().left(), origin + top), vec2(ui.available_width(), *h));
            ui.scope_builder(UiBuilder::new().id_salt(("virtualRow", i)).max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                draw(ui, i);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_in_view_finds_the_band() {
        let tops = [0.0, 30.0, 60.0, 136.0, 166.0];
        assert_eq!(rows_in_view(&tops, 0.0, 30.0), (0, 1));
        assert_eq!(rows_in_view(&tops, 35.0, 70.0), (1, 3));
        assert_eq!(rows_in_view(&tops, 140.0, 900.0), (3, 4));
        assert_eq!(rows_in_view(&tops, 900.0, 1000.0), (4, 4));
        assert_eq!(rows_in_view(&[0.0], 0.0, 100.0), (0, 0));
        assert_eq!(rows_in_view(&tops, -50.0, 10.0), (0, 1));
    }

    #[test]
    fn action_columns_sit_at_the_same_place_in_every_row() {
        let c = Columns { info_w: 120.0, for_import: true };
        let id = egui::Id::new("t");
        let a = RowRects::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, ROW_H)), id, &c, 30.0);
        let b = RowRects::new(Rect::from_min_size(pos2(0.0, 30.0), vec2(800.0, ROW_H)), id, &c, ITEM_NAME_LEFT);
        for col in [Col::ForImport, Col::Rename, Col::Group, Col::Up, Col::Down, Col::Hide, Col::Delete] {
            assert_eq!(a.col(col).x_range(), b.col(col).x_range(), "{col:?}");
        }
        assert!(a.col(Col::Delete).right() <= 800.0 && a.info.right() <= a.col(Col::ForImport).left());
        assert!(b.name.right() <= b.info.left());
    }
}
