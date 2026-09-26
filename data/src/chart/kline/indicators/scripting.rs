use super::language::PackageOrigin;
use super::region_intersects_source_range;
use super::registry::{IndicatorConfig, IndicatorId, IndicatorRegistration};
use super::{
    AvailabilityContext, IndicatorAttachmentMode, IndicatorAuthor, IndicatorColorSpec,
    IndicatorConfigError, IndicatorDefinition, IndicatorFillGeometry, IndicatorFillSpec,
    IndicatorHistoryRequirement, IndicatorInputCoverageState, IndicatorInputRequirement,
    IndicatorInputSource, IndicatorMarkSpec, IndicatorOutputLayerSpec, IndicatorOverlayChannelSpec,
    IndicatorOverlaySource, IndicatorPanelRecipe, IndicatorReadoutScope, IndicatorReadoutSpec,
    IndicatorSegmentSpec, IndicatorSetting, IndicatorSettingChoice, IndicatorSettingControl,
    IndicatorSettingValue, IndicatorSettingsContext, IndicatorSourceInput,
    IndicatorUnsupportedReason, OutputChannelSpec, OutputValue, OutputValueType, TableColumnColor,
    TableColumnFormat, TableColumnSpec, TableColumnValue,
};
use crate::chart::Basis;
use crate::chart::kline::composition::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
    PanelValueLabelMode, PanelValueLabelPolicy, PanelValuePrecision,
};
use crate::config::theme::{ThemeColorRole, ThemeColorShade};
use exchange::UnixMs;
use indicator_engine::{
    Color, CompiledIndicator, FrameOutput, HorizontalLevelOutput,
    IndicatorConfig as EngineIndicatorConfig, IndicatorFrame, IndicatorInstance, InputValue,
    MarkerOutput, OhlcOutput, PlotDescriptor, PlotValueType, RangeOutput, RegionOutput,
    RegionUpdate, RuntimeLimits, SegmentOutput, Value, ValueType,
};
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::ops::Range;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;

struct PresentationParts {
    panel_recipe: IndicatorPanelRecipe,
    mark_specs: Vec<IndicatorMarkSpec>,
    segment_specs: Vec<IndicatorSegmentSpec>,
    overlay_channels: Vec<IndicatorOverlayChannelSpec>,
    fill_specs: Vec<IndicatorFillSpec>,
    region_style_slots: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct IndicatorOutputStore {
    series: FxHashMap<String, FxHashMap<usize, Value>>,
    series_colors: FxHashMap<String, FxHashMap<usize, Option<Color>>>,
    markers: BTreeMap<String, BTreeMap<usize, MarkerOutput>>,
    segments: BTreeMap<String, BTreeMap<usize, SegmentOutput>>,
    levels: BTreeMap<String, HorizontalLevelOutput>,
    ohlc: BTreeMap<String, BTreeMap<usize, OhlcOutput>>,
    ranges: BTreeMap<String, BTreeMap<usize, RangeOutput>>,
    regions: BTreeMap<String, BTreeMap<i64, RegionOutput>>,
    persistent_regions: PersistentRegionStore,
    preview_persistent_regions: Option<PersistentRegionStore>,
}

#[derive(Debug, Clone)]
struct PersistentRegion {
    region: RegionOutput,
    sequence: usize,
}

#[derive(Debug, Clone)]
struct PersistentRegionStore {
    values: BTreeMap<(String, i64), PersistentRegion>,
    order: BTreeMap<usize, (String, i64)>,
    next_sequence: usize,
    limit: usize,
}

impl Default for PersistentRegionStore {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            order: BTreeMap::new(),
            next_sequence: 0,
            limit: RuntimeLimits::untrusted().max_output_items,
        }
    }
}

impl PersistentRegionStore {
    fn upsert(&mut self, region: RegionOutput) {
        let key = (region.id.clone(), region.key);
        if let Some(existing) = self.values.get_mut(&key) {
            existing.region = region;
            return;
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.values
            .insert(key.clone(), PersistentRegion { region, sequence });
        self.order.insert(sequence, key);
        while self.values.len() > self.limit {
            let Some((sequence, key)) = self.order.pop_first() else {
                break;
            };
            if self
                .values
                .get(&key)
                .is_some_and(|value| value.sequence == sequence)
            {
                self.values.remove(&key);
            }
        }
    }

    fn remove(&mut self, id: &str, key: i64) {
        let key = (id.to_owned(), key);
        if let Some(region) = self.values.remove(&key) {
            self.order.remove(&region.sequence);
        }
    }

    fn apply_updates(&mut self, updates: &[RegionUpdate]) {
        for update in updates {
            match update {
                RegionUpdate::Upsert(region) => self.upsert(region.clone()),
                RegionUpdate::Remove { id, key } => self.remove(id, *key),
            }
        }
    }
}

impl IndicatorOutputStore {
    fn series_value(&self, plot: &str, index: usize) -> Option<&Value> {
        self.series.get(plot)?.get(&index)
    }

    fn series_color(&self, plot: &str, index: usize) -> Option<Color> {
        self.series_colors
            .get(plot)
            .and_then(|values| values.get(&index))
            .copied()
            .flatten()
    }

    fn marker_values_at(&self, index: usize) -> Vec<MarkerOutput> {
        self.markers
            .values()
            .filter_map(|values| values.get(&index))
            .cloned()
            .collect()
    }

    fn segment_values(&self) -> impl Iterator<Item = &SegmentOutput> {
        self.segments.values().flat_map(|values| values.values())
    }

    fn level_values(&self) -> Vec<HorizontalLevelOutput> {
        self.levels.values().cloned().collect()
    }

    fn ohlc_value(&self, plot: &str, index: usize) -> Option<OhlcOutput> {
        self.ohlc.get(plot)?.get(&index).cloned()
    }

    fn range_value(&self, plot: &str, index: usize) -> Option<RangeOutput> {
        self.ranges.get(plot)?.get(&index).cloned()
    }

    fn region_values_in_range(&self, source_range: Option<&Range<usize>>) -> Vec<RegionOutput> {
        let include = |region: &RegionOutput| {
            source_range.is_none_or(|range| region_intersects_source_range(region, range))
        };
        let mut regions = self
            .regions
            .iter()
            .flat_map(|(id, values)| values.iter().map(move |(key, region)| (id, key, region)))
            .filter(|(_, _, region)| include(region))
            .map(|(id, key, region)| ((id.clone(), *key), region.clone()))
            .collect::<BTreeMap<_, _>>();
        let persistent_regions = self
            .preview_persistent_regions
            .as_ref()
            .unwrap_or(&self.persistent_regions);
        for ((id, key), value) in &persistent_regions.values {
            if !include(&value.region) {
                continue;
            }
            regions.insert((id.clone(), *key), value.region.clone());
        }
        regions.into_values().collect()
    }

    #[cfg(test)]
    fn region_values(&self) -> Vec<RegionOutput> {
        self.region_values_in_range(None)
    }

