mod bundled;
mod core;
pub mod language;
mod loader;
mod registry;
pub mod scripting;

use super::composition::{
    AxisBinding, LayerDataKind, MarkKind, PanelScaleMode, PanelValueLabelPolicy,
    PanelValuePrecision,
};
use crate::chart::Basis;
use crate::config::theme::{ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::adapter::MarketKind;
use exchange::{TickerInfo, Timeframe, UnixMs};
use indicator_engine::IndicatorFrame;
use indicator_engine::PaneTarget;
pub use indicator_engine::{BaseSeriesSource, IndicatorAuthor};
use std::fmt::{Display, Formatter};
use std::ops::Range;
use std::sync::Arc;

pub(crate) fn region_intersects_source_range(
    region: &indicator_engine::RegionOutput,
    source_range: &Range<usize>,
) -> bool {
    if source_range.start >= source_range.end {
        return false;
    }
    if region.extend_right {
        return region.left_index < source_range.end;
    }
    let left = region.left_index.min(region.right_index);
    let right = region.left_index.max(region.right_index);
    left < source_range.end && right >= source_range.start
}

pub use core::input::{IndicatorInputRequirement, IndicatorInputSource, OpenInterestSeries};
pub use core::output::{IndicatorOutputRef, OutputChannelSpec, OutputValue, OutputValueType};
pub use core::runtime::IndicatorHost;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndicatorInputAvailability {
    pub klines: bool,
    pub directional_volume: bool,
    pub trades: bool,
    pub open_interest: bool,
}

impl IndicatorInputAvailability {
    fn from_frame(frame: &IndicatorFrame) -> Self {
        Self {
            klines: frame.chart.close.is_some(),
            directional_volume: frame.chart.directional_volume_available,
            trades: frame.chart.trade.is_some(),
            open_interest: frame.open_interest.is_some(),
        }
    }

    fn contains(self, source: IndicatorInputSource) -> bool {
        match source {
            IndicatorInputSource::Klines => self.klines,
            IndicatorInputSource::DirectionalVolume => self.directional_volume,
            IndicatorInputSource::Trades => self.trades,
            IndicatorInputSource::OpenInterest => self.open_interest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorInputCoverageState {
    Unknown,
    Available,
    Missing,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndicatorInputCoverage {
    by_runtime_index: Arc<Vec<IndicatorInputAvailability>>,
    prefix_offset: usize,
}

impl IndicatorInputCoverage {
    pub fn remap(&mut self, previous_indices: &[Option<usize>]) {
        let by_runtime_index = previous_indices
            .iter()
            .map(|index| {
                index
                    .and_then(|index| self.at_index(index))
                    .unwrap_or_default()
            })
            .collect();
        self.by_runtime_index = Arc::new(by_runtime_index);
        self.prefix_offset = 0;
    }

    pub fn remap_prefix(&mut self, prefix: usize) {
        self.prefix_offset = self.prefix_offset.saturating_add(prefix);
    }

    fn at_index(&self, runtime_index: usize) -> Option<IndicatorInputAvailability> {
        runtime_index
            .checked_sub(self.prefix_offset)
            .and_then(|index| self.by_runtime_index.get(index))
            .copied()
    }

    pub fn from_frames(frames: &[IndicatorFrame]) -> Self {
        let mut by_runtime_index = vec![
            IndicatorInputAvailability::default();
            frames
                .iter()
                .map(|frame| frame.index)
                .max()
                .map_or(0, |index| index.saturating_add(1))
        ];
        for frame in frames {
            by_runtime_index[frame.index] = IndicatorInputAvailability::from_frame(frame);
        }

        Self {
            by_runtime_index: Arc::new(by_runtime_index),
            prefix_offset: 0,
        }
    }

    pub fn update_frames(&mut self, frames: &[IndicatorFrame]) {
        let Some(max_index) = frames
            .iter()
            .filter_map(|frame| frame.index.checked_sub(self.prefix_offset))
            .max()
        else {
            return;
        };

        let by_runtime_index = Arc::make_mut(&mut self.by_runtime_index);
        by_runtime_index.resize(
            by_runtime_index.len().max(max_index.saturating_add(1)),
            Default::default(),
        );
        for frame in frames {
            let Some(index) = frame.index.checked_sub(self.prefix_offset) else {
                continue;
            };
            by_runtime_index[index] = IndicatorInputAvailability::from_frame(frame);
        }
    }

    pub fn state(&self, source: IndicatorInputSource) -> IndicatorInputCoverageState {
        if self.by_runtime_index.is_empty() {
            return IndicatorInputCoverageState::Unknown;
        }
        if self
            .by_runtime_index
            .iter()
            .copied()
            .any(|availability| availability.contains(source))
        {
            IndicatorInputCoverageState::Available
        } else {
            IndicatorInputCoverageState::Missing
        }
    }

    pub fn at(&self, source: IndicatorInputSource, runtime_index: usize) -> bool {
        self.at_index(runtime_index)
            .is_some_and(|availability| availability.contains(source))
    }
}
pub use loader::{
    IndicatorCatalogPaths, initialize_indicator_catalog, initialize_indicator_catalog_with_paths,
};
pub(crate) use registry::install_catalog_with_core_diagnostics;
pub use registry::{
    CatalogInstallError, IndicatorCatalog, IndicatorConfig, IndicatorConfigError, IndicatorId,
    IndicatorIdError, IndicatorRegistration, all_indicators, availability, catalog,
    declared_input_requirements, display_name, indicator_definition, is_supported_for_market,
    panel_recipe, requires_trade_data, table_columns, unsupported_reason,
};

/// Shared chart-facing types for indicator source integrations.
pub mod prelude {
    pub use super::super::composition::{
        AxisBinding, BarMode, DataSourceId, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
        PanelValueLabelMode, PanelValueLabelPolicy, PanelValuePrecision,
    };
    pub use super::{
        AvailabilityContext, IndicatorAttachmentMode, IndicatorAuthor, IndicatorAvailability,
        IndicatorColorSpec, IndicatorDefinition, IndicatorDefinitionError, IndicatorFillGeometry,
        IndicatorFillSpec, IndicatorHistoryRequirement, IndicatorInputRequirement,
        IndicatorInputSource, IndicatorMarkSpec, IndicatorOutputLayerSpec, IndicatorOutputRef,
        IndicatorOverlayChannelSpec, IndicatorOverlaySource, IndicatorPanelRecipe,
        IndicatorReadoutScope, IndicatorReadoutSpec, IndicatorSegmentSpec, IndicatorSetting,
        IndicatorSettingChoice, IndicatorSettingControl, IndicatorSettingValue,
        IndicatorSettingsContext, IndicatorSourceInput, IndicatorUnsupportedReason,
        OpenInterestSeries, OutputChannelSpec, OutputValue, OutputValueType, TableColumnColor,
        TableColumnFormat, TableColumnSpec, TableColumnValue,
    };
    pub use crate::chart::Basis;
    pub use crate::chart::kline::IndicatorUpdateScope;
    pub use crate::config::theme::{ThemeColor, ThemeColorRole, ThemeColorShade};
    pub use crate::serde;
    pub use exchange::adapter::MarketKind;
    pub use exchange::{Kline, TickerInfo, Trade, UnixMs};
}

/// Shared Open Interest capability used by both indicator availability and
/// the connector fetch planner.
pub fn supports_open_interest(ticker_info: TickerInfo, timeframe: Timeframe) -> bool {
    super::feed_capabilities::supports_open_interest(ticker_info, timeframe)
}

/// Declarative metadata shared by startup-loaded indicator sources.
///
/// The catalog supplies stable IDs and type-erased factories, while the
/// compiled `.ind` programs describe their feeds and channels here, so chart
/// code does not need implementation-specific dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorSourceInput {
    pub id: String,
    pub label: String,
    pub default_label: String,
    pub allow_indicator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorAttachmentMode {
    Overlay,
    Panel,
}

#[derive(Debug, Clone)]
pub struct IndicatorDefinition {
    pub display_name: String,
    pub author: Option<IndicatorAuthor>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub sourceable: bool,
    pub inputs: Vec<IndicatorInputRequirement>,
    pub source_inputs: Vec<IndicatorSourceInput>,
    pub attachment_mode: IndicatorAttachmentMode,
    pub outputs: Vec<OutputChannelSpec>,
    pub style_slots: Vec<IndicatorColorSpec>,
    pub region_style_slots: Vec<String>,
    pub mark_specs: Vec<IndicatorMarkSpec>,
    pub segment_specs: Vec<IndicatorSegmentSpec>,
    pub overlay_channels: Vec<IndicatorOverlayChannelSpec>,
    pub fill_specs: Vec<IndicatorFillSpec>,
    pub supported_markets: Vec<MarketKind>,
    pub panel_recipe: IndicatorPanelRecipe,
    pub availability: fn(AvailabilityContext) -> IndicatorAvailability,
}

/// One output channel bound to a layer in an indicator's default view.
///
/// The channel ID is resolved against the definition at composition time, so
/// a view never needs to know about an indicator's implementation type.
#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorOutputLayerSpec {
    pub channel_id: String,
    pub layer_name: String,
    pub pane: PaneTarget,
    pub data_kind: LayerDataKind,
    pub mark: MarkKind,
    pub axis: AxisBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorDefinitionError {
    EmptyDisplayName,
    EmptyInputId,
    EmptyOutputId,
    EmptyOutputLabel,
    InvalidOutputPrecision,
    InvalidOutputWidth,
    DuplicateInput,
    DuplicateOutput,
    EmptyStyleSlot,
    DuplicateStyleSlot,
    UnknownOutputStyleSlot,
    EmptyMarkSource,
    UnknownMarkSource,
    InvalidMarkFallback,
    EmptySegmentId,
    EmptySegmentLabel,
    DuplicateSegment,
    InvalidSegmentWidth,
    UnknownSegmentStyleSlot,
    EmptyLayerName,
    IncompatibleLayerMark,
    InvalidOverlayWidth,
    InvalidFillOpacity,
    UnknownStyleBinding,
    EmptyReadoutLabel,
    EmptyTableColumnLabel,
    EmptyOutputLayers,
    UnknownViewOutput,
    DuplicateViewOutput,
    BooleanPlotOutput,
    IncompatibleTableOutput,
}

impl IndicatorDefinition {
    pub fn new(
        display_name: impl Into<String>,
        inputs: impl Into<Vec<IndicatorInputRequirement>>,
        outputs: impl Into<Vec<OutputChannelSpec>>,
        panel_recipe: IndicatorPanelRecipe,
    ) -> Self {
        Self {
            display_name: display_name.into(),
            author: None,
            description: None,
            source: None,
            sourceable: false,
            inputs: inputs.into(),
            source_inputs: Vec::new(),
            attachment_mode: IndicatorAttachmentMode::Panel,
            outputs: outputs.into(),
            style_slots: Vec::new(),
            region_style_slots: Vec::new(),
            mark_specs: Vec::new(),
            segment_specs: Vec::new(),
            overlay_channels: Vec::new(),
            fill_specs: Vec::new(),
            supported_markets: Vec::new(),
            panel_recipe,
            availability: default_availability,
        }
    }

    pub fn with_style_slots(mut self, style_slots: impl Into<Vec<IndicatorColorSpec>>) -> Self {
        self.style_slots = style_slots.into();
        Self { ..self }
    }

    pub fn with_region_style_slots(mut self, slots: impl Into<Vec<String>>) -> Self {
        self.region_style_slots = slots.into();
        Self { ..self }
    }

    pub fn with_source_inputs(
        mut self,
        source_inputs: impl Into<Vec<IndicatorSourceInput>>,
    ) -> Self {
        self.source_inputs = source_inputs.into();
        Self { ..self }
    }

    pub fn with_sourceable(mut self, sourceable: bool) -> Self {
        self.sourceable = sourceable;
        Self { ..self }
    }

    pub fn with_author(mut self, author: Option<IndicatorAuthor>) -> Self {
        self.author = author;
        Self { ..self }
    }

    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description;
        Self { ..self }
    }

    pub fn with_source(mut self, source: Option<String>) -> Self {
        self.source = source;
        Self { ..self }
    }

    pub fn with_attachment_mode(mut self, attachment_mode: IndicatorAttachmentMode) -> Self {
        self.attachment_mode = attachment_mode;
        Self { ..self }
    }

    pub fn with_display_specs(
        mut self,
        mark_specs: impl Into<Vec<IndicatorMarkSpec>>,
        overlay_channels: impl Into<Vec<IndicatorOverlayChannelSpec>>,
        fill_specs: impl Into<Vec<IndicatorFillSpec>>,
    ) -> Self {
        self.mark_specs = mark_specs.into();
        self.overlay_channels = overlay_channels.into();
        self.fill_specs = fill_specs.into();
        Self { ..self }
    }

    pub fn with_segment_specs(
        mut self,
        segment_specs: impl Into<Vec<IndicatorSegmentSpec>>,
    ) -> Self {
        self.segment_specs = segment_specs.into();
        Self { ..self }
    }

    pub fn with_supported_markets(mut self, supported_markets: impl Into<Vec<MarketKind>>) -> Self {
        self.supported_markets = supported_markets.into();
        Self { ..self }
    }

    pub fn with_availability(
        mut self,
        availability: fn(AvailabilityContext) -> IndicatorAvailability,
    ) -> Self {
        self.availability = availability;
        Self { ..self }
    }

    pub fn output(&self, id: &str) -> Option<&OutputChannelSpec> {
        self.outputs.iter().find(|output| output.id == id)
    }

    pub fn numeric_plot_output(&self, id: &str) -> Option<&OutputChannelSpec> {
        let output = self.output(id)?;
        (output.value_type == OutputValueType::Number
            && self
                .output_layers()
                .iter()
                .any(|layer| layer.channel_id == id))
        .then_some(output)
    }

    pub fn sourceable_numeric_output(&self, id: &str) -> Option<&OutputChannelSpec> {
        self.sourceable
            .then(|| self.numeric_plot_output(id))
            .flatten()
    }

    pub fn has_numeric_plot_layers(&self) -> bool {
        self.output_layers()
            .iter()
            .any(|layer| self.numeric_plot_output(&layer.channel_id).is_some())
    }

    pub fn default_view(&self) -> &IndicatorPanelRecipe {
        &self.panel_recipe
    }

    pub fn output_layers(&self) -> &[IndicatorOutputLayerSpec] {
        match &self.panel_recipe {
            IndicatorPanelRecipe::Plot { layers, .. }
            | IndicatorPanelRecipe::PrimaryOverlay { layers, .. } => layers,
            IndicatorPanelRecipe::Table { .. } => &[],
        }
    }

    pub fn segment_specs(&self) -> &[IndicatorSegmentSpec] {
        &self.segment_specs
    }

    pub fn requires(&self, source: IndicatorInputSource) -> bool {
        self.inputs
            .iter()
            .any(|requirement| requirement.source == source && requirement.is_required())
    }

    pub fn uses(&self, source: IndicatorInputSource) -> bool {
        self.inputs
            .iter()
            .any(|requirement| requirement.source == source)
    }

    pub fn availability(&self, context: AvailabilityContext) -> IndicatorAvailability {
        (self.availability)(context)
    }

    pub fn validate(&self) -> Result<(), IndicatorDefinitionError> {
        if self.display_name.trim().is_empty() {
            return Err(IndicatorDefinitionError::EmptyDisplayName);
        }

        for (index, input) in self.inputs.iter().enumerate() {
            if input.id().trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyInputId);
            }
            if self.inputs[..index]
                .iter()
                .any(|previous| previous.source == input.source)
            {
                return Err(IndicatorDefinitionError::DuplicateInput);
            }
        }

        for (index, output) in self.outputs.iter().enumerate() {
            if output.id.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyOutputId);
            }
            if output.label.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyOutputLabel);
            }
            if !output.precision.is_valid() {
                return Err(IndicatorDefinitionError::InvalidOutputPrecision);
            }
            if !output.line_width.is_finite() || output.line_width <= 0.0 {
                return Err(IndicatorDefinitionError::InvalidOutputWidth);
            }
            if self.outputs[..index]
                .iter()
                .any(|previous| previous.id == output.id)
            {
                return Err(IndicatorDefinitionError::DuplicateOutput);
            }
            if let Some(style_slot) = output.style_slot.as_deref() {
                if style_slot.trim().is_empty() {
                    return Err(IndicatorDefinitionError::EmptyStyleSlot);
                }
                if !self.style_slots.iter().any(|slot| slot.id == style_slot) {
                    return Err(IndicatorDefinitionError::UnknownOutputStyleSlot);
                }
            }
            if let Some(style_by) = output.style_by.as_deref()
                && self
                    .output(style_by)
                    .is_none_or(|binding| binding.value_type != OutputValueType::Boolean)
            {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
        }

        for (index, style_slot) in self.style_slots.iter().enumerate() {
            if style_slot.id.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyStyleSlot);
            }
            if self.style_slots[..index]
                .iter()
                .any(|previous| previous.id == style_slot.id)
            {
                return Err(IndicatorDefinitionError::DuplicateStyleSlot);
            }
        }

        let validate_style_slot = |style_slot: &str| {
            if style_slot.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyStyleSlot);
            }
            if !self.style_slots.iter().any(|slot| slot.id == style_slot) {
                return Err(IndicatorDefinitionError::UnknownOutputStyleSlot);
            }
            Ok(())
        };
        for style_slot in &self.region_style_slots {
            validate_style_slot(style_slot)?;
        }
        let validate_source = |source: &IndicatorOverlaySource| match source {
            IndicatorOverlaySource::Channel(channel) => self
                .output(channel)
                .map(|_| ())
                .ok_or(IndicatorDefinitionError::UnknownViewOutput),
        };
        let validate_numeric_source = |source: &IndicatorOverlaySource| {
            validate_source(source)?;
            let IndicatorOverlaySource::Channel(channel) = source;
            if self
                .output(channel)
                .is_some_and(|output| output.value_type == OutputValueType::Number)
            {
                Ok(())
            } else {
                Err(IndicatorDefinitionError::BooleanPlotOutput)
            }
        };

        for mark in &self.mark_specs {
            if mark.label.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyLayerName);
            }
            if mark.source_channel.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyMarkSource);
            }
            let Some(output) = self.output(&mark.source_channel) else {
                return Err(IndicatorDefinitionError::UnknownMarkSource);
            };
            if output.value_type != OutputValueType::Number {
                return Err(IndicatorDefinitionError::UnknownMarkSource);
            }
            if mark.color_slot.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptyStyleSlot);
            }
            validate_style_slot(&mark.color_slot)?;
            if mark.true_color_slot.is_some() != mark.false_color_slot.is_some() {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
            if let Some(slot) = mark.true_color_slot.as_deref() {
                validate_style_slot(slot)?;
            }
            if let Some(slot) = mark.false_color_slot.as_deref() {
                validate_style_slot(slot)?;
            }
            if mark.highlight_by.is_some() != mark.highlight_color_slot.is_some() {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
            if let Some(highlight_by) = mark.highlight_by.as_deref()
                && (self
                    .output(highlight_by)
                    .is_none_or(|binding| binding.value_type != OutputValueType::Boolean)
                    || !matches!(
                        mark.mark,
                        MarkKind::Bar(
                            super::composition::BarMode::Regular
                                | super::composition::BarMode::Histogram(_)
                        )
                    ))
            {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
            if let Some(slot) = mark.highlight_color_slot.as_deref() {
                validate_style_slot(slot)?;
            }
        }

        for mark in &self.mark_specs {
            let Some(fallback_mark) = mark.fallback_mark else {
                continue;
            };
            if fallback_mark == mark.mark {
                return Err(IndicatorDefinitionError::InvalidMarkFallback);
            }
            let fallback_marks = self
                .mark_specs
                .iter()
                .filter(|candidate| candidate.mark == fallback_mark)
                .collect::<Vec<_>>();
            if fallback_marks.len() != 1 || fallback_marks[0].required_input.is_some() {
                return Err(IndicatorDefinitionError::InvalidMarkFallback);
            }
        }

        for (index, segment) in self.segment_specs.iter().enumerate() {
            if segment.id.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptySegmentId);
            }
            if segment.label.trim().is_empty() {
                return Err(IndicatorDefinitionError::EmptySegmentLabel);
            }
            if self.segment_specs[..index]
                .iter()
                .any(|previous| previous.id == segment.id)
            {
                return Err(IndicatorDefinitionError::DuplicateSegment);
            }
            if !segment.line_width.is_finite() || segment.line_width <= 0.0 {
                return Err(IndicatorDefinitionError::InvalidSegmentWidth);
            }
            if segment.color_slot.trim().is_empty()
                || !self
                    .style_slots
                    .iter()
                    .any(|style| style.id == segment.color_slot)
            {
                return Err(IndicatorDefinitionError::UnknownSegmentStyleSlot);
            }
        }

        for channel in &self.overlay_channels {
            validate_numeric_source(&channel.source)?;
            validate_style_slot(&channel.color_slot)?;
            if !channel.line_width.is_finite() || channel.line_width <= 0.0 {
                return Err(IndicatorDefinitionError::InvalidOverlayWidth);
            }
            if let Some(style_by) = channel.style_by.as_deref()
                && self
                    .output(style_by)
                    .is_none_or(|binding| binding.value_type != OutputValueType::Boolean)
            {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
        }

        for fill in &self.fill_specs {
            validate_numeric_source(&fill.upper)?;
            validate_numeric_source(&fill.lower)?;
            validate_style_slot(&fill.color_slot)?;
            if fill.true_color_slot.is_some() != fill.false_color_slot.is_some() {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
            if let Some(style_slot) = fill.true_color_slot.as_deref() {
                validate_style_slot(style_slot)?;
            }
            if let Some(style_slot) = fill.false_color_slot.as_deref() {
                validate_style_slot(style_slot)?;
            }
            if let Some(style_by) = fill.style_by.as_deref()
                && self
                    .output(style_by)
                    .is_none_or(|binding| binding.value_type != OutputValueType::Boolean)
            {
                return Err(IndicatorDefinitionError::UnknownStyleBinding);
            }
            if !fill.opacity.is_finite() || !(0.0..=1.0).contains(&fill.opacity) {
                return Err(IndicatorDefinitionError::InvalidFillOpacity);
            }
        }

        let validate_layers = |layers: &[IndicatorOutputLayerSpec]| {
            for (index, layer) in layers.iter().enumerate() {
                if layer.layer_name.trim().is_empty() {
                    return Err(IndicatorDefinitionError::EmptyLayerName);
                }
                let Some(output) = self.output(&layer.channel_id) else {
                    return Err(IndicatorDefinitionError::UnknownViewOutput);
                };
                if output.value_type == OutputValueType::Boolean {
                    return Err(IndicatorDefinitionError::BooleanPlotOutput);
                }
                if super::composition::resolve_mark_for_data_kind(layer.mark, layer.data_kind)
                    != layer.mark
                {
                    return Err(IndicatorDefinitionError::IncompatibleLayerMark);
                }
                if layers[..index]
                    .iter()
                    .any(|previous| previous.channel_id == layer.channel_id)
                {
                    return Err(IndicatorDefinitionError::DuplicateViewOutput);
                }
            }
            Ok(())
        };

        match &self.panel_recipe {
            IndicatorPanelRecipe::Plot {
                layers, readouts, ..
            } => {
                if layers.is_empty() {
                    return Err(IndicatorDefinitionError::EmptyOutputLayers);
                }
                validate_layers(layers)?;
                for readout in readouts {
                    if readout.label.trim().is_empty() {
                        return Err(IndicatorDefinitionError::EmptyReadoutLabel);
                    }
                    validate_numeric_source(&readout.source)?;
                    validate_style_slot(&readout.color_slot)?;
                }
            }
            IndicatorPanelRecipe::Table { columns, .. } => {
                for column in columns {
                    if column.label.trim().is_empty() {
                        return Err(IndicatorDefinitionError::EmptyTableColumnLabel);
                    }
                    match &column.color {
                        TableColumnColor::Static { slot } => {
                            validate_style_slot(slot)?;
                        }
                        TableColumnColor::Signed {
                            positive_slot,
                            negative_slot,
                        } => {
                            validate_style_slot(positive_slot)?;
                            validate_style_slot(negative_slot)?;
                        }
                    }
                    let output = match &column.value {
                        TableColumnValue::Channel(channel) => {
                            validate_source(&IndicatorOverlaySource::Channel(channel.clone()))?;
                            self.output(channel)
                        }
                    }
                    .ok_or(IndicatorDefinitionError::UnknownViewOutput)?;
                    let expected_type = match column.format {
                        TableColumnFormat::Boolean => OutputValueType::Boolean,
                        TableColumnFormat::Quantity
                        | TableColumnFormat::Integer
                        | TableColumnFormat::Percent => OutputValueType::Number,
                    };
                    if output.value_type != expected_type {
                        return Err(IndicatorDefinitionError::IncompatibleTableOutput);
                    }
                    if matches!(&column.color, TableColumnColor::Signed { .. })
                        && output.value_type != OutputValueType::Number
                    {
                        return Err(IndicatorDefinitionError::IncompatibleTableOutput);
                    }
                }
            }
            IndicatorPanelRecipe::PrimaryOverlay { layers, .. } => {
                if layers.is_empty() {
                    return Err(IndicatorDefinitionError::EmptyOutputLayers);
                }
                validate_layers(layers)?;
            }
        }

        Ok(())
    }
}

