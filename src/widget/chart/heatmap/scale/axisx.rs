use super::{AxisInteraction, AxisInteractionKind, Message};
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
                state.kind = AxisInteractionKind::Panning { last_position: p };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.kind = AxisInteractionKind::None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let AxisInteractionKind::Panning { last_position } = &mut state.kind
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

            // Phase should behave like shader origin.x: fraction of a bucket (0..1)
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

            // phase here (geometry shift), not by moving the camera.
            // We want (u - phase) * col in [x_left, x_right]
            // => u in [x_left/col + phase, x_right/col + phase]
            let inv_col = 1.0f64 / col_f.max(1e-18);
            let eps = 1e-9f64;

            let u_min = ((x_world_left * inv_col) + phase + eps).floor() as i64;
            let u_max = ((x_world_right * inv_col) + phase - eps).ceil() as i64;

            let latest_bucket: i64 = self.latest_bucket;
            let b_min: i64 = latest_bucket.saturating_add(u_min);
            let b_max: i64 = latest_bucket.saturating_add(u_max);

            let visible_buckets = (b_max as i128 - b_min as i128).max(0);
            let visible_span_ms = visible_buckets * (aggr_time as i128);
            let fmt = pick_time_format(visible_span_ms);

            let px_per_bucket = (col_f * cam_sx_f).max(1e-9) as f32;
            let target_label_px = 110.0f32;
            let rough_every = (target_label_px / px_per_bucket).ceil() as i64;
            let every = super::nice_step_i64(rough_every.max(1));

            let text_color = theme.palette().text;
            let font_size = 12.0f32;

            let center_x = right_edge_world - (vw_f * 0.5) / cam_sx_f;
            let world_to_screen_x =
                |world_x: f64| -> f32 { ((world_x - center_x) * cam_sx_f + vw_f * 0.5) as f32 };

            let y = (0.5 * vh_f) as f32;
            let edge_pad = 26.0f32;

            let mut b = (b_min.div_euclid(every)) * every;
            if b < b_min {
                b += every;
            }

            while b <= b_max {
                let rel = b - latest_bucket;

                let world_x = ((rel as f64) - phase) * col_f;
                let x_px = world_to_screen_x(world_x);

                if x_px >= edge_pad && x_px <= (vw - edge_pad) {
                    let t_ms = (b as i128) * (aggr_time as i128);
                    let label = unix_ms_to_local_string(t_ms, fmt);

                    if !label.is_empty() {
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
            match state.kind {
                AxisInteractionKind::Panning { .. } => iced_core::mouse::Interaction::Grabbing,
                _ => iced_core::mouse::Interaction::default(),
            }
        } else {
            iced_core::mouse::Interaction::default()
        }
    }
}
