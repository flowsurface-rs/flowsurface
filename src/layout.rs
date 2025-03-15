use crate::aggr::TickMultiplier;
use crate::charts::{
    ChartBasis,
    candlestick::CandlestickChart,
    footprint::FootprintChart,
    heatmap::HeatmapChart,
    indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator},
    timeandsales::TimeAndSales,
};
use crate::screen::{
    UserTimezone,
    dashboard::{Dashboard, PaneContent, PaneSettings, PaneState},
};
use crate::style::get_icon_text;
use crate::widget::column_drag::{self, DragEvent, DropPosition};
use crate::{screen, style, tooltip};
use exchanges::{
    Ticker, Timeframe,
    adapter::{Exchange, StreamType},
};

use chrono::NaiveDate;
use iced::widget::pane_grid::{self, Configuration};
use iced::widget::{
    Space, button, center, column, container, row, scrollable, text, text_input,
    tooltip::Position as TooltipPosition,
};
use iced::{Element, Point, Size, Task, Theme, padding};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::vec;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableLayout {
    pub name: String,
    pub dashboard: SerializableDashboard,
}

impl Default for SerializableLayout {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            dashboard: SerializableDashboard::default(),
        }
    }
}

impl From<(Layout, Dashboard)> for SerializableLayout {
    fn from((layout, dashboard): (Layout, Dashboard)) -> Self {
        Self {
            name: layout.name,
            dashboard: SerializableDashboard::from(&dashboard),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableLayouts {
    pub layouts: Vec<SerializableLayout>,
    pub active_layout: String,
}

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
        }
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

    fn ensure_unique_name(&self, proposed_name: String, current_id: Uuid) -> String {
        let mut counter = 2;
        let mut final_name = proposed_name.clone();

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
                let unique_name = self.ensure_unique_name(new_name, id);
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
                        name: self.ensure_unique_name(layout.name.clone(), new_id),
                    };

                    let ser_dashboard = SerializableDashboard::from(dashboard);

                    let mut popout_windows: Vec<(Configuration<PaneState>, (Point, Size))> =
                        Vec::new();

                    for (pane, pos, size) in &ser_dashboard.popout {
                        let configuration = configuration(pane.clone());
                        popout_windows.push((
                            configuration,
                            (Point::new(pos.0, pos.1), Size::new(size.0, size.1)),
                        ));
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
                        button("i").style(move |theme, status| style::button_modifier(theme, status, true)),
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
                                    .style(move |t, s| style::button_transparent(t, s, true)),
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
                        .style(move |t, s| style::button_transparent(t, s, false))
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
                    |theme, status| style::button_layout_name(theme, *status),
                    None,
                ),
                Some("Can't delete active layout"),
                TooltipPosition::Right,
            )
        } else {
            create_icon_button(
                style::Icon::TrashBin,
                12,
                |theme, status| style::button_layout_name(theme, *status),
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
            |theme, status| style::button_layout_name(theme, *status),
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
            |theme, status| style::button_confirm(theme, *status, true),
            Some(Message::RemoveLayout(layout.id)),
        );

        let cancel = create_icon_button(
            style::Icon::Close,
            12,
            |theme, status| style::button_cancel(theme, *status, true),
            Some(Message::ToggleEditMode(Editing::Preview)),
        );

        (confirm, cancel)
    }
}

fn create_layout_button<'a>(layout: &Layout, on_press: Option<Message>) -> Element<'a, Message> {
    let mut layout_btn = button(text(layout.name.clone()))
        .width(iced::Length::Fill)
        .style(style::button_layout_name);

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

#[derive(Default, Debug, Clone, PartialEq, Copy, Deserialize, Serialize)]
pub enum Sidebar {
    #[default]
    Left,
    Right,
}

impl std::fmt::Display for Sidebar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sidebar::Left => write!(f, "Left"),
            Sidebar::Right => write!(f, "Right"),
        }
    }
}

pub struct SavedState {
    pub layout_manager: LayoutManager,
    pub selected_theme: SerializableTheme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub present_mode: screen::PresentMode,
    pub scale_factor: ScaleFactor,
}

