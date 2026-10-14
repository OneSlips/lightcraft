//! A mask's local Curve and Point Color: the commands, and Lightroom presets that carry them (import
//! and XMP export).

use serde_json::json;

use crate::Session;

fn active_dev(s: &Session) -> lightcraft_develop::DevelopSettings {
    (*s.develop_of(s.active().unwrap()).unwrap()).clone()
}

/// The demo photo with one radial mask (selected).
fn with_mask() -> (Session, u32) {
    let mut s = Session::with_demo();
    s.execute("mask.add", &json!({"kind": "radial", "center": {"x": 0.5, "y": 0.5}, "rx": 0.3, "ry": 0.3})).unwrap();
    let id = active_dev(&s).masks[0].id;
    (s, id)
}

#[test]
fn mask_curve_command_sets_resets_and_renders() {
    let (mut s, id) = with_mask();
    let photo = s.active().unwrap();
    let before = s.render_now(photo, 256, 256).unwrap().image;
    s.execute("mask.curve", &json!({"channel": "master", "points": [[0.0, 0.2], [0.5, 0.7], [1.0, 1.0]]})).unwrap();
    s.execute("mask.curve", &json!({"id": id, "channel": "red", "points": [[0.0, 0.0], [1.0, 0.8]], "refineSaturation": 40})).unwrap();
    let m = &active_dev(&s).masks[0];
    assert_eq!(m.curve.master.len(), 3);
    assert_eq!(m.curve.red.len(), 2);
    assert_eq!(m.curve.refine_saturation, 40.0);
    let after = s.render_now(photo, 256, 256).unwrap().image;
    let at = |img: &lightcraft_raster::Rgba8, x: usize, y: usize| img.data[y * img.width + x];
    let (cx, cy) = (after.width / 2, after.height / 2);
    assert_ne!(at(&after, cx, cy), at(&before, cx, cy), "the centre (in the mask) changes");
    assert_eq!(at(&after, 2, 2), at(&before, 2, 2), "the corner (outside) does not");
    // hostile input: unknown channel, nothing to do, too many points
    assert!(s.execute("mask.curve", &json!({"channel": "purple", "points": [[0, 0], [1, 1]]})).is_err());
    assert!(s.execute("mask.curve", &json!({})).is_err());
    let many: Vec<[f64; 2]> = (0..500).map(|i| [i as f64 / 500.0, 0.5]).collect();
    s.execute("mask.curve", &json!({"channel": "blue", "points": many})).unwrap();
    assert!(active_dev(&s).masks[0].curve.blue.len() <= 64);
    // reset one channel, then all
    s.execute("mask.curve", &json!({"channel": "red", "reset": true})).unwrap();
    assert!(active_dev(&s).masks[0].curve.red.is_empty());
    s.execute("mask.curve", &json!({"reset": true})).unwrap();
    assert!(active_dev(&s).masks[0].curve.is_default());
    // undo brings the curve back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!active_dev(&s).masks[0].curve.master.is_empty());
}

#[test]
fn mask_point_color_pick_set_delete() {
    let (mut s, _) = with_mask();
    let r = s.execute("mask.pickPointColor", &json!({"x": 0.5, "y": 0.5})).unwrap();
    assert_eq!(r["index"], 0);
    s.execute("mask.setPointColor", &json!({"index": 0, "values": {"satShift": -100, "hueRange": 250}})).unwrap();
    let p = active_dev(&s).masks[0].point_colors[0];
    assert_eq!((p.sat_shift, p.hue_range), (-100.0, 100.0), "clamped to the slider's range");
    assert!(s.execute("mask.setPointColor", &json!({"index": 3, "values": {"satShift": 1}})).is_err());
    assert!(s.execute("mask.setPointColor", &json!({"index": 0, "values": {"nope": 1}})).is_err());
    assert!(s.execute("mask.setPointColor", &json!({"index": -1, "values": {"satShift": 1}})).is_err());
    for _ in 0..7 {
        s.execute("mask.pickPointColor", &json!({"x": 0.4, "y": 0.6})).unwrap();
    }
    assert!(s.execute("mask.pickPointColor", &json!({"x": 0.4, "y": 0.6})).is_err(), "eight at most");
    s.execute("mask.deletePointColor", &json!({"index": 2})).unwrap();
    assert_eq!(active_dev(&s).masks[0].point_colors.len(), 7);
    // the global Point Color is untouched, and the settings survive their JSON
    let d = active_dev(&s);
    assert!(d.point_colors.is_empty());
    assert_eq!(lightcraft_develop::DevelopSettings::from_json(&d.to_json()).unwrap(), d);
}

