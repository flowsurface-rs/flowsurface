use super::fetch::FetchCoordinator;
use super::profile::{ProfilePlan, ProfileState, ProfileTradeUpdate, TpoEditCommand};
use super::updates::{DataUpdate, DataUpdateEffect};
use crate::widget::chart::kline::studies::{
    BigTradesObject, ProfileBarSource, StudyObject, StudyObjectEvent, TpoInteraction, TpoObject,
    VolumeProfileObject,
};
use crate::widget::chart::kline::{KlineWidget, StudiesLegend, StudyLegendRow};
use crate::widget::study_configurator as study;
use data::chart::Basis;
use data::chart::kline::{
    FetchBatch, FetchRange, FetchStage, KlineData, SettingsSnapshot, TpoBoundaryEdits,
    TpoProfileStatus,
};
use data::chart::kline::{KlineStudy, KlineStudyId, SourceConstraint, StudyDataSource};
use exchange::adapter::StreamKind;
use exchange::{TickerInfo, Timeframe, UnixMs};

pub enum StudyCommand {
    Set(Vec<KlineStudy>),
    Toggle(KlineStudy),
    ToggleVisibility(KlineStudyId),
    ToggleAllVisibility,
    ToggleExpansion,
    Remove(KlineStudyId),
    RemoveAll,
}

#[derive(Clone, Copy)]
pub(super) enum StudyChange {
    Unchanged,
    Visual,
    Reconfigure,
}

#[derive(Clone)]
pub(super) struct StudySnapshot {
    pub(super) active: Vec<KlineStudy>,
    pub(super) configs: Vec<KlineStudy>,
    pub(super) edits: TpoBoundaryEdits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StudyAvailability {
    Available,
    UnsupportedBasis,
    RequiresTradeData,
    SourceConflict(StudyDataSource),
}

pub(super) struct StudyState {
    profiles: ProfileState,
    active: Vec<KlineStudy>,
    configs: Vec<KlineStudy>,
    legend_expanded: bool,
}

impl Default for StudyState {
    fn default() -> Self {
        Self {
            profiles: ProfileState::default(),
            active: Vec::new(),
            configs: Vec::new(),
            legend_expanded: true,
        }
    }
}

impl StudyState {
    fn active_big_trades_threshold(&self) -> Option<u64> {
        self.active()
            .iter()
            .filter(|study| study.visible())
            .find_map(|study| study.big_trades())
            .map(|config| config.threshold)
    }

    pub(super) fn configure_caches(
        &mut self,
        basis: Basis,
        data: &mut KlineData,
        fetch: &mut FetchCoordinator,
    ) {
        let plan = self.profile_plan(basis, data);
        if plan.kline_source_timeframe.is_none() {
            fetch.clear_pending_profile_requests();
            self.profiles.clear_kline_source();
        }
        data.configure_recurring_profile_intervals(&plan.recurring_trade_profiles);
        let big_trades_threshold = self.active_big_trades_threshold();
        data.configure_big_trades_threshold(big_trades_threshold);
        if plan.tpo_study.is_none() {
            self.profiles.clear_tpo_source();
        }
    }

    fn profile_plan(&self, basis: Basis, data: &KlineData) -> ProfilePlan {
        ProfilePlan::new(basis, self.active(), data)
    }

    pub(super) fn rebuild(&mut self, basis: Basis, data: &KlineData, now: UnixMs) -> bool {
        let plan = self.profile_plan(basis, data);
        let Some(study) = plan.tpo_study else {
            return self.profiles.clear_tpo_source();
        };
        let price_step = data.tick_size();
        match study.source() {
            StudyDataSource::Kline => {
                let source_timeframe = plan
                    .kline_source_timeframe
                    .expect("Kline TPO requires a Kline source timeframe");
                let chart_source = match basis {
                    Basis::Time(timeframe) if timeframe == source_timeframe => data.base_series(),
                    _ => None,
                };
                self.profiles.rebuild_tpo_from_klines(
                    Some(study),
                    chart_source,
                    price_step,
                    source_timeframe,
                    now,
                )
            }
            StudyDataSource::Trades => {
                let series = data.base_series();
                let trades = series.map(|series| series.raw_trades()).unwrap_or_default();
                let trade_sequence_id = series.map(|series| series.trade_sequence_id());

                let profile_status = move |_: UnixMs, end: UnixMs| {
                    if end > now {
                        TpoProfileStatus::Developing
                    } else {
                        TpoProfileStatus::Complete
                    }
                };

                self.profiles.rebuild_tpo_from_trades(
                    Some(study),
                    trades,
                    trade_sequence_id,
                    price_step,
                    now,
                    profile_status,
                )
            }
        }
    }

