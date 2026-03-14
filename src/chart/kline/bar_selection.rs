use super::*;

/// Which brim of the selection range is being dragged.
#[derive(Clone, Copy)]
pub(super) enum BrimSide {
    /// Right (newer) brim — the lower visual_idx boundary.
    Lo,
    /// Left (older) brim — the higher visual_idx boundary.
    Hi,
}

/// State for interactive bar-range selection on ODB charts.
/// Shift+Left Click: 1st = set anchor, 2nd = set end, 3rd = reset anchor.
/// Left-drag near a brim: relocates that boundary in real-time.
/// Left-drag anywhere on the stats box: repositions the floating overlay.
#[derive(Default)]
pub(super) struct BarSelectionState {
    /// Visual index of the anchor bar (0 = newest/rightmost).
    pub(super) anchor: Option<usize>,
    /// Visual index of the end bar (set on second Shift+Click).
    pub(super) end: Option<usize>,
    /// Whether the Shift key is currently held (tracked via ModifiersChanged).
    pub(super) shift_held: bool,
    /// Which brim is currently being dragged (None when idle).
    pub(super) dragging_brim: Option<BrimSide>,
    /// Top-left of the stats box background rect. None = default top-centre.
    /// Resets to None on 3rd Shift+Click (selection restart).
    pub(super) stats_box_pos: Option<Point>,
    /// True while the user is dragging the stats box.
    pub(super) dragging_stats_box: bool,
    /// Cursor offset from box origin recorded at drag-start (dx, dy).
    pub(super) stats_drag_offset: (f32, f32),
}

/// Outer width of the stats box background rect (box_w + 2 × padding = 215 + 16).
pub(super) const STATS_BOX_W: f32 = 231.0;
/// Approximate outer height used for hit-testing (exact value varies with line count).
pub(super) const STATS_BOX_H: f32 = 215.0;

/// Converts left and right brim bar positions to screen-space x coordinates.
///
/// `lo` = right brim (newer, lower visual_idx), `hi` = left brim (older).
/// Screen formula: `screen_x = (chart_x + translation.x) * scaling + bounds_width / 2`
pub(super) fn brim_screen_xs(chart: &ViewState, bounds_size: Size, lo: usize, hi: usize) -> (f32, f32) {
    let to_screen = |chart_x: f32| {
        (chart_x + chart.translation.x) * chart.scaling + bounds_size.width / 2.0
    };
    // ODB: interval_to_x(idx) = -(idx as f32) * cell_width
    let right_chart_x = -(lo as f32) * chart.cell_width + chart.cell_width / 2.0;
    let left_chart_x = -(hi as f32) * chart.cell_width - chart.cell_width / 2.0;
    (to_screen(left_chart_x), to_screen(right_chart_x))
}

/// Draws the selection range highlight + brim handles in screen-space.
/// Called in the `crosshair` cache layer so it redraws on drag without
/// invalidating the heavy `klines` (candles) cache.
pub(super) fn draw_selection_highlight(
    frame: &mut canvas::Frame,
    chart: &ViewState,
    bounds_size: Size,
    lo: usize,
    hi: usize,
) {
    let (left_sx, right_sx) = brim_screen_xs(chart, bounds_size, lo, hi);
    let w = (right_sx - left_sx).max(0.0);

    // Very transparent fill so candles remain readable.
    frame.fill_rectangle(
        Point::new(left_sx, 0.0),
        Size::new(w, bounds_size.height),
        iced::Color { r: 1.0, g: 1.0, b: 0.3, a: 0.02 },
    );

    // Brim handle strips — one full bar wide so the clickable zone is obvious.
    let handle_w = (chart.cell_width * chart.scaling).clamp(3.0, 60.0);
    let handle_color = iced::Color { r: 1.0, g: 1.0, b: 0.3, a: 0.22 };
    frame.fill_rectangle(
        Point::new(left_sx, 0.0),
        Size::new(handle_w, bounds_size.height),
        handle_color,
    );
    frame.fill_rectangle(
        Point::new(right_sx - handle_w, 0.0),
        Size::new(handle_w, bounds_size.height),
        handle_color,
    );
}

