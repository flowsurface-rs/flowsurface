mod content;
mod state;

use crate::connector::fetcher::FetchSpec;

pub use content::Content;
pub use content::kline::{DataUpdate, FetchUpdate};
pub use state::{Effect, Message, State};

pub enum Action {
    Chart(ChartAction),
    ResolveStreams(Vec<data::stream::PersistStreamKind>),
    ResolveContent,
}

pub enum ChartAction {
    RequestFetch(Vec<FetchSpec>),
    RequestPalette,
}
