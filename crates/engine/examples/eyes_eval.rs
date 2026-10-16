//! Eye focus on real photos (a developer tool, not part of the app): runs the eye finder on raws
//! (their camera JPEG) or image files and writes what it saw: one line per photo, an overview with
//! every face's box (green: an eye judged, amber: closed or not visible, red: no eye placed) and a
//! contact sheet of the eye crops.
//!
//! `cargo run --release -p lightcraft-engine --features sam --example eyes_eval -- <models dir> <out dir> <photos…>`
//!
//! `<models dir>` holds `mediapipe-face-landmarker` (with `face_detection_full_range.tflite` beside
//! the bundle for small faces). Nothing is written next to the photos.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lightcraft_catalog::PhotoId;
use lightcraft_engine::eyes::{self, EyeSource, Lids};
use lightcraft_raster::Rgba8;

fn save_jpeg(img: &Rgba8, path: &Path) {
    let jpg = lightcraft_codecs::encode_jpeg(&lightcraft_codecs::EncodeImage::rgba8(img), 88, Default::default(), &Default::default()).unwrap();
    std::fs::write(path, jpg).unwrap();
}

fn rect(img: &mut Rgba8, b: [f32; 4], c: [u8; 4], t: usize) {
    let (w, h) = (img.width as f32, img.height as f32);
    let (x0, y0, x1, y1) =
        ((b[0] * w).max(0.0) as usize, (b[1] * h).max(0.0) as usize, (b[2] * w).min(w - 1.0) as usize, (b[3] * h).min(h - 1.0) as usize);
    for k in 0..t {
        for x in x0..=x1 {
            for y in [y0 + k, y1.saturating_sub(k)] {
                if let Some(p) = img.data.get_mut(y * img.width + x) {
                    *p = c;
                }
            }
        }
        for y in y0..=y1 {
            for x in [x0 + k, x1.saturating_sub(k)] {
                if let Some(p) = img.data.get_mut(y * img.width + x) {
                    *p = c;
                }
            }
        }
    }
}

struct Log;
impl log::Log for Log {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("lightcraft_engine::eyes")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            println!("  {}", r.args());
        }
    }
    fn flush(&self) {}
}

fn out_dir_eye(_: &Rgba8, stem: &str, i: usize, e: &eyes::EyeShot) -> PathBuf {
    PathBuf::from(std::env::var("LC_EYES_DUMP").unwrap()).join(format!("{stem}-{i:02}-{:?}.jpg", e.lids))
}

