pub mod big_trades;
pub mod tpo;
pub mod volume_profile;

pub use big_trades::{
    BigTradesConfig, DEFAULT_BIG_TRADES_THRESHOLD, MAX_BIG_TRADES_THRESHOLD,
    MIN_BIG_TRADES_THRESHOLD, big_trades_step_count, big_trades_step_for_threshold,
    big_trades_threshold_at_step, big_trades_threshold_label,
};
pub use tpo::{
    DEFAULT_TPO_LETTER_PERIOD, DEFAULT_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT,
    MIN_TPO_VALUE_AREA_PERCENT, TpoColorConfig,
};
pub use volume_profile::{
    DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL, DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
    DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT, DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
    MAX_VOLUME_PROFILE_OPACITY_PERCENT, MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER, MIN_VOLUME_PROFILE_OPACITY_PERCENT,
    MIN_VOLUME_PROFILE_WIDTH_PERCENT, VolumeProfileColorConfig, VolumeProfileColorSlot,
    VolumeProfileColorSpec, VolumeProfileSchedule,
};

use self::big_trades::{
    default_big_trades_buy_color, default_big_trades_sell_color, default_big_trades_threshold,
};
use self::tpo::{
    TpoAnchor, default_tpo_anchor, default_tpo_letter_period, default_tpo_value_area_percent,
    default_tpo_visible,
};
use self::volume_profile::default_volume_profile_width_percent;

use crate::chart::Basis;
use crate::config::theme::ThemeColor;
use exchange::Timeframe;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

const MAX_PROFILE_PRICE_ROWS: usize = 4_096;

const fn default_study_visible() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StudyDataSource {
    Kline,
    Trades,
}

