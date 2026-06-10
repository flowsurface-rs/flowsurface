use std::ops::RangeInclusive;

use iced::{
    Theme,
    widget::canvas::{self, Path, Stroke},
};

use crate::chart::{
    ViewState,
    indicator::plot::{Plot, PlotTooltip, Series, TooltipFn, YScale},
};

const DEFAULT_BAR_WIDTH_FACTOR: f32 = 0.9;

/// Predicate that decides whether a datapoint is valid for drawing.
type ValidFn<T> = Box<dyn Fn(&T) -> bool>;

pub struct LinePlot<V, T> {
    pub value: V,
    pub tooltip: Option<TooltipFn<T>>,
    // padding in percentage of the value range, applies both top and bottom
    pub padding: f32,
    pub stroke_width: f32,
    pub show_points: bool,
    pub point_radius_factor: f32,
    /// Horizontal shift in bucket units (screen-space).
    /// Positive values move points right, negative values move left.
    pub x_shift_buckets: i32,
    /// Optional predicate to determine whether a point is "valid".
    /// Invalid points break the line (they are not connected to
    /// neighbours) and show `invalid_point_message` in the tooltip.
    /// When `None`, all points are treated as valid.
    pub is_valid: Option<ValidFn<T>>,
    /// Message shown in the tooltip area when the user hovers over (or the
    /// always-visible label lands on) an invalid point.  When `None` (the
    /// default) the tooltip is suppressed entirely.
    pub invalid_point_message: Option<String>,
    _phantom: std::marker::PhantomData<T>,
}

impl<V, T> LinePlot<V, T> {
    /// Create a new LinePlot with the given mapping function for Y values and tooltip function.
    pub fn new(value: V) -> Self {
        Self {
            value,
            tooltip: None,
            padding: 0.08,
            stroke_width: 1.0,
            show_points: true,
            point_radius_factor: 0.2,
            x_shift_buckets: 0,
            is_valid: None,
            invalid_point_message: None,
            _phantom: std::marker::PhantomData,
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

    /// whether to draw a circle on each datapoint
    /// usually visible only when zoomed in
    pub fn show_points(mut self, on: bool) -> Self {
        self.show_points = on;
        self
    }

    /// circle radius drawn on each datapoint
    /// as a factor of cell width, e.g. 0.2 means 20% of cell width, capped at 5px
    pub fn point_radius_factor(mut self, f: f32) -> Self {
        self.point_radius_factor = f;
        self
    }

    /// Shift datapoint x-position by whole bucket units in screen-space.
    ///
    /// e.g. `shift(1)` moves each point one bucket to the right.
    pub fn shift(mut self, buckets: i32) -> Self {
        self.x_shift_buckets = buckets;
        self
    }

    /// Set a predicate that decides whether a datapoint is "valid".
    /// Invalid points break the line (no connection to neighbours)
    /// and show `invalid_point_message` if one is configured.
    /// When `None` (the default), every point is considered valid.
    pub fn valid_when<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&T) -> bool + 'static,
    {
        self.is_valid = Some(Box::new(predicate));
        self
    }

    /// Set a message to show in the tooltip area when a point is invalid
    /// (according to [`valid_when`]).  When not set the tooltip is hidden
    /// entirely for invalid points.
    ///
    /// [`valid_when`]: Self::valid_when
    pub fn invalid_point_message(mut self, msg: impl Into<String>) -> Self {
        self.invalid_point_message = Some(msg.into());
        self
    }

    pub fn with_tooltip<F>(mut self, tooltip: F) -> Self
    where
        F: Fn(&T, Option<&T>) -> PlotTooltip + 'static,
    {
        self.tooltip = Some(Box::new(tooltip));
        self
    }
}

impl<S, V> Plot<S> for LinePlot<V, S::Y>
where
    S: Series,
    V: Fn(&S::Y) -> f32,
{
    fn y_extents(&self, datapoints: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        let mut min_v = f32::MAX;
        let mut max_v = f32::MIN;

        datapoints.for_each_in(range, |_, y| {
            if self.is_valid.as_ref().is_none_or(|f| f(y)) {
                let v = (self.value)(y);
                if v < min_v {
                    min_v = v;
                }
                if v > max_v {
                    max_v = v;
                }
            }
        });

        if min_v == f32::MAX {
            None
        } else {
            Some((min_v, max_v))
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

    fn draw(
        &self,
        frame: &mut canvas::Frame,
        ctx: &ViewState,
        theme: &Theme,
        datapoints: &S,
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

        let half_bar_width = (ctx.cell_width * DEFAULT_BAR_WIDTH_FACTOR) / 2.0;
        let shift_px = (self.x_shift_buckets as f32) * ctx.cell_width;

        // Line points are anchored to the right edge of the default bar width.
        let x_for = |x: u64| -> f32 { ctx.interval_to_x(x) + half_bar_width + shift_px };

        let radius = self
            .show_points
            .then(|| (ctx.cell_width * self.point_radius_factor).min(5.0));

        // Draw line segments, skipping over invalid points.
        // When a point is invalid, `prev` is reset so the next valid
        // point starts a new disconnected segment.
        let mut prev: Option<(f32, f32)> = None;
        datapoints.for_each_in(range.clone(), |x, y| {
            let valid = self.is_valid.as_ref().is_none_or(|f| f(y));
            if valid {
                let sx = x_for(x);
                let sy = scale.to_y((self.value)(y));
                if let Some((px, py)) = prev {
                    frame.stroke(
                        &Path::line(iced::Point::new(px, py), iced::Point::new(sx, sy)),
                        stroke,
                    );
                }
                if let Some(r) = radius {
                    frame.fill(&Path::circle(iced::Point::new(sx, sy), r), color);
                }
                prev = Some((sx, sy));
            } else {
                prev = None;
            }
        });
    }

    fn tooltip_fn(&self) -> Option<&TooltipFn<S::Y>> {
        self.tooltip.as_ref()
    }

    fn is_point_valid(&self, y: &S::Y) -> bool {
        self.is_valid.as_ref().is_none_or(|f| f(y))
    }

    fn invalid_point_message(&self) -> Option<&str> {
        self.invalid_point_message.as_deref()
    }
}
