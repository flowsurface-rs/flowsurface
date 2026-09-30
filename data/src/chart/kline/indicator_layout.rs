use super::composition::{
    BarMode, ChartComposition, DataSourceId, HistogramMode, LayerDataKind, LayerId, LayerSource,
    MarkKind, PanelId, PanelPresentation, PanelRole, PanelScaleMode, PanelSpec,
};
use super::indicators::{
    AvailabilityContext, IndicatorAttachmentMode, IndicatorAvailability, IndicatorInputCoverage,
    IndicatorInputCoverageState, IndicatorInputSource, IndicatorPanelRecipe,
    IndicatorUnsupportedReason, TableColumnValue,
};
use super::series::KlineSeries;
use crate::chart::Basis;
use crate::chart::kline::{
    IndicatorId, IndicatorInstanceId, IndicatorInstanceStore, IndicatorOutputRef,
};
use exchange::TickerInfo;
use indicator_engine::PaneTarget;
use rustc_hash::{FxHashMap, FxHashSet};

const COMPARISON_SOURCE_ID: DataSourceId = DataSourceId::Synthetic("Comparison");

#[derive(Clone)]
pub struct IndicatorLayout {
    composition: ChartComposition,
    comparison_layers: FxHashMap<TickerInfo, LayerId>,
}

impl IndicatorLayout {
    pub fn empty() -> Self {
        Self {
            composition: ChartComposition::prototype_kline(),
            comparison_layers: FxHashMap::default(),
        }
    }

    pub fn new(default_volume: IndicatorInstanceId) -> Self {
        let mut layout = Self::empty();
        let volume = IndicatorId::parse("core.volume").expect("static ID is valid");
        let _ = layout.enable_indicator(default_volume, volume);
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
        panels: Vec<crate::chart::kline::composition::PanelSpec>,
        splits: Vec<f32>,
    ) {
        self.composition = ChartComposition::from_parts(panels, splits);
        self.comparison_layers.clear();
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
        let mut indicators = Vec::new();
        for panel in &self.composition.panels {
            for layer in &panel.layers {
                if let Some(value_id) = layer
                    .source
                    .indicator_output()
                    .map(|output| output.instance_id)
                    && !indicators.contains(&value_id)
                {
                    indicators.push(value_id);
                }
            }
        }
        indicators
    }

    pub fn add_indicator(
        &mut self,
        id: IndicatorInstanceId,
        indicator: IndicatorId,
        market: exchange::adapter::MarketKind,
    ) -> bool {
        if self.selected_indicators().contains(&id) || !indicator.supports_market(market) {
            return false;
        }
        self.enable_indicator(id, indicator)
    }

    pub fn add_indicator_on_source(
        &mut self,
        id: IndicatorInstanceId,
        indicator: IndicatorId,
        market: exchange::adapter::MarketKind,
        source: &crate::chart::kline::IndicatorOutputRef,
    ) -> bool {
        if self.selected_indicators().contains(&id)
            || !indicator.supports_market(market)
            || !matches!(indicator.default_view(), IndicatorPanelRecipe::Plot { .. })
            || !indicator.definition().has_numeric_plot_layers()
        {
            return false;
        }

        let Some(panel_id) = self.indicator_output_panel_id(source) else {
            return false;
        };
        if self
            .composition
            .panel(panel_id)
            .is_none_or(|panel| panel.presentation != PanelPresentation::Plot)
        {
            return false;
        }

        let IndicatorPanelRecipe::Plot { layers, .. } = indicator.default_view().clone() else {
            return false;
        };
        match indicator.definition().attachment_mode {
            IndicatorAttachmentMode::Overlay => {
                self.add_indicator_layers_to_panel(id, layers, panel_id)
            }
            IndicatorAttachmentMode::Panel => self.enable_indicator_from_recipe(id, indicator),
        }
    }

