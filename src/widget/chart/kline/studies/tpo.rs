use super::super::chrome::PreparedChrome;
use super::super::interaction::{LayerContext, LayerEffects, LayerGesture, NavigationFallback};
use super::super::painting::PaintInputs;
use super::super::painting::RenderRequest;
use super::super::painting::{PixelGeometry, PixelSpan};
use super::super::scene::Scene;
use super::super::{CHAR_W, TEXT_SIZE};
use super::{LayerHit, LayerPaintPhase, LayerPointerInput, StudyObjectKey};
use crate::style;
use data::chart::Basis;
use data::chart::kline::{TpoBoundaryEdits, TpoColorConfig, TpoProfile, TpoProfileStatus};
use data::config::theme::ACCENT_COUNT;
use exchange::{UnixMs, unit::Price};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};

const TPO_BOUNDARY_HIT_PX: f32 = 7.0;
const TPO_ACTION_BUTTON_WIDTH_PX: f32 = 62.0;
const TPO_ACTION_BUTTON_HEIGHT_PX: f32 = 20.0;
const TPO_RESET_BUTTON_WIDTH_PX: f32 = 48.0;
const TPO_ACTION_BUTTON_GAP_PX: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpoInteraction {
    SplitAt { boundary: UnixMs },
    MergeLeft { boundary: UnixMs },
    MoveBoundary { from: UnixMs, to: UnixMs },
    Reset { start: UnixMs, end: UnixMs },
}

#[derive(Debug, Clone, Copy)]
struct TpoProfileGeometry {
    index: usize,
    start: UnixMs,
    end: UnixMs,
    overlay_bounds: Rectangle,
    control_row_y: f32,
    button_bounds: Option<Rectangle>,
    reset_button_bounds: Option<Rectangle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TpoHit {
    Click(TpoClick),
    Boundary {
        boundary: UnixMs,
        minimum: Option<UnixMs>,
        maximum: Option<UnixMs>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TpoClick {
    Reset {
        start: UnixMs,
        end: UnixMs,
    },
    Profile {
        start: UnixMs,
    },
    MergeLeft {
        boundary: UnixMs,
        left_start: UnixMs,
    },
}

pub(crate) struct TpoObject<'a> {
    key: StudyObjectKey,
    profiles: &'a [TpoProfile],
    edits: &'a TpoBoundaryEdits,
    colors: Option<TpoColorConfig>,
}

#[derive(Default)]
pub(crate) struct TpoState {
    selected_profile: Option<UnixMs>,
    pending_click: Option<TpoClick>,
    drag: Option<TpoDrag>,
}

impl TpoState {
    fn clear_selection(&mut self) -> bool {
        self.selected_profile.take().is_some()
    }
}

impl<'a> TpoObject<'a> {
    pub(crate) fn new(
        key: data::chart::kline::KlineStudyId,
        profiles: &'a [TpoProfile],
        edits: &'a TpoBoundaryEdits,
        colors: Option<TpoColorConfig>,
    ) -> Self {
        Self {
            key: StudyObjectKey::Study(key),
            profiles,
            edits,
            colors,
        }
    }

    fn view<'b>(&'b self, context: LayerContext<'b>) -> TpoView<'b> {
        TpoView {
            object: self,
            context,
        }
    }
}

struct TpoView<'a> {
    object: &'a TpoObject<'a>,
    context: LayerContext<'a>,
}

impl TpoView<'_> {
    fn tpo_hit_test(&self, point: Point, selected_profile: Option<UnixMs>) -> Option<TpoHit> {
        let scene = self.context.scene;
        let geometries = self.tpo_profile_geometries(selected_profile);

        for geometry in &geometries {
            if geometry
                .reset_button_bounds
                .is_some_and(|bounds| super::super::layout::contains_inclusive(bounds, point))
            {
                return Some(TpoHit::Click(TpoClick::Reset {
                    start: geometry.start,
                    end: geometry.end,
                }));
            }
            if geometry
                .button_bounds
                .is_some_and(|bounds| super::super::layout::contains_inclusive(bounds, point))
            {
                let left_start = self
                    .object
                    .profiles
                    .get(geometry.index.saturating_sub(1))?
                    .start;
                return Some(TpoHit::Click(TpoClick::MergeLeft {
                    boundary: geometry.start,
                    left_start,
                }));
            }
        }

        if selected_profile.is_some() {
            let snap_timeframe = match self.context.coordinates.basis {
                Basis::Time(timeframe) => timeframe,
                Basis::Tick(_) => return None,
            };
            let selected_index = selected_profile.and_then(|start| {
                self.object
                    .profiles
                    .iter()
                    .position(|profile| profile.start == start)
            });
            let plot = scene.primary_plot();
            let plot_top = scene.layout.regions.plot.y + plot.y;
            let plot_bottom = plot_top + plot.height;
            for (index, profile) in self.object.profiles.iter().enumerate().skip(1) {
                let Some(boundary_unit) = scene.x.axis.unit_from_time_f32(profile.start) else {
                    continue;
                };
                let boundary_x = scene.layout.regions.plot.x + scene.map_x_plot_f32(boundary_unit);
                if (point.x - boundary_x).abs() <= TPO_BOUNDARY_HIT_PX
                    && point.y >= plot_top
                    && point.y <= plot_bottom
                {
                    let Some((minimum, maximum)) = selected_index.and_then(|selected_index| {
                        let selected = self.object.profiles.get(selected_index)?;
                        if index == selected_index {
                            Some((
                                None,
                                Some(selected.end.offset_by_timeframe(snap_timeframe, -1)),
                            ))
                        } else if index == selected_index + 1 {
                            Some((
                                Some(selected.start.offset_by_timeframe(snap_timeframe, 1)),
                                None,
                            ))
                        } else {
                            None
                        }
                    }) else {
                        continue;
                    };
                    return Some(TpoHit::Boundary {
                        boundary: profile.start,
                        minimum,
                        maximum,
                    });
                }
            }
        }

        geometries
            .iter()
            .rev()
            .find(|geometry| {
                super::super::layout::contains_inclusive(geometry.overlay_bounds, point)
            })
            .map(|geometry| {
                TpoHit::Click(TpoClick::Profile {
                    start: geometry.start,
                })
            })
    }

