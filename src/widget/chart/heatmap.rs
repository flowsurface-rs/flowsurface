mod instance;
mod scene;
mod ui;
mod view;
mod widget;

use instance::InstanceBuilder;
use scene::{
    Scene,
    depth_grid::{GridRing, HeatmapPalette},
};
use ui::axisx::AxisXLabelCanvas;
use ui::axisy::AxisYLabelCanvas;
use ui::overlay::OverlayCanvas;
use ui::{CanvasCaches, CanvasInvalidation};
use view::{ViewConfig, ViewInputs, ViewWindow};
use widget::{DEFAULT_Y_AXIS_GUTTER, HeatmapShaderWidget};

use crate::{
    chart::Action,
    modal::pane::settings::study::{self, Study},
};
use data::aggr::time::TimeSeries;
use data::chart::{
    Basis,
    heatmap::{HeatmapDataPoint, HeatmapStudy, HistoricalDepth},
    indicator::HeatmapIndicator,
};
use exchange::depth::Depth;
use exchange::unit::{Price, PriceStep};
use exchange::{TickerInfo, Trade, UnixMs};

use std::time::{Duration, Instant};

// Volume strip
const STRIP_HEIGHT_FRAC: f32 = 0.10;

// Debounce heavy CPU rebuilds (notably `rebuild_from_historical`) during interaction
const REBUILD_DEBOUNCE_MS: u64 = 250;

// If rendering stalls longer than this, assume GPU heatmap texture may have been lost/desynced
const HEATMAP_RESYNC_AFTER_STALL_MS: u64 = 750;

// Volume profile width as % of viewport width
const VOLUME_PROFILE_WIDTH_PCT: f32 = 0.10;

// Depth profile width in pixels fixed
const DEPTH_PROFILE_WIDTH_PX: f32 = 160.0;

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
    CursorMoved,
    PauseBtnClicked,
}

pub struct HeatmapShader {
    pub last_tick: Option<Instant>,
    scene: Scene,
    viewport: Option<iced::Rectangle>,
    palette: Option<HeatmapPalette>,
    instances: InstanceBuilder,
    canvas_caches: CanvasCaches,
    canvas_invalidation: CanvasInvalidation,

    step: PriceStep,
    pub basis: Basis,
    pub ticker_info: TickerInfo,
    pub config: data::chart::heatmap::Config,
    trades: TimeSeries<HeatmapDataPoint>,
    depth_history: HistoricalDepth,

    latest_time: Option<u64>,
    base_price: Option<Price>,
    clock: view::ExchangeClock,
    anchor: view::Anchor,
    y_axis_gutter: iced::Length,

    depth_grid: GridRing,
    depth_norm: view::DepthNormCache,
    data_gen: u64,
    qty_scale: f32,
    rebuild_policy: view::RebuildPolicy,
    indicators: Vec<HeatmapIndicator>,
    pub studies: Vec<HeatmapStudy>,
    pub study_configurator: study::Configurator<HeatmapStudy>,
}

