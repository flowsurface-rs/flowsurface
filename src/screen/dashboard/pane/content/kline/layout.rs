use super::fetch::FetchCoordinator;
use super::presentation::{IndicatorPresentationCache, IndicatorPresentationResolver};
use super::rebuild::{RebuildCoordinator, RebuildSnapshot};
use super::{DataRequirements, UpdatePolicy};
use crate::widget::chart::kline::IndicatorPresentationContext;
use data::chart::Basis;
use data::chart::kline::composition::{ChartComposition, MarkKind};
use data::chart::kline::indicators::{
    AvailabilityContext, IndicatorHost, IndicatorPanelRecipe, declared_input_requirements,
    unsupported_reason,
};
use data::chart::kline::{
    BaseSeriesSource, IndicatorConfig, IndicatorId, IndicatorInputCoverage,
    IndicatorInputCoverageState, IndicatorInputSource, IndicatorInstance, IndicatorInstanceId,
    IndicatorInstanceStore, IndicatorOutputRef, IndicatorStyleOverrides, IndicatorUpdateScope,
    KlineData, KlineIndexChange, LineStrokeStyle, ThemeAccent,
};
use data::config::theme::{ThemeColor, ThemeColorShade};
use exchange::TickerInfo;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorRebuildReason {
    InputFetch,
    HistoricalDataChanged(IndicatorUpdateScope),
    IndicatorSettingsChanged,
    BasisChanged,
    IndicatorStructureChanged,
    TradeDataModeChanged,
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

impl IndicatorChange {
    pub(super) fn apply(
        self,
        indicators: &mut IndicatorState,
        data: &KlineData,
    ) -> (Option<UpdatePolicy>, bool) {
        match self {
            IndicatorChange::Unchanged => (None, false),
            IndicatorChange::Visual => (Some(UpdatePolicy::Redraw), false),
            IndicatorChange::Reconfigure { id, request_fetch } => {
                indicators.mark_with_dependents(
                    data,
                    &[id],
                    IndicatorRebuildReason::IndicatorSettingsChanged,
                );
                (Some(UpdatePolicy::DeferredReconfigure), request_fetch)
            }
            IndicatorChange::Reconcile { request_fetch } => {
                (Some(UpdatePolicy::ImmediateReconfigure), request_fetch)
            }
        }
    }
}

pub(super) struct IndicatorRebuildPlan {
    dirty_ids: Vec<IndicatorInstanceId>,
    pub(super) replay_ids: Vec<IndicatorInstanceId>,
    validated_epochs: Vec<(IndicatorInstanceId, u64)>,
}

struct IndicatorRebuildGroup {
    dirty_ids: Vec<IndicatorInstanceId>,
    calculation_ids: Vec<IndicatorInstanceId>,
    scope: IndicatorUpdateScope,
}

impl IndicatorRebuildReason {
    fn allows_partial_recalculation(self) -> bool {
        matches!(
            self,
            Self::InputFetch | Self::HistoricalDataChanged(_) | Self::IndicatorSettingsChanged
        )
    }

    fn hides_output(self) -> bool {
        !self.allows_partial_recalculation()
    }

    fn invalidates_input_coverage(self) -> bool {
        matches!(self, Self::BasisChanged | Self::TradeDataModeChanged)
    }

    fn merge(self, next: Self) -> Self {
        if self.hides_output() && !next.hides_output() {
            return self;
        }

        match (self, next) {
            (Self::IndicatorSettingsChanged, Self::InputFetch | Self::HistoricalDataChanged(_)) => {
                Self::IndicatorSettingsChanged
            }
            (Self::InputFetch, Self::HistoricalDataChanged(_))
            | (Self::HistoricalDataChanged(_), Self::InputFetch) => {
                Self::HistoricalDataChanged(IndicatorUpdateScope::Full)
            }
            (Self::HistoricalDataChanged(first), Self::HistoricalDataChanged(second)) => {
                Self::HistoricalDataChanged(first.merge(second))
            }
            (_, next) => next,
        }
    }
}

pub(super) struct IndicatorState {
    presentation: IndicatorPresentationCache,
    pub(super) composition: ChartComposition,
    pub(super) instances: IndicatorInstanceStore,
    pub(super) input_coverage: IndicatorInputCoverage,
    dirty_reasons: FxHashMap<IndicatorInstanceId, IndicatorRebuildReason>,
    dirty_epochs: FxHashMap<IndicatorInstanceId, u64>,
    preview_dirty: bool,
    rebuild: RebuildCoordinator,
}

pub enum IndicatorCommand {
    InputChanged(IndicatorInputCommand),
    StyleChanged(IndicatorStyleCommand),
}

pub enum IndicatorInputCommand {
    Reset(IndicatorInstanceId),
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
    Reset(IndicatorInstanceId),
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
    pub(super) fn new() -> Self {
        let default_volume_id = IndicatorId::parse("core.volume").expect("static ID is valid");
        let default_volume = IndicatorInstanceId::default_for(&default_volume_id);
        let instances = IndicatorInstanceStore::from_instances(vec![IndicatorInstance::new(
            default_volume,
            default_volume_id.clone(),
        )]);

        let mut composition = ChartComposition::prototype_kline();
        let _ = composition.add_indicator(default_volume, default_volume_id);

        Self {
            presentation: IndicatorPresentationCache::default(),
            composition,
            instances,
            input_coverage: IndicatorInputCoverage::default(),
            dirty_reasons: FxHashMap::default(),
            dirty_epochs: FxHashMap::default(),
            preview_dirty: false,
            rebuild: RebuildCoordinator::default(),
        }
    }

    pub(super) fn refresh_presentation(
        &mut self,
        data: &KlineData,
        basis: Basis,
        trade_data_enabled: bool,
    ) {
        let resolver = IndicatorPresentationResolver::new(self, data, basis, trade_data_enabled);
        if let Some(presentation) = self.presentation.refreshed(resolver) {
            self.presentation = presentation;
        }
    }

    pub(super) fn presentation_context(&self) -> IndicatorPresentationContext {
        self.presentation.context(self)
    }

    pub(super) fn mark_rebuild_pending(&mut self, reason: IndicatorRebuildReason) {
        let selected_indicators = self.composition.selected_indicators();
        self.dirty_reasons
            .retain(|id, _| selected_indicators.contains(id));
        for id in selected_indicators {
            self.mark_rebuild(id, reason);
        }
    }

    pub(super) fn reconcile_attached_indicator_placements(&mut self) -> bool {
        self.composition
            .reconcile_attached_indicator_placements(&self.instances)
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
        self.composition.selected_indicators()
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
        self.composition.primary_mark()
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
    ) -> DataRequirements {
        let mut requirements = DataRequirements::default();
        for id in self.selected_indicators() {
            let Some(instance) = self.indicator(id) else {
                continue;
            };
            if !instance.kind.is_registered()
                || unsupported_reason(&instance.kind, AvailabilityContext { basis, base_ticker })
                    .is_some()
            {
                continue;
            }
            for requirement in declared_input_requirements(&instance.kind) {
                match requirement.source {
                    IndicatorInputSource::Klines | IndicatorInputSource::DirectionalVolume => {}
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
        unsupported_reason(indicator, AvailabilityContext { basis, base_ticker }).is_none()
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
        if unsupported_reason(indicator, AvailabilityContext { basis, base_ticker }).is_some() {
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

    pub(super) fn apply(&mut self, command: IndicatorCommand) -> IndicatorChange {
        match command {
            IndicatorCommand::InputChanged(IndicatorInputCommand::DataConfig { id, config }) => {
                let requests_history = self.selected_indicators().contains(&id);
                if !self.instances.update_config(id, config) {
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
                let mut candidate_instances = self.instances.clone();
                if !candidate_instances.update_source_binding(
                    id,
                    input_id.clone(),
                    Some(source.clone()),
                ) {
                    return IndicatorChange::Unchanged;
                }
                let active_indicators = self.selected_indicators();
                if candidate_instances
                    .validate_source_binding(&active_indicators, id, &input_id)
                    .is_err()
                    || self
                        .composition
                        .indicator_output_panel_id(&source)
                        .is_none()
                {
                    return IndicatorChange::Unchanged;
                }

                let mut candidate_composition = self.composition.clone();
                let _ = candidate_composition
                    .reconcile_attached_indicator_placements(&candidate_instances);
                self.instances = candidate_instances;
                self.composition = candidate_composition;
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
            IndicatorCommand::InputChanged(IndicatorInputCommand::Reset(id)) => {
                let data_changed = self.instances.reset_config(id);
                if data_changed {
                    IndicatorChange::Reconfigure {
                        id,
                        request_fetch: false,
                    }
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::Reset(id)) => {
                let style_changed = self.instances.reset_style(id);
                let mark_changed = self.composition.reset_indicator_mark(id);
                if style_changed || mark_changed {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
            IndicatorCommand::StyleChanged(IndicatorStyleCommand::Mark { id, mark }) => {
                let kind = self.indicator(id).map(|instance| instance.kind.clone());
                if self.composition.set_indicator_mark(id, mark, kind.as_ref()) {
                    IndicatorChange::Visual
                } else {
                    IndicatorChange::Unchanged
                }
            }
        }
    }

    fn toggle_indicator_visibility(&mut self, id: IndicatorInstanceId) -> IndicatorChange {
        if self.composition.toggle_indicator_visibility(id) {
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
        if unsupported_reason(
            &kind,
            AvailabilityContext {
                basis,
                base_ticker: data.base_ticker(),
            },
        )
        .is_some()
        {
            return IndicatorChange::Unchanged;
        }
        let id = self.create_indicator_instance(kind.clone(), data);
        if !self.composition.add_indicator(id, kind) {
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
        if unsupported_reason(
            &kind,
            AvailabilityContext {
                basis,
                base_ticker: data.base_ticker(),
            },
        )
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
        let mut candidate_composition = self.composition.clone();
        if !candidate_composition.add_indicator_on_source(id, kind.clone(), &source) {
            return IndicatorChange::Unchanged;
        }
        let active_indicators = candidate_composition.selected_indicators();
        if candidate_instances
            .validate_source_binding(&active_indicators, id, &input_id)
            .is_err()
        {
            return IndicatorChange::Unchanged;
        }
        self.instances = candidate_instances;
        self.composition = candidate_composition;
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
            changed |= self.composition.remove_indicator(removal_id);
            changed |= self.instances.remove(removal_id).is_some();
        }
        changed
    }

    fn mark_rebuild(&mut self, id: IndicatorInstanceId, reason: IndicatorRebuildReason) {
        if self.rebuild.in_flight_ids().contains(&id)
            && matches!(
                reason,
                IndicatorRebuildReason::IndicatorSettingsChanged
                    | IndicatorRebuildReason::BasisChanged
                    | IndicatorRebuildReason::IndicatorStructureChanged
                    | IndicatorRebuildReason::TradeDataModeChanged
            )
        {
            self.rebuild.cancel();
        }

        if !self.composition.selected_indicators().contains(&id) {
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

    pub(super) fn rebuild_reason(&self, id: IndicatorInstanceId) -> Option<IndicatorRebuildReason> {
        self.dirty_reasons.get(&id).copied()
    }

    fn rebuild_scope(&self, id: IndicatorInstanceId) -> IndicatorUpdateScope {
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

    pub(super) fn pending_rebuild_job(&self) -> Option<u64> {
        self.rebuild.pending_job()
    }

    pub(super) fn rebuild_in_flight_ids(&self) -> Vec<IndicatorInstanceId> {
        self.rebuild.in_flight_ids().to_vec()
    }

    pub(super) fn begin_full_rebuild(
        &mut self,
        data: &KlineData,
        fetch: &FetchCoordinator,
        trade_data_enabled: bool,
    ) -> Option<impl std::future::Future<Output = u64> + Send + 'static + use<>> {
        let active_ids = self.selected_indicators();
        if let Some(snapshot) = self.rebuild.snapshot_mut() {
            let epochs_changed = snapshot.plan.validated_epochs.iter().any(|(id, epoch)| {
                self.dirty_epochs.get(id).copied().unwrap_or_default() != *epoch
            });
            let obsolete = !snapshot.matches_calculation(data, &self.instances, &active_ids)
                || (epochs_changed
                    && !snapshot
                        .accepts_prefix(&data.committed_indicator_frames(trade_data_enabled)));
            if obsolete {
                self.rebuild.cancel();
            } else if epochs_changed {
                for (id, epoch) in &mut snapshot.plan.validated_epochs {
                    *epoch = self.dirty_epochs.get(id).copied().unwrap_or_default();
                }
            }
        }
        if self.pending_rebuild_job().is_none() {
            let plan = self.full_rebuild_plan(data, fetch)?;
            self.rebuild.prepare(RebuildSnapshot {
                plan,
                active_ids,
                frames: data.committed_indicator_frames(trade_data_enabled),
                instances: std::sync::Arc::new(self.instances.clone()),
                basis: data.base_series().map(|series| series.basis()),
                base_ticker: data.base_ticker(),
            });
        }
        self.rebuild.launch()
    }

    pub(super) fn complete_full_rebuild(
        &mut self,
        data: &mut KlineData,
        job_id: u64,
        trade_data_enabled: bool,
    ) -> bool {
        let Some(completed) = self.rebuild.poll(job_id) else {
            return false;
        };
        if !completed.snapshot.matches_calculation(
            data,
            &self.instances,
            &self.selected_indicators(),
        ) {
            return false;
        }
        let frames = data.committed_indicator_frames(trade_data_enabled);
        if !completed.snapshot.accepts_prefix(&frames) {
            return false;
        }
        if frames.len() > completed.snapshot.frames.len() {
            self.rebuild.prepare_catchup(completed, frames);
            return false;
        }
        let snapshot = completed.snapshot;
        self.accept_full_rebuild(
            data,
            completed.host,
            &snapshot.frames,
            &snapshot.plan.dirty_ids,
            &snapshot.plan.replay_ids,
            trade_data_enabled,
        );
        data.preview_indicator_open_for(&snapshot.plan.replay_ids, trade_data_enabled);
        true
    }

    fn ready_rebuild_groups(
        &self,
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> Vec<IndicatorRebuildGroup> {
        let active = self.selected_indicators();
        let dirty = self
            .dirty_reasons
            .keys()
            .copied()
            .filter(|id| active.contains(id))
            .collect::<Vec<_>>();
        rebuild_groups(data, &dirty)
            .into_iter()
            .filter_map(|group| {
                let calculation_ids = data.calculation_indicator_ids(&group);
                let waits_for_complete_inputs = group.iter().any(|id| {
                    self.rebuild_reason(*id)
                        .is_some_and(|reason| !reason.allows_partial_recalculation())
                });
                if waits_for_complete_inputs
                    && self.has_pending_inputs(&calculation_ids, data, fetch)
                {
                    return None;
                }
                let scope = group
                    .iter()
                    .map(|id| self.rebuild_scope(*id))
                    .reduce(IndicatorUpdateScope::merge)
                    .unwrap_or(IndicatorUpdateScope::Full);
                let scope = if data.has_indicator_entries(&group) {
                    scope
                } else {
                    IndicatorUpdateScope::Full
                };
                let scope = if data.can_recalculate_incrementally(&group, scope) {
                    scope
                } else {
                    IndicatorUpdateScope::Full
                };
                Some(IndicatorRebuildGroup {
                    dirty_ids: group,
                    calculation_ids,
                    scope,
                })
            })
            .collect()
    }

    fn full_rebuild_plan(
        &self,
        data: &KlineData,
        fetch: &FetchCoordinator,
    ) -> Option<IndicatorRebuildPlan> {
        let mut dirty_ids = Vec::new();
        let mut replay_ids = Vec::new();
        for group in self
            .ready_rebuild_groups(data, fetch)
            .into_iter()
            .filter(|group| group.scope.is_full())
        {
            for id in group.dirty_ids {
                if !dirty_ids.contains(&id) {
                    dirty_ids.push(id);
                }
            }
            for id in group.calculation_ids {
                if !replay_ids.contains(&id) {
                    replay_ids.push(id);
                }
            }
        }

        (!dirty_ids.is_empty()).then(|| {
            let validated_epochs = replay_ids
                .iter()
                .map(|id| (*id, self.dirty_epochs.get(id).copied().unwrap_or_default()))
                .collect();
            IndicatorRebuildPlan {
                dirty_ids,
                replay_ids,
                validated_epochs,
            }
        })
    }

    fn accept_full_rebuild(
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

        let mut rebuilt_ids = Vec::new();
        let mut incremental_frames = Vec::new();
        let mut changed = false;

        for group in self.ready_rebuild_groups(data, fetch).into_iter() {
            let IndicatorUpdateScope::From(start) = group.scope else {
                continue;
            };
            let frames = data.append_indicator_frames(&group.dirty_ids, start, trade_data_enabled);
            incremental_frames.extend(frames.iter().cloned());
            self.dirty_reasons
                .retain(|id, _| !group.dirty_ids.contains(id));
            rebuilt_ids.extend(group.dirty_ids);
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
            data::chart::kline::FetchRange::ProfileKline { .. } => false,
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

fn rebuild_groups(
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
