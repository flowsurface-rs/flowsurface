use super::PanelInteraction;
use super::composition::{MarkKind, PanelScaleMode};
use super::layout::{
    LayoutHitZone, PanelLayoutTree, contains_inclusive, place_non_overlapping_labels,
};
use super::scene::Scene;
use super::{
    CHAR_W, KlineWidget, KlineWidgetEvent, PANEL_TITLE_TOP_PAD, TEXT_SIZE, TICKER_LEGEND_PADDING,
    TICKER_LEGEND_ROW_H, TOOLTIP_ROW_LEFT_PAD,
};
use crate::style;
use data::chart::kline::KlineStudyId;
use data::chart::kline::series::KlineSeries;
use data::chart::kline_data::ClusterKind;
use data::config::theme::ACCENT_COUNT;

use exchange::TickerInfo;

use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 8.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = 0.0;
const SERIES_END_LABEL_GAP: f32 = 1.0;
const HIDDEN_LEGEND_ALPHA: f32 = 0.45;
const LIGHT_LEGEND_BACKGROUND_ALPHA: f32 = 0.7;
const DARK_LEGEND_BACKGROUND_ALPHA: f32 = 0.9;
const STUDY_LEGEND_INDENT: f32 = 8.0;
const STUDY_COUNT_BADGE_GAP: f32 = 4.0;
const STUDY_COUNT_BADGE_HORIZONTAL_PAD: f32 = 4.0;
const STUDY_COUNT_BADGE_HEIGHT: f32 = TEXT_SIZE + 2.0;

