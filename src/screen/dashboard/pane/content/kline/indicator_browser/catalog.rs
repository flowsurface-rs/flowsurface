use super::Message as BrowserMessage;
use crate::layout::indicator_catalog::{
    CatalogState, Entry, Message, SourceChange, SourceOperation, State,
};
use data::chart::kline::indicators::{IndicatorId, language::PackageOrigin};
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

        let loading = matches!(catalog.catalog_state(), CatalogState::Loading(_));
        let header = row![
            tooltip(
                button(icon_text(Icon::Return, 14))
                    .style(|theme, status| style::button::transparent(theme, status, false))
                    .padding([4, 6])
                    .on_press(BrowserMessage::Back),
                Some("Back to indicators"),
                Position::Bottom,
            ),
            text_input("Search catalog...", &self.search)
                .style(|theme, status| style::validated_text_input(theme, status, true))
                .on_input(BrowserMessage::CatalogSearchChanged)
                .padding(6)
                .width(Length::Fill),
            tooltip(
                button(icon_text(Icon::Refresh, 14))
                    .style(|theme, status| style::button::transparent(theme, status, false))
                    .padding([4, 6])
                    .on_press_maybe(
                        catalog
                            .can_refresh()
                            .then_some(BrowserMessage::Catalog(Message::Refresh))
                    ),
                Some(if loading {
                    "Refreshing..."
                } else {
                    "Refresh catalog"
                }),
                Position::Bottom,
            ),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        let mut content = column![header].spacing(12);
        if let CatalogState::Failed { error, .. } = catalog.catalog_state() {
            content = content.push(crate::widget::banner(
                &error.message,
                crate::widget::BannerKind::Error,
            ));
        }

        let mut entries = column![].spacing(2);
        if let Some(manifest) = catalog.catalog_state().catalog() {
            let query = self.search.trim().to_lowercase();
            let mut count = 0;
            for entry in &manifest.indicators {
                if !entry.name.to_lowercase().contains(&query)
                    && !entry.id.as_str().to_lowercase().contains(&query)
                {
                    continue;
                }
                count += 1;
                entries = entries.push(self.entry_view(catalog, entry));
            }
            if count == 0 {
                entries = entries.push(text(if manifest.indicators.is_empty() {
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
        let mut heading = row![
            text(&entry.name)
                .size(style::text_size::BODY)
                .width(Length::Fill),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        if !expanded && catalog.is_installed(entry) {
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
            let mut appearance = style::button::indicator_row(theme, status, expanded);
            if expanded {
                appearance.border.width = 0.0;
            }
            appearance
        })
        .on_press(BrowserMessage::ToggleCatalogEntry(entry.id.clone()));
        if !expanded {
            return heading.into();
        }
        container(column![heading, self.entry_details(catalog, entry)])
            .width(Length::Fill)
            .padding(2)
            .style(move |theme| {
                let appearance = style::button::indicator_row(
                    theme,
                    iced::widget::button::Status::Active,
                    expanded,
                );
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
    ) -> Element<'a, BrowserMessage> {
        let label = if let SourceOperation::Running { id, change } = catalog.operation()
            && id == &entry.id
        {
            match change {
                SourceChange::Installed => "Installing...",
                SourceChange::Removed => "Removing...",
            }
        } else if catalog.source_change(&entry.id) == Some(SourceChange::Installed) {
            "Installed"
        } else if catalog.source_change(&entry.id) == Some(SourceChange::Removed)
            && entry.registration().is_some()
        {
            "Removed"
        } else if let Some(registration) = entry.registration() {
            match registration.origin {
                PackageOrigin::Bundled => "Bundled",
                PackageOrigin::Installed => "Installed",
            }
        } else {
            "Install"
        };
        let mut details = column![crate::widget::text::muted(entry.id.as_str())].spacing(6);
        if let Some(description) = &entry.description {
            details = details.push(text(description));
        }
        if let Some(author) = &entry.author {
            details = details.push(crate::widget::text::muted(format!("By {}", author.name)));
        }
        details = details.push(crate::widget::text::muted(format!(
            "Chart bases: {}",
            entry.bases.join(", ")
        )));
        if let SourceOperation::Failed { id, error } = catalog.operation()
            && id == &entry.id
        {
            details = details.push(crate::widget::banner(
                &error.message,
                crate::widget::BannerKind::Error,
            ));
        }
        let mut actions = row![
            button(text(label)).on_press_maybe(
                catalog
                    .can_install(entry)
                    .then(|| BrowserMessage::Catalog(Message::Install(entry.id.clone()))),
            )
        ]
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);
        if catalog.is_installed(entry) {
            actions = actions
                .push(iced::widget::space::horizontal())
                .push(crate::widget::tooltip(
                    button(crate::style::icon_text(crate::style::Icon::TrashBin, 12))
                        .style(|theme, status| crate::style::button::cancel(theme, status, false))
                        .padding([4, 6])
                        .on_press_maybe(
                            catalog.can_remove(entry).then(|| {
                                BrowserMessage::Catalog(Message::Remove(entry.id.clone()))
                            }),
                        ),
                    Some("Remove indicator"),
                    iced::widget::tooltip::Position::Bottom,
                ));
        }
        details = details.push(actions);
        match catalog.source_change(&entry.id) {
            Some(SourceChange::Installed) => {
                details = details.push(crate::widget::text::muted("Restart to use"));
            }
            Some(SourceChange::Removed) if entry.registration().is_some() => {
                details = details.push(crate::widget::text::muted("Restart to apply removal"));
            }
            _ => {}
        }
        container(details)
            .padding([6, 8])
            .width(Length::Fill)
            .into()
    }
}
