use super::layout::{PanelLayoutTree, contains_inclusive};
use super::scene::{ChartCoordinates, Scene};
use super::{CHAR_W, PANEL_TITLE_TOP_PAD};
pub(super) mod legend;
use super::PanelInteraction;
use super::composition::{PanelScaleMode, PanelValueId};
use super::{KlineWidgetEvent, TEXT_SIZE, TICKER_LEGEND_PADDING};
use crate::style;
use data::chart::kline::series::KlineSeries;
use iced::theme::palette::Extended;
use iced::{Color, Font, Point, Rectangle, Size};
pub(super) use legend::*;
pub use legend::{StudiesLegend, StudyLegendRow};

const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
pub(in crate::widget::chart) const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 8.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = 0.0;
pub(super) const SERIES_END_LABEL_GAP: f32 = 1.0;
const HIDDEN_LEGEND_ALPHA: f32 = 0.45;
const LIGHT_LEGEND_BACKGROUND_ALPHA: f32 = 0.7;
const DARK_LEGEND_BACKGROUND_ALPHA: f32 = 0.9;
const STUDY_LEGEND_INDENT: f32 = 8.0;
const STUDY_COUNT_BADGE_GAP: f32 = 4.0;
const STUDY_COUNT_BADGE_HORIZONTAL_PAD: f32 = 4.0;
const STUDY_COUNT_BADGE_HEIGHT: f32 = TEXT_SIZE + 2.0;

#[derive(Clone)]
pub(super) struct PreparedChrome {
    bounds: Rectangle,
    quads: Vec<(iced_core::renderer::Quad, Color)>,
    text: Vec<(iced::advanced::text::Text, Point, Color)>,
}

impl PreparedChrome {
    pub(super) fn new(bounds: Rectangle) -> Self {
        Self {
            bounds,
            quads: Vec::new(),
            text: Vec::new(),
        }
    }

    pub(super) fn append(&mut self, other: &Self) {
        self.quads.extend(other.quads.iter().copied());
        self.text.extend(other.text.iter().cloned());
    }

    pub(super) fn fill_rectangle(&mut self, position: Point, size: Size, color: Color) {
        self.quads.push((
            iced_core::renderer::Quad {
                bounds: Rectangle::new(position, size),
                snap: false,
                ..Default::default()
            },
            color,
        ));
    }

    pub(super) fn fill_bordered_rectangle(
        &mut self,
        bounds: Rectangle,
        color: Color,
        border_color: Color,
        border_width: f32,
    ) {
        let half = border_width * 0.5;
        self.quads.push((
            iced_core::renderer::Quad {
                bounds: Rectangle {
                    x: bounds.x - half,
                    y: bounds.y - half,
                    width: bounds.width + border_width,
                    height: bounds.height + border_width,
                },
                border: iced::Border {
                    color: border_color,
                    width: border_width,
                    ..Default::default()
                },
                snap: false,
                ..Default::default()
            },
            color,
        ));
    }

    pub(super) fn fill_text(&mut self, text: iced::widget::canvas::Text) {
        self.text.push((
            iced::advanced::text::Text {
                content: text.content,
                bounds: Size::new(text.max_width.min(self.bounds.width), self.bounds.height),
                size: text.size,
                line_height: text.line_height,
                font: text.font,
                align_x: text.align_x,
                align_y: text.align_y,
                shaping: text.shaping,
                wrapping: Default::default(),
            },
            text.position,
            text.color,
        ));
    }

    pub(super) fn draw(&self, renderer: &mut iced::Renderer) {
        use iced::advanced::Renderer as _;
        use iced::advanced::text::Renderer as _;
        renderer.with_translation(
            iced::Vector::new(self.bounds.x, self.bounds.y),
            |renderer| {
                let clip = Rectangle::with_size(self.bounds.size());
                renderer.with_layer(clip, |renderer| {
                    for (quad, color) in &self.quads {
                        renderer.fill_quad(*quad, *color);
                    }
                    for (text, position, color) in &self.text {
                        renderer.fill_text(text.clone(), *position, *color, clip);
                    }
                });
            },
        );
    }
}

pub(super) struct PreparedPanelControls {
    pub(super) bounds: Rectangle,
    pub(super) buttons: Vec<ControlPaint>,
}

