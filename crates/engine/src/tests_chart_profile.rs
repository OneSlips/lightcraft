//! Camera profiling end to end (LR-PROF-CREATE): a made-up camera photographs a ColorChecker
//! (painted procedurally into a DNG mosaic: no photograph is involved), the chart is found, a profile
//! is built into the camera profiles, it reads back as a DCP, and developing the photo with it
//! reproduces the chart better than the file's own (naive) colour matrix does.

use std::path::PathBuf;

use lightcraft_camprof::{ChartCorners, Vec3};
use serde_json::{Value, json};

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-chartprof-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Half-resolution frame size and where the chart grid lies in it.
const W: usize = 360;
const H: usize = 240;
fn truth() -> ChartCorners {
    ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0)
}

fn chart_dng() -> Vec<u8> {
    crate::chart_profiling::demo_chart_dng(W, H, &truth()).unwrap()
}

/// Mean CIEDE2000 of the 24 patches as developed in `img` (linear Rec.2020), exposed so the middle
/// grey (D04) has its reference lightness.
fn chart_error(img: &lightcraft_raster::Rgb32f) -> f64 {
    use lightcraft_camprof::color::{D65, ciede2000, lab_to_xyz};
    let to_xyz = lightcraft_color::REC2020.to_xyz();
    let chart = lightcraft_camprof::chart();
    let layout = chart.layout;
    let (sx, sy) = (img.width as f64 / W as f64, img.height as f64 / H as f64);
    let corners = truth();
    let mut rendered = Vec::new();
    for i in 0..layout.patches() {
        let c = corners.patch_centre(layout, i);
        let (cx, cy) = ((c[0] * sx) as usize, (c[1] * sy) as usize);
        let mut sum = [0.0f64; 3];
        let mut n = 0.0;
        for y in cy - 6..cy + 6 {
            for x in cx - 6..cx + 6 {
                let p = img.data[y * img.width + x];
                for k in 0..3 {
                    sum[k] += f64::from(p[k]);
                }
                n += 1.0;
            }
        }
        let xyz = to_xyz.apply([sum[0] / n, sum[1] / n, sum[2] / n]);
        rendered.push(Vec3::new(xyz[0], xyz[1], xyz[2]));
    }
    let reference: Vec<Vec3> = chart.patches.iter().map(|p| lab_to_xyz(Vec3::new(p.lab[0], p.lab[1], p.lab[2]), D65)).collect();
    let grey = chart.patches.iter().position(|p| p.id == "D04").unwrap();
    let k = reference[grey][1] / rendered[grey][1];
    let errors: Vec<f64> = rendered.iter().zip(&reference).map(|(r, t)| ciede2000(*r * k, *t, D65)).collect();
    errors.iter().sum::<f64>() / errors.len() as f64
}

