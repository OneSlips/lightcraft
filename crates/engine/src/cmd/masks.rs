//! Masking and Remove (spot) commands on the active photo.

use lightcraft_develop::{BrushStroke, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, RedEye, Spot, SpotMode};
use lightcraft_geom::Point;
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, f64_or, has_active, ok, point, str_param};
use crate::segment::{AutoKind, Click, LANDSCAPE_CLASSES, MAX_CLICKS};
use crate::{Result, Session};

fn shape_from(kind: &str, p: &Value, c: &str) -> Result<MaskShape> {
    Ok(match kind {
        "brush" => MaskShape::Brush { strokes: vec![] },
        "linear" => {
            MaskShape::Linear { start: point(p, "start").unwrap_or(Point::new(0.5, 0.25)), end: point(p, "end").unwrap_or(Point::new(0.5, 0.6)) }
        }
        "radial" => MaskShape::Radial {
            center: point(p, "center").unwrap_or(Point::new(0.5, 0.5)),
            rx: f64_or(p, "rx", 0.22),
            ry: f64_or(p, "ry", 0.16),
            angle: f64_or(p, "angle", 0.0),
            feather: f64_or(p, "feather", 50.0),
            invert: bool_or(p, "invert", false),
        },
        "sky" => MaskShape::Sky { seg: seg_param(p) },
        "subject" => MaskShape::Subject { seg: seg_param(p) },
        "background" => MaskShape::Background { seg: seg_param(p) },
        "luminanceRange" => MaskShape::LuminanceRange {
            lo: f64_or(p, "lo", 0.6),
            hi: f64_or(p, "hi", 1.0),
            lo_feather: f64_or(p, "loFeather", 0.1),
            hi_feather: f64_or(p, "hiFeather", 0.1),
        },
        "colorRange" => {
            let samples: Vec<[f64; 3]> = p
                .get("samples")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|s| Some([s.get(0)?.as_f64()?, s.get(1)?.as_f64()?, s.get(2)?.as_f64()?])).collect())
                .unwrap_or_default();
            MaskShape::ColorRange { samples, refine: f64_or(p, "refine", 50.0), areas: Vec::new() }
        }
        "people" => MaskShape::People {
            who: who_param(p, c)?,
            person: p.get("person").and_then(Value::as_u64).map_or(0, |n| n.min(u64::from(u32::MAX)) as u32),
            parts: p
                .get("parts")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(16).collect())
                .unwrap_or_default(),
            seg: seg_param(p),
        },
        "landscape" => {
            let class = str_param(p, "class").unwrap_or_default();
            if !LANDSCAPE_CLASSES.contains(&class) {
                return Err(bad(c, format!("`class` must be one of {}", LANDSCAPE_CLASSES.join(", "))));
            }
            MaskShape::Landscape { class: class.to_string(), seg: seg_param(p) }
        }
        "concept" => {
            let name = str_param(p, "concept")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| bad(c, "missing `concept` (the remembered object's name)"))?;
            MaskShape::Concept { name: name.to_string(), seg: seg_param(p) }
        }
        "object" => MaskShape::Object { hint: points(p, "points"), exclude: points(p, "exclude"), seg: seg_param(p), detail: vec![], edge: 0.0 },
        "prompt" => {
            MaskShape::Prompt { text: str_param(p, "text").unwrap_or_default().trim().to_string(), seg: seg_param(p), detail: vec![], edge: 0.0 }
        }
        other => {
            return Err(bad(
                c,
                format!(
                    "unknown mask kind `{other}` (brush|linear|radial|sky|subject|background|people|landscape|concept|luminanceRange|colorRange|object|prompt)"
                ),
            ));
        }
    })
}

/// `who: {name} | {role}`: a People selection by name or by role in the shoot.
fn who_param(p: &Value, c: &str) -> Result<Option<lightcraft_develop::recipe::Who>> {
    match p.get("who") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let w: lightcraft_develop::recipe::Who =
                serde_json::from_value(v.clone()).map_err(|_| bad(c, "`who` must be {\"name\": …} or {\"role\": …}"))?;
            if w.text().trim().is_empty() || w.text().chars().count() > 200 {
                return Err(bad(c, "`who` needs a name or role"));
            }
            Ok(Some(w))
        }
    }
}

/// `anchor: "subject" | "people" | {"who": {…}}` (or `{to: …}`): a gradient placed in that
/// selection's box (its points then are 0..1 of the box).
fn anchor_param(p: &Value, shape: &MaskShape, c: &str) -> Result<Option<lightcraft_develop::recipe::Anchor>> {
    let Some(v) = p.get("anchor").filter(|v| !v.is_null()) else { return Ok(None) };
    if !matches!(shape, MaskShape::Linear { .. } | MaskShape::Radial { .. }) {
        return Err(bad(c, "only Linear and Radial gradients can be anchored"));
    }
    let to = v.get("to").unwrap_or(v);
    let to: lightcraft_develop::recipe::AnchorTo =
        serde_json::from_value(to.clone()).map_err(|_| bad(c, "`anchor` must be \"subject\", \"people\" or {\"who\": {\"role\": …}}"))?;
    Ok(Some(lightcraft_develop::recipe::Anchor { to, shape: shape.clone(), bounds: None }))
}

/// `[[x, y], …]` normalized points under `key`.
fn points(p: &Value, key: &str) -> Vec<Point> {
    p.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect())
        .unwrap_or_default()
}

