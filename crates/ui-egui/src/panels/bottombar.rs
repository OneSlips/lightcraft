//! The bottom bar, one flat row of icons with thin dividers between the groups (no islands):
//! view modes and sort on the left; in the middle where the view stands ("1,234 / 2,233"), the
//! culling marks of the photo (stars, flags, colour labels: click to set) and the culling filters
//! with live counts (flags, stars, colour labels, and More filters for the rest); on the right zoom
//! and the view toggles (eye focus, face boxes, Before/After, filmstrip) or the thumbnail size.
//!
//! The middle gives way from the least needed part when the window is narrow: the copy buttons
//! first, the colour-label filters and chips next; the flags and the Picks count go last.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::{ColorLabel, Flag, RatingOp, Tally};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::state::{ViewMode, Zoom};
use crate::theme::{Tokens, label_color};
use crate::widgets::{icon_button, register, stars};

const VIEW_MODES: &[(&str, Icon, ViewMode, &str)] = &[
    ("photoGrid", Icon::GridPhoto, ViewMode::PhotoGrid, "Photo Grid (G)"),
    ("squareGrid", Icon::GridSquare, ViewMode::SquareGrid, "Square Grid (G toggles)"),
    ("detail", Icon::Single, ViewMode::Detail, "Detail (D)"),
    ("compare", Icon::Compare, ViewMode::Compare, "Compare (Shift+C)"),
    ("survey", Icon::Survey, ViewMode::Survey, "Survey (N)"),
    ("people", Icon::Subject, ViewMode::People, "People"),
];

/// Space either side of a divider, and between the parts of one group.
const DIVIDER_GAP: f32 = 10.0;
const PART_GAP: f32 = 8.0;
/// Setter sizes: a star, a flag, a colour chip.
const STAR: f32 = 18.0;
const FLAG: f32 = 25.0;
const CHIP: f32 = 16.0;

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("bottom_bar")
        .exact_size(t.bottom_bar_h)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let compact_zoom = full.width() < 360.0;
            let right_start = right_side(app, ui, full, compact_zoom);
            let left_rect = Rect::from_min_max(full.min, pos2((right_start - 12.0).max(full.left()), full.bottom()));
            let mut left_ui = ui.new_child(egui::UiBuilder::new().max_rect(left_rect));
            let left_end = left_ui
                .horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if left_rect.width() >= 270.0 {
                        for &(id, icon, mode, tip) in VIEW_MODES {
                            if icon_button(ui, id, icon, vec2(32.0, 32.0), app.ui.view == mode, true, tip).clicked() {
                                let _ = app.run(&format!("view.{id}"), json!({}));
                            }
                        }
                        ui.add_space(DIVIDER_GAP);
                        let (sep, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                        ui.painter().rect_filled(sep, 0.0, t.button_border);
                        ui.add_space(DIVIDER_GAP);
                        let sort = icon_button(ui, "sort", Icon::Sort, vec2(30.0, 30.0), false, true, "Sort");
                        egui::Popup::menu(&sort).show(|ui| sort_menu(app, ui));
                    } else {
                        let current_icon = VIEW_MODES.iter().find(|(_, _, mode, _)| *mode == app.ui.view).map_or(Icon::More, |(_, icon, _, _)| *icon);
                        let more = icon_button(ui, "viewOptions", current_icon, vec2(40.0, 32.0), true, true, "View");
                        paint(
                            ui.painter(),
                            Rect::from_center_size(more.rect.right_bottom() - vec2(5.0, 5.0), vec2(7.0, 7.0)),
                            Icon::ChevronDown,
                            t.text,
                        );
                        egui::Popup::menu(&more).show(|ui| {
                            for &(id, _, mode, label) in VIEW_MODES {
                                if ui.selectable_label(app.ui.view == mode, crate::i18n::tr(label)).clicked() {
                                    let _ = app.run(&format!("view.{id}"), json!({}));
                                    ui.close();
                                }
                            }
                            ui.separator();
                            crate::widgets::menu_button(ui, crate::i18n::tr("Sort"), |ui| sort_menu(app, ui));
                            if compact_zoom && matches!(app.ui.view, ViewMode::Detail | ViewMode::Compare) {
                                crate::widgets::menu_button(ui, crate::i18n::tr_format!("Click {}:1", app.ui.click_zoom / 100), |ui| {
                                    click_zoom_menu(app, ui)
                                });
                            }
                        });
                    }
                })
                .response
                .rect
                .right();
            centre(app, ui, full, left_end + DIVIDER_GAP, right_start - DIVIDER_GAP);
        });
}

/// The parts of the middle, left to right, in their groups (a divider between groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// "1,234 / 2,233": the active photo's place in the view.
    Position,
    Stars,
    Flags,
    Labels,
    FilterFlags,
    FilterStars,
    FilterLabels,
    MoreFilters,
    Copy,
}

const GROUPS: [&[Part]; 4] = [
    &[Part::Position],
    &[Part::Stars, Part::Flags, Part::Labels],
    &[Part::FilterFlags, Part::FilterStars, Part::FilterLabels, Part::MoreFilters],
    &[Part::Copy],
];

/// How the middle gives way when the bar is narrow, step by step until it fits: first the parts
/// drop their counts and captions, then the least needed parts go. The flags and their counts (the
/// Picks count is the culling target the photographer watches) go last.
const GIVE_WAY: [(Part, bool); 12] = [
    (Part::FilterLabels, true),
    (Part::Copy, true),
    (Part::FilterFlags, true),
    (Part::Copy, false),
    (Part::FilterLabels, false),
    (Part::Labels, false),
    (Part::FilterStars, false),
    (Part::Position, false),
    (Part::MoreFilters, false),
    (Part::Stars, false),
    (Part::Flags, false),
    (Part::FilterFlags, false),
];

/// What the middle shows this frame, worked out once.
struct Facts {
    tally: Tally,
    filter: lightcraft_catalog::Filter,
    rating: u8,
    flag: Flag,
    label: Option<ColorLabel>,
    has_photo: bool,
    position: String,
}

