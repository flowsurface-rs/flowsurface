use crate::adapter::StreamError;

use reqwest::{Client, Response};
use std::time::{Duration, Instant};
use std::{collections::VecDeque, sync::LazyLock};
use tokio::sync::Mutex;
use tokio::time::sleep;

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

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
    acquire_permit(source, weight).await;

    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(StreamError::FetchError)?;

    if SourceLimit::BinancePerp == source || SourceLimit::BinanceSpot == source {
        let headers = response.headers();

        let weight = headers
            .get("x-mbx-used-weight-1m")
            .ok_or_else(|| StreamError::ParseError("Missing rate limit header".to_string()))?
            .to_str()
            .map_err(|e| StreamError::ParseError(format!("Invalid header value: {e}")))?
            .parse::<i32>()
            .map_err(|e| StreamError::ParseError(format!("Invalid weight value: {e}")))?;

        log::debug!("used weight for binance: {weight}");
    }

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

static RATE_LIMITER: LazyLock<Mutex<RateLimiter>> =
    LazyLock::new(|| Mutex::new(RateLimiter::new()));

async fn acquire_permit(source: SourceLimit, weight: usize) {
    let mut limiter = RATE_LIMITER.lock().await;
    limiter.acquire(source, weight).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// API sources with different rate limits per IP
pub enum SourceLimit {
    /// 6000 request WEIGHT within 1m sliding window
    BinanceSpot,
    /// 2400 request WEIGHT within 1m sliding window
    BinancePerp,
    /// 600 total requests within 5s fixed window
    Bybit,
}

struct RateLimiter {
    fixed_window: [FixedWindowBucket; 1],
    sliding_window: [SlidingWindowBucket; 2],
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            fixed_window: [FixedWindowBucket::new(BYBIT_LIMIT, BYBIT_REFILL_RATE)],
            sliding_window: [
                SlidingWindowBucket::new(BINANCE_SPOT_LIMIT, BINANCE_REFILL_RATE),
                SlidingWindowBucket::new(BINANCE_PERP_LIMIT, BINANCE_REFILL_RATE),
            ],
        }
    }

    async fn acquire(&mut self, source: SourceLimit, weight: usize) {
        match source {
            SourceLimit::Bybit => {
                self.fixed_window[0].acquire(weight).await;
            }
            SourceLimit::BinanceSpot => {
                self.sliding_window[0].acquire(weight).await;
            }
            SourceLimit::BinancePerp => {
                self.sliding_window[1].acquire(weight).await;
            }
        }
    }
}

#[derive(Debug)]
struct FixedWindowBucket {
    max_tokens: usize,
    available_tokens: usize,
    last_refill: Instant,
    refill_rate: Duration,
}

impl FixedWindowBucket {
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

        log::warn!("Rate limit approaching, waiting {:?}", wait_time);
        sleep(wait_time).await;

        self.refill();
        self.available_tokens -= tokens.min(self.available_tokens);
    }
}

struct SlidingWindowBucket {
    max_requests: usize,
    window_duration: Duration,
    request_timestamps: VecDeque<Instant>,
}

impl SlidingWindowBucket {
    fn new(max_requests: usize, window_duration: Duration) -> Self {
        Self {
            max_requests,
            window_duration,
            request_timestamps: VecDeque::with_capacity(max_requests),
        }
    }

    #[allow(dead_code)]
    pub fn available_tokens(&mut self) -> usize {
        let now = Instant::now();
        let window_start = now - self.window_duration;

        while let Some(timestamp) = self.request_timestamps.front() {
            if *timestamp < window_start {
                self.request_timestamps.pop_front();
            } else {
                break;
            }
        }

        self.max_requests
            .saturating_sub(self.request_timestamps.len())
    }

    async fn acquire(&mut self, tokens: usize) {
        let now = Instant::now();
        let window_start = now - self.window_duration;

        while let Some(timestamp) = self.request_timestamps.front() {
            if *timestamp < window_start {
                self.request_timestamps.pop_front();
            } else {
                break;
            }
        }

        while self.request_timestamps.len() + tokens > self.max_requests {
            if let Some(oldest) = self.request_timestamps.front() {
                let exit_time = *oldest + self.window_duration;
                let wait_time = exit_time
                    .saturating_duration_since(now)
                    .max(Duration::from_millis(10));

                log::warn!("Rate limit hit, waiting {:?}", wait_time);
                sleep(wait_time).await;

                let window_start = Instant::now() - self.window_duration;

                while let Some(timestamp) = self.request_timestamps.front() {
                    if *timestamp < window_start {
                        self.request_timestamps.pop_front();
                    } else {
                        break;
                    }
                }
            } else {
                break;
            }
        }

        for _ in 0..tokens {
            self.request_timestamps.push_back(Instant::now());
        }
    }
}
