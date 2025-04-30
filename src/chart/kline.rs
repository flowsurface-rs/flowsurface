use data::UserTimezone;
use data::aggr::ticks::TickAggr;
use data::aggr::time::TimeSeries;
use data::chart::indicators::{Indicator, KlineIndicator};
use data::chart::kline::{ClusterKind, FootprintStudy, KlineTrades, NPoc};
use data::chart::{ChartLayout, KlineChartKind};
use exchange::fetcher::{FetchRange, RequestHandler};
use exchange::{Kline, OpenInterest as OIData, TickerInfo, Timeframe, Trade};

use super::scale::PriceInfoLabel;
use super::study::ChartStudy;
use super::{
    Action, Basis, Caches, Chart, ChartConstants, ChartData, CommonChartData, Interaction, Message,
    indicator,
};
use super::{
    abbr_large_numbers, calc_splits, canvas_interaction, count_decimals,
    draw_horizontal_volume_bars, request_fetch, round_to_tick, update_chart, view_chart,
};

use crate::style;

use iced::task::Handle;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Event, Geometry, Path, Stroke};
use iced::{Alignment, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};
use ordered_float::OrderedFloat;

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

impl Chart for KlineChart {
    fn common_data(&self) -> &CommonChartData {
        &self.chart
    }

    fn common_data_mut(&mut self) -> &mut CommonChartData {
        &mut self.chart
    }

    fn update_chart(&mut self, message: &Message) {
        update_chart(self, message);
        self.render_start();
    }

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn view_indicators<I: Indicator>(&self, indicators: &[I]) -> Vec<Element<Message>> {
        self.view_indicators(indicators)
    }

    fn visible_timerange(&self) -> (u64, u64) {
        let chart = self.common_data();
        let region = chart.visible_region(chart.bounds.size());

        match &chart.basis {
            Basis::Time(interval) => {
                let (earliest, latest) = (
                    chart.x_to_interval(region.x) - (interval / 2),
                    chart.x_to_interval(region.x + region.width) + (interval / 2),
                );

                (earliest, latest)
            }
            Basis::Tick(_) => {
                unimplemented!()
            }
        }
    }

    fn interval_keys(&self) -> Vec<u64> {
        match &self.data_source {
            ChartData::TimeBased(_) => {
                //timeseries.data_points.keys().cloned().collect()
                vec![]
            }
            ChartData::TickBased(tick_aggr) => tick_aggr
                .data_points
                .iter()
                .map(|dp| dp.kline.time)
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        match &self.data_source {
            ChartData::TimeBased(timeseries) => timeseries.data_points.is_empty(),
            ChartData::TickBased(tick_aggr) => tick_aggr.data_points.is_empty(),
        }
    }
}

#[allow(dead_code)]
enum IndicatorData {
    Volume(Caches, BTreeMap<u64, (f32, f32)>),
    OpenInterest(Caches, BTreeMap<u64, f32>),
}

impl IndicatorData {
    fn clear_cache(&mut self) {
        match self {
            IndicatorData::Volume(caches, _) | IndicatorData::OpenInterest(caches, _) => {
                caches.clear_all();
            }
        }
    }

    fn create_indicator_elem<'a>(
        &'a self,
        chart: &'a CommonChartData,
        earliest: u64,
        latest: u64,
    ) -> Element<'a, Message> {
        match self {
            IndicatorData::Volume(cache, data) => {
                indicator::volume::create_indicator_elem(chart, cache, data, earliest, latest)
            }
            IndicatorData::OpenInterest(cache, data) => {
                indicator::open_interest::create_indicator_elem(
                    chart, cache, data, earliest, latest,
                )
            }
        }
    }
}

impl ChartConstants for KlineChart {
    fn min_scaling(&self) -> f32 {
        self.kind.min_scaling()
    }

    fn max_scaling(&self) -> f32 {
        self.kind.max_scaling()
    }

    fn max_cell_width(&self) -> f32 {
        self.kind.max_cell_width()
    }

    fn min_cell_width(&self) -> f32 {
        self.kind.min_cell_width()
    }

    fn max_cell_height(&self) -> f32 {
        self.kind.max_cell_height()
    }

    fn min_cell_height(&self) -> f32 {
        self.kind.min_cell_height()
    }

    fn default_cell_width(&self) -> f32 {
        self.kind.default_cell_width()
    }
}

pub struct KlineChart {
    chart: CommonChartData,
    data_source: ChartData,
    raw_trades: Vec<Trade>,
    indicators: HashMap<KlineIndicator, IndicatorData>,
    fetching_trades: (bool, Option<Handle>),
    kind: KlineChartKind,
    request_handler: RequestHandler,
    study_configurator: ChartStudy,
}

