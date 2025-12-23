use data::aggr::time::{DataPoint, TimeSeries};
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};
use iced::time::Instant;
use iced::widget::{center, column, shader};
use iced::{Element, Fill};

use crate::chart::Action;
use crate::widget::chart::heatmap::scene::Scene;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;

mod scene;

const DEFAULT_ROW_H_WORLD: f32 = 0.1;
const DEFAULT_COL_W_WORLD: f32 = 0.1;

const MIN_ROW_H_WORLD: f32 = 1e-4;
const MIN_CAMERA_SCALE: f32 = 1e-6;

// Trades (circles)
const TRADE_R_MIN_PX: f32 = 3.0;
const TRADE_R_MAX_PX: f32 = 25.0;
const TRADE_ALPHA: f32 = 0.8;

// Depth (rect alpha normalization)
const DEPTH_ALPHA_MIN: f32 = 0.05;
const DEPTH_ALPHA_MAX: f32 = 0.95;

// Zoom -> circle scaling
const ZOOM_REF_PX: f32 = 300.0;
const ZOOM_EXP: f32 = 0.15;
const ZOOM_FACTOR_MIN: f32 = 0.75;
const ZOOM_FACTOR_MAX: f32 = 1.5;

// Latest profile overlay (x > 0)
const PROFILE_COL_WIDTH_PX: f32 = 180.0;
const PROFILE_MIN_BAR_PX: f32 = 1.0;
const PROFILE_ALPHA: f32 = 0.8;

// Volume strip (bottom band)
const STRIP_HEIGHT_FRAC: f32 = 0.10;
const VOLUME_BUCKET_GAP_FRAC: f32 = 0.10;
const VOLUME_MIN_BAR_PX: f32 = 1.0;
const VOLUME_TOTAL_RGB: [f32; 3] = [0.7, 0.7, 0.7];
const VOLUME_TOTAL_ALPHA: f32 = 0.18;
const VOLUME_DELTA_ALPHA: f32 = 0.8;

#[derive(Debug, Clone, Copy)]
struct ViewWindow {
    // Derived time window
    aggr_time: u64,
    earliest: u64,
    latest_vis: u64,
    latest_bucket: i64,

    // Derived price window
    lowest: Price,
    highest: Price,
    row_h: f32,

    // Visible world bounds
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,

    // Camera scale (world->px)
    sx: f32,
    sy: f32,

    // Overlays
    profile_max_w_world: f32,
    strip_h_world: f32,
    strip_bottom_y: f32,

