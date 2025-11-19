use crate::style;
use crate::widget::chart::SeriesLike;
use crate::widget::chart::Zoom;
use crate::widget::chart::domain;

use data::UserTimezone;
use exchange::{TickerInfo, Timeframe};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout, renderer};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{
    Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse, window,
};
use iced_core::renderer::Quad;

use chrono::TimeZone;

const Y_AXIS_GUTTER: f32 = 66.0; // px
const X_AXIS_HEIGHT: f32 = 24.0;

const MIN_X_TICK_PX: f32 = 80.0;
const TEXT_SIZE: f32 = 12.0;

const ZOOM_STEP_PCT: f32 = 0.05; // 5% per scroll "line"

/// Gap breaker to avoid drawing across missing data
const GAP_BREAK_MULTIPLIER: f32 = 3.0;

pub const DEFAULT_ZOOM_POINTS: usize = 150;
pub const MIN_ZOOM_POINTS: usize = 2;
pub const MAX_ZOOM_POINTS: usize = 5000;

const LEGEND_PADDING: f32 = 4.0;
const LEGEND_LINE_H: f32 = TEXT_SIZE + 6.0;

const CHAR_W: f32 = TEXT_SIZE * 0.64;

const ICON_BOX: f32 = TEXT_SIZE + 8.0;
const ICON_SPACING: f32 = 4.0;
const ICON_GAP_AFTER_TEXT: f32 = 8.0;

#[derive(Debug, Clone)]
pub enum LineComparisonEvent {
    ZoomChanged(Zoom),
    PanChanged(f32),
    SeriesCog(TickerInfo),
    SeriesRemove(TickerInfo),
    XAxisDoubleClick,
}

struct State {
    plot_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    is_panning: bool,
    last_cursor: Option<Point>,
    last_cache_rev: u64,
    // Track previous click for double-click detection
    previous_click: Option<iced_core::mouse::Click>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_cache: canvas::Cache::new(),
            y_axis_cache: canvas::Cache::new(),
            x_axis_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
            is_panning: false,
            last_cursor: None,
            last_cache_rev: 0,
            previous_click: None,
        }
    }
}

impl State {
    fn clear_all_caches(&mut self) {
        self.plot_cache.clear();
        self.y_axis_cache.clear();
        self.x_axis_cache.clear();
        self.overlay_cache.clear();
    }
}

pub struct LineComparison<'a, S> {
    series: &'a [S],
    stroke_width: f32,
    zoom: Zoom,
    pan: f32,
    timeframe: Timeframe,
    timezone: UserTimezone,
    version: u64,
}

