use std::borrow::Cow;

use crate::{
    Deferred, screen::sidebar, split_column, style, theme::ThemeState, version, widget::tooltip,
};

use data::{
    config::theme::default_theme,
    sidebar::{Menu, Position as SidebarPosition},
};
use iced::{
    Alignment, Element,
    widget::tooltip::Position,
    widget::{button, checkbox, column, container, pick_list, row, scrollable, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    ThemeSelected(iced::Theme),
    TimezoneChanged(data::UserTimezone),
    SetMetadataCaching(bool),
    RefreshMetadata,
    SetVolumeSizeUnit(exchange::SizeUnit),
    SetAntialiasing(bool),
    ScaleFactorChanged(data::ScaleFactor),
    OpenDataFolder,
    OpenUrl(Cow<'static, str>),
    Sidebar(sidebar::Message),
}

/// Work the settings cannot do themselves, because it acts on state owned
/// elsewhere. Mirrors `tickers_table::Action`.
pub enum Action {
    /// A restart-gated setting changed, so what is on disk is now stale.
    Persist,
    ThemeSelected(iced::Theme),
    SetMetadataCaching(bool),
    RefreshMetadata,
    OpenDataFolder,
    OpenUrl(Cow<'static, str>),
    Sidebar(sidebar::Message),
}

/// The settings the app owns and edits from the settings modal.
pub struct Settings {
    pub timezone: data::UserTimezone,
    pub scale_factor: data::ScaleFactor,
    pub volume_size_unit: Deferred<exchange::SizeUnit>,
    pub boot: Deferred<data::BootSettings>,
}

impl Settings {
    pub fn new(
        timezone: data::UserTimezone,
        scale_factor: data::ScaleFactor,
        volume_size_unit: exchange::SizeUnit,
        boot: data::BootSettings,
    ) -> Self {
        Self {
            timezone,
            scale_factor,
            volume_size_unit: Deferred::new(volume_size_unit),
            boot: Deferred::new(boot),
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::TimezoneChanged(timezone) => {
                self.timezone = timezone;
            }
            Message::SetVolumeSizeUnit(unit) => {
                self.volume_size_unit.set_wanted(unit);
                return Some(Action::Persist);
            }
            Message::SetAntialiasing(enabled) => {
                self.boot.set_wanted(data::BootSettings {
                    antialiasing: enabled,
                });
                return Some(Action::Persist);
            }
            Message::ScaleFactorChanged(value) => {
                self.scale_factor = value;
            }
            Message::ThemeSelected(theme) => return Some(Action::ThemeSelected(theme)),
            Message::SetMetadataCaching(enabled) => {
                return Some(Action::SetMetadataCaching(enabled));
            }
            Message::RefreshMetadata => return Some(Action::RefreshMetadata),
            Message::OpenDataFolder => return Some(Action::OpenDataFolder),
            Message::OpenUrl(url) => return Some(Action::OpenUrl(url)),
            Message::Sidebar(message) => return Some(Action::Sidebar(message)),
        }
        None
    }

    pub fn view<'a>(
        &'a self,
        theme: &'a ThemeState,
        custom_theme: Option<&'a iced::Theme>,
        sidebar: &'a sidebar::Sidebar,
        sidebar_position: SidebarPosition,
    ) -> Element<'a, Message> {
        let theme_picklist = {
            let mut themes: Vec<iced::Theme> = iced_core::Theme::ALL.to_vec();

            let default_theme = iced_core::Theme::Custom(default_theme().into());
            themes.push(default_theme);

            if let Some(custom_theme) = custom_theme {
                themes.push(custom_theme.clone());
            }

            pick_list(
                themes,
                Some(theme.current().clone()),
                Message::ThemeSelected,
            )
        };

        let toggle_theme_editor = button(text("Theme editor")).on_press(Message::Sidebar(
            sidebar::Message::ToggleSidebarMenu(Some(Menu::ThemeEditor)),
        ));

        let toggle_network_editor = button(text("Network")).on_press(Message::Sidebar(
            sidebar::Message::ToggleSidebarMenu(Some(Menu::Network)),
        ));

        let timezone_picklist = pick_list(
            [data::UserTimezone::Utc, data::UserTimezone::Local],
            Some(self.timezone),
            Message::TimezoneChanged,
        );

        let restart_required_marker = || {
            let btn = button(
                text("!").style(|theme: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme.extended_palette().warning.base.color),
                }),
            )
            .style(move |theme, status| style::button::modifier(theme, status, true));
            tooltip(btn, Some("Restart required"), Position::Top)
        };

        let size_in_quote_ccy_checkbox = {
            let is_active = *self.volume_size_unit.wanted() == exchange::SizeUnit::Quote;
            let restart_required = self.volume_size_unit.restart_required();

            let checkbox = checkbox(is_active)
                .label("Size in quote currency")
                .on_toggle(|checked| {
                    Message::SetVolumeSizeUnit(if checked {
                        exchange::SizeUnit::Quote
                    } else {
                        exchange::SizeUnit::Base
                    })
                });

            let checkbox = tooltip(
                checkbox,
                Some(
                    "Display sizes/volumes in quote currency (USD).\n- Has no effect on inverse perps or open interest\n- Restart required to apply.",
                ),
                Position::Top,
            );

            let mut row = row![checkbox]
                .align_y(Alignment::Center)
                .spacing(8)
                .height(24);
            if restart_required {
                row = row.push(restart_required_marker());
            }
            row
        };

        let cache_metadata_checkbox = {
            let cache_enabled = sidebar.cache_enabled();

            let checkbox = tooltip(
                checkbox(cache_enabled)
                    .label("Cache ticker metadata")
                    .on_toggle(Message::SetMetadataCaching),
                Some("Cache ticker metadata for faster startup."),
                Position::Top,
            );

            if !cache_enabled {
                column![row![checkbox].height(24).align_y(Alignment::Center)]
            } else {
                let loading = sidebar.is_metadata_loading();

                let refresh_btn = {
                    let btn = button(crate::style::icon_text(crate::style::Icon::Refresh, 12))
                        .style(move |theme, status| {
                            style::button::modifier(theme, status, loading)
                        });
                    let btn = if loading {
                        btn
                    } else {
                        btn.on_press(Message::RefreshMetadata)
                    };

                    tooltip(
                        btn,
                        if loading { None } else { Some("Refresh now") },
                        Position::Top,
                    )
                };

                let last_updated = {
                    let last_update = sidebar.last_metadata_update();

                    let label = if loading {
                        "Refreshing metadata…".to_string()
                    } else {
                        match last_update {
                            Some(at) => {
                                format!(
                                    "Metadata last update: {}",
                                    data::util::relative_time_label(at)
                                )
                            }
                            None => "Metadata not fetched yet".to_string(),
                        }
                    };

                    text(label)
                        .size(crate::style::text_size::SMALL)
                        .style(style::secondary_text)
                };

                column![
                    row![checkbox, refresh_btn]
                        .align_y(Alignment::Center)
                        .spacing(8),
                    last_updated,
                ]
                .spacing(4)
            }
        };

        let sidebar_pos_picklist = pick_list(
            [SidebarPosition::Left, SidebarPosition::Right],
            Some(sidebar_position),
            |pos| Message::Sidebar(sidebar::Message::SetSidebarPosition(pos)),
        );

        let scale_factor = {
            let current_value: f32 = self.scale_factor.into();

            let decrease_btn = if current_value > data::config::MIN_SCALE {
                button(text("-"))
                    .on_press(Message::ScaleFactorChanged((current_value - 0.1).into()))
            } else {
                button(text("-"))
            };

            let increase_btn = if current_value < data::config::MAX_SCALE {
                button(text("+"))
                    .on_press(Message::ScaleFactorChanged((current_value + 0.1).into()))
            } else {
                button(text("+"))
            };

            container(
                row![
                    decrease_btn,
                    text(format!("{:.0}%", current_value * 100.0))
                        .size(crate::style::text_size::SECTION),
                    increase_btn,
                ]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(4),
            )
            .style(style::modal_container)
        };

        let antialiasing_checkbox = {
            let restart_required = self.boot.differs(|boot| &boot.antialiasing);
            let checkbox = checkbox(self.boot.wanted().antialiasing)
                .label("Antialiasing")
                .on_toggle(Message::SetAntialiasing);

            let checkbox = tooltip(
                checkbox,
                Some(
                    "Disabling may increase performance, but expect rough edges\n- Restart required to apply.",
                ),
                Position::Top,
            );

            let mut row = row![checkbox]
                .align_y(Alignment::Center)
                .spacing(8)
                .height(24);
            if restart_required {
                row = row.push(restart_required_marker());
            }
            row
        };

        let open_data_folder = {
            let button = button(text("Open data folder")).on_press(Message::OpenDataFolder);

            tooltip(
                button,
                Some("Open the folder where the data & config is stored"),
                Position::Top,
            )
        };

        let version_info = {
            let (version_label, commit_label) = version::app_build_version_parts();

            let github_link_button =
                button(text(version_label).size(crate::style::text_size::EMPHASIS))
                    .padding(0)
                    .style(style::button::text_link)
                    .on_press(Message::OpenUrl(Cow::Borrowed(
                        version::GITHUB_REPOSITORY_URL,
                    )));

            let github_button: Element<'_, Message> = iced::widget::tooltip(
                github_link_button,
                container(
                    row![
                        text("GitHub"),
                        style::icon_text(style::Icon::ExternalLink, 12),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .style(style::tooltip)
                .padding(8),
                Position::Top,
            )
            .into();

            if let (Some(commit_label), Some(commit_url)) =
                (commit_label, version::build_commit_url())
            {
                let commit_button = button(text(commit_label).size(crate::style::text_size::SMALL))
                    .padding(0)
                    .style(style::button::text_link_secondary)
                    .on_press(Message::OpenUrl(Cow::Owned(commit_url)));

                column![github_button, commit_button]
                    .spacing(2)
                    .align_x(Alignment::End)
                    .into()
            } else {
                github_button
            }
        };

        let footer = column![
            container(version_info)
                .width(iced::Length::Fill)
                .align_x(Alignment::End),
        ]
        .spacing(8);

        let column_content = split_column![
            column![open_data_folder,].spacing(8),
            column![text("Sidebar position").size(crate::style::text_size::SECTION), sidebar_pos_picklist,].spacing(12),
            column![text("Time zone").size(crate::style::text_size::SECTION), timezone_picklist,].spacing(12),
            column![text("Market data").size(crate::style::text_size::SECTION), column![size_in_quote_ccy_checkbox, cache_metadata_checkbox].spacing(4)].spacing(12),
            column![text("Theme").size(crate::style::text_size::SECTION), theme_picklist,].spacing(12),
            column![text("Interface scale").size(crate::style::text_size::SECTION), scale_factor,].spacing(12),
            column![text("Rendering").size(crate::style::text_size::SECTION), antialiasing_checkbox,].spacing(12),
            column![
                text("Experimental").size(crate::style::text_size::SECTION),
                column![toggle_theme_editor, toggle_network_editor].spacing(8),
            ]
            .spacing(12),
            footer,
            ; spacing = 16, align_x = Alignment::Start
        ];

        let content = scrollable::Scrollable::with_direction(
            column_content,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        );

        container(content)
            .align_x(Alignment::Start)
            .max_width(240)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }
}
