pub use data::chart::kline::DEFAULT_HORIZONTAL_SPACING_PX as DEFAULT_BAR_SPACING_PX;

use data::chart::kline::{
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
