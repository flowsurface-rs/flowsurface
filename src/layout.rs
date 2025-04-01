use crate::charts::{
    candlestick::CandlestickChart, footprint::FootprintChart, heatmap::HeatmapChart,
    timeandsales::TimeAndSales,
};
use crate::screen::dashboard::{Dashboard, PaneContent, PaneState};
use crate::style::get_icon_text;
use crate::widget::column_drag::{self, DragEvent, DropPosition};
use crate::{style, tooltip};
use data::{
    UserTimezone,
    chart::Basis,
    layout::{WindowSpec, pane::Axis},
};
use exchange::{TickMultiplier, Ticker, Timeframe, adapter::Exchange};

use iced::widget::{
    Space, button, center, column, container,
    pane_grid::{self, Configuration},
    row, scrollable, text, text_input,
    tooltip::Position as TooltipPosition,
};
use iced::{Element, Task, Theme, padding};
use std::{collections::HashMap, vec};
use uuid::Uuid;

#[derive(Eq, Hash, Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Editing {
    ConfirmingDelete(Uuid),
    Renaming(Uuid, String),
    Preview,
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectActive(Layout),
    SetLayoutName(Uuid, String),
    Renaming(String),
    AddLayout,
    RemoveLayout(Uuid),
    ToggleEditMode(Editing),
    CloneLayout(Uuid),
    Reorder(DragEvent),
}

pub struct LayoutManager {
    pub layouts: HashMap<Uuid, (Layout, Dashboard)>,
    pub active_layout: Layout,
    pub layout_order: Vec<Uuid>,
    edit_mode: Editing,
    is_locked: bool,
}

impl LayoutManager {
    pub fn new() -> Self {
        let mut layouts = HashMap::new();

        let layout1 = Layout {
            id: Uuid::new_v4(),
            name: "Layout 1".to_string(),
        };

        layouts.insert(layout1.id, (layout1.clone(), Dashboard::default()));

        LayoutManager {
            layouts,
            active_layout: layout1.clone(),
            layout_order: vec![layout1.id],
            edit_mode: Editing::None,
            is_locked: false,
        }
    }

    pub fn toggle_layout_lock(&mut self) {
        self.is_locked = !self.is_locked;
    }

    pub fn is_layout_locked(&self) -> bool {
        self.is_locked
    }

    fn generate_unique_layout_name(&self) -> String {
        let mut counter = 1;
        loop {
            let candidate = format!("Layout {counter}");
            if !self
                .layouts
                .values()
                .any(|(layout, _)| layout.name == candidate)
            {
                return candidate;
            }
            counter += 1;
        }
    }

    fn ensure_unique_name(&self, proposed_name: &str, current_id: Uuid) -> String {
        let mut counter = 2;
        let mut final_name = proposed_name.to_string();

        while self
            .layouts
            .values()
            .any(|(layout, _)| layout.id != current_id && layout.name == final_name)
        {
            final_name = format!("{proposed_name} ({counter})");
            counter += 1;
        }

        final_name.chars().take(20).collect()
    }

    pub fn iter_dashboards_mut(&mut self) -> impl Iterator<Item = &mut Dashboard> {
        self.layouts.values_mut().map(|(_, d)| d)
    }

    pub fn get_mut_dashboard(&mut self, id: &Uuid) -> Option<&mut Dashboard> {
        self.layouts.get_mut(id).map(|(_, d)| d)
    }

    pub fn get_dashboard(&self, id: &Uuid) -> Option<&Dashboard> {
        self.layouts.get(id).map(|(_, d)| d)
    }

    pub fn get_active_dashboard(&self) -> Option<&Dashboard> {
        self.get_dashboard(&self.active_layout.id)
    }