    pub fn remove_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        self.disable_indicator(id)
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
            return IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::NotInstalled);
        };
        let indicator = &instance.kind;
        let availability = crate::chart::kline::indicators::availability(
            indicator,
            AvailabilityContext { basis, base_ticker },
        );

        if !matches!(availability, IndicatorAvailability::Available) {
            return availability;
        }

        let Some(required) = instances.warmup_bars(id) else {
            return availability;
        };

        let available = base_series.map(KlineSeries::point_count).unwrap_or(0);
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

    pub fn close_panel(&mut self, panel_id: PanelId) -> bool {
        self.composition.remove_panel(panel_id)
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
                instances.get(*id).is_some_and(|instance| {
                    instance.kind.is_registered()
                        && crate::chart::kline::indicators::requires_trade_data(&instance.kind)
                })
            })
            .map(|id| {
                (
                    id,
                    IndicatorUnsupportedReason::MissingRequiredInput {
                        source: IndicatorInputSource::Trades,
                    },
                )
            })
            .collect()
    }

    pub fn required_input_coverage_unavailable_indicators(
        &self,
        input_coverage: &IndicatorInputCoverage,
        instances: &IndicatorInstanceStore,
    ) -> Vec<(IndicatorInstanceId, IndicatorUnsupportedReason)> {
        self.selected_indicators()
            .into_iter()
            .filter_map(|id| {
                let instance = instances.get(id)?;
                if !instance.kind.is_registered() {
                    return None;
                }
                let source =
                    crate::chart::kline::indicators::declared_input_requirements(&instance.kind)
                        .into_iter()
                        .find(|requirement| {
                            requirement.is_required()
                                && matches!(
                                    input_coverage.state(requirement.source),
                                    IndicatorInputCoverageState::Missing
                                )
                        })
                        .map(|requirement| requirement.source)?;
                let reason = IndicatorUnsupportedReason::MissingRequiredInput { source };
                Some((id, reason))
            })
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
        self.composition.panel_effective_mark(panel_id)
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

    fn indicator_layer_binding(&self, id: IndicatorInstanceId) -> Option<(PanelId, LayerId)> {
        for panel in &self.composition.panels {
            if let Some(layer) = panel.layers.iter().find(|layer| {
                layer
                    .source
                    .indicator_output()
                    .is_some_and(|output| output.instance_id == id)
            }) {
                return Some((panel.id, layer.id));
            }
        }
        None
    }

    fn indicator_mark_layer_ids(
        &self,
        id: IndicatorInstanceId,
        kind: Option<&IndicatorId>,
        mark: Option<MarkKind>,
    ) -> Option<(PanelId, Vec<LayerId>)> {
        let (panel_id, _) = self.indicator_layer_binding(id)?;
        let panel = self.composition.panel(panel_id)?;
        let output_layers = panel
            .layers
            .iter()
            .filter(|layer| {
                layer
                    .source
                    .indicator_output()
                    .is_some_and(|output| output.instance_id == id)
            })
            .collect::<Vec<_>>();
        if output_layers.is_empty() {
            return None;
        }

        if kind.is_none() {
            return Some((
                panel_id,
                output_layers.iter().map(|layer| layer.id).collect(),
            ));
        }

        let source_channel = mark
            .and_then(|mark| kind.and_then(|kind| kind.mark_source_channel(mark)))
            .or_else(|| {
                kind.and_then(|kind| {
                    output_layers.iter().find_map(|layer| {
                        let channel = layer.source.indicator_output()?.channel_id.as_str();
                        kind.mark_specs()
                            .iter()
                            .any(|spec| spec.source_channel == channel)
                            .then_some(channel)
                    })
                })
            });
        let matching_layers = source_channel.and_then(|source_channel| {
            let matching = output_layers
                .iter()
                .filter(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|output| output.channel_id == source_channel)
                })
                .map(|layer| layer.id)
                .collect::<Vec<_>>();
            (!matching.is_empty()).then_some(matching)
        });
        let layer_ids = matching_layers.unwrap_or_else(|| {
            if output_layers.len() == 1 {
                output_layers.iter().map(|layer| layer.id).collect()
            } else {
                output_layers
                    .first()
                    .map(|layer| vec![layer.id])
                    .unwrap_or_default()
            }
        });
        Some((panel_id, layer_ids))
    }

    pub fn indicator_mark(
        &self,
        id: IndicatorInstanceId,
        kind: Option<&IndicatorId>,
    ) -> Option<MarkKind> {
        let (panel_id, layer_ids) = self.indicator_mark_layer_ids(id, kind, None)?;

        self.composition
            .resolved_panel_marks(panel_id)?
            .into_iter()
            .find(|(candidate_layer_id, _)| layer_ids.contains(candidate_layer_id))
            .map(|(_, mark)| mark)
    }

    pub fn indicator_mark_target_layer_ids(
        &self,
        id: IndicatorInstanceId,
        kind: &IndicatorId,
        mark: MarkKind,
    ) -> Option<(PanelId, Vec<LayerId>)> {
        self.indicator_mark_layer_ids(id, Some(kind), Some(mark))
    }

    pub fn set_indicator_mark(
        &mut self,
        id: IndicatorInstanceId,
        mark: MarkKind,
        kind: Option<&IndicatorId>,
    ) -> bool {
        let Some((panel_id, layer_ids)) = self.indicator_mark_layer_ids(id, kind, Some(mark))
        else {
            return false;
        };

        let Some(panel) = self.composition.panel_mut(panel_id) else {
            return false;
        };
        let mut changed = false;
        for layer in &mut panel.layers {
            if layer_ids.contains(&layer.id) && layer.mark_override != Some(mark) {
                layer.mark_override = Some(mark);
                changed = true;
            }
        }
        changed
    }

    pub fn reset_indicator_mark(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, layer_id)) = self.indicator_layer_binding(id) else {
            return false;
        };

        let Some(panel) = self.composition.panel_mut(panel_id) else {
            return false;
        };
        let has_output_layers = panel.layers.iter().any(|layer| {
            layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
        });
        if !has_output_layers {
            return panel
                .layers
                .iter_mut()
                .find(|layer| layer.id == layer_id)
                .map(|layer| layer.mark_override.take().is_some())
                .unwrap_or(false);
        }

        let mut changed = false;
        for layer in &mut panel.layers {
            if layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
            {
                changed |= layer.mark_override.take().is_some();
            }
        }
        changed
    }

    pub fn toggle_indicator_visibility(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, _)) = self.indicator_layer_binding(id) else {
            return false;
        };
        let Some(panel) = self.composition.panel_mut(panel_id) else {
            return false;
        };
        let visible = !panel.indicator_visible(id);
        let mut changed = false;
        for layer in &mut panel.layers {
            if layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
                && layer.visible != visible
            {
                layer.visible = visible;
                changed = true;
            }
        }
        changed
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

    fn enable_indicator(&mut self, id: IndicatorInstanceId, indicator: IndicatorId) -> bool {
        if self.selected_indicators().contains(&id) {
            return false;
        }

        self.enable_indicator_from_recipe(id, indicator)
    }

    fn enable_indicator_from_recipe(
        &mut self,
        id: IndicatorInstanceId,
        indicator: IndicatorId,
    ) -> bool {
        if self.selected_indicators().contains(&id) {
            return false;
        }

        let definition = indicator.definition();
        match definition.panel_recipe.clone() {
            IndicatorPanelRecipe::Plot {
                panel_title,
                value_precision,
                value_label_policy,
                preferred_scale,
                layers: declared_layers,
                ..
            } => {
                if declared_layers.is_empty() {
                    return false;
                }
                let pane = declared_layers
                    .first()
                    .map(|layer| layer.pane)
                    .unwrap_or(PaneTarget::Indicator);
                match pane {
                    PaneTarget::Main => {
                        let Some(panel_id) = self.composition.primary_panel_id() else {
                            return false;
                        };
                        if !self.add_indicator_layers_to_panel(id, declared_layers, panel_id) {
                            return false;
                        }
                    }
                    PaneTarget::Indicator => {
                        let layers = self.new_indicator_layers(id, declared_layers);
                        let Some(panel_id) = self.composition.add_aux_panel(
                            panel_title,
                            layers,
                            Some(value_precision),
                            value_label_policy,
                            PanelPresentation::Plot,
                        ) else {
                            return false;
                        };
                        let _ = self
                            .composition
                            .set_panel_preferred_scale(panel_id, preferred_scale);
                    }
                    PaneTarget::Named(named_pane) => {
                        let panel_id = self
                            .composition
                            .panels
                            .iter()
                            .find(|panel| {
                                panel.role == PanelRole::Auxiliary
                                    && panel.presentation == PanelPresentation::Plot
                                    && panel.named_pane == Some(named_pane)
                            })
                            .map(|panel| panel.id);
                        let panel_id = panel_id.or_else(|| {
                            let panel_id = self.composition.add_aux_panel(
                                panel_title.clone(),
                                Vec::new(),
                                Some(value_precision),
                                value_label_policy,
                                PanelPresentation::Plot,
                            )?;
                            if let Some(panel) = self.composition.panel_mut(panel_id) {
                                panel.named_pane = Some(named_pane);
                                panel.preferred_scale = preferred_scale;
                            }
                            Some(panel_id)
                        });
                        let Some(panel_id) = panel_id else {
                            return false;
                        };
                        if !self.add_indicator_layers_to_panel(id, declared_layers, panel_id) {
                            return false;
                        }
                    }
                }
            }
            IndicatorPanelRecipe::Table {
                panel_title,
                columns,
            } => {
                let Some(channel_id) = columns.first().map(|column| match &column.value {
                    TableColumnValue::Channel(channel) => channel.clone(),
                }) else {
                    return false;
                };
                let Some(panel_id) = self
                    .composition
                    .add_table_panel(panel_title, DataSourceId::Primary)
                else {
                    return false;
                };
                if let Some(panel) = self.composition.panel_mut(panel_id)
                    && let Some(layer) = panel.layers.first_mut()
                {
                    layer.source = LayerSource::IndicatorOutput {
                        source: DataSourceId::Primary,
                        output: crate::chart::kline::IndicatorOutputRef::new(id, channel_id),
                    };
                }
            }
            IndicatorPanelRecipe::PrimaryOverlay {
                layers: declared_layers,
                ..
            } => {
                let Some(primary_panel_id) = self.composition.primary_panel_id() else {
                    return false;
                };
                if declared_layers.is_empty() {
                    return false;
                }
                for spec in declared_layers.iter().cloned() {
                    let layer = self.composition.new_layer(
                        spec.layer_name,
                        LayerSource::IndicatorOutput {
                            source: DataSourceId::Primary,
                            output: crate::chart::kline::IndicatorOutputRef::new(
                                id,
                                spec.channel_id,
                            ),
                        },
                        spec.data_kind,
                        spec.mark,
                        spec.axis,
                    );
                    if !self.composition.add_layer_to_panel(primary_panel_id, layer) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn new_indicator_layers(
        &mut self,
        id: IndicatorInstanceId,
        declared_layers: Vec<crate::chart::kline::IndicatorOutputLayerSpec>,
    ) -> Vec<crate::chart::kline::composition::LayerSpec> {
        declared_layers
            .into_iter()
            .map(|spec| {
                self.composition.new_layer(
                    spec.layer_name,
                    LayerSource::IndicatorOutput {
                        source: DataSourceId::Primary,
                        output: crate::chart::kline::IndicatorOutputRef::new(id, spec.channel_id),
                    },
                    spec.data_kind,
                    spec.mark,
                    spec.axis,
                )
            })
            .collect()
    }

    fn add_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        declared_layers: Vec<crate::chart::kline::IndicatorOutputLayerSpec>,
        panel_id: PanelId,
    ) -> bool {
        self.new_indicator_layers(id, declared_layers)
            .into_iter()
            .all(|layer| self.composition.add_layer_to_panel(panel_id, layer))
    }

    pub fn indicator_panel_id(&self, id: IndicatorInstanceId) -> Option<PanelId> {
        self.composition.panels.iter().find_map(|panel| {
            panel
                .layers
                .iter()
                .any(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|output| output.instance_id == id)
                })
                .then_some(panel.id)
        })
    }

    pub fn indicator_output_panel_id(&self, output: &IndicatorOutputRef) -> Option<PanelId> {
        self.composition.panels.iter().find_map(|panel| {
            panel
                .layers
                .iter()
                .any(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|candidate| candidate == output)
                })
                .then_some(panel.id)
        })
    }

    pub fn reconcile_attached_indicator_placements(
        &mut self,
        instances: &IndicatorInstanceStore,
    ) -> bool {
        let attached_ids = instances
            .instances()
            .iter()
            .filter(|instance| !instance.source_bindings.is_empty())
            .map(|instance| instance.id)
            .collect::<Vec<_>>();
        let mut changed = false;

        for id in attached_ids {
            let Some(instance) = instances.get(id) else {
                continue;
            };
            let overlay = matches!(
                instance.kind.definition().attachment_mode,
                IndicatorAttachmentMode::Overlay
            );
            let mut source_panel = None;
            let mut valid = instance.kind.definition().has_numeric_plot_layers()
                && matches!(
                    instance.kind.default_view(),
                    IndicatorPanelRecipe::Plot { .. }
                );
            for (input_id, source) in &instance.source_bindings {
                if !instance
                    .kind
                    .definition()
                    .source_inputs
                    .iter()
                    .any(|input| input.id == *input_id && input.allow_indicator)
                    || source.instance_id == id
                    || instances
                        .get(source.instance_id)
                        .is_none_or(|source_instance| {
                            source_instance
                                .kind
                                .definition()
                                .sourceable_numeric_output(&source.channel_id)
                                .is_none()
                        })
                {
                    valid = false;
                    break;
                }

                let Some(panel_id) = self.indicator_output_panel_id(source) else {
                    valid = false;
                    break;
                };
                if self
                    .composition
                    .panel(panel_id)
                    .is_none_or(|panel| panel.presentation != PanelPresentation::Plot)
                    || (overlay && source_panel.is_some_and(|target| target != panel_id))
                {
                    valid = false;
                    break;
                }
                source_panel = Some(panel_id);
            }

            if !valid {
                continue;
            }
            let Some(target_panel) = (if overlay {
                source_panel
            } else {
                self.standalone_indicator_panel(id, &instance.kind, instances)
            }) else {
                continue;
            };
            changed |= self.ensure_indicator_layers_to_panel(id, &instance.kind, target_panel);
        }

        changed
    }

    fn standalone_indicator_panel(
        &mut self,
        id: IndicatorInstanceId,
        indicator: &IndicatorId,
        instances: &IndicatorInstanceStore,
    ) -> Option<PanelId> {
        let IndicatorPanelRecipe::Plot {
            panel_title,
            value_precision,
            value_label_policy,
            preferred_scale,
            layers,
            ..
        } = indicator.default_view().clone()
        else {
            return None;
        };
        let pane = layers.first()?.pane;

        match pane {
            PaneTarget::Main => self.composition.primary_panel_id(),
            PaneTarget::Named(named_pane) => self
                .composition
                .panels
                .iter()
                .find(|panel| {
                    panel.role == PanelRole::Auxiliary
                        && panel.presentation == PanelPresentation::Plot
                        && panel.named_pane == Some(named_pane)
                })
                .map(|panel| panel.id)
                .or_else(|| {
                    let panel_id = self.composition.add_aux_panel(
                        panel_title,
                        Vec::new(),
                        Some(value_precision),
                        value_label_policy,
                        PanelPresentation::Plot,
                    )?;
                    if let Some(panel) = self.composition.panel_mut(panel_id) {
                        panel.named_pane = Some(named_pane);
                        panel.preferred_scale = preferred_scale;
                    }
                    Some(panel_id)
                }),
            PaneTarget::Indicator => {
                let source_instances = instances
                    .get(id)
                    .into_iter()
                    .flat_map(|instance| instance.source_bindings.values())
                    .map(|source| source.instance_id)
                    .collect::<FxHashSet<_>>();
                self.composition
                    .panels
                    .iter()
                    .find(|panel| {
                        panel.role == PanelRole::Auxiliary
                            && panel.presentation == PanelPresentation::Plot
                            && panel.layers.iter().any(|layer| {
                                layer
                                    .source
                                    .indicator_output()
                                    .is_some_and(|output| output.instance_id == id)
                            })
                            && panel
                                .layers
                                .iter()
                                .filter_map(|layer| layer.source.indicator_output())
                                .all(|output| {
                                    output.instance_id == id
                                        || !source_instances.contains(&output.instance_id)
                                })
                    })
                    .map(|panel| panel.id)
                    .or_else(|| {
                        let panel_id = self.composition.add_aux_panel(
                            panel_title,
                            Vec::new(),
                            Some(value_precision),
                            value_label_policy,
                            PanelPresentation::Plot,
                        )?;
                        let _ = self
                            .composition
                            .set_panel_preferred_scale(panel_id, preferred_scale);
                        Some(panel_id)
                    })
            }
        }
    }

    fn ensure_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        indicator: &IndicatorId,
        target_panel: PanelId,
    ) -> bool {
        let IndicatorPanelRecipe::Plot { layers, .. } = indicator.default_view().clone() else {
            return false;
        };
        let mut existing_channels = self
            .composition
            .panels
            .iter()
            .flat_map(|panel| panel.layers.iter())
            .filter_map(|layer| layer.source.indicator_output())
            .filter(|output| output.instance_id == id)
            .map(|output| output.channel_id.clone())
            .collect::<FxHashSet<_>>();
        let mut changed = false;

        for spec in layers {
            if !existing_channels.insert(spec.channel_id.clone()) {
                continue;
            }
            let layer = self.composition.new_layer(
                spec.layer_name,
                LayerSource::IndicatorOutput {
                    source: DataSourceId::Primary,
                    output: crate::chart::kline::IndicatorOutputRef::new(id, spec.channel_id),
                },
                spec.data_kind,
                spec.mark,
                spec.axis,
            );
            changed |= self.composition.add_layer_to_panel(target_panel, layer);
        }

        changed | self.move_indicator_layers_to_panel(id, target_panel)
    }

    fn move_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        target_panel: PanelId,
    ) -> bool {
        let locations = self
            .composition
            .panels
            .iter()
            .flat_map(|panel| {
                panel.layers.iter().filter_map(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .filter(|output| output.instance_id == id)
                        .map(|_| (panel.id, layer.id))
                })
            })
            .collect::<Vec<_>>();
        if locations.is_empty()
            || locations
                .iter()
                .all(|(panel_id, _)| *panel_id == target_panel)
        {
            return false;
        }

        let layers = locations
            .iter()
            .filter_map(|(panel_id, layer_id)| {
                self.composition
                    .panel(*panel_id)
                    .and_then(|panel| panel.layers.iter().find(|layer| layer.id == *layer_id))
                    .cloned()
            })
            .collect::<Vec<_>>();
        for (panel_id, layer_id) in &locations {
            let _ = self
                .composition
                .remove_layer_from_panel(*panel_id, *layer_id);
        }
        for layer in layers {
            let _ = self.composition.add_layer_to_panel(target_panel, layer);
        }

        let touched_panels = locations
            .iter()
            .map(|(panel_id, _)| *panel_id)
            .collect::<FxHashSet<_>>();
        for panel_id in touched_panels {
            if self
                .composition
                .panel(panel_id)
                .is_some_and(|panel| panel.role != PanelRole::Primary && panel.layers.is_empty())
            {
                let _ = self.composition.remove_panel(panel_id);
            }
        }
        true
    }

    fn disable_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        let layer_ids = self
            .composition
            .panels
            .iter()
            .flat_map(|panel| {
                panel.layers.iter().filter_map(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .filter(|output| output.instance_id == id)
                        .map(|_| (panel.id, layer.id))
                })
            })
            .collect::<Vec<_>>();
        if layer_ids.is_empty() {
            return false;
        }
        let touched_panels = layer_ids
            .iter()
            .map(|(panel_id, _)| *panel_id)
            .collect::<FxHashSet<_>>();
        let mut removed = false;
        for (panel_id, layer_id) in layer_ids {
            removed |= self.composition.remove_layer_from_panel(panel_id, layer_id);
        }
        let empty_panels = self
            .composition
            .panels
            .iter()
            .filter(|panel| {
                touched_panels.contains(&panel.id)
                    && panel.role != PanelRole::Primary
                    && panel.layers.is_empty()
            })
            .map(|panel| panel.id)
            .collect::<Vec<_>>();
        for panel_id in empty_panels {
            removed |= self.composition.remove_panel(panel_id);
        }
        removed
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

    fn panel_base_layer_ids(&self, panel_index: usize) -> Option<(PanelId, LayerId)> {
        let panel = self.composition.panels.get(panel_index)?;
        let layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id))?;
        Some((panel.id, layer_id))
    }
}
