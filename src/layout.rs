use iced::widget::{button, column, row, text, text_input, Space};
use regex::Regex;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use iced::widget::pane_grid::{self, Configuration};
use iced::{Element, Point, Size, Task, Theme};
use uuid::Uuid;

use crate::charts::candlestick::CandlestickChart;
use crate::charts::footprint::FootprintChart;
use crate::charts::heatmap::HeatmapChart;
use crate::charts::timeandsales::TimeAndSales;
use crate::charts::indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator};
use crate::data_providers::{Exchange, StreamType, TickMultiplier, Ticker, Timeframe};
use crate::screen::{UserTimezone, dashboard::{Dashboard, PaneContent, PaneSettings, PaneState}};
use crate::{screen, style};

use std::collections::HashMap;
use std::io::{Read, Write};
use std::fs::File;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Message {
    SelectActive(Uuid),
    RenameLayout(Uuid, String),
    ToggleRenaming(Uuid),
    Renaming(String),
    AddLayout,
    RemoveLayout(Uuid),
    ToggleEditMode(Editing),
}

#[derive(Debug, Clone)]
pub enum Editing {
    Renaming(Uuid, String),
    Deleting,
    None,
}

pub struct Layouts {
    pub layouts: HashMap<Layout, Dashboard>,
    pub selected_layout: Layout,
    edit_mode: Editing,
}

impl Layouts {
    pub fn new() -> Self {
        let mut layouts = HashMap::new();

        let layout1 = Layout {
            id: Uuid::new_v4(),
            name: "TEST".to_string(),
        };

        layouts.insert(layout1.clone(), Dashboard::default());
    
        Layouts { 
            layouts, 
            selected_layout: layout1, 
            edit_mode: Editing::None, 
        }
    }

    pub fn get_layout_name(&self, id: &Uuid) -> Option<String> {
        self.layouts
            .iter()
            .find(|(layout, _)| &layout.id == id)
            .map(|(layout, _)| layout.name.clone())
    }

    pub fn remove_layout(&mut self, id: &Uuid) {
        let layout = self.layouts.keys().find(|l| l.id == *id).cloned();
        if let Some(layout) = layout {
            self.layouts.remove(&layout);
        }
    }

    pub fn get_dashboard(&self, id: &Uuid) -> Option<&Dashboard> {
        self.layouts
            .iter()
            .find(|(layout, _)| &layout.id == id)
            .map(|(_, dashboard)| dashboard)
    }

    pub fn get_mut_dashboard(&mut self, id: &Uuid) -> Option<&mut Dashboard> {
        self.layouts
            .iter_mut()
            .find(|(layout, _)| &layout.id == id)
            .map(|(_, dashboard)| dashboard)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectActive(id) => {
                self.selected_layout = Layout {
                    id,
                    name: self.get_layout_name(&id).unwrap_or_default(),
                };
            }
            Message::ToggleEditMode(edit_mode) => {
                match (&edit_mode, &self.edit_mode) {
                    (Editing::Deleting, Editing::Deleting) => {
                        self.edit_mode = Editing::None;
                    }
                    _ => {
                        self.edit_mode = edit_mode;
                    }
                }
            }
            Message::ToggleRenaming(id) => {
                self.edit_mode = match self.edit_mode {
                    Editing::Renaming(renaming_id, _) if renaming_id == id => Editing::None,
                    _ => Editing::Renaming(id, self.get_layout_name(&id).unwrap_or_default()),
                };
            }
            Message::AddLayout => {
                let new_layout = Layout {
                    id: Uuid::new_v4(),
                    name: "New layout".to_string(),
                };

                self.layouts.insert(new_layout.clone(), Dashboard::default());
            }
            Message::RemoveLayout(id) => {
                self.remove_layout(&id);
                self.edit_mode = Editing::None;
            }
            Message::RenameLayout(id, new_name) => {
                let updated_layout = Layout {
                    id,
                    name: new_name,
                };
                
                // Find the old layout and remove/insert with the new name
                if let Some(old_layout) = self.layouts.keys().find(|l| l.id == id).cloned() {
                    if let Some(dashboard) = self.layouts.remove(&old_layout) {
                        self.layouts.insert(updated_layout.clone(), dashboard);
                        
                        // Update selected_layout if it was the renamed one
                        if self.selected_layout.id == id {
                            self.selected_layout = updated_layout;
                        }
                    }
                }

                self.edit_mode = Editing::None;
            }
            Message::Renaming(renaming) => {
                self.edit_mode = match self.edit_mode {
                    Editing::Renaming(id, _) => Editing::Renaming(id, renaming),
                    _ => Editing::None,
                };
            }
        }

