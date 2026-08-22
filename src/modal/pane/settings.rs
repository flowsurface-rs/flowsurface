use crate::widget::study_configurator;
use data::layout::pane::VisualConfig;

mod heatmap;
mod kline;
mod ladder;
mod times_and_sales;

pub use heatmap::{heatmap_cfg_view, heatmap_shader_cfg_view};
use iced::{
    Element, Length,
    widget::{button, container},
};
pub use kline::kline_cfg_view;
pub use ladder::ladder_cfg_view;
pub use times_and_sales::timesales_cfg_view;

#[derive(Debug, Clone)]
pub enum Message {
    VisualConfigChanged {
        config: VisualConfig,
        sync_all: bool,
    },
    ClusterKindSelected(data::chart::kline::ClusterKind),
    ClusterScalingSelected(data::chart::kline::ClusterScaling),
    Study(study_configurator::StudyMessage),
}

pub(crate) fn cfg_view_container<'a, T>(max_width: u32, content: T) -> Element<'a, Message>
where
    T: Into<Element<'a, Message>>,
{
    container(crate::widget::scrollable_content(content))
        .width(Length::Shrink)
        .padding(28)
        .max_width(max_width)
        .style(crate::style::chart_modal)
        .into()
}

pub(crate) fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    crate::tooltip(
        button("Sync all").on_press(Message::VisualConfigChanged {
            config,
            sync_all: true,
        }),
        Some("Apply configuration to similar panes"),
        iced::widget::tooltip::Position::Top,
    )
}
