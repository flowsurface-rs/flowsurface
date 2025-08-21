use crate::chart::{Basis, Caches, Interaction, Message, ViewState};
use crate::style::{self, dashed_line};
use data::util::{guesstimate_ticks, round_to_tick};

use iced::widget::canvas::{self, Cache, Geometry, Path, Stroke};
use iced::widget::{Canvas, container, row, vertical_rule};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

pub trait Series {
    type Y;
    type RangeIter<'a>: Iterator<Item = (u64, &'a Self::Y)>
    where
        Self: 'a;

    fn range_iter<'a>(&'a self, range: RangeInclusive<u64>) -> Self::RangeIter<'a>;

    fn at(&self, x: u64) -> Option<&Self::Y>;

    fn next_after<'a>(&'a self, x: u64) -> Option<(u64, &'a Self::Y)>
    where
        Self: 'a;
}

pub struct BTreeRangeIter<'a, Y> {
    inner: std::collections::btree_map::Range<'a, u64, Y>,
}
impl<'a, Y> Iterator for BTreeRangeIter<'a, Y> {
    type Item = (u64, &'a Y);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, v)| (*k, v))
    }
}

impl<Y> Series for BTreeMap<u64, Y> {
    type Y = Y;
    type RangeIter<'a>
        = BTreeRangeIter<'a, Y>
    where
        Self: 'a;

    fn range_iter<'a>(&'a self, range: RangeInclusive<u64>) -> Self::RangeIter<'a> {
        BTreeRangeIter {
            inner: self.range(range),
        }
    }

    fn at(&self, x: u64) -> Option<&Self::Y> {
        self.get(&x)
    }

    fn next_after<'a>(&'a self, x: u64) -> Option<(u64, &'a Self::Y)>
    where
        Self: 'a,
    {
        self.range((x + 1)..).next().map(|(k, v)| (*k, v))
    }
}

pub struct YScale {
    pub min: f32,
    pub max: f32,
    pub px_height: f32,
}
impl YScale {
    pub fn to_y(&self, v: f32) -> f32 {
        if self.max <= self.min {
            self.px_height
        } else {
            self.px_height - ((v - self.min) / (self.max - self.min)) * self.px_height
        }
    }
}

pub trait Plot<S: Series> {
    fn y_extents(&self, s: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)>;

    fn adjust_extents(&self, min: f32, max: f32) -> (f32, f32) {
        (min, max)
    }

    fn draw<'a>(
        &'a self,
        frame: &'a mut canvas::Frame,
        ctx: &'a ViewState,
        theme: &Theme,
        s: &S,
        range: RangeInclusive<u64>,
        scale: &YScale,
    );

    fn tooltip(&self, y: &S::Y, next: Option<&S::Y>, theme: &Theme) -> PlotTooltip;
}

pub struct ChartCanvas<'a, P, S>
where
    P: Plot<S>,
    S: Series,
{
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    pub ctx: &'a ViewState,
    pub plot: P,
    pub series: &'a S,
    pub max_for_labels: f32,
    pub min_for_labels: f32,
}

