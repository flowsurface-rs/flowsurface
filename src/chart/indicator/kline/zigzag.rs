// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
//! Streaming ZigZag swing structure overlay for the main candle chart.
//!
//! Draws confirmed pivot lines (solid) and pending pivot line (dashed) directly
//! on the candle pane. No subplot panel, no candle recoloring.

use crate::chart::indicator::kline::KlineIndicatorImpl;
use crate::chart::{Message, ViewState};

use data::chart::kline::KlineDataPoint;
use data::chart::PlotData;
use exchange::unit::Price;
use exchange::{Kline, Trade};

use iced::widget::center;
use iced::Color;
use std::ops::RangeInclusive;

use zigzag::{BarInput, PivotKind, ZigZagConfig, ZigZagState};

/// Per-pivot rendering data stored after processing.
#[derive(Clone, Copy)]
pub struct OverlayPivot {
    /// Storage index (0 = oldest bar).
    pub storage_idx: usize,
    /// Price in atomic units (same scale as `Price::units`).
    pub price_units: i64,
    pub kind: PivotKind,
}

/// ZigZag overlay indicator.
///
/// Processes kline data through the zigzag state machine and stores pivot
/// positions for rendering by the main chart's `draw()` method.
pub struct ZigZagOverlayIndicator {
    state: ZigZagState,
    /// Confirmed pivots in storage order (oldest first).
    pub confirmed_pivots: Vec<OverlayPivot>,
    /// Current pending (unconfirmed) pivot, if any.
    pub pending_pivot: Option<OverlayPivot>,
    /// Number of datapoints processed so far (for incremental updates).
    next_idx: usize,
}

impl ZigZagOverlayIndicator {
    pub fn new() -> Self {
        // Default: reversal_depth=3.0, epsilon=1.0, 250 dbps (0.25%)
        let config = ZigZagConfig::new(3.0, 1.0, 250)
            .expect("default ZigZag config is valid");
        Self {
            state: ZigZagState::new(config),
            confirmed_pivots: Vec::new(),
            pending_pivot: None,
            next_idx: 0,
        }
    }

    fn reset_state(&mut self) {
        let config = self.state.config().clone();
        self.state = ZigZagState::new(config);
        self.confirmed_pivots.clear();
        self.pending_pivot = None;
        self.next_idx = 0;
    }

    /// Recreate the ZigZag state machine if the chart's bar threshold changed.
    fn reconfigure_if_needed(&mut self, dbps: u32) {
        if self.state.config().bar_threshold_dbps != dbps {
            let config = ZigZagConfig::new(3.0, 1.0, dbps)
                .expect("valid dbps from chart config");
            self.state = ZigZagState::new(config);
            self.confirmed_pivots.clear();
            self.pending_pivot = None;
            self.next_idx = 0;
        }
    }

    /// Feed a single bar (by storage index) to the zigzag state machine.
    fn process_one(&mut self, storage_idx: usize, kline: &Kline, timestamp_us: i64) {
        let bar = BarInput {
            index: storage_idx,
            timestamp_us,
            high: kline.high.units,
            low: kline.low.units,
            close: kline.close.units,
            duration_us: None,
        };

        let output = self.state.process_bar(&bar);

        if let Some(pivot) = output.newly_confirmed {
            self.confirmed_pivots.push(OverlayPivot {
                storage_idx: pivot.bar_index,
                price_units: pivot.price,
                kind: pivot.kind,
            });
        }

        // Update pending pivot snapshot.
        self.pending_pivot = self.state.pending().map(|p| OverlayPivot {
            storage_idx: p.bar_index,
            price_units: p.price,
            kind: p.kind,
        });
    }
}

impl KlineIndicatorImpl for ZigZagOverlayIndicator {
    fn clear_all_caches(&mut self) {
        // ZigZag draws on the main chart cache, not its own — nothing to clear here.
    }

    fn clear_crosshair_caches(&mut self) {}

