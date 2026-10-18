mod instance;
mod layout;
pub mod scene;
pub mod ui;
pub mod view;

pub use instance::InstanceBuilder;

use crate::widget::chart::heatmap::scene::{InteractionKind, Scene};
use crate::widget::chart::heatmap::ui::AxisInteraction;
use crate::widget::chart::heatmap::ui::axisx::AxisXLabelCanvas;
use crate::widget::chart::heatmap::ui::axisy::AxisYLabelCanvas;
use crate::widget::chart::heatmap::ui::overlay::OverlayCanvas;

use self::layout::HeatmapLayout;
use crate::widget::chart::PANEL_SPLITTER_HEIGHT;
use crate::window::PixelRatio;
use data::chart::heatmap::indicators::{HeatmapIndicators, HeatmapPanel};
use data::chart::kline::viewport::PanelYValueRange;
use enum_map::EnumMap;
use exchange::TickerInfo;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout as iced_layout, renderer};
use iced::widget::shader::Program;
use iced::widget::{canvas, shader};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};
use iced_core::mouse::{Click, click};
use iced_core::renderer::Quad;
use ui::indicator::IndicatorCanvas;

pub const DEFAULT_Y_AXIS_GUTTER: Length = Length::Fixed(66.0);
pub const DEFAULT_X_AXIS_HEIGHT: Length = Length::Fixed(24.0);

#[derive(Debug, Clone)]
pub enum Message {
    IndicatorYRangeChanged {
        kind: HeatmapPanel,
        range: Option<PanelYValueRange>,
    },
    PanelSplitChanged {
        index: usize,
        split: f32,
    },
    BoundsChanged(iced::Rectangle),
    PanDeltaPx(iced::Vector),
    ZoomAt {
        factor: f32,
        cursor: iced::Point,
    },
    ScrolledAxisY {
        factor: f32,
        cursor_y: f32,
        viewport_h: f32,
    },
    AxisYDoubleClicked,
    AxisXDoubleClicked,
    ScrolledAxisX {
        factor: f32,
        cursor_x: f32,
        viewport_w: f32,
    },
    DragZoomAxisXKeepAnchor {
        factor: f32,
        anchor_screen_x: f32,
        viewport_w: f32,
    },
    CursorMoved,
    PauseBtnClicked,
}

pub struct HeatmapShaderWidget<'a> {
    scene: &'a Scene,
    x_axis: AxisXLabelCanvas<'a>,
    y_axis: AxisYLabelCanvas<'a>,
    overlay: OverlayCanvas<'a>,
    y_axis_gutter: Length,
    x_axis_height: Length,
    indicators: &'a HeatmapIndicators,
    ticker_info: TickerInfo,
    indicator_panels: &'a EnumMap<HeatmapPanel, scene::indicator::Panel>,
    pixel_ratio: PixelRatio,
}

impl<'a> HeatmapShaderWidget<'a> {
    pub fn new(
        scene: &'a Scene,
        x_axis: AxisXLabelCanvas<'a>,
        y_axis: AxisYLabelCanvas<'a>,
        overlay: OverlayCanvas<'a>,
        indicators: &'a HeatmapIndicators,
        ticker_info: TickerInfo,
        indicator_panels: &'a EnumMap<HeatmapPanel, scene::indicator::Panel>,
        pixel_ratio: PixelRatio,
    ) -> Self {
        Self {
            scene,
            indicators,
            ticker_info,
            indicator_panels,
            pixel_ratio,
            x_axis,
            y_axis,
            overlay,
            y_axis_gutter: DEFAULT_Y_AXIS_GUTTER,
            x_axis_height: DEFAULT_X_AXIS_HEIGHT,
        }
    }

    pub fn with_y_axis_gutter(mut self, width: impl Into<Length>) -> Self {
        self.y_axis_gutter = width.into();
        self
    }

    pub fn with_x_axis_height(mut self, height: impl Into<Length>) -> Self {
        self.x_axis_height = height.into();
        self
    }
}

#[derive(Default)]
struct State {
    scene_state: <Scene as shader::Program<Message>>::State,

    x_axis_state: <AxisXLabelCanvas<'static> as canvas::Program<Message>>::State,
    pub y_axis_state: <AxisYLabelCanvas<'static> as canvas::Program<Message>>::State,
    overlay_state: <OverlayCanvas<'static> as canvas::Program<Message>>::State,