fn facts(app: &mut LightcraftApp) -> Facts {
    let tally = app.session.tally();
    let active = app.session.active().and_then(|id| app.session.catalog.photo(id).cloned());
    let (_, ids) = app.session.visible_shared();
    let at = active.as_ref().and_then(|p| ids.iter().position(|x| *x == p.id));
    let n_sel = app.session.selection.ids.len();
    let mut position = match at {
        Some(i) => format!("{} / {}", crate::i18n::group_digits(i + 1), crate::i18n::group_digits(ids.len())),
        None => crate::i18n::group_digits(ids.len()),
    };
    if n_sel > 1 {
        position.push_str(&crate::i18n::tr_format!(" · {n} selected", n = crate::i18n::group_digits(n_sel)));
    }
    Facts {
        tally,
        filter: app.session.filter.clone(),
        rating: active.as_ref().map_or(0, |p| p.rating),
        flag: active.as_ref().map(|p| p.flag).unwrap_or_default(),
        label: active.as_ref().and_then(|p| p.label),
        has_photo: active.is_some(),
        position,
    }
}

/// The flags the filter shows (empty: any).
fn flags_on(f: &lightcraft_catalog::Filter) -> Vec<Flag> {
    let mut v = f.flags.clone();
    if let Some(fl) = f.flag
        && !v.contains(&fl)
    {
        v.push(fl);
    }
    v
}

/// The colour labels the filter shows (empty: any).
fn labels_on(f: &lightcraft_catalog::Filter) -> Vec<ColorLabel> {
    let mut v = f.labels.clone();
    if let Some(l) = f.label
        && !v.contains(&l)
    {
        v.push(l);
    }
    v
}

const FLAG_FILTERS: [(Flag, &str, &str); 3] =
    [(Flag::Pick, "pick", "Show picked photos"), (Flag::Reject, "reject", "Show rejected photos"), (Flag::None, "none", "Show unflagged photos")];

fn flag_count(t: &Tally, f: Flag) -> usize {
    match f {
        Flag::Pick => t.picks,
        Flag::Reject => t.rejects,
        Flag::None => t.unflagged,
    }
}

fn flag_icon(f: Flag) -> Icon {
    match f {
        Flag::Pick => Icon::FlagPick,
        Flag::Reject => Icon::FlagReject,
        Flag::None => Icon::FlagNone,
    }
}

/// The colour a flag is drawn in when it is on.
fn flag_colour(t: &Tokens, f: Flag) -> Color32 {
    match f {
        Flag::Pick => t.pick,
        Flag::Reject => t.reject,
        Flag::None => t.text,
    }
}

/// What the rating filter is set to, in words ("All", "≥ 3", "= 5", "Unrated").
fn rating_words(f: &lightcraft_catalog::Filter) -> String {
    if f.rating == 0 {
        return crate::i18n::tr(if f.unrated { "Unrated" } else { "Any" }).to_string();
    }
    let op = match f.rating_op {
        RatingOp::AtLeast => "≥",
        RatingOp::Exactly => "=",
        RatingOp::AtMost => "≤",
    };
    if f.unrated { format!("{op} {} +0", f.rating) } else { format!("{op} {}", f.rating) }
}

/// How many photos the rating filter's current choice shows.
fn rating_count(t: &Tally, f: &lightcraft_catalog::Filter) -> usize {
    (0..=5u8).filter(|r| f.rating_matches(*r)).map(|r| t.stars.get(usize::from(r)).copied().unwrap_or(0)).sum()
}

/// A part as the bar lays it out: its width, and whether it is drawn compact (no counts, no
/// caption, short words).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    part: Part,
    w: f32,
    compact: bool,
}

/// The width of `part` (points), full or `compact`.
fn width(ui: &egui::Ui, t: &Tokens, x: &Facts, part: Part, compact: bool) -> f32 {
    let text_w = |s: &str, size: f32| ui.painter().layout_no_wrap(s.to_string(), t.font(size), t.text).size().x;
    let count_w = |n: usize| text_w(&crate::i18n::group_digits(n), 11.5);
    match part {
        Part::Position => text_w(&x.position, 12.0) + 4.0,
        Part::Stars => 5.0 * STAR + 4.0,
        Part::Flags => 3.0 * FLAG + 4.0,
        Part::Labels => 6.0 * CHIP + 5.0 * 2.0,
        Part::FilterFlags => {
            let caption = if compact { 0.0 } else { text_w(crate::i18n::tr("Filter"), 12.0) + 6.0 };
            caption + FLAG_FILTERS.iter().map(|(f, _, _)| 22.0 + 3.0 + count_w(flag_count(&x.tally, *f)) + 8.0).sum::<f32>()
        }
        Part::FilterStars => 16.0 + 4.0 + text_w(&rating_words(&x.filter), 12.0) + 5.0 + count_w(rating_count(&x.tally, &x.filter)) + 18.0,
        Part::FilterLabels if compact => 6.0 * 18.0,
        Part::FilterLabels => {
            ColorLabel::ALL.iter().map(|l| 14.0 + 3.0 + count_w(x.tally.label(*l)) + 6.0).sum::<f32>() + 14.0 + 3.0 + count_w(x.tally.unlabeled)
        }
        Part::MoreFilters => 28.0,
        // room for the longer of its two captions, so the bar doesn't shift when the clipboard fills
        Part::Copy => {
            let (a, b) = if compact { ("Copy", "Paste") } else { ("Copy Edit Settings", "Paste Edit Settings") };
            text_w(crate::i18n::tr(a), 13.0).max(text_w(crate::i18n::tr(b), 13.0)) + 20.0 + 4.0 + 28.0
        }
    }
}

