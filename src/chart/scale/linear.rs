use super::{AxisLabel, LabelContent, calc_label_rect};
use data::util::abbr_large_numbers;
use exchange::util::Price;

const MAX_ITERATIONS: usize = 1000;

fn calc_optimal_ticks(highest: f32, lowest: f32, labels_can_fit: i32) -> (f32, f32) {
    let range = (highest - lowest).abs().max(f32::EPSILON);
    let labels = labels_can_fit.max(1) as f32;

    let base = 10.0f32.powf(range.log10().floor());

    let step = match range / base {
        r if r <= labels * 0.1 => 0.1 * base,
        r if r <= labels * 0.2 => 0.2 * base,
        r if r <= labels * 0.5 => 0.5 * base,
        r if r <= labels => base,
        r if r <= labels * 2.0 => 2.0 * base,
        _ => (range / labels).min(5.0 * base),
    };

    let rounded_highest = (highest / step).ceil() * step;
    (step, rounded_highest)
}

pub fn generate_labels(
    bounds: iced::Rectangle,
    lowest: f32,
    highest: f32,
    text_size: f32,
    text_color: iced::Color,
    decimals: Option<usize>,
) -> Vec<AxisLabel> {
    if !lowest.is_finite() || !highest.is_finite() {
        return Vec::new();
    }

    if (highest - lowest).abs() < f32::EPSILON {
        return Vec::new();
    }

    let labels_can_fit = (bounds.height / (text_size * 3.0)) as i32;

    if labels_can_fit <= 1 {
        let label = LabelContent {
            content: if let Some(decimals) = decimals {
                format!("{highest:.decimals$}")
            } else {
                abbr_large_numbers(highest)
            },
            background_color: None,
            text_color,
            text_size,
        };

        return vec![AxisLabel::Y {
            bounds: calc_label_rect(0.0, 1, text_size, bounds),
            value_label: label,
            timer_label: None,
        }];
    }

    let (step, max) = calc_optimal_ticks(highest, lowest, labels_can_fit);

    let mut value = max;
    while value > highest {
        value -= step;
    }

    let mut labels = Vec::with_capacity((labels_can_fit + 2) as usize);
    let mut safety_counter = 0;

    while value >= lowest && safety_counter < MAX_ITERATIONS {
        if value <= highest + step * 0.5 && value >= lowest - step * 0.5 {
            let content = if let Some(decimals) = decimals {
                format!("{value:.decimals$}")
            } else {
                abbr_large_numbers(value)
            };

            let label = LabelContent {
                content,
                background_color: None,
                text_color,
                text_size,
            };

            let clamped_value = value.max(lowest).min(highest);
            let label_pos =
                bounds.height - ((clamped_value - lowest) / (highest - lowest) * bounds.height);

            labels.push(AxisLabel::Y {
                bounds: calc_label_rect(label_pos, 1, text_size, bounds),
                value_label: label,
                timer_label: None,
            });
        }

        value -= step;
        safety_counter += 1;
    }

    labels
}

// other helpers
#[derive(Debug, Clone, Copy)]
pub enum PriceInfoLabel {
    Up(Price),
    Down(Price),
    Neutral(Price),
}

impl PriceInfoLabel {
    pub fn new(close_price: Price, open_price: Price) -> Self {
        if close_price >= open_price {
            PriceInfoLabel::Up(close_price)
        } else {
            PriceInfoLabel::Down(close_price)
        }
    }

    pub fn get_with_color(self, palette: &iced::theme::palette::Extended) -> (Price, iced::Color) {
        match self {
            PriceInfoLabel::Up(p) => (p, palette.success.base.color),
            PriceInfoLabel::Down(p) => (p, palette.danger.base.color),
            PriceInfoLabel::Neutral(p) => (p, palette.secondary.strong.color),
        }
    }
}
