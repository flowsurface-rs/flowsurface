use crate::chart::Basis;
use exchange::{TickMultiplier, Timeframe};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

use super::{ThemeColor, ThemeColorRole, ThemeColorShade};

pub const DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL: Timeframe = Timeframe::M15;
pub const MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER: u64 = 4;
pub const DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 20;
pub const DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 20;
pub const DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 55;
pub const MIN_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 100;
pub const MIN_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 100;

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

#[derive(Debug, Clone, Copy, Deserialize)]
enum PersistedKlineStudy {
    VolumeProfile {
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
        match schedule {
            VolumeProfileSchedule::VisibleRange => &VOLUME_PROFILE_COLOR_SPECS[..2],
            VolumeProfileSchedule::Recurring { .. } => &VOLUME_PROFILE_COLOR_SPECS,
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
}

impl<'de> Deserialize<'de> for KlineStudy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match PersistedKlineStudy::deserialize(deserializer)? {
            PersistedKlineStudy::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
            } => Ok(Self::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent: opacity_percent
                    .unwrap_or_else(|| Self::default_opacity_percent_for(schedule)),
                visible,
                tick_multiplier,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlineStudyId {
    VolumeProfileVisibleRange,
    VolumeProfileRecurring,
}

impl KlineStudy {
    pub const ALL: [Self; 2] = [
        Self::VolumeProfile {
            schedule: VolumeProfileSchedule::VisibleRange,
            colors: VolumeProfileColorConfig::defaults(),
            width_percent: DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
            opacity_percent: DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            visible: true,
            tick_multiplier: None,
        },
        Self::VolumeProfile {
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
        }
    }

    pub const fn with_visibility(self, visible: bool) -> Self {
        match self {
            Self::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent,
                tick_multiplier,
                ..
            } => Self::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
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
                schedule: VolumeProfileSchedule::VisibleRange,
                ..
            } => KlineStudyId::VolumeProfileVisibleRange,
            Self::VolumeProfile {
                schedule: VolumeProfileSchedule::Recurring { .. },
                ..
            } => KlineStudyId::VolumeProfileRecurring,
        }
    }

    pub fn is_same_type(self, other: Self) -> bool {
        self.id() == other.id()
    }

    pub fn is_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_available(basis),
        }
    }

    pub fn is_interval_available(self, basis: Basis) -> bool {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule.is_interval_available(basis),
        }
    }

    pub const fn requires_trade_data(self) -> bool {
        true
    }

    pub fn normalized_for_basis(self, basis: Basis) -> Option<Self> {
        if self.is_interval_available(basis) {
            return Some(self);
        }

        match self {
            Self::VolumeProfile {
                schedule: VolumeProfileSchedule::Recurring { .. },
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
            } => VolumeProfileSchedule::minimum_recurring_interval(basis).map(|interval| {
                Self::VolumeProfile {
                    schedule: VolumeProfileSchedule::Recurring { interval },
                    colors,
                    width_percent,
                    opacity_percent,
                    visible,
                    tick_multiplier,
                }
            }),
            Self::VolumeProfile { .. } => None,
        }
    }

    pub const fn volume_profile_schedule(self) -> VolumeProfileSchedule {
        match self {
            Self::VolumeProfile { schedule, .. } => schedule,
        }
    }

    pub const fn volume_profile_tick_multiplier(self) -> Option<TickMultiplier> {
        match self {
            Self::VolumeProfile {
                tick_multiplier, ..
            } => tick_multiplier,
        }
    }

    pub const fn with_volume_profile_tick_multiplier(
        self,
        tick_multiplier: Option<TickMultiplier>,
    ) -> Self {
        match self {
            Self::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                ..
            } => Self::VolumeProfile {
                schedule,
                colors,
                width_percent,
                opacity_percent,
                visible,
                tick_multiplier,
            },
        }
    }

    pub const fn volume_profile_colors(self) -> VolumeProfileColorConfig {
        match self {
            Self::VolumeProfile { colors, .. } => colors,
        }
    }

    pub const fn volume_profile_width_percent(self) -> u8 {
        match self {
            Self::VolumeProfile { width_percent, .. } => width_percent,
        }
    }

    pub const fn volume_profile_opacity_percent(self) -> u8 {
        match self {
            Self::VolumeProfile {
                opacity_percent, ..
            } => opacity_percent,
        }
    }

    pub fn volume_profile_opacity_ratio(self) -> f32 {
        f32::from(self.volume_profile_opacity_percent().clamp(
            MIN_VOLUME_PROFILE_OPACITY_PERCENT,
            MAX_VOLUME_PROFILE_OPACITY_PERCENT,
        )) / f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT)
    }

    pub fn volume_profile_opacity_scale(self) -> f32 {
        f32::from(self.volume_profile_opacity_percent().clamp(
            MIN_VOLUME_PROFILE_OPACITY_PERCENT,
            MAX_VOLUME_PROFILE_OPACITY_PERCENT,
        )) / f32::from(Self::default_opacity_percent_for(
            self.volume_profile_schedule(),
        ))
    }

    pub fn volume_profile_width_ratio(self) -> f32 {
        f32::from(self.volume_profile_width_percent().clamp(
            MIN_VOLUME_PROFILE_WIDTH_PERCENT,
            MAX_VOLUME_PROFILE_WIDTH_PERCENT,
        )) / f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT)
    }

    pub const fn is_volume_profile(self) -> bool {
        matches!(self, Self::VolumeProfile { .. })
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
        }
    }
}

impl Display for KlineStudy {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VolumeProfile { schedule, .. } => {
                write!(formatter, "Volume Profile ({schedule})")
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

#[cfg(test)]
mod tests {
    use super::{KlineStudy, TickMultiplier};

    #[test]
    fn study_visibility_is_backward_compatible_and_persistent() {
        let hidden = KlineStudy::ALL[0].with_visibility(false);
        let serialized = serde_json::to_value(hidden).expect("study should serialize");

        let restored: KlineStudy = serde_json::from_value(serialized.clone())
            .expect("study visibility should deserialize");
        assert!(!restored.visible());

        let mut legacy = serialized;
        legacy
            .get_mut("VolumeProfile")
            .and_then(serde_json::Value::as_object_mut)
            .expect("serialized study should have a variant object")
            .remove("visible");
        let restored_legacy: KlineStudy =
            serde_json::from_value(legacy).expect("legacy study should deserialize");
        assert!(restored_legacy.visible());
    }

    #[test]
    fn study_tick_multiplier_is_persisted_and_backward_compatible() {
        let configured =
            KlineStudy::ALL[0].with_volume_profile_tick_multiplier(Some(TickMultiplier(25)));
        let serialized = serde_json::to_value(configured).expect("study should serialize");

        let restored: KlineStudy = serde_json::from_value(serialized.clone())
            .expect("study tick multiplier should deserialize");
        assert_eq!(
            restored.volume_profile_tick_multiplier(),
            Some(TickMultiplier(25))
        );

        let mut legacy = serialized;
        legacy
            .get_mut("VolumeProfile")
            .and_then(serde_json::Value::as_object_mut)
            .expect("serialized study should have a variant object")
            .remove("tick_multiplier");
        let restored_legacy: KlineStudy =
            serde_json::from_value(legacy).expect("legacy study should deserialize");
        assert_eq!(restored_legacy.volume_profile_tick_multiplier(), None);
    }
}
