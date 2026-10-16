use super::super::IndicatorUpdateScope;
use super::super::values::{IndicatorData, VOLUME_BUY_FIELD_KEY, VOLUME_SELL_FIELD_KEY};
use super::input::{IndicatorInput, NormalizedTradeBar};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorMarkSpec,
    IndicatorOverlayColorRole, IndicatorOverlaySource, IndicatorReadoutScope, IndicatorReadoutSpec,
};
use crate::chart::kline::composition::{BarMode, HistogramMode, MarkKind};
use exchange::{Kline, UnixMs, unit::Qty};
use std::collections::BTreeMap;

const LINE_COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Main];
const BAR_COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Bars];
const OVERLAY_COLOR_SLOTS: [IndicatorColorSlot; 2] =
    [IndicatorColorSlot::Positive, IndicatorColorSlot::Negative];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 4] = [
    IndicatorColorSpec {
        label: "Line",
        slot: IndicatorColorSlot::Main,
    },
    IndicatorColorSpec {
        label: "Bars",
        slot: IndicatorColorSlot::Bars,
    },
    IndicatorColorSpec {
        label: "Positive overlay",
        slot: IndicatorColorSlot::Positive,
    },
    IndicatorColorSpec {
        label: "Negative overlay",
        slot: IndicatorColorSlot::Negative,
    },
];

pub(super) const MARK_SPECS: [IndicatorMarkSpec; 3] = [
    IndicatorMarkSpec {
        label: "Line",
        mark: MarkKind::Line,
        color_slots: &LINE_COLOR_SLOTS,
    },
    IndicatorMarkSpec {
        label: "Bars",
        mark: MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        color_slots: &BAR_COLOR_SLOTS,
    },
    IndicatorMarkSpec {
        label: "Overlay bars",
        mark: MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay)),
        color_slots: &OVERLAY_COLOR_SLOTS,
    },
];

pub(super) const READOUT_SPECS: [IndicatorReadoutSpec; 2] = [
    IndicatorReadoutSpec {
        label: "B",
        source: IndicatorOverlaySource::Field(VOLUME_BUY_FIELD_KEY),
        color_role: IndicatorOverlayColorRole::Success,
        color_slot: IndicatorColorSlot::Positive,
        scope: IndicatorReadoutScope::Mark(MarkKind::Bar(BarMode::Histogram(
            HistogramMode::SignedOverlay,
        ))),
    },
    IndicatorReadoutSpec {
        label: "S",
        source: IndicatorOverlaySource::Field(VOLUME_SELL_FIELD_KEY),
        color_role: IndicatorOverlayColorRole::Danger,
        color_slot: IndicatorColorSlot::Negative,
        scope: IndicatorReadoutScope::Mark(MarkKind::Bar(BarMode::Histogram(
            HistogramMode::SignedOverlay,
        ))),
    },
];

#[derive(Debug, Clone, Copy)]
struct DirectionalVolume {
    buy_qty: Qty,
    sell_qty: Qty,
}

#[derive(Debug, Clone, Copy, Default)]
struct VolumePoint {
    directional: Option<DirectionalVolume>,
}

#[derive(Debug, Clone, Default)]
pub struct VolumeState {
    kline_points: BTreeMap<UnixMs, VolumePoint>,
    trade_points: BTreeMap<UnixMs, DirectionalVolume>,
}

impl VolumeState {
    fn clear_state(&mut self) {
        self.kline_points.clear();
        self.trade_points.clear();
    }

    fn replace_kline_points(&mut self, bars: &[Kline]) {
        self.kline_points.clear();
        for bar in bars {
            let directional = bar
                .volume
                .buy_sell()
                .map(|(buy_qty, sell_qty)| DirectionalVolume { buy_qty, sell_qty });
            self.kline_points
                .insert(bar.time, VolumePoint { directional });
        }
    }

    fn replace_kline_points_from(&mut self, bars: &[Kline], first_index: usize) {
        if first_index == 0 {
            return self.replace_kline_points(bars);
        }
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.kline_points.retain(|time, _| *time < first_bar.time);
        for bar in bars.iter().skip(first_index) {
            let directional = bar
                .volume
                .buy_sell()
                .map(|(buy_qty, sell_qty)| DirectionalVolume { buy_qty, sell_qty });
            self.kline_points
                .insert(bar.time, VolumePoint { directional });
        }
    }

    fn replace_trade_points(&mut self, bars: &[NormalizedTradeBar]) {
        self.trade_points.clear();
        for bar in bars {
            if let Some(values) = bar.values {
                self.trade_points.insert(
                    bar.time,
                    DirectionalVolume {
                        buy_qty: values.buy_qty,
                        sell_qty: values.sell_qty,
                    },
                );
            }
        }
    }

    fn replace_trade_points_from(&mut self, bars: &[NormalizedTradeBar], first_index: usize) {
        if first_index == 0 {
            return self.replace_trade_points(bars);
        }
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.trade_points.retain(|time, _| *time < first_bar.time);
        for bar in bars.iter().skip(first_index) {
            if let Some(values) = bar.values {
                self.trade_points.insert(
                    bar.time,
                    DirectionalVolume {
                        buy_qty: values.buy_qty,
                        sell_qty: values.sell_qty,
                    },
                );
            } else {
                self.trade_points.remove(&bar.time);
            }
        }
    }

    fn effective_directional_volume(&self, time: UnixMs) -> Option<DirectionalVolume> {
        self.kline_points
            .get(&time)
            .and_then(|point| point.directional)
            .or_else(|| self.trade_points.get(&time).copied())
    }
}

impl IndicatorDefinition for VolumeState {
    type Config = ();

    fn warmup_bars(_config: Self::Config) -> Option<u64> {
        None
    }

    fn availability(
        _context: super::AvailabilityContext,
        _snapshot: super::IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for VolumeState {
    fn clear(&mut self) {
        VolumeState::clear_state(self);
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], _config: Self::Config) {
        self.replace_kline_points(bars);
    }

    fn on_input(&mut self, input: IndicatorInput<'_>, _config: Self::Config) {
        match input {
            IndicatorInput::Klines { bars, .. } => self.replace_kline_points(bars),
            IndicatorInput::Trades { bars } => self.replace_trade_points(bars),
            IndicatorInput::OpenInterest { .. } => {}
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        _config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        match input {
            IndicatorInput::Klines { bars, .. } => {
                self.replace_kline_points_from(bars, scope.first_index())
            }
            IndicatorInput::Trades { bars } => {
                self.replace_trade_points_from(bars, scope.first_index())
            }
            IndicatorInput::OpenInterest { .. } => {}
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        let mut data = IndicatorData::scalar(bar.volume.total().to_f32_lossy());
        if let Some(directional) = self.effective_directional_volume(bar.time) {
            data = data
                .with_field(VOLUME_BUY_FIELD_KEY, directional.buy_qty.to_f32_lossy())
                .with_field(VOLUME_SELL_FIELD_KEY, directional.sell_qty.to_f32_lossy())
                .with_signed_overlay((directional.buy_qty - directional.sell_qty).to_f32_lossy());
        }
        Some(data)
    }
}
