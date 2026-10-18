use iced::font::{Family, Stretch, Weight};
use iced::theme::palette::Extended;
use iced::widget::Text;
use iced::widget::canvas::{LineDash, Stroke};
use iced::widget::container::{self, Style};
use iced::widget::pane_grid::{Highlight, Line};
use iced::widget::scrollable::{AutoScroll, Rail, Scroller};
use iced::{Border, Color, Font, Renderer, Shadow, Theme, widget};

pub const ICONS_BYTES: &[u8] = include_bytes!(".././assets/fonts/icons.ttf");
pub const ICONS_FONT: Font = Font::with_name("icons");

pub const AZERET_MONO_BYTES: &[u8] = include_bytes!("../assets/fonts/AzeretMono-Regular.ttf");
pub const AZERET_MONO: Font = Font {
    family: Family::Name("Azeret Mono"),
    weight: Weight::Normal,
    stretch: Stretch::Normal,
    style: iced::font::Style::Normal,
};

pub const TITLE_PADDING_TOP: f32 = if cfg!(target_os = "macos") { 20.0 } else { 0.0 };

pub mod text_size {
    pub const SMALL: f32 = 11.0;
    pub const BODY: f32 = 12.0;
    pub const SECTION: f32 = 14.0;
    pub const TITLE: f32 = 16.0;

    pub const TINY: f32 = SMALL - 1.0;
    pub const EMPHASIS: f32 = BODY + 1.0;
}

pub mod text_color {
    use iced::widget::text::Style;
    use iced::{Color, Theme};

    fn from_color(color: Color) -> Style {
        Style { color: Some(color) }
    }

    pub fn title(theme: &Theme) -> Style {
        from_color(theme.extended_palette().background.strong.text)
    }

    pub fn section(theme: &Theme) -> Style {
        from_color(theme.extended_palette().background.base.text)
    }

    pub fn body(theme: &Theme) -> Style {
        from_color(theme.extended_palette().background.weak.text)
    }

    pub fn muted(theme: &Theme) -> Style {
        from_color(theme.extended_palette().background.weaker.text)
    }

    pub fn secondary(theme: &Theme) -> Style {
        from_color(theme.extended_palette().secondary.strong.color)
    }
}

/// Icon glyph code points for the `icons` font (`assets/fonts/icons.ttf`).
/// Values represent each glyph's `code` in `assets/fonts/fontello.json`.
/// Enum variant names may differ from Fontello CSS names.
#[repr(u32)]
pub enum Icon {
    Locked = 59392,
    Unlocked = 59393,
    ResizeFull = 59395,
    ResizeSmall = 59396,
    Close = 59397,
    Layout = 59398,
    Cog = 59408,
    Link = 59399,
    BinanceLogo = 59401,
    BybitLogo = 59400,
    HyperliquidLogo = 59411,
    OkexLogo = 59423,
    MexcLogo = 59425,
    Search = 59394,
    Sort = 61660,
    SortDesc = 61661,
    SortAsc = 61662,
    Star = 59402,
    StarFilled = 59403,
    Return = 59404,
    Popout = 59405,
    ChartOutline = 59406,
    TrashBin = 59407,
    Edit = 59409,
    Checkmark = 59410,
    Clone = 61637,
    SpeakerOff = 59412,
    SpeakerLow = 59414,
    SpeakerHigh = 59413,
    DragHandle = 59415,
    Folder = 61716,
    ExternalLink = 61772,
    Refresh = 59416,
    Crosshair = 59426,
    Trendline = 59427,
    DrawingBox = 59428,
    HorizontalLine = 59429,
    VerticalLine = 59430,
    Ruler = 59431,
    Magnet = 59432,
    Layers = 59433,
    ChevronUp = 59434,
    ChevronDown = 59435,
    ChevronRight = 59436,
    Eye = 59437,
    EyeOff = 59438,
}

impl Icon {
    pub fn visibility(visible: bool) -> Self {
        if visible { Self::Eye } else { Self::EyeOff }
    }
}

impl From<Icon> for char {
    fn from(icon: Icon) -> Self {
        char::from_u32(icon as u32).expect("icon codepoints must be valid Unicode scalar values")
    }
}

pub fn icon_text<'a>(icon: Icon, size: u16) -> Text<'a, Theme, Renderer> {
    iced::widget::text(char::from(icon).to_string())
        .font(ICONS_FONT)
        .size(iced::Pixels(size.into()))
}

