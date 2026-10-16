use super::helpers::PixelGeometry;
use super::*;
use data::chart::kline_data::ClusterKind;
use data::chart::kline_data::KlineTrades;
use data::config::theme::{composite_color, contrast_ratio};
use data::util::abbr_large_numbers;
use exchange::unit::{Price, PriceStep};

const FOOTPRINT_TEXT_HORIZONTAL_INSET_PX: f32 = 2.0;
const FOOTPRINT_TEXT_VERTICAL_INSET_PX: f32 = 1.0;
const FOOTPRINT_TEXT_SIZE_STEP_PX: f32 = 1.0;
const FOOTPRINT_TEXT_MIN_WIDTH_CHARS: usize = 4;
const FOOTPRINT_TEXT_MIN_SIZE_PX: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
pub(super) enum FootprintTextMode {
    Hidden,
    FixedSize(f32),
}

impl FootprintTextMode {
    fn size(self) -> Option<f32> {
        match self {
            Self::Hidden => None,
            Self::FixedSize(size) => Some(size),
        }
    }

    pub(super) fn row_intersects_plot(
        self,
        y: f32,
        row_height: f32,
        min_row_height: f32,
        plot: Rectangle,
    ) -> bool {
        // Keep edge text whose glyphs extend beyond the row's background rectangle.
        let half_height =
            (row_height.max(min_row_height) * 0.5).max(self.size().unwrap_or_default());
        y + half_height >= plot.y && y - half_height <= plot.y + plot.height
    }

    fn bar_alpha(self) -> f32 {
        match self {
            Self::Hidden => 1.0,
            Self::FixedSize(_) => 0.25,
        }
    }
}

pub(super) struct FootprintImbalance<'a> {
    footprint: &'a KlineTrades,
    price: Price,
    tick_size: PriceStep,
    threshold_percent: usize,
    ignore_zeros: bool,
    color: iced::Color,
}

impl<'a> FootprintImbalance<'a> {
    pub(super) fn new(
        footprint: &'a KlineTrades,
        price: Price,
        tick_size: PriceStep,
        threshold_percent: usize,
        ignore_zeros: bool,
        color: iced::Color,
    ) -> Self {
        Self {
            footprint,
            price,
            tick_size,
            threshold_percent,
            ignore_zeros,
            color,
        }
    }

