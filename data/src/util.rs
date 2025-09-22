use chrono::{DateTime, Datelike, Timelike};
use serde::{Deserialize, Deserializer};

const DAY_MS: u64 = 86_400_000;
const HOUR_MS: u64 = 3_600_000;
const MINUTE_MS: u64 = 60_000;
const SECOND_MS: u64 = 1_000;

pub fn ok_or_default<'a, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'a> + Default,
    D: Deserializer<'a>,
{
    let v: serde_json::Value = Deserialize::deserialize(deserializer)?;
    Ok(T::deserialize(v).unwrap_or_default())
}

pub fn abbr_large_numbers(value: f32) -> String {
    let abs_value = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };

    match abs_value {
        v if v >= 1_000_000_000.0 => {
            format!("{}{:.3}b", sign, v / 100_000_000.0)
        }
        v if v >= 1_000_000.0 => format!("{}{:.2}m", sign, v / 1_000_000.0),
        v if v >= 10_000.0 => format!("{}{:.1}k", sign, v / 1_000.0),
        v if v >= 1_000.0 => format!("{}{:.2}k", sign, v / 1_000.0),
        v if v >= 100.0 => format!("{}{:.0}", sign, v),
        v if v >= 10.0 => format!("{}{:.1}", sign, v),
        v if v >= 1.0 => format!("{}{:.2}", sign, v),
        v if v >= 0.001 => format!("{}{:.3}", sign, v),
        v if v >= 0.0001 => format!("{}{:.4}", sign, v),
        v if v >= 0.00001 => format!("{}{:.5}", sign, v),
        _ => {
            if abs_value == 0.0 {
                "0".to_string()
            } else {
                let s = format!("{}{:.3}", sign, abs_value);
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    }
}

pub fn count_decimals(value: f32) -> usize {
    let value_str = value.to_string();
    if let Some(pos) = value_str.find('.') {
        value_str.len() - pos - 1
    } else {
        0
    }
}

pub fn format_with_commas(num: f32) -> String {
    if num == 0.0 {
        return "0".to_string();
    }

    let abs_num = num.abs();
    let decimals = match abs_num {
        n if n >= 1000.0 => 0,
        n if n >= 100.0 => 1,
        n if n >= 10.0 => 2,
        _ => 3,
    };

    let is_negative = num < 0.0;

    if abs_num < 1000.0 {
        return format!(
            "{}{:.*}",
            if is_negative { "-" } else { "" },
            decimals,
            abs_num
        );
    }

    let s = format!("{:.*}", decimals, abs_num);

    let (integer_part, decimal_part) = match s.find('.') {
        Some(pos) => (&s[..pos], Some(&s[pos..])),
        None => (s.as_str(), None),
    };

    let mut result = {
        let num_commas = (integer_part.len() - 1) / 3;
        let decimal_len = decimal_part.map_or(0, str::len);

        String::with_capacity(
            usize::from(is_negative) + integer_part.len() + num_commas + decimal_len,
        )
    };

    if is_negative {
        result.push('-');
    }

    let digits_len = integer_part.len();
    for (i, ch) in integer_part.chars().enumerate() {
        result.push(ch);

        let pos_from_right = digits_len - i - 1;
        if i < digits_len - 1 && pos_from_right % 3 == 0 {
            result.push(',');
        }
    }

    if let Some(decimal) = decimal_part {
        result.push_str(decimal);
    }

    result
}

pub fn round_to_tick(value: f32, tick_size: f32) -> f32 {
    (value / tick_size).round() * tick_size
}

pub fn round_to_next_tick(value: f32, tick_size: f32, down: bool) -> f32 {
    if down {
        (value / tick_size).floor() * tick_size
    } else {
        (value / tick_size).ceil() * tick_size
    }
}

pub fn currency_abbr(price: f32) -> String {
    match price {
        p if p > 1_000_000_000.0 => format!("${:.2}b", p / 1_000_000_000.0),
        p if p > 1_000_000.0 => format!("${:.1}m", p / 1_000_000.0),
        p if p > 1000.0 => format!("${:.2}k", p / 1000.0),
        _ => format!("${:.2}", price),
    }
}

pub fn pct_change(change: f32) -> String {
    match change {
        c if c > 0.0 => format!("+{:.2}%", c),
        _ => format!("{:.2}%", change),
    }
}

pub fn guesstimate_ticks(range: f32) -> f32 {
    match range {
        r if r > 1_000_000_000.0 => 1_000_000.0,
        r if r > 100_000_000.0 => 100_000.0,
        r if r > 10_000_000.0 => 10_000.0,
        r if r > 1_000_000.0 => 1_000.0,
        r if r > 100_000.0 => 1_000.0,
        r if r > 10_000.0 => 100.0,
        r if r > 1_000.0 => 10.0,
        r if r > 100.0 => 1.0,
        r if r > 10.0 => 0.1,
        r if r > 1.0 => 0.01,
        r if r > 0.1 => 0.001,
        r if r > 0.01 => 0.0001,
        _ => 0.00001,
    }
}

pub fn format_duration_ms(diff_ms: u64) -> String {
    if diff_ms >= DAY_MS {
        let days = diff_ms / DAY_MS;
        let hours = (diff_ms % DAY_MS) / HOUR_MS;
        if hours > 0 {
            format!("{}d {}h", days, hours)
        } else {
            format!("{}d", days)
        }
    } else if diff_ms >= HOUR_MS {
        let hours = diff_ms / HOUR_MS;
        let mins = (diff_ms % HOUR_MS) / MINUTE_MS;
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    } else if diff_ms >= MINUTE_MS {
        let mins = diff_ms / MINUTE_MS;
        let secs = (diff_ms % MINUTE_MS) / SECOND_MS;
        if secs > 0 {
            format!("{}m {}s", mins, secs)
        } else {
            format!("{}m", mins)
        }
    } else if diff_ms >= 5_000 {
        format!("{}s", diff_ms / SECOND_MS)
    } else {
        format!("{}ms", diff_ms)
    }
}

/// Shrinks main panel if needed when adding a new panel.
/// Ensures indicators never shrink below `MIN_PANEL_HEIGHT`
pub fn calc_panel_splits(
    initial_main_split: f32,
    active_indicators: usize,
    previous_indicators: Option<usize>,
) -> Vec<f32> {
    const MIN_PANEL_HEIGHT: f32 = 0.1;
    const TOTAL_HEIGHT: f32 = 1.0;

    let mut main_split = initial_main_split;

    if let Some(prev_inds) = previous_indicators
        && active_indicators > prev_inds
    {
        let min_space_needed_all_indis = active_indicators as f32 * MIN_PANEL_HEIGHT;

        let max_main_split_if_indis_get_min =
            (TOTAL_HEIGHT - min_space_needed_all_indis).max(MIN_PANEL_HEIGHT);

        if main_split > max_main_split_if_indis_get_min {
            main_split = max_main_split_if_indis_get_min;
        }
    }

    let upper_bound_for_main = if active_indicators == 0 {
        TOTAL_HEIGHT
    } else {
        (TOTAL_HEIGHT - active_indicators as f32 * MIN_PANEL_HEIGHT).max(MIN_PANEL_HEIGHT)
    };

    main_split = main_split.clamp(MIN_PANEL_HEIGHT, upper_bound_for_main);
    main_split = main_split.min(TOTAL_HEIGHT);

    let mut splits = vec![main_split];

    if active_indicators > 1 {
        let indicator_total_space = (TOTAL_HEIGHT - main_split).max(0.0);
        let per_indicator_space = indicator_total_space / active_indicators as f32;

        for i in 1..active_indicators {
            let cumulative_indicator_space = per_indicator_space * i as f32;
            let split_pos = main_split + cumulative_indicator_space;
            splits.push(split_pos.min(TOTAL_HEIGHT));
        }
    }
    splits
}

pub fn reset_to_start_of_day_utc(dt: DateTime<chrono::Utc>) -> DateTime<chrono::Utc> {
    dt.with_hour(0)
        .unwrap_or(dt)
        .with_minute(0)
        .unwrap_or(dt)
        .with_second(0)
        .unwrap_or(dt)
        .with_nanosecond(0)
        .unwrap_or(dt)
}

pub fn reset_to_start_of_month_utc(dt: DateTime<chrono::Utc>) -> DateTime<chrono::Utc> {
    reset_to_start_of_day_utc(dt.with_day(1).unwrap_or(dt))
}

pub fn reset_to_start_of_year_utc(dt: DateTime<chrono::Utc>) -> DateTime<chrono::Utc> {
    reset_to_start_of_month_utc(dt.with_month(1).unwrap_or(dt))
}
