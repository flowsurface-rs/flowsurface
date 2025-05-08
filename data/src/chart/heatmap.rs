use serde::{Deserialize, Serialize};

pub const MIN_SCALING: f32 = 0.6;
pub const MAX_SCALING: f32 = 1.2;

pub const MAX_CELL_WIDTH: f32 = 12.0;
pub const MIN_CELL_WIDTH: f32 = 1.0;

pub const MAX_CELL_HEIGHT: f32 = 10.0;
pub const MIN_CELL_HEIGHT: f32 = 1.0;

pub const DEFAULT_CELL_WIDTH: f32 = 3.0;

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
    pub order_size_filter: f32,
    pub dynamic_sized_trades: bool,
    pub trade_size_scale: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
            order_size_filter: 0.0,
            dynamic_sized_trades: true,
            trade_size_scale: 100,
        }
    }
}
