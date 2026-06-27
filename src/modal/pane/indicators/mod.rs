mod editors;
mod lists;

use crate::screen::dashboard::pane::{self, Message};
use iced::{
    Element,
    widget::{container, pane_grid},
};

pub use lists::*;

pub fn view<'a>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    market_type: Option<exchange::adapter::MarketKind>,
) -> Element<'a, Message> {
    let content_allows_dragging = matches!(state.content, pane::Content::Kline { .. });
    let content = match (&state.content, market_type) {
        (pane::Content::Kline { indicators, .. }, Some(market)) => {
            content_row_kline(pane, state, indicators, market, content_allows_dragging)
        }
        (pane::Content::Heatmap { indicators, .. }, Some(market))
        | (pane::Content::ShaderHeatmap { indicators, .. }, Some(market)) => {
            content_row_heatmap(pane, state, indicators, market, false)
        }
        _ => iced::widget::column![].spacing(4).into(),
    };

    container(content)
        .max_width(320)
        .padding(16)
        .style(crate::style::chart_modal)
        .into()
}
