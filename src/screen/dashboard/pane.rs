mod content;
mod state;

use crate::connector::fetcher::FetchSpec;

pub use content::Content;
pub(crate) use content::kline::{DataUpdate, FetchUpdate};
pub use state::{Action, Effect, Event, Message, State, Status};

pub enum ChartAction {
    ErrorOccurred(data::InternalError),
    RequestFetch(Vec<FetchSpec>),
    RequestPalette,
}
