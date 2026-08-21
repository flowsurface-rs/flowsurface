use super::super::values::IndicatorData;
use super::input::IndicatorInput;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    AvailabilityContext, IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec,
    IndicatorOverlayChannelSpec, IndicatorOverlayColorRole, IndicatorOverlaySource,
    IndicatorRuntimeSnapshot, KlinePriceSource,
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

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.values
            .get(&bar.time)
            .copied()
            .map(IndicatorData::scalar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::Volume;
    use exchange::unit::Qty;

    fn bar(time: u64, close: f64, volume: f64) -> Kline {
        Kline::new(
            time,
            close,
            close,
            close,
            close,
            Volume::TotalOnly(Qty::from_f64(volume)),
            exchange::unit::MinTicksize::new(-2),
        )
    }

    #[test]
    fn computes_weighted_price_from_kline_input() {
        let mut state = AnchoredVwapState::default();
        let bars = [
            bar(1_704_067_200_000, 10.0, 1.0),
            bar(1_704_067_260_000, 20.0, 3.0),
        ];

        state.on_input(
            IndicatorInput::Klines {
                bars: &bars,
                times: None,
            },
            AnchoredVwapConfig::default(),
        );

        assert_eq!(
            state.data_for_bar(&bars[1]).map(IndicatorData::value),
            Some(17.5)
        );
    }

    #[test]
    fn custom_anchor_excludes_prior_bars() {
        let mut state = AnchoredVwapState::default();
        let config = AnchoredVwapConfig {
            anchor: VwapAnchor::Custom(UnixMs::new(2)),
            source: KlinePriceSource::Close,
        };
        let bars = [bar(1, 10.0, 1.0), bar(2, 20.0, 1.0), bar(3, 40.0, 1.0)];

        state.on_input(
            IndicatorInput::Klines {
                bars: &bars,
                times: None,
            },
            config,
        );

        assert_eq!(state.data_for_bar(&bars[0]), None);
        assert_eq!(
            state.data_for_bar(&bars[2]).map(IndicatorData::value),
            Some(30.0)
        );
    }

    #[test]
    fn month_anchor_requires_five_minute_or_larger_timeframes() {
        let config = AnchoredVwapConfig {
            anchor: VwapAnchor::Month,
            source: KlinePriceSource::Close,
        };

        assert!(!config.supports_basis(Basis::Time(exchange::Timeframe::M1)));
        assert!(!config.supports_basis(Basis::Time(exchange::Timeframe::M3)));
        assert!(config.supports_basis(Basis::Time(exchange::Timeframe::M5)));
        assert_eq!(
            config.required_history_bars(exchange::Timeframe::M5),
            Some(8_929)
        );
    }

    #[test]
    fn unsupported_month_anchor_normalizes_to_day() {
        let config = AnchoredVwapConfig {
            anchor: VwapAnchor::Month,
            source: KlinePriceSource::Open,
        };

        assert_eq!(
            config.normalized_for_basis(Basis::Time(exchange::Timeframe::M3)),
            AnchoredVwapConfig {
                anchor: VwapAnchor::Day,
                source: KlinePriceSource::Open,
            }
        );
    }

    #[test]
    fn tick_market_times_do_not_replace_synthetic_bar_outputs() {
        let mut state = AnchoredVwapState::default();
        let bars = [bar(0, 10.0, 1.0), bar(1, 20.0, 1.0)];
        let market_times = [
            UnixMs::new(1_704_067_200_000),
            UnixMs::new(1_704_067_200_000),
        ];

        state.on_input(
            IndicatorInput::Klines {
                bars: &bars,
                times: Some(&market_times),
            },
            AnchoredVwapConfig::default(),
        );

        assert_eq!(
            state.data_for_bar(&bars[0]).map(IndicatorData::value),
            Some(10.0)
        );
        assert_eq!(
            state.data_for_bar(&bars[1]).map(IndicatorData::value),
            Some(15.0)
        );
    }
}
