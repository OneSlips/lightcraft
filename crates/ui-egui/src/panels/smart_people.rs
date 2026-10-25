//! Who is who, in the Masking panel and the People view: the People picker shows the faces on the
//! photo (thumbnails, with their names and roles) instead of numbered persons; a face can be
//! named and given a role in the shoot ("Bride"); masks then select a person by name or by role,
//! which presets carry to other photos. Also: remembered objects (a mask tile and Remember as…),
//! the Roles window and the forget actions. Kept apart from `masking.rs` to keep the face-identity
//! code in its own place.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::text_field::TextField;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

/// Roles offered as one-click chips (others can be typed).
const ROLES: [&str; 6] = ["Bride", "Groom", "Best Man", "Maid of Honour", "Mother of the Bride", "Father of the Bride"];

/// Side of a face thumbnail in the picker (points).
const FACE: f32 = 52.0;

/// The People picker under the mask tiles (the People tile was clicked): everyone or one of the
/// faces found on the photo (thumbnails), naming and roles, the parts, and Create Mask. Without
/// the face model it offers Person 1–4 (counted from the left).
pub fn people_picker(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let Some((mut person, mut parts)) = app.ui.people_pick.clone() else { return };
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    // the picker's title, with a button that folds it away (as the People tile does)
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("People")).font(t.semibold(12.5)).color(t.text_label));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::icon_button(ui, "maskPeople:close", crate::icons::Icon::ChevronDown, egui::vec2(20.0, 20.0), false, true, "Close")
                .clicked()
            {
                app.ui.people_pick = None;
                app.ui.people_face = None;
            }
        });
    });
    if app.ui.people_pick.is_none() {
        return;
    }
    let Some(photo) = app.session.active() else { return };
    let faces: Option<Result<Option<Vec<Value>>, String>> =
        if lightcraft_engine::segment::Segmenter::AVAILABLE { Some(app.session.faces_here(photo)) } else { None };
    let mut chosen: Option<Value> = None;
    match &faces {
        Some(Ok(Some(list))) => {
            ui.horizontal_wrapped(|ui| {
                let r = ui.selectable_label(person == 0, crate::i18n::tr("Everyone"));
                register(ui.ctx(), "maskPeople:person:0", r.rect);
                if r.clicked() {
                    person = 0;
                    app.ui.people_face = None;
                }
                for (i, f) in list.iter().enumerate() {
                    let selected = app.ui.people_face == Some(i);
                    if face_tile(app, ui, "maskPeople:face", photo, i, f, selected) {
                        app.ui.people_face = Some(i);
                        person = u32::try_from(i + 1).unwrap_or(u32::MAX);
                        let named = f.get("person").and_then(Value::as_str).unwrap_or_default().to_string();
                        app.ui.people_name = named.clone();
                        app.ui.people_role = first_role(f).unwrap_or_default();
                        app.ui.people_by_role = !app.ui.people_role.is_empty();
                    }
                }
            });
            if list.is_empty() {
                ui.label(egui::RichText::new(crate::i18n::tr("No faces were found on this photo.")).color(t.text_dim));
            }
            chosen = app.ui.people_face.and_then(|i| list.get(i).cloned());
        }
        Some(Ok(None)) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new(crate::i18n::tr("Finding faces…")).color(t.text_dim));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        _ => {
            // no face model: people counted from the left
            ui.horizontal_wrapped(|ui| {
                let r = ui.selectable_label(person == 0, crate::i18n::tr("Everyone"));
                register(ui.ctx(), "maskPeople:person:0", r.rect);
                if r.clicked() {
                    person = 0;
                }
                for n in 1..=4u32 {
                    let r = ui.selectable_label(person == n, crate::i18n::tr_format!("Person {n}", n = n));
                    register(ui.ctx(), format!("maskPeople:person:{n}"), r.rect);
                    if r.clicked() {
                        person = n;
                    }
                }
            });
        }
    }
    if let Some(f) = &chosen {
        naming(app, ui, photo, f);
    }
    // the parts
    ui.horizontal_wrapped(|ui| {
        for part in super::masking::PEOPLE_PARTS {
            let whole = part == "Entire Person";
            let mut on = if whole { parts.is_empty() } else { parts.iter().any(|p| p == part) };
            let r = ui.checkbox(&mut on, crate::i18n::tr(part));
            register(ui.ctx(), format!("maskPeople:part:{part}"), r.rect);
            if r.changed() {
                if whole {
                    parts.clear();
                } else if on {
                    parts.push(part.to_string());
                } else {
                    parts.retain(|p| p != part);
                }
            }
        }
    });
    ui.horizontal(|ui| {
        let create = ui.button(crate::i18n::tr("Create Mask"));
        register(ui.ctx(), "maskPeople:create", create.rect);
        if text_button(ui, "maskPeople:roles", crate::i18n::tr("Roles…"), false).clicked() {
            app.ui.roles_window = true;
        }
        if create.clicked() {
            let mut p = json!({"kind": "people", "person": person, "parts": parts});
            // by name or role: a preset made from this mask finds them on other photos
            if let Some(f) = &chosen
                && let Some(name) = f.get("person").and_then(Value::as_str)
            {
                let role = first_role(f);
                p["who"] = match (app.ui.people_by_role, role) {
                    (true, Some(r)) => json!({"role": r}),
                    _ => json!({"name": name}),
                };
                p["person"] = json!(0);
            }
            app.ui.people_pick = None;
            app.ui.people_face = None;
            // (opened from Add / Subtract / Intersect: combined with the selected mask)
            let op = app.ui.mask_pick_op.take();
            super::masking::add_auto(app, ui.ctx(), p, op.as_deref());
            return;
        }
        app.ui.people_pick = Some((person, parts));
    });
}