impl StudyDataSource {
    pub const fn requires_trade_data(self) -> bool {
        matches!(self, Self::Trades)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceConstraint {
    Exclusive(StudyDataSource),
    Independent,
}

impl Display for StudyDataSource {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Kline => "Kline",
            Self::Trades => "Trades",
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
enum PersistedKlineStudy {
    VolumeProfile {
        #[serde(default)]
        source: Option<StudyDataSource>,
        schedule: VolumeProfileSchedule,
        #[serde(default)]
        colors: VolumeProfileColorConfig,
        #[serde(default = "default_volume_profile_width_percent")]
        width_percent: u8,
        opacity_percent: Option<u8>,
        #[serde(default = "default_study_visible")]
        visible: bool,
    },
    TpoProfile {
        #[serde(default)]
        source: Option<StudyDataSource>,
        #[serde(default = "default_tpo_anchor")]
        anchor: TpoAnchor,
        #[serde(default = "default_tpo_letter_period")]
        letter_period: Timeframe,
        #[serde(default = "default_tpo_value_area_percent")]
        value_area_percent: u8,
        #[serde(default)]
        colors: TpoColorConfig,
        #[serde(default = "default_tpo_visible")]
        visible: bool,
    },
    BigTrades {
        #[serde(default = "default_big_trades_threshold")]
        threshold: u64,
        #[serde(default = "default_big_trades_buy_color")]
        buy_color: ThemeColor,
        #[serde(default = "default_big_trades_sell_color")]
        sell_color: ThemeColor,
        #[serde(default = "default_study_visible")]
        visible: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KlineStudy {
    VolumeProfile {
        source: StudyDataSource,
        schedule: VolumeProfileSchedule,
        #[serde(default)]
        colors: VolumeProfileColorConfig,
        #[serde(default = "default_volume_profile_width_percent")]
        width_percent: u8,
        opacity_percent: u8,
        #[serde(default = "default_study_visible")]
        visible: bool,
    },
    TpoProfile {
        source: StudyDataSource,
        #[serde(default = "default_tpo_anchor")]
        anchor: TpoAnchor,
        #[serde(default = "default_tpo_letter_period")]
        letter_period: Timeframe,
        #[serde(default = "default_tpo_value_area_percent")]
        value_area_percent: u8,
        #[serde(default)]
        colors: TpoColorConfig,
        #[serde(default = "default_tpo_visible")]
        visible: bool,
    },
    BigTrades {
        #[serde(default = "default_big_trades_threshold")]
        threshold: u64,
        #[serde(default = "default_big_trades_buy_color")]
        buy_color: ThemeColor,
        #[serde(default = "default_big_trades_sell_color")]
        sell_color: ThemeColor,
        #[serde(default = "default_study_visible")]
        visible: bool,
    },
}

impl<'de> Deserialize<'de> for KlineStudy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match PersistedKlineStudy::deserialize(deserializer)? {
            PersistedKlineStudy::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
            } => Ok(Self::VolumeProfile {
                source: source.unwrap_or(StudyDataSource::Trades),
                schedule,
                colors,
                width_percent,
                opacity_percent: opacity_percent
                    .unwrap_or_else(|| schedule.default_opacity_percent()),
                visible,
            }),
            PersistedKlineStudy::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent,
                colors,
                visible,
            } => Ok(Self::TpoProfile {
                source: source.unwrap_or(StudyDataSource::Kline),
                anchor,
                letter_period,
                value_area_percent: value_area_percent
                    .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
                colors,
                visible,
            }),
            PersistedKlineStudy::BigTrades {
                threshold,
                buy_color,
                sell_color,
                visible,
            } => Ok(Self::BigTrades {
                threshold: threshold.clamp(MIN_BIG_TRADES_THRESHOLD, MAX_BIG_TRADES_THRESHOLD),
                buy_color,
                sell_color,
                visible,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlineStudyId {
    VolumeProfileVisibleRange(StudyDataSource),
    VolumeProfileRecurring(StudyDataSource),
    TpoProfile(StudyDataSource),
    BigTrades,
}

impl KlineStudy {
    pub const ALL: [Self; 7] = [
        Self::VolumeProfile {
            source: StudyDataSource::Kline,
            schedule: VolumeProfileSchedule::VisibleRange,
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
        },
        Self::TpoProfile {
            source: StudyDataSource::Kline,
            anchor: TpoAnchor::Daily,
            letter_period: DEFAULT_TPO_LETTER_PERIOD,
            value_area_percent: DEFAULT_TPO_VALUE_AREA_PERCENT,
            colors: TpoColorConfig::defaults(),
            visible: true,
        },
        Self::VolumeProfile {
            source: StudyDataSource::Kline,
            schedule: VolumeProfileSchedule::Recurring {
                interval: DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL,
            },
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
        },
        Self::VolumeProfile {
            source: StudyDataSource::Trades,
            schedule: VolumeProfileSchedule::VisibleRange,
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
        },
        Self::TpoProfile {
            source: StudyDataSource::Trades,
            anchor: TpoAnchor::Daily,
            letter_period: DEFAULT_TPO_LETTER_PERIOD,
            value_area_percent: DEFAULT_TPO_VALUE_AREA_PERCENT,
            colors: TpoColorConfig::defaults(),
            visible: true,
        },
        Self::VolumeProfile {
            source: StudyDataSource::Trades,
            schedule: VolumeProfileSchedule::Recurring {
                interval: DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL,
            },
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
        },
        Self::BigTrades {
            threshold: DEFAULT_BIG_TRADES_THRESHOLD,
            buy_color: default_big_trades_buy_color(),
            sell_color: default_big_trades_sell_color(),
            visible: true,
        },
    ];

    pub fn defaults() -> Vec<Self> {
        Self::ALL.to_vec()
    }

    pub fn default_for_type(self) -> Self {
        Self::ALL
            .into_iter()
            .find(|default| default.is_same_type(self))
            .unwrap_or(self)
    }

    pub fn default_for_type_for_basis(self, basis: Basis) -> Self {
        self.default_for_type()
            .normalized_for_basis(basis)
            .unwrap_or_else(|| self.default_for_type())
            .with_visibility(self.visible())
    }

    pub const fn visible(self) -> bool {
        match self {
            Self::VolumeProfile { visible, .. }
            | Self::TpoProfile { visible, .. }
            | Self::BigTrades { visible, .. } => visible,
        }
    }

    pub const fn with_visibility(self, visible: bool) -> Self {
        match self {
            Self::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                ..
            } => Self::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
            },
            Self::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent,
                colors,
                ..
            } => Self::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent,
                colors,
                visible,
            },
            Self::BigTrades {
                threshold,
                buy_color,
                sell_color,
                ..
            } => Self::BigTrades {
                threshold,
                buy_color,
                sell_color,
                visible,
            },
        }
    }

    pub const fn id(self) -> KlineStudyId {
        match self {
            Self::VolumeProfile {
                source,
                schedule: VolumeProfileSchedule::VisibleRange,
                ..
            } => KlineStudyId::VolumeProfileVisibleRange(source),
            Self::VolumeProfile {
                source,
                schedule: VolumeProfileSchedule::Recurring { .. },
                ..
            } => KlineStudyId::VolumeProfileRecurring(source),
            Self::TpoProfile { source, .. } => KlineStudyId::TpoProfile(source),
            Self::BigTrades { .. } => KlineStudyId::BigTrades,
        }
    }

    pub fn is_same_type(self, other: Self) -> bool {
        self.id() == other.id()
    }

    pub fn requires_reconfiguration(self, previous: Self) -> bool {
        match (self, previous) {
            (
                Self::VolumeProfile {
                    source,
                    schedule,
                    colors: _,
                    width_percent: _,
                    opacity_percent: _,
                    visible: _,
                },
                Self::VolumeProfile {
                    source: previous_source,
                    schedule: previous_schedule,
                    colors: _,
                    width_percent: _,
                    opacity_percent: _,
                    visible: _,
                },
            ) => (source, schedule) != (previous_source, previous_schedule),
            (
                Self::TpoProfile {
                    source,
                    anchor,
                    letter_period,
                    value_area_percent,
                    colors: _,
                    visible: _,
                },
                Self::TpoProfile {
                    source: previous_source,
                    anchor: previous_anchor,
                    letter_period: previous_letter_period,
                    value_area_percent: previous_value_area_percent,
                    colors: _,
                    visible: _,
                },
            ) => {
                (source, anchor, letter_period, value_area_percent)
                    != (
                        previous_source,
                        previous_anchor,
                        previous_letter_period,
                        previous_value_area_percent,
                    )
            }
            (
                Self::BigTrades {
                    threshold,
                    buy_color: _,
                    sell_color: _,
                    visible,
                },
                Self::BigTrades {
                    threshold: previous_threshold,
                    buy_color: _,
                    sell_color: _,
                    visible: previous_visible,
                },
            ) => (threshold, visible) != (previous_threshold, previous_visible),
            _ => true,
        }
    }

    pub fn is_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_available(basis),
            Self::TpoProfile { .. } => matches!(basis, Basis::Time(_)),
            Self::BigTrades { .. } => true,
        }
    }