    pub fn get_active_dashboard_mut(&mut self) -> Option<&mut Dashboard> {
        let id = self.active_layout.id;
        self.get_mut_dashboard(&id)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectActive(layout) => {
                self.active_layout = layout;
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
                let new_layout = Layout {
                    id: Uuid::new_v4(),
                    name: self.generate_unique_layout_name(),
                };

                self.layout_order.push(new_layout.id);
                self.layouts
                    .insert(new_layout.id, (new_layout.clone(), Dashboard::default()));

                self.active_layout = new_layout;
            }
            Message::RemoveLayout(id) => {
                if self.active_layout.id == id {
                    return Task::none();
                } else {
                    self.layouts.remove(&id);
                    self.layout_order.retain(|layout_id| *layout_id != id);
                }

                self.edit_mode = Editing::Preview;
            }
            Message::SetLayoutName(id, new_name) => {
                let unique_name = self.ensure_unique_name(&new_name, id);
                let updated_layout = Layout {
                    id,
                    name: unique_name,
                };

                if let Some((_, dashboard)) = self.layouts.remove(&id) {
                    self.layouts
                        .insert(updated_layout.id, (updated_layout.clone(), dashboard));

                    if self.active_layout.id == id {
                        self.active_layout = updated_layout;
                    }
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
                if let Some((layout, dashboard)) = self.layouts.get(&id) {
                    let new_id = Uuid::new_v4();
                    let new_layout = Layout {
                        id: new_id,
                        name: self.ensure_unique_name(&layout.name, new_id),
                    };

                    let ser_dashboard = data::Dashboard::from(dashboard);

                    let mut popout_windows: Vec<(Configuration<PaneState>, WindowSpec)> =
                        Vec::new();

                    for (pane, window_spec) in &ser_dashboard.popout {
                        let configuration = configuration(pane.clone());
                        popout_windows.push((configuration, *window_spec));
                    }

                    let dashboard = Dashboard::from_config(
                        configuration(ser_dashboard.pane.clone()),
                        popout_windows,
                        ser_dashboard.trade_fetch_enabled,
                    );

                    self.layout_order.push(new_layout.id);
                    self.layouts
                        .insert(new_layout.id, (new_layout.clone(), dashboard));
                }
            }
            Message::Reorder(event) => match event {
                DragEvent::Picked { .. } => {}
                DragEvent::Dropped {
                    index,
                    target_index,
                    drop_position,
                } => match drop_position {
                    DropPosition::Before | DropPosition::After => {
                        if target_index != index && target_index != index + 1 {
                            let item = self.layout_order.remove(index);
                            let insert_index = if index < target_index {
                                target_index - 1
                            } else {
                                target_index
                            };
                            self.layout_order.insert(insert_index, item);
                        }
                    }
                    DropPosition::Swap => {
                        if target_index != index {
                            self.layout_order.swap(index, target_index);
                        }
                    }
                },
                DragEvent::Canceled { .. } => {}
            },
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut content = column![].spacing(8);

        let edit_btn = {
            match &self.edit_mode {
                Editing::None => {
                    button(text("Edit")).on_press(Message::ToggleEditMode(Editing::Preview))
                }
                _ => button(get_icon_text(style::Icon::Return, 12))
                    .on_press(Message::ToggleEditMode(Editing::Preview)),
            }
        };

        content = content.push(
            row![
                Space::with_width(iced::Length::Fill),
                row![
                    tooltip(
                        button("i").style(move |theme, status| style::button::modifier(theme, status, true)),
                        Some("- Drag & drop to reorder layouts\n- Layouts won't be saved if app exits abruptly"),
                        TooltipPosition::Top,
                    ),
                    edit_btn,
                ].spacing(4),
            ]
        );

        let layout_btn =
            |layout: &Layout, on_press: Option<Message>| create_layout_button(layout, on_press);

        let mut layouts_column = column_drag::Column::new()
            .on_drag(Message::Reorder)
            .spacing(4);

        for id in &self.layout_order {
            if let Some((layout, _)) = self.layouts.get(id) {
                let mut layout_row = row![].padding(4).height(iced::Length::Fixed(34.0));

                if self.active_layout.id == layout.id {
                    let color_column = container(column![])
                        .height(iced::Length::Fill)
                        .width(iced::Length::Fixed(2.0))
                        .style(style::layout_card_bar);

                    layout_row = layout_row
                        .push(container(color_column).padding(padding::left(8).bottom(4).top(4)));
                }

                match &self.edit_mode {
                    Editing::ConfirmingDelete(delete_id) => {
                        if *delete_id == layout.id {
                            let (confirm_btn, cancel_btn) =
                                self.create_confirm_delete_buttons(layout);

                            layout_row = layout_row
                                .push(center(text(format!("Delete {}?", layout.name)).size(12)))
                                .push(confirm_btn)
                                .push(cancel_btn);
                        } else {
                            layout_row = layout_row.push(layout_btn(layout, None));
                        }
                    }
                    Editing::Renaming(id, name) => {
                        if *id == layout.id {
                            let input_box = text_input("New layout name", name)
                                .on_input(|new_name| Message::Renaming(new_name.clone()))
                                .on_submit(Message::SetLayoutName(*id, name.clone()));

                            let (_, cancel_btn) = self.create_confirm_delete_buttons(layout);

                            layout_row = layout_row
                                .push(center(input_box).padding(padding::left(4)))
                                .push(cancel_btn);
                        } else {
                            layout_row = layout_row.push(layout_btn(layout, None));
                        }
                    }
                    Editing::Preview => {
                        layout_row = layout_row
                            .push(layout_btn(layout, None))
                            .push(tooltip(
                                button(get_icon_text(style::Icon::Clone, 12))
                                    .on_press(Message::CloneLayout(layout.id))
                                    .style(move |t, s| style::button::transparent(t, s, true)),
                                Some("Clone layout"),
                                TooltipPosition::Top,
                            ))
                            .push(self.create_rename_button(layout));

                        if self.active_layout.id != layout.id {
                            layout_row = layout_row.push(self.create_delete_button(layout.id));
                        }
                    }
                    _ => {
                        layout_row = layout_row.push(layout_btn(
                            layout,
                            if self.active_layout.id == layout.id {
                                None
                            } else {
                                Some(Message::SelectActive(layout.clone()))
                            },
                        ));
                    }
                }

                layouts_column =
                    layouts_column.push(container(layout_row).style(style::layout_row_container));
            }
        }

        content = content.push(layouts_column);

        if self.edit_mode != Editing::None {
            content = content.push(
                container(
                    button(text("Add layout"))
                        .style(move |t, s| style::button::transparent(t, s, false))
                        .width(iced::Length::Fill)
                        .on_press(Message::AddLayout),
                )
                .style(style::chart_modal),
            );
        };

        scrollable::Scrollable::with_direction(
            content.padding(padding::left(8).right(8)),
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        )
        .into()
    }

    fn create_delete_button<'a>(&self, layout_id: Uuid) -> Element<'a, Message> {
        if self.active_layout.id == layout_id {
            tooltip(
                create_icon_button(
                    style::Icon::TrashBin,
                    12,
                    |theme, status| style::button::layout_name(theme, *status),
                    None,
                ),
                Some("Can't delete active layout"),
                TooltipPosition::Right,
            )
        } else {
            create_icon_button(
                style::Icon::TrashBin,
                12,
                |theme, status| style::button::layout_name(theme, *status),
                Some(Message::ToggleEditMode(Editing::ConfirmingDelete(
                    layout_id,
                ))),
            )
            .into()
        }
    }

    fn create_rename_button<'a>(&self, layout: &Layout) -> button::Button<'a, Message> {
        create_icon_button(
            style::Icon::Edit,
            12,
            |theme, status| style::button::layout_name(theme, *status),
            Some(Message::ToggleEditMode(Editing::Renaming(
                layout.id,
                layout.name.clone(),
            ))),
        )
    }

    fn create_confirm_delete_buttons<'a>(
        &'a self,
        layout: &Layout,
    ) -> (button::Button<'a, Message>, button::Button<'a, Message>) {
        let confirm = create_icon_button(
            style::Icon::Checkmark,
            12,
            |theme, status| style::button::confirm(theme, *status, true),
            Some(Message::RemoveLayout(layout.id)),
        );

        let cancel = create_icon_button(
            style::Icon::Close,
            12,
            |theme, status| style::button::cancel(theme, *status, true),
            Some(Message::ToggleEditMode(Editing::Preview)),
        );

        (confirm, cancel)
    }
}

