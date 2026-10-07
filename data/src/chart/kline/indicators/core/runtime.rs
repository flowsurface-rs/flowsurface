use super::super::region_intersects_source_range;
use super::super::scripting::ScriptIndicatorRuntime;
use super::input::IndicatorInputSource;
use super::{
    BaseSeriesSource, IndicatorConfig, IndicatorConfigError, IndicatorId, IndicatorOutputRef,
    OutputValue,
};
use crate::chart::kline::{
    IndicatorInstance, IndicatorInstanceId, IndicatorInstanceStore, SourceBindingProblem,
};
use indicator_engine::{IndicatorFrame, SeriesInputValue};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{ControlFlow, Range};

struct IndicatorRuntimeEntry {
    id: IndicatorInstanceId,
    kind: IndicatorId,
    config: IndicatorConfig,
    source_inputs: Vec<String>,
    source_bindings: BTreeMap<String, IndicatorOutputRef>,
    base_source_selections: BTreeMap<String, BaseSeriesSource>,
    runtime: ScriptIndicatorRuntime,
    output_indices: Option<OutputIndexMap>,
}

enum OutputIndexMap {
    Prefix(usize),
    Explicit(Vec<Option<usize>>),
}

impl OutputIndexMap {
    fn output_index(&self, index: usize) -> Option<usize> {
        match self {
            Self::Prefix(prefix) => index.checked_sub(*prefix),
            Self::Explicit(indices) => indices.get(index).copied().flatten(),
        }
    }

    fn current_index(&self, index: usize) -> Option<usize> {
        match self {
            Self::Prefix(prefix) => index.checked_add(*prefix),
            Self::Explicit(indices) => indices.iter().position(|previous| *previous == Some(index)),
        }
    }
}

impl IndicatorRuntimeEntry {
    fn new(instance: &IndicatorInstance) -> Result<Option<Self>, IndicatorConfigError> {
        let Some(registration) = super::catalog().get(&instance.kind) else {
            return Ok(None);
        };
        let runtime = registration.create_runtime(instance.config.clone())?;
        Ok(Some(Self {
            id: instance.id,
            kind: instance.kind.clone(),
            config: instance.config.clone(),
            source_inputs: registration
                .definition
                .source_inputs
                .iter()
                .map(|input| input.id.clone())
                .collect(),
            source_bindings: instance.source_bindings.clone(),
            base_source_selections: instance.base_source_selections.clone(),
            runtime,
            output_indices: None,
        }))
    }

    fn output_index(&self, index: usize) -> Option<usize> {
        self.output_indices
            .as_ref()
            .map_or(Some(index), |indices| indices.output_index(index))
    }

    fn current_index(&self, index: usize) -> Option<usize> {
        self.output_indices
            .as_ref()
            .map_or(Some(index), |indices| indices.current_index(index))
    }

    fn runtime_range_for_source_range(&self, source_range: &Range<usize>) -> Option<Range<usize>> {
        match &self.output_indices {
            None => Some(source_range.clone()),
            Some(OutputIndexMap::Prefix(prefix)) => Some(
                source_range.start.saturating_sub(*prefix)
                    ..source_range.end.saturating_sub(*prefix),
            ),
            Some(OutputIndexMap::Explicit(_)) => None,
        }
    }
}

enum IndicatorGraphDiagnostic {
    AttachedHasNoNumericPlotLayer,
    InvalidSourceBinding(SourceBindingProblem),
    DependencyCycle,
}

impl std::fmt::Display for IndicatorGraphDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AttachedHasNoNumericPlotLayer => {
                formatter.write_str("attached indicator has no numeric plot layer")
            }
            Self::InvalidSourceBinding(problem) => write!(formatter, "{problem}"),
            Self::DependencyCycle => formatter.write_str("indicator dependency cycle"),
        }
    }
}

