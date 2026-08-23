use super::KlineSeries;
use super::indicator::{
    AvailabilityContext, IndicatorAvailability, IndicatorPanelRecipe, IndicatorUnsupportedReason,
    display_name,
};
use crate::widget::chart::kline::UnavailableIndicator;
use crate::widget::chart::kline::composition::{
    BarMode, ChartComposition, DataSourceId, HistogramMode, LayerDataKind, LayerId, LayerSource,
    LayerSpec, MarkKind, PanelId, PanelPresentation, PanelRole, PanelScaleMode, PanelSpec,
};
use data::chart::Basis;
use data::chart::kline_v2::{IndicatorInstanceId, IndicatorInstanceStore, KlineIndicator};
use exchange::TickerInfo;
use rustc_hash::FxHashMap;

const COMPARISON_SOURCE_ID: DataSourceId = DataSourceId::Synthetic("Comparison");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorPanelBinding {
    AuxPanel {
        panel_id: PanelId,
    },
    PrimaryLayer {
        panel_id: PanelId,
        layer_id: LayerId,
    },
}

#[derive(Debug, Clone)]
enum DisabledIndicatorPresentation {
    AuxiliaryPanel {
        panel: PanelSpec,
        index: usize,
        splits: Vec<f32>,
    },
    PrimaryLayer {
        panel_id: PanelId,
        layer: LayerSpec,
    },
}

pub struct IndicatorLayout {
    composition: ChartComposition,
    indicator_panels: FxHashMap<IndicatorInstanceId, IndicatorPanelBinding>,
    saved_indicator_presentations: FxHashMap<IndicatorInstanceId, DisabledIndicatorPresentation>,
    comparison_layers: FxHashMap<TickerInfo, LayerId>,
}

impl IndicatorLayout {
    pub fn new(default_volume: IndicatorInstanceId) -> Self {
        let mut layout = Self {
            composition: ChartComposition::prototype_kline(),
            indicator_panels: FxHashMap::default(),
            saved_indicator_presentations: FxHashMap::default(),
            comparison_layers: FxHashMap::default(),
        };
        let _ = layout.enable_indicator(default_volume, KlineIndicator::Volume);
        layout
    }

    pub fn composition(&self) -> &ChartComposition {
        &self.composition
    }

    pub fn composition_mut(&mut self) -> &mut ChartComposition {
        &mut self.composition
    }

    pub fn replace_composition(
        &mut self,
        panels: Vec<data::chart::kline_v2::composition::PanelSpec>,
        splits: Vec<f32>,
    ) {
        self.composition = ChartComposition::from_parts(panels, splits);
        self.saved_indicator_presentations.clear();
        self.rebuild_indicator_bindings();
        self.rebuild_comparison_bindings(&[], None);
    }

    pub fn sync_comparison_sources(
        &mut self,
        base_ticker: TickerInfo,
        selected_tickers: &[TickerInfo],
    ) {
        self.rebuild_comparison_bindings(selected_tickers, Some(base_ticker));
    }

    pub fn primary_uses_multi_source(&self) -> bool {
        self.composition
            .primary_panel_id()
            .and_then(|panel_id| self.composition.panel(panel_id))
            .is_some_and(PanelSpec::uses_multi_source)
    }

    pub fn restore_primary_scale_after_comparisons(&mut self) -> bool {
        let Some(panel_id) = self.composition.primary_panel_id() else {
            return false;
        };
        let Some(panel) = self.composition.panel(panel_id) else {
            return false;
        };
        if panel.uses_multi_source() || panel.preferred_scale != PanelScaleMode::PercentFromBase {
            return false;
        }

        self.composition
            .set_panel_preferred_scale(panel_id, PanelScaleMode::Absolute)
    }

