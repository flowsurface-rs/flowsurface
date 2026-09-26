use data::chart::kline::{
    KlineStudy, StudyDataSource, TPO_TRADE_COVERAGE_TIMEFRAME, TpoAnchor, TpoBoundaryEdits,
    TpoCalculationError, TpoProfile, TpoProfileSettings, TpoProfileStatus, TradeTpoCache,
    build_tpo_profile_with_source_timeframe, build_tpo_profiles_with_source_timeframe,
};
use exchange::unit::price::PriceStep;
use exchange::{Kline, Timeframe, Trade, UnixMs};
use std::collections::BTreeSet;

use super::{ProfileTradeUpdate, TpoEditCommand};

#[derive(Clone, PartialEq, Eq)]
struct TpoBuildKey {
    source: StudyDataSource,
    source_timeframe: Timeframe,
    settings: TpoProfileSettings,
    edits: TpoBoundaryEdits,
}

#[derive(Default, Clone)]
enum TpoRebuildState {
    #[default]
    Clean,
    Full,
    Profiles(BTreeSet<UnixMs>),
}

#[derive(Default)]
pub(super) struct TpoState {
    profiles: Vec<TpoProfile>,
    edits: TpoBoundaryEdits,
    error: Option<TpoCalculationError>,
    build_key: Option<TpoBuildKey>,
    rebuild_state: TpoRebuildState,
    last_rebuild_source: Option<UnixMs>,
    trade_cache: Option<TradeTpoCache>,
    trade_cache_sequence_id: Option<uuid::Uuid>,
}

impl TpoState {
    pub fn invalidate(&mut self) {
        self.rebuild_state = TpoRebuildState::Full;
        self.last_rebuild_source = None;
    }

    pub fn clear_source(&mut self) -> bool {
        let changed = !self.profiles.is_empty() || self.error.is_some();
        self.profiles.clear();
        self.error = None;
        self.build_key = None;
        self.rebuild_state = TpoRebuildState::Clean;
        self.last_rebuild_source = None;
        self.trade_cache = None;
        self.trade_cache_sequence_id = None;
        changed
    }

    pub fn rebuild(
        &mut self,
        study: Option<KlineStudy>,
        load_bars: impl FnOnce() -> Vec<Kline>,
        price_step: PriceStep,
        source_timeframe: Timeframe,
        now: UnixMs,
    ) -> bool {
        let Some(KlineStudy::TpoProfile {
            source: StudyDataSource::Kline,
            anchor,
            letter_period,
            value_area_percent,
            ..
        }) = study
        else {
            return self.clear_source();
        };

        self.trade_cache = None;
        self.trade_cache_sequence_id = None;
        let settings = TpoProfileSettings {
            anchor,
            letter_period,
            price_step,
            value_area_percent,
        };
        let key = TpoBuildKey {
            source: StudyDataSource::Kline,
            source_timeframe,
            settings,
            edits: self.edits.clone(),
        };
        self.prepare_rebuild(&key, now);
        let Some(rebuild_state) = self.take_rebuild_state() else {
            return false;
        };
        let bars = load_bars();
        let result = match rebuild_state {
            TpoRebuildState::Full => {
                self.rebuild_all_profiles(&bars, settings, source_timeframe, now)
            }
            TpoRebuildState::Profiles(dirty) => {
                match self.rebuild_dirty_profiles(&bars, &dirty, settings, source_timeframe, now) {
                    Some(result) => result,
                    None => self.rebuild_all_profiles(&bars, settings, source_timeframe, now),
                }
            }
            TpoRebuildState::Clean => unreachable!(),
        };

        self.apply_result(key, result, now)
    }

    pub fn rebuild_trades(
        &mut self,
        study: Option<KlineStudy>,
        trades: &[Trade],
        trade_sequence_id: Option<uuid::Uuid>,
        price_step: PriceStep,
        now: UnixMs,
        coverage: impl Fn(UnixMs, UnixMs) -> TpoProfileStatus,
    ) -> bool {
        let Some(KlineStudy::TpoProfile {
            source: StudyDataSource::Trades,
            anchor,
            letter_period,
            value_area_percent,
            ..
        }) = study
        else {
            return self.clear_source();
        };

        let settings = TpoProfileSettings {
            anchor,
            letter_period,
            price_step,
            value_area_percent,
        };
        let key = TpoBuildKey {
            source: StudyDataSource::Trades,
            source_timeframe: TPO_TRADE_COVERAGE_TIMEFRAME,
            settings,
            edits: self.edits.clone(),
        };
        if self.prepare_rebuild(&key, now) || self.trade_cache_sequence_id != trade_sequence_id {
            self.trade_cache = None;
        }
        self.trade_cache_sequence_id = trade_sequence_id;
        let cache_synced = self
            .trade_cache
            .as_mut()
            .is_some_and(|cache| cache.sync(trades));
        if !cache_synced {
            let cache = match TradeTpoCache::new(trades, settings, self.edits.clone()) {
                Ok(cache) => cache,
                Err(error) => return self.apply_result(key, Err(error), now),
            };
            self.trade_cache = Some(cache);
            self.rebuild_state = TpoRebuildState::Full;
        }

        let Some(rebuild_state) = self.take_rebuild_state() else {
            return false;
        };
        let cache = self
            .trade_cache
            .as_ref()
            .expect("trade TPO cache initialized before rebuilding");
        let result = match rebuild_state {
            TpoRebuildState::Clean => unreachable!(),
            TpoRebuildState::Full => cache.build_profiles(now, &coverage),
            TpoRebuildState::Profiles(dirty) => {
                match self.rebuild_dirty_trade_profiles(cache, &dirty, now, &coverage) {
                    Some(result) => result,
                    None => cache.build_profiles(now, &coverage),
                }
            }
        };
        self.apply_result(key, result, now)
    }

