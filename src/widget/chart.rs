pub mod heatmap;
pub mod kline;

use iced::Rectangle;
use kline::{
    CHAR_W as LEGEND_CHAR_WIDTH, PANEL_SPLITTER_HEIGHT,
    PANEL_TITLE_TO_CONTROLS_GAP as LEGEND_TITLE_GAP, PANEL_TITLE_TOP_PAD as LEGEND_TOP_PAD,
    TEXT_SIZE as LEGEND_TEXT_SIZE, TICKER_LEGEND_PADDING as LEGEND_PADDING,
    TICKER_LEGEND_ROW_H as LEGEND_ROW_HEIGHT, TOOLTIP_ROW_LEFT_PAD as LEGEND_LEFT_PAD,
    Y_AXIS_LABEL_LEFT_INSET, legend_background_color, legend_row_background, panel_plot_heights,
};

const LEGEND_LABEL_VALUE_GAP: f32 = 2.0;
const LEGEND_READOUT_GAP: f32 = 6.0;

fn panel_splitter_hit_bounds(bounds: Rectangle) -> Rectangle {
    const HIT_HEIGHT: f32 = 8.0;
    Rectangle {
        y: bounds.center_y() - HIT_HEIGHT * 0.5,
        height: HIT_HEIGHT,
        ..bounds
    }
}

fn split_ratio_from_cursor(
    local_y: f32,
    split_index: usize,
    panel_count: usize,
    usable_height: f32,
    splitter_height: f32,
) -> Option<f32> {
    if split_index + 1 >= panel_count {
        return None;
    }
    let fixed_before = split_index as f32 * splitter_height + splitter_height * 0.5;
    let usable = usable_height.max(1.0);
    Some((local_y - fixed_before).round().clamp(0.0, usable) / usable)
}
