use crate::CompiledIndicator;
use crate::frame::{
    FrameOutput, HorizontalLevelOutput, IndicatorFrame, MarkerOutput, OhlcOutput, PlotOutput,
    RangeOutput, SegmentOutput, SeriesInputValue,
};
use crate::hir::{
    BarStateField, BinaryOperator, ExprId, ExpressionKind, SourceInputSlot, StateSlot,
    StatementKind,
};
use crate::intrinsic::{IntrinsicError, IntrinsicEvalContext, IntrinsicRegistry, IntrinsicState};
use crate::scene::Color;
use crate::types::{IndicatorConfig, InputType, InputValue, Value};
use std::collections::{BTreeMap, VecDeque};
use std::mem::size_of;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
struct IndexedHistory {
    first_index: usize,
    capacity: usize,
    values: VecDeque<Value>,
    resource_collections: usize,
    resource_heap_bytes: usize,
}

impl IndexedHistory {
    fn new(capacity: usize) -> Self {
        Self {
            first_index: 0,
            capacity: capacity.max(1),
            values: VecDeque::new(),
            resource_collections: 0,
            resource_heap_bytes: 0,
        }
    }

    fn get(&self, index: usize) -> Option<&Value> {
        let offset = index.checked_sub(self.first_index)?;
        self.values.get(offset)
    }

    fn set(&mut self, index: usize, value: Value) {
        let value_usage = value.resource_usage();
        if self.values.is_empty() {
            self.first_index = index;
            self.values.push_back(value);
            self.add_usage(value_usage);
            return;
        }

        if index < self.first_index {
            return;
        }

        let end_index = self.first_index.saturating_add(self.values.len());
        if index == end_index {
            self.values.push_back(value);
            self.add_usage(value_usage);
        } else if let Some(existing) = self.values.get_mut(index - self.first_index) {
            let previous_usage = existing.resource_usage();
            *existing = value;
            self.subtract_usage(previous_usage);
            self.add_usage(value_usage);
        } else {
            return;
        }

        while self.values.len() > self.capacity {
            if let Some(evicted) = self.values.pop_front() {
                self.subtract_usage(evicted.resource_usage());
            }
            self.first_index = self.first_index.saturating_add(1);
        }
    }

    fn evict_before(&mut self, index: usize) {
        if index <= self.first_index {
            return;
        }

        let remove = index
            .saturating_sub(self.first_index)
            .min(self.values.len());
        let removed_usage = self
            .values
            .drain(..remove)
            .map(|value| value.resource_usage())
            .fold(
                (0usize, 0usize),
                |(collections, bytes), (value_collections, value_bytes)| {
                    (
                        collections.saturating_add(value_collections),
                        bytes.saturating_add(value_bytes),
                    )
                },
            );
        self.subtract_usage(removed_usage);
        self.first_index = self.first_index.saturating_add(remove);
    }

    fn resource_usage(&self) -> (usize, usize) {
        (self.resource_collections, self.resource_heap_bytes)
    }

    fn container_heap_bytes(&self) -> usize {
        size_of::<VecDeque<Value>>()
            .saturating_add(self.values.capacity().saturating_mul(size_of::<Value>()))
    }

    fn add_usage(&mut self, (collections, bytes): (usize, usize)) {
        self.resource_collections = self.resource_collections.saturating_add(collections);
        self.resource_heap_bytes = self.resource_heap_bytes.saturating_add(bytes);
    }

    fn subtract_usage(&mut self, (collections, bytes): (usize, usize)) {
        self.resource_collections = self.resource_collections.saturating_sub(collections);
        self.resource_heap_bytes = self.resource_heap_bytes.saturating_sub(bytes);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstancePlan {
    pub inputs: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    pub max_evaluated_nodes: usize,
    pub max_call_depth: usize,
    pub max_heap_bytes: usize,
    pub max_collection_elements: usize,
    /// Maximum rolling-window length accepted by stateful TA intrinsics.
    pub max_window_length: usize,
    pub max_output_items: usize,
    pub max_string_bytes: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_evaluated_nodes: 1_000_000,
            max_call_depth: 128,
            max_heap_bytes: 64 * 1024 * 1024,
            max_collection_elements: 100_000,
            max_window_length: 5_000,
            max_output_items: 1_000_000,
            max_string_bytes: 1 << 20,
        }
    }
}

impl RuntimeLimits {
    pub fn untrusted() -> Self {
        Self {
            max_evaluated_nodes: 250_000,
            max_call_depth: 64,
            max_heap_bytes: 16 * 1024 * 1024,
            max_collection_elements: 25_000,
            max_window_length: 5_000,
            max_output_items: 4_096,
            max_string_bytes: 64 * 1024,
        }
    }
}

