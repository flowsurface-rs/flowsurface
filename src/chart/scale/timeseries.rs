use crate::chart::scale::{AxisLabel, TEXT_SIZE};
use data::{
    UserTimezone,
    util::{reset_to_start_of_month_utc, reset_to_start_of_year_utc},
};

use chrono::{DateTime, Datelike, Months};
use iced::theme::palette::Extended;
use iced_core::Rectangle;

pub const ONE_DAY_MS: u64 = 24 * 60 * 60 * 1000;

const M1_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 720, // 12 hour
    1000 * 60 * 180, // 3 hour
    1000 * 60 * 60,  // 1 hour
    1000 * 60 * 30,  // 30 min
    1000 * 60 * 15,  // 15 min
    1000 * 60 * 10,  // 10 min
    1000 * 60 * 5,   // 5 min
    1000 * 60 * 2,   // 2 min
    1000 * 60,       // 1 min
];

const M3_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 360,  // 6 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 9,    // 9 min
    1000 * 60 * 3,    // 3 min
];

const M5_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 5,    // 5 min
];

const HOURLY_TIME_STEPS: [u64; 8] = [
    1000 * 60 * 5760, // 96 hour
    1000 * 60 * 2880, // 48 hour
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
];

const MS_TIME_STEPS: [u64; 10] = [
    1000 * 120,
    1000 * 60,
    1000 * 30,
    1000 * 10,
    1000 * 5,
    1000 * 2,
    1000,
    500,
    200,
    100,
];

fn calc_time_step(
    earliest: u64,
    latest: u64,
    labels_can_fit: i32,
    timeframe: exchange::Timeframe,
) -> (u64, u64) {
    let timeframe_in_min = timeframe.to_milliseconds() / 60_000;

    let time_steps: &[u64] = match timeframe_in_min {
        0_u64..1_u64 => &MS_TIME_STEPS,
        1..=30 => match timeframe_in_min {
            1 => &M1_TIME_STEPS,
            3 => &M3_TIME_STEPS,
            5 => &M5_TIME_STEPS,
            15 => &M5_TIME_STEPS[..7],
            30 => &M5_TIME_STEPS[..6],
            _ => &HOURLY_TIME_STEPS,
        },
        31.. => &HOURLY_TIME_STEPS,
    };

    let duration = latest - earliest;
    let mut selected_step = time_steps[0];

    for &step in time_steps {
        if duration / step >= (labels_can_fit as u64) {
            selected_step = step;
            break;
        }
        if step <= duration {
            selected_step = step;
        }
    }

    let rounded_earliest = (earliest / selected_step) * selected_step;

    (selected_step, rounded_earliest)
}

fn calc_x_pos(time_millis: u64, min_millis: u64, max_millis: u64, width: f32) -> f64 {
    if max_millis > min_millis {
        ((time_millis - min_millis) as f64 / (max_millis - min_millis) as f64) * f64::from(width)
    } else {
        0.0
    }
}

fn is_drawable(x_pos: f64, width: f32) -> bool {
    x_pos >= (-TEXT_SIZE * 5.0).into() && x_pos <= f64::from(width) + f64::from(TEXT_SIZE * 5.0)
}

pub fn generate_time_labels(
    timeframe: exchange::Timeframe,
    timezone: UserTimezone,
    axis_bounds: iced_core::Rectangle,
    x_min: u64,
    x_max: u64,
    x_labels_can_fit: i32,
    palette: &Extended,
) -> Vec<AxisLabel> {
    let (time_step, initial_rounded_earliest) =
        calc_time_step(x_min, x_max, x_labels_can_fit, timeframe);

    if time_step == 0 {
        return vec![];
    }

    let mut labels = Vec::with_capacity(x_labels_can_fit as usize * 3);

    if time_step >= ONE_DAY_MS {
        let Some(start_utc_dt) = DateTime::from_timestamp_millis(x_min as i64) else {
            return vec![];
        };
        let Some(end_utc_dt) = DateTime::from_timestamp_millis(x_max as i64) else {
            return vec![];
        };

        daily_labels_gen(
            timezone,
            &mut labels,
            axis_bounds,
            x_min,
            x_max,
            start_utc_dt,
            end_utc_dt,
            calc_x_pos,
            is_drawable,
            palette,
        );

        monthly_labels_gen(
            timezone,
            &mut labels,
            axis_bounds,
            x_min,
            x_max,
            start_utc_dt,
            end_utc_dt,
            calc_x_pos,
            is_drawable,
            palette,
        );

        yearly_labels_gen(
            &mut labels,
            axis_bounds,
            x_min,
            x_max,
            start_utc_dt,
            end_utc_dt,
            calc_x_pos,
            is_drawable,
            palette,
        );
    } else {
        sub_daily_labels_gen(
            timezone,
            &mut labels,
            axis_bounds,
            x_min,
            x_max,
            time_step,
            initial_rounded_earliest,
            timeframe,
            calc_x_pos,
            is_drawable,
            palette,
        );
    }

    labels
}

