use crate::style;
use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::window;
use iced::{Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

const Y_AXIS_GUTTER: f32 = 66.0; // px
const X_AXIS_HEIGHT: f32 = 24.0;

const MIN_X_TICK_PX: f32 = 80.0;
const TEXT_SIZE: f32 = 12.0;

const MIN_ZOOM_POINTS: usize = 2;
const MAX_ZOOM_POINTS: usize = 5000;
const ZOOM_STEP_PCT: f32 = 0.05; // 5% per scroll "line"

/// Gap breaker to avoid drawing across missing data
const GAP_BREAK_MULTIPLIER: f32 = 3.0;

#[derive(Debug, Clone)]
pub struct Series {
    pub name: String,
    /// (x, y) where x is a time-like domain (e.g., timestamp) and y is raw value
    pub points: Vec<(u64, f32)>,
    pub color: Option<Color>,
}

pub trait SeriesLike {
    fn name(&self) -> &str;
    fn points(&self) -> &[(u64, f32)];
    fn color(&self) -> Option<Color>;
}

impl SeriesLike for Series {
    fn name(&self) -> &str {
        &self.name
    }
    fn points(&self) -> &[(u64, f32)] {
        &self.points
    }
    fn color(&self) -> Option<Color> {
        self.color
    }
}

pub struct LineComparison<'a, S, Message> {
    series: &'a [S],
    stroke_width: f32,
    zoom: Zoom,
    on_zoom_chg: Option<fn(Zoom) -> Message>,
    /// in milliseconds
    update_interval: u128,
}

struct State {
    plot_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    labels_cache: canvas::Cache,
    last_draw: Option<std::time::Instant>,
    pan_dx: f32,
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
            pan_dx: 0.0,
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
    pub fn new(series: &'a [S], update_interval: u64) -> Self {
        Self {
            series,
            stroke_width: 2.0,
            zoom: Zoom::all(),
            on_zoom_chg: None,
            update_interval: update_interval as u128,
        }
    }

    pub fn with_zoom(mut self, zoom: Zoom) -> Self {
        self.zoom = zoom;
        self
    }