/// The parts in their groups, as wide as they are drawn.
fn layout(ui: &egui::Ui, t: &Tokens, x: &Facts, compact: &[Part], hidden: &[Part]) -> Vec<Vec<Placed>> {
    GROUPS
        .iter()
        .map(|g| {
            g.iter()
                .filter(|p| !hidden.contains(p))
                .map(|p| {
                    let c = compact.contains(p);
                    Placed { part: *p, w: width(ui, t, x, *p, c), compact: c }
                })
                .collect::<Vec<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect()
}

/// The width of a layout: its parts, the gaps between them and the dividers between groups.
fn used(groups: &[Vec<Placed>]) -> f32 {
    let parts: f32 = groups.iter().map(|g| g.iter().map(|p| p.w).sum::<f32>() + PART_GAP * (g.len() as f32 - 1.0).max(0.0)).sum();
    parts + (groups.len().max(1) as f32 - 1.0) * (2.0 * DIVIDER_GAP + 1.0)
}

/// What fits in `room`: the parts after as many [`GIVE_WAY`] steps as it takes.
fn fitting(ui: &egui::Ui, t: &Tokens, x: &Facts, room: f32) -> Vec<Vec<Placed>> {
    let (mut compact, mut hidden) = (Vec::new(), Vec::new());
    let mut groups = layout(ui, t, x, &compact, &hidden);
    for (part, keep) in GIVE_WAY {
        if used(&groups) <= room {
            break;
        }
        if keep {
            compact.push(part);
        } else {
            hidden.push(part);
        }
        groups = layout(ui, t, x, &compact, &hidden);
    }
    if used(&groups) > room { Vec::new() } else { groups }
}

/// The middle of the bar, between `from` and `to` (the side groups).
fn centre(app: &mut LightcraftApp, ui: &mut egui::Ui, full: Rect, from: f32, to: f32) {
    let t = Tokens::get(ui.ctx());
    if !(to - from).is_finite() || to - from < 40.0 {
        return;
    }
    let x = facts(app);
    let groups = fitting(ui, &t, &x, to - from);
    let w = used(&groups);
    // centred in the bar when there is room, else kept between the side groups
    let mut cx = (full.center().x - w / 2.0).clamp(from, (to - w).max(from));
    let y = full.center().y;
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            cx += DIVIDER_GAP;
            ui.painter().rect_filled(Rect::from_center_size(pos2(cx + 0.5, y), vec2(1.0, 22.0)), 0.0, t.button_border);
            cx += 1.0 + DIVIDER_GAP;
        }
        for (k, p) in g.iter().enumerate() {
            if k > 0 {
                cx += PART_GAP;
            }
            let r = Rect::from_min_size(pos2(cx, y - 15.0), vec2(p.w, 30.0));
            draw_part(app, ui, &t, &x, p.part, p.compact, r);
            cx += p.w;
        }
    }
}

fn row_ui(ui: &mut egui::Ui, r: Rect, spacing: f32) -> egui::Ui {
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    c.spacing_mut().item_spacing.x = spacing;
    c
}

