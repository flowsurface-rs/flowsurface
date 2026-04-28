use crate::connector::fetcher::{FetchRange, FetchSpec, RequestHandler};
use crate::widget::chart::kline::composition::{
    ChartComposition, DEFAULT_MIN_PANEL_RATIO, DataSourceId, LayerId, LayerSource, MarkKind,
    PanelId, PanelRole, PanelScaleMode,
};
use crate::widget::chart::kline::{
    DEFAULT_BAR_SPACING_PX, HorizontalScale, KlinePanelKind, KlineSeriesLike, KlineWidget,
    KlineWidgetEvent,
};

use data::chart::Basis;
use exchange::adapter::{MarketKind, StreamKind};
use exchange::{Kline, OpenInterest, TickerInfo, Timeframe, UnixMs};

use enum_map::{Enum, EnumMap};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
const FETCH_VIEWPORT_WIDTH_ESTIMATE_PX: f32 = 1200.0;
const DEFAULT_FETCH_BARS: u64 = 500;
const SERIES_MAX_BARS: usize = 5000;
const COMPARISON_SOURCE_ID: DataSourceId = DataSourceId::Synthetic("Comparison");

mod indicator;

pub use indicator::IndicatorAvailability;
use indicator::{AvailabilityContext, IndicatorPanelRecipe, SeriesIndicatorData};

pub enum Action {
    SeriesColorChanged(TickerInfo, iced::Color),
    SeriesNameChanged(TickerInfo, String),
    RemoveSeries(TickerInfo),
    OpenSeriesEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndicatorPanelBinding {
    panel_id: PanelId,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
}

#[derive(Debug, Clone)]
pub struct KlineSeries {
    pub ticker_info: TickerInfo,
    pub name: Option<String>,
    pub bars: Vec<Kline>,
    indicators: SeriesIndicatorData,
    panel_indicators: Vec<Option<KlineIndicator>>,
}

impl KlineSeries {
    fn new(ticker_info: TickerInfo) -> Self {
        Self {
            ticker_info,
            name: None,
            bars: Vec::new(),
            indicators: SeriesIndicatorData::default(),
            panel_indicators: Vec::new(),
        }
    }

    fn set_panel_indicators(&mut self, panel_indicators: &[Option<KlineIndicator>]) {
        self.panel_indicators.clear();
        self.panel_indicators.extend_from_slice(panel_indicators);
    }

    fn oi_timerange(&self) -> Option<(UnixMs, UnixMs)> {
        self.indicators.oi_timerange()
    }

    fn refresh_indicator_inputs(&mut self) {
        self.indicators.refresh_from_bars(&self.bars);
    }
}

impl KlineSeriesLike for KlineSeries {
    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }

    fn bars(&self) -> &[Kline] {
        &self.bars
    }

    fn indicator_value(&self, bar: &Kline) -> f32 {
        f32::from(bar.volume.total())
    }

    fn indicator_value_for_panel(&self, panel_index: usize, bar: &Kline) -> f32 {
        self.indicators.value_for_indicator(
            self.panel_indicators.get(panel_index).copied().flatten(),
            bar,
        )
    }
}

pub struct KlineChartV2 {
    basis: Basis,
    timeframe: Timeframe,
    horizontal_scale: HorizontalScale,
    horizontal_offset: f32,
    composition: ChartComposition,
    panel_kinds: Vec<KlinePanelKind>,
    panel_splits: Vec<f32>,
    panel_titles: Vec<Option<String>>,
    panel_marks: Vec<MarkKind>,
    panel_scale_modes: Vec<PanelScaleMode>,
    panel_indicators: Vec<Option<KlineIndicator>>,
    indicator_panels: EnumMap<KlineIndicator, Option<IndicatorPanelBinding>>,
    last_tick: Instant,
    cache_rev: u64,

    base_ticker: TickerInfo,
    selected_tickers: Vec<TickerInfo>,
    series_index: FxHashMap<TickerInfo, usize>,
    comparison_layers: FxHashMap<TickerInfo, LayerId>,
    request_handlers: FxHashMap<TickerInfo, RequestHandler>,
    pub series: Vec<KlineSeries>,
}