    fn prepare_rebuild(&mut self, key: &TpoBuildKey, now: UnixMs) -> bool {
        self.mark_active_profile_dirty(now, key.source_timeframe);
        let changed = self.build_key.as_ref() != Some(key);
        if changed {
            self.rebuild_state = TpoRebuildState::Full;
        }
        changed
    }

    fn take_rebuild_state(&mut self) -> Option<TpoRebuildState> {
        match std::mem::take(&mut self.rebuild_state) {
            TpoRebuildState::Clean => None,
            state => Some(state),
        }
    }

    fn apply_result(
        &mut self,
        key: TpoBuildKey,
        result: Result<Vec<TpoProfile>, TpoCalculationError>,
        now: UnixMs,
    ) -> bool {
        let source_timeframe = key.source_timeframe;
        self.build_key = Some(key);
        self.last_rebuild_source = Some(now.floor_to(source_timeframe));
        let changed = match result {
            Ok(profiles) => {
                let changed = self.profiles != profiles || self.error.is_some();
                if changed {
                    self.profiles = profiles;
                }
                self.error = None;
                changed
            }
            Err(error) => {
                let changed = !self.profiles.is_empty() || self.error != Some(error);
                self.profiles.clear();
                self.error = Some(error);
                changed
            }
        };
        self.rebuild_state = TpoRebuildState::Clean;
        changed
    }

    fn rebuild_all_profiles(
        &self,
        bars: &[Kline],
        settings: TpoProfileSettings,
        source_timeframe: Timeframe,
        now: UnixMs,
    ) -> Result<Vec<TpoProfile>, TpoCalculationError> {
        build_tpo_profiles_with_source_timeframe(
            bars,
            settings,
            self.edits.clone(),
            source_timeframe,
            now,
        )
    }

    fn rebuild_dirty_profiles(
        &self,
        bars: &[Kline],
        dirty: &BTreeSet<UnixMs>,
        settings: TpoProfileSettings,
        source_timeframe: Timeframe,
        now: UnixMs,
    ) -> Option<Result<Vec<TpoProfile>, TpoCalculationError>> {
        let mut profiles = self.profiles.clone();
        for start in dirty {
            let index = profiles
                .binary_search_by_key(start, |profile| profile.start)
                .ok()?;
            let end = profiles[index].end;
            let bars = bars
                .iter()
                .copied()
                .filter(|bar| bar.time >= *start && bar.time < end)
                .collect::<Vec<_>>();
            profiles[index] = match build_tpo_profile_with_source_timeframe(
                &bars,
                *start,
                end,
                settings,
                source_timeframe,
                now,
            ) {
                Ok(profile) => profile,
                Err(error) => return Some(Err(error)),
            };
        }
        Some(Ok(profiles))
    }

    fn rebuild_dirty_trade_profiles(
        &self,
        cache: &TradeTpoCache,
        dirty: &BTreeSet<UnixMs>,
        now: UnixMs,
        coverage: &impl Fn(UnixMs, UnixMs) -> TpoProfileStatus,
    ) -> Option<Result<Vec<TpoProfile>, TpoCalculationError>> {
        let mut profiles = self.profiles.clone();
        for start in dirty {
            let index = profiles
                .binary_search_by_key(start, |profile| profile.start)
                .ok()?;
            let end = profiles[index].end;
            let profile = match cache.build_profile(*start, now, coverage(*start, end))? {
                Ok(profile) => profile,
                Err(error) => return Some(Err(error)),
            };
            profiles[index] = profile;
        }
        Some(Ok(profiles))
    }

    fn uses_source(&self, source: StudyDataSource, source_timeframe: Timeframe) -> bool {
        self.build_key
            .as_ref()
            .is_some_and(|key| key.source == source && key.source_timeframe == source_timeframe)
    }

