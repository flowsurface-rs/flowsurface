//! X- and Y-axis tick generation, shared across all charts.
//!
//! The module is split by axis:
//!
//! * [`x`] — the time-axis grid: candidate step tables per timeframe
//!   ([`TimeSteps`]) and the concrete grid over a visible range
//!   ([`TimeAxisGrid`], including sub-daily ticks and day/month/year calendar
//!   boundaries).
//! * [`y`] — the value scales: row-aligned price grids ([`RowGrid`]), plain
//!   float grids ([`FloatGrid`]) and the unified [`YAxisScale`] entry point
//!   that yields tick lists ([`YTicks`]).
//!
//! Everything is exported under its axis: `ticks::x::TimeAxisGrid` for the
//! time axis and `ticks::y::YAxisScale` for the value axis.

/// Safety cap on the number of grid lines produced by any strategy.
pub const MAX_GRID_LINES: usize = 1000;

pub mod x;
pub mod y;