    pub(super) fn fetch_ranges(
        &mut self,
        basis: Basis,
        data: &KlineData,
        window: Option<(UnixMs, UnixMs)>,
        now: UnixMs,
        fetch: &mut FetchCoordinator,
        batches: &mut Vec<FetchBatch>,
    ) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        let profile_plan = self.profile_plan(basis, data);
        let base_ticker = data.base_ticker();
        if let Some(source_timeframe) = profile_plan.kline_source_timeframe {
            let windows = self
                .profiles
                .kline_fetch_windows(&profile_plan, window, now);
            let visible_span = window
                .map(|(start, end)| end.saturating_diff(start))
                .filter(|span| *span > 0)
                .unwrap_or(Timeframe::D1.to_milliseconds());

            let source_changed = self.profiles.configure_source_retention(
                &windows,
                visible_span,
                now,
                source_timeframe,
                profile_plan
                    .tpo_study
                    .is_some_and(|study| study.source() == StudyDataSource::Kline),
            );

            if source_changed {
                fetch.clear_pending_profile_requests();
            }

            if matches!(basis, Basis::Time(timeframe) if timeframe == source_timeframe) {
                if let Some((retained_start, retained_end)) =
                    self.profiles.profile_source_range(source_timeframe)
                {
                    let latest = now.floor_to(source_timeframe);
                    for (start, end) in windows {
                        let start = start.max(retained_start).floor_to(source_timeframe);
                        let end = end
                            .min(retained_end)
                            .saturating_add(source_timeframe.to_milliseconds() - 1)
                            .floor_to(source_timeframe)
                            .min(latest);
                        if start < end {
                            batches.push(FetchBatch {
                                stage: FetchStage::PrimaryPlot,
                                range: data::chart::kline::FetchRange::Kline(start, end),
                                tickers: vec![base_ticker],
                            });
                        }
                    }
                }
                Vec::new()
            } else {
                self.profiles
                    .profile_fetch_ranges(&windows, now, base_ticker, source_timeframe)
            }
        } else {
            Vec::new()
        }
    }

    fn accepts_kline_source(
        &self,
        basis: Basis,
        data: &KlineData,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
    ) -> bool {
        self.profile_plan(basis, data).kline_source_timeframe == Some(timeframe)
            && ticker_info == data.base_ticker()
    }

    pub(super) fn insert_historical_klines(
        &mut self,
        basis: Basis,
        data: &KlineData,
        fetch: &mut FetchCoordinator,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[exchange::Kline],
        is_batches_done: bool,
        has_data: bool,
    ) -> bool {
        if !self.accepts_kline_source(basis, data, timeframe, ticker_info)
            || req_id.is_some_and(|req_id| !fetch.accepts_data(req_id))
        {
            return false;
        }
        self.profiles.insert_profile_kline_bars(timeframe, klines);
        if is_batches_done
            && let Some(req_id) = req_id
            && let Some(FetchRange::ProfileKline {
                timeframe,
                from,
                to,
            }) = fetch.mark_profile_completed(ticker_info, req_id)
            && !has_data
        {
            self.profiles
                .insert_profile_empty_range(timeframe, from, to);
        }
        true
    }

    pub(super) fn apply_live_kline(
        &mut self,
        basis: Basis,
        data: &KlineData,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        kline: &exchange::Kline,
    ) -> bool {
        let plan = self.profile_plan(basis, data);
        if !plan.uses_external_kline_source(basis)
            || !self.accepts_kline_source(basis, data, timeframe, ticker_info)
        {
            return false;
        }
        self.profiles
            .insert_profile_kline_bars(timeframe, std::slice::from_ref(kline));
        true
    }

