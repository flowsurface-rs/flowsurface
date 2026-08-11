/// Most of the stuff here is exact copy of some of the code from
/// <https://github.com/iced-rs/iced/blob/master/core/src/theme/palette.rs> &
/// <https://github.com/squidowl/halloy/blob/main/data/src/appearance/theme.rs>
/// All credits and thanks to the authors of [`Halloy`] and [`iced_core`]
use iced_core::{
    Color,
    theme::{Custom, Palette},
};
use palette::{
    FromColor, Hsva, RgbHue,
    rgb::{Rgb, Rgba},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Theme(pub iced_core::Theme);

#[derive(Serialize, Deserialize)]
struct SerTheme {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    palette: Option<Palette>,
}

impl Default for Theme {
    fn default() -> Self {
        Self(iced_core::Theme::Custom(default_theme().into()))
    }
}

impl From<Theme> for iced_core::Theme {
    fn from(val: Theme) -> Self {
        val.0
    }
}

pub fn default_theme() -> Custom {
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
        if let iced_core::Theme::Custom(custom) = &self.0 {
            let is_default_theme = custom.to_string() == "Flowsurface";
            let ser_theme = SerTheme {
                name: if is_default_theme {
                    "flowsurface"
                } else {
                    "custom"
                }
                .to_string(),
                palette: if is_default_theme {
                    None
                } else {
                    Some(self.0.palette())
                },
            };
            ser_theme.serialize(serializer)
        } else {
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
                _ => unreachable!(),
            };
            theme_str.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value =
            serde_json::Value::deserialize(deserializer).map_err(serde::de::Error::custom)?;

        if let Some(s) = value.as_str() {
            let theme = match s {
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
                _ => {
                    return Err(serde::de::Error::custom(format!("Invalid theme: {}", s)));
                }
            };
            return Ok(Theme(theme));
        }

        let serialized = SerTheme::deserialize(value).map_err(serde::de::Error::custom)?;

        let theme = match serialized.name.as_str() {
            "flowsurface" => Theme::default().0,
            "custom" => {
                if let Some(palette) = serialized.palette {
                    iced_core::Theme::Custom(Custom::new("Custom".to_string(), palette).into())
                } else {
                    return Err(serde::de::Error::custom(
                        "Custom theme missing palette data",
                    ));
                }
            }
            _ => return Err(serde::de::Error::custom("Invalid theme")),
        };

        Ok(Theme(theme))
    }
}

pub fn hex_to_color(hex: &str) -> Option<Color> {
    if hex.len() == 7 || hex.len() == 9 {
        let hash = &hex[0..1];
        let r = u8::from_str_radix(&hex[1..3], 16);
        let g = u8::from_str_radix(&hex[3..5], 16);
        let b = u8::from_str_radix(&hex[5..7], 16);
        let a = (hex.len() == 9)
            .then(|| u8::from_str_radix(&hex[7..9], 16).ok())
            .flatten();

        return match (hash, r, g, b, a) {
            ("#", Ok(r), Ok(g), Ok(b), None) => Some(Color {
                r: f32::from(r) / 255.0,
                g: f32::from(g) / 255.0,
                b: f32::from(b) / 255.0,
                a: 1.0,
            }),
            ("#", Ok(r), Ok(g), Ok(b), Some(a)) => Some(Color {
                r: f32::from(r) / 255.0,
                g: f32::from(g) / 255.0,
                b: f32::from(b) / 255.0,
                a: f32::from(a) / 255.0,
            }),
            _ => None,
        };
    }

    None
}

pub fn color_to_hex(color: Color) -> String {
    use std::fmt::Write;

    let mut hex = String::with_capacity(9);

    let [r, g, b, a] = color.into_rgba8();

    let _ = write!(&mut hex, "#");
    let _ = write!(&mut hex, "{r:02X}");
    let _ = write!(&mut hex, "{g:02X}");
    let _ = write!(&mut hex, "{b:02X}");

    if a < u8::MAX {
        let _ = write!(&mut hex, "{a:02X}");
    }

    hex
}

pub fn from_hsva(color: Hsva) -> Color {
    to_color(palette::Srgba::from_color(color))
}

fn to_color(rgba: Rgba) -> Color {
    Color {
        r: rgba.color.red,
        g: rgba.color.green,
        b: rgba.color.blue,
        a: rgba.alpha,
    }
}

pub fn to_hsva(color: Color) -> Hsva {
    Hsva::from_color(to_rgba(color))
}

fn to_rgb(color: Color) -> Rgb {
    Rgb {
        red: color.r,
        green: color.g,
        blue: color.b,
        ..Rgb::default()
    }
}

