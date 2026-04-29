mod bollinger;
mod cvd;
mod open_interest;
mod rsi;
mod volume;

use crate::widget::chart::kline::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueId,
    PanelValuePrecision,
};
use data::chart::Basis;
use exchange::adapter::MarketKind;
use exchange::{Kline, OpenInterest, TickerInfo, Timeframe, UnixMs};

use super::KlineIndicator;

pub use cvd::CapabilityProbe as CvdInputProbe;
pub use rsi::RsiConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorUnsupportedReason {
    BasisNotSupported,
    SourceNotSupported,
    ResolutionNotSupported,
    MissingRequiredInput,
    InconsistentInputCoverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorAvailability {
    Available,
    PendingProbe,
    Partial {
        available: usize,
        total: usize,
        reason: IndicatorUnsupportedReason,
    },
    Unsupported(IndicatorUnsupportedReason),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorPanelRecipe {
    AuxPanel {
        panel_title: &'static str,
        layer_name: &'static str,
        source: DataSourceId,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
        value_precision: PanelValuePrecision,
        preferred_scale: PanelScaleMode,
    },
    PrimaryOverlay {
        layer_name: &'static str,
        source: DataSourceId,
        value_id: PanelValueId,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AvailabilityContext {
    pub basis: Basis,
    pub timeframe: Timeframe,
    pub base_ticker: TickerInfo,
}

const FOR_SPOT: [KlineIndicator; 4] = [
    KlineIndicator::Volume,
    KlineIndicator::BollingerBands,
    KlineIndicator::Rsi,
    KlineIndicator::CumulativeVolumeDelta,
];
const FOR_PERPS: [KlineIndicator; 5] = [
    KlineIndicator::Volume,
    KlineIndicator::BollingerBands,
    KlineIndicator::Rsi,
    KlineIndicator::OpenInterest,
    KlineIndicator::CumulativeVolumeDelta,
];
const ALL_INDICATORS: [KlineIndicator; 5] = [
    KlineIndicator::Volume,
    KlineIndicator::BollingerBands,
    KlineIndicator::Rsi,
    KlineIndicator::OpenInterest,
    KlineIndicator::CumulativeVolumeDelta,
];

pub fn all_indicators() -> &'static [KlineIndicator] {
    &ALL_INDICATORS
}

pub fn indicators_for_market(market: MarketKind) -> &'static [KlineIndicator] {
    match market {
        MarketKind::Spot => &FOR_SPOT,
        MarketKind::LinearPerps | MarketKind::InversePerps => &FOR_PERPS,
    }
}

pub fn is_supported_for_market(indicator: KlineIndicator, market: MarketKind) -> bool {
    indicators_for_market(market).contains(&indicator)
}

pub fn display_name(indicator: KlineIndicator) -> &'static str {
    match indicator {
        KlineIndicator::Volume => "Volume",
        KlineIndicator::BollingerBands => "Bollinger Bands",
        KlineIndicator::Rsi => "RSI",
        KlineIndicator::OpenInterest => "Open Interest",
        KlineIndicator::CumulativeVolumeDelta => "CVD",
    }
}

pub fn panel_recipe(indicator: KlineIndicator) -> IndicatorPanelRecipe {
    match indicator {
        KlineIndicator::Volume => volume::panel_recipe(),
        KlineIndicator::BollingerBands => bollinger::panel_recipe(),
        KlineIndicator::Rsi => rsi::panel_recipe(),
        KlineIndicator::OpenInterest => open_interest::panel_recipe(),
        KlineIndicator::CumulativeVolumeDelta => cvd::panel_recipe(),
    }
}

pub fn kline_warmup_bars(indicator: KlineIndicator, rsi_config: RsiConfig) -> Option<u64> {
    match indicator {
        KlineIndicator::Volume => None,
        KlineIndicator::BollingerBands => Some(bollinger::kline_warmup_bars()),
        KlineIndicator::Rsi => Some(rsi::kline_warmup_bars(rsi_config)),
        KlineIndicator::OpenInterest => None,
        KlineIndicator::CumulativeVolumeDelta => None,
    }
}

pub fn availability<'a, I>(
    indicator: KlineIndicator,
    context: AvailabilityContext,
    series_data: I,
) -> IndicatorAvailability
where
    I: IntoIterator<Item = &'a SeriesIndicatorData>,
{
    if !is_supported_for_market(indicator, context.base_ticker.market_type()) {
        return IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::SourceNotSupported);
    }

    match indicator {
        KlineIndicator::Volume => volume::availability(),
        KlineIndicator::BollingerBands => bollinger::availability(),
        KlineIndicator::Rsi => rsi::availability(context.basis),
        KlineIndicator::OpenInterest => {
            open_interest::availability(context.basis, context.timeframe, context.base_ticker)
        }
        KlineIndicator::CumulativeVolumeDelta => cvd::availability(
            context.basis,
            series_data.into_iter().map(SeriesIndicatorData::cvd_probe),
        ),
    }
}

