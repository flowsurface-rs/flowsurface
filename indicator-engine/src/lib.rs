mod diagnostics;
mod frame;
mod hir;
mod intrinsic;
mod parser;
mod runtime;
mod scene;
mod types;
mod warmup;

pub use diagnostics::{Diagnostic, Severity, Span};
pub use frame::{
    BaseSeriesSource, ChartFrame, FrameField, FrameOutput, HorizontalLevelOutput, IndicatorFrame,
    MarkerOutput, OhlcOutput, PlotOutput, RangeOutput, SegmentOutput, SeriesInputValue, TimeOrTick,
    TradeAggregate,
};
pub use hir::{
    BarStateField, BinaryOperator, CallSiteId, ConstantId, ExprId, ExpressionKind, FunctionId,
    HirFunction, HirProgram, IntrinsicId, LocalSlot, PlotId, SeriesSlot, SourceInputSlot,
    StateSlot, StatementBlock, StatementBlockId, StatementId, StatementKind, TypedExpression,
    TypedStatement, UnaryOperator,
};
use intrinsic::IntrinsicRegistry;
pub use runtime::{IndicatorInstance, PlanningError, RuntimeError, RuntimeLimits};
pub use scene::{
    Color, IndicatorPresentation, OutputPrecision, PaneTarget, PlotDescriptor, PlotScale,
    PlotStyle, PlotValueType, PresentationAttachmentMode, PresentationAxis, PresentationColumn,
    PresentationColumnColor, PresentationDataKind, PresentationFill, PresentationGeometry,
    PresentationInputSource, PresentationLabelMode, PresentationLayer, PresentationMark,
    PresentationMarkKind, PresentationPrecision, PresentationReadout, PresentationScale,
    PresentationSegment, PresentationStyle, PresentationViewKind, ReadoutBehavior,
};
pub use types::{
    IndicatorConfig, IndicatorDescriptor, IndicatorId, InputDescriptor, InputOption, InputType,
    InputValue, Qualifier, SeriesInputDescriptor, Type, Value, ValueType,
};

pub type Program = HirProgram;

/// Host data activated by a compiled indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataRequirements {
    pub ohlc: bool,
    pub directional_volume: bool,
    pub trades: bool,
    pub open_interest: bool,
}

impl DataRequirements {
    pub const fn needs_directional_volume(self) -> bool {
        self.directional_volume
    }

    pub const fn needs_open_interest(self) -> bool {
        self.open_interest
    }
}

use std::path::Path;
use std::sync::Arc;

/// A compiled source program and the metadata discovered from that source.
#[derive(Debug, Clone)]
pub struct CompiledIndicator {
    pub descriptor: IndicatorDescriptor,
    pub requirements: DataRequirements,
    pub program: Program,
    pub(crate) registry: intrinsic::IntrinsicRegistry,
}

/// Parse and type-check one source-owned indicator file.
pub fn compile(source: &str, path: impl AsRef<Path>) -> Result<CompiledIndicator, Vec<Diagnostic>> {
    compile_with_registry(source, path, IntrinsicRegistry::standard())
}

/// Read and compile one indicator source file.
pub fn compile_file(path: impl AsRef<Path>) -> Result<CompiledIndicator, Vec<Diagnostic>> {
    let path = path.as_ref();
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            return Err(vec![Diagnostic::error(
                path,
                Span::new(0, 0),
                format!("could not read source: {error}"),
            )]);
        }
    };
    compile(&source, path)
}

pub(crate) fn compile_with_registry(
    source: &str,
    path: impl AsRef<Path>,
    registry: intrinsic::IntrinsicRegistry,
) -> Result<CompiledIndicator, Vec<Diagnostic>> {
    parser::compile_with_registry(source, path.as_ref(), registry)
}

impl CompiledIndicator {
    pub(crate) fn plan(
        &self,
        config: IndicatorConfig,
    ) -> Result<runtime::InstancePlan, PlanningError> {
        runtime::InstancePlan::from_compiled(self, config)
    }

    /// Static bars of history the plotted values need before they are
    /// meaningful, resolved from constants, inputs, and windowed intrinsics.
    pub fn warmup_bars(&self, config: &IndicatorConfig) -> Option<usize> {
        self.warmup_bars_with_limits(config, RuntimeLimits::default())
    }

    /// Resolve warmup using the same rolling-window policy as the runtime.
    pub fn warmup_bars_with_limits(
        &self,
        config: &IndicatorConfig,
        limits: RuntimeLimits,
    ) -> Option<usize> {
        warmup::warmup_bars(
            &self.program,
            &self.registry,
            &self.descriptor,
            config,
            limits.max_window_length,
        )
    }

    /// Earliest UTC boundary an anchored intrinsic needs data from, resolved
    /// from the configuration. Returns `None` when the indicator has no
    /// anchored calculations such as `ta.avwap`.
    pub fn anchor_start(&self, config: &IndicatorConfig, reference_time: i64) -> Option<i64> {
        warmup::anchor_start(
            &self.program,
            &self.registry,
            &self.descriptor,
            config,
            reference_time,
        )
    }

    /// Create an instance that evaluates only the frames supplied by the chart.
    pub fn instantiate(&self, config: IndicatorConfig) -> Result<IndicatorInstance, PlanningError> {
        self.instantiate_with_limits(config, RuntimeLimits::default())
    }

