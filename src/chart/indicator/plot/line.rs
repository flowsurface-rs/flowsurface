use std::ops::RangeInclusive;

use iced::{
    Theme,
    widget::canvas::{self, Path, Stroke},
};

use crate::chart::{
    ViewState,
    indicator::plot::{Plot, PlotTooltip, Series, TooltipFn, YScale},
};

pub struct LinePlot<V, T> {
    pub value: V,
    pub tooltip: Option<TooltipFn<T>>,
    // padding in percentage of the value range, applies both top and bottom
    pub padding: f32,
    pub stroke_width: f32,
    pub show_points: bool,
    pub point_radius_factor: f32,
    _phantom: std::marker::PhantomData<T>,
}

#[allow(dead_code)]
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
            let v = (self.value)(y);
            if v < min_v {
                min_v = v;
            }
            if v > max_v {
                max_v = v;
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

        // Polyline
        let mut prev: Option<(f32, f32)> = None;
        datapoints.for_each_in(range.clone(), |x, y| {
            let sx = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);
            let vy = (self.value)(y);
            let sy = scale.to_y(vy);
            if let Some((px, py)) = prev {
                frame.stroke(
                    &Path::line(iced::Point::new(px, py), iced::Point::new(sx, sy)),
                    stroke,
                );
            }
            prev = Some((sx, sy));
        });

        if self.show_points {
            let radius = (ctx.cell_width * self.point_radius_factor).min(5.0);
            datapoints.for_each_in(range, |x, y| {
                let sx = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);
                let sy = scale.to_y((self.value)(y));
                frame.fill(&Path::circle(iced::Point::new(sx, sy), radius), color);
            });
        }
    }

    fn tooltip_fn(&self) -> Option<&TooltipFn<S::Y>> {
        self.tooltip.as_ref()
    }
}