fn create_layout_button<'a>(layout: &Layout, on_press: Option<Message>) -> Element<'a, Message> {
    let mut layout_btn = button(text(layout.name.clone()))
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
    let mut btn =
        button(get_icon_text(icon, size)).style(move |theme, status| style_fn(theme, &status));

    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }

    btn
}

pub struct SavedState {
    pub layout_manager: LayoutManager,
    pub main_window: Option<WindowSpec>,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub scale_factor: data::ScaleFactor,
    pub timezone: data::UserTimezone,
    pub sidebar: data::Sidebar,
    pub theme: data::Theme,
}

impl Default for SavedState {
    fn default() -> Self {
        SavedState {
            layout_manager: LayoutManager::new(),
            main_window: None,
            favorited_tickers: Vec::new(),
            scale_factor: data::ScaleFactor::default(),
            timezone: UserTimezone::default(),
            sidebar: data::Sidebar::default(),
            theme: data::Theme::default(),
        }
    }
}

impl From<&Dashboard> for data::Dashboard {
    fn from(dashboard: &Dashboard) -> Self {
        use pane_grid::Node;

        fn from_layout(panes: &pane_grid::State<PaneState>, node: pane_grid::Node) -> data::Pane {
            match node {
                Node::Split {
                    axis, ratio, a, b, ..
                } => data::Pane::Split {
                    axis: match axis {
                        pane_grid::Axis::Horizontal => Axis::Horizontal,
                        pane_grid::Axis::Vertical => Axis::Vertical,
                    },
                    ratio,
                    a: Box::new(from_layout(panes, *a)),
                    b: Box::new(from_layout(panes, *b)),
                },
                Node::Pane(pane) => panes
                    .get(pane)
                    .map_or(data::Pane::Starter, data::Pane::from),
            }
        }

        let main_window_layout = dashboard.panes.layout().clone();

        let popouts_layout: Vec<(data::Pane, WindowSpec)> = dashboard
            .popout
            .iter()
            .map(|(_, (pane, spec))| (from_layout(pane, pane.layout().clone()), *spec))
            .collect();

        data::Dashboard {
            pane: from_layout(&dashboard.panes, main_window_layout),
            popout: {
                popouts_layout
                    .iter()
                    .map(|(pane, window_spec)| (pane.clone(), *window_spec))
                    .collect()
            },
            trade_fetch_enabled: dashboard.trade_fetch_enabled,
        }
    }
}

