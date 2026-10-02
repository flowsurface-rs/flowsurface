mod content;
pub(crate) mod settings;
mod state;

use crate::connector::fetcher::FetchSpec;

pub use content::Content;
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
