mod source;
mod tpo;

use data::chart::kline::VolumeProfileSchedule;
use data::chart::kline::{
    FetchRange, KlineStudy, TpoAnchor, TpoBoundaryEdits, TpoProfile, TpoProfileStatus,
};
use exchange::unit::price::PriceStep;
use exchange::{Kline, TickerInfo, Timeframe, Trade, UnixMs};
use std::collections::BTreeMap;

use self::source::ProfileKlineSource;
use self::tpo::TpoState;

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
        &self,
        windows: &[(UnixMs, UnixMs)],
        now: UnixMs,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        covered_bars: &[Kline],
    ) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        self.source
            .fetch_ranges(windows, now, ticker_info, timeframe, covered_bars)
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
            || {
                chart_source.map_or_else(
                    || source.bars(source_timeframe),
                    |series| {
                        series
                            .points()
                            .filter(|point| source.is_active(source_timeframe, point.kline.time))
                            .map(|point| *point.kline)
                            .collect()
                    },
                )
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
