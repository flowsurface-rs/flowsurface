/// Most of the stuff here is exact copy of some of the code from
/// <https://github.com/iced-rs/iced/blob/master/core/src/theme/palette.rs> &
/// <https://github.com/squidowl/halloy/blob/main/data/src/appearance/theme.rs>
/// All credits and thanks to the authors of [`Halloy`] and [`iced_core`]
use iced_core::{
    Color,
    theme::palette::Extended,
    theme::{Custom, Palette},
};
use palette::{
    Clamp, FromColor, Hsva, IsWithinBounds, OklabHue, Oklch, Srgb,
    convert::FromColorUnclamped,
    rgb::{Rgb, Rgba},
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Amount of accent colors, one per [`ThemeAccent`].
pub const ACCENT_COUNT: usize = ThemeAccent::ALL.len();
/// Every accent but the last one takes a hue; the last one is the endpoint of the
/// accent lightness scale.
const CHROMATIC_ACCENT_COUNT: usize = ACCENT_COUNT - 1;
/// Smallest hue distance two accents of one theme may have.
const MIN_ACCENT_HUE_SEPARATION: f32 = 42.0;
/// Smallest hue distance an accent may have to one of the theme's signal hues.
const MIN_SIGNAL_HUE_SEPARATION: f32 = 12.0;
/// Chroma below which a hue is not recognizable, so the color cannot claim one.
const MIN_HUE_REFERENCE_CHROMA: f32 = 0.04;
/// Largest hue distance possible, half of the wheel.
const MAX_HUE_DISTANCE: f32 = 180.0;
/// Placement resolution on the hue wheel.
const HUE_CANDIDATE_STEP: f32 = 0.5;
const HUE_CANDIDATE_COUNT: usize = (360.0 / HUE_CANDIDATE_STEP) as usize;
const TARGET_ACCENT_CHROMA: f32 = 0.14;
const ENDPOINT_ACCENT_CHROMA: f32 = 0.02;

#[derive(Debug, Clone)]
pub struct Theme(pub iced_core::Theme);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeAccent {
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
}

impl ThemeAccent {
    pub const ALL: [Self; 8] = [
        Self::One,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
        Self::Six,
        Self::Seven,
        Self::Eight,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::One => 0,
            Self::Two => 1,
            Self::Three => 2,
            Self::Four => 3,
            Self::Five => 4,
            Self::Six => 5,
            Self::Seven => 6,
            Self::Eight => 7,
        }
    }
}

impl std::fmt::Display for ThemeAccent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Accent {}", self.index() + 1)
    }
}

/// Derives the accent palette of `palette`.
///
/// Every chromatic accent has the same lightness, so no series color is louder than
/// another and they are told apart by hue alone. The last accent is the endpoint of
/// the lightness scale instead of a hue.
pub fn generated_theme_accents(palette: &Extended) -> [Color; ACCENT_COUNT] {
    let lightness = if palette.is_dark { 0.7 } else { 0.5 };
    let hues = AccentHues::new(palette).place_all();

    std::array::from_fn(|index| {
        hues.get(index).map_or_else(
            || endpoint_accent(palette),
            |hue| accent_color(*hue, lightness, TARGET_ACCENT_CHROMA),
        )
    })
}

/// Places the chromatic accents on the hue wheel.
///
/// Every accent keeps [`MIN_ACCENT_HUE_SEPARATION`] from the accents placed before
/// it, and prefers positions that keep [`MIN_SIGNAL_HUE_SEPARATION`] from the theme's
/// signal hues, so a series color never reads as a candle or status color.
struct AccentHues {
    signal_hues: Vec<f32>,
    identity_hue: Option<f32>,
    placed: Vec<f32>,
}

impl AccentHues {
    fn new(palette: &Extended) -> Self {
        Self {
            signal_hues: [
                palette.primary.base.color,
                palette.success.base.color,
                palette.danger.base.color,
                palette.warning.base.color,
            ]
            .into_iter()
            .filter_map(chromatic_hue)
            .collect(),
            identity_hue: chromatic_hue(palette.primary.base.color),
            placed: Vec::with_capacity(CHROMATIC_ACCENT_COUNT),
        }
    }

