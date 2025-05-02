use crate::layout::WindowSpec;
use crate::{AudioStream, Layout, Theme};
use exchange::{Ticker, adapter::Exchange};
use serde::{Deserialize, Serialize};

use super::ScaleFactor;
use super::sidebar::Sidebar;
use super::timezone::UserTimezone;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Layouts {
    pub layouts: Vec<Layout>,
    pub active_layout: String,
}

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub layout_manager: Layouts,
    pub selected_theme: Theme,
    pub custom_theme: Option<Theme>,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub main_window: Option<WindowSpec>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub scale_factor: ScaleFactor,
    pub audio_cfg: AudioStream,
}

impl State {
    pub fn from_parts(
        layout_manager: Layouts,
        selected_theme: Theme,
        custom_theme: Option<Theme>,
        favorited_tickers: Vec<(Exchange, Ticker)>,
        main_window: Option<WindowSpec>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        scale_factor: ScaleFactor,
        audio_cfg: AudioStream,
    ) -> Self {
        State {
            layout_manager,
            selected_theme: Theme(selected_theme.0),
            custom_theme: custom_theme.map(|t| Theme(t.0)),
            favorited_tickers,
            main_window,
            timezone,
            sidebar,
            scale_factor,
            audio_cfg,
        }
    }
}
