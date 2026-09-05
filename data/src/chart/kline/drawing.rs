use super::YUnit;
use super::composition::PanelId;
use crate::config::theme::{ThemeAccent, ThemeColorShade};
use exchange::UnixMs;
use serde::{Deserialize, Serialize};

pub const MIN_DRAWING_STROKE_WIDTH: f32 = 0.5;
pub const MAX_DRAWING_STROKE_WIDTH: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Rgba {
    pub const fn from_rgb(red: f32, green: f32, blue: f32) -> Self {
        Self::from_rgba(red, green, blue, 1.0)
    }

    pub const fn from_rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub fn scale_alpha(self, factor: f32) -> Self {
        Self {
            alpha: self.alpha * factor,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DrawingId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DrawingTool {
    #[default]
    Cursor,
    Trendline,
    Box,
    HorizontalLine,
    VerticalLine,
    Measure,
}

impl DrawingTool {
    pub fn allows_panning(self) -> bool {
        matches!(self, Self::Cursor)
    }

    pub const fn supports_magnet(self) -> bool {
        matches!(
            self,
            Self::Trendline | Self::Box | Self::HorizontalLine | Self::VerticalLine
        )
    }

    pub const fn all() -> &'static [Self] {
        &[
            Self::Cursor,
            Self::Trendline,
            Self::Box,
            Self::HorizontalLine,
            Self::VerticalLine,
            Self::Measure,
        ]
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Cursor => "C",
            Self::HorizontalLine => "H",
            Self::VerticalLine => "V",
            Self::Trendline => "TL",
            Self::Box => "B",
            Self::Measure => "M",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Crosshair",
            Self::Trendline => "Trendline",
            Self::Box => "Box",
            Self::HorizontalLine => "Horizontal line",
            Self::VerticalLine => "Vertical line",
            Self::Measure => "Measure",
        }
    }

    pub fn default_style(self) -> DrawingStyle {
        let mut style = DrawingStyle::default();

        match self {
            Self::Cursor | Self::Measure => {}
            Self::Trendline => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::One, 1.0);
                style.stroke_width = 1.2;
            }
            Self::Box => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::Two, 1.0);
                style.stroke_width = 1.2;
                style.fill_color = Some(DrawingColor::accent(ThemeAccent::Two, 0.16));
            }
            Self::HorizontalLine => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::Three, 1.0);
                style.stroke_width = 1.0;
            }
            Self::VerticalLine => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::Four, 1.0);
                style.stroke_width = 1.0;
            }
        }

        style
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DrawingColor {
    Accent {
        accent: ThemeAccent,
        #[serde(default)]
        shade: ThemeColorShade,
        alpha: f32,
    },
    Custom(Rgba),
}

impl DrawingColor {
    pub const fn accent(accent: ThemeAccent, alpha: f32) -> Self {
        Self::themed(accent, ThemeColorShade::Base, alpha)
    }

    pub const fn themed(accent: ThemeAccent, shade: ThemeColorShade, alpha: f32) -> Self {
        Self::Accent {
            accent,
            shade,
            alpha,
        }
    }

    pub fn accent_value(self) -> Option<ThemeAccent> {
        match self {
            Self::Accent { accent, .. } => Some(accent),
            Self::Custom(_) => None,
        }
    }

    pub fn shade_value(self) -> ThemeColorShade {
        match self {
            Self::Accent { shade, .. } => shade,
            Self::Custom(_) => ThemeColorShade::Base,
        }
    }

    pub fn with_accent(self, accent: ThemeAccent, alpha: f32) -> Self {
        Self::themed(accent, self.shade_value(), alpha)
    }

