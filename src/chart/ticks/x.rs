use super::{AxisLabel, TEXT_SIZE};
use data::chart::ticks::x::{ONE_DAY_MS, TimeAxisGrid};
use data::{UserTimezone, config::timezone::TimeLabelKind};

use chrono::{DateTime, Datelike};
use exchange::UnixMs;
use iced::theme::palette::Extended;
use iced_core::Rectangle;

fn is_drawable(x_pos: f64, width: f32) -> bool {
    x_pos >= (-TEXT_SIZE * 5.0).into() && x_pos <= f64::from(width) + f64::from(TEXT_SIZE * 5.0)
}

pub fn generate_time_labels(
    timeframe: exchange::Timeframe,
    timezone: UserTimezone,
    axis_bounds: iced_core::Rectangle,
    earliest: UnixMs,
    latest: UnixMs,
    x_labels_can_fit: i32,
    palette: &Extended,
) -> Vec<AxisLabel> {
    // All grid math (step selection and label timestamps) lives in
    // `data::chart::ticks`; this function only formats and places labels.
    let grid = TimeAxisGrid::new(earliest, latest, x_labels_can_fit, timeframe);

    if grid.step_ms == 0 {
        return vec![];
    }

    let mut labels = Vec::with_capacity(x_labels_can_fit as usize * 3);

    if grid.step_ms >= ONE_DAY_MS {
        boundary_layer(
            &grid.daily,
            &grid,
            timezone,
            axis_bounds,
            palette,
            // day-of-month label is hidden on Jan 1 (year label takes over)
            |dt| dt.month() == 1 && dt.day() == 1,
            |dt| dt.format("%d").to_string(),
            &mut labels,
        );

        boundary_layer(
            &grid.monthly,
            &grid,
            timezone,
            axis_bounds,
            palette,
            // month label is hidden in January (year label takes over)
            |dt| dt.month() == 1,
            |dt| dt.format("%b").to_string(),
            &mut labels,
        );

        boundary_layer(
            &grid.yearly,
            &grid,
            timezone,
            axis_bounds,
            palette,
            |_dt| false,
            |dt| dt.format("%Y").to_string(),
            &mut labels,
        );
    } else {
        for &ts in &grid.sub_daily {
            let x_pos = grid.x_position(ts, axis_bounds.width);
            if !is_drawable(x_pos, axis_bounds.width) {
                continue;
            }

            if let Some(content) =
                timezone.format_with_kind(ts as i64, TimeLabelKind::Axis { timeframe })
            {
                labels.push(AxisLabel::new_x(
                    x_pos as f32,
                    content,
                    axis_bounds,
                    false,
                    palette,
                ));
            }
        }
    }

    labels
}

/// Renders one calendar-boundary layer (daily / monthly / yearly) from the
/// precomputed timestamps in `data::chart::ticks`, applying the layer's
/// skip rule and formatting each label in the user's timezone.
fn boundary_layer(
    timestamps: &[u64],
    grid: &TimeAxisGrid,
    timezone: UserTimezone,
    axis_bounds: Rectangle,
    palette: &Extended,
    skip: impl Fn(&DateTime<chrono::FixedOffset>) -> bool,
    format: impl Fn(&DateTime<chrono::FixedOffset>) -> String,
    out: &mut Vec<AxisLabel>,
) {
    for &ts in timestamps {
        let x_pos = grid.x_position(ts, axis_bounds.width);
        if !is_drawable(x_pos, axis_bounds.width) {
            continue;
        }

        let Some(dt_utc) = UnixMs::new(ts).as_datetime_utc() else {
            continue;
        };
        let dt_user = timezone.to_user_datetime(dt_utc);
        if skip(&dt_user) {
            continue;
        }

        out.push(AxisLabel::new_x(
            x_pos as f32,
            format(&dt_user),
            axis_bounds,
            false,
            palette,
        ));
    }
}