    pub fn on_zoom(mut self, f: fn(Zoom) -> Message) -> Self {
        self.on_zoom_chg = Some(f);
        self
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

    fn estimated_dt(&self) -> f32 {
        let mut dts: Vec<f32> = self
            .series
            .iter()
            .filter_map(|s| {
                let pts = s.points();
                if pts.len() >= 2 {
                    let first = pts.first().unwrap().0;
                    let last = pts.last().unwrap().0;
                    if last <= first {
                        return None;
                    }
                    let steps = (pts.len() - 1) as f32;
                    let dt = (last - first) as f32 / steps;
                    (dt.is_finite() && dt > 0.0).then_some(dt)
                } else {
                    None
                }
            })
            .collect();
        if dts.is_empty() {
            return 1.0;
        }
        dts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = dts.len() / 2;
        if dts.len() % 2 == 1 {
            dts[mid]
        } else {
            (dts[mid - 1] + dts[mid]) * 0.5
        }
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
            let dt = self.estimated_dt().max(1e-6);
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

    /// Compute a "nice" step close to range/target using 1/2/5*10^k
    fn nice_step(range: f32, target: usize) -> f32 {
        let target = target.max(2) as f32;
        let raw = (range / target).max(f32::EPSILON);
        let power = raw.log10().floor();
        let base = 10f32.powf(power);
        let n = raw / base;
        let nice = if n <= 1.0 {
            1.0
        } else if n <= 2.0 {
            2.0
        } else if n <= 5.0 {
            5.0
        } else {
            10.0
        };
        nice * base
    }

    fn ticks(&self, min: f32, max: f32, target: usize) -> (Vec<f32>, f32) {
        let span = (max - min).abs().max(1e-6);
        let step = Self::nice_step(span, target);
        let start = (min / step).floor() * step;
        let end = (max / step).ceil() * step;

        let mut v = Vec::new();
        let mut t = start;
        for _ in 0..100 {
            if t > end + step * 0.5 {
                break;
            }
            v.push(t);
            t += step;
        }
        (v, step)
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

    fn collect_end_labels<Fy>(
        &self,
        plot: Rectangle,
        map_y: &Fy,
        min_x: u64,
        max_x: u64,
        step: f32,
        line_color_pool: &[Color; 5],
        text_color_pool: &[Color; 5],
        gutter: f32,
    ) -> Vec<EndLabel>
    where
        Fy: Fn(f32) -> f32,
    {
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

            let last_vis = pts.iter().rev().find(|(x, _)| *x >= min_x && *x <= max_x);
            let (_x1, y1) = match last_vis {
                Some((_x, y)) => (0u64, *y),
                None => continue,
            };

            let idx_right = pts.iter().position(|(x, _)| *x >= min_x);
            let y0 = match idx_right {
                Some(0) => pts[0].1,
                Some(i) => {
                    let (x0, y0) = pts[i - 1];
                    let (x2, y2) = pts[i];
                    let dx = (x2.saturating_sub(x0)) as f32;
                    if dx > 0.0 {
                        let t = (min_x.saturating_sub(x0)) as f32 / dx;
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

            let mut py = map_y(pct_label);
            let half_txt = TEXT_SIZE * 0.5;
            py = py.clamp(plot.y + half_txt, plot.y + plot.height - half_txt);

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

            let lbl = format_pct(pct_label, step, true);
            end_labels.push(EndLabel {
                pos: Point::new(plot.x + plot.width + gutter, py),
                text: lbl,
                bg_color,
                text_color,
            });
        }

        end_labels
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
            let dt = self.estimated_dt().max(1e-6);
            ((n.saturating_sub(1)) as f32 * dt).max(1.0)
        }
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
        _cursor: mouse::Cursor,
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

                match mouse_event {
                    mouse::Event::WheelScrolled {
                        delta: mouse::ScrollDelta::Lines { y, .. },
                    } => {
                        let state = tree.state.downcast_mut::<State>();

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
                        state.is_panning = true;
                        state.last_cursor = None;
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        state.is_panning = false;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { position } => {
                        if state.is_panning {
                            let prev = state.last_cursor.unwrap_or(*position);
                            let dx_px = position.x - prev.x;

                            if dx_px.abs() > 0.0 {
                                let x_span = self.current_x_span();
                                let dx_domain = -(dx_px)
                                    * (x_span / (layout.bounds().width - Y_AXIS_GUTTER).max(1.0));
                                state.pan_dx += dx_domain;

                                state.clear_all_caches();
                            }
                            state.last_cursor = Some(*position);
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
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use advanced::Renderer as _;
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let palette = theme.extended_palette();
        let Some(((min_x, max_x), (min_pct, max_pct))) = self.compute_domains(state.pan_dx) else {
            return;
        };

        let total_ticks = (bounds.height / TEXT_SIZE / 3.).floor() as usize;
        let (all_ticks, step) = self.ticks(min_pct, max_pct, total_ticks);

        let mut ticks: Vec<f32> = all_ticks
            .into_iter()
            .filter(|t| (*t >= min_pct - f32::EPSILON) && (*t <= max_pct + f32::EPSILON))
            .collect();
        if ticks.is_empty() {
            ticks = vec![min_pct, max_pct];
        }
        let labels: Vec<String> = ticks.iter().map(|t| format_pct(*t, step, false)).collect();

        let gutter = Y_AXIS_GUTTER;

        let plot = Rectangle {
            x: 0.0,
            y: 0.0,
            width: (bounds.width - gutter).max(1.0),
            height: (bounds.height - X_AXIS_HEIGHT).max(1.0),
        };

        let visible_min_u64 = min_x;
        let visible_max_u64 = max_x;
        let span_ms = visible_max_u64.saturating_sub(visible_min_u64).max(1);
        let px_per_ms = plot.width / span_ms as f32;

        let map_x = |x: u64| -> f32 {
            let dx = x.saturating_sub(visible_min_u64) as f32;
            plot.x + dx * px_per_ms
        };
        let y_span = (max_pct - min_pct).max(1e-6);
        let map_y = |pct: f32| -> f32 {
            let t = (pct - min_pct) / y_span;
            plot.y + plot.height - t.clamp(0.0, 1.0) * plot.height
        };

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

        let mut end_labels = self.collect_end_labels(
            plot,
            &map_y,
            visible_min_u64,
            visible_max_u64,
            step,
            &line_color_pool,
            &text_color_pool,
            gutter,
        );
        resolve_label_overlaps(&mut end_labels, plot);

        // Plot geometry: vectors only
        let geometry = state.plot_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_main_geometry(
                frame,
                &map_x,
                &map_y,
                &line_color_pool,
                visible_min_u64,
                visible_max_u64,
            );

            let axis_color = palette.background.strongest.color.scale_alpha(0.25);

            // Y-axis baseline/splitter
            let path = {
                let mut b = canvas::path::Builder::new();
                b.move_to(Point::new(plot.x + plot.width, 0.0));
                b.line_to(Point::new(plot.x + plot.width, plot.height));
                b.build()
            };
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(axis_color)
                    .with_width(1.0),
            );
            // X-axis baseline/splitter
            let axis_y = plot.y + plot.height + 0.5;
            let path = {
                let mut b = canvas::path::Builder::new();
                b.move_to(Point::new(plot.x, axis_y));
                b.line_to(Point::new(plot.x + plot.width + gutter, axis_y));
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
            renderer.draw_geometry(geometry);
        });

        // Axis labels layer (under overlay)
        let labels_geo = state.labels_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_y_axis_labels(frame, plot, &ticks, &labels, &map_y, gutter, palette);
            self.fill_x_axis_labels(
                frame,
                plot,
                visible_min_u64,
                visible_max_u64,
                &map_x,
                px_per_ms,
                palette,
            );
        });
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                use iced::advanced::graphics::geometry::Renderer as _;
                renderer.draw_geometry(labels_geo);
            });
        });

        let overlay_geo = state.overlay_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_overlay_y_labels(frame, &end_labels, gutter);
            // Future: other overlay annotations here
        });
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                use iced::advanced::graphics::geometry::Renderer as _;
                renderer.draw_geometry(overlay_geo);
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
        if cursor.is_over(layout.bounds()) {
            let state = state.state.downcast_ref::<State>();
            if state.is_panning {
                advanced::mouse::Interaction::Grabbing
            } else {
                advanced::mouse::Interaction::default()
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
    fn fill_main_geometry<Fx, Fy>(
        &self,
        frame: &mut canvas::Frame,
        map_x: &Fx,
        map_y: &Fy,
        line_color_pool: &[Color; 5],
        min_x: u64,
        max_x: u64,
    ) where
        Fx: Fn(u64) -> f32,
        Fy: Fn(f32) -> f32,
    {
        for (si, s) in self.series.iter().enumerate() {
            let pts = s.points();
            if pts.is_empty() {
                continue;
            }

            let idx_right = pts.iter().position(|(x, _)| *x >= min_x);
            let y0 = match idx_right {
                Some(0) => pts[0].1,
                Some(i) => {
                    let (x0, y0_) = pts[i - 1];
                    let (x1, y1_) = pts[i];
                    let dx = (x1.saturating_sub(x0)) as f32;
                    if dx > 0.0 {
                        let t = (min_x.saturating_sub(x0)) as f32 / dx;
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

            let gap_thresh: u64 = (self.estimated_dt() * GAP_BREAK_MULTIPLIER)
                .max(1.0)
                .round() as u64;

            let mut prev_x: Option<u64> = None;
            match idx_right {
                Some(ir) if ir > 0 => {
                    let px0 = map_x(min_x);
                    let py0 = map_y(0.0);
                    builder.move_to(Point::new(px0, py0));
                    prev_x = Some(min_x);
                }
                Some(0) => {
                    let (fx, fy) = pts[0];
                    if fx <= max_x {
                        let pct = ((fy / y0) - 1.0) * 100.0;
                        builder.move_to(Point::new(map_x(fx), map_y(pct)));
                        prev_x = Some(fx);
                    } else {
                        continue;
                    }
                }
                _ => continue,
            }

            let start_idx = idx_right.unwrap_or(pts.len());

            for (x, y) in pts.iter().skip(start_idx) {
                if *x > max_x {
                    break;
                }
                let pct = ((*y / y0) - 1.0) * 100.0;
                let px = map_x(*x);
                let py = map_y(pct);

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
    ) {
        for label in end_labels {
            frame.fill_rectangle(
                Point {
                    x: label.pos.x,
                    y: label.pos.y - TEXT_SIZE * 0.5 - 2.0,
                },
                Size {
                    width: -gutter,
                    height: TEXT_SIZE + 4.0,
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

    // underlay: y-axis tick labels (text only)
    fn fill_y_axis_labels<Fy>(
        &self,
        frame: &mut canvas::Frame,
        plot: Rectangle,
        ticks: &[f32],
        labels: &[String],
        map_y: &Fy,
        gutter: f32,
        palette: &Extended,
    ) where
        Fy: Fn(f32) -> f32,
    {
        for (i, tick) in ticks.iter().enumerate() {
            let mut y = map_y(*tick);
            let half_txt = TEXT_SIZE * 0.5;
            y = y.clamp(plot.y + half_txt, plot.y + plot.height - half_txt);
            let txt = &labels[i];
            frame.fill_text(canvas::Text {
                content: txt.clone(),
                position: Point::new(plot.x + plot.width + gutter - 4.0, y),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    // underlay, x-axis labels (text only)
    fn fill_x_axis_labels<Fx>(
        &self,
        frame: &mut canvas::Frame,
        plot: Rectangle,
        min_x: u64,
        max_x: u64,
        map_x: &Fx,
        px_per_ms: f32,
        palette: &Extended,
    ) where
        Fx: Fn(u64) -> f32,
    {
        let axis_y = plot.y + plot.height + 0.5;
        let (ticks, step_ms) = time_ticks(min_x, max_x, px_per_ms, MIN_X_TICK_PX);
        let baseline_to_text = 4.0;
        for t in ticks {
            let x = map_x(t).clamp(plot.x, plot.x + plot.width);
            let label = format_time_label(t, step_ms);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x, axis_y + baseline_to_text + 2.0 + TEXT_SIZE * 0.5),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }
}

struct EndLabel {
    pos: Point,
    text: String,
    bg_color: Color,
    text_color: Color,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Zoom(pub usize);

impl Zoom {
    /// "show all data"
    pub fn all() -> Self {
        Self(0)
    }
    pub fn points(n: usize) -> Self {
        Self(n)
    }
    pub fn is_all(self) -> bool {
        self.0 == 0
    }
}

fn format_pct(val: f32, step: f32, show_decimals: bool) -> String {
    if show_decimals {
        if step >= 1.0 {
            format!("{:+.1}%", val)
        } else if step >= 0.1 {
            format!("{:+.2}%", val)
        } else {
            format!("{:+.3}%", val)
        }
    } else if step >= 1.0 {
        format!("{:+.0}%", val)
    } else if step >= 0.1 {
        format!("{:+.1}%", val)
    } else {
        format!("{:+.2}%", val)
    }
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

fn time_tick_candidates() -> &'static [u64] {
    // milliseconds for: 1s, 2s, 5s, 10s, 15s, 30s, 1m, 2m, 5m, 10m, 15m, 30m, 1h, 2h, 4h, 6h, 12h, 1d, 2d, 1w
    const S: u64 = 1_000;
    const M: u64 = 60 * S;
    const H: u64 = 60 * M;
    const D: u64 = 24 * H;
    &[
        S,
        2 * S,
        5 * S,
        10 * S,
        15 * S,
        30 * S, //
        M,
        2 * M,
        5 * M,
        10 * M,
        15 * M,
        30 * M, //
        H,
        2 * H,
        4 * H,
        6 * H,
        12 * H, //
        D,
        2 * D,
        7 * D,
    ]
}

fn time_ticks(min_x: u64, max_x: u64, px_per_ms: f32, min_px: f32) -> (Vec<u64>, u64) {
    let span = max_x.saturating_sub(min_x).max(1);
    let mut step = *time_tick_candidates().first().unwrap_or(&1_000);
    for &candidate in time_tick_candidates() {
        let px = candidate as f32 * px_per_ms;
        if px >= min_px {
            step = candidate;
            break;
        } else {
            step = candidate;
        }
    }
    // Align first tick to the step boundary >= min_x
    let first = if min_x % step == 0 {
        min_x
    } else {
        (min_x / step + 1) * step
    };
    let mut out = Vec::new();
    let mut t = first;
    // Protect against infinite loop
    for _ in 0..=2000 {
        if t > max_x {
            break;
        }
        out.push(t);
        t = t.saturating_add(step);
        if (t - first) > span + step {
            break;
        }
    }
    (out, step)
}

fn format_time_label(ts_ms: u64, step_ms: u64) -> String {
    use chrono::{TimeZone, Utc};
    let Some(dt) = Utc.timestamp_millis_opt(ts_ms as i64).single() else {
        return String::new();
    };
    // Choose format based on step size
    const S: u64 = 1_000;
    const M: u64 = 60 * S;
    const H: u64 = 60 * M;
    const D: u64 = 24 * H;
    if step_ms < M {
        // show seconds
        dt.format("%H:%M:%S").to_string()
    } else if step_ms < D {
        // show time
        dt.format("%H:%M").to_string()
    } else if step_ms < 7 * D {
        // show day and month
        dt.format("%b %d").to_string()
    } else {
        // show date
        dt.format("%Y-%m-%d").to_string()
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