impl KlineChart {
    pub fn new(
        layout: ChartLayout,
        basis: Basis,
        tick_size: f32,
        klines_raw: &[Kline],
        raw_trades: Vec<Trade>,
        enabled_indicators: &[KlineIndicator],
        ticker_info: Option<TickerInfo>,
        kind: &KlineChartKind,
    ) -> Self {
        match basis {
            Basis::Time(interval) => {
                let timeseries =
                    TimeSeries::new(interval.into(), tick_size, &raw_trades, klines_raw);

                let base_price_y = timeseries.base_price();
                let latest_x = timeseries.latest_timestamp().unwrap_or(0);
                let (scale_high, scale_low) = timeseries.price_scale({
                    match kind {
                        KlineChartKind::Footprint { .. } => 12,
                        KlineChartKind::Candles => 60,
                    }
                });

                let y_ticks = (scale_high - scale_low) / tick_size;

                let enabled_indicators = enabled_indicators
                    .iter()
                    .map(|indicator| {
                        (
                            *indicator,
                            match indicator {
                                KlineIndicator::Volume => IndicatorData::Volume(
                                    Caches::default(),
                                    timeseries.volume_data(),
                                ),
                                KlineIndicator::OpenInterest => {
                                    IndicatorData::OpenInterest(Caches::default(), BTreeMap::new())
                                }
                            },
                        )
                    })
                    .collect();

                KlineChart {
                    chart: CommonChartData {
                        cell_width: match kind {
                            KlineChartKind::Footprint { .. } => 80.0,
                            KlineChartKind::Candles => 4.0,
                        },
                        cell_height: match kind {
                            KlineChartKind::Footprint { .. } => 800.0 / y_ticks,
                            KlineChartKind::Candles => 200.0 / y_ticks,
                        },
                        base_price_y,
                        latest_x,
                        tick_size,
                        decimals: count_decimals(tick_size),
                        crosshair: layout.crosshair,
                        splits: layout.splits,
                        ticker_info,
                        basis,
                        ..Default::default()
                    },
                    data_source: ChartData::TimeBased(timeseries),
                    raw_trades,
                    indicators: enabled_indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::new(),
                    kind: kind.clone(),
                    study_configurator: ChartStudy::new(),
                }
            }
            Basis::Tick(interval) => {
                let tick_aggr = TickAggr::new(interval, tick_size, &raw_trades);

                let enabled_indicators = enabled_indicators
                    .iter()
                    .map(|indicator| {
                        (
                            *indicator,
                            match indicator {
                                KlineIndicator::Volume => IndicatorData::Volume(
                                    Caches::default(),
                                    tick_aggr.volume_data(),
                                ),
                                KlineIndicator::OpenInterest => {
                                    IndicatorData::OpenInterest(Caches::default(), BTreeMap::new())
                                }
                            },
                        )
                    })
                    .collect();

                KlineChart {
                    chart: CommonChartData {
                        cell_width: match kind {
                            KlineChartKind::Footprint { .. } => 80.0,
                            KlineChartKind::Candles => 4.0,
                        },
                        cell_height: match kind {
                            KlineChartKind::Footprint { .. } => 90.0,
                            KlineChartKind::Candles => 8.0,
                        },
                        tick_size,
                        decimals: count_decimals(tick_size),
                        crosshair: layout.crosshair,
                        splits: layout.splits,
                        ticker_info,
                        basis,
                        ..Default::default()
                    },
                    data_source: ChartData::TickBased(TickAggr::new(
                        interval,
                        tick_size,
                        &raw_trades,
                    )),
                    raw_trades,
                    indicators: enabled_indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::new(),
                    kind: kind.clone(),
                    study_configurator: ChartStudy::new(),
                }
            }
        }
    }

    pub fn update_latest_kline(&mut self, kline: &Kline) -> Action {
        match self.data_source {
            ChartData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(&[kline.to_owned()]);

                if let Some(IndicatorData::Volume(_, data)) =
                    self.indicators.get_mut(&KlineIndicator::Volume)
                {
                    data.insert(kline.time, (kline.volume.0, kline.volume.1));
                };

                let chart = self.common_data_mut();

                if (kline.time) > chart.latest_x {
                    chart.latest_x = kline.time;
                }

                chart.last_price = Some(PriceInfoLabel::new(kline.close, kline.open));

                self.render_start();
                return self.missing_data_task();
            }
            ChartData::TickBased(_) => {
                self.render_start();
            }
        }

