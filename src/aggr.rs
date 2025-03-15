use exchanges::TickerInfo;
use rust_decimal::{
    Decimal,
    prelude::{FromPrimitive, ToPrimitive},
};
use serde::{Deserialize, Serialize};

pub mod ticks;
pub mod time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct TickMultiplier(pub u16);

impl std::fmt::Display for TickMultiplier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}x", self.0)
    }
}

impl TickMultiplier {
    pub const ALL: [TickMultiplier; 8] = [
        TickMultiplier(1),
        TickMultiplier(2),
        TickMultiplier(5),
        TickMultiplier(10),
        TickMultiplier(25),
        TickMultiplier(50),
        TickMultiplier(100),
        TickMultiplier(200),
    ];

    /// Returns the final tick size after applying the user selected multiplier
    ///
    /// Usually used for price steps in chart scales
    pub fn multiply_with_min_tick_size(&self, ticker_info: TickerInfo) -> f32 {
        let min_tick_size = ticker_info.min_ticksize;

        let multiplier = if let Some(m) = Decimal::from_f32(f32::from(self.0)) {
            m
        } else {
            log::error!("Failed to convert multiplier: {}", self.0);
            return f32::from(self.0) * min_tick_size;
        };

        let decimal_min_tick_size = if let Some(d) = Decimal::from_f32(min_tick_size) {
            d
        } else {
            log::error!("Failed to convert min_tick_size: {}", min_tick_size);
            return f32::from(self.0) * min_tick_size;
        };

        let normalized = multiplier * decimal_min_tick_size.normalize();
        if let Some(tick_size) = normalized.to_f32() {
            let decimal_places = calculate_decimal_places(min_tick_size);
            round_to_decimal_places(tick_size, decimal_places)
        } else {
            log::error!("Failed to calculate tick size for multiplier: {}", self.0);
            f32::from(self.0) * min_tick_size
        }
    }
}

// ticksize rounding helpers
fn calculate_decimal_places(value: f32) -> u32 {
    let s = value.to_string();
    if let Some(decimal_pos) = s.find('.') {
        (s.len() - decimal_pos - 1) as u32
    } else {
        0
    }
}
fn round_to_decimal_places(value: f32, places: u32) -> f32 {
    let factor = 10.0f32.powi(places as i32);
    (value * factor).round() / factor
}
