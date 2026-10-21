//! The parts of people the Retouch sections work on (teeth, eyes, lips, hair…), from the open
//! MediaPipe models, computed on the AI mask worker like the People masks and Smooth Skin's skin.
//!
//! What was found is kept in the photo's settings ([`RetouchAnalysis`]), so rendering and exporting
//! never need a model. Like Smooth Skin's skin it is the photo's own data: in no settings group,
//! stored without an undo step (and given to the undo and redo steps that want it). After every
//! command, the photos whose retouch sections want a part they don't hold (a section switched on,
//! pasted, another person, the photo turned) get it ([`Session::retouch_follow_up`]): queued in the
//! desktop app (an activity row while the models work), waited for in the CLI, MCP and tests.

use std::sync::Arc;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::{DevelopSettings, PartRequest, RetouchAnalysis, RetouchMask, SkinFace};
use lightcraft_geom::Orientation;

use super::FoundFace;
use crate::Session;

/// One request: which photo, which parts, the photo turned how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetouchKey {
    pub photo: PhotoId,
    pub parts: Vec<PartRequest>,
    pub orientation: Orientation,
}

/// The parts of `d`'s retouch sections that are on (their eye too).
pub(crate) fn wanted(d: &DevelopSettings) -> Vec<PartRequest> {
    if !d.section_enabled("retouch") {
        return Vec::new();
    }
    d.retouch.wanted(|k| d.section_enabled(&format!("retouch.{k}")))
}

/// The analysis of `found` (the parts asked for, each found or not, and every face).
pub(crate) fn analysis_of(orientation: Orientation, masks: Vec<RetouchMask>, faces: &[FoundFace]) -> RetouchAnalysis {
    let faces = faces
        .iter()
        .map(|f| {
            let c = f.centre();
            SkinFace { x: f64::from(c[0]), y: f64::from(c[1]), eyes: f.eye_distance() }
        })
        .filter(|f| f.eyes > 0.0 && f.x.is_finite() && f.y.is_finite())
        .take(lightcraft_develop::MAX_SKIN_FACES)
        .collect();
    RetouchAnalysis { orientation, masks, faces }
}

/// Settings `d` with analysis `a` merged into theirs, when it is for their orientation and adds
/// something (`None`: nothing to change).
fn with(d: &DevelopSettings, a: &RetouchAnalysis) -> Option<DevelopSettings> {
    if a.orientation != d.orientation {
        return None;
    }
    let merged = match &d.retouch_analysis {
        Some(old) => old.merged(a),
        None => a.clone(),
    };
    if d.retouch_analysis.as_ref() == Some(&merged) {
        return None;
    }
    Some(DevelopSettings { retouch_analysis: Some(merged), ..d.clone() })
}

