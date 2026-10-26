//! The Masks list as a tree (as Lightroom Classic and Capture One show it): one row per mask with
//! its thumbnail, name, the eye (the mask's effect on / off) and a bin; under the selected mask its
//! components, each with its own thumbnail, operation and Invert, then Add / Subtract / Intersect.
//! The overlay — shown by itself for a fresh mask, a tool in use, or the row / pin under the
//! pointer — has its own button (O) and a small settings popup (mode, colour, opacity, pins).

use egui::{Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, MaskOp};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{icon_button, one_line, register, text_button};

const ROW: f32 = 34.0;
const CHILD: f32 = 28.0;
const THUMB: f32 = 26.0;
const CHILD_THUMB: f32 = 20.0;

/// The Masks section: header (overlay controls), the tree, the selected mask's operations.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId, d: &DevelopSettings) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Masks")).font(t.semibold(13.0)).color(t.text_label));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let settings = icon_button(ui, "maskOverlaySettings", Icon::Dots, vec2(22.0, 22.0), false, true, "Overlay settings");
            egui::Popup::menu(&settings).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| overlay_settings(app, ui));
            overlay_button(app, ui);
            let all = app.ui.mask_overlay_all;
            if icon_button(ui, "maskOverlayAll", Icon::Stack, vec2(22.0, 22.0), all, true, "Show all overlays").clicked() {
                let _ = app.run("view.maskOverlayAll", json!({"on": !all}));
            }
        });
    });
    if d.masks.is_empty() {
        ui.label(egui::RichText::new(crate::i18n::tr("No masks yet. Choose a mask type above.")).color(t.text_dim));
        return;
    }
    let count = d.masks.len();
    for (index, m) in d.masks.iter().enumerate() {
        let sel = app.session.active_mask == Some(m.id);
        // inline rename: Enter (or leaving the field) commits, Escape cancels; empty = automatic name
        if let Some((rid, name)) = app.ui.renaming_mask.as_mut().filter(|(rid, _)| *rid == m.id) {
            let rid = *rid;
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("maskRename", rid)));
            register(ui.ctx(), format!("maskRename:{rid}"), r.rect);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.ui.renaming_mask = None;
            } else if r.lost_focus() {
                let name = name.trim().to_string();
                app.ui.renaming_mask = None;
                if name != m.name {
                    let _ = app.run("mask.rename", json!({"id": rid, "name": name}));
                }
            }
            continue;
        }
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        register(ui.ctx(), format!("mask:{}", m.id), r);
        let hovered = ui.rect_contains_pointer(r);
        if hovered {
            app.ui.mask_hover = Some(m.id);
        }
        ui.painter().rect_filled(
            r,
            4.0,
            if sel {
                t.tool_active
            } else if hovered {
                t.hover.gamma_multiply(0.7)
            } else {
                t.chrome
            },
        );
        // thumbnail
        let tr = Rect::from_min_size(r.min + vec2(4.0, (ROW - THUMB) / 2.0), vec2(THUMB, THUMB));
        let tex = crate::mask_thumbs::thumb(app, ui.ctx(), id, d, m);
        crate::mask_thumbs::paint(ui.painter(), tr, tex.as_ref(), if sel { t.text } else { t.button_border });
        // the bin and the eye (effect on / off) at the right
        let bin = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(20.0, 20.0));
        let eye = Rect::from_center_size(pos2(r.right() - 38.0, r.center().y), vec2(20.0, 20.0));
        // a long name is cut short (with …) before the buttons, the full name on hover
        let color = if m.visible { t.text } else { t.text_disabled };
        // a mask from before automatic names ("Mask 1") shows what it selects
        let shown = if lightcraft_develop::recipe::generic_name(&m.name) && !m.components.is_empty() {
            lightcraft_develop::recipe::content_name(m)
        } else {
            m.name.clone()
        };
        // note 2: a mask whose AI selection isn't there for this photo (copied, synced, a preset's,
        // made before its model) has a badge before the eye: click it to update (or to pick the
        // object again)
        let status = super::masking::mask_ai_status(m).filter(|_| crate::panels::detail::ai_computing(app, m).is_none());
        let badge = Rect::from_center_size(pos2(eye.left() - 12.0, r.center().y), vec2(16.0, 16.0));
        let name_right = if status.is_some() { badge.left() } else { eye.left() };
        let g = one_line(ui.painter(), &shown, t.font(12.5), color, (name_right - tr.right() - 12.0).max(10.0));
        let elided = g.elided;
        let at = pos2(tr.right() + 8.0, r.center().y - g.size().y / 2.0);
        register(ui.ctx(), format!("maskName:{}", m.id), Rect::from_min_size(at, g.size()));
        ui.painter().galley(at, g, color);
        if let Some(why) = status {
            let bv = ui.interact(badge, egui::Id::new(("maskStale", m.id)), Sense::click());
            register(ui.ctx(), format!("maskStale:{}", m.id), badge);
            let warn = t.caution;
            ui.painter().circle_filled(badge.center(), 7.0, if bv.hovered() { warn } else { warn.gamma_multiply(0.85) });
            ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, "!", t.semibold(11.0), egui::Color32::BLACK);
            let update = why == super::masking::NEEDS_UPDATE;
            let tip = if update {
                format!("{} {}", crate::i18n::tr(why), crate::i18n::tr("Click to update it."))
            } else {
                crate::i18n::tr(why).to_string()
            };
            if bv.on_hover_text(tip).clicked() {
                let _ = app.run("mask.select", json!({"id": m.id}));
                if update && let Err(e) = app.run("mask.recompute", json!({"id": m.id})) {
                    app.ai_error(ui.ctx(), e, None);
                }
            }
        }
        let ev = ui.interact(eye, egui::Id::new(("maskVisible", m.id)), Sense::click());
        register(ui.ctx(), format!("maskVisible:{}", m.id), eye);
        paint(ui.painter(), eye.shrink(2.0), if m.visible { Icon::Eye } else { Icon::EyeOff }, if ev.hovered() { t.text } else { t.text_dim });
        let ev = ev.on_hover_text(crate::i18n::tr("The mask's effect on / off"));
        if ev.clicked() {
            let _ = app.run("mask.visible", json!({"id": m.id}));
        }
        if hovered || sel {
            let bv = ui.interact(bin, egui::Id::new(("maskBin", m.id)), Sense::click());
            register(ui.ctx(), format!("maskDelete:{}", m.id), bin);
            paint(ui.painter(), bin.shrink(2.0), Icon::Trash, if bv.hovered() { t.text } else { t.text_dim });
            if bv.on_hover_text(crate::i18n::tr("Delete Mask")).clicked() {
                let _ = app.run("mask.delete", json!({"id": m.id}));
                return;
            }
        }
        let resp = if elided { resp.on_hover_text(&shown) } else { resp };
        if resp.clicked() {
            let _ = app.run("mask.select", json!({"id": m.id}));
        }
        if resp.double_clicked() {
            app.ui.renaming_mask = Some((m.id, m.name.clone()));
        }
        crate::widgets::context_menu(&resp, |ui| super::masking::mask_menu(app, ui, m.id, &m.name, m.visible, index, count));
        if sel {
            children(app, ui, id, d, m);
        }
    }
}

