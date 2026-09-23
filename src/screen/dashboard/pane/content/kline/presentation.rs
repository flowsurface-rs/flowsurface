use super::layout::IndicatorState;
use crate::widget::chart::kline::{
    IndicatorPresentationContext, IndicatorPresentationStatus, ResolvedIndicatorLayer,
    ResolvedIndicatorPresentation, resolve_indicator_presentation_status,
};
use data::chart::Basis;
use data::chart::kline::composition::resolve_mark_for_data_kind;
use data::chart::kline::{
    self as persisted, IndicatorId, IndicatorInputSource, IndicatorInstanceId,
    IndicatorUnsupportedReason, KlineData,
};
use rustc_hash::FxHashSet;

pub(super) struct IndicatorPresentationResolver<'a> {
    indicators: &'a IndicatorState,
    data: &'a KlineData,
    basis: Basis,
    trade_data_enabled: bool,
}

impl<'a> IndicatorPresentationResolver<'a> {
    pub(super) fn new(
        indicators: &'a IndicatorState,
        data: &'a KlineData,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> Self {
        Self {
            indicators,
            data,
            basis,
            trade_data_enabled,
        }
    }

    pub(super) fn context(&self) -> IndicatorPresentationContext {
        let input_coverage = self.indicators.input_coverage().clone();
        let unavailable = self.unavailable_indicator_panels(&input_coverage);

        let presentations = self
            .indicators
            .selected_indicators()
            .into_iter()
            .filter_map(|value_id| {
                let instance = self.indicators.indicator(value_id)?;
                let kind = &instance.kind;
                let selected_mark = self
                    .indicators
                    .layout()
                    .indicator_mark(value_id, Some(kind));

                let panel = self.indicators.composition().panels.iter().find(|panel| {
                    panel.layers.iter().any(|layer| {
                        layer
                            .source
                            .indicator_output()
                            .is_some_and(|output| output.instance_id == value_id)
                    })
                });
                let base_layer_id = panel.and_then(|panel| {
                    panel
                        .base_layer
                        .or_else(|| panel.layers.first().map(|layer| layer.id))
                });

                let required_input = selected_mark
                    .and_then(|mark| kind.mark_spec(mark))
                    .and_then(|spec| spec.required_input);
                let fallback_mark = selected_mark.and_then(|mark| kind.fallback_mark_for(mark));
                let base_message =
                    self.data
                        .indicator_host()
                        .diagnostic_for(value_id)
                        .or_else(|| {
                            unavailable
                                .iter()
                                .find(|(entry_id, _)| *entry_id == value_id)
                                .map(|(_, message)| message.as_str())
                        });

                let (effective_mark, status) =
                    if self.indicators.hides_rebuild_output(value_id) && base_message.is_none() {
                        (selected_mark, IndicatorPresentationStatus::Loading)
                    } else {
                        resolve_indicator_presentation_status(
                            base_message,
                            &input_coverage,
                            selected_mark,
                            required_input,
                            fallback_mark,
                            |source| {
                                missing_input_message(
                                    data::chart::kline::indicators::display_name(kind),
                                    source,
                                )
                            },
                        )
                    };

                let selected_layer_ids = effective_mark
                    .and_then(|mark| {
                        self.indicators
                            .layout()
                            .indicator_mark_target_layer_ids(value_id, kind, mark)
                    })
                    .map(|(_, layer_ids)| layer_ids)
                    .unwrap_or_else(|| base_layer_id.into_iter().collect());
                let resolved_marks = panel
                    .and_then(|panel| self.indicators.composition().resolved_panel_marks(panel.id))
                    .unwrap_or_default();
                let resolved_layers = panel
                    .into_iter()
                    .flat_map(|panel| panel.layers.iter())
                    .filter_map(|layer| {
                        let base_output = layer.source.indicator_output()?.clone();
                        if base_output.instance_id != value_id {
                            return None;
                        }
                        let layer_mark = resolved_marks
                            .iter()
                            .find(|(layer_id, _)| *layer_id == layer.id)
                            .map(|(_, mark)| *mark)
                            .unwrap_or_else(|| {
                                resolve_mark_for_data_kind(layer.effective_mark(), layer.data_kind)
                            });
                        let selected_layer = selected_layer_ids.contains(&layer.id);
                        let mark = effective_mark
                            .filter(|_| selected_layer)
                            .unwrap_or(layer_mark);
                        let channel_id = if selected_layer {
                            kind.mark_source_channel(mark)
                                .unwrap_or(base_output.channel_id.as_str())
                        } else {
                            base_output.channel_id.as_str()
                        };
                        Some(ResolvedIndicatorLayer {
                            base_output: base_output.clone(),
                            presentation_output: data::chart::kline::IndicatorOutputRef::new(
                                value_id, channel_id,
                            ),
                            data_kind: layer.data_kind,
                            purpose: layer.purpose,
                            mark,
                            required_input: kind
                                .mark_spec(mark)
                                .and_then(|spec| spec.required_input),
                            visible: layer.visible,
                        })
                    })
                    .collect::<Vec<_>>();
                let active_readouts = kind
                    .readout_specs()
                    .iter()
                    .filter(|readout| effective_mark.is_none_or(|mark| readout.scope.matches(mark)))
                    .cloned()
                    .collect::<Vec<_>>();

                Some(ResolvedIndicatorPresentation {
                    value_id,
                    effective_mark,
                    status,
                    resolved_layers,
                    active_readouts,
                })
            })
            .collect();

        IndicatorPresentationContext::new(input_coverage, presentations)
    }

