use super::Regions;
use super::composition::{PanelRole, PanelSpec};
use super::{KlineWidget, PANEL_SPLITTER_HEIGHT, Y_AXIS_LABEL_LEFT_INSET};
use iced::advanced::Layout;
use iced::{Point, Rectangle};

/// Guard area on the edges of the axis
/// to prevent conflicts with pane state interactions
///
/// (e.g. pane split dragging when trying to interact with labels)
const AXIS_DRAG_EDGE_GUARD: f32 = 4.0;

const PANEL_SPLITTER_HIT_PX: f32 = 8.0;

/// Inclusive edge test: rects that share a boundary both own it, so hit tests
/// stay continuous across plot, axis, and splitter seams.
pub(super) fn contains_inclusive(rect: Rectangle, p: Point) -> bool {
    p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
}

pub(super) fn place_non_overlapping_labels(
    desired_centers: &[f32],
    min_y: f32,
    max_y: f32,
    label_height: f32,
    gap: f32,
) -> Vec<Option<f32>> {
    if desired_centers.is_empty() {
        return Vec::new();
    }

    let label_height = label_height.max(1.0);
    let min_center = min_y + label_height * 0.5;
    let max_center = (max_y - label_height * 0.5).max(min_center);
    let spacing = (label_height + gap.max(0.0)).max(label_height);
    let capacity = ((max_center - min_center) / spacing).floor() as usize + 1;
    let visible_count = desired_centers.len().min(capacity.max(1));

    let mut candidates: Vec<(usize, f32)> = desired_centers.iter().copied().enumerate().collect();
    candidates.sort_by(|(_, left), (_, right)| left.total_cmp(right));

    let selected = if visible_count == candidates.len() {
        candidates
    } else {
        (0..visible_count)
            .map(|slot| {
                let source_index = if visible_count == 1 {
                    candidates.len() / 2
                } else {
                    slot * (candidates.len() - 1) / (visible_count - 1)
                };
                candidates[source_index]
            })
            .collect()
    };

    let mut centers: Vec<f32> = selected
        .iter()
        .map(|(_, center)| center.clamp(min_center, max_center))
        .collect();

    if centers.len() > 1 {
        centers[0] = centers[0].max(min_center);

        for index in 1..centers.len() {
            centers[index] = centers[index].max(centers[index - 1] + spacing);
        }

        if centers[centers.len() - 1] > max_center {
            let last = centers.len() - 1;
            centers[last] = max_center;

            for index in (0..last).rev() {
                centers[index] = centers[index].min(centers[index + 1] - spacing);
            }
        }
    }

    let mut placements = vec![None; desired_centers.len()];
    for ((source_index, _), center) in selected.into_iter().zip(centers) {
        placements[source_index] = Some(center);
    }

    placements
}