    fn tpo_boundary_from_cursor(
        &self,
        cursor: Point,
        boundary: UnixMs,
        minimum: Option<UnixMs>,
        maximum: Option<UnixMs>,
    ) -> Option<UnixMs> {
        let scene = self.context.scene;
        let pair_index = self
            .object
            .profiles
            .windows(2)
            .position(|profiles| profiles[1].start == boundary)?;
        let left_profile = self.object.profiles.get(pair_index)?;
        let snap_timeframe = match self.context.coordinates.basis {
            Basis::Time(timeframe) => timeframe,
            Basis::Tick(_) => return None,
        };
        let period_ms = snap_timeframe.to_milliseconds().max(1);
        let raw_time = scene.time_from_plot_x_f32(cursor.x - scene.layout.regions.plot.x)?;
        let (before_anchor, elapsed) = if raw_time >= left_profile.start {
            (false, raw_time.as_u64() - left_profile.start.as_u64())
        } else {
            (true, left_profile.start.as_u64() - raw_time.as_u64())
        };
        let whole_periods = elapsed / period_ms;
        let rounded_periods =
            whole_periods.saturating_add(u64::from(elapsed % period_ms >= period_ms / 2));
        let intervals = i64::try_from(rounded_periods).unwrap_or(i64::MAX);
        let candidate = left_profile.start.offset_by_timeframe(
            snap_timeframe,
            if before_anchor { -intervals } else { intervals },
        );
        let minimum = minimum.unwrap_or(self.object.profiles.first()?.start);
        let maximum = maximum.unwrap_or(self.object.profiles.last()?.end);
        (minimum <= maximum).then(|| candidate.clamp(minimum, maximum))
    }

