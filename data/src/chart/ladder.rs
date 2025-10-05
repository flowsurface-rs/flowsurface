use exchange::util::Price;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const TRADE_RETENTION_MS: u64 = 8 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub show_spread: bool,
    pub trade_retention: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_spread: false,
            trade_retention: Duration::from_millis(TRADE_RETENTION_MS),
        }
    }
}

#[derive(Default, Debug)]
pub struct ChaseTracker {
    /// Consecutive updates moving in same direction (capped at `MAX_CONSECUTIVE`)
    consecutive_moves: u32,
    /// Last known best price (raw ungrouped)
    last_best: Option<Price>,
    /// Price where the chase started (first move in this direction)
    pub chase_start_price: Option<Price>,
    /// Price where the chase ended (when reversal began)
    pub chase_end_price: Option<Price>,
    /// Tracks if we're in cooldown mode (price unchanged but no reversal)
    in_cooldown: bool,
    /// When a reversal happens fade out the previous trail
    fading_out: bool,
    /// Strength of the fading trail
    fading_strength: u32,
}

impl ChaseTracker {
    const MAX_CONSECUTIVE: u32 = 50;
    const COOLDOWN_DECAY_RATE: u32 = 1;

    pub fn update(&mut self, current_best: Option<Price>, is_bid: bool) {
        let Some(current) = current_best else {
            self.reset();
            return;
        };

        if let Some(last) = self.last_best {
            let is_chasing = if is_bid {
                current > last
            } else {
                current < last
            };
            let is_reversing = if is_bid {
                current < last
            } else {
                current > last
            };
            let is_unchanged = current == last;

            if is_chasing {
                self.consecutive_moves = self
                    .consecutive_moves
                    .saturating_add(10)
                    .min(Self::MAX_CONSECUTIVE);
                self.in_cooldown = false;

                if self.chase_start_price.is_none() {
                    self.chase_start_price = Some(last);
                }

                self.chase_end_price = Some(current);

                self.fading_out = false;
                self.fading_strength = 0;
            } else if is_reversing {
                if self.chase_start_price.is_some() && self.consecutive_moves > 0 {
                    self.fading_out = true;
                    self.fading_strength = self.consecutive_moves;
                }

                self.consecutive_moves = 0;
                self.chase_start_price = None;
                self.chase_end_price = None;
                self.in_cooldown = false;
            } else if is_unchanged {
                self.in_cooldown = true;
                self.consecutive_moves = self
                    .consecutive_moves
                    .saturating_sub(Self::COOLDOWN_DECAY_RATE);

                if self.consecutive_moves == 0 {
                    self.chase_start_price = None;
                }

                if self.fading_out {
                    self.fading_strength = self
                        .fading_strength
                        .saturating_sub(Self::COOLDOWN_DECAY_RATE);
                    if self.fading_strength == 0 {
                        self.fading_out = false;
                    }
                }
            }
        }

        self.last_best = Some(current);
    }

    pub fn reset(&mut self) {
        self.consecutive_moves = 0;
        self.in_cooldown = false;
        self.fading_out = false;
        self.fading_strength = 0;
        self.chase_start_price = None;
        self.chase_end_price = None;
    }

    pub fn opacity(&self) -> f32 {
        (self.consecutive_moves as f32 / Self::MAX_CONSECUTIVE as f32).clamp(0.0, 1.0)
    }

    pub fn fading_opacity(&self) -> f32 {
        (self.fading_strength as f32 / Self::MAX_CONSECUTIVE as f32).clamp(0.0, 1.0)
    }

    pub fn is_active(&self) -> bool {
        self.consecutive_moves >= 2
    }

    pub fn is_visible(&self) -> bool {
        self.chase_start_price.is_some() && self.opacity() > 0.0
    }

    pub fn is_fading_visible(&self) -> bool {
        self.fading_out && self.fading_opacity() > 0.0 && !self.is_visible()
    }
}
