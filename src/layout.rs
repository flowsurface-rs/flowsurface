use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use iced::widget::pane_grid;
use iced::{Point, Size, Theme};
use serde::{Deserialize, Serialize};

use crate::charts::indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator};
use crate::data_providers::{Exchange, StreamType, Ticker};
use crate::screen::dashboard::{Dashboard, PaneContent, PaneSettings, PaneState};
use crate::pane::Axis;
use crate::screen::UserTimezone;
use crate::style;

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

#[derive(Debug, Clone, PartialEq, Copy, Deserialize, Serialize)]
pub enum Sidebar {
    Left,
    Right,
}

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar::Left
    }
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
        }
    }
}

impl Default for SerializableDashboard {
    fn default() -> Self {
        Self {
            pane: SerializablePane::Starter,
            popout: vec![],
        }
    }
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
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
        indicators: Vec<HeatmapIndicator>,
    },
    FootprintChart {
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
        indicators: Vec<FootprintIndicator>,
    },
    CandlestickChart {
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
            PaneContent::Heatmap(_, indicators) => SerializablePane::HeatmapChart {
                stream_type: pane_stream,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Footprint(_, indicators) => SerializablePane::FootprintChart {
                stream_type: pane_stream,
                settings: pane.settings,
                indicators: indicators.clone(),
            },
            PaneContent::Candlestick(_, indicators) => SerializablePane::CandlestickChart {
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
