use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::style;
use crate::widget::chart::SeriesLike;
use crate::widget::chart::Zoom;

use exchange::TickerInfo;
use exchange::Timeframe;
use exchange::fetcher::FetchRange;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget};
use iced::advanced::{layout, renderer};
use iced::widget::canvas;
use iced::{
    Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse, window,
};

const Y_AXIS_GUTTER: f32 = 66.0; // px
const X_AXIS_HEIGHT: f32 = 24.0;

const MIN_X_TICK_PX: f32 = 80.0;
const TEXT_SIZE: f32 = 12.0;

const MIN_ZOOM_POINTS: usize = 2;
const MAX_ZOOM_POINTS: usize = 5000;
const ZOOM_STEP_PCT: f32 = 0.05; // 5% per scroll "line"

/// Gap breaker to avoid drawing across missing data
const GAP_BREAK_MULTIPLIER: f32 = 3.0;

pub struct LineComparison<'a, S, Message> {
    series: &'a [S],
    stroke_width: f32,
    zoom: Zoom,
    on_zoom_chg: Option<fn(Zoom) -> Message>,
    on_data_req: Option<fn(FetchRange, TickerInfo) -> Message>,
    /// in milliseconds
    update_interval: u128,
    timeframe: Timeframe,
    pan: f32,
    on_pan_chg: Option<fn(f32) -> Message>,
}

struct State {
    plot_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    labels_cache: canvas::Cache,
    last_draw: Option<std::time::Instant>,
    is_panning: bool,
    last_cursor: Option<Point>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
            labels_cache: canvas::Cache::new(),
            last_draw: None,
            is_panning: false,
            last_cursor: None,
        }
    }
}

impl State {
    fn clear_all_caches(&mut self) {
        self.plot_cache.clear();
        self.overlay_cache.clear();
        self.labels_cache.clear();
    }
}

