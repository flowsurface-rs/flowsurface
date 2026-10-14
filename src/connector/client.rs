use exchange::adapter::{AdapterError, FetchError, MarketKind};
use exchange::unit::price::Price;
use exchange::unit::qty::{QtyNormalization, RawQtyUnit, SizeUnit, volume_size_unit};
use exchange::{TickerInfo, Trade, UnixMs};

use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, RecordBatch};
use arrow_ipc::reader::StreamReader;

use exchange::adapter::{AdapterHandles, Venue};
use exchange::proxy::Proxy;

#[derive(Clone)]
pub struct DataSources {
    pub exchange: AdapterHandles,
    pub server: Option<ServerClient>,
}

impl DataSources {
    /// Build HTTP clients, spawn exchange adapter handles, and optionally
    /// create a market-data server client from the persisted configuration.
    pub fn new(network: &data::Network) -> Self {
        let proxy = network.proxy.as_ref();
        let (exchange_http, server_http) = Self::build_http_clients(proxy);

        let server = ServerClient::from_mode(&server_http, network);
        let exchange = AdapterHandles::spawn_venues(&exchange_http, Venue::ALL, proxy);

        Self { exchange, server }
    }

    /// Build the two HTTP clients used by the application.
    ///
    /// Returns `(exchange_client, server_client)`.
    ///
    /// * `exchange_client`: For exchange adapters (proxy-aware, no TLS
    ///   relaxation).
    /// * `server_client`: For the user-configured market-data server
    ///   (proxy-aware, accepts invalid TLS certificates for self-signed
    ///   server certs).
    ///
    /// # Panics
    ///
    /// Panics if either `reqwest::Client::builder().build()` fails.
    fn build_http_clients(proxy_cfg: Option<&Proxy>) -> (reqwest::Client, reqwest::Client) {
        let mut exchange_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10));
        exchange_builder = exchange::adapter::proxy::try_apply_proxy(exchange_builder, proxy_cfg);
        let exchange_client = exchange_builder.build().unwrap_or_else(|e| {
            log::error!(
                "Fatal: failed to build exchange HTTP client - TLS backend missing, \
                 resource exhaustion, or invalid proxy config?\n  {e}"
            );
            panic!("Failed to build exchange HTTP client: {e}");
        });

        let mut server_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .danger_accept_invalid_certs(true);
        server_builder = exchange::adapter::proxy::try_apply_proxy(server_builder, proxy_cfg);
        let server_client = server_builder.build().unwrap_or_else(|e| {
            log::error!(
                "Fatal: failed to build server HTTP client - TLS backend missing, \
                 resource exhaustion, or invalid proxy config?\n  {e}"
            );
            panic!("Failed to build server HTTP client: {e}");
        });

        (exchange_client, server_client)
    }
}