    fn element<'a>(
        &'a self,
        _chart: &'a ViewState,
        _visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        // Overlay has no subplot panel — return empty placeholder.
        center(iced::widget::text("")).into()
    }

    fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>) {
        if let PlotData::TickBased(tickseries) = source {
            let dbps = tickseries.range_bar_threshold_dbps.unwrap_or(250);
            self.reconfigure_if_needed(dbps);
            self.reset_state();
            for (idx, dp) in tickseries.datapoints.iter().enumerate() {
                let timestamp_us = (dp.kline.time as i64) * 1000; // ms → µs
                self.process_one(idx, &dp.kline, timestamp_us);
            }
            self.next_idx = tickseries.datapoints.len();
        }
    }

    fn on_insert_klines(&mut self, _klines: &[Kline]) {}

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        match source {
            PlotData::TimeBased(_) => (),
            PlotData::TickBased(tickseries) => {
                let new_len = tickseries.datapoints.len();
                if self.next_idx == old_dp_len {
                    // Incremental: process only newly completed bars.
                    for idx in old_dp_len..new_len {
                        let dp = &tickseries.datapoints[idx];
                        let timestamp_us = (dp.kline.time as i64) * 1000;
                        self.process_one(idx, &dp.kline, timestamp_us);
                    }
                    self.next_idx = new_len;
                } else {
                    // State mismatch: full rebuild.
                    self.rebuild_from_source(source);
                }
            }
        }
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        if let PlotData::TickBased(tickseries) = source {
            let dbps = tickseries.range_bar_threshold_dbps.unwrap_or(250);
            self.reconfigure_if_needed(dbps);
        }
        self.rebuild_from_source(source);
    }

    fn draw_overlay(
        &self,
        frame: &mut iced::widget::canvas::Frame,
        total_len: usize,
        earliest_visual: usize,
        latest_visual: usize,
        price_to_y: &dyn Fn(Price) -> f32,
        interval_to_x: &dyn Fn(u64) -> f32,
        palette: &iced::theme::palette::Extended,
    ) {
        use iced::widget::canvas::{Path, Stroke, path};

        if self.confirmed_pivots.is_empty() {
            return;
        }

        let storage_to_visual = |storage_idx: usize| -> usize {
            total_len.saturating_sub(1 + storage_idx)
        };

        let pivot_xy = |pivot: &OverlayPivot| -> (f32, f32) {
            let visual_idx = storage_to_visual(pivot.storage_idx);
            let x = interval_to_x(visual_idx as u64);
            let y = price_to_y(Price::from_units(pivot.price_units));
            (x, y)
        };

        // Pivot colour: swing high = danger (red family), swing low = success (green family).
        let pivot_color = |kind: PivotKind| -> Color {
            match kind {
                PivotKind::High => palette.danger.base.color,
                PivotKind::Low => palette.success.base.color,
            }
        };

        // Draw confirmed pivot lines.
        for window in self.confirmed_pivots.windows(2) {
            let a = &window[0];
            let b = &window[1];

            // Cull segments fully outside the visible range.
            let a_vis = storage_to_visual(a.storage_idx);
            let b_vis = storage_to_visual(b.storage_idx);
            let seg_earliest = a_vis.min(b_vis);
            let seg_latest = a_vis.max(b_vis);
            if seg_earliest > latest_visual || seg_latest < earliest_visual {
                continue;
            }

            let (ax, ay) = pivot_xy(a);
            let (bx, by) = pivot_xy(b);

            let line = Path::line(iced::Point::new(ax, ay), iced::Point::new(bx, by));
            let stroke = Stroke {
                width: 1.5,
                style: iced::widget::canvas::stroke::Style::Solid(pivot_color(b.kind)),
                ..Default::default()
            };
            frame.stroke(&line, stroke);
        }

        // Draw pending pivot line (dashed, dimmed).
        if let Some(ref pending) = self.pending_pivot
            && let Some(last_confirmed) = self.confirmed_pivots.last()
        {
            let (ax, ay) = pivot_xy(last_confirmed);
            let (bx, by) = pivot_xy(pending);

            let mut builder = path::Builder::new();
            builder.move_to(iced::Point::new(ax, ay));
            builder.line_to(iced::Point::new(bx, by));
            let dashed_path = builder.build();

            let pending_color = Color {
                a: 0.5,
                ..pivot_color(pending.kind)
            };
            let stroke = Stroke {
                width: 1.0,
                style: iced::widget::canvas::stroke::Style::Solid(pending_color),
                line_dash: iced::widget::canvas::LineDash {
                    segments: &[4.0, 3.0],
                    offset: 0,
                },
                ..Default::default()
            };
            frame.stroke(&dashed_path, stroke);
        }

        // Draw circles at confirmed pivots (visible ones only).
        let circle_radius = 3.0;
        for pivot in &self.confirmed_pivots {
            let vis = storage_to_visual(pivot.storage_idx);
            if vis < earliest_visual || vis > latest_visual {
                continue;
            }
            let (x, y) = pivot_xy(pivot);
            let circle = Path::circle(iced::Point::new(x, y), circle_radius);
            frame.fill(&circle, pivot_color(pivot.kind));
        }

        // Draw pending pivot circle (hollow, dimmed).
        if let Some(ref pending) = self.pending_pivot {
            let vis = storage_to_visual(pending.storage_idx);
            if vis >= earliest_visual && vis <= latest_visual {
                let (x, y) = pivot_xy(pending);
                let circle = Path::circle(iced::Point::new(x, y), circle_radius);
                let pending_color = Color {
                    a: 0.4,
                    ..pivot_color(pending.kind)
                };
                let stroke = Stroke {
                    width: 1.0,
                    style: iced::widget::canvas::stroke::Style::Solid(pending_color),
                    ..Default::default()
                };
                frame.stroke(&circle, stroke);
            }
        }
    }
}