    fn place_all(mut self) -> Vec<f32> {
        self.placed.push(self.initial_hue());

        while self.placed.len() < CHROMATIC_ACCENT_COUNT {
            self.placed.push(self.next_hue());
        }

        self.placed
    }

    /// The theme's own hue, or the hue furthest from every signal hue when the theme
    /// has no chromatic primary to derive the palette from.
    fn initial_hue(&self) -> f32 {
        match self.identity_hue {
            Some(identity_hue) => self.best_hue(|hue| self.identity_rank(hue, identity_hue)),
            None => self.best_hue(|hue| self.accent_rank(hue)),
        }
    }

    /// The hue in the widest gap left on the wheel.
    fn next_hue(&self) -> f32 {
        self.best_hue(|hue| self.accent_rank(hue))
    }

    /// Ranks a position for a new accent: inside both separation budgets first, then
    /// the widest gap between the placed accents, then the furthest from the signal
    /// hues. Accent separation thus outranks signal clearance where the wheel is too
    /// crowded for both.
    fn accent_rank(&self, hue: f32) -> (bool, f32, f32) {
        let accent_clearance = self.accent_clearance(hue);
        let signal_clearance = self.signal_clearance(hue);

        (
            accent_clearance >= MIN_ACCENT_HUE_SEPARATION
                && signal_clearance >= MIN_SIGNAL_HUE_SEPARATION,
            accent_clearance,
            signal_clearance,
        )
    }

    /// Ranks a position for the first accent of a theme that has a hue of its own:
    /// inside both separation budgets first, then the closest to that hue.
    fn identity_rank(&self, hue: f32, identity_hue: f32) -> (bool, f32, f32) {
        let (within_budget, _, signal_clearance) = self.accent_rank(hue);

        (
            within_budget,
            -hue_distance(hue, identity_hue),
            signal_clearance,
        )
    }

    /// The hue with the highest rank, ties going to the lower hue.
    fn best_hue(&self, rank: impl Fn(f32) -> (bool, f32, f32)) -> f32 {
        let mut best_hue = 0.0;
        let mut best_rank = (false, f32::NEG_INFINITY, f32::NEG_INFINITY);

        for hue in candidate_hues() {
            let candidate = rank(hue);
            if candidate.partial_cmp(&best_rank) == Some(Ordering::Greater) {
                best_rank = candidate;
                best_hue = hue;
            }
        }

        best_hue
    }

    /// Distance from `hue` to the nearest placed accent.
    fn accent_clearance(&self, hue: f32) -> f32 {
        self.placed
            .iter()
            .map(|placed| hue_distance(hue, *placed))
            .fold(MAX_HUE_DISTANCE, f32::min)
    }

    /// Distance from `hue` to the nearest signal hue.
    fn signal_clearance(&self, hue: f32) -> f32 {
        self.signal_hues
            .iter()
            .map(|signal_hue| hue_distance(hue, *signal_hue))
            .fold(MAX_HUE_DISTANCE, f32::min)
    }
}

/// The hue of `color`, when it has enough chroma for a hue to be recognizable.
fn chromatic_hue(color: Color) -> Option<f32> {
    let oklch = to_oklch(color);

    (oklch.chroma >= MIN_HUE_REFERENCE_CHROMA).then(|| oklch.hue.into_positive_degrees())
}

/// The endpoint of the accent lightness scale: near white on dark themes and near
/// black on light themes, so the accent stays usable on any palette. It has no place
/// in the hue placement, because at this chroma its hue cannot be told apart.
fn endpoint_accent(palette: &Extended) -> Color {
    let lightness = if palette.is_dark { 0.96 } else { 0.04 };

    match chromatic_hue(palette.primary.base.color) {
        // A trace of the theme's hue keeps the endpoint from looking foreign.
        Some(hue) => accent_color(hue, lightness, ENDPOINT_ACCENT_CHROMA),
        None => accent_color(0.0, lightness, 0.0),
    }
}

fn accent_color(hue: f32, lightness: f32, chroma: f32) -> Color {
    let mut oklch = Oklch::from_color(Srgb::new(0.0, 0.0, 0.0));
    oklch.l = lightness;
    oklch.chroma = chroma;
    oklch.hue = OklabHue::from_degrees(hue);

    let rgb = gamut_mapped_srgb(oklch);
    Color {
        r: rgb.red,
        g: rgb.green,
        b: rgb.blue,
        a: 1.0,
    }
}

