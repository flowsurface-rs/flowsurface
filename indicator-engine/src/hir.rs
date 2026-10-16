use crate::frame::FrameField;
use crate::scene::PlotDescriptor;
use crate::types::{Type, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExprId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatementId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatementBlockId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub struct StatementBlock {
    pub statements: Vec<StatementId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalSlot(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateSlot(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeriesSlot(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceInputSlot(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallSiteId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntrinsicId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlotId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConstantId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Plus,
    Minus,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStateField {
    IsFirst,
    IsLast,
    IsConfirmed,
    IsRealtime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExpressionKind {
    Constant(ConstantId),
    LoadLocal(LocalSlot),
    LoadState(StateSlot),
    LoadSeries(SeriesSlot),
    LoadInput(LocalSlot),
    LoadSeriesInput(SourceInputSlot),
    LoadFrame(FrameField),
    RecordField {
        source: ExprId,
        field: String,
    },
    LoadBarIndex,
    LoadBarState(BarStateField),
    Tuple(Vec<ExprId>),
    Array(Vec<ExprId>),
    Record(Vec<(String, ExprId)>),
    Switch {
        operand: ExprId,
        branches: Vec<(ExprId, ExprId)>,
        fallback: Option<ExprId>,
    },
    History {
        source: ExprId,
        offset: ExprId,
    },
    CollectionIndex {
        source: ExprId,
        index: ExprId,
    },
    Unary {
        operator: UnaryOperator,
        operand: ExprId,
    },
    Binary {
        operator: BinaryOperator,
        left: ExprId,
        right: ExprId,
    },
    Conditional {
        condition: ExprId,
        when_true: ExprId,
        when_false: ExprId,
    },
    Coalesce {
        source: ExprId,
        fallback: ExprId,
    },
    IntrinsicCall {
        intrinsic: IntrinsicId,
        call_site: CallSiteId,
        arguments: Vec<ExprId>,
    },
    FunctionCall {
        function: FunctionId,
        call_site: CallSiteId,
        arguments: Vec<ExprId>,
        argument_slots: Vec<usize>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpression {
    pub kind: ExpressionKind,
    pub ty: Type,
    pub series_slot: Option<SeriesSlot>,
    pub is_fixed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    Binding {
        target: LocalSlot,
        expression: ExprId,
    },
    LocalUpdate {
        target: LocalSlot,
        expression: ExprId,
    },
    StateDeclaration {
        target: StateSlot,
        initial: ExprId,
        call_site: CallSiteId,
    },
    StateUpdate {
        target: StateSlot,
        expression: ExprId,
    },
    Plot {
        plot: PlotId,
        expression: ExprId,
        color: Option<ExprId>,
    },
    Marker {
        id: String,
        expression: ExprId,
        visible: Option<ExprId>,
        color: Option<ExprId>,
        text: Option<String>,
    },
    Segment {
        id: String,
        start_value: ExprId,
        end_value: ExprId,
        start_index: ExprId,
        end_index: ExprId,
        visible: Option<ExprId>,
        color: Option<ExprId>,
        text: Option<String>,
    },
    Region {
        id: String,
        key: ExprId,
        upper: ExprId,
        lower: ExprId,
        left_index: ExprId,
        right_index: ExprId,
        visible: Option<ExprId>,
        border_color: Option<ExprId>,
        fill_color: Option<ExprId>,
        border_color_slot: Option<String>,
        fill_color_slot: Option<String>,
        style_by: Option<ExprId>,
        true_color_slot: Option<String>,
        false_color_slot: Option<String>,
        extend_right: Option<ExprId>,
        persistent: bool,
        width: f32,
    },
    RegionRemove {
        id: String,
        key: ExprId,
    },
    HorizontalLevel {
        id: String,
        expression: ExprId,
        color: Option<ExprId>,
    },
    OhlcPlot {
        plot: PlotId,
        open: ExprId,
        high: ExprId,
        low: ExprId,
        close: ExprId,
        color: Option<ExprId>,
    },
    RangePlot {
        plot: PlotId,
        upper: ExprId,
        lower: ExprId,
        color: Option<ExprId>,
    },
    Expression(ExprId),
    If {
        condition: ExprId,
        then_branch: Vec<StatementId>,
        else_branch: Vec<StatementId>,
    },
    While {
        condition: ExprId,
        body: Vec<StatementId>,
    },
    ForRange {
        target: LocalSlot,
        start: ExprId,
        end: ExprId,
        body: Vec<StatementId>,
    },
    ForEach {
        target: LocalSlot,
        collection: ExprId,
        body: Vec<StatementId>,
    },
    Break,
    Continue,
    Return(Option<ExprId>),
    Destructure {
        targets: Vec<LocalSlot>,
        expression: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedStatement {
    pub kind: StatementKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub id: FunctionId,
    pub name: String,
    pub parameters: Vec<LocalSlot>,
    pub parameter_names: Vec<String>,
    pub parameter_defaults: Vec<Option<ExprId>>,
    pub local_slots: Vec<LocalSlot>,
    pub body: Vec<StatementId>,
    pub return_type: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirProgram {
    pub expressions: Vec<TypedExpression>,
    pub statements: Vec<TypedStatement>,
    pub functions: Vec<HirFunction>,
    pub constants: Vec<Value>,
    pub locals: usize,
    pub states: usize,
    pub state_call_sites: Vec<CallSiteId>,
    pub state_functions: Vec<Option<FunctionId>>,
    pub series: usize,
    pub retained_series: Vec<bool>,
    pub call_sites: usize,
    pub plots: Vec<PlotDescriptor>,
    pub inputs: Vec<(LocalSlot, String)>,
    pub source_inputs: Vec<(SourceInputSlot, String)>,
    pub local_definitions: Vec<Option<ExprId>>,
    pub blocks: Vec<StatementBlock>,
    pub entry_point: StatementBlockId,
}

impl HirProgram {
    pub fn empty() -> Self {
        Self {
            expressions: Vec::new(),
            statements: Vec::new(),
            functions: Vec::new(),
            constants: Vec::new(),
            locals: 0,
            states: 0,
            state_call_sites: Vec::new(),
            state_functions: Vec::new(),
            series: 0,
            retained_series: Vec::new(),
            call_sites: 0,
            plots: Vec::new(),
            inputs: Vec::new(),
            source_inputs: Vec::new(),
            local_definitions: Vec::new(),
            blocks: vec![StatementBlock {
                statements: Vec::new(),
            }],
            entry_point: StatementBlockId(0),
        }
    }
}
