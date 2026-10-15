//! `faces.detect`: find faces with the open MediaPipe face model (Face Landmarker: BlazeFace and
//! FaceMesh-V2, Apache-2.0, `ai.models.download {"id": "face"}`), the one People masks, Find
//! Blemishes and Smooth Skin use. It replaced YuNet, whose weights were trained on WIDER FACE
//! (CC BY-NC-ND); a YuNet installed by an older LightCraft is left where it is and not used.
//!
//! The photo is rendered upright and uncropped with default settings (so edits, crops and rotations you have
//! made do not matter), the detector runs on it, and the faces come back as fractions of the photo's width
//! and height, the same frame face regions from XMP use. Unless `apply` is false, they are added to the photo as
//! unnamed face regions in one undoable step; earlier detections are replaced, and regions that came from XMP (or
//! that you drew or named) are never touched. Like every region edit it is catalog-only: no sidecar is written.

use std::sync::Arc;
use std::time::Instant;

use lightcraft_catalog::Op;
use lightcraft_develop::DevelopSettings;
use lightcraft_faces::yunet::Face;
use lightcraft_meta::{Region, RegionKind};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_selection};
use crate::{Result, Session};

const C: &str = "faces.detect";
/// Regions made by this command say so in their description; only those are replaced on a new run.
pub(crate) const MARK: &str = "Detected by ";
pub(crate) const LABEL: &str = "MediaPipe Face Landmarker";
/// Long edge of the picture the detector looks at.
const LOOK_EDGE: usize = 1280;
/// How much a detection must overlap a region the photo already has to count as the same face.
const EXISTING_IOU: f64 = 0.3;
/// The default score a face must reach (BlazeFace's own floor is 0.65).
const DEFAULT_SCORE: f32 = 0.65;
/// Most faces one photo gives.
const DEFAULT_MAX_FACES: usize = 200;

/// The open face model, loaded.
pub(crate) struct Finder {
    #[cfg(feature = "sam")]
    model: lightcraft_segment::people::FaceModel,
}

impl Finder {
    /// The faces in an 8-bit RGB picture (`w × h`), left to right: boxes and five landmarks as fractions of the
    /// picture (the eyes, the nose tip and the mouth corners, from FaceMesh where it placed its landmarks). `fine`
    /// also looks in overlapping tiles of half the picture, which finds smaller faces (group photos) at about four
    /// times the cost.
    pub(crate) fn detect(&self, rgb: &[u8], w: usize, h: usize, fine: bool) -> std::result::Result<Vec<Face>, String> {
        #[cfg(feature = "sam")]
        {
            use lightcraft_segment::{mediapipe, people};
            if w == 0 || h == 0 || rgb.len() < w * h * 3 {
                return Ok(Vec::new());
            }
            let mut dets = self.model.detect(rgb, w, h).map_err(|e| e.to_string())?;
            if fine && w.min(h) >= 512 {
                // half-size tiles, overlapping by half: each is searched like a whole photo
                let (tw, th) = (w / 2, h / 2);
                let tiles = [0, w / 4, w - tw].into_iter().flat_map(|x| [0, h / 4, h - th].into_iter().map(move |y| (x, y)));
                for (x0, y0) in tiles {
                    let mut tile = Vec::with_capacity(tw * th * 3);
                    for y in y0..y0 + th {
                        tile.extend_from_slice(rgb.get((y * w + x0) * 3..(y * w + x0 + tw) * 3).unwrap_or(&[]));
                    }
                    if tile.len() != tw * th * 3 {
                        continue;
                    }
                    for d in self.model.detect(&tile, tw, th).map_err(|e| e.to_string())? {
                        dets.push(d.from_window(x0 as f32, y0 as f32, 1.0));
                    }
                }
                dets = mediapipe::merge_windows(dets, 0.3);
                dets.sort_by(|a, b| a.center()[0].total_cmp(&b.center()[0]));
            }
            let mut out = Vec::new();
            for d in dets {
                let landmarks = self.model.landmarks(rgb, w, h, &d).map_err(|e| e.to_string())?;
                let found = crate::segment::FoundFace::from_pixels(&people::Face { detection: d, landmarks }, w as f32, h as f32);
                let [x0, y0, x1, y1] = found.rect;
                out.push(Face { x0, y0, x1, y1, score: found.score, landmarks: found.five() });
            }
            Ok(out)
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = (rgb, w, h, fine);
            Err(NOT_AVAILABLE.into())
        }
    }
}

