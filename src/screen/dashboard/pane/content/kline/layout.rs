use super::fetch::FetchCoordinator;
use data::chart::Basis;
use data::chart::kline::composition::{ChartComposition, MarkKind};
use data::chart::kline::indicators::{
    AvailabilityContext, IndicatorHost, IndicatorPanelRecipe, IndicatorUnsupportedReason,
    declared_input_requirements,
};
use data::chart::kline::{
    BaseSeriesSource, IndicatorConfig, IndicatorId, IndicatorInputCoverage,
    IndicatorInputCoverageState, IndicatorInputSource, IndicatorInstance, IndicatorInstanceId,
    IndicatorInstanceStore, IndicatorLayout, IndicatorOutputRef, IndicatorStyleOverrides,
    IndicatorUpdateScope, KlineData, KlineIndexChange, LineStrokeStyle, ThemeAccent,
};
use data::config::theme::{ThemeColor, ThemeColorShade};
use exchange::TickerInfo;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorRebuildReason {
    InputFetch,
    HistoricalDataChanged(IndicatorUpdateScope),
    BasisChanged,
    IndicatorConfigurationChanged,
    InputConfigurationChanged,
}

pub(super) enum IndicatorChange {
    Unchanged,
    Visual,
    Reconfigure {
        id: IndicatorInstanceId,
        request_fetch: bool,
    },
    Reconcile {
        request_fetch: bool,
    },
}

#[derive(Default)]
pub(super) struct IndicatorDataRequirements {
    pub(super) klines: bool,
    pub(super) directional_volume: bool,
    pub(super) trades: bool,
    pub(super) open_interest: bool,
}

pub(super) struct IndicatorRebuildPlan {
    pub(super) dirty_ids: Vec<IndicatorInstanceId>,
    pub(super) replay_ids: Vec<IndicatorInstanceId>,
    pub(super) dirty_epochs: Vec<(IndicatorInstanceId, u64)>,
}

impl IndicatorRebuildReason {
    pub(super) fn allows_partial_recalculation(self) -> bool {
        matches!(self, Self::InputFetch | Self::HistoricalDataChanged(_))
    }

    fn hides_output(self) -> bool {
        !self.allows_partial_recalculation()
    }

    fn invalidates_input_coverage(self) -> bool {
        matches!(self, Self::BasisChanged | Self::InputConfigurationChanged)
    }

    fn merge(self, next: Self) -> Self {
        if self.hides_output() && !next.hides_output() {
            return self;
        }

        match (self, next) {
            (Self::InputFetch, Self::HistoricalDataChanged(_))
            | (Self::HistoricalDataChanged(_), Self::InputFetch) => Self::InputFetch,
            (Self::HistoricalDataChanged(first), Self::HistoricalDataChanged(second)) => {
                Self::HistoricalDataChanged(first.merge(second))
            }
            (_, next) => next,
        }
    }
}

pub(super) struct IndicatorState {
    layout: IndicatorLayout,
    instances: IndicatorInstanceStore,
    input_coverage: IndicatorInputCoverage,
    dirty_reasons: FxHashMap<IndicatorInstanceId, IndicatorRebuildReason>,
    dirty_epochs: FxHashMap<IndicatorInstanceId, u64>,
    preview_dirty: bool,
}

pub enum IndicatorCommand {
    InputChanged(IndicatorInputCommand),
    StyleChanged(IndicatorStyleCommand),
    ResetSettings(IndicatorInstanceId),
}

pub enum IndicatorInputCommand {
    DataConfig {
        id: IndicatorInstanceId,
        config: IndicatorConfig,
    },
    SourceBinding {
        id: IndicatorInstanceId,
        input_id: String,
        source: IndicatorOutputRef,
    },
    BaseSource {
        id: IndicatorInstanceId,
        input_id: String,
        source: Option<BaseSeriesSource>,
    },
}

pub enum IndicatorStyleCommand {
    ToggleVisibility(IndicatorInstanceId),
    Color {
        id: IndicatorInstanceId,
        slot: String,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    },
    FillVisibility {
        id: IndicatorInstanceId,
        visible: bool,
    },
    LineWidth {
        id: IndicatorInstanceId,
        slot: String,
        width: u8,
    },
    StrokeStyle {
        id: IndicatorInstanceId,
        slot: String,
        stroke_style: LineStrokeStyle,
    },
    Mark {
        id: IndicatorInstanceId,
        mark: MarkKind,
    },
}

