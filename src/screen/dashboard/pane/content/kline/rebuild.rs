use super::layout::IndicatorRebuildPlan;
use data::chart::kline::{IndicatorHost, IndicatorInstanceId, IndicatorInstanceStore};
use indicator_engine::IndicatorFrame;
use std::sync::Arc;
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
    pub(super) frame_revision: u64,
    pub(super) plan: IndicatorRebuildPlan,
    pub(super) active_ids: Vec<IndicatorInstanceId>,
    pub(super) frames: Arc<[IndicatorFrame]>,
}

struct PendingRebuild {
    job_id: u64,
    snapshot: RebuildSnapshot,
    result: oneshot::Receiver<Result<IndicatorHost, RebuildFailure>>,
}

pub(super) struct CompletedRebuild {
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

    pub(super) fn in_flight_ids(&self) -> &[IndicatorInstanceId] {
        self.pending
            .as_ref()
            .map_or(&[], |pending| pending.snapshot.plan.replay_ids.as_slice())
    }

    pub(super) fn begin(
        &mut self,
        plan: IndicatorRebuildPlan,
        active_ids: Vec<IndicatorInstanceId>,
        frames: Arc<[IndicatorFrame]>,
        instances: IndicatorInstanceStore,
        frame_revision: u64,
    ) -> Option<impl std::future::Future<Output = u64> + Send + 'static + use<>> {
        if self.pending.is_some() {
            return None;
        }
        let job_id = self.next_job.wrapping_add(1);
        self.next_job = job_id;
        let (result_tx, result_rx) = oneshot::channel();
        let worker_replay_ids = plan.replay_ids.clone();
        let worker_frames = Arc::clone(&frames);
        self.pending = Some(PendingRebuild {
            job_id,
            snapshot: RebuildSnapshot {
                frame_revision,
                plan,
                active_ids,
                frames,
            },
            result: result_rx,
        });
        Some(async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut host = IndicatorHost::default();
                host.recalculate(&worker_replay_ids, &instances, &worker_frames);
                host
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
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(oneshot::error::TryRecvError::Closed) => Err(RebuildFailure::WorkerDropped),
        };
        let pending = self.pending.take().expect("indicator rebuild state exists");
        match result {
            Ok(host) => Some(CompletedRebuild {
                snapshot: pending.snapshot,
                host,
            }),
            Err(failure) => {
                log::error!("indicator rebuild job {job_id} failed: {failure}");
                None
            }
        }
    }
}
