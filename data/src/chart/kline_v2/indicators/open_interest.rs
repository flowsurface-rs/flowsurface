use super::super::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueLabelMode,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use super::{IndicatorAvailability, IndicatorPanelRecipe, IndicatorUnsupportedReason};
use crate::chart::Basis;
use exchange::adapter::Exchange;
use exchange::{OpenInterest, TickerInfo, Timeframe, UnixMs};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;

#[derive(Debug, Clone, Default)]
pub struct OpenInterestState {
    values: BTreeMap<UnixMs, f32>,
}

impl OpenInterestState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn insert_batch(&mut self, data: &[OpenInterest], basis: Basis, timeframe: Timeframe) {
        for oi in data {
            let time = Self::align_time(oi.time, basis, timeframe);
            self.values.insert(time, oi.value as f32);
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<f32> {
        self.values.get(&time).copied()
    }

    pub fn timerange(&self) -> Option<(UnixMs, UnixMs)> {
        let earliest = self.values.keys().next().copied()?;
        let latest = self.values.keys().next_back().copied()?;
        Some((earliest, latest))
    }

    fn align_time(time: UnixMs, basis: Basis, timeframe: Timeframe) -> UnixMs {
        match basis {
            Basis::Time(_) => time.floor_to(timeframe),
            Basis::Tick(_) => time,
        }
    }

    fn trim(&mut self) {
        while self.values.len() > SERIES_MAX_POINTS {
            if let Some((&earliest, _)) = self.values.first_key_value() {
                self.values.remove(&earliest);
            } else {
                break;
            }
        }
    }
}

pub fn panel_recipe() -> IndicatorPanelRecipe {
    IndicatorPanelRecipe::AuxPanel {
        panel_title: "Open Interest",
        layer_name: "Open Interest",
        source: DataSourceId::Primary,
        data_kind: LayerDataKind::Scalar,
        mark: MarkKind::Line,
        axis: AxisBinding::Secondary,
        value_precision: PanelValuePrecision::BaseTickerMinQty,
        value_label_policy: PanelValueLabelPolicy {
            axis_mode: PanelValueLabelMode::Abbreviated,
            header_mode: PanelValueLabelMode::Commas,
            max_decimals: None,
        },
        preferred_scale: PanelScaleMode::FitVisible,
    }
}

pub fn availability(
    basis: Basis,
    timeframe: Timeframe,
    base_ticker: TickerInfo,
) -> IndicatorAvailability {
    if matches!(basis, Basis::Tick(_)) {
        return IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::BasisNotSupported);
    }

    if !is_supported_exchange(base_ticker.exchange()) {
        return IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::SourceNotSupported);
    }

    if !is_supported_timeframe(timeframe) {
        return IndicatorAvailability::Unsupported(
            IndicatorUnsupportedReason::ResolutionNotSupported,
        );
    }

    IndicatorAvailability::Available
}

fn is_supported_exchange(exchange: Exchange) -> bool {
    exchange.is_perps()
        && exchange != Exchange::HyperliquidLinear
        && exchange != Exchange::MexcLinear
        && exchange != Exchange::MexcInverse
}

fn is_supported_timeframe(timeframe: Timeframe) -> bool {
    timeframe >= Timeframe::M5 && timeframe <= Timeframe::H4 && timeframe != Timeframe::H2
}