pub(super) fn label_bounds_overlap(
    first_center: f32,
    first_height: f32,
    second_center: f32,
    second_height: f32,
    gap: f32,
) -> bool {
    (first_center - second_center).abs() < (first_height + second_height) * 0.5 + gap.max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KlinePanelKind {
    PrimaryChart,
    Indicator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LayoutHitZone {
    PanelPlot(usize),
    PanelXAxis(usize),
    BottomXAxis,
    YAxis(usize),
    Splitter(usize),
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PanelLayoutNode {
    pub(super) kind: KlinePanelKind,
    pub(super) plot: Rectangle,
    pub(super) y_axis: Rectangle,
    pub(super) x_axis: Rectangle,
}

impl PanelLayoutNode {
    pub(super) fn y_axis_label_y(&self, y: f32, text_size: f32) -> f32 {
        let inset = (text_size * 0.6).min(self.y_axis.height / 2.0);
        y.clamp(
            self.y_axis.y + inset,
            self.y_axis.y + self.y_axis.height - inset,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PanelLayoutTree {
    pub(super) regions: Regions,
    pub(super) panels: Vec<PanelLayoutNode>,
    pub(super) splitters: Vec<Rectangle>,
}

impl PanelLayoutTree {
    fn child_at(layout: Layout<'_>, index: usize) -> Option<Layout<'_>> {
        (index < layout.children().len()).then(|| layout.child(index))
    }

    pub(super) fn from_layout(root: Layout<'_>, panels_spec: &[PanelSpec]) -> Option<Self> {
        let panel_count = panels_spec.len().max(1);

        let regions = Regions::from_layout(root);

        let row = Self::child_at(root, 0)?;
        let panels = Self::child_at(row, 0)?;

        let panels_bounds = panels.bounds();
        let to_plot_local = |r: Rectangle| Rectangle {
            x: r.x - panels_bounds.x,
            y: r.y - panels_bounds.y,
            width: r.width,
            height: r.height,
        };

        let panel_kind_for_index =
            |index: usize| match panels_spec.get(index).map(|panel| panel.role) {
                Some(PanelRole::Primary) => KlinePanelKind::PrimaryChart,
                Some(PanelRole::Auxiliary) => KlinePanelKind::Indicator,
                None if index == 0 => KlinePanelKind::PrimaryChart,
                None => KlinePanelKind::Indicator,
            };

        let mut cursor = 0usize;
        let mut panel_nodes = Vec::with_capacity(panel_count);
        let mut splitters = Vec::with_capacity(panel_count.saturating_sub(1));

        for index in 0..panel_count {
            let plot = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            let x_axis = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            panel_nodes.push(PanelLayoutNode {
                kind: panel_kind_for_index(index),
                plot,
                y_axis: Rectangle {
                    x: 0.0,
                    y: plot.y,
                    width: regions.y_axis.width,
                    height: plot.height,
                },
                x_axis,
            });

            if index + 1 < panel_count {
                splitters.push(to_plot_local(Self::child_at(panels, cursor)?.bounds()));
                cursor += 1;
            }
        }

        Some(Self {
            regions,
            panels: panel_nodes,
            splitters,
        })
    }

    pub(super) fn panel(&self, index: usize) -> Option<&PanelLayoutNode> {
        self.panels.get(index)
    }

    pub(super) fn y_axis_badge_bounds(&self, y: f32, height: f32, content_width: f32) -> Rectangle {
        let axis = self.regions.y_axis;
        let right = axis.x + axis.width;
        let width = axis
            .width
            .max(content_width + Y_AXIS_LABEL_LEFT_INSET * 2.0)
            .min((right - self.regions.plot.x).max(0.0));

        Rectangle {
            x: right - width,
            y,
            width,
            height,
        }
    }

    pub(super) fn primary_plot_width(&self) -> Option<f32> {
        let primary_panel = self
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_plot_width = self.panel(primary_panel)?.plot.width.max(1.0);

        Some(
            (self.regions.x_axis.width - self.regions.y_axis.width)
                .max(1.0)
                .min(primary_plot_width),
        )
    }

    pub(super) fn plot_local_point(&self, root_local: Point) -> Option<Point> {
        contains_inclusive(self.regions.plot, root_local).then_some(Point::new(
            root_local.x - self.regions.plot.x,
            root_local.y - self.regions.plot.y,
        ))
    }

    fn splitter_hit_rect(splitter: Rectangle) -> Rectangle {
        let hit_h = PANEL_SPLITTER_HIT_PX;
        let center_y = splitter.y + splitter.height * 0.5;

        Rectangle {
            x: splitter.x,
            y: center_y - hit_h * 0.5,
            width: splitter.width,
            height: hit_h,
        }
    }

    pub(super) fn hit_test(&self, root_local: Point) -> LayoutHitZone {
        if contains_inclusive(self.regions.y_axis, root_local) {
            let panel_local_y = root_local.y - self.regions.plot.y;

            for (index, panel) in self.panels.iter().enumerate() {
                let top = panel.plot.y.min(panel.x_axis.y);
                let bottom =
                    (panel.plot.y + panel.plot.height).max(panel.x_axis.y + panel.x_axis.height);

                if panel_local_y >= top && panel_local_y <= bottom {
                    return LayoutHitZone::YAxis(index);
                }
            }

            if let Some(last_index) = self.panels.len().checked_sub(1) {
                return LayoutHitZone::YAxis(last_index);
            }

            return LayoutHitZone::Outside;
        }

        if contains_inclusive(self.regions.x_axis, root_local) {
            if contains_inclusive(self.regions.x_axis.shrink(AXIS_DRAG_EDGE_GUARD), root_local) {
                return LayoutHitZone::BottomXAxis;
            }

            return LayoutHitZone::Outside;
        }

        let Some(plot_local) = self.plot_local_point(root_local) else {
            return LayoutHitZone::Outside;
        };

        for (index, splitter) in self.splitters.iter().copied().enumerate() {
            if contains_inclusive(Self::splitter_hit_rect(splitter), plot_local) {
                return LayoutHitZone::Splitter(index);
            }
        }

        for (index, panel) in self.panels.iter().enumerate() {
            if contains_inclusive(panel.plot, plot_local) {
                return LayoutHitZone::PanelPlot(index);
            }

            if panel.x_axis.height > 0.0
                && contains_inclusive(panel.x_axis.shrink(AXIS_DRAG_EDGE_GUARD), plot_local)
            {
                return LayoutHitZone::PanelXAxis(index);
            }
        }

        LayoutHitZone::Outside
    }
}

impl<'a> KlineWidget<'a> {
    pub(super) fn panel_plot_heights(
        &self,
        panel_stack_height: f32,
        panel_count: usize,
    ) -> Vec<f32> {
        if panel_count == 0 {
            return Vec::new();
        }

        let non_plot = panel_count.saturating_sub(1) as f32 * PANEL_SPLITTER_HEIGHT;
        let usable = (panel_stack_height - non_plot).max(0.0);

        if panel_count == 1 {
            return vec![usable];
        }

        let mut splits = self.normalized_panel_splits();
        let split_count = panel_count.saturating_sub(1);

        if splits.len() > split_count {
            splits.truncate(split_count);
        }

        for index in splits.len()..split_count {
            let fallback = (index + 1) as f32 / panel_count as f32;
            splits.push(fallback);
        }

        let mut heights = Vec::with_capacity(panel_count);
        let mut previous = 0.0;

        for split in splits {
            let boundary = (split.clamp(0.0, 1.0) * usable)
                .round()
                .clamp(previous, usable);
            heights.push((boundary - previous).max(0.0));
            previous = boundary;
        }

        heights.push((usable - previous).max(0.0));
        heights
    }

    pub(super) fn split_ratio_from_cursor(
        &self,
        cursor_y: f32,
        layout: &PanelLayoutTree,
        split_index: usize,
    ) -> Option<f32> {
        let panel_count = layout.panels.len();
        let split_count = panel_count.saturating_sub(1);

        if split_count == 0 || split_index >= split_count {
            return None;
        }

        let local_y = (cursor_y - layout.regions.plot.y).clamp(0.0, layout.regions.plot.height);
        let usable_plot_height: f32 = layout.panels.iter().map(|panel| panel.plot.height).sum();
        let usable = usable_plot_height.max(1.0);

        let fixed_before =
            (split_index as f32 * PANEL_SPLITTER_HEIGHT) + (PANEL_SPLITTER_HEIGHT * 0.5);
        let boundary = (local_y - fixed_before).round().clamp(0.0, usable);

        Some((boundary / usable).clamp(0.0, 1.0))
    }
}
