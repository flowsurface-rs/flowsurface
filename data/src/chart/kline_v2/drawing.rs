use super::YUnit;
use super::composition::PanelId;
use crate::config::theme::ThemeAccent;
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
    Accent { accent: ThemeAccent, alpha: f32 },
    Custom(Rgba),
}

impl DrawingColor {
    pub const fn accent(accent: ThemeAccent, alpha: f32) -> Self {
        Self::Accent { accent, alpha }
    }

    pub fn accent_value(self) -> Option<ThemeAccent> {
        match self {
            Self::Accent { accent, .. } => Some(accent),
            Self::Custom(_) => None,
        }
    }

    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            Self::Accent { accent, alpha } => Self::Accent {
                accent,
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
            } => DrawingObject::Box {
                start: *start,
                end: *current,
                extend_right: *extend_right,
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
                    DrawingObject::Box {
                        start,
                        end,
                        extend_right,
                    },
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisLabelAnchor {
    X { time: UnixMs },
    Y { panel_id: PanelId, y_unit: YUnit },
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
            Self::Trendline { start, end } | Self::Box { start, end, .. } => {
                (start.panel_id == end.panel_id).then_some(start.panel_id)
            }
            Self::HorizontalLine { .. } | Self::VerticalLine { .. } => None,
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
            extend_right,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_stroke_width_is_normalized() {
        let mut style = DrawingStyle::default();

        assert!(style.set_stroke_width(0.0));
        assert_eq!(style.stroke_width, MIN_DRAWING_STROKE_WIDTH);

        assert!(style.set_stroke_width(100.0));
        assert_eq!(style.stroke_width, MAX_DRAWING_STROKE_WIDTH);

        style.stroke_width = f32::NAN;
        assert_eq!(
            style.normalized_stroke_width(),
            DrawingStyle::default().stroke_width
        );
    }

    #[test]
    fn drawing_colors_keep_legacy_custom_values_and_support_accents() {
        let legacy: DrawingStyle = serde_json::from_value(serde_json::json!({
            "stroke_color": {"red": 0.1, "green": 0.2, "blue": 0.3, "alpha": 1.0},
            "stroke_width": 1.0,
            "fill_color": null
        }))
        .expect("legacy drawing style should deserialize");

        assert!(matches!(
            legacy.stroke_color,
            DrawingColor::Custom(Rgba { red, .. }) if red == 0.1
        ));

        let accent = DrawingStyle {
            stroke_color: DrawingColor::accent(ThemeAccent::Three, 1.0),
            ..DrawingStyle::default()
        };
        let serialized = serde_json::to_value(accent).expect("accent style should serialize");
        assert_eq!(serialized["stroke_color"]["accent"], "Three");
    }

    #[test]
    fn drawing_objects_deserialize_legacy_fields_with_defaults() {
        let anchor = |time: u64| DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(time),
            y_unit: YUnit(10),
        };

        let legacy_box: DrawingObject = serde_json::from_value(serde_json::json!({
            "Box": {"start": anchor(1), "end": anchor(2)}
        }))
        .expect("legacy box should deserialize");
        assert_eq!(
            legacy_box,
            DrawingObject::Box {
                start: anchor(1),
                end: anchor(2),
                extend_right: false,
            }
        );

        let legacy_line: DrawingObject = serde_json::from_value(serde_json::json!({
            "HorizontalLine": {"panel_id": 1, "y_unit": 10}
        }))
        .expect("legacy horizontal line should deserialize");
        assert_eq!(
            legacy_line,
            DrawingObject::HorizontalLine {
                panel_id: PanelId(1),
                y_unit: YUnit(10),
                time: UnixMs::ZERO,
                ray: false,
            }
        );
    }

    #[test]
    fn drawing_object_extend_flags_survive_round_trip() {
        let ray_line = DrawingObject::HorizontalLine {
            panel_id: PanelId(1),
            y_unit: YUnit(10),
            time: UnixMs(1234),
            ray: true,
        };
        let box_extended = DrawingObject::Box {
            start: DrawingAnchor {
                panel_id: PanelId(1),
                time: UnixMs(1),
                y_unit: YUnit(10),
            },
            end: DrawingAnchor {
                panel_id: PanelId(1),
                time: UnixMs(2),
                y_unit: YUnit(4),
            },
            extend_right: true,
        };

        for object in [ray_line, box_extended] {
            let serialized = serde_json::to_value(&object).expect("object should serialize");
            let round_trip: DrawingObject =
                serde_json::from_value(serialized).expect("object should deserialize");
            assert_eq!(object, round_trip);
        }
    }

    #[test]
    fn translating_preserves_extend_flags() {
        let origin = DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(10),
            y_unit: YUnit(5),
        };
        let current = DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(20),
            y_unit: YUnit(7),
        };

        let ray_line = DrawingObject::HorizontalLine {
            panel_id: PanelId(1),
            y_unit: YUnit(5),
            time: UnixMs(10),
            ray: true,
        };
        let translated = ray_line.translated(origin, current);
        assert_eq!(
            translated,
            DrawingObject::HorizontalLine {
                panel_id: PanelId(1),
                y_unit: YUnit(7),
                time: UnixMs(20),
                ray: true,
            }
        );

        let box_extended = DrawingObject::Box {
            start: anchor(1),
            end: anchor(2),
            extend_right: true,
        };
        let shifted = |time: u64| DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(time),
            y_unit: YUnit(12),
        };
        assert_eq!(
            box_extended.translated(origin, current),
            DrawingObject::Box {
                start: shifted(11),
                end: shifted(12),
                extend_right: true,
            }
        );
    }

    #[test]
    fn axis_label_anchors_reflect_ray_and_extend_semantics() {
        let start = DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(100),
            y_unit: YUnit(40),
        };
        let end = DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(200),
            y_unit: YUnit(10),
        };

        let x = |time: u64| AxisLabelAnchor::X { time: UnixMs(time) };
        let y = |y_unit: i64| AxisLabelAnchor::Y {
            panel_id: PanelId(1),
            y_unit: YUnit(y_unit),
        };

        assert_eq!(
            DrawingObject::Trendline { start, end }.axis_label_anchors(),
            vec![x(100), y(40), x(200), y(10)]
        );

        assert_eq!(
            DrawingObject::Box {
                start,
                end,
                extend_right: false,
            }
            .axis_label_anchors(),
            vec![x(100), y(40), y(10), x(200)]
        );

        assert_eq!(
            DrawingObject::Box {
                start,
                end,
                extend_right: true,
            }
            .axis_label_anchors(),
            vec![x(100), y(40), y(10)]
        );

        assert_eq!(
            DrawingObject::HorizontalLine {
                panel_id: PanelId(1),
                y_unit: YUnit(25),
                time: UnixMs(150),
                ray: true,
            }
            .axis_label_anchors(),
            vec![x(150), y(25)]
        );

        assert_eq!(
            DrawingObject::HorizontalLine {
                panel_id: PanelId(1),
                y_unit: YUnit(25),
                time: UnixMs(150),
                ray: false,
            }
            .axis_label_anchors(),
            vec![y(25)]
        );

        assert_eq!(
            DrawingObject::VerticalLine { time: UnixMs(175) }.axis_label_anchors(),
            vec![x(175)]
        );
    }

    fn anchor(time: u64) -> DrawingAnchor {
        DrawingAnchor {
            panel_id: PanelId(1),
            time: UnixMs(time),
            y_unit: YUnit(10),
        }
    }
}