/// A segmentation passed in (`seg`: a stored or replayed AI mask; the model isn't needed).
fn seg_param(p: &Value) -> Option<lightcraft_develop::SegMask> {
    p.get("seg").and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// What a new AI shape still needs once it is added.
enum Later {
    Nothing,
    /// Background mode: these Object clicks are segmented on the worker.
    Clicks(Vec<Point>, Vec<Point>),
    /// Background mode: the Describe selection is computed first; the mask is added when found.
    Text(String),
    /// Background mode: the automatic selection is computed on the worker and stored in the shape
    /// (the heuristic shows meanwhile).
    Auto(AutoKind),
    /// No model for this automatic selection: the heuristic stands in (the result says so, naming
    /// the model to get).
    Approximate(&'static str),
    /// A selection by name, role or remembered object, or an anchored gradient: computed as a
    /// recipe ([`Session::compute_masks`]).
    Recipe,
}

/// Compute the segmentation of a new AI shape (Object with clicks, Prompt) — or, in background
/// mode, say what to queue. An Object without clicks stays empty until the photo is clicked
/// (the photo is analyzed meanwhile); a Prompt that matches nothing is an error. Shapes given
/// with their `seg` need no model.
fn resolve_ai(s: &mut Session, shape: &mut MaskShape, c: &str) -> Result<Later> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let background = s.segmenter.background;
    match shape {
        MaskShape::Object { seg: Some(_), .. } | MaskShape::Prompt { seg: Some(_), .. } => {}
        MaskShape::Object { hint, exclude, .. } if !hint.is_empty() && background => {
            require_object_model(s)?;
            return Ok(Later::Clicks(std::mem::take(hint), std::mem::take(exclude)));
        }
        MaskShape::Object { hint, exclude, seg, .. } if !hint.is_empty() => {
            *seg = Some(segment_object(s, |s| s.segment_clicks(id, &clicks_of(hint, exclude)))?);
        }
        MaskShape::Object { .. } => {
            // the clicks come later: refuse an Object mask that could never be computed (no
            // model) rather than leave an empty one; in the app, analyze the photo meanwhile
            require_object_model(s)?;
            if background {
                segment_object(s, |s| s.segment_prepare(id))?;
            }
        }
        MaskShape::Prompt { text, .. } if background => {
            require_describe_model(s)?;
            if text.trim().is_empty() {
                return Err(ai_err("describe what to select"));
            }
            return Ok(Later::Text(text.clone()));
        }
        MaskShape::Prompt { text, seg, .. } => {
            let found = segment(s, |s| s.segment_text(id, text))?;
            *seg = Some(found.ok_or_else(|| ai_err(format!("nothing matching “{text}” was found in this photo")))?);
        }
        _ => {
            let Some(kind) = AutoKind::of(shape).filter(|k| k.seg_of(shape).is_some_and(Option::is_none)) else { return Ok(Later::Nothing) };
            if !crate::segment::Segmenter::AVAILABLE {
                return Ok(Later::Nothing);
            }
            if matches!(kind, AutoKind::People { who: Some(_), .. } | AutoKind::Concept { .. }) {
                return Ok(Later::Recipe);
            }
            match s.segmenter.auto_model(&kind) {
                Err(_) => return Ok(Later::Approximate(kind.preferred_model())),
                Ok(_) if background => return Ok(Later::Auto(kind)),
                Ok(_) => match s.segment_auto(id, &kind).map_err(ai_err)? {
                    Some(seg) => {
                        kind.set_seg(shape, Some(seg));
                    }
                    None => return Err(ai_err(not_found(&kind))),
                },
            }
        }
    }
    Ok(Later::Nothing)
}

/// What to say when an automatic selection finds nothing.
fn not_found(kind: &AutoKind) -> String {
    match kind {
        AutoKind::People { person, .. } if *person > 0 => format!("person {person} wasn't found in this photo"),
        AutoKind::People { .. } => "no people were found in this photo".into(),
        _ => "nothing of that kind was found in this photo".into(),
    }
}

/// Queue what a new AI component (component `comp` of mask `mask`) still needs.
fn queue_later(s: &mut Session, later: Later, mask: u32, comp: usize, mut out: Value) -> Result<Value> {
    match later {
        Later::Clicks(hint, exclude) => {
            let id = s.active().ok_or_else(|| ai_err("no active photo"))?;
            segment_object(s, |s| s.segment_clicks_later(id, mask, comp, hint, exclude))?;
            out["pending"] = json!(true);
        }
        Later::Auto(kind) => {
            let id = s.active().ok_or_else(|| ai_err("no active photo"))?;
            s.segment_auto_later(id, mask, comp, kind).map_err(ai_err)?;
            out["pending"] = json!(true);
        }
        Later::Approximate(model) => {
            out["approximate"] = json!(true);
            out["model"] = json!(model);
            out["note"] = json!(format!(
                "made by a heuristic: the {model} AI model isn't installed (ai.models.download {{\"id\": \"{model}\", \"acknowledged\": true}} after the user agrees)"
            ));
        }
        Later::Recipe => {
            let id = s.active().ok_or_else(|| ai_err("no active photo"))?;
            let r = s.compute_masks(&[id], false);
            if r.get("queued").and_then(Value::as_u64).unwrap_or(0) > 0 {
                out["pending"] = json!(true);
            }
            if let Some(n) = r.get("notes").filter(|n| n.as_array().is_some_and(|a| !a.is_empty())) {
                out["notes"] = n.clone();
            }
        }
        Later::Nothing | Later::Text(_) => {}
    }
    Ok(out)
}

/// AI mask failures are shown as they are (not as "invalid parameters").
fn ai_err(msg: impl Into<String>) -> crate::EngineError {
    crate::EngineError::Other(msg.into())
}

/// A Describe call: the models are checked first ([`crate::segment::Segmenter::describe_model`]:
/// SAM 3, else OWLv2 and SAM 2.1); a missing open model is an error naming it (its download is
/// offered on it), and without a folder for the open models SAM 3's
/// [`crate::EngineError::ModelMissing`] (the segmenter's own errors are text).
fn segment<T>(s: &mut Session, f: impl FnOnce(&mut Session) -> std::result::Result<T, String>) -> Result<T> {
    require_describe_model(s)?;
    f(s).map_err(ai_err)
}

/// The models for Describe are there (see [`segment`]).
fn require_describe_model(s: &Session) -> Result<()> {
    if let Err(e) = s.segmenter.describe_model() {
        if s.segmenter.models_root.is_none() {
            s.segmenter.require_sam3()?;
        }
        return Err(ai_err(e));
    }
    Ok(())
}

/// The model for Object clicks is there ([`crate::segment::Segmenter::object_model`]). Without a
/// folder for the openly licensed models only SAM 3 can be offered: then the error is
/// [`crate::EngineError::ModelMissing`], as for the other SAM 3 commands.
fn require_object_model(s: &Session) -> Result<()> {
    if let Err(e) = s.segmenter.object_model() {
        if s.segmenter.models_root.is_none() {
            s.segmenter.require_sam3()?;
        }
        return Err(ai_err(e));
    }
    Ok(())
}

/// A segmentation call for Object clicks (and their analysis and Refine): SAM 3 or the openly
/// licensed SAM 2.1 ([`crate::segment::Segmenter::object_model`]); without either, an error that
/// offers SAM 2.1.
fn segment_object<T>(s: &mut Session, f: impl FnOnce(&mut Session) -> std::result::Result<T, String>) -> Result<T> {
    require_object_model(s)?;
    f(s).map_err(ai_err)
}

pub(crate) fn clicks_of(include: &[Point], exclude: &[Point]) -> Vec<Click> {
    include.iter().map(|p| Click { at: *p, include: true }).chain(exclude.iter().map(|p| Click { at: *p, include: false })).collect()
}

fn masks_edit(s: &mut Session, c: &str, label: &str, f: impl FnOnce(&mut Vec<Mask>, &mut Option<u32>) -> Result<()>) -> Result<Value> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
    let before = d.masks.clone();
    let mut active = s.active_mask;
    f(&mut d.masks, &mut active)?;
    // automatic names follow what the masks select (until the photographer names one)
    s.refresh_mask_names(id, &before, &mut d.masks);
    s.set_develop(id, d, label)?;
    s.active_mask = active;
    Ok(json!({"activeMask": active}))
}

fn mask_id(p: &Value, active: Option<u32>, c: &str) -> Result<u32> {
    p.get("id").and_then(Value::as_u64).map(|v| v as u32).or(active).ok_or_else(|| bad(c, "no mask selected (give `id`)"))
}

fn find(masks: &mut [Mask], id: u32, c: &str) -> Result<usize> {
    masks.iter().position(|m| m.id == id).ok_or_else(|| bad(c, format!("no mask {id}")))
}

/// A colour range keeps at most this many colours (as Lightroom Classic).
const MAX_COLOR_SAMPLES: usize = 5;
/// The place of a colour sample picked before the pins existed.
const UNKNOWN_AREA: [f64; 4] = lightcraft_develop::UNKNOWN_COLOR_AREA;

/// The picked place: `rect: [x0, y0, x1, y1]` (an area dragged, in either direction) or `x, y` (a
/// click: a point), normalized; checked to lie on the photo.
fn area_param(p: &Value, c: &str) -> Result<[f64; 4]> {
    let area = match p.get("rect").and_then(Value::as_array) {
        Some(r) => {
            let v: Vec<f64> = r.iter().filter_map(Value::as_f64).collect();
            let [x0, y0, x1, y1] = v[..] else { return Err(bad(c, "rect is [x0, y0, x1, y1]")) };
            [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
        }
        None => {
            let (x, y) = (super::f64_req(p, "x", c)?, super::f64_req(p, "y", c)?);
            [x, y, x, y]
        }
    };
    let on_photo = |v: f64| (-0.01..=1.01).contains(&v);
    if !area.iter().all(|v| on_photo(*v)) {
        return Err(bad(c, "the point is outside the photo"));
    }
    Ok(area.map(|v| v.clamp(0.0, 1.0)))
}

/// The selected mask's colour range component (`comp`, else its first) and where its samples were
/// picked.
fn color_range_of(s: &Session, mid: u32, comp: Option<usize>, c: &str) -> Result<(usize, Vec<[f64; 4]>)> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let d = s.develop_of(id).unwrap_or_default();
    let m = d.masks.iter().find(|m| m.id == mid).ok_or_else(|| bad(c, format!("no mask {mid}")))?;
    let k = comp
        .or_else(|| m.components.iter().position(|x| matches!(x.shape, MaskShape::ColorRange { .. })))
        .ok_or_else(|| bad(c, "the mask has no colour range"))?;
    match m.components.get(k).map(|x| &x.shape) {
        Some(MaskShape::ColorRange { samples, areas, .. }) => {
            let mut areas = areas.clone();
            areas.resize(samples.len(), UNKNOWN_AREA);
            Ok((k, areas))
        }
        _ => Err(bad(c, "not a colour range component")),
    }
}

/// The active photo's pixels in `area` as the range pickers see them
/// ([`lightcraft_pipeline::range_area_pixels`], on a small render).
fn area_pixels(s: &mut Session, area: [f64; 4], c: &str) -> Result<Vec<([f64; 3], f64)>> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let src = s.source_now(id, crate::media::SourceLevel::Thumb).map_err(|e| bad(c, e))?;
    let info = s.source_info(id);
    let d = s.develop_of(id).unwrap_or_default();
    let req = lightcraft_pipeline::RenderRequest::fit(384, 384);
    lightcraft_pipeline::range_area_pixels(&src, &info, &d, &req, area).ok_or_else(|| bad(c, "the point is outside the photo"))
}

