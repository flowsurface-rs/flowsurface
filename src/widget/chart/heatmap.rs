use data::aggr::time::{DataPoint, TimeSeries};
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};
use iced::time::Instant;
use iced::widget::{Canvas, Space, column, container, mouse_area, row, rule, shader};
use iced::{Element, Fill, Length, padding};
use rustc_hash::FxHashMap;

use crate::chart::Action;
use crate::style::{self};
use crate::widget::chart::heatmap::scale::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::scale::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::scene::Scene;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;

mod scale;
mod scene;

const TEXT_SIZE: f32 = 12.0;

const DEFAULT_ROW_H_WORLD: f32 = 0.1;
const DEFAULT_COL_W_WORLD: f32 = 0.1;

const MIN_CAMERA_SCALE: f32 = 1e-4;

const DEPTH_MIN_ROW_PX: f32 = 1.25;
const MAX_STEPS_PER_Y_BIN: i64 = 2048;

// Trades (circles)
const TRADE_R_MIN_PX: f32 = 2.0;
const TRADE_R_MAX_PX: f32 = 25.0;
const TRADE_ALPHA: f32 = 0.8;

// Depth (rect alpha normalization)
const DEPTH_ALPHA_MIN: f32 = 0.01;
const DEPTH_ALPHA_MAX: f32 = 0.99;

// Zoom -> circle scaling
const ZOOM_REF_PX: f32 = 100.0;
const ZOOM_EXP: f32 = 0.15;
const ZOOM_FACTOR_MIN: f32 = 0.75;
const ZOOM_FACTOR_MAX: f32 = 1.5;

// Latest profile overlay (x > 0)
const PROFILE_COL_WIDTH_PX: f32 = 180.0;
const PROFILE_MIN_BAR_PX: f32 = 1.0;
const PROFILE_ALPHA: f32 = 0.8;

// Volume strip
const STRIP_HEIGHT_FRAC: f32 = 0.10;
const VOLUME_BUCKET_GAP_FRAC: f32 = 0.10;
const VOLUME_MIN_BAR_PX: f32 = 1.0; // min bar height in px
const VOLUME_MIN_BAR_W_PX: f32 = 1.25; // min bar width in px (for x-binning)
const MAX_COLS_PER_X_BIN: i64 = 4096;
const VOLUME_TOTAL_RGB: [f32; 3] = [0.7, 0.7, 0.7];
const VOLUME_TOTAL_ALPHA: f32 = 0.18;
const VOLUME_DELTA_ALPHA: f32 = 0.8;

const MIN_ROW_H_WORLD: f32 = 0.01;
const MAX_ROW_H_WORLD: f32 = 10.;

const MIN_COL_W_WORLD: f32 = 0.01;
const MAX_COL_W_WORLD: f32 = 10.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DepthRectKey {
    is_bid: bool,
    y_bin: i64,
    start_bucket: i64,
    end_bucket_excl: i64,
}

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

    // Camera scale (world->px)
    sx: f32,
    sy: f32,

    // Overlays
    profile_max_w_world: f32,
    strip_h_world: f32,
    strip_bottom_y: f32,

    // Circle scaling
    zoom_factor: f32,

    // Y downsampling
    steps_per_y_bin: i64,
    y_bin_h_world: f32,
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

#[derive(Debug, Clone)]
pub enum Message {
    BoundsChanged([f32; 2]),
    Tick(Instant),
    PanDeltaPx(iced::Vector),
    ZoomAt {
        factor: f32,
        cursor: iced::Point,
    },
    ZoomRowHeightAt {
        factor: f32,
        cursor_y: f32,
        viewport_h: f32,
    },
    ZoomColumnWorldAt {
        factor: f32,
        cursor_x: f32,
        viewport_w: f32,
    },
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
    depth_acc: FxHashMap<DepthRectKey, f32>,
    profile_bid_acc: Vec<f32>,
    profile_ask_acc: Vec<f32>,
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

