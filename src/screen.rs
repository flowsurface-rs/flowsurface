pub mod dashboard;

#[derive(thiserror::Error, Debug, Clone)]
pub enum DashboardError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Pane set error: {0}")]
    PaneSet(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct ConfirmDialog<M> {
    pub message: String,
    pub on_confirm: Box<M>,
    pub on_confirm_btn_text: Option<String>,
}

impl<M> ConfirmDialog<M> {
    pub fn new(message: String, on_confirm: Box<M>) -> Self {
        Self {
            message,
            on_confirm,
            on_confirm_btn_text: None,
        }
    }

    pub fn with_confirm_btn_text(mut self, on_confirm_btn_text: String) -> Self {
        self.on_confirm_btn_text = Some(on_confirm_btn_text);
        self
    }
}
