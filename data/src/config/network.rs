use serde::{Deserialize, Serialize};

/// Combined network configuration.
///
/// Both settings take effect after a restart of the application.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Network {
    pub proxy_cfg: Option<exchange::proxy::Proxy>,
    pub trade_fetch_mode: TradeFetchMode,
}

impl Network {
    pub fn new(
        proxy_cfg: Option<exchange::proxy::Proxy>,
        trade_fetch_mode: TradeFetchMode,
    ) -> Self {
        Self {
            proxy_cfg,
            trade_fetch_mode,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
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
        /// Stored in the system keychain, never persisted to JSON.
        #[serde(skip)]
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

    /// Build a `Server` variant from raw input strings.
    ///
    /// Trims whitespace and trailing slashes from the URL.
    /// Empty strings are treated as `None`.
    pub fn from_server_parts(url: &str, auth_token: &str) -> Self {
        let trimmed = url.trim().trim_end_matches('/');
        let auth = auth_token.trim();
        Self::Server {
            url: if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            },
            auth_token: if auth.is_empty() {
                None
            } else {
                Some(auth.to_string())
            },
        }
    }
}
