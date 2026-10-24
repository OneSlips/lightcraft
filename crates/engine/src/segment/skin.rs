//! The skin and faces Smooth Skin and Find Blemishes work with, from the open MediaPipe models
//! (Selfie Multiclass for skin, Face Landmarker for faces and their landmarks), computed on the AI
//! mask worker like the People masks.
//!
//! Smooth Skin keeps what was found in the photo's settings ([`SkinAnalysis`]), so rendering and
//! exporting never need a model. It is the photo's own data, not an edit: it is in no settings
//! group (copy, paste, sync and presets don't carry it), it is stored without an undo step, and
//! the undo and redo steps that want it get it too, so stepping through history never loses it.
//! After every command, the photos whose Smooth Skin wants an analysis they lack (just switched
//! on, pasted, synced, another area or person, turned) get one ([`Session::skin_follow_up`]):
//! queued in the desktop app, waited for in the CLI, MCP and tests.

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::{DevelopSettings, SegMask, SkinAnalysis, SkinArea, SkinFace};
use lightcraft_geom::Orientation;

use crate::Session;

/// FaceMesh landmarks: the iris centres (image left first), the nose tip, the mouth corners.
const IRIS: [usize; 2] = [468, 473];
const NOSE_TIP: usize = 1;
const MOUTH_CORNERS: [usize; 2] = [61, 291];

/// A face the MediaPipe models found, in normalized coordinates (0..1, y down) of the uncropped,
/// upright photo as developed.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundFace {
    /// `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
    pub score: f32,
    /// BlazeFace's keypoints: right eye (on the image's left), left eye, nose tip, mouth, right
    /// ear, left ear.
    pub keypoints: [[f32; 2]; 6],
    /// FaceMesh's 478 landmarks, when it placed them (it doesn't on strong profiles).
    pub mesh: Option<Vec<[f32; 2]>>,
    /// Width / height of the photo it was found in.
    pub aspect: f32,
}

impl FoundFace {
    /// From the models' output in pixels of a `w × h` input.
    #[cfg(feature = "sam")]
    pub(crate) fn from_pixels(f: &lightcraft_segment::people::Face, w: f32, h: f32) -> FoundFace {
        let d = &f.detection;
        let n = |p: [f32; 2]| [p[0] / w, p[1] / h];
        FoundFace {
            rect: [d.x0 / w, d.y0 / h, d.x1 / w, d.y1 / h],
            score: d.score,
            keypoints: d.keypoints.map(n),
            mesh: f.landmarks.as_ref().map(|lm| lm.iter().map(|p| n(*p)).collect()),
            aspect: if h > 0.0 { w / h } else { 1.0 },
        }
    }

    fn mesh_point(&self, i: usize) -> Option<[f32; 2]> {
        self.mesh.as_ref()?.get(i).copied().filter(|p| p[0].is_finite() && p[1].is_finite())
    }

    /// The eye centres (image left first): the irises when FaceMesh placed them, else BlazeFace's.
    pub fn eyes(&self) -> ([f32; 2], [f32; 2]) {
        match (self.mesh_point(IRIS[0]), self.mesh_point(IRIS[1])) {
            (Some(a), Some(b)) => (a, b),
            _ => (self.keypoints[0], self.keypoints[1]),
        }
    }

    /// The distance between the eyes as a fraction of the photo's long edge: the scale Smooth Skin
    /// measures its bands in. A turned face's eyes look closer than they are, so it is at least a
    /// share of the face's width (a fifth with landmarks, about a third without: then the face is in
    /// profile, or the eyes are guesses).
    pub fn eye_distance(&self) -> f64 {
        let (a, b) = self.eyes();
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { f64::from(self.aspect) } else { 1.0 };
        // (in units of the photo's height, then of its long edge)
        let (dx, dy) = (f64::from(b[0] - a[0]) * aspect, f64::from(b[1] - a[1]));
        let width = f64::from(self.rect[2] - self.rect[0]).abs() * aspect;
        let floor = width * if self.mesh.is_some() { 0.2 } else { 0.33 };
        let d = dx.hypot(dy).max(floor) / aspect.max(1.0);
        if d.is_finite() { d } else { 0.0 }
    }

