use exchange::UnixMs;

use crate::widget::chart::kline::{YUnit, composition::PanelId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawingId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawingTool {
    #[default]
    Cursor,
    Trendline,
    Box,
    HorizontalLine,
    VerticalLine,
}

impl DrawingTool {
    pub fn allows_panning(self) -> bool {
        matches!(self, Self::Cursor)
    }

    pub const fn all() -> &'static [Self] {
        &[
            Self::Cursor,
            Self::Trendline,
            Self::Box,
            Self::HorizontalLine,
            Self::VerticalLine,
        ]
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Cursor => "C",
            Self::HorizontalLine => "H",
            Self::VerticalLine => "V",
            Self::Trendline => "TL",
            Self::Box => "B",
        }
    }

    pub fn default_style(self) -> DrawingStyle {
        let mut style = DrawingStyle::default();

        match self {
            Self::Cursor => {}
            Self::Trendline => {
                style.stroke_color = iced::Color::from_rgb(0.78, 0.86, 0.98);
                style.stroke_width = 1.2;
            }
            Self::Box => {
                style.stroke_color = iced::Color::from_rgb(0.72, 0.84, 0.98);
                style.stroke_width = 1.2;
                style.fill_color = Some(iced::Color::from_rgba(0.50, 0.66, 0.98, 0.16));
            }
            Self::HorizontalLine => {
                style.stroke_color = iced::Color::from_rgb(0.96, 0.80, 0.40);
                style.stroke_width = 1.0;
            }
            Self::VerticalLine => {
                style.stroke_color = iced::Color::from_rgb(0.74, 0.90, 0.74);
                style.stroke_width = 1.0;
            }
        }

        style
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingAnchor {
    pub panel_id: PanelId,
    pub time: UnixMs,
    pub y_unit: YUnit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KlineWidgetDrawingEvent {
    Selected(Option<DrawingId>),
    AnchorPressed(DrawingAnchor),
    AnchorMoved(DrawingAnchor),
    DragStarted {
        id: DrawingId,
        target: DrawingDragTarget,
        anchor: DrawingAnchor,
    },
    DragMoved {
        id: DrawingId,
        target: DrawingDragTarget,
        anchor: DrawingAnchor,
    },
    DragFinished {
        id: DrawingId,
    },
    DraftCanceled,
}

#[derive(Debug, Clone, Copy)]
pub struct DrawingSnapshot<'a> {
    pub active_tool: DrawingTool,
    pub entities: &'a [DrawingEntity],
    pub selected_drawing: Option<DrawingId>,
    pub drawing_draft: Option<&'a DrawingDraft>,
}

impl<'a> DrawingSnapshot<'a> {
    pub const fn new(
        active_tool: DrawingTool,
        entities: &'a [DrawingEntity],
        selected_drawing: Option<DrawingId>,
        drawing_draft: Option<&'a DrawingDraft>,
    ) -> Self {
        Self {
            active_tool,
            entities,
            selected_drawing,
            drawing_draft,
        }
    }

    pub fn allows_panning(&self) -> bool {
        self.active_tool.allows_panning()
    }

    pub fn has_state(&self) -> bool {
        !self.entities.is_empty() || self.selected_drawing.is_some() || self.drawing_draft.is_some()
    }

    pub fn draft_panel_id(&self) -> Option<PanelId> {
        self.drawing_draft.map(DrawingDraft::panel_id)
    }

    pub fn selected_visible_drawing(&self) -> Option<&'a DrawingEntity> {
        let selected = self.selected_drawing?;

        self.entities
            .iter()
            .find(|drawing| drawing.id == selected && drawing.visible)
    }

    pub fn active_axis_labeled_object(&self) -> Option<DrawingObject> {
        if let Some(draft) = self.drawing_draft {
            return Some(draft.preview_object());
        }

        self.selected_visible_drawing()
            .map(|drawing| drawing.object.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingStyle {
    pub stroke_color: iced::Color,
    pub stroke_width: f32,
    pub fill_color: Option<iced::Color>,
}

impl Default for DrawingStyle {
    fn default() -> Self {
        Self {
            stroke_color: iced::Color::from_rgb(0.82, 0.84, 0.90),
            stroke_width: 1.2,
            fill_color: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingObject {
    Trendline {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    Box {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    HorizontalLine {
        panel_id: PanelId,
        y_unit: YUnit,
    },
    VerticalLine {
        time: UnixMs,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingEntity {
    pub id: DrawingId,
    pub object: DrawingObject,
    pub style: DrawingStyle,
    pub locked: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingDraft {
    Trendline {
        start: DrawingAnchor,
        current: DrawingAnchor,
        style: DrawingStyle,
    },
    Box {
        start: DrawingAnchor,
        current: DrawingAnchor,
        style: DrawingStyle,
    },
}

impl DrawingDraft {
    pub fn tool(&self) -> DrawingTool {
        match self {
            Self::Trendline { .. } => DrawingTool::Trendline,
            Self::Box { .. } => DrawingTool::Box,
        }
    }

    pub fn style(&self) -> DrawingStyle {
        match self {
            Self::Trendline { style, .. } | Self::Box { style, .. } => *style,
        }
    }

    pub fn preview_object(&self) -> DrawingObject {
        match self {
            Self::Trendline { start, current, .. } => DrawingObject::Trendline {
                start: *start,
                end: *current,
            },
            Self::Box { start, current, .. } => DrawingObject::Box {
                start: *start,
                end: *current,
            },
        }
    }

    pub fn panel_id(&self) -> PanelId {
        match self {
            Self::Trendline { start, .. } | Self::Box { start, .. } => start.panel_id,
        }
    }

    pub fn belongs_to_panel(&self, panel_id: PanelId) -> bool {
        match self {
            Self::Trendline { start, current, .. } | Self::Box { start, current, .. } => {
                start.panel_id == panel_id || current.panel_id == panel_id
            }
        }
    }

    pub fn try_commit(self, end: DrawingAnchor) -> Option<(DrawingObject, DrawingStyle)> {
        match self {
            Self::Trendline { start, style, .. } => {
                if end.panel_id != start.panel_id {
                    return None;
                }

                Some((DrawingObject::Trendline { start, end }, style))
            }
            Self::Box { start, style, .. } => {
                if end.panel_id != start.panel_id {
                    return None;
                }

                Some((DrawingObject::Box { start, end }, style))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingHandleKind {
    TrendlineStart,
    TrendlineEnd,
    BoxTopLeft,
    BoxTopRight,
    BoxBottomRight,
    BoxBottomLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingDragTarget {
    Translate,
    Handle(DrawingHandleKind),
}

impl DrawingObject {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Box { .. } => "Box",
            Self::Trendline { .. } => "Trendline",
            Self::HorizontalLine { .. } => "Horizontal Line",
            Self::VerticalLine { .. } => "Vertical Line",
        }
    }

    pub fn panel_id(&self) -> Option<PanelId> {
        match self {
            Self::Trendline { start, .. } | Self::Box { start, .. } => Some(start.panel_id),
            Self::HorizontalLine { panel_id, .. } => Some(*panel_id),
            Self::VerticalLine { .. } => None,
        }
    }

    pub fn handle_panel_id(&self) -> Option<PanelId> {
        match self {
            Self::Trendline { start, end } | Self::Box { start, end } => {
                (start.panel_id == end.panel_id).then_some(start.panel_id)
            }
            Self::HorizontalLine { .. } | Self::VerticalLine { .. } => None,
        }
    }

    pub fn axis_label_anchors(&self) -> Option<(DrawingAnchor, DrawingAnchor)> {
        match self {
            Self::Trendline { start, end } | Self::Box { start, end } => Some((*start, *end)),
            Self::HorizontalLine { .. } | Self::VerticalLine { .. } => None,
        }
    }

    pub fn belongs_to_panel(&self, panel_id: PanelId) -> bool {
        match self {
            Self::Trendline { start, end } | Self::Box { start, end } => {
                start.panel_id == panel_id || end.panel_id == panel_id
            }
            Self::HorizontalLine {
                panel_id: drawing_panel,
                ..
            } => *drawing_panel == panel_id,
            Self::VerticalLine { .. } => false,
        }
    }

    pub fn translated(&self, origin_anchor: DrawingAnchor, current_anchor: DrawingAnchor) -> Self {
        let (delta_ms, forward_in_time) = Self::time_delta(origin_anchor.time, current_anchor.time);
        let delta_y = current_anchor
            .y_unit
            .0
            .saturating_sub(origin_anchor.y_unit.0);

        match self {
            Self::Trendline { start, end } => Self::Trendline {
                start: Self::shift_anchor_by_delta(*start, delta_ms, forward_in_time, delta_y),
                end: Self::shift_anchor_by_delta(*end, delta_ms, forward_in_time, delta_y),
            },
            Self::Box { start, end } => Self::Box {
                start: Self::shift_anchor_by_delta(*start, delta_ms, forward_in_time, delta_y),
                end: Self::shift_anchor_by_delta(*end, delta_ms, forward_in_time, delta_y),
            },
            Self::HorizontalLine { panel_id, y_unit } => Self::HorizontalLine {
                panel_id: *panel_id,
                y_unit: Self::shift_y_by_delta(*y_unit, delta_y),
            },
            Self::VerticalLine { time } => Self::VerticalLine {
                time: Self::shift_time_by_delta_ms(*time, delta_ms, forward_in_time),
            },
        }
    }

    pub fn handle_dragged(&self, handle: DrawingHandleKind, anchor: DrawingAnchor) -> Option<Self> {
        match (self, handle) {
            (Self::Trendline { start, end }, DrawingHandleKind::TrendlineStart) => {
                Some(Self::Trendline {
                    start: DrawingAnchor {
                        panel_id: start.panel_id,
                        ..anchor
                    },
                    end: *end,
                })
            }
            (Self::Trendline { start, end }, DrawingHandleKind::TrendlineEnd) => {
                Some(Self::Trendline {
                    start: *start,
                    end: DrawingAnchor {
                        panel_id: end.panel_id,
                        ..anchor
                    },
                })
            }
            (Self::Box { start, end }, corner) => Self::resized_box(*start, *end, corner, anchor),
            _ => None,
        }
    }

    fn resized_box(
        start: DrawingAnchor,
        end: DrawingAnchor,
        corner: DrawingHandleKind,
        anchor: DrawingAnchor,
    ) -> Option<Self> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let panel_id = start.panel_id;
        if anchor.panel_id != panel_id {
            return None;
        }

        let mut left = if start.time <= end.time {
            start.time
        } else {
            end.time
        };
        let mut right = if start.time <= end.time {
            end.time
        } else {
            start.time
        };

        let mut top = start.y_unit.0.max(end.y_unit.0);
        let mut bottom = start.y_unit.0.min(end.y_unit.0);

        match corner {
            DrawingHandleKind::BoxTopLeft => {
                left = anchor.time;
                top = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxTopRight => {
                right = anchor.time;
                top = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxBottomRight => {
                right = anchor.time;
                bottom = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxBottomLeft => {
                left = anchor.time;
                bottom = anchor.y_unit.0;
            }
            DrawingHandleKind::TrendlineStart | DrawingHandleKind::TrendlineEnd => {
                return None;
            }
        }

        Some(Self::Box {
            start: DrawingAnchor {
                panel_id,
                time: left,
                y_unit: YUnit(top),
            },
            end: DrawingAnchor {
                panel_id,
                time: right,
                y_unit: YUnit(bottom),
            },
        })
    }

    fn shift_anchor_by_delta(
        anchor: DrawingAnchor,
        delta_ms: u64,
        forward_in_time: bool,
        delta_y: i64,
    ) -> DrawingAnchor {
        DrawingAnchor {
            panel_id: anchor.panel_id,
            time: Self::shift_time_by_delta_ms(anchor.time, delta_ms, forward_in_time),
            y_unit: Self::shift_y_by_delta(anchor.y_unit, delta_y),
        }
    }

    fn shift_y_by_delta(y_unit: YUnit, delta_y: i64) -> YUnit {
        YUnit(y_unit.0.saturating_add(delta_y))
    }

    fn shift_time_by_delta_ms(time: UnixMs, delta_ms: u64, forward_in_time: bool) -> UnixMs {
        if forward_in_time {
            time.saturating_add(delta_ms)
        } else {
            time.saturating_sub(delta_ms)
        }
    }

    fn time_delta(origin: UnixMs, current: UnixMs) -> (u64, bool) {
        if current >= origin {
            (current.saturating_diff(origin), true)
        } else {
            (origin.saturating_diff(current), false)
        }
    }
}

impl DrawingEntity {
    pub fn belongs_to_panel(&self, panel_id: PanelId) -> bool {
        self.object.belongs_to_panel(panel_id)
    }
}