            depth_acc: FxHashMap::default(),
            profile_bid_acc: Vec::new(),
            profile_ask_acc: Vec::new(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(viewport) => {
                self.viewport = Some(viewport);
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
            Message::ZoomRowHeightAt {
                factor,
                cursor_y,
                viewport_h,
            } => {
                self.zoom_row_h_at(factor, cursor_y, viewport_h);
                self.rebuild_instances();
            }
            Message::ZoomColumnWorldAt {
                factor,
                cursor_x,
                viewport_w,
            } => {
                self.zoom_column_world_at(factor, cursor_x, viewport_w);
                self.rebuild_instances();
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let aggr_time = match self.data.basis {
            Basis::Time(interval) => Some(u64::from(interval)),
            Basis::Tick(_) => None,
        };

        let x_axis_label = Canvas::new(AxisXLabelCanvas {
            latest_time: self.data.latest_time,
            aggr_time,
            column_world: self.column_world,
            cam_offset_x: self.scene.camera.offset[0],
            cam_sx: self.scene.camera.scale[0].max(MIN_CAMERA_SCALE),
            cam_right_pad_frac: self.scene.camera.right_pad_frac,
        })
        .width(Fill)
        .height(iced::Length::Fixed(28.));

        let y_labels_width: Length = {
            let precision = self.data.ticker_info.min_ticksize;

            let value = self.data.base_price.to_string(precision);
            let width = (value.len() as f32 * TEXT_SIZE * 0.8).max(72.0);

            Length::Fixed(width.ceil())
        };

        let content: Element<_> = {
            let y_axis_label = Canvas::new(AxisYLabelCanvas {
                base_price: self.data.base_price,
                step: self.data.step,
                row_h: self.row_h,
                cam_offset_y: self.scene.camera.offset[1],
                cam_sy: self.scene.camera.scale[1].max(MIN_CAMERA_SCALE),
            })
            .width(Fill)
            .height(Fill);

            row![
                shader(&self.scene)
                    .width(Fill)
                    .height(Fill)
                    .width(Length::FillPortion(10))
                    .height(Length::FillPortion(120)),
                rule::vertical(1).style(style::split_ruler),
                container(mouse_area(y_axis_label))
                    .width(y_labels_width)
                    .height(Length::FillPortion(120))
            ]
            .into()
        };

        column![
            content,
            rule::horizontal(1).style(style::split_ruler),
            row![
                container(mouse_area(x_axis_label))
                    .width(Length::FillPortion(10))
                    .height(Length::Fixed(26.0)),
                Space::new()
                    .width(y_labels_width)
                    .height(Length::Fixed(26.0))
            ]
        ]
        .padding(padding::left(1).right(1).bottom(1))
        .into()
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

    fn compute_view_window(&self, vw_px: f32, vh_px: f32) -> Option<ViewWindow> {
        let aggr_time: u64 = match self.data.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return None,
        };

        if self.data.latest_time == 0 || aggr_time == 0 {
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

        let latest_t = self.data.latest_time as i128;
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

        let lowest = self.data.base_price.add_steps(min_steps, self.data.step);
        let highest = self.data.base_price.add_steps(max_steps, self.data.step);

        // Mild zoom influence for circles
        let zoom = (self.scene.camera.scale[1] / ZOOM_REF_PX).max(MIN_CAMERA_SCALE);
        let zoom_factor = zoom.powf(ZOOM_EXP).clamp(ZOOM_FACTOR_MIN, ZOOM_FACTOR_MAX);

        let latest_bucket: i64 = (self.data.latest_time / aggr_time) as i64;

        // Latest profile overlay width: request a fixed pixel width, convert to world units,
        // clamp to whatever is actually visible for x>0.
        let visible_space_right_of_zero_world = (x_max - 0.0).max(0.0);
        let desired_profile_w_world = (PROFILE_COL_WIDTH_PX / sx).max(0.0);
        let profile_max_w_world = desired_profile_w_world.min(visible_space_right_of_zero_world);

        let strip_h_world: f32 = (vh_px * STRIP_HEIGHT_FRAC) / sy;
        let strip_bottom_y: f32 = y_max; // anchored to bottom visible bound

        // Y downsampling: ensure each rendered "row" is at least ~N pixels tall.
        let px_per_step = row_h * sy;
        let mut steps_per_y_bin: i64 = 1;
        if px_per_step.is_finite() && px_per_step > 0.0 {
            steps_per_y_bin = (DEPTH_MIN_ROW_PX / px_per_step).ceil() as i64;
            steps_per_y_bin = steps_per_y_bin.clamp(1, MAX_STEPS_PER_Y_BIN);
        }
        let y_bin_h_world = row_h * steps_per_y_bin as f32;

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
            sx,
            sy,
            profile_max_w_world,
            strip_h_world,
            strip_bottom_y,
            zoom_factor,
            steps_per_y_bin,
            y_bin_h_world,
        })
    }

    #[inline]
    fn y_bin_for_steps(dy_steps: i64, steps_per_y_bin: i64) -> i64 {
        dy_steps.div_euclid(steps_per_y_bin.max(1))
    }

    #[inline]
    fn y_center_for_bin(y_bin: i64, w: &ViewWindow) -> f32 {
        // Bin spans [y_bin*steps, (y_bin+1)*steps), center at (y_bin+0.5)*steps
        let center_steps = (y_bin as f32 + 0.5) * (w.steps_per_y_bin as f32);
        -(center_steps * w.row_h)
    }

    fn build_circles(&self, w: &ViewWindow, max_trade_qty: f32) -> Vec<CircleInstance> {
        let Some(palette) = &self.palette else {
            return Vec::new();
        };

        let state = &self.data;

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

                let y_position = if w.steps_per_y_bin <= 1 {
                    -((dy_steps as f32 + 0.5) * w.row_h)
                } else {
                    let y_bin = Self::y_bin_for_steps(dy_steps, w.steps_per_y_bin);
                    Self::y_center_for_bin(y_bin, w)
                };

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

    fn build_depth_rects(&mut self, w: &ViewWindow, max_depth_qty: f32) -> Vec<RectInstance> {
        let Some(palette) = &self.palette else {
            return Vec::new();
        };

        let state = &self.data;

        // Reuse allocated hash-map storage across rebuilds
        self.depth_acc.clear();

        let mut max_binned: f32 = 0.0;

        for (price, runs) in
            state
                .heatmap
                .iter_time_filtered(w.earliest, w.latest_vis, w.highest, w.lowest)
        {
            let dy_steps = (price.units - state.base_price.units) / state.step.units;
            let y_bin = Self::y_bin_for_steps(dy_steps, w.steps_per_y_bin);

            for run in runs {
                let run_start = run.start_time.max(w.earliest);
                let run_until = run.until_time.min(w.latest_vis);
                if run_until <= run_start {
                    continue;
                }

                let start_bucket = (run_start / w.aggr_time) as i64;
                let end_bucket_excl = run_until.div_ceil(w.aggr_time) as i64;
                let end_bucket_excl = end_bucket_excl.min(w.latest_bucket);

                if end_bucket_excl <= start_bucket {
                    continue;
                }

                let key = DepthRectKey {
                    is_bid: run.is_bid,
                    y_bin,
                    start_bucket,
                    end_bucket_excl,
                };

                let e = self.depth_acc.entry(key).or_insert(0.0);
                *e += run.qty();
                max_binned = max_binned.max(*e);
            }
        }

        let denom_depth = max_binned.max(max_depth_qty).max(1e-12);

        let mut rects = Vec::with_capacity(self.depth_acc.len());

        for (key, qty_sum) in self.depth_acc.drain() {
            let mut x_left = -((w.latest_bucket - key.start_bucket) as f32) * self.column_world;
            let mut x_right = -((w.latest_bucket - key.end_bucket_excl) as f32) * self.column_world;
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
            let y = Self::y_center_for_bin(key.y_bin, w);

            let a = (qty_sum / denom_depth).clamp(DEPTH_ALPHA_MIN, DEPTH_ALPHA_MAX);

            let rgb = if key.is_bid {
                palette.bid_rgb
            } else {
                palette.ask_rgb
            };

            rects.push(RectInstance {
                position: [center_x, y],
                size: [width, w.y_bin_h_world],
                color: [rgb[0], rgb[1], rgb[2], a],
            });
        }

        rects
    }

    fn push_latest_profile_rects(&mut self, w: &ViewWindow, rects: &mut Vec<RectInstance>) {
        let Some(palette) = &self.palette else {
            return;
        };

        if w.profile_max_w_world <= 0.0 {
            return;
        }

        let state = &self.data;

        let min_steps = (w.lowest.units - state.base_price.units) / state.step.units;
        let max_steps = (w.highest.units - state.base_price.units) / state.step.units;

        let min_y_bin = Self::y_bin_for_steps(min_steps, w.steps_per_y_bin);
        let max_y_bin = Self::y_bin_for_steps(max_steps, w.steps_per_y_bin);

        if max_y_bin < min_y_bin {
            return;
        }

        let len = (max_y_bin - min_y_bin + 1) as usize;

        self.profile_bid_acc.resize(len, 0.0);
        self.profile_ask_acc.resize(len, 0.0);
        self.profile_bid_acc[..].fill(0.0);
        self.profile_ask_acc[..].fill(0.0);

        let mut max_latest_qty: f32 = 0.0;

        for (price, run) in state
            .heatmap
            .latest_order_runs(w.highest, w.lowest, state.latest_time)
        {
            if *price < w.lowest || *price > w.highest {
                continue;
            }

            let dy_steps = (price.units - state.base_price.units) / state.step.units;
            let y_bin = Self::y_bin_for_steps(dy_steps, w.steps_per_y_bin);
            let idx = (y_bin - min_y_bin) as usize;

            let v = if run.is_bid {
                &mut self.profile_bid_acc[idx]
            } else {
                &mut self.profile_ask_acc[idx]
            };

            *v += run.qty();
            max_latest_qty = max_latest_qty.max(*v);
        }

        if max_latest_qty <= 0.0 {
            return;
        }

        let min_bar_w_world: f32 = PROFILE_MIN_BAR_PX / w.sx; // ~N px

        for i in 0..len {
            let y_bin = min_y_bin + i as i64;
            let y = Self::y_center_for_bin(y_bin, w);

            for (is_bid, qty_sum) in [
                (true, self.profile_bid_acc[i]),
                (false, self.profile_ask_acc[i]),
            ] {
                if qty_sum <= 0.0 {
                    continue;
                }

                let t = (qty_sum / max_latest_qty).clamp(0.0, 1.0);
                let w_world = (t * w.profile_max_w_world).max(min_bar_w_world);

                // left edge at x=0, growing into x>0
                let center_x = 0.5 * w_world;

                let rgb = if is_bid {
                    palette.bid_rgb
                } else {
                    palette.ask_rgb
                };

                rects.push(RectInstance {
                    position: [center_x, y],
                    size: [w_world, w.y_bin_h_world],
                    color: [rgb[0], rgb[1], rgb[2], PROFILE_ALPHA],
                });
            }
        }
    }

    fn push_volume_strip_rects(&self, w: &ViewWindow, rects: &mut Vec<RectInstance>) {
        let Some(palette) = &self.palette else {
            return;
        };

        if w.strip_h_world <= 0.0 {
            return;
        }

        let state = &self.data;

        // X downsampling: ensure each rendered bar is at least ~N pixels wide.
        let px_per_col = self.column_world * w.sx;
        let px_per_drawn_col = px_per_col * (1.0 - VOLUME_BUCKET_GAP_FRAC);
        let mut cols_per_x_bin: i64 = 1;
        if px_per_drawn_col.is_finite() && px_per_drawn_col > 0.0 {
            cols_per_x_bin = (VOLUME_MIN_BAR_W_PX / px_per_drawn_col).ceil() as i64;
            cols_per_x_bin = cols_per_x_bin.clamp(1, MAX_COLS_PER_X_BIN);
        }

        let start_bucket_vis = (w.earliest / w.aggr_time) as i64;
        let end_bucket_vis = (w.latest_vis / w.aggr_time) as i64;

        let min_x_bin = start_bucket_vis.div_euclid(cols_per_x_bin.max(1));
        let max_x_bin = end_bucket_vis.div_euclid(cols_per_x_bin.max(1));

        if max_x_bin < min_x_bin {
            return;
        }

        let bins_len = (max_x_bin - min_x_bin + 1) as usize;

        let mut acc: Vec<(f32, f32)> = vec![(0.0, 0.0); bins_len];

        for (time, dp) in state.trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*time / w.aggr_time) as i64;
            let x_bin = bucket.div_euclid(cols_per_x_bin.max(1));
            let idx = (x_bin - min_x_bin) as usize;

            let (buy, sell) = dp.buy_sell;
            let e = &mut acc[idx];
            e.0 += buy;
            e.1 += sell;
        }

        let mut max_total_vol: f32 = 0.0;
        for (buy, sell) in acc.iter() {
            max_total_vol = max_total_vol.max(*buy + *sell);
        }
        if max_total_vol <= 0.0 {
            return;
        }
        let denom = max_total_vol.max(1e-12);

        let min_h_world: f32 = VOLUME_MIN_BAR_PX / w.sy;
        let min_w_world: f32 = VOLUME_MIN_BAR_W_PX / w.sx;

        for (i, (buy, sell)) in acc.into_iter().enumerate() {
            let total = buy + sell;
            if total <= 0.0 {
                continue;
            }

            let x_bin = min_x_bin + i as i64;

            let delta = buy - sell;

            let total_h = ((total / denom) * w.strip_h_world).max(min_h_world);
            let delta_h = ((delta.abs() / denom) * w.strip_h_world)
                .min(total_h)
                .max(min_h_world);

            let total_center_y = w.strip_bottom_y - 0.5 * total_h;
            let delta_center_y = w.strip_bottom_y - 0.5 * delta_h;

            // Bin span in buckets -> world-x span
            let start_bucket = x_bin * cols_per_x_bin;
            let mut end_bucket_excl = start_bucket + cols_per_x_bin;
            end_bucket_excl = end_bucket_excl.min(w.latest_bucket + 1);

            if end_bucket_excl <= start_bucket {
                continue;
            }

            let mut x_left = -((w.latest_bucket - start_bucket) as f32) * self.column_world;
            let mut x_right = -((w.latest_bucket - end_bucket_excl) as f32) * self.column_world;

            if x_left > x_right {
                std::mem::swap(&mut x_left, &mut x_right);
            }

            let bin_w_world = (x_right - x_left).abs();
            let bar_w_world = (bin_w_world * (1.0 - VOLUME_BUCKET_GAP_FRAC)).max(min_w_world);

            let center_x = x_left;

            if center_x + 0.5 * bar_w_world < w.x_min || center_x - 0.5 * bar_w_world > w.x_max {
                continue;
            }

            rects.push(RectInstance {
                position: [center_x, total_center_y],
                size: [bar_w_world, total_h],
                color: [
                    VOLUME_TOTAL_RGB[0],
                    VOLUME_TOTAL_RGB[1],
                    VOLUME_TOTAL_RGB[2],
                    VOLUME_TOTAL_ALPHA,
                ],
            });

            let rgb = if delta >= 0.0 {
                palette.bid_rgb
            } else {
                palette.ask_rgb
            };
            rects.push(RectInstance {
                position: [center_x, delta_center_y],
                size: [bar_w_world, delta_h],
                color: [rgb[0], rgb[1], rgb[2], VOLUME_DELTA_ALPHA],
            });
        }
    }

