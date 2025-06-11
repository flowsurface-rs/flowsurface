use crate::adapter::StreamError;

use reqwest::{Client, Response};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

pub async fn http_request(
    url: &str,
    source: SourceLimit,
    weight: Option<usize>,
) -> Result<String, StreamError> {
    let response = rate_limited_get(url, source, weight.unwrap_or(1)).await?;
    response.text().await.map_err(StreamError::FetchError)
}

const BYBIT_LIMIT: usize = 600;
const BYBIT_REFILL_RATE: Duration = Duration::from_secs(5);

static BYBIT_LIMITER: LazyLock<Mutex<FixedWindowBucket>> =
    LazyLock::new(|| Mutex::new(FixedWindowBucket::new(BYBIT_LIMIT, BYBIT_REFILL_RATE)));

const BINANCE_SPOT_LIMIT: usize = 6000;
const BINANCE_PERP_LIMIT: usize = 2400;

static BINANCE_SPOT_LIMITER: LazyLock<Mutex<DynamicBucket>> =
    LazyLock::new(|| Mutex::new(DynamicBucket::new(BINANCE_SPOT_LIMIT)));
static BINANCE_LINEAR_LIMITER: LazyLock<Mutex<DynamicBucket>> =
    LazyLock::new(|| Mutex::new(DynamicBucket::new(BINANCE_PERP_LIMIT)));
static BINANCE_INVERSE_LIMITER: LazyLock<Mutex<DynamicBucket>> =
    LazyLock::new(|| Mutex::new(DynamicBucket::new(BINANCE_PERP_LIMIT)));

