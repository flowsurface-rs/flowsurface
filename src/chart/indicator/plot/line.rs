use std::ops::RangeInclusive;

use iced::{
    Theme,
    widget::canvas::{self, Path, Stroke},
};

use crate::chart::{
    ViewState,
    indicator::plot::{Plot, PlotTooltip, Series, YScale},
};

pub struct LinePlot<V, TT> {
    pub value: V,
    pub tooltip: TT,
    pub padding: f32,
    pub stroke_width: f32,
    pub show_points: bool,
    pub point_radius_factor: f32,
}

#[allow(dead_code)]
impl<V, TT> LinePlot<V, TT> {
    /// Create a new LinePlot with the given mapping function for Y values and tooltip function.
    pub fn new(value: V, tooltip: TT) -> Self {
        Self {
            value,
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

impl<S, V, TT> Plot<S> for LinePlot<V, TT>
where
    S: Series,
    V: Fn(&S::Y) -> f32,
    TT: Fn(&S::Y, Option<&S::Y>) -> PlotTooltip,
{
    fn y_extents(&self, s: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        let mut min_v = f32::MAX;
        let mut max_v = f32::MIN;

        s.for_each_in(range, |_, y| {
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
        s.for_each_in(range.clone(), |x, y| {
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
            s.for_each_in(range, |x, y| {
                let sx = ctx.interval_to_x(x) - (ctx.cell_width / 2.0);
                let sy = scale.to_y((self.value)(y));
                frame.fill(&Path::circle(iced::Point::new(sx, sy), radius), color);
            });
        }
    }

    fn tooltip(&self, y: &S::Y, next: Option<&S::Y>, _theme: &iced::Theme) -> PlotTooltip {
        (self.tooltip)(y, next)
    }
}