/// Owns active indicator runtimes, output stores, and the selected dependency graph.
#[derive(Default)]
pub struct IndicatorHost {
    entries: Vec<IndicatorRuntimeEntry>,
    entry_indices: FxHashMap<IndicatorInstanceId, usize>,
    execution_order: Vec<IndicatorInstanceId>,
    dependencies: FxHashMap<IndicatorInstanceId, Vec<IndicatorInstanceId>>,
    dependents: FxHashMap<IndicatorInstanceId, Vec<IndicatorInstanceId>>,
    graph_diagnostics: FxHashMap<IndicatorInstanceId, IndicatorGraphDiagnostic>,
    runtime_diagnostics: FxHashMap<IndicatorInstanceId, IndicatorConfigError>,
    blocked_nodes: FxHashSet<IndicatorInstanceId>,
}

impl IndicatorHost {
    fn record_runtime_diagnostic(
        &mut self,
        instance: &IndicatorInstance,
        error: IndicatorConfigError,
    ) {
        log::error!(
            "indicator {} (instance {:?}) runtime rebuild failure: {error}",
            instance.kind.id(),
            instance.id
        );
        self.runtime_diagnostics.insert(instance.id, error);
    }

    fn clear_runtime_diagnostic(&mut self, id: IndicatorInstanceId) {
        self.runtime_diagnostics.remove(&id);
    }

    pub fn remap_outputs(&mut self, previous_indices: &[Option<usize>]) {
        for entry in &mut self.entries {
            entry.output_indices = Some(OutputIndexMap::Explicit(
                previous_indices
                    .iter()
                    .map(|index| index.and_then(|index| entry.output_index(index)))
                    .collect(),
            ));
        }
    }

    pub fn remap_outputs_prefix(&mut self, prefix: usize) {
        if prefix == 0 {
            return;
        }

        for entry in &mut self.entries {
            let output_indices = match entry.output_indices.take() {
                None => OutputIndexMap::Prefix(prefix),
                Some(OutputIndexMap::Prefix(previous_prefix)) => {
                    OutputIndexMap::Prefix(previous_prefix.saturating_add(prefix))
                }
                Some(OutputIndexMap::Explicit(previous_indices)) => {
                    let mut indices =
                        Vec::with_capacity(prefix.saturating_add(previous_indices.len()));
                    indices.resize(prefix, None);
                    indices.extend(previous_indices);
                    OutputIndexMap::Explicit(indices)
                }
            };
            entry.output_indices = Some(output_indices);
        }
    }

    pub fn has_entries(&self, ids: &[IndicatorInstanceId]) -> bool {
        ids.iter().all(|id| self.entry_indices.contains_key(id))
    }

    pub fn can_append_from(&self, ids: &[IndicatorInstanceId], start: usize) -> bool {
        ids.iter().all(|id| {
            self.entry_indices.get(id).is_some_and(|index| {
                let entry = &self.entries[*index];
                entry.output_indices.is_none()
                    && entry
                        .runtime
                        .committed_index()
                        .map_or(start == 0, |index| index.checked_add(1) == Some(start))
            })
        })
    }

    pub fn dependent_ids(&self, roots: &[IndicatorInstanceId]) -> Vec<IndicatorInstanceId> {
        let mut affected = roots.iter().copied().collect::<FxHashSet<_>>();
        let mut pending = roots.to_vec();
        while let Some(id) = pending.pop() {
            for dependent in self.dependents.get(&id).into_iter().flatten().copied() {
                if affected.insert(dependent) {
                    pending.push(dependent);
                }
            }
        }

        let mut result = Vec::new();
        let mut returned = FxHashSet::default();
        for id in roots
            .iter()
            .copied()
            .chain(self.execution_order.iter().copied())
        {
            if affected.contains(&id) && returned.insert(id) {
                result.push(id);
            }
        }
        result
    }

