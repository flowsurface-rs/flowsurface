use super::super::values::IndicatorData;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorMarkSpec};
use crate::chart::kline_v2::composition::{BarMode, HistogramMode, MarkKind};
use exchange::Kline;

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

#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeState;

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
    fn clear(&mut self) {}

    fn rebuild_from_bars(&mut self, _bars: &[Kline], _config: Self::Config) {}

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        let mut data = IndicatorData::scalar(bar.volume.total().to_f32_lossy());
        if let Some((buy, sell)) = bar.volume.buy_sell() {
            data = data.with_signed_overlay((buy - sell).to_f32_lossy());
        }
        Some(data)
    }
}
