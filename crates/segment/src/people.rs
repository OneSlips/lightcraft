//! People masks from Google's MediaPipe models (Apache-2.0), run by the pure-Rust TFLite
//! interpreter ([`crate::tflite`]) on the files Google publishes, unchanged:
//!
//! - [`PeopleModel`]: Selfie Multiclass — every pixel's probability of background, hair, body skin,
//!   face skin, clothes and other (accessories);
//! - [`FaceModel`]: Face Landmarker — BlazeFace finds the faces (on overlapping windows, as photos
//!   hold smaller faces than the selfies it was made for), FaceMesh-V2 places 478 landmarks on each,
//!   and [`crate::mediapipe`] turns them into eyes, irises, brows, lips and teeth.
//!
//! [`people_mask`] combines them into one mask for a People selection ("Hair" + "Lips" of the
//! second person from the left, …). Original LightCraft code.

use std::path::Path;

use crate::mediapipe::{self, Detection, FacePart, Grid};
use crate::tflite;
use crate::{Error, Result};

/// The Selfie Multiclass file in its model folder.
pub const PEOPLE_FILE: &str = "selfie_multiclass_256x256.tflite";
/// The Face Landmarker bundle in its model folder.
pub const FACE_FILE: &str = "face_landmarker.task";
/// Faces scoring below this are ignored (BlazeFace's false positives on hands, tattoos and foliage
/// score 0.5–0.6 on wedding photos; real faces, even in profile, above 0.65).
pub const FACE_THRESHOLD: f32 = 0.65;

fn read(dir: &Path, name: &str, max: u64) -> Result<Vec<u8>> {
    let p = dir.join(name);
    let len = std::fs::metadata(&p).map_err(|e| Error::Message(format!("{}: {e}", p.display())))?.len();
    if len > max {
        return Err(Error::Message(format!("{}: unexpectedly large ({len} bytes)", p.display())));
    }
    std::fs::read(&p).map_err(|e| Error::Message(format!("{}: {e}", p.display())))
}

/// Selfie Multiclass.
pub struct PeopleModel {
    net: tflite::Model,
}

/// Per-pixel class probabilities over the image stretched to `side × side` (softmax over
/// [`mediapipe::SELFIE_CLASSES`], interleaved: `[y][x][class]`).
pub struct PeopleMap {
    pub side: usize,
    pub probs: Vec<f32>,
}