/// A Lightroom Classic preset with a local Curve and local Point Color (structure as LrC 15 writes it;
/// values made up for this test).
const PRESET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Normal" crs:Version="17.4"
  crs:CompatibleVersion="268435456" crs:ShowInPresets="True" crs:ShowInQuickActions="False" crs:HasSettings="True">
 <crs:Name><rdf:Alt><rdf:li xml:lang="x-default">Warm Back</rdf:li></rdf:Alt></crs:Name>
 <crs:MaskGroupBasedCorrections><rdf:Seq>
  <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:CorrectionName="Mask 1"
    crs:LocalContrast2012="0.2" crs:LocalCurveRefineSaturation="80">
   <crs:MainCurve><rdf:Seq><rdf:li>0,0</rdf:li><rdf:li>64,50</rdf:li><rdf:li>192,205</rdf:li><rdf:li>255,240</rdf:li></rdf:Seq></crs:MainCurve>
   <crs:BlueCurve><rdf:Seq><rdf:li>0,10</rdf:li><rdf:li>255,255</rdf:li></rdf:Seq></crs:BlueCurve>
   <crs:GreenCurve><rdf:Seq><rdf:li>0,0</rdf:li><rdf:li>255,255</rdf:li></rdf:Seq></crs:GreenCurve>
   <crs:LocalPointColors><rdf:Seq>
    <rdf:li>0.600000, 0.300000, 0.500000, 0.200000, 0.400000, -0.100000, 0.500000, 0.000000, 0.333333, 0.666667, 1.000000, 0.000000, 0.120000, 0.480000, 1.000000, 0.200000, 0.550000, 0.910000, 1.000000</rdf:li>
    <rdf:li>-1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000</rdf:li>
   </rdf:Seq></crs:LocalPointColors>
   <crs:LocalColorVariance><rdf:Seq><rdf:li>-30.000000</rdf:li></rdf:Seq></crs:LocalColorVariance>
   <crs:CorrectionMasks><rdf:Seq>
    <rdf:li crs:What="Mask/Image" crs:MaskActive="true" crs:MaskName="Background 1" crs:MaskBlendMode="0" crs:MaskInverted="false"
      crs:MaskValue="1" crs:MaskVersion="1" crs:MaskSubType="0" crs:MaskSubCategoryID="22"/>
   </rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
 </rdf:Seq></crs:MaskGroupBasedCorrections>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;

#[test]
fn a_lightroom_preset_with_local_curve_and_point_color_imports_whole() {
    let d = lightcraft_meta::parse_xmp(PRESET).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    assert!(unmapped.is_empty(), "nothing left behind: {unmapped:?}");
    let s = lightcraft_develop::DevelopSettings::default().merged(&partial).unwrap();
    let m = &s.masks[0];
    assert_eq!(m.curve.master.len(), 4);
    assert!((m.curve.master[1].x - 64.0 / 255.0).abs() < 1e-9 && (m.curve.master[1].y - 50.0 / 255.0).abs() < 1e-9);
    assert_eq!(m.curve.blue.len(), 2);
    assert!(m.curve.green.is_empty(), "a straight channel is no curve");
    assert_eq!(m.curve.refine_saturation, 80.0);
    assert_eq!(m.point_colors.len(), 1, "the unused slot is skipped");
    let p = m.point_colors[0];
    assert_eq!((p.hue_shift, p.sat_shift, p.lum_shift, p.variance), (20.0, 40.0, -10.0, -30.0));
    // and the preset as a whole (as `preset.import` reads it)
    let preset = crate::crs::preset_from_xmp(PRESET, "x").expect("a preset");
    assert_eq!(preset.name, "Warm Back");
    assert!(preset.settings["masks"][0]["point_colors"].is_array());
}