impl From<&PaneState> for data::Pane {
    fn from(pane: &PaneState) -> Self {
        let streams = pane.streams.clone();

        match &pane.content {
            PaneContent::Starter => data::Pane::Starter,
            PaneContent::Heatmap(chart, indicators) => data::Pane::HeatmapChart {
                layout: chart.get_chart_layout(),
                stream_type: streams,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Footprint(chart, indicators) => data::Pane::FootprintChart {
                layout: chart.get_chart_layout(),
                stream_type: streams,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Candlestick(chart, indicators) => data::Pane::CandlestickChart {
                layout: chart.get_chart_layout(),
                stream_type: streams,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::TimeAndSales(_) => data::Pane::TimeAndSales {
                stream_type: streams,
                settings: pane.settings,
            },
        }
    }
}

fn configuration(pane: data::Pane) -> Configuration<PaneState> {
    match pane {
        data::Pane::Split { axis, ratio, a, b } => Configuration::Split {
            axis: match axis {
                Axis::Horizontal => pane_grid::Axis::Horizontal,
                Axis::Vertical => pane_grid::Axis::Vertical,
            },
            ratio,
            a: Box::new(configuration(*a)),
            b: Box::new(configuration(*b)),
        },
        data::Pane::Starter => Configuration::Pane(PaneState::new()),
        data::Pane::CandlestickChart {
            layout,
            stream_type,
            settings,
            indicators,
        } => {
            if let Some(ticker_info) = settings.ticker_info {
                let basis = settings
                    .selected_basis
                    .unwrap_or(Basis::Time(Timeframe::M15.into()));

                Configuration::Pane(PaneState::from_config(
                    PaneContent::Candlestick(
                        CandlestickChart::new(
                            layout,
                            basis,
                            &[],
                            vec![],
                            ticker_info.min_ticksize,
                            &indicators,
                            settings.ticker_info,
                        ),
                        indicators,
                    ),
                    stream_type,
                    settings,
                ))
            } else {
                log::info!("Skipping a CandlestickChart initialization due to missing ticker info");
                Configuration::Pane(PaneState::new())
            }
        }
        data::Pane::FootprintChart {
            layout,
            stream_type,
            settings,
            indicators,
        } => {
            if let Some(ticker_info) = settings.ticker_info {
                let tick_size = settings
                    .tick_multiply
                    .unwrap_or(TickMultiplier(50))
                    .multiply_with_min_tick_size(ticker_info);
                let basis = settings
                    .selected_basis
                    .unwrap_or(Basis::Time(Timeframe::M5.into()));

                Configuration::Pane(PaneState::from_config(
                    PaneContent::Footprint(
                        FootprintChart::new(
                            layout,
                            basis,
                            tick_size,
                            &[],
                            vec![],
                            &indicators,
                            settings.ticker_info,
                        ),
                        indicators,
                    ),
                    stream_type,
                    settings,
                ))
            } else {
                log::info!("Skipping a FootprintChart initialization due to missing ticker info");
                Configuration::Pane(PaneState::new())
            }
        }
        data::Pane::HeatmapChart {
            layout,
            stream_type,
            settings,
            indicators,
        } => {
            if let Some(ticker_info) = settings.ticker_info {
                let tick_size = settings
                    .tick_multiply
                    .unwrap_or(TickMultiplier(10))
                    .multiply_with_min_tick_size(ticker_info);

                let config = settings.visual_config.and_then(|cfg| cfg.heatmap());

                Configuration::Pane(PaneState::from_config(
                    PaneContent::Heatmap(
                        HeatmapChart::new(
                            layout,
                            tick_size,
                            100,
                            &indicators,
                            settings.ticker_info,
                            config,
                        ),
                        indicators,
                    ),
                    stream_type,
                    settings,
                ))
            } else {
                log::info!("Skipping a HeatmapChart initialization due to missing ticker info");
                Configuration::Pane(PaneState::new())
            }
        }
        data::Pane::TimeAndSales {
            stream_type,
            settings,
        } => {
            if settings.ticker_info.is_none() {
                log::info!("Skipping a TimeAndSales initialization due to missing ticker info");
                return Configuration::Pane(PaneState::new());
            }

            let config = settings.visual_config.and_then(|cfg| cfg.time_and_sales());

            Configuration::Pane(PaneState::from_config(
                PaneContent::TimeAndSales(TimeAndSales::new(config, settings.ticker_info)),
                stream_type,
                settings,
            ))
        }
    }
}

pub fn load_saved_state() -> SavedState {
    match data::read_from_file(data::SAVED_STATE_PATH) {
        Ok(state) => {
            let mut de_layouts: Vec<(String, Dashboard)> = vec![];

            for layout in &state.layout_manager.layouts {
                let mut popout_windows = Vec::new();

                for (pane, window_spec) in &layout.dashboard.popout {
                    let configuration = configuration(pane.clone());
                    popout_windows.push((configuration, *window_spec));
                }

                let dashboard = Dashboard::from_config(
                    configuration(layout.dashboard.pane.clone()),
                    popout_windows,
                    layout.dashboard.trade_fetch_enabled,
                );

                de_layouts.push((layout.name.clone(), dashboard));
            }

            let layout_manager: LayoutManager = {
                let mut layouts = HashMap::new();

                let active_layout = Layout {
                    id: Uuid::new_v4(),
                    name: state.layout_manager.active_layout.clone(),
                };

                let mut layout_order = vec![];

                for (name, dashboard) in de_layouts {
                    let layout = Layout {
                        id: {
                            if name == active_layout.name {
                                active_layout.id
                            } else {
                                Uuid::new_v4()
                            }
                        },
                        name,
                    };

                    layout_order.push(layout.id);
                    layouts.insert(layout.id, (layout.clone(), dashboard));
                }

                LayoutManager {
                    layouts,
                    active_layout,
                    layout_order,
                    edit_mode: Editing::None,
                    is_locked: false,
                }
            };

            SavedState {
                theme: state.selected_theme,
                layout_manager,
                favorited_tickers: state.favorited_tickers,
                main_window: state.main_window,
                timezone: state.timezone,
                sidebar: state.sidebar,
                scale_factor: state.scale_factor,
            }
        }
        Err(e) => {
            log::error!(
                "Failed to load/find layout state: {}. Starting with a new layout.",
                e
            );

            SavedState::default()
        }
    }
}