pub fn venue_icon(venue: exchange::adapter::Venue) -> Icon {
    match venue {
        exchange::adapter::Venue::Bybit => Icon::BybitLogo,
        exchange::adapter::Venue::Binance => Icon::BinanceLogo,
        exchange::adapter::Venue::Hyperliquid => Icon::HyperliquidLogo,
        exchange::adapter::Venue::Okex => Icon::OkexLogo,
        exchange::adapter::Venue::Mexc => Icon::MexcLogo,
    }
}

pub fn secondary_text(theme: &Theme) -> iced::widget::text::Style {
    let palette = theme.extended_palette();

    iced::widget::text::Style {
        color: Some(palette.secondary.weak.color),
    }
}

#[cfg(target_os = "macos")]
pub fn title_text(theme: &Theme) -> iced::widget::text::Style {
    let palette = theme.extended_palette();

    iced::widget::text::Style {
        color: Some(palette.background.weakest.color),
    }
}

pub fn tooltip(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.base.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.strong.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 8.0,
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.2 }),
        },
        ..Default::default()
    }
}

pub mod button {
    use iced::{
        Border, Theme,
        widget::button::{Status, Style},
    };

    pub fn confirm(theme: &Theme, status: Status, is_active: bool) -> Style {
        let palette = theme.extended_palette();

        let color_alpha = if palette.is_dark { 0.2 } else { 0.6 };

        Style {
            text_color: match status {
                Status::Active => palette.success.base.color,
                Status::Pressed => palette.success.weak.color,
                Status::Hovered => palette.success.strong.color,
                Status::Disabled => palette.background.base.text,
            },
            background: match (status, is_active) {
                (Status::Disabled, false) => {
                    Some(palette.success.weak.color.scale_alpha(color_alpha).into())
                }
                _ => None,
            },
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn cancel(theme: &Theme, status: Status, is_active: bool) -> Style {
        let palette = theme.extended_palette();

        let color_alpha = if palette.is_dark { 0.2 } else { 0.6 };

        Style {
            text_color: match status {
                Status::Active => palette.danger.base.color,
                Status::Pressed => palette.danger.weak.color,
                Status::Hovered => palette.danger.strong.color,
                Status::Disabled => palette.background.base.text,
            },
            background: match (status, is_active) {
                (Status::Disabled, false) => {
                    Some(palette.danger.weak.color.scale_alpha(color_alpha).into())
                }
                _ => None,
            },
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn layout_name(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();

        let bg_color = match status {
            Status::Pressed => Some(palette.background.weak.color.into()),
            Status::Hovered => Some(palette.background.strong.color.into()),
            Status::Disabled | Status::Active => None,
        };

        Style {
            background: bg_color,
            text_color: palette.background.base.text,
            border: Border {
                radius: 4.0.into(),
                width: 1.0,
                color: iced::Color::TRANSPARENT,
            },
            ..Default::default()
        }
    }

    pub fn transparent(theme: &Theme, status: Status, is_clicked: bool) -> Style {
        let palette = theme.extended_palette();

        Style {
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            background: match status {
                Status::Active => {
                    if is_clicked {
                        Some(palette.background.weak.color.into())
                    } else {
                        None
                    }
                }
                Status::Pressed => Some(palette.background.weak.color.into()),
                Status::Hovered => Some(palette.background.strong.color.into()),
                Status::Disabled => {
                    if is_clicked {
                        Some(palette.background.strongest.color.into())
                    } else {
                        Some(palette.background.strong.color.into())
                    }
                }
            },
            ..Default::default()
        }
    }

    pub fn magnet(theme: &Theme, status: Status, enabled: bool, active: bool) -> Style {
        let palette = theme.extended_palette();
        let enabled_color = palette.secondary.strong.color;

        Style {
            text_color: if enabled && !active {
                enabled_color
            } else {
                palette.background.base.text
            },
            border: Border {
                radius: 3.0.into(),
                width: if enabled && !active { 1.0 } else { 0.0 },
                color: enabled_color.scale_alpha(0.75),
            },
            background: match status {
                Status::Active => {
                    if active {
                        Some(palette.background.weak.color.into())
                    } else if enabled {
                        Some(enabled_color.scale_alpha(0.14).into())
                    } else {
                        None
                    }
                }
                Status::Pressed => Some(palette.background.weak.color.into()),
                Status::Hovered => Some(palette.background.strong.color.into()),
                Status::Disabled => Some(palette.background.strong.color.into()),
            },
            ..Default::default()
        }
    }

    pub fn modifier(theme: &Theme, status: Status, is_clicked: bool) -> Style {
        let palette = theme.extended_palette();

        Style {
            text_color: if status == Status::Disabled {
                palette.secondary.strong.color
            } else {
                palette.background.base.text
            },
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            background: match status {
                Status::Active => {
                    if is_clicked {
                        Some(palette.background.weak.color.into())
                    } else {
                        Some(palette.background.base.color.into())
                    }
                }
                Status::Pressed => Some(palette.background.strongest.color.into()),
                Status::Hovered => Some(palette.background.strong.color.into()),
                Status::Disabled => {
                    if is_clicked {
                        None
                    } else {
                        Some(palette.background.weakest.color.into())
                    }
                }
            },
            ..Default::default()
        }
    }

    pub fn bordered_toggle(theme: &Theme, status: Status, is_active: bool) -> Style {
        let palette = theme.extended_palette();

        iced::widget::button::Style {
            text_color: {
                if status == Status::Disabled {
                    palette.background.stronger.color
                } else if is_active {
                    palette.secondary.strong.color
                } else {
                    palette.secondary.base.color
                }
            },
            border: iced::Border {
                radius: 3.0.into(),
                width: if is_active { 2.0 } else { 1.0 },
                color: palette.background.weak.color,
            },
            background: match status {
                iced::widget::button::Status::Active => {
                    if is_active {
                        Some(palette.background.base.color.into())
                    } else {
                        Some(palette.background.weakest.color.into())
                    }
                }
                iced::widget::button::Status::Pressed => {
                    Some(palette.background.weakest.color.into())
                }
                iced::widget::button::Status::Hovered => Some(palette.background.weak.color.into()),
                iced::widget::button::Status::Disabled => {
                    Some(palette.background.weakest.color.into())
                }
            },
            ..Default::default()
        }
    }

    pub fn indicator_row(theme: &Theme, status: Status, is_active: bool) -> Style {
        let palette = theme.extended_palette();

        iced::widget::button::Style {
            text_color: {
                if status == Status::Disabled {
                    palette.background.strongest.color
                } else if is_active {
                    palette.background.strongest.text
                } else {
                    palette.background.base.text
                }
            },
            border: iced::Border {
                radius: if is_active { 4.0.into() } else { 2.0.into() },
                width: if is_active { 2.0 } else { 1.0 },
                color: if is_active {
                    palette.background.strong.color
                } else {
                    palette.background.weak.color
                },
            },
            background: match status {
                iced::widget::button::Status::Active => {
                    if is_active {
                        Some(palette.background.weakest.color.into())
                    } else {
                        Some(palette.background.base.color.into())
                    }
                }
                iced::widget::button::Status::Pressed => {
                    Some(palette.background.weakest.color.into())
                }
                iced::widget::button::Status::Hovered => Some(palette.background.weak.color.into()),
                iced::widget::button::Status::Disabled => {
                    Some(palette.background.weakest.color.into())
                }
            },
            ..Default::default()
        }
    }

    pub fn info(theme: &Theme, _status: Status) -> Style {
        let palette = theme.extended_palette();

        Style {
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            background: Some(palette.background.weakest.color.into()),
            ..Default::default()
        }
    }

    pub fn tooltip_indicator(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();
        let accent = palette.secondary.base.color;

        Style {
            text_color: accent,
            border: Border {
                width: 1.0,
                color: accent.scale_alpha(0.75),
                radius: 3.0.into(),
            },
            background: match status {
                Status::Active => Some(accent.scale_alpha(0.1).into()),
                Status::Pressed => Some(accent.scale_alpha(0.3).into()),
                Status::Hovered => Some(accent.scale_alpha(0.2).into()),
                Status::Disabled => Some(palette.background.weakest.color.into()),
            },
            ..Default::default()
        }
    }

    pub fn text_link(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();

        let text_color = match status {
            Status::Active => palette.secondary.base.color,
            Status::Pressed => palette.secondary.base.color,
            Status::Hovered => palette.secondary.strong.color,
            Status::Disabled => palette.background.strong.color,
        };

        Style {
            text_color,
            background: None,
            border: Border {
                radius: 0.0.into(),
                width: 0.0,
                color: iced::Color::TRANSPARENT,
            },
            ..Default::default()
        }
    }

    pub fn text_link_secondary(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();

        let text_color = match status {
            Status::Active => palette.secondary.weak.color,
            Status::Pressed => palette.secondary.weak.color,
            Status::Hovered => palette.secondary.base.color,
            Status::Disabled => palette.background.strong.color,
        };

        Style {
            text_color,
            background: None,
            border: Border {
                radius: 0.0.into(),
                width: 0.0,
                color: iced::Color::TRANSPARENT,
            },
            ..Default::default()
        }
    }

    pub fn menu_body(theme: &Theme, status: Status, is_selected: bool) -> Style {
        let palette = theme.extended_palette();

        Style {
            text_color: palette.background.base.text,
            border: Border {
                radius: 3.0.into(),
                width: if is_selected { 2.0 } else { 0.0 },
                color: palette.background.strong.color,
            },
            background: match status {
                Status::Active => {
                    if is_selected {
                        Some(palette.background.base.color.into())
                    } else {
                        Some(palette.background.weakest.color.into())
                    }
                }
                Status::Pressed => Some(palette.background.base.color.into()),
                Status::Hovered => Some(palette.background.weak.color.into()),
                Status::Disabled => {
                    if is_selected {
                        None
                    } else {
                        Some(palette.secondary.base.color.into())
                    }
                }
            },
            ..Default::default()
        }
    }

    pub fn ticker_card(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();

        let color = if palette.is_dark {
            palette.background.weak.color
        } else {
            palette.background.strong.color
        };

        match status {
            Status::Hovered => Style {
                text_color: palette.background.base.text,
                background: Some(palette.background.weak.color.into()),
                border: Border {
                    width: 1.0,
                    radius: 2.0.into(),
                    color,
                },
                ..Default::default()
            },
            _ => Style {
                background: Some(color.scale_alpha(0.4).into()),
                text_color: palette.background.base.text,
                border: Border {
                    width: 1.0,
                    radius: 2.0.into(),
                    color: color.scale_alpha(0.8),
                },
                ..Default::default()
            },
        }
    }
}

// Panes
pub fn pane_grid(theme: &Theme) -> widget::pane_grid::Style {
    let palette = theme.extended_palette();

    widget::pane_grid::Style {
        hovered_region: Highlight {
            background: palette.background.strongest.color.scale_alpha(0.5).into(),
            border: Border {
                width: 1.0,
                color: palette.background.strongest.color,
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

    let color = if palette.is_dark {
        palette.background.weak.color
    } else {
        palette.background.strong.color
    };

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.weakest.color.into()),
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
                    color: color.scale_alpha(0.5),
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
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.4 } else { 0.2 }),
        },
        snap: true,
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
            color: palette.background.weak.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 20.0,
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.4 }),
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
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.2 }),
        },
        snap: true,
    }
}

