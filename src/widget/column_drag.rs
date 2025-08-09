// Modification of `Column` widget of [`dragking`]
// credits to https://github.com/airstrike/dragking/

// Removed `DropPosition` enum and "ghost drawing": drop target position is decided by the closest index
// Added limits so picked column only moves vertically, and clamping logic so it doesn't go outside bounds

//! Distribute draggable content vertically.
// This widget is a modification of the original `Column` widget from [`iced`]
//
// [`iced`]: https://github.com/iced-rs/iced
//
// Copyright 2019 Héctor Ramón, Iced contributors
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::alignment::{self, Alignment};
use iced::event::Event;
use iced::mouse;
use iced::{Element, Length, Padding, Pixels, Point, Rectangle, Size, Theme, Vector};

const DRAG_HANDLE_WIDTH: f32 = 14.0;

pub fn reorder_vec<T>(vec: &mut Vec<T>, event: &DragEvent) {
    if let DragEvent::Dropped {
        index,
        target_index,
    } = event
        && vec.len() > 1
        && target_index != index
    {
        let item = vec.remove(*index);
        let insert_index = if index < target_index {
            *target_index - 1
        } else {
            *target_index
        };
        vec.insert(insert_index, item);
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Idle,
    Picking {
        index: usize,
        origin: Point,
    },
    Dragging {
        index: usize,
        origin: Point,
        last_cursor: Point,
    },
}

#[derive(Debug, Clone)]
pub enum DragEvent {
    Picked { index: usize },
    Dropped { index: usize, target_index: usize },
    Canceled { index: usize },
}

#[allow(missing_debug_implementations)]
pub struct Column<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    spacing: f32,
    padding: Padding,
    width: Length,
    height: Length,
    max_width: f32,
    align: Alignment,
    clip: bool,
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    on_drag: Option<Box<dyn Fn(DragEvent) -> Message + 'a>>,
}

