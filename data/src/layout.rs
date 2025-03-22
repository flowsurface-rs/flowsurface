use serde::{Deserialize, Serialize};

use crate::pane::SerializablePane;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableLayout {
    pub name: String,
    pub dashboard: SerializableDashboard,
}

impl Default for SerializableLayout {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            dashboard: SerializableDashboard::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableLayouts {
    pub layouts: Vec<SerializableLayout>,
    pub active_layout: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerializableDashboard {
    pub pane: SerializablePane,
    pub popout: Vec<(SerializablePane, (f32, f32), (f32, f32))>,
    pub trade_fetch_enabled: bool,
}

impl Default for SerializableDashboard {
    fn default() -> Self {
        Self {
            pane: SerializablePane::Starter,
            popout: vec![],
            trade_fetch_enabled: false,
        }
    }
}