fn main() {
    log::set_logger(&Log).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (models, out) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    std::fs::create_dir_all(&out).unwrap();
    let model = lightcraft_segment::people::FaceModel::load(&models.join("mediapipe-face-landmarker")).unwrap();
    println!("full-range detector: {}", model.has_far());
    let people = lightcraft_segment::people::PeopleModel::load(&models.join("mediapipe-selfie-multiclass")).ok();
    let loader = lightcraft_engine::files::fs_preview_loader();
    // `@list.txt`: one photo per line
    let files: Vec<String> = args[2..]
        .iter()
        .flat_map(|a| match a.strip_prefix('@') {
            Some(list) => std::fs::read_to_string(list).unwrap().lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(),
            None => vec![a.clone()],
        })
        .collect();
    // LC_EYES_NORM=<dir>: every eye at the same size (2.4 eye widths across), 8 × 8 to a sheet,
    // numbered in `eyes.tsv`, for judging by eye
    let norm = std::env::var("LC_EYES_NORM").ok().map(PathBuf::from);
    let mut tiles: Vec<Rgba8> = Vec::new();
    // and each face (1.4× its box), for judging whether it is one at all
    let mut faces: Vec<Rgba8> = Vec::new();
    let mut tsv = String::new();
    for f in &files {
        let p = Path::new(f);
        let lower = f.to_ascii_lowercase();
        let source = if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".tif") {
            EyeSource::File(f.clone())
        } else {
            EyeSource::Embedded(f.clone(), loader.clone())
        };
        let t = std::time::Instant::now();
        let Some(img) = eyes::load(&source) else {
            println!("{f}: unreadable");
            continue;
        };
        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
        let r = eyes::analyze(&model, people.as_ref(), &img, PhotoId(0)).unwrap();
        if std::env::var("LC_EYES_FACES").is_ok() {
            let rgb: Vec<u8> = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            for d in eyes::find_faces(&model, &img).unwrap() {
                let lm = model.landmarks(&rgb, img.width, img.height, &d).unwrap().is_some();
                println!("  face {:.0},{:.0} {:.0}x{:.0} score {:.2} mesh {lm}", d.x0, d.y0, d.width(), d.height(), d.score);
                if !lm {
                    let side = d.width().max(d.height());
                    let kp = &d.keypoints[..4];
                    let kc = [kp.iter().map(|p| p[0]).sum::<f32>() / 4.0, kp.iter().map(|p| p[1]).sum::<f32>() / 4.0];
                    let ang = (d.keypoints[1][1] - d.keypoints[0][1]).atan2(d.keypoints[1][0] - d.keypoints[0][0]);
                    println!("    kp {:?} ang {ang:.2}", d.keypoints.map(|p| [(p[0] - d.x0) as i32, (p[1] - d.y0) as i32]));
                    for sc in [0.8f32, 1.0, 1.2, 1.5, 1.9, 2.4] {
                        for (cn, c) in [("box", d.center()), ("kp", kc)] {
                            for a in [ang, 0.0] {
                                let m = lightcraft_segment::mediapipe::mesh_crop_with(c, a, sc * side);
                                let ok = model.landmarks_in(&rgb, img.width, img.height, &m).unwrap().is_some();
                                if ok {
                                    println!("    ok: scale {sc} centre {cn} angle {a:.2}");
                                }
                            }
                        }
                    }
                }
            }
        }
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let mut line =
            format!("{stem}: {}x{} faces {} (no eyes {}) load {load_ms:.0} ms analyze {:.0} ms |", r.size.0, r.size.1, r.faces, r.no_eyes, r.ms);
        for (i, e) in r.eyes.iter().enumerate() {
            line += &format!(
                " #{i} {:.0}/{:.0} {:?} {:?} o{:.2} i{:.2} c{:.2}{}",
                e.sharpness,
                e.relative,
                e.focus,
                e.lids,
                e.openness.unwrap_or(-1.0),
                e.iris.unwrap_or(-1.0),
                e.contrast.unwrap_or(-1.0),
                if e.both_eyes { "" } else { " one-eye" }
            );
            if let Some(l) = e.look {
                line += &format!(
                    " [white {:.2} socket {:.2} dark {:.2} under {:.2}{}]",
                    l.sclera,
                    l.socket,
                    l.socket_dark,
                    l.under,
                    if e.landmarks { "" } else { " no mesh" }
                );
            }
            line += "
   ";
        }
        println!("{line}");
        // overview: every face found
        let s = 2400.0 / img.width.max(img.height) as f32;
        let (ow, oh) = ((img.width as f32 * s) as usize, (img.height as f32 * s) as usize);
        let rgb = eyes::rgb_resized(&img, ow, oh);
        let mut ov = Rgba8 { width: ow, height: oh, data: rgb.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2], 255]).collect() };
        for b in &r.boxes {
            rect(&mut ov, *b, [255, 40, 40, 255], 2);
        }
        for e in &r.eyes {
            let c = match e.lids {
                Lids::Closed | Lids::Half => [255, 0, 255, 255],
                Lids::Hidden => [128, 128, 128, 255],
                _ => [40, 255, 40, 255],
            };
            rect(&mut ov, e.face, c, 3);
            let (dx, dy) = (eyes::CROP_W as f32 / 2.0 / img.width as f32, eyes::CROP_H as f32 / 2.0 / img.height as f32);
            rect(&mut ov, [e.center[0] - dx, e.center[1] - dy, e.center[0] + dx, e.center[1] + dy], [255, 255, 0, 255], 1);
        }
        save_jpeg(&ov, &out.join(format!("{stem}-faces.jpg")));
        // each eye's look, marked: white of the eye green, dark red (LC_EYES_DUMP)
        if std::env::var("LC_EYES_DUMP").is_ok() {
            for (i, e) in r.eyes.iter().enumerate() {
                let c = [e.center[0] * img.width as f32, e.center[1] * img.height as f32];
                let ew = e.eye_width;
                let mut marks = Vec::new();
                eyes::eye_look_with(&img, c, ew, 0.0, |x, y| marks.push((x, y, 1u8)));
                let side = (ew * 1.6).max(8.0) as usize;
                let (x0, y0) = ((c[0] as usize).saturating_sub(side / 2), (c[1] as usize).saturating_sub(side / 2));
                let scale = (320 / side).max(1);
                let mut out = Rgba8::new(side * scale * 2, side * scale);
                for y in 0..side * scale {
                    for x in 0..side * scale {
                        let (sx, sy) = (x0 + x / scale, y0 + y / scale);
                        let p = img.data.get(sy * img.width + sx).copied().unwrap_or([0, 0, 0, 255]);
                        out.data[y * out.width + x] = p;
                        let m = marks.iter().find(|m| m.0 == sx && m.1 == sy).map(|m| m.2);
                        out.data[y * out.width + x + side * scale] = match m {
                            Some(1) => [0, 255, 0, 255],
                            Some(2) => [255, 0, 0, 255],
                            _ => p,
                        };
                    }
                }
                save_jpeg(&out, &out_dir_eye(&out, &stem, i, e));
            }
        }
        if norm.is_some() {
            for e in &r.eyes {
                let (cx, cy) = (e.center[0] * img.width as f32, e.center[1] * img.height as f32);
                let (tw, th) = (192usize, 96usize);
                let span = e.eye_width * 2.4;
                let mut t = Rgba8::new(tw, th);
                for y in 0..th {
                    for x in 0..tw {
                        let sx = cx + (x as f32 / tw as f32 - 0.5) * span;
                        let sy = cy + (y as f32 / th as f32 - 0.5) * span / 2.0;
                        if sx >= 0.0
                            && sy >= 0.0
                            && let Some(p) = img.data.get(sy as usize * img.width + sx as usize)
                        {
                            t.data[y * tw + x] = *p;
                        }
                    }
                }
                tsv += &format!(
                    "{}	{stem}	{:.1}	{:.1}	{:?}	{:?}	{:.0}	{:.2}	{:.3}	{:.2}	{:.2}	{:.2}	{:.2}	{:.2}	{}	{:.0}	{}	{:.2}
",
                    tiles.len(),
                    e.sharpness,
                    e.relative,
                    e.focus,
                    e.lids,
                    e.eye_width,
                    e.score,
                    (e.face[2] - e.face[0]) / r.eyes.iter().map(|o| o.face[2] - o.face[0]).fold(0.0f32, f32::max),
                    e.contrast.unwrap_or(-1.0),
                    e.look.map_or(-1.0, |l| l.socket),
                    e.look.map_or(-1.0, |l| l.under),
                    e.openness.unwrap_or(-1.0),
                    e.iris.unwrap_or(-1.0),
                    if e.both_eyes { "both" } else { "one" },
                    (e.face[2] - e.face[0]) * img.width as f32,
                    if e.landmarks { "mesh" } else { "nomesh" },
                    people
                        .as_ref()
                        .and_then(|pm| {
                            let (iw, ih) = (img.width as f32, img.height as f32);
                            eyes::head_share(pm, &img, [e.face[0] * iw, e.face[1] * ih, e.face[2] * iw, e.face[3] * ih])
                        })
                        .unwrap_or(-1.0)
                );
                tiles.push(t);
                let fs = 128usize;
                let (fx0, fy0, fx1, fy1) =
                    (e.face[0] * img.width as f32, e.face[1] * img.height as f32, e.face[2] * img.width as f32, e.face[3] * img.height as f32);
                let (fcx, fcy, side) = ((fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0, (fx1 - fx0).max(fy1 - fy0) * 1.4);
                let mut ft = Rgba8::new(fs, fs);
                for y in 0..fs {
                    for x in 0..fs {
                        let sx = fcx + (x as f32 / fs as f32 - 0.5) * side;
                        let sy = fcy + (y as f32 / fs as f32 - 0.5) * side;
                        if sx >= 0.0
                            && sy >= 0.0
                            && (sx as usize) < img.width
                            && let Some(p) = img.data.get(sy as usize * img.width + sx as usize)
                        {
                            ft.data[y * fs + x] = *p;
                        }
                    }
                }
                faces.push(ft);
            }
        }
        // contact sheet of crops, 10 across
        if !r.eyes.is_empty() {
            let (cw, ch) = (eyes::CROP_W, eyes::CROP_H + 4);
            let cols = r.eyes.len().min(10);
            let rows = r.eyes.len().div_ceil(10);
            let mut sheet = Rgba8::new(cols * cw, rows * ch);
            for (i, e) in r.eyes.iter().enumerate() {
                let (ox, oy) = ((i % 10) * cw, (i / 10) * ch);
                for y in 0..e.crop.height {
                    for x in 0..e.crop.width {
                        if let (Some(d), Some(s)) = (sheet.data.get_mut((oy + y) * sheet.width + ox + x), e.crop.data.get(y * e.crop.width + x)) {
                            *d = *s;
                        }
                    }
                }
                let c = match (e.lids, e.focus) {
                    (Lids::Closed, _) => [255, 0, 255, 255],
                    (Lids::Hidden, _) => [128, 128, 128, 255],
                    (_, eyes::Focus::Sharp) => [40, 220, 40, 255],
                    (_, eyes::Focus::Soft) => [240, 170, 0, 255],
                    _ => [255, 40, 40, 255],
                };
                for y in eyes::CROP_H..ch {
                    for x in 0..cw {
                        if let Some(d) = sheet.data.get_mut((oy + y) * sheet.width + ox + x) {
                            *d = c;
                        }
                    }
                }
            }
            save_jpeg(&sheet, &out.join(format!("{stem}-eyes.jpg")));
        }
    }
    if let Some(dir) = norm {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("eyes.tsv"), tsv).unwrap();
        for (k, chunk) in tiles.chunks(64).enumerate() {
            let mut sheet = Rgba8::new(8 * 194, 8 * 98);
            for (i, t) in chunk.iter().enumerate() {
                let (ox, oy) = ((i % 8) * 194, (i / 8) * 98);
                for y in 0..t.height {
                    for x in 0..t.width {
                        sheet.data[(oy + y) * sheet.width + ox + x] = t.data[y * t.width + x];
                    }
                }
            }
            save_jpeg(&sheet, &dir.join(format!("sheet{k:02}.jpg")));
        }
        for (k, chunk) in faces.chunks(64).enumerate() {
            let mut sheet = Rgba8::new(8 * 130, 8 * 130);
            for (i, t) in chunk.iter().enumerate() {
                let (ox, oy) = ((i % 8) * 130, (i / 8) * 130);
                for y in 0..t.height {
                    for x in 0..t.width {
                        sheet.data[(oy + y) * sheet.width + ox + x] = t.data[y * t.width + x];
                    }
                }
            }
            save_jpeg(&sheet, &dir.join(format!("faces{k:02}.jpg")));
        }
    }
}