        Action::None
    }

    pub fn kind(&self) -> &KlineChartKind {
        &self.kind
    }

    fn missing_data_task(&mut self) -> Action {
        match &self.data_source {
            ChartData::TimeBased(timeseries) => {
                let timeframe = timeseries.interval.to_milliseconds();

                let (visible_earliest, visible_latest) = self.visible_timerange();
                let (kline_earliest, kline_latest) = timeseries.kline_timerange();
                let earliest = visible_earliest - (visible_latest - visible_earliest);

                // priority 1, basic kline data fetch
                if visible_earliest < kline_earliest {
                    let range = FetchRange::Kline(earliest, kline_earliest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return action;
                    }
                }

                if !self.fetching_trades.0 {
                    if let Some(earliest_gap) = timeseries
                        .data_points
                        .range(visible_earliest..=visible_latest)
                        .filter(|(_, dp)| dp.footprint.trades.is_empty())
                        .map(|(time, _)| *time)
                        .min()
                    {
                        let last_kline_before_gap = timeseries
                            .data_points
                            .range(..earliest_gap)
                            .filter(|(_, dp)| !dp.footprint.trades.is_empty())
                            .max_by_key(|(time, _)| *time)
                            .map_or(earliest_gap, |(time, _)| *time);

                        let first_kline_after_gap = timeseries
                            .data_points
                            .range(earliest_gap..)
                            .filter(|(_, dp)| !dp.footprint.trades.is_empty())
                            .min_by_key(|(time, _)| *time)
                            .map_or(kline_latest, |(time, _)| *time - 1);

                        let range = FetchRange::Trades(
                            last_kline_before_gap.max(visible_earliest),
                            first_kline_after_gap.min(visible_latest),
                        );

                        if let Some(action) = request_fetch(&mut self.request_handler, range) {
                            self.fetching_trades = (true, None);
                            return action;
                        }
                    }
                }

                // priority 2, Open Interest data
                for data in self.indicators.values() {
                    if let IndicatorData::OpenInterest(_, _) = data {
                        if timeframe >= Timeframe::M5.to_milliseconds()
                            && self.chart.ticker_info.is_some_and(|t| t.is_perps())
                        {
                            let (oi_earliest, oi_latest) = self.oi_timerange(kline_latest);

                            if visible_earliest < oi_earliest {
                                let range = FetchRange::OpenInterest(earliest, oi_earliest);

                                if let Some(action) =
                                    request_fetch(&mut self.request_handler, range)
                                {
                                    return action;
                                }
                            }

                            if oi_latest < kline_latest {
                                let range =
                                    FetchRange::OpenInterest(oi_latest.max(earliest), kline_latest);

                                if let Some(action) =
                                    request_fetch(&mut self.request_handler, range)
                                {
                                    return action;
                                }
                            }
                        }
                    }
                }

                // priority 3, missing klines & integrity check
                if let Some(missing_keys) =
                    timeseries.check_integrity(kline_earliest, kline_latest, timeframe)
                {
                    let latest = missing_keys.iter().max().unwrap_or(&visible_latest) + timeframe;
                    let earliest =
                        missing_keys.iter().min().unwrap_or(&visible_earliest) - timeframe;

                    let range = FetchRange::Kline(earliest, latest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return action;
                    }
                }
            }
            ChartData::TickBased(_) => {
                // TODO: implement trade fetch
            }
        }

        Action::None
    }

    pub fn reset_request_handler(&mut self) {
        self.request_handler = RequestHandler::new();
        self.fetching_trades = (false, None);
    }

    pub fn raw_trades(&self) -> Vec<Trade> {
        self.raw_trades.clone()
    }

    pub fn clear_trades(&mut self, clear_raw: bool) {
        match self.data_source {
            ChartData::TimeBased(ref mut source) => {
                source.clear_trades();

                if clear_raw {
                    self.raw_trades.clear();
                } else {
                    source.insert_trades(&self.raw_trades, None);
                }
            }
            ChartData::TickBased(_) => {
                // TODO: implement
            }
        }
    }

    pub fn set_handle(&mut self, handle: Handle) {
        self.fetching_trades.1 = Some(handle);
    }

    pub fn tick_size(&self) -> f32 {
        self.chart.tick_size
    }

    pub fn study_configurator(&self) -> &ChartStudy {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: super::study::Message) {
        let action = self.study_configurator.update(message);

        let studies = if let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        {
            studies
        } else {
            return;
        };

        match action {
            super::study::Action::None => return,
            super::study::Action::ToggleStudy(study, is_selected) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|s| !s.is_same_type(&study));
                }
            }
            super::study::Action::ConfigureStudy(study) => {
                if let FootprintStudy::Imbalance { threshold } = study {
                    if let Some(existing_study) =
                        studies.iter_mut().find(|s| s.is_same_type(&study))
                    {
                        *existing_study = FootprintStudy::Imbalance { threshold };
                    }
                }
            }
        }

        self.render_start();
    }

    pub fn chart_layout(&self) -> ChartLayout {
        self.chart.get_chart_layout()
    }

    pub fn set_cluster_kind(&mut self, new_kind: ClusterKind) {
        if let KlineChartKind::Footprint {
            ref mut clusters, ..
        } = self.kind
        {
            *clusters = new_kind;
        }

        self.render_start();
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart = self.common_data_mut();

        chart.cell_height *= new_tick_size / chart.tick_size;
        chart.tick_size = new_tick_size;

        match self.data_source {
            ChartData::TickBased(ref mut tick_aggr) => {
                tick_aggr.change_tick_size(new_tick_size, &self.raw_trades);
            }
            ChartData::TimeBased(ref mut timeseries) => {
                timeseries.change_tick_size(new_tick_size, &self.raw_trades);
            }
        }

        self.clear_trades(false);
        self.render_start();
    }

    pub fn set_tick_basis(&mut self, tick_basis: u64) {
        self.chart.basis = Basis::Tick(tick_basis);

        let new_tick_aggr = TickAggr::new(tick_basis, self.chart.tick_size, &self.raw_trades);

        if let Some(indicator) = self.indicators.get_mut(&KlineIndicator::Volume) {
            *indicator = IndicatorData::Volume(Caches::default(), new_tick_aggr.volume_data());
        }

        self.data_source = ChartData::TickBased(new_tick_aggr);

        self.render_start();
    }

    fn oi_timerange(&self, latest_kline: u64) -> (u64, u64) {
        let mut from_time = latest_kline;
        let mut to_time = u64::MIN;

        if let Some(IndicatorData::OpenInterest(_, data)) =
            self.indicators.get(&KlineIndicator::OpenInterest)
        {
            data.iter().for_each(|(time, _)| {
                from_time = from_time.min(*time);
                to_time = to_time.max(*time);
            });
        };

        (from_time, to_time)
    }

    pub fn insert_trades_buffer(&mut self, trades_buffer: &[Trade], depth_update: u64) {
        self.raw_trades.extend_from_slice(trades_buffer);

        match self.data_source {
            ChartData::TickBased(ref mut tick_aggr) => {
                let old_dp_len = tick_aggr.data_points.len();

                tick_aggr.insert_trades(trades_buffer);

                if let Some(IndicatorData::Volume(_, data)) =
                    self.indicators.get_mut(&KlineIndicator::Volume)
                {
                    let start_idx = old_dp_len.saturating_sub(1);
                    for (idx, dp) in tick_aggr.data_points.iter().enumerate().skip(start_idx) {
                        data.insert(idx as u64, (dp.kline.volume.0, dp.kline.volume.1));
                    }
                }

                if let Some(last_dp) = tick_aggr.data_points.last() {
                    self.chart.last_price =
                        Some(PriceInfoLabel::new(last_dp.kline.close, last_dp.kline.open));
                } else {
                    self.chart.last_price = None;
                }

                self.render_start();
            }
            ChartData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades(trades_buffer, Some(depth_update));
            }
        }
    }

    pub fn insert_raw_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        match self.data_source {
            ChartData::TickBased(ref mut tick_aggr) => {
                tick_aggr.insert_trades(&raw_trades);
            }
            ChartData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades(&raw_trades, None);
            }
        }

        self.raw_trades.extend(raw_trades);

        if is_batches_done {
            self.fetching_trades = (false, None);
        }
    }

    pub fn insert_new_klines(&mut self, req_id: uuid::Uuid, klines_raw: &[Kline]) {
        match self.data_source {
            ChartData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(klines_raw);

                if let Some(IndicatorData::Volume(_, data)) =
                    self.indicators.get_mut(&KlineIndicator::Volume)
                {
                    data.extend(
                        klines_raw
                            .iter()
                            .map(|kline| (kline.time, (kline.volume.0, kline.volume.1))),
                    );
                };

                if klines_raw.is_empty() {
                    self.request_handler
                        .mark_failed(req_id, "No data received".to_string());
                } else {
                    self.request_handler.mark_completed(req_id);
                }
            }
            ChartData::TickBased(_) => {}
        }

        self.render_start();
    }

    pub fn insert_open_interest(&mut self, req_id: Option<uuid::Uuid>, oi_data: &[OIData]) {
        if let Some(req_id) = req_id {
            if oi_data.is_empty() {
                self.request_handler
                    .mark_failed(req_id, "No data received".to_string());
            } else {
                self.request_handler.mark_completed(req_id);
            }
        }

        if let Some(IndicatorData::OpenInterest(_, data)) =
            self.indicators.get_mut(&KlineIndicator::OpenInterest)
        {
            data.extend(oi_data.iter().map(|oi| (oi.time, oi.value)));
        };
    }

    fn calc_qty_scales(
        &self,
        earliest: u64,
        latest: u64,
        highest: f32,
        lowest: f32,
        tick_size: f32,
        cluster_kind: ClusterKind,
    ) -> f32 {
        let rounded_highest = OrderedFloat(round_to_tick(highest + tick_size, tick_size));
        let rounded_lowest = OrderedFloat(round_to_tick(lowest - tick_size, tick_size));

        match &self.data_source {
            ChartData::TimeBased(timeseries) => timeseries.max_qty_ts_range(
                cluster_kind,
                earliest,
                latest,
                rounded_highest,
                rounded_lowest,
            ),
            ChartData::TickBased(tick_aggr) => {
                let earliest = earliest as usize;
                let latest = latest as usize;

                tick_aggr.max_qty_idx_range(
                    cluster_kind,
                    earliest,
                    latest,
                    rounded_highest,
                    rounded_lowest,
                )
            }
        }
    }

    fn render_start(&mut self) {
        let chart_state = &mut self.chart;

        if chart_state.autoscale {
            match &self.kind {
                KlineChartKind::Footprint { .. } => {
                    chart_state.translation = Vector::new(
                        0.5 * (chart_state.bounds.width / chart_state.scaling)
                            - (chart_state.cell_width / chart_state.scaling),
                        self.data_source
                            .get_latest_price_range_y_midpoint(chart_state),
                    );
                }
                KlineChartKind::Candles => {
                    chart_state.translation = Vector::new(
                        0.5 * (chart_state.bounds.width / chart_state.scaling)
                            - (8.0 * chart_state.cell_width / chart_state.scaling),
                        self.data_source
                            .get_latest_price_range_y_midpoint(chart_state),
                    );
                }
            }
        }

        chart_state.cache.clear_all();

        self.indicators.iter_mut().for_each(|(_, data)| {
            data.clear_cache();
        });
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) {
        match self.indicators.entry(indicator) {
            Entry::Occupied(entry) => {
                entry.remove();
            }
            Entry::Vacant(entry) => {
                let data = match indicator {
                    KlineIndicator::Volume => match &self.data_source {
                        ChartData::TimeBased(timeseries) => {
                            IndicatorData::Volume(Caches::default(), timeseries.into())
                        }
                        ChartData::TickBased(tick_aggr) => {
                            IndicatorData::Volume(Caches::default(), tick_aggr.into())
                        }
                    },
                    KlineIndicator::OpenInterest => {
                        IndicatorData::OpenInterest(Caches::default(), BTreeMap::new())
                    }
                };
                entry.insert(data);
            }
        }

        if let Some(main_split) = self.chart.splits.first() {
            let active_indicators = self
                .indicators
                .iter()
                .filter(|(_, data)| match data {
                    IndicatorData::OpenInterest(_, _) | IndicatorData::Volume(_, _) => true,
                })
                .count();

            self.chart.splits = calc_splits(*main_split, active_indicators);
        }
    }

    pub fn view_indicators<I: Indicator>(&self, enabled: &[I]) -> Vec<Element<Message>> {
        let chart_state: &CommonChartData = self.common_data();

        let visible_region = chart_state.visible_region(chart_state.bounds.size());
        let (earliest, latest) = chart_state.interval_range(&visible_region);

        let mut indicators = vec![];

        let market = match chart_state.ticker_info {
            Some(ref info) => info.get_market_type(),
            None => return indicators,
        };

        for selected_indicator in enabled {
            if I::get_available(market).contains(selected_indicator) {
                if let Some(indicator) =
                    selected_indicator.as_any().downcast_ref::<KlineIndicator>()
                {
                    match indicator {
                        KlineIndicator::Volume => {
                            if let Some(data) = self.indicators.get(&KlineIndicator::Volume) {
                                indicators.push(data.create_indicator_elem(
                                    chart_state,
                                    earliest,
                                    latest,
                                ));
                            }
                        }
                        KlineIndicator::OpenInterest => {
                            if let Some(data) = self.indicators.get(&KlineIndicator::OpenInterest) {
                                indicators.push(data.create_indicator_elem(
                                    chart_state,
                                    earliest,
                                    latest,
                                ));
                            }
                        }
                    }
                }
            }
        }

        indicators
    }

    pub fn update(&mut self, message: &Message) {
        self.update_chart(message);
    }

    pub fn view<'a, I: Indicator>(
        &'a self,
        indicators: &'a [I],
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        view_chart(self, indicators, timezone)
    }
}

