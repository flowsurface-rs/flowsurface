use exchange::adapter::{AdapterError, FetchError, MarketKind};
use exchange::unit::price::Price;
use exchange::unit::qty::{QtyNormalization, RawQtyUnit, SizeUnit, volume_size_unit};
use exchange::{TickerInfo, Trade, UnixMs};

use arrow_array::{BooleanArray, Float64Array, Int64Array, RecordBatch};
use arrow_ipc::reader::StreamReader;

use exchange::adapter::{AdapterHandles, Venue};
use exchange::proxy::Proxy;

/// Maximum trades to request per Arrow IPC call.
pub const ARROW_LIMIT: usize = 400_000;

#[derive(Clone)]
pub struct DataSources {
    pub handles: AdapterHandles,
    pub server_client: Option<ServerClient>,
}

impl DataSources {
    /// Build HTTP clients, spawn exchange adapter handles, and optionally
    /// create a market-data server client from the persisted configuration.
    pub fn new(proxy_cfg: Option<&Proxy>, mode: &data::TradeFetchMode) -> Self {
        let (exchange_http, server_http) = Self::build_http_clients(proxy_cfg);
        let server_client = ServerClient::from_mode(&server_http, mode);
        let handles = AdapterHandles::spawn_venues(&exchange_http, Venue::ALL, proxy_cfg);
        Self {
            handles,
            server_client,
        }
    }

    /// Build the two HTTP clients used by the application.
    ///
    /// Returns `(exchange_client, server_client)`.
    ///
    /// * `exchange_client` — for exchange adapters (proxy-aware, no TLS
    ///   relaxation).
    /// * `server_client` — for the user-configured market-data server
    ///   (proxy-aware, accepts invalid TLS certificates for self-signed
    ///   server certs).
    fn build_http_clients(proxy_cfg: Option<&Proxy>) -> (reqwest::Client, reqwest::Client) {
        let mut exchange_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10));
        exchange_builder = exchange::adapter::proxy::try_apply_proxy(exchange_builder, proxy_cfg);
        let exchange_client = exchange_builder
            .build()
            .expect("Failed to build exchange HTTP client");

        let mut server_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .danger_accept_invalid_certs(true);
        server_builder = exchange::adapter::proxy::try_apply_proxy(server_builder, proxy_cfg);
        let server_client = server_builder
            .build()
            .expect("Failed to build server HTTP client");

        (exchange_client, server_client)
    }
}

impl std::fmt::Debug for DataSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataSources")
            .field("handles", &"…")
            .field("server_client", &self.server_client.as_ref().map(|_| "…"))
            .finish()
    }
}

/// A handle to a remote market-data HTTP server.
///
/// The server is expected to expose a `GET /trades.arrow` endpoint that
/// returns Arrow IPC streams of trade data.  Query parameters:
/// `venue`, `market`, `symbol`, `from`, `limit`.
#[derive(Debug, Clone)]
pub struct ServerClient {
    base_url: String,
    client: reqwest::Client,
    auth_token: Option<String>,
}

impl ServerClient {
    /// Create a new client targeting the given base URL
    /// (e.g. `http://127.0.0.1:8080`).
    ///
    /// The `client` is a shared `reqwest::Client` that carries the
    /// application-wide proxy and TLS configuration — it is **not** owned
    /// by this struct.
    ///
    /// An optional bearer token is sent as
    /// `Authorization: Bearer <token>` on every request.
    /// Trailing slashes are stripped. Returns `None` if the URL is empty
    /// or invalid.
    pub fn new(
        base_url: &str,
        auth_token: Option<String>,
        client: reqwest::Client,
    ) -> Option<Self> {
        let trimmed = base_url.trim().trim_end_matches('/');

        if trimmed.is_empty() {
            return None;
        }

        let _ = trimmed
            .parse::<url::Url>()
            .map_err(|e| log::warn!("Invalid server base URL '{trimmed}': {e}"))
            .ok()?;

        Some(Self {
            base_url: trimmed.to_string(),
            client,
            auth_token,
        })
    }

    /// Base URL of the market-data server.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The shared `reqwest::Client` carrying proxy & TLS settings.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Optional bearer token sent as `Authorization: Bearer <token>`.
    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Build a [`ServerClient`] from the shared HTTP client and the current
    /// trade-fetch mode configuration.
    ///
    /// Returns `None` when the mode is not [`TradeFetchMode::Server`] or the
    /// URL is empty/invalid.
    pub fn from_mode(http_client: &reqwest::Client, mode: &data::TradeFetchMode) -> Option<Self> {
        match mode {
            data::TradeFetchMode::Server {
                url: Some(url),
                auth_token,
            } => Self::new(url, auth_token.clone(), http_client.clone()),
            _ => None,
        }
    }