    pub fn calculation_ids(&self, roots: &[IndicatorInstanceId]) -> Vec<IndicatorInstanceId> {
        let mut included = roots.iter().copied().collect::<FxHashSet<_>>();
        let mut discovered = roots.to_vec();
        let mut pending = roots.to_vec();
        while let Some(id) = pending.pop() {
            for dependency in self.dependencies.get(&id).into_iter().flatten().copied() {
                if included.insert(dependency) {
                    discovered.push(dependency);
                    pending.push(dependency);
                }
            }
        }

        let mut result = self
            .execution_order
            .iter()
            .copied()
            .filter(|id| included.contains(id))
            .collect::<Vec<_>>();
        for id in discovered {
            if !result.contains(&id) {
                result.push(id);
            }
        }
        result
    }

    pub fn merge_rebuilt(
        &mut self,
        mut rebuilt: Self,
        replaced_ids: &[IndicatorInstanceId],
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) {
        let replaced = replaced_ids.iter().copied().collect::<FxHashSet<_>>();
        self.entries.retain(|entry| !replaced.contains(&entry.id));
        self.entries.append(&mut rebuilt.entries);

        self.runtime_diagnostics
            .retain(|id, _| !replaced.contains(id));
        self.runtime_diagnostics.extend(rebuilt.runtime_diagnostics);

        self.entries.sort_by_key(|entry| {
            active
                .iter()
                .position(|id| *id == entry.id)
                .unwrap_or(usize::MAX)
        });
        self.rebuild_entry_indices();
        self.rebuild_graph(active, instances);
    }

    pub fn recalculate(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
        frames: &[IndicatorFrame],
    ) {
        let _ =
            self.recalculate_interruptible(active, instances, frames, || ControlFlow::Continue(()));
    }

    pub fn recalculate_interruptible(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
        frames: &[IndicatorFrame],
        mut control: impl FnMut() -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let mut rebuilt = Self::default();
        for instance in active.iter().filter_map(|id| instances.get(*id)) {
            control()?;
            match IndicatorRuntimeEntry::new(instance) {
                Ok(Some(entry)) => rebuilt.entries.push(entry),
                Ok(None) => {}
                Err(error) => rebuilt.record_runtime_diagnostic(instance, error),
            };
        }
        rebuilt.rebuild_entry_indices();
        rebuilt.rebuild_graph(active, instances);
        rebuilt.replay_closed_interruptible(frames, control)?;
        *self = rebuilt;
        ControlFlow::Continue(())
    }

    pub fn replay_closed_interruptible(
        &mut self,
        frames: &[IndicatorFrame],
        control: impl FnMut() -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.replay_frames(frames, None, control)
    }

    fn replay_frames(
        &mut self,
        frames: &[IndicatorFrame],
        selected: Option<&FxHashSet<IndicatorInstanceId>>,
        mut control: impl FnMut() -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for chunk in frames.chunks(64) {
            for frame in chunk {
                control()?;
                for id in self.execution_order.clone() {
                    if selected.is_none_or(|selected| selected.contains(&id)) {
                        control()?;
                        self.evaluate_entry(id, frame, false);
                    }
                }
            }
            std::thread::yield_now();
        }
        ControlFlow::Continue(())
    }

    pub fn recalculate_selected(
        &mut self,
        dirty: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
        frames: &[IndicatorFrame],
    ) {
        let affected = self.dependent_ids(dirty);
        let affected_set = affected.iter().copied().collect::<FxHashSet<_>>();
        let mut active = self.execution_order.clone();
        for id in dirty {
            if !active.contains(id) {
                active.push(*id);
            }
        }

        if active.iter().all(|id| affected_set.contains(id)) {
            self.recalculate(&active, instances, frames);
            return;
        }

        let mut replacements = FxHashMap::default();
        for id in &affected {
            let Some(instance) = instances.get(*id) else {
                replacements.insert(*id, None);
                continue;
            };
            match IndicatorRuntimeEntry::new(instance) {
                Ok(Some(entry)) => {
                    self.clear_runtime_diagnostic(*id);
                    replacements.insert(*id, Some(entry));
                }
                Ok(None) => {
                    self.clear_runtime_diagnostic(*id);
                    replacements.insert(*id, None);
                }
                Err(error) => {
                    self.record_runtime_diagnostic(instance, error);
                    replacements.insert(*id, None);
                }
            }
        }

        let entries = std::mem::take(&mut self.entries);
        self.entries = entries
            .into_iter()
            .filter_map(|entry| match replacements.remove(&entry.id) {
                Some(replacement) => replacement,
                None => Some(entry),
            })
            .collect();
        for id in affected {
            if let Some(Some(entry)) = replacements.remove(&id) {
                self.entries.push(entry);
            }
        }
        self.rebuild_entry_indices();
        self.rebuild_graph(&active, instances);
        let _ = self.replay_frames(frames, Some(&affected_set), || ControlFlow::Continue(()));
    }

