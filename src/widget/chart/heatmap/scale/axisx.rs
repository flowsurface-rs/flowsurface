use super::{AxisInteraction, Message};
use chrono::TimeZone;
use iced::{Rectangle, Renderer, Theme, mouse, widget::canvas};

fn unix_ms_to_local_string(ts_ms: i128, fmt: &str) -> String {
    // Safe conversion: clamp to i64 range for chrono.
    let ts_ms_i64 = ts_ms.clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    let utc = match chrono::Utc.timestamp_millis_opt(ts_ms_i64).single() {
        Some(dt) => dt,
        None => return "".to_string(),
    };

    utc.with_timezone(&chrono::Local).format(fmt).to_string()
}

fn pick_time_format(visible_span_ms: i128) -> &'static str {
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

pub struct AxisXLabelCanvas<'a> {
    pub cache: &'a iced::widget::canvas::Cache,
    pub plot_bounds: Option<Rectangle>,
    pub latest_bucket: i64,
    pub aggr_time: Option<u64>,
    pub column_world: f32,
    pub cam_offset_x: f32,
    pub cam_sx: f32,
    pub cam_right_pad_frac: f32,
    pub x_phase_bucket: f32,
}

impl<'a> canvas::Program<Message> for AxisXLabelCanvas<'a> {
    type State = AxisInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;
                *state = AxisInteraction::Panning { last_position: p };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *state = AxisInteraction::None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let AxisInteraction::Panning { last_position } = state
                    && cursor.position_over(bounds).is_some()
                {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    // Only horizontal pan from X axis.
                    Some(canvas::Action::publish(Message::PanDeltaPx(iced::Vector {
                        x: delta_px.x,
                        y: 0.0,
                    })))
                } else {
                    None
                }
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Zoom column width around cursor X.
                let p = cursor.position_in(bounds)?;
                let scroll_amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.1,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.01,
                };

                let factor = (1.0 + scroll_amount).clamp(0.01, 100.0);

                Some(canvas::Action::publish(Message::ZoomColumnWorldAt {
                    factor,
                    cursor_x: p.x,
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
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let Some(aggr_time) = self.aggr_time else {
            return vec![];
        };

        if aggr_time == 0
            || !self.column_world.is_finite()
            || self.column_world <= 0.0
            || !self.cam_sx.is_finite()
            || self.cam_sx <= 0.0
        {
            return vec![];
        }

        let labels = self.cache.draw(renderer, bounds.size(), |frame| {
            let vw = bounds.width;
            let vh = bounds.height;

            let vw_f = vw as f64;
            let vh_f = vh as f64;
            let cam_sx_f = self.cam_sx as f64;
            let col_f = self.column_world as f64;
            let cam_offset_f = self.cam_offset_x as f64;
            let right_pad_frac_f = self.cam_right_pad_frac as f64;

            let mut phase = self.x_phase_bucket as f64;
            if !phase.is_finite() {
                phase = 0.0;
            }
            phase = phase.clamp(0.0, 0.999_999);

            // Camera math (matches camera.rs)
            let pad_world = (vw_f * right_pad_frac_f) / cam_sx_f;
            let right_edge_world = cam_offset_f + pad_world;

            let x_world_left = right_edge_world - (vw_f / cam_sx_f);
            let x_world_right = right_edge_world;

            let inv_col = 1.0f64 / col_f.max(1e-18);
            let eps = 1e-9f64;

            let u_min0 = ((x_world_left * inv_col) + phase + eps).floor() as i64;
            let u_max0 = ((x_world_right * inv_col) + phase - eps).ceil() as i64;

            let latest_bucket: i64 = self.latest_bucket;
            let b_min0: i64 = latest_bucket.saturating_add(u_min0);
            let b_max0: i64 = latest_bucket.saturating_add(u_max0);

            let visible_buckets0 = (b_max0 as i128 - b_min0 as i128).max(0);
            let visible_span_ms0 = visible_buckets0 * (aggr_time as i128);
            let fmt = pick_time_format(visible_span_ms0);

            // Approx label width in px for mono font; used to pad bucket-range so labels don't "pop"
            let font_size = 12.0f32;
            let max_label_chars: usize = match fmt {
                "%H:%M:%S%.3f" => 12, // "23:59:59.999"
                "%H:%M:%S" => 8,      // "23:59:59"
                "%H:%M" => 5,         // "23:59"
                "%m-%d %H:%M" => 11,  // "12-31 23:59"
                _ => 16,
            };
            let approx_char_w_px = font_size * 0.62;
            let half_label_w_px = (max_label_chars as f32) * approx_char_w_px * 0.5;
            let draw_margin_px = half_label_w_px + 6.0;
            let draw_margin_world = (draw_margin_px as f64) / cam_sx_f.max(1e-18);

            let x_world_left_p = x_world_left - draw_margin_world;
            let x_world_right_p = x_world_right + draw_margin_world;

            let u_min = ((x_world_left_p * inv_col) + phase + eps).floor() as i64;
            let u_max = ((x_world_right_p * inv_col) + phase - eps).ceil() as i64;

            let b_min: i64 = latest_bucket.saturating_add(u_min);
            let b_max: i64 = latest_bucket.saturating_add(u_max);

            let px_per_bucket = (col_f * cam_sx_f).max(1e-9) as f32;
            let target_label_px = 110.0f32;
            let rough_every = (target_label_px / px_per_bucket).ceil() as i64;
            let every = super::nice_step_i64(rough_every.max(1));

            let text_color = theme.palette().text;
            let palette = theme.extended_palette();
            let center_x = right_edge_world - (vw_f * 0.5) / cam_sx_f;
            let world_to_screen_x =
                |world_x: f64| -> f32 { ((world_x - center_x) * cam_sx_f + vw_f * 0.5) as f32 };

            let y = (0.5 * vh_f) as f32;

            let cursor_label_padding = 6.0f32;
            let cursor_label = self
                .plot_bounds
                .and_then(|pb| _cursor.position_in(pb))
                .map(|p| {
                    let world_x_cursor = center_x + ((p.x as f64) - vw_f * 0.5) / cam_sx_f;

                    let u_at_cursor = ((world_x_cursor / col_f) + phase).round() as i64;
                    let b_at_cursor = latest_bucket.saturating_add(u_at_cursor);
                    let world_x_for_bucket = ((u_at_cursor as f64) - phase) * col_f;
                    let x_px = world_to_screen_x(world_x_for_bucket);
                    let t_ms = (b_at_cursor as i128) * (aggr_time as i128);
                    let label = unix_ms_to_local_string(t_ms, "%H:%M:%S%.3f");

                    let label_len = label.chars().count() as f32;
                    let label_w = label_len * (font_size * 0.62) + 2.0 * cursor_label_padding;
                    let label_h = font_size + 2.0 * cursor_label_padding;

                    (x_px, label, label_w, label_h)
                });

            let mut b = (b_min.div_euclid(every)) * every;
            if b < b_min {
                b += every;
            }

            while b <= b_max {
                let rel = b - latest_bucket;

                let world_x = ((rel as f64) - phase) * col_f;
                let x_px = world_to_screen_x(world_x);

                if x_px >= -draw_margin_px && x_px <= (vw + draw_margin_px) {
                    let t_ms = (b as i128) * (aggr_time as i128);
                    let label = unix_ms_to_local_string(t_ms, fmt);

                    if !label.is_empty() {
                        let tick_label_len = label.chars().count() as f32;
                        let tick_label_w = tick_label_len * approx_char_w_px;
                        let tick_half = 0.5 * tick_label_w;

                        if let Some((cx, _, cw, _ch)) = cursor_label {
                            let cursor_half = 0.5 * cw;
                            if (x_px + tick_half + 2.0) >= (cx - cursor_half)
                                && (x_px - tick_half - 2.0) <= (cx + cursor_half)
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
                            size: font_size.into(),
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

            if let Some((x_px, label, label_w, label_h)) = cursor_label
                && x_px >= -draw_margin_px
                && x_px <= (vw + draw_margin_px)
                && !label.is_empty()
            {
                let mut bg = palette.secondary.base.color;
                bg = iced::Color { a: 1.0, ..bg };
                frame.fill_rectangle(
                    iced::Point::new(x_px - 0.5 * label_w, y - 0.5 * label_h),
                    iced::Size {
                        width: label_w,
                        height: label_h,
                    },
                    bg,
                );
                frame.fill_text(canvas::Text {
                    content: label,
                    position: iced::Point::new(x_px, y),
                    color: palette.secondary.base.text,
                    size: font_size.into(),
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
        interaction: &Self::State,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        if cursor.position_over(bounds).is_some() {
            match interaction {
                AxisInteraction::Panning { .. } => iced_core::mouse::Interaction::Grabbing,
                _ => iced_core::mouse::Interaction::ResizingHorizontally,
            }
        } else {
            iced_core::mouse::Interaction::default()
        }
    }
}