fn default_availability(_context: AvailabilityContext) -> IndicatorAvailability {
    IndicatorAvailability::Available
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IndicatorHistoryRequirement {
    history_start: Option<UnixMs>,
    history_bars: u64,
}

impl IndicatorHistoryRequirement {
    pub fn new(history_start: Option<UnixMs>, history_bars: u64) -> Self {
        Self {
            history_start,
            history_bars,
        }
    }

    pub fn history_start(self) -> Option<UnixMs> {
        self.history_start
    }

    pub fn history_bars(self) -> u64 {
        self.history_bars
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            history_start: match (self.history_start, other.history_start) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(time), None) | (None, Some(time)) => Some(time),
                (None, None) => None,
            },
            history_bars: self.history_bars.max(other.history_bars),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorColorSpec {
    pub id: String,
    pub label: String,
    pub default_role: ThemeColorRole,
    pub default_shade: ThemeColorShade,
    pub required_input: Option<IndicatorInputSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorMarkSpec {
    pub label: String,
    pub mark: MarkKind,
    pub source_channel: String,
    pub required_input: Option<IndicatorInputSource>,
    pub fallback_mark: Option<MarkKind>,
    /// Fallback style when no conditional binding is available.
    pub color_slot: String,
    /// Explicit style selected when the mark's boolean binding is true.
    pub true_color_slot: Option<String>,
    /// Explicit style selected when the mark's boolean binding is false.
    pub false_color_slot: Option<String>,
    /// Optional boolean output that draws a non-destructive bar outline.
    pub highlight_by: Option<String>,
    /// Color slot used by the optional bar outline.
    pub highlight_color_slot: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSegmentSpec {
    pub id: String,
    pub label: String,
    pub color_slot: String,
    pub line_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorOverlaySource {
    Channel(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorOverlayChannelSpec {
    pub label: String,
    pub source: IndicatorOverlaySource,
    pub line_width: f32,
    pub color_role: ThemeColorRole,
    pub color_slot: String,
    pub style_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorReadoutScope {
    Always,
    Mark(MarkKind),
}

impl IndicatorReadoutScope {
    pub fn matches(self, mark: MarkKind) -> bool {
        match self {
            Self::Always => true,
            Self::Mark(expected) => expected == mark,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorReadoutSpec {
    pub label: String,
    pub source: IndicatorOverlaySource,
    pub color_role: ThemeColorRole,
    pub color_slot: String,
    pub scope: IndicatorReadoutScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorFillGeometry {
    PanelBand,
    SeriesBand,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorFillSpec {
    pub upper: IndicatorOverlaySource,
    pub lower: IndicatorOverlaySource,
    pub upper_offset_bars: i32,
    pub lower_offset_bars: i32,
    pub style_by: Option<String>,
    pub geometry: IndicatorFillGeometry,
    pub color_slot: String,
    pub true_color_slot: Option<String>,
    pub false_color_slot: Option<String>,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableColumnValue {
    Channel(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnFormat {
    Quantity,
    Integer,
    Percent,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableColumnColor {
    Static {
        slot: String,
    },
    Signed {
        positive_slot: String,
        negative_slot: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumnSpec {
    pub label: String,
    pub value: TableColumnValue,
    pub format: TableColumnFormat,
    pub color: TableColumnColor,
}

impl IndicatorColorSpec {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        default_role: ThemeColorRole,
        default_shade: ThemeColorShade,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            default_role,
            default_shade,
            required_input: None,
        }
    }

    pub fn default_color(&self) -> ThemeColor {
        ThemeColor::new(self.default_role, self.default_shade)
    }

    pub fn with_required_input(mut self, required_input: Option<IndicatorInputSource>) -> Self {
        self.required_input = required_input;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorUnsupportedReason {
    NotInstalled,
    BasisNotSupported,
    IncompleteHistory,
    SourceNotSupported,
    ResolutionNotSupported,
    InsufficientData { available: usize, required: usize },
    MissingRequiredInput { source: IndicatorInputSource },
    MissingDirectionalVolume,
}

impl IndicatorUnsupportedReason {
    pub fn indicator_unavailable_tooltip(&self) -> &'static str {
        match self {
            IndicatorUnsupportedReason::NotInstalled => "Not included in this build",
            IndicatorUnsupportedReason::BasisNotSupported => {
                "Not available for the selected chart basis"
            }
            IndicatorUnsupportedReason::SourceNotSupported => {
                "Not available for the selected market"
            }
            IndicatorUnsupportedReason::ResolutionNotSupported => {
                "Not available at the selected timeframe"
            }
            IndicatorUnsupportedReason::IncompleteHistory => "Requires complete historical data",
            IndicatorUnsupportedReason::InsufficientData { .. } => {
                "Not enough data to use this indicator"
            }
            IndicatorUnsupportedReason::MissingRequiredInput { source } => match source {
                IndicatorInputSource::Klines => "Requires candle data",
                IndicatorInputSource::DirectionalVolume => {
                    "Requires directional volume from candles or trade data"
                }
                IndicatorInputSource::Trades => "Enable trade data to use this indicator",
                IndicatorInputSource::OpenInterest => "Requires open-interest data",
            },
            IndicatorUnsupportedReason::MissingDirectionalVolume => {
                "Requires directional volume from candles or trade data"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorAvailability {
    Available,
    PendingProbe,
    Unsupported(IndicatorUnsupportedReason),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorPanelRecipe {
    Plot {
        panel_title: String,
        layer_name: String,
        value_precision: PanelValuePrecision,
        value_label_policy: PanelValueLabelPolicy,
        preferred_scale: PanelScaleMode,
        layers: Vec<IndicatorOutputLayerSpec>,
        readouts: Vec<IndicatorReadoutSpec>,
    },
    Table {
        panel_title: String,
        columns: Vec<TableColumnSpec>,
    },
    PrimaryOverlay {
        layer_name: String,
        layers: Vec<IndicatorOutputLayerSpec>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AvailabilityContext {
    pub basis: Basis,
    pub base_ticker: TickerInfo,
}

#[derive(Debug, Clone)]
pub struct IndicatorSettingsContext {
    pub basis: Basis,
    pub input_coverage: IndicatorInputCoverage,
}

/// A value exposed by an indicator's settings schema.
///
/// Settings use signed integers and double-precision fractional values at the
/// chart boundary. Owned values and metadata let declaration modules expose
/// data whose lifetime is independent of the binary.
#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorSettingValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Color(u32),
    Choice(String),
    Text(String),
    TextList(Vec<String>),
}

impl IndicatorSettingValue {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn choice(value: impl Into<String>) -> Self {
        Self::Choice(value.into())
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Choice(value) | Self::Text(value) => Some(value),
            Self::I64(_) | Self::F64(_) | Self::Bool(_) | Self::Color(_) | Self::TextList(_) => {
                None
            }
        }
    }

    pub fn into_text(self) -> Option<String> {
        match self {
            Self::Choice(value) | Self::Text(value) => Some(value),
            Self::I64(_) | Self::F64(_) | Self::Bool(_) | Self::Color(_) | Self::TextList(_) => {
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorSettingChoice {
    pub id: String,
    pub label: String,
}

impl Display for IndicatorSettingChoice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorSettingControl {
    SliderI64 {
        min: Option<i64>,
        max: Option<i64>,
    },
    SliderF64 {
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
    },
    Toggle,
    Color,
    Text,
    Choices(Vec<IndicatorSettingChoice>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSetting {
    pub id: String,
    pub label: String,
    pub tooltip: Option<String>,
    pub group: Option<String>,
    pub value: IndicatorSettingValue,
    pub control: IndicatorSettingControl,
}
