use iced_core::Color;
use serde::{Deserialize, Serialize};

pub const DEFAULT_POC_COLOR: Color = Color {
    r: 1.0,
    g: 0.73,
    b: 0.18,
    a: 1.0,
};

pub const DEFAULT_NPOC_NAKED_COLOR: Color = Color {
    r: 1.0,
    g: 0.73,
    b: 0.18,
    a: 1.0,
};

pub const DEFAULT_NPOC_FILLED_COLOR: Color = Color {
    r: 0.45,
    g: 0.45,
    b: 0.45,
    a: 1.0,
};

pub const DEFAULT_BUY_IMBALANCE_COLOR: Color = Color {
    r: 0.10,
    g: 0.80,
    b: 0.35,
    a: 1.0,
};

pub const DEFAULT_SELL_IMBALANCE_COLOR: Color = Color {
    r: 0.95,
    g: 0.20,
    b: 0.25,
    a: 1.0,
};

fn default_poc_color() -> Color {
    DEFAULT_POC_COLOR
}

fn default_npoc_naked_color() -> Color {
    DEFAULT_NPOC_NAKED_COLOR
}

fn default_npoc_filled_color() -> Color {
    DEFAULT_NPOC_FILLED_COLOR
}

fn default_buy_imbalance_color() -> Color {
    DEFAULT_BUY_IMBALANCE_COLOR
}

fn default_sell_imbalance_color() -> Color {
    DEFAULT_SELL_IMBALANCE_COLOR
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SingleColorStyle {
    #[serde(default = "default_poc_color")]
    pub color: Color,
}

impl SingleColorStyle {
    pub const fn new(color: Color) -> Self {
        Self { color }
    }

    pub const fn default_poc() -> Self {
        Self::new(DEFAULT_POC_COLOR)
    }
}

impl Default for SingleColorStyle {
    fn default() -> Self {
        Self::default_poc()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NakedFilledColors {
    #[serde(default = "default_npoc_naked_color")]
    pub naked_color: Color,
    #[serde(default = "default_npoc_filled_color")]
    pub filled_color: Color,
}

impl NakedFilledColors {
    pub const fn new(naked_color: Color, filled_color: Color) -> Self {
        Self {
            naked_color,
            filled_color,
        }
    }

    pub const fn default_npoc() -> Self {
        Self::new(DEFAULT_NPOC_NAKED_COLOR, DEFAULT_NPOC_FILLED_COLOR)
    }
}

impl Default for NakedFilledColors {
    fn default() -> Self {
        Self::default_npoc()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BuySellColors {
    #[serde(default = "default_buy_imbalance_color")]
    pub buy_color: Color,
    #[serde(default = "default_sell_imbalance_color")]
    pub sell_color: Color,
}

impl BuySellColors {
    pub const fn new(buy_color: Color, sell_color: Color) -> Self {
        Self {
            buy_color,
            sell_color,
        }
    }

    pub const fn default_imbalance() -> Self {
        Self::new(DEFAULT_BUY_IMBALANCE_COLOR, DEFAULT_SELL_IMBALANCE_COLOR)
    }
}

impl Default for BuySellColors {
    fn default() -> Self {
        Self::default_imbalance()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RatioColorScale {
    pub color_scale: Option<usize>,
}

impl RatioColorScale {
    pub const fn new(color_scale: Option<usize>) -> Self {
        Self { color_scale }
    }

    pub fn alpha_from_ratio(self, ratio: f64) -> f32 {
        if let Some(scale) = self.color_scale {
            let divisor = ((scale as f64 / 10.0) - 1.0).max(f64::EPSILON);
            (0.2 + 0.8 * ((ratio - 1.0) / divisor).min(1.0)).min(1.0) as f32
        } else {
            1.0
        }
    }
}

impl Default for RatioColorScale {
    fn default() -> Self {
        Self::new(Some(400))
    }
}