impl canvas::Program<Message> for KlineChart {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        self.canvas_interaction(interaction, event, bounds, cursor)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let chart = self.common_data();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let bounds_size = bounds.size();

        let palette = theme.extended_palette();

        let klines = chart.cache.main.draw(renderer, bounds_size, |frame| {
            frame.translate(center);
            frame.scale(chart.scaling);
            frame.translate(chart.translation);

            let region = chart.visible_region(frame.size());
            let (earliest, latest) = chart.interval_range(&region);

            let price_to_y = |price: f32| chart.price_to_y(price);
            let interval_to_x = |interval: u64| chart.interval_to_x(interval);

            match &self.kind {
                KlineChartKind::Footprint { clusters, studies } => {
                    let (highest, lowest) = chart.price_range(&region);

                    let max_cluster_qty = self.calc_qty_scales(
                        earliest,
                        latest,
                        highest,
                        lowest,
                        chart.tick_size,
                        *clusters,
                    );

                    let cell_height_unscaled = chart.cell_height * chart.scaling;
                    let cell_width_unscaled = chart.cell_width * chart.scaling;

                    let text_size = {
                        let text_size_from_height = cell_height_unscaled.round().min(16.0) - 3.0;
                        let text_size_from_width =
                            (cell_width_unscaled * 0.1).round().min(16.0) - 3.0;

                        text_size_from_height.min(text_size_from_width)
                    };

                    let candle_width = 0.1 * chart.cell_width;

                    let imb_threshold = studies.iter().find_map(|study| {
                        if let FootprintStudy::Imbalance { threshold } = study {
                            Some(*threshold)
                        } else {
                            None
                        }
                    });

                    draw_all_npocs(
                        &self.data_source,
                        frame,
                        price_to_y,
                        interval_to_x,
                        candle_width,
                        chart.cell_width,
                        palette,
                        studies,
                    );

                    render_data_source(
                        &self.data_source,
                        frame,
                        earliest,
                        latest,
                        interval_to_x,
                        |frame, x_position, kline, trades| {
                            draw_clusters(
                                frame,
                                price_to_y,
                                x_position,
                                chart.cell_width,
                                chart.cell_height,
                                candle_width,
                                cell_height_unscaled,
                                cell_width_unscaled,
                                max_cluster_qty,
                                palette,
                                text_size,
                                self.tick_size(),
                                imb_threshold,
                                kline,
                                trades,
                                *clusters,
                            );
                        },
                    );
                }
                KlineChartKind::Candles => {
                    let candle_width = chart.cell_width * 0.8;

                    render_data_source(
                        &self.data_source,
                        frame,
                        earliest,
                        latest,
                        interval_to_x,
                        |frame, x_position, kline, _| {
                            draw_candle_dp(
                                frame,
                                price_to_y,
                                candle_width,
                                palette,
                                x_position,
                                kline,
                            );
                        },
                    );
                }
            }

            chart.draw_last_price_line(frame, palette, region);
        });

