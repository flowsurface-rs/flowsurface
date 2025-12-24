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

pub struct AxisXLabelCanvas {
    pub latest_time: u64,
    pub aggr_time: Option<u64>,
    pub column_world: f32,
    pub cam_offset_x: f32,
    pub cam_sx: f32,
    pub cam_right_pad_frac: f32, // NEW
}

impl canvas::Program<Message> for AxisXLabelCanvas {
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
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let Some(aggr_time) = self.aggr_time else {
            return vec![frame.into_geometry()];
        };

        if self.latest_time == 0
            || aggr_time == 0
            || !self.column_world.is_finite()
            || self.column_world <= 0.0
            || !self.cam_sx.is_finite()
            || self.cam_sx <= 0.0
        {
            return vec![frame.into_geometry()];
        }

        let vw = bounds.width;
        let vh = bounds.height;

        // Match camera.rs exactly:
        let pad_world = (vw * self.cam_right_pad_frac) / self.cam_sx;
        let right_edge_world = self.cam_offset_x + pad_world;

        let x_world_left = right_edge_world - (vw / self.cam_sx);
        let x_world_right = right_edge_world;

        // Visible relative bucket range u where world_x = u * column_world
        let u_min = (x_world_left / self.column_world).floor() as i64;
        let u_max = (x_world_right / self.column_world).ceil() as i64;

        // Convert to ABSOLUTE bucket coordinates so ticks scroll when latest_time advances
        let latest_bucket: i64 = (self.latest_time / aggr_time) as i64;
        let b_min: i64 = latest_bucket.saturating_add(u_min);
        let b_max: i64 = latest_bucket.saturating_add(u_max);

        // Formatting based on visible timespan (wall-clock)
        let visible_buckets = (b_max as i128 - b_min as i128).max(0);
        let visible_span_ms = visible_buckets * (aggr_time as i128);
        let fmt = pick_time_format(visible_span_ms);

        // Label density (aim ~110px)
        let px_per_bucket = (self.column_world * self.cam_sx).max(1e-6);
        let target_label_px = 110.0f32;
        let rough_every = (target_label_px / px_per_bucket).ceil() as i64;
        let every = super::nice_step_i64(rough_every);

        let text_color = theme.palette().text;
        let font_size = 12.0f32;

        // Camera center_x (camera.rs: center_x = right_edge - (vw*0.5)/sx)
        let center_x = right_edge_world - (vw * 0.5) / self.cam_sx;

        let world_to_screen_x =
            |world_x: f32| -> f32 { (world_x - center_x) * self.cam_sx + vw * 0.5 };

        let y = 0.5 * vh;
        let edge_pad = 26.0f32;

        // Start at first multiple of `every` in [b_min, b_max]
        let mut b = (b_min.div_euclid(every)) * every;
        if b < b_min {
            b += every;
        }

        while b <= b_max {
            // Map absolute bucket -> world_x using same "relative to latest" convention as data
            let rel = b - latest_bucket; // negative for past
            let world_x = (rel as f32) * self.column_world;
            let x_px = world_to_screen_x(world_x);

            if x_px >= edge_pad && x_px <= (vw - edge_pad) {
                // Label time is absolute: t = bucket * aggr_time
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

        vec![frame.into_geometry()]
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
