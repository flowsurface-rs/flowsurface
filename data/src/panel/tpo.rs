use exchange::unit::{Price, PriceStep};
use exchange::{Kline, TickerInfo};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const LETTERS: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R',
    'S', 'T', 'U', 'V', 'W', 'X',
];

const VALUE_AREA_PCT: f64 = 0.70;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct Config {
    pub period_ms: u64,
    pub session_ms: u64,
    pub show_value_area: bool,
    pub show_poc: bool,
    pub show_ib: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            period_ms: 1_800_000,
            session_ms: 86_400_000,
            show_value_area: true,
            show_poc: true,
            show_ib: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TpoPeriod {
    pub start_ms: u64,
    pub letter_idx: usize,
    pub prices: Vec<i64>,
}

impl TpoPeriod {
    pub fn letter(&self) -> char {
        LETTERS[self.letter_idx % LETTERS.len()]
    }
}

#[derive(Debug, Clone, Default)]
pub struct TpoProfile {
    pub session_start_ms: u64,
    pub periods: Vec<TpoPeriod>,
    pub price_counts: HashMap<i64, usize>,
    pub poc: Option<i64>,
    pub va_high: Option<i64>,
    pub va_low: Option<i64>,
    pub ib_high: Option<i64>,
    pub ib_low: Option<i64>,
    pub price_min: Option<i64>,
    pub price_max: Option<i64>,
}

impl TpoProfile {
    pub fn compute_analytics(&mut self, ib_periods: usize) {
        if self.price_counts.is_empty() {
            return;
        }

        let poc = self
            .price_counts
            .iter()
            .max_by_key(|(_, &cnt)| cnt)
            .map(|(&p, _)| p);
        self.poc = poc;

        let mut sorted: Vec<i64> = self.price_counts.keys().copied().collect();
        sorted.sort_unstable();
        self.price_min = sorted.first().copied();
        self.price_max = sorted.last().copied();

        if let Some(poc_price) = poc {
            let total: usize = self.price_counts.values().sum();
            let va_target = ((total as f64 * VALUE_AREA_PCT).ceil() as usize).min(total);
            let poc_idx = sorted.partition_point(|&p| p < poc_price);
            let mut accumulated = self.price_counts.get(&poc_price).copied().unwrap_or(0);
            let mut lo = poc_idx;
            let mut hi = poc_idx;

            while accumulated < va_target {
                let above = if hi + 1 < sorted.len() {
                    self.price_counts.get(&sorted[hi + 1]).copied().unwrap_or(0)
                } else {
                    0
                };
                let below = if lo > 0 {
                    self.price_counts.get(&sorted[lo - 1]).copied().unwrap_or(0)
                } else {
                    0
                };
                if above == 0 && below == 0 {
                    break;
                }
                if above >= below && hi + 1 < sorted.len() {
                    hi += 1;
                    accumulated += above;
                } else if lo > 0 {
                    lo -= 1;
                    accumulated += below;
                } else if hi + 1 < sorted.len() {
                    hi += 1;
                    accumulated += above;
                } else {
                    break;
                }
            }

            self.va_low = sorted.get(lo).copied();
            self.va_high = sorted.get(hi).copied();
        }

        let ib_count = ib_periods.min(self.periods.len());
        if ib_count > 0 {
            let mut ib_prices: Vec<i64> = self.periods[..ib_count]
                .iter()
                .flat_map(|p| p.prices.iter().copied())
                .collect();
            ib_prices.sort_unstable();
            self.ib_low = ib_prices.first().copied();
            self.ib_high = ib_prices.last().copied();
        }
    }
}

pub struct TpoData {
    pub config: Config,
    pub profiles: BTreeMap<u64, TpoProfile>,
    current_period_prices: HashSet<i64>,
    current_period_start: Option<u64>,
    current_session_start: Option<u64>,
    tick_units: i64,
}

impl TpoData {
    pub fn new(ticker_info: TickerInfo, config: Option<Config>) -> Self {
        let config = config.unwrap_or_default();
        let step: PriceStep = ticker_info.min_ticksize.into();
        let tick_units = step.units.max(1);
        Self {
            config,
            profiles: BTreeMap::new(),
            current_period_prices: HashSet::new(),
            current_period_start: None,
            current_session_start: None,
            tick_units,
        }
    }

    pub fn add_kline(&mut self, kline: &Kline) {
        let time_ms = kline.time;
        let period_ms = self.config.period_ms;
        let session_ms = self.config.session_ms;
        let session_start = (time_ms / session_ms) * session_ms;
        let period_start = (time_ms / period_ms) * period_ms;

        let new_session = self.current_session_start != Some(session_start);
        if new_session {
            self.flush_current_period();
            if let Some(s) = self.current_session_start {
                if let Some(profile) = self.profiles.get_mut(&s) {
                    profile.compute_analytics(2);
                }
            }
            self.current_session_start = Some(session_start);
            self.current_period_start = Some(period_start);
            self.current_period_prices.clear();
            self.profiles.entry(session_start).or_insert_with(|| TpoProfile {
                session_start_ms: session_start,
                ..Default::default()
            });
        } else {
            let new_period = self.current_period_start != Some(period_start);
            if new_period {
                self.flush_current_period();
                self.current_period_start = Some(period_start);
                self.current_period_prices.clear();
            }
        }

        let low_tick = kline.low.units / self.tick_units;
        let high_tick = kline.high.units / self.tick_units;
        for tick in low_tick..=high_tick {
            self.current_period_prices.insert(tick * self.tick_units);
        }
    }

    pub fn flush_current_period(&mut self) {
        let Some(period_start) = self.current_period_start else {
            return;
        };
        let Some(session_start) = self.current_session_start else {
            return;
        };
        if self.current_period_prices.is_empty() {
            return;
        }
        let profile = self.profiles.entry(session_start).or_insert_with(|| TpoProfile {
            session_start_ms: session_start,
            ..Default::default()
        });
        if profile.periods.last().map(|p| p.start_ms) == Some(period_start) {
            return;
        }
        let letter_idx = profile.periods.len();
        let mut prices: Vec<i64> = self.current_period_prices.iter().copied().collect();
        prices.sort_unstable();
        for &p in &prices {
            *profile.price_counts.entry(p).or_insert(0) += 1;
        }
        profile.periods.push(TpoPeriod {
            start_ms: period_start,
            letter_idx,
            prices,
        });
    }

    pub fn load_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            self.add_kline(kline);
        }
        self.flush_current_period();
        if let Some(s) = self.current_session_start {
            if let Some(profile) = self.profiles.get_mut(&s) {
                profile.compute_analytics(2);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn sorted_profiles(&self) -> Vec<&TpoProfile> {
        self.profiles.values().collect()
    }
}
