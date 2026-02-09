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

use data::aggr::time::TimeSeries;
use data::chart::Basis;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, Trade};

use std::time::{Duration, Instant};

const DEPTH_GRID_HORIZON_BUCKETS: u32 = 4800;
const DEPTH_GRID_TEX_H: u32 = 2048; // steps around anchor

const DEFAULT_ROW_H_WORLD: f32 = 0.04;
const DEFAULT_COL_W_WORLD: f32 = 0.02;

const MAX_STEPS_PER_Y_BIN: i64 = 2048;

// Latest profile overlay (x > 0)
const PROFILE_COL_WIDTH_PX: f32 = 180.0;

// Volume strip
const STRIP_HEIGHT_FRAC: f32 = 0.10;

// Debounce heavy CPU rebuilds (notably `rebuild_from_historical`) during interaction
const REBUILD_DEBOUNCE_MS: u64 = 250;

// If rendering stalls longer than this, assume GPU heatmap texture may have been lost/desynced
const HEATMAP_RESYNC_AFTER_STALL_MS: u64 = 750;

// Trades profile (x < 0)
const TRADE_PROFILE_WIDTH_FRAC: f32 = 0.10;

#[derive(Debug, Clone)]
pub enum Message {
    BoundsChanged(iced::Rectangle),
    PanDeltaPx(iced::Vector),
    ZoomAt {
        factor: f32,
        cursor: iced::Point,
    },
    ScrolledAxisY {
        factor: f32,
        cursor_y: f32,
        viewport_h: f32,
    },
    AxisYDoubleClicked,
    AxisXDoubleClicked,
    ScrolledAxisX {
        factor: f32,
        cursor_x: f32,
        viewport_w: f32,
    },
    DragZoomAxisXKeepAnchor {
        factor: f32,
        anchor_screen_x: f32,
        viewport_w: f32,
    },
    PauseBtnClicked,
}

pub struct HeatmapShader {
    pub last_tick: Option<Instant>,
    scene: Scene,
    viewport: Option<iced::Rectangle>,
    cell: view::Cell,
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
    qty_scale: f32,

    depth_grid: depth_grid::GridRing,
    // Cache for depth normalization denom (max qty) to avoid per-frame scans.
    depth_norm: view::DepthNormCache,
    data_gen: u64,

    anchor: view::Anchor,
    rebuild_policy: view::RebuildPolicy,
}

