use super::{
    Action, Basis, Caches, Chart, Interaction, Message, PlotConstants, PlotData, TEXT_SIZE,
    ViewState, indicator, request_fetch, scale::linear::PriceInfoLabel,
};
use crate::{modal::pane::settings::study, style};
use data::aggr::ticks::TickAggr;
use data::aggr::time::TimeSeries;
use data::chart::{
    KlineChartKind, ViewConfig,
    indicator::{Indicator, KlineIndicator},
    kline::{ClusterKind, FootprintStudy, KlineDataPoint, KlineTrades, NPoc, PointOfControl},
};
use data::util::{abbr_large_numbers, count_decimals, round_to_tick};
use exchange::{
    Kline, OpenInterest as OIData, TickerInfo, Timeframe, Trade,
    fetcher::{FetchRange, RequestHandler},
};

use iced::task::Handle;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Event, Geometry, Path, Stroke};
use iced::{Alignment, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};
use ordered_float::OrderedFloat;

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

impl Chart for KlineChart {
    type IndicatorType = KlineIndicator;

    fn state(&self) -> &ViewState {
        &self.chart
    }

    fn mut_state(&mut self) -> &mut ViewState {
        &mut self.chart
    }

    fn invalidate_crosshair(&mut self) {
        self.chart.cache.clear_crosshair();
        self.indicators.iter_mut().for_each(|(_, data)| {
            data.clear_crosshair();
        });
    }

    fn invalidate_all(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(&'_ self, enabled: &[Self::IndicatorType]) -> Vec<Element<'_, Message>> {
        let chart_state = self.state();

        let visible_region = chart_state.visible_region(chart_state.bounds.size());
        let (earliest, latest) = chart_state.interval_range(&visible_region);

        if earliest > latest {
            return vec![];
        }

        let mut indicators = vec![];

        let market = match chart_state.ticker_info {
            Some(ref info) => info.market_type(),
            None => return indicators,
        };

        for selected_indicator in enabled {
            if !KlineIndicator::for_market(market).contains(selected_indicator) {
                continue;
            }

            if let Some(data) = self.indicators.get(selected_indicator) {
                indicators.push(data.indicator_elem(chart_state, earliest, latest));
            }
        }

        indicators
    }

    fn visible_timerange(&self) -> (u64, u64) {
        let chart = self.state();
        let region = chart.visible_region(chart.bounds.size());

        match &chart.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds();

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

    fn interval_keys(&self) -> Option<Vec<u64>> {
        match &self.data_source {
            PlotData::TimeBased(_) => None,
            PlotData::TickBased(tick_aggr) => Some(
                tick_aggr
                    .datapoints
                    .iter()
                    .map(|dp| dp.kline.time)
                    .collect(),
            ),
        }
    }

    fn autoscaled_coords(&self) -> Vector {
        let chart = self.state();
        let x_translation = match &self.kind {
            KlineChartKind::Footprint { .. } => {
                0.5 * (chart.bounds.width / chart.scaling) - (chart.cell_width / chart.scaling)
            }
            KlineChartKind::Candles => {
                0.5 * (chart.bounds.width / chart.scaling)
                    - (8.0 * chart.cell_width / chart.scaling)
            }
        };
        Vector::new(x_translation, chart.translation.y)
    }

    fn supports_fit_autoscaling(&self) -> bool {
        true
    }

    fn is_empty(&self) -> bool {
        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries.datapoints.is_empty(),
            PlotData::TickBased(tick_aggr) => tick_aggr.datapoints.is_empty(),
        }
    }
}

enum IndicatorData {
    Volume(Caches, BTreeMap<u64, (f32, f32)>),
    OpenInterest(Caches, BTreeMap<u64, f32>),
}

impl IndicatorData {
    fn clear_all(&mut self) {
        match self {
            IndicatorData::Volume(caches, _) | IndicatorData::OpenInterest(caches, _) => {
                caches.clear_all();
            }
        }
    }

    fn clear_crosshair(&mut self) {
        match self {
            IndicatorData::Volume(caches, _) | IndicatorData::OpenInterest(caches, _) => {
                caches.clear_crosshair();
            }
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        chart: &'a ViewState,
        earliest: u64,
        latest: u64,
    ) -> Element<'a, Message> {
        match self {
            IndicatorData::Volume(cache, data) => {
                indicator::volume::indicator_elem(chart, cache, data, earliest, latest)
            }
            IndicatorData::OpenInterest(cache, data) => {
                indicator::open_interest::indicator_elem(chart, cache, data, earliest, latest)
            }
        }
    }
}

impl PlotConstants for KlineChart {
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
    chart: ViewState,
    data_source: PlotData<KlineDataPoint>,
    raw_trades: Vec<Trade>,
    indicators: HashMap<KlineIndicator, IndicatorData>,
    fetching_trades: (bool, Option<Handle>),
    kind: KlineChartKind,
    request_handler: RequestHandler,
    study_configurator: study::Configurator<FootprintStudy>,
    last_tick: Instant,
}

impl KlineChart {
    pub fn new(
        layout: ViewConfig,
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
                    TimeSeries::<KlineDataPoint>::new(interval, tick_size, &raw_trades, klines_raw);

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

                let mut chart = ViewState {
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
                    layout,
                    ticker_info,
                    basis,
                    ..Default::default()
                };

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                KlineChart {
                    chart,
                    data_source: PlotData::TimeBased(timeseries),
                    raw_trades,
                    indicators: enabled_indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::new(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
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

                let mut chart = ViewState {
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
                    layout,
                    ticker_info,
                    basis,
                    ..Default::default()
                };

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                KlineChart {
                    chart,
                    data_source: PlotData::TickBased(TickAggr::new(
                        interval,
                        tick_size,
                        &raw_trades,
                    )),
                    raw_trades,
                    indicators: enabled_indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::new(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                }
            }
        }
    }

    pub fn update_latest_kline(&mut self, kline: &Kline) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(&[kline.to_owned()]);

                if let Some(IndicatorData::Volume(_, data)) =
                    self.indicators.get_mut(&KlineIndicator::Volume)
                {
                    data.insert(kline.time, (kline.volume.0, kline.volume.1));
                };

                let chart = self.mut_state();

                if (kline.time) > chart.latest_x {
                    chart.latest_x = kline.time;
                }

                chart.last_price = Some(PriceInfoLabel::new(kline.close, kline.open));
            }
            PlotData::TickBased(_) => {}
        }
    }

