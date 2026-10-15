//! A camera profile is the camera calibration, LightCraft's profiles are looks on top of it (and
//! hover previews / browser thumbnails develop the photo as the choice would). Synthetic DNGs only.

use std::path::PathBuf;

use lightcraft_camprof::ChartCorners;
use lightcraft_catalog::PhotoId;
use lightcraft_develop::DevelopSettings;
use serde_json::json;

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-layers-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A library with one chart photo (naive file matrix) and a camera profile made from it (not applied).
fn setup(tag: &str) -> (Session, PathBuf, PhotoId, String) {
    let dir = temp_dir(tag);
    let path = dir.join("CHART.dng");
    let corners = ChartCorners::from_rect(90.0, 60.0, 180.0, 120.0);
    std::fs::write(&path, crate::chart_profiling::demo_chart_dng(360, 240, &corners, "D55", false).unwrap()).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    let r = s.execute("profile.createFromChart", &json!({"illuminant": "D55", "apply": false, "name": "Chart"})).unwrap();
    (s, dir, id, r["id"].as_str().unwrap().to_string())
}

fn mean(img: &lightcraft_raster::Rgba8) -> [f64; 3] {
    let n = img.data.len().max(1) as f64;
    let mut m = [0.0; 3];
    for p in &img.data {
        for c in 0..3 {
            m[c] += f64::from(p[c]) / n;
        }
    }
    m
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.6)
}

fn differ(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).any(|(x, y)| (x - y).abs() > 2.0)
}

/// The photo rendered with its own settings.
fn rendered(s: &mut Session, id: PhotoId) -> [f64; 3] {
    let job = s.render_job(id, 96, 96, false, true).unwrap();
    mean(&job.run().rendered.unwrap().image)
}

/// The photo rendered as a hover preview of `settings`.
fn previewed(s: &mut Session, id: PhotoId, settings: &DevelopSettings) -> [f64; 3] {
    let job = s.preview_job(id, 96, 96, true, settings).unwrap();
    mean(&job.run().rendered.unwrap().image)
}

fn with(s: &Session, id: PhotoId, choice: &str) -> DevelopSettings {
    let mut d = (*s.develop_of(id).unwrap()).clone();
    crate::cmd::dcp_profiles::choose(s, &mut d, choice);
    d
}

