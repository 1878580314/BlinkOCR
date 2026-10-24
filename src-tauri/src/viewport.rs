/// OS scaling already maps logical UI pixels to physical screen pixels.
/// Keep a square within the current monitor's usable area; never apply DPI twice.
pub fn square_side(work_width: u32, work_height: u32, scale: f64) -> f64 {
    let available = (work_width.min(work_height) as f64 / scale.max(0.5) - 40.0).max(120.0);
    (available * 0.88).clamp(360.0, 780.0).min(available)
}
