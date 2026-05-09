use super::composition::PanelScaleMode;
use super::scene::Scene;
use super::{
    DRAWING_HANDLE_HIT_RADIUS_PX, DRAWING_HANDLE_RADIUS_PX, DRAWING_HIT_TOLERANCE_PX,
    KlinePanelKind, KlineSeriesLike, KlineWidget, TEXT_SIZE,
};
use crate::style;
use crate::widget::chart::kline::{YUnit, composition::PanelId};

use exchange::UnixMs;

use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};

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

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    fn x_unit_for_anchor_time(&self, scene: &Scene, time: UnixMs) -> Option<i64> {
        if let Some(unit) = scene.x_unit_for_time(time) {
            return Some(unit);
        }

        let mut best: Option<(u64, i64)> = None;

        self.for_each_bar_unit_index(scene.x_axis, |series_index, _, x_unit, bar| {
            if series_index != 0 {
                return;
            }

            let delta = bar.time.as_u64().abs_diff(time.as_u64());

            if best
                .map(|(best_delta, _)| delta < best_delta)
                .unwrap_or(true)
            {
                best = Some((delta, x_unit));
            }
        });

        best.map(|(_, unit)| unit)
    }

    fn anchor_time_for_cursor_unit(&self, scene: &Scene, x_unit: i64) -> Option<UnixMs> {
        if let Some(time) = scene.time_for_x_unit(x_unit) {
            return Some(time);
        }

        let series = self.series.first()?;
        self.bar_at_or_before_unit(series, scene.x_axis, x_unit)
            .map(|bar| bar.time)
    }

    pub(super) fn drawing_anchor_from_scene_cursor(&self, scene: &Scene) -> Option<DrawingAnchor> {
        let cursor = scene.cursor?;
        let panel_id = self.panel_id(cursor.panel_index)?;
        let time = self.anchor_time_for_cursor_unit(scene, cursor.x_unit)?;
        let y_unit = cursor.y_primary_unit.or(cursor.y_indicator_unit)?;

        Some(DrawingAnchor {
            panel_id,
            time,
            y_unit,
        })
    }

    pub(super) fn drawing_anchor_from_scene_point(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        cursor_local: Point,
    ) -> Option<DrawingAnchor> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;

        let x_plot = cursor_local.x - scene.layout.regions.plot.x;
        let right_edge_px = scene.x_axis_plot_width().floor().max(1.0);
        let spacing_px = scene.bar_spacing_px().as_f32().max(1.0);
        let steps_from_right = ((right_edge_px - x_plot) / spacing_px).round() as i64;
        let x_unit = scene.max_x_unit.saturating_sub(steps_from_right);
        let time = self.anchor_time_for_cursor_unit(scene, x_unit)?;

        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel = cursor_local.y - scene.layout.regions.plot.y - panel.plot.y;
        let ratio = 1.0 - (y_in_panel / panel_h);
        let panel_precision = self.panel_value_precision(panel_index);

        let y_unit = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let uses_display_space = matches!(
                    scene.primary_scale_mode,
                    PanelScaleMode::PercentFromBase | PanelScaleMode::Logarithmic
                ) || scene.primary_domain_display_override.is_some();

                if uses_display_space {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let display_value = min_display + (max_display - min_display) * ratio;
                    let value = scene.primary_display_to_value(display_value);
                    self.panel_value_to_unit(panel_precision, value)
                } else {
                    let min_value =
                        self.panel_unit_to_value(panel_precision, scene.min_primary_unit);
                    let max_value =
                        self.panel_unit_to_value(panel_precision, scene.max_primary_unit);
                    let value = min_value + (max_value - min_value) * ratio;
                    self.panel_value_to_unit(panel_precision, value)
                }
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let min_value = self.panel_unit_to_value(panel_precision, indicator.min_unit);
                let max_value = self.panel_unit_to_value(panel_precision, indicator.max_unit);
                let value = min_value + (max_value - min_value) * ratio;
                self.panel_value_to_unit(panel_precision, value)
            }
        };

        Some(DrawingAnchor {
            panel_id,
            time,
            y_unit,
        })
    }

    pub(super) fn panel_plot_bounds_in_overlay(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<Rectangle> {
        let panel = scene.layout.panel(panel_index)?;
        Some(Rectangle {
            x: scene.layout.regions.plot.x + panel.plot.x,
            y: scene.layout.regions.plot.y + panel.plot.y,
            width: panel.plot.width,
            height: panel.plot.height,
        })
    }

    pub(super) fn plot_bounds_in_overlay(&self, scene: &Scene) -> Rectangle {
        scene.layout.regions.plot
    }

    fn drawing_object_clip_bounds(&self, scene: &Scene, panel_id: PanelId) -> Option<Rectangle> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        self.panel_plot_bounds_in_overlay(scene, panel_index)
    }

    fn clip_segment_to_bounds(
        start: Point,
        end: Point,
        bounds: Rectangle,
    ) -> Option<(Point, Point)> {
        let left = bounds.x;
        let right = bounds.x + bounds.width;
        let top = bounds.y;
        let bottom = bounds.y + bounds.height;

        let dx = end.x - start.x;
        let dy = end.y - start.y;

        let mut t0 = 0.0_f32;
        let mut t1 = 1.0_f32;

        let mut clip = |p: f32, q: f32| -> bool {
            if p.abs() <= f32::EPSILON {
                return q >= 0.0;
            }

            let r = q / p;

            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return false;
                }
                if r < t1 {
                    t1 = r;
                }
            }

            true
        };

        if !clip(-dx, start.x - left)
            || !clip(dx, right - start.x)
            || !clip(-dy, start.y - top)
            || !clip(dy, bottom - start.y)
        {
            return None;
        }

        if t0 > t1 {
            return None;
        }

        Some((
            Point::new(start.x + t0 * dx, start.y + t0 * dy),
            Point::new(start.x + t1 * dx, start.y + t1 * dy),
        ))
    }

    fn intersect_bounds(a: Rectangle, b: Rectangle) -> Option<Rectangle> {
        let left = a.x.max(b.x);
        let right = (a.x + a.width).min(b.x + b.width);
        let top = a.y.max(b.y);
        let bottom = (a.y + a.height).min(b.y + b.height);

        if right <= left || bottom <= top {
            return None;
        }

        Some(Rectangle {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    fn anchor_to_overlay_point(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<Point> {
        let panel_index = self.panel_index_for_id(anchor.panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, anchor.y_unit);
        let x_unit = self.x_unit_for_anchor_time(scene, anchor.time)?;
        let x_plot = Self::snap_plot_x_to_cell(
            scene.map_x_plot(x_unit),
            self.resolved_horizontal_pixel_ratio(),
        );
        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                scene.map_primary_plot_with_anchor(value, scene.primary_scale_anchor)
            }
            KlinePanelKind::Indicator => scene.map_indicator_plot(panel_index, value)?,
        };

        Some(Point::new(
            scene.layout.regions.plot.x + x_plot,
            scene.layout.regions.plot.y + y_plot,
        ))
    }

    fn drawing_trendline_geometry(
        &self,
        scene: &Scene,
        start: DrawingAnchor,
        end: DrawingAnchor,
    ) -> Option<(Point, Point, Point, Point, Rectangle)> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let clip_bounds = self.drawing_object_clip_bounds(scene, start.panel_id)?;
        let start_point = self.anchor_to_overlay_point(scene, start)?;
        let end_point = self.anchor_to_overlay_point(scene, end)?;
        let (visible_start, visible_end) =
            Self::clip_segment_to_bounds(start_point, end_point, clip_bounds)?;

        Some((
            start_point,
            end_point,
            visible_start,
            visible_end,
            clip_bounds,
        ))
    }

    fn drawing_box_geometry(
        &self,
        scene: &Scene,
        start: DrawingAnchor,
        end: DrawingAnchor,
    ) -> Option<(Point, Size, Rectangle, Rectangle)> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let clip_bounds = self.drawing_object_clip_bounds(scene, start.panel_id)?;
        let start_point = self.anchor_to_overlay_point(scene, start)?;
        let end_point = self.anchor_to_overlay_point(scene, end)?;

        let left = start_point.x.min(end_point.x);
        let right = start_point.x.max(end_point.x);
        let top = start_point.y.min(end_point.y);
        let bottom = start_point.y.max(end_point.y);

        let origin = Point::new(left, top);
        let size = Size::new((right - left).max(1.0), (bottom - top).max(1.0));
        let object_bounds = Rectangle {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        };
        let visible_bounds = Self::intersect_bounds(object_bounds, clip_bounds)?;

        Some((origin, size, clip_bounds, visible_bounds))
    }

    fn drawing_horizontal_line_geometry(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        y_unit: YUnit,
    ) -> Option<(Rectangle, f32)> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let clip_bounds = self.panel_plot_bounds_in_overlay(scene, panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, y_unit);

        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                scene.map_primary_plot_with_anchor(value, scene.primary_scale_anchor)
            }
            KlinePanelKind::Indicator => scene.map_indicator_plot(panel_index, value)?,
        };

        let panel_top = panel.plot.y;
        let panel_bottom = panel.plot.y + panel.plot.height;
        if y_plot < panel_top || y_plot > panel_bottom {
            return None;
        }

        Some((clip_bounds, scene.layout.regions.plot.y + y_plot))
    }

    fn drawing_vertical_line_geometry(
        &self,
        scene: &Scene,
        time: UnixMs,
    ) -> Option<(Rectangle, f32)> {
        let x_unit = self.x_unit_for_anchor_time(scene, time)?;
        let clip_bounds = self.plot_bounds_in_overlay(scene);
        let x = scene.layout.regions.plot.x
            + Self::snap_plot_x_to_cell(
                scene.map_x_plot(x_unit),
                self.resolved_horizontal_pixel_ratio(),
            );

        if x < clip_bounds.x || x > (clip_bounds.x + clip_bounds.width) {
            return None;
        }

        Some((clip_bounds, x))
    }

    fn draw_drawing_object(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        object: &DrawingObject,
        style: DrawingStyle,
        selected: bool,
        overlay_origin_in_window: Point,
        horizontal_pixel_ratio: f32,
    ) {
        let raw_stroke_width = if selected {
            (style.stroke_width + 0.6).max(1.0)
        } else {
            style.stroke_width.max(1.0)
        };
        let (stroke_width, stroke_width_phys) =
            Self::quantized_stroke_width(raw_stroke_width, horizontal_pixel_ratio);
        let stroke = canvas::Stroke::default()
            .with_color(style.stroke_color)
            .with_width(stroke_width);

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some((start, end, _, _, bounds)) =
                    self.drawing_trendline_geometry(scene, *start, *end)
                else {
                    return;
                };
                let start = Self::snap_point_for_stroke_with_origin(
                    start,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window,
                    stroke_width_phys,
                );
                let end = Self::snap_point_for_stroke_with_origin(
                    end,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    frame.stroke(&canvas::Path::line(start, end), stroke);
                });
            }
            DrawingObject::Box { start, end } => {
                let Some((origin, size, bounds, _)) =
                    self.drawing_box_geometry(scene, *start, *end)
                else {
                    return;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let (fill_left, fill_width) = Self::snapped_span_with_origin(
                    left,
                    right,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                );
                let (fill_top, fill_height) = Self::snapped_span_with_origin(
                    top,
                    bottom,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                );

                let left_stroke = Self::snap_stroke_center_with_origin(
                    left,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );
                let right_stroke = Self::snap_stroke_center_with_origin(
                    right,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );
                let top_stroke = Self::snap_stroke_center_with_origin(
                    top,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );
                let bottom_stroke = Self::snap_stroke_center_with_origin(
                    bottom,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    if let Some(fill) = style.fill_color {
                        frame.fill_rectangle(
                            Point::new(fill_left, fill_top),
                            Size::new(fill_width, fill_height),
                            fill,
                        );
                    }

                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(left_stroke, top_stroke),
                            Point::new(right_stroke, top_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(right_stroke, top_stroke),
                            Point::new(right_stroke, bottom_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(right_stroke, bottom_stroke),
                            Point::new(left_stroke, bottom_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(left_stroke, bottom_stroke),
                            Point::new(left_stroke, top_stroke),
                        ),
                        stroke,
                    );
                });
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some((bounds, y)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit)
                else {
                    return;
                };

                let y = Self::snap_stroke_center_with_origin(
                    y,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(bounds.x, y),
                            Point::new(bounds.x + bounds.width, y),
                        ),
                        stroke,
                    );
                });
            }
            DrawingObject::VerticalLine { time } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *time) else {
                    return;
                };

                let x = Self::snap_stroke_center_with_origin(
                    x,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );

                frame.with_clip(plot, |frame| {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(x, plot.y),
                            Point::new(x, plot.y + plot.height),
                        ),
                        stroke,
                    );
                });
            }
        }
    }

    fn drawing_handle_points(
        &self,
        scene: &Scene,
        object: &DrawingObject,
    ) -> Vec<(DrawingHandleKind, Point)> {
        match object {
            DrawingObject::Trendline { start, end } => {
                let mut points = Vec::with_capacity(2);

                if let Some(start_point) = self.anchor_to_overlay_point(scene, *start) {
                    points.push((DrawingHandleKind::TrendlineStart, start_point));
                }

                if let Some(end_point) = self.anchor_to_overlay_point(scene, *end) {
                    points.push((DrawingHandleKind::TrendlineEnd, end_point));
                }

                points
            }
            DrawingObject::Box { start, end } => {
                if start.panel_id != end.panel_id {
                    return Vec::new();
                }

                let Some(start_point) = self.anchor_to_overlay_point(scene, *start) else {
                    return Vec::new();
                };
                let Some(end_point) = self.anchor_to_overlay_point(scene, *end) else {
                    return Vec::new();
                };

                let left = start_point.x.min(end_point.x);
                let right = start_point.x.max(end_point.x);
                let top = start_point.y.min(end_point.y);
                let bottom = start_point.y.max(end_point.y);

                vec![
                    (DrawingHandleKind::BoxTopLeft, Point::new(left, top)),
                    (DrawingHandleKind::BoxTopRight, Point::new(right, top)),
                    (DrawingHandleKind::BoxBottomRight, Point::new(right, bottom)),
                    (DrawingHandleKind::BoxBottomLeft, Point::new(left, bottom)),
                ]
            }
            DrawingObject::HorizontalLine { .. } | DrawingObject::VerticalLine { .. } => Vec::new(),
        }
    }

    fn drawing_handle_clip_bounds(
        &self,
        scene: &Scene,
        object: &DrawingObject,
    ) -> Option<Rectangle> {
        let panel_id = object.handle_panel_id()?;

        let panel_index = self.panel_index_for_id(panel_id)?;
        self.panel_plot_bounds_in_overlay(scene, panel_index)
    }

    fn draw_selected_drawing_handles(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        horizontal_pixel_ratio: f32,
        drawing: &DrawingEntity,
    ) {
        let handles = self.drawing_handle_points(scene, &drawing.object);
        if handles.is_empty() {
            return;
        }

        let Some(clip_bounds) = self.drawing_handle_clip_bounds(scene, &drawing.object) else {
            return;
        };

        let fill_color = palette.background.strongest.color;
        let stroke_color = palette.primary.base.color.scale_alpha(0.92);
        let (handle_stroke_width, _) = Self::quantized_stroke_width(1.2, horizontal_pixel_ratio);

        frame.with_clip(clip_bounds, |frame| {
            for (_, point) in handles {
                let circle = canvas::Path::circle(point, DRAWING_HANDLE_RADIUS_PX);

                frame.fill(&circle, fill_color);
                frame.stroke(
                    &circle,
                    canvas::Stroke::default()
                        .with_color(stroke_color)
                        .with_width(handle_stroke_width),
                );
            }
        });
    }

    pub(super) fn hit_test_selected_drawing_handle(
        &self,
        scene: &Scene,
        point: Point,
    ) -> Option<(DrawingId, DrawingHandleKind)> {
        let drawing = self.drawings.selected_visible_drawing()?;
        if drawing.locked {
            return None;
        }

        let hit_radius_sq = DRAWING_HANDLE_HIT_RADIUS_PX * DRAWING_HANDLE_HIT_RADIUS_PX;
        let mut best: Option<(f32, DrawingHandleKind)> = None;

        for (kind, handle_point) in self.drawing_handle_points(scene, &drawing.object) {
            let dx = point.x - handle_point.x;
            let dy = point.y - handle_point.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > hit_radius_sq {
                continue;
            }

            if best
                .map(|(best_dist_sq, _)| dist_sq < best_dist_sq)
                .unwrap_or(true)
            {
                best = Some((dist_sq, kind));
            }
        }

        best.map(|(_, kind)| (drawing.id, kind))
    }

    pub(super) fn fill_drawings(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        overlay_origin_in_window: Point,
        horizontal_pixel_ratio: f32,
    ) {
        let drawing_state = &self.drawings;

        for drawing in drawing_state
            .entities
            .iter()
            .filter(|drawing| drawing.visible)
        {
            let selected = drawing_state.selected_drawing == Some(drawing.id);
            self.draw_drawing_object(
                frame,
                scene,
                &drawing.object,
                drawing.style,
                selected,
                overlay_origin_in_window,
                horizontal_pixel_ratio,
            );
        }

        if let Some(selected_drawing) = drawing_state.selected_visible_drawing() {
            self.draw_selected_drawing_handles(
                frame,
                scene,
                palette,
                horizontal_pixel_ratio,
                selected_drawing,
            );
        }

        if let Some(draft) = drawing_state.drawing_draft {
            let mut style = draft.style();
            style.stroke_color = style.stroke_color.scale_alpha(0.9);
            if let Some(fill) = style.fill_color {
                style.fill_color = Some(fill.scale_alpha(0.5));
            }

            self.draw_drawing_object(
                frame,
                scene,
                &draft.preview_object(),
                style,
                false,
                overlay_origin_in_window,
                horizontal_pixel_ratio,
            );
        }
    }

    fn point_segment_distance(point: Point, a: Point, b: Point) -> f32 {
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let wx = point.x - a.x;
        let wy = point.y - a.y;

        let len_sq = vx * vx + vy * vy;
        if len_sq <= f32::EPSILON {
            return ((point.x - a.x).powi(2) + (point.y - a.y).powi(2)).sqrt();
        }

        let t = ((wx * vx + wy * vy) / len_sq).clamp(0.0, 1.0);
        let proj_x = a.x + t * vx;
        let proj_y = a.y + t * vy;

        ((point.x - proj_x).powi(2) + (point.y - proj_y).powi(2)).sqrt()
    }

    fn drawing_hit_test_object(
        &self,
        scene: &Scene,
        object: &DrawingObject,
        style: DrawingStyle,
        point: Point,
    ) -> bool {
        let tolerance = DRAWING_HIT_TOLERANCE_PX;

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some((_, _, visible_start, visible_end, _)) =
                    self.drawing_trendline_geometry(scene, *start, *end)
                else {
                    return false;
                };

                Self::point_segment_distance(point, visible_start, visible_end) <= tolerance
            }
            DrawingObject::Box { start, end } => {
                let Some((origin, size, bounds, visible_bounds)) =
                    self.drawing_box_geometry(scene, *start, *end)
                else {
                    return false;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let within_expanded = point.x >= (visible_bounds.x - tolerance)
                    && point.x <= (visible_bounds.x + visible_bounds.width + tolerance)
                    && point.y >= (visible_bounds.y - tolerance)
                    && point.y <= (visible_bounds.y + visible_bounds.height + tolerance);

                if !within_expanded {
                    return false;
                }

                let top_left = Point::new(left, top);
                let top_right = Point::new(right, top);
                let bottom_right = Point::new(right, bottom);
                let bottom_left = Point::new(left, bottom);

                let near_edge = [
                    (top_left, top_right),
                    (top_right, bottom_right),
                    (bottom_right, bottom_left),
                    (bottom_left, top_left),
                ]
                .into_iter()
                .filter_map(|(edge_start, edge_end)| {
                    Self::clip_segment_to_bounds(edge_start, edge_end, bounds)
                })
                .any(|(edge_start, edge_end)| {
                    Self::point_segment_distance(point, edge_start, edge_end) <= tolerance
                });

                let inside_visible_fill = style.fill_color.is_some()
                    && point.x >= visible_bounds.x
                    && point.x <= (visible_bounds.x + visible_bounds.width)
                    && point.y >= visible_bounds.y
                    && point.y <= (visible_bounds.y + visible_bounds.height);

                near_edge || inside_visible_fill
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some((bounds, y)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit)
                else {
                    return false;
                };

                point.x >= (bounds.x - tolerance)
                    && point.x <= (bounds.x + bounds.width + tolerance)
                    && (point.y - y).abs() <= tolerance
            }
            DrawingObject::VerticalLine { time } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *time) else {
                    return false;
                };

                (point.x - x).abs() <= tolerance
                    && point.y >= (plot.y - tolerance)
                    && point.y <= (plot.y + plot.height + tolerance)
            }
        }
    }

    pub(super) fn hit_test_drawings(&self, scene: &Scene, point: Point) -> Option<DrawingId> {
        self.drawings
            .entities
            .iter()
            .rev()
            .filter(|drawing| drawing.visible)
            .find(|drawing| {
                self.drawing_hit_test_object(scene, &drawing.object, drawing.style, point)
            })
            .map(|drawing| drawing.id)
    }

    fn format_anchor_y_label(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<String> {
        let panel_index = self.panel_index_for_id(anchor.panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, anchor.y_unit);

        match panel.kind {
            KlinePanelKind::PrimaryChart => Some(scene.format_primary_cursor_label(value)),
            KlinePanelKind::Indicator => {
                Some(self.format_panel_axis_value(panel_index, panel_precision, value, 0.01))
            }
        }
    }

    pub(super) fn draw_x_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        x: f32,
        text: &str,
    ) {
        let x_label_w = (text.len() as f32 * TEXT_SIZE * 0.62).clamp(60.0, 180.0);
        let x_label_h = TEXT_SIZE + 6.0;
        let x_label_x = (x - x_label_w / 2.0).clamp(
            scene.layout.regions.plot.x,
            scene.layout.regions.plot.x + scene.layout.regions.plot.width - x_label_w,
        );
        let x_label_y = scene.layout.regions.x_axis.y + 2.0;

        frame.fill_rectangle(
            Point::new(x_label_x, x_label_y),
            Size::new(x_label_w, x_label_h),
            palette.background.strong.color,
        );

        frame.fill_text(canvas::Text {
            content: text.to_string(),
            position: Point::new(x_label_x + x_label_w / 2.0, x_label_y + x_label_h / 2.0),
            color: palette.background.strong.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    pub(super) fn draw_y_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        y: f32,
        text: &str,
        clamp_bounds: Rectangle,
    ) {
        let y_label_w = (text.len() as f32 * TEXT_SIZE * 0.6).clamp(40.0, 96.0);
        let y_label_h = TEXT_SIZE + 6.0;
        let y_label_x = scene.layout.regions.y_axis.x + 2.0;
        let y_min = clamp_bounds.y;
        let y_max = (clamp_bounds.y + clamp_bounds.height - y_label_h).max(y_min);
        let y_label_y = (y - (y_label_h / 2.0)).clamp(y_min, y_max);

        frame.fill_rectangle(
            Point::new(y_label_x, y_label_y),
            Size::new(y_label_w, y_label_h),
            palette.background.strong.color,
        );

        frame.fill_text(canvas::Text {
            content: text.to_string(),
            position: Point::new(y_label_x + y_label_w - 4.0, y_label_y + y_label_h / 2.0),
            color: palette.background.strong.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::End.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    pub(super) fn fill_active_drawing_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(object) = self.drawings.active_axis_labeled_object() else {
            return;
        };
        let Some((start, end)) = object.axis_label_anchors() else {
            return;
        };

        for anchor in [start, end] {
            let Some(point) = self.anchor_to_overlay_point(scene, anchor) else {
                continue;
            };

            if let Some(x_unit) = self.x_unit_for_anchor_time(scene, anchor.time) {
                let x_text = self.format_x_label(scene.x_axis, x_unit, 1);
                self.draw_x_axis_badge(frame, scene, palette, point.x, &x_text);
            }

            if let Some(y_text) = self.format_anchor_y_label(scene, anchor) {
                let Some(panel_index) = self.panel_index_for_id(anchor.panel_id) else {
                    continue;
                };
                let Some(panel_bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index)
                else {
                    continue;
                };

                self.draw_y_axis_badge(frame, scene, palette, point.y, &y_text, panel_bounds);
            }
        }
    }
}
