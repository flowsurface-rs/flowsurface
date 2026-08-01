use super::MAX_GRID_LINES;
use exchange::unit::{MinTicksize, Price, PriceStep};

/// A concrete grid over a visible range, unified across scale kinds.
///
/// Two concrete grids exist: a row-aligned price grid (exact atomic units)
/// and a plain float grid (continuous values). [`Grid`] exposes the interface
/// they share ([`Grid::step`], [`Grid::ticks`], [`Grid::ratio_of`]) and is
/// what [`YAxisScale`] hands to [`YTicks::from_grid`] to produce tick lists.
#[derive(Debug, Clone)]
pub enum Grid {
    /// Row-aligned price grid.
    Row(RowGrid),
    /// Plain float grid.
    Float(FloatGrid),
}

impl Grid {
    /// A row-aligned price grid over a range ("nice" step snapped to rows).
    /// Returns `None` when no grid fits the range.
    pub fn row(
        lowest: f64,
        highest: f64,
        row_step: PriceStep,
        labels_can_fit: i32,
    ) -> Option<Self> {
        RowGrid::new(lowest, highest, row_step, labels_can_fit).map(Grid::Row)
    }

    /// A dense price grid at exactly the row step (fallback for charts that
    /// always want to show rows).
    pub fn rows(lowest: f64, highest: f64, row_step: PriceStep) -> Option<Self> {
        RowGrid::from_row(lowest, highest, row_step).map(Grid::Row)
    }

    /// A plain float grid over a continuous range.
    pub fn float(lowest: f64, highest: f64, labels_can_fit: i32) -> Self {
        Grid::Float(FloatGrid::new(lowest, highest, labels_can_fit))
    }

    /// The grid step in the scale's natural representation: atomic units for
    /// [`Grid::Row`], continuous value for [`Grid::Float`].
    pub fn step(&self) -> TickValue {
        match self {
            Grid::Row(g) => TickValue::PriceUnits(g.step()),
            Grid::Float(g) => TickValue::Float(g.step),
        }
    }

    /// Every grid line as `(value, normalized position)`, ascending by value
    /// (`0.0` = bottom of the range, `1.0` = top).
    pub fn ticks(&self) -> Vec<(TickValue, f64)> {
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

    /// Normalized position of a grid value within the range (`0.0` = bottom,
    /// `1.0` = top), or `0.0` when the value is not on the grid.
    pub fn ratio_of(&self, value: TickValue) -> f64 {
        match (self, value) {
            (Grid::Row(g), TickValue::PriceUnits(units)) => g.ratio_of(units),
            (Grid::Float(g), TickValue::Float(v)) => g
                .ticks
                .iter()
                .find(|(tv, _)| (*tv - v).abs() < 1e-9)
                .map(|(_, r)| *r)
                .unwrap_or(0.0),
            _ => 0.0,
        }
    }
}

/// A row-aligned tick grid over a price range.
///
/// Construction snaps the "nice" grid step up to a multiple of the chart's
/// effective tick size (`row_step`), so every yielded value lands on a real
/// price row. All grid arithmetic is widened to `i128` so extreme (saturating)
/// price magnitudes can never overflow or underflow `i64`. Returns `None` when
/// no real grid fits the range (callers fall back to a single row label).
#[derive(Debug, Clone)]
pub struct RowGrid {
    /// Grid step in atomic units, always a positive multiple of `row_step`.
    step: i64,
    /// Bottom of the range in atomic units.
    low: i64,
    /// Top of the range in atomic units (kept in `i128` for exact extremes).
    high: i128,
    /// `high - low` in atomic units, always positive.
    range: i128,
}

impl RowGrid {
    pub fn new(
        lowest: f64,
        highest: f64,
        row_step: PriceStep,
        labels_can_fit: i32,
    ) -> Option<Self> {
        let raw_units = Price::from_f64(FloatGrid::nice_step(lowest, highest, labels_can_fit))
            .units
            .max(1);
        let row = row_step.units.max(1);

        let low = Price::from_f64(lowest).units;
        let high = Price::from_f64(highest).units;

        if high <= low {
            return None;
        }

        // Widen to i128: `high - low` can overflow i64 when both ends are
        // saturating (extreme) prices.
        let high = i128::from(high);
        let low = i128::from(low);

        // Round the "nice" step up to a multiple of the row so every label
        // lands on a real row. Since `step >= row >= 1`, the product below can
        // never overflow (in i128).
        let raw = i128::from(raw_units);
        let row = i128::from(row);
        let mut step = (raw / row) * row;
        if step < raw {
            step += row;
        }

        Self::from_units(low, high, step)
    }

