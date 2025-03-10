use super::{super::abbr_large_numbers, AxisLabel, Label, calc_label_rect};

fn calc_optimal_ticks(
    highest: f32,
    lowest: f32,
    labels_can_fit: i32,
    tick_size: f32,
) -> (f32, f32) {
    let range = highest - lowest;
    let labels = labels_can_fit.max(1) as f32;

    let base = 10.0f32.powf(range.log10().floor());

    let step = if range / (0.1 * base) <= labels {
        0.1 * base
    } else if range / (0.2 * base) <= labels {
        0.2 * base
    } else if range / (0.5 * base) <= labels {
        0.5 * base
    } else if range / base <= labels {
        base
    } else if range / (2.0 * base) <= labels {
        2.0 * base
    } else {
        (range / labels).min(5.0 * base)
    };

    let rounded_highest = (highest / step).ceil() * step;
    let rounded_highest = (rounded_highest / tick_size).round() * tick_size;
    let rounded_highest = rounded_highest.min(highest + step);

    (step, rounded_highest)
}

pub fn generate_labels(
    bounds: iced::Rectangle,
    lowest: f32,
    highest: f32,
    text_size: f32,
    text_color: iced::Color,
    tick_size: f32,
    decimals: Option<usize>,
) -> Vec<AxisLabel> {
    if !lowest.is_finite() || !highest.is_finite() || !tick_size.is_finite() {
        return Vec::new();
    }

    if (highest - lowest).abs() < f32::EPSILON {
        return Vec::new();
    }

    let labels_can_fit = (bounds.height / (text_size * 3.0)) as i32;

    if labels_can_fit <= 1 {
        let rounded = (highest / tick_size).round() * tick_size;
        let label = Label {
            content: if let Some(decimals) = decimals {
                format!("{:.*}", decimals, rounded)
            } else {
                abbr_large_numbers(rounded)
            },
            background_color: None,
            text_color,
            text_size,
        };

        return vec![AxisLabel::Y(
            calc_label_rect(0.0, 1, text_size, bounds),
            label,
            None,
        )];
    }

    let (step, max) = calc_optimal_ticks(highest, lowest, labels_can_fit, tick_size);

    let mut labels = Vec::with_capacity((labels_can_fit + 2) as usize);

    let mut safety_counter = 0;
    const MAX_ITERATIONS: usize = 1000;

    let mut value = max;
    while value >= lowest && safety_counter < MAX_ITERATIONS {
        let label = Label {
            content: {
                if let Some(decimals) = decimals {
                    format!("{:.*}", decimals, value)
                } else {
                    abbr_large_numbers(value)
                }
            },
            background_color: None,
            text_color,
            text_size,
        };

        let label_pos = bounds.height - ((value - lowest) / (highest - lowest) * bounds.height);

        labels.push(AxisLabel::Y(
            calc_label_rect(label_pos, 1, text_size, bounds),
            label,
            None,
        ));

        value -= step;
        safety_counter += 1;
    }

    labels
}

#[test]
fn test_generate_labels() {
    let bounds = iced::Rectangle {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 10.0,
    };
    let lowest = 0.0;
    let highest = 100.0;
    let text_size = 12.0;
    let text_color = iced::Color::BLACK;
    let tick_size = 1.0;
    let decimals = Some(2);

    let labels = generate_labels(
        bounds, lowest, highest, text_size, text_color, tick_size, decimals,
    );

    for label in labels {
        if let AxisLabel::Y(_, label, _) = label {
            println!("{}", label.content);
        }
    }
}
