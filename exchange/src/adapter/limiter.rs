use reqwest::Response;
use std::time::{Duration, Instant};

pub trait RateLimiter: Send + Sync {
    /// Prepare for a request with given weight. Returns wait time if needed
    fn prepare_request(&mut self, weight: usize) -> Option<Duration>;

    /// Update the limiter with response data (e.g., rate limit headers)
    fn update_from_response(&mut self, response: &Response, weight: usize);

    /// Check if response indicates rate limiting and should exit
    fn should_exit_on_response(&self, response: &Response) -> bool;
}

/// Limiter for a fixed window rate
pub struct FixedWindowBucket {
    max_tokens: usize,
    available_tokens: usize,
    last_refill: Instant,
    refill_rate: Duration,
}

impl FixedWindowBucket {
    pub fn new(max_tokens: usize, refill_rate: Duration) -> Self {
        Self {
            max_tokens,
            available_tokens: max_tokens,
            last_refill: Instant::now(),
            refill_rate,
        }
    }

    fn refill(&mut self) {
        if let Ok(current_time) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        {
            let now = Instant::now();
            let period_seconds = self.refill_rate.as_secs();
            let seconds_in_current_period = current_time.as_secs() % period_seconds;

            let elapsed = now.duration_since(self.last_refill);
            if elapsed >= self.refill_rate || seconds_in_current_period < 1 {
                self.available_tokens = self.max_tokens;
                self.last_refill = now;
            }
        }
    }

    pub fn calculate_wait_time(&mut self, tokens: usize) -> Option<Duration> {
        self.refill();

        if self.available_tokens >= tokens {
            self.available_tokens -= tokens;
            return None;
        }

        let wait_time = self
            .refill_rate
            .saturating_sub(Instant::now().duration_since(self.last_refill));
        Some(wait_time)
    }

    pub fn consume_tokens(&mut self, tokens: usize) {
        self.refill();
        self.available_tokens -= tokens.min(self.available_tokens);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DynamicLimitReason {
    HeaderRate,
    FixedWindowRate,
}

/// Limiter that can be used when source reports the rate-limit usage
///
/// Can fallback to fixed window bucket
pub struct DynamicBucket {
    max_weight: usize,
    current_used_weight: usize,
    last_updated: Instant,
    refill_rate: Duration,
    fallback_bucket: FixedWindowBucket,
}

impl DynamicBucket {
    pub fn new(max_weight: usize, refill_rate: Duration) -> Self {
        Self {
            max_weight,
            current_used_weight: 0,
            last_updated: Instant::now(),
            refill_rate,
            fallback_bucket: FixedWindowBucket::new(max_weight, refill_rate),
        }
    }

    pub fn update_weight(&mut self, new_weight: usize) {
        if new_weight > 0 {
            self.current_used_weight = new_weight;
            self.last_updated = Instant::now();
        }
    }

    pub fn prepare_request(
        &mut self,
        weight: usize,
    ) -> (Option<Duration>, Option<DynamicLimitReason>) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_updated);

        if elapsed <= self.refill_rate && self.current_used_weight > 0 {
            self.prepare_with_header_data(weight)
        } else {
            self.prepare_with_fallback(weight)
        }
    }

    fn prepare_with_header_data(
        &self,
        weight: usize,
    ) -> (Option<Duration>, Option<DynamicLimitReason>) {
        let available = self.max_weight.saturating_sub(self.current_used_weight);

        if available >= weight {
            return (None, None);
        }

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        let period_seconds = self.refill_rate.as_secs();
        let seconds_in_period = current_time.as_secs() % period_seconds;
        let wait_time = Duration::from_secs(period_seconds - seconds_in_period)
            .saturating_add(Duration::from_millis(500));

        (Some(wait_time), Some(DynamicLimitReason::HeaderRate))
    }

    fn prepare_with_fallback(
        &mut self,
        weight: usize,
    ) -> (Option<Duration>, Option<DynamicLimitReason>) {
        match self.fallback_bucket.calculate_wait_time(weight) {
            None => (None, None),
            Some(wait_time) => (Some(wait_time), Some(DynamicLimitReason::FixedWindowRate)),
        }
    }
}
