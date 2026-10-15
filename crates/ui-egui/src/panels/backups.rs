//! Settings ▸ Backups: back up the library when LightCraft exits (every time, once a day, once a
//! week, never), where to, how many to keep, check each backup; Back Up Library Now; the backups
//! there are, each with Restore….

use egui::RichText;
use serde_json::{Value, json};

use super::settings::{check, choices, heading, hint, row};
use crate::LightcraftApp;
use crate::i18n::{tr, tr_format};
use crate::theme::Tokens;
use crate::widgets::register;

/// Backups kept, as offered (0 = all).
const KEEP: [u32; 6] = [3, 5, 10, 20, 50, 0];

fn confirm_id() -> egui::Id {
    egui::Id::new("backupRestoreConfirm")
}

/// `bytes` as people read them.
fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{mb:.1} MB") }
}

pub fn settings_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let v = app.session.execute("library.backups", &json!({})).unwrap_or(Value::Null);
    let st = &v["settings"];
    heading(ui, t, "Library Backups");
    if st["available"] != true || app.session.library.as_ref().is_none_or(|l| !l.on_disk) {
        hint(ui, t, "Backups need a library on disk and LightCraft's settings folder.");
        return;
    }
    hint(
        ui,
        t,
        "A backup is a copy of the catalog (your edits, ratings, keywords, albums), the library's settings and LightCraft's settings with your presets and profiles. Never the photos themselves or the thumbnail cache: it takes a moment.",
    );
    let mut freq = st["frequency"].as_str().unwrap_or("daily").to_string();
    let freq_now = freq.clone();
    row(ui, t, "Back up when LightCraft exits", |ui| {
        let mut f = freq.as_str();
        choices(
            ui,
            "backupFrequency",
            &[("everyExit", "Every time"), ("daily", "Once a day"), ("weekly", "Once a week"), ("never", "Never")],
            &mut f,
        );
        freq = f.to_string();
    });
    if freq != freq_now {
        let _ = app.run("library.backupSettings", json!({"frequency": freq}));
    }
    let folder = st["folder"].as_str().unwrap_or_default().to_string();
    row(ui, t, "Back up to", |ui| {
        ui.add(egui::Label::new(RichText::new(&folder).color(t.text)).wrap());
    });
    row(ui, t, "", |ui| {
        let r = ui.add_enabled(app.services.pick_folder.is_some(), egui::Button::new(tr("Choose Folder…")));
        register(ui.ctx(), "button:backupChooseFolder", r.rect);
        if r.clicked()
            && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
        {
            let _ = app.run("library.backupSettings", json!({"dir": dir}));
        }
        if st["dir"].is_string() {
            let r = ui.button(tr("Use the Library's Backups Folder"));
            register(ui.ctx(), "button:backupDefaultFolder", r.rect);
            if r.clicked() {
                let _ = app.run("library.backupSettings", json!({"dir": ""}));
            }
        }
    });
    hint(ui, t, "Another drive or a network folder keeps your work safe if this disk fails. Each library gets a folder of its own there.");
    let mut keep = st["keep"].as_u64().unwrap_or(10) as u32;
    let keep_now = keep;
    row(ui, t, "Keep", |ui| {
        let labels: Vec<String> = KEEP.iter().map(|k| if *k == 0 { tr("All").to_string() } else { tr_format!("Last {n}", n = k) }).collect();
        let options: Vec<(u32, &str)> = KEEP.iter().zip(labels.iter()).map(|(k, l)| (*k, l.as_str())).collect();
        choices(ui, "backupKeep", &options, &mut keep);
    });
    if keep != keep_now {
        let _ = app.run("library.backupSettings", json!({"keep": keep}));
    }
    let mut verify = st["verify"] == true;
    if check(ui, "backupVerify", &mut verify, "Check each backup by opening its catalog") {
        let _ = app.run("library.backupSettings", json!({"verify": verify}));
    }
    ui.add_space(6.0);
    let running = app.session.backup_running();
    row(ui, t, "", |ui| {
        let r = ui.add_enabled(!running, egui::Button::new(tr("Back Up Library Now")));
        register(ui.ctx(), "button:backUpNow", r.rect);
        if r.clicked() {
            let _ = app.run("library.backUpNow", json!({"background": true}));
        }
        if running {
            ui.label(RichText::new(tr("Backing up…")).color(t.text_dim));
        } else if let Some(last) = &app.session.backups.last_result {
            let text = match last["error"].as_str() {
                Some(e) => tr_format!("The last backup failed: {e}", e = e),
                None => tr_format!(
                    "Backed up in {s} s ({size})",
                    s = format!("{:.1}", last["ms"].as_f64().unwrap_or(0.0) / 1000.0),
                    size = size(last["bytes"].as_u64().unwrap_or(0))
                ),
            };
            ui.label(RichText::new(text).color(if last["error"].is_string() { t.caution } else { t.text_dim }));
        }
    });

    heading(ui, t, "Backups of This Library");
    let backups: Vec<Value> = v["backups"].as_array().cloned().unwrap_or_default();
    if backups.is_empty() {
        hint(ui, t, "None yet.");
        return;
    }
    let mut confirm: Option<String> = ui.data(|m| m.get_temp(confirm_id()));
    for b in &backups {
        let path = b["path"].as_str().unwrap_or_default().to_string();
        let name = b["name"].as_str().unwrap_or_default();
        let detail = tr_format!("{n} photos · {size}", n = b["photos"].as_u64().unwrap_or(0), size = size(b["bytes"].as_u64().unwrap_or(0)));
        ui.horizontal(|ui| {
            ui.label(RichText::new(name).color(t.text));
            ui.label(RichText::new(detail).size(11.0).color(t.text_dim));
            let r = ui.small_button(tr("Restore…"));
            register(ui.ctx(), format!("button:backupRestore:{name}"), r.rect);
            if r.clicked() {
                confirm = Some(path.clone());
            }
        });
        if confirm.as_deref() == Some(path.as_str()) {
            egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(tr(
                            "Restore makes a new library folder from this backup, beside the current one, and opens it. Nothing is overwritten: the current library stays as it is.",
                        ))
                        .color(t.text),
                    )
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    let r = ui.button(tr("Restore and Open"));
                    register(ui.ctx(), "button:backupRestoreConfirm", r.rect);
                    if r.clicked() {
                        confirm = None;
                        match app.session.execute("library.restoreBackup", &json!({"path": path})) {
                            Ok(r) => {
                                let lib = r["library"].as_str().unwrap_or_default().to_string();
                                if let Err(e) = app.run("app.openLibrary", json!({"path": lib})) {
                                    app.ui.status = e;
                                }
                            }
                            Err(e) => app.ui.status = e.to_string(),
                        }
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        confirm = None;
                    }
                });
            });
        }
    }
    ui.data_mut(|m| match &confirm {
        Some(c) => {
            m.insert_temp(confirm_id(), c.clone());
        }
        None => m.remove::<String>(confirm_id()),
    });
}
