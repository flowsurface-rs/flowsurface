use super::helpers::PixelGeometry;
use super::*;
use data::chart::kline::TpoProfileStatus;
use data::config::theme::ACCENT_COUNT;

impl<'a> KlineWidget<'a> {
    pub(super) fn fill_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
        drawing_magnet: bool,
        selected_tpo_profile: Option<UnixMs>,
        tpo_boundary_preview: Option<UnixMs>,
    ) {
        self.fill_corner_controls(frame, scene, palette);
        self.fill_panel_controls(frame, scene, palette);
        self.fill_primary_ticker_legend(frame, scene, palette, generated_accents);
        self.fill_primary_series_end_labels(frame, scene, palette, generated_accents);
        self.fill_footprint_legend(frame, scene, palette);
        self.fill_study_legend(frame, scene, palette);
        self.fill_indicator_legend(frame, scene, palette, generated_accents);
        self.fill_unavailable_indicator_messages(frame, scene, palette);
        self.fill_tpo_profile_controls(
            frame,
            scene,
            palette,
            pixels,
            selected_tpo_profile,
            tpo_boundary_preview,
        );

        let show_primary_panel_values = scene.chrome.ticker_legend.is_none();
        self.fill_panel_header_values(
            frame,
            scene,
            palette,
            generated_accents,
            show_primary_panel_values,
        );

        if self.drawings.has_state() {
            self.fill_drawing_interaction(frame, scene, palette, generated_accents, pixels);
        }

        if !scene.interaction.readouts_visible() {
            return;
        }

        let Some(cursor) = scene.interaction.cursor else {
            return;
        };
        let magnetic_anchor = drawing_magnet
            .then(|| self.primary_magnetic_drawing_anchor_at_cursor(scene))
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
            .and_then(|anchor| self.y_overlay_for_value(scene, anchor.panel_id, anchor.y_unit))
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

    pub(super) fn fill_tpo_profile_controls(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        pixels: PixelGeometry,
        selected_profile: Option<UnixMs>,
        boundary_preview: Option<UnixMs>,
    ) {
        let geometries = self.tpo_profile_geometries(scene, selected_profile);
        if geometries.is_empty() {
            return;
        }

        let (border_width, _) = pixels.quantized_stroke_width(1.25);
        let border_color = palette.primary.base.color.scale_alpha(0.95);
        for geometry in &geometries {
            let Some(profile) = self.tpo_profiles.get(geometry.index) else {
                continue;
            };
            if profile.status == TpoProfileStatus::Incomplete {
                self.fill_tpo_incomplete_marker(frame, geometry, palette);
            }
        }

        for geometry in geometries
            .iter()
            .filter(|geometry| selected_profile == Some(geometry.start))
        {
            frame.stroke(
                &canvas::Path::rectangle(
                    geometry.overlay_bounds.position(),
                    geometry.overlay_bounds.size(),
                ),
                canvas::Stroke::default()
                    .with_color(border_color)
                    .with_width(border_width),
            );

            for (button, label) in [
                (geometry.reset_button_bounds, "Reset"),
                (geometry.button_bounds, "Merge ←"),
            ] {
                let Some(button) = button else {
                    continue;
                };
                frame.fill_rectangle(
                    button.position(),
                    button.size(),
                    palette.primary.base.color.scale_alpha(0.92),
                );
                frame.stroke(
                    &canvas::Path::rectangle(button.position(), button.size()),
                    canvas::Stroke::default()
                        .with_color(palette.primary.strong.text.scale_alpha(0.85))
                        .with_width(border_width),
                );
                frame.fill_text(canvas::Text {
                    content: label.to_owned(),
                    position: Point::new(
                        button.x + button.width * 0.5,
                        button.y + button.height * 0.5,
                    ),
                    color: palette.primary.base.text,
                    size: 10.0.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }

        let Some(boundary_preview) = boundary_preview else {
            return;
        };
        let Some(unit) = scene.x.axis.unit_from_time_f32(boundary_preview) else {
            return;
        };
        let plot = scene.primary_plot();
        let x = scene.layout.regions.plot.x + scene.map_x_plot_f32(unit);
        let y = scene.layout.regions.plot.y + plot.y;
        frame.stroke(
            &canvas::Path::line(Point::new(x, y), Point::new(x, y + plot.height)),
            style::dashed_line_from_palette(palette)
                .with_color(palette.primary.base.color)
                .with_width(border_width),
        );
    }

    fn fill_tpo_incomplete_marker(
        &self,
        frame: &mut canvas::Frame,
        geometry: &TpoProfileGeometry,
        palette: &Extended,
    ) {
        let marker_width = CHAR_W;
        let marker_height = TEXT_SIZE;
        if geometry.overlay_bounds.width < marker_width
            || geometry.overlay_bounds.height < marker_height
        {
            return;
        }

        let marker_right = geometry
            .reset_button_bounds
            .or(geometry.button_bounds)
            .map_or(
                geometry.overlay_bounds.x + geometry.overlay_bounds.width
                    - TPO_ACTION_BUTTON_GAP_PX,
                |button| button.x - TPO_ACTION_BUTTON_GAP_PX,
            );
        let marker_x = marker_right - marker_width * 0.5;
        let marker_y = geometry.overlay_bounds.y + TPO_ACTION_BUTTON_GAP_PX + marker_height * 0.5;

        frame.fill_text(canvas::Text {
            content: "!".to_owned(),
            position: Point::new(marker_x, marker_y),
            color: palette.danger.base.color,
            size: marker_height.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
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
                .then(|| self.primary_magnetic_drawing_anchor_at_cursor(scene))
                .flatten();
            let gy = magnetic_anchor
                .and_then(|anchor| self.y_overlay_for_value(scene, anchor.panel_id, anchor.y_unit))
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
                    .panel_plot_bounds_in_overlay(scene, cursor.panel_index)
                    .unwrap_or_else(|| self.plot_bounds_in_overlay(scene));
                self.draw_y_axis_badge(
                    frame,
                    scene,
                    palette,
                    gy,
                    &y_text,
                    y_badge_bounds,
                    &mut badge_filter,
                );
            }

            let x_text = self.format_x_crosshair_label(scene.x.axis, cursor.x_unit);
            self.draw_x_axis_badge(
                frame,
                scene,
                palette,
                pixels.snap_x_to_cell(scene.layout.regions.plot.x + cursor.x_plot),
                &x_text,
                &mut badge_filter,
            );
        }

        self.fill_last_price_axis_badge(frame, scene, palette, &mut badge_filter);
        self.fill_active_drawing_axis_labels(frame, scene, palette, &mut badge_filter, pixels);
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
        let Some(panel_bounds) =
            self.panel_plot_bounds_in_overlay(scene, scene.primary.panel_index)
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

        self.draw_y_axis_badge_with_timer(
            frame,
            scene,
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