pub fn inspector_section(theme: &Theme) -> Style {
    let palette = theme.extended_palette();
    let mut style = modal_container(theme);
    style.background = Some(palette.background.weaker.color.into());
    style.text_color = Some(palette.background.weaker.text);
    style
}

pub fn inspector_control(theme: &Theme) -> Style {
    let palette = theme.extended_palette();
    let mut style = modal_container(theme);
    style.background = Some(palette.background.strong.color.into());
    style.text_color = Some(palette.background.strong.text);
    style.border.color = palette.background.stronger.color;
    style
}

pub fn inspector_slider_container(theme: &Theme) -> Style {
    let surface = theme.extended_palette().background.strongest;
    let mut style = inspector_control(theme);
    style.background = Some(surface.color.into());
    style.text_color = Some(surface.text);
    style
}

pub fn inspector_slider(theme: &Theme, status: widget::slider::Status) -> widget::slider::Style {
    let palette = theme.extended_palette();

    let mut style = widget::slider::default(theme, status);
    style.rail.backgrounds.1 = palette.background.base.color.into();
    style
}

pub fn chart_sidebar_container(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: {
            if palette.is_dark {
                Some(palette.background.weak.color.scale_alpha(0.2).into())
            } else {
                Some(palette.background.strong.color.scale_alpha(0.2).into())
            }
        },
        snap: true,
        ..Default::default()
    }
}

