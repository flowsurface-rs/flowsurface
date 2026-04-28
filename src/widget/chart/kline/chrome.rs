use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::scene::Scene;
use super::{
    CHAR_W, KlineSeriesLike, KlineWidget, KlineWidgetEvent, PANEL_CONTROL_BOX, PANEL_CONTROL_GAP,
    PANEL_CONTROL_ICON_SIZE, PANEL_TITLE_LEFT_PAD, PANEL_TITLE_TO_CONTROLS_GAP,
    PANEL_TITLE_TOP_PAD, TEXT_SIZE, TICKER_LEGEND_ICON_BOX, TICKER_LEGEND_ICON_GAP,
    TICKER_LEGEND_PADDING, TICKER_LEGEND_ROW_H, TICKER_LEGEND_TOP_OFFSET,
};
use crate::style;
use crate::widget::chart::kline::composition::MarkKind;

use exchange::TickerInfo;

use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};

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

    pub(super) fn into_event(self, index: usize) -> KlineWidgetEvent {
        match self {
            Self::MoveUp => KlineWidgetEvent::PanelMoveUp { index },
            Self::MoveDown => KlineWidgetEvent::PanelMoveDown { index },
            Self::Close => KlineWidgetEvent::PanelClose { index },
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
pub(super) enum TickerLegendIconKind {
    Settings,
    Close,
}

impl TickerLegendIconKind {
    fn icon(self) -> style::Icon {
        match self {
            Self::Settings => style::Icon::Cog,
            Self::Close => style::Icon::Close,
        }
    }

    pub(super) fn into_event(self, ticker: TickerInfo) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::TickerSettings(ticker),
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

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
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
        if self.series.len() <= 1 {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let plot = panel.plot;

        let rows_count = self.series.len();

        let mut max_chars: usize = 0;
        let mut max_name_chars: usize = 0;

        for (index, series) in self.series.iter().enumerate() {
            let name_chars = series
                .ticker_info()
                .ticker
                .symbol_and_exchange_string()
                .chars()
                .count();
            max_name_chars = max_name_chars.max(name_chars);

            let value_chars = if show_values {
                if index == 0 {
                    series
                        .bars()
                        .last()
                        .map(|bar| {
                            let precision = series.ticker_info().min_ticksize;
                            format!(
                                "O {} H {} L {} C {}",
                                bar.open.to_string(precision),
                                bar.high.to_string(precision),
                                bar.low.to_string(precision),
                                bar.close.to_string(precision),
                            )
                            .chars()
                            .count()
                        })
                        .unwrap_or(0)
                } else {
                    "+100.00%".chars().count()
                }
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

        for (index, series) in self.series.iter().take(visible_rows).enumerate() {
            let has_close = index != 0;
            let label_chars = series
                .ticker_info()
                .ticker
                .symbol_and_exchange_string()
                .chars()
                .count() as f32;

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
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Close));
            }

            return Some(TickerLegendHit::Row(index));
        }

        Some(TickerLegendHit::Background)
    }

    pub(super) fn fill_panel_header_values(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        show_primary_panel_values: bool,
    ) {
        let Some(cursor) = scene.cursor else {
            return;
        };

        let Some(base_series) = self.series.first() else {
            return;
        };

        let Some(base_bar) = self.bar_at_or_before_unit(base_series, scene.x_axis, cursor.x_unit)
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
                + PANEL_TITLE_LEFT_PAD
                + title_w
                + title_gap;
            let y = scene.layout.regions.plot.y + panel.plot.y + PANEL_TITLE_TOP_PAD;

            let mut max_x = scene.layout.regions.plot.x + panel.plot.x + panel.plot.width
                - PANEL_TITLE_LEFT_PAD;
            if scene.controls_visible_for_panel == Some(panel_index)
                && let Some(left_control_x) = scene
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

            if panel_index == scene.primary_panel {
                if !show_primary_panel_values {
                    continue;
                }

                let precision = base_series.ticker_info().min_ticksize;

                if matches!(scene.primary_mark, MarkKind::Candle | MarkKind::Bar(_)) {
                    let open_f = base_bar.open.to_f32();
                    let close_f = base_bar.close.to_f32();
                    let change_pct = if open_f.abs() > f32::EPSILON {
                        ((close_f - open_f) / open_f) * 100.0
                    } else {
                        0.0
                    };

                    let value_color = if change_pct >= 0.0 {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };
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
                            align_y: iced::Alignment::Start.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        x += text.chars().count() as f32 * CHAR_W;
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
                        align_y: iced::Alignment::Start.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            } else {
                if let Some(value) =
                    base_series.indicator_value_for_panel_opt(panel_index, base_bar)
                {
                    let text = data::util::format_with_commas(value);

                    frame.fill_text(canvas::Text {
                        content: text,
                        position: Point::new(x, y),
                        color: palette.background.base.text.scale_alpha(0.82),
                        size: TEXT_SIZE.into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Start.into(),
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
    ) {
        let Some(legend) = scene.ticker_legend.as_ref() else {
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

        let target_x_unit = scene.cursor.map(|cursor| cursor.x_unit);
        let show_buttons = scene.hovering_ticker_legend;
        let row_hover_fill = palette.background.strong.color.scale_alpha(0.22);

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.row_rect);
            let hovered_row = scene.hovered_ticker_row == Some(index);

            if show_buttons && hovered_row {
                let highlight = Rectangle {
                    x: row_rect.x + 1.0,
                    y: row_rect.y,
                    width: (row_rect.width - 2.0).max(0.0),
                    height: row_rect.height,
                };
                frame.fill_rectangle(highlight.position(), highlight.size(), row_hover_fill);
            }

            let label = row.ticker.ticker.symbol_and_exchange_string();
            let label_color = if index == 0 {
                palette.background.base.text
            } else {
                Self::comparison_line_color(&row.ticker).scale_alpha(0.96)
            };

            let value_text = if let Some(target_x_unit) = target_x_unit
                && !show_buttons
                && let Some(series) = self.series.get(index)
                && let Some(bar) = self.bar_at_or_before_unit(series, scene.x_axis, target_x_unit)
            {
                let precision = series.ticker_info().min_ticksize;
                if index == 0 {
                    Some(format!(
                        "O {} H {} L {} C {}",
                        bar.open.to_string(precision),
                        bar.high.to_string(precision),
                        bar.low.to_string(precision),
                        bar.close.to_string(precision),
                    ))
                } else {
                    Some(
                        self.bar_at_or_before_unit(series, scene.x_axis, scene.min_x_unit)
                            .and_then(|anchor_bar| {
                                let anchor = anchor_bar.close.to_f32();
                                (anchor.abs() > f32::EPSILON)
                                    .then_some(((bar.close.to_f32() / anchor) - 1.0) * 100.0)
                            })
                            .map(|pct| format!("{pct:+.2}%"))
                            .unwrap_or_else(|| "n/a".to_string()),
                    )
                }
            } else {
                None
            };

            let content = if let Some(value_text) = value_text {
                format!("{label} {value_text}")
            } else {
                label
            };

            frame.fill_text(canvas::Text {
                content,
                position: Point::new(
                    bg.x + TICKER_LEGEND_PADDING,
                    scene.layout.regions.plot.y + row.y_center,
                ),
                color: label_color,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });

            if show_buttons {
                let settings_root = to_root(row.settings);
                let close_root = to_root(row.close);

                let settings_hovered =
                    scene.hovered_ticker_icon == Some((index, TickerLegendIconKind::Settings));
                Self::draw_legend_icon_button(
                    frame,
                    settings_root,
                    TickerLegendIconKind::Settings.icon(),
                    settings_hovered,
                    palette,
                );

                if row.has_close {
                    let close_hovered =
                        scene.hovered_ticker_icon == Some((index, TickerLegendIconKind::Close));
                    Self::draw_legend_icon_button(
                        frame,
                        close_root,
                        TickerLegendIconKind::Close.icon(),
                        close_hovered,
                        palette,
                    );
                }
            }
        }
    }

    pub(super) fn fill_panel_controls(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(visible_panel) = scene.controls_visible_for_panel else {
            return;
        };

        for control in scene
            .panel_controls
            .iter()
            .copied()
            .filter(|control| control.panel_index == visible_panel)
        {
            let hovered = scene
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
