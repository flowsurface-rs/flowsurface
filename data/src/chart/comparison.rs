use exchange::SerTicker;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub colors: Vec<(SerTicker, iced_core::Color)>,
}