pub fn chart_toolbar(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(palette.background.base.color.into()),
        border: Border {
            width: 1.0,
            color: palette.background.strong.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 8.0,
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.2 }),
        },
        snap: true,
    }
}

pub fn chart_info_tooltip(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        text_color: Some(palette.primary.weak.text),
        background: Some(palette.primary.weak.color.into()),
        border: Border {
            width: 1.0,
            color: palette.primary.strong.color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 8.0,
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.2 }),
        },
        snap: true,
    }
}

pub fn dragger_row_container(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    let bg_color = palette.background.strong.color;

    Style {
        text_color: Some(palette.background.base.text),
        background: Some(bg_color.into()),
        border: Border {
            width: 1.0,
            color: bg_color,
            radius: 4.0.into(),
        },
        shadow: Shadow {
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 4.0,
            color: Color::BLACK.scale_alpha(if palette.is_dark { 0.8 } else { 0.2 }),
        },
        snap: true,
    }
}

// Tickers Table
pub fn validated_text_input(
    theme: &Theme,
    status: widget::text_input::Status,
    is_valid: bool,
) -> widget::text_input::Style {
    let palette = theme.extended_palette();

    let (background, border_color, placeholder) = match status {
        widget::text_input::Status::Active => (
            palette.background.weakest.color,
            palette.background.weak.color,
            palette.background.strongest.color,
        ),
        widget::text_input::Status::Hovered => (
            palette.background.weak.color,
            palette.background.strong.color,
            palette.background.weak.text,
        ),
        widget::text_input::Status::Focused { .. } | widget::text_input::Status::Disabled => (
            palette.background.base.color,
            palette.background.strong.color,
            palette.background.strong.color,
        ),
    };

    widget::text_input::Style {
        background: background.into(),
        border: Border {
            radius: 3.0.into(),
            width: 1.0,
            color: if is_valid {
                border_color
            } else {
                palette.danger.base.color
            },
        },
        icon: palette.background.strong.text,
        placeholder,
        value: palette.background.base.text,
        selection: palette.background.strongest.color,
    }
}

