use iced::theme::{Custom, Palette};
use iced::widget::button::Status;
use iced::widget::container::{self, Style};
use iced::widget::pane_grid::{Highlight, Line};
use iced::widget::scrollable::{Rail, Scroller};
use iced::widget::{text, Text};
use iced::{widget, Border, Color, Font, Renderer, Shadow, Theme};

pub const ICON_BYTES: &[u8] = include_bytes!("fonts/icons.ttf");
pub const ICON_FONT: Font = Font::with_name("icons");

pub enum Icon {
    Locked,
    Unlocked,
    ResizeFull,
    ResizeSmall,
    Close,
    Layout,
    Cog,
    Link,
    BinanceLogo,
    BybitLogo,
    Search,
    Sort,
    SortDesc,
    SortAsc,
    Star,
    StarFilled,
    Return,
    Popout,
    ChartOutline,
    TrashBin,
    Edit,
    Checkmark,
    Clone,
}

impl From<Icon> for char {
    fn from(icon: Icon) -> Self {
        match icon {
            Icon::Locked => '\u{E800}',
            Icon::Unlocked => '\u{E801}',
            Icon::Search => '\u{E802}',
            Icon::ResizeFull => '\u{E803}',
            Icon::ResizeSmall => '\u{E804}',
            Icon::Close => '\u{E805}',
            Icon::Layout => '\u{E806}',
            Icon::Link => '\u{E807}',
            Icon::BybitLogo => '\u{E808}',
            Icon::BinanceLogo => '\u{E809}',
            Icon::Cog => '\u{E810}',
            Icon::Sort => '\u{F0DC}',
            Icon::SortDesc => '\u{F0DD}',
            Icon::SortAsc => '\u{F0DE}',
            Icon::Star => '\u{E80A}',
            Icon::StarFilled => '\u{E80B}',
            Icon::Return => '\u{E80C}',
            Icon::Popout => '\u{E80D}',
            Icon::ChartOutline => '\u{E80E}',
            Icon::TrashBin => '\u{E80F}',
            Icon::Edit => '\u{E811}',
            Icon::Checkmark => '\u{E812}',
            Icon::Clone => '\u{F0C5}',
        }
    }
}

pub fn get_icon_text<'a>(icon: Icon, size: u16) -> Text<'a, Theme, Renderer> {
    text(char::from(icon).to_string())
        .font(ICON_FONT)
        .size(iced::Pixels(size.into()))
}

pub fn custom_theme() -> Custom {
    Custom::new(
        "Flowsurface".to_string(),
        Palette {
            background: Color::from_rgb8(24, 22, 22),
            text: Color::from_rgb8(197, 201, 197),
            primary: Color::from_rgb8(200, 200, 200),
            success: Color::from_rgb8(81, 205, 160),
            danger: Color::from_rgb8(192, 80, 77),
            warning: Color::from_rgb8(238, 216, 139),
        },
    )
}

#[cfg(target_os = "macos")]
pub fn branding_text(theme: &Theme) -> iced::widget::text::Style {
    let palette = theme.extended_palette();

    iced::widget::text::Style {
        color: Some(
            palette
                .secondary
                .weak
                .color
                .scale_alpha(if palette.is_dark { 0.1 } else { 0.6 })
        ),
    }
}