fn to_rgba(color: Color) -> Rgba {
    Rgba {
        alpha: color.a,
        color: to_rgb(color),
    }
}

pub fn darken(color: Color, amount: f32) -> Color {
    let mut hsl = to_hsl(color);

    hsl.l = if hsl.l - amount < 0.0 {
        0.0
    } else {
        hsl.l - amount
    };

    from_hsl(hsl)
}

pub fn lighten(color: Color, amount: f32) -> Color {
    let mut hsl = to_hsl(color);

    hsl.l = if hsl.l + amount > 1.0 {
        1.0
    } else {
        hsl.l + amount
    };

    from_hsl(hsl)
}

fn to_hsl(color: Color) -> Hsl {
    let x_max = color.r.max(color.g).max(color.b);
    let x_min = color.r.min(color.g).min(color.b);
    let c = x_max - x_min;
    let l = x_max.midpoint(x_min);

    let h = if c == 0.0 {
        0.0
    } else if x_max == color.r {
        60.0 * ((color.g - color.b) / c).rem_euclid(6.0)
    } else if x_max == color.g {
        60.0 * (((color.b - color.r) / c) + 2.0)
    } else {
        // x_max == color.b
        60.0 * (((color.r - color.g) / c) + 4.0)
    };

    let s = if l == 0.0 || l == 1.0 {
        0.0
    } else {
        (x_max - l) / l.min(1.0 - l)
    };

    Hsl {
        h,
        s,
        l,
        a: color.a,
    }
}

pub fn is_dark(color: Color) -> bool {
    let brightness = (color.r * 299.0 + color.g * 587.0 + color.b * 114.0) / 1000.0;
    brightness < 0.5
}

struct Hsl {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

// https://en.wikipedia.org/wiki/HSL_and_HSV#HSL_to_RGB
fn from_hsl(hsl: Hsl) -> Color {
    let c = (1.0 - (2.0 * hsl.l - 1.0).abs()) * hsl.s;
    let h = hsl.h / 60.0;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());

    let (r1, g1, b1) = if h < 1.0 {
        (c, x, 0.0)
    } else if h < 2.0 {
        (x, c, 0.0)
    } else if h < 3.0 {
        (0.0, c, x)
    } else if h < 4.0 {
        (0.0, x, c)
    } else if h < 5.0 {
        (x, 0.0, c)
    } else {
        // h < 6.0
        (c, 0.0, x)
    };

    let m = hsl.l - (c / 2.0);

    Color {
        r: r1 + m,
        g: g1 + m,
        b: b1 + m,
        a: hsl.a,
    }
}

pub fn from_hsv_degrees(h_deg: f32, s: f32, v: f32) -> Color {
    // Hue in degrees [0,360), s,v in [0,1]
    let hue = RgbHue::from_degrees(h_deg);
    from_hsva(Hsva::new(hue, s, v, 1.0))
}

/// Alpha-composite a foreground color over a background.
pub fn composite_color(foreground: Color, background: Color) -> Color {
    let alpha = foreground.a.clamp(0.0, 1.0);
    Color {
        r: foreground.r.mul_add(alpha, background.r * (1.0 - alpha)),
        g: foreground.g.mul_add(alpha, background.g * (1.0 - alpha)),
        b: foreground.b.mul_add(alpha, background.b * (1.0 - alpha)),
        a: 1.0,
    }
}

/// Compute the WCAG contrast ratio between two colors.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let l1 = relative_luminance(a);
    let l2 = relative_luminance(b);
    let lighter = l1.max(l2);
    let darker = l1.min(l2);

    (lighter + 0.05) / (darker + 0.05)
}

/// Compute the relative luminance of a color per the WCAG specification.
pub fn relative_luminance(color: Color) -> f32 {
    let channel = |value: f32| {
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };

    (0.2126 * channel(color.r)) + (0.7152 * channel(color.g)) + (0.0722 * channel(color.b))
}

/// Linearly interpolate (mix) two colors by a foreground-weight factor.
pub fn mix_color(foreground: Color, background: Color, foreground_weight: f32) -> Color {
    let foreground_weight = foreground_weight.clamp(0.0, 1.0);
    let background_weight = 1.0 - foreground_weight;

    Color {
        r: foreground
            .r
            .mul_add(foreground_weight, background.r * background_weight),
        g: foreground
            .g
            .mul_add(foreground_weight, background.g * background_weight),
        b: foreground
            .b
            .mul_add(foreground_weight, background.b * background_weight),
        a: foreground
            .a
            .mul_add(foreground_weight, background.a * background_weight),
    }
}
