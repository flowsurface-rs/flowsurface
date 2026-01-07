mod grid;
mod lod;
mod raster;
mod scale;
mod scene;
mod view;

use crate::chart::Action;
use crate::style::{self};
use crate::widget::chart::heatmap::grid::{Abs, Bucket, Rel, YBin};
use crate::widget::chart::heatmap::scale::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::scale::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::scene::Scene;
use crate::widget::chart::heatmap::scene::pipeline::ParamsUniform;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;
use crate::widget::chart::heatmap::view::{ViewConfig, ViewInputs, ViewWindow};

use data::aggr::time::{DataPoint, TimeSeries};
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HeatmapPalette, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};
use iced::time::Instant;
use iced::widget::{Canvas, Space, column, container, mouse_area, row, rule, shader};
use iced::{Element, Fill, Length, padding};
use std::sync::Arc;

const TEXT_SIZE: f32 = 12.0;

const DEFAULT_ROW_H_WORLD: f32 = 0.05;
const DEFAULT_COL_W_WORLD: f32 = 0.05;

const MIN_CAMERA_SCALE: f32 = 1e-4;

const DEPTH_MIN_ROW_PX: f32 = 2.0;
const MAX_STEPS_PER_Y_BIN: i64 = 2048;

// Trades (circles)
const TRADE_R_MIN_PX: f32 = 2.0;
const TRADE_R_MAX_PX: f32 = 25.0;
const TRADE_ALPHA: f32 = 0.8;

// Depth (rect alpha normalization)
const DEPTH_ALPHA_MIN: f32 = 0.01;
const DEPTH_ALPHA_MAX: f32 = 0.99;

// Latest profile overlay (x > 0)
const PROFILE_COL_WIDTH_PX: f32 = 180.0;
const PROFILE_MIN_BAR_PX: f32 = 1.0;
const PROFILE_ALPHA: f32 = 0.8;

// Volume strip
const STRIP_HEIGHT_FRAC: f32 = 0.10;
const VOLUME_BUCKET_GAP_FRAC: f32 = 0.10;
const VOLUME_MIN_BAR_PX: f32 = 1.0; // min bar height in px
const VOLUME_MIN_BAR_W_PX: f32 = 2.0; // min bar width in px (for x-binning)
const MAX_COLS_PER_X_BIN: i64 = 4096;
const VOLUME_TOTAL_RGB: [f32; 3] = [0.7, 0.7, 0.7];
const VOLUME_TOTAL_ALPHA: f32 = 0.18;

const MIN_ROW_H_WORLD: f32 = 0.01;
const MAX_ROW_H_WORLD: f32 = 10.;

const MIN_COL_W_WORLD: f32 = 0.01;
const MAX_COL_W_WORLD: f32 = 10.;

// Heatmap X-LOD
const HEATMAP_BIN_ENABLE_COL_PX: f32 = 1.10; // start binning only when a column is ~1px
const HEATMAP_BIN_DISABLE_COL_PX: f32 = 1.40; // stop binning when column grows beyond this
const HEATMAP_BIN_TARGET_PX: f32 = 2.0;

#[derive(Debug, Clone)]
pub enum Message {
    BoundsChanged([f32; 2]),
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
    last_update_exchange_ms: Option<u64>,
    last_update_instant: Option<Instant>,
    last_estimated_exchange_now_ms: Option<u64>,
}

impl RealDataState {
    pub fn new(
        basis: Basis,
        step: PriceStep,
        ticker_info: TickerInfo,
        heatmap: HistoricalDepth,
        trades: TimeSeries<HeatmapDataPoint>,
    ) -> Self {
        Self {
            basis,
            step,
            ticker_info,
            trades,
            heatmap,
            latest_time: 0,
            base_price: Price::from_units(0),
            last_update_exchange_ms: None,
            last_update_instant: None,
            last_estimated_exchange_now_ms: None,
        }
    }
}

