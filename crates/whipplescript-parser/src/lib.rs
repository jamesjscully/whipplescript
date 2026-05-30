//! Source parser for `.whip` programs.
//!
//! The v0 grammar is still stabilizing, so this crate uses a small
//! hand-written parser. It preserves source spans and keeps rule/effect bodies
//! as source text until the typed IR is ready to lower them.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    fn join(self, other: Self) -> Self {
        Self {
            start: self.start,
            end: other.end,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub span: SourceSpan,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringLiteral {
    pub value: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    pub workflow: Option<Ident>,
    pub explicit_workflow_body: bool,
    pub workflows: Vec<WorkflowDecl>,
    pub patterns: Vec<PatternDecl>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowDecl {
    pub name: Ident,
    pub items: Vec<Item>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    Include(IncludeDecl),
    Use(UseDecl),
    Pattern(PatternDecl),
    Apply(ApplyDecl),
    WorkflowContract(WorkflowContractDecl),
    Agent(AgentDecl),
    Enum(EnumDecl),
    Class(ClassDecl),
    Coerce(CoerceDecl),
    Assert(AssertDecl),
    Rule(RuleDecl),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternDecl {
    pub name: Ident,
    pub type_params: Vec<Ident>,
    pub items: Vec<Item>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyDecl {
    pub pattern: Ident,
    pub type_args: Vec<TypeSyntax>,
    pub alias: Ident,
    pub body: BlockSource,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeDecl {
    pub path: StringLiteral,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowContractDecl {
    pub kind: WorkflowContractKind,
    pub name: Ident,
    pub ty: TypeSyntax,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowContractKind {
    Input,
    Output,
    Failure,
}

impl WorkflowContractKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssertDecl {
    pub expr: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseDecl {
    pub name: StringLiteral,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDecl {
    pub name: Ident,
    pub fields: Vec<AgentField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentField {
    Profile(StringLiteral),
    Capacity(u32, SourceSpan),
    Skills(Vec<StringLiteral>, SourceSpan),
    Capabilities(Vec<StringLiteral>, SourceSpan),
    Unknown { name: Ident, span: SourceSpan },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub variants: Vec<Ident>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDecl {
    pub name: Ident,
    pub fields: Vec<ClassField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassField {
    pub name: Ident,
    pub ty: TypeSyntax,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoerceDecl {
    pub name: Ident,
    pub params: Vec<ParamDecl>,
    pub output: TypeSyntax,
    pub body: BlockSource,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamDecl {
    pub name: Ident,
    pub ty: TypeSyntax,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeSyntax {
    Primitive {
        name: String,
        span: SourceSpan,
    },
    LiteralString {
        value: String,
        span: SourceSpan,
    },
    Ref {
        name: Ident,
    },
    AgentRef {
        agents: Vec<Ident>,
        span: SourceSpan,
    },
    Optional {
        inner: Box<TypeSyntax>,
        span: SourceSpan,
    },
    Array {
        inner: Box<TypeSyntax>,
        span: SourceSpan,
    },
    Map {
        inner: Box<TypeSyntax>,
        span: SourceSpan,
    },
    Union {
        variants: Vec<TypeSyntax>,
        span: SourceSpan,
    },
}

impl TypeSyntax {
    fn span(&self) -> SourceSpan {
        match self {
            Self::Primitive { span, .. }
            | Self::LiteralString { span, .. }
            | Self::Optional { span, .. }
            | Self::Array { span, .. }
            | Self::Map { span, .. }
            | Self::Union { span, .. }
            | Self::AgentRef { span, .. } => *span,
            Self::Ref { name } => name.span,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleDecl {
    pub name: Ident,
    pub whens: Vec<WhenClause>,
    pub body: BlockSource,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhenClause {
    pub text: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockSource {
    pub text: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOutput {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileOutput {
    pub ir: Option<IrProgram>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatOutput {
    pub formatted: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrProgram {
    pub workflow: String,
    pub includes: Vec<IrInclude>,
    pub pattern_applications: Vec<IrPatternApplication>,
    pub workflow_contracts: Vec<IrWorkflowContract>,
    pub uses: Vec<IrUse>,
    pub schemas: Vec<IrSchema>,
    pub agents: Vec<IrAgent>,
    pub coerces: Vec<IrCoerce>,
    pub assertions: Vec<IrAssertion>,
    pub rules: Vec<IrRule>,
    pub rule_dependencies: Vec<IrRuleDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrPatternApplication {
    pub pattern: String,
    pub alias: String,
    pub type_args: Vec<IrType>,
    pub value_args: Vec<IrPatternArgument>,
    pub generated: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrPatternArgument {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrWorkflowContract {
    pub kind: IrWorkflowContractKind,
    pub name: String,
    pub ty: IrType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrWorkflowContractKind {
    Input,
    Output,
    Failure,
}

impl IrWorkflowContractKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrInclude {
    pub path: String,
    pub source_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrAssertion {
    pub expr: IrExpression,
    pub projection_reads: Vec<IrProjectionRead>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrExpression {
    pub source: String,
    pub expr: Expr,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrUse {
    pub kind: IrUseKind,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrUseKind {
    Plugin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrSchema {
    Enum(IrEnum),
    Class(IrClass),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEnum {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrClass {
    pub name: String,
    pub fields: Vec<IrClassField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrClassField {
    pub name: String,
    pub ty: IrType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrType {
    Primitive(IrPrimitiveType),
    LiteralString(String),
    Ref(String),
    AgentRef(Vec<String>),
    Object(Vec<IrClassField>),
    Optional(Box<IrType>),
    Array(Box<IrType>),
    Map(Box<IrType>),
    Union(Vec<IrType>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrPrimitiveType {
    String,
    Int,
    Float,
    Bool,
    Null,
    Duration,
    Time,
    Image,
    Audio,
    Pdf,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrAgent {
    pub name: String,
    pub profile: Option<String>,
    pub capacity: Option<u32>,
    pub skills: Vec<String>,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrCoerce {
    pub name: String,
    pub params: Vec<IrParam>,
    pub output: IrType,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrParam {
    pub name: String,
    pub ty: IrType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrRule {
    pub name: String,
    pub whens: Vec<IrWhen>,
    pub body: String,
    pub metadata: IrRuleMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrWhen {
    pub source: String,
    pub pattern: String,
    pub guard: Option<IrExpression>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrRuleDependency {
    pub producer: String,
    pub consumer: String,
    pub fact: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IrRuleMetadata {
    pub fact_reads: Vec<String>,
    pub projection_reads: Vec<IrProjectionRead>,
    pub fact_writes: Vec<String>,
    pub fact_consumes: Vec<String>,
    pub effects: Vec<IrEffectNode>,
    pub dependencies: Vec<IrEffectDependency>,
    pub case_branches: Vec<IrRuleCaseBranch>,
    pub terminal_outputs: Vec<IrTerminalOutput>,
    pub terminal_branches: Vec<IrTerminalCaseBranch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrProjectionRead {
    pub kind: QueryKind,
    pub head: String,
    pub guard: Option<String>,
}

impl IrProjectionRead {
    fn to_snapshot(&self) -> String {
        let prefix = match self.kind {
            QueryKind::Fact => format!("fact:{}", self.head),
            QueryKind::Effect => format!("effect:{}", self.head),
        };
        match &self.guard {
            Some(guard) => format!("{prefix} where {guard}"),
            None => prefix,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEffectNode {
    pub id: String,
    pub kind: IrEffectKind,
    pub binding: Option<String>,
    pub required_capabilities: Vec<String>,
    pub idempotency_key: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrEffectKind {
    AgentTell,
    BamlCoerce,
    LoftClaim,
    HumanAsk,
    CapabilityCall,
    EventEmit,
    WorkflowInvoke,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEffectDependency {
    pub upstream: String,
    pub predicate: DependencyPredicate,
    pub downstream: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrRuleCaseBranch {
    pub scrutinee: String,
    pub scrutinee_type: IrType,
    pub pattern: IrCasePattern,
    pub guard: Option<IrExpression>,
    pub body_hash: String,
    pub pattern_span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrCasePattern {
    EnumVariant(String),
    LiteralString(String),
    Agent(String),
    OptionalSome { binding: String },
    OptionalNone,
    Wildcard,
}

impl IrCasePattern {
    fn to_snapshot(&self) -> String {
        match self {
            IrCasePattern::EnumVariant(value) => format!("enum:{value}"),
            IrCasePattern::LiteralString(value) => format!("literal:\"{value}\""),
            IrCasePattern::Agent(value) => format!("agent:{value}"),
            IrCasePattern::OptionalSome { binding } => format!("some:{binding}"),
            IrCasePattern::OptionalNone => "none".to_owned(),
            IrCasePattern::Wildcard => "_".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrTerminalOutput {
    pub binding: String,
    pub alternatives: Vec<IrTerminalAlternative>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrTerminalAlternative {
    pub tag: String,
    pub payload_type: IrType,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrTerminalCaseBranch {
    pub scrutinee: String,
    pub tag: Option<String>,
    pub binding: Option<String>,
    pub guard: Option<IrExpression>,
    pub body_hash: String,
    pub pattern_span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyPredicate {
    Succeeds,
    Fails,
    Completes,
}

#[derive(Clone, Debug)]
struct SemanticContext {
    workflow: Option<String>,
    schemas: SchemaIndex,
    agents: BTreeSet<String>,
    agent_capabilities: BTreeMap<String, BTreeSet<String>>,
    coerce_outputs: BTreeMap<String, TypeSyntax>,
    coerce_params: BTreeMap<String, Vec<ParamDecl>>,
    workflow_inputs: BTreeMap<String, WorkflowInputSurface>,
}

#[derive(Clone, Debug, Default)]
struct WorkflowInputSurface {
    inputs: BTreeMap<String, TypeSyntax>,
    schemas: SchemaIndex,
}

#[derive(Clone, Debug, Default)]
struct SchemaIndex {
    classes: BTreeMap<String, BTreeMap<String, TypeSyntax>>,
    enums: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlockFrame {
    After {
        binding: String,
        predicate: DependencyPredicate,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LiteralExpr<'a> {
    String(&'a str),
    Number(&'a str),
    Bool,
    Null,
    Ident(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExprType {
    Bool,
    Int,
    Float,
    String,
    Duration,
    Time,
    Null,
    Object,
    Optional(Box<ExprType>),
    Array(Box<ExprType>),
    Map(Box<ExprType>),
    Finite { label: String, values: Vec<String> },
    Collection,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expr {
    Literal(ExprLiteral),
    Path(Vec<String>),
    Index {
        target: Box<Expr>,
        key: Box<Expr>,
    },
    Array(Vec<Expr>),
    Object(Vec<ExprObjectField>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Query {
        kind: QueryKind,
        head: String,
        guard: Option<Box<Expr>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExprObjectField {
    pub key: String,
    pub value: Expr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExprLiteral {
    String(String),
    Number(String),
    Bool(bool),
    Null,
    Ident(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Not,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryKind {
    Fact,
    Effect,
}

/// Parses a deterministic expression used by guards, assertions, and branch guards.
pub fn parse_expression(expr: &str) -> Result<Expr, String> {
    ExprParser::new(expr).parse()
}

impl Expr {
    pub fn to_snapshot(&self) -> String {
        match self {
            Self::Literal(literal) => literal.to_snapshot(),
            Self::Path(path) => path.join("."),
            Self::Index { target, key } => {
                format!(
                    "{}[{}]",
                    target.to_snapshot_with_parentheses(),
                    key.to_snapshot()
                )
            }
            Self::Array(items) => {
                let items = items
                    .iter()
                    .map(Self::to_snapshot)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{items}]")
            }
            Self::Object(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| format!("{} {}", field.key, field.value.to_snapshot()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{fields}}}")
            }
            Self::Unary { op, expr } => match op {
                UnaryOp::Not => format!("!{}", expr.to_snapshot_with_parentheses()),
            },
            Self::Binary { op, left, right } => format!(
                "{} {} {}",
                left.to_snapshot_with_parentheses(),
                op.to_snapshot(),
                right.to_snapshot_with_parentheses()
            ),
            Self::Call { name, args } => {
                let args = args
                    .iter()
                    .map(Self::to_snapshot)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}({args})")
            }
            Self::Query { kind, head, guard } => {
                let prefix = match kind {
                    QueryKind::Fact => head.clone(),
                    QueryKind::Effect => format!("effect {head}"),
                };
                match guard {
                    Some(guard) => format!("{prefix} where {}", guard.to_snapshot()),
                    None => prefix,
                }
            }
        }
    }

    fn to_snapshot_with_parentheses(&self) -> String {
        match self {
            Self::Binary { .. } => format!("({})", self.to_snapshot()),
            _ => self.to_snapshot(),
        }
    }
}

impl ExprLiteral {
    fn to_snapshot(&self) -> String {
        match self {
            Self::String(value) => format!("{value:?}"),
            Self::Number(value) | Self::Ident(value) => value.clone(),
            Self::Bool(value) => value.to_string(),
            Self::Null => "null".to_owned(),
        }
    }
}

impl BinaryOp {
    fn to_snapshot(self) -> &'static str {
        match self {
            Self::Or => "||",
            Self::And => "&&",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::In => "in",
            Self::NotIn => "not in",
        }
    }
}

/// Parses a source file into a recoverable AST plus diagnostics.
pub fn parse_program(source: &str) -> ParseOutput {
    let lexed = lex(source);
    let mut parser = Parser {
        source,
        tokens: lexed.tokens,
        pos: 0,
        diagnostics: lexed.diagnostics,
    };

    let program = parser.parse_program();
    ParseOutput {
        program,
        diagnostics: parser.diagnostics,
    }
}

/// Parses and lowers a source file into deterministic typed IR.
pub fn compile_program(source: &str) -> CompileOutput {
    compile_program_with_root(source, None)
}

/// Parses and lowers a source bundle into deterministic typed IR with an
/// optional explicit root workflow selection.
pub fn compile_program_with_root(source: &str, root: Option<&str>) -> CompileOutput {
    let parsed = parse_program(source);
    if !parsed.diagnostics.is_empty() {
        return CompileOutput {
            ir: None,
            diagnostics: parsed.diagnostics,
        };
    }

    let workflow_inputs = collect_workflow_input_surfaces(&parsed.program);
    match select_root_workflow(parsed.program, root) {
        Ok(program) => lower_program(program, workflow_inputs),
        Err(diagnostics) => CompileOutput {
            ir: None,
            diagnostics,
        },
    }
}

/// Formats the syntax tree without lowering or analyzing rule bodies.
pub fn format_program(source: &str) -> FormatOutput {
    let parsed = parse_program(source);
    if !parsed.diagnostics.is_empty() {
        return FormatOutput {
            formatted: None,
            diagnostics: parsed.diagnostics,
        };
    }

    FormatOutput {
        formatted: Some(format_syntax(parsed.program)),
        diagnostics: Vec::new(),
    }
}

fn select_root_workflow(
    mut program: Program,
    root: Option<&str>,
) -> Result<Program, Vec<Diagnostic>> {
    if program.workflows.is_empty() {
        if let Some(root) = root {
            match program.workflow.as_ref() {
                Some(workflow) if workflow.name == root => {}
                Some(workflow) => {
                    return Err(vec![Diagnostic {
                        span: workflow.span,
                        message: format!("root workflow `{root}` was not found"),
                        suggestion: Some(format!("available workflow: `{}`", workflow.name)),
                    }]);
                }
                None => {
                    return Err(vec![Diagnostic {
                        span: SourceSpan { start: 0, end: 0 },
                        message: format!("root workflow `{root}` was not found"),
                        suggestion: Some(
                            "add an explicit `workflow Name { ... }` declaration".to_owned(),
                        ),
                    }]);
                }
            }
        }
        return Ok(program);
    }

    let selected_index = match root {
        Some(root) => match program
            .workflows
            .iter()
            .position(|workflow| workflow.name.name == root)
        {
            Some(index) => index,
            None => {
                let names = program
                    .workflows
                    .iter()
                    .map(|workflow| format!("`{}`", workflow.name.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(vec![Diagnostic {
                    span: SourceSpan { start: 0, end: 0 },
                    message: format!("root workflow `{root}` was not found"),
                    suggestion: Some(format!("available workflows: {names}")),
                }]);
            }
        },
        None if program.workflows.len() == 1 => 0,
        None => {
            let names = program
                .workflows
                .iter()
                .map(|workflow| format!("`{}`", workflow.name.name))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(vec![Diagnostic {
                span: SourceSpan { start: 0, end: 0 },
                message: "multiple workflow declarations require an explicit root".to_owned(),
                suggestion: Some(format!(
                    "pass `--root <name>`; available workflows: {names}"
                )),
            }]);
        }
    };

    let selected = program.workflows.remove(selected_index);
    let mut items = program.items;
    items.extend(selected.items);
    Ok(Program {
        workflow: Some(selected.name),
        explicit_workflow_body: true,
        workflows: Vec::new(),
        patterns: program.patterns,
        items,
    })
}

impl IrProgram {
    pub fn to_snapshot(&self) -> String {
        let mut snapshot = String::new();
        push_line(&mut snapshot, format!("workflow {}", self.workflow));

        if !self.includes.is_empty() {
            push_line(&mut snapshot, "includes");
            for include in &self.includes {
                match &include.source_hash {
                    Some(source_hash) => {
                        push_line(
                            &mut snapshot,
                            format!("  {} hash {}", include.path, source_hash),
                        );
                    }
                    None => push_line(&mut snapshot, format!("  {}", include.path)),
                }
            }
        }

        if !self.pattern_applications.is_empty() {
            push_line(&mut snapshot, "pattern_applications");
            for application in &self.pattern_applications {
                let type_args = application
                    .type_args
                    .iter()
                    .map(IrType::to_snapshot)
                    .collect::<Vec<_>>()
                    .join(", ");
                push_line(
                    &mut snapshot,
                    format!(
                        "  {} as {}<{}>",
                        application.pattern, application.alias, type_args
                    ),
                );
                for argument in &application.value_args {
                    push_line(
                        &mut snapshot,
                        format!("    arg {} {}", argument.name, argument.value),
                    );
                }
                for generated in &application.generated {
                    push_line(&mut snapshot, format!("    generated {generated}"));
                }
            }
        }

        if !self.workflow_contracts.is_empty() {
            push_line(&mut snapshot, "workflow_contracts");
            for contract in &self.workflow_contracts {
                push_line(
                    &mut snapshot,
                    format!(
                        "  {} {} {}",
                        contract.kind.as_str(),
                        contract.name,
                        contract.ty.to_snapshot()
                    ),
                );
            }
        }

        if !self.uses.is_empty() {
            push_line(&mut snapshot, "uses");
            for use_decl in &self.uses {
                push_line(
                    &mut snapshot,
                    format!("  {} {}", use_decl.kind.as_str(), use_decl.name),
                );
            }
        }

        if !self.schemas.is_empty() {
            push_line(&mut snapshot, "schemas");
            for schema in &self.schemas {
                match schema {
                    IrSchema::Enum(enum_decl) => {
                        push_line(
                            &mut snapshot,
                            format!(
                                "  enum {} {{ {} }}",
                                enum_decl.name,
                                enum_decl.variants.join(", ")
                            ),
                        );
                    }
                    IrSchema::Class(class_decl) => {
                        push_line(&mut snapshot, format!("  class {}", class_decl.name));
                        for field in &class_decl.fields {
                            push_line(
                                &mut snapshot,
                                format!("    {} {}", field.name, field.ty.to_snapshot()),
                            );
                        }
                    }
                }
            }
        }

        if !self.agents.is_empty() {
            push_line(&mut snapshot, "agents");
            for agent in &self.agents {
                let profile = agent.profile.as_deref().unwrap_or("<missing>");
                let capacity = agent
                    .capacity
                    .map(|capacity| capacity.to_string())
                    .unwrap_or_else(|| "<missing>".to_owned());
                let skills = if agent.skills.is_empty() {
                    "[]".to_owned()
                } else {
                    format!("[{}]", agent.skills.join(", "))
                };
                let capabilities = if agent.capabilities.is_empty() {
                    "[]".to_owned()
                } else {
                    format!("[{}]", agent.capabilities.join(", "))
                };
                push_line(
                    &mut snapshot,
                    format!(
                        "  agent {} profile={} capacity={} skills={} capabilities={}",
                        agent.name, profile, capacity, skills, capabilities
                    ),
                );
            }
        }

        if !self.coerces.is_empty() {
            push_line(&mut snapshot, "coerces");
            for coerce in &self.coerces {
                let params = coerce
                    .params
                    .iter()
                    .map(|param| format!("{} {}", param.name, param.ty.to_snapshot()))
                    .collect::<Vec<_>>()
                    .join(", ");
                push_line(
                    &mut snapshot,
                    format!(
                        "  coerce {}({}) -> {}",
                        coerce.name,
                        params,
                        coerce.output.to_snapshot()
                    ),
                );
            }
        }

        if !self.assertions.is_empty() {
            push_line(&mut snapshot, "assertions");
            for assertion in &self.assertions {
                push_line(
                    &mut snapshot,
                    format!("  assert {}", assertion.expr.expr.to_snapshot()),
                );
                if !assertion.projection_reads.is_empty() {
                    push_line(&mut snapshot, "    reads");
                    for read in &assertion.projection_reads {
                        push_line(&mut snapshot, format!("      {}", read.to_snapshot()));
                    }
                }
            }
        }

        if !self.rules.is_empty() {
            push_line(&mut snapshot, "rules");
            for rule in &self.rules {
                push_line(&mut snapshot, format!("  rule {}", rule.name));
                for when in &rule.whens {
                    match &when.guard {
                        Some(guard) => push_line(
                            &mut snapshot,
                            format!(
                                "    when {} where {}",
                                when.pattern,
                                guard.expr.to_snapshot()
                            ),
                        ),
                        None => push_line(&mut snapshot, format!("    when {}", when.pattern)),
                    }
                }
                if !rule.metadata.fact_reads.is_empty() {
                    push_line(&mut snapshot, "    reads");
                    for read in &rule.metadata.fact_reads {
                        push_line(&mut snapshot, format!("      {}", read));
                    }
                }
                if !rule.metadata.projection_reads.is_empty() {
                    push_line(&mut snapshot, "    projection_reads");
                    for read in &rule.metadata.projection_reads {
                        push_line(&mut snapshot, format!("      {}", read.to_snapshot()));
                    }
                }
                if !rule.metadata.fact_writes.is_empty() {
                    push_line(&mut snapshot, "    writes");
                    for write in &rule.metadata.fact_writes {
                        push_line(&mut snapshot, format!("      {}", write));
                    }
                }
                if !rule.metadata.fact_consumes.is_empty() {
                    push_line(&mut snapshot, "    consumes");
                    for consumed in &rule.metadata.fact_consumes {
                        push_line(&mut snapshot, format!("      {}", consumed));
                    }
                }
                if !rule.metadata.effects.is_empty() {
                    push_line(&mut snapshot, "    effects");
                    for effect in &rule.metadata.effects {
                        let binding = effect.binding.as_deref().unwrap_or("-");
                        push_line(
                            &mut snapshot,
                            format!(
                                "      {} kind={} binding={} key={}",
                                effect.id,
                                effect.kind.as_str(),
                                binding,
                                effect.idempotency_key
                            ),
                        );
                    }
                }
                if !rule.metadata.dependencies.is_empty() {
                    push_line(&mut snapshot, "    dependencies");
                    for dependency in &rule.metadata.dependencies {
                        push_line(
                            &mut snapshot,
                            format!(
                                "      {} --{}--> {}",
                                dependency.upstream,
                                dependency.predicate.as_str(),
                                dependency.downstream
                            ),
                        );
                    }
                }
                if !rule.metadata.case_branches.is_empty() {
                    push_line(&mut snapshot, "    case_branches");
                    for branch in &rule.metadata.case_branches {
                        let guard = branch
                            .guard
                            .as_ref()
                            .map(|guard| guard.expr.to_snapshot())
                            .unwrap_or_else(|| "-".to_owned());
                        push_line(
                            &mut snapshot,
                            format!(
                                "      case {} type={} pattern={} guard={} body_hash={} span={}..{}",
                                branch.scrutinee,
                                branch.scrutinee_type.to_snapshot(),
                                branch.pattern.to_snapshot(),
                                guard,
                                branch.body_hash,
                                branch.pattern_span.start,
                                branch.pattern_span.end
                            ),
                        );
                    }
                }
                if !rule.metadata.terminal_outputs.is_empty() {
                    push_line(&mut snapshot, "    terminal_outputs");
                    for output in &rule.metadata.terminal_outputs {
                        push_line(
                            &mut snapshot,
                            format!(
                                "      {} span={}..{}",
                                output.binding, output.span.start, output.span.end
                            ),
                        );
                        for alternative in &output.alternatives {
                            push_line(
                                &mut snapshot,
                                format!(
                                    "        {} payload={} span={}..{}",
                                    alternative.tag,
                                    alternative.payload_type.to_snapshot(),
                                    alternative.source_span.start,
                                    alternative.source_span.end
                                ),
                            );
                        }
                    }
                }
                if !rule.metadata.terminal_branches.is_empty() {
                    push_line(&mut snapshot, "    terminal_branches");
                    for branch in &rule.metadata.terminal_branches {
                        let tag = branch.tag.as_deref().unwrap_or("_");
                        let binding = branch.binding.as_deref().unwrap_or("-");
                        let guard = branch
                            .guard
                            .as_ref()
                            .map(|guard| guard.expr.to_snapshot())
                            .unwrap_or_else(|| "-".to_owned());
                        push_line(
                            &mut snapshot,
                            format!(
                                "      case {} {} binding={} guard={} body_hash={} span={}..{}",
                                branch.scrutinee,
                                tag,
                                binding,
                                guard,
                                branch.body_hash,
                                branch.pattern_span.start,
                                branch.pattern_span.end
                            ),
                        );
                    }
                }
                push_line(
                    &mut snapshot,
                    format!("    body_hash {}", stable_hash(&rule.body)),
                );
            }
        }

        if !self.rule_dependencies.is_empty() {
            push_line(&mut snapshot, "rule_dependencies");
            for dependency in &self.rule_dependencies {
                push_line(
                    &mut snapshot,
                    format!(
                        "  {} --{}--> {}",
                        dependency.producer, dependency.fact, dependency.consumer
                    ),
                );
            }
        }

        snapshot
    }
}

impl IrEffectKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AgentTell => "agent.tell",
            Self::BamlCoerce => "baml.coerce",
            Self::LoftClaim => "loft.claim",
            Self::HumanAsk => "human.ask",
            Self::CapabilityCall => "capability.call",
            Self::EventEmit => "event.emit",
            Self::WorkflowInvoke => "workflow.invoke",
        }
    }
}

impl DependencyPredicate {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeds => "succeeds",
            Self::Fails => "fails",
            Self::Completes => "completes",
        }
    }
}

impl IrUseKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Plugin => "plugin",
        }
    }
}

impl IrType {
    fn to_snapshot(&self) -> String {
        match self {
            Self::Primitive(primitive) => primitive.as_str().to_owned(),
            Self::LiteralString(value) => format!("literal<{value:?}>"),
            Self::Ref(name) => format!("ref<{name}>"),
            Self::AgentRef(agents) => format!("agentref<{}>", agents.join(" | ")),
            Self::Object(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| format!("{} {}", field.name, field.ty.to_snapshot()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("object<{{{fields}}}>")
            }
            Self::Optional(inner) => format!("optional<{}>", inner.to_snapshot()),
            Self::Array(inner) => format!("array<{}>", inner.to_snapshot()),
            Self::Map(inner) => format!("map<{}>", inner.to_snapshot()),
            Self::Union(variants) => {
                let variants = variants
                    .iter()
                    .map(Self::to_snapshot)
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("union<{variants}>")
            }
        }
    }
}

impl IrPrimitiveType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Null => "null",
            Self::Duration => "duration",
            Self::Time => "time",
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Pdf => "pdf",
            Self::Video => "video",
        }
    }
}

fn lower_program(
    program: Program,
    workflow_inputs: BTreeMap<String, WorkflowInputSurface>,
) -> CompileOutput {
    let mut diagnostics = Vec::new();
    let (program, pattern_applications) = expand_pattern_applications(program, &mut diagnostics);
    let schema_names = collect_schema_names(&program, &mut diagnostics);
    let agent_names = collect_agent_names(&program, &mut diagnostics);
    let workflow_contract_names = collect_workflow_contract_names(&program, &mut diagnostics);
    let semantic = SemanticContext::from_program(&program, workflow_inputs);
    let workflow = match program.workflow {
        Some(workflow) => workflow.name,
        None => {
            diagnostics.push(Diagnostic {
                span: SourceSpan { start: 0, end: 0 },
                message: "expected workflow declaration".to_owned(),
                suggestion: Some("add `workflow Name` before declarations".to_owned()),
            });
            "<missing>".to_owned()
        }
    };

    let mut ir = IrProgram {
        workflow,
        includes: Vec::new(),
        pattern_applications,
        workflow_contracts: Vec::new(),
        uses: Vec::new(),
        schemas: Vec::new(),
        agents: Vec::new(),
        coerces: Vec::new(),
        assertions: Vec::new(),
        rules: Vec::new(),
        rule_dependencies: Vec::new(),
    };

    for item in program.items {
        match item {
            Item::Include(include) => lower_include(include, &mut ir),
            Item::WorkflowContract(contract) => lower_workflow_contract(
                contract,
                &mut ir,
                &schema_names,
                &agent_names,
                &mut diagnostics,
            ),
            Item::Use(use_decl) => lower_use(use_decl, &mut ir, &mut diagnostics),
            Item::Pattern(pattern) => diagnostics.push(Diagnostic {
                span: pattern.span,
                message: format!(
                    "pattern `{}` is not allowed inside this declaration scope",
                    pattern.name.name
                ),
                suggestion: Some("declare patterns at source top level".to_owned()),
            }),
            Item::Apply(apply) => diagnostics.push(Diagnostic {
                span: apply.span,
                message: format!(
                    "pattern application `{}` was not expanded",
                    apply.alias.name
                ),
                suggestion: Some(
                    "ensure the applied pattern is declared at source top level".to_owned(),
                ),
            }),
            Item::Agent(agent) => lower_agent(agent, &mut ir, &mut diagnostics),
            Item::Enum(enum_decl) => lower_enum(enum_decl, &mut ir, &mut diagnostics),
            Item::Class(class_decl) => lower_class(
                class_decl,
                &mut ir,
                &schema_names,
                &agent_names,
                &mut diagnostics,
            ),
            Item::Coerce(coerce) => lower_coerce(
                coerce,
                &mut ir,
                &schema_names,
                &agent_names,
                &mut diagnostics,
            ),
            Item::Assert(assertion) => {
                lower_assert(assertion, &semantic, &mut ir, &mut diagnostics)
            }
            Item::Rule(rule) => lower_rule(
                rule,
                &semantic,
                program.explicit_workflow_body,
                &workflow_contract_names,
                &mut ir,
                &mut diagnostics,
            ),
        }
    }

    ir.rule_dependencies = build_rule_dependencies(&ir.rules);

    CompileOutput {
        ir: diagnostics.is_empty().then_some(ir),
        diagnostics,
    }
}

fn expand_pattern_applications(
    mut program: Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Program, Vec<IrPatternApplication>) {
    let mut patterns = BTreeMap::new();
    for pattern in &program.patterns {
        if patterns
            .insert(pattern.name.name.clone(), pattern.clone())
            .is_some()
        {
            diagnostics.push(Diagnostic {
                span: pattern.name.span,
                message: format!("pattern `{}` is declared more than once", pattern.name.name),
                suggestion: Some("rename one pattern declaration".to_owned()),
            });
        }
    }

    let mut expanded_items = Vec::new();
    let mut applications = Vec::new();
    for item in program.items {
        let Item::Apply(apply) = item else {
            expanded_items.push(item);
            continue;
        };
        let Some(pattern) = patterns.get(&apply.pattern.name) else {
            diagnostics.push(Diagnostic {
                span: apply.pattern.span,
                message: format!("pattern `{}` was not found", apply.pattern.name),
                suggestion: Some("declare the pattern before applying it".to_owned()),
            });
            continue;
        };
        if pattern.type_params.len() != apply.type_args.len() {
            diagnostics.push(Diagnostic {
                span: apply.span,
                message: format!(
                    "pattern `{}` expects {} type arguments but got {}",
                    pattern.name.name,
                    pattern.type_params.len(),
                    apply.type_args.len()
                ),
                suggestion: Some("match the pattern type parameter list".to_owned()),
            });
            continue;
        }
        let type_substitutions = pattern
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .zip(apply.type_args.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let value_substitutions = parse_pattern_value_arguments(&apply, diagnostics);
        let local_names = pattern_local_names(pattern, &apply.alias.name);
        let mut generated = Vec::new();
        for pattern_item in pattern.items.iter().cloned() {
            if let Some((generated_name, item)) = expand_pattern_item(
                pattern_item,
                &apply.alias.name,
                &type_substitutions,
                &value_substitutions,
                &local_names,
                diagnostics,
            ) {
                generated.push(generated_name);
                expanded_items.push(item);
            }
        }
        applications.push(IrPatternApplication {
            pattern: pattern.name.name.clone(),
            alias: apply.alias.name,
            type_args: apply.type_args.into_iter().map(lower_type).collect(),
            value_args: value_substitutions
                .into_iter()
                .map(|(name, value)| IrPatternArgument { name, value })
                .collect(),
            generated,
        });
    }
    program.items = expanded_items;
    (program, applications)
}

fn pattern_local_names(pattern: &PatternDecl, alias: &str) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for item in &pattern.items {
        match item {
            Item::Agent(agent) => {
                names.insert(
                    agent.name.name.clone(),
                    generated_pattern_name(alias, &agent.name.name),
                );
            }
            Item::Enum(enum_decl) => {
                names.insert(
                    enum_decl.name.name.clone(),
                    generated_pattern_name(alias, &enum_decl.name.name),
                );
            }
            Item::Class(class_decl) => {
                names.insert(
                    class_decl.name.name.clone(),
                    generated_pattern_name(alias, &class_decl.name.name),
                );
            }
            Item::Coerce(coerce) => {
                names.insert(
                    coerce.name.name.clone(),
                    generated_pattern_name(alias, &coerce.name.name),
                );
            }
            Item::Rule(rule) => {
                names.insert(
                    rule.name.name.clone(),
                    generated_pattern_name(alias, &rule.name.name),
                );
            }
            _ => {}
        }
    }
    names
}

fn generated_pattern_name(alias: &str, name: &str) -> String {
    format!("{alias}_{name}")
}

fn parse_pattern_value_arguments(
    apply: &ApplyDecl,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, String> {
    let mut args = BTreeMap::new();
    for line in apply
        .body
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut parts = line.splitn(2, char::is_whitespace);
        let Some(name) = parts.next().filter(|name| is_identifier(name)) else {
            diagnostics.push(Diagnostic {
                span: apply.body.span,
                message: format!(
                    "pattern application `{}` has malformed argument `{line}`",
                    apply.alias.name
                ),
                suggestion: Some("write pattern arguments as `name value`".to_owned()),
            });
            continue;
        };
        let Some(value) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            diagnostics.push(Diagnostic {
                span: apply.body.span,
                message: format!(
                    "pattern application `{}` argument `{name}` is missing a value",
                    apply.alias.name
                ),
                suggestion: Some("write pattern arguments as `name value`".to_owned()),
            });
            continue;
        };
        if args.insert(name.to_owned(), value.to_owned()).is_some() {
            diagnostics.push(Diagnostic {
                span: apply.body.span,
                message: format!(
                    "pattern application `{}` passes argument `{name}` more than once",
                    apply.alias.name
                ),
                suggestion: Some("remove the duplicate pattern argument".to_owned()),
            });
        }
    }
    args
}

fn expand_pattern_item(
    item: Item,
    alias: &str,
    type_substitutions: &BTreeMap<String, TypeSyntax>,
    value_substitutions: &BTreeMap<String, String>,
    local_names: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<(String, Item)> {
    match item {
        Item::Include(include) => Some((
            format!("include:{}", include.path.value),
            Item::Include(include),
        )),
        Item::Use(use_decl) => Some((format!("use:{}", use_decl.name.value), Item::Use(use_decl))),
        Item::WorkflowContract(contract) => {
            diagnostics.push(Diagnostic {
                span: contract.span,
                message: "workflow contracts are not allowed in pattern bodies".to_owned(),
                suggestion: Some(
                    "declare workflow inputs, outputs, and failures on the workflow".to_owned(),
                ),
            });
            None
        }
        Item::Pattern(pattern) => {
            diagnostics.push(Diagnostic {
                span: pattern.span,
                message: "nested pattern declarations are not supported".to_owned(),
                suggestion: Some("declare reusable patterns at source top level".to_owned()),
            });
            None
        }
        Item::Apply(apply) => {
            diagnostics.push(Diagnostic {
                span: apply.span,
                message: "pattern applications inside pattern bodies are not supported yet"
                    .to_owned(),
                suggestion: Some(
                    "apply patterns from workflow bodies only in this implementation slice"
                        .to_owned(),
                ),
            });
            None
        }
        Item::Agent(mut agent) => {
            let name = rename_ident(agent.name, alias, local_names);
            let generated = format!("agent:{}", name.name);
            agent.name = name;
            Some((generated, Item::Agent(agent)))
        }
        Item::Enum(mut enum_decl) => {
            let name = rename_ident(enum_decl.name, alias, local_names);
            let generated = format!("enum:{}", name.name);
            enum_decl.name = name;
            Some((generated, Item::Enum(enum_decl)))
        }
        Item::Class(mut class_decl) => {
            let name = rename_ident(class_decl.name, alias, local_names);
            let generated = format!("class:{}", name.name);
            class_decl.name = name;
            for field in &mut class_decl.fields {
                field.ty =
                    substitute_pattern_type(field.ty.clone(), type_substitutions, local_names);
            }
            Some((generated, Item::Class(class_decl)))
        }
        Item::Coerce(mut coerce) => {
            let name = rename_ident(coerce.name, alias, local_names);
            let generated = format!("coerce:{}", name.name);
            coerce.name = name;
            for param in &mut coerce.params {
                param.ty =
                    substitute_pattern_type(param.ty.clone(), type_substitutions, local_names);
            }
            coerce.output =
                substitute_pattern_type(coerce.output.clone(), type_substitutions, local_names);
            coerce.body.text = substitute_pattern_text(
                &coerce.body.text,
                type_substitutions,
                value_substitutions,
                local_names,
            );
            Some((generated, Item::Coerce(coerce)))
        }
        Item::Assert(mut assertion) => {
            assertion.expr = substitute_pattern_text(
                &assertion.expr,
                type_substitutions,
                value_substitutions,
                local_names,
            );
            Some((format!("assert:{alias}"), Item::Assert(assertion)))
        }
        Item::Rule(mut rule) => {
            let name = rename_ident(rule.name, alias, local_names);
            let generated = format!("rule:{}", name.name);
            rule.name = name;
            for when in &mut rule.whens {
                when.text = substitute_pattern_text(
                    &when.text,
                    type_substitutions,
                    value_substitutions,
                    local_names,
                );
            }
            rule.body.text = substitute_pattern_text(
                &rule.body.text,
                type_substitutions,
                value_substitutions,
                local_names,
            );
            Some((generated, Item::Rule(rule)))
        }
    }
}

fn rename_ident(ident: Ident, alias: &str, local_names: &BTreeMap<String, String>) -> Ident {
    Ident {
        name: local_names
            .get(&ident.name)
            .cloned()
            .unwrap_or_else(|| generated_pattern_name(alias, &ident.name)),
        span: ident.span,
    }
}

fn substitute_pattern_type(
    ty: TypeSyntax,
    type_substitutions: &BTreeMap<String, TypeSyntax>,
    local_names: &BTreeMap<String, String>,
) -> TypeSyntax {
    match ty {
        TypeSyntax::Ref { name } => {
            if let Some(replacement) = type_substitutions.get(&name.name) {
                return replacement.clone();
            }
            TypeSyntax::Ref {
                name: Ident {
                    name: local_names.get(&name.name).cloned().unwrap_or(name.name),
                    span: name.span,
                },
            }
        }
        TypeSyntax::AgentRef { agents, span } => TypeSyntax::AgentRef {
            agents: agents
                .into_iter()
                .map(|agent| Ident {
                    name: local_names.get(&agent.name).cloned().unwrap_or(agent.name),
                    span: agent.span,
                })
                .collect(),
            span,
        },
        TypeSyntax::Optional { inner, span } => TypeSyntax::Optional {
            inner: Box::new(substitute_pattern_type(
                *inner,
                type_substitutions,
                local_names,
            )),
            span,
        },
        TypeSyntax::Array { inner, span } => TypeSyntax::Array {
            inner: Box::new(substitute_pattern_type(
                *inner,
                type_substitutions,
                local_names,
            )),
            span,
        },
        TypeSyntax::Map { inner, span } => TypeSyntax::Map {
            inner: Box::new(substitute_pattern_type(
                *inner,
                type_substitutions,
                local_names,
            )),
            span,
        },
        TypeSyntax::Union { variants, span } => TypeSyntax::Union {
            variants: variants
                .into_iter()
                .map(|variant| substitute_pattern_type(variant, type_substitutions, local_names))
                .collect(),
            span,
        },
        other => other,
    }
}

fn substitute_pattern_text(
    text: &str,
    type_substitutions: &BTreeMap<String, TypeSyntax>,
    value_substitutions: &BTreeMap<String, String>,
    local_names: &BTreeMap<String, String>,
) -> String {
    let mut output = text.to_owned();
    for (name, replacement) in type_substitutions {
        output = replace_identifier(&output, name, &replacement.to_source());
    }
    for (name, replacement) in local_names {
        output = replace_identifier(&output, name, replacement);
    }
    for (name, replacement) in value_substitutions {
        output = replace_identifier(&output, name, replacement);
    }
    output
}

fn replace_identifier(source: &str, from: &str, to: &str) -> String {
    let mut output = String::new();
    let mut index = 0usize;
    while let Some(offset) = source[index..].find(from) {
        let start = index + offset;
        let end = start + from.len();
        output.push_str(&source[index..start]);
        let before = source[..start].chars().next_back();
        let after = source[end..].chars().next();
        if before.is_none_or(|ch| !is_identifier_char(ch))
            && after.is_none_or(|ch| !is_identifier_char(ch))
        {
            output.push_str(to);
        } else {
            output.push_str(&source[start..end]);
        }
        index = end;
    }
    output.push_str(&source[index..]);
    output
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn lower_assert(
    assertion: AssertDecl,
    semantic: &SemanticContext,
    ir: &mut IrProgram,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match parse_expression(&assertion.expr) {
        Ok(expr) => {
            validate_parsed_expression(
                &expr,
                semantic,
                &ExprScope::default(),
                &ExprValidationContext::assertion(assertion.span),
                "assertion",
                diagnostics,
            );
            let mut projection_reads = collect_projection_reads(&expr);
            sort_projection_reads(&mut projection_reads);
            ir.assertions.push(IrAssertion {
                expr: IrExpression {
                    source: assertion.expr,
                    expr,
                    span: assertion.span,
                },
                projection_reads,
            });
        }
        Err(message) => diagnostics.push(Diagnostic {
            span: assertion.span,
            message: format!("invalid assertion expression: {message}"),
            suggestion: Some(
                "use a deterministic expression such as `count(Fact) == 1`".to_owned(),
            ),
        }),
    }
}

fn lower_expression(source: &str, span: SourceSpan) -> Option<IrExpression> {
    parse_expression(source).ok().map(|expr| IrExpression {
        source: source.to_owned(),
        expr,
        span,
    })
}

fn collect_projection_reads(expr: &Expr) -> Vec<IrProjectionRead> {
    let mut reads = Vec::new();
    collect_projection_reads_into(expr, &mut reads);
    reads
}

fn collect_projection_reads_into(expr: &Expr, reads: &mut Vec<IrProjectionRead>) {
    match expr {
        Expr::Literal(_) | Expr::Path(_) => {}
        Expr::Index { target, key } => {
            collect_projection_reads_into(target, reads);
            collect_projection_reads_into(key, reads);
        }
        Expr::Array(items) => {
            for item in items {
                collect_projection_reads_into(item, reads);
            }
        }
        Expr::Object(fields) => {
            for field in fields {
                collect_projection_reads_into(&field.value, reads);
            }
        }
        Expr::Unary { expr, .. } => collect_projection_reads_into(expr, reads),
        Expr::Binary { left, right, .. } => {
            collect_projection_reads_into(left, reads);
            collect_projection_reads_into(right, reads);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_projection_reads_into(arg, reads);
            }
        }
        Expr::Query { kind, head, guard } => {
            reads.push(IrProjectionRead {
                kind: *kind,
                head: head.clone(),
                guard: guard.as_ref().map(|guard| guard.to_snapshot()),
            });
            if let Some(guard) = guard {
                collect_projection_reads_into(guard, reads);
            }
        }
    }
}

fn sort_projection_reads(reads: &mut Vec<IrProjectionRead>) {
    reads.sort_by_key(IrProjectionRead::to_snapshot);
    reads.dedup();
}

fn collect_schema_names(program: &Program, diagnostics: &mut Vec<Diagnostic>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &program.items {
        let name = match item {
            Item::Enum(enum_decl) => &enum_decl.name,
            Item::Class(class_decl) => &class_decl.name,
            _ => continue,
        };

        if !names.insert(name.name.clone()) {
            diagnostics.push(Diagnostic {
                span: name.span,
                message: format!("schema `{}` is declared more than once", name.name),
                suggestion: Some("rename one declaration or merge the schemas".to_owned()),
            });
        }
    }

    names
}

fn collect_agent_names(program: &Program, diagnostics: &mut Vec<Diagnostic>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &program.items {
        let Item::Agent(agent) = item else {
            continue;
        };
        if !names.insert(agent.name.name.clone()) {
            diagnostics.push(Diagnostic {
                span: agent.name.span,
                message: format!("agent `{}` is declared more than once", agent.name.name),
                suggestion: Some("rename one agent declaration or merge the settings".to_owned()),
            });
        }
    }
    names
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WorkflowContractNames {
    inputs: BTreeMap<String, TypeSyntax>,
    outputs: BTreeMap<String, TypeSyntax>,
    failures: BTreeMap<String, TypeSyntax>,
}

fn collect_workflow_contract_names(
    program: &Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> WorkflowContractNames {
    let mut names = WorkflowContractNames::default();
    for item in &program.items {
        let Item::WorkflowContract(contract) = item else {
            continue;
        };
        let set = match contract.kind {
            WorkflowContractKind::Input => &mut names.inputs,
            WorkflowContractKind::Output => &mut names.outputs,
            WorkflowContractKind::Failure => &mut names.failures,
        };
        if set
            .insert(contract.name.name.clone(), contract.ty.clone())
            .is_some()
        {
            diagnostics.push(Diagnostic {
                span: contract.name.span,
                message: format!(
                    "workflow declares {} `{}` more than once",
                    contract.kind.as_str(),
                    contract.name.name
                ),
                suggestion: Some("remove the duplicate workflow contract".to_owned()),
            });
        }
    }
    names
}

impl SemanticContext {
    fn from_program(
        program: &Program,
        workflow_inputs: BTreeMap<String, WorkflowInputSurface>,
    ) -> Self {
        let mut schemas = SchemaIndex::with_builtins();
        let mut agents = BTreeSet::new();
        let mut agent_capabilities = BTreeMap::new();
        let mut coerce_outputs = BTreeMap::new();
        let mut coerce_params = BTreeMap::new();

        for item in &program.items {
            schemas.insert_item(item);
            match item {
                Item::Agent(agent) => {
                    agents.insert(agent.name.name.clone());
                    let capabilities = agent
                        .fields
                        .iter()
                        .find_map(|field| match field {
                            AgentField::Capabilities(capabilities, _) => Some(
                                capabilities
                                    .iter()
                                    .map(|capability| capability.value.clone())
                                    .collect::<BTreeSet<_>>(),
                            ),
                            _ => None,
                        })
                        .unwrap_or_default();
                    agent_capabilities.insert(agent.name.name.clone(), capabilities);
                }
                Item::Coerce(coerce) => {
                    coerce_outputs.insert(coerce.name.name.clone(), coerce.output.clone());
                    coerce_params.insert(coerce.name.name.clone(), coerce.params.clone());
                }
                _ => {}
            }
        }

        Self {
            workflow: program
                .workflow
                .as_ref()
                .map(|workflow| workflow.name.clone()),
            schemas,
            agents,
            agent_capabilities,
            coerce_outputs,
            coerce_params,
            workflow_inputs,
        }
    }
}

fn collect_workflow_input_surfaces(program: &Program) -> BTreeMap<String, WorkflowInputSurface> {
    let mut surfaces = BTreeMap::new();
    let top_level_schemas = schema_index_for_items(&program.items);

    if let Some(workflow) = &program.workflow {
        let inputs = workflow_inputs_for_items(&program.items);
        surfaces.insert(
            workflow.name.clone(),
            WorkflowInputSurface {
                inputs,
                schemas: top_level_schemas.clone(),
            },
        );
    }

    for workflow in &program.workflows {
        let mut schemas = top_level_schemas.clone();
        schemas.merge(schema_index_for_items(&workflow.items));
        surfaces.insert(
            workflow.name.name.clone(),
            WorkflowInputSurface {
                inputs: workflow_inputs_for_items(&workflow.items),
                schemas,
            },
        );
    }

    surfaces
}

fn schema_index_for_items(items: &[Item]) -> SchemaIndex {
    let mut schemas = SchemaIndex::with_builtins();
    for item in items {
        schemas.insert_item(item);
    }
    schemas
}

fn workflow_inputs_for_items(items: &[Item]) -> BTreeMap<String, TypeSyntax> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::WorkflowContract(contract) if contract.kind == WorkflowContractKind::Input => {
                Some((contract.name.name.clone(), contract.ty.clone()))
            }
            _ => None,
        })
        .collect()
}

impl SchemaIndex {
    fn with_builtins() -> Self {
        let mut index = Self::default();
        index.insert_class(
            "AgentTurn",
            [
                ("id", string_ty()),
                ("issue", ref_ty("LoftIssue")),
                ("summary", string_ty()),
                ("changedFiles", array_ty(string_ty())),
            ],
        );
        index.insert_class(
            "LoftIssue",
            [
                ("id", string_ty()),
                ("title", string_ty()),
                ("body", string_ty()),
            ],
        );
        index.insert_class("LoftClaim", [("issue", ref_ty("LoftIssue"))]);
        index.insert_class(
            "HumanAnswer",
            [
                ("subject", string_ty()),
                ("decision", string_ty()),
                ("reason", string_ty()),
            ],
        );
        index.insert_class(
            "WorkItem",
            [
                ("id", string_ty()),
                ("title", string_ty()),
                ("body", string_ty()),
            ],
        );
        index.insert_class(
            "Evidence",
            [
                ("title", string_ty()),
                ("path", string_ty()),
                ("summary", string_ty()),
            ],
        );
        index.insert_class(
            "TerminalFailed",
            [
                ("reason", string_ty()),
                ("summary", string_ty()),
                ("effect_id", string_ty()),
                ("run_id", string_ty()),
            ],
        );
        index.insert_class(
            "TerminalTimedOut",
            [
                ("summary", string_ty()),
                ("effect_id", string_ty()),
                ("run_id", string_ty()),
            ],
        );
        index.insert_class(
            "TerminalCancelled",
            [
                ("summary", string_ty()),
                ("effect_id", string_ty()),
                ("run_id", string_ty()),
            ],
        );
        index
    }

    fn insert_class<const N: usize>(&mut self, name: &str, fields: [(&str, TypeSyntax); N]) {
        self.classes.insert(
            name.to_owned(),
            fields
                .into_iter()
                .map(|(field, ty)| (field.to_owned(), ty))
                .collect(),
        );
    }

    fn insert_item(&mut self, item: &Item) {
        match item {
            Item::Enum(enum_decl) => {
                self.enums.insert(
                    enum_decl.name.name.clone(),
                    enum_decl
                        .variants
                        .iter()
                        .map(|variant| variant.name.clone())
                        .collect(),
                );
            }
            Item::Class(class_decl) => {
                self.classes.insert(
                    class_decl.name.name.clone(),
                    class_decl
                        .fields
                        .iter()
                        .map(|field| (field.name.name.clone(), field.ty.clone()))
                        .collect(),
                );
            }
            _ => {}
        }
    }

    fn merge(&mut self, other: SchemaIndex) {
        self.classes.extend(other.classes);
        self.enums.extend(other.enums);
    }

    fn class_exists(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }

    fn resolve_field_path(&self, root_schema: &str, path: &[String]) -> Result<TypeSyntax, String> {
        let mut schema = root_schema.to_owned();
        let mut current = TypeSyntax::Ref {
            name: Ident {
                name: schema.clone(),
                span: zero_span(),
            },
        };

        for field in path {
            let Some(fields) = self.classes.get(&schema) else {
                return Err(format!("schema `{schema}` has no declared fields"));
            };
            let Some(field_ty) = fields.get(field) else {
                return Err(format!("schema `{schema}` has no field `{field}`"));
            };

            current = field_ty.clone();
            match schema_name_for_path(&current) {
                Some(next_schema) => schema = next_schema,
                None if field != path.last().expect("path is non-empty") => {
                    return Err(format!("field `{field}` is not a schema value"));
                }
                None => {}
            }
        }

        Ok(current)
    }
}

fn zero_span() -> SourceSpan {
    SourceSpan { start: 0, end: 0 }
}

fn string_ty() -> TypeSyntax {
    TypeSyntax::Primitive {
        name: "string".to_owned(),
        span: zero_span(),
    }
}

fn array_ty(inner: TypeSyntax) -> TypeSyntax {
    TypeSyntax::Array {
        inner: Box::new(inner),
        span: zero_span(),
    }
}

fn ref_ty(name: &str) -> TypeSyntax {
    TypeSyntax::Ref {
        name: Ident {
            name: name.to_owned(),
            span: zero_span(),
        },
    }
}

fn schema_name_for_path(ty: &TypeSyntax) -> Option<String> {
    match ty {
        TypeSyntax::Ref { name } => Some(name.name.clone()),
        TypeSyntax::Optional { inner, .. } => schema_name_for_path(inner),
        _ => None,
    }
}

fn lower_include(include: IncludeDecl, ir: &mut IrProgram) {
    ir.includes.push(IrInclude {
        path: include.path.value,
        source_hash: None,
    });
}

fn lower_workflow_contract(
    contract: WorkflowContractDecl,
    ir: &mut IrProgram,
    schema_names: &BTreeSet<String>,
    agent_names: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    validate_type_refs(&contract.ty, schema_names, agent_names, diagnostics);
    let kind = match contract.kind {
        WorkflowContractKind::Input => IrWorkflowContractKind::Input,
        WorkflowContractKind::Output => IrWorkflowContractKind::Output,
        WorkflowContractKind::Failure => IrWorkflowContractKind::Failure,
    };
    ir.workflow_contracts.push(IrWorkflowContract {
        kind,
        name: contract.name.name,
        ty: lower_type(contract.ty),
    });
}

fn lower_use(use_decl: UseDecl, ir: &mut IrProgram, _diagnostics: &mut Vec<Diagnostic>) {
    let kind = IrUseKind::Plugin;
    ir.uses.push(IrUse {
        kind,
        name: use_decl.name.value,
    });
}

fn lower_agent(agent: AgentDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    let mut lowered = IrAgent {
        name: agent.name.name.clone(),
        profile: None,
        capacity: None,
        skills: Vec::new(),
        capabilities: Vec::new(),
    };

    for field in agent.fields {
        match field {
            AgentField::Profile(profile) => lowered.profile = Some(profile.value),
            AgentField::Capacity(capacity, span) => {
                if capacity == 0 {
                    diagnostics.push(Diagnostic {
                        span,
                        message: format!(
                            "agent `{}` capacity must be greater than zero",
                            agent.name.name
                        ),
                        suggestion: Some("use `capacity 1` or a larger integer".to_owned()),
                    });
                }
                lowered.capacity = Some(capacity);
            }
            AgentField::Skills(skills, _) => {
                let mut seen = BTreeSet::new();
                for skill in skills {
                    if !seen.insert(skill.value.clone()) {
                        diagnostics.push(Diagnostic {
                            span: skill.span,
                            message: format!(
                                "agent `{}` attaches skill `{}` more than once",
                                agent.name.name, skill.value
                            ),
                            suggestion: Some("remove the duplicate skill entry".to_owned()),
                        });
                    }
                    lowered.skills.push(skill.value);
                }
            }
            AgentField::Capabilities(capabilities, _) => {
                let mut seen = BTreeSet::new();
                for capability in capabilities {
                    if !seen.insert(capability.value.clone()) {
                        diagnostics.push(Diagnostic {
                            span: capability.span,
                            message: format!(
                                "agent `{}` declares capability `{}` more than once",
                                agent.name.name, capability.value
                            ),
                            suggestion: Some("remove the duplicate capability entry".to_owned()),
                        });
                    }
                    lowered.capabilities.push(capability.value);
                }
            }
            AgentField::Unknown { name, .. } => {
                diagnostics.push(Diagnostic {
                    span: name.span,
                    message: format!(
                        "unknown agent field `{}` on agent `{}`",
                        name.name, agent.name.name
                    ),
                    suggestion: Some(
                        "supported agent fields are `profile`, `capacity`, `skills`, and `capabilities`".to_owned(),
                    ),
                });
            }
        }
    }

    if lowered.profile.is_none() {
        diagnostics.push(Diagnostic {
            span: agent.name.span,
            message: format!("agent `{}` is missing a profile", agent.name.name),
            suggestion: Some("add `profile \"profile-name\"` inside the agent block".to_owned()),
        });
    }

    if lowered.capacity.is_none() {
        diagnostics.push(Diagnostic {
            span: agent.name.span,
            message: format!("agent `{}` is missing capacity", agent.name.name),
            suggestion: Some("add `capacity 1` inside the agent block".to_owned()),
        });
    }

    ir.agents.push(lowered);
}

fn lower_enum(enum_decl: EnumDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    let mut variants = BTreeSet::new();
    for variant in &enum_decl.variants {
        if !variants.insert(variant.name.clone()) {
            diagnostics.push(Diagnostic {
                span: variant.span,
                message: format!(
                    "enum `{}` declares variant `{}` more than once",
                    enum_decl.name.name, variant.name
                ),
                suggestion: Some(
                    "remove the duplicate variant or give it a distinct name".to_owned(),
                ),
            });
        }
    }

    ir.schemas.push(IrSchema::Enum(IrEnum {
        name: enum_decl.name.name,
        variants: enum_decl
            .variants
            .into_iter()
            .map(|variant| variant.name)
            .collect(),
    }));
}

fn lower_class(
    class_decl: ClassDecl,
    ir: &mut IrProgram,
    schema_names: &BTreeSet<String>,
    agent_names: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut fields = BTreeSet::new();
    for field in &class_decl.fields {
        if !fields.insert(field.name.name.clone()) {
            diagnostics.push(Diagnostic {
                span: field.name.span,
                message: format!(
                    "class `{}` declares field `{}` more than once",
                    class_decl.name.name, field.name.name
                ),
                suggestion: Some(
                    "remove the duplicate field or give it a distinct name".to_owned(),
                ),
            });
        }
        validate_type_refs(&field.ty, schema_names, agent_names, diagnostics);
    }

    ir.schemas.push(IrSchema::Class(IrClass {
        name: class_decl.name.name,
        fields: class_decl
            .fields
            .into_iter()
            .map(|field| IrClassField {
                name: field.name.name,
                ty: lower_type(field.ty),
            })
            .collect(),
    }));
}

fn lower_coerce(
    coerce: CoerceDecl,
    ir: &mut IrProgram,
    schema_names: &BTreeSet<String>,
    agent_names: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut params = BTreeSet::new();
    for param in &coerce.params {
        if !params.insert(param.name.name.clone()) {
            diagnostics.push(Diagnostic {
                span: param.name.span,
                message: format!(
                    "coerce `{}` declares parameter `{}` more than once",
                    coerce.name.name, param.name.name
                ),
                suggestion: Some(
                    "remove the duplicate parameter or give it a distinct name".to_owned(),
                ),
            });
        }
        validate_type_refs(&param.ty, schema_names, agent_names, diagnostics);
    }
    validate_type_refs(&coerce.output, schema_names, agent_names, diagnostics);

    ir.coerces.push(IrCoerce {
        name: coerce.name.name,
        params: coerce
            .params
            .into_iter()
            .map(|param| IrParam {
                name: param.name.name,
                ty: lower_type(param.ty),
            })
            .collect(),
        output: lower_type(coerce.output),
        body: coerce.body.text,
    });
}

fn validate_type_refs(
    ty: &TypeSyntax,
    schema_names: &BTreeSet<String>,
    agent_names: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match ty {
        TypeSyntax::Primitive { .. } | TypeSyntax::LiteralString { .. } => {}
        TypeSyntax::Ref { name } => {
            if !schema_names.contains(&name.name) && !is_builtin_schema_ref(&name.name) {
                diagnostics.push(Diagnostic {
                    span: name.span,
                    message: format!("unknown schema reference `{}`", name.name),
                    suggestion: Some(format!(
                        "declare `class {}` or `enum {}` before using it",
                        name.name, name.name
                    )),
                });
            }
        }
        TypeSyntax::AgentRef { agents, .. } => {
            let mut seen = BTreeSet::new();
            for agent in agents {
                if !seen.insert(agent.name.clone()) {
                    diagnostics.push(Diagnostic {
                        span: agent.span,
                        message: format!("AgentRef lists agent `{}` more than once", agent.name),
                        suggestion: Some(
                            "remove the duplicate agent from the AgentRef domain".to_owned(),
                        ),
                    });
                }
                if !agent_names.contains(&agent.name) {
                    diagnostics.push(Diagnostic {
                        span: agent.span,
                        message: format!("AgentRef references unknown agent `{}`", agent.name),
                        suggestion: Some(format!(
                            "declare `agent {}` before using it in AgentRef",
                            agent.name
                        )),
                    });
                }
            }
        }
        TypeSyntax::Optional { inner, .. }
        | TypeSyntax::Array { inner, .. }
        | TypeSyntax::Map { inner, .. } => {
            validate_type_refs(inner, schema_names, agent_names, diagnostics)
        }
        TypeSyntax::Union { variants, .. } => {
            for variant in variants {
                validate_type_refs(variant, schema_names, agent_names, diagnostics);
            }
        }
    }
}

fn is_builtin_schema_ref(name: &str) -> bool {
    matches!(
        name,
        "AgentTurn"
            | "WorkItem"
            | "LoftIssue"
            | "LoftClaim"
            | "HumanAnswer"
            | "Evidence"
            | "TerminalFailed"
            | "TerminalTimedOut"
            | "TerminalCancelled"
    )
}

fn lower_rule(
    mut rule: RuleDecl,
    semantic: &SemanticContext,
    explicit_workflow_body: bool,
    workflow_contract_names: &WorkflowContractNames,
    ir: &mut IrProgram,
    diagnostics: &mut Vec<Diagnostic>,
) {
    rule.body.text = desugar_then_chains(&rule.body.text);
    let metadata = analyze_rule(&rule, semantic, diagnostics);
    validate_workflow_terminal_actions(
        &rule,
        semantic,
        &binding_types_for_rule(&rule),
        explicit_workflow_body,
        workflow_contract_names,
        diagnostics,
    );
    validate_effectful_self_trigger(&rule, &metadata, diagnostics);
    ir.rules.push(IrRule {
        name: rule.name.name,
        whens: rule.whens.into_iter().map(lower_when_clause).collect(),
        body: rule.body.text,
        metadata,
    });
}

fn lower_when_clause(when: WhenClause) -> IrWhen {
    let source = when.text;
    let (pattern, guard_source) = split_when_guard(&source);
    let pattern = pattern.to_owned();
    let guard = guard_source.and_then(|guard_source| {
        let guard_offset = source.find(guard_source).unwrap_or(0);
        lower_expression(
            guard_source,
            SourceSpan {
                start: when.span.start + guard_offset,
                end: when.span.start + guard_offset + guard_source.len(),
            },
        )
    });
    IrWhen {
        source,
        pattern,
        guard,
        span: when.span,
    }
}

fn build_rule_dependencies(rules: &[IrRule]) -> Vec<IrRuleDependency> {
    let mut dependencies = Vec::new();
    for producer in rules {
        for produced_fact in &producer.metadata.fact_writes {
            for consumer in rules {
                if consumer.metadata.fact_reads.contains(produced_fact) {
                    dependencies.push(IrRuleDependency {
                        producer: producer.name.clone(),
                        consumer: consumer.name.clone(),
                        fact: produced_fact.clone(),
                    });
                }
            }
        }
    }
    dependencies.sort_by(|left, right| {
        (&left.producer, &left.consumer, &left.fact).cmp(&(
            &right.producer,
            &right.consumer,
            &right.fact,
        ))
    });
    dependencies
}

fn validate_effectful_self_trigger(
    rule: &RuleDecl,
    metadata: &IrRuleMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if metadata.effects.is_empty() {
        return;
    }

    for written_fact in &metadata.fact_writes {
        if metadata.fact_reads.contains(written_fact)
            && !metadata.fact_consumes.contains(written_fact)
        {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "effectful rule `{}` preserves trigger fact `{written_fact}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "consume or advance the triggering fact, or move the next effect behind an external completion event"
                        .to_owned(),
                ),
            });
        }
    }
}

fn binding_types_for_rule(rule: &RuleDecl) -> BTreeMap<String, String> {
    let mut binding_types = BTreeMap::new();
    for when in &rule.whens {
        if let Some((binding, schema)) = binding_from_when(&when.text) {
            binding_types.insert(binding, schema);
        }
    }
    binding_types
}

fn validate_workflow_terminal_actions(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    explicit_workflow_body: bool,
    contracts: &WorkflowContractNames,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for line in rule.body.text.lines().map(str::trim) {
        let terminal = line
            .strip_prefix("complete ")
            .map(|rest| ("complete", rest, &contracts.outputs))
            .or_else(|| {
                line.strip_prefix("fail ")
                    .map(|rest| ("fail", rest, &contracts.failures))
            });
        let Some((action, rest, declared)) = terminal else {
            continue;
        };
        if !explicit_workflow_body {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` uses `{action}` outside an explicit workflow body",
                    rule.name.name
                ),
                suggestion: Some(
                    "wrap the workflow declarations in `workflow Name { ... }` before using terminal actions"
                        .to_owned(),
                ),
            });
            continue;
        }
        let Some(name) = rest.split('{').next().and_then(|header| {
            let mut parts = header.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(name), None) => Some(name),
                _ => None,
            }
        }) else {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!("rule `{}` has malformed `{action}` action", rule.name.name),
                suggestion: Some(format!(
                    "{action} a declared workflow terminal with a payload block"
                )),
            });
            continue;
        };
        if !declared.contains_key(name) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` {action}s unknown workflow terminal `{name}`",
                    rule.name.name
                ),
                suggestion: Some(format!(
                    "declare `{kind} {name} Type` on the workflow first",
                    kind = if action == "complete" {
                        "output"
                    } else {
                        "failure"
                    }
                )),
            });
            continue;
        }
        let Some(contract_ty) = declared.get(name) else {
            continue;
        };
        validate_workflow_terminal_payload(
            rule,
            action,
            name,
            contract_ty,
            semantic,
            binding_types,
            diagnostics,
        );
    }
}

fn validate_workflow_terminal_payload(
    rule: &RuleDecl,
    action: &str,
    terminal_name: &str,
    contract_ty: &TypeSyntax,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some((_, _, body)) = workflow_terminal_blocks(&rule.body.text).into_iter().find(
        |(candidate_action, candidate_name, _)| {
            candidate_action == action && candidate_name == terminal_name
        },
    ) else {
        return;
    };
    let schema = match contract_ty {
        TypeSyntax::Ref { name } if semantic.schemas.class_exists(&name.name) => &name.name,
        _ => {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "workflow terminal `{terminal_name}` uses a non-class payload contract"
                ),
                suggestion: Some(
                    "declare terminal payloads as a class until scalar terminal payload syntax is supported"
                        .to_owned(),
                ),
            });
            return;
        }
    };
    for assignment in collect_field_assignments(&body) {
        let (field, value) = match assignment {
            RecordFieldAssignment::Value { field, value } => (field, value),
            RecordFieldAssignment::Shorthand { field } => (field.clone(), field),
        };
        let line = format!("{field} {value}");
        validate_record_field(rule, &line, schema, semantic, binding_types, diagnostics);
    }
    validate_required_terminal_fields(rule, schema, terminal_name, &body, semantic, diagnostics);
}

fn validate_required_terminal_fields(
    rule: &RuleDecl,
    schema: &str,
    terminal_name: &str,
    body: &str,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(schema_fields) = semantic.schemas.classes.get(schema) else {
        return;
    };
    let seen = collect_field_assignments(body)
        .into_iter()
        .map(|assignment| match assignment {
            RecordFieldAssignment::Value { field, .. }
            | RecordFieldAssignment::Shorthand { field } => field,
        })
        .collect::<BTreeSet<_>>();
    for (required, ty) in schema_fields {
        if seen.contains(required) || matches!(ty, TypeSyntax::Optional { .. }) {
            continue;
        }
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "workflow terminal `{terminal_name}` is missing required field `{schema}.{required}`"
            ),
            suggestion: Some(format!("add `{required}` to the `{terminal_name}` payload")),
        });
    }
}

fn analyze_rule(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> IrRuleMetadata {
    let mut metadata = IrRuleMetadata {
        fact_reads: rule
            .whens
            .iter()
            .map(|when| fact_read_from_when(&when.text))
            .collect(),
        ..IrRuleMetadata::default()
    };
    let mut seen_bindings = BTreeSet::new();
    let mut binding_types = BTreeMap::new();
    for when in &rule.whens {
        if let Some((binding, schema)) = binding_from_when(&when.text) {
            binding_types.insert(binding, schema);
        }
    }
    let effect_payload_types = collect_effect_payload_types(rule, semantic);
    for (binding, payload_type) in &effect_payload_types {
        if let IrType::Ref(schema) = payload_type {
            binding_types.insert(binding.clone(), schema.clone());
        }
    }
    for when in &rule.whens {
        if let (_, Some(guard)) = split_when_guard(&when.text) {
            validate_expression(rule, guard, semantic, &binding_types, "guard", diagnostics);
            validate_known_field_paths(rule, guard, semantic, &binding_types, diagnostics);
            if let Some(expr) = lower_expression(guard, when.span) {
                metadata
                    .projection_reads
                    .extend(collect_projection_reads(&expr.expr));
            }
        }
        validate_availability_when(rule, &when.text, semantic, &binding_types, diagnostics);
    }
    validate_case_blocks(rule, semantic, &binding_types, diagnostics);
    metadata.case_branches =
        collect_rule_case_metadata(rule, semantic, &binding_types, diagnostics);
    let terminal_metadata = collect_terminal_case_metadata(
        rule,
        semantic,
        &binding_types,
        &effect_payload_types,
        diagnostics,
    );
    validate_record_blocks(rule, semantic, &binding_types, diagnostics);
    validate_effect_payloads(rule, semantic, &binding_types, diagnostics);
    validate_workflow_invocations(rule, semantic, &binding_types, diagnostics);
    let mut block_stack: Vec<BlockFrame> = Vec::new();
    let mut misplaced_effect_bindings = BTreeSet::new();
    let mut anonymous_effects = 0usize;
    let mut record_depth = 0i32;

    for raw_line in rule.body.text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if record_depth > 0 {
            record_depth += brace_delta(line);
            continue;
        }

        if let Some(binding) = binding_after_multiline_string_end(line) {
            misplaced_effect_bindings.insert(binding.clone());
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` places effect binding `{binding}` after a multiline string delimiter",
                    rule.name.name
                ),
                suggestion: Some(format!(
                    "move `as {binding}` onto the effect line, before the multiline string body"
                )),
            });
            continue;
        }

        if line.starts_with('}') {
            block_stack.pop();
            continue;
        }

        if line.starts_with("case ") || (!line.starts_with("after ") && is_case_branch_start(line))
        {
            validate_known_field_paths(rule, line, semantic, &binding_types, diagnostics);
            continue;
        }

        let active_afters = after_scopes(&block_stack);
        validate_binding_uses(rule, line, &seen_bindings, &active_afters, diagnostics);
        validate_known_field_paths(rule, line, semantic, &binding_types, diagnostics);

        if let Some(binding) = parse_consume_line(line) {
            match binding_types.get(&binding) {
                Some(schema) => metadata.fact_consumes.push(format!("schema:{schema}")),
                None => diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` consumes unknown fact binding `{binding}`",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "consume a binding introduced by a `when Class as binding` clause"
                            .to_owned(),
                    ),
                }),
            }
            if !line.contains("->") {
                continue;
            }
        }

        if line.starts_with("after ") {
            match parse_after_line(line) {
                Some((binding, predicate)) => {
                    if !seen_bindings.contains(&binding) {
                        let suggestion = if misplaced_effect_bindings.contains(&binding) {
                            format!(
                                "move `as {binding}` onto the effect line before the multiline string"
                            )
                        } else {
                            format!("create an effect with `as {binding}` before the `after` block")
                        };
                        diagnostics.push(Diagnostic {
                            span: rule.body.span,
                            message: format!(
                                "rule `{}` has `after` block for unknown effect binding `{binding}`",
                                rule.name.name
                            ),
                            suggestion: Some(suggestion),
                        });
                    }
                    block_stack.push(BlockFrame::After { binding, predicate });
                }
                None => {
                    diagnostics.push(Diagnostic {
                        span: rule.body.span,
                        message: format!(
                            "rule `{}` has unsupported `after` dependency predicate",
                            rule.name.name
                        ),
                        suggestion: Some(
                            "use `after name succeeds`, `after name fails`, or `after name completes`"
                                .to_owned(),
                        ),
                    });
                }
            }
            continue;
        }

        if let Some((schema, _)) = parse_record_start(line) {
            if !semantic.schemas.class_exists(&schema) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!("rule `{}` records unknown class `{schema}`", rule.name.name),
                    suggestion: Some(format!("declare `class {schema}` before recording it")),
                });
            }
            metadata.fact_writes.push(format!("schema:{schema}"));
            record_depth = brace_delta(line).max(1);
            continue;
        }

        if let Some((kind, binding)) = parse_effect_line(line) {
            validate_agent_tell_target(rule, line, &kind, semantic, &binding_types, diagnostics);
            anonymous_effects += 1;
            let id = binding
                .clone()
                .unwrap_or_else(|| format!("effect{anonymous_effects}"));
            if let Some(binding) = &binding {
                seen_bindings.insert(binding.clone());
                if let Some(schema) = effect_binding_schema(line, &kind, semantic) {
                    binding_types.insert(binding.clone(), schema);
                }
            }
            for (upstream, predicate) in after_scopes(&block_stack) {
                metadata.dependencies.push(IrEffectDependency {
                    upstream,
                    predicate,
                    downstream: id.clone(),
                });
            }
            let idempotency_key = effect_idempotency_key(&rule.name.name, &id, &kind, &binding);
            metadata.effects.push(IrEffectNode {
                id,
                kind,
                binding,
                required_capabilities: parse_required_capabilities(line),
                idempotency_key,
                span: rule.body.span,
            });
        }
    }

    metadata.fact_reads.sort();
    metadata.fact_reads.dedup();
    sort_projection_reads(&mut metadata.projection_reads);
    metadata.fact_writes.sort();
    metadata.fact_writes.dedup();
    metadata.fact_consumes.sort();
    metadata.fact_consumes.dedup();
    metadata.terminal_outputs = terminal_metadata.outputs;
    metadata.terminal_branches = terminal_metadata.branches;
    metadata
}

#[derive(Clone, Debug, Default)]
struct TerminalMetadata {
    outputs: Vec<IrTerminalOutput>,
    branches: Vec<IrTerminalCaseBranch>,
}

#[derive(Clone, Debug)]
struct TerminalBranchSource {
    scrutinee: String,
    pattern: String,
    guard: Option<String>,
    body: String,
    pattern_span: SourceSpan,
}

#[derive(Clone, Debug)]
struct RuleCaseBranchSource {
    scrutinee: String,
    scrutinee_type: TypeSyntax,
    pattern: String,
    guard: Option<String>,
    body: String,
    pattern_span: SourceSpan,
}

fn collect_effect_payload_types(
    rule: &RuleDecl,
    semantic: &SemanticContext,
) -> BTreeMap<String, IrType> {
    let mut payloads = BTreeMap::new();
    for statement in effect_payload_statements(&rule.body.text) {
        let line = statement.trim();
        let Some((kind, Some(binding))) = parse_effect_line(line) else {
            continue;
        };
        payloads.insert(
            binding,
            terminal_completed_payload_type(line, &kind, semantic),
        );
    }

    payloads
}

fn terminal_completed_payload_type(
    line: &str,
    kind: &IrEffectKind,
    semantic: &SemanticContext,
) -> IrType {
    match kind {
        IrEffectKind::BamlCoerce => parse_coerce_call_name(line)
            .and_then(|name| semantic.coerce_outputs.get(name))
            .cloned()
            .map(lower_type)
            .unwrap_or_else(terminal_unknown_payload_type),
        IrEffectKind::LoftClaim => IrType::Ref("LoftClaim".to_owned()),
        IrEffectKind::HumanAsk => IrType::Ref("HumanAnswer".to_owned()),
        IrEffectKind::AgentTell => IrType::Ref("AgentTurn".to_owned()),
        IrEffectKind::CapabilityCall | IrEffectKind::EventEmit | IrEffectKind::WorkflowInvoke => {
            terminal_unknown_payload_type()
        }
    }
}

fn collect_rule_case_metadata(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<IrRuleCaseBranch> {
    let mut branches = Vec::new();
    for branch in rule_case_branch_sources(rule, semantic, binding_types) {
        let mut branch_scope = binding_types.clone();
        if let Some((binding, schema)) =
            case_branch_payload_binding(&branch.pattern, &branch.scrutinee_type, semantic)
        {
            branch_scope.insert(binding, schema);
        }
        if let Some(guard) = &branch.guard {
            validate_expression(
                rule,
                guard,
                semantic,
                &branch_scope,
                "case guard",
                diagnostics,
            );
            validate_known_field_paths_at_span(
                rule,
                guard,
                branch.pattern_span,
                semantic,
                &branch_scope,
                diagnostics,
            );
        }
        validate_known_field_paths_at_span(
            rule,
            &branch.body,
            branch.pattern_span,
            semantic,
            &branch_scope,
            diagnostics,
        );
        if let Some(pattern) = lower_case_pattern(&branch.pattern, &branch.scrutinee_type, semantic)
        {
            branches.push(IrRuleCaseBranch {
                scrutinee: branch.scrutinee,
                scrutinee_type: lower_type(branch.scrutinee_type),
                pattern,
                guard: branch.guard.as_ref().and_then(|guard| {
                    lower_expression(
                        guard,
                        SourceSpan {
                            start: branch.pattern_span.start,
                            end: branch.pattern_span.end,
                        },
                    )
                }),
                body_hash: stable_hash(&branch.body),
                pattern_span: branch.pattern_span,
            });
        }
    }
    branches.sort_by(|left, right| {
        (left.scrutinee.as_str(), left.pattern_span.start)
            .cmp(&(right.scrutinee.as_str(), right.pattern_span.start))
    });
    branches
}

fn rule_case_branch_sources(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
) -> Vec<RuleCaseBranchSource> {
    let lines = rule
        .body
        .text
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((line, current))
        })
        .collect::<Vec<_>>();
    let text_lines = lines.iter().map(|(line, _)| *line).collect::<Vec<_>>();
    let mut branches = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let (line, _) = lines[index];
        let trimmed = line.trim();
        let Some(scrutinee) = case_scrutinee(trimmed) else {
            index += 1;
            continue;
        };
        if active_completes_binding_for_case(&text_lines, index, scrutinee) {
            index += 1;
            continue;
        }
        let Some(scrutinee_type) = expression_type(scrutinee, semantic, binding_types) else {
            index += 1;
            continue;
        };
        let mut depth = brace_delta(trimmed).max(1);
        index += 1;
        while index < lines.len() && depth > 0 {
            let (branch_line, branch_line_offset) = lines[index];
            let branch_trimmed = branch_line.trim();
            if depth == 1 {
                if let Some((pattern, guard, body_start)) = terminal_branch_header(branch_trimmed) {
                    let pattern_column = case_pattern_column(branch_line, pattern);
                    let pattern_span = SourceSpan {
                        start: rule_body_text_start(rule) + branch_line_offset + pattern_column,
                        end: rule_body_text_start(rule)
                            + branch_line_offset
                            + pattern_column
                            + pattern.len(),
                    };
                    let mut body_lines = Vec::new();
                    let mut branch_depth = brace_delta(body_start).max(1);
                    index += 1;
                    while index < lines.len() && branch_depth > 0 {
                        let body_line = lines[index].0;
                        let next_depth = branch_depth + brace_delta(body_line);
                        if next_depth >= 1 {
                            body_lines.push(body_line.to_owned());
                        }
                        branch_depth = next_depth;
                        index += 1;
                    }
                    branches.push(RuleCaseBranchSource {
                        scrutinee: scrutinee.to_owned(),
                        scrutinee_type: scrutinee_type.clone(),
                        pattern: pattern.to_owned(),
                        guard,
                        body: body_lines.join("\n"),
                        pattern_span,
                    });
                    continue;
                }
            }
            depth += brace_delta(branch_trimmed);
            index += 1;
        }
    }
    branches
}

fn lower_case_pattern(
    pattern: &str,
    scrutinee_type: &TypeSyntax,
    semantic: &SemanticContext,
) -> Option<IrCasePattern> {
    if is_fallback_pattern(pattern) {
        return Some(IrCasePattern::Wildcard);
    }
    if pattern == "None" {
        return Some(IrCasePattern::OptionalNone);
    }
    if let Some(binding) = pattern.strip_prefix("Some ").map(str::trim) {
        if !binding.is_empty() {
            return Some(IrCasePattern::OptionalSome {
                binding: binding.to_owned(),
            });
        }
    }
    match scrutinee_type {
        TypeSyntax::Ref { name } if semantic.schemas.enums.contains_key(&name.name) => {
            Some(IrCasePattern::EnumVariant(pattern.to_owned()))
        }
        TypeSyntax::Union { .. } => parse_literal_expr(pattern).and_then(|literal| match literal {
            LiteralExpr::String(value) => Some(IrCasePattern::LiteralString(value.to_owned())),
            LiteralExpr::Ident(value) => Some(IrCasePattern::LiteralString(value.to_owned())),
            _ => None,
        }),
        TypeSyntax::AgentRef { .. } => {
            parse_literal_expr(pattern).and_then(|literal| match literal {
                LiteralExpr::String(value) | LiteralExpr::Ident(value) => {
                    Some(IrCasePattern::Agent(value.to_owned()))
                }
                _ => None,
            })
        }
        TypeSyntax::Optional { inner, .. } => lower_case_pattern(pattern, inner, semantic),
        _ => None,
    }
}

fn case_branch_payload_binding(
    pattern: &str,
    scrutinee_type: &TypeSyntax,
    semantic: &SemanticContext,
) -> Option<(String, String)> {
    let binding = pattern.strip_prefix("Some ").map(str::trim)?;
    if binding.is_empty() {
        return None;
    }
    let TypeSyntax::Optional { inner, .. } = scrutinee_type else {
        return None;
    };
    let schema = match inner.as_ref() {
        TypeSyntax::Ref { name } if semantic.schemas.class_exists(&name.name) => {
            Some(name.name.clone())
        }
        _ => None,
    }?;
    Some((binding.to_owned(), schema))
}

fn collect_terminal_case_metadata(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    effect_payload_types: &BTreeMap<String, IrType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> TerminalMetadata {
    let mut metadata = TerminalMetadata::default();
    let mut output_bindings = BTreeSet::new();

    for branch in terminal_case_branch_sources(rule) {
        if output_bindings.insert(branch.scrutinee.clone()) {
            let completed_payload = effect_payload_types
                .get(&branch.scrutinee)
                .cloned()
                .unwrap_or_else(terminal_unknown_payload_type);
            metadata.outputs.push(IrTerminalOutput {
                binding: branch.scrutinee.clone(),
                alternatives: terminal_alternatives(completed_payload, branch.pattern_span),
                span: branch.pattern_span,
            });
        }

        let (tag, binding) = parse_terminal_pattern_parts(&branch.pattern);
        let mut branch_scope = binding_types.clone();
        if let (Some(tag), Some(binding)) = (&tag, &binding) {
            if let Some(schema) =
                terminal_payload_schema_for_tag(tag, &branch.scrutinee, effect_payload_types)
            {
                branch_scope.insert(binding.clone(), schema);
            }
        }
        if let Some(guard) = &branch.guard {
            validate_expression(
                rule,
                guard,
                semantic,
                &branch_scope,
                "case guard",
                diagnostics,
            );
            validate_known_field_paths(rule, guard, semantic, &branch_scope, diagnostics);
        }
        validate_known_field_paths(rule, &branch.body, semantic, &branch_scope, diagnostics);
        metadata.branches.push(IrTerminalCaseBranch {
            scrutinee: branch.scrutinee,
            tag,
            binding,
            guard: branch.guard.as_ref().and_then(|guard| {
                lower_expression(
                    guard,
                    SourceSpan {
                        start: branch.pattern_span.start,
                        end: branch.pattern_span.end,
                    },
                )
            }),
            body_hash: stable_hash(&branch.body),
            pattern_span: branch.pattern_span,
        });
    }

    metadata
        .outputs
        .sort_by(|left, right| left.binding.cmp(&right.binding));
    metadata.branches.sort_by(|left, right| {
        (left.scrutinee.as_str(), left.pattern_span.start)
            .cmp(&(right.scrutinee.as_str(), right.pattern_span.start))
    });
    metadata
}

fn terminal_case_branch_sources(rule: &RuleDecl) -> Vec<TerminalBranchSource> {
    let lines = rule
        .body
        .text
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((line, current))
        })
        .collect::<Vec<_>>();
    let text_lines = lines.iter().map(|(line, _)| *line).collect::<Vec<_>>();
    let mut branches = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let (line, line_offset) = lines[index];
        let trimmed = line.trim();
        let Some(scrutinee) = case_scrutinee(trimmed) else {
            index += 1;
            continue;
        };
        if !active_completes_binding_for_case(&text_lines, index, scrutinee) {
            index += 1;
            continue;
        }
        let mut depth = brace_delta(trimmed).max(1);
        index += 1;
        while index < lines.len() && depth > 0 {
            let (branch_line, branch_line_offset) = lines[index];
            let branch_trimmed = branch_line.trim();
            if depth == 1 {
                if let Some((pattern, guard, body_start)) = terminal_branch_header(branch_trimmed) {
                    let pattern_column = case_pattern_column(branch_line, pattern);
                    let pattern_span = SourceSpan {
                        start: rule_body_text_start(rule) + branch_line_offset + pattern_column,
                        end: rule_body_text_start(rule)
                            + branch_line_offset
                            + pattern_column
                            + pattern.len(),
                    };
                    let mut body_lines = Vec::new();
                    let mut branch_depth = brace_delta(body_start).max(1);
                    index += 1;
                    while index < lines.len() && branch_depth > 0 {
                        let body_line = lines[index].0;
                        let next_depth = branch_depth + brace_delta(body_line);
                        if next_depth >= 1 {
                            body_lines.push(body_line.to_owned());
                        }
                        branch_depth = next_depth;
                        index += 1;
                    }
                    branches.push(TerminalBranchSource {
                        scrutinee: scrutinee.to_owned(),
                        pattern: pattern.to_owned(),
                        guard,
                        body: body_lines.join("\n"),
                        pattern_span,
                    });
                    continue;
                }
            }
            depth += brace_delta(branch_trimmed);
            index += 1;
        }
        let _ = line_offset;
    }
    branches
}

fn rule_body_text_start(rule: &RuleDecl) -> usize {
    rule.body.span.end.saturating_sub(2 + rule.body.text.len())
}

fn terminal_branch_header(line: &str) -> Option<(&str, Option<String>, &str)> {
    let (head, body_start) = line.split_once("=>")?;
    let body_start = body_start.trim();
    if !body_start.starts_with('{') {
        return None;
    }
    let head = head.trim();
    let (pattern, guard) = match head.split_once(" where ") {
        Some((pattern, guard)) => (pattern.trim(), Some(guard.trim().to_owned())),
        None => (head, None),
    };
    Some((pattern, guard, body_start))
}

fn case_pattern_column(line: &str, pattern: &str) -> usize {
    line.find(pattern).unwrap_or_else(|| {
        let indent = line.len().saturating_sub(line.trim_start().len());
        indent + line.trim_start().find(pattern).unwrap_or(0)
    })
}

fn parse_terminal_pattern_parts(pattern: &str) -> (Option<String>, Option<String>) {
    if is_fallback_pattern(pattern) {
        return (None, None);
    }
    let mut parts = pattern.split_whitespace();
    let tag = parts.next().map(str::to_owned);
    let binding = parts.next().map(str::to_owned);
    if parts.next().is_some() {
        return (tag, None);
    }
    (tag, binding)
}

fn terminal_payload_schema_for_tag(
    tag: &str,
    scrutinee: &str,
    effect_payload_types: &BTreeMap<String, IrType>,
) -> Option<String> {
    match tag {
        "Completed" => match effect_payload_types.get(scrutinee) {
            Some(IrType::Ref(schema)) => Some(schema.clone()),
            _ => None,
        },
        "Failed" => Some("TerminalFailed".to_owned()),
        "TimedOut" => Some("TerminalTimedOut".to_owned()),
        "Cancelled" => Some("TerminalCancelled".to_owned()),
        _ => None,
    }
}

fn terminal_alternatives(
    completed_payload: IrType,
    span: SourceSpan,
) -> Vec<IrTerminalAlternative> {
    [
        ("Completed", completed_payload),
        ("Failed", terminal_failure_payload_type()),
        ("TimedOut", terminal_timeout_payload_type()),
        ("Cancelled", terminal_cancelled_payload_type()),
    ]
    .into_iter()
    .map(|(tag, payload_type)| IrTerminalAlternative {
        tag: tag.to_owned(),
        payload_type,
        source_span: span,
    })
    .collect()
}

fn terminal_failure_payload_type() -> IrType {
    IrType::Object(vec![
        ir_field("reason", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("summary", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("effect_id", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("run_id", IrType::Primitive(IrPrimitiveType::String)),
    ])
}

fn terminal_timeout_payload_type() -> IrType {
    IrType::Object(vec![
        ir_field("summary", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("effect_id", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("run_id", IrType::Primitive(IrPrimitiveType::String)),
    ])
}

fn terminal_cancelled_payload_type() -> IrType {
    IrType::Object(vec![
        ir_field("summary", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("effect_id", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("run_id", IrType::Primitive(IrPrimitiveType::String)),
    ])
}

fn terminal_unknown_payload_type() -> IrType {
    IrType::Object(vec![
        ir_field("summary", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("effect_id", IrType::Primitive(IrPrimitiveType::String)),
        ir_field("run_id", IrType::Primitive(IrPrimitiveType::String)),
    ])
}

fn ir_field(name: &str, ty: IrType) -> IrClassField {
    IrClassField {
        name: name.to_owned(),
        ty,
    }
}

fn effect_idempotency_key(
    rule_name: &str,
    effect_id: &str,
    kind: &IrEffectKind,
    binding: &Option<String>,
) -> String {
    stable_hash(&format!(
        "rule={rule_name};effect={effect_id};kind={};binding={}",
        kind.as_str(),
        binding.as_deref().unwrap_or("-")
    ))
}

fn validate_coerce_call(
    rule: &RuleDecl,
    line: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some((function_name, args)) = parse_coerce_call(line) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has malformed coerce call", rule.name.name),
            suggestion: Some("write `coerce functionName(arg, ...) as name`".to_owned()),
        });
        return;
    };
    let Some(params) = semantic.coerce_params.get(function_name) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` calls unknown coerce function `{function_name}`",
                rule.name.name
            ),
            suggestion: Some(format!(
                "declare `coerce {function_name}(...) -> Output {{ ... }}` before using it"
            )),
        });
        return;
    };
    if args.len() != params.len() {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` calls coerce `{function_name}` with {} argument(s), expected {}",
                rule.name.name,
                args.len(),
                params.len()
            ),
            suggestion: Some("pass one argument for each declared coerce parameter".to_owned()),
        });
        return;
    }
    let scope = ExprScope::from_bindings(binding_types);
    for (arg, param) in args.iter().zip(params) {
        validate_expr_source_against_type(
            rule,
            &format!("coerce `{function_name}`"),
            &param.name.name,
            &param.ty,
            arg,
            semantic,
            &scope,
            diagnostics,
        );
    }
}

fn validate_effect_payloads(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in effect_payload_statements(&rule.body.text) {
        let trimmed = statement.trim();
        if trimmed.starts_with("coerce ") {
            validate_coerce_call(rule, trimmed, semantic, binding_types, diagnostics);
        } else if trimmed.starts_with("claim ") {
            validate_loft_claim_payload(rule, trimmed, semantic, binding_types, diagnostics);
        }
    }
}

fn validate_workflow_invocations(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in workflow_invoke_statements(&rule.body.text) {
        let Some((target, body)) = invoke_statement_parts(&statement) else {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` has malformed workflow invocation",
                    rule.name.name
                ),
                suggestion: Some("write `invoke Workflow { input value } as binding`".to_owned()),
            });
            continue;
        };
        if semantic.workflow.as_deref() == Some(target) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` recursively invokes workflow `{target}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "split recursive orchestration into an explicit bounded scheduler workflow"
                        .to_owned(),
                ),
            });
            continue;
        }
        let Some(surface) = semantic.workflow_inputs.get(target) else {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` invokes unknown workflow `{target}`",
                    rule.name.name
                ),
                suggestion: Some("invoke a workflow declared in this source bundle".to_owned()),
            });
            continue;
        };

        let mut invocation_semantic = semantic.clone();
        invocation_semantic.schemas.merge(surface.schemas.clone());
        let assignments = collect_field_assignments(body);
        let mut seen = BTreeSet::new();
        for assignment in assignments {
            let (field, value) = match assignment {
                RecordFieldAssignment::Value { field, value } => (field, value),
                RecordFieldAssignment::Shorthand { field } => (field.clone(), field),
            };
            if !seen.insert(field.clone()) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!("workflow invocation `{target}` repeats input `{field}`"),
                    suggestion: Some("remove the duplicate invocation input".to_owned()),
                });
                continue;
            }
            let Some(input_ty) = surface.inputs.get(&field) else {
                let known = surface
                    .inputs
                    .keys()
                    .map(|input| format!("`{input}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!("workflow `{target}` has no input `{field}`"),
                    suggestion: Some(if known.is_empty() {
                        "remove the invocation payload; the target declares no inputs".to_owned()
                    } else {
                        format!("pass one of: {known}")
                    }),
                });
                continue;
            };
            validate_expr_source_against_type(
                rule,
                target,
                &field,
                input_ty,
                &value,
                &invocation_semantic,
                &ExprScope::from_bindings(binding_types),
                diagnostics,
            );
        }
        for input in surface.inputs.keys() {
            if seen.contains(input) {
                continue;
            }
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!("workflow invocation `{target}` is missing input `{input}`"),
                suggestion: Some(format!(
                    "add `{input}` to the `{target}` invocation payload"
                )),
            });
        }
    }
}

fn validate_loft_claim_payload(
    rule: &RuleDecl,
    line: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(issue_expr) = parse_loft_claim_issue_expr(line) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has malformed loft claim", rule.name.name),
            suggestion: Some("write `claim issueBinding with loft as claim`".to_owned()),
        });
        return;
    };
    validate_expr_source_against_type(
        rule,
        "loft claim",
        "issue",
        &ref_ty("LoftIssue"),
        issue_expr,
        semantic,
        &ExprScope::from_bindings(binding_types),
        diagnostics,
    );
}

fn validate_agent_tell_target(
    rule: &RuleDecl,
    line: &str,
    kind: &IrEffectKind,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if kind != &IrEffectKind::AgentTell {
        return;
    }
    let Some(target) = parse_tell_target(line) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has malformed tell target", rule.name.name),
            suggestion: Some("write `tell agentName ...` or `tell task.agentRef ...`".to_owned()),
        });
        return;
    };
    if target.starts_with('"') {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` uses a string literal as a tell target",
                rule.name.name
            ),
            suggestion: Some("use a declared agent name or an AgentRef field".to_owned()),
        });
        return;
    }
    let required_capabilities = parse_required_capabilities(line);
    if target.contains('.') {
        let Some(ty) = expression_type(target, semantic, binding_types) else {
            return;
        };
        if let TypeSyntax::AgentRef { agents, .. } = ty {
            for agent in agents {
                validate_agent_capabilities(
                    rule,
                    &agent.name,
                    &required_capabilities,
                    semantic,
                    diagnostics,
                );
            }
        } else {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` uses non-AgentRef dynamic tell target `{target}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "declare the field as `AgentRef<...>` before using it as a tell target"
                        .to_owned(),
                ),
            });
        }
        return;
    }
    if !semantic.agents.contains(target) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` tells unknown agent `{target}`", rule.name.name),
            suggestion: Some("declare the target agent before telling it".to_owned()),
        });
        return;
    }
    validate_agent_capabilities(rule, target, &required_capabilities, semantic, diagnostics);
}

fn validate_agent_capabilities(
    rule: &RuleDecl,
    agent: &str,
    required_capabilities: &[String],
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if required_capabilities.is_empty() {
        return;
    }
    let declared = semantic
        .agent_capabilities
        .get(agent)
        .cloned()
        .unwrap_or_default();
    for capability in required_capabilities {
        if !declared.contains(capability) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` tells agent `{agent}` requiring undeclared capability `{capability}`",
                    rule.name.name
                ),
                suggestion: Some(format!(
                    "add `{capability}` to agent `{agent}` capabilities or choose another AgentRef target"
                )),
            });
        }
    }
}

fn validate_availability_when(
    rule: &RuleDecl,
    when: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (pattern, _) = split_when_guard(when);
    let Some(target) = pattern.strip_suffix(" is available").map(str::trim) else {
        return;
    };
    if target.contains('.') {
        let Some(ty) = expression_type(target, semantic, binding_types) else {
            return;
        };
        if !matches!(ty, TypeSyntax::AgentRef { .. }) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` checks availability for non-AgentRef `{target}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "availability checks must name a declared agent or an AgentRef field"
                        .to_owned(),
                ),
            });
        }
        return;
    }
    if !semantic.agents.contains(target) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` checks unknown agent `{target}`", rule.name.name),
            suggestion: Some("declare the target agent before checking availability".to_owned()),
        });
    }
}

#[derive(Clone, Debug, Default)]
struct ExprScope {
    binding_types: BTreeMap<String, String>,
    implicit_schema: Option<String>,
}

impl ExprScope {
    fn from_bindings(binding_types: &BTreeMap<String, String>) -> Self {
        Self {
            binding_types: binding_types.clone(),
            implicit_schema: None,
        }
    }

    fn with_implicit_schema(&self, schema: String) -> Self {
        let mut scope = self.clone();
        scope.implicit_schema = Some(schema);
        scope
    }
}

#[derive(Clone, Debug)]
struct ExprValidationContext {
    subject: String,
    span: SourceSpan,
}

impl ExprValidationContext {
    fn rule(rule: &RuleDecl) -> Self {
        Self {
            subject: format!("rule `{}`", rule.name.name),
            span: rule.body.span,
        }
    }

    fn assertion(span: SourceSpan) -> Self {
        Self {
            subject: "assertion".to_owned(),
            span,
        }
    }
}

fn validate_expression(
    rule: &RuleDecl,
    expr: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match parse_expression(expr) {
        Ok(expr) => {
            validate_parsed_expression(
                &expr,
                semantic,
                &ExprScope::from_bindings(binding_types),
                &ExprValidationContext::rule(rule),
                label,
                diagnostics,
            );
        }
        Err(message) => diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has invalid {label} expression: {message}", rule.name.name),
            suggestion: Some("use deterministic field paths, literals, boolean operators, comparisons, membership, or count/exists/empty".to_owned()),
        }),
    }
}

fn validate_parsed_expression(
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let presence_proofs = BTreeSet::new();
    validate_expr_node(
        expr,
        semantic,
        scope,
        context,
        &presence_proofs,
        diagnostics,
    );
    let ty = infer_expr_type(expr, semantic, scope, context, diagnostics);
    if ty != ExprType::Bool && ty != ExprType::Unknown {
        diagnostics.push(Diagnostic {
            span: context.span,
            message: format!("{} has non-boolean {label} expression", context.subject),
            suggestion: Some(format!("{label} expressions must evaluate to bool")),
        });
    }
}

fn validate_expr_node(
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    presence_proofs: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Path(path) => {
            if path.len() < 2 {
                return;
            }
            let root = &path[0];
            let Some(schema) = scope.binding_types.get(root) else {
                if let Some(schema) = &scope.implicit_schema {
                    if let Err(message) =
                        validate_optional_path_access(schema, path, semantic, presence_proofs)
                    {
                        diagnostics.push(Diagnostic {
                            span: context.span,
                            message: format!(
                                "{} has unsafe optional path `{}`: {message}",
                                context.subject,
                                path.join(".")
                            ),
                            suggestion: Some(
                                "prove the optional value is present before reading through it"
                                    .to_owned(),
                            ),
                        });
                        return;
                    }
                    if let Err(message) = semantic.schemas.resolve_field_path(schema, path) {
                        diagnostics.push(Diagnostic {
                            span: context.span,
                            message: format!(
                                "{} has invalid expression path `{}`: {message}",
                                context.subject,
                                path.join(".")
                            ),
                            suggestion: Some(
                                "use a field declared on the queried schema".to_owned(),
                            ),
                        });
                    }
                    return;
                }
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} has unknown expression root `{root}`", context.subject),
                    suggestion: Some(
                        "use a binding introduced by a `when ... as name` clause".to_owned(),
                    ),
                });
                return;
            };
            if let Err(message) =
                validate_optional_path_access(schema, &path[1..], semantic, presence_proofs)
            {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} has unsafe optional path `{}`: {message}",
                        context.subject,
                        path.join(".")
                    ),
                    suggestion: Some(
                        "prove the optional value is present before reading through it".to_owned(),
                    ),
                });
                return;
            }
            if let Err(message) = semantic.schemas.resolve_field_path(schema, &path[1..]) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} has invalid expression path `{}`: {message}",
                        context.subject,
                        path.join(".")
                    ),
                    suggestion: Some("use a field declared on the bound schema".to_owned()),
                });
            }
        }
        Expr::Index { target, key } => {
            validate_expr_node(
                target,
                semantic,
                scope,
                context,
                presence_proofs,
                diagnostics,
            );
            validate_expr_node(key, semantic, scope, context, presence_proofs, diagnostics);
            let key_ty = infer_expr_type(key, semantic, scope, context, diagnostics);
            if !matches!(key_ty, ExprType::String | ExprType::Unknown) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} indexes a map with a non-string key", context.subject),
                    suggestion: Some(
                        "use a string literal or string expression as the map key".to_owned(),
                    ),
                });
            }
        }
        Expr::Array(items) => {
            for item in items {
                validate_expr_node(item, semantic, scope, context, presence_proofs, diagnostics);
            }
        }
        Expr::Object(fields) => {
            diagnostics.push(Diagnostic {
                span: context.span,
                message: format!(
                    "{} uses an object literal without an expected object or map type",
                    context.subject
                ),
                suggestion: Some(
                    "use object literals only in typed record fields or typed effect arguments"
                        .to_owned(),
                ),
            });
            for field in fields {
                validate_expr_node(
                    &field.value,
                    semantic,
                    scope,
                    context,
                    presence_proofs,
                    diagnostics,
                );
            }
        }
        Expr::Unary { expr, .. } => {
            validate_expr_node(expr, semantic, scope, context, presence_proofs, diagnostics)
        }
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            validate_expr_node(left, semantic, scope, context, presence_proofs, diagnostics);
            let mut right_proofs = presence_proofs.clone();
            collect_presence_proofs(left, &mut right_proofs);
            validate_expr_node(right, semantic, scope, context, &right_proofs, diagnostics);
        }
        Expr::Binary { op, left, right } => {
            validate_expr_node(left, semantic, scope, context, presence_proofs, diagnostics);
            validate_expr_node(
                right,
                semantic,
                scope,
                context,
                presence_proofs,
                diagnostics,
            );
            validate_unknown_implicit_idents(
                *op,
                left,
                right,
                semantic,
                scope,
                context,
                diagnostics,
            );
            validate_finite_domain_expr(*op, left, right, semantic, scope, context, diagnostics);
        }
        Expr::Call { name, args } => {
            validate_function_call(name, args, semantic, scope, context, diagnostics);
            for arg in args {
                validate_expr_node(arg, semantic, scope, context, presence_proofs, diagnostics);
            }
        }
        Expr::Query { guard, .. } => {
            validate_query_expr(expr, semantic, scope, context, diagnostics);
            if let Some(guard) = guard {
                let guard_scope = query_guard_scope(expr, semantic, scope);
                validate_expr_node(
                    guard,
                    semantic,
                    &guard_scope,
                    context,
                    presence_proofs,
                    diagnostics,
                );
            }
        }
        Expr::Literal(_) => {}
    }
}

