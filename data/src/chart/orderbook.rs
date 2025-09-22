use serde::{Deserialize, Serialize};
use std::time::Duration;

const TRADE_RETENTION_MS: u64 = 8 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub show_spread: bool,
    pub trade_retention: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_spread: false,
            trade_retention: Duration::from_millis(TRADE_RETENTION_MS),
        }
    }
}