fn first_role(f: &Value) -> Option<String> {
    f.get("roles").and_then(Value::as_array).and_then(|r| r.first()).and_then(Value::as_str).map(str::to_string)
}

/// The people this photo's masks couldn't find with confidence (a look-alike beside the bride, a
/// portrait framed closer than where she was named: test notes 2026-10-25, note 12): their part
/// of the mask is left out here, and one click on their face among the photo's faces points at
/// them (`people.pointAt`: their face named, the mask computed again).
pub fn point_people(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let Some(photo) = app.session.active() else { return };
    let who = app.session.people_to_point(photo);
    if who.is_empty() || !lightcraft_engine::segment::Segmenter::AVAILABLE {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let faces = match app.session.faces_here(photo) {
        Ok(Some(f)) => f,
        Ok(None) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new(crate::i18n::tr("Finding faces…")).color(t.text_dim));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        Err(_) => return,
    };
    for (k, w) in who.iter().enumerate() {
        let label = w.label();
        ui.add_space(6.0);
        let say = if faces.is_empty() {
            format!("{label}: {}", crate::i18n::tr("not found on this photo, and no faces were found to point at."))
        } else {
            format!("{label}: {}", crate::i18n::tr("not found with confidence on this photo, so their part of the mask is left out. Click their face:"))
        };
        let r = ui.add(egui::Label::new(egui::RichText::new(say).color(t.text_label)).wrap());
        register(ui.ctx(), format!("maskPoint:{k}"), r.rect);
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            for (i, f) in faces.iter().enumerate() {
                if face_tile(app, ui, &format!("maskPoint:{k}:face"), photo, i, f, false) {
                    clicked = Some(i);
                }
            }
        });
        if let Some(i) = clicked {
            let who = match w {
                lightcraft_develop::recipe::Who::Name(n) => json!({"name": n}),
                lightcraft_develop::recipe::Who::Role(r) => json!({"role": r}),
            };
            match app.run("people.pointAt", json!({"id": photo.0, "face": i, "who": who})) {
                Ok(_) => app.toast(ui.ctx(), format!("{label}: {}", crate::i18n::tr("pointed out; the mask is updating"))),
                Err(e) => app.toast_error(ui.ctx(), e),
            }
        }
    }
}

