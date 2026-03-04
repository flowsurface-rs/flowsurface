// Historical trading session boundary rendering (NY / London / Tokyo).
// Handles both time-based charts (direct timestamp→x) and range bar charts
// (binary search on close timestamps via partition_point).
use data::chart::{Basis, PlotData};
use data::chart::kline::KlineDataPoint;
use data::session::{BoundaryKind, TradingSession, compute_boundaries};

use iced::widget::canvas::{self, LineDash, Path, Stroke};
use iced::{Alignment, Color, Point, Rectangle, Size};

use crate::style::AZERET_MONO;

/// Resolved x-position for a session boundary, ready for rendering.
struct ResolvedBoundary {
    session: TradingSession,
    kind: BoundaryKind,
    x: f32,
    /// Calendar day index (days since epoch) for pairing open/close into strips.
    day_key: u64,
}

/// Draw session boundary lines, strips, and labels onto the chart canvas.
///
/// For `Basis::Time`: `earliest`/`latest` are UTC timestamps, `interval_to_x` maps directly.
/// For `Basis::RangeBar`/`Tick`: `earliest`/`latest` are visual indices (0=newest),
/// and we binary-search `TickAggr::datapoints` to snap session timestamps to bar positions.
pub fn draw_sessions(
    frame: &mut canvas::Frame,
    region: &Rectangle,
    basis: &Basis,
    _cell_width: f32,
    interval_to_x: impl Fn(u64) -> f32,
    data_source: &PlotData<KlineDataPoint>,
    earliest: u64,
    latest: u64,
) {
    let resolved = match basis {
        Basis::Time(_) => {
            log::trace!(
                "[SESSION/render] Time basis: earliest_ts={earliest}, latest_ts={latest}"
            );
            resolve_time_based(earliest, latest, &interval_to_x)
        }
        Basis::Tick(_) | Basis::RangeBar(_) => {
            log::trace!(
                "[SESSION/render] Tick/RangeBar basis: earliest_vis={earliest}, latest_vis={latest}"
            );
            resolve_tick_based(data_source, earliest, latest, &interval_to_x)
        }
    };

    if resolved.is_empty() {
        log::trace!("[SESSION/render] no resolved boundaries — nothing to draw");
        return;
    }

    log::debug!(
        "[SESSION/render] drawing {} boundaries (region: {:.0}x{:.0})",
        resolved.len(),
        region.width,
        region.height,
    );

    // Log a sample of resolved boundaries for diagnostics
    for rb in resolved.iter().take(6) {
        log::trace!(
            "[SESSION/render]   {} {} x={:.1} day_key={}",
            rb.session, rb.kind, rb.x, rb.day_key,
        );
    }
    if resolved.len() > 6 {
        log::trace!("[SESSION/render]   ... and {} more", resolved.len() - 6);
    }

    let strip_height = region.height * 0.03;

    // 1) Draw strips first (colored fills + labels)
    draw_session_strips(frame, region, &resolved, strip_height);

    // 2) Draw vertical dotted lines ON TOP of strips for clear visual connection
    for rb in &resolved {
        let (r, g, b) = rb.session.color_rgb();
        let color = Color::from_rgb8(r, g, b);

        let line_stroke = Stroke::with_color(
            Stroke {
                width: 1.0,
                line_dash: LineDash {
                    segments: &[2.0, 4.0],
                    offset: 0,
                },
                ..Default::default()
            },
            Color { a: 0.7, ..color },
        );

        frame.stroke(
            &Path::line(
                Point::new(rb.x, region.y),
                Point::new(rb.x, region.y + region.height),
            ),
            line_stroke,
        );
    }
}

/// Time-based charts: session timestamps map directly to x via `interval_to_x`.
fn resolve_time_based(
    earliest: u64,
    latest: u64,
    interval_to_x: &impl Fn(u64) -> f32,
) -> Vec<ResolvedBoundary> {
    let boundaries = compute_boundaries(earliest, latest);
    let resolved: Vec<_> = boundaries
        .into_iter()
        .map(|b| {
            let x = interval_to_x(b.timestamp_ms);
            log::trace!(
                "[SESSION/time] {} {} ts={} → x={:.1}",
                b.session, b.kind, b.timestamp_ms, x,
            );
            ResolvedBoundary {
                session: b.session,
                kind: b.kind,
                x,
                day_key: b.timestamp_ms / 86_400_000,
            }
        })
        .collect();
    log::debug!("[SESSION/time] resolved {} boundaries", resolved.len());
    resolved
}

