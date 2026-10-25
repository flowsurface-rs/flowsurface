use crate::connector::client::DataSources;
use crate::layout::{Layout, LayoutId, SharedChartResources, SharedChartResourcesMessage};
use crate::screen::dashboard::{self, Dashboard};
use crate::style::icon_text;
use crate::widget::column_drag::DragEvent;
use crate::widget::{manager_icon_button, manager_list, manager_row};
use crate::{style, tooltip};
use data::layout::MAX_NAME_LENGTH;
use data::layout::presets::{self, Library as PresetLibrary};

use iced::widget::{
    button, center, column, container, row, scrollable, space, text, text_input,
    tooltip::Position as TooltipPosition,
};
use iced::{Element, padding, window};
use std::vec;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Editing {
    ConfirmingDelete(Uuid),
    Renaming(Uuid, String),
    Preview,
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectActive(Uuid),
    SetLayoutName(Uuid, String),
    Renaming(String),
    AddLayout,
    RemoveLayout(Uuid),
    ToggleEditMode(Editing),
    CloneLayout(Uuid),
    Reorder(DragEvent),
}

pub enum Action {
    Select(Uuid),
    Clone(Uuid),
}

pub struct LayoutManager {
    pub layouts: Vec<Layout>,
    chart_resources: SharedChartResources,
    active_layout_id: Option<Uuid>,
    pub edit_mode: Editing,
}

impl LayoutManager {
    fn default_layout() -> (LayoutId, Layout) {
        let id = LayoutId {
            unique: Uuid::new_v4(),
            name: "Layout 1".into(),
        };
        let layout = Layout {
            id: id.clone(),
            dashboard: Dashboard::default(),
        };

        (id, layout)
    }

    pub fn new() -> Self {
        let (default_layout, layout) = Self::default_layout();

        Self {
            layouts: vec![layout],
            chart_resources: SharedChartResources::default(),
            active_layout_id: Some(default_layout.unique),
            edit_mode: Editing::None,
        }
    }

    pub fn from_config(
        layouts: Vec<Layout>,
        active_layout: Option<LayoutId>,
        chart_presets: Vec<data::chart::kline::ChartPreset>,
    ) -> Self {
        let mut manager = Self {
            layouts,
            chart_resources: SharedChartResources::from_config(chart_presets),
            active_layout_id: active_layout.map(|l| l.unique),
            edit_mode: Editing::None,
        };
        manager.ensure_layout();
        manager
    }

    pub fn chart_resources(&self) -> &SharedChartResources {
        &self.chart_resources
    }

    pub fn presets(&self) -> &PresetLibrary {
        self.chart_resources.presets()
    }

    pub fn ensure_layout(&mut self) {
        if !self.layouts.is_empty() {
            return;
        }

        let (default_layout, layout) = Self::default_layout();
        self.layouts.push(layout);
        self.active_layout_id = Some(default_layout.unique);
        self.edit_mode = Editing::None;
    }

    pub fn get(&self, unique: Uuid) -> Option<&Layout> {
        self.layouts
            .iter()
            .find(|layout| layout.id.unique == unique)
    }

    pub fn get_mut(&mut self, unique: Uuid) -> Option<&mut Layout> {
        self.layouts
            .iter_mut()
            .find(|layout| layout.id.unique == unique)
    }

    pub fn active_layout_id(&self) -> Option<&LayoutId> {
        self.get(self.active_layout_id?).map(|layout| &layout.id)
    }

    pub fn insert_layout(&mut self, id: LayoutId, dashboard: Dashboard) {
        self.layouts.push(Layout { id, dashboard });
    }

    pub fn generate_unique_layout_name(&self) -> String {
        let mut counter = 1;
        loop {
            let candidate = format!("Layout {counter}");
            if !self
                .layouts
                .iter()
                .any(|layout| layout.id.name == candidate)
            {
                return candidate;
            }
            counter += 1;
        }
    }

    pub fn ensure_unique_name(&self, proposed: &str, current_id: Uuid) -> String {
        data::util::unique_name(
            proposed,
            self.layouts
                .iter()
                .filter(|layout| layout.id.unique != current_id)
                .map(|layout| layout.id.name.as_str()),
            MAX_NAME_LENGTH,
        )
    }

    pub fn mut_dashboard(&mut self, id: Uuid) -> Option<&mut Dashboard> {
        self.get_mut(id).map(|e| &mut e.dashboard)
    }

    pub fn update_dashboard(
        &mut self,
        message: dashboard::Message,
        main_window: &crate::window::Window,
        layout_id: &Uuid,
        data_sources: &DataSources,
    ) -> Option<(iced::Task<dashboard::Message>, Option<dashboard::Event>)> {
        let layout_index = self
            .layouts
            .iter()
            .position(|layout| layout.id.unique == *layout_id)?;
        let dashboard = &mut self.layouts[layout_index].dashboard;

        Some(dashboard.update(message, main_window, data_sources, &self.chart_resources))
    }

