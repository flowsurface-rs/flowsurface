use super::Element;
use crate::{
    style::{self, Icon, icon_text},
    theme::ThemeAccents,
};
use data::config::theme::{
    ACCENT_COUNT, ThemeColorRole, ThemeColorShade, darken, lighten, mix_color,
};
use data::{chart::kline::ThemeAccent, config::theme::ThemeColor};
use iced::{
    Alignment, Color,
    Length::{self, Fill},
    Theme, padding,
    widget::{
        button, column, container, row, scrollable, space, text as iced_text, tooltip::Position,
    },
};

pub mod chart;
pub mod color_picker;
pub mod column_drag;
pub mod decorate;
pub mod inspector;
pub mod multi_split;
pub mod series_editor;
pub mod study_configurator;
pub mod toast;

pub const PANE_CONTROL_BTN_HEIGHT: f32 = 26.0;

#[allow(dead_code)]
pub const DEFAULT_TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Clone, Copy, Default)]
pub enum BannerKind {
    #[default]
    Normal,
    Warning,
    Error,
}

pub fn banner<'a, Message: 'a>(
    message: impl iced::widget::text::IntoFragment<'a>,
    kind: BannerKind,
) -> Element<'a, Message> {
    container(
        row![
            container(iced_text("!").size(style::text_size::SECTION)).padding([0, 8]),
            iced_text(message)
                .wrapping(iced::widget::text::Wrapping::Word)
                .size(style::text_size::SMALL)
                .width(Fill),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(6)
    .width(Fill)
    .style(move |theme: &Theme| {
        let palette = theme.palette();
        iced::widget::container::Style {
            text_color: Some(match kind {
                BannerKind::Normal => palette.text,
                BannerKind::Warning => palette.warning,
                BannerKind::Error => palette.danger,
            }),
            ..style::modal_container(theme)
        }
    })
    .into()
}

pub fn tooltip<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip: Option<&'a str>,
    position: Position,
) -> Element<'a, Message> {
    tooltip_with_delay(content, tooltip, position, std::time::Duration::ZERO)
}

pub fn tooltip_with_delay<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip: Option<&'a str>,
    position: Position,
    delay: std::time::Duration,
) -> Element<'a, Message> {
    match tooltip {
        Some(tooltip) => iced::widget::tooltip(
            content,
            container(iced_text(tooltip))
                .style(style::tooltip)
                .padding(8),
            position,
        )
        .delay(delay)
        .into(),
        None => content.into(),
    }
}

pub mod text {
    use iced::{Renderer, Theme};

    pub fn title<'a>(
        content: impl iced::widget::text::IntoFragment<'a>,
    ) -> iced::widget::Text<'a, Theme, Renderer> {
        iced::widget::text(content)
            .size(crate::style::text_size::TITLE)
            .style(crate::style::text_color::title)
    }

    pub fn section<'a>(
        content: impl iced::widget::text::IntoFragment<'a>,
    ) -> iced::widget::Text<'a, Theme, Renderer> {
        iced::widget::text(content)
            .size(crate::style::text_size::SECTION)
            .style(crate::style::text_color::section)
    }

    pub fn emphasis<'a>(
        content: impl iced::widget::text::IntoFragment<'a>,
    ) -> iced::widget::Text<'a, Theme, Renderer> {
        iced::widget::text(content)
            .size(crate::style::text_size::EMPHASIS)
            .style(crate::style::text_color::body)
    }

    pub fn body<'a>(
        content: impl iced::widget::text::IntoFragment<'a>,
    ) -> iced::widget::Text<'a, Theme, Renderer> {
        iced::widget::text(content)
            .size(crate::style::text_size::BODY)
            .style(crate::style::text_color::body)
    }

    pub fn muted<'a>(
        content: impl iced::widget::text::IntoFragment<'a>,
    ) -> iced::widget::Text<'a, Theme, Renderer> {
        iced::widget::text(content)
            .size(crate::style::text_size::SMALL)
            .style(crate::style::text_color::muted)
    }
}

pub fn scrollable_content<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    scrollable::Scrollable::with_direction(
        content,
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)),
    )
    .into()
}