fn validate_unknown_implicit_idents(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    ) {
        return;
    }
    validate_unknown_implicit_ident(left, right, semantic, scope, context, diagnostics);
    validate_unknown_implicit_ident(right, left, semantic, scope, context, diagnostics);
}

fn validate_unknown_implicit_ident(
    expr: &Expr,
    other: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Expr::Literal(ExprLiteral::Ident(name)) = expr else {
        return;
    };
    let Some(schema) = &scope.implicit_schema else {
        return;
    };
    let field_exists = semantic
        .schemas
        .classes
        .get(schema)
        .is_some_and(|fields| fields.contains_key(name));
    if field_exists
        || expr_domain(other, semantic, scope).is_some()
        || implicit_ident_field_exists(other, semantic, scope)
    {
        return;
    }
    diagnostics.push(Diagnostic {
        span: context.span,
        message: format!(
            "{} fact query `{schema}` has unknown field `{name}`",
            context.subject
        ),
        suggestion: Some(format!(
            "use a field declared on `{schema}` inside the query `where` expression"
        )),
    });
}

fn implicit_ident_field_exists(expr: &Expr, semantic: &SemanticContext, scope: &ExprScope) -> bool {
    let Expr::Literal(ExprLiteral::Ident(name)) = expr else {
        return false;
    };
    let Some(schema) = &scope.implicit_schema else {
        return false;
    };
    semantic
        .schemas
        .classes
        .get(schema)
        .is_some_and(|fields| fields.contains_key(name))
}

