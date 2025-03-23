use exchanges::{Ticker, adapter::Exchange};
use iced_core::{Point, Size};
use serde::{Deserialize, Serialize};

use crate::{Layout, Theme};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layouts {
    pub layouts: Vec<Layout>,
    pub active_layout: String,
}

pub use super::timezone::UserTimezone;
pub use super::{ScaleFactor, Sidebar};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct State {
    pub layout_manager: Layouts,
    pub selected_theme: Theme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub scale_factor: ScaleFactor,
}

impl State {
    pub fn from_parts(
        layout_manager: Layouts,
        selected_theme: Theme,
        favorited_tickers: Vec<(Exchange, Ticker)>,
        size: Option<Size>,
        position: Option<Point>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        scale_factor: ScaleFactor,
    ) -> Self {
        State {
            layout_manager,
            selected_theme: Theme(selected_theme.0),
            favorited_tickers,
            window_size: size.map(|s| (s.width, s.height)),
            window_position: position.map(|p| (p.x, p.y)),
            timezone,
            sidebar,
            scale_factor,
        }
    }
}