impl PeopleMap {
    /// The summed probability of `classes` at normalized point `(u, v)` (bilinear).
    pub fn at(&self, classes: &[usize], u: f32, v: f32) -> f32 {
        let n = mediapipe::SELFIE_CLASSES.len();
        let s = self.side;
        if s == 0 {
            return 0.0;
        }
        let fx = (u * s as f32 - 0.5).clamp(0.0, (s - 1) as f32);
        let fy = (v * s as f32 - 0.5).clamp(0.0, (s - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let px = |x: usize, y: usize| -> f32 {
            let base = (y.min(s - 1) * s + x.min(s - 1)) * n;
            classes.iter().filter_map(|c| self.probs.get(base + c)).sum()
        };
        let top = px(x0, y0) * (1.0 - tx) + px(x0 + 1, y0) * tx;
        let bot = px(x0, y0 + 1) * (1.0 - tx) + px(x0 + 1, y0 + 1) * tx;
        (top * (1.0 - ty) + bot * ty).clamp(0.0, 1.0)
    }

    /// The share of the image that is a person (any class but background), 0..1.
    pub fn person_share(&self) -> f32 {
        let n = mediapipe::SELFIE_CLASSES.len();
        let cells = self.probs.len() / n.max(1);
        if cells == 0 {
            return 0.0;
        }
        self.probs.chunks(n).filter(|p| p.first().is_some_and(|bg| *bg < 0.5)).count() as f32 / cells as f32
    }
}

impl PeopleModel {
    pub fn load(dir: &Path) -> Result<Self> {
        let bytes = read(dir, PEOPLE_FILE, 64 << 20)?;
        let net = tflite::Model::from_bytes(&bytes)?;
        let side = mediapipe::SELFIE_INPUT;
        if net.input_shapes() != vec![vec![1, side, side, 3]]
            || net.output_shapes().first() != Some(&vec![1, side, side, mediapipe::SELFIE_CLASSES.len()])
        {
            return Err(Error::Message(format!("{}: not the Selfie Multiclass model", dir.join(PEOPLE_FILE).display())));
        }
        Ok(Self { net })
    }

    /// Class probabilities for an 8-bit RGB image (`w × h`), stretched to the model's input.
    pub fn segment(&self, rgb: &[u8], w: usize, h: usize) -> Result<PeopleMap> {
        let side = mediapipe::SELFIE_INPUT;
        let (sx, sy) = (side as f32 / w.max(1) as f32, side as f32 / h.max(1) as f32);
        let input = mediapipe::warp(rgb, w, h, &[sx, 0.0, 0.0, 0.0, sy, 0.0], side, -1.0, 1.0);
        let out = self.net.run(&[&input])?;
        let logits = out.into_iter().next().ok_or_else(|| Error::Message("Selfie Multiclass gave no output".into()))?;
        if logits.len() != side * side * mediapipe::SELFIE_CLASSES.len() {
            return Err(Error::Message("Selfie Multiclass: unexpected output size".into()));
        }
        Ok(PeopleMap { side, probs: mediapipe::selfie_softmax(&logits) })
    }
}

/// Face Landmarker: BlazeFace + FaceMesh-V2.
pub struct FaceModel {
    detector: tflite::Model,
    mesh: tflite::Model,
}

/// A face with its landmarks (photo pixels).
#[derive(Clone, Debug)]
pub struct Face {
    pub detection: Detection,
    /// FaceMesh's 478 landmarks, when it accepted the face (it doesn't handle strong profiles).
    pub landmarks: Option<Vec<[f32; 2]>>,
}

impl FaceModel {
    pub fn load(dir: &Path) -> Result<Self> {
        let bundle = read(dir, FACE_FILE, 64 << 20)?;
        let entry = |name: &str| {
            tflite::zip::entry(&bundle, name).ok_or_else(|| Error::Message(format!("{}: no {name} inside", dir.join(FACE_FILE).display())))
        };
        let detector = tflite::Model::from_bytes(entry("face_detector.tflite")?)?;
        let mesh = tflite::Model::from_bytes(entry("face_landmarks_detector.tflite")?)?;
        let (b, m) = (mediapipe::BLAZEFACE_INPUT, mediapipe::MESH_INPUT);
        if detector.input_shapes() != vec![vec![1, b, b, 3]] || mesh.input_shapes() != vec![vec![1, m, m, 3]] {
            return Err(Error::Message(format!("{}: not the Face Landmarker bundle", dir.join(FACE_FILE).display())));
        }
        Ok(Self { detector, mesh })
    }

    /// The faces in an 8-bit RGB image (`w × h`), left to right, scoring at least [`FACE_THRESHOLD`].
    pub fn detect(&self, rgb: &[u8], w: usize, h: usize) -> Result<Vec<Detection>> {
        let b = mediapipe::BLAZEFACE_INPUT;
        let mut all = Vec::new();
        for win in mediapipe::face_windows(w, h) {
            let input = mediapipe::window(rgb, w, h, win, b);
            let out = self.detector.run(&[&input])?;
            let (Some(reg), Some(cls)) = (out.first(), out.get(1)) else { return Err(Error::Message("BlazeFace gave no output".into())) };
            for d in mediapipe::decode_blazeface(reg, cls, FACE_THRESHOLD) {
                all.push(d.from_window(win.0 as f32, win.1 as f32, win.2 as f32));
            }
        }
        let mut faces = mediapipe::merge_windows(all, 0.3);
        faces.sort_by(|a, b| a.center()[0].total_cmp(&b.center()[0]));
        Ok(faces)
    }

    /// FaceMesh's landmarks for `det` (photo pixels); `None` when it doesn't see a face there.
    pub fn landmarks(&self, rgb: &[u8], w: usize, h: usize, det: &Detection) -> Result<Option<Vec<[f32; 2]>>> {
        let m = mediapipe::mesh_crop(det);
        let input = mediapipe::warp(rgb, w, h, &m, mediapipe::MESH_INPUT, 0.0, 1.0);
        let out = self.mesh.run(&[&input])?;
        let (Some(raw), Some(presence)) = (out.first(), out.get(1).and_then(|p| p.first())) else {
            return Err(Error::Message("FaceMesh gave no output".into()));
        };
        let p = 1.0 / (1.0 + (-presence.clamp(-80.0, 80.0)).exp());
        if p < 0.5 {
            return Ok(None);
        }
        Ok(mediapipe::landmarks_to_photo(raw, &m))
    }

    /// Faces with landmarks, left to right.
    pub fn faces(&self, rgb: &[u8], w: usize, h: usize) -> Result<Vec<Face>> {
        self.detect(rgb, w, h)?.into_iter().map(|d| Ok(Face { landmarks: self.landmarks(rgb, w, h, &d)?, detection: d })).collect()
    }
}

/// What a People selection asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeopleRequest {
    /// 0: everyone; n: the n-th person from the left (by their face).
    pub person: u32,
    /// Selfie Multiclass classes (indices into [`mediapipe::SELFIE_CLASSES`]).
    pub classes: Vec<usize>,
    pub face_parts: Vec<FacePart>,
}

impl PeopleRequest {
    /// From Lightroom's part names (`"Entire Person"`, `"Face Skin"`, `"Body Skin"`, `"Hair"`,
    /// `"Clothes"`, `"Eyebrows"`, `"Eye Sclera"`, `"Iris and Pupil"`, `"Lips"`, `"Teeth"`); none,
    /// or "Entire Person", is the whole person. `None` for a name it doesn't know.
    pub fn parse(person: u32, parts: &[String]) -> Option<PeopleRequest> {
        let mut r = PeopleRequest { person, ..Default::default() };
        let whole = [1, 2, 3, 4, 5];
        if parts.is_empty() {
            r.classes = whole.to_vec();
        }
        for p in parts {
            let n = p.to_ascii_lowercase();
            let class = match n.as_str() {
                "person" | "entire person" | "entireperson" => {
                    r.classes.extend(whole);
                    continue;
                }
                "hair" => 1,
                "body skin" | "bodyskin" => 2,
                "face skin" | "faceskin" => 3,
                "skin" => {
                    r.classes.extend([2, 3]);
                    continue;
                }
                "clothes" | "clothing" => 4,
                _ => {
                    r.face_parts.push(FacePart::parse(&n)?);
                    continue;
                }
            };
            r.classes.push(class);
        }
        r.classes.sort_unstable();
        r.classes.dedup();
        r.face_parts.dedup();
        Some(r)
    }