        if chart.crosshair {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    let (_, rounded_aggregation) =
                        chart.draw_crosshair(frame, theme, bounds_size, cursor_position);

                    draw_crosshair_tooltip(&self.data_source, frame, palette, rounded_aggregation);
                }
            });

            vec![klines, crosshair]
        } else {
            vec![klines]
        }
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Panning { .. } => mouse::Interaction::Grabbing,
            Interaction::Zoomin { .. } => mouse::Interaction::ZoomIn,
            Interaction::None => {
                if cursor.is_over(bounds) && self.chart.crosshair {
                    return mouse::Interaction::Crosshair;
                }
                mouse::Interaction::default()
            }
        }
    }
}

fn draw_footprint_kline(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(f32) -> f32,
    x_position: f32,
    candle_width: f32,
    kline: &Kline,
    palette: &Extended,
) {
    let y_open = price_to_y(kline.open);
    let y_high = price_to_y(kline.high);
    let y_low = price_to_y(kline.low);
    let y_close = price_to_y(kline.close);

    let body_color = if kline.close >= kline.open {
        palette.success.weak.color
    } else {
        palette.danger.weak.color
    };
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 8.0), y_open.min(y_close)),
        Size::new(candle_width / 4.0, (y_open - y_close).abs()),
        body_color,
    );

    let wick_color = if kline.close >= kline.open {
        palette.success.weak.color
    } else {
        palette.danger.weak.color
    };
    let marker_line = Stroke::with_color(
        Stroke {
            width: 1.0,
            ..Default::default()
        },
        wick_color.scale_alpha(0.6),
    );
    frame.stroke(
        &Path::line(
            Point::new(x_position, y_high),
            Point::new(x_position, y_low),
        ),
        marker_line,
    );
}

