//! Evaluate smart masks on real photos (a developer tool, not part of the app): runs a JSON-lines
//! script of commands on a library, waiting for the AI models, and renders before / after /
//! mask overlays as PPM images.
//!
//! `cargo run --release -p lightcraft-engine --features sam --example smart_eval -- <library dir> <models dir> <out dir> <script.jsonl> [photos…]`
//!
//! Script lines: `{"cmd": "...", "params": {...}}` (any command), `{"cmd": "eval.select",
//! "params": {"file": "substring"}}` (make that photo active), `{"cmd": "eval.render", "params":
//! {"name": "nd-060-dress", "edge": 1200}}` (writes `<name>-before.ppm`, `-after.ppm` and
//! `-mask<N>.ppm` for each mask).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use lightcraft_engine::Session;
use serde_json::{Value, json};

fn write_ppm(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    write!(f, "P6\n{w} {h}\n255\n").unwrap();
    f.write_all(rgb).unwrap();
}

fn render(s: &mut Session, id: lightcraft_catalog::PhotoId, d: &lightcraft_develop::DevelopSettings, edge: usize) -> (usize, usize, Vec<u8>) {
    let job = s.preview_job(id, edge, edge, false, d).expect("render job");
    let r = job.run().rendered.expect("rendered");
    let img = r.image;
    (img.width, img.height, img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect())
}

struct Log;
impl log::Log for Log {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record) {
        if r.target().starts_with("lightcraft_engine::segment") || r.target().starts_with("lightcraft_segment") {
            println!("  log: {}", r.args());
        }
    }
    fn flush(&self) {}
}

fn main() {
    let _ = log::set_logger(&Log);
    log::set_max_level(log::LevelFilter::Info);
    let a: Vec<String> = std::env::args().collect();
    let (lib, models, out, script) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), PathBuf::from(&a[3]), PathBuf::from(&a[4]));
    std::fs::create_dir_all(&out).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).expect("library");
    s.segmenter.models_root = Some(models);
    s.segmenter.background = false;
    let photos: Vec<String> = a[5..].to_vec();
    if !photos.is_empty() {
        let r = lightcraft_engine::import::import(&mut s, &photos, lightcraft_engine::import::ImportMode::Add).expect("import");
        println!("imported {}", r.imported.len());
    }
    let lines = std::io::BufReader::new(std::fs::File::open(&script).unwrap());
    for line in lines.lines() {
        let line = line.unwrap();
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let v: Value = serde_json::from_str(line).expect("script line");
        let cmd = v["cmd"].as_str().unwrap_or_default();
        let p = v.get("params").cloned().unwrap_or(json!({}));
        let t = std::time::Instant::now();
        match cmd {
            "eval.select" => {
                let want = p["file"].as_str().unwrap_or_default().to_lowercase();
                let id = s.catalog.photos().find(|ph| ph.file_name.to_lowercase().contains(&want)).map(|ph| ph.id);
                match id {
                    Some(id) => {
                        s.execute("library.select", &json!({"ids": [id.0], "active": id.0})).unwrap();
                        println!("select {want} -> {}", id.0);
                    }
                    None => println!("select {want}: NOT FOUND"),
                }
            }
            "eval.selectAll" => {
                let want = p["file"].as_str().unwrap_or_default().to_lowercase();
                let ids: Vec<u64> = s.catalog.photos().filter(|ph| ph.file_name.to_lowercase().contains(&want)).map(|ph| ph.id.0).collect();
                for ph in s.catalog.photos().filter(|ph| ids.contains(&ph.id.0)) {
                    println!("photo {} {}", ph.id.0, ph.file_name.get(..8).unwrap_or_default());
                }
                if let Some(first) = ids.first() {
                    s.execute("library.select", &json!({"ids": ids, "active": first})).unwrap();
                }
            }
            "eval.render" => {
                let id = s.active().unwrap();
                let name = p["name"].as_str().unwrap_or("photo").to_string();
                let edge = p["edge"].as_u64().unwrap_or(1200) as usize;
                let d = (*s.develop_of(id).unwrap()).clone();
                let mut before = d.clone();
                before.masks.clear();
                let (w, h, rgb) = render(&mut s, id, &before, edge);
                write_ppm(&out.join(format!("{name}-before.ppm")), w, h, &rgb);
                let (w2, h2, rgb2) = render(&mut s, id, &d, edge);
                write_ppm(&out.join(format!("{name}-after.ppm")), w2, h2, &rgb2);
                // each mask's alpha over the photo, in red
                let frame = lightcraft_pipeline::geometry::Frame::new(w, h, &lightcraft_develop::DevelopSettings::default(), false);
                let lin = |c: u8| {
                    let c = f32::from(c) / 255.0;
                    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
                };
                let img = lightcraft_raster::Rgb32f::from_fn(w, h, |x, y| {
                    let i = (y * w + x) * 3;
                    [lin(rgb[i]), lin(rgb[i + 1]), lin(rgb[i + 2])]
                });
                let log_l = lightcraft_raster::Plane::from_fn(w, h, |x, y| {
                    let p = img.data[y * w + x];
                    (0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2]).max(1e-6).log2()
                });
                for m in &d.masks {
                    let mut m = m.clone();
                    m.visible = true;
                    m.adjust.amount = 100.0;
                    let alpha = lightcraft_pipeline::masks::evaluate_one(&m, &frame, w, h, &img, &log_l, 0.0, None);
                    let mut o = rgb.clone();
                    for (k, a) in alpha.data.iter().enumerate() {
                        let a = a.clamp(0.0, 1.0);
                        let i = k * 3;
                        o[i] = (f32::from(o[i]) * (1.0 - 0.55 * a) + 255.0 * 0.55 * a) as u8;
                        o[i + 1] = (f32::from(o[i + 1]) * (1.0 - 0.55 * a)) as u8;
                        o[i + 2] = (f32::from(o[i + 2]) * (1.0 - 0.55 * a)) as u8;
                    }
                    write_ppm(&out.join(format!("{name}-mask{}.ppm", m.id)), w, h, &o);
                }
                println!("render {name} {w}x{h} masks {}", d.masks.len());
            }
            _ => match s.execute(cmd, &p) {
                Ok(r) => {
                    let txt = r.to_string();
                    let short: String = txt.chars().take(200000).collect();
                    println!("{cmd} ({:.1}s) -> {short}", t.elapsed().as_secs_f32());
                }
                Err(e) => println!("{cmd} ({:.1}s) ERROR {e}", t.elapsed().as_secs_f32()),
            },
        }
        for m in s.take_smart_messages() {
            println!("  note: {m}");
        }
    }
    let _ = s.close_library();
}
