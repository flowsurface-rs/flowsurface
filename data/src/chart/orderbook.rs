use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub show_spread: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_spread: true,
        }
    }
}