impl IndicatorState {
    pub(super) fn new(basis: Basis) -> Self {
        let default_volume_id = IndicatorId::parse("core.volume").expect("static ID is valid");
        let default_volume = IndicatorInstanceId::default_for(&default_volume_id);
        let instances = IndicatorInstanceStore::from_instances(
            vec![IndicatorInstance::new(default_volume, default_volume_id)],
            basis,
        );

        Self {
            layout: IndicatorLayout::new(default_volume),
            instances,
            input_coverage: IndicatorInputCoverage::default(),
            dirty_reasons: FxHashMap::default(),
            dirty_epochs: FxHashMap::default(),
            preview_dirty: false,
        }
    }

    pub(super) fn mark_rebuild_pending(&mut self, reason: IndicatorRebuildReason) {
        let selected_indicators = self.layout.selected_indicators();
        self.dirty_reasons
            .retain(|id, _| selected_indicators.contains(id));
        for id in selected_indicators {
            self.mark_rebuild(id, reason);
        }
    }

    pub(super) fn layout(&self) -> &IndicatorLayout {
        &self.layout
    }

    pub(super) fn composition(&self) -> &ChartComposition {
        self.layout.composition()
    }

    pub(super) fn composition_mut(&mut self) -> &mut ChartComposition {
        self.layout.composition_mut()
    }

    pub(super) fn replace_layout(&mut self, layout: IndicatorLayout) {
        self.layout = layout;
    }

    pub(super) fn normalize_for_basis(&mut self, basis: Basis) {
        self.instances = self.instances.normalized_for_basis(basis);
    }

    pub(super) fn replace_instances(&mut self, instances: IndicatorInstanceStore) {
        self.instances = instances;
    }

    pub(super) fn sync_comparison_sources(
        &mut self,
        base_ticker: TickerInfo,
        selected_tickers: &[TickerInfo],
    ) {
        self.layout
            .sync_comparison_sources(base_ticker, selected_tickers);
    }

    pub(super) fn reconcile_attached_indicator_placements(&mut self) -> bool {
        self.layout
            .reconcile_attached_indicator_placements(&self.instances)
    }

    pub(super) fn restore_primary_scale_after_comparisons(&mut self) -> bool {
        self.layout.restore_primary_scale_after_comparisons()
    }

    pub(super) fn layout_mut(&mut self) -> &mut IndicatorLayout {
        &mut self.layout
    }

    pub(super) fn instances(&self) -> &IndicatorInstanceStore {
        &self.instances
    }

    pub(super) fn input_coverage(&self) -> &IndicatorInputCoverage {
        &self.input_coverage
    }

    pub(super) fn remap_coverage(&mut self, change: KlineIndexChange) {
        match change {
            KlineIndexChange::None => {}
            KlineIndexChange::Prefix(prefix) => self.input_coverage.remap_prefix(prefix),
            KlineIndexChange::Explicit(previous_indices) => {
                self.input_coverage.remap(&previous_indices)
            }
        }
    }

    pub(super) fn update_coverage_frames(&mut self, frames: &[indicator_engine::IndicatorFrame]) {
        self.input_coverage.update_frames(frames);
    }

    pub(super) fn set_preview_dirty(&mut self, dirty: bool) {
        self.preview_dirty = dirty;
    }

    pub(super) fn retain_dirty_for(&mut self, active: &[IndicatorInstanceId]) {
        self.dirty_reasons.retain(|id, _| active.contains(id));
        self.dirty_epochs.retain(|id, _| active.contains(id));
    }

