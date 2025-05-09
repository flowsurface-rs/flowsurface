pub fn abbr_large_numbers(value: f32) -> String {
    let abs_value = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };

    if abs_value >= 1_000_000_000.0 {
        format!("{}{:.2}b", sign, abs_value / 1_000_000_000.0)
    } else if abs_value >= 1_000_000.0 {
        format!("{}{:.2}m", sign, abs_value / 1_000_000.0)
    } else if abs_value >= 1000.0 {
        format!("{}{:.1}k", sign, abs_value / 1000.0)
    } else if abs_value >= 100.0 {
        format!("{}{:.0}", sign, abs_value)
    } else if abs_value >= 10.0 {
        format!("{}{:.1}", sign, abs_value)
    } else if abs_value >= 1.0 {
        format!("{}{:.2}", sign, abs_value)
    } else {
        format!("{}{:.3}", sign, abs_value)
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
    let is_negative = num < 0.0;

    let decimals = if abs_num >= 100.0 {
        0
    } else if abs_num >= 10.0 {
        1
    } else if abs_num >= 1.0 {
        2
    } else {
        3
    };

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

    let num_commas = (integer_part.len() - 1) / 3;
    let decimal_len = decimal_part.map_or(0, str::len);
    let capacity = usize::from(is_negative) + integer_part.len() + num_commas + decimal_len;

    let mut result = String::with_capacity(capacity);

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