/// The colours to sample in `area`: a click, the mean round it; an area, up to `room` (at most 3)
/// of its main colours.
fn sample_area_colours(s: &mut Session, area: [f64; 4], room: usize, c: &str) -> Result<Vec<[f64; 3]>> {
    let px: Vec<[f64; 3]> = area_pixels(s, area, c)?.into_iter().map(|(lab, _)| lab).collect();
    let point = area[0] == area[2] && area[1] == area[3];
    let k = if point { 1 } else { room.clamp(1, 3) };
    let out = colour_clusters(&px, k);
    if out.is_empty() { Err(bad(c, "no colour there")) } else { Ok(out) }
}

/// Up to `k` main colours of `px` (OkLab): k-means from the mean and then the colours farthest from
/// those chosen; a cluster of under 10 % of the pixels is dropped (a few stray pixels at the edge
/// of a dragged area don't become a sample of their own). Deterministic.
pub(crate) fn colour_clusters(px: &[[f64; 3]], k: usize) -> Vec<[f64; 3]> {
    let dist = |a: &[f64; 3], b: &[f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
    let mean = |pts: &mut dyn Iterator<Item = &[f64; 3]>| {
        let (mut acc, mut n) = ([0.0f64; 3], 0usize);
        for p in pts {
            for i in 0..3 {
                acc[i] += p[i];
            }
            n += 1;
        }
        (n > 0).then(|| acc.map(|v| v / n as f64))
    };
    let Some(m) = mean(&mut px.iter()) else { return Vec::new() };
    let mut centres = vec![m];
    while centres.len() < k.max(1) {
        let far = px.iter().max_by(|a, b| {
            let da = centres.iter().map(|c| dist(a, c)).fold(f64::MAX, f64::min);
            let db = centres.iter().map(|c| dist(b, c)).fold(f64::MAX, f64::min);
            da.total_cmp(&db)
        });
        match far {
            Some(f) if centres.iter().all(|c| dist(f, c) > 1e-6) => centres.push(*f),
            _ => break,
        }
    }
    let labels_of = |centres: &[[f64; 3]]| -> Vec<usize> {
        px.iter().map(|p| centres.iter().enumerate().min_by(|a, b| dist(p, a.1).total_cmp(&dist(p, b.1))).map_or(0, |(i, _)| i)).collect()
    };
    for _ in 0..10 {
        let labels = labels_of(&centres);
        for (i, c) in centres.iter_mut().enumerate() {
            if let Some(m) = mean(&mut px.iter().zip(&labels).filter(|(_, l)| **l == i).map(|(p, _)| p)) {
                *c = m;
            }
        }
    }
    let labels = labels_of(&centres);
    let members: Vec<usize> = (0..centres.len()).map(|i| labels.iter().filter(|l| **l == i).count()).collect();
    let min = px.len() / 10;
    let mut out: Vec<([f64; 3], usize)> = centres.into_iter().zip(members).filter(|(_, n)| *n > 0 && *n >= min).collect();
    // the biggest first
    out.sort_by_key(|c| std::cmp::Reverse(c.1));
    out.into_iter().map(|(c, _)| c).collect()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "mask.add",
            "Create New Mask",
            [],
            None,
            "{kind: brush|linear|radial|sky|subject|background|people|landscape|luminanceRange|colorRange|object|prompt, ...shape params (start/end, center/rx/ry/angle/feather, lo/hi…; object: points/exclude [[x,y],…]; prompt: text; people: person? (0 everyone, n the n-th from the left), parts?; landscape: class sky|water|vegetation|mountains|architecture|naturalGround|artificialGround; AI kinds: seg? a stored segmentation, no model needed), name?} — Object needs SAM 3 or SAM 2.1, Describe SAM 3 or OWLv2 with SAM 2.1; Subject/Background/Sky/People/Landscape use their AI model when installed (BiRefNet, EoMT), else a heuristic (→ {approximate, model, note}); in the app AI kinds return {pending} and the selection follows",
            has_active,
            |s, p| {
                let kind = str_param(p, "kind").unwrap_or("radial").to_string();
                let mut shape = shape_from(&kind, p, "mask.add")?;
                let anchor = anchor_param(p, &shape, "mask.add")?;
                if let Some(a) = &anchor
                    && let Some(id) = s.active()
                    && let Some(placed) = a.placed(s.photo_aspect(id))
                {
                    shape = placed;
                }
                let mut later = resolve_ai(s, &mut shape, "mask.add")?;
                if anchor.is_some() {
                    later = Later::Recipe;
                }
                let name = str_param(p, "name").map(str::to_string);
                if let Later::Text(text) = &later {
                    let id = s.active().ok_or_else(|| bad("mask.add", "no active photo"))?;
                    segment(s, |s| s.segment_text_later(id, None, "add", name, text))?;
                    return Ok(json!({"pending": true}));
                }
                let next = s.active().and_then(|id| s.develop_of(id)).map(|d| d.next_mask_id()).unwrap_or(1);
                // the effect last applied to a mask of this kind
                let adjust = super::mask_effects::new_mask_adjust(s, &kind);
                let out = masks_edit(s, "mask.add", "Add Mask", |masks, active| {
                    masks.push(Mask {
                        id: next,
                        name: name.unwrap_or_else(|| format!("Mask {next}")),
                        components: vec![MaskComponent { anchor, name: None, op: MaskOp::Add, invert: false, shape }],
                        adjust,
                        ..Default::default()
                    });
                    *active = Some(next);
                    Ok(())
                })?;
                queue_later(s, later, next, 0, out)
            }
        ),
        cmd!(
            "mask.addComponent",
            "Add/Subtract/Intersect Mask",
            [],
            None,
            "{id?: maskId, op: add|subtract|intersect, kind, ...shape params}",
            has_active,
            |s, p| {
                let kind = str_param(p, "kind").unwrap_or("brush").to_string();
                let op: MaskOp =
                    serde_json::from_value(p.get("op").cloned().unwrap_or(json!("add"))).map_err(|e| bad("mask.addComponent", e.to_string()))?;
                let active = s.active_mask;
                let mid = mask_id(p, active, "mask.addComponent")?;
                let id = s.active().ok_or_else(|| bad("mask.addComponent", "no active photo"))?;
                if !s.develop_of(id).is_some_and(|d| d.masks.iter().any(|m| m.id == mid)) {
                    return Err(bad("mask.addComponent", format!("no mask {mid}")));
                }
                let mut shape = shape_from(&kind, p, "mask.addComponent")?;
                let anchor = anchor_param(p, &shape, "mask.addComponent")?;
                if let Some(a) = &anchor
                    && let Some(placed) = a.placed(s.photo_aspect(id))
                {
                    shape = placed;
                }
                let mut later = resolve_ai(s, &mut shape, "mask.addComponent")?;
                if anchor.is_some() {
                    later = Later::Recipe;
                }
                if let Later::Text(text) = &later {
                    let op = p.get("op").and_then(Value::as_str).unwrap_or("add");
                    segment(s, |s| s.segment_text_later(id, Some(mid), op, None, text))?;
                    return Ok(json!({"pending": true}));
                }
                let mut comp = 0;
                let out = masks_edit(s, "mask.addComponent", "Edit Mask", |masks, _| {
                    let i = find(masks, mid, "mask.addComponent")?;
                    masks[i].components.push(MaskComponent { anchor, name: None, op, invert: bool_or(p, "invert", false), shape });
                    comp = masks[i].components.len() - 1;
                    Ok(())
                })?;
                queue_later(s, later, mid, comp, out)
            }
        ),
        cmd!(
            "mask.brushStroke",
            "Brush Stroke",
            [],
            None,
            "{id?: maskId, points: [[x,y],…] normalized, size?: fraction of long edge (0.03), feather?, flow?, density?, erase?: bool, autoMask?: bool, pressure?: [p,…] pen pressure 0..1 per point (scales each dab's size and flow; omitted: full), continue?: bool (the stroke being painted, sent again as it grows: it replaces the mask's last stroke, so Auto Mask shows live)}",
            has_active,
            |s, p| {
                let pts: Vec<Point> = p
                    .get("points")
                    .and_then(Value::as_array)
                    .ok_or_else(|| bad("mask.brushStroke", "missing points"))?
                    .iter()
                    .filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?)))
                    .collect();
                let pts_len = pts.len();
                let d = BrushStroke::default();
                let stroke = BrushStroke {
                    points: pts,
                    size: f64_or(p, "size", d.size),
                    feather: f64_or(p, "feather", d.feather),
                    flow: f64_or(p, "flow", d.flow),
                    density: f64_or(p, "density", d.density),
                    erase: bool_or(p, "erase", false),
                    auto_mask: bool_or(p, "autoMask", false),
                    pressure: pressure_list(p.get("pressure"), pts_len)?,
                };
                let next = s.active().and_then(|id| s.develop_of(id)).map(|d| d.next_mask_id()).unwrap_or(1);
                let want = p.get("id").and_then(Value::as_u64).map(|v| v as u32).or(s.active_mask);
                let adjust = super::mask_effects::new_mask_adjust(s, "brush");
                let again = bool_or(p, "continue", false);
                masks_edit(s, "mask.brushStroke", "Brush", |masks, active| {
                    let i = match want.and_then(|m| masks.iter().position(|x| x.id == m)) {
                        Some(i) => i,
                        None => {
                            masks.push(Mask { id: next, name: format!("Mask {next}"), adjust, ..Default::default() });
                            *active = Some(next);
                            masks.len() - 1
                        }
                    };
                    let m = &mut masks[i];
                    if let Some(MaskComponent { shape: MaskShape::Brush { strokes }, .. }) =
                        m.components.iter_mut().rev().find(|c| matches!(c.shape, MaskShape::Brush { .. }))
                    {
                        match strokes.last_mut() {
                            Some(last) if again => *last = stroke,
                            _ => strokes.push(stroke),
                        }
                    } else {
                        m.components.push(MaskComponent {
                            anchor: None,
                            name: None,
                            op: MaskOp::Add,
                            invert: false,
                            shape: MaskShape::Brush { strokes: vec![stroke] },
                        });
                    }
                    Ok(())
                })
            }
        ),
        cmd!("mask.update", "Update Mask Shape", [], None, "{id?, component?: index (0), shape: MaskShape JSON}", has_active, |s, p| {
            let shape: MaskShape =
                serde_json::from_value(p.get("shape").cloned().unwrap_or_default()).map_err(|e| bad("mask.update", e.to_string()))?;
            let comp = p.get("component").and_then(Value::as_u64).unwrap_or(0) as usize;
            let mid = mask_id(p, s.active_mask, "mask.update")?;
            masks_edit(s, "mask.update", "Edit Mask", |masks, _| {
                let i = find(masks, mid, "mask.update")?;
                let c = masks[i].components.get_mut(comp).ok_or_else(|| bad("mask.update", "no such component"))?;
                c.shape = shape;
                Ok(())
            })
        }),
        cmd!(
            "mask.sampleColor",
            "Sample Mask Color",
            [],
            None,
            "{x, y: normalized image point | rect: [x0, y0, x1, y1] (an area dragged: up to 3 of its colours), add?: bool (⇧: add to the samples, max 5), id?, component?} — the colour range of the selected mask (its first colour-range component) samples the colour there; each picked place gets a pin (mask.colorPin) → {samples, pins}",
            has_active,
            |s, p| {
                let c = "mask.sampleColor";
                let area = area_param(p, c)?;
                let add = p.get("add").and_then(Value::as_bool).unwrap_or(false);
                let mid = mask_id(p, s.active_mask, c)?;
                let comp = p.get("component").and_then(Value::as_u64).map(|v| v as usize);
                let (k, current) = color_range_of(s, mid, comp, c)?;
                let room = if add { MAX_COLOR_SAMPLES.saturating_sub(current.len()) } else { MAX_COLOR_SAMPLES };
                if room == 0 {
                    return Err(bad(c, "a colour range has at most 5 samples: remove one first (⌥-click its pin)"));
                }
                let colours = sample_area_colours(s, area, room, c)?;
                let mut out = json!({});
                masks_edit(s, c, "Color Range", |masks, _| {
                    let i = find(masks, mid, c)?;
                    let Some(MaskShape::ColorRange { samples, areas, .. }) = masks[i].components.get_mut(k).map(|x| &mut x.shape) else {
                        return Err(bad(c, "not a colour range component"));
                    };
                    if !add {
                        samples.clear();
                        areas.clear();
                    }
                    // samples from before the pins have no place: an unknown one keeps them in step
                    areas.resize(samples.len(), UNKNOWN_AREA);
                    for lab in colours.iter().take(room) {
                        samples.push(*lab);
                        areas.push(area);
                    }
                    out = json!({"samples": samples.clone(), "pins": lightcraft_develop::color_range_pins(areas).len()});
                    Ok(())
                })?;
                Ok(out)
            }
        ),
        cmd!(
            "mask.colorPin",
            "Edit Color Range Sample",
            [],
            None,
            "{index: pin (0-based, in the order picked), remove?: bool (⌥-click), x?, y?: normalized point to move it to (the colours there are sampled again), id?, component?} — one picked place of the selected mask's colour range → {samples, pins}",
            has_active,
            |s, p| {
                let c = "mask.colorPin";
                let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(c, "index is required"))? as usize;
                let remove = p.get("remove").and_then(Value::as_bool).unwrap_or(false);
                let mid = mask_id(p, s.active_mask, c)?;
                let comp = p.get("component").and_then(Value::as_u64).map(|v| v as usize);
                let (k, current) = color_range_of(s, mid, comp, c)?;
                let pins = lightcraft_develop::color_range_pins(&current);
                let (area, range) =
                    pins.get(index).cloned().ok_or_else(|| bad(c, format!("no pin {index} (the colour range has {})", pins.len())))?;
                let moved = if remove {
                    None
                } else {
                    let (x, y) = (super::f64_req(p, "x", c)?, super::f64_req(p, "y", c)?);
                    // the same area, its centre at (x, y)
                    let (cx, cy) = ((area[0] + area[2]) / 2.0, (area[1] + area[3]) / 2.0);
                    let to = [area[0] + x - cx, area[1] + y - cy, area[2] + x - cx, area[3] + y - cy];
                    let colours = sample_area_colours(s, to, range.len().max(1), c)?;
                    Some((to, colours))
                };
                let mut out = json!({});
                masks_edit(s, c, "Color Range", |masks, _| {
                    let i = find(masks, mid, c)?;
                    let Some(MaskShape::ColorRange { samples, areas, .. }) = masks[i].components.get_mut(k).map(|x| &mut x.shape) else {
                        return Err(bad(c, "not a colour range component"));
                    };
                    areas.resize(samples.len(), UNKNOWN_AREA);
                    let (start, end) = (range.start.min(samples.len()), range.end.min(samples.len()));
                    let (new_samples, new_areas): (Vec<[f64; 3]>, Vec<[f64; 4]>) = match &moved {
                        Some((to, colours)) => (colours.clone(), vec![*to; colours.len()]),
                        None => (Vec::new(), Vec::new()),
                    };
                    samples.splice(start..end, new_samples);
                    areas.splice(start..end, new_areas);
                    out = json!({"samples": samples.clone(), "pins": lightcraft_develop::color_range_pins(areas).len()});
                    Ok(())
                })?;
                Ok(out)
            }
        ),
        cmd!(
            "mask.sampleLuminance",
            "Sample Mask Luminance",
            [],
            None,
            "{x, y: normalized image point | rect: [x0, y0, x1, y1] (an area dragged), add?: bool (⇧: widen the current range to take it in), id?, component?} — the luminance range of the selected mask (its first luminance-range component) selects the tones there → {lo, hi}",
            has_active,
            |s, p| {
                let c = "mask.sampleLuminance";
                let area = area_param(p, c)?;
                let add = p.get("add").and_then(Value::as_bool).unwrap_or(false);
                let mid = mask_id(p, s.active_mask, c)?;
                let comp = p.get("component").and_then(Value::as_u64).map(|v| v as usize);
                let px = area_pixels(s, area, c)?;
                let mut lum: Vec<f64> = px.iter().map(|(_, l)| *l).collect();
                lum.sort_by(f64::total_cmp);
                let at = |q: f64| lum.get(((lum.len() - 1) as f64 * q).round() as usize).copied().unwrap_or(0.5);
                // a click: a band round its tone; an area: the tones in it (5th to 95th percentile)
                let (mut lo, mut hi) = if area[0] == area[2] && area[1] == area[3] {
                    let m = at(0.5);
                    (m - 0.08, m + 0.08)
                } else {
                    (at(0.05), at(0.95))
                };
                let mut out = json!({});
                masks_edit(s, c, "Luminance Range", |masks, _| {
                    let i = find(masks, mid, c)?;
                    let k = comp
                        .or_else(|| masks[i].components.iter().position(|x| matches!(x.shape, MaskShape::LuminanceRange { .. })))
                        .ok_or_else(|| bad(c, "the mask has no luminance range"))?;
                    let Some(MaskShape::LuminanceRange { lo: l0, hi: h0, .. }) = masks[i].components.get_mut(k).map(|x| &mut x.shape) else {
                        return Err(bad(c, "not a luminance range component"));
                    };
                    if add {
                        lo = lo.min(*l0);
                        hi = hi.max(*h0);
                    }
                    *l0 = lo.clamp(0.0, 0.99);
                    *h0 = hi.clamp(*l0 + 0.01, 1.0);
                    out = json!({"lo": *l0, "hi": *h0});
                    Ok(())
                })?;
                Ok(out)
            }
        ),
        cmd!(
            "mask.objectPoint",
            "Add Object Click",
            [],
            None,
            "{x, y: normalized image point, exclude?: bool (⌥-click: leave this part out), id?: maskId} — a click on the selected mask's Object selection (its last Object component); SAM 3 re-segments the object → {include, exclude}",
            has_active,
            |s, p| {
                let c = "mask.objectPoint";
                let at = Point::new(super::f64_req(p, "x", c)?, super::f64_req(p, "y", c)?);
                if !(0.0..=1.0).contains(&at.x) || !(0.0..=1.0).contains(&at.y) {
                    return Err(bad(c, "the point is outside the photo"));
                }
                let exclude = bool_or(p, "exclude", false);
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mid = mask_id(p, s.active_mask, c)?;
                let d = s.develop_of(id).unwrap_or_default();
                let m = d.masks.iter().find(|m| m.id == mid).ok_or_else(|| bad(c, format!("no mask {mid}")))?;
                let k = m
                    .components
                    .iter()
                    .rposition(|x| matches!(x.shape, MaskShape::Object { .. }))
                    .ok_or_else(|| bad(c, "the mask has no Object selection (add one first)"))?;
                let Some(MaskShape::Object { hint, exclude: ex, edge, .. }) = m.components.get(k).map(|x| &x.shape) else {
                    return Err(bad(c, "not an Object component"));
                };
                let (mut hint, mut ex, edge) = (hint.clone(), ex.clone(), *edge);
                // clicks still on their way to the model count too
                if let Some(pending) = s.segmenter.pending_clicks().filter(|q| (q.photo, q.mask, q.comp) == (id, mid, k)) {
                    (hint, ex) = (pending.hint.clone(), pending.exclude.clone());
                }
                if hint.len() + ex.len() >= MAX_CLICKS {
                    return Err(ai_err(format!("an Object selection takes at most {MAX_CLICKS} clicks; start a new one")));
                }
                if exclude {
                    ex.push(at)
                } else {
                    hint.push(at)
                }
                if s.segmenter.background && !hint.is_empty() {
                    let out = json!({"include": hint.len(), "exclude": ex.len(), "pending": true});
                    segment_object(s, |s| s.segment_clicks_later(id, mid, k, hint, ex))?;
                    return Ok(out);
                }
                let seg = if hint.is_empty() { None } else { Some(segment_object(s, |s| s.segment_clicks(id, &clicks_of(&hint, &ex)))?) };
                let out = json!({"include": hint.len(), "exclude": ex.len()});
                masks_edit(s, c, "Object Mask", |masks, _| {
                    let i = find(masks, mid, c)?;
                    if let Some(comp) = masks[i].components.get_mut(k) {
                        comp.shape = MaskShape::Object { hint, exclude: ex, seg, detail: vec![], edge };
                    }
                    Ok(())
                })?;
                Ok(out)
            }
        ),
        cmd!(
            "mask.recompute",
            "Update AI Mask",
            [],
            None,
            "{id?: maskId, component?: index} — compute the mask's automatic AI selections (Subject, Background, Sky, People, Landscape) again with their model, e.g. after the photo changed or once the model is installed (all of them without `component`) → {updated, pending}",
            has_active,
            |s, p| {
                let c = "mask.recompute";
                let photo = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mid = mask_id(p, s.active_mask, c)?;
                let d = s.develop_of(photo).ok_or_else(|| bad(c, "no active photo"))?;
                let m = d.masks.iter().find(|m| m.id == mid).ok_or_else(|| bad(c, format!("no mask {mid}")))?;
                let wanted = p.get("component").and_then(Value::as_u64).map(|k| k as usize);
                let todo: Vec<(usize, AutoKind)> = m
                    .components
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| wanted.is_none_or(|w| w == *k))
                    .filter_map(|(k, comp)| AutoKind::of(&comp.shape).map(|kind| (k, kind)))
                    .collect();
                let recipe = todo.iter().any(|(_, k)| matches!(k, AutoKind::People { who: Some(_), .. } | AutoKind::Concept { .. }))
                    || m.components.iter().any(|c| c.anchor.is_some());
                if todo.is_empty() && !recipe {
                    return Err(bad(c, "the mask has no Subject, Background, Sky, People or Landscape selection"));
                }
                for (_, kind) in &todo {
                    s.segmenter.auto_model(kind).map_err(ai_err)?;
                }
                if recipe {
                    // by name, role or remembered object, or anchored: back to the recipe, then computed
                    let aspect = s.photo_aspect(photo);
                    let mut d = (*d).clone();
                    if let Some(m) = d.masks.iter_mut().find(|m| m.id == mid) {
                        for (k, comp) in m.components.iter_mut().enumerate() {
                            if wanted.is_none_or(|w| w == k) {
                                comp.shape = comp.shape.recipe();
                                if let Some(a) = &mut comp.anchor {
                                    a.bounds = None;
                                    if let Some(placed) = a.placed(aspect) {
                                        comp.shape = placed;
                                    }
                                }
                            }
                        }
                    }
                    s.set_develop(photo, d, "Update AI Mask")?;
                    let r = s.compute_masks(&[photo], false);
                    let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
                    return Ok(json!({"updated": n("updated"), "pending": n("queued"), "notes": r.get("notes")}));
                }
                // compute each (the app queues them: meanwhile the mask shows the heuristic)
                let mut d = (*d).clone();
                let mut updated = 0;
                let mut pending = 0;
                for (k, kind) in todo {
                    if s.segmenter.background {
                        if let Some(comp) = d.masks.iter_mut().find(|m| m.id == mid).and_then(|m| m.components.get_mut(k)) {
                            kind.set_seg(&mut comp.shape, None);
                        }
                        pending += 1;
                        continue;
                    }
                    let seg = s.segment_auto(photo, &kind).map_err(ai_err)?.ok_or_else(|| ai_err(not_found(&kind)))?;
                    if let Some(comp) = d.masks.iter_mut().find(|m| m.id == mid).and_then(|m| m.components.get_mut(k))
                        && kind.set_seg(&mut comp.shape, Some(seg))
                    {
                        updated += 1;
                    }
                }
                let queued: Vec<(usize, AutoKind)> = d
                    .masks
                    .iter()
                    .find(|m| m.id == mid)
                    .map(|m| {
                        m.components
                            .iter()
                            .enumerate()
                            .filter_map(|(k, comp)| {
                                AutoKind::of(&comp.shape)
                                    .filter(|kind| wanted.is_none_or(|w| w == k) && kind.seg_of(&comp.shape).is_some_and(Option::is_none))
                                    .map(|kind| (k, kind))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if updated > 0 || pending > 0 {
                    s.set_develop(photo, d, "Update AI Mask")?;
                }
                if s.segmenter.background {
                    for (k, kind) in queued {
                        s.segment_auto_later(photo, mid, k, kind).map_err(ai_err)?;
                    }
                }
                Ok(json!({"updated": updated, "pending": pending}))
            }
        ),
        cmd!(
            "mask.refineDetail",
            "Refine AI Mask Detail",
            [],
            None,
            "{id?: maskId, component?} — a zoomed-in AI pass over an Object/Describe selection (default: the mask's last one): the photo around it is analyzed again at a higher resolution, in the background; the mask updates when it's done → {started}",
            has_active,
            |s, p| {
                let c = "mask.refineDetail";
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mid = mask_id(p, s.active_mask, c)?;
                let d = s.develop_of(id).unwrap_or_default();
                let m = d.masks.iter().find(|m| m.id == mid).ok_or_else(|| bad(c, format!("no mask {mid}")))?;
                let k = match p.get("component").and_then(Value::as_u64) {
                    Some(k) => k as usize,
                    None => m
                        .components
                        .iter()
                        .rposition(|x| matches!(x.shape, MaskShape::Object { .. } | MaskShape::Prompt { .. }))
                        .ok_or_else(|| bad(c, "the mask has no Object or Describe selection"))?,
                };
                let started = segment_object(s, |s| s.segment_detail(id, mid, k))?;
                Ok(json!({"started": started}))
            }
        ),
        cmd!(
            query "segment.prepare",
            "Prepare AI Masks",
            [],
            None,
            "{} — load SAM 3 and analyze the active photo in the background, so Object clicks are instant → {busy}",
            has_active,
            |s, _| {
                let c = "segment.prepare";
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                segment_object(s, |s| s.segment_prepare(id))?;
                Ok(json!({"busy": s.segmenter.busy()}))
            }
        ),
        cmd!(
            query "segment.model.status",
            "AI Mask Model Status",
            [],
            None,
            "{} — whether this build has AI masks, whether the SAM 3 model is installed (and where), loaded, busy, and the download's progress → {available, installed, dir, loaded, busy, analyzing, sizeBytes, license, licenseUrl, mirrors, download: {running, done, total, file, error, finished}}",
            super::always,
            |s, _| {
                let g = &s.segmenter;
                Ok(json!({
                    "available": crate::segment::Segmenter::AVAILABLE,
                    "installed": g.installed(),
                    "dir": g.dir.as_ref().map(|d| d.display().to_string()),
                    "remote": g.remote_endpoint(),
                    "loaded": g.loaded(),
                    "busy": g.busy(),
                    "analyzing": g.analyzing(),
                    "sizeBytes": crate::segment::MODEL_BYTES,
                    "license": crate::segment::LICENSE_NAME,
                    "licenseUrl": crate::segment::LICENSE_URL,
                    "mirrors": g.mirrors().len(),
                    "download": g.download_status(),
                }))
            }
        ),
        cmd!(
            query "segment.model.download",
            "Download AI Mask Model",
            [],
            None,
            "{acknowledged: true} — download the SAM 3 model (about 3.4 GB, Meta's SAM License, not LightCraft's) in the background, from the configured mirrors; only after the user agreed to it. Watch segment.model.status; segment.model.cancel stops it (it resumes later) → {started, installed, downloading}",
            super::always,
            |s, p| {
                let c = "segment.model.download";
                if p.get("acknowledged").and_then(Value::as_bool) != Some(true) {
                    return Err(bad(
                        c,
                        format!(
                            "the SAM 3 model is a {:.1} GB download under Meta's {} ({}): pass `acknowledged: true` once the user has agreed to download it",
                            crate::segment::MODEL_BYTES as f64 / 1e9,
                            crate::segment::LICENSE_NAME,
                            crate::segment::LICENSE_URL
                        ),
                    ));
                }
                let started = s.segmenter.start_download(&s.activity).map_err(ai_err)?;
                Ok(json!({"started": started, "installed": s.segmenter.installed(), "downloading": s.segmenter.download_status().running}))
            }
        ),
        cmd!(
            query "segment.model.cancel",
            "Cancel AI Mask Model Download",
            [],
            None,
            "{} — stop the SAM 3 download (what has arrived is kept, and a new download resumes from it) → {cancelled}",
            super::always,
            |s, _| Ok(json!({"cancelled": s.segmenter.cancel_download()}))
        ),
        cmd!(
            query "ai.models.status",
            "AI Model Status",
            [],
            None,
            "{} — every AI mask model this build knows (subject, scene, sam2, depth, sam3): what it does, whether it is installed (and where), its size and licence, and its download's progress → {available, models: [{id, title, purpose, installed, dir, sizeBytes, license, licenseUrl, pageUrl, open, mirrors, download}]}",
            super::always,
            |s, _| Ok(json!({"available": crate::segment::Segmenter::AVAILABLE, "models": s.segmenter.model_status()}))
        ),
        cmd!(
            query "ai.models.download",
            "Download AI Model",
            [],
            None,
            "{id, acknowledged: true} — download AI model `id` (see ai.models.status for its size and licence) in the background; only after the user agreed to it. Watch ai.models.status; ai.models.cancel stops it (it resumes later) → {started, installed, downloading}",
            super::always,
            |s, p| {
                let c = "ai.models.download";
                let keys = || crate::segment::models::keys().join(", ");
                let id = str_param(p, "id").ok_or_else(|| bad(c, format!("which model? `id`: one of {}", keys())))?.to_string();
                let Some(m) = s.segmenter.model_status().into_iter().find(|m| m.id == id) else {
                    return Err(bad(c, format!("unknown AI model `{id}` (one of {})", keys())));
                };
                if p.get("acknowledged").and_then(Value::as_bool) != Some(true) {
                    return Err(bad(
                        c,
                        format!(
                            "the {} model is a {:.0} MB download under the {} licence ({}): pass `acknowledged: true` once the user has agreed to download it",
                            m.title,
                            m.size_bytes as f64 / 1e6,
                            m.license,
                            m.license_url
                        ),
                    ));
                }
                let activity = s.activity.clone();
                let started = s.segmenter.start_model_download(&id, &activity).map_err(ai_err)?;
                Ok(json!({"started": started, "installed": s.segmenter.model_installed(&id), "downloading": s.segmenter.model_download_status(&id).running}))
            }
        ),
        cmd!(
            query "ai.models.cancel",
            "Cancel AI Model Download",
            [],
            None,
            "{id} — stop AI model `id`'s download (what has arrived is kept, and a new download resumes from it) → {cancelled}",
            super::always,
            |s, p| {
                let id = str_param(p, "id").ok_or_else(|| bad("ai.models.cancel", "which model? `id`"))?;
                Ok(json!({"cancelled": s.segmenter.cancel_model_download(id)}))
            }
        ),
        cmd!(
            "mask.refine",
            "Refine Mask Edges",
            [],
            None,
            "{id?, value: 0..100} — the mask's edges snap to the photo's (guided filter; rendered on the CPU)",
            has_active,
            |s, p| {
                let v = f64_or(p, "value", 0.0).clamp(0.0, 100.0);
                let mid = mask_id(p, s.active_mask, "mask.refine")?;
                masks_edit(s, "mask.refine", "Refine Edges", |masks, _| {
                    let i = find(masks, mid, "mask.refine")?;
                    masks[i].refine = v;
                    Ok(())
                })
            }
        ),
        cmd!(
            "mask.adjust",
            "Set Mask Adjustments",
            [],
            None,
            "{id?, values: {exposure, contrast, highlights, shadows, whites, blacks, temp, tint, texture, clarity, dehaze, hue, saturation, sharpness, noise, moire, defringe, color_hue, color_sat, amount}}",
            has_active,
            |s, p| {
                let vals = p.get("values").cloned().ok_or_else(|| bad("mask.adjust", "missing values"))?;
                let mid = mask_id(p, s.active_mask, "mask.adjust")?;
                masks_edit(s, "mask.adjust", "Mask Adjustment", |masks, _| {
                    let i = find(masks, mid, "mask.adjust")?;
                    let mut v = serde_json::to_value(masks[i].adjust).unwrap_or_default();
                    lightcraft_develop::presets::deep_merge(&mut v, &vals);
                    let a: LocalAdjustments = serde_json::from_value(v).map_err(|e| bad("mask.adjust", e.to_string()))?;
                    masks[i].adjust = clamp_local(a);
                    Ok(())
                })
            }
        ),
        cmd!("mask.select", "Select Mask", [], None, "{id|null}", has_active, |s, p| {
            s.active_mask = p.get("id").and_then(Value::as_u64).map(|v| v as u32);
            ok()
        }),
        cmd!("mask.delete", "Delete Mask", [], None, "{id?}", has_active, |s, p| {
            let mid = mask_id(p, s.active_mask, "mask.delete")?;
            masks_edit(s, "mask.delete", "Delete Mask", |masks, active| {
                let i = find(masks, mid, "mask.delete")?;
                masks.remove(i);
                *active = masks.last().map(|m| m.id);
                Ok(())
            })
        }),
        cmd!("mask.deleteAll", "Delete All Masks", [], None, "{}", has_active, |s, _| {
            masks_edit(s, "mask.deleteAll", "Delete All Masks", |masks, active| {
                masks.clear();
                *active = None;
                Ok(())
            })
        }),
        cmd!("mask.rename", "Rename Mask", [], None, "{id?, name}", has_active, |s, p| {
            let name = str_param(p, "name").ok_or_else(|| bad("mask.rename", "missing name"))?.to_string();
            let mid = mask_id(p, s.active_mask, "mask.rename")?;
            masks_edit(s, "mask.rename", "Rename Mask", |masks, _| {
                let i = find(masks, mid, "mask.rename")?;
                masks[i].name = name;
                Ok(())
            })
        }),
        cmd!("mask.invert", "Invert Mask", [], None, "{id?}", has_active, |s, p| {
            let mid = mask_id(p, s.active_mask, "mask.invert")?;
            masks_edit(s, "mask.invert", "Invert Mask", |masks, _| {
                let i = find(masks, mid, "mask.invert")?;
                masks[i].invert = !masks[i].invert;
                Ok(())
            })
        }),
        cmd!("mask.visible", "Show/Hide Mask", [], None, "{id?, visible?: bool}", has_active, |s, p| {
            let mid = mask_id(p, s.active_mask, "mask.visible")?;
            masks_edit(s, "mask.visible", "Toggle Mask", |masks, _| {
                let i = find(masks, mid, "mask.visible")?;
                masks[i].visible = bool_or(p, "visible", !masks[i].visible);
                Ok(())
            })
        }),
        cmd!(
            "mask.duplicate",
            "Duplicate Mask",
            [],
            None,
            "{id?, invert?: bool (Duplicate and Invert)} — the copy goes right after the original and is selected",
            has_active,
            |s, p| {
                let mid = mask_id(p, s.active_mask, "mask.duplicate")?;
                let invert = bool_or(p, "invert", false);
                let next = s.active().and_then(|id| s.develop_of(id)).map(|d| d.next_mask_id()).unwrap_or(1);
                let label = if invert { "Duplicate and Invert Mask" } else { "Duplicate Mask" };
                masks_edit(s, "mask.duplicate", label, |masks, active| {
                    let i = find(masks, mid, "mask.duplicate")?;
                    let mut m = masks[i].clone();
                    m.id = next;
                    m.name = format!("{} copy", m.name);
                    if invert {
                        m.invert = !m.invert;
                    }
                    masks.insert(i + 1, m);
                    *active = Some(next);
                    Ok(())
                })
            }
        ),
        cmd!(
            "mask.component",
            "Edit Mask Component",
            [],
            None,
            "{id?, component: index, action: invert|duplicate|delete|rename|op, name? (rename; empty clears), op?: add|subtract|intersect} — one component of a mask (deleting the last one deletes the mask)",
            has_active,
            |s, p| {
                let c = "mask.component";
                let mid = mask_id(p, s.active_mask, c)?;
                let k = p.get("component").and_then(Value::as_u64).ok_or_else(|| bad(c, "missing component"))? as usize;
                let action = str_param(p, "action").ok_or_else(|| bad(c, "missing action"))?.to_string();
                let op: Option<MaskOp> = match p.get("op") {
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| bad(c, e.to_string()))?),
                    None => None,
                };
                let label = match action.as_str() {
                    "invert" => "Invert Component",
                    "duplicate" => "Duplicate Component",
                    "delete" => "Delete Component",
                    "rename" => "Rename Component",
                    "op" => "Change Component Mode",
                    a => return Err(bad(c, format!("unknown action {a:?} (invert|duplicate|delete|rename|op)"))),
                };
                masks_edit(s, c, label, |masks, active| {
                    let i = find(masks, mid, c)?;
                    let comps = &mut masks[i].components;
                    let comp = comps.get_mut(k).ok_or_else(|| bad(c, "no such component"))?;
                    match action.as_str() {
                        "invert" => comp.invert = !comp.invert,
                        "rename" => comp.name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string),
                        "op" => comp.op = op.ok_or_else(|| bad(c, "missing op"))?,
                        "duplicate" => {
                            let copy = comp.clone();
                            comps.insert(k + 1, copy);
                        }
                        _ => {
                            comps.remove(k);
                            if comps.is_empty() {
                                masks.remove(i);
                                *active = masks.last().map(|m| m.id);
                            }
                        }
                    }
                    Ok(())
                })
            }
        ),
        cmd!("mask.move", "Move Mask", [], None, "{id?, to?: index (0 = top), delta?: ±n} — reorder the masks list", has_active, |s, p| {
            let mid = mask_id(p, s.active_mask, "mask.move")?;
            let mut masks = s.active().and_then(|id| s.develop_of(id)).map(|d| d.masks.clone()).unwrap_or_default();
            let i = find(&mut masks, mid, "mask.move")?;
            let to = match (p.get("to").and_then(Value::as_i64), p.get("delta").and_then(Value::as_i64)) {
                (Some(t), _) => t,
                // (an agent's delta can be anything: saturate rather than overflow)
                (None, Some(d)) => (i as i64).saturating_add(d),
                (None, None) => return Err(bad("mask.move", "give `to` (index) or `delta`")),
            }
            .clamp(0, masks.len() as i64 - 1) as usize;
            if to == i {
                return Ok(json!({"activeMask": s.active_mask}));
            }
            masks_edit(s, "mask.move", "Reorder Masks", |masks, _| {
                let i = find(masks, mid, "mask.move")?;
                let m = masks.remove(i);
                masks.insert(to, m);
                Ok(())
            })
        }),
        // ---- Remove tool (spots)
        cmd!(
            "spot.add",
            "Add Remove Spot",
            [],
            None,
            "{mode?: remove|heal|clone|fill (content-aware), points: [[x,y],…], size?: fraction of long edge, feather?, opacity?, source?: [dx,dy] (default: the best match nearby)} — selects the new spot; returns {index}",
            has_active,
            |s, p| {
                let mode: SpotMode =
                    serde_json::from_value(p.get("mode").cloned().unwrap_or(json!("remove"))).map_err(|e| bad("spot.add", e.to_string()))?;
                let pts: Vec<Point> = p
                    .get("points")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect())
                    .unwrap_or_default();
                if pts.is_empty() {
                    return Err(bad("spot.add", "missing points"));
                }
                let d = Spot::default();
                let mut spot = Spot {
                    mode,
                    points: pts,
                    size: f64_or(p, "size", d.size).clamp(SPOT_SIZE.0, SPOT_SIZE.1),
                    feather: f64_or(p, "feather", d.feather).clamp(0.0, 100.0),
                    opacity: f64_or(p, "opacity", d.opacity).clamp(0.0, 100.0),
                    source_offset: point(p, "source"),
                    auto: None,
                    generated: None,
                };
                let id = s.active().ok_or_else(|| bad("spot.add", "no active photo"))?;
                let mut dd = (*s.develop_of(id).unwrap_or_default()).clone();
                if spot.source_offset.is_none() && !matches!(spot.mode, SpotMode::Fill | SpotMode::Generate) {
                    // resolve the automatic source now: it gets a pin and stays put at every size
                    spot.source_offset = pick_source(s, id, &dd, &spot, None);
                }
                dd.spots.push(spot);
                let n = dd.spots.len();
                s.set_develop(id, dd, "Remove")?;
                s.active_spot = Some(n - 1);
                Ok(json!({"index": n - 1}))
            }
        ),
        cmd!("spot.select", "Select Spot", [], None, "{index|null}", has_active, |s, p| {
            let n = spots_len(s);
            s.active_spot = match p.get("index").and_then(Value::as_u64) {
                Some(i) if (i as usize) < n => Some(i as usize),
                Some(_) => return Err(bad("spot.select", "no such spot")),
                None => None,
            };
            Ok(json!({"activeSpot": s.active_spot}))
        }),
        cmd!(
            "spot.update",
            "Edit Spot",
            [],
            None,
            "{index? (the selected spot), move?: [dx,dy] (the target, normalized), source?: [dx,dy] (offset from the target), moveSource?: [dx,dy], size?, feather?, opacity?, mode?: remove|heal|clone|fill|flat} — moving or resizing a spot Remove Dust added makes it an ordinary spot",
            has_active,
            |s, p| {
                let c = "spot.update";
                let i = spot_index(s, p, c)?;
                let mode: Option<SpotMode> = match p.get("mode") {
                    Some(m) => Some(serde_json::from_value(m.clone()).map_err(|e| bad(c, e.to_string()))?),
                    None => None,
                };
                spots_edit(s, c, "Edit Spot", |spots| {
                    let sp = &mut spots[i];
                    if let Some(d) = point(p, "move") {
                        sp.points.iter_mut().for_each(|q| *q = Point::new(q.x + d.x, q.y + d.y));
                        // the source moves along (its offset is relative to the target)
                    }
                    if let Some(o) = point(p, "source") {
                        sp.source_offset = Some(o);
                    }
                    if let Some(d) = point(p, "moveSource") {
                        let o = sp.source_offset.unwrap_or(Point::new(0.0, 0.0));
                        sp.source_offset = Some(Point::new(o.x + d.x, o.y + d.y));
                    }
                    if let Some(v) = p.get("size").and_then(Value::as_f64) {
                        sp.size = v.clamp(SPOT_SIZE.0, SPOT_SIZE.1);
                    }
                    if let Some(v) = p.get("feather").and_then(Value::as_f64) {
                        sp.feather = v.clamp(0.0, 100.0);
                    }
                    if let Some(v) = p.get("opacity").and_then(Value::as_f64) {
                        sp.opacity = v.clamp(0.0, 100.0);
                    }
                    if let Some(m) = mode {
                        sp.mode = m;
                    }
                    // a spot Remove Dust added that the user moves or resizes becomes theirs (a later Remove Dust keeps it)
                    if p.get("move").is_some() || p.get("size").is_some() {
                        sp.auto = None;
                    }
                    Ok(())
                })?;
                Ok(json!({"index": i}))
            }
        ),
        cmd!(
            "spot.refreshSource",
            "Refresh Source",
            [],
            None,
            "{index? (the selected spot)} — picks the next-best source away from the current one",
            has_active,
            |s, p| {
                let c = "spot.refreshSource";
                let i = spot_index(s, p, c)?;
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let dd = (*s.develop_of(id).unwrap_or_default()).clone();
                let spot = dd.spots[i].clone();
                let next = pick_source(s, id, &dd, &spot, spot.source_offset).ok_or_else(|| bad(c, "no other source fits in the photo"))?;
                spots_edit(s, c, "Refresh Source", |spots| {
                    spots[i].source_offset = Some(next);
                    Ok(())
                })?;
                Ok(json!({"index": i, "source": [next.x, next.y]}))
            }
        ),
        cmd!("spot.delete", "Delete Spot", [], None, "{index? (the selected spot)}", has_active, |s, p| {
            let c = "spot.delete";
            let i = spot_index(s, p, c)?;
            spots_edit(s, c, "Delete Spot", |spots| {
                spots.remove(i);
                Ok(())
            })?;
            s.active_spot = match s.active_spot {
                Some(a) if a == i => None,
                Some(a) if a > i => Some(a - 1),
                a => a,
            };
            ok()
        }),
        // ---- Red eye / pet eye
        cmd!(
            "redeye.add",
            "Add Red Eye Correction",
            [],
            None,
            "{center: [x,y] normalized, rx, ry: radii as fractions of the long edge, pet?: bool, pupilSize?: 0..100, darken?: 0..100} — the pupil inside is found automatically; returns {index}",
            has_active,
            |s, p| {
                let c = "redeye.add";
                let center = point(p, "center").ok_or_else(|| bad(c, "missing center"))?;
                let d = RedEye::default();
                let eye = RedEye {
                    center,
                    rx: f64_or(p, "rx", d.rx).clamp(1e-4, 0.5),
                    ry: f64_or(p, "ry", d.ry).clamp(1e-4, 0.5),
                    pupil_size: f64_or(p, "pupilSize", d.pupil_size).clamp(0.0, 100.0),
                    darken: f64_or(p, "darken", d.darken).clamp(0.0, 100.0),
                    pet: bool_or(p, "pet", false),
                    catchlight: None,
                };
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mut dd = (*s.develop_of(id).unwrap_or_default()).clone();
                dd.red_eye.push(eye);
                let n = dd.red_eye.len();
                s.set_develop(id, dd, if eye.pet { "Pet Eye" } else { "Red Eye" })?;
                Ok(json!({"index": n - 1}))
            }
        ),
        cmd!(
            "redeye.catchlight",
            "Pet Eye Catchlight",
            [],
            None,
            "{index, on?: bool (default true), offset?: [dx, dy] from the pupil centre in pupil radii (default [-0.35, -0.35])}",
            has_active,
            |s, p| {
                let c = "redeye.catchlight";
                let i = super::index_req(p, "index", c)?;
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mut dd = (*s.develop_of(id).unwrap_or_default()).clone();
                let eye = dd.red_eye.get_mut(i).ok_or_else(|| bad(c, "no such eye"))?;
                if !eye.pet {
                    return Err(bad(c, "catchlights are for pet eyes"));
                }
                let off = point(p, "offset").or(eye.catchlight).unwrap_or(Point::new(-0.35, -0.35));
                eye.catchlight = bool_or(p, "on", true).then(|| Point::new(off.x.clamp(-1.0, 1.0), off.y.clamp(-1.0, 1.0)));
                s.set_develop(id, dd, "Catchlight")?;
                ok()
            }
        ),
        cmd!("redeye.delete", "Delete Red Eye Correction", [], None, "{index}", has_active, |s, p| {
            let c = "redeye.delete";
            let i = super::index_req(p, "index", c)?;
            let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
            let mut dd = (*s.develop_of(id).unwrap_or_default()).clone();
            if i >= dd.red_eye.len() {
                return Err(bad(c, "no such eye"));
            }
            dd.red_eye.remove(i);
            s.set_develop(id, dd, "Delete Red Eye")?;
            ok()
        }),
    ]
}

