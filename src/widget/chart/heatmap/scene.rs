pub mod camera;
pub mod cell;
pub mod depth_grid;
pub mod pipeline;
mod uniform;

use super::Message;
use cell::Cell;
use pipeline::Pipeline;
use pipeline::circle::CircleInstance;
use pipeline::rectangle::RectInstance;
use pipeline::{DrawItem, DrawLayer, DrawOp};
use uniform::ParamsUniform;

use iced::Rectangle;
use iced::mouse;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const PX_PER_NOTCH: f32 = 120.0;
const ZOOM_BASE_PER_NOTCH: f32 = 1.08;
const MAX_ABS_NOTCHES_PER_EVENT: f32 = 6.0;

#[derive(Clone, Debug)]
pub struct HeatmapColumnCpu {
    pub x: u32,              // ring x index to update
    pub bid_col: Arc<[u32]>, // len == height
    pub ask_col: Arc<[u32]>, // len == height
}

#[derive(Clone, Debug)]
pub enum HeatmapUpload {
    Full {
        width: u32,
        height: u32,
        bid: Arc<[u32]>, // len == width*height
        ask: Arc<[u32]>, // len == width*height
    },
    Cols {
        width: u32,
        height: u32,
        cols: Arc<[HeatmapColumnCpu]>,
    },
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Scene {
    pub id: u64,

    pub rectangles: Arc<[RectInstance]>,
    pub rectangles_gen: u64,

    pub circles: Arc<[CircleInstance]>,
    pub circles_gen: u64,

    pub draw_list: Arc<[DrawItem]>,

    pub camera: camera::Camera,
    pub params: ParamsUniform,

    pub heatmap_tex_gen: u64,
    heatmap_upload: Option<(u64, HeatmapUpload)>,

    pub cell: cell::Cell,
}

impl Scene {
    pub fn new() -> Self {
        let cell = Cell::default();

        let mut params = ParamsUniform::default();
        params.set_cell_world(cell.width_world(), cell.height_world());

        Self {
            id: next_scene_id(),
            rectangles: Arc::from(Vec::<RectInstance>::new()),
            rectangles_gen: 1,
            circles: Arc::from(Vec::<CircleInstance>::new()),
            circles_gen: 1,
            draw_list: Arc::from(Vec::<DrawItem>::new()),
            camera: camera::Camera::default(),
            params,
            heatmap_tex_gen: 1,
            heatmap_upload: None,
            cell,
        }
    }

    pub fn clear(&mut self) {
        self.set_rectangles(vec![]);
        self.set_circles(vec![]);
        self.set_draw_list(vec![DrawItem::new(DrawLayer::HEATMAP, DrawOp::Heatmap)]);
        self.heatmap_upload = None;
    }

    pub fn set_rectangles(&mut self, rectangles: Vec<RectInstance>) {
        self.rectangles = Arc::from(rectangles);
        self.rectangles_gen = self.rectangles_gen.wrapping_add(1);
    }

    pub fn set_circles(&mut self, circles: Vec<CircleInstance>) {
        self.circles = Arc::from(circles);
        self.circles_gen = self.circles_gen.wrapping_add(1);
    }

    /// Layering API: the draw order is *only* defined by `draw_list`.
    pub fn set_draw_list(&mut self, mut draw_list: Vec<DrawItem>) {
        draw_list.sort_by_key(|d| d.layer);
        self.draw_list = Arc::from(draw_list);
    }

    #[inline]
    fn bump_heatmap_gen(&mut self) -> u64 {
        self.heatmap_tex_gen = self.heatmap_tex_gen.wrapping_add(1);
        self.heatmap_tex_gen
    }

    fn schedule_heatmap_upload(&mut self, upload: HeatmapUpload) {
        let generation = self.bump_heatmap_gen();
        self.heatmap_upload = Some((generation, upload));
    }

    /// Is the *profile start boundary* (world x=0) visible on screen?
    /// Uses the camera's full world->screen mapping (includes right_pad_frac).
    #[inline]
    pub fn profile_start_visible_x0(&self, viewport_size: iced::Size) -> bool {
        let [vw_px, vh_px] = viewport_size.into();

        if !vw_px.is_finite() || vw_px <= 1.0 || !vh_px.is_finite() || vh_px <= 1.0 {
            return true;
        }

        let y = self.camera.offset[1];
        let [sx, _sy] = self.camera.world_to_screen(0.0, y, vw_px, vh_px);

        sx.is_finite() && (0.0..=vw_px).contains(&sx)
    }

    pub fn set_default_column_width(&mut self) {
        self.cell.set_default_width();
        self.sync_cell_world_uniform();
    }

    #[inline]
    fn sync_cell_world_uniform(&mut self) {
        self.params
            .set_cell_world(self.cell.width_world(), self.cell.height_world());
    }