impl InstancePlan {
    pub(crate) fn from_compiled(
        compiled: &CompiledIndicator,
        config: IndicatorConfig,
    ) -> Result<Self, PlanningError> {
        let mut inputs = vec![Value::Na; compiled.program.locals];
        for (slot, name) in &compiled.program.inputs {
            let Some(descriptor) = compiled
                .descriptor
                .inputs
                .iter()
                .find(|input| &input.id == name)
            else {
                return Err(PlanningError::MissingInput(name.clone()));
            };
            let configured = config.value(descriptor);
            if !descriptor.accepts(&configured) {
                return Err(PlanningError::InvalidInput(name.clone()));
            }
            let configured = match (descriptor.value_type, configured) {
                (InputType::Float, InputValue::Int(value)) => Value::Float(value as f64),
                (_, configured) => configured.into_value(),
            };
            inputs[slot.0] = configured;
        }
        Ok(Self { inputs })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlanningError {
    #[error("input descriptor is missing for {0}")]
    MissingInput(String),
    #[error("input {0} has a value with the wrong type")]
    InvalidInput(String),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum RuntimeError {
    #[error("unknown expression {0:?}")]
    UnknownExpression(ExprId),
    #[error("unknown intrinsic {0:?}")]
    UnknownIntrinsic(crate::hir::IntrinsicId),
    #[error("invalid value for {operation}")]
    InvalidValue { operation: &'static str },
    #[error("intrinsic error: {0}")]
    Intrinsic(#[from] IntrinsicError),
    #[error("execution limit exceeded")]
    ExecutionLimitExceeded,
    #[error("resource limit exceeded: {0}")]
    ResourceLimitExceeded(&'static str),
    #[error("state slot {0:?} is out of range")]
    UnknownState(StateSlot),
}

#[derive(Debug, Clone, PartialEq)]
struct SeriesHistory {
    values: Vec<IndexedHistory>,
}

impl SeriesHistory {
    fn new(count: usize, capacity: usize) -> Self {
        Self {
            values: (0..count).map(|_| IndexedHistory::new(capacity)).collect(),
        }
    }

    fn get(&self, slot: usize, index: usize) -> Option<Value> {
        self.values.get(slot)?.get(index).cloned()
    }

    fn set(&mut self, slot: usize, index: usize, value: Value) {
        if let Some(values) = self.values.get_mut(slot) {
            values.set(index, value);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CommittedState {
    states: Arc<Vec<StateValue>>,
    function_states: Arc<BTreeMap<crate::hir::CallSiteId, StateValue>>,
    history: SeriesHistory,
    intrinsics: Arc<BTreeMap<crate::hir::CallSiteId, IntrinsicState>>,
}

#[derive(Debug, Clone, PartialEq)]
struct WorkingState {
    states: Arc<Vec<StateValue>>,
    function_states: Arc<BTreeMap<crate::hir::CallSiteId, StateValue>>,
    current_series: Vec<Option<Value>>,
    intrinsics: Arc<BTreeMap<crate::hir::CallSiteId, IntrinsicState>>,
}

#[derive(Debug, Clone, PartialEq)]
enum StateValue {
    Uninitialized,
    Value(Value),
}

impl StateValue {
    fn value(&self) -> Option<&Value> {
        match self {
            Self::Uninitialized => None,
            Self::Value(value) => Some(value),
        }
    }

    fn is_initialized(&self) -> bool {
        matches!(self, Self::Value(_))
    }
}

impl WorkingState {
    fn from_committed(committed: &CommittedState, series_count: usize) -> Self {
        Self {
            states: Arc::clone(&committed.states),
            function_states: Arc::clone(&committed.function_states),
            current_series: vec![None; series_count],
            intrinsics: Arc::clone(&committed.intrinsics),
        }
    }

    fn reset_from_committed(&mut self, committed: &CommittedState, series_count: usize) {
        self.states = Arc::clone(&committed.states);
        self.function_states = Arc::clone(&committed.function_states);
        self.intrinsics = Arc::clone(&committed.intrinsics);
        if self.current_series.len() != series_count {
            self.current_series.clear();
            self.current_series.resize(series_count, None);
        } else {
            self.current_series.fill(None);
        }
    }

    fn clear_current_series(&mut self) {
        self.current_series.fill(None);
    }
}

struct OutputBuffer {
    max_items: usize,
    items: usize,
    plots: Vec<PlotOutput>,
    markers: Vec<MarkerOutput>,
    segments: Vec<SegmentOutput>,
    levels: Vec<HorizontalLevelOutput>,
    ohlc: Vec<OhlcOutput>,
    ranges: Vec<RangeOutput>,
}

impl OutputBuffer {
    fn new(max_items: usize) -> Self {
        Self {
            max_items,
            items: 0,
            plots: Vec::new(),
            markers: Vec::new(),
            segments: Vec::new(),
            levels: Vec::new(),
            ohlc: Vec::new(),
            ranges: Vec::new(),
        }
    }

    fn reserve(&mut self) -> Result<(), RuntimeError> {
        if self.items >= self.max_items {
            return Err(RuntimeError::ResourceLimitExceeded("output items"));
        }
        self.items += 1;
        Ok(())
    }

    fn push_plot(&mut self, value: PlotOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.plots.push(value);
        Ok(())
    }

    fn push_marker(&mut self, value: MarkerOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.markers.push(value);
        Ok(())
    }

    fn push_segment(&mut self, value: SegmentOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.segments.push(value);
        Ok(())
    }

    fn push_level(&mut self, value: HorizontalLevelOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.levels.push(value);
        Ok(())
    }

    fn push_ohlc(&mut self, value: OhlcOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.ohlc.push(value);
        Ok(())
    }

    fn push_range(&mut self, value: RangeOutput) -> Result<(), RuntimeError> {
        self.reserve()?;
        self.ranges.push(value);
        Ok(())
    }

    fn resource_usage(&self) -> (usize, usize) {
        let mut bytes = size_of::<Self>()
            .saturating_add(
                self.plots
                    .capacity()
                    .saturating_mul(size_of::<PlotOutput>()),
            )
            .saturating_add(
                self.markers
                    .capacity()
                    .saturating_mul(size_of::<MarkerOutput>()),
            )
            .saturating_add(
                self.segments
                    .capacity()
                    .saturating_mul(size_of::<SegmentOutput>()),
            )
            .saturating_add(
                self.levels
                    .capacity()
                    .saturating_mul(size_of::<HorizontalLevelOutput>()),
            )
            .saturating_add(self.ohlc.capacity().saturating_mul(size_of::<OhlcOutput>()))
            .saturating_add(
                self.ranges
                    .capacity()
                    .saturating_mul(size_of::<RangeOutput>()),
            );
        for plot in &self.plots {
            bytes = bytes
                .saturating_add(plot.id.capacity())
                .saturating_add(plot.value.resource_usage().1);
        }
        for marker in &self.markers {
            bytes = bytes
                .saturating_add(marker.id.capacity())
                .saturating_add(marker.text.as_ref().map_or(0, String::capacity));
        }
        for segment in &self.segments {
            bytes = bytes
                .saturating_add(segment.id.capacity())
                .saturating_add(segment.text.as_ref().map_or(0, String::capacity));
        }
        for level in &self.levels {
            bytes = bytes.saturating_add(level.id.capacity());
        }
        for ohlc in &self.ohlc {
            bytes = bytes.saturating_add(ohlc.id.capacity());
        }
        for range in &self.ranges {
            bytes = bytes.saturating_add(range.id.capacity());
        }
        (self.items, bytes)
    }

    fn has_string_over(&self, limit: usize) -> bool {
        self.plots
            .iter()
            .any(|plot| plot.id.capacity() > limit || plot.value.has_string_over(limit))
            || self.markers.iter().any(|marker| {
                marker.id.capacity() > limit
                    || marker
                        .text
                        .as_ref()
                        .is_some_and(|text| text.capacity() > limit)
            })
            || self.segments.iter().any(|segment| {
                segment.id.capacity() > limit
                    || segment
                        .text
                        .as_ref()
                        .is_some_and(|text| text.capacity() > limit)
            })
            || self.levels.iter().any(|level| level.id.capacity() > limit)
            || self.ohlc.iter().any(|ohlc| ohlc.id.capacity() > limit)
            || self.ranges.iter().any(|range| range.id.capacity() > limit)
    }

    fn finish(self, index: usize, time: i64) -> FrameOutput {
        FrameOutput {
            index,
            time,
            plots: self.plots.into(),
            markers: self.markers.into(),
            segments: self.segments.into(),
            levels: self.levels.into(),
            ohlc: self.ohlc.into(),
            ranges: self.ranges.into(),
        }
    }
}

pub struct IndicatorInstance {
    compiled: Arc<CompiledIndicator>,
    plan: InstancePlan,
    committed: CommittedState,
    working: WorkingState,
    locals: Vec<Value>,
    current_bar_index: usize,
    current_frame: Option<IndicatorFrame>,
    executed_nodes: usize,
    registry: IntrinsicRegistry,
    previewing: bool,
    limits: RuntimeLimits,
    break_requested: bool,
    continue_requested: bool,
    return_requested: bool,
    return_value: Option<Value>,
    call_depth: usize,
    call_site_stack: Vec<crate::hir::CallSiteId>,
    max_bars_back: usize,
    quarantined: Option<RuntimeError>,
    resource_check_dirty: bool,
}

impl std::fmt::Debug for IndicatorInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndicatorInstance")
            .field("indicator", &self.compiled.descriptor.id)
            .field("bar_index", &self.current_bar_index)
            .finish_non_exhaustive()
    }
}

impl IndicatorInstance {
    pub(crate) fn with_limits(
        compiled: Arc<CompiledIndicator>,
        plan: InstancePlan,
        limits: RuntimeLimits,
    ) -> Self {
        let committed = CommittedState {
            states: Arc::new(vec![StateValue::Uninitialized; compiled.program.states]),
            function_states: Arc::new(BTreeMap::new()),
            history: SeriesHistory::new(
                compiled.program.series,
                compiled.descriptor.max_bars_back.saturating_add(1),
            ),
            intrinsics: Arc::new(BTreeMap::new()),
        };
        Self {
            working: WorkingState::from_committed(&committed, compiled.program.series),
            locals: vec![Value::Na; compiled.program.locals],
            current_bar_index: 0,
            current_frame: None,
            executed_nodes: 0,
            registry: compiled.registry.clone(),
            previewing: false,
            limits,
            break_requested: false,
            continue_requested: false,
            return_requested: false,
            return_value: None,
            call_depth: 0,
            call_site_stack: Vec::new(),
            max_bars_back: compiled.descriptor.max_bars_back,
            quarantined: None,
            resource_check_dirty: false,
            compiled,
            plan,
            committed,
        }
    }

    fn evaluate_frame(
        &mut self,
        frame: IndicatorFrame,
        previewing: bool,
    ) -> Result<FrameOutput, RuntimeError> {
        if let Some(error) = &self.quarantined {
            return Err(error.clone());
        }

        let index = frame.index;
        let time = frame.time;
        self.current_bar_index = index;
        self.previewing = previewing;
        self.current_frame = Some(frame.clone());
        self.locals.fill(Value::Na);
        self.executed_nodes = 0;
        self.break_requested = false;
        self.continue_requested = false;
        self.return_requested = false;
        self.return_value = None;
        self.working
            .reset_from_committed(&self.committed, self.compiled.program.series);
        self.resource_check_dirty = false;
        self.seed_frame_series(&frame);

        let mut output = OutputBuffer::new(self.limits.max_output_items);
        let result = (|| {
            self.enforce_resource_limits(false)?;
            let entry_point = self
                .compiled
                .program
                .blocks
                .get(self.compiled.program.entry_point.0)
                .map(|block| block.statements.clone())
                .unwrap_or_default();
            for statement_id in entry_point {
                self.execute_statement(statement_id, &mut output)?;
            }
            self.complete_outputs(&mut output)?;
            self.enforce_output_limits(&output)?;
            self.enforce_resource_limits(false)?;
            self.resource_check_dirty = false;
            if !previewing {
                self.commit(index);
                self.enforce_resource_limits(true)?;
            }
            Ok(output.finish(index, time))
        })();

        if let Err(error) = &result
            && matches!(
                error,
                RuntimeError::ExecutionLimitExceeded | RuntimeError::ResourceLimitExceeded(_)
            )
        {
            self.quarantined = Some(error.clone());
        }
        self.working.clear_current_series();
        self.current_frame = None;
        result
    }

    /// Evaluate and commit one closed chart frame.
    pub fn evaluate_closed(&mut self, frame: IndicatorFrame) -> Result<FrameOutput, RuntimeError> {
        self.evaluate_frame(frame, false)
    }

    /// Preview the current open frame from the previous closed state.
    pub fn preview_open(&mut self, frame: IndicatorFrame) -> Result<FrameOutput, RuntimeError> {
        self.evaluate_frame(frame, true)
    }

    /// Discard all committed values and bind a new input configuration.
    ///
    /// The caller must recalculate the loaded closed frames before requesting
    /// an open preview.
    pub fn reset(&mut self, config: IndicatorConfig) -> Result<(), PlanningError> {
        let plan = InstancePlan::from_compiled(&self.compiled, config)?;
        let replacement = Self::with_limits(self.compiled.clone(), plan, self.limits);
        *self = replacement;
        Ok(())
    }

    fn seed_frame_series(&mut self, frame: &IndicatorFrame) {
        for expression in &self.compiled.program.expressions {
            let Some(slot) = expression.series_slot else {
                continue;
            };
            let value = match &expression.kind {
                ExpressionKind::LoadFrame(field) => Some(frame.primary_value(*field)),
                _ => None,
            };
            let value = match &expression.kind {
                ExpressionKind::LoadSeriesInput(slot) => {
                    Some(self.series_input_value(*slot, frame))
                }
                _ => value,
            };
            if let Some(value) = value
                && let Some(current) = self.working.current_series.get_mut(slot.0)
            {
                *current = Some(value);
            }
        }
    }

    fn series_input_value(&self, slot: SourceInputSlot, frame: &IndicatorFrame) -> Value {
        let Some((_, input_id)) = self
            .compiled
            .program
            .source_inputs
            .iter()
            .find(|(candidate, _)| *candidate == slot)
        else {
            return Value::Na;
        };
        let Some(descriptor) = self
            .compiled
            .descriptor
            .series_inputs
            .iter()
            .find(|descriptor| descriptor.id == *input_id)
        else {
            return Value::Na;
        };
        match frame.series_input(slot.0) {
            SeriesInputValue::Default => frame.primary_value(descriptor.default_field),
            SeriesInputValue::Base(source) => source.value(frame).map_or(Value::Na, Value::Float),
            SeriesInputValue::Value(Some(value)) if value.is_finite() => Value::Float(value),
            SeriesInputValue::Value(Some(_)) => Value::Na,
            SeriesInputValue::Value(None) => Value::Na,
        }
    }

    fn complete_outputs(&self, output: &mut OutputBuffer) -> Result<(), RuntimeError> {
        for plot in &self.compiled.program.plots {
            if !output.plots.iter().any(|output| output.id == plot.id) {
                output.push_plot(PlotOutput {
                    id: plot.id.clone(),
                    value: Value::Na,
                    color: None,
                })?;
            }
        }

        for statement in &self.compiled.program.statements {
            match &statement.kind {
                StatementKind::Marker { id, .. }
                    if !output.markers.iter().any(|output| output.id == *id) =>
                {
                    output.push_marker(MarkerOutput {
                        id: id.clone(),
                        value: None,
                        color: None,
                        text: None,
                    })?;
                }
                StatementKind::HorizontalLevel { id, .. }
                    if !output.levels.iter().any(|output| output.id == *id) =>
                {
                    output.push_level(HorizontalLevelOutput {
                        id: id.clone(),
                        value: None,
                        color: None,
                    })?;
                }
                StatementKind::OhlcPlot { plot, .. }
                    if self.compiled.program.plots.get(plot.0).is_some_and(|plot| {
                        !output.ohlc.iter().any(|output| output.id == plot.id)
                    }) =>
                {
                    if let Some(plot) = self.compiled.program.plots.get(plot.0) {
                        output.push_ohlc(OhlcOutput {
                            id: plot.id.clone(),
                            open: None,
                            high: None,
                            low: None,
                            close: None,
                            color: None,
                        })?;
                    }
                }
                StatementKind::RangePlot { plot, .. }
                    if self.compiled.program.plots.get(plot.0).is_some_and(|plot| {
                        !output.ranges.iter().any(|output| output.id == plot.id)
                    }) =>
                {
                    if let Some(plot) = self.compiled.program.plots.get(plot.0) {
                        output.push_range(RangeOutput {
                            id: plot.id.clone(),
                            upper: None,
                            lower: None,
                            color: None,
                        })?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn checkpoint(&mut self, output: &OutputBuffer) -> Result<(), RuntimeError> {
        self.enforce_output_limits(output)?;
        if self.resource_check_dirty {
            self.enforce_resource_limits(false)?;
            self.resource_check_dirty = false;
        }
        Ok(())
    }

    fn evaluate_color(
        &mut self,
        expression: Option<ExprId>,
    ) -> Result<Option<Color>, RuntimeError> {
        expression
            .map(|expression| self.evaluate_expression(expression))
            .transpose()
            .map(|value| match value {
                Some(Value::Color(color)) => Some(Color(color)),
                _ => None,
            })
    }

    fn execute_statement(
        &mut self,
        statement_id: crate::hir::StatementId,
        output: &mut OutputBuffer,
    ) -> Result<(), RuntimeError> {
        let statement = self
            .compiled
            .program
            .statements
            .get(statement_id.0)
            .ok_or(RuntimeError::UnknownExpression(ExprId(statement_id.0)))?
            .clone();
        self.charge()?;
        match statement.kind {
            StatementKind::Binding { target, expression } => {
                let value = self.evaluate_expression(expression)?;
                self.locals[target.0] = value;
            }
            StatementKind::LocalUpdate { target, expression } => {
                let value = self.evaluate_expression(expression)?;
                if let Some(local) = self.locals.get_mut(target.0) {
                    *local = value;
                }
            }
            StatementKind::Destructure {
                targets,
                expression,
            } => {
                let value = self.evaluate_expression(expression)?;
                let values = match value {
                    Value::Tuple(values) | Value::Array(values) => values,
                    _ => Vec::new(),
                };
                for (index, target) in targets.into_iter().enumerate() {
                    if let Some(local) = self.locals.get_mut(target.0) {
                        *local = values.get(index).cloned().unwrap_or(Value::Na);
                    }
                }
            }
            StatementKind::StateDeclaration {
                target, initial, ..
            } => {
                if !self.state_initialized(target) {
                    let value = self.evaluate_expression(initial)?;
                    if !value.is_na() {
                        self.set_state_value(target, value);
                    }
                }
            }
            StatementKind::StateUpdate { target, expression } => {
                let value = self.evaluate_expression(expression)?;
                self.set_state_value(target, value);
            }
            StatementKind::Plot {
                plot,
                expression,
                color,
            } => {
                let value = self.evaluate_expression(expression)?;
                let Some(plot_id) = self
                    .compiled
                    .program
                    .plots
                    .get(plot.0)
                    .map(|descriptor| descriptor.id.clone())
                else {
                    return Ok(());
                };
                let color = self.evaluate_color(color)?;
                output.push_plot(PlotOutput {
                    id: plot_id,
                    value,
                    color,
                })?;
            }
            StatementKind::Marker {
                id,
                expression,
                visible,
                color,
                text,
            } => {
                let value = self.evaluate_expression(expression)?;
                let visible = visible
                    .map(|expression| self.evaluate_expression(expression))
                    .transpose()?
                    .is_none_or(|value| value.is_truthy());
                let value = if !visible {
                    None
                } else if matches!(value, Value::Bool(true)) {
                    self.current_frame
                        .as_ref()
                        .and_then(|frame| frame.chart.close)
                } else if matches!(value, Value::Bool(false)) {
                    None
                } else {
                    value.as_float()
                };
                output.push_marker(MarkerOutput {
                    id,
                    value,
                    color: self.evaluate_color(color)?,
                    text,
                })?;
            }
            StatementKind::Segment {
                id,
                start_value,
                end_value,
                start_index,
                end_index,
                visible,
                color,
                text,
            } => {
                let visible = visible
                    .map(|expression| self.evaluate_expression(expression))
                    .transpose()?
                    .is_none_or(|value| value.is_truthy());
                if !visible {
                    return Ok(());
                }
                let Some(start_index) = self
                    .evaluate_expression(start_index)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let Some(end_index) = self
                    .evaluate_expression(end_index)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let (Some(start_value), Some(end_value)) = (
                    self.evaluate_expression(start_value)?.as_float(),
                    self.evaluate_expression(end_value)?.as_float(),
                ) else {
                    return Ok(());
                };
                output.push_segment(SegmentOutput {
                    id,
                    start_index,
                    start_value: start_value.is_finite().then_some(start_value),
                    end_index,
                    end_value: end_value.is_finite().then_some(end_value),
                    color: self.evaluate_color(color)?,
                    text,
                })?;
            }
            StatementKind::HorizontalLevel {
                id,
                expression,
                color,
            } => {
                output.push_level(crate::frame::HorizontalLevelOutput {
                    id,
                    value: self.evaluate_expression(expression)?.as_float(),
                    color: self.evaluate_color(color)?,
                })?;
            }
            StatementKind::OhlcPlot {
                plot,
                open,
                high,
                low,
                close,
                color,
            } => {
                let Some(descriptor) = self.compiled.program.plots.get(plot.0) else {
                    return Ok(());
                };
                output.push_ohlc(OhlcOutput {
                    id: descriptor.id.clone(),
                    open: self.evaluate_expression(open)?.as_float(),
                    high: self.evaluate_expression(high)?.as_float(),
                    low: self.evaluate_expression(low)?.as_float(),
                    close: self.evaluate_expression(close)?.as_float(),
                    color: self.evaluate_color(color)?,
                })?;
            }
            StatementKind::RangePlot {
                plot,
                upper,
                lower,
                color,
            } => {
                let Some(descriptor) = self.compiled.program.plots.get(plot.0) else {
                    return Ok(());
                };
                output.push_range(RangeOutput {
                    id: descriptor.id.clone(),
                    upper: self.evaluate_expression(upper)?.as_float(),
                    lower: self.evaluate_expression(lower)?.as_float(),
                    color: self.evaluate_color(color)?,
                })?;
            }
            StatementKind::Expression(expression) => {
                let _ = self.evaluate_expression(expression)?;
            }
            StatementKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let branch = if self.evaluate_expression(condition)?.is_truthy() {
                    then_branch
                } else {
                    else_branch
                };
                for statement in branch {
                    self.execute_statement(statement, output)?;
                    if self.return_requested || self.break_requested || self.continue_requested {
                        break;
                    }
                }
                self.checkpoint(output)?;
            }
            StatementKind::While { condition, body } => {
                let mut iterations = 0usize;
                loop {
                    if iterations >= self.limits.max_evaluated_nodes {
                        return Err(RuntimeError::ExecutionLimitExceeded);
                    }
                    if !self.evaluate_expression(condition)?.is_truthy() {
                        break;
                    }
                    for statement in &body {
                        self.execute_statement(*statement, output)?;
                        if self.return_requested || self.break_requested || self.continue_requested
                        {
                            break;
                        }
                    }
                    self.checkpoint(output)?;
                    if self.return_requested {
                        break;
                    }
                    if self.break_requested {
                        self.break_requested = false;
                        break;
                    }
                    if self.continue_requested {
                        self.continue_requested = false;
                    }
                    iterations = iterations
                        .checked_add(1)
                        .ok_or(RuntimeError::ExecutionLimitExceeded)?;
                }
            }
            StatementKind::ForRange {
                target,
                start,
                end,
                body,
            } => {
                let mut index = self.evaluate_expression(start)?.as_int().unwrap_or(0);
                let limit = self.evaluate_expression(end)?.as_int().unwrap_or(-1);
                let mut iterations = 0usize;
                while index <= limit {
                    if iterations >= self.limits.max_evaluated_nodes {
                        return Err(RuntimeError::ExecutionLimitExceeded);
                    }
                    if let Some(value) = self.locals.get_mut(target.0) {
                        *value = Value::Int(index);
                    }
                    for statement in &body {
                        self.execute_statement(*statement, output)?;
                        if self.return_requested || self.break_requested || self.continue_requested
                        {
                            break;
                        }
                    }
                    self.checkpoint(output)?;
                    if self.return_requested {
                        break;
                    }
                    if self.break_requested {
                        self.break_requested = false;
                        break;
                    }
                    self.continue_requested = false;
                    index = index
                        .checked_add(1)
                        .ok_or(RuntimeError::ExecutionLimitExceeded)?;
                    iterations = iterations
                        .checked_add(1)
                        .ok_or(RuntimeError::ExecutionLimitExceeded)?;
                }
            }
            StatementKind::ForEach {
                target,
                collection,
                body,
            } => {
                let values = match self.evaluate_expression(collection)? {
                    Value::Array(values) | Value::Tuple(values) => values,
                    _ => Vec::new(),
                };
                for value in values {
                    if let Some(local) = self.locals.get_mut(target.0) {
                        *local = value;
                    }
                    for statement in &body {
                        self.execute_statement(*statement, output)?;
                        if self.return_requested || self.break_requested || self.continue_requested
                        {
                            break;
                        }
                    }
                    self.checkpoint(output)?;
                    if self.return_requested {
                        break;
                    }
                    if self.break_requested {
                        self.break_requested = false;
                        break;
                    }
                    self.continue_requested = false;
                }
            }
            StatementKind::Break => self.break_requested = true,
            StatementKind::Continue => self.continue_requested = true,
            StatementKind::Return(expression) => {
                self.return_value = expression
                    .map(|expression| self.evaluate_expression(expression))
                    .transpose()?;
                self.return_requested = true;
            }
        }
        Ok(())
    }

    fn evaluate_expression(&mut self, expression_id: ExprId) -> Result<Value, RuntimeError> {
        self.charge()?;
        let expression = self
            .compiled
            .program
            .expressions
            .get(expression_id.0)
            .ok_or(RuntimeError::UnknownExpression(expression_id))?
            .clone();
        let value = match expression.kind {
            ExpressionKind::Constant(constant) => self
                .compiled
                .program
                .constants
                .get(constant.0)
                .cloned()
                .ok_or(RuntimeError::UnknownExpression(expression_id))?,
            ExpressionKind::LoadLocal(slot) => {
                self.locals.get(slot.0).cloned().unwrap_or(Value::Na)
            }
            ExpressionKind::LoadInput(slot) => {
                self.plan.inputs.get(slot.0).cloned().unwrap_or(Value::Na)
            }
            ExpressionKind::LoadSeriesInput(slot) => self
                .current_frame
                .as_ref()
                .map_or(Value::Na, |frame| self.series_input_value(slot, frame)),
            ExpressionKind::LoadState(slot) => self.state_value(slot).unwrap_or(Value::Na),
            ExpressionKind::LoadBarIndex => Value::Int(self.current_bar_index as i64),
            ExpressionKind::LoadBarState(field) => Value::Bool(match field {
                BarStateField::IsFirst => self.current_bar_index == 0,
                BarStateField::IsLast => self.previewing,
                BarStateField::IsConfirmed => !self.previewing,
                BarStateField::IsRealtime => self.previewing,
            }),
            ExpressionKind::Tuple(expressions) => Value::Tuple(
                expressions
                    .iter()
                    .map(|expression| self.evaluate_expression(*expression))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            ExpressionKind::Array(expressions) => Value::Array(
                expressions
                    .iter()
                    .map(|expression| self.evaluate_expression(*expression))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            ExpressionKind::Record(fields) => {
                let mut values = BTreeMap::new();
                for (name, expression) in fields {
                    values.insert(name, self.evaluate_expression(expression)?);
                }
                Value::Record(values)
            }
            ExpressionKind::Switch {
                operand,
                branches,
                fallback,
            } => {
                let operand = self.evaluate_expression(operand)?;

                let mut selected = None;
                for (condition, result) in branches {
                    let condition = self.evaluate_expression(condition)?;
                    if operand.equals(&condition).is_some_and(|equal| equal) {
                        selected = Some(self.evaluate_expression(result)?);
                        break;
                    }
                }
                if let Some(value) = selected {
                    value
                } else if let Some(fallback) = fallback {
                    self.evaluate_expression(fallback)?
                } else {
                    Value::Na
                }
            }
            ExpressionKind::LoadSeries(slot) => self
                .working
                .current_series
                .get(slot.0)
                .and_then(Clone::clone)
                .unwrap_or(Value::Na),
            ExpressionKind::LoadFrame(field) => self
                .current_frame
                .as_ref()
                .map(|frame| frame.primary_value(field))
                .unwrap_or(Value::Na),
            ExpressionKind::RecordField { source, field } => {
                let source = self.evaluate_expression(source)?;
                match source {
                    Value::Record(values) => values.get(&field).cloned().unwrap_or(Value::Na),
                    _ => Value::Na,
                }
            }
            ExpressionKind::History { source, offset } => {
                let offset = self.evaluate_expression(offset)?.as_int().unwrap_or(-1);
                if offset < 0 {
                    Value::Na
                } else if offset == 0 {
                    self.evaluate_expression(source)?
                } else {
                    let offset = offset as usize;
                    if offset > self.current_bar_index {
                        Value::Na
                    } else {
                        let target = self.current_bar_index - offset;
                        self.compiled.program.expressions[source.0]
                            .series_slot
                            .and_then(|slot| self.committed.history.get(slot.0, target))
                            .unwrap_or(Value::Na)
                    }
                }
            }
            ExpressionKind::CollectionIndex { source, index } => {
                let source = self.evaluate_expression(source)?;
                let index = self.evaluate_expression(index)?.as_int();
                match (source, index.and_then(|index| usize::try_from(index).ok())) {
                    (Value::Tuple(values), Some(index)) | (Value::Array(values), Some(index)) => {
                        values.get(index).cloned().unwrap_or(Value::Na)
                    }
                    _ => Value::Na,
                }
            }
            ExpressionKind::Unary { operator, operand } => {
                let operand = self.evaluate_expression(operand)?;
                unary(operator, operand)?
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => {
                let left = self.evaluate_expression(left)?;
                let short_circuit = matches!(
                    (&left, operator),
                    (Value::Bool(false), BinaryOperator::And)
                        | (Value::Bool(true), BinaryOperator::Or)
                );
                if short_circuit {
                    left
                } else {
                    let right = self.evaluate_expression(right)?;
                    binary(operator, left, right)?
                }
            }
            ExpressionKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                if self.evaluate_expression(condition)?.is_truthy() {
                    self.evaluate_expression(when_true)?
                } else {
                    self.evaluate_expression(when_false)?
                }
            }
            ExpressionKind::Coalesce { source, fallback } => {
                let source = self.evaluate_expression(source)?;
                if source.is_na() {
                    self.evaluate_expression(fallback)?
                } else {
                    source
                }
            }
            ExpressionKind::IntrinsicCall {
                intrinsic,
                call_site,
                arguments,
            } => {
                let values = arguments
                    .iter()
                    .map(|argument| self.evaluate_expression(*argument))
                    .collect::<Result<Vec<_>, _>>()?;
                self.charge_by(values.iter().fold(0usize, |work, value| {
                    work.saturating_add(value.resource_usage().0)
                }))?;
                let definition = self
                    .registry
                    .get(intrinsic)
                    .ok_or(RuntimeError::UnknownIntrinsic(intrinsic))?;
                let evaluator = definition.evaluator;
                if definition.effects.stateful {
                    self.resource_check_dirty = true;
                }
                let state_call_site = self.state_call_site(call_site);
                if let Some(factory) = definition.state_factory
                    && !self.working.intrinsics.contains_key(&state_call_site)
                {
                    Arc::make_mut(&mut self.working.intrinsics).insert(state_call_site, factory());
                }
                let (result, state_heap_exceeded) = {
                    let mut context = IntrinsicEvalContext {
                        bar_index: self.current_bar_index,
                        bar_time: self.current_frame.as_ref().map_or(0, |frame| frame.time),
                        max_heap_bytes: self.limits.max_heap_bytes,
                        max_window_length: self.limits.max_window_length,
                        max_collection_elements: self.limits.max_collection_elements,
                        states: Arc::make_mut(&mut self.working.intrinsics),
                    };
                    let result = evaluator(&mut context, state_call_site, &values);
                    let state_heap_exceeded = context
                        .states
                        .values()
                        .map(IntrinsicState::heap_bytes)
                        .fold(0usize, usize::saturating_add)
                        > context.max_heap_bytes;
                    (result, state_heap_exceeded)
                };
                if state_heap_exceeded {
                    return Err(RuntimeError::ResourceLimitExceeded("heap bytes"));
                }
                match result {
                    Ok(value) => value,
                    Err(crate::intrinsic::IntrinsicError::LimitExceeded(resource)) => {
                        return Err(RuntimeError::ResourceLimitExceeded(resource));
                    }
                    Err(error) => return Err(RuntimeError::Intrinsic(error)),
                }
            }
            ExpressionKind::FunctionCall {
                function,
                call_site,
                arguments,
                argument_slots,
                ..
            } => {
                if self.call_depth >= self.limits.max_call_depth {
                    return Err(RuntimeError::ExecutionLimitExceeded);
                }
                let function = self
                    .compiled
                    .program
                    .functions
                    .get(function.0)
                    .cloned()
                    .ok_or(RuntimeError::InvalidValue {
                        operation: "function call",
                    })?;
                let values = arguments
                    .iter()
                    .map(|argument| self.evaluate_expression(*argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let saved_locals = self.locals.clone();
                let saved_return_requested = self.return_requested;
                let saved_return_value = self.return_value.take();
                self.return_requested = false;
                self.call_depth += 1;
                self.call_site_stack.push(call_site);
                for slot in &function.local_slots {
                    if let Some(value) = self.locals.get_mut(slot.0) {
                        *value = Value::Na;
                    }
                }
                let mut provided = vec![false; function.parameters.len()];
                for (parameter_index, value) in argument_slots.into_iter().zip(values) {
                    let Some(slot) = function.parameters.get(parameter_index) else {
                        continue;
                    };
                    if let Some(local) = self.locals.get_mut(slot.0) {
                        *local = value;
                        provided[parameter_index] = true;
                    }
                }
                let mut nested_output = OutputBuffer::new(self.limits.max_output_items);
                let execution = (|| {
                    for (parameter_index, default) in function.parameter_defaults.iter().enumerate()
                    {
                        if provided.get(parameter_index).copied().unwrap_or(false) {
                            continue;
                        }
                        let Some(default) = default else {
                            continue;
                        };
                        let value = self.evaluate_expression(*default)?;
                        if let Some(slot) = function.parameters.get(parameter_index)
                            && let Some(local) = self.locals.get_mut(slot.0)
                        {
                            *local = value;
                        }
                    }
                    for statement in function.body {
                        self.execute_statement(statement, &mut nested_output)?;
                        if self.return_requested {
                            break;
                        }
                    }
                    self.checkpoint(&nested_output)?;
                    Ok::<(), RuntimeError>(())
                })();
                self.call_depth = self.call_depth.saturating_sub(1);
                self.call_site_stack.pop();
                let result = self.return_value.take().unwrap_or(Value::Na);
                self.locals = saved_locals;
                self.return_requested = saved_return_requested;
                self.return_value = saved_return_value;
                execution?;
                result
            }
        };
        let value = value.normalize_nonfinite();
        self.enforce_value_limits(&value)?;
        if let Some(slot) = expression.series_slot
            && let Some(current) = self.working.current_series.get_mut(slot.0)
        {
            *current = Some(value.clone());
        }
        Ok(value)
    }

    fn state_value(&self, slot: StateSlot) -> Option<Value> {
        let call_site = self
            .compiled
            .program
            .state_call_sites
            .get(slot.0)
            .copied()
            .unwrap_or(crate::hir::CallSiteId(slot.0));
        if self.state_owner(slot).is_some() && !self.call_site_stack.is_empty() {
            return self
                .working
                .function_states
                .get(&self.state_call_site(call_site))
                .and_then(|value| value.value().cloned());
        }
        self.working
            .states
            .get(slot.0)
            .and_then(|value| value.value().cloned())
    }

    fn state_initialized(&self, slot: StateSlot) -> bool {
        let call_site = self
            .compiled
            .program
            .state_call_sites
            .get(slot.0)
            .copied()
            .unwrap_or(crate::hir::CallSiteId(slot.0));
        if self.state_owner(slot).is_some() && !self.call_site_stack.is_empty() {
            return self
                .working
                .function_states
                .get(&self.state_call_site(call_site))
                .is_some_and(StateValue::is_initialized);
        }
        self.working
            .states
            .get(slot.0)
            .is_some_and(StateValue::is_initialized)
    }

    fn set_state_value(&mut self, slot: StateSlot, value: Value) {
        let call_site = self
            .compiled
            .program
            .state_call_sites
            .get(slot.0)
            .copied()
            .unwrap_or(crate::hir::CallSiteId(slot.0));
        self.resource_check_dirty = true;
        if self.state_owner(slot).is_some() && !self.call_site_stack.is_empty() {
            let state_call_site = self.state_call_site(call_site);
            Arc::make_mut(&mut self.working.function_states)
                .insert(state_call_site, StateValue::Value(value));
            return;
        }
        if let Some(state) = Arc::make_mut(&mut self.working.states).get_mut(slot.0) {
            *state = StateValue::Value(value);
        }
    }

    fn state_owner(&self, slot: StateSlot) -> Option<crate::hir::FunctionId> {
        self.compiled
            .program
            .state_functions
            .get(slot.0)
            .copied()
            .flatten()
    }

    fn state_call_site(&self, call_site: crate::hir::CallSiteId) -> crate::hir::CallSiteId {
        if self.call_site_stack.is_empty() {
            return call_site;
        }
        let mut value = call_site.0.wrapping_add(1);
        for parent in &self.call_site_stack {
            value = value
                .wrapping_mul(1_000_003)
                .wrapping_add(parent.0.wrapping_add(1));
        }
        crate::hir::CallSiteId(value)
    }

    fn charge(&mut self) -> Result<(), RuntimeError> {
        self.charge_by(1)
    }

    fn charge_by(&mut self, work: usize) -> Result<(), RuntimeError> {
        self.executed_nodes = self
            .executed_nodes
            .checked_add(work)
            .ok_or(RuntimeError::ExecutionLimitExceeded)?;
        if self.executed_nodes > self.limits.max_evaluated_nodes {
            Err(RuntimeError::ExecutionLimitExceeded)
        } else {
            Ok(())
        }
    }

    fn enforce_resource_limits(&self, include_history: bool) -> Result<(), RuntimeError> {
        match self.resource_limit_exceeded(include_history) {
            Some(resource) => Err(RuntimeError::ResourceLimitExceeded(resource)),
            None => Ok(()),
        }
    }

    fn enforce_output_limits(&self, output: &OutputBuffer) -> Result<(), RuntimeError> {
        if output.has_string_over(self.limits.max_string_bytes) {
            return Err(RuntimeError::ResourceLimitExceeded("string bytes"));
        }
        let (_, heap_bytes) = output.resource_usage();
        if heap_bytes > self.limits.max_heap_bytes {
            return Err(RuntimeError::ResourceLimitExceeded("heap bytes"));
        }
        Ok(())
    }

    fn enforce_value_limits(&mut self, value: &Value) -> Result<(), RuntimeError> {
        if value.has_string_over(self.limits.max_string_bytes) {
            return Err(RuntimeError::ResourceLimitExceeded("string bytes"));
        }
        let (collections, heap_bytes) = value.resource_usage();
        if collections > 0 || heap_bytes > 0 {
            self.resource_check_dirty = true;
        }
        if collections > self.limits.max_collection_elements {
            return Err(RuntimeError::ResourceLimitExceeded("collection elements"));
        }
        if heap_bytes > self.limits.max_heap_bytes {
            return Err(RuntimeError::ResourceLimitExceeded("heap bytes"));
        }
        Ok(())
    }

    fn resource_limit_exceeded(&self, include_history: bool) -> Option<&'static str> {
        let shared_states = Arc::ptr_eq(&self.committed.states, &self.working.states);
        let shared_function_states = Arc::ptr_eq(
            &self.committed.function_states,
            &self.working.function_states,
        );
        let shared_intrinsics = Arc::ptr_eq(&self.committed.intrinsics, &self.working.intrinsics);
        let mut collections = 0usize;
        let mut heap_bytes = size_of::<CommittedState>()
            .saturating_add(size_of::<WorkingState>())
            .saturating_add(
                self.plan
                    .inputs
                    .capacity()
                    .saturating_mul(size_of::<Value>()),
            )
            .saturating_add(self.locals.capacity().saturating_mul(size_of::<Value>()))
            .saturating_add(
                self.committed
                    .states
                    .capacity()
                    .saturating_mul(size_of::<StateValue>()),
            )
            .saturating_add(
                self.working
                    .current_series
                    .capacity()
                    .saturating_mul(size_of::<Option<Value>>()),
            )
            .saturating_add(
                self.committed
                    .history
                    .values
                    .capacity()
                    .saturating_mul(size_of::<IndexedHistory>()),
            )
            .saturating_add(
                self.call_site_stack
                    .capacity()
                    .saturating_mul(size_of::<crate::hir::CallSiteId>()),
            );
        if !shared_states {
            heap_bytes = heap_bytes.saturating_add(
                self.working
                    .states
                    .capacity()
                    .saturating_mul(size_of::<StateValue>()),
            );
        }
        heap_bytes = heap_bytes
            .saturating_add(size_of::<BTreeMap<crate::hir::CallSiteId, StateValue>>())
            .saturating_add(
                self.committed
                    .function_states
                    .len()
                    .saturating_mul(size_of::<(crate::hir::CallSiteId, StateValue)>()),
            );
        if !shared_function_states {
            heap_bytes = heap_bytes
                .saturating_add(size_of::<BTreeMap<crate::hir::CallSiteId, StateValue>>())
                .saturating_add(
                    self.working
                        .function_states
                        .len()
                        .saturating_mul(size_of::<(crate::hir::CallSiteId, StateValue)>()),
                );
        }
        heap_bytes = heap_bytes
            .saturating_add(size_of::<BTreeMap<crate::hir::CallSiteId, IntrinsicState>>())
            .saturating_add(
                self.committed
                    .intrinsics
                    .len()
                    .saturating_mul(size_of::<(crate::hir::CallSiteId, IntrinsicState)>()),
            );
        if !shared_intrinsics {
            heap_bytes = heap_bytes
                .saturating_add(size_of::<BTreeMap<crate::hir::CallSiteId, IntrinsicState>>())
                .saturating_add(
                    self.working
                        .intrinsics
                        .len()
                        .saturating_mul(size_of::<(crate::hir::CallSiteId, IntrinsicState)>()),
                );
        }
        let mut inspect_value = |value: &Value| -> Option<&'static str> {
            if value.has_string_over(self.limits.max_string_bytes) {
                return Some("string bytes");
            }
            let (value_collections, value_bytes) = value.resource_usage();
            collections = collections.saturating_add(value_collections);
            heap_bytes = heap_bytes.saturating_add(value_bytes);
            None
        };

        for value in self.plan.inputs.iter().chain(self.locals.iter()) {
            if inspect_value(value).is_some() {
                return Some("string bytes");
            }
        }
        for value in self.committed.states.iter().filter_map(StateValue::value) {
            if inspect_value(value).is_some() {
                return Some("string bytes");
            }
        }
        for value in self
            .committed
            .function_states
            .values()
            .filter_map(StateValue::value)
        {
            if inspect_value(value).is_some() {
                return Some("string bytes");
            }
        }
        if !shared_states {
            for value in self.working.states.iter().filter_map(StateValue::value) {
                if inspect_value(value).is_some() {
                    return Some("string bytes");
                }
            }
        }
        if !shared_function_states {
            for value in self
                .working
                .function_states
                .values()
                .filter_map(StateValue::value)
            {
                if inspect_value(value).is_some() {
                    return Some("string bytes");
                }
            }
        }
        for value in self.working.current_series.iter().flatten() {
            if inspect_value(value).is_some() {
                return Some("string bytes");
            }
        }

        if include_history {
            for usage in self
                .committed
                .history
                .values
                .iter()
                .map(IndexedHistory::resource_usage)
            {
                let (value_collections, value_bytes) = usage;
                collections = collections.saturating_add(value_collections);
                heap_bytes = heap_bytes.saturating_add(value_bytes);
            }
            heap_bytes = self
                .committed
                .history
                .values
                .iter()
                .fold(heap_bytes, |total, history| {
                    total.saturating_add(history.container_heap_bytes())
                });
        }

        for state in self.committed.intrinsics.values() {
            heap_bytes = heap_bytes.saturating_add(state.heap_bytes());
        }
        if !shared_intrinsics {
            for state in self.working.intrinsics.values() {
                heap_bytes = heap_bytes.saturating_add(state.heap_bytes());
            }
        }

        if collections > self.limits.max_collection_elements {
            Some("collection elements")
        } else if heap_bytes > self.limits.max_heap_bytes {
            Some("heap bytes")
        } else {
            None
        }
    }

    fn commit(&mut self, bar_index: usize) {
        for (slot, value) in self.working.current_series.iter().enumerate() {
            self.committed
                .history
                .set(slot, bar_index, value.clone().unwrap_or(Value::Na));
        }
        self.committed.states = Arc::clone(&self.working.states);
        self.committed.function_states = Arc::clone(&self.working.function_states);
        self.committed.intrinsics = Arc::clone(&self.working.intrinsics);
        self.working.clear_current_series();
        let first_retained = bar_index.saturating_sub(self.max_bars_back);
        for values in &mut self.committed.history.values {
            values.evict_before(first_retained);
        }
    }
}

fn unary(operator: crate::hir::UnaryOperator, value: Value) -> Result<Value, RuntimeError> {
    match operator {
        crate::hir::UnaryOperator::Plus => match value {
            Value::Int(_) | Value::Na => Ok(value),
            Value::Float(value) => Ok(Value::Float(value).normalize_nonfinite()),
            _ => Err(RuntimeError::InvalidValue {
                operation: "unary plus",
            }),
        },
        crate::hir::UnaryOperator::Minus => match value {
            Value::Int(value) => Ok(value.checked_neg().map(Value::Int).unwrap_or(Value::Na)),
            Value::Float(value) => Ok(Value::Float(-value).normalize_nonfinite()),
            Value::Na => Ok(Value::Na),
            _ => Err(RuntimeError::InvalidValue {
                operation: "unary minus",
            }),
        },
        crate::hir::UnaryOperator::Not => match value {
            Value::Bool(value) => Ok(Value::Bool(!value)),
            Value::Na => Ok(Value::Na),
            _ => Err(RuntimeError::InvalidValue {
                operation: "logical not",
            }),
        },
    }
}

fn binary(operator: BinaryOperator, left: Value, right: Value) -> Result<Value, RuntimeError> {
    use BinaryOperator::*;
    match operator {
        And | Or => {
            let (Value::Bool(left), Value::Bool(right)) = (left, right) else {
                return Ok(Value::Na);
            };
            Ok(Value::Bool(if matches!(operator, And) {
                left && right
            } else {
                left || right
            }))
        }
        Equal | NotEqual => {
            let equal = match left.equals(&right) {
                Some(equal) => equal,
                None => return Ok(Value::Na),
            };
            Ok(Value::Bool(if matches!(operator, Equal) {
                equal
            } else {
                !equal
            }))
        }
        Less | LessEqual | Greater | GreaterEqual => {
            let (Some(left), Some(right)) = (left.as_float(), right.as_float()) else {
                return Ok(Value::Na);
            };
            Ok(Value::Bool(match operator {
                Less => left < right,
                LessEqual => left <= right,
                Greater => left > right,
                GreaterEqual => left >= right,
                _ => unreachable!(),
            }))
        }
        Add | Subtract | Multiply | Divide | Remainder => numeric_binary(operator, left, right),
    }
}

fn numeric_binary(
    operator: BinaryOperator,
    left: Value,
    right: Value,
) -> Result<Value, RuntimeError> {
    let (Some(left), Some(right)) = (left.as_float(), right.as_float()) else {
        return Ok(Value::Na);
    };
    let value = match operator {
        BinaryOperator::Add => left + right,
        BinaryOperator::Subtract => left - right,
        BinaryOperator::Multiply => left * right,
        BinaryOperator::Divide if right != 0.0 => left / right,
        BinaryOperator::Remainder if right != 0.0 => left % right,
        BinaryOperator::Divide | BinaryOperator::Remainder => return Ok(Value::Na),
        _ => unreachable!(),
    };
    Ok(Value::Float(value).normalize_nonfinite())
}
