use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
        }
    }
}
