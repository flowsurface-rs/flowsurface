use super::{AxisInteraction, Message};
use crate::widget::chart::heatmap::{
    scene::camera::Camera,
    ui::{AXIS_TEXT_SIZE, AxisZoomAnchor},
};
use data::config::timezone::TimeLabelKind;

use iced::{Rectangle, Renderer, Theme, widget::canvas};
use iced_core::mouse;

const DRAG_ZOOM_SENS: f32 = 0.005;
const PHASE_MAX: f64 = 0.999_999;
const COL_EPS: f64 = 1e-18;
const BUCKET_EPS: f64 = 1e-9;
const APPROX_CHAR_WIDTH_RATIO: f32 = 0.62;
const DRAW_MARGIN_EXTRA_PX: f32 = 6.0;
const TARGET_LABEL_SPACING_PX: f32 = 110.0;
const CURSOR_LABEL_PADDING_X: f32 = 10.0;
const CURSOR_LABEL_PADDING_Y: f32 = 6.0;
const TICK_CURSOR_GAP_PX: f32 = 2.0;

pub struct AxisXLabelCanvas<'a> {
    pub cache: &'a iced::widget::canvas::Cache,
    pub camera: &'a Camera,
    pub timezone: data::UserTimezone,
    pub plot_bounds: Option<Rectangle>,
    pub is_paused: bool,
    pub latest_bucket: i64,
    pub aggr_time: u64,
    pub column_world: f32,
    pub x_phase_bucket: f32,
    pub is_x0_visible: Option<bool>,
}

impl<'a> canvas::Program<Message> for AxisXLabelCanvas<'a> {
    type State = super::AxisState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;

                // Double-click detection uses global cursor position + previous click.
                if let Some(global_pos) = cursor.position() {
                    let new_click =
                        mouse::Click::new(global_pos, mouse::Button::Left, state.previous_click);
                    let is_double = new_click.kind() == iced_core::mouse::click::Kind::Double;

                    state.previous_click = Some(new_click);

                    if is_double {
                        state.interaction = AxisInteraction::None;
                        return Some(canvas::Action::publish(Message::AxisXDoubleClicked));
                    }
                } else {
                    state.previous_click = None;
                }

                let use_world_anchor = self.is_x0_visible == Some(true);

                let zoom_anchor = if use_world_anchor {
                    let vw = self.plot_bounds.map(|r| r.width).unwrap_or(bounds.width);
                    let x0_screen = self.camera.world_to_screen_x(0.0, vw);

                    Some(AxisZoomAnchor::World {
                        world: 0.0,
                        screen: x0_screen,
                    })
                } else {
                    Some(AxisZoomAnchor::Cursor { screen: p.x })
                };

