//! Control-surface icons (`assets/surface-icons`, the same pack the surface plugins draw on keys)
//! as egui textures, for Settings ▸ Controls and the on-screen readout. Rasterised once per name
//! and size and kept for the session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use lightcraft_surface as ls;

type Cache = Arc<Mutex<HashMap<(String, u32), Option<egui::TextureHandle>>>>;

/// The most icon textures kept (names × sizes); beyond it the cache starts over.
const MAX_CACHED: usize = 1024;

fn cache(ctx: &egui::Context) -> Cache {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::new("surfaceIcons")).clone())
}

/// Rasterise an SVG to `px` × `px` straight RGBA (`None`: not an SVG resvg reads).
pub fn rasterize(svg: &str, px: u32) -> Option<egui::ColorImage> {
    let px = px.clamp(8, 512);
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let s = (px as f32 / w).min(px as f32 / h);
    let dx = (px as f32 - w * s) / 2.0;
    let dy = (px as f32 - h * s) / 2.0;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut pixmap.as_mut());
    // tiny-skia keeps premultiplied alpha, as egui's textures do
    let pixels: Vec<egui::Color32> =
        pixmap.pixels().iter().map(|p| egui::Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha())).collect();
    Some(egui::ColorImage { size: [px as usize, px as usize], source_size: egui::vec2(px as f32, px as f32), pixels })
}

/// The texture of icon `name` at `points` (sized for the screen's pixels).
pub fn texture(ctx: &egui::Context, name: &str, points: f32) -> Option<egui::TextureHandle> {
    let svg = ls::icons::svg(name)?;
    let px = (points * ctx.pixels_per_point()).round().clamp(8.0, 512.0) as u32;
    let c = cache(ctx);
    let mut map = c.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(t) = map.get(&(name.to_string(), px)) {
        return t.clone();
    }
    if map.len() >= MAX_CACHED {
        map.clear();
    }
    let tex = rasterize(svg, px).map(|img| ctx.load_texture(format!("surfaceIcon:{name}:{px}"), img, egui::TextureOptions::LINEAR));
    map.insert((name.to_string(), px), tex.clone());
    tex
}

/// Paint icon `name` into `rect` (nothing when there's no such icon).
pub fn paint(ui: &egui::Ui, name: &str, rect: egui::Rect, tint: egui::Color32) -> bool {
    let side = rect.width().min(rect.height());
    let Some(t) = texture(ui.ctx(), name, side) else { return false };
    let r = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_rasterises_and_bad_svg_does_not() {
        let svg = ls::icons::svg("exposure").expect("exposure icon");
        let img = rasterize(svg, 48).expect("rasterised");
        assert_eq!(img.size, [48, 48]);
        assert!(img.pixels.iter().any(|p| p.a() > 0), "something is drawn");
        assert!(rasterize("not svg", 32).is_none());
        assert!(rasterize("<svg xmlns='http://www.w3.org/2000/svg' width='0' height='0'/>", 32).is_none());
        // every icon of the pack reads
        for n in ls::icons::names() {
            assert!(rasterize(ls::icons::svg(n).unwrap_or_default(), 16).is_some(), "{n}");
        }
    }
}
