use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::IndicatorInput;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    AvailabilityContext, IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec,
    IndicatorConfigValue, IndicatorHistoryRequirement, IndicatorOverlayChannelSpec,
    IndicatorOverlayColorRole, IndicatorOverlaySource, IndicatorRuntimeSnapshot, IndicatorSetting,
    IndicatorSettingChoice, IndicatorSettingControl, IndicatorSettingValue,
    IndicatorSettingsContext, IndicatorUnsupportedReason, KlinePriceSource,
};
use crate::chart::Basis;
use crate::config::timezone::UserTimezone;
use chrono::Datelike;
use exchange::{Kline, Timeframe, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 1] = [IndicatorColorSpec {
    label: "Line",
    slot: IndicatorColorSlot::Main,
}];

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 1] =
    [IndicatorOverlayChannelSpec {
        label: "VWAP",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.2,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Main,
    }];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VwapAnchor {
    Day,
    Week,
    Month,
    Custom(UnixMs),
}

impl VwapAnchor {
    pub const STANDARD: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    pub fn supports_basis(self, basis: Basis) -> bool {
        match self {
            Self::Month => matches!(basis, Basis::Time(timeframe) if timeframe >= Timeframe::M5),
            Self::Day | Self::Week | Self::Custom(_) => true,
        }
    }

    pub fn history_start(self, time: UnixMs, timezone: UserTimezone) -> Option<UnixMs> {
        match self {
            Self::Day => timezone
                .start_of_local_day_utc_ms(time.as_u64())
                .map(UnixMs::new),
            Self::Week => {
                let datetime = time.as_datetime_utc()?;
                let local_date = timezone.to_user_datetime(datetime).date_naive();
                let days_from_monday = u64::from(local_date.weekday().num_days_from_monday());
                let monday = local_date.checked_sub_days(chrono::Days::new(days_from_monday))?;
                timezone.local_midnight_utc_ms(monday).map(UnixMs::new)
            }
            Self::Month => timezone
                .start_of_local_month_utc_ms(time.as_u64())
                .map(UnixMs::new),
            Self::Custom(anchor) => (time >= anchor).then_some(anchor),
        }
    }
}

impl Display for VwapAnchor {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Custom(_) => "Custom",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchoredVwapConfig {
    pub anchor: VwapAnchor,
    pub source: KlinePriceSource,
}

impl Default for AnchoredVwapConfig {
    fn default() -> Self {
        Self {
            anchor: VwapAnchor::Day,
            source: KlinePriceSource::Hlc3,
        }
    }
}

impl AnchoredVwapConfig {
    pub fn supports_basis(self, basis: Basis) -> bool {
        self.anchor.supports_basis(basis)
    }

    pub fn normalized_for_basis(self, basis: Basis) -> Self {
        if self.supports_basis(basis) {
            self
        } else {
            Self {
                anchor: VwapAnchor::Day,
                ..self
            }
        }
    }

    pub fn history_start(self, time: UnixMs, timezone: UserTimezone) -> Option<UnixMs> {
        self.anchor.history_start(time, timezone)
    }

    pub fn required_history_bars(self, timeframe: Timeframe) -> Option<u64> {
        let duration_ms: u64 = match self.anchor {
            VwapAnchor::Day => 24 * 60 * 60 * 1_000,
            VwapAnchor::Week => 7 * 24 * 60 * 60 * 1_000,
            VwapAnchor::Month => 31 * 24 * 60 * 60 * 1_000,
            VwapAnchor::Custom(_) => return None,
        };
        Some(duration_ms.div_ceil(timeframe.to_milliseconds()) + 1)
    }
}

impl IndicatorConfigValue for AnchoredVwapConfig {
    fn normalized_for_basis(self, basis: Basis) -> Self {
        self.normalized_for_basis(basis)
    }

    fn history_requirement(
        self,
        basis: Basis,
        reference_time: UnixMs,
    ) -> IndicatorHistoryRequirement {
        let Basis::Time(timeframe) = basis else {
            return IndicatorHistoryRequirement::default();
        };

        IndicatorHistoryRequirement::new(
            self.history_start(reference_time, UserTimezone::Utc),
            self.required_history_bars(timeframe).unwrap_or_default(),
        )
    }

