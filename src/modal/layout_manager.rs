use crate::layout::{Layout, LayoutId};
use crate::screen::dashboard::Dashboard;
use crate::style::{Icon, icon_text};
use crate::widget::column_drag::{self, DragEvent};
use crate::widget::dragger_row;
use crate::{style, tooltip};

use iced::widget::{
    button, center, column, container, row, scrollable, space, text, text_input,
    tooltip::Position as TooltipPosition,
};
use iced::{Element, Theme, padding};
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
    active_layout_id: Option<Uuid>,
    pub edit_mode: Editing,
}

impl LayoutManager {
    pub fn new() -> Self {
        let default_layout = LayoutId {
            unique: Uuid::new_v4(),
            name: "Layout 1".into(),
        };

        Self {
            layouts: vec![Layout {
                id: default_layout.clone(),
                dashboard: Dashboard::default(),
            }],
            active_layout_id: Some(default_layout.unique),
            edit_mode: Editing::None,
        }
    }

    pub fn from_config(layouts: Vec<Layout>, active_layout: Option<LayoutId>) -> Self {
        Self {
            layouts,
            active_layout_id: active_layout.map(|l| l.unique),
            edit_mode: Editing::None,
        }
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
        let mut final_name = proposed.to_string();
        let mut suffix = 2;
        while self
            .layouts
            .iter()
            .any(|layout| layout.id.unique != current_id && layout.id.name == final_name)
        {
            final_name = format!("{proposed} ({suffix})");
            suffix += 1;
        }
        final_name.chars().take(20).collect()
    }

    pub fn iter_dashboards_mut(&mut self) -> impl Iterator<Item = &mut Dashboard> {
        self.layouts.iter_mut().map(|entry| &mut entry.dashboard)
    }

    pub fn mut_dashboard(&mut self, id: Uuid) -> Option<&mut Dashboard> {
        self.get_mut(id).map(|e| &mut e.dashboard)
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
                        let truncated = name.chars().take(20).collect();
                        Editing::Renaming(id, truncated)
                    }
                    _ => Editing::None,
                };
            }
            Message::CloneLayout(id) => {
                return Some(Action::Clone(id));
            }
            Message::Reorder(event) => column_drag::reorder_vec(&mut self.layouts, &event),
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

            let mut layout_row = row![].height(iced::Length::Fixed(32.0)).padding(4);

            let is_active = self.active_layout_id == Some(layout_id.unique);
            match &self.edit_mode {
                Editing::ConfirmingDelete(delete_id) => {
                    if *delete_id == layout_id.unique {
                        let (confirm_btn, cancel_btn) = create_confirm_delete_buttons(layout_id);

                        layout_row = layout_row
                            .push(center(text(format!("Delete {}?", layout.id.name)).size(12)))
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
                layout_row = layout_row
                    .push(container(icon_text(Icon::Checkmark, 12)).padding(padding::right(16)));
            }

            let styled_container = container(layout_row.align_y(iced::Alignment::Center))
                .style(move |theme| {
                    let palette = theme.extended_palette();
                    let color = if is_active {
                        palette.background.weak.color
                    } else {
                        palette.background.weakest.color
                    };

                    iced::widget::container::Style {
                        background: Some(color.into()),
                        ..Default::default()
                    }
                })
                .into();

            layout_widgets.push(dragger_row(styled_container, is_edit_mode));
        }

        let layouts_list: Element<'_, Message> = if is_edit_mode {
            column_drag::Column::with_children(layout_widgets)
                .on_drag(Message::Reorder)
                .spacing(4)
                .into()
        } else {
            iced::widget::Column::with_children(layout_widgets)
                .spacing(4)
                .into()
        };

        content = content.push(layouts_list);

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
    create_icon_button(
        style::Icon::TrashBin,
        12,
        |theme, status| style::button::layout_name(theme, *status),
        Some(Message::ToggleEditMode(Editing::ConfirmingDelete(
            layout.unique,
        ))),
    )
    .into()
}

fn create_rename_button<'a>(layout: &LayoutId) -> button::Button<'a, Message> {
    create_icon_button(
        style::Icon::Edit,
        12,
        |theme, status| style::button::layout_name(theme, *status),
        Some(Message::ToggleEditMode(Editing::Renaming(
            layout.unique,
            layout.name.clone(),
        ))),
    )
}

fn create_clone_button<'a>(layout: &LayoutId) -> Element<'a, Message> {
    tooltip(
        create_icon_button(
            style::Icon::Clone,
            12,
            |theme, status| style::button::layout_name(theme, *status),
            Some(Message::CloneLayout(layout.unique)),
        ),
        Some("Clone layout"),
        TooltipPosition::Top,
    )
}

fn create_confirm_delete_buttons<'a>(
    layout: &LayoutId,
) -> (button::Button<'a, Message>, button::Button<'a, Message>) {
    let confirm = create_icon_button(
        style::Icon::Checkmark,
        12,
        |theme, status| style::button::confirm(theme, *status, true),
        Some(Message::RemoveLayout(layout.unique)),
    );

    let cancel = create_icon_button(
        style::Icon::Close,
        12,
        |theme, status| style::button::cancel(theme, *status, true),
        Some(Message::ToggleEditMode(Editing::Preview)),
    );

    (confirm, cancel)
}

fn create_layout_button<'a>(layout: &LayoutId, on_press: Option<Message>) -> Element<'a, Message> {
    let mut layout_btn = button(text(layout.name.clone()).align_y(iced::Alignment::Center))
        .width(iced::Length::Fill)
        .style(style::button::layout_name);

    if let Some(msg) = on_press {
        layout_btn = layout_btn.on_press(msg);
    }

    layout_btn.into()
}

fn create_icon_button<'a>(
    icon: style::Icon,
    size: u16,
    style_fn: impl Fn(&Theme, &button::Status) -> button::Style + 'static,
    on_press: Option<Message>,
) -> button::Button<'a, Message> {
    let mut btn = button(icon_text(icon, size).align_y(iced::Alignment::Center))
        .style(move |theme, status| style_fn(theme, &status));

    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }

    btn
}