    /// Synchronizes heatmap texture parameters with the depth grid state
    pub fn sync_heatmap_texture(
        &mut self,
        depth_grid: &depth_grid::GridRing,
        base_price: exchange::util::Price,
        step: exchange::util::PriceStep,
        qty_scale_inv: f32, // 1.0 / qty_scale
        latest_time: u64,
        aggr_time: u64,
        scroll_ref_bucket: i64,
    ) {
        let tex_w = depth_grid.tex_w();
        let tex_h = depth_grid.tex_h();

        if tex_w == 0 || tex_h == 0 {
            return;
        }

        let latest_bucket: i64 = (latest_time / aggr_time) as i64;
        let latest_rel: i64 = latest_bucket - scroll_ref_bucket;

        self.params.set_heatmap_latest_rel_bucket(latest_rel);

        let latest_x_ring: u32 = depth_grid.ring_x_for_bucket(latest_bucket);
        self.params.set_heatmap_latest_x_ring(latest_x_ring);

        self.params
            .set_heatmap_tex_info(tex_w, tex_h, qty_scale_inv);

        self.params
            .set_heatmap_y_start_bin(depth_grid.heatmap_y_start_bin(base_price, step));
    }

    /// Sync heatmap uniforms that depend on time/x-scrolling:
    /// - latest_rel_bucket
    /// - latest_x_ring
    #[inline]
    pub fn sync_heatmap_time_uniforms(
        &mut self,
        depth_grid: &depth_grid::GridRing,
        latest_time: u64,
        aggr_time: u64,
        scroll_ref_bucket: i64,
    ) {
        let tex_w = depth_grid.tex_w();
        let tex_h = depth_grid.tex_h();
        if tex_w == 0 || tex_h == 0 {
            return;
        }

        let aggr_time = aggr_time.max(1);

        let latest_bucket: i64 = (latest_time / aggr_time) as i64;
        let latest_rel: i64 = latest_bucket - scroll_ref_bucket;
        self.params.set_heatmap_latest_rel_bucket(latest_rel);

        let latest_x_ring: u32 = depth_grid.ring_x_for_bucket(latest_bucket);
        self.params.set_heatmap_latest_x_ring(latest_x_ring);
    }

    /// Sync all uniforms that depend on time/x-scrolling
    /// - origin.x (bucket delta + phase)
    /// - heatmap_latest_rel_bucket
    /// - heatmap_latest_x_ring
    #[inline]
    pub fn sync_time_uniforms(
        &mut self,
        depth_grid: &depth_grid::GridRing,
        origin_x: f32,
        latest_time: u64,
        aggr_time: u64,
        scroll_ref_bucket: i64,
    ) {
        self.params.set_origin_x(origin_x);
        self.sync_heatmap_time_uniforms(depth_grid, latest_time, aggr_time, scroll_ref_bucket);
    }

    /// Pulls an upload from the grid and schedules it on the scene.
    /// Returns `true` if a full texture upload was scheduled.
    pub fn sync_heatmap_upload_from_grid(
        &mut self,
        grid: &mut depth_grid::GridRing,
        force_full: bool,
    ) -> bool {
        let Some(upload) = grid.upload_to_scene(force_full) else {
            return false;
        };

        let is_full = matches!(&upload, HeatmapUpload::Full { .. });
        self.schedule_heatmap_upload(upload);
        is_full
    }

    pub fn sync_palette(&mut self, palette: Option<&depth_grid::HeatmapPalette>) {
        if let Some(p) = palette {
            self.params.set_palette_rgb(p.bid_rgb, p.ask_rgb);
        }
    }

    /// 1) sync heatmap uniforms derived from grid + timing
    /// 2) pull/apply upload plan from grid (Full/Cols/None)
    ///
    /// Returns `true` if a full texture upload was scheduled.
    #[inline]
    pub fn sync_heatmap_from_grid(
        &mut self,
        grid: &mut depth_grid::GridRing,
        base_price: exchange::util::Price,
        step: exchange::util::PriceStep,
        qty_scale: f32,
        latest_time: u64,
        aggr_time: u64,
        scroll_ref_bucket: i64,
        force_full_upload: bool,
    ) -> bool {
        let qty_scale_inv = 1.0 / qty_scale;

        self.sync_heatmap_texture(
            grid,
            base_price,
            step,
            qty_scale_inv,
            latest_time,
            aggr_time,
            scroll_ref_bucket,
        );

        self.sync_heatmap_upload_from_grid(grid, force_full_upload)
    }

    pub fn zoom_column_world_keep_anchor(
        &mut self,
        factor: f32,
        anchor_world_x: f32,
        anchor_screen_x: f32,
        vw_px: f32,
    ) {
        self.camera.zoom_column_world_keep_anchor(
            factor,
            anchor_world_x,
            anchor_screen_x,
            vw_px,
            &mut self.cell,
        );

        self.sync_cell_world_uniform();
    }

    pub fn zoom_row_h_at(&mut self, factor: f32, cursor_y: f32, vh_px: f32) {
        self.camera
            .zoom_row_h_at(factor, cursor_y, vh_px, &mut self.cell);

        self.sync_cell_world_uniform();
    }

