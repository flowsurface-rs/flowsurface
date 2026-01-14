mod depth_grid;
mod scale;
mod scene;
mod view;

use crate::chart::Action;
use crate::style::{self};
use crate::widget::chart::heatmap::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::scale::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::scale::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::scene::Scene;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;
use crate::widget::chart::heatmap::view::{ViewConfig, ViewInputs, ViewWindow};

use data::aggr::time::{DataPoint, TimeSeries};
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};
use iced::time::Instant;
use iced::widget::{Canvas, Space, column, container, mouse_area, row, rule, shader};
use iced::{Element, Fill, Length, padding};

const DEPTH_GRID_HORIZON_BUCKETS: u32 = 4800;
const DEPTH_GRID_TEX_H: u32 = 2048; // 2048 steps around anchor
const DEPTH_QTY_SCALE: f32 = 1.0; // dollars-per-u32 step

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

const VOLUME_TOTAL_ALPHA: f32 = 0.65;
const VOLUME_DELTA_ALPHA: f32 = 1.0;

const VOLUME_DELTA_TINT_TO_WHITE: f32 = 0.12;

const MIN_ROW_H_WORLD: f32 = 0.01;
const MAX_ROW_H_WORLD: f32 = 10.;

const MIN_COL_W_WORLD: f32 = 0.01;
const MAX_COL_W_WORLD: f32 = 10.;

// Debounce heavy CPU rebuilds (notably `rebuild_from_historical`) during interaction.
const REBUILD_DEBOUNCE_MS: u64 = 250;

// Throttle depth denom recompute while interacting (keeps zoom smooth).
const NORM_RECOMPUTE_THROTTLE_MS: u64 = 100;

// Shift volume-strip rects left by half a bucket to align with circle centers.
const VOLUME_X_SHIFT_BUCKET: f32 = -0.5;

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
    clock: view::ExchangeClock,
    depth_grid: depth_grid::GridRing,
}

const DEPTH_GRID_GRACE_MS: u64 = 500;

