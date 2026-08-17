pub mod comparison;
pub mod heatmap;
pub mod kline;

use exchange::TickerInfo;
use iced::Rectangle;

#[derive(Debug, Clone, Copy)]
pub struct Regions {
    pub plot: Rectangle,
    pub x_axis: Rectangle,
    pub y_axis: Rectangle,
}

impl Regions {
    pub fn from_layout(root: iced_core::Layout<'_>) -> Self {
        let root_bounds = root.bounds();

        let row = root.child(0);
        let x_abs = root.child(1).bounds();

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

    pub fn is_in_plot(&self, p: iced_core::Point) -> bool {
        p.x >= self.plot.x
            && p.x <= self.plot.x + self.plot.width
            && p.y >= self.plot.y
            && p.y <= self.plot.y + self.plot.height
    }

    pub fn is_in_x_axis(&self, p: iced_core::Point) -> bool {
        p.x >= self.x_axis.x
            && p.x <= self.x_axis.x + self.x_axis.width
            && p.y >= self.x_axis.y
            && p.y <= self.x_axis.y + self.x_axis.height
    }

    pub fn is_in_y_axis(&self, p: iced_core::Point) -> bool {
        p.x >= self.y_axis.x
            && p.x <= self.y_axis.x + self.y_axis.width
            && p.y >= self.y_axis.y
            && p.y <= self.y_axis.y + self.y_axis.height
    }
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

#[derive(Debug, Clone)]
pub struct Series {
    pub ticker_info: TickerInfo,
    pub name: Option<String>,
    pub points: Vec<(u64, f32)>,
    pub color: iced::Color,
}

impl Series {
    pub fn new(ticker_info: TickerInfo, color: iced::Color, name: Option<String>) -> Self {
        Self {
            ticker_info,
            name,
            points: Vec::new(),
            color,
        }
    }
}

pub trait SeriesLike {
    fn name(&self) -> String;
    fn points(&self) -> &[(u64, f32)];
    fn color(&self) -> iced::Color;
    fn ticker_info(&self) -> &TickerInfo;
}

impl SeriesLike for Series {
    fn name(&self) -> String {
        if let Some(name) = &self.name {
            name.clone()
        } else {
            self.ticker_info.ticker.to_string()
        }
    }

    fn points(&self) -> &[(u64, f32)] {
        &self.points
    }

    fn color(&self) -> iced::Color {
        self.color
    }

    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }
}

fn format_pct(val: f32, step: f32, show_decimals: bool) -> String {
    let precision = percent_precision(step, show_decimals);
    format!("{val:+.*}%", precision)
}

fn percent_precision(step: f32, show_decimals: bool) -> usize {
    if show_decimals || step.fract() != 0.0 {
        if step >= 1.0 {
            1
        } else if step >= 0.1 {
            2
        } else {
            3
        }
    } else if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    }
}

