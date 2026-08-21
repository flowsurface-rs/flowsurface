use super::*;
use data::config::theme::{composite_color, contrast_ratio};
use data::util::abbr_large_numbers;

const FOOTPRINT_TEXT_ROW_HEIGHT_PX: f32 = 10.0;
const FOOTPRINT_TEXT_MAX_LABEL_CHARS: usize = 8;

fn draw_footprint_bid_ask_row(
    frame: &mut canvas::Frame,
    x_center: f32,
    y: f32,
    row_height: f32,
    cell_width: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_side_qty: f64,
    show_text: bool,
    palette: &Extended,
) {
    let row_height = row_height.max(1.0);
    let row_top = y - row_height * 0.5;
    let cell_height = (row_height - 1.0).max(1.0);
    let center_divider_width = (cell_width * 0.025).max(1.0);
    let cluster_gap = (cell_width * 0.02).max(2.0);
    let ask_area_right = x_center - center_divider_width * 0.5 - cluster_gap;
    let bid_area_left = x_center + center_divider_width * 0.5 + cluster_gap;
    let side_width = (cell_width * 0.5 - center_divider_width * 0.5 - cluster_gap).max(0.0);
    let sell_ratio = (sell_qty / max_side_qty).clamp(0.0, 1.0) as f32;
    let buy_ratio = (buy_qty / max_side_qty).clamp(0.0, 1.0) as f32;
    let bar_alpha = if show_text && row_height >= FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        0.25
    } else {
        1.0
    };

    if sell_qty > 0.0 && side_width > 0.0 {
        frame.fill_rectangle(
            Point::new(ask_area_right - side_width * sell_ratio, row_top),
            Size::new(side_width * sell_ratio, cell_height),
            palette.danger.base.color.scale_alpha(bar_alpha),
        );
    }
    if buy_qty > 0.0 && side_width > 0.0 {
        frame.fill_rectangle(
            Point::new(bid_area_left, row_top),
            Size::new(side_width * buy_ratio, cell_height),
            palette.success.base.color.scale_alpha(bar_alpha),
        );
    }

    if !show_text || row_height < FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        return;
    }

    let text_size = (row_height - 1.0)
        .min(side_width / FOOTPRINT_TEXT_MAX_LABEL_CHARS as f32 / (CHAR_W / TEXT_SIZE))
        .clamp(FOOTPRINT_TEXT_MIN_SIZE_PX, TEXT_SIZE);
    let text_y = y + text_size * 0.08;
    let text_color = palette.background.weakest.text;

    if sell_qty > 0.0 {
        frame.fill_text(canvas::Text {
            content: abbr_large_numbers(sell_qty),
            position: Point::new(ask_area_right, text_y),
            color: text_color,
            size: text_size.into(),
            align_x: iced::Alignment::End.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }
    if buy_qty > 0.0 {
        frame.fill_text(canvas::Text {
            content: abbr_large_numbers(buy_qty),
            position: Point::new(bid_area_left, text_y),
            color: text_color,
            size: text_size.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }
}

pub(super) fn draw_footprint_cluster_row(
    frame: &mut canvas::Frame,
    cluster_kind: ClusterKind,
    x_center: f32,
    y: f32,
    row_height: f32,
    cell_width: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    show_text: bool,
    palette: &Extended,
) {
    match cluster_kind {
        ClusterKind::BidAsk => draw_footprint_bid_ask_row(
            frame,
            x_center,
            y,
            row_height,
            cell_width,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            show_text,
            palette,
        ),
        ClusterKind::VolumeProfile => draw_footprint_volume_profile_row(
            frame,
            x_center,
            y,
            row_height,
            cell_width,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            show_text,
            palette,
        ),
        ClusterKind::DeltaProfile => draw_footprint_delta_profile_row(
            frame,
            x_center,
            y,
            row_height,
            cell_width,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            show_text,
            palette,
        ),
        ClusterKind::Table => draw_footprint_table_row(
            frame,
            x_center,
            y,
            row_height,
            cell_width,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            show_text,
            palette,
        ),
    }
}

fn draw_footprint_volume_profile_row(
    frame: &mut canvas::Frame,
    x_center: f32,
    y: f32,
    row_height: f32,
    cell_width: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    show_text: bool,
    palette: &Extended,
) {
    let row_height = row_height.max(1.0);
    let cell_height = (row_height - 1.0).max(1.0);
    let total_qty = sell_qty + buy_qty;
    if total_qty <= 0.0 || max_cluster_qty <= 0.0 {
        return;
    }

    let left = x_center - cell_width * 0.5;
    let total_width = (total_qty / max_cluster_qty).clamp(0.0, 1.0) as f32 * cell_width;
    let sell_width = total_width * (sell_qty / total_qty) as f32;
    let bar_alpha = if show_text && row_height >= FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        0.25
    } else {
        1.0
    };

    if sell_width > 0.0 {
        frame.fill_rectangle(
            Point::new(left, y - cell_height * 0.5),
            Size::new(sell_width, cell_height),
            palette.danger.base.color.scale_alpha(bar_alpha),
        );
    }
    if total_width - sell_width > 0.0 {
        frame.fill_rectangle(
            Point::new(left + sell_width, y - cell_height * 0.5),
            Size::new(total_width - sell_width, cell_height),
            palette.success.base.color.scale_alpha(bar_alpha),
        );
    }

    draw_footprint_row_text(
        frame,
        abbr_large_numbers(total_qty),
        Point::new(left + 2.0, y),
        row_height,
        cell_width,
        show_text,
        palette,
        iced::Alignment::Start,
    );
}

fn draw_footprint_delta_profile_row(
    frame: &mut canvas::Frame,
    x_center: f32,
    y: f32,
    row_height: f32,
    cell_width: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    show_text: bool,
    palette: &Extended,
) {
    let row_height = row_height.max(1.0);
    let cell_height = (row_height - 1.0).max(1.0);
    let delta = buy_qty - sell_qty;
    let bar_width = if max_cluster_qty > 0.0 {
        (delta.abs() / max_cluster_qty).clamp(0.0, 1.0) as f32 * cell_width
    } else {
        0.0
    };
    let bar_alpha = if show_text && row_height >= FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        0.25
    } else {
        1.0
    };

    if bar_width > 0.0 {
        frame.fill_rectangle(
            Point::new(x_center - cell_width * 0.5, y - cell_height * 0.5),
            Size::new(bar_width, cell_height),
            if delta >= 0.0 {
                palette.success.base.color.scale_alpha(bar_alpha)
            } else {
                palette.danger.base.color.scale_alpha(bar_alpha)
            },
        );
    }

    draw_footprint_row_text(
        frame,
        abbr_large_numbers(delta),
        Point::new(x_center - cell_width * 0.5 + 2.0, y),
        row_height,
        cell_width,
        show_text,
        palette,
        iced::Alignment::Start,
    );
}

pub(super) fn draw_footprint_table_row(
    frame: &mut canvas::Frame,
    x_center: f32,
    y: f32,
    row_height: f32,
    cell_width: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    show_text: bool,
    palette: &Extended,
) {
    let row_height = row_height.max(1.0);
    let cell_height = (row_height - 1.0).max(1.0);
    let left = x_center - cell_width * 0.5;
    let half_width = cell_width * 0.5;
    let sell_alpha = footprint_volume_alpha(sell_qty, max_cluster_qty);
    let buy_alpha = footprint_volume_alpha(buy_qty, max_cluster_qty);
    let row_top = y - cell_height * 0.5;
    let sell_color = palette.danger.base.color.scale_alpha(sell_alpha);
    let buy_color = palette.success.base.color.scale_alpha(buy_alpha);

    frame.fill_rectangle(
        Point::new(left, row_top),
        Size::new(cell_width, cell_height),
        palette.background.base.color,
    );
    frame.fill_rectangle(
        Point::new(left, row_top),
        Size::new(half_width, cell_height),
        sell_color,
    );
    frame.fill_rectangle(
        Point::new(left + half_width, row_top),
        Size::new(half_width, cell_height),
        buy_color,
    );

    if show_text && row_height >= FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        let text_size = (row_height - 1.0)
            .min(half_width / FOOTPRINT_TEXT_MAX_LABEL_CHARS as f32 / (CHAR_W / TEXT_SIZE))
            .clamp(FOOTPRINT_TEXT_MIN_SIZE_PX, TEXT_SIZE);
        let sell_text_color = footprint_table_text_color(sell_color, palette);
        let buy_text_color = footprint_table_text_color(buy_color, palette);
        let text_y = y + text_size * 0.08;
        frame.fill_text(canvas::Text {
            content: abbr_large_numbers(sell_qty),
            position: Point::new(left + half_width - 2.0, text_y),
            color: sell_text_color,
            size: text_size.into(),
            align_x: iced::Alignment::End.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
        frame.fill_text(canvas::Text {
            content: abbr_large_numbers(buy_qty),
            position: Point::new(left + half_width + 2.0, text_y),
            color: buy_text_color,
            size: text_size.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    let grid_color = palette.background.weakest.text.scale_alpha(0.32);
    frame.fill_rectangle(
        Point::new(left, row_top),
        Size::new(cell_width, 1.0),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(left, row_top + cell_height - 1.0),
        Size::new(cell_width, 1.0),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(left, row_top),
        Size::new(1.0, cell_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(x_center - 0.5, row_top),
        Size::new(1.0, cell_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(left + cell_width - 1.0, row_top),
        Size::new(1.0, cell_height),
        grid_color,
    );
}

fn draw_footprint_row_text(
    frame: &mut canvas::Frame,
    content: String,
    position: Point,
    row_height: f32,
    cell_width: f32,
    show_text: bool,
    palette: &Extended,
    align_x: iced::Alignment,
) {
    if !show_text || row_height < FOOTPRINT_TEXT_ROW_HEIGHT_PX {
        return;
    }

    let text_size = (row_height - 1.0)
        .min(cell_width / FOOTPRINT_TEXT_MAX_LABEL_CHARS as f32 / (CHAR_W / TEXT_SIZE))
        .clamp(FOOTPRINT_TEXT_MIN_SIZE_PX, TEXT_SIZE);
    frame.fill_text(canvas::Text {
        content,
        position,
        color: palette.background.weakest.text,
        size: text_size.into(),
        align_x: align_x.into(),
        align_y: iced::Alignment::Center.into(),
        font: style::AZERET_MONO,
        ..Default::default()
    });
}

fn footprint_volume_alpha(qty: f64, max_qty: f64) -> f32 {
    const MIN_ALPHA: f32 = 0.04;

    let intensity = if max_qty > 0.0 {
        (qty / max_qty).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    MIN_ALPHA + intensity * (1.0 - MIN_ALPHA)
}

fn footprint_table_text_color(cell_color: iced::Color, palette: &Extended) -> iced::Color {
    let cell_background = composite_color(cell_color, palette.background.base.color);
    let default_color = palette.background.weakest.text;
    let inverted_color = palette.background.base.color;

    if contrast_ratio(cell_background, inverted_color)
        > contrast_ratio(cell_background, default_color)
    {
        inverted_color
    } else {
        default_color
    }
}

pub(super) fn draw_footprint_candle(
    frame: &mut canvas::Frame,
    x_center: f32,
    candle_body_width: f32,
    y_open: f32,
    y_high: f32,
    y_low: f32,
    y_close: f32,
    color: iced::Color,
) {
    let body_top = y_open.min(y_close);
    let body_height = (y_open - y_close).abs().max(1.0);

    frame.stroke(
        &canvas::Path::rectangle(
            Point::new(x_center - candle_body_width * 0.5, body_top),
            Size::new(candle_body_width, body_height),
        ),
        canvas::Stroke::default().with_width(1.0).with_color(color),
    );
    frame.stroke(
        &canvas::Path::line(Point::new(x_center, y_high), Point::new(x_center, y_low)),
        canvas::Stroke::default()
            .with_width(1.0)
            .with_color(color.scale_alpha(0.6)),
    );
}
