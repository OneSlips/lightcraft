//! `concepts.*`: remembered objects. Select a thing on a photo (any mask: an Object click, a
//! brush, People ▸ Clothes…) and name it ("Wedding dress"); masks and presets then find it on
//! other photos (`mask.add {kind: "concept", concept: "Wedding dress"}`). Learning and finding run
//! on SAM 2.1 (open model) on the AI worker; what is learned stays in the library
//! (`concepts.json`).

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, str_param};
use crate::{Result, Session};

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn name_param(p: &Value, key: &str, c: &str) -> Result<String> {
    let n = str_param(p, key).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, format!("missing `{key}`")))?;
    if n.chars().count() > 120 {
        return Err(bad(c, format!("`{key}` is too long")));
    }
    Ok(n.to_string())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "concepts.list", "Remembered Objects", [], None, "→ {concepts: [{name, examples, photos}]}", always, |s, _| {
            let list: Vec<Value> =
                s.concept_store().concepts.iter().map(|c| json!({"name": c.name, "examples": c.examples.len(), "photos": c.photos})).collect();
            Ok(json!({"concepts": list}))
        }),
        cmd!(
            "concepts.teach",
            "Remember as…",
            [],
            None,
            "{name: \"Wedding dress\", id?: photo, mask?: maskId (the selected mask)} — learn what the mask selects (SAM 2.1), adding an example to the remembered object (made when new); teach it on a few photos (worn and hanging, light and dark) → {examples} (the app: {pending: true}, a notice follows)",
            always,
            |s, p| {
                let c = "concepts.teach";
                let name = name_param(p, "name", c)?;
                let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active()).ok_or_else(|| bad(c, "no photo"))?;
                let d = s.develop_of(id).ok_or_else(|| bad(c, "no such photo"))?;
                let mid = p
                    .get("mask")
                    .and_then(Value::as_u64)
                    .map(|m| m as u32)
                    .or(s.active_mask)
                    .ok_or_else(|| bad(c, "select the mask to learn from (give `mask`)"))?;
                let mask = d.masks.iter().find(|m| m.id == mid).cloned().ok_or_else(|| bad(c, format!("no mask {mid}")))?;
                if mask.components.is_empty() {
                    return Err(bad(c, "the mask selects nothing"));
                }
                #[cfg(feature = "sam")]
                {
                    use crate::segment::worker::{Kind, Payload};
                    let kind = Kind::Learn { mask: Box::new(mask) };
                    let tag = crate::segment::Tag::Learn { photo: id, name: name.clone() };
                    if s.segmenter.background {
                        s.segment_queue_kind(id, kind, tag).map_err(crate::EngineError::Other)?;
                        return Ok(json!({"pending": true, "name": name}));
                    }
                    let o = s.segment_wait_outcome(id, kind, tag).map_err(crate::EngineError::Other)?;
                    o.result.map_err(crate::EngineError::Other)?;
                    let Payload::Example(ex) = o.payload else { return Err(bad(c, "the mask selects too little to learn from")) };
                    let n = s.store_concept_example(&name, id, *ex).map_err(|e| bad(c, e))?;
                    Ok(json!({"name": name, "examples": n}))
                }
                #[cfg(not(feature = "sam"))]
                {
                    let _ = (mask, name);
                    Err(bad(c, "remembered objects need the AI models, which this build doesn't have (use the desktop app)"))
                }
            }
        ),
        cmd!("concepts.rename", "Rename Remembered Object", [], None, "{from, to}", always, |s, p| {
            let c = "concepts.rename";
            let (from, to) = (name_param(p, "from", c)?, name_param(p, "to", c)?);
            s.with_concepts(|st| st.rename(&from, &to)).map_err(crate::EngineError::Other)?.map_err(|e| bad(c, e))?;
            Ok(json!({"name": to}))
        }),
        cmd!("concepts.forget", "Forget Remembered Object", [], None, "{name} → {forgotten}", always, |s, p| {
            let c = "concepts.forget";
            let name = name_param(p, "name", c)?;
            let f = s.with_concepts(|st| st.forget(&name)).map_err(crate::EngineError::Other)?;
            Ok(json!({"forgotten": f}))
        }),
        cmd!(
            "masks.recompute",
            "Update AI Masks",
            [],
            None,
            "{ids?, force?: bool} — compute the AI parts of the masks on the photos that don't have them yet (a preset's, a paste's, ones made before a model was installed); `force` recomputes all of them (after the photo changed). In the app in the background (one activity row) → {queued, updated, approximate, notes}",
            always,
            |s, p| {
                let ids = s.targets(p)?;
                if ids.len() > 50_000 {
                    return Err(bad("masks.recompute", "at most 50000 photos at a time"));
                }
                let force = super::bool_or(p, "force", false);
                Ok(s.compute_masks(&ids, force))
            }
        ),
        cmd!(
            "masks.recalculate",
            "Update All AI Masks",
            ["Photo"],
            None,
            "{ids?} — compute every AI mask on the photos (default: the selection) again from each photo's own pixels, replacing whatever they hold (a copied, stale or approximate selection); Object masks from their clicks on that photo. In the app in the background (one activity row, cancellable) → {queued, updated, approximate, notes}",
            super::has_selection,
            |s, p| {
                let ids = s.targets(p)?;
                if ids.len() > 50_000 {
                    return Err(bad("masks.recalculate", "at most 50000 photos at a time"));
                }
                Ok(s.compute_masks(&ids, true))
            }
        ),
    ]
}