        Task::none()
    }

    pub fn view<'a>(&'a self, _bounds: iced::Size) -> Element<'a, Message> {
        let mut layouts = column![].spacing(8);

        layouts = layouts.push(
            row![
                text(&self.selected_layout.name).size(14),
                Space::with_width(iced::Length::Fill),
                button(text("-"))
                    .on_press(Message::ToggleEditMode(
                        Editing::Deleting
                    )),
                button(text("+"))
                    .on_press(Message::AddLayout)
            ].spacing(4)
        );

        for (layout, _) in &self.layouts {
            let mut buttons = row![].spacing(4);

            match &self.edit_mode {
                Editing::Renaming(id, name) if *id == layout.id => {
                    let input = text_input(
                        "New name",
                        &name,
                    )
                    .on_submit(Message::RenameLayout(*id, name.clone())
                    )
                    .on_input(|new_name| {
                        Message::Renaming(new_name.clone())
                    });

                    buttons = buttons.push(input);

                    let cancel_button = button(
                        text("X")
                    ).on_press(Message::ToggleRenaming(layout.id));

                    buttons = buttons
                        .push(cancel_button);
                }
                Editing::Deleting => {
                    let debug_button = button(
                        text(layout.name.clone())
                    ).on_press(Message::SelectActive(layout.id));
        
                    buttons = buttons.push(debug_button);

                    let delete_button = button(
                        text("Delete")
                    ).on_press(Message::RemoveLayout(layout.id));

                    buttons = buttons.push(delete_button);
                }
                _ => {
                    let debug_button = button(
                        text(layout.name.clone())
                    ).on_press(Message::SelectActive(layout.id));
        
                    buttons = buttons.push(debug_button);

                    let rename_button = button(
                        text("Rename")
                    ).on_press(Message::ToggleRenaming(layout.id));

                    buttons = buttons.push(rename_button);
                }
            }

            layouts = layouts.push(buttons);
        }

        layouts.into()
    }
}

#[derive(Eq, Hash, Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum LayoutId {
    Layout1,
    Layout2,
    Layout3,
    Layout4,
}

impl std::fmt::Display for LayoutId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutId::Layout1 => write!(f, "Layout 1"),
            LayoutId::Layout2 => write!(f, "Layout 2"),
            LayoutId::Layout3 => write!(f, "Layout 3"),
            LayoutId::Layout4 => write!(f, "Layout 4"),
        }
    }
}

impl LayoutId {
    pub const ALL: [LayoutId; 4] = [
        LayoutId::Layout1,
        LayoutId::Layout2,
        LayoutId::Layout3,
        LayoutId::Layout4,
    ];
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
    pub layouts: HashMap<LayoutId, Dashboard>,
    pub selected_theme: SerializableTheme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub last_active_layout: LayoutId,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub present_mode: screen::PresentMode,
    pub scale_factor: ScaleFactor,
}

