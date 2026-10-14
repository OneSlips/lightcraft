//! Try Smooth Skin and Find Blemishes on real photos, read-only: each file is imported into an
//! in-memory library (nothing is written next to it), analysed with the open MediaPipe models and
//! rendered at 100 % before and after, around each face.
//!
//! `cargo run --release -p lightcraft-engine --features sam --example retouch_probe -- --models <models folder>
//!  --out <folder> [--mode faces|smooth|blemish] [--set amount=35,texture=0,evenTone=25,redness=20] [--area face|body|both]
//!  [--edge 0 (native)] FILES…`
//!
//! - `faces`: lists the faces found (size, eye distance, landmarks) and how much of the photo is skin;
//! - `smooth`: writes `<stem>_f<n>.jpg` (before | after, side by side, at 100 %) for each face;
//! - `blemish`: finds blemishes (heal spots added) and writes `<stem>_b<n>.jpg` the same way.
use std::path::{Path, PathBuf};
use std::time::Instant;

use lightcraft_develop::{DevelopSettings, SkinArea};
use lightcraft_catalog::PhotoId;
use lightcraft_engine::Session;
use lightcraft_raster::Rgba8;
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn save_jpeg(img: &Rgba8, path: &Path) {
    let enc = lightcraft_codecs::encode::EncodeImage::rgba8(img);
    match lightcraft_codecs::encode::encode_jpeg(&enc, 92, lightcraft_codecs::encode::ChromaSubsampling::S444, &Default::default()) {
        Ok(b) => {
            if let Err(e) = std::fs::write(path, b) {
                eprintln!("{}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("{}: {e}", path.display()),
    }
}

/// `a` and `b` side by side with a 6 px gap.
fn side_by_side(a: &Rgba8, b: &Rgba8) -> Rgba8 {
    let (w, h) = (a.width + 6 + b.width, a.height.max(b.height));
    let mut out = Rgba8::filled(w, h, [40, 40, 40, 255]);
    for (img, x0) in [(a, 0), (b, a.width + 6)] {
        for y in 0..img.height {
            for x in 0..img.width {
                out.data[y * w + x0 + x] = img.data[y * img.width + x];
            }
        }
    }
    out
}

fn render(s: &mut Session, id: PhotoId, edge: usize, d: &DevelopSettings) -> Option<Rgba8> {
    let job = s.preview_job(id, edge, edge, true, d)?;
    job.run().rendered.ok().map(|r| r.image)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let models = arg(&args, "--models").map(PathBuf::from).expect("--models <folder>");
    let out = arg(&args, "--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let mode = arg(&args, "--mode").unwrap_or_else(|| "faces".into());
    let edge: usize = arg(&args, "--edge").and_then(|v| v.parse().ok()).unwrap_or(0);
    let area = arg(&args, "--area").and_then(|a| SkinArea::parse(&a)).unwrap_or(SkinArea::Face);
    let set = arg(&args, "--set").unwrap_or_default();
    let mut skip = false;
    let files: Vec<String> = args
        .iter()
        .filter(|a| {
            if skip {
                skip = false;
                return false;
            }
            if a.starts_with("--") {
                skip = true;
                return false;
            }
            true
        })
        .cloned()
        .collect();
    std::fs::create_dir_all(&out).expect("out folder");
    for file in files {
        let stem = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut s = Session::new().with_fs();
        s.segmenter.models_root = Some(models.clone());
        let r = match s.execute("library.import", &json!({"paths": [file]})) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{file}: {e}");
                continue;
            }
        };
        let Some(id) = r["imported"][0].as_u64().map(PhotoId) else {
            eprintln!("{file}: not imported: {r}");
            continue;
        };
        let _ = s.execute("library.select", &json!({"ids": [id.0], "active": id.0}));
        let (pw, ph) = s.catalog.photo(id).map(|p| (p.width as usize, p.height as usize)).unwrap_or((0, 0));
        let long = if edge > 0 { edge } else { pw.max(ph) };
        let t = Instant::now();
        let found = match s.segment_skin(id, area, 0) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{file}: {e}");
                continue;
            }
        };
        let skin = found.seg.as_ref().and_then(|g| g.logits()).map(|l| l.iter().filter(|v| **v > 0.0).count() as f64 / l.len().max(1) as f64).unwrap_or(0.0);
        let faces: Vec<String> = found
            .faces
            .iter()
            .map(|f| format!("{{w {:.0}px, eyes {:.0}px, mesh {}, score {:.2}}}", (f.rect[2] - f.rect[0]) as f64 * pw as f64, f.eye_distance() * pw.max(ph) as f64, f.mesh.is_some(), f.score))
            .collect();
        println!("{stem}\t{pw}x{ph}\t{} faces {}\tskin {:.1}%\t{:.1}s", found.faces.len(), faces.join(" "), skin * 100.0, t.elapsed().as_secs_f64());
        if mode == "faces" {
            continue;
        }
        let before_settings = (*s.develop_of(id).unwrap_or_default()).clone();
        let label = match mode.as_str() {
            "blemish" => {
                let r = s.execute("spot.findBlemishes", &json!({"body": area != SkinArea::Face})).unwrap_or_else(|e| json!({"error": e.to_string()}));
                println!("  blemishes: added {} {}", r["added"], r["spots"]);
                "b"
            }
            _ => {
                let mut p = json!({});
                for kv in set.split(',').filter(|kv| !kv.is_empty()) {
                    if let Some((k, v)) = kv.split_once('=')
                        && let Ok(v) = v.parse::<f64>()
                    {
                        p[k] = json!(v);
                    }
                }
                let _ = s.execute("skin.smooth", &json!({}));
                p["area"] = json!(area.name());
                let r = s.execute("skin.set", &p).unwrap_or(Value::Null);
                println!("  smooth skin: {}", r);
                "f"
            }
        };
        let after_settings = (*s.develop_of(id).unwrap_or_default()).clone();
        let t = Instant::now();
        let (Some(before), Some(after)) = (render(&mut s, id, long, &before_settings), render(&mut s, id, long, &after_settings)) else {
            eprintln!("{file}: render failed");
            continue;
        };
        println!("  rendered {}x{} twice in {:.1}s", after.width, after.height, t.elapsed().as_secs_f64());
        for (i, f) in found.faces.iter().enumerate().take(6) {
            let (w, h) = (after.width as f64, after.height as f64);
            let (fw, fh) = ((f.rect[2] - f.rect[0]) as f64 * w, (f.rect[3] - f.rect[1]) as f64 * h);
            if fw < 120.0 {
                continue;
            }
            let (cx, cy) = ((f.rect[0] + f.rect[2]) as f64 / 2.0 * w, (f.rect[1] + f.rect[3]) as f64 / 2.0 * h);
            let side = (fw.max(fh) * 1.5).min(1400.0);
            let x0 = (cx - side / 2.0).clamp(0.0, (w - side).max(0.0)) as usize;
            let y0 = (cy - side / 2.0 + 0.1 * side).clamp(0.0, (h - side).max(0.0)) as usize;
            let side = side as usize;
            let pair = side_by_side(&before.crop(x0, y0, side, side), &after.crop(x0, y0, side, side));
            let path = out.join(format!("{stem}_{label}{i}.jpg"));
            save_jpeg(&pair, &path);
            println!("  wrote {}", path.display());
        }
    }
}