    pub fn set_active(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) -> Vec<IndicatorInstanceId> {
        let changed = active
            .iter()
            .copied()
            .filter(|id| {
                instances.get(*id).is_some_and(|instance| {
                    instance.kind.is_registered()
                        && self.entry_indices.get(id).is_none_or(|index| {
                            let entry = &self.entries[*index];
                            entry.kind != instance.kind
                                || entry.config != instance.config
                                || entry.source_bindings != instance.source_bindings
                                || entry.base_source_selections != instance.base_source_selections
                        })
                })
            })
            .collect::<Vec<_>>();
        self.entries.retain(|entry| {
            active.contains(&entry.id)
                && !changed.contains(&entry.id)
                && instances
                    .get(entry.id)
                    .is_some_and(|instance| instance.kind.is_registered())
        });
        self.runtime_diagnostics.retain(|id, _| {
            active.contains(id)
                && instances
                    .get(*id)
                    .is_some_and(|instance| instance.kind.is_registered())
        });
        self.entries.sort_by_key(|entry| {
            active
                .iter()
                .position(|id| *id == entry.id)
                .unwrap_or(usize::MAX)
        });
        self.rebuild_entry_indices();
        self.rebuild_graph(active, instances);
        changed
    }

    pub fn evaluate_closed(&mut self, frame: &IndicatorFrame) {
        let order = self.execution_order.clone();
        for id in order {
            self.evaluate_entry(id, frame, false);
        }
    }

    pub fn evaluate_closed_selected(
        &mut self,
        ids: &[IndicatorInstanceId],
        frame: &IndicatorFrame,
    ) {
        let selected = ids.iter().copied().collect::<FxHashSet<_>>();
        self.evaluate_nodes(&selected, frame, false);
    }

    pub fn preview_open_selected(&mut self, ids: &[IndicatorInstanceId], frame: &IndicatorFrame) {
        let selected = ids.iter().copied().collect::<FxHashSet<_>>();
        self.evaluate_nodes(&selected, frame, true);
    }

    fn evaluate_nodes(
        &mut self,
        selected: &FxHashSet<IndicatorInstanceId>,
        frame: &IndicatorFrame,
        preview: bool,
    ) {
        let order = self.execution_order.clone();
        for id in order {
            if selected.contains(&id) {
                self.evaluate_entry(id, frame, preview);
            }
        }
    }

    fn evaluate_entry(
        &mut self,
        id: IndicatorInstanceId,
        base_frame: &IndicatorFrame,
        preview: bool,
    ) {
        if self.blocked_nodes.contains(&id) {
            return;
        }
        let Some(entry_index) = self.entry_indices.get(&id).copied() else {
            return;
        };
        let frame = self.frame_for(entry_index, base_frame);
        if let Some(entry) = self.entries.get_mut(entry_index) {
            if preview {
                entry.runtime.preview_open(&frame);
            } else {
                entry.runtime.evaluate_closed(&frame);
            }
        }
    }