impl KlineChartV2 {
    pub fn new(basis: Basis, ticker_info: TickerInfo) -> Self {
        Self::new_with_tickers(basis, &[ticker_info])
    }

    pub fn new_with_tickers(basis: Basis, tickers: &[TickerInfo]) -> Self {
        let base_ticker = tickers
            .first()
            .copied()
            .expect("Kline v2 requires a base ticker");
        let timeframe = Self::timeframe_for_basis(basis);
        let composition = ChartComposition::prototype_kline();

        let mut chart = Self {
            basis,
            timeframe,
            horizontal_scale: HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX),
            horizontal_offset: DEFAULT_HORIZONTAL_OFFSET_UNITS,
            composition,
            panel_kinds: Vec::new(),
            panel_splits: Vec::new(),
            panel_titles: Vec::new(),
            panel_marks: Vec::new(),
            panel_scale_modes: Vec::new(),
            panel_indicators: Vec::new(),
            indicator_panels: EnumMap::default(),
            last_tick: Instant::now(),
            cache_rev: 0,
            base_ticker,
            selected_tickers: Vec::new(),
            series_index: FxHashMap::default(),
            comparison_layers: FxHashMap::default(),
            request_handlers: FxHashMap::default(),
            series: Vec::new(),
        };

        let _ = chart.add_ticker_state(base_ticker);

        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart.add_ticker_state(ticker);
            }
        }

        chart.sync_primary_panel_comparison_sources();

        chart.install_default_indicator_panels();
        chart.sync_widget_panel_layout();
        chart
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }

    pub fn set_primary_scale_mode(&mut self, scale: PanelScaleMode) -> bool {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return false;
        };

        if self
            .composition
            .set_panel_preferred_scale(primary_panel_id, scale)
        {
            self.sync_widget_panel_layout();
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) -> bool {
        let changed = if self.indicator_panels[indicator].is_some() {
            self.disable_indicator(indicator)
        } else {
            self.enable_indicator(indicator)
        };

        if changed {
            self.sync_widget_panel_layout();
            self.bump_rev();
        }

        changed
    }

    pub fn ticker_info(&self) -> TickerInfo {
        self.base_ticker
    }

    pub fn selected_tickers(&self) -> &[TickerInfo] {
        &self.selected_tickers
    }

    pub fn add_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if *ticker_info == self.base_ticker {
            return self.streams_for_all();
        }

        if self.add_ticker_state(*ticker_info) {
            self.sync_primary_panel_comparison_sources();
            self.sync_widget_panel_layout();
            self.bump_rev();
        }

        self.streams_for_all()
    }

    pub fn remove_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if *ticker_info == self.base_ticker {
            return self.streams_for_all();
        }

        if self.remove_ticker_state(*ticker_info) {
            self.sync_primary_panel_comparison_sources();
            self.sync_widget_panel_layout();
            self.bump_rev();
        }

        self.streams_for_all()
    }

    pub fn set_series_color(&mut self, _ticker_info: TickerInfo, _color: iced::Color) -> bool {
        todo!()
    }

    pub fn set_series_name(&mut self, ticker_info: TickerInfo, name: String) -> bool {
        if let Some(idx) = self.series_index.get(&ticker_info).copied() {
            self.series[idx].name = Some(name);
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::Chart(event) => match event {
                KlineWidgetEvent::HorizontalScaleChanged(scale) => {
                    self.horizontal_scale = scale;
                    self.bump_rev();
                }
                KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                    self.horizontal_offset = offset;
                    self.bump_rev();
                }
                KlineWidgetEvent::PanelSplitChanged { index, split } => {
                    if self
                        .composition
                        .set_split(index, split, DEFAULT_MIN_PANEL_RATIO)
                    {
                        self.sync_widget_panel_layout();
                        self.bump_rev();
                    }
                }
                KlineWidgetEvent::PanelMoveUp { index } => {
                    if index > 0 && self.composition.move_panel(index, index - 1) {
                        self.sync_widget_panel_layout();
                        self.bump_rev();
                    }
                }
                KlineWidgetEvent::PanelMoveDown { index } => {
                    let target = index.saturating_add(1);
                    if target < self.composition.panels.len()
                        && self.composition.move_panel(index, target)
                    {
                        self.sync_widget_panel_layout();
                        self.bump_rev();
                    }
                }
                KlineWidgetEvent::PanelSettings { .. } => {
                    // TODO: Hook for upcoming panel settings modal/workflow.
                }
                KlineWidgetEvent::PanelClose { index } => {
                    if let Some(panel_id) = self.composition.panels.get(index).map(|panel| panel.id)
                        && self.composition.remove_panel(panel_id)
                    {
                        self.prune_stale_indicator_panel_bindings();
                        self.sync_widget_panel_layout();
                        self.bump_rev();
                    }
                }
                KlineWidgetEvent::TickerSettings(_ticker) => {
                    // Hook for ticker-specific settings editor.
                }
                KlineWidgetEvent::TickerRemove(ticker) => {
                    if ticker != self.base_ticker {
                        return Some(Action::RemoveSeries(ticker));
                    }
                }
                KlineWidgetEvent::XAxisDoubleClick => {
                    self.horizontal_scale = HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX);
                    self.horizontal_offset = DEFAULT_HORIZONTAL_OFFSET_UNITS;
                    self.bump_rev();
                }
            },
        }

        None
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
        if self.series.iter().all(|series| series.bars.is_empty()) {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let chart: iced::Element<_> = KlineWidget::new(&self.series, self.timeframe)
            .with_basis(self.basis)
            .with_horizontal_scale(self.horizontal_scale)
            .with_horizontal_offset(self.horizontal_offset)
            .with_panel_layout(&self.panel_kinds, &self.panel_splits)
            .with_panel_titles(&self.panel_titles)
            .with_panel_rendering(&self.panel_marks, &self.panel_scale_modes)
            .with_timezone(timezone)
            .version(self.cache_rev)
            .into();

        iced::widget::container(chart.map(Message::Chart))
            .padding(1)
            .into()
    }

    pub fn insert_history(
        &mut self,
        req_id: uuid::Uuid,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
                handler.mark_failed(req_id, "ticker mismatch".to_string());
            }
            return;
        };

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
                handler.mark_failed(req_id, "No data received".to_string());
            }
            return;
        }

        merge_bars(&mut self.series[idx].bars, incoming);
        trim_bars(&mut self.series[idx].bars);
        self.series[idx].refresh_indicator_inputs();
        self.enforce_indicator_availability();

        if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
            handler.mark_completed(req_id);
        }
        self.bump_rev();
    }

    pub fn insert_snapshot(&mut self, ticker_info: TickerInfo, klines: &[Kline]) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return;
        };

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            return;
        }

        merge_bars(&mut self.series[idx].bars, incoming);
        trim_bars(&mut self.series[idx].bars);
        self.series[idx].refresh_indicator_inputs();
        self.enforce_indicator_availability();
        self.bump_rev();
    }

    pub fn update_latest_kline(&mut self, ticker_info: &TickerInfo, kline: &Kline) {
        let Some(idx) = self.series_index.get(ticker_info).copied() else {
            return;
        };

        let new_bar = Self::kline_to_bar(kline, self.basis, self.timeframe);

        let series = &mut self.series[idx];

        if let Some(last) = series.bars.last_mut() {
            if last.time == new_bar.time {
                *last = new_bar;
            } else if new_bar.time > last.time {
                series.bars.push(new_bar);
            }
        } else {
            series.bars.push(new_bar);
        }

        trim_bars(&mut series.bars);
        series.refresh_indicator_inputs();
        self.enforce_indicator_availability();
        self.bump_rev();
    }

    pub fn set_basis(&mut self, basis: Basis) -> Option<super::Action> {
        self.basis = basis;
        self.timeframe = Self::timeframe_for_basis(basis);

        for series in &mut self.series {
            series.bars.clear();
            series.indicators.clear();
        }

        self.enforce_indicator_availability();

        self.rebuild_handlers();
        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_offset));
        self.fetch_action(reqs)
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(ts) = now {
            self.last_tick = ts;
        }

        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_offset));
        self.fetch_action(reqs)
    }

    fn bump_rev(&mut self) {
        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    fn install_default_indicator_panels(&mut self) {
        for indicator in KlineIndicator::for_market(self.base_ticker.market_type()) {
            let _ = self.enable_indicator(*indicator);
        }
    }

    fn enable_indicator(&mut self, indicator: KlineIndicator) -> bool {
        if self.indicator_panels[indicator].is_some() {
            return false;
        }

        if !self.is_indicator_available(indicator) {
            return false;
        }

        let recipe = indicator::panel_recipe(indicator);

        let panel_id = match recipe {
            IndicatorPanelRecipe::AuxPanel {
                panel_title,
                layer_name,
                source,
                data_kind,
                mark,
                axis,
                preferred_scale,
            } => {
                let layer = self.composition.new_layer(
                    layer_name,
                    LayerSource::RawKline { source },
                    data_kind,
                    mark,
                    axis,
                );

                let panel_id = self.composition.add_aux_panel(panel_title, vec![layer]);
                let _ = self
                    .composition
                    .set_panel_preferred_scale(panel_id, preferred_scale);
                panel_id
            }
        };

        self.indicator_panels[indicator] = Some(IndicatorPanelBinding { panel_id });
        true
    }

    fn is_indicator_available(&self, indicator: KlineIndicator) -> bool {
        !matches!(
            self.indicator_availability(indicator),
            IndicatorAvailability::Unsupported(_)
        )
    }

    fn indicator_availability(&self, indicator: KlineIndicator) -> IndicatorAvailability {
        indicator::availability(
            indicator,
            AvailabilityContext {
                basis: self.basis,
                timeframe: self.timeframe,
                base_ticker: self.base_ticker,
            },
            self.series.iter().map(|series| &series.indicators),
        )
    }

    fn disable_indicator(&mut self, indicator: KlineIndicator) -> bool {
        let Some(binding) = self.indicator_panels[indicator].take() else {
            return false;
        };

        if self.composition.remove_panel(binding.panel_id) {
            true
        } else {
            self.indicator_panels[indicator] = Some(binding);
            false
        }
    }

    fn enforce_indicator_availability(&mut self) {
        let mut changed = false;

        for &indicator in indicator::all_indicators() {
            if self.indicator_panels[indicator].is_none() {
                continue;
            }

            if matches!(
                self.indicator_availability(indicator),
                IndicatorAvailability::Unsupported(_)
            ) {
                changed |= self.disable_indicator(indicator);
            }
        }

        if changed {
            self.sync_widget_panel_layout();
            self.bump_rev();
        }
    }

    fn prune_stale_indicator_panel_bindings(&mut self) {
        for indicator in indicator::all_indicators().iter().copied() {
            if let Some(binding) = self.indicator_panels[indicator]
                && self.composition.panel(binding.panel_id).is_none()
            {
                self.indicator_panels[indicator] = None;
            }
        }
    }

    fn sync_widget_panel_layout(&mut self) {
        self.prune_stale_indicator_panel_bindings();

        self.panel_kinds.clear();
        self.panel_titles.clear();
        self.panel_marks.clear();
        self.panel_scale_modes.clear();
        self.panel_indicators.clear();

        for panel in &self.composition.panels {
            let panel_kind = match panel.role {
                PanelRole::Primary => KlinePanelKind::PrimaryChart,
                PanelRole::Auxiliary => KlinePanelKind::Indicator,
            };

            self.panel_kinds.push(panel_kind);

            self.panel_titles.push(panel.title.clone());

            let fallback_mark = match panel_kind {
                KlinePanelKind::PrimaryChart => MarkKind::Candle,
                KlinePanelKind::Indicator => MarkKind::Bar,
            };

            let effective_mark = self
                .composition
                .resolved_panel_marks(panel.id)
                .and_then(|marks| {
                    panel
                        .base_layer
                        .and_then(|base| {
                            marks
                                .iter()
                                .find(|(layer_id, _)| *layer_id == base)
                                .map(|(_, mark)| *mark)
                        })
                        .or_else(|| marks.first().map(|(_, mark)| *mark))
                })
                .unwrap_or(fallback_mark);

            self.panel_marks.push(effective_mark);

            self.panel_scale_modes.push(
                self.composition
                    .panel_effective_scale_mode(panel.id)
                    .unwrap_or(PanelScaleMode::Absolute),
            );

            self.panel_indicators
                .push(self.indicator_for_panel(panel.id));
        }

        self.panel_splits = self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        self.sync_series_panel_indicator_map();
    }

    fn indicator_for_panel(&self, panel_id: PanelId) -> Option<KlineIndicator> {
        indicator::all_indicators()
            .iter()
            .copied()
            .find(|indicator| {
                self.indicator_panels[*indicator]
                    .map(|binding| binding.panel_id == panel_id)
                    .unwrap_or(false)
            })
    }

    fn sync_series_panel_indicator_map(&mut self) {
        for series in &mut self.series {
            series.set_panel_indicators(&self.panel_indicators);
        }
    }

    fn timeframe_for_basis(basis: Basis) -> Timeframe {
        match basis {
            Basis::Time(tf) => tf,
            // Keep widget math operational until tick-domain widget path lands.
            Basis::Tick(_) => Timeframe::MS100,
        }
    }

    fn kline_to_bar(kline: &Kline, basis: Basis, timeframe: Timeframe) -> Kline {
        let mut adjusted = *kline;
        adjusted.time = match basis {
            Basis::Time(_) => kline.time.floor_to(timeframe),
            Basis::Tick(_) => kline.time,
        };
        adjusted
    }

    fn klines_to_bars(&self, klines: &[Kline]) -> Vec<Kline> {
        let mut incoming: Vec<Kline> = klines
            .iter()
            .map(|kline| Self::kline_to_bar(kline, self.basis, self.timeframe))
            .collect();

        incoming.sort_by_key(|bar| bar.time);
        incoming.dedup_by_key(|bar| bar.time);
        incoming
    }

    fn queue_kline_fetch(
        &mut self,
        ticker_info: TickerInfo,
        range: FetchRange,
        out: &mut Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) {
        let handler = self.request_handlers.entry(ticker_info).or_default();

        if let Ok(Some(req_id)) = handler.add_request(range) {
            out.push((
                req_id,
                range,
                Some(StreamKind::Kline {
                    ticker_info,
                    timeframe: self.timeframe,
                }),
            ));
        }
    }

    fn collect_fetch_reqs(
        &mut self,
        batches: Vec<(FetchRange, Vec<TickerInfo>)>,
    ) -> Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)> {
        let mut reqs = Vec::new();

        for (range, tickers) in batches {
            for ticker_info in tickers {
                self.queue_kline_fetch(ticker_info, range, &mut reqs);
            }
        }

        reqs
    }

    fn fetch_action(
        &self,
        reqs: Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) -> Option<super::Action> {
        if reqs.is_empty() {
            None
        } else {
            let specs = reqs
                .into_iter()
                .map(FetchSpec::from)
                .collect::<Vec<FetchSpec>>();
            Some(super::Action::RequestFetch(specs))
        }
    }

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds().max(1)
    }

    fn align_floor(&self, ts: UnixMs) -> UnixMs {
        ts.floor_to(self.timeframe)
    }

    fn estimate_visible_points_for_fetch(&self) -> i64 {
        let spacing = self.horizontal_scale.as_pixels_per_bar().max(1e-3);
        ((FETCH_VIEWPORT_WIDTH_ESTIMATE_PX / spacing).floor() as i64).max(2)
    }

    fn compute_visible_window(&self, horizontal_offset: f32) -> Option<(UnixMs, UnixMs)> {
        let dt = self.dt_ms_est();

        let max_seen = self
            .series
            .iter()
            .flat_map(|series| series.bars.iter())
            .map(|bar| bar.time)
            .max()?;

        let span_units = self.estimate_visible_points_for_fetch().saturating_sub(1);
        let dt_i128 = i128::from(dt);
        let right_unit = horizontal_offset.round() as i128;
        let right = (max_seen.as_u64() as i128) + right_unit.saturating_mul(dt_i128);
        let left = right.saturating_sub((span_units as i128).saturating_mul(dt_i128));

        let left_u64 = left.max(0).min(u64::MAX as i128) as u64;
        let right_u64 = right.max(0).min(u64::MAX as i128) as u64;

        Some((UnixMs::new(left_u64), UnixMs::new(right_u64)))
    }

    fn desired_fetch_batches(&self, horizontal_offset: f32) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        let dt = self.dt_ms_est();
        let span = DEFAULT_FETCH_BARS.saturating_mul(dt);
        let last_closed = self.align_floor(UnixMs::now());

        let mut batches: Vec<(FetchRange, Vec<TickerInfo>)> = Vec::new();

        let mut empty_tickers: Vec<TickerInfo> = Vec::new();
        for series in &self.series {
            if series.bars.is_empty() {
                empty_tickers.push(series.ticker_info);
            }
        }

        if !empty_tickers.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(span);
            batches.push((FetchRange::Kline(start, end), empty_tickers));
        }

        if let Some((window_min, _window_max)) = self.compute_visible_window(horizontal_offset) {
            let mut need: Vec<(UnixMs, TickerInfo)> = Vec::new();

            for series in &self.series {
                if let Some(series_min) = series.bars.first().map(|bar| bar.time)
                    && window_min < series_min
                {
                    need.push((series_min, series.ticker_info));
                }
            }

            if !need.is_empty() {
                let end = need.iter().map(|(end, _)| *end).min().unwrap_or(window_min);
                let end = self.align_floor(end);
                let start = end.saturating_sub(span);
                let tickers = need.into_iter().map(|(_, ticker)| ticker).collect();
                batches.push((FetchRange::Kline(start, end), tickers));
            }

            if self.oi_enabled_for_base()
                && matches!(
                    self.indicator_availability(KlineIndicator::OpenInterest),
                    IndicatorAvailability::Available
                )
                && let Some(base_series) = self.base_series()
                && let Some(kline_latest) = base_series.bars.last().map(|bar| bar.time)
            {
                let visible_window = self.compute_visible_window(horizontal_offset);
                let visible_earliest = visible_window.map(|(start, _)| start).unwrap_or(window_min);
                let visible_span = visible_window
                    .map(|(start, end)| end.as_u64().saturating_sub(start.as_u64()))
                    .unwrap_or(span);
                let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

                match base_series.oi_timerange() {
                    Some((oi_earliest, oi_latest)) => {
                        if visible_earliest < oi_earliest {
                            batches.push((
                                FetchRange::OpenInterest(prefetch_earliest, oi_earliest),
                                vec![self.base_ticker],
                            ));
                        }

                        if oi_latest < kline_latest {
                            let start = oi_latest.max(prefetch_earliest);
                            if start < kline_latest {
                                batches.push((
                                    FetchRange::OpenInterest(start, kline_latest),
                                    vec![self.base_ticker],
                                ));
                            }
                        }
                    }
                    None => {
                        if prefetch_earliest < kline_latest {
                            batches.push((
                                FetchRange::OpenInterest(prefetch_earliest, kline_latest),
                                vec![self.base_ticker],
                            ));
                        }
                    }
                }
            }
        }

        batches
    }

    fn oi_enabled_for_base(&self) -> bool {
        self.indicator_panels[KlineIndicator::OpenInterest].is_some()
    }

    fn base_series(&self) -> Option<&KlineSeries> {
        let idx = self.series_index.get(&self.base_ticker).copied()?;
        self.series.get(idx)
    }

    pub fn insert_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &[OpenInterest],
    ) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(req_id) = req_id
                && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
            {
                handler.mark_failed(req_id, "ticker mismatch".to_string());
            }
            return;
        };

        if let Some(req_id) = req_id
            && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
        {
            if data.is_empty() {
                handler.mark_failed(req_id, "No data received".to_string());
            } else {
                handler.mark_completed(req_id);
            }
        }

        if data.is_empty() {
            return;
        }

        let series = &mut self.series[idx];
        series
            .indicators
            .insert_open_interest_batch(data, self.basis, self.timeframe);
        self.bump_rev();
    }

    fn streams_for_all(&self) -> Vec<StreamKind> {
        self.selected_tickers
            .iter()
            .copied()
            .map(|ticker_info| StreamKind::Kline {
                ticker_info,
                timeframe: self.timeframe,
            })
            .collect()
    }

    fn add_ticker_state(&mut self, ticker_info: TickerInfo) -> bool {
        if self.selected_tickers.contains(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let mut series = KlineSeries::new(ticker_info);
        series.set_panel_indicators(&self.panel_indicators);
        self.series.push(series);
        self.series_index.insert(ticker_info, idx);
        self.request_handlers
            .insert(ticker_info, RequestHandler::default());
        self.selected_tickers.push(ticker_info);

        true
    }

    fn remove_ticker_state(&mut self, ticker_info: TickerInfo) -> bool {
        let Some(idx) = self.series_index.remove(&ticker_info) else {
            return false;
        };

        self.series.remove(idx);
        self.selected_tickers
            .retain(|ticker| *ticker != ticker_info);
        self.request_handlers.remove(&ticker_info);
        self.rebuild_series_index();

        true
    }

    fn rebuild_series_index(&mut self) {
        self.series_index.clear();
        for (idx, series) in self.series.iter().enumerate() {
            self.series_index.insert(series.ticker_info, idx);
        }
    }

    fn rebuild_handlers(&mut self) {
        self.request_handlers.clear();

        for &ticker in &self.selected_tickers {
            self.request_handlers
                .insert(ticker, RequestHandler::default());
        }
    }

    fn sync_primary_panel_comparison_sources(&mut self) {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };

        let stale_layers: Vec<(TickerInfo, LayerId)> = self
            .comparison_layers
            .iter()
            .filter_map(|(ticker, layer_id)| {
                (!self.selected_tickers.contains(ticker) || *ticker == self.base_ticker)
                    .then_some((*ticker, *layer_id))
            })
            .collect();

        for (ticker, layer_id) in stale_layers {
            let _ = self
                .composition
                .remove_layer_from_panel(primary_panel_id, layer_id);
            self.comparison_layers.remove(&ticker);
        }

        for &ticker in self
            .selected_tickers
            .iter()
            .filter(|ticker| **ticker != self.base_ticker)
        {
            if self.comparison_layers.contains_key(&ticker) {
                continue;
            }

            let layer_name = ticker.ticker.symbol_and_exchange_string();

            if let Some(layer_id) = self.composition.add_comparison_source_to_panel(
                primary_panel_id,
                COMPARISON_SOURCE_ID,
                layer_name,
            ) {
                self.comparison_layers.insert(ticker, layer_id);
            }
        }
    }
}

