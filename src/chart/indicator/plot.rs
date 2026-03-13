// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
use crate::chart::{Basis, Interaction, Message, ViewState};
use crate::style::{self, dashed_line};
use data::util::{guesstimate_ticks, round_to_tick};

use iced::widget::canvas::{self, Cache, Geometry, Path};
use iced::{Alignment, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

pub mod bar;
pub mod bar_with_ema_overlay;
pub mod line;

pub trait Series {
    type Y;

    fn for_each_in<F: FnMut(u64, &Self::Y)>(&self, range: RangeInclusive<u64>, f: F);

    fn at(&self, x: u64) -> Option<&Self::Y>;

    fn next_after<'a>(&'a self, x: u64) -> Option<(u64, &'a Self::Y)>
    where
        Self: 'a;
}

impl<Y> Series for &BTreeMap<u64, Y> {
    type Y = Y;

    fn for_each_in<F: FnMut(u64, &Self::Y)>(&self, range: RangeInclusive<u64>, mut f: F) {
        for (k, v) in (**self).range(range) {
            f(*k, v);
        }
    }

    fn at(&self, x: u64) -> Option<&Self::Y> {
        (**self).get(&x)
    }

    fn next_after<'a>(&'a self, x: u64) -> Option<(u64, &'a Self::Y)>
    where
        Self: 'a,
    {
        (**self).range((x + 1)..).next().map(|(k, v)| (*k, v))
    }
}

pub struct ReversedBTreeSeries<'a, Y> {
    inner: &'a BTreeMap<u64, Y>,
    offset: u64, // largest key in inner
}

impl<'a, Y> ReversedBTreeSeries<'a, Y> {
    pub fn new(inner: &'a BTreeMap<u64, Y>) -> Self {
        let offset = inner.last_key_value().map(|(k, _)| *k).unwrap_or(0);
        Self { inner, offset }
    }
}

impl<'m, Y> Series for ReversedBTreeSeries<'m, Y> {
    type Y = Y;

    fn for_each_in<F: FnMut(u64, &Self::Y)>(&self, range: RangeInclusive<u64>, mut f: F) {
        let earliest = self.offset.saturating_sub(*range.end());
        let latest = self.offset.saturating_sub(*range.start());

        for (k, v) in self.inner.range(earliest..=latest).rev() {
            f(self.offset - *k, v);
        }
    }

    fn at(&self, x: u64) -> Option<&Self::Y> {
        let k = self.offset.checked_sub(x)?;
        self.inner.get(&k)
    }

    fn next_after<'a>(&'a self, x: u64) -> Option<(u64, &'a Self::Y)>
    where
        Self: 'a,
    {
        let k = self.offset.checked_sub(x)?;
        self.inner
            .range(..k)
            .next_back()
            .map(|(kk, v)| (self.offset - *kk, v))
    }
}

// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
/// Series backed by a contiguous `&[Y]` slice where index 0 = oldest bar (forward order).
/// Used for Odb/Tick indicators that store data in `Vec<Y>` for O(1) rebuild.
/// Translates the `.rev()` index space used by Odb rendering into slice indices.
pub struct ReversedSliceSeries<'a, Y> {
    inner: &'a [Y],
    /// Largest `.rev()` index = index of the OLDEST bar = slice.len() - 1.
    offset: u64,
}

impl<'a, Y> ReversedSliceSeries<'a, Y> {
    pub fn new(inner: &'a [Y]) -> Self {
        let offset = inner.len().saturating_sub(1) as u64;
        Self { inner, offset }
    }
}

impl<'a, Y> Series for ReversedSliceSeries<'a, Y> {
    type Y = Y;

    fn for_each_in<F: FnMut(u64, &Self::Y)>(&self, range: RangeInclusive<u64>, mut f: F) {
        if self.inner.is_empty() {
            return;
        }
        // Convert .rev() range to forward slice range.
        // range.start() = earliest_visual (newer, smaller .rev() idx)
        // range.end()   = latest_visual   (older,  larger  .rev() idx)
        let earliest_fwd = self.offset.saturating_sub(*range.end()) as usize;
        let latest_fwd =
            (self.offset.saturating_sub(*range.start()) as usize).min(self.inner.len() - 1);
        if earliest_fwd > latest_fwd {
            return;
        }
        // Iterate newest → oldest (decreasing forward index = increasing .rev() index).
        for fwd_idx in (earliest_fwd..=latest_fwd).rev() {
            let rev_idx = self.offset - fwd_idx as u64;
            f(rev_idx, &self.inner[fwd_idx]);
        }
    }