    fn frame_for(&self, entry_index: usize, base_frame: &IndicatorFrame) -> IndicatorFrame {
        let Some(entry) = self.entries.get(entry_index) else {
            return base_frame.clone();
        };
        let values = entry
            .source_inputs
            .iter()
            .map(|input_id| match entry.source_bindings.get(input_id) {
                None => entry
                    .base_source_selections
                    .get(input_id)
                    .copied()
                    .map_or(SeriesInputValue::Default, SeriesInputValue::Base),
                Some(_)
                    if !entry
                        .kind
                        .definition()
                        .source_inputs
                        .iter()
                        .any(|input| input.id == *input_id && input.allow_indicator) =>
                {
                    SeriesInputValue::Value(None)
                }
                Some(reference) if reference.instance_id == entry.id => {
                    SeriesInputValue::Value(None)
                }
                Some(reference) => self
                    .entry_indices
                    .get(&reference.instance_id)
                    .and_then(|target_index| self.entries.get(*target_index))
                    .filter(|target| {
                        !self.blocked_nodes.contains(&target.id)
                            && target
                                .kind
                                .definition()
                                .sourceable_numeric_output(&reference.channel_id)
                                .is_some()
                    })
                    .and_then(|_| {
                        self.output_value_for(
                            reference.instance_id,
                            &reference.channel_id,
                            base_frame.index,
                        )
                    })
                    .map_or(SeriesInputValue::Value(None), |value| {
                        SeriesInputValue::Value(value.as_number())
                    }),
            })
            .collect::<Vec<_>>();
        let mut frame = base_frame.clone();
        frame.series_inputs = values.into();
        frame
    }

    fn rebuild_graph(
        &mut self,
        active_ids: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) {
        self.execution_order.clear();
        self.dependencies.clear();
        self.dependents.clear();
        self.graph_diagnostics.clear();
        self.blocked_nodes.clear();
        let ids = active_ids
            .iter()
            .copied()
            .filter(|id| {
                instances
                    .get(*id)
                    .is_some_and(|instance| instance.kind.is_registered())
            })
            .collect::<Vec<_>>();
        let active = ids.iter().copied().collect::<BTreeSet<_>>();
        for id in &ids {
            let Some(instance) = instances.get(*id) else {
                continue;
            };
            if !instance.source_bindings.is_empty()
                && !instance.kind.definition().has_numeric_plot_layers()
            {
                self.graph_diagnostics
                    .insert(*id, IndicatorGraphDiagnostic::AttachedHasNoNumericPlotLayer);
                self.blocked_nodes.insert(*id);
                continue;
            }
            for source_id in instance.source_bindings.keys() {
                let target = match instances.dependency_target(instance, source_id, &active) {
                    Ok(Some(target)) => target,
                    Ok(None) => continue,
                    Err(problem) => {
                        self.graph_diagnostics
                            .entry(*id)
                            .or_insert(IndicatorGraphDiagnostic::InvalidSourceBinding(problem));
                        continue;
                    }
                };
                let dependencies = self.dependencies.entry(*id).or_default();
                if !dependencies.contains(&target.id) {
                    dependencies.push(target.id);
                    self.dependents.entry(target.id).or_default().push(*id);
                }
            }
        }
        let mut indegree = ids
            .iter()
            .map(|id| (*id, self.dependencies.get(id).map_or(0, Vec::len)))
            .collect::<FxHashMap<_, _>>();
        let mut emitted = FxHashSet::default();
        while let Some(id) = ids
            .iter()
            .copied()
            .find(|id| !emitted.contains(id) && indegree.get(id).copied() == Some(0))
        {
            emitted.insert(id);
            self.execution_order.push(id);
            for dependent in self.dependents.get(&id).into_iter().flatten() {
                if let Some(degree) = indegree.get_mut(dependent) {
                    *degree = degree.saturating_sub(1);
                }
            }
        }
        for id in ids {
            if emitted.contains(&id) {
                continue;
            }
            self.blocked_nodes.insert(id);
            self.graph_diagnostics
                .entry(id)
                .or_insert(IndicatorGraphDiagnostic::DependencyCycle);
            self.execution_order.push(id);
        }
    }

