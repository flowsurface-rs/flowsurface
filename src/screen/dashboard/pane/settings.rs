use data::layout::pane::VisualConfig;

mod heatmap;
mod kline;
mod ladder;
mod timesales;

pub use heatmap::heatmap_shader_cfg_view;
pub use kline::kline_cfg_view;
pub use ladder::ladder_cfg_view;
pub use timesales::timesales_cfg_view;

use iced::{Element, widget::button};

#[derive(Debug, Clone)]
pub enum HeatmapIndicatorMessage {
    SpreadUnit(data::chart::heatmap::indicators::SpreadUnit),
    VolumeProfile(data::chart::heatmap::ProfileKind),
    Enabled(data::chart::heatmap::indicators::HeatmapIndicator, bool),
    DistanceEdited(String),
    DistanceSubmitted,
}

#[derive(Debug, Clone)]
pub enum Message {
    HeatmapIndicator(HeatmapIndicatorMessage),
    VisualConfigChanged {
        config: VisualConfig,
        sync_all: bool,
    },
}

pub fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    crate::tooltip(
        button("Sync all").on_press(Message::VisualConfigChanged {
            config,
            sync_all: true,
        }),
        Some("Apply configuration to similar panes"),
        iced::widget::tooltip::Position::Left,
    )
}
