//! The profile browser (the profile menu's "Browse…", or the grid button next to it): every
//! profile as a live thumbnail of the current photo, grouped (Favorites first), with a favourite
//! star, the applied profile's amount, a temporary loupe preview while hovering and apply on click.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::DevelopSettings;
use serde_json::json;

use super::presets::cover_uv;
use crate::icons::{Icon, paint};
use crate::state::RightPanel;
use crate::theme::Tokens;
use crate::widgets::{divider, icon_button, one_line, register};
use crate::{HoverPreview, LightcraftApp};

/// Long edge of the variant thumbnails (px).
const THUMB_EDGE: usize = 256;
/// Thumbnail columns.
const COLUMNS: usize = 2;

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    let d = (*app.session.develop_of(id).unwrap_or_default()).clone();
    // header: back + title
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let mut back = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(hr.min + vec2(12.0, 9.0), vec2(28.0, 28.0))));
    if icon_button(&mut back, "profilesBack", Icon::ChevronLeft, vec2(28.0, 28.0), false, true, "Back to Edit").clicked() {
        app.ui.right = RightPanel::Edit;
    }
    ui.painter().text(pos2(hr.left() + 46.0, hr.center().y + 2.0), Align2::LEFT_CENTER, crate::i18n::tr("Profiles"), t.semibold(15.0), t.text);
    // the Profile Manager: hide, delete, rename, group, import, body defaults
    let manage = Rect::from_min_size(pos2(hr.right() - 92.0, hr.top() + 11.0), vec2(80.0, 24.0));
    let mut mb = ui.new_child(egui::UiBuilder::new().max_rect(manage).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let r = mb.small_button(crate::i18n::tr("Manage…"));
    register(ui.ctx(), "button:profilesManage", r.rect);
    if r.on_hover_text(crate::i18n::tr("Hide, delete, rename, group and import profiles; camera body defaults")).clicked() {
        let _ = app.run("app.settings", json!({"tab": "profiles"}));
    }
    divider(ui);
    // the applied profile and its amount
    let (cr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let name = app.session.profile_info(&d.profile.id).map(|(name, _)| name).unwrap_or("Color");
    ui.painter().text(pos2(cr.left() + 24.0, cr.center().y), Align2::LEFT_CENTER, crate::i18n::tr("Profile"), t.font(13.0), t.text_dim);
    // a long name is cut short (with …) at the panel's edge
    let g = one_line(ui.painter(), crate::i18n::profile_label(&d.profile.id, name), t.font(14.0), t.text_label, cr.width() - 84.0 - 24.0);
    ui.painter().galley(pos2(cr.left() + 84.0, cr.center().y - g.size().y / 2.0), g, t.text_label);
    if d.profile.id != "lc.color" {
        super::edit::control(app, ui, &d, "profile.amount", true);
        ui.add_space(6.0);
    }
    divider(ui);
    // (title, cell tag, imported LUT folder, profiles)
    let mut sections: Vec<(&str, &str, bool, Vec<(String, String, String)>)> = Vec::new();
    let favs: Vec<(String, String, String)> = app
        .session
        .profile_favorites
        .iter()
        .filter_map(|f| app.session.profile_info(f).map(|(name, group)| (f.clone(), name.to_string(), group.to_string())))
        .collect();
    if !favs.is_empty() {
        sections.push(("Favorites", "fav", false, favs));
    }
    // every group as the Profile Manager orders it, without the hidden profiles
    let shown = app.session.shown_profiles();
    let group_names = lightcraft_engine::Session::profile_group_order(&shown);
    for g in &group_names {
        let members: Vec<&lightcraft_engine::profile_manager::ProfileItem> = shown.iter().filter(|p| &p.group == g).collect();
        let imported = !members.iter().all(|p| p.builtin_group());
        sections.push((g.as_str(), "", imported, members.iter().map(|p| (p.id.clone(), p.name.clone(), p.group.clone())).collect()));
    }
    for (title, tag, imported, list) in sections {
        // an imported folder named like a built-in group still opens and closes on its own
        let open_id = egui::Id::new(("profile-group", imported, title));
        let open: bool = ui.data(|m| m.get_temp(open_id)).unwrap_or(true);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        crate::widgets::announce(&resp, title.to_string());
        register(ui.ctx(), if imported { format!("profileGroup:lut:{title}") } else { format!("profileGroup:{title}") }, r);
        paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(16.0, 16.0)),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            t.text_label,
        );
        ui.painter().text(
            pos2(r.left() + 36.0, r.center().y),
            Align2::LEFT_CENTER,
            crate::i18n::builtin_label(title, !imported),
            t.semibold(13.0),
            t.text_label,
        );
        ui.painter().text(pos2(r.right() - 22.0, r.center().y), Align2::RIGHT_CENTER, list.len().to_string(), t.font(12.0), t.text_dim);
        if resp.clicked() {
            ui.data_mut(|m| m.insert_temp(open_id, !open));
        }
        if !open {
            continue;
        }
        let gap = 8.0;
        let cw = ((ui.available_width() - 44.0 - gap * (COLUMNS as f32 - 1.0)) / COLUMNS as f32).max(40.0);
        let th = (cw * 0.75).round();
        for row in list.chunks(COLUMNS) {
            let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), th + 30.0), Sense::hover());
            // only the cells in view ask for their thumbnail: every cell asking re-rendered the
            // photo once per profile at every edit — hundreds of imported profiles, hundreds of
            // renders queued at once (TEST-NOTES 2026-10-16 item 15)
            let in_view = ui.is_rect_visible(rr.expand(th));
            for (i, p) in row.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(rr.left() + 22.0 + i as f32 * (cw + gap), rr.top()), vec2(cw, th + 24.0));
                cell_ui(app, ui, &d, id, tag, p, cell, th, in_view);
            }
        }
        ui.add_space(6.0);
    }
    ui.add_space(30.0);
    let last_hover_id = egui::Id::new("last-profile-hover");
    let pointer_in_profiles = ui.ctx().input(|i| i.pointer.hover_pos()).is_some_and(|pos| ui.max_rect().contains(pos));
    if pointer_in_profiles {
        if app.hover_preview.is_none()
            && let Some((saved_id, saved_profile_id)) = ui.data(|m| m.get_temp::<(PhotoId, String)>(last_hover_id))
            && saved_id == id
            && let Some((p_name, _)) = app.session.profile_info(&saved_profile_id)
        {
            let s = with_profile_id(&app.session, &d, &saved_profile_id);
            app.hover_preview = Some(HoverPreview {
                label: crate::i18n::tr_format!("Profile: {}", crate::i18n::profile_label(&saved_profile_id, p_name)),
                settings: s,
            });
        }
    } else {
        ui.data_mut(|m| m.remove::<(PhotoId, String)>(last_hover_id));
    }
}