    /// Fetch trades from the server's **Arrow IPC** endpoint (`/trades.arrow`).
    pub async fn fetch_trades_arrow(
        &self,
        ticker_info: TickerInfo,
        from: UnixMs,
        to: UnixMs,
        limit: usize,
    ) -> Result<Vec<Trade>, AdapterError> {
        let (venue, market) = {
            let exchange = ticker_info.exchange();

            let venue = exchange.venue().to_string().to_lowercase();
            let market = exchange.market_type().to_string().to_lowercase();

            (venue, market)
        };
        let symbol = if let Some(display_symbol) = ticker_info.ticker.display_symbol() {
            display_symbol.to_lowercase()
        } else {
            ticker_info.ticker.to_string().to_lowercase()
        };

        let url = format!("{}/trades.arrow", self.base_url());

        log::debug!(
            "Querying server (arrow): {url} | venue={venue} market={market} symbol={symbol} from={from} to={to} limit={limit}",
        );

        let mut request = self
            .http_client()
            .get(&url)
            .query(&[
                ("venue", venue.as_str()),
                ("market", market.as_str()),
                ("symbol", symbol.as_str()),
            ])
            .query(&[("from", from.as_u64().to_string())])
            .query(&[("to", to.as_u64().to_string())])
            .query(&[("limit", limit.to_string())]);

        if let Some(token) = self.auth_token() {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|e| {
            log::warn!("Server arrow request failed: {e}");
            AdapterError::FetchError(FetchError::new(
                "server arrow request failed".to_string(),
                "External data source error. Check logs for details.",
            ))
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            log::warn!("Server returned {status}: {body}");
            return Err(AdapterError::http_status_failed(
                status,
                format!("server (arrow): {body}"),
            ));
        }

        let bytes = response.bytes().await.map_err(|e| {
            log::warn!("Failed to read arrow response body: {e}");
            AdapterError::ParseError(format!("server (arrow): {e}"))
        })?;

        parse_arrow_trades(bytes, &ticker_info)
    }
}

/// Parse raw Arrow IPC stream bytes into a sorted `Vec<Trade>`.
///
/// Expects the Arrow IPC streaming format with the schema:
/// `ts (int64)`, `price (float64)`, `qty (float64)`, `is_sell (bool)`.
fn parse_arrow_trades(
    data: bytes::Bytes,
    ticker_info: &TickerInfo,
) -> Result<Vec<Trade>, AdapterError> {
    let reader = StreamReader::try_new(std::io::Cursor::new(data), None)
        .map_err(|e| AdapterError::ParseError(format!("arrow stream open: {e}")))?;

    let market_kind = ticker_info.exchange().market_type();
    let raw_qty_unit = match market_kind {
        MarketKind::InversePerps => RawQtyUnit::Quote,
        MarketKind::Spot | MarketKind::LinearPerps => RawQtyUnit::Base,
    };

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm =
        QtyNormalization::with_raw_qty_unit(size_in_quote_ccy, *ticker_info, raw_qty_unit);

    let mut trades = Vec::new();
    for result in reader {
        let batch: RecordBatch =
            result.map_err(|e| AdapterError::ParseError(format!("arrow batch: {e}")))?;

        let ts_col = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| AdapterError::ParseError("column 0 (ts) is not Int64".into()))?;
        let price_col = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| AdapterError::ParseError("column 1 (price) is not Float64".into()))?;
        let qty_col = batch
            .column(2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| AdapterError::ParseError("column 2 (qty) is not Float64".into()))?;
        let is_sell_col = batch
            .column(3)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(|| AdapterError::ParseError("column 3 (is_sell) is not Boolean".into()))?;

        for i in 0..batch.num_rows() {
            trades.push(Trade {
                time: UnixMs::new(ts_col.value(i) as u64),
                is_sell: is_sell_col.value(i),
                price: Price::from_f64(price_col.value(i)),
                qty: qty_norm.normalize_qty(qty_col.value(i), price_col.value(i)),
            });
        }
    }

    // The server should order via `ORDER BY ts ASC` as
    // the cursor-based paging loop depends on it — verify in debug builds.
    debug_assert!(
        trades.windows(2).all(|w| w[0].time <= w[1].time),
        "server returned trades out of order"
    );

    Ok(trades)
}
