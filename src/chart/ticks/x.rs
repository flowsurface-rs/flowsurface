use super::{AxisLabel, TEXT_SIZE, X_LABEL_CHAR_W};
use data::UserTimezone;
use data::chart::Autoscale;
use data::chart::ticks::x::{TimeTickLabel, TimeTickTier};

use data::config::timezone::TimeLabelKind;
use exchange::UnixMs;
use iced::Point;

use super::{Basis, Interaction, Message};
use iced::theme::palette::Extended;
use iced::{
    Event, Rectangle, Renderer, Size, Theme, mouse,
    widget::canvas::{self, Cache, Geometry},
};

fn is_drawable(x_pos: f32, width: f32) -> bool {
    x_pos >= -TEXT_SIZE * 5.0 && x_pos <= width + TEXT_SIZE * 5.0
}

/// Minimum pixel spacing between tick labels, sized so the widest label
/// (`HH:MM` / `MM:SS`, 5 chars) never touches a neighbour.
const MIN_TICK_SPACING: f32 = X_LABEL_CHAR_W * 11.0;

pub fn generate_time_labels(
    timeframe: exchange::Timeframe,
    timezone: UserTimezone,
    axis_bounds: iced_core::Rectangle,
    earliest: UnixMs,
    latest: UnixMs,
    x_labels_can_fit: i32,
    palette: &Extended,
) -> Vec<AxisLabel> {
    let width = axis_bounds.width;

    let raw = TimeTickLabel::for_range(
        earliest,
        latest,
        width,
        MIN_TICK_SPACING,
        x_labels_can_fit,
        timeframe,
        timezone,
    );

    let mut labels = Vec::with_capacity(raw.len());
    for label in raw {
        if !is_drawable(label.x_pos, width) {
            continue;
        }
        labels.push(AxisLabel::new_x(
            label.x_pos,
            label.content,
            axis_bounds,
            label.tier,
            false,
            palette,
        ));
    }

    labels
}

// X-AXIS LABELS
pub struct AxisLabelsX<'a> {
    pub labels_cache: &'a Cache,
    pub max: u64,
    pub scaling: f32,
    pub translation_x: f32,
    pub basis: Basis,
    pub cell_width: f32,
    pub timezone: data::UserTimezone,
    pub chart_bounds: Rectangle,
    pub interval_keys: Option<Vec<u64>>,
    pub autoscaling: Option<data::chart::Autoscale>,
}

impl AxisLabelsX<'_> {
    fn drag_bounds(bounds: Rectangle) -> Rectangle {
        bounds.shrink(super::AXIS_DRAG_EDGE_GUARD)
    }

    fn calc_crosshair_pos(&self, cursor_pos: Point, region: Rectangle) -> (f32, f32, i32) {
        let crosshair_ratio = f64::from(cursor_pos.x) / f64::from(self.chart_bounds.width);
        let chart_x_min = region.x;
        let crosshair_pos = chart_x_min + crosshair_ratio as f32 * region.width;
        let cell_index = (crosshair_pos / self.cell_width).round();

        (crosshair_pos, crosshair_ratio as f32, cell_index as i32)
    }

    fn generate_crosshair(
        &self,
        cursor_pos: Point,
        region: Rectangle,
        bounds: Rectangle,
        palette: &Extended,
    ) -> Option<AxisLabel> {
        match self.basis {
            Basis::Tick(_) => {
                let Some(interval_keys) = &self.interval_keys else {
                    return None;
                };

                let (crosshair_pos, _, cell_index) = self.calc_crosshair_pos(cursor_pos, region);

                let chart_x_min = region.x;
                let chart_x_max = region.x + region.width;

                let snapped_position = (crosshair_pos / self.cell_width).round() * self.cell_width;
                let snap_ratio = (snapped_position - chart_x_min) / (chart_x_max - chart_x_min);
                let snap_x = snap_ratio * bounds.width;

                if snap_x.is_nan() || snap_x < 0.0 || snap_x > bounds.width {
                    return None;
                }

                let last_index = interval_keys.len() - 1;
                let offset = i64::from(-cell_index) as usize;
                if offset > last_index {
                    return None;
                }

                let array_index = last_index - offset;

                if let Some(timestamp) = interval_keys.get(array_index) {
                    let label_content = self.timezone.format_with_kind(
                        *timestamp as i64,
                        TimeLabelKind::Crosshair { show_millis: true },
                    );

                    if let Some(content) = label_content {
                        return Some(AxisLabel::new_x(
                            snap_x,
                            content,
                            bounds,
                            TimeTickTier::Main,
                            true,
                            palette,
                        ));
                    }
                }
            }
            Basis::Time(timeframe) => {
                let (_, crosshair_ratio, _) = self.calc_crosshair_pos(cursor_pos, region);

                let x_min = self.x_to_interval(region.x);
                let x_max = self.x_to_interval(region.x + region.width);

                let crosshair_millis =
                    x_min as f64 + f64::from(crosshair_ratio) * (x_max as f64 - x_min as f64);

                let interval = timeframe.to_milliseconds();

                let crosshair_time =
                    chrono::DateTime::from_timestamp_millis(crosshair_millis as i64)?;
                let rounded_timestamp =
                    (crosshair_time.timestamp_millis() as f64 / (interval as f64)).round() as u64
                        * interval;

                let snap_ratio =
                    (rounded_timestamp as f64 - x_min as f64) / (x_max as f64 - x_min as f64);

                let snap_x = snap_ratio * f64::from(bounds.width);
                if snap_x.is_nan() || snap_x < 0.0 || snap_x > f64::from(bounds.width) {
                    return None;
                }

                let label_content = self.timezone.format_with_kind(
                    rounded_timestamp as i64,
                    TimeLabelKind::Crosshair {
                        show_millis: interval < 10_000,
                    },
                );

                if let Some(content) = label_content {
                    return Some(AxisLabel::new_x(
                        snap_x as f32,
                        content,
                        bounds,
                        TimeTickTier::Main,
                        true,
                        palette,
                    ));
                }
            }
        }
        None
    }

    fn visible_region(&self, size: Size) -> Rectangle {
        let width = size.width / self.scaling;
        let height = size.height / self.scaling;

        Rectangle {
            x: -self.translation_x - width / 2.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn x_to_interval(&self, x: f32) -> u64 {
        match self.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds() as f64;

                if x <= 0.0 {
                    let diff = (f64::from(-x / self.cell_width) * interval) as u64;
                    self.max.saturating_sub(diff)
                } else {
                    let diff = (f64::from(x / self.cell_width) * interval) as u64;
                    self.max.saturating_add(diff)
                }
            }
            Basis::Tick(_) => {
                let tick = -(x / self.cell_width);
                tick.round() as u64
            }
        }
    }
}

