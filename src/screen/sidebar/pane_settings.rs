use crate::widget::study_configurator;
use data::layout::pane::VisualConfig;

mod heatmap;
mod ladder;
mod timesales;

pub use heatmap::heatmap_shader_cfg_view;
use iced::{Element, widget::button};
pub use ladder::ladder_cfg_view;
pub use timesales::timesales_cfg_view;

#[derive(Debug, Clone)]
pub enum Message {
    VisualConfigChanged {
        config: VisualConfig,
        sync_all: bool,
    },
    Study(study_configurator::StudyMessage),
}

pub fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    crate::tooltip(
        button("Sync all").on_press(Message::VisualConfigChanged {
            config,
            sync_all: true,
        }),
        Some("Apply configuration to similar panes"),
        iced::widget::tooltip::Position::Top,
    )
}
