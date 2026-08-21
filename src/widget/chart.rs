pub mod heatmap;
pub mod kline;

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
