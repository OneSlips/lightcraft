//! Write procedural ColorChecker photos (DNG) for trying Photo > Create Camera Profile from Color
//! Chart… without a real chart: `cargo run -p lightcraft-engine --example chart_dngs -- OUT_DIR`
//! writes CHART_A.dng (tungsten) and CHART_D65.dng (daylight). No photograph is involved.

fn main() -> Result<(), String> {
    let out = std::env::args().nth(1).ok_or("usage: chart_dngs OUT_DIR")?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let corners = lightcraft_engine::chart_profiling::camprof::ChartCorners::from_rect(330.0, 230.0, 540.0, 360.0);
    for light in ["A", "D65"] {
        let dng = lightcraft_engine::chart_profiling::demo_chart_dng(1200, 800, &corners, light, true)?;
        let path = std::path::Path::new(&out).join(format!("CHART_{light}.dng"));
        std::fs::write(&path, dng).map_err(|e| e.to_string())?;
        println!("{}", path.display());
    }
    Ok(())
}