    /// A grid whose step is exactly `row_step`. Used as a fallback when no
    /// coarser "nice" grid fits the range (e.g. the visible range is narrower
    /// than one [`FloatGrid::nice_step`]). Returns `None` when the range is empty.
    pub fn from_row(lowest: f64, highest: f64, row_step: PriceStep) -> Option<Self> {
        let row = row_step.units.max(1);
        let low = Price::from_f64(lowest).units;
        let high = Price::from_f64(highest).units;
        if high <= low {
            return None;
        }
        Self::from_units(i128::from(low), i128::from(high), i128::from(row))
    }

    /// Build a grid from widened bounds and an `i128` step, narrowing the step
    /// back to `i64`. Returns `None` when no grid can fit the range.
    fn from_units(low: i128, high: i128, step: i128) -> Option<Self> {
        let range = high - low;

        // No grid can fit a range narrower than a single step.
        if step > range {
            return None;
        }

        // Narrow back to i64. Whenever the range itself fits in i64 this is
        // guaranteed to succeed; an i128-only range (absurd inputs) bails out.
        let Ok(step) = i64::try_from(step) else {
            return None;
        };

        Some(Self {
            step,
            low: low as i64,
            high,
            range,
        })
    }

    /// The grid step in atomic units (always a positive multiple of `row_step`).
    pub fn step(&self) -> i64 {
        self.step
    }

    /// The topmost grid value: the largest multiple of `step` not above
    /// `high`, computed in `i128` to stay exact at the i64 extremes.
    fn top(&self) -> i128 {
        let step = i128::from(self.step);
        self.high.div_euclid(step) * step
    }

    /// Grid values from the top down to `low` (inclusive), in atomic units.
    ///
    /// Always terminates: values strictly decrease by `step >= 1`, stop at
    /// `low`, and the total count is capped at [`MAX_GRID_LINES`].
    pub fn values(&self) -> impl Iterator<Item = i64> + '_ {
        let step = i128::from(self.step);
        let low = self.low as i128;
        std::iter::successors(Some(self.top()), move |&v| {
            (v - step >= low).then_some(v - step)
        })
        .map(|v| v as i64)
        .take(MAX_GRID_LINES)
    }

    /// Normalized position of a grid value within `[low, high]`, where `0.0`
    /// is the bottom of the range and `1.0` the top. Computed in `i128`.
    pub fn ratio_of(&self, units: i64) -> f64 {
        (i128::from(units) - self.low as i128) as f64 / self.range as f64
    }

    /// Align `value` down to the nearest multiple of `row_step` (floor), in
    /// atomic units. Never overflows: `div_euclid` floors and the product's
    /// magnitude cannot exceed the input's.
    pub fn floor_to_row(value: f64, row_step: PriceStep) -> i64 {
        let row = row_step.units.max(1);
        let units = Price::from_f64(value).units;
        units.div_euclid(row) * row
    }
}

/// A plain f64 grid over a range with no row alignment (indicator axes,
/// non-price scales such as percent change).
///
/// [`FloatGrid::step`] is the selected "nice" step and [`FloatGrid::ticks`]
/// holds `(value, ratio)` pairs, where `ratio` is the normalized position
/// within `[lowest, highest]` (`0.0` = bottom, `1.0` = top). Edge ticks may
/// extend up to half a step outside the range. The tick list is empty when the
/// range is too narrow for even a single step (callers fall back to a single
/// label). Terminates within [`MAX_GRID_LINES`] iterations.
#[derive(Debug, Clone)]
pub struct FloatGrid {
    /// The selected grid step.
    pub step: f64,
    /// `(value, ratio)` pairs, ascending by value.
    pub ticks: Vec<(f64, f64)>,
}

