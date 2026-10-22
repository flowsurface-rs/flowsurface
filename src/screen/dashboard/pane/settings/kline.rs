use crate::screen::dashboard::pane::content::kline::{Message, SettingsMessage};
use crate::tooltip;
use crate::widget::inspector::{InspectorContent, InspectorSection};
use data::chart::kline::LegendVisibility;
use iced::{
    Length,
    widget::{
        button, checkbox, column, container, pick_list, text, tooltip::Position as TooltipPosition,
    },
};

pub fn kline_cfg_view(
    trade_data_enabled: bool,
    legend_visibility: LegendVisibility,
    trade_data_lock_reason: Option<&'static str>,
) -> InspectorContent<'static, Message> {
    let trade_data_enabled_checkbox = {
        let cb = checkbox(trade_data_enabled)
            .label("Trade data")
            .on_toggle_maybe(trade_data_lock_reason.is_none().then_some(|enabled| {
                Message::Settings(SettingsMessage::TradeDataEnabledChanged(enabled))
            }));

        tooltip(
            cb,
            Some(
                trade_data_lock_reason
                    .unwrap_or("Allow plots that need trade data to subscribe and fetch it"),
            ),
            TooltipPosition::Left,
        )
    };
    let mut trade_data_setting = column![trade_data_enabled_checkbox].spacing(4);
    if trade_data_enabled {
        trade_data_setting = trade_data_setting
            .push(
                text("Live trade data is enabled.\n\nHistorical trade data depends on your network settings.")
                    .size(crate::style::text_size::SMALL),
            )
            .push(button("Open Network settings").on_press(Message::OpenNetworkSettings));
    }

    let legend_picklist = tooltip(
        pick_list(
            LegendVisibility::ALL,
            Some(legend_visibility),
            |visibility| Message::Settings(SettingsMessage::LegendVisibilityChanged(visibility)),
        )
        .width(Length::Fill),
        None,
        TooltipPosition::Left,
    );

    InspectorContent::new(vec![
        InspectorSection::titled(
            "Data",
            container(trade_data_setting)
                .width(Length::Fill)
                .padding(8)
                .style(crate::style::inspector_control),
        )
        .with_modal_container(),
        InspectorSection::titled("Legend visibility", legend_picklist).with_modal_container(),
    ])
}

pub mod presets {
    use crate::{
        screen::dashboard::pane::{KlineChart, PresetApplication},
        style::{self, Icon, icon_text},
        widget::{column_drag::DragEvent, manager_icon_button, manager_list, manager_row},
    };
    use data::layout::presets::{Command, Library, MAX_NAME_LENGTH, Response};
    use iced::{
        Element, Length,
        widget::{button, center, column, container, row, space, text, text_input},
    };
    use uuid::Uuid;

    #[derive(Debug, Clone)]
    pub enum Message {
        ToggleEdit,
        Apply(Uuid),
        Save,
        SaveAs,
        Rename(Uuid),
        Duplicate(Uuid),
        Delete(Uuid),
        ConfirmDelete(Uuid),
        Reorder(DragEvent),
        NameChanged(String),
        SubmitName,
        ConfirmApply(Uuid),
        Cancel,
    }

    enum Naming {
        Create,
        Rename(Uuid),
    }

    #[derive(Default)]
    enum Dialog {
        #[default]
        None,
        Naming {
            purpose: Naming,
            name: String,
        },
        Apply(Uuid),
        Delete(Uuid),
        Error {
            target: Option<Uuid>,
            message: String,
        },
    }