#[derive(Debug, Clone, Default)]
pub struct SeriesIndicatorData {
    bollinger_bands: bollinger::BollingerBandsState,
    rsi: rsi::RsiState,
    open_interest: open_interest::OpenInterestState,
    cumulative_volume_delta: cvd::CumulativeVolumeDeltaState,
}

impl SeriesIndicatorData {
    pub fn set_rsi_config(&mut self, config: RsiConfig) -> bool {
        self.rsi.set_config(config)
    }

    pub fn clear(&mut self) {
        self.bollinger_bands.clear();
        self.rsi.clear();
        self.open_interest.clear();
        self.cumulative_volume_delta.clear();
    }

    pub fn refresh_from_bars(&mut self, bars: &[Kline]) {
        self.bollinger_bands.recompute_from_bars(bars);
        self.rsi.recompute_from_bars(bars);
        self.cumulative_volume_delta.recompute_from_bars(bars);
    }

    pub fn insert_open_interest_batch(
        &mut self,
        data: &[OpenInterest],
        basis: Basis,
        timeframe: Timeframe,
    ) {
        self.open_interest.insert_batch(data, basis, timeframe);
    }

    pub fn oi_timerange(&self) -> Option<(UnixMs, UnixMs)> {
        self.open_interest.timerange()
    }

    pub fn cvd_probe(&self) -> CvdInputProbe {
        self.cumulative_volume_delta.probe()
    }

    pub fn value_for_indicator(
        &self,
        indicator: Option<KlineIndicator>,
        bar: &Kline,
    ) -> Option<f32> {
        match indicator {
            Some(KlineIndicator::BollingerBands) => self
                .bollinger_bands
                .value_at(bar.time)
                .map(|bands| bands.basis),
            Some(KlineIndicator::Rsi) => self.rsi.value_at(bar.time).map(|point| point.value),
            Some(KlineIndicator::OpenInterest) => self.open_interest.value_at(bar.time),
            Some(KlineIndicator::CumulativeVolumeDelta) => {
                self.cumulative_volume_delta.value_at(bar.time)
            }
            Some(KlineIndicator::Volume) | None => Some(f32::from(bar.volume.total())),
        }
    }

    pub fn rsi_fields_for_bar(&self, bar: &Kline) -> Option<rsi::RsiPoint> {
        self.rsi.value_at(bar.time)
    }

    pub fn bollinger_bands_for_bar(&self, bar: &Kline) -> Option<(f32, f32)> {
        self.bollinger_bands
            .value_at(bar.time)
            .map(|bands| (bands.upper, bands.lower))
    }

    pub fn volume_overlay_for_bar(&self, bar: &Kline) -> Option<f32> {
        if self.cumulative_volume_delta.probe() != cvd::CapabilityProbe::Complete {
            return None;
        }

        bar.volume
            .buy_sell()
            .map(|(buy, sell)| f32::from(buy - sell))
    }
}
