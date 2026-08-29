use data::config::theme::generated_theme_accents;
use iced_core::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeAccents {
    colors: [Color; 6],
    generation: u64,
}

impl ThemeAccents {
    fn from_theme(theme: &Theme, generation: u64) -> Self {
        Self {
            colors: generated_theme_accents(theme.extended_palette()),
            generation,
        }
    }

    pub(crate) fn as_array(&self) -> &[Color; 6] {
        &self.colors
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ThemeState {
    theme: data::Theme,
    accents: ThemeAccents,
}

impl ThemeState {
    pub(crate) fn new(theme: data::Theme) -> Self {
        Self {
            accents: ThemeAccents::from_theme(&theme.0, 0),
            theme,
        }
    }

    pub(crate) fn replace(&mut self, theme: Theme) -> bool {
        if self.theme.0 == theme {
            return false;
        }

        let generation = self.accents.generation().wrapping_add(1);
        let accents = ThemeAccents::from_theme(&theme, generation);

        self.theme = data::Theme(theme);
        self.accents = accents;
        true
    }

    pub(crate) fn current(&self) -> &Theme {
        &self.theme.0
    }

    pub(crate) fn data_theme(&self) -> data::Theme {
        self.theme.clone()
    }

    pub(crate) fn accents(&self) -> ThemeAccents {
        self.accents
    }
}
