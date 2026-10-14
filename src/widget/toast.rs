use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Operation, Tree};
use iced::advanced::{Shell, Widget, shell};
use iced::time::{self, Duration, Instant};
use iced::widget::{button, column, container, row, space, text};
use iced::{
    Alignment, Center, Element, Event, Fill, Length, Rectangle, Renderer, Size, Theme, Vector,
};
use iced::{Border, mouse, padding, theme, window};

use crate::style;

const DEFAULT_TIMEOUT: u64 = 8;
const MAX_TOAST_BODY_HEIGHT: f32 = 120.0;

const MIN_VISIBLE_TOAST_HEIGHT: f32 = 40.0;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Status {
    #[default]
    Primary,
    Secondary,
    Success,
    Danger,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Toast {
    title: String,
    body: String,
    status: Status,
}

impl Toast {
    pub fn custom(title: impl Into<String>, body: impl Into<String>, status: Status) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            status,
        }
    }

    pub fn error(body: impl Into<String>) -> Self {
        Self {
            title: "Error".to_string(),
            body: body.into(),
            status: Status::Danger,
        }
    }

    pub fn warn(body: impl Into<String>) -> Self {
        Self {
            title: "Warning".to_string(),
            body: body.into(),
            status: Status::Warning,
        }
    }

    pub fn info(body: impl Into<String>) -> Self {
        Self {
            title: "Info".to_string(),
            body: body.into(),
            status: Status::Primary,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn status(&self) -> Status {
        self.status
    }
}

pub struct Manager<'a, Message> {
    content: Element<'a, Message>,
    toasts: Vec<Element<'a, Message>>,
    timeout_secs: u64,
    on_close: Box<dyn Fn(usize) -> Message + 'a>,
    alignment: Alignment,
}

impl<'a, Message> Manager<'a, Message>
where
    Message: 'a + Clone,
{
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        toasts: &'a [Toast],
        alignment: Alignment,
        on_close: impl Fn(usize) -> Message + 'a,
    ) -> Self {
        let toasts = toasts
            .iter()
            .enumerate()
            .map(|(index, toast)| {
                let header = container(
                    row![
                        text(toast.title.as_str()),
                        space::horizontal(),
                        button("X")
                            .on_press((on_close)(index))
                            .style(move |theme, status| {
                                style::button::transparent(theme, status, true)
                            })
                            .padding(padding::right(6).left(6).top(2).bottom(2))
                    ]
                    .align_y(Center),
                )
                .style(|theme| toast.status.style(theme))
                .width(Fill)
                .padding(4);

                let body = container(
                    text(toast.body.as_str())
                        .wrapping(iced::widget::text::Wrapping::Word)
                        .width(Fill),
                )
                .width(Fill)
                .height(MAX_TOAST_BODY_HEIGHT)
                .clip(true)
                .padding(4);

                container(column![header, body])
                    .style(style::chart_modal)
                    .padding(4)
                    .width(200)
                    .into()
            })
            .collect();

        Self {
            content: content.into(),
            alignment,
            toasts,
            timeout_secs: DEFAULT_TIMEOUT,
            on_close: Box::new(on_close),
        }
    }

    pub fn timeout(self, seconds: u64) -> Self {
        Self {
            timeout_secs: seconds,
            ..self
        }
    }
}

#[derive(Default)]
struct State {
    instants: Vec<Option<Instant>>,
    layout_cache: layout::flex::Cache,
}

impl<Message> Widget<Message, Theme, Renderer> for Manager<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        tree.children[0].translation = Vector::ZERO;
        tree.size = tree.children[0].size;
    }

    fn tag(&self) -> widget::tree::Tag {
        struct Marker;
        widget::tree::Tag::of::<Marker>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        let instants = &mut tree.state.downcast_mut::<State>().instants;

        // Invalidating removed instants to None allows us to remove
        // them here so that diffing for removed / new toast instants
        // is accurate
        instants.retain(Option::is_some);

        match (instants.len(), self.toasts.len()) {
            (old, new) if old > new => {
                instants.truncate(new);
            }
            (old, new) if old < new => {
                instants.extend(std::iter::repeat_n(Some(Instant::now()), new - old));
            }
            _ => {}
        }

        let mut children: Vec<_> = std::iter::once(&mut self.content)
            .chain(self.toasts.iter_mut())
            .collect();
        tree.diff_children(&mut children);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        operation.container(None, layout.bounds(), viewport);

        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            child_layout,
            viewport,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child_layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            child_layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        let state = tree.state.downcast_mut::<State>();
        let (content_state, toasts_state) = tree.children.split_at_mut(1);
        let mut overlays = self.content.as_widget_mut().overlay(
            &mut content_state[0],
            child_layout,
            renderer,
            viewport,
            translation,
            window,
        );

        if !self.toasts.is_empty() {
            let limits = layout::Limits::new(Size::ZERO, layout.size());
            let size = layout::flex::resolve(
                layout::flex::Axis::Vertical,
                renderer,
                &limits,
                Fill,
                Fill,
                32.into(),
                10.0,
                self.alignment,
                toasts_state,
                &mut self.toasts,
                &mut state.layout_cache,
            );
            let toast_layout = Layout::new(size).move_to(layout.position() + translation);
            overlays.push(overlay::Element::new(Box::new(Overlay {
                layout: toast_layout,
                viewport: *viewport,
                toasts: &mut self.toasts,
                state: toasts_state,
                instants: &mut state.instants,
                on_close: &self.on_close,
                timeout_secs: self.timeout_secs,
            })));
        }
        overlays
    }
}