fn format_value(value: f32, step: f32) -> String {
    if step >= 10.0 {
        format!("{value:.0}")
    } else if step >= 1.0 {
        format!("{value:.2}")
    } else if step >= 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

fn format_time_label(ts_ms: u64, step_ms: u64, tz: data::UserTimezone) -> String {
    tz.format_with_kind(
        ts_ms as i64,
        data::config::timezone::TimeLabelKind::AxisStepMs { step_ms },
    )
    .unwrap_or_else(|| ts_ms.to_string())
}

fn div_ceil(value: i64, divisor: i64) -> i64 {
    let quotient = value.div_euclid(divisor);
    let remainder = value.rem_euclid(divisor);
    if remainder == 0 {
        quotient
    } else {
        quotient.saturating_add(1)
    }
}

fn nice_step_units(rough: i64) -> i64 {
    let rough = rough.max(1);
    let mut magnitude = 1i64;

    while magnitude <= rough / 10 {
        magnitude = magnitude.saturating_mul(10);
    }

    let fraction = rough as f64 / magnitude as f64;
    let multiplier = if fraction <= 1.0 {
        1
    } else if fraction <= 2.0 {
        2
    } else if fraction <= 5.0 {
        5
    } else {
        10
    };

    multiplier * magnitude
}

fn unit_ticks(min_x: i64, max_x: i64, width_px: f32, min_tick_px: f32) -> (Vec<i64>, i64) {
    let span = (max_x - min_x).max(1);
    let target_ticks = (width_px / min_tick_px.max(1.0)).floor().max(2.0);
    let rough_step = ((span as f32) / target_ticks).ceil().max(1.0) as i64;
    let step = nice_step_units(rough_step);

    let first = div_ceil(min_x, step).saturating_mul(step);
    let mut ticks = Vec::new();
    let mut current = first;
    for _ in 0..=4096 {
        if current > max_x {
            break;
        }
        ticks.push(current);
        current = current.saturating_add(step);
    }

    (ticks, step)
}

pub mod domain {
    pub fn align_floor(ts: u64, dt: u64) -> u64 {
        if dt == 0 {
            return ts;
        }
        (ts / dt) * dt
    }

    pub fn align_ceil(ts: u64, dt: u64) -> u64 {
        if dt == 0 {
            return ts;
        }
        let f = (ts / dt) * dt;
        if f == ts { ts } else { f.saturating_add(dt) }
    }

    pub fn interpolate_y_at(points: &[(u64, f32)], x: u64) -> Option<f32> {
        if points.is_empty() {
            return None;
        }
        let idx_right = points.iter().position(|(px, _)| *px >= x)?;
        Some(match idx_right {
            0 => points[0].1,
            i => {
                let (x0, y0) = points[i - 1];
                let (x1, y1) = points[i];
                let dx = x1.saturating_sub(x0) as f32;
                if dx > 0.0 {
                    let t = (x.saturating_sub(x0)) as f32 / dx;
                    y0 + (y1 - y0) * t.clamp(0.0, 1.0)
                } else {
                    y0
                }
            }
        })
    }

    pub fn window(
        series: &[&[(u64, f32)]],
        zoom: super::Zoom,
        pan_points: f32,
        dt: u64,
    ) -> Option<(u64, u64)> {
        if series.is_empty() {
            return None;
        }

        let mut any = false;
        let mut data_min_x = u64::MAX;
        let mut data_max_x = u64::MIN;
        for pts in series {
            for (x, _) in *pts {
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

        let add_signed = |v: u64, d: i64| -> u64 {
            if d >= 0 {
                v.saturating_add(d as u64)
            } else {
                v.saturating_sub((-d) as u64)
            }
        };

        let span = if zoom.is_all() {
            data_max_x.saturating_sub(data_min_x).max(1)
        } else {
            let n = zoom.0;
            let mut s = ((n.saturating_sub(1)) as u64).saturating_mul(dt);
            if s == 0 {
                s = 1;
            }
            s
        };

        let pad_ms = (pan_points * dt as f32).round() as i64;
        let mut right = add_signed(data_max_x, pad_ms);
        let right_cap = data_max_x.saturating_add(span);
        if right > right_cap {
            right = right_cap;
        }
        let left = right.saturating_sub(span);

        let left = align_floor(left, dt);
        let right = align_ceil(right, dt);

        Some((left, right))
    }

    pub fn pct_domain(series: &[&[(u64, f32)]], min_x: u64, max_x: u64) -> Option<(f32, f32)> {
        let mut min_pct = f32::INFINITY;
        let mut max_pct = f32::NEG_INFINITY;
        let mut any = false;

        for pts in series {
            if pts.is_empty() {
                continue;
            }

            let y0 = interpolate_y_at(pts, min_x).unwrap_or(0.0);
            if y0 == 0.0 {
                continue;
            }

            let mut has_visible = false;
            for (_x, y) in pts.iter().filter(|(x, _)| *x >= min_x && *x <= max_x) {
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
                any = true;
                if 0.0 < min_pct {
                    min_pct = 0.0;
                }
                if 0.0 > max_pct {
                    max_pct = 0.0;
                }
            }
        }

        if !any {
            return None;
        }

        if (max_pct - min_pct).abs() < f32::EPSILON {
            min_pct -= 1.0;
            max_pct += 1.0;
        }

        let span = (max_pct - min_pct).max(1e-6);
        let pad = span * 0.05;
        Some((min_pct - pad, max_pct + pad))
    }
}
