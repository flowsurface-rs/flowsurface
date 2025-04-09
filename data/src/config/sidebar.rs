use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Sidebar {
    pub position: Position,
    #[serde(skip)]
    pub active_menu: Menu,
}

impl Sidebar {
    pub fn set_menu(&mut self, new_menu: Menu) {
        self.active_menu = new_menu;
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    pub fn is_menu_active(&self, menu: Menu) -> bool {
        self.active_menu == menu
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar {
            position: Position::Left,
            active_menu: Menu::None,
        }
    }
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

#[derive(Default, Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub enum Menu {
    Layout,
    Settings,
    Audio,
    #[default]
    None,
}
