use super::MAX_GRID_LINES;
use crate::util::abbr_large_numbers;
use exchange::unit::{MinTicksize, Price, PriceStep};

/// The family of Y-axis scales.
///
/// A scale owns both how tick *values* are chosen and how values map to
/// normalized positions.
#[derive(Debug, Clone, Copy)]
pub enum YAxisScale {
    /// Row-aligned price scale: grid steps are snapped to multiples of
    /// `row_step` so every label lands on a real price row. Values are exact
    /// atomic units; positions are linear in price.
    Price { row_step: PriceStep },
    /// Plain continuous linear scale (indicator values, etc.). Steps are
    /// "nice"; positions are linear.
    Linear,
    /// Percentage scale (comparison chart). Shares the linear grid with
    /// [`YAxisScale::Linear`] for now, kept distinct so percent-specific step or
    /// formatting policy has a home.
    Percent,
}

/// A concrete grid over a visible range, unified across scale kinds.
///
/// Two concrete grids exist: a row-aligned price grid (exact atomic units)
/// and a plain float grid (continuous values). [`Grid`] exposes the interface
/// they share ([`Grid::ticks`]) and is what [`YAxisScale`] hands to
/// [`YTicks::from_grid`] to produce tick lists.
#[derive(Debug, Clone)]
enum Grid {
    /// Row-aligned price grid.
    Row(RowGrid),
    /// Plain float grid.
    Float(FloatGrid),
}

impl Grid {
    /// Every grid line as `(value, normalized position)`, ascending by value
    /// (`0.0` = bottom of the range, `1.0` = top).
    fn ticks(&self) -> Vec<(TickValue, f64)> {
        match self {
            Grid::Row(g) => {
                let mut out: Vec<_> = g
                    .values()
                    .map(|units| (TickValue::PriceUnits(units), g.ratio_of(units)))
                    .collect();
                // `RowGrid` iterates top-down; standardize on ascending.
                out.reverse();
                out
            }
            Grid::Float(g) => g
                .ticks
                .iter()
                .map(|(v, r)| (TickValue::Float(*v), *r))
                .collect(),
        }
    }
}

/// A row-aligned tick grid over a price range.
#[derive(Debug, Clone)]
struct RowGrid {
    /// Grid step in atomic units, always a positive multiple of `row_step`.
    step: i64,
    /// Bottom of the range in atomic units.
    low: i64,
    /// Top of the range in atomic units.
    high: i64,
    /// `high - low` in atomic units, always positive. Saturates to
    /// `i64::MAX` when the true range exceeds `i64` (only reachable with
    /// extreme, saturating prices); exact for every real range.
    range: i64,
}

impl RowGrid {
    fn new(lowest: f64, highest: f64, row_step: PriceStep, labels_can_fit: i32) -> Option<Self> {
        let row = row_step.units.max(1);
        let low = Price::from_f64(lowest).units;
        let high = Price::from_f64(highest).units;

        if high <= low {
            return None;
        }

        let range_rows = (high as f64 - low as f64) / row as f64;
        let budget = labels_can_fit.max(1) as f64;
        let span_rows = tick_span_min(0.0, range_rows, range_rows / budget, 1.0);
        let step = (span_rows.ceil() as i64).max(1).saturating_mul(row);

        Self::from_units(low, high, step)
    }

    /// A grid whose step is exactly `row_step`. Used as a fallback when no
    /// coarser "nice" grid fits the range (e.g. the visible range is narrower
    /// than one row step). Returns `None` when the range is empty.
    fn from_row(lowest: f64, highest: f64, row_step: PriceStep) -> Option<Self> {
        let row = row_step.units.max(1);
        let low = Price::from_f64(lowest).units;
        let high = Price::from_f64(highest).units;
        if high <= low {
            return None;
        }
        Self::from_units(low, high, row)
    }

    /// Build a grid from raw bounds and an `i64` step. Returns `None` when no
    /// grid line can fit the range.
    fn from_units(low: i64, high: i64, step: i64) -> Option<Self> {
        let range = high.saturating_sub(low);

        // No grid can fit a range narrower than a single step.
        if step > range {
            return None;
        }

        Some(Self {
            step,
            low,
            high,
            range,
        })
    }

    /// The topmost grid value: the largest multiple of `step` not above
    /// `high`. `high - (high % step)` cannot overflow for `high >= 0` (all
    /// real prices); the saturating form keeps pathological inputs from
    /// panicking.
    fn top(&self) -> i64 {
        self.high.saturating_sub(self.high.rem_euclid(self.step))
    }