    pub fn kind(&self) -> &KlineChartKind {
        &self.kind
    }

    fn missing_data_task(&mut self) -> Option<Action> {
        match &self.data_source {
            PlotData::TimeBased(timeseries) => {
                let timeframe = timeseries.interval.to_milliseconds();

                let (visible_earliest, visible_latest) = self.visible_timerange();
                let (kline_earliest, kline_latest) = timeseries.timerange();
                let earliest = visible_earliest.saturating_sub(visible_latest - visible_earliest);

                // priority 1, basic kline data fetch
                if visible_earliest < kline_earliest {
                    let range = FetchRange::Kline(earliest, kline_earliest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                // priority 2, trades fetch
                if !self.fetching_trades.0
                    && exchange::fetcher::is_trade_fetch_enabled()
                    && let Some((fetch_from, fetch_to)) =
                        timeseries.suggest_trade_fetch_range(visible_earliest, visible_latest)
                {
                    let range = FetchRange::Trades(fetch_from, fetch_to);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        self.fetching_trades = (true, None);
                        return Some(action);
                    }
                }

                // priority 3, Open Interest data
                for data in self.indicators.values() {
                    if let IndicatorData::OpenInterest(_, _) = data
                        && timeframe >= Timeframe::M5.to_milliseconds()
                        && self.chart.ticker_info.is_some_and(|t| {
                            t.is_perps()
                                && t.exchange() != exchange::adapter::Exchange::HyperliquidLinear
                        })
                    {
                        let (oi_earliest, oi_latest) = self.oi_timerange(kline_latest);

                        if visible_earliest < oi_earliest {
                            let range = FetchRange::OpenInterest(earliest, oi_earliest);

                            if let Some(action) = request_fetch(&mut self.request_handler, range) {
                                return Some(action);
                            }
                        }

                        if oi_latest < kline_latest {
                            let range =
                                FetchRange::OpenInterest(oi_latest.max(earliest), kline_latest);

                            if let Some(action) = request_fetch(&mut self.request_handler, range) {
                                return Some(action);
                            }
                        }
                    }
                }

                // priority 4, missing klines & integrity check
                if let Some(missing_keys) =
                    timeseries.check_kline_integrity(kline_earliest, kline_latest, timeframe)
                {
                    let latest = missing_keys.iter().max().unwrap_or(&visible_latest) + timeframe;
                    let earliest =
                        missing_keys.iter().min().unwrap_or(&visible_earliest) - timeframe;

                    let range = FetchRange::Kline(earliest, latest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }
            }
            PlotData::TickBased(_) => {
                // TODO: implement trade fetch
            }
        }

        None
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
            PlotData::TimeBased(ref mut source) => {
                source.clear_trades();

                if clear_raw {
                    self.raw_trades.clear();
                } else {
                    source.insert_trades(&self.raw_trades);
                }
            }
            PlotData::TickBased(_) => {
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

    pub fn study_configurator(&self) -> &study::Configurator<FootprintStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<FootprintStudy>) {
        let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        else {
            return;
        };

        match self.study_configurator.update(message) {
            Some(study::Action::ToggleStudy(study, is_selected)) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|s| !s.is_same_type(&study));
                }
            }
            Some(study::Action::ConfigureStudy(study)) => {
                if let Some(existing_study) = studies.iter_mut().find(|s| s.is_same_type(&study)) {
                    *existing_study = study;
                }
            }
            None => {}
        }

        self.invalidate(None);
    }