fn draw_candle_dp(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(f32) -> f32,
    candle_width: f32,
    palette: &Extended,
    x_position: f32,
    kline: &Kline,
) {
    let y_open = price_to_y(kline.open);
    let y_high = price_to_y(kline.high);
    let y_low = price_to_y(kline.low);
    let y_close = price_to_y(kline.close);

    let body_color = if kline.close >= kline.open {
        palette.success.base.color
    } else {
        palette.danger.base.color
    };
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 2.0), y_open.min(y_close)),
        Size::new(candle_width, (y_open - y_close).abs()),
        body_color,
    );

    let wick_color = if kline.close >= kline.open {
        palette.success.base.color
    } else {
        palette.danger.base.color
    };
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 8.0), y_high),
        Size::new(candle_width / 4.0, (y_high - y_low).abs()),
        wick_color,
    );
}

fn render_data_source<F>(
    data_source: &ChartData,
    frame: &mut canvas::Frame,
    earliest: u64,
    latest: u64,
    interval_to_x: impl Fn(u64) -> f32,
    draw_fn: F,
) where
    F: Fn(&mut canvas::Frame, f32, &Kline, &KlineTrades),
{
    match data_source {
        ChartData::TickBased(tick_aggr) => {
            let earliest = earliest as usize;
            let latest = latest as usize;

            tick_aggr
                .data_points
                .iter()
                .rev()
                .enumerate()
                .filter(|(index, _)| *index <= latest && *index >= earliest)
                .for_each(|(index, tick_aggr)| {
                    let x_position = interval_to_x(index as u64);

                    draw_fn(frame, x_position, &tick_aggr.kline, &tick_aggr.footprint);
                });
        }
        ChartData::TimeBased(timeseries) => {
            if latest < earliest {
                return;
            }

            timeseries
                .data_points
                .range(earliest..=latest)
                .for_each(|(timestamp, dp)| {
                    let x_position = interval_to_x(*timestamp);

                    draw_fn(frame, x_position, &dp.kline, &dp.footprint);
                });
        }
    }
}

fn draw_all_npocs(
    data_source: &ChartData,
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(f32) -> f32,
    interval_to_x: impl Fn(u64) -> f32,
    candle_width: f32,
    cell_width: f32,
    palette: &Extended,
    studies: &[FootprintStudy],
) {
    if !studies.contains(&FootprintStudy::NPoC) {
        return;
    }

    match data_source {
        ChartData::TickBased(tick_aggr) => {
            tick_aggr
                .data_points
                .iter()
                .rev()
                .enumerate()
                .for_each(|(index, dp)| {
                    if let Some(poc) = dp.footprint.poc {
                        let x_position = interval_to_x(index as u64);
                        let poc_y = price_to_y(poc.price);

                        let start_x = x_position + (candle_width / 4.0);
                        let (until_x, color) = match poc.status {
                            NPoc::Naked => (-x_position, palette.warning.weak.color),
                            NPoc::Filled { at } => {
                                let until_x = interval_to_x(at) - start_x;
                                if until_x.abs() <= cell_width {
                                    return;
                                }
                                (until_x, palette.background.strong.color)
                            }
                        };

                        frame.fill_rectangle(
                            Point::new(start_x, poc_y - 1.0),
                            Size::new(until_x, 1.0),
                            color,
                        );
                    }
                });
        }
        ChartData::TimeBased(timeseries) => {
            timeseries.data_points.iter().for_each(|(timestamp, dp)| {
                if let Some(poc) = dp.footprint.poc {
                    let x_position = interval_to_x(*timestamp);
                    let poc_y = price_to_y(poc.price);

                    let start_x = x_position + (candle_width / 4.0);
                    let (until_x, color) = match poc.status {
                        NPoc::Naked => (-x_position, palette.warning.weak.color),
                        NPoc::Filled { at } => {
                            let until_x = interval_to_x(at) - start_x;
                            if until_x.abs() <= cell_width {
                                return;
                            }
                            (until_x, palette.background.strong.color)
                        }
                    };

                    frame.fill_rectangle(
                        Point::new(start_x, poc_y - 1.0),
                        Size::new(until_x, 1.0),
                        color,
                    );
                }
            });
        }
    }
}

