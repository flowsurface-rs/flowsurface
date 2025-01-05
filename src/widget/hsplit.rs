use iced::{
    advanced::{
        layout::{Limits, Node},
        renderer::Style,
        widget::{tree, Tree},
        Clipboard, Layout, Shell, Widget,
    }, border::Radius, mouse::{Cursor, Interaction}, widget::Rule, Element, Length, Rectangle, Renderer, Size, Theme, Vector
};
use std::fmt::{Debug, Formatter};

const DRAG_SIZE: f32 = 4.0;

struct State {
    split_at: f32,
    dragging: bool,
    offset: f32,
}

pub struct HSplit<'a, Message, Theme, Renderer> {
    children: [Element<'a, Message, Theme, Renderer>; 3],
    starting_split_at: f32,
}

impl<Message, Theme, Renderer> Debug for HSplit<'_, Message, Theme, Renderer> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HSplit").finish_non_exhaustive()
    }
}

impl<'a, Message> HSplit<'a, Message, Theme, Renderer>
where
    Message: 'a,
{
    pub fn new(
        top: impl Into<Element<'a, Message, Theme, Renderer>>,
        bottom: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            children: [
                top.into(), 
                Rule::horizontal(DRAG_SIZE)
                    .style(move |theme: &Theme| iced::widget::rule::Style {
                        color: {
                            let palette = theme.extended_palette();

                            if palette.is_dark {
                                palette.background.weak.color.scale_alpha(0.2)
                            } else {
                                palette.background.strong.color.scale_alpha(0.2)
                            }
                        },
                        width: 1,
                        radius: Radius::default(),
                        fill_mode: iced::widget::rule::FillMode::Full,
                    })
                    .into(),
                bottom.into()
            ],
            starting_split_at: 0.8,
        }
    }

    pub fn split(mut self, split_at: f32) -> Self {
        self.starting_split_at = split_at;
        self
    }

    fn new_state(&self) -> State {
        State {
            split_at: self.starting_split_at,
            dragging: false,
            offset: 0.0,
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for HSplit<'_, Message, Theme, Renderer> {
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
        tree::State::new(self.new_state())
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        let state = tree.state.downcast_ref::<State>();
        let max_limits = limits.max();

        let top_height = max_limits.height.mul_add(state.split_at, -(DRAG_SIZE * 0.5));
        let top_limits = Limits::new(
            Size::new(0.0, 0.0),
            Size::new(max_limits.width, top_height),
        );

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
                .layout(&mut tree.children[1], renderer, &Limits::new(Size::new(DRAG_SIZE, DRAG_SIZE), Size::new(max_limits.width, DRAG_SIZE)))
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
        event: iced::Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        if let iced::Event::Mouse(event) = event {
            match event {
                iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left) => {
                    if let Some(position) =
                        cursor.position_in(layout.children().nth(1).unwrap().bounds())
                    {
                        state.offset = DRAG_SIZE.mul_add(-0.5, position.y);
                        state.dragging = true;
                    }
                }
                iced::mouse::Event::CursorMoved { .. } if state.dragging => {
                    if let Some(position) = cursor.position() {
                        state.split_at = (DRAG_SIZE
                            .mul_add(-0.5, position.y - bounds.position().y - state.offset)
                            / (bounds.height - DRAG_SIZE))
                            .clamp(0.0, 1.0);
                    } else {
                        state.dragging = false;
                    }
                }
                iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left) if state.dragging => {
                    state.dragging = false;
                }
                _ => {}
            }

            shell.request_redraw();
        }
        
        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .for_each(|((child, tree), layout)| {
                child.as_widget_mut().update(
                    tree,
                    event.clone(),
                    layout,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                )
            });

        if state.dragging {
            shell.invalidate_layout();
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
        if state.dragging
            || cursor
                .position_in(layout.children().nth(1).unwrap().bounds())
                .is_some()
        {
            Interaction::ResizingVertically
        } else {
            self.children
                .iter()
                .zip(&tree.children)
                .zip(layout.children())
                .find(|(_, layout)| cursor.position_in(layout.bounds()).is_some())
                .map_or_else(Interaction::default, |((child, tree), layout)| {
                    child
                        .as_widget()
                        .mouse_interaction(tree, layout, cursor, viewport, renderer)
                })
        }
    }
}

impl<'a, Message> From<HSplit<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
{
    fn from(vsplit: HSplit<'a, Message, Theme, Renderer>) -> Self {
        Self::new(vsplit)
    }
}