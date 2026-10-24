//! Previews are made once and used (notes 47–49): the loupe shows a built or prepared preview
//! instead of rendering the photo again, a build skips photos whose previews are there, built
//! previews are on disk when the build says it is done, the import's queue shows one row, and the
//! background preparation follows the user and gives way to a build that was asked for.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::Session;
use crate::media::QuickSource;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-preview-prep-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(p: &Path, seed: u8) {
    png_n(p, usize::from(seed));
}

/// A 600 × 400 PNG whose pixels differ for every `n` (no two are duplicates).
fn png_n(p: &Path, n: usize) {
    let img = lightcraft_raster::Rgba8::from_fn(600, 400, |x, y| [((x + n / 256) % 251) as u8, (y % 241) as u8, (n % 256) as u8, 255]);
    let b =
        crate::export::encode_image(&img, &crate::export::ExportOptions { format: crate::export::ExportFormat::Png, ..Default::default() }).unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

/// A library on disk with `n` photos (600 × 400 PNGs) imported, in name order.
fn library(tag: &str, n: usize) -> (Session, PathBuf, Vec<PhotoId>) {
    let root = temp_dir(tag);
    let paths: Vec<String> = (0..n)
        .map(|i| {
            let p = root.join("shoot").join(format!("p{i:02}.png"));
            if n > 12 {
                png_n(&p, i)
            } else {
                png(&p, (i * 20) as u8)
            }
            p.to_string_lossy().to_string()
        })
        .collect();
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    s.execute("library.import", &json!({"paths": paths})).unwrap();
    let mut ids: Vec<(String, PhotoId)> = s.catalog.photos().map(|p| (p.file_name.clone(), p.id)).collect();
    ids.sort();
    (s, root, ids.into_iter().map(|(_, id)| id).collect())
}

fn ids_json(ids: &[PhotoId]) -> Vec<u64> {
    ids.iter().map(|i| i.0).collect()
}

/// Note 47: a built preview at least as big as the loupe's size is what the loupe shows (fitted
/// down, no render); a smaller one is not (the photo renders).
#[test]
fn the_loupe_uses_a_built_preview_that_is_big_enough() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    s.execute("library.buildPreviews", &json!({"ids": [id.0], "edge": 512, "wait": true})).unwrap();
    let r = s.loupe_job(id, 300, 300, true).unwrap().run();
    assert_eq!(r.quick, Some(QuickSource::Cached), "taken from the built preview");
    let img = r.rendered.unwrap().image;
    assert_eq!(img.width.max(img.height), 300, "fitted to the loupe's size");
    // bigger than what was built: rendered
    let r = s.loupe_job(id, 900, 900, true).unwrap().run();
    assert_eq!(r.quick, None, "a preview too small for the loupe is not used");
    assert!(r.rendered.is_ok());
    // a draft (a slider held) never comes from the cache
    let r = s.loupe_job(id, 300, 300, true).unwrap().draft().run();
    assert_eq!(r.quick, None);
}

/// Shrinking a cached view keeps its colour (an average of the pixels it covers) and its aspect,
/// and refuses to enlarge.
#[test]
fn a_cached_view_is_fitted_down_never_up() {
    let img = lightcraft_raster::Rgba8::from_fn(400, 200, |x, _| if x % 2 == 0 { [200, 100, 0, 255] } else { [100, 50, 0, 255] });
    let f = crate::media::fit_cached(&img, 100, 100).unwrap();
    assert_eq!((f.width, f.height), (100, 50));
    assert!(f.data.iter().all(|p| p[0] == 150 && p[1] == 75), "averaged");
    assert!(crate::media::fit_cached(&img, 800, 800).is_none(), "never enlarged");
    assert_eq!(crate::media::fit_cached(&img, 401, 999).map(|f| f.width), Some(400), "a pixel short is the same size");
    assert!(crate::media::fit_cached(&img, 0, 10).is_none());
}

