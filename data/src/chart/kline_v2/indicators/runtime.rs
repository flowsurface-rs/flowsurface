use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::super::{IndicatorInstanceId, IndicatorInstanceStore};
use super::IndicatorRuntimeSnapshot;
use super::IndicatorRuntimeState;
use super::input::IndicatorInput;
use super::{AvailabilityContext, IndicatorAvailability};
use exchange::Kline;

pub trait IndicatorDefinition {
    type Config: Copy + Default;

    fn warmup_bars(config: Self::Config) -> Option<u64>;

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
pub struct IndicatorRuntimeSet {
    states: Vec<(IndicatorInstanceId, IndicatorRuntimeState)>,
}

impl IndicatorRuntimeSet {
    pub(crate) fn set_active(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) {
        self.states.retain(|(id, _)| active.contains(id));
        for id in active {
            if self.states.iter().any(|(existing, _)| existing == id) {
                continue;
            }
            if let Some(instance) = instances.get(*id) {
                self.states
                    .push((*id, IndicatorRuntimeState::for_indicator(instance.kind)));
            }
        }
    }

    #[cfg(test)]
    fn is_active(&self, id: IndicatorInstanceId) -> bool {
        self.states.iter().any(|(candidate, _)| *candidate == id)
    }

    pub fn clear(&mut self) {
        for (_, state) in &mut self.states {
            state.clear();
        }
    }

    pub fn on_input(&mut self, input: IndicatorInput<'_>, instances: &IndicatorInstanceStore) {
        self.on_input_from(input, instances, IndicatorUpdateScope::Full);
    }

    pub fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        instances: &IndicatorInstanceStore,
        scope: IndicatorUpdateScope,
    ) {
        let source = input.source();
        for (id, state) in &mut self.states {
            let Some(instance) = instances.get(*id) else {
                continue;
            };
            if super::input_requirements(instance.kind).contains(&source) {
                state.on_input_from(input, instance.config, scope);
            }
        }
    }

    pub fn snapshot_for(&self, id: IndicatorInstanceId) -> IndicatorRuntimeSnapshot {
        self.states
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, state)| state.snapshot())
            .unwrap_or_default()
    }

    pub fn data_for_indicator(
        &self,
        id: Option<IndicatorInstanceId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let id = id?;
        self.states
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .and_then(|(_, state)| state.data_for_bar(bar))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::kline_v2::KlineIndicator;

    #[test]
    fn retains_states_only_for_active_indicators() {
        let mut runtime = IndicatorRuntimeSet::default();

        let mut instances = IndicatorInstanceStore::default();
        let volume = instances.create(KlineIndicator::Volume);
        let atr = instances.create(KlineIndicator::Atr);

        assert!(!runtime.is_active(volume));
        runtime.set_active(&[volume], &instances);
        assert!(runtime.is_active(volume));
        assert!(!runtime.is_active(atr));

        runtime.set_active(&[atr], &instances);
        assert!(!runtime.is_active(volume));
        assert!(runtime.is_active(atr));
    }
}
