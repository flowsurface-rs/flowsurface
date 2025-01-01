use super::{
    AxisLabel, Label, 
    super::abbr_large_numbers,
    calc_label_rect,
};

const NICE_NUMBERS: &[f32] = &[1.0, 2.0, 2.5, 4.0, 5.0, 8.0];

struct TickConfig {
    min_space: f32,
    max_labels: i32,
}

fn calc_optimal_ticks(
    highest: f32,
    lowest: f32,
    config: TickConfig,
) -> (f32, f32, f32) { // (step, rounded_min, rounded_max)
    let range = highest - lowest;
    let magnitude = 10.0f32.powf(range.log10().floor());
    
    let mut best_score = f32::MAX;
    let mut best_step = 0.0;
    
    for &nice in NICE_NUMBERS {
        for scale in [-2, -1, 0, 1, 2] {
            let step = nice * magnitude * 10.0f32.powi(scale);
            let score = evaluate_step(step, lowest, highest, &config);
            
            if score < best_score {
                best_score = score;
                best_step = step;
            }
        }
    }
    
    let rounded_min = (lowest / best_step).floor() * best_step;
    let rounded_max = (highest / best_step).ceil() * best_step;
    
    (best_step, rounded_min, rounded_max)
}

fn evaluate_step(step: f32, min: f32, max: f32, config: &TickConfig) -> f32 {
    let count = ((max - min) / step).ceil();
    let density = config.min_space / step;
    
    let label_penalty = if count > config.max_labels as f32 { 1000.0 } else { 0.0 };
    let density_penalty = (density - 1.0).abs();
    
    label_penalty + density_penalty
}

pub fn generate_labels(
    bounds: iced::Rectangle,
    lowest: f32,
    highest: f32,
    text_size: f32,
    text_color: iced::Color,
    decimals: Option<usize>,
) -> Vec<AxisLabel> {
    let config = TickConfig {
        min_space: text_size * 1.5,
        max_labels: (bounds.height / (text_size * 2.0)) as i32,
    };

    let (step, min, max) = calc_optimal_ticks(highest, lowest, config);
    let range = highest - lowest;
    let mut all_labels = Vec::new();
    let mut value = min;

    while value <= max {
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

        let label_pos = bounds.height - ((value - lowest) / range * bounds.height);

        all_labels.push(AxisLabel::Y(
            calc_label_rect(label_pos, 1, text_size, bounds),
            label,
            None,
        ));

        value += step;
    }

    all_labels
}