pub fn classic_slider_row<'a, T, Message>(
    label: Option<iced::widget::Text<'a>>,
    range: std::ops::RangeInclusive<T>,
    current: T,
    on_change: impl Fn(T) -> Message + 'a,
    format_value: impl Fn(&T) -> String,
    step: Option<T>,
) -> Element<'a, Message>
where
    T: 'static + Copy + PartialOrd + Into<f64> + From<u8> + num_traits::FromPrimitive,
    Message: Clone + 'a,
{
    let mut slider = iced::widget::slider(range, current, on_change)
        .width(Fill)
        .style(style::inspector_slider);

    if let Some(step) = step {
        slider = slider.step(step);
    }

    let slider = column![
        slider,
        iced_text(format_value(&current)).size(style::text_size::EMPHASIS),
    ]
    .width(Length::Fill)
    .spacing(2)
    .align_x(Alignment::Center);
    let content = match label {
        Some(label) => row![
            container(
                label
                    .style(iced::widget::text::default)
                    .wrapping(iced::widget::text::Wrapping::Word),
            )
            .width(Length::Shrink)
            .max_width(80.0),
            slider,
        ]
        .spacing(8),
        None => row![slider],
    };

    container(content.align_y(Alignment::Center).padding(8))
        .style(style::inspector_slider_container)
        .into()
}

pub fn button_with_tooltip<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    message: M,
    tooltip_text: Option<&'a str>,
    tooltip_pos: crate::TooltipPosition,
    style_fn: impl Fn(&Theme, button::Status) -> button::Style + 'static,
) -> Element<'a, M> {
    let btn = button(content).style(style_fn).on_press(message);

    if let Some(text) = tooltip_text {
        tooltip(btn, Some(text), tooltip_pos)
    } else {
        btn.into()
    }
}

/// Creates a tooltip button that dispatches on mouse-down.
///
/// Use this when the action must run before a parent widget handles the click.
pub fn press_button_with_tooltip<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    message: M,
    tooltip_text: Option<&'a str>,
    tooltip_pos: crate::TooltipPosition,
    style_fn: impl Fn(&Theme, button::Status) -> button::Style + 'static,
) -> Element<'a, M> {
    let button = button(content).style(style_fn).on_press(message.clone());
    let press_area =
        iced::widget::mouse_area(container(space::horizontal()).width(Fill).height(Fill))
            .on_press(message);

    tooltip(
        iced::widget::stack([button.into(), press_area.into()]),
        tooltip_text,
        tooltip_pos,
    )
}

pub fn dragger_row<'a, Message>(
    content: Element<'a, Message>,
    is_enabled: bool,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let content = if is_enabled {
        let icon = icon_text(Icon::DragHandle, 11);
        row![icon, content,]
            .align_y(Alignment::Center)
            .spacing(2)
            .into()
    } else {
        content
    };

    container(content)
        .padding(2)
        .style(style::dragger_row_container)
        .into()
}

pub fn theme_color_control<'a, Message>(
    selected: ThemeColor,
    theme_accents: ThemeAccents,
    on_change: impl Fn(Option<ThemeAccent>) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    let color_at_shade = move |theme: &Theme, accent: Option<ThemeAccent>| {
        let palette = theme.extended_palette();
        resolve_theme_color(
            ThemeColor::with_accent(selected.role, accent, selected.shade),
            palette,
            theme_accents.as_array(),
        )
    };

    let default_color = color_swatch_button(
        move |theme: &Theme| color_at_shade(theme, None),
        selected.accent.is_none(),
        on_change.clone()(None),
    );
    let color_choices =
        std::iter::once(default_color).chain(ThemeAccent::ALL.into_iter().map(|accent| {
            color_swatch_button(
                move |theme: &Theme| color_at_shade(theme, Some(accent)),
                selected.accent == Some(accent),
                on_change.clone()(Some(accent)),
            )
        }));

    row(color_choices).spacing(2).into()
}

