use super::PreparedChrome;
use super::{
    HIDDEN_LEGEND_ALPHA, STUDY_COUNT_BADGE_GAP, STUDY_COUNT_BADGE_HEIGHT,
    STUDY_COUNT_BADGE_HORIZONTAL_PAD, STUDY_LEGEND_INDENT, TICKER_LEGEND_ICON_BOX,
    TICKER_LEGEND_ICON_GAP, TICKER_LEGEND_TOP_OFFSET, legend_background_color,
    legend_row_background, series_display_name,
};
use super::{PANEL_TITLE_LEFT_PAD, PANEL_TITLE_TO_CONTROLS_GAP};
use crate::style;
use crate::widget::chart::kline::composition;
use crate::widget::chart::kline::composition::{MarkKind, PanelScaleMode, PanelValueId};
use crate::widget::chart::kline::layout::{KlinePanelKind, PanelLayoutTree};
use crate::widget::chart::kline::painting;
use crate::widget::chart::kline::presentation::IndicatorView;
use crate::widget::chart::kline::scene::{ChartCoordinates, CursorTarget, LegendTarget, Scene};
use crate::widget::chart::kline::{
    CHAR_W, KlineWidgetEvent, PANEL_TITLE_TOP_PAD, TEXT_SIZE, TICKER_LEGEND_PADDING,
    TICKER_LEGEND_ROW_H, TOOLTIP_ROW_LEFT_PAD,
};
use crate::widget::chart::{LEGEND_LABEL_VALUE_GAP, LEGEND_READOUT_GAP};
use data::chart::kline::{
    CandlestickConfig, FootprintConfig, KlineStudyId, LegendVisibility, footprint::ClusterKind,
};
use data::config::theme::{ACCENT_COUNT, ThemeColorRole};
use exchange::TickerInfo;
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle};

pub struct LegendView<'a> {
    pub coordinates: ChartCoordinates<'a>,
    pub indicators: IndicatorView<'a>,
    pub candlestick: &'a CandlestickConfig,
    pub footprint: &'a FootprintConfig,
    pub study_legend: Option<&'a StudiesLegend>,
    pub legend_visibility: LegendVisibility,
}

struct PreparedIndicatorReadouts {
    panel_index: usize,
    value_id: PanelValueId,
    text: Vec<canvas::Text>,
    end_x: f32,
}

