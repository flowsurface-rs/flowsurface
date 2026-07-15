use exchange::adapter::{AdapterError, Exchange, FetchError, MarketKind};
use exchange::unit::price::Price;
use exchange::unit::qty::{QtyNormalization, RawQtyUnit, SizeUnit, volume_size_unit};
use exchange::{TickerInfo, Trade, UnixMs};

use serde::Deserialize;

/// JSON shape returned by the server's `GET /trades` endpoint.
///
/// ```json
/// { "exchange": "binance", "symbol": "btcusdt", "ts": 123, "price": 1.0, "qty": 2.0, "is_sell": false }
/// ```
#[derive(Debug, Deserialize)]
struct ServerTrade {
    #[allow(dead_code)]
    exchange: String,
    #[allow(dead_code)]
    symbol: String,
    ts: u64,
    price: f64,
    qty: f64,
    is_sell: bool,
}

#[derive(Debug, Deserialize)]
struct TradesResponse {
    trades: Vec<ServerTrade>,
}

/// A handle to a remote market-data HTTP server.
///
/// The server is expected to expose a `GET /trades` endpoint that accepts
/// `venue`, `market`, `symbol`, `from`, `to`, and `limit` query parameters
/// and returns a JSON body matching [`TradesResponse`].
#[derive(Clone)]
pub struct ServerClient {
    base_url: String,
    client: reqwest::Client,
    auth_token: Option<String>,
}

impl ServerClient {
    /// Create a new client targeting the given base URL (e.g. `http://127.0.0.1:8080`).
    ///
    /// An optional bearer token is sent as `Authorization: Bearer <token>` on every request.
    /// Trailing slashes are stripped. Returns `None` if the URL is empty or invalid.
    pub fn new(base_url: &str, auth_token: Option<String>) -> Option<Self> {
        let trimmed = base_url.trim().trim_end_matches('/');

        if trimmed.is_empty() {
            return None;
        }

        // Validate that it parses as a real URL.
        let _ = trimmed
            .parse::<url::Url>()
            .map_err(|e| log::warn!("Invalid server base URL '{trimmed}': {e}"))
            .ok()?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| log::warn!("Failed to build server HTTP client: {e}"))
            .ok()?;

        Some(Self {
            base_url: trimmed.to_string(),
            client,
            auth_token,
        })
    }

    /// Fetch a batch of trades from the server for the given ticker starting
    /// from `from`, returning up to `limit` trades.
    ///
    /// Trades are returned in **ascending** order by timestamp (oldest first),
    /// matching the convention of the direct exchange fetch path.
    pub async fn fetch_trades(
        &self,
        ticker_info: TickerInfo,
        from: UnixMs,
        limit: usize,
    ) -> Result<Vec<Trade>, AdapterError> {
        let (venue, market) = venue_market_strings(ticker_info.exchange());
        let symbol = ticker_info.ticker.to_string().to_lowercase();

        let url = format!("{}/trades", self.base_url);

        log::debug!(
            "Querying server: {url} | venue={venue} market={market} symbol={symbol} from={from} limit={limit}",
        );

        let mut request = self
            .client
            .get(&url)
            .query(&[
                ("venue", venue.as_str()),
                ("market", market.as_str()),
                ("symbol", symbol.as_str()),
            ])
            .query(&[("from", from.as_u64().to_string())])
            .query(&[("limit", limit.to_string())]);

        if let Some(ref token) = self.auth_token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|e| {
            log::warn!("Server request failed: {e}");
            AdapterError::FetchError(FetchError::new(
                "server request failed".to_string(),
                "External data source error. Check logs for details.",
            ))
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            log::warn!("Server returned {status}: {body}");
            return Err(AdapterError::http_status_failed(
                status,
                format!("server: {body}"),
            ));
        }

        let parsed: TradesResponse = response.json().await.map_err(|e| {
            log::warn!("Failed to parse server response: {e}");
            AdapterError::ParseError(format!("server: {e}"))
        })?;

        let market_kind = ticker_info.exchange().market_type();
        let raw_qty_unit = match market_kind {
            MarketKind::InversePerps => RawQtyUnit::Quote,
            MarketKind::Spot | MarketKind::LinearPerps => RawQtyUnit::Base,
        };

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
        let qty_norm =
            QtyNormalization::with_raw_qty_unit(size_in_quote_ccy, ticker_info, raw_qty_unit);

        let mut trades: Vec<Trade> = parsed
            .trades
            .into_iter()
            .map(|ct| Trade {
                time: UnixMs::new(ct.ts),
                is_sell: ct.is_sell,
                price: Price::from_f64(ct.price),
                qty: qty_norm.normalize_qty(ct.qty, ct.price),
            })
            .collect();

        trades.sort_by_key(|t| t.time);

        Ok(trades)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Map `Exchange` to the `(venue, market)` strings expected by
/// the server's query parameters.
fn venue_market_strings(exchange: Exchange) -> (String, String) {
    let venue = exchange.venue().to_string().to_lowercase();
    let market = exchange.market_type().to_string().to_lowercase();

    (venue, market)
}