async fn rate_limited_get(
    url: &str,
    source: SourceLimit,
    weight: usize,
) -> Result<Response, StreamError> {
    if source.is_binance() {
        let mut limiter = match source {
            SourceLimit::BinanceSpot => BINANCE_SPOT_LIMITER.lock().await,
            SourceLimit::BinanceInverse => BINANCE_INVERSE_LIMITER.lock().await,
            SourceLimit::BinanceLinear => BINANCE_LINEAR_LIMITER.lock().await,
            _ => unreachable!(),
        };

        let (wait_time, reason_for_wait_opt) = limiter.prepare_request(weight);

        if let Some(reason_for_wait) = reason_for_wait_opt {
            if wait_time > Duration::ZERO {
                tokio::time::sleep(wait_time).await;
            }
            limiter.finalize_request_after_wait(weight, reason_for_wait);
        }

        let response = HTTP_CLIENT
            .get(url)
            .send()
            .await
            .map_err(StreamError::FetchError)?;

        match response
            .headers()
            .get("x-mbx-used-weight-1m")
            .and_then(|header| header.to_str().ok())
            .and_then(|str| str.parse::<usize>().ok())
        {
            Some(reported_weight) => {
                //println!("{:?}: {}", source, reported_weight);
                limiter.update_weight(reported_weight);
            }
            None => {
                log::warn!("Binance rate limit header missing or invalid for: {}", url);
            }
        }

        let status = response.status();
        if status == 429 || status == 418 {
            eprintln!("Binance API request returned {} for: {}", status, url);
            std::process::exit(1);
        }

        Ok(response)
    } else {
        let (wait_time, need_to_wait) = {
            let mut limiter = BYBIT_LIMITER.lock().await;
            limiter.calculate_wait_time(weight)
        };

        if need_to_wait && wait_time > Duration::ZERO {
            tokio::time::sleep(wait_time).await;

            let mut limiter = BYBIT_LIMITER.lock().await;
            limiter.consume_tokens(weight);
        }

        let response = HTTP_CLIENT
            .get(url)
            .send()
            .await
            .map_err(StreamError::FetchError)?;

        let status = response.status();
        if source == SourceLimit::Bybit && status == 403 {
            eprintln!("Bybit API request returned {} for: {}", status, url);
            std::process::exit(1);
        }

        Ok(response)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// API sources with different rate limits per IP
pub enum SourceLimit {
    /// 6000 request WEIGHT within 1m clock window
    BinanceSpot,
    /// 2400 request WEIGHT within 1m clock window
    BinanceLinear,
    /// 2400 request WEIGHT within 1m clock window
    BinanceInverse,
    /// 600 total requests within 5s ?? window
    Bybit,
}

impl SourceLimit {
    pub fn is_binance(&self) -> bool {
        matches!(
            self,
            SourceLimit::BinanceSpot | SourceLimit::BinanceLinear | SourceLimit::BinanceInverse
        )
    }
}

/// Limiter for a fixed window rate that works on guesstimation
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

    fn calculate_wait_time(&mut self, tokens: usize) -> (Duration, bool) {
        self.refill();

        if self.available_tokens >= tokens {
            self.available_tokens -= tokens;
            return (Duration::ZERO, false);
        }

        let wait_time = self
            .refill_rate
            .saturating_sub(Instant::now().duration_since(self.last_refill));
        (wait_time, true)
    }

    fn consume_tokens(&mut self, tokens: usize) {
        self.refill();
        self.available_tokens -= tokens.min(self.available_tokens);
    }
}

#[derive(Debug, Clone, Copy)]
enum DynamicLimitReason {
    HeaderRate,
    FixedWindowRate,
}

/// Limiter that can be used when source reports the rate-limit usage
///
/// Can fallback to fixed window bucket
struct DynamicBucket {
    max_weight: usize,
    current_used_weight: usize,
    last_updated: Instant,

    // Fixed window fallback for endpoints that don't return headers
    available_tokens: usize,
    last_refill: Instant,
    window_duration: Duration,
}

impl DynamicBucket {
    fn new(max_weight: usize) -> Self {
        Self {
            max_weight,
            current_used_weight: 0,
            last_updated: Instant::now(),

            available_tokens: max_weight,
            last_refill: Instant::now(),
            window_duration: Duration::from_secs(60),
        }
    }

    fn update_weight(&mut self, new_weight: usize) {
        if new_weight > 0 {
            self.current_used_weight = new_weight;
            self.last_updated = Instant::now();
        }
    }

    fn refill_fixed_window(&mut self) {
        if let Ok(current_time) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        {
            let now = Instant::now();
            let seconds_in_current_minute = current_time.as_secs() % 60;

            let elapsed = now.duration_since(self.last_refill);
            if elapsed >= Duration::from_secs(60) || seconds_in_current_minute < 1 {
                self.available_tokens = self.max_weight;
                self.last_refill = now;
            }
        }
    }

    fn prepare_request(&mut self, weight: usize) -> (Duration, Option<DynamicLimitReason>) {
        let now = Instant::now();
        let elapsed_since_last_update = now.duration_since(self.last_updated);

        let can_use_header_data =
            elapsed_since_last_update <= self.window_duration && self.current_used_weight > 0;

        if can_use_header_data {
            let available_weight = self.max_weight.saturating_sub(self.current_used_weight);

            if available_weight >= weight {
                self.current_used_weight += weight;
                self.last_updated = now;
                (Duration::ZERO, None)
            } else {
                if let Ok(current_time) =
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                {
                    let seconds_in_current_minute = current_time.as_secs() % 60;
                    let time_until_next_minute =
                        Duration::from_secs(60 - seconds_in_current_minute);

                    let wait_time =
                        time_until_next_minute.saturating_add(Duration::from_millis(100));
                    (wait_time, Some(DynamicLimitReason::HeaderRate))
                } else {
                    (Duration::ZERO, Some(DynamicLimitReason::HeaderRate))
                }
            }
        } else {
            self.refill_fixed_window();

            if self.available_tokens >= weight {
                self.available_tokens -= weight;
                (Duration::ZERO, None)
            } else {
                let wait_time = self
                    .window_duration
                    .saturating_sub(now.duration_since(self.last_refill));
                (wait_time, Some(DynamicLimitReason::FixedWindowRate))
            }
        }
    }

    fn finalize_request_after_wait(&mut self, weight: usize, reason: DynamicLimitReason) {
        match reason {
            DynamicLimitReason::HeaderRate => {
                self.current_used_weight = weight;
                self.last_updated = Instant::now();
            }
            DynamicLimitReason::FixedWindowRate => {
                self.refill_fixed_window();
                self.available_tokens -= weight.min(self.available_tokens);
            }
        }
    }
}
