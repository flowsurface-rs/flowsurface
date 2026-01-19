mod instance;
mod scene;
mod ui;
mod view;
mod widget;

use crate::chart::Action;
use crate::widget::chart::heatmap::instance::InstanceBuilder;
use crate::widget::chart::heatmap::scene::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::scene::{Scene, depth_grid};
use crate::widget::chart::heatmap::ui::CanvasCaches;
use crate::widget::chart::heatmap::ui::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::ui::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::ui::overlay::OverlayCanvas;
use crate::widget::chart::heatmap::view::{ViewConfig, ViewInputs, ViewWindow};
use crate::widget::chart::heatmap::widget::HeatmapShaderWidget;

use data::aggr::time::{DataPoint, TimeSeries};
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};

use std::time::{Duration, Instant};

const DEPTH_GRID_HORIZON_BUCKETS: u32 = 4800;
const DEPTH_GRID_TEX_H: u32 = 2048; // 2048 steps around anchor
const DEPTH_QTY_SCALE: f32 = 1.0; // dollars-per-u32 step

const DEFAULT_ROW_H_WORLD: f32 = 0.05;
const DEFAULT_COL_W_WORLD: f32 = 0.05;

const MIN_CAMERA_SCALE: f32 = 1e-4;

const DEPTH_MIN_ROW_PX: f32 = 2.0;
const MAX_STEPS_PER_Y_BIN: i64 = 2048;

// Latest profile overlay (x > 0)
const PROFILE_COL_WIDTH_PX: f32 = 180.0;

// Volume strip
const STRIP_HEIGHT_FRAC: f32 = 0.10;

const MIN_ROW_H_WORLD: f32 = 0.01;
const MAX_ROW_H_WORLD: f32 = 10.;

const MIN_COL_W_WORLD: f32 = 0.01;
const MAX_COL_W_WORLD: f32 = 10.;

// Debounce heavy CPU rebuilds (notably `rebuild_from_historical`) during interaction
const REBUILD_DEBOUNCE_MS: u64 = 250;

// Throttle depth denom recompute while interacting (keeps zoom smooth)
const NORM_RECOMPUTE_THROTTLE_MS: u64 = 100;

// Shift volume-strip rects left by half a bucket to align with circle centers
const VOLUME_X_SHIFT_BUCKET: f32 = -0.5;

// If rendering stalls longer than this, assume GPU heatmap texture may have been lost/desynced
const HEATMAP_RESYNC_AFTER_STALL_MS: u64 = 750;

#[derive(Debug, Clone)]
pub enum Message {
    BoundsChanged(iced::Rectangle),
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
    PauseBtnClicked,
}

pub struct HeatmapShader {
    pub last_tick: Option<Instant>,
    scene: Scene,
    viewport: Option<iced::Rectangle>,
    row_h: f32,
    column_world: f32,
    palette: Option<HeatmapPalette>,
    instances: InstanceBuilder,
    canvas_caches: CanvasCaches,

    pub basis: Basis,
    pub step: PriceStep,
    pub ticker_info: TickerInfo,
    trades: TimeSeries<HeatmapDataPoint>,
    heatmap: HistoricalDepth,
    latest_time: u64,
    base_price: Price,
    clock: view::ExchangeClock,

    depth_grid: depth_grid::GridRing,
    // Cache for depth normalization denom (max qty) to avoid per-frame scans.
    depth_norm: view::DepthNormCache,
    data_gen: u64,

    anchor: view::Anchor,
    rebuild_policy: view::RebuildPolicy,

    // Force a full CPU->GPU heatmap upload (without rebuilding from historical).
    needs_heatmap_full_upload: bool,
}