    pub fn needs_people(&self) -> bool {
        !self.classes.is_empty()
    }

    /// Face parts, or one person among several (told apart by their faces).
    pub fn needs_faces(&self) -> bool {
        !self.face_parts.is_empty() || self.person > 0
    }
}

/// A computed mask: `side²` logits over `rect` of the image (normalized; `None` = all of it).
pub struct MaskGrid {
    pub side: usize,
    pub logits: Vec<f32>,
    pub rect: Option<[f64; 4]>,
}

/// The People selection `req` on an 8-bit RGB image (`w × h`), from the Selfie Multiclass map and
/// the faces (left to right) its parts need. `None`: nothing of it is there (no such person, no
/// face for face parts).
pub fn people_mask(req: &PeopleRequest, rgb: &[u8], w: usize, h: usize, map: Option<&PeopleMap>, faces: &[Face]) -> Option<MaskGrid> {
    if w == 0 || h == 0 {
        return None;
    }
    // the faces it is about: everyone's, or the n-th from the left
    let chosen: Vec<usize> = match req.person {
        0 => (0..faces.len()).collect(),
        n => vec![(n as usize).checked_sub(1).filter(|i| *i < faces.len())?],
    };
    let aspect = w as f32 / h as f32;
    // a pixel belongs to the person whose face is nearest (distances in photo proportions)
    let owner = |u: f32, v: f32| -> Option<usize> {
        faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let [cx, cy] = f.detection.center();
                let (dx, dy) = ((u - cx / w as f32) * aspect, (v - cy / h as f32) * 0.6);
                (i, dx * dx + dy * dy)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let mut rect = None;
    let side;
    if req.needs_people() {
        // the whole image: classes cover bodies; face parts drawn at a finer grid when only they are asked for
        side = if req.face_parts.is_empty() { 512 } else { 1024 };
    } else {
        // only face parts: a grid over the chosen faces
        let boxes: Vec<&Detection> = chosen.iter().filter_map(|i| faces.get(*i)).map(|f| &f.detection).collect();
        if boxes.is_empty() {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for b in boxes {
            let m = 0.3 * b.width().max(b.height());
            (x0, y0, x1, y1) = (x0.min(b.x0 - m), y0.min(b.y0 - m), x1.max(b.x1 + m), y1.max(b.y1 + m));
        }
        let r = [
            f64::from((x0 / w as f32).clamp(0.0, 1.0)),
            f64::from((y0 / h as f32).clamp(0.0, 1.0)),
            f64::from((x1 / w as f32).clamp(0.0, 1.0)),
            f64::from((y1 / h as f32).clamp(0.0, 1.0)),
        ];
        if !(r[2] > r[0] && r[3] > r[1]) {
            return None;
        }
        rect = Some(r);
        side = 768;
    }
    let grid = Grid { side, rect: rect.unwrap_or([0.0, 0.0, 1.0, 1.0]), w, h };
    let mut cov = vec![0f32; side * side];
    if let Some(map) = map.filter(|_| req.needs_people()) {
        let r = grid.rect;
        for j in 0..side {
            for i in 0..side {
                let u = (r[0] + (i as f64 + 0.5) / side as f64 * (r[2] - r[0])) as f32;
                let v = (r[1] + (j as f64 + 0.5) / side as f64 * (r[3] - r[1])) as f32;
                if req.person > 0 && owner(u, v).is_none_or(|o| !chosen.contains(&o)) {
                    continue;
                }
                if let Some(c) = cov.get_mut(j * side + i) {
                    *c = map.at(&req.classes, u, v);
                }
            }
        }
    }
    for i in &chosen {
        let Some(lm) = faces.get(*i).and_then(|f| f.landmarks.as_ref()) else { continue };
        for part in &req.face_parts {
            mediapipe::rasterize(&grid, &mut cov, lm, *part, rgb);
        }
    }
    if cov.iter().all(|c| *c < 0.5) {
        return None;
    }
    Some(MaskGrid { side, logits: mediapipe::coverage_to_logits(&cov), rect })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_lightrooms_part_names() {
        let r = PeopleRequest::parse(0, &[]).unwrap();
        assert_eq!(r.classes, vec![1, 2, 3, 4, 5]);
        assert!(!r.needs_faces());
        let r = PeopleRequest::parse(2, &["Hair".into(), "Lips".into(), "Iris and Pupil".into()]).unwrap();
        assert_eq!(r.classes, vec![1]);
        assert_eq!(r.face_parts, vec![FacePart::Lips, FacePart::Iris]);
        assert!(r.needs_faces() && r.needs_people());
        assert!(PeopleRequest::parse(0, &["Elbows".into()]).is_none());
    }

    fn map_left_half_hair() -> PeopleMap {
        let side = 16;
        let mut probs = vec![0f32; side * side * 6];
        for y in 0..side {
            for x in 0..side {
                let c = if x < side / 2 { 1 } else { 0 };
                probs[(y * side + x) * 6 + c] = 1.0;
            }
        }
        PeopleMap { side, probs }
    }

    #[test]
    fn masks_follow_the_classes_and_the_chosen_person() {
        let map = map_left_half_hair();
        assert!((map.person_share() - 0.5).abs() < 1e-6);
        let req = PeopleRequest::parse(0, &["Hair".into()]).unwrap();
        let m = people_mask(&req, &[], 100, 100, Some(&map), &[]).unwrap();
        assert_eq!(m.side, 512);
        assert!(m.logits[256 * 512 + 50] > 4.0 && m.logits[256 * 512 + 450] < -4.0);
        // two faces: person 2 (the right one) owns the right half, where there is no hair
        let face = |x: f32| Face {
            detection: Detection { x0: x - 5.0, y0: 10.0, x1: x + 5.0, y1: 20.0, score: 1.0, keypoints: [[0.0; 2]; 6] },
            landmarks: None,
        };
        let faces = [face(25.0), face(75.0)];
        assert!(people_mask(&PeopleRequest::parse(2, &["Hair".into()]).unwrap(), &[], 100, 100, Some(&map), &faces).is_none());
        let m = people_mask(&PeopleRequest::parse(1, &["Hair".into()]).unwrap(), &[], 100, 100, Some(&map), &faces).unwrap();
        assert!(m.logits[256 * 512 + 50] > 4.0);
        // no such person, or face parts without landmarks: nothing
        assert!(people_mask(&PeopleRequest::parse(3, &["Hair".into()]).unwrap(), &[], 100, 100, Some(&map), &faces).is_none());
        assert!(people_mask(&PeopleRequest::parse(0, &["Lips".into()]).unwrap(), &[], 100, 100, None, &faces).is_none());
        assert!(people_mask(&req, &[], 0, 0, Some(&map), &[]).is_none());
    }

    #[test]
    fn missing_or_wrong_model_files_are_errors() {
        let dir = std::env::temp_dir().join(format!("lc-people-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(PeopleModel::load(&dir).is_err());
        assert!(FaceModel::load(&dir).is_err());
        std::fs::write(dir.join(PEOPLE_FILE), b"not a tflite file").unwrap();
        std::fs::write(dir.join(FACE_FILE), b"PK not a zip").unwrap();
        assert!(PeopleModel::load(&dir).is_err());
        assert!(FaceModel::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