fn validate_function_call(
    name: &str,
    args: &[Expr],
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match name {
        "count" => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `count` with {} arguments, expected 1",
                        context.subject,
                        args.len()
                    ),
                    suggestion: Some(
                        "call `count` with exactly one array, map, fact query, or effect query argument"
                            .to_owned(),
                    ),
                });
                return;
            }
            let ty = infer_expr_type(&args[0], semantic, scope, context, diagnostics);
            if !is_countable_type(&ty) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `count` with unsupported argument type `{}`",
                        context.subject,
                        expr_type_label(&ty)
                    ),
                    suggestion: Some(
                        "use `count` only with arrays, maps, fact queries, or effect queries"
                            .to_owned(),
                    ),
                });
            }
        }
        "one" | "none" => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `{name}` with {} arguments, expected 1",
                        context.subject,
                        args.len()
                    ),
                    suggestion: Some(format!(
                        "call `{name}` with exactly one fact or effect query argument"
                    )),
                });
                return;
            }
            let ty = infer_expr_type(&args[0], semantic, scope, context, diagnostics);
            if !matches!(ty, ExprType::Collection | ExprType::Unknown) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `{name}` with unsupported argument type `{}`",
                        context.subject,
                        expr_type_label(&ty)
                    ),
                    suggestion: Some(format!(
                        "use `{name}` only with fact or effect projection queries"
                    )),
                });
            }
        }
        "exists" => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `exists` with {} arguments, expected 1",
                        context.subject,
                        args.len()
                    ),
                    suggestion: Some("call `exists` with exactly one argument".to_owned()),
                });
                return;
            }
            let ty = infer_expr_type(&args[0], semantic, scope, context, diagnostics);
            if !matches!(args[0], Expr::Index { .. }) && !is_exists_type(&ty) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `exists` with unsupported argument type `{}`",
                        context.subject,
                        expr_type_label(&ty)
                    ),
                    suggestion: Some(
                        "use `exists path` for optional/map presence checks or pass an array, map, fact query, or effect query"
                            .to_owned(),
                    ),
                });
            }
        }
        "empty" => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls `empty` with {} arguments, expected 1",
                        context.subject,
                        args.len()
                    ),
                    suggestion: Some("call `empty` with exactly one argument".to_owned()),
                });
                return;
            }
            let ty = infer_expr_type(&args[0], semantic, scope, context, diagnostics);
            if !is_empty_type(&ty) {
                let optional = matches!(ty, ExprType::Optional(_));
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: if optional {
                        format!(
                            "{} calls `empty` with unsupported optional argument type `{}`",
                            context.subject,
                            expr_type_label(&ty)
                        )
                    } else {
                        format!(
                            "{} calls `empty` with unsupported argument type `{}`",
                            context.subject,
                            expr_type_label(&ty)
                        )
                    },
                    suggestion: Some(
                        "use `empty` only with arrays, maps, strings, fact queries, effect queries, null, or supported optional values"
                            .to_owned(),
                    ),
                });
            }
        }
        _ => {}
    }
}