    pub fn instantiate_with_limits(
        &self,
        config: IndicatorConfig,
        limits: RuntimeLimits,
    ) -> Result<IndicatorInstance, PlanningError> {
        let plan = self.plan(config)?;
        Ok(IndicatorInstance::with_limits(
            Arc::new(self.clone()),
            plan,
            limits,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(index: usize, time: i64, close: f64) -> IndicatorFrame {
        IndicatorFrame {
            index,
            time,
            basis: TimeOrTick::Time,
            chart: ChartFrame {
                open: Some(close),
                high: Some(close),
                low: Some(close),
                close: Some(close),
                volume: Some(1.0),
                directional_volume_available: false,
                buy_qty: 0.0,
                sell_qty: 0.0,
                trade: None,
            },
            open_interest: None,
            series_inputs: Vec::new().into(),
        }
    }

    fn plot_value<'a>(output: &'a FrameOutput, id: &str) -> &'a Value {
        output
            .plots
            .iter()
            .find(|plot| plot.id == id)
            .map(|plot| &plot.value)
            .expect("plot output should be present")
    }

    #[test]
    fn warmup_produces_historical_values() {
        let source = r#"
            language(1)
            indicator("Simple average", id = "test.sma")
            format(precision = "base_ticker_min_tick")
            length = input.int(2, "Length", min = 1, max = 100)
            plot(ta.sma(chart.close, length), id = "sma")
        "#;
        let compiled = compile(source, "sma.ind").expect("source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(2));
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(0, 1, 10.0))
                    .expect("first frame should evaluate"),
                "sma"
            ),
            &Value::Na
        );
        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(1, 2, 20.0))
                    .expect("second frame should evaluate"),
                "sma"
            ),
            &Value::Float(15.0)
        );
    }

    #[test]
    fn history_indexing_reads_an_immutable_series_local() {
        let source = r#"
            language(1)
            indicator("Lagged value", id = "test.lagged")
            format(precision = "base_ticker_min_tick")
            value = chart.close * 2
            plot(value[1], id = "lagged")
        "#;
        let compiled = compile(source, "lagged.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(0, 1, 10.0))
                    .expect("first frame should evaluate"),
                "lagged"
            ),
            &Value::Na
        );
        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(1, 2, 20.0))
                    .expect("second frame should evaluate"),
                "lagged"
            ),
            &Value::Float(20.0)
        );
    }

    #[test]
    fn source_inputs_support_history_ta_and_host_overrides() {
        let source = r#"
            language(1)
            indicator("Source input", id = "test.source-input")
            display(attachment = "panel")
            format(precision = "base_ticker_min_tick")
            source = input.source(chart.close, "Source")
            plot(source, id = "current")
            plot(source[1], id = "lagged")
            plot(ta.sma(source, 2), id = "average")
        "#;
        let compiled = compile(source, "source-input.ind").expect("source input should compile");
        assert!(compiled.descriptor.sourceable);
        assert_eq!(compiled.descriptor.series_inputs.len(), 1);
        assert_eq!(compiled.descriptor.series_inputs[0].id, "source");
        assert!(compiled.descriptor.series_inputs[0].allow_indicator);
        assert_eq!(
            compiled.descriptor.presentation.attachment,
            Some(PresentationAttachmentMode::Panel)
        );
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(2));
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("source input should bind");

        let first = instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("first frame should evaluate");
        assert_eq!(plot_value(&first, "current"), &Value::Float(10.0));
        assert_eq!(plot_value(&first, "lagged"), &Value::Na);
        assert_eq!(plot_value(&first, "average"), &Value::Na);

        let second = instance
            .evaluate_closed(frame(1, 2, 20.0))
            .expect("fallback frame should evaluate");
        assert_eq!(plot_value(&second, "current"), &Value::Float(20.0));
        assert_eq!(plot_value(&second, "lagged"), &Value::Float(10.0));
        assert_eq!(plot_value(&second, "average"), &Value::Float(15.0));

        let mut bound = frame(2, 3, 30.0);
        bound.series_inputs = vec![SeriesInputValue::Value(Some(100.0))].into();
        let bound = instance
            .evaluate_closed(bound)
            .expect("bound frame should evaluate");
        assert_eq!(plot_value(&bound, "current"), &Value::Float(100.0));
        assert_eq!(plot_value(&bound, "lagged"), &Value::Float(20.0));
        assert_eq!(plot_value(&bound, "average"), &Value::Float(60.0));

        let mut unavailable = frame(3, 4, 40.0);
        unavailable.series_inputs = vec![SeriesInputValue::Value(None)].into();
        let unavailable = instance
            .evaluate_closed(unavailable)
            .expect("unavailable frame should evaluate");
        assert_eq!(plot_value(&unavailable, "current"), &Value::Na);
        assert_eq!(plot_value(&unavailable, "lagged"), &Value::Float(100.0));
        assert_eq!(plot_value(&unavailable, "average"), &Value::Na);

        let mut selected = frame(4, 5, 50.0);
        selected.chart.high = Some(9.0);
        selected.chart.low = Some(3.0);
        selected.series_inputs = vec![SeriesInputValue::Base(BaseSeriesSource::Hl2)].into();
        let selected = instance
            .evaluate_closed(selected)
            .expect("base source selection should evaluate");
        assert_eq!(plot_value(&selected, "current"), &Value::Float(6.0));
    }

    #[test]
    fn chart_composite_sources_compile_and_evaluate() {
        let source = r#"
            language(1)
            indicator("Composite sources", id = "test.composite-sources")
            display(attachment = "panel")
            format(precision = "base_ticker_min_tick")
            source = input.source(chart.hlc3, "Source")
            plot(chart.hl2, id = "hl2")
            plot(chart.hlc3, id = "hlc3")
            plot(chart.ohlc4, id = "ohlc4")
            plot(chart.hlcc4, id = "hlcc4")
            plot(source, id = "source")
        "#;
        let compiled = compile(source, "composite-sources.ind")
            .expect("composite chart sources should compile");
        assert!(compiled.requirements.ohlc);
        assert_eq!(
            compiled.descriptor.series_inputs[0].default_field,
            FrameField::ChartHlc3
        );

        let mut complete = frame(0, 1, 5.0);
        complete.chart.open = Some(1.0);
        complete.chart.high = Some(8.0);
        complete.chart.low = Some(3.0);
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("composite source instance should bind");
        let output = instance
            .evaluate_closed(complete)
            .expect("complete composite frame should evaluate");
        assert_eq!(plot_value(&output, "hl2"), &Value::Float(5.5));
        assert_eq!(plot_value(&output, "hlc3"), &Value::Float(16.0 / 3.0));
        assert_eq!(plot_value(&output, "ohlc4"), &Value::Float(17.0 / 4.0));
        assert_eq!(plot_value(&output, "hlcc4"), &Value::Float(21.0 / 4.0));
        assert_eq!(plot_value(&output, "source"), &Value::Float(16.0 / 3.0));

        let mut incomplete = frame(1, 2, 5.0);
        incomplete.chart.high = Some(8.0);
        incomplete.chart.low = None;
        let output = instance
            .evaluate_closed(incomplete)
            .expect("incomplete composite frame should evaluate");
        assert_eq!(plot_value(&output, "hl2"), &Value::Na);
        assert_eq!(plot_value(&output, "hlc3"), &Value::Na);
        assert_eq!(plot_value(&output, "ohlc4"), &Value::Na);
        assert_eq!(plot_value(&output, "hlcc4"), &Value::Na);
        assert_eq!(plot_value(&output, "source"), &Value::Na);

        let mut nonfinite = frame(2, 3, 5.0);
        nonfinite.chart.high = Some(f64::NAN);
        let output = instance
            .evaluate_closed(nonfinite)
            .expect("non-finite composite frame should evaluate");
        assert_eq!(plot_value(&output, "source"), &Value::Na);
    }

    #[test]
    fn source_input_requires_a_direct_chart_series_default() {
        let diagnostics = compile(
            r#"
                language(1)
                indicator("Invalid source", id = "test.invalid-source")
                display(attachment = "panel")
                format(precision = "base_ticker_min_tick")
                source = input.source(chart.close + 1, "Source")
                plot(source, id = "value")
            "#,
            "invalid-source.ind",
        )
        .expect_err("source defaults must be direct chart fields");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("input.source default must be chart.open")
        }));
    }

    #[test]
    fn source_policies_map_and_allow_multiple_engine_inputs() {
        let policy = compile(
            r#"
                language(1)
                indicator("Policies", id = "test.policies", sourceable = false)
                display(attachment = "panel")
                format(precision = "base_ticker_min_tick")
                base = input.source(chart.close, "Base", allow_indicator = false)
                source = input.source(chart.close, "Source")
                plot(base + source, id = "value")
            "#,
            "policies.ind",
        )
        .expect("source policies should compile");
        assert!(!policy.descriptor.sourceable);
        assert_eq!(policy.descriptor.series_inputs.len(), 2);
        assert!(!policy.descriptor.series_inputs[0].allow_indicator);
        assert!(policy.descriptor.series_inputs[1].allow_indicator);

        let multiple = compile(
            r#"
                language(1)
                indicator("Multiple sources", id = "test.multiple-sources")
                display(attachment = "panel")
                format(precision = "base_ticker_min_tick")
                left = input.source(chart.close, "Left")
                right = input.source(chart.open, "Right")
                plot(left - right, id = "value")
            "#,
            "multiple-sources.ind",
        )
        .expect("multiple source inputs should compile");
        assert_eq!(multiple.descriptor.series_inputs.len(), 2);
        assert!(
            multiple
                .descriptor
                .series_inputs
                .iter()
                .all(|input| input.allow_indicator)
        );
    }

    #[test]
    fn base_only_source_inputs_can_use_non_plot_views() {
        let table = compile(
            r#"
                language(1)
                indicator("Base table", id = "test.base-table")
                display(view = "table")
                source = input.source(chart.close, "Source", allow_indicator = false)
                plot(source, id = "value", layer = false)
                column(title = "Value", source = "value")
            "#,
            "base-table.ind",
        )
        .expect("base-only table source should compile");
        assert_eq!(
            table.descriptor.presentation.view,
            PresentationViewKind::Table
        );

        let overlay = compile(
            r#"
                language(1)
                indicator("Base overlay", id = "test.base-overlay")
                display(view = "primary_overlay")
                source = input.source(chart.close, "Source", allow_indicator = false)
                plot(source, id = "value", pane = "main")
            "#,
            "base-overlay.ind",
        )
        .expect("base-only primary overlay source should compile");
        assert_eq!(
            overlay.descriptor.presentation.view,
            PresentationViewKind::PrimaryOverlay
        );
    }

    #[test]
    fn indicator_enabled_source_inputs_require_plot_views_and_visible_numeric_layers() {
        let table = compile(
            r#"
                language(1)
                indicator("Invalid table", id = "test.invalid-table")
                display(view = "table")
                source = input.source(chart.close, "Source")
                plot(source, id = "value", layer = false)
                column(title = "Value", source = "value")
            "#,
            "invalid-table.ind",
        )
        .expect_err("indicator-enabled table source should fail");
        assert!(table.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("allow_indicator = true requires display(view = \"plot\")")
        }));
        assert!(table.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("requires at least one visible numeric plot layer")
        }));

        let overlay = compile(
            r#"
                language(1)
                indicator("Invalid overlay", id = "test.invalid-overlay")
                display(view = "primary_overlay")
                source = input.source(chart.close, "Source")
                plot(source, id = "value", pane = "main")
            "#,
            "invalid-overlay.ind",
        )
        .expect_err("indicator-enabled primary overlay source should fail");
        assert!(overlay.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("allow_indicator = true requires display(view = \"plot\")")
        }));
    }

    #[test]
    fn catalog_source_slots_preserve_declared_defaults() {
        let migrated = [
            (
                "../indicators/core/rsi.ind",
                "core.rsi",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/core/moving-averages.ind",
                "core.moving-averages",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/bollinger-bands.ind",
                "catalog.bollinger-bands",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/cci.ind",
                "catalog.cci",
                Some(FrameField::ChartHlc3),
            ),
            (
                "../indicators/catalog/sources/macd.ind",
                "catalog.macd",
                Some(FrameField::ChartClose),
            ),
            ("../indicators/catalog/sources/mfi.ind", "catalog.mfi", None),
            (
                "../indicators/catalog/sources/momentum.ind",
                "catalog.momentum",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/obv.ind",
                "catalog.obv",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/roc.ind",
                "catalog.roc",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/stoch-rsi.ind",
                "catalog.stoch-rsi",
                Some(FrameField::ChartClose),
            ),
            (
                "../indicators/catalog/sources/trix.ind",
                "catalog.trix",
                Some(FrameField::ChartClose),
            ),
        ];

        for (relative_path, expected_id, default_field) in migrated {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
            let compiled = compile_file(&path).expect("migrated source should compile");
            assert_eq!(compiled.descriptor.id.as_str(), expected_id);
            match default_field {
                Some(default_field) => {
                    assert_eq!(compiled.descriptor.series_inputs.len(), 1);
                    let source = &compiled.descriptor.series_inputs[0];
                    assert_eq!(source.id, "source");
                    assert_eq!(source.label, "Source");
                    assert_eq!(source.default_field, default_field);
                }
                None => assert!(compiled.descriptor.series_inputs.is_empty()),
            }
        }
    }

    #[test]
    fn source_indicators_require_an_explicit_attachment_mode() {
        let overlay = compile(
            r#"
                language(1)
                indicator("Overlay source", id = "test.overlay-source")
                display(attachment = "overlay")
                format(precision = "base_ticker_min_tick")
                source = input.source(chart.close, "Source")
                plot(source, id = "value")
            "#,
            "overlay-source.ind",
        )
        .expect("overlay source should compile");
        assert_eq!(
            overlay.descriptor.presentation.attachment,
            Some(PresentationAttachmentMode::Overlay)
        );

        let missing = compile(
            r#"
                language(1)
                indicator("Missing attachment", id = "test.missing-attachment")
                format(precision = "base_ticker_min_tick")
                source = input.source(chart.close, "Source")
                plot(source, id = "value")
            "#,
            "missing-attachment.ind",
        )
        .expect_err("source indicators must declare an attachment mode");
        assert!(missing.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("display must declare attachment")
        }));

        let invalid = compile(
            r#"
                language(1)
                indicator("Invalid attachment", id = "test.invalid-attachment")
                display(attachment = "split")
                format(precision = "base_ticker_min_tick")
                source = input.source(chart.close, "Source")
                plot(source, id = "value")
            "#,
            "invalid-attachment.ind",
        )
        .expect_err("unknown attachment modes must be rejected");
        assert!(
            invalid
                .iter()
                .any(|diagnostic| { diagnostic.message.contains("unknown attachment split") })
        );
    }

    #[test]
    fn named_panes_and_numeric_layer_targets_are_validated_by_the_compiler() {
        let named = compile(
            r#"
                language(1)
                indicator("Named", id = "test.named")
                format(precision = "base_ticker_min_tick")
                plot(chart.close, id = "value", pane = "named:7")
            "#,
            "named.ind",
        )
        .expect("named pane should compile");
        assert_eq!(named.descriptor.plots[0].pane, PaneTarget::Named(7));
        assert_eq!(
            named.descriptor.presentation.layers[0].pane,
            PaneTarget::Named(7)
        );

        let primary_overlay = compile(
            r#"
                language(1)
                indicator("Overlay", id = "test.named-overlay")
                display(view = "primary_overlay")
                plot(chart.close, id = "value", pane = "named:7")
            "#,
            "named-overlay.ind",
        )
        .expect_err("primary overlays must target the main pane");
        assert!(primary_overlay.iter().any(|diagnostic| {
            diagnostic.message.contains("primary_overlay")
                && diagnostic.message.contains("pane main")
        }));

        let mixed = compile(
            r#"
                language(1)
                indicator("Mixed", id = "test.mixed")
                format(precision = "base_ticker_min_tick")
                plot(chart.close, id = "main", pane = "main")
                plot(chart.close, id = "indicator", pane = "indicator")
            "#,
            "mixed.ind",
        )
        .expect_err("numeric layers must share a pane target");
        assert!(mixed.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("all numeric plot layers in one indicator must use the same pane")
        }));
    }

    #[test]
    fn preview_restarts_from_the_last_committed_state() {
        let source = r#"
            language(1)
            indicator("Committed state", id = "test.state")
            format(precision = "base_ticker_min_tick")
            state total = 0
            total := total + chart.close
            plot(total, id = "total")
        "#;
        let compiled = compile(source, "state.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");
        instance
            .evaluate_closed(frame(0, 1, 1.0))
            .expect("first frame should evaluate");
        instance
            .evaluate_closed(frame(1, 2, 2.0))
            .expect("second frame should evaluate");

        let first_preview = instance
            .preview_open(frame(2, 3, 10.0))
            .expect("preview should evaluate");
        let second_preview = instance
            .preview_open(frame(2, 3, 20.0))
            .expect("preview should evaluate");
        assert_eq!(plot_value(&first_preview, "total"), &Value::Float(13.0));
        assert_eq!(plot_value(&second_preview, "total"), &Value::Float(23.0));

        let committed = instance
            .evaluate_closed(frame(2, 3, 20.0))
            .expect("final open frame should commit");
        assert_eq!(plot_value(&committed, "total"), &Value::Float(23.0));
    }

    #[test]
    fn cumulative_volume_skips_missing_directional_samples() {
        let source = r#"
            language(1)
            indicator("CVD", id = "test.cvd")
            format(precision = "base_ticker_min_qty")
            delta = select(chart.directional_volume_available, chart.buy_qty - chart.sell_qty, na)
            value = ta.cum(delta)
            plot(value, id = "value")
        "#;
        let compiled = compile(source, "cvd.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        let mut first = frame(0, 1, 10.0);
        first.chart.directional_volume_available = true;
        first.chart.buy_qty = 5.0;
        first.chart.sell_qty = 2.0;
        let first_output = instance
            .evaluate_closed(first)
            .expect("first directional frame should evaluate");
        assert_eq!(plot_value(&first_output, "value"), &Value::Float(3.0));

        let mut second = frame(1, 2, 10.0);
        second.chart.directional_volume_available = true;
        second.chart.buy_qty = 4.0;
        second.chart.sell_qty = 1.0;
        let second_output = instance
            .evaluate_closed(second)
            .expect("second directional frame should evaluate");
        assert_eq!(plot_value(&second_output, "value"), &Value::Float(6.0));

        let missing_output = instance
            .evaluate_closed(frame(2, 3, 10.0))
            .expect("missing directional frame should evaluate");
        assert_eq!(plot_value(&missing_output, "value"), &Value::Na);

        let mut third = frame(3, 4, 10.0);
        third.chart.directional_volume_available = true;
        third.chart.buy_qty = 3.0;
        third.chart.sell_qty = 1.0;
        let third_output = instance
            .evaluate_closed(third)
            .expect("directional frame after gap should evaluate");
        assert_eq!(plot_value(&third_output, "value"), &Value::Float(8.0));

        let mut fourth = frame(4, 5, 10.0);
        fourth.chart.directional_volume_available = true;
        fourth.chart.buy_qty = 2.0;
        fourth.chart.sell_qty = 1.0;
        let fourth_output = instance
            .evaluate_closed(fourth)
            .expect("directional frame should resume after gap");
        assert_eq!(plot_value(&fourth_output, "value"), &Value::Float(9.0));
    }

    #[test]
    fn optional_trade_inputs_fall_back_to_candle_volume() {
        let source = r#"
            language(1)
            indicator("Volume", id = "test.volume", optional_trades = true)
            format(precision = "base_ticker_min_qty")
            has_directional_volume = chart.directional_volume_available
            buy = select(has_directional_volume, chart.buy_qty, chart.volume / 2)
            sell = select(has_directional_volume, chart.sell_qty, chart.volume / 2)
            signed = select(has_directional_volume, buy - sell, na)
            plot(buy, id = "buy")
            plot(signed, id = "signed")
        "#;
        let compiled = compile(source, "volume.ind").expect("source should compile");
        assert!(compiled.descriptor.optional_trades);
        assert!(compiled.requirements.needs_directional_volume());
        assert!(!compiled.requirements.trades);
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        let output = instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("candle-only frame should evaluate");
        assert_eq!(plot_value(&output, "buy"), &Value::Float(0.5));
        assert_eq!(plot_value(&output, "signed"), &Value::Na);

        let mut flow_frame = frame(1, 2, 10.0);
        flow_frame.chart.directional_volume_available = true;
        flow_frame.chart.buy_qty = 3.0;
        flow_frame.chart.trade = Some(TradeAggregate {
            buy_qty: 3.0,
            sell_qty: 0.0,
            buy_count: 2,
            sell_count: 0,
        });
        let output = instance
            .evaluate_closed(flow_frame)
            .expect("trade frame should evaluate");
        assert_eq!(plot_value(&output, "buy"), &Value::Float(3.0));
        assert_eq!(plot_value(&output, "signed"), &Value::Float(3.0));
    }

    #[test]
    fn removed_cross_ticker_inputs_fail_compilation() {
        for (name, expression) in [
            ("source", r#"source("BTCUSDT")"#),
            ("sources", "sources"),
            ("flow", "flow.trade.count"),
        ] {
            let source = format!(
                "language(1)\nindicator(\"Removed\", id = \"test.removed.{name}\")\nplot({expression}, id = \"value\")"
            );
            assert!(
                compile(&source, format!("{name}.ind")).is_err(),
                "{name} should fail"
            );
        }
    }

    #[test]
    fn chart_trade_fields_use_the_base_chart() {
        let source = r#"
            language(1)
            indicator("Base trades", id = "test.base-trades")
            format(precision = "base_ticker_min_qty")
            plot(chart.trade.available, id = "chart_available")
            plot(chart.trade.buy_qty, id = "chart_buy")
            plot(chart.trade.sell_qty, id = "chart_sell")
            plot(chart.trade.buy_count, id = "chart_cbuy")
            plot(chart.trade.sell_count, id = "chart_csell")
            plot(chart.trade.count, id = "chart_trades")
            plot(chart.directional_volume_available, id = "directional_available")
            plot(chart.buy_qty, id = "buy_qty")
            plot(chart.sell_qty, id = "sell_qty")
        "#;
        let compiled = compile(source, "base-trades.ind").expect("source should compile");
        assert!(compiled.requirements.trades);
        assert!(compiled.requirements.needs_directional_volume());

        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");
        let trade = TradeAggregate {
            buy_qty: 7.0,
            sell_qty: 3.0,
            buy_count: 3,
            sell_count: 1,
        };
        let mut populated = frame(0, 1, 10.0);
        populated.chart.directional_volume_available = true;
        populated.chart.buy_qty = trade.buy_qty;
        populated.chart.sell_qty = trade.sell_qty;
        populated.chart.trade = Some(trade);

        let populated_output = instance
            .evaluate_closed(populated)
            .expect("populated trade frame should evaluate");
        assert_eq!(
            plot_value(&populated_output, "chart_available"),
            &Value::Bool(true)
        );
        assert_eq!(
            plot_value(&populated_output, "chart_buy"),
            &Value::Float(7.0)
        );
        assert_eq!(
            plot_value(&populated_output, "chart_sell"),
            &Value::Float(3.0)
        );
        assert_eq!(plot_value(&populated_output, "chart_cbuy"), &Value::Int(3));
        assert_eq!(plot_value(&populated_output, "chart_csell"), &Value::Int(1));
        assert_eq!(
            plot_value(&populated_output, "chart_trades"),
            &Value::Int(4)
        );
        assert_eq!(
            plot_value(&populated_output, "directional_available"),
            &Value::Bool(true)
        );
        assert_eq!(plot_value(&populated_output, "buy_qty"), &Value::Float(7.0));
        assert_eq!(
            plot_value(&populated_output, "sell_qty"),
            &Value::Float(3.0)
        );

        for (name, expression) in [
            ("bare-close", "close"),
            ("bare-directional", "directional_volume_available"),
            ("vbuy", "vbuy"),
            ("vsell", "vsell"),
            ("trades", "trades"),
            ("cbuy", "cbuy"),
            ("csell", "csell"),
        ] {
            let source = format!(
                "language(1)\nindicator(\"Removed\", id = \"community.removed-{name}\")\nformat(precision = \"base_ticker_min_qty\")\nplot({expression}, id = \"value\")"
            );
            assert!(
                compile(&source, format!("{name}.ind")).is_err(),
                "{name} should fail"
            );
        }
    }

    #[test]
    fn removed_dsl_aliases_fail_compilation() {
        for (name, statement) in [
            ("minval", r#"input.int(14, "Period", minval = 1)"#),
            ("plotshape", "plotshape(value)"),
            ("plotchar", "plotchar(value)"),
            ("hline", "hline(1)"),
            ("cloud", "cloud(value, value)"),
            (
                "channel",
                r#"readout(title = "Value", channel = "value", color_slot = "base")"#,
            ),
            (
                "fill-offset",
                r#"fill(upper = "value", lower = "value", color_slot = "base", upper_offset_bars = 1)"#,
            ),
            (
                "fill-geometry",
                r#"fill(upper = "value", lower = "value", color_slot = "base", geometry = "panel")"#,
            ),
            (
                "plot-precision",
                r#"plot(value, id = "second", precision = 2)"#,
            ),
            (
                "plot-scale",
                r#"plot(value, id = "second", scale = "separate")"#,
            ),
            (
                "plot-style",
                r#"plot(value, id = "second", style = "columns")"#,
            ),
            (
                "plot-display",
                r#"plot(value, id = "second", display = "hidden")"#,
            ),
            ("role", r#"style("secondary", role = "positive")"#),
            ("shade", r#"style("secondary", shade = "intense")"#),
            (
                "mark",
                r#"mark(title = "Histogram", source = "value", kind = "histogram_plain", color_slot = "base")"#,
            ),
        ] {
            let source = format!(
                "language(1)\nindicator(\"Removed\", id = \"community.removed-{name}\")\nformat(precision = \"base_ticker_min_tick\")\nstyle(\"base\")\nvalue = chart.close\nplot(value, id = \"value\", color_slot = \"base\")\n{statement}\n"
            );
            assert!(
                compile(&source, format!("{name}.ind")).is_err(),
                "{name} should fail"
            );
        }

        let indicator_aliases = [
            ("overlay", r#", overlay = true"#),
            ("kind", r#", kind = "plot""#),
            ("shorttitle", r#", shorttitle = "Removed""#),
            ("precision", r#", precision = 2"#),
            ("layer-title", r#", layer_title = "Removed""#),
            ("view", r#", view = "overlay""#),
            ("attachment", r#", attachment = "panel""#),
            ("panel-title", r#", panel_title = "Removed""#),
            ("layer-name", r#", layer_name = "Removed""#),
            ("data-kind", r#", data_kind = "scalar""#),
            ("mark", r#", mark = "line""#),
            ("axis", r#", axis = "primary""#),
            (
                "value-precision",
                r#", value_precision = "base_ticker_min_tick""#,
            ),
            ("axis-label-mode", r#", axis_label_mode = "compact""#),
            ("header-label-mode", r#", header_label_mode = "compact""#),
            ("preferred-scale", r#", preferred_scale = "fit_visible""#),
            ("max-decimals", r#", max_decimals = 2""#),
        ];
        for (name, arguments) in indicator_aliases {
            let source = format!(
                "language(1)\nindicator(\"Removed\", id = \"community.removed-indicator-{name}\"{arguments})\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n"
            );
            assert!(
                compile(&source, format!("indicator-{name}.ind")).is_err(),
                "indicator {name} should fail"
            );
        }

        let table = r#"
            language(1)
            indicator("Removed bool", id = "community.removed-bool")
            display(view = "table")
            plot(chart.close, id = "value")
            column(title = "Value", source = "value", format = "bool")
        "#;
        assert!(compile(table, "bool-format.ind").is_err());
    }

    #[test]
    fn signed_table_columns_require_typed_color_bindings() {
        let source = r#"
            language(1)
            indicator("Signed table", id = "test.signed-table")
            display(view = "table")
            delta = chart.close - chart.open
            plot(delta, id = "delta")
            style("positive", title = "Positive values")
            style("negative", title = "Negative values")
            column(title = "Delta", source = "delta", color_mode = "signed",
                positive_color_slot = "positive", negative_color_slot = "negative")
        "#;
        let compiled = compile(source, "signed-table.ind").expect("table should compile");
        assert_eq!(compiled.descriptor.presentation.columns.len(), 1);
        assert!(matches!(
            compiled.descriptor.presentation.columns[0].color,
            PresentationColumnColor::Signed {
                ref positive_slot,
                ref negative_slot
            } if positive_slot == "positive" && negative_slot == "negative"
        ));

        for (source, expected_message) in [
            (
                r#"
                    language(1)
                    indicator("Invalid table", id = "test.invalid-table")
                    display(view = "table")
                    plot(chart.close, id = "value")
                    style("positive")
                    style("negative")
                    column(title = "Value", source = "value", color_mode = "signed",
                        color_slot = "positive", positive_color_slot = "positive",
                        negative_color_slot = "negative")
                "#,
                "cannot provide color_slot",
            ),
            (
                r#"
                    language(1)
                    indicator("Invalid table", id = "test.invalid-table")
                    display(view = "table")
                    plot(chart.close, id = "value")
                    style("positive")
                    column(title = "Value", source = "value", color_mode = "signed",
                        positive_color_slot = "positive")
                "#,
                "require negative_color_slot",
            ),
        ] {
            let diagnostics = compile(source, "invalid-table.ind")
                .expect_err("invalid table colors should be diagnosed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected_message))
            );
        }
    }

    #[test]
    fn unsupported_execution_paths_are_rejected_at_compile_time() {
        for (name, expression) in [
            (
                "request",
                "request.security(\"ETHUSDT\", \"1h\", chart.close)",
            ),
            ("drawing", "box.new(1, 2, 3, 4)"),
            ("map", "map.new()"),
        ] {
            let source = format!(
                "language(1)\nindicator(\"Unsupported\", id = \"test.{name}\")\nvalue = {expression}\nplot(value)"
            );
            let diagnostics = compile(&source, format!("{name}.ind"))
                .expect_err("unsupported paths must fail compilation");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| { diagnostic.message.contains("not available") })
            );
        }

        let diagnostics = compile(
            "language(1)\nindicator(\"Intrabar\", id = \"test.intrabar\")\nvarip ticks = 0\nplot(ticks)",
            "intrabar.ind",
        )
        .expect_err("varip must fail compilation");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("varip"))
        );
    }

    #[test]
    fn presentation_scene_is_compiled_with_owned_defaults() {
        let source = r#"
            language(1)
            indicator("Scene", id = "community.scene")
            display(panel_title = "Panel", layer_name = "Layer")
            format(scale = "fit_visible_include_zero", precision = "fixed_step:0.5",
                axis_labels = "abbreviated", header_labels = "commas", max_decimals = 2)
            period = input.int(3, "Period", min = 1, max = 20, step = 1)
            value = ta.sma(chart.close, period)
            upper = value + 1
            lower = value - 1
            rising = value > value[1]
            plot(value, id = "value", title = "Value", precision = "decimals:2",
                width = 1.5, offset = 1, color_slot = "base", style_by = "rising",
                true_color_slot = "bull", false_color_slot = "bear")
            plot(upper, id = "upper", title = "Upper", color_slot = "band")
            plot(lower, id = "lower", title = "Lower", color_slot = "band")
            plot(rising, id = "rising", title = "Rising", layer = false)
            style("base", title = "Base", role = "neutral", shade = "base")
            style("bull", title = "Bull", role = "success", shade = "base")
            style("bear", title = "Bear", role = "danger", shade = "base")
            style("band", title = "Band", role = "secondary", shade = "soft")
            mark(title = "Histogram", source = "value", kind = "histogram", color_slot = "base",
                highlight_by = "rising", highlight_color_slot = "bull",
                requires = "directional_volume", fallback = "line")
            mark(title = "Line", source = "value", kind = "line", color_slot = "base")
            fill(upper = "upper", lower = "lower", color_slot = "band", opacity = 0.2,
                style_by = "rising", true_color_slot = "bull", false_color_slot = "bear")
            readout(title = "Signal", source = "value", color_slot = "base")
            segment(value, upper, bar_index, bar_index, id = "projection",
                title = "Projection", color_slot = "band", width = 1.5)
        "#;
        let compiled = compile(source, "scene.ind").expect("presentation scene should compile");
        let presentation = &compiled.descriptor.presentation;
        assert!(!compiled.requirements.needs_directional_volume());
        assert_eq!(
            presentation.value_precision,
            Some(PresentationPrecision::FixedStep(0.5))
        );
        assert_eq!(
            compiled.descriptor.plots[0].output_precision,
            Some(OutputPrecision::Decimals(2))
        );
        assert_eq!(presentation.panel_title.as_deref(), Some("Panel"));
        assert_eq!(presentation.layer_name.as_deref(), Some("Layer"));
        assert_eq!(presentation.styles.len(), 4);
        assert_eq!(presentation.layers.len(), 3);
        assert_eq!(
            presentation
                .layers
                .iter()
                .map(|layer| layer.channel.as_str())
                .collect::<Vec<_>>(),
            vec!["value", "upper", "lower"]
        );
        assert_eq!(
            compiled.descriptor.plots[0].color_slot.as_deref(),
            Some("base")
        );
        assert_eq!(compiled.descriptor.plots[0].offset_bars, 1);
        assert_eq!(
            compiled.descriptor.plots[3].readout,
            ReadoutBehavior::Hidden,
            "boolean plots are output-only by default"
        );
        assert_eq!(presentation.marks[0].kind, PresentationMarkKind::Histogram);
        assert_eq!(
            presentation.marks[0].required_input,
            Some(PresentationInputSource::DirectionalVolume)
        );
        assert_eq!(
            presentation.marks[0].fallback_mark,
            Some(PresentationMarkKind::Line)
        );
        assert_eq!(
            presentation.marks[0].highlight_by.as_deref(),
            Some("rising")
        );
        assert_eq!(
            presentation.marks[0].highlight_color_slot.as_deref(),
            Some("bull")
        );
        assert_eq!(presentation.fills[0].style_by.as_deref(), Some("rising"));
        assert_eq!(presentation.fills[0].opacity, 0.2);
        assert_eq!(presentation.readouts[0].channel, "value");
        assert_eq!(presentation.segments[0].id, "projection");
    }

    #[test]
    fn mark_highlight_bindings_are_validated() {
        let cases = [
            (
                "numeric",
                r#"highlight_by = "value", highlight_color_slot = "warning""#,
                "highlight_by must reference a boolean output",
            ),
            (
                "missing-color",
                r#"highlight_by = "rising""#,
                "highlight_by requires highlight_color_slot",
            ),
            (
                "line",
                r#"highlight_by = "rising", highlight_color_slot = "warning""#,
                "highlight_by requires a bar mark",
            ),
        ];

        for (name, binding, expected_message) in cases {
            let kind = if name == "line" { "line" } else { "histogram" };
            let source = format!(
                r#"
                    language(1)
                    indicator("Highlight", id = "community.highlight-{name}")
                    format(precision = "base_ticker_min_tick")
                    value = chart.close
                    rising = chart.close > chart.open
                    style("base")
                    style("warning", role = "warning")
                    plot(value, id = "value", title = "Value", color_slot = "base")
                    plot(rising, id = "rising", layer = false)
                    mark(title = "Mark", source = "value", kind = "{kind}", color_slot = "base", {binding})
                "#
            );
            let diagnostics = compile(&source, format!("{name}.ind"))
                .expect_err("invalid mark highlight binding should be diagnosed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected_message)),
                "expected {expected_message}, got {diagnostics:?}"
            );
        }
    }

    #[test]
    fn numeric_plot_panels_require_format_precision() {
        let diagnostics = compile(
            "language(1)\nindicator(\"Missing precision\", id = \"test.missing-precision\")\nplot(chart.close, id = \"value\")\n",
            "missing-precision.ind",
        )
        .expect_err("numeric plot panels should require value precision");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.message.contains("must declare format(precision") })
        );

        let explicit = compile(
            "language(1)\nindicator(\"Explicit precision\", id = \"test.explicit-precision\")\nformat(precision = \"fixed_step:0.01\")\nplot(chart.close, id = \"value\")\n",
            "explicit-precision.ind",
        )
        .expect("numeric plot panels with value precision should compile");
        assert_eq!(
            explicit.descriptor.presentation.value_precision,
            Some(PresentationPrecision::FixedStep(0.01))
        );

        compile(
            "language(1)\nindicator(\"Overlay\", id = \"test.overlay\")\ndisplay(view = \"primary_overlay\")\nplot(chart.close, id = \"value\")\n",
            "overlay.ind",
        )
        .expect("primary overlays should not require panel precision");

        compile(
            "language(1)\nindicator(\"Table\", id = \"test.table\")\ndisplay(view = \"table\")\nplot(chart.close, id = \"value\", display = \"none\", overlay = false, layer = false)\ncolumn(title = \"Value\", source = \"value\")\n",
            "table.ind",
        )
        .expect("table views should not require panel precision");
    }

    #[test]
    fn display_and_format_declarations_are_unique_and_precede_outputs() {
        for (source, message) in [
            (
                "language(1)\nindicator(\"Duplicate display\", id = \"test.duplicate-display\")\ndisplay()\ndisplay()\nplot(chart.close, id = \"value\")\n",
                "duplicate display declaration",
            ),
            (
                "language(1)\nindicator(\"Duplicate format\", id = \"test.duplicate-format\")\nformat(precision = \"base_ticker_min_tick\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
                "duplicate format declaration",
            ),
            (
                "language(1)\nindicator(\"Late display\", id = \"test.late-display\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\ndisplay()\n",
                "display declarations must appear before output declarations",
            ),
            (
                "language(1)\nindicator(\"Late format\", id = \"test.late-format\")\nplot(chart.close, id = \"value\")\nformat(precision = \"base_ticker_min_tick\")\n",
                "format declarations must appear before output declarations",
            ),
        ] {
            let diagnostics = compile(source, "metadata.ind")
                .expect_err("invalid metadata declarations should be diagnosed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(message))
            );
        }
    }

    #[test]
    fn generic_plot_accepts_scalar_styles_and_structured_constructors() {
        let scalar_source = r#"
            language(1)
            indicator("Scalar styles", id = "test.scalar-styles")
            format(precision = "base_ticker_min_tick")
            plot(chart.close, id = "line", style = "line")
            plot(chart.close, id = "step", style = "step")
            plot(chart.close, id = "histogram", style = "histogram")
            plot(chart.close, id = "area", style = "area")
        "#;
        compile(scalar_source, "scalar-styles.ind").expect("scalar plot styles should compile");

        let structured_source = r#"
            language(1)
            indicator("Structured plots", id = "test.structured-plots")
            display(view = "primary_overlay")
            format(precision = "base_ticker_min_tick")
            plotcandle(chart.open, chart.high, chart.low, chart.close, id = "candles")
            plotohlc(chart.open, chart.high, chart.low, chart.close, id = "ohlc")
            plotrange(chart.high, chart.low, id = "range")
            broken_cloud(chart.high, chart.low, id = "broken")
        "#;
        compile(structured_source, "structured-plots.ind")
            .expect("structured plot constructors should compile");
    }

    #[test]
    fn generic_plot_rejects_structured_styles() {
        for (style, constructors) in [
            ("candles", "plotcandle or plotohlc"),
            ("bars", "plotcandle or plotohlc"),
            ("cloud", "plotrange or broken_cloud"),
            ("broken_cloud", "plotrange or broken_cloud"),
        ] {
            let source = format!(
                "language(1)\nindicator(\"Invalid plot\", id = \"test.invalid-{style}\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\", style = \"{style}\")"
            );
            let diagnostics = compile(&source, format!("invalid-{style}.ind"))
                .expect_err("structured styles should not be accepted by generic plot");
            assert!(diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .contains(&format!("plot style '{style}'"))
                    && diagnostic.message.contains("requires a structured plot")
                    && diagnostic.message.contains(constructors)
            }));
        }
    }

    #[test]
    fn unknown_presentation_input_is_rejected_without_becoming_an_indicator_requirement() {
        let diagnostics = compile(
            r#"
                language(1)
                indicator("Unknown input", id = "community.unknown-input")
                format(precision = "base_ticker_min_tick")
                value = chart.close
                style("base", title = "Base")
                plot(value, id = "value", title = "Value", color_slot = "base")
                mark(title = "Bars", source = "value", kind = "histogram", color_slot = "base",
                    requires = "not_a_real_input")
            "#,
            "unknown-input.ind",
        )
        .expect_err("unknown presentation input should be diagnosed");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unknown presentation input not_a_real_input")
        }));
    }

    #[test]
    fn invalid_presentation_fallbacks_are_rejected() {
        let cases = [
            (
                "unknown",
                r#"fallback = "not_a_real_mark""#,
                "unknown presentation mark not_a_real_mark",
            ),
            (
                "missing",
                r#"fallback = "line""#,
                "references unknown fallback mark",
            ),
            (
                "self",
                r#"fallback = "histogram""#,
                "cannot fall back to itself",
            ),
        ];

        for (name, fallback, expected_message) in cases {
            let source = format!(
                r#"
                    language(1)
                    indicator("Fallback", id = "community.fallback-{name}")
                    format(precision = "base_ticker_min_tick")
                    value = chart.close
                    style("base")
                    plot(value, id = "value", title = "Value", color_slot = "base")
                    mark(title = "Histogram", source = "value", kind = "histogram",
                        color_slot = "base", {fallback})
                "#
            );
            let diagnostics = compile(&source, format!("{name}.ind"))
                .expect_err("invalid presentation fallback should be diagnosed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected_message))
            );
        }

        let diagnostics = compile(
            r#"
                language(1)
                indicator("Input fallback", id = "community.input-fallback")
                format(precision = "base_ticker_min_tick")
                value = chart.close
                style("base")
                plot(value, id = "value", title = "Value", color_slot = "base")
                mark(title = "Overlay", source = "value", kind = "histogram",
                    color_slot = "base", requires = "directional_volume", fallback = "line")
                mark(title = "Line", source = "value", kind = "line", color_slot = "base",
                    requires = "trades")
            "#,
            "input-fallback.ind",
        )
        .expect_err("input-dependent fallback should be diagnosed");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("fallback mark Line cannot require input")
        }));
    }

    #[test]
    fn sparse_plot_declarations_materialize_the_same_layer_defaults() {
        let explicit = compile(
            r#"
                language(1)
                indicator("Sparse", id = "community.sparse")
                display(view = "plot", panel_title = "Sparse", layer_name = "Sparse")
                format(precision = "base_ticker_min_tick", scale = "fit_visible")
                value = chart.close
                style("value", title = "value", role = "neutral", shade = "base")
                plot(value, id = "value", title = "Value", pane = "indicator",
                    scale = "price", style = "line", display = "value", width = 1,
                    offset = 0, color_slot = "value", overlay = true,
                    data_kind = "scalar", mark = "line", axis = "secondary", layer = true)
            "#,
            "explicit.ind",
        )
        .expect("explicit defaults should compile");
        let sparse = compile(
            r#"
                language(1)
                indicator("Sparse", id = "community.sparse")
                format(precision = "base_ticker_min_tick")
                value = chart.close
                style("value")
                plot(value, id = "value", title = "Value")
            "#,
            "sparse.ind",
        )
        .expect("sparse declarations should compile");

        assert_eq!(
            explicit.descriptor.presentation.layers,
            sparse.descriptor.presentation.layers
        );
        assert_eq!(
            explicit.descriptor.presentation.styles,
            sparse.descriptor.presentation.styles
        );
        assert_eq!(
            explicit.descriptor.plots[0].pane,
            sparse.descriptor.plots[0].pane
        );
        assert_eq!(
            explicit.descriptor.plots[0].scale,
            sparse.descriptor.plots[0].scale
        );
        assert_eq!(
            explicit.descriptor.plots[0].width,
            sparse.descriptor.plots[0].width
        );
        assert_eq!(
            explicit.descriptor.plots[0].overlay,
            sparse.descriptor.plots[0].overlay
        );
        assert_eq!(
            explicit.descriptor.plots[0].layer,
            sparse.descriptor.plots[0].layer
        );
    }

    #[test]
    fn table_columns_are_typed_and_require_a_table_view() {
        let source = r#"
            language(1)
            indicator("Table", id = "community.table")
            display(view = "table")
            style("main")
            value = chart.close
            trend = chart.close > chart.close[1]
            plot(value, id = "value", layer = false)
            plot(trend, id = "trend", layer = false)
            column(title = "Value", source = "value", format = "quantity", color_slot = "main")
            column(title = "Trend", source = "trend", format = "boolean", color_slot = "main")
        "#;
        let compiled = compile(source, "table.ind").expect("table scene should compile");
        assert_eq!(compiled.descriptor.presentation.columns.len(), 2);
        assert_eq!(
            compiled.descriptor.presentation.view,
            PresentationViewKind::Table
        );
    }

    #[test]
    fn na_comparisons_propagate_and_switch_falls_back() {
        let source = r#"
            language(1)
            indicator("Missing values", id = "community.missing-values")
            format(precision = "base_ticker_min_tick")
            plot(na == na, id = "equal")
            plot(switch na { na => 1, default => 2 }, id = "switch")
        "#;
        let compiled = compile(source, "missing-values.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");
        let output = instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("frame should evaluate");

        assert_eq!(plot_value(&output, "equal"), &Value::Na);
        assert_eq!(plot_value(&output, "switch"), &Value::Int(2));
    }

    #[test]
    fn state_initializers_retry_na_and_warm_up() {
        let source = r#"
            language(1)
            indicator("State seed", id = "community.state-seed")
            format(precision = "base_ticker_min_tick")
            state average = ta.sma(chart.close, 2)
            plot(average, id = "average")
        "#;
        let compiled = compile(source, "state-seed.ind").expect("source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(2));
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(0, 1, 10.0))
                    .expect("first frame should evaluate"),
                "average"
            ),
            &Value::Na
        );
        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(1, 2, 20.0))
                    .expect("second frame should evaluate"),
                "average"
            ),
            &Value::Float(15.0)
        );
    }

    #[test]
    fn explicit_na_state_updates_remain_authoritative() {
        let source = r#"
            language(1)
            indicator("Explicit missing", id = "community.explicit-missing")
            format(precision = "base_ticker_min_tick")
            state value = chart.close
            if bar_index == 0 {
                value := na
            }
            plot(value, id = "value")
        "#;
        let compiled = compile(source, "explicit-missing.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");
        instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("first frame should evaluate");
        let output = instance
            .evaluate_closed(frame(1, 2, 20.0))
            .expect("second frame should evaluate");

        assert_eq!(plot_value(&output, "value"), &Value::Na);
    }

    #[test]
    fn mutable_bindings_keep_compatible_types_across_updates_and_branches() {
        for source in [
            r#"
                language(1)
                indicator("Invalid state", id = "test.invalid-state")
                state value = 0
                value := true
                plot(value, id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid local", id = "test.invalid-local")
                value = 0
                value := true
                plot(value, id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid conditional", id = "test.invalid-conditional")
                state value = 0
                if true {
                    value := true
                }
                plot(value, id = "value")
            "#,
        ] {
            let diagnostics = compile(source, "invalid-binding.ind")
                .expect_err("incompatible binding updates should fail");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("incompatible types"))
            );
        }

        let source = r#"
            language(1)
            indicator("Widened state", id = "test.widened-state")
            format(precision = "base_ticker_min_tick")
            state value = 0
            value := chart.close
            value := na
            plot(value, id = "value")
        "#;
        let compiled = compile(source, "widened-state.ind").expect("compatible updates compile");
        assert_eq!(
            compiled.descriptor.plots[0].value_type,
            PlotValueType::Number
        );

        let source = r#"
            language(1)
            indicator("Conditional value", id = "test.conditional-value")
            state value = na
            if true {
                value := true
            } else {
                value := false
            }
            plot(value, id = "value")
        "#;
        let compiled = compile(source, "conditional-value.ind").expect("Any should resolve");
        assert_eq!(
            compiled.descriptor.plots[0].value_type,
            PlotValueType::Boolean
        );

        for source in [
            r#"
                language(1)
                indicator("While value", id = "test.while-value")
                state value = na
                while false {
                    value := true
                }
                plot(value, id = "value")
            "#,
            r#"
                language(1)
                indicator("For value", id = "test.for-value")
                state value = na
                for index = 0 to 1 {
                    value := true
                }
                plot(value, id = "value")
            "#,
        ] {
            let compiled = compile(source, "loop-value.ind").expect("loop joins compile");
            assert_eq!(
                compiled.descriptor.plots[0].value_type,
                PlotValueType::Boolean
            );
        }
    }

    #[test]
    fn result_branches_and_binary_operands_are_checked() {
        for source in [
            r#"
                language(1)
                indicator("Invalid select", id = "test.invalid-select")
                plot(select(true, 1, false), id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid coalesce", id = "test.invalid-coalesce")
                plot(coalesce(1, false), id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid switch", id = "test.invalid-switch")
                plot(switch 1 { 1 => 1, default => false }, id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid order", id = "test.invalid-order")
                plot(true < false, id = "value")
            "#,
            r#"
                language(1)
                indicator("Invalid logical", id = "test.invalid-logical")
                plot(1 && true, id = "value")
            "#,
        ] {
            let diagnostics = compile(source, "invalid-expression.ind")
                .expect_err("invalid expression types should fail");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("incompatible")
                        || diagnostic.message.contains("operands"))
            );
        }

        let source = r#"
            language(1)
            indicator("Valid comparisons", id = "test.valid-comparisons")
            plot(chart.close < chart.open, id = "numeric")
            plot(chart.time < chart.time[1], id = "time")
        "#;
        let compiled = compile(source, "valid-comparisons.ind").expect("valid comparisons compile");
        assert_eq!(
            compiled.descriptor.plots[0].value_type,
            PlotValueType::Boolean
        );
        assert_eq!(
            compiled.descriptor.plots[1].value_type,
            PlotValueType::Boolean
        );
    }

    #[test]
    fn cumulative_overflow_becomes_missing_without_poisoning_state() {
        let source = r#"
            language(1)
            indicator("Cumulative overflow", id = "test.cumulative-overflow")
            format(precision = "base_ticker_min_tick")
            plot(ta.cum(chart.close), id = "value")
        "#;
        let compiled = compile(source, "cumulative-overflow.ind").expect("source should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");

        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(0, 1, f64::MAX))
                    .expect("first frame should evaluate"),
                "value"
            ),
            &Value::Float(f64::MAX)
        );
        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(1, 2, f64::MAX))
                    .expect("overflow should remain recoverable"),
                "value"
            ),
            &Value::Na
        );
        assert_eq!(
            plot_value(
                &instance
                    .evaluate_closed(frame(2, 3, -f64::MAX))
                    .expect("state should remain usable"),
                "value"
            ),
            &Value::Float(0.0)
        );
    }

    #[test]
    fn functions_are_numeric_and_typed_before_their_bodies_compile() {
        let source = r#"
            language(1)
            indicator("Function", id = "community.function")
            format(precision = "base_ticker_min_tick")
            plot(identity(chart.close), id = "value")
            fn identity(value) {
                return value
            }
        "#;
        let compiled = compile(source, "function.ind").expect("numeric function should compile");
        let mut instance = compiled
            .instantiate(IndicatorConfig::default())
            .expect("inputs should bind");
        let output = instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("frame should evaluate");
        assert_eq!(plot_value(&output, "value"), &Value::Float(10.0));

        let diagnostics = compile(
            r#"
                language(1)
                indicator("Invalid function", id = "community.invalid-function")
                format(precision = "base_ticker_min_tick")
                plot(identity(chart.close > chart.open), id = "value")
                fn identity(value) {
                    return value
                }
            "#,
            "invalid-function.ind",
        )
        .expect_err("boolean function arguments should be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("must be numeric or na"))
        );
    }

    #[test]
    fn input_arguments_and_runtime_values_are_validated() {
        for (source, message) in [
            (
                r#"
                    language(1)
                    indicator("Unknown input argument", id = "community.unknown-input")
                    value = input.int(1, "Value", unknown = 2)
                    plot(value, id = "value")
                "#,
                "unknown argument",
            ),
            (
                r#"
                    language(1)
                    indicator("Duplicate options", id = "community.duplicate-options")
                    value = input.string("a", "Value", options = ["a", "a"])
                    plot(chart.close, id = "value")
                "#,
                "is duplicated",
            ),
        ] {
            let diagnostics = compile(source, "invalid-input.ind")
                .expect_err("invalid input declarations should be diagnosed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(message))
            );
        }

        let compiled = compile(
            r#"
                language(1)
                indicator("Bounded input", id = "community.bounded-input")
                format(precision = "base_ticker_min_tick")
                value = input.int(1, "Value", min = 1, max = 5)
                plot(chart.close, id = "value")
            "#,
            "bounded-input.ind",
        )
        .expect("bounded input should compile");
        let error = compiled
            .instantiate(IndicatorConfig::default().with("value", InputValue::Int(6)))
            .expect_err("out-of-range input should be rejected");
        assert_eq!(error, PlanningError::InvalidInput("value".to_owned()));

        let compiled = compile(
            r#"
                language(1)
                indicator("Choice input", id = "community.choice-input")
                format(precision = "base_ticker_min_tick")
                method = input.string("a", "Method", options = ["a", "b"])
                plot(chart.close, id = "value")
            "#,
            "choice-input.ind",
        )
        .expect("choice input should compile");
        let error = compiled
            .instantiate(
                IndicatorConfig::default().with("method", InputValue::String("other".to_owned())),
            )
            .expect_err("unknown option should be rejected");
        assert_eq!(error, PlanningError::InvalidInput("method".to_owned()));
    }

    #[test]
    fn transient_collection_values_obey_heap_limits() {
        let source = r#"
            language(1)
            indicator("Collection", id = "community.collection")
            format(precision = "base_ticker_min_tick")
            plot(array.size(array.push(array.new(), "12345")), id = "size")
        "#;
        let compiled = compile(source, "collection.ind").expect("source should compile");
        let limits = RuntimeLimits {
            max_heap_bytes: 4,
            ..RuntimeLimits::default()
        };
        let mut instance = compiled
            .instantiate_with_limits(IndicatorConfig::default(), limits)
            .expect("inputs should bind");
        assert_eq!(
            instance
                .evaluate_closed(frame(0, 1, 10.0))
                .expect_err("temporary string collection should hit the heap limit"),
            RuntimeError::ResourceLimitExceeded("heap bytes")
        );
    }

    #[test]
    fn ta_window_limits_are_checked_without_counting_ta_state_as_collections() {
        let source = r#"
            language(1)
            indicator("Window", id = "community.window")
            format(precision = "base_ticker_min_tick")
            value = ta.sma(chart.close, 6)
            plot(value, id = "value")
        "#;
        let compiled = compile(source, "window.ind").expect("source should compile");
        let limits = RuntimeLimits {
            max_window_length: 5,
            ..RuntimeLimits::default()
        };
        let mut instance = compiled
            .instantiate_with_limits(IndicatorConfig::default(), limits)
            .expect("inputs should bind");
        assert_eq!(
            instance
                .evaluate_closed(frame(0, 1, 10.0))
                .expect_err("oversized TA window should hit the window limit"),
            RuntimeError::ResourceLimitExceeded("window length")
        );

        let source = source.replace("ta.sma(chart.close, 6)", "ta.sma(chart.close, 2)");
        let compiled = compile(&source, "window-collection.ind").expect("source should compile");
        let limits = RuntimeLimits {
            max_collection_elements: 0,
            ..RuntimeLimits::default()
        };
        let mut instance = compiled
            .instantiate_with_limits(IndicatorConfig::default(), limits)
            .expect("inputs should bind");
        instance
            .evaluate_closed(frame(0, 1, 10.0))
            .expect("TA state should not consume collection elements");
    }
}
