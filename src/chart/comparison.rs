use crate::widget::chart::comparison::{
    DEFAULT_ZOOM_POINTS, LineComparison, LineComparisonEvent, MAX_ZOOM_POINTS, MIN_ZOOM_POINTS,
};
use crate::widget::chart::{Series, Zoom};

use data::chart::Basis;
use data::chart::comparison::Config;
use exchange::adapter::StreamKind;
use exchange::fetcher::{FetchRange, RequestHandler};
use exchange::{Kline, SerTicker, TickerInfo, Timeframe};

use rustc_hash::FxHashMap;
use std::time::Instant;

const SERIES_MAX_POINTS: usize = 5000;

pub enum Action {
    TickerColorChanged(TickerInfo, iced::Color),
    RemoveSeries(TickerInfo),
    OpenColorEditor,
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
    pub color_editor: color_editor::TickerColorEditor,
    cache_rev: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(LineComparisonEvent),
    ColorUpdated(TickerInfo, iced::Color),
    ColorEditor(color_editor::Message),
    OpenColorEditorFor(TickerInfo),
}

impl ComparisonChart {
    pub fn new(basis: Basis, tickers: &[TickerInfo], config: Option<Config>) -> Self {
        let timeframe = match basis {
            Basis::Time(tf) => tf,
            Basis::Tick(_) => todo!("WIP: ComparisonChart does not support tick basis"),
        };

        let cfg = config.unwrap_or_default();

        let color_map: FxHashMap<SerTicker, iced::Color> = cfg.colors.iter().cloned().collect();

        let mut series = Vec::with_capacity(tickers.len());
        let mut series_index = FxHashMap::default();
        for (i, t) in tickers.iter().enumerate() {
            let ser = SerTicker::from_parts(t.ticker.exchange, t.ticker);
            let color = color_map
                .get(&ser)
                .copied()
                .unwrap_or_else(|| default_color_for(t));

            series.push(Series {
                name: *t,
                points: Vec::new(),
                color,
            });
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
            pan: 0.0,
            config: cfg,
            color_editor: color_editor::TickerColorEditor {
                show_color_for: None,
                editing: None,
            },
            cache_rev: 0,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::Chart(evt) => match evt {
                LineComparisonEvent::ZoomChanged(zoom) => {
                    self.zoom = zoom;
                    None
                }
                LineComparisonEvent::PanChanged(pan) => {
                    self.pan = pan;
                    None
                }
                LineComparisonEvent::SeriesCog(ticker_info) => {
                    self.color_editor.show_color_for = Some(ticker_info);
                    Some(Action::OpenColorEditor)
                }
                LineComparisonEvent::SeriesRemove(ticker_info) => {
                    Some(Action::RemoveSeries(ticker_info))
                }
            },
            Message::ColorUpdated(ticker_info, color) => {
                if let Some(idx) = self.series_index.get(&ticker_info)
                    && let Some(s) = self.series.get_mut(*idx)
                {
                    s.color = color;
                    self.upsert_config_color(ticker_info, color);

                    self.cache_rev = self.cache_rev.wrapping_add(1)
                }
                None
            }
            Message::ColorEditor(msg) => self.color_editor.update(msg),
            Message::OpenColorEditorFor(ticker_info) => {
                self.color_editor.show_color_for = Some(ticker_info);
                Some(Action::OpenColorEditor)
            }
        }
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
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

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(t) = now {
            self.last_tick = t;
            self.cache_rev = self.cache_rev.wrapping_add(1);
        }

        if let Some((range, tickers)) = self.desired_fetch_range(self.pan) {
            for ticker in tickers {
                let handler = self.request_handler.entry(ticker).or_default();
                if let Ok(Some(req_id)) = handler.add_request(range) {
                    let stream = StreamKind::Kline {
                        ticker_info: ticker,
                        timeframe: self.timeframe,
                    };
                    return Some(super::Action::FetchRequested(req_id, range, Some(stream)));
                }
            }
        }
        None
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

    fn get_or_create_series_idx(&mut self, ticker_info: &TickerInfo) -> usize {
        if let Some(&i) = self.series_index.get(ticker_info) {
            i
        } else {
            let i = self.series.len();
            self.series.push(Series {
                name: *ticker_info,
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
                self.series_index.insert(s.name, i);
            }
        }
        self.selected_tickers.retain(|t| t != ticker_info);

        if self
            .color_editor
            .show_color_for
            .is_some_and(|t| t == *ticker_info)
        {
            self.color_editor.show_color_for = None;
        }

        self.rebuild_handlers();
        self.streams_for_all()
    }

    pub fn set_basis(&mut self, basis: data::chart::Basis) {
        match basis {
            Basis::Time(tf) => {
                self.timeframe = tf;
            }
            Basis::Tick(_) => {
                todo!("WIP: ComparisonChart does not support tick basis");
            }
        }

        let prev_colors: FxHashMap<TickerInfo, iced::Color> =
            self.series.iter().map(|s| (s.name, s.color)).collect();
        self.series.clear();
        self.series_index.clear();

        for (i, &t) in self.selected_tickers.iter().enumerate() {
            let color = prev_colors
                .get(&t)
                .copied()
                .unwrap_or_else(|| self.color_for_or_default(&t));
            self.series.push(Series {
                name: t,
                points: Vec::new(),
                color,
            });
            self.series_index.insert(t, i);
        }

        self.zoom = Zoom::points(DEFAULT_ZOOM_POINTS);
        self.pan = 0.0;

        self.rebuild_handlers();
        self.color_editor.show_color_for = None;
    }

    pub fn set_ticker_color(&mut self, ticker: TickerInfo, color: iced::Color) {
        if let Some(idx) = self.series_index.get(&ticker)
            && let Some(s) = self.series.get_mut(*idx)
        {
            s.color = color;
            self.upsert_config_color(ticker, color);

            self.cache_rev = self.cache_rev.wrapping_add(1)
        }
    }

    pub fn serializable_config(&self) -> data::chart::comparison::Config {
        let mut colors = vec![];
        for s in &self.series {
            let ser_ticker = SerTicker::from_parts(s.name.ticker.exchange, s.name.ticker);
            colors.push((ser_ticker, s.color));
        }
        data::chart::comparison::Config { colors }
    }

    fn color_for_or_default(&self, ticker_info: &TickerInfo) -> iced::Color {
        let ser = SerTicker::from_parts(ticker_info.ticker.exchange, ticker_info.ticker);
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

    fn streams_for_all(&self) -> Vec<StreamKind> {
        let mut streams = Vec::with_capacity(self.selected_tickers.len());
        for &t in &self.selected_tickers {
            streams.push(StreamKind::Kline {
                ticker_info: t,
                timeframe: self.timeframe,
            });
        }
        streams
    }

    fn upsert_config_color(&mut self, ticker: TickerInfo, color: iced::Color) {
        let ser = SerTicker::from_parts(ticker.ticker.exchange, ticker.ticker);
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

    fn align_ceil(ts: u64, dt: u64) -> u64 {
        if dt == 0 {
            return ts;
        }
        let f = (ts / dt) * dt;
        if f == ts { ts } else { f.saturating_add(dt) }
    }

    fn compute_visible_window(&self, pan_dx: f32) -> Option<(u64, u64)> {
        // X-only window based on current zoom and series data
        if self.series.is_empty() {
            return None;
        }
        let mut any = false;
        let mut data_min_x = u64::MAX;
        let mut data_max_x = u64::MIN;
        for s in &self.series {
            for (x, _) in &s.points {
                any = true;
                if *x < data_min_x {
                    data_min_x = *x;
                }
                if *x > data_max_x {
                    data_max_x = *x;
                }
            }
        }
        if !any {
            return None;
        }
        if data_max_x == data_min_x {
            data_max_x = data_max_x.saturating_add(1);
        }

        let (mut win_min_x, mut win_max_x) = if self.zoom.is_all() {
            (data_min_x, data_max_x)
        } else {
            let n = self.zoom.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS);
            let dt = (self.dt_ms_est() as f32).max(1e-6);
            let mut span = ((n.saturating_sub(1)) as f32 * dt).round() as u64;
            if span == 0 {
                span = 1;
            }
            let max_x = data_max_x;
            let min_x = max_x.saturating_sub(span);
            (min_x, max_x)
        };

        let delta = pan_dx.round() as i64;
        let shift = |v: u64, d: i64| -> u64 {
            if d >= 0 {
                v.saturating_add(d as u64)
            } else {
                v.saturating_sub((-d) as u64)
            }
        };
        win_min_x = shift(win_min_x, delta);
        win_max_x = shift(win_max_x, delta);

        let dt = self.dt_ms_est();
        win_min_x = Self::align_floor(win_min_x, dt);
        win_max_x = Self::align_ceil(win_max_x, dt);

        Some((win_min_x, win_max_x))
    }

    fn desired_fetch_batches(&self, pan_dx: f32) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        let dt = self.dt_ms_est();
        let span = 500u64.saturating_mul(dt);
        let last_closed = Self::align_floor(Self::now_ms(), dt);

        let mut batches: Vec<(FetchRange, Vec<TickerInfo>)> = Vec::new();

        // Seed empties: fetch tail for all empty series
        let mut empty_tickers: Vec<TickerInfo> = Vec::new();
        for s in &self.series {
            if s.points.is_empty() {
                empty_tickers.push(s.name);
            }
        }
        if !empty_tickers.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(span);
            batches.push((FetchRange::Kline(start, end), empty_tickers));
        }

        // Backfill-left: group all that start after visible min
        if let Some((win_min, _win_max)) = self.compute_visible_window(pan_dx) {
            let mut need: Vec<(u64, TickerInfo)> = Vec::new();
            for s in &self.series {
                if let Some(series_min) = s.points.first().map(|(x, _)| *x)
                    && win_min < series_min
                {
                    need.push((series_min, s.name));
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

    fn desired_fetch_range(&self, pan_dx: f32) -> Option<(FetchRange, Vec<TickerInfo>)> {
        let dt = self.dt_ms_est();
        let last_closed = Self::align_floor(Self::now_ms(), dt);
        let win = self.compute_visible_window(pan_dx);

        let mut batches = self.desired_fetch_batches(pan_dx);
        if batches.is_empty() {
            return None;
        }
        if batches.len() == 1 {
            return batches.pop();
        }

        // Prioritize: 0=intersects window, 1=backfill-left, 2=seed, 3=others
        let score = |r: &FetchRange| -> u8 {
            match r {
                FetchRange::Kline(start, end) => {
                    let intersects = if let Some((wmin, wmax)) = win {
                        !(*end < wmin || *start > wmax)
                    } else {
                        false
                    };
                    if intersects {
                        0
                    } else if *end != last_closed {
                        1
                    } else {
                        2
                    }
                }
                _ => 3,
            }
        };

        batches.sort_by_key(|(r, _)| score(r));
        batches.into_iter().next()
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

pub mod color_editor {
    use crate::style;
    use crate::widget::chart::Series;
    use crate::widget::color_picker::color_picker;
    use exchange::TickerInfo;
    use iced::widget::{button, column, container, row, text};
    use iced::{Element, Length};
    use palette::Hsva;

    #[derive(Debug, Clone)]
    pub enum Message {
        ToggleEditFor(TickerInfo, iced::Color),
        ColorChangedHsva(Hsva),
    }

    pub struct TickerColorEditor {
        pub show_color_for: Option<TickerInfo>,
        pub editing: Option<Hsva>,
    }

    impl TickerColorEditor {
        pub fn update(&mut self, msg: Message) -> Option<super::Action> {
            match msg {
                Message::ToggleEditFor(ticker, current_color) => {
                    if let Some(current) = self.show_color_for
                        && current == ticker
                    {
                        self.show_color_for = None;
                        self.editing = None;
                        return None;
                    }

                    self.show_color_for = Some(ticker);
                    self.editing = Some(data::config::theme::to_hsva(current_color));
                    None
                }
                Message::ColorChangedHsva(hsva) => {
                    self.editing = Some(hsva);
                    if let Some(t) = self.show_color_for {
                        return Some(super::Action::TickerColorChanged(
                            t,
                            data::config::theme::from_hsva(hsva),
                        ));
                    }
                    None
                }
            }
        }

        pub fn view<'a>(&'a self, series: &'a Vec<Series>) -> Element<'a, Message> {
            let mut content = column![].spacing(6).padding(4);

            for s in series {
                let applied = s.color;
                let is_open = self.show_color_for.is_some_and(|t| t == s.name);

                let header = button(
                    row![
                        container("").width(12).height(12).style(move |theme| {
                            style::colored_circle_container(theme, applied)
                        }),
                        text(s.name.ticker.symbol_and_exchange_string()).size(14),
                    ]
                    .width(Length::Fill)
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .on_press(Message::ToggleEditFor(s.name, applied))
                .style(move |theme, status| style::button::transparent(theme, status, !is_open))
                .width(Length::Fill);

                let mut col = column![header].spacing(6);

                if is_open {
                    let hsva_in = self
                        .editing
                        .unwrap_or_else(|| data::config::theme::to_hsva(applied));
                    col = col.push(color_picker(hsva_in, Message::ColorChangedHsva));
                }

                content = content.push(container(col).padding(6).style(style::modal_container));
            }

            content.into()
        }
    }
}
