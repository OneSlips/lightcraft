//! Camera profiling end to end (LR-PROF-CREATE): a made-up camera photographs a ColorChecker
//! (painted procedurally into a DNG mosaic: no photograph is involved), the chart is found, a profile
//! is built into the camera profiles, it reads back as a DCP, and developing the photo with it
//! reproduces the chart better than the file's own (naive) colour matrix does.

use std::path::PathBuf;

use lightcraft_camprof::{ChartCorners, Vec3};
use serde_json::json;

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
    crate::chart_profiling::demo_chart_dng(W, H, &truth(), "D55", false).unwrap()
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

    let r = s.execute("profile.createFromChart", &json!({"name": "Studio D55", "illuminant": "D55"})).unwrap();
    assert!(r["meanDeltaE"].as_f64().unwrap() < 1.0, "{r}");
    assert_eq!(r["patches"].as_array().unwrap().len() as u64, r["patchesUsed"].as_u64().unwrap());
    let pid = r["id"].as_str().unwrap().to_string();
    assert!(pid.starts_with("dcp:"), "{r}");
    assert_eq!(r["applied"], json!([id.0]));
    assert!(s.activity.list().is_empty(), "the task row is gone once the profile is made");
    // it is a camera profile like an imported one, its file in the library's Profiles folder
    let listed = s.execute("profiles.camera", &json!({})).unwrap();
    assert!(listed["profiles"].as_array().unwrap().iter().any(|p| p["id"] == pid.as_str() && p["name"] == "Studio D55"), "{listed}");
    let file = PathBuf::from(r["file"].as_str().unwrap());
    assert!(file.starts_with(&lib), "{}", file.display());
    let dcp = lightcraft_raw::dcp::parse_dcp(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!((dcp.name.as_str(), dcp.camera_model.as_str()), ("Studio D55", "LightCraft Chart Camera"));
    assert!(dcp.color.profile.hue_sat_map[0].is_some());
    assert_eq!(s.catalog.photo(id).unwrap().develop.calibration_id(), Some(pid.as_str()), "the chart photo develops with it");

    // developed with the new profile, the chart is reproduced; with the file's naive matrix it isn't
    let (plain, _) = crate::files::load_bytes(&dng, 4096).unwrap();
    let (profiled, _) = crate::files::load_bytes_with(&dng, 4096, Some(&dcp)).unwrap();
    let (before, after) = (chart_error(&plain), chart_error(&profiled));
    assert!(after < before * 0.5 && after < 3.0, "mean ΔE {before:.2} with the file's matrix, {after:.2} with the profile");

    // a second profile of the same name: refused by default, saved beside it with keepBoth
    let e =
        s.execute("profile.createFromChart", &json!({"name": "Studio D55", "rendering": "reproduction", "apply": false})).unwrap_err().to_string();
    assert!(e.contains("Studio D55") && e.contains("replace") && e.contains("keepBoth"), "{e}");
    let r2 = s
        .execute("profile.createFromChart", &json!({"name": "Studio D55", "rendering": "reproduction", "apply": false, "onExists": "keepBoth"}))
        .unwrap();
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
    assert_eq!(r["applied"], json!([]), "a path is not a photo of the library");
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

/// Two chart photos, tungsten (A) and daylight (D65), of a camera whose files carry its true matrix
/// (so each photo's light can be told from its greys).
fn lit_dng(light: &str) -> Vec<u8> {
    crate::chart_profiling::demo_chart_dng(W, H, &truth(), light, true).unwrap()
}

fn profile_of(s: &Session, id: &str) -> lightcraft_raw::dcp::DcpProfile {
    let file = &s.dcp_profiles.iter().find(|p| p.id == id).unwrap().file;
    lightcraft_raw::dcp::parse_dcp(&std::fs::read(file).unwrap()).unwrap()
}

#[test]
fn two_lights_make_a_dual_illuminant_profile_that_beats_either_single_one() {
    let dir = temp_dir("dual");
    let (a, d) = (dir.join("CHART_A.dng"), dir.join("CHART_D65.dng"));
    let (a_dng, d_dng) = (lit_dng("A"), lit_dng("D65"));
    std::fs::write(&a, &a_dng).unwrap();
    std::fs::write(&d, &d_dng).unwrap();
    let mut s = Session::new();
    let (pa, pd) = (a.to_string_lossy().to_string(), d.to_string_lossy().to_string());
    // each photo's light is told from its greys
    for (p, light) in [(&pa, "A"), (&pd, "D65")] {
        let f = s.execute("profile.findChart", &json!({"path": p})).unwrap();
        assert_eq!(f["chart"]["estimatedIlluminant"], light, "{f}");
    }
    // daylight given first: calibration 1 is still tungsten
    let dual =
        s.execute("profile.createFromChart", &json!({"path": pd, "illuminant": "D65", "second": {"path": pa, "illuminant": "tungsten"}})).unwrap();
    assert_eq!(dual["dual"], true, "{dual}");
    let cals = dual["calibrations"].as_array().unwrap();
    assert_eq!((cals[0]["illuminant"].as_str(), cals[0]["shot"].as_u64()), (Some("A"), Some(1)));
    assert_eq!((cals[1]["illuminant"].as_str(), cals[1]["shot"].as_u64()), (Some("D65"), Some(0)));
    assert!(cals.iter().all(|c| c["meanDeltaE"].as_f64().unwrap() < 1.0), "{dual}");
    assert!(dual["name"].as_str().unwrap().starts_with("LightCraft Chart Camera CH · Tungsten + D65 · Photo · "), "{}", dual["name"]);
    let dual = profile_of(&s, dual["id"].as_str().unwrap());
    assert_eq!(dual.color.illuminant, [17, 21]);
    let single = |s: &mut Session, p: &str, light: &str| {
        let r = s.execute("profile.createFromChart", &json!({"path": p, "illuminant": light})).unwrap();
        profile_of(s, r["id"].as_str().unwrap())
    };
    let (only_a, only_d) = (single(&mut s, &pa, "A"), single(&mut s, &pd, "D65"));
    assert_eq!((only_a.color.illuminant[0], only_d.color.illuminant[0]), (17, 21));

    // each light developed with each profile
    let error = |dng: &[u8], p: &lightcraft_raw::dcp::DcpProfile| chart_error(&crate::files::load_bytes_with(dng, 4096, Some(p)).unwrap().0);
    let (a_dual, a_a, a_d) = (error(&a_dng, &dual), error(&a_dng, &only_a), error(&a_dng, &only_d));
    let (d_dual, d_a, d_d) = (error(&d_dng, &dual), error(&d_dng, &only_a), error(&d_dng, &only_d));
    eprintln!(
        "tungsten shot: dual {a_dual:.2}, A-only {a_a:.2}, D65-only {a_d:.2}; daylight shot: dual {d_dual:.2}, A-only {d_a:.2}, D65-only {d_d:.2}"
    );
    assert!(a_dual < a_d && d_dual < d_a, "the dual profile beats the other light's profile");
    assert!(a_dual <= a_a + 0.05 && d_dual <= d_d + 0.05, "and matches the shot's own light's profile");
    assert!(a_dual < 2.0 && d_dual < 2.0);

    // a shot between the two lights takes matrices between them (DNG 6.3: by inverse colour temperature)
    let mid = lightcraft_color::cct::temp_tint_to_xy(4000.0, 0.0);
    let g = lightcraft_raw::color::illuminant_weight(&dual.color, mid);
    assert!(g > 0.2 && g < 0.8, "weight of calibration 1 at 4000 K: {g}");
    let m = |c: &lightcraft_raw::ColorData| lightcraft_raw::color::camera_to_xyz_d50(c, mid);
    let (mid_dual, mid_a, mid_d) = (m(&dual.color), m(&only_a.color), m(&only_d.color));
    for i in 0..3 {
        for j in 0..3 {
            let (lo, hi) = (mid_a.0[i][j].min(mid_d.0[i][j]), mid_a.0[i][j].max(mid_d.0[i][j]));
            assert!(mid_dual.0[i][j] > lo - 0.02 && mid_dual.0[i][j] < hi + 0.02, "{mid_dual:?} between {mid_a:?} and {mid_d:?}");
        }
    }
    assert_ne!(mid_dual, mid_a);
    assert_ne!(mid_dual, mid_d);
    let blended = dual.color.profile.hue_sat_for(g).unwrap();
    assert_ne!(Some(&blended), dual.color.profile.hue_sat_map[0].as_ref());
    assert_ne!(Some(&blended), dual.color.profile.hue_sat_map[1].as_ref());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_light_twice_is_not_a_dual_profile() {
    let dir = temp_dir("same");
    let (a, b) = (dir.join("ONE.dng"), dir.join("TWO.dng"));
    std::fs::write(&a, lit_dng("D65")).unwrap();
    std::fs::write(&b, lit_dng("D65")).unwrap();
    let mut s = Session::new();
    let (pa, pb) = (a.to_string_lossy().to_string(), b.to_string_lossy().to_string());
    let e = s.execute("profile.createFromChart", &json!({"path": pa, "second": {"path": pb}})).unwrap_err().to_string();
    assert!(e.contains("same light"), "{e}");
    // declared apart, but the camera saw one light
    let e = s.execute("profile.createFromChart", &json!({"path": pa, "second": {"path": pb, "illuminant": "A"}})).unwrap_err().to_string();
    assert!(e.contains("same light"), "{e}");
    assert!(s.execute("profile.createFromChart", &json!({"path": pa, "second": {}})).is_err());
    assert!(s.execute("profile.createFromChart", &json!({"path": pa, "second": 3})).is_err());
    assert!(s.dcp_profiles.is_empty(), "nothing was added");
    let _ = std::fs::remove_dir_all(&dir);
}
