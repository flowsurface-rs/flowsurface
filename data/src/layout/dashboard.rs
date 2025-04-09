use exchange::{Ticker, adapter::Exchange};
use serde::{Deserialize, Serialize};

use super::{WindowSpec, pane::Pane};
use crate::layout::pane::ok_or_default;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Dashboard {
    #[serde(deserialize_with = "ok_or_default")]
    pub pane: Pane,
    #[serde(deserialize_with = "ok_or_default")]
    pub popout: Vec<(Pane, WindowSpec)>,
    #[serde(default)]
    pub enabled_audio_streams: Vec<(Exchange, Ticker)>,
    pub trade_fetch_enabled: bool,
}