impl HeatmapShader {
    pub fn new(
        basis: Basis,
        step: PriceStep,
        ticker_info: TickerInfo,
        studies: Vec<HeatmapStudy>,
        indicators: Vec<HeatmapIndicator>,
    ) -> Self {
        let depth_history = HistoricalDepth::new(ticker_info.min_qty, step, basis);
        let trades = TimeSeries::<HeatmapDataPoint>::new(basis, step);

        let qty_scale: f32 = match exchange::unit::qty::volume_size_unit() {
            exchange::SizeUnit::Base => {
                let min_qty_f: f32 = ticker_info.min_qty.into();
                1.0 / min_qty_f
            }
            exchange::SizeUnit::Quote => 1.0,
        };

        Self {
            last_tick: None,
            scene: Scene::new(),
            viewport: None,
            palette: None,
            qty_scale,
            depth_history,
            step,
            basis,
            ticker_info,
            trades,
            latest_time: None,
            base_price: None,
            clock: view::ExchangeClock::Uninit,
            y_axis_gutter: DEFAULT_Y_AXIS_GUTTER,
            instances: InstanceBuilder::default(),
            canvas_caches: CanvasCaches::default(),
            canvas_invalidation: CanvasInvalidation::default(),
            depth_grid: GridRing::default(),
            depth_norm: view::DepthNormCache::new(),
            data_gen: 1,
            rebuild_policy: view::RebuildPolicy::Idle,
            indicators,
            anchor: view::Anchor::default(),
            config: data::chart::heatmap::Config::default(),
            studies,
            study_configurator: study::Configurator::new(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::BoundsChanged(bounds) => {
                self.viewport = Some(bounds);
                self.canvas_invalidation.mark_all();

                self.rebuild_policy = self.rebuild_policy.promote_to_immediate();
                self.rebuild_all(None);
            }
            Message::DragZoomAxisXKeepAnchor {
                factor,
                anchor_screen_x,
                viewport_w,
            } => {
                self.scene
                    .zoom_column_world_keep_anchor(factor, 0.0, anchor_screen_x, viewport_w);
                self.canvas_invalidation.mark_axis_x_motion();

                let resumed = self.try_resume_if_x0_visible();
                self.try_rebuild_instances();
                if !resumed {
                    self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                }
            }
            Message::PanDeltaPx(delta_px) => {
                let cam_scale = self.scene.camera.scale();

                let dx_world = delta_px.x / cam_scale;
                let dy_world = delta_px.y / cam_scale;

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;
                self.canvas_invalidation.mark_axes_motion();

                let resumed = self.try_resume_if_x0_visible();
                self.try_rebuild_instances();
                if !resumed {
                    self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                }
            }
            Message::ZoomAt { factor, cursor } => {
                let Some(size) = self.viewport_size_px() else {
                    return;
                };

                let current_scale = self.scene.camera.scale();
                let desired_scale = current_scale * factor;

                let Some((min_scale, max_scale)) = self.scene.cell.camera_scale_bounds_for_pixels()
                else {
                    return;
                };

                let target_scale = desired_scale.clamp(min_scale, max_scale);

                self.scene.camera.zoom_at_cursor_to_scale(
                    target_scale,
                    cursor.x,
                    cursor.y,
                    size.width,
                    size.height,
                );
                self.canvas_invalidation.mark_axes_motion();

                let resumed = self.try_resume_if_x0_visible();
                self.try_rebuild_instances();
                self.force_rebuild_if_ybin_changed();

                if !resumed {
                    self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                }
            }
            Message::ScrolledAxisY {
                factor,
                cursor_y,
                viewport_h,
            } => {
                self.scene.zoom_row_h_at(factor, cursor_y, viewport_h);
                self.canvas_invalidation.mark_axis_y_motion();

                self.try_rebuild_instances();
                self.force_rebuild_if_ybin_changed();

                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::ScrolledAxisX {
                factor,
                cursor_x,
                viewport_w,
            } => {
                self.scene
                    .zoom_column_world_at(factor, cursor_x, viewport_w);
                self.canvas_invalidation.mark_axis_x_motion();

                let resumed = self.try_resume_if_x0_visible();
                self.try_rebuild_instances();
                if !resumed {
                    self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                }
            }
            Message::AxisYDoubleClicked => {
                self.scene.camera.offset[1] = 0.0;
                self.canvas_invalidation.mark_axis_y_motion();

                self.try_rebuild_instances();
                self.force_rebuild_if_ybin_changed();

                self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
            }
            Message::AxisXDoubleClicked => {
                if let Some(size) = self.viewport_size_px() {
                    self.scene
                        .camera
                        .reset_to_live_edge(size.width, false, false);
                    self.scene.set_default_column_width();
                    self.canvas_invalidation.mark_axis_x_motion();

                    let resumed = self.try_resume_if_x0_visible();
                    self.try_rebuild_instances();
                    if !resumed {
                        self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                    }
                }
            }
            Message::PauseBtnClicked => {
                if let Some(size) = self.viewport_size_px() {
                    self.scene.camera.reset_to_live_edge(size.width, true, true);
                    self.canvas_invalidation.mark_axis_x_motion();

                    let resumed = self.try_resume_if_x0_visible();

                    self.try_rebuild_instances();

                    if !resumed {
                        self.rebuild_policy = self.rebuild_policy.mark_input(Instant::now());
                    }
                }
            }
            Message::CursorMoved => {
                self.canvas_invalidation
                    .mark_cursor_moved(self.anchor.is_paused());
            }
        }
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
        if self.base_price.is_none() {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let render_latest_time = self.anchor.render_latest_time();
        let scroll_ref_bucket = self.anchor.scroll_ref_bucket();
        let is_paused = self.anchor.is_paused();

        let aggr_time = self.depth_history.aggr_time_ms();
        let latest_bucket = (render_latest_time / aggr_time) as i64;
        let render_base_price = self.anchor.effective_base_price(self.base_price);

        let overlay_geometry = self.viewport.and_then(|vp| {
            view::OverlayGeometry::compute(
                &self.scene.camera,
                vp.size(),
                view::OverlayGeometryConfig {
                    depth_profile_col_width_px: DEPTH_PROFILE_WIDTH_PX,
                    volume_strip_height_pct: STRIP_HEIGHT_FRAC,
                    volume_profile_width_pct: VOLUME_PROFILE_WIDTH_PCT,
                },
            )
        });

        let x_axis = AxisXLabelCanvas {
            cache: &self.canvas_caches.x_axis,
            camera: &self.scene.camera,
            timezone,
            plot_bounds: self.viewport,
            is_paused,
            latest_bucket,
            aggr_time,
            column_world: self.scene.cell.width_world(),
            x_phase_bucket: self.anchor.x_phase_bucket(),
            is_x0_visible: self
                .viewport
                .map(|vp| self.scene.profile_start_visible_x0(vp.size())),
        };
        let y_axis = AxisYLabelCanvas {
            cache: &self.canvas_caches.y_axis,
            plot_bounds: self.viewport,
            is_paused,
            camera: &self.scene.camera,
            base_price: render_base_price,
            step: self.step,
            row_h: self.scene.cell.height_world(),
            label_precision: self.ticker_info.min_ticksize,
        };

        let overlay = OverlayCanvas {
            scene: &self.scene,
            depth_grid: &self.depth_grid,
            base_price: render_base_price,
            step: self.step,
            scroll_ref_bucket,
            qty_scale: self.qty_scale,
            tooltip_cache: &self.canvas_caches.overlay,
            scale_labels_cache: &self.canvas_caches.scale_labels,
            geometry: overlay_geometry,
            is_paused,
            volume_strip_max_qty: self.instances.volume_strip_scale_max_qty,
            depth_profile_max_qty: self.instances.depth_profile_scale_max_qty,
            volume_profile_max_qty: self.instances.volume_profile_scale_max_qty,
        };

        let chart = HeatmapShaderWidget::new(&self.scene, x_axis, y_axis, overlay)
            .with_y_axis_gutter(self.y_axis_gutter);

        iced::widget::container(chart).padding(1).into()
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);

        self.scene.sync_palette(self.palette.as_ref());
        self.canvas_invalidation.mark_all();
    }

    pub fn tick_size(&self) -> PriceStep {
        self.step
    }

    /// called periodically on every window frame
    /// to update time-based rendering and animate/scroll
    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let now_i = now.unwrap_or_else(Instant::now);
        self.last_tick = Some(now_i);

        if self.palette.is_none() {
            return Some(Action::RequestPalette);
        }
        let viewport_size = self.viewport_size_px()?;

        if let Some(exchange_now_ms) = self.clock.estimate_now_ms(now_i) {
            let aggr_time = self.depth_history.aggr_time_ms();
            let x0_visible = self.scene.profile_start_visible_x0(viewport_size);

            let state_change = self.anchor.tick_live_and_auto_follow(
                exchange_now_ms,
                aggr_time,
                x0_visible,
                self.base_price,
            );

            self.auto_update_anchor(state_change);

            if let Some(w) = self.compute_view_window(viewport_size) {
                self.invalidate_with_view_window(now_i, aggr_time, &w);
            }

            if !self.anchor.is_paused() {
                self.canvas_invalidation.mark_axis_x();
                self.canvas_invalidation.mark_overlay_tooltip();
            }
        }

        self.canvas_invalidation.apply(&self.canvas_caches);
        None
    }

    pub fn insert_depth(&mut self, depth: &Depth, update_t: UnixMs) {
        self.mark_needs_full_upload_if_stalled();
        let prev_effective_base = self.anchor.effective_base_price(self.base_price);

        let paused = self.anchor.is_paused();
        let is_interacting = matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

        let aggr_interval = self.depth_history.aggr_time;
        let aggr_time = aggr_interval.to_milliseconds();
        let update_t_ms = update_t.as_u64();
        let rounded_t = update_t.floor_to(aggr_interval);
        let rounded_t_ms = rounded_t.as_u64();

        if let Some(mid) = depth.mid_price() {
            let mid_rounded = mid.round_to_step(self.step);
            self.anchor
                .apply_mid_price(mid_rounded, &mut self.base_price);
        }

        self.latest_time = Some(rounded_t_ms);

        self.clock = self.clock.anchor_with_update(update_t_ms);

        self.anchor
            .set_scroll_ref_bucket_if_zero((rounded_t_ms / aggr_time) as i64);

        self.depth_grid.ensure_layout(aggr_time);
        self.depth_history.insert_latest_depth(depth, rounded_t);

        if !paused {
            self.update_live_ring_and_scene(depth, rounded_t_ms, is_interacting);
        }

        self.data_gen = self.data_gen.wrapping_add(1);

        if (self.data_gen & 0x3F) == 0 {
            self.cleanup_old_data(aggr_time);
        }

        if !paused && !is_interacting {
            self.rebuild_policy = self.rebuild_policy.promote_to_immediate();
        }

        if self.anchor.effective_base_price(self.base_price) != prev_effective_base {
            self.refresh_y_axis_gutter();
            self.canvas_invalidation.mark_axis_y();
        }
    }

    pub fn insert_trades(&mut self, buffer: &[Trade], update_t: UnixMs) {
        if buffer.is_empty() {
            return;
        }

        let aggr_interval = self.depth_history.aggr_time;
        let rounded_t = update_t.floor_to(aggr_interval);

        self.trades
            .ingest_trades_bucket(rounded_t, buffer, self.step);

        self.canvas_invalidation.mark_overlay_tooltip();
        self.canvas_invalidation.mark_overlay_scale_labels();
        self.try_rebuild_instances();
    }

    pub fn visual_config(&self) -> data::chart::heatmap::Config {
        self.config
    }

    pub fn study_configurator(&self) -> &study::Configurator<HeatmapStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<HeatmapStudy>) {
        let studies = &mut self.studies;
        let mut studies_changed = false;

        match self.study_configurator.update(message) {
            Some(study::Action::ToggleStudy(study, is_selected)) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                        studies_changed = true;
                    }
                } else {
                    let before = studies.len();
                    studies.retain(|s| !s.is_same_type(&study));
                    studies_changed = studies.len() != before;
                }
            }
            Some(study::Action::ConfigureStudy(study)) => {
                if let Some(existing_study) = studies.iter_mut().find(|s| s.is_same_type(&study)) {
                    *existing_study = study;
                    studies_changed = true;
                }
            }
            None => {}
        }

        if !studies_changed {
            return;
        }

        self.canvas_invalidation.mark_overlay_tooltip();
        self.canvas_invalidation.mark_overlay_scale_labels();
        self.try_rebuild_instances();
    }

    pub fn set_visual_config(&mut self, config: data::chart::heatmap::Config) {
        if self.config == config {
            return;
        }

        let prev = self.config;
        self.config = config;

        let order_filter_changed =
            prev.order_size_filter.to_bits() != self.config.order_size_filter.to_bits();
        let trade_visual_changed = prev.trade_size_filter.to_bits()
            != self.config.trade_size_filter.to_bits()
            || prev.trade_size_scale != self.config.trade_size_scale;

        self.canvas_invalidation.mark_overlay_tooltip();
        self.canvas_invalidation.mark_overlay_scale_labels();

        if trade_visual_changed || order_filter_changed {
            self.try_rebuild_instances();
        }

        if order_filter_changed {
            self.data_gen = self.data_gen.wrapping_add(1);
            self.rebuild_policy = self
                .rebuild_policy
                .request_rebuild_from_historical()
                .mark_input(Instant::now());
        }
    }

    pub fn toggle_indicator(&mut self, indicator: HeatmapIndicator) {
        if self.indicators.contains(&indicator) {
            self.indicators.retain(|i| i != &indicator);
        } else {
            self.indicators.push(indicator);
        }

        self.canvas_invalidation.mark_overlay_tooltip();
        self.canvas_invalidation.mark_overlay_scale_labels();
        self.try_rebuild_instances();
    }

    fn cleanup_old_data(&mut self, aggr_time: u64) {
        // Keep CPU history aligned with what the ring can represent
        let keep_buckets: u64 = (self.depth_grid.tex_w().max(1)) as u64;

        let Some(latest_time) = self.latest_time else {
            return;
        };

        let keep_ms = keep_buckets.saturating_mul(aggr_time);
        let cutoff = latest_time.saturating_sub(keep_ms);
        let cutoff_rounded = (cutoff / aggr_time) * aggr_time;

        // Prune trades (TimeSeries datapoints are bucket timestamps)
        let keep = self
            .trades
            .datapoints
            .split_off(&UnixMs::new(cutoff_rounded));
        self.trades.datapoints = keep;

        // Prune HistoricalDepth to match the oldest remaining trade bucket (if any),
        // otherwise prune by cutoff directly
        if let Some(oldest_time) = self.trades.datapoints.keys().next().copied() {
            self.depth_history.cleanup_old_price_levels(oldest_time);
        } else {
            self.depth_history
                .cleanup_old_price_levels(UnixMs::new(cutoff_rounded));
        }
    }

    fn compute_view_window(&self, viewport_size: iced::Size) -> Option<ViewWindow> {
        let latest_time = self.latest_time?;
        let base_price = self.anchor.effective_base_price(self.base_price)?;

        let cfg = ViewConfig {
            depth_profile_col_width_px: DEPTH_PROFILE_WIDTH_PX,
            volume_strip_height_pct: STRIP_HEIGHT_FRAC,
            volume_profile_width_pct: VOLUME_PROFILE_WIDTH_PCT,
            max_steps_per_y_bin: i64::from(self.depth_grid.tex_h()),
        };

        let latest_render = self.anchor.effective_render_latest_time(latest_time);
        let latest_data_for_view = if self.anchor.is_paused() && latest_render > 0 {
            latest_render
        } else {
            latest_time
        };

        let input = ViewInputs {
            aggr_time: self.depth_history.aggr_time_ms(),
            latest_time_data: latest_data_for_view,
            latest_time_render: latest_render,
            base_price,
            step: self.step,
            cell: self.scene.cell,
        };

        ViewWindow::compute(cfg, &self.scene.camera, viewport_size, input)
    }

    /// Rebuild only CPU overlay instances (profile/volume/trades). This is intended to be
    /// cheap enough to run during interaction, unlike [`GridRing::rebuild_from_historical()`].
    fn rebuild_instances(&mut self, w: &ViewWindow) {
        let Some(palette) = &self.palette else {
            return;
        };
        let latest_time = match self.latest_time {
            Some(time) => time,
            None => return,
        };
        let base_price = match self.anchor.effective_base_price(self.base_price) {
            Some(price) => price,
            None => return,
        };

        // Keep trade-profile fade params synchronized with the same window used for
        // instance building so overlay labels anchor to current geometry immediately.
        self.scene.params.set_trade_fade(w);

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

        let volume_profile = self
            .studies
            .iter()
            .map(|study| match study {
                HeatmapStudy::VolumeProfile(profile) => profile,
            })
            .next();

        let latest_depth = self
            .depth_history
            .latest_order_runs(
                effective_window.highest,
                effective_window.lowest,
                UnixMs::new(latest_time),
            )
            .map(|(price, run)| (*price, run.qty, run.is_bid));

        let show_volume_indicator = self.indicators.contains(&HeatmapIndicator::Volume);

        let built = self.instances.build_instances(
            &effective_window,
            &self.trades,
            latest_depth,
            base_price,
            self.step,
            latest_time,
            self.anchor.scroll_ref_bucket(),
            palette,
            &self.config,
            &self.ticker_info.market_type(),
            volume_profile,
            show_volume_indicator,
        );

        let draw_list = built.draw_list();

        self.scene.set_circles(built.circles);
        self.scene.set_rectangles(built.rects);
        self.scene.set_draw_list(draw_list);
        self.canvas_invalidation.mark_overlay_scale_labels();
    }

    fn try_rebuild_instances(&mut self) {
        let Some(size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(size) else {
            return;
        };

        self.rebuild_instances(&w);
    }

    fn rebuild_all(&mut self, window: Option<ViewWindow>) {
        let Some(w) = window.or_else(|| {
            let size = self.viewport_size_px()?;
            self.compute_view_window(size)
        }) else {
            self.scene.clear();
            self.depth_grid.force_full_upload();
            return;
        };

        let latest_time = match self.latest_time {
            Some(time) => time,
            None => return,
        };
        let base_price = match self.anchor.effective_base_price(self.base_price) {
            Some(price) => price,
            None => return,
        };

        let aggr_time: u64 = self.depth_history.aggr_time_ms();
        let market_type = self.ticker_info.market_type();
        let size_in_quote_ccy =
            exchange::unit::qty::volume_size_unit() == exchange::SizeUnit::Quote;
        let order_size_filter = self.config.order_size_filter.max(0.0);

        let prev_steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();
        let new_steps_per_y_bin: i64 = w.steps_per_y_bin.max(1);

        self.scene.params.set_steps_per_y_bin(new_steps_per_y_bin);

        // Consume one-shot rebuild directives.
        let force_from_policy = self.rebuild_policy.take_force_rebuild_from_historical();
        let force_full_rebuild = force_from_policy;

        let recenter_target = self.scene.price_at_center(base_price, self.step);

        let need_full_rebuild = self.depth_grid.should_full_rebuild(
            prev_steps_per_y_bin,
            new_steps_per_y_bin,
            recenter_target,
            self.step,
            force_full_rebuild,
        );

        let effective_latest = if self.anchor.is_paused() {
            self.anchor.effective_render_latest_time(latest_time).max(1)
        } else {
            latest_time.max(1)
        };

        let final_latest = if need_full_rebuild {
            self.depth_grid.ensure_layout(aggr_time);

            let (oldest, newest) = self
                .depth_grid
                .horizon_time_window_ms(effective_latest, aggr_time);

            let (rebuild_highest, rebuild_lowest) = self.depth_grid.rebuild_price_bounds(
                recenter_target,
                self.step,
                new_steps_per_y_bin,
            );

            self.depth_grid.rebuild_from_historical(
                &self.depth_history,
                oldest,
                newest,
                recenter_target,
                self.step,
                new_steps_per_y_bin,
                self.qty_scale,
                rebuild_highest,
                rebuild_lowest,
                &market_type,
                size_in_quote_ccy,
                order_size_filter,
            );

            self.data_gen = self.data_gen.wrapping_add(1);

            newest
        } else {
            effective_latest
        };

        self.scene.sync_heatmap_texture(
            &self.depth_grid,
            base_price,
            self.step,
            self.qty_scale,
            final_latest,
            aggr_time,
            self.anchor.scroll_ref_bucket(),
        );

        // Guard for callers that trigger `rebuild_all` outside `invalidate` (e.g. resume
        // actions from `update`) which can lead to stale texture data but new y-mapping.
        self.scene
            .sync_heatmap_upload_from_grid(&mut self.depth_grid, need_full_rebuild);

        self.rebuild_instances(&w);
    }

    /// If the y-binning (steps_per_y_bin) would change, we must rebuild the heatmap texture.
    fn force_rebuild_if_ybin_changed(&mut self) {
        if matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. }) {
            return;
        }

        let Some(viewport_size) = self.viewport_size_px() else {
            return;
        };
        let Some(w) = self.compute_view_window(viewport_size) else {
            return;
        };

        let cur_steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();
        if w.steps_per_y_bin != cur_steps_per_y_bin {
            self.rebuild_policy = self.rebuild_policy.promote_to_immediate();
            self.rebuild_all(Some(w));
        }
    }

    fn viewport_size_px(&self) -> Option<iced::Size<f32>> {
        self.viewport.map(|r| r.size())
    }

    fn invalidate_with_view_window(&mut self, now_i: Instant, aggr_time: u64, w: &ViewWindow) {
        let latest_time = match self.latest_time {
            Some(time) => time,
            None => return,
        };
        let base_price = match self.anchor.effective_base_price(self.base_price) {
            Some(price) => price,
            None => return,
        };

        self.scene.params.set_trade_fade(w);
        {
            let recenter_target = self.scene.price_at_center(base_price, self.step);

            if self.depth_grid.should_recenter(recenter_target, self.step) {
                // Recenter implies a y-mapping change: force rebuild-from-historical so older cols
                // get repopulated under the new anchor
                self.rebuild_policy = self
                    .rebuild_policy
                    .request_rebuild_from_historical()
                    .promote_to_immediate();
            }
        }

        let aggr_time = aggr_time.max(1);
        let render_latest_time_eff = self.anchor.effective_render_latest_time(latest_time);
        let render_bucket: i64 = (render_latest_time_eff / aggr_time) as i64;

        let (scroll_ref_bucket, origin_x) = self.anchor.sync_scroll_ref_and_origin_x(render_bucket);

        let latest_time_for_heatmap = if self.anchor.is_paused() {
            render_latest_time_eff
        } else {
            latest_time
        };

        self.scene.params.set_origin_x(origin_x);
        self.scene.sync_heatmap_texture(
            &self.depth_grid,
            base_price,
            self.step,
            self.qty_scale,
            latest_time_for_heatmap,
            aggr_time,
            scroll_ref_bucket,
        );

        let (do_overlays, do_full, next_policy) =
            self.rebuild_policy.decide(now_i, REBUILD_DEBOUNCE_MS);

        if do_overlays {
            self.rebuild_instances(w);
        }
        if do_full {
            self.rebuild_all(Some(*w));
        }

        self.scene
            .sync_heatmap_upload_from_grid(&mut self.depth_grid, false);

        self.rebuild_policy = next_policy;

        self.update_depth_norm_and_params(*w, now_i);
    }

    fn update_depth_norm_and_params(&mut self, w: ViewWindow, now_i: Instant) {
        let latest_incl = w.latest_vis.saturating_add(w.aggr_time);
        let is_interacting = matches!(self.rebuild_policy, view::RebuildPolicy::Debounced { .. });

        let norm_gen = if is_interacting || self.anchor.is_paused() {
            self.data_gen
        } else {
            latest_incl / w.aggr_time.max(1)
        };

        let denom = self.depth_norm.compute_throttled(
            &self.depth_history,
            &w,
            latest_incl,
            self.step,
            &self.ticker_info.market_type(),
            self.config.order_size_filter.max(0.0),
            norm_gen,
            now_i,
            is_interacting,
        );

        self.scene.params.set_depth_denom(denom);
    }

    fn mark_needs_full_upload_if_stalled(&mut self) {
        if let Some(last) = self.last_tick
            && last.elapsed() >= Duration::from_millis(HEATMAP_RESYNC_AFTER_STALL_MS)
        {
            self.depth_grid.force_full_upload();
        }
    }

    /// Resume paused mode immediately if x=0 becomes visible due user input (pan/zoom/reset).
    /// Returns whether a Paused -> Live transition happened.
    fn try_resume_if_x0_visible(&mut self) -> bool {
        if !self.anchor.is_paused() {
            return false;
        }

        let Some(viewport_size) = self.viewport_size_px() else {
            return false;
        };

        if !self.scene.profile_start_visible_x0(viewport_size) {
            return false;
        }

        let resumed = self.anchor.resume_to_live();
        if resumed {
            self.refresh_y_axis_gutter();
            self.canvas_invalidation.mark_all();

            self.rebuild_policy = self
                .rebuild_policy
                .request_rebuild_from_historical()
                .promote_to_immediate();

            self.rebuild_all(None);
            self.rebuild_policy = view::RebuildPolicy::Idle;
        }

        resumed
    }

    fn update_live_ring_and_scene(&mut self, depth: &Depth, rounded_t: u64, is_interacting: bool) {
        let Some(base_price) = self.base_price else {
            return;
        };

        let steps_per_y_bin: i64 = self.scene.params.steps_per_y_bin();

        let recenter_target = if is_interacting {
            self.depth_grid.y_anchor_price().unwrap_or(base_price)
        } else {
            self.scene.price_at_center(base_price, self.step)
        };

        // If live ingest is about to recenter, schedule a forced rebuild-from-historical
        if self.depth_grid.should_recenter(recenter_target, self.step) {
            self.rebuild_policy = self
                .rebuild_policy
                .request_rebuild_from_historical()
                .promote_to_immediate();
        }

        self.depth_grid.ingest_snapshot(
            depth,
            rounded_t,
            self.step,
            self.qty_scale,
            recenter_target,
            steps_per_y_bin,
            &self.ticker_info.market_type(),
            exchange::unit::qty::volume_size_unit() == exchange::SizeUnit::Quote,
            self.config.order_size_filter.max(0.0),
        );
    }

    /// Apply follow-transition side effects after anchor timing/auto-follow has been updated.
    fn auto_update_anchor(&mut self, state_change: view::FollowStateChange) {
        if state_change == view::FollowStateChange::Unchanged {
            return;
        }

        self.refresh_y_axis_gutter();
        self.canvas_invalidation.mark_all();
        self.rebuild_policy = self.rebuild_policy.promote_to_immediate();

        if state_change == view::FollowStateChange::ResumedToLive {
            self.rebuild_policy = self
                .rebuild_policy
                .request_rebuild_from_historical()
                .promote_to_immediate();

            self.rebuild_all(None);
            self.rebuild_policy = view::RebuildPolicy::Idle;
        }
    }

    fn refresh_y_axis_gutter(&mut self) {
        self.y_axis_gutter = AxisYLabelCanvas::width(
            self.anchor.effective_base_price(self.base_price),
            self.ticker_info.min_ticksize,
        )
        .unwrap_or(DEFAULT_Y_AXIS_GUTTER);
    }
}
