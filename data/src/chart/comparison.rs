use exchange::SerTicker;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub colors: Vec<(SerTicker, iced_core::Color)>,
    pub names: Vec<(SerTicker, String)>,
}
