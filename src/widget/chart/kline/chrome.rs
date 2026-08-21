use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::scene::Scene;
use super::{
    CHAR_W, KlineSeriesLike, KlineWidget, KlineWidgetEvent, PANEL_TITLE_TOP_PAD, TEXT_SIZE,
    TICKER_LEGEND_PADDING, TICKER_LEGEND_ROW_H, TOOLTIP_ROW_LEFT_PAD,
};
use crate::style;
use crate::widget::chart::kline::PanelInteraction;
use crate::widget::chart::kline::composition::{MarkKind, PanelScaleMode};
use data::chart::kline::ClusterKind;

use exchange::TickerInfo;

use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};

const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 8.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = 0.0;
const SERIES_END_LABEL_GAP: f32 = 2.0;
const HIDDEN_LEGEND_ALPHA: f32 = 0.45;

fn series_display_name<S: KlineSeriesLike>(series: &S) -> String {
    series.ticker_info().ticker.symbol_and_exchange_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelControlKind {
    MoveUp,
    MoveDown,
    Close,
}

impl PanelControlKind {
    fn icon(self) -> &'static str {
        match self {
            Self::MoveUp => "^",
            Self::MoveDown => "v",
            Self::Close => "X",
        }
    }

    pub(super) fn into_interaction_kind(self, index: usize) -> PanelInteraction {
        match self {
            Self::MoveUp => PanelInteraction::MoveUp { index },
            Self::MoveDown => PanelInteraction::MoveDown { index },
            Self::Close => PanelInteraction::Close { index },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PanelControlHit {
    pub(super) panel_index: usize,
    pub(super) kind: PanelControlKind,
    pub(super) rect: Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CornerControlKind {
    PrimaryAutoscaleToggle,
    PrimaryScaleModeCycle,
}

impl CornerControlKind {
    fn label(self, primary_scale_mode: PanelScaleMode) -> &'static str {
        match self {
            Self::PrimaryAutoscaleToggle => "A",
            Self::PrimaryScaleModeCycle => match primary_scale_mode {
                PanelScaleMode::PercentFromBase => "%",
                _ => "L",
            },
        }
    }

    fn is_active(self, primary_scale_mode: PanelScaleMode, primary_autoscale: bool) -> bool {
        match self {
            Self::PrimaryAutoscaleToggle => primary_autoscale,
            Self::PrimaryScaleModeCycle => {
                matches!(
                    primary_scale_mode,
                    PanelScaleMode::Logarithmic | PanelScaleMode::PercentFromBase
                )
            }
        }
    }

    pub(super) fn into_event(self) -> KlineWidgetEvent {
        match self {
            Self::PrimaryAutoscaleToggle => KlineWidgetEvent::PrimaryAutoscaleToggled,
            Self::PrimaryScaleModeCycle => KlineWidgetEvent::PrimaryScaleModeCycleRequested,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CornerControlHit {
    pub(super) kind: CornerControlKind,
    pub(super) rect: Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TickerLegendIconKind {
    Settings,
    Hide,
    Close,
}

impl TickerLegendIconKind {
    fn icon(self) -> style::Icon {
        match self {
            Self::Settings => style::Icon::Cog,
            Self::Hide => style::Icon::SpeakerOff,
            Self::Close => style::Icon::Close,
        }
    }

    pub(super) fn into_event(self, ticker: TickerInfo) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::TickerSettings(ticker),
            Self::Hide => KlineWidgetEvent::CandlesticksVisibilityToggled,
            Self::Close => KlineWidgetEvent::TickerRemove(ticker),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TickerLegendRowHit {
    pub(super) ticker: TickerInfo,
    pub(super) y_center: f32,
    pub(super) row_rect: Rectangle,
    pub(super) settings: Rectangle,
    pub(super) close: Rectangle,
    pub(super) has_close: bool,
}

#[derive(Debug, Clone)]
pub(super) struct TickerLegendLayout {
    pub(super) bg: Rectangle,
    pub(super) rows: Vec<TickerLegendRowHit>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TickerLegendHit {
    Background,
    Row(usize),
    Icon(usize, TickerLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FootprintLegendIconKind {
    Settings,
    Hide,
}

impl FootprintLegendIconKind {
    fn icon(self) -> style::Icon {
        match self {
            Self::Settings => style::Icon::Cog,
            Self::Hide => style::Icon::SpeakerOff,
        }
    }

    pub(super) fn into_event(self) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::FootprintSettings,
            Self::Hide => KlineWidgetEvent::FootprintsVisibilityToggled,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FootprintLegendRowHit {
    pub(super) row_rect: Rectangle,
    pub(super) settings: Rectangle,
    pub(super) hide: Rectangle,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FootprintLegendLayout {
    pub(super) row: FootprintLegendRowHit,
    pub(super) cluster_kind: ClusterKind,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FootprintLegendHit {
    Row,
    Icon(FootprintLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorLegendIconKind {
    Visibility,
    Settings,
    Close,
}

impl IndicatorLegendIconKind {
    fn icon(self, visible: bool) -> style::Icon {
        match self {
            Self::Visibility if visible => style::Icon::SpeakerHigh,
            Self::Visibility => style::Icon::SpeakerOff,
            Self::Settings => style::Icon::Cog,
            Self::Close => style::Icon::Close,
        }
    }

    pub(super) fn into_event(self, value_id: super::PanelValueId) -> KlineWidgetEvent {
        match self {
            Self::Visibility => KlineWidgetEvent::IndicatorVisibilityToggled(value_id),
            Self::Settings => KlineWidgetEvent::IndicatorSettings(value_id),
            Self::Close => KlineWidgetEvent::IndicatorRemove(value_id),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorLegendRowHit {
    pub(super) value_id: super::PanelValueId,
    pub(super) y_center: f32,
    pub(super) row_rect: Rectangle,
    pub(super) visibility: Rectangle,
    pub(super) settings: Rectangle,
    pub(super) close: Rectangle,
    pub(super) visible: bool,
    pub(super) primary_overlay: bool,
}

#[derive(Debug, Clone)]
pub(super) struct IndicatorLegendLayout {
    pub(super) rows: Vec<IndicatorLegendRowHit>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum IndicatorLegendHit {
    Row(usize),
    Icon(usize, IndicatorLegendIconKind),
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn fill_primary_series_end_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        if self.series.len() < 2 {
            return;
        }

        let primary_plot = scene.primary_plot();
        let plot_origin = scene.layout.regions.plot;
        let split_x = plot_origin.x + primary_plot.x + primary_plot.width;
        let min_y = plot_origin.y + primary_plot.y;
        let max_y = min_y + primary_plot.height;
        let mut labels = Vec::new();

        for (index, series) in self.series.iter().enumerate().skip(1) {
            let Some(bar) = self.bar_at_or_before_unit(series, scene.x.axis, scene.x.max_unit)
            else {
                continue;
            };
            let anchor = scene
                .primary
                .series_percent_anchors
                .get(index)
                .copied()
                .flatten();
            let y = (plot_origin.y
                + scene.map_primary_plot_with_anchor(bar.close.to_f32_lossy(), anchor))
            .clamp(min_y, max_y);
            let color = self.series_line_color(series, palette).scale_alpha(0.92);
            let text_color = if data::config::theme::is_dark(color) {
                iced::Color::WHITE
            } else {
                iced::Color::BLACK
            };
            let value = anchor
                .filter(|anchor| anchor.abs() > f32::EPSILON)
                .map(|anchor| {
                    format!(
                        "{:+.2}%",
                        ((bar.close.to_f32_lossy() / anchor) - 1.0) * 100.0
                    )
                })
                .unwrap_or_else(|| scene.format_primary_cursor_label(bar.close.to_f32_lossy()));

            labels.push(SeriesEndLabel {
                y,
                color,
                text_color,
                text: series_display_name(series),
                value: Some(value),
            });
        }

        resolve_series_end_label_overlaps(&mut labels, min_y, max_y);

        for label in labels {
            let label_height = TEXT_SIZE + 4.0;
            let label_width = label.text.chars().count() as f32 * CHAR_W + 8.0;
            let rect = Rectangle {
                x: split_x - label_width,
                y: label.y - label_height * 0.5,
                width: label_width,
                height: label_height,
            };

            frame.fill_rectangle(rect.position(), rect.size(), label.color);
            frame.fill_text(canvas::Text {
                content: label.text,
                position: Point::new(rect.x + rect.width - 4.0, label.y),
                color: label.text_color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });

            if let Some(value) = label.value {
                let axis_width = scene.layout.regions.y_axis.width;
                let value_rect = Rectangle {
                    x: split_x + 2.0,
                    y: rect.y,
                    width: (axis_width - 3.0).max(0.0),
                    height: rect.height,
                };
                frame.fill_rectangle(value_rect.position(), value_rect.size(), label.color);
                frame.fill(
                    &canvas::Path::circle(Point::new(split_x + axis_width, label.y), 4.0),
                    label.color,
                );
                frame.fill_text(canvas::Text {
                    content: value,
                    position: Point::new(value_rect.x + value_rect.width - 4.0, label.y),
                    color: label.text_color,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::End.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }
    }

    pub(super) fn build_corner_control_hits(
        &self,
        layout: &PanelLayoutTree,
        primary_scale_mode: PanelScaleMode,
    ) -> Vec<CornerControlHit> {
        let mut kinds = Vec::with_capacity(2);

        if !matches!(primary_scale_mode, PanelScaleMode::PercentFromBase) {
            kinds.push(CornerControlKind::PrimaryAutoscaleToggle);
        }

        kinds.push(CornerControlKind::PrimaryScaleModeCycle);

        let corner = Rectangle {
            x: layout.regions.y_axis.x,
            y: layout.regions.x_axis.y,
            width: layout.regions.y_axis.width,
            height: layout.regions.x_axis.height,
        };

        let count = kinds.len() as f32;
        let total_w = count * PANEL_CONTROL_BOX + (count - 1.0).max(0.0) * PANEL_CONTROL_GAP;
        let start_x = (corner.x + corner.width - total_w - 3.0).max(corner.x + 2.0);
        let y = corner.y + ((corner.height - PANEL_CONTROL_BOX) * 0.5).max(0.0);

        let mut hits = Vec::with_capacity(kinds.len());
        let mut x_cursor = start_x;

        for kind in kinds {
            hits.push(CornerControlHit {
                kind,
                rect: Rectangle {
                    x: x_cursor,
                    y,
                    width: PANEL_CONTROL_BOX,
                    height: PANEL_CONTROL_BOX,
                },
            });

            x_cursor += PANEL_CONTROL_BOX + PANEL_CONTROL_GAP;
        }

        hits
    }

    pub(super) fn hit_corner_control(
        controls: &[CornerControlHit],
        root_local: Point,
    ) -> Option<CornerControlHit> {
        controls
            .iter()
            .copied()
            .find(|control| PanelLayoutTree::contains(control.rect, root_local))
    }

    fn panel_control_kinds(
        &self,
        panel_index: usize,
        panel_count: usize,
        primary_panel: usize,
    ) -> Vec<PanelControlKind> {
        let mut controls = Vec::with_capacity(4);
        let is_primary = panel_index == primary_panel;

        if panel_index > 0 {
            controls.push(PanelControlKind::MoveUp);
        }

        if panel_index + 1 < panel_count {
            controls.push(PanelControlKind::MoveDown);
        }

        if !is_primary {
            controls.push(PanelControlKind::Close);
        }

        controls
    }

    pub(super) fn build_panel_control_hits(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
    ) -> Vec<PanelControlHit> {
        let panel_count = layout.panels.len();
        let mut hits = Vec::with_capacity(panel_count.saturating_mul(4));

        for (panel_index, panel) in layout.panels.iter().enumerate() {
            let title_w = self
                .resolved_panel_title(panel_index, panel.kind)
                .map(|title| title.chars().count() as f32 * CHAR_W)
                .unwrap_or(0.0);
            let title_gap = if title_w > 0.0 {
                PANEL_TITLE_TO_CONTROLS_GAP
            } else {
                0.0
            };
            let min_x = panel.plot.x + PANEL_TITLE_LEFT_PAD + title_w + title_gap;

            let mut controls = self.panel_control_kinds(panel_index, panel_count, primary_panel);
            while !controls.is_empty() {
                let count = controls.len() as f32;
                let total_w =
                    count * PANEL_CONTROL_BOX + (count - 1.0).max(0.0) * PANEL_CONTROL_GAP;
                let x = panel.plot.x + panel.plot.width - PANEL_TITLE_LEFT_PAD - total_w;
                let y = panel.plot.y + PANEL_TITLE_TOP_PAD - 1.0;

                if x < min_x {
                    controls.remove(0);
                    continue;
                }

                let mut x_cursor = x;
                for kind in controls.into_iter() {
                    hits.push(PanelControlHit {
                        panel_index,
                        kind,
                        rect: Rectangle {
                            x: x_cursor,
                            y,
                            width: PANEL_CONTROL_BOX,
                            height: PANEL_CONTROL_BOX,
                        },
                    });

                    x_cursor += PANEL_CONTROL_BOX + PANEL_CONTROL_GAP;
                }
                break;
            }
        }

        hits
    }

    pub(super) fn hit_panel_control(
        layout: &PanelLayoutTree,
        controls: &[PanelControlHit],
        root_local: Point,
    ) -> Option<PanelControlHit> {
        let plot_local = layout.plot_local_point(root_local)?;

        controls
            .iter()
            .copied()
            .find(|control| PanelLayoutTree::contains(control.rect, plot_local))
    }

    pub(super) fn control_visibility_panel(
        &self,
        layout: &PanelLayoutTree,
        root_local: Point,
    ) -> Option<usize> {
        match layout.hit_test(root_local) {
            LayoutHitZone::PanelPlot(panel_index) | LayoutHitZone::PanelXAxis(panel_index) => {
                Some(panel_index)
            }
            _ => None,
        }
    }

    pub(super) fn build_ticker_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        show_values: bool,
        show_icons: bool,
    ) -> Option<TickerLegendLayout> {
        if self.series.is_empty() {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let plot = panel.plot;

        let rows_count = self.series.len();

        let mut max_chars: usize = 0;
        let mut max_name_chars: usize = 0;

        for (index, series) in self.series.iter().enumerate() {
            let name_chars = series_display_name(series).chars().count();
            max_name_chars = max_name_chars.max(name_chars);

            let value_chars = if index == 0 {
                // Always reserve width for the primary series so its OHLC + %
                // readout stays visible by default (latest bar), matching the
                // legacy always-on top-left legend. Estimate includes the
                // change-percent segment rendered at draw time.
                series
                    .bars()
                    .last()
                    .map(|bar| {
                        let precision = series.ticker_info().min_ticksize;
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
        let icons_pack_w = if show_icons {
            2.0 * TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP
        } else {
            0.0
        };
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
        let x_right = bg.x + bg.width - TICKER_LEGEND_PADDING;

        let mut rows = Vec::with_capacity(visible_rows);
        let mut row_top = bg.y + TICKER_LEGEND_PADDING;

        for series in self.series.iter().take(visible_rows) {
            let has_close = true;
            let label_chars = series_display_name(series).chars().count() as f32;

            let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
            let text_end_x = x_left + label_chars * CHAR_W;

            let (settings, close, row_w) = if show_icons {
                let icon_pack_w = if has_close {
                    2.0 * TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP
                } else {
                    TICKER_LEGEND_ICON_BOX
                };

                let free_left = text_end_x + 8.0;
                let free_right = x_right;

                let (settings_x, close_x_opt) = if free_right - free_left >= icon_pack_w {
                    let settings_x = free_left;
                    let close_x_opt = if has_close {
                        Some(settings_x + TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP)
                    } else {
                        None
                    };
                    (settings_x, close_x_opt)
                } else if has_close {
                    let close_x = free_right - TICKER_LEGEND_ICON_BOX;
                    let settings_x =
                        (close_x - TICKER_LEGEND_ICON_GAP - TICKER_LEGEND_ICON_BOX).max(free_left);
                    (settings_x, Some(close_x))
                } else {
                    let settings_x = (free_right - TICKER_LEGEND_ICON_BOX).max(free_left);
                    (settings_x, None)
                };

                let settings = Rectangle {
                    x: settings_x,
                    y: y_center - TICKER_LEGEND_ICON_BOX * 0.5,
                    width: TICKER_LEGEND_ICON_BOX,
                    height: TICKER_LEGEND_ICON_BOX,
                };

                let close = if let Some(close_x) = close_x_opt {
                    Rectangle {
                        x: close_x,
                        y: y_center - TICKER_LEGEND_ICON_BOX * 0.5,
                        width: TICKER_LEGEND_ICON_BOX,
                        height: TICKER_LEGEND_ICON_BOX,
                    }
                } else {
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    }
                };

                let content_right = if has_close {
                    close.x + close.width
                } else {
                    settings.x + settings.width
                };

                let row_w = (content_right + TICKER_LEGEND_PADDING - bg.x).clamp(0.0, bg.width);

                (settings, close, row_w)
            } else {
                let hidden = Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                };
                let row_w = (text_end_x + TICKER_LEGEND_PADDING - bg.x).clamp(0.0, bg.width);

                (hidden, hidden, row_w)
            };

            rows.push(TickerLegendRowHit {
                ticker: *series.ticker_info(),
                y_center,
                row_rect: Rectangle {
                    x: bg.x,
                    y: row_top,
                    width: row_w,
                    height: TICKER_LEGEND_ROW_H,
                },
                settings,
                close,
                has_close,
            });

            row_top += TICKER_LEGEND_ROW_H;
        }

        Some(TickerLegendLayout { bg, rows })
    }

    pub(super) fn hit_ticker_legend(
        layout: &PanelLayoutTree,
        legend: &TickerLegendLayout,
        root_local: Point,
    ) -> Option<TickerLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        if !legend.bg.contains(plot_local) {
            return None;
        }

        for (index, row) in legend.rows.iter().enumerate() {
            if !row.row_rect.contains(plot_local) {
                continue;
            }

            if row.settings.contains(plot_local) {
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Settings));
            }

            if row.has_close && row.close.contains(plot_local) {
                let kind = if index == 0 {
                    TickerLegendIconKind::Hide
                } else {
                    TickerLegendIconKind::Close
                };
                return Some(TickerLegendHit::Icon(index, kind));
            }

            return Some(TickerLegendHit::Row(index));
        }

        Some(TickerLegendHit::Background)
    }

    pub(super) fn build_footprint_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
        cluster_kind: ClusterKind,
        active: bool,
    ) -> Option<FootprintLegendLayout> {
        if !active {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let row_top = panel.plot.y
            + PANEL_TITLE_TOP_PAD
            + TICKER_LEGEND_PADDING
            + preceding_rows as f32 * TICKER_LEGEND_ROW_H;
        let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
        let label = format!("Footprint · {cluster_kind}");
        let text_end_x =
            panel.plot.x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
        let (_, settings, hide, row_w) =
            self.indicator_legend_controls(panel.plot, text_end_x, y_center, false, true);

        Some(FootprintLegendLayout {
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

    pub(super) fn hit_footprint_legend(
        layout: &PanelLayoutTree,
        legend: &FootprintLegendLayout,
        root_local: Point,
    ) -> Option<FootprintLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        if !legend.row.row_rect.contains(plot_local) {
            return None;
        }
        if legend.row.settings.contains(plot_local) {
            return Some(FootprintLegendHit::Icon(FootprintLegendIconKind::Settings));
        }
        if legend.row.hide.contains(plot_local) {
            return Some(FootprintLegendHit::Icon(FootprintLegendIconKind::Hide));
        }
        Some(FootprintLegendHit::Row)
    }

    fn indicator_label(
        &self,
        panel_index: usize,
        primary_overlay: bool,
        value_id: super::PanelValueId,
    ) -> String {
        if primary_overlay {
            return self
                .overlay_channels_for_panel_value(Some(value_id))
                .iter()
                .map(|channel| channel.label)
                .collect::<Vec<_>>()
                .join(" ");
        }

        self.resolved_panel_title(panel_index, super::KlinePanelKind::Indicator)
            .unwrap_or("Indicator")
            .to_string()
    }

    pub(super) fn tooltip_rows_before_indicators(
        &self,
        ticker_legend: Option<&TickerLegendLayout>,
        footprint_legend: Option<&FootprintLegendLayout>,
        show_legend_values: bool,
    ) -> usize {
        ticker_legend
            .map(|legend| legend.rows.len())
            .unwrap_or(usize::from(show_legend_values))
            + usize::from(footprint_legend.is_some())
    }

    pub(super) fn build_indicator_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
    ) -> Option<IndicatorLegendLayout> {
        let primary_overlay_entries = self.primary_overlay_entries();
        let has_primary_overlays = !primary_overlay_entries.is_empty();
        let has_indicator_panels = layout.panels.iter().enumerate().any(|(index, panel)| {
            panel.kind == super::KlinePanelKind::Indicator && self.panel_value_id(index).is_some()
        });

        if !has_primary_overlays && !has_indicator_panels {
            return None;
        }

        let mut rows = Vec::new();

        for (panel_index, panel) in layout.panels.iter().enumerate() {
            if panel.kind != super::KlinePanelKind::Indicator {
                continue;
            }

            let Some(value_id) = self.panel_value_id(panel_index) else {
                continue;
            };

            let label = self.indicator_label(panel_index, false, value_id);
            let x_left = self.indicator_label_x(panel.plot.x, value_id);
            let text_end_x = x_left + label.chars().count() as f32 * CHAR_W;
            let y_center = panel.plot.y + PANEL_TITLE_TOP_PAD + TICKER_LEGEND_ROW_H * 0.5;
            let (visibility, settings, close, row_w) =
                self.indicator_legend_controls(panel.plot, text_end_x, y_center, false, true);

            rows.push(IndicatorLegendRowHit {
                value_id,
                y_center,
                row_rect: Rectangle {
                    x: panel.plot.x,
                    y: y_center - TICKER_LEGEND_ROW_H * 0.5,
                    width: row_w,
                    height: TICKER_LEGEND_ROW_H,
                },
                visibility,
                settings,
                close,
                visible: true,
                primary_overlay: false,
            });
        }

        if has_primary_overlays {
            let panel = layout.panel(primary_panel)?;
            for (overlay_index, (value_id, visible)) in primary_overlay_entries.iter().enumerate() {
                let row_top = panel.plot.y
                    + PANEL_TITLE_TOP_PAD
                    + TICKER_LEGEND_PADDING
                    + (preceding_rows + overlay_index) as f32 * TICKER_LEGEND_ROW_H;
                let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
                let label = self.indicator_label(primary_panel, true, *value_id);
                let x_left = self.indicator_label_x(panel.plot.x, *value_id);
                let text_end_x = x_left + label.chars().count() as f32 * CHAR_W;
                let (visibility, settings, close, row_w) =
                    self.indicator_legend_controls(panel.plot, text_end_x, y_center, true, true);

                rows.push(IndicatorLegendRowHit {
                    value_id: *value_id,
                    y_center,
                    row_rect: Rectangle {
                        x: panel.plot.x,
                        y: row_top,
                        width: row_w,
                        height: TICKER_LEGEND_ROW_H,
                    },
                    visibility,
                    settings,
                    close,
                    visible: *visible,
                    primary_overlay: true,
                });
            }
        }

        Some(IndicatorLegendLayout { rows })
    }

    fn indicator_legend_controls(
        &self,
        panel: Rectangle,
        text_end_x: f32,
        y_center: f32,
        has_visibility: bool,
        has_close: bool,
    ) -> (Rectangle, Rectangle, Rectangle, f32) {
        let x_right = panel.x + panel.width - TICKER_LEGEND_PADDING;
        let free_left = text_end_x + TICKER_LEGEND_PADDING * 2.0;
        let icon_count = 1 + usize::from(has_visibility) + usize::from(has_close);
        let icon_pack_w = icon_count as f32 * TICKER_LEGEND_ICON_BOX
            + icon_count.saturating_sub(1) as f32 * TICKER_LEGEND_ICON_GAP;

        let settings_x = free_left.min((x_right - icon_pack_w).max(free_left));
        let first_icon = Rectangle {
            x: settings_x,
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
        let mut next_x = settings_x;
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
        let settings = Rectangle {
            x: next_x,
            ..first_icon
        };
        next_x += TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP;
        let close = if has_close {
            Rectangle {
                x: next_x,
                ..first_icon
            }
        } else {
            empty_icon()
        };
        let content_right = if has_close {
            close.x + close.width
        } else {
            settings.x + settings.width
        };
        let row_w = (content_right + TICKER_LEGEND_PADDING - panel.x).clamp(0.0, panel.width);
        (visibility, settings, close, row_w)
    }

    pub(super) fn hit_indicator_legend(
        layout: &PanelLayoutTree,
        legend: &IndicatorLegendLayout,
        root_local: Point,
    ) -> Option<IndicatorLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        for (index, row) in legend.rows.iter().enumerate() {
            if !row.row_rect.contains(plot_local) {
                continue;
            }

            if row.visibility.contains(plot_local) {
                return Some(IndicatorLegendHit::Icon(
                    index,
                    IndicatorLegendIconKind::Visibility,
                ));
            }

            if row.settings.contains(plot_local) {
                return Some(IndicatorLegendHit::Icon(
                    index,
                    IndicatorLegendIconKind::Settings,
                ));
            }

            if row.close.contains(plot_local) {
                return Some(IndicatorLegendHit::Icon(
                    index,
                    IndicatorLegendIconKind::Close,
                ));
            }

            return Some(IndicatorLegendHit::Row(index));
        }

        None
    }

    pub(super) fn fill_panel_header_values(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
        show_primary_panel_values: bool,
    ) {
        let Some(cursor) = scene.interaction.cursor else {
            return;
        };

        let Some(base_series) = self.series.first() else {
            return;
        };

        let Some(base_bar) = self.bar_at_or_before_unit(base_series, scene.x.axis, cursor.x_unit)
        else {
            return;
        };

        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let title_w = self
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
            let panel_precision = self.panel_value_precision(panel_index);

            let mut max_x = scene.layout.regions.plot.x + panel.plot.x + panel.plot.width
                - PANEL_TITLE_LEFT_PAD;
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

            if x >= max_x {
                continue;
            }

            if panel_index == scene.primary.panel_index {
                if !show_primary_panel_values {
                    continue;
                }

                let precision = base_series.ticker_info().min_ticksize;

                if matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                    let open_f = base_bar.open.to_f32_lossy();
                    let close_f = base_bar.close.to_f32_lossy();
                    let change_pct = if open_f.abs() > f32::EPSILON {
                        ((close_f - open_f) / open_f) * 100.0
                    } else {
                        0.0
                    };

                    let slot = if change_pct >= 0.0 {
                        data::chart::kline_v2::CandlestickColorSlot::Bullish
                    } else {
                        data::chart::kline_v2::CandlestickColorSlot::Bearish
                    };
                    let value_color = Self::resolve_theme_color(
                        self.candlestick_config.colors.color_for(slot),
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

                        frame.fill_text(canvas::Text {
                            content: text.clone(),
                            position: Point::new(x, y),
                            color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        x += text.chars().count() as f32 * CHAR_W;
                        x += if is_value { 6.0 } else { 2.0 };
                    }
                } else {
                    let text = format!("C {}", base_bar.close.to_string(precision));
                    frame.fill_text(canvas::Text {
                        content: text.clone(),
                        position: Point::new(x, y),
                        color: palette.background.base.text.scale_alpha(0.85),
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            } else if let Some(value_id) = self.panel_value_id(panel_index)
                && let Some(indicator_data) =
                    self.indicator_data_for_series(base_series, Some(value_id), base_bar)
            {
                if matches!(
                    self.composition
                        .panels
                        .get(panel_index)
                        .map(|panel| panel.presentation),
                    Some(super::PanelPresentation::Table)
                ) {
                    continue;
                }

                let hovered_indicator = scene.interaction.hovering_indicator_legend
                    && scene
                        .chrome
                        .indicator_legend
                        .as_ref()
                        .and_then(|legend| {
                            scene
                                .interaction
                                .hovered_indicator_row
                                .and_then(|index| legend.rows.get(index))
                        })
                        .map(|row| row.value_id == value_id)
                        .unwrap_or(false);

                if hovered_indicator {
                    continue;
                }

                let mut drew_any_channel = false;

                for channel in self
                    .overlay_channels_for_panel_value(Some(value_id))
                    .iter()
                    .copied()
                {
                    let Some(value) = self.overlay_channel_value(value_id, indicator_data, channel)
                    else {
                        continue;
                    };

                    if x >= max_x {
                        break;
                    }

                    let value_text =
                        self.format_panel_header_value(panel_index, panel_precision, value);
                    let label_color = self.overlay_channel_color(
                        Some(value_id),
                        channel,
                        palette,
                        generated_accents,
                    );
                    let label_text = channel.label.to_string();

                    frame.fill_text(canvas::Text {
                        content: label_text.clone(),
                        position: Point::new(x, y),
                        color: label_color,
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });

                    x += label_text.chars().count() as f32 * CHAR_W + 2.0;

                    if x >= max_x {
                        break;
                    }

                    frame.fill_text(canvas::Text {
                        content: value_text.clone(),
                        position: Point::new(x, y),
                        color: label_color,
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });

                    x += value_text.chars().count() as f32 * CHAR_W + 6.0;
                    drew_any_channel = true;
                }

                if !drew_any_channel {
                    let value = indicator_data.value();
                    let text = self.format_panel_header_value(panel_index, panel_precision, value);

                    frame.fill_text(canvas::Text {
                        content: text,
                        position: Point::new(x, y),
                        color: palette.background.base.text.scale_alpha(0.82),
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            }
        }
    }

    fn draw_legend_icon_button(
        frame: &mut canvas::Frame,
        rect: Rectangle,
        icon: style::Icon,
        hovered: bool,
        palette: &Extended,
    ) {
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

    pub(super) fn fill_primary_ticker_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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

        let bg = to_root(legend.bg);
        frame.fill_rectangle(
            bg.position(),
            bg.size(),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        let target_x_unit = scene.interaction.cursor.map(|cursor| cursor.x_unit);
        let show_buttons = scene.interaction.hovering_ticker_legend;
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.row_rect);
            let hovered_row = scene.interaction.hovered_ticker_row == Some(index);

            if show_buttons && hovered_row {
                let highlight = Rectangle {
                    x: row_rect.x + 1.0,
                    y: row_rect.y,
                    width: (row_rect.width - 2.0).max(0.0),
                    height: row_rect.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            let label = self
                .series
                .get(index)
                .map(series_display_name)
                .unwrap_or_else(|| row.ticker.ticker.symbol_and_exchange_string());
            let row_alpha = if index == 0 && !self.candlesticks_visible {
                HIDDEN_LEGEND_ALPHA
            } else {
                1.0
            };
            let label_color = if index == 0 && !matches!(scene.primary.mark, MarkKind::Line) {
                palette.background.base.text
            } else {
                self.series
                    .get(index)
                    .map(|series| self.series_line_color(series, palette))
                    .unwrap_or(palette.background.base.text)
            }
            .scale_alpha(if index == 0 { 1.0 } else { 0.96 });

            // Build the colored, segmented text for this row. The primary
            // series shows O/H/L/C + change% colored green/red by direction
            // (close vs open), persisting on the latest bar and switching to
            // the hovered bar when the cursor is over the plot.
            let mut segments: Vec<(String, iced::Color)> = Vec::new();

            if let Some(series) = self.series.get(index) {
                let effective_bar = if !show_buttons {
                    if let Some(target_x_unit) = target_x_unit {
                        self.bar_at_or_before_unit(series, scene.x.axis, target_x_unit)
                    } else {
                        series.bars().last()
                    }
                } else {
                    None
                };

                if index == 0 {
                    if let Some(bar) = effective_bar {
                        let precision = series.ticker_info().min_ticksize;
                        let open_f = bar.open.to_f32_lossy();
                        let close_f = bar.close.to_f32_lossy();
                        let change_pct = if open_f.abs() > f32::EPSILON {
                            ((close_f - open_f) / open_f) * 100.0
                        } else {
                            0.0
                        };
                        let slot = if change_pct >= 0.0 {
                            data::chart::kline_v2::CandlestickColorSlot::Bullish
                        } else {
                            data::chart::kline_v2::CandlestickColorSlot::Bearish
                        };
                        let value_color = Self::resolve_theme_color(
                            self.candlestick_config.colors.color_for(slot),
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
                    let pct_text = self
                        .bar_at_or_before_unit(series, scene.x.axis, scene.x.min_unit)
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

            let mut text_x = bg.x + TICKER_LEGEND_PADDING;
            let text_y = scene.layout.regions.plot.y + row.y_center;

            for (text, color) in segments {
                frame.fill_text(canvas::Text {
                    content: text.clone(),
                    position: Point::new(text_x, text_y),
                    color: color.scale_alpha(row_alpha),
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
                text_x += text.chars().count() as f32 * CHAR_W;
            }

            if show_buttons {
                let settings_root = to_root(row.settings);
                let close_root = to_root(row.close);

                let settings_hovered = scene.interaction.hovered_ticker_icon
                    == Some((index, TickerLegendIconKind::Settings));
                Self::draw_legend_icon_button(
                    frame,
                    settings_root,
                    TickerLegendIconKind::Settings.icon(),
                    settings_hovered,
                    palette,
                );

                if row.has_close {
                    let secondary_kind = if index == 0 {
                        TickerLegendIconKind::Hide
                    } else {
                        TickerLegendIconKind::Close
                    };
                    let close_hovered =
                        scene.interaction.hovered_ticker_icon == Some((index, secondary_kind));
                    Self::draw_legend_icon_button(
                        frame,
                        close_root,
                        secondary_kind.icon(),
                        close_hovered,
                        palette,
                    );
                }
            }
        }
    }

    pub(super) fn fill_footprint_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
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
        let hovered_row = scene.interaction.hovering_footprint_legend;
        frame.fill_rectangle(
            row_rect.position(),
            row_rect.size(),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        if hovered_row {
            frame.fill_rectangle(
                row_rect.position(),
                row_rect.size(),
                palette.background.strong.color.scale_alpha(0.22),
            );
        }

        frame.fill_text(canvas::Text {
            content: format!("Footprint · {}", legend.cluster_kind),
            position: Point::new(
                row_rect.x + TOOLTIP_ROW_LEFT_PAD,
                row_rect.y + row_rect.height * 0.5,
            ),
            color: palette
                .background
                .base
                .text
                .scale_alpha(if self.footprints_visible {
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
        let settings_hovered =
            scene.interaction.hovered_footprint_icon == Some(FootprintLegendIconKind::Settings);
        Self::draw_legend_icon_button(
            frame,
            settings,
            FootprintLegendIconKind::Settings.icon(),
            settings_hovered,
            palette,
        );
        let hide_hovered =
            scene.interaction.hovered_footprint_icon == Some(FootprintLegendIconKind::Hide);
        Self::draw_legend_icon_button(
            frame,
            hide,
            FootprintLegendIconKind::Hide.icon(),
            hide_hovered,
            palette,
        );
    }

    pub(super) fn fill_indicator_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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
        let show_buttons = scene.interaction.hovering_indicator_legend;
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.row_rect);
            let hovered_row = scene.interaction.hovered_indicator_row == Some(index);

            if show_buttons && hovered_row {
                let highlight = Rectangle {
                    x: row_rect.x + 1.0,
                    y: row_rect.y,
                    width: (row_rect.width - 2.0).max(0.0),
                    height: row_rect.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            if row.primary_overlay {
                let marker_x = self.indicator_marker_x(row_rect.x);
                let label_x = self.indicator_label_x(row_rect.x, row.value_id);
                let label_y = scene.layout.regions.plot.y + row.y_center;
                let row_alpha = if row.visible {
                    1.0
                } else {
                    HIDDEN_LEGEND_ALPHA
                };

                if self
                    .indicator_unavailable_message(Some(row.value_id))
                    .is_some()
                {
                    frame.fill_text(canvas::Text {
                        content: "!".to_string(),
                        position: Point::new(marker_x, label_y),
                        color: palette.danger.base.color.scale_alpha(row_alpha),
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }

                let base_bar = scene.interaction.cursor.and_then(|cursor| {
                    self.series.first().and_then(|series| {
                        self.bar_at_or_before_unit(series, scene.x.axis, cursor.x_unit)
                    })
                });
                let primary_anchor = scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten();
                let precision = self.panel_value_precision(scene.primary.panel_index);
                let mut x = label_x;

                for channel in self
                    .overlay_channels_for_panel_value(Some(row.value_id))
                    .iter()
                    .copied()
                {
                    let label_text = channel.label.to_string();
                    let color = self
                        .overlay_channel_color(
                            Some(row.value_id),
                            channel,
                            palette,
                            generated_accents,
                        )
                        .scale_alpha(row_alpha);

                    frame.fill_text(canvas::Text {
                        content: label_text.clone(),
                        position: Point::new(x, label_y),
                        color,
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                    x += label_text.chars().count() as f32 * CHAR_W + 2.0;

                    if show_buttons {
                        continue;
                    }

                    let Some(value) = base_bar.and_then(|bar| {
                        self.series.first().and_then(|series| {
                            self.indicator_data_for_series(series, Some(row.value_id), bar)
                                .and_then(|data| {
                                    self.overlay_channel_value(row.value_id, data, channel)
                                })
                        })
                    }) else {
                        continue;
                    };

                    let value_text =
                        if matches!(scene.primary.mode, PanelScaleMode::PercentFromBase)
                            && primary_anchor
                                .map(|anchor| anchor.abs() > f32::EPSILON)
                                .unwrap_or(false)
                        {
                            let anchor = primary_anchor.unwrap_or(1.0);
                            format!("{:+.2}%", ((value / anchor) - 1.0) * 100.0)
                        } else {
                            self.format_panel_header_value(
                                scene.primary.panel_index,
                                precision,
                                value,
                            )
                        };

                    frame.fill_text(canvas::Text {
                        content: value_text.clone(),
                        position: Point::new(x, label_y),
                        color,
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                    x += value_text.chars().count() as f32 * CHAR_W + 6.0;
                }
            }

            if show_buttons && hovered_row {
                let visibility_hovered = scene.interaction.hovered_indicator_icon
                    == Some((index, IndicatorLegendIconKind::Visibility));
                Self::draw_legend_icon_button(
                    frame,
                    to_root(row.visibility),
                    IndicatorLegendIconKind::Visibility.icon(row.visible),
                    visibility_hovered,
                    palette,
                );

                let settings_hovered = scene.interaction.hovered_indicator_icon
                    == Some((index, IndicatorLegendIconKind::Settings));
                Self::draw_legend_icon_button(
                    frame,
                    to_root(row.settings),
                    IndicatorLegendIconKind::Settings.icon(row.visible),
                    settings_hovered,
                    palette,
                );

                let close_hovered = scene.interaction.hovered_indicator_icon
                    == Some((index, IndicatorLegendIconKind::Close));
                Self::draw_legend_icon_button(
                    frame,
                    to_root(row.close),
                    IndicatorLegendIconKind::Close.icon(row.visible),
                    close_hovered,
                    palette,
                );
            }
        }
    }

    pub(super) fn fill_corner_controls(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        for control in scene.chrome.corner_controls.iter().copied() {
            let hovered = scene
                .interaction
                .hovered_corner_control
                .map(|hit| hit.kind == control.kind)
                .unwrap_or(false);

            let active = control
                .kind
                .is_active(scene.primary.mode, self.primary_autoscale);

            let fill_color = if hovered {
                if active {
                    palette.primary.base.color.scale_alpha(0.42)
                } else {
                    palette.background.strong.color
                }
            } else if active {
                palette.primary.base.color.scale_alpha(0.24)
            } else {
                palette.background.base.color.scale_alpha(0.72)
            };

            let text_color = if active {
                palette.primary.base.text
            } else if hovered {
                palette.background.strong.text
            } else {
                palette.background.base.text.scale_alpha(0.86)
            };

            frame.fill_rectangle(
                Point::new(control.rect.x, control.rect.y),
                Size::new(control.rect.width, control.rect.height),
                fill_color,
            );

            frame.fill_text(canvas::Text {
                content: control.kind.label(scene.primary.mode).to_string(),
                position: Point::new(
                    control.rect.x + control.rect.width * 0.5,
                    control.rect.y + control.rect.height * 0.5,
                ),
                color: text_color,
                size: PANEL_CONTROL_ICON_SIZE.into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    pub(super) fn fill_panel_controls(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(visible_panel) = scene.interaction.controls_visible_for_panel else {
            return;
        };

        for control in scene
            .chrome
            .panel_controls
            .iter()
            .copied()
            .filter(|control| control.panel_index == visible_panel)
        {
            let hovered = scene
                .interaction
                .hovered_control
                .map(|hit| hit.panel_index == control.panel_index && hit.kind == control.kind)
                .unwrap_or(false);

            let is_close = matches!(control.kind, PanelControlKind::Close);

            let fill_color = if hovered && is_close {
                palette.danger.base.color.scale_alpha(0.22)
            } else if hovered {
                palette.background.strong.color
            } else {
                palette.background.base.color.scale_alpha(0.72)
            };

            let text_color = if hovered && is_close {
                palette.danger.base.text
            } else if hovered {
                palette.background.strong.text
            } else {
                palette.background.base.text.scale_alpha(0.86)
            };

            let x = scene.layout.regions.plot.x + control.rect.x;
            let y = scene.layout.regions.plot.y + control.rect.y;

            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(control.rect.width, control.rect.height),
                fill_color,
            );

            frame.fill_text(canvas::Text {
                content: control.kind.icon().to_string(),
                position: Point::new(x + control.rect.width / 2.0, y + control.rect.height / 2.0),
                color: text_color,
                size: PANEL_CONTROL_ICON_SIZE.into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }
}

struct SeriesEndLabel {
    y: f32,
    color: iced::Color,
    text_color: iced::Color,
    text: String,
    value: Option<String>,
}

fn resolve_series_end_label_overlaps(labels: &mut [SeriesEndLabel], min_y: f32, max_y: f32) {
    if labels.len() <= 1 {
        return;
    }

    labels.sort_by(|left, right| left.y.total_cmp(&right.y));
    let half_height = TEXT_SIZE * 0.5 + SERIES_END_LABEL_GAP;
    let available_height = (max_y - min_y).max(0.0);
    let label_count = labels.len();
    let spacing = (available_height / (label_count - 1) as f32).min(TEXT_SIZE + 4.0);

    for (index, label) in labels.iter_mut().enumerate() {
        let lower = min_y + half_height + index as f32 * spacing;
        let upper = max_y - half_height - (label_count - 1 - index) as f32 * spacing;
        label.y = label.y.clamp(lower.min(upper), lower.max(upper));
    }
}
