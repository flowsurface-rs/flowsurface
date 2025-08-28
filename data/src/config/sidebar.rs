use serde::{Deserialize, Deserializer, Serialize};

use crate::tickers_table;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Sidebar {
    pub position: Position,
    #[serde(skip)]
    pub active_menu: Option<Menu>,
    #[serde(default)]
    pub tickers_table: Option<tickers_table::Settings>,
}

impl Sidebar {
    pub fn set_menu(&mut self, new_menu: Menu) {
        self.active_menu = Some(new_menu);
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    pub fn is_menu_active(&self, menu: Menu) -> bool {
        self.active_menu == Some(menu)
    }

    pub fn sync_tickers_table_settings(&mut self, settings: &tickers_table::Settings) {
        self.tickers_table = Some(settings.clone());
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar {
            position: Position::Left,
            active_menu: None,
            tickers_table: None,
        }
    }
}

pub fn deserialize_sidebar_fallback<'de, D>(deserializer: D) -> Result<Sidebar, D::Error>
where
    D: Deserializer<'de>,
{
    Sidebar::deserialize(deserializer).or(Ok(Sidebar::default()))
}

#[derive(Default, Debug, Clone, PartialEq, Copy, Deserialize, Serialize)]
pub enum Position {
    #[default]
    Left,
    Right,
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Position::Left => write!(f, "Left"),
            Position::Right => write!(f, "Right"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub enum Menu {
    Layout,
    Settings,
    Audio,
    ThemeEditor,
}
