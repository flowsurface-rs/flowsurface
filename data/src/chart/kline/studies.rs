pub mod tpo;
pub mod volume_profile;

use self::tpo::TpoAnchor;

use crate::chart::Basis;
use crate::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::unit::price::PriceStep;
use exchange::{TickMultiplier, TickerInfo, Timeframe};

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub const DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL: Timeframe = Timeframe::M15;
pub const MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER: u64 = 4;
pub const DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 20;
pub const DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 20;
pub const DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 55;
pub const MIN_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 100;
pub const MIN_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 100;
pub const DEFAULT_TPO_LETTER_PERIOD: Timeframe = Timeframe::M30;
pub const DEFAULT_TPO_VALUE_AREA_PERCENT: u8 = 70;
pub const MIN_TPO_VALUE_AREA_PERCENT: u8 = 5;
pub const MAX_TPO_VALUE_AREA_PERCENT: u8 = 100;
pub(crate) const MAX_PROFILE_PRICE_ROWS: usize = 4_096;

fn default_volume_profile_width_percent() -> u8 {
    DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT
}

const fn default_study_visible() -> bool {
    true
}

const fn default_show_volume_profile_poc() -> bool {
    false
}

const fn default_show_volume_profile_npoc() -> bool {
    true
}

const fn default_tpo_letter_period() -> Timeframe {
    DEFAULT_TPO_LETTER_PERIOD
}

const fn default_tpo_anchor() -> TpoAnchor {
    TpoAnchor::Daily
}

const fn default_tpo_value_area_percent() -> u8 {
    DEFAULT_TPO_VALUE_AREA_PERCENT
}

const fn default_tpo_visible() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSource {
    Kline,
    Trades,
}

impl ProfileSource {
    pub const fn requires_trade_data(self) -> bool {
        matches!(self, Self::Trades)
    }
}