fn draw_clusters(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(f32) -> f32,
    x_position: f32,
    cell_width: f32,
    cell_height: f32,
    candle_width: f32,
    cell_height_unscaled: f32,
    cell_width_unscaled: f32,
    max_cluster_qty: f32,
    palette: &Extended,
    text_size: f32,
    tick_size: f32,
    imb_threshold: Option<i32>,
    kline: &Kline,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
) {
    let text_color = palette.background.weakest.text;

    match cluster_kind {
        ClusterKind::VolumeProfile => {
            let should_show_text = cell_height_unscaled > 8.0 && cell_width_unscaled > 80.0;
            let bar_color_alpha = if should_show_text { 0.25 } else { 1.0 };

            for (price, (buy_qty, sell_qty)) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some(threshold) = imb_threshold {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        price,
                        higher_price,
                        threshold,
                        cell_height,
                        palette,
                        x_position,
                        cell_width,
                        cluster_kind,
                    );
                }

                let start_x = x_position + (candle_width / 4.0);

                draw_horizontal_volume_bars(
                    frame,
                    start_x,
                    y_position,
                    *buy_qty,
                    *sell_qty,
                    max_cluster_qty,
                    cell_height,
                    cell_width * 0.8,
                    palette.success.base.color,
                    palette.danger.base.color,
                    bar_color_alpha,
                );

                if should_show_text {
                    draw_cluster_text(
                        frame,
                        &abbr_large_numbers(buy_qty + sell_qty),
                        Point::new(x_position + (candle_width / 4.0), y_position),
                        text_size,
                        text_color,
                        Alignment::Start,
                        Alignment::Center,
                    );
                }
            }
        }
        ClusterKind::DeltaProfile => {
            let should_show_text = cell_height_unscaled > 8.0 && cell_width_unscaled > 80.0;
            let bar_color_alpha = if should_show_text { 0.25 } else { 1.0 };

            for (price, (buy_qty, sell_qty)) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some(threshold) = imb_threshold {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        price,
                        higher_price,
                        threshold,
                        cell_height,
                        palette,
                        x_position,
                        cell_width,
                        cluster_kind,
                    );
                }

                let delta_qty = buy_qty - sell_qty;

                if should_show_text {
                    draw_cluster_text(
                        frame,
                        &abbr_large_numbers(delta_qty),
                        Point::new(x_position + (candle_width / 4.0), y_position),
                        text_size,
                        text_color,
                        Alignment::Start,
                        Alignment::Center,
                    );
                }

                let bar_width = (delta_qty.abs() / max_cluster_qty) * (cell_width * 0.8);
                let bar_color = if delta_qty >= 0.0 {
                    palette.success.base.color.scale_alpha(bar_color_alpha)
                } else {
                    palette.danger.base.color.scale_alpha(bar_color_alpha)
                };

                frame.fill_rectangle(
                    Point::new(
                        x_position + (candle_width / 4.0),
                        y_position - (cell_height / 2.0),
                    ),
                    Size::new(bar_width, cell_height),
                    bar_color,
                );
            }
        }
        ClusterKind::BidAsk => {
            let should_show_text = cell_height_unscaled > 8.0 && cell_width_unscaled > 120.0;
            let bar_color_alpha = if should_show_text { 0.25 } else { 1.0 };

            for (price, (buy_qty, sell_qty)) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some(threshold) = imb_threshold {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        price,
                        higher_price,
                        threshold,
                        cell_height,
                        palette,
                        x_position,
                        cell_width,
                        cluster_kind,
                    );
                }

                if *buy_qty > 0.0 {
                    if should_show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(*buy_qty),
                            Point::new(x_position + (candle_width / 4.0), y_position),
                            text_size,
                            text_color,
                            Alignment::Start,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (buy_qty / max_cluster_qty) * (cell_width * 0.4);
                    frame.fill_rectangle(
                        Point::new(
                            x_position + (candle_width / 4.0),
                            y_position - (cell_height / 2.0),
                        ),
                        Size::new(bar_width, cell_height),
                        palette.success.base.color.scale_alpha(bar_color_alpha),
                    );
                }

                if *sell_qty > 0.0 {
                    if should_show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(*sell_qty),
                            Point::new(x_position - (candle_width / 4.0), y_position),
                            text_size,
                            text_color,
                            Alignment::End,
                            Alignment::Center,
                        );
                    }

                    let bar_width = -(sell_qty / max_cluster_qty) * (cell_width * 0.4);
                    frame.fill_rectangle(
                        Point::new(
                            x_position - (candle_width / 4.0),
                            y_position - (cell_height / 2.0),
                        ),
                        Size::new(bar_width, cell_height),
                        palette.danger.base.color.scale_alpha(bar_color_alpha),
                    );
                }
            }
        }
    }

    draw_footprint_kline(frame, &price_to_y, x_position, candle_width, kline, palette);
}

