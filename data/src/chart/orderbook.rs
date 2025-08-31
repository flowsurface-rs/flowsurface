use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub max_levels: usize,
    pub precision: u8,
    pub show_size: bool,
    pub show_spread: bool,
    pub price_grouping: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_levels: 15,
            precision: 4,
            show_size: true,
            show_spread: true,
            price_grouping: 1.0,
        }
    }
}