impl<P, S> canvas::Program<Message> for ChartCanvas<'_, P, S>
where
    P: Plot<S>,
    S: Series,
{
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let msg = matches!(*interaction, Interaction::None)
                    .then(|| cursor.is_over(bounds))
                    .and_then(|over| over.then_some(Message::CrosshairMoved));
                let action = msg.map_or(canvas::Action::request_redraw(), canvas::Action::publish);
                Some(match interaction {
                    Interaction::None => action,
                    _ => action.and_capture(),
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let ctx = &self.ctx;
        if ctx.bounds.width == 0.0 {
            return vec![];
        }

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let indicator = self.indicator_cache.draw(renderer, bounds.size(), |frame| {
            frame.translate(center);
            frame.scale(ctx.scaling);
            frame.translate(Vector::new(
                ctx.translation.x,
                (-bounds.height / ctx.scaling) / 2.0,
            ));

            let width = frame.width() / ctx.scaling;
            let region = Rectangle {
                x: -ctx.translation.x - width / 2.0,
                y: 0.0,
                width,
                height: frame.height() / ctx.scaling,
            };
            let (earliest, latest) = ctx.interval_range(&region);
            if latest < earliest {
                return;
            }

            let scale = YScale {
                min: self.min_for_labels,
                max: self.max_for_labels,
                px_height: frame.height() / ctx.scaling,
            };

            self.plot
                .draw(frame, ctx, theme, self.series, earliest..=latest, &scale);
        });

        let crosshair = self.crosshair_cache.draw(renderer, bounds.size(), |frame| {
            let dashed = dashed_line(theme);
            if let Some(cursor_position) = cursor.position_in(ctx.bounds) {
                // vertical snap by basis
                let width = frame.width() / ctx.scaling;
                let region = Rectangle {
                    x: -ctx.translation.x - width / 2.0,
                    y: 0.0,
                    width,
                    height: frame.height() / ctx.scaling,
                };
                let earliest = ctx.x_to_interval(region.x) as f64;
                let latest = ctx.x_to_interval(region.x + region.width) as f64;

                let crosshair_ratio = f64::from(cursor_position.x / bounds.width);
                let rounded_x = match ctx.basis {
                    Basis::Time(tf) => {
                        let step = tf.to_milliseconds() as f64;
                        ((earliest + crosshair_ratio * (latest - earliest)) / step).round() as u64
                            * step as u64
                    }
                    Basis::Tick(_) => {
                        let chart_x_min = region.x;
                        let crosshair_pos = chart_x_min + (crosshair_ratio as f32) * region.width;
                        let idx = (crosshair_pos / ctx.cell_width).round();
                        ctx.x_to_interval(idx * ctx.cell_width)
                    }
                };
                let snap_ratio = ((rounded_x as f64 - earliest) / (latest - earliest)) as f32;

                frame.stroke(
                    &Path::line(
                        Point::new(snap_ratio * bounds.width, 0.0),
                        Point::new(snap_ratio * bounds.width, bounds.height),
                    ),
                    dashed,
                );

                // tooltip text
                if let Some(y) = self.series.at(rounded_x) {
                    let next = self.series.next_after(rounded_x).map(|(_, v)| v);

                    let plot_tooltip = self.plot.tooltip(y, next, theme);
                    let (tooltip_w, tooltip_h) = plot_tooltip.guesstimate();

                    let palette = theme.extended_palette();

                    frame.fill_rectangle(
                        Point::new(4.0, 0.0),
                        Size::new(tooltip_w, tooltip_h),
                        palette.background.weakest.color.scale_alpha(0.9),
                    );
                    frame.fill_text(canvas::Text {
                        content: plot_tooltip.text,
                        position: Point::new(8.0, 2.0),
                        size: iced::Pixels(10.0),
                        color: palette.background.base.text,
                        font: style::AZERET_MONO,
                        ..canvas::Text::default()
                    });
                }
            } else if let Some(cursor_position) = cursor.position_in(bounds) {
                // horizontal snap uses label extents
                let highest = self.max_for_labels;
                let lowest = self.min_for_labels;
                let tick = guesstimate_ticks(highest - lowest);

                let ratio = cursor_position.y / bounds.height;
                let value = highest + ratio * (lowest - highest);
                let rounded = round_to_tick(value, tick);
                let snap_ratio = (rounded - highest) / (lowest - highest);

                frame.stroke(
                    &Path::line(
                        Point::new(0.0, snap_ratio * bounds.height),
                        Point::new(bounds.width, snap_ratio * bounds.height),
                    ),
                    dashed,
                );
            }
        });

        vec![indicator, crosshair]
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
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}

pub fn indicator_row<'a, P, S>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    plot: P,
    series: &'a S,
    visible_range: RangeInclusive<u64>,
) -> Element<'a, Message>
where
    P: Plot<S> + 'a,
    S: Series + 'a,
{
    let (min, max) = plot
        .y_extents(series, visible_range)
        .map(|(min, max)| plot.adjust_extents(min, max))
        .unwrap_or((0.0, 0.0));

    let canvas = Canvas::new(ChartCanvas::<P, S> {
        indicator_cache: &cache.main,
        crosshair_cache: &cache.crosshair,
        ctx: chart_state,
        plot,
        series,
        max_for_labels: max,
        min_for_labels: min,
    })
    .height(Length::Fill)
    .width(Length::Fill);

    let labels = Canvas::new(super::IndicatorLabel {
        label_cache: &cache.y_labels,
        max,
        min,
        chart_bounds: chart_state.bounds,
    })
    .height(Length::Fill)
    .width(chart_state.y_labels_width());

    row![
        canvas,
        vertical_rule(1).style(style::split_ruler),
        container(labels),
    ]
    .into()
}

#[derive(Clone, Copy)]
/// What kind of bar to render, and whether it carries a signed overlay.
/// The sign of `overlay` selects up (success) vs down (danger).
pub enum BarClass {
    /// draw a single bar using secondary strong color
    Single,
    /// draw two bars, a success/danger colored (alpha) and an overlay using full color.
    Overlay { overlay: f32 }, // signed; sign decides color
}

#[derive(Clone, Copy)]
#[allow(unused)]
/// How to anchor bar heights.
pub enum Baseline {
    /// Use zero as baseline (classic volume). Extents: [0, max].
    Zero,
    /// Use the minimum value in the visible range. Extents: [min, max].
    Min,
    /// Use a fixed numeric baseline.
    Fixed(f32),
}

pub struct BarPlot<MH, CL, TT> {
    pub bar_width_factor: f32,
    pub padding: f32,
    pub main_height: MH, // main/total value
    pub classify: CL,    // Single vs Overlay with signed overlay
    pub tooltip: TT,     // tooltip formatter
    pub baseline: Baseline,
}

impl<MH, CL, TT> BarPlot<MH, CL, TT> {
    pub fn new(main_height: MH, classify: CL, tooltip: TT) -> Self {
        Self {
            bar_width_factor: 0.9,
            padding: 0.0,
            main_height,
            classify,
            tooltip,
            baseline: Baseline::Zero,
        }
    }

    pub fn bar_width_factor(mut self, f: f32) -> Self {
        self.bar_width_factor = f;
        self
    }

    pub fn padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }

    #[allow(unused)]
    pub fn baseline(mut self, b: Baseline) -> Self {
        self.baseline = b;
        self
    }
}