impl Display for ProfileSource {
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
        source: Option<ProfileSource>,
        schedule: VolumeProfileSchedule,
        #[serde(default)]
        colors: VolumeProfileColorConfig,
        #[serde(default = "default_volume_profile_width_percent")]
        width_percent: u8,
        opacity_percent: Option<u8>,
        #[serde(default = "default_study_visible")]
        visible: bool,
        #[serde(default)]
        tick_multiplier: Option<TickMultiplier>,
    },
    TpoProfile {
        #[serde(default)]
        source: Option<ProfileSource>,
        #[serde(default = "default_tpo_anchor")]
        anchor: TpoAnchor,
        #[serde(default = "default_tpo_letter_period")]
        letter_period: Timeframe,
        #[serde(default, alias = "price_increment_ticks")]
        tick_multiplier: Option<TickMultiplier>,
        #[serde(default = "default_tpo_value_area_percent")]
        value_area_percent: u8,
        #[serde(default)]
        colors: TpoColorConfig,
        #[serde(default = "default_tpo_visible")]
        visible: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeProfileColorSlot {
    Buy,
    Sell,
    NakedPoc,
    FilledPoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeProfileColorSpec {
    pub label: &'static str,
    pub slot: VolumeProfileColorSlot,
}

const VOLUME_PROFILE_COLOR_SPECS: [VolumeProfileColorSpec; 4] = [
    VolumeProfileColorSpec {
        label: "Buy volume",
        slot: VolumeProfileColorSlot::Buy,
    },
    VolumeProfileColorSpec {
        label: "Sell volume",
        slot: VolumeProfileColorSlot::Sell,
    },
    VolumeProfileColorSpec {
        label: "NPOC",
        slot: VolumeProfileColorSlot::NakedPoc,
    },
    VolumeProfileColorSpec {
        label: "POC",
        slot: VolumeProfileColorSlot::FilledPoc,
    },
];

const KLINE_VOLUME_PROFILE_COLOR_SPECS: [VolumeProfileColorSpec; 4] = [
    VolumeProfileColorSpec {
        label: "Up volume",
        slot: VolumeProfileColorSlot::Buy,
    },
    VolumeProfileColorSpec {
        label: "Down volume",
        slot: VolumeProfileColorSlot::Sell,
    },
    VolumeProfileColorSpec {
        label: "NPOC",
        slot: VolumeProfileColorSlot::NakedPoc,
    },
    VolumeProfileColorSpec {
        label: "POC",
        slot: VolumeProfileColorSlot::FilledPoc,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeProfileColorConfig {
    pub buy: ThemeColor,
    pub sell: ThemeColor,
    pub naked_poc: ThemeColor,
    pub filled_poc: ThemeColor,
    #[serde(default = "default_show_volume_profile_poc")]
    pub show_poc: bool,
    #[serde(default = "default_show_volume_profile_npoc")]
    pub show_npoc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TpoColorConfig {
    pub letters: ThemeColor,
    pub poc: ThemeColor,
    pub value_area: ThemeColor,
    pub initial_balance: ThemeColor,
    pub single_print: ThemeColor,
    pub poor_high: ThemeColor,
    pub poor_low: ThemeColor,
    #[serde(default)]
    pub show_poc: bool,
    #[serde(default)]
    pub show_initial_balance: bool,
    #[serde(default)]
    pub show_single_print: bool,
    #[serde(default)]
    pub show_poor_high: bool,
    #[serde(default)]
    pub show_poor_low: bool,
}

impl Default for TpoColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl TpoColorConfig {
    pub const fn defaults() -> Self {
        Self {
            letters: ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Soft),
            poc: ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong),
            value_area: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Strong),
            initial_balance: ThemeColor::new(ThemeColorRole::Primary, ThemeColorShade::Base),
            single_print: ThemeColor::with_accent(
                ThemeColorRole::Neutral,
                Some(ThemeAccent::Three),
                ThemeColorShade::Base,
            ),
            poor_high: ThemeColor::with_accent(
                ThemeColorRole::Primary,
                Some(ThemeAccent::Seven),
                ThemeColorShade::Base,
            ),
            poor_low: ThemeColor::with_accent(
                ThemeColorRole::Primary,
                Some(ThemeAccent::Seven),
                ThemeColorShade::Base,
            ),
            show_poc: false,
            show_initial_balance: false,
            show_single_print: false,
            show_poor_high: false,
            show_poor_low: false,
        }
    }

    pub const fn with_show_poc(self, show_poc: bool) -> Self {
        Self { show_poc, ..self }
    }

    pub const fn with_show_initial_balance(self, show_initial_balance: bool) -> Self {
        Self {
            show_initial_balance,
            ..self
        }
    }

    pub const fn with_show_single_print(self, show_single_print: bool) -> Self {
        Self {
            show_single_print,
            ..self
        }
    }

    pub const fn with_show_poor_extremes(self, show: bool) -> Self {
        Self {
            show_poor_high: show,
            show_poor_low: show,
            ..self
        }
    }
}

impl Default for VolumeProfileColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl VolumeProfileColorConfig {
    pub const fn defaults() -> Self {
        Self {
            buy: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Strong),
            sell: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Soft),
            naked_poc: ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong),
            filled_poc: ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Strong),
            show_poc: false,
            show_npoc: true,
        }
    }

    pub const fn color_specs() -> &'static [VolumeProfileColorSpec] {
        &VOLUME_PROFILE_COLOR_SPECS
    }

    pub fn color_specs_for(schedule: VolumeProfileSchedule) -> &'static [VolumeProfileColorSpec] {
        Self::color_specs_for_source(ProfileSource::Trades, schedule)
    }

    pub fn color_specs_for_source(
        source: ProfileSource,
        schedule: VolumeProfileSchedule,
    ) -> &'static [VolumeProfileColorSpec] {
        let specs = match source {
            ProfileSource::Kline => &KLINE_VOLUME_PROFILE_COLOR_SPECS,
            ProfileSource::Trades => &VOLUME_PROFILE_COLOR_SPECS,
        };
        match schedule {
            VolumeProfileSchedule::VisibleRange => &specs[..2],
            VolumeProfileSchedule::Recurring { .. } => specs,
        }
    }

    pub const fn color_for(self, slot: VolumeProfileColorSlot) -> ThemeColor {
        match slot {
            VolumeProfileColorSlot::Buy => self.buy,
            VolumeProfileColorSlot::Sell => self.sell,
            VolumeProfileColorSlot::NakedPoc => self.naked_poc,
            VolumeProfileColorSlot::FilledPoc => self.filled_poc,
        }
    }

    pub const fn with_color(self, slot: VolumeProfileColorSlot, color: ThemeColor) -> Self {
        match slot {
            VolumeProfileColorSlot::Buy => Self { buy: color, ..self },
            VolumeProfileColorSlot::Sell => Self {
                sell: color,
                ..self
            },
            VolumeProfileColorSlot::NakedPoc => Self {
                naked_poc: color,
                ..self
            },
            VolumeProfileColorSlot::FilledPoc => Self {
                filled_poc: color,
                ..self
            },
        }
    }

    pub const fn with_show_poc(self, show_poc: bool) -> Self {
        Self { show_poc, ..self }
    }

    pub const fn with_show_npoc(self, show_npoc: bool) -> Self {
        Self { show_npoc, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeProfileSchedule {
    VisibleRange,
    Recurring { interval: Timeframe },
}

impl VolumeProfileSchedule {
    pub fn recurring_intervals_for(basis: Basis) -> Vec<Timeframe> {
        let Some(minimum_interval) = Self::minimum_recurring_interval(basis) else {
            return Vec::new();
        };

        Timeframe::KLINE
            .iter()
            .copied()
            .filter(|interval| interval.to_milliseconds() >= minimum_interval.to_milliseconds())
            .collect()
    }

    pub fn minimum_recurring_interval(basis: Basis) -> Option<Timeframe> {
        let Basis::Time(basis_interval) = basis else {
            return None;
        };

        let minimum_interval_ms = basis_interval
            .to_milliseconds()
            .saturating_mul(MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER);

        Timeframe::KLINE
            .iter()
            .copied()
            .find(|interval| interval.to_milliseconds() >= minimum_interval_ms)
    }

    pub fn is_available(self, basis: Basis) -> bool {
        match (self, basis) {
            (Self::VisibleRange, Basis::Time(_)) => true,
            (Self::VisibleRange, Basis::Tick(_)) => false,
            (Self::Recurring { .. }, Basis::Time(_)) => {
                Self::minimum_recurring_interval(basis).is_some()
            }
            (Self::Recurring { .. }, Basis::Tick(_)) => false,
        }
    }

    pub fn is_interval_available(self, basis: Basis) -> bool {
        match (self, basis) {
            (Self::VisibleRange, _) => self.is_available(basis),
            (Self::Recurring { interval }, Basis::Time(_)) => {
                Self::minimum_recurring_interval(basis).is_some_and(|minimum_interval| {
                    interval.to_milliseconds() >= minimum_interval.to_milliseconds()
                })
            }
            (Self::Recurring { .. }, Basis::Tick(_)) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KlineStudy {
    VolumeProfile {
        source: ProfileSource,
        schedule: VolumeProfileSchedule,
        #[serde(default)]
        colors: VolumeProfileColorConfig,
        #[serde(default = "default_volume_profile_width_percent")]
        width_percent: u8,
        opacity_percent: u8,
        #[serde(default = "default_study_visible")]
        visible: bool,
        #[serde(default)]
        tick_multiplier: Option<TickMultiplier>,
    },
    TpoProfile {
        source: ProfileSource,
        #[serde(default = "default_tpo_anchor")]
        anchor: TpoAnchor,
        #[serde(default = "default_tpo_letter_period")]
        letter_period: Timeframe,
        #[serde(default)]
        tick_multiplier: Option<TickMultiplier>,
        #[serde(default = "default_tpo_value_area_percent")]
        value_area_percent: u8,
        #[serde(default)]
        colors: TpoColorConfig,
        #[serde(default = "default_tpo_visible")]
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
                tick_multiplier,
            } => Ok(Self::VolumeProfile {
                source: source.unwrap_or(ProfileSource::Trades),
                schedule,
                colors,
                width_percent,
                opacity_percent: opacity_percent
                    .unwrap_or_else(|| Self::default_opacity_percent_for(schedule)),
                visible,
                tick_multiplier,
            }),
            PersistedKlineStudy::TpoProfile {
                source,
                anchor,
                letter_period,
                tick_multiplier,
                value_area_percent,
                colors,
                visible,
            } => Ok(Self::TpoProfile {
                source: source.unwrap_or(ProfileSource::Kline),
                anchor,
                letter_period,
                tick_multiplier,
                value_area_percent: value_area_percent
                    .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
                colors,
                visible,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlineStudyId {
    VolumeProfileVisibleRange(ProfileSource),
    VolumeProfileRecurring(ProfileSource),
    TpoProfile(ProfileSource),
}

impl KlineStudy {
    pub const ALL: [Self; 6] = [
        Self::VolumeProfile {
            source: ProfileSource::Kline,
            schedule: VolumeProfileSchedule::VisibleRange,
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
            tick_multiplier: None,
        },
        Self::TpoProfile {
            source: ProfileSource::Kline,
            anchor: TpoAnchor::Daily,
            letter_period: DEFAULT_TPO_LETTER_PERIOD,
            tick_multiplier: None,
            value_area_percent: DEFAULT_TPO_VALUE_AREA_PERCENT,
            colors: TpoColorConfig::defaults(),
            visible: true,
        },
        Self::VolumeProfile {
            source: ProfileSource::Kline,
            schedule: VolumeProfileSchedule::Recurring {
                interval: DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL,
            },
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
            tick_multiplier: None,
        },
        Self::VolumeProfile {
            source: ProfileSource::Trades,
            schedule: VolumeProfileSchedule::VisibleRange,
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
            tick_multiplier: None,
        },
        Self::TpoProfile {
            source: ProfileSource::Trades,
            anchor: TpoAnchor::Daily,
            letter_period: DEFAULT_TPO_LETTER_PERIOD,
            tick_multiplier: None,
            value_area_percent: DEFAULT_TPO_VALUE_AREA_PERCENT,
            colors: TpoColorConfig::defaults(),
            visible: true,
        },
        Self::VolumeProfile {
            source: ProfileSource::Trades,
            schedule: VolumeProfileSchedule::Recurring {
                interval: DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL,
            },
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
            tick_multiplier: None,
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
            Self::VolumeProfile { visible, .. } => visible,
            Self::TpoProfile { visible, .. } => visible,
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
                tick_multiplier,
                ..
            } => Self::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
            },
            Self::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent,
                colors,
                tick_multiplier,
                ..
            } => Self::TpoProfile {
                source,
                anchor,
                letter_period,
                tick_multiplier,
                value_area_percent,
                colors,
                visible,
            },
        }
    }

    const fn default_opacity_percent_for(schedule: VolumeProfileSchedule) -> u8 {
        match schedule {
            VolumeProfileSchedule::VisibleRange => DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            VolumeProfileSchedule::Recurring { .. } => {
                DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT
            }
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
        }
    }

    pub fn is_same_type(self, other: Self) -> bool {
        self.id() == other.id()
    }

    pub fn is_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_available(basis),
            Self::TpoProfile { .. } => matches!(basis, Basis::Time(_)),
        }
    }

    pub fn is_interval_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_interval_available(basis),
            Self::TpoProfile { .. } => self.is_available(basis),
        }
    }

    pub const fn requires_trade_data(self) -> bool {
        self.source().requires_trade_data()
    }

    pub const fn source(self) -> ProfileSource {
        match self {
            Self::VolumeProfile { source, .. } | Self::TpoProfile { source, .. } => source,
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
                tick_multiplier,
            } => VolumeProfileSchedule::minimum_recurring_interval(basis).map(|interval| {
                Self::VolumeProfile {
                    source,
                    schedule: VolumeProfileSchedule::Recurring { interval },
                    colors,
                    width_percent,
                    opacity_percent,
                    visible,
                    tick_multiplier,
                }
            }),
            Self::VolumeProfile { .. } => None,
            Self::TpoProfile { .. } => self.is_available(basis).then_some(self),
        }
    }

    pub const fn volume_profile_schedule(self) -> Option<VolumeProfileSchedule> {
        match self {
            Self::VolumeProfile { schedule, .. } => Some(schedule),
            Self::TpoProfile { .. } => None,
        }
    }

    pub const fn profile_tick_multiplier(self) -> Option<TickMultiplier> {
        match self {
            Self::VolumeProfile {
                tick_multiplier, ..
            } => tick_multiplier,
            Self::TpoProfile {
                tick_multiplier, ..
            } => tick_multiplier,
        }
    }

    pub fn effective_profile_tick_size(
        self,
        ticker_info: TickerInfo,
        footprint_tick_size: PriceStep,
    ) -> PriceStep {
        let Some(multiplier) = self.profile_tick_multiplier() else {
            return footprint_tick_size;
        };
        let step = multiplier.multiply_with_min_tick_step(ticker_info);
        if step.units < footprint_tick_size.units {
            footprint_tick_size
        } else {
            step
        }
    }

    pub const fn with_volume_profile_tick_multiplier(
        self,
        tick_multiplier: Option<TickMultiplier>,
    ) -> Self {
        match self {
            Self::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                ..
            } => Self::VolumeProfile {
                source,
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
            },
            Self::TpoProfile { .. } => self,
        }
    }

    pub const fn volume_profile_colors(self) -> Option<VolumeProfileColorConfig> {
        match self {
            Self::VolumeProfile { colors, .. } => Some(colors),
            Self::TpoProfile { .. } => None,
        }
    }

    pub const fn volume_profile_width_percent(self) -> Option<u8> {
        match self {
            Self::VolumeProfile { width_percent, .. } => Some(width_percent),
            Self::TpoProfile { .. } => None,
        }
    }

    pub const fn volume_profile_opacity_percent(self) -> Option<u8> {
        match self {
            Self::VolumeProfile {
                opacity_percent, ..
            } => Some(opacity_percent),
            Self::TpoProfile { .. } => None,
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
        Some(
            opacity
                / f32::from(Self::default_opacity_percent_for(
                    self.volume_profile_schedule()?,
                )),
        )
    }

    pub fn volume_profile_width_ratio(self) -> Option<f32> {
        Some(
            f32::from(self.volume_profile_width_percent()?.clamp(
                MIN_VOLUME_PROFILE_WIDTH_PERCENT,
                MAX_VOLUME_PROFILE_WIDTH_PERCENT,
            )) / f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
        )
    }

    pub const fn is_volume_profile(self) -> bool {
        matches!(self, Self::VolumeProfile { .. })
    }

    pub const fn is_tpo_profile(self) -> bool {
        matches!(self, Self::TpoProfile { .. })
    }

    pub const fn tpo_anchor(self) -> Option<TpoAnchor> {
        match self {
            Self::TpoProfile { anchor, .. } => Some(anchor),
            Self::VolumeProfile { .. } => None,
        }
    }

    pub const fn tpo_colors(self) -> Option<TpoColorConfig> {
        match self {
            Self::TpoProfile { colors, .. } => Some(colors),
            Self::VolumeProfile { .. } => None,
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
            Self::TpoProfile { .. } => None,
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
        }
    }
}

impl Display for VolumeProfileSchedule {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VisibleRange => formatter.write_str("Visible Range"),
            Self::Recurring { interval } => write!(formatter, "Recurring {interval}"),
        }
    }
}