/// Note 47: the rendered previews in memory follow the memory budget (they were capped at
/// 128 MB, about 84 loupe renders, whatever the budget).
#[test]
fn the_memory_for_rendered_previews_grows_with_the_budget() {
    let share = crate::memory::cache_share(20 << 30);
    assert!(crate::media::rendered_budget(share) >= 2 << 30, "a 20 GB budget keeps gigabytes of previews");
    assert_eq!(crate::media::rendered_budget(256 << 20), 128 << 20, "a small budget keeps the old floor where it can");
    assert!(crate::media::rendered_budget(64 << 20) <= 32 << 20, "and never more than the share allows");
}

/// Note 47: the disk cache's default room grows with the library, so a wedding's previews are not
/// pruned while they are built (the fixed 2 GB held about 2 000 standard previews).
#[test]
fn the_disk_cache_default_grows_with_the_library() {
    assert_eq!(crate::media::default_disk_cache_bytes(10), crate::media::DISK_CACHE_BYTES);
    assert!(crate::media::default_disk_cache_bytes(4000) >= 4000 * (1 << 20));
    let (mut s, root, ids) = library("diskroom", 2);
    // Automatic: a share of the cache drive's room (note 31 of 2026-10-24), here whatever this drive has
    let free = lightcraft_sysmem::free_disk_space(&root);
    let thumbs = s.media.rendered.disk().unwrap().dir().to_path_buf();
    assert_eq!(s.cache_bytes(), crate::media::automatic_disk_cache_bytes(2, free, s.media.rendered.disk().unwrap().known_size().unwrap_or(0)));
    assert_eq!(s.cache_bytes(), s.cache_bytes_in(&thumbs));
    assert_eq!(s.automatic_cache_bytes(), s.cache_bytes());
    s.cache_mb = 700;
    assert_eq!(s.cache_bytes(), 700 << 20, "a size the user chose is kept");
    assert_eq!(ids.len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

/// Notes 47 and 49: a build skips photos whose previews are there (built before, or by the loupe),
/// renders those whose preview is too small, and its previews are on disk when it is done (they
/// were left to a writer that skips writes when it falls behind).
#[test]
fn a_build_skips_what_is_there_and_leaves_it_on_disk() {
    let (mut s, root, ids) = library("skip", 4);
    let r = s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 300, "wait": true})).unwrap();
    assert_eq!((r["done"].as_u64(), r["skipped"].as_u64(), r["failed"].as_u64()), (Some(4), Some(0), Some(0)), "{r}");
    // on disk now (no waiting for a writer), at the size built
    for id in &ids {
        let p = s.catalog.photo(*id).unwrap().clone();
        let key = s.view_key(&p, true);
        let disk = s.media.rendered.disk().expect("a library has a disk cache");
        let (w, h) = disk.dims(key).expect("the view preview is on disk");
        assert_eq!(w.max(h), 300);
    }
    // again: nothing is rendered
    let r = s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 300, "wait": true})).unwrap();
    assert_eq!((r["done"].as_u64(), r["skipped"].as_u64()), (Some(4), Some(4)), "{r}");
    // forced, or bigger: rendered
    let r = s.execute("library.buildPreviews", &json!({"ids": [ids[0].0], "edge": 300, "wait": true, "force": true})).unwrap();
    assert_eq!(r["skipped"], 0, "{r}");
    let r = s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 500, "wait": true})).unwrap();
    assert_eq!(r["skipped"], 0, "too small for what is asked: built again {r}");
    // a library opened again finds them on disk (memory is empty)
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    let r = s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 500, "wait": true})).unwrap();
    assert_eq!(r["skipped"], 4, "{r}");
    let r = s.loupe_job(ids[1], 400, 400, true).unwrap().run();
    assert_eq!(r.quick, Some(QuickSource::Cached), "the loupe uses the preview built in the last session");
    let _ = std::fs::remove_dir_all(root);
}