impl canvas::Program<Message> for AxisLabelsX<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let drag_bounds = Self::drag_bounds(bounds);

        if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            *interaction = Interaction::None;
        }

        if let Event::Mouse(mouse_event) = event {
            match mouse_event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if cursor.position_in(drag_bounds).is_some()
                        && let Some(cursor_position) = cursor.position()
                    {
                        *interaction = Interaction::Zoomin {
                            last_position: cursor_position,
                        };
                    }
                }
                mouse::Event::CursorMoved { .. } => {
                    if let Interaction::Zoomin {
                        ref mut last_position,
                    } = *interaction
                        && let Some(cursor_position) = cursor.position()
                    {
                        let difference_x = last_position.x - cursor_position.x;

                        if difference_x.abs() > 1.0 {
                            *last_position = cursor_position;

                            let delta = if self.autoscaling == Some(Autoscale::FitToVisible) {
                                difference_x * 0.05
                            } else {
                                difference_x * 0.2
                            };

                            let message = Message::XScaling(delta, 0.0, false);

                            return Some(canvas::Action::publish(message).and_capture());
                        }
                    }
                }
                mouse::Event::WheelScrolled { delta } => match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        cursor.position_in(drag_bounds)?;

                        let message = Message::XScaling(
                            *y,
                            {
                                if let Some(cursor_to_center) =
                                    cursor.position_from(bounds.center())
                                {
                                    cursor_to_center.x
                                } else {
                                    0.0
                                }
                            },
                            true,
                        );

                        return Some(canvas::Action::publish(message).and_capture());
                    }
                },
                _ => {}
            }
        }

        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = theme.extended_palette();

        let labels = self.labels_cache.draw(renderer, bounds.size(), |frame| {
            let region = self.visible_region(frame.size());

            let target_spacing = super::REGULAR_LABEL_WIDTH * 2.0;
            let target_count = (bounds.width / target_spacing).floor() as usize;

            let label_count = target_count.max(2);

            let mut labels: Vec<AxisLabel> = Vec::with_capacity(label_count + 1); // +1 for crosshair

            match self.basis {
                Basis::Tick(_) => {
                    if let Some(interval_keys) = &self.interval_keys {
                        let last_idx = interval_keys.len() - 1;
                        let mut last_x: Option<f32> = None;
                        for (i, timestamp) in interval_keys.iter().enumerate() {
                            let cell_index = -(last_idx as i32) + i as i32;
                            let x_position = cell_index as f32 * self.cell_width;

                            let x_min_region = region.x;
                            let x_max_region = region.x + region.width;
                            let snap_ratio = if (x_max_region - x_min_region).abs() < f32::EPSILON {
                                0.5
                            } else {
                                (x_position - x_min_region) / (x_max_region - x_min_region)
                            };
                            let snap_x = snap_ratio * bounds.width;

                            if last_x.is_none_or(|lx| (snap_x - lx).abs() >= target_spacing) {
                                let label_content = self.timezone.format_with_kind(
                                    *timestamp as i64,
                                    TimeLabelKind::Axis {
                                        timeframe: exchange::Timeframe::MS100,
                                    },
                                );

                                if let Some(content) = label_content {
                                    labels.push(AxisLabel::new_x(
                                        snap_x,
                                        content,
                                        bounds,
                                        TimeTickTier::Main,
                                        false,
                                        palette,
                                    ));

                                    last_x = Some(snap_x);
                                }
                            }
                        }
                    }
                }
                Basis::Time(timeframe) => {
                    let earliest = exchange::UnixMs(self.x_to_interval(region.x));
                    let latest = exchange::UnixMs(self.x_to_interval(region.x + region.width));

                    let generated_labels = generate_time_labels(
                        timeframe,
                        self.timezone,
                        bounds,
                        earliest,
                        latest,
                        label_count as i32,
                        palette,
                    );

                    labels.extend(generated_labels);
                }
            }

            if let Some(cursor_pos) = cursor.position_in(self.chart_bounds)
                && let Some(label) = self.generate_crosshair(cursor_pos, region, bounds, palette)
            {
                labels.push(label);
            }

            AxisLabel::filter_and_draw(&labels, frame);
        });

        vec![labels]
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Panning { .. } => mouse::Interaction::None,
            Interaction::Zoomin { .. } => mouse::Interaction::ResizingHorizontally,
            Interaction::None if cursor.is_over(Self::drag_bounds(bounds)) => {
                mouse::Interaction::ResizingHorizontally
            }
            _ => mouse::Interaction::default(),
        }
    }
}