impl HeatmapShader {
    pub fn new(basis: Basis, tick_size: f32, ticker_info: TickerInfo) -> Self {
        let step = PriceStep::from_f32(tick_size);

        let heatmap = HistoricalDepth::new(ticker_info.min_qty.into(), step, basis);
        let trades = TimeSeries::<HeatmapDataPoint>::new(basis, step);

        let depth_grid = depth_grid::GridRing::new(DEPTH_GRID_HORIZON_BUCKETS, DEPTH_GRID_TEX_H);

        let qty_scale: f32 = match exchange::volume_size_unit() {
            exchange::SizeUnit::Base => {
                let min_qty_f: f32 = ticker_info.min_qty.into();
                assert!(min_qty_f > 0.0, "ticker_info.min_qty must be > 0");
                1.0 / min_qty_f
            }
            exchange::SizeUnit::Quote => 1.0,
        };

        Self {
            last_tick: None,
            scene: Scene::new(),
            viewport: None,
            cell: view::Cell {
                width_world: DEFAULT_COL_W_WORLD,
                height_world: DEFAULT_ROW_H_WORLD,
            },
            palette: None,
            qty_scale,
            heatmap,
            step,
            basis,
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
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(bounds) => {
                self.viewport = Some(bounds);

                self.rebuild_policy = view::RebuildPolicy::Immediate;
                self.rebuild_all(None);
            }
            Message::DragZoomAxisXKeepAnchor {
                factor,
                anchor_screen_x,
                viewport_w,
            } => {
                self.scene.camera.zoom_column_world_keep_anchor(
                    factor,
                    0.0,
                    anchor_screen_x,
                    viewport_w,
                    &mut self.cell,
                );

                self.try_rebuild_overlays();
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::PanDeltaPx(delta_px) => {
                let cam_scale = self.scene.camera.scale();

                let dx_world = delta_px.x / cam_scale;
                let dy_world = delta_px.y / cam_scale;

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;

                self.try_rebuild_overlays();
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::ZoomAt { factor, cursor } => {
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());

                let Some(size) = self.viewport_size_px() else {
                    return;
                };

                // Enforce ">= 1px per row/col" even for uniform camera zoom.
                let cur_s = self.scene.camera.scale();

                let min_s_y = view::MIN_ROW_PX / self.cell.height_world.max(view::MIN_ROW_H_WORLD);
                let min_s_x = view::MIN_COL_PX / self.cell.width_world.max(view::MIN_COL_W_WORLD);
                let min_s = min_s_y.max(min_s_x);

                let desired_s = cur_s * factor;
                let clamped_s = desired_s.max(min_s);
                let effective_factor = if cur_s > 0.0 { clamped_s / cur_s } else { 1.0 };

                self.scene.camera.zoom_at_cursor(
                    effective_factor,
                    cursor.x,
                    cursor.y,
                    size.width,
                    size.height,
                );

                self.try_rebuild_overlays();
                self.force_rebuild_if_ybin_changed();
            }
            Message::ScrolledAxisY {
                factor,
                cursor_y,
                viewport_h,
            } => {
                self.scene
                    .camera
                    .zoom_row_h_at(factor, cursor_y, viewport_h, &mut self.cell);

                self.try_rebuild_overlays();
                self.force_rebuild_if_ybin_changed();

                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::ScrolledAxisX {
                factor,
                cursor_x,
                viewport_w,
            } => {
                self.scene.camera.zoom_column_world_at(
                    factor,
                    cursor_x,
                    viewport_w,
                    &mut self.cell,
                );

                self.try_rebuild_overlays();
                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::AxisYDoubleClicked => {
                self.scene.camera.offset[1] = 0.0;

                self.try_rebuild_overlays();
                self.force_rebuild_if_ybin_changed();

                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::AxisXDoubleClicked => {
                if let Some(size) = self.viewport_size_px() {
                    self.scene.camera.reset_offset_x(size.width);
                    self.cell.width_world = DEFAULT_COL_W_WORLD;

                    self.try_rebuild_overlays();
                    self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                }
            }
            Message::PauseBtnClicked => {
                let Ok((new_anchor, pending_price)) =
                    std::mem::take(&mut self.anchor).resume_from_pause()
                else {
                    unreachable!("PauseBtnClicked should only be possible when paused");
                };

                self.anchor = new_anchor;

                if let Some(mid_price) = pending_price {
                    self.base_price = mid_price;
                }

                if let Some(size) = self.viewport_size_px() {
                    self.scene.camera = Default::default();
                    self.scene.camera.reset_offset_x(size.width);
                    self.scene.camera.offset[1] = 0.0;
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

        let aggr_time = self.heatmap.aggr_time.max(1);
        let latest_bucket: i64 = (render_latest_time / aggr_time) as i64;

        let cam_scale = self.scene.camera.scale();

        let x_axis = AxisXLabelCanvas {
            cache: &self.canvas_caches.x_axis,
            plot_bounds: self.viewport,
            latest_bucket,
            aggr_time,
            column_world: self.cell.width_world,
            cam_offset_x: self.scene.camera.offset[0],
            cam_sx: cam_scale,
            cam_right_pad_frac: self.scene.camera.right_pad_frac,
            x_phase_bucket: self.anchor.x_phase_bucket(),
            is_x0_visible: self
                .viewport
                .map(|vb| self.scene.profile_start_visible_x0(vb.width, vb.height)),
        };
        let y_axis = AxisYLabelCanvas {
            cache: &self.canvas_caches.y_axis,
            plot_bounds: self.viewport,
            base_price: self.base_price,
            step: self.step,
            row_h: self.cell.height_world,
            cam_offset_y: self.scene.camera.offset[1],
            cam_sy: cam_scale,
            label_precision: self.ticker_info.min_ticksize,
        };

        let overlay = OverlayCanvas {
            scene: &self.scene,
            depth_grid: &self.depth_grid,
            base_price: self.base_price,
            step: self.step,
            scroll_ref_bucket,
            qty_scale_inv: 1.0 / self.qty_scale,
            cell_world: self.cell,
            tooltip_cache: &self.canvas_caches.overlay,
            scale_labels_cache: &self.canvas_caches.scale_labels,

            profile_col_width_px: PROFILE_COL_WIDTH_PX,
            strip_height_frac: STRIP_HEIGHT_FRAC,

            is_paused: self.anchor.is_paused(),
            volume_strip_max_qty: self.instances.volume_strip_scale_max_qty,
            profile_max_qty: self.instances.profile_scale_max_qty,
            trade_profile_max_qty: self.instances.trade_profile_scale_max_qty,
        };

        let chart = HeatmapShaderWidget::new(&self.scene, x_axis, y_axis, overlay);

        iced::widget::container(chart).padding(1).into()
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);

        self.scene.sync_palette(self.palette.as_ref());
    }

    pub fn tick_size(&self) -> f32 {
        self.step.to_f32_lossy()
    }

    /// called periodically on every window frame
    /// to update time-based rendering and animate/scroll
    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let now_i = now.unwrap_or_else(Instant::now);
        self.last_tick = Some(now_i);

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }
        let size = self.viewport_size_px()?;

        if let Some(exchange_now_ms) = self.clock.estimate_now_ms(now_i) {
            let aggr_time = self.heatmap.aggr_time.max(1);

            let (bucketed, live_render_latest_time, live_phase) =
                view::live_timing(self.anchor, exchange_now_ms, aggr_time);

            self.anchor.update_live_timing(bucketed, live_phase);
            self.auto_update_anchor(size.width, size.height, live_render_latest_time, live_phase);

            if let Some(w) = self.compute_view_window(size.width, size.height) {
                self.invalidate_with_view_window(now_i, aggr_time, &w);
            }

            self.canvas_caches.clear_axes();
        }

        self.canvas_caches.clear_overlays();
        None
    }

    /// the only data insertion point, called when new data arrives
    /// could be 1s, 500ms or 100ms, on par with aggregation interval but with additional network latency
    pub fn insert_datapoint(
        &mut self,
        trades_buffer: &[Trade],
        depth_update_t: u64,
        depth: &Depth,
    ) {
        self.mark_needs_full_upload_if_stalled();

        let paused = self.anchor.is_paused();
        let is_interacting = matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

        let aggr_time = self.heatmap.aggr_time.max(1);
        let rounded_t = view::round_time_to_bucket(depth_update_t, aggr_time);

        self.trades
            .ingest_trades_bucket(rounded_t, trades_buffer, self.step);

        if rounded_t < self.latest_time {
            return;
        }

        if let Some(mid) = depth.mid_price() {
            let mid_rounded = mid.round_to_step(self.step);
            self.anchor
                .apply_mid_price(mid_rounded, &mut self.base_price);
        }

        self.latest_time = rounded_t;

        self.clock = self.clock.anchor_with_update(depth_update_t);

        self.anchor
            .set_scroll_ref_bucket_if_zero((rounded_t / aggr_time) as i64);

        self.depth_grid.ensure_layout(aggr_time);
        self.heatmap.insert_latest_depth(depth, rounded_t);

        if !paused {
            self.update_live_ring_and_scene(depth, rounded_t, aggr_time, is_interacting);
        }

        self.data_gen = self.data_gen.wrapping_add(1);
        self.depth_norm.invalidate();

        if (self.data_gen & 0x3F) == 0 {
            self.cleanup_old_data(aggr_time);
        }

        if !paused && !is_interacting {
            self.rebuild_policy = view::RebuildPolicy::Immediate;
        }
    }

    fn cleanup_old_data(&mut self, aggr_time: u64) {
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

    fn compute_view_window(&self, vw_px: f32, vh_px: f32) -> Option<ViewWindow> {
        let cfg = ViewConfig {
            profile_col_width_px: PROFILE_COL_WIDTH_PX,
            strip_height_frac: STRIP_HEIGHT_FRAC,
            trade_profile_width_frac: TRADE_PROFILE_WIDTH_FRAC,
            max_steps_per_y_bin: MAX_STEPS_PER_Y_BIN,
        };

        let latest_render = self.anchor.effective_render_latest_time(self.latest_time);
        let latest_data_for_view = if self.anchor.is_paused() && latest_render > 0 {
            latest_render
        } else {
            self.latest_time
        };

        let input = ViewInputs {
            aggr_time: self.heatmap.aggr_time.max(1),
            latest_time_data: latest_data_for_view,
            latest_time_render: latest_render,
            base_price: self.base_price,
            step: self.step,
            cell: self.cell,
        };

        ViewWindow::compute(cfg, &self.scene.camera, [vw_px, vh_px], input)
    }

    /// Rebuild only CPU overlay instances (profile/volume/trades). This is intended to be
    /// cheap enough to run during interaction, unlike `rebuild_from_historical`.
    fn rebuild_overlays(&mut self, w: &ViewWindow) {
        let Some(palette) = &self.palette else {
            return;
        };

        // If we are interacting (debounced), keep overlays on the *same* y-binning
        let mut effective_window = *w;
        if matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. }) {
            let heatmap_steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();

            if effective_window.steps_per_y_bin != heatmap_steps_per_y_bin {
                effective_window.steps_per_y_bin = heatmap_steps_per_y_bin;
                effective_window.y_bin_h_world =
                    effective_window.row_h * (heatmap_steps_per_y_bin as f32);
            }
        }

        let built = self.instances.build_instances(
            &effective_window,
            &self.trades,
            &self.heatmap,
            self.base_price,
            self.step,
            self.latest_time,
            self.anchor.scroll_ref_bucket(),
            palette,
        );

        let draw_list = built.draw_list();

        self.scene.set_circles(built.circles);
        self.scene.set_rectangles(built.rects);
        self.scene.set_draw_list(draw_list);
    }

    fn try_rebuild_overlays(&mut self) {
        let Some(size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(size.width, size.height) else {
            return;
        };

        self.scene.params.set_cell_world(self.cell);
        self.sync_fade_params(&w);

        self.rebuild_overlays(&w);
    }

    fn rebuild_all(&mut self, window: Option<ViewWindow>) {
        let Some(w) = window.or_else(|| {
            let size = self.viewport_size_px()?;
            self.compute_view_window(size.width, size.height)
        }) else {
            self.scene.clear();
            self.depth_grid.force_full_upload();
            return;
        };

        self.sync_fade_params(&w);

        let aggr_time: u64 = self.heatmap.aggr_time.max(1);

        let prev_steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();
        let new_steps_per_y_bin: i64 = w.steps_per_y_bin.max(1);

        self.scene.params.set_cell_world(self.cell);
        self.scene.params.set_steps_per_y_bin(new_steps_per_y_bin);

        // Consume resume directive (if any).
        let resume = self.anchor.take_live_resume();
        let force_full_rebuild = matches!(resume, view::ResumeAction::FullRebuildFromHistorical);

        let recenter_target = {
            let row_h = self.cell.height_world;
            self.scene
                .camera
                .price_at_center(row_h, self.base_price, self.step)
        };

        let need_full_rebuild = self.depth_grid.should_full_rebuild(
            prev_steps_per_y_bin,
            new_steps_per_y_bin,
            recenter_target,
            self.step,
            force_full_rebuild,
        );

        if need_full_rebuild {
            self.depth_grid.ensure_layout(aggr_time);

            let latest_time = if self.anchor.is_paused() {
                self.anchor
                    .effective_render_latest_time(self.latest_time)
                    .max(1)
            } else {
                self.latest_time.max(1)
            };

            let (oldest_time, latest_time) = self
                .depth_grid
                .horizon_time_window_ms(latest_time, aggr_time);

            let (rebuild_highest, rebuild_lowest) = self.depth_grid.rebuild_price_bounds(
                recenter_target,
                self.step,
                new_steps_per_y_bin,
            );

            self.depth_grid.rebuild_from_historical(
                &self.heatmap,
                oldest_time,
                latest_time,
                recenter_target,
                self.step,
                new_steps_per_y_bin,
                self.qty_scale,
                rebuild_highest,
                rebuild_lowest,
            );

            self.data_gen = self.data_gen.wrapping_add(1);
            self.depth_norm.invalidate();

            self.scene.sync_heatmap_from_grid(
                &mut self.depth_grid,
                self.base_price,
                self.step,
                1.0 / self.qty_scale,
                latest_time,
                aggr_time,
                self.anchor.scroll_ref_bucket(),
                false,
            );
        } else {
            self.scene.set_heatmap_update(None);

            let latest_time_for_scene = if self.anchor.is_paused() {
                self.anchor
                    .effective_render_latest_time(self.latest_time)
                    .max(1)
            } else {
                self.latest_time.max(1)
            };

            self.scene.sync_heatmap_from_grid(
                &mut self.depth_grid,
                self.base_price,
                self.step,
                1.0 / self.qty_scale,
                latest_time_for_scene,
                aggr_time,
                self.anchor.scroll_ref_bucket(),
                false,
            );
        }

        self.rebuild_overlays(&w);
    }

    /// If the y-binning (steps_per_y_bin) would change, we must rebuild the heatmap texture.
    fn force_rebuild_if_ybin_changed(&mut self) {
        if matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. }) {
            return;
        }

        let Some(size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(size.width, size.height) else {
            return;
        };

        let cur_steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();
        if w.steps_per_y_bin != cur_steps_per_y_bin {
            self.rebuild_policy = view::RebuildPolicy::Immediate;
            self.rebuild_all(Some(w));
        }
    }

    fn viewport_size_px(&self) -> Option<iced::Size<f32>> {
        self.viewport.map(|r| r.size())
    }

    fn invalidate_with_view_window(&mut self, now_i: Instant, aggr_time: u64, w: &ViewWindow) {
        self.sync_fade_params(w);
        {
            let row_h = self.cell.height_world;
            let recenter_target =
                self.scene
                    .camera
                    .price_at_center(row_h, self.base_price, self.step);

            if self.depth_grid.should_recenter(recenter_target, self.step) {
                self.rebuild_policy = view::RebuildPolicy::Immediate;
            }
        }

        let aggr_time = aggr_time.max(1);
        let render_latest_time_eff = self.anchor.effective_render_latest_time(self.latest_time);
        let render_bucket: i64 = (render_latest_time_eff / aggr_time) as i64;

        let (scroll_ref_bucket, origin_x) = self.anchor.sync_scroll_ref_and_origin_x(render_bucket);

        let latest_time_for_heatmap = if self.anchor.is_paused() {
            render_latest_time_eff
        } else {
            self.latest_time
        };

        self.scene.sync_time_uniforms(
            &self.depth_grid,
            origin_x,
            latest_time_for_heatmap,
            aggr_time,
            scroll_ref_bucket,
        );

        let (do_overlays, do_full, next_policy) =
            self.rebuild_policy.decide(now_i, REBUILD_DEBOUNCE_MS);

        if do_overlays {
            self.rebuild_overlays(w);
        }
        if do_full {
            self.rebuild_all(Some(*w));
        }
        self.rebuild_policy = next_policy;

        self.update_depth_norm_and_params(*w, now_i);
    }

    fn update_depth_norm_and_params(&mut self, w: ViewWindow, now_i: Instant) {
        let latest_incl = w.latest_vis.saturating_add(w.aggr_time);
        let is_interacting = matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

        let denom = self.depth_norm.compute_throttled(
            &self.heatmap,
            &w,
            latest_incl,
            self.step,
            self.data_gen,
            now_i,
            is_interacting,
        );

        self.scene.params.set_depth_denom(denom);
    }

    #[inline]
    fn mark_needs_full_upload_if_stalled(&mut self) {
        if let Some(last) = self.last_tick
            && last.elapsed() >= Duration::from_millis(HEATMAP_RESYNC_AFTER_STALL_MS)
        {
            self.depth_grid.force_full_upload();
        }
    }

    fn update_live_ring_and_scene(
        &mut self,
        depth: &Depth,
        rounded_t: u64,
        aggr_time: u64,
        is_interacting: bool,
    ) {
        let steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();

        // During interaction, keep the current anchor to avoid churn.
        let recenter_target = if is_interacting {
            self.depth_grid.y_anchor_price().unwrap_or(self.base_price)
        } else {
            let row_h = self.cell.height_world;
            self.scene
                .camera
                .price_at_center(row_h, self.base_price, self.step)
        };

        self.depth_grid.ingest_snapshot(
            depth,
            rounded_t,
            self.step,
            self.qty_scale,
            recenter_target,
            steps_per_y_bin,
        );

        self.scene.sync_heatmap_from_grid(
            &mut self.depth_grid,
            self.base_price,
            self.step,
            1.0 / self.qty_scale,
            rounded_t,
            aggr_time,
            self.anchor.scroll_ref_bucket(),
            false,
        );
    }

    #[inline]
    fn sync_fade_params(&mut self, w: &ViewWindow) {
        let fade_start = w.trade_profile_max_w_world * 1.5;
        let fade_end = w.left_edge_world;

        // fade only in the left trade-profile zone
        self.scene
            .params
            .set_trade_fade(fade_end, fade_start, 0.15, 1.0);
    }

    /// Auto pause/resume follow based on whether the x=0 profile start boundary is visible.
    fn auto_update_anchor(
        &mut self,
        vw_px: f32,
        vh_px: f32,
        live_render_latest_time: u64,
        live_x_phase_bucket: f32,
    ) {
        let x0_visible = self.scene.profile_start_visible_x0(vw_px, vh_px);

        let (state_changed, pending_price) = self.anchor.update_auto_follow(
            x0_visible,
            live_render_latest_time,
            live_x_phase_bucket,
        );

        if let Some(price) = pending_price {
            self.base_price = price;
        }

        if state_changed {
            self.rebuild_policy = view::RebuildPolicy::Immediate;
        }
    }
}