/// A face thumbnail with its name (or role, or suggestion) under it, registered as `<id>:<i>`;
/// true when clicked.
fn face_tile(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, photo: lightcraft_catalog::PhotoId, i: usize, f: &Value, selected: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(FACE, FACE + 16.0), Sense::click());
    register(ui.ctx(), format!("{id}:{i}"), r);
    let pic = Rect::from_min_size(r.min, vec2(FACE, FACE));
    ui.painter().rect_filled(pic, 3.0, t.canvas);
    let rect = f.get("rect").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<f64>>()).unwrap_or_default();
    if let [x0, y0, x1, y1] = rect[..] {
        let ppp = ui.ctx().pixels_per_point();
        let face = lightcraft_geom::Rect { x0, y0, x1, y1 };
        if let Some(job) = app.session.face_job(photo, face, (FACE * ppp).ceil() as usize)
            && let Some(tex) = app.renderer.variant(job)
        {
            ui.painter().image(tex.tex.id(), pic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
    let stroke = if selected {
        Stroke::new(2.0, t.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, t.text_dim)
    } else {
        Stroke::NONE
    };
    ui.painter().rect_stroke(pic, 3.0, stroke, StrokeKind::Outside);
    let caption = match (f.get("person").and_then(Value::as_str), first_role(f)) {
        (Some(n), Some(r)) => format!("{r} · {n}"),
        (Some(n), None) => n.to_string(),
        _ => match f.pointer("/suggestion/name").and_then(Value::as_str) {
            Some(s) => format!("{s}?"),
            None => crate::i18n::tr_format!("Person {n}", n = i + 1),
        },
    };
    let g = crate::widgets::one_line(ui.painter(), &caption, t.font(10.5), t.text_dim, FACE + 8.0);
    ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, pic.bottom() + 2.0), g, t.text_dim);
    resp.on_hover_text(caption).clicked()
}

