use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::super::{IndicatorInstanceId, IndicatorInstanceStore};
use super::IndicatorRuntimeSnapshot;
use super::IndicatorRuntimeState;
use super::input::IndicatorInput;
use super::{AvailabilityContext, IndicatorAvailability, IndicatorUnsupportedReason};
use exchange::Kline;
use exchange::UnixMs;
use rustc_hash::FxHashMap;

pub trait IndicatorDefinition {
    type Config: Copy + Default;

    fn warmup_bars(config: Self::Config) -> Option<u64>;

    fn unsupported_reason(_context: AvailabilityContext) -> Option<IndicatorUnsupportedReason> {
        None
    }

    fn availability(
        context: AvailabilityContext,
        snapshot: IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability;
}

pub trait IndicatorRuntime: IndicatorDefinition {
    fn clear(&mut self);

    fn rebuild_from_bars(&mut self, bars: &[Kline], config: Self::Config);

    fn on_input(&mut self, input: IndicatorInput<'_>, config: Self::Config) {
        if let IndicatorInput::Klines { bars, .. } = input {
            self.rebuild_from_bars(bars, config);
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        config: Self::Config,
        scope: IndicatorUpdateScope,
    );

    fn snapshot(&self) -> IndicatorRuntimeSnapshot {
        IndicatorRuntimeSnapshot::default()
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData>;
}

#[derive(Debug, Clone, Default)]
struct IndicatorDataCache {
    values: Vec<Option<IndicatorData>>,
    bar_times: Vec<UnixMs>,
    first_bar_time: Option<UnixMs>,
    last_bar_time: Option<UnixMs>,
    signed_overlay_count: usize,
}

impl IndicatorDataCache {
    fn clear(&mut self) {
        self.values.clear();
        self.bar_times.clear();
        self.first_bar_time = None;
        self.last_bar_time = None;
        self.signed_overlay_count = 0;
    }

    fn is_aligned_with(&self, bars: &[Kline]) -> bool {
        self.values.len() == self.bar_times.len()
            && self.values.len() <= bars.len()
            && self.first_bar_time == bars.first().map(|bar| bar.time)
            && (self.values.len() < bars.len()
                || self.last_bar_time == bars.last().map(|bar| bar.time))
    }

    fn refresh_from(&mut self, state: &IndicatorRuntimeState, bars: &[Kline], first_index: usize) {
        let refresh_from = if self.is_aligned_with(bars) {
            let candidate = first_index.min(bars.len()).min(self.values.len());
            if self
                .bar_times
                .get(candidate)
                .zip(bars.get(candidate))
                .is_some_and(|(cached, current)| *cached != current.time)
            {
                0
            } else {
                candidate
            }
        } else {
            0
        };

        if refresh_from == 0 {
            self.values.clear();
            self.bar_times.clear();
            self.signed_overlay_count = 0;
        } else {
            for value in &self.values[refresh_from..] {
                if value.is_some_and(|data| data.signed_overlay().is_some()) {
                    self.signed_overlay_count = self.signed_overlay_count.saturating_sub(1);
                }
            }
            self.values.truncate(bars.len());
            self.bar_times.truncate(bars.len());
        }

        self.values.resize(bars.len(), None);
        for (index, bar) in bars.iter().enumerate().skip(refresh_from) {
            let data = state.data_for_bar(bar);
            if data.is_some_and(|data| data.signed_overlay().is_some()) {
                self.signed_overlay_count += 1;
            }
            self.values[index] = data;
            if let Some(cached_time) = self.bar_times.get_mut(index) {
                *cached_time = bar.time;
            } else {
                self.bar_times.push(bar.time);
            }
        }

        self.first_bar_time = bars.first().map(|bar| bar.time);
        self.last_bar_time = bars.last().map(|bar| bar.time);
    }

    fn data_at(&self, index: usize) -> Option<IndicatorData> {
        self.values.get(index).copied().flatten()
    }

    fn has_signed_overlay(&self) -> bool {
        self.signed_overlay_count > 0
    }
}

#[derive(Debug, Clone)]
struct IndicatorRuntimeEntry {
    id: IndicatorInstanceId,
    state: IndicatorRuntimeState,
    data: IndicatorDataCache,
}

impl IndicatorRuntimeEntry {
    fn new(id: IndicatorInstanceId, kind: super::KlineIndicator) -> Self {
        Self {
            id,
            state: IndicatorRuntimeState::for_indicator(kind),
            data: IndicatorDataCache::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndicatorRuntimeSet {
    entries: Vec<IndicatorRuntimeEntry>,
    entry_indices: FxHashMap<IndicatorInstanceId, usize>,
}

impl IndicatorRuntimeSet {
    pub(crate) fn set_active(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) {
        self.entries.retain(|entry| active.contains(&entry.id));
        for id in active {
            let Some(instance) = instances.get(*id) else {
                continue;
            };

            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == *id) {
                if !entry.state.matches_indicator(instance.kind) {
                    *entry = IndicatorRuntimeEntry::new(*id, instance.kind);
                }
            } else {
                self.entries
                    .push(IndicatorRuntimeEntry::new(*id, instance.kind));
            }
        }
        self.rebuild_entry_indices();
    }

    pub fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.state.clear();
            entry.data.clear();
        }
    }

    pub fn on_input(
        &mut self,
        input: IndicatorInput<'_>,
        instances: &IndicatorInstanceStore,
        chart_bars: &[Kline],
    ) {
        self.on_input_from(input, instances, chart_bars, IndicatorUpdateScope::Full);
    }

    pub fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        instances: &IndicatorInstanceStore,
        chart_bars: &[Kline],
        scope: IndicatorUpdateScope,
    ) {
        let source = input.source();
        for entry in &mut self.entries {
            let Some(instance) = instances.get(entry.id) else {
                continue;
            };
            if super::input_requirements(instance.kind).contains(&source) {
                entry.state.on_input_from(input, instance.config, scope);
                entry
                    .data
                    .refresh_from(&entry.state, chart_bars, scope.first_index());
            }
        }
    }

    pub fn snapshot_for(&self, id: IndicatorInstanceId) -> IndicatorRuntimeSnapshot {
        self.entry_indices
            .get(&id)
            .and_then(|index| self.entries.get(*index))
            .map(|entry| entry.state.snapshot())
            .unwrap_or_default()
    }

    pub fn data_for_indicator_at(
        &self,
        id: Option<IndicatorInstanceId>,
        index: usize,
    ) -> Option<IndicatorData> {
        let id = id?;
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| entry.data.data_at(index))
    }

    pub fn signed_overlay_available(&self, id: IndicatorInstanceId) -> bool {
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .is_some_and(|entry| entry.data.has_signed_overlay())
    }

    pub fn data_for_indicator(
        &self,
        id: Option<IndicatorInstanceId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let id = id?;
        self.entry_indices
            .get(&id)
            .and_then(|entry_index| self.entries.get(*entry_index))
            .and_then(|entry| entry.state.data_for_bar(bar))
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