pub struct HeatmapShader {
    pub last_tick: Option<Instant>,
    scene: Scene,
    viewport: Option<[f32; 2]>,
    row_h: f32,
    column_world: f32,
    palette: Option<HeatmapPalette>,
    data: RealDataState,
    profile_bid_acc: Vec<f32>,
    profile_ask_acc: Vec<f32>,
    scroll_ref_bucket: i64,
    x_phase_bucket: f32,
    render_latest_time: u64,
    needs_rebuild: bool,
    x_axis_cache: iced::widget::canvas::Cache,
    heatmap_tex_gen: u64,
    heatmap_x_lod: lod::HeatmapXLod,
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
            data: RealDataState::new(basis, step, ticker_info, heatmap, trades),
            profile_bid_acc: Vec::new(),
            profile_ask_acc: Vec::new(),
            scroll_ref_bucket: 0,
            x_phase_bucket: 0.0,
            render_latest_time: 0,
            needs_rebuild: false,
            x_axis_cache: iced::widget::canvas::Cache::new(),
            heatmap_tex_gen: 1,
            heatmap_x_lod: lod::HeatmapXLod::default(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(viewport) => {
                self.viewport = Some(viewport);
                self.rebuild_instances();
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

        let render_latest_bucket: i64 = match aggr_time {
            Some(aggr) if aggr > 0 => (self.render_latest_time / aggr) as i64,
            _ => self.scroll_ref_bucket,
        };

        let x_axis_label = Canvas::new(AxisXLabelCanvas {
            cache: &self.x_axis_cache,
            latest_bucket: render_latest_bucket,
            aggr_time,
            column_world: self.column_world,
            cam_offset_x: self.scene.camera.offset[0],
            cam_sx: self.scene.camera.scale[0].max(MIN_CAMERA_SCALE),
            cam_right_pad_frac: self.scene.camera.right_pad_frac,
            x_phase_bucket: self.x_phase_bucket,
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

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let now_i = now.unwrap_or_else(Instant::now);
        self.last_tick = Some(now_i);

        let Some([vw_px, vh_px]) = self.viewport else {
            if self.palette.is_none() {
                return Some(Action::RequestPalette);
            }
            return None;
        };

        let aggr_time: u64 = match self.data.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return None,
        };
        if aggr_time == 0 {
            return None;
        }

        if let (Some(anchor_ms), Some(anchor_i)) = (
            self.data.last_update_exchange_ms,
            self.data.last_update_instant,
        ) {
            let elapsed_ms = now_i.saturating_duration_since(anchor_i).as_millis() as u64;

            let mut exchange_now_ms = anchor_ms.saturating_add(elapsed_ms);

            if let Some(prev_est) = self.data.last_estimated_exchange_now_ms {
                exchange_now_ms = exchange_now_ms.max(prev_est);
            }
            self.data.last_estimated_exchange_now_ms = Some(exchange_now_ms);

            let render_latest_time = (exchange_now_ms / aggr_time) * aggr_time;
            let phase_ms = exchange_now_ms.saturating_sub(render_latest_time);
            let phase = (phase_ms as f32 / aggr_time as f32).clamp(0.0, 0.999_999);

            self.render_latest_time = render_latest_time;
            self.x_phase_bucket = phase;

            if self.needs_rebuild {
                self.needs_rebuild = false;
                self.rebuild_instances();
            }

            if let Some(w) = self.compute_view_window(vw_px, vh_px) {
                let volume_min_w_world: f32 = VOLUME_MIN_BAR_W_PX / w.sx;

                let render_bucket: i64 = (render_latest_time / aggr_time) as i64;

                let delta_buckets: i64 = render_bucket - self.scroll_ref_bucket;
                let now_bucket_rel_f: f32 = (delta_buckets as f32) + self.x_phase_bucket;

                let col_px = self.column_world * w.sx;
                self.heatmap_x_lod
                    .update_from_col_px(col_px, self.heatmap_x_lod_cfg());

                self.scene.params.origin = [
                    now_bucket_rel_f,
                    volume_min_w_world,
                    VOLUME_BUCKET_GAP_FRAC,
                    0.0,
                ];
                self.scene.params.grid =
                    [self.column_world, self.row_h, w.steps_per_y_bin as f32, 0.0];
            }

            self.x_axis_cache.clear();
        }

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }
        None
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
            Basis::Tick(_) => unimplemented!(),
        };

        let now_i = Instant::now();

        let predicted_now_ms: Option<u64> =
            match (state.last_update_exchange_ms, state.last_update_instant) {
                (Some(ms), Some(inst)) => {
                    let elapsed = now_i.saturating_duration_since(inst).as_millis() as u64;
                    Some(ms.saturating_add(elapsed))
                }
                _ => None,
            };

        let mut monotonic_now_ms = depth_update_t;
        if let Some(p) = predicted_now_ms {
            monotonic_now_ms = monotonic_now_ms.max(p);
        }
        if let Some(prev_est) = state.last_estimated_exchange_now_ms {
            monotonic_now_ms = monotonic_now_ms.max(prev_est);
        }

        state.last_update_exchange_ms = Some(monotonic_now_ms);
        state.last_update_instant = Some(now_i);
        state.last_estimated_exchange_now_ms = Some(monotonic_now_ms);

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

        if rounded_t >= state.latest_time {
            let mid = depth.mid_price().unwrap_or(state.base_price);
            state.base_price = mid.round_to_step(state.step);
            state.latest_time = rounded_t;
        }

        self.needs_rebuild = true;
    }

