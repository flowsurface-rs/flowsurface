use super::YUnit;
use super::composition::PanelId;
use crate::config::theme::{ThemeAccent, ThemeColorShade};
use exchange::UnixMs;
use serde::{Deserialize, Serialize};

pub const MIN_DRAWING_STROKE_WIDTH: f32 = 0.5;
pub const MAX_DRAWING_STROKE_WIDTH: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DrawingId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DrawingTool {
    #[default]
    Cursor,
    Trendline,
    HorizontalLine,
    VerticalLine,
    Box,
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
            Self::HorizontalLine,
            Self::VerticalLine,
            Self::Box,
            Self::Measure,
        ]
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
                style.stroke_color = DrawingColor::accent(ThemeAccent::One, 1.0);
                style.stroke_width = 1.2;
                style.fill_color = Some(DrawingColor::accent(ThemeAccent::One, 0.16));
            }
            Self::HorizontalLine => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::One, 1.0);
                style.stroke_width = 1.0;
            }
            Self::VerticalLine => {
                style.stroke_color = DrawingColor::accent(ThemeAccent::One, 1.0);
                style.stroke_width = 1.0;
            }
        }

        style
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawingColor {
    pub accent: ThemeAccent,
    #[serde(default)]
    pub shade: ThemeColorShade,
    pub alpha: f32,
}

impl DrawingColor {
    pub const fn accent(accent: ThemeAccent, alpha: f32) -> Self {
        Self::themed(accent, ThemeColorShade::Base, alpha)
    }

    pub const fn themed(accent: ThemeAccent, shade: ThemeColorShade, alpha: f32) -> Self {
        Self {
            accent,
            shade,
            alpha,
        }
    }

    pub fn with_accent(self, accent: ThemeAccent, alpha: f32) -> Self {
        Self {
            accent,
            alpha,
            ..self
        }
    }

    pub fn scale_alpha(self, factor: f32) -> Self {
        Self {
            alpha: self.alpha * factor,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawingX {
    Time(UnixMs),
    Tick(f64),
}

impl Default for DrawingX {
    fn default() -> Self {
        Self::Time(UnixMs::new(0))
    }
}

impl Serialize for DrawingX {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Time(time) => time.serialize(serializer),
            Self::Tick(_) => Err(serde::ser::Error::custom("tick drawings are session-only")),
        }
    }
}

impl<'de> Deserialize<'de> for DrawingX {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UnixMs::deserialize(deserializer).map(Self::Time)
    }
}

impl PartialOrd for DrawingX {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (*self, *other) {
            (Self::Time(a), Self::Time(b)) => a.partial_cmp(&b),
            (Self::Tick(a), Self::Tick(b)) if a.is_finite() && b.is_finite() => a.partial_cmp(&b),
            _ => None,
        }
    }
}