    pub fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        self.indicator_panels.keys().copied().collect()
    }

    pub fn add_indicator(
        &mut self,
        id: IndicatorInstanceId,
        indicator: KlineIndicator,
        market: exchange::adapter::MarketKind,
    ) -> bool {
        let changed = if self.indicator_panels.contains_key(&id)
            || !super::indicator::is_supported_for_market(indicator, market)
        {
            false
        } else {
            self.enable_indicator(id, indicator)
        };

        if changed {
            self.prune_stale_indicator_bindings();
        }
        changed
    }

    pub fn remove_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        let changed = self.disable_indicator(id);
        if changed {
            self.prune_stale_indicator_bindings();
        }
        changed
    }

    pub fn indicator_is_available(
        &self,
        id: IndicatorInstanceId,
        basis: Basis,
        base_ticker: TickerInfo,
        base_series: Option<&KlineSeries>,
        instances: &IndicatorInstanceStore,
    ) -> bool {
        matches!(
            self.indicator_availability(id, basis, base_ticker, base_series, instances,),
            IndicatorAvailability::Available
        )
    }

    pub fn indicator_is_supported_for_market(
        &self,
        indicator: KlineIndicator,
        base_ticker: TickerInfo,
    ) -> bool {
        super::indicator::is_supported_for_market(indicator, base_ticker.market_type())
    }

    pub fn indicator_availability(
        &self,
        id: IndicatorInstanceId,
        basis: Basis,
        base_ticker: TickerInfo,
        base_series: Option<&KlineSeries>,
        instances: &IndicatorInstanceStore,
    ) -> IndicatorAvailability {
        let Some(instance) = instances.get(id) else {
            return IndicatorAvailability::Unsupported(
                IndicatorUnsupportedReason::MissingRequiredInput,
            );
        };
        let indicator = instance.kind;
        let availability = super::indicator::availability(
            id,
            indicator,
            AvailabilityContext { basis, base_ticker },
            base_series.and_then(|series| series.indicators.as_ref()),
        );

        if !matches!(availability, IndicatorAvailability::Available) {
            return availability;
        }

        let Some(required) = instance.config.warmup_bars() else {
            return availability;
        };

        let available = base_series.map(|series| series.bars.len()).unwrap_or(0);
        let required = required as usize;
        if available < required {
            IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::InsufficientData {
                available,
                required,
            })
        } else {
            availability
        }
    }

    pub fn unavailable_indicators(
        &self,
        basis: Basis,
        base_ticker: TickerInfo,
        base_series: Option<&KlineSeries>,
        instances: &IndicatorInstanceStore,
    ) -> Vec<(IndicatorInstanceId, IndicatorUnsupportedReason)> {
        self.selected_indicators()
            .into_iter()
            .filter_map(|id| {
                let IndicatorAvailability::Unsupported(reason) =
                    self.indicator_availability(id, basis, base_ticker, base_series, instances)
                else {
                    return None;
                };

                Some((id, reason))
            })
            .collect()
    }

    /// Indicators that are active (have a panel) and require trade data, but
    /// trade data is disabled. Returned as unsupported so the chart hides them.
    pub fn trade_data_unavailable_indicators(
        &self,
        trade_data_enabled: bool,
        instances: &IndicatorInstanceStore,
    ) -> Vec<(IndicatorInstanceId, IndicatorUnsupportedReason)> {
        if trade_data_enabled {
            return Vec::new();
        }
        self.selected_indicators()
            .into_iter()
            .filter(|id| {
                instances
                    .get(*id)
                    .is_some_and(|instance| super::indicator::requires_trade_data(instance.kind))
            })
            .map(|id| (id, IndicatorUnsupportedReason::MissingRequiredInput))
            .collect()
    }

    pub fn set_panel_mark(&mut self, panel_index: usize, mark: MarkKind) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };
        self.composition
            .set_panel_layer_mark(panel_id, layer_id, mark)
    }

    pub fn primary_mark(&self) -> Option<MarkKind> {
        let panel_id = self.composition.primary_panel_id()?;
        self.composition
            .panel_effective_mark_with_runtime(panel_id, false)
    }

    pub fn set_primary_mark(&mut self, mark: MarkKind) -> bool {
        let Some(primary_panel_index) = self
            .composition
            .panels
            .iter()
            .position(|panel| matches!(panel.role, PanelRole::Primary))
        else {
            return false;
        };

        self.set_panel_mark(primary_panel_index, mark)
    }

    pub fn reset_primary_mark(&mut self) -> bool {
        let Some(panel_id) = self.composition.primary_panel_id() else {
            return false;
        };
        let Some(layer_id) = self.composition.panel(panel_id).and_then(|panel| {
            panel
                .base_layer
                .or_else(|| panel.layers.first().map(|layer| layer.id))
        }) else {
            return false;
        };

        self.composition.clear_panel_layer_mark(panel_id, layer_id)
    }

    pub fn indicator_mark(
        &self,
        id: IndicatorInstanceId,
        signed_overlay_input: bool,
    ) -> Option<MarkKind> {
        let (panel_id, layer_id) = self.indicator_layer_binding(id)?;

        self.composition
            .resolved_panel_marks_with_runtime(panel_id, signed_overlay_input)?
            .into_iter()
            .find(|(candidate_layer_id, _)| *candidate_layer_id == layer_id)
            .map(|(_, mark)| mark)
    }

    fn indicator_layer_binding(&self, id: IndicatorInstanceId) -> Option<(PanelId, LayerId)> {
        let binding = *self.indicator_panels.get(&id)?;
        Some(match binding {
            IndicatorPanelBinding::AuxPanel { panel_id } => {
                let layer_id = self.composition.panel(panel_id)?.base_layer.or_else(|| {
                    self.composition
                        .panel(panel_id)?
                        .layers
                        .first()
                        .map(|layer| layer.id)
                })?;
                (panel_id, layer_id)
            }
            IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id } => (panel_id, layer_id),
        })
    }

    pub fn set_indicator_mark(&mut self, id: IndicatorInstanceId, mark: MarkKind) -> bool {
        let Some((panel_id, layer_id)) = self.indicator_layer_binding(id) else {
            return false;
        };

        self.composition
            .set_panel_layer_mark(panel_id, layer_id, mark)
    }

    pub fn toggle_indicator_visibility(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, layer_id)) = self.indicator_layer_binding(id) else {
            return false;
        };
        let Some(visible) = self
            .composition
            .panel(panel_id)
            .and_then(|panel| panel.layers.iter().find(|layer| layer.id == layer_id))
            .map(|layer| layer.visible)
        else {
            return false;
        };

        self.composition
            .set_panel_layer_visibility(panel_id, layer_id, !visible)
    }

    pub fn reset_indicator_mark(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, layer_id)) = self.indicator_layer_binding(id) else {
            return false;
        };

        self.composition.clear_panel_layer_mark(panel_id, layer_id)
    }

    pub fn reset_indicator_presentation(&mut self, id: IndicatorInstanceId) -> bool {
        self.saved_indicator_presentations.remove(&id).is_some()
    }

    pub fn retains_panel(&self, panel_id: PanelId) -> bool {
        self.saved_indicator_presentations
            .values()
            .any(|presentation| {
                matches!(presentation, DisabledIndicatorPresentation::AuxiliaryPanel { panel, .. }
                if panel.id == panel_id)
            })
    }

    pub fn set_panel_data_kind(&mut self, panel_index: usize, data_kind: LayerDataKind) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };
        self.composition
            .set_panel_layer_data_kind(panel_id, layer_id, data_kind)
    }

    pub fn set_panel_bar_mode(&mut self, panel_index: usize, mode: BarMode) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };
        self.composition
            .set_panel_layer_bar_mode(panel_id, layer_id, mode)
    }

    pub fn set_panel_histogram_mode(&mut self, panel_index: usize, mode: HistogramMode) -> bool {
        self.set_panel_bar_mode(panel_index, BarMode::Histogram(mode))
    }

    pub fn oi_enabled(&self, instances: &IndicatorInstanceStore) -> bool {
        self.selected_indicators().into_iter().any(|id| {
            instances
                .get(id)
                .is_some_and(|instance| instance.kind == KlineIndicator::OpenInterest)
        })
    }

    pub fn prune_stale_bindings(&mut self) {
        self.prune_stale_indicator_bindings();
    }

    fn enable_indicator(&mut self, id: IndicatorInstanceId, indicator: KlineIndicator) -> bool {
        if self.indicator_panels.contains_key(&id) {
            return false;
        }

        if self.restore_saved_indicator(id) {
            return true;
        }

        self.enable_indicator_from_recipe(id, indicator)
    }

    fn enable_indicator_from_recipe(
        &mut self,
        id: IndicatorInstanceId,
        indicator: KlineIndicator,
    ) -> bool {
        if self.indicator_panels.contains_key(&id) {
            return false;
        }

        let binding = match super::indicator::panel_recipe(indicator) {
            IndicatorPanelRecipe::Plot {
                panel_title,
                layer_name,
                source,
                data_kind,
                mark,
                axis,
                value_precision,
                value_label_policy,
                preferred_scale,
            } => {
                let layer = self.composition.new_layer(
                    layer_name,
                    LayerSource::RawKline { source },
                    data_kind,
                    mark,
                    axis,
                );
                let panel_id = self.composition.add_aux_panel(
                    panel_title,
                    vec![layer],
                    Some(value_precision),
                    value_label_policy,
                    PanelPresentation::Plot,
                );
                let _ = self
                    .composition
                    .set_panel_preferred_scale(panel_id, preferred_scale);
                let _ = self.composition.set_panel_value_id(panel_id, Some(id));
                IndicatorPanelBinding::AuxPanel { panel_id }
            }
            IndicatorPanelRecipe::Table {
                panel_title,
                source,
                ..
            } => {
                let panel_id = self.composition.add_table_panel(panel_title, source);
                let _ = self.composition.set_panel_value_id(panel_id, Some(id));
                IndicatorPanelBinding::AuxPanel { panel_id }
            }
            IndicatorPanelRecipe::PrimaryOverlay {
                layer_name,
                source,
                data_kind,
                mark,
                axis,
            } => {
                let Some(primary_panel_id) = self.composition.primary_panel_id() else {
                    return false;
                };
                let layer = self.composition.new_layer(
                    layer_name,
                    LayerSource::RawIndicator {
                        source,
                        value_id: id,
                    },
                    data_kind,
                    mark,
                    axis,
                );
                let layer_id = layer.id;
                if !self.composition.add_layer_to_panel(primary_panel_id, layer) {
                    return false;
                }
                IndicatorPanelBinding::PrimaryLayer {
                    panel_id: primary_panel_id,
                    layer_id,
                }
            }
        };

        self.indicator_panels.insert(id, binding);
        true
    }

    fn disable_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(binding) = self.indicator_panels.remove(&id) else {
            return false;
        };

        let removed = match binding {
            IndicatorPanelBinding::AuxPanel { panel_id } => {
                let Some(index) = self
                    .composition
                    .panels
                    .iter()
                    .position(|panel| panel.id == panel_id)
                else {
                    self.indicator_panels.insert(id, binding);
                    return false;
                };
                let Some(panel) = self.composition.panel(panel_id).cloned() else {
                    self.indicator_panels.insert(id, binding);
                    return false;
                };
                let splits = self.composition.splits.clone();
                let removed = self.composition.remove_panel(panel_id);
                if removed {
                    self.saved_indicator_presentations.insert(
                        id,
                        DisabledIndicatorPresentation::AuxiliaryPanel {
                            panel,
                            index,
                            splits,
                        },
                    );
                }
                removed
            }
            IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id } => {
                let Some(layer) = self
                    .composition
                    .panel(panel_id)
                    .and_then(|panel| panel.layers.iter().find(|layer| layer.id == layer_id))
                    .cloned()
                else {
                    self.indicator_panels.insert(id, binding);
                    return false;
                };
                let removed = self.composition.remove_layer_from_panel(panel_id, layer_id);
                if removed {
                    self.saved_indicator_presentations.insert(
                        id,
                        DisabledIndicatorPresentation::PrimaryLayer { panel_id, layer },
                    );
                }
                removed
            }
        };

        if !removed {
            self.indicator_panels.insert(id, binding);
        }
        removed
    }

    fn restore_saved_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(saved) = self.saved_indicator_presentations.remove(&id) else {
            return false;
        };

        match saved {
            DisabledIndicatorPresentation::AuxiliaryPanel {
                panel,
                index,
                splits,
            } => {
                let panel_id = panel.id;
                let insert_at = index.min(self.composition.panels.len());
                let mut panels = self.composition.panels.clone();
                panels.insert(insert_at, panel);
                self.composition = ChartComposition::from_parts(panels, splits);
                self.indicator_panels
                    .insert(id, IndicatorPanelBinding::AuxPanel { panel_id });
                true
            }
            DisabledIndicatorPresentation::PrimaryLayer { panel_id, layer } => {
                let layer_id = layer.id;
                if self.composition.add_layer_to_panel(panel_id, layer.clone()) {
                    self.indicator_panels.insert(
                        id,
                        IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id },
                    );
                    true
                } else {
                    self.saved_indicator_presentations.insert(
                        id,
                        DisabledIndicatorPresentation::PrimaryLayer { panel_id, layer },
                    );
                    false
                }
            }
        }
    }

    fn rebuild_indicator_bindings(&mut self) {
        self.indicator_panels.clear();
        for panel in &self.composition.panels {
            if let Some(value_id) = panel.value_id {
                self.indicator_panels.insert(
                    value_id,
                    IndicatorPanelBinding::AuxPanel { panel_id: panel.id },
                );
            }
            for layer in &panel.layers {
                let Some(value_id) = layer.source.indicator_value_id() else {
                    continue;
                };
                self.indicator_panels.insert(
                    value_id,
                    IndicatorPanelBinding::PrimaryLayer {
                        panel_id: panel.id,
                        layer_id: layer.id,
                    },
                );
            }
        }
    }

    fn rebuild_comparison_bindings(
        &mut self,
        selected_tickers: &[TickerInfo],
        base_ticker: Option<TickerInfo>,
    ) {
        self.comparison_layers.clear();
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };
        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return;
        };

        let stale_layer_ids: Vec<LayerId> = primary_panel
            .layers
            .iter()
            .filter_map(|layer| {
                let is_comparison = matches!(
                    layer.source,
                    LayerSource::RawKline {
                        source: DataSourceId::Synthetic("Comparison")
                    }
                );
                if !is_comparison {
                    return None;
                }

                let is_selected_comparison = base_ticker.is_some_and(|base| {
                    selected_tickers.iter().any(|ticker| {
                        *ticker != base && ticker.ticker.symbol_and_exchange_string() == layer.name
                    })
                });
                (!is_selected_comparison).then_some(layer.id)
            })
            .collect();

        for layer_id in stale_layer_ids {
            let _ = self
                .composition
                .remove_layer_from_panel(primary_panel_id, layer_id);
        }

        let Some(base_ticker) = base_ticker else {
            return;
        };

        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return;
        };
        for layer in &primary_panel.layers {
            if !matches!(
                layer.source,
                LayerSource::RawKline {
                    source: DataSourceId::Synthetic("Comparison")
                }
            ) {
                continue;
            }
            if let Some(ticker) = selected_tickers.iter().copied().find(|ticker| {
                *ticker != base_ticker && ticker.ticker.symbol_and_exchange_string() == layer.name
            }) {
                self.comparison_layers.insert(ticker, layer.id);
            }
        }

        for &ticker in selected_tickers
            .iter()
            .filter(|ticker| **ticker != base_ticker)
        {
            if self.comparison_layers.contains_key(&ticker) {
                continue;
            }
            let name = ticker.ticker.symbol_and_exchange_string();
            if let Some(layer_id) = self.composition.add_comparison_source_to_panel(
                primary_panel_id,
                COMPARISON_SOURCE_ID,
                name,
            ) {
                self.comparison_layers.insert(ticker, layer_id);
            }
        }
    }

    fn prune_stale_indicator_bindings(&mut self) {
        self.indicator_panels.retain(|_, binding| !match *binding {
            IndicatorPanelBinding::AuxPanel { panel_id } => {
                self.composition.panel(panel_id).is_none()
            }
            IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id } => self
                .composition
                .panel(panel_id)
                .map(|panel| !panel.layers.iter().any(|layer| layer.id == layer_id))
                .unwrap_or(true),
        });
    }

    fn panel_base_layer_ids(&self, panel_index: usize) -> Option<(PanelId, LayerId)> {
        let panel = self.composition.panels.get(panel_index)?;
        let layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id))?;
        Some((panel.id, layer_id))
    }
}