/// Tick/RangeBar charts: binary search on close timestamps to snap to bar positions.
///
/// `partition_point` finds the first bar whose close_time >= session_timestamp,
/// mirroring MQL5's `iBarShift()` for nearest-bar semantics.
fn resolve_tick_based(
    data_source: &PlotData<KlineDataPoint>,
    earliest: u64,
    latest: u64,
    interval_to_x: &impl Fn(u64) -> f32,
) -> Vec<ResolvedBoundary> {
    let PlotData::TickBased(tick_aggr) = data_source else {
        log::warn!("[SESSION/tick] expected TickBased data for RangeBar/Tick basis — got TimeBased");
        return Vec::new();
    };

    let dps = &tick_aggr.datapoints;
    let len = dps.len();
    if len == 0 {
        log::trace!("[SESSION/tick] empty datapoints — nothing to resolve");
        return Vec::new();
    }

    // Narrow to visible time range for performance.
    // earliest = smallest visual idx (right/newest), latest = largest (left/oldest).
    // Convert visual indices → forward storage indices → close timestamps.
    // Forward index = (len-1) - visual_idx (0=oldest in storage).
    let oldest_visible_fwd = (len - 1).saturating_sub(latest as usize);
    let newest_visible_fwd = ((len - 1).saturating_sub(earliest as usize)).min(len - 1);
    let vis_start_ms = dps[oldest_visible_fwd].kline.time;  // older timestamp
    let vis_end_ms = dps[newest_visible_fwd].kline.time;    // newer timestamp

    log::debug!(
        "[SESSION/tick] len={len}, visible: earliest_vis={earliest} latest_vis={latest}, \
         fwd=[{oldest_visible_fwd}..{newest_visible_fwd}], \
         time_range=[{vis_start_ms}, {vis_end_ms}]"
    );

    let boundaries = compute_boundaries(vis_start_ms, vis_end_ms);
    let mut resolved = Vec::with_capacity(boundaries.len());
    let mut skipped_oob = 0u32;
    let mut skipped_vis = 0u32;

    for b in boundaries {
        // Binary search: find first bar that closed at or after this timestamp
        let fwd = dps.partition_point(|dp| dp.kline.time < b.timestamp_ms);

        // Skip if outside loaded data
        if fwd == 0 || fwd >= len {
            skipped_oob += 1;
            log::trace!(
                "[SESSION/tick] {} {} ts={} → fwd={fwd} OUT OF BOUNDS (len={len})",
                b.session, b.kind, b.timestamp_ms,
            );
            continue;
        }

        // Convert forward index to visual index (0 = newest = rightmost)
        let visual_idx = (len - 1) - fwd;

        // Visibility check: earliest/latest are visual indices
        // interval_range returns (earliest=right/newest/smallest_vis, latest=left/oldest/largest_vis)
        // Same semantics as render_data_source: index >= earliest && index <= latest
        let vis = visual_idx as u64;
        if vis < earliest || vis > latest {
            skipped_vis += 1;
            log::trace!(
                "[SESSION/tick] {} {} ts={} → fwd={fwd} vis={vis} NOT VISIBLE [earliest={earliest}..latest={latest}]",
                b.session, b.kind, b.timestamp_ms,
            );
            continue;
        }

        let x = interval_to_x(vis);
        let bar_close_ms = dps[fwd].kline.time;
        log::trace!(
            "[SESSION/tick] {} {} ts={} → fwd={fwd} vis={vis} x={x:.1} (bar_close={bar_close_ms}, delta={}ms)",
            b.session, b.kind, b.timestamp_ms, bar_close_ms as i64 - b.timestamp_ms as i64,
        );

        resolved.push(ResolvedBoundary {
            session: b.session,
            kind: b.kind,
            x,
            day_key: b.timestamp_ms / 86_400_000,
        });
    }

    log::debug!(
        "[SESSION/tick] resolved={}, skipped: oob={skipped_oob} vis={skipped_vis}",
        resolved.len(),
    );

    resolved
}

/// Draw colored strips and labels for paired open/close boundaries.
fn draw_session_strips(
    frame: &mut canvas::Frame,
    region: &Rectangle,
    resolved: &[ResolvedBoundary],
    strip_height: f32,
) {
    // Group by (session, day_key) to pair open + close
    // Simple O(n^2) pairing — boundary count per visible window is small (<50)
    let mut used = vec![false; resolved.len()];
    let mut paired = 0u32;
    let mut unpaired = 0u32;

    for (i, open) in resolved.iter().enumerate() {
        if used[i] || open.kind != BoundaryKind::Open {
            continue;
        }
        used[i] = true;

        // Find matching close for same session + day
        let close_idx = resolved.iter().enumerate().position(|(j, close)| {
            !used[j]
                && close.kind == BoundaryKind::Close
                && close.session == open.session
                && close.day_key == open.day_key
        });

        if let Some(j) = close_idx {
            used[j] = true;
            let close = &resolved[j];

            let (r, g, b) = open.session.color_rgb();
            let color = Color::from_rgb8(r, g, b);

            let x_left = open.x.min(close.x);
            let width = (close.x - open.x).abs();

            log::trace!(
                "[SESSION/strip] {} day={} open_x={:.1} close_x={:.1} width={:.1}",
                open.session, open.day_key, open.x, close.x, width,
            );

            // Colored strip at top of chart.
            // Use Path::rectangle + fill (not fill_rectangle) so the strip
            // goes through the same tessellate_path pipeline as the dotted
            // stroke lines — avoids vertex drift between the two lyon paths.
            frame.fill(
                &Path::rectangle(Point::new(x_left, region.y), Size::new(width, strip_height)),
                Color { a: 0.2, ..color },
            );

            // Session label at left edge of strip (where the session starts)
            if width > 15.0 {
                frame.fill_text(canvas::Text {
                    content: open.session.label().to_string(),
                    position: Point::new(
                        x_left + 3.0,
                        region.y + strip_height / 2.0,
                    ),
                    size: iced::Pixels(9.0),
                    color: Color { a: 0.8, ..color },
                    align_x: Alignment::Start.into(),
                    align_y: Alignment::Center.into(),
                    font: AZERET_MONO,
                    ..canvas::Text::default()
                });
            }

            paired += 1;
        } else {
            unpaired += 1;
            log::trace!(
                "[SESSION/strip] {} day={} UNPAIRED (open at x={:.1}, no matching close)",
                open.session, open.day_key, open.x,
            );
        }
    }

    log::debug!("[SESSION/strip] {paired} strips drawn, {unpaired} unpaired opens");
}
