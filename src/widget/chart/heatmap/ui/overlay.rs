use crate::style;
use crate::widget::chart::heatmap::Message;
use crate::widget::chart::heatmap::depth_grid::GridRing;
use crate::widget::chart::heatmap::scene::Scene;

use data::util::abbr_large_numbers;
use exchange::util::{Price, PriceStep};

use iced::Vector;
use iced::widget::canvas::Path;
use iced::{Alignment, Point, Rectangle, Renderer, Theme, mouse, widget::canvas};

const TOOLTIP_WIDTH: f32 = 198.0;
const TOOLTIP_HEIGHT: f32 = 66.0;
const TOOLTIP_PADDING: f32 = 12.0;

const OVERLAY_LABEL_PAD_PX: f32 = 6.0;
const OVERLAY_LABEL_TEXT_SIZE: f32 = 11.0;

const TOOLTIP_ROW_OFFSETS: [i64; 3] = [1, 0, -1];
const TOOLTIP_COL_OFFSETS: [i64; 4] = [-2, -1, 0, 1];

const HIGHLIGHT_CROSSHAIR_GAP_PX: f32 = 1.0;
const HIGHLIGHT_BORDER_WIDTH_PX: f32 = 1.0;
const HIGHLIGHT_BORDER_ALPHA: f32 = 0.95;

const PAUSE_ICON_BAR_WIDTH_FRAC: f32 = 0.008;
const PAUSE_ICON_BAR_HEIGHT_FRAC: f32 = 0.032;
const PAUSE_ICON_PADDING_FRAC: f32 = 0.02;

#[derive(Debug, Default)]
pub enum Interaction {
    #[default]
    Hovering,
    Panning {
        last_position: iced::Point,
    },
}

#[derive(Debug, Clone, Copy)]
struct TooltipLayout {
    rect: Rectangle,
    cell_w: f32,
    cell_h: f32,
}

impl TooltipLayout {
    fn from_cursor(bounds: Rectangle, local_x: f32, local_y: f32) -> Self {
        let should_draw_below = local_y < TOOLTIP_HEIGHT + TOOLTIP_PADDING;
        let should_draw_left = local_x > bounds.width - (TOOLTIP_WIDTH + TOOLTIP_PADDING);

        let x = if should_draw_left {
            local_x - TOOLTIP_WIDTH - TOOLTIP_PADDING
        } else {
            local_x + TOOLTIP_PADDING
        };

        let y = if should_draw_below {
            local_y + TOOLTIP_PADDING
        } else {
            local_y - TOOLTIP_HEIGHT - TOOLTIP_PADDING
        };

        let rect = Rectangle {
            x: x.max(0.0),
            y: y.max(0.0),
            width: TOOLTIP_WIDTH,
            height: TOOLTIP_HEIGHT,
        };

        let cell_w = TOOLTIP_WIDTH / (TOOLTIP_COL_OFFSETS.len() as f32);
        let cell_h = TOOLTIP_HEIGHT / (TOOLTIP_ROW_OFFSETS.len() as f32);

        Self {
            rect,
            cell_w,
            cell_h,
        }
    }

    fn cell_center(&self, row_idx: usize, col_idx: usize) -> Point {
        let x = self.rect.x + ((col_idx as f32) * self.cell_w) + self.cell_w / 2.0;
        let y = self.rect.y + ((row_idx as f32) * self.cell_h) + self.cell_h / 2.0;
        Point::new(x, y)
    }
}

pub struct OverlayCanvas<'a> {
    pub tooltip_cache: &'a iced::widget::canvas::Cache,
    pub scale_labels_cache: &'a iced::widget::canvas::Cache,

    pub scene: &'a Scene,
    pub depth_grid: &'a GridRing,
    pub base_price: Option<Price>,
    pub step: PriceStep,
    pub scroll_ref_bucket: i64,
    pub qty_scale: f32,

    pub profile_col_width_px: f32,
    pub strip_height_frac: f32,

    /// Max qty used to scale the volume strip bars (display units).
    pub volume_strip_max_qty: Option<f32>,
    /// Max qty used to scale the latest profile bars (display units).
    pub profile_max_qty: Option<f32>,
    /// Max qty used to scale the trade profile bars (display units, total=buy+sell).
    pub trade_profile_max_qty: Option<f32>,

    pub is_paused: bool,
}

