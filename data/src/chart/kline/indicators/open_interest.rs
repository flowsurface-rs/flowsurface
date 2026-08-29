use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::IndicatorInput;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorMarkSpec,
    IndicatorRuntimeSnapshot, IndicatorUnsupportedReason,
};
use crate::chart::Basis;
use crate::chart::kline::composition::{BarMode, MarkKind};
use exchange::adapter::Exchange;
use exchange::{Kline, OpenInterest, Timeframe, UnixMs};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;

const COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Main];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 1] = [IndicatorColorSpec {
    label: "Line",
    slot: IndicatorColorSlot::Main,
}];

pub(super) const MARK_SPECS: [IndicatorMarkSpec; 2] = [
    IndicatorMarkSpec {
        label: "Line",
        mark: MarkKind::Line,
        color_slots: &COLOR_SLOTS,
    },
    IndicatorMarkSpec {
        label: "Bars",
        mark: MarkKind::Bar(BarMode::Regular),
        color_slots: &COLOR_SLOTS,
    },
];

#[derive(Debug, Clone, Default)]
pub struct OpenInterestState {
    values: BTreeMap<UnixMs, f32>,
}

impl OpenInterestState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn insert_batch(&mut self, data: &[OpenInterest], basis: Basis) {
        for oi in data {
            let time = Self::align_time(oi.time, basis);
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

    fn align_time(time: UnixMs, basis: Basis) -> UnixMs {
        match basis {
            Basis::Time(timeframe) => time.floor_to(timeframe),
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

impl IndicatorDefinition for OpenInterestState {
    type Config = ();

    fn warmup_bars(_config: Self::Config) -> Option<u64> {
        None
    }

    fn unsupported_reason(
        context: super::AvailabilityContext,
    ) -> Option<IndicatorUnsupportedReason> {
        let Basis::Time(timeframe) = context.basis else {
            return Some(IndicatorUnsupportedReason::BasisNotSupported);
        };

        if !is_supported_exchange(context.base_ticker.exchange()) {
            return Some(IndicatorUnsupportedReason::SourceNotSupported);
        }

        if !is_supported_timeframe(timeframe) {
            return Some(IndicatorUnsupportedReason::ResolutionNotSupported);
        }

        None
    }

    fn availability(
        _context: super::AvailabilityContext,
        _snapshot: IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for OpenInterestState {
    fn clear(&mut self) {
        OpenInterestState::clear(self);
    }

    fn rebuild_from_bars(&mut self, _bars: &[Kline], _config: Self::Config) {}

    fn on_input(&mut self, input: IndicatorInput<'_>, _config: Self::Config) {
        if let IndicatorInput::OpenInterest { data, basis } = input {
            self.insert_batch(data, basis);
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

    fn snapshot(&self) -> IndicatorRuntimeSnapshot {
        IndicatorRuntimeSnapshot {
            value_range: self.timerange(),
            ..IndicatorRuntimeSnapshot::default()
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.value_at(bar.time).map(IndicatorData::scalar)
    }
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