fn merge_bars(dst: &mut Vec<Kline>, mut incoming: Vec<Kline>) {
    if incoming.is_empty() {
        return;
    }

    if dst.is_empty() {
        *dst = incoming;
        return;
    }

    incoming.sort_by_key(|bar| bar.time);
    incoming.dedup_by_key(|bar| bar.time);

    let mut i = 0usize;
    let mut j = 0usize;
    let mut merged = Vec::with_capacity(dst.len() + incoming.len());

    while i < dst.len() && j < incoming.len() {
        let a = dst[i];
        let b = incoming[j];

        if a.time < b.time {
            merged.push(a);
            i += 1;
        } else if b.time < a.time {
            merged.push(b);
            j += 1;
        } else {
            merged.push(b);
            i += 1;
            j += 1;
        }
    }

    if i < dst.len() {
        merged.extend_from_slice(&dst[i..]);
    }

    if j < incoming.len() {
        merged.extend_from_slice(&incoming[j..]);
    }

    merged.dedup_by_key(|bar| bar.time);
    *dst = merged;
}

fn trim_bars(bars: &mut Vec<Kline>) {
    if bars.len() > SERIES_MAX_BARS {
        let to_drop = bars.len() - SERIES_MAX_BARS;
        bars.drain(0..to_drop);
    }
}

pub trait Indicator: PartialEq + Display + 'static {
    fn for_market(market: MarketKind) -> &'static [Self]
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum KlineIndicator {
    Volume,
    OpenInterest,
    CumulativeVolumeDelta,
}

impl Indicator for KlineIndicator {
    fn for_market(market: MarketKind) -> &'static [Self] {
        indicator::indicators_for_market(market)
    }
}

impl Display for KlineIndicator {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", indicator::display_name(*self))
    }
}
