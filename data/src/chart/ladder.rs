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
}

impl ChaseTracker {
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