/// `op` with analysis `a` given to photo `id`'s settings wherever they want it.
fn patch_op(op: &mut Op, id: PhotoId, a: &RetouchAnalysis) {
    match op {
        Op::SetDevelop { id: pid, settings, .. } if *pid == id => {
            if let Some(d) = with(settings, a) {
                *settings = Arc::new(d);
            }
        }
        Op::Batch { ops } => ops.iter_mut().for_each(|o| patch_op(o, id, a)),
        _ => {}
    }
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
    /// Find the parts `parts` on photo `id`, waiting for the models (CLI, MCP, tests).
    pub fn segment_retouch(&mut self, id: PhotoId, parts: &[PartRequest]) -> Result<(Vec<RetouchMask>, Vec<FoundFace>), String> {
        let (people, face) = (self.segmenter.require_model("people")?, self.segmenter.require_model("face")?);
        #[cfg(feature = "sam")]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let kind = super::worker::Kind::Retouch { parts: parts.to_vec(), people, face };
            let job = self.segment_job(id, kind, super::Tag::Wait, tx, None)?;
            self.segmenter.worker.submit(job)?;
            match rx.recv_timeout(super::WAIT) {
                Ok(o) => {
                    o.result?;
                    match o.payload {
                        super::worker::Payload::Retouch(masks) => Ok((masks, o.faces.unwrap_or_default())),
                        _ => Err("the AI model returned nothing".into()),
                    }
                }
                Err(_) => Err("the AI model took too long; try again".into()),
            }
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = (id, parts, people, face);
            Err("AI masks are not available in this build".into())
        }
    }

    /// Background mode: queue the request `key`; [`Session::segment_poll`] applies it.
    fn segment_retouch_later(&mut self, key: &RetouchKey) -> Result<(), String> {
        let (people, face) = (self.segmenter.require_model("people")?, self.segmenter.require_model("face")?);
        #[cfg(feature = "sam")]
        {
            let kind = super::worker::Kind::Retouch { parts: key.parts.clone(), people, face };
            self.segment_queue_kind(key.photo, kind, super::Tag::Retouch { key: key.clone() })
        }
        #[cfg(not(feature = "sam"))]
        {
            let _ = (key, people, face);
            Err("AI masks are not available in this build".into())
        }
    }

    /// Give photo `id` the parts found (`a`): in its settings, and in every undo and redo step
    /// (and a slider drag's starting point) of the same orientation. Stored and written to the
    /// sidecar like any change, but without an undo step of its own.
    pub(crate) fn store_retouch_analysis(&mut self, id: PhotoId, a: &RetouchAnalysis) {
        for e in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            patch_op(&mut e.op, id, a);
        }
        if let Some(i) = self.interaction.as_mut().filter(|i| i.photo == id)
            && let Some(d) = with(&i.original, a)
        {
            i.original = Arc::new(d);
        }
        let Some(p) = self.catalog.photo(id) else { return };
        let Some(d) = with(&p.develop, a) else { return };
        let op = Op::SetDevelop { id, settings: Arc::new(d), label: "Find Retouch Parts".into(), edited: p.edited.clone() };
        match self.catalog.apply(op.clone()) {
            Ok(_) => self.pending_log.push(op),
            Err(e) => log::warn!("Retouch: could not store the parts found on photo {}: {e}", id.0),
        }
    }

    /// The parts photo `id`'s retouch sections want and its settings don't hold (`None`: none).
    pub(crate) fn retouch_wanted(&self, id: PhotoId) -> Option<RetouchKey> {
        let d = self.develop_of(id)?;
        let wanted = wanted(&d);
        let parts = match &d.retouch_analysis {
            Some(a) => a.missing(&wanted, d.orientation),
            None => wanted,
        };
        (!parts.is_empty()).then_some(RetouchKey { photo: id, parts, orientation: d.orientation })
    }

    /// Get photo `id` the parts its retouch sections want, if it lacks some: queued in background
    /// mode, waited for otherwise. `force` asks again after a failure. `Ok(true)`: they are there
    /// (or on their way); `Ok(false)`: nothing was wanted.
    pub(crate) fn retouch_ensure(&mut self, id: PhotoId, force: bool) -> Result<bool, String> {
        let Some(key) = self.retouch_wanted(id) else { return Ok(false) };
        if force {
            self.segmenter.retouch_failed.retain(|k| k.photo != id);
        }
        if self.segmenter.retouch_pending.contains(&key) {
            return Ok(true);
        }
        if self.segmenter.retouch_failed.contains(&key) {
            return Err("Retouch couldn't find these parts on this photo before; use Find Again to try again".into());
        }
        if self.segmenter.background {
            match self.segment_retouch_later(&key) {
                Ok(()) => {
                    self.segmenter.retouch_pending.push(key);
                    Ok(true)
                }
                Err(e) => {
                    self.segmenter.retouch_failed.push(key);
                    Err(e)
                }
            }
        } else {
            match self.segment_retouch(id, &key.parts) {
                Ok((masks, faces)) => {
                    self.store_retouch_analysis(id, &analysis_of(key.orientation, masks, &faces));
                    Ok(true)
                }
                Err(e) => {
                    self.segmenter.retouch_failed.push(key);
                    Err(e)
                }
            }
        }
    }

    /// After a command: every photo whose settings it set, and whose retouch wants parts it
    /// lacks, gets them. Failures are reported once, as messages.
    pub(crate) fn retouch_follow_up(&mut self, log_start: usize) {
        if !super::Segmenter::AVAILABLE {
            return;
        }
        let mut ids = Vec::new();
        for op in self.pending_log.get(log_start..).unwrap_or(&[]) {
            developed(op, &mut ids);
        }
        if let Some(i) = &self.interaction
            && !ids.contains(&i.photo)
        {
            ids.push(i.photo);
        }
        for id in ids {
            if self.retouch_wanted(id).is_none() {
                continue;
            }
            if let Err(e) = self.retouch_ensure(id, false)
                && !e.starts_with("Retouch couldn't")
            {
                log::warn!("Retouch: {e}");
                self.segmenter.push_message(format!("Retouch: {e}"));
            }
        }
    }

    /// A finished background retouch request.
    #[cfg(feature = "sam")]
    pub(crate) fn apply_retouch_outcome(
        &mut self,
        key: RetouchKey,
        result: Result<Option<lightcraft_develop::SegMask>, String>,
        payload: super::worker::Payload,
        faces: Option<Vec<FoundFace>>,
        polled: &mut super::Polled,
    ) {
        self.segmenter.retouch_pending.retain(|k| *k != key);
        let masks = match (result, payload) {
            (Err(e), _) => {
                self.segmenter.retouch_failed.push(key);
                return polled.messages.push(format!("Retouch: {e}"));
            }
            (Ok(_), super::worker::Payload::Retouch(masks)) => masks,
            _ => return,
        };
        let a = analysis_of(key.orientation, masks, &faces.unwrap_or_default());
        let r = self.execute_fn("retouch.found", |s| {
            s.store_retouch_analysis(key.photo, &a);
            Ok(serde_json::Value::Null)
        });
        match r {
            Ok(_) => polled.changed = true,
            Err(e) => polled.messages.push(e.to_string()),
        }
    }
}

/// A stored segmentation of a computed mask grid.
#[cfg(feature = "sam")]
pub(crate) fn seg_of(g: lightcraft_segment::people::MaskGrid) -> lightcraft_develop::SegMask {
    match g.rect {
        Some(r) => lightcraft_develop::SegMask::from_logits_in(g.side, &g.logits, r),
        None => lightcraft_develop::SegMask::from_logits(g.side, &g.logits),
    }
}
