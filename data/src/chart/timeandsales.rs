use serde::{Deserialize, Serialize};

const DEFAULT_BUFFER_SIZE: usize = 900;

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
    #[serde(default = "default_buffer_filter")]
    pub buffer_filter: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
            buffer_filter: DEFAULT_BUFFER_SIZE,
        }
    }
}

fn default_buffer_filter() -> usize {
    DEFAULT_BUFFER_SIZE
}
