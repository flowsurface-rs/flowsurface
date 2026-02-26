use crate::widget::chart::heatmap::scene::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::view::ViewWindow;
use bytemuck::{Pod, Zeroable};
use exchange::unit::Qty;

pub const RECT_VERTICES: &[[f32; 2]] = &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
pub const RECT_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];

pub const MIN_BAR_PX: f32 = 1.0;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub x0_bin: i32,
    pub x1_bin_excl: i32,
    pub x_from_bins: u32,
    pub fade_mode: u32, // 0=fade, 1=skip
    pub subpx_alpha: f32,
}

impl RectInstance {
    const DEPTH_PROFILE_ALPHA: f32 = 1.0;

    const VOLUME_TOTAL_ALPHA: f32 = 0.65;
    const VOLUME_DELTA_ALPHA: f32 = 1.0;
    const VOLUME_DELTA_TINT_TO_WHITE: f32 = 0.12;

    const VOLUME_PROFILE_ALPHA: f32 = 1.0;

    fn extent_and_subpx_alpha(raw_world: f32, cam_scale: f32) -> (f32, f32) {
        let raw_world = raw_world.max(0.0);
        if raw_world <= 0.0 {
            return (0.0, 0.0);
        }

        let sx = cam_scale.max(1e-6);
        let raw_px = raw_world * sx;

        if raw_px >= MIN_BAR_PX {
            return (raw_world, 1.0);
        }

        let draw_world = MIN_BAR_PX / sx;
        let subpx_alpha = (raw_px / MIN_BAR_PX).clamp(0.0, 1.0);
        (draw_world, subpx_alpha)
    }

    pub fn depth_profile_bar(
        y_world: f32,
        qty: Qty,
        max_qty: Qty,
        is_bid: bool,
        w: &ViewWindow,
        palette: &HeatmapPalette,
    ) -> Self {
        let t = (qty.to_f32_lossy() / max_qty.to_scale_or_one()).clamp(0.0, 1.0);
        let raw_w_world = t * w.depth_profile_max_width;
        let (w_world, subpx_alpha) = Self::extent_and_subpx_alpha(raw_w_world, w.cam_scale);
        let center_x = 0.5 * w_world;

        let rgb = if is_bid {
            palette.bid_rgb
        } else {
            palette.ask_rgb
        };

        Self {
            position: [center_x, y_world],
            size: [w_world, w.y_bin_h_world],
            color: [rgb[0], rgb[1], rgb[2], Self::DEPTH_PROFILE_ALPHA],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 0,
            subpx_alpha,
        }
    }

    pub fn volume_total_bar(
        total_qty: Qty,
        max_qty: Qty,
        buy_qty: Qty,
        sell_qty: Qty,
        x0_bin: i32,
        x1_bin_excl: i32,
        w: &ViewWindow,
        palette: &HeatmapPalette,
    ) -> Self {
        let denom = max_qty.to_scale_or_one();

        let (base_rgb, _is_tie) = if buy_qty > sell_qty {
            (palette.buy_rgb, false)
        } else if sell_qty > buy_qty {
            (palette.sell_rgb, false)
        } else {
            (palette.secondary_rgb, true)
        };

        let raw_total_h = (total_qty.to_f32_lossy() / denom) * w.volume_area_max_height;
        let (total_h, subpx_alpha) = Self::extent_and_subpx_alpha(raw_total_h, w.cam_scale);
        let total_center_y = w.volume_area_bottom_y - 0.5 * total_h;

        Self {
            position: [0.0, total_center_y],
            size: [0.0, total_h],
            color: [
                base_rgb[0],
                base_rgb[1],
                base_rgb[2],
                Self::VOLUME_TOTAL_ALPHA,
            ],
            x0_bin,
            x1_bin_excl,
            x_from_bins: 1,
            fade_mode: 0,
            subpx_alpha,
        }
    }

    pub fn volume_delta_bar(
        diff_qty: Qty,
        total_h: f32,
        max_qty: Qty,
        base_rgb: [f32; 3],
        x0_bin: i32,
        x1_bin_excl: i32,
        w: &ViewWindow,
    ) -> Self {
        let denom = max_qty.to_scale_or_one();
        let raw_overlay_h =
            ((diff_qty.to_f32_lossy() / denom) * w.volume_area_max_height).min(total_h.max(0.0));
        let (overlay_h, subpx_alpha) = Self::extent_and_subpx_alpha(raw_overlay_h, w.cam_scale);
        let overlay_h = overlay_h.min(total_h.max(0.0));

        let t = Self::VOLUME_DELTA_TINT_TO_WHITE;
        let overlay_rgb = [
            base_rgb[0] + (1.0 - base_rgb[0]) * t,
            base_rgb[1] + (1.0 - base_rgb[1]) * t,
            base_rgb[2] + (1.0 - base_rgb[2]) * t,
        ];

        let overlay_center_y = w.volume_area_bottom_y - 0.5 * overlay_h;

        Self {
            position: [0.0, overlay_center_y],
            size: [0.0, overlay_h],
            color: [
                overlay_rgb[0],
                overlay_rgb[1],
                overlay_rgb[2],
                Self::VOLUME_DELTA_ALPHA,
            ],
            x0_bin,
            x1_bin_excl,
            x_from_bins: 1,
            fade_mode: 0,
            subpx_alpha,
        }
    }

    pub fn volume_profile_split_bar(
        y_world: f32,
        width_world: f32,
        left_edge_world: f32,
        w: &ViewWindow,
        rgb: [f32; 3],
    ) -> Self {
        let raw_width_world = width_world.max(0.0);
        let (width_world, subpx_alpha) = Self::extent_and_subpx_alpha(raw_width_world, w.cam_scale);
        let center_x = left_edge_world + 0.5 * width_world;

        Self {
            position: [center_x, y_world],
            size: [width_world, w.y_bin_h_world],
            color: [rgb[0], rgb[1], rgb[2], Self::VOLUME_PROFILE_ALPHA],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 1,
            subpx_alpha,
        }
    }

    #[inline]
    pub fn y_center_for_bin(y_bin: i64, w: &ViewWindow) -> f32 {
        -((y_bin as f32 + 0.5) * w.y_bin_h_world)
    }
}
