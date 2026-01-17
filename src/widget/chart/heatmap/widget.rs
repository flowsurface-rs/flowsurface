use crate::widget::chart::heatmap::overlay::OverlayCanvas;
use crate::widget::chart::heatmap::scale::AxisInteraction;
use crate::widget::chart::heatmap::scale::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::scale::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::scene::{InteractionKind, Scene};

use super::Message;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, renderer};
use iced::widget::shader::Program;
use iced::widget::{canvas, shader};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};
use iced_core::renderer::Quad;

const DEFAULT_Y_AXIS_GUTTER: f32 = 66.0; // px
const DEFAULT_X_AXIS_HEIGHT: f32 = 24.0; // px

pub struct HeatmapShaderWidget<'a> {
    scene: &'a Scene,
    x_axis: AxisXLabelCanvas<'a>,
    y_axis: AxisYLabelCanvas<'a>,
    overlay: OverlayCanvas<'a>,
    y_axis_gutter_px: f32,
    x_axis_height_px: f32,
}

impl<'a> HeatmapShaderWidget<'a> {
    pub fn new(
        scene: &'a Scene,
        x_axis: AxisXLabelCanvas<'a>,
        y_axis: AxisYLabelCanvas<'a>,
        overlay: OverlayCanvas<'a>,
    ) -> Self {
        Self {
            scene,
            x_axis,
            y_axis,
            overlay,
            y_axis_gutter_px: DEFAULT_Y_AXIS_GUTTER,
            x_axis_height_px: DEFAULT_X_AXIS_HEIGHT,
        }
    }

    pub fn with_y_axis_gutter_px(mut self, px: f32) -> Self {
        self.y_axis_gutter_px = px.max(0.0);
        self
    }

    pub fn with_x_axis_height_px(mut self, px: f32) -> Self {
        self.x_axis_height_px = px.max(0.0);
        self
    }
}

#[derive(Default)]
struct State {
    scene_state: <Scene as shader::Program<Message>>::State,

    x_axis_state: <AxisXLabelCanvas<'static> as canvas::Program<Message>>::State,
    y_axis_state: <AxisYLabelCanvas<'static> as canvas::Program<Message>>::State,
    overlay_state: <OverlayCanvas<'static> as canvas::Program<Message>>::State,

    // `Message::BoundsChanged` publishes when plot bounds change,
    // so `HeatmapShader` keeps working even if the current event is over an axis.
    last_plot_size: [f32; 2],
}

impl State {
    // Children order (root):
    // 0 plot, 1 y_axis, 2 x_axis, 3 corner
    fn bounds_from_layout(layout: Layout<'_>) -> (Rectangle, Rectangle, Rectangle, Rectangle) {
        let mut children = layout.children();

        let plot = children
            .next()
            .expect("HeatmapShaderWidget layout: missing plot child")
            .bounds();

        let y_axis = children
            .next()
            .expect("HeatmapShaderWidget layout: missing y_axis child")
            .bounds();

        let x_axis = children
            .next()
            .expect("HeatmapShaderWidget layout: missing x_axis child")
            .bounds();

        let corner = children
            .next()
            .expect("HeatmapShaderWidget layout: missing corner child")
            .bounds();

        (plot, y_axis, x_axis, corner)
    }

    fn apply_action(shell: &mut Shell<'_, Message>, action: iced::widget::canvas::Action<Message>) {
        let (message, redraw_request, event_status) = action.into_inner();

        shell.request_redraw_at(redraw_request);

        if let Some(message) = message {
            shell.publish(message);
        }

        if event_status == iced::event::Status::Captured {
            shell.capture_event();
        }
    }
}