#[test]
fn a_chart_photo_becomes_a_camera_profile_that_reproduces_the_chart() {
    let dir = temp_dir("e2e");
    let (lib, files) = (dir.join("lib"), dir.join("files"));
    std::fs::create_dir_all(&files).unwrap();
    let dng = chart_dng();
    let path = files.join("CHART_0001.dng");
    std::fs::write(&path, &dng).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();

    // found where it was painted, A01 top left
    let f = s.execute("profile.findChart", &json!({})).unwrap();
    assert_eq!(f["found"], true, "{f}");
    assert_eq!(f["cameraModel"], "LightCraft Chart Camera");
    assert_eq!(f["frame"], json!([W, H]));
    assert_eq!(f["chart"]["turns"], 0);
    let c = &f["chart"]["corners"];
    for (got, want) in c.as_array().unwrap().iter().zip(truth().points()) {
        assert!((got[0].as_f64().unwrap() * W as f64 - want[0]).abs() < 6.0, "{c}");
        assert!((got[1].as_f64().unwrap() * H as f64 - want[1]).abs() < 6.0, "{c}");
    }

    let r = s.execute("profile.createFromChart", &json!({"name": "Studio D55"})).unwrap();
    assert!(r["meanDeltaE"].as_f64().unwrap() < 1.0, "{r}");
    assert_eq!(r["patches"].as_array().unwrap().len() as u64, r["patchesUsed"].as_u64().unwrap());
    let pid = r["id"].as_str().unwrap().to_string();
    assert!(pid.starts_with("dcp:"), "{r}");
    assert_eq!(r["applied"], json!(id.0));
    assert!(s.activity.list().is_empty(), "the task row is gone once the profile is made");
    // it is a camera profile like an imported one, its file in the library's Profiles folder
    let listed = s.execute("profiles.camera", &json!({})).unwrap();
    assert!(listed["profiles"].as_array().unwrap().iter().any(|p| p["id"] == pid.as_str() && p["name"] == "Studio D55"), "{listed}");
    let file = PathBuf::from(r["file"].as_str().unwrap());
    assert!(file.starts_with(&lib), "{}", file.display());
    let dcp = lightcraft_raw::dcp::parse_dcp(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!((dcp.name.as_str(), dcp.camera_model.as_str()), ("Studio D55", "LightCraft Chart Camera"));
    assert!(dcp.color.profile.hue_sat_map[0].is_some());
    assert_eq!(s.catalog.photo(id).unwrap().develop.profile.id, pid, "the chart photo develops with it");

    // developed with the new profile, the chart is reproduced; with the file's naive matrix it isn't
    let (plain, _) = crate::files::load_bytes(&dng, 4096).unwrap();
    let (profiled, _) = crate::files::load_bytes_with(&dng, 4096, Some(&dcp)).unwrap();
    let (before, after) = (chart_error(&plain), chart_error(&profiled));
    assert!(after < before * 0.5 && after < 3.0, "mean ΔE {before:.2} with the file's matrix, {after:.2} with the profile");

    // a second profile of the same name doesn't overwrite the first
    let r2 = s.execute("profile.createFromChart", &json!({"name": "Studio D55", "rendering": "natural", "apply": false})).unwrap();
    assert_eq!(r2["name"], "Studio D55 2");
    assert_ne!(r2["id"], r["id"]);
    // and both survive reopening the library
    s.persist().unwrap();
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(s.execute("profiles.camera", &json!({})).unwrap()["profiles"].as_array().unwrap().len(), 2);
    assert!(s.media.camera_profiles.contains_key(&pid));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corners_given_by_hand_and_bad_requests() {
    let dir = temp_dir("hand");
    let path = dir.join("CHART.dng");
    std::fs::write(&path, chart_dng()).unwrap();
    let mut s = Session::new();
    let p = path.to_string_lossy().to_string();
    let corners: Vec<[f64; 2]> = truth().points().iter().map(|q| [q[0] / W as f64, q[1] / H as f64]).collect();
    let r = s.execute("profile.createFromChart", &json!({"path": p, "corners": corners, "illuminant": "D65"})).unwrap();
    assert_eq!(r["turns"], 0, "orientation decided by the fit");
    assert!(r["meanDeltaE"].as_f64().unwrap() < 1.5, "{r}");
    assert_eq!(r["applied"], Value::Null, "a path is not a photo of the library");
    for bad in [
        json!({"path": p, "illuminant": "Moon"}),
        json!({"path": p, "rendering": "vivid"}),
        json!({"path": p, "corners": [[0, 0], [1, 0]]}),
        json!({"path": p, "corners": "everywhere"}),
        json!({"path": p, "corners": [[0.5, 0.5], [0.5, 0.5], [0.5, 0.5], [0.5, 0.5]]}),
        json!({"path": dir.join("missing.dng").to_string_lossy()}),
        json!({}),
    ] {
        assert!(s.execute("profile.createFromChart", &bad).is_err(), "{bad}");
    }
    // a file that is not a raw photo
    let jpeg = dir.join("not-raw.jpg");
    std::fs::write(&jpeg, b"\xff\xd8\xff\xe0 not really").unwrap();
    assert!(s.execute("profile.findChart", &json!({"path": jpeg.to_string_lossy()})).is_err());
    assert!(s.activity.list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
