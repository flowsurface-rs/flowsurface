use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::{IndicatorInput, NormalizedTradeBar};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorMarkSpec,
    IndicatorRuntimeSnapshot, IndicatorUnsupportedReason,
};
use crate::chart::Basis;
use crate::chart::kline::composition::{BarMode, HistogramMode, MarkKind};
use exchange::{Kline, UnixMs};
use std::collections::BTreeMap;

const LINE_COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Main];
const BAR_COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Bars];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 2] = [
    IndicatorColorSpec {
        label: "Line",
        slot: IndicatorColorSlot::Main,
    },
    IndicatorColorSpec {
        label: "Bars",
        slot: IndicatorColorSlot::Bars,
    },
];

pub(super) const MARK_SPECS: [IndicatorMarkSpec; 2] = [
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
];

#[derive(Debug, Clone, Default)]
pub struct TradeCountState {
    values: BTreeMap<UnixMs, u64>,
}

impl TradeCountState {
    fn recompute_from(&mut self, bars: &[NormalizedTradeBar], first_index: usize) {
        if first_index == 0 {
            self.values.clear();
        } else {
            let Some(first_bar) = bars.get(first_index) else {
                return;
            };
            self.values.retain(|time, _| *time < first_bar.time);
        }

        self.values.extend(
            bars.iter()
                .skip(first_index)
                .filter_map(|bar| bar.values.map(|values| (bar.time, values.trade_count))),
        );
    }
}

impl IndicatorDefinition for TradeCountState {
    type Config = ();

    fn warmup_bars(_: Self::Config) -> Option<u64> {
        None
    }

    fn unsupported_reason(
        context: super::AvailabilityContext,
    ) -> Option<IndicatorUnsupportedReason> {
        matches!(context.basis, Basis::Tick(_))
            .then_some(IndicatorUnsupportedReason::BasisNotSupported)
    }

    fn availability(
        _: super::AvailabilityContext,
        _: IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for TradeCountState {
    fn clear(&mut self) {
        self.values.clear();
    }

    fn rebuild_from_bars(&mut self, _: &[Kline], _: Self::Config) {}

    fn on_input(&mut self, input: IndicatorInput<'_>, _: Self::Config) {
        if let IndicatorInput::Trades { bars } = input {
            self.recompute_from(bars, 0);
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        _: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        if let IndicatorInput::Trades { bars } = input {
            self.recompute_from(bars, scope.first_index());
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.values
            .get(&bar.time)
            .map(|value| IndicatorData::scalar(*value as f32))
    }
}