/// Name the chosen face and give them a role in this shoot; and choose whether the mask finds them
/// by name or by role.
fn naming(app: &mut LightcraftApp, ui: &mut egui::Ui, photo: lightcraft_catalog::PhotoId, f: &Value) {
    let t = Tokens::get(ui.ctx());
    let index = f.get("index").and_then(Value::as_u64).unwrap_or(0);
    let current = f.get("person").and_then(Value::as_str).map(str::to_string);
    let (names, roles) = app.session.people_names();
    ui.add_space(4.0);
    // (one field a row: the panel is narrow, and translated labels are longer)
    let field = (ui.available_width() - 60.0).clamp(80.0, 200.0);
    ui.horizontal(|ui| {
        ui.add_sized(vec2(50.0, 18.0), egui::Label::new(crate::i18n::tr("Name")));
        let mut name = app.ui.people_name.clone();
        TextField::singleline("maskPeople:name", &mut name).hint(crate::i18n::tr("Name")).width(field).show(ui);
        app.ui.people_name = name;
    });
    ui.horizontal(|ui| {
        ui.add_sized(vec2(50.0, 18.0), egui::Label::new(crate::i18n::tr("Role")));
        let mut role = app.ui.people_role.clone();
        TextField::singleline("maskPeople:role", &mut role).hint(crate::i18n::tr("Bride")).width(field).show(ui);
        app.ui.people_role = role;
    });
    // suggestions: names starting like the typed one, a recognised face's suggestion, the usual roles
    ui.horizontal_wrapped(|ui| {
        let typed = app.ui.people_name.trim().to_lowercase();
        let mut chips: Vec<String> = Vec::new();
        if let Some(s) = f.pointer("/suggestion/name").and_then(Value::as_str)
            && current.is_none()
        {
            chips.push(s.to_string());
        }
        chips
            .extend(names.iter().filter(|n| !typed.is_empty() && n.to_lowercase().starts_with(&typed) && n.to_lowercase() != typed).take(5).cloned());
        for (k, n) in chips.into_iter().enumerate() {
            let b = ui.small_button(&n);
            register(ui.ctx(), format!("maskPeople:nameChip:{k}"), b.rect);
            if b.clicked() {
                app.ui.people_name = n;
            }
        }
        let mut role_chips: Vec<String> = ROLES.iter().map(|r| r.to_string()).collect();
        for r in roles {
            if !role_chips.iter().any(|x| x.eq_ignore_ascii_case(&r)) {
                role_chips.push(r);
            }
        }
        for (k, r) in role_chips.into_iter().enumerate() {
            let b = ui.small_button(crate::i18n::tr(&r));
            register(ui.ctx(), format!("maskPeople:roleChip:{k}"), b.rect);
            if b.clicked() {
                app.ui.people_role = r;
            }
        }
    });
    ui.horizontal(|ui| {
        let can = !app.ui.people_name.trim().is_empty();
        let b = ui.add_enabled(can, egui::Button::new(crate::i18n::tr("Save Name")));
        register(ui.ctx(), "maskPeople:save", b.rect);
        if b.clicked() {
            let mut p = json!({"id": photo.0, "face": index, "name": app.ui.people_name.trim()});
            if !app.ui.people_role.trim().is_empty() {
                p["role"] = json!(app.ui.people_role.trim());
            }
            match app.run("people.name", p) {
                Ok(_) => {
                    app.ui.people_by_role = !app.ui.people_role.trim().is_empty();
                    app.toast(ui.ctx(), crate::i18n::tr_format!("Named {name}", name = app.ui.people_name.trim()));
                }
                Err(e) => app.toast_error(ui.ctx(), e),
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if let (Some(name), Some(role)) = (current.as_deref(), first_role(f)) {
            ui.label(egui::RichText::new(crate::i18n::tr("Find on other photos by")).color(t.text_dim));
            let r = ui.selectable_label(!app.ui.people_by_role, name);
            register(ui.ctx(), "maskPeople:byName", r.rect);
            if r.clicked() {
                app.ui.people_by_role = false;
            }
            let r = ui.selectable_label(app.ui.people_by_role, crate::i18n::tr(&role));
            register(ui.ctx(), "maskPeople:byRole", r.rect);
            if r.clicked() {
                app.ui.people_by_role = true;
            }
        }
    });
}

/// The remembered objects, under the mask tiles once the Remembered tile was clicked: one button
/// per object (a new mask finding it on this photo), and Remember Selected Mask As….
pub fn concept_picker(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    if !app.ui.concept_pick {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    let names: Vec<String> = app.session.concept_store().concepts.iter().map(|c| c.name.clone()).collect();
    if names.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::i18n::tr(
                    "Nothing remembered yet. Select a thing with any mask (an Object click, a brush, or the clothes of a person), then Remember Selected Mask As….",
                ))
                .color(t.text_dim),
            )
            .wrap(),
        );
    }
    ui.horizontal_wrapped(|ui| {
        for (k, n) in names.iter().enumerate() {
            let b = ui.button(n);
            register(ui.ctx(), format!("maskConcept:{k}"), b.rect);
            if b.clicked() {
                app.ui.concept_pick = false;
                let op = app.ui.mask_pick_op.take();
                super::masking::add_auto(app, ui.ctx(), json!({"kind": "concept", "concept": n, "name": n}), op.as_deref());
            }
        }
    });
    let has_mask = app.session.active_mask.is_some();
    let b = ui.add_enabled(has_mask, egui::Button::new(crate::i18n::tr("Remember Selected Mask As…")));
    register(ui.ctx(), "maskConcept:teach", b.rect);
    if b.clicked() {
        super::dialogs::prompt(app, "Remember as", "Name, e.g. Wedding dress", "", "concepts.teach", json!({}), "name");
    }
}

/// The context menu of a person's card in the People view: rename (merge), forget.
pub fn person_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, name: &str) {
    if ui.button(crate::i18n::tr("Rename or Merge…")).clicked() {
        super::dialogs::prompt(app, "Rename Person", "New name (an existing name merges them)", name, "people.rename", json!({"from": name}), "to");
        ui.close();
    }
    forget_menu(app, ui, name);
}