impl Default for SavedState {
    fn default() -> Self {
        let mut layouts = HashMap::new();
        layouts.insert(LayoutId::Layout1, Dashboard::default());
        layouts.insert(LayoutId::Layout2, Dashboard::default());
        layouts.insert(LayoutId::Layout3, Dashboard::default());
        layouts.insert(LayoutId::Layout4, Dashboard::default());

        SavedState {
            layouts,
            selected_theme: SerializableTheme::default(),
            favorited_tickers: Vec::new(),
            last_active_layout: LayoutId::Layout1,
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
    pub layouts: HashMap<LayoutId, SerializableDashboard>,
    pub selected_theme: SerializableTheme,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub last_active_layout: LayoutId,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub present_mode: screen::PresentMode,
    pub scale_factor: ScaleFactor,
}

impl SerializableState {
    pub fn from_parts(
        layouts: HashMap<LayoutId, SerializableDashboard>,
        selected_theme: Theme,
        favorited_tickers: Vec<(Exchange, Ticker)>,
        last_active_layout: LayoutId,
        size: Option<Size>,
        position: Option<Point>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        present_mode: screen::PresentMode,
        scale_factor: ScaleFactor,
    ) -> Self {
        SerializableState {
            layouts,
            selected_theme: SerializableTheme {
                theme: selected_theme,
            },
            favorited_tickers,
            last_active_layout,
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

pub fn load_saved_state(file_path: &str) -> SavedState {
    match read_from_file(file_path) {
        Ok(state) => {
            let mut de_state = SavedState {
                selected_theme: state.selected_theme,
                layouts: HashMap::new(),
                favorited_tickers: state.favorited_tickers,
                last_active_layout: state.last_active_layout,
                window_size: state.window_size,
                window_position: state.window_position,
                timezone: state.timezone,
                sidebar: state.sidebar,
                present_mode: state.present_mode,
                scale_factor: state.scale_factor,
            };

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
                            let timeframe = settings.selected_timeframe.unwrap_or(Timeframe::M15);
                            Configuration::Pane(PaneState::from_config(
                                PaneContent::Candlestick(
                                    CandlestickChart::new(
                                        layout,
                                        vec![],
                                        timeframe,
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
                            let tick_size = settings.tick_multiply
                                .unwrap_or(TickMultiplier(50))
                                .multiply_with_min_tick_size(ticker_info);
                            let timeframe = settings.selected_timeframe.unwrap_or(Timeframe::M5);
                            Configuration::Pane(PaneState::from_config(
                                PaneContent::Footprint(
                                    FootprintChart::new(
                                        layout,
                                        timeframe,
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
                            let tick_size = settings.tick_multiply
                                .unwrap_or(TickMultiplier(10))
                                .multiply_with_min_tick_size(ticker_info);

                            let config = settings.visual_config
                                .map_or(None, |cfg| cfg.heatmap());

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
                        let config = settings.visual_config
                            .map_or(None, |cfg| cfg.time_and_sales());

                        Configuration::Pane(PaneState::from_config(
                            PaneContent::TimeAndSales(TimeAndSales::new(config)),
                            stream_type,
                            settings,
                        ))
                    },
                }
            }

            for (id, dashboard) in &state.layouts {
                let mut popout_windows: Vec<(Configuration<PaneState>, (Point, Size))> = Vec::new();

                for (popout, pos, size) in &dashboard.popout {
                    let configuration = configuration(popout.clone());
                    popout_windows.push((
                        configuration,
                        (Point::new(pos.0, pos.1), Size::new(size.0, size.1)),
                    ));
                }

                let dashboard = Dashboard::from_config(
                    configuration(dashboard.pane.clone()), popout_windows, dashboard.trade_fetch_enabled
                );

                de_state.layouts.insert(*id, dashboard);
            }

            de_state
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
    let path = PathBuf::from(get_data_path(file_name));
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn read_from_file(file_name: &str) -> Result<SerializableState, Box<dyn std::error::Error>> {
    let path = PathBuf::from(get_data_path(file_name));
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
    let data_path = get_data_path(
        "market_data/binance/data/futures/um/daily/aggTrades"
    );

    if !data_path.exists() {
        log::warn!("Data path {:?} does not exist, skipping cleanup", data_path);
        return 0;
    }

    let re = Regex::new(r".*-(\d{4}-\d{2}-\d{2})\.zip$")
        .expect("Cleanup regex pattern is valid");
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
    
    log::info!("File cleanup completed. Deleted {} files", deleted_files.len());
    deleted_files.len()
}
