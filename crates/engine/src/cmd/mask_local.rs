//! A mask's local Curve, Color Mixer and Point Color (Lightroom's Curve and Point Color sections under a
//! mask, and a Color Mixer Lightroom's masks don't have): the point curves and Refine Saturation, the 8
//! bands' Hue / Saturation / Luminance, and up to eight Point Color samples, each applied within the mask
//! where the global ones apply (`lightcraft_pipeline::finish::MaskLocal`).

use lightcraft_develop::{MAX_POINT_COLORS, MIXER_BANDS, Mask, Mixer, PointColor, ToneCurve};
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

/// The slider of a `mask.mixer` key (`<band>.<hue|sat|lum>`, as the global `mixer.<band>.<…>` controls).
fn mixer_field<'a>(m: &'a mut Mixer, key: &str) -> Option<&'a mut f64> {
    let (band, attr) = key.split_once('.')?;
    let b = m.band_mut(MIXER_BANDS.iter().position(|n| *n == band)?)?;
    Some(match attr {
        "hue" => &mut b.hue,
        "sat" => &mut b.sat,
        "lum" => &mut b.lum,
        _ => return None,
    })
}

/// A mask's Color Mixer as the commands report it: `{"<band>.<hue|sat|lum>": value}`.
pub fn mixer_json(m: &Mixer) -> Value {
    let mut o = serde_json::Map::new();
    for (band, b) in MIXER_BANDS.iter().zip(m.bands()) {
        for (attr, v) in [("hue", b.hue), ("sat", b.sat), ("lum", b.lum)] {
            o.insert(format!("{band}.{attr}"), json!(v));
        }
    }
    Value::Object(o)
}

/// `mask.mixer`'s `values` checked before anything changes: each key a band's slider, each value a
/// finite number (clamped to −100..100).
fn mixer_values(p: &Value, c: &str) -> Result<Vec<(String, f64)>> {
    let Some(v) = p.get("values") else { return Ok(Vec::new()) };
    let o = v.as_object().ok_or_else(|| bad(c, "`values` must be {\"<band>.<hue|sat|lum>\": number, …}"))?;
    let mut probe = Mixer::default();
    o.iter()
        .map(|(k, v)| {
            if mixer_field(&mut probe, k).is_none() {
                return Err(bad(c, format!("unknown slider `{k}` (<band>.<hue|sat|lum>, band one of {})", MIXER_BANDS.join(", "))));
            }
            let v = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(c, format!("`{k}` must be a number")))?;
            Ok((k.clone(), v.clamp(-100.0, 100.0)))
        })
        .collect()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "mask.mixer",
            "Set Mask Color Mixer",
            [],
            None,
            "{id?, values?: {\"<band>.<hue|sat|lum>\": −100..100, …} (band: red, orange, yellow, green, aqua, blue, purple, magenta), reset?: bool} — the mask's local Color Mixer (applied within the mask after the global colour tools, with the global mixer's maths); `reset` first sets every band back to 0 → {mixer}",
            has_active,
            |s, p| {
                let c = "mask.mixer";
                let vals = mixer_values(p, c)?;
                let reset = p.get("reset").and_then(Value::as_bool).unwrap_or(false);
                if vals.is_empty() && !reset {
                    return Err(bad(c, "give `values` or `reset`"));
                }
                edit_mask(s, p, c, "Mask Color Mixer", |m| {
                    if reset {
                        m.mixer = Mixer::default();
                    }
                    for (k, v) in &vals {
                        if let Some(f) = mixer_field(&mut m.mixer, k) {
                            *f = *v;
                        }
                    }
                    Ok(json!({"mixer": mixer_json(&m.mixer)}))
                })
            }
        ),
        cmd!(
            "mask.mixerTargeted",
            "Mask Color Mixer Targeted Adjustment",
            [],
            None,
            "{id?, target: hue|sat|lum, x, y (normalized image coords), delta (slider units)} — moves the mask's Color Mixer bands of the colour under the point (as `develop.targeted` does the global mixer's) → the changed sliders",
            has_active,
            |s, p| {
                let c = "mask.mixerTargeted";
                let (x, y, delta) = (f64_req(p, "x", c)?, f64_req(p, "y", c)?, f64_req(p, "delta", c)?);
                if !(x.is_finite() && y.is_finite() && delta.is_finite()) {
                    return Err(bad(c, "`x`, `y` and `delta` must be numbers"));
                }
                let target = str_param(p, "target").unwrap_or("hue").to_string();
                if !matches!(target.as_str(), "hue" | "sat" | "lum") {
                    return Err(bad(c, "target must be hue, sat or lum"));
                }
                let photo = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                let mid = p.get("id").and_then(Value::as_u64).map(|v| v.min(u64::from(u32::MAX)) as u32).or(s.active_mask);
                if !s.develop_of(photo).is_some_and(|d| d.masks.iter().any(|m| Some(m.id) == mid)) {
                    return Err(bad(c, "no mask selected (give `id`)"));
                }
                // the colour the masks' mixers see there: theirs and everything after them neutralized
                let e = probe(s, c, x, y, |d| {
                    local_point_neutral(d);
                    for m in &mut d.masks {
                        m.mixer = Mixer::default();
                    }
                })?;
                let [_, _, h] = encoded_to_oklch(e);
                let w = lightcraft_pipeline::colorops::band_weights(h);
                let top = w.iter().copied().fold(0.0f32, f32::max).max(1e-6);
                let delta = delta.clamp(-200.0, 200.0);
                edit_mask(s, p, c, "Mask Color Mixer", |m| {
                    let mut changed = serde_json::Map::new();
                    for (i, band) in MIXER_BANDS.iter().enumerate() {
                        let k = f64::from(w.get(i).copied().unwrap_or(0.0) / top);
                        if k < 0.05 {
                            continue;
                        }
                        let key = format!("{band}.{target}");
                        if let Some(f) = mixer_field(&mut m.mixer, &key) {
                            *f = (*f + delta * k).clamp(-100.0, 100.0);
                            changed.insert(key, json!(*f));
                        }
                    }
                    Ok(Value::Object(changed))
                })
            }
        ),
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