impl<S, MH, CL, TT> Plot<S> for BarPlot<MH, CL, TT>
where
    S: Series,
    MH: Fn(&S::Y) -> f32 + Copy,
    CL: Fn(&S::Y) -> BarClass + Copy,
    TT: Fn(&S::Y, Option<&S::Y>) -> PlotTooltip + Copy,
{
    fn y_extents(&self, s: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        let mut min_v = f32::MAX;
        let mut max_v = f32::MIN;
        let mut n = 0u32;

        for (_, y) in s.range_iter(range.clone()) {
            let v = (self.main_height)(y);
            if v < min_v {
                min_v = v;
            }
            if v > max_v {
                max_v = v;
            }
            n += 1;
        }

        if n == 0 || (max_v <= 0.0 && matches!(self.baseline, Baseline::Zero)) {
            return None;
        }

        let min_ext = match self.baseline {
            Baseline::Zero => 0.0,
            Baseline::Min => min_v,
            Baseline::Fixed(v) => v,
        };

        let lo = min_ext;
        let mut hi = max_v.max(min_ext + f32::EPSILON);
        if hi > lo && self.padding > 0.0 {
            hi *= 1.0 + self.padding;
        }

        Some((lo, hi))
    }

    fn adjust_extents(&self, min: f32, max: f32) -> (f32, f32) {
        (min, max)
    }

    fn draw<'a>(
        &self,
        frame: &mut canvas::Frame,
        ctx: &'a ViewState,
        theme: &Theme,
        s: &S,
        range: RangeInclusive<u64>,
        scale: &YScale,
    ) {
        let palette = theme.extended_palette();
        let bar_width = ctx.cell_width * self.bar_width_factor;

        let baseline_value = match self.baseline {
            Baseline::Zero => 0.0,
            Baseline::Min => scale.min, // extents min
            Baseline::Fixed(v) => v,
        };
        let y_base = scale.to_y(baseline_value);

        for (x, y) in s.range_iter(range.clone()) {
            let left = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);

            let total = (self.main_height)(y);
            let rel = total - baseline_value;

            let (top_y, h_total) = if rel > 0.0 {
                let y_total = scale.to_y(total);
                let h = (y_base - y_total).max(0.0);
                (y_total, h)
            } else {
                (y_base, 0.0)
            };
            if h_total <= 0.0 {
                continue;
            }

            match (self.classify)(y) {
                BarClass::Single => {
                    frame.fill_rectangle(
                        Point::new(left, top_y),
                        Size::new(bar_width, h_total),
                        palette.secondary.strong.color,
                    );
                }
                BarClass::Overlay { overlay } => {
                    let up = overlay >= 0.0;
                    let base_color = if up {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };

                    frame.fill_rectangle(
                        Point::new(left, top_y),
                        Size::new(bar_width, h_total),
                        base_color.scale_alpha(0.3),
                    );

                    let ov_abs = overlay.abs().max(0.0);
                    if ov_abs > 0.0 {
                        let y_overlay = scale.to_y(baseline_value + ov_abs);
                        let h_overlay = (y_base - y_overlay).max(0.0);
                        if h_overlay > 0.0 {
                            frame.fill_rectangle(
                                Point::new(left, y_overlay),
                                Size::new(bar_width, h_overlay),
                                base_color,
                            );
                        }
                    }
                }
            }
        }
    }

    fn tooltip(&self, y: &S::Y, next: Option<&S::Y>, _theme: &iced::Theme) -> PlotTooltip {
        (self.tooltip)(y, next)
    }
}