    pub(super) fn unavailable_reason(
        &self,
        indicator: &IndicatorId,
    ) -> Option<persisted::IndicatorUnsupportedReason> {
        if let Some(reason) = persisted::unsupported_reason(
            indicator,
            persisted::indicators::AvailabilityContext {
                basis: self.basis,
                base_ticker: self.data.base_ticker(),
            },
        ) {
            return Some(reason);
        }

        if !self.trade_data_enabled
            && self
                .indicators
                .requires_trade_data(indicator, self.basis, self.data.base_ticker())
        {
            return Some(
                persisted::IndicatorUnsupportedReason::MissingRequiredInput {
                    source: IndicatorInputSource::Trades,
                },
            );
        }

        let source = self.indicators.missing_required_input(
            indicator,
            self.basis,
            self.data.base_ticker(),
        )?;
        if matches!(source, IndicatorInputSource::OpenInterest) {
            return None;
        }
        Some(persisted::IndicatorUnsupportedReason::MissingRequiredInput { source })
    }

    fn unavailable_indicator_panels(
        &self,
        input_coverage: &data::chart::kline::IndicatorInputCoverage,
    ) -> Vec<(IndicatorInstanceId, String)> {
        let trade_data_disabled = self.indicators.layout().trade_data_unavailable_indicators(
            self.trade_data_enabled,
            self.indicators.instances(),
        );
        let required_input_coverage_unavailable = self
            .indicators
            .layout()
            .required_input_coverage_unavailable_indicators(
                input_coverage,
                self.indicators.instances(),
            );

        let mut seen = FxHashSet::default();
        self.indicators
            .layout()
            .unavailable_indicators(
                self.basis,
                self.data.base_ticker(),
                self.data.base_series(),
                self.indicators.instances(),
            )
            .into_iter()
            .chain(trade_data_disabled)
            .chain(required_input_coverage_unavailable)
            .filter_map(|(id, reason)| {
                if !seen.insert(id) {
                    return None;
                }
                let kind = self.indicators.indicator(id)?.kind.clone();
                Some((
                    id,
                    unavailable_indicator_message(
                        kind,
                        reason,
                        self.basis,
                        self.data.base_ticker(),
                    ),
                ))
            })
            .collect()
    }
}

fn unavailable_indicator_message(
    indicator: IndicatorId,
    reason: IndicatorUnsupportedReason,
    basis: Basis,
    base_ticker: exchange::TickerInfo,
) -> String {
    let name = data::chart::kline::indicators::display_name(&indicator);
    match reason {
        IndicatorUnsupportedReason::BasisNotSupported => {
            format!("{name} is not available for {basis}.")
        }
        IndicatorUnsupportedReason::IncompleteHistory => {
            "Anchored VWAP requires complete history from its anchor; tick charts currently contain live data only.".to_owned()
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
        IndicatorUnsupportedReason::MissingRequiredInput { source } => {
            missing_input_message(name, source)
        }
        IndicatorUnsupportedReason::NotInstalled => {
            format!("Indicator unavailable in this build: {}.", indicator.id())
        }
    }
}

fn missing_input_message(name: &str, source: IndicatorInputSource) -> String {
    match source {
        IndicatorInputSource::Klines => format!("{name} requires candle data."),
        IndicatorInputSource::DirectionalVolume => {
            format!("{name} requires directional volume from candles or enabled trade data.")
        }
        IndicatorInputSource::Trades => format!("{name} requires trade data."),
        IndicatorInputSource::OpenInterest => format!("{name} requires open-interest data."),
    }
}