fn validate_query_expr(
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Expr::Query { kind, head, guard } = expr else {
        return;
    };
    if *kind == QueryKind::Fact {
        let Some(schema) = query_head_schema(head, semantic) else {
            diagnostics.push(Diagnostic {
                span: context.span,
                message: format!(
                    "{} queries unknown fact schema `{}`",
                    context.subject,
                    head.trim()
                ),
                suggestion: Some("use a declared class name in fact queries".to_owned()),
            });
            return;
        };
        if let Some(guard) = guard {
            let guard_scope = scope.with_implicit_schema(schema);
            let ty = infer_expr_type(guard, semantic, &guard_scope, context, diagnostics);
            if !matches!(ty, ExprType::Bool | ExprType::Unknown) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} fact query `{}` has non-boolean `where` expression",
                        context.subject,
                        head.trim()
                    ),
                    suggestion: Some("query `where` expressions must evaluate to bool".to_owned()),
                });
            }
        }
    }
}

fn validate_optional_path_access(
    root_schema: &str,
    path: &[String],
    semantic: &SemanticContext,
    presence_proofs: &BTreeSet<String>,
) -> Result<(), String> {
    let mut schema = root_schema.to_owned();
    let mut prefix = Vec::new();
    for (index, field) in path.iter().enumerate() {
        let Some(fields) = semantic.schemas.classes.get(&schema) else {
            return Ok(());
        };
        let Some(field_ty) = fields.get(field) else {
            return Ok(());
        };
        prefix.push(field.clone());
        if let TypeSyntax::Optional { inner, .. } = field_ty {
            if index + 1 < path.len() && !presence_proofs.contains(&prefix.join(".")) {
                return Err(format!(
                    "`{}` must be proven present before accessing `{}`",
                    prefix.join("."),
                    path[index + 1..].join(".")
                ));
            }
            if let Some(next_schema) = schema_name_for_path(inner) {
                schema = next_schema;
            }
            continue;
        }
        if let Some(next_schema) = schema_name_for_path(field_ty) {
            schema = next_schema;
        }
    }
    Ok(())
}

fn collect_presence_proofs(expr: &Expr, proofs: &mut BTreeSet<String>) {
    match expr {
        Expr::Binary {
            op: BinaryOp::Ne,
            left,
            right,
        } => {
            if matches!(**right, Expr::Literal(ExprLiteral::Null)) {
                if let Some(path) = expr_path_key(left) {
                    proofs.insert(path);
                }
            }
            if matches!(**left, Expr::Literal(ExprLiteral::Null)) {
                if let Some(path) = expr_path_key(right) {
                    proofs.insert(path);
                }
            }
        }
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
        } => {
            if let Expr::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } = expr.as_ref()
            {
                if matches!(**right, Expr::Literal(ExprLiteral::Null)) {
                    if let Some(path) = expr_path_key(left) {
                        proofs.insert(path);
                    }
                }
                if matches!(**left, Expr::Literal(ExprLiteral::Null)) {
                    if let Some(path) = expr_path_key(right) {
                        proofs.insert(path);
                    }
                }
            }
        }
        Expr::Call { name, args } if name == "exists" && args.len() == 1 => {
            if let Some(path) = expr_path_key(&args[0]) {
                proofs.insert(path);
            }
        }
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            collect_presence_proofs(left, proofs);
            collect_presence_proofs(right, proofs);
        }
        _ => {}
    }
}

fn expr_path_key(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Literal(ExprLiteral::Ident(name)) => Some(name.clone()),
        Expr::Path(path) if path.len() >= 2 => Some(path[1..].join(".")),
        Expr::Index { target, key } => {
            let target = expr_path_key(target)?;
            let key = match key.as_ref() {
                Expr::Literal(ExprLiteral::String(value) | ExprLiteral::Ident(value)) => value,
                _ => return None,
            };
            Some(format!("{target}[{key:?}]"))
        }
        _ => None,
    }
}

fn query_guard_scope(expr: &Expr, semantic: &SemanticContext, scope: &ExprScope) -> ExprScope {
    let Expr::Query {
        kind: QueryKind::Fact,
        head,
        ..
    } = expr
    else {
        return scope.clone();
    };
    query_head_schema(head, semantic)
        .map(|schema| scope.with_implicit_schema(schema))
        .unwrap_or_else(|| scope.clone())
}

fn query_head_schema(head: &str, semantic: &SemanticContext) -> Option<String> {
    let mut parts = head.split_whitespace();
    let schema = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    semantic
        .schemas
        .class_exists(schema)
        .then(|| schema.to_owned())
}

fn implicit_field_type(
    name: &str,
    semantic: &SemanticContext,
    scope: &ExprScope,
) -> Option<TypeSyntax> {
    let schema = scope.implicit_schema.as_ref()?;
    semantic
        .schemas
        .resolve_field_path(schema, &[name.to_owned()])
        .ok()
}

fn infer_expr_type(
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> ExprType {
    match expr {
        Expr::Literal(ExprLiteral::Ident(name)) => implicit_field_type(name, semantic, scope)
            .map(|ty| expr_type_from_type_syntax(&ty, semantic))
            .unwrap_or_else(|| expr_literal_type(&ExprLiteral::Ident(name.clone()))),
        Expr::Literal(literal) => expr_literal_type(literal),
        Expr::Path(path) => expr_path_type(path, semantic, scope).unwrap_or(ExprType::Unknown),
        Expr::Index { target, key } => {
            let target_ty = infer_expr_type(target, semantic, scope, context, diagnostics);
            let key_ty = infer_expr_type(key, semantic, scope, context, diagnostics);
            if !matches!(key_ty, ExprType::String | ExprType::Unknown) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} indexes a map with a non-string key", context.subject),
                    suggestion: Some(
                        "use a string literal or string expression as the map key".to_owned(),
                    ),
                });
            }
            match target_ty {
                ExprType::Map(inner) => *inner,
                ExprType::Unknown => ExprType::Unknown,
                _ => {
                    diagnostics.push(Diagnostic {
                        span: context.span,
                        message: format!("{} indexes a non-map expression", context.subject),
                        suggestion: Some("use indexing only on map values".to_owned()),
                    });
                    ExprType::Unknown
                }
            }
        }
        Expr::Array(items) => infer_array_type(items, semantic, scope, context, diagnostics),
        Expr::Object(fields) => {
            for field in fields {
                infer_expr_type(&field.value, semantic, scope, context, diagnostics);
            }
            ExprType::Object
        }
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
        } => {
            let inner = infer_expr_type(expr, semantic, scope, context, diagnostics);
            if !matches!(inner, ExprType::Bool | ExprType::Unknown) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} applies `!` to a non-boolean expression",
                        context.subject
                    ),
                    suggestion: Some("use `!` only with boolean expressions".to_owned()),
                });
            }
            ExprType::Bool
        }
        Expr::Binary { op, left, right } => {
            infer_binary_type(*op, left, right, semantic, scope, context, diagnostics)
        }
        Expr::Call { name, args } => match name.as_str() {
            "count" => ExprType::Int,
            "exists" | "empty" | "one" | "none" => ExprType::Bool,
            _ => {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} calls unsupported expression function `{name}`",
                        context.subject
                    ),
                    suggestion: Some("use `count`, `exists`, `empty`, `one`, or `none`".to_owned()),
                });
                for arg in args {
                    infer_expr_type(arg, semantic, scope, context, diagnostics);
                }
                ExprType::Unknown
            }
        },
        Expr::Query { guard, .. } => {
            if let Some(guard) = guard {
                let guard_scope = query_guard_scope(expr, semantic, scope);
                infer_expr_type(guard, semantic, &guard_scope, context, diagnostics);
            }
            ExprType::Collection
        }
    }
}

fn infer_binary_type(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> ExprType {
    let left_ty = infer_expr_type(left, semantic, scope, context, diagnostics);
    let right_ty = infer_expr_type(right, semantic, scope, context, diagnostics);
    match op {
        BinaryOp::And | BinaryOp::Or => {
            for ty in [&left_ty, &right_ty] {
                if !matches!(ty, ExprType::Bool | ExprType::Unknown) {
                    diagnostics.push(Diagnostic {
                        span: context.span,
                        message: format!(
                            "{} uses boolean operator with non-boolean operand",
                            context.subject
                        ),
                        suggestion: Some(
                            "use `&&` and `||` only with boolean expressions".to_owned(),
                        ),
                    });
                    break;
                }
            }
            ExprType::Bool
        }
        BinaryOp::Eq | BinaryOp::Ne => {
            if !types_comparable(&left_ty, &right_ty) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} compares incompatible expression types", context.subject),
                    suggestion: Some(
                        "compare values with compatible scalar or finite-domain types".to_owned(),
                    ),
                });
            }
            ExprType::Bool
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            if !is_orderable_pair(&left_ty, &right_ty) {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} orders non-orderable expression values", context.subject),
                    suggestion: Some(
                        "use ordering only with int, float, duration, or time values".to_owned(),
                    ),
                });
            }
            ExprType::Bool
        }
        BinaryOp::In | BinaryOp::NotIn => {
            match &right_ty {
                ExprType::Array(item_ty) => {
                    if !types_comparable(&left_ty, item_ty) {
                        diagnostics.push(Diagnostic {
                            span: context.span,
                            message: format!(
                                "{} uses membership with incompatible item type",
                                context.subject
                            ),
                            suggestion: Some(
                                "make the left value compatible with the array item type"
                                    .to_owned(),
                            ),
                        });
                    }
                }
                ExprType::Map(_) => {
                    if !is_string_like_key_type(&left_ty) {
                        diagnostics.push(Diagnostic {
                            span: context.span,
                            message: format!(
                                "{} uses map membership with a non-string key",
                                context.subject
                            ),
                            suggestion: Some(
                                "use a string value on the left side of map membership".to_owned(),
                            ),
                        });
                    }
                }
                ExprType::Unknown => {}
                _ => diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} uses membership against a non-array/non-map expression",
                        context.subject
                    ),
                    suggestion: Some(
                        "use `in` with an array literal, array value, or map value".to_owned(),
                    ),
                }),
            }
            ExprType::Bool
        }
    }
}

fn infer_array_type(
    items: &[Expr],
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> ExprType {
    let mut item_ty: Option<ExprType> = None;
    for item in items {
        let ty = infer_expr_type(item, semantic, scope, context, diagnostics);
        if matches!(ty, ExprType::Unknown) {
            continue;
        }
        match &item_ty {
            None => item_ty = Some(ty),
            Some(existing) if types_comparable(existing, &ty) => {}
            Some(_) => {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!("{} has mixed-type array literal", context.subject),
                    suggestion: Some("use array literals whose elements share one type".to_owned()),
                });
                return ExprType::Array(Box::new(ExprType::Unknown));
            }
        }
    }
    ExprType::Array(Box::new(item_ty.unwrap_or(ExprType::Unknown)))
}

fn expr_path_type(
    path: &[String],
    semantic: &SemanticContext,
    scope: &ExprScope,
) -> Option<ExprType> {
    if path.len() < 2 {
        return None;
    }
    if let Some(schema) = scope.binding_types.get(&path[0]) {
        return semantic
            .schemas
            .resolve_field_path(schema, &path[1..])
            .ok()
            .map(|ty| expr_type_from_type_syntax(&ty, semantic));
    }
    let schema = scope.implicit_schema.as_ref()?;
    semantic
        .schemas
        .resolve_field_path(schema, path)
        .ok()
        .map(|ty| expr_type_from_type_syntax(&ty, semantic))
}

fn expr_type_from_type_syntax(ty: &TypeSyntax, semantic: &SemanticContext) -> ExprType {
    match ty {
        TypeSyntax::Primitive { name, .. } => match name.as_str() {
            "bool" => ExprType::Bool,
            "int" => ExprType::Int,
            "float" => ExprType::Float,
            "string" => ExprType::String,
            "duration" => ExprType::Duration,
            "time" => ExprType::Time,
            _ => ExprType::Unknown,
        },
        TypeSyntax::LiteralString { value, .. } => ExprType::Finite {
            label: "literal".to_owned(),
            values: vec![value.clone()],
        },
        TypeSyntax::AgentRef { agents, .. } => ExprType::Finite {
            label: "AgentRef".to_owned(),
            values: agents.iter().map(|agent| agent.name.clone()).collect(),
        },
        TypeSyntax::Ref { name } => semantic
            .schemas
            .enums
            .get(&name.name)
            .map(|variants| ExprType::Finite {
                label: format!("enum `{}`", name.name),
                values: variants.iter().cloned().collect(),
            })
            .unwrap_or(ExprType::Object),
        TypeSyntax::Optional { inner, .. } => {
            ExprType::Optional(Box::new(expr_type_from_type_syntax(inner, semantic)))
        }
        TypeSyntax::Array { inner, .. } => {
            ExprType::Array(Box::new(expr_type_from_type_syntax(inner, semantic)))
        }
        TypeSyntax::Map { inner, .. } => {
            ExprType::Map(Box::new(expr_type_from_type_syntax(inner, semantic)))
        }
        TypeSyntax::Union { variants, .. } => {
            let values = variants
                .iter()
                .filter_map(|variant| match variant {
                    TypeSyntax::LiteralString { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if values.len() == variants.len() && !values.is_empty() {
                ExprType::Finite {
                    label: "literal union".to_owned(),
                    values,
                }
            } else {
                ExprType::Unknown
            }
        }
    }
}

fn expr_literal_type(literal: &ExprLiteral) -> ExprType {
    match literal {
        ExprLiteral::String(_) | ExprLiteral::Ident(_) => ExprType::String,
        ExprLiteral::Number(value) if value.contains('.') => ExprType::Float,
        ExprLiteral::Number(_) => ExprType::Int,
        ExprLiteral::Bool(_) => ExprType::Bool,
        ExprLiteral::Null => ExprType::Null,
    }
}

fn types_comparable(left: &ExprType, right: &ExprType) -> bool {
    if matches!(left, ExprType::Unknown) || matches!(right, ExprType::Unknown) {
        return true;
    }
    if matches!(left, ExprType::Null) || matches!(right, ExprType::Null) {
        return true;
    }
    if is_numeric_type(left) && is_numeric_type(right) {
        return true;
    }
    match (left, right) {
        (ExprType::Optional(left), right) | (right, ExprType::Optional(left)) => {
            types_comparable(left, right)
        }
        (ExprType::Finite { .. }, ExprType::String)
        | (ExprType::String, ExprType::Finite { .. })
        | (ExprType::Finite { .. }, ExprType::Finite { .. }) => true,
        _ => left == right,
    }
}

fn is_numeric_type(ty: &ExprType) -> bool {
    matches!(ty, ExprType::Int | ExprType::Float)
}

fn is_string_like_key_type(ty: &ExprType) -> bool {
    match ty {
        ExprType::String | ExprType::Unknown | ExprType::Finite { .. } => true,
        ExprType::Optional(inner) => is_string_like_key_type(inner),
        _ => false,
    }
}

fn is_orderable_pair(left: &ExprType, right: &ExprType) -> bool {
    if matches!(left, ExprType::Unknown) || matches!(right, ExprType::Unknown) {
        return true;
    }
    (is_numeric_type(left) && is_numeric_type(right))
        || matches!(
            (left, right),
            (ExprType::Duration, ExprType::Duration) | (ExprType::Time, ExprType::Time)
        )
}

fn is_countable_type(ty: &ExprType) -> bool {
    matches!(
        ty,
        ExprType::Array(_) | ExprType::Map(_) | ExprType::Collection | ExprType::Unknown
    )
}

fn is_exists_type(ty: &ExprType) -> bool {
    matches!(
        ty,
        ExprType::Array(_)
            | ExprType::Map(_)
            | ExprType::Collection
            | ExprType::Optional(_)
            | ExprType::Unknown
    )
}

fn is_empty_type(ty: &ExprType) -> bool {
    match ty {
        ExprType::String
        | ExprType::Array(_)
        | ExprType::Map(_)
        | ExprType::Collection
        | ExprType::Null
        | ExprType::Unknown => true,
        ExprType::Optional(inner) => is_empty_type(inner),
        _ => false,
    }
}

fn expr_type_label(ty: &ExprType) -> String {
    match ty {
        ExprType::Bool => "bool".to_owned(),
        ExprType::Int => "int".to_owned(),
        ExprType::Float => "float".to_owned(),
        ExprType::String => "string".to_owned(),
        ExprType::Finite { label, values } => format!("{label}<{}>", values.join(" | ")),
        ExprType::Duration => "duration".to_owned(),
        ExprType::Time => "time".to_owned(),
        ExprType::Null => "null".to_owned(),
        ExprType::Object => "object".to_owned(),
        ExprType::Array(inner) => format!("{}[]", expr_type_label(inner)),
        ExprType::Map(inner) => format!("map<{}>", expr_type_label(inner)),
        ExprType::Optional(inner) => format!("{}?", expr_type_label(inner)),
        ExprType::Collection => "query".to_owned(),
        ExprType::Unknown => "unknown".to_owned(),
    }
}

fn validate_finite_domain_expr(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::In | BinaryOp::NotIn
    ) {
        return;
    }
    let Some((domain, literals)) = finite_domain_comparison(left, right, semantic, scope)
        .or_else(|| finite_domain_comparison(right, left, semantic, scope))
    else {
        validate_finite_domain_relation(op, left, right, semantic, scope, context, diagnostics);
        return;
    };
    for literal in literals.into_iter().flatten() {
        if !domain.iter().any(|value| value == &literal) {
            diagnostics.push(Diagnostic {
                span: context.span,
                message: format!(
                    "{} compares finite-domain value to unknown `{literal}`",
                    context.subject
                ),
                suggestion: Some(format!("use one of: {}", domain.join(", "))),
            });
        }
    }
    validate_finite_domain_relation(op, left, right, semantic, scope, context, diagnostics);
}

fn validate_finite_domain_relation(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    context: &ExprValidationContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match op {
        BinaryOp::Eq => {
            let Some(left_domain) = expr_domain(left, semantic, scope) else {
                return;
            };
            let Some(right_domain) = expr_domain(right, semantic, scope) else {
                return;
            };
            if left_domain
                .iter()
                .all(|value| !right_domain.iter().any(|right| right == value))
            {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} has statically unsatisfiable finite-domain equality",
                        context.subject
                    ),
                    suggestion: Some(format!(
                        "compare domains with at least one shared value; left: {}, right: {}",
                        left_domain.join(", "),
                        right_domain.join(", ")
                    )),
                });
            }
        }
        BinaryOp::In => {
            let Some(domain) = expr_domain(left, semantic, scope) else {
                return;
            };
            let Some(literals) = literal_array_values(right) else {
                return;
            };
            if literals
                .iter()
                .all(|literal| !domain.iter().any(|value| value == literal))
            {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} has statically unsatisfiable finite-domain membership",
                        context.subject
                    ),
                    suggestion: Some(format!("use one of: {}", domain.join(", "))),
                });
            }
        }
        BinaryOp::NotIn => {
            let Some(domain) = expr_domain(left, semantic, scope) else {
                return;
            };
            let Some(literals) = literal_array_values(right) else {
                return;
            };
            if !domain.is_empty()
                && domain
                    .iter()
                    .all(|value| literals.iter().any(|literal| literal == value))
            {
                diagnostics.push(Diagnostic {
                    span: context.span,
                    message: format!(
                        "{} has statically unsatisfiable finite-domain exclusion",
                        context.subject
                    ),
                    suggestion: Some(
                        "leave at least one domain value outside the exclusion set".to_owned(),
                    ),
                });
            }
        }
        _ => {}
    }
}

