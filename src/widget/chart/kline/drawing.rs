use super::composition::PanelScaleMode;
use super::composition::{MarkKind, PanelId};
use super::helpers::PixelGeometry;
use super::layout::contains_inclusive;
use super::scene::Scene;
use super::{KlinePanelKind, KlineWidget, TEXT_SIZE, Y_AXIS_LABEL_LEFT_INSET, YUnit};
use crate::style;

use data::config::theme::{ACCENT_COUNT, ThemeColor, ThemeColorRole};
use exchange::UnixMs;

use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};

const DRAWING_HIT_TOLERANCE_PX: f32 = 7.0;
const DRAWING_HANDLE_RADIUS_PX: f32 = 4.0;
const DRAWING_HANDLE_HIT_RADIUS_PX: f32 = 8.0;

#[derive(Default)]
pub(super) struct AxisBadgeFilter {
    occupied: Vec<Rectangle>,
}

impl AxisBadgeFilter {
    pub(super) fn reserve(&mut self, bounds: Rectangle) -> bool {
        if self
            .occupied
            .iter()
            .any(|occupied| occupied.intersects(&bounds))
        {
            return false;
        }

        self.occupied.push(bounds);
        true
    }
}

pub use data::chart::kline::drawing::{
    AxisLabelAnchor, DrawingAnchor, DrawingColor, DrawingDraft, DrawingDragOperation,
    DrawingDragTarget, DrawingEntity, DrawingHandleKind, DrawingId, DrawingMeasure, DrawingObject,
    DrawingSnapshot, DrawingStyle, DrawingTool, DrawingX, KlineWidgetDrawingEvent,
    MAX_DRAWING_STROKE_WIDTH, MIN_DRAWING_STROKE_WIDTH, MeasureState,
};

