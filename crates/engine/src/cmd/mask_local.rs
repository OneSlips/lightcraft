//! A mask's local Curve and Point Color (Lightroom's Curve and Point Color sections under a mask): the
//! point curves and Refine Saturation, and up to eight Point Color samples, each applied within the mask
//! where the global ones apply (`lightcraft_pipeline::finish::MaskLocal`).

use lightcraft_develop::{MAX_POINT_COLORS, Mask, PointColor, ToneCurve};
use lightcraft_geom::Point;
use serde_json::{Value, json};

use super::color::{encoded_to_oklch, probe};
use super::{CommandSpec, bad, cmd, f64_req, has_active, str_param};
use crate::{Result, Session};

/// Most points in one curve channel (a hostile list is cut there).
const MAX_CURVE_POINTS: usize = 64;

/// Run `f` on mask `id` (or the selected one) of the active photo as one undo step `label`.
fn edit_mask(s: &mut Session, p: &Value, c: &str, label: &str, f: impl FnOnce(&mut Mask) -> Result<Value>) -> Result<Value> {
    let photo = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let mid = p
        .get("id")
        .and_then(Value::as_u64)
        .map(|v| v.min(u64::from(u32::MAX)) as u32)
        .or(s.active_mask)
        .ok_or_else(|| bad(c, "no mask selected (give `id`)"))?;
    let mut d = (*s.develop_of(photo).unwrap_or_default()).clone();
    let m = d.masks.iter_mut().find(|m| m.id == mid).ok_or_else(|| bad(c, format!("no mask {mid}")))?;
    let out = f(m)?;
    s.set_develop(photo, d, label)?;
    Ok(out)
}

/// A curve channel's points from `[[x, y], …]` (0..1, sorted by x, at most [`MAX_CURVE_POINTS`]).
fn points(v: &Value, c: &str) -> Result<Vec<Point>> {
    let a = v.as_array().ok_or_else(|| bad(c, "`points` must be [[x, y], …]"))?;
    let mut pts: Vec<Point> = a
        .iter()
        .filter_map(|q| Some((q.get(0)?.as_f64()?, q.get(1)?.as_f64()?)))
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .take(MAX_CURVE_POINTS)
        .map(|(x, y)| Point::new(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        .collect();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x));
    Ok(pts)
}

fn channel<'a>(curve: &'a mut ToneCurve, ch: &str, c: &str) -> Result<&'a mut Vec<Point>> {
    Ok(match ch {
        "master" | "rgb" => &mut curve.master,
        "red" => &mut curve.red,
        "green" => &mut curve.green,
        "blue" => &mut curve.blue,
        other => return Err(bad(c, format!("unknown channel `{other}` (master, red, green or blue)"))),
    })
}

/// The Point Color fields a `values` object may set (the global sliders' names) and their ranges.
const POINT_FIELDS: [(&str, f64, f64); 8] = [
    ("hueShift", -100.0, 100.0),
    ("satShift", -100.0, 100.0),
    ("lumShift", -100.0, 100.0),
    ("variance", -100.0, 100.0),
    ("range", 0.0, 100.0),
    ("hueRange", 0.0, 100.0),
    ("satRange", 0.0, 100.0),
    ("lumRange", 0.0, 100.0),
];

fn point_field<'a>(p: &'a mut PointColor, key: &str) -> Option<&'a mut f64> {
    Some(match key {
        "hueShift" => &mut p.hue_shift,
        "satShift" => &mut p.sat_shift,
        "lumShift" => &mut p.lum_shift,
        "variance" => &mut p.variance,
        "range" => &mut p.range,
        "hueRange" => &mut p.hue_range,
        "satRange" => &mut p.sat_range,
        "lumRange" => &mut p.lum_range,
        _ => return None,
    })
}

/// A sample's values as the commands report them.
pub fn point_json(p: &PointColor) -> Value {
    json!({
        "lum": p.lum, "chroma": p.chroma, "hue": p.hue,
        "hueShift": p.hue_shift, "satShift": p.sat_shift, "lumShift": p.lum_shift, "variance": p.variance,
        "range": p.range, "hueRange": p.hue_range, "satRange": p.sat_range, "lumRange": p.lum_range,
    })
}

fn index(p: &Value, c: &str) -> Result<usize> {
    let i = f64_req(p, "index", c)?;
    if !(i.is_finite() && i >= 0.0) {
        return Err(bad(c, "`index` must be 0 or more"));
    }
    Ok(i as usize)
}

