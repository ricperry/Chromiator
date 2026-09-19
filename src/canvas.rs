//! GTK-independent canvas geometry and site hit semantics.

/// Result of interpreting a primary click on the Voronoi canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasSiteAction {
    Ignore,
    Select(u64),
    Add,
}

/// Whether a point in logical pixels hits the 14-pixel site marker radius.
pub fn marker_hit_test(dx: f64, dy: f64) -> bool {
    dx.powi(2) + dy.powi(2) <= 14.0_f64.powi(2)
}

/// Convert a logical canvas x coordinate to a normalized split position.
pub fn divider_from_canvas_x(x: f64, canvas_width: i32) -> f64 {
    if canvas_width <= 0 {
        0.5
    } else {
        (x / f64::from(canvas_width)).clamp(0.0, 1.0)
    }
}

/// Test the 12-logical-pixel direct-manipulation target around the split divider.
pub fn split_divider_hit(x: f64, canvas_width: i32, divider: f64) -> bool {
    canvas_width > 0 && (x - f64::from(canvas_width) * divider).abs() <= 12.0
}

/// Chooses selection for a hit site and addition for empty artwork.
pub fn canvas_site_action(hit: Option<u64>) -> CanvasSiteAction {
    hit.map_or(CanvasSiteAction::Add, CanvasSiteAction::Select)
}

/// Returns the creator-facing one-based label for a stable site row.
pub fn site_label(index: usize) -> String {
    format!("Site {}", index + 1)
}

/// Returns the semantic label exposed to assistive technology.
pub fn accessible_site_label(index: usize) -> String {
    site_label(index)
}