    // Circle scaling
    zoom_factor: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatmapPalette {
    pub bid_rgb: [f32; 3],
    pub ask_rgb: [f32; 3],
    pub buy_rgb: [f32; 3],
    pub sell_rgb: [f32; 3],
}

impl HeatmapPalette {
    pub fn from_theme(theme: &iced_core::Theme) -> Self {
        let bid = theme.extended_palette().success.strong.color;
        let ask = theme.extended_palette().danger.strong.color;

        let buy = theme.extended_palette().success.weak.color;
        let sell = theme.extended_palette().danger.weak.color;

        Self {
            bid_rgb: [bid.r, bid.g, bid.b],
            ask_rgb: [ask.r, ask.g, ask.b],
            buy_rgb: [buy.r, buy.g, buy.b],
            sell_rgb: [sell.r, sell.g, sell.b],
        }
    }
}

struct RealDataState {
    basis: Basis,
    step: PriceStep,
    ticker_info: TickerInfo,
    trades: TimeSeries<HeatmapDataPoint>,
    heatmap: HistoricalDepth,
    latest_time: u64,
    base_price: Price,
}

pub struct HeatmapShader {
    pub last_tick: Option<Instant>,
    scene: Scene,
    viewport: Option<[f32; 2]>,
    row_h: f32,
    column_world: f32,
    palette: Option<HeatmapPalette>,
    data: RealDataState,
}

#[derive(Debug, Clone)]
pub enum Message {
    BoundsChanged([f32; 2]),
    RowHeightChanged(f32),
    Tick(Instant),
    PanDeltaPx(iced::Vector),
    ZoomAt { factor: f32, cursor: iced::Point },
}

impl HeatmapShader {
    pub fn new(basis: Basis, tick_size: f32, ticker_info: TickerInfo) -> Self {
        let step = PriceStep::from_f32(tick_size);

        let heatmap = HistoricalDepth::new(ticker_info.min_qty.into(), step, basis);
        let trades = TimeSeries::<HeatmapDataPoint>::new(basis, step);

        Self {
            last_tick: None,
            scene: Scene::new(),
            viewport: None,
            row_h: DEFAULT_ROW_H_WORLD,
            column_world: DEFAULT_COL_W_WORLD,
            palette: None,
            data: RealDataState {
                basis,
                step,
                ticker_info,
                trades,
                heatmap,
                latest_time: 0,
                base_price: Price::from_units(0),
            },
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(viewport) => {
                self.viewport = Some(viewport);
                self.rebuild_instances();
            }
            Message::RowHeightChanged(h) => {
                self.row_h = h.max(0.0001);
                self.rebuild_instances();
            }
            Message::Tick(now) => {
                self.last_tick = Some(now);
            }
            Message::PanDeltaPx(delta_px) => {
                let dx_world = delta_px.x / self.scene.camera.scale[0];
                let dy_world = delta_px.y / self.scene.camera.scale[1];

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;

                self.rebuild_instances();
            }
            Message::ZoomAt { factor, cursor } => {
                let Some([vw, vh]) = self.viewport else {
                    return;
                };

                self.scene
                    .camera
                    .zoom_at_cursor(factor, cursor.x, cursor.y, vw, vh);

                self.rebuild_instances();
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let shader = shader(&self.scene).width(Fill).height(Fill);
        center(column![shader]).into()
    }

    pub fn insert_datapoint(
        &mut self,
        trades_buffer: &[Trade],
        depth_update_t: u64,
        depth: &Depth,
    ) {
        let state = &mut self.data;

        let aggr_time: u64 = match state.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return, // keep it simple for now
        };

        let rounded_t = (depth_update_t / aggr_time) * aggr_time;

        {
            let entry =
                state
                    .trades
                    .datapoints
                    .entry(rounded_t)
                    .or_insert_with(|| HeatmapDataPoint {
                        grouped_trades: Box::new([]),
                        buy_sell: (0.0, 0.0),
                    });

            for trade in trades_buffer {
                entry.add_trade(trade, state.step);
            }
        }

        state.heatmap.insert_latest_depth(depth, rounded_t);

        let mid = depth.mid_price().unwrap_or(state.base_price);
        state.base_price = mid.round_to_step(state.step);
        state.latest_time = rounded_t;

        self.rebuild_instances();
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);
    }

    fn clear_scene(&mut self) {
        self.scene.set_rectangles(Vec::new());
        self.scene.set_circles(Vec::new());
    }

    fn compute_view_window(
        &self,
        state: &RealDataState,
        vw_px: f32,
        vh_px: f32,
    ) -> Option<ViewWindow> {
        let aggr_time: u64 = match state.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return None,
        };

        if state.latest_time == 0 || aggr_time == 0 {
            return None;
        }

        // Camera semantics:
        // offset.x sits at the viewport RIGHT EDGE, offset.y at the vertical center.
        let sx = self.scene.camera.scale[0].max(MIN_CAMERA_SCALE);
        let sy = self.scene.camera.scale[1].max(MIN_CAMERA_SCALE);

        let x_max = self.scene.camera.right_edge(vw_px);
        let x_min = x_max - (vw_px / sx);

        // Visible time window from visible world-x window.
        let bucket_min = ((x_min / self.column_world).floor() as i64).saturating_sub(2);
        let bucket_max = ((x_max / self.column_world).ceil() as i64).saturating_add(2);

        let y_center = self.scene.camera.offset[1];
        let half_h_world = (vh_px / sy) * 0.5;
        let y_min = y_center - half_h_world;
        let y_max = y_center + half_h_world;