    /// The box's centre.
    pub fn centre(&self) -> [f32; 2] {
        [(self.rect[0] + self.rect[2]) / 2.0, (self.rect[1] + self.rect[3]) / 2.0]
    }

    /// The five points a face recogniser aligns by, in its order: right eye, left eye, nose tip,
    /// right and left mouth corner (from the subject's point of view: the right eye is on the
    /// image's left). Without landmarks the mouth corners are placed beside BlazeFace's mouth
    /// centre, as far apart as a face's are relative to its eyes.
    pub fn five(&self) -> [(f32, f32); 5] {
        let (re, le) = self.eyes();
        let nose = self.mesh_point(NOSE_TIP).unwrap_or(self.keypoints[2]);
        let (mr, ml) = match (self.mesh_point(MOUTH_CORNERS[0]), self.mesh_point(MOUTH_CORNERS[1])) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                let m = self.keypoints[3];
                let half = [(le[0] - re[0]) * 0.415, (le[1] - re[1]) * 0.415];
                ([m[0] - half[0], m[1] - half[1]], [m[0] + half[0], m[1] + half[1]])
            }
        };
        [re, le, nose, mr, ml].map(|p| (p[0], p[1]))
    }
}

/// The open AI model a command's error asks for (its key), when it says one isn't installed: the
/// error as a command reports it (`invalid parameters for `…`: The … model is not installed …`).
pub fn missing_model_in(error: &str) -> Option<&'static str> {
    super::models::missing_model(error).or_else(|| error.split_once(": ").and_then(|(_, rest)| super::models::missing_model(rest)))
}

/// The skin and faces found on a photo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinFound {
    /// The skin asked for (`None`: none found, e.g. no such person).
    pub seg: Option<SegMask>,
    /// Every face, left to right.
    pub faces: Vec<FoundFace>,
}

/// One analysis: which photo, which skin, the photo turned how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SkinKey {
    pub photo: PhotoId,
    pub area: SkinArea,
    pub person: u32,
    pub orientation: Orientation,
}

impl SkinKey {
    /// What photo `photo`'s settings `d` want analysed.
    fn of(photo: PhotoId, d: &DevelopSettings) -> SkinKey {
        SkinKey { photo, area: d.skin.area, person: d.skin.person, orientation: d.orientation }
    }
}

/// What a queued skin request is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SkinPurpose {
    /// Store it for Smooth Skin.
    Smooth,
    /// Find blemishes with it and add a heal spot on each.
    Blemishes(lightcraft_pipeline::blemish::Options),
}

/// Smooth Skin's analysis of `found`: its skin (an empty one when none was found, so nothing is
/// smoothed and nothing is asked again) and the faces its bands are measured by (the chosen
/// person's, or everyone's).
pub(crate) fn analysis_of(area: SkinArea, person: u32, orientation: Orientation, found: &SkinFound) -> SkinAnalysis {
    let faces: Vec<&FoundFace> = match person {
        0 => found.faces.iter().collect(),
        n => found.faces.get(n as usize - 1).into_iter().collect(),
    };
    let faces = faces
        .into_iter()
        .map(|f| {
            let c = f.centre();
            SkinFace { x: f64::from(c[0]), y: f64::from(c[1]), eyes: f.eye_distance() }
        })
        .filter(|f| f.eyes > 0.0 && f.x.is_finite() && f.y.is_finite())
        .take(lightcraft_develop::MAX_SKIN_FACES)
        .collect();
    SkinAnalysis { area, person, orientation, seg: found.seg.clone().unwrap_or_else(|| SegMask::from_logits(1, &[-16.0])), faces }
}

