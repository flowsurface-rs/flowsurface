use super::{IndicatorAvailability, IndicatorPanelRecipe, IndicatorUnsupportedReason};
use crate::widget::chart::kline::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueLabelMode,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use data::chart::Basis;
use exchange::{Kline, UnixMs};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapabilityProbe {
    #[default]
    Unknown,
    Complete,
    MissingRequiredInput,
    InconsistentInputCoverage,
}

#[derive(Debug, Clone, Default)]
pub struct CumulativeVolumeDeltaState {
    values: BTreeMap<UnixMs, f32>,
    probe: CapabilityProbe,
}

impl CumulativeVolumeDeltaState {
    pub fn clear(&mut self) {
        self.values.clear();
        self.probe = CapabilityProbe::Unknown;
    }

    pub fn recompute_from_bars(&mut self, bars: &[Kline]) {
        self.values.clear();

        let mut saw_with_input = false;
        let mut saw_without_input = false;

        for bar in bars {
            if bar.volume.buy_sell().is_some() {
                saw_with_input = true;
            } else {
                saw_without_input = true;
            }
        }

        self.probe = match (saw_with_input, saw_without_input) {
            (false, false) => CapabilityProbe::Unknown,
            (true, false) => CapabilityProbe::Complete,
            (false, true) => CapabilityProbe::MissingRequiredInput,
            (true, true) => CapabilityProbe::InconsistentInputCoverage,
        };

        if self.probe != CapabilityProbe::Complete {
            return;
        }

        let mut cumulative = 0.0f32;
        for bar in bars {
            if let Some((buy, sell)) = bar.volume.buy_sell() {
                cumulative += (buy - sell).to_f32_lossy();
                self.values.insert(bar.time, cumulative);
            }
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<f32> {
        self.values.get(&time).copied()
    }

    pub fn probe(&self) -> CapabilityProbe {
        self.probe
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
        panel_title: "CVD",
        layer_name: "Cumulative Volume Delta",
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

pub fn availability(basis: Basis, probe: CapabilityProbe) -> IndicatorAvailability {
    if matches!(basis, Basis::Tick(_)) {
        return IndicatorAvailability::Available;
    }

    match probe {
        CapabilityProbe::Unknown => IndicatorAvailability::PendingProbe,
        CapabilityProbe::Complete => IndicatorAvailability::Available,
        CapabilityProbe::MissingRequiredInput => {
            IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::MissingRequiredInput)
        }
        CapabilityProbe::InconsistentInputCoverage => IndicatorAvailability::Unsupported(
            IndicatorUnsupportedReason::InconsistentInputCoverage,
        ),
    }
}