/// Note 47: standard previews are built at least as big as the loupe on this screen asks for.
#[test]
fn standard_previews_are_as_big_as_the_loupe() {
    let mut s = Session::with_demo();
    assert_eq!(crate::cmd::previews::standard_edge(&s), crate::cmd::previews::STANDARD_EDGE);
    s.loupe_edge = 3000;
    assert_eq!(crate::cmd::previews::standard_edge(&s), 3000);
    s.loupe_edge = 1200;
    assert_eq!(crate::cmd::previews::standard_edge(&s), crate::cmd::previews::STANDARD_EDGE);
}

/// Note 48: a background build goes from where the user is, in the direction they step, and
/// follows them when they jump.
#[test]
fn the_background_build_starts_where_the_user_is() {
    use crate::cmd::previews::PreviewBuild;
    let b = PreviewBuild { order: (10..20).collect(), ..Default::default() };
    let mut queue: Vec<Option<u64>> = (10..20).map(Some).collect();
    b.focus_on(PhotoId(14), false);
    let mut took = Vec::new();
    for _ in 0..3 {
        let i = b.next_index(&queue).unwrap();
        took.push(queue[i].take().unwrap());
    }
    assert_eq!(took, vec![14, 15, 16], "forward from the photo on screen");
    b.focus_on(PhotoId(12), true);
    let i = b.next_index(&queue).unwrap();
    assert_eq!(queue[i], Some(12), "jumped back: from there, backwards");
    queue[i] = None;
    assert_eq!(b.next_index(&queue).map(|i| queue[i]), Some(Some(11)));
    // the end reached: the rest the other way
    b.focus_on(PhotoId(19), false);
    for _ in 0..6 {
        let i = b.next_index(&queue).unwrap();
        queue[i] = None;
    }
    assert!(b.next_index(&queue).is_none(), "all taken");
}

/// Note 49: the background preparation gives way to a build that is asked for (no error, no
/// two builds), and is not started again for a view the user stopped it in.
#[test]
fn a_build_asked_for_replaces_the_background_one() {
    let (mut s, root, ids) = library("replace", 3);
    // held back: the photo on screen is being made all along
    crate::prep::foreground();
    s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 300, "background": true})).unwrap();
    let bg = s.preview_build.clone().unwrap();
    assert!(bg.background && bg.running());
    assert!(s.activity.list().iter().any(|t| t.label == "Preparing photos in the background"), "{:?}", s.activity.list());
    // a second background build is refused; one that is asked for replaces it
    assert!(s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "background": true})).is_err());
    let r = s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 300, "wait": true})).unwrap();
    assert_eq!(r["done"], 3, "{r}");
    assert!(bg.cancel.load(Ordering::Relaxed), "the background build was stopped");
    let _ = std::fs::remove_dir_all(root);
}

/// Note 48: the background preparation starts once the user steps through photos in the loupe
/// (not when a library opens), builds the view's previews in one cancellable row, and a stop
/// from that row holds for the view.
#[test]
fn the_background_preparation_starts_when_culling_and_can_be_stopped() {
    let (mut s, root, ids) = library("prep", 4);
    let mode = crate::prep::Mode::Library;
    s.prep_tick(mode, None, false);
    s.prep_tick(mode, Some(ids[0]), false);
    assert!(s.preview_build.is_none(), "opening and looking at one photo starts nothing");
    s.prep_tick(mode, Some(ids[1]), false);
    s.prep_tick(mode, Some(ids[2]), false);
    let b = s.preview_build.clone().expect("culling: the view is prepared");
    assert!(b.background);
    assert_eq!(b.total, 4);
    for _ in 0..600 {
        s.prep_tick(mode, Some(ids[2]), false);
        if !b.running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!b.running(), "built");
    assert_eq!(b.done.load(Ordering::Relaxed), 4);
    s.prep_tick(mode, Some(ids[3]), false);
    assert!(same_build(&s, &b), "not started again for the same view");
    // switched off: nothing starts in a new view either
    s.execute("prefs.setBackgroundPrep", &json!({"settings": {"library": {"previews": false}}})).unwrap();
    let off = s.execute("prefs.backgroundPrep", &json!({})).unwrap();
    assert_eq!(off["settings"]["library"]["previews"], false, "{off}");
    assert_eq!(off["settings"]["culling"]["previews"], true, "each mode has its own");
    let _ = std::fs::remove_dir_all(root);
}

