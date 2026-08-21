use super::IndicatorUpdateScope;
use super::indicators::IndicatorInput;
use super::series::KlineSeries;
use crate::chart::Basis;

use super::IndicatorColorSlot;
pub use super::indicators::{IndicatorConfig, IndicatorConfigStore};
use super::{IndicatorInstanceId, IndicatorInstanceStore, KlineIndicator};
use crate::config::theme::ThemeColor;

#[derive(Debug, Clone, Default)]
pub struct IndicatorEngine {
    instances: IndicatorInstanceStore,
}

impl IndicatorEngine {
    pub fn instances(&self) -> &IndicatorInstanceStore {
        &self.instances
    }

    pub fn create_instance(&mut self, kind: KlineIndicator) -> IndicatorInstanceId {
        self.instances.create(kind)
    }

    pub fn remove_instance(&mut self, id: IndicatorInstanceId) -> bool {
        self.instances.remove(id).is_some()
    }

    pub fn update_color(
        &mut self,
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        color: ThemeColor,
    ) -> bool {
        self.instances.update_color(id, slot, color)
    }

    pub fn reset_colors(&mut self, id: IndicatorInstanceId) -> bool {
        self.instances.reset_colors(id)
    }

    pub fn update_config(&mut self, id: IndicatorInstanceId, config: IndicatorConfig) -> bool {
        self.instances.update_config(id, config)
    }

    pub fn reset_config(&mut self, id: IndicatorInstanceId) -> bool {
        self.instances.reset_config(id)
    }

    pub fn replace_instances(&mut self, instances: IndicatorInstanceStore) -> bool {
        if self.instances == instances {
            return false;
        }
        self.instances = instances;
        true
    }

    pub fn normalize_configs_for_basis(&mut self, basis: Basis) -> bool {
        self.replace_instances(self.instances.normalized_for_basis(basis))
    }

    pub fn update_config_for_basis(
        &mut self,
        id: IndicatorInstanceId,
        config: IndicatorConfig,
        basis: Basis,
    ) -> bool {
        self.update_config(id, config.normalized_for_basis(basis))
    }

    pub fn history_requirement(
        &self,
        active: &[IndicatorInstanceId],
        basis: Basis,
        reference_time: exchange::UnixMs,
    ) -> super::indicators::IndicatorHistoryRequirement {
        active.iter().fold(Default::default(), |requirement, id| {
            self.instances.get(*id).map_or(requirement, |instance| {
                requirement.merge(instance.config.history_requirement(basis, reference_time))
            })
        })
    }

    pub fn configure_series(
        &self,
        series: &mut KlineSeries,
        basis: Basis,
        active: &[IndicatorInstanceId],
    ) {
        series.set_active_indicators(active, &self.instances);
        self.refresh_series(series, basis);
    }

    pub fn refresh_series(&self, series: &mut KlineSeries, basis: Basis) {
        series.refresh_indicator_inputs(&self.instances);
        let trade_bars = series.normalized_trade_bars(basis);
        series.insert_indicator_input(
            IndicatorInput::Trades { bars: &trade_bars },
            &self.instances,
        );
    }

    pub fn update_series_after_trades(
        &self,
        series: &mut KlineSeries,
        basis: Basis,
        scope: IndicatorUpdateScope,
    ) {
        if matches!(basis, Basis::Tick(_)) {
            series.refresh_indicator_inputs_from(&self.instances, scope);
        }

        let trade_bars = series.normalized_trade_bars(basis);
        series.insert_indicator_input_from(
            IndicatorInput::Trades { bars: &trade_bars },
            &self.instances,
            scope,
        );
    }

    pub fn update_series_after_klines(
        &self,
        series: &mut KlineSeries,
        scope: IndicatorUpdateScope,
    ) {
        series.refresh_indicator_inputs_from(&self.instances, scope);
    }
}
