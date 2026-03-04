// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
//! Composite Plot that renders bars (BarPlot-style) with N colored EMA polylines overlaid.
//! Reusable for any bar indicator that needs smoothed line overlays.

use std::ops::RangeInclusive;

use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Point, Size, Theme};

use super::bar::{BarClass, Baseline};
use crate::chart::ViewState;
use crate::chart::indicator::plot::{Plot, PlotTooltip, Series, TooltipFn, YScale};

/// Configuration for a single EMA overlay line.
/// Uses function pointers for the extract fn so multiple configs
/// can coexist in a Vec (closures have unique types).
pub struct EmaLineConfig<T> {
    /// Extract the precomputed EMA value from the data point.
    pub extract: fn(&T) -> f32,
    /// Derive line color from the theme palette.
    pub color: fn(&Extended) -> Color,
    pub stroke_width: f32,
}

/// Composite plot: bars (like BarPlot) with N EMA polylines overlaid.
pub struct BarWithEmaOverlay<V, CL, T> {
    pub value: V,
    pub bar_width_factor: f32,
    pub classify: CL,
    pub baseline: Baseline,
    pub tooltip: Option<TooltipFn<T>>,
    pub ema_lines: Vec<EmaLineConfig<T>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<V, CL, T> BarWithEmaOverlay<V, CL, T> {
    pub fn new(value: V, classify: CL, ema_lines: Vec<EmaLineConfig<T>>) -> Self {
        Self {
            value,
            bar_width_factor: 0.9,
            classify,
            baseline: Baseline::Zero,
            tooltip: None,
            ema_lines,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_tooltip<F>(mut self, tooltip: F) -> Self
    where
        F: Fn(&T, Option<&T>) -> PlotTooltip + 'static,
    {
        self.tooltip = Some(Box::new(tooltip));
        self
    }
}

impl<S, V, CL> Plot<S> for BarWithEmaOverlay<V, CL, S::Y>
where
    S: Series,
    V: Fn(&S::Y) -> f32,
    CL: Fn(&S::Y) -> BarClass,
{
    fn y_extents(&self, datapoints: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        let mut min_v = f32::MAX;
        let mut max_v = f32::MIN;
        let mut n = 0u32;

        datapoints.for_each_in(range, |_, y| {
            // Bar value
            let v = (self.value)(y);
            if v < min_v {
                min_v = v;
            }
            if v > max_v {
                max_v = v;
            }
            // EMA values
            for ema in &self.ema_lines {
                let ev = (ema.extract)(y);
                if ev < min_v {
                    min_v = ev;
                }
                if ev > max_v {
                    max_v = ev;
                }
            }
            n += 1;
        });

        if n == 0 {
            return None;
        }

        let min_ext = match self.baseline {
            Baseline::Zero => min_v.min(0.0),
            Baseline::Min => min_v,
            Baseline::Fixed(v) => v,
        };

        let max_ext = match self.baseline {
            Baseline::Zero => max_v.max(0.0),
            _ => max_v,
        };

        if min_ext >= max_ext && max_ext <= 0.0 && !matches!(self.baseline, Baseline::Zero) {
            return None;
        }

        let lowest = min_ext;
        let highest = max_ext.max(lowest + f32::EPSILON);
        Some((lowest, highest))
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
        let bar_width = ctx.cell_width * self.bar_width_factor;

        let baseline_value = match self.baseline {
            Baseline::Zero => 0.0,
            Baseline::Min => scale.min,
            Baseline::Fixed(v) => v,
        };
        let y_base = scale.to_y(baseline_value);

        // Layer 1: Draw bars (same logic as BarPlot)
        datapoints.for_each_in(range.clone(), |x, y| {
            let center_x = ctx.interval_to_x(x);
            let left = center_x - (bar_width / 2.0);

            let total = (self.value)(y);
            let rel = total - baseline_value;

            let (top_y, h_total) = if rel > 0.0 {
                let y_total = scale.to_y(total);
                let h = (y_base - y_total).max(0.0);
                (y_total, h)
            } else if rel < 0.0 {
                let y_total = scale.to_y(total);
                let h = (y_total - y_base).max(0.0);
                (y_base, h)
            } else {
                (y_base, 0.0)
            };
            if h_total <= 0.0 {
                return;
            }

            match (self.classify)(y) {
                BarClass::Single => {
                    frame.fill_rectangle(
                        Point::new(left, top_y),
                        Size::new(bar_width, h_total),
                        palette.secondary.strong.color,
                    );
                }
                BarClass::Signed => {
                    let color = if rel >= 0.0 {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };
                    frame.fill_rectangle(
                        Point::new(left, top_y),
                        Size::new(bar_width, h_total),
                        color,
                    );
                }
                BarClass::CandleColored { bullish } => {
                    let color = if bullish {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };
                    frame.fill_rectangle(
                        Point::new(left, top_y),
                        Size::new(bar_width, h_total),
                        color,
                    );
                }
                BarClass::BuySell { buy, sell } => {
                    let buy_color = palette.success.base.color;
                    let sell_color = palette.danger.base.color;
                    if sell > 0.0 {
                        let y_sell_top = scale.to_y(sell);
                        let h_sell = (y_base - y_sell_top).max(0.0);
                        if h_sell > 0.0 {
                            frame.fill_rectangle(
                                Point::new(left, y_sell_top),
                                Size::new(bar_width, h_sell),
                                sell_color,
                            );
                        }
                    }
                    if buy > 0.0 {
                        let y_buy_top = scale.to_y(sell + buy);
                        let y_buy_bottom = scale.to_y(sell);
                        let h_buy = (y_buy_bottom - y_buy_top).max(0.0);
                        if h_buy > 0.0 {
                            frame.fill_rectangle(
                                Point::new(left, y_buy_top),
                                Size::new(bar_width, h_buy),
                                buy_color,
                            );
                        }
                    }
                }
            }
        });

        // Layer 2: Draw EMA polylines on top of bars
        for ema in &self.ema_lines {
            let color = (ema.color)(palette);
            let stroke = Stroke::with_color(
                Stroke {
                    width: ema.stroke_width,
                    ..Stroke::default()
                },
                color,
            );

            let mut prev: Option<(f32, f32)> = None;
            datapoints.for_each_in(range.clone(), |x, y| {
                let sx = ctx.interval_to_x(x);
                let vy = (ema.extract)(y);
                let sy = scale.to_y(vy);
                if let Some((px, py)) = prev {
                    frame.stroke(&Path::line(Point::new(px, py), Point::new(sx, sy)), stroke);
                }
                prev = Some((sx, sy));
            });
        }
    }

    fn tooltip_fn(&self) -> Option<&TooltipFn<S::Y>> {
        self.tooltip.as_ref()
    }
}
