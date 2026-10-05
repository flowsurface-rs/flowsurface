use super::composition::MarkKind;
use super::painting::PaintInputs;
use super::painting::PixelGeometry;
use super::scene::Scene;
use super::{CHAR_W, TEXT_SIZE};
use crate::style;
use data::chart::kline::footprint::{ClusterKind, KlineTrades};
use data::chart::kline::poc::NPoc;
use data::chart::kline::series::KlineSeries;
use data::chart::kline::viewport::{TickIndex, XAxis};
use data::chart::kline::{
    CandlestickColorSlot, CandlestickConfig, FootprintConfig, FootprintPocColorSlot,
};
use data::config::theme::ACCENT_COUNT;
use data::config::theme::{composite_color, contrast_ratio};
use data::util::abbr_large_numbers;
use exchange::UnixMs;
use exchange::unit::{Price, PriceStep, Qty};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};
use std::collections::BTreeMap;

pub(super) struct PixelRows {
    top: f32,
    height: f32,
    pub(super) bucket_height: f32,
    quantities: Vec<(Qty, Qty)>,
    touched: Vec<usize>,
}

impl PixelRows {
    pub(super) fn new(plot: Rectangle, pixels: PixelGeometry) -> Self {
        let pixel_ratio = pixels.ratio();
        let origin_y = pixels.origin().y;
        let top_physical = ((plot.y + origin_y) * pixel_ratio).floor() as i32;
        let bottom_physical = ((plot.y + plot.height + origin_y) * pixel_ratio).ceil() as i32;
        let bucket_count = bottom_physical.saturating_sub(top_physical) as usize;
        let bucket_height = 1.0 / pixel_ratio;
        Self {
            top: top_physical as f32 / pixel_ratio - origin_y,
            height: bucket_count as f32 / pixel_ratio,
            bucket_height,
            quantities: vec![(Qty::ZERO, Qty::ZERO); bucket_count],
            touched: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, y: f32, buy: Qty, sell: Qty) {
        let offset = y - self.top;
        let half_bucket = self.bucket_height * 0.5;
        if !offset.is_finite()
            || offset < -half_bucket
            || offset > self.height + half_bucket
            || self.quantities.is_empty()
        {
            return;
        }
        let index = (offset / self.bucket_height)
            .floor()
            .clamp(0.0, self.quantities.len().saturating_sub(1) as f32)
            as usize;
        if let Some(row) = self.quantities.get_mut(index) {
            let was_empty = row.0.is_zero() && row.1.is_zero();
            row.0 += buy;
            row.1 += sell;
            if was_empty && (!row.0.is_zero() || !row.1.is_zero()) {
                self.touched.push(index);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.touched.iter().all(|&index| {
            let (buy, sell) = self.quantities[index];
            buy.is_zero() && sell.is_zero()
        })
    }

    pub(super) fn max_qty(&self, kind: ClusterKind) -> Qty {
        self.touched
            .iter()
            .map(|&index| {
                let (buy, sell) = self.quantities[index];
                match kind {
                    ClusterKind::BidAsk | ClusterKind::Table => buy.max(sell),
                    ClusterKind::VolumeProfile => buy + sell,
                    ClusterKind::DeltaProfile => buy.abs_diff(sell),
                }
            })
            .max()
            .unwrap_or_default()
    }

    pub(super) fn rows(&self) -> impl Iterator<Item = (f32, Qty, Qty)> + '_ {
        let mut touched = self.touched.clone();
        touched.sort_unstable();
        touched.into_iter().filter_map(|index| {
            let (buy, sell) = self.quantities[index];
            (!buy.is_zero() || !sell.is_zero()).then_some((
                self.top + (index as f32 + 0.5) * self.bucket_height,
                buy,
                sell,
            ))
        })
    }
}

pub(super) fn row_height(
    scene: &Scene,
    price: Price,
    tick_size: PriceStep,
    anchor: Option<f32>,
) -> f32 {
    let current_y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
    let Some(next_price) = price.checked_add_steps(1, tick_size) else {
        return 0.0;
    };
    let next_y = scene.map_primary_plot_with_anchor_f32(next_price.to_f32_lossy(), anchor);
    (current_y - next_y).abs()
}

pub(super) fn coalesce_price_rows(
    scene: &Scene,
    tick_size: PriceStep,
    anchor: Option<f32>,
    pixels: PixelGeometry,
) -> bool {
    let pixel_ratio = pixels.ratio();
    let (low, high) = scene.primary_domain_value_range();
    [low, high].into_iter().all(|price| {
        let row_height = row_height(scene, price, tick_size, anchor);
        row_height.is_finite() && row_height * pixel_ratio < 1.0
    })
}

const FOOTPRINT_TEXT_HORIZONTAL_INSET_PX: f32 = 2.0;
const FOOTPRINT_TEXT_VERTICAL_INSET_PX: f32 = 1.0;
const FOOTPRINT_TEXT_SIZE_STEP_PX: f32 = 1.0;
const FOOTPRINT_TEXT_MIN_WIDTH_CHARS: usize = 4;
const FOOTPRINT_TEXT_MIN_SIZE_PX: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
enum FootprintTextMode {
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

    fn row_intersects_plot(
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

struct FootprintImbalance<'a> {
    footprint: &'a KlineTrades,
    price: Price,
    tick_size: PriceStep,
    threshold_percent: usize,
    ignore_zeros: bool,
    color: iced::Color,
}

impl<'a> FootprintImbalance<'a> {
    fn new(
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
struct FootprintColumnLayout {
    sell_cluster: FootprintLane,
    sell_marker: Option<FootprintLane>,
    candle: FootprintLane,
    buy_marker: Option<FootprintLane>,
    buy_cluster: FootprintLane,
    profile_cluster: FootprintLane,
}

impl FootprintColumnLayout {
    fn new(x_center: f32, width: f32, reserve_marker_lanes: bool) -> Self {
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

fn resolve_footprint_text_mode(
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

fn footprint_text_content_lengths(
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
    frame: &mut PaintInputs,
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

fn draw_footprint_cluster_row(
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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

fn draw_footprint_table_row(
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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

fn draw_footprint_candle(
    frame: &mut PaintInputs,
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

pub(super) struct FootprintRenderer<'a> {
    pub(super) series: &'a [KlineSeries],
    pub(super) candlestick: &'a CandlestickConfig,
    pub(super) footprint: &'a FootprintConfig,
}

impl FootprintRenderer<'_> {
    pub(super) fn candlestick_color(
        &self,
        bullish: bool,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let slot = if bullish {
            CandlestickColorSlot::Bullish
        } else {
            CandlestickColorSlot::Bearish
        };
        crate::widget::resolve_theme_color(
            self.candlestick.colors.color_for(slot),
            palette,
            generated_accents,
        )
        .scale_alpha(self.candlestick.opacity_ratio())
    }

    pub(super) fn fill_footprint_overlay(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        draw_clusters: bool,
        pixels: PixelGeometry,
    ) {
        let Some(base_series) = self.series.first() else {
            return;
        };
        let tick_size = base_series.tick_size();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let min_row_height = pixels.logical_width_from_physical(1);
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.x.min_unit && x_unit <= scene.x.max_unit;
        let coalesce = draw_clusters && coalesce_price_rows(scene, tick_size, anchor, pixels);

        let mut pixel_rows = BTreeMap::new();
        if coalesce {
            scene.for_each_bar(self.series, |series_index, index, _, _, point| {
                if series_index != 0 || point.footprint.trades.is_empty() {
                    return;
                }
                let mut rows = PixelRows::new(*scene.primary_plot(), pixels);
                for (price, group) in &point.footprint.trades {
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    rows.add(y, group.buy_qty, group.sell_qty);
                }
                if !rows.is_empty() {
                    pixel_rows.insert(index, rows);
                }
            });
        }
        let max_cluster_qty = if coalesce {
            pixel_rows
                .values()
                .map(|rows| rows.max_qty(self.footprint.cluster_kind))
                .max()
                .unwrap_or_default()
                .to_f64()
        } else if draw_clusters {
            let visible_point_range = scene.bar_units.first().map(|units| {
                let range = units.index_range(scene.x.min_unit, scene.x.max_unit);
                let (start, end) = (range.start, range.end);
                let end = end.min(units.values().len()).min(
                    base_series
                        .point_count()
                        .saturating_sub(units.start_index()),
                );
                units.global_range(start..end)
            });
            let (lowest, highest) = scene.primary_domain_value_range();
            visible_point_range
                .map(|point_range| {
                    base_series
                        .max_cluster_qty_in_point_range(
                            self.footprint.cluster_kind,
                            point_range,
                            lowest,
                            highest,
                        )
                        .to_f64()
                })
                .unwrap_or_default()
        } else {
            0.0
        };

        let primary_plot = *scene.primary_plot();
        let footprint_width =
            (scene.logical_bar_spacing() - FOOTPRINT_CELL_INSET_PX * 2.0).max(1.0);
        let imbalance = (self.footprint.imbalance.show && !coalesce).then(|| {
            (
                self.footprint.imbalance.threshold_percent,
                self.footprint.imbalance.ignore_zeros,
                crate::widget::resolve_theme_color(
                    self.footprint.imbalance.color,
                    palette,
                    generated_accents,
                ),
            )
        });
        let primary_plot_bottom = primary_plot.y + primary_plot.height;
        let mut minimum_row_height: Option<f32> = None;
        let mut maximum_content_chars = 0;
        if draw_clusters && !coalesce {
            scene.for_each_bar(self.series, |series_index, _, _series, x_unit, point| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let footprint = point.footprint;
                for (price, group) in &footprint.trades {
                    let row_height =
                        row_height(scene, *price, tick_size, anchor).max(min_row_height);
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    let cell_height = (row_height - min_row_height).max(min_row_height);
                    let row_top = y - cell_height * 0.5;
                    let row_bottom = row_top + cell_height;
                    if row_bottom < primary_plot.y || row_top > primary_plot_bottom {
                        continue;
                    }

                    minimum_row_height = Some(
                        minimum_row_height.map_or(row_height, |minimum| minimum.min(row_height)),
                    );

                    let [sell_chars, buy_chars] = footprint_text_content_lengths(
                        self.footprint.cluster_kind,
                        group.sell_qty.to_f64(),
                        group.buy_qty.to_f64(),
                    );
                    let row_max_chars = sell_chars.max(buy_chars);
                    if row_max_chars == 0 {
                        continue;
                    }

                    maximum_content_chars = maximum_content_chars.max(row_max_chars);
                }
            });
        }
        let text_mode = resolve_footprint_text_mode(
            footprint_width,
            imbalance.is_some(),
            minimum_row_height,
            maximum_content_chars,
        );
        let draw_candlesticks = self.candlestick.visible
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_));

        frame.with_clip(primary_plot, |frame| {
            scene.for_each_bar(
                self.series,
                |series_index, index, _series, x_unit, point| {
                    if series_index != 0 || !in_visible_range(x_unit) {
                        return;
                    }

                    let series_anchor = scene
                        .primary
                        .series_percent_anchors
                        .first()
                        .copied()
                        .flatten();
                    let y_open = scene.map_primary_plot_with_anchor_f32(
                        point.kline.open.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_high = scene.map_primary_plot_with_anchor_f32(
                        point.kline.high.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_low = scene.map_primary_plot_with_anchor_f32(
                        point.kline.low.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_close = scene.map_primary_plot_with_anchor_f32(
                        point.kline.close.to_f32_lossy(),
                        series_anchor,
                    );
                    let footprint = point.footprint;
                    let x_center = scene.map_x_plot(x_unit);
                    let layout = FootprintColumnLayout::new(
                        x_center,
                        footprint_width,
                        imbalance.is_some() && !footprint.trades.is_empty(),
                    );

                    if let Some(rows) = pixel_rows.get(&index) {
                        let scale = self.footprint.cluster_scaling.effective_qty(
                            max_cluster_qty,
                            rows.max_qty(self.footprint.cluster_kind),
                        );
                        for (y, buy, sell) in rows.rows() {
                            draw_footprint_cluster_row(
                                frame,
                                self.footprint.cluster_kind,
                                layout,
                                y,
                                rows.bucket_height,
                                min_row_height,
                                sell.to_f64(),
                                buy.to_f64(),
                                scale,
                                FootprintTextMode::Hidden,
                                palette,
                                None,
                            );
                        }
                    } else if draw_clusters && !coalesce {
                        let cluster_scale = self.footprint.cluster_scaling.effective_qty(
                            max_cluster_qty,
                            footprint.max_cluster_qty_all(self.footprint.cluster_kind),
                        );
                        for (price, group) in &footprint.trades {
                            let row_height =
                                row_height(scene, *price, tick_size, anchor).max(min_row_height);
                            let y = scene
                                .map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                            if !text_mode.row_intersects_plot(
                                y,
                                row_height,
                                min_row_height,
                                primary_plot,
                            ) {
                                continue;
                            }
                            draw_footprint_cluster_row(
                                frame,
                                self.footprint.cluster_kind,
                                layout,
                                y,
                                row_height,
                                min_row_height,
                                group.sell_qty.to_f64(),
                                group.buy_qty.to_f64(),
                                cluster_scale,
                                text_mode,
                                palette,
                                imbalance.map(|(threshold_percent, ignore_zeros, color)| {
                                    FootprintImbalance::new(
                                        footprint,
                                        *price,
                                        tick_size,
                                        threshold_percent,
                                        ignore_zeros,
                                        color,
                                    )
                                }),
                            );
                        }
                    }

                    if draw_candlesticks {
                        draw_footprint_candle(
                            frame,
                            layout,
                            y_open,
                            y_high,
                            y_low,
                            y_close,
                            pixels,
                            self.candlestick_color(
                                point.kline.close >= point.kline.open,
                                palette,
                                generated_accents,
                            ),
                        );
                    }
                },
            );
        });
    }

    pub(super) fn fill_footprint_poc_overlay(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if !self.footprint.visible {
            return;
        }

        let config = self.footprint.poc;
        if !config.show_poc && !config.show_npoc {
            return;
        }
        let Some(base_series) = self.series.first() else {
            return;
        };
        let point_count = base_series.point_count();
        let (earliest, latest) = match scene.x.axis {
            XAxis::Time { .. } => {
                let (Some(earliest), Some(latest)) = (
                    scene.x.axis.time_from_unit(scene.x.min_unit),
                    scene.x.axis.time_from_unit(scene.x.max_unit),
                ) else {
                    return;
                };
                (earliest, latest)
            }
            XAxis::Tick { .. } => {
                let to_forward_index = |unit: i64| {
                    scene
                        .x
                        .axis
                        .tick_from_unit(unit)
                        .map(|from_latest| {
                            point_count
                                .saturating_sub(1)
                                .saturating_sub(from_latest.as_u64() as usize)
                        })
                        .unwrap_or_default()
                };
                (
                    UnixMs::new(to_forward_index(scene.x.min_unit) as u64),
                    UnixMs::new(to_forward_index(scene.x.max_unit) as u64),
                )
            }
        };
        let lines = base_series.footprint_poc_lines(earliest, latest);
        if lines.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let tick_size = base_series.tick_size();
        let min_row_height = pixels.logical_width_from_physical(1);
        let naked_line_end = primary_plot.x + primary_plot.width;
        let npoc_color = crate::widget::resolve_theme_color(
            config.colors.color_for(FootprintPocColorSlot::Npoc),
            palette,
            generated_accents,
        )
        .scale_alpha(0.7);
        let poc_color = crate::widget::resolve_theme_color(
            config.colors.color_for(FootprintPocColorSlot::Poc),
            palette,
            generated_accents,
        )
        .scale_alpha(0.85);
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let x_for_time = |value: UnixMs| -> Option<f32> {
            scene
                .x
                .axis
                .unit_from_time_f32(value)
                .map(|unit| scene.map_x_plot_f32(unit))
        };
        let x_for_tick_from_latest = |value: UnixMs| -> Option<f32> {
            Some(
                scene
                    .x
                    .axis
                    .unit_from_tick(TickIndex::from_u64(value.as_u64())) as f32,
            )
            .map(|unit| scene.map_x_plot_f32(unit))
        };
        let x_for_tick_start = |value: UnixMs| -> Option<f32> {
            let index = usize::try_from(value.as_u64()).ok()?;
            let from_latest = point_count.checked_sub(index.checked_add(1)?)?;
            x_for_tick_from_latest(UnixMs::new(from_latest as u64))
        };
        frame.with_clip(primary_plot, |frame| {
            for line in lines {
                let start_x = match scene.x.axis {
                    XAxis::Time { .. } => x_for_time(line.start),
                    XAxis::Tick { .. } => x_for_tick_start(line.start),
                };
                let Some(start_x) = start_x else {
                    continue;
                };
                let (end_x, color) = match line.poc.status {
                    NPoc::Naked if config.show_npoc => (naked_line_end, npoc_color),
                    NPoc::Filled { at } if config.show_poc => {
                        let x = match scene.x.axis {
                            XAxis::Time { .. } => x_for_time(UnixMs::new(at)),
                            XAxis::Tick { .. } => x_for_tick_from_latest(UnixMs::new(at)),
                        };
                        (x.unwrap_or(naked_line_end), poc_color)
                    }
                    NPoc::Filled { .. } => continue,
                    NPoc::None => continue,
                    _ => continue,
                };
                if !start_x.is_finite() || !end_x.is_finite() {
                    continue;
                }

                let poc_y =
                    scene.map_primary_plot_with_anchor_f32(line.poc.price.to_f32_lossy(), anchor);
                if !poc_y.is_finite() {
                    continue;
                }
                let poc_price = line.poc.price;
                let Some(line) = pixels.positive_x_span(start_x, end_x) else {
                    continue;
                };
                let raw_line_height = row_height(scene, poc_price, tick_size, anchor)
                    .abs()
                    .max(min_row_height)
                    .min(1.0_f32.max(min_row_height));
                let line_span = pixels
                    .snapped_y_span(poc_y - raw_line_height * 0.5, poc_y + raw_line_height * 0.5);
                frame.fill_rectangle(
                    Point::new(line.start, line_span.start),
                    Size::new(line.size, line_span.size),
                    color,
                );
            }
        });
    }
}

const FOOTPRINT_CELL_INSET_PX: f32 = 2.0;
