use super::{IndicatorAvailability, IndicatorPanelRecipe};
use crate::widget::chart::kline::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelValueId,
};
use exchange::{Kline, UnixMs};
use std::collections::{BTreeMap, VecDeque};

const SERIES_MAX_POINTS: usize = 5000;
const DEFAULT_PERIOD: usize = 20;
const DEFAULT_STDDEV_MULTIPLIER: f32 = 2.0;

pub fn kline_warmup_bars() -> u64 {
    DEFAULT_PERIOD as u64
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerBandsPoint {
    pub basis: f32,
    pub upper: f32,
    pub lower: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BollingerBandsState {
    values: BTreeMap<UnixMs, BollingerBandsPoint>,
}

impl BollingerBandsState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn recompute_from_bars(&mut self, bars: &[Kline]) {
        self.values.clear();

        let period = DEFAULT_PERIOD;
        if period == 0 {
            return;
        }

        let mut window = VecDeque::with_capacity(period + 1);
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;

        for bar in bars {
            let close = bar.close.to_f32();
            window.push_back(close);
            sum += close;
            sum_sq += close * close;

            if window.len() > period
                && let Some(removed) = window.pop_front()
            {
                sum -= removed;
                sum_sq -= removed * removed;
            }

            if window.len() == period {
                let count = period as f32;
                let basis = sum / count;
                let variance = (sum_sq / count - basis * basis).max(0.0);
                let std_dev = variance.sqrt();
                let delta = std_dev * DEFAULT_STDDEV_MULTIPLIER;

                self.values.insert(
                    bar.time,
                    BollingerBandsPoint {
                        basis,
                        upper: basis + delta,
                        lower: basis - delta,
                    },
                );
            }
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<BollingerBandsPoint> {
        self.values.get(&time).copied()
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
    IndicatorPanelRecipe::PrimaryOverlay {
        layer_name: "Bollinger Basis",
        source: DataSourceId::Primary,
        value_id: PanelValueId::BollingerBands,
        data_kind: LayerDataKind::Scalar,
        mark: MarkKind::Line,
        axis: AxisBinding::Primary,
    }
}

pub fn availability() -> IndicatorAvailability {
    IndicatorAvailability::Available
}