impl PreparedIndicatorReadouts {
    fn push_text(&mut self, content: String, position: Point, color: iced::Color) -> f32 {
        let end_x = position.x + content.chars().count() as f32 * CHAR_W;
        self.text.push(canvas::Text {
            content,
            position,
            color,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
        end_x
    }

    fn paint(&mut self, frame: &mut PreparedChrome) {
        for text in self.text.drain(..) {
            frame.fill_text(text);
        }
    }
}

impl<'a> LegendView<'a> {
    pub fn indicator_unavailable_message(&self, value_id: Option<PanelValueId>) -> Option<&'a str> {
        let value_id = value_id?;
        self.indicators.presentation.unavailable_message(value_id)
    }

    pub fn indicator_marker(&self, value_id: PanelValueId) -> Option<(&'static str, bool)> {
        if self.indicators.presentation.is_loading(value_id) {
            return Some(("...", true));
        }
        self.indicator_unavailable_message(Some(value_id))
            .map(|_| ("!", false))
    }

    pub fn indicator_label_x(&self, row_x: f32, value_id: PanelValueId) -> f32 {
        let marker_width = self.indicator_marker(value_id).map_or(0.0, |(marker, _)| {
            marker.chars().count() as f32 * CHAR_W + TICKER_LEGEND_PADDING
        });
        row_x + TOOLTIP_ROW_LEFT_PAD + marker_width
    }

    pub fn build_ticker_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        show_values: bool,
        show_icons: bool,
    ) -> Option<TickerLegendLayout> {
        if self.coordinates.series.is_empty() {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let plot = panel.plot;

        let rows_count = self.coordinates.series.len();

        let mut max_chars: usize = 0;
        let mut max_name_chars: usize = 0;

        for (index, series) in self.coordinates.series.iter().enumerate() {
            let name_chars = series_display_name(series).chars().count();
            max_name_chars = max_name_chars.max(name_chars);

            let value_chars = if show_values && index == 0 {
                series
                    .latest_point()
                    .map(|point| {
                        let bar = point.kline;
                        let precision = series.ticker_info.min_ticksize;
                        format!(
                            "O {} H {} L {} C {} +00.00%",
                            bar.open.to_string(precision),
                            bar.high.to_string(precision),
                            bar.low.to_string(precision),
                            bar.close.to_string(precision),
                        )
                        .chars()
                        .count()
                    })
                    .unwrap_or(0)
            } else if show_values {
                "+100.00%".chars().count()
            } else {
                0
            };

            let total_chars = if value_chars > 0 {
                name_chars + 1 + value_chars
            } else {
                name_chars
            };

            max_chars = max_chars.max(total_chars);
        }

        let text_w = max_chars as f32 * CHAR_W;
        let icon_count = if self.coordinates.series.len() > 1 {
            3.0
        } else {
            2.0
        };
        let icons_pack_w =
            icon_count * TICKER_LEGEND_ICON_BOX + (icon_count - 1.0) * TICKER_LEGEND_ICON_GAP;
        let min_for_icons = if show_icons {
            max_name_chars as f32 * CHAR_W + 8.0 + icons_pack_w
        } else {
            0.0
        };
        let bg_w = (text_w.max(min_for_icons) + TICKER_LEGEND_PADDING * 2.0)
            .clamp(80.0, (plot.width * 0.6).max(80.0));

        let max_bg_h = ((rows_count as f32) * TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0)
            .min(plot.height * 0.5)
            .max(TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0);
        let max_rows_fit =
            (((max_bg_h - TICKER_LEGEND_PADDING * 2.0) / TICKER_LEGEND_ROW_H).floor() as usize)
                .max(1);
        let visible_rows = rows_count.min(max_rows_fit);
        let bg_h = visible_rows as f32 * TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0;

        let bg = Rectangle {
            x: plot.x + TICKER_LEGEND_PADDING,
            y: plot.y + PANEL_TITLE_TOP_PAD + TICKER_LEGEND_TOP_OFFSET,
            width: bg_w,
            height: bg_h,
        };

        let x_left = bg.x + TICKER_LEGEND_PADDING;

        let mut rows = Vec::with_capacity(visible_rows);
        let mut row_top = bg.y + TICKER_LEGEND_PADDING;

        for (index, series) in self
            .coordinates
            .series
            .iter()
            .take(visible_rows)
            .enumerate()
        {
            let has_close = index != 0;
            let label_chars = series_display_name(series).chars().count() as f32;

            let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
            let text_end_x = x_left + label_chars * CHAR_W;

            let (_, mut visibility, mut settings, mut close, row_w) =
                legend_controls(bg, text_end_x, y_center, false, true, true, has_close);
            if !show_icons {
                visibility = Rectangle::default();
                settings = Rectangle::default();
                close = Rectangle::default();
            }

            rows.push(TickerLegendRowHit {
                ticker: series.ticker_info,
                y_center,
                row_rect: Rectangle {
                    x: bg.x,
                    y: row_top,
                    width: row_w,
                    height: TICKER_LEGEND_ROW_H,
                },
                settings,
                visibility,
                close,
                has_close,
            });

            row_top += TICKER_LEGEND_ROW_H;
        }

        Some(TickerLegendLayout { bg, rows })
    }

    pub fn build_study_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
    ) -> Option<StudyLegendLayout> {
        let legend = self.study_legend?;
        let study_count = legend.rows.len();
        if study_count == 0 {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let category_row_top = panel.plot.y
            + PANEL_TITLE_TOP_PAD
            + TICKER_LEGEND_PADDING
            + preceding_rows as f32 * TICKER_LEGEND_ROW_H;
        let category_y_center = category_row_top + TICKER_LEGEND_ROW_H * 0.5;
        let category_label = "Studies";
        let category_label_end_x =
            panel.plot.x + TOOLTIP_ROW_LEFT_PAD + category_label.chars().count() as f32 * CHAR_W;
        let count_badge_width = study_count.to_string().chars().count() as f32 * CHAR_W
            + STUDY_COUNT_BADGE_HORIZONTAL_PAD * 2.0;
        let count_badge = Rectangle {
            x: category_label_end_x + STUDY_COUNT_BADGE_GAP,
            y: category_y_center - STUDY_COUNT_BADGE_HEIGHT * 0.5,
            width: count_badge_width,
            height: STUDY_COUNT_BADGE_HEIGHT,
        };
        let category_text_end_x = count_badge.x + count_badge.width;
        let (category_expand, category_visibility, _, category_close, category_row_w) =
            legend_controls(
                panel.plot,
                category_text_end_x,
                category_y_center,
                true,
                true,
                false,
                true,
            );
        let category = StudiesLegendRowHit {
            row_rect: Rectangle {
                x: panel.plot.x,
                y: category_row_top,
                width: category_row_w,
                height: TICKER_LEGEND_ROW_H,
            },
            expand: category_expand,
            visibility: category_visibility,
            close: category_close,
            count_badge,
            expanded: legend.expanded,
            visible: legend.visible(),
        };
        let mut rows = Vec::new();

        if legend.expanded {
            for (index, study) in legend.rows.iter().enumerate() {
                let label = &study.label;
                let row_top = panel.plot.y
                    + PANEL_TITLE_TOP_PAD
                    + TICKER_LEGEND_PADDING
                    + (preceding_rows + 1 + index) as f32 * TICKER_LEGEND_ROW_H;
                let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
                let row_x = panel.plot.x + STUDY_LEGEND_INDENT;
                let text_end_x =
                    row_x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
                rows.push(StudyLegendRowHit {
                    controls: LegendRowControls::new(panel.plot, row_x, text_end_x, y_center),
                    visible: study.visible,
                });
            }
        }

        Some(StudyLegendLayout {
            category,
            study_count,
            rows,
        })
    }

    fn indicator_title(
        &self,
        panel_index: usize,
        primary_overlay: bool,
        value_id: PanelValueId,
    ) -> String {
        if primary_overlay || self.indicators.panel_indicator_value_ids(panel_index).len() > 1 {
            return self
                .indicators
                .indicator_kind(value_id)
                .map(|indicator| indicator.to_string())
                .unwrap_or_else(|| "Indicator".to_string());
        }

        self.coordinates
            .resolved_panel_title(panel_index, KlinePanelKind::Indicator)
            .unwrap_or("Indicator")
            .to_string()
    }

    fn readout_bar_index(&self, scene: &Scene) -> Option<usize> {
        let series = self.coordinates.series.first()?;
        scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene.interaction.cursor.and_then(|cursor| {
                    self.coordinates
                        .bar_index_at_or_before_unit(scene, 0, cursor.x_unit)
                })
            })
            .flatten()
            .or_else(|| {
                self.legend_visibility
                    .legend_values_always_visible()
                    .then(|| series.point_count().checked_sub(1))
                    .flatten()
            })
    }

    fn primary_overlay_value_text(
        &self,
        scene: &Scene,
        value_id: PanelValueId,
        channel_id: &str,
        bar_index: usize,
    ) -> Option<String> {
        if self.indicators.indicator_is_unavailable(Some(value_id)) {
            return None;
        }
        let series = self.coordinates.series.first()?;
        let value = self.indicators.indicator_source_value_for_scene_at(
            scene, 0, series, value_id, bar_index, channel_id,
        )?;
        let primary_anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();

        if matches!(scene.primary.mode, PanelScaleMode::PercentFromBase)
            && primary_anchor
                .map(|anchor| anchor.abs() > f32::EPSILON)
                .unwrap_or(false)
        {
            let anchor = f64::from(primary_anchor.unwrap_or(1.0));
            Some(format!("{:+.2}%", ((value / anchor) - 1.0) * 100.0))
        } else {
            Some(
                self.coordinates
                    .format_panel_header_value(scene.primary.panel_index, value),
            )
        }
    }

    fn prepare_indicator_readouts(
        &self,
        scene: &Scene,
        bar_index: Option<usize>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> Vec<PreparedIndicatorReadouts> {
        let values_visible = scene.interaction.readouts_visible()
            || self.legend_visibility.legend_values_always_visible();
        let hovered_indicator_id = scene.interaction.target.indicator_row().and_then(|index| {
            scene
                .chrome
                .indicator_legend
                .as_ref()?
                .rows
                .get(index)
                .map(|row| row.value_id)
        });
        let mut prepared = Vec::new();
        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            if panel.kind != KlinePanelKind::Indicator {
                continue;
            }
            for (indicator_index, value_id) in self
                .indicators
                .panel_indicator_value_ids(panel_index)
                .into_iter()
                .enumerate()
            {
                prepared.push(self.prepare_panel_readouts(
                    scene,
                    panel_index,
                    indicator_index,
                    value_id,
                    values_visible && Some(value_id) != hovered_indicator_id,
                    bar_index,
                    palette,
                    generated_accents,
                ));
            }
        }
        if let Some(legend) = scene.chrome.indicator_legend.as_ref() {
            for (index, row) in legend.rows.iter().enumerate() {
                if row.primary_overlay {
                    prepared.push(self.prepare_overlay_readouts(
                        scene,
                        row,
                        values_visible && !scene.interaction.target.indicator_row_is_hovered(index),
                        bar_index,
                        palette,
                        generated_accents,
                    ));
                }
            }
        }
        prepared
    }

    fn prepare_overlay_readouts(
        &self,
        scene: &Scene,
        row: &IndicatorLegendRowHit,
        show_values: bool,
        bar_index: Option<usize>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> PreparedIndicatorReadouts {
        let panel_index = scene.primary.panel_index;
        let row_x = scene.layout.regions.plot.x + row.controls.row_rect.x;
        let title = self.indicator_title(panel_index, true, row.value_id);
        let title_end_x =
            self.indicator_label_x(row_x, row.value_id) + title.chars().count() as f32 * CHAR_W;
        let mut prepared = PreparedIndicatorReadouts {
            panel_index,
            value_id: row.value_id,
            text: Vec::new(),
            end_x: title_end_x,
        };
        if !show_values {
            return prepared;
        }
        let mut x = title_end_x + PANEL_TITLE_TO_CONTROLS_GAP;
        let y = scene.layout.regions.plot.y + row.y_center;
        let row_alpha = if row.visible {
            1.0
        } else {
            HIDDEN_LEGEND_ALPHA
        };
        for channel in self
            .indicators
            .overlay_channels_for_panel_value(Some(row.value_id))
        {
            let color = self
                .indicators
                .overlay_channel_color(Some(row.value_id), &channel, palette, generated_accents)
                .scale_alpha(row_alpha);
            prepared.end_x = prepared.push_text(channel.label.to_owned(), Point::new(x, y), color);
            x = prepared.end_x + LEGEND_LABEL_VALUE_GAP;
            let Some(value_text) = bar_index.and_then(|bar_index| {
                self.primary_overlay_value_text(scene, row.value_id, channel.source, bar_index)
            }) else {
                continue;
            };
            prepared.end_x = prepared.push_text(value_text, Point::new(x, y), color);
            x = prepared.end_x + LEGEND_READOUT_GAP;
        }
        prepared
    }

    fn panel_readout_max_x(&self, scene: &Scene, panel_index: usize) -> f32 {
        let panel = &scene.layout.panels[panel_index];
        let mut max_x =
            scene.layout.regions.plot.x + panel.plot.x + panel.plot.width - PANEL_TITLE_LEFT_PAD;
        if scene.interaction.controls_visible_for_panel == Some(panel_index)
            && let Some(left_control_x) = scene
                .chrome
                .panel_controls
                .iter()
                .filter(|control| control.panel_index == panel_index)
                .map(|control| control.rect.x)
                .reduce(f32::min)
        {
            max_x = max_x.min(scene.layout.regions.plot.x + left_control_x - 4.0);
        }
        max_x
    }

    fn prepare_panel_readouts(
        &self,
        scene: &Scene,
        panel_index: usize,
        indicator_index: usize,
        value_id: PanelValueId,
        include_values: bool,
        bar_index: Option<usize>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> PreparedIndicatorReadouts {
        let panel = &scene.layout.panels[panel_index];
        let label = self.indicator_title(panel_index, false, value_id);
        let title_width = label.chars().count() as f32 * CHAR_W;
        let label_end_x = self
            .indicator_label_x(scene.layout.regions.plot.x + panel.plot.x, value_id)
            + title_width;
        let mut prepared = PreparedIndicatorReadouts {
            panel_index,
            value_id,
            text: Vec::new(),
            end_x: label_end_x,
        };
        if !include_values
            || !self
                .indicators
                .panel_indicator_visible(panel_index, value_id)
            || matches!(
                self.coordinates
                    .composition
                    .panels
                    .get(panel_index)
                    .map(|panel| panel.presentation),
                Some(composition::PanelPresentation::Table)
            )
        {
            return prepared;
        }
        let (Some(series), Some(bar_index)) = (self.coordinates.series.first(), bar_index) else {
            return prepared;
        };
        let title_gap = if title_width > 0.0 {
            PANEL_TITLE_TO_CONTROLS_GAP
        } else {
            0.0
        };
        let mut x = label_end_x + title_gap;
        let y = scene.layout.regions.plot.y
            + panel.plot.y
            + PANEL_TITLE_TOP_PAD
            + (indicator_index as f32 + 0.5) * TICKER_LEGEND_ROW_H;
        let max_x = self.panel_readout_max_x(scene, panel_index);
        for readout in self.indicators.indicator_header_values_for_indicator(
            scene,
            panel_index,
            value_id,
            0,
            series,
            bar_index,
        ) {
            if x >= max_x {
                break;
            }
            let value_text = self
                .coordinates
                .format_panel_header_value(panel_index, readout.value);
            let color = self.indicators.indicator_value_color(
                Some(readout.value_id),
                readout.color_slot,
                readout.color_role,
                palette,
                generated_accents,
            );
            x = prepared.push_text(readout.label.to_owned(), Point::new(x, y), color)
                + LEGEND_LABEL_VALUE_GAP;
            if x >= max_x {
                break;
            }
            prepared.end_x = prepared
                .push_text(value_text, Point::new(x, y), color)
                .max(label_end_x);
            x = prepared.end_x + LEGEND_READOUT_GAP;
        }
        prepared
    }

    fn indicator_panel_index(&self, scene: &Scene, value_id: PanelValueId) -> Option<usize> {
        scene
            .layout
            .panels
            .iter()
            .enumerate()
            .find_map(|(index, panel)| {
                (panel.kind == KlinePanelKind::Indicator
                    && self
                        .indicators
                        .panel_indicator_value_ids(index)
                        .contains(&value_id))
                .then_some(index)
            })
    }

    fn fill_indicator_panel_title(
        &self,
        frame: &mut PreparedChrome,
        scene: &Scene,
        row: &IndicatorLegendRowHit,
        row_rect: Rectangle,
        palette: &Extended,
    ) {
        let Some(panel_index) = self.indicator_panel_index(scene, row.value_id) else {
            return;
        };

        let row_alpha = if row.visible {
            1.0
        } else {
            HIDDEN_LEGEND_ALPHA
        };
        let title_y = scene.layout.regions.plot.y + row.y_center;

        if let Some((marker, loading)) = self.indicator_marker(row.value_id) {
            frame.fill_text(canvas::Text {
                content: marker.to_owned(),
                position: Point::new(indicator_marker_x(row_rect.x), title_y),
                color: if loading {
                    palette.background.base.text.scale_alpha(0.72 * row_alpha)
                } else {
                    palette.danger.base.color.scale_alpha(row_alpha)
                },
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }

        frame.fill_text(canvas::Text {
            content: self.indicator_title(panel_index, false, row.value_id),
            position: Point::new(self.indicator_label_x(row_rect.x, row.value_id), title_y),
            color: palette.background.base.text.scale_alpha(row_alpha),
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    pub fn build_indicator_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
    ) -> Option<IndicatorLegendLayout> {
        let primary_overlay_entries = self.indicators.primary_overlay_entries();
        let has_primary_overlays = !primary_overlay_entries.is_empty();
        let has_indicator_panels = layout.panels.iter().enumerate().any(|(index, panel)| {
            panel.kind == KlinePanelKind::Indicator
                && self.indicators.indicator_panel_value_id(index).is_some()
        });

        if !has_primary_overlays && !has_indicator_panels {
            return None;
        }

        let mut rows = Vec::new();

        for (panel_index, panel) in layout.panels.iter().enumerate() {
            if panel.kind != KlinePanelKind::Indicator {
                continue;
            }

            let value_ids = self.indicators.panel_indicator_value_ids(panel_index);
            if value_ids.is_empty() {
                continue;
            }

            for (indicator_index, value_id) in value_ids.into_iter().enumerate() {
                let label = self.indicator_title(panel_index, false, value_id);
                let x_left = self.indicator_label_x(panel.plot.x, value_id);
                let text_end_x = x_left + label.chars().count() as f32 * CHAR_W;
                let y_center = panel.plot.y
                    + PANEL_TITLE_TOP_PAD
                    + (indicator_index as f32 + 0.5) * TICKER_LEGEND_ROW_H;

                rows.push(IndicatorLegendRowHit {
                    value_id,
                    y_center,
                    controls: LegendRowControls::new(
                        panel.plot,
                        panel.plot.x,
                        text_end_x,
                        y_center,
                    ),
                    visible: self
                        .indicators
                        .panel_indicator_visible(panel_index, value_id),
                    primary_overlay: false,
                });
            }
        }

        if has_primary_overlays {
            let panel = layout.panel(primary_panel)?;
            for (overlay_index, (value_id, visible)) in primary_overlay_entries.iter().enumerate() {
                let row_top = panel.plot.y
                    + PANEL_TITLE_TOP_PAD
                    + TICKER_LEGEND_PADDING
                    + (preceding_rows + overlay_index) as f32 * TICKER_LEGEND_ROW_H;
                let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
                let title = self.indicator_title(primary_panel, true, *value_id);
                let title_x = self.indicator_label_x(panel.plot.x, *value_id);
                let text_end_x = title_x + title.chars().count() as f32 * CHAR_W;

                rows.push(IndicatorLegendRowHit {
                    value_id: *value_id,
                    y_center,
                    controls: LegendRowControls::new(
                        panel.plot,
                        panel.plot.x,
                        text_end_x,
                        y_center,
                    ),
                    visible: *visible,
                    primary_overlay: true,
                });
            }
        }

        Some(IndicatorLegendLayout { rows })
    }

    fn fill_panel_header_values(
        &self,
        frame: &mut PreparedChrome,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        show_primary_panel_values: bool,
        bar_index: Option<usize>,
        readouts: &mut [PreparedIndicatorReadouts],
    ) {
        let persistent_values_visible = self.legend_visibility.legend_values_always_visible();
        if !scene.interaction.readouts_visible() && !persistent_values_visible {
            return;
        }

        let Some(base_series) = self.coordinates.series.first() else {
            return;
        };

        if bar_index.is_none() {
            return;
        }

        let Some(base_bar) = scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene.interaction.cursor.and_then(|cursor| {
                    base_series.bar_at_or_before_unit(scene.x.axis, cursor.x_unit)
                })
            })
            .flatten()
            .or_else(|| {
                persistent_values_visible
                    .then(|| base_series.latest_point().map(|point| point.kline))
                    .flatten()
            })
        else {
            return;
        };

        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let title_w = self
                .coordinates
                .resolved_panel_title(panel_index, panel.kind)
                .map(|title| title.chars().count() as f32 * CHAR_W)
                .unwrap_or(0.0);
            let title_gap = if title_w > 0.0 {
                PANEL_TITLE_TO_CONTROLS_GAP
            } else {
                0.0
            };

            let mut x = scene.layout.regions.plot.x
                + panel.plot.x
                + TOOLTIP_ROW_LEFT_PAD
                + title_w
                + title_gap;
            let y = scene.layout.regions.plot.y
                + panel.plot.y
                + PANEL_TITLE_TOP_PAD
                + TICKER_LEGEND_ROW_H * 0.5;

            let max_x = self.panel_readout_max_x(scene, panel_index);

            if x >= max_x {
                continue;
            }

            if panel_index == scene.primary.panel_index {
                if !show_primary_panel_values {
                    continue;
                }

                let precision = base_series.ticker_info.min_ticksize;

                if matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                    let open_f = base_bar.open.to_f32_lossy();
                    let close_f = base_bar.close.to_f32_lossy();
                    let change_pct = if open_f.abs() > f32::EPSILON {
                        ((close_f - open_f) / open_f) * 100.0
                    } else {
                        0.0
                    };

                    let slot = if change_pct >= 0.0 {
                        data::chart::kline::CandlestickColorSlot::Bullish
                    } else {
                        data::chart::kline::CandlestickColorSlot::Bearish
                    };
                    let value_color = crate::widget::resolve_theme_color(
                        self.candlestick.colors.color_for(slot),
                        palette,
                        generated_accents,
                    );
                    let label_color = palette.background.base.text.scale_alpha(0.82);

                    let segments: Vec<(String, iced::Color, bool)> = vec![
                        ("O".to_string(), label_color, false),
                        (base_bar.open.to_string(precision), value_color, true),
                        ("H".to_string(), label_color, false),
                        (base_bar.high.to_string(precision), value_color, true),
                        ("L".to_string(), label_color, false),
                        (base_bar.low.to_string(precision), value_color, true),
                        ("C".to_string(), label_color, false),
                        (base_bar.close.to_string(precision), value_color, true),
                        (format!("{change_pct:+.2}%"), value_color, true),
                    ];

                    for (text, color, is_value) in segments {
                        if x >= max_x {
                            break;
                        }

                        let text_width = text.chars().count() as f32 * CHAR_W;
                        frame.fill_text(canvas::Text {
                            content: text,
                            position: Point::new(x, y),
                            color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        x += text_width;
                        x += if is_value { 6.0 } else { 2.0 };
                    }
                } else {
                    let text = format!("C {}", base_bar.close.to_string(precision));
                    frame.fill_text(canvas::Text {
                        content: text,
                        position: Point::new(x, y),
                        color: palette.background.base.text.scale_alpha(0.85),
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            } else {
                for readout in readouts
                    .iter_mut()
                    .filter(|readout| readout.panel_index == panel_index)
                {
                    readout.paint(frame);
                }
            }
        }
    }

    fn fill_primary_ticker_legend(
        &self,
        frame: &mut PreparedChrome,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let Some(legend) = scene.chrome.ticker_legend.as_ref() else {
            return;
        };

        let to_root = |rect: Rectangle| Rectangle {
            x: scene.layout.regions.plot.x + rect.x,
            y: scene.layout.regions.plot.y + rect.y,
            width: rect.width,
            height: rect.height,
        };

        let persistent_values_visible = self.legend_visibility.legend_values_always_visible();
        let target_x_unit = scene
            .interaction
            .readouts_visible()
            .then(|| scene.interaction.cursor.map(|cursor| cursor.x_unit))
            .flatten();
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.row_rect);
            let show_buttons = scene.interaction.target.ticker_row_is_hovered(index);
            let show_values = !show_buttons
                && (scene.interaction.readouts_visible() || persistent_values_visible);

            let label = self
                .coordinates
                .series
                .get(index)
                .map(series_display_name)
                .unwrap_or_else(|| row.ticker.ticker.symbol_and_exchange_string());
            let visible = if index == 0 {
                self.candlestick.visible
            } else {
                self.coordinates
                    .series_participates_in_primary_plot(row.ticker)
            };
            let row_alpha = if !visible { HIDDEN_LEGEND_ALPHA } else { 1.0 };
            let label_color = if index == 0 && !matches!(scene.primary.mark, MarkKind::Line) {
                palette.background.base.text
            } else {
                self.coordinates
                    .series
                    .get(index)
                    .map(|series| painting::series_line_color(series, palette, generated_accents))
                    .unwrap_or(palette.background.base.text)
            }
            .scale_alpha(if index == 0 { 1.0 } else { 0.96 });

            // Build the colored, segmented text for this row. The primary
            // series shows O/H/L/C + change% colored green/red by direction
            // (close vs open), using the latest bar when values are persistent
            // and switching to the hovered bar over the plot.
            let mut segments: Vec<(String, iced::Color)> = Vec::new();

            if let Some(series) = self.coordinates.series.get(index) {
                let effective_bar = if show_values {
                    if let Some(target_x_unit) = target_x_unit {
                        series.bar_at_or_before_unit(scene.x.axis, target_x_unit)
                    } else if self.legend_visibility.legend_values_always_visible() {
                        series.latest_point().map(|point| point.kline)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if index == 0 {
                    if let Some(bar) = effective_bar {
                        let precision = series.ticker_info.min_ticksize;
                        let open_f = bar.open.to_f32_lossy();
                        let close_f = bar.close.to_f32_lossy();
                        let change_pct = if open_f.abs() > f32::EPSILON {
                            ((close_f - open_f) / open_f) * 100.0
                        } else {
                            0.0
                        };
                        let slot = if change_pct >= 0.0 {
                            data::chart::kline::CandlestickColorSlot::Bullish
                        } else {
                            data::chart::kline::CandlestickColorSlot::Bearish
                        };
                        let value_color = crate::widget::resolve_theme_color(
                            self.candlestick.colors.color_for(slot),
                            palette,
                            generated_accents,
                        );

                        segments.push((label, label_color));
                        segments.push((" O".to_string(), label_color));
                        segments.push((bar.open.to_string(precision), value_color));
                        segments.push((" H".to_string(), label_color));
                        segments.push((bar.high.to_string(precision), value_color));
                        segments.push((" L".to_string(), label_color));
                        segments.push((bar.low.to_string(precision), value_color));
                        segments.push((" C".to_string(), label_color));
                        segments.push((bar.close.to_string(precision), value_color));
                        segments.push((format!(" {:+.2}%", change_pct), value_color));
                    } else {
                        segments.push((label, label_color));
                    }
                } else if let Some(bar) = effective_bar {
                    let pct_text = series
                        .bar_at_or_before_unit(scene.x.axis, scene.x.min_unit)
                        .and_then(|anchor_bar| {
                            let anchor = anchor_bar.close.to_f32_lossy();
                            (anchor.abs() > f32::EPSILON)
                                .then_some(((bar.close.to_f32_lossy() / anchor) - 1.0) * 100.0)
                        })
                        .map(|pct| format!("{pct:+.2}%"))
                        .unwrap_or_else(|| "n/a".to_string());
                    segments.push((label, label_color));
                    segments.push((format!(" {pct_text}"), label_color));
                } else {
                    segments.push((label, label_color));
                }
            } else {
                segments.push((label, label_color));
            }

            let text_width = segments
                .iter()
                .map(|(text, _)| text.chars().count() as f32 * CHAR_W)
                .sum::<f32>();
            let background = if show_buttons {
                row_rect
            } else {
                Rectangle {
                    width: text_width + TICKER_LEGEND_PADDING * 2.0,
                    ..row_rect
                }
            };
            frame.fill_rectangle(
                background.position(),
                background.size(),
                legend_background_color(palette),
            );

            if show_buttons {
                let highlight = Rectangle {
                    x: background.x + 1.0,
                    y: background.y,
                    width: (background.width - 2.0).max(0.0),
                    height: background.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            let mut text_x = row_rect.x + TICKER_LEGEND_PADDING;
            let text_y = scene.layout.regions.plot.y + row.y_center;

            for (text, color) in segments {
                let text_width = text.chars().count() as f32 * CHAR_W;
                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(text_x, text_y),
                    color: color.scale_alpha(row_alpha),
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
                text_x += text_width;
            }

            if show_buttons {
                let settings_root = to_root(row.settings);
                let close_root = to_root(row.close);

                let settings_hovered = scene
                    .interaction
                    .target
                    .ticker_icon_is_hovered(index, TickerLegendIconKind::Settings);
                draw_legend_icon_button(
                    frame,
                    settings_root,
                    TickerLegendIconKind::Settings.icon(true),
                    settings_hovered,
                    palette,
                );

                draw_legend_icon_button(
                    frame,
                    to_root(row.visibility),
                    TickerLegendIconKind::Visibility.icon(visible),
                    scene
                        .interaction
                        .target
                        .ticker_icon_is_hovered(index, TickerLegendIconKind::Visibility),
                    palette,
                );
                if row.has_close {
                    draw_legend_icon_button(
                        frame,
                        close_root,
                        TickerLegendIconKind::Close.icon(true),
                        scene
                            .interaction
                            .target
                            .ticker_icon_is_hovered(index, TickerLegendIconKind::Close),
                        palette,
                    );
                }
            }
        }
    }

    fn fill_footprint_legend(&self, frame: &mut PreparedChrome, scene: &Scene, palette: &Extended) {
        let Some(legend) = scene.chrome.footprint_legend.as_ref() else {
            return;
        };

        let to_root = |rect: Rectangle| Rectangle {
            x: scene.layout.regions.plot.x + rect.x,
            y: scene.layout.regions.plot.y + rect.y,
            width: rect.width,
            height: rect.height,
        };
        let row_rect = to_root(legend.row.row_rect);
        let hovered_row = scene.interaction.target.footprint_legend_is_hovered();
        let label = format!("Footprint · {}", legend.cluster_kind);
        let content_end_x = if hovered_row {
            let hide = to_root(legend.row.hide);
            hide.x + hide.width
        } else {
            row_rect.x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W
        };
        let background = legend_row_background(row_rect, content_end_x);
        frame.fill_rectangle(
            background.position(),
            background.size(),
            legend_background_color(palette),
        );

        if hovered_row {
            frame.fill_rectangle(
                background.position(),
                background.size(),
                palette.background.strong.color.scale_alpha(0.22),
            );
        }

        frame.fill_text(canvas::Text {
            content: label,
            position: Point::new(
                row_rect.x + TOOLTIP_ROW_LEFT_PAD,
                row_rect.y + row_rect.height * 0.5,
            ),
            color: palette
                .background
                .base
                .text
                .scale_alpha(if self.footprint.visible {
                    1.0
                } else {
                    HIDDEN_LEGEND_ALPHA
                }),
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });

        if !hovered_row {
            return;
        }

        let settings = to_root(legend.row.settings);
        let hide = to_root(legend.row.hide);
        let settings_hovered = scene
            .interaction
            .target
            .footprint_icon_is_hovered(FootprintLegendIconKind::Settings);
        draw_legend_icon_button(
            frame,
            settings,
            FootprintLegendIconKind::Settings.icon(true),
            settings_hovered,
            palette,
        );
        let hide_hovered = scene
            .interaction
            .target
            .footprint_icon_is_hovered(FootprintLegendIconKind::Hide);
        draw_legend_icon_button(
            frame,
            hide,
            FootprintLegendIconKind::Hide.icon(self.footprint.visible),
            hide_hovered,
            palette,
        );
    }

    fn fill_study_legend(&self, frame: &mut PreparedChrome, scene: &Scene, palette: &Extended) {
        let Some(legend) = scene.chrome.study_legend.as_ref() else {
            return;
        };

        let to_root = |rect: Rectangle| Rectangle {
            x: scene.layout.regions.plot.x + rect.x,
            y: scene.layout.regions.plot.y + rect.y,
            width: rect.width,
            height: rect.height,
        };
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);
        let category_rect = to_root(legend.category.row_rect);
        let category_hovered = scene.interaction.target.study_category_is_hovered();
        let category_label = "Studies";
        let count_badge = to_root(legend.category.count_badge);
        let count_badge_end_x = count_badge.x + count_badge.width;
        let category_content_end_x = if category_hovered {
            [
                legend.category.expand,
                legend.category.visibility,
                legend.category.close,
            ]
            .iter()
            .map(|rect| {
                let rect = to_root(*rect);
                rect.x + rect.width
            })
            .max_by(f32::total_cmp)
            .unwrap_or(count_badge_end_x)
        } else {
            count_badge_end_x
        };
        let category_background = legend_row_background(category_rect, category_content_end_x);

        frame.fill_rectangle(
            category_background.position(),
            category_background.size(),
            legend_background_color(palette),
        );

        if category_hovered {
            let highlight = Rectangle {
                x: category_background.x + 1.0,
                y: category_background.y,
                width: (category_background.width - 2.0).max(0.0),
                height: category_background.height,
            };
            frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
        }

        frame.fill_text(canvas::Text {
            content: category_label.to_string(),
            position: Point::new(
                category_rect.x + TOOLTIP_ROW_LEFT_PAD,
                category_rect.y + category_rect.height * 0.5,
            ),
            color: palette.background.base.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });

        frame.fill_rectangle(
            count_badge.position(),
            count_badge.size(),
            palette.background.strong.color.scale_alpha(0.65),
        );
        frame.fill_text(canvas::Text {
            content: legend.study_count.to_string(),
            position: Point::new(
                count_badge.x + count_badge.width * 0.5,
                count_badge.y + count_badge.height * 0.5,
            ),
            color: palette.background.strongest.text,
            size: (TEXT_SIZE - 1.0).into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });

        if category_hovered {
            let expand = to_root(legend.category.expand);
            let visibility = to_root(legend.category.visibility);
            let close = to_root(legend.category.close);
            draw_legend_icon_button(
                frame,
                expand,
                StudiesLegendIconKind::ExpandCollapse.icon(&legend.category),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::ExpandCollapse),
                palette,
            );
            draw_legend_icon_button(
                frame,
                visibility,
                StudiesLegendIconKind::Visibility.icon(&legend.category),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::Visibility),
                palette,
            );
            draw_legend_icon_button(
                frame,
                close,
                StudiesLegendIconKind::Close.icon(&legend.category),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::Close),
                palette,
            );
        }

        for (index, row) in legend.rows.iter().enumerate() {
            let Some(study) = self
                .study_legend
                .as_ref()
                .and_then(|legend| legend.rows.get(index))
            else {
                continue;
            };

            let row_rect = to_root(row.controls.row_rect);
            let hovered_row = scene.interaction.target.study_row_is_hovered(index);
            let label = &study.label;
            let label_end_x =
                row_rect.x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
            let content_end_x = if hovered_row {
                [
                    row.controls.visibility,
                    row.controls.settings,
                    row.controls.close,
                ]
                .iter()
                .map(|rect| {
                    let rect = to_root(*rect);
                    rect.x + rect.width
                })
                .max_by(f32::total_cmp)
                .unwrap_or(label_end_x)
            } else {
                label_end_x
            };
            let background = legend_row_background(row_rect, content_end_x);

            frame.fill_rectangle(
                background.position(),
                background.size(),
                legend_background_color(palette),
            );

            if hovered_row {
                let highlight = Rectangle {
                    x: background.x + 1.0,
                    y: background.y,
                    width: (background.width - 2.0).max(0.0),
                    height: background.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            frame.fill_text(canvas::Text {
                content: label.clone(),
                position: Point::new(
                    row_rect.x + TOOLTIP_ROW_LEFT_PAD,
                    row_rect.y + row_rect.height * 0.5,
                ),
                color: palette.background.base.text.scale_alpha(if row.visible {
                    1.0
                } else {
                    HIDDEN_LEGEND_ALPHA
                }),
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });

            if !hovered_row {
                continue;
            }

            let hovered = match scene.interaction.target {
                CursorTarget::Legend(LegendTarget::Study { row, icon }) if row == index => icon,
                _ => None,
            };
            row.controls.draw(
                frame,
                scene.layout.regions.plot.position(),
                row.visible,
                hovered,
                palette,
            );
        }
    }

    fn fill_indicator_legend(
        &self,
        frame: &mut PreparedChrome,
        scene: &Scene,
        palette: &Extended,
        readouts: &mut [PreparedIndicatorReadouts],
    ) {
        let Some(legend) = scene.chrome.indicator_legend.as_ref() else {
            return;
        };

        let to_root = |rect: Rectangle| Rectangle {
            x: scene.layout.regions.plot.x + rect.x,
            y: scene.layout.regions.plot.y + rect.y,
            width: rect.width,
            height: rect.height,
        };
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.controls.row_rect);
            let show_buttons = scene.interaction.target.indicator_row_is_hovered(index);
            let panel_index = if row.primary_overlay {
                Some(scene.primary.panel_index)
            } else {
                self.indicator_panel_index(scene, row.value_id)
            };
            let prepared = readouts.iter_mut().find(|readout| {
                Some(readout.panel_index) == panel_index && readout.value_id == row.value_id
            });
            let content_end_x = if show_buttons {
                [
                    row.controls.visibility,
                    row.controls.settings,
                    row.controls.close,
                ]
                .iter()
                .filter(|rect| rect.width > 0.0)
                .map(|rect| scene.layout.regions.plot.x + rect.x + rect.width)
                .max_by(f32::total_cmp)
                .unwrap_or(row_rect.x)
            } else {
                prepared
                    .as_ref()
                    .map_or(row_rect.x + TOOLTIP_ROW_LEFT_PAD, |readout| readout.end_x)
            };
            let background = legend_row_background(row_rect, content_end_x);

            frame.fill_rectangle(
                background.position(),
                background.size(),
                legend_background_color(palette),
            );

            if show_buttons {
                let highlight = Rectangle {
                    x: background.x + 1.0,
                    y: background.y,
                    width: (background.width - 2.0).max(0.0),
                    height: background.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            if row.primary_overlay {
                let marker_x = indicator_marker_x(row_rect.x);
                let label_x = self.indicator_label_x(row_rect.x, row.value_id);
                let label_y = scene.layout.regions.plot.y + row.y_center;
                let row_alpha = if row.visible {
                    1.0
                } else {
                    HIDDEN_LEGEND_ALPHA
                };

                if let Some((marker, loading)) = self.indicator_marker(row.value_id) {
                    frame.fill_text(canvas::Text {
                        content: marker.to_owned(),
                        position: Point::new(marker_x, label_y),
                        color: if loading {
                            palette.background.base.text.scale_alpha(0.72 * row_alpha)
                        } else {
                            palette.danger.base.color.scale_alpha(row_alpha)
                        },
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }

                let title = self.indicator_title(scene.primary.panel_index, true, row.value_id);

                frame.fill_text(canvas::Text {
                    content: title,
                    position: Point::new(label_x, label_y),
                    color: palette.background.base.text.scale_alpha(row_alpha),
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });

                if let Some(prepared) = prepared {
                    prepared.paint(frame);
                }
            } else {
                self.fill_indicator_panel_title(frame, scene, row, row_rect, palette);
            }

            if show_buttons {
                let hovered = match scene.interaction.target {
                    CursorTarget::Legend(LegendTarget::Indicator { row, icon }) if row == index => {
                        icon
                    }
                    _ => None,
                };
                row.controls.draw(
                    frame,
                    scene.layout.regions.plot.position(),
                    row.visible,
                    hovered,
                    palette,
                );
            }
        }
    }

    pub fn fill_legends(
        &self,
        inputs: &mut PreparedChrome,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let bar_index = self.readout_bar_index(scene);
        let mut readouts =
            self.prepare_indicator_readouts(scene, bar_index, palette, generated_accents);
        self.fill_primary_ticker_legend(inputs, scene, palette, generated_accents);
        self.fill_footprint_legend(inputs, scene, palette);
        self.fill_study_legend(inputs, scene, palette);
        self.fill_indicator_legend(inputs, scene, palette, &mut readouts);
        self.fill_panel_header_values(
            inputs,
            scene,
            palette,
            generated_accents,
            scene.chrome.ticker_legend.is_none(),
            bar_index,
            &mut readouts,
        );
    }
}

pub fn indicator_marker_x(row_x: f32) -> f32 {
    row_x + TOOLTIP_ROW_LEFT_PAD
}

pub fn tooltip_rows_before_studies(
    ticker_legend: Option<&TickerLegendLayout>,
    footprint_legend: Option<&FootprintLegendLayout>,
    show_legend_values: bool,
) -> usize {
    ticker_legend
        .map(|legend| legend.rows.len())
        .unwrap_or(usize::from(show_legend_values))
        + usize::from(footprint_legend.is_some())
}

pub fn tooltip_rows_before_indicators(
    ticker_legend: Option<&TickerLegendLayout>,
    footprint_legend: Option<&FootprintLegendLayout>,
    study_legend: Option<&StudyLegendLayout>,
    show_legend_values: bool,
) -> usize {
    tooltip_rows_before_studies(ticker_legend, footprint_legend, show_legend_values)
        + study_legend
            .map(|legend| legend.rows.len() + 1)
            .unwrap_or_default()
}

fn legend_controls(
    panel: Rectangle,
    text_end_x: f32,
    y_center: f32,
    has_expand: bool,
    has_visibility: bool,
    has_settings: bool,
    has_close: bool,
) -> (Rectangle, Rectangle, Rectangle, Rectangle, f32) {
    let x_right = panel.x + panel.width - TICKER_LEGEND_PADDING;
    let free_left = text_end_x + TICKER_LEGEND_PADDING * 2.0;
    let icon_count = usize::from(has_expand)
        + usize::from(has_visibility)
        + usize::from(has_settings)
        + usize::from(has_close);
    let icon_pack_w = icon_count as f32 * TICKER_LEGEND_ICON_BOX
        + icon_count.saturating_sub(1) as f32 * TICKER_LEGEND_ICON_GAP;

    let pack_x = free_left.min((x_right - icon_pack_w).max(free_left));
    let first_icon = Rectangle {
        x: pack_x,
        y: y_center - TICKER_LEGEND_ICON_BOX * 0.5,
        width: TICKER_LEGEND_ICON_BOX,
        height: TICKER_LEGEND_ICON_BOX,
    };
    let empty_icon = || Rectangle {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
    let mut next_x = pack_x;
    let expand = if has_expand {
        let expand = Rectangle {
            x: next_x,
            ..first_icon
        };
        next_x += TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP;
        expand
    } else {
        empty_icon()
    };
    let settings = if has_settings {
        let settings = Rectangle {
            x: next_x,
            ..first_icon
        };
        next_x += TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP;
        settings
    } else {
        empty_icon()
    };
    let visibility = if has_visibility {
        let visibility = Rectangle {
            x: next_x,
            ..first_icon
        };
        next_x += TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP;
        visibility
    } else {
        empty_icon()
    };
    let close = if has_close {
        Rectangle {
            x: next_x,
            ..first_icon
        }
    } else {
        empty_icon()
    };
    let content_right = [expand, settings, visibility, close]
        .iter()
        .filter(|rect| rect.width > 0.0)
        .map(|rect| rect.x + rect.width)
        .max_by(f32::total_cmp)
        .unwrap_or(settings.x + settings.width);
    let row_w = (content_right + TICKER_LEGEND_PADDING - panel.x).clamp(0.0, panel.width);
    (expand, visibility, settings, close, row_w)
}

fn draw_legend_icon_button(
    frame: &mut PreparedChrome,
    rect: Rectangle,
    icon: style::Icon,
    hovered: bool,
    palette: &Extended,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let text = if hovered {
        palette.background.strongest.text
    } else {
        palette.background.base.text
    };
    frame.fill_text(canvas::Text {
        content: char::from(icon).to_string(),
        position: Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
        color: text,
        size: TEXT_SIZE.into(),
        align_x: iced::Alignment::Center.into(),
        align_y: iced::Alignment::Center.into(),
        font: style::ICONS_FONT,
        ..Default::default()
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickerLegendIconKind {
    Settings,
    Visibility,
    Close,
}

impl TickerLegendIconKind {
    fn icon(self, visible: bool) -> style::Icon {
        match self {
            Self::Settings => style::Icon::Cog,
            Self::Visibility => style::Icon::visibility(visible),
            Self::Close => style::Icon::Close,
        }
    }

    pub fn into_event(self, ticker: TickerInfo) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::TickerSettings(ticker),
            Self::Visibility => KlineWidgetEvent::TickerVisibilityToggled(ticker),
            Self::Close => KlineWidgetEvent::TickerRemove(ticker),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TickerLegendRowHit {
    pub ticker: TickerInfo,
    y_center: f32,
    row_rect: Rectangle,
    settings: Rectangle,
    visibility: Rectangle,
    close: Rectangle,
    has_close: bool,
}

#[derive(Debug, Clone)]
pub struct TickerLegendLayout {
    bg: Rectangle,
    pub rows: Vec<TickerLegendRowHit>,
}

impl TickerLegendLayout {
    pub fn hit_test(&self, layout: &PanelLayoutTree, root_local: Point) -> Option<TickerLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        if !self.bg.contains(plot_local) {
            return None;
        }

        for (index, row) in self.rows.iter().enumerate() {
            if !row.row_rect.contains(plot_local) {
                continue;
            }

            if row.settings.contains(plot_local) {
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Settings));
            }

            if row.visibility.contains(plot_local) {
                return Some(TickerLegendHit::Icon(
                    index,
                    TickerLegendIconKind::Visibility,
                ));
            }
            if row.has_close && row.close.contains(plot_local) {
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Close));
            }

            return Some(TickerLegendHit::Row(index));
        }

        None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TickerLegendHit {
    Row(usize),
    Icon(usize, TickerLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootprintLegendIconKind {
    Settings,
    Hide,
}

impl FootprintLegendIconKind {
    fn icon(self, visible: bool) -> style::Icon {
        match self {
            Self::Settings => style::Icon::Cog,
            Self::Hide => style::Icon::visibility(visible),
        }
    }

    pub fn into_event(self) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::FootprintSettings,
            Self::Hide => KlineWidgetEvent::FootprintsVisibilityToggled,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FootprintLegendRowHit {
    row_rect: Rectangle,
    settings: Rectangle,
    hide: Rectangle,
}

#[derive(Debug, Clone, Copy)]
pub struct FootprintLegendLayout {
    row: FootprintLegendRowHit,
    cluster_kind: ClusterKind,
}

impl FootprintLegendLayout {
    pub fn new(
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
        cluster_kind: ClusterKind,
    ) -> Option<Self> {
        let panel = layout.panel(primary_panel)?;
        let row_top = panel.plot.y
            + PANEL_TITLE_TOP_PAD
            + TICKER_LEGEND_PADDING
            + preceding_rows as f32 * TICKER_LEGEND_ROW_H;
        let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
        let label = format!("Footprint · {cluster_kind}");
        let text_end_x =
            panel.plot.x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
        let (_, _, settings, hide, row_w) =
            legend_controls(panel.plot, text_end_x, y_center, false, false, true, true);

        Some(Self {
            row: FootprintLegendRowHit {
                row_rect: Rectangle {
                    x: panel.plot.x,
                    y: row_top,
                    width: row_w,
                    height: TICKER_LEGEND_ROW_H,
                },
                settings,
                hide,
            },
            cluster_kind,
        })
    }

    pub fn hit_test(
        &self,
        layout: &PanelLayoutTree,
        root_local: Point,
    ) -> Option<FootprintLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        if !self.row.row_rect.contains(plot_local) {
            return None;
        }
        if self.row.settings.contains(plot_local) {
            return Some(FootprintLegendHit::Icon(FootprintLegendIconKind::Settings));
        }
        if self.row.hide.contains(plot_local) {
            return Some(FootprintLegendHit::Icon(FootprintLegendIconKind::Hide));
        }
        Some(FootprintLegendHit::Row)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FootprintLegendHit {
    Row,
    Icon(FootprintLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudiesLegendIconKind {
    ExpandCollapse,
    Visibility,
    Close,
}

impl StudiesLegendIconKind {
    fn icon(self, row: &StudiesLegendRowHit) -> style::Icon {
        match self {
            Self::ExpandCollapse => {
                if row.expanded {
                    style::Icon::ChevronDown
                } else {
                    style::Icon::ChevronRight
                }
            }
            Self::Visibility => style::Icon::visibility(row.visible),
            Self::Close => style::Icon::Close,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StudiesLegendRowHit {
    row_rect: Rectangle,
    expand: Rectangle,
    visibility: Rectangle,
    close: Rectangle,
    count_badge: Rectangle,
    expanded: bool,
    visible: bool,
}

pub struct StudyLegendRow {
    pub id: KlineStudyId,
    pub label: String,
    pub visible: bool,
}

pub struct StudiesLegend {
    pub expanded: bool,
    pub rows: Vec<StudyLegendRow>,
}

impl StudiesLegend {
    fn visible(&self) -> bool {
        self.rows.iter().all(|row| row.visible)
    }
}

#[derive(Debug, Clone, Copy)]
struct StudyLegendRowHit {
    controls: LegendRowControls,
    visible: bool,
}

#[derive(Debug, Clone)]
pub struct StudyLegendLayout {
    category: StudiesLegendRowHit,
    study_count: usize,
    rows: Vec<StudyLegendRowHit>,
}

impl StudyLegendLayout {
    pub fn hit_test(&self, layout: &PanelLayoutTree, root_local: Point) -> Option<StudyLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;

        if self.category.row_rect.contains(plot_local) {
            if self.category.expand.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(
                    StudiesLegendIconKind::ExpandCollapse,
                ));
            }

            if self.category.visibility.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(
                    StudiesLegendIconKind::Visibility,
                ));
            }

            if self.category.close.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(StudiesLegendIconKind::Close));
            }

            return Some(StudyLegendHit::CategoryRow);
        }

        for (index, row) in self.rows.iter().enumerate() {
            if let Some(hit) = row.controls.hit(plot_local) {
                return Some(match hit {
                    LegendRowHit::Action(action) => StudyLegendHit::StudyIcon(index, action),
                    LegendRowHit::Row => StudyLegendHit::StudyRow(index),
                });
            }
        }

        None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum StudyLegendHit {
    CategoryRow,
    CategoryIcon(StudiesLegendIconKind),
    StudyRow(usize),
    StudyIcon(usize, LegendRowAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendRowAction {
    Visibility,
    Settings,
    Close,
}

impl LegendRowAction {
    fn icon(self, visible: bool) -> style::Icon {
        match self {
            Self::Visibility => style::Icon::visibility(visible),
            Self::Settings => style::Icon::Cog,
            Self::Close => style::Icon::Close,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LegendRowHit {
    Row,
    Action(LegendRowAction),
}

#[derive(Debug, Clone, Copy)]
struct LegendRowControls {
    row_rect: Rectangle,
    visibility: Rectangle,
    settings: Rectangle,
    close: Rectangle,
}

impl LegendRowControls {
    fn new(panel: Rectangle, row_x: f32, text_end_x: f32, y_center: f32) -> Self {
        let (_, visibility, settings, close, width) =
            legend_controls(panel, text_end_x, y_center, false, true, true, true);
        Self {
            row_rect: Rectangle {
                x: row_x,
                y: y_center - TICKER_LEGEND_ROW_H * 0.5,
                width,
                height: TICKER_LEGEND_ROW_H,
            },
            visibility,
            settings,
            close,
        }
    }

    fn buttons(&self) -> [(LegendRowAction, Rectangle); 3] {
        [
            (LegendRowAction::Visibility, self.visibility),
            (LegendRowAction::Settings, self.settings),
            (LegendRowAction::Close, self.close),
        ]
    }

    fn hit(&self, point: Point) -> Option<LegendRowHit> {
        self.row_rect.contains(point).then(|| {
            self.buttons()
                .into_iter()
                .find_map(|(action, rect)| rect.contains(point).then_some(action))
                .map_or(LegendRowHit::Row, LegendRowHit::Action)
        })
    }

    fn draw(
        &self,
        frame: &mut PreparedChrome,
        plot_origin: Point,
        visible: bool,
        hovered: Option<LegendRowAction>,
        palette: &Extended,
    ) {
        for (action, rect) in self.buttons() {
            let rect = Rectangle {
                x: rect.x + plot_origin.x,
                y: rect.y + plot_origin.y,
                ..rect
            };
            draw_legend_icon_button(
                frame,
                rect,
                action.icon(visible),
                hovered == Some(action),
                palette,
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorLegendRowHit {
    pub value_id: super::PanelValueId,
    y_center: f32,
    controls: LegendRowControls,
    visible: bool,
    primary_overlay: bool,
}

#[derive(Debug, Clone)]
pub struct IndicatorLegendLayout {
    pub rows: Vec<IndicatorLegendRowHit>,
}

impl IndicatorLegendLayout {
    pub fn hit_test(
        &self,
        layout: &PanelLayoutTree,
        root_local: Point,
    ) -> Option<IndicatorLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        for (index, row) in self.rows.iter().enumerate() {
            if let Some(hit) = row.controls.hit(plot_local) {
                return Some(match hit {
                    LegendRowHit::Action(action) => IndicatorLegendHit::Icon(index, action),
                    LegendRowHit::Row => IndicatorLegendHit::Row(index),
                });
            }
        }

        None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IndicatorLegendHit {
    Row(usize),
    Icon(usize, LegendRowAction),
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorHeaderValue<'a> {
    pub value_id: PanelValueId,
    pub label: &'a str,
    pub value: f64,
    pub color_role: ThemeColorRole,
    pub color_slot: &'a str,
}