        let latest_t = state.latest_time as i128;
        let aggr_i = aggr_time as i128;

        let t_min_i = latest_t + (bucket_min as i128) * aggr_i;
        let t_max_i = latest_t + (bucket_max as i128) * aggr_i;

        let earliest = t_min_i.clamp(0, latest_t) as u64;
        let latest_vis = t_max_i.clamp(0, latest_t) as u64;

        if earliest >= latest_vis {
            return None;
        }

        // Price window from visible world-y.
        let row_h = self.row_h.max(MIN_ROW_H_WORLD);

        // dy_steps = -(y_world / row_h)
        let min_steps = (-(y_max) / row_h).floor() as i64;
        let max_steps = (-(y_min) / row_h).ceil() as i64;

        let lowest = state.base_price.add_steps(min_steps, state.step);
        let highest = state.base_price.add_steps(max_steps, state.step);

        // Mild zoom influence for circles
        let zoom = (self.scene.camera.scale[1] / ZOOM_REF_PX).max(MIN_CAMERA_SCALE);
        let zoom_factor = zoom.powf(ZOOM_EXP).clamp(ZOOM_FACTOR_MIN, ZOOM_FACTOR_MAX);

        let latest_bucket: i64 = (state.latest_time / aggr_time) as i64;

        // Latest profile overlay width: request a fixed pixel width, convert to world units,
        // clamp to whatever is actually visible for x>0.
        let visible_space_right_of_zero_world = (x_max - 0.0).max(0.0);
        let desired_profile_w_world = (PROFILE_COL_WIDTH_PX / sx).max(0.0);
        let profile_max_w_world = desired_profile_w_world.min(visible_space_right_of_zero_world);

        let strip_h_world: f32 = (vh_px * STRIP_HEIGHT_FRAC) / sy;
        let strip_bottom_y: f32 = y_max; // anchored to bottom visible bound

        Some(ViewWindow {
            aggr_time,
            earliest,
            latest_vis,
            latest_bucket,
            lowest,
            highest,
            row_h,
            x_min,
            x_max,
            y_min,
            y_max,
            sx,
            sy,
            profile_max_w_world,
            strip_h_world,
            strip_bottom_y,
            zoom_factor,
        })
    }

    fn max_depth_qty(state: &RealDataState, w: &ViewWindow) -> f32 {
        let mut max_qty = 0.0f32;

        for (_price, runs) in
            state
                .heatmap
                .iter_time_filtered(w.earliest, w.latest_vis, w.highest, w.lowest)
        {
            for run in runs {
                let run_start = run.start_time.max(w.earliest);
                let run_until = run.until_time.min(w.latest_vis);
                if run_until > run_start {
                    max_qty = max_qty.max(run.qty());
                }
            }
        }

        max_qty
    }

    fn max_trade_qty(state: &RealDataState, w: &ViewWindow) -> f32 {
        let mut max_qty = 0.0f32;

        for (_time, dp) in state.trades.datapoints.range(w.earliest..=w.latest_vis) {
            for tr in dp.grouped_trades.iter() {
                if tr.price < w.lowest || tr.price > w.highest {
                    continue;
                }
                max_qty = max_qty.max(tr.qty);
            }
        }

        max_qty
    }

    fn build_circles(
        &self,
        state: &RealDataState,
        w: &ViewWindow,
        palette: &HeatmapPalette,
        max_trade_qty: f32,
    ) -> Vec<CircleInstance> {
        let denom_trade = max_trade_qty.max(1e-12);

        let mut circles = Vec::new();

        for (time, dp) in state.trades.datapoints.range(w.earliest..=w.latest_vis) {
            let x_position = -(((state.latest_time as i128 - *time as i128) as f32)
                / (w.aggr_time as f32))
                * self.column_world;

            for trade in dp.grouped_trades.iter() {
                if trade.price < w.lowest || trade.price > w.highest {
                    continue;
                }

                let dy_steps = (trade.price.units - state.base_price.units) / state.step.units;
                let y_position = -((dy_steps as f32) * w.row_h);

                let t = (trade.qty / denom_trade).clamp(0.0, 1.0);
                let mut radius_px = TRADE_R_MIN_PX + t.sqrt() * (TRADE_R_MAX_PX - TRADE_R_MIN_PX);
                radius_px *= w.zoom_factor;

                let rgb = if trade.is_sell {
                    palette.sell_rgb
                } else {
                    palette.buy_rgb
                };

                circles.push(CircleInstance {
                    center: [x_position, y_position],
                    radius_px,
                    _pad: 0.0,
                    color: [rgb[0], rgb[1], rgb[2], TRADE_ALPHA],
                });
            }
        }

        circles
    }