/// The overlay button (O): its own icon (a frame with a tinted area, not the Masking tool's), lit
/// in the overlay's colour while the overlay is switched on to stay, which overrides showing it
/// only when useful; the tooltip says which state it is in.
fn overlay_button(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let on = app.ui.mask_overlay;
    let tip = if on { "Overlay always on: O turns it off" } else { "Show overlay (O)" };
    let b = icon_button(ui, "maskOverlay", Icon::Overlay, vec2(22.0, 22.0), on, true, tip);
    if on {
        // lit: the tinted area in the overlay's own colour, and a ring round the button
        let [r, g, bl] = app.ui.mask_overlay_color;
        let c = egui::Color32::from_rgb(r, g, bl);
        let glyph = Rect::from_center_size(b.rect.center(), vec2(13.6, 13.6));
        ui.painter().circle_filled(glyph.center() + vec2(0.7, 0.35), 3.4, c.gamma_multiply(0.85));
        ui.painter().rect_stroke(b.rect.shrink(0.5), 4.0, Stroke::new(1.0, c), egui::StrokeKind::Inside);
    }
    if b.clicked() {
        let _ = app.run("view.maskOverlay", json!({}));
    }
}

/// The selected mask's components (tree children) and its Add / Subtract / Intersect / Invert.
fn children(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId, d: &DevelopSettings, m: &lightcraft_develop::Mask) {
    let t = Tokens::get(ui.ctx());
    for (i, c) in m.components.iter().enumerate() {
        let label = c.name.clone().unwrap_or_else(|| lightcraft_develop::recipe::shape_name(&c.shape));
        if let Some((_, _, name)) = app.ui.renaming_component.as_mut().filter(|(mid, k, _)| *mid == m.id && *k == i) {
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("compRename", m.id, i)));
            register(ui.ctx(), format!("componentRename:{i}"), r.rect);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                app.ui.renaming_component = None;
            } else if r.lost_focus() {
                let name = name.trim().to_string();
                app.ui.renaming_component = None;
                if Some(&name) != c.name.as_ref() {
                    let _ = app.run("mask.component", json!({"id": m.id, "component": i, "action": "rename", "name": name}));
                }
            }
            continue;
        }
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), CHILD), Sense::click());
        register(ui.ctx(), format!("component:{i}"), r);
        let hovered = ui.rect_contains_pointer(r);
        if hovered {
            app.ui.mask_hover = Some(m.id);
            ui.painter().rect_filled(r, 3.0, t.hover.gamma_multiply(0.5));
        }
        // tree line, operation, thumbnail of this component alone
        let x0 = r.left() + 14.0;
        ui.painter().line_segment([pos2(x0, r.top()), pos2(x0, r.center().y)], Stroke::new(1.0, t.button_border));
        ui.painter().line_segment([pos2(x0, r.center().y), pos2(x0 + 8.0, r.center().y)], Stroke::new(1.0, t.button_border));
        let op = match c.op {
            MaskOp::Add => "+",
            MaskOp::Subtract => "−",
            MaskOp::Intersect => "∩",
        };
        let opr = Rect::from_center_size(pos2(x0 + 16.0, r.center().y), vec2(14.0, 14.0));
        ui.painter().text(opr.center(), egui::Align2::CENTER_CENTER, if i == 0 { "" } else { op }, t.semibold(13.0), t.text_label);
        let tr = Rect::from_min_size(pos2(opr.right() + 4.0, r.center().y - CHILD_THUMB / 2.0), vec2(CHILD_THUMB, CHILD_THUMB));
        let single = lightcraft_develop::Mask {
            id: m.id,
            components: vec![lightcraft_develop::MaskComponent { op: MaskOp::Add, ..c.clone() }],
            ..Default::default()
        };
        let tex = crate::mask_thumbs::thumb(app, ui.ctx(), id, d, &single);
        crate::mask_thumbs::paint(ui.painter(), tr, tex.as_ref(), t.button_border);
        // Invert and the options at the right
        let more = Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(18.0, 18.0));
        let inv = Rect::from_center_size(pos2(r.right() - 34.0, r.center().y), vec2(18.0, 18.0));
        let text = format!("{label}{}", if c.invert { crate::i18n::tr(" (inverted)") } else { "" });
        let g = one_line(ui.painter(), &text, t.font(12.0), t.text, (inv.left() - tr.right() - 10.0).max(10.0));
        let elided = g.elided;
        ui.painter().galley(pos2(tr.right() + 6.0, r.center().y - g.size().y / 2.0), g, t.text);
        let iv = ui.interact(inv, egui::Id::new(("compInvert", m.id, i)), Sense::click());
        register(ui.ctx(), format!("componentInvert:{i}"), inv);
        if c.invert {
            ui.painter().rect_filled(inv, 3.0, t.pressed);
        }
        paint(ui.painter(), inv.shrink(2.0), Icon::Invert, if iv.hovered() || c.invert { t.text } else { t.text_dim });
        if iv.on_hover_text(crate::i18n::tr("Invert")).clicked() {
            let _ = app.run("mask.component", json!({"id": m.id, "component": i, "action": "invert"}));
        }
        let mv = ui.interact(more, egui::Id::new(("compMore", m.id, i)), Sense::click());
        register(ui.ctx(), format!("button:componentMenu{i}"), more);
        paint(ui.painter(), more.shrink(2.0), Icon::Dots, if mv.hovered() { t.text } else { t.text_dim });
        egui::Popup::menu(&mv).show(|ui| super::masking::component_row_menu(app, ui, m.id, i, &label, m.components.len()));
        let resp = if elided { resp.on_hover_text(&text) } else { resp };
        if resp.double_clicked() {
            app.ui.renaming_component = Some((m.id, i, label.clone()));
        }
        crate::widgets::context_menu(&resp, |ui| super::masking::component_row_menu(app, ui, m.id, i, &label, m.components.len()));
        super::masking::range_controls(app, ui, i, &c.shape);
    }
    // Add / Subtract / Intersect (menus of what to combine) and Invert, compact under the children
    ui.horizontal_wrapped(|ui| {
        ui.add_space(22.0);
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let add = text_button(ui, "maskAddComp", crate::i18n::tr("Add"), false);
        egui::Popup::menu(&add).show(|ui| super::masking::component_menu(app, ui, "add"));
        let sub = text_button(ui, "maskSubComp", crate::i18n::tr("Subtract"), false);
        egui::Popup::menu(&sub).show(|ui| super::masking::component_menu(app, ui, "subtract"));
        let int = text_button(ui, "maskIntComp", crate::i18n::tr("Intersect"), false);
        egui::Popup::menu(&int).show(|ui| super::masking::component_menu(app, ui, "intersect"));
        if icon_button(ui, "maskInvert", Icon::Invert, vec2(24.0, 22.0), m.invert, true, "Invert Mask").clicked() {
            let _ = app.run("mask.invert", json!({}));
        }
    });
    super::masking::describe_field(app, ui, false);
    if app.ui.mask_pick_op.is_some() {
        super::masking::pickers(app, ui);
    }
}