/// `d` with profile `profile_id` chosen as the menus choose it (a camera profile sets the camera
/// calibration, a look the look; at 100 % unless it is the applied one).
fn with_profile_id(session: &lightcraft_engine::Session, d: &DevelopSettings, profile_id: &str) -> DevelopSettings {
    let mut s = d.clone();
    if s.profile.id != profile_id && s.calibration_id() != Some(profile_id) {
        lightcraft_engine::cmd::dcp_profiles::choose(session, &mut s, profile_id);
        s.profile.amount = 100.0;
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn cell_ui(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    d: &DevelopSettings,
    id: PhotoId,
    tag: &str,
    p: &(String, String, String),
    cell: Rect,
    th: f32,
    in_view: bool,
) {
    let (p_id, p_name, p_group) = (p.0.as_str(), p.1.as_str(), p.2.as_str());
    let t = Tokens::get(ui.ctx());
    let key = if tag.is_empty() { format!("profileCell:{p_id}") } else { format!("profileCell:{tag}:{p_id}") };
    let resp = ui.interact(cell, egui::Id::new(&key), Sense::click());
    crate::widgets::announce(&resp, p_name.to_string());
    register(ui.ctx(), key, cell);
    let thumb = Rect::from_min_size(cell.min, vec2(cell.width(), th));
    // (the photo's settings with this profile: only for a cell in view or under the pointer)
    let s = (in_view || resp.hovered()).then(|| with_profile_id(&app.session, d, p_id));
    let painter = ui.painter();
    painter.rect_filled(thumb, 3.0, t.canvas);
    if in_view
        && let Some(s) = &s
        && let Some(job) = app.session.variant_job(id, s, THUMB_EDGE)
        && let Some(tex) = app.renderer.variant(job)
    {
        painter.image(tex.id(), thumb, cover_uv(thumb, tex.size), Color32::WHITE);
    }
    let applied = d.profile.id == p_id;
    if applied {
        painter.rect_stroke(thumb.expand(1.0), 4.0, Stroke::new(2.0, t.pick), egui::StrokeKind::Outside);
    } else if resp.hovered() {
        painter.rect_stroke(thumb, 3.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Outside);
    }
    let name_color = if applied { t.text } else { t.text_label };
    // cut short (with …) to the cell, so it never runs into its neighbour; the full name on hover
    let g = one_line(painter, shown_label(&app.session, p_id, p_name), t.font(12.0), name_color, cell.width() - 2.0);
    painter.galley(pos2(cell.left() + 2.0, thumb.bottom() + 12.0 - g.size().y / 2.0), g, name_color);
    // favourite star (always shown when set, on hover otherwise)
    let fav = app.session.profile_favorites.iter().any(|f| f == p_id);
    let star = Rect::from_center_size(pos2(thumb.right() - 13.0, thumb.top() + 13.0), vec2(20.0, 20.0));
    register(ui.ctx(), format!("profileStar:{}{}", if tag.is_empty() { "" } else { "fav:" }, p_id), star);
    let over_star = resp.hover_pos().is_some_and(|q| star.contains(q));
    if fav || resp.hovered() {
        painter.circle_filled(star.center(), 10.0, Color32::from_black_alpha(110));
        paint(painter, star.shrink(4.0), if fav { Icon::StarFilled } else { Icon::Star }, if fav || over_star { t.star } else { Color32::WHITE });
    }
    if resp.hovered()
        && !applied
        && !over_star
        && let Some(s) = s
    {
        app.hover_preview =
            Some(HoverPreview { label: crate::i18n::tr_format!("Profile: {}", crate::i18n::profile_label(p_id, p_name)), settings: s });
        ui.data_mut(|m| m.insert_temp(egui::Id::new("last-profile-hover"), (id, p_id.to_string())));
    } else if resp.hovered() && applied {
        // resting on the applied profile shows it (no preview), not the last other profile hovered
        ui.data_mut(|m| m.remove::<(PhotoId, String)>(egui::Id::new("last-profile-hover")));
    }
    let resp = resp.on_hover_text(format!(
        "{} ({})",
        crate::i18n::profile_label(p_id, p_name),
        crate::i18n::builtin_label(p_group, lightcraft_engine::presets::profile(p_id).is_some())
    ));
    if resp.clicked() {
        if resp.interact_pointer_pos().is_some_and(|q| star.contains(q)) {
            let _ = app.run("profile.favorite", json!({"id": p_id}));
        } else if !applied {
            let _ = app.run("develop.profile", json!({"id": p_id, "amount": 100}));
            ui.data_mut(|m| m.remove::<(PhotoId, String)>(egui::Id::new("last-profile-hover")));
        }
    }
}

/// A profile's name for display: a built-in look's own name is translated, a name the user gave it is
/// shown as typed.
fn shown_label<'a>(s: &lightcraft_engine::Session, id: &str, name: &'a str) -> &'a str {
    if s.profile_state.names.contains_key(id) { name } else { crate::i18n::profile_label(id, name) }
}
