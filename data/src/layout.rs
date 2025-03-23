pub use dashboard::Dashboard;
pub use pane::Pane;
use serde::{Deserialize, Serialize};

pub mod dashboard;
pub mod pane;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub dashboard: Dashboard,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            dashboard: Dashboard::default(),
        }
    }
}