#[cfg(not(feature = "sam"))]
const NOT_AVAILABLE: &str = "finding faces needs the open AI models, which this build does not include";

/// The installed face model, loaded once and kept (a new one is loaded when its folder changes): from the AI models
/// folder, or else the face models folder (by default they are the same, `<settings>/models`).
/// (also used by the background scan, which aligns faces by the landmarks and finds faces in photos that have none)
pub(crate) fn detector(s: &Session) -> std::result::Result<Arc<Finder>, String> {
    #[cfg(feature = "sam")]
    {
        use std::path::PathBuf;
        use std::sync::{Mutex, PoisonError};
        static LOADED: Mutex<Option<(PathBuf, Arc<Finder>)>> = Mutex::new(None);
        let info = &lightcraft_segment::models::FACE;
        let dir = match s.segmenter.require_model("face") {
            Ok(d) => d,
            Err(e) => match s.face_models_dir.as_ref().map(|d| d.join(info.dir_name)).filter(|d| info.installed_in(d)) {
                Some(d) => d,
                None => return Err(e),
            },
        };
        let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((p, d)) = loaded.as_ref()
            && *p == dir
        {
            return Ok(d.clone());
        }
        let model = lightcraft_segment::people::FaceModel::load(&dir)
            .map_err(|e| format!("the face model doesn't load ({e}): remove it and download it again"))?;
        let d = Arc::new(Finder { model });
        *loaded = Some((dir, d.clone()));
        Ok(d)
    }
    #[cfg(not(feature = "sam"))]
    {
        let _ = s;
        Err(NOT_AVAILABLE.into())
    }
}

fn face_json(f: &Face) -> Value {
    json!({
        "rect": {"x0": f.x0, "y0": f.y0, "x1": f.x1, "y1": f.y1},
        "score": f.score,
        "landmarks": f.landmarks.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>(),
    })
}

/// A detected face that lies on a region the photo already has (from XMP, drawn or named) is that region, not a new face.
fn overlaps_existing(f: &Face, regions: &[Region]) -> bool {
    regions.iter().any(|r| {
        let (w, h) = (
            (r.rect.x1.min(f64::from(f.x1)) - r.rect.x0.max(f64::from(f.x0))).max(0.0),
            (r.rect.y1.min(f64::from(f.y1)) - r.rect.y0.max(f64::from(f.y0))).max(0.0),
        );
        let inter = w * h;
        let union = (r.rect.x1 - r.rect.x0) * (r.rect.y1 - r.rect.y0) + f64::from((f.x1 - f.x0) * (f.y1 - f.y0)) - inter;
        union > 0.0 && inter / union >= EXISTING_IOU
    })
}

pub(crate) fn is_detected(r: &Region) -> bool {
    r.description.as_deref().is_some_and(|d| d.starts_with(MARK))
}

/// The photo as the faces see it: upright, uncropped, default settings, as 8-bit RGB with its size, its long edge at
/// most `edge`.
pub(crate) fn render_rgb(s: &mut Session, id: lightcraft_catalog::PhotoId, edge: usize) -> std::result::Result<(Vec<u8>, usize, usize), String> {
    let settings = DevelopSettings::default();
    let job = s.preview_job(id, edge, edge, false, &settings).ok_or("no such photo")?;
    let image = job.run().rendered?.image;
    let rgb: Vec<u8> = image.data.iter().flat_map(|px| [px[0], px[1], px[2]]).collect();
    Ok((rgb, image.width, image.height))
}

