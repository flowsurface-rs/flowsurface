use serde::{Deserialize, Serialize};

use super::{WindowSpec, pane::Pane};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dashboard {
    pub pane: Pane,
    pub popout: Vec<(Pane, WindowSpec)>,
    pub trade_fetch_enabled: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            pane: Pane::Starter,
            popout: vec![],
            trade_fetch_enabled: false,
        }
    }
}
