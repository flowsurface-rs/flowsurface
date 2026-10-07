use super::layout::IndicatorRebuildPlan;
use data::chart::Basis;
use data::chart::kline::{IndicatorHost, IndicatorInstanceId, IndicatorInstanceStore, KlineData};
use exchange::TickerInfo;
use indicator_engine::IndicatorFrame;
use std::ops::ControlFlow;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

enum RebuildFailure {
    WorkerPanicked(tokio::task::JoinError),
    WorkerDropped,
}

impl std::fmt::Display for RebuildFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkerPanicked(error) => write!(f, "rebuild worker failed: {error}"),
            Self::WorkerDropped => f.write_str("rebuild worker dropped without reporting a result"),
        }
    }
}

pub(super) struct RebuildSnapshot {
    pub(super) plan: IndicatorRebuildPlan,
    pub(super) active_ids: Vec<IndicatorInstanceId>,
    pub(super) frames: Arc<[IndicatorFrame]>,
    pub(super) instances: Arc<IndicatorInstanceStore>,
    pub(super) basis: Option<Basis>,
    pub(super) base_ticker: TickerInfo,
}

impl RebuildSnapshot {
    pub(super) fn matches_calculation(
        &self,
        data: &KlineData,
        instances: &IndicatorInstanceStore,
        active: &[IndicatorInstanceId],
    ) -> bool {
        self.base_ticker == data.base_ticker()
            && self.basis == data.base_series().map(|series| series.basis())
            && self.active_ids == active
            && self.plan.replay_ids.iter().all(|id| {
                match (self.instances.get(*id), instances.get(*id)) {
                    (Some(previous), Some(current)) => {
                        previous.kind == current.kind
                            && previous.config == current.config
                            && previous.source_bindings == current.source_bindings
                            && previous.base_source_selections == current.base_source_selections
                    }
                    (None, None) => true,
                    _ => false,
                }
            })
    }

    pub(super) fn accepts_prefix(&self, frames: &[IndicatorFrame]) -> bool {
        frames.starts_with(&self.frames)
    }
}

enum RebuildPhase {
    Prepared {
        host: Option<Box<IndicatorHost>>,
        start: usize,
    },
    Running(oneshot::Receiver<Result<Option<IndicatorHost>, RebuildFailure>>),
}

struct PendingRebuild {
    job_id: u64,
    snapshot: RebuildSnapshot,
    cancelled: Arc<AtomicBool>,
    phase: RebuildPhase,
}

pub(super) struct CompletedRebuild {
    pub(super) job_id: u64,
    pub(super) snapshot: RebuildSnapshot,
    pub(super) host: IndicatorHost,
}

#[derive(Default)]
pub(super) struct RebuildCoordinator {
    next_job: u64,
    pending: Option<PendingRebuild>,
}

impl RebuildCoordinator {
    pub(super) fn pending_job(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.job_id)
    }

    pub(super) fn snapshot_mut(&mut self) -> Option<&mut RebuildSnapshot> {
        self.pending
            .as_mut()
            .filter(|pending| !pending.cancelled.load(Ordering::Relaxed))
            .map(|pending| &mut pending.snapshot)
    }

    pub(super) fn in_flight_ids(&self) -> &[IndicatorInstanceId] {
        self.pending
            .as_ref()
            .map_or(&[], |pending| pending.snapshot.plan.replay_ids.as_slice())
    }

    pub(super) fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.phase, RebuildPhase::Prepared { .. }))
        {
            self.pending = None;
        }
    }

    pub(super) fn prepare(&mut self, snapshot: RebuildSnapshot) {
        assert!(self.pending.is_none());
        self.next_job = self.next_job.wrapping_add(1);
        self.pending = Some(PendingRebuild {
            job_id: self.next_job,
            snapshot,
            cancelled: Arc::new(AtomicBool::new(false)),
            phase: RebuildPhase::Prepared {
                host: None,
                start: 0,
            },
        });
    }

    pub(super) fn prepare_catchup(
        &mut self,
        completed: CompletedRebuild,
        frames: Arc<[IndicatorFrame]>,
    ) {
        assert!(self.pending.is_none());
        let start = completed.snapshot.frames.len();
        let mut snapshot = completed.snapshot;
        snapshot.frames = frames;
        self.pending = Some(PendingRebuild {
            job_id: completed.job_id,
            snapshot,
            cancelled: Arc::new(AtomicBool::new(false)),
            phase: RebuildPhase::Prepared {
                host: Some(Box::new(completed.host)),
                start,
            },
        });
    }

    pub(super) fn launch(
        &mut self,
    ) -> Option<impl std::future::Future<Output = u64> + Send + 'static + use<>> {
        let pending = self.pending.as_mut()?;
        if !matches!(pending.phase, RebuildPhase::Prepared { .. }) {
            return None;
        }
        let job_id = pending.job_id;
        let (result_tx, result_rx) = oneshot::channel();
        let RebuildPhase::Prepared { host, start } =
            std::mem::replace(&mut pending.phase, RebuildPhase::Running(result_rx))
        else {
            unreachable!();
        };
        let ids = pending.snapshot.plan.replay_ids.clone();
        let frames = Arc::clone(&pending.snapshot.frames);
        let instances = Arc::clone(&pending.snapshot.instances);
        let cancelled = Arc::clone(&pending.cancelled);
        Some(async move {
            let result = tokio::task::spawn_blocking(move || {
                let continuing = host.is_some();
                let mut host = host.map(|host| *host).unwrap_or_default();
                let control = || {
                    if cancelled.load(Ordering::Relaxed) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                };
                let result = if continuing {
                    host.replay_closed_interruptible(&frames[start..], control)
                } else {
                    host.recalculate_interruptible(&ids, &instances, &frames, control)
                };
                result.is_continue().then_some(host)
            })
            .await
            .map_err(RebuildFailure::WorkerPanicked);
            let _ = result_tx.send(result);
            job_id
        })
    }

    pub(super) fn poll(&mut self, job_id: u64) -> Option<CompletedRebuild> {
        let pending = self.pending.as_mut()?;
        if pending.job_id != job_id {
            return None;
        }
        let RebuildPhase::Running(receiver) = &mut pending.phase else {
            return None;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(oneshot::error::TryRecvError::Closed) => Err(RebuildFailure::WorkerDropped),
        };
        let pending = self.pending.take().expect("indicator rebuild state exists");
        let cancelled = pending.cancelled.load(Ordering::Relaxed);
        let PendingRebuild {
            job_id, snapshot, ..
        } = pending;
        match result {
            Ok(Some(host)) if !cancelled => Some(CompletedRebuild {
                job_id,
                snapshot,
                host,
            }),
            Ok(_) => None,
            Err(failure) => {
                log::error!("indicator rebuild job {job_id} failed: {failure}");
                None
            }
        }
    }
}

impl Drop for RebuildCoordinator {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
    }
}
