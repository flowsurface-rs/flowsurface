use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

static RATE_LIMITER: Lazy<Arc<Mutex<RateLimiter>>> =
    Lazy::new(|| Arc::new(Mutex::new(RateLimiter::new())));

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
struct RateBucket {
    max_tokens: usize,
    available_tokens: usize,
    last_refill: Instant,
    refill_rate: Duration,
}

impl RateBucket {
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
    buckets: HashMap<SourceLimit, RateBucket>,
}

impl RateLimiter {
    fn new() -> Self {
        let mut buckets = HashMap::new();

        buckets.insert(
            SourceLimit::BinanceSpot,
            RateBucket::new(6000, Duration::from_secs(60)),
        );
        buckets.insert(
            SourceLimit::BinancePerp,
            RateBucket::new(2400, Duration::from_secs(60)),
        );
        buckets.insert(
            SourceLimit::Bybit,
            RateBucket::new(600, Duration::from_secs(5)),
        );

        Self { buckets }
    }

    pub async fn acquire(&mut self, source: SourceLimit, weight: usize) {
        if let Some(bucket) = self.buckets.get_mut(&source) {
            dbg!(source, weight, &bucket);
            bucket.acquire(weight).await;
        }
    }

    pub fn update_limit(&mut self, source: SourceLimit, max_tokens: usize) {
        if let Some(bucket) = self.buckets.get_mut(&source) {
            bucket.max_tokens = max_tokens;
        } else {
            self.buckets
                .insert(source, RateBucket::new(max_tokens, Duration::from_secs(60)));
        }
    }
}
