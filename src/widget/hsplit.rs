// Modification of `VSplit` widget of [generic-daw]
// https://github.com/generic-daw/generic-daw/blob/main/generic_daw_gui/src/widget/vsplit.rs
//
// credits to authors of https://github.com/generic-daw/generic-daw/
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        overlay,
        renderer::Style,
        widget::{Operation, Tree, tree},
    },
    mouse::{self, Cursor, Interaction},
    widget::Rule,
};
use std::fmt::{Debug, Formatter};

use crate::style;

const DRAG_SIZE: f32 = 1.0;

#[derive(Default)]
struct State {
    dragging: bool,
    hovering: bool,
}

pub struct HSplit<'a, Message> {
    children: [Element<'a, Message>; 3],
    split_at: f32,
    resize: fn(f32) -> Message,
}

impl<Message> Debug for HSplit<'_, Message> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HSplit")
            .field("split_at", &self.split_at)
            .finish_non_exhaustive()
    }
}

impl<'a, Message> HSplit<'a, Message>
where
    Message: 'a,
{
    pub fn new(
        top: impl Into<Element<'a, Message>>,
        bottom: impl Into<Element<'a, Message>>,
        split_at: f32,
        resize: fn(f32) -> Message,
    ) -> Self {
        Self {
            children: [
                top.into(),
                Rule::horizontal(DRAG_SIZE).style(style::split_ruler).into(),
                bottom.into(),
            ],
            split_at,
            resize,
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for HSplit<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        let max_limits = limits.max();

        let top_height = max_limits
            .height
            .mul_add(self.split_at, -(DRAG_SIZE * 0.5))
            .floor();
        let top_limits = Limits::new(Size::new(0.0, 0.0), Size::new(max_limits.width, top_height));

        let bottom_height = max_limits.height - top_height - DRAG_SIZE;
        let bottom_limits = Limits::new(
            Size::new(0.0, 0.0),
            Size::new(max_limits.width, bottom_height),
        );

        let children = vec![
            self.children[0]
                .as_widget()
                .layout(&mut tree.children[0], renderer, &top_limits),
            self.children[1]
                .as_widget()
                .layout(&mut tree.children[1], renderer, limits)
                .translate(Vector::new(0.0, top_height)),
            self.children[2]
                .as_widget()
                .layout(&mut tree.children[2], renderer, &bottom_limits)
                .translate(Vector::new(0.0, top_height + DRAG_SIZE)),
        ];

        Node::with_children(max_limits, children)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .for_each(|((child, tree), layout)| {
                child.as_widget_mut().update(
                    tree, event, layout, cursor, renderer, clipboard, shell, viewport,
                );
            });

        if shell.is_event_captured() {
            return;
        }

        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        let dragger_bounds = match layout.children().nth(1) {
            Some(dragger) => dragger.bounds().expand(4.0),
            None => {
                log::error!("Failed to find dragger bounds in HSplit layout");
                return;
            }
        };

        if let Event::Mouse(event) = event {
            match event {
                mouse::Event::ButtonPressed(iced::mouse::Button::Left) => {
                    if cursor.is_over(dragger_bounds) {
                        state.dragging = true;
                        shell.capture_event();
                    }
                }
                mouse::Event::CursorMoved {
                    position: Point { y, .. },
                    ..
                } => {
                    if state.dragging {
                        let split_at = ((y - bounds.y) / bounds.height).clamp(0.0, 1.0);
                        shell.publish((self.resize)(split_at));
                        shell.capture_event();
                    } else if state.hovering != cursor.is_over(dragger_bounds) {
                        state.hovering ^= true;
                        shell.request_redraw();
                    }
                }
                mouse::Event::ButtonReleased(mouse::Button::Left) if state.dragging => {
                    state.dragging = false;
                    shell.capture_event();
                }
                _ => {}
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .filter(|(_, layout)| layout.bounds().intersects(viewport))
            .for_each(|((child, tree), layout)| {
                child
                    .as_widget()
                    .draw(tree, renderer, theme, style, layout, cursor, viewport);
            });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> Interaction {
        let state = tree.state.downcast_ref::<State>();
        if state.dragging || state.hovering {
            Interaction::ResizingVertically
        } else {
            self.children
                .iter()
                .zip(&tree.children)
                .zip(layout.children())
                .map(|((child, tree), layout)| {
                    child
                        .as_widget()
                        .mouse_interaction(tree, layout, cursor, viewport, renderer)
                })
                .max()
                .unwrap_or_default()
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(&mut self.children, tree, layout, renderer, translation)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds(), &mut |operation| {
            self.children
                .iter()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .as_widget()
                        .operate(state, layout, renderer, operation);
                });
        });
    }
}

impl<'a, Message> From<HSplit<'a, Message>> for Element<'a, Message>
where
    Message: 'a,
{
    fn from(widget: HSplit<'a, Message>) -> Self {
        Self::new(widget)
    }
}
