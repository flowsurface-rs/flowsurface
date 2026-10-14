use serde::{Deserialize, Serialize};

/// Combined network configuration.
///
/// Both settings take effect after a restart of the application.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Network {
    pub proxy: Option<exchange::proxy::Proxy>,
    pub server_url: Option<String>,
    /// Bearer token for the market-data server.
    /// Stored in the system keychain, never persisted to JSON.
    #[serde(skip)]
    pub server_auth_token: Option<String>,
    pub trade_fetch_mode: TradeFetchMode,
}

impl Network {
    /// Return a copy suitable for disk persistence (proxy auth stripped).
    /// Auth credentials are stored separately in the system keychain.
    pub fn for_persistence(&self) -> Self {
        Self {
            proxy: self.proxy.clone().map(|p| p.without_auth()),
            ..self.clone()
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
    /// Remote Arrow IPC market-data server.
    Server,
}

impl std::fmt::Display for TradeFetchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "Off"),
            Self::Exchange => write!(f, "Exchange"),
            Self::Server => write!(f, "Server"),
        }
    }
}