    fn apply_output(&mut self, output: FrameOutput, preview: bool) {
        for values in self.series.values_mut() {
            values.remove(&output.index);
        }
        for values in self.series_colors.values_mut() {
            values.remove(&output.index);
        }
        for plot in output.plots.iter() {
            self.series
                .entry(plot.id.clone())
                .or_default()
                .insert(output.index, plot.value.clone());
            self.series_colors
                .entry(plot.id.clone())
                .or_default()
                .insert(output.index, plot.color);
        }
        for values in self.markers.values_mut() {
            values.remove(&output.index);
        }
        for marker in output.markers.iter() {
            self.markers
                .entry(marker.id.clone())
                .or_default()
                .insert(output.index, marker.clone());
        }
        for values in self.segments.values_mut() {
            values.remove(&output.index);
        }
        for segment in output.segments.iter() {
            self.segments
                .entry(segment.id.clone())
                .or_default()
                .insert(output.index, segment.clone());
        }
        for level in output.levels.iter() {
            self.levels.insert(level.id.clone(), level.clone());
        }
        for values in self.ohlc.values_mut() {
            values.remove(&output.index);
        }
        for ohlc in output.ohlc.iter() {
            self.ohlc
                .entry(ohlc.id.clone())
                .or_default()
                .insert(output.index, ohlc.clone());
        }
        for values in self.ranges.values_mut() {
            values.remove(&output.index);
        }
        for range in output.ranges.iter() {
            self.ranges
                .entry(range.id.clone())
                .or_default()
                .insert(output.index, range.clone());
        }
        self.regions.clear();
        for region in output.regions.iter() {
            self.regions
                .entry(region.id.clone())
                .or_default()
                .insert(region.key, region.clone());
        }
        if preview {
            let mut persistent_regions = self.persistent_regions.clone();
            persistent_regions.apply_updates(&output.region_updates);
            self.preview_persistent_regions = Some(persistent_regions);
        } else {
            self.persistent_regions
                .apply_updates(&output.region_updates);
            self.preview_persistent_regions = None;
        }
    }
}

fn validated_http_url(
    value: &str,
    indicator: impl std::fmt::Display,
    field: &str,
) -> Result<String, String> {
    let url = Url::parse(value)
        .map_err(|_| format!("{indicator}: indicator {field} must be a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!(
            "{indicator}: indicator {field} must be an absolute http(s) URL"
        ));
    }
    Ok(value.to_owned())
}

fn definition_from_compiled(compiled: &CompiledIndicator) -> Result<IndicatorDefinition, String> {
    let requirements = compiled.requirements;
    let mut inputs = Vec::new();
    if requirements.ohlc {
        inputs.push(IndicatorInputRequirement::required(
            IndicatorInputSource::Klines,
        ));
    }
    if requirements.needs_directional_volume() {
        let requirement = if compiled.descriptor.optional_trades {
            IndicatorInputRequirement::optional(IndicatorInputSource::DirectionalVolume)
        } else {
            IndicatorInputRequirement::required(IndicatorInputSource::DirectionalVolume)
        };
        inputs.push(requirement);
    }
    if requirements.trades {
        inputs.push(IndicatorInputRequirement::required(
            IndicatorInputSource::Trades,
        ));
    }
    if requirements.open_interest {
        inputs.push(IndicatorInputRequirement::required(
            IndicatorInputSource::OpenInterest,
        ));
    }
    let style_slots = descriptor_styles(compiled)?;
    let outputs = descriptor_outputs(compiled);
    let author = compiled
        .descriptor
        .author
        .as_ref()
        .map(|author| -> Result<IndicatorAuthor, String> {
            let url = validated_http_url(&author.url, &compiled.descriptor.id, "author_url")?;
            Ok(IndicatorAuthor {
                name: author.name.clone(),
                url,
            })
        })
        .transpose()?;
    let source = compiled
        .descriptor
        .source
        .as_ref()
        .map(|source| validated_http_url(source, &compiled.descriptor.id, "source"))
        .transpose()?;
    let PresentationParts {
        panel_recipe,
        mark_specs,
        segment_specs,
        overlay_channels,
        fill_specs,
        region_style_slots,
    } = descriptor_view(compiled, &style_slots)?;
    let definition = IndicatorDefinition::new(
        compiled.descriptor.title.clone(),
        inputs,
        outputs,
        panel_recipe,
    )
    .with_author(author)
    .with_description(compiled.descriptor.description.clone())
    .with_source(source)
    .with_sourceable(compiled.descriptor.sourceable)
    .with_source_inputs(
        compiled
            .descriptor
            .series_inputs
            .iter()
            .map(|input| IndicatorSourceInput {
                id: input.id.clone(),
                label: input.label.clone(),
                default_label: input
                    .default_field
                    .source_input_label()
                    .unwrap_or("Declared default")
                    .to_owned(),
                allow_indicator: input.allow_indicator,
            })
            .collect::<Vec<_>>(),
    )
    .with_attachment_mode(
        compiled
            .descriptor
            .presentation
            .attachment
            .map(IndicatorAttachmentMode::from)
            .unwrap_or(IndicatorAttachmentMode::Panel),
    )
    .with_style_slots(style_slots)
    .with_region_style_slots(region_style_slots)
    .with_display_specs(mark_specs, overlay_channels, fill_specs)
    .with_segment_specs(segment_specs);
    definition.validate().map_err(|error| {
        format!(
            "{}: invalid indicator definition: {error:?}",
            compiled.descriptor.id
        )
    })?;
    Ok(definition)
}

fn descriptor_styles(compiled: &CompiledIndicator) -> Result<Vec<IndicatorColorSpec>, String> {
    compiled
        .descriptor
        .presentation
        .styles
        .iter()
        .map(|style| {
            Ok(IndicatorColorSpec::new(
                style.id.clone(),
                style.label.clone(),
                ThemeColorRole::from_str(&style.role).map_err(|error| error.to_string())?,
                ThemeColorShade::from_str(&style.shade).map_err(|error| error.to_string())?,
            )
            .with_required_input(style.required_input.map(IndicatorInputSource::from)))
        })
        .collect::<Result<Vec<_>, String>>()
}

impl From<&PlotDescriptor> for OutputValueType {
    fn from(plot: &PlotDescriptor) -> Self {
        match plot.value_type {
            PlotValueType::Number => Self::Number,
            PlotValueType::Boolean => Self::Boolean,
            PlotValueType::Ohlc => Self::Ohlc,
            PlotValueType::Range => Self::Range,
        }
    }
}

impl From<&PlotDescriptor> for OutputChannelSpec {
    fn from(plot: &PlotDescriptor) -> Self {
        Self {
            id: plot.id.clone(),
            label: plot.label.clone(),
            value_type: plot.into(),
            precision: plot
                .output_precision
                .unwrap_or(indicator_engine::OutputPrecision::Instrument),
            line_width: plot.width,
            readout: plot.readout,
            style_slot: plot.color_slot.clone(),
            offset_bars: plot.offset_bars,
            style_by: plot.style_by.clone(),
            independent_scale: matches!(plot.scale, indicator_engine::PlotScale::Independent),
        }
    }
}

fn descriptor_outputs(compiled: &CompiledIndicator) -> Vec<OutputChannelSpec> {
    compiled
        .descriptor
        .plots
        .iter()
        .map(OutputChannelSpec::from)
        .collect()
}