fn iced_color(
    color: DrawingColor,
    palette: &Extended,
    generated_accents: &[iced::Color; ACCENT_COUNT],
) -> iced::Color {
    match color {
        DrawingColor::Accent {
            accent,
            shade,
            alpha,
        } => crate::widget::resolve_theme_color(
            ThemeColor::with_accent(ThemeColorRole::Primary, Some(accent), shade),
            palette,
            generated_accents,
        )
        .scale_alpha(alpha),
        DrawingColor::Custom(color) => {
            iced::Color::from_rgba(color.red, color.green, color.blue, color.alpha)
        }
    }
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

fn nearest_y_unit(target: YUnit, candidates: impl IntoIterator<Item = YUnit>) -> Option<YUnit> {
    candidates
        .into_iter()
        .min_by_key(|candidate| candidate.0.abs_diff(target.0))
}

impl<'a> KlineWidget<'a> {
    fn x_unit_for_anchor(&self, scene: &Scene, x: DrawingX) -> Option<f64> {
        match (self.basis, x) {
            (data::chart::Basis::Time(_), DrawingX::Time(time)) => {
                scene.x_unit_for_time(time).map(|unit| unit as f64)
            }
            (data::chart::Basis::Tick(interval), DrawingX::Tick(_)) => {
                let count = self.series.first()?.point_count();
                let super::scene::XAxis::Tick { anchor } = scene.x.axis else {
                    return None;
                };
                if count == 0 {
                    return None;
                }
                Some(
                    x.tick_bar_position(interval.0 as u64)? - (count - 1) as f64
                        + anchor.as_u64() as f64,
                )
            }
            _ => None,
        }
    }

    fn anchor_x_for_cursor_unit(&self, scene: &Scene, x_unit: i64) -> Option<DrawingX> {
        match self.basis {
            data::chart::Basis::Time(_) => scene.time_for_x_unit(x_unit).map(DrawingX::Time),
            data::chart::Basis::Tick(interval) => {
                let count = self.series.first()?.point_count();
                let super::scene::XAxis::Tick { anchor } = scene.x.axis else {
                    return None;
                };
                if count == 0 {
                    return None;
                }
                let index = x_unit as f64 + (count - 1) as f64 - anchor.as_u64() as f64;
                let x = DrawingX::Tick(index * interval.0 as f64);
                x.is_valid().then_some(x)
            }
        }
    }

    fn drawing_timestamp(&self, x: DrawingX) -> Option<UnixMs> {
        match (self.basis, x) {
            (data::chart::Basis::Time(_), DrawingX::Time(time)) => Some(time),
            (data::chart::Basis::Tick(interval), DrawingX::Tick(_)) => {
                let index = x.tick_bar_position(interval.0 as u64)?;
                let series = self.series.first()?;
                if index < 0.0 || index >= series.point_count() as f64 {
                    return None;
                }
                series
                    .point_at(index.floor() as usize)
                    .map(|point| point.kline.time)
            }
            _ => None,
        }
    }

    pub(super) fn drawing_anchor_from_scene_cursor(&self, scene: &Scene) -> Option<DrawingAnchor> {
        let cursor = scene.interaction.cursor?;
        let panel_id = self.panel_id(cursor.panel_index)?;
        let x = self.anchor_x_for_cursor_unit(scene, cursor.x_unit)?;
        let y_unit = cursor.y_primary_unit.or(cursor.y_indicator_unit)?;

        Some(DrawingAnchor {
            panel_id,
            x,
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
        let x_unit = scene.unit_from_plot_x(x_plot);
        let x = self.anchor_x_for_cursor_unit(scene, x_unit)?;

        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel = cursor_local.y - scene.layout.regions.plot.y - panel.plot.y;
        let ratio = (1.0 - (y_in_panel / panel_h)).clamp(0.0, 1.0);
        let value_scale = self.panel_value_scale(panel_index);

        let y_unit = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let primary_ratio = scene.primary_value_ratio_at_plot_y(y_in_panel);
                let uses_display_space = matches!(
                    scene.primary.mode,
                    PanelScaleMode::PercentFromBase | PanelScaleMode::Logarithmic
                ) || scene.primary.display_domain_override.is_some();

                if uses_display_space {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let display_value = min_display + (max_display - min_display) * primary_ratio;
                    let value = scene.primary_display_to_value(display_value);
                    value_scale.value_to_unit_f32(value)
                } else {
                    let (min_value, max_value) = scene
                        .primary
                        .value_scale
                        .values_f32(scene.primary.domain.active());
                    let value = min_value + (max_value - min_value) * primary_ratio;
                    value_scale.value_to_unit_f32(value)
                }
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let min_value = indicator.min_value();
                let max_value = indicator.max_value();
                let value = min_value + (max_value - min_value) * f64::from(ratio);
                value_scale.value_to_unit(value)
            }
        };

        Some(DrawingAnchor {
            panel_id,
            x,
            y_unit,
        })
    }

    pub(super) fn drawing_draft_anchor_from_scene_point(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        cursor_local: Point,
        magnetic: bool,
    ) -> Option<DrawingAnchor> {
        let anchor = self.drawing_anchor_from_scene_point(scene, panel_id, cursor_local)?;

        if !magnetic {
            return Some(anchor);
        }

        let x_plot = cursor_local.x - scene.layout.regions.plot.x;
        let x_unit = scene.unit_from_plot_x(x_plot);
        Some(
            self.primary_magnetic_drawing_anchor(scene, x_unit, anchor)
                .unwrap_or(anchor),
        )
    }

    pub(super) fn primary_magnetic_drawing_anchor_at_cursor(
        &self,
        scene: &Scene,
    ) -> Option<DrawingAnchor> {
        let cursor = scene.interaction.cursor?;
        let anchor = self.drawing_anchor_from_scene_cursor(scene)?;
        self.primary_magnetic_drawing_anchor(scene, cursor.x_unit, anchor)
    }

    fn primary_magnetic_drawing_anchor(
        &self,
        scene: &Scene,
        x_unit: i64,
        mut anchor: DrawingAnchor,
    ) -> Option<DrawingAnchor> {
        if self.panel_index_for_id(anchor.panel_id) != Some(scene.primary.panel_index) {
            return None;
        }

        let series = self.series.first()?;
        let units = scene.bar_units.first()?;
        let unit_index = units.values.binary_search(&x_unit).ok()?;
        let source_index = units.global_index(unit_index)?;
        let bar = series.point_at(source_index)?.kline;
        let value_scale = self.panel_value_scale(scene.primary.panel_index);
        let line_values = [bar.close.to_f32_lossy()];
        let ohlc_values = [
            bar.open.to_f32_lossy(),
            bar.high.to_f32_lossy(),
            bar.low.to_f32_lossy(),
            bar.close.to_f32_lossy(),
        ];
        let values = if matches!(scene.primary.mark, MarkKind::Line) {
            &line_values[..]
        } else {
            &ohlc_values[..]
        };

        anchor.y_unit = nearest_y_unit(
            anchor.y_unit,
            values
                .iter()
                .map(|value| value_scale.value_to_unit_f32(*value)),
        )?;
        Some(anchor)
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

    fn x_overlay_for_anchor(&self, scene: &Scene, x: DrawingX) -> Option<f32> {
        let x_unit = self.x_unit_for_anchor(scene, x)?;
        Some(scene.layout.regions.plot.x + scene.map_x_plot_f32(x_unit as f32))
    }

    pub(super) fn y_overlay_for_value(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        y_unit: YUnit,
    ) -> Option<f32> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let value_scale = self.panel_value_scale(panel_index);
        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let value = value_scale.unit_to_value_f32(y_unit);
                scene.map_primary_plot_with_anchor_f32(value, scene.primary.anchor)
            }
            KlinePanelKind::Indicator => {
                let value = value_scale.unit_to_value(y_unit);
                scene.map_indicator_plot(panel_index, value)?
            }
        };

        Some(scene.layout.regions.plot.y + y_plot)
    }

    fn anchor_to_overlay_point(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<Point> {
        let x = self.x_overlay_for_anchor(scene, anchor.x)?;
        let y = self.y_overlay_for_value(scene, anchor.panel_id, anchor.y_unit)?;

        Some(Point::new(x, y))
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
        extend_right: bool,
    ) -> Option<(Point, Size, Rectangle, Rectangle)> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let clip_bounds = self.drawing_object_clip_bounds(scene, start.panel_id)?;
        let start_point = self.anchor_to_overlay_point(scene, start)?;
        let end_point = self.anchor_to_overlay_point(scene, end)?;

        let left = start_point.x.min(end_point.x);
        let right = if extend_right {
            clip_bounds.x + clip_bounds.width
        } else {
            start_point.x.max(end_point.x)
        };
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
        let visible_bounds = intersect_bounds(object_bounds, clip_bounds)?;

        Some((origin, size, clip_bounds, visible_bounds))
    }

    fn drawing_horizontal_line_geometry(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        y_unit: YUnit,
        x: DrawingX,
        ray: bool,
    ) -> Option<(Rectangle, f32, f32)> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let clip_bounds = self.panel_plot_bounds_in_overlay(scene, panel_index)?;
        let value_scale = self.panel_value_scale(panel_index);

        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let value = value_scale.unit_to_value_f32(y_unit);
                scene.map_primary_plot_with_anchor_f32(value, scene.primary.anchor)
            }
            KlinePanelKind::Indicator => {
                let value = value_scale.unit_to_value(y_unit);
                scene.map_indicator_plot(panel_index, value)?
            }
        };

        let panel_top = panel.plot.y;
        let panel_bottom = panel.plot.y + panel.plot.height;
        if y_plot < panel_top || y_plot > panel_bottom {
            return None;
        }

        let start_x = if ray {
            let x_unit = self.x_unit_for_anchor(scene, x)?;
            let x = scene.layout.regions.plot.x + scene.map_x_plot_f32(x_unit as f32);
            x.max(clip_bounds.x)
        } else {
            clip_bounds.x
        };

        Some((clip_bounds, scene.layout.regions.plot.y + y_plot, start_x))
    }

    fn drawing_vertical_line_geometry(
        &self,
        scene: &Scene,
        x: DrawingX,
    ) -> Option<(Rectangle, f32)> {
        let x_unit = self.x_unit_for_anchor(scene, x)?;
        let clip_bounds = self.plot_bounds_in_overlay(scene);
        let x = scene.layout.regions.plot.x + scene.map_x_plot_f32(x_unit as f32);

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
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        selected: bool,
        pixels: PixelGeometry,
    ) {
        let raw_stroke_width = if selected {
            style.normalized_stroke_width() + 0.6
        } else {
            style.normalized_stroke_width()
        };
        let (stroke_width, stroke_width_phys) = pixels.quantized_stroke_width(raw_stroke_width);
        let stroke = canvas::Stroke::default()
            .with_color(iced_color(style.stroke_color, palette, generated_accents))
            .with_width(stroke_width);

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some((start, end, _, _, bounds)) =
                    self.drawing_trendline_geometry(scene, *start, *end)
                else {
                    return;
                };
                let start = pixels.snap_point_for_stroke(start, stroke_width_phys);
                let end = pixels.snap_point_for_stroke(end, stroke_width_phys);

                frame.with_clip(bounds, |frame| {
                    frame.stroke(&canvas::Path::line(start, end), stroke);
                });
            }
            DrawingObject::Box {
                start,
                end,
                extend_right,
            } => {
                let Some((origin, size, bounds, _)) =
                    self.drawing_box_geometry(scene, *start, *end, *extend_right)
                else {
                    return;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let fill_span = pixels.snapped_x_span(left, right);
                let fill_top = pixels.snapped_y_span(top, bottom);
                let left_stroke = pixels.snap_stroke_center_x(left, stroke_width_phys);
                let right_stroke = pixels.snap_stroke_center_x(right, stroke_width_phys);
                let top_stroke = pixels.snap_stroke_center_y(top, stroke_width_phys);
                let bottom_stroke = pixels.snap_stroke_center_y(bottom, stroke_width_phys);

                frame.with_clip(bounds, |frame| {
                    if let Some(fill) = style.fill_color {
                        frame.fill_rectangle(
                            Point::new(fill_span.start, fill_top.start),
                            Size::new(fill_span.size, fill_top.size),
                            iced_color(fill, palette, generated_accents),
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
            DrawingObject::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray,
            } => {
                let Some((bounds, y, start_x)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit, *x, *ray)
                else {
                    return;
                };

                let y = pixels.snap_stroke_center_y(y, stroke_width_phys);
                let start_x = pixels.snap_stroke_center_x(start_x, stroke_width_phys);

                frame.with_clip(bounds, |frame| {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(start_x, y),
                            Point::new(bounds.x + bounds.width, y),
                        ),
                        stroke,
                    );
                });
            }
            DrawingObject::VerticalLine { x } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *x) else {
                    return;
                };

                let x = pixels.snap_stroke_center_x(x, stroke_width_phys);

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

    pub(super) fn fill_marquee_rectangle(
        &self,
        frame: &mut canvas::Frame,
        bounds: Rectangle,
        palette: &Extended,
    ) {
        frame.fill_rectangle(
            Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
            palette.primary.base.color.scale_alpha(0.08),
        );
        frame.stroke_rectangle(
            Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
            canvas::Stroke::default()
                .with_color(palette.primary.base.color.scale_alpha(0.6))
                .with_width(1.0),
        );
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
            DrawingObject::Box {
                start,
                end,
                extend_right,
            } => {
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

                if *extend_right {
                    return vec![
                        (DrawingHandleKind::BoxTopLeft, Point::new(left, top)),
                        (DrawingHandleKind::BoxBottomLeft, Point::new(left, bottom)),
                    ];
                }

                vec![
                    (DrawingHandleKind::BoxTopLeft, Point::new(left, top)),
                    (DrawingHandleKind::BoxTopRight, Point::new(right, top)),
                    (DrawingHandleKind::BoxBottomRight, Point::new(right, bottom)),
                    (DrawingHandleKind::BoxBottomLeft, Point::new(left, bottom)),
                ]
            }
            DrawingObject::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray: true,
            } => self
                .anchor_to_overlay_point(
                    scene,
                    DrawingAnchor {
                        panel_id: *panel_id,
                        x: *x,
                        y_unit: *y_unit,
                    },
                )
                .into_iter()
                .map(|point| (DrawingHandleKind::HorizontalLineStart, point))
                .collect(),
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
        pixels: PixelGeometry,
        object: &DrawingObject,
    ) {
        let handles = self.drawing_handle_points(scene, object);
        if handles.is_empty() {
            return;
        }

        let Some(clip_bounds) = self.drawing_handle_clip_bounds(scene, object) else {
            return;
        };

        let fill_color = palette.background.strongest.color;
        let stroke_color = palette.primary.base.color.scale_alpha(0.92);
        let (handle_stroke_width, _) = pixels.quantized_stroke_width(1.2);

        frame.with_clip(clip_bounds, |frame| {
            for (_, point) in handles {
                let point = Point::new(pixels.snap_x_to_cell(point.x), point.y);
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
        let hit_radius_sq = DRAWING_HANDLE_HIT_RADIUS_PX * DRAWING_HANDLE_HIT_RADIUS_PX;
        let mut best: Option<(f32, DrawingId, DrawingHandleKind)> = None;

        for drawing in self.drawings.selected_visible_drawings() {
            if drawing.locked {
                continue;
            }

            for (kind, handle_point) in self.drawing_handle_points(scene, &drawing.object) {
                let dx = point.x - handle_point.x;
                let dy = point.y - handle_point.y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq > hit_radius_sq {
                    continue;
                }

                if best
                    .map(|(best_dist_sq, _, _)| dist_sq < best_dist_sq)
                    .unwrap_or(true)
                {
                    best = Some((dist_sq, drawing.id, kind));
                }
            }
        }

        best.map(|(_, id, kind)| (id, kind))
    }

    pub(super) fn fill_drawing_objects(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let drawing_state = &self.drawings;

        for drawing in drawing_state
            .entities
            .iter()
            .filter(|drawing| drawing.visible)
        {
            let selected = drawing_state.selected_drawings.contains(&drawing.id);
            self.draw_drawing_object(
                frame,
                scene,
                &drawing.object,
                drawing.style,
                palette,
                generated_accents,
                selected,
                pixels,
            );
        }
    }

    pub(super) fn fill_drawing_interaction(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let drawing_state = &self.drawings;

        for selected_drawing in drawing_state.selected_visible_drawings() {
            self.draw_selected_drawing_handles(
                frame,
                scene,
                palette,
                pixels,
                &selected_drawing.object,
            );
        }

        if let Some(draft) = drawing_state.drawing_draft {
            let mut style = draft.style();
            style.stroke_color = style.stroke_color.scale_alpha(0.9);
            if let Some(fill) = style.fill_color {
                style.fill_color = Some(fill.scale_alpha(0.5));
            }

            let preview = draft.preview_object();
            self.draw_drawing_object(
                frame,
                scene,
                &preview,
                style,
                palette,
                generated_accents,
                false,
                pixels,
            );
            self.draw_selected_drawing_handles(frame, scene, palette, pixels, &preview);
        }

        if let Some(measure) = drawing_state.measure {
            self.fill_measure(frame, scene, palette, measure, pixels);
        }
    }

    fn fill_measure(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        measure: DrawingMeasure,
        pixels: PixelGeometry,
    ) {
        let state = measure.state();
        let Some(start_point) = self.anchor_to_overlay_point(scene, state.start) else {
            return;
        };
        let Some(end_point) = self.anchor_to_overlay_point(scene, state.end) else {
            return;
        };
        let Some(panel_index) = self.panel_index_for_id(state.panel_id) else {
            return;
        };
        let Some(clip_bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index) else {
            return;
        };
        let panel_precision = self.panel_value_precision(panel_index);
        let value_scale = self.panel_value_scale(panel_index);

        let start_value = value_scale.unit_to_value(state.start.y_unit);
        let end_value = value_scale.unit_to_value(state.end.y_unit);
        let delta = end_value - start_value;
        let pct = if start_value.abs() > 1e-12 {
            (delta / start_value.abs()) * 100.0
        } else {
            0.0
        };

        let color = if delta > 1e-9 {
            palette.success.base.color
        } else if delta < -1e-9 {
            palette.danger.base.color
        } else {
            palette.background.base.text
        };

        let span_ms = self
            .drawing_timestamp(state.start.x)
            .zip(self.drawing_timestamp(state.end.x))
            .map(|(start, end)| start.as_u64().abs_diff(end.as_u64()));
        let bars = self
            .x_unit_for_anchor(scene, state.start.x)
            .zip(self.x_unit_for_anchor(scene, state.end.x))
            .map_or(0.0, |(start, end)| (start - end).abs());

        let sign = if delta > 1e-9 {
            "+"
        } else if delta < -1e-9 {
            "-"
        } else {
            ""
        };
        let delta_label =
            self.format_panel_axis_value(panel_index, panel_precision, delta.abs(), 0.01);
        let text1 = format!("{sign}{delta_label} ({sign}{:.2}%)", pct.abs());
        let text2 = match span_ms {
            Some(span_ms) => format!("{} ({bars}b)", data::util::format_duration_ms(span_ms)),
            None => format!("{bars}b"),
        };

        let (_, stroke_width_phys) = pixels.quantized_stroke_width(1.25);
        let start = pixels.snap_point_for_stroke(start_point, stroke_width_phys);
        let end = pixels.snap_point_for_stroke(end_point, stroke_width_phys);
        let stroke = canvas::Stroke::default().with_color(color).with_width(1.25);

        frame.with_clip(clip_bounds, |frame| {
            let left = start.x.min(end.x);
            let right = start.x.max(end.x);
            let top = start.y.min(end.y);
            let bottom = start.y.max(end.y);

            frame.fill_rectangle(
                Point::new(left, top),
                Size::new((right - left).max(1.0), (bottom - top).max(1.0)),
                color.scale_alpha(0.08),
            );

            frame.stroke(&canvas::Path::line(start, end), stroke);

            for point in [start, end] {
                let dot = canvas::Path::circle(point, 2.5);
                frame.fill(&dot, color);
            }

            self.fill_measure_bubble(
                frame,
                palette,
                clip_bounds,
                color,
                &text1,
                &text2,
                start,
                end,
            );
        });
    }

    fn fill_measure_bubble(
        &self,
        frame: &mut canvas::Frame,
        palette: &Extended,
        bounds: Rectangle,
        color: iced::Color,
        text1: &str,
        text2: &str,
        start: Point,
        end: Point,
    ) {
        let mid_x = (start.x + end.x) / 2.0;
        let mid_y = (start.y + end.y) / 2.0;

        let text_width = |text: &str| text.chars().count() as f32 * TEXT_SIZE * 0.6 + 16.0;
        let bubble_width = text_width(text1).max(text_width(text2));
        let bubble_height = TEXT_SIZE * 2.0 + 8.0;

        let bubble_x = (mid_x - bubble_width / 2.0).clamp(
            bounds.x + 4.0,
            (bounds.x + bounds.width - bubble_width - 4.0).max(bounds.x + 4.0),
        );
        let mut bubble_y = mid_y - bubble_height - 8.0;
        if bubble_y < bounds.y + 4.0 {
            bubble_y = mid_y + 8.0;
        }

        frame.fill_rectangle(
            Point::new(bubble_x, bubble_y),
            Size::new(bubble_width, bubble_height),
            palette.background.strong.color,
        );
        frame.stroke_rectangle(
            Point::new(bubble_x, bubble_y),
            Size::new(bubble_width, bubble_height),
            canvas::Stroke::default().with_color(color).with_width(1.0),
        );

        for (index, line) in [text1, text2].into_iter().enumerate() {
            frame.fill_text(canvas::Text {
                content: line.to_string(),
                position: Point::new(
                    bubble_x + bubble_width / 2.0,
                    bubble_y + TEXT_SIZE / 2.0 + TEXT_SIZE * index as f32 + 1.0,
                ),
                color: palette.background.strong.text,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
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
            DrawingObject::Box {
                start,
                end,
                extend_right,
            } => {
                let Some((origin, size, bounds, visible_bounds)) =
                    self.drawing_box_geometry(scene, *start, *end, *extend_right)
                else {
                    return false;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let within_expanded = contains_inclusive(visible_bounds.expand(tolerance), point);

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

                let inside_visible_fill =
                    style.fill_color.is_some() && contains_inclusive(visible_bounds, point);

                near_edge || inside_visible_fill
            }
            DrawingObject::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray,
            } => {
                let Some((bounds, y, start_x)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit, *x, *ray)
                else {
                    return false;
                };

                point.x >= (start_x - tolerance)
                    && point.x <= (bounds.x + bounds.width + tolerance)
                    && (point.y - y).abs() <= tolerance
            }
            DrawingObject::VerticalLine { x } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *x) else {
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

    pub(super) fn clipped_marquee_rect(
        &self,
        scene: &Scene,
        panel_index: usize,
        start: Point,
        current: Point,
    ) -> Option<Rectangle> {
        let rect = Rectangle {
            x: start.x.min(current.x),
            y: start.y.min(current.y),
            width: (current.x - start.x).abs(),
            height: (current.y - start.y).abs(),
        };
        let panel_bounds = self.panel_plot_bounds_in_overlay(scene, panel_index)?;
        intersect_bounds(rect, panel_bounds)
    }

    pub(super) fn marquee_selected_drawings(
        &self,
        scene: &Scene,
        panel_index: usize,
        start: Point,
        current: Point,
    ) -> Vec<DrawingId> {
        let Some(rect) = self.clipped_marquee_rect(scene, panel_index, start, current) else {
            return Vec::new();
        };

        self.drawings
            .entities
            .iter()
            .filter(|drawing| drawing.visible)
            .filter(|drawing| {
                self.drawing_selection_corners(scene, &drawing.object)
                    .iter()
                    .any(|corner| contains_inclusive(rect, *corner))
            })
            .map(|drawing| drawing.id)
            .collect()
    }

    fn drawing_selection_corners(&self, scene: &Scene, object: &DrawingObject) -> Vec<Point> {
        match object {
            DrawingObject::Trendline { start, end } => {
                let mut corners = Vec::with_capacity(2);
                if let Some(point) = self.anchor_to_overlay_point(scene, *start) {
                    corners.push(point);
                }
                if let Some(point) = self.anchor_to_overlay_point(scene, *end) {
                    corners.push(point);
                }
                corners
            }
            DrawingObject::Box {
                start,
                end,
                extend_right,
            } => {
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
                if *extend_right {
                    vec![Point::new(left, top), Point::new(left, bottom)]
                } else {
                    vec![
                        Point::new(left, top),
                        Point::new(right, top),
                        Point::new(right, bottom),
                        Point::new(left, bottom),
                    ]
                }
            }
            DrawingObject::HorizontalLine {
                panel_id,
                y_unit,
                x,
                ray,
            } => {
                if !*ray {
                    return Vec::new();
                }
                self.anchor_to_overlay_point(
                    scene,
                    DrawingAnchor {
                        panel_id: *panel_id,
                        x: *x,
                        y_unit: *y_unit,
                    },
                )
                .into_iter()
                .collect()
            }
            DrawingObject::VerticalLine { .. } => Vec::new(),
        }
    }

    fn format_y_label(&self, scene: &Scene, panel_id: PanelId, y_unit: YUnit) -> Option<String> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value_scale = self.panel_value_scale(panel_index);

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                Some(scene.format_primary_cursor_label(value_scale.unit_to_value_f32(y_unit)))
            }
            KlinePanelKind::Indicator => Some(self.format_panel_axis_value(
                panel_index,
                panel_precision,
                value_scale.unit_to_value(y_unit),
                0.01,
            )),
        }
    }

    pub(super) fn draw_x_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        x: f32,
        text: &str,
        filter: &mut AxisBadgeFilter,
    ) {
        let x_label_w = (text.chars().count() as f32 * TEXT_SIZE * 0.67 + 12.0).clamp(100.0, 240.0);
        let x_label_h = TEXT_SIZE + 6.0;
        let x_label_x = (x - x_label_w / 2.0).clamp(
            scene.layout.regions.plot.x,
            scene.layout.regions.plot.x + scene.layout.regions.plot.width - x_label_w,
        );
        let x_label_y = scene.layout.regions.x_axis.y + 2.0;
        let label_bounds = Rectangle {
            x: x_label_x,
            y: x_label_y,
            width: x_label_w,
            height: x_label_h,
        };

        if !filter.reserve(label_bounds) {
            return;
        }

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
        filter: &mut AxisBadgeFilter,
    ) {
        self.draw_y_axis_badge_with_colors(
            frame,
            scene,
            y,
            text,
            clamp_bounds,
            palette.background.strong.color,
            palette.background.strong.text,
            filter,
        );
    }

    pub(super) fn draw_y_axis_badge_with_colors(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        y: f32,
        text: &str,
        clamp_bounds: Rectangle,
        background_color: iced::Color,
        text_color: iced::Color,
        filter: &mut AxisBadgeFilter,
    ) {
        let y_label_h = TEXT_SIZE + 6.0;
        let y_axis = scene.layout.regions.y_axis;
        let y_min = clamp_bounds.y;
        let y_max = (clamp_bounds.y + clamp_bounds.height - y_label_h).max(y_min);
        let y_label_y = (y - (y_label_h / 2.0)).clamp(y_min, y_max);
        let label_bounds = Rectangle {
            x: y_axis.x,
            y: y_label_y,
            width: y_axis.width,
            height: y_label_h,
        };

        if !filter.reserve(label_bounds) {
            return;
        }

        frame.fill_rectangle(
            Point::new(y_axis.x, y_label_y),
            Size::new(y_axis.width, y_label_h),
            background_color,
        );

        frame.fill_text(canvas::Text {
            content: text.to_string(),
            position: Point::new(
                y_axis.x + Y_AXIS_LABEL_LEFT_INSET,
                y_label_y + y_label_h / 2.0,
            ),
            color: text_color,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    pub(super) fn draw_y_axis_badge_with_timer(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        y: f32,
        value_text: &str,
        timer_text: Option<&str>,
        clamp_bounds: Rectangle,
        background_color: iced::Color,
        value_text_color: iced::Color,
        timer_text_color: iced::Color,
        filter: &mut AxisBadgeFilter,
    ) {
        let y_label_h = if timer_text.is_some() {
            TEXT_SIZE * 2.0 + 4.0
        } else {
            TEXT_SIZE + 6.0
        };
        let y_axis = scene.layout.regions.y_axis;
        let y_min = clamp_bounds.y;
        let y_max = (clamp_bounds.y + clamp_bounds.height - y_label_h).max(y_min);
        let y_label_y = (y - (y_label_h / 2.0)).clamp(y_min, y_max);
        let label_bounds = Rectangle {
            x: y_axis.x,
            y: y_label_y,
            width: y_axis.width,
            height: y_label_h,
        };

        if !filter.reserve(label_bounds) {
            return;
        }

        frame.fill_rectangle(
            Point::new(y_axis.x, y_label_y),
            Size::new(y_axis.width, y_label_h),
            background_color,
        );

        let value_y = if timer_text.is_some() {
            y_label_y + 2.0
        } else {
            y_label_y + y_label_h / 2.0
        };
        let text_x = y_axis.x + Y_AXIS_LABEL_LEFT_INSET;
        frame.fill_text(canvas::Text {
            content: value_text.to_string(),
            position: Point::new(text_x, value_y),
            color: value_text_color,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Start.into(),
            align_y: if timer_text.is_some() {
                iced::Alignment::Start.into()
            } else {
                iced::Alignment::Center.into()
            },
            font: style::AZERET_MONO,
            ..Default::default()
        });

        if let Some(timer_text) = timer_text {
            frame.fill_text(canvas::Text {
                content: timer_text.to_string(),
                position: Point::new(text_x, y_label_y + 15.0),
                color: timer_text_color,
                size: (TEXT_SIZE - 1.0).into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Start.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    pub(super) fn fill_active_drawing_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        filter: &mut AxisBadgeFilter,
        pixels: PixelGeometry,
    ) {
        let Some(object) = self.drawings.active_axis_labeled_object() else {
            return;
        };

        for anchor in object.axis_label_anchors() {
            match anchor {
                AxisLabelAnchor::X { x } => {
                    let Some(screen_x) = self.x_overlay_for_anchor(scene, x) else {
                        continue;
                    };
                    let Some(x_unit) = self.x_unit_for_anchor(scene, x) else {
                        continue;
                    };
                    let x_text = match (self.basis, x) {
                        (data::chart::Basis::Tick(interval), DrawingX::Tick(_)) => {
                            format!("{}b", x.tick_bar_position(interval.0 as u64).unwrap())
                        }
                        _ => self.format_x_label(scene.x.axis, x_unit as i64, 1),
                    };
                    self.draw_x_axis_badge(
                        frame,
                        scene,
                        palette,
                        pixels.snap_x_to_cell(screen_x),
                        &x_text,
                        filter,
                    );
                }
                AxisLabelAnchor::Y { panel_id, y_unit } => {
                    let Some(y) = self.y_overlay_for_value(scene, panel_id, y_unit) else {
                        continue;
                    };
                    let Some(y_text) = self.format_y_label(scene, panel_id, y_unit) else {
                        continue;
                    };
                    let Some(panel_index) = self.panel_index_for_id(panel_id) else {
                        continue;
                    };
                    let Some(panel_bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index)
                    else {
                        continue;
                    };

                    self.draw_y_axis_badge(frame, scene, palette, y, &y_text, panel_bounds, filter);
                }
            }
        }
    }
}
