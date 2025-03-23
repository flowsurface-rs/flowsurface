use iced_core::{
    Color,
    theme::{Custom, Palette},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Theme(pub iced_core::Theme);

impl Default for Theme {
    fn default() -> Self {
        Self(iced_core::Theme::Custom(custom_theme().into()))
    }
}

impl From<Theme> for iced_core::Theme {
    fn from(val: Theme) -> Self {
        val.0
    }
}

pub fn custom_theme() -> Custom {
    Custom::new(
        "Flowsurface".to_string(),
        Palette {
            background: Color::from_rgb8(24, 22, 22),
            text: Color::from_rgb8(197, 201, 197),
            primary: Color::from_rgb8(200, 200, 200),
            success: Color::from_rgb8(81, 205, 160),
            danger: Color::from_rgb8(192, 80, 77),
            warning: Color::from_rgb8(238, 216, 139),
        },
    )
}

impl Serialize for Theme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let theme_str = match self.0 {
            iced_core::Theme::Ferra => "ferra",
            iced_core::Theme::Dark => "dark",
            iced_core::Theme::Light => "light",
            iced_core::Theme::Dracula => "dracula",
            iced_core::Theme::Nord => "nord",
            iced_core::Theme::SolarizedLight => "solarized_light",
            iced_core::Theme::SolarizedDark => "solarized_dark",
            iced_core::Theme::GruvboxLight => "gruvbox_light",
            iced_core::Theme::GruvboxDark => "gruvbox_dark",
            iced_core::Theme::CatppuccinLatte => "catppuccino_latte",
            iced_core::Theme::CatppuccinFrappe => "catppuccino_frappe",
            iced_core::Theme::CatppuccinMacchiato => "catppuccino_macchiato",
            iced_core::Theme::CatppuccinMocha => "catppuccino_mocha",
            iced_core::Theme::TokyoNight => "tokyo_night",
            iced_core::Theme::TokyoNightStorm => "tokyo_night_storm",
            iced_core::Theme::TokyoNightLight => "tokyo_night_light",
            iced_core::Theme::KanagawaWave => "kanagawa_wave",
            iced_core::Theme::KanagawaDragon => "kanagawa_dragon",
            iced_core::Theme::KanagawaLotus => "kanagawa_lotus",
            iced_core::Theme::Moonfly => "moonfly",
            iced_core::Theme::Nightfly => "nightfly",
            iced_core::Theme::Oxocarbon => "oxocarbon",
            iced_core::Theme::Custom(_) => "flowsurface",
        };
        theme_str.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let theme_str = String::deserialize(deserializer)?;
        let theme = match theme_str.as_str() {
            "ferra" => iced_core::Theme::Ferra,
            "dark" => iced_core::Theme::Dark,
            "light" => iced_core::Theme::Light,
            "dracula" => iced_core::Theme::Dracula,
            "nord" => iced_core::Theme::Nord,
            "solarized_light" => iced_core::Theme::SolarizedLight,
            "solarized_dark" => iced_core::Theme::SolarizedDark,
            "gruvbox_light" => iced_core::Theme::GruvboxLight,
            "gruvbox_dark" => iced_core::Theme::GruvboxDark,
            "catppuccino_latte" => iced_core::Theme::CatppuccinLatte,
            "catppuccino_frappe" => iced_core::Theme::CatppuccinFrappe,
            "catppuccino_macchiato" => iced_core::Theme::CatppuccinMacchiato,
            "catppuccino_mocha" => iced_core::Theme::CatppuccinMocha,
            "tokyo_night" => iced_core::Theme::TokyoNight,
            "tokyo_night_storm" => iced_core::Theme::TokyoNightStorm,
            "tokyo_night_light" => iced_core::Theme::TokyoNightLight,
            "kanagawa_wave" => iced_core::Theme::KanagawaWave,
            "kanagawa_dragon" => iced_core::Theme::KanagawaDragon,
            "kanagawa_lotus" => iced_core::Theme::KanagawaLotus,
            "moonfly" => iced_core::Theme::Moonfly,
            "nightfly" => iced_core::Theme::Nightfly,
            "oxocarbon" => iced_core::Theme::Oxocarbon,
            "flowsurface" => Theme::default().0,
            _ => return Err(serde::de::Error::custom("Invalid theme")),
        };
        Ok(Theme(theme))
    }
}
