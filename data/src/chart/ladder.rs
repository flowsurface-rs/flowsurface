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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Default)]
enum ChaseProgress {
    #[default]
    Idle,
    Chasing {
        direction: Direction,
        start: Price,
        end: Price,
        consecutive: u32,
    },
    Fading {
        start: Price,
        end: Price,
        strength: u32,
    },
}

#[derive(Debug, Default)]
pub struct ChaseTracker {
    /// Last known best price (raw ungrouped)
    last_best: Option<Price>,
    state: ChaseProgress,
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

            let step_gain = Self::MAX_CONSECUTIVE / 10;

            self.state =
                match (&self.state, is_continue, is_reverse, is_unchanged) {
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
                        consecutive: consecutive
                            .saturating_add(step_gain)
                            .min(Self::MAX_CONSECUTIVE),
                    },

                    // Start a new chase (from idle or while fading)
                    (ChaseProgress::Idle, true, _, _)
                    | (ChaseProgress::Fading { .. }, true, _, _) => ChaseProgress::Chasing {
                        direction,
                        start: last,
                        end: current,
                        consecutive: step_gain.min(Self::MAX_CONSECUTIVE),
                    },

                    // Reversal while chasing -> fade previous trail
                    (
                        ChaseProgress::Chasing {
                            start, consecutive, ..
                        },
                        _,
                        true,
                        _,
                    ) if *consecutive > 0 => ChaseProgress::Fading {
                        start: *start,
                        end: current,
                        strength: *consecutive,
                    },

                    // Unchanged while chasing -> decay
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
                    ) if *sdir == direction => {
                        let next = consecutive.saturating_sub(Self::COOLDOWN_DECAY_RATE);
                        if next == 0 {
                            ChaseProgress::Idle
                        } else {
                            ChaseProgress::Chasing {
                                direction,
                                start: *start,
                                end: *end,
                                consecutive: next,
                            }
                        }
                    }

                    // Unchanged while fading -> fade out
                    (
                        ChaseProgress::Fading {
                            start,
                            end,
                            strength,
                        },
                        _,
                        _,
                        true,
                    ) => {
                        let next = strength.saturating_sub(Self::COOLDOWN_DECAY_RATE);
                        if next == 0 {
                            ChaseProgress::Idle
                        } else {
                            ChaseProgress::Fading {
                                start: *start,
                                end: *end,
                                strength: next,
                            }
                        }
                    }

                    // Reversal when idle or already fading -> no change
                    (ChaseProgress::Idle, _, true, _)
                    | (ChaseProgress::Fading { .. }, _, true, _) => self.state,

                    // Unchanged when idle -> no change
                    (ChaseProgress::Idle, _, _, true) => ChaseProgress::Idle,

                    // Any other "continue" case -> (re)start a chase in this dir
                    (_, true, _, _) => ChaseProgress::Chasing {
                        direction,
                        start: last,
                        end: current,
                        consecutive: step_gain.min(Self::MAX_CONSECUTIVE),
                    },

                    // Default: keep state
                    _ => self.state,
                };
        }

        self.last_best = Some(current);
    }

    fn reset(&mut self) {
        self.last_best = None;
        self.state = ChaseProgress::Idle;
    }

    pub fn opacity(&self) -> f32 {
        match self.state {
            ChaseProgress::Chasing { consecutive, .. } => {
                (consecutive as f32 / Self::MAX_CONSECUTIVE as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    pub fn fading_opacity(&self) -> f32 {
        match self.state {
            ChaseProgress::Fading { strength, .. } => {
                (strength as f32 / Self::MAX_CONSECUTIVE as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    pub fn is_visible(&self) -> bool {
        matches!(self.state, ChaseProgress::Chasing { .. }) && self.opacity() > 0.0
    }

    pub fn is_fading_visible(&self) -> bool {
        matches!(self.state, ChaseProgress::Fading { .. })
            && self.fading_opacity() > 0.0
            && !self.is_visible()
    }

    pub fn chase_start_price(&self) -> Option<Price> {
        match self.state {
            ChaseProgress::Chasing { start, .. } => Some(start),
            _ => None,
        }
    }

    pub fn chase_end_price(&self) -> Option<Price> {
        match self.state {
            ChaseProgress::Chasing { end, .. } => Some(end),
            _ => None,
        }
    }
}