fn candidate_hues() -> impl Iterator<Item = f32> {
    (0..HUE_CANDIDATE_COUNT).map(|step| step as f32 * HUE_CANDIDATE_STEP)
}

fn hue_distance(first: f32, second: f32) -> f32 {
    let distance = (first - second).abs().rem_euclid(360.0);
    distance.min(360.0 - distance)
}

fn gamut_mapped_srgb(mut oklch: Oklch) -> Srgb {
    let mut rgb = Srgb::from_color_unclamped(oklch);

    if rgb.is_within_bounds() {
        return rgb;
    }

    let mut lower_chroma = 0.0;
    let mut upper_chroma = oklch.chroma;

    for _ in 0..12 {
        let chroma = (lower_chroma + upper_chroma) / 2.0;
        oklch.chroma = chroma;
        rgb = Srgb::from_color_unclamped(oklch);

        if rgb.is_within_bounds() {
            lower_chroma = chroma;
        } else {
            upper_chroma = chroma;
        }
    }

    oklch.chroma = lower_chroma;
    Srgb::from_color_unclamped(oklch).clamp()
}

fn to_oklch(color: Color) -> Oklch {
    Oklch::from_color(Srgb::new(color.r, color.g, color.b))
}

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
            primary: Color::from_rgb8(169, 174, 191),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeColorRole {
    Neutral,
    Secondary,
    Primary,
    Success,
    Danger,
    Warning,
}

impl ThemeColorRole {
    pub const ALL: &'static [Self] = &[
        Self::Neutral,
        Self::Secondary,
        Self::Primary,
        Self::Success,
        Self::Danger,
        Self::Warning,
    ];
}

impl std::str::FromStr for ThemeColorRole {
    type Err = UnknownThemeColor;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|role| role.to_string().eq_ignore_ascii_case(value))
            .ok_or_else(|| UnknownThemeColor::Role(value.to_owned()))
    }
}

impl std::fmt::Display for ThemeColorRole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Neutral => "Neutral",
            Self::Secondary => "Secondary",
            Self::Primary => "Primary",
            Self::Success => "Success",
            Self::Danger => "Danger",
            Self::Warning => "Warning",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeColorShade {
    Faint,
    Soft,
    #[default]
    Base,
    Strong,
    Vivid,
}

impl ThemeColorShade {
    pub const ALL: &'static [Self] = &[
        Self::Faint,
        Self::Soft,
        Self::Base,
        Self::Strong,
        Self::Vivid,
    ];
}

impl std::fmt::Display for ThemeColorShade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Faint => "Faint",
            Self::Soft => "Soft",
            Self::Base => "Base",
            Self::Strong => "Strong",
            Self::Vivid => "Vivid",
        };
        formatter.write_str(label)
    }
}

impl std::str::FromStr for ThemeColorShade {
    type Err = UnknownThemeColor;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|shade| shade.to_string().eq_ignore_ascii_case(value))
            .ok_or_else(|| UnknownThemeColor::Shade(value.to_owned()))
    }
}

/// A theme colour name that does not match any known role or shade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownThemeColor {
    Role(String),
    Shade(String),
}

impl std::fmt::Display for UnknownThemeColor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Role(value) => write!(formatter, "unknown theme role '{value}'"),
            Self::Shade(value) => write!(formatter, "unknown theme shade '{value}'"),
        }
    }
}

impl std::error::Error for UnknownThemeColor {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColor {
    pub role: ThemeColorRole,
    pub shade: ThemeColorShade,
    #[serde(default)]
    pub accent: Option<ThemeAccent>,
}

impl Default for ThemeColor {
    fn default() -> Self {
        Self::new(ThemeColorRole::Neutral, ThemeColorShade::Base)
    }
}

impl ThemeColor {
    pub const fn new(role: ThemeColorRole, shade: ThemeColorShade) -> Self {
        Self {
            role,
            shade,
            accent: None,
        }
    }

    pub const fn with_accent(
        role: ThemeColorRole,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    ) -> Self {
        Self {
            role,
            shade,
            accent,
        }
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