// Tooltips
pub fn tooltip(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

// Buttons
pub fn button_confirm(
    theme: &Theme,
    status: Status,
    is_active: bool,
) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    let color_alpha = if palette.is_dark { 0.2 } else { 0.6 };

    match status {
        Status::Active => iced::widget::button::Style {
            background: None,
            text_color: palette.success.base.color,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            text_color: palette.success.weak.color,
            background: None,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Hovered => iced::widget::button::Style {
            background: None,
            text_color: palette.success.strong.color,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Disabled => iced::widget::button::Style {
            background: if is_active {
                None
            } else {
                Some(palette.success.weak.color.scale_alpha(color_alpha).into())
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

pub fn button_cancel(
    theme: &Theme,
    status: Status,
    is_active: bool,
) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    let color_alpha = if palette.is_dark { 0.2 } else { 0.6 };

    match status {
        Status::Active => iced::widget::button::Style {
            background: None,
            text_color: palette.danger.base.color,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            text_color: palette.danger.weak.color,
            background: None,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Hovered => iced::widget::button::Style {
            background: None,
            text_color: palette.danger.strong.color,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Disabled => iced::widget::button::Style {
            background: if is_active {
                None
            } else {
                Some(palette.danger.weak.color.scale_alpha(color_alpha).into())
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

pub fn button_layout_name(
    theme: &Theme,
    status: Status,
) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active => iced::widget::button::Style {
            background: None,
            text_color: palette.background.base.text,
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            background: None,
            text_color: palette.background.weak.text,
            ..Default::default()
        },
        Status::Hovered => iced::widget::button::Style {
            background: None,
            text_color: palette.background.strong.text,
            ..Default::default()
        },
        Status::Disabled => iced::widget::button::Style {
            background: None,
            text_color: palette.background.weakest.text,
            ..Default::default()
        },
    }
}

pub fn button_transparent(
    theme: &Theme,
    status: Status,
    is_active: bool,
) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active => iced::widget::button::Style {
            background: if is_active {
                Some(palette.background.weak.color.into())
            } else {
                None
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            background: Some(palette.background.strong.color.into()),
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Hovered => iced::widget::button::Style {
            background: Some(palette.background.strongest.color.into()),
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Disabled => iced::widget::button::Style {
            background: if is_active {
                Some(palette.background.weak.color.into())
            } else {
                Some(palette.background.strong.color.into())
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

pub fn button_modifier(
    theme: &Theme,
    status: Status,
    disabled: bool,
) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    match status {
        Status::Active => iced::widget::button::Style {
            background: if disabled {
                Some(palette.background.weak.color.into())
            } else {
                Some(palette.background.base.color.into())
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            background: Some(palette.background.strong.color.into()),
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Hovered => iced::widget::button::Style {
            background: Some(palette.background.strongest.color.into()),
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        Status::Disabled => iced::widget::button::Style {
            background: if disabled {
                None
            } else {
                Some(palette.secondary.weak.color.into())
            },
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

// Panes
pub fn pane_grid(theme: &Theme) -> widget::pane_grid::Style {
    let palette = theme.extended_palette();

    widget::pane_grid::Style {
        hovered_region: Highlight {
            background: palette.background.strong.color.into(),
            border: Border {
                width: 1.0,
                color: palette.primary.base.color,
                radius: 4.0.into(),
            },
        },
        picked_split: Line {
            color: palette.primary.strong.color,
            width: 4.0,
        },
        hovered_split: Line {
            color: palette.primary.weak.color,
            width: 4.0,
        },
    }
}

pub fn pane_title_bar(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: {
            if palette.is_dark {
                Some(palette.background.weak.color.scale_alpha(0.2).into())
            } else {
                Some(palette.background.strong.color.scale_alpha(0.2).into())
            }
        },
        ..Default::default()
    }
}

pub fn pane_background(theme: &Theme, is_focused: bool) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(
            palette
                .background
                .weakest
                .color
                .into(),
        ),
        border: {
            if is_focused {
                Border {
                    width: 1.0,
                    color: palette.background.strong.color,
                    radius: 4.0.into(),
                }
            } else {
                Border {
                    width: 1.0,
                    color: palette.background.base.color,
                    radius: 2.0.into(),
                }
            }
        },
        ..Default::default()
    }
}

// Modals
pub fn chart_modal(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(
            Color {
                a: 0.99,
                ..palette.background.base.color
            }
            .into(),
        ),
        border: Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 12.0,
            color: Color::BLACK.scale_alpha(
                if palette.is_dark {
                    0.4
                } else {
                    0.2
                }
            ),
        },
        ..Default::default()
    }
}

pub fn dashboard_modal(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: Some(
            Color {
                a: 0.99,
                ..palette.background.base.color
            }
            .into(),
        ),
        border: Border {
            width: 1.0,
            color: palette.secondary.weak.color,
            radius: 6.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 20.0,
            color: Color::BLACK.scale_alpha(
                if palette.is_dark {
                    0.4
                } else {
                    0.2
                }
            ),
        },
        ..Default::default()
    }
}

pub fn modal_container(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 2.0,
            color: Color::BLACK.scale_alpha(
                if palette.is_dark {
                    0.8
                } else {
                    0.2
                }
            ),
        },
    }
}

pub fn layout_row_container(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 2.0,
            color: Color::BLACK.scale_alpha(
                if palette.is_dark {
                    0.8
                } else {
                    0.2
                }
            ),
        },
    }
}

pub fn layout_card_bar(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: Some(palette.secondary.weak.color.into()),
        ..Default::default()
    }
}

// Time&Sales Table
pub fn ts_table_container(theme: &Theme, is_sell: bool, color_alpha: f32) -> Style {
    let palette = theme.extended_palette();

    let color = if is_sell {
        palette.danger.base.color
    } else {
        palette.success.base.color
    };

    Style {
        text_color: color.into(),
        border: Border {
            width: 1.0,
            color: color.scale_alpha(color_alpha),
            ..Border::default()
        },
        ..Default::default()
    }
}

// Tickers Table
pub fn search_input(
    theme: &Theme,
    status: widget::text_input::Status,
) -> widget::text_input::Style {
    let palette = theme.extended_palette();

    match status {
        widget::text_input::Status::Active => widget::text_input::Style {
            background: palette.background.weak.color.into(),
            border: Border {
                radius: 3.0.into(),
                width: 1.0,
                color: palette.secondary.base.color,
            },
            icon: palette.background.strong.text,
            placeholder: palette.background.base.text,
            value: palette.background.weak.text,
            selection: palette.background.strong.color,
        },
        widget::text_input::Status::Hovered => widget::text_input::Style {
            background: palette.background.weak.color.into(),
            border: Border {
                radius: 3.0.into(),
                width: 1.0,
                color: palette.secondary.strong.color,
            },
            icon: palette.background.strong.text,
            placeholder: palette.background.base.text,
            value: palette.background.weak.text,
            selection: palette.background.strong.color,
        },
        widget::text_input::Status::Focused { .. } => widget::text_input::Style {
            background: palette.background.weak.color.into(),
            border: Border {
                radius: 3.0.into(),
                width: 2.0,
                color: palette.secondary.strong.color,
            },
            icon: palette.background.strong.text,
            placeholder: palette.background.base.text,
            value: palette.background.weak.text,
            selection: palette.background.strong.color,
        },
        widget::text_input::Status::Disabled => widget::text_input::Style {
            background: palette.background.weak.color.into(),
            border: Border {
                radius: 3.0.into(),
                width: 1.0,
                color: palette.secondary.weak.color,
            },
            icon: palette.background.weak.text,
            placeholder: palette.background.weak.text,
            value: palette.background.weak.text,
            selection: palette.background.weak.text,
        },
    }
}

pub fn sorter_container(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 3.0.into(),
        },
        ..Default::default()
    }
}

pub fn ticker_card(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: Some(
            palette
                .background
                .weakest
                .color
                .into(),
        ),
        border: Border {
            radius: 4.0.into(),
            width: 1.0,
            color: palette.background.strong.color,
            ..Border::default()
        },
        ..Default::default()
    }
}

pub fn ticker_card_button(theme: &Theme, status: Status) -> iced::widget::button::Style {
    let palette = theme.extended_palette();

    match status {
        Status::Hovered => iced::widget::button::Style {
            text_color: palette.background.base.text,
            background: Some(palette.background.weak.color.into()),
            border: Border {
                radius: 4.0.into(),
                width: 1.0,
                color: palette.background.weak.color,
            },
            ..Default::default()
        },
        Status::Pressed => iced::widget::button::Style {
            text_color: palette.background.base.text,
            background: Some(palette.background.weakest.color.into()),
            border: Border {
                radius: 2.0.into(),
                width: 1.0,
                color: palette.background.weak.color,
            },
            ..Default::default()
        },
        _ => iced::widget::button::Style {
            text_color: palette.background.base.text,
            background: Some(palette.background.weakest.color.into()),
            border: Border {
                radius: 2.0.into(),
                width: 1.0,
                color: palette.background.weak.color,
            },
            ..Default::default()
        },
    }
}

// the bar that lights up depending on the price change 
pub fn ticker_card_bar(theme: &Theme, color_alpha: f32) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: {
            if color_alpha > 0.0 {
                Some(palette.success.strong.color.scale_alpha(color_alpha).into())
            } else {
                Some(palette.danger.strong.color.scale_alpha(-color_alpha).into())
            }
        },
        border: Border {
            radius: 4.0.into(),
            width: 1.0,
            color: if color_alpha > 0.0 {
                palette.success.strong.color.scale_alpha(color_alpha)
            } else {
                palette.danger.strong.color.scale_alpha(-color_alpha)
            },
        },
        ..Default::default()
    }
}

// Scrollable
pub fn scroll_bar(theme: &Theme, status: widget::scrollable::Status) -> widget::scrollable::Style {
    let palette = theme.extended_palette();

    let (rail_bg, scroller_bg) = match status {
        widget::scrollable::Status::Hovered { .. } 
        | widget::scrollable::Status::Dragged { .. } => {
            (
                palette.background.weakest.color,
                palette.background.weak.color,
            )
        },
        _ => (
            palette.background.base.color,
            palette.background.weakest.color,
        ),
    };

    let rail = Rail {
        background: Some(iced::Background::Color(rail_bg)),
        border: Border {
            radius: 2.0.into(),
            width: 1.0,
            color: Color::TRANSPARENT,
        },
        scroller: Scroller {
            color: scroller_bg,
            border: Border {
                radius: 2.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
        },
    };

    widget::scrollable::Style {
        container: container::Style {
            ..Default::default()
        },
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}

// custom widgets
pub fn split_ruler(theme: &Theme) -> iced::widget::rule::Style {
    let palette = theme.extended_palette();

    iced::widget::rule::Style {
        color: palette.background.strong.color.scale_alpha(0.4),
        width: 1,
        radius: iced::border::Radius::default(),
        fill_mode: iced::widget::rule::FillMode::Full,
    }
}