fn draw_part(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, x: &Facts, part: Part, compact: bool, r: Rect) {
    match part {
        Part::Position => {
            let pr = ui.painter().text(r.left_center(), Align2::LEFT_CENTER, &x.position, t.font(12.0), t.text_label);
            register(ui.ctx(), "bottom:position", pr);
            let resp = ui.interact(pr, egui::Id::new("bottom-position"), Sense::hover());
            crate::widgets::announce(&resp, x.position.clone());
            resp.on_hover_text(crate::i18n::tr("Where the photo is in the view, and how many it shows"));
        }
        Part::Stars => {
            let mut c = row_ui(ui, r, 1.0);
            if let Some(n) = stars(&mut c, "bottom", x.rating, STAR) {
                let mut params = json!({"rating": n});
                crate::panels::compare::target_active(app, &mut params);
                let _ = app.run("photo.rate", params);
            }
        }
        Part::Flags => {
            let mut c = row_ui(ui, r, 2.0);
            for (id, f, word) in [("pick", Flag::Pick, "Pick (P)"), ("reject", Flag::Reject, "Reject (X)"), ("unflag", Flag::None, "Unflagged (U)")] {
                let (fr, resp) = c.allocate_exact_size(vec2(FLAG, FLAG), Sense::click());
                crate::widgets::announce_selected(&resp, crate::i18n::tr(word), x.flag == f && x.has_photo);
                register(c.ctx(), format!("flag:{id}"), fr);
                let on = x.flag == f && x.has_photo;
                if resp.hovered() {
                    c.painter().rect_filled(fr, 4.0, t.hover.gamma_multiply(0.7));
                }
                let colour = if on {
                    flag_colour(t, f)
                } else if resp.hovered() {
                    t.text
                } else {
                    t.icon
                };
                paint(c.painter(), fr.shrink(3.0), flag_icon(f), colour);
                if resp.on_hover_text(crate::i18n::tr(word)).clicked() {
                    let mut params = json!({"flag": match f { Flag::Pick => "pick", Flag::Reject => "reject", Flag::None => "none" }});
                    crate::panels::compare::target_active(app, &mut params);
                    let _ = app.run("photo.flag", params);
                }
            }
        }
        Part::Labels => {
            let mut c = row_ui(ui, r, 2.0);
            for l in ColorLabel::ALL.iter().copied().map(Some).chain([None]) {
                let (cr, resp) = c.allocate_exact_size(vec2(CHIP, CHIP), Sense::click());
                let name = match l {
                    Some(l) => crate::i18n::tr_format!("{} label", crate::i18n::color_label(&app.session.catalog, l)),
                    None => crate::i18n::tr("No Label").to_string(),
                };
                let on = x.has_photo && x.label == l;
                crate::widgets::announce_selected(&resp, name.clone(), on);
                register(c.ctx(), format!("label:{}", l.map_or("none".to_string(), |l| format!("{l:?}").to_lowercase())), cr);
                label_chip(c.painter(), t, cr, l, on, resp.hovered());
                let tip = match l {
                    Some(_) if on => crate::i18n::tr_format!("{name} (click again to clear)", name = name),
                    _ => name,
                };
                if resp.on_hover_text(tip).clicked() {
                    let label = l.map_or("none".to_string(), |l| format!("{l:?}").to_lowercase());
                    let mut params = json!({"label": label, "toggle": true});
                    crate::panels::compare::target_active(app, &mut params);
                    let _ = app.run("photo.label", params);
                }
            }
        }
        Part::FilterFlags => {
            let mut c = row_ui(ui, r, 0.0);
            if !compact {
                c.label(egui::RichText::new(crate::i18n::tr("Filter")).font(t.font(12.0)).color(t.text_dim));
                c.add_space(6.0);
            }
            let on_now = flags_on(&x.filter);
            for (f, id, tip) in FLAG_FILTERS {
                let n = flag_count(&x.tally, f);
                let on = on_now.contains(&f);
                let count = crate::i18n::group_digits(n);
                let g = c.painter().layout_no_wrap(count, t.font(11.5), if on { t.text } else { t.text_dim });
                let (br, resp) = c.allocate_exact_size(vec2(22.0 + 3.0 + g.size().x + 8.0, 28.0), Sense::click());
                register(c.ctx(), format!("filter:{id}"), br);
                crate::widgets::announce_selected(&resp, format!("{} {n}", crate::i18n::tr(tip)), on);
                if on {
                    c.painter().rect_filled(br, 4.0, t.tool_active);
                } else if resp.hovered() {
                    c.painter().rect_filled(br, 4.0, t.hover.gamma_multiply(0.7));
                }
                let ir = Rect::from_min_size(pos2(br.left() + 3.0, br.center().y - 10.0), vec2(20.0, 20.0));
                paint(c.painter(), ir, flag_icon(f), if on { flag_colour(t, f) } else { t.icon });
                c.painter().galley(pos2(ir.right() + 3.0, br.center().y - g.size().y / 2.0), g, Color32::PLACEHOLDER);
                if resp.on_hover_text(crate::i18n::tr(tip)).clicked() {
                    let mut next: Vec<Flag> = on_now.clone();
                    if on {
                        next.retain(|v| *v != f);
                    } else {
                        next.push(f);
                    }
                    // all three is no filter
                    if next.len() == 3 {
                        next.clear();
                    }
                    let names: Vec<&str> = [Flag::Pick, Flag::Reject, Flag::None]
                        .into_iter()
                        .filter(|v| next.contains(v))
                        .map(|v| match v {
                            Flag::Pick => "pick",
                            Flag::Reject => "reject",
                            Flag::None => "none",
                        })
                        .collect();
                    let _ = app.run("library.filter", json!({"flag": Value::Null, "flags": names}));
                }
            }
        }
        Part::FilterStars => {
            let mut c = row_ui(ui, r, 0.0);
            let on = x.filter.rating > 0 || x.filter.unrated;
            let n = rating_count(&x.tally, &x.filter);
            let words = rating_words(&x.filter);
            let (br, resp) = c.allocate_exact_size(r.size() - vec2(0.0, 2.0), Sense::click());
            register(c.ctx(), "filter:stars", br);
            crate::widgets::announce_selected(&resp, format!("{}: {words} ({n})", crate::i18n::tr("Rating filter")), on);
            if on {
                c.painter().rect_filled(br, 4.0, t.tool_active);
            } else if resp.hovered() {
                c.painter().rect_filled(br, 4.0, t.hover.gamma_multiply(0.7));
            }
            let sr = Rect::from_min_size(pos2(br.left() + 3.0, br.center().y - 8.0), vec2(16.0, 16.0));
            paint(c.painter(), sr, if on { Icon::StarFilled } else { Icon::Star }, if on { t.star } else { t.icon });
            let tr = c.painter().text(
                pos2(sr.right() + 4.0, br.center().y),
                Align2::LEFT_CENTER,
                &words,
                t.font(12.0),
                if on { t.text } else { t.text_label },
            );
            c.painter().text(pos2(tr.right() + 5.0, br.center().y), Align2::LEFT_CENTER, crate::i18n::group_digits(n), t.font(11.5), t.text_dim);
            paint(c.painter(), Rect::from_center_size(pos2(br.right() - 8.0, br.center().y), vec2(12.0, 12.0)), Icon::ChevronDown, t.icon);
            let resp = resp.on_hover_text(crate::i18n::tr("Show photos by rating"));
            egui::Popup::menu(&resp).show(|ui| rating_filter_menu(app, ui, &x.tally, &x.filter));
        }
        Part::FilterLabels => {
            let mut c = row_ui(ui, r, 0.0);
            let on_now = labels_on(&x.filter);
            for l in ColorLabel::ALL.iter().copied().map(Some).chain([None]) {
                let (n, on) = match l {
                    Some(l) => (x.tally.label(l), on_now.contains(&l)),
                    None => (x.tally.unlabeled, x.filter.unlabeled),
                };
                let shown = if compact { String::new() } else { crate::i18n::group_digits(n) };
                let g = c.painter().layout_no_wrap(shown, t.font(11.5), if on { t.text } else { t.text_dim });
                let w = if compact { 18.0 } else { 14.0 + 3.0 + g.size().x + 6.0 };
                let (br, resp) = c.allocate_exact_size(vec2(w, 26.0), Sense::click());
                let id = l.map_or("none".to_string(), |l| format!("{l:?}").to_lowercase());
                register(c.ctx(), format!("filter:label-{id}"), br);
                let name = match l {
                    Some(l) => crate::i18n::tr_format!("Show photos labelled {}", crate::i18n::color_label(&app.session.catalog, l)),
                    None => crate::i18n::tr("Show photos without a label").to_string(),
                };
                crate::widgets::announce_selected(&resp, format!("{name} ({n})"), on);
                if on {
                    c.painter().rect_filled(br, 4.0, t.tool_active);
                } else if resp.hovered() {
                    c.painter().rect_filled(br, 4.0, t.hover.gamma_multiply(0.7));
                }
                let dot = Rect::from_center_size(pos2(br.left() + 2.0 + 7.0, br.center().y), vec2(15.0, 15.0));
                label_chip(c.painter(), t, dot, l, false, false);
                c.painter().galley(pos2(dot.right() + 3.0, br.center().y - g.size().y / 2.0), g, Color32::PLACEHOLDER);
                if resp.on_hover_text(format!("{name} ({})", crate::i18n::group_digits(n))).clicked() {
                    let mut labels = on_now.clone();
                    let mut unlabeled = x.filter.unlabeled;
                    match l {
                        Some(l) if on => labels.retain(|v| *v != l),
                        Some(l) => labels.push(l),
                        None => unlabeled = !unlabeled,
                    }
                    labels.sort();
                    let names: Vec<String> = labels.iter().map(|v| format!("{v:?}").to_lowercase()).collect();
                    let _ = app.run("library.filter", json!({"label": Value::Null, "labels": names, "unlabeled": unlabeled}));
                }
            }
        }
        Part::MoreFilters => {
            let active = more_filters_active(&x.filter);
            let mut c = row_ui(ui, r, 0.0);
            let resp = icon_button(&mut c, "moreFilters", Icon::Dots, vec2(28.0, 28.0), active, true, "More filters");
            if active {
                c.painter().circle_filled(resp.rect.right_top() + vec2(-5.0, 5.0), 3.5, t.accent);
            }
            let mut popup = egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            // View ▸ More Filters… (Shift+F) opens it
            if std::mem::take(&mut app.ui.filter_bar) {
                popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
            }
            popup.show(|ui| {
                ui.set_min_width(260.0);
                super::filterbar::more_filters(app, ui);
            });
        }
        Part::Copy => copy_buttons(app, ui, t, r, x.has_photo, compact),
    }
}

