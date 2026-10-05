use super::super::interaction::LayerContext;
use super::super::painting::PaintInputs;
use super::LayerPaintPhase;
use crate::widget::chart::kline::footprint::row_height;
use crate::widget::chart::kline::painting::PixelGeometry;
use crate::widget::chart::kline::scene::XAxis;
use crate::widget::resolve_theme_color;
use data::chart::Basis;
use data::chart::kline::{BigTradesConfig, series::BigTradeEntry};
use data::config::theme::ACCENT_COUNT;
use iced::Point;
use iced::theme::palette::Extended;
use iced::widget::canvas;

const MAX_BUBBLES: usize = 2_048;
const MIN_ROW_PX: f32 = 16.0;
const MIN_COLUMN_PX: f32 = 16.0;
const MAX_RADIUS_PX: f32 = 24.0;
const MIN_RADIUS_PX: f32 = 3.0;

#[derive(Default)]
struct Cell {
    notional: f64,
    count: u32,
    anchor: Option<(BigTradeEntry, i64)>,
}

impl Cell {
    fn add(&mut self, entry: BigTradeEntry, x_unit: i64) {
        self.notional += entry.notional;
        self.count = self.count.saturating_add(1);
        if self
            .anchor
            .is_none_or(|(anchor, _)| entry.notional > anchor.notional)
        {
            self.anchor = Some((entry, x_unit));
        }
    }
}

pub(crate) struct BigTradesObject {
    config: BigTradesConfig,
    enabled: bool,
}

impl BigTradesObject {
    pub(crate) fn new(config: BigTradesConfig, enabled: bool) -> Self {
        Self { config, enabled }
    }

    pub(super) fn paint(
        &self,
        phase: LayerPaintPhase,
        context: LayerContext<'_>,
        frame: &mut PaintInputs,
        palette: &Extended,
        accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if phase == LayerPaintPhase::AfterIndicators {
            self.fill(frame, context, palette, accents, pixels);
        }
    }

    fn fill(
        &self,
        frame: &mut PaintInputs,
        context: LayerContext<'_>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if !self.enabled {
            return;
        }
        let Some(series) = context.coordinates.series.first() else {
            return;
        };
        let scene = context.scene;
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let plot = *scene.primary_plot();

        let bar_spacing = scene.logical_bar_spacing();
        let col_merge = (MIN_COLUMN_PX / bar_spacing).ceil().max(1.0) as i64;
        let tick_size = series.tick_size();
        let (low, high) = scene.primary_domain_value_range();
        let target_rows = (plot.height / MIN_ROW_PX).floor().max(1.0) as usize;
        let row_step = tick_size
            .nested_coarsened_to_fit(low, high, target_rows)
            .unwrap_or(tick_size);
        let cell_width_px = bar_spacing * col_merge as f32;
        let cell_height_px = row_height(scene, low, row_step, anchor);
        let max_radius = (0.25 * cell_width_px)
            .min(0.5 * cell_height_px)
            .clamp(MIN_RADIUS_PX, MAX_RADIUS_PX);
        let static_radius = (0.5 * row_height(scene, low, tick_size, anchor)).max(MIN_RADIUS_PX);

        let mut groups = rustc_hash::FxHashMap::<(i64, i64, bool), Cell>::default();

