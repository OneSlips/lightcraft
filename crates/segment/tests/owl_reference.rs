//! The OWLv2 port against the reference implementation (Hugging Face `transformers`
//! `Owlv2ForObjectDetection`, fp32 CPU), and Describe (OWLv2 boxes, SAM 2.1 masks) on a photo.
//!
//! Needs the `google/owlv2-base-patch16-ensemble` checkpoint (Hugging Face format) and a
//! reference file made by the scratch script `make_ref.py` (the photo, the processor's pixels and
//! token ids, and the model's class embeddings, boxes, logits and query embeddings):
//! `LIGHTCRAFT_OWL_DIR=<model dir> LIGHTCRAFT_OWL_REF=<ref.safetensors> LIGHTCRAFT_OWL_QUERIES="white dress|person|sand|tattoo"
//! cargo test -p lightcraft-segment --release --test owl_reference -- --nocapture`; with
//! `LIGHTCRAFT_SAM2_DIR` too it also checks the Describe mask of the strongest query. Skipped
//! (passes) when they are missing.

use std::collections::HashMap;
use std::path::PathBuf;

use candle_core::{Device, Tensor};
use lightcraft_segment::describe::{self, Params};
use lightcraft_segment::owl::{self, Owl};
use lightcraft_segment::{SAM2_MASK_SIDE, Sam2};

fn setup() -> Option<(PathBuf, HashMap<String, Tensor>, Vec<String>)> {
    let dir = PathBuf::from(std::env::var_os("LIGHTCRAFT_OWL_DIR")?);
    let reference = PathBuf::from(std::env::var_os("LIGHTCRAFT_OWL_REF")?);
    let queries = std::env::var("LIGHTCRAFT_OWL_QUERIES").ok()?.split('|').map(str::to_string).collect();
    let r = candle_core::safetensors::load(&reference, &Device::Cpu).ok()?;
    Some((dir, r, queries))
}

fn vec(t: &Tensor) -> Vec<f32> {
    t.to_dtype(candle_core::DType::F32).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap()
}

