use crate::widget::chart::kline::{IndicatorData, KlineSeriesLike};
use data::chart::kline::composition::PanelValueId;
use data::chart::kline::series::{
    AnchoredPriceLevelTradeDistribution, FootprintPocLine, PriceLevelTradeDistribution,
};
use data::chart::kline::{LineStrokeStyle, SeriesColor};
use data::chart::kline_data::KlineTrades;
use exchange::{
    Kline, TickerInfo, Timeframe, UnixMs,
    unit::{Price, PriceStep},
};

pub use data::chart::kline::series::KlineSeries;

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

    fn indicator_data_for_panel_value_at(
        &self,
        panel_value: Option<PanelValueId>,
        index: usize,
    ) -> Option<IndicatorData> {
        KlineSeries::indicator_data_for_panel_value_at(self, panel_value, index)
    }

    fn indicator_produces_signed_overlay(&self, value_id: PanelValueId) -> bool {
        KlineSeries::indicator_produces_signed_overlay(self, value_id)
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
