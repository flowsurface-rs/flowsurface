use data::chart::heatmap::HistoricalDepth;
use exchange::util::{Price, PriceStep};
use std::num::NonZeroI64;

use crate::widget::chart::heatmap::grid::{Abs, Bucket, BucketSpan, SpanEnd, YBin};

#[derive(Debug, Clone, Copy)]
pub struct HeatmapXLodConfig {
    /// Start binning only when a column gets ~this small (px).
    pub enable_col_px: f32,
    /// Stop binning once the column grows beyond this (px).
    pub disable_col_px: f32,
    /// Try to reach about this many pixels per *binned* column.
    pub target_px: f32,
    pub max_cols_per_x_bin: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct HeatmapXLod {
    cols_per_x_bin: NonZeroI64,
}

impl Default for HeatmapXLod {
    fn default() -> Self {
        Self {
            cols_per_x_bin: NonZeroI64::new(1).unwrap(),
        }
    }
}

impl HeatmapXLod {
    #[inline]
    pub fn cols_per_x_bin(self) -> i64 {
        self.cols_per_x_bin.get()
    }

    /// Updates `cols_per_x_bin` using hysteresis + gentle ramping (<=2x step).
    #[inline]
    pub fn update_from_col_px(&mut self, col_px: f32, cfg: HeatmapXLodConfig) {
        if !col_px.is_finite() || col_px <= 0.0 {
            self.cols_per_x_bin = NonZeroI64::new(1).unwrap();
            return;
        }

        let maxc = cfg.max_cols_per_x_bin.max(1);
        let prev = self.cols_per_x_bin.get().clamp(1, maxc);

        // Hysteresis around "no binning"
        if prev == 1 {
            if col_px >= cfg.enable_col_px {
                self.cols_per_x_bin = NonZeroI64::new(1).unwrap();
                return;
            }
        } else if col_px >= cfg.disable_col_px {
            self.cols_per_x_bin = NonZeroI64::new(1).unwrap();
            return;
        }

        // Desired binning to reach ~target px
        let mut desired = (cfg.target_px / col_px).ceil() as i64;
        desired = desired.clamp(1, maxc);

        let lo = (prev / 2).max(1);
        let hi = (prev.saturating_mul(2)).min(maxc);

        let next = desired.clamp(lo, hi);
        self.cols_per_x_bin = NonZeroI64::new(next).unwrap();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BinnedDepthRectKeyAbs {
    pub is_bid: bool,
    pub abs_y_bin: YBin<Abs>,
    pub x: BucketSpan,
}

/// Produces “binned depth rectangles” keyed in **bucket space** (x) and **abs y-bin space** (y).
///
/// - Inputs `earliest/latest/render_bucket_end_excl_ms` are in **ms**.
/// - Outputs `start_x_bin/end_x_bin_excl` are **bucket indices** (time/aggr_time).
pub fn binned_depth_rect_contribs_abs_ybin(
    heatmap: &HistoricalDepth,
    earliest: u64,
    latest: u64,
    render_bucket_end_excl_ms: u64,
    highest: Price,
    lowest: Price,
    step: PriceStep,
    steps_per_y_bin: i64,
    aggr_time: u64,
    latest_time_data: u64,
    latest_bucket: i64,
) -> Vec<(BinnedDepthRectKeyAbs, f32)> {
    if earliest >= latest || aggr_time == 0 {
        return Vec::new();
    }

    let step_units = step.units.max(1);
    let y_div = steps_per_y_bin.max(1);

    let mut pairs: Vec<(BinnedDepthRectKeyAbs, f32)> = Vec::new();

    for (price, runs) in heatmap.iter_time_filtered(earliest, latest, highest, lowest) {
        let abs_steps = price.units / step_units;
        let abs_y_bin = YBin::<Abs>::abs(abs_steps.div_euclid(y_div));

        for (idx, run) in runs.iter().enumerate() {
            let run_start = run.start_time.max(earliest);
            let mut run_until = run.until_time.min(latest);

            let is_open_ended = idx + 1 == runs.len() && run.until_time >= latest_time_data;

            if is_open_ended {
                let extend_to = render_bucket_end_excl_ms.min(latest);
                if extend_to > run_until {
                    run_until = extend_to;
                }
            }

            if run_until <= run_start {
                continue;
            }

            let start_bucket = Bucket::<Abs>::abs((run_start / aggr_time) as i64);

            let end_excl = if is_open_ended {
                SpanEnd::Open
            } else {
                let mut end = div_ceil_u64(run_until, aggr_time) as i64;
                end = end.min(latest_bucket + 1);
                SpanEnd::Closed(Bucket::<Abs>::abs(end))
            };

            if matches!(end_excl, SpanEnd::Closed(b) if b.0 <= start_bucket.0) {
                continue;
            }

            pairs.push((
                BinnedDepthRectKeyAbs {
                    is_bid: run.is_bid,
                    abs_y_bin,
                    x: BucketSpan {
                        start: start_bucket,
                        end_excl,
                    },
                },
                run.qty(),
            ));
        }
    }

    if pairs.is_empty() {
        return pairs;
    }

    pairs.sort_unstable_by(|(ka, _), (kb, _)| ka.cmp(kb));

    let mut combined: Vec<(BinnedDepthRectKeyAbs, f32)> = Vec::with_capacity(pairs.len());
    let mut iter = pairs.into_iter();

    let Some((mut cur_k, mut cur_v)) = iter.next() else {
        return combined;
    };

    for (k, q) in iter {
        if k == cur_k {
            cur_v = cur_v.max(q);
        } else {
            combined.push((cur_k, cur_v));
            cur_k = k;
            cur_v = q;
        }
    }
    combined.push((cur_k, cur_v));
    combined
}

#[inline]
fn div_ceil_u64(a: u64, b: u64) -> u64 {
    if a == 0 {
        0
    } else {
        (a.saturating_add(b - 1)) / b
    }
}
