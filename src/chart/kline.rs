use super::{
    Action, Basis, Chart, Interaction, Message, PlotConstants, PlotData, TEXT_SIZE, ViewState,
    indicator, request_fetch, scale::linear::PriceInfoLabel,
};
use crate::chart::indicator::kline::KlineIndicatorImpl;
use crate::connector::fetcher::{FetchRange, RequestHandler, is_trade_fetch_enabled};
use crate::{modal::pane::settings::study, style};
use data::aggr::ticks::TickAggr;
use data::aggr::time::TimeSeries;
use data::chart::indicator::{Indicator, KlineIndicator, KlineIndicatorConfig};
use data::chart::kline::{
    ClusterKind, ClusterScaling, Config, FootprintStudy, FootprintSummary, KlineDataPoint,
    KlineTrades, NPoc, PointOfControl,
};
use data::chart::style::{BuySellColors, RatioColorScale, SingleColorStyle};
use data::chart::{Autoscale, KlineChartKind, ViewConfig};

use data::util::abbr_large_numbers;
use exchange::unit::{Price, PriceStep, Qty};
use exchange::{Kline, OpenInterest as OIData, TickerInfo, Trade, UnixMs};

use iced::task::Handle;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Event, Geometry, Path, Stroke};
use iced::{Alignment, Color, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use enum_map::EnumMap;
use std::time::Instant;

impl Chart for KlineChart {
    type IndicatorSelection = KlineIndicatorConfig;

    fn state(&self) -> &ViewState {
        &self.chart
    }

    fn mut_state(&mut self) -> &mut ViewState {
        &mut self.chart
    }

    fn invalidate_crosshair(&mut self) {
        self.chart.cache.clear_crosshair();
        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.clear_crosshair_caches());
    }

    fn invalidate_all(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(
        &'_ self,
        enabled: &[Self::IndicatorSelection],
    ) -> Vec<Element<'_, Message>> {
        let chart_state = self.state();
        let visible_region = chart_state.visible_region(chart_state.bounds.size());
        let (earliest, latest) = chart_state.interval_range(&visible_region);
        if earliest > latest {
            return vec![];
        }

        let data_labels_always_visible = self.visual_config.data_labels_always_visible;

        let market = chart_state.ticker_info.market_type();
        let mut elements = vec![];

        for selected_indicator in enabled {
            let kind = selected_indicator.kind();
            if !KlineIndicator::for_market(market).contains(&kind) {
                continue;
            }
            if let Some(indi) = self.indicators[kind].as_ref() {
                elements.push(indi.element(
                    chart_state,
                    data_labels_always_visible,
                    earliest..=latest,
                ));
            }
        }
        elements
    }

    fn visible_timerange(&self) -> Option<(u64, u64)> {
        let chart = self.state();
        let region = chart.visible_region(chart.bounds.size());

        if region.width == 0.0 {
            return None;
        }

        Some(chart.interval_range(&region))
    }

    fn interval_keys(&self) -> Option<Vec<u64>> {
        match &self.data_source {
            PlotData::TimeBased(_) => None,
            PlotData::TickBased(tick_aggr) => Some(
                tick_aggr
                    .datapoints
                    .iter()
                    .map(|dp| dp.kline.time.as_u64())
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
    indicators: EnumMap<KlineIndicator, Option<Box<dyn KlineIndicatorImpl>>>,
    fetching_trades: (bool, Option<Handle>),
    pub(crate) kind: KlineChartKind,
    request_handler: RequestHandler,
    study_configurator: study::Configurator<FootprintStudy>,
    last_tick: Instant,
    visual_config: Config,
}

impl KlineChart {
    pub fn new(
        layout: ViewConfig,
        basis: Basis,
        step: PriceStep,
        klines_raw: &[Kline],
        raw_trades: Vec<Trade>,
        enabled_indicators: &[KlineIndicatorConfig],
        ticker_info: TickerInfo,
        kind: &KlineChartKind,
        visual_config: Option<Config>,
    ) -> Self {
        let visual_config = visual_config.unwrap_or_default();

        match basis {
            Basis::Time(interval) => {
                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, klines_raw)
                    .with_trades(&raw_trades);

                let base_price_y = timeseries.base_price();
                let latest_x = timeseries
                    .latest_timestamp()
                    .map_or(0, |timestamp| timestamp.as_u64());
                let (scale_high, scale_low) = timeseries.price_scale({
                    match kind {
                        KlineChartKind::Footprint { .. } => 12,
                        KlineChartKind::Candles => 60,
                    }
                });

                let low_rounded = scale_low.round_to_side_step(true, step);
                let high_rounded = scale_high.round_to_side_step(false, step);

                let y_ticks = Price::steps_between_inclusive(low_rounded, high_rounded, step)
                    .map(|n| n.saturating_sub(1))
                    .unwrap_or(1)
                    .max(1) as f32;

                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 800.0 / y_ticks,
                    KlineChartKind::Candles => 200.0 / y_ticks,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    step.decimal_places(),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits.clone(),
                        autoscale: Some(Autoscale::FitToVisible),
                    },
                    cell_width,
                    cell_height,
                );
                chart.base_price_y = base_price_y;
                chart.latest_x = latest_x;

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

                let data_source = PlotData::TimeBased(timeseries);

                let mut indicators = EnumMap::default();
                for &config in enabled_indicators {
                    let mut indi = indicator::kline::make(config);
                    indi.rebuild_from_source(&data_source);
                    indicators[config.kind()] = Some(indi);
                }

                KlineChart {
                    chart,
                    visual_config,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                }
            }
            Basis::Tick(interval) => {
                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 90.0,
                    KlineChartKind::Candles => 8.0,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    step.decimal_places(),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits.clone(),
                        autoscale: Some(Autoscale::FitToVisible),
                    },
                    cell_width,
                    cell_height,
                );

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

                let data_source = PlotData::TickBased(TickAggr::new(interval, step, &raw_trades));

                let mut indicators = EnumMap::default();
                for &config in enabled_indicators {
                    let mut indi = indicator::kline::make(config);
                    indi.rebuild_from_source(&data_source);
                    indicators[config.kind()] = Some(indi);
                }

                KlineChart {
                    chart,
                    visual_config,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
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
                timeseries.insert_klines(&[*kline]);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(&[*kline], &self.data_source));

                let chart = self.mut_state();

                if kline.time.as_u64() > chart.latest_x {
                    chart.latest_x = kline.time.as_u64();
                }

                chart.last_price = Some(PriceInfoLabel::new(kline.close, kline.open));
            }
            PlotData::TickBased(_) => {}
        }
    }

    pub fn kind(&self) -> &KlineChartKind {
        &self.kind
    }

    fn fetch_missing_data(&mut self) -> Option<Action> {
        match &self.data_source {
            PlotData::TimeBased(timeseries) => {
                let timeframe_ms = timeseries.interval.to_milliseconds();

                if timeseries.datapoints.is_empty() {
                    let latest = chrono::Utc::now().timestamp_millis() as u64;
                    let earliest = latest.saturating_sub(450 * timeframe_ms);

                    let range = FetchRange::Kline(UnixMs::new(earliest), UnixMs::new(latest));
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                let (visible_earliest, visible_latest) = self.visible_timerange()?;
                let (kline_earliest, kline_latest) = timeseries.timerange();
                let visible_earliest_ms = UnixMs::new(visible_earliest);
                let visible_latest_ms = UnixMs::new(visible_latest);
                let visible_span = visible_latest.saturating_sub(visible_earliest);
                let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

                // priority 1, initial klines for visible range
                if visible_earliest_ms < kline_earliest {
                    let range = FetchRange::Kline(UnixMs::new(prefetch_earliest), kline_earliest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                // priority 2, trades
                if let KlineChartKind::Footprint { .. } = self.kind
                    && !self.fetching_trades.0
                    && is_trade_fetch_enabled()
                    && let Some((fetch_from, fetch_to)) =
                        timeseries.suggest_trade_fetch_range(visible_earliest_ms, visible_latest_ms)
                {
                    let range = FetchRange::Trades(fetch_from, fetch_to);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        self.fetching_trades = (true, None);
                        return Some(action);
                    }
                }

                // priority 3, indicators
                // (e.g. open interest needs external fetch as it's not derived from klines)
                let ctx = indicator::kline::FetchCtx {
                    main_chart: &self.chart,
                    timeframe: timeseries.interval,
                    visible_earliest: visible_earliest_ms,
                    kline_latest,
                    prefetch_earliest: UnixMs::new(prefetch_earliest),
                };
                for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
                    if let Some(range) = indi.fetch_range(&ctx)
                        && let Some(action) = request_fetch(&mut self.request_handler, range)
                    {
                        return Some(action);
                    }
                }

                // priority 4, missing klines & integrity check
                let check_earliest = UnixMs::new(prefetch_earliest).max(kline_earliest);
                let check_latest = visible_latest_ms.saturating_add(timeframe_ms);

                if let Some(missing_keys) =
                    timeseries.check_kline_integrity(check_earliest, check_latest)
                {
                    let latest = missing_keys
                        .iter()
                        .max()
                        .unwrap_or(&visible_latest_ms)
                        .saturating_add(timeframe_ms);
                    let earliest = missing_keys
                        .iter()
                        .min()
                        .unwrap_or(&visible_earliest_ms)
                        .saturating_sub(timeframe_ms);

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
        self.request_handler = RequestHandler::default();
        self.fetching_trades = (false, None);
    }

    pub fn raw_trades(&self) -> Vec<Trade> {
        self.raw_trades.clone()
    }

    pub fn set_handle(&mut self, handle: Handle) {
        self.fetching_trades.1 = Some(handle);
    }

    pub fn tick_size(&self) -> PriceStep {
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

    pub fn visual_config(&self) -> Config {
        self.visual_config
    }

    pub fn set_visual_config(&mut self, visual_config: Config) {
        self.visual_config = visual_config;
        self.chart.cache.clear_all();
        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.clear_all_caches());
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

    pub fn set_cluster_scaling(&mut self, new_scaling: ClusterScaling) {
        if let KlineChartKind::Footprint {
            ref mut scaling, ..
        } = self.kind
        {
            *scaling = new_scaling;
        }

        self.invalidate(None);
    }

    pub fn basis(&self) -> Basis {
        self.chart.basis
    }

    pub fn change_tick_size(&mut self, new_step: PriceStep) {
        let chart = self.mut_state();

        chart.cell_height *= (new_step.units as f32) / (chart.tick_size.units as f32);
        chart.tick_size = new_step;

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.change_tick_size(new_step, &self.raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.change_tick_size(new_step, &self.raw_trades);
            }
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_ticksize_change(&self.data_source));

        self.invalidate(None);
    }

    pub fn set_basis(&mut self, new_basis: Basis) -> Option<Action> {
        self.chart.last_price = None;
        self.chart.basis = new_basis;

        match new_basis {
            Basis::Time(interval) => {
                let step = self.chart.tick_size;
                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, &[]);
                self.data_source = PlotData::TimeBased(timeseries);
            }
            Basis::Tick(tick_count) => {
                let step = self.chart.tick_size;
                let tick_aggr = TickAggr::new(tick_count, step, &self.raw_trades);
                self.data_source = PlotData::TickBased(tick_aggr);
            }
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_basis_change(&self.data_source));

        self.reset_request_handler();
        self.invalidate(Some(Instant::now()))
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

    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        self.raw_trades.extend_from_slice(buffer);

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                let old_dp_len = tick_aggr.datapoints.len();
                tick_aggr.insert_trades(buffer);

                if let Some(last_dp) = tick_aggr.datapoints.last() {
                    self.chart.last_price =
                        Some(PriceInfoLabel::new(last_dp.kline.close, last_dp.kline.open));
                } else {
                    self.chart.last_price = None;
                }

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_trades(buffer, old_dp_len, &self.data_source));

                self.invalidate(None);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades_existing_buckets(buffer);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_trades(buffer, 0, &self.data_source));

                self.invalidate(None);
            }
        }
    }

    pub fn insert_raw_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.insert_trades(&raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades_existing_buckets(&raw_trades);
            }
        }

        self.raw_trades.extend_from_slice(&raw_trades);

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_insert_trades(&raw_trades, 0, &self.data_source));

        if is_batches_done {
            self.fetching_trades = (false, None);
        }

        self.invalidate(None);
    }

    pub fn insert_hist_klines(&mut self, req_id: uuid::Uuid, klines_raw: &[Kline]) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(klines_raw);
                timeseries.insert_trades_existing_buckets(&self.raw_trades);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(klines_raw, &self.data_source));

                if klines_raw.is_empty() {
                    self.request_handler
                        .mark_failed(req_id, "No data received".to_string());
                } else {
                    self.request_handler.mark_completed(req_id);
                }
                self.invalidate(None);
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

        if let Some(indi) = self.indicators[KlineIndicator::OpenInterest].as_mut() {
            indi.on_open_interest(oi_data);
        }
    }

    fn calc_qty_scales(
        &self,
        earliest: u64,
        latest: u64,
        highest: Price,
        lowest: Price,
        step: PriceStep,
        cluster_kind: ClusterKind,
    ) -> f64 {
        let rounded_highest = highest.round_to_side_step(false, step).add_steps(1, step);
        let rounded_lowest = lowest.round_to_side_step(true, step).add_steps(-1, step);

        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries
                .max_qty_ts_range(
                    cluster_kind,
                    UnixMs::new(earliest),
                    UnixMs::new(latest),
                    rounded_highest,
                    rounded_lowest,
                )
                .to_f64(),
            PlotData::TickBased(tick_aggr) => {
                let earliest = earliest as usize;
                let latest = latest as usize;

                tick_aggr
                    .max_qty_idx_range(
                        cluster_kind,
                        earliest,
                        latest,
                        rounded_highest,
                        rounded_lowest,
                    )
                    .to_f64()
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
                        let chart_height = chart.bounds.height;
                        let tick_size = chart.tick_size.to_f32_lossy();

                        if chart_height > f32::EPSILON && tick_size > 0.0 {
                            let (fit_lowest, fit_highest) =
                                if let KlineChartKind::Footprint { .. } = self.kind {
                                    if let Some((footprint_low, footprint_high)) = self
                                        .data_source
                                        .visible_footprint_price_range(start_interval, end_interval)
                                    {
                                        let half_tick = tick_size * 0.5;
                                        (
                                            footprint_low.to_f32_lossy() - half_tick,
                                            footprint_high.to_f32_lossy() + half_tick,
                                        )
                                    } else {
                                        (lowest, highest)
                                    }
                                } else {
                                    (lowest, highest)
                                };

                            let visible_span = (fit_highest - fit_lowest).max(tick_size);
                            let base_padding = visible_span * 0.05; // 5% padding on top and bottom

                            let mut top_padding = base_padding;
                            let mut bottom_padding = base_padding;

                            if let KlineChartKind::Footprint { clusters, .. } = self.kind {
                                let provisional_span = visible_span + top_padding + bottom_padding;
                                if provisional_span > 0.0 {
                                    let provisional_cell_height =
                                        (chart_height * tick_size) / provisional_span;

                                    let outer_padding = price_padding_from_pixels(
                                        provisional_cell_height,
                                        tick_size,
                                    );

                                    top_padding += outer_padding;
                                    bottom_padding += outer_padding;

                                    bottom_padding = bottom_padding.max(footprint_summary_padding(
                                        provisional_cell_height,
                                        chart.scaling,
                                        chart.cell_width,
                                        tick_size,
                                        clusters,
                                    ));
                                }
                            }

                            let padded_span = visible_span + top_padding + bottom_padding;
                            if padded_span > 0.0 {
                                chart.cell_height = (chart_height * tick_size) / padded_span;
                                chart.base_price_y = Price::from_f32(fit_highest + top_padding);
                                chart.translation.y = -chart_height / 2.0;
                            }
                        }
                    }
                }
            }
        }

        chart.cache.clear_all();
        for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
            indi.clear_all_caches();
        }

        if let Some(t) = now {
            self.last_tick = t;
            self.fetch_missing_data()
        } else {
            None
        }
    }

    pub fn sync_indicator_configs(&mut self, configs: &[KlineIndicatorConfig]) {
        let prev_indi_count = self.indicators.values().filter(|v| v.is_some()).count();

        for kind in [
            KlineIndicator::Volume,
            KlineIndicator::BarAnalysis,
            KlineIndicator::CumulativeDelta,
            KlineIndicator::OpenInterest,
        ] {
            if matches!(self.kind, KlineChartKind::Candles)
                && matches!(kind, KlineIndicator::BarAnalysis)
            {
                self.indicators[kind] = None;
                continue;
            }
            let config = configs.iter().find(|cfg| cfg.kind() == kind);

            match (self.indicators[kind].as_mut(), config) {
                (Some(indicator), Some(config)) => {
                    indicator.apply_config(config, &self.data_source);
                }
                (None, Some(config)) => {
                    let mut indicator = indicator::kline::make(*config);
                    indicator.rebuild_from_source(&self.data_source);
                    self.indicators[kind] = Some(indicator);
                }
                (Some(_), None) => {
                    self.indicators[kind] = None;
                }
                (None, None) => {}
            }
        }

        if let Some(main_split) = self.chart.layout.splits.first() {
            let current_indi_count = configs.len();
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

            let price_to_y = |price| chart.price_to_y(price);
            let interval_to_x = |interval| chart.interval_to_x(interval);

            match &self.kind {
                KlineChartKind::Footprint {
                    clusters,
                    scaling,
                    studies,
                } => {
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

                    let text_size =
                        footprint_cluster_text_size(cell_height_unscaled, cell_width_unscaled);

                    let candle_width = 0.1 * chart.cell_width;
                    let content_spacing = ContentGaps::from_view(candle_width, chart.scaling);

                    let styles = FootprintStyles::from_studies(studies);

                    let show_text = should_show_text(
                        cell_height_unscaled,
                        cell_width_unscaled,
                        footprint_cluster_min_width(*clusters),
                    );

                    if *clusters != ClusterKind::Table {
                        draw_all_npocs(
                            &self.data_source,
                            frame,
                            price_to_y,
                            interval_to_x,
                            candle_width,
                            chart.cell_width,
                            chart.cell_height,
                            studies,
                            earliest,
                            latest,
                            *clusters,
                            content_spacing,
                            styles.imbalance.is_some(),
                        );
                    }

                    let draw_ctx = FootprintDrawCtx {
                        price_to_y: &price_to_y,
                        palette,
                        text_size,
                        step: self.tick_size(),
                        show_text,
                        show_summary: self.visual_config.show_footprint_summary,
                        show_table_candle: self.visual_config.show_footprint_table_candle,
                        styles,
                        spacing: content_spacing,
                    };

                    render_data_source(
                        &self.data_source,
                        frame,
                        earliest,
                        latest,
                        interval_to_x,
                        |frame, x_position, kline, trades| {
                            let cluster_scaling =
                                effective_cluster_qty(*scaling, max_cluster_qty, trades, *clusters);

                            draw_clusters(
                                frame,
                                &draw_ctx,
                                x_position,
                                chart.cell_width,
                                chart.cell_height,
                                candle_width,
                                cluster_scaling,
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
            let visible_region = chart.visible_region(bounds_size);
            let visible_range = chart.interval_range(&visible_region);

            if let Some(cursor_position) = cursor.position_in(bounds) {
                let (_, rounded_aggregation) =
                    chart.draw_crosshair(frame, theme, bounds_size, cursor_position, interaction);

                draw_crosshair_tooltip(
                    &self.data_source,
                    &chart.ticker_info,
                    frame,
                    palette,
                    chart.basis,
                    Some(rounded_aggregation),
                    visible_range,
                );
            } else if self.visual_config.data_labels_always_visible {
                draw_crosshair_tooltip(
                    &self.data_source,
                    &chart.ticker_info,
                    frame,
                    palette,
                    chart.basis,
                    None,
                    visible_range,
                );
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
    price_to_y: impl Fn(Price) -> f32,
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
    price_to_y: impl Fn(Price) -> f32,
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
                .range(UnixMs::new(earliest)..=UnixMs::new(latest))
                .for_each(|(timestamp, dp)| {
                    let x_position = interval_to_x(timestamp.as_u64());

                    draw_fn(frame, x_position, &dp.kline, &dp.footprint);
                });
        }
    }
}

fn draw_all_npocs(
    data_source: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(Price) -> f32,
    interval_to_x: impl Fn(u64) -> f32,
    candle_width: f32,
    cell_width: f32,
    cell_height: f32,
    studies: &[FootprintStudy],
    visible_earliest: u64,
    visible_latest: u64,
    cluster_kind: ClusterKind,
    spacing: ContentGaps,
    imb_study_on: bool,
) {
    let Some((lookback, colors)) = studies.iter().find_map(|study| {
        if let FootprintStudy::NPoC { lookback, colors } = study {
            Some((*lookback, *colors))
        } else {
            None
        }
    }) else {
        return;
    };

    let line_height = cell_height.min(1.0);

    let bar_width_factor: f32 = 0.9;
    let inset = (cell_width * (1.0 - bar_width_factor)) / 2.0;

    let candle_lane_factor: f32 = match cluster_kind {
        ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => 0.25,
        ClusterKind::BidAsk | ClusterKind::Table => 1.0,
    };

    let start_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk | ClusterKind::Table => {
                cell_center_x + (candle_width / 2.0) + spacing.candle_to_cluster
            }
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
                let content_left = (cell_center_x - (cell_width / 2.0)) + inset;
                let candle_lane_left = content_left
                    + if imb_study_on {
                        candle_width + spacing.marker_to_candle
                    } else {
                        0.0
                    };
                candle_lane_left + candle_width * candle_lane_factor + spacing.candle_to_cluster
            }
        }
    };

    let wick_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk | ClusterKind::Table => cell_center_x, // not used for BidAsk/Table clustering
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
                let content_left = (cell_center_x - (cell_width / 2.0)) + inset;
                let candle_lane_left = content_left
                    + if imb_study_on {
                        candle_width + spacing.marker_to_candle
                    } else {
                        0.0
                    };
                candle_lane_left + (candle_width * candle_lane_factor) / 2.0
                    - (spacing.candle_to_cluster * 0.5)
            }
        }
    };

    let end_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk | ClusterKind::Table => {
                cell_center_x - (candle_width / 2.0) - spacing.candle_to_cluster
            }
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => wick_x_for(cell_center_x),
        }
    };

    let rightmost_cell_center_x = {
        let earliest_x = interval_to_x(visible_earliest);
        let latest_x = interval_to_x(visible_latest);
        if earliest_x > latest_x {
            earliest_x
        } else {
            latest_x
        }
    };

    let mut draw_the_line = |interval: u64, poc: &PointOfControl| {
        let start_x = start_x_for(interval_to_x(interval));

        let (line_width, color) = match poc.status {
            NPoc::Naked => {
                let end_x = end_x_for(rightmost_cell_center_x);
                let line_width = end_x - start_x;
                if line_width.abs() <= cell_width {
                    return;
                }
                (line_width, colors.naked_color)
            }
            NPoc::Filled { at } => {
                let end_x = end_x_for(interval_to_x(at));
                let line_width = end_x - start_x;
                if line_width.abs() <= cell_width {
                    return;
                }
                (line_width, colors.filled_color)
            }
            _ => return,
        };

        frame.fill_rectangle(
            Point::new(start_x, price_to_y(poc.price) - line_height / 2.0),
            Size::new(line_width, line_height),
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
                    dp.footprint
                        .poc
                        .as_ref()
                        .map(|poc| (timestamp.as_u64(), poc))
                })
                .for_each(|(interval, poc)| draw_the_line(interval, poc));
        }
    }
}

