use super::{
    AxisLabel, Label, 
    super::abbr_large_numbers,
    calc_label_rect,
};

fn calc_optimal_ticks(
    highest: f32, 
    lowest: f32, 
    labels_can_fit: i32, 
    tick_size: f32
) -> (f32, f32) {
    let range = highest - lowest;
    let labels = labels_can_fit as f32;

    // Find the order of magnitude of the range
    let base = 10.0f32.powf(range.log10().floor());

    // Try steps of 1, 2, 5 times the base magnitude
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
        5.0 * base
    };

    let rounded_lowest = (lowest / step).floor() * step;
    let rounded_lowest = (rounded_lowest / tick_size).round() * tick_size;

    (step, rounded_lowest)
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
    let labels_can_fit = (bounds.height / (text_size * 4.0)) as i32;

    let (step, min) = calc_optimal_ticks(highest, lowest, labels_can_fit, tick_size);
    
    let mut labels = Vec::with_capacity((labels_can_fit + 2) as usize);

    let mut value = min;
    while value <= highest {
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

        value += step;
    }

    labels
}