/// Returns the top-left origin of the stats box background rect.
/// Defaults to top-centre of the frame when `pos` is `None`.
pub(super) fn stats_box_origin(pos: Option<iced::Point>, frame_w: f32) -> iced::Point {
    pos.unwrap_or_else(|| iced::Point::new(frame_w / 2.0 - STATS_BOX_W / 2.0, 6.0))
}

/// Draws bar selection statistics overlay (screen-space, top-center of chart).
/// Called in the `legend` cache layer when both anchor and end are confirmed.
///
/// Metrics computed from raw `trade_intensity` (t/s) in `OdbMicrostructure` —
/// no pre-normalized rolling-window state needed.
///
/// ## Intensity Metrics
/// - **↑t / ↓t**: within-selection rank-normalized mean (0=coldest, 1=hottest in this window).
///   Parenthetical shows raw trades/sec.
/// - **flow (IWDS)**: `Σ(intensity × ±1) / Σ(intensity)` ∈ [-1,+1]. +1 = all urgency on up-bars.
/// - **P(↑>↓)**: Mann-Whitney AUC — P(random up-bar > random dn-bar intensity). 0.5 = no edge.
/// - **log₂(↑/↓)**: log2 ratio of raw means; session baseline cancels; +1 = 2× advantage.
/// - **conv**: mean_t(dominant) / mean_t(minority). >1 = trend has intensity fuel.
/// - **absorp**: mean_t of minority-direction bars — how hard the losing side fought.
/// - **climax**: fraction of top-25%-intensity bars that are up-bars (tail concentration).
///
/// ## Regime Labels
/// BULL/BEAR CONVICTION = IWDS and AUC agree strongly.
/// BULL/BEAR ABSORPTION = direction count and intensity rank disagree (divergence signal).
/// BULL/BEAR CLIMAX ◈   = top-intensity events concentrated ≥78% in one direction.
pub(super) fn draw_bar_selection_stats(
    frame: &mut canvas::Frame,
    palette: &Extended,
    tick_aggr: &data::aggr::ticks::TickAggr,
    anchor: usize,
    end: usize,
    stats_box_pos: Option<iced::Point>,
) {
    let len = tick_aggr.datapoints.len();
    if len == 0 {
        return;
    }
    let (lo, hi) = (anchor.min(end), anchor.max(end));
    let hi = hi.min(len - 1);
    let lo = lo.min(len - 1);
    let distance = hi - lo;

    // ── Collect raw intensity + direction per bar ──────────────────────────
    struct BarSample {
        raw: f32,
        is_up: bool,
    }
    let bars: Vec<BarSample> = (lo..=hi)
        .map(|vi| {
            let si = len - 1 - vi;
            let dp = &tick_aggr.datapoints[si];
            BarSample {
                raw: dp.microstructure.map_or(0.0, |m| m.trade_intensity),
                is_up: dp.kline.close >= dp.kline.open,
            }
        })
        .collect();

    let n = bars.len();
    let n_up = bars.iter().filter(|b| b.is_up).count();
    let n_dn = n - n_up;
    let up_pct = n_up as f32 / n as f32 * 100.0;
    let dn_pct = n_dn as f32 / n as f32 * 100.0;

    // ── Within-selection rank normalisation (shared by ↑t/↓t and AUC) ─────
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| {
        bars[a].raw.partial_cmp(&bars[b].raw).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut rank_norm = vec![0.5_f32; n];
    if n > 1 {
        let mut i = 0;
        while i < n {
            let mut j = i;
            while j + 1 < n && (bars[order[j + 1]].raw - bars[order[i]].raw).abs() < 1e-6 {
                j += 1;
            }
            let avg = (i + j) as f32 * 0.5 / (n - 1) as f32;
            for k in i..=j {
                rank_norm[order[k]] = avg;
            }
            i = j + 1;
        }
    }

    // ── Per-direction aggregates ───────────────────────────────────────────
    let (sum_t_up, sum_raw_up, sum_t_dn, sum_raw_dn) =
        bars.iter().enumerate().fold((0_f32, 0_f32, 0_f32, 0_f32), |(su, ru, sd, rd), (i, b)| {
            if b.is_up { (su + rank_norm[i], ru + b.raw, sd, rd) }
            else       { (su, ru, sd + rank_norm[i], rd + b.raw) }
        });
    let mean_t_up   = if n_up > 0 { sum_t_up   / n_up as f32 } else { f32::NAN };
    let mean_t_dn   = if n_dn > 0 { sum_t_dn   / n_dn as f32 } else { f32::NAN };
    let mean_raw_up = if n_up > 0 { sum_raw_up / n_up as f32 } else { f32::NAN };
    let mean_raw_dn = if n_dn > 0 { sum_raw_dn / n_dn as f32 } else { f32::NAN };

    // ── IWDS ──────────────────────────────────────────────────────────────
    let total_raw: f32 = bars.iter().map(|b| b.raw).sum();
    let iwds = if total_raw > 0.0 {
        bars.iter().map(|b| b.raw * if b.is_up { 1.0 } else { -1.0 }).sum::<f32>() / total_raw
    } else {
        0.0
    };

    // ── Mann-Whitney AUC via rank-sum O(N log N) ───────────────────────────
    let auc: f32 = if n_up > 0 && n_dn > 0 {
        let r_up: f32 = order.iter().enumerate()
            .filter(|(_, orig)| bars[**orig].is_up)
            .map(|(rank_0, _)| rank_0 as f32 + 1.0)
            .sum();
        let u_up = r_up - n_up as f32 * (n_up as f32 + 1.0) / 2.0;
        u_up / (n_up as f32 * n_dn as f32)
    } else {
        f32::NAN
    };

    // ── Log₂ ratio of raw means ────────────────────────────────────────────
    let log2_ratio = if mean_raw_up > 0.0 && mean_raw_dn > 0.0 {
        (mean_raw_up / mean_raw_dn).log2()
    } else {
        f32::NAN
    };

    // ── Conviction and Absorption ──────────────────────────────────────────
    let dominant_up = n_up >= n_dn;
    let conviction = if dominant_up {
        if !mean_t_dn.is_nan() && mean_t_dn > 0.0 { mean_t_up / mean_t_dn } else { f32::NAN }
    } else if !mean_t_up.is_nan() && mean_t_up > 0.0 {
        mean_t_dn / mean_t_up
    } else {
        f32::NAN
    };
    let absorption = if dominant_up { mean_t_dn } else { mean_t_up };

    // ── Climax concentration ───────────────────────────────────────────────
    let (top_n, top_up) = bars.iter().enumerate()
        .filter(|(i, _)| rank_norm[*i] > 0.75)
        .fold((0_usize, 0_usize), |(t, u), (_, b)| (t + 1, if b.is_up { u + 1 } else { u }));
    let climax_up_frac = if top_n > 0 { top_up as f32 / top_n as f32 } else { f32::NAN };

    // ── ASCII intensity bar ────────────────────────────────────────────────
    let fill = ((5.0 + iwds * 5.0).round() as usize).clamp(0, 10);
    let bar_str = format!("[{}{}]", "█".repeat(fill), "░".repeat(10 - fill));

    // ── Regime classification ──────────────────────────────────────────────
    enum Regime { BullConviction, BearConviction, BullAbsorption, BearAbsorption, BullClimax, BearClimax, Contested }
    let regime = if !climax_up_frac.is_nan() && climax_up_frac >= 0.78 {
        Regime::BullClimax
    } else if !climax_up_frac.is_nan() && climax_up_frac <= 0.22 {
        Regime::BearClimax
    } else if iwds > 0.15 && !auc.is_nan() && auc >= 0.60 {
        Regime::BullConviction
    } else if iwds < -0.15 && !auc.is_nan() && auc <= 0.40 {
        Regime::BearConviction
    } else if iwds > 0.15 && !auc.is_nan() && auc < 0.50 {
        Regime::BullAbsorption
    } else if iwds < -0.15 && !auc.is_nan() && auc > 0.50 {
        Regime::BearAbsorption
    } else {
        Regime::Contested
    };

    // ── Colors ────────────────────────────────────────────────────────────
    let amber     = iced::Color { r: 0.85, g: 0.65, b: 0.15, a: 1.00 };
    let amber_dim = iced::Color { r: 0.85, g: 0.65, b: 0.15, a: 0.55 };
    let orange    = iced::Color { r: 0.95, g: 0.55, b: 0.10, a: 1.00 };
    let magenta   = iced::Color { r: 0.90, g: 0.25, b: 0.80, a: 1.00 };
    let dim       = iced::Color { r: 0.50, g: 0.50, b: 0.50, a: 0.65 };
    let dim_white = iced::Color { r: 0.75, g: 0.75, b: 0.75, a: 0.65 };
    let neutral   = palette.background.strong.text;
    let success   = palette.success.base.color;
    let danger    = palette.danger.base.color;

    let (regime_label, regime_color, border_col): (&str, iced::Color, Option<iced::Color>) = match regime {
        Regime::BullConviction => ("BULL CONVICTION", success, Some(success)),
        Regime::BearConviction => ("BEAR CONVICTION", danger,  Some(danger)),
        Regime::BullAbsorption => ("BULL ABSORPTION", orange,  Some(orange)),
        Regime::BearAbsorption => ("BEAR ABSORPTION", orange,  Some(orange)),
        Regime::BullClimax     => ("BULL CLIMAX ◈",   magenta, Some(magenta)),
        Regime::BearClimax     => ("BEAR CLIMAX ◈",   magenta, Some(magenta)),
        Regime::Contested      => ("CONTESTED",       dim,     None),
    };

    // ── Plain-English caption ──────────────────────────────────────────────
    let caption = if !mean_raw_up.is_nan() && !mean_raw_dn.is_nan()
        && mean_raw_dn > 0.0 && mean_raw_up > 0.0
    {
        let (dom, min_raw) = if mean_raw_up >= mean_raw_dn {
            (mean_raw_up, mean_raw_dn)
        } else {
            (mean_raw_dn, mean_raw_up)
        };
        let side = if mean_raw_up >= mean_raw_dn { "buyers" } else { "sellers" };
        format!("{side} {:.1}× more urgent", dom / min_raw)
    } else if n_dn == 0 {
        "all bars bullish — no dn comparison".to_string()
    } else {
        "all bars bearish — no up comparison".to_string()
    };

    // ── Climax line ────────────────────────────────────────────────────────
    let climax_line = if climax_up_frac.is_nan() {
        "◈ climax: — (no top-25% bars yet)".to_string()
    } else {
        let (frac, dir) = if climax_up_frac >= 0.5 {
            (climax_up_frac, "↑")
        } else {
            (1.0 - climax_up_frac, "↓")
        };
        format!("◈ climax: {:.0}% {dir}  (of top-25% bars)", frac * 100.0)
    };
    let climax_color = if border_col.is_some() { orange } else { amber_dim };

    // ── Format helpers ─────────────────────────────────────────────────────
    let t_s   = |v: f32| -> String { if v.is_nan() { "—".to_string() } else { format!("{v:.2}") } };
    let raw_s = |v: f32| -> String { if v.is_nan() { "—".to_string() } else { format!("{v:.0}") } };
    let pct_s = |v: f32| -> String { if v.is_nan() { "—".to_string() } else { format!("{:.0}%", v * 100.0) } };
    let lg2_s = |v: f32| -> String { if v.is_nan() { "—".to_string() } else { format!("{v:+.2}") } };
    let con_s = |v: f32| -> String { if v.is_nan() { "—".to_string() } else { format!("{v:.2}×") } };

    // ── Lines: (text, color, font_size) ───────────────────────────────────
    let ts = 13.0_f32;
    let sm = 11.0_f32;
    let lines: &[(String, iced::Color, f32)] = &[
        (format!("{bar_str}  flow: {:+.2}", iwds),                                    amber,        ts),
        (regime_label.to_string(),                                                    regime_color, ts),
        (caption,                                                                     dim_white,    sm),
        (format!("P(↑>↓): {}   log₂(↑/↓): {}", pct_s(auc), lg2_s(log2_ratio)),     amber_dim,    sm),
        (format!("conv: {}   absorp: {}", con_s(conviction), t_s(absorption)),       amber_dim,    sm),
        (climax_line,                                                                 climax_color, sm),
    ];

    // ── Layout ────────────────────────────────────────────────────────────
    let lh_main = ts + 5.0;
    let lh_sm   = sm + 4.0;
    let box_w   = 255.0_f32;
    let origin  = stats_box_origin(stats_box_pos, frame.width());
    let x = origin.x + 8.0;
    let y = origin.y + 4.0;
    // total_h: header row + separator (3px) + intensity row + remaining lines + padding
    let total_h: f32 = lh_main
        + 3.0
        + lh_sm   // ↑t/↓t combined row
        + lines.iter()
            .map(|(_, _, sz)| if (*sz - ts).abs() < 0.5 { lh_main } else { lh_sm })
            .sum::<f32>()
        + 12.0;

    // Background
    frame.fill_rectangle(
        origin,
        Size::new(box_w + 16.0, total_h),
        iced::Color { r: 0.07, g: 0.07, b: 0.07, a: 0.92 },
    );

    // Colored border when regime has signal
    if let Some(bc) = border_col {
        frame.stroke(
            &Path::rectangle(
                origin,
                Size::new(box_w + 16.0, total_h),
            ),
            Stroke::with_color(
                Stroke { width: 1.5, ..Default::default() },
                iced::Color { a: 0.65, ..bc },
            ),
        );
    }

    // Draw lines
    let mut cur_y = y;

    // ── Header row: "{N} bars  ↑ N (%)  ↓ N (%)" on one line ────────────
    let col_w = box_w / 3.0;
    let header = [
        (format!("{distance} bars"),                neutral),
        (format!("↑ {n_up}  ({up_pct:.0}%)"),       success),
        (format!("↓ {n_dn}  ({dn_pct:.0}%)"),        danger),
    ];
    for (i, (text, color)) in header.iter().enumerate() {
        frame.fill_text(canvas::Text {
            content: text.clone(),
            position: Point::new(x + i as f32 * col_w, cur_y),
            size: iced::Pixels(ts),
            color: *color,
            ..Default::default()
        });
    }
    cur_y += lh_main;

    // Thin separator line replaces the "─────" text row — less vertical waste
    frame.stroke(
        &Path::line(
            Point::new(x - 2.0, cur_y - 2.0),
            Point::new(x + box_w + 2.0, cur_y - 2.0),
        ),
        Stroke::with_color(
            Stroke { width: 0.5, ..Default::default() },
            dim,
        ),
    );
    cur_y += 3.0;

    // ── Intensity row: ↑t/↓t side-by-side, green | red ──────────────────
    let half = box_w / 2.0;
    for (i, (text, color)) in [
        (format!("↑t {}  ↑ {} t/s", t_s(mean_t_up), raw_s(mean_raw_up)), success),
        (format!("↓t {}  ↓ {} t/s", t_s(mean_t_dn), raw_s(mean_raw_dn)), danger),
    ].iter().enumerate() {
        frame.fill_text(canvas::Text {
            content: text.clone(),
            position: Point::new(x + i as f32 * half, cur_y),
            size: iced::Pixels(sm),
            color: *color,
            ..Default::default()
        });
    }
    cur_y += lh_sm;

    for (text, color, sz) in lines {
        frame.fill_text(canvas::Text {
            content: text.clone(),
            position: Point::new(x, cur_y),
            size: iced::Pixels(*sz),
            color: *color,
            ..Default::default()
        });
        cur_y += if (*sz - ts).abs() < 0.5 { lh_main } else { lh_sm };
    }
}
