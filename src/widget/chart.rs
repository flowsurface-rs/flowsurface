pub mod comparison;

use chrono::{TimeZone, Utc};
use exchange::TickerInfo;

/// Represents the horizontal scale as pixels per time unit (bar/candle).
/// Higher values = more zoomed in (fewer bars visible, wider bars).
/// Lower values = more zoomed out (more bars visible, narrower bars).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BarWidth(pub f32);

impl Default for BarWidth {
    fn default() -> Self {
        Self(8.0) // 8 pixels per bar as default
    }
}

impl BarWidth {
    // Non-BBO (minute+ timeframes) constraints
    pub const MIN: f32 = 1.0;
    pub const MAX: f32 = 100.0;

    // BBO (sub-minute timeframes) constraints - allow much smaller values
    pub const MIN_BBO: f32 = 0.05;
    pub const MAX_BBO: f32 = 20.0;

    pub fn new(px: f32) -> Self {
        Self(px.clamp(Self::MIN, Self::MAX))
    }

    /// Create a new BarWidth with BBO-mode constraints
    pub fn new_bbo(px: f32) -> Self {
        Self(px.clamp(Self::MIN_BBO, Self::MAX_BBO))
    }

    /// Clamp to appropriate range based on whether this is BBO mode
    pub fn clamped(self, is_bbo: bool) -> Self {
        if is_bbo {
            Self(self.0.clamp(Self::MIN_BBO, Self::MAX_BBO))
        } else {
            Self(self.0.clamp(Self::MIN, Self::MAX))
        }
    }

    pub fn pixels(&self) -> f32 {
        self.0
    }

    /// Calculate how many milliseconds are visible given viewport width
    pub fn visible_span_ms(&self, viewport_width: f32, timeframe_ms: u64) -> u64 {
        let bars_visible = (viewport_width / self.0).max(1.0);
        (bars_visible * timeframe_ms as f32).round() as u64
    }

    /// Zoom in by a percentage (increases bar width), respecting mode constraints
    pub fn zoom_in(&self, pct: f32) -> Self {
        Self(self.0 * (1.0 + pct))
    }

    /// Zoom out by a percentage (decreases bar width), respecting mode constraints
    pub fn zoom_out(&self, pct: f32) -> Self {
        Self(self.0 / (1.0 + pct))
    }

    /// Zoom in with mode-aware clamping
    pub fn zoom_in_clamped(&self, pct: f32, is_bbo: bool) -> Self {
        self.zoom_in(pct).clamped(is_bbo)
    }

    /// Zoom out with mode-aware clamping
    pub fn zoom_out_clamped(&self, pct: f32, is_bbo: bool) -> Self {
        self.zoom_out(pct).clamped(is_bbo)
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

    /// Drop points older than `min_x` (by x/time), but keep one point just before `min_x`
    /// as an anchor for continuity/interpolation.
    pub fn trim_before_x(&mut self, min_x: u64) {
        if self.points.is_empty() {
            return;
        }

        let idx = self.points.partition_point(|(x, _)| *x < min_x);
        let drain_up_to = idx.saturating_sub(1);

        if drain_up_to > 0 {
            self.points.drain(0..drain_up_to);
        }
    }

    /// Trim points to keep the series size manageable.
    /// If the number of points exceeds `max_points * trigger_multiplier`,
    /// remove `len * drain_multiplier` points from the start.
    /// This helps to prevent unbounded memory growth for real-time data.
    pub fn trim_to_max_points(
        &mut self,
        max_points: usize,
        trigger_multiplier: f32,
        drain_multiplier: f32,
    ) {
        let max_points = max_points as f32;
        if (self.points.len() as f32) > max_points * trigger_multiplier {
            let drain_count = (self.points.len() as f32 * drain_multiplier) as usize;
            self.points.drain(0..drain_count.min(self.points.len()));
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

/// Align timestamp down to the nearest multiple of `dt`
pub fn align_floor(ts: u64, dt: u64) -> u64 {
    if dt == 0 {
        return ts;
    }
    (ts / dt) * dt
}

/// Align timestamp up to the nearest multiple of `dt`
pub fn align_ceil(ts: u64, dt: u64) -> u64 {
    if dt == 0 {
        return ts;
    }
    let f = (ts / dt) * dt;
    if f == ts { ts } else { f.saturating_add(dt) }
}

/// Returns true if timeframe is sub-minute (BBO-based)
pub fn is_bbo_timeframe(timeframe: &exchange::Timeframe) -> bool {
    timeframe.to_milliseconds() < 60_000
}

/// Compute visible X window given reference max, pan offset, and span
pub fn compute_x_window(reference_max: u64, pan_ms: i64, span_ms: u64) -> (u64, u64) {
    let max_x = if pan_ms >= 0 {
        reference_max.saturating_sub(pan_ms as u64)
    } else {
        reference_max.saturating_add((-pan_ms) as u64)
    };
    let min_x = max_x.saturating_sub(span_ms);
    (min_x, max_x)
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

fn ticks(min: f32, max: f32, target: usize) -> (Vec<f32>, f32) {
    let span = (max - min).abs().max(1e-6);
    let step = nice_step(span, target);
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

fn format_pct(val: f32, step: f32, show_decimals: bool) -> String {
    let decimals = if show_decimals {
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
    } else if step >= 0.01 {
        2
    } else if step >= 0.001 {
        3
    } else {
        4
    };

    let zero_threshold = 0.5 * 10f32.powi(-(decimals));
    if val.abs() < zero_threshold {
        return "0%".to_string();
    }

    match decimals {
        0 => format!("{:+.0}%", val),
        1 => format!("{:+.1}%", val),
        2 => format!("{:+.2}%", val),
        3 => format!("{:+.3}%", val),
        _ => format!("{:+.4}%", val),
    }
}

fn time_tick_candidates() -> &'static [u64] {
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
        14 * D, //
        30 * D,
        90 * D,
        180 * D,
        365 * D,
    ]
}

fn format_time_label(ts_ms: u64, step_ms: u64) -> String {
    let Some(dt) = Utc.timestamp_millis_opt(ts_ms as i64).single() else {
        return String::new();
    };

    const S: u64 = 1_000;
    const M: u64 = 60 * S;
    const H: u64 = 60 * M;
    const D: u64 = 24 * H;

    if step_ms < M {
        dt.format("%H:%M:%S").to_string()
    } else if step_ms < D {
        dt.format("%H:%M").to_string()
    } else if step_ms < 7 * D {
        dt.format("%b %d").to_string()
    } else if step_ms < 365 * D {
        dt.format("%Y-%m").to_string()
    } else {
        dt.format("%Y").to_string()
    }
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
    let first = if min_x.is_multiple_of(step) {
        min_x
    } else {
        (min_x / step + 1) * step
    };
    let mut out = Vec::new();
    let mut t = first;
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

pub mod domain {
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