/// Whether a filter the More filters popover sets is on (type, edited, camera, lens, keyword).
fn more_filters_active(f: &lightcraft_catalog::Filter) -> bool {
    f.kind.is_some() || f.merged.is_some() || f.edited.is_some() || f.camera.is_some() || f.lens.is_some() || f.keyword.is_some()
}

/// A colour-label chip (`None`: no label, an empty ring), filled bigger when it is the photo's.
fn label_chip(p: &egui::Painter, t: &Tokens, r: Rect, l: Option<ColorLabel>, on: bool, hovered: bool) {
    let rad = r.width().min(r.height()) / 2.0 - if on { 0.5 } else { 2.5 };
    match l {
        Some(l) => {
            p.circle_filled(r.center(), rad, label_color(l));
            if on {
                p.circle_stroke(r.center(), rad + 1.5, Stroke::new(1.5, t.text));
            } else if hovered {
                p.circle_stroke(r.center(), rad + 1.0, Stroke::new(1.0, t.text_label));
            }
        }
        None => {
            p.circle_stroke(r.center(), rad - 0.5, Stroke::new(1.4, if on || hovered { t.text } else { t.icon }));
            p.line_segment([r.center() + vec2(-rad * 0.6, rad * 0.6), r.center() + vec2(rad * 0.6, -rad * 0.6)], Stroke::new(1.2, t.icon));
        }
    }
}

fn rating_filter_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, tally: &Tally, f: &lightcraft_catalog::Filter) {
    let mut item = |ui: &mut egui::Ui, label: String, n: usize, on: bool, patch: Value, id: &str| {
        let r = ui.selectable_label(on, format!("{label}    {}", crate::i18n::group_digits(n)));
        register(ui.ctx(), format!("filter:stars:{id}"), r.rect);
        if r.clicked() {
            let _ = app.run("library.filter", patch);
            ui.close();
        }
    };
    let none = f.rating == 0 && !f.unrated;
    item(ui, crate::i18n::tr("Any rating").to_string(), tally.total, none, json!({"rating": 0, "unrated": false}), "any");
    item(ui, crate::i18n::tr("Unrated").to_string(), tally.stars[0], f.rating == 0 && f.unrated, json!({"rating": 0, "unrated": true}), "unrated");
    ui.separator();
    for n in 1..=4u8 {
        let on = f.rating == n && f.rating_op == RatingOp::AtLeast && !f.unrated;
        item(
            ui,
            crate::i18n::tr_format!("{n} stars or more", n = n),
            tally.at_least(n),
            on,
            json!({"rating": n, "ratingOp": "atLeast", "unrated": false}),
            &format!("atLeast{n}"),
        );
    }
    ui.separator();
    for n in 1..=5u8 {
        let on = f.rating == n && f.rating_op == RatingOp::Exactly && !f.unrated;
        let count = tally.stars.get(usize::from(n)).copied().unwrap_or(0);
        item(
            ui,
            crate::i18n::tr_format!("Exactly {n} stars", n = n),
            count,
            on,
            json!({"rating": n, "ratingOp": "exactly", "unrated": false}),
            &format!("exactly{n}"),
        );
    }
}