    fn compute_view_window(&self, vw_px: f32, vh_px: f32) -> Option<ViewWindow> {
        let aggr_time: u64 = match self.data.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return None,
        };

        let cfg = ViewConfig {
            min_camera_scale: MIN_CAMERA_SCALE,
            profile_col_width_px: PROFILE_COL_WIDTH_PX,
            strip_height_frac: STRIP_HEIGHT_FRAC,
            depth_min_row_px: DEPTH_MIN_ROW_PX,
            max_steps_per_y_bin: MAX_STEPS_PER_Y_BIN,
            min_row_h_world: MIN_ROW_H_WORLD,
        };

        let input = ViewInputs {
            aggr_time,
            latest_time_data: self.data.latest_time,
            latest_time_render: self.render_latest_time,
            base_price: self.data.base_price,
            step: self.data.step,
            row_h_world: self.row_h,
            col_w_world: self.column_world,
        };

        ViewWindow::compute(cfg, &self.scene.camera, [vw_px, vh_px], input)
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);
    }

    fn clear_scene(&mut self) {
        self.scene.set_rectangles(Vec::new());
        self.scene.set_circles(Vec::new());
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
            return vec![];
        };

        let aggr = w.aggr_time.max(1); // ms per bucket (u64)
        let ref_bucket = self.scroll_ref_bucket;

        let mut out: Vec<CircleInstance> = Vec::new();

        for (bucket_time, dp) in self.data.trades.datapoints.range(w.earliest..=w.latest_vis) {
            // bucket_time is already rounded to aggr, so x_frac will be 0 unless you use per-trade timestamps
            let bucket = (*bucket_time / aggr) as i64;

            for tr in dp.grouped_trades.iter() {
                let x_frac: f32 = 0.0;

                let x_bin_rel: i32 =
                    (bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;

                let y_world: f32 = self.y_world_for_trade_price(tr.price, w);

                // radius in px (keep your existing scaling logic)
                let q = tr.qty.max(0.0);
                let t = if max_trade_qty > 0.0 {
                    (q / max_trade_qty).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let radius_px = TRADE_R_MIN_PX + t * (TRADE_R_MAX_PX - TRADE_R_MIN_PX);

                let rgba = if tr.is_sell {
                    [
                        palette.ask_rgb[0],
                        palette.ask_rgb[1],
                        palette.ask_rgb[2],
                        TRADE_ALPHA,
                    ]
                } else {
                    [
                        palette.bid_rgb[0],
                        palette.bid_rgb[1],
                        palette.bid_rgb[2],
                        TRADE_ALPHA,
                    ]
                };

                out.push(CircleInstance {
                    y_world,
                    x_bin_rel,
                    x_frac,
                    radius_px,
                    _pad: 0.0,
                    color: rgba,
                });
            }
        }

        out
    }

    fn y_world_for_trade_price(&self, price: Price, w: &ViewWindow) -> f32 {
        let step_units = self.data.step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);

        let base_steps = self.data.base_price.units / step_units;
        let base_abs_y_bin = base_steps.div_euclid(y_div);

        let abs_steps = price.units / step_units;
        let abs_y_bin = abs_steps.div_euclid(y_div);

        let rel_y_bin = abs_y_bin - base_abs_y_bin;
        Self::y_center_for_bin(rel_y_bin, w)
    }

    fn push_latest_profile_rects(&mut self, w: &ViewWindow, rects: &mut Vec<RectInstance>) {
        let Some(palette) = &self.palette else {
            return;
        };

        if w.profile_max_w_world <= 0.0 {
            return;
        }

        let state = &self.data;

        // Use absolute-step anchored y-bins (matches heatmap texture binning)
        let step_units = state.step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);

        let base_steps = state.base_price.units / step_units;
        let base_abs_y_bin = base_steps.div_euclid(y_div);

        let lowest_abs_steps = w.lowest.units / step_units;
        let highest_abs_steps = w.highest.units / step_units;

        let min_abs_y_bin = lowest_abs_steps.div_euclid(y_div);
        let max_abs_y_bin = highest_abs_steps.div_euclid(y_div);

        if max_abs_y_bin < min_abs_y_bin {
            return;
        }

        let len = (max_abs_y_bin - min_abs_y_bin + 1) as usize;

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

            let abs_steps = price.units / step_units;
            let abs_y_bin = abs_steps.div_euclid(y_div);
            let idx = (abs_y_bin - min_abs_y_bin) as usize;

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
            let abs_y_bin = min_abs_y_bin + i as i64;
            let rel_y_bin = abs_y_bin - base_abs_y_bin;
            let y = Self::y_center_for_bin(rel_y_bin, w);

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
                    qty: 0.0,
                    side_sign: 0.0,
                    x0_bin: 0,
                    x1_bin_excl: 0,
                    abs_y_bin: 0,
                    flags: 1, // overlay
                });
            }
        }
    }

    fn push_volume_strip_rects(&self, w: &ViewWindow, rects: &mut Vec<RectInstance>) {
        if w.strip_h_world <= 0.0 {
            return;
        }

        let Some(palette) = &self.palette else {
            return;
        };

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

        let ref_bucket = self.scroll_ref_bucket;

        let eps = 1e-12f32;

        for (i, (buy, sell)) in acc.into_iter().enumerate() {
            let total = buy + sell;
            if total <= 0.0 {
                continue;
            }

            let x_bin = min_x_bin + i as i64;

            let start_bucket = x_bin * cols_per_x_bin;
            let mut end_bucket_excl = start_bucket + cols_per_x_bin;
            end_bucket_excl = end_bucket_excl.min(w.latest_bucket + 1);

            if end_bucket_excl <= start_bucket {
                continue;
            }

            if start_bucket <= w.latest_bucket && end_bucket_excl == w.latest_bucket + 1 {
                end_bucket_excl = i64::MAX;
            }

            let x0_rel = (start_bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            let x1_rel =
                (end_bucket_excl - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            let (base_rgb, is_tie) = if buy > sell + eps {
                (palette.buy_rgb, false)
            } else if sell > buy + eps {
                (palette.sell_rgb, false)
            } else {
                (VOLUME_TOTAL_RGB, true)
            };

            let total_h = ((total / denom) * w.strip_h_world).max(min_h_world);
            let total_center_y = w.strip_bottom_y - 0.5 * total_h;

            rects.push(RectInstance {
                position: [0.0, total_center_y],
                size: [0.0, total_h],
                color: [base_rgb[0], base_rgb[1], base_rgb[2], VOLUME_TOTAL_ALPHA],
                qty: 0.0,
                side_sign: 0.0,
                x0_bin: x0_rel,
                x1_bin_excl: x1_rel,
                abs_y_bin: 0,
                flags: 1 | 2,
            });

            if !is_tie {
                let diff = (buy - sell).abs();
                if diff > eps {
                    let mut overlay_h = (diff / denom) * w.strip_h_world;
                    overlay_h = overlay_h.min(total_h);

                    if overlay_h > 0.0 {
                        let overlay_center_y = w.strip_bottom_y - 0.5 * overlay_h;

                        rects.push(RectInstance {
                            position: [0.0, overlay_center_y],
                            size: [0.0, overlay_h],
                            color: [base_rgb[0], base_rgb[1], base_rgb[2], VOLUME_TOTAL_ALPHA],
                            qty: 0.0,
                            side_sign: 0.0,
                            x0_bin: x0_rel,
                            x1_bin_excl: x1_rel,
                            abs_y_bin: 0,
                            flags: 1 | 2,
                        });
                    }
                }
            }
        }
    }

    fn build_depth_texture(&mut self, w: &ViewWindow) -> Option<scene::HeatmapTextureCpu> {
        let Some(palette) = &self.palette else {
            return None;
        };
        let state = &self.data;

        let latest_for_depth = w.latest_vis.saturating_add(w.aggr_time);

        let render_bucket_end_excl_ms: u64 = (w.latest_bucket as i128 + 1)
            .saturating_mul(w.aggr_time as i128)
            .max(0) as u64;

        let combined = crate::widget::chart::heatmap::lod::binned_depth_rect_contribs_abs_ybin(
            &state.heatmap,
            w.earliest,
            latest_for_depth,
            render_bucket_end_excl_ms,
            w.highest,
            w.lowest,
            state.step,
            w.steps_per_y_bin,
            w.aggr_time,
            state.latest_time,
            w.latest_bucket,
        );

        if combined.is_empty() {
            let origin = self.scene.params.origin;
            self.scene.set_params(ParamsUniform {
                depth: [1.0, DEPTH_ALPHA_MIN, DEPTH_ALPHA_MAX, 0.0],
                bid_rgb: [
                    palette.bid_rgb[0],
                    palette.bid_rgb[1],
                    palette.bid_rgb[2],
                    0.0,
                ],
                ask_rgb: [
                    palette.ask_rgb[0],
                    palette.ask_rgb[1],
                    palette.ask_rgb[2],
                    0.0,
                ],
                grid: [self.column_world, self.row_h, w.steps_per_y_bin as f32, 0.0],
                origin,
                heatmap_a: [0.0, 0.0, 1.0, 0.0],
                heatmap_b: [0.0, 0.0, 0.0, 0.0],
            });
            return None;
        }

        let col_px = self.column_world * w.sx;
        self.heatmap_x_lod
            .update_from_col_px(col_px, self.heatmap_x_lod_cfg());
        let cols_per_x_bin = self.heatmap_x_lod.cols_per_x_bin().max(1);

        let ref_bucket = Bucket::<Abs>::abs(self.scroll_ref_bucket);

        let step_units = state.step.units.max(1);
        let base_steps = state.base_price.units / step_units;
        let base_abs_y_bin = YBin::<Abs>::abs(base_steps.div_euclid(w.steps_per_y_bin.max(1)));

        let min_steps = (w.lowest.units - state.base_price.units) / state.step.units;
        let max_steps = (w.highest.units - state.base_price.units) / state.step.units;
        let y_start_bin = YBin::<Rel>::rel(Self::y_bin_for_steps(min_steps, w.steps_per_y_bin));
        let y_end_bin_excl =
            YBin::<Rel>::rel(Self::y_bin_for_steps(max_steps, w.steps_per_y_bin) + 1);

        let mut spec = crate::widget::chart::heatmap::raster::spec_from_view(
            w,
            ref_bucket,
            cols_per_x_bin,
            base_abs_y_bin,
        )?;
        spec.set_y_range(y_start_bin, y_end_bin_excl);

        let width = spec.width();
        let height = spec.height();
        if width == 0 || height == 0 {
            return None;
        }

        let mut rg = vec![[0.0f32, 0.0f32]; (width as usize) * (height as usize)];

        // Build texture values over padded domain
        for (key, qty) in combined.into_iter() {
            let mut dummy_max = 0.0f32;
            spec.accumulate_max(key, qty, ref_bucket, &mut rg, &mut dummy_max);
        }

        // Normalize using *strict visible* x-range (no +/- bucket padding)
        let start_bucket_strict = (w.earliest_strict / w.aggr_time) as i64;
        let end_bucket_strict_excl = ((w.latest_vis_strict / w.aggr_time) as i64) + 1;

        let start_rel = Bucket::<Abs>::abs(start_bucket_strict).to_rel(ref_bucket);
        let end_rel_excl = Bucket::<Abs>::abs(end_bucket_strict_excl).to_rel(ref_bucket);

        let cols = spec.cols_per_x_bin();
        let g0_vis = start_rel.0.div_euclid(cols).max(spec.x_group_start());
        let g1_vis = {
            let e = end_rel_excl.0;
            let q = e.div_euclid(cols);
            let r = e.rem_euclid(cols);
            let div_ceil = if r == 0 { q } else { q + 1 };
            div_ceil.min(spec.x_group_end_excl())
        };

        let mut max_depth_visible = 0.0f32;
        if g1_vis > g0_vis {
            let x0 = (g0_vis - spec.x_group_start()) as usize;
            let x1 = (g1_vis - spec.x_group_start()) as usize;
            let w_tex = width as usize;

            for y in 0..(height as usize) {
                let row = &rg[(y * w_tex)..(y * w_tex + w_tex)];
                for px in &row[x0..x1] {
                    max_depth_visible = max_depth_visible.max(px[0]).max(px[1]);
                }
            }
        } else {
            // Fallback: if strict range collapses for any reason, use whole texture max
            for px in &rg {
                max_depth_visible = max_depth_visible.max(px[0]).max(px[1]);
            }
        }

        let denom_depth = max_depth_visible.max(1e-12);
        let origin = self.scene.params.origin;
        let samp = spec.sampling_params();

        self.scene.set_params(ParamsUniform {
            depth: [denom_depth, DEPTH_ALPHA_MIN, DEPTH_ALPHA_MAX, 0.0],
            bid_rgb: [
                palette.bid_rgb[0],
                palette.bid_rgb[1],
                palette.bid_rgb[2],
                0.0,
            ],
            ask_rgb: [
                palette.ask_rgb[0],
                palette.ask_rgb[1],
                palette.ask_rgb[2],
                0.0,
            ],
            grid: [self.column_world, self.row_h, w.steps_per_y_bin as f32, 0.0],
            origin,
            heatmap_a: samp.heatmap_a,
            heatmap_b: samp.heatmap_b,
        });

        self.heatmap_tex_gen = self.heatmap_tex_gen.wrapping_add(1);

        Some(scene::HeatmapTextureCpu {
            width,
            height,
            rg: Arc::new(rg),
            generation: self.heatmap_tex_gen,
        })
    }

    fn rebuild_instances(&mut self) {
        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };

        let Some(w) = self.compute_view_window(vw_px, vh_px) else {
            self.clear_scene();
            self.scene.set_heatmap(None);
            return;
        };

        let old_ref = self.scroll_ref_bucket;
        self.scroll_ref_bucket = w.latest_bucket;

        if self.scroll_ref_bucket != old_ref {
            self.x_axis_cache.clear();
        }

        let render_bucket: i64 = if self.render_latest_time > 0 && w.aggr_time > 0 {
            (self.render_latest_time / w.aggr_time) as i64
        } else {
            w.latest_bucket
        };

        let delta_buckets: i64 = render_bucket - self.scroll_ref_bucket;
        self.scene.params.origin[0] = (delta_buckets as f32) + self.x_phase_bucket;

        self.scene.params.grid = [self.column_world, self.row_h, w.steps_per_y_bin as f32, 0.0];

        let volume_min_w_world: f32 = VOLUME_MIN_BAR_W_PX / w.sx;
        self.scene.params.origin[1] = volume_min_w_world;
        self.scene.params.origin[2] = VOLUME_BUCKET_GAP_FRAC;

        let heatmap = self.build_depth_texture(&w);
        self.scene.set_heatmap(heatmap);

        let max_trade_qty = self.max_trade_qty(&w);
        let circles = if max_trade_qty > 0.0 {
            self.build_circles(&w, max_trade_qty)
        } else {
            vec![]
        };

        let mut rects: Vec<RectInstance> = Vec::new();
        self.push_latest_profile_rects(&w, &mut rects);
        self.push_volume_strip_rects(&w, &mut rects);

        self.scene.set_rectangles(rects);
        self.scene.set_circles(circles);
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

    fn zoom_row_h_at(&mut self, factor: f32, cursor_y: f32, vh_px: f32) {
        if !factor.is_finite() || vh_px <= 1.0 {
            return;
        }

        let world_y_before =
            self.scene
                .camera
                .world_y_at_screen_y_centered(cursor_y, vh_px, MIN_CAMERA_SCALE);

        let row_units_at_cursor = world_y_before / self.row_h.max(MIN_ROW_H_WORLD);
        let new_row_h = (self.row_h * factor).clamp(MIN_ROW_H_WORLD, MAX_ROW_H_WORLD);

        let world_y_after = row_units_at_cursor * new_row_h;

        self.scene
            .camera
            .set_offset_y_for_world_y_at_screen_y_centered(
                world_y_after,
                cursor_y,
                vh_px,
                MIN_CAMERA_SCALE,
            );

        self.row_h = new_row_h;
    }

    fn zoom_column_world_at(&mut self, factor: f32, cursor_x: f32, vw_px: f32) {
        if !factor.is_finite() || vw_px <= 1.0 {
            return;
        }

        let world_x_before =
            self.scene
                .camera
                .world_x_at_screen_x_right_anchored(cursor_x, vw_px, MIN_CAMERA_SCALE);

        let col_units_at_cursor = world_x_before / self.column_world.max(MIN_COL_W_WORLD);
        let new_col_w = (self.column_world * factor).clamp(MIN_COL_W_WORLD, MAX_COL_W_WORLD);

        let world_x_after = col_units_at_cursor * new_col_w;

        self.scene
            .camera
            .set_offset_x_for_world_x_at_screen_x_right_anchored(
                world_x_after,
                cursor_x,
                vw_px,
                MIN_CAMERA_SCALE,
            );

        self.column_world = new_col_w;
    }

    #[inline]
    fn heatmap_x_lod_cfg(&self) -> lod::HeatmapXLodConfig {
        lod::HeatmapXLodConfig {
            enable_col_px: HEATMAP_BIN_ENABLE_COL_PX,
            disable_col_px: HEATMAP_BIN_DISABLE_COL_PX,
            target_px: HEATMAP_BIN_TARGET_PX,
            max_cols_per_x_bin: MAX_COLS_PER_X_BIN,
        }
    }
}