fn effective_cluster_qty(
    scaling: ClusterScaling,
    visible_max: f64,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
) -> f64 {
    let individual_max = match cluster_kind {
        ClusterKind::BidAsk | ClusterKind::Table => footprint
            .trades
            .values()
            .map(|group| group.buy_qty.max(group.sell_qty))
            .max()
            .unwrap_or_default(),
        ClusterKind::DeltaProfile => footprint
            .trades
            .values()
            .map(|group| group.buy_qty.abs_diff(group.sell_qty))
            .max()
            .unwrap_or_default(),
        ClusterKind::VolumeProfile => footprint
            .trades
            .values()
            .map(|group| group.buy_qty + group.sell_qty)
            .max()
            .unwrap_or_default(),
    };

    match scaling {
        ClusterScaling::VisibleRange => Qty::scale_or_one(visible_max),
        ClusterScaling::Datapoint => individual_max.to_scale_or_one(),
        ClusterScaling::Hybrid { weight } => {
            let w = weight.clamp(0.0, 1.0) as f64;
            Qty::scale_or_one(visible_max * w + individual_max.to_f64() * (1.0 - w))
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FootprintStyles {
    poc: Option<SingleColorStyle>,
    imbalance: Option<ImbalanceStyle>,
}

impl FootprintStyles {
    fn from_studies(studies: &[FootprintStudy]) -> Self {
        let poc = studies.iter().find_map(|study| {
            if let FootprintStudy::PointOfControl { style } = study {
                Some(*style)
            } else {
                None
            }
        });

        let imbalance = studies.iter().find_map(|study| {
            if let FootprintStudy::Imbalance {
                threshold,
                scale,
                ignore_zeros,
                colors,
            } = study
            {
                Some(ImbalanceStyle {
                    threshold: *threshold,
                    scale: *scale,
                    ignore_zeros: *ignore_zeros,
                    colors: *colors,
                })
            } else {
                None
            }
        });

        Self { poc, imbalance }
    }
}

#[derive(Clone, Copy, Debug)]
struct ImbalanceStyle {
    threshold: usize,
    scale: RatioColorScale,
    ignore_zeros: bool,
    colors: BuySellColors,
}

struct FootprintDrawCtx<'a, F>
where
    F: Fn(Price) -> f32,
{
    price_to_y: &'a F,
    palette: &'a Extended,
    text_size: f32,
    step: PriceStep,
    show_text: bool,
    show_summary: bool,
    show_table_candle: bool,
    styles: FootprintStyles,
    spacing: ContentGaps,
}

impl<F> FootprintDrawCtx<'_, F>
where
    F: Fn(Price) -> f32,
{
    fn text_color(&self) -> Color {
        self.palette.background.weakest.text
    }

    fn candle_poc_color(&self) -> Option<Color> {
        self.styles.poc.map(|style| style.color)
    }

    fn imbalance(&self) -> Option<ImbalanceStyle> {
        self.styles.imbalance
    }
}

#[derive(Clone, Copy, Debug)]
struct TableArea {
    table_left: f32,
    table_right: f32,
}

impl TableArea {
    fn new<F>(
        frame: &mut canvas::Frame,
        ctx: &FootprintDrawCtx<'_, F>,
        content_left: f32,
        content_right: f32,
        candle_width: f32,
        cell_height: f32,
        kline: &Kline,
        footprint: &KlineTrades,
    ) -> Self
    where
        F: Fn(Price) -> f32,
    {
        let (table_left, table_right) = if ctx.show_table_candle {
            let candle_center_x = content_left + (candle_width / 2.0);

            draw_footprint_kline(
                frame,
                ctx.price_to_y,
                candle_center_x,
                candle_width,
                kline,
                ctx.palette,
            );
            draw_candle_poc_marker(
                frame,
                ctx.price_to_y,
                candle_center_x,
                candle_width,
                cell_height,
                footprint,
                ctx.candle_poc_color(),
            );

            (
                (content_left + candle_width + ctx.spacing.candle_to_cluster).min(content_right),
                content_right,
            )
        } else {
            (content_left, content_right)
        };

        Self {
            table_left,
            table_right,
        }
    }

    fn width(&self) -> f32 {
        (self.table_right - self.table_left).max(0.0)
    }
}

fn draw_clusters<F>(
    frame: &mut canvas::Frame,
    ctx: &FootprintDrawCtx<'_, F>,
    x_position: f32,
    cell_width: f32,
    cell_height: f32,
    candle_width: f32,
    max_cluster_qty: f64,
    kline: &Kline,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
) where
    F: Fn(Price) -> f32,
{
    let text_color = ctx.text_color();
    let palette = ctx.palette;
    let text_size = ctx.text_size;
    let show_text = ctx.show_text;
    let imbalance = ctx.imbalance();
    let candle_poc_color = ctx.candle_poc_color();

    let bar_width_factor: f32 = 0.9;
    let inset = (cell_width * (1.0 - bar_width_factor)) / 2.0;

    let cell_left = x_position - (cell_width / 2.0);
    let content_left = cell_left + inset;
    let content_right = x_position + (cell_width / 2.0) - inset;

    match cluster_kind {
        ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
            let area = ProfileArea::new(
                content_left,
                content_right,
                candle_width,
                ctx.spacing,
                imbalance.is_some(),
            );
            let bar_alpha = if show_text { 0.25 } else { 1.0 };

            for (price, group) in &footprint.trades {
                let buy_qty = group.buy_qty.to_f64();
                let sell_qty = group.sell_qty.to_f64();
                let y = (ctx.price_to_y)(*price);

                match cluster_kind {
                    ClusterKind::VolumeProfile => {
                        super::draw_volume_bar(
                            frame,
                            area.bars_left,
                            y,
                            buy_qty,
                            sell_qty,
                            max_cluster_qty,
                            area.bars_width,
                            cell_height,
                            palette.success.base.color,
                            palette.danger.base.color,
                            bar_alpha,
                            true,
                        );

                        if show_text {
                            draw_cluster_text(
                                frame,
                                &abbr_large_numbers(f64::from(group.total_qty())),
                                Point::new(area.bars_left, y),
                                text_size,
                                text_color,
                                Alignment::Start,
                                Alignment::Center,
                            );
                        }
                    }
                    ClusterKind::DeltaProfile => {
                        let delta = group.delta_qty().to_f64();
                        if show_text {
                            draw_cluster_text(
                                frame,
                                &abbr_large_numbers(delta),
                                Point::new(area.bars_left, y),
                                text_size,
                                text_color,
                                Alignment::Start,
                                Alignment::Center,
                            );
                        }

                        let bar_width = (delta.abs() / max_cluster_qty) as f32 * area.bars_width;
                        if bar_width > 0.0 {
                            let color = if delta >= 0.0 {
                                palette.success.base.color.scale_alpha(bar_alpha)
                            } else {
                                palette.danger.base.color.scale_alpha(bar_alpha)
                            };
                            frame.fill_rectangle(
                                Point::new(area.bars_left, y - (cell_height / 2.0)),
                                Size::new(bar_width, cell_height),
                                color,
                            );
                        }
                    }
                    _ => {}
                }

                if let Some(imbalance) = imbalance {
                    let higher_price = price.add_steps(1, ctx.step);

                    let rect_w = ((area.imb_marker_width - 1.0) / 2.0).max(1.0);
                    let buyside_x = area.imb_marker_left + area.imb_marker_width - rect_w;
                    let sellside_x =
                        area.imb_marker_left + area.imb_marker_width - (2.0 * rect_w) - 1.0;

                    draw_imbalance_markers(
                        frame,
                        ctx.price_to_y,
                        footprint,
                        *price,
                        sell_qty,
                        higher_price,
                        imbalance,
                        cell_height,
                        buyside_x,
                        sellside_x,
                        rect_w,
                    );
                }
            }

            draw_footprint_kline(
                frame,
                ctx.price_to_y,
                area.candle_center_x,
                candle_width,
                kline,
                palette,
            );
            draw_candle_poc_marker(
                frame,
                ctx.price_to_y,
                area.candle_center_x,
                candle_width,
                cell_height,
                footprint,
                candle_poc_color,
            );
        }
        ClusterKind::Table => {
            let area = TableArea::new(
                frame,
                ctx,
                content_left,
                content_right,
                candle_width,
                cell_height,
                kline,
                footprint,
            );
            let table_left = area.table_left;
            let table_right = area.table_right;
            let table_width = area.width();
            let half_width = table_width / 2.0;
            let cell_border = 1.0;
            let grid_color = palette.background.weakest.text.scale_alpha(0.32);

            for (price, group) in &footprint.trades {
                let buy_qty = group.buy_qty.to_f64();
                let sell_qty = group.sell_qty.to_f64();
                let y = (ctx.price_to_y)(*price);
                let row_top = y - (cell_height / 2.0);

                frame.fill_rectangle(
                    Point::new(table_left, row_top),
                    Size::new(half_width, cell_height),
                    palette.danger.base.color.scale_alpha(0.14),
                );
                frame.fill_rectangle(
                    Point::new(table_left + half_width, row_top),
                    Size::new(half_width, cell_height),
                    palette.success.base.color.scale_alpha(0.14),
                );

                if let Some(poc_color) = candle_poc_color
                    && footprint
                        .poc
                        .as_ref()
                        .is_some_and(|poc| poc.price == *price)
                {
                    frame.fill_rectangle(
                        Point::new(table_left, row_top),
                        Size::new(table_width, cell_height),
                        poc_color.scale_alpha(0.55),
                    );
                }

                let mut sell_text_color = text_color;
                let mut buy_text_color = text_color;

                if let Some(imbalance) = imbalance {
                    if let Some(alpha) = sell_imbalance_alpha(
                        footprint,
                        *price,
                        sell_qty,
                        ctx.step,
                        imbalance.threshold,
                        imbalance.scale,
                        imbalance.ignore_zeros,
                    ) {
                        frame.fill_rectangle(
                            Point::new(table_left, row_top),
                            Size::new(half_width, cell_height),
                            imbalance.colors.sell_color.scale_alpha(alpha.max(0.65)),
                        );
                        sell_text_color = palette.danger.base.color;
                    }

                    if let Some(alpha) = buy_imbalance_alpha(
                        footprint,
                        *price,
                        buy_qty,
                        ctx.step,
                        imbalance.threshold,
                        imbalance.scale,
                        imbalance.ignore_zeros,
                    ) {
                        frame.fill_rectangle(
                            Point::new(table_left + half_width, row_top),
                            Size::new(half_width, cell_height),
                            imbalance.colors.buy_color.scale_alpha(alpha.max(0.65)),
                        );
                        buy_text_color = palette.danger.base.color;
                    }
                }

                // Table grid: draw full cell borders so the footprint table reads as a real table.
                frame.fill_rectangle(
                    Point::new(table_left, row_top),
                    Size::new(table_width, cell_border),
                    grid_color,
                );
                frame.fill_rectangle(
                    Point::new(table_left, row_top + cell_height - cell_border),
                    Size::new(table_width, cell_border),
                    grid_color,
                );
                frame.fill_rectangle(
                    Point::new(table_left, row_top),
                    Size::new(cell_border, cell_height),
                    grid_color,
                );
                frame.fill_rectangle(
                    Point::new(table_left + half_width, row_top),
                    Size::new(cell_border, cell_height),
                    grid_color,
                );
                frame.fill_rectangle(
                    Point::new(table_right - cell_border, row_top),
                    Size::new(cell_border, cell_height),
                    grid_color,
                );

                if show_text {
                    draw_cluster_text(
                        frame,
                        &abbr_large_numbers(sell_qty),
                        Point::new(table_left + half_width - 3.0, y),
                        text_size,
                        sell_text_color,
                        Alignment::End,
                        Alignment::Center,
                    );
                    draw_cluster_text(
                        frame,
                        &abbr_large_numbers(buy_qty),
                        Point::new(table_left + half_width + 3.0, y),
                        text_size,
                        buy_text_color,
                        Alignment::Start,
                        Alignment::Center,
                    );
                }
            }
        }
        ClusterKind::BidAsk => {
            let area = BidAskArea::new(
                x_position,
                content_left,
                content_right,
                candle_width,
                ctx.spacing,
            );

            let bar_alpha = if show_text { 0.25 } else { 1.0 };

            let imb_marker_reserve = if imbalance.is_some() {
                ((area.imb_marker_width - 1.0) / 2.0).max(1.0)
            } else {
                0.0
            };

            let right_max_x =
                area.bid_area_right - imb_marker_reserve - (2.0 * ctx.spacing.marker_to_bars);
            let right_area_width = (right_max_x - area.bid_area_left).max(0.0);

            let left_min_x =
                area.ask_area_left + imb_marker_reserve + (2.0 * ctx.spacing.marker_to_bars);
            let left_area_width = (area.ask_area_right - left_min_x).max(0.0);

            for (price, group) in &footprint.trades {
                let buy_qty = group.buy_qty.to_f64();
                let sell_qty = group.sell_qty.to_f64();
                let y = (ctx.price_to_y)(*price);

                if buy_qty > 0.0 && right_area_width > 0.0 {
                    if show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(buy_qty),
                            Point::new(area.bid_area_left, y),
                            text_size,
                            text_color,
                            Alignment::Start,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (buy_qty / max_cluster_qty) as f32 * right_area_width;
                    if bar_width > 0.0 {
                        frame.fill_rectangle(
                            Point::new(area.bid_area_left, y - (cell_height / 2.0)),
                            Size::new(bar_width, cell_height),
                            palette.success.base.color.scale_alpha(bar_alpha),
                        );
                    }
                }
                if sell_qty > 0.0 && left_area_width > 0.0 {
                    if show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(sell_qty),
                            Point::new(area.ask_area_right, y),
                            text_size,
                            text_color,
                            Alignment::End,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (sell_qty / max_cluster_qty) as f32 * left_area_width;
                    if bar_width > 0.0 {
                        frame.fill_rectangle(
                            Point::new(area.ask_area_right, y - (cell_height / 2.0)),
                            Size::new(-bar_width, cell_height),
                            palette.danger.base.color.scale_alpha(bar_alpha),
                        );
                    }
                }

                if let Some(imbalance) = imbalance
                    && area.imb_marker_width > 0.0
                {
                    let higher_price = price.add_steps(1, ctx.step);

                    let rect_width = ((area.imb_marker_width - 1.0) / 2.0).max(1.0);

                    let buyside_x = area.bid_area_right - rect_width - ctx.spacing.marker_to_bars;
                    let sellside_x = area.ask_area_left + ctx.spacing.marker_to_bars;

                    draw_imbalance_markers(
                        frame,
                        ctx.price_to_y,
                        footprint,
                        *price,
                        sell_qty,
                        higher_price,
                        imbalance,
                        cell_height,
                        buyside_x,
                        sellside_x,
                        rect_width,
                    );
                }
            }

            draw_footprint_kline(
                frame,
                ctx.price_to_y,
                area.candle_center_x,
                candle_width,
                kline,
                palette,
            );
            draw_candle_poc_marker(
                frame,
                ctx.price_to_y,
                area.candle_center_x,
                candle_width,
                cell_height,
                footprint,
                candle_poc_color,
            );
        }
    }

    if show_text && ctx.show_summary {
        draw_footprint_summary(frame, ctx, x_position, cell_height, kline, footprint);
    }
}

fn draw_footprint_summary<F>(
    frame: &mut canvas::Frame,
    ctx: &FootprintDrawCtx<'_, F>,
    x_position: f32,
    cell_height: f32,
    kline: &Kline,
    footprint: &KlineTrades,
) where
    F: Fn(Price) -> f32,
{
    let Some(summary) = FootprintSummary::from_trades(footprint) else {
        return;
    };

    let summary_y = (ctx.price_to_y)(kline.low) + cell_height * 1.5;
    let line_spacing = ctx.text_size * 1.2;
    let small_text_size = ctx.text_size * 0.9;
    let total_vol_f = summary.total.to_f64();
    let total_delta_f = summary.delta.to_f64();
    let delta_color = if summary.delta >= Qty::ZERO {
        ctx.palette.success.base.color
    } else {
        ctx.palette.danger.base.color
    };

    let mut next_y = summary_y;

    draw_cluster_text(
        frame,
        &format!("V: {}", abbr_large_numbers(total_vol_f)),
        Point::new(x_position, next_y),
        small_text_size,
        ctx.palette.background.weakest.text,
        Alignment::Center,
        Alignment::Start,
    );
    next_y += line_spacing;

    draw_cluster_text(
        frame,
        &format!("Δ: {}", abbr_large_numbers(total_delta_f)),
        Point::new(x_position, next_y),
        small_text_size,
        delta_color,
        Alignment::Center,
        Alignment::Start,
    );
    next_y += line_spacing;

    draw_cluster_text(
        frame,
        &format!("Δ%: {:+.1}%", summary.delta_pct),
        Point::new(x_position, next_y),
        small_text_size,
        delta_color,
        Alignment::Center,
        Alignment::Start,
    );
}

fn imbalance_alpha(
    dominant_qty: f64,
    opposite_qty: f64,
    threshold: usize,
    scale: RatioColorScale,
) -> Option<f32> {
    let required_qty = opposite_qty * threshold as f64 / 100.0;

    if required_qty <= 0.0 {
        return (dominant_qty > 0.0).then_some(1.0);
    }

    if dominant_qty > required_qty {
        Some(scale.alpha_from_ratio(dominant_qty / required_qty))
    } else {
        None
    }
}

fn buy_imbalance_alpha(
    footprint: &KlineTrades,
    price: Price,
    buy_qty: f64,
    step: PriceStep,
    threshold: usize,
    scale: RatioColorScale,
    ignore_zeros: bool,
) -> Option<f32> {
    let lower_price = price.add_steps(-1, step);
    let diagonal_sell_qty = footprint
        .trades
        .get(&lower_price)
        .map(|group| group.sell_qty.to_f64())
        .unwrap_or_default();

    if ignore_zeros && (buy_qty <= 0.0 || diagonal_sell_qty <= 0.0) {
        return None;
    }

    imbalance_alpha(buy_qty, diagonal_sell_qty, threshold, scale)
}

fn sell_imbalance_alpha(
    footprint: &KlineTrades,
    price: Price,
    sell_qty: f64,
    step: PriceStep,
    threshold: usize,
    scale: RatioColorScale,
    ignore_zeros: bool,
) -> Option<f32> {
    let higher_price = price.add_steps(1, step);
    let diagonal_buy_qty = footprint
        .trades
        .get(&higher_price)
        .map(|group| group.buy_qty.to_f64())
        .unwrap_or_default();

    if ignore_zeros && (sell_qty <= 0.0 || diagonal_buy_qty <= 0.0) {
        return None;
    }

    imbalance_alpha(sell_qty, diagonal_buy_qty, threshold, scale)
}

fn draw_candle_poc_marker(
    frame: &mut canvas::Frame,
    price_to_y: &impl Fn(Price) -> f32,
    x_position: f32,
    candle_width: f32,
    cell_height: f32,
    footprint: &KlineTrades,
    color: Option<Color>,
) {
    let (Some(poc), Some(color)) = (footprint.poc.as_ref(), color) else {
        return;
    };

    let y = price_to_y(poc.price);
    let marker_width = (candle_width * 0.45).max(2.0);
    let marker_height = cell_height.clamp(1.0, 3.0);

    frame.fill_rectangle(
        Point::new(x_position - (marker_width / 2.0), y - (marker_height / 2.0)),
        Size::new(marker_width, marker_height),
        color,
    );
}

fn draw_imbalance_markers(
    frame: &mut canvas::Frame,
    price_to_y: &impl Fn(Price) -> f32,
    footprint: &KlineTrades,
    price: Price,
    sell_qty: f64,
    higher_price: Price,
    imbalance: ImbalanceStyle,
    cell_height: f32,
    buyside_x: f32,
    sellside_x: f32,
    rect_width: f32,
) {
    if imbalance.ignore_zeros && sell_qty <= 0.0 {
        return;
    }

    if let Some(group) = footprint.trades.get(&higher_price) {
        let diagonal_buy_qty = group.buy_qty.to_f64();

        if imbalance.ignore_zeros && diagonal_buy_qty <= 0.0 {
            return;
        }

        let rect_height = cell_height / 2.0;

        if diagonal_buy_qty >= sell_qty {
            let required_qty = sell_qty * imbalance.threshold as f64 / 100.0;
            if diagonal_buy_qty > required_qty {
                let ratio = diagonal_buy_qty / required_qty;
                let alpha = imbalance.scale.alpha_from_ratio(ratio);

                let y = price_to_y(higher_price);
                frame.fill_rectangle(
                    Point::new(buyside_x, y - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    imbalance.colors.buy_color.scale_alpha(alpha),
                );
            }
        } else {
            let required_qty = diagonal_buy_qty * imbalance.threshold as f64 / 100.0;
            if sell_qty > required_qty {
                let ratio = sell_qty / required_qty;
                let alpha = imbalance.scale.alpha_from_ratio(ratio);

                let y = price_to_y(price);
                frame.fill_rectangle(
                    Point::new(sellside_x, y - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    imbalance.colors.sell_color.scale_alpha(alpha),
                );
            }
        }
    }
}

impl ContentGaps {
    fn from_view(candle_width: f32, scaling: f32) -> Self {
        let px = |p: f32| p / scaling;
        let base = (candle_width * 0.2).max(px(2.0));
        Self {
            marker_to_candle: base,
            candle_to_cluster: base,
            marker_to_bars: px(2.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ContentGaps {
    /// Space between imb. markers candle body
    marker_to_candle: f32,
    /// Space between candle body and clusters
    candle_to_cluster: f32,
    /// Inner space reserved between imb. markers and clusters (used for BidAsk)
    marker_to_bars: f32,
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
    ticker_info: &TickerInfo,
    frame: &mut canvas::Frame,
    palette: &Extended,
    basis: Basis,
    at_interval: Option<u64>,
    visible_range: (u64, u64),
) {
    let (visible_earliest, visible_latest) = visible_range;

    let kline_opt = match (data, at_interval) {
        (PlotData::TimeBased(timeseries), Some(at_interval)) => {
            let in_visible = at_interval >= visible_earliest && at_interval <= visible_latest;

            timeseries
                .datapoints
                .get(&UnixMs::new(at_interval))
                .map(|dp| &dp.kline)
                .or_else(|| {
                    if in_visible {
                        let search_end = at_interval.min(visible_latest);
                        timeseries
                            .datapoints
                            .range(UnixMs::new(visible_earliest)..=UnixMs::new(search_end))
                            .next_back()
                            .map(|(_, dp)| &dp.kline)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    let right_of_latest = match basis {
                        Basis::Time(_) => at_interval > visible_latest,
                        Basis::Tick(_) => at_interval < visible_earliest,
                    };

                    if right_of_latest {
                        timeseries
                            .datapoints
                            .range(UnixMs::new(visible_earliest)..=UnixMs::new(visible_latest))
                            .next_back()
                            .map(|(_, dp)| &dp.kline)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    let (last_time, dp) = timeseries.datapoints.last_key_value()?;
                    (at_interval > last_time.as_u64()).then_some(&dp.kline)
                })
        }
        (PlotData::TickBased(tick_aggr), Some(at_interval)) => {
            let kline_at = |interval: u64| {
                let index = (interval / u64::from(tick_aggr.interval.0)) as usize;
                (index < tick_aggr.datapoints.len())
                    .then(|| &tick_aggr.datapoints[tick_aggr.datapoints.len() - 1 - index].kline)
            };

            let in_visible = at_interval >= visible_earliest && at_interval <= visible_latest;

            kline_at(at_interval).or_else(|| {
                let right_of_latest = match basis {
                    Basis::Time(_) => at_interval > visible_latest,
                    Basis::Tick(_) => at_interval < visible_earliest,
                };

                if in_visible || right_of_latest {
                    kline_at(visible_earliest)
                } else {
                    None
                }
            })
        }
        (PlotData::TimeBased(timeseries), None) => timeseries
            .datapoints
            .last_key_value()
            .map(|(_, dp)| &dp.kline),
        (PlotData::TickBased(tick_aggr), None) => tick_aggr.datapoints.last().map(|dp| &dp.kline),
    };

    if let Some(kline) = kline_opt {
        let change_pct = ((kline.close - kline.open) / kline.open * 100.0) as f32;
        let change_color = if change_pct >= 0.0 {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };

        let base_color = palette.background.base.text;
        let precision = ticker_info.min_ticksize;

        let segments = [
            ("O", base_color, false),
            (&kline.open.to_string(precision), change_color, true),
            ("H", base_color, false),
            (&kline.high.to_string(precision), change_color, true),
            ("L", base_color, false),
            (&kline.low.to_string(precision), change_color, true),
            ("C", base_color, false),
            (&kline.close.to_string(precision), change_color, true),
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
                size: iced::Pixels(crate::style::text_size::BODY),
                color: seg_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            x += text.len() as f32 * 8.0;
            x += if is_value { 6.0 } else { 2.0 };
        }
    }
}

struct ProfileArea {
    imb_marker_left: f32,
    imb_marker_width: f32,
    bars_left: f32,
    bars_width: f32,
    candle_center_x: f32,
}

impl ProfileArea {
    fn new(
        content_left: f32,
        content_right: f32,
        candle_width: f32,
        gaps: ContentGaps,
        has_imbalance: bool,
    ) -> Self {
        let candle_lane_left = if has_imbalance {
            content_left + candle_width + gaps.marker_to_candle
        } else {
            content_left
        };
        let candle_lane_width = candle_width * 0.25;

        let bars_left = candle_lane_left + candle_lane_width + gaps.candle_to_cluster;
        let bars_width = (content_right - bars_left).max(0.0);

        let candle_center_x = candle_lane_left + (candle_lane_width / 2.0);

        Self {
            imb_marker_left: content_left,
            imb_marker_width: if has_imbalance { candle_width } else { 0.0 },
            bars_left,
            bars_width,
            candle_center_x,
        }
    }
}

struct BidAskArea {
    bid_area_left: f32,
    bid_area_right: f32,
    ask_area_left: f32,
    ask_area_right: f32,
    candle_center_x: f32,
    imb_marker_width: f32,
}

impl BidAskArea {
    fn new(
        x_position: f32,
        content_left: f32,
        content_right: f32,
        candle_width: f32,
        spacing: ContentGaps,
    ) -> Self {
        let candle_body_width = candle_width * 0.25;

        let candle_left = x_position - (candle_body_width / 2.0);
        let candle_right = x_position + (candle_body_width / 2.0);

        let ask_area_right = candle_left - spacing.candle_to_cluster;
        let bid_area_left = candle_right + spacing.candle_to_cluster;

        Self {
            bid_area_left,
            bid_area_right: content_right,
            ask_area_left: content_left,
            ask_area_right,
            candle_center_x: x_position,
            imb_marker_width: candle_width,
        }
    }
}

#[inline]
fn footprint_cluster_min_width(cluster_kind: ClusterKind) -> f32 {
    match cluster_kind {
        ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => 80.0,
        ClusterKind::BidAsk => 120.0,
        ClusterKind::Table => 100.0,
    }
}

#[inline]
fn footprint_cluster_text_size(cell_height_unscaled: f32, cell_width_unscaled: f32) -> f32 {
    let text_size_from_height = cell_height_unscaled.round().min(16.0) - 3.0;
    let text_size_from_width = (cell_width_unscaled * 0.1).round().min(16.0) - 3.0;

    text_size_from_height.min(text_size_from_width)
}

#[inline]
fn price_padding_from_pixels(cell_height: f32, tick_size: f32) -> f32 {
    const OUTER_BOUND_PADDING_PX: f32 = 4.0;

    if cell_height <= f32::EPSILON {
        return 0.0;
    }

    (OUTER_BOUND_PADDING_PX / cell_height) * tick_size
}

fn footprint_summary_padding(
    cell_height: f32,
    scaling: f32,
    cell_width: f32,
    tick_size: f32,
    cluster_kind: ClusterKind,
) -> f32 {
    if cell_height <= f32::EPSILON {
        return 0.0;
    }

    let cell_height_unscaled = cell_height * scaling;
    let cell_width_unscaled = cell_width * scaling;

    if !should_show_text(
        cell_height_unscaled,
        cell_width_unscaled,
        footprint_cluster_min_width(cluster_kind),
    ) {
        return 0.0;
    }

    let text_size = footprint_cluster_text_size(cell_height_unscaled, cell_width_unscaled);
    let line_spacing = text_size * 1.2;

    let summary_text_height_px = text_size * 0.9;
    let summary_y_start_px = cell_height * 1.5;

    let second_line_y_start_px = summary_y_start_px + line_spacing;
    let summary_y_end_px = second_line_y_start_px + summary_text_height_px;

    let extra_bottom_padding_px = summary_text_height_px;
    let summary_y_end_with_padding_px = summary_y_end_px + extra_bottom_padding_px;
    let summary_ticks = summary_y_end_with_padding_px / cell_height;

    summary_ticks * tick_size
}

#[inline]
fn should_show_text(cell_height_unscaled: f32, cell_width_unscaled: f32, min_w: f32) -> bool {
    cell_height_unscaled > 8.0 && cell_width_unscaled > min_w
}