impl Default for SavedState {
    fn default() -> Self {
        SavedState {
            layout_manager: LayoutManager::new(),
            selected_theme: SerializableTheme::default(),
            favorited_tickers: Vec::new(),
            window_size: None,
            window_position: None,
            timezone: UserTimezone::default(),
            sidebar: Sidebar::default(),
            present_mode: screen::PresentMode::default(),
            scale_factor: ScaleFactor::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SerializableTheme {
    pub theme: Theme,
}

impl Default for SerializableTheme {
    fn default() -> Self {
        Self {
            theme: Theme::Custom(style::custom_theme().into()),
        }
    }
}

impl Serialize for SerializableTheme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let theme_str = match self.theme {
            Theme::Ferra => "ferra",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Dracula => "dracula",
            Theme::Nord => "nord",
            Theme::SolarizedLight => "solarized_light",
            Theme::SolarizedDark => "solarized_dark",
            Theme::GruvboxLight => "gruvbox_light",
            Theme::GruvboxDark => "gruvbox_dark",
            Theme::CatppuccinLatte => "catppuccino_latte",
            Theme::CatppuccinFrappe => "catppuccino_frappe",
            Theme::CatppuccinMacchiato => "catppuccino_macchiato",
            Theme::CatppuccinMocha => "catppuccino_mocha",
            Theme::TokyoNight => "tokyo_night",
            Theme::TokyoNightStorm => "tokyo_night_storm",
            Theme::TokyoNightLight => "tokyo_night_light",
            Theme::KanagawaWave => "kanagawa_wave",
            Theme::KanagawaDragon => "kanagawa_dragon",
            Theme::KanagawaLotus => "kanagawa_lotus",
            Theme::Moonfly => "moonfly",
            Theme::Nightfly => "nightfly",
            Theme::Oxocarbon => "oxocarbon",
            Theme::Custom(_) => "flowsurface",
        };
        theme_str.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SerializableTheme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let theme_str = String::deserialize(deserializer)?;
        let theme = match theme_str.as_str() {
            "ferra" => Theme::Ferra,
            "dark" => Theme::Dark,
            "light" => Theme::Light,
            "dracula" => Theme::Dracula,
            "nord" => Theme::Nord,
            "solarized_light" => Theme::SolarizedLight,
            "solarized_dark" => Theme::SolarizedDark,
            "gruvbox_light" => Theme::GruvboxLight,
            "gruvbox_dark" => Theme::GruvboxDark,
            "catppuccino_latte" => Theme::CatppuccinLatte,
            "catppuccino_frappe" => Theme::CatppuccinFrappe,
            "catppuccino_macchiato" => Theme::CatppuccinMacchiato,
            "catppuccino_mocha" => Theme::CatppuccinMocha,
            "tokyo_night" => Theme::TokyoNight,
            "tokyo_night_storm" => Theme::TokyoNightStorm,
            "tokyo_night_light" => Theme::TokyoNightLight,
            "kanagawa_wave" => Theme::KanagawaWave,
            "kanagawa_dragon" => Theme::KanagawaDragon,
            "kanagawa_lotus" => Theme::KanagawaLotus,
            "moonfly" => Theme::Moonfly,
            "nightfly" => Theme::Nightfly,
            "oxocarbon" => Theme::Oxocarbon,
            "flowsurface" => SerializableTheme::default().theme,
            _ => return Err(serde::de::Error::custom("Invalid theme")),
        };
        Ok(SerializableTheme { theme })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerializableState {
    pub layout_manager: SerializableLayouts,
    pub selected_theme: SerializableTheme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub present_mode: screen::PresentMode,
    pub scale_factor: ScaleFactor,
}

impl SerializableState {
    pub fn from_parts(
        layout_manager: SerializableLayouts,
        selected_theme: Theme,
        favorited_tickers: Vec<(Exchange, Ticker)>,
        size: Option<Size>,
        position: Option<Point>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        present_mode: screen::PresentMode,
        scale_factor: ScaleFactor,
    ) -> Self {
        SerializableState {
            layout_manager,
            selected_theme: SerializableTheme {
                theme: selected_theme,
            },
            favorited_tickers,
            window_size: size.map(|s| (s.width, s.height)),
            window_position: position.map(|p| (p.x, p.y)),
            timezone,
            sidebar,
            present_mode,
            scale_factor,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerializableDashboard {
    pub pane: SerializablePane,
    pub popout: Vec<(SerializablePane, (f32, f32), (f32, f32))>,
    pub trade_fetch_enabled: bool,
}

impl<'a> From<&'a Dashboard> for SerializableDashboard {
    fn from(dashboard: &'a Dashboard) -> Self {
        use pane_grid::Node;

        fn from_layout(
            panes: &pane_grid::State<PaneState>,
            node: pane_grid::Node,
        ) -> SerializablePane {
            match node {
                Node::Split {
                    axis, ratio, a, b, ..
                } => SerializablePane::Split {
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
                    .map_or(SerializablePane::Starter, SerializablePane::from),
            }
        }

        let main_window_layout = dashboard.panes.layout().clone();

        let popouts_layout: Vec<(SerializablePane, (Point, Size))> = dashboard
            .popout
            .iter()
            .map(|(_, (pane, specs))| (from_layout(pane, pane.layout().clone()), *specs))
            .collect();

        SerializableDashboard {
            pane: from_layout(&dashboard.panes, main_window_layout),
            popout: {
                popouts_layout
                    .iter()
                    .map(|(pane, (pos, size))| {
                        (pane.clone(), (pos.x, pos.y), (size.width, size.height))
                    })
                    .collect()
            },
            trade_fetch_enabled: dashboard.trade_fetch_enabled,
        }
    }
}

impl Default for SerializableDashboard {
    fn default() -> Self {
        Self {
            pane: SerializablePane::Starter,
            popout: vec![],
            trade_fetch_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerializableChartData {
    pub crosshair: bool,
    pub indicators_split: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SerializablePane {
    Split {
        axis: Axis,
        ratio: f32,
        a: Box<SerializablePane>,
        b: Box<SerializablePane>,
    },
    Starter,
    HeatmapChart {
        layout: SerializableChartData,
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
        indicators: Vec<HeatmapIndicator>,
    },
    FootprintChart {
        layout: SerializableChartData,
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
        indicators: Vec<FootprintIndicator>,
    },
    CandlestickChart {
        layout: SerializableChartData,
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
        indicators: Vec<CandlestickIndicator>,
    },
    TimeAndSales {
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
    },
}

impl From<&PaneState> for SerializablePane {
    fn from(pane: &PaneState) -> Self {
        let pane_stream = pane.stream.clone();

        match &pane.content {
            PaneContent::Starter => SerializablePane::Starter,
            PaneContent::Heatmap(chart, indicators) => SerializablePane::HeatmapChart {
                layout: chart.get_chart_layout(),
                stream_type: pane_stream,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Footprint(chart, indicators) => SerializablePane::FootprintChart {
                layout: chart.get_chart_layout(),
                stream_type: pane_stream,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Candlestick(chart, indicators) => SerializablePane::CandlestickChart {
                layout: chart.get_chart_layout(),
                stream_type: pane_stream,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::TimeAndSales(_) => SerializablePane::TimeAndSales {
                stream_type: pane_stream,
                settings: pane.settings,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct ScaleFactor(f64);

impl Default for ScaleFactor {
    fn default() -> Self {
        Self(1.0)
    }
}

impl From<f64> for ScaleFactor {
    fn from(value: f64) -> Self {
        ScaleFactor(value.clamp(0.8, 1.8))
    }
}

impl From<ScaleFactor> for f64 {
    fn from(value: ScaleFactor) -> Self {
        value.0
    }
}

fn configuration(pane: SerializablePane) -> Configuration<PaneState> {
    match pane {
        SerializablePane::Split { axis, ratio, a, b } => Configuration::Split {
            axis: match axis {
                Axis::Horizontal => pane_grid::Axis::Horizontal,
                Axis::Vertical => pane_grid::Axis::Vertical,
            },
            ratio,
            a: Box::new(configuration(*a)),
            b: Box::new(configuration(*b)),
        },
        SerializablePane::Starter => {
            Configuration::Pane(PaneState::new(vec![], PaneSettings::default()))
        }
        SerializablePane::CandlestickChart {
            layout,
            stream_type,
            settings,
            indicators,
        } => {
            if let Some(ticker_info) = settings.ticker_info {
                let basis = settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M15.into()));

                Configuration::Pane(PaneState::from_config(
                    PaneContent::Candlestick(
                        CandlestickChart::new(
                            layout,
                            basis,
                            vec![],
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
                Configuration::Pane(PaneState::new(vec![], PaneSettings::default()))
            }
        }
        SerializablePane::FootprintChart {
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
                    .unwrap_or(ChartBasis::Time(Timeframe::M5.into()));

                Configuration::Pane(PaneState::from_config(
                    PaneContent::Footprint(
                        FootprintChart::new(
                            layout,
                            basis,
                            tick_size,
                            vec![],
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
                Configuration::Pane(PaneState::new(vec![], PaneSettings::default()))
            }
        }
        SerializablePane::HeatmapChart {
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
                Configuration::Pane(PaneState::new(vec![], PaneSettings::default()))
            }
        }
        SerializablePane::TimeAndSales {
            stream_type,
            settings,
        } => {
            let config = settings.visual_config.and_then(|cfg| cfg.time_and_sales());

            Configuration::Pane(PaneState::from_config(
                PaneContent::TimeAndSales(TimeAndSales::new(config)),
                stream_type,
                settings,
            ))
        }
    }
}

pub fn load_saved_state(file_path: &str) -> SavedState {
    match read_from_file(file_path) {
        Ok(state) => {
            let mut de_layouts: Vec<(String, Dashboard)> = vec![];

            for layout in &state.layout_manager.layouts {
                let mut popout_windows: Vec<(Configuration<PaneState>, (Point, Size))> = Vec::new();

                for (pane, pos, size) in &layout.dashboard.popout {
                    let configuration = configuration(pane.clone());
                    popout_windows.push((
                        configuration,
                        (Point::new(pos.0, pos.1), Size::new(size.0, size.1)),
                    ));
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
                }
            };

            SavedState {
                selected_theme: state.selected_theme,
                layout_manager,
                favorited_tickers: state.favorited_tickers,
                window_size: state.window_size,
                window_position: state.window_position,
                timezone: state.timezone,
                sidebar: state.sidebar,
                present_mode: state.present_mode,
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

pub fn write_json_to_file(json: &str, file_name: &str) -> std::io::Result<()> {
    let path = get_data_path(file_name);
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn read_from_file(file_name: &str) -> Result<SerializableState, Box<dyn std::error::Error>> {
    let path = get_data_path(file_name);
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    Ok(serde_json::from_str(&contents)?)
}

pub fn get_data_path(path_name: &str) -> PathBuf {
    if let Ok(path) = std::env::var("FLOWSURFACE_DATA_PATH") {
        PathBuf::from(path)
    } else {
        let data_dir = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
        data_dir.join("flowsurface").join(path_name)
    }
}

pub fn cleanup_old_data() -> usize {
    let data_path = get_data_path("market_data/binance/data/futures/um/daily/aggTrades");

    if !data_path.exists() {
        log::warn!("Data path {:?} does not exist, skipping cleanup", data_path);
        return 0;
    }

    let re = Regex::new(r".*-(\d{4}-\d{2}-\d{2})\.zip$").expect("Cleanup regex pattern is valid");
    let today = chrono::Local::now().date_naive();
    let mut deleted_files = Vec::new();

    let entries = match std::fs::read_dir(data_path) {
        Ok(entries) => entries,
        Err(e) => {
            log::error!("Failed to read data directory: {}", e);
            return 0;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let symbol_dir = match std::fs::read_dir(entry.path()) {
            Ok(dir) => dir,
            Err(e) => {
                log::error!("Failed to read symbol directory {:?}: {}", entry.path(), e);
                continue;
            }
        };

        for file in symbol_dir.filter_map(Result::ok) {
            let path = file.path();
            let filename = match path.to_str() {
                Some(name) => name,
                None => continue,
            };

            if let Some(cap) = re.captures(filename) {
                if let Ok(file_date) = NaiveDate::parse_from_str(&cap[1], "%Y-%m-%d") {
                    let days_old = today.signed_duration_since(file_date).num_days();
                    if days_old > 4 {
                        if let Err(e) = std::fs::remove_file(&path) {
                            log::error!("Failed to remove old file {}: {}", filename, e);
                        } else {
                            deleted_files.push(filename.to_string());
                            log::info!("Removed old file: {}", filename);
                        }
                    }
                }
            }
        }
    }

    log::info!(
        "File cleanup completed. Deleted {} files",
        deleted_files.len()
    );
    deleted_files.len()
}
