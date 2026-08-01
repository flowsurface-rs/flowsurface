use super::{AxisLabel, LabelContent, calc_label_rect};
use data::util::abbr_large_numbers;
use exchange::unit::{MinTicksize, Price, PriceStep};

const MAX_ITERATIONS: usize = 1000;

/// A "nice" grid step for a numeric range that keeps ~`labels_can_fit` lines.
///
/// The range is floored at the smallest positive f64 rather than f32::EPSILON:
/// the machine epsilon (~1.19e-7) is a huge absolute price span for sub-penny
/// markets and would force a far-too-coarse grid (or no grid at all).
fn calc_optimal_ticks(highest: f64, lowest: f64, labels_can_fit: i32) -> f64 {
    let range = (highest - lowest).abs().max(f64::MIN_POSITIVE);
    let labels = labels_can_fit.max(1) as f64;

    let base = 10.0f64.powf(range.log10().floor());

    match range / base {
        r if r <= labels * 0.1 => 0.1 * base,
        r if r <= labels * 0.2 => 0.2 * base,
        r if r <= labels * 0.5 => 0.5 * base,
        r if r <= labels => base,
        r if r <= labels * 2.0 => 2.0 * base,
        _ => (range / labels).min(5.0 * base),
    }
}

/// A row-aligned tick grid over a price range.
///
/// Construction snaps the "nice" grid step up to a multiple of the chart's
/// effective tick size (`row_step`), so every yielded value lands on a real
/// price row. Iteration runs in integer atomic units (10^-11), which keeps the
/// grid exact. Returns `None` when no real grid fits the range (callers fall
/// back to a single row label).
struct RowGrid {
    /// Grid step in atomic units, always a positive multiple of `row_step`.
    step: i64,
    /// Bottom of the range in atomic units.
    low: i64,
    /// Top of the range in atomic units.
    high: i64,
    /// `high - low`, always positive.
    range: i64,
}

impl RowGrid {
    fn new(lowest: f64, highest: f64, row_step: PriceStep, labels_can_fit: i32) -> Option<Self> {
        let raw_units = Price::from_f64(calc_optimal_ticks(highest, lowest, labels_can_fit))
            .units
            .max(1);
        let row = row_step.units.max(1);

        // Round the "nice" step up to a multiple of the row so every label
        // lands on a real row. Since `step >= row >= 1`, the product below can
        // never overflow.
        let step = {
            let mut step = (raw_units / row) * row;
            if step < raw_units {
                step += row;
            }
            step
        };

        let low = Price::from_f64(lowest).units;
        let high = Price::from_f64(highest).units;

        if high <= low || step > high - low {
            return None;
        }

        Some(Self {
            step,
            low,
            high,
            range: high - low,
        })
    }

    /// The topmost grid value: the largest multiple of `step` not above
    /// `high`. With `step >= 1` the product can never overflow.
    fn top(&self) -> i64 {
        self.high.div_euclid(self.step) * self.step
    }

    /// Grid values from the top down to `low` (inclusive).
    fn values(&self) -> impl Iterator<Item = i64> + '_ {
        let step = self.step;
        let low = self.low;
        std::iter::successors(Some(self.top()), move |&v| {
            (v - step >= low).then_some(v - step)
        })
    }
}

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
    /// `row_step` is the chart's effective tick size; when set, labels are
    /// aligned to its rows. `precision` controls how label values are
    /// formatted (the market's min tick); when unset, the row's own precision
    /// is used for row grids and large numbers are abbreviated otherwise.
    pub fn generate(
        &self,
        lowest: f64,
        highest: f64,
        row_step: Option<PriceStep>,
        precision: Option<MinTicksize>,
    ) -> Vec<AxisLabel> {
        if !lowest.is_finite() || !highest.is_finite() || highest - lowest <= 0.0 {
            return Vec::new();
        }

        if self.labels_can_fit <= 1 {
            return match row_step {
                Some(row_step) => {
                    self.single_row(highest, row_step, self.precision(row_step, precision))
                }
                None => self.single_y(highest, None),
            };
        }

        match row_step {
            Some(row_step) => self.row_grid(lowest, highest, row_step, precision),
            None => self.float_grid(lowest, highest),
        }
    }

    /// Resolve the label precision: prefer the caller's `precision`, else the
    /// precision implied by the row step.
    fn precision(&self, row_step: PriceStep, precision: Option<MinTicksize>) -> MinTicksize {
        precision.unwrap_or_else(|| MinTicksize::new(-(row_step.decimal_places() as i8)))
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
    /// formatted at `precision`. Keeps the axis pointing at a real row even
    /// when there is only room for one label.
    fn single_row(
        &self,
        value: f64,
        row_step: PriceStep,
        precision: MinTicksize,
    ) -> Vec<AxisLabel> {
        let row = row_step.units.max(1);
        let aligned = Price::from_f64(value).units.div_euclid(row) * row;
        let content = Price::from_units(aligned).to_string(precision);
        self.single(content)
    }

    /// A grid of labels aligned exactly to the price row grid. Every label
    /// lands on a real row; iteration runs in integer atomic units (10^-11).
    fn row_grid(
        &self,
        lowest: f64,
        highest: f64,
        row_step: PriceStep,
        precision: Option<MinTicksize>,
    ) -> Vec<AxisLabel> {
        let precision = self.precision(row_step, precision);

        let Some(grid) = RowGrid::new(lowest, highest, row_step, self.labels_can_fit) else {
            return self.single_row(highest, row_step, precision);
        };

        let mut labels = Vec::with_capacity((self.labels_can_fit + 2) as usize);
        for value in grid.values().take(MAX_ITERATIONS) {
            let content = Price::from_units(value).to_string(precision);

            let ratio = (value - grid.low) as f64 / grid.range as f64;
            let label_pos = self.bounds.height - ratio as f32 * self.bounds.height;

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
    /// have no row grid). Labels use abbreviated large-number formatting.
    fn float_grid(&self, lowest: f64, highest: f64) -> Vec<AxisLabel> {
        let step = calc_optimal_ticks(highest, lowest, self.labels_can_fit);
        let max = (highest / step).ceil() * step;

        if step > highest - lowest {
            return self.single_y(highest, None);
        }

        let mut value = max;
        while value > highest {
            value -= step;
        }

        let mut labels = Vec::with_capacity((self.labels_can_fit + 2) as usize);
        let mut safety_counter = 0;

        while value >= lowest && safety_counter < MAX_ITERATIONS {
            if value <= highest + step * 0.5 && value >= lowest - step * 0.5 {
                let content = abbr_large_numbers(value);

                let clamped_value = value.max(lowest).min(highest);
                let ratio = (clamped_value - lowest) / (highest - lowest);
                let label_pos = self.bounds.height - ratio as f32 * self.bounds.height;

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

            value -= step;
            safety_counter += 1;
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