    pub fn mark_kline_bars_dirty(&mut self, timeframe: Timeframe, bars: &[UnixMs]) {
        if self.uses_source(StudyDataSource::Kline, timeframe) {
            self.mark_times_dirty(bars.iter().copied());
        }
    }

    pub fn apply_trade_update(&mut self, update: ProfileTradeUpdate<'_>) {
        match update {
            ProfileTradeUpdate::Live(trades) => self.mark_live_trades_dirty(trades),
            ProfileTradeUpdate::Historical {
                trades,
                batches_done,
            } => self.mark_historical_trades_dirty(trades, batches_done),
        }
    }

    fn mark_live_trades_dirty(&mut self, trades: &[Trade]) {
        if self.uses_source(StudyDataSource::Trades, TPO_TRADE_COVERAGE_TIMEFRAME) {
            self.mark_times_dirty(trades.iter().map(|trade| trade.time));
        }
    }

    fn mark_historical_trades_dirty(&mut self, trades: &[Trade], is_batches_done: bool) {
        if !self.uses_source(StudyDataSource::Trades, TPO_TRADE_COVERAGE_TIMEFRAME) {
            return;
        }
        self.mark_times_dirty(trades.iter().map(|trade| trade.time));
        if is_batches_done {
            let incomplete = self
                .profiles
                .iter()
                .filter(|profile| {
                    profile.status == data::chart::kline::TpoProfileStatus::Incomplete
                })
                .map(|profile| profile.start)
                .collect::<Vec<_>>();
            for start in incomplete {
                self.mark_profile_dirty(start);
            }
        }
    }

    fn mark_times_dirty(&mut self, times: impl IntoIterator<Item = UnixMs>) {
        if matches!(self.rebuild_state, TpoRebuildState::Full) {
            return;
        }

        let mut dirty = BTreeSet::new();
        for time in times {
            let Some(profile_start) = self.profile_start_for(time) else {
                self.rebuild_state = TpoRebuildState::Full;
                return;
            };
            dirty.insert(profile_start);
        }
        for profile_start in dirty {
            self.mark_profile_dirty(profile_start);
        }
    }

    fn profile_start_for(&self, time: UnixMs) -> Option<UnixMs> {
        let index = self
            .profiles
            .partition_point(|profile| profile.start <= time);
        index.checked_sub(1).and_then(|index| {
            let profile = &self.profiles[index];
            (time < profile.end).then_some(profile.start)
        })
    }

    fn mark_profile_dirty(&mut self, start: UnixMs) {
        if matches!(self.rebuild_state, TpoRebuildState::Full) {
            return;
        }
        if let TpoRebuildState::Profiles(dirty) = &mut self.rebuild_state {
            dirty.insert(start);
            return;
        }
        let mut dirty = BTreeSet::new();
        dirty.insert(start);
        self.rebuild_state = TpoRebuildState::Profiles(dirty);
    }

    fn mark_active_profile_dirty(&mut self, now: UnixMs, source_timeframe: Timeframe) {
        let source_time = now.floor_to(source_timeframe);
        if self.last_rebuild_source == Some(source_time) {
            return;
        }
        self.last_rebuild_source = Some(source_time);
        let active_profile = self
            .profiles
            .iter()
            .find(|profile| {
                profile.start <= source_time
                    && source_time < profile.end
                    && profile.status != data::chart::kline::TpoProfileStatus::Complete
            })
            .map(|profile| profile.start);
        if let Some(start) = active_profile {
            self.mark_profile_dirty(start);
        }
    }

    pub fn profiles(&self) -> &[TpoProfile] {
        &self.profiles
    }

    pub fn edits(&self) -> &TpoBoundaryEdits {
        &self.edits
    }

    pub fn profile_ranges_overlapping(
        &self,
        start: UnixMs,
        end: UnixMs,
        anchor: TpoAnchor,
    ) -> Vec<(UnixMs, UnixMs)> {
        self.edits.profile_ranges_overlapping(start, end, anchor)
    }

    pub fn restore_edits(&mut self, edits: TpoBoundaryEdits) {
        self.edits = edits;
    }

    pub fn apply_edit(&mut self, edit: TpoEditCommand, anchor: TpoAnchor) {
        match edit {
            TpoEditCommand::SplitAt(boundary) => self.edits.split_at(boundary),
            TpoEditCommand::MergeAt(boundary) => self.edits.merge_at(boundary),
            TpoEditCommand::MoveBoundary { from, to } => {
                if from != to {
                    self.edits.move_boundary(from, to, anchor);
                }
            }
            TpoEditCommand::Reset { start, end } => self.edits.clear_range(start, end, anchor),
        }
    }
}
