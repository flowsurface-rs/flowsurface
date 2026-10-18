use crate::widget::chart::{PANEL_SPLITTER_HEIGHT, panel_plot_heights, panel_splitter_hit_bounds};
use data::chart::heatmap::indicators::{HeatmapIndicatorConfig, HeatmapPanel};
use iced::{
    Point, Rectangle, Size,
    advanced::{Layout, layout},
    mouse,
};

#[derive(Clone, Copy)]
pub(super) struct IndicatorRegion {
    pub kind: HeatmapPanel,
    pub splitter: Rectangle,
    pub bounds: Rectangle,
}

impl IndicatorRegion {
    pub fn plot_bounds(self, width: f32) -> Rectangle {
        Rectangle {
            width,
            ..self.bounds
        }
    }

    fn axis_bounds(self, plot_width: f32) -> Rectangle {
        Rectangle {
            x: self.bounds.x + plot_width,
            width: (self.bounds.width - plot_width).max(0.0),
            ..self.bounds
        }
    }
}

pub(super) struct HeatmapLayout {
    pub plot: Rectangle,
    pub y_axis: Rectangle,
    pub x_axis: Rectangle,
    pub indicators: Vec<IndicatorRegion>,
}

impl HeatmapLayout {
    pub fn from_layout(layout: Layout<'_>, config: &HeatmapIndicatorConfig) -> Self {
        let mut children = layout.children();
        let plot = children.next().expect("heatmap plot").bounds();
        let y_axis = children.next().expect("heatmap price axis").bounds();
        let x_axis = children.next().expect("heatmap time axis").bounds();
        let _corner = children.next().expect("heatmap axis corner");
        let indicators = config
            .enabled_panels()
            .map(|kind| IndicatorRegion {
                kind,
                splitter: children.next().expect("indicator splitter").bounds(),
                bounds: children.next().expect("indicator bounds").bounds(),
            })
            .collect();

        Self {
            plot,
            y_axis,
            x_axis,
            indicators,
        }
    }

    pub fn stack_bounds(&self) -> Rectangle {
        Rectangle {
            height: (self.x_axis.y - self.plot.y).max(0.0),
            ..self.plot
        }
    }

    pub fn indicator_axis_at(&self, cursor: mouse::Cursor) -> Option<IndicatorRegion> {
        self.indicators.iter().copied().find(|region| {
            cursor
                .position_over(region.axis_bounds(self.plot.width))
                .is_some()
        })
    }

    pub fn indicator_plot_at(&self, cursor: mouse::Cursor) -> Option<IndicatorRegion> {
        self.indicators.iter().copied().find(|region| {
            cursor
                .position_over(region.plot_bounds(self.plot.width))
                .is_some()
        })
    }

    pub fn splitter_at(&self, cursor: mouse::Cursor) -> Option<usize> {
        self.indicators.iter().position(|region| {
            cursor
                .position_over(panel_splitter_hit_bounds(region.splitter))
                .is_some()
        })
    }
}

pub(super) fn build_node(
    size: Size,
    gutter_width: f32,
    axis_height: f32,
    config: &HeatmapIndicatorConfig,
) -> layout::Node {
    let plot_width = (size.width - gutter_width).max(0.0);
    let stack_height = (size.height - axis_height).max(0.0);
    let weights = config.normalized_weights();
    let splits: Vec<_> = weights
        .iter()
        .take(weights.len().saturating_sub(1))
        .scan(0.0, |sum, weight| {
            *sum += weight;
            Some(*sum)
        })
        .collect();
    let heights = panel_plot_heights(stack_height, weights.len(), &splits, PANEL_SPLITTER_HEIGHT);
    let primary_height = heights[0];
    let mut children = vec![
        layout::Node::new(Size::new(plot_width, primary_height)),
        layout::Node::new(Size::new(gutter_width, primary_height))
            .move_to(Point::new(plot_width, 0.0)),
        layout::Node::new(Size::new(plot_width, axis_height))
            .move_to(Point::new(0.0, stack_height)),
        layout::Node::new(Size::new(gutter_width, axis_height))
            .move_to(Point::new(plot_width, stack_height)),
    ];

    let mut y = primary_height;
    for height in heights.into_iter().skip(1) {
        children.push(
            layout::Node::new(Size::new(size.width, PANEL_SPLITTER_HEIGHT))
                .move_to(Point::new(0.0, y)),
        );
        y += PANEL_SPLITTER_HEIGHT;
        children.push(layout::Node::new(Size::new(size.width, height)).move_to(Point::new(0.0, y)));
        y += height;
    }

    layout::Node::with_children(size, children)
}