impl<'a> canvas::Program<Message> for OverlayCanvas<'a> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(cursor_in_abs) = cursor.position_over(bounds) {
                    if self.is_paused && self.pause_icon_rect(bounds).contains(cursor_in_abs) {
                        return Some(canvas::Action::publish(Message::PauseBtnClicked));
                    }

                    *interaction = Interaction::Panning {
                        last_position: cursor_in_abs,
                    };
                }
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *interaction = Interaction::Hovering;
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        interaction: &Interaction,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if bounds.width <= 1.0 || bounds.height <= 1.0 {
            return vec![];
        }

        let scale_labels = self
            .scale_labels_cache
            .draw(renderer, bounds.size(), |frame| {
                let palette = theme.extended_palette();

                if self.is_paused
                // pause indicator (top-right corner)
                {
                    let bar_width = PAUSE_ICON_BAR_WIDTH_FRAC * bounds.height;
                    let bar_height = PAUSE_ICON_BAR_HEIGHT_FRAC * bounds.height;
                    let padding = bounds.area().sqrt() * PAUSE_ICON_PADDING_FRAC;

                    let region = Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: bounds.width,
                        height: bounds.height,
                    };

                    let total_icon_width = bar_width * 3.0;

                    let pause_bar = Rectangle {
                        x: (region.x + region.width) - total_icon_width - padding,
                        y: region.y + padding,
                        width: bar_width,
                        height: bar_height,
                    };

                    let hovered = cursor
                        .position_over(bounds)
                        .map(|p| self.pause_icon_rect(bounds).contains(p))
                        .unwrap_or(false);

                    let alpha = if hovered { 0.65 } else { 0.4 };

                    frame.fill_rectangle(
                        pause_bar.position(),
                        pause_bar.size(),
                        palette.background.base.text.scale_alpha(alpha),
                    );

                    frame.fill_rectangle(
                        pause_bar.position() + Vector::new(pause_bar.width * 2.0, 0.0),
                        pause_bar.size(),
                        palette.background.base.text.scale_alpha(alpha),
                    );
                }

                let strip_h_px = (bounds.height * self.strip_height_frac).clamp(0.0, bounds.height);
                let strip_top_y = (bounds.height - strip_h_px).clamp(0.0, bounds.height);

                // Volume strip denom label:
                // HUD-anchored to the overlay bounds (top-right of the whole overlay).
                if let Some(qty) = self.volume_strip_max_qty
                    && strip_h_px >= 16.0
                {
                    let tx = bounds.width - OVERLAY_LABEL_PAD_PX;
                    let ty = strip_top_y + OVERLAY_LABEL_PAD_PX;

                    frame.fill_text(canvas::Text {
                        content: abbr_large_numbers(qty),
                        position: Point::new(tx, ty),
                        size: iced::Pixels(OVERLAY_LABEL_TEXT_SIZE - 1.),
                        color: palette.background.base.text.scale_alpha(0.85),
                        font: style::AZERET_MONO,
                        align_x: Alignment::End.into(),
                        align_y: Alignment::Start.into(),
                        ..canvas::Text::default()
                    });
                }

                // Profile denom label:
                // anchored to the *world-space end* of the profile scale (x = profile_max_w_world).
                if let Some(qty) = self.profile_max_qty {
                    let vw_px = bounds.width;
                    let vh_px = bounds.height;

                    let cam_scale = self.scene.camera.scale();

                    let visible_space_right_of_zero_world =
                        (self.scene.camera.right_edge(vw_px) - 0.0).max(0.0);
                    let desired_profile_w_world = (self.profile_col_width_px.max(0.0)) / cam_scale;
                    let profile_max_w_world =
                        desired_profile_w_world.min(visible_space_right_of_zero_world);

                    if profile_max_w_world > 0.0 {
                        // Profile ends at world x = profile_max_w_world (since it starts at x=0).
                        let profile_end_world_x = profile_max_w_world;

                        let [profile_end_px_x, _] = self.scene.camera.world_to_screen(
                            profile_end_world_x,
                            0.0,
                            vw_px,
                            vh_px,
                        );

                        // Only draw if visible.
                        if (0.0..=vw_px).contains(&profile_end_px_x) {
                            let tx = (profile_end_px_x - OVERLAY_LABEL_PAD_PX).clamp(0.0, vw_px);
                            let ty = OVERLAY_LABEL_PAD_PX;

                            frame.fill_text(canvas::Text {
                                content: abbr_large_numbers(qty),
                                position: Point::new(tx, ty),
                                size: iced::Pixels(OVERLAY_LABEL_TEXT_SIZE - 1.),
                                color: palette.background.base.text.scale_alpha(0.85),
                                font: style::AZERET_MONO,
                                align_x: Alignment::End.into(),
                                align_y: Alignment::Start.into(),
                                ..canvas::Text::default()
                            });
                        }
                    }
                }

                // Trade-profile denom label:
                // anchored to the *world-space end* of the trade-profile zone.
                if let Some(qty) = self.trade_profile_max_qty {
                    let vw_px = bounds.width;
                    let vh_px = bounds.height;

                    let left_edge_world = self.scene.params.fade_start();
                    let trade_profile_max_w_world = self.scene.params.fade_width();

                    if left_edge_world.is_finite()
                        && trade_profile_max_w_world.is_finite()
                        && trade_profile_max_w_world > 0.0
                    {
                        let trade_profile_end_world_x = left_edge_world + trade_profile_max_w_world;

                        let y_world = self.scene.camera.offset[1];

                        let [end_px_x, _] = self.scene.camera.world_to_screen(
                            trade_profile_end_world_x,
                            y_world,
                            vw_px,
                            vh_px,
                        );

                        if end_px_x.is_finite() && (0.0..=vw_px).contains(&end_px_x) {
                            let tx = (end_px_x - OVERLAY_LABEL_PAD_PX).clamp(0.0, vw_px);
                            let ty = OVERLAY_LABEL_PAD_PX;

                            frame.fill_text(canvas::Text {
                                content: abbr_large_numbers(qty),
                                position: Point::new(tx, ty),
                                size: iced::Pixels(OVERLAY_LABEL_TEXT_SIZE - 1.),
                                color: palette.background.base.text.scale_alpha(0.85),
                                font: style::AZERET_MONO,
                                align_x: Alignment::End.into(),
                                align_y: Alignment::Start.into(),
                                ..canvas::Text::default()
                            });
                        }
                    }
                }
            });

        let Some(pos) = cursor.position_over(bounds) else {
            return vec![scale_labels];
        };

        let tooltip = self.tooltip_cache.draw(renderer, bounds.size(), |frame| {
            let cell_width = self.scene.cell.width_world();
            let cell_height = self.scene.cell.height_world();

            let tex_w = self.depth_grid.tex_w() as i64;
            let tex_h = self.depth_grid.tex_h() as i64;

            if tex_w <= 0 || tex_h <= 0 {
                return;
            }

            let origin0 = self.scene.params.origin_x();
            if !origin0.is_finite() || cell_width <= 0.0 || cell_height <= 0.0 {
                return;
            }

            let local_x = pos.x - bounds.x;
            let local_y = pos.y - bounds.y;

            let [world_x, world_y] =
                self.scene
                    .camera
                    .screen_to_world(local_x, local_y, bounds.width, bounds.height);

            let x_bin_rel_f = (world_x / cell_width) + origin0;
            if !x_bin_rel_f.is_finite() {
                return;
            }

            let x_bin_rel = x_bin_rel_f.round();
            let snapped_world_x = (x_bin_rel - origin0) * cell_width;

            let steps_per_y_bin = self.scene.params.steps_per_y_bin();
            let steps_at_y = ((-world_y) / cell_height).floor() as i64;
            let base_rel_y_bin = steps_at_y.div_euclid(steps_per_y_bin.max(1));
            let snapped_world_y =
                -((base_rel_y_bin as f32 + 0.5) * (steps_per_y_bin as f32) * cell_height);

            let [snap_px_x, snap_px_y] = self.scene.camera.world_to_screen(
                snapped_world_x,
                snapped_world_y,
                bounds.width,
                bounds.height,
            );

            let x = (snap_px_x.round() + 0.5).clamp(0.0, bounds.width);
            let y = (snap_px_y.round() + 0.5).clamp(0.0, bounds.height);

            if let Interaction::Panning { .. } = interaction {
                self.draw_full_crosshair(frame, theme, bounds, x, y);
                return;
            }

            let base_bucket_abs = self
                .scroll_ref_bucket
                .saturating_add(x_bin_rel_f.round() as i64);

            let y_start_bin = self.scene.params.heatmap_start_bin();

            let any_nonzero = self.tooltip_neighborhood_has_data(
                tex_w,
                tex_h,
                base_rel_y_bin,
                base_bucket_abs,
                y_start_bin,
            );

            if !any_nonzero {
                self.draw_full_crosshair(frame, theme, bounds, x, y);
                return;
            }

            if let Some(neighborhood_rect) = self.tooltip_neighborhood_rect_px(
                bounds,
                origin0,
                cell_width,
                cell_height,
                steps_per_y_bin,
                base_rel_y_bin,
                base_bucket_abs,
            ) {
                self.draw_crosshair_around_rect(frame, theme, bounds, x, y, neighborhood_rect);
                self.draw_neighborhood_outline(frame, theme, neighborhood_rect);
            } else {
                self.draw_full_crosshair(frame, theme, bounds, x, y);
            }

            let palette = theme.extended_palette();
            let bg = palette.background.weakest.color.scale_alpha(0.90);
            let layout = TooltipLayout::from_cursor(bounds, local_x, local_y);

            frame.fill_rectangle(layout.rect.position(), layout.rect.size(), bg);

            for (row_idx, &dy) in TOOLTIP_ROW_OFFSETS.iter().enumerate() {
                let rel_y_bin = base_rel_y_bin.saturating_add(dy);
                let y_tex = rel_y_bin.saturating_sub(y_start_bin);
                if y_tex < 0 || y_tex >= tex_h {
                    continue;
                }

                for (col_idx, &dx) in TOOLTIP_COL_OFFSETS.iter().enumerate() {
                    let bucket = base_bucket_abs.saturating_add(dx);
                    let x_ring = self.depth_grid.ring_x_for_bucket(bucket) as i64;
                    if x_ring < 0 || x_ring >= tex_w {
                        continue;
                    }

                    let idx = (y_tex as usize) * (tex_w as usize) + (x_ring as usize);
                    if idx >= self.depth_grid.bid.len() || idx >= self.depth_grid.ask.len() {
                        continue;
                    }

                    let bid_u32 = self.depth_grid.bid[idx];
                    let ask_u32 = self.depth_grid.ask[idx];

                    if bid_u32 == 0 && ask_u32 == 0 {
                        continue;
                    }

                    let (is_bid, qty_u32) = if bid_u32 >= ask_u32 {
                        (true, bid_u32)
                    } else {
                        (false, ask_u32)
                    };

                    let qty: f32 = (qty_u32 as f32) / self.qty_scale;

                    let color = if is_bid {
                        palette.success.strong.color
                    } else {
                        palette.danger.strong.color
                    };

                    frame.fill_text(canvas::Text {
                        content: abbr_large_numbers(qty),
                        position: layout.cell_center(row_idx, col_idx),
                        size: iced::Pixels(11.0),
                        color: color.scale_alpha(0.95),
                        align_x: Alignment::Center.into(),
                        align_y: Alignment::Center.into(),
                        font: crate::style::AZERET_MONO,
                        ..canvas::Text::default()
                    });
                }
            }
        });

        vec![tooltip, scale_labels]
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(pos) = cursor.position_over(bounds) {
            if self.is_paused && self.pause_icon_rect(bounds).contains(pos) {
                return mouse::Interaction::Pointer;
            }

            if let Interaction::Panning { .. } = interaction {
                mouse::Interaction::Grabbing
            } else {
                mouse::Interaction::Crosshair
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a> OverlayCanvas<'a> {
    /// Compute the pause icon hit rectangle in local (canvas) coordinates.
    fn pause_icon_rect(&self, bounds: Rectangle) -> Rectangle {
        let bar_width = 0.008 * bounds.height;
        let bar_height = 0.032 * bounds.height;
        let padding = bounds.area().sqrt() * 0.02;
        let total_icon_width = bar_width * 3.0;

        Rectangle {
            x: (bounds.x + bounds.width) - total_icon_width - padding,
            y: bounds.y + padding,
            width: total_icon_width,
            height: bar_height,
        }
    }

    fn draw_full_crosshair(
        &self,
        frame: &mut canvas::Frame,
        theme: &Theme,
        bounds: Rectangle,
        x: f32,
        y: f32,
    ) {
        frame.stroke(
            &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
            style::dashed_line(theme),
        );
        frame.stroke(
            &Path::line(Point::new(0.0, y), Point::new(bounds.width, y)),
            style::dashed_line(theme),
        );
    }

    fn draw_crosshair_around_rect(
        &self,
        frame: &mut canvas::Frame,
        theme: &Theme,
        bounds: Rectangle,
        x: f32,
        y: f32,
        rect: Rectangle,
    ) {
        let cut_left = (rect.x - HIGHLIGHT_CROSSHAIR_GAP_PX).clamp(0.0, bounds.width);
        let cut_right = (rect.x + rect.width + HIGHLIGHT_CROSSHAIR_GAP_PX).clamp(0.0, bounds.width);
        let cut_top = (rect.y - HIGHLIGHT_CROSSHAIR_GAP_PX).clamp(0.0, bounds.height);
        let cut_bottom =
            (rect.y + rect.height + HIGHLIGHT_CROSSHAIR_GAP_PX).clamp(0.0, bounds.height);

        if (cut_left..=cut_right).contains(&x) {
            if cut_top > 0.0 {
                frame.stroke(
                    &Path::line(Point::new(x, 0.0), Point::new(x, cut_top)),
                    style::dashed_line(theme),
                );
            }
            if cut_bottom < bounds.height {
                frame.stroke(
                    &Path::line(Point::new(x, cut_bottom), Point::new(x, bounds.height)),
                    style::dashed_line(theme),
                );
            }
        } else {
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
                style::dashed_line(theme),
            );
        }

        if (cut_top..=cut_bottom).contains(&y) {
            if cut_left > 0.0 {
                frame.stroke(
                    &Path::line(Point::new(0.0, y), Point::new(cut_left, y)),
                    style::dashed_line(theme),
                );
            }
            if cut_right < bounds.width {
                frame.stroke(
                    &Path::line(Point::new(cut_right, y), Point::new(bounds.width, y)),
                    style::dashed_line(theme),
                );
            }
        } else {
            frame.stroke(
                &Path::line(Point::new(0.0, y), Point::new(bounds.width, y)),
                style::dashed_line(theme),
            );
        }
    }

    fn draw_neighborhood_outline(&self, frame: &mut canvas::Frame, theme: &Theme, rect: Rectangle) {
        let mut rect_w = rect.width.max(0.0);
        let mut rect_h = rect.height.max(0.0);

        if rect_w < 1.0 || rect_h < 1.0 {
            return;
        }

        let palette = theme.extended_palette();

        let stroke = canvas::Stroke {
            style: canvas::Style::Solid(
                palette
                    .secondary
                    .strong
                    .color
                    .scale_alpha(HIGHLIGHT_BORDER_ALPHA),
            ),
            width: HIGHLIGHT_BORDER_WIDTH_PX,
            ..canvas::Stroke::default()
        };

        let x = rect.x.round() + 0.5;
        let y = rect.y.round() + 0.5;
        rect_w = (rect_w.round() - 1.0).max(0.0);
        rect_h = (rect_h.round() - 1.0).max(0.0);

        frame.stroke(
            &Path::rectangle(Point::new(x, y), iced::Size::new(rect_w, rect_h)),
            stroke,
        );
    }

    fn tooltip_neighborhood_has_data(
        &self,
        tex_w: i64,
        tex_h: i64,
        base_rel_y_bin: i64,
        base_bucket_abs: i64,
        y_start_bin: i64,
    ) -> bool {
        for &dy in &TOOLTIP_ROW_OFFSETS {
            let rel_y_bin = base_rel_y_bin.saturating_add(dy);
            let y_tex = rel_y_bin.saturating_sub(y_start_bin);
            if y_tex < 0 || y_tex >= tex_h {
                continue;
            }

            for &dx in &TOOLTIP_COL_OFFSETS {
                let bucket = base_bucket_abs.saturating_add(dx);
                let x_ring = self.depth_grid.ring_x_for_bucket(bucket) as i64;
                if x_ring < 0 || x_ring >= tex_w {
                    continue;
                }

                let idx = (y_tex as usize) * (tex_w as usize) + (x_ring as usize);
                if idx >= self.depth_grid.bid.len() || idx >= self.depth_grid.ask.len() {
                    continue;
                }

                if self.depth_grid.bid[idx] != 0 || self.depth_grid.ask[idx] != 0 {
                    return true;
                }
            }
        }

        false
    }

    fn tooltip_neighborhood_rect_px(
        &self,
        bounds: Rectangle,
        origin0: f32,
        cell_width: f32,
        cell_height: f32,
        steps_per_y_bin: i64,
        base_rel_y_bin: i64,
        base_bucket_abs: i64,
    ) -> Option<Rectangle> {
        let min_col = TOOLTIP_COL_OFFSETS.iter().copied().min()?;
        let max_col = TOOLTIP_COL_OFFSETS.iter().copied().max()?;
        let min_row = TOOLTIP_ROW_OFFSETS.iter().copied().min()?;
        let max_row = TOOLTIP_ROW_OFFSETS.iter().copied().max()?;

        let left_bucket = base_bucket_abs.saturating_add(min_col);
        let right_bucket_excl = base_bucket_abs.saturating_add(max_col.saturating_add(1));

        let left_world_x = (((left_bucket - self.scroll_ref_bucket) as f32) - origin0) * cell_width;
        let right_world_x =
            (((right_bucket_excl - self.scroll_ref_bucket) as f32) - origin0) * cell_width;

        let min_rel_y_bin = base_rel_y_bin.saturating_add(min_row);
        let max_rel_y_bin = base_rel_y_bin.saturating_add(max_row);
        let y_bin_h_world = (steps_per_y_bin.max(1) as f32) * cell_height;

        let top_world_y = -((max_rel_y_bin as f32 + 1.0) * y_bin_h_world);
        let bottom_world_y = -((min_rel_y_bin as f32) * y_bin_h_world);

        let [x0_px, y0_px] = self.scene.camera.world_to_screen(
            left_world_x,
            top_world_y,
            bounds.width,
            bounds.height,
        );
        let [x1_px, y1_px] = self.scene.camera.world_to_screen(
            right_world_x,
            bottom_world_y,
            bounds.width,
            bounds.height,
        );

        if !x0_px.is_finite() || !y0_px.is_finite() || !x1_px.is_finite() || !y1_px.is_finite() {
            return None;
        }

        let left_px = x0_px.min(x1_px).clamp(0.0, bounds.width);
        let right_px = x0_px.max(x1_px).clamp(0.0, bounds.width);
        let top_px = y0_px.min(y1_px).clamp(0.0, bounds.height);
        let bottom_px = y0_px.max(y1_px).clamp(0.0, bounds.height);

        let rect_w = (right_px - left_px).max(0.0);
        let rect_h = (bottom_px - top_px).max(0.0);

        if rect_w < 1.0 || rect_h < 1.0 {
            return None;
        }

        Some(Rectangle {
            x: left_px,
            y: top_px,
            width: rect_w,
            height: rect_h,
        })
    }
}