    pub fn is_interval_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_interval_available(basis),
            Self::TpoProfile { .. } | Self::BigTrades { .. } => self.is_available(basis),
        }
    }

    pub const fn requires_trade_data(self) -> bool {
        self.source().requires_trade_data()
    }

    pub const fn source(self) -> StudyDataSource {
        match self {
            Self::VolumeProfile { source, .. } | Self::TpoProfile { source, .. } => source,
            Self::BigTrades { .. } => StudyDataSource::Trades,
        }
    }

    /// How the study relates to the pane's exclusive profile source.
    pub const fn source_constraint(self) -> SourceConstraint {
        match self {
            Self::VolumeProfile { source, .. } | Self::TpoProfile { source, .. } => {
                SourceConstraint::Exclusive(source)
            }
            Self::BigTrades { .. } => SourceConstraint::Independent,
        }
    }

    pub fn same_family(self, other: Self) -> bool {
        matches!(
            (self, other),
            (
                Self::VolumeProfile {
                    schedule: VolumeProfileSchedule::VisibleRange,
                    ..
                },
                Self::VolumeProfile {
                    schedule: VolumeProfileSchedule::VisibleRange,
                    ..
                },
            ) | (
                Self::VolumeProfile {
                    schedule: VolumeProfileSchedule::Recurring { .. },
                    ..
                },
                Self::VolumeProfile {
                    schedule: VolumeProfileSchedule::Recurring { .. },
                    ..
                },
            ) | (Self::TpoProfile { .. }, Self::TpoProfile { .. })
                | (Self::BigTrades { .. }, Self::BigTrades { .. })
        )
    }

    pub fn normalized_for_basis(self, basis: Basis) -> Option<Self> {
        if self.is_interval_available(basis) {
            return Some(self);
        }

        match self {
            Self::VolumeProfile {
                source,
                schedule: VolumeProfileSchedule::Recurring { .. },
                colors,
                width_percent,
                opacity_percent,
                visible,
            } => VolumeProfileSchedule::minimum_recurring_interval(basis).map(|interval| {
                Self::VolumeProfile {
                    source,
                    schedule: VolumeProfileSchedule::Recurring { interval },
                    colors,
                    width_percent,
                    opacity_percent,
                    visible,
                }
            }),
            Self::VolumeProfile { .. } => None,
            Self::TpoProfile { .. } | Self::BigTrades { .. } => {
                self.is_available(basis).then_some(self)
            }
        }
    }

    pub const fn volume_profile_schedule(self) -> Option<VolumeProfileSchedule> {
        match self {
            Self::VolumeProfile { schedule, .. } => Some(schedule),
            Self::TpoProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub const fn volume_profile_colors(self) -> Option<VolumeProfileColorConfig> {
        match self {
            Self::VolumeProfile { colors, .. } => Some(colors),
            Self::TpoProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub const fn volume_profile_width_percent(self) -> Option<u8> {
        match self {
            Self::VolumeProfile { width_percent, .. } => Some(width_percent),
            Self::TpoProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub const fn volume_profile_opacity_percent(self) -> Option<u8> {
        match self {
            Self::VolumeProfile {
                opacity_percent, ..
            } => Some(opacity_percent),
            Self::TpoProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub fn volume_profile_opacity_ratio(self) -> Option<f32> {
        Some(
            f32::from(self.volume_profile_opacity_percent()?.clamp(
                MIN_VOLUME_PROFILE_OPACITY_PERCENT,
                MAX_VOLUME_PROFILE_OPACITY_PERCENT,
            )) / f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT),
        )
    }

    pub fn volume_profile_opacity_scale(self) -> Option<f32> {
        let opacity = f32::from(self.volume_profile_opacity_percent()?.clamp(
            MIN_VOLUME_PROFILE_OPACITY_PERCENT,
            MAX_VOLUME_PROFILE_OPACITY_PERCENT,
        ));
        Some(opacity / f32::from(self.volume_profile_schedule()?.default_opacity_percent()))
    }

    pub fn volume_profile_width_ratio(self) -> Option<f32> {
        Some(
            f32::from(self.volume_profile_width_percent()?.clamp(
                MIN_VOLUME_PROFILE_WIDTH_PERCENT,
                MAX_VOLUME_PROFILE_WIDTH_PERCENT,
            )) / f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
        )
    }

    pub const fn is_tpo_profile(self) -> bool {
        matches!(self, Self::TpoProfile { .. })
    }

    pub const fn big_trades(self) -> Option<BigTradesConfig> {
        match self {
            Self::BigTrades {
                threshold,
                buy_color,
                sell_color,
                ..
            } => Some(BigTradesConfig {
                threshold,
                buy_color,
                sell_color,
            }),
            _ => None,
        }
    }

    pub const fn tpo_anchor(self) -> Option<TpoAnchor> {
        match self {
            Self::TpoProfile { anchor, .. } => Some(anchor),
            Self::VolumeProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub const fn tpo_colors(self) -> Option<TpoColorConfig> {
        match self {
            Self::TpoProfile { colors, .. } => Some(colors),
            Self::VolumeProfile { .. } | Self::BigTrades { .. } => None,
        }
    }

    pub const fn recurring_profile_interval(self) -> Option<Timeframe> {
        match self {
            Self::VolumeProfile {
                schedule: VolumeProfileSchedule::Recurring { interval },
                ..
            } => Some(interval),
            Self::VolumeProfile {
                schedule: VolumeProfileSchedule::VisibleRange,
                ..
            } => None,
            Self::TpoProfile { .. } | Self::BigTrades { .. } => None,
        }
    }
}

impl Display for KlineStudy {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VolumeProfile {
                source, schedule, ..
            } => {
                write!(formatter, "{source} Volume Profile ({schedule})")
            }
            Self::TpoProfile {
                source,
                anchor,
                letter_period,
                ..
            } => {
                write!(
                    formatter,
                    "{source} TPO Profile ({anchor}, {letter_period})"
                )
            }
            Self::BigTrades { threshold, .. } => {
                write!(
                    formatter,
                    "Big Trades ({})",
                    big_trades_threshold_label(*threshold)
                )
            }
        }
    }
}