/// Spot radius limits (fraction of the long edge).
pub const SPOT_SIZE: (f64, f64) = (0.001, 0.25);

fn spots_len(s: &Session) -> usize {
    s.active().and_then(|id| s.develop_of(id)).map(|d| d.spots.len()).unwrap_or(0)
}

/// `index` from the params, else the selected spot.
fn spot_index(s: &Session, p: &Value, c: &str) -> Result<usize> {
    let i = p.get("index").and_then(Value::as_u64).map(|v| v as usize).or(s.active_spot).ok_or_else(|| bad(c, "no spot selected (give `index`)"))?;
    if i >= spots_len(s) {
        return Err(bad(c, "no such spot"));
    }
    Ok(i)
}

fn spots_edit(s: &mut Session, c: &str, label: &str, f: impl FnOnce(&mut Vec<Spot>) -> Result<()>) -> Result<()> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
    f(&mut d.spots)?;
    s.set_develop(id, d, label)
}

/// An automatic source for `spot` on photo `id` (a small proxy of the photo, framed by `d`).
pub(super) fn pick_source(
    s: &mut Session,
    id: crate::PhotoId,
    d: &lightcraft_develop::DevelopSettings,
    spot: &Spot,
    avoid: Option<Point>,
) -> Option<Point> {
    let src = s.source_now(id, crate::media::SourceLevel::Thumb).ok()?;
    let info = s.source_info(id);
    lightcraft_pipeline::spots::pick_source(&src, &info, d, spot, avoid)
}