    pub(super) fn configure_widget<'a>(
        &'a self,
        chart: KlineWidget<'a>,
        basis: Basis,
        data: &KlineData,
        trade_data_enabled: bool,
    ) -> KlineWidget<'a> {
        let plan = self.profile_plan(basis, data);
        let active_studies = self.active();
        let tpo_study = active_studies.iter().find(|study| study.is_tpo_profile());
        let tpo_colors = tpo_study
            .filter(|study| study.visible())
            .and_then(|study| study.tpo_colors());
        let profile_bars = match plan.kline_source_timeframe {
            Some(timeframe) if matches!(basis, Basis::Time(chart) if chart == timeframe) => self
                .profiles
                .profile_source_range(timeframe)
                .map_or(ProfileBarSource::Empty, |retained| {
                    ProfileBarSource::Chart { retained }
                }),
            Some(timeframe) => self
                .profiles
                .profile_source_map(timeframe)
                .map_or(ProfileBarSource::Empty, ProfileBarSource::External),
            None => ProfileBarSource::Empty,
        };
        let chart = if let Some(study) = tpo_study {
            chart.with_study_object(StudyObject::Tpo(TpoObject::new(
                study.id(),
                self.profiles.profiles(),
                self.profiles.edits(),
                tpo_colors,
            )))
        } else {
            chart
        };
        let chart = if active_studies
            .iter()
            .any(|study| study.volume_profile_schedule().is_some())
        {
            chart.with_study_object(StudyObject::VolumeProfiles(VolumeProfileObject::new(
                active_studies,
                profile_bars,
            )))
        } else {
            chart
        };
        let chart = if let Some(config) = active_studies
            .iter()
            .filter(|study| study.visible())
            .find_map(|study| study.big_trades())
        {
            chart.with_study_object(StudyObject::BigTrades(BigTradesObject::new(
                config,
                trade_data_enabled,
            )))
        } else {
            chart
        };

        let study_legend = StudiesLegend {
            expanded: self.legend_expanded(),
            rows: active_studies
                .iter()
                .map(|study| StudyLegendRow {
                    id: study.id(),
                    label: study.to_string(),
                    visible: study.visible(),
                })
                .collect(),
        };

        chart.with_study_legend(study_legend)
    }

    pub(super) fn apply_data_update(
        &mut self,
        update: &DataUpdate<'_>,
        effect: &DataUpdateEffect,
        base_ticker: TickerInfo,
    ) {
        if matches!(effect, DataUpdateEffect::Unchanged) {
            return;
        }
        match update {
            DataUpdate::HistoricalKlines {
                timeframe,
                ticker_info,
                klines,
                ..
            } if *ticker_info == base_ticker
                && matches!(effect, DataUpdateEffect::HistoricalChanged) =>
            {
                self.profiles.apply_chart_kline_bars(*timeframe, klines);
            }
            DataUpdate::LiveKline {
                timeframe,
                ticker_info,
                kline,
            } if **ticker_info == base_ticker => {
                self.profiles
                    .apply_chart_kline_bars(*timeframe, std::slice::from_ref(*kline));
            }
            DataUpdate::LiveTrades {
                ticker_info,
                trades,
            } if **ticker_info == base_ticker => {
                self.profiles
                    .apply_trade_update(ProfileTradeUpdate::Live(trades));
            }
            DataUpdate::HistoricalTrades {
                ticker_info,
                trades,
                is_batches_done,
                ..
            } if *ticker_info == base_ticker => {
                self.profiles
                    .apply_trade_update(ProfileTradeUpdate::Historical {
                        trades,
                        batches_done: *is_batches_done,
                    });
            }
            _ => {}
        }
    }

    pub(super) fn apply_interaction(
        &mut self,
        event: StudyObjectEvent,
        basis: Basis,
        data: &KlineData,
    ) {
        let command = match event {
            StudyObjectEvent::Tpo(interaction) => match interaction {
                TpoInteraction::SplitAt { boundary } => TpoEditCommand::SplitAt(boundary),
                TpoInteraction::MergeLeft { boundary } => TpoEditCommand::MergeAt(boundary),
                TpoInteraction::MoveBoundary { from, to } => {
                    TpoEditCommand::MoveBoundary { from, to }
                }
                TpoInteraction::Reset { start, end } => TpoEditCommand::Reset { start, end },
            },
        };
        let anchor = self
            .profile_plan(basis, data)
            .tpo_study
            .and_then(|study| study.tpo_anchor())
            .unwrap_or_default();
        self.profiles.apply_edit(command, anchor);
    }

    pub(super) fn additional_streams(&self, basis: Basis, data: &KlineData) -> Vec<StreamKind> {
        let plan = self.profile_plan(basis, data);
        let mut streams = Vec::new();
        if plan.uses_external_kline_source(basis) {
            let profile_timeframe = plan
                .kline_source_timeframe
                .expect("external Kline source requires a timeframe");
            streams.push(StreamKind::Kline {
                ticker_info: data.base_ticker(),
                timeframe: profile_timeframe,
            });
        }
        streams
    }

    pub(super) fn active(&self) -> &[KlineStudy] {
        &self.active
    }

    pub(super) fn snapshot(&self) -> StudySnapshot {
        StudySnapshot {
            active: self.active.clone(),
            configs: self.configs.clone(),
            edits: self.profiles.edits().clone(),
        }
    }

    fn legend_expanded(&self) -> bool {
        self.legend_expanded
    }

    pub(super) fn study(&self, id: KlineStudyId) -> Option<KlineStudy> {
        self.active.iter().copied().find(|study| study.id() == id)
    }

    pub(super) fn availability(
        &self,
        study: KlineStudy,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> StudyAvailability {
        if study.normalized_for_basis(basis).is_none() {
            return StudyAvailability::UnsupportedBasis;
        }

        if let SourceConstraint::Exclusive(source) = study.source_constraint()
            && let Some(conflicting) =
                self.active
                    .iter()
                    .find_map(|active| match active.source_constraint() {
                        SourceConstraint::Exclusive(other) if other != source => Some(other),
                        SourceConstraint::Exclusive(_) | SourceConstraint::Independent => None,
                    })
        {
            return StudyAvailability::SourceConflict(conflicting);
        }

        if !trade_data_enabled && study.requires_trade_data() {
            return StudyAvailability::RequiresTradeData;
        }

        StudyAvailability::Available
    }

    pub(super) fn restore_settings(&mut self, snapshot: &SettingsSnapshot, basis: Basis) {
        self.active = snapshot.studies.clone();
        self.configs = snapshot.study_configs.clone();
        self.profiles
            .restore_edits(snapshot.tpo_profile_edits.clone());
        self.normalize_for_basis(basis);
    }

    pub(super) fn set_basis(&mut self, basis: Basis) {
        self.normalize_for_basis(basis);
        self.profiles.invalidate_tpo();
    }

    pub(super) fn runtime_revision(&self) -> u64 {
        self.profiles.source_revision()
    }

    pub(super) fn requires_trade_data(&self) -> bool {
        self.active.iter().any(|study| study.requires_trade_data())
    }

    fn normalize_for_basis(&mut self, basis: Basis) {
        self.active = Self::normalized_active(self.active.iter().copied(), basis);
        self.configs = Self::normalized_for_basis(self.configs.iter().copied(), basis);
        self.remember_configs(&self.active.clone());
    }

    pub(super) fn apply(
        &mut self,
        command: StudyCommand,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> StudyChange {
        match command {
            StudyCommand::Set(studies) => self.apply_set(studies, basis),
            StudyCommand::Toggle(study) => {
                let is_active = self.active.iter().any(|active| active.is_same_type(study));
                let Some(studies) =
                    self.studies_with_active(study, !is_active, basis, trade_data_enabled)
                else {
                    return StudyChange::Unchanged;
                };
                self.apply_set(studies, basis)
            }
            StudyCommand::ToggleVisibility(id) => {
                let Some(index) = self.active.iter().position(|study| study.id() == id) else {
                    return StudyChange::Unchanged;
                };
                let mut studies = self.active.clone();
                let study = studies[index];
                studies[index] = study.with_visibility(!study.visible());
                self.apply_set(studies, basis)
            }
            StudyCommand::ToggleAllVisibility => {
                let should_show = !self.active.iter().all(|study| study.visible());
                let mut studies = self.active.clone();
                for study in &mut studies {
                    if study.visible() != should_show {
                        *study = study.with_visibility(should_show);
                    }
                }
                self.apply_set(studies, basis)
            }
            StudyCommand::ToggleExpansion => {
                self.legend_expanded = !self.legend_expanded;
                StudyChange::Visual
            }
            StudyCommand::Remove(id) => {
                let original_len = self.active.len();
                self.active.retain(|study| study.id() != id);
                let changed = self.active.len() != original_len;
                if changed {
                    StudyChange::Reconfigure
                } else {
                    StudyChange::Unchanged
                }
            }
            StudyCommand::RemoveAll => {
                let changed = !self.active.is_empty();
                self.active.clear();
                if changed {
                    StudyChange::Reconfigure
                } else {
                    StudyChange::Unchanged
                }
            }
        }
    }

    pub(super) fn update_configurator(
        &mut self,
        message: study::Message<KlineStudy>,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> Option<StudyCommand> {
        match message {
            study::Message::StudyToggled(study, is_active) => self
                .studies_with_active(study, is_active, basis, trade_data_enabled)
                .map(StudyCommand::Set),
            study::Message::StudyValueChanged(study) => {
                let mut studies = self.active.clone();
                let active = studies
                    .iter_mut()
                    .find(|active| active.is_same_type(study))?;
                *active = study;
                Some(StudyCommand::Set(studies))
            }
        }
    }

    fn apply_set(&mut self, studies: Vec<KlineStudy>, basis: Basis) -> StudyChange {
        let normalized = Self::normalized_active(studies, basis);
        self.remember_configs(&normalized);
        if self.active == normalized {
            return StudyChange::Unchanged;
        }
        let change = if self.active.len() != normalized.len()
            || normalized
                .iter()
                .zip(&self.active)
                .any(|(study, previous)| study.requires_reconfiguration(*previous))
        {
            StudyChange::Reconfigure
        } else {
            StudyChange::Visual
        };
        self.active = normalized;
        change
    }

    fn studies_with_active(
        &self,
        study: KlineStudy,
        is_active: bool,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> Option<Vec<KlineStudy>> {
        if is_active
            && self.availability(study, basis, trade_data_enabled) != StudyAvailability::Available
        {
            return None;
        }

        let mut studies = self.active.clone();
        if is_active {
            let configured = self
                .configs
                .iter()
                .find(|saved| saved.is_same_type(study))
                .copied()
                .unwrap_or(study)
                .with_visibility(true);
            if studies.iter().any(|active| active.is_same_type(configured)) {
                return None;
            }
            studies.retain(|active| !active.same_family(configured));
            studies.push(configured);
        } else {
            let original_len = studies.len();
            studies.retain(|active| !active.is_same_type(study));
            if studies.len() == original_len {
                return None;
            }
        }
        Some(studies)
    }

    fn remember_configs(&mut self, studies: &[KlineStudy]) {
        for study in studies {
            if let Some(configured) = self
                .configs
                .iter_mut()
                .find(|configured| configured.is_same_type(*study))
            {
                *configured = *study;
            } else {
                self.configs.push(*study);
            }
        }
    }

    fn normalized_active(
        studies: impl IntoIterator<Item = KlineStudy>,
        basis: Basis,
    ) -> Vec<KlineStudy> {
        let studies = studies.into_iter().collect::<Vec<_>>();
        let source = studies
            .iter()
            .rev()
            .find_map(|study| match study.source_constraint() {
                SourceConstraint::Exclusive(source) => Some(source),
                SourceConstraint::Independent => None,
            });
        let studies = studies
            .into_iter()
            .filter(|study| match study.source_constraint() {
                SourceConstraint::Exclusive(study_source) => Some(study_source) == source,
                SourceConstraint::Independent => true,
            });

        Self::active_for_families(Self::normalized_for_basis(studies, basis))
    }

    fn normalized_for_basis(
        studies: impl IntoIterator<Item = KlineStudy>,
        basis: Basis,
    ) -> Vec<KlineStudy> {
        let mut normalized = Vec::new();
        for study in studies {
            let Some(study) = study.normalized_for_basis(basis) else {
                continue;
            };
            if !normalized
                .iter()
                .any(|existing: &KlineStudy| existing.is_same_type(study))
            {
                normalized.push(study);
            }
        }
        normalized
    }

    fn active_for_families(studies: Vec<KlineStudy>) -> Vec<KlineStudy> {
        let mut active: Vec<KlineStudy> = Vec::new();
        for study in studies {
            active.retain(|existing| !existing.same_family(study));
            active.push(study);
        }
        active
    }
}