    fn settings(self, context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        let anchors = VwapAnchor::STANDARD
            .into_iter()
            .filter(|anchor| anchor.supports_basis(context.basis))
            .map(|anchor| IndicatorSettingChoice {
                id: vwap_anchor_id(anchor),
                label: vwap_anchor_id(anchor),
            })
            .collect();
        vec![
            IndicatorSetting {
                id: "anchor",
                label: "Anchor",
                group: None,
                value: IndicatorSettingValue::Choice(vwap_anchor_id(self.anchor)),
                control: IndicatorSettingControl::Choices(anchors),
            },
            IndicatorSetting {
                id: "source",
                label: "Price source",
                group: None,
                value: IndicatorSettingValue::Choice(self.source.id()),
                control: IndicatorSettingControl::Choices(
                    KlinePriceSource::ALL
                        .into_iter()
                        .map(|source| IndicatorSettingChoice {
                            id: source.id(),
                            label: source.id(),
                        })
                        .collect(),
                ),
            },
        ]
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("anchor", IndicatorSettingValue::Choice(value)) => Some(Self {
                anchor: vwap_anchor_from_id(value)?,
                ..self
            }),
            ("source", IndicatorSettingValue::Choice(value)) => Some(Self {
                source: KlinePriceSource::from_id(value)?,
                ..self
            }),
            _ => None,
        }
    }
}

fn vwap_anchor_id(anchor: VwapAnchor) -> &'static str {
    match anchor {
        VwapAnchor::Day => "Day",
        VwapAnchor::Week => "Week",
        VwapAnchor::Month => "Month",
        VwapAnchor::Custom(_) => "Custom",
    }
}

fn vwap_anchor_from_id(id: &str) -> Option<VwapAnchor> {
    match id {
        "Day" => Some(VwapAnchor::Day),
        "Week" => Some(VwapAnchor::Week),
        "Month" => Some(VwapAnchor::Month),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct AnchoredVwapState {
    values: BTreeMap<UnixMs, f32>,
}

impl AnchoredVwapState {
    fn recompute(&mut self, bars: &[Kline], times: Option<&[UnixMs]>, config: AnchoredVwapConfig) {
        self.values.clear();
        let mut current_anchor = None;
        let mut cumulative_price_volume = 0.0;
        let mut cumulative_volume = 0.0;

        for (index, bar) in bars.iter().enumerate() {
            let market_time = times
                .and_then(|times| times.get(index).copied())
                .unwrap_or(bar.time);
            let Some(anchor) = config.history_start(market_time, UserTimezone::Utc) else {
                continue;
            };

            if current_anchor != Some(anchor) {
                current_anchor = Some(anchor);
                cumulative_price_volume = 0.0;
                cumulative_volume = 0.0;
            }

            let volume = bar.volume.total().to_f64();
            cumulative_price_volume += config.source.value(bar) * volume;
            cumulative_volume += volume;
            if cumulative_volume > f64::EPSILON {
                self.values.insert(
                    bar.time,
                    (cumulative_price_volume / cumulative_volume) as f32,
                );
            }
        }
    }
}

impl IndicatorDefinition for AnchoredVwapState {
    type Config = AnchoredVwapConfig;

    fn warmup_bars(_: Self::Config) -> Option<u64> {
        None
    }

    fn unsupported_reason(context: AvailabilityContext) -> Option<IndicatorUnsupportedReason> {
        matches!(context.basis, Basis::Tick(_))
            .then_some(IndicatorUnsupportedReason::IncompleteHistory)
    }

    fn availability(_: AvailabilityContext, _: IndicatorRuntimeSnapshot) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for AnchoredVwapState {
    fn clear(&mut self) {
        self.values.clear();
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], config: Self::Config) {
        self.recompute(bars, None, config);
    }

    fn on_input(&mut self, input: IndicatorInput<'_>, config: Self::Config) {
        if let IndicatorInput::Klines { bars, times } = input {
            self.recompute(bars, times, config);
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        config: Self::Config,
        _scope: IndicatorUpdateScope,
    ) {
        self.on_input(input, config);
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.values
            .get(&bar.time)
            .copied()
            .map(IndicatorData::scalar)
    }
}
