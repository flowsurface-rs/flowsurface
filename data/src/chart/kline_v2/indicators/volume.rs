use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::{IndicatorInput, NormalizedTradeBar};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorMarkSpec};
use crate::chart::kline_v2::composition::{BarMode, HistogramMode, MarkKind};
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

#[derive(Debug, Clone, Copy, Default)]
struct VolumePoint {
    signed_overlay: Option<Qty>,
}

#[derive(Debug, Clone, Default)]
pub struct VolumeState {
    kline_points: BTreeMap<UnixMs, VolumePoint>,
    trade_deltas: BTreeMap<UnixMs, Qty>,
}

impl VolumeState {
    fn clear_state(&mut self) {
        self.kline_points.clear();
        self.trade_deltas.clear();
    }

    fn replace_kline_points(&mut self, bars: &[Kline]) {
        self.kline_points.clear();
        for bar in bars {
            let delta = bar.volume.buy_sell().map(|(buy, sell)| buy - sell);
            self.kline_points.insert(
                bar.time,
                VolumePoint {
                    signed_overlay: delta,
                },
            );
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
            let delta = bar.volume.buy_sell().map(|(buy, sell)| buy - sell);
            self.kline_points.insert(
                bar.time,
                VolumePoint {
                    signed_overlay: delta,
                },
            );
        }
    }

    fn replace_trade_deltas(&mut self, bars: &[NormalizedTradeBar]) {
        self.trade_deltas.clear();
        for bar in bars {
            if let Some(values) = bar.values {
                self.trade_deltas
                    .insert(bar.time, values.buy_qty - values.sell_qty);
            }
        }
    }

    fn replace_trade_deltas_from(&mut self, bars: &[NormalizedTradeBar], first_index: usize) {
        if first_index == 0 {
            return self.replace_trade_deltas(bars);
        }
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.trade_deltas.retain(|time, _| *time < first_bar.time);
        for bar in bars.iter().skip(first_index) {
            if let Some(values) = bar.values {
                self.trade_deltas
                    .insert(bar.time, values.buy_qty - values.sell_qty);
            } else {
                self.trade_deltas.remove(&bar.time);
            }
        }
    }

    fn effective_overlay(&self, time: UnixMs) -> Option<Qty> {
        self.kline_points
            .get(&time)
            .and_then(|point| point.signed_overlay)
            .or_else(|| self.trade_deltas.get(&time).copied())
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
            IndicatorInput::Trades { bars } => self.replace_trade_deltas(bars),
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
                self.replace_trade_deltas_from(bars, scope.first_index())
            }
            IndicatorInput::OpenInterest { .. } => {}
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        let mut data = IndicatorData::scalar(bar.volume.total().to_f32_lossy());
        if let Some(delta) = self.effective_overlay(bar.time) {
            data = data.with_signed_overlay(delta.to_f32_lossy());
        }
        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::kline_v2::indicators::input::TradeBarValues;
    use exchange::unit::{Price, Qty};

    fn kline(time: u64, volume: exchange::Volume) -> Kline {
        Kline {
            time: time.into(),
            open: Price::from_f64(100.0),
            high: Price::from_f64(101.0),
            low: Price::from_f64(99.0),
            close: Price::from_f64(100.0),
            volume,
        }
    }

    #[test]
    fn kline_directional_overlay_available() {
        let mut state = VolumeState::default();
        state.rebuild_from_bars(
            &[
                kline(
                    1,
                    exchange::Volume::BuySell(Qty::from_f64(6.0), Qty::from_f64(4.0)),
                ),
                kline(
                    2,
                    exchange::Volume::BuySell(Qty::from_f64(3.0), Qty::from_f64(7.0)),
                ),
            ],
            (),
        );

        let buy_heavy = kline(
            1,
            exchange::Volume::BuySell(Qty::from_f64(6.0), Qty::from_f64(4.0)),
        );
        let overlay = state
            .data_for_bar(&buy_heavy)
            .and_then(IndicatorData::signed_overlay);
        assert_eq!(overlay, Some(2.0));
    }

    #[test]
    fn kline_without_directional_data_has_no_overlay() {
        let mut state = VolumeState::default();
        state.rebuild_from_bars(
            &[
                kline(1, exchange::Volume::TotalOnly(Qty::from_f64(10.0))),
                kline(2, exchange::Volume::TotalOnly(Qty::from_f64(10.0))),
            ],
            (),
        );

        let bar = kline(1, exchange::Volume::TotalOnly(Qty::from_f64(10.0)));
        assert!(
            state
                .data_for_bar(&bar)
                .and_then(IndicatorData::signed_overlay)
                .is_none()
        );
        // Total volume is still reported.
        assert_eq!(
            state.data_for_bar(&bar).map(IndicatorData::value),
            Some(10.0)
        );
    }

    #[test]
    fn trades_fallback_provides_overlay() {
        let mut state = VolumeState::default();
        // Klines carry only total volume...
        state.rebuild_from_bars(
            &[
                kline(1, exchange::Volume::TotalOnly(Qty::from_f64(10.0))),
                kline(2, exchange::Volume::TotalOnly(Qty::from_f64(10.0))),
            ],
            (),
        );
        // ...then directional volume arrives via trades.
        state.on_input(
            IndicatorInput::Trades {
                bars: &[
                    NormalizedTradeBar {
                        time: 1u64.into(),
                        values: Some(TradeBarValues {
                            trade_count: 2,
                            buy_qty: Qty::from_f64(8.0),
                            sell_qty: Qty::from_f64(2.0),
                        }),
                    },
                    NormalizedTradeBar {
                        time: 2u64.into(),
                        values: Some(TradeBarValues {
                            trade_count: 2,
                            buy_qty: Qty::from_f64(1.0),
                            sell_qty: Qty::from_f64(9.0),
                        }),
                    },
                ],
            },
            (),
        );

        let bar = kline(1, exchange::Volume::TotalOnly(Qty::from_f64(10.0)));
        assert_eq!(
            state
                .data_for_bar(&bar)
                .and_then(IndicatorData::signed_overlay),
            Some(6.0)
        );
    }

    #[test]
    fn kline_overlay_takes_precedence_over_trades() {
        let mut state = VolumeState::default();
        state.rebuild_from_bars(
            &[kline(
                1,
                exchange::Volume::BuySell(Qty::from_f64(5.0), Qty::from_f64(5.0)),
            )],
            (),
        );
        state.on_input(
            IndicatorInput::Trades {
                bars: &[NormalizedTradeBar {
                    time: 1u64.into(),
                    values: Some(TradeBarValues {
                        trade_count: 1,
                        buy_qty: Qty::from_f64(9.0),
                        sell_qty: Qty::from_f64(1.0),
                    }),
                }],
            },
            (),
        );

        let bar = kline(
            1,
            exchange::Volume::BuySell(Qty::from_f64(5.0), Qty::from_f64(5.0)),
        );
        // Kline delta is 0, trade delta is 8; kline takes precedence (0).
        assert_eq!(
            state
                .data_for_bar(&bar)
                .and_then(IndicatorData::signed_overlay),
            Some(0.0)
        );
    }
}
