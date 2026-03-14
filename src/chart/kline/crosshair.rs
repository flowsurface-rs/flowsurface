use super::*;

/// Formats a duration given in milliseconds as a compact human-readable string.
/// Examples: 500 → "500ms", 45_234 → "45.234s", 63_555 → "1m 3.555s", 3_661_000 → "1h 1m 1s"
pub(super) fn format_duration_ms(ms: u64) -> String {
    if ms >= 3_600_000 {
        let h = ms / 3_600_000;
        let rem = ms % 3_600_000;
        let m = rem / 60_000;
        let s = rem % 60_000 / 1_000;
        if m == 0 && s == 0 {
            format!("{h}h")
        } else if s == 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h {m}m {s}s")
        }
    } else if ms >= 60_000 {
        let m = ms / 60_000;
        let rem_ms = ms % 60_000;
        if rem_ms == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {:.3}s", rem_ms as f64 / 1000.0)
        }
    } else if ms >= 1_000 {
        format!("{:.3}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

pub(super) fn draw_crosshair_tooltip(
    data: &PlotData<KlineDataPoint>,
    ticker_info: &TickerInfo,
    frame: &mut canvas::Frame,
    palette: &Extended,
    at_interval: u64,
    basis: Basis,
    timezone: data::UserTimezone,
    forming_kline: Option<&Kline>,
) {
    // Resolve both the kline and (for tick-based) the agg_trade_id_range
    let (kline_opt, agg_id_range): (Option<&Kline>, Option<(u64, u64)>) = match data {
        PlotData::TimeBased(timeseries) => {
            let kline = timeseries
                .datapoints
                .iter()
                .find(|(time, _)| **time == at_interval)
                .map(|(_, dp)| &dp.kline)
                .or_else(|| {
                    if timeseries.datapoints.is_empty() {
                        None
                    } else {
                        let (last_time, dp) = timeseries.datapoints.last_key_value()?;
                        if at_interval > *last_time {
                            Some(&dp.kline)
                        } else {
                            None
                        }
                    }
                });
            (kline, None)
        }
        PlotData::TickBased(tick_aggr) => {
            if at_interval == u64::MAX {
                log::trace!(
                    "[TOOLTIP] forming bar sentinel detected, forming_kline={}",
                    forming_kline.is_some()
                );
                // Forming bar: use last completed bar's agg_trade_id_range
                let ids = tick_aggr
                    .datapoints
                    .last()
                    .and_then(|dp| dp.agg_trade_id_range);
                (forming_kline, ids)
            } else {
                let index = (at_interval / u64::from(tick_aggr.interval.0)) as usize;
                if index < tick_aggr.datapoints.len() {
                    let dp = &tick_aggr.datapoints[tick_aggr.datapoints.len() - 1 - index];
                    (Some(&dp.kline), dp.agg_trade_id_range)
                } else {
                    (None, None)
                }
            }
        }
    };

    if let Some(kline) = kline_opt {
        let change_pct = ((kline.close - kline.open).to_f32() / kline.open.to_f32()) * 100.0;
        let change_color = if change_pct >= 0.0 {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };

        let base_color = palette.background.base.text;
        let dim_color = base_color.scale_alpha(0.65);
        let precision = ticker_info.min_ticksize;

        let pct_str = format!("{change_pct:+.2}%");
        let open_str = kline.open.to_string(precision);
        let high_str = kline.high.to_string(precision);
        let low_str = kline.low.to_string(precision);
        let close_str = kline.close.to_string(precision);

        let segments: &[(&str, iced::Color, bool)] = &[
            ("O", base_color, false),
            (&open_str, change_color, true),
            ("H", base_color, false),
            (&high_str, change_color, true),
            ("L", base_color, false),
            (&low_str, change_color, true),
            ("C", base_color, false),
            (&close_str, change_color, true),
            (&pct_str, change_color, true),
        ];

        let ohlc_width: f32 = segments
            .iter()
            .map(|(s, _, is_val)| s.len() as f32 * 10.0 + if *is_val { 8.0 } else { 3.0 })
            .sum();

        // Timing rows: open time, close time, duration — only for index-based bases.
        // Shows both UTC and Local so the user always sees both at a glance.
        let timing_lines: Option<(String, String)> = match (basis, data) {
            (Basis::Odb(_) | Basis::Tick(_), PlotData::TickBased(tick_aggr)) => {
                let (open_ms, close_ms) = if at_interval == u64::MAX {
                    // Forming bar: open = last completed bar's close_time, close = forming kline's time
                    let open = tick_aggr.datapoints.last().map(|dp| dp.kline.time as i64);
                    let close = kline.time as i64;
                    log::trace!(
                        "[TOOLTIP] forming timing: open_ms={:?} close_ms={}",
                        open,
                        close
                    );
                    (open, close)
                } else {
                    let index = (at_interval / u64::from(tick_aggr.interval.0)) as usize;
                    let fwd = tick_aggr.datapoints.len().saturating_sub(1 + index);
                    let close = kline.time as i64;
                    // Open time: use open_time_ms from ClickHouse if available (correct for ODB
                    // bars — prev_bar.close_time ≠ this_bar.open_time due to gap between the
                    // trigger trade and the first trade of the new bar).
                    // Fallback: previous bar's close time (correct for Tick basis).
                    let open = tick_aggr
                        .datapoints
                        .get(fwd)
                        .and_then(|dp| dp.open_time_ms)
                        .map(|ms| ms as i64)
                        .or_else(|| {
                            (fwd > 0)
                                .then(|| tick_aggr.datapoints[fwd - 1].kline.time as i64)
                        });
                    (open, close)
                };

                let alt_tz = match timezone {
                    data::UserTimezone::Utc => data::UserTimezone::Local,
                    data::UserTimezone::Local => data::UserTimezone::Utc,
                };

                let dur_fmt = open_ms
                    .map(|open| format_duration_ms(close_ms.saturating_sub(open).max(0) as u64))
                    .unwrap_or_else(|| "—".into());

                let fmt_row = |tz: data::UserTimezone| {
                    let close_fmt = tz.format_bar_time_ms(close_ms).unwrap_or_default();
                    let open_fmt = open_ms
                        .and_then(|ms| tz.format_bar_time_ms(ms))
                        .unwrap_or_else(|| "—".into());
                    format!("{open_fmt}  →  {close_fmt}   ({dur_fmt})  {tz}")
                };

                Some((fmt_row(timezone), fmt_row(alt_tz)))
            }
            _ => None,
        };

        // Row 4: agg_trade_id range (ODB bars only)
        let agg_id_line: Option<String> = agg_id_range.map(|(first, last)| {
            let span = last.saturating_sub(first) + 1;
            format!("ID {first}  →  {last}   (n={span})")
        });

        let timing_width = timing_lines
            .as_ref()
            .map(|(a, b)| {
                let wa = a.len() as f32 * 9.0 + 16.0;
                let wb = b.len() as f32 * 9.0 + 16.0;
                wa.max(wb)
            })
            .unwrap_or(0.0);
        let agg_id_width = agg_id_line
            .as_ref()
            .map(|s| s.len() as f32 * 9.0 + 16.0)
            .unwrap_or(0.0);
        let bg_width = ohlc_width.max(timing_width).max(agg_id_width);
        let has_timing = timing_lines.is_some();
        let has_agg_id = agg_id_line.is_some();
        let bg_height = match (has_timing, has_agg_id) {
            (true, true) => 78.0,   // OHLC + 2 timing + agg_id
            (true, false) => 60.0,  // OHLC + 2 timing
            (false, true) => 38.0,  // OHLC + agg_id
            (false, false) => 20.0, // OHLC only
        };

        // Right margin: 72 px reserves space for the intensity heatmap legend
        // (LEGEND_W≈59.5 + PAD=4 + gap=8) so the two widgets never overlap.
        let position = Point::new(
            frame.width() - bg_width - 72.0,
            frame.height() - bg_height - 8.0,
        );

        frame.fill_rectangle(
            position,
            iced::Size::new(bg_width, bg_height),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        // Row 1: O H L C %
        let mut x = position.x;
        for (text, seg_color, is_value) in segments {
            frame.fill_text(canvas::Text {
                content: text.to_string(),
                position: Point::new(x, position.y),
                size: iced::Pixels(15.0),
                color: *seg_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            x += text.len() as f32 * 10.0;
            x += if *is_value { 8.0 } else { 3.0 };
        }

        let mut next_y = position.y + 22.0;

        // Row 2 + 3: open → close (duration) in both timezones
        if let Some((primary, alt)) = timing_lines {
            frame.fill_text(canvas::Text {
                content: primary,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            next_y += 17.0;
            frame.fill_text(canvas::Text {
                content: alt,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            next_y += 17.0;
        }

        // Row 4: agg_trade_id range
        if let Some(id_line) = agg_id_line {
            frame.fill_text(canvas::Text {
                content: id_line,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
        }
    }
}