fn same_build(s: &Session, b: &std::sync::Arc<crate::cmd::previews::PreviewBuild>) -> bool {
    s.preview_build.as_ref().is_some_and(|p| std::sync::Arc::ptr_eq(p, b))
}

/// Note 48: a library keeps its own background settings (a timelapse library prepares nothing),
/// saved with it; the app-wide ones apply elsewhere.
#[test]
fn a_library_can_keep_its_own_background_settings() {
    let (mut s, root, _ids) = library("own", 1);
    let r = s.execute("prefs.setBackgroundPrep", &json!({"library": true, "settings": {"culling": {"previews": false, "ahead": 0}}})).unwrap();
    assert_eq!(r["own"], true, "{r}");
    assert_eq!(r["settings"]["culling"]["ahead"], 0);
    assert_eq!(r["app"]["culling"]["ahead"], 5, "the app-wide ones are unchanged");
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(root.join("lib"), false).unwrap();
    assert_eq!(s.prep_settings().culling.ahead, 0, "kept with the library");
    assert_eq!(s.prep_settings().ahead(crate::prep::Mode::Culling), 0);
    s.execute("prefs.setBackgroundPrep", &json!({"reset": true})).unwrap();
    assert_eq!(s.prep_settings().culling.ahead, 5, "back to the app-wide settings");
    assert!(s.execute("prefs.setBackgroundPrep", &json!({"settings": 3})).is_err());
    let _ = std::fs::remove_dir_all(root);
}

/// Note 49: an import's queue shows one row (the overall progress in per cent, the stage's photos
/// in its detail), its preview build has no row of its own, and leftovers of an earlier queue are
/// said as such.
#[test]
fn the_import_queue_shows_one_row_and_names_the_leftovers() {
    let (mut s, root, ids) = library("finish", 3);
    // an earlier queue with photo 0, held at its previews by a build already running
    s.preview_build = Some(std::sync::Arc::new(crate::cmd::previews::PreviewBuild { total: 1, ..Default::default() }));
    s.finish_import(crate::finishing::FinishPlan { ids: vec![ids[0].0], previews: "standard".into(), started: String::new() });
    // the next import brings photos 1 and 2: the queue takes photo 0 along, and says so
    s.finish_import(crate::finishing::FinishPlan { ids: vec![ids[1].0, ids[2].0], previews: "standard".into(), started: String::new() });
    let f = s.finishing.as_ref().unwrap();
    assert_eq!(f.earlier, 1);
    assert_eq!(f.plan.ids.len(), 3);
    assert!(f.detail().contains("2 new + 1 from earlier"), "{}", f.detail());
    s.preview_build = None;
    let mut rows = Vec::new();
    for _ in 0..1000 {
        s.finishing_tick();
        rows.extend(s.activity.list());
        if s.finishing.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.finishing.is_none(), "done");
    assert!(rows.iter().all(|t| t.kind != "previews"), "the build had no row of its own: {rows:?}");
    let finishing: Vec<_> = rows.iter().filter(|t| t.kind == "finishing").collect();
    assert!(!finishing.is_empty());
    assert!(finishing.iter().all(|t| t.unit == crate::activity::Unit::Percent && t.total == 100), "{finishing:?}");
    assert!(finishing.iter().any(|t| t.detail.contains("Previews") && t.detail.contains("of 3")), "{finishing:?}");
    let _ = std::fs::remove_dir_all(root);
}