    pub fn set_shade(&mut self, shade: ThemeColorShade) -> bool {
        let Self::Accent {
            shade: current_shade,
            ..
        } = self
        else {
            return false;
        };

        if *current_shade == shade {
            return false;
        }

        *current_shade = shade;
        true
    }

    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            Self::Accent {
                accent,
                shade,
                alpha,
            } => Self::Accent {
                accent,
                shade,
                alpha: alpha * factor,
            },
            Self::Custom(color) => Self::Custom(color.scale_alpha(factor)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawingAnchor {
    pub panel_id: PanelId,
    pub time: UnixMs,
    pub y_unit: YUnit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KlineWidgetDrawingEvent {
    MagnetModifierChanged(bool),
    Selected(Option<DrawingId>),
    MarqueeSelected(Vec<DrawingId>),
    AnchorPressed(DrawingAnchor),
    AnchorMoved(DrawingAnchor),
    DragStarted {
        id: DrawingId,
        target: DrawingDragTarget,
        operation: DrawingDragOperation,
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
    MeasureClick(DrawingAnchor),
    MeasureMove(DrawingAnchor),
    MeasureCanceled,
    DraftCanceled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureState {
    pub panel_id: PanelId,
    pub start: DrawingAnchor,
    pub end: DrawingAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawingMeasure {
    Pending(MeasureState),
    Committed(MeasureState),
}

impl DrawingMeasure {
    pub fn state(self) -> MeasureState {
        match self {
            Self::Pending(state) | Self::Committed(state) => state,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DrawingSnapshot<'a> {
    pub active_tool: DrawingTool,
    pub magnet_enabled: bool,
    pub entities: &'a [DrawingEntity],
    pub selected_drawings: &'a [DrawingId],
    pub drawing_draft: Option<&'a DrawingDraft>,
    pub measure: Option<DrawingMeasure>,
}

impl<'a> DrawingSnapshot<'a> {
    pub const fn new(
        active_tool: DrawingTool,
        magnet_enabled: bool,
        entities: &'a [DrawingEntity],
        selected_drawings: &'a [DrawingId],
        drawing_draft: Option<&'a DrawingDraft>,
        measure: Option<DrawingMeasure>,
    ) -> Self {
        Self {
            active_tool,
            magnet_enabled,
            entities,
            selected_drawings,
            drawing_draft,
            measure,
        }
    }

    pub fn allows_panning(&self) -> bool {
        self.active_tool.allows_panning()
    }

    pub fn has_state(&self) -> bool {
        !self.entities.is_empty()
            || !self.selected_drawings.is_empty()
            || self.drawing_draft.is_some()
            || self.measure.is_some()
    }

    pub fn draft_panel_id(&self) -> Option<PanelId> {
        self.drawing_draft.map(DrawingDraft::panel_id)
    }

    pub fn selected_visible_drawings(&self) -> impl Iterator<Item = &'a DrawingEntity> {
        self.entities
            .iter()
            .filter(|drawing| drawing.visible && self.selected_drawings.contains(&drawing.id))
    }

    pub fn selected_visible_drawing(&self) -> Option<&'a DrawingEntity> {
        if self.selected_drawings.len() == 1 {
            self.selected_visible_drawings().next()
        } else {
            None
        }
    }

    pub fn active_axis_labeled_object(&self) -> Option<DrawingObject> {
        if let Some(draft) = self.drawing_draft {
            return Some(draft.preview_object());
        }

        self.selected_visible_drawing()
            .map(|drawing| drawing.object.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawingStyle {
    pub stroke_color: DrawingColor,
    pub stroke_width: f32,
    pub fill_color: Option<DrawingColor>,
}

impl Default for DrawingStyle {
    fn default() -> Self {
        Self {
            stroke_color: DrawingColor::accent(ThemeAccent::One, 1.0),
            stroke_width: 1.2,
            fill_color: None,
        }
    }
}

impl DrawingStyle {
    pub fn normalized_stroke_width(self) -> f32 {
        Self::normalize_stroke_width(self.stroke_width)
    }

    pub fn set_stroke_width(&mut self, width: f32) -> bool {
        let width = Self::normalize_stroke_width(width);

        if self.stroke_width == width {
            return false;
        }

        self.stroke_width = width;
        true
    }

    fn normalize_stroke_width(width: f32) -> f32 {
        if width.is_finite() {
            width.clamp(MIN_DRAWING_STROKE_WIDTH, MAX_DRAWING_STROKE_WIDTH)
        } else {
            Self::default().stroke_width
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingStyleDefaults {
    pub trendline: DrawingStyle,
    pub box_drawing: DrawingStyle,
    pub horizontal_line: DrawingStyle,
    pub vertical_line: DrawingStyle,
}

impl Default for DrawingStyleDefaults {
    fn default() -> Self {
        Self {
            trendline: DrawingTool::Trendline.default_style(),
            box_drawing: DrawingTool::Box.default_style(),
            horizontal_line: DrawingTool::HorizontalLine.default_style(),
            vertical_line: DrawingTool::VerticalLine.default_style(),
        }
    }
}

impl DrawingStyleDefaults {
    pub fn style_for(self, tool: DrawingTool) -> DrawingStyle {
        match tool {
            DrawingTool::Trendline => self.trendline,
            DrawingTool::Box => self.box_drawing,
            DrawingTool::HorizontalLine => self.horizontal_line,
            DrawingTool::VerticalLine => self.vertical_line,
            DrawingTool::Cursor | DrawingTool::Measure => tool.default_style(),
        }
    }

    pub fn set_style(&mut self, tool: DrawingTool, style: DrawingStyle) -> bool {
        let stored_style = match tool {
            DrawingTool::Trendline => &mut self.trendline,
            DrawingTool::Box => &mut self.box_drawing,
            DrawingTool::HorizontalLine => &mut self.horizontal_line,
            DrawingTool::VerticalLine => &mut self.vertical_line,
            DrawingTool::Cursor | DrawingTool::Measure => return false,
        };

        if *stored_style == style {
            return false;
        }

        *stored_style = style;
        true
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSettings {
    pub style_defaults: DrawingStyleDefaults,
    pub horizontal_line_ray: bool,
    pub box_extend_right: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawingObject {
    Trendline {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    Box {
        start: DrawingAnchor,
        end: DrawingAnchor,
        #[serde(default)]
        extend_right: bool,
    },
    HorizontalLine {
        panel_id: PanelId,
        y_unit: YUnit,
        #[serde(default)]
        time: UnixMs,
        #[serde(default)]
        ray: bool,
    },
    VerticalLine {
        time: UnixMs,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        extend_right: bool,
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
            Self::Box {
                start,
                current,
                extend_right,
                ..
            } => DrawingObject::normalized_box(*start, *current, *extend_right),
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
            Self::Box {
                start,
                style,
                extend_right,
                ..
            } => {
                if end.panel_id != start.panel_id {
                    return None;
                }

                Some((
                    DrawingObject::normalized_box(start, end, extend_right),
                    style,
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingHandleKind {
    TrendlineStart,
    TrendlineEnd,
    HorizontalLineStart,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingDragOperation {
    Move,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisLabelAnchor {
    X { time: UnixMs },
    Y { panel_id: PanelId, y_unit: YUnit },
}

impl DrawingObject {
    pub fn normalized_box(
        mut start: DrawingAnchor,
        mut end: DrawingAnchor,
        extend_right: bool,
    ) -> Self {
        if extend_right && end.time < start.time {
            std::mem::swap(&mut start.time, &mut end.time);
        }

        Self::Box {
            start,
            end,
            extend_right,
        }
    }

    pub fn tool(&self) -> DrawingTool {
        match self {
            Self::Trendline { .. } => DrawingTool::Trendline,
            Self::Box { .. } => DrawingTool::Box,
            Self::HorizontalLine { .. } => DrawingTool::HorizontalLine,
            Self::VerticalLine { .. } => DrawingTool::VerticalLine,
        }
    }

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
            Self::Trendline { start, end } | Self::Box { start, end, .. } => {
                (start.panel_id == end.panel_id).then_some(start.panel_id)
            }
            Self::HorizontalLine { panel_id, .. } => Some(*panel_id),
            Self::VerticalLine { .. } => None,
        }
    }

    pub fn axis_label_anchors(&self) -> Vec<AxisLabelAnchor> {
        match self {
            Self::Trendline { start, end } => vec![
                AxisLabelAnchor::X { time: start.time },
                AxisLabelAnchor::Y {
                    panel_id: start.panel_id,
                    y_unit: start.y_unit,
                },
                AxisLabelAnchor::X { time: end.time },
                AxisLabelAnchor::Y {
                    panel_id: end.panel_id,
                    y_unit: end.y_unit,
                },
            ],
            Self::Box {
                start,
                end,
                extend_right,
            } => {
                let panel_id = start.panel_id;
                let left = start.time.min(end.time);
                let top = YUnit(start.y_unit.0.max(end.y_unit.0));
                let bottom = YUnit(start.y_unit.0.min(end.y_unit.0));

                let mut anchors = vec![
                    AxisLabelAnchor::X { time: left },
                    AxisLabelAnchor::Y {
                        panel_id,
                        y_unit: top,
                    },
                    AxisLabelAnchor::Y {
                        panel_id,
                        y_unit: bottom,
                    },
                ];

                if !extend_right {
                    anchors.push(AxisLabelAnchor::X {
                        time: start.time.max(end.time),
                    });
                }

                anchors
            }
            Self::HorizontalLine {
                panel_id,
                y_unit,
                time,
                ray,
            } => {
                let mut anchors = Vec::with_capacity(2);
                if *ray {
                    anchors.push(AxisLabelAnchor::X { time: *time });
                }
                anchors.push(AxisLabelAnchor::Y {
                    panel_id: *panel_id,
                    y_unit: *y_unit,
                });
                anchors
            }
            Self::VerticalLine { time } => vec![AxisLabelAnchor::X { time: *time }],
        }
    }

    pub fn belongs_to_panel(&self, panel_id: PanelId) -> bool {
        match self {
            Self::Trendline { start, end } | Self::Box { start, end, .. } => {
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
            Self::Box {
                start,
                end,
                extend_right,
            } => Self::Box {
                start: Self::shift_anchor_by_delta(*start, delta_ms, forward_in_time, delta_y),
                end: Self::shift_anchor_by_delta(*end, delta_ms, forward_in_time, delta_y),
                extend_right: *extend_right,
            },
            Self::HorizontalLine {
                panel_id,
                y_unit,
                time,
                ray,
            } => Self::HorizontalLine {
                panel_id: *panel_id,
                y_unit: Self::shift_y_by_delta(*y_unit, delta_y),
                time: Self::shift_time_by_delta_ms(*time, delta_ms, forward_in_time),
                ray: *ray,
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
            (
                Self::HorizontalLine {
                    panel_id,
                    ray: true,
                    ..
                },
                DrawingHandleKind::HorizontalLineStart,
            ) if anchor.panel_id == *panel_id => Some(Self::HorizontalLine {
                panel_id: *panel_id,
                y_unit: anchor.y_unit,
                time: anchor.time,
                ray: true,
            }),
            (
                Self::Box {
                    start,
                    end,
                    extend_right,
                },
                corner,
            ) => Self::resized_box(*start, *end, *extend_right, corner, anchor),
            _ => None,
        }
    }

    fn resized_box(
        start: DrawingAnchor,
        end: DrawingAnchor,
        extend_right: bool,
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

        if extend_right {
            return Self::resized_extended_box(start, end, corner, anchor);
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
            DrawingHandleKind::TrendlineStart
            | DrawingHandleKind::TrendlineEnd
            | DrawingHandleKind::HorizontalLineStart => {
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
            extend_right,
        })
    }

    fn resized_extended_box(
        start: DrawingAnchor,
        end: DrawingAnchor,
        corner: DrawingHandleKind,
        anchor: DrawingAnchor,
    ) -> Option<Self> {
        let top = start.y_unit.0.max(end.y_unit.0);
        let bottom = start.y_unit.0.min(end.y_unit.0);
        let (top, bottom) = match corner {
            DrawingHandleKind::BoxTopLeft => (anchor.y_unit.0, bottom),
            DrawingHandleKind::BoxBottomLeft => (top, anchor.y_unit.0),
            DrawingHandleKind::TrendlineStart
            | DrawingHandleKind::TrendlineEnd
            | DrawingHandleKind::HorizontalLineStart
            | DrawingHandleKind::BoxTopRight
            | DrawingHandleKind::BoxBottomRight => return None,
        };

        Some(Self::Box {
            start: DrawingAnchor {
                panel_id: start.panel_id,
                time: anchor.time,
                y_unit: YUnit(top),
            },
            end: DrawingAnchor {
                panel_id: end.panel_id,
                time: end.time,
                y_unit: YUnit(bottom),
            },
            extend_right: true,
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