/// Whether settings `d` want analysis `a` (it is for their area, person and orientation) and
/// don't hold it yet.
fn wants(d: &DevelopSettings, a: &SkinAnalysis) -> bool {
    a.is_for(&d.skin, d.orientation) && d.skin_analysis.as_ref() != Some(a)
}

/// Photo ids whose settings `op` sets.
fn developed(op: &Op, out: &mut Vec<PhotoId>) {
    match op {
        Op::SetDevelop { id, .. } if !out.contains(id) => out.push(*id),
        Op::Batch { ops } => ops.iter().for_each(|o| developed(o, out)),
        _ => {}
    }
}

impl Session {
    /// The People (Selfie Multiclass) and face (Face Landmarker) model folders, or an error naming
    /// the one to download.
    fn skin_models(&self) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
        Ok((self.segmenter.require_model("people")?, self.segmenter.require_model("face")?))
    }

    /// Find the skin of `area` (everyone's, or the `person`-th person's from the left) and every
    /// face on photo `id`, waiting for the models (CLI, MCP, tests).
    pub fn segment_skin(&mut self, id: PhotoId, area: SkinArea, person: u32) -> Result<SkinFound, String> {
        let (people, face) = self.skin_models()?;
        #[cfg(feature = "sam")]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let kind = super::worker::Kind::Skin { area, person, people, face };
            let job = self.segment_job(id, kind, super::Tag::Wait, tx, None)?;
            self.segmenter.worker.submit(job)?;
            match rx.recv_timeout(super::WAIT) {
                Ok(o) => Ok(SkinFound { seg: o.result?, faces: o.faces.unwrap_or_default() }),
                Err(_) => Err("the AI model took too long; try again".into()),
            }
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = (id, area, person, people, face);
            Err("AI masks are not available in this build".into())
        }
    }

    /// Background mode: queue the skin request `key` for `purpose`; [`Session::segment_poll`]
    /// applies it.
    pub(crate) fn segment_skin_later(&mut self, key: SkinKey, purpose: SkinPurpose) -> Result<(), String> {
        let (people, face) = self.skin_models()?;
        #[cfg(feature = "sam")]
        {
            let kind = super::worker::Kind::Skin { area: key.area, person: key.person, people, face };
            self.segment_queue(key.photo, kind, super::Tag::Skin { key, purpose }, None)
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = (key, purpose, people, face);
            Err("AI masks are not available in this build".into())
        }
    }

    /// Give photo `id` Smooth Skin's analysis `a`: in its settings when they want it, and in every
    /// undo and redo step, an interaction's starting points and the History steps that want it
    /// ([`Session::merge_background`]: edits made while it ran are kept). Stored and written to the
    /// sidecar like any change, but without an undo step of its own.
    pub(crate) fn store_skin_analysis(&mut self, id: PhotoId, a: &SkinAnalysis) {
        let give = |d: &mut DevelopSettings| {
            if !wants(d, a) {
                return false;
            }
            d.skin_analysis = Some(a.clone());
            true
        };
        self.merge_background(id, "Find Skin", false, &give);
    }

    /// Whether photo `id`'s Smooth Skin wants an analysis it doesn't have (`Some(key)`).
    pub(crate) fn skin_wanted(&self, id: PhotoId) -> Option<SkinKey> {
        let d = self.develop_of(id)?;
        if !d.skin.active() || !d.retouch_section_on("skin") {
            return None;
        }
        if d.skin_analysis.as_ref().is_some_and(|a| a.is_for(&d.skin, d.orientation)) {
            return None;
        }
        Some(SkinKey::of(id, &d))
    }

    /// Get photo `id` the analysis its Smooth Skin wants, if it lacks one: queued in background
    /// mode, waited for otherwise. `force` asks again after a failure. `Ok(true)`: it has one now
    /// (or it is on its way); `Ok(false)`: nothing was wanted.
    pub(crate) fn skin_ensure(&mut self, id: PhotoId, force: bool) -> Result<bool, String> {
        let Some(key) = self.skin_wanted(id) else { return Ok(false) };
        if force {
            self.segmenter.skin_failed.retain(|k| *k != key);
        }
        if self.segmenter.skin_pending.contains(&key) {
            return Ok(true);
        }
        if self.segmenter.skin_failed.contains(&key) {
            return Err("Smooth Skin couldn't find the skin on this photo before; use Find Skin to try again".into());
        }
        if self.segmenter.background {
            match self.segment_skin_later(key, SkinPurpose::Smooth) {
                Ok(()) => {
                    self.segmenter.skin_pending.push(key);
                    Ok(true)
                }
                Err(e) => {
                    self.segmenter.skin_failed.push(key);
                    Err(e)
                }
            }
        } else {
            match self.segment_skin(id, key.area, key.person) {
                Ok(found) => {
                    self.store_skin_analysis(id, &analysis_of(key.area, key.person, key.orientation, &found));
                    Ok(true)
                }
                Err(e) => {
                    self.segmenter.skin_failed.push(key);
                    Err(e)
                }
            }
        }
    }

    /// After a command: every photo whose settings it set, and whose Smooth Skin wants an
    /// analysis, gets one (see the module doc). Failures are reported once, as messages.
    pub(crate) fn skin_follow_up(&mut self, log_start: usize) {
        if !super::Segmenter::AVAILABLE {
            return;
        }
        let mut ids = Vec::new();
        for op in self.pending_log.get(log_start..).unwrap_or(&[]) {
            developed(op, &mut ids);
        }
        // (a slider drag changes the photo without a step until it ends: start looking at once)
        if let Some(i) = &self.interaction
            && !ids.contains(&i.photo)
        {
            ids.push(i.photo);
        }
        for id in ids {
            if self.skin_wanted(id).is_none() {
                continue;
            }
            if let Err(e) = self.skin_ensure(id, false)
                && !e.starts_with("Smooth Skin couldn't")
            {
                log::warn!("Smooth Skin: {e}");
                self.segmenter.push_message(format!("Smooth Skin: {e}"));
            }
        }
    }

    /// A finished background skin request.
    #[cfg(feature = "sam")]
    pub(crate) fn apply_skin_outcome(
        &mut self,
        key: SkinKey,
        purpose: SkinPurpose,
        result: Result<Option<SegMask>, String>,
        faces: Option<Vec<FoundFace>>,
        polled: &mut super::Polled,
    ) {
        let found = match result {
            Ok(seg) => SkinFound { seg, faces: faces.unwrap_or_default() },
            Err(e) => {
                if purpose == SkinPurpose::Smooth {
                    self.segmenter.skin_pending.retain(|k| *k != key);
                    self.segmenter.skin_failed.push(key);
                    return polled.messages.push(format!("Smooth Skin: {e}"));
                }
                return polled.messages.push(format!("Find Blemishes: {e}"));
            }
        };
        match purpose {
            SkinPurpose::Smooth => {
                self.segmenter.skin_pending.retain(|k| *k != key);
                if found.seg.is_none() {
                    polled.messages.push(match key.person {
                        0 => "Smooth Skin: no skin was found in this photo.".to_string(),
                        n => format!("Smooth Skin: person {n} wasn't found in this photo."),
                    });
                }
                let a = analysis_of(key.area, key.person, key.orientation, &found);
                let r = self.execute_fn("skin.found", |s| {
                    s.store_skin_analysis(key.photo, &a);
                    Ok(serde_json::Value::Null)
                });
                match r {
                    Ok(_) => polled.changed = true,
                    Err(e) => polled.messages.push(e.to_string()),
                }
            }
            SkinPurpose::Blemishes(opts) => {
                let r = self.execute_fn("spot.findBlemishes", |s| crate::cmd::retouch::add_blemishes(s, key.photo, &found, &opts));
                match r {
                    Ok(v) => {
                        polled.changed = true;
                        polled.messages.push(crate::cmd::retouch::blemishes_message(&v));
                    }
                    Err(e) => polled.messages.push(e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(mesh: bool) -> FoundFace {
        let mut lm = vec![[0.5f32, 0.5]; 478];
        lm[IRIS[0]] = [0.4, 0.4];
        lm[IRIS[1]] = [0.6, 0.4];
        lm[NOSE_TIP] = [0.5, 0.5];
        lm[MOUTH_CORNERS[0]] = [0.42, 0.6];
        lm[MOUTH_CORNERS[1]] = [0.58, 0.6];
        FoundFace {
            rect: [0.3, 0.25, 0.7, 0.75],
            score: 0.9,
            keypoints: [[0.41, 0.41], [0.59, 0.41], [0.5, 0.5], [0.5, 0.6], [0.3, 0.45], [0.7, 0.45]],
            mesh: mesh.then_some(lm),
            aspect: 1.5,
        }
    }

    #[test]
    fn eye_distance_is_a_fraction_of_the_long_edge() {
        // 0.2 of the width apart on a 3:2 photo: 0.2 of the long edge
        assert!((face(true).eye_distance() - 0.2).abs() < 1e-6);
        // without landmarks, BlazeFace's eyes, but at least a third of the face's width
        assert!((face(false).eye_distance() - 0.18).abs() < 1e-6);
        let mut turned = face(false);
        turned.keypoints[1] = [0.43, 0.41];
        assert!((turned.eye_distance() - 0.4 * 0.33).abs() < 1e-6, "{}", turned.eye_distance());
        // portrait 2:3: x is the short edge
        let mut f = face(true);
        f.aspect = 2.0 / 3.0;
        assert!((f.eye_distance() - 0.2 * 2.0 / 3.0).abs() < 1e-6);
        f.aspect = f32::NAN;
        assert!(f.eye_distance().is_finite());
    }

    #[test]
    fn the_five_alignment_points_follow_the_recognisers_order() {
        let p = face(true).five();
        assert_eq!(p[0], (0.4, 0.4));
        assert_eq!(p[1], (0.6, 0.4));
        assert_eq!(p[2], (0.5, 0.5));
        assert_eq!((p[3], p[4]), ((0.42, 0.6), (0.58, 0.6)));
        // without landmarks the mouth corners sit beside the mouth centre, image left first
        let q = face(false).five();
        assert!(q[3].0 < 0.5 && q[4].0 > 0.5 && (q[3].1 - 0.6).abs() < 1e-6);
    }

    #[test]
    fn an_analysis_takes_the_chosen_persons_face() {
        let mut left = face(true);
        left.rect = [0.1, 0.2, 0.3, 0.5];
        let found = SkinFound { seg: Some(SegMask::from_logits(2, &[1.0; 4])), faces: vec![left, face(true)] };
        let all = analysis_of(SkinArea::Face, 0, Orientation::Normal, &found);
        assert_eq!(all.faces.len(), 2);
        let second = analysis_of(SkinArea::Face, 2, Orientation::Normal, &found);
        assert_eq!(second.faces.len(), 1);
        assert!((second.faces[0].x - 0.5).abs() < 1e-6);
        assert!(analysis_of(SkinArea::Face, 3, Orientation::Normal, &found).faces.is_empty());
        // nothing found: an empty skin, so nothing is smoothed and nothing is asked again
        let none = analysis_of(SkinArea::Body, 1, Orientation::Rotate90, &SkinFound::default());
        assert_eq!((none.area, none.person, none.orientation), (SkinArea::Body, 1, Orientation::Rotate90));
        assert!(none.seg.logits().is_some_and(|l| l.iter().all(|v| *v < 0.0)));
    }
}
