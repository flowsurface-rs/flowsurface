use super::ScaleFactor;
use super::sidebar::Sidebar;
use super::timezone::UserTimezone;
use crate::layout::WindowSpec;
use crate::{AudioStream, Layout, Theme};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Layouts {
    pub layouts: Vec<Layout>,
    pub active_layout: Option<String>,
}

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub layout_manager: Layouts,
    pub selected_theme: Theme,
    pub custom_theme: Option<Theme>,
    pub main_window: Option<WindowSpec>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub scale_factor: ScaleFactor,
    pub audio_cfg: AudioStream,
    pub trade_fetch_mode: TradeFetchMode,
    pub size_in_quote_ccy: exchange::SizeUnit,
    pub proxy_cfg: Option<exchange::proxy::Proxy>,
}

impl State {
    pub fn from_parts(
        layout_manager: Layouts,
        selected_theme: Theme,
        custom_theme: Option<Theme>,
        main_window: Option<WindowSpec>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        scale_factor: ScaleFactor,
        audio_cfg: AudioStream,
        trade_fetch_mode: TradeFetchMode,
        volume_size_unit: exchange::SizeUnit,
        proxy_cfg: Option<exchange::proxy::Proxy>,
    ) -> Self {
        State {
            layout_manager,
            selected_theme: Theme(selected_theme.0),
            custom_theme: custom_theme.map(|t| Theme(t.0)),
            main_window,
            timezone,
            sidebar,
            scale_factor,
            audio_cfg,
            trade_fetch_mode,
            size_in_quote_ccy: volume_size_unit,
            proxy_cfg,
        }
    }
}

/// Controls how historical trade data is fetched for footprint charts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeFetchMode {
    #[default]
    Off,
    /// Direct exchange API only (Binance spot/linear/inverse).
    Exchange,
    Server {
        /// Base URL of the market-data server (e.g. `http://127.0.0.1:8080`).
        /// `None` means the server mode is selected but no URL is configured yet.
        url: Option<String>,
        /// Optional bearer token sent as `Authorization: Bearer <token>` on every request.
        auth_token: Option<String>,
    },
}

impl std::fmt::Display for TradeFetchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "Off"),
            Self::Exchange => write!(f, "Exchange"),
            Self::Server { .. } => write!(f, "Server"),
        }
    }
}

impl TradeFetchMode {
    /// Returns the server URL if this is the `Server` variant.
    pub fn server_url(&self) -> Option<&str> {
        match self {
            Self::Server { url, .. } => url.as_deref(),
            _ => None,
        }
    }

    /// Returns the server auth token if this is the `Server` variant.
    pub fn server_auth_token(&self) -> Option<&str> {
        match self {
            Self::Server { auth_token, .. } => auth_token.as_deref(),
            _ => None,
        }
    }
}