/// Everything after a mask's local Point Color, neutralized (the colour its samples see): the global
/// vignette, grain and curve, and every mask's local Curve and Point Color.
fn local_point_neutral(d: &mut lightcraft_develop::DevelopSettings) {
    d.vignette.amount = 0.0;
    d.grain.amount = 0.0;
    d.curve = Default::default();
    for m in &mut d.masks {
        m.curve = ToneCurve::default();
        m.point_colors.clear();
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "mask.curve",
            "Set Mask Curve",
            [],
            None,
            "{id?, channel?: master|red|green|blue, points?: [[x,y],…] in 0..1, refineSaturation?: 0..100, reset?: bool} — the mask's local tone curve (applied within the mask after the global curve); `reset` with a channel straightens that channel, without one the whole curve → {curve}",
            has_active,
            |s, p| {
                let c = "mask.curve";
                let ch = str_param(p, "channel").map(str::to_string);
                let pts = match p.get("points") {
                    Some(v) => Some(points(v, c)?),
                    None => None,
                };
                let refine = p.get("refineSaturation").and_then(Value::as_f64).filter(|v| v.is_finite());
                let reset = p.get("reset").and_then(Value::as_bool).unwrap_or(false);
                if pts.is_none() && refine.is_none() && !reset {
                    return Err(bad(c, "give `points` (with `channel`), `refineSaturation` or `reset`"));
                }
                edit_mask(s, p, c, "Mask Curve", |m| {
                    if reset {
                        match &ch {
                            Some(ch) => channel(&mut m.curve, ch, c)?.clear(),
                            None => m.curve = ToneCurve::default(),
                        }
                    }
                    if let Some(pts) = pts {
                        *channel(&mut m.curve, ch.as_deref().unwrap_or("master"), c)? =
                            if ToneCurve::point_curve_is_identity(&pts) { Vec::new() } else { pts };
                    }
                    if let Some(v) = refine {
                        m.curve.refine_saturation = v.clamp(0.0, 100.0);
                    }
                    Ok(json!({"curve": m.curve}))
                })
            }
        ),
        cmd!(
            "mask.pickPointColor",
            "Add Mask Point Color Sample",
            [],
            None,
            "{id?, x, y} normalized image coords — samples the colour there for the mask's local Point Color (max 8; the colour the mask's samples see: after the global colour tools) → {index, lum, chroma, hue}",
            has_active,
            |s, p| {
                let c = "mask.pickPointColor";
                let (x, y) = (f64_req(p, "x", c)?, f64_req(p, "y", c)?);
                if !(x.is_finite() && y.is_finite()) {
                    return Err(bad(c, "`x` and `y` must be numbers"));
                }
                let photo = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mid = p.get("id").and_then(Value::as_u64).map(|v| v.min(u64::from(u32::MAX)) as u32).or(s.active_mask);
                let full =
                    s.develop_of(photo).and_then(|d| d.masks.iter().find(|m| Some(m.id) == mid).map(|m| m.point_colors.len() >= MAX_POINT_COLORS));
                match full {
                    None => return Err(bad(c, "no mask selected (give `id`)")),
                    Some(true) => return Err(bad(c, format!("at most {MAX_POINT_COLORS} point colours per mask"))),
                    Some(false) => {}
                }
                let [l, ch, h] = encoded_to_oklch(probe(s, c, x, y, local_point_neutral)?);
                let sample = PointColor { lum: l as f64, chroma: ch as f64, hue: (h as f64).to_degrees().rem_euclid(360.0), ..Default::default() };
                edit_mask(s, p, c, "Mask Point Color", |m| {
                    m.point_colors.push(sample);
                    let i = m.point_colors.len() - 1;
                    Ok(json!({"index": i, "lum": sample.lum, "chroma": sample.chroma, "hue": sample.hue}))
                })
            }
        ),
        cmd!(
            "mask.setPointColor",
            "Set Mask Point Color",
            [],
            None,
            "{id?, index, values: {hueShift, satShift, lumShift, variance (−100..100), range, hueRange, satRange, lumRange (0..100)}} — one of the mask's Point Color samples → {sample}",
            has_active,
            |s, p| {
                let c = "mask.setPointColor";
                let i = index(p, c)?;
                let vals = p.get("values").and_then(Value::as_object).cloned().ok_or_else(|| bad(c, "missing `values`"))?;
                edit_mask(s, p, c, "Mask Point Color", |m| {
                    let n = m.point_colors.len();
                    let pc = m.point_colors.get_mut(i).ok_or_else(|| bad(c, format!("no sample {i} (the mask has {n})")))?;
                    for (k, v) in &vals {
                        let (_, lo, hi) = POINT_FIELDS.iter().find(|(f, ..)| f == k).ok_or_else(|| bad(c, format!("unknown field `{k}`")))?;
                        let v = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(c, format!("`{k}` must be a number")))?;
                        if let Some(f) = point_field(pc, k) {
                            *f = v.clamp(*lo, *hi);
                        }
                    }
                    Ok(json!({"sample": point_json(pc)}))
                })
            }
        ),
        cmd!(
            "mask.deletePointColor",
            "Delete Mask Point Color Sample",
            [],
            None,
            "{id?, index} — removes one of the mask's Point Color samples",
            has_active,
            |s, p| {
                let c = "mask.deletePointColor";
                let i = index(p, c)?;
                edit_mask(s, p, c, "Delete Mask Point Color", |m| {
                    if i >= m.point_colors.len() {
                        return Err(bad(c, format!("no sample {i}")));
                    }
                    m.point_colors.remove(i);
                    Ok(json!({"count": m.point_colors.len()}))
                })
            }
        ),
    ]
}