fn finite_domain_comparison(
    domain_expr: &Expr,
    literal_expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
) -> Option<(Vec<String>, Vec<Option<String>>)> {
    let domain = expr_domain(domain_expr, semantic, scope)?;
    let literals = match literal_expr {
        Expr::Literal(literal) => vec![expr_literal_name(literal)],
        Expr::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Expr::Literal(literal) => Some(expr_literal_name(literal)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some((domain, literals))
}

fn expr_domain(expr: &Expr, semantic: &SemanticContext, scope: &ExprScope) -> Option<Vec<String>> {
    let ty = match expr {
        Expr::Path(path) => {
            let root = path.first()?;
            if let Some(schema) = scope.binding_types.get(root) {
                semantic
                    .schemas
                    .resolve_field_path(schema, path.get(1..)?)
                    .ok()?
            } else {
                let schema = scope.implicit_schema.as_ref()?;
                semantic.schemas.resolve_field_path(schema, path).ok()?
            }
        }
        Expr::Literal(ExprLiteral::Ident(name)) => implicit_field_type(name, semantic, scope)?,
        _ => return None,
    };
    finite_expr_domain(&ty, semantic)
}

fn finite_expr_domain(ty: &TypeSyntax, semantic: &SemanticContext) -> Option<Vec<String>> {
    match ty {
        TypeSyntax::Ref { name } => semantic
            .schemas
            .enums
            .get(&name.name)
            .map(|variants| variants.iter().cloned().collect()),
        TypeSyntax::Union { variants, .. } => {
            let values = variants
                .iter()
                .filter_map(|variant| match variant {
                    TypeSyntax::LiteralString { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(values)
        }
        TypeSyntax::AgentRef { agents, .. } => {
            Some(agents.iter().map(|agent| agent.name.clone()).collect())
        }
        _ => None,
    }
}

fn expr_literal_name(literal: &ExprLiteral) -> Option<String> {
    match literal {
        ExprLiteral::String(value) | ExprLiteral::Ident(value) => Some(value.clone()),
        _ => None,
    }
}

fn literal_array_values(expr: &Expr) -> Option<Vec<String>> {
    let Expr::Array(items) = expr else {
        return None;
    };
    items
        .iter()
        .map(|item| match item {
            Expr::Literal(literal) => expr_literal_name(literal),
            _ => None,
        })
        .collect()
}

fn parse_tell_target(line: &str) -> Option<&str> {
    line.strip_prefix("tell ")?
        .split_whitespace()
        .next()
        .filter(|target| !target.is_empty())
}

fn parse_required_capabilities(line: &str) -> Vec<String> {
    let Some(rest) = line.split_once(" requires ") else {
        return Vec::new();
    };
    let Some(list) = rest.1.trim_start().strip_prefix('[') else {
        return Vec::new();
    };
    let Some((items, _)) = list.split_once(']') else {
        return Vec::new();
    };
    let mut capabilities = items
        .split(',')
        .filter_map(|item| {
            let value = item.trim().trim_matches('"');
            (!value.is_empty()).then(|| value.to_owned())
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn validate_case_blocks(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lines = rule
        .body
        .text
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((line, current))
        })
        .collect::<Vec<_>>();
    let text_lines = lines.iter().map(|(line, _)| *line).collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].0.trim();
        let Some(scrutinee) = case_scrutinee(trimmed) else {
            index += 1;
            continue;
        };
        let scrutinee_ty = expression_type(scrutinee, semantic, binding_types);
        let terminal_case = scrutinee_ty.is_none()
            && active_completes_binding_for_case(&text_lines, index, scrutinee);
        if scrutinee_ty.is_none() && !terminal_case {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` has case scrutinee `{scrutinee}` that is not a typed path",
                    rule.name.name
                ),
                suggestion: Some("match on a bound field such as `task.provider`".to_owned()),
            });
        }
        let mut depth = brace_delta(trimmed).max(1);
        let mut case_index = index + 1;
        let mut branches = Vec::new();
        while case_index < lines.len() && depth > 0 {
            let (raw_line, line_offset) = lines[case_index];
            let line = raw_line.trim();
            if depth == 1 {
                if let Some(branch) = parse_case_branch_head(line) {
                    let pattern_column = case_pattern_column(raw_line, branch.pattern);
                    let branch = SpanCaseBranchHead {
                        pattern: branch.pattern,
                        guard: branch.guard,
                        pattern_span: SourceSpan {
                            start: rule_body_text_start(rule) + line_offset + pattern_column,
                            end: rule_body_text_start(rule)
                                + line_offset
                                + pattern_column
                                + branch.pattern.len(),
                        },
                    };
                    branches.push(branch);
                    if terminal_case {
                        validate_terminal_case_pattern(
                            rule,
                            branch.pattern,
                            branch.pattern_span,
                            diagnostics,
                        );
                    } else {
                        validate_case_pattern(
                            rule,
                            branch.pattern,
                            scrutinee_ty.as_ref(),
                            branch.pattern_span,
                            semantic,
                            diagnostics,
                        );
                    }
                    if let Some(guard) = branch.guard {
                        let mut branch_scope = binding_types.clone();
                        if let Some(scrutinee_ty) = scrutinee_ty.as_ref() {
                            if let Some((binding, schema)) =
                                case_branch_payload_binding(branch.pattern, scrutinee_ty, semantic)
                            {
                                branch_scope.insert(binding, schema);
                            }
                        }
                        validate_expression(
                            rule,
                            guard,
                            semantic,
                            &branch_scope,
                            "case guard",
                            diagnostics,
                        );
                        validate_known_field_paths_at_span(
                            rule,
                            guard,
                            branch.pattern_span,
                            semantic,
                            &branch_scope,
                            diagnostics,
                        );
                    }
                }
            }
            depth += brace_delta(line);
            case_index += 1;
        }
        if terminal_case {
            validate_terminal_case_coverage(rule, &branches, diagnostics);
        } else {
            validate_case_coverage(
                rule,
                scrutinee_ty.as_ref(),
                &branches,
                semantic,
                diagnostics,
            );
        }
        index += 1;
    }
}

fn active_completes_binding_for_case(lines: &[&str], case_index: usize, scrutinee: &str) -> bool {
    let mut scopes: Vec<(String, DependencyPredicate, i32)> = Vec::new();
    for line in lines.iter().take(case_index) {
        let trimmed = line.trim();
        if let Some((binding, predicate)) = parse_after_line(trimmed) {
            scopes.push((binding, predicate, brace_delta(trimmed).max(1)));
        } else {
            let delta = brace_delta(trimmed);
            for (_, _, depth) in &mut scopes {
                *depth += delta;
            }
            scopes.retain(|(_, _, depth)| *depth > 0);
        }
    }
    scopes.iter().any(|(binding, predicate, _)| {
        binding == scrutinee && predicate == &DependencyPredicate::Completes
    })
}

fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

fn case_scrutinee(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("case ")?;
    let expr = rest.strip_suffix('{').unwrap_or(rest).trim();
    (!expr.is_empty()).then_some(expr)
}

fn is_case_branch_start(line: &str) -> bool {
    line.contains("=>")
}

#[derive(Clone, Copy)]
struct CaseBranchHead<'a> {
    pattern: &'a str,
    guard: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct SpanCaseBranchHead<'a> {
    pattern: &'a str,
    guard: Option<&'a str>,
    pattern_span: SourceSpan,
}

fn parse_case_branch_head(line: &str) -> Option<CaseBranchHead<'_>> {
    let (pattern, _) = line.split_once("=>")?;
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    match pattern.split_once(" where ") {
        Some((pattern, guard)) => Some(CaseBranchHead {
            pattern: pattern.trim(),
            guard: Some(guard.trim()),
        }),
        None => Some(CaseBranchHead {
            pattern,
            guard: None,
        }),
    }
}

fn expression_type(
    expr: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
) -> Option<TypeSyntax> {
    let (root, path) = expression_path(expr)?;
    let schema = binding_types.get(&root)?;
    semantic.schemas.resolve_field_path(schema, &path).ok()
}

fn validate_case_pattern(
    rule: &RuleDecl,
    pattern: &str,
    scrutinee_ty: Option<&TypeSyntax>,
    span: SourceSpan,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if matches!(pattern, "_" | "default") {
        return;
    }
    if pattern == "None" {
        if !matches!(scrutinee_ty, Some(TypeSyntax::Optional { .. })) {
            diagnostics.push(Diagnostic {
                span,
                message: format!(
                    "rule `{}` uses `None` for a non-optional case",
                    rule.name.name
                ),
                suggestion: Some("use `None` only when matching an optional field".to_owned()),
            });
        }
        return;
    }
    if pattern.starts_with("Some ") {
        if !matches!(scrutinee_ty, Some(TypeSyntax::Optional { .. })) {
            diagnostics.push(Diagnostic {
                span,
                message: format!(
                    "rule `{}` uses `Some` for a non-optional case",
                    rule.name.name
                ),
                suggestion: Some("use `Some name` only when matching an optional field".to_owned()),
            });
        }
        return;
    }
    let Some(scrutinee_ty) = scrutinee_ty else {
        return;
    };
    match scrutinee_ty {
        TypeSyntax::Ref { name } => {
            let Some(variants) = semantic.schemas.enums.get(&name.name) else {
                return;
            };
            if !variants.contains(pattern) {
                diagnostics.push(Diagnostic {
                    span,
                    message: format!("enum `{}` has no variant `{pattern}`", name.name),
                    suggestion: Some(format!(
                        "use one of: {}",
                        variants.iter().cloned().collect::<Vec<_>>().join(", ")
                    )),
                });
            }
        }
        TypeSyntax::Union { variants, .. } => {
            let Some(literal) = parse_literal_expr(pattern) else {
                diagnostics.push(Diagnostic {
                    span,
                    message: format!(
                        "rule `{}` has unsupported case pattern `{pattern}`",
                        rule.name.name
                    ),
                    suggestion: Some("use a literal branch value or `_`".to_owned()),
                });
                return;
            };
            validate_union_case_pattern(rule, variants, &literal, span, diagnostics);
        }
        TypeSyntax::AgentRef { agents, .. } => {
            let Some(literal) = parse_literal_expr(pattern) else {
                diagnostics.push(Diagnostic {
                    span,
                    message: format!(
                        "rule `{}` has unsupported AgentRef case pattern `{pattern}`",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "use a declared agent name, a string literal, or `_`".to_owned(),
                    ),
                });
                return;
            };
            validate_agent_ref_case_pattern(rule, agents, &literal, span, diagnostics);
        }
        TypeSyntax::Optional { inner, .. } => {
            validate_case_pattern(rule, pattern, Some(inner), span, semantic, diagnostics);
        }
        _ => {
            diagnostics.push(Diagnostic {
                span,
                message: format!(
                    "rule `{}` cannot pattern-match this scrutinee type",
                    rule.name.name
                ),
                suggestion: Some(
                    "match an enum, literal union, optional, or tagged output union".to_owned(),
                ),
            });
        }
    }
}

fn terminal_case_tags() -> [&'static str; 4] {
    ["Completed", "Failed", "TimedOut", "Cancelled"]
}

fn validate_terminal_case_pattern(
    rule: &RuleDecl,
    pattern: &str,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if is_fallback_pattern(pattern) {
        return;
    }
    let mut parts = pattern.split_whitespace();
    let Some(tag) = parts.next() else {
        return;
    };
    let binding = parts.next();
    if parts.next().is_some() || binding.is_none() {
        diagnostics.push(Diagnostic {
            span,
            message: format!(
                "rule `{}` has malformed terminal-output case pattern `{pattern}`",
                rule.name.name
            ),
            suggestion: Some("write `Completed result`, `Failed failure`, `TimedOut timeout`, or `Cancelled cancel`".to_owned()),
        });
        return;
    }
    let tags = terminal_case_tags();
    if !tags.contains(&tag) {
        diagnostics.push(Diagnostic {
            span,
            message: format!(
                "rule `{}` terminal-output case pattern cannot be `{tag}`",
                rule.name.name
            ),
            suggestion: Some(format!("use one of: {}", tags.join(", "))),
        });
    }
}

fn validate_terminal_case_coverage(
    rule: &RuleDecl,
    branches: &[SpanCaseBranchHead<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if branches.is_empty()
        || branches
            .iter()
            .any(|branch| is_fallback_pattern(branch.pattern))
    {
        validate_duplicate_terminal_case_patterns(rule, branches, diagnostics);
        return;
    }
    validate_duplicate_terminal_case_patterns(rule, branches, diagnostics);
    let covered = branches
        .iter()
        .filter(|branch| branch.guard.is_none())
        .filter_map(|branch| normalized_terminal_case_pattern(branch.pattern))
        .collect::<BTreeSet<_>>();
    let missing = terminal_case_tags()
        .iter()
        .filter(|tag| !covered.contains(**tag))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` has non-exhaustive terminal-output case; missing {}",
                rule.name.name,
                missing.join(", ")
            ),
            suggestion: Some(
                "add terminal branches for every value or add `_ => { ... }`".to_owned(),
            ),
        });
    }
}

fn validate_duplicate_terminal_case_patterns(
    rule: &RuleDecl,
    branches: &[SpanCaseBranchHead<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for branch in branches.iter().filter(|branch| branch.guard.is_none()) {
        let Some(pattern) = normalized_terminal_case_pattern(branch.pattern) else {
            continue;
        };
        if !seen.insert(pattern.to_owned()) {
            diagnostics.push(Diagnostic {
                span: branch.pattern_span,
                message: format!(
                    "rule `{}` has duplicate unguarded terminal-output case pattern `{pattern}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "remove the duplicate branch or add mutually exclusive `where` guards"
                        .to_owned(),
                ),
            });
        }
    }
}

fn validate_case_coverage(
    rule: &RuleDecl,
    scrutinee_ty: Option<&TypeSyntax>,
    branches: &[SpanCaseBranchHead<'_>],
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if branches.is_empty()
        || branches
            .iter()
            .any(|branch| is_fallback_pattern(branch.pattern))
    {
        validate_duplicate_case_patterns(rule, branches, diagnostics);
        return;
    }
    validate_duplicate_case_patterns(rule, branches, diagnostics);

    let Some(domain) = finite_case_domain(scrutinee_ty, semantic) else {
        return;
    };
    let covered = branches
        .iter()
        .filter(|branch| branch.guard.is_none())
        .filter_map(|branch| normalized_case_pattern(branch.pattern))
        .collect::<BTreeSet<_>>();
    let missing = domain
        .iter()
        .filter(|value| !covered.contains(value.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` has non-exhaustive case; missing {}",
                rule.name.name,
                missing.join(", ")
            ),
            suggestion: Some("add branches for every value or add `_ => { ... }`".to_owned()),
        });
    }
}

fn validate_duplicate_case_patterns(
    rule: &RuleDecl,
    branches: &[SpanCaseBranchHead<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for branch in branches.iter().filter(|branch| branch.guard.is_none()) {
        let Some(pattern) = normalized_case_pattern(branch.pattern) else {
            continue;
        };
        if !seen.insert(pattern.to_owned()) {
            diagnostics.push(Diagnostic {
                span: branch.pattern_span,
                message: format!(
                    "rule `{}` has duplicate unguarded case pattern `{pattern}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "remove the duplicate branch or add mutually exclusive `where` guards"
                        .to_owned(),
                ),
            });
        }
    }
}

fn finite_case_domain(
    scrutinee_ty: Option<&TypeSyntax>,
    semantic: &SemanticContext,
) -> Option<Vec<String>> {
    match scrutinee_ty? {
        TypeSyntax::Ref { name } => semantic
            .schemas
            .enums
            .get(&name.name)
            .map(|variants| variants.iter().cloned().collect()),
        TypeSyntax::Union { variants, .. } => {
            let values = variants
                .iter()
                .filter_map(|variant| match variant {
                    TypeSyntax::LiteralString { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(values)
        }
        TypeSyntax::Optional { .. } => Some(vec!["Some".to_owned(), "None".to_owned()]),
        TypeSyntax::AgentRef { agents, .. } => {
            Some(agents.iter().map(|agent| agent.name.clone()).collect())
        }
        _ => None,
    }
}

fn normalized_case_pattern(pattern: &str) -> Option<&str> {
    if is_fallback_pattern(pattern) {
        return None;
    }
    if pattern.starts_with("Some ") {
        return Some("Some");
    }
    if pattern == "None" {
        return Some("None");
    }
    parse_literal_expr(pattern).and_then(|literal| match literal {
        LiteralExpr::String(value) | LiteralExpr::Ident(value) => Some(value),
        _ => None,
    })
}

fn normalized_terminal_case_pattern(pattern: &str) -> Option<&str> {
    if is_fallback_pattern(pattern) {
        return None;
    }
    pattern.split_whitespace().next()
}

fn is_fallback_pattern(pattern: &str) -> bool {
    matches!(pattern, "_" | "default")
}

fn validate_union_case_pattern(
    rule: &RuleDecl,
    variants: &[TypeSyntax],
    literal: &LiteralExpr<'_>,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = variants
        .iter()
        .filter_map(|variant| match variant {
            TypeSyntax::LiteralString { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if allowed.is_empty() {
        return;
    }
    let LiteralExpr::String(value) = literal else {
        diagnostics.push(Diagnostic {
            span,
            message: format!(
                "rule `{}` case pattern must be one of its literal variants",
                rule.name.name
            ),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            span,
            message: format!("rule `{}` case pattern cannot be `{value}`", rule.name.name),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
    }
}

fn validate_agent_ref_case_pattern(
    rule: &RuleDecl,
    agents: &[Ident],
    literal: &LiteralExpr<'_>,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<Vec<_>>();
    let (LiteralExpr::String(value) | LiteralExpr::Ident(value)) = literal else {
        diagnostics.push(Diagnostic {
            span,
            message: format!("rule `{}` has non-agent case pattern", rule.name.name),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            span,
            message: format!("AgentRef has no agent `{value}`"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
    }
}

fn validate_binding_uses(
    rule: &RuleDecl,
    line: &str,
    seen_bindings: &BTreeSet<String>,
    scope_stack: &[(String, DependencyPredicate)],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for root in interpolation_roots(line) {
        if !seen_bindings.contains(&root) {
            continue;
        }
        if scope_stack.iter().any(|(binding, _)| binding == &root) {
            continue;
        }

        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` uses effect output `{root}` outside a matching `after {root} ...` block",
                rule.name.name
            ),
            suggestion: Some(format!(
                "move this use into `after {root} succeeds {{ ... }}` or another matching terminal branch"
            )),
        });
    }
}

fn after_scopes(block_stack: &[BlockFrame]) -> Vec<(String, DependencyPredicate)> {
    block_stack
        .iter()
        .filter_map(|frame| match frame {
            BlockFrame::After { binding, predicate } => Some((binding.clone(), predicate.clone())),
        })
        .collect()
}

fn binding_from_when(when: &str) -> Option<(String, String)> {
    let (pattern, _) = split_when_guard(when);
    let binding = binding_after_as(pattern)?;
    let first = pattern.split_whitespace().next()?;
    let schema = if first.chars().next().is_some_and(char::is_uppercase) {
        first.to_owned()
    } else if pattern.starts_with("loft has ready issue ") {
        "LoftIssue".to_owned()
    } else if pattern.starts_with("worker completed turn ") {
        "AgentTurn".to_owned()
    } else if pattern.starts_with("human answered ") {
        "HumanAnswer".to_owned()
    } else if pattern.starts_with("manual review requested ") {
        "WorkItem".to_owned()
    } else {
        return None;
    };

    Some((binding, schema))
}

fn split_when_guard(when: &str) -> (&str, Option<&str>) {
    match when.split_once(" where ") {
        Some((pattern, guard)) => (pattern.trim(), Some(guard.trim())),
        None => (when.trim(), None),
    }
}

fn effect_binding_schema(
    line: &str,
    kind: &IrEffectKind,
    semantic: &SemanticContext,
) -> Option<String> {
    match kind {
        IrEffectKind::LoftClaim => Some("LoftClaim".to_owned()),
        IrEffectKind::HumanAsk => Some("HumanAnswer".to_owned()),
        IrEffectKind::BamlCoerce => parse_coerce_call_name(line).and_then(|name| {
            semantic
                .coerce_outputs
                .get(name)
                .and_then(schema_name_for_path)
        }),
        IrEffectKind::AgentTell
        | IrEffectKind::CapabilityCall
        | IrEffectKind::EventEmit
        | IrEffectKind::WorkflowInvoke => None,
    }
}

fn parse_coerce_call_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("coerce ")?;
    rest.split_once('(').map(|(name, _)| name.trim())
}

fn parse_coerce_call(line: &str) -> Option<(&str, Vec<&str>)> {
    let rest = line.strip_prefix("coerce ")?;
    let call = rest.split(" as ").next().unwrap_or(rest).trim();
    let (name, tail) = call.split_once('(')?;
    let (args, _) = tail.rsplit_once(')')?;
    Some((name.trim(), split_expression_args(args)))
}

fn split_expression_args(args: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut previous = '\0';
    for (index, ch) in args.char_indices() {
        if ch == '"' && previous != '\\' {
            in_string = !in_string;
        } else if !in_string {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    let value = args[start..index].trim();
                    if !value.is_empty() {
                        values.push(value);
                    }
                    start = index + ch.len_utf8();
                }
                _ => {}
            }
        }
        previous = ch;
    }
    let value = args[start..].trim();
    if !value.is_empty() {
        values.push(value);
    }
    values
}

fn parse_loft_claim_issue_expr(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("claim ")?;
    let (issue_expr, _) = rest.split_once(" with loft")?;
    let issue_expr = issue_expr.trim();
    (!issue_expr.is_empty()).then_some(issue_expr)
}

fn effect_payload_statements(body: &str) -> Vec<String> {
    collect_body_statements(body, effect_payload_statement_balance)
}

fn workflow_invoke_statements(body: &str) -> Vec<String> {
    collect_body_statements(body, workflow_invoke_statement_balance)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatementBalance {
    None,
    Parens,
    Braces,
}

fn collect_body_statements(
    body: &str,
    statement_balance: fn(&str) -> Option<StatementBalance>,
) -> Vec<String> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut index = 0usize;
    let mut record_depth = 0i32;
    let mut multiline_string = false;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if multiline_string {
            if trimmed.contains("\"\"\"") {
                multiline_string = false;
            }
            index += 1;
            continue;
        }
        if record_depth > 0 {
            record_depth += brace_delta(trimmed);
            index += 1;
            continue;
        }
        if parse_record_start(trimmed).is_some() {
            record_depth = brace_delta(trimmed).max(1);
            index += 1;
            continue;
        }
        if trimmed.contains("\"\"\"") {
            multiline_string = trimmed.matches("\"\"\"").count() % 2 == 1;
            index += 1;
            continue;
        }
        if let Some(balance) = statement_balance(trimmed) {
            match balance {
                StatementBalance::None => statements.push(trimmed.to_owned()),
                StatementBalance::Parens => {
                    let (statement, next_index) =
                        statement_until_balanced(&lines, index, trimmed, paren_delta);
                    statements.push(statement);
                    index = next_index + 1;
                    continue;
                }
                StatementBalance::Braces => {
                    let (statement, next_index) =
                        statement_until_balanced(&lines, index, trimmed, brace_delta);
                    statements.push(statement);
                    index = next_index + 1;
                    continue;
                }
            }
        }
        index += 1;
    }
    statements
}

fn effect_payload_statement_balance(trimmed: &str) -> Option<StatementBalance> {
    if trimmed.starts_with("coerce ") {
        Some(StatementBalance::Parens)
    } else if trimmed.starts_with("claim ") {
        Some(StatementBalance::None)
    } else {
        None
    }
}

fn workflow_invoke_statement_balance(trimmed: &str) -> Option<StatementBalance> {
    trimmed
        .starts_with("invoke ")
        .then_some(StatementBalance::Braces)
}

fn invoke_statement_parts(statement: &str) -> Option<(&str, &str)> {
    let rest = statement.trim().strip_prefix("invoke ")?;
    let target = rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('{');
    if target.is_empty() {
        return None;
    }
    let open = statement.find('{')?;
    let close = statement.rfind('}')?;
    (close > open).then_some((target, statement[open + 1..close].trim()))
}

fn statement_until_balanced(
    lines: &[&str],
    index: usize,
    trimmed: &str,
    delta: fn(&str) -> i32,
) -> (String, usize) {
    let mut statement = trimmed.to_owned();
    let mut depth = delta(trimmed);
    let mut cursor = index;
    while depth > 0 && cursor + 1 < lines.len() {
        cursor += 1;
        let next = lines[cursor].trim();
        statement.push(' ');
        statement.push_str(next);
        depth += delta(next);
    }
    (statement, cursor)
}

fn paren_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    })
}

fn validate_known_field_paths(
    rule: &RuleDecl,
    line: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    validate_known_field_paths_at_span(
        rule,
        line,
        rule.body.span,
        semantic,
        binding_types,
        diagnostics,
    );
}

fn validate_known_field_paths_at_span(
    rule: &RuleDecl,
    line: &str,
    span: SourceSpan,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (root, path) in dotted_paths(line) {
        let Some(schema) = binding_types.get(&root) else {
            continue;
        };
        if !semantic.schemas.class_exists(schema) {
            continue;
        }
        if let Err(message) = semantic.schemas.resolve_field_path(schema, &path) {
            diagnostics.push(Diagnostic {
                span,
                message: format!(
                    "rule `{}` has invalid field path `{root}.{}`: {message}",
                    rule.name.name,
                    path.join(".")
                ),
                suggestion: Some(
                    "use a field declared on the bound schema or add it to the class declaration"
                        .to_owned(),
                ),
            });
        }
    }
}

fn dotted_paths(line: &str) -> Vec<(String, Vec<String>)> {
    let bytes = line.as_bytes();
    let mut paths = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if !is_ident_start(bytes[index]) {
            index += 1;
            continue;
        }

        let root_start = index;
        index += 1;
        while index < bytes.len() && is_ident_continue(bytes[index]) {
            index += 1;
        }
        let root = &line[root_start..index];
        let mut fields = Vec::new();

        while bytes.get(index) == Some(&b'.')
            && bytes
                .get(index + 1)
                .is_some_and(|byte| is_ident_start(*byte))
        {
            index += 1;
            let field_start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            fields.push(line[field_start..index].to_owned());
        }

        if !fields.is_empty() {
            paths.push((root.to_owned(), fields));
        }
    }

    paths
}

fn interpolation_roots(line: &str) -> Vec<String> {
    let mut roots = Vec::new();
    let mut rest = line;

    while let Some(open) = rest.find("{{") {
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            break;
        };
        let expr = after_open[..close].trim();
        if let Some(root) = expr
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .find(|part| !part.is_empty())
        {
            roots.push(root.to_owned());
        }
        rest = &after_open[close + 2..];
    }

    roots
}

fn fact_read_from_when(when: &str) -> String {
    let (pattern, _) = split_when_guard(when);
    let first = pattern.split_whitespace().next().unwrap_or("<empty>");
    if first.chars().next().is_some_and(char::is_uppercase) {
        format!("schema:{first}")
    } else {
        format!("pattern:{pattern}")
    }
}

fn parse_record_start(line: &str) -> Option<(String, Option<String>)> {
    let rest = line.strip_prefix("record ").or_else(|| {
        line.strip_prefix("done ")
            .and_then(|rest| rest.split_once("->"))
            .map(|(_, record)| record.trim())
            .and_then(|record| record.strip_prefix("record "))
    })?;
    let before_brace = rest.split('{').next().unwrap_or(rest).trim();
    let mut parts = before_brace.split_whitespace();
    let schema = parts.next()?.to_owned();
    let from_binding = match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => None,
        (Some("from"), Some(binding), None) => Some(binding.to_owned()),
        _ => return None,
    };
    Some((schema, from_binding))
}

fn validate_record_field(
    rule: &RuleDecl,
    line: &str,
    record_schema: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some((field, expr)) = record_field_assignment(line) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "rule `{}` has malformed field assignment in `record {record_schema}`",
                rule.name.name
            ),
            suggestion: Some("write record fields as `field value`".to_owned()),
        });
        return;
    };

    let Some(fields) = semantic.schemas.classes.get(record_schema) else {
        return;
    };
    let Some(field_ty) = fields.get(field) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("class `{record_schema}` has no field `{field}`"),
            suggestion: Some(format!(
                "add `{field}` to `class {record_schema}` or record an existing field"
            )),
        });
        return;
    };

    if let Some((root, path)) = expression_path(expr) {
        if let Some(schema) = binding_types.get(&root) {
            if !semantic.schemas.class_exists(schema) {
                return;
            }
            if let Err(message) = semantic.schemas.resolve_field_path(schema, &path) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has invalid field path `{root}.{}`: {message}",
                        rule.name.name,
                        path.join(".")
                    ),
                    suggestion: Some(
                        "use a field declared on the bound schema or add it to the class declaration"
                            .to_owned(),
                    ),
                });
            }
        }
    }

    validate_literal_assignment(
        rule,
        record_schema,
        field,
        field_ty,
        expr,
        semantic,
        diagnostics,
    );
    validate_expected_assignment(
        rule,
        record_schema,
        field,
        field_ty,
        expr,
        semantic,
        binding_types,
        diagnostics,
    );
}

fn record_field_assignment(line: &str) -> Option<(&str, &str)> {
    let field_end = line.find(char::is_whitespace)?;
    let field = &line[..field_end];
    let expr = line[field_end..].trim();
    (!field.is_empty() && !expr.is_empty()).then_some((field, expr))
}

fn validate_record_blocks(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (schema, from_binding, body) in record_blocks(&rule.body.text) {
        for assignment in collect_field_assignments(&body) {
            let (field, value) = match assignment {
                RecordFieldAssignment::Value { field, value } => (field, value),
                RecordFieldAssignment::Shorthand { field } => {
                    let value = from_binding
                        .as_ref()
                        .map(|binding| format!("{binding}.{field}"))
                        .unwrap_or_else(|| field.clone());
                    (field, value)
                }
            };
            let line = format!("{field} {value}");
            validate_record_field(rule, &line, &schema, semantic, binding_types, diagnostics);
        }
    }
}

fn record_blocks(body: &str) -> Vec<(String, Option<String>, String)> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some((schema, from_binding)) = parse_record_start(trimmed) else {
            index += 1;
            continue;
        };
        let mut depth = brace_delta(trimmed);
        let mut record_lines = Vec::new();
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let before = depth;
            depth += brace_delta(line);
            if !(before == 1 && depth == 0 && line.trim() == "}") {
                record_lines.push(line.to_owned());
            }
            index += 1;
        }
        blocks.push((schema, from_binding, record_lines.join("\n")));
    }
    blocks
}

fn workflow_terminal_blocks(body: &str) -> Vec<(String, String, String)> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let terminal = trimmed
            .strip_prefix("complete ")
            .map(|rest| ("complete", rest))
            .or_else(|| trimmed.strip_prefix("fail ").map(|rest| ("fail", rest)));
        let Some((action, rest)) = terminal else {
            index += 1;
            continue;
        };
        let Some(name) = rest.split('{').next().and_then(|header| {
            let mut parts = header.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(name), None) => Some(name.to_owned()),
                _ => None,
            }
        }) else {
            index += 1;
            continue;
        };
        let mut depth = brace_delta(trimmed);
        let mut terminal_lines = Vec::new();
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let before = depth;
            depth += brace_delta(line);
            if !(before == 1 && depth == 0 && line.trim() == "}") {
                terminal_lines.push(line.to_owned());
            }
            index += 1;
        }
        blocks.push((action.to_owned(), name, terminal_lines.join("\n")));
    }
    blocks
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecordFieldAssignment {
    Value { field: String, value: String },
    Shorthand { field: String },
}

fn collect_field_assignments(body: &str) -> Vec<RecordFieldAssignment> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut assignments = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim().trim_end_matches(',');
        if trimmed.is_empty() || trimmed == "}" {
            index += 1;
            continue;
        }
        let Some((name, value)) = record_field_assignment(trimmed) else {
            if is_identifier(trimmed) {
                assignments.push(RecordFieldAssignment::Shorthand {
                    field: trimmed.to_owned(),
                });
            }
            index += 1;
            continue;
        };
        let mut value_lines = vec![value.to_owned()];
        let mut depth = brace_delta(value);
        index += 1;
        while depth > 0 && index < lines.len() {
            let next = lines[index].trim().trim_end_matches(',');
            depth += brace_delta(next);
            value_lines.push(next.to_owned());
            index += 1;
        }
        assignments.push(RecordFieldAssignment::Value {
            field: name.to_owned(),
            value: value_lines.join(" "),
        });
    }
    assignments
}

fn expression_path(expr: &str) -> Option<(String, Vec<String>)> {
    let mut paths = dotted_paths(expr);
    if paths.len() != 1 {
        return None;
    }
    Some(paths.remove(0))
}

fn validate_literal_assignment(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    field_ty: &TypeSyntax,
    expr: &str,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(literal) = parse_literal_expr(expr) else {
        return;
    };

    match field_ty {
        TypeSyntax::Primitive { name, .. } => {
            validate_primitive_literal(rule, record_schema, field, name, &literal, diagnostics)
        }
        TypeSyntax::LiteralString { value, .. } => {
            if literal != LiteralExpr::String(value.as_str()) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!(
                        "field `{record_schema}.{field}` expects literal string `{value}`"
                    ),
                    suggestion: Some(format!("record `{field} {value:?}`")),
                });
            }
        }
        TypeSyntax::Ref { name } => {
            validate_enum_literal(
                rule,
                record_schema,
                field,
                &name.name,
                &literal,
                semantic,
                diagnostics,
            );
        }
        TypeSyntax::Union { variants, .. } => {
            validate_union_literal(rule, record_schema, field, variants, &literal, diagnostics);
        }
        TypeSyntax::AgentRef { agents, .. } => {
            validate_agent_ref_literal(rule, record_schema, field, agents, &literal, diagnostics);
        }
        TypeSyntax::Optional { inner, .. } => {
            if literal != LiteralExpr::Null {
                validate_literal_assignment(
                    rule,
                    record_schema,
                    field,
                    inner,
                    expr,
                    semantic,
                    diagnostics,
                );
            }
        }
        TypeSyntax::Array { .. } | TypeSyntax::Map { .. } => {}
    }
}

fn validate_expected_assignment(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    field_ty: &TypeSyntax,
    expr: &str,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !(expr.trim_start().starts_with('{') || expr.trim_start().starts_with('[')) {
        return;
    }
    validate_expr_source_against_type(
        rule,
        record_schema,
        field,
        field_ty,
        expr,
        semantic,
        &ExprScope::from_bindings(binding_types),
        diagnostics,
    );
}

fn validate_expr_source_against_type(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    expected_ty: &TypeSyntax,
    expr: &str,
    semantic: &SemanticContext,
    scope: &ExprScope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expected_ty {
        TypeSyntax::Map { inner, .. } => {
            let parsed = match parse_expression(expr) {
                Ok(Expr::Object(fields)) => fields,
                Ok(_) => {
                    diagnostics.push(Diagnostic {
                        span: rule.body.span,
                        message: format!("field `{record_schema}.{field}` expects a map literal"),
                        suggestion: Some(format!("record `{field} {{ key value }}`")),
                    });
                    return;
                }
                Err(message) => {
                    diagnostics.push(Diagnostic {
                        span: rule.body.span,
                        message: format!(
                            "field `{record_schema}.{field}` expects a map literal: {message}"
                        ),
                        suggestion: Some(format!("record `{field} {{ key value }}`")),
                    });
                    return;
                }
            };
            for map_field in &parsed {
                validate_expr_against_type(
                    rule,
                    record_schema,
                    field,
                    inner,
                    &map_field.value,
                    semantic,
                    scope,
                    diagnostics,
                );
            }
        }
        TypeSyntax::Array { inner, .. } => match parse_expression(expr) {
            Ok(Expr::Array(items)) => {
                for item in items {
                    validate_expr_against_type(
                        rule,
                        record_schema,
                        field,
                        inner,
                        &item,
                        semantic,
                        scope,
                        diagnostics,
                    );
                }
            }
            Ok(expr) => validate_inferred_assignment_type(
                rule,
                record_schema,
                field,
                expected_ty,
                &expr,
                semantic,
                scope,
                diagnostics,
            ),
            Err(message) => {
                push_invalid_assignment_expr(rule, record_schema, field, message, diagnostics)
            }
        },
        TypeSyntax::Optional { inner, .. } => {
            if expr.trim() != "null" {
                validate_expr_source_against_type(
                    rule,
                    record_schema,
                    field,
                    inner,
                    expr,
                    semantic,
                    scope,
                    diagnostics,
                );
            }
        }
        TypeSyntax::Ref { name } if semantic.schemas.class_exists(&name.name) => {
            let parsed = match parse_expression(expr) {
                Ok(Expr::Object(fields)) => fields,
                Ok(expr) => {
                    validate_inferred_assignment_type(
                        rule,
                        record_schema,
                        field,
                        expected_ty,
                        &expr,
                        semantic,
                        scope,
                        diagnostics,
                    );
                    return;
                }
                Err(message) => {
                    push_invalid_assignment_expr(rule, record_schema, field, message, diagnostics);
                    return;
                }
            };
            validate_object_literal_fields(
                rule,
                record_schema,
                field,
                &name.name,
                &parsed,
                semantic,
                scope,
                diagnostics,
            );
        }
        _ => match parse_expression(expr) {
            Ok(expr) => validate_inferred_assignment_type(
                rule,
                record_schema,
                field,
                expected_ty,
                &expr,
                semantic,
                scope,
                diagnostics,
            ),
            Err(message) => {
                push_invalid_assignment_expr(rule, record_schema, field, message, diagnostics)
            }
        },
    }
}

fn validate_expr_against_type(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    expected_ty: &TypeSyntax,
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Array(items) if matches!(expected_ty, TypeSyntax::Array { .. }) => {
            if let TypeSyntax::Array { inner, .. } = expected_ty {
                for item in items {
                    validate_expr_against_type(
                        rule,
                        record_schema,
                        field,
                        inner,
                        item,
                        semantic,
                        scope,
                        diagnostics,
                    );
                }
            }
        }
        Expr::Object(fields) => match expected_ty {
            TypeSyntax::Map { inner, .. } => {
                for field in fields {
                    validate_expr_against_type(
                        rule,
                        record_schema,
                        field.key.as_str(),
                        inner,
                        &field.value,
                        semantic,
                        scope,
                        diagnostics,
                    );
                }
            }
            TypeSyntax::Ref { name } if semantic.schemas.class_exists(&name.name) => {
                validate_object_literal_fields(
                    rule,
                    record_schema,
                    field,
                    &name.name,
                    fields,
                    semantic,
                    scope,
                    diagnostics,
                );
            }
            _ => validate_inferred_assignment_type(
                rule,
                record_schema,
                field,
                expected_ty,
                expr,
                semantic,
                scope,
                diagnostics,
            ),
        },
        _ => validate_inferred_assignment_type(
            rule,
            record_schema,
            field,
            expected_ty,
            expr,
            semantic,
            scope,
            diagnostics,
        ),
    }
}

fn push_invalid_assignment_expr(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    message: String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(Diagnostic {
        span: rule.body.span,
        message: format!(
            "rule `{}` has invalid expression for field `{record_schema}.{field}`: {message}",
            rule.name.name
        ),
        suggestion: Some(
            "use array literals or expected-schema object literals for collection fields"
                .to_owned(),
        ),
    });
}

fn validate_object_literal_fields(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    object_schema: &str,
    object_fields: &[ExprObjectField],
    semantic: &SemanticContext,
    scope: &ExprScope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(schema_fields) = semantic.schemas.classes.get(object_schema) else {
        return;
    };
    let mut seen = BTreeSet::new();
    for object_field in object_fields {
        if !seen.insert(object_field.key.clone()) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "field `{record_schema}.{field}` repeats object field `{}`",
                    object_field.key
                ),
                suggestion: Some("remove the duplicate object field".to_owned()),
            });
            continue;
        }
        let Some(field_ty) = schema_fields.get(&object_field.key) else {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "class `{object_schema}` has no field `{}`",
                    object_field.key
                ),
                suggestion: Some(format!(
                    "add `{}` to `class {object_schema}` or use an existing field",
                    object_field.key
                )),
            });
            continue;
        };
        validate_expr_against_type(
            rule,
            object_schema,
            &object_field.key,
            field_ty,
            &object_field.value,
            semantic,
            scope,
            diagnostics,
        );
    }
    for (required, ty) in schema_fields {
        if seen.contains(required) || matches!(ty, TypeSyntax::Optional { .. }) {
            continue;
        }
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "field `{record_schema}.{field}` is missing required object field `{object_schema}.{required}`"
            ),
            suggestion: Some(format!("add `{required}` to the `{field}` object literal")),
        });
    }
}

fn validate_inferred_assignment_type(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    expected_ty: &TypeSyntax,
    expr: &Expr,
    semantic: &SemanticContext,
    scope: &ExprScope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let literal = expr_literal_as_literal_expr(expr);
    if let Some(literal) = literal {
        validate_literal_against_type(
            rule,
            record_schema,
            field,
            expected_ty,
            &literal,
            semantic,
            diagnostics,
        );
        return;
    }

    let context = ExprValidationContext::rule(rule);
    let mut local_diagnostics = Vec::new();
    let actual_ty = infer_expr_type(expr, semantic, scope, &context, &mut local_diagnostics);
    diagnostics.extend(local_diagnostics);
    let expected_expr_ty = expr_type_from_type_syntax(expected_ty, semantic);
    if !types_comparable(&actual_ty, &expected_expr_ty) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "field `{record_schema}.{field}` receives incompatible expression type"
            ),
            suggestion: Some(format!(
                "record a value compatible with `{}`",
                expected_ty.to_source()
            )),
        });
    }
}

fn validate_literal_against_type(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    field_ty: &TypeSyntax,
    literal: &LiteralExpr<'_>,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match field_ty {
        TypeSyntax::Primitive { name, .. } => {
            validate_primitive_literal(rule, record_schema, field, name, literal, diagnostics)
        }
        TypeSyntax::LiteralString { value, .. } => {
            if literal != &LiteralExpr::String(value.as_str()) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!(
                        "field `{record_schema}.{field}` expects literal string `{value}`"
                    ),
                    suggestion: Some(format!("record `{field} {value:?}`")),
                });
            }
        }
        TypeSyntax::Ref { name } => {
            validate_enum_literal(
                rule,
                record_schema,
                field,
                &name.name,
                literal,
                semantic,
                diagnostics,
            );
        }
        TypeSyntax::Union { variants, .. } => {
            validate_union_literal(rule, record_schema, field, variants, literal, diagnostics);
        }
        TypeSyntax::AgentRef { agents, .. } => {
            validate_agent_ref_literal(rule, record_schema, field, agents, literal, diagnostics);
        }
        TypeSyntax::Optional { inner, .. } => {
            if literal != &LiteralExpr::Null {
                validate_literal_against_type(
                    rule,
                    record_schema,
                    field,
                    inner,
                    literal,
                    semantic,
                    diagnostics,
                );
            }
        }
        TypeSyntax::Array { .. } | TypeSyntax::Map { .. } => {}
    }
}

fn expr_literal_as_literal_expr(expr: &Expr) -> Option<LiteralExpr<'_>> {
    match expr {
        Expr::Literal(ExprLiteral::String(value)) => Some(LiteralExpr::String(value)),
        Expr::Literal(ExprLiteral::Number(value)) => Some(LiteralExpr::Number(value)),
        Expr::Literal(ExprLiteral::Bool(_)) => Some(LiteralExpr::Bool),
        Expr::Literal(ExprLiteral::Null) => Some(LiteralExpr::Null),
        Expr::Literal(ExprLiteral::Ident(value)) => Some(LiteralExpr::Ident(value)),
        _ => None,
    }
}

fn validate_agent_ref_literal(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    agents: &[Ident],
    literal: &LiteralExpr<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<Vec<_>>();
    if let LiteralExpr::String(value) = literal {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!(
                "field `{record_schema}.{field}` expects an AgentRef value, not string `{value}`"
            ),
            suggestion: Some(format!(
                "use an unquoted declared agent name: {}",
                allowed.join(", ")
            )),
        });
        return;
    }
    let LiteralExpr::Ident(value) = literal else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects an AgentRef value"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` cannot reference agent `{value}`"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
    }
}

fn parse_literal_expr(expr: &str) -> Option<LiteralExpr<'_>> {
    let expr = expr.trim().trim_end_matches(',');
    if let Some(value) = expr
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        return Some(LiteralExpr::String(value));
    }
    if expr.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && expr.chars().any(|ch| ch.is_ascii_digit())
    {
        return Some(LiteralExpr::Number(expr));
    }
    match expr {
        "true" => Some(LiteralExpr::Bool),
        "false" => Some(LiteralExpr::Bool),
        "null" => Some(LiteralExpr::Null),
        value if value.chars().all(|ch| ch.is_alphanumeric() || ch == '_') => {
            Some(LiteralExpr::Ident(value))
        }
        _ => None,
    }
}

struct ExprParser<'a> {
    source: &'a str,
    tokens: Vec<ExprToken>,
    pos: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExprToken {
    kind: ExprTokenKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExprTokenKind {
    Ident(String),
    String(String),
    Number(String),
    Symbol(char),
    Op(&'static str),
}

impl<'a> ExprParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            tokens: lex_expr(source),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<Expr, String> {
        let expr = self.parse_or()?;
        if self.peek().is_some() {
            return Err(format!(
                "unexpected token in expression `{}`",
                self.source.trim()
            ));
        }
        Ok(expr)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_and()?;
        while self.consume_op("||") {
            let right = self.parse_and()?;
            expr = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_comparison()?;
        while self.consume_op("&&") {
            let right = self.parse_comparison()?;
            expr = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.consume_op("==") {
                Some(BinaryOp::Eq)
            } else if self.consume_op("!=") {
                Some(BinaryOp::Ne)
            } else if self.consume_op("<=") {
                Some(BinaryOp::Le)
            } else if self.consume_op(">=") {
                Some(BinaryOp::Ge)
            } else if self.consume_symbol('<') {
                Some(BinaryOp::Lt)
            } else if self.consume_symbol('>') {
                Some(BinaryOp::Gt)
            } else if self.consume_ident("not") {
                if !self.consume_ident("in") {
                    return Err("expected `in` after `not`".to_owned());
                }
                Some(BinaryOp::NotIn)
            } else if self.consume_ident("in") {
                Some(BinaryOp::In)
            } else {
                None
            };
            let Some(op) = op else {
                return Ok(expr);
            };
            let right = self.parse_unary()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.consume_symbol('!') {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.consume_symbol('[') {
                let key = self.parse_or()?;
                self.expect_symbol(']')?;
                expr = Expr::Index {
                    target: Box::new(expr),
                    key: Box::new(key),
                };
                continue;
            }
            return Ok(expr);
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        if self.consume_symbol('(') {
            let expr = self.parse_or()?;
            self.expect_symbol(')')?;
            return Ok(expr);
        }
        if self.consume_symbol('[') {
            let mut items = Vec::new();
            if self.consume_symbol(']') {
                return Ok(Expr::Array(items));
            }
            loop {
                items.push(self.parse_or()?);
                if self.consume_symbol(']') {
                    break;
                }
                self.expect_symbol(',')?;
            }
            return Ok(Expr::Array(items));
        }
        if self.consume_symbol('{') {
            let mut fields = Vec::new();
            if self.consume_symbol('}') {
                return Ok(Expr::Object(fields));
            }
            loop {
                let key = match self.advance().map(|token| token.kind.clone()) {
                    Some(ExprTokenKind::Ident(value) | ExprTokenKind::String(value)) => value,
                    _ => return Err("expected object field name".to_owned()),
                };
                let value = self.parse_or()?;
                fields.push(ExprObjectField { key, value });
                if self.consume_symbol('}') {
                    break;
                }
                let _ = self.consume_symbol(',');
            }
            return Ok(Expr::Object(fields));
        }
        match self.advance().map(|token| token.kind.clone()) {
            Some(ExprTokenKind::String(value)) => Ok(Expr::Literal(ExprLiteral::String(value))),
            Some(ExprTokenKind::Number(value)) => Ok(Expr::Literal(ExprLiteral::Number(value))),
            Some(ExprTokenKind::Ident(value)) if value == "true" => {
                Ok(Expr::Literal(ExprLiteral::Bool(true)))
            }
            Some(ExprTokenKind::Ident(value)) if value == "false" => {
                Ok(Expr::Literal(ExprLiteral::Bool(false)))
            }
            Some(ExprTokenKind::Ident(value)) if value == "null" => {
                Ok(Expr::Literal(ExprLiteral::Null))
            }
            Some(ExprTokenKind::Ident(value)) if value == "exists" && !self.at_symbol('(') => {
                let arg = match self.parse_postfix()? {
                    Expr::Literal(ExprLiteral::Ident(path)) => Expr::Path(vec![path]),
                    expr => expr,
                };
                Ok(Expr::Call {
                    name: value,
                    args: vec![arg],
                })
            }
            Some(ExprTokenKind::Ident(value))
                if matches!(
                    value.as_str(),
                    "count" | "exists" | "empty" | "one" | "none"
                ) && self.at_symbol('(') =>
            {
                self.expect_symbol('(')?;
                if let Some(query) = self.try_parse_query()? {
                    self.expect_symbol(')')?;
                    Ok(Expr::Call {
                        name: value,
                        args: vec![query],
                    })
                } else {
                    let mut args = Vec::new();
                    if self.consume_symbol(')') {
                        return Ok(Expr::Call { name: value, args });
                    }
                    loop {
                        args.push(self.parse_or()?);
                        if self.consume_symbol(')') {
                            break;
                        }
                        self.expect_symbol(',')?;
                    }
                    Ok(Expr::Call { name: value, args })
                }
            }
            Some(ExprTokenKind::Ident(value)) => {
                let mut path = vec![value];
                while self.consume_symbol('.') {
                    let Some(ExprTokenKind::Ident(field)) =
                        self.advance().map(|token| token.kind.clone())
                    else {
                        return Err("expected field name after `.`".to_owned());
                    };
                    path.push(field);
                }
                if path.len() == 1 {
                    Ok(Expr::Literal(ExprLiteral::Ident(path.remove(0))))
                } else {
                    Ok(Expr::Path(path))
                }
            }
            _ => Err(format!("expected expression in `{}`", self.source.trim())),
        }
    }

    fn try_parse_query(&mut self) -> Result<Option<Expr>, String> {
        let checkpoint = self.pos;
        let kind = if self.consume_ident("effect") {
            QueryKind::Effect
        } else if matches!(
            self.peek().map(|token| &token.kind),
            Some(ExprTokenKind::Ident(value)) if value.chars().next().is_some_and(char::is_uppercase)
        ) {
            QueryKind::Fact
        } else {
            return Ok(None);
        };
        let mut head = Vec::new();
        while let Some(token) = self.peek() {
            if self.at_symbol(')') || self.at_ident("where") {
                break;
            }
            head.push(self.token_text(token));
            self.pos += 1;
        }
        if head.is_empty() {
            self.pos = checkpoint;
            return Ok(None);
        }
        let guard = if self.consume_ident("where") {
            Some(Box::new(self.parse_or()?))
        } else {
            None
        };
        Ok(Some(Expr::Query {
            kind,
            head: join_query_head(&head),
            guard,
        }))
    }

    fn token_text(&self, token: &ExprToken) -> String {
        match &token.kind {
            ExprTokenKind::Ident(value) | ExprTokenKind::Number(value) => value.clone(),
            ExprTokenKind::String(value) => format!("{value:?}"),
            ExprTokenKind::Symbol(value) => value.to_string(),
            ExprTokenKind::Op(value) => value.to_string(),
        }
    }

    fn peek(&self) -> Option<&ExprToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&ExprToken> {
        let token = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(token)
    }

    fn at_symbol(&self, symbol: char) -> bool {
        matches!(
            self.peek().map(|token| &token.kind),
            Some(ExprTokenKind::Symbol(value)) if *value == symbol
        )
    }

    fn consume_symbol(&mut self, symbol: char) -> bool {
        if self.at_symbol(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_symbol(&mut self, symbol: char) -> Result<(), String> {
        if self.consume_symbol(symbol) {
            Ok(())
        } else {
            Err(format!("expected `{symbol}`"))
        }
    }

    fn at_ident(&self, ident: &str) -> bool {
        matches!(
            self.peek().map(|token| &token.kind),
            Some(ExprTokenKind::Ident(value)) if value == ident
        )
    }

    fn consume_ident(&mut self, ident: &str) -> bool {
        if self.at_ident(ident) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn consume_op(&mut self, op: &'static str) -> bool {
        if matches!(
            self.peek().map(|token| &token.kind),
            Some(ExprTokenKind::Op(value)) if *value == op
        ) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

fn join_query_head(tokens: &[String]) -> String {
    let mut head = String::new();
    for token in tokens {
        if token == "." {
            head.push('.');
        } else if head.ends_with('.') || head.is_empty() {
            head.push_str(token);
        } else {
            head.push(' ');
            head.push_str(token);
        }
    }
    head
}

fn lex_expr(source: &str) -> Vec<ExprToken> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if is_ident_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            tokens.push(ExprToken {
                kind: ExprTokenKind::Ident(source[start..index].to_owned()),
            });
            continue;
        }
        if byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
                index += 1;
            }
            tokens.push(ExprToken {
                kind: ExprTokenKind::Number(source[start..index].to_owned()),
            });
            continue;
        }
        if byte == b'"' {
            let start = index + 1;
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            let value = source[start..index.min(bytes.len())].to_owned();
            index = (index + 1).min(bytes.len());
            tokens.push(ExprToken {
                kind: ExprTokenKind::String(value),
            });
            continue;
        }
        let rest = &source[index..];
        if rest.starts_with("&&") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op("&&"),
            });
            index += 2;
        } else if rest.starts_with("||") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op("||"),
            });
            index += 2;
        } else if rest.starts_with("==") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op("=="),
            });
            index += 2;
        } else if rest.starts_with("!=") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op("!="),
            });
            index += 2;
        } else if rest.starts_with("<=") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op("<="),
            });
            index += 2;
        } else if rest.starts_with(">=") {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op(">="),
            });
            index += 2;
        } else {
            tokens.push(ExprToken {
                kind: ExprTokenKind::Symbol(byte as char),
            });
            index += 1;
        }
    }
    tokens
}