impl<'a, S, Message> LineComparison<'a, S, Message>
where
    S: SeriesLike,
{
    pub fn new(series: &'a [S], update_interval: u64, timeframe: Timeframe) -> Self {
        Self {
            series,
            stroke_width: 2.0,
            zoom: Zoom::points(100),
            on_zoom_chg: None,
            on_data_req: None,
            update_interval: update_interval as u128,
            timeframe,
            pan: 0.0,
            on_pan_chg: None,
        }
    }

    pub fn with_zoom(mut self, zoom: Zoom) -> Self {
        self.zoom = zoom;
        self
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }

    pub fn on_zoom(mut self, f: fn(Zoom) -> Message) -> Self {
        self.on_zoom_chg = Some(f);
        self
    }

    pub fn on_pan(mut self, f: fn(f32) -> Message) -> Self {
        self.on_pan_chg = Some(f);
        self
    }

    pub fn on_data_request(mut self, f: fn(FetchRange, TickerInfo) -> Message) -> Self {
        self.on_data_req = Some(f);
        self
    }

    // Snap helpers
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

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.points().len())
            .max()
            .unwrap_or(0)
    }

    fn normalize_zoom(&self, z: Zoom) -> Zoom {
        if z.is_all() {
            return Zoom::all();
        }
        let n = z.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS);
        Zoom::points(n)
    }

    fn step_zoom_percent(&self, current: Zoom, zoom_in: bool) -> Zoom {
        let len = self.max_points_available().max(MIN_ZOOM_POINTS);
        let base_n = if current.is_all() {
            len
        } else {
            current.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS)
        };

        let step = ((base_n as f32) * ZOOM_STEP_PCT).ceil().max(1.0) as usize;

        let new_n = if zoom_in {
            base_n.saturating_sub(step).max(MIN_ZOOM_POINTS)
        } else {
            base_n.saturating_add(step).min(MAX_ZOOM_POINTS)
        };

        Zoom::points(new_n)
    }

    /// Current x-span in domain units (ms) for the active zoom.
    fn current_x_span(&self) -> f32 {
        let mut any = false;
        let mut data_min_x = u64::MAX;
        let mut data_max_x = u64::MIN;
        for s in self.series {
            for (x, _) in s.points() {
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
            return 1.0;
        }
        if self.zoom.is_all() {
            ((data_max_x - data_min_x) as f32).max(1.0)
        } else {
            let n = self.zoom.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS);
            let dt = (self.dt_ms_est() as f32).max(1e-6);
            ((n.saturating_sub(1)) as f32 * dt).max(1.0)
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds()
    }

    fn desired_fetch_range(&self, pan_dx: f32) -> Option<(FetchRange, TickerInfo)> {
        let dt = self.dt_ms_est().max(1);
        let span = 500u64.saturating_mul(dt);
        let last_closed = Self::align_floor(Self::now_ms(), dt);

        // 1) Seed: if any series is empty, fetch last 500 candles for the first one.
        for s in self.series {
            if s.points().is_empty() {
                let end = last_closed;
                let start = end.saturating_sub(span);
                return Some((FetchRange::Kline(start, end), *s.ticker_info()));
            }
        }

        // 2) If we have some data, optionally backfill left when the visible window
        //    starts before the first candle of a series.
        if let Some((win_min, _win_max)) = self.compute_visible_window(pan_dx) {
            for s in self.series {
                if let Some(series_min) = s.points().first().map(|(x, _)| *x)
                    && win_min < series_min
                {
                    let end = series_min;
                    let start = end.saturating_sub(span);
                    return Some((FetchRange::Kline(start, end), *s.ticker_info()));
                }
            }
        }

        None
    }

    /// rectangles in the widget-local coordinates
    fn compute_regions(&self, bounds: Rectangle) -> Regions {
        let gutter = Y_AXIS_GUTTER;

        let plot = Rectangle {
            x: 0.0,
            y: 0.0,
            width: (bounds.width - gutter).max(0.0),
            height: (bounds.height - X_AXIS_HEIGHT).max(0.0),
        };

        let x_axis = Rectangle {
            x: 0.0,
            y: plot.y + plot.height,
            width: bounds.width,
            height: X_AXIS_HEIGHT.max(0.0),
        };
        let y_axis = Rectangle {
            x: plot.x + plot.width,
            y: 0.0,
            width: gutter.max(0.0),
            height: plot.height,
        };

        Regions {
            plot,
            x_axis,
            y_axis,
        }
    }

    fn compute_visible_window(&self, pan_dx: f32) -> Option<(u64, u64)> {
        // X-only window, does not depend on y computations
        if self.series.is_empty() {
            return None;
        }
        let mut any = false;
        let mut data_min_x = u64::MAX;
        let mut data_max_x = u64::MIN;
        for s in self.series {
            for (x, _) in s.points() {
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

        let dt = self.dt_ms_est().max(1);
        win_min_x = Self::align_floor(win_min_x, dt);
        win_max_x = Self::align_ceil(win_max_x, dt);

        Some((win_min_x, win_max_x))
    }

    fn compute_domains(&self, pan_dx: f32) -> Option<((u64, u64), (f32, f32))> {
        if self.series.is_empty() {
            return None;
        }
        let mut any = false;
        let mut data_min_x = u64::MAX;
        let mut data_max_x = u64::MIN;
        for s in self.series {
            for (x, _) in s.points() {
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

        let mut min_pct = f32::INFINITY;
        let mut max_pct = f32::NEG_INFINITY;
        let mut any_point = false;

        for s in self.series {
            let pts = s.points();
            if pts.is_empty() {
                continue;
            }

            let idx_right = pts.iter().position(|(x, _)| *x >= win_min_x);
            let y0 = match idx_right {
                Some(0) => pts[0].1,
                Some(i) => {
                    let (x0, y0_) = pts[i - 1];
                    let (x1, y1_) = pts[i];
                    let dx = (x1.saturating_sub(x0)) as f32;
                    if dx > 0.0 {
                        let t = (win_min_x.saturating_sub(x0)) as f32 / dx;
                        y0_ + (y1_ - y0_) * t.clamp(0.0, 1.0)
                    } else {
                        y0_
                    }
                }
                None => continue,
            };

            if y0 == 0.0 {
                continue;
            }

            let mut has_visible = false;
            for (_x, y) in pts
                .iter()
                .filter(|(x, _)| *x >= win_min_x && *x <= win_max_x)
            {
                has_visible = true;
                let pct = ((*y / y0) - 1.0) * 100.0;
                if pct < min_pct {
                    min_pct = pct;
                }
                if pct > max_pct {
                    max_pct = pct;
                }
            }

            if has_visible {
                any_point = true;
                if 0.0 < min_pct {
                    min_pct = 0.0;
                }
                if 0.0 > max_pct {
                    max_pct = 0.0;
                }
            }
        }

        if !any_point {
            return None;
        }

        if (max_pct - min_pct).abs() < f32::EPSILON {
            min_pct -= 1.0;
            max_pct += 1.0;
        }

        let span = (max_pct - min_pct).max(1e-6);
        let pad = span * 0.05;
        let min_pct = min_pct - pad;
        let max_pct = max_pct + pad;

        Some(((win_min_x, win_max_x), (min_pct, max_pct)))
    }

    fn compute_scene(
        &self,
        bounds: Rectangle,
        theme: &Theme,
        cursor: mouse::Cursor,
    ) -> Option<Scene> {
        let palette = theme.extended_palette();
        let ((min_x, max_x), (min_pct, max_pct)) = self.compute_domains(self.pan)?;

        let regions = self.compute_regions(bounds);
        let plot = regions.plot;

        let span_ms = max_x.saturating_sub(min_x).max(1) as f32;
        let px_per_ms = if plot.width > 0.0 {
            plot.width / span_ms
        } else {
            1.0
        };

        let ctx = PlotContext {
            regions,
            plot,
            gutter: Y_AXIS_GUTTER,
            min_x,
            max_x,
            min_pct,
            max_pct,
            px_per_ms,
        };

        // Y ticks and labels
        let total_ticks = (bounds.height / TEXT_SIZE / 3.).floor() as usize;
        let (all_ticks, step) = super::ticks(min_pct, max_pct, total_ticks);
        let mut ticks: Vec<f32> = all_ticks
            .into_iter()
            .filter(|t| (*t >= min_pct - f32::EPSILON) && (*t <= max_pct + f32::EPSILON))
            .collect();
        if ticks.is_empty() {
            ticks = vec![min_pct, max_pct];
        }
        let labels: Vec<String> = ticks
            .iter()
            .map(|t| super::format_pct(*t, step, false))
            .collect();

        let line_color_pool = [
            palette.primary.base.color,
            palette.secondary.base.color,
            palette.success.base.color,
            palette.danger.base.color,
            palette.warning.base.color,
        ];
        let text_color_pool = [
            palette.primary.base.text,
            palette.secondary.base.text,
            palette.success.base.text,
            palette.danger.base.text,
            palette.warning.base.text,
        ];

        // End labels
        let mut end_labels =
            self.collect_end_labels(&ctx, step, &line_color_pool, &text_color_pool);
        resolve_label_overlaps(&mut end_labels, ctx.plot);

        // Cursor
        let cursor_info: Option<CursorInfo> = if let Some(global) = cursor.position() {
            let local = Point::new(global.x - bounds.x, global.y - bounds.y);
            match ctx.regions.hit_test(local) {
                HitZone::Plot => {
                    let cx = local.x.clamp(ctx.plot.x, ctx.plot.x + ctx.plot.width);
                    let ms_from_min = ((cx - ctx.plot.x) / ctx.px_per_ms).round() as u64;
                    let x_domain = ctx.min_x.saturating_add(ms_from_min);

                    let t = ((local.y - ctx.plot.y) / ctx.plot.height).clamp(0.0, 1.0);
                    let pct = ctx.min_pct + (1.0 - t) * (ctx.max_pct - ctx.min_pct);
                    Some(CursorInfo {
                        x_domain,
                        y_pct: pct,
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        // reserve space for Y cursor label to avoid overlap with end labels
        let mut reserved_y: Option<Rectangle> = None;
        if let Some(ci) = cursor_info {
            let t =
                ((ci.y_pct - ctx.min_pct) / (ctx.max_pct - ctx.min_pct).max(1e-6)).clamp(0.0, 1.0);
            let cy_px = ctx.plot.y + ctx.plot.height - t * ctx.plot.height;

            let pct_str = super::format_pct(ci.y_pct, step, true);
            let pct_est_w = (pct_str.len() as f32) * (TEXT_SIZE * 0.6) + 10.0;
            let y_w = pct_est_w.clamp(40.0, Y_AXIS_GUTTER - 8.0);
            let y_h = TEXT_SIZE + 6.0;
            let ylbl_x_right = ctx.plot.x + ctx.plot.width + Y_AXIS_GUTTER - 2.0;
            let ylbl_x = (ylbl_x_right - y_w).max(ctx.plot.x + ctx.plot.width + 2.0);
            let ylbl_y = cy_px.clamp(
                ctx.plot.y + y_h * 0.5,
                ctx.plot.y + ctx.plot.height - y_h * 0.5,
            );
            reserved_y = Some(Rectangle {
                x: ylbl_x,
                y: ylbl_y - y_h * 0.5,
                width: y_w,
                height: y_h,
            });
        }

        Some(Scene {
            ctx,
            y_ticks: ticks,
            y_labels: labels,
            line_color_pool,
            end_labels,
            cursor: cursor_info,
            reserved_y,
            y_step: step,
        })
    }

    fn collect_end_labels(
        &self,
        ctx: &PlotContext,
        step: f32,
        line_color_pool: &[Color; 5],
        text_color_pool: &[Color; 5],
    ) -> Vec<EndLabel> {
        let mut end_labels: Vec<EndLabel> = Vec::new();

        for (si, s) in self.series.iter().enumerate() {
            let pts = s.points();
            if pts.is_empty() {
                continue;
            }
            let global_base = pts[0].1;
            if global_base == 0.0 {
                continue;
            }

            let last_vis = pts
                .iter()
                .rev()
                .find(|(x, _)| *x >= ctx.min_x && *x <= ctx.max_x);
            let (_x1, y1) = match last_vis {
                Some((_x, y)) => (0u64, *y),
                None => continue,
            };

            let idx_right = pts.iter().position(|(x, _)| *x >= ctx.min_x);
            let y0 = match idx_right {
                Some(0) => pts[0].1,
                Some(i) => {
                    let (x0, y0) = pts[i - 1];
                    let (x2, y2) = pts[i];
                    let dx = (x2.saturating_sub(x0)) as f32;
                    if dx > 0.0 {
                        let t = (ctx.min_x.saturating_sub(x0)) as f32 / dx;
                        y0 + (y2 - y0) * t.clamp(0.0, 1.0)
                    } else {
                        y0
                    }
                }
                None => continue,
            };

            if y0 == 0.0 {
                continue;
            }
            let pct_label = ((y1 / y0) - 1.0) * 100.0;

            let mut py = ctx.map_y(pct_label);
            let half_txt = TEXT_SIZE * 0.5;
            py = py.clamp(
                ctx.plot.y + half_txt,
                ctx.plot.y + ctx.plot.height - half_txt,
            );

            let text_color = s.color().unwrap_or_else(|| {
                text_color_pool
                    .get(si % text_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });
            let bg_color = s.color().unwrap_or_else(|| {
                line_color_pool
                    .get(si % line_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });

            let lbl = super::format_pct(pct_label, step, true);
            end_labels.push(EndLabel {
                pos: Point::new(ctx.plot.x + ctx.plot.width + ctx.gutter, py),
                text: lbl,
                bg_color,
                text_color,
            });
        }

        end_labels
    }
}

impl<'a, S, Message> Widget<Message, Theme, Renderer> for LineComparison<'a, S, Message>
where
    S: SeriesLike,
    Message: Clone + 'static,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Mouse(mouse_event) => {
                let state = tree.state.downcast_mut::<State>();
                let bounds = layout.bounds();
                let regions = self.compute_regions(bounds);

                let Some(cursor_pos) = cursor.position_in(bounds) else {
                    if state.is_panning {
                        state.is_panning = false;
                        state.last_cursor = None;
                    }
                    return;
                };

                let zone = regions.hit_test(cursor_pos);

                match mouse_event {
                    mouse::Event::WheelScrolled {
                        delta: mouse::ScrollDelta::Lines { y, .. },
                    } => {
                        if !matches!(zone, HitZone::Plot | HitZone::XAxis) {
                            return;
                        }

                        let on_zoom_chg = match self.on_zoom_chg {
                            Some(f) => f,
                            None => return,
                        };

                        let zoom_in = *y > 0.0;
                        let new_zoom = self.step_zoom_percent(self.zoom, zoom_in);

                        if new_zoom != self.zoom {
                            shell.publish((on_zoom_chg)(self.normalize_zoom(new_zoom)));
                            state.clear_all_caches();
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Left) => {
                        if matches!(zone, HitZone::Plot | HitZone::XAxis) {
                            state.is_panning = true;
                            state.last_cursor = Some(cursor_pos);
                        }
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        state.is_panning = false;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        let Some(on_pan_chg) = self.on_pan_chg else {
                            return;
                        };

                        if state.is_panning {
                            let prev = state.last_cursor.unwrap_or(cursor_pos);
                            let dx_px = cursor_pos.x - prev.x;

                            if dx_px.abs() > 0.0 {
                                let x_span = self.current_x_span();
                                let plot_w = regions.plot.width.max(1.0);
                                let dx_domain = -(dx_px) * (x_span / plot_w);

                                shell.publish((on_pan_chg)(self.pan + dx_domain));
                                state.clear_all_caches();
                            }
                            state.last_cursor = Some(cursor_pos);
                        } else if matches!(zone, HitZone::Plot) {
                            state.overlay_cache.clear();
                        }
                    }
                    _ => {}
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let state = tree.state.downcast_mut::<State>();

                if let Some(last) = state.last_draw {
                    let dur = now.saturating_duration_since(last);

                    if dur.as_millis() < self.update_interval {
                        return;
                    } else {
                        state.clear_all_caches();

                        if let Some(on_req) = self.on_data_req
                            && let Some((range, info)) = self.desired_fetch_range(self.pan)
                        {
                            shell.publish(on_req(range, info));
                        }
                    }
                }
                state.last_draw = Some(*now);
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use advanced::Renderer as _;
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let Some(scene) = self.compute_scene(bounds, theme, cursor) else {
            return;
        };

        let labels = state.labels_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_y_axis_labels(frame, &scene.ctx, &scene.y_ticks, &scene.y_labels, theme);
            self.fill_x_axis_labels(frame, &scene.ctx, theme);
        });
        let plots = state.plot_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_main_geometry(frame, &scene.ctx, &scene.line_color_pool);

            let palette = theme.extended_palette();
            let axis_color = palette.background.strongest.color.scale_alpha(0.25);

            // Y-axis splitter
            let path = {
                let mut b = canvas::path::Builder::new();
                b.move_to(Point::new(scene.ctx.plot.x + scene.ctx.plot.width, 0.0));
                b.line_to(Point::new(
                    scene.ctx.plot.x + scene.ctx.plot.width,
                    scene.ctx.plot.height,
                ));
                b.build()
            };
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(axis_color)
                    .with_width(1.0),
            );
            // X-axis splitter
            let axis_y = scene.ctx.plot.y + scene.ctx.plot.height + 0.5;
            let path = {
                let mut b = canvas::path::Builder::new();
                b.move_to(Point::new(scene.ctx.plot.x, axis_y));
                b.line_to(Point::new(
                    scene.ctx.plot.x + scene.ctx.plot.width + scene.ctx.gutter,
                    axis_y,
                ));
                b.build()
            };
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(axis_color)
                    .with_width(1.0),
            );
        });
        renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
            use iced::advanced::graphics::geometry::Renderer as _;
            renderer.draw_geometry(plots);
            renderer.draw_geometry(labels);
        });

        // (end labels, legend, crosshair) filter end labels against crosshair Y label
        let overlays = state.overlay_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_overlay_y_labels(
                frame,
                &scene.end_labels,
                scene.ctx.gutter,
                scene.reserved_y.as_ref(),
            );
            self.fill_top_left_legend(
                frame,
                &scene.ctx,
                scene.cursor.map(|c| c.x_domain),
                &scene.line_color_pool,
                theme,
                scene.y_step,
            );
            self.fill_crosshair(frame, &scene, theme);
        });
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                use iced::advanced::graphics::geometry::Renderer as _;
                renderer.draw_geometry(overlays);
            });
        });
    }

    fn mouse_interaction(
        &self,
        state: &Tree,
        layout: Layout<'_>,
        cursor: advanced::mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> advanced::mouse::Interaction {
        if let Some(cursor_in_layout) = cursor.position_in(layout.bounds()) {
            let state = state.state.downcast_ref::<State>();
            if state.is_panning {
                return advanced::mouse::Interaction::Grabbing;
            }

            let regions = self.compute_regions(layout.bounds());
            match regions.hit_test(cursor_in_layout) {
                HitZone::Plot => advanced::mouse::Interaction::Crosshair,
                _ => advanced::mouse::Interaction::default(),
            }
        } else {
            advanced::mouse::Interaction::default()
        }
    }
}