    pub(super) fn allow_partial_rebuilds(&mut self) {
        let changed = self
            .dirty_reasons
            .iter_mut()
            .filter_map(|(id, reason)| {
                if reason.hides_output() {
                    *reason = IndicatorRebuildReason::InputFetch;
                    Some(*id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for id in changed {
            self.bump_dirty_epoch(id);
        }
    }

    pub(super) fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        self.layout.selected_indicators()
    }

    pub(super) fn indicator(&self, id: IndicatorInstanceId) -> Option<&IndicatorInstance> {
        self.instances.get(id)
    }

    pub(super) fn indicator_kind(&self, id: IndicatorInstanceId) -> Option<IndicatorId> {
        self.indicator(id).map(|instance| instance.kind.clone())
    }

    pub(super) fn indicator_data_config(&self, id: IndicatorInstanceId) -> Option<IndicatorConfig> {
        self.indicator(id).map(|instance| instance.config.clone())
    }

    pub(super) fn indicator_style_config(
        &self,
        id: IndicatorInstanceId,
    ) -> Option<IndicatorStyleOverrides> {
        self.indicator(id)
            .map(|instance| instance.style_overrides.clone())
    }

    pub(super) fn indicator_mark_selectable(
        &self,
        id: IndicatorInstanceId,
        mark: MarkKind,
    ) -> bool {
        let Some(kind) = self.indicator_kind(id) else {
            return false;
        };
        let Some(spec) = kind.mark_spec(mark) else {
            return false;
        };
        spec.required_input.is_none_or(|required_input| {
            !matches!(
                self.input_coverage.state(required_input),
                IndicatorInputCoverageState::Missing
            )
        })
    }

    pub(super) fn primary_mark(&self) -> Option<MarkKind> {
        self.layout.primary_mark()
    }

    pub(super) fn unsupported_reason(
        &self,
        indicator: &IndicatorId,
        basis: Basis,
        base_ticker: TickerInfo,
    ) -> Option<IndicatorUnsupportedReason> {
        data::chart::kline::indicators::unsupported_reason(
            indicator,
            AvailabilityContext { basis, base_ticker },
        )
    }

    pub(super) fn history_requirement(
        &self,
        active_indicators: &[IndicatorInstanceId],
        basis: Basis,
        reference_time: exchange::UnixMs,
    ) -> data::chart::kline::IndicatorHistoryRequirement {
        self.instances
            .history_requirement(active_indicators, basis, reference_time)
    }

    pub(super) fn data_requirements(
        &self,
        basis: Basis,
        base_ticker: TickerInfo,
    ) -> IndicatorDataRequirements {
        let mut requirements = IndicatorDataRequirements::default();
        for id in self.selected_indicators() {
            let Some(instance) = self.indicator(id) else {
                continue;
            };
            if !instance.kind.is_registered()
                || self
                    .unsupported_reason(&instance.kind, basis, base_ticker)
                    .is_some()
            {
                continue;
            }
            for requirement in declared_input_requirements(&instance.kind) {
                match requirement.source {
                    IndicatorInputSource::Klines => requirements.klines = true,
                    IndicatorInputSource::DirectionalVolume => {
                        requirements.directional_volume = true;
                    }
                    IndicatorInputSource::Trades if requirement.is_required() => {
                        requirements.trades = true;
                    }
                    IndicatorInputSource::Trades => {}
                    IndicatorInputSource::OpenInterest => requirements.open_interest = true,
                }
            }
        }
        requirements
    }

    pub(super) fn requires_trade_data(
        &self,
        indicator: &IndicatorId,
        basis: Basis,
        base_ticker: TickerInfo,
    ) -> bool {
        self.unsupported_reason(indicator, basis, base_ticker)
            .is_none()
            && declared_input_requirements(indicator)
                .into_iter()
                .any(|requirement| {
                    requirement.source == IndicatorInputSource::Trades && requirement.is_required()
                })
    }

    pub(super) fn missing_required_input(
        &self,
        indicator: &IndicatorId,
        basis: Basis,
        base_ticker: TickerInfo,
    ) -> Option<IndicatorInputSource> {
        if self
            .unsupported_reason(indicator, basis, base_ticker)
            .is_some()
        {
            return None;
        }
        declared_input_requirements(indicator)
            .into_iter()
            .find(|requirement| {
                requirement.is_required()
                    && matches!(
                        self.input_coverage.state(requirement.source),
                        IndicatorInputCoverageState::Missing
                    )
            })
            .map(|requirement| requirement.source)
    }

    pub(super) fn apply(
        &mut self,
        command: IndicatorCommand,
        basis: Basis,
        data: &KlineData,
    ) -> IndicatorChange {
        match command {
            IndicatorCommand::InputChanged(IndicatorInputCommand::DataConfig { id, config }) => {
                let requests_history = self.selected_indicators().contains(&id);
                let normalized = self
                    .indicator(id)
                    .map(|instance| config.clone().normalized_for_basis(&instance.kind, basis))
                    .unwrap_or(config);
                if !self.instances.update_config(id, normalized) {
                    return IndicatorChange::Unchanged;
                }
                IndicatorChange::Reconfigure {
                    id,
                    request_fetch: requests_history,
                }
            }
            IndicatorCommand::InputChanged(IndicatorInputCommand::SourceBinding {
                id,
                input_id,
                source,
            }) => {
                if !self.selected_indicators().contains(&id)
                    || !self.selected_indicators().contains(&source.instance_id)
                    || self.indicator(id).is_none_or(|instance| {
                        !instance
                            .kind
                            .definition()
                            .source_inputs
                            .iter()
                            .any(|input| input.id == input_id && input.allow_indicator)
                    })
                    || self.indicator(source.instance_id).is_none_or(|instance| {
                        instance
                            .kind
                            .definition()
                            .sourceable_numeric_output(&source.channel_id)
                            .is_none()
                    })
                {
                    return IndicatorChange::Unchanged;
                }

                let mut candidate_instances = self.instances.clone();
                if !candidate_instances.update_source_binding(
                    id,
                    input_id.clone(),
                    Some(source.clone()),
                ) {
                    return IndicatorChange::Unchanged;
                }
                let active_indicators = self.selected_indicators();
                if data
                    .validate_indicator_source_binding(
                        &active_indicators,
                        &candidate_instances,
                        id,
                        &input_id,
                    )
                    .is_err()
                    || self.layout.indicator_output_panel_id(&source).is_none()
                {
                    return IndicatorChange::Unchanged;
                }

                let mut candidate_layout = self.layout.clone();
                let _ =
                    candidate_layout.reconcile_attached_indicator_placements(&candidate_instances);
                self.instances = candidate_instances;
                self.layout = candidate_layout;
                IndicatorChange::Reconcile {
                    request_fetch: false,
                }
            }
            IndicatorCommand::InputChanged(IndicatorInputCommand::BaseSource {
                id,
                input_id,
                source,
            }) => {
                let valid = self.selected_indicators().contains(&id)
                    && self.indicator(id).is_some_and(|instance| {
                        !instance.source_bindings.contains_key(&input_id)
                            && instance
                                .kind
                                .definition()
                                .source_inputs
                                .iter()
                                .any(|input| input.id == input_id)
                    });
                if !valid {
                    return IndicatorChange::Unchanged;
                }
                if !self.instances.update_base_source(id, input_id, source) {
                    return IndicatorChange::Unchanged;
                }
                IndicatorChange::Reconcile {
                    request_fetch: false,
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::Color {
                id,
                slot,
                accent,
                shade,
            }) => {
                let Some(current) = self.indicator(id).and_then(|instance| {
                    instance.style_overrides.color(&slot).or_else(|| {
                        instance
                            .kind
                            .color_specs()
                            .iter()
                            .find(|spec| spec.id == slot)
                            .map(|spec| spec.default_color())
                    })
                }) else {
                    return IndicatorChange::Unchanged;
                };
                if self.instances.update_color(
                    id,
                    slot,
                    ThemeColor::with_accent(current.role, accent, shade),
                ) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::FillVisibility {
                id,
                visible,
            }) => {
                if self.instances.update_fill_visibility(id, visible) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::LineWidth {
                id,
                slot,
                width,
            }) => {
                if self.instances.update_line_width(id, slot, width) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::StrokeStyle {
                id,
                slot,
                stroke_style,
            }) => {
                if self.instances.update_stroke_style(id, slot, stroke_style) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::ToggleVisibility(id)) => {
                self.toggle_indicator_visibility(id)
            }
            IndicatorCommand::ResetSettings(id) => {
                let data_changed = self.instances.reset_config(id);
                let style_changed = self.instances.reset_style(id);
                let mark_changed = self.layout.reset_indicator_mark(id);
                if data_changed {
                    IndicatorChange::Reconfigure {
                        id,
                        request_fetch: false,
                    }
                } else if style_changed || mark_changed {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::Mark { id, mark }) => {
                let kind = self.indicator(id).map(|instance| instance.kind.clone());
                if self.layout.set_indicator_mark(id, mark, kind.as_ref()) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
        }
    }

    pub(super) fn toggle_indicator_visibility(
        &mut self,
        id: IndicatorInstanceId,
    ) -> IndicatorChange {
        if self.layout.toggle_indicator_visibility(id) {
            IndicatorChange::Visual
        } else {
            IndicatorChange::Unchanged
        }
    }

    pub(super) fn add_indicator(
        &mut self,
        kind: IndicatorId,
        basis: Basis,
        data: &KlineData,
    ) -> IndicatorChange {
        if self
            .unsupported_reason(&kind, basis, data.base_ticker())
            .is_some()
        {
            return IndicatorChange::Unchanged;
        }
        let id = self.create_indicator_instance(kind.clone(), data);
        if !self
            .layout
            .add_indicator(id, kind, data.base_ticker().market_type())
        {
            let _ = self.instances.remove(id);
            return IndicatorChange::Unchanged;
        }
        IndicatorChange::Reconcile {
            request_fetch: true,
        }
    }

    pub(super) fn add_indicator_on_source(
        &mut self,
        kind: IndicatorId,
        input_id: String,
        source: IndicatorOutputRef,
        basis: Basis,
        data: &KlineData,
    ) -> IndicatorChange {
        let inputs = kind
            .definition()
            .source_inputs
            .iter()
            .filter(|input| input.allow_indicator)
            .collect::<Vec<_>>();
        if self
            .unsupported_reason(&kind, basis, data.base_ticker())
            .is_some()
            || !self.selected_indicators().contains(&source.instance_id)
            || inputs.len() != 1
            || inputs[0].id != input_id
            || !kind.definition().has_numeric_plot_layers()
            || !matches!(kind.default_view(), IndicatorPanelRecipe::Plot { .. })
            || self.indicator(source.instance_id).is_none_or(|instance| {
                instance
                    .kind
                    .definition()
                    .sourceable_numeric_output(&source.channel_id)
                    .is_none()
            })
        {
            return IndicatorChange::Unchanged;
        }

        let mut candidate_instances = self.instances.clone();
        let id = candidate_instances.create(kind.clone());
        if !candidate_instances.update_source_binding(id, input_id.clone(), Some(source.clone())) {
            return IndicatorChange::Unchanged;
        }
        let mut candidate_layout = self.layout.clone();
        if !candidate_layout.add_indicator_on_source(
            id,
            kind.clone(),
            data.base_ticker().market_type(),
            &source,
        ) {
            return IndicatorChange::Unchanged;
        }
        let active_indicators = candidate_layout.selected_indicators();
        if data
            .validate_indicator_source_binding(
                &active_indicators,
                &candidate_instances,
                id,
                &input_id,
            )
            .is_err()
        {
            return IndicatorChange::Unchanged;
        }
        self.instances = candidate_instances;
        self.layout = candidate_layout;
        if let Some(spec) = kind.color_specs().first() {
            let color = spec.default_color();
            let _ = self.instances.update_color(
                id,
                spec.id.clone(),
                ThemeColor::with_accent(
                    color.role,
                    Some(self.next_indicator_accent(data)),
                    color.shade,
                ),
            );
        }
        IndicatorChange::Reconcile {
            request_fetch: true,
        }
    }

    pub(super) fn remove_indicator(&mut self, id: IndicatorInstanceId) -> IndicatorChange {
        if self.remove_indicator_instance(id) {
            IndicatorChange::Reconcile {
                request_fetch: false,
            }
        } else {
            IndicatorChange::Unchanged
        }
    }

    fn create_indicator_instance(
        &mut self,
        kind: IndicatorId,
        data: &KlineData,
    ) -> IndicatorInstanceId {
        let id = self.instances.create(kind.clone());
        if let Some(spec) = kind.color_specs().first() {
            let color = spec.default_color();
            let _ = self.instances.update_color(
                id,
                spec.id.clone(),
                ThemeColor::with_accent(
                    color.role,
                    Some(self.next_indicator_accent(data)),
                    color.shade,
                ),
            );
        }
        id
    }

    fn next_indicator_accent(&self, data: &KlineData) -> ThemeAccent {
        ThemeAccent::ALL
            .into_iter()
            .find(|accent| {
                let used_by_series = data
                    .series()
                    .iter()
                    .any(|series| series.color().accent_value() == Some(*accent));
                let used_by_indicator = self.instances.instances().iter().any(|instance| {
                    instance.kind.color_specs().iter().any(|spec| {
                        instance
                            .style_overrides
                            .color(&spec.id)
                            .is_some_and(|color| color.accent == Some(*accent))
                    })
                });
                !used_by_series && !used_by_indicator
            })
            .unwrap_or(ThemeAccent::One)
    }

    pub(super) fn remove_indicator_instance(&mut self, id: IndicatorInstanceId) -> bool {
        let mut removal_ids = vec![id];
        let mut index = 0;
        while let Some(source_id) = removal_ids.get(index).copied() {
            index += 1;
            for dependent_id in self
                .instances
                .instances()
                .iter()
                .filter(|instance| {
                    instance
                        .source_bindings
                        .values()
                        .any(|source| source.instance_id == source_id)
                })
                .map(|instance| instance.id)
            {
                if !removal_ids.contains(&dependent_id) {
                    removal_ids.push(dependent_id);
                }
            }
        }

        let mut changed = false;
        for removal_id in removal_ids {
            changed |= self.layout.remove_indicator(removal_id);
            changed |= self.instances.remove(removal_id).is_some();
        }
        changed
    }

    pub(super) fn mark_rebuild(&mut self, id: IndicatorInstanceId, reason: IndicatorRebuildReason) {
        if !self.layout.selected_indicators().contains(&id) {
            return;
        }

        self.bump_dirty_epoch(id);
        if reason.invalidates_input_coverage() {
            self.input_coverage = IndicatorInputCoverage::default();
        }
        self.dirty_reasons
            .entry(id)
            .and_modify(|current| *current = current.merge(reason))
            .or_insert(reason);
    }

    fn bump_dirty_epoch(&mut self, id: IndicatorInstanceId) {
        let epoch = self.dirty_epochs.entry(id).or_default();
        *epoch = epoch.wrapping_add(1);
    }

    pub(super) fn dirty_epochs_match(&self, snapshot: &[(IndicatorInstanceId, u64)]) -> bool {
        snapshot
            .iter()
            .all(|(id, epoch)| self.dirty_epochs.get(id).copied().unwrap_or_default() == *epoch)
    }

    pub(super) fn rebuild_reason(&self, id: IndicatorInstanceId) -> Option<IndicatorRebuildReason> {
        self.dirty_reasons.get(&id).copied()
    }

    pub(super) fn rebuild_scope(&self, id: IndicatorInstanceId) -> IndicatorUpdateScope {
        match self.rebuild_reason(id) {
            Some(IndicatorRebuildReason::HistoricalDataChanged(scope)) => scope,
            _ => IndicatorUpdateScope::Full,
        }
    }

    pub(super) fn hides_rebuild_output(&self, id: IndicatorInstanceId) -> bool {
        self.rebuild_reason(id)
            .is_some_and(|reason| reason.hides_output())
    }

    pub(super) fn incremental_ids(&self) -> Vec<IndicatorInstanceId> {
        self.selected_indicators()
            .into_iter()
            .filter(|id| {
                self.rebuild_reason(*id)
                    .is_none_or(|reason| matches!(reason, IndicatorRebuildReason::InputFetch))
            })
            .collect()
    }

    pub(super) fn needs_rebuild(&self) -> bool {
        !self.dirty_reasons.is_empty()
    }

    pub(super) fn mark_for_inputs(
        &mut self,
        data: &KlineData,
        sources: &[IndicatorInputSource],
        reason: IndicatorRebuildReason,
    ) {
        let roots = self
            .selected_indicators()
            .into_iter()
            .filter(|id| {
                self.indicator(*id).is_some_and(|instance| {
                    sources
                        .iter()
                        .any(|source| instance.kind.definition().uses(*source))
                })
            })
            .collect::<Vec<_>>();
        self.mark_with_dependents(data, &roots, reason);
    }

    pub(super) fn mark_for_candle_ticker(
        &mut self,
        data: &KlineData,
        ticker: TickerInfo,
        reason: IndicatorRebuildReason,
    ) {
        if ticker == data.base_ticker() {
            self.mark_rebuild_pending(reason);
        }
    }

    pub(super) fn mark_for_trade_ticker(
        &mut self,
        data: &KlineData,
        ticker: TickerInfo,
        reason: IndicatorRebuildReason,
    ) {
        if ticker != data.base_ticker() {
            return;
        }
        let roots = self
            .selected_indicators()
            .into_iter()
            .filter(|id| self.trade_request_affects(*id))
            .collect::<Vec<_>>();
        self.mark_with_dependents(data, &roots, reason);
    }

    pub(super) fn trade_request_affects_indicator(
        &self,
        id: IndicatorInstanceId,
        data: &KlineData,
        ticker: TickerInfo,
    ) -> bool {
        ticker == data.base_ticker() && self.trade_request_affects(id)
    }

    pub(super) fn mark_with_dependents(
        &mut self,
        data: &KlineData,
        roots: &[IndicatorInstanceId],
        reason: IndicatorRebuildReason,
    ) {
        for id in data.dependent_indicator_ids(roots) {
            self.mark_rebuild(id, reason);
        }
    }

    fn rebuild_groups(
        &self,
        data: &KlineData,
        dirty: &[IndicatorInstanceId],
    ) -> Vec<Vec<IndicatorInstanceId>> {
        let mut groups: Vec<Vec<IndicatorInstanceId>> = Vec::new();

        for root in dirty {
            let mut group = data
                .dependent_indicator_ids(&[*root])
                .into_iter()
                .filter(|id| dirty.contains(id))
                .collect::<Vec<_>>();
            if !group.contains(root) {
                group.push(*root);
            }

            let mut index = 0;
            while index < groups.len() {
                if groups[index].iter().any(|id| group.contains(id)) {
                    let merged = groups.remove(index);
                    for id in merged {
                        if !group.contains(&id) {
                            group.push(id);
                        }
                    }
                    index = 0;
                } else {
                    index += 1;
                }
            }
            groups.push(group);
        }

        groups
    }

    pub(super) fn full_rebuild_plan(
        &self,
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> Option<IndicatorRebuildPlan> {
        let active = self.selected_indicators();
        let dirty = self
            .dirty_reasons
            .keys()
            .copied()
            .filter(|id| active.contains(id))
            .collect::<Vec<_>>();
        if dirty.is_empty() {
            return None;
        }

        let mut dirty_ids = Vec::new();
        let mut replay_ids = Vec::new();
        for group in self.rebuild_groups(data, &dirty) {
            let calculation_ids = data.calculation_indicator_ids(&group);
            let waits_for_complete_inputs = group.iter().any(|id| {
                self.rebuild_reason(*id)
                    .is_some_and(|reason| !reason.allows_partial_recalculation())
            });
            if waits_for_complete_inputs && self.has_pending_inputs(&calculation_ids, data, fetch) {
                continue;
            }

            let scope = group
                .iter()
                .map(|id| self.rebuild_scope(*id))
                .reduce(IndicatorUpdateScope::merge)
                .unwrap_or(IndicatorUpdateScope::Full);
            let scope = if scope.is_full() || data.has_indicator_entries(&group) {
                scope
            } else {
                IndicatorUpdateScope::Full
            };
            if !scope.is_full() {
                continue;
            }

            for id in group {
                if !dirty_ids.contains(&id) {
                    dirty_ids.push(id);
                }
            }
            for id in calculation_ids {
                if !replay_ids.contains(&id) {
                    replay_ids.push(id);
                }
            }
        }

        (!dirty_ids.is_empty()).then(|| {
            let dirty_epochs = replay_ids
                .iter()
                .map(|id| (*id, self.dirty_epochs.get(id).copied().unwrap_or_default()))
                .collect();
            IndicatorRebuildPlan {
                dirty_ids,
                replay_ids,
                dirty_epochs,
            }
        })
    }

    pub(super) fn accept_full_rebuild(
        &mut self,
        data: &mut KlineData,
        host: IndicatorHost,
        frames: &[indicator_engine::IndicatorFrame],
        dirty_ids: &[IndicatorInstanceId],
        replay_ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) {
        let active_ids = self.selected_indicators();
        data.merge_indicator_host(host, replay_ids, &active_ids, &self.instances);
        self.input_coverage = data.indicator_input_coverage_for_frames(frames, trade_data_enabled);
        self.dirty_reasons.retain(|id, _| !dirty_ids.contains(id));
        self.preview_dirty = self.dirty_reasons.keys().any(|id| active_ids.contains(id));
    }

    pub(super) fn recalculate(
        &mut self,
        data: &mut KlineData,
        fetch: &FetchCoordinator,
        trade_data_enabled: bool,
    ) -> bool {
        if !self.needs_rebuild() {
            return false;
        }
        let active = self.selected_indicators();
        self.dirty_reasons.retain(|id, _| active.contains(id));
        let dirty = self.dirty_reasons.keys().copied().collect::<Vec<_>>();

        let mut rebuilt_ids = Vec::new();
        let mut incremental_frames = Vec::new();
        let mut changed = false;

        for group in self.rebuild_groups(data, &dirty) {
            let waits_for_complete_inputs = group.iter().any(|id| {
                self.rebuild_reason(*id)
                    .is_some_and(|reason| !reason.allows_partial_recalculation())
            });
            if waits_for_complete_inputs && self.has_pending_inputs(&group, data, fetch) {
                continue;
            }

            let scope = group
                .iter()
                .map(|id| self.rebuild_scope(*id))
                .reduce(IndicatorUpdateScope::merge)
                .unwrap_or(IndicatorUpdateScope::Full);
            let scope = if scope.is_full() || data.has_indicator_entries(&group) {
                scope
            } else {
                IndicatorUpdateScope::Full
            };
            if scope.is_full() {
                continue;
            }
            let frames =
                data.recalculate_indicators(&self.instances, &group, scope, trade_data_enabled);
            incremental_frames.extend(frames.iter().cloned());
            self.dirty_reasons.retain(|id, _| !group.contains(id));
            rebuilt_ids.extend(group);
            changed = true;
        }

        if !changed {
            return false;
        }

        self.input_coverage.update_frames(&incremental_frames);
        if let Some(frame) = data.latest_indicator_frame(trade_data_enabled) {
            self.input_coverage.update_frames(&[frame]);
        }

        let clean_preview = if self.preview_dirty {
            self.incremental_ids()
                .into_iter()
                .filter(|id| !rebuilt_ids.contains(id))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if !clean_preview.is_empty() {
            data.preview_indicator_open_for(&clean_preview, trade_data_enabled);
            self.preview_dirty = false;
        } else if self.dirty_reasons.is_empty() {
            self.preview_dirty = false;
        }
        changed
    }

    pub(super) fn preview_clean(&mut self, data: &mut KlineData, trade_data_enabled: bool) -> bool {
        if !self.preview_dirty {
            return false;
        }
        let ids = self.incremental_ids();
        data.preview_indicator_open_for(&ids, trade_data_enabled);
        self.preview_dirty = false;
        true
    }

    fn has_pending_inputs(
        &self,
        ids: &[IndicatorInstanceId],
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> bool {
        ids.iter()
            .any(|id| self.has_pending_input(*id, data, fetch))
    }

    pub(super) fn pending_input_ids(
        &self,
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> Vec<IndicatorInstanceId> {
        self.selected_indicators()
            .into_iter()
            .filter(|id| self.has_pending_input(*id, data, fetch))
            .collect()
    }

    fn has_pending_input(
        &self,
        id: IndicatorInstanceId,
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> bool {
        fetch
            .pending_requests()
            .any(|(ticker, range)| self.request_blocks(id, ticker, range, data))
    }

    fn request_blocks(
        &self,
        id: IndicatorInstanceId,
        ticker: TickerInfo,
        range: data::chart::kline::FetchRange,
        data: &KlineData,
    ) -> bool {
        let Some(instance) = self.indicator(id) else {
            return false;
        };
        match range {
            data::chart::kline::FetchRange::Kline(..) => ticker == data.base_ticker(),
            data::chart::kline::FetchRange::OpenInterest { .. } => instance
                .kind
                .definition()
                .uses(IndicatorInputSource::OpenInterest),
            data::chart::kline::FetchRange::Trades(..) => self.trade_request_affects(id),
        }
    }

    fn trade_request_affects(&self, id: IndicatorInstanceId) -> bool {
        let Some(instance) = self.indicator(id) else {
            return false;
        };
        let definition = instance.kind.definition();
        // Later pages can extend the range after the first directional sample.
        definition.uses(IndicatorInputSource::Trades)
            || definition.uses(IndicatorInputSource::DirectionalVolume)
    }
}
