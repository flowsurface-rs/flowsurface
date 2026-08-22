use super::*;
use crate::widget::chart::unit_ticks;

const MIN_X_TICK_PX: f32 = 80.0;

pub(super) fn horizontal_world_for_drag(
    scene: &Scene,
    initial_world: HorizontalWorld,
    initial_right_unit: f32,
    anchor_x: f32,
    delta: f32,
) -> HorizontalWorld {
    let right_edge = scene.x_axis_plot_width().floor().max(1.0);
    let anchor_x = (anchor_x - scene.layout.regions.plot.x).clamp(0.0, right_edge);

    initial_world.zoom_around_screen(initial_right_unit, anchor_x, right_edge, delta)
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn horizontal_world_for_cursor(
        &self,
        scene: &Scene,
        cursor_x: f32,
        delta: f32,
    ) -> HorizontalWorld {
        let right_edge = scene.x_axis_plot_width().floor().max(1.0);
        let anchor_x = self.horizontal_zoom_anchor(scene, cursor_x, right_edge);
        let right_unit = scene.effective_right_x_unit();
        self.horizontal_world
            .zoom_around_screen(right_unit, anchor_x, right_edge, delta)
    }

    pub(super) fn horizontal_zoom_anchor(
        &self,
        scene: &Scene,
        cursor_x: f32,
        right_edge: f32,
    ) -> f32 {
        let cursor_x = (cursor_x - scene.layout.regions.plot.x).clamp(0.0, right_edge);
        if !self.primary_autoscale {
            return cursor_x;
        }

        let latest_x = scene.map_x_plot(0);
        if latest_x.is_finite() && (0.0..=right_edge).contains(&latest_x) {
            latest_x
        } else {
            right_edge
        }
    }

    pub(super) fn fill_last_price_line(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(base_series) = self.series.first() else {
            return;
        };
        let Some(last_bar) = base_series.bars().last() else {
            return;
        };

        let last_price = last_bar.close.to_f32_lossy();
        let y = scene.map_primary_plot_with_anchor(
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

    pub(super) fn fill_panel_titles(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let Some(title) = self.resolved_panel_title(panel_index, panel.kind) else {
                continue;
            };

            if panel.kind == KlinePanelKind::Indicator && self.panel_value_id(panel_index).is_some()
            {
                continue;
            }

            let value_id = self.panel_value_id(panel_index);
            let title_x = value_id.map_or(panel.plot.x + TOOLTIP_ROW_LEFT_PAD, |value_id| {
                self.indicator_label_x(panel.plot.x, value_id)
            });
            let title_y = panel.plot.y + PANEL_TITLE_TOP_PAD + TICKER_LEGEND_ROW_H * 0.5;

            if let Some(value_id) = value_id
                && self.indicator_is_unavailable(Some(value_id))
            {
                frame.fill_text(canvas::Text {
                    content: "!".to_string(),
                    position: Point::new(self.indicator_marker_x(panel.plot.x), title_y),
                    color: palette.danger.base.color,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }

            frame.fill_text(canvas::Text {
                content: title.to_string(),
                position: Point::new(title_x, title_y),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    pub(super) fn y_axis_label_specs(&self, scene: &Scene) -> Vec<(usize, String, f32)> {
        let labels_for_height =
            |height: f32| y_labels_that_fit(height, TEXT_SIZE, Y_LABEL_DENSITY).max(2) as i32;
        let mut labels = Vec::new();
        let uses_display_space_ticks = matches!(
            scene.primary.mode,
            PanelScaleMode::PercentFromBase | PanelScaleMode::Logarithmic
        ) || scene.primary.display_domain_override.is_some();

        if !uses_display_space_ticks {
            if let Some(ticker) = self.base_ticker_info() {
                use data::chart::ticks::y::PriceAxis;

                let price_axis =
                    PriceAxis::new(ticker.min_ticksize.into(), Some(ticker.min_ticksize));
                let (min_price_value, max_price_value) = scene.primary.domain.values;
                let min_price = min_price_value as f64;
                let max_price = max_price_value as f64;
                let labels_can_fit = labels_for_height(scene.primary_plot().height);
                let y_ticks = price_axis.ticks(min_price, max_price, labels_can_fit);
                let step_value = y_ticks.step.unwrap_or(1.0) as f32;

                for tick in y_ticks.ticks {
                    let TickValue::PriceUnits(units) = tick.value else {
                        continue;
                    };
                    let price = Price::from_units(units).to_f32_lossy();
                    let value = price;
                    let y = scene.map_primary_plot_value(value).clamp(
                        scene.primary_plot().y,
                        scene.primary_plot().y + scene.primary_plot().height,
                    );
                    labels.push((
                        scene.primary.panel_index,
                        scene.format_primary_axis_label(value, step_value),
                        y,
                    ));
                }
            } else {
                let (min_domain_value, max_domain_value) = scene.primary.domain.values;
                let min_value = min_domain_value as f64;
                let max_value = max_domain_value as f64;
                let y_ticks = YAxisScale::Linear.ticks(
                    min_value,
                    max_value,
                    labels_for_height(scene.primary_plot().height),
                );
                let step_value = y_ticks.step.unwrap_or(1.0) as f32;

                for tick in y_ticks.ticks {
                    let TickValue::Float(value) = tick.value else {
                        continue;
                    };
                    let value = value as f32;
                    let y = scene.map_primary_plot_value(value).clamp(
                        scene.primary_plot().y,
                        scene.primary_plot().y + scene.primary_plot().height,
                    );
                    labels.push((
                        scene.primary.panel_index,
                        scene.format_primary_axis_label(value, step_value),
                        y,
                    ));
                }
            }
        } else {
            let (min_display, max_display) = scene.primary_domain_display_values();
            let scale = if matches!(scene.primary.mode, PanelScaleMode::PercentFromBase) {
                YAxisScale::Percent
            } else {
                YAxisScale::Linear
            };
            let y_ticks = scale.ticks(
                f64::from(min_display),
                f64::from(max_display),
                labels_for_height(scene.primary_plot().height),
            );
            let step = y_ticks.step.unwrap_or(1.0) as f32;

            for tick in y_ticks.ticks {
                let ratio = tick.ratio as f32;
                let TickValue::Float(tick) = tick.value else {
                    continue;
                };
                let y = scene.primary_plot().y + (1.0 - ratio) * scene.primary_plot().height;
                labels.push((
                    scene.primary.panel_index,
                    scene.format_primary_axis_label(tick as f32, step),
                    y,
                ));
            }
        }

        for indicator in &scene.indicator_panels {
            if self.indicator_is_unavailable(indicator.value_id) {
                continue;
            }
            let panel_index = indicator.panel_index;
            if matches!(indicator.presentation, PanelPresentation::Table) {
                let Some(value_id) = indicator.value_id else {
                    continue;
                };
                let Some(panel) = scene.layout.panel(panel_index) else {
                    continue;
                };
                let Some(kind) = self.indicator_kind(value_id) else {
                    continue;
                };
                let columns = table_columns(kind);
                let row_height = (panel.plot.height / columns.len().max(1) as f32).max(1.0);
                for (row, column) in columns.iter().enumerate() {
                    labels.push((
                        panel_index,
                        column.label.to_string(),
                        panel.plot.y + row as f32 * row_height + row_height * 0.5,
                    ));
                }
                continue;
            }

            let Some(panel) = scene.layout.panel(panel_index) else {
                continue;
            };
            let panel_precision = self.panel_value_precision(panel_index);
            let min_value = indicator.min_value() as f64;
            let max_value = indicator.max_value() as f64;
            let y_ticks = YAxisScale::Linear.ticks(
                min_value,
                max_value,
                labels_for_height(panel.plot.height),
            );
            let step_value = y_ticks.step.unwrap_or(1.0) as f32;

            for tick in y_ticks.ticks {
                let TickValue::Float(value) = tick.value else {
                    continue;
                };
                let value = value as f32;
                let Some(y) = scene.map_indicator_plot(panel_index, value) else {
                    continue;
                };
                let y = y.clamp(panel.plot.y, panel.plot.y + panel.plot.height);
                labels.push((
                    panel_index,
                    self.format_panel_axis_value(panel_index, panel_precision, value, step_value),
                    y,
                ));
            }
        }

        labels
    }

    pub(super) fn required_y_axis_gutter(&self, scene: &Scene) -> f32 {
        let label_width = |label: &str| label.chars().count() as f32 * CHAR_W;
        let mut widest_label = self
            .y_axis_label_specs(scene)
            .into_iter()
            .map(|(_, label, _)| label_width(&label))
            .fold(0.0, f32::max);

        for value in [scene.primary.domain.values.0, scene.primary.domain.values.1] {
            widest_label = widest_label.max(label_width(&scene.format_primary_cursor_label(value)));
        }

        for indicator in &scene.indicator_panels {
            if self.indicator_is_unavailable(indicator.value_id) {
                continue;
            }
            if matches!(indicator.presentation, PanelPresentation::Table) {
                continue;
            }

            let panel_precision = self.panel_value_precision(indicator.panel_index);
            for value in [indicator.min_value(), indicator.max_value()] {
                widest_label = widest_label.max(label_width(&self.format_panel_axis_value(
                    indicator.panel_index,
                    panel_precision,
                    value,
                    0.01,
                )));
            }
        }

        (widest_label + Y_AXIS_LABEL_LEFT_INSET * 2.0).max(Y_AXIS_GUTTER)
    }

    pub(super) fn fill_y_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        for (panel_index, text, y) in self.y_axis_label_specs(scene) {
            let Some(axis) = scene.layout.panel(panel_index) else {
                continue;
            };

            self.draw_y_axis_label(frame, axis, text, y, palette.background.base.text);
        }
    }

    pub(super) fn fill_unavailable_indicator_messages(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        for indicator in &scene.indicator_panels {
            if !self.indicator_is_unavailable(indicator.value_id) {
                continue;
            }

            let Some(panel) = scene.layout.panel(indicator.panel_index) else {
                continue;
            };
            let Some(content) = self
                .indicator_unavailable_message(indicator.value_id)
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

    pub(super) fn draw_y_axis_label(
        &self,
        frame: &mut canvas::Frame,
        axis: &PanelLayoutNode,
        content: String,
        y: f32,
        color: iced::Color,
    ) {
        let y = axis.y_axis_label_y(y, TEXT_SIZE);
        frame.with_clip(axis.y_axis, |frame| {
            frame.fill_text(canvas::Text {
                content,
                position: Point::new(Y_AXIS_LABEL_LEFT_INSET, y),
                color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        });
    }

    pub(super) fn fill_x_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let pixel_ratio = self.resolved_pixel_ratio();
        let plot_width = scene.x_axis_plot_width();

        if let XAxis::Time { timeframe, .. } = scene.x.axis
            && let Some((earliest, latest, earliest_unit, latest_unit)) = (|| {
                let spacing = scene.logical_bar_spacing().max(1.0);
                let right_unit = scene.effective_right_x_unit();
                let visible_units = plot_width / spacing;
                let earliest_unit = (right_unit - visible_units).floor() as i64;
                let latest_unit = right_unit.ceil() as i64;

                Some((
                    scene.time_for_x_unit(earliest_unit)?,
                    scene.time_for_x_unit(latest_unit)?,
                    earliest_unit,
                    latest_unit,
                ))
            })()
        {
            let span_ms = latest
                .as_u64()
                .saturating_sub(earliest.as_u64())
                .max(timeframe.to_milliseconds());
            let labels = TimeTickLabel::for_range(
                earliest,
                latest,
                span_ms,
                plot_width,
                CHAR_W,
                x_labels_that_fit(plot_width, CHAR_W) as i32,
                timeframe,
                self.timezone,
            );

            let clip_region = Rectangle {
                x: 0.0,
                y: 0.0,
                width: plot_width,
                height: scene.layout.regions.x_axis.height,
            };

            frame.with_clip(clip_region, |frame| {
                for label in labels {
                    let color = match label.tier {
                        TimeTickTier::Secondary => palette.background.strongest.text,
                        TimeTickTier::Main => palette.background.base.text,
                    };
                    let tick_unit = earliest_unit as f32
                        + (label.x_pos / plot_width) * (latest_unit - earliest_unit) as f32;
                    let x = Self::snap_plot_x_to_cell(scene.map_x_plot_f32(tick_unit), pixel_ratio);

                    frame.fill_text(canvas::Text {
                        content: label.content,
                        position: Point::new(x, scene.layout.regions.x_axis.height / 2.0),
                        color,
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Center.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            });
            return;
        }

        if matches!(scene.x.axis, XAxis::Tick { .. }) {
            let target_spacing = data::chart::ticks::x_label_spacing_px(TEXT_SIZE);
            let clip_region = Rectangle {
                x: 0.0,
                y: 0.0,
                width: plot_width,
                height: scene.layout.regions.x_axis.height,
            };

            frame.with_clip(clip_region, |frame| {
                let mut last_x: Option<f32> = None;

                if let Some(base_series) = self.series.first() {
                    for index in 0..base_series.bars().len() {
                        let Some(unit) = self.tick_unit_for_index(scene.x.axis, index) else {
                            continue;
                        };
                        let x = Self::snap_plot_x_to_cell(scene.map_x_plot(unit), pixel_ratio);

                        if last_x.is_some_and(|previous| (x - previous).abs() < target_spacing) {
                            continue;
                        }

                        let Some(time) = self.tick_time_for_unit(scene.x.axis, unit) else {
                            continue;
                        };
                        let Some(content) = self.timezone.format_with_kind(
                            time.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Axis {
                                timeframe: Timeframe::MS100,
                            },
                        ) else {
                            continue;
                        };

                        frame.fill_text(canvas::Text {
                            content,
                            position: Point::new(x, scene.layout.regions.x_axis.height / 2.0),
                            color: palette.background.base.text,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                        last_x = Some(x);
                    }
                }
            });
            return;
        }

        let (ticks, step_units) = unit_ticks(
            scene.x.min_unit,
            scene.x.max_unit,
            plot_width,
            MIN_X_TICK_PX.max(40.0),
        );

        let mut ticks_with_bounds = Vec::with_capacity(ticks.len().saturating_add(2));

        if let Some(first) = ticks.first().copied() {
            ticks_with_bounds.push(first.saturating_sub(step_units));
        }

        ticks_with_bounds.extend(ticks);

        if let Some(last) = ticks_with_bounds.last().copied() {
            ticks_with_bounds.push(last.saturating_add(step_units));
        }

        let clip_region = Rectangle {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: scene.layout.regions.x_axis.height,
        };

        frame.with_clip(clip_region, |frame| {
            for tick in ticks_with_bounds {
                let x = Self::snap_plot_x_to_cell(scene.map_x_plot(tick), pixel_ratio);
                let label = self.format_x_label(scene.x.axis, tick, step_units);

                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(x, scene.layout.regions.x_axis.height / 2.0),
                    color: palette.background.base.text,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        });
    }
}