impl<'a, S> LineComparison<'a, S>
where
    S: SeriesLike,
{
    pub fn new(series: &'a [S], timeframe: Timeframe) -> Self {
        Self {
            series,
            stroke_width: 2.0,
            zoom: Zoom::points(DEFAULT_ZOOM_POINTS),
            timeframe,
            pan: 0.0,
            timezone: UserTimezone::Utc,
            version: 0,
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

    pub fn with_timezone(mut self, tz: UserTimezone) -> Self {
        self.timezone = tz;
        self
    }

    pub fn version(mut self, rev: u64) -> Self {
        self.version = rev;
        self
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

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds()
    }

    fn compute_domains(&self, pan_points: f32) -> Option<((u64, u64), (f32, f32))> {
        if self.series.is_empty() {
            return None;
        }

        let dt = self.dt_ms_est().max(1);
        let all_points: Vec<&[(u64, f32)]> = self.series.iter().map(|s| s.points()).collect();

        let (min_x, max_x) = domain::window(&all_points, self.zoom, pan_points, dt)?;
        let (min_pct, max_pct) = domain::pct_domain(&all_points, min_x, max_x)?;

        Some(((min_x, max_x), (min_pct, max_pct)))
    }

    fn compute_scene(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<Scene> {
        let ((min_x, max_x), (min_pct, max_pct)) = self.compute_domains(self.pan)?;

        let regions = Regions::from_layout(layout);
        let plot = regions.plot;
        let span_ms = max_x.saturating_sub(min_x).max(1) as f32;
        let px_per_ms = if plot.width > 0.0 {
            plot.width / span_ms
        } else {
            1.0
        };

        let ctx = PlotContext {
            regions,
            min_x,
            max_x,
            min_pct,
            max_pct,
            px_per_ms,
        };

        let total_ticks = (plot.height / TEXT_SIZE / 3.).floor() as usize;
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

        let mut end_labels = self.collect_end_labels(&ctx, step);
        let plot_rect = ctx.plot_rect();

        resolve_label_overlaps(&mut end_labels, plot_rect);

        let cursor_root_local = cursor.position_in(layout.bounds());

        let cursor_info: Option<CursorInfo> = if let Some(local) = cursor_root_local {
            match ctx.regions.hit_test(local) {
                HitZone::Plot => {
                    let cx = local.x.clamp(plot_rect.x, plot_rect.x + plot_rect.width);
                    let ms_from_min = ((cx - plot_rect.x) / ctx.px_per_ms).round() as u64;
                    let x_domain_raw = ctx.min_x.saturating_add(ms_from_min);

                    let dt = self.dt_ms_est().max(1);
                    let lower = Self::align_floor(x_domain_raw, dt);
                    let upper = Self::align_ceil(x_domain_raw, dt);
                    let snapped_x = if x_domain_raw.saturating_sub(lower)
                        <= upper.saturating_sub(x_domain_raw)
                    {
                        lower
                    } else {
                        upper
                    }
                    .clamp(ctx.min_x, ctx.max_x);

                    let t = ((local.y - plot_rect.y) / plot_rect.height).clamp(0.0, 1.0);
                    let pct = ctx.min_pct + (1.0 - t) * (ctx.max_pct - ctx.min_pct);
                    Some(CursorInfo {
                        x_domain: snapped_x,
                        y_pct: pct,
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        let show_pct_in_compact = cursor_info.is_some();
        let compact_layout = self.compute_legend_layout(
            &ctx,
            cursor_info.map(|c| c.x_domain),
            step,
            LegendMode::Compact {
                include_pct: show_pct_in_compact,
            },
        );
        let expanded_layout = self.compute_legend_layout(
            &ctx,
            cursor_info.map(|c| c.x_domain),
            step,
            LegendMode::Expanded,
        );

        let mut hovering_legend = false;
        let mut hovered_row: Option<usize> = None;
        let mut hovered_icon: Option<(usize, IconKind)> = None;

        if let Some(local) = cursor_root_local {
            let in_compact = compact_layout
                .as_ref()
                .map(|l| l.bg.contains(local))
                .unwrap_or(false);
            let in_expanded = expanded_layout
                .as_ref()
                .map(|l| l.bg.contains(local))
                .unwrap_or(false);

            if in_compact || in_expanded {
                hovering_legend = true;
            }
        }

        let legend_layout = if hovering_legend {
            expanded_layout.clone()
        } else {
            compact_layout.clone()
        };

        if hovering_legend
            && let (Some(local), Some(layout)) = (cursor_root_local, expanded_layout.as_ref())
        {
            for (i, row) in layout.rows.iter().enumerate() {
                if row.row_rect.contains(local) {
                    hovered_row = Some(i);
                    if row.cog.contains(local) {
                        hovered_icon = Some((i, IconKind::Cog));
                    } else if row.has_close && row.close.contains(local) {
                        hovered_icon = Some((i, IconKind::Close));
                    }
                    break;
                }
            }
        }

        let should_draw_crosshair = !(hovering_legend && hovered_row.is_some());
        let mut reserved_y: Option<Rectangle> = None;
        if should_draw_crosshair && let Some(ci) = cursor_info {
            let plot_rect = ctx.plot_rect();

            let t =
                ((ci.y_pct - ctx.min_pct) / (ctx.max_pct - ctx.min_pct).max(1e-6)).clamp(0.0, 1.0);
            let cy_px = plot_rect.y + plot_rect.height - t * plot_rect.height;

            let pct_str = super::format_pct(ci.y_pct, step, true);
            let pct_est_w = (pct_str.len() as f32) * (TEXT_SIZE * 0.6) + 10.0;

            let gutter_w = ctx.gutter_width();
            let y_w = pct_est_w.clamp(40.0, gutter_w - 8.0);
            let y_h = TEXT_SIZE + 6.0;

            let ylbl_x_right = ctx.regions.y_axis.x + gutter_w - 2.0;
            let ylbl_x = (ylbl_x_right - y_w).max(ctx.regions.y_axis.x + 2.0);
            let ylbl_y = cy_px.clamp(
                plot_rect.y + y_h * 0.5,
                plot_rect.y + plot_rect.height - y_h * 0.5,
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
            end_labels,
            cursor: cursor_info,
            reserved_y,
            y_step: step,
            legend: legend_layout,
            hovering_legend,
            hovered_icon,
            hovered_row,
        })
    }

    fn compute_legend_layout(
        &self,
        ctx: &PlotContext,
        cursor_x: Option<u64>,
        step: f32,
        mode: LegendMode,
    ) -> Option<LegendLayout> {
        if self.series.is_empty() {
            return None;
        }

        let padding = LEGEND_PADDING;
        let line_h = LEGEND_LINE_H;

        let (include_icons, include_pct_in_width) = match mode {
            LegendMode::Expanded => (true, false),
            LegendMode::Compact { include_pct } => (false, include_pct),
        };

        let mut max_chars: usize = 0;
        let mut max_name_chars: usize = 0;
        let mut rows_count: usize = 0;

        for s in self.series.iter() {
            rows_count += 1;

            let name_len = s.ticker_info().ticker.symbol_and_exchange_string().len();
            max_name_chars = max_name_chars.max(name_len);

            let pct_len = if include_pct_in_width {
                domain::interpolate_y_at(s.points(), ctx.min_x)
                    .filter(|&y0| y0 != 0.0)
                    .and_then(|y0| {
                        cursor_x.and_then(|cx| {
                            domain::interpolate_y_at(s.points(), cx).map(|yc| {
                                let pct = ((yc / y0) - 1.0) * 100.0;
                                super::format_pct(pct, step, true)
                            })
                        })
                    })
                    .map(|s| s.len())
                    .unwrap_or(0)
            } else {
                0
            };

            let total = if pct_len > 0 {
                name_len + 1 + pct_len
            } else {
                name_len
            };
            max_chars = max_chars.max(total);
        }

        let text_w = (max_chars as f32) * CHAR_W;

        let icons_pack_w = if include_icons {
            2.0 * ICON_BOX + ICON_SPACING
        } else {
            0.0
        };
        let min_for_icons = if include_icons {
            (max_name_chars as f32) * CHAR_W + ICON_GAP_AFTER_TEXT + icons_pack_w
        } else {
            0.0
        };

        let plot_rect = ctx.plot_rect();

        let bg_w = (text_w.max(min_for_icons) + padding * 2.0)
            .clamp(80.0, (plot_rect.width * 0.6).max(80.0));

        if rows_count == 0 {
            return None;
        }

        let bg_max_h = ((rows_count as f32) * line_h + padding * 2.0)
            .min(plot_rect.height * 0.6)
            .max(line_h + padding * 2.0);

        let max_rows_fit = (((bg_max_h - padding * 2.0) / line_h).floor() as usize).max(1);
        let visible_rows = rows_count.min(max_rows_fit);
        let bg_h = (visible_rows as f32) * line_h + padding * 2.0;

        let bg = Rectangle {
            x: plot_rect.x + 4.0,
            y: plot_rect.y + 4.0,
            width: bg_w,
            height: bg_h,
        };

        let x_left = bg.x + padding;
        let x_right = bg.x + bg.width - padding;

        let mut rows: Vec<LegendRowHit> = Vec::with_capacity(visible_rows);
        let mut row_top = bg.y + padding;

        for (i, s) in self.series.iter().take(visible_rows).enumerate() {
            let y_center = row_top + line_h * 0.5;

            // Base ticker (i == 0) cannot be removed
            let has_close = i != 0;

            let name_len = s.ticker_info().ticker.symbol_and_exchange_string().len() as f32;
            let text_end_x = x_left + name_len * CHAR_W;

            let (cog, close, row_width) = if include_icons {
                let icons_pack_w = if has_close {
                    2.0 * ICON_BOX + ICON_SPACING
                } else {
                    ICON_BOX
                };

                let free_left = text_end_x + ICON_GAP_AFTER_TEXT;
                let free_right = x_right;

                let (cog_left, close_left_opt) = if free_right - free_left >= icons_pack_w {
                    let cog_left = free_left;
                    let close_left_opt = if has_close {
                        Some(cog_left + ICON_BOX + ICON_SPACING)
                    } else {
                        None
                    };
                    (cog_left, close_left_opt)
                } else if has_close {
                    let close_left = free_right - ICON_BOX;
                    let cog_left = (close_left - ICON_SPACING - ICON_BOX).max(free_left);
                    (cog_left, Some(close_left))
                } else {
                    let cog_left = (free_right - ICON_BOX).max(free_left);
                    (cog_left, None)
                };

                let cog = Rectangle {
                    x: cog_left,
                    y: y_center - ICON_BOX * 0.5,
                    width: ICON_BOX,
                    height: ICON_BOX,
                };
                let close = if let Some(cl) = close_left_opt {
                    Rectangle {
                        x: cl,
                        y: y_center - ICON_BOX * 0.5,
                        width: ICON_BOX,
                        height: ICON_BOX,
                    }
                } else {
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    }
                };

                let content_right = if has_close {
                    close.x + close.width
                } else {
                    cog.x + cog.width
                };
                let row_width = (content_right + padding) - bg.x;
                (cog, close, row_width.clamp(0.0, bg.width))
            } else {
                let cog = Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                };
                let close = cog;
                let row_width = (text_end_x + padding) - bg.x;
                (cog, close, row_width.clamp(0.0, bg.width))
            };

            let row_rect = Rectangle {
                x: bg.x,
                y: row_top,
                width: row_width,
                height: line_h,
            };

            rows.push(LegendRowHit {
                ticker: *s.ticker_info(),
                cog,
                close,
                y_center,
                row_rect,
                has_close,
            });
            row_top += line_h;
        }

        Some(LegendLayout { bg, rows })
    }

    fn collect_end_labels(&self, ctx: &PlotContext, step: f32) -> Vec<EndLabel> {
        let mut end_labels: Vec<EndLabel> = Vec::new();
        let plot_height = ctx.plot_rect().height;

        for s in self.series.iter() {
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

            let mut py_local = ctx.map_y(pct_label);
            let half_txt = TEXT_SIZE * 0.5;
            py_local = py_local.clamp(half_txt, plot_height - half_txt);

            let is_color_dark = data::config::theme::is_dark(s.color());
            let text_color = if is_color_dark {
                Color::WHITE
            } else {
                Color::BLACK
            };
            let bg_color = s.color();

            let label_text = super::format_pct(pct_label, step, true);

            end_labels.push(EndLabel {
                pos: Point::new(
                    ctx.regions.y_axis.x + ctx.regions.y_axis.width,
                    ctx.regions.plot.y + py_local,
                ),
                pct_change: label_text,
                bg_color,
                text_color,
                symbol: s.name(),
            });
        }

        end_labels
    }

    fn format_crosshair_time(ts_ms: u64, tz: UserTimezone) -> String {
        let ts_i64 = ts_ms as i64;
        match tz {
            UserTimezone::Utc => {
                if let Some(dt) = chrono::Utc.timestamp_millis_opt(ts_i64).single() {
                    dt.format("%a %b %-d %H:%M").to_string()
                } else {
                    ts_ms.to_string()
                }
            }
            UserTimezone::Local => {
                if let Some(dt) = chrono::Local.timestamp_millis_opt(ts_i64).single() {
                    dt.format("%a %b %-d %H:%M").to_string()
                } else {
                    ts_ms.to_string()
                }
            }
        }
    }

    fn to_tz_ms(ts_ms: u64, tz: UserTimezone) -> u64 {
        match tz {
            UserTimezone::Utc => ts_ms,
            UserTimezone::Local => {
                if let Some(dt) = chrono::Local.timestamp_millis_opt(ts_ms as i64).single() {
                    let off_ms = (dt.offset().local_minus_utc() as i64) * 1000;
                    if off_ms >= 0 {
                        ts_ms.saturating_add(off_ms as u64)
                    } else {
                        ts_ms.saturating_sub((-off_ms) as u64)
                    }
                } else {
                    ts_ms
                }
            }
        }
    }
}

impl<'a, S, M> Widget<M, Theme, Renderer> for LineComparison<'a, S>
where
    S: SeriesLike,
    M: Clone + 'static + From<LineComparisonEvent>,
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
        // Column: [ Row(plot, y_axis) , x_axis ]
        let gutter_w = Y_AXIS_GUTTER;
        let x_axis_h = X_AXIS_HEIGHT;

        // First row: plot + y-axis
        let row_node = layout::next_to_each_other(
            &limits.shrink(Size::new(0.0, x_axis_h)),
            0.0,
            |l| {
                layout::atomic(
                    &l.shrink(Size::new(gutter_w, 0.0)),
                    Length::Fill,
                    Length::Fill,
                )
            },
            |l| layout::atomic(l, gutter_w, Length::Fill),
        );

        // X axis full width at bottom
        let x_axis_node = layout::atomic(limits, Length::Fill, x_axis_h);

        let row_node_height = row_node.size().height;

        let total_w = row_node.size().width;
        let total_h = row_node_height + x_axis_h;

        layout::Node::with_children(
            Size::new(total_w, total_h),
            vec![
                row_node.move_to(Point::new(0.0, 0.0)),
                x_axis_node.move_to(Point::new(0.0, row_node_height)),
            ],
        )
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Mouse(mouse_event) => {
                let state = tree.state.downcast_mut::<State>();
                let bounds = layout.bounds();
                let regions = Regions::from_layout(layout);

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
                        if !matches!(zone, HitZone::Plot) {
                            return;
                        }

                        let zoom_in = *y > 0.0;
                        let new_zoom = self.step_zoom_percent(self.zoom, zoom_in);

                        if new_zoom != self.zoom {
                            shell.publish(M::from(LineComparisonEvent::ZoomChanged(
                                self.normalize_zoom(new_zoom),
                            )));
                            state.clear_all_caches();
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Left) => {
                        if let Some(global_pos) = cursor.position() {
                            let new_click = iced_core::mouse::Click::new(
                                global_pos,
                                mouse::Button::Left,
                                state.previous_click,
                            );

                            if matches!(zone, HitZone::XAxis)
                                && new_click.kind() == iced_core::mouse::click::Kind::Double
                            {
                                shell.publish(M::from(LineComparisonEvent::XAxisDoubleClick));
                                state.clear_all_caches();
                                state.previous_click = Some(new_click);
                                return;
                            }

                            state.previous_click = Some(new_click);
                        } else {
                            state.previous_click = None;
                        }

                        if matches!(zone, HitZone::XAxis) {
                            return;
                        }

                        if let Some(scene) = self.compute_scene(layout, cursor)
                            && let Some(legend) = scene.legend.as_ref()
                        {
                            for row in &legend.rows {
                                if row.cog.contains(cursor_pos) {
                                    shell.publish(M::from(LineComparisonEvent::SeriesCog(
                                        row.ticker,
                                    )));
                                    state.clear_all_caches();
                                    return;
                                }
                                if row.has_close && row.close.contains(cursor_pos) {
                                    shell.publish(M::from(LineComparisonEvent::SeriesRemove(
                                        row.ticker,
                                    )));
                                    state.clear_all_caches();
                                    return;
                                }
                            }
                        }

                        if matches!(zone, HitZone::Plot) {
                            state.is_panning = true;
                            state.last_cursor = Some(cursor_pos);
                        }
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        state.is_panning = false;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        if state.is_panning {
                            let prev = state.last_cursor.unwrap_or(cursor_pos);
                            let dx_px = cursor_pos.x - prev.x;

                            if dx_px.abs() > 0.0 {
                                let x_span = self.current_x_span(); // in milliseconds
                                let plot_w = regions.plot.width.max(1.0);
                                let dx_ms = -(dx_px) * (x_span / plot_w);
                                let dt = self.dt_ms_est().max(1) as f32;
                                let dx_pts = dx_ms / dt;

                                let event = LineComparisonEvent::PanChanged(self.pan + dx_pts);

                                shell.publish(M::from(event));
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
            Event::Window(window::Event::RedrawRequested(_)) => {
                let state = tree.state.downcast_mut::<State>();

                if state.last_cache_rev != self.version {
                    state.clear_all_caches();
                    state.last_cache_rev = self.version;
                }
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
        let Some(scene) = self.compute_scene(layout, cursor) else {
            return;
        };

        let bounds = layout.bounds();
        let palette = theme.extended_palette();

        renderer.with_translation(Vector::new(bounds.x, bounds.y), |r| {
            let plot_rect = scene.ctx.plot_rect();

            let plot_geom = state.plot_cache.draw(r, plot_rect.size(), |frame| {
                self.fill_main_geometry(frame, &scene.ctx);
            });

            let splitter_color = palette.background.strong.color.scale_alpha(0.25);
            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: plot_rect.x,
                        y: plot_rect.y + plot_rect.height,
                        width: plot_rect.width + scene.ctx.regions.y_axis.width,
                        height: 1.0,
                    },
                    snap: true,
                    ..Default::default()
                },
                splitter_color,
            );
            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: plot_rect.x + plot_rect.width,
                        y: plot_rect.y,
                        width: 1.0,
                        height: plot_rect.height,
                    },
                    snap: true,
                    ..Default::default()
                },
                splitter_color,
            );

            let y_rect = scene.ctx.regions.y_axis;
            let y_geom = state.y_axis_cache.draw(r, y_rect.size(), |frame| {
                self.fill_y_axis_labels(
                    frame,
                    &scene.ctx,
                    &scene.y_ticks,
                    &scene.y_labels,
                    palette,
                );
            });

            let x_rect = scene.ctx.regions.x_axis;
            let x_geom = state.x_axis_cache.draw(r, x_rect.size(), |frame| {
                self.fill_x_axis_labels(frame, &scene.ctx, palette);
            });

            let overlay_geom = state.overlay_cache.draw(r, bounds.size(), |frame| {
                self.fill_overlay_y_labels(
                    frame,
                    &scene.end_labels,
                    scene.ctx.regions.y_axis.x,
                    scene.ctx.gutter_width(),
                    scene.reserved_y.as_ref(),
                );
                self.fill_top_left_legend(
                    frame,
                    &scene.ctx,
                    if scene.hovering_legend {
                        None
                    } else {
                        scene.cursor.map(|c| c.x_domain)
                    },
                    palette,
                    scene.y_step,
                    scene.legend.as_ref(),
                    scene.hovering_legend,
                    scene.hovered_icon,
                    scene.hovered_row,
                );
                if !(scene.hovering_legend && scene.hovered_row.is_some()) {
                    self.fill_crosshair(frame, &scene, palette);
                }
            });

            r.with_translation(Vector::new(plot_rect.x, plot_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(plot_geom);
            });
            r.with_translation(Vector::new(y_rect.x, y_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(y_geom);
            });
            r.with_translation(Vector::new(x_rect.x, x_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(x_geom);
            });

            r.with_layer(
                Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: bounds.width,
                    height: bounds.height,
                },
                |r| {
                    use iced::advanced::graphics::geometry::Renderer as _;
                    r.draw_geometry(overlay_geom);
                },
            );
        });
    }

    fn mouse_interaction(
        &self,
        _state: &Tree,
        layout: Layout<'_>,
        cursor: advanced::mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> advanced::mouse::Interaction {
        if let Some(cursor_in_layout) = cursor.position_in(layout.bounds()) {
            if let Some(scene) = self.compute_scene(layout, cursor) {
                if let Some(legend) = scene.legend.as_ref() {
                    for row in &legend.rows {
                        if row.cog.contains(cursor_in_layout)
                            || (row.has_close && row.close.contains(cursor_in_layout))
                        {
                            return advanced::mouse::Interaction::Pointer;
                        }
                    }
                }

                if scene.hovering_legend && scene.hovered_row.is_some() {
                    return advanced::mouse::Interaction::default();
                }

                let state = _state.state.downcast_ref::<State>();
                if state.is_panning {
                    return advanced::mouse::Interaction::Grabbing;
                }

                match scene.ctx.regions.hit_test(cursor_in_layout) {
                    HitZone::Plot => advanced::mouse::Interaction::Crosshair,
                    _ => advanced::mouse::Interaction::default(),
                }
            } else {
                advanced::mouse::Interaction::default()
            }
        } else {
            advanced::mouse::Interaction::default()
        }
    }
}

impl<'a, S> LineComparison<'a, S>
where
    S: SeriesLike,
{
    #[allow(unused_assignments)]
    fn fill_main_geometry(&self, frame: &mut canvas::Frame, ctx: &PlotContext) {
        for s in self.series.iter() {
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
                    .with_color(s.color())
                    .with_width(self.stroke_width),
            );
        }
    }

    fn fill_overlay_y_labels(
        &self,
        frame: &mut canvas::Frame,
        end_labels: &[EndLabel],
        plot_right_x: f32,
        gutter: f32,
        reserved_y: Option<&Rectangle>,
    ) {
        let split_x = plot_right_x;

        for label in end_labels {
            let label_h = TEXT_SIZE + 4.0;

            let rect = Rectangle {
                x: split_x + 2.0,
                y: label.pos.y - TEXT_SIZE * 0.5 - 2.0,
                width: (gutter - 1.0).max(0.0),
                height: label_h,
            };

            let intersects_reserved = reserved_y.map(|res| rect.intersects(res)).unwrap_or(false);

            if !intersects_reserved {
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
                    content: label.pct_change.clone(),
                    position: label.pos - Vector::new(4.0, 0.0),
                    color: label.text_color,
                    size: TEXT_SIZE.into(),
                    font: style::AZERET_MONO,
                    align_x: iced::Alignment::End.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }

            let sym_right = split_x - 1.0;
            let sym_h = TEXT_SIZE + 4.0;
            let sym_w = (label.symbol.len() as f32) * CHAR_W + 8.0;
            let sym_rect = Rectangle {
                x: sym_right - sym_w,
                y: label.pos.y - sym_h * 0.5,
                width: sym_w,
                height: sym_h,
            };

            frame.fill_rectangle(
                Point::new(sym_rect.x, sym_rect.y),
                Size::new(sym_rect.width, sym_rect.height),
                label.bg_color,
            );
            frame.fill_text(canvas::Text {
                content: label.symbol.clone(),
                position: Point::new(sym_rect.x + sym_rect.width - 4.0, label.pos.y),
                color: label.text_color,
                size: TEXT_SIZE.into(),
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
        palette: &Extended,
    ) {
        let plot = ctx.plot_rect();
        for (i, tick) in ticks.iter().enumerate() {
            let mut y_local = ctx.map_y(*tick);
            let half_txt = TEXT_SIZE * 0.5;
            y_local = y_local.clamp(half_txt, plot.height - half_txt);

            let right_x = ctx.gutter_width() - 4.0;
            frame.fill_text(canvas::Text {
                content: labels[i].clone(),
                position: Point::new(right_x, y_local),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    fn fill_x_axis_labels(&self, frame: &mut canvas::Frame, ctx: &PlotContext, palette: &Extended) {
        let (ticks, step_ms) =
            super::time_ticks(ctx.min_x, ctx.max_x, ctx.px_per_ms, MIN_X_TICK_PX);

        let baseline_to_text = 4.0;
        let y_center_local = baseline_to_text + 2.0 + TEXT_SIZE * 0.5;

        let plot_rect = ctx.plot_rect();

        let mut last_right = f32::NEG_INFINITY;
        for t in ticks {
            let x_local = ctx.map_x(t).clamp(0.0, plot_rect.width);

            let label_ts = Self::to_tz_ms(t, self.timezone);
            let label = super::format_time_label(label_ts, step_ms);

            let est_w = (label.len() as f32) * CHAR_W + 8.0;
            let left = x_local - est_w * 0.5;
            let right = x_local + est_w * 0.5;

            if left <= last_right {
                continue;
            }

            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x_local, y_center_local),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });

            last_right = right;
        }
    }

    fn fill_top_left_legend(
        &self,
        frame: &mut canvas::Frame,
        ctx: &PlotContext,
        cursor_x: Option<u64>,
        palette: &Extended,
        step: f32,
        legend_layout: Option<&LegendLayout>,
        hovering_legend: bool,
        hovered_icon: Option<(usize, IconKind)>,
        hovered_row: Option<usize>,
    ) {
        let padding = LEGEND_PADDING;
        let line_h = LEGEND_LINE_H;
        let show_buttons = hovering_legend;

        let icon_normal = palette.background.base.text;
        let icon_hover = palette.background.strongest.text;
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        if let Some(layout) = legend_layout {
            frame.fill_rectangle(
                Point::new(layout.bg.x, layout.bg.y),
                Size::new(layout.bg.width, layout.bg.height),
                palette.background.weakest.color.scale_alpha(0.9),
            );

            let x0 = layout.bg.x + padding;

            for (i, s) in self.series.iter().take(layout.rows.len()).enumerate() {
                let row = &layout.rows[i];
                let y = (row.y_center).round() + 0.0;

                if show_buttons && hovered_row == Some(i) {
                    let hl = Rectangle {
                        x: row.row_rect.x + 1.0,
                        y: row.row_rect.y,
                        width: (row.row_rect.width - 2.0).max(0.0),
                        height: row.row_rect.height,
                    };
                    frame.fill_rectangle(hl.position(), hl.size(), row_hover_fill);
                }

                let pct_str = if hovering_legend {
                    None
                } else {
                    domain::interpolate_y_at(s.points(), ctx.min_x)
                        .filter(|&y0| y0 != 0.0)
                        .and_then(|y0| {
                            cursor_x.and_then(|cx| {
                                domain::interpolate_y_at(s.points(), cx).map(|yc| {
                                    let pct = ((yc / y0) - 1.0) * 100.0;
                                    super::format_pct(pct, step, true)
                                })
                            })
                        })
                };

                let symbol_and_exchange = s.ticker_info().ticker.symbol_and_exchange_string();
                let content = if let Some(pct) = pct_str {
                    format!("{symbol_and_exchange} {pct}")
                } else {
                    symbol_and_exchange
                };

                frame.fill_text(canvas::Text {
                    content,
                    position: Point::new(x0, y),
                    color: s.color(),
                    size: TEXT_SIZE.into(),
                    font: style::AZERET_MONO,
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });

                if show_buttons {
                    let (cog_col, close_col) = match hovered_icon {
                        Some((hi, IconKind::Cog)) if hi == i => (icon_hover, icon_normal),
                        Some((hi, IconKind::Close)) if hi == i => (icon_normal, icon_hover),
                        _ => (icon_normal, icon_normal),
                    };

                    frame.fill_text(canvas::Text {
                        content: char::from(style::Icon::Cog).to_string(),
                        position: Point {
                            x: row.cog.center_x(),
                            y,
                        },
                        color: cog_col,
                        size: TEXT_SIZE.into(),
                        font: style::ICONS_FONT,
                        align_x: iced::Alignment::Center.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });

                    if row.has_close {
                        frame.fill_text(canvas::Text {
                            content: char::from(style::Icon::Close).to_string(),
                            position: Point {
                                x: row.close.center_x(),
                                y,
                            },
                            color: close_col,
                            size: TEXT_SIZE.into(),
                            font: style::ICONS_FONT,
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            ..Default::default()
                        });
                    }
                }
            }
            return;
        }

        let mut max_chars: usize = 0;
        let mut rows_count: usize = 0;

        for s in self.series.iter() {
            rows_count += 1;

            let pct_len = if hovering_legend {
                0
            } else {
                domain::interpolate_y_at(s.points(), ctx.min_x)
                    .filter(|&y0| y0 != 0.0)
                    .and_then(|y0| {
                        cursor_x.and_then(|cx| {
                            domain::interpolate_y_at(s.points(), cx).map(|yc| {
                                let pct = ((yc / y0) - 1.0) * 100.0;
                                super::format_pct(pct, step, true)
                            })
                        })
                    })
                    .map(|s| s.len())
                    .unwrap_or(0)
            };

            let name_len = s.ticker_info().ticker.symbol_and_exchange_string().len();
            let total = if pct_len > 0 {
                name_len + 1 + pct_len
            } else {
                name_len
            };
            if total > max_chars {
                max_chars = total;
            }
        }

        let plot_rect = ctx.plot_rect();

        let max_chars_f = max_chars as f32;
        let char_w = TEXT_SIZE * 0.64;
        let text_w = max_chars_f * char_w;
        let bg_w = (text_w + padding * 2.0).clamp(80.0, (plot_rect.width * 0.6).max(80.0));

        let rows_count_f = rows_count as f32;
        if rows_count_f > 0.0 {
            let bg_h = (rows_count_f * line_h + padding * 2.0).min(plot_rect.height * 0.6);
            frame.fill_rectangle(
                Point::new(plot_rect.x + 4.0, plot_rect.y + 4.0),
                Size::new(bg_w, bg_h),
                palette.background.weakest.color.scale_alpha(0.9),
            );
        }

        let mut y = plot_rect.y + padding + TEXT_SIZE * 0.5;
        let x0 = plot_rect.x + padding;

        for s in self.series.iter() {
            if y > plot_rect.y + plot_rect.height - TEXT_SIZE {
                break;
            }

            let pct_str = if hovering_legend {
                None
            } else {
                domain::interpolate_y_at(s.points(), ctx.min_x)
                    .filter(|&y0| y0 != 0.0)
                    .and_then(|y0| {
                        cursor_x.and_then(|cx| {
                            domain::interpolate_y_at(s.points(), cx).map(|yc| {
                                let pct = ((yc / y0) - 1.0) * 100.0;
                                super::format_pct(pct, step, true)
                            })
                        })
                    })
            };

            let symbol_and_exchange = s.ticker_info().ticker.symbol_and_exchange_string();
            let content = if let Some(pct) = pct_str {
                format!("{symbol_and_exchange} {pct}")
            } else {
                symbol_and_exchange
            };

            frame.fill_text(canvas::Text {
                content,
                position: Point::new(x0, y),
                color: s.color(),
                size: TEXT_SIZE.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });

            y += line_h;
        }
    }

    fn fill_crosshair(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let Some(ci) = scene.cursor else {
            return;
        };
        let ctx = &scene.ctx;
        let plot_rect = ctx.plot_rect();

        let cx = {
            let dx = ci.x_domain.saturating_sub(ctx.min_x) as f32;
            plot_rect.x + dx * ctx.px_per_ms
        };
        let y_span = (ctx.max_pct - ctx.min_pct).max(1e-6);
        let t = ((ci.y_pct - ctx.min_pct) / y_span).clamp(0.0, 1.0);
        let cy = plot_rect.y + plot_rect.height - t * plot_rect.height;

        let stroke = style::dashed_line_from_palette(palette);

        // Vertical
        let mut b = canvas::path::Builder::new();
        b.move_to(Point::new(cx, plot_rect.y));
        b.line_to(Point::new(cx, plot_rect.y + plot_rect.height));
        frame.stroke(&b.build(), stroke);

        // Horizontal
        let mut b = canvas::path::Builder::new();
        b.move_to(Point::new(plot_rect.x, cy));
        b.line_to(Point::new(plot_rect.x + plot_rect.width, cy));
        frame.stroke(&b.build(), stroke);

        let time_str = Self::format_crosshair_time(ci.x_domain, self.timezone);

        let text_col = palette.secondary.base.text;
        let bg_col = palette.secondary.base.color;

        let est_w = (time_str.len() as f32) * (TEXT_SIZE * 0.67) + 12.0;
        let label_w = est_w.clamp(100.0, 240.0);
        let label_h = TEXT_SIZE + 6.0;

        let time_x = cx.clamp(
            plot_rect.x + label_w * 0.5,
            plot_rect.x + plot_rect.width - label_w * 0.5,
        );
        let time_y = plot_rect.y + plot_rect.height + 2.0 + label_h * 0.5;

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

        let gutter = ctx.gutter_width();
        let pct_str = super::format_pct(ci.y_pct, scene.y_step, true);
        let label_h = TEXT_SIZE + 6.0;

        let split_x = plot_rect.x + plot_rect.width;
        let gutter_right = split_x + gutter;

        let ylbl_x_right = gutter_right;
        let ylbl_y = cy.clamp(
            plot_rect.y + label_h * 0.5,
            plot_rect.y + plot_rect.height - label_h * 0.5,
        );

        frame.fill_rectangle(
            Point::new(split_x + 2.0, ylbl_y - label_h * 0.5),
            Size::new((gutter - 1.0).max(0.0), label_h),
            bg_col,
        );
        frame.fill_text(canvas::Text {
            content: pct_str,
            position: Point::new(ylbl_x_right - 4.0, ylbl_y),
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
    bg_color: Color,
    text_color: Color,
    pct_change: String,
    symbol: String,
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

impl<'a, S, M> From<LineComparison<'a, S>> for Element<'a, M, Theme, Renderer>
where
    S: SeriesLike,
    M: Clone + 'a + 'static + From<LineComparisonEvent>,
{
    fn from(chart: LineComparison<'a, S>) -> Self {
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
    fn from_layout(root: Layout<'_>) -> Self {
        let root_bounds = root.bounds();

        // root.children = [ row, x_axis ]
        let row = root.child(0);
        let x_abs = root.child(1).bounds();

        // row.children  = [ plot, y_axis ]
        let plot_abs = row.child(0).bounds();
        let y_abs = row.child(1).bounds();

        let to_local = |r: Rectangle| Rectangle {
            x: r.x - root_bounds.x,
            y: r.y - root_bounds.y,
            width: r.width,
            height: r.height,
        };

        Regions {
            plot: to_local(plot_abs),
            y_axis: to_local(y_abs),
            x_axis: to_local(x_abs),
        }
    }

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
    min_x: u64,
    max_x: u64,
    min_pct: f32,
    max_pct: f32,
    px_per_ms: f32,
}

impl PlotContext {
    fn plot_rect(&self) -> Rectangle {
        self.regions.plot
    }

    fn gutter_width(&self) -> f32 {
        self.regions.y_axis.width
    }

    fn map_x(&self, x: u64) -> f32 {
        let dx = x.saturating_sub(self.min_x) as f32;
        dx * self.px_per_ms
    }

    fn map_y(&self, pct: f32) -> f32 {
        let span = (self.max_pct - self.min_pct).max(1e-6);
        let t = (pct - self.min_pct) / span;
        let plot = self.plot_rect();
        plot.height - t.clamp(0.0, 1.0) * plot.height
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
    end_labels: Vec<EndLabel>,
    cursor: Option<CursorInfo>,
    reserved_y: Option<Rectangle>,
    y_step: f32,
    legend: Option<LegendLayout>,
    hovering_legend: bool,
    hovered_icon: Option<(usize, IconKind)>,
    hovered_row: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct LegendRowHit {
    ticker: TickerInfo,
    cog: Rectangle,
    close: Rectangle,
    y_center: f32,
    row_rect: Rectangle,
    has_close: bool,
}

#[derive(Debug, Clone)]
struct LegendLayout {
    bg: Rectangle,
    rows: Vec<LegendRowHit>,
}

#[derive(Debug, Clone, Copy)]
enum LegendMode {
    Compact { include_pct: bool },
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Cog,
    Close,
}