                state.interaction = AxisInteraction::Panning {
                    last_position: p,
                    zoom_anchor,
                };

                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.interaction = AxisInteraction::None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let AxisInteraction::Panning {
                    last_position,
                    zoom_anchor,
                } = &mut state.interaction
                {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    let scroll_amount = -delta_px.x * DRAG_ZOOM_SENS;
                    let factor = (1.0 + scroll_amount).clamp(0.01, 100.0);

                    match *zoom_anchor {
                        Some(AxisZoomAnchor::World { screen, .. }) => {
                            let vw = self.plot_bounds.map(|r| r.width).unwrap_or(bounds.width);

                            Some(canvas::Action::publish(Message::DragZoomAxisXKeepAnchor {
                                factor,
                                anchor_screen_x: screen,
                                viewport_w: vw,
                            }))
                        }
                        Some(AxisZoomAnchor::Cursor { screen }) => {
                            Some(canvas::Action::publish(Message::ScrolledAxisX {
                                factor,
                                cursor_x: screen,
                                viewport_w: bounds.width,
                            }))
                        }
                        None => None,
                    }
                } else {
                    None
                }
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let cursor_rel_pos = cursor.position_in(bounds)?;
                let scroll_amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.1,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.01,
                };

                let factor = (1.0 + scroll_amount).clamp(0.01, 100.0);

                Some(canvas::Action::publish(Message::ScrolledAxisX {
                    factor,
                    cursor_x: cursor_rel_pos.x,
                    viewport_w: bounds.width,
                }))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if self.aggr_time == 0 || !self.column_world.is_finite() || self.column_world <= 0.0 {
            return vec![];
        }
        let palette = theme.extended_palette();

        let labels = self.cache.draw(renderer, bounds.size(), |frame| {
            let vw = bounds.width;
            let vh = bounds.height;

            let vh_f = vh as f64;
            let cam_sx_f = self.camera.scale() as f64;
            let col_f = self.column_world as f64;
            let aggr_time_ms = i64::try_from(self.aggr_time).unwrap_or(i64::MAX);

            let phase = {
                let mut phase = self.x_phase_bucket as f64;
                if !phase.is_finite() {
                    phase = 0.0;
                }

                phase.clamp(0.0, PHASE_MAX)
            };

            let (x_world_left_f32, x_world_right_f32) = self.camera.x_world_bounds(vw);
            let x_world_left = x_world_left_f32 as f64;
            let x_world_right = x_world_right_f32 as f64;

            let inv_col = 1.0f64 / col_f.max(COL_EPS);

            let latest_bucket: i64 = self.latest_bucket;
            let (b_min0, b_max0) =
                bucket_bounds(x_world_left, x_world_right, inv_col, phase, latest_bucket);

            let fmt = {
                let visible_buckets0 = (b_max0 - b_min0).max(0);

                let visible_span = visible_buckets0.saturating_mul(aggr_time_ms);
                let visible_span_ms0 = visible_span.clamp(0, i64::MAX);

                pick_time_format(visible_span_ms0)
            };

            let half_label_w_px = 0.5 * approx_text_width_px(max_label_chars(fmt));
            let draw_margin_px = half_label_w_px + DRAW_MARGIN_EXTRA_PX;
            let draw_margin_world = (draw_margin_px as f64) / cam_sx_f.max(COL_EPS);

            let x_world_left_p = x_world_left - draw_margin_world;
            let x_world_right_p = x_world_right + draw_margin_world;

            let (b_min, b_max) = bucket_bounds(
                x_world_left_p,
                x_world_right_p,
                inv_col,
                phase,
                latest_bucket,
            );

            let text_color = palette.background.base.text;

            let world_to_screen_x =
                |world_x: f64| -> f32 { self.camera.world_to_screen_x(world_x as f32, vw) };

            let y = (0.5 * vh_f) as f32;

            let suppress_cursor_label =
                super::paused_control_hovered(self.is_paused, self.plot_bounds, cursor);

            let cursor_label = if suppress_cursor_label {
                None
            } else {
                self.plot_bounds
                    .and_then(|pb| cursor.position_in(pb))
                    .and_then(|p| {
                        let world_x_cursor = self.camera.screen_to_world_x(p.x, vw) as f64;

                        let u_at_cursor = ((world_x_cursor / col_f) + phase).round() as i64;
                        let b_at_cursor = latest_bucket.saturating_add(u_at_cursor);
                        let world_x_for_bucket = ((u_at_cursor as f64) - phase) * col_f;
                        let x_px = world_to_screen_x(world_x_for_bucket);
                        let t_ms = b_at_cursor.saturating_mul(aggr_time_ms);

                        if let Some(label) = self
                            .timezone
                            .format_with_kind(t_ms, TimeLabelKind::Crosshair { show_millis: true })
                        {
                            let (width, height) = {
                                let text_w = approx_text_width_px(label.chars().count());
                                let label_w = text_w + 2.0 * CURSOR_LABEL_PADDING_X;
                                let label_h = AXIS_TEXT_SIZE + 2.0 * CURSOR_LABEL_PADDING_Y;

                                (label_w, label_h)
                            };

                            Some(CursorLabel {
                                x_px,
                                text: label,
                                width,
                                height,
                            })
                        } else {
                            None
                        }
                    })
            };

            let every = {
                let px_per_bucket = (col_f * cam_sx_f).max(1e-9) as f32;
                let rough = (TARGET_LABEL_SPACING_PX / px_per_bucket).ceil().max(1.) as i64;

                let mut pow10 = 1i64;
                while pow10.saturating_mul(10) <= rough {
                    pow10 *= 10;
                }
                let m = (rough + pow10 - 1) / pow10;
                let mult = crate::widget::chart::nice_step_multiplier_125(m as f32) as i64;
                mult * pow10
            };

            let mut b = (b_min.div_euclid(every)) * every;
            if b < b_min {
                b += every;
            }

            while b <= b_max {
                let rel = b - latest_bucket;

                let world_x = ((rel as f64) - phase) * col_f;
                let x_px = world_to_screen_x(world_x);

                if x_px >= -draw_margin_px && x_px <= (vw + draw_margin_px) {
                    let t_ms = b.saturating_mul(aggr_time_ms);

                    if let Some(label) = self
                        .timezone
                        .format_with_kind(t_ms, TimeLabelKind::Custom(fmt))
                    {
                        let tick_label_w = approx_text_width_px(label.chars().count());
                        let tick_half = 0.5 * tick_label_w;

                        if let Some(cursor_label) = cursor_label.as_ref() {
                            let cursor_half = 0.5 * cursor_label.width;
                            if (x_px + tick_half + TICK_CURSOR_GAP_PX)
                                >= (cursor_label.x_px - cursor_half)
                                && (x_px - tick_half - TICK_CURSOR_GAP_PX)
                                    <= (cursor_label.x_px + cursor_half)
                            {
                                b = b.saturating_add(every);
                                if every <= 0 {
                                    break;
                                }
                                continue;
                            }
                        }

                        frame.fill_text(canvas::Text {
                            content: label,
                            position: iced::Point::new(x_px, y),
                            color: text_color,
                            font: crate::style::AZERET_MONO,
                            size: AXIS_TEXT_SIZE.into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            ..Default::default()
                        });
                    }
                }

                b = b.saturating_add(every);
                if every <= 0 {
                    break;
                }
            }

            if let Some(cursor_label) = cursor_label
                && cursor_label.x_px >= -draw_margin_px
                && cursor_label.x_px <= (vw + draw_margin_px)
                && !cursor_label.text.is_empty()
            {
                let mut bg = palette.secondary.base.color;
                bg = iced::Color { a: 1.0, ..bg };
                frame.fill_rectangle(
                    iced::Point::new(
                        cursor_label.x_px - 0.5 * cursor_label.width,
                        y - 0.5 * cursor_label.height,
                    ),
                    iced::Size {
                        width: cursor_label.width,
                        height: cursor_label.height,
                    },
                    bg,
                );
                frame.fill_text(canvas::Text {
                    content: cursor_label.text,
                    position: iced::Point::new(cursor_label.x_px, y),
                    color: palette.secondary.base.text,
                    size: AXIS_TEXT_SIZE.into(),
                    font: crate::style::AZERET_MONO,
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }
        });

        vec![labels]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        if cursor.position_over(bounds).is_some() {
            match state.interaction {
                AxisInteraction::Panning { .. } => iced_core::mouse::Interaction::Grabbing,
                _ => iced_core::mouse::Interaction::ResizingHorizontally,
            }
        } else {
            iced_core::mouse::Interaction::default()
        }
    }
}