impl FloatGrid {
    /// Compute a plain f64 grid (no row alignment) for a range.
    ///
    /// Universal across charts: used for indicator axes and for non-price scales
    /// (e.g. the comparison chart's percent domain), where there is no price row
    /// to align to.
    pub fn new(lowest: f64, highest: f64, labels_can_fit: i32) -> Self {
        let step = Self::nice_step(lowest, highest, labels_can_fit);
        let span = highest - lowest;

        if !step.is_finite() || step <= 0.0 || span <= 0.0 || step > span {
            return FloatGrid {
                step,
                ticks: Vec::new(),
            };
        }

        // `max` is at most one step above `highest`, but the multiplication
        // overflows to infinity when `highest` sits within a step of f64::MAX;
        // in that case the walk below could never terminate, so bail out with
        // an empty grid.
        let max = (highest / step).ceil() * step;
        if !max.is_finite() {
            return FloatGrid {
                step,
                ticks: Vec::new(),
            };
        }
        let mut value = max;
        while value > highest {
            value -= step;
        }

        let mut ticks = Vec::with_capacity((labels_can_fit.max(1) + 2) as usize);
        let mut safety_counter = 0;
        while value >= lowest && safety_counter < MAX_GRID_LINES {
            if value <= highest + step * 0.5 && value >= lowest - step * 0.5 {
                let clamped_value = value.max(lowest).min(highest);
                let ratio = (clamped_value - lowest) / span;
                ticks.push((value, ratio));
            }
            value -= step;
            safety_counter += 1;
        }

        // The loop walks top-down; store ascending to match the documented
        // `ticks` order (bottom-first).
        ticks.reverse();

        FloatGrid { step, ticks }
    }

    /// A "nice" grid step for a numeric range that keeps ~`labels_can_fit` lines.
    ///
    /// The range is floored at the smallest positive f64 rather than f32::EPSILON:
    /// the machine epsilon (~1.19e-7) is a huge absolute price span for sub-penny
    /// markets and would force a far-too-coarse grid (or no grid at all).
    pub fn nice_step(lowest: f64, highest: f64, labels_can_fit: i32) -> f64 {
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

    /// Round a positive target up to the nearest "nice" `1/2/5 * 10^k` value.
    ///
    /// This is the generic "nice number" rounder (e.g. for bucket steps and
    /// target-derived steps); it is distinct from [`Self::nice_step`], which
    /// sizes a grid to a label budget over a range.
    pub fn round_125(v: f64) -> f64 {
        if !v.is_finite() || v <= 0.0 {
            return 1.0;
        }

        let base = 10.0f64.powf(v.log10().floor());
        let fraction = v / base;
        let mult = if fraction <= 1.0 {
            1.0
        } else if fraction <= 2.0 {
            2.0
        } else if fraction <= 5.0 {
            5.0
        } else {
            10.0
        };
        mult * base
    }
}

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
    /// Logarithmic scale over a positive domain: ticks at `1/2/5 * 10^k`
    /// values; positions are logarithmic (non-equal screen distance per unit).
    Log,
}

/// A tick value in the scale's natural representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickValue {
    /// Exact atomic price units (for [`YAxisScale::Price`]).
    PriceUnits(i64),
    /// Continuous float value (for [`YAxisScale::Linear`], [`YAxisScale::Percent`]
    /// and [`YAxisScale::Log`]).
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
    /// [`YAxisScale::Price`] and [`YAxisScale::Log`].
    pub step: Option<f64>,
    /// Ticks ascending by value (bottom-first).
    pub ticks: Vec<YTick>,
}