pub fn ticker_card(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: {
            if palette.is_dark {
                Some(palette.background.weak.color.scale_alpha(0.4).into())
            } else {
                Some(palette.background.strong.color.scale_alpha(0.4).into())
            }
        },
        border: Border {
            radius: 4.0.into(),
            width: 1.0,
            color: palette.background.strong.color,
        },
        ..Default::default()
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
        widget::scrollable::Status::Hovered { .. } | widget::scrollable::Status::Dragged { .. } => {
            (
                palette.background.weakest.color,
                palette.background.weak.color,
            )
        }
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
            background: iced::Background::Color(scroller_bg),
            border: Border {
                radius: 2.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
        },
    };

    let auto_scroll = AutoScroll {
        background: iced::Background::Color(palette.background.weakest.color),
        border: Border {
            radius: 2.0.into(),
            width: 1.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow {
            color: Color::TRANSPARENT,
            ..Default::default()
        },
        icon: palette.background.strong.color,
    };

    widget::scrollable::Style {
        container: container::Style {
            ..Default::default()
        },
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll,
    }
}

// custom widgets
pub fn split_ruler(theme: &Theme) -> iced::widget::rule::Style {
    let palette = theme.extended_palette();

    iced::widget::rule::Style {
        color: palette.background.strong.color.scale_alpha(0.25),
        radius: iced::border::Radius::default(),
        fill_mode: iced::widget::rule::FillMode::Full,
        snap: true,
    }
}

pub fn tab_ruler(theme: &Theme) -> iced::widget::rule::Style {
    let palette = theme.extended_palette();

    iced::widget::rule::Style {
        color: palette.background.strong.color.scale_alpha(0.5),
        radius: iced::border::Radius::default(),
        fill_mode: iced::widget::rule::FillMode::Full,
        snap: true,
    }
}

// crosshair dashed line for charts
pub fn chart_crosshair(theme: &Theme) -> Stroke<'_> {
    dashed_line(theme).with_color(
        theme
            .extended_palette()
            .background
            .base
            .text
            .scale_alpha(0.35),
    )
}

pub fn dashed_line(theme: &'_ Theme) -> Stroke<'_> {
    let palette = theme.extended_palette();

    Stroke::with_color(
        Stroke {
            width: 1.0,
            line_dash: LineDash {
                segments: &[4.0, 4.0],
                offset: 8,
            },
            ..Default::default()
        },
        palette
            .secondary
            .strong
            .color
            .scale_alpha(if palette.is_dark { 0.8 } else { 1.0 }),
    )
}

pub fn dashed_line_from_palette(palette: &'_ Extended) -> Stroke<'_> {
    Stroke::with_color(
        Stroke {
            width: 1.0,
            line_dash: LineDash {
                segments: &[4.0, 4.0],
                offset: 8,
            },
            ..Default::default()
        },
        palette
            .secondary
            .strong
            .color
            .scale_alpha(if palette.is_dark { 0.8 } else { 1.0 }),
    )
}
