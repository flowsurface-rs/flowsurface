use exchanges::{Ticker, adapter::Exchange};
use serde::{Deserialize, Serialize};

use crate::layout::WindowSpec;
use crate::{Layout, Theme};

use super::ScaleFactor;
use super::sidebar::Sidebar;
use super::timezone::UserTimezone;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Layouts {
    pub layouts: Vec<Layout>,
    pub active_layout: String,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub layout_manager: Layouts,
    pub selected_theme: Theme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub main_window: Option<WindowSpec>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub scale_factor: ScaleFactor,
}

impl State {
    pub fn from_parts(
        layout_manager: Layouts,
        selected_theme: Theme,
        favorited_tickers: Vec<(Exchange, Ticker)>,
        main_window: Option<WindowSpec>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        scale_factor: ScaleFactor,
    ) -> Self {
        State {
            layout_manager,
            selected_theme: Theme(selected_theme.0),
            favorited_tickers,
            main_window,
            timezone,
            sidebar,
            scale_factor,
        }
    }
}
