use serde::{Deserialize, Serialize};

use super::{WindowSpec, pane::Pane};
use crate::layout::pane::ok_or_default;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dashboard {
    #[serde(deserialize_with = "ok_or_default")]
    pub pane: Pane,
    #[serde(deserialize_with = "ok_or_default")]
    pub popout: Vec<(Pane, WindowSpec)>,
    pub trade_fetch_enabled: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            pane: Pane::default(),
            popout: vec![],
            trade_fetch_enabled: false,
        }
    }
}