    fn build_depth_rects(
        &self,
        state: &RealDataState,
        w: &ViewWindow,
        palette: &HeatmapPalette,
        max_depth_qty: f32,
    ) -> Vec<RectInstance> {
        let denom_depth = max_depth_qty.max(1e-12);
        let mut rects = Vec::new();

        for (price, runs) in
            state
                .heatmap
                .iter_time_filtered(w.earliest, w.latest_vis, w.highest, w.lowest)
        {
            let dy_steps = (price.units - state.base_price.units) / state.step.units;
            let y = -((dy_steps as f32) * w.row_h);

            for run in runs {
                let run_start = run.start_time.max(w.earliest);
                let run_until = run.until_time.min(w.latest_vis);
                if run_until <= run_start {
                    continue;
                }

                let start_bucket: i64 = (run_start / w.aggr_time) as i64;
                let end_bucket_excl: i64 = ((run_until + w.aggr_time - 1) / w.aggr_time) as i64;
                let end_bucket_excl = end_bucket_excl.min(w.latest_bucket);

                let mut x_left = -((w.latest_bucket - start_bucket) as f32) * self.column_world;
                let mut x_right = -((w.latest_bucket - end_bucket_excl) as f32) * self.column_world;
                if x_left > x_right {
                    std::mem::swap(&mut x_left, &mut x_right);
                }

                let x0 = x_left.clamp(w.x_min, w.x_max);
                let x1 = x_right.clamp(w.x_min, w.x_max);
                let width = (x1 - x0).max(0.0);
                if width <= 1e-6 {
                    continue;
                }

                let center_x = 0.5 * (x0 + x1);

                let a = (run.qty() / denom_depth).clamp(DEPTH_ALPHA_MIN, DEPTH_ALPHA_MAX);

                let rgb = if run.is_bid {
                    palette.bid_rgb
                } else {
                    palette.ask_rgb
                };

                rects.push(RectInstance {
                    position: [center_x, y],
                    size: [width, w.row_h],
                    color: [rgb[0], rgb[1], rgb[2], a],
                });
            }
        }

        rects
    }

    fn push_latest_profile_rects(
        &self,
        state: &RealDataState,
        w: &ViewWindow,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        // Find max qty among latest visible depth
        let mut max_latest_qty: f32 = 0.0;
        for (price, run) in state
            .heatmap
            .latest_order_runs(w.highest, w.lowest, state.latest_time)
        {
            if *price < w.lowest || *price > w.highest {
                continue;
            }
            max_latest_qty = max_latest_qty.max(run.qty());
        }

        if max_latest_qty <= 0.0 || w.profile_max_w_world <= 0.0 {
            return;
        }

        let min_bar_w_world: f32 = PROFILE_MIN_BAR_PX / w.sx; // ~N px

        for (price, run) in state
            .heatmap
            .latest_order_runs(w.highest, w.lowest, state.latest_time)
        {
            if *price < w.lowest || *price > w.highest {
                continue;
            }

            let dy_steps = (price.units - state.base_price.units) / state.step.units;
            let y = -((dy_steps as f32) * w.row_h);

            let t = (run.qty() / max_latest_qty).clamp(0.0, 1.0);
            let w_world = (t * w.profile_max_w_world).max(min_bar_w_world);

            // left edge at x=0, growing into x>0
            let center_x = 0.5 * w_world;

            let rgb = if run.is_bid {
                palette.bid_rgb
            } else {
                palette.ask_rgb
            };

            rects.push(RectInstance {
                position: [center_x, y],
                size: [w_world, w.row_h],
                color: [rgb[0], rgb[1], rgb[2], PROFILE_ALPHA],
            });
        }
    }