    fn tpo_profile_geometries(&self, selected_profile: Option<UnixMs>) -> Vec<TpoProfileGeometry> {
        let scene = self.context.scene;
        if self.object.profiles.is_empty() || !self.context.coordinates.basis.is_time() {
            return Vec::new();
        }
        if self.object.colors.is_none() {
            return Vec::new();
        }

        let plot = *scene.primary_plot();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let plot_right = plot.x + plot.width;
        let plot_bottom = plot.y + plot.height;

        self.object
            .profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                let start_unit = scene.x.axis.unit_from_time_f32(profile.start)?;
                let end_unit = scene.x.axis.unit_from_time_f32(profile.end)?;
                let left = scene.map_x_plot_f32(start_unit);
                let right = scene.map_x_plot_f32(end_unit);
                let (line_start, line_end) = (left.min(right), left.max(right));
                if line_end < plot.x || line_start > plot_right {
                    return None;
                }

                let row_height = tpo_row_height(scene, profile, anchor);
                if row_height <= 0.0 || profile.rows.is_empty() {
                    return None;
                }

                let mut top = f32::INFINITY;
                let mut bottom = f32::NEG_INFINITY;
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if y.is_finite() {
                        top = top.min(y - row_height * 0.5);
                        bottom = bottom.max(y + row_height * 0.5);
                    }
                }
                if !top.is_finite() || !bottom.is_finite() {
                    return None;
                }

                let visible_left = line_start.max(plot.x);
                let visible_right = line_end.min(plot_right);
                let visible_top = top.max(plot.y);
                let visible_bottom = bottom.min(plot_bottom);
                if visible_right <= visible_left || visible_bottom <= visible_top {
                    return None;
                }

                let overlay_bounds = Rectangle {
                    x: scene.layout.regions.plot.x + visible_left,
                    y: scene.layout.regions.plot.y + visible_top,
                    width: visible_right - visible_left,
                    height: visible_bottom - visible_top,
                };
                let button_y =
                    (overlay_bounds.y - TPO_ACTION_BUTTON_GAP_PX - TPO_ACTION_BUTTON_HEIGHT_PX)
                        .max(scene.layout.regions.plot.y + plot.y);
                let selected = selected_profile == Some(profile.start);
                let reset_visible = selected
                    && self
                        .object
                        .edits
                        .has_boundary_in_range(profile.start, profile.end);
                let merge_with_reset_width = TPO_RESET_BUTTON_WIDTH_PX
                    + TPO_ACTION_BUTTON_WIDTH_PX
                    + TPO_ACTION_BUTTON_GAP_PX * 3.0;
                let button_bounds = (selected
                    && index > 0
                    && overlay_bounds.width
                        >= if reset_visible {
                            merge_with_reset_width
                        } else {
                            TPO_ACTION_BUTTON_WIDTH_PX + TPO_ACTION_BUTTON_GAP_PX * 2.0
                        })
                .then_some(Rectangle {
                    x: overlay_bounds.x + overlay_bounds.width
                        - TPO_ACTION_BUTTON_WIDTH_PX
                        - TPO_ACTION_BUTTON_GAP_PX,
                    y: button_y,
                    width: TPO_ACTION_BUTTON_WIDTH_PX,
                    height: TPO_ACTION_BUTTON_HEIGHT_PX,
                });
                let reset_button_bounds = (selected
                    && reset_visible
                    && overlay_bounds.width
                        >= TPO_RESET_BUTTON_WIDTH_PX + TPO_ACTION_BUTTON_GAP_PX * 2.0)
                    .then(|| {
                        let x = button_bounds.map_or(
                            overlay_bounds.x + overlay_bounds.width
                                - TPO_RESET_BUTTON_WIDTH_PX
                                - TPO_ACTION_BUTTON_GAP_PX,
                            |button| {
                                button.x - TPO_ACTION_BUTTON_GAP_PX - TPO_RESET_BUTTON_WIDTH_PX
                            },
                        );
                        Rectangle {
                            x,
                            y: button_y,
                            width: TPO_RESET_BUTTON_WIDTH_PX,
                            height: TPO_ACTION_BUTTON_HEIGHT_PX,
                        }
                    });

