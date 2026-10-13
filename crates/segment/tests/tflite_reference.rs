//! The TFLite interpreter against LiteRT (Google's TensorFlow Lite runtime, fp32 CPU) on the three
//! official MediaPipe models.
//!
//! Needs a directory with `selfie_multiclass_256x256.tflite` and either `face_detector.tflite` +
//! `face_landmarks_detector.tflite` or `face_landmarker.task` (as downloaded), and a reference file
//! made by the scratch script `tflite/ref.py` (inputs and outputs on real photos):
//! `LIGHTCRAFT_MEDIAPIPE_DIR=<dir> LIGHTCRAFT_TFLITE_REF=<ref.safetensors> cargo test -p lightcraft-segment --release --test tflite_reference -- --nocapture`.
//! Skipped (passes) when either is missing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::{Device, Tensor};
use lightcraft_segment::tflite::{Model, zip};

fn setup() -> Option<(PathBuf, HashMap<String, Tensor>)> {
    let dir = PathBuf::from(std::env::var_os("LIGHTCRAFT_MEDIAPIPE_DIR")?);
    let reference = PathBuf::from(std::env::var_os("LIGHTCRAFT_TFLITE_REF")?);
    let r = candle_core::safetensors::load(&reference, &Device::Cpu).ok()?;
    Some((dir, r))
}

/// A model file of the directory, or the entry of the same name in `face_landmarker.task`.
fn model_bytes(dir: &Path, name: &str) -> Vec<u8> {
    if let Ok(b) = std::fs::read(dir.join(name)) {
        return b;
    }
    let task = std::fs::read(dir.join("face_landmarker.task")).unwrap_or_else(|_| panic!("{name} not found in {}", dir.display()));
    zip::entry(&task, name).unwrap_or_else(|| panic!("{name} not in face_landmarker.task")).to_vec()
}

fn vec(t: &Tensor) -> Vec<f32> {
    t.flatten_all().unwrap().to_vec1::<f32>().unwrap()
}

#[test]
fn matches_litert() {
    let Some((dir, r)) = setup() else {
        eprintln!("skipped: set LIGHTCRAFT_MEDIAPIPE_DIR and LIGHTCRAFT_TFLITE_REF");
        return;
    };
    for (name, file) in [
        ("selfie", "selfie_multiclass_256x256.tflite"),
        ("detector", "face_detector.tflite"),
        ("landmarks", "face_landmarks_detector.tflite"),
    ] {
        let t = std::time::Instant::now();
        let model = Model::from_bytes(&model_bytes(&dir, file)).unwrap();
        eprintln!("{name}: loaded in {:?}, inputs {:?}, outputs {:?}", t.elapsed(), model.input_shapes(), model.output_shapes());
        let mut worst = vec![(0f32, 0f32); model.output_shapes().len()];
        let mut photos = 0;
        while let Some(x) = r.get(&format!("{name}.{photos}.input")) {
            let outs = model.run(&[&vec(x)]).unwrap();
            for (oi, got) in outs.iter().enumerate() {
                let want = vec(&r[&format!("{name}.{photos}.out{oi}")]);
                assert_eq!(got.len(), want.len());
                let scale = want.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
                let err = got.iter().zip(&want).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
                let w = &mut worst[oi];
                *w = (w.0.max(err), w.1.max(err / scale));
            }
            photos += 1;
        }
        assert!(photos > 0, "no reference inputs for {name}");
        for (oi, (abs, rel)) in worst.iter().enumerate() {
            eprintln!("{name} output {oi}: max abs err {abs:.2e}, max rel err {rel:.2e} over {photos} photos");
            assert!(*rel < 1e-3, "{name} output {oi}: relative error {rel}");
        }
    }
}