impl DrawingX {
    pub fn is_time(self) -> bool {
        matches!(self, Self::Time(_))
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::Time(_) => true,
            Self::Tick(position) => position.is_finite(),
        }
    }

    pub fn compatible(self, other: Self) -> bool {
        self.partial_cmp(&other).is_some()
    }

    pub fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    pub fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    pub fn tick_bar_position(self, trades_per_bar: u64) -> Option<f64> {
        match self {
            Self::Tick(position) if position.is_finite() && trades_per_bar > 0 => {
                Some(position / trades_per_bar as f64)
            }
            _ => None,
        }
    }

    fn translated(self, origin: Self, current: Self) -> Option<Self> {
        match (self, origin, current) {
            (Self::Time(time), Self::Time(origin), Self::Time(current)) => {
                Some(Self::Time(if current >= origin {
                    time.saturating_add(current.saturating_diff(origin))
                } else {
                    time.saturating_sub(origin.saturating_diff(current))
                }))
            }
            (Self::Tick(position), Self::Tick(origin), Self::Tick(current))
                if position.is_finite() && origin.is_finite() && current.is_finite() =>
            {
                let translated = position + (current - origin);
                translated.is_finite().then_some(Self::Tick(translated))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawingAnchor {
    pub panel_id: PanelId,
    #[serde(rename = "time")]
    pub x: DrawingX,
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
        #[serde(rename = "time")]
        x: DrawingX,
        #[serde(default)]
        ray: bool,
    },
    VerticalLine {
        #[serde(rename = "time")]
        x: DrawingX,
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
                if end.panel_id != start.panel_id || !start.x.compatible(end.x) {
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
                if end.panel_id != start.panel_id || !start.x.compatible(end.x) {
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
    X { x: DrawingX },
    Y { panel_id: PanelId, y_unit: YUnit },
}

impl DrawingObject {
    pub fn normalized_box(
        mut start: DrawingAnchor,
        mut end: DrawingAnchor,
        extend_right: bool,
    ) -> Self {
        if extend_right && end.x < start.x {
            std::mem::swap(&mut start.x, &mut end.x);
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
                AxisLabelAnchor::X { x: start.x },
                AxisLabelAnchor::Y {
                    panel_id: start.panel_id,
                    y_unit: start.y_unit,
                },
                AxisLabelAnchor::X { x: end.x },
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
                let left = start.x.min(end.x);
                let top = YUnit(start.y_unit.0.max(end.y_unit.0));
                let bottom = YUnit(start.y_unit.0.min(end.y_unit.0));

                let mut anchors = vec![
                    AxisLabelAnchor::X { x: left },
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
                        x: start.x.max(end.x),
                    });
                }

                anchors
            }
            Self::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray,
            } => {
                let mut anchors = Vec::with_capacity(2);
                if *ray {
                    anchors.push(AxisLabelAnchor::X { x: *x });
                }
                anchors.push(AxisLabelAnchor::Y {
                    panel_id: *panel_id,
                    y_unit: *y_unit,
                });
                anchors
            }
            Self::VerticalLine { x } => vec![AxisLabelAnchor::X { x: *x }],
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

    pub fn coordinates_match(&self, x: DrawingX) -> bool {
        match self {
            Self::Trendline { start, end } | Self::Box { start, end, .. } => {
                start.x.compatible(x) && end.x.compatible(x)
            }
            Self::HorizontalLine { x: origin, .. } | Self::VerticalLine { x: origin } => {
                origin.compatible(x)
            }
        }
    }

    pub fn translated(&self, origin: DrawingAnchor, current: DrawingAnchor) -> Option<Self> {
        let delta_y = current.y_unit.0.saturating_sub(origin.y_unit.0);
        let shift = |anchor: DrawingAnchor| -> Option<DrawingAnchor> {
            Some(DrawingAnchor {
                x: anchor.x.translated(origin.x, current.x)?,
                y_unit: Self::shift_y_by_delta(anchor.y_unit, delta_y),
                ..anchor
            })
        };
        Some(match self {
            Self::Trendline { start, end } => Self::Trendline {
                start: shift(*start)?,
                end: shift(*end)?,
            },
            Self::Box {
                start,
                end,
                extend_right,
            } => Self::Box {
                start: shift(*start)?,
                end: shift(*end)?,
                extend_right: *extend_right,
            },
            Self::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray,
            } => Self::HorizontalLine {
                panel_id: *panel_id,
                y_unit: Self::shift_y_by_delta(*y_unit, delta_y),
                x: x.translated(origin.x, current.x)?,
                ray: *ray,
            },
            Self::VerticalLine { x } => Self::VerticalLine {
                x: x.translated(origin.x, current.x)?,
            },
        })
    }

    pub fn handle_dragged(
        &self,
        handle: DrawingHandleKind,
        origin: DrawingAnchor,
        mut anchor: DrawingAnchor,
    ) -> Option<Self> {
        if !self.coordinates_match(anchor.x) || !origin.x.compatible(anchor.x) {
            return None;
        }
        if matches!(anchor.x, DrawingX::Tick(_)) {
            let x = match (self, handle) {
                (Self::Trendline { start, .. }, DrawingHandleKind::TrendlineStart) => start.x,
                (Self::Trendline { end, .. }, DrawingHandleKind::TrendlineEnd) => end.x,
                (Self::HorizontalLine { x, .. }, DrawingHandleKind::HorizontalLineStart) => *x,
                (
                    Self::Box { start, end, .. },
                    DrawingHandleKind::BoxTopLeft | DrawingHandleKind::BoxBottomLeft,
                ) => start.x.min(end.x),
                (
                    Self::Box { start, end, .. },
                    DrawingHandleKind::BoxTopRight | DrawingHandleKind::BoxBottomRight,
                ) => start.x.max(end.x),
                _ => return None,
            };
            anchor.x = x.translated(origin.x, anchor.x)?;
        }
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
                x: anchor.x,
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

        let mut left = if start.x <= end.x { start.x } else { end.x };
        let mut right = if start.x <= end.x { end.x } else { start.x };

        let mut top = start.y_unit.0.max(end.y_unit.0);
        let mut bottom = start.y_unit.0.min(end.y_unit.0);

        match corner {
            DrawingHandleKind::BoxTopLeft => {
                left = anchor.x;
                top = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxTopRight => {
                right = anchor.x;
                top = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxBottomRight => {
                right = anchor.x;
                bottom = anchor.y_unit.0;
            }
            DrawingHandleKind::BoxBottomLeft => {
                left = anchor.x;
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
                x: left,
                y_unit: YUnit(top),
            },
            end: DrawingAnchor {
                panel_id,
                x: right,
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
                x: anchor.x,
                y_unit: YUnit(top),
            },
            end: DrawingAnchor {
                panel_id: end.panel_id,
                x: end.x,
                y_unit: YUnit(bottom),
            },
            extend_right: true,
        })
    }

    fn shift_y_by_delta(y_unit: YUnit, delta_y: i64) -> YUnit {
        YUnit(y_unit.0.saturating_add(delta_y))
    }
}

impl DrawingEntity {
    pub fn belongs_to_panel(&self, panel_id: PanelId) -> bool {
        self.object.belongs_to_panel(panel_id)
    }
}
