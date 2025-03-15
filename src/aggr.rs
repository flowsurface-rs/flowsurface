pub mod ticks;
pub mod time;

fn round_to_tick(value: f32, tick_size: f32) -> f32 {
    (value / tick_size).round() * tick_size
}

pub fn format_with_commas(num: f32) -> String {
    let s = format!("{num:.0}");

    // Handle special case for small numbers
    if s.len() <= 4 && s.starts_with('-') {
        return s; // Return as-is if it's a small negative number
    }

    let mut result = String::with_capacity(s.len() + (s.len() - 1) / 3);
    let (sign, digits) = if s.starts_with('-') {
        ("-", &s[1..]) // Split into sign and digits
    } else {
        ("", &s[..])
    };

    let mut i = digits.len();
    while i > 0 {
        if !result.is_empty() {
            result.insert(0, ',');
        }
        let start = if i >= 3 { i - 3 } else { 0 };
        result.insert_str(0, &digits[start..i]);
        i = start;
    }

    // Add sign at the start if negative
    if !sign.is_empty() {
        result.insert_str(0, sign);
    }

    result
}