impl<'a, S, Message> LineComparison<'a, S, Message>
where
    S: SeriesLike,
{
    #[allow(unused_assignments)]
    fn fill_main_geometry(
        &self,
        frame: &mut canvas::Frame,
        ctx: &PlotContext,
        line_color_pool: &[Color; 5],
    ) {
        for (si, s) in self.series.iter().enumerate() {
            let pts = s.points();
            if pts.is_empty() {
                continue;
            }

            let idx_right = pts.iter().position(|(x, _)| *x >= ctx.min_x);
            let y0 = match idx_right {
                Some(0) => pts[0].1,
                Some(i) => {
                    let (x0, y0_) = pts[i - 1];
                    let (x1, y1_) = pts[i];
                    let dx = (x1.saturating_sub(x0)) as f32;
                    if dx > 0.0 {
                        let t = (ctx.min_x.saturating_sub(x0)) as f32 / dx;
                        y0_ + (y1_ - y0_) * t.clamp(0.0, 1.0)
                    } else {
                        y0_
                    }
                }
                None => continue,
            };

            if y0 == 0.0 {
                continue;
            }

            let color = s.color().unwrap_or_else(|| {
                line_color_pool
                    .get(si % line_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });

            let mut builder = canvas::path::Builder::new();

            let gap_thresh: u64 = ((self.dt_ms_est() as f32) * GAP_BREAK_MULTIPLIER)
                .max(1.0)
                .round() as u64;

            let mut prev_x: Option<u64> = None;
            match idx_right {
                Some(ir) if ir > 0 => {
                    let px0 = ctx.map_x(ctx.min_x);
                    let py0 = ctx.map_y(0.0);
                    builder.move_to(Point::new(px0, py0));
                    prev_x = Some(ctx.min_x);
                }
                Some(0) => {
                    let (fx, fy) = pts[0];
                    if fx <= ctx.max_x {
                        let pct = ((fy / y0) - 1.0) * 100.0;
                        builder.move_to(Point::new(ctx.map_x(fx), ctx.map_y(pct)));
                        prev_x = Some(fx);
                    } else {
                        continue;
                    }
                }
                _ => continue,
            }

            let start_idx = idx_right.unwrap_or(pts.len());

            for (x, y) in pts.iter().skip(start_idx) {
                if *x > ctx.max_x {
                    break;
                }
                let pct = ((*y / y0) - 1.0) * 100.0;
                let px = ctx.map_x(*x);
                let py = ctx.map_y(pct);

                let connect = match prev_x {
                    Some(prev) => x.saturating_sub(prev) <= gap_thresh,
                    None => false,
                };

                if connect {
                    builder.line_to(Point::new(px, py));
                } else {
                    builder.move_to(Point::new(px, py));
                }
                prev_x = Some(*x);
            }

            let path = builder.build();
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(color)
                    .with_width(self.stroke_width),
            );
        }
    }

    fn fill_overlay_y_labels(
        &self,
        frame: &mut canvas::Frame,
        end_labels: &[EndLabel],
        gutter: f32,
        reserved_y: Option<&Rectangle>,
    ) {
        for label in end_labels {
            let label_h = TEXT_SIZE + 4.0;
            let rect = Rectangle {
                x: label.pos.x - gutter,
                y: label.pos.y - TEXT_SIZE * 0.5 - 2.0,
                width: gutter,
                height: label_h,
            };

            if let Some(res) = reserved_y
                && rect.intersects(res)
            {
                continue;
            }

            frame.fill_rectangle(
                Point {
                    x: rect.x,
                    y: rect.y,
                },
                Size {
                    width: rect.width,
                    height: rect.height,
                },
                label.bg_color,
            );

            frame.fill(
                &canvas::Path::circle(Point::new(label.pos.x, label.pos.y), 4.0),
                label.bg_color,
            );

            frame.fill_text(canvas::Text {
                content: label.text.clone(),
                position: label.pos - Vector::new(4.0, 0.0),
                color: label.text_color,
                size: 12.0.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    fn fill_y_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        ctx: &PlotContext,
        ticks: &[f32],
        labels: &[String],
        theme: &Theme,
    ) {
        let palette = theme.extended_palette();
        for (i, tick) in ticks.iter().enumerate() {
            let mut y = ctx.map_y(*tick);
            let half_txt = TEXT_SIZE * 0.5;
            y = y.clamp(
                ctx.plot.y + half_txt,
                ctx.plot.y + ctx.plot.height - half_txt,
            );

            let txt = &labels[i];
            let right_x = ctx.plot.x + ctx.plot.width + ctx.gutter - 4.0;

            frame.fill_text(canvas::Text {
                content: txt.clone(),
                position: Point::new(right_x, y),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    fn fill_x_axis_labels(&self, frame: &mut canvas::Frame, ctx: &PlotContext, theme: &Theme) {
        let palette = theme.extended_palette();
        let axis_y = ctx.plot.y + ctx.plot.height + 0.5;
        let (ticks, step_ms) =
            super::time_ticks(ctx.min_x, ctx.max_x, ctx.px_per_ms, MIN_X_TICK_PX);
        let baseline_to_text = 4.0;
        for t in ticks {
            let x = ctx.map_x(t).clamp(ctx.plot.x, ctx.plot.x + ctx.plot.width);
            let label = super::format_time_label(t, step_ms);
            let y_center = axis_y + baseline_to_text + 2.0 + TEXT_SIZE * 0.5;

            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x, y_center),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    fn fill_top_left_legend(
        &self,
        frame: &mut canvas::Frame,
        ctx: &PlotContext,
        cursor_x: Option<u64>,
        line_color_pool: &[Color; 5],
        theme: &Theme,
        step: f32,
    ) {
        let palette = theme.extended_palette();
        let padding = 8.0;
        let r = 4.0;
        let line_h = TEXT_SIZE + 6.0;

        let rows = self.series.len() as f32;
        if rows > 0.0 {
            let bg_h = (rows * line_h + padding * 2.0).min(ctx.plot.height * 0.6);
            frame.fill_rectangle(
                Point::new(ctx.plot.x + 2.0, ctx.plot.y + 2.0),
                Size::new(120.0, bg_h),
                palette.background.weakest.color.scale_alpha(0.9),
            );
        }

        let mut y = ctx.plot.y + padding + TEXT_SIZE * 0.5;
        let x0 = ctx.plot.x + padding + r;

        for (si, s) in self.series.iter().enumerate() {
            if y > ctx.plot.y + ctx.plot.height - TEXT_SIZE {
                break;
            }
            let color = s
                .color()
                .unwrap_or_else(|| line_color_pool[si % line_color_pool.len()]);

            frame.fill(&canvas::Path::circle(Point::new(x0, y), r), color);

            let pct_str = if let Some(y0) = super::interpolate_y_at(s.points(), ctx.min_x) {
                if y0 != 0.0 {
                    if let Some(cx) = cursor_x {
                        if let Some(yc) = super::interpolate_y_at(s.points(), cx) {
                            let pct = ((yc / y0) - 1.0) * 100.0;
                            super::format_pct(pct, step, true)
                        } else {
                            "—".into()
                        }
                    } else {
                        "—".into()
                    }
                } else {
                    "—".into()
                }
            } else {
                "—".into()
            };

            let content = format!("{}  {}", s.name(), pct_str);
            frame.fill_text(canvas::Text {
                content,
                position: Point::new(x0 + r + 6.0, y),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });

            y += line_h;
        }
    }

    fn fill_crosshair(&self, frame: &mut canvas::Frame, scene: &Scene, theme: &Theme) {
        let Some(ci) = scene.cursor else {
            return;
        };
        let ctx = &scene.ctx;

        let cx = {
            let dx = ci.x_domain.saturating_sub(ctx.min_x) as f32;
            ctx.plot.x + dx * ctx.px_per_ms
        };
        let y_span = (ctx.max_pct - ctx.min_pct).max(1e-6);
        let t = ((ci.y_pct - ctx.min_pct) / y_span).clamp(0.0, 1.0);
        let cy = ctx.plot.y + ctx.plot.height - t * ctx.plot.height;

        let palette = theme.extended_palette();
        let stroke = style::dashed_line_from_palette(palette);

        // Vertical
        let mut b = canvas::path::Builder::new();
        b.move_to(Point::new(cx, ctx.plot.y));
        b.line_to(Point::new(cx, ctx.plot.y + ctx.plot.height));
        frame.stroke(&b.build(), stroke);

        // Horizontal
        let mut b = canvas::path::Builder::new();
        b.move_to(Point::new(ctx.plot.x, cy));
        b.line_to(Point::new(ctx.plot.x + ctx.plot.width, cy));
        frame.stroke(&b.build(), stroke);

        // Time label (x-axis)
        let (_ticks, step_ms) =
            super::time_ticks(ctx.min_x, ctx.max_x, ctx.px_per_ms, MIN_X_TICK_PX);
        let time_str = super::format_time_label(ci.x_domain, step_ms);

        let text_col = palette.secondary.base.text;
        let bg_col = palette.secondary.base.color;

        let est_w = (time_str.len() as f32) * (TEXT_SIZE * 0.6) + 10.0;
        let label_w = est_w.clamp(40.0, 160.0);
        let label_h = TEXT_SIZE + 6.0;

        let time_x = cx.clamp(
            ctx.plot.x + label_w * 0.5,
            ctx.plot.x + ctx.plot.width - label_w * 0.5,
        );
        let time_y = ctx.plot.y + ctx.plot.height + 2.0 + label_h * 0.5;

        frame.fill_rectangle(
            Point::new(time_x - label_w * 0.5, time_y - label_h * 0.5),
            Size::new(label_w, label_h),
            bg_col,
        );
        frame.fill_text(canvas::Text {
            content: time_str,
            position: Point::new(time_x, time_y),
            color: text_col,
            size: TEXT_SIZE.into(),
            font: style::AZERET_MONO,
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            ..Default::default()
        });

        // Percentage label at y-axis gutter
        let pct_str = super::format_pct(ci.y_pct, scene.y_step, true);
        let est_w = (pct_str.len() as f32) * (TEXT_SIZE * 0.6) + 10.0;
        let label_w = est_w.clamp(40.0, Y_AXIS_GUTTER - 8.0);
        let label_h = TEXT_SIZE + 6.0;

        let ylbl_x_right = ctx.plot.x + ctx.plot.width + Y_AXIS_GUTTER - 2.0;
        let ylbl_x = (ylbl_x_right - label_w).max(ctx.plot.x + ctx.plot.width + 2.0);
        let ylbl_y = cy.clamp(
            ctx.plot.y + label_h * 0.5,
            ctx.plot.y + ctx.plot.height - label_h * 0.5,
        );

        frame.fill_rectangle(
            Point::new(ctx.plot.x + ctx.plot.width, ylbl_y - label_h * 0.5),
            Size::new(Y_AXIS_GUTTER, label_h),
            bg_col,
        );
        frame.fill_text(canvas::Text {
            content: pct_str,
            position: Point::new(ylbl_x + label_w - 6.0, ylbl_y),
            color: text_col,
            size: TEXT_SIZE.into(),
            font: style::AZERET_MONO,
            align_x: iced::Alignment::End.into(),
            align_y: iced::Alignment::Center.into(),
            ..Default::default()
        });
    }
}

struct EndLabel {
    pos: Point,
    text: String,
    bg_color: Color,
    text_color: Color,
}

fn resolve_label_overlaps(end_labels: &mut [EndLabel], plot: Rectangle) {
    if end_labels.len() <= 1 {
        return;
    }

    let half_h = TEXT_SIZE * 0.5 + 2.0;
    let mut min_y = plot.y + half_h;
    let mut max_y = plot.y + plot.height - half_h;
    if max_y < min_y {
        core::mem::swap(&mut min_y, &mut max_y);
    }

    let mut sep = TEXT_SIZE + 4.0;

    if end_labels.len() > 1 {
        let avail = (max_y - min_y).max(0.0);
        let needed = sep * (end_labels.len() as f32 - 1.0);
        if needed > avail {
            sep = if end_labels.len() > 1 {
                avail / (end_labels.len() as f32 - 1.0)
            } else {
                sep
            };
        }
    }

    end_labels.sort_by(|a, b| {
        a.pos
            .y
            .partial_cmp(&b.pos.y)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    let mut prev_y = f32::NAN;
    for i in 0..end_labels.len() {
        let low = if i == 0 { min_y } else { prev_y + sep };
        let high = max_y - sep * (end_labels.len() as f32 - 1.0 - i as f32);
        let target = end_labels[i].pos.y;
        let y = target.clamp(low, high);
        end_labels[i].pos.y = y;
        prev_y = y;
    }
}

impl<'a, S, Message> From<LineComparison<'a, S, Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a + 'static,
    S: SeriesLike,
{
    fn from(chart: LineComparison<'a, S, Message>) -> Self {
        Element::new(chart)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitZone {
    Plot,
    XAxis,
    YAxis,
    Outside,
}

#[derive(Debug, Clone, Copy)]
struct Regions {
    plot: Rectangle,
    x_axis: Rectangle,
    y_axis: Rectangle,
}

impl Regions {
    fn is_in_plot(&self, p: Point) -> bool {
        p.x >= self.plot.x
            && p.x <= self.plot.x + self.plot.width
            && p.y >= self.plot.y
            && p.y <= self.plot.y + self.plot.height
    }

    fn is_in_x_axis(&self, p: Point) -> bool {
        p.x >= self.x_axis.x
            && p.x <= self.x_axis.x + self.x_axis.width
            && p.y >= self.x_axis.y
            && p.y <= self.x_axis.y + self.x_axis.height
    }

    fn is_in_y_axis(&self, p: Point) -> bool {
        p.x >= self.y_axis.x
            && p.x <= self.y_axis.x + self.y_axis.width
            && p.y >= self.y_axis.y
            && p.y <= self.y_axis.y + self.y_axis.height
    }

    fn hit_test(&self, p: Point) -> HitZone {
        if self.is_in_plot(p) {
            HitZone::Plot
        } else if self.is_in_x_axis(p) {
            HitZone::XAxis
        } else if self.is_in_y_axis(p) {
            HitZone::YAxis
        } else {
            HitZone::Outside
        }
    }
}

struct PlotContext {
    regions: Regions,
    plot: Rectangle,
    gutter: f32,
    min_x: u64,
    max_x: u64,
    min_pct: f32,
    max_pct: f32,
    px_per_ms: f32,
}

impl PlotContext {
    fn map_x(&self, x: u64) -> f32 {
        let dx = x.saturating_sub(self.min_x) as f32;
        self.plot.x + dx * self.px_per_ms
    }

    fn map_y(&self, pct: f32) -> f32 {
        let span = (self.max_pct - self.min_pct).max(1e-6);
        let t = (pct - self.min_pct) / span;
        self.plot.y + self.plot.height - t.clamp(0.0, 1.0) * self.plot.height
    }
}

#[derive(Clone, Copy)]
struct CursorInfo {
    x_domain: u64,
    y_pct: f32,
}

struct Scene {
    ctx: PlotContext,
    y_ticks: Vec<f32>,
    y_labels: Vec<String>,
    line_color_pool: [Color; 5],
    end_labels: Vec<EndLabel>,
    cursor: Option<CursorInfo>,
    reserved_y: Option<Rectangle>,
    y_step: f32,
}