impl PreparedPanelControls {
    pub(super) fn new(
        scene: &Scene,
        bounds: Rectangle,
        palette: &Extended,
        primary_autoscale: bool,
    ) -> Self {
        let mut buttons = Vec::new();
        for control in scene.chrome.corner_controls.iter().copied() {
            let hovered = scene
                .interaction
                .target
                .corner_control_is_hovered(control.kind);
            let active = control
                .kind
                .is_active(scene.primary.mode, primary_autoscale);
            buttons.push(ControlPaint::new(
                control.rect,
                control.kind.label(scene.primary.mode).to_owned(),
                style::AZERET_MONO,
                control_colors(palette, hovered, active),
            ));
        }

        if let Some(visible_panel) = scene.interaction.controls_visible_for_panel {
            for control in scene
                .chrome
                .panel_controls
                .iter()
                .filter(|control| control.panel_index == visible_panel)
            {
                let hovered = scene
                    .interaction
                    .target
                    .panel_control_is_hovered(control.panel_index, control.kind);
                let rect = Rectangle {
                    x: scene.layout.regions.plot.x + control.rect.x,
                    y: scene.layout.regions.plot.y + control.rect.y,
                    ..control.rect
                };
                buttons.push(ControlPaint::new(
                    rect,
                    char::from(control.kind.icon()).to_string(),
                    style::ICONS_FONT,
                    control_colors(palette, hovered, false),
                ));
            }
        }
        Self { bounds, buttons }
    }
}

pub(super) struct ControlPaint {
    pub(super) bounds: Rectangle,
    pub(super) background: Color,
    pub(super) text: iced::advanced::text::Text,
    pub(super) text_color: Color,
}

impl ControlPaint {
    fn new(bounds: Rectangle, content: String, font: Font, colors: (Color, Color)) -> Self {
        Self {
            bounds,
            background: colors.0,
            text_color: colors.1,
            text: iced::advanced::text::Text {
                content,
                bounds: bounds.size(),
                size: PANEL_CONTROL_ICON_SIZE.into(),
                line_height: iced::advanced::text::LineHeight::Relative(1.2),
                font,
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                shaping: iced::advanced::text::Shaping::default(),
                wrapping: iced::advanced::text::Wrapping::default(),
            },
        }
    }
}

pub(super) fn series_display_name(series: &KlineSeries) -> String {
    series.ticker_info.ticker.symbol_and_exchange_string()
}

pub(in crate::widget::chart) fn legend_background_color(palette: &Extended) -> Color {
    let alpha = if palette.is_dark {
        DARK_LEGEND_BACKGROUND_ALPHA
    } else {
        LIGHT_LEGEND_BACKGROUND_ALPHA
    };

    palette.background.weakest.color.scale_alpha(alpha)
}

pub(in crate::widget::chart) fn legend_row_background(
    row: Rectangle,
    content_end_x: f32,
) -> Rectangle {
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
    fn icon(self) -> style::Icon {
        match self {
            Self::MoveUp => style::Icon::ChevronUp,
            Self::MoveDown => style::Icon::ChevronDown,
            Self::Close => style::Icon::Close,
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
    rect: Rectangle,
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
    rect: Rectangle,
}

pub(super) enum ChromeClick {
    Event(KlineWidgetEvent),
    Blocked,
    Outside,
}

pub(super) fn build_corner_control_hits(
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

pub(super) fn build_panel_control_hits(
    coordinates: ChartCoordinates<'_>,
    layout: &PanelLayoutTree,
    primary_panel: usize,
) -> Vec<PanelControlHit> {
    let panel_count = layout.panels.len();
    let mut hits = Vec::with_capacity(panel_count.saturating_mul(4));

    for (panel_index, panel) in layout.panels.iter().enumerate() {
        let title_w = coordinates
            .resolved_panel_title(panel_index, panel.kind)
            .map(|title| title.chars().count() as f32 * CHAR_W)
            .unwrap_or(0.0);
        let title_gap = if title_w > 0.0 {
            PANEL_TITLE_TO_CONTROLS_GAP
        } else {
            0.0
        };
        let min_x = panel.plot.x + PANEL_TITLE_LEFT_PAD + title_w + title_gap;

        let mut controls = {
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
        };
        while !controls.is_empty() {
            let count = controls.len() as f32;
            let total_w = count * PANEL_CONTROL_BOX + (count - 1.0).max(0.0) * PANEL_CONTROL_GAP;
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
