pub mod aggr;
pub mod chart;
pub mod config;
pub mod layout;

pub use config::ScaleFactor;
pub use config::sidebar::{self, Sidebar};
pub use config::state::{Layouts, State};
pub use config::theme::Theme;
pub use config::timezone::UserTimezone;

pub use layout::{Dashboard, Layout, Pane};

#[derive(thiserror::Error, Debug, Clone)]
pub enum InternalError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Layout error: {0}")]
    Layout(String),
}
