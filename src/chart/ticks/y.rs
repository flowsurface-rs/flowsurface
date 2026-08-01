use super::{AxisLabel, LabelContent, calc_label_rect};
pub use data::chart::ticks::y::PriceAxis;
use data::chart::ticks::y::{TickValue, YAxisScale};
use data::util::abbr_large_numbers;
use exchange::unit::Price;

/// Shared layout context for building Y-axis labels on a single axis canvas.
///
/// Holds the layout parameters every label shares (canvas bounds, text size
/// and color) and offers one method per label strategy: a single fallback
/// label, a row-aligned grid, or a plain float grid.
pub struct LabelLayout {
    bounds: iced::Rectangle,
    text_size: f32,
    text_color: iced::Color,
    labels_can_fit: i32,
}

impl LabelLayout {
    pub fn new(bounds: iced::Rectangle, text_size: f32, text_color: iced::Color) -> Self {
        let labels_can_fit = (bounds.height / (text_size * 3.0)) as i32;
        Self {
            bounds,
            text_size,
            text_color,
            labels_can_fit,
        }
    }

    /// Generate the full label set for a price range.
    ///
    /// `axis` bundles the chart's effective tick size with the label precision
    /// (the market's min tick); when set, labels are aligned to its rows and
    /// formatted at its precision. When unset, a plain float grid is used and
    /// large numbers are abbreviated.
    pub fn generate(&self, lowest: f64, highest: f64, axis: Option<PriceAxis>) -> Vec<AxisLabel> {
        if !lowest.is_finite() || !highest.is_finite() || highest - lowest <= 0.0 {
            return Vec::new();
        }

        if self.labels_can_fit <= 1 {
            return match axis {
                Some(axis) => self.single_row(highest, axis),
                None => self.single_y(highest, None),
            };
        }

        match axis {
            Some(axis) => self.row_grid(lowest, highest, axis),
            None => self.float_grid(lowest, highest),
        }
    }

    /// A single label built from a ready-made content string.
    fn single(&self, content: String) -> Vec<AxisLabel> {
        let label = LabelContent {
            content,
            background_color: None,
            text_color: self.text_color,
            text_size: self.text_size,
        };

        vec![AxisLabel::Y {
            bounds: calc_label_rect(0.0, 1, self.text_size, self.bounds),
            value_label: label,
            timer_label: None,
        }]
    }

    /// A single label for `value` at `decimals` decimal places when available.
    fn single_y(&self, value: f64, decimals: Option<usize>) -> Vec<AxisLabel> {
        let content = if let Some(decimals) = decimals {
            format!("{value:.decimals$}")
        } else {
            abbr_large_numbers(value)
        };
        self.single(content)
    }

    /// A single fallback label aligned down to the nearest price row,
    /// formatted at the axis precision. Keeps the axis pointing at a real row
    /// even when there is only room for one label.
    fn single_row(&self, value: f64, axis: PriceAxis) -> Vec<AxisLabel> {
        let aligned = axis.floor_to_row(value);
        let content = axis.format_units(aligned);
        self.single(content)
    }

    /// A grid of labels aligned exactly to the price row grid. Every label
    /// lands on a real row; the grid math lives in `data::chart::ticks`
    /// ([`PriceAxis::ticks`]).
    fn row_grid(&self, lowest: f64, highest: f64, axis: PriceAxis) -> Vec<AxisLabel> {
        let ticks = axis.ticks(lowest, highest, self.labels_can_fit);
        if ticks.ticks.is_empty() {
            return self.single_row(highest, axis);
        }

        let mut labels = Vec::with_capacity((self.labels_can_fit + 2) as usize);
        for tick in ticks.ticks {
            let TickValue::PriceUnits(units) = tick.value else {
                continue;
            };
            let content = axis.format_units(units);

            let label_pos = self.bounds.height - tick.ratio as f32 * self.bounds.height;

            labels.push(AxisLabel::Y {
                bounds: calc_label_rect(label_pos, 1, self.text_size, self.bounds),
                value_label: LabelContent {
                    content,
                    background_color: None,
                    text_color: self.text_color,
                    text_size: self.text_size,
                },
                timer_label: None,
            });
        }

        labels
    }

    /// A grid of labels from a plain f64 range (used for indicator axes that
    /// have no row grid). Labels use abbreviated large-number formatting. The
    /// grid values themselves are computed by `data::chart::ticks`
    /// ([`YAxisScale::Linear`]).
    fn float_grid(&self, lowest: f64, highest: f64) -> Vec<AxisLabel> {
        let ticks = YAxisScale::Linear.ticks(lowest, highest, self.labels_can_fit);
        if ticks.ticks.is_empty() {
            return self.single_y(highest, None);
        }

        let mut labels = Vec::with_capacity((self.labels_can_fit + 2) as usize);
        for tick in ticks.ticks {
            let TickValue::Float(value) = tick.value else {
                continue;
            };
            let content = abbr_large_numbers(value);
            let label_pos = self.bounds.height - tick.ratio as f32 * self.bounds.height;

            labels.push(AxisLabel::Y {
                bounds: calc_label_rect(label_pos, 1, self.text_size, self.bounds),
                value_label: LabelContent {
                    content,
                    background_color: None,
                    text_color: self.text_color,
                    text_size: self.text_size,
                },
                timer_label: None,
            });
        }

        labels
    }
}

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