pub(crate) fn clamp_local(mut a: LocalAdjustments) -> LocalAdjustments {
    let c = |v: &mut f64, lo: f64, hi: f64| *v = if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
    c(&mut a.exposure, -4.0, 4.0);
    for v in [
        &mut a.temp,
        &mut a.tint,
        &mut a.contrast,
        &mut a.highlights,
        &mut a.shadows,
        &mut a.whites,
        &mut a.blacks,
        &mut a.texture,
        &mut a.clarity,
        &mut a.dehaze,
        &mut a.hue,
        &mut a.saturation,
        &mut a.sharpness,
        &mut a.noise,
        &mut a.moire,
        &mut a.defringe,
    ] {
        c(v, -100.0, 100.0);
    }
    c(&mut a.color_hue, 0.0, 360.0);
    c(&mut a.color_sat, 0.0, 100.0);
    c(&mut a.amount, 0.0, 200.0);
    a
}

/// `mask.brushStroke`'s `pressure`: one number 0..1 per point (clamped), at most one per point; absent or `null`
/// is no pressure data.
fn pressure_list(v: Option<&Value>, points: usize) -> Result<Vec<f32>> {
    let Some(v) = v.filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| bad("mask.brushStroke", "pressure must be a list of numbers 0..1"))?;
    if list.len() > points {
        return Err(bad("mask.brushStroke", "pressure has more values than points"));
    }
    list.iter()
        .map(|q| q.as_f64().filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0) as f32))
        .collect::<Option<Vec<f32>>>()
        .ok_or_else(|| bad("mask.brushStroke", "pressure must be a list of numbers 0..1"))
}

