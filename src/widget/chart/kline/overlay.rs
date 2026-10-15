use super::chrome::{PreparedChrome, SERIES_END_LABEL_GAP, series_display_name};
use super::drawing::{AxisBadgeFilter, DrawingView};
use super::layout::place_non_overlapping_labels;
use super::painting::{self, PaintInputs, PixelGeometry};
use super::presentation::IndicatorView;
use super::scene::{ChartCoordinates, Scene, XAxis};
use super::{CHAR_W, TEXT_SIZE, Y_AXIS_LABEL_LEFT_INSET};
use crate::style;
use data::chart::Basis;
use data::config::theme::ACCENT_COUNT;
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle};

/// Crosshair, last-price/countdown, and active drawing badges on the axes.
/// Retains countdown widths and collision decisions between scene preparations.
pub(super) struct AxisBadgeInputs {
    pub(super) interval: Option<u64>,
    pub(super) variants: Vec<CountdownVariant>,
}

pub(super) struct CountdownVariant {
    pub(super) timer_length: usize,
    pub(super) before_timer: PreparedChrome,
    pub(super) timer: Option<canvas::Text>,
    pub(super) after_timer: PreparedChrome,
}

impl AxisBadgeInputs {
    pub(super) fn resolve(&self, utc_ms: u64) -> PreparedChrome {
        let (variant, timer) = self.countdown_variant(utc_ms);
        variant.resolve(timer)
    }

    fn countdown_variant(&self, utc_ms: u64) -> (&CountdownVariant, Option<String>) {
        let timer = self.interval.and_then(|interval| {
            let remaining = ((utc_ms / interval + 1) * interval - utc_ms) / 1000;
            if remaining == 0 {
                return None;
            }
            let hours = remaining / 3600;
            let minutes = (remaining % 3600) / 60;
            let seconds = remaining % 60;
            Some(if hours > 0 {
                format!("{hours:02}:{minutes:02}:{seconds:02}")
            } else {
                format!("{minutes:02}:{seconds:02}")
            })
        });
        let length = timer.as_ref().map_or(0, String::len);
        let variant = self
            .variants
            .iter()
            .find(|variant| variant.timer_length == length)
            .expect("countdown variants cover the admitted timeframe");
        (variant, timer)
    }
}

impl CountdownVariant {
    fn resolve(&self, timer: Option<String>) -> PreparedChrome {
        let mut inputs = self.before_timer.clone();
        if let (Some(text), Some(content)) = (&self.timer, timer) {
            let mut text = text.clone();
            text.content = content;
            inputs.fill_text(text);
        }
        inputs.append(&self.after_timer);
        inputs
    }
}

pub(super) struct OverlayView<'a> {
    pub(super) coordinates: ChartCoordinates<'a>,
    pub(super) indicators: IndicatorView<'a>,
    pub(super) drawing: DrawingView<'a>,
}