    fn push_volume_strip_rects(
        &self,
        state: &RealDataState,
        w: &ViewWindow,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        if w.strip_h_world <= 0.0 {
            return;
        }

        // Normalize bar heights by the max total volume in the *visible time range*
        let mut max_total_vol: f32 = 0.0;
        for (_time, dp) in state.trades.datapoints.range(w.earliest..=w.latest_vis) {
            let (buy, sell) = dp.buy_sell;
            max_total_vol = max_total_vol.max(buy + sell);
        }
        if max_total_vol <= 0.0 {
            return;
        }

        let denom = max_total_vol.max(1e-12);

        let bar_w: f32 = self.column_world * (1.0 - VOLUME_BUCKET_GAP_FRAC);
        let min_h_world: f32 = VOLUME_MIN_BAR_PX / w.sy;

        for (time, dp) in state.trades.datapoints.range(w.earliest..=w.latest_vis) {
            let x_right = -(((state.latest_time as i128 - *time as i128) as f32)
                / (w.aggr_time as f32))
                * self.column_world;

            let (buy, sell) = dp.buy_sell;
            let total = buy + sell;
            if total <= 0.0 {
                continue;
            }

            let total_h = ((total / denom) * w.strip_h_world).max(min_h_world);

            let delta = buy - sell;
            let delta_h = ((delta.abs() / denom) * w.strip_h_world)
                .min(total_h)
                .max(min_h_world);

            let total_center_y = w.strip_bottom_y - 0.5 * total_h;
            let delta_center_y = w.strip_bottom_y - 0.5 * delta_h;

            // Quick visibility cull in x
            if x_right + 0.5 * bar_w < w.x_min || x_right - 0.5 * bar_w > w.x_max {
                continue;
            }

            // Background total volume
            rects.push(RectInstance {
                position: [x_right, total_center_y],
                size: [bar_w, total_h],
                color: [
                    VOLUME_TOTAL_RGB[0],
                    VOLUME_TOTAL_RGB[1],
                    VOLUME_TOTAL_RGB[2],
                    VOLUME_TOTAL_ALPHA,
                ],
            });

            // Delta overlay
            let rgb = if delta >= 0.0 {
                palette.bid_rgb
            } else {
                palette.ask_rgb
            };
            rects.push(RectInstance {
                position: [x_right, delta_center_y],
                size: [bar_w, delta_h],
                color: [rgb[0], rgb[1], rgb[2], VOLUME_DELTA_ALPHA],
            });
        }
    }

    fn rebuild_instances(&mut self) {
        let Some(palette) = &self.palette else {
            self.clear_scene();
            return;
        };

        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };

        let state = &self.data;

        let Some(w) = self.compute_view_window(state, vw_px, vh_px) else {
            self.clear_scene();
            return;
        };

        let max_depth_qty = Self::max_depth_qty(state, &w);
        let max_trade_qty = Self::max_trade_qty(state, &w);

        if max_depth_qty <= 0.0 && max_trade_qty <= 0.0 {
            self.clear_scene();
            return;
        }

        let circles = self.build_circles(state, &w, palette, max_trade_qty);

        let mut rects = self.build_depth_rects(state, &w, palette, max_depth_qty);
        self.push_latest_profile_rects(state, &w, palette, &mut rects);
        self.push_volume_strip_rects(state, &w, palette, &mut rects);

        self.scene.set_rectangles(rects);
        self.scene.set_circles(circles);
    }

    pub fn tick_size(&self) -> f32 {
        self.data.step.to_f32_lossy()
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        if let Some(now) = now {
            self.last_tick = Some(now);
        }

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }

        None
    }
}