#[test]
fn hovering_a_camera_profile_previews_it_and_back() {
    let (mut s, dir, id, dcp) = setup("hover");
    let builtin = rendered(&mut s, id);
    // hovering the camera profile: its colour, not the built-in one (failed before: the preview
    // developed the source decoded without it)
    let hover_dcp = {
        let w = with(&s, id, &dcp);
        previewed(&mut s, id, &w)
    };
    assert!(differ(hover_dcp, builtin), "{hover_dcp:?} vs {builtin:?}");
    s.execute("develop.profile", &json!({"id": dcp})).unwrap();
    let selected = rendered(&mut s, id);
    assert!(close(hover_dcp, selected), "the hover is what selecting gives: {hover_dcp:?} vs {selected:?}");
    // and back: hovering LightCraft's own colour while the camera profile is applied
    let hover_builtin = {
        let w = with(&s, id, "");
        previewed(&mut s, id, &w)
    };
    assert!(close(hover_builtin, builtin), "{hover_builtin:?} vs {builtin:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn variant_thumbnails_and_caches_key_on_the_camera_profile() {
    let (mut s, dir, id, dcp) = setup("variant");
    let plain = s.develop_of(id).unwrap();
    let a = s.variant_job(id, &plain, 64).unwrap();
    let b = s.variant_job(id, &with(&s, id, &dcp), 64).unwrap();
    assert_ne!(a.key, b.key);
    assert_ne!(a.cache.as_ref().unwrap().1, b.cache.as_ref().unwrap().1);
    match &b.source {
        crate::media::SourceRef::File { camera_profile: Some((made_with, _)), .. } => assert_eq!(made_with, &dcp),
        _ => panic!("the variant decodes with the camera profile"),
    }
    let (ma, mb) = (mean(&a.run().rendered.unwrap().image), mean(&b.run().rendered.unwrap().image));
    assert!(differ(ma, mb), "a camera profile's thumbnail shows its colour: {ma:?} vs {mb:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn looks_render_on_the_calibration_and_switch_independently() {
    let (mut s, dir, id, dcp) = setup("layers");
    let builtin_neutral = {
        let w = with(&s, id, "lc.neutral");
        previewed(&mut s, id, &w)
    };
    s.execute("develop.profile", &json!({"id": dcp})).unwrap();
    let bare = rendered(&mut s, id);
    // a look keeps the camera profile
    s.execute("develop.profile", &json!({"id": "lc.neutral"})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), ("lc.neutral", dcp.as_str()));
    let neutral_on_dcp = rendered(&mut s, id);
    assert!(differ(neutral_on_dcp, builtin_neutral) && differ(neutral_on_dcp, bare), "{neutral_on_dcp:?} / {builtin_neutral:?} / {bare:?}");
    // another camera profile keeps the look; LightCraft's own colour too
    s.execute("develop.cameraProfile", &json!({"id": ""})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.profile.id.as_str(), d.calibration_id()), ("lc.neutral", None));
    s.execute("develop.cameraProfile", &json!({"id": dcp})).unwrap();
    assert_eq!(s.develop_of(id).unwrap().profile.id, "lc.neutral");
    // a preset that sets a look keeps the calibration
    s.execute("develop.profile", &json!({"id": "lc.vivid"})).unwrap();
    let preset = s.execute("preset.create", &json!({"name": "Vivid look", "groups": ["profile"]})).unwrap();
    let pid = preset["id"].as_str().unwrap_or("user.vivid-look").to_string();
    s.execute("develop.cameraProfile", &json!({"id": ""})).unwrap();
    s.execute("develop.profile", &json!({"id": "lc.color"})).unwrap();
    s.execute("develop.cameraProfile", &json!({"id": dcp})).unwrap();
    s.execute("preset.apply", &json!({"id": pid})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), ("lc.vivid", dcp.as_str()), "{preset}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_edits_naming_a_camera_profile_render_as_before() {
    let (mut s, dir, id, dcp) = setup("legacy");
    // before: the camera profile as the profile; now: calibration + its own look
    let mut legacy = (*s.develop_of(id).unwrap()).clone();
    legacy.profile.id = dcp.clone();
    let mut layered = legacy.clone();
    layered.camera_profile = dcp.clone();
    assert_eq!(legacy.calibration_id(), Some(dcp.as_str()));
    let (a, b) = (previewed(&mut s, id, &legacy), previewed(&mut s, id, &layered));
    assert!(close(a, b), "{a:?} vs {b:?}");
    // picking a look on an old edit keeps its camera profile as the calibration
    let mut d = legacy.clone();
    crate::cmd::dcp_profiles::choose(&s, &mut d, "lc.neutral");
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), ("lc.neutral", dcp.as_str()));
    // settings without a camera profile keep their JSON (and so their hash)
    assert!(DevelopSettings::default().to_json().get("camera_profile").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_camera_profile_with_its_own_tone_curve_as_profile_or_under_a_look() {
    let (mut s, dir, id, dcp) = setup("ownlook");
    // the same calibration with a tone curve of its own (a third-party style profile)
    let mut p = (*s.media.camera_profiles[&dcp]).clone();
    p.name = "Contrasty".into();
    p.color.profile.tone_curve = lightcraft_raw::profile::ToneCurve::from_tag(&[0.0, 0.0, 0.25, 0.12, 0.5, 0.5, 0.75, 0.88, 1.0, 1.0]);
    let file = dir.join("contrasty.dcp");
    std::fs::write(&file, lightcraft_raw::dcp::write_dcp(&p).unwrap()).unwrap();
    let r = s.execute("profile.import", &json!({"paths": [file.to_string_lossy()]})).unwrap();
    let own = r["imported"][0]["id"].as_str().unwrap().to_string();
    // chosen: "As profile" (its own look)
    s.execute("develop.profile", &json!({"id": own})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), (own.as_str(), own.as_str()));
    let as_profile = rendered(&mut s, id);
    // a LightCraft look replaces its look parts, keeps its matrices
    s.execute("develop.profile", &json!({"id": "lc.color"})).unwrap();
    let under_look = rendered(&mut s, id);
    assert!(differ(as_profile, under_look), "{as_profile:?} vs {under_look:?}");
    s.execute("develop.cameraProfile", &json!({"id": dcp})).unwrap();
    let chart = rendered(&mut s, id);
    assert!(close(under_look, chart), "same matrices, no own look: {under_look:?} vs {chart:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn xmp_keeps_both_layers_and_maps_a_lightroom_camera_profile_name() {
    let (mut s, dir, id, dcp) = setup("xmp");
    s.execute("develop.profile", &json!({"id": dcp})).unwrap();
    s.execute("develop.profile", &json!({"id": "lc.neutral"})).unwrap();
    s.execute("photo.saveMetadataToFile", &json!({})).unwrap();
    s.execute("develop.cameraProfile", &json!({"id": ""})).unwrap();
    s.execute("develop.profile", &json!({"id": "lc.color"})).unwrap();
    s.execute("photo.readMetadataFromFile", &json!({})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.profile.id.as_str(), d.camera_profile.as_str()), ("lc.neutral", dcp.as_str()), "both layers round-trip");
    // a Lightroom sidecar naming the user's camera profile: it becomes the calibration
    let name = s.dcp_profiles[0].name.clone();
    let x = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:CameraProfile="{name}" crs:Exposure2012="+0.30"/></rdf:RDF></x:xmpmeta>"#
    );
    let sc = crate::sidecar::parse_sidecar(&x, crate::crs::Target::RawAbsolute).unwrap();
    assert_eq!(sc.camera_profile.as_deref(), Some(name.as_str()));
    let mut p = (**s.catalog.photo(id).unwrap()).clone();
    let mut fresh = (*p.develop).clone();
    fresh.camera_profile.clear();
    fresh.profile.id = "lc.color".into();
    p.develop = std::sync::Arc::new(fresh);
    assert!(crate::cmd::dcp_profiles::from_xmp_name(&s, &mut p, &name));
    assert_eq!(p.develop.calibration_id(), Some(dcp.as_str()));
    assert!(!crate::cmd::dcp_profiles::from_xmp_name(&s, &mut p, "Adobe Portrait"), "not one of the user's");
    let _ = std::fs::remove_dir_all(&dir);
}
