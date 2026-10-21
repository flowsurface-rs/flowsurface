pub mod heatmap;
pub mod kline;

use iced::{Color, Rectangle, theme::palette::Extended};

const LEGEND_TEXT_SIZE: f32 = 12.0;
const LEGEND_CHAR_WIDTH: f32 = LEGEND_TEXT_SIZE * 0.64;
const LEGEND_TOP_PAD: f32 = 4.0;
const LEGEND_PADDING: f32 = 4.0;
const LEGEND_LEFT_PAD: f32 = LEGEND_PADDING * 2.0;
const LEGEND_ROW_HEIGHT: f32 = LEGEND_TEXT_SIZE + 6.0;
const LEGEND_TITLE_GAP: f32 = 8.0;
const LEGEND_LABEL_VALUE_GAP: f32 = 2.0;
const LEGEND_READOUT_GAP: f32 = 6.0;

const PANEL_SPLITTER_HEIGHT: f32 = 1.0;

fn legend_background_color(palette: &Extended) -> Color {
    let alpha = if palette.is_dark { 0.9 } else { 0.7 };
    palette.background.weakest.color.scale_alpha(alpha)
}

fn legend_row_background(row: Rectangle, content_end_x: f32) -> Rectangle {
    Rectangle {
        x: row.x + LEGEND_PADDING,
        y: row.y,
        width: (content_end_x - row.x).max(0.0),
        height: row.height,
    }
}

pub(crate) const Y_AXIS_LABEL_LEFT_INSET: f32 = 4.0;

fn panel_splitter_hit_bounds(bounds: Rectangle) -> Rectangle {
    const HIT_HEIGHT: f32 = 8.0;
    Rectangle {
        y: bounds.center_y() - HIT_HEIGHT * 0.5,
        height: HIT_HEIGHT,
        ..bounds
    }
}

fn panel_plot_heights(
    panel_stack_height: f32,
    panel_count: usize,
    normalized_splits: &[f32],
    splitter_height: f32,
) -> Vec<f32> {
    if panel_count == 0 {
        return Vec::new();
    }

    let non_plot = panel_count.saturating_sub(1) as f32 * splitter_height;
    let usable = (panel_stack_height - non_plot).max(0.0);

    if panel_count == 1 {
        return vec![usable];
    }

    let mut splits = normalized_splits.to_vec();
    let split_count = panel_count.saturating_sub(1);

    if splits.len() > split_count {
        splits.truncate(split_count);
    }

    for index in splits.len()..split_count {
        let fallback = (index + 1) as f32 / panel_count as f32;
        splits.push(fallback);
    }

    let mut heights = Vec::with_capacity(panel_count);
    let mut previous = 0.0;

    for split in splits {
        let boundary = (split.clamp(0.0, 1.0) * usable)
            .round()
            .clamp(previous, usable);
        heights.push((boundary - previous).max(0.0));
        previous = boundary;
    }

    heights.push((usable - previous).max(0.0));
    heights
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
