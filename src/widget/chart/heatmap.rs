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
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;

mod scene;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatmapPalette {
    pub bid_rgb: [f32; 3],
    pub ask_rgb: [f32; 3],
}

impl HeatmapPalette {
    pub fn from_theme(theme: &iced_core::Theme) -> Self {
        let bid = theme.extended_palette().success.strong.color;
        let ask = theme.extended_palette().danger.strong.color;

        Self {
            bid_rgb: [bid.r, bid.g, bid.b],
            ask_rgb: [ask.r, ask.g, ask.b],
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
            row_h: 0.1,
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
                self.rebuild_depth_rectangles();
            }
            Message::RowHeightChanged(h) => {
                self.row_h = h.max(0.0001);
                self.rebuild_depth_rectangles();
            }
            Message::Tick(now) => {
                self.last_tick = Some(now);
            }
            Message::PanDeltaPx(delta_px) => {
                let dx_world = delta_px.x / self.scene.camera.scale[0];
                let dy_world = delta_px.y / self.scene.camera.scale[1];

                self.scene.camera.offset[0] -= dx_world;
                self.scene.camera.offset[1] -= dy_world;

                self.rebuild_depth_rectangles();
            }
            Message::ZoomAt { factor, cursor } => {
                let Some([vw, vh]) = self.viewport else {
                    return;
                };

                self.scene
                    .camera
                    .zoom_at_cursor(factor, cursor.x, cursor.y, vw, vh);

                self.rebuild_depth_rectangles();
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

        self.rebuild_depth_rectangles();
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        let palette = HeatmapPalette::from_theme(theme);
        self.palette = Some(palette);
    }

    fn rebuild_depth_rectangles(&mut self) {
        let Some(palette) = &self.palette else {
            self.scene.set_rectangles(Vec::new());
            return;
        };

        let state = &self.data;

        let aggr_time: u64 = match state.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => return,
        };

        if state.latest_time == 0 || aggr_time == 0 {
            self.scene.set_rectangles(Vec::new());
            return;
        }

        let Some([vw_px, vh_px]) = self.viewport else {
            return;
        };

        // Camera semantics:
        // offset.x sits at the viewport RIGHT EDGE, offset.y at the vertical center.
        let sx = self.scene.camera.scale[0].max(1e-6);
        let sy = self.scene.camera.scale[1].max(1e-6);

        let x_max = self.scene.camera.offset[0];
        let x_min = x_max - (vw_px / sx);

        let y_center = self.scene.camera.offset[1];
        let half_h_world = (vh_px / sy) * 0.5;
        let y_min = y_center - half_h_world;
        let y_max = y_center + half_h_world;

        // Visible time window from visible world-x window.
        // Mapping: x = -((latest - t) / aggr_time)  =>  t = latest + x * aggr_time
        // Clamp to [0, latest] so "future" (x > 0) doesn't create a fake latest_vis > latest.
        let bucket_min = (x_min.floor() as i64).saturating_sub(2);
        let bucket_max = (x_max.ceil() as i64).saturating_add(2);

        let latest_t = state.latest_time as i128;
        let aggr_i = aggr_time as i128;

        let t_min_i = latest_t + (bucket_min as i128) * aggr_i;
        let t_max_i = latest_t + (bucket_max as i128) * aggr_i;

        let earliest = t_min_i.clamp(0, latest_t) as u64;
        let latest_vis = t_max_i.clamp(0, latest_t) as u64;

        if earliest >= latest_vis {
            self.scene.set_rectangles(Vec::new());
            return;
        }

        // Price window from visible world-y.
        let row_h = self.row_h.max(0.0001);

        // dy_steps = -(y_world / row_h)
        let min_steps = (-(y_max) / row_h).floor() as i64;
        let max_steps = (-(y_min) / row_h).ceil() as i64;

        let lowest = state.base_price.add_steps(min_steps, state.step);
        let highest = state.base_price.add_steps(max_steps, state.step);

        // Precompute buckets for snapping
        let latest_bucket: i64 = (state.latest_time / aggr_time) as i64;

        // Pass 1: normalization max within visible window
        let mut max_qty = 0.0f32;
        for (_price, runs) in state
            .heatmap
            .iter_time_filtered(earliest, latest_vis, highest, lowest)
        {
            for run in runs {
                let run_start = run.start_time.max(earliest);
                let run_until = run.until_time.min(latest_vis);
                if run_until > run_start {
                    max_qty = max_qty.max(run.qty());
                }
            }
        }

        if max_qty <= 0.0 {
            self.scene.set_rectangles(Vec::new());
            return;
        }

        // Pass 2: build instances
        let mut rects = Vec::new();

        for (price, runs) in state
            .heatmap
            .iter_time_filtered(earliest, latest_vis, highest, lowest)
        {
            let dy_steps = (price.units - state.base_price.units) / state.step.units;

            // Invert: higher price -> y up (negative world y)
            let y = -((dy_steps as f32) * row_h);

            for run in runs {
                let run_start = run.start_time.max(earliest);
                let run_until = run.until_time.min(latest_vis);
                if run_until <= run_start {
                    continue;
                }

                // Snap to buckets to avoid ragged "live edge" from non-aligned times.
                let start_bucket: i64 = (run_start / aggr_time) as i64;

                // Ceil to bucket boundary so the run fills the bucket it touches.
                let end_bucket_excl: i64 = ((run_until + aggr_time - 1) / aggr_time) as i64;

                let end_bucket_excl = end_bucket_excl.min(latest_bucket);

                let mut x_left = -((latest_bucket - start_bucket) as f32);
                let mut x_right = -((latest_bucket - end_bucket_excl) as f32);

                // Ensure ordering (just in case)
                if x_left > x_right {
                    std::mem::swap(&mut x_left, &mut x_right);
                }

                // Clip to visible x-range
                let x0 = x_left.clamp(x_min, x_max);
                let x1 = x_right.clamp(x_min, x_max);

                let w = (x1 - x0).max(0.0);
                if w <= 1e-6 {
                    continue;
                }

                let center_x = 0.5 * (x0 + x1);

                let a = (run.qty() / max_qty).clamp(0.05, 0.95);
                let rgb = if run.is_bid {
                    palette.bid_rgb
                } else {
                    palette.ask_rgb
                };
                let color = [rgb[0], rgb[1], rgb[2], a];

                rects.push(RectInstance {
                    position: [center_x, y],
                    size: [w, row_h],
                    color,
                });
            }
        }

        self.scene.set_rectangles(rects);
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