    pub fn update_chart_resources(
        &mut self,
        message: SharedChartResourcesMessage,
        main_window: window::Id,
        catalog_http: &reqwest::Client,
    ) -> iced::Task<SharedChartResourcesMessage> {
        match message {
            SharedChartResourcesMessage::PresetCommand {
                layout_id,
                window,
                pane,
                command,
            } => {
                self.apply_preset_command(main_window, layout_id, (window, pane), command);
                iced::Task::none()
            }
            SharedChartResourcesMessage::IndicatorCatalog(message) => self
                .chart_resources
                .update_indicator_catalog(message, catalog_http)
                .map(SharedChartResourcesMessage::IndicatorCatalog),
        }
    }

    fn apply_preset_command(
        &mut self,
        main_window: window::Id,
        layout_id: Uuid,
        origin: (window::Id, iced::widget::pane_grid::Pane),
        command: presets::Command,
    ) {
        let response = self.chart_resources.update_presets(command);

        for layout in &mut self.layouts {
            layout
                .dashboard
                .reconcile_selected_presets(main_window, self.chart_resources.presets());
        }

        if let Some(layout) = self
            .layouts
            .iter_mut()
            .find(|layout| layout.id.unique == layout_id)
        {
            layout
                .dashboard
                .preset_command_completed(main_window, origin.0, origin.1, response);
        }
    }

    pub fn park_inactive_layouts(&mut self, active_layout_id: Uuid, main_window: window::Id) {
        self.layouts
            .iter_mut()
            .filter(|layout| layout.id.unique != active_layout_id)
            .for_each(|layout| layout.dashboard.park_for_inactive_layout(main_window));
    }

    pub fn set_active_layout(&mut self, layout_id: Uuid) -> Result<&mut Layout, String> {
        self.active_layout_id = Some(layout_id);

        self.get_mut(layout_id)
            .ok_or_else(|| "Layout not found".into())
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::SelectActive(id) => {
                self.active_layout_id = Some(id);
                return Some(Action::Select(id));
            }
            Message::ToggleEditMode(new_mode) => match (&new_mode, &self.edit_mode) {
                (Editing::Preview, Editing::Preview) => {
                    self.edit_mode = Editing::None;
                }
                (Editing::Renaming(id, _), Editing::Renaming(renaming_id, _))
                    if id == renaming_id =>
                {
                    self.edit_mode = Editing::None;
                }
                _ => {
                    self.edit_mode = new_mode;
                }
            },
            Message::AddLayout => {
                let new_layout = LayoutId {
                    unique: Uuid::new_v4(),
                    name: self.generate_unique_layout_name(),
                };

                self.insert_layout(new_layout.clone(), Dashboard::default());

                return Some(Action::Select(new_layout.unique));
            }
            Message::RemoveLayout(id) => {
                if Some(id) == self.active_layout_id {
                    return None;
                }
                self.layouts.retain(|layout| layout.id.unique != id);
                self.edit_mode = Editing::Preview;
            }
            Message::SetLayoutName(id, new_name) => {
                let unique_name = self.ensure_unique_name(&new_name, id);

                if let Some(layout) = self.get_mut(id) {
                    layout.id.name = unique_name;
                }

                self.edit_mode = Editing::Preview;
            }
            Message::Renaming(name) => {
                self.edit_mode = match self.edit_mode {
                    Editing::Renaming(id, _) => {
                        let truncated = name.chars().take(MAX_NAME_LENGTH).collect();
                        Editing::Renaming(id, truncated)
                    }
                    _ => Editing::None,
                };
            }
            Message::CloneLayout(id) => {
                return Some(Action::Clone(id));
            }
            Message::Reorder(DragEvent::Dropped {
                index,
                target_index,
            }) => {
                let _ = data::util::reorder_vec(&mut self.layouts, index, target_index);
            }
            Message::Reorder(_) => {}
        }

        None
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut content = column![].spacing(8);

        let is_edit_mode = self.edit_mode != Editing::None;

        let edit_btn = if is_edit_mode {
            button(icon_text(style::Icon::Return, 12))
                .on_press(Message::ToggleEditMode(Editing::Preview))
        } else {
            button(text("Edit")).on_press(Message::ToggleEditMode(Editing::Preview))
        };