impl<'a, Message, Theme, Renderer> Column<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Creates an empty [`Column`].
    pub fn new() -> Self {
        Self::from_vec(Vec::new())
    }

    /// Creates a [`Column`] with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::from_vec(Vec::with_capacity(capacity))
    }

    /// Creates a [`Column`] with the given elements.
    pub fn with_children(
        children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        let iterator = children.into_iter();

        Self::with_capacity(iterator.size_hint().0).extend(iterator)
    }

    /// Creates a [`Column`] from an already allocated [`Vec`].
    ///
    /// Keep in mind that the [`Column`] will not inspect the [`Vec`], which means
    /// it won't automatically adapt to the sizing strategy of its contents.
    ///
    /// If any of the children have a [`Length::Fill`] strategy, you will need to
    /// call [`Column::width`] or [`Column::height`] accordingly.
    pub fn from_vec(children: Vec<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            spacing: 0.0,
            padding: Padding::ZERO,
            width: Length::Shrink,
            height: Length::Shrink,
            max_width: f32::INFINITY,
            align: Alignment::Start,
            clip: false,
            children,
            on_drag: None,
        }
    }

    /// Sets the vertical spacing _between_ elements.
    ///
    /// Custom margins per element do not exist in iced. You should use this
    /// method instead! While less flexible, it helps you keep spacing between
    /// elements consistent.
    pub fn spacing(mut self, amount: impl Into<Pixels>) -> Self {
        self.spacing = amount.into().0;
        self
    }

    /// Sets the [`Padding`] of the [`Column`].
    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the width of the [`Column`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Column`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the maximum width of the [`Column`].
    pub fn max_width(mut self, max_width: impl Into<Pixels>) -> Self {
        self.max_width = max_width.into().0;
        self
    }

    /// Sets the horizontal alignment of the contents of the [`Column`] .
    pub fn align_x(mut self, align: impl Into<alignment::Horizontal>) -> Self {
        self.align = Alignment::from(align.into());
        self
    }

    /// Sets whether the contents of the [`Column`] should be clipped on
    /// overflow.
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// Adds an element to the [`Column`].
    pub fn push(mut self, child: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        let child = child.into();
        let child_size = child.as_widget().size_hint();

        self.width = self.width.enclose(child_size.width);
        self.height = self.height.enclose(child_size.height);

        self.children.push(child);
        self
    }

    /// Adds an element to the [`Column`], if `Some`.
    pub fn push_maybe(
        self,
        child: Option<impl Into<Element<'a, Message, Theme, Renderer>>>,
    ) -> Self {
        if let Some(child) = child {
            self.push(child)
        } else {
            self
        }
    }

    /// Extends the [`Column`] with the given children.
    pub fn extend(
        self,
        children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        children.into_iter().fold(self, Self::push)
    }

    /// The message produced by the [`Column`] when a child is dragged.
    pub fn on_drag(mut self, on_reorder: impl Fn(DragEvent) -> Message + 'a) -> Self {
        self.on_drag = Some(Box::new(on_reorder));
        self
    }

    fn compute_target_index(
        &self,
        cursor_position: Point,
        layout: Layout<'_>,
        dragged_index: usize,
    ) -> usize {
        let cursor_y = cursor_position.y;
        let bounds = layout.bounds();

        // Check if cursor is at the top edge
        if cursor_y <= bounds.y {
            return 0;
        }

        // Check if cursor is at the bottom edge
        if cursor_y >= bounds.y + bounds.height {
            return self.children.len();
        }

        for (i, child_layout) in layout.children().enumerate() {
            let child_bounds = child_layout.bounds();
            let y = child_bounds.y;
            let height = child_bounds.height;
            let middle = y + height / 2.0;

            if cursor_y >= y && cursor_y <= y + height {
                // Skip the dragged item itself
                if i == dragged_index {
                    continue;
                }

                // Insert before or after based on which half of the item we're over
                if cursor_y < middle {
                    return i;
                } else {
                    return i + 1;
                }
            } else if cursor_y < y {
                // Cursor is above this child
                return i;
            }
        }

        // Cursor is below all children
        self.children.len()
    }
}

impl<Message, Renderer> Default for Column<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Message, Theme, Renderer: renderer::Renderer>
    FromIterator<Element<'a, Message, Theme, Renderer>> for Column<'a, Message, Theme, Renderer>
{
    fn from_iter<T: IntoIterator<Item = Element<'a, Message, Theme, Renderer>>>(iter: T) -> Self {
        Self::with_children(iter)
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Column<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Action>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Action::Idle)
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.max_width(self.max_width);

        layout::flex::resolve(
            layout::flex::Axis::Vertical,
            renderer,
            &limits,
            self.width,
            self.height,
            self.padding,
            self.spacing,
            self.align,
            &self.children,
            &mut tree.children,
        )
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

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let action = tree.state.downcast_mut::<Action>();

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let bounds = layout.bounds();

                if let Some(cursor_position) = cursor.position_over(bounds) {
                    let drag_handle_bounds = Rectangle {
                        x: bounds.x,
                        y: bounds.y,
                        width: DRAG_HANDLE_WIDTH,
                        height: bounds.height,
                    };

                    for (index, child_layout) in layout.children().enumerate() {
                        if cursor.is_over(drag_handle_bounds)
                            && child_layout.bounds().contains(cursor_position)
                        {
                            *action = Action::Picking {
                                index,
                                origin: cursor_position,
                            };
                            break;
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                match *action {
                    Action::Picking { index, origin } => {
                        if let Some(cursor_position) = cursor.position() {
                            *action = Action::Dragging {
                                index,
                                origin,
                                last_cursor: cursor_position,
                            };
                            if let Some(on_reorder) = &self.on_drag {
                                shell.publish(on_reorder(DragEvent::Picked { index }));
                            }
                        }
                    }
                    Action::Dragging { origin, index, .. } => {
                        if let Some(cursor_position) = cursor.position() {
                            let bounds = layout.bounds();
                            // clamp y so the ghost never goes outside the column
                            let clamped_y =
                                cursor_position.y.clamp(bounds.y, bounds.y + bounds.height);
                            let clamped_cursor = Point {
                                x: cursor_position.x,
                                y: clamped_y,
                            };

                            *action = Action::Dragging {
                                last_cursor: clamped_cursor,
                                origin,
                                index,
                            };

                            shell.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                match *action {
                    Action::Dragging {
                        index, last_cursor, ..
                    } => {
                        let target_index = self.compute_target_index(last_cursor, layout, index);

                        if let Some(on_reorder) = &self.on_drag {
                            shell.publish(on_reorder(DragEvent::Dropped {
                                index,
                                target_index,
                            }));
                        }
                        *action = Action::Idle;
                    }
                    Action::Picking { .. } => {
                        // Did not move enough to start dragging
                        *action = Action::Idle;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .for_each(|((child, tree), layout)| {
                child.as_widget_mut().update(
                    tree, event, layout, cursor, renderer, clipboard, shell, viewport,
                );
            });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let action = tree.state.downcast_ref::<Action>();

        match action {
            Action::Dragging { .. } | Action::Picking { .. } => {
                return mouse::Interaction::Grabbing;
            }
            _ => {
                let bounds = layout.bounds();
                let drag_handle_bounds = Rectangle {
                    x: bounds.x,
                    y: bounds.y,
                    width: DRAG_HANDLE_WIDTH,
                    height: bounds.height,
                };

                if cursor.position_in(drag_handle_bounds).is_some() {
                    return mouse::Interaction::Grab;
                }
            }
        }

        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let action = tree.state.downcast_ref::<Action>();

        match action {
            Action::Dragging {
                index,
                last_cursor,
                origin,
                ..
            } => {
                let child_count = self.children.len();
                // Always use last_cursor (which is clamped) instead of checking actual cursor position
                let target_index = self
                    .compute_target_index(*last_cursor, layout, *index)
                    .min(child_count);

                let drag_bounds = layout.children().nth(*index).unwrap().bounds();
                let drag_height = drag_bounds.height + self.spacing;

                // Draw all children with appropriate translations
                for i in 0..child_count {
                    let child = &self.children[i];
                    let state = &tree.children[i];
                    let child_layout = layout.children().nth(i).unwrap();

                    if i == *index {
                        // Draw the dragged item at cursor position
                        let translation_y = last_cursor.y - origin.y;
                        renderer.with_translation(
                            Vector {
                                x: 0.0,
                                y: translation_y,
                            },
                            |renderer| {
                                renderer.with_layer(child_layout.bounds(), |renderer| {
                                    child.as_widget().draw(
                                        state,
                                        renderer,
                                        theme,
                                        defaults,
                                        child_layout,
                                        cursor,
                                        viewport,
                                    );
                                });
                            },
                        );
                    } else {
                        // Calculate offset for non-dragged items
                        let offset: i32 = match target_index.cmp(index) {
                            std::cmp::Ordering::Less if i >= target_index && i < *index => 1,
                            std::cmp::Ordering::Greater if i > *index && i < target_index => -1,
                            _ => 0,
                        };

                        let translation = Vector::new(0.0, offset as f32 * drag_height);
                        renderer.with_translation(translation, |renderer| {
                            child.as_widget().draw(
                                state,
                                renderer,
                                theme,
                                defaults,
                                child_layout,
                                cursor,
                                viewport,
                            );
                        });
                    }
                }
            }
            _ => {
                // Draw all children normally when not dragging
                for ((child, state), layout) in self
                    .children
                    .iter()
                    .zip(&tree.children)
                    .zip(layout.children())
                {
                    child
                        .as_widget()
                        .draw(state, renderer, theme, defaults, layout, cursor, viewport);
                }
            }
        }
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Column<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(column: Column<'a, Message, Theme, Renderer>) -> Self {
        Self::new(column)
    }
}