fn validate_primitive_literal(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    primitive: &str,
    literal: &LiteralExpr<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let valid = matches!(
        (primitive, literal),
        ("string", LiteralExpr::String(_))
            | ("string", LiteralExpr::Ident(_))
            | ("int", LiteralExpr::Number(_))
            | ("float", LiteralExpr::Number(_))
            | ("bool", LiteralExpr::Bool)
            | ("null", LiteralExpr::Null)
            | ("duration", LiteralExpr::String(_))
            | ("time", LiteralExpr::String(_))
    );
    if !valid {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects `{primitive}`"),
            suggestion: Some(format!("record a value compatible with `{primitive}`")),
        });
        return;
    }
    match (primitive, literal) {
        ("duration", LiteralExpr::String(value)) if parse_duration_seconds(value).is_none() => {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!("field `{record_schema}.{field}` has invalid duration literal"),
                suggestion: Some("use an ISO-8601 duration such as `\"PT30M\"`".to_owned()),
            });
        }
        ("time", LiteralExpr::String(value)) if parse_time_epoch_seconds(value).is_none() => {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!("field `{record_schema}.{field}` has invalid time literal"),
                suggestion: Some(
                    "use an RFC3339 timestamp such as `\"2026-05-29T10:00:00Z\"`".to_owned(),
                ),
            });
        }
        _ => {}
    }
}

fn validate_enum_literal(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    schema: &str,
    literal: &LiteralExpr<'_>,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(variants) = semantic.schemas.enums.get(schema) else {
        return;
    };
    let LiteralExpr::Ident(variant) = literal else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects enum `{schema}`"),
            suggestion: Some(format!(
                "use one of: {}",
                variants.iter().cloned().collect::<Vec<_>>().join(", ")
            )),
        });
        return;
    };
    if !variants.contains(*variant) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("enum `{schema}` has no variant `{variant}`"),
            suggestion: Some(format!(
                "use one of: {}",
                variants.iter().cloned().collect::<Vec<_>>().join(", ")
            )),
        });
    }
}

