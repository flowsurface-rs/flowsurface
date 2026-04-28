use super::{
    KlinePanelKind, KlineSeriesLike, KlineWidget, MIN_PANEL_HEIGHT, PANEL_SPLITTER_HEIGHT,
    PANEL_SPLITTER_HIT_PX,
};
use crate::widget::chart::Regions;
use iced::advanced::Layout;
use iced::{Point, Rectangle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LayoutHitZone {
    PanelPlot(usize),
    PanelXAxis(usize),
    BottomXAxis,
    YAxis,
    Splitter(usize),
    Outside,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PanelLayoutNode {
    pub(super) kind: KlinePanelKind,
    pub(super) plot: Rectangle,
    pub(super) x_axis: Rectangle,
}

#[derive(Debug, Clone)]
pub(super) struct PanelLayoutTree {
    pub(super) regions: Regions,
    pub(super) panels: Vec<PanelLayoutNode>,
    pub(super) splitters: Vec<Rectangle>,
}

impl PanelLayoutTree {
    fn child_at(layout: Layout<'_>, index: usize) -> Option<Layout<'_>> {
        (index < layout.children().len()).then(|| layout.child(index))
    }

    pub(super) fn from_layout(root: Layout<'_>, panel_kinds: &[KlinePanelKind]) -> Option<Self> {
        if panel_kinds.is_empty() {
            return None;
        }

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

        let mut cursor = 0usize;
        let mut panel_nodes = Vec::with_capacity(panel_kinds.len());
        let mut splitters = Vec::with_capacity(panel_kinds.len().saturating_sub(1));

        for (index, kind) in panel_kinds.iter().copied().enumerate() {
            let plot = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            let x_axis = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            panel_nodes.push(PanelLayoutNode { kind, plot, x_axis });

            if index + 1 < panel_kinds.len() {
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

    pub(super) fn contains(rect: Rectangle, p: Point) -> bool {
        p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
    }

    pub(super) fn plot_local_point(&self, root_local: Point) -> Option<Point> {
        self.regions.is_in_plot(root_local).then_some(Point::new(
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
        if self.regions.is_in_y_axis(root_local) {
            return LayoutHitZone::YAxis;
        }

        if self.regions.is_in_x_axis(root_local) {
            return LayoutHitZone::BottomXAxis;
        }

        let Some(plot_local) = self.plot_local_point(root_local) else {
            return LayoutHitZone::Outside;
        };

        for (index, splitter) in self.splitters.iter().copied().enumerate() {
            if Self::contains(Self::splitter_hit_rect(splitter), plot_local) {
                return LayoutHitZone::Splitter(index);
            }
        }

        for (index, panel) in self.panels.iter().enumerate() {
            if Self::contains(panel.plot, plot_local) {
                return LayoutHitZone::PanelPlot(index);
            }

            if panel.x_axis.height > 0.0 && Self::contains(panel.x_axis, plot_local) {
                return LayoutHitZone::PanelXAxis(index);
            }
        }

        LayoutHitZone::Outside
    }
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    fn panel_min_ratio(&self, panel_count: usize, usable_plot_height: f32) -> f32 {
        if panel_count <= 1 {
            return 0.0;
        }

        let usable = usable_plot_height.max(1.0);
        let geometric_min = MIN_PANEL_HEIGHT / usable;
        let feasible_cap = 1.0 / panel_count as f32;

        geometric_min.min(feasible_cap)
    }

    fn normalized_panel_splits(&self, panel_count: usize, usable_plot_height: f32) -> Vec<f32> {
        let split_count = panel_count.saturating_sub(1);
        if split_count == 0 {
            return Vec::new();
        }

        let mut splits = Vec::with_capacity(split_count);
        for index in 0..split_count {
            let fallback = (index + 1) as f32 / panel_count as f32;
            splits.push(self.panel_splits.get(index).copied().unwrap_or(fallback));
        }

        let min_ratio = self.panel_min_ratio(panel_count, usable_plot_height);

        for index in 0..split_count {
            let remaining_panels_after = panel_count.saturating_sub(index + 1);

            let lower = if index > 0 {
                splits[index - 1] + min_ratio
            } else {
                min_ratio
            };

            let upper = 1.0 - (remaining_panels_after as f32 * min_ratio);
            let (min_bound, max_bound) = if lower <= upper {
                (lower, upper)
            } else {
                (upper, lower)
            };

            splits[index] = splits[index].clamp(min_bound, max_bound);
        }

        splits
    }

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

        let splits = self.normalized_panel_splits(panel_count, usable.max(1.0));
        let mut heights = Vec::with_capacity(panel_count);
        let mut previous = 0.0;

        for split in splits {
            let boundary = split.clamp(0.0, 1.0) * usable;
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
        let boundary = (local_y - fixed_before).clamp(0.0, usable);
        let ratio = (boundary / usable).clamp(0.0, 1.0);

        let splits = self.normalized_panel_splits(panel_count, usable);
        let min_ratio = self.panel_min_ratio(panel_count, usable);

        let lower = if split_index > 0 {
            splits[split_index - 1] + min_ratio
        } else {
            min_ratio
        };

        let upper = if split_index + 1 < splits.len() {
            splits[split_index + 1] - min_ratio
        } else {
            1.0 - min_ratio
        };

        let (min_bound, max_bound) = if lower <= upper {
            (lower, upper)
        } else {
            (upper, lower)
        };

        Some(ratio.clamp(min_bound, max_bound))
    }
}
