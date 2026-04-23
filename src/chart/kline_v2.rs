use crate::chart::composition::{
    ChartComposition, DEFAULT_MIN_PANEL_RATIO, MarkKind, PanelDataHint, PanelScaleMode,
};
use crate::connector::fetcher::{FetchRange, FetchSpec, RequestHandler};
use crate::widget::chart::kline::{
    DEFAULT_ZOOM_POINTS, KlinePanelKind, KlineSeriesLike, KlineWidget, KlineWidgetEvent,
};
use crate::widget::chart::{Zoom, domain};

use data::chart::Basis;
use exchange::adapter::StreamKind;
use exchange::{Kline, TickerInfo, Timeframe, UnixMs};

use std::time::Instant;

const DEFAULT_PAN_POINTS: f32 = 8.0;
const DEFAULT_FETCH_BARS: u64 = 500;
const SERIES_MAX_BARS: usize = 5000;

pub enum Action {
    RequestFetch(Vec<FetchSpec>),
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
}

impl KlineSeries {
    fn new(ticker_info: TickerInfo) -> Self {
        Self {
            ticker_info,
            name: None,
            bars: Vec::new(),
        }
    }
}

impl KlineSeriesLike for KlineSeries {
    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }

    fn bars(&self) -> &[Kline] {
        &self.bars
    }

    fn secondary_value(&self, bar: &Kline) -> f32 {
        f32::from(bar.volume.total())
    }
}

pub struct KlineChartV2 {
    basis: Basis,
    timeframe: Timeframe,
    zoom: Zoom,
    pan: f32,
    composition: ChartComposition,
    panel_kinds: Vec<KlinePanelKind>,
    panel_splits: Vec<f32>,
    panel_marks: Vec<MarkKind>,
    panel_scale_modes: Vec<PanelScaleMode>,
    last_tick: Instant,
    cache_rev: u64,

    ticker_info: TickerInfo,
    request_handler: RequestHandler,
    pub series: KlineSeries,
}

impl KlineChartV2 {
    pub fn new(basis: Basis, ticker_info: TickerInfo) -> Self {
        let timeframe = Self::timeframe_for_basis(basis);
        let composition = ChartComposition::prototype_kline();

        let mut chart = Self {
            basis,
            timeframe,
            zoom: Zoom::points(DEFAULT_ZOOM_POINTS),
            pan: DEFAULT_PAN_POINTS,
            composition,
            panel_kinds: Vec::new(),
            panel_splits: Vec::new(),
            panel_marks: Vec::new(),
            panel_scale_modes: Vec::new(),
            last_tick: Instant::now(),
            cache_rev: 0,
            ticker_info,
            request_handler: RequestHandler::default(),
            series: KlineSeries::new(ticker_info),
        };

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

    pub fn ticker_info(&self) -> TickerInfo {
        self.ticker_info
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::Chart(event) => match event {
                KlineWidgetEvent::ZoomChanged(zoom) => {
                    self.zoom = zoom;
                    self.bump_rev();
                }
                KlineWidgetEvent::PanChanged(pan) => {
                    self.pan = pan;
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
                KlineWidgetEvent::XAxisDoubleClick => {
                    self.zoom = Zoom::points(DEFAULT_ZOOM_POINTS);
                    self.pan = DEFAULT_PAN_POINTS;
                    self.bump_rev();
                }
            },
        }

        None
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
        if self.series.bars.is_empty() {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let chart: iced::Element<_> =
            KlineWidget::new(std::slice::from_ref(&self.series), self.timeframe)
                .with_basis(self.basis)
                .with_zoom(self.zoom)
                .with_pan(self.pan)
                .with_panel_layout(&self.panel_kinds, &self.panel_splits)
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
        if ticker_info != self.ticker_info {
            self.request_handler
                .mark_failed(req_id, "ticker mismatch".to_string());
            return;
        }

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            self.request_handler
                .mark_failed(req_id, "No data received".to_string());
            return;
        }

        merge_bars(&mut self.series.bars, incoming);
        trim_bars(&mut self.series.bars);

        self.request_handler.mark_completed(req_id);
        self.bump_rev();
    }

    pub fn insert_snapshot(&mut self, ticker_info: TickerInfo, klines: &[Kline]) {
        if ticker_info != self.ticker_info {
            return;
        }

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            return;
        }

        merge_bars(&mut self.series.bars, incoming);
        trim_bars(&mut self.series.bars);
        self.bump_rev();
    }