    pub fn has_input_consumer(
        &self,
        source: IndicatorInputSource,
        instances: &IndicatorInstanceStore,
    ) -> bool {
        self.execution_order.iter().any(|id| {
            instances
                .get(*id)
                .is_some_and(|instance| instance.kind.definition().uses(source))
        })
    }

    pub fn output_value_for(
        &self,
        id: IndicatorInstanceId,
        channel_id: &str,
        index: usize,
    ) -> Option<OutputValue> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| {
                entry
                    .runtime
                    .output_value(channel_id, entry.output_index(index)?)
            })
    }

    pub fn output_color_for(
        &self,
        id: IndicatorInstanceId,
        channel_id: &str,
        index: usize,
    ) -> Option<indicator_engine::Color> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| {
                entry
                    .runtime
                    .output_color(channel_id, entry.output_index(index)?)
            })
    }

    pub fn marker_outputs_for(
        &self,
        id: IndicatorInstanceId,
        index: usize,
    ) -> Vec<indicator_engine::MarkerOutput> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| Some(entry.runtime.marker_outputs_at(entry.output_index(index)?)))
            .unwrap_or_default()
    }

    pub fn segment_outputs_for(
        &self,
        id: IndicatorInstanceId,
        source_range: Range<usize>,
        mut intersects_viewport: impl FnMut(usize, usize) -> bool,
    ) -> Vec<indicator_engine::SegmentOutput> {
        if source_range.is_empty() {
            return Vec::new();
        }
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .map(|entry| {
                let inverse = match &entry.output_indices {
                    Some(OutputIndexMap::Explicit(indices)) => Some(
                        indices
                            .iter()
                            .enumerate()
                            .filter_map(|(current, previous)| {
                                previous.map(|previous| (previous, current))
                            })
                            .collect::<FxHashMap<_, _>>(),
                    ),
                    _ => None,
                };
                let current_index = |index| match &inverse {
                    Some(indices) => indices.get(&index).copied(),
                    None => entry.current_index(index),
                };
                // shortcut: scan borrowed segments; add an interval index if selection becomes costly.
                entry
                    .runtime
                    .segment_outputs()
                    .filter_map(|segment| {
                        let start = current_index(segment.start_index)?;
                        let end = current_index(segment.end_index)?;
                        if start.min(end) >= source_range.end
                            || start.max(end) < source_range.start
                            || !intersects_viewport(start, end)
                        {
                            return None;
                        }
                        let mut segment = segment.clone();
                        segment.start_index = start;
                        segment.end_index = end;
                        Some(segment)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn level_outputs_for(
        &self,
        id: IndicatorInstanceId,
    ) -> Vec<indicator_engine::HorizontalLevelOutput> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .map(|entry| entry.runtime.level_outputs())
            .unwrap_or_default()
    }

    pub fn ohlc_output_for(
        &self,
        id: IndicatorInstanceId,
        channel_id: &str,
        index: usize,
    ) -> Option<indicator_engine::OhlcOutput> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| {
                entry
                    .runtime
                    .ohlc_output(channel_id, entry.output_index(index)?)
            })
    }

    pub fn range_output_for(
        &self,
        id: IndicatorInstanceId,
        channel_id: &str,
        index: usize,
    ) -> Option<indicator_engine::RangeOutput> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| {
                entry
                    .runtime
                    .range_output(channel_id, entry.output_index(index)?)
            })
    }

    pub fn region_outputs_for_range(
        &self,
        id: IndicatorInstanceId,
        source_range: Option<Range<usize>>,
    ) -> Vec<indicator_engine::RegionOutput> {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .map(|entry| {
                let runtime_range = source_range
                    .as_ref()
                    .and_then(|range| entry.runtime_range_for_source_range(range));
                entry
                    .runtime
                    .region_outputs_in_range(runtime_range)
                    .into_iter()
                    .filter_map(|mut region| {
                        region.left_index = entry.current_index(region.left_index)?;
                        if !region.extend_right {
                            region.right_index = entry.current_index(region.right_index)?;
                        } else if let Some(right_index) = entry.current_index(region.right_index) {
                            region.right_index = right_index;
                        }
                        if let Some(source_range) = &source_range {
                            region_intersects_source_range(&region, source_range).then_some(region)
                        } else {
                            Some(region)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn diagnostic_for(&self, id: IndicatorInstanceId) -> Option<String> {
        if let Some(diagnostic) = self.graph_diagnostics.get(&id) {
            return Some(diagnostic.to_string());
        }
        if let Some(diagnostic) = self.runtime_diagnostics.get(&id) {
            return Some(format!("rebuild: {diagnostic}"));
        }

        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| entry.runtime.diagnostic().map(str::to_owned))
    }

    fn rebuild_entry_indices(&mut self) {
        self.entry_indices.clear();
        self.entry_indices.extend(
            self.entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id, index)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_index_remapping_supports_region_endpoints() {
        let explicit = OutputIndexMap::Explicit(vec![Some(10), None, Some(12)]);
        assert_eq!(explicit.current_index(10), Some(0));
        assert_eq!(explicit.current_index(12), Some(2));
        assert_eq!(explicit.current_index(11), None);

        let prefixed = OutputIndexMap::Prefix(3);
        assert_eq!(prefixed.current_index(7), Some(10));
    }

    #[test]
    fn region_culling_keeps_intersecting_finite_and_right_extending_zones() {
        let finite = |left, right| indicator_engine::RegionOutput {
            id: "zones".to_owned(),
            key: left as i64,
            left_index: left,
            right_index: right,
            upper: 10.0,
            lower: 5.0,
            border_color: None,
            fill_color: None,
            border_color_slot: None,
            fill_color_slot: None,
            extend_right: false,
            width: 1.0,
        };
        let extending = |left| indicator_engine::RegionOutput {
            extend_right: true,
            ..finite(left, left)
        };
        let visible = 5..10;

        assert!(!region_intersects_source_range(&finite(0, 4), &visible));
        assert!(region_intersects_source_range(&finite(0, 5), &visible));
        assert!(region_intersects_source_range(&finite(8, 12), &visible));
        assert!(!region_intersects_source_range(&finite(10, 12), &visible));
        assert!(region_intersects_source_range(&extending(0), &visible));
        assert!(region_intersects_source_range(&extending(9), &visible));
        assert!(!region_intersects_source_range(&extending(10), &visible));
    }

    #[test]
    fn interrupted_replay_preserves_the_previous_host() {
        let id = IndicatorInstanceId::default_for(&IndicatorId::parse("core.volume").unwrap());
        let mut host = IndicatorHost {
            execution_order: vec![id],
            ..IndicatorHost::default()
        };
        let frames = (0..130)
            .map(|index| IndicatorFrame {
                index,
                time: index as i64,
                basis: indicator_engine::TimeOrTick::Tick,
                instrument: indicator_engine::InstrumentContext::default(),
                chart: indicator_engine::ChartFrame {
                    open: Some(1.0),
                    high: Some(1.0),
                    low: Some(1.0),
                    close: Some(1.0),
                    volume: Some(1.0),
                    directional_volume_available: false,
                    buy_qty: 0.0,
                    sell_qty: 0.0,
                    trade: None,
                },
                open_interest: None,
                series_inputs: std::sync::Arc::from([]),
            })
            .collect::<Vec<_>>();
        let mut checks = 0;
        assert!(
            host.replay_closed_interruptible(&frames, || {
                checks += 1;
                if checks == 5 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .is_break()
        );
        assert_eq!(checks, 5);
        assert!(
            host.recalculate_interruptible(
                &[],
                &IndicatorInstanceStore::default(),
                &frames,
                || ControlFlow::Break(())
            )
            .is_break()
        );
        assert_eq!(host.execution_order, vec![id]);
        assert!(
            host.recalculate_interruptible(
                &[],
                &IndicatorInstanceStore::default(),
                &frames,
                || ControlFlow::Continue(())
            )
            .is_continue()
        );
        assert!(host.execution_order.is_empty());
    }
}
