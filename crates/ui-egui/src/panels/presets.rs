//! The Presets column (opens to the left of the Edit panel): grouped presets with hover preview
//! (the loupe shows the photo with the preset while the pointer rests on it; nothing is
//! committed), optional live thumbnails, amount slider, create/favourite, import/export of preset
//! files.

use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, pos2, vec2};
use lightcraft_develop::{ControlSpec, Preset, Section, Track};
use lightcraft_engine::presets::PresetList;
use serde_json::json;

use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{divider, icon_button, register, slider};
use crate::{HoverPreview, LightcraftApp};

/// Long edge of preset thumbnails (px).
const THUMB_EDGE: usize = 128;

/// The preset amount slider may replace only the undo step it created.
#[derive(Clone)]
struct AppliedPreset {
    id: String,
    amount: f64,
    selected: Vec<lightcraft_catalog::PhotoId>,
    active: Option<lightcraft_catalog::PhotoId>,
    commits: u64,
}

impl AppliedPreset {
    fn new(session: &lightcraft_engine::Session, id: String, amount: f64) -> Self {
        Self { id, amount, selected: session.selection.ids.clone(), active: session.active(), commits: session.commits() }
    }

    fn is_current(&self, session: &lightcraft_engine::Session) -> bool {
        self.selected == session.selection.ids
            && self.active == session.active()
            && self.commits == session.commits()
            && session.interaction.is_none()
            && session.redo.is_empty()
            && session
                .undo
                .last()
                .is_some_and(|entry| session.presets.iter().any(|preset| preset.id == self.id && entry.label == format!("Preset: {}", preset.name)))
    }
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.fill(t.chrome).stroke(egui::Stroke::new(1.0, t.divider)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
        ui.painter().text(pos2(hr.left() + 24.0, hr.center().y + 2.0), Align2::LEFT_CENTER, crate::i18n::tr("Presets"), t.semibold(15.0), t.text);
        let mut hdr = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(hr.right() - 80.0, hr.top()), hr.right_bottom()))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if icon_button(
            &mut hdr,
            "presetCreate",
            Icon::Plus,
            vec2(28.0, 28.0),
            false,
            app.session.active().is_some(),
            crate::i18n::tr("Create Preset…"),
        )
        .clicked()
        {
            app.ui.dialog = Some(crate::state::Dialog::create_preset());
        }
        let more = icon_button(&mut hdr, "presetMore", Icon::More, vec2(28.0, 28.0), false, true, crate::i18n::tr("More preset options"));
        egui::Popup::menu(&more).show(|ui| {
            let r = ui.checkbox(&mut app.ui.preset_thumbs, crate::i18n::tr("Show Thumbnails"));
            register(ui.ctx(), "presetMenu:thumbnails", r.rect);
            ui.separator();
            if ui.button(crate::i18n::tr("Import Presets…")).clicked() {
                let _ = app.run("file.importPresets", json!({}));
            }
            let r = ui.button(crate::i18n::tr("Manage Presets…"));
            register(ui.ctx(), "presetMenu:manage", r.rect);
            if r.clicked() {
                let _ = app.run("app.settings", json!({"tab": "presets"}));
            }
            let any_user = app.session.presets.iter().any(|p| !p.builtin);
            if ui.add_enabled(any_user, egui::Button::new(crate::i18n::tr("Export User Presets…"))).clicked() {
                let _ = app.run("file.exportPresets", json!({}));
            }
        });
        divider(ui);
        // amount slider (applies to the last applied preset)
        let amt_id = egui::Id::new("preset-amount");
        let last: Option<AppliedPreset> = ui.data(|d| d.get_temp(amt_id));
        if let Some(last) = last.clone().filter(|last| last.is_current(&app.session)) {
            let spec = ControlSpec {
                id: "presetAmount",
                label: "Amount",
                section: Section::Profile,
                min: 0.0,
                max: 200.0,
                default: 100.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let out = slider(ui, &spec, last.amount, true, None);
            if let Some(v) = out.value
                && app.run("edit.undo", json!({})).is_ok()
            {
                // Only replace the most recent preset; never undo intervening edits.
                if app.run("preset.apply", json!({"id": &last.id, "amount": v})).is_ok() {
                    ui.data_mut(|d| d.insert_temp(amt_id, AppliedPreset::new(&app.session, last.id, v)));
                } else {
                    let _ = app.run("edit.redo", json!({}));
                    ui.data_mut(|d| d.remove::<AppliedPreset>(amt_id));
                }
            }
            divider(ui);
        } else {
            ui.data_mut(|d| d.remove::<AppliedPreset>(amt_id));
        }
        let active = app.session.active();
        let current = active.and_then(|id| app.session.develop_of(id)).map(|d| (*d).clone());
        // the photo's proportions place a preset's gradients
        let aspect = active.map_or(lightcraft_develop::recipe::RECIPE_ASPECT, |id| app.session.photo_aspect(id));
        let now = ui.input(|i| i.time);
        let hover_since_id = egui::Id::new("preset-hover-since");
        let mut hovered_any = false;
        let thumbs = app.ui.preset_thumbs && active.is_some();
        let row_h = if thumbs { 54.0 } else { 26.0 };
        // search, as in every preset picker
        let query_id = egui::Id::new("presets-panel-query");
        let mut query: String = ui.data(|d| d.get_temp(query_id)).unwrap_or_default();
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 6 }).show(ui, |ui| {
            crate::text_field::TextField::singleline("field:presetsPanelSearch", &mut query)
                .hint(crate::i18n::tr("Search presets"))
                .width(ui.available_width())
                .show(ui);
        });
        ui.data_mut(|d| d.insert_temp(query_id, query.clone()));
        let groups = grouped(ui.ctx(), &app.session, &query);
        let scroll = egui::ScrollArea::vertical().id_salt("presets-scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Only the rows in view are laid out, drawn and given thumbnails: a Lightroom collection
            // imported whole is 10 000+ presets, and building every row each frame took seconds.
            let visible = ui.clip_rect().expand(row_h);
            let searching = !query.trim().is_empty();
            for grp in groups.iter() {
                let g = grp.name.clone();
                let open_id = egui::Id::new(("preset-group", &g));
                let open: bool = searching || ui.data(|d| d.get_temp(open_id)).unwrap_or(true);
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                crate::widgets::announce(&resp, g.to_string());
                register(ui.ctx(), format!("presetGroup:{g}"), r);
                paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(16.0, 16.0)),
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    t.text_label,
                );
                let mut x = r.left() + 36.0;
                let icon = match g.as_str() {
                    crate::preset_picker::FAVORITES => Some(Icon::StarFilled),
                    crate::preset_picker::FOR_IMPORT => Some(Icon::ImportArrow),
                    _ => None,
                };
                if let Some(i) = icon {
                    paint(ui.painter(), Rect::from_center_size(pos2(x + 7.0, r.center().y), vec2(14.0, 14.0)), i, t.text_label);
                    x += 20.0;
                }
                ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, &grp.shown, t.semibold(13.0), t.text_label);
                if resp.clicked() {
                    ui.data_mut(|d| d.insert_temp(open_id, !open));
                }
                if !grp.pseudo {
                    resp.context_menu(|ui| {
                        if ui.button(crate::i18n::tr("Export Group…")).clicked() {
                            let _ = app.run("file.exportPresets", json!({"group": g}));
                        }
                    });
                }
                if !open {
                    continue;
                }
                let (first, end) = rows_in_view(ui.cursor().top(), row_h, grp.items.len(), visible);
                if first > 0 {
                    ui.allocate_space(vec2(ui.available_width(), first as f32 * row_h));
                }
                for &i in grp.items.get(first..end).unwrap_or_default() {
                    let Some(pr) = app.session.presets.get(i).cloned() else { continue };
                    let (pid, name, fav) = (pr.id.clone(), crate::i18n::builtin_label(&pr.name, pr.builtin).to_string(), pr.favorite);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    crate::widgets::announce(&resp, name.clone());
                    register(ui.ctx(), format!("preset:{pid}"), r);
                    let is_last = last.as_ref().is_some_and(|l| l.id == pid);
                    if is_last {
                        ui.painter().rect_filled(r, 0.0, t.tool_active);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.7));
                    }
                    let look = if thumbs || resp.hovered() { current.as_ref().map(|d| pr.apply(d, 1.0, aspect)) } else { None };
                    // its AI masks: what is known shows at once; resting on it ~150 ms computes the rest
                    let look = match (look, active) {
                        (Some(s), Some(id)) if pr.settings.get("masks").is_some() => {
                            let compute = resp.hovered() && {
                                let since = ui.data(|d| d.get_temp::<(String, f64)>(hover_since_id)).filter(|(p, _)| *p == pid).map(|(_, t)| t);
                                let since = since.unwrap_or_else(|| {
                                    ui.data_mut(|d| d.insert_temp(hover_since_id, (pid.clone(), now)));
                                    now
                                });
                                now - since >= 0.15
                            };
                            let (s, complete) = app.session.preview_preset(id, s, compute);
                            if resp.hovered() && !complete {
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                            }
                            Some(s)
                        }
                        (look, _) => look,
                    };
                    hovered_any |= resp.hovered();
                    if pr.has_ai_masks() {
                        // AI: its masks are computed on each photo
                        let badge = ui.painter().text(
                            pos2(r.right() - if fav { 38.0 } else { 20.0 }, r.center().y),
                            Align2::RIGHT_CENTER,
                            crate::i18n::tr("AI"),
                            t.font(10.5),
                            t.text_dim,
                        );
                        register(ui.ctx(), format!("presetAi:{pid}"), badge);
                    }
                    let mut text_x = r.left() + 40.0;
                    if thumbs && let (Some(id), Some(s)) = (active, look.as_ref()) {
                        let tr = Rect::from_min_size(pos2(r.left() + 36.0, r.top() + 5.0), vec2(66.0, row_h - 10.0));
                        ui.painter().rect_filled(tr, 2.0, t.canvas);
                        if let Some(job) = app.session.variant_job(id, s, THUMB_EDGE)
                            && let Some(tex) = app.renderer.variant(job)
                        {
                            ui.painter().image(tex.id(), tr, cover_uv(tr, tex.size), Color32::WHITE);
                        }
                        text_x = tr.right() + 10.0;
                    }
                    ui.painter().text(pos2(text_x, r.center().y), Align2::LEFT_CENTER, &name, t.font(13.0), t.text_label);
                    if fav {
                        paint(ui.painter(), Rect::from_center_size(pos2(r.right() - 20.0, r.center().y), vec2(16.0, 16.0)), Icon::StarFilled, t.star);
                    }
                    // resting on a preset previews it in the loupe (no history entry)
                    if resp.hovered()
                        && let Some(s) = look
                    {
                        let label = crate::i18n::tr_format!("Preset: {name}", name = name);
                        app.hover_preview = Some(HoverPreview { label: label.clone(), settings: s });
                        if let Some(id) = active {
                            ui.data_mut(|d| d.insert_temp(egui::Id::new("last-preset-hover"), (id, pr.clone(), label)));
                        }
                    }
                    if resp.clicked() && app.run("preset.apply", json!({"id": pid, "amount": 100})).is_ok() {
                        ui.data_mut(|d| d.insert_temp(amt_id, AppliedPreset::new(&app.session, pid.clone(), 100.0)));
                        ui.data_mut(|d| d.remove::<(lightcraft_catalog::PhotoId, Preset, String)>(egui::Id::new("last-preset-hover")));
                        app.toast(ui.ctx(), crate::i18n::tr_format!("Preset: {name}", name = name));
                    }
                    let (builtin, group) = (pr.builtin, pr.group.clone());
                    let for_import = app.session.preset_for_import(&pr);
                    resp.context_menu(|ui| {
                        if ui.button(crate::i18n::tr(if fav { "Remove from Favorites" } else { "Add to Favorites" })).clicked() {
                            let _ = app.run("preset.favorite", json!({"id": pid}));
                        }
                        let label = if for_import { "Take out of At Import" } else { "Add to At Import (shown first when importing)" };
                        if ui.button(crate::i18n::tr(label)).clicked() {
                            let _ = app.run("preset.forImport", json!({"id": pid, "on": !for_import}));
                        }
                        if builtin {
                            return;
                        }
                        ui.separator();
                        if ui
                            .add_enabled(app.session.active().is_some(), egui::Button::new(crate::i18n::tr("Update with Current Settings")))
                            .clicked()
                        {
                            match app.run("preset.update", json!({"id": pid})) {
                                Ok(_) => app.toast(ui.ctx(), crate::i18n::tr_format!("Updated “{name}”", name = name)),
                                Err(e) => app.toast(ui.ctx(), e),
                            }
                        }
                        if ui.button(crate::i18n::tr("Rename…")).clicked() {
                            crate::panels::dialogs::prompt(app, "Rename Preset", "Preset name", &name, "preset.rename", json!({"id": pid}), "name");
                        }
                        crate::widgets::menu_button(ui, crate::i18n::tr("Move to Group"), |ui| {
                            for g in groups.iter().filter(|x| x.user && x.name != group).map(|x| &x.name) {
                                if ui.button(g).clicked() {
                                    let _ = app.run("preset.move", json!({"id": pid, "group": g}));
                                }
                            }
                            if ui.button(crate::i18n::tr("New Group…")).clicked() {
                                crate::panels::dialogs::prompt(
                                    app,
                                    "Move Preset to New Group",
                                    "Group name",
                                    "",
                                    "preset.move",
                                    json!({"id": pid}),
                                    "group",
                                );
                            }
                        });
                        ui.separator();
                        if ui.button(crate::i18n::tr("Delete Preset")).clicked() {
                            let _ = app.run("preset.delete", json!({"id": pid}));
                        }
                    });
                }
                if end < grp.items.len() {
                    ui.allocate_space(vec2(ui.available_width(), (grp.items.len() - end) as f32 * row_h));
                }
            }
            ui.add_space(30.0);
        });
        let last_hover_id = egui::Id::new("last-preset-hover");
        let pointer_in_scroll = ui.ctx().input(|i| i.pointer.hover_pos()).is_some_and(|pos| scroll.inner_rect.contains(pos));
        if pointer_in_scroll {
            if app.hover_preview.is_none()
                && let (Some(id), Some(curr)) = (active, current.as_ref())
                && let Some((saved_id, saved_preset, saved_label)) =
                    ui.data(|d| d.get_temp::<(lightcraft_catalog::PhotoId, Preset, String)>(last_hover_id))
                && saved_id == id
            {
                let look = saved_preset.apply(curr, 1.0, aspect);
                let look = if saved_preset.settings.get("masks").is_some() { app.session.preview_preset(id, look, false).0 } else { look };
                app.hover_preview = Some(HoverPreview { label: saved_label, settings: look });
            }
        } else {
            ui.data_mut(|d| d.remove::<(lightcraft_catalog::PhotoId, Preset, String)>(last_hover_id));
        }
        if !hovered_any {
            // off the presets: the preview's selections still queued are skipped
            ui.data_mut(|d| d.remove::<(String, f64)>(hover_since_id));
            app.session.cancel_preview();
        }
    });
}