fn validate_union_literal(
    rule: &RuleDecl,
    record_schema: &str,
    field: &str,
    variants: &[TypeSyntax],
    literal: &LiteralExpr<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = variants
        .iter()
        .filter_map(|variant| match variant {
            TypeSyntax::LiteralString { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if allowed.is_empty() {
        return;
    }
    let LiteralExpr::String(value) = literal else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects one of its literal variants"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` cannot be `{value}`"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
    }
}

fn parse_effect_line(line: &str) -> Option<(IrEffectKind, Option<String>)> {
    let kind = if line.starts_with("tell ") {
        IrEffectKind::AgentTell
    } else if line.starts_with("coerce ") {
        IrEffectKind::BamlCoerce
    } else if line.starts_with("claim ") {
        IrEffectKind::LoftClaim
    } else if line.starts_with("askHuman") {
        IrEffectKind::HumanAsk
    } else if line.starts_with("call ") {
        IrEffectKind::CapabilityCall
    } else if line.starts_with("emit ") {
        IrEffectKind::EventEmit
    } else if line.starts_with("invoke ") {
        IrEffectKind::WorkflowInvoke
    } else {
        return None;
    };

    Some((kind, binding_after_as(line)))
}

fn parse_consume_line(line: &str) -> Option<String> {
    let binding = line
        .trim()
        .trim_end_matches(';')
        .strip_prefix("consume ")
        .or_else(|| line.trim().trim_end_matches(';').strip_prefix("done "))?
        .split("->")
        .next()
        .unwrap_or_default()
        .trim();
    let mut chars = binding.chars();
    let Some(first) = chars.next() else {
        return None;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    chars
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        .then(|| binding.to_owned())
}

fn binding_after_multiline_string_end(line: &str) -> Option<String> {
    line.strip_prefix("\"\"\"")
        .and_then(|rest| rest.trim().strip_prefix("as "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(|binding| binding.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_'))
        .filter(|binding| !binding.is_empty())
        .map(str::to_owned)
}

fn binding_after_as(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "as" {
            return tokens
                .next()
                .map(|binding| binding.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_'))
                .filter(|binding| !binding.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

fn parse_after_line(line: &str) -> Option<(String, DependencyPredicate)> {
    let rest = line.strip_prefix("after ")?;
    let before_body = rest
        .split('{')
        .next()
        .unwrap_or(rest)
        .split("=>")
        .next()
        .unwrap_or(rest)
        .trim();
    let mut parts = before_body.split_whitespace();
    let binding = parts.next()?.to_owned();
    let predicate = match parts.next()? {
        "succeeds" => DependencyPredicate::Succeeds,
        "fails" => DependencyPredicate::Fails,
        "completes" => DependencyPredicate::Completes,
        _ => return None,
    };
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => {}
        (Some("as"), Some(alias), None) if is_identifier(alias) => {}
        _ => return None,
    }
    Some((binding, predicate))
}

fn desugar_then_chains(body: &str) -> String {
    let lines = body.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut last_effect_binding: Option<String> = None;
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let (Some(statement), Some(upstream)) = (
            trimmed.strip_prefix("then ").map(str::trim),
            last_effect_binding.as_deref(),
        ) {
            output.push(format!("after {upstream} succeeds {{"));
            output.push(format!("  {statement}"));
            let mut depth = brace_delta(statement);
            index += 1;
            while depth > 0 && index < lines.len() {
                let line = lines[index];
                depth += brace_delta(line);
                output.push(format!("  {line}"));
                index += 1;
            }
            output.push("}".to_owned());
            if let Some(binding) = effect_binding_from_statement(statement) {
                last_effect_binding = Some(binding);
            }
            continue;
        }
        output.push(lines[index].to_owned());
        if let Some(binding) = effect_binding_from_statement(trimmed) {
            last_effect_binding = Some(binding);
        }
        index += 1;
    }
    output.join("\n")
}

fn effect_binding_from_statement(statement: &str) -> Option<String> {
    if parse_effect_line(statement).is_some() {
        binding_after_as(statement)
    } else {
        None
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn lower_type(ty: TypeSyntax) -> IrType {
    match ty {
        TypeSyntax::Primitive { name, .. } => IrType::Primitive(lower_primitive_type(&name)),
        TypeSyntax::LiteralString { value, .. } => IrType::LiteralString(value),
        TypeSyntax::Ref { name } => IrType::Ref(name.name),
        TypeSyntax::AgentRef { agents, .. } => {
            IrType::AgentRef(agents.into_iter().map(|agent| agent.name).collect())
        }
        TypeSyntax::Optional { inner, .. } => IrType::Optional(Box::new(lower_type(*inner))),
        TypeSyntax::Array { inner, .. } => IrType::Array(Box::new(lower_type(*inner))),
        TypeSyntax::Map { inner, .. } => IrType::Map(Box::new(lower_type(*inner))),
        TypeSyntax::Union { variants, .. } => {
            IrType::Union(variants.into_iter().map(lower_type).collect())
        }
    }
}

fn lower_primitive_type(name: &str) -> IrPrimitiveType {
    match name {
        "string" => IrPrimitiveType::String,
        "int" => IrPrimitiveType::Int,
        "float" => IrPrimitiveType::Float,
        "bool" => IrPrimitiveType::Bool,
        "null" => IrPrimitiveType::Null,
        "duration" => IrPrimitiveType::Duration,
        "time" => IrPrimitiveType::Time,
        "image" => IrPrimitiveType::Image,
        "audio" => IrPrimitiveType::Audio,
        "pdf" => IrPrimitiveType::Pdf,
        "video" => IrPrimitiveType::Video,
        _ => IrPrimitiveType::String,
    }
}

fn push_line(snapshot: &mut String, line: impl AsRef<str>) {
    snapshot.push_str(line.as_ref());
    snapshot.push('\n');
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn parse_duration_seconds(value: &str) -> Option<f64> {
    let value = value.strip_prefix('P')?;
    let mut rest = value;
    let mut seconds = 0.0;
    let mut consumed = false;
    let mut in_time = false;

    while !rest.is_empty() {
        if let Some(next) = rest.strip_prefix('T') {
            if in_time {
                return None;
            }
            in_time = true;
            rest = next;
            continue;
        }

        let number_len = rest
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_digit() || *ch == '.')
            .map(|(index, ch)| index + ch.len_utf8())
            .last()?;
        let number = rest[..number_len].parse::<f64>().ok()?;
        if !number.is_finite() {
            return None;
        }
        let unit = rest[number_len..].chars().next()?;
        rest = &rest[number_len + unit.len_utf8()..];
        let multiplier = match (in_time, unit) {
            (false, 'D') => 86_400.0,
            (true, 'H') => 3_600.0,
            (true, 'M') => 60.0,
            (true, 'S') => 1.0,
            _ => return None,
        };
        seconds += number * multiplier;
        consumed = true;
    }

    consumed.then_some(seconds)
}

pub fn parse_time_epoch_seconds(value: &str) -> Option<f64> {
    if value.len() < 20 {
        return None;
    }
    let year = parse_fixed_i32(value, 0, 4)?;
    require_byte(value, 4, b'-')?;
    let month = parse_fixed_u32(value, 5, 2)?;
    require_byte(value, 7, b'-')?;
    let day = parse_fixed_u32(value, 8, 2)?;
    require_byte(value, 10, b'T')?;
    let hour = parse_fixed_u32(value, 11, 2)?;
    require_byte(value, 13, b':')?;
    let minute = parse_fixed_u32(value, 14, 2)?;
    require_byte(value, 16, b':')?;
    let second = parse_fixed_u32(value, 17, 2)?;
    let mut offset_start = 19;
    let mut fractional_second = 0.0;
    if value.as_bytes().get(offset_start).copied() == Some(b'.') {
        let fraction_start = offset_start + 1;
        let fraction_len = value[fraction_start..]
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_digit())
            .map(|(index, ch)| index + ch.len_utf8())
            .last()?;
        let fraction = &value[fraction_start..fraction_start + fraction_len];
        let scale = 10_f64.powi(i32::try_from(fraction.len()).ok()?);
        fractional_second = fraction.parse::<f64>().ok()? / scale;
        offset_start = fraction_start + fraction_len;
    }
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let offset_seconds = match value.as_bytes().get(offset_start).copied()? {
        b'Z' if value.len() == offset_start + 1 => 0,
        b'+' | b'-' if value.len() == offset_start + 6 => {
            let sign = if value.as_bytes()[offset_start] == b'+' {
                1
            } else {
                -1
            };
            let offset_hour = parse_fixed_i32(value, offset_start + 1, 2)?;
            require_byte(value, offset_start + 3, b':')?;
            let offset_minute = parse_fixed_i32(value, offset_start + 4, 2)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            sign * (offset_hour * 3_600 + offset_minute * 60)
        }
        _ => return None,
    };

    let days = days_from_civil(year, month, day);
    let local_seconds = days * 86_400 + i64::from(hour * 3_600 + minute * 60 + second.min(59));
    Some((local_seconds - i64::from(offset_seconds)) as f64 + fractional_second)
}

fn parse_fixed_i32(value: &str, start: usize, len: usize) -> Option<i32> {
    value.get(start..start + len)?.parse::<i32>().ok()
}

fn parse_fixed_u32(value: &str, start: usize, len: usize) -> Option<u32> {
    value.get(start..start + len)?.parse::<u32>().ok()
}

fn require_byte(value: &str, index: usize, expected: u8) -> Option<()> {
    (value.as_bytes().get(index).copied()? == expected).then_some(())
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era * 146_097 + day_of_era - 719_468)
}

fn format_syntax(program: Program) -> String {
    let mut formatted = String::new();
    if let Some(workflow) = program.workflow {
        push_line(&mut formatted, format!("workflow {}", workflow.name));
        formatted.push('\n');
    }

    let mut top_level_items = Vec::new();
    top_level_items.extend(program.patterns.into_iter().map(Item::Pattern));
    top_level_items.extend(program.items);
    format_items(top_level_items, &mut formatted);

    if !formatted.is_empty() && !program.workflows.is_empty() {
        formatted.push('\n');
    }
    let workflow_count = program.workflows.len();
    for (index, workflow) in program.workflows.into_iter().enumerate() {
        format_workflow(workflow, &mut formatted);
        if index + 1 < workflow_count {
            formatted.push('\n');
        }
    }

    formatted
}

fn format_items(items: Vec<Item>, formatted: &mut String) {
    let item_count = items.len();
    for (index, item) in items.into_iter().enumerate() {
        format_item(item, formatted);
        if index + 1 < item_count {
            formatted.push('\n');
        }
    }
}

fn format_item(item: Item, formatted: &mut String) {
    match item {
        Item::Include(include) => {
            push_line(formatted, format!("include {:?}", include.path.value));
        }
        Item::Use(use_decl) => {
            push_line(formatted, format!("use {}", use_decl.name.value));
        }
        Item::Pattern(pattern) => format_pattern(pattern, formatted),
        Item::Apply(apply) => format_apply(apply, formatted),
        Item::WorkflowContract(contract) => {
            push_line(
                formatted,
                format!(
                    "{} {} {}",
                    contract.kind.as_str(),
                    contract.name.name,
                    contract.ty.to_source()
                ),
            );
        }
        Item::Agent(agent) => format_agent(agent, formatted),
        Item::Enum(enum_decl) => format_enum(enum_decl, formatted),
        Item::Class(class_decl) => format_class(class_decl, formatted),
        Item::Coerce(coerce) => format_coerce(coerce, formatted),
        Item::Assert(assertion) => push_line(formatted, format!("assert {}", assertion.expr)),
        Item::Rule(rule) => format_rule(rule, formatted),
    }
}

fn format_pattern(pattern: PatternDecl, formatted: &mut String) {
    let params = if pattern.type_params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            pattern
                .type_params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    push_line(
        formatted,
        format!("pattern {}{} {{", pattern.name.name, params),
    );
    let mut inner = String::new();
    format_items(pattern.items, &mut inner);
    for line in inner.lines() {
        if line.is_empty() {
            formatted.push('\n');
        } else {
            push_line(formatted, format!("  {line}"));
        }
    }
    push_line(formatted, "}");
}

fn format_apply(apply: ApplyDecl, formatted: &mut String) {
    let args = if apply.type_args.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            apply
                .type_args
                .iter()
                .map(TypeSyntax::to_source)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    push_line(
        formatted,
        format!(
            "apply {}{} as {} {{",
            apply.pattern.name, args, apply.alias.name
        ),
    );
    push_block_body(&apply.body.text, formatted);
    push_line(formatted, "}");
}

fn format_workflow(workflow: WorkflowDecl, formatted: &mut String) {
    push_line(formatted, format!("workflow {} {{", workflow.name.name));
    let mut inner = String::new();
    format_items(workflow.items, &mut inner);
    for line in inner.lines() {
        if line.is_empty() {
            formatted.push('\n');
        } else {
            push_line(formatted, format!("  {line}"));
        }
    }
    push_line(formatted, "}");
}

fn format_agent(agent: AgentDecl, formatted: &mut String) {
    push_line(formatted, format!("agent {} {{", agent.name.name));
    for field in agent.fields {
        match field {
            AgentField::Profile(profile) => {
                push_line(formatted, format!("  profile {:?}", profile.value));
            }
            AgentField::Capacity(capacity, _) => {
                push_line(formatted, format!("  capacity {capacity}"));
            }
            AgentField::Skills(skills, _) => {
                let skills = skills
                    .into_iter()
                    .map(|skill| format!("{:?}", skill.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                push_line(formatted, format!("  skills [{skills}]"));
            }
            AgentField::Capabilities(capabilities, _) => {
                let capabilities = capabilities
                    .into_iter()
                    .map(|capability| format!("{:?}", capability.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                push_line(formatted, format!("  capabilities [{capabilities}]"));
            }
            AgentField::Unknown { name, .. } => {
                push_line(formatted, format!("  {}", name.name));
            }
        }
    }
    push_line(formatted, "}");
}

fn format_enum(enum_decl: EnumDecl, formatted: &mut String) {
    push_line(formatted, format!("enum {} {{", enum_decl.name.name));
    for variant in enum_decl.variants {
        push_line(formatted, format!("  {}", variant.name));
    }
    push_line(formatted, "}");
}

fn format_class(class_decl: ClassDecl, formatted: &mut String) {
    push_line(formatted, format!("class {} {{", class_decl.name.name));
    for field in class_decl.fields {
        push_line(
            formatted,
            format!("  {} {}", field.name.name, field.ty.to_source()),
        );
    }
    push_line(formatted, "}");
}

fn format_coerce(coerce: CoerceDecl, formatted: &mut String) {
    let params = coerce
        .params
        .into_iter()
        .map(|param| format!("{} {}", param.name.name, param.ty.to_source()))
        .collect::<Vec<_>>()
        .join(", ");
    push_line(
        formatted,
        format!(
            "coerce {}({}) -> {} {{",
            coerce.name.name,
            params,
            coerce.output.to_source()
        ),
    );
    push_block_body(&coerce.body.text, formatted);
    push_line(formatted, "}");
}

fn format_rule(rule: RuleDecl, formatted: &mut String) {
    push_line(formatted, format!("rule {}", rule.name.name));
    for when in rule.whens {
        push_line(formatted, format!("  when {}", when.text));
    }
    push_line(formatted, "=> {");
    push_block_body(&rule.body.text, formatted);
    push_line(formatted, "}");
}

fn push_block_body(body: &str, formatted: &mut String) {
    if body.is_empty() {
        return;
    }

    for line in body.lines() {
        if line.trim().is_empty() {
            formatted.push('\n');
        } else {
            push_line(formatted, format!("  {}", line.trim_end()));
        }
    }
}

impl TypeSyntax {
    fn to_source(&self) -> String {
        match self {
            Self::Primitive { name, .. } => name.clone(),
            Self::LiteralString { value, .. } => format!("{value:?}"),
            Self::Ref { name } => name.name.clone(),
            Self::AgentRef { agents, .. } => {
                let agents = agents
                    .iter()
                    .map(|agent| agent.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("AgentRef<{agents}>")
            }
            Self::Optional { inner, .. } => format!("{}?", inner.to_source()),
            Self::Array { inner, .. } => format!("{}[]", inner.to_source()),
            Self::Map { inner, .. } => format!("map<{}>", inner.to_source()),
            Self::Union { variants, .. } => variants
                .iter()
                .map(Self::to_source)
                .collect::<Vec<_>>()
                .join(" | "),
        }
    }
}

/// Stage marker retained for the CLI scaffold.
pub fn parser_stage() -> &'static str {
    whipplescript_core::IMPLEMENTATION_STAGE
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Lexed {
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenKind {
    Ident(String),
    String(String),
    Number(String),
    Arrow,
    ThinArrow,
    Symbol(char),
}

impl TokenKind {
    fn label(&self) -> String {
        match self {
            Self::Ident(value) => format!("identifier `{value}`"),
            Self::String(_) => "string literal".to_owned(),
            Self::Number(_) => "number literal".to_owned(),
            Self::Arrow => "`=>`".to_owned(),
            Self::ThinArrow => "`->`".to_owned(),
            Self::Symbol(value) => format!("`{value}`"),
        }
    }
}

fn lex(source: &str) -> Lexed {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if byte == b'#' {
            index = skip_line(bytes, index + 1);
            continue;
        }

        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line(bytes, index + 2);
            continue;
        }

        if is_ident_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Ident(source[start..index].to_owned()),
                span: SourceSpan { start, end: index },
            });
            continue;
        }

        if byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Number(source[start..index].to_owned()),
                span: SourceSpan { start, end: index },
            });
            continue;
        }

        if byte == b'"' {
            let (token, next, diagnostic) = lex_string(source, index);
            tokens.push(token);
            if let Some(diagnostic) = diagnostic {
                diagnostics.push(diagnostic);
            }
            index = next;
            continue;
        }

        if byte == b'=' && bytes.get(index + 1) == Some(&b'>') {
            tokens.push(Token {
                kind: TokenKind::Arrow,
                span: SourceSpan {
                    start: index,
                    end: index + 2,
                },
            });
            index += 2;
            continue;
        }

        if byte == b'=' && bytes.get(index + 1) == Some(&b'=') {
            index += 2;
            continue;
        }

        if byte == b'!' && bytes.get(index + 1) == Some(&b'=') {
            index += 2;
            continue;
        }

        if matches!(byte, b'<' | b'>') && bytes.get(index + 1) == Some(&b'=') {
            index += 2;
            continue;
        }

        if matches!(byte, b'&' | b'|') && bytes.get(index + 1) == Some(&byte) {
            index += 2;
            continue;
        }

        if byte == b'-' && bytes.get(index + 1) == Some(&b'>') {
            tokens.push(Token {
                kind: TokenKind::ThinArrow,
                span: SourceSpan {
                    start: index,
                    end: index + 2,
                },
            });
            index += 2;
            continue;
        }

        if b"{}[]()<>,?|.+!:".contains(&byte) {
            tokens.push(Token {
                kind: TokenKind::Symbol(byte as char),
                span: SourceSpan {
                    start: index,
                    end: index + 1,
                },
            });
            index += 1;
            continue;
        }

        diagnostics.push(Diagnostic {
            span: SourceSpan {
                start: index,
                end: index + 1,
            },
            message: format!("unexpected character `{}`", byte as char),
            suggestion: None,
        });
        index += 1;
    }

    Lexed {
        tokens,
        diagnostics,
    }
}

fn skip_line(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit() || byte == b'-'
}

fn lex_string(source: &str, start: usize) -> (Token, usize, Option<Diagnostic>) {
    let bytes = source.as_bytes();
    let triple = bytes.get(start..start + 3) == Some(b"\"\"\"");
    let content_start = if triple { start + 3 } else { start + 1 };
    let mut index = content_start;

    while index < bytes.len() {
        if triple && bytes.get(index..index + 3) == Some(b"\"\"\"") {
            let end = index + 3;
            return (
                Token {
                    kind: TokenKind::String(source[content_start..index].to_owned()),
                    span: SourceSpan { start, end },
                },
                end,
                None,
            );
        }

        if !triple && bytes[index] == b'"' {
            let end = index + 1;
            return (
                Token {
                    kind: TokenKind::String(source[content_start..index].to_owned()),
                    span: SourceSpan { start, end },
                },
                end,
                None,
            );
        }

        if !triple && bytes[index] == b'\\' && index + 1 < bytes.len() {
            index += 2;
        } else {
            index += 1;
        }
    }

    (
        Token {
            kind: TokenKind::String(source[content_start..].to_owned()),
            span: SourceSpan {
                start,
                end: source.len(),
            },
        },
        source.len(),
        Some(Diagnostic {
            span: SourceSpan {
                start,
                end: source.len(),
            },
            message: "unterminated string literal".to_owned(),
            suggestion: Some("close the string literal".to_owned()),
        }),
    )
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

struct ParsedWorkflow {
    decl: WorkflowDecl,
    explicit_body: bool,
}

impl Parser<'_> {
    fn parse_program(&mut self) -> Program {
        let mut workflow = None;
        let mut explicit_workflow_body = false;
        let mut workflows = Vec::new();
        let mut patterns = Vec::new();
        let mut items = Vec::new();

        while !self.is_at_end() {
            if self.at_ident("workflow") {
                if let Some(parsed_workflow) = self.parse_workflow() {
                    if parsed_workflow.explicit_body {
                        workflows.push(parsed_workflow.decl);
                    } else {
                        if workflow.is_some() {
                            self.diagnostics.push(Diagnostic {
                                span: parsed_workflow.decl.name.span,
                                message: "multiple legacy workflow headers are not supported"
                                    .to_owned(),
                                suggestion: Some(
                                    "use explicit `workflow Name { ... }` declarations with `--root`"
                                        .to_owned(),
                                ),
                            });
                        }
                        workflow = Some(parsed_workflow.decl.name);
                        explicit_workflow_body = false;
                    }
                }
            } else if self.at_ident("pattern") {
                if let Some(pattern) = self.parse_pattern() {
                    patterns.push(pattern);
                }
            } else if let Some(item) = self.parse_declaration_item() {
                items.push(item);
            } else {
                if self.is_at_end() {
                    break;
                }
                self.unexpected("top-level declaration");
                if !self.is_at_end() {
                    self.advance();
                }
            }
        }

        Program {
            workflow,
            explicit_workflow_body,
            workflows,
            patterns,
            items,
        }
    }

    fn parse_workflow(&mut self) -> Option<ParsedWorkflow> {
        let start = self.expect_keyword("workflow")?.span.start;
        let name = self.expect_ident("workflow name")?;
        let mut explicit_body = false;
        let mut items = Vec::new();
        let mut end = name.span.end;
        if self.at_symbol('{') {
            explicit_body = true;
            self.expect_symbol('{')?;
            while !self.is_at_end() && !self.at_symbol('}') {
                if self.at_ident("workflow") || self.at_ident("pattern") {
                    self.unexpected("workflow body declaration");
                    self.advance();
                    continue;
                }
                if let Some(item) = self.parse_declaration_item() {
                    items.push(item);
                } else {
                    if self.is_at_end() {
                        break;
                    }
                    self.unexpected("workflow body declaration");
                    if !self.is_at_end() {
                        self.advance();
                    }
                }
            }
            if let Some(close) = self.expect_symbol('}') {
                end = close.span.end;
            }
        }
        Some(ParsedWorkflow {
            decl: WorkflowDecl {
                name,
                items,
                span: SourceSpan { start, end },
            },
            explicit_body,
        })
    }

    fn parse_declaration_item(&mut self) -> Option<Item> {
        if self.at_ident("include") {
            self.parse_include().map(Item::Include)
        } else if self.at_ident("use") {
            self.parse_use().map(Item::Use)
        } else if self.at_ident("pattern") {
            self.parse_pattern().map(Item::Pattern)
        } else if self.at_ident("apply") {
            self.parse_apply().map(Item::Apply)
        } else if self.at_ident("input") || self.at_ident("output") || self.at_ident("failure") {
            self.parse_workflow_contract().map(Item::WorkflowContract)
        } else if self.at_ident("agent") {
            self.parse_agent().map(Item::Agent)
        } else if self.at_ident("enum") {
            self.parse_enum().map(Item::Enum)
        } else if self.at_ident("class") {
            self.parse_class().map(Item::Class)
        } else if self.at_ident("coerce") {
            self.parse_coerce().map(Item::Coerce)
        } else if self.at_ident("assert") {
            self.parse_assert().map(Item::Assert)
        } else if self.at_ident("rule") {
            self.parse_rule().map(Item::Rule)
        } else {
            None
        }
    }

    fn parse_pattern(&mut self) -> Option<PatternDecl> {
        let start = self.expect_keyword("pattern")?.span.start;
        let name = self.expect_ident("pattern name")?;
        let type_params = self.parse_type_param_list().unwrap_or_default();
        let open = self.expect_symbol('{')?;
        let mut items = Vec::new();
        while !self.is_at_end() && !self.at_symbol('}') {
            if self.at_ident("workflow") || self.at_ident("pattern") {
                self.unexpected("pattern body declaration");
                self.advance();
                continue;
            }
            if let Some(item) = self.parse_declaration_item() {
                items.push(item);
            } else {
                if self.is_at_end() {
                    break;
                }
                self.unexpected("pattern body declaration");
                self.advance();
            }
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some(PatternDecl {
            name,
            type_params,
            items,
            span: SourceSpan { start, end },
        })
    }

    fn parse_type_param_list(&mut self) -> Option<Vec<Ident>> {
        if !self.at_symbol('<') {
            return Some(Vec::new());
        }
        self.expect_symbol('<')?;
        let mut params = Vec::new();
        while !self.is_at_end() && !self.at_symbol('>') {
            params.push(self.expect_ident("type parameter")?);
            if self.at_symbol(',') {
                self.advance();
            } else if !self.at_symbol('>') {
                self.unexpected("`,` or `>`");
                while !self.is_at_end() && !self.at_symbol('>') && !self.at_symbol(',') {
                    self.advance();
                }
            }
        }
        self.expect_symbol('>')?;
        Some(params)
    }

    fn parse_type_arg_list(&mut self) -> Option<Vec<TypeSyntax>> {
        if !self.at_symbol('<') {
            return Some(Vec::new());
        }
        self.expect_symbol('<')?;
        let mut args = Vec::new();
        while !self.is_at_end() && !self.at_symbol('>') {
            args.push(self.parse_type()?);
            if self.at_symbol(',') {
                self.advance();
            } else if !self.at_symbol('>') {
                self.unexpected("`,` or `>`");
                while !self.is_at_end() && !self.at_symbol('>') && !self.at_symbol(',') {
                    self.advance();
                }
            }
        }
        self.expect_symbol('>')?;
        Some(args)
    }

    fn parse_apply(&mut self) -> Option<ApplyDecl> {
        let start = self.expect_keyword("apply")?.span.start;
        let pattern = self.expect_ident("pattern name")?;
        let type_args = self.parse_type_arg_list().unwrap_or_default();
        self.expect_keyword("as")?;
        let alias = self.expect_ident("pattern application alias")?;
        let body = self.parse_block_source()?;
        let span = SourceSpan {
            start,
            end: body.span.end,
        };
        Some(ApplyDecl {
            pattern,
            type_args,
            alias,
            body,
            span,
        })
    }

    fn parse_include(&mut self) -> Option<IncludeDecl> {
        self.expect_keyword("include")?;
        Some(IncludeDecl {
            path: self.expect_string("include path")?,
        })
    }

    fn parse_workflow_contract(&mut self) -> Option<WorkflowContractDecl> {
        let keyword = self.advance().clone();
        let kind = match &keyword.kind {
            TokenKind::Ident(value) if value == "input" => WorkflowContractKind::Input,
            TokenKind::Ident(value) if value == "output" => WorkflowContractKind::Output,
            TokenKind::Ident(value) if value == "failure" => WorkflowContractKind::Failure,
            _ => return None,
        };
        let name = self.expect_ident("workflow contract name")?;
        let ty = self.parse_type()?;
        let span = keyword.span.join(ty.span());
        Some(WorkflowContractDecl {
            kind,
            name,
            ty,
            span,
        })
    }

    fn parse_use(&mut self) -> Option<UseDecl> {
        self.expect_keyword("use")?;
        if self.at_ident("plugin") || self.at_ident("skill") {
            let legacy_kind = self.advance().clone();
            let legacy_label = match &legacy_kind.kind {
                TokenKind::Ident(value) => value.as_str(),
                _ => "",
            };
            self.diagnostics.push(Diagnostic {
                span: legacy_kind.span,
                message: format!("`use {legacy_label}` is no longer supported"),
                suggestion: Some(
                    "write `use memory` for plugins; attach skills with `agent { skills [...] }`"
                        .to_owned(),
                ),
            });
        }
        Some(UseDecl {
            name: self.expect_use_name("plugin name")?,
        })
    }

    fn parse_agent(&mut self) -> Option<AgentDecl> {
        let start = self.expect_keyword("agent")?.span.start;
        let name = self.expect_ident("agent name")?;
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();

        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field_name) = self.expect_ident("agent field") else {
                self.synchronize_to_block_item();
                continue;
            };

            match field_name.name.as_str() {
                "profile" => {
                    if let Some(value) = self.expect_string("profile string") {
                        fields.push(AgentField::Profile(value));
                    } else {
                        self.synchronize_to_block_item();
                    }
                }
                "capacity" => {
                    if let Some((value, span)) = self.expect_u32("capacity value") {
                        fields.push(AgentField::Capacity(value, span));
                    } else {
                        self.synchronize_to_block_item();
                    }
                }
                "skills" => {
                    if let Some((skills, span)) = self.parse_string_list() {
                        fields.push(AgentField::Skills(skills, span));
                    } else {
                        self.synchronize_to_block_item();
                    }
                }
                "capabilities" => {
                    if let Some((capabilities, span)) = self.parse_string_list() {
                        fields.push(AgentField::Capabilities(capabilities, span));
                    } else {
                        self.synchronize_to_block_item();
                    }
                }
                _ => {
                    let span = field_name.span;
                    fields.push(AgentField::Unknown {
                        name: field_name,
                        span,
                    });
                    self.synchronize_to_block_item();
                }
            }
        }

        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);

        Some(AgentDecl {
            name,
            fields,
            span: SourceSpan { start, end },
        })
    }

    fn parse_enum(&mut self) -> Option<EnumDecl> {
        let start = self.expect_keyword("enum")?.span.start;
        let name = self.expect_ident("enum name")?;
        let open = self.expect_symbol('{')?;
        let mut variants = Vec::new();

        while !self.is_at_end() && !self.at_symbol('}') {
            if let Some(variant) = self.expect_ident("enum variant") {
                variants.push(variant);
            } else {
                self.synchronize_to_block_item();
            }
        }

        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);

        Some(EnumDecl {
            name,
            variants,
            span: SourceSpan { start, end },
        })
    }

    fn parse_class(&mut self) -> Option<ClassDecl> {
        let start = self.expect_keyword("class")?.span.start;
        let name = self.expect_ident("class name")?;
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();

        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field_name) = self.expect_ident("class field name") else {
                self.synchronize_to_block_item();
                continue;
            };
            let Some(ty) = self.parse_type() else {
                self.synchronize_to_block_item();
                continue;
            };
            fields.push(ClassField {
                span: field_name.span.join(ty.span()),
                name: field_name,
                ty,
            });
        }

        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);

        Some(ClassDecl {
            name,
            fields,
            span: SourceSpan { start, end },
        })
    }

    fn parse_coerce(&mut self) -> Option<CoerceDecl> {
        let start = self.expect_keyword("coerce")?.span.start;
        let name = self.expect_ident("coerce name")?;
        let params = self.parse_param_list()?;
        self.expect_thin_arrow()?;
        let output = self.parse_type()?;
        let body = self.parse_block_source()?;
        let span = SourceSpan {
            start,
            end: body.span.end,
        };
        Some(CoerceDecl {
            name,
            params,
            output,
            body,
            span,
        })
    }

    fn parse_param_list(&mut self) -> Option<Vec<ParamDecl>> {
        self.expect_symbol('(')?;
        let mut params = Vec::new();

        while !self.is_at_end() && !self.at_symbol(')') {
            let name = self.expect_ident("parameter name")?;
            let ty = self.parse_type()?;
            params.push(ParamDecl {
                span: name.span.join(ty.span()),
                name,
                ty,
            });

            if self.at_symbol(',') {
                self.advance();
            } else if !self.at_symbol(')') {
                self.unexpected("`,` or `)`");
                while !self.is_at_end() && !self.at_symbol(')') && !self.at_symbol(',') {
                    self.advance();
                }
            }
        }

        self.expect_symbol(')')?;
        Some(params)
    }

    fn parse_rule(&mut self) -> Option<RuleDecl> {
        let start = self.expect_keyword("rule")?.span.start;
        let name = self.expect_ident("rule name")?;
        let mut whens = Vec::new();

        while !self.is_at_end() && !self.at_arrow() {
            if self.at_ident("when") {
                whens.push(self.parse_when_clause()?);
            } else {
                self.unexpected("`when` clause or `=>`");
                self.advance();
            }
        }

        self.expect_arrow()?;
        let body = self.parse_block_source()?;
        let span = SourceSpan {
            start,
            end: body.span.end,
        };
        Some(RuleDecl {
            name,
            whens,
            body,
            span,
        })
    }

    fn parse_assert(&mut self) -> Option<AssertDecl> {
        let assert = self.expect_keyword("assert")?;
        let expr_start = assert.span.end;
        let line_end = self.source[expr_start..]
            .find('\n')
            .map(|offset| expr_start + offset)
            .unwrap_or(self.source.len());
        let mut expr_end = line_end;

        while !self.is_at_end() && self.peek()?.span.start < line_end {
            expr_end = self.peek()?.span.end.min(line_end);
            self.advance();
        }

        let span = SourceSpan {
            start: expr_start,
            end: expr_end,
        };
        let (expr, span) = trimmed_source_text(self.source_text(span), span);
        Some(AssertDecl { expr, span })
    }

    fn parse_when_clause(&mut self) -> Option<WhenClause> {
        let when = self.expect_keyword("when")?;
        let text_start = when.span.end;
        let mut text_end = text_start;

        while !self.is_at_end()
            && !self.at_arrow()
            && !self.at_ident("when")
            && !self.at_ident("rule")
        {
            text_end = self.peek()?.span.end;
            self.advance();
        }

        let span = SourceSpan {
            start: text_start,
            end: text_end,
        };
        let (text, span) = trimmed_source_text(self.source_text(span), span);
        Some(WhenClause { text, span })
    }

    fn parse_block_source(&mut self) -> Option<BlockSource> {
        let open = self.expect_symbol('{')?;
        let body_start = open.span.end;
        let mut depth = 1usize;
        let mut body_end = body_start;

        while !self.is_at_end() {
            let token = self.advance().clone();
            match token.kind {
                TokenKind::Symbol('{') => {
                    depth += 1;
                    body_end = token.span.end;
                }
                TokenKind::Symbol('}') => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = token.span.start;
                        return Some(BlockSource {
                            text: self
                                .source_text(SourceSpan {
                                    start: body_start,
                                    end: body_end,
                                })
                                .trim()
                                .to_owned(),
                            span: SourceSpan {
                                start: open.span.start,
                                end: token.span.end,
                            },
                        });
                    }
                    body_end = token.span.end;
                }
                _ => {
                    body_end = token.span.end;
                }
            }
        }

        self.diagnostics.push(Diagnostic {
            span: SourceSpan {
                start: open.span.start,
                end: body_end,
            },
            message: "unterminated block".to_owned(),
            suggestion: Some("add a closing `}`".to_owned()),
        });
        Some(BlockSource {
            text: self
                .source_text(SourceSpan {
                    start: body_start,
                    end: body_end,
                })
                .trim()
                .to_owned(),
            span: SourceSpan {
                start: open.span.start,
                end: body_end,
            },
        })
    }

    fn parse_type(&mut self) -> Option<TypeSyntax> {
        let first = self.parse_type_atom()?;
        let first = self.parse_type_suffixes(first);

        if !self.at_symbol('|') {
            return Some(first);
        }

        let start = first.span().start;
        let mut end = first.span().end;
        let mut variants = vec![first];

        while self.at_symbol('|') {
            self.advance();
            let variant = self.parse_type_atom()?;
            let variant = self.parse_type_suffixes(variant);
            end = variant.span().end;
            variants.push(variant);
        }

        Some(TypeSyntax::Union {
            variants,
            span: SourceSpan { start, end },
        })
    }

    fn parse_type_atom(&mut self) -> Option<TypeSyntax> {
        Some(if self.at_ident("AgentRef") {
            let agent_ref = self.advance().clone();
            self.expect_symbol('<')?;
            let mut agents = Vec::new();
            while !self.is_at_end() && !self.at_symbol('>') {
                if self.at_symbol('|') {
                    self.advance();
                    continue;
                }
                let Some(agent) = self.expect_ident("agent reference") else {
                    break;
                };
                agents.push(agent);
            }
            let close = self.expect_symbol('>')?;
            TypeSyntax::AgentRef {
                agents,
                span: agent_ref.span.join(close.span),
            }
        } else if self.at_ident("map") {
            let map = self.advance().clone();
            self.expect_symbol('<')?;
            let inner = self.parse_type()?;
            let close = self.expect_symbol('>')?;
            TypeSyntax::Map {
                span: map.span.join(close.span),
                inner: Box::new(inner),
            }
        } else if matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::String(_))
        ) {
            let literal = self.expect_string("literal type")?;
            TypeSyntax::LiteralString {
                value: literal.value,
                span: literal.span,
            }
        } else {
            let ident = self.expect_ident("type name")?;
            if is_primitive_type(&ident.name) {
                TypeSyntax::Primitive {
                    name: ident.name,
                    span: ident.span,
                }
            } else {
                TypeSyntax::Ref { name: ident }
            }
        })
    }

    fn parse_type_suffixes(&mut self, mut ty: TypeSyntax) -> TypeSyntax {
        loop {
            if self.at_symbol('?') {
                let question = self.advance().clone();
                ty = TypeSyntax::Optional {
                    span: ty.span().join(question.span),
                    inner: Box::new(ty),
                };
            } else if self.at_symbol('[') {
                self.advance();
                let Some(close) = self.expect_symbol(']') else {
                    return ty;
                };
                ty = TypeSyntax::Array {
                    span: ty.span().join(close.span),
                    inner: Box::new(ty),
                };
            } else {
                return ty;
            }
        }
    }

    fn parse_string_list(&mut self) -> Option<(Vec<StringLiteral>, SourceSpan)> {
        let open = self.expect_symbol('[')?;
        let mut values = Vec::new();

        while !self.is_at_end() && !self.at_symbol(']') {
            values.push(self.expect_string("skill string")?);
            if self.at_symbol(',') {
                self.advance();
            } else if !self.at_symbol(']') {
                self.unexpected("`,` or `]`");
                self.synchronize_to_block_item();
                break;
            }
        }

        let close = self.expect_symbol(']')?;
        Some((values, open.span.join(close.span)))
    }

    fn expect_keyword(&mut self, keyword: &str) -> Option<Token> {
        if self.at_ident(keyword) {
            Some(self.advance().clone())
        } else {
            self.expected(format!("`{keyword}`"));
            None
        }
    }

    fn expect_ident(&mut self, label: &str) -> Option<Ident> {
        let token = self.peek()?;
        if let TokenKind::Ident(name) = &token.kind {
            let ident = Ident {
                name: name.clone(),
                span: token.span,
            };
            self.advance();
            Some(ident)
        } else {
            self.expected(label);
            None
        }
    }

    fn expect_string(&mut self, label: &str) -> Option<StringLiteral> {
        let token = self.peek()?;
        if let TokenKind::String(value) = &token.kind {
            let literal = StringLiteral {
                value: value.clone(),
                span: token.span,
            };
            self.advance();
            Some(literal)
        } else {
            self.expected(label);
            None
        }
    }

    fn expect_use_name(&mut self, label: &str) -> Option<StringLiteral> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Ident(value) | TokenKind::String(value) => {
                let literal = StringLiteral {
                    value: value.clone(),
                    span: token.span,
                };
                self.advance();
                Some(literal)
            }
            _ => {
                self.expected(label);
                None
            }
        }
    }

    fn expect_u32(&mut self, label: &str) -> Option<(u32, SourceSpan)> {
        let token = self.peek()?;
        if let TokenKind::Number(value) = &token.kind {
            let span = token.span;
            let parsed = value.parse::<u32>();
            self.advance();
            match parsed {
                Ok(value) => Some((value, span)),
                Err(_) => {
                    self.diagnostics.push(Diagnostic {
                        span,
                        message: format!("{label} must fit in u32"),
                        suggestion: Some("use a non-negative integer such as `1`".to_owned()),
                    });
                    None
                }
            }
        } else {
            self.expected(label);
            None
        }
    }

    fn expect_symbol(&mut self, symbol: char) -> Option<Token> {
        if self.at_symbol(symbol) {
            Some(self.advance().clone())
        } else {
            self.expected(format!("`{symbol}`"));
            None
        }
    }

    fn expect_arrow(&mut self) -> Option<Token> {
        if self.at_arrow() {
            Some(self.advance().clone())
        } else {
            self.expected("`=>`");
            None
        }
    }

    fn expect_thin_arrow(&mut self) -> Option<Token> {
        if self.at_thin_arrow() {
            Some(self.advance().clone())
        } else {
            self.expected("`->`");
            None
        }
    }

    fn at_ident(&self, expected: &str) -> bool {
        matches!(self.peek().map(|token| &token.kind), Some(TokenKind::Ident(value)) if value == expected)
    }

    fn at_symbol(&self, expected: char) -> bool {
        matches!(self.peek().map(|token| &token.kind), Some(TokenKind::Symbol(value)) if *value == expected)
    }

    fn at_arrow(&self) -> bool {
        matches!(self.peek().map(|token| &token.kind), Some(TokenKind::Arrow))
    }

    fn at_thin_arrow(&self) -> bool {
        matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::ThinArrow)
        )
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> &Token {
        let index = self.pos;
        self.pos += 1;
        &self.tokens[index]
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn expected(&mut self, expected: impl fmt::Display) {
        let expected = expected.to_string();
        let (span, found) = match self.peek() {
            Some(token) => (token.span, token.kind.label()),
            None => (
                SourceSpan {
                    start: self.source.len(),
                    end: self.source.len(),
                },
                "end of file".to_owned(),
            ),
        };
        self.diagnostics.push(Diagnostic {
            span,
            message: format!("expected {expected}, found {found}"),
            suggestion: suggestion_for_expected(&expected),
        });
    }

    fn unexpected(&mut self, expected: impl fmt::Display) {
        let Some(token) = self.peek() else {
            self.expected(expected);
            return;
        };
        let expected = expected.to_string();
        self.diagnostics.push(Diagnostic {
            span: token.span,
            message: format!("expected {expected}, found {}", token.kind.label()),
            suggestion: suggestion_for_expected(&expected),
        });
    }

    fn synchronize_to_block_item(&mut self) {
        while !self.is_at_end() {
            if self.at_symbol('}')
                || self.at_ident("profile")
                || self.at_ident("capacity")
                || self.at_ident("skills")
                || self.at_ident("capabilities")
            {
                return;
            }
            self.advance();
        }
    }

    fn source_text(&self, span: SourceSpan) -> &str {
        &self.source[span.start..span.end]
    }
}

fn trimmed_source_text(source: &str, span: SourceSpan) -> (String, SourceSpan) {
    let leading = source.len() - source.trim_start().len();
    let trailing = source.len() - source.trim_end().len();
    let end = source.len().saturating_sub(trailing);
    if leading > end {
        return (
            String::new(),
            SourceSpan {
                start: span.end,
                end: span.end,
            },
        );
    }
    (
        source[leading..end].to_owned(),
        SourceSpan {
            start: span.start + leading,
            end: span.start + end,
        },
    )
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "int"
            | "float"
            | "bool"
            | "null"
            | "duration"
            | "time"
            | "image"
            | "audio"
            | "pdf"
            | "video"
    )
}