impl RealDataState {
    pub fn new(
        basis: Basis,
        step: PriceStep,
        ticker_info: TickerInfo,
        heatmap: HistoricalDepth,
        trades: TimeSeries<HeatmapDataPoint>,
    ) -> Self {
        let mut depth_grid =
            depth_grid::GridRing::new(DEPTH_GRID_HORIZON_BUCKETS, DEPTH_GRID_TEX_H);
        depth_grid.set_grace_ms(DEPTH_GRID_GRACE_MS);

        Self {
            basis,
            step,
            ticker_info,
            trades,
            heatmap,
            latest_time: 0,
            base_price: Price::from_units(0),
            depth_grid,
            clock: view::ExchangeClock::Uninit,
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
    x_axis_cache: iced::widget::canvas::Cache,

    // Cache for depth normalization denom (max qty) to avoid per-frame scans.
    depth_norm: view::DepthNormCache,
    data_gen: u64,

    // reusable buffers to avoid per-frame allocations
    volume_acc: Vec<(f32, f32)>,
    volume_touched: Vec<usize>,

    rebuild_policy: view::RebuildPolicy,
}

impl HeatmapShader {
    pub fn new(basis: Basis, tick_size: f32, ticker_info: TickerInfo) -> Self {
        let step = PriceStep::from_f32(tick_size);
        let data = Self::make_real_data_state(basis, step, ticker_info);

        let mut scene = Scene::new();
        scene.params.origin[3] = VOLUME_X_SHIFT_BUCKET;

        Self {
            last_tick: None,
            scene,
            viewport: None,
            row_h: DEFAULT_ROW_H_WORLD,
            column_world: DEFAULT_COL_W_WORLD,
            palette: None,
            data,
            profile_bid_acc: Vec::new(),
            profile_ask_acc: Vec::new(),
            scroll_ref_bucket: 0,
            x_phase_bucket: 0.0,
            render_latest_time: 0,
            x_axis_cache: iced::widget::canvas::Cache::new(),
            depth_norm: view::DepthNormCache::new(),
            data_gen: 1,
            volume_acc: Vec::new(),
            volume_touched: Vec::new(),
            rebuild_policy: view::RebuildPolicy::Idle,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(viewport) => {
                self.viewport = Some(viewport);

                self.rebuild_policy = view::RebuildPolicy::Immediate;
                self.rebuild_instances();
            }
            Message::PanDeltaPx(delta_px) => {
                let dx_world = delta_px.x / self.scene.camera.scale[0];
                let dy_world = delta_px.y / self.scene.camera.scale[1];

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;

                // Keep overlays aligned immediately (axes update on message; overlays should too).
                self.try_rebuild_overlays();

                self.rebuild_policy = view::RebuildPolicy::Debounced {
                    last_input: Instant::now(),
                };
            }
            Message::ZoomAt { factor, cursor } => {
                let Some([vw, vh]) = self.viewport else {
                    return;
                };

                self.scene
                    .camera
                    .zoom_at_cursor(factor, cursor.x, cursor.y, vw, vh);

                self.try_rebuild_overlays();
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::ZoomRowHeightAt {
                factor,
                cursor_y,
                viewport_h,
            } => {
                self.zoom_row_h_at(factor, cursor_y, viewport_h);
                self.sync_grid_xy();

                self.try_rebuild_overlays();

                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::ZoomColumnWorldAt {
                factor,
                cursor_x,
                viewport_w,
            } => {
                self.zoom_column_world_at(factor, cursor_x, viewport_w);
                self.sync_grid_xy();

                self.try_rebuild_overlays();
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
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

    /// called periodically on every frame, monitor refresh rates
    /// to update time-based rendering and animate/scroll
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

        if let Some(exchange_now_ms) = self.data.clock.estimate_now_ms(now_i) {
            let render_latest_time = (exchange_now_ms / aggr_time) * aggr_time;
            let phase_ms = exchange_now_ms.saturating_sub(render_latest_time);
            let phase = (phase_ms as f32 / aggr_time as f32).clamp(0.0, 0.999_999);

            self.render_latest_time = render_latest_time;
            self.x_phase_bucket = phase;

            if let Some(w) = self.compute_view_window(vw_px, vh_px) {
                // Keep time-origin uniforms up to date regardless of interaction
                let render_bucket: i64 = (render_latest_time / aggr_time) as i64;

                if self.scroll_ref_bucket == 0 {
                    self.scroll_ref_bucket = render_bucket;
                }

                let delta_buckets: i64 = render_bucket - self.scroll_ref_bucket;
                self.scene.params.origin[0] = (delta_buckets as f32) + self.x_phase_bucket;

                let latest_bucket_data: i64 = (self.data.latest_time / aggr_time) as i64;
                let latest_rel: i64 = latest_bucket_data - self.scroll_ref_bucket;
                self.scene.params.heatmap_a[0] = latest_rel as f32;

                match self.rebuild_policy {
                    view::RebuildPolicy::Immediate => {
                        self.rebuild_instances();
                        self.rebuild_policy = view::RebuildPolicy::Idle;
                    }
                    view::RebuildPolicy::Debounced { last_input } => {
                        self.rebuild_overlays(&w);

                        if now_i.saturating_duration_since(last_input).as_millis() as u64
                            >= REBUILD_DEBOUNCE_MS
                        {
                            self.rebuild_instances();
                            self.rebuild_policy = view::RebuildPolicy::Idle;
                        }
                    }
                    view::RebuildPolicy::Idle => {}
                }

                // Keep denom + y_start updated (but throttle denom scans while interacting).
                let latest_incl = w.latest_vis.saturating_add(w.aggr_time);

                let is_interacting =
                    matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

                let denom = self.depth_norm.compute_throttled(
                    &self.data.heatmap,
                    &w,
                    latest_incl,
                    self.data.step,
                    self.data_gen,
                    now_i,
                    is_interacting,
                    NORM_RECOMPUTE_THROTTLE_MS,
                );
                self.scene.params.depth[0] = denom;

                // Keep y_start_bin aligned to ring anchor/base_price.
                self.scene.params.heatmap_a[1] = self
                    .data
                    .depth_grid
                    .heatmap_y_start_bin(self.data.base_price, self.data.step);
            }

            self.x_axis_cache.clear();
        }

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }
        None
    }

    /// only data insertion point, called from outside when new data arrives
    /// could be 1s, 500ms or 100ms, on par with aggregation interval but with additional network latency
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

        state.clock = state.clock.anchor_with_update(depth_update_t);
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

        state.depth_grid.ensure_layout(aggr_time);
        state.heatmap.insert_latest_depth(depth, rounded_t);

        if rounded_t >= state.latest_time {
            if let Some(mid) = depth.mid_price() {
                state.base_price = mid.round_to_step(state.step);
            }
            state.latest_time = rounded_t;
        }

        state.depth_grid.ensure_layout(aggr_time);

        let steps_per_y_bin: i64 = self.scene.params.grid[2].round().max(1.0) as i64;

        state.depth_grid.ingest_snapshot(
            depth,
            rounded_t,
            state.step,
            DEPTH_QTY_SCALE,
            state.base_price,
            steps_per_y_bin,
        );

        self.data_gen = self.data_gen.wrapping_add(1);
        self.depth_norm.invalidate();

        let tex_w = state.depth_grid.tex_w();
        let tex_h = state.depth_grid.tex_h();

        if let Some(p) = &self.palette {
            self.scene.params.bid_rgb = [p.bid_rgb[0], p.bid_rgb[1], p.bid_rgb[2], 0.0];
            self.scene.params.ask_rgb = [p.ask_rgb[0], p.ask_rgb[1], p.ask_rgb[2], 0.0];
        }

        if tex_w > 0 && tex_h > 0 {
            let bucket = (rounded_t / aggr_time) as i64;

            if self.scroll_ref_bucket == 0 {
                self.scroll_ref_bucket = bucket;
            }

            // uniforms
            let latest_rel: i64 = bucket - self.scroll_ref_bucket;
            self.scene.params.heatmap_a[0] = latest_rel as f32;
            let latest_x_ring: u32 = state.depth_grid.ring_x_for_bucket(bucket);
            self.scene.params.heatmap_a[3] = latest_x_ring as f32;

            self.scene.params.heatmap_b = [
                tex_w as f32,
                tex_h as f32,
                (tex_w - 1) as f32,
                1.0 / DEPTH_QTY_SCALE,
            ];

            self.scene.params.heatmap_a[1] = state
                .depth_grid
                .heatmap_y_start_bin(state.base_price, state.step);

            let plan = state.depth_grid.build_scene_upload_plan();
            self.scene.apply_heatmap_upload_plan(plan);
        }

        if (self.data_gen & 0x3F) == 0 {
            self.cleanup_old_data(aggr_time);
        }

        self.rebuild_policy = view::RebuildPolicy::Immediate;
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

    /// Rebuild only CPU overlay instances (profile/volume/trades). This is intended to be
    /// cheap enough to run during interaction, unlike `rebuild_from_historical`.
    fn rebuild_overlays(&mut self, w: &ViewWindow) {
        let max_trade_qty = self.max_trade_qty(w);
        let circles = if max_trade_qty > 0.0 {
            self.build_circles(w, max_trade_qty)
        } else {
            vec![]
        };

        let mut rects: Vec<RectInstance> = Vec::new();
        self.push_latest_profile_rects(w, &mut rects);
        self.push_volume_strip_rects(w, &mut rects);

        self.scene.set_rectangles(rects);
        self.scene.set_circles(circles);
    }

    fn try_rebuild_overlays(&mut self) {
        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };
        let Some(w) = self.compute_view_window(vw_px, vh_px) else {
            return;
        };
        self.rebuild_overlays(&w);
    }

    fn rebuild_instances(&mut self) {
        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };

        let Some(w) = self.compute_view_window(vw_px, vh_px) else {
            self.clear_scene();
            return;
        };

        let aggr_time: u64 = match self.data.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return,
        };

        let prev_steps_per_y_bin: i64 = self.scene.params.grid[2].round().max(1.0) as i64;
        let new_steps_per_y_bin: i64 = w.steps_per_y_bin.max(1);

        self.scene.params.grid = [
            self.column_world,
            self.row_h,
            new_steps_per_y_bin as f32,
            0.0,
        ];

        if new_steps_per_y_bin != prev_steps_per_y_bin {
            self.data.depth_grid.ensure_layout(aggr_time);

            let latest_time = self.render_latest_time.max(self.data.latest_time);
            let oldest_time =
                latest_time.saturating_sub(u64::from(DEPTH_GRID_HORIZON_BUCKETS) * aggr_time);

            let tex_h_i64 = self.data.depth_grid.tex_h().max(1) as i64;
            let half_bins = tex_h_i64 / 2;

            let step_units = self.data.step.units.max(1);
            let steps_per = new_steps_per_y_bin.max(1);

            let half_steps = half_bins.saturating_mul(steps_per);
            let delta_units = half_steps.saturating_mul(step_units);

            let base_u = self.data.base_price.units;
            let rebuild_highest = Price::from_units(base_u.saturating_add(delta_units));
            let rebuild_lowest = Price::from_units(base_u.saturating_sub(delta_units));

            self.data.depth_grid.rebuild_from_historical(
                &self.data.heatmap,
                oldest_time,
                latest_time,
                self.data.base_price,
                self.data.step,
                new_steps_per_y_bin,
                DEPTH_QTY_SCALE,
                rebuild_highest,
                rebuild_lowest,
            );

            self.data_gen = self.data_gen.wrapping_add(1);
            self.depth_norm.invalidate();

            let tex_w = self.data.depth_grid.tex_w();
            let tex_h = self.data.depth_grid.tex_h();

            if tex_w > 0 && tex_h > 0 {
                let latest_bucket: i64 = (latest_time / aggr_time) as i64;

                let latest_rel: i64 = latest_bucket - self.scroll_ref_bucket;
                self.scene.params.heatmap_a[0] = latest_rel as f32;

                let latest_x_ring: u32 = self.data.depth_grid.ring_x_for_bucket(latest_bucket);
                self.scene.params.heatmap_a[3] = latest_x_ring as f32;

                self.scene.params.heatmap_b = [
                    tex_w as f32,
                    tex_h as f32,
                    (tex_w - 1) as f32,
                    1.0 / DEPTH_QTY_SCALE,
                ];

                self.scene.params.heatmap_a[1] = self
                    .data
                    .depth_grid
                    .heatmap_y_start_bin(self.data.base_price, self.data.step);

                let plan = self.data.depth_grid.build_scene_upload_plan();
                self.scene.apply_heatmap_upload_plan(plan);
            }
        } else {
            self.scene.set_heatmap_update(None);
        }

        // Always rebuild overlays when we do a full rebuild.
        self.rebuild_overlays(&w);
    }

    fn cleanup_old_data(&mut self, aggr_time: u64) {
        let aggr_time = aggr_time.max(1);

        // Keep CPU history aligned with what the ring can represent
        let keep_buckets: u64 = (self.data.depth_grid.tex_w().max(1)) as u64;

        let latest_time = self.data.latest_time;
        if latest_time == 0 {
            return;
        }

        let keep_ms = keep_buckets.saturating_mul(aggr_time);
        let cutoff = latest_time.saturating_sub(keep_ms);
        let cutoff_rounded = (cutoff / aggr_time) * aggr_time;

        // Prune trades (TimeSeries datapoints are bucket timestamps)
        let keep = self.data.trades.datapoints.split_off(&cutoff_rounded);
        self.data.trades.datapoints = keep;

        // Prune HistoricalDepth to match the oldest remaining trade bucket (if any),
        // otherwise prune by cutoff directly
        if let Some(oldest_time) = self.data.trades.datapoints.keys().next().copied() {
            self.data.heatmap.cleanup_old_price_levels(oldest_time);
        } else {
            self.data.heatmap.cleanup_old_price_levels(cutoff_rounded);
        }
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);
    }

    fn clear_scene(&mut self) {
        self.scene.set_rectangles(Vec::new());
        self.scene.set_circles(Vec::new());
        self.scene.set_heatmap_update(None);
    }

    #[inline]
    fn y_center_for_bin(y_bin: i64, w: &ViewWindow) -> f32 {
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
            let bucket = (*bucket_time / aggr) as i64;

            for tr in dp.grouped_trades.iter() {
                let x_frac: f32 = 0.0;

                let x_bin_rel: i32 =
                    (bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;

                let y_world: f32 = self.y_world_for_trade_price(tr.price, w);

                let q = tr.qty.max(0.0);
                let t = if max_trade_qty > 0.0 {
                    (q / max_trade_qty).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let radius_px = TRADE_R_MIN_PX + t * (TRADE_R_MAX_PX - TRADE_R_MIN_PX);

                let rgba = if tr.is_sell {
                    [
                        palette.sell_rgb[0],
                        palette.sell_rgb[1],
                        palette.sell_rgb[2],
                        TRADE_ALPHA,
                    ]
                } else {
                    [
                        palette.buy_rgb[0],
                        palette.buy_rgb[1],
                        palette.buy_rgb[2],
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
                    x0_bin: 0,
                    x1_bin_excl: 0,
                    x_from_bins: 0,
                });
            }
        }
    }

    fn push_volume_strip_rects(&mut self, w: &ViewWindow, rects: &mut Vec<RectInstance>) {
        if w.strip_h_world <= 0.0 {
            return;
        }

        let Some(palette) = &self.palette else {
            return;
        };

        let state = &self.data;

        let latest_vis_for_strip: u64 = if self.render_latest_time > 0 {
            w.latest_vis.min(self.render_latest_time.saturating_sub(1))
        } else {
            w.latest_vis
        };
        if latest_vis_for_strip < w.earliest {
            return;
        }
        let latest_bucket_vis: i64 = (latest_vis_for_strip / w.aggr_time) as i64;

        // X downsampling
        let px_per_col = self.column_world * w.sx;
        let px_per_drawn_col = px_per_col * (1.0 - VOLUME_BUCKET_GAP_FRAC);
        let mut cols_per_x_bin: i64 = 1;
        if px_per_drawn_col.is_finite() && px_per_drawn_col > 0.0 {
            cols_per_x_bin = (VOLUME_MIN_BAR_W_PX / px_per_drawn_col).ceil() as i64;
            cols_per_x_bin = cols_per_x_bin.clamp(1, MAX_COLS_PER_X_BIN);
        }
        let cols_per_x_bin = cols_per_x_bin.max(1);

        let start_bucket_vis = (w.earliest / w.aggr_time) as i64;
        let end_bucket_vis = latest_bucket_vis;

        let min_x_bin = start_bucket_vis.div_euclid(cols_per_x_bin);
        let max_x_bin = end_bucket_vis.div_euclid(cols_per_x_bin);
        if max_x_bin < min_x_bin {
            return;
        }

        let bins_len = (max_x_bin - min_x_bin + 1) as usize;

        self.volume_acc.resize(bins_len, (0.0, 0.0));
        self.volume_acc.iter_mut().for_each(|e| *e = (0.0, 0.0));
        self.volume_touched.clear();

        // Accumulate only up to `latest_vis_for_strip` (completed buckets only)
        for (time, dp) in state
            .trades
            .datapoints
            .range(w.earliest..=latest_vis_for_strip)
        {
            let bucket = (*time / w.aggr_time) as i64;
            let x_bin = bucket.div_euclid(cols_per_x_bin);
            let idx_i64 = x_bin - min_x_bin;
            if idx_i64 < 0 {
                continue;
            }
            let idx = idx_i64 as usize;
            if idx >= bins_len {
                continue;
            }

            let (buy, sell) = dp.buy_sell;
            if buy == 0.0 && sell == 0.0 {
                continue;
            }

            let e = &mut self.volume_acc[idx];
            let was_zero = e.0 == 0.0 && e.1 == 0.0;
            e.0 += buy;
            e.1 += sell;
            if was_zero {
                self.volume_touched.push(idx);
            }
        }

        if self.volume_touched.is_empty() {
            return;
        }

        self.volume_touched.sort_unstable();
        self.volume_touched.dedup();

        let mut max_total_vol: f32 = 0.0;
        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            max_total_vol = max_total_vol.max(buy + sell);
        }
        if max_total_vol <= 0.0 {
            return;
        }
        let denom = max_total_vol.max(1e-12);

        let min_h_world: f32 = VOLUME_MIN_BAR_PX / w.sy;
        let ref_bucket = self.scroll_ref_bucket;
        let eps = 1e-12f32;

        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            let total = buy + sell;
            if total <= 0.0 {
                continue;
            }

            let x_bin = min_x_bin + idx as i64;

            let start_bucket = x_bin * cols_per_x_bin;
            let mut end_bucket_excl = start_bucket + cols_per_x_bin;

            // Clamp to the last completed bucket (+1 for exclusivity)
            end_bucket_excl = end_bucket_excl.min(latest_bucket_vis + 1);
            if end_bucket_excl <= start_bucket {
                continue;
            }

            let x0_rel = (start_bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            let x1_rel =
                (end_bucket_excl - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;

            let (base_rgb, is_tie) = if buy > sell + eps {
                (palette.buy_rgb, false)
            } else if sell > buy + eps {
                (palette.sell_rgb, false)
            } else {
                (palette.secondary_rgb, true)
            };

            let total_h = ((total / denom) * w.strip_h_world).max(min_h_world);
            let total_center_y = w.strip_bottom_y - 0.5 * total_h;

            rects.push(RectInstance {
                position: [0.0, total_center_y],
                size: [0.0, total_h],
                color: [base_rgb[0], base_rgb[1], base_rgb[2], VOLUME_TOTAL_ALPHA],
                x0_bin: x0_rel,
                x1_bin_excl: x1_rel,
                x_from_bins: 1,
            });

            if !is_tie {
                let diff = (buy - sell).abs();
                if diff > eps {
                    let mut overlay_h = ((diff / denom) * w.strip_h_world).max(min_h_world);
                    overlay_h = overlay_h.min(total_h);

                    let t = VOLUME_DELTA_TINT_TO_WHITE;
                    let overlay_rgb = [
                        base_rgb[0] + (1.0 - base_rgb[0]) * t,
                        base_rgb[1] + (1.0 - base_rgb[1]) * t,
                        base_rgb[2] + (1.0 - base_rgb[2]) * t,
                    ];

                    let overlay_center_y = w.strip_bottom_y - 0.5 * overlay_h;

                    rects.push(RectInstance {
                        position: [0.0, overlay_center_y],
                        size: [0.0, overlay_h],
                        color: [
                            overlay_rgb[0],
                            overlay_rgb[1],
                            overlay_rgb[2],
                            VOLUME_DELTA_ALPHA,
                        ],
                        x0_bin: x0_rel,
                        x1_bin_excl: x1_rel,
                        x_from_bins: 1,
                    });
                }
            }
        }
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

    #[inline]
    fn sync_grid_xy(&mut self) {
        // Keep shader mapping in sync during interaction without doing any heavy rebuild work.
        self.scene.params.grid[0] = self.column_world;
        self.scene.params.grid[1] = self.row_h;
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

    fn make_real_data_state(
        basis: Basis,
        step: PriceStep,
        ticker_info: TickerInfo,
    ) -> RealDataState {
        let heatmap = HistoricalDepth::new(ticker_info.min_qty.into(), step, basis);
        let trades = TimeSeries::<HeatmapDataPoint>::new(basis, step);
        RealDataState::new(basis, step, ticker_info, heatmap, trades)
    }

    fn apply_data_state(&mut self, basis: Basis, step: PriceStep, ticker_info: TickerInfo) {
        self.data = Self::make_real_data_state(basis, step, ticker_info);
        self.scene.params.origin[3] = VOLUME_X_SHIFT_BUCKET;

        self.profile_bid_acc.clear();
        self.profile_ask_acc.clear();
        self.volume_acc.clear();
        self.volume_touched.clear();

        self.scroll_ref_bucket = 0;
        self.x_phase_bucket = 0.0;
        self.render_latest_time = 0;

        self.data_gen = self.data_gen.wrapping_add(1);
        self.depth_norm.invalidate();
        self.x_axis_cache.clear();

        self.clear_scene();
        self.rebuild_policy = view::RebuildPolicy::Immediate;

        if self.viewport.is_some() {
            self.rebuild_instances();
        }
    }

    pub fn set_tick_size(&mut self, tick_size: f32) {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return;
        }

        let basis = self.data.basis;
        let ticker_info = self.data.ticker_info;

        let step = PriceStep::from_f32(tick_size);

        self.apply_data_state(basis, step, ticker_info);
    }

    pub fn set_basis(&mut self, basis: Basis) {
        if basis == self.data.basis {
            return;
        }

        let tick_size = self.data.step.to_f32_lossy();
        let ticker_info = self.data.ticker_info;

        let step = PriceStep::from_f32(tick_size);

        self.apply_data_state(basis, step, ticker_info);
    }
}