enum LegendButtonContent {
    Icon(style::Icon),
    Text(&'static str),
}

fn series_display_name(series: &KlineSeries) -> String {
    series.ticker_info.ticker.symbol_and_exchange_string()
}

fn legend_background_color(palette: &Extended) -> Color {
    let alpha = if palette.is_dark {
        DARK_LEGEND_BACKGROUND_ALPHA
    } else {
        LIGHT_LEGEND_BACKGROUND_ALPHA
    };

    palette.background.weakest.color.scale_alpha(alpha)
}

fn legend_row_background(row: Rectangle, content_end_x: f32) -> Rectangle {
    Rectangle {
        x: row.x + TICKER_LEGEND_PADDING,
        y: row.y,
        width: (content_end_x - row.x).max(0.0),
        height: row.height,
    }
}

fn control_colors(palette: &Extended, hovered: bool, active: bool) -> (Color, Color) {
    let fill_color = if hovered && active {
        palette.primary.base.color
    } else if hovered || active {
        palette.primary.weak.color
    } else {
        palette.background.weak.color
    };

    let text_color = if hovered && active {
        palette.primary.base.text
    } else if hovered || active {
        palette.primary.strong.text
    } else {
        palette.background.strong.text
    };

    (fill_color, text_color)
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
    fn icon(self, _visible: bool) -> LegendButtonContent {
        match self {
            Self::Settings => LegendButtonContent::Icon(style::Icon::Cog),
            Self::Hide => LegendButtonContent::Text("○"),
            Self::Close => LegendButtonContent::Icon(style::Icon::Close),
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
    Row(usize),
    Icon(usize, TickerLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FootprintLegendIconKind {
    Settings,
    Hide,
}

impl FootprintLegendIconKind {
    fn icon(self, _visible: bool) -> LegendButtonContent {
        match self {
            Self::Settings => LegendButtonContent::Icon(style::Icon::Cog),
            Self::Hide => LegendButtonContent::Text("○"),
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
pub(super) enum StudiesLegendIconKind {
    ExpandCollapse,
    Visibility,
    Close,
}

impl StudiesLegendIconKind {
    fn icon(self, expanded: bool) -> LegendButtonContent {
        match self {
            Self::ExpandCollapse => LegendButtonContent::Text(if expanded { "v" } else { ">" }),
            Self::Visibility => LegendButtonContent::Text("○"),
            Self::Close => LegendButtonContent::Icon(style::Icon::Close),
        }
    }

    pub(super) fn into_event(self) -> KlineWidgetEvent {
        match self {
            Self::ExpandCollapse => KlineWidgetEvent::StudiesExpansionToggled,
            Self::Visibility => KlineWidgetEvent::StudiesVisibilityToggled,
            Self::Close => KlineWidgetEvent::StudiesRemove,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StudiesLegendRowHit {
    pub(super) row_rect: Rectangle,
    pub(super) expand: Rectangle,
    pub(super) visibility: Rectangle,
    pub(super) close: Rectangle,
    pub(super) count_badge: Rectangle,
    pub(super) expanded: bool,
    pub(super) visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StudyLegendIconKind {
    Visibility,
    Settings,
    Close,
}

impl StudyLegendIconKind {
    fn icon(self, _visible: bool) -> LegendButtonContent {
        match self {
            Self::Visibility => LegendButtonContent::Text("○"),
            Self::Settings => LegendButtonContent::Icon(style::Icon::Cog),
            Self::Close => LegendButtonContent::Icon(style::Icon::Close),
        }
    }

    pub(super) fn into_event(self, study_id: KlineStudyId) -> KlineWidgetEvent {
        match self {
            Self::Visibility => KlineWidgetEvent::StudyVisibilityToggled(study_id),
            Self::Settings => KlineWidgetEvent::StudySettings(study_id),
            Self::Close => KlineWidgetEvent::StudyRemove(study_id),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StudyLegendRowHit {
    pub(super) study_id: KlineStudyId,
    pub(super) row_rect: Rectangle,
    pub(super) visibility: Rectangle,
    pub(super) settings: Rectangle,
    pub(super) close: Rectangle,
    pub(super) visible: bool,
}

#[derive(Debug, Clone)]
pub(super) struct StudyLegendLayout {
    pub(super) category: StudiesLegendRowHit,
    pub(super) study_count: usize,
    pub(super) rows: Vec<StudyLegendRowHit>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum StudyLegendHit {
    CategoryRow,
    CategoryIcon(StudiesLegendIconKind),
    StudyRow(usize),
    StudyIcon(usize, StudyLegendIconKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorLegendIconKind {
    Visibility,
    Settings,
    Close,
}

impl IndicatorLegendIconKind {
    fn icon(self, _visible: bool) -> LegendButtonContent {
        match self {
            Self::Visibility => LegendButtonContent::Text("○"),
            Self::Settings => LegendButtonContent::Icon(style::Icon::Cog),
            Self::Close => LegendButtonContent::Icon(style::Icon::Close),
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

impl<'a> KlineWidget<'a> {
    pub(super) fn fill_primary_series_end_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let primary_plot = scene.primary_plot();
        let plot_origin = scene.layout.regions.plot;
        let split_x = plot_origin.x + primary_plot.x + primary_plot.width;
        let axis_width = scene.layout.regions.y_axis.width;

        for (index, label_y) in self.primary_series_end_label_positions(scene) {
            let Some(series) = self.series.get(index) else {
                continue;
            };
            let Some(bar) = self.bar_at_or_before_unit(series, scene.x.axis, scene.x.max_unit)
            else {
                continue;
            };
            let color = iced::Color {
                a: 1.0,
                ..self.series_line_color(series, palette, generated_accents)
            };
            let text_color = if data::config::theme::is_dark(color) {
                iced::Color::WHITE
            } else {
                iced::Color::BLACK
            };
            let label = series_display_name(series);
            let label_height = TEXT_SIZE + 4.0;
            let label_width = label.chars().count() as f32 * CHAR_W + 8.0;
            let rect = Rectangle {
                x: split_x - label_width,
                y: label_y - label_height * 0.5,
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
            let value_rect = Rectangle {
                x: split_x,
                y: rect.y,
                width: axis_width,
                height: rect.height,
            };
            frame.fill_rectangle(value_rect.position(), value_rect.size(), color);
            frame.fill_text(canvas::Text {
                content: value,
                position: Point::new(value_rect.x + 4.0, label_y),
                color: text_color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    pub(super) fn primary_series_end_label_positions(&self, scene: &Scene) -> Vec<(usize, f32)> {
        if self.series.len() < 2 {
            return Vec::new();
        }

        let primary_plot = scene.primary_plot();
        let plot_origin = scene.layout.regions.plot;
        let min_y = plot_origin.y + primary_plot.y;
        let max_y = min_y + primary_plot.height;
        let desired = self
            .series
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, series)| {
                let bar = self.bar_at_or_before_unit(series, scene.x.axis, scene.x.max_unit)?;
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
            .cloned()
            .find(|control| contains_inclusive(control.rect, root_local))
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
            .find(|control| contains_inclusive(control.rect, plot_local))
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
        let icons_pack_w = 2.0 * TICKER_LEGEND_ICON_BOX + TICKER_LEGEND_ICON_GAP;
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

            let (settings, close, row_w) = {
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
                    settings.x.max(close.x) + close.width
                } else {
                    settings.x + settings.width
                };

                let row_w = (content_right + TICKER_LEGEND_PADDING - bg.x).clamp(0.0, bg.width);

                if show_icons {
                    (settings, close, row_w)
                } else {
                    let hidden = Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    };
                    (hidden, hidden, row_w)
                }
            };

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

        None
    }

    pub(super) fn build_footprint_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
        cluster_kind: ClusterKind,
    ) -> Option<FootprintLegendLayout> {
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
            Self::legend_controls(panel.plot, text_end_x, y_center, false, false, true, true);

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

    pub(super) fn tooltip_rows_before_studies(
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

    pub(super) fn build_study_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
        preceding_rows: usize,
        expanded: bool,
    ) -> Option<StudyLegendLayout> {
        let study_count = self.studies.len();
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
            Self::legend_controls(
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
            expanded,
            visible: self.studies.iter().all(|study| study.visible()),
        };
        let mut rows = Vec::new();

        if expanded {
            for (index, study) in self.studies.iter().enumerate() {
                let label = study.to_string();
                let row_top = panel.plot.y
                    + PANEL_TITLE_TOP_PAD
                    + TICKER_LEGEND_PADDING
                    + (preceding_rows + 1 + index) as f32 * TICKER_LEGEND_ROW_H;
                let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
                let row_x = panel.plot.x + STUDY_LEGEND_INDENT;
                let text_end_x =
                    row_x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
                let (_, visibility, settings, close, row_w) = Self::legend_controls(
                    panel.plot, text_end_x, y_center, false, true, true, true,
                );

                rows.push(StudyLegendRowHit {
                    study_id: study.id(),
                    row_rect: Rectangle {
                        x: row_x,
                        y: row_top,
                        width: row_w,
                        height: TICKER_LEGEND_ROW_H,
                    },
                    visibility,
                    settings,
                    close,
                    visible: study.visible(),
                });
            }
        }

        Some(StudyLegendLayout {
            category,
            study_count,
            rows,
        })
    }

    pub(super) fn hit_study_legend(
        layout: &PanelLayoutTree,
        legend: &StudyLegendLayout,
        root_local: Point,
    ) -> Option<StudyLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;

        if legend.category.row_rect.contains(plot_local) {
            if legend.category.expand.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(
                    StudiesLegendIconKind::ExpandCollapse,
                ));
            }

            if legend.category.visibility.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(
                    StudiesLegendIconKind::Visibility,
                ));
            }

            if legend.category.close.contains(plot_local) {
                return Some(StudyLegendHit::CategoryIcon(StudiesLegendIconKind::Close));
            }

            return Some(StudyLegendHit::CategoryRow);
        }

        for (index, row) in legend.rows.iter().enumerate() {
            if !row.row_rect.contains(plot_local) {
                continue;
            }

            if row.visibility.contains(plot_local) {
                return Some(StudyLegendHit::StudyIcon(
                    index,
                    StudyLegendIconKind::Visibility,
                ));
            }

            if row.settings.contains(plot_local) {
                return Some(StudyLegendHit::StudyIcon(
                    index,
                    StudyLegendIconKind::Settings,
                ));
            }

            if row.close.contains(plot_local) {
                return Some(StudyLegendHit::StudyIcon(index, StudyLegendIconKind::Close));
            }

            return Some(StudyLegendHit::StudyRow(index));
        }

        None
    }

    fn indicator_title(
        &self,
        panel_index: usize,
        primary_overlay: bool,
        value_id: super::PanelValueId,
    ) -> String {
        if primary_overlay || self.panel_indicator_value_ids(panel_index).len() > 1 {
            return self
                .indicator_kind(value_id)
                .map(|indicator| indicator.to_string())
                .unwrap_or_else(|| "Indicator".to_string());
        }

        self.resolved_panel_title(panel_index, super::KlinePanelKind::Indicator)
            .unwrap_or("Indicator")
            .to_string()
    }

    fn primary_overlay_value_text(
        &self,
        scene: &Scene,
        value_id: super::PanelValueId,
        channel_id: &str,
    ) -> Option<String> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return None;
        }

        let series = self.series.first()?;
        let bar_index = scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene
                    .interaction
                    .cursor
                    .and_then(|cursor| self.bar_index_at_or_before_unit(scene, 0, cursor.x_unit))
            })
            .flatten()
            .or_else(|| {
                self.legend_visibility
                    .legend_values_always_visible()
                    .then(|| self.series.first()?.point_count().checked_sub(1))
                    .flatten()
            })?;
        let value = self.indicator_source_value_for_scene_at(
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
            Some(self.format_panel_header_value(
                scene.primary.panel_index,
                self.panel_value_precision(scene.primary.panel_index),
                value,
            ))
        }
    }

    fn primary_overlay_channel_start_x(
        &self,
        row_x: f32,
        primary_panel: usize,
        value_id: super::PanelValueId,
    ) -> f32 {
        let title = self.indicator_title(primary_panel, true, value_id);
        self.indicator_label_x(row_x, value_id)
            + title.chars().count() as f32 * CHAR_W
            + PANEL_TITLE_TO_CONTROLS_GAP
    }

    fn primary_overlay_text_end_x(
        &self,
        scene: &Scene,
        row_rect: Rectangle,
        value_id: super::PanelValueId,
        show_values: bool,
    ) -> f32 {
        let title = self.indicator_title(scene.primary.panel_index, true, value_id);
        let title_x = self.indicator_label_x(row_rect.x, value_id);
        let title_end_x = title_x + title.chars().count() as f32 * CHAR_W;
        if !show_values {
            return title_end_x;
        }

        let mut x =
            self.primary_overlay_channel_start_x(row_rect.x, scene.primary.panel_index, value_id);
        let mut text_end_x = title_end_x;

        for channel in self
            .overlay_channels_for_panel_value(Some(value_id))
            .into_iter()
        {
            let label_width = channel.label.chars().count() as f32 * CHAR_W;
            text_end_x = x + label_width;
            x = text_end_x + 2.0;

            let Some(value_text) = self.primary_overlay_value_text(scene, value_id, channel.source)
            else {
                continue;
            };

            text_end_x = x + value_text.chars().count() as f32 * CHAR_W;
            x = text_end_x + 6.0;
        }

        text_end_x
    }

    fn indicator_panel_text_end_x(
        &self,
        scene: &Scene,
        row_rect: Rectangle,
        value_id: super::PanelValueId,
        include_values: bool,
    ) -> f32 {
        let Some(panel_index) = self.indicator_panel_index(scene, value_id) else {
            return row_rect.x + TOOLTIP_ROW_LEFT_PAD;
        };
        let Some(panel) = scene.layout.panels.get(panel_index) else {
            return row_rect.x + TOOLTIP_ROW_LEFT_PAD;
        };

        let label = self.indicator_title(panel_index, false, value_id);
        let label_end_x =
            self.indicator_label_x(row_rect.x, value_id) + label.chars().count() as f32 * CHAR_W;

        if !self.panel_indicator_visible(panel_index, value_id) {
            return label_end_x;
        }

        if matches!(
            self.composition
                .panels
                .get(panel_index)
                .map(|panel| panel.presentation),
            Some(super::composition::PanelPresentation::Table)
        ) {
            return label_end_x;
        }

        if !include_values
            || (!scene.interaction.readouts_visible()
                && !self.legend_visibility.legend_values_always_visible())
        {
            return label_end_x;
        }

        let Some(series) = self.series.first() else {
            return label_end_x;
        };
        let Some(bar_index) = scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene
                    .interaction
                    .cursor
                    .and_then(|cursor| self.bar_index_at_or_before_unit(scene, 0, cursor.x_unit))
            })
            .flatten()
            .or_else(|| {
                self.legend_visibility
                    .legend_values_always_visible()
                    .then(|| series.point_count().checked_sub(1))
                    .flatten()
            })
        else {
            return label_end_x;
        };

        let title_width = label.chars().count() as f32 * CHAR_W;
        let title_gap = if title_width > 0.0 {
            PANEL_TITLE_TO_CONTROLS_GAP
        } else {
            0.0
        };
        let mut value_x = self.indicator_label_x(row_rect.x, value_id) + title_width + title_gap;
        let mut text_end_x = label_end_x;
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

        for readout in self.indicator_header_values_for_indicator(
            scene,
            panel_index,
            value_id,
            0,
            series,
            bar_index,
        ) {
            if value_x >= max_x {
                break;
            }

            value_x += readout.label.chars().count() as f32 * CHAR_W + 2.0;
            if value_x >= max_x {
                break;
            }

            let value_text = self.format_panel_header_value(
                panel_index,
                self.panel_value_precision(panel_index),
                readout.value,
            );
            text_end_x = value_x + value_text.chars().count() as f32 * CHAR_W;
            value_x = text_end_x + 6.0;
        }

        text_end_x.max(label_end_x)
    }

    fn indicator_panel_index(&self, scene: &Scene, value_id: super::PanelValueId) -> Option<usize> {
        scene
            .layout
            .panels
            .iter()
            .enumerate()
            .find_map(|(index, panel)| {
                (panel.kind == super::KlinePanelKind::Indicator
                    && self.panel_indicator_value_ids(index).contains(&value_id))
                .then_some(index)
            })
    }

    fn fill_indicator_panel_title(
        &self,
        frame: &mut canvas::Frame,
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
                position: Point::new(self.indicator_marker_x(row_rect.x), title_y),
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

    fn indicator_legend_content_end_x(
        &self,
        scene: &Scene,
        row: &IndicatorLegendRowHit,
        row_rect: Rectangle,
        show_buttons: bool,
    ) -> f32 {
        if show_buttons {
            let plot_x = scene.layout.regions.plot.x;
            return [row.visibility, row.settings, row.close]
                .iter()
                .filter(|rect| rect.width > 0.0)
                .map(|rect| plot_x + rect.x + rect.width)
                .max_by(f32::total_cmp)
                .unwrap_or(row_rect.x);
        }

        let values_visible = scene.interaction.readouts_visible()
            || self.legend_visibility.legend_values_always_visible();
        if row.primary_overlay {
            self.primary_overlay_text_end_x(
                scene,
                row_rect,
                row.value_id,
                values_visible && !show_buttons,
            )
        } else {
            self.indicator_panel_text_end_x(
                scene,
                row_rect,
                row.value_id,
                values_visible && !show_buttons,
            )
        }
    }

    pub(super) fn tooltip_rows_before_indicators(
        &self,
        ticker_legend: Option<&TickerLegendLayout>,
        footprint_legend: Option<&FootprintLegendLayout>,
        study_legend: Option<&StudyLegendLayout>,
        show_legend_values: bool,
    ) -> usize {
        self.tooltip_rows_before_studies(ticker_legend, footprint_legend, show_legend_values)
            + study_legend
                .map(|legend| legend.rows.len() + 1)
                .unwrap_or_default()
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
            panel.kind == super::KlinePanelKind::Indicator
                && self.indicator_panel_value_id(index).is_some()
        });

        if !has_primary_overlays && !has_indicator_panels {
            return None;
        }

        let mut rows = Vec::new();

        for (panel_index, panel) in layout.panels.iter().enumerate() {
            if panel.kind != super::KlinePanelKind::Indicator {
                continue;
            }

            let value_ids = self.panel_indicator_value_ids(panel_index);
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
                let (_, visibility, settings, close, row_w) = Self::legend_controls(
                    panel.plot, text_end_x, y_center, false, false, true, true,
                );

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
                    visible: self.panel_indicator_visible(panel_index, value_id),
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
                let (_, visibility, settings, close, row_w) = Self::legend_controls(
                    panel.plot, text_end_x, y_center, false, true, true, true,
                );

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
        generated_accents: &[iced::Color; ACCENT_COUNT],
        show_primary_panel_values: bool,
    ) {
        let persistent_values_visible = self.legend_visibility.legend_values_always_visible();
        if !scene.interaction.readouts_visible() && !persistent_values_visible {
            return;
        }

        let Some(base_series) = self.series.first() else {
            return;
        };

        let Some(base_bar_index) = scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene
                    .interaction
                    .cursor
                    .and_then(|cursor| self.bar_index_at_or_before_unit(scene, 0, cursor.x_unit))
            })
            .flatten()
            .or_else(|| {
                persistent_values_visible
                    .then(|| base_series.point_count().checked_sub(1))
                    .flatten()
            })
        else {
            return;
        };

        let Some(base_bar) = scene
            .interaction
            .readouts_visible()
            .then(|| {
                scene.interaction.cursor.and_then(|cursor| {
                    self.bar_at_or_before_unit(base_series, scene.x.axis, cursor.x_unit)
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
                    let value_color = Self::resolve_theme_color(
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
            } else if self.indicator_panel_value_id(panel_index).is_some() {
                if matches!(
                    self.composition
                        .panels
                        .get(panel_index)
                        .map(|panel| panel.presentation),
                    Some(super::composition::PanelPresentation::Table)
                ) {
                    continue;
                }

                let hovered_indicator_id =
                    scene.interaction.target.indicator_row().and_then(|index| {
                        scene
                            .chrome
                            .indicator_legend
                            .as_ref()
                            .and_then(|legend| legend.rows.get(index))
                            .map(|row| row.value_id)
                    });

                for (indicator_index, value_id) in self
                    .panel_indicator_value_ids(panel_index)
                    .into_iter()
                    .enumerate()
                {
                    if Some(value_id) == hovered_indicator_id
                        || !self.panel_indicator_visible(panel_index, value_id)
                    {
                        continue;
                    }

                    let label = self.indicator_title(panel_index, false, value_id);
                    let title_width = label.chars().count() as f32 * CHAR_W;
                    let title_gap = if title_width > 0.0 {
                        PANEL_TITLE_TO_CONTROLS_GAP
                    } else {
                        0.0
                    };
                    let mut value_x = scene.layout.regions.plot.x
                        + self.indicator_label_x(panel.plot.x, value_id)
                        + title_width
                        + title_gap;
                    let value_y = scene.layout.regions.plot.y
                        + panel.plot.y
                        + PANEL_TITLE_TOP_PAD
                        + (indicator_index as f32 + 0.5) * TICKER_LEGEND_ROW_H;

                    for readout in self.indicator_header_values_for_indicator(
                        scene,
                        panel_index,
                        value_id,
                        0,
                        base_series,
                        base_bar_index,
                    ) {
                        if value_x >= max_x {
                            break;
                        }

                        let value_text = self.format_panel_header_value(
                            panel_index,
                            panel_precision,
                            readout.value,
                        );
                        let label_color = self.indicator_value_color(
                            Some(readout.value_id),
                            readout.color_slot,
                            readout.color_role,
                            palette,
                            generated_accents,
                        );
                        let label_width = readout.label.chars().count() as f32 * CHAR_W;
                        let value_width = value_text.chars().count() as f32 * CHAR_W;

                        frame.fill_text(canvas::Text {
                            content: readout.label.to_owned(),
                            position: Point::new(value_x, value_y),
                            color: label_color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        value_x += label_width + 2.0;

                        if value_x >= max_x {
                            break;
                        }

                        frame.fill_text(canvas::Text {
                            content: value_text,
                            position: Point::new(value_x, value_y),
                            color: label_color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        value_x += value_width + 6.0;
                    }
                }
            }
        }
    }

    fn draw_legend_icon_button(
        frame: &mut canvas::Frame,
        rect: Rectangle,
        content: LegendButtonContent,
        hovered: bool,
        palette: &Extended,
    ) {
        let text = if hovered {
            palette.background.strongest.text
        } else {
            palette.background.base.text
        };
        let (content, font) = match content {
            LegendButtonContent::Icon(icon) => (char::from(icon).to_string(), style::ICONS_FONT),
            LegendButtonContent::Text(content) => (content.to_string(), iced::Font::DEFAULT),
        };

        frame.fill_text(canvas::Text {
            content,
            position: Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
            color: text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font,
            ..Default::default()
        });
    }

    pub(super) fn fill_primary_ticker_legend(
        &self,
        frame: &mut canvas::Frame,
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
                .series
                .get(index)
                .map(series_display_name)
                .unwrap_or_else(|| row.ticker.ticker.symbol_and_exchange_string());
            let row_alpha = if index == 0 && !self.candlestick.visible {
                HIDDEN_LEGEND_ALPHA
            } else {
                1.0
            };
            let label_color = if index == 0 && !matches!(scene.primary.mark, MarkKind::Line) {
                palette.background.base.text
            } else {
                self.series
                    .get(index)
                    .map(|series| self.series_line_color(series, palette, generated_accents))
                    .unwrap_or(palette.background.base.text)
            }
            .scale_alpha(if index == 0 { 1.0 } else { 0.96 });

            // Build the colored, segmented text for this row. The primary
            // series shows O/H/L/C + change% colored green/red by direction
            // (close vs open), using the latest bar when values are persistent
            // and switching to the hovered bar over the plot.
            let mut segments: Vec<(String, iced::Color)> = Vec::new();

            if let Some(series) = self.series.get(index) {
                let effective_bar = if show_values {
                    if let Some(target_x_unit) = target_x_unit {
                        self.bar_at_or_before_unit(series, scene.x.axis, target_x_unit)
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
                        let value_color = Self::resolve_theme_color(
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
                Self::draw_legend_icon_button(
                    frame,
                    settings_root,
                    TickerLegendIconKind::Settings.icon(true),
                    settings_hovered,
                    palette,
                );

                if row.has_close {
                    let secondary_kind = if index == 0 {
                        TickerLegendIconKind::Hide
                    } else {
                        TickerLegendIconKind::Close
                    };
                    let secondary_content = if index == 0 {
                        secondary_kind.icon(self.candlestick.visible)
                    } else {
                        secondary_kind.icon(true)
                    };
                    let close_hovered = scene
                        .interaction
                        .target
                        .ticker_icon_is_hovered(index, secondary_kind);
                    Self::draw_legend_icon_button(
                        frame,
                        close_root,
                        secondary_content,
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
        Self::draw_legend_icon_button(
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
        Self::draw_legend_icon_button(
            frame,
            hide,
            FootprintLegendIconKind::Hide.icon(self.footprint.visible),
            hide_hovered,
            palette,
        );
    }

    pub(super) fn fill_study_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
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
            Self::draw_legend_icon_button(
                frame,
                expand,
                StudiesLegendIconKind::ExpandCollapse.icon(legend.category.expanded),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::ExpandCollapse),
                palette,
            );
            Self::draw_legend_icon_button(
                frame,
                visibility,
                StudiesLegendIconKind::Visibility.icon(legend.category.visible),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::Visibility),
                palette,
            );
            Self::draw_legend_icon_button(
                frame,
                close,
                StudiesLegendIconKind::Close.icon(legend.category.visible),
                scene
                    .interaction
                    .target
                    .studies_icon_is_hovered(StudiesLegendIconKind::Close),
                palette,
            );
        }

        for (index, row) in legend.rows.iter().enumerate() {
            let Some(study) = self.studies.iter().find(|study| study.id() == row.study_id) else {
                continue;
            };

            let row_rect = to_root(row.row_rect);
            let hovered_row = scene.interaction.target.study_row_is_hovered(index);
            let label = study.to_string();
            let label_end_x =
                row_rect.x + TOOLTIP_ROW_LEFT_PAD + label.chars().count() as f32 * CHAR_W;
            let content_end_x = if hovered_row {
                [row.visibility, row.settings, row.close]
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
                content: label,
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

            let visibility = to_root(row.visibility);
            let settings = to_root(row.settings);
            let close = to_root(row.close);
            Self::draw_legend_icon_button(
                frame,
                visibility,
                StudyLegendIconKind::Visibility.icon(row.visible),
                scene
                    .interaction
                    .target
                    .study_icon_is_hovered(index, StudyLegendIconKind::Visibility),
                palette,
            );
            Self::draw_legend_icon_button(
                frame,
                settings,
                StudyLegendIconKind::Settings.icon(row.visible),
                scene
                    .interaction
                    .target
                    .study_icon_is_hovered(index, StudyLegendIconKind::Settings),
                palette,
            );
            Self::draw_legend_icon_button(
                frame,
                close,
                StudyLegendIconKind::Close.icon(row.visible),
                scene
                    .interaction
                    .target
                    .study_icon_is_hovered(index, StudyLegendIconKind::Close),
                palette,
            );
        }
    }

    pub(super) fn fill_indicator_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
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
            let row_rect = to_root(row.row_rect);
            let show_buttons = scene.interaction.target.indicator_row_is_hovered(index);
            let show_values = !show_buttons
                && (scene.interaction.readouts_visible()
                    || self.legend_visibility.legend_values_always_visible());
            let content_end_x =
                self.indicator_legend_content_end_x(scene, row, row_rect, show_buttons);
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
                let marker_x = self.indicator_marker_x(row_rect.x);
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

                if show_values {
                    let mut x = self.primary_overlay_channel_start_x(
                        row_rect.x,
                        scene.primary.panel_index,
                        row.value_id,
                    );

                    for channel in self
                        .overlay_channels_for_panel_value(Some(row.value_id))
                        .into_iter()
                    {
                        let label_width = channel.label.chars().count() as f32 * CHAR_W;
                        let color = self
                            .overlay_channel_color(
                                Some(row.value_id),
                                &channel,
                                palette,
                                generated_accents,
                            )
                            .scale_alpha(row_alpha);

                        frame.fill_text(canvas::Text {
                            content: channel.label.to_owned(),
                            position: Point::new(x, label_y),
                            color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                        x += label_width + 2.0;

                        let Some(value_text) =
                            self.primary_overlay_value_text(scene, row.value_id, channel.source)
                        else {
                            continue;
                        };
                        let value_width = value_text.chars().count() as f32 * CHAR_W;

                        frame.fill_text(canvas::Text {
                            content: value_text,
                            position: Point::new(x, label_y),
                            color,
                            size: TEXT_SIZE.into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                        x += value_width + 6.0;
                    }
                }
            } else {
                self.fill_indicator_panel_title(frame, scene, row, row_rect, palette);
            }

            if show_buttons {
                let visibility_hovered = scene
                    .interaction
                    .target
                    .indicator_icon_is_hovered(index, IndicatorLegendIconKind::Visibility);
                Self::draw_legend_icon_button(
                    frame,
                    to_root(row.visibility),
                    IndicatorLegendIconKind::Visibility.icon(row.visible),
                    visibility_hovered,
                    palette,
                );

                let settings_hovered = scene
                    .interaction
                    .target
                    .indicator_icon_is_hovered(index, IndicatorLegendIconKind::Settings);
                Self::draw_legend_icon_button(
                    frame,
                    to_root(row.settings),
                    IndicatorLegendIconKind::Settings.icon(row.visible),
                    settings_hovered,
                    palette,
                );

                let close_hovered = scene
                    .interaction
                    .target
                    .indicator_icon_is_hovered(index, IndicatorLegendIconKind::Close);
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
                .target
                .corner_control_is_hovered(control.kind);

            let active = control
                .kind
                .is_active(scene.primary.mode, self.primary_viewport.is_auto());

            let (fill_color, text_color) = control_colors(palette, hovered, active);

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
                .target
                .panel_control_is_hovered(control.panel_index, control.kind);

            let (fill_color, text_color) = control_colors(palette, hovered, false);

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
