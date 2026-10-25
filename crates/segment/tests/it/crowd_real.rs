//! A group's People class map with the real MediaPipe models: each small person segmented again on
//! their own crop of the sharper render, pasted back; prints the time for 1, 5, 20 and 60 crops and
//! how much of the faces the class map calls clothes before and after. Needs
//! `LIGHTCRAFT_MEDIAPIPE_MODELS` (a folder with `selfie_multiclass_256x256.tflite`,
//! `face_landmarker.task` and `face_detection_full_range.tflite`) and `LIGHTCRAFT_CROWD_PHOTO` (a
//! JPEG of a group, people under a third of the frame's height); skipped without them.

use lightcraft_segment::people::{FaceModel, PeopleMap, PeopleModel, person_boxes};

fn resized(img: &image::RgbImage, edge: u32) -> (usize, usize, Vec<u8>) {
    let s = edge as f32 / img.width().max(img.height()) as f32;
    let img = image::imageops::resize(img, (img.width() as f32 * s) as u32, (img.height() as f32 * s) as u32, image::imageops::FilterType::Triangle);
    (img.width() as usize, img.height() as usize, img.into_raw())
}

/// Mean probability of `class` over the middle of each face box (normalized boxes).
fn on_faces(map: &PeopleMap, faces: &[[f32; 4]], class: usize) -> f32 {
    let mut acc = 0.0;
    let mut n = 0.0;
    for b in faces {
        for k in 0..25 {
            let u = b[0] + (b[2] - b[0]) * (0.3 + 0.1 * (k % 5) as f32);
            let v = b[1] + (b[3] - b[1]) * (0.3 + 0.1 * (k / 5) as f32);
            acc += map.at(&[class], u, v);
            n += 1.0;
        }
    }
    if n > 0.0 { acc / n } else { 0.0 }
}

#[test]
fn a_groups_people_are_segmented_on_their_own_crops() {
    let (Some(dir), Some(photo)) = (std::env::var_os("LIGHTCRAFT_MEDIAPIPE_MODELS"), std::env::var_os("LIGHTCRAFT_CROWD_PHOTO")) else {
        lightcraft_testkit::skip!(Model, "set LIGHTCRAFT_MEDIAPIPE_MODELS and LIGHTCRAFT_CROWD_PHOTO");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let img = image::open(photo).unwrap().to_rgb8();
    // as the app does: the model input at 1024 on the long edge, crops from a 3072 render
    let ((w, h, rgb), (fw, fh, fine)) = (resized(&img, 1024), resized(&img, 3072));
    let people = PeopleModel::load(&dir).unwrap();
    let faces = FaceModel::load(&dir).unwrap().faces(&rgb, w, h).unwrap();
    let t = std::time::Instant::now();
    let whole = people.segment(&rgb, w, h).unwrap();
    eprintln!("{} faces; whole-image map in {:?}", faces.len(), t.elapsed());
    let boxes = person_boxes(&faces, w, h);
    eprintln!("{} people small enough to crop", boxes.len());
    if boxes.is_empty() {
        lightcraft_testkit::skip!(Model, "nobody in LIGHTCRAFT_CROWD_PHOTO is small enough to crop");
        return;
    }
    for n in [1usize, 5, 20, 60] {
        let set: Vec<[f32; 4]> = boxes.iter().cycle().take(n).copied().collect();
        let t = std::time::Instant::now();
        let map = people.segment_people(&fine, fw, fh, &whole, &set, 1024).unwrap();
        eprintln!("{n:>3} crops: {:?}", t.elapsed());
        assert_eq!(map.side, 1024);
    }
    let crowd = people.segment_people(&fine, fw, fh, &whole, &boxes, 1024).unwrap();
    let face_boxes: Vec<[f32; 4]> = faces
        .iter()
        .filter(|f| f.detection.height() * 9.5 < h as f32 / 3.0)
        .map(|f| {
            let d = &f.detection;
            [d.x0 / w as f32, d.y0 / h as f32, d.x1 / w as f32, d.y1 / h as f32]
        })
        .collect();
    let (cb, ca) = (on_faces(&whole, &face_boxes, 4), on_faces(&crowd, &face_boxes, 4));
    let (sb, sa) = (on_faces(&whole, &face_boxes, 3), on_faces(&crowd, &face_boxes, 3));
    eprintln!("small faces: clothes {cb:.3} → {ca:.3}, face skin {sb:.3} → {sa:.3}");
    assert!(sa >= sb - 0.05, "the crops see the faces at least as well");
}