impl<'a> Widget<Message, Theme, Renderer> for HeatmapShaderWidget<'a> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);

        let gutter_w = self.y_axis_gutter_px.min(size.width).max(0.0);
        let x_axis_h = self.x_axis_height_px.min(size.height).max(0.0);

        let plot_w = (size.width - gutter_w).max(0.0);
        let plot_h = (size.height - x_axis_h).max(0.0);

        let plot_node = layout::Node::new(Size::new(plot_w, plot_h));

        let y_axis_node =
            layout::Node::new(Size::new(gutter_w, plot_h)).move_to(iced::Point::new(plot_w, 0.0));

        // X axis must match plot width
        let x_axis_node =
            layout::Node::new(Size::new(plot_w, x_axis_h)).move_to(iced::Point::new(0.0, plot_h));

        // Bottom-right empty corner cell
        let corner_node = layout::Node::new(Size::new(gutter_w, x_axis_h))
            .move_to(iced::Point::new(plot_w, plot_h));

        layout::Node::with_children(size, vec![plot_node, y_axis_node, x_axis_node, corner_node])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        use iced::widget::canvas::Program as _;

        let (plot_bounds, y_axis_bounds, x_axis_bounds, _corner) =
            State::bounds_from_layout(layout);

        let state = tree.state.downcast_mut::<State>();

        // Ensure HeatmapShader gets bounds updates even when interacting with axes
        let plot_size = [plot_bounds.width, plot_bounds.height];
        if plot_size != state.last_plot_size && plot_bounds.width > 0.0 && plot_bounds.height > 0.0
        {
            state.last_plot_size = plot_size;
            shell.publish(Message::BoundsChanged(plot_bounds));
        }

        let over_x = cursor.position_over(x_axis_bounds).is_some();
        let over_y = cursor.position_over(y_axis_bounds).is_some();
        let over_plot = cursor.position_over(plot_bounds).is_some();

        let x_dragging = matches!(state.x_axis_state, AxisInteraction::Panning { .. });
        let y_dragging = matches!(state.y_axis_state, AxisInteraction::Panning { .. });
        let plot_dragging = matches!(state.scene_state.kind, InteractionKind::Panning { .. });

        // If user releases outside the original bounds, we still want to reset panning state
        let is_release = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        );

        if is_release {
            if let Some(a) =
                self.x_axis
                    .update(&mut state.x_axis_state, event, x_axis_bounds, cursor)
            {
                State::apply_action(shell, a);
            }

            if let Some(a) =
                self.y_axis
                    .update(&mut state.y_axis_state, event, y_axis_bounds, cursor)
            {
                State::apply_action(shell, a);
            }

            if let Some(a) = self
                .scene
                .update(&mut state.scene_state, event, plot_bounds, cursor)
            {
                State::apply_action(shell, a);
            }

            if let Some(a) =
                self.overlay
                    .update(&mut state.overlay_state, event, plot_bounds, cursor)
            {
                State::apply_action(shell, a);
            }

            return;
        }

        if (over_x || x_dragging)
            && let Some(a) =
                self.x_axis
                    .update(&mut state.x_axis_state, event, x_axis_bounds, cursor)
        {
            State::apply_action(shell, a);
        } else if (over_y || y_dragging)
            && let Some(a) =
                self.y_axis
                    .update(&mut state.y_axis_state, event, y_axis_bounds, cursor)
        {
            State::apply_action(shell, a);
        } else if over_plot || plot_dragging {
            if let Some(a) = self
                .scene
                .update(&mut state.scene_state, event, plot_bounds, cursor)
            {
                State::apply_action(shell, a);
            }

            if let Some(a) =
                self.overlay
                    .update(&mut state.overlay_state, event, plot_bounds, cursor)
            {
                State::apply_action(shell, a);
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let (plot_bounds, y_axis_bounds, x_axis_bounds, _corner) =
            State::bounds_from_layout(layout);
        if plot_bounds.width < 1.0 || plot_bounds.height < 1.0 {
            return;
        }

        let state = tree.state.downcast_ref::<State>();

        {
            use iced_wgpu::primitive::Renderer;

            // 1) Shader
            renderer.draw_primitive(
                plot_bounds,
                self.scene.draw(&state.scene_state, cursor, plot_bounds),
            );
        }

        {
            use iced::widget::canvas::Program;
            use iced_core::Renderer;
            use iced_wgpu::graphics::geometry::Renderer as _;

            // 2) Overlay canvas on top of shader (same bounds)
            renderer.with_layer(plot_bounds, |renderer| {
                renderer.with_translation(Vector::new(plot_bounds.x, plot_bounds.y), |renderer| {
                    for layer in self.overlay.draw(
                        &state.overlay_state,
                        renderer,
                        theme,
                        plot_bounds,
                        cursor,
                    ) {
                        renderer.draw_geometry(layer);
                    }
                });
            });

            // 3) Axes canvases
            renderer.with_translation(Vector::new(x_axis_bounds.x, x_axis_bounds.y), |renderer| {
                for layer in
                    self.x_axis
                        .draw(&state.x_axis_state, renderer, theme, x_axis_bounds, cursor)
                {
                    renderer.draw_geometry(layer);
                }
            });
            renderer.with_translation(Vector::new(y_axis_bounds.x, y_axis_bounds.y), |renderer| {
                for layer in
                    self.y_axis
                        .draw(&state.y_axis_state, renderer, theme, y_axis_bounds, cursor)
                {
                    renderer.draw_geometry(layer);
                }
            });

            // 4) Splitters
            let palette = theme.extended_palette();
            let splitter_color = palette.background.strong.color.scale_alpha(0.25);

            // Horizontal splitter between (plot+y_axis) and x_axis
            // spans plot width + y gutter width
            let total_top_row_w = (plot_bounds.width + y_axis_bounds.width).max(0.0);
            if total_top_row_w >= 1.0 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: plot_bounds.x,
                            y: plot_bounds.y + plot_bounds.height,
                            width: total_top_row_w,
                            height: 1.0,
                        },
                        snap: true,
                        ..Default::default()
                    },
                    splitter_color,
                );
            }

            // Vertical splitter between plot and y-axis
            if plot_bounds.height >= 1.0 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: plot_bounds.x + plot_bounds.width,
                            y: plot_bounds.y,
                            width: 1.0,
                            height: plot_bounds.height,
                        },
                        snap: true,
                        ..Default::default()
                    },
                    splitter_color,
                );
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        use iced::widget::canvas::Program as _;

        let (plot_bounds, y_axis_bounds, x_axis_bounds, _corner) =
            State::bounds_from_layout(layout);

        let state = tree.state.downcast_ref::<State>();

        let over_x = cursor.position_over(x_axis_bounds).is_some();
        let over_y = cursor.position_over(y_axis_bounds).is_some();

        let x_dragging = matches!(state.x_axis_state, AxisInteraction::Panning { .. });
        let y_dragging = matches!(state.y_axis_state, AxisInteraction::Panning { .. });

        if over_x || x_dragging {
            self.x_axis
                .mouse_interaction(&state.x_axis_state, x_axis_bounds, cursor)
        } else if over_y || y_dragging {
            self.y_axis
                .mouse_interaction(&state.y_axis_state, y_axis_bounds, cursor)
        } else {
            self.scene
                .mouse_interaction(&state.scene_state, plot_bounds, cursor)
        }
    }
}

impl<'a> From<HeatmapShaderWidget<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(widget: HeatmapShaderWidget<'a>) -> Self {
        Element::new(widget)
    }
}