    pub fn zoom_column_world_at(&mut self, factor: f32, cursor_x: f32, vw_px: f32) {
        self.camera
            .zoom_column_world_at(factor, cursor_x, vw_px, &mut self.cell);

        self.sync_cell_world_uniform();
    }

    pub fn price_at_center(
        &self,
        base_price: exchange::util::Price,
        step: exchange::util::PriceStep,
    ) -> exchange::util::Price {
        self.camera
            .price_at_center(self.cell.height_world(), base_price, step)
    }
}

impl shader::Program<Message> for Scene {
    type State = Interaction;
    type Primitive = Primitive;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor_in_abs = cursor.position_over(bounds)?;

                *interaction = Interaction {
                    last_bounds: interaction.last_bounds,
                    kind: InteractionKind::Panning {
                        last_position: cursor_in_abs,
                    },
                };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                interaction.kind = InteractionKind::None;
                None
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let cursor_in_relative = cursor.position_in(bounds)?;

                let notches: f32 = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / PX_PER_NOTCH,
                }
                .clamp(-MAX_ABS_NOTCHES_PER_EVENT, MAX_ABS_NOTCHES_PER_EVENT);

                let factor = ZOOM_BASE_PER_NOTCH.powf(notches).clamp(0.01, 100.0);

                Some(shader::Action::publish(Message::ZoomAt {
                    factor,
                    cursor: cursor_in_relative,
                }))
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let InteractionKind::Panning { last_position } = &mut interaction.kind
                    && cursor.position_over(bounds).is_some()
                {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    Some(shader::Action::publish(Message::PanDeltaPx(delta_px)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        Primitive::new(
            self.id,
            self.rectangles.clone(),
            self.rectangles_gen,
            self.circles.clone(),
            self.circles_gen,
            self.draw_list.clone(),
            self.camera,
            self.params,
            self.heatmap_upload.clone(),
        )
    }

    fn mouse_interaction(
        &self,
        _interaction: &Interaction,
        _bounds: Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        // NOTE: this gets overridden by the overlay widget in heatmap/widget.rs
        iced_core::mouse::Interaction::default()
    }
}

#[derive(Debug)]
pub struct Primitive {
    id: u64,
    camera: camera::Camera,
    params: ParamsUniform,

    rectangles: Arc<[RectInstance]>,
    rectangles_gen: u64,

    circles: Arc<[CircleInstance]>,
    circles_gen: u64,

    draw_list: Arc<[DrawItem]>,

    heatmap_upload: Option<(u64, HeatmapUpload)>,
}

impl Primitive {
    fn new(
        id: u64,
        rectangles: Arc<[RectInstance]>,
        rectangles_gen: u64,
        circles: Arc<[CircleInstance]>,
        circles_gen: u64,
        draw_list: Arc<[DrawItem]>,
        camera: camera::Camera,
        params: ParamsUniform,
        heatmap_upload: Option<(u64, HeatmapUpload)>,
    ) -> Self {
        Self {
            id,
            rectangles,
            rectangles_gen,
            circles,
            circles_gen,
            draw_list,
            camera,
            params,
            heatmap_upload,
        }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        let cam_u = self.camera.to_uniform(bounds.width, bounds.height);

        pipeline.update_camera(self.id, device, queue, &cam_u);
        pipeline.update_params(self.id, device, queue, &self.params);

        pipeline.update_rect_instances(
            self.id,
            device,
            queue,
            self.rectangles.as_ref(),
            self.rectangles_gen,
        );
        pipeline.update_circle_instances(
            self.id,
            device,
            queue,
            self.circles.as_ref(),
            self.circles_gen,
        );

        if let Some((generation, upload)) = &self.heatmap_upload {
            match upload {
                HeatmapUpload::Full {
                    width,
                    height,
                    bid,
                    ask,
                } => {
                    pipeline.update_heatmap_textures_u32(
                        self.id,
                        device,
                        queue,
                        *width,
                        *height,
                        bid.as_ref(),
                        ask.as_ref(),
                        *generation,
                    );
                }
                HeatmapUpload::Cols {
                    width,
                    height,
                    cols,
                } => {
                    pipeline.update_heatmap_columns_u32(
                        self.id,
                        device,
                        queue,
                        *width,
                        *height,
                        cols.as_ref(),
                        *generation,
                    );
                }
            }
        }
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.begin_render_pass_ordered(
            self.id,
            encoder,
            target,
            *clip_bounds,
            self.rectangles.len() as u32,
            self.circles.len() as u32,
            self.draw_list.as_ref(),
        );
    }
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Pipeline {
        Self::new(device, queue, format)
    }
}

fn next_scene_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Default)]
pub struct Interaction {
    pub last_bounds: Rectangle,
    pub kind: InteractionKind,
}

#[derive(Debug, Default)]
pub enum InteractionKind {
    #[default]
    None,
    Panning {
        last_position: iced::Point,
    },
}