#[cfg(test)]
mod pick_tests {
    use super::*;

    #[test]
    fn an_area_gives_its_main_colours_biggest_first() {
        // 70 % one colour, 30 % another, a few stray pixels of a third
        let (a, b, c) = ([0.8, 0.0, 0.0], [0.3, 0.1, -0.1], [0.5, -0.2, 0.2]);
        let mut px = vec![a; 70];
        px.extend(vec![b; 30]);
        px.extend(vec![c; 3]);
        let out = colour_clusters(&px, 3);
        assert_eq!(out.len(), 2, "the stray pixels make no sample: {out:?}");
        assert!(out[0].iter().zip(a).all(|(x, y)| (x - y).abs() < 0.02), "{out:?}");
        assert!(out[1].iter().zip(b).all(|(x, y)| (x - y).abs() < 0.03), "{out:?}");
        // one colour asked: the mean; nothing given: nothing
        assert_eq!(colour_clusters(&[a, a], 1).len(), 1);
        assert!(colour_clusters(&[], 3).is_empty());
        // a single colour never splits
        assert_eq!(colour_clusters(&vec![a; 50], 3).len(), 1);
    }

    #[test]
    fn colour_pins_group_the_samples_of_one_place() {
        let p = [0.2, 0.2, 0.2, 0.2];
        let area = [0.1, 0.1, 0.4, 0.3];
        let pins = lightcraft_develop::color_range_pins(&[UNKNOWN_AREA, p, area, area, area, [f64::NAN; 4]]);
        assert_eq!(pins, vec![(p, 1..2), (area, 2..5)]);
    }
}