/// Max |a − b| relative to the reference's max |b|.
fn rel_err(name: &str, a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "{name}: length");
    let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let err = a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
    eprintln!("{name:>16}: max abs err {err:.2e}, rel {:.2e} (scale {scale:.2e})", err / scale);
    err / scale
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[test]
fn matches_reference() {
    let Some((dir, r, queries)) = setup() else {
        lightcraft_testkit::skip!(Model, "set LIGHTCRAFT_OWL_DIR, LIGHTCRAFT_OWL_REF and LIGHTCRAFT_OWL_QUERIES");
        return;
    };
    let t = std::time::Instant::now();
    let model = Owl::load(&dir).unwrap();
    eprintln!("load: {:?}", t.elapsed());

    // preprocessing: our Gaussian + linear resize against the processor's (scipy)
    let rgb: Vec<u8> = r["rgb"].flatten_all().unwrap().to_vec1().unwrap();
    let size: Vec<i64> = r["size"].to_vec1().unwrap();
    let (w, h) = (size[0] as usize, size[1] as usize);
    let (ours, side) = owl::preprocess(&rgb, w, h).unwrap();
    assert_eq!(side, w.max(h));
    let want = vec(&r["pixel_values"]);
    let diff: Vec<f32> = ours.iter().zip(&want).map(|(a, b)| (a - b).abs()).collect();
    let mean = diff.iter().sum::<f32>() / diff.len() as f32;
    let max = diff.iter().fold(0f32, |m, v| m.max(*v));
    eprintln!("   preprocessing: mean diff {mean:.2e}, max {max:.2e}");
    assert!(mean < 0.01 && max < 0.5, "{mean} {max}");

    // image side from the reference's own pixels
    let t = std::time::Instant::now();
    let enc = model.encode_pixels(&want).unwrap();
    eprintln!("encode (CPU): {:?}", t.elapsed());
    assert!(rel_err("class embeds", enc.class_embeds(), &vec(&r["class_embeds"])) < 2e-3);
    let boxes: Vec<f32> = enc.boxes().iter().flatten().copied().collect();
    assert!(rel_err("boxes", &boxes, &vec(&r["pred_boxes"])) < 2e-3);

    // text side and scores
    let t = std::time::Instant::now();
    let text: Vec<f32> = queries.iter().flat_map(|q| model.embed_text(&format!("a photo of a {q}")).unwrap()).collect();
    eprintln!("text ({} queries): {:?}", queries.len(), t.elapsed());
    assert!(rel_err("text embeds", &text, &vec(&r["text_embeds"])) < 2e-3);
    let logits = vec(&r["logits"]);
    let nq = queries.len();
    let best: Vec<f32> = logits.chunks(nq).map(|l| sigmoid(l.iter().fold(f32::NEG_INFINITY, |m, v| m.max(*v)))).collect();
    let dets = model.detect(&enc, &queries).unwrap();
    let scores: Vec<f32> = dets.iter().map(|d| d.score).collect();
    assert!(rel_err("scores", &scores, &best) < 5e-3);

    // the whole image path on our own preprocessing finds the same strongest box
    let enc = model.encode(&rgb, w, h).unwrap();
    let found = describe::boxes(model.detect(&enc, &queries).unwrap(), &Params::default());
    let top = found.first().expect("something is found");
    eprintln!("strongest: {} {:.3} at {:?}", queries[top.query], top.score, top.rect);
    let (ri, rs) = best.iter().enumerate().fold((0, 0f32), |m, (i, s)| if *s > m.1 { (i, *s) } else { m });
    let rq = logits.chunks(nq).nth(ri).unwrap().iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (q, v)| if *v > m.1 { (q, *v) } else { m }).0;
    assert_eq!(top.query, rq);
    assert!((top.score - rs).abs() < 0.05, "{} vs {rs}", top.score);

    // Describe: the strongest query's boxes as SAM 2.1 masks, merged
    let Some(sam_dir) = std::env::var_os("LIGHTCRAFT_SAM2_DIR") else {
        lightcraft_testkit::skip!(Model, "set LIGHTCRAFT_SAM2_DIR to check the Describe mask too");
        return;
    };
    let sam = Sam2::load_on(&PathBuf::from(sam_dir), Device::Cpu).unwrap();
    let sam_enc = sam.encode(&rgb, w, h).unwrap();
    let t = std::time::Instant::now();
    let p = describe::describe(&model, &enc, &sam, &sam_enc, &queries[rq], &Params::default()).unwrap().expect("a mask");
    eprintln!("describe {:?}: {:?}", queries[rq], t.elapsed());
    assert_eq!(p.logits.len(), SAM2_MASK_SIDE * SAM2_MASK_SIDE);
    // the mask lies inside the strongest box (with a margin), and fills a good part of it
    let r = top.rect;
    let (mut inside, mut outside) = (0usize, 0usize);
    for (i, l) in p.logits.iter().enumerate() {
        if *l > 0.0 {
            let (x, y) = ((i % SAM2_MASK_SIDE) as f32 / SAM2_MASK_SIDE as f32, (i / SAM2_MASK_SIDE) as f32 / SAM2_MASK_SIDE as f32);
            let m = 0.03;
            if x >= r[0] - m && x <= r[2] + m && y >= r[1] - m && y <= r[3] + m { inside += 1 } else { outside += 1 }
        }
    }
    let box_px = (r[2] - r[0]) * (r[3] - r[1]) * (SAM2_MASK_SIDE * SAM2_MASK_SIDE) as f32;
    eprintln!("mask: {inside} px inside the box ({box_px:.0}), {outside} outside");
    assert!(inside as f32 > 0.3 * box_px && (outside as f32) < 0.2 * inside as f32);
}