/// A preset group as the panel lists it: indices into the session's presets.
struct Group {
    /// The group's name, or Favorites' / At Import's key.
    name: String,
    /// As shown (a stock group in the UI language).
    shown: String,
    /// Holds a user preset (a Move to Group target).
    user: bool,
    /// Favorites or At Import.
    pseudo: bool,
    items: Vec<usize>,
}

/// The presets by group as every preset picker lists them ([`crate::preset_picker::groups`]:
/// Favorites and At Import first, then the groups in the Presets Manager's order; hidden and
/// removed ones left out; `query` searches), rebuilt only when the list, the manager's choices or
/// the search changed.
fn grouped(ctx: &egui::Context, session: &lightcraft_engine::Session, query: &str) -> Arc<Vec<Group>> {
    let presets: &PresetList = &session.presets;
    let id = egui::Id::new("presets-grouped");
    let key = (presets.generation(), session.preset_state_gen, query.to_string());
    if let Some((k, groups)) = ctx.data(|d| d.get_temp::<((u64, u64, String), Arc<Vec<Group>>)>(id))
        && k == key
    {
        return groups;
    }
    let list: Vec<Group> = crate::preset_picker::groups(session, query, crate::preset_picker::View::All)
        .into_iter()
        .map(|g| {
            let pseudo = g.pseudo();
            let user = !pseudo && g.items.iter().any(|i| presets.get(*i).is_some_and(|p| !p.builtin));
            Group { name: g.key, shown: g.shown, user, pseudo, items: g.items }
        })
        .collect();
    let groups = Arc::new(list);
    ctx.data_mut(|d| d.insert_temp(id, (key, groups.clone())));
    groups
}

