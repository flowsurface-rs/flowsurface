use std::ops::Range;

use crate::widget::chart::heatmap::grid::{Abs, Bucket, Rel, SpanEnd, YBin};
use crate::widget::chart::heatmap::lod::BinnedDepthRectKeyAbs;
use crate::widget::chart::heatmap::view::ViewWindow;

#[derive(Debug, Clone, Copy)]
pub struct HeatmapSamplingParams {
    pub heatmap_a: [f32; 4],
    pub heatmap_b: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
struct XDomain {
    start_group: i64,
    end_group_excl: i64,
    start_rel_bucket: Bucket<Rel>,
    end_rel_bucket_excl: Bucket<Rel>,
}

#[derive(Debug, Clone, Copy)]
struct YDomain {
    start: YBin<Rel>,
    end_excl: YBin<Rel>,
    base_abs: YBin<Abs>,
}

/// A typed “cell grid” domain for the heatmap texture.
/// This replaces the public-field bag style with operations.
#[derive(Debug, Clone, Copy)]
pub struct HeatmapGridDomain {
    x: XDomain,
    y: YDomain,
    cols_per_x_bin: i64,
}

impl HeatmapGridDomain {
    #[inline]
    pub fn set_y_range(&mut self, start: YBin<Rel>, end_excl: YBin<Rel>) {
        self.y.start = start;
        self.y.end_excl = end_excl;
    }

    #[inline]
    pub fn width(self) -> u32 {
        (self.x.end_group_excl - self.x.start_group).max(0) as u32
    }

    #[inline]
    pub fn height(self) -> u32 {
        (self.y.end_excl.0 - self.y.start.0).max(0) as u32
    }

    #[inline]
    pub fn sampling_params(self) -> HeatmapSamplingParams {
        let w = self.width().max(1) as f32;
        let h = self.height().max(1) as f32;

        HeatmapSamplingParams {
            heatmap_a: [
                self.x.start_group as f32,
                self.y.start.0 as f32,
                self.cols_per_x_bin.max(1) as f32,
                0.0,
            ],
            heatmap_b: [w, h, 1.0 / w, 1.0 / h],
        }
    }

    #[inline]
    pub fn x_group_start(self) -> i64 {
        self.x.start_group
    }

    #[inline]
    pub fn x_group_end_excl(self) -> i64 {
        self.x.end_group_excl
    }

    #[inline]
    pub fn cols_per_x_bin(self) -> i64 {
        self.cols_per_x_bin.max(1)
    }

    #[inline]
    fn y_index_for_abs_y_bin(self, abs_y_bin: YBin<Abs>) -> Option<usize> {
        let y_rel = abs_y_bin.to_rel(self.y.base_abs).0;
        if y_rel < self.y.start.0 || y_rel >= self.y.end_excl.0 {
            return None;
        }
        Some((y_rel - self.y.start.0) as usize)
    }

    #[inline]
    fn x_group_range_for_span(
        self,
        start_rel: Bucket<Rel>,
        end_rel_excl: Bucket<Rel>,
    ) -> Option<Range<i64>> {
        let cols = self.cols_per_x_bin.max(1);

        let s_rel = start_rel.0.max(self.x.start_rel_bucket.0);
        let e_rel_excl = end_rel_excl.0.min(self.x.end_rel_bucket_excl.0);

        if e_rel_excl <= s_rel {
            return None;
        }

        let g0 = s_rel.div_euclid(cols).max(self.x.start_group);
        let g1 = div_ceil_i64(e_rel_excl, cols).min(self.x.end_group_excl);

        if g1 <= g0 {
            return None;
        }

        Some(g0..g1)
    }

    #[inline]
    fn texel_index(self, x_group: i64, y_idx: usize) -> Option<usize> {
        let w = self.width() as usize;
        if w == 0 {
            return None;
        }
        let x = (x_group - self.x.start_group) as usize;
        if x >= w {
            return None;
        }
        Some(y_idx * w + x)
    }

    #[inline]
    pub fn accumulate_max(
        self,
        key: BinnedDepthRectKeyAbs,
        qty: f32,
        ref_bucket_abs: Bucket<Abs>,
        rg: &mut [[f32; 2]],
        max_depth: &mut f32,
    ) {
        let qty = qty.max(0.0);
        if qty <= 0.0 {
            return;
        }

        let Some(y_idx) = self.y_index_for_abs_y_bin(key.abs_y_bin) else {
            return;
        };

        let start_rel = key.x.start.to_rel(ref_bucket_abs);
        let end_rel_excl = match key.x.end_excl {
            SpanEnd::Open => self.x.end_rel_bucket_excl,
            SpanEnd::Closed(end_abs) => end_abs.to_rel(ref_bucket_abs),
        };

        let Some(gr) = self.x_group_range_for_span(start_rel, end_rel_excl) else {
            return;
        };

        for g in gr {
            let Some(idx) = self.texel_index(g, y_idx) else {
                continue;
            };

            if key.is_bid {
                rg[idx][0] = rg[idx][0].max(qty);
                *max_depth = (*max_depth).max(rg[idx][0]);
            } else {
                rg[idx][1] = rg[idx][1].max(qty);
                *max_depth = (*max_depth).max(rg[idx][1]);
            }
        }
    }
}

pub fn spec_from_view(
    w: &ViewWindow,
    ref_bucket_abs: Bucket<Abs>,
    cols_per_x_bin: i64,
    base_abs_y_bin: YBin<Abs>,
) -> Option<HeatmapGridDomain> {
    let cols = cols_per_x_bin.max(1);

    let start_bucket_vis = (w.earliest / w.aggr_time) as i64;
    let end_bucket_vis_excl = ((w.latest_vis / w.aggr_time) as i64) + 2;

    let start_rel = Bucket::<Abs>::abs(start_bucket_vis).to_rel(ref_bucket_abs);
    let end_rel_excl = Bucket::<Abs>::abs(end_bucket_vis_excl).to_rel(ref_bucket_abs);

    let start_group = start_rel.0.div_euclid(cols);
    let end_group_excl = div_ceil_i64(end_rel_excl.0, cols);

    Some(HeatmapGridDomain {
        x: XDomain {
            start_group,
            end_group_excl,
            start_rel_bucket: start_rel,
            end_rel_bucket_excl: end_rel_excl,
        },
        y: YDomain {
            start: YBin::<Rel>::rel(0),
            end_excl: YBin::<Rel>::rel(0),
            base_abs: base_abs_y_bin,
        },
        cols_per_x_bin: cols,
    })
}

#[inline]
fn div_ceil_i64(a: i64, b: i64) -> i64 {
    debug_assert!(b > 0);
    let q = a.div_euclid(b);
    let r = a.rem_euclid(b);
    if r == 0 { q } else { q + 1 }
}
