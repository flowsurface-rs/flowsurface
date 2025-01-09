use regex::Regex;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use iced::widget::pane_grid::{self, Configuration};
use iced::{Point, Size, Theme};

use crate::charts::candlestick::CandlestickChart;
use crate::charts::footprint::FootprintChart;
use crate::charts::heatmap::HeatmapChart;
use crate::charts::timeandsales::TimeAndSales;
use crate::charts::indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator};
use crate::data_providers::{Exchange, StreamType, TickMultiplier, Ticker, Timeframe};
use crate::screen::{UserTimezone, dashboard::{Dashboard, PaneContent, PaneSettings, PaneState}};
use crate::style;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::fs::File;
use std::path::Path;

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
                                        UserTimezone::default(),
                                        &indicators,
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
                                        UserTimezone::default(),
                                        &indicators,
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

                            Configuration::Pane(PaneState::from_config(
                                PaneContent::Heatmap(
                                    HeatmapChart::new(
                                        layout,
                                        tick_size,
                                        100,
                                        UserTimezone::default(),
                                        &indicators,
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
                    } => Configuration::Pane(PaneState::from_config(
                        PaneContent::TimeAndSales(TimeAndSales::new()),
                        stream_type,
                        settings,
                    )),
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

pub fn write_json_to_file(json: &str, file_path: &str) -> std::io::Result<()> {
    let path = Path::new(file_path);
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn read_from_file(file_path: &str) -> Result<SerializableState, Box<dyn std::error::Error>> {
    let path = Path::new(file_path);
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    Ok(serde_json::from_str(&contents)?)
}

pub fn cleanup_old_data(data_path: &std::path::Path) -> usize {
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
