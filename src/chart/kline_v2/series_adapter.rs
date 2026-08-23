use crate::widget::chart::kline::composition::PanelValueId;
use crate::widget::chart::kline::{IndicatorData, KlineSeriesLike};
use data::chart::kline::KlineTrades;
use data::chart::kline_v2::series::{
    AnchoredPriceLevelTradeDistribution, FootprintPocLine, PriceLevelTradeDistribution,
};
use data::chart::kline_v2::{LineStrokeStyle, SeriesColor};
use exchange::{
    Kline, TickerInfo, Timeframe, UnixMs,
    unit::{Price, PriceStep},
};

pub use data::chart::kline_v2::series::KlineSeries;

impl KlineSeriesLike for KlineSeries {
    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }

    fn series_color(&self) -> SeriesColor {
        self.color()
    }

    fn series_line_width(&self) -> u8 {
        self.line_width()
    }

    fn series_stroke_style(&self) -> LineStrokeStyle {
        self.stroke_style()
    }

    fn bars(&self) -> &[Kline] {
        &self.bars
    }

    fn time_at_index(&self, index: usize) -> Option<UnixMs> {
        KlineSeries::time_at_index(self, index)
    }

    fn indicator_value(&self, bar: &Kline) -> f32 {
        bar.volume.total().to_f32_lossy()
    }

    fn indicator_data_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        KlineSeries::indicator_data_for_panel_value(self, panel_value, bar)
    }

    fn footprint_for_bar(&self, bar: &Kline) -> Option<&KlineTrades> {
        KlineSeries::footprint_for_bar(self, bar)
    }

    fn trade_price_distribution(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        lowest: Price,
        highest: Price,
        tick_size: PriceStep,
    ) -> Option<PriceLevelTradeDistribution> {
        KlineSeries::trade_price_distribution(self, earliest, latest, lowest, highest, tick_size)
    }

    fn recurring_trade_price_distributions(
        &self,
        interval: Timeframe,
        tick_size: PriceStep,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredPriceLevelTradeDistribution> {
        KlineSeries::recurring_trade_price_distributions(
            self, interval, tick_size, earliest, latest,
        )
    }

    fn footprint_poc_lines(&self, earliest: UnixMs, latest: UnixMs) -> Vec<FootprintPocLine> {
        KlineSeries::footprint_poc_lines(self, earliest, latest)
    }

    fn footprint_tick_index_offset(&self) -> u64 {
        KlineSeries::footprint_tick_index_offset(self)
    }

    fn footprint_tick_size(&self) -> PriceStep {
        self.footprint_tick_size
    }
}
