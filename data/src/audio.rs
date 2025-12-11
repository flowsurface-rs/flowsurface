use crate::util::ok_or_default;
use exchange::SerTicker;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Threshold {
    Count(usize),
    Qty(f32),
}

impl std::fmt::Display for Threshold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Threshold::Count(count) => write!(f, "Count based: {}", count),
            Threshold::Qty(qty) => write!(f, "Qty based: {:.2}", qty),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct StreamCfg {
    pub enabled: bool,
    pub threshold: Threshold,
}

impl Default for StreamCfg {
    fn default() -> Self {
        StreamCfg {
            enabled: true,
            threshold: Threshold::Count(10),
        }
    }
}

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AudioStream {
    #[serde(deserialize_with = "ok_or_default")]
    pub streams: FxHashMap<SerTicker, StreamCfg>,
    #[serde(deserialize_with = "ok_or_default")]
    pub volume: Option<f32>,
}