    /// Grid values from the top down, in atomic units. The topmost value is
    /// the largest multiple of `step` at or below `high`; iteration stops
    /// strictly above `low`.
    ///
    /// Always terminates: values strictly decrease by `step >= 1`, stop at
    /// `low`, and the total count is capped at [`MAX_GRID_LINES`].
    fn values(&self) -> impl Iterator<Item = i64> + '_ {
        std::iter::successors(Some(self.top()), move |&v| {
            v.checked_sub(self.step).filter(|&n| n > self.low)
        })
        .take(MAX_GRID_LINES)
    }

    /// Normalized position of a grid value within `[low, high]`, where `0.0`
    /// is the bottom of the range and `1.0` the top. Exact for every range
    /// that fits in `i64`; the saturating fallback only kicks in past it.
    fn ratio_of(&self, units: i64) -> f64 {
        units.saturating_sub(self.low) as f64 / self.range as f64
    }
}

/// A plain f64 grid over a range with no row alignment (indicator axes,
/// non-price scales such as percent change).
#[derive(Debug, Clone)]
pub struct FloatGrid {
    /// The selected grid step.
    step: f64,
    /// `(value, ratio)` pairs, ascending by value.
    ticks: Vec<(f64, f64)>,
}

impl FloatGrid {
    /// Compute a plain f64 grid (no row alignment) for a range.
    ///
    /// Universal across charts: used for indicator axes and for non-price scales
    /// (e.g. the comparison chart's percent domain), where there is no price row
    /// to align to.
    fn new(lowest: f64, highest: f64, labels_can_fit: i32) -> Self {
        let span = highest - lowest;
        if !span.is_finite() || span <= 0.0 {
            return FloatGrid {
                step: 0.0,
                ticks: Vec::new(),
            };
        }

        let budget = labels_can_fit.max(1) as f64;
        let step = tick_span_min(0.0, span, span / budget, 0.0);

        if !step.is_finite() || step <= 0.0 || step > span {
            return FloatGrid {
                step,
                ticks: Vec::new(),
            };
        }

        let mut value = (highest / step).floor() * step;

        let mut ticks = Vec::with_capacity((budget + 2.0) as usize);
        let mut safety_counter = 0;
        while value > lowest && safety_counter < MAX_GRID_LINES {
            let ratio = (value - lowest) / span;
            ticks.push((value, ratio));
            value -= step;
            safety_counter += 1;
        }

        // The loop walks top-down; store ascending to match the documented
        // `ticks` order (bottom-first).
        ticks.reverse();

        FloatGrid { step, ticks }
    }
}

/// A tick value in the scale's natural representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickValue {
    /// Exact atomic price units (for [`YAxisScale::Price`]).
    PriceUnits(i64),
    /// Continuous float value (for [`YAxisScale::Linear`] and [`YAxisScale::Percent`])
    Float(f64),
}

/// A single Y-axis tick: value plus normalized position.
#[derive(Debug, Clone, Copy)]
pub struct YTick {
    pub value: TickValue,
    /// Normalized position within the visible range: `0.0` = bottom, `1.0` = top.
    pub ratio: f64,
}

/// The result of [`YAxisScale::ticks`].
#[derive(Debug, Clone, Default)]
pub struct YTicks {
    /// The chosen grid step for float scales (`Linear`/`Percent`); `None` for
    /// [`YAxisScale::Price`].
    pub step: Option<f64>,
    /// Ticks ascending by value (bottom-first).
    pub ticks: Vec<YTick>,
}

impl YTicks {
    /// Convert any [`Grid`] into ascending ticks.
    fn from_grid(grid: Grid) -> YTicks {
        let ticks = grid
            .ticks()
            .into_iter()
            .map(|(value, ratio)| YTick { value, ratio })
            .collect();
        let step = match grid {
            Grid::Float(g) => Some(g.step),
            Grid::Row(_) => None,
        };
        YTicks { step, ticks }
    }
}