fn pick_time_format(visible_span_ms: i64) -> &'static str {
    // Pick shorter/longer formats based on current visible time span.
    if visible_span_ms <= 10_000 {
        "%H:%M:%S%.3f" // up to ~10s: show milliseconds
    } else if visible_span_ms <= 10 * 60_000 {
        "%H:%M:%S" // up to ~10m: seconds
    } else if visible_span_ms <= 24 * 3_600_000 {
        "%H:%M" // up to ~1d: minutes
    } else {
        "%m-%d %H:%M" // zoomed way out: include date
    }
}

struct CursorLabel {
    x_px: f32,
    text: String,
    width: f32,
    height: f32,
}

fn max_label_chars(fmt: &str) -> usize {
    match fmt {
        "%H:%M:%S%.3f" => 12,
        "%H:%M:%S" => 8,
        "%H:%M" => 5,
        "%m-%d %H:%M" => 11,
        _ => 16,
    }
}

fn approx_text_width_px(chars: usize) -> f32 {
    chars as f32 * (AXIS_TEXT_SIZE * APPROX_CHAR_WIDTH_RATIO)
}

fn bucket_bounds(
    x_world_left: f64,
    x_world_right: f64,
    inv_col: f64,
    phase: f64,
    latest_bucket: i64,
) -> (i64, i64) {
    let u_min = ((x_world_left * inv_col) + phase + BUCKET_EPS).floor() as i64;
    let u_max = ((x_world_right * inv_col) + phase - BUCKET_EPS).ceil() as i64;

    (
        latest_bucket.saturating_add(u_min),
        latest_bucket.saturating_add(u_max),
    )
}