    enum DialogView<'a> {
        Inline(iced::widget::Row<'a, Message>),
        Expanded(Element<'a, Message>),
    }

    impl Dialog {
        fn target(&self) -> Option<Uuid> {
            match self {
                Self::Naming {
                    purpose: Naming::Rename(id),
                    ..
                }
                | Self::Apply(id)
                | Self::Delete(id) => Some(*id),
                Self::Naming {
                    purpose: Naming::Create,
                    ..
                } => None,
                Self::Error { target, .. } => *target,
                Self::None => None,
            }
        }
    }

    pub struct State {
        managing: bool,
        dialog: Dialog,
        name_input: iced::widget::Id,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                managing: false,
                dialog: Dialog::None,
                name_input: iced::widget::Id::unique(),
            }
        }
    }

    #[derive(Default)]
    pub enum Action {
        #[default]
        None,
        FocusWidget(iced::widget::Id),
        Command(Command),
        Apply {
            id: Uuid,
            application: PresetApplication,
        },
    }

    enum ApplyStage {
        Preview,
        Confirmed,
    }

    impl State {
        pub fn cancel(&mut self) -> bool {
            if !matches!(self.dialog, Dialog::None) {
                self.dialog = Dialog::None;
                true
            } else if self.managing {
                self.managing = false;
                true
            } else {
                false
            }
        }

        fn start_name(&mut self, library: &Library, purpose: Naming) -> Action {
            let name = match &purpose {
                Naming::Rename(id) => library
                    .get(Some(*id))
                    .map(|preset| preset.name.clone())
                    .unwrap_or_default(),
                Naming::Create => library.unique_name("Preset 1", None),
            };
            self.dialog = Dialog::Naming { purpose, name };
            Action::FocusWidget(self.name_input.clone())
        }

        fn apply(
            &mut self,
            id: Uuid,
            stage: ApplyStage,
            library: &Library,
            chart: &KlineChart,
        ) -> Action {
            let Some(preset) = library.get(Some(id)) else {
                self.dialog = Dialog::Error {
                    target: Some(id),
                    message: "This preset no longer exists.".into(),
                };
                return Action::None;
            };
            let application = match chart.prepare_preset(&preset.settings) {
                Ok(application) => application,
                Err(error) => {
                    self.dialog = Dialog::Error {
                        target: Some(id),
                        message: error,
                    };
                    return Action::None;
                }
            };

            match stage {
                ApplyStage::Preview if application.preview.requires_confirmation() => {
                    self.dialog = Dialog::Apply(id);
                    Action::None
                }
                _ => {
                    self.dialog = Dialog::None;
                    Action::Apply { id, application }
                }
            }
        }

        pub fn command_completed(&mut self, response: Result<Response, String>) {
            match response {
                Ok(Response::Reordered) => {}
                Ok(_) => self.dialog = Dialog::None,
                Err(error) => {
                    self.dialog = Dialog::Error {
                        target: self.dialog.target(),
                        message: error,
                    };
                }
            }
        }

        pub fn update(
            &mut self,
            message: Message,
            library: &Library,
            chart: &KlineChart,
        ) -> Action {
            let command = match message {
                Message::ToggleEdit => {
                    self.managing = !self.managing;
                    return Action::None;
                }
                Message::Cancel => {
                    self.dialog = Dialog::None;
                    return Action::None;
                }
                Message::Apply(id) => {
                    return self.apply(id, ApplyStage::Preview, library, chart);
                }
                Message::ConfirmApply(id) => {
                    return self.apply(id, ApplyStage::Confirmed, library, chart);
                }
                Message::Save => {
                    let Some(id) = chart.selected_preset else {
                        return Action::None;
                    };
                    Command::Save {
                        id,
                        settings: chart.preset_settings(),
                    }
                }
                Message::SaveAs => {
                    return self.start_name(library, Naming::Create);
                }
                Message::Rename(id) => return self.start_name(library, Naming::Rename(id)),
                Message::NameChanged(value) => {
                    if let Dialog::Naming { name, .. } = &mut self.dialog {
                        *name = value.chars().take(MAX_NAME_LENGTH).collect();
                    }
                    return Action::None;
                }
                Message::SubmitName => {
                    let Dialog::Naming { purpose, name } = &self.dialog else {
                        return Action::None;
                    };
                    if name.trim().is_empty() {
                        return Action::None;
                    }
                    match purpose {
                        Naming::Rename(id) => Command::Rename {
                            id: *id,
                            name: name.clone(),
                        },
                        Naming::Create => Command::Create {
                            name: name.clone(),
                            settings: chart.preset_settings(),
                        },
                    }
                }
                Message::Duplicate(id) => Command::Duplicate(id),
                Message::Delete(id) => {
                    self.dialog = Dialog::Delete(id);
                    return Action::None;
                }
                Message::ConfirmDelete(id) => Command::Remove(id),
                Message::Reorder(DragEvent::Dropped {
                    index,
                    target_index,
                }) => Command::Reorder {
                    index,
                    target: target_index,
                },
                Message::Reorder(_) => return Action::None,
            };
            Action::Command(command)
        }

        fn dialog_view<'a>(
            &'a self,
            library: &'a Library,
            chart: &'a KlineChart,
        ) -> Option<DialogView<'a>> {
            let cancel = || button("Cancel").on_press(Message::Cancel);
            if self
                .dialog
                .target()
                .is_some_and(|id| library.get(Some(id)).is_none())
            {
                return Some(DialogView::Expanded(
                    column![text("This preset no longer exists."), cancel()]
                        .spacing(8)
                        .into(),
                ));
            }

            let mut content = column![].spacing(8);
            let confirm_button = |message| {
                manager_icon_button(Icon::Checkmark, message)
                    .style(|theme, status| style::button::confirm(theme, status, true))
            };
            let cancel_button = || {
                manager_icon_button(Icon::Close, Some(Message::Cancel))
                    .style(|theme, status| style::button::cancel(theme, status, true))
            };

            match &self.dialog {
                Dialog::Naming { name, .. } => {
                    return Some(DialogView::Inline(row![
                        center(
                            text_input("Preset name", name)
                                .id(self.name_input.clone())
                                .on_input(Message::NameChanged)
                                .on_submit(Message::SubmitName),
                        )
                        .padding(iced::padding::left(4)),
                        cancel_button(),
                    ]));
                }
                Dialog::Apply(id) => {
                    let application = library
                        .get(Some(*id))
                        .ok_or_else(|| "This preset no longer exists.".to_owned())
                        .and_then(|preset| chart.prepare_preset(&preset.settings));

                    match application {
                        Ok(application) => {
                            let preview = application.preview;
                            for adjustment in preview.adjustments {
                                content = content.push(text(adjustment));
                            }
                            if preview.affected_drawings > 0 {
                                content = content.push(text(format!("Applying removes {} drawing(s) on replaced indicator panels. Price-panel drawings are kept.", preview.affected_drawings)));
                            }
                            content =
                                content.push(button("Apply").on_press(Message::ConfirmApply(*id)));
                        }
                        Err(error) => content = content.push(text(error)),
                    }
                    content = content.push(cancel());
                }
                Dialog::Delete(id) => {
                    return Some(DialogView::Inline(row![
                        center(
                            text("Delete this preset?")
                                .size(style::text_size::BODY)
                                .wrapping(iced::widget::text::Wrapping::None),
                        )
                        .clip(true),
                        confirm_button(Some(Message::ConfirmDelete(*id))),
                        cancel_button(),
                    ]));
                }
                Dialog::Error { message, .. } => {
                    content = content.push(text(message)).push(cancel())
                }
                Dialog::None => return None,
            }

            Some(DialogView::Expanded(content.into()))
        }

        pub fn view<'a>(
            &'a self,
            library: &'a Library,
            chart: &'a KlineChart,
        ) -> Element<'a, Message> {
            let idle = matches!(self.dialog, Dialog::None);
            let show_actions = idle && self.managing;
            let selected = library.get(chart.selected_preset);
            let modified = selected.is_some_and(|preset| chart.preset_modified(&preset.settings));
            let target = self
                .dialog
                .target()
                .filter(|id| library.get(Some(*id)).is_some());
            let mut dialog = self.dialog_view(library, chart);
            let edit_button: Element<_> = if self.managing {
                crate::tooltip(
                    button(icon_text(Icon::Return, 12))
                        .on_press_maybe(idle.then_some(Message::ToggleEdit)),
                    Some("Finish editing"),
                    iced::widget::tooltip::Position::Top,
                )
            } else {
                button("Edit")
                    .on_press_maybe(idle.then_some(Message::ToggleEdit))
                    .into()
            };
            let mut content = column![
                row![
                    button("Save").on_press_maybe((idle && modified).then_some(Message::Save)),
                    button("Save as").on_press_maybe(idle.then_some(Message::SaveAs)),
                    space::horizontal(),
                    edit_button,
                ]
                .spacing(4)
            ]
            .spacing(8);

            let mut rows = Vec::new();
            for preset in library.entries() {
                let is_selected = chart.selected_preset == Some(preset.id);
                let name = if is_selected && modified {
                    format!("{}*", preset.name)
                } else {
                    preset.name.clone()
                };
                let label = button(
                    text(name)
                        .size(style::text_size::BODY)
                        .align_y(iced::Alignment::Center)
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .width(Length::Fill)
                .clip(true)
                .on_press_maybe(
                    (idle && !self.managing && (!is_selected || modified))
                        .then_some(Message::Apply(preset.id)),
                )
                .style(style::button::manager_entry);
                let mut entry = row![label];
                if show_actions {
                    entry = entry
                        .push(crate::tooltip(
                            manager_icon_button(Icon::Clone, Some(Message::Duplicate(preset.id))),
                            Some("Duplicate preset"),
                            iced::widget::tooltip::Position::Top,
                        ))
                        .push(manager_icon_button(
                            Icon::Edit,
                            Some(Message::Rename(preset.id)),
                        ));
                    if !is_selected {
                        entry = entry.push(manager_icon_button(
                            Icon::TrashBin,
                            Some(Message::Delete(preset.id)),
                        ));
                    }
                } else if idle && is_selected && !modified {
                    entry = entry.push(
                        container(icon_text(Icon::Checkmark, 12)).padding(iced::padding::right(16)),
                    );
                }

                let row_body = if target == Some(preset.id) {
                    match dialog.take() {
                        Some(DialogView::Inline(row)) => {
                            entry = row;
                            None
                        }
                        Some(DialogView::Expanded(body)) => Some(body),
                        None => None,
                    }
                } else {
                    None
                };
                rows.push(manager_row(entry, is_selected, self.managing, row_body));
            }

            let mut list = column![manager_list(
                rows,
                self.managing.then_some(Message::Reorder),
            )]
            .spacing(4);
            if target.is_none()
                && let Some(dialog) = dialog.take()
            {
                match dialog {
                    DialogView::Inline(row) => {
                        list = list.push(manager_row(row, false, false, None));
                    }
                    DialogView::Expanded(body) => {
                        content = content.push(
                            container(
                                container(body)
                                    .width(Length::Fill)
                                    .padding(12)
                                    .style(|theme| style::manager_row_container(theme, true)),
                            )
                            .width(Length::Fill)
                            .padding(2)
                            .style(style::dragger_row_container),
                        );
                    }
                }
            }

            content.push(list).into()
        }
    }
}
