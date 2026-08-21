pub use data::chart::kline_v2::DEFAULT_HORIZONTAL_SPACING_PX as DEFAULT_BAR_SPACING_PX;

use data::chart::kline_v2::{
    MAX_HORIZONTAL_SPACING_PX as MAX_BAR_SPACING_PX,
    MIN_HORIZONTAL_SPACING_PX as MIN_BAR_SPACING_PX,
};
use iced::mouse;

const PIXELS_PER_SCROLL_LINE: f32 = 40.0;
const BAR_SPACING_SUBPIXEL_UNITS_PER_PX: i32 = 128;
const BAR_SPACING_QUANTUM_PX: f32 = 1.0 / BAR_SPACING_SUBPIXEL_UNITS_PER_PX as f32;

pub(super) const HORIZONTAL_DRAG_SCALE_DELTA_PER_PX: f32 = 0.3 / 9.0;

pub(super) fn normalized_scroll_delta(delta: &mouse::ScrollDelta) -> f32 {
    let delta = match delta {
        mouse::ScrollDelta::Lines { y, .. } => *y,
        mouse::ScrollDelta::Pixels { y, .. } => *y / PIXELS_PER_SCROLL_LINE,
    };

    if delta.is_finite() { delta } else { 0.0 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalScale(pub f32);

impl HorizontalScale {
    pub fn pixels_per_bar(px: f32) -> Self {
        Self(px)
    }

    pub fn as_pixels_per_bar(self) -> f32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BarSpacingPx(i32);

impl BarSpacingPx {
    fn min_units() -> i32 {
        ((MIN_BAR_SPACING_PX / BAR_SPACING_QUANTUM_PX).ceil() as i32).max(1)
    }

    fn max_units() -> i32 {
        ((MAX_BAR_SPACING_PX / BAR_SPACING_QUANTUM_PX).floor() as i32).max(Self::min_units())
    }

    fn from_units(units: i32) -> Self {
        Self(units.clamp(Self::min_units(), Self::max_units()))
    }

    pub(super) fn from_logical(px: f32) -> Self {
        let quantized_units = (px.clamp(MIN_BAR_SPACING_PX, MAX_BAR_SPACING_PX)
            / BAR_SPACING_QUANTUM_PX)
            .round() as i32;

        Self::from_units(quantized_units)
    }

    pub(super) fn as_f32(self) -> f32 {
        self.0 as f32 * BAR_SPACING_QUANTUM_PX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approximately_equal(left: f32, right: f32) {
        assert!((left - right).abs() < 1e-6, "{left} != {right}");
    }

    #[test]
    fn scroll_lines_and_pixels_share_logical_units() {
        let line_delta = normalized_scroll_delta(&mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 });
        let pixel_delta = normalized_scroll_delta(&mouse::ScrollDelta::Pixels { x: 0.0, y: 40.0 });

        assert_approximately_equal(line_delta, 1.0);
        assert_approximately_equal(pixel_delta, line_delta);
    }

    #[test]
    fn small_pixel_scrolls_are_not_forced_to_a_minimum_zoom() {
        let delta = normalized_scroll_delta(&mouse::ScrollDelta::Pixels { x: 0.0, y: 1.0 });

        assert_approximately_equal(delta, 1.0 / PIXELS_PER_SCROLL_LINE);
    }
}