/// The overlay's settings (rarely changed: set and forget): how the overlay shows, its colour and
/// opacity, and the pins.
fn overlay_settings(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    use lightcraft_pipeline::MaskView;
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(200.0);
    ui.label(egui::RichText::new(crate::i18n::tr("Overlay")).font(t.semibold(12.5)));
    // on: the overlay stays (O); off: it shows only when useful (a new mask, a mask tool, the
    // pointer on a mask's row or pin)
    let mut on = app.ui.mask_overlay;
    let r = ui.checkbox(&mut on, crate::i18n::tr("Show overlay (O)"));
    register(ui.ctx(), "check:maskOverlayOn", r.rect);
    if r.changed() {
        let _ = app.run("view.maskOverlay", json!({"show": on}));
    }
    ui.separator();
    let view = MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default();
    for v in MaskView::ALL {
        let r = ui.selectable_label(v == view, crate::i18n::tr(v.label()));
        register(ui.ctx(), format!("maskOverlayMode:{}", v.name()), r.rect);
        if r.clicked() {
            // choosing how it looks doesn't switch the overlay on for good (only ⇧O does)
            let _ = app.run("view.maskOverlayMode", json!({"mode": v.name(), "show": false}));
        }
    }
    if matches!(view, MaskView::Color | MaskView::ColorOnBw) {
        ui.separator();
        ui.horizontal(|ui| {
            for c in super::masking::OVERLAY_COLORS {
                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                register(ui.ctx(), format!("maskOverlayColor:{:02x}{:02x}{:02x}", c[0], c[1], c[2]), r);
                ui.painter().rect_filled(r.shrink(2.0), 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                if app.ui.mask_overlay_color == c {
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    let _ = app.run("view.maskOverlayColor", json!({"color": c}));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("Opacity"));
            let mut v = app.ui.mask_overlay_opacity;
            let r = ui.add(egui::Slider::new(&mut v, 10.0..=100.0).suffix(" %").integer());
            register(ui.ctx(), "maskOverlayOpacity", r.rect);
            if r.changed() {
                app.ui.mask_overlay_opacity = v;
            }
        });
    }
    ui.separator();
    let mut pins = app.ui.mask_pins;
    if ui.checkbox(&mut pins, crate::i18n::tr("Show Pins")).changed() {
        let _ = app.run("view.maskPins", json!({"show": pins}));
    }
    // when the pins show (Auto / Always / Selected / Never; Shift+H)
    if app.ui.mask_pins {
        super::right::tool_overlay_options(app, ui, true);
    }
}

/// While All Mask Overlays is on (Ctrl+Shift+O), a pill at the top of the photo says so and how
/// to leave it; a click turns it off (widget `loupe:allMasks`; test notes 2026-10-25 note 14: the
/// user couldn't tell which overlay was on or how to get rid of it).
pub(crate) fn all_masks_pill(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, over: Rect) {
    if !app.ui.mask_overlay_all || app.ui.right != crate::state::RightPanel::Masking || app.ui.fullscreen {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let text = crate::i18n::tr("All masks shown · O to hide");
    let g = p.layout_no_wrap(text.to_string(), t.font(12.0), egui::Color32::from_gray(225));
    let size = vec2(g.size().x + 20.0, 22.0);
    let over = if over.is_positive() { over.intersect(canvas) } else { canvas };
    let r = Rect::from_min_size(pos2(over.center().x - size.x / 2.0, over.top() + 10.0), size);
    let resp = ui.interact(r, egui::Id::new("loupe-all-masks-pill"), Sense::click());
    crate::widgets::announce(&resp, text);
    register(ui.ctx(), "loupe:allMasks", r);
    p.rect_filled(r, 11.0, egui::Color32::from_black_alpha(if resp.hovered() { 225 } else { 185 }));
    p.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, egui::Color32::from_gray(225));
    if resp.on_hover_text(crate::i18n::tr("Click to hide the overlay")).clicked() {
        let _ = app.run("view.maskOverlay", json!({"show": false}));
    }
}
