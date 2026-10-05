pub(super) mod marks;
pub(super) mod plot;
use super::layout::PanelLayoutTree;
use super::{KlineSeries, MIN_FULL_CACHE_REFRESH_INTERVAL};
use crate::window::PixelRatio;
use data::chart::kline::SeriesColor;
use data::config::theme::ACCENT_COUNT;
use enum_map::{Enum, EnumMap};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(super) enum CanvasLayer {
    Background,
    StudyUnderlay,
    Foreground,
    Drawings,
    PriceBars,
    YAxis,
    XAxis,
    Overlay,
    InteractionText,
    Marquee,
}

impl CanvasLayer {
    pub(super) fn is_interaction(self) -> bool {
        matches!(self, Self::Overlay | Self::InteractionText | Self::Marquee)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RenderRequest {
    #[default]
    None,
    Interaction,
    All,
}

#[derive(Debug, PartialEq)]
pub(super) struct PresentationEnvironment {
    pub(super) bounds: Rectangle,
    pub(super) layout: PanelLayoutTree,
    pixel_ratio: PixelRatio,
    theme_generation: u64,
}

#[derive(Default)]
pub(super) struct RenderScheduler {
    pending: RenderRequest,
    admitted_revision: Option<u64>,
    last_data_admission: Option<Instant>,
    environment: Option<PresentationEnvironment>,
}

impl RenderScheduler {
    pub(super) fn request(&mut self, request: RenderRequest) {
        self.pending = self.pending.max(request);
    }

    pub(super) fn presentation(&self) -> Option<&PresentationEnvironment> {
        self.environment.as_ref()
    }

    pub(super) fn data_deadline(&self, revision: u64, now: Instant) -> Option<Instant> {
        (self.admitted_revision != Some(revision)).then(|| {
            self.last_data_admission
                .map_or(now, |last| last + MIN_FULL_CACHE_REFRESH_INTERVAL)
        })
    }

    pub(super) fn admit(
        &mut self,
        revision: u64,
        now: Instant,
        bounds: Rectangle,
        layout: &PanelLayoutTree,
        pixel_ratio: PixelRatio,
        theme_generation: u64,
    ) -> RenderRequest {
        let data_due = self
            .data_deadline(revision, now)
            .is_some_and(|deadline| now >= deadline);
        let environment_changed = self.environment.as_ref().is_none_or(|environment| {
            environment.bounds != bounds
                || environment.layout != *layout
                || environment.pixel_ratio != pixel_ratio
                || environment.theme_generation != theme_generation
        });
        let request = std::mem::take(&mut self.pending).max(if data_due || environment_changed {
            RenderRequest::All
        } else {
            RenderRequest::None
        });
        if request == RenderRequest::All {
            if self.admitted_revision != Some(revision) {
                self.last_data_admission = Some(now);
            }
            self.admitted_revision = Some(revision);
            if environment_changed {
                self.environment = Some(PresentationEnvironment {
                    bounds,
                    layout: layout.clone(),
                    pixel_ratio,
                    theme_generation,
                });
            }
        }
        request
    }
}

#[derive(Debug, Clone)]
struct OwnedPath(canvas::Path);

impl PartialEq for OwnedPath {
    fn eq(&self, other: &Self) -> bool {
        self.0.raw().iter().eq(other.0.raw().iter())
    }
}

#[derive(Debug, Clone)]
struct OwnedFill(canvas::Fill);

impl PartialEq for OwnedFill {
    fn eq(&self, other: &Self) -> bool {
        self.0.style == other.0.style
            && std::mem::discriminant(&self.0.rule) == std::mem::discriminant(&other.0.rule)
    }
}

#[derive(Debug, Clone)]
struct OwnedStroke {
    style: canvas::Style,
    width: f32,
    line_cap: canvas::LineCap,
    line_join: canvas::LineJoin,
    segments: Vec<f32>,
    offset: usize,
}

impl PartialEq for OwnedStroke {
    fn eq(&self, other: &Self) -> bool {
        self.style == other.style
            && self.width == other.width
            && std::mem::discriminant(&self.line_cap) == std::mem::discriminant(&other.line_cap)
            && std::mem::discriminant(&self.line_join) == std::mem::discriminant(&other.line_join)
            && self.segments == other.segments
            && self.offset == other.offset
    }
}

impl OwnedStroke {
    fn borrowed(&self) -> canvas::Stroke<'_> {
        canvas::Stroke {
            style: self.style,
            width: self.width,
            line_cap: self.line_cap,
            line_join: self.line_join,
            line_dash: canvas::LineDash {
                segments: &self.segments,
                offset: self.offset,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedText(canvas::Text);

impl PartialEq for PreparedText {
    fn eq(&self, other: &Self) -> bool {
        let canvas::Text {
            content,
            position,
            max_width,
            color,
            size,
            line_height,
            font,
            align_x,
            align_y,
            shaping,
        } = &self.0;
        let b = &other.0;
        *content == b.content
            && *position == b.position
            && *max_width == b.max_width
            && *color == b.color
            && *size == b.size
            && *line_height == b.line_height
            && *font == b.font
            && *align_x == b.align_x
            && *align_y == b.align_y
            && *shaping == b.shaping
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PaintOperation {
    Text(PreparedText),
    Rectangle {
        position: Point,
        size: Size,
        fill: OwnedFill,
    },
    Fill {
        path: OwnedPath,
        fill: OwnedFill,
    },
    Stroke {
        path: OwnedPath,
        stroke: OwnedStroke,
    },
    Clip {
        bounds: Rectangle,
        inputs: PaintInputs,
    },
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct PaintInputs {
    operations: Vec<PaintOperation>,
}

impl PaintInputs {
    pub(super) fn fill_text(&mut self, text: impl Into<canvas::Text>) {
        self.operations
            .push(PaintOperation::Text(PreparedText(text.into())));
    }

    pub(super) fn append(&mut self, other: &Self) {
        self.operations.extend(other.operations.iter().cloned());
    }

    pub(super) fn fill_rectangle(
        &mut self,
        position: Point,
        size: Size,
        fill: impl Into<canvas::Fill>,
    ) {
        self.operations.push(PaintOperation::Rectangle {
            position,
            size,
            fill: OwnedFill(fill.into()),
        });
    }

    pub(super) fn fill(&mut self, path: &canvas::Path, fill: impl Into<canvas::Fill>) {
        self.operations.push(PaintOperation::Fill {
            path: OwnedPath(path.clone()),
            fill: OwnedFill(fill.into()),
        });
    }

    pub(super) fn stroke<'a>(
        &mut self,
        path: &canvas::Path,
        stroke: impl Into<canvas::Stroke<'a>>,
    ) {
        let stroke = stroke.into();
        self.operations.push(PaintOperation::Stroke {
            path: OwnedPath(path.clone()),
            stroke: OwnedStroke {
                style: stroke.style,
                width: stroke.width,
                line_cap: stroke.line_cap,
                line_join: stroke.line_join,
                segments: stroke.line_dash.segments.to_vec(),
                offset: stroke.line_dash.offset,
            },
        });
    }

    pub(super) fn stroke_rectangle<'a>(
        &mut self,
        position: Point,
        size: Size,
        stroke: impl Into<canvas::Stroke<'a>>,
    ) {
        self.stroke(&canvas::Path::rectangle(position, size), stroke);
    }

    pub(super) fn with_clip(&mut self, bounds: Rectangle, paint: impl FnOnce(&mut Self)) {
        let mut inputs = Self::default();
        paint(&mut inputs);
        self.operations
            .push(PaintOperation::Clip { bounds, inputs });
    }

    pub(super) fn replay(&self, frame: &mut canvas::Frame) {
        for operation in &self.operations {
            match operation {
                PaintOperation::Text(text) => frame.fill_text(text.0.clone()),
                PaintOperation::Rectangle {
                    position,
                    size,
                    fill,
                } => frame.fill_rectangle(*position, *size, fill.0),
                PaintOperation::Fill { path, fill } => frame.fill(&path.0, fill.0),
                PaintOperation::Stroke { path, stroke } => frame.stroke(&path.0, stroke.borrowed()),
                PaintOperation::Clip { bounds, inputs } => {
                    frame.with_clip(*bounds, |frame| inputs.replay(frame))
                }
            }
        }
    }
}

/// Places cached, layer-local geometry within the chart widget during `Widget::draw`.
///
/// All variants first apply `ResolvedLayer::root_origin`, the widget's screen position.
/// Rectangle positions are offsets within the widget; sizes determine the canvas frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LayerPlacement {
    /// Region-local geometry shifted to the rectangle's origin, without an additional
    /// renderer clip. Local `(0, 0)` becomes the axis region's top-left.
    Axis(Rectangle),
    /// Region-local geometry shifted to the plot origin and clipped to the plot rectangle.
    Plot(Rectangle),
    /// Widget-local geometry clipped to the widget rectangle at `(0, 0)`.
    /// Used for drawings, interaction overlays, and text.
    Widget(Size),
}

impl LayerPlacement {
    pub(super) fn size(self) -> Size {
        match self {
            Self::Widget(size) => size,
            Self::Axis(bounds) | Self::Plot(bounds) => bounds.size(),
        }
    }

    pub(super) fn origin(self) -> Point {
        match self {
            Self::Widget(_) => Point::ORIGIN,
            Self::Axis(bounds) | Self::Plot(bounds) => bounds.position(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedLayer {
    pub(super) root_origin: Point,
    pub(super) placement: LayerPlacement,
    // DPI participates in geometry validity even when resolved paint operations are unchanged.
    pub(super) pixel_ratio: PixelRatio,
    pub(super) inputs: PaintInputs,
}

impl ResolvedLayer {
    pub(super) fn with_inputs(&self, inputs: PaintInputs) -> Self {
        Self {
            root_origin: self.root_origin,
            placement: self.placement,
            pixel_ratio: self.pixel_ratio,
            inputs,
        }
    }
}

#[derive(Default)]
pub(super) struct KeyedCache {
    pub(super) geometry: canvas::Cache,
    layer: Option<ResolvedLayer>,
}

impl KeyedCache {
    pub(super) fn prepare(&mut self, layer: ResolvedLayer) {
        self.geometry.clear();
        self.layer = Some(layer);
    }

    pub(super) fn resolved(&self) -> Option<&ResolvedLayer> {
        self.layer.as_ref()
    }
}

pub(super) type CanvasLayers = EnumMap<CanvasLayer, KeyedCache>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PixelSpan {
    pub(super) start: f32,
    pub(super) size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PixelGeometry {
    ratio: f32,
    origin: Point,
}

impl PixelGeometry {
    pub(super) fn new(pixel_ratio: PixelRatio, origin: Point) -> Self {
        Self::from_ratio(pixel_ratio.value(), origin)
    }

    fn from_ratio(ratio: f32, origin: Point) -> Self {
        Self {
            ratio: if ratio.is_finite() && ratio > 0.0 {
                ratio
            } else {
                1.0
            },
            origin: Point::new(
                if origin.x.is_finite() { origin.x } else { 0.0 },
                if origin.y.is_finite() { origin.y } else { 0.0 },
            ),
        }
    }

    pub(super) fn ratio(self) -> f32 {
        self.ratio
    }

    pub(super) fn origin(self) -> Point {
        self.origin
    }

    pub(super) fn logical_width_from_physical(self, width_phys: i32) -> f32 {
        width_phys.max(1) as f32 / self.ratio
    }

    pub(super) fn quantized_stroke_width(self, width_logical: f32) -> (f32, i32) {
        let stroke_width_phys = (width_logical.max(0.0) * self.ratio).round() as i32;
        let stroke_width_phys = stroke_width_phys.max(1);
        (
            self.logical_width_from_physical(stroke_width_phys),
            stroke_width_phys,
        )
    }

    fn snap_axis_to_physical(self, value_local: f32, origin: f32) -> i32 {
        ((value_local + origin) * self.ratio).round() as i32
    }

    fn physical_to_local(self, physical: i32, origin: f32) -> f32 {
        physical as f32 / self.ratio - origin
    }

    fn snap_stroke_center(self, value_local: f32, origin: f32, stroke_width_phys: i32) -> f32 {
        let axis_phys = (value_local + origin) * self.ratio;
        let snapped_phys = if (stroke_width_phys & 1) == 0 {
            axis_phys.round()
        } else {
            (axis_phys - 0.5).round() + 0.5
        };

        (snapped_phys / self.ratio) - origin
    }

    fn snapped_span(self, start_local: f32, end_local: f32, origin: f32) -> PixelSpan {
        let start = start_local.min(end_local);
        let end = start_local.max(end_local);
        let start_phys = self.snap_axis_to_physical(start, origin);
        let end_phys = self.snap_axis_to_physical(end, origin).max(start_phys + 1);

        PixelSpan {
            start: self.physical_to_local(start_phys, origin),
            size: (end_phys - start_phys) as f32 / self.ratio,
        }
    }

    fn positive_span(self, start_local: f32, end_local: f32, origin: f32) -> Option<PixelSpan> {
        if !start_local.is_finite() || !end_local.is_finite() {
            return None;
        }

        let start = start_local.min(end_local);
        let end = start_local.max(end_local);
        (end > start).then(|| self.snapped_span(start, end, origin))
    }

    pub(super) fn snap_x_to_cell(self, x: f32) -> f32 {
        self.physical_to_local(self.snap_axis_to_physical(x, self.origin.x), self.origin.x)
    }

    fn centered_left(self, x: f32, width_phys: i32) -> f32 {
        let center_phys = self.snap_axis_to_physical(x, self.origin.x);
        self.physical_to_local(center_phys - width_phys / 2, self.origin.x)
    }

    pub(super) fn snapped_x_span(self, start: f32, end: f32) -> PixelSpan {
        self.snapped_span(start, end, self.origin.x)
    }

    pub(super) fn snapped_y_span(self, start: f32, end: f32) -> PixelSpan {
        self.snapped_span(start, end, self.origin.y)
    }

    pub(super) fn positive_x_span(self, start: f32, end: f32) -> Option<PixelSpan> {
        self.positive_span(start, end, self.origin.x)
    }

    fn positive_y_span(self, start: f32, end: f32) -> Option<PixelSpan> {
        self.positive_span(start, end, self.origin.y)
    }

    pub(super) fn positive_x_span_in_bounds(
        self,
        start: f32,
        end: f32,
        lower: f32,
        upper: f32,
    ) -> Option<PixelSpan> {
        if !start.is_finite()
            || !end.is_finite()
            || !lower.is_finite()
            || !upper.is_finite()
            || lower > upper
        {
            return None;
        }
        let raw_start = start.min(end);
        let raw_end = start.max(end);
        let clipped_start = raw_start.max(lower).min(upper);
        let clipped_end = raw_end.min(upper).max(lower);
        self.positive_x_span(clipped_start, clipped_end)
    }

    fn positive_y_span_in_bounds(
        self,
        start: f32,
        end: f32,
        lower: f32,
        upper: f32,
    ) -> Option<PixelSpan> {
        if !start.is_finite()
            || !end.is_finite()
            || !lower.is_finite()
            || !upper.is_finite()
            || lower > upper
        {
            return None;
        }
        let raw_start = start.min(end);
        let raw_end = start.max(end);
        let clipped_start = raw_start.max(lower).min(upper);
        let clipped_end = raw_end.min(upper).max(lower);
        self.positive_y_span(clipped_start, clipped_end)
    }

    pub(super) fn snap_x_for_stroke(self, x: f32, stroke_width_phys: i32) -> f32 {
        let x_cell = self.snap_x_to_cell(x);
        self.snap_stroke_center(x_cell, self.origin.x, stroke_width_phys)
    }

    pub(super) fn snap_stroke_center_x(self, x: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(x, self.origin.x, stroke_width_phys)
    }

    pub(super) fn snap_stroke_center_y(self, y: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(y, self.origin.y, stroke_width_phys)
    }

    pub(super) fn snap_y_for_stroke(self, y: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(y, self.origin.y, stroke_width_phys)
    }

    pub(super) fn snap_point_for_stroke(self, point: Point, stroke_width_phys: i32) -> Point {
        Point::new(
            self.snap_stroke_center(point.x, self.origin.x, stroke_width_phys),
            self.snap_stroke_center(point.y, self.origin.y, stroke_width_phys),
        )
    }
}

pub(super) fn resolve_indicator_color(color: indicator_engine::Color) -> iced::Color {
    indicator_color_from_argb(color.0)
}

fn indicator_color_from_argb(value: u32) -> iced::Color {
    iced::Color::from_rgba8(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
        f32::from(((value >> 24) & 0xff) as u8) / 255.0,
    )
}

pub(super) fn series_line_color(
    series: &KlineSeries,
    palette: &Extended,
    generated_accents: &[iced::Color; ACCENT_COUNT],
) -> iced::Color {
    match series.color() {
        SeriesColor::Theme(color) => {
            crate::widget::resolve_theme_color(color, palette, generated_accents)
        }
        SeriesColor::Custom(color) => color,
    }
}