/// Note 47: Build Previews with one photo open (selected) builds every photo in view, as its menu
/// item says ("Visible"); it built only the open photo, so culling after it found nothing built.
#[test]
fn build_previews_with_one_photo_open_builds_the_view() {
    let mut s = Session::with_demo();
    let n = s.visible_cloned().len();
    assert!(n > 2);
    let one = s.visible_cloned()[0];
    s.selection = crate::Selection::single(one);
    let r = s.execute("library.buildPreviews", &json!({"edge": 256, "wait": true})).unwrap();
    assert_eq!(r["total"], n, "{r}");
    let two: Vec<PhotoId> = s.visible_cloned().into_iter().take(2).collect();
    s.selection = crate::Selection { ids: two.clone(), active: two.first().copied() };
    let r = s.execute("library.buildPreviews", &json!({"edge": 256, "wait": true})).unwrap();
    assert_eq!(r["total"], 2, "a choice of photos is built alone {r}");
}

/// The loupe at rest asks for at least the preview level and shrinks the render to the drawn size
/// (a display fit, upstream #2012): a built preview as big as the drawn size is what it shows,
/// fitted to that size, even when it is smaller than the size asked for; smaller than drawn, it renders.
#[test]
fn the_loupe_uses_a_built_preview_as_big_as_the_drawn_size() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    s.execute("library.buildPreviews", &json!({"ids": [id.0], "edge": 512, "wait": true})).unwrap();
    let r = s.loupe_job(id, 2560, 2560, true).unwrap().with_display_fit(300, 300).run();
    assert_eq!(r.quick, Some(QuickSource::Cached), "taken from the built preview");
    let img = r.rendered.unwrap().image;
    assert_eq!(img.width.max(img.height), 300, "fitted to the drawn size");
    let r = s.loupe_job(id, 2560, 2560, true).unwrap().with_display_fit(900, 900).run();
    assert_eq!(r.quick, None, "a preview smaller than drawn is not used");
}

/// Note 31 of 2026-10-24: Automatic was sized from the open library alone (3.2 GB for 544 photos),
/// so a wedding's previews and each round of edits pruned the last ones and reopening the library
/// rendered everything again. Now it is a share of the cache drive's free space.
#[test]
fn the_automatic_preview_cache_is_a_share_of_the_drives_free_space() {
    use crate::media::{AUTO_DISK_CACHE_MIN, automatic_disk_cache_bytes as auto};
    const GB: u64 = 1 << 30;
    // the user's 544-photo library on a drive with 1.5 TB free: far more than 1.5 MB a photo
    assert_eq!(auto(544, Some(1500 * GB), 0), 225 * GB);
    assert!(auto(544, Some(1500 * GB), 0) > crate::media::default_disk_cache_bytes(544) * 50);
    // at least 20 GB where there is room, and what the cache holds counts as room
    assert_eq!(auto(10, Some(60 * GB), 0), AUTO_DISK_CACHE_MIN);
    assert_eq!(auto(10, Some(1000 * GB), 500 * GB), 225 * GB);
    // a nearly full drive: the last 10 GB (or half, on a small one) stay free
    assert_eq!(auto(10, Some(25 * GB), 0), 15 * GB);
    assert_eq!(auto(10, Some(8 * GB), 0), 4 * GB);
    assert_eq!(auto(10, Some(GB), 0), GB, "the old floor where the drive has it");
    assert_eq!(auto(10, Some(0), 0), 0);
    // free space unknown: the minimum, or room for every photo
    assert_eq!(auto(10, None, 0), AUTO_DISK_CACHE_MIN);
    assert_eq!(auto(100_000, None, 0), crate::media::default_disk_cache_bytes(100_000));
}

