mod content;
pub mod settings;
mod state;

use crate::connector::fetcher::FetchSpec;
use std::ops::ControlFlow;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

pub use content::kline::KlineChart;
pub use content::{Content, heatmap::HeatmapContent};
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

enum Phase<Work, Output> {
    Prepared(Work),
    Running(oneshot::Receiver<Result<Option<Output>, Failure>>),
}

struct BackgroundJob<Snapshot, Work, Output> {
    next_job: u64,
    pending: Option<Pending<Snapshot, Work, Output>>,
}

impl<Snapshot, Work, Output> Default for BackgroundJob<Snapshot, Work, Output> {
    fn default() -> Self {
        Self {
            next_job: 0,
            pending: None,
        }
    }
}

impl<Snapshot, Work, Output> BackgroundJob<Snapshot, Work, Output> {
    fn pending_job(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.job_id)
    }

    fn snapshot(&self) -> Option<&Snapshot> {
        self.pending.as_ref().map(|pending| &pending.snapshot)
    }

    fn snapshot_mut(&mut self) -> Option<&mut Snapshot> {
        self.pending
            .as_mut()
            .filter(|pending| pending.cancelled.check().is_continue())
            .map(|pending| &mut pending.snapshot)
    }

    fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.0.store(true, Ordering::Relaxed);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.phase, Phase::Prepared(_)))
        {
            self.pending = None;
        }
    }

    fn prepare(&mut self, snapshot: Snapshot, work: Work) {
        self.next_job = self.next_job.wrapping_add(1);
        self.prepare_continuation(self.next_job, snapshot, work);
    }

    fn prepare_continuation(&mut self, job_id: u64, snapshot: Snapshot, work: Work) {
        assert!(self.pending.is_none());
        self.pending = Some(Pending {
            job_id,
            snapshot,
            cancelled: Cancellation(Arc::new(AtomicBool::new(false))),
            phase: Phase::Prepared(work),
        });
    }

    fn launch<F>(
        &mut self,
        calculate: F,
    ) -> Option<
        impl std::future::Future<Output = u64> + Send + 'static + use<Snapshot, Work, Output, F>,
    >
    where
        F: FnOnce(Work, Cancellation) -> Option<Output> + Send + 'static,
        Work: Send + 'static,
        Output: Send + 'static,
    {
        let pending = self.pending.as_mut()?;
        if !matches!(pending.phase, Phase::Prepared(_)) {
            return None;
        }
        let job_id = pending.job_id;
        let (result_tx, result_rx) = oneshot::channel();
        let Phase::Prepared(work) =
            std::mem::replace(&mut pending.phase, Phase::Running(result_rx))
        else {
            unreachable!();
        };
        let cancelled = Cancellation(Arc::clone(&pending.cancelled.0));

        Some(async move {
            let result = tokio::task::spawn_blocking(move || calculate(work, cancelled))
                .await
                .map_err(Failure::WorkerPanicked);
            let _ = result_tx.send(result);
            job_id
        })
    }

    fn poll(&mut self, job_id: u64) -> Option<Completed<Snapshot, Output>> {
        let pending = self.pending.as_mut()?;
        if pending.job_id != job_id {
            return None;
        }
        let Phase::Running(receiver) = &mut pending.phase else {
            return None;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(oneshot::error::TryRecvError::Closed) => Err(Failure::WorkerDropped),
        };

        let pending = self.pending.take().expect("background job exists");
        match result {
            Ok(Some(output)) if pending.cancelled.check().is_continue() => Some(Completed {
                job_id: pending.job_id,
                snapshot: pending.snapshot,
                output,
            }),
            Ok(_) => None,
            Err(failure) => {
                log::error!("background job {job_id} failed: {failure}");
                None
            }
        }
    }
}

impl<Snapshot, Work, Output> Drop for BackgroundJob<Snapshot, Work, Output> {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.0.store(true, Ordering::Relaxed);
        }
    }
}

struct Pending<Snapshot, Work, Output> {
    job_id: u64,
    snapshot: Snapshot,
    cancelled: Cancellation,
    phase: Phase<Work, Output>,
}

struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub(super) fn check(&self) -> ControlFlow<()> {
        if self.0.load(Ordering::Relaxed) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

struct Completed<Snapshot, Output> {
    job_id: u64,
    snapshot: Snapshot,
    output: Output,
}

enum Failure {
    WorkerPanicked(tokio::task::JoinError),
    WorkerDropped,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkerPanicked(error) => write!(f, "background worker failed: {error}"),
            Self::WorkerDropped => {
                f.write_str("background worker dropped without reporting a result")
            }
        }
    }
}
