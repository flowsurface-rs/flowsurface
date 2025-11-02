use exchange::TickerInfo;
use iced::Element;

use crate::dashboard::tickers_table::TickersTable;

#[derive(Debug, Clone, PartialEq)]
pub enum RowSelection {
    Switch(TickerInfo),
    Add(TickerInfo),
    Remove(TickerInfo),
}

pub enum Action {
    RowSelected(RowSelection),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MiniPanel {
    search_query: String,
    pub search_box_id: iced::widget::Id,
    scroll_offset: iced::widget::scrollable::AbsoluteOffset,
}

impl Default for MiniPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SearchChanged(String),
    RowSelected(RowSelection),
    Scrolled(iced::widget::scrollable::Viewport),
}

impl MiniPanel {
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            search_box_id: iced::widget::Id::unique(),
            scroll_offset: iced::widget::scrollable::AbsoluteOffset::default(),
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::SearchChanged(q) => self.search_query = q.to_uppercase(),
            Message::RowSelected(t) => {
                return Some(Action::RowSelected(t));
            }
            Message::Scrolled(vp) => {
                self.scroll_offset = vp.absolute_offset();
            }
        }
        None
    }

    pub fn view<'a>(
        &'a self,
        table: &'a TickersTable,
        selected_tickers: Option<&'a [TickerInfo]>,
        base_ticker: Option<TickerInfo>,
    ) -> Element<'a, Message> {
        iced::widget::responsive(move |bounds| {
            table.view_compact_with(
                bounds,
                &self.search_query,
                &self.search_box_id,
                self.scroll_offset,
                Message::RowSelected,
                Message::SearchChanged,
                Message::Scrolled,
                selected_tickers,
                base_ticker,
            )
        })
        .into()
    }
}