#[test]
fn local_curve_and_point_color_export_to_lightroom_and_read_back() {
    let d = lightcraft_meta::parse_xmp(PRESET).unwrap();
    let (partial, _) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    let mine = lightcraft_develop::DevelopSettings::default().merged(&partial).unwrap();
    let preset = lightcraft_develop::Preset {
        id: "user.t".into(),
        name: "t".into(),
        group: "g".into(),
        settings: json!({"masks": serde_json::to_value(&mine.masks).unwrap()}),
        favorite: false,
        builtin: false,
    };
    let (xmp, _) = crate::preset_xmp::to_xmp(&preset);
    assert!(xmp.contains("<crs:MainCurve>") && xmp.contains("<crs:LocalPointColors>") && xmp.contains("crs:LocalCurveRefineSaturation=\"80"));
    // Lightroom's view of the file: the crs: corrections, read as LightCraft reads Lightroom's
    let back = lightcraft_meta::parse_xmp(&xmp).unwrap();
    let (masks, unmapped) = crate::crs_masks::masks(&back.values, 1.5);
    assert!(unmapped.is_empty(), "{unmapped:?}");
    let got: lightcraft_develop::Mask = serde_json::from_value(masks[0].clone()).unwrap();
    let want = &mine.masks[0];
    assert_eq!(got.curve.master.len(), want.curve.master.len());
    for (a, b) in got.curve.master.iter().zip(&want.curve.master) {
        assert!((a.x - b.x).abs() < 0.5 / 255.0 && (a.y - b.y).abs() < 0.5 / 255.0);
    }
    assert_eq!(got.curve.refine_saturation, want.curve.refine_saturation);
    let (a, b) = (got.point_colors[0], want.point_colors[0]);
    for (x, y) in [(a.lum, b.lum), (a.chroma, b.chroma), (a.hue / 100.0, b.hue / 100.0), (a.sat_shift, b.sat_shift), (a.variance, b.variance)] {
        assert!((x - y).abs() < 0.01, "{a:?} vs {b:?}");
    }
}

#[test]
fn global_point_colors_import_too() {
    let line = "0.847183, 0.282116, 0.585183, 0.772730, -0.654550, -0.127270, 0.500000, 0.000000, 0.330000, 0.670000, 1.000000, 0.000000, 0.100000, 0.460000, 1.000000, 0.060000, 0.610000, 0.970000, 1.000000";
    let xmp = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Version="17.4">
 <crs:PointColors><rdf:Seq><rdf:li>{line}</rdf:li></rdf:Seq></crs:PointColors>
 <crs:ColorVariance><rdf:Seq><rdf:li>-50.000000</rdf:li></rdf:Seq></crs:ColorVariance>
</rdf:Description></rdf:RDF></x:xmpmeta>"#
    );
    let d = lightcraft_meta::parse_xmp(&xmp).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    assert!(unmapped.is_empty(), "{unmapped:?}");
    let s = lightcraft_develop::DevelopSettings::default().merged(&partial).unwrap();
    assert_eq!(s.point_colors.len(), 1);
    let p = s.point_colors[0];
    assert_eq!(p.variance, -50.0);
    // a yellowed-grass sample (48° in sRGB hue) shifted towards green and desaturated
    assert!(p.hue > 60.0 && p.hue < 110.0 && p.hue_shift > 70.0 && p.sat_shift < -60.0, "{p:?}");
}
