mod source;
mod tpo;

use data::chart::Basis;
use data::chart::kline::{
    FetchRange, KlineStudy, TpoAnchor, TpoBoundaryEdits, TpoProfile, TpoProfileStatus,
};
use data::chart::kline::{KlineData, StudyDataSource, VolumeProfileSchedule};
use exchange::unit::price::PriceStep;
use exchange::{Kline, TickerInfo, Timeframe, Trade, UnixMs};
use std::collections::BTreeMap;

use self::source::ProfileKlineSource;
use self::tpo::{TpoBarScope, TpoState};

pub(super) fn profile_source_timeframe(chart_timeframe: Timeframe) -> Timeframe {
    if chart_timeframe <= Timeframe::H2 {
        Timeframe::M1
    } else if chart_timeframe <= Timeframe::H4 {
        Timeframe::M3
    } else if chart_timeframe <= Timeframe::H12 {
        Timeframe::M5
    } else {
        Timeframe::M15
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ProfilePlan {
    pub(super) kline_source_timeframe: Option<Timeframe>,
    pub(super) kline_volume_profiles: Vec<VolumeProfileSchedule>,
    pub(super) tpo_study: Option<KlineStudy>,
    pub(super) recurring_trade_profiles: Vec<(Timeframe, PriceStep)>,
}

impl ProfilePlan {
    pub(super) fn new(basis: Basis, active_studies: &[KlineStudy], data: &KlineData) -> Self {
        let time_basis = basis.is_time();

        let mut tpo_study = None;
        let mut kline_volume_profiles = Vec::new();
        let mut has_kline_source = false;
        for study in active_studies {
            if time_basis && tpo_study.is_none() && study.is_tpo_profile() {
                tpo_study = Some(*study);
            }
            if time_basis && study.source() == StudyDataSource::Kline {
                has_kline_source = true;
                if let Some(schedule) = study.volume_profile_schedule() {
                    kline_volume_profiles.push(schedule);
                }
            }
        }

        ProfilePlan {
            kline_source_timeframe: has_kline_source.then(|| match basis {
                Basis::Time(timeframe) => profile_source_timeframe(timeframe),
                Basis::Tick(_) => Timeframe::M1,
            }),
            kline_volume_profiles,
            tpo_study,
            recurring_trade_profiles: data
                .recurring_profile_cache_requirements(active_studies, basis),
        }
    }

    pub(super) fn uses_external_kline_source(&self, basis: data::chart::Basis) -> bool {
        matches!(
            (self.kline_source_timeframe, basis),
            (Some(source), data::chart::Basis::Time(chart)) if source != chart
        )
    }

    pub(super) fn fetch_windows(
        &self,
        visible_window: (UnixMs, UnixMs),
        tpo_ranges: &[(UnixMs, UnixMs)],
    ) -> Vec<(UnixMs, UnixMs)> {
        let mut windows = Vec::new();
        for schedule in &self.kline_volume_profiles {
            match schedule {
                VolumeProfileSchedule::VisibleRange => windows.push(visible_window),
                VolumeProfileSchedule::Recurring { interval } => {
                    let mut start = visible_window.0.floor_to(*interval);
                    let end = visible_window
                        .1
                        .saturating_sub(1)
                        .floor_to(*interval)
                        .offset_by_timeframe(*interval, 1);
                    while start < end {
                        let next = start.offset_by_timeframe(*interval, 1);
                        if next <= start {
                            break;
                        }
                        windows.push((start, next));
                        start = next;
                    }
                }
            }
        }
        windows.extend_from_slice(tpo_ranges);
        windows.retain(|(start, end)| start < end);
        windows.sort_unstable();

        let mut merged: Vec<(UnixMs, UnixMs)> = Vec::new();
        for (start, end) in windows {
            if let Some((_, previous_end)) = merged.last_mut()
                && start <= *previous_end
            {
                *previous_end = (*previous_end).max(end);
            } else {
                merged.push((start, end));
            }
        }
        merged
    }
}

pub(super) enum ProfileTradeUpdate<'a> {
    Live(&'a [Trade]),
    Historical {
        trades: &'a [Trade],
        batches_done: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum TpoEditCommand {
    SplitAt(UnixMs),
    MergeAt(UnixMs),
    MoveBoundary { from: UnixMs, to: UnixMs },
    Reset { start: UnixMs, end: UnixMs },
}

#[derive(Default)]
pub(super) struct ProfileState {
    source: ProfileKlineSource,
    tpo: TpoState,
}

impl ProfileState {
    pub(super) fn kline_fetch_windows(
        &self,
        plan: &ProfilePlan,
        visible_window: Option<(UnixMs, UnixMs)>,
        now: UnixMs,
    ) -> Vec<(UnixMs, UnixMs)> {
        let anchor = plan
            .tpo_study
            .filter(|study| study.source() == StudyDataSource::Kline)
            .and_then(|study| study.tpo_anchor())
            .unwrap_or_default();
        let fallback_start = anchor.floor(now);
        let visible_window =
            visible_window.unwrap_or_else(|| (fallback_start, anchor.offset(fallback_start, 1)));
        let tpo_ranges = if plan
            .tpo_study
            .is_some_and(|study| study.source() == StudyDataSource::Kline)
        {
            self.profile_ranges_overlapping(visible_window.0, visible_window.1, anchor)
        } else {
            Vec::new()
        };
        plan.fetch_windows(visible_window, &tpo_ranges)
    }

    pub(super) fn insert_profile_kline_bars(&mut self, timeframe: Timeframe, bars: &[Kline]) {
        let changed = self.source.insert_bars(timeframe, bars);
        self.tpo.mark_kline_bars_dirty(timeframe, &changed);
    }

    pub(super) fn insert_profile_empty_range(
        &mut self,
        timeframe: Timeframe,
        from: UnixMs,
        to: UnixMs,
    ) -> bool {
        self.source.insert_empty_range(timeframe, from, to)
    }

    pub(super) fn apply_chart_kline_bars(&mut self, timeframe: Timeframe, bars: &[Kline]) {
        let changed = bars
            .iter()
            .map(|bar| bar.time)
            .filter(|time| self.source.is_active(timeframe, *time))
            .collect::<Vec<_>>();
        self.tpo.mark_kline_bars_dirty(timeframe, &changed);
    }

    pub(super) fn apply_edit(&mut self, edit: TpoEditCommand, anchor: TpoAnchor) {
        self.tpo.apply_edit(edit, anchor);
    }

    pub(super) fn profile_source_map(
        &self,
        timeframe: Timeframe,
    ) -> Option<&BTreeMap<UnixMs, Kline>> {
        self.source.bars_ref(timeframe)
    }

    pub(super) fn profile_source_range(&self, timeframe: Timeframe) -> Option<(UnixMs, UnixMs)> {
        self.source.retained(timeframe)
    }

    pub(super) fn source_revision(&self) -> u64 {
        self.source.revision()
    }

    pub(super) fn configure_source_retention(
        &mut self,
        windows: &[(UnixMs, UnixMs)],
        visible_span: u64,
        now: UnixMs,
        timeframe: Timeframe,
        invalidate_tpo: bool,
    ) -> bool {
        let fallback_start = now.floor_to(Timeframe::D1);
        let windows = if windows.is_empty() {
            vec![(
                fallback_start,
                fallback_start.offset_by_timeframe(Timeframe::D1, 1),
            )]
        } else {
            windows.to_vec()
        };
        let changed = self
            .source
            .configure_retention(&windows, visible_span, timeframe);
        if changed && invalidate_tpo {
            self.tpo.invalidate();
        }
        changed
    }

    pub(super) fn profile_fetch_ranges(
        &mut self,
        windows: &[(UnixMs, UnixMs)],
        now: UnixMs,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
    ) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        self.source
            .fetch_ranges(windows, now, ticker_info, timeframe)
    }

    pub(super) fn clear_kline_source(&mut self) {
        self.source.clear();
    }

    pub(super) fn rebuild_tpo_from_klines(
        &mut self,
        study: Option<KlineStudy>,
        chart_source: Option<&data::chart::kline::series::KlineSeries>,
        price_step: PriceStep,
        source_timeframe: Timeframe,
        now: UnixMs,
    ) -> bool {
        let source = &self.source;
        self.tpo.rebuild(
            study,
            |scope| {
                let Some((retained_start, retained_end)) = source.retained(source_timeframe) else {
                    return Vec::new();
                };
                let (start, end) = match scope {
                    TpoBarScope::Retained => (retained_start, retained_end),
                    TpoBarScope::Session { start, end } => {
                        (start.max(retained_start), end.min(retained_end))
                    }
                };
                if start >= end {
                    return Vec::new();
                }
                if let Some(series) = chart_source {
                    let latest = end.saturating_sub(1);
                    let indices = series.time_index_range(start, latest);
                    let mut bars = Vec::with_capacity(indices.len());
                    series.for_each_point_in_range(indices, Some((start, latest)), |point| {
                        bars.push(*point.kline);
                    });
                    bars
                } else {
                    source
                        .bars_ref(source_timeframe)
                        .map(|bars| bars.range(start..end).map(|(_, bar)| *bar).collect())
                        .unwrap_or_default()
                }
            },
            price_step,
            source_timeframe,
            now,
        )
    }

    pub(super) fn rebuild_tpo_from_trades(
        &mut self,
        study: Option<KlineStudy>,
        trades: &[Trade],
        trade_sequence_id: Option<uuid::Uuid>,
        price_step: PriceStep,
        now: UnixMs,
        coverage: impl Fn(UnixMs, UnixMs) -> TpoProfileStatus,
    ) -> bool {
        self.tpo
            .rebuild_trades(study, trades, trade_sequence_id, price_step, now, coverage)
    }

    pub(super) fn clear_tpo_source(&mut self) -> bool {
        self.tpo.clear_source()
    }

    pub(super) fn apply_trade_update(&mut self, update: ProfileTradeUpdate<'_>) {
        self.tpo.apply_trade_update(update);
    }

    pub(super) fn invalidate_tpo(&mut self) {
        self.tpo.invalidate();
    }

    pub(super) fn profiles(&self) -> &[TpoProfile] {
        self.tpo.profiles()
    }

    pub(super) fn edits(&self) -> &TpoBoundaryEdits {
        self.tpo.edits()
    }

    pub(super) fn profile_ranges_overlapping(
        &self,
        start: UnixMs,
        end: UnixMs,
        anchor: TpoAnchor,
    ) -> Vec<(UnixMs, UnixMs)> {
        self.tpo.profile_ranges_overlapping(start, end, anchor)
    }

    pub(super) fn restore_edits(&mut self, edits: TpoBoundaryEdits) {
        self.tpo.restore_edits(edits);
    }
}