struct Overlay<'a, 'b, Message> {
    layout: Layout,
    viewport: Rectangle,
    toasts: &'b mut [Element<'a, Message>],
    state: &'b mut [Tree],
    instants: &'b mut [Option<Instant>],
    on_close: &'b dyn Fn(usize) -> Message,
    timeout_secs: u64,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Overlay<'_, '_, Message> {
    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = &event {
            self.instants
                .iter_mut()
                .enumerate()
                .for_each(|(index, maybe_instant)| {
                    if let Some(instant) = maybe_instant.as_mut() {
                        let remaining =
                            time::seconds(self.timeout_secs).saturating_sub(instant.elapsed());

                        if remaining == Duration::ZERO {
                            maybe_instant.take();
                            shell.publish((self.on_close)(index));
                        } else {
                            shell.request_redraw_at(*now + remaining);
                        }
                    }
                });
        }

        let viewport = self.layout.bounds();

        for ((child, (child_layout, state)), instant) in self
            .toasts
            .iter_mut()
            .zip(self.layout.iter_mut(self.state))
            .zip(self.instants.iter_mut())
        {
            if !toast_body_visible(child_layout.bounds(), viewport) {
                continue;
            }

            let mut local_messages = shell::Bus::new();
            let mut local_shell = shell.local(&mut local_messages);

            child.as_widget_mut().update(
                state,
                event,
                child_layout,
                cursor,
                renderer,
                &mut local_shell,
                &viewport,
            );

            if !local_shell.is_empty() {
                instant.take();
            }

            shell.merge(local_shell, std::convert::identity);
        }
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        let viewport = self.layout.bounds();

        renderer.with_layer(viewport, |renderer| {
            for (child, (child_layout, state)) in
                self.toasts.iter().zip(self.layout.iter(self.state))
            {
                if !toast_body_visible(child_layout.bounds(), viewport) {
                    continue;
                }

                child.as_widget().draw(
                    state,
                    renderer,
                    theme,
                    style,
                    child_layout,
                    cursor,
                    &viewport,
                );
            }
        });
    }

    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        operation.container(None, self.layout.bounds(), &self.layout.bounds());

        let viewport = self.layout.bounds();

        for (child, (child_layout, state)) in
            self.toasts.iter_mut().zip(self.layout.iter_mut(self.state))
        {
            if !toast_body_visible(child_layout.bounds(), viewport) {
                continue;
            }

            child
                .as_widget_mut()
                .operate(state, child_layout, &viewport, renderer, operation);
        }
    }

    fn mouse_interaction(&self, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        let viewport = self.layout.bounds();

        self.toasts
            .iter()
            .zip(self.layout.iter(self.state))
            .filter_map(|(child, (child_layout, state))| {
                if !toast_body_visible(child_layout.bounds(), viewport) {
                    return None;
                }

                Some(
                    child
                        .as_widget()
                        .mouse_interaction(state, child_layout, cursor, &self.viewport, renderer)
                        .max(if cursor.is_over(child_layout.bounds()) {
                            mouse::Interaction::Idle
                        } else {
                            Default::default()
                        }),
                )
            })
            .max()
            .unwrap_or_default()
    }
}

impl<'a, Message> From<Manager<'a, Message>> for Element<'a, Message>
where
    Message: 'a,
{
    fn from(manager: Manager<'a, Message>) -> Self {
        Element::new(manager)
    }
}

fn toast_body_visible(child: Rectangle, viewport: Rectangle) -> bool {
    match child.intersection(&viewport) {
        Some(visible) => visible.height >= MIN_VISIBLE_TOAST_HEIGHT,
        None => false,
    }
}

fn styled(pair: theme::palette::Pair) -> container::Style {
    container::Style {
        background: Some(pair.color.into()),
        text_color: pair.text.into(),
        border: Border {
            width: 1.0,
            color: pair.color,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

impl Status {
    pub fn style(&self, theme: &Theme) -> container::Style {
        let palette = theme.palette();

        match self {
            Status::Primary => styled(palette.primary.weak),
            Status::Secondary => styled(palette.secondary.weak),
            Status::Success => styled(palette.success.weak),
            Status::Danger => styled(palette.danger.weak),
            Status::Warning => styled(palette.warning.weak),
        }
    }
}