impl HeatmapShader {
    pub fn new(basis: Basis, tick_size: f32, ticker_info: TickerInfo) -> Self {
        let step = PriceStep::from_f32(tick_size);

        let heatmap = HistoricalDepth::new(ticker_info.min_qty.into(), step, basis);
        let trades = TimeSeries::<HeatmapDataPoint>::new(basis, step);

        let mut scene = Scene::new();
        scene.params.origin[3] = VOLUME_X_SHIFT_BUCKET;

        let depth_grid = depth_grid::GridRing::new(DEPTH_GRID_HORIZON_BUCKETS, DEPTH_GRID_TEX_H);

        Self {
            last_tick: None,
            scene,
            viewport: None,
            row_h: DEFAULT_ROW_H_WORLD,
            column_world: DEFAULT_COL_W_WORLD,
            palette: None,
            heatmap,
            basis,
            step,
            ticker_info,
            trades,
            latest_time: 0,
            base_price: Price::from_units(0),
            clock: view::ExchangeClock::Uninit,
            instances: InstanceBuilder::new(),
            canvas_caches: CanvasCaches::new(),
            depth_grid,
            depth_norm: view::DepthNormCache::new(),
            data_gen: 1,
            rebuild_policy: view::RebuildPolicy::Idle,
            anchor: view::Anchor::default(),
            needs_heatmap_full_upload: true,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(bounds) => {
                self.viewport = Some(bounds);

                self.rebuild_policy = view::RebuildPolicy::Immediate;
                self.rebuild_instances();
            }
            Message::PanDeltaPx(delta_px) => {
                let dx_world = delta_px.x / self.scene.camera.scale[0];
                let dy_world = delta_px.y / self.scene.camera.scale[1];

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;

                self.try_rebuild_overlays();

                self.rebuild_policy = view::RebuildPolicy::Debounced {
                    last_input: Instant::now(),
                };
            }
            Message::ZoomAt { factor, cursor } => {
                let Some(size) = self.viewport_size_px() else {
                    return;
                };

                self.scene.camera.zoom_at_cursor(
                    factor,
                    cursor.x,
                    cursor.y,
                    size.width,
                    size.height,
                );

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
                self.force_rebuild_if_ybin_changed();

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
            Message::PauseBtnClicked => {
                match self.anchor {
                    view::Anchor::Live { .. } => {
                        unreachable!("PauseBtnClicked should only be possible when paused")
                    }
                    view::Anchor::Paused {
                        pending_mid_price,
                        scroll_ref_bucket,
                        render_latest_time,
                        x_phase_bucket,
                        resume,
                    } => {
                        self.anchor = view::Anchor::Live {
                            scroll_ref_bucket,
                            render_latest_time,
                            x_phase_bucket,
                            resume,
                        };

                        // On resume, if we have a pending mid-price update, apply it now.
                        if let Some(mid_price) = pending_mid_price {
                            self.base_price = mid_price;
                        }

                        if let Some(size) = self.viewport_size_px() {
                            self.scene.camera = Default::default();
                            self.scene.camera.reset_offset_x(size.width);
                            self.scene.camera.offset[1] = 0.0;
                        }

                        // Ensure we catch up once we’re live again.
                        if let view::Anchor::Live { resume, .. } = &mut self.anchor {
                            *resume = view::ResumeAction::FullRebuildFromHistorical;
                        }
                    }
                }

                self.rebuild_policy = view::RebuildPolicy::Immediate;
            }
        }
    }

    pub fn view(&self) -> iced::Element<'_, Message> {
        if self.trades.datapoints.is_empty() {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let render_latest_time = self.anchor.render_latest_time();
        let scroll_ref_bucket = self.anchor.scroll_ref_bucket();

        let aggr_time = match self.basis {
            Basis::Time(interval) => Some(u64::from(interval)),
            Basis::Tick(_) => None,
        };
        let latest_bucket: i64 = match aggr_time {
            Some(aggr) if aggr > 0 => (render_latest_time / aggr) as i64,
            _ => scroll_ref_bucket,
        };

        let x_axis = AxisXLabelCanvas {
            cache: &self.canvas_caches.x_axis,
            plot_bounds: self.viewport,
            latest_bucket,
            aggr_time,
            column_world: self.column_world,
            cam_offset_x: self.scene.camera.offset[0],
            cam_sx: self.scene.camera.scale[0].max(MIN_CAMERA_SCALE),
            cam_right_pad_frac: self.scene.camera.right_pad_frac,
            x_phase_bucket: self.anchor.x_phase_bucket(),
        };
        let y_axis = AxisYLabelCanvas {
            cache: &self.canvas_caches.y_axis,
            plot_bounds: self.viewport,
            base_price: self.base_price,
            step: self.step,
            row_h: self.row_h,
            cam_offset_y: self.scene.camera.offset[1],
            cam_sy: self.scene.camera.scale[1].max(MIN_CAMERA_SCALE),
            label_precision: self.ticker_info.min_ticksize,
        };

        let overlay = OverlayCanvas {
            scene: &self.scene,
            depth_grid: &self.depth_grid,
            base_price: self.base_price,
            step: self.step,
            scroll_ref_bucket,
            qty_scale_inv: 1.0 / DEPTH_QTY_SCALE,
            col_w_world: self.column_world,
            row_h_world: self.row_h,
            min_camera_scale: MIN_CAMERA_SCALE,
            tooltip_cache: &self.canvas_caches.overlay,
            scale_labels_cache: &self.canvas_caches.scale_labels,

            profile_col_width_px: PROFILE_COL_WIDTH_PX,
            strip_height_frac: STRIP_HEIGHT_FRAC,

            is_paused: self.anchor.is_paused(),
            volume_strip_max_qty: self.instances.profile_scale_max_qty,
            profile_max_qty: self.instances.volume_strip_scale_max_qty,
        };

        let chart = HeatmapShaderWidget::new(&self.scene, x_axis, y_axis, overlay);

        iced::widget::container(chart).padding(1).into()
    }

    /// called periodically on every frame, monitor refresh rates
    /// to update time-based rendering and animate/scroll
    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let now_i = now.unwrap_or_else(Instant::now);
        self.last_tick = Some(now_i);

        let Some(size) = self.viewport_size_px() else {
            if self.palette.is_none() {
                return Some(Action::RequestPalette);
            }
            return None;
        };

        let aggr_time: u64 = match self.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return None,
        };
        if aggr_time == 0 {
            return None;
        }
        if let Some(exchange_now_ms) = self.clock.estimate_now_ms(now_i) {
            // Bucket boundary can jitter due to clock estimation => make render time monotonic in Live mode.
            let bucketed = (exchange_now_ms / aggr_time) * aggr_time;

            let live_render_latest_time = match self.anchor {
                view::Anchor::Live {
                    render_latest_time, ..
                } => render_latest_time.max(bucketed),
                view::Anchor::Paused {
                    render_latest_time, ..
                } => render_latest_time,
            };

            let live_phase_ms = exchange_now_ms.saturating_sub(live_render_latest_time);
            let live_phase = (live_phase_ms as f32 / aggr_time as f32).clamp(0.0, 0.999_999);

            // Keep Live timing updated in the anchor (Paused remains frozen).
            self.anchor.update_live_timing(bucketed, live_phase);

            self.auto_update_anchor(size.width, size.height, live_render_latest_time, live_phase);

            if let Some(w) = self.compute_view_window(size.width, size.height) {
                let render_latest_time_eff = self.effective_render_latest_time();
                let render_bucket: i64 = (render_latest_time_eff / aggr_time) as i64;

                self.anchor.set_scroll_ref_bucket_if_zero(render_bucket);
                let scroll_ref_bucket = self.anchor.scroll_ref_bucket();

                let delta_buckets: i64 = render_bucket - scroll_ref_bucket;
                self.scene.params.origin[0] = (delta_buckets as f32) + self.anchor.x_phase_bucket();

                let latest_time_for_heatmap = if self.anchor.is_paused() {
                    render_latest_time_eff
                } else {
                    self.latest_time
                };

                let latest_bucket_for_scene: i64 = (latest_time_for_heatmap / aggr_time) as i64;
                let latest_rel: i64 = latest_bucket_for_scene - scroll_ref_bucket;
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

                let latest_incl = w.latest_vis.saturating_add(w.aggr_time);

                let is_interacting =
                    matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

                let denom = self.depth_norm.compute_throttled(
                    &self.heatmap,
                    &w,
                    latest_incl,
                    self.step,
                    self.data_gen,
                    now_i,
                    is_interacting,
                    NORM_RECOMPUTE_THROTTLE_MS,
                );
                self.scene.params.depth[0] = denom;

                self.scene.params.heatmap_a[1] = self
                    .depth_grid
                    .heatmap_y_start_bin(self.base_price, self.step);
            }

            self.canvas_caches.clear_axes();
        }

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }

        self.canvas_caches.clear_overlays();

        None
    }

    /// only data insertion point, called when new data arrives
    /// could be 1s, 500ms or 100ms, on par with aggregation interval but with additional network latency
    pub fn insert_datapoint(
        &mut self,
        trades_buffer: &[Trade],
        depth_update_t: u64,
        depth: &Depth,
    ) {
        // If rendering stalled (window hidden / OS hitch), resync heatmap texture on next data tick.
        if let Some(last) = self.last_tick
            && last.elapsed() >= Duration::from_millis(HEATMAP_RESYNC_AFTER_STALL_MS)
        {
            self.needs_heatmap_full_upload = true;
        }

        let paused = matches!(self.anchor, view::Anchor::Paused { .. });

        let aggr_time: u64 = match self.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => unimplemented!(),
        };

        self.clock = self.clock.anchor_with_update(depth_update_t);
        let rounded_t = (depth_update_t / aggr_time) * aggr_time;

        {
            let entry =
                self.trades
                    .datapoints
                    .entry(rounded_t)
                    .or_insert_with(|| HeatmapDataPoint {
                        grouped_trades: Box::new([]),
                        buy_sell: (0.0, 0.0),
                    });

            for trade in trades_buffer {
                entry.add_trade(trade, self.step);
            }
        }

        self.depth_grid.ensure_layout(aggr_time);
        self.heatmap.insert_latest_depth(depth, rounded_t);

        if rounded_t >= self.latest_time {
            if let Some(mid) = depth.mid_price() {
                let mid_rounded = mid.round_to_step(self.step);

                if let view::Anchor::Paused {
                    ref mut pending_mid_price,
                    ..
                } = self.anchor
                {
                    *pending_mid_price = Some(mid_rounded);
                } else {
                    self.base_price = mid_rounded;
                }
            }

            self.latest_time = rounded_t;
        }

        // While paused, avoid updating the GPU ring + uniforms per-tick; we'll catch up via
        // rebuild_from_historical on resume.
        if !paused {
            let steps_per_y_bin: i64 = self.scene.params.grid[2].round().max(1.0) as i64;

            self.depth_grid.ingest_snapshot(
                depth,
                rounded_t,
                self.step,
                DEPTH_QTY_SCALE,
                self.base_price,
                steps_per_y_bin,
            );

            let tex_w = self.depth_grid.tex_w();
            let tex_h = self.depth_grid.tex_h();

            if let Some(p) = &self.palette {
                self.scene.params.bid_rgb = [p.bid_rgb[0], p.bid_rgb[1], p.bid_rgb[2], 0.0];
                self.scene.params.ask_rgb = [p.ask_rgb[0], p.ask_rgb[1], p.ask_rgb[2], 0.0];
            }

            if tex_w > 0 && tex_h > 0 {
                let bucket = (rounded_t / aggr_time) as i64;

                self.anchor.set_scroll_ref_bucket_if_zero(bucket);
                let scroll_ref_bucket = self.anchor.scroll_ref_bucket();

                let latest_rel: i64 = bucket - scroll_ref_bucket;
                self.scene.params.heatmap_a[0] = latest_rel as f32;

                let latest_x_ring: u32 = self.depth_grid.ring_x_for_bucket(bucket);
                self.scene.params.heatmap_a[3] = latest_x_ring as f32;

                self.scene.params.heatmap_b = [
                    tex_w as f32,
                    tex_h as f32,
                    (tex_w - 1) as f32,
                    1.0 / DEPTH_QTY_SCALE,
                ];

                self.scene.params.heatmap_a[1] = self
                    .depth_grid
                    .heatmap_y_start_bin(self.base_price, self.step);

                let plan = self.depth_grid.build_scene_upload_plan();
                self.scene.apply_heatmap_upload_plan(plan);

                self.try_upload_heatmap();
            }
        }

        self.data_gen = self.data_gen.wrapping_add(1);
        self.depth_norm.invalidate();

        if (self.data_gen & 0x3F) == 0 {
            self.cleanup_old_data(aggr_time);
        }

        // If paused, don't force immediate rebuilds every tick; resume will trigger a full rebuild.
        if !paused {
            self.rebuild_policy = view::RebuildPolicy::Immediate;
        }
    }

    fn compute_view_window(&self, vw_px: f32, vh_px: f32) -> Option<ViewWindow> {
        let aggr_time: u64 = match self.basis {
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

        let latest_render = self.effective_render_latest_time();
        let latest_data_for_view = if self.anchor.is_paused() && latest_render > 0 {
            latest_render
        } else {
            self.latest_time
        };

        let input = ViewInputs {
            aggr_time,
            latest_time_data: latest_data_for_view,
            latest_time_render: latest_render,
            base_price: self.base_price,
            step: self.step,
            row_h_world: self.row_h,
            col_w_world: self.column_world,
        };

        ViewWindow::compute(cfg, &self.scene.camera, [vw_px, vh_px], input)
    }

    /// Rebuild only CPU overlay instances (profile/volume/trades). This is intended to be
    /// cheap enough to run during interaction, unlike `rebuild_from_historical`.
    fn rebuild_overlays(&mut self, w: &ViewWindow) {
        let Some(palette) = &self.palette else {
            return;
        };

        let (circles, rects) = self.instances.build_instances(
            w,
            &self.trades,
            &self.heatmap,
            self.base_price,
            self.step,
            self.latest_time,
            self.anchor.scroll_ref_bucket(),
            palette,
        );

        self.scene.set_rectangles(rects);
        self.scene.set_circles(circles);
    }

    /// Upload heatmap texture updates to the GPU.
    /// If `needs_heatmap_full_upload` is set, forces a full upload from the *current ring state*.
    fn try_upload_heatmap(&mut self) {
        let tex_w = self.depth_grid.tex_w();
        let tex_h = self.depth_grid.tex_h();
        if tex_w == 0 || tex_h == 0 {
            return;
        }

        if self.needs_heatmap_full_upload {
            self.depth_grid.force_full_upload();
        }

        let plan = self.depth_grid.build_scene_upload_plan();

        // Clear the flag only once we actually schedule a full upload.
        if matches!(plan, scene::HeatmapUploadPlan::Full(_)) {
            self.needs_heatmap_full_upload = false;
        }

        self.scene.apply_heatmap_upload_plan(plan);
    }

    fn try_rebuild_overlays(&mut self) {
        let Some(size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(size.width, size.height) else {
            return;
        };
        self.rebuild_overlays(&w);
    }

    fn viewport_size_px(&self) -> Option<iced::Size<f32>> {
        self.viewport.map(|r| r.size())
    }

    fn rebuild_instances(&mut self) {
        let Some(size) = self.viewport_size_px() else {
            return;
        };

        let Some(w) = self.compute_view_window(size.width, size.height) else {
            self.clear_scene();
            return;
        };

        let aggr_time: u64 = match self.basis {
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

        // Consume resume directive (if any).
        let resume = self.anchor.take_live_resume();
        let force_full_rebuild = matches!(resume, view::ResumeAction::FullRebuildFromHistorical);

        let need_full_rebuild = new_steps_per_y_bin != prev_steps_per_y_bin || force_full_rebuild;

        if need_full_rebuild {
            self.depth_grid.ensure_layout(aggr_time);

            let latest_time = if self.anchor.is_paused() {
                self.effective_render_latest_time().max(1)
            } else {
                self.latest_time.max(1)
            };

            let oldest_time =
                latest_time.saturating_sub(u64::from(DEPTH_GRID_HORIZON_BUCKETS) * aggr_time);

            let tex_h_i64 = self.depth_grid.tex_h().max(1) as i64;
            let half_bins = tex_h_i64 / 2;

            let step_units = self.step.units.max(1);
            let steps_per = new_steps_per_y_bin.max(1);

            let half_steps = half_bins.saturating_mul(steps_per);
            let delta_units = half_steps.saturating_mul(step_units);

            let base_u = self.base_price.units;
            let rebuild_highest = Price::from_units(base_u.saturating_add(delta_units));
            let rebuild_lowest = Price::from_units(base_u.saturating_sub(delta_units));

            self.depth_grid.rebuild_from_historical(
                &self.heatmap,
                oldest_time,
                latest_time,
                self.base_price,
                self.step,
                new_steps_per_y_bin,
                DEPTH_QTY_SCALE,
                rebuild_highest,
                rebuild_lowest,
            );

            self.data_gen = self.data_gen.wrapping_add(1);
            self.depth_norm.invalidate();

            let tex_w = self.depth_grid.tex_w();
            let tex_h = self.depth_grid.tex_h();

            if tex_w > 0 && tex_h > 0 {
                let latest_bucket: i64 = (latest_time / aggr_time) as i64;

                let scroll_ref_bucket = self.anchor.scroll_ref_bucket();
                let latest_rel: i64 = latest_bucket - scroll_ref_bucket;
                self.scene.params.heatmap_a[0] = latest_rel as f32;

                let latest_x_ring: u32 = self.depth_grid.ring_x_for_bucket(latest_bucket);
                self.scene.params.heatmap_a[3] = latest_x_ring as f32;

                self.scene.params.heatmap_b = [
                    tex_w as f32,
                    tex_h as f32,
                    (tex_w - 1) as f32,
                    1.0 / DEPTH_QTY_SCALE,
                ];

                self.scene.params.heatmap_a[1] = self
                    .depth_grid
                    .heatmap_y_start_bin(self.base_price, self.step);

                let plan = self.depth_grid.build_scene_upload_plan();
                self.scene.apply_heatmap_upload_plan(plan);

                self.try_upload_heatmap();
            }
        } else {
            self.scene.set_heatmap_update(None);

            if self.needs_heatmap_full_upload {
                self.try_upload_heatmap();
            }
        }

        self.rebuild_overlays(&w);
    }

    fn cleanup_old_data(&mut self, aggr_time: u64) {
        let aggr_time = aggr_time.max(1);

        // Keep CPU history aligned with what the ring can represent
        let keep_buckets: u64 = (self.depth_grid.tex_w().max(1)) as u64;

        let latest_time = self.latest_time;
        if latest_time == 0 {
            return;
        }

        let keep_ms = keep_buckets.saturating_mul(aggr_time);
        let cutoff = latest_time.saturating_sub(keep_ms);
        let cutoff_rounded = (cutoff / aggr_time) * aggr_time;

        // Prune trades (TimeSeries datapoints are bucket timestamps)
        let keep = self.trades.datapoints.split_off(&cutoff_rounded);
        self.trades.datapoints = keep;

        // Prune HistoricalDepth to match the oldest remaining trade bucket (if any),
        // otherwise prune by cutoff directly
        if let Some(oldest_time) = self.trades.datapoints.keys().next().copied() {
            self.heatmap.cleanup_old_price_levels(oldest_time);
        } else {
            self.heatmap.cleanup_old_price_levels(cutoff_rounded);
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
        // If we cleared GPU-side state, make sure we reupload the ring when we can render again.
        self.needs_heatmap_full_upload = true;
    }

    pub fn tick_size(&self) -> f32 {
        self.step.to_f32_lossy()
    }

    /// Render time used for view/overlay computations.
    /// While paused, clamp to the latest bucket we actually have data for to avoid "future" drift.
    #[inline]
    fn effective_render_latest_time(&self) -> u64 {
        let render_latest_time = self.anchor.render_latest_time();
        if self.anchor.is_paused() && render_latest_time > 0 && self.latest_time > 0 {
            render_latest_time.min(self.latest_time)
        } else {
            render_latest_time
        }
    }

    /// If the y-binning (steps_per_y_bin) would change, we must rebuild the heatmap texture
    /// immediately, otherwise overlays will be computed with a different binning than the shader/texture.
    fn force_rebuild_if_ybin_changed(&mut self) {
        let Some(size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(size.width, size.height) else {
            return;
        };

        let cur_steps_per_y_bin: i64 = self.scene.params.grid[2].round().max(1.0) as i64;
        if w.steps_per_y_bin != cur_steps_per_y_bin {
            self.rebuild_policy = view::RebuildPolicy::Immediate;
            self.rebuild_instances();
        }
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

    /// Is the *profile start boundary* (world x=0) visible on screen?
    /// Uses the camera's full world->screen mapping (includes right_pad_frac).
    #[inline]
    fn profile_start_visible_x0(&self, vw_px: f32, vh_px: f32) -> bool {
        if !vw_px.is_finite() || vw_px <= 1.0 || !vh_px.is_finite() || vh_px <= 1.0 {
            return true;
        }

        let y = self.scene.camera.offset[1];
        let [sx, _sy] = self.scene.camera.world_to_screen(0.0, y, vw_px, vh_px);

        sx.is_finite() && (0.0..=vw_px).contains(&sx)
    }

    /// Auto pause/resume follow based on whether the x=0 profile start boundary is visible.
    fn auto_update_anchor(
        &mut self,
        vw_px: f32,
        vh_px: f32,
        live_render_latest_time: u64,
        live_x_phase_bucket: f32,
    ) {
        let x0_visible = self.profile_start_visible_x0(vw_px, vh_px);

        match self.anchor {
            view::Anchor::Live {
                scroll_ref_bucket,
                render_latest_time,
                x_phase_bucket,
                ..
            } => {
                if !x0_visible {
                    self.anchor = view::Anchor::Paused {
                        render_latest_time: live_render_latest_time.max(render_latest_time),
                        x_phase_bucket: live_x_phase_bucket.max(x_phase_bucket),
                        pending_mid_price: None,
                        scroll_ref_bucket,
                        resume: view::ResumeAction::FullRebuildFromHistorical,
                    };
                }
            }
            view::Anchor::Paused {
                pending_mid_price,
                scroll_ref_bucket,
                render_latest_time,
                x_phase_bucket,
                resume,
            } => {
                if x0_visible {
                    self.anchor = view::Anchor::Live {
                        scroll_ref_bucket,
                        render_latest_time,
                        x_phase_bucket,
                        resume,
                    };

                    if let Some(p) = pending_mid_price {
                        self.base_price = p;
                    }

                    self.rebuild_policy = view::RebuildPolicy::Immediate;
                }
            }
        }
    }
}