/// Copy / Paste Edit Settings and its options, flat like the rest of the bar.
fn copy_buttons(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, r: Rect, has_photo: bool, compact: bool) {
    let has_clip = app.session.clipboard.is_some();
    let full_label = crate::i18n::tr(if has_clip { "Paste Edit Settings" } else { "Copy Edit Settings" });
    let label = if compact { crate::i18n::tr(if has_clip { "Paste" } else { "Copy" }) } else { full_label };
    let copy_r = Rect::from_min_max(r.min, pos2(r.right() - 32.0, r.bottom()));
    let cresp = ui.interact(copy_r, egui::Id::new("copy-settings"), Sense::click());
    crate::widgets::announce(&cresp, label.to_string());
    register(ui.ctx(), "button:copySettings", copy_r);
    let cresp = if compact { cresp.on_hover_text(full_label) } else { cresp };
    if cresp.hovered() && has_photo {
        ui.painter().rect_filled(copy_r, 4.0, t.hover.gamma_multiply(0.7));
    }
    ui.painter().text(copy_r.center(), Align2::CENTER_CENTER, label, t.font(13.0), if has_photo { t.text_label } else { t.text_disabled });
    if cresp.clicked() && has_photo {
        let _ = if has_clip { app.run("develop.paste", json!({})) } else { app.run("develop.copy", json!({})) };
        let msg = if has_clip { "Settings pasted" } else { "Edit settings copied" };
        app.toast(ui.ctx(), crate::i18n::tr(msg));
    }
    let gear_r = Rect::from_min_size(pos2(r.right() - 28.0, r.top() + 1.0), vec2(28.0, 28.0));
    let gresp = ui.interact(gear_r, egui::Id::new("copy-gear"), Sense::click());
    crate::widgets::announce(&gresp, crate::i18n::tr("Copy Settings…"));
    register(ui.ctx(), "icon:copyGear", gear_r);
    if gresp.hovered() {
        ui.painter().rect_filled(gear_r, 4.0, t.hover.gamma_multiply(0.7));
    }
    paint(ui.painter(), gear_r.shrink(6.0), Icon::Gear, t.icon);
    if gresp.on_hover_text(crate::i18n::tr("Copy Settings…")).clicked() {
        let _ = app.run("dialog.copySettings", json!({}));
    }
    let _ = StrokeKind::Inside;
}

/// The right-hand group (zoom, view toggles or thumbnail size); returns its left edge.
fn right_side(app: &mut LightcraftApp, ui: &mut egui::Ui, full: Rect, compact_zoom: bool) -> f32 {
    let t = Tokens::get(ui.ctx());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2((full.right() - 400.0).max(full.left()), full.top()), full.right_bottom()))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 4.0;
    if matches!(app.ui.view, ViewMode::Compare | ViewMode::Survey)
        && icon_button(
            &mut child,
            "autoAdvance",
            Icon::ChevronRight,
            vec2(30.0, 30.0),
            app.auto_advance_on(),
            true,
            "Auto Advance after rating or flagging (or Caps Lock)",
        )
        .clicked()
    {
        let _ = app.run("view.autoAdvance", json!({}));
    }
    if app.ui.view == ViewMode::Survey {
        // no zoom or thumbnail size in Survey
    } else if matches!(app.ui.view, ViewMode::Detail | ViewMode::Compare) {
        if icon_button(&mut child, "filmstrip", Icon::Filmstrip, vec2(30.0, 30.0), app.ui.filmstrip, true, "Filmstrip (/)").clicked() {
            let _ = app.run("view.filmstrip", json!({}));
        }
        if icon_button(&mut child, "beforeAfter", Icon::BeforeAfter, vec2(30.0, 30.0), app.ui.before_after.is_layout(), true, "Before/After (Y)")
            .clicked()
        {
            let _ = app.run("view.beforeAfter", json!({}));
        }
        if app.ui.view == ViewMode::Detail
            && icon_button(&mut child, "faceBoxes", Icon::FaceBox, vec2(30.0, 30.0), app.ui.face_boxes, true, "Face boxes").clicked()
        {
            let _ = app.run("view.faceBoxes", json!({}));
        }
        if app.ui.view == ViewMode::Detail {
            let on = app.eye_focus_on();
            let tip = if app.importing() { "Eye focus strip while importing (Shift+T)" } else { "Eye focus strip in the library (Shift+T)" };
            if icon_button(&mut child, "eyeFocus", Icon::Eye, vec2(30.0, 30.0), on, true, tip).clicked() {
                let _ = app.run("view.eyeFocus", json!({}));
            }
        }
        child.add_space(DIVIDER_GAP);
        let (sep, _) = child.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
        child.painter().rect_filled(sep, 0.0, t.button_border);
        child.add_space(DIVIDER_GAP);
        let zoom_label = match app.ui.zoom {
            Zoom::Fit => "Fit".to_string(),
            Zoom::Fill => "Fill".to_string(),
            Zoom::Percent(p) => format!("{p:.0}%"),
        };
        let zr = crate::widgets::dropdown(&mut child, "zoom", crate::i18n::tr(&zoom_label), t.font(13.0), t.text_label);
        egui::Popup::menu(&zr).show(|ui| {
            for (label, z) in [
                ("Fit", Zoom::Fit),
                ("Fill", Zoom::Fill),
                ("50%", Zoom::Percent(50.0)),
                ("100%", Zoom::Percent(100.0)),
                ("200%", Zoom::Percent(200.0)),
                ("400%", Zoom::Percent(400.0)),
            ] {
                if ui.selectable_label(app.ui.zoom == z, crate::i18n::tr(label)).clicked() {
                    app.ui.zoom = z;
                    app.ui.pan = (0.5, 0.5);
                }
            }
        });
        if !compact_zoom {
            child.add_space(6.0);
            let cz = crate::widgets::dropdown(
                &mut child,
                "clickZoom",
                &crate::i18n::tr_format!("Click {}:1", app.ui.click_zoom / 100),
                t.font(13.0),
                t.text_label,
            );
            egui::Popup::menu(&cz).show(|ui| click_zoom_menu(app, ui));
        }
    } else {
        // thumbnail size slider
        let (r, resp) = child.allocate_exact_size(vec2(110.0, 20.0), Sense::click_and_drag());
        crate::widgets::announce(&resp, crate::i18n::tr("Thumbnail size"));
        register(child.ctx(), "slider:thumbSize", r);
        let p = child.painter();
        let y = r.center().y;
        p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], egui::Stroke::new(2.0, t.track));
        let (min, max) = (crate::state::THUMB_SIZE_MIN, crate::state::THUMB_SIZE_MAX);
        let f = (app.ui.thumb_size - min) / (max - min);
        let x = r.left() + f * r.width();
        p.rect_filled(Rect::from_center_size(pos2(x, y), vec2(3.0, 14.0)), 1.0, t.thumb_hover);
        if let Some(pp) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked())
        {
            app.ui.thumb_size = min + ((pp.x - r.left()) / r.width()).clamp(0.0, 1.0) * (max - min);
        }
    }
    if child.min_rect().width() > 0.0 { child.min_rect().left() } else { full.right() }
}