    // `Message::BoundsChanged` publishes when plot bounds change,
    // so `HeatmapShader` keeps working even if the current event is over an axis.
    last_plot_bounds: Option<Rectangle>,
    panel_drag: Option<PanelDrag>,
    indicator_axis_clicks: EnumMap<HeatmapPanel, Option<Click>>,
    // Used to publish one extra cursor event when exiting plot so cursor-dependent
    // labels/tooltips can be cleared without publishing every global mouse move.
    was_cursor_in_plot: bool,
}

enum PanelDrag {
    Splitter(usize),
    TimePan(iced::Point),
    IndicatorY {
        kind: HeatmapPanel,
        origin: iced::Point,
        initial_range: PanelYValueRange,
        bounds: Rectangle,
    },
}

impl State {
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
        limits: &iced_layout::Limits,
    ) -> iced_layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);

        let gutter_w = iced_layout::Limits::new(Size::ZERO, size)
            .width(self.y_axis_gutter)
            .resolve(self.y_axis_gutter, Length::Shrink, Size::ZERO)
            .width
            .min(size.width)
            .max(0.0);
        let x_axis_h = iced_layout::Limits::new(Size::ZERO, size)
            .height(self.x_axis_height)
            .resolve(Length::Shrink, self.x_axis_height, Size::ZERO)
            .height
            .min(size.height)
            .max(0.0);

        layout::build_node(size, gutter_w, x_axis_h, self.indicators.config())
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

        let geometry = HeatmapLayout::from_layout(layout, self.indicators.config());
        let plot_bounds = geometry.plot;
        let y_axis_bounds = geometry.y_axis;
        let x_axis_bounds = geometry.x_axis;

        let regions = &geometry.indicators;
        let stack_bounds = geometry.stack_bounds();
        self.x_axis.plot_bounds = Some(stack_bounds);
        let state = tree.state.downcast_mut::<State>();

        if Some(plot_bounds) != state.last_plot_bounds
            && plot_bounds.width > 0.0
            && plot_bounds.height > 0.0
        {
            state.last_plot_bounds = Some(plot_bounds);
            shell.publish(Message::BoundsChanged(plot_bounds));
        }

        let over_x = cursor.position_over(x_axis_bounds).is_some();
        let over_y = cursor.position_over(y_axis_bounds).is_some();
        let over_plot = cursor.position_over(plot_bounds).is_some();

        if matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. })) {
            let over_stack = cursor.position_over(stack_bounds).is_some();
            if over_stack || state.was_cursor_in_plot {
                shell.publish(Message::CursorMoved);
            }

            state.was_cursor_in_plot = over_stack;
        }

        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) {
            if state.panel_drag.take().is_some() {
                shell.capture_event();
            }
        } else {
            if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                match &mut state.panel_drag {
                    Some(PanelDrag::TimePan(last)) => {
                        shell.publish(Message::PanDeltaPx(Vector::new(position.x - last.x, 0.0)));
                        *last = *position;
                        shell.capture_event();
                        return;
                    }

                    Some(PanelDrag::Splitter(index)) => {
                        let usable = plot_bounds.height
                            + regions
                                .iter()
                                .map(|region| region.bounds.height)
                                .sum::<f32>();

                        if let Some(split) = super::split_ratio_from_cursor(
                            position.y - stack_bounds.y,
                            *index,
                            regions.len() + 1,
                            usable,
                            PANEL_SPLITTER_HEIGHT,
                        ) {
                            shell.publish(Message::PanelSplitChanged {
                                index: *index,
                                split,
                            });
                        }

                        shell.capture_event();
                        return;
                    }

                    Some(PanelDrag::IndicatorY {
                        kind,
                        origin,
                        initial_range,
                        bounds,
                    }) => {
                        let delta = (origin.y - position.y).round() * 0.1;

                        if let Some(range) = self.indicator_panels[*kind].zoom_range(
                            *initial_range,
                            0.5,
                            delta,
                            bounds.height,
                            self.pixel_ratio,
                            kind.value_scale(self.indicators.config(), self.ticker_info),
                        ) {
                            shell.publish(Message::IndicatorYRangeChanged {
                                kind: *kind,
                                range: Some(range),
                            });
                        }

                        shell.capture_event();
                        return;
                    }
                    None => {}
                }
            }

            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            ) {
                if let Some(region) = geometry.indicator_axis_at(cursor)
                    && let Some(position) = cursor.position()
                {
                    let kind = region.kind;
                    let bounds = region.bounds;
                    let click = Click::new(
                        position,
                        mouse::Button::Left,
                        state.indicator_axis_clicks[kind],
                    );
                    state.indicator_axis_clicks[kind] = Some(click);

                    if click.kind() == click::Kind::Double {
                        state.panel_drag = None;
                        shell.publish(Message::IndicatorYRangeChanged { kind, range: None });
                    } else if let Some(initial_range) = self.indicator_panels[kind].range(
                        kind.value_scale(self.indicators.config(), self.ticker_info),
                        bounds.height,
                        self.pixel_ratio,
                    ) {
                        state.panel_drag = Some(PanelDrag::IndicatorY {
                            kind,
                            origin: position,
                            initial_range,
                            bounds,
                        });
                    }

                    shell.capture_event();
                    return;
                }

                if let Some(index) = geometry.splitter_at(cursor) {
                    state.panel_drag = Some(PanelDrag::Splitter(index));
                    shell.capture_event();
                    return;
                }

                if geometry.indicator_plot_at(cursor).is_some() {
                    state.panel_drag = cursor.position().map(PanelDrag::TimePan);
                    shell.capture_event();
                    return;
                }
            }

            if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
                && let Some(region) = geometry.indicator_axis_at(cursor)
                && let Some(position) = cursor.position()
            {
                let kind = region.kind;
                let bounds = region.bounds;
                let delta = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                }
                .clamp(-6.0, 6.0);
                let panel = &self.indicator_panels[kind];

                if delta.abs() > f32::EPSILON
                    && let Some(initial) = panel.range(
                        kind.value_scale(self.indicators.config(), self.ticker_info),
                        bounds.height,
                        self.pixel_ratio,
                    )
                    && let Some(range) = panel.zoom_range(
                        initial,
                        f64::from(1.0 - (position.y - bounds.y) / bounds.height.max(1.0)),
                        delta,
                        bounds.height,
                        self.pixel_ratio,
                        kind.value_scale(self.indicators.config(), self.ticker_info),
                    )
                {
                    shell.publish(Message::IndicatorYRangeChanged {
                        kind,
                        range: Some(range),
                    });
                }

                shell.capture_event();
                return;
            }

            if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
                && geometry.indicator_plot_at(cursor).is_some()
                && let Some(position) = cursor.position()
            {
                let amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.1,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.01,
                };

                shell.publish(Message::ScrolledAxisX {
                    factor: (1.0 + amount).clamp(0.01, 100.0),
                    cursor_x: position.x - plot_bounds.x,
                    viewport_w: plot_bounds.width,
                });
                shell.capture_event();
                return;
            }
        }

        let x_dragging = matches!(
            state.x_axis_state.interaction,
            AxisInteraction::Panning { .. }
        );
        let y_dragging = matches!(
            state.y_axis_state.interaction,
            AxisInteraction::Panning { .. }
        );
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
        let geometry = HeatmapLayout::from_layout(layout, self.indicators.config());
        let plot_bounds = geometry.plot;
        let y_axis_bounds = geometry.y_axis;
        let x_axis_bounds = geometry.x_axis;
        if plot_bounds.width < 1.0 || plot_bounds.height < 1.0 {
            return;
        }

        let stack_bounds = geometry.stack_bounds();
        let x_axis = AxisXLabelCanvas {
            plot_bounds: Some(stack_bounds),
            ..self.x_axis
        };
        let cursor_time = cursor
            .position_in(stack_bounds)
            .map(|position| x_axis.time.time_at_x(position.x, plot_bounds.width));
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
                    x_axis.draw(&state.x_axis_state, renderer, theme, x_axis_bounds, cursor)
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

            for region in &geometry.indicators {
                let kind = region.kind;
                let bounds = region.bounds;
                let panel = &self.indicator_panels[kind];
                let plot = region.plot_bounds(plot_bounds.width);

                let (primitive, range) = panel.prepare(
                    self.indicators,
                    kind,
                    x_axis.time,
                    plot,
                    self.ticker_info,
                    theme,
                    self.pixel_ratio,
                );

                {
                    use iced_wgpu::primitive::Renderer;
                    renderer.draw_primitive(plot, primitive);
                }

                let canvas = IndicatorCanvas {
                    indicators: self.indicators,
                    kind,
                    time: x_axis.time,
                    plot_width: plot_bounds.width,
                    cursor_time,
                    range,
                    ticker_info: self.ticker_info,
                };
                renderer.with_layer(bounds, |renderer| {
                    renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                        for geometry in canvas.draw(&(), renderer, theme, bounds, cursor) {
                            renderer.draw_geometry(geometry);
                        }
                    });
                });
            }

            if let Some(time) = cursor_time {
                let x = (x_axis.time.screen_x(time, plot_bounds.width).round() + 0.5)
                    .clamp(0.0, plot_bounds.width);
                let regions = &geometry.indicators;
                let primary = (cursor.position_over(plot_bounds).is_none() && !regions.is_empty())
                    .then_some(plot_bounds);

                for bounds in primary
                    .into_iter()
                    .chain(regions.iter().map(|region| region.bounds))
                {
                    let mut frame = canvas::Frame::new(renderer, bounds.size());
                    frame.with_clip(
                        Rectangle::with_size(Size::new(plot_bounds.width, bounds.height)),
                        |frame| {
                            frame.stroke(
                                &canvas::Path::line(
                                    iced::Point::new(x, 0.0),
                                    iced::Point::new(x, bounds.height),
                                ),
                                crate::style::chart_crosshair(theme),
                            );
                        },
                    );

                    renderer.with_layer(bounds, |renderer| {
                        renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                            renderer.draw_geometry(frame.into_geometry());
                        });
                    });
                }
            }

            let splitter_style = crate::style::split_ruler(theme);
            for region in &geometry.indicators {
                renderer.fill_quad(
                    Quad {
                        bounds: region.splitter,
                        snap: splitter_style.snap,
                        ..Default::default()
                    },
                    splitter_style.color,
                );
            }

            // 4) Splitters
            let splitter_color = splitter_style.color;

            // Divider above the shared time axis.
            let total_top_row_w = (plot_bounds.width + y_axis_bounds.width).max(0.0);
            if total_top_row_w >= 1.0 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: plot_bounds.x,
                            y: x_axis_bounds.y,
                            width: total_top_row_w,
                            height: 1.0,
                        },
                        snap: true,
                        ..Default::default()
                    },
                    splitter_color,
                );
            }

            if stack_bounds.height >= 1.0 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: plot_bounds.x + plot_bounds.width,
                            y: plot_bounds.y,
                            width: 1.0,
                            height: stack_bounds.height,
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

        let geometry = HeatmapLayout::from_layout(layout, self.indicators.config());
        let plot_bounds = geometry.plot;
        let y_axis_bounds = geometry.y_axis;
        let x_axis_bounds = geometry.x_axis;

        let state = tree.state.downcast_ref::<State>();

        if matches!(state.panel_drag, Some(PanelDrag::Splitter(_)))
            || geometry.splitter_at(cursor).is_some()
        {
            return mouse::Interaction::ResizingVertically;
        }
        if matches!(state.panel_drag, Some(PanelDrag::IndicatorY { .. }))
            || geometry.indicator_axis_at(cursor).is_some()
        {
            return mouse::Interaction::ResizingVertically;
        }
        if matches!(state.panel_drag, Some(PanelDrag::TimePan(_))) {
            return mouse::Interaction::Grabbing;
        }
        if geometry.indicator_plot_at(cursor).is_some() {
            return mouse::Interaction::Crosshair;
        }
        let over_x = cursor.position_over(x_axis_bounds).is_some();
        let over_y = cursor.position_over(y_axis_bounds).is_some();

        let x_dragging = matches!(
            state.x_axis_state.interaction,
            AxisInteraction::Panning { .. }
        );
        let y_dragging = matches!(
            state.y_axis_state.interaction,
            AxisInteraction::Panning { .. }
        );

        if over_x || x_dragging {
            self.x_axis
                .mouse_interaction(&state.x_axis_state, x_axis_bounds, cursor)
        } else if over_y || y_dragging {
            self.y_axis
                .mouse_interaction(&state.y_axis_state, y_axis_bounds, cursor)
        } else {
            self.overlay
                .mouse_interaction(&state.overlay_state, plot_bounds, cursor)
        }
    }
}

impl<'a> From<HeatmapShaderWidget<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(widget: HeatmapShaderWidget<'a>) -> Self {
        Element::new(widget)
    }
}