    fn sides(&self, sell_qty: f64, buy_qty: f64) -> ImbalanceSides {
        let higher_buy_qty = self
            .price
            .checked_add_steps(1, self.tick_size)
            .map(|price| {
                self.footprint
                    .trades
                    .get(&price)
                    .map_or(0.0, |group| group.buy_qty.to_f64())
            });
        let lower_sell_qty = self
            .price
            .checked_add_steps(-1, self.tick_size)
            .map(|price| {
                self.footprint
                    .trades
                    .get(&price)
                    .map_or(0.0, |group| group.sell_qty.to_f64())
            });

        ImbalanceSides {
            sell: higher_buy_qty.is_some_and(|diagonal_qty| {
                exceeds_diagonal_threshold(
                    sell_qty,
                    diagonal_qty,
                    self.threshold_percent,
                    self.ignore_zeros,
                )
            }),
            buy: lower_sell_qty.is_some_and(|diagonal_qty| {
                exceeds_diagonal_threshold(
                    buy_qty,
                    diagonal_qty,
                    self.threshold_percent,
                    self.ignore_zeros,
                )
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct ImbalanceSides {
    sell: bool,
    buy: bool,
}

fn exceeds_diagonal_threshold(
    qty: f64,
    diagonal_qty: f64,
    threshold_percent: usize,
    ignore_zeros: bool,
) -> bool {
    if ignore_zeros && (qty <= 0.0 || diagonal_qty <= 0.0) {
        return false;
    }

    let required_qty = diagonal_qty * (100 + threshold_percent) as f64 / 100.0;
    qty > required_qty
}

#[derive(Clone, Copy, Debug)]
struct FootprintLane {
    left: f32,
    width: f32,
}

impl FootprintLane {
    fn right(self) -> f32 {
        self.left + self.width
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FootprintColumnLayout {
    sell_cluster: FootprintLane,
    sell_marker: Option<FootprintLane>,
    candle: FootprintLane,
    buy_marker: Option<FootprintLane>,
    buy_cluster: FootprintLane,
    profile_cluster: FootprintLane,
}

impl FootprintColumnLayout {
    pub(super) fn new(x_center: f32, width: f32, reserve_marker_lanes: bool) -> Self {
        let width = width.max(1.0);
        let marker_width = if reserve_marker_lanes && width >= 12.0 {
            (width * 0.08).clamp(1.0, 3.0)
        } else {
            0.0
        };
        let marker_gap = if marker_width > 0.0 {
            (width * 0.02).clamp(0.5, 1.5)
        } else {
            0.0
        };
        let left = x_center - width * 0.5;
        let cluster_left = left + marker_width + marker_gap;
        let profile_cluster = FootprintLane {
            left: cluster_left,
            width: (width - 2.0 * (marker_width + marker_gap)).max(0.0),
        };
        let cluster_width = profile_cluster.width * 0.5;
        let sell_marker = (marker_width > 0.0).then_some(FootprintLane {
            left,
            width: marker_width,
        });
        let sell_cluster = FootprintLane {
            left: profile_cluster.left,
            width: cluster_width,
        };
        let candle = FootprintLane { left, width };
        let buy_cluster = FootprintLane {
            left: sell_cluster.right(),
            width: profile_cluster.right() - sell_cluster.right(),
        };
        let buy_marker = (marker_width > 0.0).then_some(FootprintLane {
            left: profile_cluster.right() + marker_gap,
            width: marker_width,
        });

        Self {
            sell_cluster,
            sell_marker,
            candle,
            buy_marker,
            buy_cluster,
            profile_cluster,
        }
    }

    fn candle_center(self) -> f32 {
        self.candle.left + self.candle.width * 0.5
    }

    fn candle_width(self) -> f32 {
        self.candle.width
    }

    fn minimum_cluster_width(self) -> f32 {
        self.sell_cluster.width.min(self.buy_cluster.width)
    }
}

pub(super) fn resolve_footprint_text_mode(
    column_width: f32,
    reserve_marker_lanes: bool,
    minimum_row_height: Option<f32>,
    maximum_content_chars: usize,
) -> FootprintTextMode {
    let layout = FootprintColumnLayout::new(0.0, column_width, reserve_marker_lanes);
    let required_width = FOOTPRINT_TEXT_HORIZONTAL_INSET_PX * 2.0
        + FOOTPRINT_TEXT_MIN_WIDTH_CHARS as f32 * (CHAR_W / TEXT_SIZE) * FOOTPRINT_TEXT_MIN_SIZE_PX;
    if layout.minimum_cluster_width() < required_width {
        return FootprintTextMode::Hidden;
    }

    let Some(minimum_row_height) = minimum_row_height else {
        return FootprintTextMode::Hidden;
    };
    if maximum_content_chars == 0 {
        return FootprintTextMode::Hidden;
    }

    let available_width = layout.minimum_cluster_width() - FOOTPRINT_TEXT_HORIZONTAL_INSET_PX * 2.0;
    let available_height = minimum_row_height - FOOTPRINT_TEXT_VERTICAL_INSET_PX;
    if available_width <= 0.0 || available_height <= 0.0 {
        return FootprintTextMode::Hidden;
    }

    let width_per_size = maximum_content_chars as f32 * (CHAR_W / TEXT_SIZE);
    let text_size = available_height
        .min(available_width / width_per_size)
        .min(TEXT_SIZE);
    let text_size = (text_size / FOOTPRINT_TEXT_SIZE_STEP_PX).floor() * FOOTPRINT_TEXT_SIZE_STEP_PX;

    if text_size >= FOOTPRINT_TEXT_MIN_SIZE_PX {
        FootprintTextMode::FixedSize(text_size)
    } else {
        FootprintTextMode::Hidden
    }
}

pub(super) fn footprint_text_content_lengths(
    cluster_kind: ClusterKind,
    sell_qty: f64,
    buy_qty: f64,
) -> [usize; 2] {
    let formatted_length = |quantity| abbr_large_numbers(quantity).chars().count();

    match cluster_kind {
        ClusterKind::BidAsk => [
            if sell_qty > 0.0 {
                formatted_length(sell_qty)
            } else {
                0
            },
            if buy_qty > 0.0 {
                formatted_length(buy_qty)
            } else {
                0
            },
        ],
        ClusterKind::VolumeProfile => {
            let total_qty = sell_qty + buy_qty;
            if total_qty > 0.0 {
                [formatted_length(total_qty), 0]
            } else {
                [0; 2]
            }
        }
        ClusterKind::DeltaProfile => [formatted_length(buy_qty - sell_qty), 0],
        ClusterKind::Table => [formatted_length(sell_qty), formatted_length(buy_qty)],
    }
}

fn draw_footprint_bid_ask_row(
    frame: &mut canvas::Frame,
    layout: FootprintColumnLayout,
    y: f32,
    row_height: f32,
    min_row_height: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_side_qty: f64,
    text_mode: FootprintTextMode,
    palette: &Extended,
) {
    let row_height = row_height.max(min_row_height);
    let row_top = y - row_height * 0.5;
    let cell_height = (row_height - min_row_height).max(min_row_height);
    let text_size = text_mode.size();
    let sell_text = (sell_qty > 0.0 && text_size.is_some()).then(|| abbr_large_numbers(sell_qty));
    let buy_text = (buy_qty > 0.0 && text_size.is_some()).then(|| abbr_large_numbers(buy_qty));
    let text_y = y + text_size.unwrap_or_default() * 0.08;
    let sell_ratio = (sell_qty / max_side_qty).clamp(0.0, 1.0) as f32;
    let buy_ratio = (buy_qty / max_side_qty).clamp(0.0, 1.0) as f32;
    let bar_alpha = text_mode.bar_alpha();

    if sell_qty > 0.0 && layout.sell_cluster.width > 0.0 {
        let width = (layout.sell_cluster.width * sell_ratio)
            .max(min_row_height)
            .min(layout.sell_cluster.width);
        frame.fill_rectangle(
            Point::new(layout.sell_cluster.right() - width, row_top),
            Size::new(width, cell_height),
            palette.danger.base.color.scale_alpha(bar_alpha),
        );
    }
    if buy_qty > 0.0 && layout.buy_cluster.width > 0.0 {
        let width = (layout.buy_cluster.width * buy_ratio)
            .max(min_row_height)
            .min(layout.buy_cluster.width);
        frame.fill_rectangle(
            Point::new(layout.buy_cluster.left, row_top),
            Size::new(width, cell_height),
            palette.success.base.color.scale_alpha(bar_alpha),
        );
    }

    let text_color = palette.background.weakest.text;
    if let (Some(content), Some(text_size)) = (sell_text, text_size) {
        draw_footprint_text(
            frame,
            content,
            Point::new(
                layout.sell_cluster.right() - FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
                text_y,
            ),
            text_size,
            text_color,
            iced::Alignment::End,
        );
    }
    if let (Some(content), Some(text_size)) = (buy_text, text_size) {
        draw_footprint_text(
            frame,
            content,
            Point::new(
                layout.buy_cluster.left + FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
                text_y,
            ),
            text_size,
            text_color,
            iced::Alignment::Start,
        );
    }
}

pub(super) fn draw_footprint_cluster_row(
    frame: &mut canvas::Frame,
    cluster_kind: ClusterKind,
    layout: FootprintColumnLayout,
    y: f32,
    row_height: f32,
    min_row_height: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    text_mode: FootprintTextMode,
    palette: &Extended,
    imbalance: Option<FootprintImbalance<'_>>,
) {
    match cluster_kind {
        ClusterKind::BidAsk => draw_footprint_bid_ask_row(
            frame,
            layout,
            y,
            row_height,
            min_row_height,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            text_mode,
            palette,
        ),
        ClusterKind::VolumeProfile => draw_footprint_volume_profile_row(
            frame,
            layout,
            y,
            row_height,
            min_row_height,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            text_mode,
            palette,
        ),
        ClusterKind::DeltaProfile => draw_footprint_delta_profile_row(
            frame,
            layout,
            y,
            row_height,
            min_row_height,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            text_mode,
            palette,
        ),
        ClusterKind::Table => draw_footprint_table_row(
            frame,
            layout,
            y,
            row_height,
            min_row_height,
            sell_qty,
            buy_qty,
            max_cluster_qty,
            text_mode,
            palette,
        ),
    }

    let Some(imbalance) = imbalance else {
        return;
    };
    let sides = imbalance.sides(sell_qty, buy_qty);
    if !sides.sell && !sides.buy {
        return;
    }

    let cell_height = (row_height.max(min_row_height) - min_row_height).max(min_row_height);
    let row_top = y - cell_height * 0.5;

    if sides.sell
        && let Some(marker) = layout.sell_marker
    {
        frame.fill_rectangle(
            Point::new(marker.left, row_top),
            Size::new(marker.width, cell_height),
            imbalance.color,
        );
    }
    if sides.buy
        && let Some(marker) = layout.buy_marker
    {
        frame.fill_rectangle(
            Point::new(marker.left, row_top),
            Size::new(marker.width, cell_height),
            imbalance.color,
        );
    }
}

fn draw_footprint_volume_profile_row(
    frame: &mut canvas::Frame,
    layout: FootprintColumnLayout,
    y: f32,
    row_height: f32,
    min_row_height: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    text_mode: FootprintTextMode,
    palette: &Extended,
) {
    let row_height = row_height.max(min_row_height);
    let cell_height = (row_height - min_row_height).max(min_row_height);
    let total_qty = sell_qty + buy_qty;
    if total_qty <= 0.0 || max_cluster_qty <= 0.0 {
        return;
    }

    let total_width = ((total_qty / max_cluster_qty).clamp(0.0, 1.0) as f32
        * layout.profile_cluster.width)
        .max(min_row_height)
        .min(layout.profile_cluster.width);
    let sell_width = total_width * (sell_qty / total_qty) as f32;
    let text_size = text_mode.size();
    let bar_alpha = text_mode.bar_alpha();

    if sell_width > 0.0 {
        frame.fill_rectangle(
            Point::new(layout.profile_cluster.left, y - cell_height * 0.5),
            Size::new(sell_width, cell_height),
            palette.danger.base.color.scale_alpha(bar_alpha),
        );
    }
    if total_width - sell_width > 0.0 {
        frame.fill_rectangle(
            Point::new(
                layout.profile_cluster.left + sell_width,
                y - cell_height * 0.5,
            ),
            Size::new(total_width - sell_width, cell_height),
            palette.success.base.color.scale_alpha(bar_alpha),
        );
    }

    draw_footprint_row_text(
        frame,
        total_qty,
        Point::new(
            layout.profile_cluster.left + FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
            y,
        ),
        text_size,
        palette,
        iced::Alignment::Start,
    );
}

fn draw_footprint_delta_profile_row(
    frame: &mut canvas::Frame,
    layout: FootprintColumnLayout,
    y: f32,
    row_height: f32,
    min_row_height: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    text_mode: FootprintTextMode,
    palette: &Extended,
) {
    let row_height = row_height.max(min_row_height);
    let cell_height = (row_height - min_row_height).max(min_row_height);
    let delta = buy_qty - sell_qty;
    let bar_width = if max_cluster_qty > 0.0 && delta != 0.0 {
        ((delta.abs() / max_cluster_qty).clamp(0.0, 1.0) as f32 * layout.profile_cluster.width)
            .max(min_row_height)
            .min(layout.profile_cluster.width)
    } else {
        0.0
    };
    let text_size = text_mode.size();
    let bar_alpha = text_mode.bar_alpha();

    if bar_width > 0.0 {
        frame.fill_rectangle(
            Point::new(layout.profile_cluster.left, y - cell_height * 0.5),
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
        delta,
        Point::new(
            layout.profile_cluster.left + FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
            y,
        ),
        text_size,
        palette,
        iced::Alignment::Start,
    );
}

pub(super) fn draw_footprint_table_row(
    frame: &mut canvas::Frame,
    layout: FootprintColumnLayout,
    y: f32,
    row_height: f32,
    min_row_height: f32,
    sell_qty: f64,
    buy_qty: f64,
    max_cluster_qty: f64,
    text_mode: FootprintTextMode,
    palette: &Extended,
) {
    let row_height = row_height.max(min_row_height);
    let cell_height = (row_height - min_row_height).max(min_row_height);
    let sell_alpha = footprint_volume_alpha(sell_qty, max_cluster_qty);
    let buy_alpha = footprint_volume_alpha(buy_qty, max_cluster_qty);
    let row_top = y - cell_height * 0.5;
    let sell_color = palette.danger.base.color.scale_alpha(sell_alpha);
    let buy_color = palette.success.base.color.scale_alpha(buy_alpha);
    let text_size = text_mode.size();
    let text_y = y + text_size.unwrap_or_default() * 0.08;

    frame.fill_rectangle(
        Point::new(layout.sell_cluster.left, row_top),
        Size::new(layout.sell_cluster.width, cell_height),
        palette.background.base.color,
    );
    frame.fill_rectangle(
        Point::new(layout.buy_cluster.left, row_top),
        Size::new(layout.buy_cluster.width, cell_height),
        palette.background.base.color,
    );
    frame.fill_rectangle(
        Point::new(layout.sell_cluster.left, row_top),
        Size::new(layout.sell_cluster.width, cell_height),
        sell_color,
    );
    frame.fill_rectangle(
        Point::new(layout.buy_cluster.left, row_top),
        Size::new(layout.buy_cluster.width, cell_height),
        buy_color,
    );

    if let Some(text_size) = text_size {
        let sell_text_color = footprint_table_text_color(sell_color, palette);
        draw_footprint_text(
            frame,
            abbr_large_numbers(sell_qty),
            Point::new(
                layout.sell_cluster.right() - FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
                text_y,
            ),
            text_size,
            sell_text_color,
            iced::Alignment::End,
        );
    }
    if let Some(text_size) = text_size {
        let buy_text_color = footprint_table_text_color(buy_color, palette);
        draw_footprint_text(
            frame,
            abbr_large_numbers(buy_qty),
            Point::new(
                layout.buy_cluster.left + FOOTPRINT_TEXT_HORIZONTAL_INSET_PX,
                text_y,
            ),
            text_size,
            buy_text_color,
            iced::Alignment::Start,
        );
    }

    if cell_height <= min_row_height {
        return;
    }
    let grid_color = palette.background.weakest.text.scale_alpha(0.32);
    frame.fill_rectangle(
        Point::new(layout.sell_cluster.left, row_top),
        Size::new(layout.sell_cluster.width, min_row_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(
            layout.sell_cluster.left,
            row_top + cell_height - min_row_height,
        ),
        Size::new(layout.sell_cluster.width, min_row_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(layout.buy_cluster.left, row_top),
        Size::new(layout.buy_cluster.width, min_row_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(
            layout.buy_cluster.left,
            row_top + cell_height - min_row_height,
        ),
        Size::new(layout.buy_cluster.width, min_row_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(layout.sell_cluster.left, row_top),
        Size::new(min_row_height, cell_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(layout.sell_cluster.right() - min_row_height, row_top),
        Size::new(min_row_height, cell_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(layout.buy_cluster.left, row_top),
        Size::new(min_row_height, cell_height),
        grid_color,
    );
    frame.fill_rectangle(
        Point::new(layout.buy_cluster.right() - min_row_height, row_top),
        Size::new(min_row_height, cell_height),
        grid_color,
    );
}

fn draw_footprint_row_text(
    frame: &mut canvas::Frame,
    quantity: f64,
    position: Point,
    text_size: Option<f32>,
    palette: &Extended,
    align_x: iced::Alignment,
) {
    let Some(text_size) = text_size else {
        return;
    };

    draw_footprint_text(
        frame,
        abbr_large_numbers(quantity),
        position,
        text_size,
        palette.background.weakest.text,
        align_x,
    );
}

fn draw_footprint_text(
    frame: &mut canvas::Frame,
    content: String,
    position: Point,
    text_size: f32,
    color: iced::Color,
    align_x: iced::Alignment,
) {
    frame.fill_text(canvas::Text {
        content,
        position,
        color,
        size: text_size.into(),
        align_x: align_x.into(),
        align_y: iced::Alignment::Center.into(),
        font: style::AZERET_MONO,
        ..Default::default()
    });
}

fn footprint_volume_alpha(qty: f64, max_qty: f64) -> f32 {
    if max_qty > 0.0 {
        (qty / max_qty).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
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
    layout: FootprintColumnLayout,
    y_open: f32,
    y_high: f32,
    y_low: f32,
    y_close: f32,
    pixels: PixelGeometry,
    color: iced::Color,
) {
    if [y_open, y_high, y_low, y_close, pixels.origin().y]
        .into_iter()
        .any(|value| !value.is_finite())
    {
        return;
    }
    let body = pixels.snapped_y_span(y_open, y_close);
    let wick = pixels.snapped_y_span(y_high, y_low);
    let stroke_width = pixels.quantized_stroke_width(1.0).0;

    frame.stroke(
        &canvas::Path::rectangle(
            Point::new(layout.candle.left, body.start),
            Size::new(layout.candle_width(), body.size),
        ),
        canvas::Stroke::default()
            .with_width(stroke_width)
            .with_color(color),
    );
    frame.stroke(
        &canvas::Path::line(
            Point::new(layout.candle_center(), wick.start),
            Point::new(layout.candle_center(), wick.start + wick.size),
        ),
        canvas::Stroke::default()
            .with_width(stroke_width)
            .with_color(color.scale_alpha(0.6)),
    );
}