fn descriptor_view(
    compiled: &CompiledIndicator,
    style_slots: &[IndicatorColorSpec],
) -> Result<PresentationParts, String> {
    let presentation = &compiled.descriptor.presentation;
    let layers = presentation
        .layers
        .iter()
        .map(|layer| {
            let plot = compiled
                .descriptor
                .plots
                .iter()
                .find(|plot| plot.id == layer.channel)
                .ok_or_else(|| {
                    format!(
                        "{}: presentation layer references unknown output '{}'",
                        compiled.descriptor.id, layer.channel
                    )
                })?;
            if matches!(plot.value_type, PlotValueType::Boolean) {
                return Err(format!(
                    "{}: boolean output '{}' cannot be rendered as a layer",
                    compiled.descriptor.id, layer.channel
                ));
            }
            let data_kind = layer.data_kind.ok_or_else(|| {
                format!(
                    "{}: presentation layer '{}' is missing its data kind",
                    compiled.descriptor.id, layer.channel
                )
            })?;
            let mark = layer.mark.ok_or_else(|| {
                format!(
                    "{}: presentation layer '{}' is missing its mark",
                    compiled.descriptor.id, layer.channel
                )
            })?;
            let axis = layer.axis.ok_or_else(|| {
                format!(
                    "{}: presentation layer '{}' is missing its axis",
                    compiled.descriptor.id, layer.channel
                )
            })?;
            Ok(IndicatorOutputLayerSpec {
                channel_id: layer.channel.clone(),
                layer_name: layer.name.clone(),
                pane: layer.pane,
                data_kind: data_kind.into(),
                mark: mark.into(),
                axis: axis.into(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut marks = presentation
        .marks
        .iter()
        .map(|mark| IndicatorMarkSpec {
            label: mark.label.clone(),
            mark: mark.kind.into(),
            source_channel: mark.source_channel.clone(),
            required_input: mark.required_input.map(IndicatorInputSource::from),
            fallback_mark: mark.fallback_mark.map(MarkKind::from),
            color_slot: mark.color_slot.clone(),
            true_color_slot: mark.true_color_slot.clone(),
            false_color_slot: mark.false_color_slot.clone(),
            highlight_by: mark.highlight_by.clone(),
            highlight_color_slot: mark.highlight_color_slot.clone(),
        })
        .collect::<Vec<_>>();
    // A plot can opt into explicit true/false style slots without declaring a
    // separate mark. Materialize a mark carrying those named bindings so the
    // boolean state is honored consistently for panel and primary-overlay
    // lines.
    for plot in &compiled.descriptor.plots {
        let (Some(true_slot), Some(false_slot)) = (
            plot.true_color_slot.as_deref(),
            plot.false_color_slot.as_deref(),
        ) else {
            continue;
        };
        if plot.style_by.is_none() || marks.iter().any(|mark| mark.source_channel == plot.id) {
            continue;
        }
        let color_slot = plot.color_slot.as_deref().unwrap_or(true_slot).to_owned();
        let mark = presentation
            .layers
            .iter()
            .find(|layer| layer.channel == plot.id)
            .and_then(|layer| layer.mark)
            .or(plot.presentation_mark)
            .unwrap_or(indicator_engine::PresentationMarkKind::Line);
        marks.push(IndicatorMarkSpec {
            label: plot.label.clone(),
            mark: mark.into(),
            source_channel: plot.id.clone(),
            required_input: None,
            fallback_mark: None,
            color_slot,
            true_color_slot: Some(true_slot.to_owned()),
            false_color_slot: Some(false_slot.to_owned()),
            highlight_by: None,
            highlight_color_slot: None,
        });
    }
    let mut segments = presentation
        .segments
        .iter()
        .map(|segment| IndicatorSegmentSpec {
            id: segment.id.clone(),
            label: segment.label.clone(),
            color_slot: segment.color_slot.clone(),
            line_width: segment.line_width,
        })
        .collect::<Vec<_>>();
    let overlays = compiled
        .descriptor
        .plots
        .iter()
        .filter(|plot| plot.overlay && plot.value_type == PlotValueType::Number)
        .filter_map(|plot| {
            let color_slot = plot.color_slot.clone()?;
            Some(IndicatorOverlayChannelSpec {
                label: plot.label.clone(),
                source: IndicatorOverlaySource::Channel(plot.id.clone()),
                line_width: plot.width,
                color_role: style_role_for_plot(style_slots, &color_slot),
                color_slot,
                style_by: plot.style_by.clone(),
            })
        })
        .collect::<Vec<_>>();
    let fills = presentation
        .fills
        .iter()
        .map(|fill| IndicatorFillSpec {
            upper: IndicatorOverlaySource::Channel(fill.upper.clone()),
            lower: IndicatorOverlaySource::Channel(fill.lower.clone()),
            upper_offset_bars: fill.upper_offset_bars,
            lower_offset_bars: fill.lower_offset_bars,
            style_by: fill.style_by.clone(),
            geometry: match fill.geometry {
                indicator_engine::PresentationGeometry::PanelBand => {
                    IndicatorFillGeometry::PanelBand
                }
                indicator_engine::PresentationGeometry::SeriesBand => {
                    IndicatorFillGeometry::SeriesBand
                }
            },
            color_slot: fill.color_slot.clone(),
            true_color_slot: fill.true_color_slot.clone(),
            false_color_slot: fill.false_color_slot.clone(),
            opacity: fill.opacity,
        })
        .collect::<Vec<_>>();
    let readouts = presentation
        .readouts
        .iter()
        .map(|readout| IndicatorReadoutSpec {
            label: readout.label.clone(),
            source: IndicatorOverlaySource::Channel(readout.channel.clone()),
            color_role: style_role_for_plot(style_slots, &readout.color_slot),
            color_slot: readout.color_slot.clone(),
            scope: readout
                .mark
                .map(|mark| IndicatorReadoutScope::Mark(mark.into()))
                .unwrap_or(IndicatorReadoutScope::Always),
        })
        .collect::<Vec<_>>();
    let has_numeric_output = compiled
        .descriptor
        .plots
        .iter()
        .any(|plot| !matches!(plot.value_type, PlotValueType::Boolean));
    let view_kind = if matches!(
        presentation.view,
        indicator_engine::PresentationViewKind::Plot
    ) && !has_numeric_output
        && presentation.columns.is_empty()
    {
        indicator_engine::PresentationViewKind::Table
    } else {
        presentation.view
    };
    let panel_title = presentation
        .panel_title
        .clone()
        .unwrap_or_else(|| compiled.descriptor.title.clone());
    let layer_name = presentation
        .layer_name
        .clone()
        .unwrap_or_else(|| compiled.descriptor.title.clone());
    let panel_recipe = match view_kind {
        indicator_engine::PresentationViewKind::Table => IndicatorPanelRecipe::Table {
            panel_title,
            columns: presentation
                .columns
                .iter()
                .map(|column| TableColumnSpec {
                    label: column.label.clone(),
                    value: TableColumnValue::Channel(column.channel.clone()),
                    format: parse_table_format(&column.format),
                    color: match &column.color {
                        indicator_engine::PresentationColumnColor::Static { slot } => {
                            TableColumnColor::Static { slot: slot.clone() }
                        }
                        indicator_engine::PresentationColumnColor::Signed {
                            positive_slot,
                            negative_slot,
                        } => TableColumnColor::Signed {
                            positive_slot: positive_slot.clone(),
                            negative_slot: negative_slot.clone(),
                        },
                    },
                })
                .collect(),
        },
        indicator_engine::PresentationViewKind::PrimaryOverlay => {
            if layers.is_empty() {
                return Err(format!(
                    "{}: presentation has no numeric output layers",
                    compiled.descriptor.id
                ));
            }
            IndicatorPanelRecipe::PrimaryOverlay { layer_name, layers }
        }
        indicator_engine::PresentationViewKind::Plot => {
            let value_precision = presentation
                .value_precision
                .map(PanelValuePrecision::from)
                .ok_or_else(|| {
                    format!(
                        "{}: numeric plot presentation requires format(precision = ...)",
                        compiled.descriptor.id
                    )
                })?;
            IndicatorPanelRecipe::Plot {
                panel_title,
                layer_name,
                value_precision,
                value_label_policy: PanelValueLabelPolicy {
                    axis_mode: presentation
                        .axis_label_mode
                        .map(PanelValueLabelMode::from)
                        .unwrap_or(PanelValueLabelMode::Compact),
                    header_mode: presentation
                        .header_label_mode
                        .map(PanelValueLabelMode::from)
                        .unwrap_or(PanelValueLabelMode::Commas),
                    max_decimals: presentation.max_decimals,
                },
                preferred_scale: presentation
                    .preferred_scale
                    .map(PanelScaleMode::from)
                    .unwrap_or(PanelScaleMode::FitVisible),
                layers,
                readouts,
            }
        }
    };
    if matches!(view_kind, indicator_engine::PresentationViewKind::Table) {
        marks.clear();
        segments.clear();
    }
    Ok(PresentationParts {
        panel_recipe,
        mark_specs: marks,
        segment_specs: segments,
        overlay_channels: overlays,
        fill_specs: fills,
        region_style_slots: presentation.region_style_slots.clone(),
    })
}

fn style_role_for_plot(style_slots: &[IndicatorColorSpec], slot: &str) -> ThemeColorRole {
    style_slots
        .iter()
        .find(|style| style.id == slot)
        .map(|style| style.default_role)
        .unwrap_or(ThemeColorRole::Neutral)
}

impl From<indicator_engine::PresentationDataKind> for LayerDataKind {
    fn from(kind: indicator_engine::PresentationDataKind) -> Self {
        match kind {
            indicator_engine::PresentationDataKind::Scalar => Self::Scalar,
            indicator_engine::PresentationDataKind::Histogram => Self::Histogram,
            indicator_engine::PresentationDataKind::Ohlc => Self::Ohlc,
            indicator_engine::PresentationDataKind::Range => Self::Range,
        }
    }
}

impl From<indicator_engine::PresentationAttachmentMode> for IndicatorAttachmentMode {
    fn from(mode: indicator_engine::PresentationAttachmentMode) -> Self {
        match mode {
            indicator_engine::PresentationAttachmentMode::Overlay => Self::Overlay,
            indicator_engine::PresentationAttachmentMode::Panel => Self::Panel,
        }
    }
}

impl From<indicator_engine::PresentationMarkKind> for MarkKind {
    fn from(mark: indicator_engine::PresentationMarkKind) -> Self {
        match mark {
            indicator_engine::PresentationMarkKind::Candle => Self::Candle,
            indicator_engine::PresentationMarkKind::RegularBar => Self::Bar(BarMode::Regular),
            indicator_engine::PresentationMarkKind::Histogram => {
                Self::Bar(BarMode::Histogram(HistogramMode::Plain))
            }
            indicator_engine::PresentationMarkKind::HistogramSignedOverlay => {
                Self::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
            }
            indicator_engine::PresentationMarkKind::Line => Self::Line,
            indicator_engine::PresentationMarkKind::Step => Self::StepLine,
            indicator_engine::PresentationMarkKind::Area => Self::Area,
            indicator_engine::PresentationMarkKind::Point => Self::Point,
            indicator_engine::PresentationMarkKind::Cross => Self::Cross,
            indicator_engine::PresentationMarkKind::Segment => Self::Segment,
            indicator_engine::PresentationMarkKind::HorizontalLevel => Self::HorizontalLevel,
        }
    }
}

impl From<indicator_engine::PresentationInputSource> for IndicatorInputSource {
    fn from(source: indicator_engine::PresentationInputSource) -> Self {
        match source {
            indicator_engine::PresentationInputSource::Klines => Self::Klines,
            indicator_engine::PresentationInputSource::DirectionalVolume => Self::DirectionalVolume,
            indicator_engine::PresentationInputSource::Trades => Self::Trades,
            indicator_engine::PresentationInputSource::OpenInterest => Self::OpenInterest,
        }
    }
}

impl From<indicator_engine::PresentationAxis> for AxisBinding {
    fn from(axis: indicator_engine::PresentationAxis) -> Self {
        match axis {
            indicator_engine::PresentationAxis::Primary => Self::Primary,
            indicator_engine::PresentationAxis::Secondary => Self::Secondary,
        }
    }
}

impl From<indicator_engine::PresentationPrecision> for PanelValuePrecision {
    fn from(precision: indicator_engine::PresentationPrecision) -> Self {
        match precision {
            indicator_engine::PresentationPrecision::BaseTickerMinTick => Self::BaseTickerMinTick,
            indicator_engine::PresentationPrecision::BaseTickerMinQty => Self::BaseTickerMinQty,
            indicator_engine::PresentationPrecision::FixedStep(step) => Self::FixedStep(step),
        }
    }
}

impl From<indicator_engine::PresentationLabelMode> for PanelValueLabelMode {
    fn from(mode: indicator_engine::PresentationLabelMode) -> Self {
        match mode {
            indicator_engine::PresentationLabelMode::Compact => Self::Compact,
            indicator_engine::PresentationLabelMode::Commas => Self::Commas,
            indicator_engine::PresentationLabelMode::Abbreviated => Self::Abbreviated,
        }
    }
}

impl From<indicator_engine::PresentationScale> for PanelScaleMode {
    fn from(scale: indicator_engine::PresentationScale) -> Self {
        match scale {
            indicator_engine::PresentationScale::Absolute => Self::Absolute,
            indicator_engine::PresentationScale::FitVisible => Self::FitVisible,
            indicator_engine::PresentationScale::FitVisibleIncludeZero => {
                Self::FitVisibleIncludeZero
            }
            indicator_engine::PresentationScale::Logarithmic => Self::Logarithmic,
            indicator_engine::PresentationScale::PercentFromBase => Self::PercentFromBase,
        }
    }
}

fn parse_table_format(value: &str) -> TableColumnFormat {
    match value.to_ascii_lowercase().as_str() {
        "integer" => TableColumnFormat::Integer,
        "percent" => TableColumnFormat::Percent,
        "boolean" => TableColumnFormat::Boolean,
        _ => TableColumnFormat::Quantity,
    }
}

fn input_value_to_json(value: &InputValue) -> serde_json::Value {
    match value {
        InputValue::Int(value) => (*value).into(),
        InputValue::Float(value) => (*value).into(),
        InputValue::Bool(value) => (*value).into(),
        InputValue::String(value) => value.clone().into(),
        InputValue::Color(value) => (*value).into(),
        InputValue::Time(value) => (*value).into(),
    }
}

fn integer_bound(value: Option<f64>, round: fn(f64) -> f64) -> Option<i64> {
    let value = value.filter(|value| value.is_finite()).map(round)?;
    Some(if value <= i64::MIN as f64 {
        i64::MIN
    } else if value >= i64::MAX as f64 {
        i64::MAX
    } else {
        value as i64
    })
}

fn setting_from_input(
    input: &indicator_engine::InputDescriptor,
    config: &serde_json::Value,
) -> IndicatorSetting {
    let configured = config
        .as_object()
        .and_then(|values| values.get(&input.id))
        .and_then(|value| input_value_from_json(&input.value_type, value))
        .and_then(|value| input.accepts(&value).then_some(value))
        .unwrap_or_else(|| input.default.clone());
    let (value, control) = match configured {
        InputValue::Int(value) => (
            IndicatorSettingValue::I64(value),
            IndicatorSettingControl::SliderI64 {
                min: integer_bound(input.min, f64::ceil),
                max: integer_bound(input.max, f64::floor),
            },
        ),
        InputValue::Float(value) => (
            IndicatorSettingValue::F64(value),
            IndicatorSettingControl::SliderF64 {
                min: input.min,
                max: input.max,
                step: input.step.unwrap_or(0.01).max(f64::EPSILON),
            },
        ),
        InputValue::Bool(value) => (
            IndicatorSettingValue::Bool(value),
            IndicatorSettingControl::Toggle,
        ),
        InputValue::String(value) => {
            if input.options.is_empty() {
                (
                    IndicatorSettingValue::Text(value),
                    IndicatorSettingControl::Text,
                )
            } else {
                let choices = input
                    .options
                    .iter()
                    .map(|option| IndicatorSettingChoice {
                        id: option.id.clone(),
                        label: option.label.clone(),
                    })
                    .collect();
                (
                    IndicatorSettingValue::Choice(value),
                    IndicatorSettingControl::Choices(choices),
                )
            }
        }
        InputValue::Color(value) => (
            IndicatorSettingValue::Color(value),
            IndicatorSettingControl::Color,
        ),
        InputValue::Time(value) => (
            IndicatorSettingValue::I64(value),
            IndicatorSettingControl::SliderI64 {
                min: integer_bound(input.min, f64::ceil),
                max: integer_bound(input.max, f64::floor),
            },
        ),
    };
    IndicatorSetting {
        id: input.id.clone(),
        label: input.label.clone(),
        tooltip: input.tooltip.clone(),
        group: input.group.clone(),
        value,
        control,
    }
}

fn setting_value_to_json(
    input: &indicator_engine::InputDescriptor,
    value: IndicatorSettingValue,
) -> Option<serde_json::Value> {
    match (input.value_type, value) {
        (indicator_engine::ValueType::Int, IndicatorSettingValue::I64(value)) => {
            Some(serde_json::Value::from(value))
        }
        (indicator_engine::ValueType::Float, IndicatorSettingValue::F64(value)) => {
            Some(serde_json::Value::from(value))
        }
        (indicator_engine::ValueType::Bool, IndicatorSettingValue::Bool(value)) => {
            Some(serde_json::Value::from(value))
        }
        (indicator_engine::ValueType::String, IndicatorSettingValue::Text(value))
        | (indicator_engine::ValueType::String, IndicatorSettingValue::Choice(value)) => {
            Some(serde_json::Value::from(value))
        }
        (indicator_engine::ValueType::Time, IndicatorSettingValue::I64(value)) => {
            Some(serde_json::Value::from(value))
        }
        (indicator_engine::ValueType::Color, IndicatorSettingValue::Color(value)) => {
            Some(serde_json::Value::from(value))
        }
        _ => None,
    }
}

/// Register a source-compiled indicator. The `.ind` source owns
/// both the executable program metadata and the host-neutral presentation
/// scene used by the chart adapter.
pub fn registration(
    compiled: CompiledIndicator,
    origin: PackageOrigin,
) -> Result<IndicatorRegistration, Vec<String>> {
    let descriptor = compiled.descriptor.clone();
    let origin_label = match origin {
        PackageOrigin::Bundled => "bundled",
        PackageOrigin::Installed => "installed",
    };
    let id = IndicatorId::parse(descriptor.id.as_str()).map_err(|error| {
        vec![format!(
            "{origin_label} indicator source has invalid ID: {error}"
        )]
    })?;
    let definition = definition_from_compiled(&compiled).map_err(|error| vec![error])?;
    let registration =
        IndicatorRegistration::from_parts(id, origin, definition, Arc::new(compiled));
    registration
        .validate_config(&registration.default_config())
        .map_err(|error| {
            vec![format!(
                "{origin_label} indicator source '{}': {error}",
                registration.id.id()
            )]
        })?;
    Ok(registration)
}

impl IndicatorRegistration {
    pub fn default_config(&self) -> IndicatorConfig {
        let values = self
            .compiled
            .descriptor
            .inputs
            .iter()
            .map(|input| (input.id.clone(), input_value_to_json(&input.default)))
            .collect();
        IndicatorConfig::from_value(serde_json::Value::Object(values))
    }

    pub fn normalize_config(&self, config: IndicatorConfig) -> IndicatorConfig {
        let mut values = config.value().as_object().cloned().unwrap_or_default();
        for input in &self.compiled.descriptor.inputs {
            values
                .entry(input.id.clone())
                .or_insert_with(|| input_value_to_json(&input.default));
        }
        IndicatorConfig::from_value(serde_json::Value::Object(values))
    }

    pub fn settings(
        &self,
        config: IndicatorConfig,
        context: IndicatorSettingsContext,
    ) -> Result<Vec<IndicatorSetting>, IndicatorConfigError> {
        let values = config.value();
        let inputs = &self.compiled.descriptor.inputs;
        Ok(inputs
            .iter()
            .filter(|input| input_is_visible(input, inputs, values, &context.input_coverage))
            .map(|input| setting_from_input(input, values))
            .collect())
    }

    pub fn with_setting(
        &self,
        config: IndicatorConfig,
        id: &str,
        value: IndicatorSettingValue,
    ) -> Option<IndicatorConfig> {
        let mut object = config.value().as_object().cloned().unwrap_or_default();
        let input = self
            .compiled
            .descriptor
            .inputs
            .iter()
            .find(|input| input.id == id)?;
        let value = setting_value_to_json(input, value)?;
        object.insert(id.to_owned(), value);
        Some(IndicatorConfig::from_value(serde_json::Value::Object(
            object,
        )))
    }

    pub fn history_requirement(
        &self,
        config: IndicatorConfig,
        basis: Basis,
        reference_time: exchange::UnixMs,
    ) -> IndicatorHistoryRequirement {
        self.history_requirement_with_warmup(config, basis, reference_time, None)
    }

    pub(crate) fn history_requirement_with_warmup(
        &self,
        config: IndicatorConfig,
        basis: Basis,
        reference_time: exchange::UnixMs,
        warmup_bars: Option<u64>,
    ) -> IndicatorHistoryRequirement {
        let Ok(next_config) = config_to_runtime(&config, &self.compiled.descriptor) else {
            return IndicatorHistoryRequirement::default();
        };
        let warmup = warmup_bars
            .unwrap_or_else(|| self.compiled.warmup_bars(&next_config).unwrap_or(0) as u64);
        let start = self
            .compiled
            .anchor_start(&next_config, reference_time.as_u64() as i64);
        let bars = match (start, basis) {
            (Some(start), Basis::Time(timeframe)) => {
                let elapsed = (reference_time.as_u64() as i64 - start).max(0) as u64;
                let dt = timeframe.to_milliseconds().max(1);
                warmup.max(elapsed.div_ceil(dt).saturating_add(1))
            }
            _ => warmup,
        };
        IndicatorHistoryRequirement::new(start.map(|start| UnixMs::new(start as u64)), bars)
    }

    pub fn warmup_bars(&self, config: &IndicatorConfig) -> Option<u64> {
        let next_config = config_to_runtime(config, &self.compiled.descriptor).ok()?;
        self.compiled
            .warmup_bars(&next_config)
            .map(|warmup| warmup as u64)
    }

    pub fn validate_config(&self, config: &IndicatorConfig) -> Result<(), IndicatorConfigError> {
        let next_config = config_to_runtime(config, &self.compiled.descriptor)?;
        self.compiled
            .validate_history_analysis(&next_config)
            .map_err(|error| IndicatorConfigError::Deserialize(error.to_string()))
    }

    pub fn unsupported_reason(
        &self,
        context: AvailabilityContext,
    ) -> Option<IndicatorUnsupportedReason> {
        let basis = match context.basis {
            Basis::Time(_) => indicator_engine::TimeOrTick::Time,
            Basis::Tick(_) => indicator_engine::TimeOrTick::Tick,
        };
        if self
            .compiled
            .descriptor
            .bases
            .as_ref()
            .is_some_and(|bases| !bases.contains(&basis))
        {
            return Some(IndicatorUnsupportedReason::BasisNotSupported);
        }
        if basis == indicator_engine::TimeOrTick::Tick && self.compiled.requires_anchored_history()
        {
            return Some(IndicatorUnsupportedReason::IncompleteHistory);
        }
        if self.compiled.requirements.open_interest {
            let Basis::Time(timeframe) = context.basis else {
                return Some(IndicatorUnsupportedReason::BasisNotSupported);
            };
            if !crate::chart::kline::fetch_plan::supports_open_interest_exchange(
                context.base_ticker.exchange(),
            ) {
                return Some(IndicatorUnsupportedReason::SourceNotSupported);
            }
            if !crate::chart::kline::fetch_plan::supports_open_interest_timeframe(timeframe) {
                return Some(IndicatorUnsupportedReason::ResolutionNotSupported);
            }
        }
        None
    }

    pub(crate) fn create_runtime(
        &self,
        config: IndicatorConfig,
    ) -> Result<ScriptIndicatorRuntime, IndicatorConfigError> {
        self.validate_config(&config)?;
        ScriptIndicatorRuntime::new(self.compiled.clone(), config)
    }
}

pub(crate) struct ScriptIndicatorRuntime {
    compiled: Arc<CompiledIndicator>,
    config: IndicatorConfig,
    instance: Option<IndicatorInstance>,
    outputs: IndicatorOutputStore,
    diagnostic: Option<String>,
}

impl ScriptIndicatorRuntime {
    fn new(
        compiled: Arc<CompiledIndicator>,
        config: IndicatorConfig,
    ) -> Result<Self, IndicatorConfigError> {
        let instance = Self::instantiate(&compiled, &config)?;
        Ok(Self {
            compiled,
            config,
            instance: Some(instance),
            outputs: IndicatorOutputStore::default(),
            diagnostic: None,
        })
    }

    fn quarantine(&mut self, phase: &str, error: impl std::fmt::Display) {
        if self.diagnostic.is_none() {
            let diagnostic = format!("{phase}: {error}");
            log::error!(
                "indicator {} quarantined: {diagnostic}",
                self.compiled.descriptor.id
            );
            self.diagnostic = Some(diagnostic);
        }
        self.outputs = IndicatorOutputStore::default();
        self.instance = None;
    }

    fn instantiate(
        compiled: &CompiledIndicator,
        config: &IndicatorConfig,
    ) -> Result<IndicatorInstance, IndicatorConfigError> {
        let next_config = config_to_runtime(config, &compiled.descriptor)?;
        compiled
            .instantiate_with_limits(next_config, RuntimeLimits::untrusted())
            .map_err(|error| {
                IndicatorConfigError::Deserialize(format!(
                    "indicator could not be planned: {error}"
                ))
            })
    }
}

impl ScriptIndicatorRuntime {
    pub(super) fn set_config(
        &mut self,
        config: &IndicatorConfig,
    ) -> Result<bool, IndicatorConfigError> {
        if self.config == *config {
            return Ok(false);
        }
        let instance = Self::instantiate(&self.compiled, config)?;
        self.instance = Some(instance);
        self.config = config.clone();
        self.outputs = IndicatorOutputStore::default();
        self.diagnostic = None;
        Ok(true)
    }

    pub(super) fn evaluate_closed(&mut self, frame: &IndicatorFrame) {
        if self.diagnostic.is_some() {
            return;
        }
        let result = self
            .instance
            .as_mut()
            .map(|instance| instance.evaluate_closed(frame.clone()));
        match result {
            Some(Ok(output)) => self.outputs.apply_output(output, false),
            Some(Err(error)) => {
                self.quarantine(&format!("closed bar {} failed", frame.index), error)
            }
            None => {}
        }
    }

    pub(super) fn preview_open(&mut self, frame: &IndicatorFrame) {
        if self.diagnostic.is_some() {
            return;
        }
        let result = self
            .instance
            .as_mut()
            .map(|instance| instance.preview_open(frame.clone()));
        match result {
            Some(Ok(output)) => self.outputs.apply_output(output, true),
            Some(Err(error)) => {
                self.quarantine(&format!("preview bar {} failed", frame.index), error)
            }
            None => {}
        }
    }

    pub(super) fn output_value(&self, channel_id: &str, index: usize) -> Option<OutputValue> {
        self.outputs
            .series_value(channel_id, index)
            .and_then(|value| match value {
                Value::Float(value) => finite_number(*value).map(OutputValue::Number),
                Value::Int(value) => finite_number(*value as f64).map(OutputValue::Number),
                Value::Bool(value) => Some(OutputValue::Boolean(*value)),
                _ => None,
            })
    }

    pub(super) fn output_color(
        &self,
        channel_id: &str,
        index: usize,
    ) -> Option<indicator_engine::Color> {
        self.outputs.series_color(channel_id, index)
    }

    pub(super) fn marker_outputs_at(&self, index: usize) -> Vec<MarkerOutput> {
        self.outputs.marker_values_at(index)
    }

    pub(super) fn segment_outputs(&self) -> impl Iterator<Item = &SegmentOutput> {
        self.outputs.segment_values()
    }

    pub(super) fn level_outputs(&self) -> Vec<HorizontalLevelOutput> {
        self.outputs.level_values()
    }

    pub(super) fn ohlc_output(&self, channel_id: &str, index: usize) -> Option<OhlcOutput> {
        self.outputs.ohlc_value(channel_id, index)
    }

    pub(super) fn range_output(&self, channel_id: &str, index: usize) -> Option<RangeOutput> {
        self.outputs.range_value(channel_id, index)
    }

    pub(super) fn region_outputs_in_range(
        &self,
        source_range: Option<Range<usize>>,
    ) -> Vec<RegionOutput> {
        self.outputs.region_values_in_range(source_range.as_ref())
    }

    pub(super) fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

fn finite_number(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn config_to_runtime(
    config: &IndicatorConfig,
    descriptor: &indicator_engine::IndicatorDescriptor,
) -> Result<EngineIndicatorConfig, IndicatorConfigError> {
    let object = config.value().as_object();
    let mut values = BTreeMap::new();
    for input in &descriptor.inputs {
        let value = match object.and_then(|object| object.get(&input.id)) {
            Some(value) => validated_input_value(input, value)?,
            None => input.default.clone(),
        };
        values.insert(input.id.clone(), value);
    }
    Ok(EngineIndicatorConfig { values })
}

fn input_value_from_json(value_type: &ValueType, value: &serde_json::Value) -> Option<InputValue> {
    match value_type {
        ValueType::Int => value.as_i64().map(InputValue::Int),
        ValueType::Float => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(InputValue::Float),
        ValueType::Bool => value.as_bool().map(InputValue::Bool),
        ValueType::String => value
            .as_str()
            .map(|value| InputValue::String(value.to_owned())),
        ValueType::Color => value
            .as_i64()
            .and_then(|value| u32::try_from(value).ok())
            .map(InputValue::Color),
        ValueType::Time => value.as_i64().map(InputValue::Time),
        _ => None,
    }
}

fn input_is_visible(
    input: &indicator_engine::InputDescriptor,
    inputs: &[indicator_engine::InputDescriptor],
    config: &serde_json::Value,
    input_coverage: &super::IndicatorInputCoverage,
) -> bool {
    if input
        .required_input
        .map(IndicatorInputSource::from)
        .is_some_and(|source| {
            matches!(
                input_coverage.state(source),
                IndicatorInputCoverageState::Missing
            )
        })
    {
        return false;
    }
    let Some(dependency) = input.visible_when.as_deref() else {
        return true;
    };
    let Some(controller) = inputs.iter().find(|candidate| candidate.id == dependency) else {
        return true;
    };
    config
        .as_object()
        .and_then(|values| values.get(dependency))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(matches!(&controller.default, InputValue::Bool(true)))
}

fn validated_input_value(
    input: &indicator_engine::InputDescriptor,
    value: &serde_json::Value,
) -> Result<InputValue, IndicatorConfigError> {
    let parsed = input_value_from_json(&input.value_type, value).ok_or_else(|| {
        IndicatorConfigError::Deserialize(format!("invalid value for input '{}'", input.id))
    })?;
    if !input.accepts(&parsed) {
        return Err(IndicatorConfigError::Deserialize(format!(
            "invalid value for input '{}'",
            input.id
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::kline::IndicatorInputCoverage;

    fn settings_context() -> IndicatorSettingsContext {
        IndicatorSettingsContext {
            basis: Basis::Time(exchange::Timeframe::M1),
            input_coverage: IndicatorInputCoverage::default(),
        }
    }

    fn input_coverage(directional_volume: bool) -> IndicatorInputCoverage {
        IndicatorInputCoverage::from_frames(&[IndicatorFrame {
            index: 0,
            time: 0,
            basis: indicator_engine::TimeOrTick::Time,
            instrument: indicator_engine::InstrumentContext { min_tick: None },
            chart: indicator_engine::ChartFrame {
                open: Some(1.0),
                high: Some(1.0),
                low: Some(1.0),
                close: Some(1.0),
                volume: Some(1.0),
                directional_volume_available: directional_volume,
                buy_qty: 1.0,
                sell_qty: 0.0,
                trade: None,
            },
            open_interest: None,
            series_inputs: Vec::new().into(),
        }])
    }

    #[test]
    fn indicator_metadata_is_exposed_to_host_definition() {
        let compiled = indicator_engine::compile(
            r#"
                language(1)
                indicator("Metadata", id = "catalog.metadata",
                          author = "Ada", author_url = "https://example.com/ada",
                          description = "A sample indicator",
                          source = "https://example.com/source")
                format(precision = "base_ticker_min_tick")
                plot(chart.close, id = "value")
            "#,
            "metadata.ind",
        )
        .expect("metadata source should compile");
        let registration = registration(compiled, PackageOrigin::Installed)
            .expect("metadata source should register");

        assert_eq!(
            registration.definition.author,
            Some(IndicatorAuthor {
                name: "Ada".to_owned(),
                url: "https://example.com/ada".to_owned(),
            })
        );
        assert_eq!(
            registration.definition.description.as_deref(),
            Some("A sample indicator")
        );
        assert_eq!(
            registration.definition.source.as_deref(),
            Some("https://example.com/source")
        );
    }

    #[test]
    fn visible_inputs_are_filtered_without_dropping_persisted_values() {
        let compiled = indicator_engine::compile(
            r#"
                language(1)
                indicator("Visibility", id = "catalog.visibility")
                format(precision = "base_ticker_min_tick")
                show = input.bool(false, "Show", group = "divergence")
                length = input.int(5, "Length", min = 1, max = 100,
                                   group = "divergence", visible_when = "show",
                                   tooltip = "Explains the setting")
                mode = input.bool(true, "Mode", group = "divergence",
                                  visible_when = "show")
                plot(chart.close, id = "value")
            "#,
            "visibility.ind",
        )
        .expect("visibility source should compile");
        let registration = registration(compiled, PackageOrigin::Installed)
            .expect("visibility source should register");

        let hidden = registration
            .settings(registration.default_config(), settings_context())
            .expect("default settings should be available");
        assert_eq!(
            hidden
                .iter()
                .map(|setting| setting.id.as_str())
                .collect::<Vec<_>>(),
            vec!["show"]
        );
        assert_eq!(hidden[0].tooltip, None);

        let enabled = registration
            .with_setting(
                registration.default_config(),
                "show",
                IndicatorSettingValue::Bool(true),
            )
            .expect("show setting should be accepted");
        let customized = registration
            .with_setting(enabled, "length", IndicatorSettingValue::I64(42))
            .expect("hidden setting value should be accepted");
        let shown = registration
            .settings(customized.clone(), settings_context())
            .expect("enabled settings should be available");
        assert_eq!(shown.len(), 3);
        assert_eq!(
            shown
                .iter()
                .find(|setting| setting.id == "length")
                .and_then(|setting| setting.tooltip.as_deref()),
            Some("Explains the setting")
        );
        assert!(
            shown
                .iter()
                .all(|setting| setting.group.as_deref() == Some("divergence"))
        );
        assert_eq!(
            shown
                .iter()
                .find(|setting| setting.id == "length")
                .map(|setting| &setting.value),
            Some(&IndicatorSettingValue::I64(42))
        );

        let disabled = registration
            .with_setting(customized, "show", IndicatorSettingValue::Bool(false))
            .expect("show setting should be accepted");
        let hidden_again = registration
            .settings(disabled.clone(), settings_context())
            .expect("disabled settings should be available");
        assert_eq!(hidden_again.len(), 1);
        let reenabled = registration
            .with_setting(disabled, "show", IndicatorSettingValue::Bool(true))
            .expect("show setting should be accepted");
        let restored = registration
            .settings(reenabled, settings_context())
            .expect("re-enabled settings should be available");
        assert_eq!(
            restored
                .iter()
                .find(|setting| setting.id == "length")
                .map(|setting| &setting.value),
            Some(&IndicatorSettingValue::I64(42))
        );
    }

    #[test]
    fn host_input_dependencies_hide_settings_and_styles_without_coverage() {
        let compiled = indicator_engine::compile(
            r#"
                language(1)
                indicator("Dependencies", id = "catalog.dependencies")
                format(precision = "base_ticker_min_tick")
                highlight = input.bool(false, "Highlight", requires = "directional_volume")
                plot(chart.close, id = "value", color_slot = "main")
                style("main", title = "Main")
                style("warning", title = "Warning", role = "warning",
                      requires = "directional_volume")
            "#,
            "dependencies.ind",
        )
        .expect("dependency source should compile");
        let registration = registration(compiled, PackageOrigin::Installed)
            .expect("dependency source should register");

        assert_eq!(
            registration
                .definition
                .style_slots
                .iter()
                .find(|style| style.id == "warning")
                .and_then(|style| style.required_input),
            Some(IndicatorInputSource::DirectionalVolume)
        );

        let unavailable = registration
            .settings(
                registration.default_config(),
                IndicatorSettingsContext {
                    basis: Basis::Time(exchange::Timeframe::M1),
                    input_coverage: input_coverage(false),
                },
            )
            .expect("unavailable settings should resolve");
        assert!(unavailable.is_empty());

        let available = registration
            .settings(
                registration.default_config(),
                IndicatorSettingsContext {
                    basis: Basis::Time(exchange::Timeframe::M1),
                    input_coverage: input_coverage(true),
                },
            )
            .expect("available settings should resolve");
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].id, "highlight");
    }

    fn region(key: i64, upper: f64) -> RegionOutput {
        RegionOutput {
            id: "zones".to_owned(),
            key,
            left_index: 1,
            right_index: 3,
            upper,
            lower: upper - 1.0,
            border_color: None,
            fill_color: None,
            border_color_slot: None,
            fill_color_slot: None,
            extend_right: true,
            width: 1.0,
        }
    }

    #[test]
    fn region_store_replaces_the_active_snapshot_and_duplicate_keys() {
        let mut store = IndicatorOutputStore::default();
        let first = FrameOutput {
            regions: vec![region(1, 10.0), region(2, 20.0)].into(),
            ..Default::default()
        };
        store.apply_output(first, false);
        assert_eq!(store.region_values().len(), 2);

        let second = FrameOutput {
            regions: vec![region(1, 30.0), region(1, 40.0)].into(),
            ..Default::default()
        };
        store.apply_output(second, false);
        let regions = store.region_values();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].upper, 40.0);
    }

    #[test]
    fn persistent_region_store_commits_and_previews_transactionally() {
        let mut store = IndicatorOutputStore::default();
        let closed = FrameOutput {
            region_updates: vec![RegionUpdate::Upsert(region(1, 10.0))].into(),
            ..Default::default()
        };
        store.apply_output(closed, false);
        assert_eq!(store.region_values()[0].upper, 10.0);

        let preview = FrameOutput {
            region_updates: vec![
                RegionUpdate::Upsert(region(1, 20.0)),
                RegionUpdate::Upsert(region(2, 30.0)),
            ]
            .into(),
            ..Default::default()
        };
        store.apply_output(preview, true);
        let preview_regions = store.region_values();
        assert_eq!(preview_regions.len(), 2);
        assert_eq!(
            preview_regions
                .iter()
                .find(|region| region.key == 1)
                .map(|region| region.upper),
            Some(20.0)
        );

        store.apply_output(FrameOutput::default(), true);
        let reverted = store.region_values();
        assert_eq!(reverted.len(), 1);
        assert_eq!(reverted[0].upper, 10.0);

        let preview_removed = FrameOutput {
            region_updates: vec![RegionUpdate::Remove {
                id: "zones".to_owned(),
                key: 1,
            }]
            .into(),
            ..Default::default()
        };
        store.apply_output(preview_removed, true);
        assert!(store.region_values().is_empty());
        store.apply_output(FrameOutput::default(), true);
        assert_eq!(store.region_values()[0].upper, 10.0);

        let removed = FrameOutput {
            region_updates: vec![RegionUpdate::Remove {
                id: "zones".to_owned(),
                key: 1,
            }]
            .into(),
            ..Default::default()
        };
        store.apply_output(removed, false);
        assert!(store.region_values().is_empty());
    }

    #[test]
    fn persistent_region_store_evicts_oldest_without_reordering_updates() {
        let mut store = IndicatorOutputStore::default();
        let limit = RuntimeLimits::untrusted().max_output_items;
        let updates = (0..limit as i64)
            .map(|key| RegionUpdate::Upsert(region(key, key as f64)))
            .collect::<Vec<_>>();
        store.apply_output(
            FrameOutput {
                region_updates: updates.into(),
                ..Default::default()
            },
            false,
        );

        store.apply_output(
            FrameOutput {
                region_updates: vec![
                    RegionUpdate::Upsert(region(1, 100.0)),
                    RegionUpdate::Upsert(region(limit as i64, 200.0)),
                ]
                .into(),
                ..Default::default()
            },
            false,
        );

        let regions = store.region_values();
        assert_eq!(regions.len(), limit);
        assert!(!regions.iter().any(|region| region.key == 0));
        assert!(
            regions
                .iter()
                .any(|region| region.key == 1 && region.upper == 100.0)
        );
        assert!(regions.iter().any(|region| region.key == limit as i64));
    }

    #[test]
    fn preview_region_retention_matches_commit_without_mutating_base() {
        let mut store = IndicatorOutputStore::default();
        let limit = RuntimeLimits::untrusted().max_output_items;
        let committed_updates = (0..limit as i64)
            .map(|key| RegionUpdate::Upsert(region(key, key as f64)))
            .collect::<Vec<_>>();
        store.apply_output(
            FrameOutput {
                region_updates: committed_updates.into(),
                ..Default::default()
            },
            false,
        );

        let preview_update = RegionUpdate::Upsert(region(limit as i64, 100.0));
        store.apply_output(
            FrameOutput {
                region_updates: vec![preview_update.clone()].into(),
                ..Default::default()
            },
            true,
        );
        let preview_regions = store.region_values();
        assert_eq!(preview_regions.len(), limit);
        assert!(!preview_regions.iter().any(|region| region.key == 0));
        assert!(
            preview_regions
                .iter()
                .any(|region| region.key == limit as i64)
        );

        store.apply_output(FrameOutput::default(), true);
        let reverted = store.region_values();
        assert_eq!(reverted.len(), limit);
        assert!(reverted.iter().any(|region| region.key == 0));
        assert!(!reverted.iter().any(|region| region.key == limit as i64));

        store.apply_output(
            FrameOutput {
                region_updates: vec![preview_update].into(),
                ..Default::default()
            },
            false,
        );
        let committed = store.region_values();
        assert_eq!(committed.len(), limit);
        assert!(!committed.iter().any(|region| region.key == 0));
        assert!(committed.iter().any(|region| region.key == limit as i64));
    }
}
