use super::context::LoweringContext;
use super::presentation_defaults::{
    default_data_kind_for_plot, default_mark_for_plot, numeric_mark_compatible,
    plot_value_supports_data_kind, presentation_mark_compatible, valid_presentation_role,
    valid_presentation_shade,
};
use crate::diagnostics::Span;
use crate::hir::*;
use crate::scene::*;
use crate::types::*;
use std::collections::{BTreeMap, BTreeSet};

const HISTORY_RETENTION_ERROR: &str = "history indexing requires an immutable retained series; mutable locals, function parameters, and function results cannot be indexed";

impl LoweringContext {
    pub(super) fn validate_inputs(&mut self, descriptor: &IndicatorDescriptor) {
        for input in &descriptor.inputs {
            let span = self
                .declarations
                .input_spans
                .get(&input.id)
                .copied()
                .unwrap_or_else(|| Span::new(0, 0));
            if let (Some(min), Some(max)) = (input.min, input.max)
                && min > max
            {
                self.error(
                    span,
                    format!("input '{}' min must not exceed max", input.id),
                );
            }
            if let Some(step) = input.step
                && (!step.is_finite() || step <= 0.0)
            {
                self.error(span, format!("input '{}' step must be positive", input.id));
            }
            let default = match &input.default {
                InputValue::Int(value) => Some(*value as f64),
                InputValue::Float(value) => Some(*value),
                _ => None,
            };
            if let Some(default) = default {
                if input.min.is_some_and(|min| default < min) {
                    self.error(
                        span,
                        format!("input '{}' default is below its minimum", input.id),
                    );
                }
                if input.max.is_some_and(|max| default > max) {
                    self.error(
                        span,
                        format!("input '{}' default is above its maximum", input.id),
                    );
                }
            }
            if !input.options.is_empty()
                && let InputValue::String(default) = &input.default
                && !input.options.iter().any(|option| option.id == *default)
            {
                self.error(
                    span,
                    format!("input '{}' default is not one of its options", input.id),
                );
            }
        }
    }
    pub(super) fn validate_presentation(&mut self, descriptor: &IndicatorDescriptor) {
        let has_indicator_source_inputs = descriptor
            .series_inputs
            .iter()
            .any(|input| input.allow_indicator);
        if has_indicator_source_inputs && descriptor.presentation.attachment.is_none() {
            self.error(
                self.display_span(),
                "display must declare attachment = \"overlay\" or \"panel\" when input.source allows indicator bindings",
            );
        }
        if has_indicator_source_inputs
            && !matches!(descriptor.presentation.view, PresentationViewKind::Plot)
        {
            self.error(
                self.display_span(),
                "input.source with allow_indicator = true requires display(view = \"plot\")",
            );
        }
        if has_indicator_source_inputs
            && !descriptor
                .plots
                .iter()
                .any(|plot| plot.layer && matches!(plot.value_type, PlotValueType::Number))
        {
            self.error(
                self.display_span(),
                "input.source with allow_indicator = true requires at least one visible numeric plot layer",
            );
        }
        let output_types = descriptor
            .plots
            .iter()
            .map(|plot| (plot.id.as_str(), plot.value_type))
            .collect::<BTreeMap<_, _>>();
        let has_numeric_output = descriptor
            .plots
            .iter()
            .any(|plot| plot.value_type != PlotValueType::Boolean);
        if matches!(descriptor.presentation.view, PresentationViewKind::Plot)
            && has_numeric_output
            && descriptor.presentation.value_precision.is_none()
        {
            self.error(
                self.format_span(),
                "numeric plot indicators must declare format(precision = ...) for panel axes and crosshair snapping",
            );
        }
        if matches!(
            descriptor.presentation.view,
            PresentationViewKind::PrimaryOverlay
        ) {
            for plot in &descriptor.plots {
                if plot.pane != PaneTarget::Main {
                    self.error(
                        self.plot_span(&plot.id),
                        format!(
                            "primary_overlay plot '{}' must use pane main, got {:?}",
                            plot.id, plot.pane
                        ),
                    );
                }
            }
        } else if matches!(descriptor.presentation.view, PresentationViewKind::Plot) {
            let mut layer_pane = None;
            for plot in descriptor
                .plots
                .iter()
                .filter(|plot| plot.layer && !matches!(plot.value_type, PlotValueType::Boolean))
            {
                if let Some(expected) = layer_pane {
                    if expected != plot.pane {
                        self.error(
                            self.plot_span(&plot.id),
                            "all numeric plot layers in one indicator must use the same pane",
                        );
                    }
                } else {
                    layer_pane = Some(plot.pane);
                }
            }
        }
        let style_ids = descriptor
            .presentation
            .styles
            .iter()
            .map(|style| style.id.as_str())
            .collect::<BTreeSet<_>>();
        for (index, plot) in descriptor.plots.iter().enumerate() {
            if plot.id.trim().is_empty() {
                self.error(self.plot_span(&plot.id), "plot ID cannot be empty");
            }
            if plot.label.trim().is_empty() {
                self.error(self.plot_span(&plot.id), "plot title cannot be empty");
            }
            if descriptor.plots[..index]
                .iter()
                .any(|previous| previous.id == plot.id)
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!("duplicate plot ID '{}'; output IDs must be unique", plot.id),
                );
            }
            for slot in [
                plot.color_slot.as_deref(),
                plot.true_color_slot.as_deref(),
                plot.false_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.plot_span(&plot.id),
                        format!("plot '{}' references unknown style slot '{slot}'", plot.id),
                    );
                }
            }
            if let Some(style_by) = &plot.style_by
                && output_types.get(style_by.as_str()) != Some(&PlotValueType::Boolean)
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' style_by must reference a boolean output",
                        plot.id
                    ),
                );
            }
            let data_kind = plot
                .presentation_data_kind
                .unwrap_or_else(|| default_data_kind_for_plot(plot));
            if !plot_value_supports_data_kind(plot.value_type, data_kind) {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' value type is incompatible with presentation data kind",
                        plot.id
                    ),
                );
            }
            let mark = plot
                .presentation_mark
                .unwrap_or_else(|| default_mark_for_plot(plot));
            if !presentation_mark_compatible(data_kind, mark) {
                self.error(
                    self.plot_span(&plot.id),
                    format!("plot '{}' mark is incompatible with its data kind", plot.id),
                );
            }
            if plot.value_type != PlotValueType::Number && plot.style_by.is_some() {
                self.error(
                    self.plot_span(&plot.id),
                    format!("plot '{}' style_by requires a numeric output", plot.id),
                );
            }
            if plot.true_color_slot.is_some() != plot.false_color_slot.is_some() {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' must provide both true_color_slot and false_color_slot",
                        plot.id
                    ),
                );
            }
            if plot.style_by.is_some()
                && (plot.true_color_slot.is_none() || plot.false_color_slot.is_none())
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' style_by requires both true_color_slot and false_color_slot",
                        plot.id
                    ),
                );
            }
        }
        for (index, style) in descriptor.presentation.styles.iter().enumerate() {
            if style.id.trim().is_empty() {
                self.error(self.style_span(&style.id), "style ID cannot be empty");
            }
            if !valid_presentation_role(&style.role) {
                self.error(
                    self.style_span(&style.id),
                    format!("unknown style role '{}'", style.role),
                );
            }
            if !valid_presentation_shade(&style.shade) {
                self.error(
                    self.style_span(&style.id),
                    format!("unknown style shade '{}'", style.shade),
                );
            }
            if descriptor.presentation.styles[..index]
                .iter()
                .any(|previous| previous.id == style.id)
            {
                self.error(
                    self.style_span(&style.id),
                    format!("duplicate style ID '{}'", style.id),
                );
            }
        }
        for slot in &descriptor.presentation.region_style_slots {
            if !style_ids.contains(slot.as_str()) {
                let span = self
                    .presentation
                    .spans
                    .region_style_slots
                    .get(slot)
                    .copied()
                    .unwrap_or(Span::new(0, 0));
                self.error(
                    span,
                    format!("region references unknown style slot '{slot}'"),
                );
            }
        }
        for (layer_index, layer) in descriptor.presentation.layers.iter().enumerate() {
            if layer.channel.trim().is_empty() {
                self.error(
                    self.plot_span(&layer.channel),
                    "layer output ID cannot be empty",
                );
            }
            if layer.name.trim().is_empty() {
                self.error(
                    self.plot_span(&layer.channel),
                    "layer title cannot be empty",
                );
            }
            if descriptor.presentation.layers[..layer_index]
                .iter()
                .any(|previous| previous.channel == layer.channel)
            {
                self.error(
                    self.plot_span(&layer.channel),
                    format!(
                        "duplicate presentation layer for output '{}'",
                        layer.channel
                    ),
                );
            }
            if !output_types.contains_key(layer.channel.as_str()) {
                self.error(
                    self.plot_span(&layer.channel),
                    format!("layer references unknown output '{}'", layer.channel),
                );
                continue;
            }
            if let Some(plot) = descriptor
                .plots
                .iter()
                .find(|plot| plot.id == layer.channel)
            {
                if plot.value_type == PlotValueType::Boolean {
                    self.error(
                        self.plot_span(&layer.channel),
                        format!(
                            "layer '{}' cannot reference a boolean output; use a table column or output-only plot",
                            layer.channel
                        ),
                    );
                    continue;
                }
                let data_kind = layer.data_kind.unwrap_or_else(|| {
                    plot.presentation_data_kind
                        .unwrap_or_else(|| default_data_kind_for_plot(plot))
                });
                let mark = layer.mark.unwrap_or_else(|| {
                    plot.presentation_mark
                        .unwrap_or_else(|| default_mark_for_plot(plot))
                });
                if !plot_value_supports_data_kind(plot.value_type, data_kind)
                    || !presentation_mark_compatible(data_kind, mark)
                {
                    self.error(
                        self.plot_span(&layer.channel),
                        format!("layer '{}' is incompatible with its output", layer.channel),
                    );
                }
            }
        }
        let mut mark_keys = BTreeSet::new();
        for (mark_index, mark) in descriptor.presentation.marks.iter().enumerate() {
            if !mark_keys.insert((mark.source_channel.clone(), mark.kind)) {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "duplicate mark for output '{}' and kind {:?}",
                        mark.source_channel, mark.kind
                    ),
                );
            }
            let Some(source_plot) = descriptor
                .plots
                .iter()
                .find(|plot| plot.id == mark.source_channel)
            else {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' references unknown output '{}'",
                        mark.label, mark.source_channel
                    ),
                );
                continue;
            };
            if source_plot.value_type != PlotValueType::Number {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' source must reference a numeric output",
                        mark.label
                    ),
                );
            } else if !numeric_mark_compatible(mark.kind) {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' uses a rendering incompatible with a numeric output",
                        mark.label
                    ),
                );
            }
            if mark.label.trim().is_empty() {
                self.error(self.mark_span(mark_index), "mark title cannot be empty");
            }
            for slot in [
                Some(mark.color_slot.as_str()),
                mark.true_color_slot.as_deref(),
                mark.false_color_slot.as_deref(),
                mark.highlight_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' references unknown style slot '{slot}'",
                            mark.label
                        ),
                    );
                }
            }
            if mark.color_slot.trim().is_empty() {
                self.error(
                    self.mark_span(mark_index),
                    format!("mark '{}' requires a color_slot", mark.label),
                );
            }
            if mark.true_color_slot.is_some() != mark.false_color_slot.is_some() {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' must provide both true_color_slot and false_color_slot",
                        mark.label
                    ),
                );
            }
            if mark.highlight_by.is_some() != mark.highlight_color_slot.is_some() {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' highlight_by requires highlight_color_slot",
                        mark.label
                    ),
                );
            }
            if let Some(highlight_by) = &mark.highlight_by {
                if output_types.get(highlight_by.as_str()) != Some(&PlotValueType::Boolean) {
                    self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' highlight_by must reference a boolean output",
                            mark.label
                        ),
                    );
                }
                if !matches!(
                    mark.kind,
                    PresentationMarkKind::RegularBar
                        | PresentationMarkKind::Histogram
                        | PresentationMarkKind::HistogramSignedOverlay
                ) {
                    self.error(
                        self.mark_span(mark_index),
                        format!("mark '{}' highlight_by requires a bar mark", mark.label),
                    );
                }
            }

            if let Some(fallback_mark) = mark.fallback_mark {
                if fallback_mark == mark.kind {
                    self.error(
                        self.mark_span(mark_index),
                        format!("mark '{}' cannot fall back to itself", mark.label),
                    );
                }

                let fallback_candidates = descriptor
                    .presentation
                    .marks
                    .iter()
                    .filter(|candidate| candidate.kind == fallback_mark)
                    .collect::<Vec<_>>();
                match fallback_candidates.as_slice() {
                    [] => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' references unknown fallback mark {:?}",
                            mark.label, fallback_mark
                        ),
                    ),
                    [fallback] if fallback.required_input.is_some() => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' fallback mark {:?} cannot require input",
                            mark.label, fallback_mark
                        ),
                    ),
                    [_] => {}
                    _ => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' fallback mark {:?} is ambiguous",
                            mark.label, fallback_mark
                        ),
                    ),
                }
            }
        }
        for (index, segment) in descriptor.presentation.segments.iter().enumerate() {
            if segment.id.trim().is_empty() {
                self.error(self.segment_span(&segment.id), "segment ID cannot be empty");
            }
            if segment.label.trim().is_empty() {
                self.error(
                    self.segment_span(&segment.id),
                    "segment title cannot be empty",
                );
            }
            if !segment.line_width.is_finite() || segment.line_width <= 0.0 {
                self.error(
                    self.segment_span(&segment.id),
                    "segment width must be positive",
                );
            }
            if descriptor.presentation.segments[..index]
                .iter()
                .any(|previous| previous.id == segment.id)
            {
                self.error(
                    self.segment_span(&segment.id),
                    format!("duplicate segment ID '{}'", segment.id),
                );
            }
            if !style_ids.contains(segment.color_slot.as_str()) {
                self.error(
                    self.segment_span(&segment.id),
                    format!(
                        "segment '{}' references unknown style slot '{}'",
                        segment.id, segment.color_slot
                    ),
                );
            }
        }
        let mut fill_keys = BTreeSet::new();
        for (fill_index, fill) in descriptor.presentation.fills.iter().enumerate() {
            if fill.upper == fill.lower {
                self.error(
                    self.fill_span(fill_index),
                    format!(
                        "fill upper and lower outputs must differ; '{}' cannot reference itself",
                        fill.upper
                    ),
                );
            }
            if !fill_keys.insert((
                fill.upper.clone(),
                fill.lower.clone(),
                fill.geometry,
                fill.upper_offset_bars,
                fill.lower_offset_bars,
            )) {
                self.error(
                    self.fill_span(fill_index),
                    format!(
                        "duplicate fill for outputs '{}' and '{}'",
                        fill.upper, fill.lower
                    ),
                );
            }
            for channel in [&fill.upper, &fill.lower] {
                if output_types.get(channel.as_str()) != Some(&PlotValueType::Number) {
                    self.error(
                        self.fill_span(fill_index),
                        format!("fill references unknown or non-numeric output '{channel}'"),
                    );
                }
            }
            if !fill.opacity.is_finite() || !(0.0..=1.0).contains(&fill.opacity) {
                self.error(
                    self.fill_span(fill_index),
                    "fill opacity must be between 0 and 1",
                );
            }
            for slot in [
                Some(fill.color_slot.as_str()),
                fill.true_color_slot.as_deref(),
                fill.false_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.fill_span(fill_index),
                        format!("fill references unknown style slot '{slot}'"),
                    );
                }
            }
            if fill.true_color_slot.is_some() != fill.false_color_slot.is_some() {
                self.error(
                    self.fill_span(fill_index),
                    "fill must provide both true_color_slot and false_color_slot",
                );
            }
            if let Some(style_by) = &fill.style_by
                && output_types.get(style_by.as_str()) != Some(&PlotValueType::Boolean)
            {
                self.error(
                    self.fill_span(fill_index),
                    "fill style_by must reference a boolean output",
                );
            }
            if fill.style_by.is_some()
                && (fill.true_color_slot.is_none() || fill.false_color_slot.is_none())
            {
                self.error(
                    self.fill_span(fill_index),
                    "fill style_by requires both true_color_slot and false_color_slot",
                );
            }
        }
        let mut readout_keys = BTreeSet::new();
        for (readout_index, readout) in descriptor.presentation.readouts.iter().enumerate() {
            if !readout_keys.insert((readout.channel.clone(), readout.mark)) {
                self.error(
                    self.readout_span(readout_index),
                    format!("duplicate readout for output '{}'", readout.channel),
                );
            }
            if readout.label.trim().is_empty() {
                self.error(
                    self.readout_span(readout_index),
                    "readout title cannot be empty",
                );
            }
            if output_types.get(readout.channel.as_str()) != Some(&PlotValueType::Number) {
                self.error(
                    self.readout_span(readout_index),
                    format!(
                        "readout references unknown or non-numeric output '{}'",
                        readout.channel
                    ),
                );
            }
            if !style_ids.contains(readout.color_slot.as_str()) {
                self.error(
                    self.readout_span(readout_index),
                    format!(
                        "readout references unknown style slot '{}'",
                        readout.color_slot
                    ),
                );
            }
            if let Some(mark) = readout.mark
                && !numeric_mark_compatible(mark)
            {
                self.error(
                    self.readout_span(readout_index),
                    "readout mark is incompatible with a numeric output",
                );
            }
        }
        let mut column_channels = BTreeSet::new();
        for (column_index, column) in descriptor.presentation.columns.iter().enumerate() {
            if !column_channels.insert(column.channel.clone()) {
                self.error(
                    self.column_span(column_index),
                    format!("duplicate table column for output '{}'", column.channel),
                );
            }
            if column.label.trim().is_empty() {
                self.error(
                    self.column_span(column_index),
                    "table column title cannot be empty",
                );
            }
            let Some(value_type) = output_types.get(column.channel.as_str()).copied() else {
                self.error(
                    self.column_span(column_index),
                    format!("column references unknown output '{}'", column.channel),
                );
                continue;
            };
            let format = column.format.to_ascii_lowercase();
            let compatible = match format.as_str() {
                "boolean" => value_type == PlotValueType::Boolean,
                "quantity" | "integer" | "percent" => value_type == PlotValueType::Number,
                _ => {
                    self.error(
                        self.column_span(column_index),
                        format!("unknown table column format '{}'", column.format),
                    );
                    false
                }
            };
            if !compatible {
                self.error(
                    self.column_span(column_index),
                    format!("table column '{}' has an incompatible output", column.label),
                );
            }
            let validate_column_style_slot = |slot: &str, compiler: &mut Self| {
                if !style_ids.contains(slot) {
                    compiler.error(
                        compiler.column_span(column_index),
                        format!("column references unknown style slot '{slot}'"),
                    );
                }
            };
            match &column.color {
                PresentationColumnColor::Static { slot } => {
                    validate_column_style_slot(slot, self);
                }
                PresentationColumnColor::Signed {
                    positive_slot,
                    negative_slot,
                } => {
                    validate_column_style_slot(positive_slot, self);
                    validate_column_style_slot(negative_slot, self);
                    if value_type == PlotValueType::Boolean {
                        self.error(
                            self.column_span(column_index),
                            "signed table column colors require a numeric output",
                        );
                    }
                }
            }
        }
        if !descriptor.presentation.columns.is_empty()
            && !matches!(descriptor.presentation.view, PresentationViewKind::Table)
        {
            self.error(
                self.column_span(0),
                "table column declarations require display(view = \"table\")",
            );
        }
        if matches!(descriptor.presentation.view, PresentationViewKind::Table)
            && descriptor.presentation.columns.is_empty()
        {
            self.error(
                self.display_span(),
                "display(view = \"table\") requires at least one column declaration",
            );
        }
        if matches!(descriptor.presentation.view, PresentationViewKind::Plot)
            && descriptor.presentation.layers.is_empty()
            && descriptor
                .plots
                .iter()
                .any(|plot| plot.value_type != PlotValueType::Boolean)
        {
            self.error(
                descriptor
                    .plots
                    .first()
                    .map_or_else(|| self.indicator_span(), |plot| self.plot_span(&plot.id)),
                "indicator presentation could not derive any output layers",
            );
        }
        if matches!(
            descriptor.presentation.view,
            PresentationViewKind::PrimaryOverlay
        ) && descriptor.presentation.layers.is_empty()
        {
            self.error(
                self.display_span(),
                "primary_overlay view requires at least one numeric output layer",
            );
        }
    }

    pub(super) fn plot_span(&self, id: &str) -> Span {
        self.presentation
            .spans
            .plots
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn indicator_span(&self) -> Span {
        self.presentation
            .spans
            .indicator
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn display_span(&self) -> Span {
        self.presentation
            .spans
            .display
            .unwrap_or_else(|| self.indicator_span())
    }

    pub(super) fn format_span(&self) -> Span {
        self.presentation
            .spans
            .format
            .unwrap_or_else(|| self.indicator_span())
    }

    pub(super) fn style_span(&self, id: &str) -> Span {
        self.presentation
            .spans
            .styles
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn mark_span(&self, index: usize) -> Span {
        self.presentation
            .spans
            .marks
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn segment_span(&self, id: &str) -> Span {
        self.presentation
            .spans
            .segments
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn fill_span(&self, index: usize) -> Span {
        self.presentation
            .spans
            .fills
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn readout_span(&self, index: usize) -> Span {
        self.presentation
            .spans
            .readouts
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    pub(super) fn column_span(&self, index: usize) -> Span {
        self.presentation
            .spans
            .columns
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }
    pub(super) fn reject_recursive_functions(&mut self) {
        let function_ids = self
            .ir
            .program
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        for function in function_ids {
            let mut path = Vec::new();
            let mut visited = vec![false; self.ir.program.functions.len()];
            if self.function_reaches(function, function, &mut path, &mut visited) {
                self.error(
                    Span::new(0, 0),
                    format!(
                        "recursive function cycle involving {} is not supported in language version 1",
                        self.ir.program
                            .functions
                            .get(function.0)
                            .map(|function| function.name.as_str())
                            .unwrap_or("function")
                    ),
                );
            }
        }
    }

    pub(super) fn function_reaches(
        &self,
        start: FunctionId,
        current: FunctionId,
        path: &mut Vec<FunctionId>,
        visited: &mut [bool],
    ) -> bool {
        let Some(seen) = visited.get_mut(current.0) else {
            return false;
        };
        if *seen {
            return false;
        }
        *seen = true;
        path.push(current);
        let reaches = self
            .function
            .call_edges
            .get(current.0)
            .is_some_and(|edges| {
                edges.iter().any(|next| {
                    (path.len() > 1 && *next == start)
                        || self.function_reaches(start, *next, path, visited)
                })
            });
        path.pop();
        reaches
    }
    pub(super) fn validate_history_sources(&mut self) {
        let invalid_spans = self
            .ir
            .history_expressions
            .iter()
            .filter_map(|(history, span)| {
                let Some(ExpressionKind::History { source, .. }) = self
                    .ir
                    .program
                    .expressions
                    .get(history.0)
                    .map(|expression| &expression.kind)
                else {
                    return None;
                };
                (!self.history_source_is_retained(*source)).then_some(*span)
            })
            .collect::<Vec<_>>();
        for span in invalid_spans {
            self.error(span, HISTORY_RETENTION_ERROR);
        }
    }

    pub(super) fn history_source_is_retained(&self, source: ExprId) -> bool {
        let Some(expression) = self.ir.program.expressions.get(source.0) else {
            return false;
        };
        let is_mutable_local = matches!(
            &expression.kind,
            ExpressionKind::LoadLocal(slot)
                if self
                    .scope
                    .mutable_values
                    .get(slot.0)
                    .copied()
                    .unwrap_or(false)
        );
        expression.series_slot.is_some()
            && !matches!(&expression.kind, ExpressionKind::FunctionCall { .. })
            && !is_mutable_local
    }
}