impl OverlayView<'_> {
    fn fill_unavailable_indicator_messages(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
    ) {
        for indicator in &scene.indicator_panels {
            if !self.indicators.indicator_is_unavailable(indicator.value_id) {
                continue;
            }

            let Some(panel) = scene.layout.panel(indicator.panel_index) else {
                continue;
            };
            let Some(content) = indicator
                .value_id
                .and_then(|id| self.indicators.presentation.unavailable_message(id))
                .map(str::to_owned)
            else {
                continue;
            };
            let position = Point::new(
                panel.plot.x + panel.plot.width * 0.5,
                panel.plot.y + panel.plot.height * 0.5,
            );

            frame.with_clip(panel.plot, |frame| {
                frame.fill_text(canvas::Text {
                    content,
                    position,
                    color: palette.background.base.text.scale_alpha(0.72),
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            });
        }
    }

    pub(super) fn paint(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
        drawing_magnet: bool,
    ) {
        if scene.interaction.readouts_visible()
            && let Some(cursor) = scene.interaction.cursor
        {
            let magnetic_anchor = drawing_magnet
                .then(|| self.drawing.primary_magnetic_drawing_anchor_at_cursor())
                .flatten();

            let line_color = palette.background.base.text.scale_alpha(0.35);
            let (crosshair_width, crosshair_width_phys) = pixels.quantized_stroke_width(1.0);

            let gx = pixels.snap_x_for_stroke(
                scene.layout.regions.plot.x + cursor.x_plot,
                crosshair_width_phys,
            );
            let panel_plot = scene
                .layout
                .panel(cursor.panel_index)
                .map(|panel| panel.plot)
                .unwrap_or(*scene.primary_plot());
            let panel_bounds = (
                scene.layout.regions.plot.y + panel_plot.y,
                scene.layout.regions.plot.y + panel_plot.y + panel_plot.height,
            );
            let crosshair_y = magnetic_anchor
                .and_then(|anchor| {
                    self.drawing
                        .y_overlay_for_value(anchor.panel_id, anchor.y_unit)
                })
                .unwrap_or(scene.layout.regions.plot.y + cursor.y_plot);
            let gy = pixels
                .snap_y_for_stroke(crosshair_y, crosshair_width_phys)
                .clamp(panel_bounds.0, panel_bounds.1);

            let stroke = style::dashed_line_from_palette(palette)
                .with_color(line_color)
                .with_width(crosshair_width);

            frame.stroke(
                &canvas::Path::line(
                    Point::new(gx, scene.layout.regions.plot.y),
                    Point::new(
                        gx,
                        scene.layout.regions.plot.y + scene.layout.regions.plot.height,
                    ),
                ),
                stroke,
            );

            if cursor.y_primary_unit.is_some() || cursor.y_indicator_unit.is_some() {
                frame.stroke(
                    &canvas::Path::line(
                        Point::new(scene.layout.regions.plot.x, gy),
                        Point::new(
                            scene.layout.regions.plot.x + scene.layout.regions.plot.width,
                            gy,
                        ),
                    ),
                    stroke,
                );
            }
        }

        self.fill_primary_series_end_labels(frame, scene, palette, generated_accents);
        self.fill_unavailable_indicator_messages(frame, scene, palette);
    }

    pub(super) fn axis_badge_inputs(
        &self,
        scene: &Scene,
        bounds: Rectangle,
        palette: &Extended,
        drawing_magnet: bool,
        pixels: PixelGeometry,
    ) -> AxisBadgeInputs {
        let interval = match self.coordinates.basis {
            Basis::Time(timeframe) => Some(timeframe.to_milliseconds().max(1)),
            _ => None,
        };
        let mut timers = vec![None];
        if let Some(interval) = interval {
            timers.push(Some("00:00".to_owned()));
            let max_hours = (interval / 1000) / 3600;
            if max_hours > 0 {
                for digits in 2..=max_hours.to_string().len().max(2) {
                    timers.push(Some(format!("{}:00:00", "0".repeat(digits))));
                }
            }
        }
        let variants = timers
            .into_iter()
            .map(|timer| {
                self.prepare_countdown_variant(
                    scene,
                    bounds,
                    palette,
                    drawing_magnet,
                    pixels,
                    timer.as_deref(),
                )
            })
            .collect();
        AxisBadgeInputs { interval, variants }
    }

    fn prepare_countdown_variant(
        &self,
        scene: &Scene,
        bounds: Rectangle,
        palette: &Extended,
        drawing_magnet: bool,
        pixels: PixelGeometry,
        timer_text: Option<&str>,
    ) -> CountdownVariant {
        let mut before_timer = PreparedChrome::new(bounds);
        let frame = &mut before_timer;
        let mut badge_filter = AxisBadgeFilter::default();

        if scene.interaction.readouts_visible()
            && let Some(cursor) = scene.interaction.cursor
        {
            let magnetic_anchor = drawing_magnet
                .then(|| self.drawing.primary_magnetic_drawing_anchor_at_cursor())
                .flatten();
            let gy = magnetic_anchor
                .and_then(|anchor| {
                    self.drawing
                        .y_overlay_for_value(anchor.panel_id, anchor.y_unit)
                })
                .unwrap_or(scene.layout.regions.plot.y + cursor.y_plot);

            if let Some(y_text) = magnetic_anchor
                .map(|anchor| anchor.y_unit)
                .or(cursor.y_primary_unit)
                .map(|primary_unit| {
                    let value_scale = self.coordinates.panel_value_scale(cursor.panel_index);
                    let primary_value = value_scale.unit_to_value_f32(primary_unit);
                    scene.format_primary_cursor_label(primary_value)
                })
                .or_else(|| {
                    cursor.y_indicator_unit.map(|indicator_unit| {
                        let panel_precision =
                            self.coordinates.panel_value_precision(cursor.panel_index);
                        let value_scale = self.coordinates.panel_value_scale(cursor.panel_index);
                        let indicator_value = value_scale.unit_to_value(indicator_unit);
                        self.coordinates.format_panel_axis_value(
                            cursor.panel_index,
                            panel_precision,
                            indicator_value,
                            0.01,
                        )
                    })
                })
            {
                let y_badge_bounds = self
                    .drawing
                    .panel_plot_bounds_in_overlay(cursor.panel_index)
                    .unwrap_or_else(|| self.drawing.plot_bounds_in_overlay());
                self.drawing.draw_y_axis_badge(
                    frame,
                    palette,
                    gy,
                    &y_text,
                    y_badge_bounds,
                    &mut badge_filter,
                );
            }

            let x_text = self.format_x_crosshair_label(scene.x.axis, cursor.x_unit);
            self.drawing.draw_x_axis_badge(
                frame,
                palette,
                pixels.snap_x_to_cell(scene.layout.regions.plot.x + cursor.x_plot),
                &x_text,
                &mut badge_filter,
            );
        }

        let timer =
            self.fill_last_price_axis_badge(frame, scene, palette, &mut badge_filter, timer_text);
        let mut after_timer = PreparedChrome::new(bounds);
        self.drawing.fill_active_drawing_axis_labels(
            &mut after_timer,
            palette,
            &mut badge_filter,
            pixels,
        );
        CountdownVariant {
            timer_length: timer_text.map_or(0, str::len),
            before_timer,
            timer,
            after_timer,
        }
    }

    fn fill_last_price_axis_badge(
        &self,
        frame: &mut PreparedChrome,
        scene: &Scene,
        palette: &Extended,
        filter: &mut AxisBadgeFilter,
        timer_text: Option<&str>,
    ) -> Option<canvas::Text> {
        let base_series = self.coordinates.series.first()?;
        let last_point = base_series.latest_point()?;
        let last_bar = last_point.kline;
        let panel_bounds = self
            .drawing
            .panel_plot_bounds_in_overlay(scene.primary.panel_index)?;

        let last_price = last_bar.close.to_f32_lossy();
        let y = scene.layout.regions.plot.y
            + scene.map_primary_plot_with_anchor_f32(
                last_price,
                scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten(),
            );
        let color = if last_bar.close >= last_bar.open {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };
        let text = scene.format_primary_cursor_label(last_price);
        let value_text_color = if timer_text.is_some() {
            if palette.is_dark {
                iced::Color::BLACK
            } else {
                iced::Color::WHITE
            }
        } else {
            palette.primary.strong.text
        };
        let timer_text_color = if palette.is_dark {
            iced::Color::BLACK.scale_alpha(0.8)
        } else {
            iced::Color::WHITE.scale_alpha(0.8)
        };

        self.drawing.draw_y_axis_badge_with_timer(
            frame,
            y,
            &text,
            timer_text,
            panel_bounds,
            color,
            value_text_color,
            timer_text_color,
            filter,
        )
    }

    fn fill_primary_series_end_labels(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        for (index, label_y) in self.series_end_label_positions(scene) {
            let Some(series) = self.coordinates.series.get(index) else {
                continue;
            };
            let Some(bar) = series.bar_at_or_before_unit(scene.x.axis, scene.x.max_unit) else {
                continue;
            };
            let color = iced::Color {
                a: 1.0,
                ..painting::series_line_color(series, palette, generated_accents)
            };
            let text_color = if data::config::theme::is_dark(color) {
                iced::Color::WHITE
            } else {
                iced::Color::BLACK
            };
            let anchor = scene
                .primary
                .series_percent_anchors
                .get(index)
                .copied()
                .flatten();
            let value = anchor
                .filter(|anchor| anchor.abs() > f32::EPSILON)
                .map(|anchor| {
                    format!(
                        "{:+.2}%",
                        ((bar.close.to_f32_lossy() / anchor) - 1.0) * 100.0
                    )
                })
                .unwrap_or_else(|| scene.format_primary_cursor_label(bar.close.to_f32_lossy()));
            let label_height = TEXT_SIZE + 4.0;
            let value_rect = scene.layout.y_axis_badge_bounds(
                label_y - label_height * 0.5,
                label_height,
                value.chars().count() as f32 * CHAR_W,
            );
            let label = series_display_name(series);
            let label_width = label.chars().count() as f32 * CHAR_W + 8.0;
            let rect = Rectangle {
                x: value_rect.x - label_width,
                y: value_rect.y,
                width: label_width,
                height: label_height,
            };

            frame.fill_rectangle(rect.position(), rect.size(), color);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(rect.x + 4.0, label_y),
                color: text_color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });

            frame.fill_rectangle(value_rect.position(), value_rect.size(), color);
            frame.fill_text(canvas::Text {
                content: value,
                position: Point::new(value_rect.x + Y_AXIS_LABEL_LEFT_INSET, label_y),
                color: text_color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    pub(super) fn series_end_label_positions(&self, scene: &Scene) -> Vec<(usize, f32)> {
        if self.coordinates.series.len() < 2 {
            return Vec::new();
        }

        let primary_plot = scene.primary_plot();
        let plot_origin = scene.layout.regions.plot;
        let min_y = plot_origin.y + primary_plot.y;
        let max_y = min_y + primary_plot.height;
        let desired = self
            .coordinates
            .series
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, series)| {
                if !self
                    .coordinates
                    .series_participates_in_primary_plot(series.ticker_info)
                {
                    return None;
                }
                let bar = series.bar_at_or_before_unit(scene.x.axis, scene.x.max_unit)?;
                let anchor = scene
                    .primary
                    .series_percent_anchors
                    .get(index)
                    .copied()
                    .flatten();
                let y = (plot_origin.y
                    + scene.map_primary_plot_with_anchor_f32(bar.close.to_f32_lossy(), anchor))
                .clamp(min_y, max_y);

                Some((index, y))
            })
            .collect::<Vec<_>>();
        let desired_centers: Vec<f32> = desired.iter().map(|(_, y)| *y).collect();
        let placements = place_non_overlapping_labels(
            &desired_centers,
            min_y,
            max_y,
            TEXT_SIZE + 4.0,
            SERIES_END_LABEL_GAP,
        );

        desired
            .into_iter()
            .zip(placements)
            .filter_map(|((index, _), placement)| placement.map(|y| (index, y)))
            .collect()
    }

    fn format_x_crosshair_label(&self, x_axis: XAxis, unit: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    self.coordinates
                        .timezone
                        .format_with_kind(
                            ts.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Crosshair { show_millis: false },
                        )
                        .unwrap_or_else(|| ts.as_u64().to_string())
                },
            ),
            XAxis::Tick { .. } => self
                .coordinates
                .tick_time_for_unit(x_axis, unit)
                .and_then(|time| {
                    self.coordinates.timezone.format_with_kind(
                        time.as_u64() as i64,
                        data::config::timezone::TimeLabelKind::Crosshair { show_millis: true },
                    )
                })
                .unwrap_or_else(|| self.coordinates.format_x_label(x_axis, unit, 1)),
        }
    }

    pub(super) fn paint_last_price_line(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(base_series) = self.coordinates.series.first() else {
            return;
        };
        let Some(last_point) = base_series.latest_point() else {
            return;
        };
        let last_bar = last_point.kline;

        let last_price = last_bar.close.to_f32_lossy();
        let y = scene.map_primary_plot_with_anchor_f32(
            last_price,
            scene
                .primary
                .series_percent_anchors
                .first()
                .copied()
                .flatten(),
        );
        let color = if last_bar.close >= last_bar.open {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };
        let stroke = canvas::Stroke {
            width: 1.0,
            line_dash: canvas::LineDash {
                segments: &[2.0, 2.0],
                offset: 4,
            },
            ..Default::default()
        }
        .with_color(color.scale_alpha(0.5));
        let primary_plot = scene.primary_plot();

        frame.with_clip(*primary_plot, |frame| {
            frame.stroke(
                &canvas::Path::line(
                    Point::new(primary_plot.x, y),
                    Point::new(primary_plot.x + primary_plot.width, y),
                ),
                stroke,
            );
        });
    }
}
