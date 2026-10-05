use super::marks::{MarkRenderer, candlestick_widths};
use super::{CanvasLayer, PaintInputs, PixelGeometry};
use crate::widget::chart::kline::{
    composition::{LayerDataKind, MarkKind, PanelPresentation},
    footprint::FootprintRenderer,
    interaction::LayerContext,
    presentation::IndicatorView,
    scene::{ChartCoordinates, Scene},
    studies::{LayerPaintPhase, StudyObjects, StudyStates},
};
use data::config::theme::ACCENT_COUNT;
use iced::keyboard;
use iced::theme::palette::Extended;
use rustc_hash::FxHashSet;

const FOOTPRINT_MIN_COLUMN_WIDTH_PX: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryPlotDetail {
    CandleOnly,
    Footprint,
}

pub(crate) struct PlotRenderer<'a> {
    pub(crate) coordinates: ChartCoordinates<'a>,
    pub(crate) indicators: IndicatorView<'a>,
    pub(crate) footprints: FootprintRenderer<'a>,
    pub(crate) study_objects: &'a StudyObjects<'a>,
    pub(crate) trade_data_enabled: bool,
}

impl PlotRenderer<'_> {
    pub(crate) fn paint(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        layers: &StudyStates,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
        layer: CanvasLayer,
    ) {
        let context = LayerContext {
            scene,
            coordinates: self.coordinates,
            position: None,
            modifiers: keyboard::Modifiers::empty(),
        };
        let marks = MarkRenderer {
            indicators: self.indicators,
            scene,
            palette,
            generated_accents,
            pixels,
        };
        let primary_plot_detail = {
            let footprint_detail_enabled = self.trade_data_enabled
                && scene.logical_bar_spacing() >= FOOTPRINT_MIN_COLUMN_WIDTH_PX;

            if footprint_detail_enabled {
                PrimaryPlotDetail::Footprint
            } else {
                PrimaryPlotDetail::CandleOnly
            }
        };
        let (candle_body_width_phys, wick_width_phys) =
            candlestick_widths(scene.logical_bar_spacing(), pixels);
        let indicator_width_phys = candle_body_width_phys;

        if layer == CanvasLayer::Background {
            for indicator_panel in &scene.indicator_panels {
                if matches!(indicator_panel.presentation, PanelPresentation::Table) {
                    marks.fill_table_panel(frame, indicator_panel);
                } else {
                    marks.fill_indicator_panel_background(frame, indicator_panel);
                }
            }
            marks.fill_primary_overlay_backgrounds(frame);

            for indicator_panel in &scene.indicator_panels {
                if self
                    .indicators
                    .panel_indicator_value_ids(indicator_panel.panel_index)
                    .iter()
                    .all(|value_id| self.indicators.indicator_is_unavailable(Some(*value_id)))
                {
                    continue;
                }

                let Some(panel_plot) = scene
                    .layout
                    .panel(indicator_panel.panel_index)
                    .map(|panel| panel.plot)
                else {
                    continue;
                };

                let structural_layers = self
                    .indicators
                    .panel_output_layers(indicator_panel.panel_index)
                    .into_iter()
                    .filter(|layer| layer.visible && layer.purpose.is_plot())
                    .collect::<Vec<_>>();
                frame.with_clip(panel_plot, |frame| {
                    for layer in &structural_layers {
                        let output = &layer.presentation_output;
                        let Some(spec) = self.indicators.indicator_output_spec(output) else {
                            continue;
                        };
                        match spec.value_type {
                            data::chart::kline::OutputValueType::Ohlc => {
                                marks.draw_indicator_ohlc_output(
                                    frame,
                                    output.instance_id,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                                    },
                                    indicator_width_phys,
                                    wick_width_phys,
                                );
                            }
                            data::chart::kline::OutputValueType::Range => {
                                marks.draw_indicator_range_output(
                                    frame,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                                    },
                                );
                            }
                            data::chart::kline::OutputValueType::Number
                            | data::chart::kline::OutputValueType::Boolean => {}
                        }
                    }
                    for value_id in self
                        .indicators
                        .panel_indicator_value_ids(indicator_panel.panel_index)
                    {
                        if !self
                            .indicators
                            .panel_indicator_visible(indicator_panel.panel_index, value_id)
                        {
                            continue;
                        }
                        marks.draw_indicator_markers_and_levels(frame, value_id, |value| {
                            scene.map_indicator_plot(indicator_panel.panel_index, value)
                        });
                    }
                });

                let output_layers = self
                    .indicators
                    .panel_output_layers(indicator_panel.panel_index)
                    .into_iter()
                    .filter(|layer| {
                        layer.visible
                            && layer.purpose.is_plot()
                            && matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_))
                            && layer.data_kind != LayerDataKind::Ohlc
                            && self
                                .indicators
                                .indicator_output_spec(&layer.presentation_output)
                                .is_some_and(|spec| {
                                    spec.value_type == data::chart::kline::OutputValueType::Number
                                })
                    })
                    .collect::<Vec<_>>();
                if output_layers.is_empty() {
                    continue;
                }

                frame.with_clip(panel_plot, |frame| {
                    marks.draw_indicator_bars(
                        frame,
                        indicator_panel.panel_index,
                        &output_layers,
                        indicator_width_phys,
                    );
                });
            }

            if let Some(primary_panel_id) = self.coordinates.composition.primary_panel_id()
                && let Some(primary_panel) = self.coordinates.composition.panel(primary_panel_id)
            {
                let base_series_anchor = scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten();
                let primary_plot = *scene.primary_plot();

                let region_ids = self
                    .indicators
                    .primary_overlay_entries()
                    .into_iter()
                    .filter_map(|(value_id, visible)| visible.then_some(value_id))
                    .collect::<Vec<_>>();
                frame.with_clip(primary_plot, |frame| {
                    for value_id in region_ids {
                        marks.draw_indicator_regions(frame, value_id, |value| {
                            scene.map_primary_plot_with_anchor(value, base_series_anchor)
                        });
                    }
                });

                self.footprints.fill_footprint_poc_overlay(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    pixels,
                );
                self.study_objects.paint(
                    LayerPaintPhase::BeforeIndicators,
                    context,
                    layers,
                    frame,
                    palette,
                    generated_accents,
                    pixels,
                );

                let structural_layers = primary_panel
                    .layers
                    .iter()
                    .filter_map(|layer| {
                        let output = layer.source.indicator_output()?;
                        let resolved = self
                            .indicators
                            .resolved_indicator_layer_for_composition_layer(layer)?;
                        (layer.visible
                            && resolved.purpose.is_plot()
                            && !self
                                .indicators
                                .indicator_is_unavailable(Some(output.instance_id)))
                        .then_some(resolved)
                    })
                    .collect::<Vec<_>>();
                frame.with_clip(primary_plot, |frame| {
                    let mut annotation_ids = FxHashSet::default();
                    for layer in &structural_layers {
                        let output = &layer.presentation_output;
                        let Some(spec) = self.indicators.indicator_output_spec(output) else {
                            continue;
                        };
                        match spec.value_type {
                            data::chart::kline::OutputValueType::Ohlc => {
                                marks.draw_indicator_ohlc_output(
                                    frame,
                                    output.instance_id,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene
                                            .map_primary_plot_with_anchor(value, base_series_anchor)
                                    },
                                    indicator_width_phys,
                                    wick_width_phys,
                                );
                            }
                            data::chart::kline::OutputValueType::Range => {
                                marks.draw_indicator_range_output(
                                    frame,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene
                                            .map_primary_plot_with_anchor(value, base_series_anchor)
                                    },
                                );
                            }
                            data::chart::kline::OutputValueType::Number
                            | data::chart::kline::OutputValueType::Boolean => {}
                        }
                        if annotation_ids.insert(output.instance_id) {
                            marks.draw_indicator_markers_and_levels(
                                frame,
                                output.instance_id,
                                |value| {
                                    scene.map_primary_plot_with_anchor(value, base_series_anchor)
                                },
                            );
                        }
                    }
                });

                frame.with_clip(primary_plot, |frame| {
                    for layer in &primary_panel.layers {
                        if !layer.visible {
                            continue;
                        }

                        let Some(resolved_layer) = self
                            .indicators
                            .resolved_indicator_layer_for_composition_layer(layer)
                        else {
                            continue;
                        };
                        if !resolved_layer.purpose.is_plot() {
                            continue;
                        }
                        let value_id = resolved_layer.base_output.instance_id;
                        if self.indicators.indicator_is_unavailable(Some(value_id)) {
                            continue;
                        }

                        let output = &resolved_layer.presentation_output;
                        if self
                            .indicators
                            .indicator_output_spec(output)
                            .is_some_and(|spec| {
                                spec.value_type != data::chart::kline::OutputValueType::Number
                            })
                        {
                            // Boolean channels are event/table values. They do
                            // not have a numeric y-coordinate until a compatible
                            // mark is introduced.
                            continue;
                        }
                        let mark = resolved_layer.mark;
                        for channel in self
                            .indicators
                            .overlay_channels_for_layer(Some(output))
                            .into_iter()
                        {
                            if matches!(mark, MarkKind::Point | MarkKind::Cross) {
                                marks.draw_points(
                                    frame,
                                    value_id,
                                    output,
                                    channel,
                                    mark,
                                    resolved_layer.required_input,
                                    |value| {
                                        scene
                                            .map_primary_plot_with_anchor(value, base_series_anchor)
                                    },
                                );
                                continue;
                            }
                            let source_channel = channel.source;
                            let has_dynamic_style = channel.style_by.is_some()
                                || self
                                    .indicators
                                    .indicator_output_spec_for_channel(value_id, source_channel)
                                    .is_some_and(|spec| spec.style_by.is_some());
                            let static_color = (!has_dynamic_style).then(|| {
                                self.indicators.overlay_channel_color_for_output(
                                    Some(value_id),
                                    Some(output),
                                    &channel,
                                    palette,
                                    generated_accents,
                                )
                            });
                            marks.draw_line(
                                frame,
                                value_id,
                                output,
                                &channel,
                                mark,
                                resolved_layer.required_input,
                                primary_plot,
                                |value| {
                                    scene.map_primary_plot_with_anchor(value, base_series_anchor)
                                },
                                scene.map_primary_plot_with_anchor_f32(0.0, base_series_anchor),
                                |series, index| {
                                    if has_dynamic_style {
                                        self.indicators.overlay_channel_color_for_channel_at(
                                            series,
                                            index,
                                            value_id,
                                            value_id,
                                            source_channel,
                                            &channel,
                                            palette,
                                            generated_accents,
                                        )
                                    } else {
                                        static_color.expect("static overlay color is precomputed")
                                    }
                                },
                            );
                        }
                    }
                });
            }
        }

        if layer == CanvasLayer::Background {
            return;
        }
        if layer == CanvasLayer::StudyUnderlay {
            self.study_objects.paint(
                LayerPaintPhase::Underlay,
                context,
                layers,
                frame,
                palette,
                generated_accents,
                pixels,
            );
            return;
        }

        if layer == CanvasLayer::PriceBars
            && self.footprints.candlestick.visible
            && matches!(primary_plot_detail, PrimaryPlotDetail::CandleOnly)
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_))
        {
            let primary_plot = *scene.primary_plot();
            frame.with_clip(primary_plot, |frame| {
                let anchor = scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten();
                marks.draw_candles(
                    frame,
                    |value| scene.map_primary_plot_with_anchor_f32(value, anchor),
                    |bullish| {
                        self.footprints
                            .candlestick_color(bullish, palette, generated_accents)
                    },
                );
            });
        }

        if layer == CanvasLayer::PriceBars
            && matches!(primary_plot_detail, PrimaryPlotDetail::Footprint)
        {
            self.footprints.fill_footprint_overlay(
                frame,
                scene,
                palette,
                generated_accents,
                self.footprints.footprint.visible,
                pixels,
            );
        }

        if layer == CanvasLayer::PriceBars {
            return;
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            for series_index in 0..self.coordinates.series.len() {
                if !self.coordinates.series_participates_in_primary_plot(
                    self.coordinates.series[series_index].ticker_info,
                ) {
                    continue;
                }
                marks.draw_series_line(frame, series_index);
            }
        });

        for indicator_panel in &scene.indicator_panels {
            if self
                .indicators
                .panel_indicator_value_ids(indicator_panel.panel_index)
                .iter()
                .all(|value_id| self.indicators.indicator_is_unavailable(Some(*value_id)))
            {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            let output_layers = self
                .indicators
                .panel_output_layers(indicator_panel.panel_index)
                .into_iter()
                .filter(|layer| {
                    layer.visible
                        && layer.purpose.is_plot()
                        && layer.mark.is_series_line_like()
                        && self
                            .indicators
                            .indicator_output_spec(&layer.presentation_output)
                            .is_some_and(|spec| {
                                spec.value_type == data::chart::kline::OutputValueType::Number
                            })
                })
                .collect::<Vec<_>>();
            if output_layers.is_empty() {
                continue;
            }

            frame.with_clip(panel_plot, |frame| {
                for layer in &output_layers {
                    let output = &layer.presentation_output;
                    let Some(channel) = self.indicators.overlay_channels_for_layer(Some(output))
                    else {
                        continue;
                    };
                    marks.draw_line(
                        frame,
                        output.instance_id,
                        output,
                        &channel,
                        layer.mark,
                        layer.required_input,
                        panel_plot,
                        |value| scene.map_indicator_plot(indicator_panel.panel_index, value),
                        scene
                            .map_indicator_plot(indicator_panel.panel_index, 0.0)
                            .unwrap_or(panel_plot.y + panel_plot.height),
                        |series, index| {
                            self.indicators.overlay_channel_color_for_output_at(
                                series,
                                index,
                                Some(output.instance_id),
                                Some(output),
                                &channel,
                                palette,
                                generated_accents,
                            )
                        },
                    );
                }
            });
        }

        self.study_objects.paint(
            LayerPaintPhase::AfterIndicators,
            context,
            layers,
            frame,
            palette,
            generated_accents,
            pixels,
        );
    }
}