impl IndicatorLayout {
    pub fn close_panel(&mut self, panel_id: PanelId) -> bool {
        let removed = self.composition.remove_panel(panel_id);
        if removed {
            self.prune_stale_bindings();
        }
        removed
    }

    pub fn unavailable_indicator_panels(
        &self,
        basis: Basis,
        base_ticker: TickerInfo,
        base_series: Option<&KlineSeries>,
        instances: &IndicatorInstanceStore,
        trade_data_enabled: bool,
    ) -> Vec<UnavailableIndicator> {
        let trade_data_disabled =
            self.trade_data_unavailable_indicators(trade_data_enabled, instances);
        self.unavailable_indicators(basis, base_ticker, base_series, instances)
            .into_iter()
            .chain(trade_data_disabled)
            .filter_map(|(id, reason)| {
                let kind = instances.get(id)?.kind;
                Some(UnavailableIndicator {
                    value_id: id,
                    message: unavailable_indicator_message(kind, reason, basis, base_ticker),
                })
            })
            .collect()
    }
}

fn unavailable_indicator_message(
    indicator: KlineIndicator,
    reason: IndicatorUnsupportedReason,
    basis: Basis,
    base_ticker: TickerInfo,
) -> String {
    let name = display_name(indicator);
    match reason {
        IndicatorUnsupportedReason::BasisNotSupported => {
            format!("{name} is not available for {basis}.")
        }
        IndicatorUnsupportedReason::SourceNotSupported => {
            format!("{name} is not available for {}.", base_ticker.exchange())
        }
        IndicatorUnsupportedReason::ResolutionNotSupported => match basis {
            Basis::Time(timeframe) => {
                format!("{name} is not available on {timeframe} timeframe.")
            }
            Basis::Tick(_) => format!("{name} is not available for tick charts."),
        },
        IndicatorUnsupportedReason::InsufficientData {
            available,
            required,
        } => {
            format!("{name} needs {required} candles; only {available} available.")
        }
        IndicatorUnsupportedReason::MissingRequiredInput => {
            format!("{name} requires directional trade-volume data.")
        }
    }
}