/// The rows `first..end` of a block of `n` rows `row_h` high starting at `top` that meet
/// `visible`.
fn rows_in_view(top: f32, row_h: f32, n: usize, visible: Rect) -> (usize, usize) {
    if row_h <= 0.0 || !top.is_finite() {
        return (0, n);
    }
    // `as usize` saturates (negative and NaN give 0)
    let first = (((visible.top() - top) / row_h).floor() as usize).min(n);
    let end = (((visible.bottom() - top) / row_h).ceil() as usize).clamp(first, n);
    (first, end)
}

/// The part of a `size` image that covers `r` (centre crop), as texture coordinates.
pub fn cover_uv(r: Rect, size: [usize; 2]) -> Rect {
    let a = size[0] as f32 / size[1].max(1) as f32;
    let ra = r.width() / r.height().max(1.0);
    if a > ra {
        let f = ra / a;
        Rect::from_min_max(pos2((1.0 - f) / 2.0, 0.0), pos2((1.0 + f) / 2.0, 1.0))
    } else {
        let f = a / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - f) / 2.0), pos2(1.0, (1.0 + f) / 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_slider_does_not_undo_a_later_edit() {
        let mut session = lightcraft_engine::Session::with_demo();
        let pid = "lc.golden-hour";
        session.execute("preset.apply", &json!({"id": pid})).unwrap();
        let state = AppliedPreset::new(&session, pid.into(), 100.0);
        assert!(state.is_current(&session));

        session.execute("develop.set", &json!({"values": {"light.exposure": 0.75}})).unwrap();
        assert!(!state.is_current(&session), "a newer edit owns the undo step");
    }

    #[test]
    fn amount_slider_does_not_undo_on_another_photo() {
        let mut session = lightcraft_engine::Session::with_demo();
        let pid = "lc.golden-hour";
        session.execute("preset.apply", &json!({"id": pid})).unwrap();
        let state = AppliedPreset::new(&session, pid.into(), 100.0);
        let active = session.active();
        let other = session.visible().iter().copied().find(|id| Some(*id) != active).unwrap();
        session.execute("library.select", &json!({"ids": [other.0]})).unwrap();
        assert!(!state.is_current(&session), "the preset belongs to the earlier selection");
    }
}
