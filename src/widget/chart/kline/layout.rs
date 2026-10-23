use super::Y_AXIS_LABEL_LEFT_INSET;
use super::composition::PanelSpec;
use crate::widget::chart::panel_splitter_hit_bounds;
pub(super) use data::chart::axis::place_non_overlapping_labels;
use iced::Size;
use iced::advanced::Layout;
use iced::advanced::layout as iced_layout;
use iced::{Point, Rectangle};

/// Guard area on the edges of the axis
/// to prevent conflicts with pane state interactions
///
/// (e.g. pane split dragging when trying to interact with labels)
const AXIS_DRAG_EDGE_GUARD: f32 = 4.0;

/// Inclusive edge test: rects that share a boundary both own it, so hit tests
/// stay continuous across plot, axis, and splitter seams.
pub(super) fn contains_inclusive(rect: Rectangle, p: Point) -> bool {
    p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
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
    x_axis: Rectangle,
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

        let panel_kind_for_index = |index: usize| match panels_spec.get(index) {
            Some(panel) if panel.id.is_main() => KlinePanelKind::PrimaryChart,
            Some(_) => KlinePanelKind::Indicator,
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
            if contains_inclusive(panel_splitter_hit_bounds(splitter), plot_local) {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Regions {
    pub(super) plot: Rectangle,
    pub(super) x_axis: Rectangle,
    pub(super) y_axis: Rectangle,
}

impl Regions {
    fn from_layout(root: iced_core::Layout<'_>) -> Self {
        let root_bounds = root.bounds();

        let row = root.child(0);
        let x_abs = root.child(1).bounds();

        let plot_abs = row.child(0).bounds();
        let y_abs = row.child(1).bounds();

        let to_local = |r: Rectangle| Rectangle {
            x: r.x - root_bounds.x,
            y: r.y - root_bounds.y,
            width: r.width,
            height: r.height,
        };

        Regions {
            plot: to_local(plot_abs),
            y_axis: to_local(y_abs),
            x_axis: to_local(x_abs),
        }
    }
}

pub(super) fn build_panel_stack(
    panel_count: usize,
    normalized_splits: &[f32],
    stack_size: Size,
    splitter_height: f32,
) -> iced_layout::Node {
    let plot_heights = crate::widget::chart::panel_plot_heights(
        stack_size.height,
        panel_count,
        normalized_splits,
        splitter_height,
    );
    let mut children = Vec::with_capacity(panel_count.saturating_mul(3).saturating_sub(1));

    let mut y = 0.0;

    for panel_index in 0..panel_count {
        let plot_h = plot_heights.get(panel_index).copied().unwrap_or_default();

        children.push(
            iced_layout::Node::new(Size::new(stack_size.width, plot_h)).move_to(Point::new(0.0, y)),
        );
        y += plot_h;

        let axis_h = if panel_index + 1 == panel_count {
            (stack_size.height - y).max(0.0)
        } else {
            0.0
        };

        children.push(
            iced_layout::Node::new(Size::new(stack_size.width, axis_h)).move_to(Point::new(0.0, y)),
        );
        y += axis_h;

        if panel_index + 1 < panel_count {
            children.push(
                iced_layout::Node::new(Size::new(stack_size.width, splitter_height))
                    .move_to(Point::new(0.0, y)),
            );
            y += splitter_height;
        }
    }

    iced_layout::Node::with_children(stack_size, children)
}

impl PanelLayoutTree {
    pub(super) fn split_ratio_from_cursor(
        &self,
        cursor_y: f32,
        split_index: usize,
        splitter_height: f32,
    ) -> Option<f32> {
        crate::widget::chart::split_ratio_from_cursor(
            (cursor_y - self.regions.plot.y).clamp(0.0, self.regions.plot.height),
            split_index,
            self.panels.len(),
            self.panels.iter().map(|panel| panel.plot.height).sum(),
            splitter_height,
        )
    }
}