pub fn resolve_theme_color(
    color: ThemeColor,
    palette: &iced::theme::palette::Extended,
    generated_accents: &[Color; ACCENT_COUNT],
) -> Color {
    let base = color.accent.map_or_else(
        || match color.role {
            ThemeColorRole::Neutral => palette.background.base.text,
            ThemeColorRole::Secondary => palette.secondary.strong.color,
            ThemeColorRole::Primary => palette.primary.base.color,
            ThemeColorRole::Success => palette.success.base.color,
            ThemeColorRole::Danger => palette.danger.base.color,
            ThemeColorRole::Warning => palette.warning.base.color,
        },
        |accent| generated_accents[accent.index()],
    );
    let background = palette.background.base.color;

    match color.shade {
        ThemeColorShade::Faint => mix_color(base, background, 0.3),
        ThemeColorShade::Soft => mix_color(base, background, 0.65),
        ThemeColorShade::Base => base,
        ThemeColorShade::Strong => {
            if palette.is_dark {
                lighten(base, 0.1)
            } else {
                darken(base, 0.1)
            }
        }
        ThemeColorShade::Vivid => {
            if palette.is_dark {
                lighten(base, 0.18)
            } else {
                darken(base, 0.18)
            }
        }
    }
}

pub fn color_swatch_button<'a, Message, ColorResolver>(
    resolve_color: ColorResolver,
    selected: bool,
    message: Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
    ColorResolver: Fn(&Theme) -> Color + Copy + 'static,
{
    let swatch = container("")
        .width(16)
        .height(16)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let color = resolve_color(theme);
            iced::widget::container::Style {
                background: Some(color.into()),
                border: iced::Border {
                    width: 2.0,
                    color: palette.background.weak.color,
                    radius: 3.0.into(),
                },
                shadow: if selected {
                    iced::Shadow {
                        color,
                        offset: iced::Vector { x: 0.0, y: 0.0 },
                        blur_radius: 2.0,
                    }
                } else {
                    iced::Shadow::default()
                },
                ..Default::default()
            }
        });

    button(swatch)
        .style(|theme, status| style::button::transparent(theme, status, false))
        .padding(2)
        .on_press(message)
        .into()
}

pub fn numeric_input_box<'a, F, Message: Clone + 'static>(
    label: &'a str,
    placeholder: &str,
    raw_input_buf: &str,
    is_input_valid: bool,
    on_input_changed: F,
    on_submit_maybe: Option<Message>,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + 'static,
{
    let text_input_widget = iced::widget::text_input(placeholder, raw_input_buf)
        .on_input(on_input_changed)
        .on_submit_maybe(on_submit_maybe)
        .align_x(iced::Alignment::Center)
        .style(move |theme, status| style::validated_text_input(theme, status, is_input_valid));

    row![iced_text(label), text_input_widget]
        .padding(padding::right(20).left(20))
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
}

pub fn link_group_button<'a, Message, F>(
    id: iced::widget::pane_grid::Pane,
    link_group: Option<data::layout::pane::LinkGroup>,
    on_press: F,
) -> Element<'a, Message>
where
    Message: Clone + 'static,
    F: Fn(iced::widget::pane_grid::Pane) -> Message + 'static,
{
    let is_active = link_group.is_some();

    let icon = if let Some(group) = link_group {
        iced_text(group.to_string())
    } else {
        iced_text("-")
    };

    button(
        icon.font(style::AZERET_MONO)
            .align_x(Alignment::Start)
            .align_y(Alignment::Center),
    )
    .style(move |theme: &Theme, status| style::button::bordered_toggle(theme, status, is_active))
    .on_press(on_press(id))
    .height(PANE_CONTROL_BTN_HEIGHT)
    .width(28)
    .into()
}

#[macro_export]
/// Creates a column with horizontal rules between each item.
///
/// # Examples
/// ```
/// split_column![
///     text("Item 1"),
///     text("Item 2"),
///     text("Item 3"),
/// ] ; spacing = 8, align_x = Alignment::Start
/// ```
///
macro_rules! split_column {
    () => {
        column![]
    };

    ($item:expr $(,)?) => {
        column![$item]
    };

    ($first:expr, $($rest:expr),+ $(,)?) => {{
        let mut col = column![$first];
        $(
            col = col.push(iced::widget::rule::horizontal(1.0).style($crate::style::split_ruler));
            col = col.push($rest);
        )+
        col
    }};

    ($($item:expr),* $(,)?; spacing = $spacing:expr) => {{
        $crate::split_column![$($item),*].spacing($spacing)
    }};

    ($($item:expr),* $(,)?; spacing = $spacing:expr, align_x = $align:expr) => {{
        $crate::split_column![$($item),*].spacing($spacing).align_x($align)
    }};
}
