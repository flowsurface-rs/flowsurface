use crate::screen::dashboard::pane::{BackgroundJob, Completed};

use super::layout::IndicatorRebuildPlan;
use data::chart::Basis;
use data::chart::kline::{IndicatorHost, IndicatorInstanceId, IndicatorInstanceStore, KlineData};
use exchange::TickerInfo;
use indicator_engine::IndicatorFrame;
use std::sync::Arc;

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

struct RebuildWork {
    host: Option<Box<IndicatorHost>>,
    start: usize,
    ids: Vec<IndicatorInstanceId>,
    frames: Arc<[IndicatorFrame]>,
    instances: Arc<IndicatorInstanceStore>,
}

impl RebuildWork {
    fn new(snapshot: &RebuildSnapshot, host: Option<Box<IndicatorHost>>, start: usize) -> Self {
        Self {
            host,
            start,
            ids: snapshot.plan.replay_ids.clone(),
            frames: Arc::clone(&snapshot.frames),
            instances: Arc::clone(&snapshot.instances),
        }
    }
}

pub(super) type CompletedRebuild = Completed<RebuildSnapshot, IndicatorHost>;

#[derive(Default)]
pub(super) struct RebuildCoordinator {
    job: BackgroundJob<RebuildSnapshot, RebuildWork, IndicatorHost>,
}

impl RebuildCoordinator {
    pub(super) fn pending_job(&self) -> Option<u64> {
        self.job.pending_job()
    }

    pub(super) fn snapshot_mut(&mut self) -> Option<&mut RebuildSnapshot> {
        self.job.snapshot_mut()
    }

    pub(super) fn in_flight_ids(&self) -> &[IndicatorInstanceId] {
        self.job
            .snapshot()
            .map_or(&[], |snapshot| snapshot.plan.replay_ids.as_slice())
    }

    pub(super) fn cancel(&mut self) {
        self.job.cancel();
    }

    pub(super) fn prepare(&mut self, snapshot: RebuildSnapshot) {
        let work = RebuildWork::new(&snapshot, None, 0);
        self.job.prepare(snapshot, work);
    }

    pub(super) fn prepare_catchup(
        &mut self,
        completed: CompletedRebuild,
        frames: Arc<[IndicatorFrame]>,
    ) {
        let start = completed.snapshot.frames.len();
        let mut snapshot = completed.snapshot;
        snapshot.frames = frames;
        let work = RebuildWork::new(&snapshot, Some(Box::new(completed.output)), start);
        self.job
            .prepare_continuation(completed.job_id, snapshot, work);
    }

    pub(super) fn launch(
        &mut self,
    ) -> Option<impl std::future::Future<Output = u64> + Send + 'static + use<>> {
        self.job.launch(|work, cancelled| {
            let continuing = work.host.is_some();
            let mut host = work.host.map(|host| *host).unwrap_or_default();
            let control = || cancelled.check();
            let result = if continuing {
                host.replay_closed_interruptible(&work.frames[work.start..], control)
            } else {
                host.recalculate_interruptible(&work.ids, &work.instances, &work.frames, control)
            };
            result.is_continue().then_some(host)
        })
    }

    pub(super) fn poll(&mut self, job_id: u64) -> Option<CompletedRebuild> {
        self.job.poll(job_id)
    }
}