fn suggestion_for_expected(expected: &str) -> Option<String> {
    match expected {
        "`{`" => Some("add a `{ ... }` block".to_owned()),
        "`=>`" => Some("add `=> { ... }` after the rule conditions".to_owned()),
        "`->`" => Some("add `-> OutputType` before the coerce prompt block".to_owned()),
        "profile string" => Some("write `profile \"profile-name\"`".to_owned()),
        "capacity value" => Some("write `capacity 1`".to_owned()),
        "plugin name" => Some("write a plugin name, such as `memory`".to_owned()),
        "type name" => Some("write a primitive type or schema name".to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_scaffold_links_to_core() {
        assert_eq!(parser_stage(), "stage-0-skeleton");
    }

    #[test]
    fn parses_schema_agent_and_rule_slice() {
        let source = r#"
workflow LoftWorkerWithReview

use memory

enum ReviewStatus {
  Accept
  Revise
}

class WorkReview {
  state "accepted" | "rejected"
  status ReviewStatus
  followups string[]
  maybeReason string?
  scores map<int>
}

coerce reviewWork(issueTitle string, changedFiles string[]) -> WorkReview {
  prompt """
  Review {{ issueTitle }} with files {{ changedFiles }}
  """
}

agent worker {
  profile "repo-writer"
  capacity 1
  skills ["loft-user"]
}

rule start_ready_issue
  when loft has ready issue as issue
  when worker is available
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """
    Implement {{ claim.issue.title }}
    """
  }
}
"#;

        let parsed = parse_program(source);
        assert_eq!(parsed.diagnostics, Vec::new());
        let workflow = parsed
            .program
            .workflow
            .as_ref()
            .map(|ident| ident.name.as_str());
        assert_eq!(workflow, Some("LoftWorkerWithReview"));
        assert_eq!(parsed.program.items.len(), 6);

        let coerce = parsed.program.items.iter().find_map(|item| match item {
            Item::Coerce(coerce) => Some(coerce),
            _ => None,
        });
        let coerce = match coerce {
            Some(coerce) => coerce,
            None => panic!("expected coerce item"),
        };
        assert_eq!(coerce.params.len(), 2);

        let rule = parsed.program.items.iter().find_map(|item| match item {
            Item::Rule(rule) => Some(rule),
            _ => None,
        });
        let rule = match rule {
            Some(rule) => rule,
            None => panic!("expected rule item"),
        };
        assert_eq!(rule.whens.len(), 2);
        assert_eq!(rule.whens[0].text, "loft has ready issue as issue");
        assert!(rule.body.text.contains("after claim succeeds"));
    }

    #[test]
    fn use_short_form_imports_plugins_and_rejects_legacy_kinds() {
        let parsed = parse_program("workflow Imports\n\nuse memory\n");
        assert_eq!(parsed.diagnostics, Vec::new());
        let use_decl = parsed.program.items.iter().find_map(|item| match item {
            Item::Use(use_decl) => Some(use_decl),
            _ => None,
        });
        assert_eq!(
            use_decl.map(|decl| decl.name.value.as_str()),
            Some("memory")
        );

        let legacy_plugin = parse_program("workflow Imports\n\nuse plugin \"memory\"\n");
        assert_eq!(legacy_plugin.diagnostics.len(), 1);
        assert_eq!(
            legacy_plugin.diagnostics[0].message,
            "`use plugin` is no longer supported"
        );

        let legacy_skill = parse_program("workflow Imports\n\nuse skill \"loft-user\"\n");
        assert_eq!(legacy_skill.diagnostics.len(), 1);
        assert_eq!(
            legacy_skill.diagnostics[0].message,
            "`use skill` is no longer supported"
        );
    }

    #[test]
    fn parses_include_declarations_and_records_ir_metadata() {
        let source = r#"include "library.whip"

workflow Imports

class Task {
  id string
}
"#;
        let parsed = parse_program(source);
        assert_eq!(parsed.diagnostics, Vec::new());
        let include = parsed.program.items.iter().find_map(|item| match item {
            Item::Include(include) => Some(include),
            _ => None,
        });
        assert_eq!(
            include.map(|decl| decl.path.value.as_str()),
            Some("library.whip")
        );

        let compiled = compile_program(source);
        let ir = compiled.ir.expect("source compiles");
        assert_eq!(ir.includes[0].path, "library.whip");
        assert!(ir.to_snapshot().contains("includes\n  library.whip\n"));
    }

    #[test]
    fn parses_explicit_workflow_block_and_contracts() {
        let source = r#"
workflow ReviewPhase {
  input phase PhaseReviewRequest
  output result PhaseReviewResult
  failure error ReviewFailure

  class PhaseReviewRequest {
    title string
  }

  class PhaseReviewResult {
    accepted bool
  }

  class ReviewFailure {
    reason string
  }

  rule noop
    when started
  => {
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("source compiles");
        assert_eq!(ir.workflow, "ReviewPhase");
        assert_eq!(ir.workflow_contracts.len(), 3);
        let snapshot = ir.to_snapshot();
        assert!(snapshot.contains("workflow_contracts\n  input phase ref<PhaseReviewRequest>"));
        assert!(snapshot.contains("  output result ref<PhaseReviewResult>"));
        assert!(snapshot.contains("  failure error ref<ReviewFailure>"));
    }

    #[test]
    fn expands_pattern_applications_with_hygienic_names() {
        let source = r#"
pattern Review<Input> {
  class Result {
    item Input
  }

  rule dispatch
    when Input as item
  => {
  }
}

workflow Root {
  class Task {
    title string
  }

  apply Review<Task> as taskReview {
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("source compiles");
        let snapshot = ir.to_snapshot();
        assert!(snapshot.contains("pattern_applications\n  Review as taskReview<ref<Task>>"));
        assert!(snapshot.contains("    generated class:taskReview_Result"));
        assert!(snapshot.contains("    generated rule:taskReview_dispatch"));
        assert!(snapshot.contains("class taskReview_Result"));
        assert!(snapshot.contains("    item ref<Task>"));
        assert!(snapshot.contains("rule taskReview_dispatch"));
        assert!(snapshot.contains("    when Task as item"));
    }

    #[test]
    fn parses_workflow_invoke_effect_metadata() {
        let source = r#"
workflow Parent {
  class Task {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task task } as child
  }
}

workflow Child {
  input task Task

  class Task {
    title string
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("source compiles");
        let rule = ir
            .rules
            .iter()
            .find(|rule| rule.name == "dispatch")
            .unwrap();
        assert_eq!(rule.metadata.effects.len(), 1);
        assert_eq!(rule.metadata.effects[0].kind, IrEffectKind::WorkflowInvoke);
        assert_eq!(rule.metadata.effects[0].binding.as_deref(), Some("child"));
        assert!(ir
            .to_snapshot()
            .contains("child kind=workflow.invoke binding=child"));
    }

    #[test]
    fn rejects_unknown_workflow_invocation_target() {
        let source = r#"
workflow Parent {
  class Task {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Missing { task task } as child
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("invokes unknown workflow `Missing`")));
    }

    #[test]
    fn validates_workflow_invocation_inputs_against_target_contract() {
        let source = r#"
workflow Parent {
  class Task {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { wrong task } as child
  }
}

workflow Child {
  input task Task

  class Task {
    title string
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert!(compiled.ir.is_none());
        let messages = compiled
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("workflow `Child` has no input `wrong`")),
            "{messages:#?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message
                    .contains("workflow invocation `Child` is missing input `task`")),
            "{messages:#?}"
        );
    }

    #[test]
    fn validates_nested_workflow_invocation_input_payloads() {
        let source = r#"
workflow Parent {
  class Task {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { count "bad" } } as child
  }
}

workflow Child {
  input task ChildTask

  class ChildTask {
    count int
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("field `ChildTask.count` expects `int`")
        }));
    }

    #[test]
    fn rejects_direct_recursive_workflow_invocation() {
        let source = r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Parent { task task } as next
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("recursively invokes workflow `Parent`")
        }));
    }

    #[test]
    fn expands_pattern_application_value_arguments() {
        let source = r#"
pattern Review<Input> {
  rule dispatch
    when Input as item
  => {
  }
}

workflow Root {
  class Task {
    title string
  }

  apply Review<Task> as taskReview {
    item task
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let snapshot = compiled.ir.expect("source compiles").to_snapshot();
        assert!(snapshot.contains("    arg item task"));
    }

    #[test]
    fn rejects_malformed_pattern_application_arguments() {
        let source = r#"
pattern Review<Input> {
  rule dispatch
    when Input as item
  => {
  }
}

workflow Root {
  class Task {
    title string
  }

  apply Review<Task> as taskReview {
    item
    item task
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("argument `item` is missing a value")));
    }

    #[test]
    fn rejects_unknown_workflow_terminal_actions() {
        let source = r#"
workflow BadTerminal {
  output result Result

  class Result {
    status "ok"
  }

  rule bad
    when started
  => {
    complete missing {
      status "ok"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("completes unknown workflow terminal `missing`")));
    }

    #[test]
    fn rejects_duplicate_workflow_inputs() {
        let source = r#"
workflow DuplicateInput {
  input phase PhaseRequest
  input phase PhaseRequest

  class PhaseRequest {
    title string
  }

  rule noop
    when started
  => {
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("workflow declares input `phase` more than once")));
    }

    #[test]
    fn validates_workflow_terminal_payload_fields() {
        let source = r#"
workflow BadTerminalPayload {
  output result Result

  class Result {
    status "ok"
    summary string
  }

  rule bad
    when started
  => {
    complete result {
      status "bad"
      extra "ignored"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `Result.status` expects literal string `ok`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("class `Result` has no field `extra`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("workflow terminal `result` is missing required field `Result.summary`")));
    }

    #[test]
    fn rejects_workflow_terminal_actions_outside_explicit_workflow_body() {
        let source = r#"
workflow LegacyTerminal

output result Result

class Result {
  status "ok"
}

rule bad
  when started
=> {
  complete result {
    status "ok"
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("uses `complete` outside an explicit workflow body")));
    }

    #[test]
    fn rejects_non_class_workflow_terminal_payload_contracts_for_now() {
        let source = r#"
workflow ScalarTerminal {
  output result string

  rule bad
    when started
  => {
    complete result {
      value "ok"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("workflow terminal `result` uses a non-class payload contract")));
    }

    #[test]
    fn selects_root_from_multiple_explicit_workflows() {
        let source = r#"
class Shared {
  id string
}

workflow First {
  rule one
    when started
  => {
    record Shared {
      id "first"
    }
  }
}

workflow Second {
  rule two
    when started
  => {
    record Shared {
      id "second"
    }
  }
}
"#;
        let ambiguous = compile_program(source);
        assert!(ambiguous.ir.is_none());
        assert!(ambiguous.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("multiple workflow declarations require an explicit root")));

        let compiled = compile_program_with_root(source, Some("Second"));
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("selected root compiles");
        assert_eq!(ir.workflow, "Second");
        assert_eq!(ir.rules.len(), 1);
        assert_eq!(ir.rules[0].name, "two");
        assert!(ir.to_snapshot().contains("class Shared"));
    }

    #[test]
    fn reports_recoverable_diagnostics() {
        let source = r#"
workflow Broken

agent worker {
  profile 42
  capacity nope
}

rule missing_body
  when started
=>
"#;

        let parsed = parse_program(source);
        assert!(parsed.diagnostics.len() >= 3);
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("profile string")));
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.suggestion.as_deref()
                == Some("write `profile \"profile-name\"`")));
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("capacity value")));
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("`{`")));
    }

    #[test]
    fn accepts_agent_ref_dynamic_tell_targets() {
        let source = r#"
workflow AgentRefRouting

agent codex {
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

agent claude {
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

class LanguageTask {
  provider AgentRef<codex | claude>
  prompt string
}

rule run_task
  when LanguageTask as task
  when task.provider is available
=> {
  tell task.provider requires ["agent.tell"] as turn "{{ task.prompt }}"
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let rule = ir
            .rules
            .iter()
            .find(|rule| rule.name == "run_task")
            .expect("run_task");
        assert_eq!(rule.metadata.effects.len(), 1);
        assert_eq!(rule.metadata.effects[0].kind, IrEffectKind::AgentTell);
    }

    #[test]
    fn rejects_agent_ref_targets_missing_required_capabilities() {
        let source = r#"
workflow BadAgentRefCapabilities

agent codex {
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell", "repo.write"]
}

agent claude {
  profile "repo-reader"
  capacity 1
  capabilities ["agent.tell"]
}

class LanguageTask {
  provider AgentRef<codex | claude>
  prompt string
}

rule run_task
  when LanguageTask as task
=> {
  tell task.provider requires ["repo.write"] as turn """
  {{ task.prompt }}
  """
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("agent `claude` requiring undeclared capability `repo.write`")));
    }

    #[test]
    fn rejects_plain_string_dynamic_tell_targets() {
        let source = r#"
workflow BadAgentRefRouting

agent codex {
  profile "repo-writer"
  capacity 1
}

class LanguageTask {
  provider string
}

rule run_task
  when LanguageTask as task
=> {
  tell task.provider "bad"
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-AgentRef dynamic tell target `task.provider`")));
    }

    #[test]
    fn rejects_unknown_agent_ref_domain_values() {
        let source = r#"
workflow BadAgentRefDomain

agent codex {
  profile "repo-writer"
  capacity 1
}

class LanguageTask {
  provider AgentRef<codex | pi>
}

rule seed
  when started
=> {
  record LanguageTask {
    provider claude
  }
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("AgentRef references unknown agent `pi`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `LanguageTask.provider` cannot reference agent `claude`")));
    }

    #[test]
    fn rejects_quoted_agent_ref_record_values() {
        let source = r#"
workflow BadQuotedAgentRef

agent codex {
  profile "repo-writer"
  capacity 1
}

class LanguageTask {
  provider AgentRef<codex>
}

rule seed
  when started
=> {
  record LanguageTask {
    provider "codex"
  }
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("expects an AgentRef value, not string `codex`")));
    }

    #[test]
    fn requires_presence_proof_for_optional_field_access() {
        let source = r#"
workflow OptionalProof

class Person {
  name string
}

class Issue {
  assignee Person?
}

rule unsafe_optional
  when Issue as issue where issue.assignee.name == "Ada"
=> {
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unsafe optional path `issue.assignee.name`")));
    }

    #[test]
    fn accepts_presence_proof_before_optional_field_access() {
        let source = r#"
workflow OptionalProof

class Person {
  name string
}

class Issue {
  assignee Person?
}

rule safe_optional
  when Issue as issue where issue.assignee != null && issue.assignee.name == "Ada"
=> {
}

rule safe_exists
  when Issue as issue where exists issue.assignee && issue.assignee.name == "Ada"
=> {
}

rule safe_not_null
  when Issue as issue where !(issue.assignee == null) && issue.assignee.name == "Ada"
=> {
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn parses_expression_kernel_surface() {
        let cases = [
            ("true || false && !ready", "true || (false && !ready)"),
            (
                "empty(task.labels) || exists(Result where status == \"done\")",
                "empty(task.labels) || exists(Result where status == \"done\")",
            ),
            (
                "task.labels[\"priority\"] == [\"high\", \"urgent\"][0]",
                "task.labels[\"priority\"] == [\"high\", \"urgent\"][0]",
            ),
            (
                "exists issue.assignee && issue.assignee.name == \"Ada\"",
                "exists(issue.assignee) && (issue.assignee.name == \"Ada\")",
            ),
            (
                "{title task.title, metadata {phase \"kernel\"}}",
                "{title task.title, metadata {phase \"kernel\"}}",
            ),
            (
                "count(effect agent.tell where target == \"worker\") >= 1",
                "count(effect agent.tell where target == \"worker\") >= 1",
            ),
        ];

        for (source, expected) in cases {
            let expr = parse_expression(source).expect(source);
            assert_eq!(expr.to_snapshot(), expected);
        }

        for source in ["task.labels[", "count(Result where)", "[1,,2]"] {
            assert!(
                parse_expression(source).is_err(),
                "{source} unexpectedly parsed"
            );
        }
    }

    #[test]
    fn validates_expected_schema_object_and_map_record_fields() {
        let source = r#"
workflow ObjectRecordFields

class Owner {
  name string
}

class Task {
  title string
  metadata map<string>
  owner Owner?
}

rule seed
  when started
=> {
    record Task {
    title "Implement object literals"
    metadata { phase "kernel" }
    owner { name "Ada" }
  }

  record Task {
    title "Implement multiline object literals"
    metadata {
      phase "kernel"
      owner "Ada"
    }
    owner {
      name "Ada"
    }
  }
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn rejects_invalid_expected_schema_object_and_map_record_fields() {
        let source = r#"
workflow BadObjectRecordFields

class Owner {
  name string
}

class Task {
  metadata map<string>
  owner Owner
}

rule seed
  when started
=> {
  record Task {
    metadata { phase 1 }
    owner { alias "Ada" }
  }
}

rule bad_guard
  when Task as task where { phase "kernel" } == task.metadata
=> {
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `Task.metadata` expects `string`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("class `Owner` has no field `alias`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("missing required object field `Owner.name`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("compares incompatible expression types")));
    }

    #[test]
    fn rejects_invalid_expression_types() {
        let source = r#"
workflow BadExpressionTypes

class Task {
  title string
  labels map<string>
  priority int
  ready bool
}

rule non_bool_guard
  when Task as task where task.priority
=> {
}

rule bad_ordering
  when Task as task where task.title > "abc"
=> {
}

rule bad_membership
  when Task as task where task.title in task.priority
=> {
}

rule bad_equality
  when Task as task where task.ready == "yes"
=> {
}

rule bad_array
  when Task as task where task.title in ["abc", 1]
=> {
}

rule bad_map_key
  when Task as task where task.labels[1] == "urgent"
=> {
}

rule bad_map_membership
  when Task as task where 1 in task.labels
=> {
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-boolean guard expression")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("orders non-orderable expression values")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("uses membership against a non-array/non-map expression")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("compares incompatible expression types")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("mixed-type array literal")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-string key")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("map membership with a non-string key")));
    }

    #[test]
    fn validates_duration_and_time_ordering_and_literals() {
        let source = r#"
workflow DurationTimeExpressions

class Window {
  elapsed duration
  limit duration
  opened_at time
  due_at time
}

assert exists(Window where elapsed < limit)
assert exists(Window where opened_at <= due_at)

rule seed
  when started
=> {
  record Window {
    elapsed "PT30.5M"
    limit "PT1.25H"
    opened_at "2026-05-29T10:00:00.250-04:00"
    due_at "2026-05-29T14:00:00.500Z"
  }
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn rejects_invalid_duration_and_time_literals() {
        let source = r#"
workflow BadDurationTimeExpressions

class Window {
  elapsed duration
  limit duration
  opened_at time
}

rule seed
  when started
=> {
  record Window {
    elapsed "thirty minutes"
    limit "P1M"
    opened_at "morning"
  }
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `Window.elapsed` has invalid duration literal")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `Window.limit` has invalid duration literal")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("field `Window.opened_at` has invalid time literal")));
    }

    #[test]
    fn validates_assertion_expression_types_and_paths() {
        let source = r#"
workflow BadAssertions

class Task {
  provider "codex" | "claude"
  priority int
}

assert count(Task where provider == "bad") == 0
assert count(Task)
assert missing.root == "value"
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assertion compares finite-domain value to unknown `bad`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assertion has non-boolean assertion expression")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assertion has unknown expression root `missing`")));
    }

    #[test]
    fn validates_symmetric_finite_domain_literals_and_unknown_guard_roots() {
        let source = r#"
workflow SymmetricFiniteDomain

enum ReviewStatus {
  Accept
  Revise
}

class Task {
  status ReviewStatus
  provider "codex" | "claude"
}

rule symmetric_literal
  when Task as task where "bad" == task.provider
=> {
}

rule enum_variant_literal
  when Task as task where Missing == task.status
=> {
}

rule array_membership_literal
  when Task as task where task.provider in ["codex", "bad"]
=> {
}

rule implicit_query_head
  when Task as task where exists(Task where status == Missing)
=> {
}

rule unknown_root
  when Task as task where other.provider == "codex"
=> {
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("finite-domain value to unknown `bad`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("finite-domain value to unknown `Missing`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unknown expression root `other`")));
    }

    #[test]
    fn rejects_unsatisfiable_finite_domain_expression_relations() {
        let source = r#"
workflow UnsatisfiableFiniteDomains

class Task {
  provider "codex" | "claude"
  route "pi" | "baml"
}

rule disjoint_equality
  when Task as task where task.provider == task.route
=> {
}

rule empty_membership
  when Task as task where task.provider in []
=> {
}

rule excluded_membership
  when Task as task where task.provider not in ["codex", "claude"]
=> {
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("statically unsatisfiable finite-domain equality")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("statically unsatisfiable finite-domain membership")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("statically unsatisfiable finite-domain exclusion")));
    }

    #[test]
    fn accepts_map_index_expressions() {
        let source = r#"
workflow MapIndex

class Task {
  labels map<string>
}

rule route
  when Task as task where task.labels["priority"] == "high"
=> {
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let guard = ir.rules[0].whens[0].guard.as_ref().expect("guard");
        assert_eq!(
            guard.expr.to_snapshot(),
            "task.labels[\"priority\"] == \"high\""
        );
    }

    #[test]
    fn lowers_deterministic_ir_snapshot() {
        let source = r#"
workflow Snapshot


class Work {
  title string
  files string[]
  state "open" | "done"
}

agent worker {
  profile "repo-writer"
  capacity 2
  skills ["loft-user"]
}

rule start
  when Work as work
=>
{
  tell worker "{{ work.title }}"
}

rule finish
  when Result as result
=>
{
  record Work {
    title result.title
    files result.files
    state "done"
  }
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = match compiled.ir {
            Some(ir) => ir,
            None => panic!("expected lowered IR"),
        };

        let expected = "\
workflow Snapshot
schemas
  class Work
    title string
    files array<string>
    state union<literal<\"open\"> | literal<\"done\">>
agents
  agent worker profile=repo-writer capacity=2 skills=[loft-user] capabilities=[]
rules
  rule start
    when Work as work
    reads
      schema:Work
    effects
      effect1 kind=agent.tell binding=- key=ee9ce01260428bc3
    body_hash c94fd61e67dfb2e5
  rule finish
    when Result as result
    reads
      schema:Result
    writes
      schema:Work
    body_hash e48280a8be017f84
rule_dependencies
  finish --schema:Work--> start
";

        assert_eq!(ir.to_snapshot(), expected);
    }

    #[test]
    fn example_ir_snapshots_are_stable() {
        let examples = [
            (
                include_str!("../../../examples/minimal-noop.whip"),
                include_str!("../../../examples/minimal-noop.ir"),
            ),
            (
                include_str!("../../../examples/loft-worker-with-review.whip"),
                include_str!("../../../examples/loft-worker-with-review.ir"),
            ),
            (
                include_str!("../../../examples/coerce-branch.whip"),
                include_str!("../../../examples/coerce-branch.ir"),
            ),
            (
                include_str!("../../../examples/codex-french-poem-dogfood.whip"),
                include_str!("../../../examples/codex-french-poem-dogfood.ir"),
            ),
            (
                include_str!("../../../examples/codex-poem-coerce-review.whip"),
                include_str!("../../../examples/codex-poem-coerce-review.ir"),
            ),
            (
                include_str!("../../../examples/human-review.whip"),
                include_str!("../../../examples/human-review.ir"),
            ),
            (
                include_str!("../../../examples/multi-agent-bounded-concurrency.whip"),
                include_str!("../../../examples/multi-agent-bounded-concurrency.ir"),
            ),
            (
                include_str!("../../../examples/openclaw-lite.whip"),
                include_str!("../../../examples/openclaw-lite.ir"),
            ),
            (
                include_str!("../../../examples/plugin-memory.whip"),
                include_str!("../../../examples/plugin-memory.ir"),
            ),
            (
                include_str!("../../../examples/provider-language-e2e.whip"),
                include_str!("../../../examples/provider-language-e2e.ir"),
            ),
            (
                include_str!("../../../examples/companion-skill-dogfood.whip"),
                include_str!("../../../examples/companion-skill-dogfood.ir"),
            ),
            (
                include_str!("../../../examples/expression-kernel-dogfood.whip"),
                include_str!("../../../examples/expression-kernel-dogfood.ir"),
            ),
            (
                include_str!("../../../examples/terminal-output-union.whip"),
                include_str!("../../../examples/terminal-output-union.ir"),
            ),
            (
                include_str!("../../../examples/ralph.whip"),
                include_str!("../../../examples/ralph.ir"),
            ),
        ];

        for (source, expected) in examples {
            let compiled = compile_program(source);
            assert_eq!(compiled.diagnostics, Vec::new());
            let ir = match compiled.ir {
                Some(ir) => ir,
                None => panic!("expected lowered IR"),
            };
            assert_eq!(ir.to_snapshot(), expected);
        }
    }

    #[test]
    fn rejects_unknown_schema_references() {
        let source = include_str!("../../../examples/invalid/unknown-schema.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 2);
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "unknown schema reference `MissingStatus`"));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "unknown schema reference `MissingOutput`"));
    }

    #[test]
    fn rejects_invalid_agent_declarations() {
        let source = include_str!("../../../examples/invalid/bad-agent.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 4);
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("capacity must be greater than zero")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("more than once")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown agent field")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing a profile")));
    }

    #[test]
    fn rejects_invalid_effect_dependencies() {
        let source = include_str!("../../../examples/invalid/bad-effect-graph.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 2);
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown effect binding")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unsupported `after`")));
    }

    #[test]
    fn accepts_equality_guards_in_when_clauses() {
        let source = r#"
workflow GuardGuess

class WorkItem {
  state "ready" | "blocked"
}

rule branch
  when WorkItem as item where item.state == "ready"
=> {
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let when = ir
            .rules
            .iter()
            .flat_map(|rule| &rule.whens)
            .find(|when| when.source == "WorkItem as item where item.state == \"ready\"")
            .expect("guarded when");
        assert_eq!(when.pattern, "WorkItem as item");
        assert_eq!(
            when.guard.as_ref().map(|guard| guard.expr.to_snapshot()),
            Some("item.state == \"ready\"".to_owned())
        );
    }

    #[test]
    fn lowers_assertions_to_parsed_expression_ir() {
        let source = r#"
workflow AssertionGuess

class Result {
  status "done"
}

assert count(Result where status == "done") == 1
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let assertion = ir.assertions.first().expect("assertion");
        assert_eq!(
            assertion.expr.source,
            "count(Result where status == \"done\") == 1"
        );
        assert_eq!(
            assertion.expr.expr.to_snapshot(),
            "count(Result where status == \"done\") == 1"
        );
        assert_eq!(
            assertion
                .projection_reads
                .iter()
                .map(IrProjectionRead::to_snapshot)
                .collect::<Vec<_>>(),
            vec!["fact:Result where status == \"done\""]
        );
    }

    #[test]
    fn lowers_guard_projection_reads_to_rule_metadata() {
        let source = r#"
workflow GuardProjection

class Task {
  status "ready"
}

class Result {
  status "done"
}

rule gated
  when Task as task where exists(Result where status == "done")
=> {
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let rule = ir.rules.first().expect("rule");
        assert_eq!(
            rule.metadata
                .projection_reads
                .iter()
                .map(IrProjectionRead::to_snapshot)
                .collect::<Vec<_>>(),
            vec!["fact:Result where status == \"done\""]
        );
    }

    #[test]
    fn accepts_typed_case_branches_in_rule_bodies() {
        let source = r#"
workflow CaseGuess

enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class Review {
  status ReviewStatus
  assignee string?
}

class Routed {
  status ReviewStatus
}

rule route
  when Review as review
=> {
  case review.status {
    Accept => {
      record Routed {
        status Accept
      }
    }
    Revise => {
      record Routed {
        status Revise
      }
    }
    Blocked => {
      record Routed {
        status Blocked
      }
    }
  }

  case review.assignee {
    Some owner => {
      record Routed {
        status Accept
      }
    }
    None => {
      record Routed {
        status Blocked
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn accepts_terminal_output_case_branches_inside_completes_after() {
        let source = r#"
workflow TerminalCaseGuess

class WorkItem {
  title string
}

class MessageClassification {
  summary string
}

class Routed {
  branch string
  detail string
}

coerce classifyMessage(title string) -> MessageClassification {
  prompt "Classify"
}

rule classify
  when WorkItem as item
=> {
  coerce classifyMessage(item.title) as classification

  after classification completes {
    case classification {
      Completed result => {
        record Routed {
          branch "completed"
          detail result.summary
        }
      }
      Failed failure => {
        record Routed {
          branch "failed"
          detail failure.reason
        }
      }
      TimedOut timeout => {
        record Routed {
          branch "timed_out"
          detail timeout.summary
        }
      }
      Cancelled cancel => {
        record Routed {
          branch "cancelled"
          detail cancel.summary
        }
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn lowers_terminal_output_case_branches_to_typed_ir() {
        let source = include_str!("../../../examples/terminal-output-union.whip");
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("expected lowered IR");
        let rule = ir
            .rules
            .iter()
            .find(|rule| rule.name == "classify_work")
            .expect("rule");

        let terminal_output = rule
            .metadata
            .terminal_outputs
            .iter()
            .find(|output| output.binding == "classification")
            .expect("terminal output");
        assert_eq!(terminal_output.alternatives.len(), 4);
        assert_eq!(
            terminal_output.alternatives[0].payload_type,
            IrType::Ref("Classification".to_owned())
        );
        assert_eq!(
            rule.metadata
                .terminal_branches
                .iter()
                .map(|branch| {
                    (
                        branch.tag.as_deref().unwrap_or("_"),
                        branch.binding.as_deref().unwrap_or("-"),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("Completed", "result"),
                ("Failed", "failure"),
                ("TimedOut", "timeout"),
                ("Cancelled", "cancel"),
            ]
        );
    }

    #[test]
    fn rejects_terminal_payload_fields_outside_refined_tag_schema() {
        let source = r#"
workflow BadTerminalPayload

class WorkItem {
  title string
}

class Classification {
  summary string
}

class TerminalRoute {
  detail string
}

coerce classify(title string) -> Classification {
  prompt "Classify"
}

rule classify_work
  when WorkItem as item
=> {
  coerce classify(item.title) as classification

  after classification completes {
    case classification {
      Completed result => {
        record TerminalRoute {
          detail result.reason
        }
      }
      Failed failure => {
        record TerminalRoute {
          detail failure.reason
        }
      }
      TimedOut timeout => {
        record TerminalRoute {
          detail timeout.summary
        }
      }
      Cancelled cancel => {
        record TerminalRoute {
          detail cancel.summary
        }
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("invalid field path `result.reason`")));
    }

    #[test]
    fn rejects_invalid_terminal_output_case_branches() {
        let source = r#"
workflow BadTerminalCaseGuess

class WorkItem {
  title string
}

class MessageClassification {
  summary string
}

coerce classifyMessage(title string) -> MessageClassification {
  prompt "Classify"
}

rule classify
  when WorkItem as item
=> {
  coerce classifyMessage(item.title) as classification

  after classification completes {
    case classification {
      Success result => {
      }
      Completed result => {
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("terminal-output case pattern cannot be `Success`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive terminal-output case; missing Failed, TimedOut, Cancelled")));
    }

    #[test]
    fn rejects_invalid_case_branch_patterns() {
        let source = r#"
workflow BadCaseGuess

enum ReviewStatus {
  Accept
  Revise
}

class Review {
  status ReviewStatus
  assignee string
}

rule route
  when Review as review
=> {
  case review.status {
    Missing => {
    }
  }

  case review.assignee {
    Some owner => {
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        let missing = compiled
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("enum `ReviewStatus` has no variant `Missing`")
            })
            .expect("missing variant diagnostic");
        assert!(source[missing.span.start..missing.span.end].contains("Mis"));
        let some = compiled
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("uses `Some` for a non-optional case")
            })
            .expect("some diagnostic");
        assert!(source[some.span.start..some.span.end].contains("Some"));
    }

    #[test]
    fn diagnoses_non_exhaustive_and_duplicate_case_branches() {
        let source = r#"
workflow CaseCoverageGuess

enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class Review {
  status ReviewStatus
  provider "codex" | "claude" | "pi"
  owner string?
}

rule route
  when Review as review
=> {
  case review.status {
    Accept => {
    }
    Accept => {
    }
    Revise => {
    }
  }

  case review.provider {
    "codex" => {
    }
    "claude" => {
    }
  }

  case review.owner {
    Some owner => {
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("duplicate unguarded case pattern `Accept`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive case; missing Blocked")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive case; missing pi")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive case; missing None")));
    }

    #[test]
    fn accepts_fallback_and_guarded_duplicate_case_branches() {
        let source = r#"
workflow CaseFallbackGuess

enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class Review {
  status ReviewStatus
  owner string?
}

rule route
  when Review as review
=> {
  case review.status {
    Accept where review.owner != null => {
    }
    Accept where review.owner == null => {
    }
    _ => {
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn explains_multiline_string_binding_position() {
        let source = r#"
workflow BindingGuess

agent worker {
  profile "repo-writer"
  capacity 1
}

rule branch
  when started
=> {
  tell worker """
  do work
  """ as turn

  after turn succeeds {
    askHuman "review"
  }
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("places effect binding `turn` after a multiline string delimiter")
            && diagnostic.suggestion.as_deref().is_some_and(
                |suggestion| suggestion.contains("move `as turn` onto the effect line")
            )));
    }

    #[test]
    fn invalid_fixtures_have_actionable_diagnostics() {
        let fixtures = [
            (
                "bad-agent",
                include_str!("../../../examples/invalid/bad-agent.whip"),
            ),
            (
                "bad-record",
                include_str!("../../../examples/invalid/bad-record.whip"),
            ),
            (
                "bad-effect-graph",
                include_str!("../../../examples/invalid/bad-effect-graph.whip"),
            ),
            (
                "bad-effect-payload",
                include_str!("../../../examples/invalid/bad-effect-payload.whip"),
            ),
            (
                "bad-expression-functions",
                include_str!("../../../examples/invalid/bad-expression-functions.whip"),
            ),
            (
                "bad-finite-domain",
                include_str!("../../../examples/invalid/bad-finite-domain.whip"),
            ),
            (
                "broken",
                include_str!("../../../examples/invalid/broken.whip"),
            ),
            (
                "effect-output-scope",
                include_str!("../../../examples/invalid/effect-output-scope.whip"),
            ),
            (
                "effectful-self-loop",
                include_str!("../../../examples/invalid/effectful-self-loop.whip"),
            ),
            (
                "unknown-schema",
                include_str!("../../../examples/invalid/unknown-schema.whip"),
            ),
        ];

        for (name, source) in fixtures {
            let compiled = compile_program(source);
            assert!(compiled.ir.is_none(), "{name} unexpectedly compiled");
            assert!(
                !compiled.diagnostics.is_empty(),
                "{name} did not emit diagnostics"
            );
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.suggestion.is_some()),
                "{name} emitted a diagnostic without a suggestion: {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn rejects_invalid_record_fields_paths_and_literals() {
        let source = include_str!("../../../examples/invalid/bad-record.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 5);
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("request.missing")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no variant `Maybe`")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("expects `float`")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot be `scripted`")));
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no field `extra`")));
    }

    #[test]
    fn rejects_effect_output_outside_after_scope() {
        let source = include_str!("../../../examples/invalid/effect-output-scope.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 1);
        assert!(compiled.diagnostics[0]
            .message
            .contains("outside a matching `after claim ...` block"));
    }

    #[test]
    fn rejects_effectful_self_trigger_loop() {
        let source = include_str!("../../../examples/invalid/effectful-self-loop.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 1);
        assert!(compiled.diagnostics[0]
            .message
            .contains("preserves trigger fact `schema:WorkItem`"));
    }

    #[test]
    fn rejects_unknown_or_wrong_arity_coerce_calls() {
        let source = r#"
workflow BadCoerce

class Review {
  reason string
}

coerce review(summary string) -> Review {
  prompt "review"
}

rule bad
  when started
=> {
  coerce missing("x") as one
  coerce review("x", "y") as two
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert_eq!(compiled.diagnostics.len(), 2);
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unknown coerce function `missing`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("with 2 argument(s), expected 1")));
    }

    #[test]
    fn rejects_bad_effect_payload_argument_types() {
        let source = r#"
workflow BadEffectPayloads

class Owner {
  name string
}

class Payload {
  title string
  owner Owner
  metadata map<string>
  tags string[]
}

class Task {
  title string
  owner string
}

class Review {
  accepted bool
}

coerce reviewPayload(payload Payload, metadata map<string>, score int) -> Review {
  prompt "review"
}

rule bad_coerce
  when Task as task where { owner "Ada" } == task.owner
=> {
  coerce reviewPayload(
    {
      title task.title
      owner { handle task.owner }
      metadata { phase 3 }
      tags ["object", 7]
      extra "bad"
    },
    { phase task.owner, count 3 },
    "high"
  ) as review
}

rule bad_claim
  when Task as task
=> {
  claim task.title with loft as claim
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        let messages = compiled
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("object literal without an expected object")));
        assert!(messages
            .iter()
            .any(|message| message.contains("class `Owner` has no field `handle`")));
        assert!(messages
            .iter()
            .any(|message| message.contains("missing required object field `Owner.name`")));
        assert!(messages
            .iter()
            .any(|message| message.contains("class `Payload` has no field `extra`")));
        assert!(messages
            .iter()
            .any(|message| message
                .contains("field `coerce `reviewPayload`.metadata` expects `string`")));
        assert!(messages.iter().any(|message| {
            message.contains("field `coerce `reviewPayload`.score` expects `int`")
        }));
        assert!(messages
            .iter()
            .any(|message| message.contains("field `loft claim.issue` receives incompatible")));
    }

    #[test]
    fn lowers_fact_consumption_metadata() {
        let source = r#"
workflow ConsumeTask

class Task {
  status "queued"
}

rule finish
  when Task as task
=> {
  consume task
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.expect("program compiles");

        assert_eq!(ir.rules[0].metadata.fact_consumes, vec!["schema:Task"]);
        assert!(ir.to_snapshot().contains("consumes\n      schema:Task"));
    }

    #[test]
    fn rejects_unknown_fact_consumption_binding() {
        let source = r#"
workflow BadConsume

class Task {
  status "queued"
}

rule finish
  when Task as task
=> {
  consume missing
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("consumes unknown fact binding `missing`")));
    }

    #[test]
    fn lowers_workflow_sugar_to_existing_metadata() {
        let source = r#"
workflow Sugar

class Task {
  topic string
  status "queued"
}

class Result {
  topic string
  turn AgentTurn
  status "done"
}

agent codex {
  profile "repo-writer"
  capacity 1
}

assert none(Task where status == "queued")
assert one(Result where status == "done")

rule finish
  when Task as task where task.status == "queued"
  when codex is available
=> {
  tell codex as turn "write"
  then done task -> record Result from task {
    topic
    turn turn
    status "done"
  }
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.expect("program compiles");
        let rule = &ir.rules[0];

        assert_eq!(ir.assertions.len(), 2);
        assert_eq!(rule.metadata.fact_consumes, vec!["schema:Task"]);
        assert_eq!(rule.metadata.fact_writes, vec!["schema:Result"]);
        assert_eq!(rule.metadata.effects.len(), 1);
        assert!(ir
            .to_snapshot()
            .contains("assert none(Task where status == \"queued\")"));
    }

    #[test]
    fn formats_top_level_syntax_scaffold() {
        let source = r#"workflow Messy
class Status {
kind "open"|"done"
}
rule start
when started
=> {tell worker "hi"}
"#;

        let formatted = format_program(source);
        assert_eq!(formatted.diagnostics, Vec::new());
        let expected = concat!(
            "workflow Messy\n",
            "\n",
            "class Status {\n",
            "  kind \"open\" | \"done\"\n",
            "}\n",
            "\n",
            "rule start\n",
            "  when started\n",
            "=> {\n",
            "  tell worker \"hi\"\n",
            "}\n",
        );

        assert_eq!(formatted.formatted.as_deref(), Some(expected));
    }

    #[test]
    fn formats_explicit_workflow_blocks() {
        let source = r#"class Shared {
id string
}
workflow One {
input item Shared
rule start
when Shared as item
=> {complete result {id item.id}}
}
"#;

        let formatted = format_program(source);
        assert_eq!(formatted.diagnostics, Vec::new());
        let expected = concat!(
            "class Shared {\n",
            "  id string\n",
            "}\n",
            "\n",
            "workflow One {\n",
            "  input item Shared\n",
            "\n",
            "  rule start\n",
            "    when Shared as item\n",
            "  => {\n",
            "    complete result {id item.id}\n",
            "  }\n",
            "}\n",
        );

        assert_eq!(formatted.formatted.as_deref(), Some(expected));
    }

    #[test]
    fn formats_patterns_and_apply_syntax() {
        let source = r#"pattern Review<Input>{
rule dispatch
when Input as item
=> {}
}
workflow Root {
apply Review<Task> as taskReview {}
}
"#;

        let formatted = format_program(source);
        assert_eq!(formatted.diagnostics, Vec::new());
        let expected = concat!(
            "pattern Review<Input> {\n",
            "  rule dispatch\n",
            "    when Input as item\n",
            "  => {\n",
            "  }\n",
            "}\n",
            "\n",
            "workflow Root {\n",
            "  apply Review<Task> as taskReview {\n",
            "  }\n",
            "}\n",
        );

        assert_eq!(formatted.formatted.as_deref(), Some(expected));
    }
}