/// Tick the preparation at `at` until `done` (at most 30 s).
fn tick_until(s: &mut Session, at: PhotoId, done: impl Fn(&Session) -> bool) -> bool {
    for _ in 0..1500 {
        s.prep_tick(crate::prep::Mode::Library, Some(at), false);
        if done(s) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// Whether photo `id`'s standard preview for its current settings is in the cache.
fn has_view(s: &mut Session, id: PhotoId) -> bool {
    let edge = crate::cmd::previews::standard_edge(s);
    let jobs = crate::cmd::previews::jobs_for(s, id, Some(edge));
    crate::cmd::previews::already_built(&jobs)
}

/// Note 11 of 2026-10-24: edits synced to other photos (Auto Sync, Sync, paste, presets) never
/// re-rendered them, so stepping to one showed its old look (or the camera JPEG) for seconds. Now
/// the photos whose settings changed off screen are rendered again in the background once the
/// edits settle, the photo on screen left to the loupe.
#[test]
fn photos_changed_off_screen_are_rendered_again_in_the_background() {
    let (mut s, root, ids) = library("resync", 5);
    // the user is on the first photo and the view was prepared
    for at in [ids[0], ids[1], ids[0]] {
        s.prep_tick(crate::prep::Mode::Library, Some(at), false);
    }
    assert!(tick_until(&mut s, ids[0], |s| s.preview_build.as_ref().is_some_and(|b| !b.running())), "the view is prepared");
    assert!(has_view(&mut s, ids[3]));
    // synced from photo 0: photos 2 and 3 (and 0 itself, on screen) change
    for id in [ids[0], ids[2], ids[3]] {
        let mut d = (*s.catalog.photo(id).unwrap().develop).clone();
        d.light.exposure = 0.8;
        s.set_develop(id, d, "Sync").unwrap();
    }
    assert!(!has_view(&mut s, ids[2]) && !has_view(&mut s, ids[3]));
    let first = s.preview_build.clone();
    assert!(
        tick_until(&mut s, ids[0], |s| s
            .preview_build
            .as_ref()
            .is_some_and(|b| first.as_ref().is_none_or(|f| !std::sync::Arc::ptr_eq(f, b)) && !b.running())),
        "a build for the changed photos ran"
    );
    let b = s.preview_build.clone().unwrap();
    assert!(b.background);
    assert_eq!(b.total, 2, "only the photos changed off screen");
    assert!(has_view(&mut s, ids[2]) && has_view(&mut s, ids[3]), "their previews are made for the new settings");
    let _ = std::fs::remove_dir_all(root);
}

/// Note 30 of 2026-10-24: preparing photos in the background competed with an export. While the
/// user's own work runs, the background build pauses completely and its row says so.
#[test]
fn the_background_build_pauses_while_an_export_runs() {
    let (mut s, root, ids) = library("pause", 3);
    let export = s.activity.start_batch("export", "Exporting", crate::activity::Cancel::Yes);
    assert_eq!(s.activity.user_work(), Some("export"));
    s.execute("library.buildPreviews", &json!({"ids": ids_json(&ids), "edge": 300, "background": true})).unwrap();
    let b = s.preview_build.clone().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert_eq!(b.done.load(Ordering::Relaxed), 0, "nothing prepared while the export runs");
    let row = s.activity.list().into_iter().find(|t| t.label == "Preparing photos in the background").expect("its row");
    assert_eq!(row.detail, "Paused while exporting");
    drop(export);
    assert_eq!(s.activity.user_work(), None);
    for _ in 0..1500 {
        if !b.running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(b.done.load(Ordering::Relaxed), 3, "carried on once the export ended");
    let _ = std::fs::remove_dir_all(root);
}

/// Note 30 of 2026-10-24: in a library the background build prepares the photos around the one on
/// screen, not the whole (already edited) gallery.
#[test]
fn the_background_build_prepares_around_the_photo_on_screen() {
    let (mut s, root, ids) = library("around", 130);
    let at = ids[100];
    for a in [ids[98], ids[99], at] {
        s.prep_tick(crate::prep::Mode::Library, Some(a), false);
    }
    let b = s.preview_build.clone().expect("prepared");
    b.cancel.store(true, Ordering::Relaxed);
    assert!(b.total <= 3 * crate::prep::AROUND && b.total < ids.len(), "{} of {}", b.total, ids.len());
    assert!(b.order.contains(&at.0) && b.order.contains(&ids[129].0) && !b.order.contains(&ids[0].0));
    let _ = std::fs::remove_dir_all(root);
}