/// Forget This Person: delete how LightCraft recognises them (and, if asked, their name on photos).
pub fn forget_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, name: &str) {
    ui.menu_button(crate::i18n::tr("Forget This Person"), |ui| {
        ui.label(crate::i18n::tr("Deletes how LightCraft recognises them (biometric data) and their roles."));
        if ui.button(crate::i18n::tr("Forget, Keep Their Name on Photos")).clicked() {
            let r = app.run("people.forget", json!({"person": name}));
            app.toast(ui.ctx(), r.map(|_| crate::i18n::tr_format!("Forgot {name}", name = name)).unwrap_or_else(|e| e));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Forget and Remove Their Name")).clicked() {
            let r = app.run("people.forget", json!({"person": name, "names": true}));
            app.toast(ui.ctx(), r.map(|_| crate::i18n::tr_format!("Forgot {name}", name = name)).unwrap_or_else(|e| e));
            ui.close();
        }
    });
}

/// The Roles window: who has which role in the active photo's shoot (folder), adding and removing
/// roles, and forgetting all face data.
pub fn roles_window(app: &mut LightcraftApp, ctx: &egui::Context) {
    if !app.ui.roles_window {
        return;
    }
    let mut open = true;
    let t = Tokens::get(ctx);
    egui::Window::new(crate::i18n::tr("People and Roles")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
        register(ui.ctx(), "rolesWindow", ui.max_rect());
        let here = app.session.execute("people.roles", &json!({})).ok();
        let list: Vec<Value> = here.as_ref().and_then(|h| h.get("here")).and_then(Value::as_array).cloned().unwrap_or_default();
        ui.label(egui::RichText::new(crate::i18n::tr("In this photo's folder")).color(t.text_dim));
        if list.is_empty() {
            ui.label(crate::i18n::tr("No roles yet."));
        }
        for (k, r) in list.iter().enumerate() {
            let (role, person) = (r["role"].as_str().unwrap_or_default().to_string(), r["person"].as_str().unwrap_or_default().to_string());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::i18n::tr(&role)).strong());
                ui.label(&person);
                let b = ui.small_button("×");
                register(ui.ctx(), format!("rolesWindow:remove:{k}"), b.rect);
                if b.on_hover_text(crate::i18n::tr("Remove this role here")).clicked() {
                    let _ = app.run("people.clearRole", json!({"role": role, "scope": "folder"}));
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            let mut role = app.ui.people_role.clone();
            TextField::singleline("rolesWindow:role", &mut role).hint(crate::i18n::tr("Bride")).width(110.0).show(ui);
            app.ui.people_role = role;
            let mut name = app.ui.people_name.clone();
            TextField::singleline("rolesWindow:person", &mut name).hint(crate::i18n::tr("Name")).width(130.0).show(ui);
            app.ui.people_name = name;
            let can = !app.ui.people_role.trim().is_empty() && !app.ui.people_name.trim().is_empty();
            let b = ui.add_enabled(can, egui::Button::new(crate::i18n::tr("Set Role")));
            register(ui.ctx(), "rolesWindow:set", b.rect);
            if b.clicked() {
                let r = app.run("people.setRole", json!({"role": app.ui.people_role.trim(), "person": app.ui.people_name.trim()}));
                if let Err(e) = r {
                    app.toast(ui.ctx(), e);
                }
            }
        });
        ui.separator();
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::i18n::tr(
                    "Face data stays on this computer. It is biometric data of your clients: forget it when you no longer need it.",
                ))
                .color(t.text_dim),
            )
            .wrap(),
        );
        let label = if app.ui.forget_all_armed { "Click Again to Forget All Face Data" } else { "Forget All Face Data…" };
        let b = ui.button(crate::i18n::tr(label));
        register(ui.ctx(), "rolesWindow:forgetAll", b.rect);
        if b.clicked() {
            if app.ui.forget_all_armed {
                app.ui.forget_all_armed = false;
                let r = app.run("people.forgetAll", json!({}));
                app.toast(ui.ctx(), r.map(|_| crate::i18n::tr("All face data was deleted.").to_string()).unwrap_or_else(|e| e));
            } else {
                app.ui.forget_all_armed = true;
            }
        }
    });
    if !open {
        app.ui.roles_window = false;
        app.ui.forget_all_armed = false;
    }
}