        match (context.coordinates.basis, scene.x.axis) {
            (Basis::Time(timeframe), XAxis::Time { .. }) => {
                let Some(window_start) = scene.x.axis.time_from_unit(scene.x.min_unit) else {
                    return;
                };
                let Some(window_end) = scene
                    .x
                    .axis
                    .time_from_unit(scene.x.max_unit.saturating_add(1))
                else {
                    return;
                };
                for entry in series.big_trades_in_time_range(window_start, window_end) {
                    let x_unit = scene.x.axis.unit_from_time(entry.time.floor_to(timeframe));
                    if x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                        continue;
                    }
                    let price = entry.price.round_to_step(tick_size);
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    if !y.is_finite() || y < plot.y || y > plot.y + plot.height {
                        continue;
                    }
                    let row = price.round_to_step(row_step);
                    groups
                        .entry((x_unit.div_euclid(col_merge), row.units, entry.is_sell))
                        .or_default()
                        .add(*entry, x_unit);
                }
            }
            (Basis::Tick(tick_count), XAxis::Tick { .. }) => {
                let point_count = series.point_count();
                let tick_count = usize::from(tick_count.0).max(1);
                let start_bar = context
                    .coordinates
                    .bar_index_at_or_before_unit(scene, 0, scene.x.min_unit)
                    .unwrap_or_default();
                let end_bar = context
                    .coordinates
                    .bar_index_at_or_before_unit(scene, 0, scene.x.max_unit)
                    .unwrap_or_else(|| point_count.saturating_sub(1))
                    .min(point_count.saturating_sub(1));
                let lo = start_bar.saturating_mul(tick_count);
                let hi = end_bar.saturating_add(1).saturating_mul(tick_count);
                for entry in series.big_trades_in_index_range(lo, hi) {
                    let bar_index = entry.index / tick_count;
                    let from_latest =
                        point_count.saturating_sub(1).saturating_sub(bar_index) as u64;
                    let x_unit = scene
                        .x
                        .axis
                        .unit_from_tick(super::super::scene::TickIndex::from_u64(from_latest));
                    if x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                        continue;
                    }
                    let price = entry.price.round_to_step(tick_size);
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    if !y.is_finite() || y < plot.y || y > plot.y + plot.height {
                        continue;
                    }
                    let row = price.round_to_step(row_step);
                    let cell_x = (bar_index as i64).div_euclid(col_merge);
                    groups
                        .entry((cell_x, row.units, entry.is_sell))
                        .or_default()
                        .add(*entry, x_unit);
                }
            }
            _ => return,
        }

        if groups.is_empty() {
            return;
        }
        let max_notional = groups
            .values()
            .map(|cell| cell.notional)
            .fold(0.0_f64, f64::max);
        if max_notional <= 0.0 {
            return;
        }
        let buy_color = resolve_theme_color(self.config.buy_color, palette, generated_accents);
        let sell_color = resolve_theme_color(self.config.sell_color, palette, generated_accents);
        let stroke_width = pixels.quantized_stroke_width(1.0).0;

        let mut groups = groups.into_iter().collect::<Vec<_>>();
        if groups.len() > MAX_BUBBLES {
            groups.sort_unstable_by(|left, right| right.1.notional.total_cmp(&left.1.notional));
            groups.truncate(MAX_BUBBLES);
        }

        frame.with_clip(plot, |frame| {
            for ((_, _, is_sell), cell) in groups {
                let Some((entry, x_unit)) = cell.anchor else {
                    continue;
                };
                let x = scene.map_x_plot_f32(x_unit as f32);
                let price = entry.price.round_to_step(tick_size);
                let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                let y = y.clamp(plot.y, plot.y + plot.height);
                let radius = if let Some(scale) = self.config.radius_scale {
                    let normalized = (cell.notional / max_notional).sqrt().clamp(0.0, 1.0) as f32;
                    max_radius * (0.4 + 0.6 * normalized) * (scale.clamp(10, 200) as f32 / 100.0)
                } else {
                    static_radius
                };
                let side_offset = if is_sell { -radius } else { radius };
                let center = Point::new(x + side_offset, y);
                let color = if is_sell { sell_color } else { buy_color };
                frame.fill(
                    &canvas::Path::circle(center, radius),
                    color.scale_alpha(0.8),
                );
                for ring in 1..cell.count.min(3) {
                    frame.stroke(
                        &canvas::Path::circle(center, radius + 2.0 * ring as f32),
                        canvas::Stroke::default()
                            .with_color(color)
                            .with_width(stroke_width),
                    );
                }
            }
        });
    }
}
