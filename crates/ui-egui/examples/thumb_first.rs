//! Time to first pixels of grid thumbnails, through the real app logic, headless (no compositor, so
//! no frame throttling): browse FOLDER as the Import tab does, then step frames in real time and
//! report, for the cells on screen, how long each stayed empty, and the same after jumping a few
//! screens down and to the end (a cull of a big folder scrolls far). Run it twice: the second run
//! reads what the first one kept.
//! `cargo run --release -p lightcraft-ui-egui --example thumb_first -- FOLDER [LIBRARY_DIR]`
use lightcraft_engine::Session;
use lightcraft_ui_egui::{LightcraftApp, Services, headless::Headless};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(folder) = args.next() else {
        eprintln!("usage: thumb_first FOLDER [LIBRARY_DIR]");
        return;
    };
    let mut s = Session::new().with_fs();
    if let Some(dir) = args.next()
        && let Err(e) = s.open_library(&dir, false)
    {
        eprintln!("{dir}: {e}");
        return;
    }
    let t0 = Instant::now();
    let ms = move || t0.elapsed().as_secs_f64() * 1e3;
    let r = s.execute("library.browse", &json!({"path": folder}));
    eprintln!("{:9.1} browse: {}", ms(), r.map(|v| v.to_string()).unwrap_or_else(|e| e.to_string()));
    let mut h = Headless::new(LightcraftApp::new(s, Services::default()), [1600.0, 1000.0], 1.0);
    let req = |h: &mut Headless, m: &str, p: Value| h.request(m, p, Duration::from_secs(60));
    req(&mut h, "engine.execute", json!({"command": "view.photoGrid"}));
    h.app.ui.thumb_size = 220.0;
    let n = h.app.session.visible_cloned().len();
    for (label, at) in [("top", 0usize), ("a few screens down", n / 3), ("the end", n.saturating_sub(1))] {
        if at > 0 {
            let id = h.app.session.visible_cloned().get(at).map(|p| p.0).unwrap_or(1);
            req(&mut h, "engine.execute", json!({"command": "library.select", "params": {"ids": [id]}}));
        }
        let start = ms();
        h.app.thumb_first.reset();
        let mut full_at = None;
        // frames at about 60 a second, as the window would draw them
        while ms() - start < 15_000.0 {
            let frame = Instant::now();
            h.step();
            let s = h.app.thumb_first.summary();
            if full_at.is_none() && s.seen > 0 && s.empty_now == 0 {
                full_at = Some(ms() - start);
            }
            if full_at.is_some_and(|f| ms() - start > f + 300.0) {
                break;
            }
            std::thread::sleep(Duration::from_millis(16).saturating_sub(frame.elapsed()));
        }
        let s = h.app.thumb_first.summary();
        eprintln!(
            "{:9.1} [{label}] cells {} · first pixels p50 {:.0} ms, p90 {:.0} ms, max {:.0} ms · every cell filled after {} · empty frames-cells {}",
            ms(),
            s.seen,
            s.p50,
            s.p90,
            s.max,
            full_at.map_or("never".to_string(), |f| format!("{f:.0} ms")),
            s.empty_cell_frames
        );
    }
}
