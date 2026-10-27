use super::Message as BrowserMessage;
use data::chart::kline::indicators::IndicatorId;
use data::chart::kline::indicators::manifest::{Entry, Installation, Message, State};
use iced::{
    Element, Length,
    widget::{button, column, container, row, scrollable, text, text_input},
};

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ViewState {
    search: String,
    expanded: Option<IndicatorId>,
}

impl ViewState {
    pub(super) fn set_search(&mut self, query: String) {
        self.search = query;
    }

    pub(super) fn toggle_details(&mut self, id: IndicatorId) {
        self.expanded = (self.expanded.as_ref() != Some(&id)).then_some(id);
    }

    pub(super) fn view<'a>(&'a self, catalog: &'a State) -> Element<'a, BrowserMessage> {
        use crate::{
            style::{self, Icon, icon_text},
            widget::tooltip,
        };
        use iced::widget::tooltip::Position;

        let loading = catalog.is_loading();
        let header = row![
            button(icon_text(Icon::Return, 14))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .padding([4, 6])
                .on_press(BrowserMessage::Back),
            text_input("Search catalog...", &self.search)
                .style(|theme, status| style::validated_text_input(theme, status, true))
                .on_input(BrowserMessage::CatalogSearchChanged)
                .padding(6)
                .width(Length::Fill),
            tooltip(
                button(icon_text(Icon::Refresh, 14))
                    .style(|theme, status| {
                        let mut appearance = style::button::transparent(theme, status, false);
                        if status == button::Status::Disabled {
                            appearance.text_color = theme.extended_palette().secondary.strong.color;
                        }
                        appearance
                    })
                    .padding([4, 6])
                    .on_press_maybe(
                        catalog
                            .can_refresh()
                            .then_some(BrowserMessage::Catalog(Message::Refresh))
                    ),
                Some("Refresh catalog"),
                Position::Bottom,
            ),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        let mut content = column![header].spacing(12);

        if let Some(error) = catalog.error() {
            content = content.push(crate::widget::banner(
                &error.message,
                crate::widget::BannerKind::Error,
            ));
        }

        let mut entries = column![].spacing(2);

        if let Some(published) = catalog.catalog() {
            let query = self.search.trim().to_lowercase();
            let mut count = 0;

            for entry in &published.indicators {
                if !entry.name.to_lowercase().contains(&query)
                    && !entry.id.as_str().to_lowercase().contains(&query)
                {
                    continue;
                }
                count += 1;
                entries = entries.push(self.entry_view(catalog, entry));
            }

            if count == 0 {
                entries = entries.push(text(if published.indicators.is_empty() {
                    "No indicators are available."
                } else {
                    "No matching indicators."
                }));
            }
        } else if loading {
            entries = entries.push(text("Loading indicators..."));
        }

        let list = scrollable::Scrollable::with_direction(
            entries,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            ),
        )
        .height(Length::Fill)
        .style(style::scroll_bar);

        container(content.push(list))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn entry_view<'a>(
        &'a self,
        catalog: &'a State,
        entry: &'a Entry,
    ) -> Element<'a, BrowserMessage> {
        use crate::style::{self, Icon, icon_text};

        let expanded = self.expanded.as_ref() == Some(&entry.id);
        let installation = catalog.installation(entry);
        let mut heading = row![].spacing(4).align_y(iced::Alignment::Center);

        if catalog.restart_pending_for(&entry.id) {
            heading = heading.push(crate::widget::tooltip(
                button(
                    text("!")
                        .size(style::text_size::BODY)
                        .style(|theme: &iced::Theme| iced::widget::text::Style {
                            color: Some(theme.extended_palette().warning.base.color),
                        }),
                )
                .padding([0, 2])
                .style(|theme, status| style::button::modifier(theme, status, true)),
                Some("Restart required"),
                iced::widget::tooltip::Position::Top,
            ));
        }

        heading = heading.push(
            text(&entry.name)
                .size(style::text_size::BODY)
                .width(Length::Fill),
        );

        if matches!(installation, Installation::Installed) {
            heading = heading.push(icon_text(Icon::Checkmark, 12));
        }

        let heading = button(heading.push(icon_text(
            if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            12,
        )))
        .width(Length::Fill)
        .style(move |theme, status| {
            let mut appearance = style::button::indicator_row(theme, status, false);
            if expanded {
                appearance.border.width = 0.0;
                if status == button::Status::Active {
                    appearance.background = None;
                }
            }
            appearance
        })
        .on_press(BrowserMessage::ToggleCatalogEntry(entry.id.clone()));

        if !expanded {
            return heading.into();
        }

        container(column![
            heading,
            self.entry_details(catalog, entry, installation)
        ])
        .width(Length::Fill)
        .style(move |theme| {
            let appearance = style::button::indicator_row(theme, button::Status::Active, expanded);
            container::Style {
                background: appearance.background,
                border: appearance.border,
                text_color: Some(appearance.text_color),
                ..Default::default()
            }
        })
        .into()
    }

    fn entry_details<'a>(
        &'a self,
        catalog: &'a State,
        entry: &'a Entry,
        installation: Installation,
    ) -> Element<'a, BrowserMessage> {
        use crate::{
            style,
            widget::inspector::{MODAL_SECTION_SPACING, setting_section},
        };

        let action = match installation {
            Installation::Installed => "Remove",
            Installation::Bundled => "Bundled",
            Installation::NotInstalled => "Install",
        };

        let description = entry.description.as_deref().unwrap_or("Not provided");
        let author = entry
            .author
            .as_ref()
            .map_or("Not provided", |author| author.name.as_str());
        let bases = entry.bases.join(", ");

        let mut details = column![
            crate::widget::text::muted(entry.id.as_str()),
            setting_section("Author", text(author).style(style::text_color::secondary))
                .with_modal_container()
                .into_element(),
            setting_section(
                "Description",
                text(description).style(style::text_color::secondary),
            )
            .with_modal_container()
            .into_element(),
            setting_section("Basis", text(bases).style(style::text_color::secondary))
                .with_modal_container()
                .into_element(),
        ]
        .spacing(MODAL_SECTION_SPACING)
        .width(Length::Fill);

        if let Some(error) = catalog.error_for(&entry.id) {
            details = details.push(crate::widget::banner(
                &error.message,
                crate::widget::BannerKind::Error,
            ));
        }

        details = details.push(
            row![
                iced::widget::space::horizontal(),
                button(text(action)).on_press_maybe(
                    catalog
                        .can_toggle(installation)
                        .then(|| BrowserMessage::Catalog(Message::ToggleSource(entry.id.clone()))),
                ),
            ]
            .spacing(8)
            .width(Length::Fill)
            .align_y(iced::Alignment::Center),
        );

        container(details).padding(10).width(Length::Fill).into()
    }
}
