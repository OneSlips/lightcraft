//! The lens database without setup (client note 6): on first run the app fetches lensfun's database by itself
//! (`lightcraft_engine::lensfun_download`, an activity row while it runs), and whenever the database in use changes
//! (it arrived, a newer pin replaced it) or Enable Lens Corrections is ticked on a photo without a profile, the
//! library's raws that have none are looked up again on a worker thread (`lightcraft_engine::cmd::lens_refresh`).

use lightcraft_engine::cmd::{lens_refresh, recheck};
use lightcraft_engine::{lens_profiles, lensfun_download};

use crate::LightcraftApp;

/// The activity stack's label while files are read for their lens.
pub const LABEL: &str = "Looking up lens profiles";

/// Called every frame.
pub fn tick(app: &mut LightcraftApp) {
    start_download(app);
    refresh(app);
}

/// Once per run, when the host asked for the app's own copy and it is missing (or from an older pin).
fn start_download(app: &mut LightcraftApp) {
    if app.ui.lens_download_checked || !app.session.lens_auto_download {
        return;
    }
    app.ui.lens_download_checked = true;
    // a folder of the user's own (LIGHTCRAFT_LENSFUN_DIR) is never replaced
    if std::env::var_os(lens_profiles::ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let Some(dir) = lensfun_download::default_dir() else { return };
    if lensfun_download::installed(&dir) {
        return;
    }
    let activity = app.session.activity.clone();
    if let Err(e) = app.session.lensfun.start(&dir, lensfun_download::mirrors(), lensfun_download::FILES, &activity) {
        log::warn!("lens database: {e}");
    }
}

/// Look the library's raws up again when the database changed, the library changed, or photos asked for it.
fn refresh(app: &mut LightcraftApp) {
    let key = (app.session.library_generation(), lens_profiles::generation());
    let asked = std::mem::take(&mut app.ui.lens_lookup_ids);
    if app.ui.lens_refreshed == Some(key) && asked.is_empty() {
        return;
    }
    // (a lookup already under way finishes first; what was asked meanwhile waits for the next frame)
    if app.tasks.is_running(LABEL) {
        app.ui.lens_lookup_ids.extend(asked);
        return;
    }
    let ids = (app.ui.lens_refreshed == Some(key)).then_some(asked);
    app.ui.lens_refreshed = Some(key);
    let Some(probe) = recheck::file_probe(&app.session) else { return };
    let Some(db) = lens_profiles::database() else { return };
    let listed = lens_refresh::missing(&app.session.catalog, &db, ids.as_deref());
    if listed.is_empty() {
        return;
    }
    let library = key.0;
    let work = move || lens_refresh::probe(&probe, listed);
    let done = move |app: &mut LightcraftApp, ctx: &egui::Context, results| {
        if app.session.library_generation() != library {
            return;
        }
        match lens_refresh::apply(&mut app.session, results) {
            Ok(found) if found.len() == 1 => app.toast(ctx, crate::i18n::tr("1 photo got its lens profile")),
            Ok(found) if found.len() > 1 => app.toast(ctx, crate::i18n::tr_format!("{n} photos got their lens profile", n = found.len())),
            Ok(_) => {}
            Err(e) => log::warn!("lens profile lookup: {e}"),
        }
    };
    if let Err(e) = crate::tasks::spawn(app, LABEL, Some("lensLookup"), work, done) {
        log::warn!("{e}");
    }
}

/// The database's name for a lens, remembered per lens and database (the panel asks every frame; a search scores
/// every lens in the database).
fn database_name(camera: &str, lens: &str) -> Option<String> {
    type Key = (String, String, u64);
    thread_local! {
        static LAST: std::cell::RefCell<Option<(Key, Option<String>)>> = const { std::cell::RefCell::new(None) };
    }
    let key = (camera.to_string(), lens.to_string(), lens_profiles::generation());
    LAST.with(|last| {
        if let Some((k, name)) = last.borrow().as_ref()
            && *k == key
        {
            return name.clone();
        }
        let name = lens_profiles::database().and_then(|db| lens_profiles::profile_name(&db, camera, lens));
        *last.borrow_mut() = Some((key, name.clone()));
        name
    })
}

/// What the Optics panel says about photo `p`'s lens profile: which one is loaded and from where, or why there is
/// none.
pub fn readout(app: &LightcraftApp, p: &lightcraft_catalog::Photo) -> String {
    use lightcraft_develop::LensSource;
    match p.embedded_lens.map(|l| l.source) {
        Some(LensSource::File) => crate::i18n::tr("Profile: from the file (the camera's own correction)").to_string(),
        Some(LensSource::Database) => {
            let name = database_name(&p.meta.camera, &p.meta.lens).unwrap_or_else(|| p.meta.lens.trim().to_string());
            crate::i18n::tr_format!("Profile: {} (lens database)", name)
        }
        None if app.session.lensfun.running() => crate::i18n::tr("Downloading the lens database…").to_string(),
        None => match lens_refresh::no_profile_reason(p) {
            Some(r) if p.meta.lens.trim().is_empty() => crate::i18n::tr(&r).to_string(),
            Some(_) => crate::i18n::tr_format!("No lens profile found for {}.", p.meta.lens.trim()),
            None => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::{LightcraftApp, Services};
    use lightcraft_develop::{EmbeddedLens, LensSource};

    /// The Optics panel says which profile is loaded and from where, or why there is none.
    #[test]
    fn the_optics_readout_names_the_profile_or_the_lens_without_one() {
        let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
        let id = app.session.active().expect("a photo");
        let mut p = (**app.session.catalog.photo(id).expect("the photo")).clone();
        p.meta.lens = "EF35mm f/1.4L II USM".into();
        p.embedded_lens = None;
        assert_eq!(super::readout(&app, &p), "No lens profile found for EF35mm f/1.4L II USM.");
        p.meta.lens.clear();
        assert_eq!(super::readout(&app, &p), "This file names no lens.");
        p.embedded_lens = Some(EmbeddedLens::default());
        assert_eq!(super::readout(&app, &p), "Profile: from the file (the camera's own correction)");
        p.meta.lens = "EF70-200mm f/2.8L IS II USM".into();
        p.embedded_lens = Some(EmbeddedLens { source: LensSource::Database, ..Default::default() });
        // the database's name for it, or the file's own without a database (LIGHTCRAFT_LENSFUN_DIR may be set)
        let r = super::readout(&app, &p);
        assert!(r.starts_with("Profile: ") && r.contains("70-200mm f/2.8L IS II USM") && r.ends_with(" (lens database)"), "{r}");
    }
}
