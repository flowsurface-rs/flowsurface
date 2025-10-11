pub mod comparison;

use chrono::{TimeZone, Utc};
use exchange::TickerInfo;

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
    pub name: TickerInfo,
    /// (x, y) where x is a time-like domain (e.g., timestamp) and y is raw value
    pub points: Vec<(u64, f32)>,
    pub color: Option<iced::Color>,
}

pub trait SeriesLike {
    fn name(&self) -> String;
    fn points(&self) -> &[(u64, f32)];
    fn color(&self) -> Option<iced::Color>;
    fn ticker_info(&self) -> &TickerInfo;
}

impl SeriesLike for Series {
    fn name(&self) -> String {
        self.name.ticker.symbol_and_exchange_string()
    }
    fn points(&self) -> &[(u64, f32)] {
        &self.points
    }
    fn color(&self) -> Option<iced::Color> {
        self.color
    }
    fn ticker_info(&self) -> &TickerInfo {
        &self.name
    }
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
    let Some(dt) = Utc.timestamp_millis_opt(ts_ms as i64).single() else {
        return String::new();
    };
    // Choose format based on step size
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
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

// Linear interpolation helper
fn interpolate_y_at(pts: &[(u64, f32)], x: u64) -> Option<f32> {
    if pts.is_empty() {
        return None;
    }
    // binary search
    match pts.binary_search_by(|(px, _)| px.cmp(&x)) {
        Ok(i) => Some(pts[i].1),
        Err(i) => {
            if i == 0 {
                // before first: match existing behavior (use first point)
                Some(pts[0].1)
            } else if i >= pts.len() {
                None
            } else {
                let (x0, y0) = pts[i - 1];
                let (x1, y1) = pts[i];
                let dx = (x1.saturating_sub(x0)) as f32;
                if dx > 0.0 {
                    let t = (x.saturating_sub(x0)) as f32 / dx;
                    Some(y0 + (y1 - y0) * t.clamp(0.0, 1.0))
                } else {
                    Some(y0)
                }
            }
        }
    }
}
