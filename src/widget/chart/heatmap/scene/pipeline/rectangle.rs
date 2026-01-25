use crate::widget::chart::heatmap::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::view::ViewWindow;
use bytemuck::{Pod, Zeroable};

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
}

impl RectInstance {
    const PROFILE_ALPHA: f32 = 1.0;

    const VOLUME_TOTAL_ALPHA: f32 = 0.65;
    const VOLUME_DELTA_ALPHA: f32 = 1.0;
    const VOLUME_DELTA_TINT_TO_WHITE: f32 = 0.12;

    const TRADE_PROFILE_ALPHA: f32 = 1.0;

    pub fn profile_bar(
        y_world: f32,
        qty: f32,
        max_qty: f32,
        is_bid: bool,
        w: &ViewWindow,
        palette: &HeatmapPalette,
    ) -> Self {
        let min_bar_w_world = MIN_BAR_PX / w.cam_scale;
        let t = (qty / max_qty).clamp(0.0, 1.0);
        let w_world = (t * w.profile_max_w_world).max(min_bar_w_world);
        let center_x = 0.5 * w_world;

        let rgb = if is_bid {
            palette.bid_rgb
        } else {
            palette.ask_rgb
        };

        Self {
            position: [center_x, y_world],
            size: [w_world, w.y_bin_h_world],
            color: [rgb[0], rgb[1], rgb[2], Self::PROFILE_ALPHA],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 0,
        }
    }

    pub fn volume_total_bar(
        total_qty: f32,
        max_qty: f32,
        buy_qty: f32,
        sell_qty: f32,
        x0_bin: i32,
        x1_bin_excl: i32,
        w: &ViewWindow,
        palette: &HeatmapPalette,
    ) -> Self {
        const MIN_BAR_PX: f32 = 1.0;
        const EPS: f32 = 1e-12;

        let denom = max_qty.max(1e-12);
        let min_h_world = MIN_BAR_PX / w.cam_scale;

        let (base_rgb, _is_tie) = if buy_qty > sell_qty + EPS {
            (palette.buy_rgb, false)
        } else if sell_qty > buy_qty + EPS {
            (palette.sell_rgb, false)
        } else {
            (palette.secondary_rgb, true)
        };

        let total_h = ((total_qty / denom) * w.strip_h_world).max(min_h_world);
        let total_center_y = w.strip_bottom_y - 0.5 * total_h;

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
        }
    }

    pub fn volume_delta_bar(
        diff_qty: f32,
        total_h: f32,
        max_qty: f32,
        base_rgb: [f32; 3],
        x0_bin: i32,
        x1_bin_excl: i32,
        w: &ViewWindow,
    ) -> Self {
        const MIN_BAR_PX: f32 = 1.0;

        let denom = max_qty.max(1e-12);
        let min_h_world = MIN_BAR_PX / w.cam_scale;

        let mut overlay_h = ((diff_qty / denom) * w.strip_h_world).max(min_h_world);
        overlay_h = overlay_h.min(total_h);

        let t = Self::VOLUME_DELTA_TINT_TO_WHITE;
        let overlay_rgb = [
            base_rgb[0] + (1.0 - base_rgb[0]) * t,
            base_rgb[1] + (1.0 - base_rgb[1]) * t,
            base_rgb[2] + (1.0 - base_rgb[2]) * t,
        ];

        let overlay_center_y = w.strip_bottom_y - 0.5 * overlay_h;

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
        }
    }

    pub fn trade_profile_split_bar(
        y_world: f32,
        width_world: f32,
        left_edge_world: f32,
        w: &ViewWindow,
        rgb: [f32; 3],
    ) -> Self {
        let width_world = width_world.max(0.0);
        let center_x = left_edge_world + 0.5 * width_world;

        Self {
            position: [center_x, y_world],
            size: [width_world, w.y_bin_h_world],
            color: [rgb[0], rgb[1], rgb[2], Self::TRADE_PROFILE_ALPHA],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 1,
        }
    }

    pub fn trade_profile_total_bar(
        y_world: f32,
        total_qty: f32,
        max_qty: f32,
        buy_qty: f32,
        sell_qty: f32,
        left_edge_world: f32,
        w: &ViewWindow,
        palette: &HeatmapPalette,
    ) -> Self {
        const MIN_BAR_PX: f32 = 1.0;
        const EPS: f32 = 1e-12;

        let denom = max_qty.max(1e-12);
        let min_w_world = MIN_BAR_PX / w.cam_scale;

        let base_rgb = if buy_qty > sell_qty + EPS {
            palette.buy_rgb
        } else if sell_qty > buy_qty + EPS {
            palette.sell_rgb
        } else {
            [
                0.5 * (palette.buy_rgb[0] + palette.sell_rgb[0]),
                0.5 * (palette.buy_rgb[1] + palette.sell_rgb[1]),
                0.5 * (palette.buy_rgb[2] + palette.sell_rgb[2]),
            ]
        };

        let total_w = ((total_qty / denom) * w.trade_profile_max_w_world).max(min_w_world);
        let center_x = left_edge_world + 0.5 * total_w;

        Self {
            position: [center_x, y_world],
            size: [total_w, w.y_bin_h_world],
            color: [
                base_rgb[0],
                base_rgb[1],
                base_rgb[2],
                Self::TRADE_PROFILE_ALPHA,
            ],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 1,
        }
    }

    pub fn trade_profile_delta_bar(
        y_world: f32,
        diff_qty: f32,
        total_w: f32,
        max_qty: f32,
        base_rgb: [f32; 3],
        left_edge_world: f32,
        w: &ViewWindow,
    ) -> Self {
        const MIN_BAR_PX: f32 = 1.0;

        let denom = max_qty.max(1e-12);
        let min_w_world = MIN_BAR_PX / w.cam_scale;

        let mut overlay_w = ((diff_qty / denom) * w.trade_profile_max_w_world).max(min_w_world);
        overlay_w = overlay_w.min(total_w);

        let t = Self::VOLUME_DELTA_TINT_TO_WHITE;
        let overlay_rgb = [
            base_rgb[0] + (1.0 - base_rgb[0]) * t,
            base_rgb[1] + (1.0 - base_rgb[1]) * t,
            base_rgb[2] + (1.0 - base_rgb[2]) * t,
        ];

        let center_x = left_edge_world + 0.5 * overlay_w;

        Self {
            position: [center_x, y_world],
            size: [overlay_w, w.y_bin_h_world],
            color: [
                overlay_rgb[0],
                overlay_rgb[1],
                overlay_rgb[2],
                Self::VOLUME_DELTA_ALPHA,
            ],
            x0_bin: 0,
            x1_bin_excl: 0,
            x_from_bins: 0,
            fade_mode: 1,
        }
    }

    #[inline]
    pub fn y_center_for_bin(y_bin: i64, w: &ViewWindow) -> f32 {
        -((y_bin as f32 + 0.5) * w.y_bin_h_world)
    }
}