    pub fn chart_layout(&self) -> ViewConfig {
        self.chart.layout()
    }

    pub fn set_cluster_kind(&mut self, new_kind: ClusterKind) {
        if let KlineChartKind::Footprint {
            ref mut clusters, ..
        } = self.kind
        {
            *clusters = new_kind;
        }

        self.invalidate(None);
    }

    pub fn basis(&self) -> Basis {
        self.chart.basis
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart = self.mut_state();

        chart.cell_height *= new_tick_size / chart.tick_size;
        chart.tick_size = new_tick_size;

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.change_tick_size(new_tick_size, &self.raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.change_tick_size(new_tick_size, &self.raw_trades);
            }
        }

        self.clear_trades(false);
        self.invalidate(None);
    }

    pub fn set_tick_basis(&mut self, tick_basis: data::aggr::TickCount) {
        self.chart.basis = Basis::Tick(tick_basis);

        let new_tick_aggr = TickAggr::new(tick_basis, self.chart.tick_size, &self.raw_trades);

        if let Some(indicator) = self.indicators.get_mut(&KlineIndicator::Volume) {
            *indicator = IndicatorData::Volume(Caches::default(), new_tick_aggr.volume_data());
        }

        self.data_source = PlotData::TickBased(new_tick_aggr);

        self.invalidate(None);
    }

    pub fn studies(&self) -> Option<Vec<FootprintStudy>> {
        match &self.kind {
            KlineChartKind::Footprint { studies, .. } => Some(studies.clone()),
            _ => None,
        }
    }

    pub fn set_studies(&mut self, new_studies: Vec<FootprintStudy>) {
        if let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        {
            *studies = new_studies;
        }

        self.invalidate(None);
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

    pub fn insert_trades_buffer(&mut self, trades_buffer: &[Trade]) {
        self.raw_trades.extend_from_slice(trades_buffer);

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                let old_dp_len = tick_aggr.datapoints.len();

                tick_aggr.insert_trades(trades_buffer);

                if let Some(IndicatorData::Volume(_, data)) =
                    self.indicators.get_mut(&KlineIndicator::Volume)
                {
                    let start_idx = old_dp_len.saturating_sub(1);
                    for (idx, dp) in tick_aggr.datapoints.iter().enumerate().skip(start_idx) {
                        data.insert(idx as u64, (dp.kline.volume.0, dp.kline.volume.1));
                    }
                }

                if let Some(last_dp) = tick_aggr.datapoints.last() {
                    self.chart.last_price =
                        Some(PriceInfoLabel::new(last_dp.kline.close, last_dp.kline.open));
                } else {
                    self.chart.last_price = None;
                }

                self.invalidate(None);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades(trades_buffer);
            }
        }
    }

    pub fn insert_raw_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.insert_trades(&raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades(&raw_trades);
            }
        }

        self.raw_trades.extend(raw_trades);

        if is_batches_done {
            self.fetching_trades = (false, None);
        }
    }

    pub fn insert_new_klines(&mut self, req_id: uuid::Uuid, klines_raw: &[Kline]) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
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
            PlotData::TickBased(_) => {}
        }
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
            PlotData::TimeBased(timeseries) => timeseries.max_qty_ts_range(
                cluster_kind,
                earliest,
                latest,
                rounded_highest,
                rounded_lowest,
            ),
            PlotData::TickBased(tick_aggr) => {
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

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let chart = &mut self.chart;

        if let Some(autoscale) = chart.layout.autoscale {
            match autoscale {
                super::Autoscale::CenterLatest => {
                    let x_translation = match &self.kind {
                        KlineChartKind::Footprint { .. } => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (chart.cell_width / chart.scaling)
                        }
                        KlineChartKind::Candles => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (8.0 * chart.cell_width / chart.scaling)
                        }
                    };
                    chart.translation.x = x_translation;

                    let calculate_target_y = |kline: exchange::Kline| -> f32 {
                        let y_low = chart.price_to_y(kline.low);
                        let y_high = chart.price_to_y(kline.high);
                        let y_close = chart.price_to_y(kline.close);

                        let mut target_y_translation = -(y_low + y_high) / 2.0;

                        if chart.bounds.height > f32::EPSILON && chart.scaling > f32::EPSILON {
                            let visible_half_height = (chart.bounds.height / chart.scaling) / 2.0;

                            let view_center_y_centered = -target_y_translation;

                            let visible_y_top = view_center_y_centered - visible_half_height;
                            let visible_y_bottom = view_center_y_centered + visible_half_height;

                            let padding = chart.cell_height;

                            if y_close < visible_y_top {
                                target_y_translation = -(y_close - padding + visible_half_height);
                            } else if y_close > visible_y_bottom {
                                target_y_translation = -(y_close + padding - visible_half_height);
                            }
                        }
                        target_y_translation
                    };

                    chart.translation.y = self.data_source.latest_y_midpoint(calculate_target_y);
                }
                super::Autoscale::FitToVisible => {
                    let visible_region = chart.visible_region(chart.bounds.size());
                    let (start_interval, end_interval) = chart.interval_range(&visible_region);

                    if let Some((lowest, highest)) = self
                        .data_source
                        .visible_price_range(start_interval, end_interval)
                    {
                        let padding = (highest - lowest) * 0.05;
                        let price_span = (highest - lowest) + (2.0 * padding);

                        if price_span > 0.0 && chart.bounds.height > f32::EPSILON {
                            let padded_highest = highest + padding;
                            let chart_height = chart.bounds.height;
                            let tick_size = chart.tick_size;

                            if tick_size > 0.0 {
                                chart.cell_height = (chart_height * tick_size) / price_span;
                                chart.base_price_y = padded_highest;
                                chart.translation.y = -chart_height / 2.0;
                            }
                        }
                    }
                }
            }
        }

        chart.cache.clear_all();
        self.indicators.iter_mut().for_each(|(_, data)| {
            data.clear_all();
        });

        if let Some(t) = now {
            self.last_tick = t;
            self.missing_data_task()
        } else {
            None
        }
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) {
        let prev_indi_count = self.indicators.len();

        match self.indicators.entry(indicator) {
            Entry::Occupied(entry) => {
                entry.remove();
            }
            Entry::Vacant(entry) => {
                let data = match indicator {
                    KlineIndicator::Volume => match &self.data_source {
                        PlotData::TimeBased(timeseries) => {
                            IndicatorData::Volume(Caches::default(), timeseries.into())
                        }
                        PlotData::TickBased(tick_aggr) => {
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

        if let Some(main_split) = self.chart.layout.splits.first() {
            let current_indi_count = self.indicators.len();
            self.chart.layout.splits = data::util::calc_panel_splits(
                *main_split,
                current_indi_count,
                Some(prev_indi_count),
            );
        }
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
        super::canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn draw(
        &self,
        interaction: &Interaction,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let chart = self.state();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let bounds_size = bounds.size();
        let palette = theme.extended_palette();

        let klines = chart.cache.main.draw(renderer, bounds_size, |frame| {
            let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);

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

                    let imbalance = studies.iter().find_map(|study| {
                        if let FootprintStudy::Imbalance {
                            threshold,
                            color_scale,
                            ignore_zeros,
                        } = study
                        {
                            Some((*threshold, *color_scale, *ignore_zeros))
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
                        chart.cell_height,
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
                                imbalance,
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

        let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
            if let Some(cursor_position) = cursor.position_in(bounds) {
                let (_, rounded_aggregation) =
                    chart.draw_crosshair(frame, theme, bounds_size, cursor_position, interaction);

                draw_crosshair_tooltip(&self.data_source, frame, palette, rounded_aggregation);
            }
        });

        vec![klines, crosshair]
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
            Interaction::None | Interaction::Ruler { .. } => {
                if cursor.is_over(bounds) {
                    mouse::Interaction::Crosshair
                } else {
                    mouse::Interaction::default()
                }
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
    data_source: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    earliest: u64,
    latest: u64,
    interval_to_x: impl Fn(u64) -> f32,
    draw_fn: F,
) where
    F: Fn(&mut canvas::Frame, f32, &Kline, &KlineTrades),
{
    match data_source {
        PlotData::TickBased(tick_aggr) => {
            let earliest = earliest as usize;
            let latest = latest as usize;

            tick_aggr
                .datapoints
                .iter()
                .rev()
                .enumerate()
                .filter(|(index, _)| *index <= latest && *index >= earliest)
                .for_each(|(index, tick_aggr)| {
                    let x_position = interval_to_x(index as u64);

                    draw_fn(frame, x_position, &tick_aggr.kline, &tick_aggr.footprint);
                });
        }
        PlotData::TimeBased(timeseries) => {
            if latest < earliest {
                return;
            }

            timeseries
                .datapoints
                .range(earliest..=latest)
                .for_each(|(timestamp, dp)| {
                    let x_position = interval_to_x(*timestamp);

                    draw_fn(frame, x_position, &dp.kline, &dp.footprint);
                });
        }
    }
}

fn draw_all_npocs(
    data_source: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(f32) -> f32,
    interval_to_x: impl Fn(u64) -> f32,
    candle_width: f32,
    cell_width: f32,
    cell_height: f32,
    palette: &Extended,
    studies: &[FootprintStudy],
) {
    let Some(lookback) = studies.iter().find_map(|study| {
        if let FootprintStudy::NPoC { lookback } = study {
            Some(*lookback)
        } else {
            None
        }
    }) else {
        return;
    };

    let (filled_color, naked_color) = (
        palette.background.strong.color,
        if palette.is_dark {
            palette.warning.weak.color.scale_alpha(0.5)
        } else {
            palette.warning.strong.color
        },
    );

    let line_height = cell_height.min(1.0);

    let mut draw_the_line = |interval: u64, poc: &PointOfControl| {
        let x_position = interval_to_x(interval);
        let start_x = x_position + (candle_width / 4.0);

        let (until_x, color) = match poc.status {
            NPoc::Naked => (-x_position, naked_color),
            NPoc::Filled { at } => {
                let until_x = interval_to_x(at) - start_x;
                if until_x.abs() <= cell_width {
                    return;
                }
                (until_x, filled_color)
            }
            _ => return,
        };

        frame.fill_rectangle(
            Point::new(start_x, price_to_y(poc.price) - line_height / 2.0),
            Size::new(until_x, line_height),
            color,
        );
    };

    match data_source {
        PlotData::TickBased(tick_aggr) => {
            tick_aggr
                .datapoints
                .iter()
                .rev()
                .enumerate()
                .take(lookback)
                .filter_map(|(index, dp)| dp.footprint.poc.as_ref().map(|poc| (index as u64, poc)))
                .for_each(|(interval, poc)| draw_the_line(interval, poc));
        }
        PlotData::TimeBased(timeseries) => {
            timeseries
                .datapoints
                .iter()
                .rev()
                .take(lookback)
                .filter_map(|(timestamp, dp)| {
                    dp.footprint.poc.as_ref().map(|poc| (*timestamp, poc))
                })
                .for_each(|(interval, poc)| draw_the_line(interval, poc));
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
    imbalance: Option<(usize, Option<usize>, bool)>,
    kline: &Kline,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
) {
    let text_color = palette.background.weakest.text;

    match cluster_kind {
        ClusterKind::VolumeProfile => {
            let should_show_text = cell_height_unscaled > 8.0 && cell_width_unscaled > 80.0;
            let bar_color_alpha = if should_show_text { 0.25 } else { 1.0 };

            for (price, group) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some((threshold, color_scale, ignore_zeros)) = imbalance {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        *price,
                        group.sell_qty,
                        higher_price,
                        threshold,
                        color_scale,
                        ignore_zeros,
                        cell_height,
                        palette,
                        x_position - (candle_width / 4.0),
                        cell_width,
                        cluster_kind,
                    );
                }

                let start_x = x_position + (candle_width / 4.0);

                super::draw_volume_bar(
                    frame,
                    start_x,
                    y_position,
                    group.buy_qty,
                    group.sell_qty,
                    max_cluster_qty,
                    cell_width * 0.8,
                    cell_height,
                    palette.success.base.color,
                    palette.danger.base.color,
                    bar_color_alpha,
                    true,
                );

                if should_show_text {
                    draw_cluster_text(
                        frame,
                        &abbr_large_numbers(group.total_qty()),
                        Point::new(start_x, y_position),
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

            for (price, group) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some((threshold, color_scale, ignore_zeros)) = imbalance {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        *price,
                        group.sell_qty,
                        higher_price,
                        threshold,
                        color_scale,
                        ignore_zeros,
                        cell_height,
                        palette,
                        x_position - (candle_width / 4.0),
                        cell_width,
                        cluster_kind,
                    );
                }

                let delta_qty = group.delta_qty();

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

            for (price, group) in &footprint.trades {
                let y_position = price_to_y(**price);

                if let Some((threshold, color_scale, ignore_zeros)) = imbalance {
                    let higher_price = OrderedFloat(round_to_tick(**price + tick_size, tick_size));

                    draw_imbalance_marker(
                        frame,
                        &price_to_y,
                        footprint,
                        *price,
                        group.sell_qty,
                        higher_price,
                        threshold,
                        color_scale,
                        ignore_zeros,
                        cell_height,
                        palette,
                        x_position,
                        cell_width,
                        cluster_kind,
                    );
                }

                if group.buy_qty > 0.0 {
                    if should_show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(group.buy_qty),
                            Point::new(x_position + (candle_width / 4.0), y_position),
                            text_size,
                            text_color,
                            Alignment::Start,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (group.buy_qty / max_cluster_qty) * (cell_width * 0.4);
                    frame.fill_rectangle(
                        Point::new(
                            x_position + (candle_width / 4.0),
                            y_position - (cell_height / 2.0),
                        ),
                        Size::new(bar_width, cell_height),
                        palette.success.base.color.scale_alpha(bar_color_alpha),
                    );
                }

                if group.sell_qty > 0.0 {
                    if should_show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(group.sell_qty),
                            Point::new(x_position - (candle_width / 4.0), y_position),
                            text_size,
                            text_color,
                            Alignment::End,
                            Alignment::Center,
                        );
                    }

                    let bar_width = -(group.sell_qty / max_cluster_qty) * (cell_width * 0.4);
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
    price: OrderedFloat<f32>,
    sell_qty: f32,
    higher_price: OrderedFloat<f32>,
    threshold: usize,
    color_scale: Option<usize>,
    ignore_zeros: bool,
    cell_height: f32,
    palette: &Extended,
    x_position: f32,
    cell_width: f32,
    cluster_kind: ClusterKind,
) {
    if ignore_zeros && sell_qty <= 0.0 {
        return;
    }

    if let Some(group) = footprint.trades.get(&higher_price) {
        let diagonal_buy_qty = &group.buy_qty;

        if ignore_zeros && *diagonal_buy_qty <= 0.0 {
            return;
        }

        let rect_width = cell_width / 16.0;
        let rect_height = cell_height / 2.0;

        let (success_x, danger_x) = match cluster_kind {
            ClusterKind::BidAsk => (
                x_position + (cell_width / 2.0) - rect_width,
                x_position - (cell_width / 2.0),
            ),
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
                (x_position - rect_width, x_position - 2.0 * rect_width - 1.0)
            }
        };

        if *diagonal_buy_qty >= sell_qty {
            let required_qty = sell_qty * (100 + threshold) as f32 / 100.0;

            if *diagonal_buy_qty > required_qty {
                let ratio = *diagonal_buy_qty / required_qty;

                let alpha = if let Some(scale) = color_scale {
                    let divisor = (scale as f32 / 10.0) - 1.0;
                    (0.2 + 0.8 * ((ratio - 1.0) / divisor).min(1.0)).min(1.0)
                } else {
                    1.0
                };

                let y_position = price_to_y(*higher_price);
                frame.fill_rectangle(
                    Point::new(success_x, y_position - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    palette.success.weak.color.scale_alpha(alpha),
                );
            }
        } else {
            let required_qty = *diagonal_buy_qty * (100 + threshold) as f32 / 100.0;

            if sell_qty > required_qty {
                let ratio = sell_qty / required_qty;

                let alpha = if let Some(scale) = color_scale {
                    let divisor = (scale as f32 / 10.0) - 1.0;
                    (0.2 + 0.8 * ((ratio - 1.0) / divisor).min(1.0)).min(1.0)
                } else {
                    1.0
                };

                let y_position = price_to_y(*price);
                frame.fill_rectangle(
                    Point::new(danger_x, y_position - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    palette.danger.weak.color.scale_alpha(alpha),
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
    data: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    palette: &Extended,
    at_interval: u64,
) {
    let kline_opt = match data {
        PlotData::TimeBased(timeseries) => timeseries
            .datapoints
            .iter()
            .find(|(time, _)| **time == at_interval)
            .map(|(_, dp)| &dp.kline)
            .or_else(|| {
                if timeseries.datapoints.is_empty() {
                    None
                } else {
                    let (last_time, dp) = timeseries.datapoints.last_key_value()?;
                    if at_interval > *last_time {
                        Some(&dp.kline)
                    } else {
                        None
                    }
                }
            }),
        PlotData::TickBased(tick_aggr) => {
            let index = (at_interval / u64::from(tick_aggr.interval.0)) as usize;
            if index < tick_aggr.datapoints.len() {
                Some(&tick_aggr.datapoints[tick_aggr.datapoints.len() - 1 - index].kline)
            } else {
                None
            }
        }
    };

    if let Some(kline) = kline_opt {
        let change_pct = ((kline.close - kline.open) / kline.open) * 100.0;
        let change_color = if change_pct >= 0.0 {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };

        let base_color = palette.background.base.text;

        let segments = [
            ("O", base_color, false),
            (&kline.open.to_string(), change_color, true),
            ("H", base_color, false),
            (&kline.high.to_string(), change_color, true),
            ("L", base_color, false),
            (&kline.low.to_string(), change_color, true),
            ("C", base_color, false),
            (&kline.close.to_string(), change_color, true),
            (&format!("{change_pct:+.2}%"), change_color, true),
        ];

        let total_width: f32 = segments
            .iter()
            .map(|(s, _, _)| s.len() as f32 * (TEXT_SIZE * 0.8))
            .sum();

        let position = Point::new(8.0, 8.0);

        let tooltip_rect = Rectangle {
            x: position.x,
            y: position.y,
            width: total_width,
            height: 16.0,
        };

        frame.fill_rectangle(
            tooltip_rect.position(),
            tooltip_rect.size(),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        let mut x = position.x;
        for (text, seg_color, is_value) in segments {
            frame.fill_text(canvas::Text {
                content: text.to_string(),
                position: Point::new(x, position.y),
                size: iced::Pixels(12.0),
                color: seg_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            x += text.len() as f32 * 8.0;
            x += if is_value { 6.0 } else { 2.0 };
        }
    }
}
