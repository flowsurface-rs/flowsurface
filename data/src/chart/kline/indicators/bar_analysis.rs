use super::super::IndicatorUpdateScope;
use super::super::values::{
    BAR_ANALYSIS_BUY_FIELD_KEY, BAR_ANALYSIS_DELTA_PCT_FIELD_KEY, BAR_ANALYSIS_SELL_FIELD_KEY,
    BAR_ANALYSIS_TOTAL_FIELD_KEY, IndicatorData,
};
use super::input::IndicatorInput;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorRuntimeSnapshot,
    TableColumnColor, TableColumnFormat, TableColumnSpec, TableColumnValue,
};
use exchange::Kline;
use exchange::UnixMs;
use std::collections::BTreeMap;

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 3] = [
    IndicatorColorSpec {
        label: "Positive values",
        slot: IndicatorColorSlot::Positive,
    },
    IndicatorColorSpec {
        label: "Negative values",
        slot: IndicatorColorSlot::Negative,
    },
    IndicatorColorSpec {
        label: "Neutral values",
        slot: IndicatorColorSlot::Main,
    },
];

pub(super) const TABLE_COLUMNS: &[TableColumnSpec] = &[
    TableColumnSpec {
        label: "Buy",
        value: TableColumnValue::Field(BAR_ANALYSIS_BUY_FIELD_KEY),
        format: TableColumnFormat::Quantity,
        color: TableColumnColor::Positive,
    },
    TableColumnSpec {
        label: "Sell",
        value: TableColumnValue::Field(BAR_ANALYSIS_SELL_FIELD_KEY),
        format: TableColumnFormat::Quantity,
        color: TableColumnColor::Negative,
    },
    TableColumnSpec {
        label: "Total",
        value: TableColumnValue::Field(BAR_ANALYSIS_TOTAL_FIELD_KEY),
        format: TableColumnFormat::Quantity,
        color: TableColumnColor::Neutral,
    },
    TableColumnSpec {
        label: "Delta",
        value: TableColumnValue::Primary,
        format: TableColumnFormat::Quantity,
        color: TableColumnColor::Signed,
    },
    TableColumnSpec {
        label: "Delta%",
        value: TableColumnValue::Field(BAR_ANALYSIS_DELTA_PCT_FIELD_KEY),
        format: TableColumnFormat::Percent,
        color: TableColumnColor::Signed,
    },
];

#[derive(Debug, Clone, Default)]
pub struct BarAnalysisState {
    values: BTreeMap<UnixMs, IndicatorData>,
}

impl IndicatorDefinition for BarAnalysisState {
    type Config = ();

    fn warmup_bars(_config: Self::Config) -> Option<u64> {
        None
    }

    fn availability(
        _context: super::AvailabilityContext,
        _snapshot: IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for BarAnalysisState {
    fn clear(&mut self) {
        self.values.clear();
    }

    fn rebuild_from_bars(&mut self, _bars: &[Kline], _config: Self::Config) {}

    fn on_input(&mut self, input: IndicatorInput<'_>, _config: Self::Config) {
        let IndicatorInput::Trades { bars } = input else {
            return;
        };

        self.values.clear();
        for bar in bars {
            let Some(values) = bar.values else {
                continue;
            };

            let buy = values.buy_qty.to_f32_lossy();
            let sell = values.sell_qty.to_f32_lossy();
            let total = buy + sell;
            let delta = buy - sell;
            let delta_pct = if total.abs() > f32::EPSILON {
                delta / total * 100.0
            } else {
                0.0
            };

            let data = IndicatorData::scalar(delta)
                .with_field(BAR_ANALYSIS_BUY_FIELD_KEY, buy)
                .with_field(BAR_ANALYSIS_SELL_FIELD_KEY, sell)
                .with_field(BAR_ANALYSIS_TOTAL_FIELD_KEY, total)
                .with_field(BAR_ANALYSIS_DELTA_PCT_FIELD_KEY, delta_pct);
            self.values.insert(bar.time, data);
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        _config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        let IndicatorInput::Trades { bars } = input else {
            return;
        };

        if scope.is_full() {
            return self.on_input(input, ());
        }

        let first_index = scope.first_index();
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.values.retain(|time, _| *time < first_bar.time);

        for bar in bars.iter().skip(first_index) {
            let Some(values) = bar.values else {
                self.values.remove(&bar.time);
                continue;
            };

            let buy = values.buy_qty.to_f32_lossy();
            let sell = values.sell_qty.to_f32_lossy();
            let total = buy + sell;
            let delta = buy - sell;
            let delta_pct = if total.abs() > f32::EPSILON {
                delta / total * 100.0
            } else {
                0.0
            };

            self.values.insert(
                bar.time,
                IndicatorData::scalar(delta)
                    .with_field(BAR_ANALYSIS_BUY_FIELD_KEY, buy)
                    .with_field(BAR_ANALYSIS_SELL_FIELD_KEY, sell)
                    .with_field(BAR_ANALYSIS_TOTAL_FIELD_KEY, total)
                    .with_field(BAR_ANALYSIS_DELTA_PCT_FIELD_KEY, delta_pct),
            );
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.values.get(&bar.time).copied()
    }
}