    fn rebuild_instances(&mut self) {
        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };

        let Some(w) = self.compute_view_window(vw_px, vh_px) else {
            self.clear_scene();
            return;
        };

        let max_depth_qty = self.max_depth_qty(&w);
        let max_trade_qty = self.max_trade_qty(&w);

        if max_depth_qty <= 0.0 && max_trade_qty <= 0.0 {
            self.clear_scene();
            return;
        }

        let circles = self.build_circles(&w, max_trade_qty);

        let mut rects = self.build_depth_rects(&w, max_depth_qty);
        self.push_latest_profile_rects(&w, &mut rects);
        self.push_volume_strip_rects(&w, &mut rects);
        self.scene.set_rectangles(rects);
        self.scene.set_circles(circles);
    }

    fn max_depth_qty(&self, w: &ViewWindow) -> f32 {
        let mut max_qty = 0.0f32;

        for (_price, runs) in
            self.data
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

    fn max_trade_qty(&self, w: &ViewWindow) -> f32 {
        let mut max_qty = 0.0f32;

        for (_time, dp) in self.data.trades.datapoints.range(w.earliest..=w.latest_vis) {
            for tr in dp.grouped_trades.iter() {
                if tr.price < w.lowest || tr.price > w.highest {
                    continue;
                }
                max_qty = max_qty.max(tr.qty);
            }
        }

        max_qty
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

    fn zoom_row_h_at(&mut self, factor: f32, cursor_y: f32, vh_px: f32) {
        if !factor.is_finite() || vh_px <= 1.0 {
            return;
        }

        let sy = self.scene.camera.scale[1].max(MIN_CAMERA_SCALE);

        let y_world_before = self.scene.camera.offset[1] + (cursor_y - 0.5 * vh_px) / sy;

        let row_units = y_world_before / self.row_h.max(MIN_ROW_H_WORLD);
        let new_row_h = (self.row_h * factor).clamp(MIN_ROW_H_WORLD, MAX_ROW_H_WORLD);

        let y_world_after = row_units * new_row_h;
        self.scene.camera.offset[1] = y_world_after - (cursor_y - 0.5 * vh_px) / sy;

        self.row_h = new_row_h;
    }

    fn zoom_column_world_at(&mut self, factor: f32, cursor_x: f32, vw_px: f32) {
        if !factor.is_finite() || vw_px <= 1.0 {
            return;
        }

        let sx = self.scene.camera.scale[0].max(MIN_CAMERA_SCALE);
        let x_world_before = self.scene.camera.offset[0] + (cursor_x - vw_px) / sx;

        let col_units = x_world_before / self.column_world.max(MIN_COL_W_WORLD);
        let new_col_w = (self.column_world * factor).clamp(MIN_COL_W_WORLD, MAX_COL_W_WORLD);

        let x_world_after = col_units * new_col_w;

        self.scene.camera.offset[0] = x_world_after - (cursor_x - vw_px) / sx;

        self.column_world = new_col_w;
    }
}
