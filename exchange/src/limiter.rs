use crate::adapter::StreamError;
use crate::limiter;
use reqwest::Response;

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

pub async fn http_request(
    url: &str,
    source: SourceLimit,
    weight: Option<usize>,
) -> Result<String, StreamError> {
    let response = rate_limited_get(url, source, weight.unwrap_or(1)).await?;
    response.text().await.map_err(StreamError::FetchError)
}

async fn rate_limited_get(
    url: &str,
    source: SourceLimit,
    weight: usize,
) -> Result<Response, StreamError> {
    limiter::acquire_permit(source, weight).await;

    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(StreamError::FetchError)?;

    let status = response.status();
    // These errors mostly related to IP/rate limiting/location restrictions
    // They may be serious as in they can act as a warning before IP ban;
    // we shouldn't ever end up here, so currently we just terminate the whole app
    // TODO: should probably handle this gracefully on higher level
    match source {
        SourceLimit::BinanceSpot | SourceLimit::BinancePerp => {
            if status == 429 || status == 418 {
                eprintln!("Binance API request returned {} for: {}", status, url);
                std::process::exit(1);
            }
        }
        SourceLimit::Bybit => {
            if status == 403 {
                eprintln!("Bybit API request returned {} for: {}", status, url);
                std::process::exit(1);
            }
        }
    }

    Ok(response)
}

const BYBIT_LIMIT: usize = 600;
const BYBIT_REFILL_RATE: Duration = Duration::from_secs(5);

const BINANCE_SPOT_LIMIT: usize = 6000;
const BINANCE_PERP_LIMIT: usize = 2400;
const BINANCE_REFILL_RATE: Duration = Duration::from_secs(60);

static RATE_LIMITER: Lazy<Mutex<RateLimiter>> = Lazy::new(|| Mutex::new(RateLimiter::new()));

pub async fn acquire_permit(source: SourceLimit, weight: usize) {
    let mut limiter = RATE_LIMITER.lock().await;
    limiter.acquire(source, weight).await;
}

pub async fn update_rate_limit(source: SourceLimit, max_tokens: usize) {
    let mut limiter = RATE_LIMITER.lock().await;
    limiter.update_limit(source, max_tokens);
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
/// API sources with different rate limits per IP
pub enum SourceLimit {
    /// 6000 request WEIGHT within 1m sliding window
    // TODO: handle sliding window properly
    BinanceSpot,
    /// 2400 request WEIGHT within 1m sliding window
    BinancePerp,
    /// 600 total requests within 5s fixed window
    Bybit,
}

#[derive(Debug)]
struct Bucket {
    max_tokens: usize,
    available_tokens: usize,
    last_refill: Instant,
    refill_rate: Duration,
}

impl Bucket {
    fn new(max_tokens: usize, refill_rate: Duration) -> Self {
        Self {
            max_tokens,
            available_tokens: max_tokens,
            last_refill: Instant::now(),
            refill_rate,
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);

        if elapsed >= self.refill_rate {
            self.available_tokens = self.max_tokens;
            self.last_refill = now;
        }
    }

    async fn acquire(&mut self, tokens: usize) {
        self.refill();

        if self.available_tokens >= tokens {
            self.available_tokens -= tokens;
            return;
        }

        let wait_time = self
            .refill_rate
            .saturating_sub(Instant::now().duration_since(self.last_refill));

        log::debug!("Rate limit approaching, waiting {:?}", wait_time);
        sleep(wait_time).await;

        self.refill();
        self.available_tokens -= tokens.min(self.available_tokens);
    }
}

pub struct RateLimiter {
    buckets: HashMap<SourceLimit, Bucket>,
}

impl RateLimiter {
    fn new() -> Self {
        let mut buckets = HashMap::new();

        buckets.insert(
            SourceLimit::BinanceSpot,
            Bucket::new(BINANCE_SPOT_LIMIT, BINANCE_REFILL_RATE),
        );
        buckets.insert(
            SourceLimit::BinancePerp,
            Bucket::new(BINANCE_PERP_LIMIT, BINANCE_REFILL_RATE),
        );
        buckets.insert(
            SourceLimit::Bybit,
            Bucket::new(BYBIT_LIMIT, BYBIT_REFILL_RATE),
        );

        Self { buckets }
    }

    pub async fn acquire(&mut self, source: SourceLimit, weight: usize) {
        if let Some(bucket) = self.buckets.get_mut(&source) {
            bucket.acquire(weight).await;
        }
    }

    pub fn update_limit(&mut self, source: SourceLimit, max_tokens: usize) {
        if let Some(bucket) = self.buckets.get_mut(&source) {
            bucket.max_tokens = max_tokens;
        } else {
            let refill_rate = match source {
                SourceLimit::BinanceSpot | SourceLimit::BinancePerp => BINANCE_REFILL_RATE,
                SourceLimit::Bybit => BYBIT_REFILL_RATE,
            };

            self.buckets
                .insert(source, Bucket::new(max_tokens, refill_rate));
        }
    }
}
