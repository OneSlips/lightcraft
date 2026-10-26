//! Note 2 of 2026-10-25 (the editing flow): culling a ~3,000-photo wedding, next/back sometimes froze
//! the window 1–4.6 s before the step happened; one frame spent 1.4 s in the logic pass with nothing
//! named. Here a library of that size on disk is stepped through and flagged as a photographer does
//! it, and every frame is timed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(30);

/// A session on a library on disk (`dir`) holding `n` raws: their camera JPEGs read at once, a raw
/// decode taking `decode`.
pub(crate) fn big_library(dir: &std::path::Path, n: u64, decode: Duration) -> lightcraft_engine::Session {
    let mut s = lightcraft_engine::Session::new();
    s.open_library(dir.join("Lib"), false).unwrap();
    s.media.preview_loader = Some(Arc::new(|_: &str, edge: usize| {
        let w = edge.clamp(1, 96);
        let h = (w * 2 / 3).max(1);
        Some(lightcraft_raster::Rgba8 { width: w, height: h, data: vec![[120, 110, 100, 255]; w * h] })
    }));
    s.media.file_loader = Some(Arc::new(move |_, _, _| {
        std::thread::sleep(decode);
        Ok((lightcraft_raster::Rgb32f::new(48, 32), Default::default()))
    }));
    for id in 1..=n {
        let mut p =
            Photo::new(PhotoId(id), Source::File { path: format!("shoot/r{id:05}.cr3") }, &format!("r{id:05}.cr3"), "CR3", 6000, 4000, "2026-10-24");
        p.kind = lightcraft_catalog::MediaKind::Raw;
        p.develop = Arc::new(p.camera_defaults());
        s.commit("Add", Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    s.persist().unwrap();
    s
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
}

/// Note 2: stepping and flagging through a 3,000-photo library keeps every frame short.
#[test]
fn stepping_and_flagging_a_3000_photo_library_never_freezes_a_frame() {
    let dir = std::env::temp_dir().join(format!("lc-ui-flow-step-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = big_library(&dir, 3000, Duration::from_millis(30));
    let app = LightcraftApp::new(s, Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1600.0, 1000.0], 1.0);
    exec(&mut h, "library.select", json!({"ids": [1500]}));
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    // the first frames after opening are slow by design (fonts, the library): measured from here
    h.app.perf.max_update_ms = 0.0;
    // the disk busy as it is while previews are built and raws read: files written and synced
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let load = {
        let (stop, dir) = (stop.clone(), dir.join("load"));
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = std::fs::create_dir_all(&dir);
            let block = vec![7u8; 4 << 20];
            let mut i = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(mut f) = std::fs::File::create(dir.join(format!("f{}", i % 16))) {
                    let _ = f.write_all(&block);
                    let _ = f.sync_all();
                }
                i += 1;
            }
        })
    };
    h.app.slowest_frame = None;
    h.app.slowest_kept_ms = 30.0;
    let mut worst = (0.0f64, String::new());
    let mut worst_logic = 0.0f64;
    let t0 = Instant::now();
    for i in 0..120u32 {
        let key = if i % 9 == 8 { "ArrowLeft" } else { "ArrowRight" };
        let r = h.request("ui.key", json!({"key": key}), T);
        assert_eq!(r["ok"], true, "{r}");
        worst_logic = worst_logic.max(h.app.perf.logic_ms);
        if i % 3 == 0 {
            let r = h.request("ui.key", json!({"key": "p"}), T);
            assert_eq!(r["ok"], true, "{r}");
            worst_logic = worst_logic.max(h.app.perf.logic_ms);
        }
        // the user's pace: a step every ~250 ms (frames meanwhile)
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(40) {
            h.step();
            worst_logic = worst_logic.max(h.app.perf.logic_ms);
            if h.app.perf.update_ms > worst.0 {
                worst = (h.app.perf.update_ms, format!("frame {} after step {i}", h.frames()));
            }
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = load.join();
    let worst_logic = worst_logic.max(h.app.slowest_frame.as_ref().map_or(0.0, |f| f.logic_ms));
    let persist = h.app.session.execute("library.info", &json!({})).map(|v| v["persistence"].clone()).unwrap_or_default();
    eprintln!("persist: {persist}");
    eprintln!(
        "stepped 120 photos in {:.1} s; max frame {:.0} ms ({}), max logic pass {worst_logic:.0} ms, slowest {:?}",
        t0.elapsed().as_secs_f64(),
        h.app.perf.max_update_ms,
        worst.1,
        h.app.slowest_frame
    );
    let _ = std::fs::remove_dir_all(&dir);
    // what a flag waits for on the UI thread is writing its record, never the drive's sync (72–94 ms
    // here on a busy disk before, seconds on a photographer's machine; 410–775 ms with this test
    // before the fix). A debug build among the whole workspace's tests leaves room for the rest of
    // the pass (renderer polls of 40 ms seen there).
    let logic_limit = if cfg!(debug_assertions) { 80.0 } else { 30.0 };
    assert!(worst_logic < logic_limit, "a logic pass took {worst_logic:.0} ms: {:?}", h.app.slowest_frame);
    let limit = if cfg!(debug_assertions) { 250.0 } else { 100.0 };
    assert!(h.app.perf.max_update_ms < limit, "a frame took {:.0} ms: {:?}", h.app.perf.max_update_ms, h.app.slowest_frame);
}
