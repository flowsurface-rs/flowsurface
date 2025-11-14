use crate::chart::kline::KlineTrades;
use crate::util::ok_or_default;
use exchange::{
    Trade,
    util::{Price, PriceStep},
};

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

const TRADE_RETENTION_MS: u64 = 8 * 60_000;
const CHASE_MIN_VISIBLE_OPACITY: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub show_spread: bool,
    #[serde(deserialize_with = "ok_or_default", default)]
    pub show_chase_tracker: bool,
    pub trade_retention: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_spread: false,
            show_chase_tracker: true,
            trade_retention: Duration::from_millis(TRADE_RETENTION_MS),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

#[derive(Copy, Clone)]
pub enum Side {
    Bid,
    Ask,
}

impl Side {
    pub fn idx(self) -> usize {
        match self {
            Side::Bid => 0,
            Side::Ask => 1,
        }
    }

    pub fn is_bid(self) -> bool {
        matches!(self, Side::Bid)
    }
}

#[derive(Default)]
pub struct GroupedDepth {
    pub orders: BTreeMap<Price, f32>,
    pub chase: ChaseTracker,
}

impl GroupedDepth {
    pub fn new() -> Self {
        Self {
            orders: BTreeMap::new(),
            chase: ChaseTracker::default(),
        }
    }

    pub fn regroup_from_raw(&mut self, levels: &BTreeMap<Price, f32>, side: Side, step: PriceStep) {
        self.orders.clear();
        for (price, qty) in levels.iter() {
            let grouped_price = price.round_to_side_step(side.is_bid(), step);
            *self.orders.entry(grouped_price).or_insert(0.0) += *qty;
        }
    }

    pub fn best_price(&self, side: Side) -> Option<Price> {
        match side {
            Side::Bid => self.orders.last_key_value().map(|(p, _)| *p),
            Side::Ask => self.orders.first_key_value().map(|(p, _)| *p),
        }
    }
}

#[derive(Debug)]
pub struct TradeStore {
    pub raw: VecDeque<Trade>,
    pub grouped: KlineTrades,
}

impl Default for TradeStore {
    fn default() -> Self {
        Self {
            raw: VecDeque::new(),
            grouped: KlineTrades::new(),
        }
    }
}

