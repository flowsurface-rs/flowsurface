use super::helpers::PixelGeometry;
use super::*;
use data::config::theme::ACCENT_COUNT;

impl<'a> KlineWidget<'a> {
    pub(super) fn fill_legends(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        self.fill_primary_ticker_legend(frame, scene, palette, generated_accents);
        self.fill_footprint_legend(frame, scene, palette);
        self.fill_study_legend(frame, scene, palette);
        self.fill_indicator_legend(frame, scene, palette, generated_accents);
        self.fill_panel_header_values(
            frame,
            scene,
            palette,
            generated_accents,
            scene.chrome.ticker_legend.is_none(),
        );
    }

    pub(super) fn fill_overlay(
        &self,
        frame: &mut canvas::Frame,
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
                .then(|| {
                    self.drawing_view(scene)
                        .primary_magnetic_drawing_anchor_at_cursor()
                })
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
                    self.drawing_view(scene)
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

        self.fill_corner_controls(frame, scene, palette);
        self.fill_panel_controls(frame, scene, palette);
        self.fill_primary_series_end_labels(frame, scene, palette, generated_accents);
        self.fill_unavailable_indicator_messages(frame, scene, palette);
    }

    pub(super) fn fill_overlay_interaction_text(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        drawing_magnet: bool,
        pixels: PixelGeometry,
    ) {
        let mut badge_filter = AxisBadgeFilter::default();

        if scene.interaction.readouts_visible()
            && let Some(cursor) = scene.interaction.cursor
        {
            let magnetic_anchor = drawing_magnet
                .then(|| {
                    self.drawing_view(scene)
                        .primary_magnetic_drawing_anchor_at_cursor()
                })
                .flatten();
            let gy = magnetic_anchor
                .and_then(|anchor| {
                    self.drawing_view(scene)
                        .y_overlay_for_value(anchor.panel_id, anchor.y_unit)
                })
                .unwrap_or(scene.layout.regions.plot.y + cursor.y_plot);

            if let Some(y_text) = magnetic_anchor
                .map(|anchor| anchor.y_unit)
                .or(cursor.y_primary_unit)
                .map(|primary_unit| {
                    let value_scale = self.panel_value_scale(cursor.panel_index);
                    let primary_value = value_scale.unit_to_value_f32(primary_unit);
                    scene.format_primary_cursor_label(primary_value)
                })
                .or_else(|| {
                    cursor.y_indicator_unit.map(|indicator_unit| {
                        let panel_precision = self.panel_value_precision(cursor.panel_index);
                        let value_scale = self.panel_value_scale(cursor.panel_index);
                        let indicator_value = value_scale.unit_to_value(indicator_unit);
                        self.format_panel_axis_value(
                            cursor.panel_index,
                            panel_precision,
                            indicator_value,
                            0.01,
                        )
                    })
                })
            {
                let y_badge_bounds = self
                    .drawing_view(scene)
                    .panel_plot_bounds_in_overlay(cursor.panel_index)
                    .unwrap_or_else(|| self.drawing_view(scene).plot_bounds_in_overlay());
                self.drawing_view(scene).draw_y_axis_badge(
                    frame,
                    palette,
                    gy,
                    &y_text,
                    y_badge_bounds,
                    &mut badge_filter,
                );
            }

            let x_text = self.format_x_crosshair_label(scene.x.axis, cursor.x_unit);
            self.drawing_view(scene).draw_x_axis_badge(
                frame,
                palette,
                pixels.snap_x_to_cell(scene.layout.regions.plot.x + cursor.x_plot),
                &x_text,
                &mut badge_filter,
            );
        }

        self.fill_last_price_axis_badge(frame, scene, palette, &mut badge_filter);
        self.drawing_view(scene).fill_active_drawing_axis_labels(
            frame,
            palette,
            &mut badge_filter,
            pixels,
        );
    }

    pub(super) fn fill_last_price_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        filter: &mut AxisBadgeFilter,
    ) {
        let Some(base_series) = self.series.first() else {
            return;
        };
        let Some(last_point) = base_series.latest_point() else {
            return;
        };
        let last_bar = last_point.kline;
        let Some(panel_bounds) = self
            .drawing_view(scene)
            .panel_plot_bounds_in_overlay(scene.primary.panel_index)
        else {
            return;
        };

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
        let timer_text = self.candle_close_countdown();
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

        self.drawing_view(scene).draw_y_axis_badge_with_timer(
            frame,
            y,
            &text,
            timer_text.as_deref(),
            panel_bounds,
            color,
            value_text_color,
            timer_text_color,
            filter,
        );
    }

    pub(super) fn candle_close_countdown(&self) -> Option<String> {
        let Basis::Time(timeframe) = self.basis else {
            return None;
        };

        let interval = timeframe.to_milliseconds();
        let current_time = chrono::Utc::now().timestamp_millis() as u64;
        let next_kline_open = (current_time / interval + 1) * interval;
        let remaining_seconds = (next_kline_open - current_time) / 1000;

        if remaining_seconds == 0 {
            return None;
        }

        let hours = remaining_seconds / 3600;
        let minutes = (remaining_seconds % 3600) / 60;
        let seconds = remaining_seconds % 60;

        Some(if hours > 0 {
            format!("{hours:02}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes:02}:{seconds:02}")
        })
    }
}
