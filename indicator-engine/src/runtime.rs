use crate::CompiledIndicator;
use crate::frame::{
    FrameOutput, HorizontalLevelOutput, IndicatorFrame, MarkerOutput, OhlcOutput, PlotOutput,
    RangeOutput, RegionOutput, RegionUpdate, SegmentOutput, SeriesInputValue,
};
use crate::hir::{
    BarStateField, BinaryOperator, ExprId, ExpressionKind, SourceInputSlot, StateSlot,
    StatementKind,
};
use crate::intrinsic::{IntrinsicError, IntrinsicEvalContext, IntrinsicState, WarmupSpec};
use crate::scene::Color;
use crate::types::{ArrayValue, IndicatorConfig, InputType, InputValue, Value, ValueType};
use std::collections::{BTreeMap, VecDeque};
use std::mem::size_of;
use std::sync::Arc;
use thiserror::Error;

const MAX_COLLECTION_NESTING: usize = 256;

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
    /// Weighted work units per frame, including scans and copies.
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
    #[error("output {output} field {field} expects {expected}, got {actual}")]
    InvalidOutput {
        output: String,
        field: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
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

    fn get(&self, slot: usize, index: usize) -> Option<&Value> {
        self.values.get(slot)?.get(index)
    }

    fn set(&mut self, slot: usize, index: usize, value: Value) {
        if let Some(values) = self.values.get_mut(slot) {
            values.set(index, value);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CommittedState {
    index: Option<usize>,
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
    regions: Vec<RegionOutput>,
    region_updates: Vec<RegionUpdate>,
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
            regions: Vec::new(),
            region_updates: Vec::new(),
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

    fn push_region(&mut self, value: RegionOutput) -> Result<(), RuntimeError> {
        if let Some(existing) = self
            .regions
            .iter_mut()
            .find(|region| region.id == value.id && region.key == value.key)
        {
            *existing = value;
            return Ok(());
        }
        self.reserve()?;
        self.regions.push(value);
        Ok(())
    }

    fn push_region_update(&mut self, value: RegionUpdate) -> Result<(), RuntimeError> {
        let (id, key) = match &value {
            RegionUpdate::Upsert(region) => (&region.id, region.key),
            RegionUpdate::Remove { id, key } => (id, *key),
        };
        if let Some(existing) = self.region_updates.iter_mut().find(|update| {
            let (existing_id, existing_key) = match update {
                RegionUpdate::Upsert(region) => (&region.id, region.key),
                RegionUpdate::Remove { id, key } => (id, *key),
            };
            existing_id == id && existing_key == key
        }) {
            *existing = value;
            return Ok(());
        }
        self.reserve()?;
        self.region_updates.push(value);
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
            )
            .saturating_add(
                self.regions
                    .capacity()
                    .saturating_mul(size_of::<RegionOutput>()),
            );
        bytes = bytes.saturating_add(
            self.region_updates
                .capacity()
                .saturating_mul(size_of::<RegionUpdate>()),
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
        for region in &self.regions {
            bytes = bytes
                .saturating_add(region.id.capacity())
                .saturating_add(
                    region
                        .border_color_slot
                        .as_ref()
                        .map_or(0, String::capacity),
                )
                .saturating_add(region.fill_color_slot.as_ref().map_or(0, String::capacity));
        }
        for update in &self.region_updates {
            match update {
                RegionUpdate::Upsert(region) => {
                    bytes = bytes
                        .saturating_add(region.id.capacity())
                        .saturating_add(
                            region
                                .border_color_slot
                                .as_ref()
                                .map_or(0, String::capacity),
                        )
                        .saturating_add(
                            region.fill_color_slot.as_ref().map_or(0, String::capacity),
                        );
                }
                RegionUpdate::Remove { id, .. } => {
                    bytes = bytes.saturating_add(id.capacity());
                }
            }
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
            || self.regions.iter().any(|region| {
                region.id.capacity() > limit
                    || region
                        .border_color_slot
                        .as_ref()
                        .is_some_and(|slot| slot.capacity() > limit)
                    || region
                        .fill_color_slot
                        .as_ref()
                        .is_some_and(|slot| slot.capacity() > limit)
            })
            || self.region_updates.iter().any(|update| match update {
                RegionUpdate::Upsert(region) => {
                    region.id.capacity() > limit
                        || region
                            .border_color_slot
                            .as_ref()
                            .is_some_and(|slot| slot.capacity() > limit)
                        || region
                            .fill_color_slot
                            .as_ref()
                            .is_some_and(|slot| slot.capacity() > limit)
                }
                RegionUpdate::Remove { id, .. } => id.capacity() > limit,
            })
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
            regions: self.regions.into(),
            region_updates: self.region_updates.into(),
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
    executed_work: usize,
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
    /// Instantiate a shared compiled program without copying its HIR or registry.
    pub fn with_limits(
        compiled: Arc<CompiledIndicator>,
        config: IndicatorConfig,
        limits: RuntimeLimits,
    ) -> Result<Self, PlanningError> {
        let plan = InstancePlan::from_compiled(&compiled, config)?;
        let committed = CommittedState {
            index: None,
            states: Arc::new(vec![StateValue::Uninitialized; compiled.program.states]),
            function_states: Arc::new(BTreeMap::new()),
            history: SeriesHistory::new(
                compiled.program.series,
                compiled.descriptor.max_bars_back.saturating_add(1),
            ),
            intrinsics: Arc::new(BTreeMap::new()),
        };
        Ok(Self {
            working: WorkingState::from_committed(&committed, compiled.program.series),
            locals: vec![Value::Na; compiled.program.locals],
            current_bar_index: 0,
            current_frame: None,
            executed_work: 0,
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
        })
    }

    fn evaluate_frame(
        &mut self,
        frame: IndicatorFrame,
        previewing: bool,
    ) -> Result<FrameOutput, RuntimeError> {
        if let Some(error) = &self.quarantined {
            return Err(error.clone());
        }
        if !previewing
            && self
                .committed
                .index
                .is_some_and(|index| frame.index <= index)
        {
            return Err(RuntimeError::InvalidValue {
                operation: "closed frame order",
            });
        }

        let index = frame.index;
        let time = frame.time;
        self.current_bar_index = index;
        self.previewing = previewing;
        self.current_frame = Some(frame.clone());
        self.locals.fill(Value::Na);
        self.executed_work = 0;
        self.break_requested = false;
        self.continue_requested = false;
        self.return_requested = false;
        self.return_value = None;
        self.working
            .reset_from_committed(&self.committed, self.compiled.program.series);
        self.resource_check_dirty = false;

        let mut output = OutputBuffer::new(self.limits.max_output_items);
        let result = (|| {
            self.charge_by(self.compiled.program.expressions.len())?;
            self.seed_frame_series(&frame);
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
                self.commit(index)?;
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

    pub fn committed_index(&self) -> Option<usize> {
        self.committed.index
    }

    /// Discard all committed values and bind a new input configuration.
    ///
    /// The caller must recalculate the loaded closed frames before requesting
    /// an open preview.
    pub fn reset(&mut self, config: IndicatorConfig) -> Result<(), PlanningError> {
        let replacement = Self::with_limits(Arc::clone(&self.compiled), config, self.limits)?;
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
        let Some(descriptor) = self.compiled.descriptor.series_inputs.get(slot.0) else {
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

    fn complete_outputs(&mut self, output: &mut OutputBuffer) -> Result<(), RuntimeError> {
        self.charge_by(
            self.compiled.program.statements.len().saturating_add(
                self.compiled
                    .program
                    .plots
                    .len()
                    .saturating_mul(output.plots.len()),
            ),
        )?;
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

    fn output_value(
        &mut self,
        expression: ExprId,
        output: &str,
        field: &'static str,
        expected: ValueType,
    ) -> Result<Value, RuntimeError> {
        let value = self.evaluate_expression(expression)?;
        let valid = value.is_na()
            || matches!(
                (expected, &value),
                (ValueType::Float, Value::Int(_) | Value::Float(_))
                    | (ValueType::Int, Value::Int(_))
                    | (ValueType::Bool, Value::Bool(_))
                    | (ValueType::Color, Value::Color(_))
                    | (
                        ValueType::Any,
                        Value::Int(_) | Value::Float(_) | Value::Bool(_)
                    )
            );
        if valid {
            return Ok(value);
        }
        let expected = match expected {
            ValueType::Float => "number",
            ValueType::Int => "integer",
            ValueType::Bool => "boolean",
            ValueType::Color => "color",
            _ => "number or boolean",
        };
        Err(RuntimeError::InvalidOutput {
            output: output.to_owned(),
            field,
            expected,
            actual: value.kind(),
        })
    }

    fn output_visibility(
        &mut self,
        expression: Option<ExprId>,
        output: &str,
        field: &'static str,
        default: bool,
    ) -> Result<bool, RuntimeError> {
        expression
            .map(|expression| self.output_value(expression, output, field, ValueType::Bool))
            .transpose()
            .map(|value| value.map_or(default, |value| value.is_truthy()))
    }

    fn evaluate_color(
        &mut self,
        expression: Option<ExprId>,
        output: &str,
        field: &'static str,
    ) -> Result<Option<Color>, RuntimeError> {
        expression
            .map(|expression| self.output_value(expression, output, field, ValueType::Color))
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
                if !self.try_push_array(target, expression)? {
                    let value = self.evaluate_expression(expression)?;
                    if let Some(local) = self.locals.get_mut(target.0) {
                        *local = value;
                    }
                }
            }
            StatementKind::Destructure {
                targets,
                expression,
            } => {
                let value = self.evaluate_expression(expression)?;
                let values = match &value {
                    Value::Tuple(values) => values.as_slice(),
                    Value::Array(values) => values.as_slice(),
                    _ => &[],
                };
                for (index, target) in targets.into_iter().enumerate() {
                    let value = values.get(index);
                    self.charge_by(1usize.saturating_add(value.map_or(0, Value::copy_work)))?;
                    if let Some(local) = self.locals.get_mut(target.0) {
                        *local = value.cloned().unwrap_or(Value::Na);
                    }
                }
            }
            StatementKind::StateDeclaration {
                target, initial, ..
            } => {
                if !self.state_initialized(target) {
                    let value = self.evaluate_expression(initial)?;
                    if !value.is_na() {
                        self.set_state_value(target, value)?;
                    }
                }
            }
            StatementKind::StateUpdate { target, expression } => {
                let value = self.evaluate_expression(expression)?;
                self.set_state_value(target, value)?;
            }
            StatementKind::Plot {
                plot,
                expression,
                color,
            } => {
                let Some(descriptor) = self.compiled.program.plots.get(plot.0) else {
                    return Ok(());
                };
                let id = descriptor.id.clone();
                let expected = if descriptor.value_type == crate::scene::PlotValueType::Boolean {
                    ValueType::Bool
                } else {
                    ValueType::Float
                };
                let value = self.output_value(expression, &id, "value", expected)?;
                let color = self.evaluate_color(color, &id, "color")?;
                output.push_plot(PlotOutput { id, value, color })?;
            }
            StatementKind::Marker {
                id,
                expression,
                visible,
                color,
                text,
            } => {
                let value = self.output_value(expression, &id, "value", ValueType::Any)?;
                let visible = self.output_visibility(visible, &id, "visible", true)?;
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
                    id: id.clone(),
                    value,
                    color: self.evaluate_color(color, &id, "color")?,
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
                let visible = self.output_visibility(visible, &id, "visible", true)?;
                if !visible {
                    return Ok(());
                }
                let Some(start_index) = self
                    .output_value(start_index, &id, "start_index", ValueType::Int)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let Some(end_index) = self
                    .output_value(end_index, &id, "end_index", ValueType::Int)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let (Some(start_value), Some(end_value)) = (
                    self.output_value(start_value, &id, "start_value", ValueType::Float)?
                        .as_float(),
                    self.output_value(end_value, &id, "end_value", ValueType::Float)?
                        .as_float(),
                ) else {
                    return Ok(());
                };
                output.push_segment(SegmentOutput {
                    id: id.clone(),
                    start_index,
                    start_value: start_value.is_finite().then_some(start_value),
                    end_index,
                    end_value: end_value.is_finite().then_some(end_value),
                    color: self.evaluate_color(color, &id, "color")?,
                    text,
                })?;
            }
            StatementKind::Region {
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
                style_by,
                true_color_slot,
                false_color_slot,
                extend_right,
                persistent,
                width,
            } => {
                let visible = self.output_visibility(visible, &id, "visible", true)?;
                if !visible {
                    return Ok(());
                }
                let Some(key) = self.output_value(key, &id, "key", ValueType::Int)?.as_int() else {
                    return Ok(());
                };
                let Some(left_index) = self
                    .output_value(left_index, &id, "left_index", ValueType::Int)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let Some(right_index) = self
                    .output_value(right_index, &id, "right_index", ValueType::Int)?
                    .as_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Ok(());
                };
                let (Some(upper), Some(lower)) = (
                    self.output_value(upper, &id, "upper", ValueType::Float)?
                        .as_float(),
                    self.output_value(lower, &id, "lower", ValueType::Float)?
                        .as_float(),
                ) else {
                    return Ok(());
                };
                let (border_color_slot, fill_color_slot) = if let Some(style_by) = style_by {
                    let selected = if self
                        .output_value(style_by, &id, "style_by", ValueType::Bool)?
                        .is_truthy()
                    {
                        true_color_slot
                    } else {
                        false_color_slot
                    };
                    (selected.clone(), selected)
                } else {
                    (border_color_slot, fill_color_slot)
                };
                let region = RegionOutput {
                    id: id.clone(),
                    key,
                    left_index,
                    right_index,
                    upper,
                    lower,
                    border_color: self.evaluate_color(border_color, &id, "border_color")?,
                    fill_color: self.evaluate_color(fill_color, &id, "fill_color")?,
                    border_color_slot,
                    fill_color_slot,
                    extend_right: self.output_visibility(
                        extend_right,
                        &id,
                        "extend_right",
                        false,
                    )?,
                    width,
                };
                if persistent {
                    self.charge_by(output.region_updates.len())?;
                    output.push_region_update(RegionUpdate::Upsert(region))?;
                } else {
                    self.charge_by(output.regions.len())?;
                    output.push_region(region)?;
                }
            }
            StatementKind::RegionRemove { id, key } => {
                let Some(key) = self.output_value(key, &id, "key", ValueType::Int)?.as_int() else {
                    return Ok(());
                };
                self.charge_by(output.region_updates.len())?;
                output.push_region_update(RegionUpdate::Remove { id, key })?;
            }
            StatementKind::HorizontalLevel {
                id,
                expression,
                color,
            } => {
                output.push_level(crate::frame::HorizontalLevelOutput {
                    id: id.clone(),
                    value: self
                        .output_value(expression, &id, "value", ValueType::Float)?
                        .as_float(),
                    color: self.evaluate_color(color, &id, "color")?,
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
                let Some(id) = self
                    .compiled
                    .program
                    .plots
                    .get(plot.0)
                    .map(|plot| plot.id.clone())
                else {
                    return Ok(());
                };
                output.push_ohlc(OhlcOutput {
                    id: id.clone(),
                    open: self
                        .output_value(open, &id, "open", ValueType::Float)?
                        .as_float(),
                    high: self
                        .output_value(high, &id, "high", ValueType::Float)?
                        .as_float(),
                    low: self
                        .output_value(low, &id, "low", ValueType::Float)?
                        .as_float(),
                    close: self
                        .output_value(close, &id, "close", ValueType::Float)?
                        .as_float(),
                    color: self.evaluate_color(color, &id, "color")?,
                })?;
            }
            StatementKind::RangePlot {
                plot,
                upper,
                lower,
                color,
            } => {
                let Some(id) = self
                    .compiled
                    .program
                    .plots
                    .get(plot.0)
                    .map(|plot| plot.id.clone())
                else {
                    return Ok(());
                };
                output.push_range(RangeOutput {
                    id: id.clone(),
                    upper: self
                        .output_value(upper, &id, "upper", ValueType::Float)?
                        .as_float(),
                    lower: self
                        .output_value(lower, &id, "lower", ValueType::Float)?
                        .as_float(),
                    color: self.evaluate_color(color, &id, "color")?,
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
                let (array, tuple) = match self.evaluate_expression(collection)? {
                    Value::Array(values) => (Some(values), Vec::new()),
                    Value::Tuple(values) => (None, values),
                    _ => (None, Vec::new()),
                };
                let values = array
                    .iter()
                    .flat_map(|values| values.as_slice().iter())
                    .chain(tuple.iter());
                for value in values {
                    self.charge_by(1usize.saturating_add(value.copy_work()))?;
                    if let Some(local) = self.locals.get_mut(target.0) {
                        *local = value.clone();
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

    fn try_push_array(
        &mut self,
        target: crate::hir::LocalSlot,
        expression: ExprId,
    ) -> Result<bool, RuntimeError> {
        let Some(ExpressionKind::IntrinsicCall {
            intrinsic,
            arguments,
            ..
        }) = self
            .compiled
            .program
            .expressions
            .get(expression.0)
            .map(|expression| expression.kind.clone())
        else {
            return Ok(false);
        };
        if self
            .compiled
            .registry
            .get(intrinsic)
            .is_none_or(|definition| definition.name != "array.push")
            || arguments.len() != 2
            || !matches!(
                self.compiled
                    .program
                    .expressions
                    .get(arguments[0].0)
                .map(|expression| &expression.kind),
                Some(ExpressionKind::LoadLocal(source)) if *source == target
            )
            || !matches!(self.locals.get(target.0), Some(Value::Array(_)))
        {
            return Ok(false);
        }
        self.charge_by(2)?;
        let value = self.evaluate_expression(arguments[1])?;
        if value.nesting_depth() >= MAX_COLLECTION_NESTING {
            return Err(RuntimeError::ResourceLimitExceeded("collection nesting"));
        }
        let copy_work = match &self.locals[target.0] {
            Value::Array(values) if Arc::strong_count(values) > 1 => {
                values.as_slice().iter().fold(values.len(), |work, value| {
                    work.saturating_add(value.copy_work())
                })
            }
            _ => 0,
        };
        self.charge_by(copy_work)?;
        let Some(Value::Array(values)) = self.locals.get_mut(target.0) else {
            return Ok(false);
        };
        let aliased = Arc::strong_count(values) > 1;
        Arc::make_mut(values).push(value);
        let (collections, heap_bytes) = self.locals[target.0].resource_usage();
        if collections > self.limits.max_collection_elements {
            return Err(RuntimeError::ResourceLimitExceeded("collection elements"));
        }
        if heap_bytes > self.limits.max_heap_bytes {
            return Err(RuntimeError::ResourceLimitExceeded("heap bytes"));
        }
        if aliased {
            self.resource_check_dirty = true;
        }
        Ok(true)
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
        let copy_work = match &expression.kind {
            ExpressionKind::Constant(id) => self.compiled.program.constants.get(id.0),
            ExpressionKind::LoadLocal(slot) => self.locals.get(slot.0),
            ExpressionKind::LoadInput(slot) => self.plan.inputs.get(slot.0),
            ExpressionKind::LoadState(slot) => self.state_value(*slot),
            _ => None,
        }
        .map_or(0, Value::copy_work);
        self.charge_by(copy_work)?;
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
            ExpressionKind::LoadState(slot) => self.state_value(slot).cloned().unwrap_or(Value::Na),
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
            ExpressionKind::Array(expressions) => Value::Array(Arc::new(ArrayValue::new(
                expressions
                    .iter()
                    .map(|expression| self.evaluate_expression(*expression))
                    .collect::<Result<Vec<_>, _>>()?,
            ))),
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
                    self.charge_by(
                        operand
                            .comparison_work()
                            .saturating_add(condition.comparison_work())
                            .saturating_mul(2),
                    )?;
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
                    Value::Record(values) => {
                        self.charge_by(values.get(&field).map_or(0, Value::copy_work))?;
                        values.get(&field).cloned().unwrap_or(Value::Na)
                    }
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
                        let slot = self.compiled.program.expressions[source.0].series_slot;
                        let work = slot
                            .and_then(|slot| self.committed.history.get(slot.0, target))
                            .map_or(0, Value::copy_work);
                        self.charge_by(work)?;
                        slot.and_then(|slot| self.committed.history.get(slot.0, target))
                            .cloned()
                            .unwrap_or(Value::Na)
                    }
                }
            }
            ExpressionKind::CollectionIndex { source, index } => {
                let source = self.evaluate_expression(source)?;
                let index = self.evaluate_expression(index)?.as_int();
                match (source, index.and_then(|index| usize::try_from(index).ok())) {
                    (Value::Tuple(values), Some(index)) => {
                        self.charge_by(values.get(index).map_or(0, Value::copy_work))?;
                        values.get(index).cloned().unwrap_or(Value::Na)
                    }
                    (Value::Array(values), Some(index)) => {
                        self.charge_by(values.get(index).map_or(0, Value::copy_work))?;
                        values.get(index).cloned().unwrap_or(Value::Na)
                    }
                    _ => Value::Na,
                }
            }
            ExpressionKind::Unary { operator, operand } => {
                let operand = self.evaluate_expression(operand)?;
                operand.unary(operator).ok_or(RuntimeError::InvalidValue {
                    operation: "unary expression",
                })?
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
                    if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) {
                        self.charge_by(
                            left.comparison_work()
                                .saturating_add(right.comparison_work())
                                .saturating_mul(2),
                        )?;
                    }
                    left.binary(operator, right)
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
                if self
                    .compiled
                    .registry
                    .get(intrinsic)
                    .is_some_and(|definition| definition.name == "record.get")
                    && arguments.len() == 2
                    && let ExpressionKind::LoadLocal(slot) =
                        self.compiled.program.expressions[arguments[0].0].kind
                {
                    self.charge()?;
                    let key = self.evaluate_expression(arguments[1])?;
                    let Value::String(key) = key else {
                        return Err(RuntimeError::Intrinsic(IntrinsicError::InvalidArguments(
                            "record.get expects a string key",
                        )));
                    };
                    let value = match self.locals.get(slot.0) {
                        Some(Value::Record(values)) => values.get(&key),
                        _ => None,
                    };
                    let work = value.map_or(0, Value::copy_work);
                    self.charge_by(work)?;
                    let value = match self.locals.get(slot.0) {
                        Some(Value::Record(values)) => {
                            values.get(&key).cloned().unwrap_or(Value::Na)
                        }
                        _ => Value::Na,
                    };
                    self.enforce_value_limits(&value)?;
                    if let Some(slot) = expression.series_slot {
                        self.working.current_series[slot.0] = Some(value.clone());
                    }
                    return Ok(value);
                }
                let values = arguments
                    .iter()
                    .map(|argument| self.evaluate_expression(*argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let (stateful, evaluator, fixed_window) = self
                    .compiled
                    .registry
                    .get(intrinsic)
                    .map(|definition| {
                        let fixed_argument = |index: usize| {
                            arguments.get(index).is_some_and(|argument| {
                                self.compiled.program.expressions[argument.0].is_fixed
                            })
                        };
                        let fixed_window = match &definition.signature.warmup {
                            Some(WarmupSpec::Window { argument, .. }) => fixed_argument(*argument),
                            Some(WarmupSpec::Centered {
                                left_argument,
                                right_argument,
                            }) => fixed_argument(*left_argument) && fixed_argument(*right_argument),
                            None => false,
                        };
                        (
                            definition.effects.stateful,
                            definition.evaluator,
                            fixed_window,
                        )
                    })
                    .ok_or(RuntimeError::UnknownIntrinsic(intrinsic))?;
                if stateful {
                    self.resource_check_dirty = true;
                }
                let state_call_site = self.state_call_site(call_site);
                if Arc::strong_count(&self.working.intrinsics) > 1 {
                    let work = self
                        .working
                        .intrinsics
                        .values()
                        .fold(0usize, |work, state| work.saturating_add(state.copy_work()));
                    self.charge_by(work)?;
                }
                let (result, state_heap_exceeded) = {
                    let mut context = IntrinsicEvalContext {
                        bar_index: self.current_bar_index,
                        bar_time: self.current_frame.as_ref().map_or(0, |frame| frame.time),
                        max_heap_bytes: self.limits.max_heap_bytes,
                        max_window_length: self.limits.max_window_length,
                        fixed_window,
                        max_collection_elements: self.limits.max_collection_elements,
                        states: Arc::make_mut(&mut self.working.intrinsics),
                        executed_work: &mut self.executed_work,
                        max_work: self.limits.max_evaluated_nodes,
                    };
                    let result = evaluator(&mut context, state_call_site, &values);
                    context
                        .charge(context.states.len())
                        .map_err(|_| RuntimeError::ExecutionLimitExceeded)?;
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
                    Err(crate::intrinsic::IntrinsicError::ExecutionLimitExceeded) => {
                        return Err(RuntimeError::ExecutionLimitExceeded);
                    }
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
                self.charge_by(self.locals.iter().fold(self.locals.len(), |work, value| {
                    work.saturating_add(value.copy_work())
                }))?;
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

    fn state_value(&self, slot: StateSlot) -> Option<&Value> {
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
                .and_then(StateValue::value);
        }
        self.working.states.get(slot.0).and_then(StateValue::value)
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

    fn set_state_value(&mut self, slot: StateSlot, value: Value) -> Result<(), RuntimeError> {
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
            if Arc::strong_count(&self.working.function_states) > 1 {
                let work = self
                    .working
                    .function_states
                    .values()
                    .filter_map(StateValue::value)
                    .fold(self.working.function_states.len(), |work, value| {
                        work.saturating_add(value.copy_work())
                    });
                self.charge_by(work)?;
            }
            Arc::make_mut(&mut self.working.function_states)
                .insert(state_call_site, StateValue::Value(value));
            return Ok(());
        }
        if Arc::strong_count(&self.working.states) > 1 {
            let work = self
                .working
                .states
                .iter()
                .filter_map(StateValue::value)
                .fold(self.working.states.len(), |work, value| {
                    work.saturating_add(value.copy_work())
                });
            self.charge_by(work)?;
        }
        if let Some(state) = Arc::make_mut(&mut self.working.states).get_mut(slot.0) {
            *state = StateValue::Value(value);
        }
        Ok(())
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
        if charge_work(
            &mut self.executed_work,
            self.limits.max_evaluated_nodes,
            work,
        ) {
            Ok(())
        } else {
            Err(RuntimeError::ExecutionLimitExceeded)
        }
    }

    fn enforce_resource_limits(&mut self, include_history: bool) -> Result<(), RuntimeError> {
        let work = self
            .plan
            .inputs
            .iter()
            .chain(self.locals.iter())
            .chain(self.committed.states.iter().filter_map(StateValue::value))
            .chain(self.working.states.iter().filter_map(StateValue::value))
            .chain(
                self.committed
                    .function_states
                    .values()
                    .filter_map(StateValue::value),
            )
            .chain(
                self.working
                    .function_states
                    .values()
                    .filter_map(StateValue::value),
            )
            .chain(self.working.current_series.iter().flatten())
            .fold(
                self.committed
                    .intrinsics
                    .len()
                    .saturating_add(self.working.intrinsics.len()),
                |work, value| work.saturating_add(value.scan_work().saturating_mul(2)),
            );
        self.charge_by(work.saturating_add(if include_history {
            self.committed.history.values.len().saturating_mul(2)
        } else {
            0
        }))?;
        match self.resource_limit_exceeded(include_history) {
            Some(resource) => Err(RuntimeError::ResourceLimitExceeded(resource)),
            None => Ok(()),
        }
    }

    fn enforce_output_limits(&mut self, output: &OutputBuffer) -> Result<(), RuntimeError> {
        self.charge_by(output.items.saturating_mul(2))?;
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
        if value.nesting_depth() > MAX_COLLECTION_NESTING {
            return Err(RuntimeError::ResourceLimitExceeded("collection nesting"));
        }
        self.charge_by(value.scan_work().saturating_mul(2))?;
        if value.has_string_over(self.limits.max_string_bytes) {
            return Err(RuntimeError::ResourceLimitExceeded("string bytes"));
        }
        let (collections, heap_bytes) = value.resource_usage();
        if !matches!(value, Value::Tuple(_)) && (collections > 0 || heap_bytes > 0) {
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

    fn commit(&mut self, bar_index: usize) -> Result<(), RuntimeError> {
        let work = self
            .working
            .current_series
            .iter()
            .zip(&self.compiled.program.retained_series)
            .filter_map(|(value, retained)| retained.then_some(value.as_ref()).flatten())
            .fold(self.working.current_series.len(), |work, value| {
                work.saturating_add(value.copy_work())
                    .saturating_add(value.scan_work())
            });
        self.charge_by(work)?;
        for (slot, value) in self.working.current_series.iter().enumerate() {
            if !self
                .compiled
                .program
                .retained_series
                .get(slot)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            self.committed
                .history
                .set(slot, bar_index, value.clone().unwrap_or(Value::Na));
        }
        self.committed.states = Arc::clone(&self.working.states);
        self.committed.index = Some(bar_index);
        self.committed.function_states = Arc::clone(&self.working.function_states);
        self.committed.intrinsics = Arc::clone(&self.working.intrinsics);
        self.working.clear_current_series();
        let first_retained = bar_index.saturating_sub(self.max_bars_back);
        for values in &mut self.committed.history.values {
            values.evict_before(first_retained);
        }
        Ok(())
    }
}

pub(crate) fn charge_work(used: &mut usize, limit: usize, work: usize) -> bool {
    match used.checked_add(work) {
        Some(next) if next <= limit => {
            *used = next;
            true
        }
        _ => false,
    }
}