fn above_daily_labels_gen<Tz, Next, Format, Skip>(
    mut current: chrono::DateTime<Tz>,
    end: &chrono::DateTime<Tz>,
    x_min: u64,
    x_max: u64,
    axis_bounds: iced_core::Rectangle,
    all_labels: &mut Vec<AxisLabel>,
    calc_x_pos: impl Fn(u64, u64, u64, f32) -> f64,
    is_drawable: impl Fn(f64, f32) -> bool,
    next: Next,
    format_label: Format,
    skip_label: Skip,
    palette: &Extended,
) where
    Tz: chrono::TimeZone,
    Next: Fn(&chrono::DateTime<Tz>) -> Option<chrono::DateTime<Tz>>,
    Format: Fn(&chrono::DateTime<Tz>) -> String,
    Skip: Fn(&chrono::DateTime<Tz>) -> bool,
{
    while current.timestamp_millis() as u64 <= x_max {
        let ts = current.timestamp_millis() as u64;

        if ts >= x_min && !skip_label(&current) {
            let x_pos = calc_x_pos(ts, x_min, x_max, axis_bounds.width);
            if is_drawable(x_pos, axis_bounds.width) {
                let label = format_label(&current);
                all_labels.push(AxisLabel::new_x(
                    x_pos as f32,
                    label,
                    axis_bounds,
                    false,
                    palette,
                ));
            }
        }

        if let Some(next_dt) = next(&current) {
            current = next_dt;
        } else {
            break;
        }

        if current > *end {
            break;
        }
    }
}

fn daily_labels_gen(
    timezone: UserTimezone,
    all_labels: &mut Vec<AxisLabel>,
    axis_bounds: Rectangle,
    x_min: u64,
    x_max: u64,
    start_utc_dt: DateTime<chrono::Utc>,
    end_utc_dt: DateTime<chrono::Utc>,
    calc_x_pos: impl Fn(u64, u64, u64, f32) -> f64,
    is_drawable: impl Fn(f64, f32) -> bool,
    palette: &Extended,
) {
    let current = start_utc_dt
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map_or(start_utc_dt, |d| {
            DateTime::<chrono::Utc>::from_naive_utc_and_offset(d, chrono::Utc)
        });

    above_daily_labels_gen(
        current,
        &end_utc_dt,
        x_min,
        x_max,
        axis_bounds,
        all_labels,
        &calc_x_pos,
        &is_drawable,
        move |dt| dt.checked_add_signed(chrono::Duration::days(1)),
        with_user_timezone(timezone, |dt| dt.format("%d").to_string()),
        with_user_timezone(timezone, |dt| dt.month() == 1 && dt.day() == 1),
        palette,
    );
}

fn monthly_labels_gen(
    timezone: UserTimezone,
    all_labels: &mut Vec<AxisLabel>,
    axis_bounds: Rectangle,
    x_min: u64,
    x_max: u64,
    start_utc_dt: DateTime<chrono::Utc>,
    end_utc_dt: DateTime<chrono::Utc>,
    calc_x_pos: impl Fn(u64, u64, u64, f32) -> f64,
    is_drawable: impl Fn(f64, f32) -> bool,
    palette: &Extended,
) {
    let current = reset_to_start_of_month_utc(start_utc_dt);

    above_daily_labels_gen(
        current,
        &end_utc_dt,
        x_min,
        x_max,
        axis_bounds,
        all_labels,
        &calc_x_pos,
        &is_drawable,
        |dt| {
            dt.checked_add_months(Months::new(1))
                .map(reset_to_start_of_month_utc)
        },
        with_user_timezone(timezone, |dt| dt.format("%b").to_string()),
        with_user_timezone(timezone, |dt| dt.month() == 1),
        palette,
    );
}

fn yearly_labels_gen(
    all_labels: &mut Vec<AxisLabel>,
    axis_bounds: Rectangle,
    x_min: u64,
    x_max: u64,
    start_utc_dt: DateTime<chrono::Utc>,
    end_utc_dt: DateTime<chrono::Utc>,
    calc_x_pos: impl Fn(u64, u64, u64, f32) -> f64,
    is_drawable: impl Fn(f64, f32) -> bool,
    palette: &Extended,
) {
    let current = reset_to_start_of_year_utc(start_utc_dt);

    above_daily_labels_gen(
        current,
        &end_utc_dt,
        x_min,
        x_max,
        axis_bounds,
        all_labels,
        &calc_x_pos,
        &is_drawable,
        |dt| {
            dt.checked_add_months(Months::new(12))
                .map(reset_to_start_of_year_utc)
        },
        |dt| dt.format("%Y").to_string(),
        |_dt| false,
        palette,
    );
}

fn sub_daily_labels_gen(
    timezone: UserTimezone,
    all_labels: &mut Vec<AxisLabel>,
    axis_bounds: Rectangle,
    x_min: u64,
    x_max: u64,
    time_step: u64,
    initial_rounded_earliest: u64,
    timeframe: exchange::Timeframe,
    calc_x_pos: impl Fn(u64, u64, u64, f32) -> f64,
    is_drawable: impl Fn(f64, f32) -> bool,
    palette: &Extended,
) {
    let mut current_time = initial_rounded_earliest;

    while current_time <= x_max {
        if current_time >= x_min {
            let x_position = calc_x_pos(current_time, x_min, x_max, axis_bounds.width);

            if is_drawable(x_position, axis_bounds.width) {
                let label_content = timezone.format_with_kind(
                    current_time as i64,
                    data::config::timezone::TimeLabelKind::Axis { timeframe },
                );

                if let Some(content) = label_content {
                    all_labels.push(AxisLabel::new_x(
                        x_position as f32,
                        content,
                        axis_bounds,
                        false,
                        palette,
                    ));
                }
            }
        }
        let prev_current_time = current_time;
        current_time = current_time.saturating_add(time_step);
        if current_time <= prev_current_time && time_step > 0 {
            break;
        }

        if current_time > x_max && prev_current_time < x_min {
            break;
        }
    }
}

fn with_user_timezone<Tz, F, R>(timezone: UserTimezone, f: F) -> impl Fn(&chrono::DateTime<Tz>) -> R
where
    Tz: chrono::TimeZone,
    F: Fn(&chrono::DateTime<chrono::FixedOffset>) -> R,
{
    move |dt| {
        let utc = dt.with_timezone(&chrono::Utc);
        let dt_in_timezone = timezone.to_user_datetime(utc);
        f(&dt_in_timezone)
    }
}
