use super::super::IndicatorUpdateScope;
use super::super::engine::IndicatorConfigStore;
use super::super::values::IndicatorData;
use super::IndicatorRuntimeSnapshot;
use super::input::IndicatorInput;
use super::{AvailabilityContext, IndicatorAvailability};
use super::{IndicatorRuntimeState, KlineIndicator};
use enum_map::EnumMap;
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
        _scope: IndicatorUpdateScope,
    ) {
        self.on_input(input, config);
    }

    fn snapshot(&self) -> IndicatorRuntimeSnapshot {
        IndicatorRuntimeSnapshot::default()
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData>;
}

#[derive(Debug, Clone)]
pub struct IndicatorRuntimeSet {
    states: EnumMap<KlineIndicator, Option<IndicatorRuntimeState>>,
}

impl Default for IndicatorRuntimeSet {
    fn default() -> Self {
        let mut states = EnumMap::default();
        for indicator in super::all_indicators() {
            states[*indicator] = Some(IndicatorRuntimeState::for_indicator(*indicator));
        }
        Self { states }
    }
}

impl IndicatorRuntimeSet {
    pub fn clear(&mut self) {
        for state in self.states.values_mut().flatten() {
            state.clear();
        }
    }

    pub fn on_input(&mut self, input: IndicatorInput<'_>, configs: &IndicatorConfigStore) {
        self.on_input_from(input, configs, IndicatorUpdateScope::Full);
    }

    pub fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        configs: &IndicatorConfigStore,
        scope: IndicatorUpdateScope,
    ) {
        let source = input.source();
        for indicator in super::all_indicators() {
            if !super::input_requirements(*indicator).contains(&source) {
                continue;
            }

            if let Some(state) = self.states[*indicator].as_mut() {
                state.on_input_from(input, configs, scope);
            }
        }
    }

    pub fn snapshot_for(&self, indicator: KlineIndicator) -> IndicatorRuntimeSnapshot {
        self.states[indicator]
            .as_ref()
            .map(IndicatorRuntimeState::snapshot)
            .unwrap_or_default()
    }

    pub fn data_for_indicator(
        &self,
        indicator: Option<KlineIndicator>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let indicator = indicator.unwrap_or(KlineIndicator::Volume);
        self.states[indicator]
            .as_ref()
            .and_then(|state| state.data_for_bar(bar))
    }
}