impl TradeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn insert_trades(&mut self, buffer: &[Trade], step: PriceStep) {
        for trade in buffer {
            self.grouped.add_trade_to_side_bin(trade, step);
            self.raw.push_back(*trade);
        }
    }

    pub fn rebuild_grouped(&mut self, step: PriceStep) {
        self.grouped.clear();
        for trade in &self.raw {
            self.grouped.add_trade_to_side_bin(trade, step);
        }
    }

    pub fn trade_qty_at(&self, price: Price) -> (f32, f32) {
        if let Some(g) = self.grouped.trades.get(&price) {
            (g.buy_qty, g.sell_qty)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn price_range(&self) -> Option<(Price, Price)> {
        let mut min_p: Option<Price> = None;
        let mut max_p: Option<Price> = None;
        for &p in self.grouped.trades.keys() {
            min_p = Some(min_p.map_or(p, |cur| cur.min(p)));
            max_p = Some(max_p.map_or(p, |cur| cur.max(p)));
        }
        match (min_p, max_p) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        }
    }

    /// Returns true if it removed trades and regrouped.
    pub fn maybe_cleanup(&mut self, now_ms: u64, retention: Duration, step: PriceStep) -> bool {
        let Some(oldest) = self.raw.front() else {
            return false;
        };

        let retention_ms = retention.as_millis() as u64;
        if retention_ms == 0 {
            return false;
        }

        // ~1/10th of retention, min 5s
        let cleanup_step_ms = (retention_ms / 10).max(5_000);
        let threshold_ms = retention_ms + cleanup_step_ms;
        if now_ms.saturating_sub(oldest.time) < threshold_ms {
            return false;
        }

        let keep_from_ms = now_ms.saturating_sub(retention_ms);
        let mut removed = 0usize;
        while let Some(trade) = self.raw.front() {
            if trade.time < keep_from_ms {
                self.raw.pop_front();
                removed += 1;
            } else {
                break;
            }
        }

        if removed > 0 {
            self.rebuild_grouped(step);
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, Copy, Default)]
enum ChaseProgress {
    #[default]
    Idle,
    Chasing {
        direction: Direction,
        start: Price,
        end: Price,
        /// Number of consecutive moves in the current direction
        consecutive: u32,
    },
    Fading {
        direction: Direction,
        start: Price,
        end: Price,
        /// Consecutive count at the moment fading started
        start_consecutive: u32,
        /// How many unchanged updates we have been fading
        fade_steps: u32,
    },
}

#[derive(Debug, Default)]
pub struct ChaseTracker {
    /// Last known best price (raw ungrouped)
    last_best: Option<Price>,
    state: ChaseProgress,
    last_update_ms: Option<u64>,
}

impl ChaseTracker {
    pub fn update(
        &mut self,
        current_best: Option<Price>,
        is_bid: bool,
        now_ms: u64,
        max_interval: Duration,
    ) {
        let max_ms = max_interval.as_millis() as u64;
        if let Some(prev) = self.last_update_ms
            && max_ms > 0
            && now_ms.saturating_sub(prev) > max_ms
        {
            self.reset();
        }

        self.last_update_ms = Some(now_ms);

        let Some(current) = current_best else {
            self.reset();
            return;
        };

        if let Some(last) = self.last_best {
            let direction = if is_bid {
                Direction::Up
            } else {
                Direction::Down
            };

            let is_continue = match direction {
                Direction::Up => current > last,
                Direction::Down => current < last,
            };
            let is_reverse = match direction {
                Direction::Up => current < last,
                Direction::Down => current > last,
            };
            let is_unchanged = current == last;

            self.state = match (&self.state, is_continue, is_reverse, is_unchanged) {
                // Continue in same direction while already chasing: extend chase
                (
                    ChaseProgress::Chasing {
                        direction: sdir,
                        start,
                        consecutive,
                        ..
                    },
                    true,
                    _,
                    _,
                ) if *sdir == direction => ChaseProgress::Chasing {
                    direction,
                    start: *start,
                    end: current,
                    consecutive: consecutive.saturating_add(1),
                },
                // Start or restart a chase (from idle or from fading)
                (ChaseProgress::Idle, true, _, _) | (ChaseProgress::Fading { .. }, true, _, _) => {
                    ChaseProgress::Chasing {
                        direction,
                        start: last,
                        end: current,
                        consecutive: 1,
                    }
                }
                // Reversal while chasing -> start fading from the last chase extreme (freeze end)
                (
                    ChaseProgress::Chasing {
                        direction: sdir,
                        start,
                        end,
                        consecutive,
                    },
                    _,
                    true,
                    _,
                ) if *consecutive > 0 => ChaseProgress::Fading {
                    direction: *sdir,
                    start: *start,
                    end: *end, // keep the extreme reached during the chase
                    start_consecutive: *consecutive,
                    fade_steps: 0,
                },
                // Unchanged while chasing -> start fading from the last chase extreme (freeze end)
                (
                    ChaseProgress::Chasing {
                        direction: sdir,
                        start,
                        end,
                        consecutive,
                    },
                    _,
                    _,
                    true,
                ) if *consecutive > 0 => ChaseProgress::Fading {
                    direction: *sdir,
                    start: *start,
                    end: *end, // keep the extreme reached during the chase
                    start_consecutive: *consecutive,
                    fade_steps: 0,
                },
                // Unchanged while fading -> keep fading (decay)
                (
                    ChaseProgress::Fading {
                        direction: sdir,
                        start,
                        end,
                        start_consecutive,
                        fade_steps,
                    },
                    _,
                    _,
                    true,
                ) => ChaseProgress::Fading {
                    direction: *sdir,
                    start: *start,
                    end: *end,
                    start_consecutive: *start_consecutive,
                    fade_steps: fade_steps.saturating_add(1),
                },
                // Reversal while fading -> keep fading and decay
                (
                    ChaseProgress::Fading {
                        direction: sdir,
                        start,
                        end,
                        start_consecutive,
                        fade_steps,
                    },
                    _,
                    true,
                    _,
                ) => ChaseProgress::Fading {
                    direction: *sdir,
                    start: *start,
                    end: *end, // freeze
                    start_consecutive: *start_consecutive,
                    fade_steps: fade_steps.saturating_add(1),
                },
                // Unchanged when idle -> no change
                (ChaseProgress::Idle, _, _, true) => ChaseProgress::Idle,
                _ => self.state,
            };

            if let ChaseProgress::Fading {
                start_consecutive,
                fade_steps,
                ..
            } = self.state
            {
                let base = Self::consecutive_to_alpha(start_consecutive);
                let alpha = base / (1.0 + fade_steps as f32);
                if alpha < CHASE_MIN_VISIBLE_OPACITY {
                    self.state = ChaseProgress::Idle;
                }
            }
        }

        self.last_best = Some(current);
    }

    pub fn reset(&mut self) {
        self.last_best = None;
        self.state = ChaseProgress::Idle;
        self.last_update_ms = None;
    }

    /// Maps consecutive steps n to [0,1): 1 - 1/(1+n)
    fn consecutive_to_alpha(n: u32) -> f32 {
        let nf = n as f32;
        1.0 - 1.0 / (1.0 + nf)
    }

    pub fn segment(&self) -> Option<(Price, Price, f32)> {
        match self.state {
            ChaseProgress::Chasing {
                start,
                end,
                consecutive,
                ..
            } => Some((start, end, Self::consecutive_to_alpha(consecutive))),
            ChaseProgress::Fading {
                start,
                end,
                start_consecutive,
                fade_steps,
                ..
            } => {
                let alpha = {
                    let base = Self::consecutive_to_alpha(start_consecutive);
                    base / (1.0 + fade_steps as f32)
                };
                Some((start, end, alpha))
            }
            _ => None,
        }
    }
}