impl std::fmt::Debug for DataSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataSources")
            .field("exchange", &"…")
            .field("server", &self.server.as_ref().map(|_| "…"))
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
    fn new(base_url: &str, auth_token: Option<String>, client: reqwest::Client) -> Option<Self> {
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
    fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Optional bearer token sent as `Authorization: Bearer <token>`.
    fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Build a [`ServerClient`] from the shared HTTP client and the current
    /// network configuration.
    ///
    /// Returns `None` when the mode is not [`TradeFetchMode::Server`] or the
    /// URL is empty/invalid.
    fn from_mode(http_client: &reqwest::Client, network: &data::Network) -> Option<Self> {
        match &network.trade_fetch_mode {
            data::TradeFetchMode::Server => {
                let url = network.server_url.as_deref()?;
                Self::new(url, network.server_auth_token.clone(), http_client.clone())
            }
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
    ) -> Result<ParsedArrowBatch, AdapterError> {
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

        if let Some(content_type) = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            && content_type != "application/vnd.apache.arrow.stream"
        {
            log::warn!(
                "Server returned unexpected Content-Type '{content_type}', \
                     expected 'application/vnd.apache.arrow.stream'"
            );
        }

        let bytes = response.bytes().await.map_err(|e| {
            log::warn!("Failed to read arrow response body: {e}");
            AdapterError::ParseError(format!("server (arrow): {e}"))
        })?;

        parse_arrow_trades(bytes, &ticker_info)
    }
}

/// Expected Arrow column names in the IPC stream.
const ARROW_TS_COL: &str = "ts";
const ARROW_PRICE_COL: &str = "price";
const ARROW_QTY_COL: &str = "qty";
const ARROW_IS_SELL_COL: &str = "is_sell";

/// Resolved column indices for the four required Arrow fields.
struct TradeColumns {
    ts: usize,
    price: usize,
    qty: usize,
    is_sell: usize,
}

/// Validate the Arrow stream schema up-front: resolve column indices by
/// name, verify the expected Arrow data types, and fail fast on any
/// mismatch before a single row is parsed.
fn validate_arrow_schema(schema: &arrow_schema::Schema) -> Result<TradeColumns, AdapterError> {
    use arrow_schema::DataType;

    let index_of = |name: &str| {
        schema.index_of(name).map_err(|_| {
            AdapterError::ParseError(format!(
                "Arrow stream schema missing required column '{name}'"
            ))
        })
    };

    let cols = TradeColumns {
        ts: index_of(ARROW_TS_COL)?,
        price: index_of(ARROW_PRICE_COL)?,
        qty: index_of(ARROW_QTY_COL)?,
        is_sell: index_of(ARROW_IS_SELL_COL)?,
    };

    let check_type = |idx: usize, name: &str, expected: &DataType| -> Result<(), AdapterError> {
        let field = schema.field(idx);
        if field.data_type() != expected {
            return Err(AdapterError::ParseError(format!(
                "Arrow stream schema column '{name}': expected {expected:?}, got {:?}",
                field.data_type()
            )));
        }
        Ok(())
    };

    check_type(cols.ts, ARROW_TS_COL, &DataType::Int64)?;
    check_type(cols.price, ARROW_PRICE_COL, &DataType::Float64)?;
    check_type(cols.qty, ARROW_QTY_COL, &DataType::Float64)?;
    check_type(cols.is_sell, ARROW_IS_SELL_COL, &DataType::Boolean)?;

    Ok(cols)
}

/// Result of parsing an Arrow IPC trade stream.
///
/// Separates displayable trades from stream-level metadata so the
/// paging loop can distinguish "server returned rows but all were
/// null-filtered" from "server genuinely had no data for this range."
#[derive(Debug)]
pub struct ParsedArrowBatch {
    /// Valid, non-null trades ready for display.
    pub trades: Vec<Trade>,
    /// Total rows received across all batches in the stream.
    pub raw_row_count: usize,
    /// Latest non-null `ts` seen across *all* rows (including rows
    /// filtered out due to nulls in other columns).  Used for cursor
    /// advancement in the paging loop.
    pub last_ts: Option<UnixMs>,
}

/// Parse raw Arrow IPC stream bytes into a [`ParsedArrowBatch`].
///
/// Expects the Arrow IPC streaming format. The four required columns
/// (`ts`, `price`, `qty`, `is_sell`) are looked up by name. Column order
/// is not significant and extra columns are silently ignored.
fn parse_arrow_trades(
    data: bytes::Bytes,
    ticker_info: &TickerInfo,
) -> Result<ParsedArrowBatch, AdapterError> {
    let reader = StreamReader::try_new(std::io::Cursor::new(data), None)
        .map_err(|e| AdapterError::ParseError(format!("arrow stream open: {e}")))?;

    // Resolve column positions by name and validate types once up-front.
    let cols = validate_arrow_schema(&reader.schema())?;

    let market_kind = ticker_info.exchange().market_type();
    let raw_qty_unit = match market_kind {
        MarketKind::InversePerps => RawQtyUnit::Quote,
        MarketKind::Spot | MarketKind::LinearPerps => RawQtyUnit::Base,
    };

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm =
        QtyNormalization::with_raw_qty_unit(size_in_quote_ccy, *ticker_info, raw_qty_unit);

    let mut trades = Vec::new();
    let mut raw_row_count: usize = 0;
    let mut last_ts: Option<UnixMs> = None;
    let mut skipped_total: u64 = 0;

    for result in reader {
        let batch: RecordBatch =
            result.map_err(|e| AdapterError::ParseError(format!("arrow batch: {e}")))?;

        raw_row_count += batch.num_rows();

        let ts_col = batch
            .column(cols.ts)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| {
                AdapterError::ParseError(format!("column '{ARROW_TS_COL}' is not Int64"))
            })?;
        let price_col = batch
            .column(cols.price)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| {
                AdapterError::ParseError(format!("column '{ARROW_PRICE_COL}' is not Float64"))
            })?;
        let qty_col = batch
            .column(cols.qty)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| {
                AdapterError::ParseError(format!("column '{ARROW_QTY_COL}' is not Float64"))
            })?;
        let is_sell_col = batch
            .column(cols.is_sell)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(|| {
                AdapterError::ParseError(format!("column '{ARROW_IS_SELL_COL}' is not Boolean"))
            })?;

        let has_nulls = ts_col.null_count() > 0
            || price_col.null_count() > 0
            || qty_col.null_count() > 0
            || is_sell_col.null_count() > 0;

        if has_nulls {
            let mut skipped_in_batch: u64 = 0;

            for i in 0..batch.num_rows() {
                // Track the latest non-null ts across *all* rows for
                // cursor advancement, even if the row is filtered out.
                if !ts_col.is_null(i) {
                    let ts = ts_col.value(i).max(0) as u64;
                    let candidate = UnixMs::new(ts);
                    last_ts = Some(last_ts.map_or(candidate, |prev| prev.max(candidate)));
                }

                if ts_col.is_null(i)
                    || price_col.is_null(i)
                    || qty_col.is_null(i)
                    || is_sell_col.is_null(i)
                {
                    skipped_in_batch += 1;
                    continue;
                }
                let ts = ts_col.value(i).max(0) as u64;
                trades.push(Trade {
                    time: UnixMs::new(ts),
                    is_sell: is_sell_col.value(i),
                    price: Price::from_f64(price_col.value(i)),
                    qty: qty_norm.normalize_qty(qty_col.value(i), price_col.value(i)),
                });
            }

            skipped_total += skipped_in_batch;
        } else {
            for i in 0..batch.num_rows() {
                let ts = ts_col.value(i).max(0) as u64;
                trades.push(Trade {
                    time: UnixMs::new(ts),
                    is_sell: is_sell_col.value(i),
                    price: Price::from_f64(price_col.value(i)),
                    qty: qty_norm.normalize_qty(qty_col.value(i), price_col.value(i)),
                });
            }
        }
    }

    // Fast path: if no nulls were encountered, last_ts is simply the
    // timestamp of the last valid trade.
    if last_ts.is_none() {
        last_ts = trades.last().map(|t| t.time);
    }

    if skipped_total > 0 {
        log::warn!(
            "Arrow stream for {} ({}): skipped {skipped_total} of {raw_row_count} row(s) with nulls in required fields",
            ticker_info.ticker,
            ticker_info.exchange(),
        );
    }

    if trades.is_empty() {
        log::info!(
            "Arrow stream contained no valid trades for {} ({}) (raw rows: {raw_row_count})",
            ticker_info.ticker,
            ticker_info.exchange(),
        );
    }

    Ok(ParsedArrowBatch {
        trades,
        raw_row_count,
        last_ts,
    })
}