fn click_zoom_menu(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    for pct in crate::state::CLICK_ZOOMS {
        if ui.selectable_label(app.ui.click_zoom == pct, format!("{}:1", pct / 100)).clicked() {
            let _ = app.run("view.clickZoom", json!({"ratio": pct / 100}));
        }
    }
}

fn sort_menu(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    use lightcraft_catalog::GroupBy;
    use lightcraft_catalog::SortKey::*;
    let cur = app.session.sort;
    ui.label(egui::RichText::new(crate::i18n::tr("Sort by")).weak());
    for (label, key, k) in [
        ("Capture Date", CaptureDate, "captureDate"),
        ("Import Date", ImportDate, "importDate"),
        ("Modified Date", EditDate, "editDate"),
        ("File Name", FileName, "fileName"),
        ("Rating", Rating, "rating"),
        ("File Size", FileSize, "fileSize"),
        ("Random", Random, "random"),
    ] {
        if ui.selectable_label(cur.key == key, crate::i18n::tr(label)).clicked() {
            let _ = app.run("library.sort", json!({"key": k}));
        }
    }
    if cur.key == Random && ui.button(crate::i18n::tr("Reshuffle")).clicked() {
        let _ = app.run("library.shuffle", json!({}));
    }
    // a view that holds its photos shows what matches now when asked to
    if app.session.hold_view {
        let r = ui.button(crate::i18n::tr("Refresh View"));
        crate::widgets::register(ui.ctx(), "button:refreshView", r.rect);
        if r.clicked() {
            let _ = app.run("library.refresh", json!({}));
        }
    }
    ui.separator();
    // a shuffle has no direction worth choosing
    ui.add_enabled_ui(cur.key != Random, |ui| {
        if ui.selectable_label(cur.key != Random && cur.ascending, crate::i18n::tr("Ascending")).clicked() {
            let _ = app.run("library.sort", json!({"ascending": true}));
        }
        if ui.selectable_label(cur.key != Random && !cur.ascending, crate::i18n::tr("Descending")).clicked() {
            let _ = app.run("library.sort", json!({"ascending": false}));
        }
    });
    ui.separator();
    ui.label(egui::RichText::new(crate::i18n::tr("Group by date")).weak());
    for (label, g, k) in [
        ("Automatic", GroupBy::Auto, "auto"),
        ("Day", GroupBy::Day, "day"),
        ("Month", GroupBy::Month, "month"),
        ("Year", GroupBy::Year, "year"),
        ("None", GroupBy::None, "none"),
    ] {
        if ui.selectable_label(cur.group == g, crate::i18n::tr(label)).clicked() {
            let _ = app.run("library.sort", json!({"group": k}));
        }
    }
}

#[cfg(test)]
mod visual_regressions {
    use super::*;

    fn scaled_input(size: egui::Vec2, scale: f32) -> egui::RawInput {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        input
    }