impl YTicks {
    /// Convert any [`Grid`] into ascending ticks.
    pub fn from_grid(grid: Grid) -> YTicks {
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

    /// Row-aligned price ticks over a range, ascending by value.
    ///
    /// Every value is an exact atomic price unit on the row grid; positions
    /// are linear in price.
    pub fn price(lowest: f64, highest: f64, row_step: PriceStep, labels_can_fit: i32) -> YTicks {
        Grid::row(lowest, highest, row_step, labels_can_fit)
            .map(YTicks::from_grid)
            .unwrap_or_default()
    }

    /// Plain linear ticks over a continuous domain (indicator values, percent
    /// change, ...).
    pub fn linear(lowest: f64, highest: f64, labels_can_fit: i32) -> YTicks {
        YTicks::from_grid(Grid::float(lowest, highest, labels_can_fit))
    }

    /// Logarithmic ticks over a positive domain: values at `1/2/5 * 10^k`
    /// inside the range, thinned to the label budget, positioned
    /// logarithmically.
    pub fn log(lowest: f64, highest: f64, labels_can_fit: i32) -> YTicks {
        let labels = labels_can_fit.max(1) as usize;
        if !lowest.is_finite()
            || !highest.is_finite()
            || lowest <= 0.0
            || highest <= 0.0
            || highest <= lowest
        {
            return YTicks::default();
        }

        // Candidate values: m * 10^d for m in {1, 2, 5} within [lowest, highest].
        let mut values: Vec<f64> = Vec::new();
        let d_min = lowest.log10().floor() as i32;
        let d_max = highest.log10().ceil() as i32;
        for d in d_min..=d_max {
            for m in [1.0, 2.0, 5.0] {
                let v = m * 10f64.powi(d);
                if v >= lowest && v <= highest {
                    values.push(v);
                }
            }
        }

        // Thin to the label budget: keep 1&5 mantissas, then powers of ten only.
        let mantissa = |v: f64| -> f64 {
            let d = v.log10().floor() as i32;
            v / 10f64.powi(d)
        };
        if values.len() > labels {
            values.retain(|v| {
                let m = mantissa(*v);
                (m - 1.0).abs() < 1e-9 || (m - 5.0).abs() < 1e-9
            });
        }
        if values.len() > labels {
            values.retain(|v| (mantissa(*v) - 1.0).abs() < 1e-9);
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        YTicks {
            step: None,
            ticks: values
                .into_iter()
                .map(|v| YTick {
                    value: TickValue::Float(v),
                    ratio: YAxisScale::Log.ratio(v, lowest, highest),
                })
                .collect(),
        }
    }
}

impl YAxisScale {
    /// Generate the ticks for a visible range. Returns an empty list when the
    /// range is empty or no grid fits (callers decide their fallback).
    pub fn ticks(&self, lowest: f64, highest: f64, labels_can_fit: i32) -> YTicks {
        match self {
            YAxisScale::Price { row_step } => {
                YTicks::price(lowest, highest, *row_step, labels_can_fit)
            }
            YAxisScale::Linear | YAxisScale::Percent => {
                YTicks::linear(lowest, highest, labels_can_fit)
            }
            YAxisScale::Log => YTicks::log(lowest, highest, labels_can_fit),
        }
    }

    /// A `Price` grid at exactly the row step: a dense fallback used when no
    /// coarser grid fits (e.g. a heatmap that always wants to show rows).
    /// Returns an empty list for non-`Price` scales or an empty range.
    pub fn ticks_from_row(&self, lowest: f64, highest: f64) -> YTicks {
        let YAxisScale::Price { row_step } = self else {
            return YTicks::default();
        };
        Grid::rows(lowest, highest, *row_step)
            .map(YTicks::from_grid)
            .unwrap_or_default()
    }

    /// Map a value to its normalized position within `[lowest, highest]`
    /// (`0.0` = bottom, `1.0` = top), using the scale's mapping (logarithmic
    /// for [`YAxisScale::Log`]). Used to position crosshairs and arbitrary values
    /// consistently with the ticks.
    pub fn position(&self, value: f64, lowest: f64, highest: f64) -> f64 {
        self.ratio(value, lowest, highest)
    }

    /// Scale-aware value -> normalized position (`0.0` = bottom, `1.0` = top),
    /// `0.0` for invalid ranges. Shared by [`Self::position`] and
    /// [`YTicks::log`].
    fn ratio(&self, value: f64, lowest: f64, highest: f64) -> f64 {
        match self {
            YAxisScale::Price { .. } | YAxisScale::Linear | YAxisScale::Percent => {
                let span = highest - lowest;
                if span <= 0.0 {
                    0.0
                } else {
                    (value - lowest) / span
                }
            }
            YAxisScale::Log => {
                if lowest <= 0.0 || highest <= 0.0 || value <= 0.0 {
                    return 0.0;
                }
                let lo = lowest.log10();
                let hi = highest.log10();
                let span = hi - lo;
                if span <= 0.0 {
                    0.0
                } else {
                    (value.log10() - lo) / span
                }
            }
        }
    }
}

/// The row step and label precision of a price axis, bundled into one value.
///
/// Every price grid in the app (kline, footprint, heatmap) shares this shape:
/// labels are aligned to `row_step` multiples and formatted at `precision`
/// (the market's min tick). Bundling the pair keeps the two consistent and
/// gives the common "ticks + format" idiom a single home.
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

    /// The row-aligned price scale over this axis.
    pub fn scale(&self) -> YAxisScale {
        YAxisScale::Price {
            row_step: self.row_step,
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

    /// Align `value` down to the nearest row multiple, in atomic units.
    pub fn floor_to_row(&self, value: f64) -> i64 {
        RowGrid::floor_to_row(value, self.row_step)
    }

    /// Format a row value (atomic units) at this axis's precision.
    pub fn format_units(&self, units: i64) -> String {
        Price::from_units(units).to_string(self.precision)
    }

    /// Format a price at this axis's precision.
    pub fn format(&self, price: Price) -> String {
        price.to_string(self.precision)
    }
}