        content = content.push(row![
            space::horizontal(),
            if is_edit_mode {
                row![edit_btn]
            } else {
                row![
                    tooltip(
                        button("i").style(style::button::info),
                        Some("Layouts won't be saved if app exits abruptly"),
                        TooltipPosition::Top,
                    ),
                    edit_btn,
                ]
                .spacing(4)
            }
        ]);

        let mut layout_widgets: Vec<Element<'_, Message>> = vec![];

        for layout in &self.layouts {
            let layout_id = &layout.id;

            let mut layout_row = row![];

            let is_active = self.active_layout_id == Some(layout_id.unique);
            match &self.edit_mode {
                Editing::ConfirmingDelete(delete_id) => {
                    if *delete_id == layout_id.unique {
                        let (confirm_btn, cancel_btn) = create_confirm_delete_buttons(layout_id);

                        layout_row = layout_row
                            .push(center(
                                text(format!("Delete {}?", layout.id.name))
                                    .size(crate::style::text_size::BODY),
                            ))
                            .push(confirm_btn)
                            .push(cancel_btn);
                    } else {
                        layout_row = layout_row.push(create_layout_button(layout_id, None));
                    }
                }
                Editing::Renaming(renaming_id, name) => {
                    if *renaming_id == layout_id.unique {
                        let input_box = text_input("New layout name", name)
                            .on_input(|new_name| Message::Renaming(new_name.clone()))
                            .on_submit(Message::SetLayoutName(*renaming_id, name.clone()));

                        let (_, cancel_btn) = create_confirm_delete_buttons(layout_id);

                        layout_row = layout_row
                            .push(center(input_box).padding(padding::left(4)))
                            .push(cancel_btn);
                    } else {
                        layout_row = layout_row.push(create_layout_button(layout_id, None));
                    }
                }
                Editing::Preview => {
                    layout_row = layout_row
                        .push(create_layout_button(layout_id, None))
                        .push(create_clone_button(layout_id))
                        .push(create_rename_button(layout_id));

                    if !is_active {
                        layout_row = layout_row.push(create_delete_button(layout_id));
                    }
                }
                Editing::None => {
                    layout_row = layout_row.push(create_layout_button(
                        layout_id,
                        if is_active {
                            None
                        } else {
                            Some(Message::SelectActive(layout_id.unique))
                        },
                    ));
                }
            }

            if is_active && !is_edit_mode {
                layout_row = layout_row.push(
                    container(icon_text(style::Icon::Checkmark, 12)).padding(padding::right(16)),
                );
            }

            layout_widgets.push(manager_row(layout_row, is_active, is_edit_mode));
        }

        content = content.push(manager_list(
            layout_widgets,
            is_edit_mode.then_some(Message::Reorder),
        ));

        if self.edit_mode != Editing::None {
            content = content.push(
                button(text("Add layout"))
                    .style(move |t, s| style::button::transparent(t, s, true))
                    .width(iced::Length::Fill)
                    .on_press(Message::AddLayout),
            );
        };

        scrollable::Scrollable::with_direction(
            content,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        )
        .into()
    }
}

fn create_delete_button<'a>(layout: &LayoutId) -> Element<'a, Message> {
    manager_icon_button(
        style::Icon::TrashBin,
        Some(Message::ToggleEditMode(Editing::ConfirmingDelete(
            layout.unique,
        ))),
    )
    .into()
}

fn create_rename_button<'a>(layout: &LayoutId) -> button::Button<'a, Message> {
    manager_icon_button(
        style::Icon::Edit,
        Some(Message::ToggleEditMode(Editing::Renaming(
            layout.unique,
            layout.name.clone(),
        ))),
    )
}

fn create_clone_button<'a>(layout: &LayoutId) -> Element<'a, Message> {
    tooltip(
        manager_icon_button(
            style::Icon::Clone,
            Some(Message::CloneLayout(layout.unique)),
        ),
        Some("Duplicate layout"),
        TooltipPosition::Top,
    )
}

fn create_confirm_delete_buttons<'a>(
    layout: &LayoutId,
) -> (button::Button<'a, Message>, button::Button<'a, Message>) {
    let confirm = manager_icon_button(
        style::Icon::Checkmark,
        Some(Message::RemoveLayout(layout.unique)),
    )
    .style(|theme, status| style::button::confirm(theme, status, true));

    let cancel = manager_icon_button(
        style::Icon::Close,
        Some(Message::ToggleEditMode(Editing::Preview)),
    )
    .style(|theme, status| style::button::cancel(theme, status, true));

    (confirm, cancel)
}

fn create_layout_button<'a>(
    layout: &'a LayoutId,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(
        text(&layout.name)
            .align_y(iced::Alignment::Center)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .width(iced::Length::Fill)
    .clip(true)
    .style(style::button::manager_entry)
    .on_press_maybe(on_press)
    .into()
}