                Some(TpoProfileGeometry {
                    index,
                    start: profile.start,
                    end: profile.end,
                    overlay_bounds,
                    control_row_y: button_y,
                    button_bounds,
                    reset_button_bounds,
                })
            })
            .collect()
    }

    fn fill_tpo_profile_overlay(
        &self,
        frame: &mut PaintInputs,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let scene = self.context.scene;
        if self.object.profiles.is_empty() || !self.context.coordinates.basis.is_time() {
            return;
        }
        let Some(colors) = self.object.colors else {
            return;
        };
        let plot = *scene.primary_plot();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let line_width = pixels.quantized_stroke_width(1.0).0;

        frame.with_clip(plot, |frame| {
            for profile in self.object.profiles {
                let Some(start_unit) = scene.x.axis.unit_from_time_f32(profile.start) else {
                    continue;
                };
                let Some(end_unit) = scene.x.axis.unit_from_time_f32(profile.end) else {
                    continue;
                };
                let left = scene.map_x_plot_f32(start_unit);
                let right = scene.map_x_plot_f32(end_unit);
                let (line_start, line_end) = (left.min(right), left.max(right));
                if line_end < plot.x || line_start > plot.x + plot.width {
                    continue;
                }
                let profile_width = (line_end - line_start).max(0.0);
                let row_height = tpo_row_height(scene, profile, anchor);
                let column_width = profile.period_count.max(1) as f32;
                let column_width = profile_width / column_width;
                let show_letters = column_width >= 8.0 && row_height >= 8.0;
                let status_alpha = match profile.status {
                    TpoProfileStatus::Complete => 0.95,
                    TpoProfileStatus::Developing => 0.8,
                    TpoProfileStatus::Incomplete => 0.55,
                };
                let color = |configured| {
                    crate::widget::resolve_theme_color(configured, palette, generated_accents)
                        .scale_alpha(status_alpha)
                };
                let is_single_print = |price: Price| profile.single_prints.contains(&price);
                let poor_color = |price: Price| {
                    if colors.show_poor_high && profile.poor_highs.contains(&price) {
                        Some(colors.poor_high)
                    } else if colors.show_poor_low && profile.poor_lows.contains(&price) {
                        Some(colors.poor_low)
                    } else {
                        None
                    }
                };
                let row_color = |price: Price| {
                    if colors.show_poc && profile.poc == Some(price) {
                        colors.poc
                    } else if profile.val.is_some_and(|val| price >= val)
                        && profile.vah.is_some_and(|vah| price <= vah)
                    {
                        colors.value_area
                    } else {
                        colors.letters
                    }
                };
                let cell_color = |price: Price| {
                    if !show_letters {
                        if let Some(configured) = poor_color(price) {
                            configured
                        } else if colors.show_single_print && is_single_print(price) {
                            colors.single_print
                        } else {
                            row_color(price)
                        }
                    } else {
                        row_color(price)
                    }
                };
                let text_color = |price: Price| {
                    poor_color(price).unwrap_or_else(|| {
                        if colors.show_single_print && is_single_print(price) {
                            colors.single_print
                        } else {
                            row_color(price)
                        }
                    })
                };

                let cell_gap = pixels.logical_width_from_physical(1);
                let plot_right = plot.x + plot.width;
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        continue;
                    }
                    let y_span = pixels.snapped_y_span(y - row_height * 0.5, y + row_height * 0.5);
                    let cell_y = PixelSpan {
                        start: y_span.start + cell_gap * 0.5,
                        size: (y_span.size - cell_gap).max(cell_gap),
                    };
                    let configured = cell_color(row.price);
                    let cell_alpha = if !show_letters
                        && (poor_color(row.price).is_some()
                            || (colors.show_single_print && is_single_print(row.price)))
                        || (colors.show_poc && profile.poc == Some(row.price))
                    {
                        0.38
                    } else if profile.val.is_some_and(|val| row.price >= val)
                        && profile.vah.is_some_and(|vah| row.price <= vah)
                    {
                        0.26
                    } else {
                        0.18
                    };
                    for (column_index, period_index) in row.periods.iter().enumerate() {
                        if usize::from(*period_index) >= profile.period_count {
                            continue;
                        }
                        let x_start = line_start + column_width * column_index as f32;
                        let x_end = x_start + column_width;
                        let Some(x_span) =
                            pixels.positive_x_span_in_bounds(x_start, x_end, plot.x, plot_right)
                        else {
                            continue;
                        };
                        let cell_x = PixelSpan {
                            start: x_span.start + cell_gap * 0.5,
                            size: (x_span.size - cell_gap).max(cell_gap),
                        };
                        frame.fill_rectangle(
                            Point::new(cell_x.start, cell_y.start),
                            Size::new(cell_x.size, cell_y.size),
                            color(configured).scale_alpha(cell_alpha),
                        );
                    }
                }
                let line = |frame: &mut PaintInputs,
                            price: Option<Price>,
                            configured: data::config::theme::ThemeColor,
                            width: f32| {
                    let Some(price) = price else { return };
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        return;
                    }
                    frame.stroke(
                        &canvas::Path::line(Point::new(line_start, y), Point::new(line_end, y)),
                        canvas::Stroke::default()
                            .with_width(width)
                            .with_color(color(configured)),
                    );
                };

                line(frame, profile.vah, colors.value_area, line_width);
                line(frame, profile.val, colors.value_area, line_width);
                if colors.show_poc {
                    line(frame, profile.poc, colors.poc, line_width.max(1.5));
                }
                line(frame, profile.session_high, colors.letters, line_width);
                line(frame, profile.session_low, colors.letters, line_width);
                if colors.show_initial_balance {
                    line(
                        frame,
                        profile.initial_balance_high,
                        colors.initial_balance,
                        line_width,
                    );
                    line(
                        frame,
                        profile.initial_balance_low,
                        colors.initial_balance,
                        line_width,
                    );
                }

                if !show_letters {
                    continue;
                }
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        continue;
                    }
                    for (column_index, period_index) in row.periods.iter().enumerate() {
                        if usize::from(*period_index) >= profile.period_count {
                            continue;
                        }
                        let x = line_start + column_width * (column_index as f32 + 0.5);
                        frame.fill_text(canvas::Text {
                            content: TpoProfile::letter_label(usize::from(*period_index)),
                            position: Point::new(x, y),
                            color: color(text_color(row.price)),
                            size: row_height.min(24.0).min(column_width * 0.8).max(8.0).into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                    }
                }
            }
        });
    }

    fn fill_tpo_profile_interaction(
        &self,
        frame: &mut PaintInputs,
        palette: &Extended,
        pixels: PixelGeometry,
        selected_profile: Option<UnixMs>,
        boundary_preview: Option<UnixMs>,
    ) {
        let scene = self.context.scene;
        let geometries = self.tpo_profile_geometries(selected_profile);
        if geometries.is_empty() {
            return;
        }

        let (border_width, _) = pixels.quantized_stroke_width(1.25);
        let border_color = palette.primary.base.color.scale_alpha(0.95);
        for geometry in geometries
            .iter()
            .filter(|geometry| selected_profile == Some(geometry.start))
        {
            frame.stroke(
                &canvas::Path::rectangle(
                    geometry.overlay_bounds.position(),
                    geometry.overlay_bounds.size(),
                ),
                canvas::Stroke::default()
                    .with_color(border_color)
                    .with_width(border_width),
            );
        }

        let Some(boundary_preview) = boundary_preview else {
            return;
        };
        let Some(unit) = scene.x.axis.unit_from_time_f32(boundary_preview) else {
            return;
        };
        let plot = scene.primary_plot();
        let x = scene.layout.regions.plot.x + scene.map_x_plot_f32(unit);
        let y = scene.layout.regions.plot.y + plot.y;
        frame.stroke(
            &canvas::Path::line(Point::new(x, y), Point::new(x, y + plot.height)),
            style::dashed_line_from_palette(palette)
                .with_color(palette.primary.base.color)
                .with_width(border_width),
        );
    }

    fn prepare_tpo_profile_controls(
        &self,
        frame: &mut PreparedChrome,
        palette: &Extended,
        pixels: PixelGeometry,
        selected_profile: Option<UnixMs>,
    ) {
        let scene = self.context.scene;
        let geometries = self.tpo_profile_geometries(selected_profile);
        let (border_width, _) = pixels.quantized_stroke_width(1.25);
        let border_color = palette.primary.base.color.scale_alpha(0.95);
        for geometry in &geometries {
            let Some(profile) = self.object.profiles.get(geometry.index) else {
                continue;
            };
            if profile.status == TpoProfileStatus::Incomplete {
                self.fill_tpo_incomplete_marker(frame, geometry, palette);
            }
        }

        for geometry in geometries
            .iter()
            .filter(|geometry| selected_profile == Some(geometry.start))
        {
            let plot = scene.primary_plot();
            let plot_left = scene.layout.regions.plot.x + plot.x;
            let plot_top = scene.layout.regions.plot.y + plot.y;
            let handle_y = geometry.overlay_bounds.center().y - 10.0;
            let boundaries = [
                (geometry.index > 0).then_some(geometry.start),
                self.object
                    .profiles
                    .get(geometry.index + 1)
                    .map(|profile| profile.start),
            ];
            for boundary in boundaries.into_iter().flatten() {
                let Some(unit) = scene.x.axis.unit_from_time_f32(boundary) else {
                    continue;
                };
                let x = scene.layout.regions.plot.x + scene.map_x_plot_f32(unit);
                if x - 6.0 < plot_left
                    || x + 6.0 > plot_left + plot.width
                    || handle_y < plot_top
                    || handle_y + 20.0 > plot_top + plot.height
                {
                    continue;
                }
                frame.fill_bordered_rectangle(
                    Rectangle::new(Point::new(x - 6.0, handle_y), Size::new(12.0, 20.0)),
                    palette.background.base.color,
                    border_color,
                    border_width,
                );
                for offset in [-2.0, 2.0] {
                    frame.fill_rectangle(
                        Point::new(x + offset - border_width * 0.5, handle_y + 6.0),
                        Size::new(border_width, 8.0),
                        border_color,
                    );
                }
            }

            for (button, label) in [
                (geometry.reset_button_bounds, "Reset"),
                (geometry.button_bounds, "Merge ←"),
            ] {
                let Some(button) = button else {
                    continue;
                };
                frame.fill_bordered_rectangle(
                    button,
                    palette.primary.base.color.scale_alpha(0.92),
                    palette.primary.strong.text.scale_alpha(0.85),
                    border_width,
                );
                frame.fill_text(canvas::Text {
                    content: label.to_owned(),
                    position: Point::new(
                        button.x + button.width * 0.5,
                        button.y + button.height * 0.5,
                    ),
                    color: palette.primary.base.text,
                    size: 10.0.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }
    }

    fn fill_tpo_incomplete_marker(
        &self,
        frame: &mut PreparedChrome,
        geometry: &TpoProfileGeometry,
        palette: &Extended,
    ) {
        let marker_width = CHAR_W;
        let marker_height = TEXT_SIZE;
        let marker_right = geometry
            .reset_button_bounds
            .or(geometry.button_bounds)
            .map_or(
                geometry.overlay_bounds.x + geometry.overlay_bounds.width
                    - TPO_ACTION_BUTTON_GAP_PX,
                |button| button.x - TPO_ACTION_BUTTON_GAP_PX,
            );
        if marker_right - marker_width < geometry.overlay_bounds.x + TPO_ACTION_BUTTON_GAP_PX {
            return;
        }
        let marker_x = marker_right - marker_width * 0.5;
        let marker_y = geometry.control_row_y + TPO_ACTION_BUTTON_HEIGHT_PX * 0.5;

        frame.fill_text(canvas::Text {
            content: "!".to_owned(),
            position: Point::new(marker_x, marker_y),
            color: palette.danger.base.color,
            size: marker_height.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct TpoDrag {
    boundary: UnixMs,
    preview: UnixMs,
    minimum: Option<UnixMs>,
    maximum: Option<UnixMs>,
}

impl TpoState {
    fn selected_profile(&self) -> Option<UnixMs> {
        self.selected_profile
    }
}

impl TpoHit {
    fn priority(self) -> i16 {
        match self {
            Self::Click(TpoClick::Profile { .. }) => 0,
            _ => 3,
        }
    }

    fn cursor_appearance(self) -> iced::advanced::mouse::Interaction {
        match self {
            Self::Boundary { .. } => iced::advanced::mouse::Interaction::ResizingHorizontally,
            Self::Click(_) => iced::advanced::mouse::Interaction::Pointer,
        }
    }
}

impl TpoView<'_> {
    fn hit(&self, point: Point, state: &TpoState) -> Option<TpoHit> {
        if self.context.scene.layout.hit_test(point)
            != super::super::layout::LayoutHitZone::PanelPlot(
                self.context.scene.primary.panel_index,
            )
        {
            return None;
        }
        self.tpo_hit_test(point, state.selected_profile)
    }

    fn begin(&self, hit: TpoHit, state: &mut TpoState) -> LayerEffects<TpoInteraction> {
        let mut effects = LayerEffects::captured(RenderRequest::Interaction);
        let gesture = match hit {
            TpoHit::Click(click) => {
                state.pending_click = Some(click);
                LayerGesture::ClickOrNavigate {
                    selection: super::super::SelectionDisposition::Clear,
                    captures_outside_release: true,
                    fallback: if matches!(click, TpoClick::Profile { .. }) {
                        NavigationFallback::PanOrMarquee
                    } else {
                        NavigationFallback::Pan
                    },
                }
            }
            TpoHit::Boundary {
                boundary,
                minimum,
                maximum,
            } => {
                state.selected_profile = Some(boundary);
                state.drag = Some(TpoDrag {
                    boundary,
                    preview: boundary,
                    minimum,
                    maximum,
                });
                LayerGesture::Drag {
                    selection: super::super::SelectionDisposition::Clear,
                    active_outside_bounds: true,
                    cursor: iced::advanced::mouse::Interaction::ResizingHorizontally,
                }
            }
        };
        effects.gesture = Some(gesture);
        effects
    }

    fn update_drag(&self, drag: &mut TpoDrag) -> LayerEffects<TpoInteraction> {
        let Some(point) = self.context.position else {
            return LayerEffects::default();
        };
        if let Some(preview) =
            self.tpo_boundary_from_cursor(point, drag.boundary, drag.minimum, drag.maximum)
        {
            drag.preview = preview;
            LayerEffects::captured(RenderRequest::Interaction)
        } else {
            LayerEffects::default()
        }
    }
}

impl TpoState {
    fn release_press(&mut self, press: TpoClick, confirmed: bool) -> LayerEffects<TpoInteraction> {
        if !confirmed {
            return LayerEffects::default();
        }
        let mut effects = LayerEffects::captured(RenderRequest::All);
        match press {
            TpoClick::Reset { start, end } => effects.publish(TpoInteraction::Reset { start, end }),
            TpoClick::MergeLeft {
                boundary,
                left_start,
            } => {
                self.selected_profile = Some(left_start);
                effects.publish(TpoInteraction::MergeLeft { boundary });
            }
            TpoClick::Profile { start } => {
                self.selected_profile = (self.selected_profile != Some(start)).then_some(start);
                effects.render_request = RenderRequest::Interaction;
            }
        }
        effects
    }

    fn release_drag(&mut self, drag: TpoDrag) -> LayerEffects<TpoInteraction> {
        self.selected_profile = Some(drag.preview);
        let mut effects = LayerEffects::captured(RenderRequest::All);
        if drag.boundary != drag.preview {
            effects.publish(TpoInteraction::MoveBoundary {
                from: drag.boundary,
                to: drag.preview,
            });
        }
        effects
    }
}

impl TpoState {
    fn plot_clicked(&mut self, target_is_self: bool) -> LayerEffects<TpoInteraction> {
        if !target_is_self && self.clear_selection() {
            LayerEffects::captured(RenderRequest::Interaction)
        } else {
            LayerEffects::default()
        }
    }
}

impl TpoObject<'_> {
    pub(super) fn key(&self) -> StudyObjectKey {
        self.key
    }

    pub(super) fn hit_test(&self, context: LayerContext<'_>, state: &TpoState) -> Option<LayerHit> {
        let point = context.position?;
        let hit = self.view(context).hit(point, state)?;
        Some(LayerHit {
            key: self.key,
            priority: hit.priority(),
            precedes_tools: true,
            readouts_visible: true,
            cursor: hit.cursor_appearance(),
        })
    }

    pub(super) fn pointer(
        &self,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        state: &mut TpoState,
    ) -> LayerEffects<TpoInteraction> {
        match input {
            LayerPointerInput::PlotClicked { target } => {
                state.plot_clicked(target == Some(self.key))
            }
            LayerPointerInput::Pressed => {
                let Some(point) = context.position else {
                    return LayerEffects::default();
                };
                self.view(context)
                    .hit(point, state)
                    .map_or_else(LayerEffects::default, |hit| {
                        self.view(context).begin(hit, state)
                    })
            }
            LayerPointerInput::Moved => state
                .drag
                .as_mut()
                .map_or_else(LayerEffects::default, |drag| {
                    self.view(context).update_drag(drag)
                }),
            LayerPointerInput::Clicked { confirmed } => state
                .pending_click
                .take()
                .map_or_else(LayerEffects::default, |click| {
                    state.release_press(click, confirmed)
                }),
            LayerPointerInput::DragReleased => state
                .drag
                .take()
                .map_or_else(LayerEffects::default, |drag| state.release_drag(drag)),
            LayerPointerInput::Cancelled => {
                state.pending_click = None;
                state.drag = None;
                LayerEffects::default()
            }
        }
    }

    pub(super) fn prepare_controls(
        &self,
        context: LayerContext<'_>,
        state: &TpoState,
        controls: &mut PreparedChrome,
        palette: &Extended,
        pixels: PixelGeometry,
    ) {
        self.view(context).prepare_tpo_profile_controls(
            controls,
            palette,
            pixels,
            state.selected_profile(),
        );
    }

    pub(super) fn paint(
        &self,
        phase: LayerPaintPhase,
        context: LayerContext<'_>,
        state: &TpoState,
        frame: &mut PaintInputs,
        palette: &Extended,
        accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let view = self.view(context);
        match phase {
            LayerPaintPhase::Underlay => {
                view.fill_tpo_profile_overlay(frame, palette, accents, pixels);
            }
            LayerPaintPhase::BeforeIndicators | LayerPaintPhase::AfterIndicators => {}
            LayerPaintPhase::Overlay => {
                view.fill_tpo_profile_interaction(
                    frame,
                    palette,
                    pixels,
                    state.selected_profile(),
                    state.drag.map(|drag| drag.preview),
                );
            }
        }
    }
}

fn tpo_row_height(scene: &Scene, profile: &TpoProfile, anchor: Option<f32>) -> f32 {
    let row_height = if profile.rows.len() < 2 {
        12.0
    } else {
        profile
            .rows
            .windows(2)
            .filter_map(|rows| {
                let y1 =
                    scene.map_primary_plot_with_anchor_f32(rows[0].price.to_f32_lossy(), anchor);
                let y2 =
                    scene.map_primary_plot_with_anchor_f32(rows[1].price.to_f32_lossy(), anchor);
                (y1.is_finite() && y2.is_finite()).then_some((y1 - y2).abs())
            })
            .fold(f32::INFINITY, f32::min)
    };

    if row_height.is_finite() {
        row_height
    } else {
        0.0
    }
}