    #[test]
    fn capture_bottom_bar_visual_fixtures() {
        let Some(directory) = std::env::var_os("CRAFT_UI_VISUAL_FIXTURES").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&directory).unwrap();
        for width in [280.0, 400.0, 600.0, 1000.0] {
            for view in [
                ViewMode::PhotoGrid,
                ViewMode::SquareGrid,
                ViewMode::Detail,
                ViewMode::Compare,
                ViewMode::Survey,
                ViewMode::Reference,
                ViewMode::People,
            ] {
                for scale in [1.0, 1.5, 2.0] {
                    let mut app = LightcraftApp::new(lightcraft_engine::Session::new(), Default::default());
                    app.ui.view = view;
                    let mut ready = false;
                    let mut harness =
                        egui_kittest::Harness::builder().with_size(vec2(width, 100.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                            move |ui, app: &mut LightcraftApp| {
                                if !ready {
                                    crate::theme::install_fonts(ui.ctx());
                                    crate::theme::apply(ui.ctx());
                                    ready = true;
                                    return;
                                }
                                crate::widgets::take_registry(ui.ctx());
                                show(app, ui);
                            },
                            app,
                        );
                    harness.input_mut().max_texture_side = Some(8192);
                    harness.run_steps(4);
                    let stem = format!("lightcraft-bottom-{view:?}-{width}-{scale}x");
                    harness.render().unwrap().save(directory.join(format!("{stem}.png"))).unwrap();
                    let panel = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("bottom_bar")).unwrap().outer_rect;
                    let controls: serde_json::Map<String, serde_json::Value> = crate::widgets::take_registry(&harness.ctx)
                        .into_iter()
                        .map(|(id, r)| (id, json!([r.left(), r.top(), r.width(), r.height()])))
                        .collect();
                    std::fs::write(
                        directory.join(format!("{stem}.json")),
                        serde_json::to_vec_pretty(&json!({
                            "screen": [width, 100.0], "scale": scale,
                            "regions": {"bottom-bar": [panel.left(), panel.top(), panel.width(), panel.height()]},
                            "controls": controls
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }

    /// The registered controls that can sit side by side (a part's own sub-ids, like a stars menu
    /// row, are not on the bar).
    fn on_bar(id: &str) -> bool {
        !id.starts_with("filter:stars:")
    }

    #[test]
    fn bottom_bar_controls_do_not_overlap_in_narrow_canvas() {
        for width in [280.0, 400.0, 600.0, 1000.0, 1300.0, 1900.0] {
            for view in [
                ViewMode::PhotoGrid,
                ViewMode::SquareGrid,
                ViewMode::Detail,
                ViewMode::Compare,
                ViewMode::Survey,
                ViewMode::Reference,
                ViewMode::People,
            ] {
                for scale in [1.0, 1.5, 2.0] {
                    let ctx = egui::Context::default();
                    crate::theme::install_fonts(&ctx);
                    crate::theme::apply(&ctx);
                    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Default::default());
                    app.ui.view = view;
                    let mut output = ctx.run_ui(scaled_input(vec2(width, 100.0), scale), |ui| show(&mut app, ui));
                    output.textures_delta.clear();
                    assert_eq!(ctx.content_rect().size(), vec2(width, 100.0));
                    assert_eq!(ctx.pixels_per_point(), scale);
                    let controls: Vec<(String, Rect)> = crate::widgets::take_registry(&ctx).into_iter().filter(|(id, _)| on_bar(id)).collect();
                    assert!(!controls.is_empty());
                    for (index, (id, rect)) in controls.iter().enumerate() {
                        assert!(rect.left() >= 0.0 && rect.right() <= width, "{view:?} at {width}: {id} outside viewport: {rect:?}");
                        for (other_id, other) in controls.iter().skip(index + 1) {
                            // a part's frame holds its own controls
                            if other_id.starts_with(id.as_str()) || id.starts_with(other_id.as_str()) {
                                continue;
                            }
                            assert!(!rect.intersect(*other).is_positive(), "{view:?} at {width}: {id} overlaps {other_id}: {rect:?} / {other:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn compact_view_menu_opens_and_changes_view_by_keyboard() {
        use egui_kittest::kittest::Queryable;

        for scale in [1.0, 1.5, 2.0] {
            let app = LightcraftApp::new(lightcraft_engine::Session::new(), Default::default());
            let mut ready = false;
            let mut harness = egui_kittest::Harness::builder().with_size(vec2(280.0, 640.0)).with_pixels_per_point(scale).build_ui_state(
                move |ui, app: &mut LightcraftApp| {
                    if !ready {
                        crate::theme::install_fonts(ui.ctx());
                        crate::theme::apply(ui.ctx());
                        ready = true;
                        return;
                    }
                    crate::shortcuts::handle(app, ui.ctx());
                    crate::widgets::take_registry(ui.ctx());
                    show(app, ui);
                },
                app,
            );
            harness.run_steps(4);
            assert_eq!(harness.ctx.content_rect().size(), vec2(280.0, 640.0));
            assert_eq!(harness.ctx.pixels_per_point(), scale);
            let mut focused = false;
            for _ in 0..12 {
                harness.key_press(egui::Key::Tab);
                harness.run_steps(3);
                focused = harness.get_by_label("View").is_focused();
                if focused {
                    break;
                }
            }
            assert!(focused, "Tab reaches the compact View control at {scale}x");
            harness.key_press(egui::Key::Space);
            harness.run_steps(3);
            let mut people_focused = false;
            for _ in 0..20 {
                harness.key_press(egui::Key::Tab);
                harness.run_steps(3);
                people_focused = harness.get_by_label("People").is_focused();
                if people_focused {
                    break;
                }
            }
            assert!(people_focused, "Tab reaches People at {scale}x");
            harness.key_press(egui::Key::Space);
            harness.run_steps(3);
            assert_eq!(harness.state().ui.view, ViewMode::People, "keyboard activation dispatches the view command");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use crate::headless::HeadlessView;

    fn bar(app: &mut LightcraftApp, width: f32) -> Vec<(String, Rect)> {
        let mut view = HeadlessView::new();
        let mut out = Vec::new();
        for frame in 0..3 {
            let raw = HeadlessView::raw_input(vec2(width, 60.0), 1.0, frame as f64 / 60.0, vec![]);
            view.run(raw, |ui| show(app, ui));
            out = crate::widgets::take_registry(&view.ctx);
        }
        out
    }

    /// The Picks count is the culling target the photographer watches: it stays on the bar down to a
    /// narrow window, after the copy buttons and the label filters have given way.
    #[test]
    fn the_picks_count_stays_while_less_needed_parts_give_way() {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        app.ui.view = ViewMode::Detail;
        let wide = bar(&mut app, 2200.0);
        for id in [
            "filter:pick",
            "filter:reject",
            "filter:none",
            "filter:stars",
            "filter:label-red",
            "icon:moreFilters",
            "button:copySettings",
            "flag:pick",
            "label:red",
            "bottom:position",
        ] {
            assert!(wide.iter().any(|(k, _)| k == id), "{id} on a wide bar: {:?}", wide.iter().map(|(k, _)| k).collect::<Vec<_>>());
        }
        let narrow = bar(&mut app, 900.0);
        assert!(narrow.iter().any(|(k, _)| k == "filter:pick"), "the Picks filter and count stay");
        assert!(!narrow.iter().any(|(k, _)| k == "button:copySettings"), "the copy button gives way first");
    }

    #[test]
    fn flag_filters_combine_and_count_live() {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        let ids: Vec<u64> = app.session.visible_cloned().iter().map(|p| p.0).take(3).collect();
        app.run("photo.flag", json!({"flag": "none", "ids": ids})).unwrap();
        let base = app.session.tally();
        app.run("photo.flag", json!({"flag": "pick", "ids": [ids[0], ids[1]]})).unwrap();
        app.run("photo.flag", json!({"flag": "reject", "ids": [ids[2]]})).unwrap();
        let t = app.session.tally();
        assert_eq!((t.picks, t.rejects), (base.picks + 2, base.rejects + 1));
        // picks and unflagged: the rejects are hidden, the counts stay what the view holds
        app.run("library.filter", json!({"flags": ["pick", "none"]})).unwrap();
        let shown = app.session.visible_cloned();
        assert!(!shown.iter().any(|p| p.0 == ids[2]));
        assert_eq!(app.session.tally().rejects, base.rejects + 1, "the count is of the view, not of what the filter shows");
        app.run("photo.flag", json!({"flag": "pick", "ids": [ids[2]]})).unwrap();
        assert_eq!(app.session.tally().picks, base.picks + 3, "a pick shows in the count at once");
    }
}
