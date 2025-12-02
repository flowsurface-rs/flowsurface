use crate::widget::chart::comparison::{DEFAULT_ZOOM_POINTS, LineComparison, LineComparisonEvent};
use crate::widget::chart::{Series, Zoom, domain};

use data::chart::Basis;
use data::chart::comparison::Config;
use exchange::adapter::StreamKind;
use exchange::fetcher::{FetchRange, FetchSpec, RequestHandler};
use exchange::{Kline, SerTicker, TickerInfo, Timeframe};

use rustc_hash::FxHashMap;
use std::time::Instant;

const SERIES_MAX_POINTS: usize = 5000;
const DEFAULT_PAN_POINTS: f32 = 8.0;

pub enum Action {
    SeriesColorChanged(TickerInfo, iced::Color),
    SeriesNameChanged(TickerInfo, String),
    RemoveSeries(TickerInfo),
    OpenSeriesEditor,
}

pub struct ComparisonChart {
    zoom: Zoom,
    pan: f32,
    last_tick: Instant,
    pub series: Vec<Series>,
    series_index: FxHashMap<TickerInfo, usize>,
    pub timeframe: Timeframe,
    request_handler: FxHashMap<TickerInfo, RequestHandler>,
    selected_tickers: Vec<TickerInfo>,
    pub config: data::chart::comparison::Config,
    pub series_editor: series_editor::TickerSeriesEditor,
    cache_rev: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(LineComparisonEvent),
    Editor(series_editor::Message),
    OpenEditorFor(TickerInfo),
}

