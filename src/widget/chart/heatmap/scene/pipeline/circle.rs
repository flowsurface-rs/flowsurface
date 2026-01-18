use data::chart::heatmap::GroupedTrade;
use exchange::util::{Price, PriceStep};

use crate::widget::chart::heatmap::{depth_grid::HeatmapPalette, view::ViewWindow};

pub const CIRCLE_VERTICES: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
pub const CIRCLE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CircleInstance {
    pub y_world: f32,
    pub x_bin_rel: i32,
    pub x_frac: f32,
    pub radius_px: f32,
    _pad: f32,
    pub color: [f32; 4],
}

impl CircleInstance {
    const R_MIN_PX: f32 = 2.0;
    const R_MAX_PX: f32 = 25.0;
    const ALPHA: f32 = 0.8;

    pub fn from_trade(
        trade: &GroupedTrade,
        bucket: i64,
        ref_bucket: i64,
        base_price: Price,
        step: PriceStep,
        w: &ViewWindow,
        palette: &HeatmapPalette,
        max_trade_qty: f32,
    ) -> Self {
        let x_bin_rel = (bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let x_frac = 0.0;

        let y_world = Self::y_world_for_price(trade.price, base_price, step, w);

        let q = trade.qty.max(0.0);
        let t = if max_trade_qty > 0.0 {
            (q / max_trade_qty).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let radius_px = Self::R_MIN_PX + t * (Self::R_MAX_PX - Self::R_MIN_PX);

        let rgba = if trade.is_sell {
            [
                palette.sell_rgb[0],
                palette.sell_rgb[1],
                palette.sell_rgb[2],
                Self::ALPHA,
            ]
        } else {
            [
                palette.buy_rgb[0],
                palette.buy_rgb[1],
                palette.buy_rgb[2],
                Self::ALPHA,
            ]
        };

        Self {
            y_world,
            x_bin_rel,
            x_frac,
            radius_px,
            _pad: 0.0,
            color: rgba,
        }
    }

    #[inline]
    fn y_world_for_price(price: Price, base_price: Price, step: PriceStep, w: &ViewWindow) -> f32 {
        let step_units = step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);

        let base_steps = base_price.units / step_units;
        let base_abs_y_bin = base_steps.div_euclid(y_div);

        let abs_steps = price.units / step_units;
        let abs_y_bin = abs_steps.div_euclid(y_div);

        let rel_y_bin = abs_y_bin - base_abs_y_bin;

        let center_steps = (rel_y_bin as f32 + 0.5) * (w.steps_per_y_bin as f32);
        -(center_steps * w.row_h)
    }
}