    pub fn update_latest_kline(&mut self, ticker_info: &TickerInfo, kline: &Kline) {
        if *ticker_info != self.ticker_info {
            return;
        }

        let new_bar = Self::kline_to_bar(kline, self.basis, self.timeframe);

        if let Some(last) = self.series.bars.last_mut() {
            if last.time == new_bar.time {
                *last = new_bar;
            } else if new_bar.time > last.time {
                self.series.bars.push(new_bar);
            }
        } else {
            self.series.bars.push(new_bar);
        }

        trim_bars(&mut self.series.bars);
        self.bump_rev();
    }

    pub fn set_basis(&mut self, basis: Basis) -> Option<Action> {
        self.basis = basis;
        self.timeframe = Self::timeframe_for_basis(basis);
        self.series.bars.clear();
        self.request_handler = RequestHandler::default();
        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_ranges(self.pan));
        self.fetch_action(reqs)
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        if let Some(ts) = now {
            self.last_tick = ts;
        }

        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_ranges(self.pan));
        self.fetch_action(reqs)
    }

    fn bump_rev(&mut self) {
        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    fn sync_widget_panel_layout(&mut self) {
        self.panel_kinds.clear();
        self.panel_marks.clear();
        self.panel_scale_modes.clear();

        for panel in &self.composition.panels {
            let panel_hint = panel.data_hint();

            self.panel_kinds.push(match panel_hint {
                PanelDataHint::ValueLike => KlinePanelKind::Value,
                PanelDataHint::HistogramLike => KlinePanelKind::Histogram,
            });

            let fallback_mark = match panel_hint {
                PanelDataHint::ValueLike => MarkKind::Candle,
                PanelDataHint::HistogramLike => MarkKind::Bar,
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
        }

        self.panel_splits = self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
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
        range: FetchRange,
        out: &mut Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) {
        if let Ok(Some(req_id)) = self.request_handler.add_request(range) {
            out.push((
                req_id,
                range,
                Some(StreamKind::Kline {
                    ticker_info: self.ticker_info,
                    timeframe: self.timeframe,
                }),
            ));
        }
    }

    fn collect_fetch_reqs(
        &mut self,
        batches: Vec<FetchRange>,
    ) -> Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)> {
        let mut reqs = Vec::new();

        for range in batches {
            self.queue_kline_fetch(range, &mut reqs);
        }

        reqs
    }

    fn fetch_action(
        &self,
        reqs: Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) -> Option<Action> {
        if reqs.is_empty() {
            None
        } else {
            let specs = reqs
                .into_iter()
                .map(FetchSpec::from)
                .collect::<Vec<FetchSpec>>();
            Some(Action::RequestFetch(specs))
        }
    }

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds().max(1)
    }

    fn align_floor(&self, ts: UnixMs) -> UnixMs {
        ts.floor_to(self.timeframe)
    }

    fn compute_visible_window(&self, pan_points: f32) -> Option<(UnixMs, UnixMs)> {
        let dt = self.dt_ms_est();

        let points_owned = vec![
            self.series
                .bars
                .iter()
                .map(|bar| (bar.time.as_u64(), bar.close.to_f32()))
                .collect::<Vec<(u64, f32)>>(),
        ];

        let points: Vec<&[(u64, f32)]> = points_owned.iter().map(Vec::as_slice).collect();

        domain::window(&points, self.zoom, pan_points, dt)
            .map(|(start, end)| (UnixMs::new(start), UnixMs::new(end)))
    }

    fn desired_fetch_ranges(&self, pan_points: f32) -> Vec<FetchRange> {
        let dt = self.dt_ms_est();
        let span = DEFAULT_FETCH_BARS.saturating_mul(dt);
        let last_closed = self.align_floor(UnixMs::now());

        let mut ranges = Vec::new();

        if self.series.bars.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(span);
            ranges.push(FetchRange::Kline(start, end));
            return ranges;
        }

        if let Some((window_min, _window_max)) = self.compute_visible_window(pan_points)
            && let Some(series_min) = self.series.bars.first().map(|bar| bar.time)
            && window_min < series_min
        {
            let end = self.align_floor(series_min);
            let start = end.saturating_sub(span);
            ranges.push(FetchRange::Kline(start, end));
        }

        ranges
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
