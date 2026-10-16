pub mod dashboard;
pub mod sidebar;

#[derive(thiserror::Error, Debug, Clone)]
pub enum DashboardError {
    #[error("Fetch error: {0}")]
    Fetch(String, Option<uuid::Uuid>),
    #[error("Pane set error: {0}")]
    PaneSet(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}