    fn at(&self, x: u64) -> Option<&Self::Y> {
        let fwd = self.offset.checked_sub(x)? as usize;
        self.inner.get(fwd)
    }

    fn next_after<'b>(&'b self, x: u64) -> Option<(u64, &'b Self::Y)>
    where
        Self: 'b,
    {
        // Next bar in .rev() order = older bar = lower forward index.
        let fwd = self.offset.checked_sub(x)? as usize;
        if fwd == 0 {
            return None;
        }
        self.inner.get(fwd - 1).map(|v| (x + 1, v))
    }
}

pub enum AnySeries<'a, Y> {
    Forward(&'a BTreeMap<u64, Y>),
    Reversed(ReversedBTreeSeries<'a, Y>),
    /// Slice-backed series for Vec-based indicators (Odb/Tick only).
    ReversedSlice(ReversedSliceSeries<'a, Y>),
}

impl<'a, Y> AnySeries<'a, Y> {
    pub fn for_basis(basis: Basis, data: &'a BTreeMap<u64, Y>) -> Self {
        match basis {
            Basis::Tick(_) | Basis::Odb(_) => Self::Reversed(ReversedBTreeSeries::new(data)),
            Basis::Time(_) => Self::Forward(data),
        }
    }

    /// Create a slice-backed series for Vec-based indicators.
    /// `data` is forward-indexed (0 = oldest bar). Only valid for Odb/Tick basis.
    pub fn for_basis_slice(data: &'a [Y]) -> Self {
        Self::ReversedSlice(ReversedSliceSeries::new(data))
    }
}

impl<'a, Y> Series for AnySeries<'a, Y> {
    type Y = Y;

    fn for_each_in<F: FnMut(u64, &Self::Y)>(&self, range: RangeInclusive<u64>, mut f: F) {
        match self {
            AnySeries::Forward(map) => {
                for (k, v) in (**map).range(range) {
                    f(*k, v);
                }
            }
            AnySeries::Reversed(rv) => rv.for_each_in(range, f),
            AnySeries::ReversedSlice(rv) => rv.for_each_in(range, f),
        }
    }

    fn at(&self, x: u64) -> Option<&Self::Y> {
        match self {
            AnySeries::Forward(map) => (**map).get(&x),
            AnySeries::Reversed(rv) => rv.at(x),
            AnySeries::ReversedSlice(rv) => rv.at(x),
        }
    }

    fn next_after<'b>(&'b self, x: u64) -> Option<(u64, &'b Self::Y)>
    where
        Self: 'b,
    {
        match self {
            AnySeries::Forward(map) => (**map).range((x + 1)..).next().map(|(k, v)| (*k, v)),
            AnySeries::Reversed(rv) => rv.next_after(x),
            AnySeries::ReversedSlice(rv) => rv.next_after(x),
        }
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

    fn tooltip_fn(&self) -> Option<&TooltipFn<S::Y>>;

    fn tooltip(&self, y: &S::Y, next: Option<&S::Y>, _theme: &Theme) -> Option<PlotTooltip> {
        self.tooltip_fn().map(|tt| tt(y, next))
    }

    /// Draw a persistent screen-space overlay on top of the indicator panel.
    /// Called in a separate, untransformed frame (screen coordinates, no scaling/translation).
    /// Default: no-op. Override to draw legends, watermarks, or other fixed-position overlays.
    fn draw_panel_legend(&self, _frame: &mut canvas::Frame) {}
}

pub struct ChartCanvas<'a, P, S>
where
    P: Plot<S>,
    S: Series,
{
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    /// If `Some`, a third untransformed geometry layer is rendered by calling
    /// `plot.draw_panel_legend()`. Reuses this cache to avoid redrawing every frame.
    pub legend_cache: Option<&'a Cache>,
    pub ctx: &'a ViewState,
    pub plot: P,
    pub series: S,
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

        let indicator = self.indicator_cache.draw(renderer, bounds.size(), |frame| {
            let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);

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
                .draw(frame, ctx, theme, &self.series, earliest..=latest, &scale);
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
                let (rounded_x, snap_ratio) = match ctx.basis {
                    Basis::Time(tf) => {
                        let step = tf.to_milliseconds() as f64;
                        let rx = ((earliest + crosshair_ratio * (latest - earliest)) / step).round()
                            as u64
                            * step as u64;

                        let sr = if latest <= earliest {
                            0.5
                        } else {
                            ((rx as f64 - earliest) / (latest - earliest)) as f32
                        };
                        (rx, sr)
                    }
                    Basis::Tick(_) | Basis::Odb(_) => {
                        let world_x = region.x + (cursor_position.x / bounds.width) * region.width;
                        let snapped_world_x = (world_x / ctx.cell_width).round() * ctx.cell_width;

                        let sr = if region.width.abs() > f32::EPSILON {
                            (snapped_world_x - region.x) / region.width
                        } else {
                            0.5
                        };
                        let rx = ctx.x_to_interval(snapped_world_x);
                        (rx, sr)
                    }
                };

                if snap_ratio.is_finite() {
                    frame.stroke(
                        &Path::line(
                            Point::new(snap_ratio * bounds.width, 0.0),
                            Point::new(snap_ratio * bounds.width, bounds.height),
                        ),
                        dashed,
                    );
                }

                // tooltip text
                if let Some(y) = self.series.at(rounded_x) {
                    let next = self.series.next_after(rounded_x).map(|(_, v)| v);

                    if let Some(tooltip) = self.plot.tooltip(y, next, theme) {
                        tooltip.draw(frame, theme, bounds, cursor_position.x);
                    }
                }
            } else if let Some(cursor_position) = cursor.position_in(bounds) {
                // horizontal snap uses label extents
                let highest = self.max_for_labels;
                let lowest = self.min_for_labels;
                let label_range = lowest - highest;
                let tick = guesstimate_ticks(label_range);

                let ratio = cursor_position.y / bounds.height;
                let value = highest + ratio * label_range;
                let rounded = round_to_tick(value, tick);
                let snap_ratio = if label_range.abs() > f32::EPSILON {
                    (rounded - highest) / label_range
                } else {
                    0.5
                };

                if snap_ratio.is_finite() {
                    frame.stroke(
                        &Path::line(
                            Point::new(0.0, snap_ratio * bounds.height),
                            Point::new(bounds.width, snap_ratio * bounds.height),
                        ),
                        dashed,
                    );
                }
            }
        });

        let mut geoms = vec![indicator, crosshair];
        if let Some(lc) = self.legend_cache {
            let legend = lc.draw(renderer, bounds.size(), |frame| {
                self.plot.draw_panel_legend(frame);
            });
            geoms.push(legend);
        }
        geoms
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

pub type TooltipFn<T> = Box<dyn Fn(&T, Option<&T>) -> PlotTooltip>;

const TOOLTIP_MARGIN: f32 = 4.0; // px from edge of canvas
const TOOLTIP_PADDING: f32 = 8.0; // px inside tooltip box

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

    pub fn draw(&self, frame: &mut canvas::Frame, theme: &Theme, bounds: Rectangle, cursor_x: f32) {
        let (tooltip_w, tooltip_h) = self.guesstimate();
        let palette = theme.extended_palette();

        // decide side to avoid covering hovered datapoint and fit in bounds
        let switch_sides = {
            let right_half = cursor_x < bounds.width / 2.0;

            if 3.0 * tooltip_h > bounds.height {
                right_half
            } else if right_half {
                cursor_x + TOOLTIP_MARGIN + tooltip_w > bounds.width
            } else {
                cursor_x < TOOLTIP_MARGIN + tooltip_w
            }
        };

        let (rect_x, text_x, align_x) = if switch_sides {
            let rx = bounds.width - tooltip_w - TOOLTIP_MARGIN;
            let tx = rx + tooltip_w - TOOLTIP_PADDING;
            (rx, tx, Alignment::End)
        } else {
            let rx = TOOLTIP_MARGIN;
            let tx = rx + TOOLTIP_PADDING;
            (rx, tx, Alignment::Start)
        };

        frame.fill_rectangle(
            Point::new(rect_x, 0.0),
            Size::new(tooltip_w, tooltip_h),
            palette.background.weakest.color.scale_alpha(0.9),
        );
        frame.fill_text(canvas::Text {
            content: self.text.clone(),
            position: Point::new(text_x, 2.0),
            size: iced::Pixels(10.0),
            color: palette.background.base.text,
            font: style::AZERET_MONO,
            align_x: align_x.into(),
            ..canvas::Text::default()
        });
    }
}
