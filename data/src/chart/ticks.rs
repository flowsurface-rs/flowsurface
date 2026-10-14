pub mod x;
pub mod y;

/// Safety cap on the number of grid lines produced by any strategy.
pub const MAX_GRID_LINES: usize = 1000;

/// Vertical budget per Y-axis label, as a multiple of the text size.
pub const Y_LABEL_DENSITY: f32 = 3.0;
/// Denser budget for the shorter indicator panes (legacy value).
pub const Y_LABEL_DENSITY_INDICATOR: f32 = 2.5;

/// Number of Y-axis labels that fit in `height` pixels at `text_size` for a
/// given `density` (one label per `density` text-size units of height).
pub fn y_labels_that_fit(height: f32, text_size: f32, density: f32) -> usize {
    (height / (text_size * density)) as usize
}

/// Horizontal pixel spacing reserved per time label at `text_size`. The
/// single source for x-axis label density (kline + comparison time axes).
pub fn x_label_spacing_px(text_size: f32) -> f32 {
    text_size * 7.2
}

/// Number of time labels that fit in a `width`-wide axis at `text_size`,
/// used as the step-selection budget.
pub fn x_labels_that_fit(width: f32, text_size: f32) -> usize {
    ((width / x_label_spacing_px(text_size)).floor() as usize).max(2)
}