fn draw_imbalance_marker(
    frame: &mut canvas::Frame,
    price_to_y: &impl Fn(f32) -> f32,
    footprint: &KlineTrades,
    price: &OrderedFloat<f32>,
    higher_price: OrderedFloat<f32>,
    threshold: i32,
    cell_height: f32,
    palette: &Extended,
    x_position: f32,
    cell_width: f32,
    cluster_kind: ClusterKind,
) {
    if let Some((diagonal_buy_qty, _)) = footprint.trades.get(&higher_price) {
        let (_, sell_qty) = footprint.trades.get(price).unwrap();
        let radius = (cell_height / 2.0).min(2.0);

        let (success_x, danger_x) = match cluster_kind {
            ClusterKind::BidAsk => (
                x_position + (cell_width / 2.0) - (radius * 2.0),
                x_position - (cell_width / 2.0) + (radius * 2.0),
            ),
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => (
                x_position - (radius * 2.0),
                x_position - 2.0 * (radius * 2.0) - 1.0,
            ),
        };

        if diagonal_buy_qty >= sell_qty {
            let required_qty = *sell_qty * (100 + threshold) as f32 / 100.0;

            if *diagonal_buy_qty > required_qty {
                let y_position = price_to_y(*higher_price);
                frame.fill(
                    &Path::circle(Point::new(success_x, y_position), radius),
                    palette.success.weak.color,
                );
            }
        } else {
            let required_qty = *diagonal_buy_qty * (100 + threshold) as f32 / 100.0;

            if *sell_qty > required_qty {
                let y_position = price_to_y(**price);
                frame.fill(
                    &Path::circle(Point::new(danger_x, y_position), radius),
                    palette.danger.weak.color,
                );
            }
        }
    }
}

fn draw_cluster_text(
    frame: &mut canvas::Frame,
    text: &str,
    position: Point,
    text_size: f32,
    color: iced::Color,
    align_x: Alignment,
    align_y: Alignment,
) {
    frame.fill_text(canvas::Text {
        content: text.to_string(),
        position,
        size: iced::Pixels(text_size),
        color,
        align_x: align_x.into(),
        align_y: align_y.into(),
        font: style::AZERET_MONO,
        ..canvas::Text::default()
    });
}

fn draw_crosshair_tooltip(
    data: &ChartData,
    frame: &mut canvas::Frame,
    palette: &Extended,
    at_interval: u64,
) {
    let tooltip = match data {
        ChartData::TimeBased(timeseries) => {
            let dp_opt = timeseries
                .data_points
                .iter()
                .find(|(time, _)| **time == at_interval)
                .map(|(_, dp)| dp);

            let dp_opt = if dp_opt.is_none() && !timeseries.data_points.is_empty() {
                if let Some((last_time, _)) = timeseries.data_points.last_key_value() {
                    if at_interval > *last_time {
                        timeseries.data_points.last_key_value().map(|(_, dp)| dp)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                dp_opt
            };

            if let Some(dp) = dp_opt {
                let change_pct = ((dp.kline.close - dp.kline.open) / dp.kline.open) * 100.0;

                let tooltip_text = format!(
                    "O: {} H: {} L: {} C: {} {:+.2}%",
                    dp.kline.open, dp.kline.high, dp.kline.low, dp.kline.close, change_pct
                );

                Some((
                    tooltip_text,
                    if change_pct >= 0.0 {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    },
                ))
            } else {
                None
            }
        }
        ChartData::TickBased(tick_aggr) => {
            let index = (at_interval / tick_aggr.interval) as usize;

            if index < tick_aggr.data_points.len() {
                let dp = &tick_aggr.data_points[tick_aggr.data_points.len() - 1 - index];

                let change_pct = ((dp.kline.close - dp.kline.open) / dp.kline.open) * 100.0;

                let tooltip_text = format!(
                    "O: {} H: {} L: {} C: {} {:+.2}%",
                    dp.kline.open, dp.kline.high, dp.kline.low, dp.kline.close, change_pct
                );

                Some((
                    tooltip_text,
                    if change_pct >= 0.0 {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    },
                ))
            } else {
                None
            }
        }
    };

    if let Some((content, color)) = tooltip {
        let position = Point::new(8.0, 8.0);

        let tooltip_rect = Rectangle {
            x: position.x,
            y: position.y,
            width: content.len() as f32 * 8.0,
            height: 16.0,
        };

        frame.fill_rectangle(
            tooltip_rect.position(),
            tooltip_rect.size(),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        let text = canvas::Text {
            content,
            position,
            size: iced::Pixels(12.0),
            color,
            font: style::AZERET_MONO,
            ..canvas::Text::default()
        };
        frame.fill_text(text);
    }
}