pub struct LinePlot<M, TT> {
    pub map_y: M,
    pub tooltip: TT,
    pub padding: f32,
    pub stroke_width: f32,
    pub show_points: bool,
    pub point_radius_factor: f32,
}

impl<M, TT> LinePlot<M, TT> {
    pub fn new(map_y: M, tooltip: TT) -> Self {
        Self {
            map_y,
            tooltip,
            padding: 0.08,
            stroke_width: 1.0,
            show_points: true,
            point_radius_factor: 0.2,
        }
    }
    pub fn padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }

    pub fn stroke_width(mut self, w: f32) -> Self {
        self.stroke_width = w;
        self
    }

    pub fn show_points(mut self, on: bool) -> Self {
        self.show_points = on;
        self
    }

    pub fn point_radius_factor(mut self, f: f32) -> Self {
        self.point_radius_factor = f;
        self
    }
}

impl<S, M, TT> Plot<S> for LinePlot<M, TT>
where
    S: Series,
    M: Fn(&S::Y) -> f32 + Copy,
    TT: Fn(&S::Y, Option<&S::Y>) -> PlotTooltip + Copy,
{
    fn y_extents(&self, s: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for (_, y) in s.range_iter(range) {
            let v = (self.map_y)(y);
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
        if min == f32::MAX {
            None
        } else {
            Some((min, max))
        }
    }

    fn adjust_extents(&self, min: f32, max: f32) -> (f32, f32) {
        if self.padding > 0.0 && max > min {
            let range = max - min;
            let pad = range * self.padding;
            (min - pad, max + pad)
        } else {
            (min, max)
        }
    }

    fn draw<'a>(
        &self,
        frame: &mut canvas::Frame,
        ctx: &'a ViewState,
        theme: &Theme,
        s: &S,
        range: RangeInclusive<u64>,
        scale: &YScale,
    ) {
        let palette = theme.extended_palette();
        let color = palette.secondary.strong.color;

        let stroke = Stroke::with_color(
            Stroke {
                width: self.stroke_width,
                ..Stroke::default()
            },
            color,
        );

        // Polyline
        let mut prev: Option<(f32, f32)> = None;
        for (x, y) in s.range_iter(range.clone()) {
            let sx = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);
            let vy = (self.map_y)(y);
            let sy = scale.to_y(vy);
            if let Some((px, py)) = prev {
                frame.stroke(
                    &Path::line(iced::Point::new(px, py), iced::Point::new(sx, sy)),
                    stroke,
                );
            }
            prev = Some((sx, sy));
        }

        if self.show_points {
            let radius = (ctx.cell_width * self.point_radius_factor).min(5.0);
            for (x, y) in s.range_iter(range) {
                let sx = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);
                let sy = scale.to_y((self.map_y)(y));
                frame.fill(&Path::circle(iced::Point::new(sx, sy), radius), color);
            }
        }
    }

    fn tooltip(&self, y: &S::Y, next: Option<&S::Y>, _theme: &iced::Theme) -> PlotTooltip {
        (self.tooltip)(y, next)
    }
}

pub struct PlotTooltip {
    pub text: String,
}

impl PlotTooltip {
    const TOOLTIP_CHAR_W: f32 = 8.0;
    const TOOLTIP_LINE_H: f32 = 14.0;
    const TOOLTIP_PAD_X: f32 = 8.0; // left+right padding total
    const TOOLTIP_PAD_Y: f32 = 6.0; // top+bottom padding total

    pub fn new<T: Into<String>>(text: T) -> Self {
        Self { text: text.into() }
    }

    pub fn guesstimate(&self) -> (f32, f32) {
        let mut max_cols: usize = 0;
        let mut lines: usize = 0;

        for line in self.text.split('\n') {
            lines += 1;
            let cols = line.chars().count();
            if cols > max_cols {
                max_cols = cols;
            }
        }

        let width = (max_cols as f32) * Self::TOOLTIP_CHAR_W + Self::TOOLTIP_PAD_X;
        let height = (lines.max(1) as f32) * Self::TOOLTIP_LINE_H + Self::TOOLTIP_PAD_Y;
        (width, height)
    }
}