impl YAxisScale {
    /// Generate the ticks for a visible range. Returns an empty list when the
    /// range is empty or no grid fits (callers decide their fallback).
    pub fn ticks(&self, lowest: f64, highest: f64, labels_can_fit: i32) -> YTicks {
        match self {
            YAxisScale::Price { row_step } => {
                let row = RowGrid::new(lowest, highest, *row_step, labels_can_fit).map(Grid::Row);
                row.map(YTicks::from_grid).unwrap_or_default()
            }
            YAxisScale::Linear | YAxisScale::Percent => {
                let grid = Grid::Float(FloatGrid::new(lowest, highest, labels_can_fit));
                YTicks::from_grid(grid)
            }
        }
    }

    /// A `Price` grid at exactly the row step: a dense fallback used when no
    /// coarser grid fits (e.g. a heatmap that always wants to show rows).
    /// Returns an empty list for non-`Price` scales or an empty range.
    fn ticks_from_row(&self, lowest: f64, highest: f64) -> YTicks {
        let YAxisScale::Price { row_step } = self else {
            return YTicks::default();
        };

        let row = RowGrid::from_row(lowest, highest, *row_step).map(Grid::Row);
        row.map(YTicks::from_grid).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PriceAxis {
    /// Effective tick size the grid rows are aligned to.
    pub row_step: PriceStep,
    /// Precision used to format label values.
    pub precision: MinTicksize,
}

impl PriceAxis {
    /// Bundle a row step with a label precision. `precision` falls back to the
    /// precision implied by `row_step` when not given.
    pub fn new(row_step: PriceStep, precision: Option<MinTicksize>) -> Self {
        Self {
            row_step,
            precision: precision
                .unwrap_or_else(|| MinTicksize::new(-(row_step.decimal_places() as i8))),
        }
    }

    /// Row-aligned ticks over a range; empty when no coarser "nice" grid fits.
    pub fn ticks(&self, lowest: f64, highest: f64, labels_can_fit: i32) -> YTicks {
        self.scale().ticks(lowest, highest, labels_can_fit)
    }

    /// A dense tick grid at exactly the row step (fallback for charts that
    /// always want to show rows).
    pub fn ticks_from_row(&self, lowest: f64, highest: f64) -> YTicks {
        self.scale().ticks_from_row(lowest, highest)
    }

    /// Format a row value (atomic units) at this axis's precision.
    pub fn format_units(&self, units: i64) -> String {
        Price::from_units(units).to_string(self.precision)
    }

    /// Format a price at this axis's precision.
    pub fn format(&self, price: Price) -> String {
        price.to_string(self.precision)
    }

    /// The row-aligned price scale over this axis.
    fn scale(&self) -> YAxisScale {
        YAxisScale::Price {
            row_step: self.row_step,
        }
    }

    /// Convert a canvas y position (pixels) to a price, exact to within one
    /// atomic unit (1e-11). Only the pixel geometry (`y`, `cell_height`) is
    /// f32; the base price and row step are exact atomic-unit values, so the
    /// offset arithmetic runs in f64 and is quantized once at the end.
    pub fn y_to_price(&self, y: f32, cell_height: f32, min: Price) -> Price {
        let ticks = f64::from(y) / f64::from(cell_height);
        let price = min.to_f64() - ticks * self.row_step.to_f64_lossy();
        Price::from_f64(price)
    }

    /// Align `value` down to the nearest row multiple, in atomic units.
    fn floor_to_row(&self, value: f64) -> i64 {
        let row = self.row_step.units.max(1);
        let units = Price::from_f64(value).units;
        units.div_euclid(row) * row
    }
}

/// Computes a "nice" span for a range `[low, high]` (both in the same units):
/// starting from the power of ten at/above the range, it repeatedly divides by
/// the `dividers` pattern while the span still fits the pixel budget
/// (`max_span` — the largest span that yields the target label count) and stays
/// above `min_span`. The final span is always `>= min_span`.
fn tick_span(low: f64, high: f64, max_span: f64, min_span: f64, dividers: [f64; 3]) -> f64 {
    const EPS: f64 = 1e-14;

    // The starting power of ten is clamped so extreme ranges can never turn
    // the walk below into an infinite loop (`10^309` overflows to infinity).
    let mut span = 10f64.powf((high - low).log10().ceil().clamp(0.0, 308.0));
    let mut i = 0;
    loop {
        let c = dividers[i % dividers.len()];
        let above_min = span >= min_span - EPS && span > min_span + EPS;
        let above_budget = span >= max_span * c - EPS;
        if !(above_min && above_budget) {
            break;
        }
        span /= c;
        i += 1;
    }
    span.max(min_span)
}

/// The minimum span over three divider patterns, which yields steps from
/// the `{1, 2, 2.5, 4, 5} * 10^k` family.
fn tick_span_min(low: f64, high: f64, max_span: f64, min_span: f64) -> f64 {
    [
        tick_span(low, high, max_span, min_span, [2.0, 2.5, 2.0]),
        tick_span(low, high, max_span, min_span, [2.0, 2.0, 2.5]),
        tick_span(low, high, max_span, min_span, [2.5, 2.0, 2.0]),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

/// A fully positioned, formatted Y-axis label, ready to be drawn.
///
/// UI-agnostic: carries the vertical position and label text, so callers can
/// map it onto their own rendering primitives.
#[derive(Debug, Clone, PartialEq)]
pub struct YTickLabel {
    /// Vertical position in pixels within the axis height (`0.0` = top).
    pub y_pos: f32,
    /// Rendered label text.
    pub content: String,
}

impl YTickLabel {
    /// Compute the Y-axis labels for a price range.
    ///
    /// `axis` bundles the chart's effective tick size with the label precision;
    /// when set, labels are aligned to its rows and formatted at its precision.
    /// When unset, a plain float grid is used and large numbers are abbreviated.
    /// Positions are pixels from the top of an axis `height` pixels tall.
    pub fn for_range(
        lowest: f64,
        highest: f64,
        height: f32,
        labels_can_fit: i32,
        axis: Option<PriceAxis>,
    ) -> Vec<Self> {
        if !lowest.is_finite() || !highest.is_finite() || highest - lowest <= 0.0 {
            return Vec::new();
        }

        if labels_can_fit <= 1 {
            return match axis {
                Some(axis) => Self::single_row(highest, axis),
                None => Self::single_y(highest),
            };
        }

        match axis {
            Some(axis) => Self::row_grid(lowest, highest, height, labels_can_fit, axis),
            None => Self::float_grid(lowest, highest, height, labels_can_fit),
        }
    }

    /// A single label built from a ready-made content string, at the top of the
    /// axis.
    fn single(content: String) -> Vec<Self> {
        vec![Self {
            y_pos: 0.0,
            content,
        }]
    }

    /// A single fallback label for `value` with abbreviated large-number
    /// formatting.
    fn single_y(value: f64) -> Vec<Self> {
        Self::single(abbr_large_numbers(value))
    }

    /// A single fallback label aligned down to the nearest price row, formatted
    /// at the axis precision. Keeps the axis pointing at a real row even when
    /// there is only room for one label.
    fn single_row(value: f64, axis: PriceAxis) -> Vec<Self> {
        let aligned = axis.floor_to_row(value);
        Self::single(axis.format_units(aligned))
    }

    /// A grid of labels aligned exactly to the price row grid. Every label
    /// lands on a real row; the grid math lives in [`PriceAxis::ticks`].
    fn row_grid(
        lowest: f64,
        highest: f64,
        height: f32,
        labels_can_fit: i32,
        axis: PriceAxis,
    ) -> Vec<Self> {
        let ticks = axis.ticks(lowest, highest, labels_can_fit);
        if ticks.ticks.is_empty() {
            return Self::single_row(highest, axis);
        }

        let mut labels = Vec::with_capacity((labels_can_fit + 2) as usize);
        for tick in ticks.ticks {
            let TickValue::PriceUnits(units) = tick.value else {
                continue;
            };
            labels.push(Self {
                y_pos: height - tick.ratio as f32 * height,
                content: axis.format_units(units),
            });
        }

        labels
    }

    /// A grid of labels from a plain f64 range (used for indicator axes that
    /// have no row grid). Labels use abbreviated large-number formatting. The
    /// grid values themselves are computed by [`YAxisScale::Linear`].
    fn float_grid(lowest: f64, highest: f64, height: f32, labels_can_fit: i32) -> Vec<Self> {
        let ticks = YAxisScale::Linear.ticks(lowest, highest, labels_can_fit);
        if ticks.ticks.is_empty() {
            return Self::single_y(highest);
        }

        let mut labels = Vec::with_capacity((labels_can_fit + 2) as usize);
        for tick in ticks.ticks {
            let TickValue::Float(value) = tick.value else {
                continue;
            };
            labels.push(Self {
                y_pos: height - tick.ratio as f32 * height,
                content: abbr_large_numbers(value),
            });
        }

        labels
    }
}
