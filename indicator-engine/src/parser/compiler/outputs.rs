use super::super::ast::{AstCall, AstExpression, constant_value};
use super::context::{LoweringContext, argument_at};
use crate::hir::*;
use crate::scene::*;
use crate::types::*;

impl LoweringContext {
    pub(super) fn compile_plot(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "plot");
        let Some(argument) = argument_at(call, 0, "value") else {
            self.error(call.span, "plot expects a series expression");
            return None;
        };
        let expression = self.compile_expression(&argument.expression);
        let ty = self.ir.program.expressions[expression.0].ty;
        if !matches!(
            ty.value,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(argument.expression.span(), "plot expects a series value");
            return None;
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| match &argument.expression {
                AstExpression::Identifier(path, _) => path.last().cloned(),
                _ => None,
            })
            .unwrap_or_else(|| format!("plot_{}", self.ir.program.plots.len()));
        let plot = crate::hir::PlotId(self.ir.program.plots.len());
        let label = self
            .static_named_string(call, "title")
            .unwrap_or_else(|| "Plot".to_owned());
        let value_type = if matches!(ty.value, ValueType::Bool) {
            PlotValueType::Boolean
        } else {
            PlotValueType::Number
        };
        let mut descriptor = self.plot_descriptor(call, id, label, value_type);
        let structured_constructor = match descriptor.style {
            PlotStyle::Candles => Some(("candles", "plotcandle or plotohlc")),
            PlotStyle::Bars => Some(("bars", "plotcandle or plotohlc")),
            PlotStyle::Cloud => Some(("cloud", "plotrange or broken_cloud")),
            PlotStyle::BrokenCloud => Some(("broken_cloud", "plotrange or broken_cloud")),
            PlotStyle::Line | PlotStyle::Step | PlotStyle::Histogram | PlotStyle::Area => None,
        };
        if let Some((style, constructors)) = structured_constructor {
            self.error(
                call.span,
                format!("plot style '{style}' requires a structured plot; use {constructors}"),
            );
            descriptor.style = PlotStyle::Line;
        }
        self.presentation
            .spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.ir.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Plot {
                plot,
                expression,
                color,
            },
        }))
    }

    pub(super) fn compile_emit(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let Some(id_argument) = argument_at(call, 0, "id") else {
            self.error(
                call.span,
                "emit expects a constant output ID as its first argument",
            );
            return None;
        };
        let Some(Value::String(id)) = constant_value(&id_argument.expression) else {
            self.error(
                id_argument.expression.span(),
                "emit expects a constant output ID as its first argument",
            );
            return None;
        };
        let Some(argument) = argument_at(call, 1, "value") else {
            self.error(call.span, "emit expects a value as its second argument");
            return None;
        };
        let expression = self.compile_expression(&argument.expression);
        let ty = self.ir.program.expressions[expression.0].ty;
        if !matches!(
            ty.value,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(
                argument.expression.span(),
                "emit currently supports numeric and boolean values",
            );
            return None;
        }
        let plot = crate::hir::PlotId(self.ir.program.plots.len());
        let value_type = if matches!(ty.value, ValueType::Bool) {
            PlotValueType::Boolean
        } else {
            PlotValueType::Number
        };
        let descriptor = self.plot_descriptor(call, id, "Output".to_owned(), value_type);
        self.presentation
            .spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.ir.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Plot {
                plot,
                expression,
                color,
            },
        }))
    }

    pub(super) fn compile_marker(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "marker declarations must be top-level");
            return None;
        }
        let value_argument = argument_at(call, 0, "value");
        let Some(value_argument) = value_argument else {
            self.error(call.span, "marker expects a value expression");
            return None;
        };
        let expression = self.compile_expression(&value_argument.expression);
        let value_type = self.ir.program.expressions[expression.0].ty.value;
        if !matches!(
            value_type,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(
                value_argument.expression.span(),
                "marker expects a numeric or boolean value",
            );
            return None;
        }
        let visible = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("visible"))
            .map(|argument| self.compile_expression(&argument.expression));
        if let Some(visible) = visible {
            let visible_type = self.ir.program.expressions[visible.0].ty.value;
            if !matches!(visible_type, ValueType::Bool | ValueType::Any) {
                self.error(call.span, "marker visibility must be boolean");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("marker_{}", self.marker_count()));
        let color = self.compile_color_argument(call);
        let text = self.static_named_string(call, "text");
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Marker {
                id,
                expression,
                visible,
                color,
                text,
            },
        }))
    }

    pub(super) fn compile_segment(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "segment declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 4, "segment");
        self.validate_named_arguments(
            call,
            &[
                "start_value",
                "end_value",
                "start_index",
                "end_index",
                "id",
                "title",
                "label",
                "color_slot",
                "width",
                "visible",
                "color",
                "text",
            ],
        );

        let Some(start_value_argument) = argument_at(call, 0, "start_value") else {
            self.error(call.span, "segment expects a start value expression");
            return None;
        };
        let Some(end_value_argument) = argument_at(call, 1, "end_value") else {
            self.error(call.span, "segment expects an end value expression");
            return None;
        };
        let Some(start_index_argument) = argument_at(call, 2, "start_index") else {
            self.error(call.span, "segment expects a start index expression");
            return None;
        };
        let Some(end_index_argument) = argument_at(call, 3, "end_index") else {
            self.error(call.span, "segment expects an end index expression");
            return None;
        };

        let start_value = self.compile_expression(&start_value_argument.expression);
        let end_value = self.compile_expression(&end_value_argument.expression);
        let start_index = self.compile_expression(&start_index_argument.expression);
        let end_index = self.compile_expression(&end_index_argument.expression);

        for (argument, expression, label) in [
            (start_value_argument, start_value, "start value"),
            (end_value_argument, end_value, "end value"),
        ] {
            if !matches!(
                self.ir.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("segment {label} must be numeric"),
                );
            }
        }
        for (argument, expression, label) in [
            (start_index_argument, start_index, "start index"),
            (end_index_argument, end_index, "end index"),
        ] {
            if !matches!(
                self.ir.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("segment {label} must be an integer"),
                );
            }
        }

        let visible = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("visible"))
            .map(|argument| self.compile_expression(&argument.expression));
        if let Some(visible) = visible
            && !matches!(
                self.ir.program.expressions[visible.0].ty.value,
                ValueType::Bool | ValueType::Any
            )
        {
            self.error(call.span, "segment visibility must be boolean");
        }

        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("segment_{}", self.segment_count()));
        let label = self
            .static_named_string(call, "title")
            .or_else(|| self.static_named_string(call, "label"))
            .unwrap_or_else(|| id.clone());
        let color_slot = self
            .static_named_string(call, "color_slot")
            .unwrap_or_else(|| "main".to_owned());
        let line_width = self.presentation_float(call, "width", 1.0);
        self.presentation
            .spans
            .segments
            .insert(id.clone(), call.span);
        self.presentation.data.segments.push(PresentationSegment {
            id: id.clone(),
            label,
            color_slot,
            line_width,
        });
        let color = self.compile_color_argument(call);
        let text = self.static_named_string(call, "text");
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Segment {
                id,
                start_value,
                end_value,
                start_index,
                end_index,
                visible,
                color,
                text,
            },
        }))
    }

    pub(super) fn compile_region(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if self.function.current.is_some() {
            self.error(
                call.span,
                "region declarations are not valid inside functions",
            );
            return None;
        }
        self.validate_positional_arguments(call, 5, "region");
        self.validate_named_arguments(
            call,
            &[
                "key",
                "upper",
                "lower",
                "left_index",
                "right_index",
                "visible",
                "border_color",
                "fill_color",
                "border_color_slot",
                "fill_color_slot",
                "extend_right",
                "persistent",
                "id",
                "width",
            ],
        );
        let argument = |name: &str, index: usize| argument_at(call, index, name);
        let Some(key_argument) = argument("key", 0) else {
            self.error(call.span, "region expects a key expression");
            return None;
        };
        let Some(upper_argument) = argument("upper", 1) else {
            self.error(call.span, "region expects an upper expression");
            return None;
        };
        let Some(lower_argument) = argument("lower", 2) else {
            self.error(call.span, "region expects a lower expression");
            return None;
        };
        let Some(left_index_argument) = argument("left_index", 3) else {
            self.error(call.span, "region expects a left_index expression");
            return None;
        };
        let Some(right_index_argument) = argument("right_index", 4) else {
            self.error(call.span, "region expects a right_index expression");
            return None;
        };
        let key = self.compile_expression(&key_argument.expression);
        let upper = self.compile_expression(&upper_argument.expression);
        let lower = self.compile_expression(&lower_argument.expression);
        let left_index = self.compile_expression(&left_index_argument.expression);
        let right_index = self.compile_expression(&right_index_argument.expression);
        for (argument, expression, label) in [
            (key_argument, key, "key"),
            (left_index_argument, left_index, "left_index"),
            (right_index_argument, right_index, "right_index"),
        ] {
            if !matches!(
                self.ir.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("region {label} must be an integer"),
                );
            }
        }
        for (argument, expression, label) in [
            (upper_argument, upper, "upper"),
            (lower_argument, lower, "lower"),
        ] {
            if !matches!(
                self.ir.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("region {label} must be numeric"),
                );
            }
        }
        let compile_optional = |compiler: &mut Self, name: &str| {
            call.arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(name))
                .map(|argument| compiler.compile_expression(&argument.expression))
        };
        let visible = compile_optional(self, "visible");
        let extend_right = compile_optional(self, "extend_right");
        for (expression, label) in [(visible, "visibility"), (extend_right, "extend_right")] {
            if let Some(expression) = expression
                && !matches!(
                    self.ir.program.expressions[expression.0].ty.value,
                    ValueType::Bool | ValueType::Any
                )
            {
                self.error(call.span, format!("region {label} must be boolean"));
            }
        }
        let border_color = self.compile_color_named_argument(call, "border_color");
        let fill_color = self.compile_color_named_argument(call, "fill_color");
        let border_color_slot = self.static_named_string(call, "border_color_slot");
        let fill_color_slot = self.static_named_string(call, "fill_color_slot");
        if border_color.is_some() && border_color_slot.is_some() {
            self.error(
                call.span,
                "region cannot provide both border_color and border_color_slot",
            );
        }
        if fill_color.is_some() && fill_color_slot.is_some() {
            self.error(
                call.span,
                "region cannot provide both fill_color and fill_color_slot",
            );
        }
        for slot in [border_color_slot.as_ref(), fill_color_slot.as_ref()]
            .into_iter()
            .flatten()
        {
            if !self.presentation.data.region_style_slots.contains(slot) {
                self.presentation.data.region_style_slots.push(slot.clone());
                self.presentation
                    .spans
                    .region_style_slots
                    .insert(slot.clone(), call.span);
            }
        }
        let id = self
            .static_named_string(call, "id")
            .unwrap_or_else(|| format!("region_{}", self.region_count()));
        let persistent = match self.static_named_value(call, "persistent") {
            Some(Value::Bool(value)) => value,
            Some(_) => {
                self.error(call.span, "region persistent must be a boolean literal");
                false
            }
            None => false,
        };
        let width = self.presentation_float(call, "width", 1.0);
        let valid_width = width.is_finite() && (0.0..=64.0).contains(&width) && width > 0.0;
        if !valid_width {
            self.error(call.span, "region width must be a positive number up to 64");
        }
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Region {
                id,
                key,
                upper,
                lower,
                left_index,
                right_index,
                visible,
                border_color,
                fill_color,
                border_color_slot,
                fill_color_slot,
                extend_right,
                persistent,
                width: if valid_width { width } else { 1.0 },
            },
        }))
    }

    pub(super) fn compile_region_remove(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if self.function.current.is_some() {
            self.error(
                call.span,
                "region_remove declarations are not valid inside functions",
            );
            return None;
        }
        self.validate_positional_arguments(call, 0, "region_remove");
        self.validate_named_arguments(call, &["id", "key"]);
        let Some(id) = self.static_named_string(call, "id") else {
            self.error(call.span, "region_remove expects a static id");
            return None;
        };
        let Some(key_argument) = argument_at(call, 0, "key") else {
            self.error(call.span, "region_remove expects a key expression");
            return None;
        };
        let key = self.compile_expression(&key_argument.expression);
        if !matches!(
            self.ir.program.expressions[key.0].ty.value,
            ValueType::Int | ValueType::Any
        ) {
            self.error(
                key_argument.expression.span(),
                "region_remove key must be an integer",
            );
        }
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::RegionRemove { id, key },
        }))
    }

    pub(super) fn compile_level(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "level declarations must be top-level");
            return None;
        }
        let value_argument = argument_at(call, 0, "value");
        let Some(value_argument) = value_argument else {
            self.error(call.span, "level expects a value expression");
            return None;
        };
        let expression = self.compile_expression(&value_argument.expression);
        let value_type = self.ir.program.expressions[expression.0].ty.value;
        if !matches!(
            value_type,
            ValueType::Int | ValueType::Float | ValueType::Any
        ) {
            self.error(
                value_argument.expression.span(),
                "level expects a numeric value",
            );
            return None;
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("level_{}", self.level_count()));
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::HorizontalLevel {
                id,
                expression,
                color,
            },
        }))
    }

    pub(super) fn compile_ohlc(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let argument = |name: &str, index: usize| argument_at(call, index, name);
        let Some(open) = argument("open", 0) else {
            self.error(
                call.span,
                "plotohlc expects open, high, low, and close expressions",
            );
            return None;
        };
        let Some(high) = argument("high", 1) else {
            self.error(
                call.span,
                "plotohlc expects high, low, and close expressions",
            );
            return None;
        };
        let Some(low) = argument("low", 2) else {
            self.error(call.span, "plotohlc expects low and close expressions");
            return None;
        };
        let Some(close) = argument("close", 3) else {
            self.error(call.span, "plotohlc expects a close expression");
            return None;
        };
        let values = [open, high, low, close]
            .into_iter()
            .map(|argument| self.compile_expression(&argument.expression))
            .collect::<Vec<_>>();
        for (argument, expression) in [open, high, low, close].into_iter().zip(values.iter()) {
            if !matches!(
                self.ir.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(argument.expression.span(), "OHLC values must be numeric");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("plot_{}", self.ir.program.plots.len()));
        let plot = crate::hir::PlotId(self.ir.program.plots.len());
        let mut descriptor = self.plot_descriptor(call, id, "OHLC".to_owned(), PlotValueType::Ohlc);
        descriptor.style = if call.path.join(".") == "plotohlc" {
            PlotStyle::Bars
        } else {
            PlotStyle::Candles
        };
        self.presentation
            .spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.ir.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::OhlcPlot {
                plot,
                open: values[0],
                high: values[1],
                low: values[2],
                close: values[3],
                color,
            },
        }))
    }

    pub(super) fn compile_range(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let upper_argument = argument_at(call, 0, "upper").or_else(|| argument_at(call, 0, "high"));
        let lower_argument = argument_at(call, 1, "lower").or_else(|| argument_at(call, 1, "low"));
        let (Some(upper_argument), Some(lower_argument)) = (upper_argument, lower_argument) else {
            self.error(call.span, "range plot expects upper and lower expressions");
            return None;
        };
        let upper = self.compile_expression(&upper_argument.expression);
        let lower = self.compile_expression(&lower_argument.expression);
        for (argument, expression) in [(upper_argument, upper), (lower_argument, lower)] {
            let value_type = self.ir.program.expressions[expression.0].ty.value;
            if !matches!(
                value_type,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(argument.expression.span(), "range values must be numeric");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("plot_{}", self.ir.program.plots.len()));
        let plot = crate::hir::PlotId(self.ir.program.plots.len());
        let mut descriptor =
            self.plot_descriptor(call, id, "Range".to_owned(), PlotValueType::Range);
        descriptor.style = match call.path.join(".").as_str() {
            "broken_cloud" => PlotStyle::BrokenCloud,
            _ => PlotStyle::Cloud,
        };
        self.presentation
            .spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.ir.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::RangePlot {
                plot,
                upper,
                lower,
                color,
            },
        }))
    }

    pub(super) fn compile_color_argument(&mut self, call: &AstCall) -> Option<ExprId> {
        self.compile_color_named_argument(call, "color")
    }

    pub(super) fn compile_color_named_argument(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<ExprId> {
        let argument = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some(name))?;
        let expression = self.compile_expression(&argument.expression);
        let value_type = self.ir.program.expressions[expression.0].ty.value;
        if !matches!(value_type, ValueType::Color | ValueType::Any) {
            self.error(
                argument.expression.span(),
                format!("{name} must be a color value"),
            );
        }
        Some(expression)
    }
    pub(super) fn plot_descriptor(
        &mut self,
        call: &AstCall,
        id: String,
        label: String,
        value_type: PlotValueType,
    ) -> PlotDescriptor {
        self.validate_named_arguments(
            call,
            &[
                "value",
                "id",
                "title",
                "pane",
                "scale",
                "style",
                "display",
                "precision",
                "width",
                "offset",
                "color_slot",
                "style_by",
                "true_color_slot",
                "false_color_slot",
                "layer_title",
                "overlay",
                "data_kind",
                "mark",
                "axis",
                "layer",
                "color",
            ],
        );
        let default_pane = if matches!(
            self.presentation.data.view,
            PresentationViewKind::PrimaryOverlay
        ) {
            PaneTarget::Main
        } else {
            PaneTarget::Indicator
        };
        let pane = match self.static_named_string(call, "pane").as_deref() {
            Some("main") => PaneTarget::Main,
            Some("indicator") => PaneTarget::Indicator,
            None => default_pane,
            Some(value) if value.strip_prefix("named:").is_some() => value
                .strip_prefix("named:")
                .and_then(|value| value.parse::<u32>().ok())
                .map(PaneTarget::Named)
                .unwrap_or_else(|| {
                    self.error(
                        call.span,
                        "plot pane must be main, indicator, or named:<id>",
                    );
                    default_pane
                }),
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot pane {value}; use main, indicator, or named:<id>"),
                );
                default_pane
            }
        };
        let scale = match self.static_named_string(call, "scale").as_deref() {
            Some("independent") => PlotScale::Independent,
            Some("price") | None => PlotScale::Price,
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot scale {value}; use price or independent"),
                );
                PlotScale::Price
            }
        };
        let style = match self.static_named_string(call, "style").as_deref() {
            Some("line") => PlotStyle::Line,
            Some("step") => PlotStyle::Step,
            Some("histogram") => PlotStyle::Histogram,
            Some("area") => PlotStyle::Area,
            Some("candles") => PlotStyle::Candles,
            Some("bars") => PlotStyle::Bars,
            Some("cloud") => PlotStyle::Cloud,
            Some("broken_cloud") => PlotStyle::BrokenCloud,
            Some(value) => {
                self.error(call.span, format!("unknown plot style {value}"));
                PlotStyle::Line
            }
            None => PlotStyle::Line,
        };
        let explicit_display = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("display"));
        let readout = match self.static_named_string(call, "display").as_deref() {
            Some("none") => ReadoutBehavior::Hidden,
            Some("value") | None => ReadoutBehavior::Value,
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot display mode {value}; use value or none"),
                );
                ReadoutBehavior::Value
            }
        };
        // Boolean series are signals for other declarations (for example
        // `style_by` or a table `column`), not visual output by themselves.
        // Boolean series are hidden unless the source explicitly displays them.
        let readout = if value_type == PlotValueType::Boolean && !explicit_display {
            ReadoutBehavior::Hidden
        } else {
            readout
        };
        let output_precision = match self.static_named_value(call, "precision") {
            Some(Value::String(value)) => {
                let output_precision = if value.eq_ignore_ascii_case("instrument") {
                    Some(OutputPrecision::Instrument)
                } else if let Some(decimals) = value.strip_prefix("decimals:") {
                    decimals
                        .parse::<u8>()
                        .ok()
                        .filter(|decimals| *decimals <= 18)
                        .map(OutputPrecision::Decimals)
                } else if let Some(step) = value.strip_prefix("step:") {
                    step.parse::<f64>()
                        .ok()
                        .filter(|step| step.is_finite() && *step > 0.0)
                        .map(OutputPrecision::Step)
                } else {
                    None
                };
                if output_precision.is_none() {
                    self.error(
                        call.span,
                        "plot precision must be instrument, decimals:<n>, or step:<value>",
                    );
                }
                output_precision
            }
            Some(_) => {
                self.error(
                    call.span,
                    "plot precision must be instrument, decimals:<n>, or step:<value>",
                );
                None
            }
            None => None,
        };
        let width = match self.static_named_value(call, "width") {
            Some(Value::Int(value)) if (0..=64).contains(&value) && value > 0 => value as f32,
            Some(Value::Float(value))
                if value.is_finite() && (0.0..=64.0).contains(&value) && value > 0.0 =>
            {
                value as f32
            }
            Some(_) => {
                self.error(call.span, "plot width must be a positive number up to 64");
                1.0
            }
            None => 1.0,
        };
        let offset_bars = match self.static_named_value(call, "offset") {
            Some(Value::Int(value)) => match i32::try_from(value) {
                Ok(value) => value,
                Err(_) => {
                    self.error(call.span, "plot offset must fit in a 32-bit integer");
                    0
                }
            },
            Some(_) => {
                self.error(call.span, "plot offset must be an integer");
                0
            }
            None => 0,
        };
        PlotDescriptor {
            id,
            label,
            value_type,
            pane,
            scale,
            width,
            output_precision,
            style,
            readout,
            offset_bars,
            color_slot: self.static_named_string(call, "color_slot"),
            style_by: self.static_named_string(call, "style_by"),
            true_color_slot: self.static_named_string(call, "true_color_slot"),
            false_color_slot: self.static_named_string(call, "false_color_slot"),
            layer_title: self.static_named_string(call, "layer_title"),
            overlay: match self.static_named_value(call, "overlay") {
                Some(Value::Bool(value)) => value,
                Some(_) => {
                    self.error(call.span, "plot overlay must be a boolean literal");
                    true
                }
                None => true,
            },
            presentation_data_kind: self.parse_presentation_data_kind(call, "data_kind"),
            presentation_mark: self.parse_presentation_mark(call, "mark"),
            presentation_axis: self.parse_presentation_axis(call, "axis"),
            layer: match self.static_named_value(call, "layer") {
                Some(Value::Bool(value)) => value,
                Some(_) => {
                    self.error(call.span, "plot layer must be a boolean literal");
                    true
                }
                None => true,
            },
        }
    }
}