fn detect(s: &mut Session, p: &Value) -> Result<Value> {
    let apply = p.get("apply").and_then(Value::as_bool).unwrap_or(true);
    let score = p.get("score").and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(DEFAULT_SCORE, |v| v as f32);
    let max_faces = p.get("maxFaces").and_then(Value::as_u64).map_or(DEFAULT_MAX_FACES, |v| usize::try_from(v).unwrap_or(DEFAULT_MAX_FACES));
    let fine = p.get("fine").and_then(Value::as_bool).unwrap_or(true);
    let detector = detector(s).map_err(|e| bad(C, e))?;
    let started = Instant::now();
    let (mut results, mut ops) = (Vec::new(), Vec::new());
    for id in s.targets(p) {
        let (rgb, width, height) = match render_rgb(s, id, LOOK_EDGE) {
            Ok(r) => r,
            Err(e) => {
                results.push(json!({"id": id.0, "error": e}));
                continue;
            }
        };
        let faces = match detector.detect(&rgb, width, height, fine) {
            Ok(mut f) => {
                f.retain(|f| f.score >= score);
                f.truncate(max_faces);
                f
            }
            Err(e) => {
                results.push(json!({"id": id.0, "error": e.to_string()}));
                continue;
            }
        };
        if apply && let Some(photo) = s.catalog.photo(id) {
            let mut meta = photo.meta.clone();
            meta.regions.retain(|r| !is_detected(r));
            let fresh: Vec<&Face> = faces.iter().filter(|f| !overlaps_existing(f, &meta.regions)).collect();
            for f in fresh {
                meta.regions.push(Region {
                    rect: lightcraft_geom::Rect { x0: f64::from(f.x0), y0: f64::from(f.y0), x1: f64::from(f.x1), y1: f64::from(f.y1) },
                    kind: RegionKind::Face,
                    name: None,
                    description: Some(format!("{MARK}{LABEL}")),
                });
            }
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
        results.push(json!({"id": id.0, "faces": faces.iter().map(face_json).collect::<Vec<_>>()}));
    }
    let applied = !ops.is_empty();
    if applied {
        s.commit("Detect Faces", Op::Batch { ops })?;
        // regions are not written to XMP, so a sidecar must not be rewritten for this
        s.skip_auto_write = true;
    }
    Ok(json!({"detector": LABEL, "applied": applied, "photos": results, "ms": started.elapsed().as_secs_f64() * 1000.0}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "faces.detect",
        "Detect Faces",
        ["Photo"],
        None,
        "{ids?, apply?: bool (default true), score?: 0.65, maxFaces?: 200, fine?: bool (true: also looks in tiles, for the small faces of group photos)} → {photos: [{id, faces: [{rect, score, landmarks}]}]}. Finds faces with the open MediaPipe face model (Apache-2.0; download it first: `ai.models.download {id: \"face\", acknowledged: true}`) and, unless `apply` is false, adds them as unnamed face regions (one undo step), replacing earlier detections but never regions from XMP or ones you named; the sidecar is not touched",
        has_selection,
        detect
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x0: f32, y0: f32, x1: f32, y1: f32) -> Face {
        Face { x0, y0, x1, y1, score: 0.9, landmarks: [(0.0, 0.0); 5] }
    }
    fn region(x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
        Region { rect: lightcraft_geom::Rect { x0, y0, x1, y1 }, kind: RegionKind::Face, name: Some("Ann".into()), description: None }
    }

    #[test]
    fn a_detection_on_an_existing_region_is_not_a_new_face() {
        let existing = [region(0.40, 0.20, 0.60, 0.50), region(0.10, 0.10, 0.20, 0.20)];
        // the same face, a bit larger or shifted (a Lightroom box is not the detector's box)
        assert!(overlaps_existing(&face(0.38, 0.18, 0.64, 0.54), &existing));
        assert!(overlaps_existing(&face(0.42, 0.22, 0.58, 0.48), &existing));
        // another face, a sliver of overlap, and no regions at all
        assert!(!overlaps_existing(&face(0.70, 0.20, 0.90, 0.50), &existing));
        assert!(!overlaps_existing(&face(0.59, 0.49, 0.80, 0.80), &existing));
        assert!(!overlaps_existing(&face(0.0, 0.0, 0.0, 0.0), &existing), "an empty box overlaps nothing");
        assert!(!overlaps_existing(&face(0.4, 0.2, 0.6, 0.5), &[]));
    }
}