impl ComparisonChart {
    pub fn new(basis: Basis, tickers: &[TickerInfo], config: Option<Config>) -> Self {
        let timeframe = match basis {
            Basis::Time(tf) => tf,
            Basis::Tick(_) => todo!("WIP: ComparisonChart does not support tick basis"),
        };

        let cfg = config.unwrap_or_default();

        let color_map: FxHashMap<SerTicker, iced::Color> = cfg.colors.iter().cloned().collect();
        let name_map: FxHashMap<SerTicker, String> = cfg.names.iter().cloned().collect();

        let mut series = Vec::with_capacity(tickers.len());
        let mut series_index = FxHashMap::default();
        for (i, t) in tickers.iter().enumerate() {
            let ser = SerTicker::from_parts(t.ticker);

            let color = color_map
                .get(&ser)
                .copied()
                .unwrap_or_else(|| default_color_for(t));
            let name = name_map.get(&ser).cloned();

            series.push(Series::new(*t, color, name));

            series_index.insert(*t, i);
        }

        Self {
            last_tick: Instant::now(),
            zoom: Zoom::points(DEFAULT_ZOOM_POINTS),
            series,
            series_index,
            timeframe,
            request_handler: tickers
                .iter()
                .map(|t| (*t, RequestHandler::new()))
                .collect(),
            selected_tickers: tickers.to_vec(),
            pan: DEFAULT_PAN_POINTS,
            config: cfg,
            series_editor: series_editor::TickerSeriesEditor::default(),
            cache_rev: 0,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::Chart(event) => match event {
                LineComparisonEvent::ZoomChanged(zoom) => {
                    self.zoom = zoom;
                    None
                }
                LineComparisonEvent::PanChanged(pan) => {
                    self.pan = pan;
                    None
                }
                LineComparisonEvent::SeriesCog(ticker_info) => {
                    self.open_editor_for_ticker(ticker_info)
                }
                LineComparisonEvent::SeriesRemove(ticker_info) => {
                    Some(Action::RemoveSeries(ticker_info))
                }
                LineComparisonEvent::XAxisDoubleClick => {
                    self.zoom = Zoom::points(DEFAULT_ZOOM_POINTS);
                    self.pan = DEFAULT_PAN_POINTS;
                    None
                }
            },
            Message::Editor(msg) => self.series_editor.update(msg),
            Message::OpenEditorFor(ticker_info) => self.open_editor_for_ticker(ticker_info),
        }
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
        if self.series.iter().all(|s| s.points.is_empty()) {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let chart: iced::Element<_> = LineComparison::<Series>::new(&self.series, self.timeframe)
            .with_timezone(timezone)
            .with_zoom(self.zoom)
            .with_pan(self.pan)
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
        let idx = self.get_or_create_series_idx(&ticker_info);
        let dst = &mut self.series[idx].points;

        let dt = self.timeframe.to_milliseconds().max(1);
        let align = |t: u64| (t / dt) * dt;

        let mut incoming: Vec<(u64, f32)> = klines
            .iter()
            .map(|k| (align(k.time), k.close.to_f32()))
            .collect();

        incoming.sort_by_key(|(x, _)| *x);
        incoming.dedup_by_key(|(x, _)| *x);

        if incoming.is_empty()
            && let Some(handler) = self.request_handler.get_mut(&ticker_info)
        {
            handler.mark_failed(req_id, "No data received".to_string());
            return;
        }

        if dst.is_empty() {
            *dst = incoming;
        } else {
            let mut i = 0usize;
            let mut j = 0usize;
            let mut merged = Vec::with_capacity(dst.len() + incoming.len());

            while i < dst.len() && j < incoming.len() {
                let (x0, y0) = dst[i];
                let (x1, y1) = incoming[j];
                if x0 < x1 {
                    merged.push((x0, y0));
                    i += 1;
                } else if x1 < x0 {
                    merged.push((x1, y1));
                    j += 1;
                } else {
                    // equal timestamp: prefer incoming
                    merged.push((x1, y1));
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

            merged.dedup_by_key(|(x, _)| *x);

            *dst = merged;
        }

        if self.series[idx].points.len() > SERIES_MAX_POINTS {
            let drop = self.series[idx].points.len() - SERIES_MAX_POINTS;
            self.series[idx].points.drain(0..drop);
        }

        if let Some(handler) = self.request_handler.get_mut(&ticker_info) {
            handler.mark_completed(req_id);
        }
    }

    pub fn update_latest_kline(&mut self, ticker_info: &TickerInfo, kline: &Kline) {
        let idx = self.get_or_create_series_idx(ticker_info);
        let series = &mut self.series[idx];

        // Align to timeframe grid
        let dt = self.timeframe.to_milliseconds().max(1);
        let t = (kline.time / dt) * dt;
        let new_point = (t, kline.close.to_f32());

        if let Some((last_x, last_y)) = series.points.last_mut() {
            if *last_x == new_point.0 {
                *last_y = new_point.1;
            } else if new_point.0 > *last_x {
                series.points.push(new_point);
            }
        } else {
            series.points.push(new_point);
        }

        // Use same cap as history to avoid churn/backfill loops
        if series.points.len() > SERIES_MAX_POINTS {
            let drop = series.points.len() - SERIES_MAX_POINTS;
            series.points.drain(0..drop);
        }
    }

    pub fn update_interval(&self) -> u64 {
        if self.is_bbo_based() {
            self.timeframe.to_milliseconds().max(10)
        } else {
            1000
        }
    }

    /// Returns true if the current timeframe is sub-minute (BBO-based, no history)
    fn is_bbo_based(&self) -> bool {
        self.timeframe.to_milliseconds() < 60_000
    }

    pub fn insert_depth(
        &mut self,
        ticker_info: &TickerInfo,
        update_time: u64,
        depth: &exchange::depth::Depth,
    ) {
        if !self.is_bbo_based() {
            return;
        }

        let Some(mid_price) = depth.mid_price() else {
            return;
        };

        let idx = self.get_or_create_series_idx(ticker_info);
        let series = &mut self.series[idx];

        let dt = self.timeframe.to_milliseconds().max(1);
        let aligned_time = (update_time / dt) * dt;
        let price = mid_price.to_f32();

        if let Some((last_x, last_y)) = series.points.last_mut() {
            if *last_x == aligned_time {
                // Same time bucket - update the price (last value wins, or you could average)
                *last_y = price;
            } else if aligned_time > *last_x {
                // New time bucket - push new point
                series.points.push((aligned_time, price));
            }
            // Ignore out-of-order updates (aligned_time < *last_x)
        } else {
            // First point for this series
            series.points.push((aligned_time, price));
        }

        // Cap the series length
        if series.points.len() > SERIES_MAX_POINTS {
            let drop = series.points.len() - SERIES_MAX_POINTS;
            series.points.drain(0..drop);
        }

        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    pub fn insert_bbo(
        &mut self,
        ticker_info: &TickerInfo,
        update_time: u64,
        bbo: &exchange::depth::BestBidAsk,
    ) {
        if !self.is_bbo_based() {
            return;
        }

        let Some(mid_price) = bbo.mid_price() else {
            return;
        };

        let idx = self.get_or_create_series_idx(ticker_info);
        let series = &mut self.series[idx];

        let dt = self.timeframe.to_milliseconds().max(1);
        let aligned_time = (update_time / dt) * dt;
        let price = mid_price.to_f32();

        if let Some((last_x, last_y)) = series.points.last_mut() {
            if *last_x == aligned_time {
                // Same time bucket - update the price (last value wins, or you could average)
                *last_y = price;
            } else if aligned_time > *last_x {
                // New time bucket - push new point
                series.points.push((aligned_time, price));
            }
            // Ignore out-of-order updates (aligned_time < *last_x)
        } else {
            // First point for this series
            series.points.push((aligned_time, price));
        }

        // Cap the series length
        if series.points.len() > SERIES_MAX_POINTS {
            let drop = series.points.len() - SERIES_MAX_POINTS;
            series.points.drain(0..drop);
        }

        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    /// Returns the required stream kinds for all selected tickers
    /// based on the current timeframe mode
    pub fn streams_for_all(&self) -> Vec<StreamKind> {
        let mut streams = Vec::with_capacity(self.selected_tickers.len());

        if self.is_bbo_based() {
            // For sub-minute timeframes, subscribe to BBO/depth streams
            for &t in &self.selected_tickers {
                streams.push(StreamKind::PartialBook(t));
            }
        } else {
            // For minute+ timeframes, subscribe to kline streams
            for &t in &self.selected_tickers {
                streams.push(StreamKind::Kline {
                    ticker_info: t,
                    timeframe: self.timeframe,
                });
            }
        }

        streams
    }

    fn get_or_create_series_idx(&mut self, ticker_info: &TickerInfo) -> usize {
        if let Some(&i) = self.series_index.get(ticker_info) {
            i
        } else {
            let i = self.series.len();
            self.series.push(Series {
                ticker_info: *ticker_info,
                name: None,
                points: Vec::new(),
                color: self.color_for_or_default(ticker_info),
            });
            self.series_index.insert(*ticker_info, i);
            i
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn add_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if !self.selected_tickers.contains(ticker_info) {
            self.selected_tickers.push(*ticker_info);
        }

        let _ = self.get_or_create_series_idx(ticker_info);
        self.rebuild_handlers();
        self.streams_for_all()
    }

    pub fn remove_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if let Some(idx) = self.series_index.remove(ticker_info) {
            self.series.remove(idx);
            self.series_index.clear();
            for (i, s) in self.series.iter().enumerate() {
                self.series_index.insert(s.ticker_info, i);
            }
        }
        self.selected_tickers.retain(|t| t != ticker_info);

        if self
            .series_editor
            .show_config_for
            .is_some_and(|t| t == *ticker_info)
        {
            self.series_editor.show_config_for = None;
        }

        self.rebuild_handlers();
        self.streams_for_all()
    }

    fn queue_kline_fetch(
        &mut self,
        ticker: TickerInfo,
        range: FetchRange,
        out: &mut Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) {
        let handler = self.request_handler.entry(ticker).or_default();
        if let Ok(Some(req_id)) = handler.add_request(range) {
            out.push((
                req_id,
                range,
                Some(StreamKind::Kline {
                    ticker_info: ticker,
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
            for ticker in tickers {
                self.queue_kline_fetch(ticker, range, &mut reqs);
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
            let fetch_specs: Vec<FetchSpec> = reqs
                .into_iter()
                .map(|(req_id, fetch, stream)| FetchSpec {
                    req_id,
                    fetch,
                    stream,
                })
                .collect();
            let requests = exchange::fetcher::FetchRequests::from(fetch_specs);
            Some(super::Action::RequestFetch(requests))
        }
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(t) = now {
            self.last_tick = t;
            self.cache_rev = self.cache_rev.wrapping_add(1);
        }

        if self.is_bbo_based() {
            return None;
        }

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.pan));
        self.fetch_action(reqs)
    }

    pub fn set_basis(&mut self, basis: data::chart::Basis) -> Option<super::Action> {
        match basis {
            Basis::Time(tf) => {
                self.timeframe = tf;
                let is_bbo_based = self.is_bbo_based();

                let prev_colors: FxHashMap<TickerInfo, iced::Color> = self
                    .series
                    .iter()
                    .map(|s| (s.ticker_info, s.color))
                    .collect();
                let prev_names: FxHashMap<TickerInfo, Option<String>> = self
                    .series
                    .iter()
                    .map(|s| (s.ticker_info, s.name.clone()))
                    .collect();

                // Clear series data when switching timeframes
                // (especially important when switching between kline and BBO modes)
                self.series.clear();
                self.series_index.clear();

                for (i, &t) in self.selected_tickers.iter().enumerate() {
                    let color = prev_colors
                        .get(&t)
                        .copied()
                        .unwrap_or_else(|| self.color_for_or_default(&t));
                    let name = prev_names.get(&t).cloned().unwrap_or(None);

                    self.series.push(Series::new(t, color, name));
                    self.series_index.insert(t, i);
                }

                self.rebuild_handlers();

                // Only fetch for kline-based timeframes
                if is_bbo_based {
                    None
                } else {
                    let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.pan));
                    self.fetch_action(reqs)
                }
            }
            Basis::Tick(_) => unimplemented!(),
        }
    }

    fn open_editor_for_ticker(&mut self, ticker_info: TickerInfo) -> Option<Action> {
        self.series_editor.show_config_for = Some(ticker_info);

        if let Some(idx) = self.series_index.get(&ticker_info) {
            self.series_editor.editing_color =
                Some(data::config::theme::to_hsva(self.series[*idx].color));
            self.series_editor.editing_name = self.series[*idx].name.clone();
        } else {
            self.series_editor.editing_color = None;
            self.series_editor.editing_name = None;
        }

        Some(Action::OpenSeriesEditor)
    }

    fn clamp_label(name: &str) -> String {
        name.chars().take(24).collect()
    }

    pub fn set_series_color(&mut self, ticker: TickerInfo, color: iced::Color) {
        if let Some(idx) = self.series_index.get(&ticker)
            && let Some(s) = self.series.get_mut(*idx)
        {
            s.color = color;
            self.upsert_config_color(ticker, color);

            self.cache_rev = self.cache_rev.wrapping_add(1)
        }
    }

    pub fn set_series_name(&mut self, ticker: TickerInfo, name: String) {
        let clamped = Self::clamp_label(name.trim());
        if let Some(idx) = self.series_index.get(&ticker)
            && let Some(s) = self.series.get_mut(*idx)
        {
            s.name = if clamped.is_empty() {
                None
            } else {
                Some(clamped)
            };

            self.cache_rev = self.cache_rev.wrapping_add(1)
        }
    }

    pub fn serializable_config(&self) -> data::chart::comparison::Config {
        let mut colors = vec![];
        let mut names = vec![];

        for s in &self.series {
            let ser_ticker = SerTicker::from_parts(s.ticker_info.ticker);

            colors.push((ser_ticker.clone(), s.color));
            if let Some(name) = &s.name {
                names.push((ser_ticker, name.clone()));
            }
        }
        data::chart::comparison::Config { colors, names }
    }

    fn color_for_or_default(&self, ticker_info: &TickerInfo) -> iced::Color {
        let ser = SerTicker::from_parts(ticker_info.ticker);
        if let Some((_, c)) = self.config.colors.iter().find(|(s, _)| s == &ser) {
            *c
        } else {
            default_color_for(ticker_info)
        }
    }

    pub fn selected_tickers(&self) -> &[TickerInfo] {
        &self.selected_tickers
    }

    fn rebuild_handlers(&mut self) {
        self.request_handler.clear();

        for &t in &self.selected_tickers {
            self.request_handler.insert(t, RequestHandler::new());
        }
    }

    fn upsert_config_color(&mut self, ticker_info: TickerInfo, color: iced::Color) {
        let ser = SerTicker::from_parts(ticker_info.ticker);
        if let Some((_, c)) = self.config.colors.iter_mut().find(|(t, _)| *t == ser) {
            *c = color;
        } else {
            self.config.colors.push((ser, color));
        }
    }

    fn now_ms() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds().max(1)
    }

    fn align_floor(ts: u64, dt: u64) -> u64 {
        if dt == 0 {
            return ts;
        }
        (ts / dt) * dt
    }

    fn compute_visible_window(&self, pan_points: f32) -> Option<(u64, u64)> {
        let dt = self.dt_ms_est().max(1);
        let points: Vec<&[(u64, f32)]> = self.series.iter().map(|s| s.points.as_slice()).collect();

        domain::window(&points, self.zoom, pan_points, dt)
    }

    fn desired_fetch_batches(&self, pan_points: f32) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        let dt = self.dt_ms_est().max(1);
        let span = 500u64.saturating_mul(dt);
        let last_closed = Self::align_floor(Self::now_ms(), dt);

        let mut batches: Vec<(FetchRange, Vec<TickerInfo>)> = Vec::new();

        // Seed empties
        let mut empty_tickers: Vec<TickerInfo> = Vec::new();
        for s in &self.series {
            if s.points.is_empty() {
                empty_tickers.push(s.ticker_info);
            }
        }
        if !empty_tickers.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(span);
            batches.push((FetchRange::Kline(start, end), empty_tickers));
        }

        // Backfill-left relative to visible window
        if let Some((win_min, _win_max)) = self.compute_visible_window(pan_points) {
            let mut need: Vec<(u64, TickerInfo)> = Vec::new();
            for s in &self.series {
                if let Some(series_min) = s.points.first().map(|(x, _)| *x)
                    && win_min < series_min
                {
                    need.push((series_min, s.ticker_info));
                }
            }
            if !need.is_empty() {
                let end = need.iter().map(|(e, _)| *e).min().unwrap_or(win_min);
                let end = Self::align_floor(end, dt);
                let start = end.saturating_sub(span);
                let tickers = need.into_iter().map(|(_, t)| t).collect();
                batches.push((FetchRange::Kline(start, end), tickers));
            }
        }

        batches
    }
}

fn default_color_for(ticker: &TickerInfo) -> iced::Color {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    ticker.hash(&mut hasher);
    let seed = hasher.finish();

    // Golden-angle distribution for hue (in degrees)
    let golden = 0.618_034_f32;
    let base = ((seed as f32 / u64::MAX as f32) + 0.12345).fract();
    let hue = (base + golden).fract() * 360.0;

    // Slightly vary saturation and value in a pleasant range
    let s = 0.60 + (((seed >> 8) & 0xFF) as f32 / 255.0) * 0.25; // 0.60..=0.85
    let v = 0.85 + (((seed >> 16) & 0x7F) as f32 / 127.0) * 0.10; // 0.85..=0.95

    data::config::theme::from_hsv_degrees(hue, s.min(1.0), v.min(1.0))
}

pub mod series_editor {
    use crate::style;
    use crate::widget::chart::Series;
    use crate::widget::color_picker::color_picker;
    use exchange::TickerInfo;
    use iced::widget::{button, column, container, row, text};
    use iced::{Element, Length};
    use palette::Hsva;

    const MAX_LABEL_CHARS: usize = 24;

    #[derive(Debug, Clone)]
    pub enum Message {
        ToggleEditFor {
            ticker: TickerInfo,
            applied_color: iced::Color,
            applied_name: Option<String>,
        },
        ColorChangedHsva(Hsva),
        NameChanged(String),
    }

    #[derive(Default)]
    pub struct TickerSeriesEditor {
        pub show_config_for: Option<TickerInfo>,
        pub editing_color: Option<Hsva>,
        pub editing_name: Option<String>,
    }

    impl TickerSeriesEditor {
        pub fn update(&mut self, msg: Message) -> Option<super::Action> {
            match msg {
                Message::ToggleEditFor {
                    ticker,
                    applied_color,
                    applied_name,
                } => {
                    if let Some(current) = self.show_config_for
                        && current == ticker
                    {
                        self.show_config_for = None;
                        self.editing_color = None;
                        self.editing_name = None;
                        return None;
                    }
                    self.show_config_for = Some(ticker);
                    self.editing_color = Some(data::config::theme::to_hsva(applied_color));
                    self.editing_name = applied_name;
                    None
                }
                Message::ColorChangedHsva(hsva) => {
                    self.editing_color = Some(hsva);
                    if let Some(t) = self.show_config_for {
                        return Some(super::Action::SeriesColorChanged(
                            t,
                            data::config::theme::from_hsva(hsva),
                        ));
                    }
                    None
                }
                Message::NameChanged(new_name) => {
                    let trimmed = new_name.trim();
                    let limited = Self::clamp(trimmed);
                    self.editing_name = Some(limited.clone());
                    if let Some(t) = self.show_config_for {
                        return Some(super::Action::SeriesNameChanged(t, limited));
                    }
                    None
                }
            }
        }

        pub fn view<'a>(&'a self, series: &'a Vec<Series>) -> Element<'a, Message> {
            let mut content = column![].spacing(6);

            for s in series {
                let applied = s.color;
                let is_open = self.show_config_for.is_some_and(|t| t == s.ticker_info);

                let header = button(
                    row![
                        container("").width(14).height(14).style(move |theme| {
                            style::colored_circle_container(theme, applied)
                        }),
                        text(s.ticker_info.ticker.symbol_and_exchange_string()).size(13),
                    ]
                    .width(Length::Fill)
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .on_press(Message::ToggleEditFor {
                    ticker: s.ticker_info,
                    applied_color: applied,
                    applied_name: s.name.clone(),
                })
                .style(move |theme, status| style::button::transparent(theme, status, is_open))
                .width(Length::Fill);

                let mut col = column![header].padding(4);
                let mut inner_col = column![];

                if is_open {
                    let hsva_in = self
                        .editing_color
                        .unwrap_or_else(|| data::config::theme::to_hsva(applied));
                    inner_col = inner_col.push(color_picker(hsva_in, Message::ColorChangedHsva));

                    let label_name = self
                        .editing_name
                        .clone()
                        .unwrap_or_else(|| s.name.clone().unwrap_or_default());
                    inner_col = inner_col.push(
                        iced::widget::text_input("Set a custom label name", &label_name)
                            .on_input(Message::NameChanged)
                            .size(14)
                            .padding(4)
                            .width(Length::Fill),
                    );

                    col = col.push(inner_col.spacing(12).padding(4)).spacing(4);
                }

                content = content.push(container(col).style(style::modal_container));
            }

            content.into()
        }

        fn clamp(s: &str) -> String {
            s.chars().take(MAX_LABEL_CHARS).collect()
        }
    }
}
