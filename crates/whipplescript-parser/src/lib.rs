//! Source parser for `.whip` programs.
//!
//! The v0 grammar is still stabilizing, so this crate uses a small
//! hand-written parser. It preserves source spans and keeps rule/effect bodies
//! as source text until the typed IR is ready to lower them.

mod action_expand;
pub mod body;
mod flow_expand;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
};
use whipplescript_core::{
    ConstructField, ConstructInterface, ConstructRegistration, ContractRegistry, EffectContract,
    LibraryRegistration, TypedOutputValidation, CONSTRUCT_FAMILY_EFFECT_OPERATION,
    CONSTRUCT_INTERFACE_CAPABILITY, CONSTRUCT_INTERFACE_CARDINALITY_EXACTLY_ONE,
    CONSTRUCT_INTERFACE_PHASE_COMPILE_RUNTIME, CONSTRUCT_LOWERING_CAPABILITY_CALL,
    CONSTRUCT_SCOPE_RULE_BODY,
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
    /// Secondary spans carrying supporting context (spec/error-handling.md "Spans
    /// And Labels"): a `note`-style related-information label pointing at a
    /// definition, prior claim, or other related site. Empty for most
    /// diagnostics; surfaced in CLI text, JSON reports, and LSP
    /// `relatedInformation`.
    pub related: Vec<RelatedInfo>,
}

/// A secondary span + short label attached to a [`Diagnostic`] as related
/// information (never a top-level diagnostic of its own).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelatedInfo {
    pub span: SourceSpan,
    pub message: String,
}

impl Diagnostic {
    /// Attaches a related-information label at `span` (builder style, so the
    /// common no-related case stays a plain struct literal that only needs the
    /// new field defaulted).
    pub fn with_related(mut self, span: SourceSpan, message: impl Into<String>) -> Self {
        self.related.push(RelatedInfo {
            span,
            message: message.into(),
        });
        self
    }
}

/// The marker that introduced a comment, preserved so a formatter can re-emit it
/// faithfully.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommentMarker {
    /// `# …`
    Hash,
    /// `// …`
    Slash,
}

/// A source comment captured by the lexer. Comments are kept out of the token
/// stream (so the parser is unaffected) but retained here so tooling — `whip fmt`,
/// the LSP — can preserve them. `text` is the trimmed content after the marker;
/// `span` covers the marker through end of line (exclusive of the newline).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Comment {
    pub marker: CommentMarker,
    pub text: String,
    pub span: SourceSpan,
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
    pub workflow_tags: Vec<TagDecl>,
    pub workflow_description: Option<StringLiteral>,
    pub explicit_workflow_body: bool,
    pub workflows: Vec<WorkflowDecl>,
    pub patterns: Vec<PatternDecl>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowDecl {
    pub name: Ident,
    pub tags: Vec<TagDecl>,
    pub description: Option<StringLiteral>,
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
    Harness(HarnessDecl),
    Queue(QueueDecl),
    Channel(ChannelDecl),
    FileStore(FileStoreDecl),
    Flow(FlowDecl),
    Action(ActionDecl),
    Agent(AgentDecl),
    Enum(EnumDecl),
    Event(EventDecl),
    Source(SourceDecl),
    Test(TestDecl),
    Lease(LeaseDecl),
    Ledger(LedgerDecl),
    Counter(CounterDecl),
    Class(ClassDecl),
    Table(TableDecl),
    Coerce(CoerceDecl),
    Assert(AssertDecl),
    Rule(RuleDecl),
}

impl Item {
    /// Source span of this top-level item, used to interleave preserved comments.
    fn span(&self) -> SourceSpan {
        match self {
            Self::Include(decl) => decl.path.span,
            Self::Use(decl) => decl.name.span,
            Self::Pattern(decl) => decl.span,
            Self::Apply(decl) => decl.span,
            Self::WorkflowContract(decl) => decl.span,
            Self::Harness(decl) => decl.span,
            Self::Queue(decl) => decl.span,
            Self::Channel(decl) => decl.span,
            Self::FileStore(decl) => decl.span,
            Self::Flow(decl) => decl.span,
            Self::Action(decl) => decl.span,
            Self::Agent(decl) => decl.span,
            Self::Enum(decl) => decl.span,
            Self::Event(decl) => decl.span,
            Self::Source(decl) => decl.span,
            Self::Test(decl) => decl.span,
            Self::Lease(decl) => decl.span,
            Self::Ledger(decl) => decl.span,
            Self::Counter(decl) => decl.span,
            Self::Class(decl) => decl.span,
            Self::Table(decl) => decl.span,
            Self::Coerce(decl) => decl.span,
            Self::Assert(decl) => decl.span,
            Self::Rule(decl) => decl.span,
        }
    }
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
    pub tags: Vec<TagDecl>,
    pub description: Option<StringLiteral>,
    pub expr: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagDecl {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseDecl {
    pub name: StringLiteral,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessDecl {
    pub name: Ident,
    pub kind: Ident,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueueDecl {
    pub name: Ident,
    pub tracker: Ident,
    pub span: SourceSpan,
}

/// `channel <name> { provider <p> [workspace <w>] [destination "<d>"] }`
/// (std.messaging): a named communication route through a provider. The bare
/// `channel` construct shape is reserved by the platform for `std.messaging`
/// (spec/messaging.md), so third-party packages cannot author channel-like
/// semantics with weaker guarantees. Lowers to a `metadata_only` declaration
/// (like `queue`); the runtime messaging provider is later-stage work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelDecl {
    pub name: Ident,
    pub provider: Ident,
    pub workspace: Option<Ident>,
    pub destination: Option<StringLiteral>,
    pub span: SourceSpan,
}

/// `file store <name> { root "<dir>" }` (std.files): a capability-scoped file
/// store identity with a literal root directory. v0 is a local storage boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStoreDecl {
    pub name: Ident,
    pub root: String,
    pub read_globs: Vec<String>,
    pub write_globs: Vec<String>,
    /// Source spans of each clause keyword (`root` / the `allow` of read / write),
    /// so `whip fmt` can interleave own-line and trailing body comments by position
    /// (the body otherwise rebuilds from the AST, dropping comments).
    pub root_span: Option<SourceSpan>,
    pub read_span: Option<SourceSpan>,
    pub write_span: Option<SourceSpan>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowDecl {
    pub name: Ident,
    pub tags: Vec<TagDecl>,
    pub description: Option<StringLiteral>,
    pub whens: Vec<WhenClause>,
    pub body: BlockSource,
    pub span: SourceSpan,
}

/// One typed parameter of an `action` template (DR-0023).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionParam {
    pub name: Ident,
    pub ty: TypeSyntax,
    pub span: SourceSpan,
}

/// `action <name>(<param: type>, …) { <effect chain> }` (DR-0023): a static,
/// hygienic, inline-expanded template over rule-body effect chains. Consumed by
/// `expand_action_calls` before lowering; never a runtime construct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionDecl {
    pub name: Ident,
    pub params: Vec<ActionParam>,
    pub body: BlockSource,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDecl {
    pub name: Ident,
    pub harness: Option<Ident>,
    pub fields: Vec<AgentField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentField {
    Provider(Ident),
    Profile(StringLiteral),
    Capacity(u32, SourceSpan),
    Skills(Vec<StringLiteral>, SourceSpan),
    Capabilities(Vec<StringLiteral>, SourceSpan),
    /// `tools [Foo, Bar]`: the workflows this agent may invoke as typed tools
    /// (DR-0025). Entries are workflow names resolved against the program/packages.
    Tools(Vec<Ident>, SourceSpan),
    Unknown {
        name: Ident,
        span: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub variants: Vec<EnumVariantDecl>,
    pub span: SourceSpan,
}

/// One enum variant: bare (`Accept`) or data-carrying with a brace body that
/// reuses the class field grammar (sum types, spec/sum-types.md).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumVariantDecl {
    pub name: Ident,
    pub fields: Vec<ClassField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDecl {
    pub name: Ident,
    pub fields: Vec<ClassField>,
    pub span: SourceSpan,
}

/// Coordination resources (spec/coordination.md): a closed family of shared,
/// workspace-scoped resources with typed keys, atomic branchable operations,
/// and mandatory bounds (`ttl`/`retain`/`cap`+`reset`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseDecl {
    pub name: Ident,
    pub key_type: Ident,
    pub slots: u32,
    pub ttl_seconds: u64,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerDecl {
    pub name: Ident,
    pub entry_schema: Ident,
    pub partition_field: Ident,
    pub retain_seconds: u64,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CounterDecl {
    pub name: Ident,
    pub key_type: Ident,
    pub cap: i64,
    pub reset: String,
    pub span: SourceSpan,
}

/// A typed external-signal declaration (`signal deploy.finished { ... }`):
/// the ingress manifest naming a dotted event and its payload schema
/// (spec/event-ingress.md).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDecl {
    /// Dotted lowercase signal name (`deploy.finished`).
    pub name: String,
    pub name_span: SourceSpan,
    pub fields: Vec<ClassField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassField {
    pub name: Ident,
    pub ty: TypeSyntax,
    /// `@key`: this field is the class's natural key (used for import per-row
    /// idempotency, spec/std-library/files.md). At most one per class in v0.
    pub is_key: bool,
    /// Family B (discriminant-string schemas): `<field> <Type> when <disc> == "<lit>"`
    /// — this field is present only when the literal-union discriminant field `disc`
    /// equals `lit`. `(discriminant field name, required literal)`.
    pub presence_condition: Option<(String, String)>,
    pub span: SourceSpan,
}

/// A top-level source declaration: `source <provider> as <name> { ... }` or
/// `source clock as <name> { ... }`. Lowers through the `source_declaration`
/// construct family to a `signal_source` (generic provider) or `clock_source`
/// (the `clock` provider) admission template (spec/std-time.md,
/// spec/construct-grammar.md). A source admits a durable signal fact; it never
/// fires a rule directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDecl {
    /// `as <name>` — the source instance name.
    pub name: Ident,
    /// The provider keyword (`clock`) or a generic provider identifier.
    pub provider: Ident,
    /// Recurrence/timezone/missed policy; `Some` only for the `clock` provider.
    pub clock: Option<ClockPolicy>,
    /// `observe as <binding>` — binds the provider observation schema.
    pub observe_binding: Ident,
    /// `emit <signal> { <field> <value> ... }` — maps the observation into the
    /// declared signal payload.
    pub emit: SourceEmit,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockPolicy {
    pub recurrence: Recurrence,
    pub timezone: Option<StringLiteral>,
    pub missed: Option<MissedPolicy>,
    pub span: SourceSpan,
}

/// Recurrence forms from spec/std-time.md (conservative first surface).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Recurrence {
    /// `at <hh:mm>` — a single scheduled occurrence.
    At { time: TimeOfDay, span: SourceSpan },
    /// `every <duration>` — interval occurrences.
    EveryDuration {
        seconds: u64,
        source: String,
        span: SourceSpan,
    },
    /// `every <calendar-pattern> at <hh:mm>` — calendar occurrences.
    EveryCalendar {
        pattern: CalendarPattern,
        time: TimeOfDay,
        span: SourceSpan,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarPattern {
    Day,
    Weekday,
    Weekly(Weekday),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
    pub span: SourceSpan,
}

/// Missed-occurrence policy from spec/std-time.md. No silent default: a recurring
/// source must declare one (enforced by the checker).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissedPolicy {
    Skip,
    Coalesce,
    CatchUp { limit: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEmit {
    /// Dotted lowercase signal name materialized by this source.
    pub signal: String,
    pub signal_span: SourceSpan,
    pub fields: Vec<SourceEmitField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEmitField {
    pub name: Ident,
    pub value: SourceValue,
    pub span: SourceSpan,
}

/// A value mapped into an emitted signal field: an observation path
/// (`tick.scheduled_at`) or a literal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceValue {
    Path {
        binding: Ident,
        segments: Vec<Ident>,
        span: SourceSpan,
    },
    String(StringLiteral),
    Number(String, SourceSpan),
}

/// A deterministic test scenario (spec/workflow-testing.md). Validated by
/// `whip check`; excluded from compile/run IR; executed by `whip test`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestDecl {
    pub name: StringLiteral,
    /// Optional `workflow <Name>` header binding the scenario to one workflow
    /// in a multi-workflow bundle (spec/workflow-testing.md). Single-workflow
    /// files may omit it and bind implicitly.
    pub workflow: Option<Ident>,
    pub clauses: Vec<TestClause>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TestClause {
    Given(GivenClause),
    Stub(StubClause),
    Run(RunClause),
    Expect(ExpectClause),
}

/// A `<field> <expr>` mapping inside a `given` record body. `value` is the source
/// text of the expression (parsed via `parse_expression` when validated), matching
/// how guards and assertions capture expressions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestField {
    pub name: Ident,
    pub value: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GivenClause {
    Input {
        fields: Vec<TestField>,
        span: SourceSpan,
    },
    Fact {
        ty: Ident,
        fields: Vec<TestField>,
        span: SourceSpan,
    },
    Signal {
        name: String,
        fields: Vec<TestField>,
        span: SourceSpan,
    },
    Clock {
        at: StringLiteral,
        span: SourceSpan,
    },
    Tracker {
        tracker: String,
        fields: Vec<TestField>,
        span: SourceSpan,
    },
    /// `given file <store> at <path> "<content>"` seeds a fixture file in the
    /// named `file store` so a `read` during `whip test` resolves deterministic
    /// content (the harness redirects the store root to a temp dir).
    File {
        store: String,
        path: StringLiteral,
        content: StringLiteral,
        span: SourceSpan,
    },
}

/// `stub <surface…> <outcome> [record | string]`. The surface path and outcome
/// are kept as tokens; provider-specific validation happens in the harness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StubClause {
    /// Surface path segments (each may be dotted, e.g. `script.run`); the trailing
    /// segment is the outcome.
    pub surface: Vec<String>,
    pub outcome: String,
    pub payload: Option<StubPayload>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StubPayload {
    Record(Vec<TestField>),
    Message(StringLiteral),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunClause {
    pub kind: RunKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunKind {
    UntilIdle,
    UntilWorkflowCompleted,
    UntilWorkflowFailed,
    ForSteps(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectClause {
    pub target: ExpectTarget,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpectTarget {
    WorkflowCompleted,
    WorkflowFailed { failure: Option<Ident> },
    Rule { name: Ident, status: RuleStatus },
    Effect { name: String, status: EffectStatus },
    Diagnostic { code: String },
    NoEffect { name: String },
    Projection(ProjQuery),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleStatus {
    Fired,
    FiredTimes(u32),
    DidNotFire,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectStatus {
    Requested,
    Completed,
    Failed,
}

/// A projection query: `<noun> exists | count <predicate> is <N> | where <predicate>`.
/// The predicate reuses the guard expression kernel, restricted to projection
/// fields. The noun is a dotted fact name, so a scenario can assert over runtime
/// facts such as `agent.turn.completed` as well as single-identifier user facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjQuery {
    pub noun: String,
    pub kind: ProjQueryKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjQueryKind {
    Exists,
    Count { predicate: String, count: u32 },
    Where { predicate: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDecl {
    pub name: Ident,
    pub tags: Vec<TagDecl>,
    pub description: Option<StringLiteral>,
    pub schema: Ident,
    pub rows: Vec<TableRow>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableRow {
    pub body: BlockSource,
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
    pub tags: Vec<TagDecl>,
    pub description: Option<StringLiteral>,
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
    /// Non-fatal diagnostics (deprecations, style); never block compilation.
    pub warnings: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatOutput {
    pub formatted: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrProgram {
    pub workflow: String,
    pub source_tags: Vec<IrSourceTag>,
    pub source_descriptions: Vec<IrSourceDescription>,
    pub includes: Vec<IrInclude>,
    pub pattern_applications: Vec<IrPatternApplication>,
    pub workflow_contracts: Vec<IrWorkflowContract>,
    pub uses: Vec<IrUse>,
    pub harnesses: Vec<IrHarness>,
    pub queues: Vec<IrQueue>,
    pub channels: Vec<IrChannel>,
    pub file_stores: Vec<IrFileStore>,
    pub events: Vec<IrEvent>,
    pub sources: Vec<IrSource>,
    pub tests: Vec<IrTest>,
    pub leases: Vec<IrLease>,
    pub ledgers: Vec<IrLedger>,
    pub counters: Vec<IrCounter>,
    pub schemas: Vec<IrSchema>,
    pub agents: Vec<IrAgent>,
    pub coerces: Vec<IrCoerce>,
    pub assertions: Vec<IrAssertion>,
    pub rules: Vec<IrRule>,
    pub rule_dependencies: Vec<IrRuleDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrSourceTag {
    pub name: String,
    pub target_kind: String,
    pub target: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrSourceDescription {
    pub value: String,
    pub target_kind: String,
    pub target: String,
    pub span: SourceSpan,
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
    pub span: SourceSpan,
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
    Package,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrQueue {
    pub name: String,
    pub tracker: String,
    pub span: SourceSpan,
}

/// A lowered `channel` declaration (std.messaging): the channel identity, its
/// provider, and optional workspace/destination config. Lowering class is
/// `metadata_only`; the runtime messaging provider consumes it later.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrChannel {
    pub name: String,
    pub provider: String,
    pub workspace: Option<String>,
    pub destination: Option<String>,
    pub span: SourceSpan,
}

/// A lowered `file store` declaration (std.files): the store identity + its
/// literal local root directory, consumed by the runtime file provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrFileStore {
    pub name: String,
    pub root: String,
    /// Path globs (relative to `root`) a `read` may touch; empty = any path
    /// inside the root. Enforced at runtime in addition to root-containment.
    pub read_globs: Vec<String>,
    /// Path globs a `write` may touch; empty = any path inside the root.
    pub write_globs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrHarness {
    pub name: String,
    pub kind: String,
    pub span: SourceSpan,
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
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrClass {
    pub name: String,
    pub fields: Vec<IrClassField>,
    pub span: SourceSpan,
}

/// A declared external event: the typed ingress manifest
/// (spec/event-ingress.md). Dotted name, class-shaped payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEvent {
    pub name: String,
    pub fields: Vec<IrClassField>,
    pub span: SourceSpan,
}

/// A lowered source declaration (spec/std-time.md). `is_clock` selects the
/// `clock_source` lowering; otherwise `signal_source`. Both lower through the
/// `source_declaration` construct family and admit a durable signal fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrSource {
    pub name: String,
    pub provider: String,
    pub is_clock: bool,
    pub recurrence: Option<Recurrence>,
    pub timezone: Option<String>,
    pub missed: Option<MissedPolicy>,
    pub observe_binding: String,
    pub emit_signal: String,
    pub emit_fields: Vec<IrSourceEmitField>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrSourceEmitField {
    pub name: String,
    pub value: SourceValue,
    pub span: SourceSpan,
}

/// A lowered test scenario (spec/workflow-testing.md). Tests are excluded from
/// the executable IR (`compile`/`run` ignore them); `whip check` validates them
/// and `whip test` runs them. The clause detail is retained for the harness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrTest {
    pub name: String,
    pub workflow: Option<String>,
    pub clauses: Vec<TestClause>,
    pub span: SourceSpan,
}

/// Coordination resources (spec/coordination.md), lowered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrLease {
    pub name: String,
    pub key_type: String,
    pub slots: u32,
    pub ttl_seconds: u64,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrLedger {
    pub name: String,
    pub entry_schema: String,
    pub partition_field: String,
    pub retain_seconds: u64,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrCounter {
    pub name: String,
    pub key_type: String,
    pub cap: i64,
    pub reset: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrClassField {
    pub name: String,
    pub ty: IrType,
    /// `@key`: this field is the class's natural key (import per-row idempotency).
    pub is_key: bool,
    /// Family B presence condition: `(discriminant field name, required literal)`.
    /// When set, the field is present only when the discriminant equals the literal
    /// (spec/decision-records/discriminated-families-design.md §5.7).
    pub presence_condition: Option<(String, String)>,
    pub span: SourceSpan,
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
    pub harness: Option<String>,
    pub provider: Option<String>,
    pub profile: Option<String>,
    pub capacity: Option<u32>,
    pub skills: Vec<String>,
    pub capabilities: Vec<String>,
    /// Workflows this agent may invoke as typed tools (DR-0025 `tools [...]`).
    pub tools: Vec<String>,
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
    pub record_sources: Vec<IrRecordSource>,
    pub fact_consumes: Vec<String>,
    pub effects: Vec<IrEffectNode>,
    pub dependencies: Vec<IrEffectDependency>,
    pub case_branches: Vec<IrRuleCaseBranch>,
    pub terminal_outputs: Vec<IrTerminalOutput>,
    pub terminal_branches: Vec<IrTerminalCaseBranch>,
    /// The output bindings this rule `complete`s (the `name` of each `complete
    /// <binding> {…}` in the body, recursing into after/case/branch/handler blocks).
    /// Surfaced for the information-flow checker: a `complete result` returns a value
    /// to the workflow's invoker, an egress sink at the invoker boundary (DR-0030 X2).
    /// IFC-only — deliberately NOT rendered in the `.ir` snapshot, so it adds no
    /// golden/hash churn.
    pub terminal_completes: Vec<String>,
    /// The `redact <source> keep [..] as <out>` projections in this rule body
    /// (recursing into after/case/branch/handler blocks). Surfaced for the
    /// information-flow value-flow engine: a redaction is the explicit crossing at
    /// which the rule-level opaque join box is refined — the projected binding
    /// carries only the kept fields' labels (DR-0027, proven in
    /// models/lean/Whipple/Redaction.lean). IFC-only — NOT rendered in the `.ir`
    /// snapshot, so it adds no golden/hash churn.
    pub redactions: Vec<IrRedaction>,
    /// Per egress sink, the set of binding roots its payload references (union
    /// across branches), keyed by the sink string the IFC engine uses: a `complete
    /// <binding>` by its binding, a `record <Schema>` by `fact:<Schema>`, a `send via
    /// <channel>` by the channel. IFC-only (NOT in the `.ir` snapshot). The engine
    /// uses this to recognize a FULLY-REDACTED egress — one whose payload references
    /// only redaction outputs — and govern its leak check by the projection's
    /// per-field label rather than the rule's whole read set (DR-0027 redact, the
    /// static refinement).
    pub egress_payload_reads: BTreeMap<String, BTreeSet<String>>,
    /// Per `complete <binding>` egress, the binding roots each RESULT FIELD
    /// references — a two-level map `binding -> field -> {roots}`. Where
    /// `egress_payload_reads` joins all of a sink's fields into one set (enough for the
    /// fully-redacted recognizer), this keeps them SEPARATE so the IFC engine can
    /// compute a PER-FIELD flow signature (DR-0030 X2 v2): the reads reaching each
    /// result field, refined at fact granularity. IFC-only (NOT in the `.ir`
    /// snapshot). Union across branches; a `Shorthand` field resolves to the
    /// terminal's `from` binding.
    pub complete_field_reads: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    /// Bounded-type projection egresses (`record <T> from <src>`): each is governed
    /// by the kept fields' per-field label join, like an explicit `redact`. IFC-only
    /// (NOT in the `.ir` snapshot). DR-0027 auto-redaction, the bounded-type reading.
    pub bounded_egresses: Vec<IrBoundedEgress>,
    /// Maximum nesting depth of `after` blocks in the rule body (0 = no `after`,
    /// 1 = a top-level `after`, 2 = an `after` inside an `after`, …). Surfaced for the
    /// `lint.deep_after_nesting` maintainability check.
    pub max_after_depth: usize,
}

/// A bounded-type projection egress (`record <T> from <src>`): the recorded fact
/// keeps exactly `T`'s fields, copied from `src`, so the egress carries only the
/// kept fields' per-field labels — the "bounded-type" auto-redaction reading
/// (DR-0027). The bound is the declared target type `T`; the labels are the
/// SOURCE schema's (a target field mislabelled public is still caught against the
/// source's label). The IFC engine governs it exactly like an explicit `redact`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrBoundedEgress {
    /// The engine's sink string (`fact:<T>` for a record).
    pub sink: String,
    /// The schema of the `from` source binding, whose per-field labels bound the
    /// projection.
    pub source_schema: String,
    /// The kept field names (the target type `T`'s fields).
    pub keep: Vec<String>,
}

/// A `redact <source> keep [..] as <binding>` projection, surfaced for the
/// information-flow value-flow engine (DR-0027). `source` is the binding being
/// projected, `keep` the kept field names, `binding` the projected output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrRedaction {
    pub source: String,
    pub keep: Vec<String>,
    pub binding: String,
    /// The schema of the source binding, when resolvable (a matched class, a
    /// coerce/decide/exec result, an `after … as` alias, or an earlier redaction's
    /// output). The information-flow engine derives the projection's confidentiality
    /// from the kept fields of this schema (`<schema>.<field>` labels), so a redacted
    /// egress needs only the kept fields' clearance, not the whole record's. `None`
    /// when the source type is not statically known (the engine then stays
    /// conservative for that redaction).
    pub source_schema: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrRecordSource {
    pub schema: String,
    pub construct: String,
    pub span: SourceSpan,
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
    pub construct_use: Option<IrConstructUse>,
    pub idempotency_key: String,
    pub span: SourceSpan,
    /// Creation-anchored deadline from a `timeout <duration>` clause.
    pub timeout_seconds: Option<u64>,
    /// Turn-access grants (`with access to …`) lowered onto an `agent.tell` effect as
    /// authority-narrowing metadata (Proposal A). Empty for non-grant effects.
    pub access_grants: Vec<IrAccessGrant>,
    /// The named resource (file store / channel) a direct effect touches, if any —
    /// e.g. the store of a `read`/`write`. Surfaced so information-flow analysis can
    /// see rule-body data flows, not just turn-access grants. `None` for effects
    /// that touch no named resource. Not part of the `.ir` snapshot.
    pub resource: Option<String>,
    /// The agent a `tell` addresses (its `target`), surfaced so information-flow
    /// analysis can model the turn's egress to that agent's provider. `None` for
    /// non-`tell` effects. Not part of the `.ir` snapshot.
    pub agent: Option<String>,
    /// The `endorsed` source marker (DR-0027 I-IFC3): the author declared this effect
    /// (a `coerce`) an integrity-raising crossing. Surfaced so the trusted surface is
    /// visible at the source crossing point. Not part of the `.ir` snapshot.
    pub endorsed: bool,
    /// The `declassified` source marker (DR-0027 I-IFC3): the author declared this
    /// `coerce` a confidentiality-lowering crossing (its output schema bounds the
    /// leak). Surfaced for audit. Not part of the `.ir` snapshot.
    pub declassified: bool,
    /// The innermost `case <scrutinee> { <pattern> => … }` arm this effect sits in,
    /// as `(scrutinee, pattern)` — the discriminated-families *selector*. Lets the
    /// IFC checker apply NMIF-on-the-selector: a crossing (`endorsed`/`declassified`)
    /// selected by a low-integrity discriminant is rejected (DR §5.6 / §7.4). `None`
    /// for effects outside any `case`. Not part of the `.ir` snapshot.
    pub selected_by: Option<(String, String)>,
}

/// A lowered turn-access grant: the granted operations narrow the turn's effective
/// authority on `resource` (modeled in `models/maude/turn-access-grant.maude`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrAccessGrant {
    pub resource: String,
    pub operations: Vec<IrAccessGrantOp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrAccessGrantOp {
    pub operation: String,
    pub target: Option<String>,
    pub globs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrConstructUse {
    pub keyword: String,
    pub scope: String,
    pub construct_family: String,
    pub lowering_target: String,
    pub target_capability: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrEffectKind {
    AgentTell,
    Coerce,
    LoftClaim,
    HumanAsk,
    CapabilityCall,
    EventEmit,
    WorkflowInvoke,
    TimerWait,
    ExecCommand,
    QueueFile,
    QueueClaim,
    QueueRelease,
    QueueFinish,
    LeaseAcquire,
    LedgerAppend,
    CounterConsume,
    EventNotify,
    FileRead,
    FileWrite,
    FileImport,
    FileExport,
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
    TimedOut,
    Cancelled,
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
    /// Declared coordination resources (spec/coordination.md).
    leases: BTreeSet<String>,
    ledgers: BTreeSet<String>,
    counters: BTreeSet<String>,
    /// Declared `channel` names (std.messaging); `send via <channel>` must name one.
    channels: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct WorkflowInputSurface {
    inputs: BTreeMap<String, TypeSyntax>,
    schemas: SchemaIndex,
    /// Milestones the workflow may project (Family C): name -> payload class
    /// (empty string for a bare, payload-less milestone). Derived by scanning the
    /// workflow's rule bodies for `emit milestone "<name>" [of <Class>]`. This is
    /// the `declared(S)` set a parent's `after p reaches "<name>"` validates
    /// against (reject-undeclared) and the source of the observing binding's type.
    milestones: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct SchemaIndex {
    classes: BTreeMap<String, BTreeMap<String, TypeSyntax>>,
    enums: BTreeMap<String, BTreeSet<String>>,
    /// Declared external signals (spec/event-ingress.md); their payload
    /// schemas live in `classes` keyed by the dotted signal name.
    events: BTreeSet<String>,
    /// Family B: per-schema field presence conditions, `schema -> field ->
    /// (discriminant field, required literal)`. A conditioned field is readable
    /// only inside a matching `case <root>.<disc>` arm.
    presence: BTreeMap<String, BTreeMap<String, (String, String)>>,
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
    Add,
    Sub,
    Mul,
    Div,
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
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
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
            warnings: Vec::new(),
        };
    }

    // Program-level static check over ALL workflows (before root selection):
    // transitive runtime invocation cycles have no compile-time convergence proof
    // and are rejected (RESOLVED 2026-07-01). Direct self-invocation is caught
    // per-rule during lowering.
    let mut invoke_recursion_diagnostics = Vec::new();
    detect_workflow_invoke_recursion(&parsed.program, &mut invoke_recursion_diagnostics);
    if !invoke_recursion_diagnostics.is_empty() {
        return CompileOutput {
            ir: None,
            diagnostics: invoke_recursion_diagnostics,
            warnings: Vec::new(),
        };
    }

    let workflow_inputs = collect_workflow_input_surfaces(&parsed.program);

    // Whole-program validation (RESOLVED 2026-07-01): when a program declares
    // more than one explicit `workflow`, validate EVERY workflow — not only the
    // selected root — so a broken sibling is caught in a single compile
    // regardless of which `--root` is chosen. Each workflow is lowered against
    // its own scope (top-level globals + that workflow's local block items),
    // which is exactly the scoped program `select_root_workflow` builds for that
    // name. Root selection below still produces the single entry IR for
    // `dev`/`deploy`; this pass only adds validation coverage and never changes
    // the emitted IR (when it finds no errors it returns nothing, so the root is
    // lowered once more, cleanly, below). See models/maude/workflow-scoping.maude.
    if parsed.program.workflows.len() > 1 {
        // Names declared at the top level are global (shared across every
        // workflow); names declared inside a `workflow { ... }` block are private
        // to it. Map each workflow-local name to its owning workflow(s) so that
        // when a workflow references a name that is really a sibling's local, the
        // resulting unknown-name error can point the author at where it lives —
        // the "names do not leak into sibling workflows" guarantee, surfaced.
        let global_names: BTreeSet<String> = parsed
            .program
            .items
            .iter()
            .filter_map(|item| referenced_decl_name(item).map(|(name, _)| name))
            .collect();
        let mut sibling_locals: BTreeMap<String, Vec<(String, SourceSpan)>> = BTreeMap::new();
        for workflow in &parsed.program.workflows {
            for item in &workflow.items {
                if let Some((name, span)) = referenced_decl_name(item) {
                    sibling_locals
                        .entry(name)
                        .or_default()
                        .push((workflow.name.name.clone(), span));
                }
            }
        }

        let mut aggregated = Vec::new();
        for workflow in &parsed.program.workflows {
            let name = workflow.name.name.clone();
            let own_locals: BTreeSet<String> = workflow
                .items
                .iter()
                .filter_map(|item| referenced_decl_name(item).map(|(name, _)| name))
                .collect();
            let mut diagnostics = match select_root_workflow(parsed.program.clone(), Some(&name)) {
                Ok(scoped) => lower_program(scoped, workflow_inputs.clone()).diagnostics,
                Err(diagnostics) => diagnostics,
            };
            for diagnostic in &mut diagnostics {
                annotate_cross_workflow_leak(
                    diagnostic,
                    &name,
                    &own_locals,
                    &global_names,
                    &sibling_locals,
                );
            }
            aggregated.extend(diagnostics);
        }
        if !aggregated.is_empty() {
            return CompileOutput {
                ir: None,
                diagnostics: aggregated,
                warnings: Vec::new(),
            };
        }
    }

    match select_root_workflow(parsed.program, root) {
        Ok(program) => lower_program(program, workflow_inputs),
        Err(diagnostics) => CompileOutput {
            ir: None,
            diagnostics,
            warnings: Vec::new(),
        },
    }
}

/// One top-level declaration for an editor outline (`whip lsp`'s
/// `textDocument/documentSymbol`): its name, a coarse kind tag, and source span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclSymbol {
    pub name: String,
    pub kind: &'static str,
    pub span: SourceSpan,
}

/// Top-level declarations of `source` in source order, for an editor outline. On a
/// parse error it returns whatever declarations parsed (best-effort outline).
pub fn document_symbols(source: &str) -> Vec<DeclSymbol> {
    let program = parse_program(source).program;
    let mut symbols = Vec::new();
    if let Some(workflow) = &program.workflow {
        symbols.push(DeclSymbol {
            name: workflow.name.clone(),
            kind: "workflow",
            span: workflow.span,
        });
    }
    for workflow in &program.workflows {
        symbols.push(DeclSymbol {
            name: workflow.name.name.clone(),
            kind: "workflow",
            span: workflow.span,
        });
    }
    for pattern in &program.patterns {
        symbols.push(DeclSymbol {
            name: pattern.name.name.clone(),
            kind: "pattern",
            span: pattern.span,
        });
    }
    for item in &program.items {
        let symbol = match item {
            Item::Class(decl) => ("class", decl.name.name.clone(), decl.span),
            Item::Enum(decl) => ("enum", decl.name.name.clone(), decl.span),
            Item::Agent(decl) => ("agent", decl.name.name.clone(), decl.span),
            Item::Rule(decl) => ("rule", decl.name.name.clone(), decl.span),
            Item::Coerce(decl) => ("coerce", decl.name.name.clone(), decl.span),
            Item::Flow(decl) => ("flow", decl.name.name.clone(), decl.span),
            Item::Action(decl) => ("action", decl.name.name.clone(), decl.span),
            Item::Lease(decl) => ("lease", decl.name.name.clone(), decl.span),
            Item::Ledger(decl) => ("ledger", decl.name.name.clone(), decl.span),
            Item::Counter(decl) => ("counter", decl.name.name.clone(), decl.span),
            Item::Queue(decl) => ("queue", decl.name.name.clone(), decl.span),
            Item::Channel(decl) => ("channel", decl.name.name.clone(), decl.span),
            Item::FileStore(decl) => ("file store", decl.name.name.clone(), decl.span),
            Item::Event(decl) => ("signal", decl.name.clone(), decl.span),
            Item::Table(decl) => ("table", decl.name.name.clone(), decl.span),
            _ => continue,
        };
        symbols.push(DeclSymbol {
            name: symbol.1,
            kind: symbol.0,
            span: symbol.2,
        });
    }
    symbols
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

/// Format `source` while preserving comments where they can be placed safely:
/// top-level **leading** comments (a `# …` or `// …` line above a declaration, or
/// a file-header block) and **trailing** comments on a single-line top-level
/// declaration (`workflow Demo  # …`, attached to that element's line); comments
/// inside raw-body declarations (`rule`/`apply`/`coerce`/`table`/`flow`, carried by
/// the body substring); and comments inside `class`/`agent`/`enum` bodies, including a
/// data-carrying `enum` variant's nested field block — both own-line (interleaved
/// by source position) and trailing comments on a field/variant line (appended to
/// it), and `signal`/`queue`/`file store` bodies the same way — even though those
/// bodies rebuild from the AST. Returns `None` when the program does not parse, or
/// when a comment has nowhere to attach — e.g. one trailing a declaration's
/// opening-brace line, with no field on that line. The caller refuses such files
/// rather than dropping comments.
pub fn format_program_preserving_comments(source: &str) -> Option<String> {
    let parsed = parse_program(source);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let mut comments = lex_comments(source);
    if comments.is_empty() {
        return Some(format_syntax(parsed.program));
    }
    // Both the top-level interleave and the per-body interleave below assume
    // ascending source order.
    comments.sort_by_key(|comment| comment.span.start);
    let program = parsed.program;

    // Each top-level element as (source span, formatted chunk), in source order.
    let mut elements: Vec<(SourceSpan, String)> = Vec::new();
    if let Some(workflow) = program.workflow {
        let mut chunk = String::new();
        format_tags(&program.workflow_tags, &mut chunk);
        format_description(program.workflow_description.as_ref(), &mut chunk);
        push_line(&mut chunk, format!("workflow {}", workflow.name));
        elements.push((workflow.span, chunk));
    }
    for pattern in program.patterns {
        let span = pattern.span;
        let mut chunk = String::new();
        format_pattern(pattern, &mut chunk);
        elements.push((span, chunk));
    }
    for item in program.items {
        let span = item.span();
        let mut chunk = String::new();
        // Field-list bodies (`class`/`agent`/`enum`) rebuild from the AST, which
        // drops comments. Interleave their own-line body comments here; a body
        // comment that cannot be placed safely refuses the whole file (the
        // raw-body formatters — rule/coerce/table — already carry their comments).
        let placed = match &item {
            Item::Class(class_decl) => Some(try_format_class_with_comments(
                class_decl, source, &comments, &mut chunk,
            )),
            Item::Agent(agent) => Some(try_format_agent_with_comments(
                agent, source, &comments, &mut chunk,
            )),
            Item::Enum(enum_decl) => Some(try_format_enum_with_comments(
                enum_decl, source, &comments, &mut chunk,
            )),
            Item::Event(event) => Some(try_format_event_with_comments(
                event, source, &comments, &mut chunk,
            )),
            Item::Queue(queue) => Some(try_format_queue_with_comments(
                queue, source, &comments, &mut chunk,
            )),
            Item::FileStore(file_store) => Some(try_format_filestore_with_comments(
                file_store, source, &comments, &mut chunk,
            )),
            _ => None,
        };
        match placed {
            Some(true) => {}
            Some(false) => return None,
            None => format_item(item, &mut chunk),
        }
        elements.push((span, chunk));
    }
    for workflow in program.workflows {
        let span = workflow.span;
        let mut chunk = String::new();
        format_workflow(workflow, &mut chunk);
        elements.push((span, chunk));
    }
    elements.sort_by_key(|(span, _)| span.start);

    // Classify top-level comments. A comment INSIDE an element's span is preserved
    // by that element's body formatter — a raw `body.text` substring
    // (rule/coerce/table) or the per-body interleave above (class/agent/enum) — so
    // emitting it here too would duplicate it; skip it. Otherwise an own-line
    // comment is `leading` (interleaved between elements by position), and a
    // trailing comment (code before it) attaches to the element whose last source
    // line it shares — typically a single-line declaration (`workflow Demo  # x`).
    // A trailing comment with no such element has nowhere to attach, so the file is
    // refused rather than dropping it.
    let mut leading: Vec<&Comment> = Vec::new();
    let mut element_trailing: Vec<Option<&Comment>> = vec![None; elements.len()];
    for comment in &comments {
        let in_body = elements
            .iter()
            .any(|(span, _)| span.start < comment.span.start && comment.span.start < span.end);
        if in_body {
            continue;
        }
        let line_start = source[..comment.span.start]
            .rfind('\n')
            .map(|newline| newline + 1)
            .unwrap_or(0);
        if source[line_start..comment.span.start].trim().is_empty() {
            leading.push(comment);
            continue;
        }
        let comment_line = line_index(source, comment.span.start);
        let mut placed = false;
        for (index, (span, _)) in elements.iter().enumerate() {
            if line_index(source, span.end.saturating_sub(1)) == comment_line {
                if element_trailing[index].is_some() {
                    return None;
                }
                element_trailing[index] = Some(comment);
                placed = true;
                break;
            }
        }
        if !placed {
            return None;
        }
    }

    let mut out = String::new();
    let mut next_comment = 0;
    let element_count = elements.len();
    for (index, (span, chunk)) in elements.iter().enumerate() {
        while next_comment < leading.len() && leading[next_comment].span.start < span.start {
            push_line(&mut out, format_comment(leading[next_comment]));
            next_comment += 1;
        }
        match element_trailing[index] {
            Some(comment) => {
                out.push_str(chunk.strip_suffix('\n').unwrap_or(chunk));
                out.push_str(&format!("  {}\n", format_comment(comment)));
            }
            None => out.push_str(chunk),
        }
        if index + 1 < element_count {
            out.push('\n');
        }
    }
    if next_comment < leading.len() {
        if element_count > 0 {
            out.push('\n');
        }
        while next_comment < leading.len() {
            push_line(&mut out, format_comment(leading[next_comment]));
            next_comment += 1;
        }
    }

    // Safety net against silent data loss: in-body comments are left to each
    // element's body formatter, and some formatters rebuild from the AST (which
    // drops comments). The idempotency self-check can't catch a *consistent*
    // drop, so verify here that every source comment survives — refuse otherwise.
    if lex_comments(&out).len() != comments.len() {
        return None;
    }
    Some(out)
}

fn format_comment(comment: &Comment) -> String {
    let marker = match comment.marker {
        CommentMarker::Hash => "#",
        CommentMarker::Slash => "//",
    };
    let text = comment.text.trim();
    if text.is_empty() {
        marker.to_owned()
    } else {
        format!("{marker} {text}")
    }
}

/// Zero-based source line of a byte offset.
fn line_index(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
}

/// Classify the comments inside `body` (a field-list declaration's brace region)
/// against its `members` (each member's span + already-formatted lines, in source
/// order). Returns the own-line comments to interleave between members, plus a
/// per-member optional trailing comment (appended to that member's last line).
/// Returns `None` when a comment cannot be placed safely — a comment inside a
/// *multi-line* member's own body (a deeper level this pass does not place), or a
/// trailing comment with no single-line member on its line — so the caller refuses
/// the file rather than misplace it. `comments` must be sorted by `span.start`.
fn classify_body_comments<'a>(
    source: &str,
    body: SourceSpan,
    members: &[(SourceSpan, Vec<String>)],
    comments: &'a [Comment],
) -> Option<(Vec<&'a Comment>, Vec<Option<&'a Comment>>)> {
    let mut own_line: Vec<&Comment> = Vec::new();
    let mut trailing: Vec<Option<&Comment>> = vec![None; members.len()];
    for comment in comments {
        if comment.span.start <= body.start || comment.span.start >= body.end {
            continue;
        }
        // A comment inside a multi-line member's own braces is a deeper level we do
        // not place here (e.g. a data-carrying `enum` variant's nested field).
        if members.iter().any(|(span, lines)| {
            lines.len() > 1 && span.start < comment.span.start && comment.span.start < span.end
        }) {
            return None;
        }
        let line_start = source[..comment.span.start]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        if source[line_start..comment.span.start].trim().is_empty() {
            own_line.push(comment);
            continue;
        }
        // Trailing: attach to a single-line member sharing the comment's line.
        let comment_line = line_index(source, comment.span.start);
        let mut placed = false;
        for (index, (span, lines)) in members.iter().enumerate() {
            if lines.len() == 1 && line_index(source, span.start) == comment_line {
                if trailing[index].is_some() {
                    return None;
                }
                trailing[index] = Some(comment);
                placed = true;
                break;
            }
        }
        if !placed {
            return None;
        }
    }
    Some((own_line, trailing))
}

/// Emit each member's lines, interleaving `own_line` comments by source position
/// (at `indent`) and appending each member's `trailing` comment to its last line.
/// `members` and `own_line` must be in ascending `span.start` order; `trailing`
/// is parallel to `members`.
fn emit_members_with_comments(
    members: &[(SourceSpan, Vec<String>)],
    own_line: &[&Comment],
    trailing: &[Option<&Comment>],
    indent: &str,
    formatted: &mut String,
) {
    let mut next = 0;
    for (index, (span, lines)) in members.iter().enumerate() {
        while next < own_line.len() && own_line[next].span.start < span.start {
            push_line(
                formatted,
                format!("{indent}{}", format_comment(own_line[next])),
            );
            next += 1;
        }
        let last = lines.len().saturating_sub(1);
        for (offset, line) in lines.iter().enumerate() {
            match trailing[index] {
                Some(comment) if offset == last => {
                    push_line(formatted, format!("{line}  {}", format_comment(comment)));
                }
                _ => push_line(formatted, line.clone()),
            }
        }
    }
    while next < own_line.len() {
        push_line(
            formatted,
            format!("{indent}{}", format_comment(own_line[next])),
        );
        next += 1;
    }
}

/// Format a `class` body with its own-line and trailing comments preserved.
/// Returns `false` (caller refuses the file) when a body comment cannot be placed
/// safely.
fn try_format_class_with_comments(
    class_decl: &ClassDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let members: Vec<(SourceSpan, Vec<String>)> = class_decl
        .fields
        .iter()
        .map(|field| {
            let key = if field.is_key { " @key" } else { "" };
            (
                field.span,
                vec![format!(
                    "  {} {}{key}",
                    field.name.name,
                    field.ty.to_source()
                )],
            )
        })
        .collect();
    let Some((own_line, trailing)) =
        classify_body_comments(source, class_decl.span, &members, comments)
    else {
        return false;
    };
    push_line(formatted, format!("class {} {{", class_decl.name.name));
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

/// Format a `queue` body (its single `tracker` member) with own-line and trailing
/// comments preserved. Returns `false` (caller refuses the file) when a body
/// comment cannot be placed safely.
fn try_format_queue_with_comments(
    queue: &QueueDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let members: Vec<(SourceSpan, Vec<String>)> = vec![(
        queue.tracker.span,
        vec![format!("  tracker {}", queue.tracker.name)],
    )];
    let Some((own_line, trailing)) = classify_body_comments(source, queue.span, &members, comments)
    else {
        return false;
    };
    push_line(formatted, format!("queue {} {{", queue.name.name));
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

/// Format a `file store` body (its `root` and optional `allow read`/`allow write`
/// clauses) with own-line and trailing comments preserved, interleaved by the
/// clause spans captured during parsing. Returns `false` (caller refuses the file)
/// when a body comment cannot be placed safely.
fn try_format_filestore_with_comments(
    file_store: &FileStoreDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let render = |globs: &[String]| {
        globs
            .iter()
            .map(|glob| format!("{glob:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut members: Vec<(SourceSpan, Vec<String>)> = Vec::new();
    if let Some(span) = file_store.root_span {
        members.push((span, vec![format!("  root {:?}", file_store.root)]));
    }
    if !file_store.read_globs.is_empty() {
        if let Some(span) = file_store.read_span {
            members.push((
                span,
                vec![format!("  allow read [{}]", render(&file_store.read_globs))],
            ));
        }
    }
    if !file_store.write_globs.is_empty() {
        if let Some(span) = file_store.write_span {
            members.push((
                span,
                vec![format!(
                    "  allow write [{}]",
                    render(&file_store.write_globs)
                )],
            ));
        }
    }
    members.sort_by_key(|(span, _)| span.start);
    let Some((own_line, trailing)) =
        classify_body_comments(source, file_store.span, &members, comments)
    else {
        return false;
    };
    push_line(formatted, format!("file store {} {{", file_store.name.name));
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

/// Format a `signal` body (a typed payload schema of `ClassField`s, like a class)
/// with its own-line and trailing comments preserved. Returns `false` (caller
/// refuses the file) when a body comment cannot be placed safely.
fn try_format_event_with_comments(
    event: &EventDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let members: Vec<(SourceSpan, Vec<String>)> = event
        .fields
        .iter()
        .map(|field| {
            (
                field.span,
                vec![format!("  {} {}", field.name.name, field.ty.to_source())],
            )
        })
        .collect();
    let Some((own_line, trailing)) = classify_body_comments(source, event.span, &members, comments)
    else {
        return false;
    };
    push_line(formatted, format!("signal {} {{", event.name));
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

fn agent_field_span(field: &AgentField) -> SourceSpan {
    match field {
        AgentField::Provider(ident) => ident.span,
        AgentField::Profile(profile) => profile.span,
        AgentField::Capacity(_, span)
        | AgentField::Skills(_, span)
        | AgentField::Capabilities(_, span)
        | AgentField::Tools(_, span) => *span,
        AgentField::Unknown { span, .. } => *span,
    }
}

fn agent_field_line(field: &AgentField) -> String {
    match field {
        AgentField::Provider(provider) => format!("  provider {}", provider.name),
        AgentField::Profile(profile) => format!("  profile {:?}", profile.value),
        AgentField::Capacity(capacity, _) => format!("  capacity {capacity}"),
        AgentField::Skills(skills, _) => {
            let skills = skills
                .iter()
                .map(|skill| format!("{:?}", skill.value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("  skills [{skills}]")
        }
        AgentField::Capabilities(capabilities, _) => {
            let capabilities = capabilities
                .iter()
                .map(|capability| format!("{:?}", capability.value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("  capabilities [{capabilities}]")
        }
        AgentField::Tools(tools, _) => {
            let tools = tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("  tools [{tools}]")
        }
        AgentField::Unknown { name, .. } => format!("  {}", name.name),
    }
}

/// Format an `agent` body with its own-line and trailing comments preserved.
/// Returns `false` (caller refuses the file) when a body comment cannot be placed
/// safely.
fn try_format_agent_with_comments(
    agent: &AgentDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let members: Vec<(SourceSpan, Vec<String>)> = agent
        .fields
        .iter()
        .map(|field| (agent_field_span(field), vec![agent_field_line(field)]))
        .collect();
    let Some((own_line, trailing)) = classify_body_comments(source, agent.span, &members, comments)
    else {
        return false;
    };
    let harness = agent
        .harness
        .as_ref()
        .map(|harness| format!(" using {}", harness.name))
        .unwrap_or_default();
    push_line(
        formatted,
        format!("agent {}{} {{", agent.name.name, harness),
    );
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

/// Lines for one enum variant, with comments inside a data-carrying variant's
/// nested field block preserved (own-line interleaved, trailing appended) — the
/// block is a field list in braces, so it reuses the same classify/emit one level
/// deeper. Returns `None` when a nested comment cannot be placed safely.
fn enum_variant_lines_with_comments(
    variant: &EnumVariantDecl,
    source: &str,
    comments: &[Comment],
) -> Option<Vec<String>> {
    if variant.fields.is_empty() {
        return Some(vec![format!("  {}", variant.name.name)]);
    }
    let members: Vec<(SourceSpan, Vec<String>)> = variant
        .fields
        .iter()
        .map(|field| {
            (
                field.span,
                vec![format!("    {} {}", field.name.name, field.ty.to_source())],
            )
        })
        .collect();
    // `comments` is filtered to this variant's span by classify (via variant.span).
    let (own_line, trailing) = classify_body_comments(source, variant.span, &members, comments)?;
    let mut block = String::new();
    emit_members_with_comments(&members, &own_line, &trailing, "    ", &mut block);
    let mut lines = vec![format!("  {} {{", variant.name.name)];
    lines.extend(block.lines().map(str::to_owned));
    lines.push("  }".to_owned());
    Some(lines)
}

/// Format an `enum` body with its comments preserved at both levels: between
/// variants (own-line interleaved, trailing appended to a bare variant's line) and
/// inside a data-carrying variant's nested field block. Each brace-body filters
/// comments by its own span, so the two levels never double-count. Returns `false`
/// (caller refuses the file) when a comment cannot be placed safely.
fn try_format_enum_with_comments(
    enum_decl: &EnumDecl,
    source: &str,
    comments: &[Comment],
    formatted: &mut String,
) -> bool {
    let mut members: Vec<(SourceSpan, Vec<String>)> = Vec::with_capacity(enum_decl.variants.len());
    for variant in &enum_decl.variants {
        let Some(lines) = enum_variant_lines_with_comments(variant, source, comments) else {
            return false;
        };
        members.push((variant.span, lines));
    }
    // Enum-body-level comments are those NOT inside a data variant's nested block
    // (those are placed by `enum_variant_lines_with_comments`); pass only those to
    // the body-level classify so the nested ones are not counted twice.
    let body_level: Vec<Comment> = comments
        .iter()
        .filter(|comment| {
            !enum_decl.variants.iter().any(|variant| {
                !variant.fields.is_empty()
                    && variant.span.start < comment.span.start
                    && comment.span.start < variant.span.end
            })
        })
        .cloned()
        .collect();
    let Some((own_line, trailing)) =
        classify_body_comments(source, enum_decl.span, &members, &body_level)
    else {
        return false;
    };
    push_line(formatted, format!("enum {} {{", enum_decl.name.name));
    emit_members_with_comments(&members, &own_line, &trailing, "  ", formatted);
    push_line(formatted, "}");
    true
}

/// The name a top-level named declaration introduces, paired with its span, when
/// it is a kind that another workflow can reference by name (schemas, agents,
/// coordination resources, signals). Rules/tests/asserts/apply/contracts/patterns
/// introduce no such cross-referenced name here. Mirrors `document_symbols`'
/// named-decl set. Used to attach a "declared in workflow B" note when a
/// workflow references a name that is really private to a sibling.
fn referenced_decl_name(item: &Item) -> Option<(String, SourceSpan)> {
    match item {
        Item::Class(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Enum(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Agent(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Coerce(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Lease(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Ledger(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Counter(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Queue(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Channel(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::FileStore(decl) => Some((decl.name.name.clone(), decl.span)),
        Item::Event(decl) => Some((decl.name.clone(), decl.span)),
        Item::Table(decl) => Some((decl.name.name.clone(), decl.span)),
        _ => None,
    }
}

/// If `diagnostic` (produced while validating workflow `current`) reports an
/// unknown name that is actually declared *private to a sibling workflow*, attach
/// a related note pointing at that sibling's declaration. This turns a bare
/// "unknown class `X`" into an actionable "…and `X` lives in workflow `B`; move
/// it to the top level to share it." A name that is global or one of `current`'s
/// own locals is legitimately in scope and never annotated.
fn annotate_cross_workflow_leak(
    diagnostic: &mut Diagnostic,
    current: &str,
    own_locals: &BTreeSet<String>,
    global_names: &BTreeSet<String>,
    sibling_locals: &BTreeMap<String, Vec<(String, SourceSpan)>>,
) {
    for (name, owners) in sibling_locals {
        if global_names.contains(name) || own_locals.contains(name) {
            continue;
        }
        // Only names actually referenced (as `` `name` ``) in this diagnostic, and
        // owned by some workflow other than the one being validated.
        if !diagnostic.message.contains(&format!("`{name}`")) {
            continue;
        }
        let Some((owner, span)) = owners.iter().find(|(owner, _)| owner != current) else {
            continue;
        };
        diagnostic.related.push(RelatedInfo {
            span: *span,
            message: format!(
                "`{name}` is declared inside workflow `{owner}`, which makes it \
                 private to that workflow; move it to a top-level declaration to \
                 share it across workflows"
            ),
        });
        return;
    }
}

fn select_root_workflow(
    mut program: Program,
    root: Option<&str>,
) -> Result<Program, Vec<Diagnostic>> {
    // A runnable program requires at least one explicit `workflow`. The implicit
    // compatibility root is removed (RESOLVED 2026-07-01): a source that declares
    // no `workflow` at all (neither the header form nor a `workflow Name { ... }`
    // block) is a library fragment, not a program, and is rejected here rather
    // than silently compiled as an anonymous root.
    if program.workflow.is_none() && program.workflows.is_empty() {
        return Err(vec![Diagnostic {
            related: Vec::new(),
            span: SourceSpan { start: 0, end: 0 },
            message: "program declares no `workflow`".to_owned(),
            suggestion: Some(
                "add an explicit `workflow Name { ... }` declaration; a runnable \
                 program requires at least one workflow (files that only declare \
                 shared types or patterns are libraries, meant to be `include`d)"
                    .to_owned(),
            ),
        }]);
    }

    if program.workflows.is_empty() {
        if let Some(root) = root {
            match program.workflow.as_ref() {
                Some(workflow) if workflow.name == root => {}
                Some(workflow) => {
                    return Err(vec![Diagnostic {
                        related: Vec::new(),
                        span: workflow.span,
                        message: format!("root workflow `{root}` was not found"),
                        suggestion: Some(format!("available workflow: `{}`", workflow.name)),
                    }]);
                }
                None => {
                    return Err(vec![Diagnostic {
                        related: Vec::new(),
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
                    related: Vec::new(),
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
                related: Vec::new(),
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
    let workflow_tags = selected.tags;
    let workflow_description = selected.description;
    items.extend(selected.items);
    Ok(Program {
        workflow: Some(selected.name),
        workflow_tags,
        workflow_description,
        explicit_workflow_body: true,
        workflows: Vec::new(),
        patterns: program.patterns,
        items,
    })
}

impl IrProgram {
    pub fn construct_uses(&self) -> Vec<&IrConstructUse> {
        self.rules
            .iter()
            .flat_map(|rule| rule.metadata.effects.iter())
            .filter_map(|effect| effect.construct_use.as_ref())
            .collect()
    }

    pub fn contract_registry(&self) -> ContractRegistry {
        let mut libraries = BTreeMap::<String, LibraryRegistration>::new();
        let mut contracts = BTreeMap::<(String, String), EffectContract>::new();

        for use_decl in &self.uses {
            libraries
                .entry(use_decl.name.clone())
                .or_insert_with(|| LibraryRegistration {
                    id: use_decl.name.clone(),
                    version: "unlocked".to_owned(),
                    standard: false,
                });
        }

        if !self.harnesses.is_empty() || !self.agents.is_empty() {
            register_standard_library(&mut libraries, "std.agent");
        }
        if !self.queues.is_empty() {
            register_standard_library(&mut libraries, "std.tracker");
        }
        if !self.events.is_empty() {
            register_standard_library(&mut libraries, "std.ingress");
        }
        if !self.leases.is_empty() || !self.ledgers.is_empty() || !self.counters.is_empty() {
            register_standard_library(&mut libraries, "std.coord");
        }
        if !self.channels.is_empty() {
            register_standard_library(&mut libraries, "std.messaging");
        }
        if !self.coerces.is_empty() {
            register_standard_library(&mut libraries, "std.coerce");
            register_effect_contract(
                &mut libraries,
                &mut contracts,
                IrEffectKind::Coerce,
                Vec::new(),
            );
        }

        for rule in &self.rules {
            for effect in &rule.metadata.effects {
                register_effect_contract(
                    &mut libraries,
                    &mut contracts,
                    effect.kind.clone(),
                    effect.required_capabilities.clone(),
                );
            }
        }

        // Built-in standard-library construct registrations. Unlike third-party
        // packages (whose constructs come from a package manifest + lock), these
        // are compiled into the platform, so they are available without a lock and
        // are EXEMPT from the package-lock requirement (1929 OPTION A). Registered
        // only when actually used, so channel-only programs and registry-shape
        // tests are unaffected. Modeled in
        // `models/maude/std-construct-authorization.maude`.
        let mut constructs = Vec::new();
        if self
            .construct_uses()
            .iter()
            .any(|use_form| use_form.keyword == "send")
        {
            register_standard_library(&mut libraries, "std.messaging");
            constructs.push(builtin_messaging_send_construct());
            contracts
                .entry((MESSAGING_SEND_CAPABILITY.to_owned(), "v0".to_owned()))
                .or_insert_with(builtin_messaging_send_effect_contract);
        }

        ContractRegistry {
            libraries: libraries.into_values().collect(),
            constructs,
            effect_contracts: contracts.into_values().collect(),
        }
    }

    pub fn to_snapshot(&self) -> String {
        let mut snapshot = String::new();
        push_line(&mut snapshot, format!("workflow {}", self.workflow));

        if !self.source_tags.is_empty() {
            push_line(&mut snapshot, "source_tags");
            for tag in &self.source_tags {
                push_line(
                    &mut snapshot,
                    format!("@{} {} {}", tag.name, tag.target_kind, tag.target),
                );
            }
        }

        if !self.source_descriptions.is_empty() {
            push_line(&mut snapshot, "source_descriptions");
            for description in &self.source_descriptions {
                push_line(
                    &mut snapshot,
                    format!(
                        "{:?} {} {}",
                        description.value, description.target_kind, description.target
                    ),
                );
            }
        }

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
                            // `@key` is serialized only when set, so non-keyed
                            // classes keep their prior snapshot (no ripple).
                            let key = if field.is_key { " @key" } else { "" };
                            push_line(
                                &mut snapshot,
                                format!("    {} {}{key}", field.name, field.ty.to_snapshot()),
                            );
                        }
                    }
                }
            }
        }

        if !self.harnesses.is_empty() {
            push_line(&mut snapshot, "harnesses");
            for harness in &self.harnesses {
                push_line(
                    &mut snapshot,
                    format!("  harness {} kind={}", harness.name, harness.kind),
                );
            }
        }
        if !self.queues.is_empty() {
            push_line(&mut snapshot, "queues");
            for queue in &self.queues {
                push_line(
                    &mut snapshot,
                    format!("  queue {} tracker={}", queue.name, queue.tracker),
                );
            }
        }

        if !self.channels.is_empty() {
            push_line(&mut snapshot, "channels");
            for channel in &self.channels {
                let mut line = format!("  channel {} provider={}", channel.name, channel.provider);
                if let Some(workspace) = &channel.workspace {
                    line.push_str(&format!(" workspace={workspace}"));
                }
                if let Some(destination) = &channel.destination {
                    line.push_str(&format!(" destination={destination:?}"));
                }
                push_line(&mut snapshot, line);
            }
        }

        if !self.file_stores.is_empty() {
            push_line(&mut snapshot, "file_stores");
            for file_store in &self.file_stores {
                push_line(
                    &mut snapshot,
                    format!(
                        "  file store {} root={:?}",
                        file_store.name, file_store.root
                    ),
                );
                // Globs are serialized only when present, so stores without an
                // `allow` clause keep their prior snapshot (no ripple).
                if !file_store.read_globs.is_empty() {
                    push_line(
                        &mut snapshot,
                        format!("    allow read {:?}", file_store.read_globs),
                    );
                }
                if !file_store.write_globs.is_empty() {
                    push_line(
                        &mut snapshot,
                        format!("    allow write {:?}", file_store.write_globs),
                    );
                }
            }
        }

        if !self.agents.is_empty() {
            push_line(&mut snapshot, "agents");
            for agent in &self.agents {
                let profile = agent.profile.as_deref().unwrap_or("<missing>");
                let harness = agent.harness.as_deref().unwrap_or("<fallback>");
                let provider = agent.provider.as_deref().unwrap_or("<fallback>");
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
                let tools = if agent.tools.is_empty() {
                    "[]".to_owned()
                } else {
                    format!("[{}]", agent.tools.join(", "))
                };
                push_line(
                    &mut snapshot,
                    format!(
                        "  agent {} harness={} provider={} profile={} capacity={} skills={} capabilities={} tools={}",
                        agent.name, harness, provider, profile, capacity, skills, capabilities, tools
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
                if !rule.metadata.record_sources.is_empty() {
                    push_line(&mut snapshot, "    record_sources");
                    for source in &rule.metadata.record_sources {
                        push_line(
                            &mut snapshot,
                            format!(
                                "      schema:{} construct={} span={}..{}",
                                source.schema, source.construct, source.span.start, source.span.end
                            ),
                        );
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
                        let construct = effect
                            .construct_use
                            .as_ref()
                            .map(|form| {
                                format!(" construct={}->{}", form.keyword, form.target_capability)
                            })
                            .unwrap_or_default();
                        // Turn-access grants are appended only when present, so
                        // grant-free effects keep their existing snapshot shape.
                        let grants = if effect.access_grants.is_empty() {
                            String::new()
                        } else {
                            let rendered = effect
                                .access_grants
                                .iter()
                                .map(|grant| {
                                    let ops = grant
                                        .operations
                                        .iter()
                                        .map(|op| op.operation.as_str())
                                        .collect::<Vec<_>>()
                                        .join(",");
                                    format!("{}[{ops}]", grant.resource)
                                })
                                .collect::<Vec<_>>()
                                .join(";");
                            format!(" grants={rendered}")
                        };
                        push_line(
                            &mut snapshot,
                            format!(
                                "      {} kind={} binding={}{} key={}{}",
                                effect.id,
                                effect.kind.as_str(),
                                binding,
                                construct,
                                effect.idempotency_key,
                                grants
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

/// The `std.messaging` outbound `send` capability id (the target of the `send`
/// construct's `capability.call` lowering).
const MESSAGING_SEND_CAPABILITY: &str = "messaging.send";

/// Built-in `std.messaging` `send` construct registration (1929 OPTION A): the
/// compiler-provided equivalent of a package-authored `capability_call`
/// construct, available without a package lock. Mirrors how a `recall`-style
/// construct would be declared in a package manifest, but owned by the platform.
fn builtin_messaging_send_construct() -> ConstructRegistration {
    ConstructRegistration {
        id: MESSAGING_SEND_CAPABILITY.to_owned(),
        library_id: "std.messaging".to_owned(),
        version: "v0".to_owned(),
        construct_family: CONSTRUCT_FAMILY_EFFECT_OPERATION.to_owned(),
        keyword: "send".to_owned(),
        scope: CONSTRUCT_SCOPE_RULE_BODY.to_owned(),
        fields: vec![
            ConstructField {
                name: "channel".to_owned(),
                kind: "identifier".to_owned(),
                required: true,
            },
            ConstructField {
                name: "text".to_owned(),
                kind: "expression".to_owned(),
                required: true,
            },
        ],
        requires: vec![ConstructInterface {
            kind: CONSTRUCT_INTERFACE_CAPABILITY.to_owned(),
            name: Some(MESSAGING_SEND_CAPABILITY.to_owned()),
            type_ref: None,
            phase: CONSTRUCT_INTERFACE_PHASE_COMPILE_RUNTIME.to_owned(),
            cardinality: CONSTRUCT_INTERFACE_CARDINALITY_EXACTLY_ONE.to_owned(),
        }],
        provides: Vec::new(),
        lowering_target: CONSTRUCT_LOWERING_CAPABILITY_CALL.to_owned(),
        target_capability: Some(MESSAGING_SEND_CAPABILITY.to_owned()),
    }
}

/// Built-in `std.messaging` `messaging.send` `capability.call` effect contract,
/// the target the `send` construct lowers to. Present without a lock so
/// `validate_construct_uses` resolves the std-library exemption.
fn builtin_messaging_send_effect_contract() -> EffectContract {
    EffectContract {
        id: MESSAGING_SEND_CAPABILITY.to_owned(),
        library_id: "std.messaging".to_owned(),
        version: "v0".to_owned(),
        effect_kind: "capability.call".to_owned(),
        source_forms: vec!["send".to_owned()],
        input_schema: Some("messaging.send.input".to_owned()),
        output_schema: Some("MessageSendReceipt".to_owned()),
        required_capabilities: vec![MESSAGING_SEND_CAPABILITY.to_owned()],
        provider_kinds: vec!["messaging".to_owned()],
        projected_facts: vec!["effect.output".to_owned()],
        validation: TypedOutputValidation::RuntimeBoundary,
    }
}

fn register_standard_library(libraries: &mut BTreeMap<String, LibraryRegistration>, id: &str) {
    libraries
        .entry(id.to_owned())
        .or_insert_with(|| LibraryRegistration {
            id: id.to_owned(),
            version: "v0".to_owned(),
            standard: true,
        });
}

fn register_effect_contract(
    libraries: &mut BTreeMap<String, LibraryRegistration>,
    contracts: &mut BTreeMap<(String, String), EffectContract>,
    kind: IrEffectKind,
    required_capabilities: Vec<String>,
) {
    let contract = effect_contract_for_kind(kind, required_capabilities);
    register_standard_library(libraries, contract.library_id.as_str());
    contracts
        .entry((contract.id.clone(), contract.version.clone()))
        .and_modify(|existing| {
            merge_unique(
                &mut existing.required_capabilities,
                &contract.required_capabilities,
            );
            merge_unique(&mut existing.provider_kinds, &contract.provider_kinds);
            merge_unique(&mut existing.source_forms, &contract.source_forms);
            merge_unique(&mut existing.projected_facts, &contract.projected_facts);
        })
        .or_insert(contract);
}

fn merge_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
    target.sort();
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn effect_contract_for_kind(
    kind: IrEffectKind,
    required_capabilities: Vec<String>,
) -> EffectContract {
    let mut required_capabilities = required_capabilities;
    required_capabilities.sort();
    required_capabilities.dedup();
    let effect_kind = kind.as_str().to_owned();

    let (
        library_id,
        source_forms,
        input_schema,
        output_schema,
        default_capabilities,
        provider_kinds,
        projected_facts,
        validation,
    ) = match kind {
        IrEffectKind::AgentTell => (
            "std.agent",
            strings(&["tell"]),
            Some("agent.turn.request"),
            Some("AgentTurn"),
            strings(&["agent.turn"]),
            strings(&["agent"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::Coerce => (
            "std.coerce",
            strings(&["coerce", "decide"]),
            Some("coerce.input"),
            Some("typed-provider-output"),
            strings(&["model.invoke"]),
            strings(&["model"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::LoftClaim => (
            "std.tracker",
            strings(&["claim with"]),
            Some("tracker.claim.input"),
            Some("LoftClaim"),
            Vec::new(),
            strings(&["tracker"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::HumanAsk => (
            "std.human",
            strings(&["askHuman"]),
            Some("human.ask.input"),
            Some("HumanAnswer"),
            strings(&["human.ask"]),
            strings(&["human"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::CapabilityCall => (
            "std.exec",
            strings(&["call"]),
            Some("capability.call.input"),
            Some("capability.call.output"),
            Vec::new(),
            strings(&["capability"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::EventEmit => (
            "std.ingress",
            strings(&["emit"]),
            Some("event.emit.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::WorkflowInvoke => (
            "std.workflow",
            strings(&["invoke"]),
            Some("workflow.invoke.input"),
            Some("workflow.terminal"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::TimerWait => (
            "std.schedule",
            strings(&["timer"]),
            Some("timer.wait.input"),
            Some("TimerElapsed"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::None,
        ),
        IrEffectKind::ExecCommand => (
            "std.exec",
            strings(&["exec"]),
            Some("exec.command.input"),
            Some("exec.command.output"),
            strings(&["exec.run"]),
            strings(&["script", "command"]),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::QueueFile => (
            "std.tracker",
            strings(&["file"]),
            Some("queue.file.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::QueueClaim => (
            "std.tracker",
            strings(&["claim"]),
            Some("queue.claim.input"),
            Some("QueueClaim"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::None,
        ),
        IrEffectKind::QueueRelease => (
            "std.tracker",
            strings(&["release"]),
            Some("queue.release.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::QueueFinish => (
            "std.tracker",
            strings(&["finish"]),
            Some("queue.finish.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::LeaseAcquire => (
            "std.coord",
            strings(&["acquire"]),
            Some("lease.acquire.input"),
            Some("LeaseAcquireOutcome"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::None,
        ),
        IrEffectKind::LedgerAppend => (
            "std.coord",
            strings(&["append"]),
            Some("ledger.append.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::CounterConsume => (
            "std.coord",
            strings(&["consume"]),
            Some("counter.consume.input"),
            Some("CounterConsumeOutcome"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::None,
        ),
        IrEffectKind::EventNotify => (
            "std.ingress",
            strings(&["emit", "signal"]),
            Some("event.notify.input"),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TypedOutputValidation::None,
        ),
        IrEffectKind::FileRead => (
            "std.files",
            strings(&["read"]),
            Some("file.read.input"),
            Some("FileReadResult"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::FileWrite => (
            "std.files",
            strings(&["write"]),
            Some("file.write.input"),
            Some("FileWriteResult"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::FileImport => (
            "std.files",
            strings(&["import"]),
            Some("file.import.input"),
            Some("FileImportResult"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
        IrEffectKind::FileExport => (
            "std.files",
            strings(&["export"]),
            Some("file.export.input"),
            Some("FileExportResult"),
            Vec::new(),
            Vec::new(),
            strings(&["effect.output"]),
            TypedOutputValidation::RuntimeBoundary,
        ),
    };

    merge_unique(&mut required_capabilities, &default_capabilities);

    EffectContract {
        id: effect_kind.clone(),
        library_id: library_id.to_owned(),
        version: "v0".to_owned(),
        effect_kind,
        source_forms,
        input_schema: input_schema.map(str::to_owned),
        output_schema: output_schema.map(str::to_owned),
        required_capabilities,
        provider_kinds,
        projected_facts,
        validation,
    }
}

impl IrEffectKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AgentTell => "agent.tell",
            Self::Coerce => "coerce",
            Self::LoftClaim => "loft.claim",
            Self::HumanAsk => "human.ask",
            Self::CapabilityCall => "capability.call",
            Self::EventEmit => "event.emit",
            Self::WorkflowInvoke => "workflow.invoke",
            Self::TimerWait => "timer.wait",
            Self::ExecCommand => "exec.command",
            Self::QueueFile => "queue.file",
            Self::QueueClaim => "queue.claim",
            Self::QueueRelease => "queue.release",
            Self::QueueFinish => "queue.finish",
            Self::LeaseAcquire => "lease.acquire",
            Self::LedgerAppend => "ledger.append",
            Self::CounterConsume => "counter.consume",
            Self::EventNotify => "event.notify",
            Self::FileRead => "file.read",
            Self::FileWrite => "file.write",
            Self::FileImport => "file.import",
            Self::FileExport => "file.export",
        }
    }
}

impl DependencyPredicate {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeds => "succeeds",
            Self::Fails => "fails",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Completes => "completes",
        }
    }
}

impl IrUseKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Package => "package",
        }
    }
}

impl IrType {
    /// A human-readable label for this type (e.g. `ref<TicketRequest>`), for use in
    /// diagnostics such as workflow-input errors.
    pub fn display_label(&self) -> String {
        self.to_snapshot()
    }

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
    let mut warnings = Vec::new();
    let (program, pattern_applications) = expand_pattern_applications(program, &mut diagnostics);
    let program = {
        let mut program = program;
        // DR-0023: collect `action` templates, then expand their calls inside
        // every rule body — including rules generated by flow expansion below.
        // Calls are rewritten into ordinary effect-chain text that re-enters the
        // normal lowering pipeline; the `action` declarations themselves are
        // consumed (never a runtime construct).
        let actions: Vec<ActionDecl> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Action(action) => Some(action.clone()),
                _ => None,
            })
            .collect();
        let mut expanded = Vec::with_capacity(program.items.len());
        for item in program.items {
            match item {
                Item::Flow(flow) => expanded.extend(flow_expand::expand_flow(
                    flow,
                    &mut diagnostics,
                    &mut warnings,
                )),
                Item::Action(_) => {}
                other => expanded.push(other),
            }
        }
        action_expand::expand_action_calls(&mut expanded, &actions, &mut diagnostics);
        program.items = expanded;
        program
    };
    let schema_names = collect_schema_names(&program, &mut diagnostics);
    let harness_names = collect_harness_names(&program, &mut diagnostics);
    let agent_names = collect_agent_names(&program, &mut diagnostics);
    let workflow_contract_names = collect_workflow_contract_names(&program, &mut diagnostics);
    let mut semantic = SemanticContext::from_program(&program, workflow_inputs);
    let workflow = match program.workflow {
        Some(workflow) => workflow.name,
        None => {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan { start: 0, end: 0 },
                message: "expected workflow declaration".to_owned(),
                suggestion: Some("add `workflow Name` before declarations".to_owned()),
            });
            "<missing>".to_owned()
        }
    };

    let mut ir = IrProgram {
        workflow,
        source_tags: Vec::new(),
        source_descriptions: Vec::new(),
        includes: Vec::new(),
        pattern_applications,
        workflow_contracts: Vec::new(),
        uses: Vec::new(),
        harnesses: Vec::new(),
        queues: Vec::new(),
        channels: Vec::new(),
        file_stores: Vec::new(),
        events: Vec::new(),
        sources: Vec::new(),
        tests: Vec::new(),
        leases: Vec::new(),
        ledgers: Vec::new(),
        counters: Vec::new(),
        schemas: Vec::new(),
        agents: Vec::new(),
        coerces: Vec::new(),
        assertions: Vec::new(),
        rules: Vec::new(),
        rule_dependencies: Vec::new(),
    };
    let workflow_tag_target = ir.workflow.clone();
    lower_source_tags(
        &program.workflow_tags,
        "workflow",
        &workflow_tag_target,
        &mut ir,
    );
    lower_source_description(
        program.workflow_description.as_ref(),
        "workflow",
        &workflow_tag_target,
        &mut ir,
    );

    // Inline `decide -> { … } as <binding>` synthesizes a hygienic
    // `decide.<rule>.<binding>` class so its anonymous result shape flows like a
    // named `coerce -> Schema`. Done before the rule loop so `analyze_rule` sees
    // the class in the semantic index when type-checking field/`case` access.
    collect_inline_decide_schemas(&program.items, &mut semantic, &mut ir);

    // `redact <source> keep [..] as <out>` synthesizes a hygienic
    // `redact.<rule>.<out>` class holding only the kept fields of the source
    // schema, so the projection cannot expose a dropped field. Run after the
    // decide synthesis so a redact whose source is a decide result resolves.
    collect_redact_schemas(&program.items, &mut semantic, &mut ir);

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
            // Flows are expanded into rules and classes before this loop;
            // reaching one here is unreachable by construction.
            Item::Flow(flow) => {
                let _ = flow;
            }
            // Actions are consumed before this loop (DR-0023 slice 1 drops them;
            // slice 2 expands their calls); reaching one here is unreachable.
            Item::Action(action) => {
                let _ = action;
            }
            Item::Pattern(pattern) => diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: pattern.span,
                message: format!(
                    "pattern `{}` is not allowed inside this declaration scope",
                    pattern.name.name
                ),
                suggestion: Some("declare patterns at source top level".to_owned()),
            }),
            Item::Apply(apply) => diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: apply.span,
                message: format!(
                    "pattern application `{}` was not expanded",
                    apply.alias.name
                ),
                suggestion: Some(
                    "ensure the applied pattern is declared at source top level".to_owned(),
                ),
            }),
            Item::Harness(harness) => lower_harness(harness, &mut ir, &mut diagnostics),
            Item::Queue(queue) => lower_queue(queue, &mut ir, &mut diagnostics),
            Item::Channel(channel) => lower_channel(channel, &mut ir, &mut diagnostics),
            // The `file store` declaration (capability-scoped store identity)
            // lowers to its name + literal root; the runtime file provider reads
            // `<root>/<path>` for `read` effects against this store.
            Item::FileStore(file_store) => {
                ir.file_stores.push(IrFileStore {
                    name: file_store.name.name,
                    root: file_store.root,
                    read_globs: file_store.read_globs,
                    write_globs: file_store.write_globs,
                });
            }
            Item::Agent(agent) => lower_agent(agent, &mut ir, &harness_names, &mut diagnostics),
            Item::Enum(enum_decl) => lower_enum(enum_decl, &mut ir, &mut diagnostics),
            Item::Event(event) => lower_event(event, &mut ir, &mut diagnostics),
            Item::Source(source) => lower_source(source, &mut ir, &mut diagnostics),
            Item::Test(test) => lower_test(test, &mut ir, &mut diagnostics),
            Item::Lease(lease) => {
                if !schema_names.contains(&lease.key_type.name) {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: lease.key_type.span,
                        message: format!(
                            "lease `{}` keys on undeclared type `{}`",
                            lease.name.name, lease.key_type.name
                        ),
                        suggestion: Some(
                            "key a lease on an entity class the workflow already models".to_owned(),
                        ),
                    });
                }
                ir.leases.push(IrLease {
                    name: lease.name.name,
                    key_type: lease.key_type.name,
                    slots: lease.slots.max(1),
                    ttl_seconds: lease.ttl_seconds,
                    span: lease.span,
                });
            }
            Item::Ledger(ledger) => {
                if !schema_names.contains(&ledger.entry_schema.name) {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: ledger.entry_schema.span,
                        message: format!(
                            "ledger `{}` records undeclared entry type `{}`",
                            ledger.name.name, ledger.entry_schema.name
                        ),
                        suggestion: Some("declare the entry class before the ledger".to_owned()),
                    });
                }
                ir.ledgers.push(IrLedger {
                    name: ledger.name.name,
                    entry_schema: ledger.entry_schema.name,
                    partition_field: ledger.partition_field.name,
                    retain_seconds: ledger.retain_seconds,
                    span: ledger.span,
                });
            }
            Item::Counter(counter) => {
                if !schema_names.contains(&counter.key_type.name) {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: counter.key_type.span,
                        message: format!(
                            "counter `{}` keys on undeclared type `{}`",
                            counter.name.name, counter.key_type.name
                        ),
                        suggestion: Some(
                            "key a counter on an entity class the workflow already models"
                                .to_owned(),
                        ),
                    });
                }
                ir.counters.push(IrCounter {
                    name: counter.name.name,
                    key_type: counter.key_type.name,
                    cap: counter.cap,
                    reset: counter.reset,
                    span: counter.span,
                });
            }
            Item::Class(class_decl) => lower_class(
                class_decl,
                &mut ir,
                &schema_names,
                &agent_names,
                &mut diagnostics,
            ),
            Item::Table(table) => {
                lower_source_tags(&table.tags, "table", &table.name.name, &mut ir);
                lower_source_description(
                    table.description.as_ref(),
                    "table",
                    &table.name.name,
                    &mut ir,
                );
                lower_table(
                    table,
                    &semantic,
                    &workflow_contract_names,
                    &mut ir,
                    &mut diagnostics,
                )
            }
            Item::Coerce(coerce) => lower_coerce(
                coerce,
                &mut ir,
                &schema_names,
                &agent_names,
                &mut diagnostics,
            ),
            Item::Assert(assertion) => {
                let assertion_target = stable_hash(&assertion.expr);
                lower_source_tags(&assertion.tags, "assertion", &assertion_target, &mut ir);
                lower_source_description(
                    assertion.description.as_ref(),
                    "assertion",
                    &assertion_target,
                    &mut ir,
                );
                lower_assert(assertion, &semantic, &mut ir, &mut diagnostics)
            }
            Item::Rule(rule) => {
                lower_source_tags(&rule.tags, "rule", &rule.name.name, &mut ir);
                lower_source_description(
                    rule.description.as_ref(),
                    "rule",
                    &rule.name.name,
                    &mut ir,
                );
                warn_deprecated_consume(&rule, &mut warnings);
                lower_rule(
                    rule,
                    &semantic,
                    &workflow_contract_names,
                    &mut ir,
                    &mut diagnostics,
                )
            }
        }
    }

    ir.rule_dependencies = build_rule_dependencies(&ir.rules);
    validate_turn_access_grant_file_operations(&ir, &mut diagnostics);

    CompileOutput {
        ir: diagnostics.is_empty().then_some(ir),
        diagnostics,
        warnings,
    }
}

/// Post-lowering check: a turn-access grant whose resource is a declared `file store`
/// may only grant file operations (`read`/`write`/`import`/`export`). Runs after the
/// whole program is lowered so every file-store declaration is visible regardless of
/// source order. Grants whose resource is NOT a declared file store are left alone —
/// they may be package-provided resources whose operation vocabulary lives in the
/// capability registry (validated at the construct-graph layer), so this stays
/// zero-false-positive.
fn validate_turn_access_grant_file_operations(ir: &IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    const FILE_OPERATIONS: [&str; 4] = ["read", "write", "import", "export"];
    let file_stores: BTreeSet<&str> = ir
        .file_stores
        .iter()
        .map(|store| store.name.as_str())
        .collect();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            for grant in &effect.access_grants {
                if !file_stores.contains(grant.resource.as_str()) {
                    continue;
                }
                for op in &grant.operations {
                    if !FILE_OPERATIONS.contains(&op.operation.as_str()) {
                        diagnostics.push(Diagnostic { related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` grants `{}` on file store `{}`, which is not a file operation",
                                rule.name, op.operation, grant.resource
                            ),
                            suggestion: Some(
                                "file-store grants allow `read`, `write`, `import`, or `export`"
                                    .to_owned(),
                            ),
                        });
                    }
                }
            }
        }
    }
}

fn warn_deprecated_consume(rule: &RuleDecl, warnings: &mut Vec<Diagnostic>) {
    for line in rule.body.text.lines() {
        let line = line.trim().trim_end_matches(';');
        let mut words = line.split_whitespace();
        let is_counter_consume = words.next() == Some("consume")
            && words.next().is_some()
            && words.next() == Some("for");
        if (line == "consume" || line.starts_with("consume ")) && !is_counter_consume {
            warnings.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!("rule `{}` uses deprecated `consume`", rule.name.name),
                suggestion: Some(
                    "use `done` instead; `consume` will be removed in a future release".to_owned(),
                ),
            });
        }
    }
}

fn lower_source_tags(tags: &[TagDecl], target_kind: &str, target: &str, ir: &mut IrProgram) {
    for tag in tags {
        ir.source_tags.push(IrSourceTag {
            name: tag.name.clone(),
            target_kind: target_kind.to_owned(),
            target: target.to_owned(),
            span: tag.span,
        });
    }
}

fn lower_source_description(
    description: Option<&StringLiteral>,
    target_kind: &str,
    target: &str,
    ir: &mut IrProgram,
) {
    if let Some(description) = description {
        ir.source_descriptions.push(IrSourceDescription {
            value: description.value.clone(),
            target_kind: target_kind.to_owned(),
            target: target.to_owned(),
            span: description.span,
        });
    }
}

/// Detect recursive pattern application over the pattern-declaration graph and
/// emit `graph.unbounded_pattern_recursion` (severity error) for each expansion
/// cycle, naming the cycle. Returns the set of patterns that participate in a
/// cycle so the caller can suppress the generic "nested apply" message for them.
///
/// A pattern's body that `apply`s another pattern is an edge; a pattern that can
/// reach itself (directly via a self-apply, or transitively) cannot elaborate into
/// a finite first-order program, so v0 rejects it (spec/static-analysis.md). The
/// reachability closure mirrors `models/maude/pattern-recursion.maude`.
fn detect_pattern_recursion(
    patterns: &BTreeMap<String, PatternDecl>,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeSet<String> {
    // Application edges: pattern name -> the patterns its body applies, with spans.
    let mut edges: BTreeMap<&str, Vec<(&str, SourceSpan)>> = BTreeMap::new();
    for pattern in patterns.values() {
        let mut applied = Vec::new();
        for item in &pattern.items {
            if let Item::Apply(apply) = item {
                applied.push((apply.pattern.name.as_str(), apply.span));
            }
        }
        edges.insert(pattern.name.name.as_str(), applied);
    }

    // A pattern is recursive iff it can reach itself. Find a shortest cycle path
    // back to `start` via breadth-first search, tracking each node's predecessor.
    let find_cycle = |start: &str| -> Option<(Vec<String>, SourceSpan)> {
        let mut queue: VecDeque<&str> = VecDeque::new();
        // predecessor[node] = (came_from, span_of_edge) used to first reach `node`.
        let mut predecessor: BTreeMap<&str, (&str, SourceSpan)> = BTreeMap::new();
        for &(target, span) in edges.get(start).into_iter().flatten() {
            if target == start {
                // Direct self-application.
                return Some((vec![start.to_owned(), start.to_owned()], span));
            }
            if predecessor.insert(target, (start, span)).is_none() {
                queue.push_back(target);
            }
        }
        while let Some(node) = queue.pop_front() {
            for &(target, span) in edges.get(node).into_iter().flatten() {
                if target == start {
                    // Reconstruct start -> ... -> node, then close back to start.
                    let mut path = vec![node.to_owned()];
                    let mut cursor = node;
                    while cursor != start {
                        let (from, _) = predecessor[cursor];
                        path.push(from.to_owned());
                        cursor = from;
                    }
                    path.reverse();
                    path.push(start.to_owned());
                    // Report at the first apply edge of `start` that enters the cycle.
                    let first = &path[1];
                    let entry_span = edges
                        .get(start)
                        .into_iter()
                        .flatten()
                        .find(|(target, _)| target == first)
                        .map(|(_, span)| *span)
                        .unwrap_or(span);
                    return Some((path, entry_span));
                }
                if predecessor.insert(target, (node, span)).is_none() {
                    queue.push_back(target);
                }
            }
        }
        None
    };

    let mut recursive = BTreeSet::new();
    // Iterate patterns in declaration-name order for deterministic diagnostics, and
    // report each cycle once by skipping members already covered by a prior cycle.
    for name in patterns.keys() {
        if recursive.contains(name) {
            continue;
        }
        if let Some((cycle, span)) = find_cycle(name) {
            for member in &cycle {
                recursive.insert(member.clone());
            }
            diagnostics.push(Diagnostic { related: Vec::new(),
                span,
                message: format!(
                    "recursive pattern application is not allowed (graph.unbounded_pattern_recursion): expansion cycle {}",
                    cycle.join(" -> ")
                ),
                suggestion: Some(
                    "break the cycle: pattern expansion must elaborate into a finite program"
                        .to_owned(),
                ),
            });
        }
    }
    recursive
}

/// Reject a *transitive* runtime workflow-invocation cycle (A invokes B invokes A,
/// or longer). RESOLVED 2026-07-01: the invoke-recursion policy is "as permissive
/// as provable convergence at compile time allows"; whipplescript has no
/// convergence proof for runtime `invoke` recursion (termination is data-dependent
/// and there is no decreasing-measure mechanism yet), so — exactly parallel to
/// `detect_pattern_recursion` — any cycle is rejected as
/// `graph.unbounded_workflow_invocation_recursion`. Direct self-invocation (a cycle
/// of length 1) is already rejected per-rule in `validate_workflow_invocations`, so
/// self-edges are excluded here and this catches only length >= 2 cycles. Modeled
/// as invoke-graph non-convergence in `models/maude/subworkflow-convergence.maude`.
fn detect_workflow_invoke_recursion(program: &Program, diagnostics: &mut Vec<Diagnostic>) {
    // Invoke edges: workflow name -> the workflows its rules invoke, with the span
    // of the invoking rule body. Built over the raw AST (all workflows), so it is
    // independent of root selection. Self-edges are excluded (owned by the direct
    // per-rule recursion check).
    let mut edges: BTreeMap<String, Vec<(String, SourceSpan)>> = BTreeMap::new();
    let record_invokes =
        |name: &str, items: &[Item], edges: &mut BTreeMap<String, Vec<(String, SourceSpan)>>| {
            let entry = edges.entry(name.to_owned()).or_default();
            for item in items {
                let Item::Rule(rule) = item else {
                    continue;
                };
                for statement in workflow_invoke_statements(&rule.body.text) {
                    if let Some((target, _)) = invoke_statement_parts(&statement) {
                        if target != name {
                            entry.push((target.to_owned(), rule.body.span));
                        }
                    }
                }
            }
        };
    if let Some(root) = &program.workflow {
        record_invokes(&root.name, &program.items, &mut edges);
    }
    for workflow in &program.workflows {
        record_invokes(&workflow.name.name, &workflow.items, &mut edges);
    }

    // A workflow is in a cycle iff it can reach itself over invoke edges. BFS for a
    // shortest path back to `start` (mirrors `detect_pattern_recursion`).
    let find_cycle = |start: &str| -> Option<(Vec<String>, SourceSpan)> {
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut predecessor: BTreeMap<&str, (&str, SourceSpan)> = BTreeMap::new();
        for (target, span) in edges.get(start).into_iter().flatten() {
            if predecessor
                .insert(target.as_str(), (start, *span))
                .is_none()
            {
                queue.push_back(target.as_str());
            }
        }
        while let Some(node) = queue.pop_front() {
            for (target, span) in edges.get(node).into_iter().flatten() {
                if target == start {
                    let mut path = vec![node.to_owned()];
                    let mut cursor = node;
                    while cursor != start {
                        let (from, _) = predecessor[cursor];
                        path.push(from.to_owned());
                        cursor = from;
                    }
                    path.reverse();
                    path.push(start.to_owned());
                    let first = &path[1];
                    let entry_span = edges
                        .get(start)
                        .into_iter()
                        .flatten()
                        .find(|(target, _)| target == first)
                        .map(|(_, span)| *span)
                        .unwrap_or(*span);
                    return Some((path, entry_span));
                }
                if predecessor.insert(target.as_str(), (node, *span)).is_none() {
                    queue.push_back(target.as_str());
                }
            }
        }
        None
    };

    let mut flagged: BTreeSet<String> = BTreeSet::new();
    for name in edges.keys() {
        if flagged.contains(name) {
            continue;
        }
        if let Some((cycle, span)) = find_cycle(name) {
            for member in &cycle {
                flagged.insert(member.clone());
            }
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!(
                    "recursive workflow invocation is not allowed (graph.unbounded_workflow_invocation_recursion): invocation cycle {}",
                    cycle.join(" -> ")
                ),
                suggestion: Some(
                    "break the cycle: a runtime `invoke` cycle has no compile-time convergence proof; route the recurrence through an external event, clock, or durable boundary instead"
                        .to_owned(),
                ),
            });
        }
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
                related: Vec::new(),
                span: pattern.name.span,
                message: format!("pattern `{}` is declared more than once", pattern.name.name),
                suggestion: Some("rename one pattern declaration".to_owned()),
            });
        }
    }

    // v0 forbids recursive pattern application (spec/static-analysis.md,
    // graph.unbounded_pattern_recursion): an `apply` that reaches, directly or
    // transitively, a pattern already on the active expansion stack can never
    // elaborate into a finite first-order program. Detect cycles up front so the
    // precise diagnostic is emitted and the generic "nested apply not supported
    // yet" message is suppressed for the recursive case.
    let recursive_patterns = detect_pattern_recursion(&patterns, diagnostics);

    let mut expanded_items = Vec::new();
    let mut applications = Vec::new();
    for item in program.items {
        let Item::Apply(apply) = item else {
            expanded_items.push(item);
            continue;
        };
        let Some(pattern) = patterns.get(&apply.pattern.name) else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: apply.pattern.span,
                message: format!("pattern `{}` was not found", apply.pattern.name),
                suggestion: Some("declare the pattern before applying it".to_owned()),
            });
            continue;
        };
        if pattern.type_params.len() != apply.type_args.len() {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
                &recursive_patterns,
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
            Item::Harness(harness) => {
                names.insert(
                    harness.name.name.clone(),
                    generated_pattern_name(alias, &harness.name.name),
                );
            }
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
                related: Vec::new(),
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
                related: Vec::new(),
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
                related: Vec::new(),
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
    recursive_patterns: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<(String, Item)> {
    match item {
        Item::Include(include) => Some((
            format!("include:{}", include.path.value),
            Item::Include(include),
        )),
        Item::Use(use_decl) => Some((format!("use:{}", use_decl.name.value), Item::Use(use_decl))),
        Item::Queue(queue) => Some((format!("queue:{}", queue.name.name), Item::Queue(queue))),
        Item::Channel(channel) => Some((
            format!("channel:{}", channel.name.name),
            Item::Channel(channel),
        )),
        Item::FileStore(file_store) => Some((
            format!("file-store:{}", file_store.name.name),
            Item::FileStore(file_store),
        )),
        Item::Event(event) => Some((format!("event:{}", event.name), Item::Event(event))),
        Item::Source(source) => {
            Some((format!("source:{}", source.name.name), Item::Source(source)))
        }
        Item::Test(test) => Some((format!("test:{}", test.name.value), Item::Test(test))),
        Item::Lease(lease) => Some((format!("lease:{}", lease.name.name), Item::Lease(lease))),
        Item::Ledger(ledger) => {
            Some((format!("ledger:{}", ledger.name.name), Item::Ledger(ledger)))
        }
        Item::Counter(counter) => Some((
            format!("counter:{}", counter.name.name),
            Item::Counter(counter),
        )),
        Item::Flow(flow) => Some((format!("flow:{}", flow.name.name), Item::Flow(flow))),
        Item::Action(action) => {
            Some((format!("action:{}", action.name.name), Item::Action(action)))
        }
        Item::Harness(mut harness) => {
            let name = rename_ident(harness.name, alias, local_names);
            let generated = format!("harness:{}", name.name);
            harness.name = name;
            Some((generated, Item::Harness(harness)))
        }
        Item::WorkflowContract(contract) => {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
                related: Vec::new(),
                span: pattern.span,
                message: "nested pattern declarations are not supported".to_owned(),
                suggestion: Some("declare reusable patterns at source top level".to_owned()),
            });
            None
        }
        Item::Apply(apply) => {
            // A recursive nested apply was already rejected with the precise
            // graph.unbounded_pattern_recursion diagnostic by detect_pattern_recursion;
            // don't also emit the generic "not supported yet" message for it.
            if !recursive_patterns.contains(&apply.pattern.name) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: apply.span,
                    message: "pattern applications inside pattern bodies are not supported yet"
                        .to_owned(),
                    suggestion: Some(
                        "apply patterns from workflow bodies only in this implementation slice"
                            .to_owned(),
                    ),
                });
            }
            None
        }
        Item::Agent(mut agent) => {
            let name = rename_ident(agent.name, alias, local_names);
            let generated = format!("agent:{}", name.name);
            agent.name = name;
            if let Some(harness) = agent.harness {
                agent.harness = Some(Ident {
                    name: local_names
                        .get(&harness.name)
                        .cloned()
                        .unwrap_or(harness.name),
                    span: harness.span,
                });
            }
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
        Item::Table(mut table) => {
            let name = rename_ident(table.name, alias, local_names);
            let generated = format!("table:{}", name.name);
            table.name = name;
            for row in &mut table.rows {
                row.body.text = substitute_pattern_text(
                    &row.body.text,
                    type_substitutions,
                    value_substitutions,
                    local_names,
                );
            }
            Some((generated, Item::Table(table)))
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
            related: Vec::new(),
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
    // Track the first declaration span per name so a duplicate can point back to
    // it as related information ("first declared here").
    let mut first_spans: BTreeMap<String, SourceSpan> = BTreeMap::new();
    for item in &program.items {
        let name = match item {
            Item::Enum(enum_decl) => &enum_decl.name,
            Item::Class(class_decl) => &class_decl.name,
            _ => continue,
        };

        if !names.insert(name.name.clone()) {
            let mut diagnostic = Diagnostic {
                related: Vec::new(),
                span: name.span,
                message: format!("schema `{}` is declared more than once", name.name),
                suggestion: Some("rename one declaration or merge the schemas".to_owned()),
            };
            if let Some(first) = first_spans.get(&name.name) {
                diagnostic = diagnostic.with_related(*first, "first declared here");
            }
            diagnostics.push(diagnostic);
        } else {
            first_spans.insert(name.name.clone(), name.span);
        }
    }

    names
}

fn collect_harness_names(program: &Program, diagnostics: &mut Vec<Diagnostic>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &program.items {
        let Item::Harness(harness) = item else {
            continue;
        };
        if !names.insert(harness.name.name.clone()) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: harness.name.span,
                message: format!("harness `{}` is declared more than once", harness.name.name),
                suggestion: Some(
                    "rename one harness declaration or merge the harness settings".to_owned(),
                ),
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
                related: Vec::new(),
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
                related: Vec::new(),
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
        let mut leases = BTreeSet::new();
        let mut ledgers = BTreeSet::new();
        let mut counters = BTreeSet::new();
        let mut channels = BTreeSet::new();

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
                Item::Lease(lease) => {
                    leases.insert(lease.name.name.clone());
                }
                Item::Ledger(ledger) => {
                    ledgers.insert(ledger.name.name.clone());
                }
                Item::Counter(counter) => {
                    counters.insert(counter.name.name.clone());
                }
                Item::Channel(channel) => {
                    channels.insert(channel.name.name.clone());
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
            leases,
            ledgers,
            counters,
            channels,
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
                milestones: collect_milestone_declarations(&program.items),
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
                milestones: collect_milestone_declarations(&workflow.items),
            },
        );
    }

    surfaces
}

/// Scans a workflow's rule bodies for `emit milestone "<name>" [of <Class>]`
/// projections (Family C) and returns the name -> payload-class map (empty class
/// string for a bare milestone). The emit statement IS the declaration — the
/// declared milestone set is exactly what the workflow's rules can project, which
/// is what a parent's `after p reaches "<name>"` is validated against.
fn collect_milestone_declarations(items: &[Item]) -> BTreeMap<String, String> {
    let mut milestones = BTreeMap::new();
    for item in items {
        let Item::Rule(rule) = item else {
            continue;
        };
        for (name, class) in milestone_emissions_in_body(&rule.body.text) {
            milestones.entry(name).or_insert(class);
        }
    }
    milestones
}

/// Validates Family C milestone statements in a rule (spec/decision-records/
/// discriminated-families-design.md sections 6.4 / 7.3):
///   - child `emit milestone "<name>" of <Class>` — `<Class>` must be a declared
///     class (the payload the observing parent narrows into scope);
///   - parent `after <p> reaches "<name>"` — `<p>` must be a workflow-invoke
///     binding in this rule, and `<name>` must be a milestone that the invoked
///     child workflow actually declares (the reject-undeclared / terminal-only
///     observation invariant: a parent cannot observe a state the child never
///     projects).
fn validate_milestone_statements(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Child side: every `emit milestone "<name>" of <Class>` payload class must
    // exist.
    for (name, class) in milestone_emissions_in_body(&rule.body.text) {
        if !class.is_empty() && !semantic.schemas.class_exists(&class) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` emits milestone `{name}` with unknown payload class `{class}`",
                    rule.name.name
                ),
                suggestion: Some(format!("declare `class {class}` before projecting it")),
            });
        }
    }

    // Parent side: every `after <p> reaches "<name>"` must name a milestone the
    // invoked child declares.
    for (binding, milestone) in milestone_reaches_in_body(&rule.body.text) {
        let Some(workflow) = invoke_binding_workflow(rule, &binding) else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` has `after {binding} reaches \"{milestone}\"` for `{binding}`, which is not a workflow-invoke binding in this rule",
                    rule.name.name
                ),
                suggestion: Some(
                    "`reaches` observes a child workflow milestone; bind the child with `invoke W { ... } as <binding>` first"
                        .to_owned(),
                ),
            });
            continue;
        };
        let declared = semantic
            .workflow_inputs
            .get(&workflow)
            .map(|surface| surface.milestones.contains_key(&milestone))
            .unwrap_or(false);
        if !declared {
            let available = semantic
                .workflow_inputs
                .get(&workflow)
                .map(|surface| {
                    surface
                        .milestones
                        .keys()
                        .map(|name| format!("\"{name}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let suggestion = if available.is_empty() {
                format!("workflow `{workflow}` declares no milestones; add `emit milestone \"{milestone}\" ...` to it")
            } else {
                format!("workflow `{workflow}` declares: {available}")
            };
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` reaches milestone `{milestone}` that workflow `{workflow}` does not declare",
                    rule.name.name
                ),
                suggestion: Some(suggestion),
            });
        }
    }
}

/// Parses `after <binding> reaches "<name>"` headers out of a rule body's text,
/// returning (binding, milestone-name) pairs. Mirrors `milestone_emissions_in_body`.
fn milestone_reaches_in_body(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let trimmed = raw.trim();
        let Some(rest) = trimmed.strip_prefix("after ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let Some(binding) = words.next() else {
            continue;
        };
        if words.next() != Some("reaches") {
            continue;
        }
        let Some(quoted) = words.next() else {
            continue;
        };
        if !(quoted.starts_with('"') && quoted.ends_with('"') && quoted.len() >= 2) {
            continue;
        }
        out.push((binding.to_owned(), quoted.trim_matches('"').to_owned()));
    }
    out
}

/// Maps an `invoke <Workflow> { ... } as <binding>` binding to the invoked
/// workflow name within a single rule, so a sibling `after <binding> reaches`
/// can find the child workflow whose milestones it observes.
fn invoke_binding_workflow(rule: &RuleDecl, binding: &str) -> Option<String> {
    for statement in workflow_invoke_statements(&rule.body.text) {
        let (target, _) = invoke_statement_parts(&statement)?;
        if let Some(as_binding) = binding_after_as(&statement) {
            if as_binding == binding {
                return Some(target.to_owned());
            }
        }
    }
    None
}

/// Resolves the payload class of a child milestone for `after <binding> reaches
/// "<milestone>"`: follow `binding` to its invoked workflow, then look up the
/// milestone in that workflow's declared set. `Some("")` means the milestone is
/// declared but payload-less; `None` means undeclared (reject) or unresolvable.
fn milestone_payload_class(
    rule: &RuleDecl,
    binding: &str,
    milestone: &str,
    semantic: &SemanticContext,
) -> Option<String> {
    let workflow = invoke_binding_workflow(rule, binding)?;
    let surface = semantic.workflow_inputs.get(&workflow)?;
    surface.milestones.get(milestone).cloned()
}

/// Parses `emit milestone "<name>" [of <Class>]` headers out of a rule body's
/// text, returning (name, class) pairs (class is empty for a bare milestone).
/// Text-based to mirror the other body scanners (`workflow_invoke_statements`)
/// and stay independent of flow-vs-rule body provenance.
fn milestone_emissions_in_body(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let trimmed = raw.trim();
        let Some(rest) = trimmed.strip_prefix("emit milestone ") else {
            continue;
        };
        // The name is a quoted string literal; take the text between the first
        // pair of quotes.
        let rest = rest.trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        let Some(close) = rest[1..].find('"') else {
            continue;
        };
        let name = rest[1..=close].to_owned();
        let after_name = rest[close + 2..].trim_start();
        let class = after_name
            .strip_prefix("of ")
            .map(|tail| {
                tail.trim_start()
                    .split(|c: char| c.is_whitespace() || c == '{')
                    .next()
                    .unwrap_or("")
                    .to_owned()
            })
            .unwrap_or_default();
        out.push((name, class));
    }
    out
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
                ("summary", string_ty()),
                ("agent", string_ty()),
                ("provider", string_ty()),
                ("status", string_ty()),
                ("run_id", string_ty()),
                ("effect_id", string_ty()),
            ],
        );
        index.insert_class(
            "HumanAnswer",
            [
                ("inbox_item_id", string_ty()),
                ("effect_id", string_ty()),
                ("prompt", string_ty()),
                ("answered_by", string_ty()),
                ("choice", string_ty()),
                ("text", string_ty()),
            ],
        );
        index.insert_class(
            "WorkItem",
            [
                ("id", string_ty()),
                ("title", string_ty()),
                ("body", string_ty()),
                ("queue", string_ty()),
                ("status", string_ty()),
                ("labels", array_ty(string_ty())),
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
                // DR-0032: `kind` names the failing effect — the `EffectError` base
                // field that lets a future runtime union dispatch and that
                // telemetry reads. Static narrowing does not require it.
                ("kind", string_ty()),
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
        // The generic inbound messaging envelope (spec/messaging.md): a
        // `when message from <channel> as msg` binding sees a `Message`, never a
        // domain type. Structured sub-payloads (sender_claims, interaction,
        // correlation) are JSON-serialized strings here; provider-specific
        // payloads live in bounded evidence / `raw_ref`, not as untyped facts.
        index.insert_class(
            "Message",
            [
                ("message_id", string_ty()),
                ("channel", string_ty()),
                ("provider", string_ty()),
                ("received_at", string_ty()),
                ("sender", string_ty()),
                ("sender_claims", string_ty()),
                ("thread_id", string_ty()),
                ("text", string_ty()),
                ("markdown", string_ty()),
                ("attachments", array_ty(string_ty())),
                ("interaction", string_ty()),
                ("raw_ref", string_ty()),
                ("correlation", string_ty()),
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
                        .map(|variant| variant.name.name.clone())
                        .collect(),
                );
                // Data-carrying variants are visible as generated
                // `<Enum>.<Variant>` classes (spec/sum-types.md), so case
                // bindings type-check field access against them.
                for variant in &enum_decl.variants {
                    if variant.fields.is_empty() {
                        continue;
                    }
                    let mut fields = BTreeMap::new();
                    fields.insert(
                        "variant".to_owned(),
                        TypeSyntax::LiteralString {
                            value: variant.name.name.clone(),
                            span: variant.name.span,
                        },
                    );
                    for field in &variant.fields {
                        fields.insert(field.name.name.clone(), field.ty.clone());
                    }
                    self.classes.insert(
                        format!("{}.{}", enum_decl.name.name, variant.name.name),
                        fields,
                    );
                }
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
                self.insert_presence(&class_decl.name.name, &class_decl.fields);
            }
            Item::Event(event) => {
                self.events.insert(event.name.clone());
                // The payload schema is indexed under the dotted signal name,
                // unreachable from user class declarations, so bare `when
                // <signal> as x` bindings type-check field access.
                self.classes.insert(
                    event.name.clone(),
                    event
                        .fields
                        .iter()
                        .map(|field| (field.name.name.clone(), field.ty.clone()))
                        .collect(),
                );
                self.insert_presence(&event.name, &event.fields);
            }
            _ => {}
        }
    }

    /// Record Family B presence conditions for a schema's fields (if any).
    fn insert_presence(&mut self, schema: &str, fields: &[ClassField]) {
        let conditions: BTreeMap<String, (String, String)> = fields
            .iter()
            .filter_map(|field| {
                field
                    .presence_condition
                    .clone()
                    .map(|condition| (field.name.name.clone(), condition))
            })
            .collect();
        if !conditions.is_empty() {
            self.presence.insert(schema.to_owned(), conditions);
        }
    }

    /// The presence condition `(discriminant, literal)` for a schema field, if any.
    fn field_presence(&self, schema: &str, field: &str) -> Option<&(String, String)> {
        self.presence
            .get(schema)
            .and_then(|fields| fields.get(field))
    }

    fn merge(&mut self, other: SchemaIndex) {
        self.classes.extend(other.classes);
        self.enums.extend(other.enums);
        self.presence.extend(other.presence);
    }

    fn class_exists(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }

    fn resolve_field_path(&self, root_schema: &str, path: &[String]) -> Result<TypeSyntax, String> {
        // Dotted runtime fact names (general `when fact <name>` matches) are
        // untyped — unless a declared `event` (or generated `<Enum>.<Variant>`
        // class) indexes a payload schema under the dotted name, in which
        // case field paths are statically validated against it.
        if root_schema.contains('.') && !self.classes.contains_key(root_schema) {
            return Ok(TypeSyntax::Ref {
                name: Ident {
                    name: root_schema.to_owned(),
                    span: zero_span(),
                },
            });
        }
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
        span: contract.span,
    });
}

fn lower_use(use_decl: UseDecl, ir: &mut IrProgram, _diagnostics: &mut Vec<Diagnostic>) {
    let kind = IrUseKind::Package;
    ir.uses.push(IrUse {
        kind,
        name: use_decl.name.value,
    });
}

fn lower_queue(queue: QueueDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if queue.tracker.name != "builtin" {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: queue.tracker.span,
            message: format!(
                "queue `{}` uses unavailable tracker `{}`",
                queue.name.name, queue.tracker.name
            ),
            suggestion: Some(
                "`builtin` is the available tracker; loft/github/linear/jira are deferred bindings"
                    .to_owned(),
            ),
        });
    }
    ir.queues.push(IrQueue {
        name: queue.name.name,
        tracker: queue.tracker.name,
        span: queue.span,
    });
}

fn lower_channel(channel: ChannelDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    // Two channels with the same name would make `send via <name>` / `when
    // message from <name>` ambiguous (a channel name is a routing identity).
    if let Some(existing) = ir
        .channels
        .iter()
        .find(|other| other.name == channel.name.name)
    {
        diagnostics.push(
            Diagnostic {
                related: Vec::new(),
                span: channel.name.span,
                message: format!("channel `{}` is declared more than once", channel.name.name),
                suggestion: Some("give each channel a unique name".to_owned()),
            }
            .with_related(existing.span, "first declared here"),
        );
        return;
    }
    // The construct-side accepts any declared provider; runtime provider
    // availability (and the outbound/inbound feature checks in
    // spec/messaging.md "Static Checks") is later runtime-stage work.
    ir.channels.push(IrChannel {
        name: channel.name.name,
        provider: channel.provider.name,
        workspace: channel.workspace.map(|workspace| workspace.name),
        destination: channel.destination.map(|destination| destination.value),
        span: channel.span,
    });
}

fn lower_harness(harness: HarnessDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if !is_supported_harness_kind(&harness.kind.name) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: harness.kind.span,
            message: format!(
                "harness `{}` uses unsupported kind `{}`",
                harness.name.name, harness.kind.name
            ),
            suggestion: Some(
                "supported harness kinds are `codex`, `claude`, `pi`, `fixture`, and `command`"
                    .to_owned(),
            ),
        });
    }

    ir.harnesses.push(IrHarness {
        name: harness.name.name,
        kind: harness.kind.name,
        span: harness.span,
    });
}

fn is_supported_harness_kind(kind: &str) -> bool {
    matches!(
        kind,
        "codex" | "claude" | "pi" | "fixture" | "native-fixture" | "command" | "owned"
    )
}

fn lower_agent(
    agent: AgentDecl,
    ir: &mut IrProgram,
    harness_names: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut lowered = IrAgent {
        name: agent.name.name.clone(),
        harness: agent.harness.as_ref().map(|harness| harness.name.clone()),
        provider: None,
        profile: None,
        capacity: None,
        skills: Vec::new(),
        capabilities: Vec::new(),
        tools: Vec::new(),
    };

    if let Some(harness) = &agent.harness {
        if !harness_names.contains(&harness.name) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: harness.span,
                message: format!(
                    "agent `{}` uses unknown harness `{}`",
                    agent.name.name, harness.name
                ),
                suggestion: Some(format!(
                    "declare `harness {}: fixture` before using it",
                    harness.name
                )),
            });
        }
    }

    for field in agent.fields {
        match field {
            AgentField::Provider(provider) => {
                if lowered.provider.is_some() {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: provider.span,
                        message: format!(
                            "agent `{}` declares provider more than once",
                            agent.name.name
                        ),
                        suggestion: Some(
                            "keep exactly one `provider` field in the agent block".to_owned(),
                        ),
                    });
                }
                if agent.harness.is_some() {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: provider.span,
                        message: format!(
                            "agent `{}` declares both `using` harness and direct provider `{}`",
                            agent.name.name, provider.name
                        ),
                        suggestion: Some(
                            "use either `agent name using harness { ... }` or `provider codex`, not both"
                                .to_owned(),
                        ),
                    });
                }
                if !is_supported_harness_kind(&provider.name) {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: provider.span,
                        message: format!(
                            "agent `{}` uses unsupported provider `{}`",
                            agent.name.name, provider.name
                        ),
                        suggestion: Some(
                            "supported providers are `owned`, `codex`, `claude`, `pi`, `fixture`, `native-fixture`, and `command`"
                                .to_owned(),
                        ),
                    });
                }
                lowered.provider = Some(provider.name);
            }
            AgentField::Profile(profile) => lowered.profile = Some(profile.value),
            AgentField::Capacity(capacity, span) => {
                if capacity == 0 {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
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
                            related: Vec::new(),
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
                            related: Vec::new(),
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
            AgentField::Tools(tools, _) => {
                let mut seen = BTreeSet::new();
                for tool in tools {
                    if !seen.insert(tool.name.clone()) {
                        diagnostics.push(Diagnostic {
                            related: Vec::new(),
                            span: tool.span,
                            message: format!(
                                "agent `{}` grants tool `{}` more than once",
                                agent.name.name, tool.name
                            ),
                            suggestion: Some("remove the duplicate tool entry".to_owned()),
                        });
                    }
                    lowered.tools.push(tool.name);
                }
            }
            AgentField::Unknown { name, .. } => {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: name.span,
                    message: format!(
                        "unknown agent field `{}` on agent `{}`",
                        name.name, agent.name.name
                    ),
                    suggestion: Some(
                        "supported agent fields are `provider`, `profile`, `capacity`, `skills`, `capabilities`, and `tools`".to_owned(),
                    ),
                });
            }
        }
    }

    if lowered.profile.is_none() {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: agent.name.span,
            message: format!("agent `{}` is missing a profile", agent.name.name),
            suggestion: Some("add `profile \"profile-name\"` inside the agent block".to_owned()),
        });
    }

    if lowered.harness.is_none() && lowered.provider.is_none() {
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: agent.name.span,
            message: format!("agent `{}` is missing provider binding", agent.name.name),
            suggestion: Some("add `provider codex`, `provider claude`, `provider pi`, `provider fixture`, or use an explicit harness".to_owned()),
        });
    }

    if lowered.capacity.is_none() {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
        if !variants.insert(variant.name.name.clone()) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: variant.span,
                message: format!(
                    "enum `{}` declares variant `{}` more than once",
                    enum_decl.name.name, variant.name.name
                ),
                suggestion: Some(
                    "remove the duplicate variant or give it a distinct name".to_owned(),
                ),
            });
        }
        // The discriminant is synthesized from the variant name
        // (spec/sum-types.md): `variant` is reserved inside variant bodies.
        for field in &variant.fields {
            if field.name.name == "variant" {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: field.name.span,
                    message: format!(
                        "variant `{}` of enum `{}` declares reserved field `variant`",
                        variant.name.name, enum_decl.name.name
                    ),
                    suggestion: Some(
                        "the discriminant is synthesized from the variant name; rename the field"
                            .to_owned(),
                    ),
                });
            }
        }
    }

    // Each data-carrying variant lowers to a generated `<Enum>.<Variant>`
    // class holding the literal `variant` discriminant plus its payload
    // (spec/sum-types.md). The dotted name is unreachable from user source.
    for variant in &enum_decl.variants {
        if variant.fields.is_empty() {
            continue;
        }
        let mut fields = vec![IrClassField {
            name: "variant".to_owned(),
            ty: IrType::LiteralString(variant.name.name.clone()),
            is_key: false,
            presence_condition: None,
            span: variant.name.span,
        }];
        fields.extend(variant.fields.iter().map(|field| IrClassField {
            name: field.name.name.clone(),
            ty: lower_type(field.ty.clone()),
            is_key: false,
            presence_condition: field.presence_condition.clone(),
            span: field.span,
        }));
        ir.schemas.push(IrSchema::Class(IrClass {
            name: format!("{}.{}", enum_decl.name.name, variant.name.name),
            fields,
            span: variant.span,
        }));
    }

    ir.schemas.push(IrSchema::Enum(IrEnum {
        name: enum_decl.name.name,
        variants: enum_decl
            .variants
            .into_iter()
            .map(|variant| variant.name.name)
            .collect(),
        span: enum_decl.span,
    }));
}

fn validate_test_expr_source(
    label: &str,
    source: &str,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if source.trim().is_empty() {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span,
            message: format!("{label} is empty"),
            suggestion: Some("provide an expression".to_owned()),
        });
        return;
    }
    if let Err(error) = parse_expression(source) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span,
            message: format!("{label} is not a valid expression: {error}"),
            suggestion: None,
        });
    }
}

fn lower_test(test: TestDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if ir
        .tests
        .iter()
        .any(|existing| existing.name == test.name.value)
    {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: test.name.span,
            message: format!("test `{}` is declared more than once", test.name.value),
            suggestion: Some("give each test scenario a distinct name".to_owned()),
        });
    }
    if !test
        .clauses
        .iter()
        .any(|clause| matches!(clause, TestClause::Expect(_)))
    {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: test.span,
            message: format!("test `{}` has no `expect` clause", test.name.value),
            suggestion: Some("a test must assert at least one expected outcome".to_owned()),
        });
    }
    // Validate that captured expression source (given field values, projection
    // predicates) parses, so `whip check` catches malformed test scenarios.
    for clause in &test.clauses {
        match clause {
            TestClause::Given(
                GivenClause::Input { fields, .. }
                | GivenClause::Fact { fields, .. }
                | GivenClause::Signal { fields, .. },
            ) => {
                for field in fields {
                    validate_test_expr_source(
                        &format!("given field `{}`", field.name.name),
                        &field.value,
                        field.span,
                        diagnostics,
                    );
                }
            }
            TestClause::Expect(ExpectClause {
                target: ExpectTarget::Projection(query),
                ..
            }) => match &query.kind {
                ProjQueryKind::Count { predicate, .. } | ProjQueryKind::Where { predicate } => {
                    validate_test_expr_source(
                        &format!("predicate on `{}`", query.noun),
                        predicate,
                        query.span,
                        diagnostics,
                    );
                }
                ProjQueryKind::Exists => {}
            },
            _ => {}
        }
    }
    ir.tests.push(IrTest {
        name: test.name.value,
        workflow: test.workflow.map(|identifier| identifier.name),
        clauses: test.clauses,
        span: test.span,
    });
}

fn lower_source(source: SourceDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if ir
        .sources
        .iter()
        .any(|existing| existing.name == source.name.name)
    {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: source.name.span,
            message: format!("source `{}` is declared more than once", source.name.name),
            suggestion: Some("remove the duplicate source declaration".to_owned()),
        });
    }
    // Clock-source static checks (spec/std-time.md): a recurring schedule must
    // declare a missed policy (no silent default), and a calendar schedule should
    // declare a timezone.
    if let Some(clock) = &source.clock {
        let recurring = !matches!(clock.recurrence, Recurrence::At { .. });
        if recurring && clock.missed.is_none() {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: clock.span,
                message: format!(
                    "recurring source `{}` must declare a `missed` policy",
                    source.name.name
                ),
                suggestion: Some(
                    "add `missed skip`, `missed coalesce`, or `missed catch_up limit N`".to_owned(),
                ),
            });
        }
        if matches!(clock.recurrence, Recurrence::EveryCalendar { .. }) && clock.timezone.is_none()
        {
            diagnostics.push(Diagnostic { related: Vec::new(),
                span: clock.span,
                message: format!(
                    "calendar source `{}` should declare a `timezone`",
                    source.name.name
                ),
                suggestion: Some(
                    "add `timezone \"America/New_York\"`; a calendar schedule without one defaults to UTC".to_owned(),
                ),
            });
        }
    }
    let is_clock = source.clock.is_some();
    let recurrence = source.clock.as_ref().map(|clock| clock.recurrence.clone());
    let timezone = source
        .clock
        .as_ref()
        .and_then(|clock| clock.timezone.as_ref().map(|tz| tz.value.clone()));
    let missed = source.clock.as_ref().and_then(|clock| clock.missed);
    ir.sources.push(IrSource {
        name: source.name.name,
        provider: source.provider.name,
        is_clock,
        recurrence,
        timezone,
        missed,
        observe_binding: source.observe_binding.name,
        emit_signal: source.emit.signal,
        emit_fields: source
            .emit
            .fields
            .into_iter()
            .map(|field| IrSourceEmitField {
                name: field.name.name,
                value: field.value,
                span: field.span,
            })
            .collect(),
        span: source.span,
    });
}

fn lower_event(event: EventDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if ir.events.iter().any(|existing| existing.name == event.name) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: event.name_span,
            message: format!("signal `{}` is declared more than once", event.name),
            suggestion: Some("remove the duplicate signal declaration".to_owned()),
        });
    }
    let mut fields = BTreeSet::new();
    for field in &event.fields {
        if !fields.insert(field.name.name.clone()) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: field.name.span,
                message: format!(
                    "signal `{}` declares field `{}` more than once",
                    event.name, field.name.name
                ),
                suggestion: Some(
                    "remove the duplicate field or give it a distinct name".to_owned(),
                ),
            });
        }
    }
    validate_presence_conditions(&event.name, &event.fields, diagnostics);

    ir.events.push(IrEvent {
        name: event.name,
        fields: event
            .fields
            .into_iter()
            .map(|field| IrClassField {
                name: field.name.name,
                ty: lower_type(field.ty),
                is_key: false,
                presence_condition: field.presence_condition,
                span: field.span,
            })
            .collect(),
        span: event.span,
    });
}

/// The string-literal values of a literal-union (or single-literal) type, or `None`
/// if the type is not a pure string-literal union. Used to validate Family B
/// discriminants.
fn literal_union_values(ty: &TypeSyntax) -> Option<Vec<String>> {
    match ty {
        TypeSyntax::LiteralString { value, .. } => Some(vec![value.clone()]),
        TypeSyntax::Union { variants, .. } => {
            let values = variants
                .iter()
                .filter_map(|variant| match variant {
                    TypeSyntax::LiteralString { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (!values.is_empty() && values.len() == variants.len()).then_some(values)
        }
        _ => None,
    }
}

/// Family B validation (spec/decision-records/discriminated-families-design.md §6.3):
/// every `<field> <T> when <disc> is "<lit>"` must name a same-schema discriminant
/// that is a string-literal union, and `<lit>` must be one of its values.
fn validate_presence_conditions(
    container: &str,
    fields: &[ClassField],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for field in fields {
        let Some((disc, literal)) = &field.presence_condition else {
            continue;
        };
        let Some(disc_field) = fields.iter().find(|candidate| &candidate.name.name == disc) else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: field.span,
                message: format!(
                    "`{container}` field `{}` is conditioned on unknown discriminant `{disc}`",
                    field.name.name
                ),
                suggestion: Some(
                    "`when <field> is \"...\"` must name a literal-union field of the same schema"
                        .to_owned(),
                ),
            });
            continue;
        };
        match literal_union_values(&disc_field.ty) {
            Some(values) if values.iter().any(|value| value == literal) => {}
            Some(values) => diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: field.span,
                message: format!(
                    "`{container}` field `{}` is conditioned on `{disc} is \"{literal}\"`, which is not a value of `{disc}`",
                    field.name.name
                ),
                suggestion: Some(format!("use one of: {}", values.join(", "))),
            }),
            None => diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: field.span,
                message: format!(
                    "`{container}` field `{}` is conditioned on `{disc}`, which is not a string-literal discriminant",
                    field.name.name
                ),
                suggestion: Some(
                    "the discriminant must be a string-literal union, e.g. `kind \"a\" | \"b\"`"
                        .to_owned(),
                ),
            }),
        }
    }
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
                related: Vec::new(),
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

    // v0 allows a single natural key per class (import keys one fact per row).
    let key_fields = class_decl
        .fields
        .iter()
        .filter(|field| field.is_key)
        .collect::<Vec<_>>();
    if key_fields.len() > 1 {
        for field in key_fields.into_iter().skip(1) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: field.span,
                message: format!(
                    "class `{}` declares more than one `@key` field",
                    class_decl.name.name
                ),
                suggestion: Some("a class has at most one `@key` natural key in v0".to_owned()),
            });
        }
    }

    validate_presence_conditions(&class_decl.name.name, &class_decl.fields, diagnostics);

    ir.schemas.push(IrSchema::Class(IrClass {
        name: class_decl.name.name,
        span: class_decl.span,
        fields: class_decl
            .fields
            .into_iter()
            .map(|field| IrClassField {
                name: field.name.name,
                ty: lower_type(field.ty),
                is_key: field.is_key,
                presence_condition: field.presence_condition,
                span: field.span,
            })
            .collect(),
    }));
}

fn lower_table(
    table: TableDecl,
    semantic: &SemanticContext,
    workflow_contract_names: &WorkflowContractNames,
    ir: &mut IrProgram,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !semantic.schemas.class_exists(&table.schema.name) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: table.schema.span,
            message: format!(
                "table `{}` targets unknown class `{}`",
                table.name.name, table.schema.name
            ),
            suggestion: Some("declare the class before seeding rows for it".to_owned()),
        });
        return;
    }

    if table.rows.is_empty() {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: table.span,
            message: format!("table `{}` has no rows", table.name.name),
            suggestion: Some("add at least one `{ ... }` row".to_owned()),
        });
        return;
    }

    let mut body = String::new();
    for row in &table.rows {
        push_line(&mut body, format!("record {} {{", table.schema.name));
        push_block_body(&row.body.text, &mut body);
        push_line(&mut body, "}");
        body.push('\n');
    }
    if body.ends_with('\n') {
        body.pop();
    }

    let rule = RuleDecl {
        name: Ident {
            name: format!("table_{}", table.name.name),
            span: table.name.span,
        },
        tags: Vec::new(),
        description: None,
        whens: vec![WhenClause {
            text: "started".to_owned(),
            span: table.name.span,
        }],
        body: BlockSource {
            text: body,
            span: table.span,
        },
        span: table.span,
    };

    let record_sources = table
        .rows
        .iter()
        .map(|row| IrRecordSource {
            schema: table.schema.name.clone(),
            construct: "table_row".to_owned(),
            span: row.span,
        })
        .collect::<Vec<_>>();

    let rule_name = rule.name.name.clone();
    lower_rule(rule, semantic, workflow_contract_names, ir, diagnostics);
    if let Some(rule) = ir
        .rules
        .iter_mut()
        .rev()
        .find(|rule| rule.name == rule_name)
    {
        rule.metadata.record_sources = record_sources;
    }
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
                related: Vec::new(),
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
    validate_coerce_prompt_content_type_annotations(&coerce, diagnostics);

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
                    related: Vec::new(),
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
                        related: Vec::new(),
                        span: agent.span,
                        message: format!("AgentRef lists agent `{}` more than once", agent.name),
                        suggestion: Some(
                            "remove the duplicate agent from the AgentRef domain".to_owned(),
                        ),
                    });
                }
                if !agent_names.contains(&agent.name) {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
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
            | "HumanAnswer"
            | "Evidence"
            | "TerminalFailed"
            | "TerminalTimedOut"
            | "TerminalCancelled"
    )
}

/// The terminal-family schemas are `origin = observer` (discriminated-families
/// design §5.4): the kernel projects them when it observes an effect or child
/// terminal, and user rules may only *eliminate* them (`after … fails/times
/// out/cancels as f`), never *construct* them. A rule that `record`s one would
/// forge a terminal outcome the kernel never produced, misleading the
/// `after`/terminal-case reaction machinery. Rejected at check time.
fn is_observer_only_schema(name: &str) -> bool {
    matches!(
        name,
        "TerminalFailed" | "TerminalTimedOut" | "TerminalCancelled"
    )
}

fn lower_rule(
    rule: RuleDecl,
    semantic: &SemanticContext,
    workflow_contract_names: &WorkflowContractNames,
    ir: &mut IrProgram,
    diagnostics: &mut Vec<Diagnostic>,
) {
    validate_canonical_rule_body_syntax(&rule, diagnostics);
    let metadata = analyze_rule(&rule, semantic, diagnostics);
    validate_workflow_terminal_actions(
        &rule,
        semantic,
        &binding_types_for_rule(&rule),
        &known_roots_for_rule(&rule),
        workflow_contract_names,
        diagnostics,
    );
    validate_effectful_self_trigger(&rule, &metadata, diagnostics);
    validate_flowfail_generated_only(&rule, diagnostics);
    validate_send_channels(&rule, semantic, diagnostics);
    validate_message_from_channels(&rule, semantic, diagnostics);
    validate_flow_namespace_access(&rule, &metadata, diagnostics);
    validate_evidence_fact_not_matched(&rule, diagnostics);
    validate_turn_access_grants(&rule, &metadata, diagnostics);
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

fn validate_canonical_rule_body_syntax(rule: &RuleDecl, diagnostics: &mut Vec<Diagnostic>) {
    for line in rule.body.text.lines().map(str::trim) {
        if line.starts_with("then ") {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` uses unsupported `then` sequencing",
                    rule.name.name
                ),
                suggestion: Some(
                    "use `after <effect> succeeds { ... }` blocks for effect sequencing".to_owned(),
                ),
            });
        }
        if line.starts_with("after ") && line.contains("=>") {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` uses unsupported `after ... =>` sequencing",
                    rule.name.name
                ),
                suggestion: Some("write `after <effect> succeeds { ... }`".to_owned()),
            });
        }
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

/// The `flowfail` terminal is generated-only: flow expansion emits it for an
/// effect whose failure is unhandled in a self-terminating flow (the 503 auto-fail
/// trigger), routing to the kernel generic failed terminal. Authors drive failure
/// with the typed `fail <Failure> { ... }` terminal instead, so a `flowfail` in a
/// user (non-`flow.`) rule is rejected. Generated flow rules carry a dotted `flow.`
/// name a user identifier cannot form, so they are exempt.
/// `send via <channel>` (std.messaging) must name a declared `channel`. The
/// channel name is carried as the construct's `channel` field; an unknown channel
/// would lower to a `messaging.send` effect that no provider can route, so it is
/// rejected at compile time (mirrors `acquire`/`consume` resource-existence checks).
/// `when message from <channel> as msg` (spec/messaging.md) must name a declared
/// channel, mirroring the outbound `send via <channel>` check.
fn validate_message_from_channels(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for when in &rule.whens {
        let (pattern, _) = split_when_guard(&when.text);
        let Some(rest) = pattern.trim_start().strip_prefix("message from ") else {
            continue;
        };
        let Some(channel) = rest.split_whitespace().next() else {
            continue;
        };
        if !semantic.channels.iter().any(|c| c.as_str() == channel) {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: when.span,
                message: format!("`when message from {channel}` names an unknown channel"),
                suggestion: Some(
                    "declare it with `channel <name> { provider … }`, or correct the channel name"
                        .to_owned(),
                ),
            });
        }
    }
}

fn validate_send_channels(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (ast, _) =
        body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
    fn walk(
        statements: &[body::BodyStmt],
        semantic: &SemanticContext,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for statement in statements {
            match statement {
                body::BodyStmt::Effect(effect) => {
                    if let body::BodyEffectKind::ConstructCapabilityCall {
                        keyword, fields, ..
                    } = &effect.kind
                    {
                        if keyword == "send" {
                            if let Some(channel) =
                                fields.iter().find(|field| field.name == "channel")
                            {
                                if !semantic.channels.contains(&channel.source) {
                                    diagnostics.push(Diagnostic {
                                        related: Vec::new(),
                                        span: effect.span,
                                        message: format!(
                                            "`send via {}` names an unknown channel",
                                            channel.source
                                        ),
                                        suggestion: Some(
                                            "declare it with `channel <name> { provider … }`, or correct the channel name"
                                                .to_owned(),
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                body::BodyStmt::After(after) => walk(&after.body, semantic, diagnostics),
                body::BodyStmt::Case(case) => {
                    for branch in &case.branches {
                        walk(&branch.body, semantic, diagnostics);
                    }
                }
                body::BodyStmt::Branch(branch) => {
                    walk(&branch.then_body, semantic, diagnostics);
                    if let Some(else_body) = &branch.else_body {
                        walk(else_body, semantic, diagnostics);
                    }
                }
                body::BodyStmt::Handler(handler) => walk(&handler.body, semantic, diagnostics),
                _ => {}
            }
        }
    }
    walk(&ast.statements, semantic, diagnostics);
}

fn validate_flowfail_generated_only(rule: &RuleDecl, diagnostics: &mut Vec<Diagnostic>) {
    if rule.name.name.starts_with("flow.") {
        return;
    }
    for line in rule.body.text.lines() {
        if line.trim() == "flowfail" {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` uses the generated-only `flowfail` terminal",
                    rule.name.name
                ),
                suggestion: Some(
                    "`flowfail` is emitted internally by flow auto-fail; use a typed `fail <Failure> { ... }` terminal instead"
                        .to_owned(),
                ),
            });
            break;
        }
    }
}

/// Flow progression state (the `FlowAwait_*` classes a `flow` lowers to) is owned
/// by the flow's own generated rules. A user (non-generated) rule may not read,
/// match, consume, or record any flow-state fact (spec/static-analysis.md): touching
/// it would let user logic corrupt or short-circuit the flow's progression. The
/// rule's read/write/consume metadata is the structural signal (no text scanning,
/// so no false positives from a prompt that happens to mention the prefix).
/// Generated flow rules carry a dotted `flow.` name a user identifier cannot form,
/// so they are exempt. Modeled in `models/maude/flow-namespace.maude`.
fn validate_flow_namespace_access(
    rule: &RuleDecl,
    metadata: &IrRuleMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if rule.name.name.starts_with("flow.") {
        return;
    }
    let prefixed = format!("schema:{}", flow_expand::FLOW_STATE_PREFIX);
    let mut reported = BTreeSet::new();
    let accesses = metadata
        .fact_reads
        .iter()
        .chain(&metadata.fact_writes)
        .chain(&metadata.fact_consumes);
    for fact in accesses {
        if !fact.starts_with(&prefixed) || !reported.insert(fact.clone()) {
            continue;
        }
        let class = fact.strip_prefix("schema:").unwrap_or(fact);
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: rule.name.span,
            message: format!(
                "rule `{}` may not reference flow-state fact `{class}`: flow progression state is owned by the flow's generated rules",
                rule.name.name
            ),
            suggestion: Some(
                "drive the workflow from your own fact classes; a flow's `FlowAwait_*` state is internal".to_owned(),
            ),
        });
    }
}

/// In-turn agent observations — `agent.turn.streamed` (streamed progress),
/// `agent.turn.tool_requested` (in-turn tool call), and `agent.turn.artifact_captured`
/// (captured artifact/diff) — are recorded as EVIDENCE, never as rule-matchable facts
/// (spec/agent-harness.md). The rule-matchable lifecycle facts are
/// `agent.turn.started/completed/failed/timed_out/cancelled`. A `when` that matches an
/// evidence-only fact can never fire, so it is a compile-time error.
const EVIDENCE_ONLY_TURN_FACTS: [&str; 3] = [
    "agent.turn.streamed",
    "agent.turn.tool_requested",
    "agent.turn.artifact_captured",
];

/// Structural well-formedness of turn-access grants (`with access to <resource> { … }`)
/// on `agent.tell` effects: a grant must grant at least one operation, and a single
/// tell must not list the same resource twice (merge them). The deeper "required
/// Resource/Operation/Capability ports" validation against the capability registry is a
/// separate construct-graph-layer concern, so this stays registry-independent and
/// zero-false-positive.
fn validate_turn_access_grants(
    rule: &RuleDecl,
    metadata: &IrRuleMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for effect in &metadata.effects {
        if effect.access_grants.is_empty() {
            continue;
        }
        let mut seen = BTreeSet::new();
        for grant in &effect.access_grants {
            if grant.operations.is_empty() {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: effect.span,
                    message: format!(
                        "rule `{}` has a `with access to {}` grant that grants no operations",
                        rule.name.name, grant.resource
                    ),
                    suggestion: Some(
                        "list at least one operation in the grant block, or drop the grant"
                            .to_owned(),
                    ),
                });
            }
            if !seen.insert(grant.resource.clone()) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: effect.span,
                    message: format!(
                        "rule `{}` lists turn-access resource `{}` more than once on one `tell`",
                        rule.name.name, grant.resource
                    ),
                    suggestion: Some(
                        "merge the grant clauses for a resource into a single block".to_owned(),
                    ),
                });
            }
        }
    }
}

fn validate_evidence_fact_not_matched(rule: &RuleDecl, diagnostics: &mut Vec<Diagnostic>) {
    for when in &rule.whens {
        let (pattern, _) = split_when_guard(&when.text);
        let Some(name) = runtime_fact_name_for_pattern(pattern) else {
            continue;
        };
        if EVIDENCE_ONLY_TURN_FACTS.contains(&name.as_str()) {
            diagnostics.push(Diagnostic { related: Vec::new(),
                span: when.span,
                message: format!(
                    "rule `{}` matches evidence-only fact `{name}`: in-turn observations are evidence, not rule-matchable facts",
                    rule.name.name
                ),
                suggestion: Some(
                    "match a lifecycle fact (`agent.turn.completed`/`failed`/`timed_out`/`cancelled`) and read in-turn detail from its evidence".to_owned(),
                ),
            });
        }
    }
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
            diagnostics.push(Diagnostic { related: Vec::new(),
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
    known_roots: &BTreeSet<String>,
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
        // Header is `<name>` or (for `complete`) `<name> from <binding>` — the
        // bounded-type projection form (DR-0027), whose payload copies the binding.
        let Some(name) = rest.split('{').next().and_then(|header| {
            let mut parts = header.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(name), None, _) => Some(name),
                (Some(name), Some("from"), Some(binding))
                    if action == "complete" && is_identifier(binding) =>
                {
                    Some(name)
                }
                _ => None,
            }
        }) else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
                related: Vec::new(),
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
            known_roots,
            diagnostics,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_workflow_terminal_payload(
    rule: &RuleDecl,
    action: &str,
    terminal_name: &str,
    contract_ty: &TypeSyntax,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    known_roots: &BTreeSet<String>,
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
            diagnostics.push(Diagnostic { related: Vec::new(),
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
        validate_record_field(
            rule,
            &line,
            schema,
            semantic,
            binding_types,
            known_roots,
            diagnostics,
        );
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
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: rule.body.span,
            message: format!(
                "workflow terminal `{terminal_name}` is missing required field `{schema}.{required}`"
            ),
            suggestion: Some(format!("add `{required}` to the `{terminal_name}` payload")),
        });
    }
}

/// Maximum nesting depth of `after` blocks across `statements` (an `after` inside an
/// `after` is depth 2, …). Other nesting (`case`/`when`/handlers) is descended into so
/// an `after` buried inside them still counts, but only `after` increments the depth —
/// it is `after`-chaining specifically that `lint.deep_after_nesting` advises moving to
/// a `flow`. Computed from the body AST so prompt braces never confuse it.
fn max_after_depth(statements: &[body::BodyStmt]) -> usize {
    use body::BodyStmt;
    statements
        .iter()
        .map(|statement| match statement {
            BodyStmt::After(after) => 1 + max_after_depth(&after.body),
            BodyStmt::Case(case) => case
                .branches
                .iter()
                .map(|branch| max_after_depth(&branch.body))
                .max()
                .unwrap_or(0),
            BodyStmt::Branch(branch) => {
                let then_depth = max_after_depth(&branch.then_body);
                let else_depth = branch
                    .else_body
                    .as_deref()
                    .map(max_after_depth)
                    .unwrap_or(0);
                then_depth.max(else_depth)
            }
            BodyStmt::Handler(handler) => max_after_depth(&handler.body),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

fn analyze_rule(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> IrRuleMetadata {
    // Statement-form gate: every body must parse into the body AST. Unknown
    // statements, malformed modifiers, and unclosed blocks are spanned
    // errors here rather than silent no-ops at lowering time.
    let (body_ast, body_diagnostics) =
        body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
    diagnostics.extend(body_diagnostics);
    let mut metadata = IrRuleMetadata {
        fact_reads: rule
            .whens
            .iter()
            .map(|when| fact_read_from_when(&when.text))
            .collect(),
        max_after_depth: max_after_depth(&body_ast.statements),
        ..IrRuleMetadata::default()
    };
    let mut seen_bindings = BTreeSet::new();
    let mut binding_types = BTreeMap::new();
    for when in &rule.whens {
        // A pattern that binds (`... as x`) but maps to no known readiness
        // form would otherwise be a silently-dead rule.
        let (pattern_text, _) = split_when_guard(&when.text);
        if binding_after_as(pattern_text).is_some()
            && binding_from_when(&when.text).is_none()
            && !pattern_text.ends_with(" is available")
        {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: when.span,
                message: format!(
                    "rule `{}` has unknown readiness pattern `{pattern_text}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "match a class (`when Class as x`) or a runtime fact (`when fact <name> as x`)"
                        .to_owned(),
                ),
            });
        }
        if let Some((binding, schema)) = binding_from_when(&when.text) {
            validate_binding_name(rule, &binding, when.span, diagnostics);
            if !schema.contains('.') && !semantic.schemas.class_exists(&schema) {
                let suggestion = match closest_name(&schema, semantic.schemas.classes.keys()) {
                    Some(candidate) => {
                        format!("did you mean `{candidate}`? otherwise declare `class {schema}`")
                    }
                    None => format!("declare `class {schema}` before matching it"),
                };
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: when.span,
                    message: format!("rule `{}` matches unknown class `{schema}`", rule.name.name),
                    suggestion: Some(suggestion),
                });
            }
            // The bare dotted form is the typed signal reaction
            // (spec/event-ingress.md): it requires a declared `signal`;
            // undeclared dotted facts keep the untyped `when fact` form.
            if schema.contains('.')
                && !pattern_text.trim_start().starts_with("fact ")
                && !semantic.schemas.events.contains(&schema)
            {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: when.span,
                    message: format!(
                        "rule `{}` reacts to undeclared signal `{schema}`",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "declare `signal {schema} {{ ... }}` for a typed reaction, or use `when fact {schema} as ...` for an untyped one"
                    )),
                });
            }
            binding_types.insert(binding, schema);
        }
    }
    let mut effect_payload_types = collect_effect_payload_types(rule, semantic, diagnostics);
    // `exec ... -> Schema as binding` is parsed from the AST (the command text
    // can itself contain `->`/` as `, so a text scan is unsafe), giving its
    // result the same after-binding type flow a named `coerce -> Schema` gets.
    collect_exec_payload_types(&body_ast.statements, semantic, &mut effect_payload_types);
    // Inline `decide … as <binding>` carries the synthesized
    // `decide.<rule>.<binding>` class (see `collect_inline_decide_schemas`), so
    // its result is `case`able / field-accessible like a named coerce result.
    collect_decide_payload_types(
        &body_ast.statements,
        &rule.name.name,
        &mut effect_payload_types,
    );
    // `redact … as <binding>` result carries the synthesized `redact.<rule>.<binding>`
    // projected class (see `collect_redact_schemas`), so access through it resolves
    // against the kept-only fields.
    collect_redact_payload_types(
        &body_ast.statements,
        &rule.name.name,
        &mut effect_payload_types,
    );
    for (binding, payload_type) in &effect_payload_types {
        if let IrType::Ref(schema) = payload_type {
            binding_types.insert(binding.clone(), schema.clone());
        }
    }
    // `after <binding> <predicate> as <alias>`: the alias carries the
    // effect's completed payload type, so case dispatch and field access
    // through it type-check.
    for line in rule.body.text.lines() {
        let Some(rest) = line.trim().strip_prefix("after ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let Some(binding) = words.next() else {
            continue;
        };
        let Some(predicate) = words.next() else {
            continue;
        };
        // `after p reaches "<name>" as m` (Family C): the milestone name sits
        // between the predicate and `as`, so the alias lands one token later.
        // Type `m` to the child's declared milestone payload class.
        if predicate == "reaches" {
            let Some(quoted) = words.next() else {
                continue;
            };
            let milestone = quoted.trim_matches('"');
            let (Some("as"), Some(alias)) = (words.next(), words.next()) else {
                continue;
            };
            let alias = alias.trim_end_matches('{').trim();
            if alias.is_empty() {
                continue;
            }
            if let Some(class) = milestone_payload_class(rule, binding, milestone, semantic) {
                if !class.is_empty() {
                    binding_types.insert(alias.to_owned(), class);
                }
            }
            continue;
        }
        // `times out` is the only two-token predicate; skip its second word so
        // the `as <alias>` clause lines up.
        if predicate == "times" && words.next() != Some("out") {
            continue;
        }
        let (Some(keyword), Some(alias)) = (words.next(), words.next()) else {
            continue;
        };
        if keyword != "as" {
            continue;
        }
        let alias = alias.trim_end_matches('{').trim();
        if alias.is_empty() {
            continue;
        }
        // Bind the alias to the terminal payload schema that matches the
        // predicate, consistent with the case-tag payload schemas
        // (terminal_payload_schema_for_tag): `times out` -> `TerminalTimedOut`,
        // `cancelled` -> `TerminalCancelled`. Other predicates carry the
        // effect's completed payload schema.
        match predicate {
            "times" => {
                binding_types.insert(alias.to_owned(), "TerminalTimedOut".to_owned());
            }
            "cancelled" => {
                binding_types.insert(alias.to_owned(), "TerminalCancelled".to_owned());
            }
            // DR-0032: the `fails` branch binds the EffectError BASE — every
            // effect's `.failed` fact now carries `value: {reason, summary,
            // effect_id, run_id, kind}` (the `TerminalFailed` base schema). Per-kind
            // failure extras (exec `exit_code`, …) are deferred behind static
            // effect-kind narrowing (a future variant), so the base is what is
            // typed today. This replaces the prior untyped no-op.
            "fails" => {
                binding_types.insert(alias.to_owned(), "TerminalFailed".to_owned());
            }
            _ => {
                if let Some(IrType::Ref(schema)) = effect_payload_types.get(binding) {
                    binding_types.insert(alias.to_owned(), schema.clone());
                }
            }
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
    // Complete value-position root set: typed bindings plus every binding NAME
    // the body introduces (AST-collected, so multi-line-prompt `tell`/`exec`
    // results and `case` payloads are covered, which `binding_types` omits).
    let mut known_roots: BTreeSet<String> = binding_types.keys().cloned().collect();
    collect_all_binding_names(&body_ast.statements, &mut known_roots);
    validate_record_blocks(rule, semantic, &binding_types, &known_roots, diagnostics);
    validate_effect_payloads(rule, semantic, &binding_types, &known_roots, diagnostics);
    validate_effect_field_roots(rule, &body_ast.statements, &known_roots, diagnostics);
    validate_workflow_invocations(rule, semantic, &binding_types, &known_roots, diagnostics);
    validate_milestone_statements(rule, semantic, diagnostics);
    let mut block_stack: Vec<BlockFrame> = Vec::new();
    let mut misplaced_effect_bindings = BTreeSet::new();
    seed_ast_only_effect_bindings(&body_ast.statements, &mut seen_bindings, &mut binding_types);
    validate_body_effect_operands(
        rule,
        &body_ast.statements,
        semantic,
        &binding_types,
        diagnostics,
    );
    validate_coordination_discipline(rule, &body_ast.statements, diagnostics);
    // `redact <source> keep [..] as <out>`: the source must resolve to a known
    // schema and every kept field must exist on it (fail-closed).
    validate_redactions(
        rule,
        &body_ast.statements,
        semantic,
        &binding_types,
        diagnostics,
    );
    // Family B read-narrowing: a presence-conditioned field is readable only inside a
    // matching `case <root>.<disc>` arm (starts with nothing allowed at the rule top).
    validate_conditioned_field_reads(
        rule,
        &body_ast.statements,
        semantic,
        &binding_types,
        &BTreeSet::new(),
        diagnostics,
    );
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
            diagnostics.push(Diagnostic { related: Vec::new(),
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
        validate_rule_prompt_content_type_annotation(rule, line, diagnostics);

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
                    related: Vec::new(),
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
            if let Some(alias) = binding_after_as(line) {
                validate_binding_name(rule, &alias, rule.body.span, diagnostics);
            }
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
                        diagnostics.push(Diagnostic { related: Vec::new(),
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
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: rule.body.span,
                        message: format!(
                            "rule `{}` has unsupported `after` dependency predicate",
                            rule.name.name
                        ),
                        suggestion: Some(
                            "use `after name succeeds`, `after name fails`, `after name completes`, `after name times out`, or `after name cancelled`"
                                .to_owned(),
                        ),
                    });
                }
            }
            continue;
        }

        if let Some((schema, _)) = parse_record_start(line) {
            if is_observer_only_schema(&schema) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` cannot record kernel-owned terminal schema `{schema}`",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "the terminal family (`TerminalFailed`/`TerminalTimedOut`/`TerminalCancelled`) is produced only by the kernel; to fail this workflow use `fail <failure> { ... }`, and to react to an effect terminal use `after <effect> fails/times out/cancels as f`"
                            .to_owned(),
                    ),
                });
            } else if !semantic.schemas.class_exists(&schema) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
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
            validate_agent_tell_target(
                rule,
                line,
                &kind,
                semantic,
                &binding_types,
                &known_roots,
                diagnostics,
            );
            anonymous_effects += 1;
            let id = binding
                .clone()
                .unwrap_or_else(|| format!("effect{anonymous_effects}"));
            if let Some(binding) = &binding {
                validate_binding_name(rule, binding, rule.body.span, diagnostics);
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
                construct_use: None,
                idempotency_key,
                span: rule.body.span,
                timeout_seconds: None,
                // The line-scanner result is overwritten by collect_effects_from_ast
                // below (which carries the real grants); empty here is fine.
                access_grants: Vec::new(),
                resource: None,
                agent: None,
                endorsed: false,
                declassified: false,
                selected_by: None,
            });
        }
    }

    let (ast_effects, ast_dependencies) =
        collect_effects_from_ast(&body_ast.statements, &rule.name.name);
    metadata.effects = ast_effects;
    metadata.dependencies = ast_dependencies;

    // `exec ... -> each Schema` produces one `Schema` fact per stream element
    // (spec/json-ingestion.md) — a fact write for liveness and effect-graph
    // analysis, like `record`.
    push_ingest_fact_writes(&body_ast.statements, &mut metadata.fact_writes);

    metadata.fact_reads.sort();
    metadata.fact_reads.dedup();
    sort_projection_reads(&mut metadata.projection_reads);
    metadata.fact_writes.sort();
    metadata.fact_writes.dedup();
    metadata.fact_consumes.sort();
    metadata.fact_consumes.dedup();
    metadata.terminal_outputs = terminal_metadata.outputs;
    metadata.terminal_branches = terminal_metadata.branches;
    collect_terminal_complete_bindings(&body_ast.statements, &mut metadata.terminal_completes);
    metadata.terminal_completes.sort();
    metadata.terminal_completes.dedup();
    collect_redaction_metadata(
        &body_ast.statements,
        &binding_types,
        &mut metadata.redactions,
    );
    collect_bounded_egresses(
        &body_ast.statements,
        &binding_types,
        &mut metadata.bounded_egresses,
    );
    let mut egress_reads = Vec::new();
    collect_egress_payload_reads(&body_ast.statements, &mut egress_reads);
    for (sink, roots) in egress_reads {
        metadata
            .egress_payload_reads
            .entry(sink)
            .or_default()
            .extend(roots);
    }
    collect_complete_field_reads(&body_ast.statements, &mut metadata.complete_field_reads);
    metadata
}

/// For each `complete <binding> { field: <expr>, … }` egress in a rule body
/// (recursing into nested blocks), the binding roots EACH result field references,
/// as `binding -> field -> {roots}`. A `Shorthand` field (`complete result from src
/// { f }`) resolves to the terminal's `from` binding. Unlike
/// `collect_egress_payload_reads` (which joins a sink's fields), this keeps fields
/// separate so the IFC engine can compute a per-field flow signature (DR-0030 X2
/// v2). Union across branches (a field completed in two arms references the union).
fn collect_complete_field_reads(
    statements: &[body::BodyStmt],
    out: &mut BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Terminal(terminal) if terminal.kind == body::TerminalKind::Complete => {
                let per_field = out.entry(terminal.name.clone()).or_default();
                for field in &terminal.fields {
                    let mut roots = BTreeSet::new();
                    match &field.value {
                        body::FieldValue::Shorthand => {
                            if let Some(root) = &terminal.from {
                                roots.insert(root.clone());
                            }
                        }
                        body::FieldValue::Expr { expr, .. } => {
                            collect_expr_binding_roots(expr, &mut roots)
                        }
                        body::FieldValue::Nested { fields, .. } => collect_payload_field_roots(
                            fields,
                            terminal.from.as_deref(),
                            &mut roots,
                        ),
                    }
                    per_field
                        .entry(field.name.clone())
                        .or_default()
                        .extend(roots);
                }
            }
            body::BodyStmt::After(after) => collect_complete_field_reads(&after.body, out),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_complete_field_reads(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_complete_field_reads(&branch.then_body, out);
                if let Some(else_body) = &branch.else_body {
                    collect_complete_field_reads(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => collect_complete_field_reads(&handler.body, out),
            _ => {}
        }
    }
}

/// Collects the `redact <source> keep [..] as <out>` projections of a rule body
/// (recursing into nested blocks) as IFC value-flow metadata, preserving body
/// order so a chained redaction's source resolves against the earlier projection.
/// `binding_types` (the rule's fully-resolved binding -> schema map, including
/// redaction outputs via their synthetic class) supplies each source's schema so
/// the IFC engine can derive the projection's per-field label.
fn collect_redaction_metadata(
    statements: &[body::BodyStmt],
    binding_types: &BTreeMap<String, String>,
    out: &mut Vec<IrRedaction>,
) {
    let mut redacts = Vec::new();
    collect_redact_effects(statements, &mut redacts);
    for (source, keep, binding, _span) in redacts {
        out.push(IrRedaction {
            source: source.to_owned(),
            keep: keep.to_vec(),
            binding: binding.to_owned(),
            source_schema: binding_types.get(source).cloned(),
        });
    }
}

/// Collect the bounded-type projection egresses (`record <T> from <src>`) of a rule
/// body (recursing into nested blocks). A `record T from src` keeps exactly `T`'s
/// declared fields, copied from `src`, so the IFC engine can govern it by the kept
/// fields' per-field labels (sourced from `src`'s schema) — the "bounded-type"
/// auto-redaction reading. Only recorded when the source schema resolves and the
/// target type is declared; otherwise the egress stays conservative.
/// Records a bounded-type projection egress for a PURE `from` projection — a
/// `from <src>` egress every field of which is a shorthand copy of `src.<name>`.
/// The runtime materializes exactly these fields, so the kept set is their names,
/// governed by `src`'s schema per-field labels. `None` source schema, no `from`, or
/// any explicit value field → not a clean projection, so it stays conservative
/// (handled by the whole-read join). `sink` is the engine sink string
/// (`fact:<Schema>` for a record, the completed binding for a `complete`).
fn push_bounded_projection(
    from: Option<&str>,
    fields: &[body::FieldAssign],
    sink: String,
    binding_types: &BTreeMap<String, String>,
    out: &mut Vec<IrBoundedEgress>,
) {
    let Some(source_schema) = from.and_then(|src| binding_types.get(src)) else {
        return;
    };
    if fields.is_empty()
        || !fields
            .iter()
            .all(|field| matches!(field.value, body::FieldValue::Shorthand))
    {
        return;
    }
    out.push(IrBoundedEgress {
        sink,
        source_schema: source_schema.clone(),
        keep: fields.iter().map(|field| field.name.clone()).collect(),
    });
}

fn push_bounded_record(
    record: &body::RecordStmt,
    binding_types: &BTreeMap<String, String>,
    out: &mut Vec<IrBoundedEgress>,
) {
    push_bounded_projection(
        record.from.as_deref(),
        &record.fields,
        format!("fact:{}", record.schema),
        binding_types,
        out,
    );
}

fn collect_bounded_egresses(
    statements: &[body::BodyStmt],
    binding_types: &BTreeMap<String, String>,
    out: &mut Vec<IrBoundedEgress>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Record(record) => push_bounded_record(record, binding_types, out),
            body::BodyStmt::Done {
                replacement: Some(record),
                ..
            } => push_bounded_record(record, binding_types, out),
            // `complete <T> from <src> { … }`: bounded-type projection to the invoker.
            // The engine sink for a complete is the completed binding (its name).
            body::BodyStmt::Terminal(terminal)
                if terminal.kind == body::TerminalKind::Complete && terminal.from.is_some() =>
            {
                push_bounded_projection(
                    terminal.from.as_deref(),
                    &terminal.fields,
                    terminal.name.clone(),
                    binding_types,
                    out,
                );
            }
            body::BodyStmt::After(after) => {
                collect_bounded_egresses(&after.body, binding_types, out)
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_bounded_egresses(&branch.body, binding_types, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_bounded_egresses(&branch.then_body, binding_types, out);
                if let Some(else_body) = &branch.else_body {
                    collect_bounded_egresses(else_body, binding_types, out);
                }
            }
            body::BodyStmt::Handler(handler) => {
                collect_bounded_egresses(&handler.body, binding_types, out)
            }
            _ => {}
        }
    }
}

/// Collect EVERY binding root referenced by an expression, for the information-flow
/// value-flow engine. SOUNDNESS: a missed reference under-approximates a payload's
/// sources — so this over-collects (an over-collected name that is not a relevant
/// binding contributes nothing downstream). It walks every `Expr` variant and, for
/// string literals, extracts `{{ … }}` interpolation roots (those refs live as raw
/// text inside the literal, not as structured nodes). A bare identifier parses as
/// `Literal(Ident)`, a dotted ref as `Path` — both are roots.
fn collect_expr_binding_roots(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Literal(ExprLiteral::String(text)) => collect_template_binding_roots(text, out),
        Expr::Literal(ExprLiteral::Ident(name)) => {
            out.insert(name.clone());
        }
        Expr::Literal(ExprLiteral::Number(_) | ExprLiteral::Bool(_) | ExprLiteral::Null) => {}
        Expr::Path(segments) => {
            if let Some(root) = segments.first() {
                out.insert(root.clone());
            }
        }
        Expr::Index { target, key } => {
            collect_expr_binding_roots(target, out);
            collect_expr_binding_roots(key, out);
        }
        Expr::Array(items) => {
            for item in items {
                collect_expr_binding_roots(item, out);
            }
        }
        Expr::Object(fields) => {
            for field in fields {
                collect_expr_binding_roots(&field.value, out);
            }
        }
        Expr::Unary { expr, .. } => collect_expr_binding_roots(expr, out),
        Expr::Binary { left, right, .. } => {
            collect_expr_binding_roots(left, out);
            collect_expr_binding_roots(right, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_expr_binding_roots(arg, out);
            }
        }
        Expr::Query { head, guard, .. } => {
            out.insert(head.clone());
            if let Some(guard) = guard {
                collect_expr_binding_roots(guard, out);
            }
        }
    }
}

/// Collect every binding root inside `{{ … }}` interpolations of a string. Unlike
/// `interpolation_roots` (first root per interpolation), value-flow needs EVERY
/// root, so `{{ a.b + c.d }}` yields both `a` and `c`. Each interpolation body is
/// parsed and walked; an unparseable body falls back to a conservative identifier
/// scan (over-collection is sound).
fn collect_template_binding_roots(text: &str, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            break;
        };
        let body = after_open[..close].trim();
        if let Ok(expr) = parse_expression(body) {
            collect_expr_binding_roots(&expr, out);
        } else {
            for token in body.split(|ch: char| !ch.is_alphanumeric() && ch != '_') {
                if token
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| is_ident_start(*byte))
                {
                    out.insert(token.to_owned());
                }
            }
        }
        rest = &after_open[close + 2..];
    }
}

/// Collect the binding roots a payload field list references, threading the
/// enclosing `from <binding>` source so a `Shorthand` field resolves to it.
fn collect_payload_field_roots(
    fields: &[body::FieldAssign],
    from_binding: Option<&str>,
    out: &mut BTreeSet<String>,
) {
    for field in fields {
        match &field.value {
            body::FieldValue::Shorthand => {
                if let Some(root) = from_binding {
                    out.insert(root.to_owned());
                }
            }
            body::FieldValue::Expr { expr, .. } => collect_expr_binding_roots(expr, out),
            body::FieldValue::Nested { fields, .. } => {
                collect_payload_field_roots(fields, from_binding, out)
            }
        }
    }
}

/// For each egress sink in a rule body (recursing into nested blocks), the set of
/// binding roots its payload references, keyed by the sink string the IFC engine
/// uses: a `complete <binding>` by its binding, a `record <Schema>` by
/// `fact:<Schema>`. Surfaced so the engine can recognize a FULLY-REDACTED egress —
/// one whose payload references only redaction outputs (and constants) — and
/// govern it by the projection's per-field label instead of the rule's whole read
/// set. A `record <Schema> from <binding>` references that `from` binding too (its
/// fields are copied). A sink with no recorded entry references nothing resolvable.
fn collect_egress_payload_reads(
    statements: &[body::BodyStmt],
    out: &mut Vec<(String, BTreeSet<String>)>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Terminal(terminal) if terminal.kind == body::TerminalKind::Complete => {
                let mut roots = BTreeSet::new();
                collect_payload_field_roots(&terminal.fields, None, &mut roots);
                out.push((terminal.name.clone(), roots));
            }
            body::BodyStmt::Record(record) => out.push(record_payload_reads(record)),
            // `done <b> -> record <Schema> { … }` is also a record egress.
            body::BodyStmt::Done {
                replacement: Some(record),
                ..
            } => out.push(record_payload_reads(record)),
            // `send via <channel> { text … }` egresses to the channel; its payload
            // fields (text/markdown/thread_id) are construct-use source text. Keyed by
            // the channel (the engine's send sink, per `resource_for_body`).
            body::BodyStmt::Effect(effect) => {
                if let body::BodyEffectKind::ConstructCapabilityCall {
                    keyword, fields, ..
                } = &effect.kind
                {
                    if keyword == "send" {
                        if let Some(reads) = send_payload_reads(fields) {
                            out.push(reads);
                        }
                    }
                }
            }
            body::BodyStmt::After(after) => collect_egress_payload_reads(&after.body, out),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_egress_payload_reads(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_egress_payload_reads(&branch.then_body, out);
                if let Some(else_body) = &branch.else_body {
                    collect_egress_payload_reads(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => collect_egress_payload_reads(&handler.body, out),
            _ => {}
        }
    }
}

/// The channel sink key and the binding roots a `send` payload references. The
/// payload fields (`text`/`markdown`/`thread_id`) carry expression SOURCE TEXT, so
/// each is parsed and walked (a string literal's `{{ … }}` interpolations count);
/// the `channel` field names the sink. `None` if no channel is present.
fn send_payload_reads(fields: &[body::ConstructUseField]) -> Option<(String, BTreeSet<String>)> {
    let channel = fields
        .iter()
        .find(|field| field.name == "channel")
        .map(|field| field.source.clone())?;
    let mut roots = BTreeSet::new();
    for field in fields.iter().filter(|field| field.name != "channel") {
        if let Ok(expr) = parse_expression(&field.source) {
            collect_expr_binding_roots(&expr, &mut roots);
        } else {
            // Unparseable source: scan its interpolations conservatively.
            collect_template_binding_roots(&field.source, &mut roots);
        }
    }
    Some((channel, roots))
}

/// The `fact:<Schema>` sink key and the binding roots a `record` payload
/// references — its explicit field values plus, for `record <S> from <b>`, the
/// copied-from binding `b`.
fn record_payload_reads(record: &body::RecordStmt) -> (String, BTreeSet<String>) {
    let mut roots = BTreeSet::new();
    if let Some(from) = &record.from {
        roots.insert(from.clone());
    }
    collect_payload_field_roots(&record.fields, record.from.as_deref(), &mut roots);
    (format!("fact:{}", record.schema), roots)
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
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, IrType> {
    let mut payloads = BTreeMap::new();
    for statement in effect_payload_statements(&rule.body.text) {
        let line = statement.trim();
        let Some((kind, Some(binding))) = parse_effect_line(line) else {
            continue;
        };
        let payload = terminal_completed_payload_type(line, &kind, semantic);
        // A binding name keys the per-rule payload map, so reusing it for two effects
        // with DIFFERENT result types makes `after <binding> …` ambiguous (§5.5).
        // Same-type reuse (and mutually-exclusive `case` arms, which never both run)
        // is harmless and left alone.
        match payloads.get(&binding) {
            Some(existing) if existing != &payload => {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` reuses effect binding `{binding}` for effects with conflicting result types",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "give each effect a distinct binding — `as {binding}` is reused with a different result type, so `after {binding} …` is ambiguous"
                    )),
                });
            }
            Some(_) => {}
            None => {
                payloads.insert(binding, payload);
            }
        }
    }

    payloads
}

fn terminal_completed_payload_type(
    line: &str,
    kind: &IrEffectKind,
    semantic: &SemanticContext,
) -> IrType {
    match kind {
        IrEffectKind::Coerce => parse_coerce_call_name(line)
            .and_then(|name| semantic.coerce_outputs.get(name))
            .cloned()
            .map(lower_type)
            .unwrap_or_else(terminal_unknown_payload_type),
        IrEffectKind::LoftClaim => IrType::Ref("LoftClaim".to_owned()),
        IrEffectKind::HumanAsk => IrType::Ref("HumanAnswer".to_owned()),
        IrEffectKind::AgentTell => IrType::Ref("AgentTurn".to_owned()),
        IrEffectKind::CapabilityCall
        | IrEffectKind::EventEmit
        | IrEffectKind::WorkflowInvoke
        | IrEffectKind::TimerWait
        | IrEffectKind::ExecCommand
        | IrEffectKind::QueueFile
        | IrEffectKind::QueueClaim
        | IrEffectKind::QueueRelease
        | IrEffectKind::QueueFinish
        | IrEffectKind::LeaseAcquire
        | IrEffectKind::LedgerAppend
        | IrEffectKind::CounterConsume
        | IrEffectKind::EventNotify
        | IrEffectKind::FileRead
        | IrEffectKind::FileWrite
        | IrEffectKind::FileImport
        | IrEffectKind::FileExport => terminal_unknown_payload_type(),
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
            // The IR pattern is the variant name; the payload binding is a
            // branch-scope concern, not part of dispatch identity.
            let (variant, _) = sum_case_pattern_parts(pattern);
            Some(IrCasePattern::EnumVariant(variant.to_owned()))
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
    // Sum types: `Variant as b` binds the payload typed as the generated
    // `<Enum>.<Variant>` class (spec/sum-types.md).
    if let TypeSyntax::Ref { name } = scrutinee_type {
        if semantic.schemas.enums.contains_key(&name.name) {
            let (variant, binding) = sum_case_pattern_parts(pattern);
            let binding = binding?;
            let generated = format!("{}.{variant}", name.name);
            if binding.is_empty() || !semantic.schemas.class_exists(&generated) {
                return None;
            }
            return Some((binding.to_owned(), generated));
        }
    }
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
    // Binding is `Tag as binding` (Stage 1b: the space form `Tag binding` is gone).
    let second = parts.next();
    let binding = match second {
        Some("as") => parts.next().map(str::to_owned),
        Some(_) => return (tag, None),
        None => None,
    };
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
        is_key: false,
        presence_condition: None,
        span: SourceSpan { start: 0, end: 0 },
    }
}

/// Lower a `tell`'s parsed turn-access grants to IR. Empty for any other effect kind.
fn ir_access_grants_for_body(kind: &body::BodyEffectKind) -> Vec<IrAccessGrant> {
    match kind {
        body::BodyEffectKind::Tell { access_grants, .. } => access_grants
            .iter()
            .map(|grant| IrAccessGrant {
                resource: grant.resource.clone(),
                operations: grant
                    .operations
                    .iter()
                    .map(|op| IrAccessGrantOp {
                        operation: op.operation.clone(),
                        target: op.target.clone(),
                        globs: op.globs.clone(),
                    })
                    .collect(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ir_effect_kind_for_body(kind: &body::BodyEffectKind) -> IrEffectKind {
    match kind {
        body::BodyEffectKind::Tell { .. } => IrEffectKind::AgentTell,
        body::BodyEffectKind::Coerce { .. } | body::BodyEffectKind::Decide { .. } => {
            IrEffectKind::Coerce
        }
        body::BodyEffectKind::AskHuman { .. } => IrEffectKind::HumanAsk,
        body::BodyEffectKind::Call { .. }
        | body::BodyEffectKind::ConstructCapabilityCall { .. } => IrEffectKind::CapabilityCall,
        body::BodyEffectKind::Invoke { .. } => IrEffectKind::WorkflowInvoke,
        body::BodyEffectKind::Timer { .. } => IrEffectKind::TimerWait,
        body::BodyEffectKind::Exec { .. } => IrEffectKind::ExecCommand,
        body::BodyEffectKind::QueueFile { .. } => IrEffectKind::QueueFile,
        body::BodyEffectKind::QueueClaim { .. } => IrEffectKind::QueueClaim,
        body::BodyEffectKind::QueueRelease { .. } => IrEffectKind::QueueRelease,
        body::BodyEffectKind::QueueFinish { .. } => IrEffectKind::QueueFinish,
        body::BodyEffectKind::LeaseAcquire { .. } => IrEffectKind::LeaseAcquire,
        body::BodyEffectKind::LedgerAppend { .. } => IrEffectKind::LedgerAppend,
        body::BodyEffectKind::CounterConsume { .. } => IrEffectKind::CounterConsume,
        body::BodyEffectKind::Notify { .. } => IrEffectKind::EventNotify,
        body::BodyEffectKind::FileRead { .. } => IrEffectKind::FileRead,
        body::BodyEffectKind::FileWrite { .. } => IrEffectKind::FileWrite,
        body::BodyEffectKind::FileImport { .. } => IrEffectKind::FileImport,
        body::BodyEffectKind::FileExport { .. } => IrEffectKind::FileExport,
    }
}

/// The agent a `tell` addresses, surfaced for information-flow analysis of the
/// turn's egress to the agent's provider. `None` for non-`tell` effects.
fn agent_for_body(kind: &body::BodyEffectKind) -> Option<String> {
    match kind {
        body::BodyEffectKind::Tell { target, .. } => Some(target.clone()),
        _ => None,
    }
}

/// Whether an effect carries the `endorsed` source marker (I-IFC3) — a `coerce` the
/// author declared an integrity-raising crossing.
fn endorsed_for_body(kind: &body::BodyEffectKind) -> bool {
    matches!(kind, body::BodyEffectKind::Coerce { endorsed: true, .. })
}

/// Whether an effect carries the `declassified` source marker (I-IFC3) — a `coerce`
/// the author declared a confidentiality-lowering crossing.
fn declassified_for_body(kind: &body::BodyEffectKind) -> bool {
    matches!(
        kind,
        body::BodyEffectKind::Coerce {
            declassified: true,
            ..
        }
    )
}

/// The named resource a direct file/channel effect touches, surfaced for
/// information-flow analysis. `None` for effects with no named resource.
fn resource_for_body(kind: &body::BodyEffectKind) -> Option<String> {
    match kind {
        body::BodyEffectKind::FileRead { store, .. }
        | body::BodyEffectKind::FileWrite { store, .. }
        | body::BodyEffectKind::FileImport { store, .. }
        | body::BodyEffectKind::FileExport { store, .. } => Some(store.clone()),
        // `send via <channel>` carries the channel as a construct field.
        body::BodyEffectKind::ConstructCapabilityCall {
            keyword, fields, ..
        } if keyword == "send" => fields
            .iter()
            .find(|field| field.name == "channel")
            .map(|field| field.source.clone()),
        // `emit signal <name> to <peer>` touches the signal port `signal:<name>` (the
        // emit-port door, DR-0027 E6/H8); surfaced so the IFC checker can carry the
        // emitter's label to the receiver and enumerate the port in the surface.
        body::BodyEffectKind::Notify { event, .. } => Some(format!("signal:{event}")),
        _ => None,
    }
}

fn construct_use_for_body(kind: &body::BodyEffectKind) -> Option<IrConstructUse> {
    match kind {
        body::BodyEffectKind::ConstructCapabilityCall {
            keyword,
            target_capability,
            ..
        } => Some(IrConstructUse {
            keyword: keyword.clone(),
            scope: "rule_body".to_owned(),
            construct_family: "effect_operation".to_owned(),
            lowering_target: "capability_call".to_owned(),
            target_capability: target_capability.clone(),
        }),
        _ => None,
    }
}

fn is_ast_only_effect_kind(kind: &body::BodyEffectKind) -> bool {
    // `send via <channel> { … } as x` closes its `as` on the block line (unlike
    // `recall`, whose `as` is inline), so the line scanner cannot see the binding;
    // seed it from the AST. Other `ConstructCapabilityCall`s (e.g. `recall`) are
    // line-visible and must NOT be treated as AST-only.
    if let body::BodyEffectKind::ConstructCapabilityCall { keyword, .. } = kind {
        return keyword == "send";
    }
    matches!(
        kind,
        body::BodyEffectKind::Timer { .. }
            | body::BodyEffectKind::Exec { .. }
            | body::BodyEffectKind::Decide { .. }
            | body::BodyEffectKind::QueueFile { .. }
            | body::BodyEffectKind::QueueClaim { .. }
            | body::BodyEffectKind::QueueRelease { .. }
            | body::BodyEffectKind::QueueFinish { .. }
            | body::BodyEffectKind::LeaseAcquire { .. }
            | body::BodyEffectKind::LedgerAppend { .. }
            | body::BodyEffectKind::CounterConsume { .. }
            | body::BodyEffectKind::Notify { .. }
            // `write`/`export` put their `as <binding>` on the block's closing
            // line, so the line-based scanner cannot see it; seed it from the AST
            // so `after <binding>` blocks and sequence checks resolve.
            | body::BodyEffectKind::FileWrite { .. }
            | body::BodyEffectKind::FileExport { .. }
    )
}

/// Bindings introduced by AST-only effect kinds are unknown to the
/// line-based scanner; seed them so sequence checks and `after` blocks see
/// them. Binding types for typed outputs are registered where known.
fn seed_ast_only_effect_bindings(
    statements: &[body::BodyStmt],
    seen_bindings: &mut BTreeSet<String>,
    binding_types: &mut BTreeMap<String, String>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) if is_ast_only_effect_kind(&effect.kind) => {
                if let Some(binding) = &effect.binding {
                    seen_bindings.insert(binding.clone());
                    let _ = binding_types;
                }
            }
            body::BodyStmt::After(after) => {
                seed_ast_only_effect_bindings(&after.body, seen_bindings, binding_types)
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    seed_ast_only_effect_bindings(&branch.body, seen_bindings, binding_types);
                }
            }
            body::BodyStmt::Branch(branch) => {
                seed_ast_only_effect_bindings(&branch.then_body, seen_bindings, binding_types);
                if let Some(else_body) = &branch.else_body {
                    seed_ast_only_effect_bindings(else_body, seen_bindings, binding_types);
                }
            }
            body::BodyStmt::Handler(handler) => {
                seed_ast_only_effect_bindings(&handler.body, seen_bindings, binding_types)
            }
            _ => {}
        }
    }
}

/// Derives effect nodes and dependency edges from the body AST, in document
/// order, with ids and idempotency keys identical to the historical
/// line-scanner derivation.
/// Collect the output bindings a rule `complete`s, recursing through the body's
/// nested blocks (after / case / branch / handler). A `complete <binding> {…}` is the
/// workflow's output to its invoker; the IFC checker treats it as an egress sink at
/// the invoker boundary (DR-0030 X2). `fail`/`flowfail` terminals are NOT collected —
/// they carry an error to the runtime, not a value to the invoker.
fn collect_terminal_complete_bindings(statements: &[body::BodyStmt], out: &mut Vec<String>) {
    for statement in statements {
        match statement {
            body::BodyStmt::Terminal(terminal) if terminal.kind == body::TerminalKind::Complete => {
                out.push(terminal.name.clone());
            }
            body::BodyStmt::After(after) => collect_terminal_complete_bindings(&after.body, out),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_terminal_complete_bindings(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_terminal_complete_bindings(&branch.then_body, out);
                if let Some(else_body) = branch.else_body.as_deref() {
                    collect_terminal_complete_bindings(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => {
                collect_terminal_complete_bindings(&handler.body, out);
            }
            _ => {}
        }
    }
}

fn collect_effects_from_ast(
    statements: &[body::BodyStmt],
    rule_name: &str,
) -> (Vec<IrEffectNode>, Vec<IrEffectDependency>) {
    let mut effects = Vec::new();
    let mut dependencies = Vec::new();
    let mut counter = 0usize;
    let mut after_stack: Vec<(String, DependencyPredicate)> = Vec::new();
    let mut case_stack: Vec<(String, String)> = Vec::new();
    walk_effects(
        statements,
        rule_name,
        &mut counter,
        &mut after_stack,
        &mut case_stack,
        &mut effects,
        &mut dependencies,
    );
    (effects, dependencies)
}

fn walk_effects(
    statements: &[body::BodyStmt],
    rule_name: &str,
    counter: &mut usize,
    after_stack: &mut Vec<(String, DependencyPredicate)>,
    case_stack: &mut Vec<(String, String)>,
    effects: &mut Vec<IrEffectNode>,
    dependencies: &mut Vec<IrEffectDependency>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                *counter += 1;
                let id = effect
                    .binding
                    .clone()
                    .unwrap_or_else(|| format!("effect{counter}"));
                let kind = ir_effect_kind_for_body(&effect.kind);
                for (upstream, predicate) in after_stack.iter() {
                    dependencies.push(IrEffectDependency {
                        upstream: upstream.clone(),
                        predicate: predicate.clone(),
                        downstream: id.clone(),
                    });
                }
                let idempotency_key =
                    effect_idempotency_key(rule_name, &id, &kind, &effect.binding);
                let mut required_capabilities = effect.requires.clone();
                match &effect.kind {
                    body::BodyEffectKind::Call { capability, .. } => {
                        required_capabilities.push(capability.clone());
                    }
                    body::BodyEffectKind::ConstructCapabilityCall {
                        target_capability, ..
                    } => {
                        required_capabilities.push(target_capability.clone());
                    }
                    _ => {}
                }
                required_capabilities.sort();
                required_capabilities.dedup();
                let construct_use = construct_use_for_body(&effect.kind);
                let access_grants = ir_access_grants_for_body(&effect.kind);
                let resource = resource_for_body(&effect.kind);
                let agent = agent_for_body(&effect.kind);
                let endorsed = endorsed_for_body(&effect.kind);
                let declassified = declassified_for_body(&effect.kind);
                effects.push(IrEffectNode {
                    id,
                    kind,
                    binding: effect.binding.clone(),
                    required_capabilities,
                    construct_use,
                    idempotency_key,
                    span: effect.span,
                    timeout_seconds: effect.timeout_seconds,
                    access_grants,
                    resource,
                    agent,
                    endorsed,
                    declassified,
                    selected_by: case_stack.last().cloned(),
                });
            }
            body::BodyStmt::After(after) => {
                let predicate = match after.predicate {
                    body::AfterPredicate::Succeeds => DependencyPredicate::Succeeds,
                    body::AfterPredicate::Fails => DependencyPredicate::Fails,
                    // `times out` / `cancelled` are distinct non-success terminal
                    // statuses, so the downstream effect releases only on that
                    // specific status (mirroring succeeds/fails), not on any
                    // terminal.
                    body::AfterPredicate::TimedOut => DependencyPredicate::TimedOut,
                    body::AfterPredicate::Cancelled => DependencyPredicate::Cancelled,
                    // Coordination outcomes are completion-valued: the downstream
                    // depends on the op reaching a terminal state; the outcome
                    // variant selects the arm at lowering.
                    body::AfterPredicate::Completes
                    | body::AfterPredicate::Held
                    | body::AfterPredicate::Contended
                    | body::AfterPredicate::Ok
                    | body::AfterPredicate::Over => DependencyPredicate::Completes,
                    // `reaches "<name>"` (Family C) is completion-shaped for the
                    // construct-graph provenance edge; the milestone-specific
                    // gating happens at runtime against the
                    // `workflow.invoke.reached:<name>` fact (text-keyed, see
                    // `fact_matches_after_predicate`), so this IR predicate is
                    // metadata only.
                    body::AfterPredicate::Reaches => DependencyPredicate::Completes,
                };
                after_stack.push((after.binding.clone(), predicate));
                walk_effects(
                    &after.body,
                    rule_name,
                    counter,
                    after_stack,
                    case_stack,
                    effects,
                    dependencies,
                );
                after_stack.pop();
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    // Record the selector: an effect in this arm is gated by
                    // `case <scrutinee> { <pattern> => … }` (DR §7.4).
                    case_stack.push((case.scrutinee.clone(), branch.pattern.clone()));
                    walk_effects(
                        &branch.body,
                        rule_name,
                        counter,
                        after_stack,
                        case_stack,
                        effects,
                        dependencies,
                    );
                    case_stack.pop();
                }
            }
            body::BodyStmt::Branch(branch) => {
                walk_effects(
                    &branch.then_body,
                    rule_name,
                    counter,
                    after_stack,
                    case_stack,
                    effects,
                    dependencies,
                );
                if let Some(else_body) = &branch.else_body {
                    walk_effects(
                        else_body,
                        rule_name,
                        counter,
                        after_stack,
                        case_stack,
                        effects,
                        dependencies,
                    );
                }
            }
            body::BodyStmt::Handler(handler) => {
                walk_effects(
                    &handler.body,
                    rule_name,
                    counter,
                    after_stack,
                    case_stack,
                    effects,
                    dependencies,
                );
            }
            _ => {}
        }
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
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some((function_name, args)) = parse_coerce_call(line) else {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: rule.body.span,
            message: format!("rule `{}` has malformed coerce call", rule.name.name),
            suggestion: Some("write `coerce functionName(arg, ...) as name`".to_owned()),
        });
        return;
    };
    let Some(params) = semantic.coerce_params.get(function_name) else {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
            related: Vec::new(),
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
        // Dangling-root check (mirrors record/terminal value validation): an arg
        // whose root is not a known binding is a typo/unbound reference, which the
        // type-checker below accepts leniently.
        if let Some(root) = dangling_value_root(arg, known_roots) {
            diagnostics.push(Diagnostic { related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` has unknown binding `{root}` in coerce `{function_name}` argument",
                    rule.name.name
                ),
                suggestion: Some(
                    "reference a binding from a `when ... as name` clause, an effect `as` binding, or a `case` pattern"
                        .to_owned(),
                ),
            });
        }
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
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in effect_payload_statements(&rule.body.text) {
        let trimmed = statement.trim();
        if trimmed.starts_with("coerce ") {
            validate_coerce_call(
                rule,
                trimmed,
                semantic,
                binding_types,
                known_roots,
                diagnostics,
            );
        } else if trimmed.starts_with("claim ") && trimmed.contains(" with ") {
            // Legacy loft form only; plain `claim <item>` is a queue verb
            // validated by the body AST.
            validate_loft_claim_payload(rule, trimmed, semantic, binding_types, diagnostics);
        }
    }
}

fn validate_workflow_invocations(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in workflow_invoke_statements(&rule.body.text) {
        let Some((target, body)) = invoke_statement_parts(&statement) else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
                related: Vec::new(),
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
                related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
            if let Some(root) = dangling_value_root(&value, known_roots) {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has unknown binding `{root}` in `invoke {target}` input `{field}`",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "reference a binding from a `when ... as name` clause, an effect `as` binding, or a `case` pattern"
                            .to_owned(),
                    ),
                });
            }
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
                related: Vec::new(),
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
            related: Vec::new(),
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
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if kind != &IrEffectKind::AgentTell {
        return;
    }
    let Some(target) = parse_tell_target(line) else {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span: rule.body.span,
            message: format!("rule `{}` has malformed tell target", rule.name.name),
            suggestion: Some("write `tell agentName ...` or `tell task.agentRef ...`".to_owned()),
        });
        return;
    };
    if target.starts_with('"') {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
            // Unknown type can mean a dangling root (the path's binding does not
            // exist) — caught here since the type lookup returns None silently. A
            // known root with a bad path is left to other validation.
            if let Some(root) = dangling_value_root(target, known_roots) {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has unknown binding `{root}` in tell target `{target}`",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "reference a binding from a `when ... as name` clause or an effect `as` binding"
                            .to_owned(),
                    ),
                });
            }
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
                related: Vec::new(),
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
            related: Vec::new(),
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
            diagnostics.push(Diagnostic { related: Vec::new(),
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
                related: Vec::new(),
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
            related: Vec::new(),
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
        Err(message) => diagnostics.push(Diagnostic { related: Vec::new(),
            span: rule.body.span,
            message: format!("rule `{}` has invalid {label} expression: {message}", rule.name.name),
            suggestion: Some("use deterministic field paths, literals, boolean operators, comparisons, membership, count, or exists".to_owned()),
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
            related: Vec::new(),
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
                            related: Vec::new(),
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
                            related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                related: Vec::new(),
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
        related: Vec::new(),
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
                diagnostics.push(Diagnostic { related: Vec::new(),
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
                    related: Vec::new(),
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
        "exists" => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
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
                diagnostics.push(Diagnostic { related: Vec::new(),
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
                related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                        related: Vec::new(),
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
                    related: Vec::new(),
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
            "exists" => ExprType::Bool,
            _ => {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: context.span,
                    message: format!(
                        "{} calls unsupported expression function `{name}`",
                        context.subject
                    ),
                    suggestion: Some("use `count` or `exists`".to_owned()),
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
                        related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                            related: Vec::new(),
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
                            related: Vec::new(),
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
                    related: Vec::new(),
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
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            for ty in [&left_ty, &right_ty] {
                if !matches!(ty, ExprType::Int | ExprType::Float | ExprType::Unknown) {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: context.span,
                        message: format!(
                            "{} uses arithmetic with a non-numeric operand",
                            context.subject
                        ),
                        suggestion: Some("use `+ - * /` only with int or float values".to_owned()),
                    });
                    break;
                }
            }
            if matches!(left_ty, ExprType::Float) || matches!(right_ty, ExprType::Float) {
                ExprType::Float
            } else if matches!(left_ty, ExprType::Int) && matches!(right_ty, ExprType::Int) {
                ExprType::Int
            } else {
                ExprType::Unknown
            }
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
                    related: Vec::new(),
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
        if schema.contains('.') {
            // Untyped runtime fact binding (general `when fact <name>`).
            return Some(ExprType::Unknown);
        }
        return semantic
            .schemas
            .resolve_field_path(schema, &path[1..])
            .ok()
            .map(|ty| expr_type_from_type_syntax(&ty, semantic));
    }
    let schema = scope.implicit_schema.as_ref()?;
    if schema.contains('.') {
        return Some(ExprType::Unknown);
    }
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
            (ExprType::Duration, ExprType::Duration)
                | (ExprType::Time, ExprType::Time)
                // A quoted ISO-8601 string in a time-typed comparison is a
                // time literal (spec/scheduled-time.md).
                | (ExprType::Time, ExprType::String)
                | (ExprType::String, ExprType::Time)
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
                related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                    related: Vec::new(),
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
                related: Vec::new(),
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
                    // Terminal-case guards are validated by
                    // `collect_terminal_case_metadata`, which is the only path
                    // with `effect_payload_types` and so the only one that can
                    // bind the tag-refined payload (`Completed as result where
                    // result.x ...`) into the guard scope. Validating them here
                    // too would reject that binding as an unknown root.
                    if let Some(guard) = branch.guard.filter(|_| !terminal_case) {
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
    // A bare enum-typed binding is a valid scrutinee: `case o` dispatches a
    // sum-type payload (spec/sum-types.md). Class-typed bare bindings stay
    // untyped here so the "match on a bound field" guidance still fires.
    let is_bare_ident = !expr.is_empty()
        && expr.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        && expr.chars().next().is_some_and(char::is_alphabetic);
    if is_bare_ident {
        let schema = binding_types.get(expr)?;
        if semantic.schemas.enums.contains_key(schema) {
            return Some(TypeSyntax::Ref {
                name: Ident {
                    name: schema.clone(),
                    span: zero_span(),
                },
            });
        }
        return None;
    }
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
                related: Vec::new(),
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
                related: Vec::new(),
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
            let (variant, binding) = sum_case_pattern_parts(pattern);
            if !variants.contains(variant) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!("enum `{}` has no variant `{variant}`", name.name),
                    suggestion: Some(format!(
                        "use one of: {}",
                        variants.iter().cloned().collect::<Vec<_>>().join(", ")
                    )),
                });
                return;
            }
            // `as` binds a data-carrying variant's payload (spec/sum-types.md);
            // a bare variant has no payload to bind.
            if binding.is_some()
                && !semantic
                    .schemas
                    .class_exists(&format!("{}.{variant}", name.name))
            {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "variant `{variant}` of enum `{}` carries no payload to bind",
                        name.name
                    ),
                    suggestion: Some(format!("write `{variant} => {{ ... }}` without `as`")),
                });
            }
        }
        TypeSyntax::Union { variants, .. } => {
            let Some(literal) = parse_literal_expr(pattern) else {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
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
                    related: Vec::new(),
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
        // `case` over a `bool` field: only the two literals `true`/`false` (plus
        // the `_`/`default` fallbacks already handled above) are valid patterns.
        TypeSyntax::Primitive { name, .. } if name == "bool" => {
            if !matches!(pattern, "true" | "false") {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "rule `{}` has case pattern `{pattern}` that is not a `bool` value",
                        rule.name.name
                    ),
                    suggestion: Some("match `true`, `false`, or `_`".to_owned()),
                });
            }
        }
        _ => {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
    // Binding is `Tag as binding` (Stage 1b: the legacy space form `Tag binding` is
    // no longer accepted — it aligns terminal cases with enum-variant `as` binding).
    let second = parts.next();
    let binding = match second {
        Some("as") => parts.next(),
        other => other,
    };
    let uses_as = matches!(second, Some("as"));
    if parts.next().is_some() || binding.is_none() || !uses_as {
        diagnostics.push(Diagnostic { related: Vec::new(),
            span,
            message: format!(
                "rule `{}` has malformed terminal-output case pattern `{pattern}`",
                rule.name.name
            ),
            suggestion: Some("write `Completed as result`, `Failed as failure`, `TimedOut as timeout`, or `Cancelled as cancel` (the `as` is required)".to_owned()),
        });
        return;
    }
    let tags = terminal_case_tags();
    if !tags.contains(&tag) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
    validate_unreachable_after_fallback(rule, branches, diagnostics);
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
            related: Vec::new(),
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
                related: Vec::new(),
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
    validate_unreachable_after_fallback(rule, branches, diagnostics);
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
            related: Vec::new(),
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
                related: Vec::new(),
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

/// Flags case branches that can never be reached because an earlier *unguarded*
/// wildcard (`_`/`default`) already matches everything. Shared by rule cases and
/// terminal-output cases. Mirrors case-family.maude inv c (redundant-postwild): any
/// arm after the wildcard is redundant. A *guarded* fallback (`_ where g`) does not
/// shadow, since its guard can fail at runtime.
fn validate_unreachable_after_fallback(
    rule: &RuleDecl,
    branches: &[SpanCaseBranchHead<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut ordered: Vec<&SpanCaseBranchHead<'_>> = branches.iter().collect();
    ordered.sort_by_key(|branch| branch.pattern_span.start);
    let mut fallback_span: Option<SourceSpan> = None;
    for branch in ordered {
        if let Some(prior) = fallback_span {
            diagnostics.push(
                Diagnostic {
                    related: Vec::new(),
                    span: branch.pattern_span,
                    message: format!(
                        "rule `{}` has an unreachable case branch after the `_` wildcard",
                        rule.name.name
                    ),
                    suggestion: Some(
                        "move this branch before the wildcard, or remove it".to_owned(),
                    ),
                }
                .with_related(
                    prior,
                    "this unguarded wildcard already matches every remaining value",
                ),
            );
        } else if branch.guard.is_none() && is_fallback_pattern(branch.pattern) {
            fallback_span = Some(branch.pattern_span);
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
        // `bool` is a finite two-value domain: an exhaustive `case` over it must
        // cover both `true` and `false` (or carry a `_`).
        TypeSyntax::Primitive { name, .. } if name == "bool" => {
            Some(vec!["true".to_owned(), "false".to_owned()])
        }
        _ => None,
    }
}

/// Splits a sum-type case pattern `Variant as binding` into variant and
/// binding (spec/sum-types.md); a plain pattern returns no binding.
fn sum_case_pattern_parts(pattern: &str) -> (&str, Option<&str>) {
    match pattern.split_once(" as ") {
        Some((variant, binding)) => (variant.trim(), Some(binding.trim())),
        None => (pattern.trim(), None),
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
    // Coverage counts the variant, not its payload binding.
    let (pattern, _) = sum_case_pattern_parts(pattern);
    // `bool` literals parse to the value-less `LiteralExpr::Bool`; return them
    // verbatim so they count toward `true`/`false` coverage.
    if matches!(pattern, "true" | "false") {
        return Some(pattern);
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
            related: Vec::new(),
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
            related: Vec::new(),
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
            related: Vec::new(),
            span,
            message: format!("rule `{}` has non-agent case pattern", rule.name.name),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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

        diagnostics.push(Diagnostic { related: Vec::new(),
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
        .map(|frame| match frame {
            BlockFrame::After { binding, predicate } => (binding.clone(), predicate.clone()),
        })
        .collect()
}

/// The single lowering table for readiness sugar: maps a `when` pattern to
/// the runtime fact name it matches. The general form is
/// `when fact <name> as x`; the English phrases are documented abbreviations
/// of it.
pub fn runtime_fact_name_for_pattern(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if let Some(rest) = pattern.strip_prefix("fact ") {
        let name = rest.split_whitespace().next()?;
        return Some(name.to_owned());
    }
    if pattern.starts_with("human answered") {
        return Some("human.answer.received".to_owned());
    }
    // Inbound messaging (spec/messaging.md): `message from <channel>` matches the
    // channel-specific `message.<channel>` fact ingested by `whip message`.
    if let Some(rest) = pattern.strip_prefix("message from ") {
        if let Some(channel) = rest.split_whitespace().next() {
            return Some(format!("message.{channel}"));
        }
    }
    let mut words = pattern.split_whitespace();
    let first = words.next()?;
    if words.next() == Some("completed") && words.next() == Some("turn") {
        let _ = first;
        return Some("agent.turn.completed".to_owned());
    }
    {
        let mut words = pattern.split_whitespace();
        let _queue = words.next();
        if words.next() == Some("has")
            && words.next() == Some("ready")
            && words.next() == Some("item")
        {
            return Some("queue.item.ready".to_owned());
        }
    }
    if first.chars().next().is_some_and(char::is_uppercase) {
        return Some(first.to_owned());
    }
    None
}

/// The schema used to type-check fields on the pattern's binding. Dotted
/// runtime fact names are untyped (no class declares them); the sugar forms
/// map to their builtin schemas.
fn binding_from_when(when: &str) -> Option<(String, String)> {
    let (pattern, _) = split_when_guard(when);
    let binding = binding_after_as(pattern)?;
    let first = pattern.split_whitespace().next()?;
    let completed_turn = {
        let mut words = pattern.split_whitespace();
        words.next();
        words.next() == Some("completed") && words.next() == Some("turn")
    };
    let has_ready_item = {
        let mut words = pattern.split_whitespace();
        words.next();
        words.next() == Some("has") && words.next() == Some("ready") && words.next() == Some("item")
    };
    let schema = if let Some(rest) = pattern.strip_prefix("fact ") {
        rest.split_whitespace().next()?.to_owned()
    } else if first.chars().next().is_some_and(char::is_uppercase) {
        first.to_owned()
    } else if first.contains('.') {
        // Bare dotted reaction `when deploy.finished as d` — typed against a
        // declared `event` (validated at the call site,
        // spec/event-ingress.md).
        first.to_owned()
    } else if completed_turn {
        "AgentTurn".to_owned()
    } else if pattern.starts_with("human answered ") {
        "HumanAnswer".to_owned()
    } else if has_ready_item {
        "WorkItem".to_owned()
    } else if pattern.starts_with("message from ") {
        // Inbound messaging (spec/messaging.md): `when message from <channel> as
        // msg` binds the generic `Message` envelope, never a domain type.
        "Message".to_owned()
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
        IrEffectKind::Coerce => parse_coerce_call_name(line).and_then(|name| {
            semantic
                .coerce_outputs
                .get(name)
                .and_then(schema_name_for_path)
        }),
        IrEffectKind::AgentTell
        | IrEffectKind::CapabilityCall
        | IrEffectKind::EventEmit
        | IrEffectKind::WorkflowInvoke
        | IrEffectKind::TimerWait
        | IrEffectKind::ExecCommand
        | IrEffectKind::QueueFile
        | IrEffectKind::QueueClaim
        | IrEffectKind::QueueRelease
        | IrEffectKind::QueueFinish
        | IrEffectKind::LeaseAcquire
        | IrEffectKind::LedgerAppend
        | IrEffectKind::CounterConsume
        | IrEffectKind::EventNotify
        | IrEffectKind::FileRead
        | IrEffectKind::FileWrite
        | IrEffectKind::FileImport
        | IrEffectKind::FileExport => None,
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

/// The hygienic class name synthesized for an inline `decide -> { … } as
/// <binding>`. Dots are illegal in user class names (like the `flow.<name>.seg*`
/// rule convention), so `decide.<rule>.<binding>` can never collide with a
/// declared schema. The lowering pass, the type checker, and the runtime fixture
/// all derive the same name, so the anonymous result shape flows exactly like a
/// named `coerce -> Schema`: `after <binding> succeeds as r` resolves `r`'s
/// fields for `case` dispatch and field access.
pub fn inline_decide_schema_name(rule: &str, binding: &str) -> String {
    format!("decide.{rule}.{binding}")
}

/// A single-identifier `decide` field type is either a primitive keyword
/// (`bool`, `string`, …) or a reference to a declared class/enum. The `decide`
/// grammar only admits single identifiers, so no compound parsing is needed.
fn decide_field_type_syntax(ty: &str, span: SourceSpan) -> TypeSyntax {
    if is_primitive_type(ty) {
        TypeSyntax::Primitive {
            name: ty.to_owned(),
            span,
        }
    } else {
        TypeSyntax::Ref {
            name: Ident {
                name: ty.to_owned(),
                span,
            },
        }
    }
}

/// Collects every inline `decide … as <binding>` in a rule body — recursing
/// through nested after/case/branch/handler blocks — yielding
/// `(binding, result_fields, span)` for synthesis and type registration.
#[allow(clippy::type_complexity)]
fn collect_decide_effects<'a>(
    statements: &'a [body::BodyStmt],
    out: &mut Vec<(&'a str, &'a [(String, String)], SourceSpan)>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                if let body::BodyEffectKind::Decide { result_fields } = &effect.kind {
                    if let Some(binding) = &effect.binding {
                        out.push((binding.as_str(), result_fields.as_slice(), effect.span));
                    }
                }
            }
            body::BodyStmt::After(after) => collect_decide_effects(&after.body, out),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_decide_effects(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_decide_effects(&branch.then_body, out);
                if let Some(else_body) = &branch.else_body {
                    collect_decide_effects(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => collect_decide_effects(&handler.body, out),
            _ => {}
        }
    }
}

/// Registers each inline `decide … as <binding>` result as `Ref(decide.<rule>.<binding>)`
/// so the after-binding type flow resolves the anonymous shape's fields, exactly
/// like a named `coerce -> Schema`. The synthesized class is injected into both
/// the semantic schema index and the IR by [`collect_inline_decide_schemas`].
fn collect_decide_payload_types(
    statements: &[body::BodyStmt],
    rule_name: &str,
    payloads: &mut BTreeMap<String, IrType>,
) {
    let mut decides = Vec::new();
    collect_decide_effects(statements, &mut decides);
    for (binding, _fields, _span) in decides {
        payloads.insert(
            binding.to_owned(),
            IrType::Ref(inline_decide_schema_name(rule_name, binding)),
        );
    }
}

/// Synthesizes a hygienic `decide.<rule>.<binding>` class for every inline
/// `decide -> { … } as <binding>`, injecting it into both the semantic schema
/// index (so field access / `case` type-check) and the IR (so the runtime
/// fixture can generate the anonymous shape). Mirrors the generated
/// `<Enum>.<Variant>` class synthesis for data-carrying sum-type variants.
fn collect_inline_decide_schemas(
    items: &[Item],
    semantic: &mut SemanticContext,
    ir: &mut IrProgram,
) {
    for item in items {
        let Item::Rule(rule) = item else {
            continue;
        };
        let (body_ast, _) =
            body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
        let mut decides = Vec::new();
        collect_decide_effects(&body_ast.statements, &mut decides);
        for (binding, fields, span) in decides {
            let name = inline_decide_schema_name(&rule.name.name, binding);
            // Build the field shape once as `TypeSyntax` (the schema-index form),
            // then lower it for the IR so both representations stay in lockstep.
            let mut syntax_fields: BTreeMap<String, TypeSyntax> = BTreeMap::new();
            let mut ir_fields = Vec::new();
            for (field_name, field_ty) in fields {
                let ty = decide_field_type_syntax(field_ty, span);
                ir_fields.push(IrClassField {
                    name: field_name.clone(),
                    ty: lower_type(ty.clone()),
                    is_key: false,
                    presence_condition: None,
                    span,
                });
                syntax_fields.insert(field_name.clone(), ty);
            }
            semantic.schemas.classes.insert(name.clone(), syntax_fields);
            ir.schemas.push(IrSchema::Class(IrClass {
                name,
                fields: ir_fields,
                span,
            }));
        }
    }
}

/// The hygienic synthetic class name for a `redact … as <binding>` projection:
/// `redact.<rule>.<binding>`, holding only the kept fields of the source schema.
pub fn redact_schema_name(rule: &str, binding: &str) -> String {
    format!("redact.{rule}.{binding}")
}

/// Collects every `redact <source> keep [..] as <binding>` in a rule body —
/// recursing through nested after/case/branch/handler blocks — for projected-type
/// synthesis, type registration, and IFC value-flow.
#[allow(clippy::type_complexity)]
fn collect_redact_effects<'a>(
    statements: &'a [body::BodyStmt],
    out: &mut Vec<(&'a str, &'a [String], &'a str, SourceSpan)>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Redact {
                source,
                keep,
                binding,
                span,
            } => out.push((source.as_str(), keep.as_slice(), binding.as_str(), *span)),
            body::BodyStmt::After(after) => collect_redact_effects(&after.body, out),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_redact_effects(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_redact_effects(&branch.then_body, out);
                if let Some(else_body) = &branch.else_body {
                    collect_redact_effects(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => collect_redact_effects(&handler.body, out),
            _ => {}
        }
    }
}

/// Resolves binding -> schema name for a rule's redact SOURCES: `when Class as x`
/// matches, plus coerce/decide/exec result bindings. Used only to find the schema
/// a `redact` projects from, so the synthetic projected class copies the kept
/// fields' types. (`after`-alias sources are a documented follow-up; an
/// unresolved source surfaces as an empty projection + a `validate_redactions`
/// diagnostic.) Diagnostics from the reused collector are discarded — the real
/// pass re-emits them.
fn rule_binding_schemas(rule: &RuleDecl, semantic: &SemanticContext) -> BTreeMap<String, String> {
    let mut schemas = binding_types_for_rule(rule);
    let (body_ast, _) =
        body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
    let mut payloads = collect_effect_payload_types(rule, semantic, &mut Vec::new());
    collect_exec_payload_types(&body_ast.statements, semantic, &mut payloads);
    collect_decide_payload_types(&body_ast.statements, &rule.name.name, &mut payloads);
    collect_redact_payload_types(&body_ast.statements, &rule.name.name, &mut payloads);
    // `after <binding> <predicate> as <alias>` aliases the effect's completed
    // payload schema, so a `coerce … as c` then `after c succeeds as cust` then
    // `redact cust …` resolves (the primary read-then-redact flow). Only
    // payload-carrying predicates are mapped here; terminal predicates
    // (`times out`/`fails`) bind synthetic terminal schemas not usefully redacted.
    for line in rule.body.text.lines() {
        let Some(rest) = line.trim().strip_prefix("after ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let Some(binding) = words.next() else {
            continue;
        };
        let Some(predicate) = words.next() else {
            continue;
        };
        if predicate == "times" && words.next() != Some("out") {
            continue;
        }
        let (Some("as"), Some(alias)) = (words.next(), words.next()) else {
            continue;
        };
        let alias = alias.trim_end_matches('{').trim();
        if alias.is_empty() {
            continue;
        }
        if let Some(IrType::Ref(schema)) = payloads.get(binding) {
            schemas.insert(alias.to_owned(), schema.clone());
        }
    }
    for (binding, ty) in payloads {
        if let IrType::Ref(schema) = ty {
            schemas.insert(binding, schema);
        }
    }
    schemas
}

/// Synthesizes a hygienic `redact.<rule>.<binding>` class for every
/// `redact <source> keep [..] as <binding>`, holding ONLY the kept fields of the
/// source schema (with their source types). This is what makes a redaction sound:
/// the projected binding cannot expose a dropped field (accessing one is a
/// type error, since it is absent from the synthetic class), so the lowered IFC
/// label the checker assigns the projection is honoured by the type system too.
/// Mirrors [`collect_inline_decide_schemas`]; run before the rule loop so
/// `analyze_rule` sees the class. A redact chained off an earlier redact's output
/// resolves via the local map built as the pass proceeds.
fn collect_redact_schemas(items: &[Item], semantic: &mut SemanticContext, ir: &mut IrProgram) {
    for item in items {
        let Item::Rule(rule) = item else {
            continue;
        };
        let (body_ast, _) =
            body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
        let mut redacts = Vec::new();
        collect_redact_effects(&body_ast.statements, &mut redacts);
        if redacts.is_empty() {
            continue;
        }
        let binding_schemas = rule_binding_schemas(rule, semantic);
        let mut local: BTreeMap<String, String> = BTreeMap::new();
        for (source, keep, binding, span) in redacts {
            let name = redact_schema_name(&rule.name.name, binding);
            let source_schema = binding_schemas
                .get(source)
                .cloned()
                .or_else(|| local.get(source).cloned());
            // Clone the kept fields' types out of the source schema first, so the
            // immutable borrow ends before we insert the new class.
            let projected: Vec<(String, TypeSyntax)> = source_schema
                .as_ref()
                .and_then(|schema| semantic.schemas.classes.get(schema))
                .map(|src_fields| {
                    keep.iter()
                        .filter_map(|field| {
                            src_fields.get(field).map(|ty| (field.clone(), ty.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut syntax_fields: BTreeMap<String, TypeSyntax> = BTreeMap::new();
            let mut ir_fields = Vec::new();
            for (field_name, ty) in &projected {
                syntax_fields.insert(field_name.clone(), ty.clone());
                ir_fields.push(IrClassField {
                    name: field_name.clone(),
                    ty: lower_type(ty.clone()),
                    is_key: false,
                    presence_condition: None,
                    span,
                });
            }
            semantic.schemas.classes.insert(name.clone(), syntax_fields);
            ir.schemas.push(IrSchema::Class(IrClass {
                name: name.clone(),
                fields: ir_fields,
                span,
            }));
            local.insert(binding.to_owned(), name);
        }
    }
}

/// Registers each `redact … as <binding>` result as `Ref(redact.<rule>.<binding>)`
/// so field access / `case` through the projection resolves against the kept-only
/// synthetic class (a dropped field is an unknown-field error). Mirrors
/// [`collect_decide_payload_types`].
fn collect_redact_payload_types(
    statements: &[body::BodyStmt],
    rule_name: &str,
    payloads: &mut BTreeMap<String, IrType>,
) {
    let mut redacts = Vec::new();
    collect_redact_effects(statements, &mut redacts);
    for (_source, _keep, binding, _span) in redacts {
        payloads.insert(
            binding.to_owned(),
            IrType::Ref(redact_schema_name(rule_name, binding)),
        );
    }
}

/// Validates each `redact <source> keep [..] as <out>`: the source must resolve to
/// a known schema, and every kept field must exist on it. Fail-closed — an
/// unresolvable source or unknown kept field is a hard error, so a redaction can
/// never silently project nothing (which would carry no data and mask a mistake).
fn validate_redactions(
    rule: &RuleDecl,
    statements: &[body::BodyStmt],
    semantic: &SemanticContext,
    binding_schemas: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut redacts = Vec::new();
    collect_redact_effects(statements, &mut redacts);
    let mut local: BTreeMap<String, String> = BTreeMap::new();
    for (source, keep, binding, span) in redacts {
        let source_schema = binding_schemas
            .get(source)
            .cloned()
            .or_else(|| local.get(source).cloned());
        local.insert(
            binding.to_owned(),
            redact_schema_name(&rule.name.name, binding),
        );
        let Some(schema) = source_schema else {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!(
                    "rule `{}` redacts `{source}`, which has no known schema",
                    rule.name.name
                ),
                suggestion: Some(
                    "redact a binding with a known record type — a matched `when Class as x`, or a \
                     coerce/decide/exec result"
                        .to_owned(),
                ),
            });
            continue;
        };
        let Some(src_fields) = semantic.schemas.classes.get(&schema) else {
            continue;
        };
        for field in keep {
            if !src_fields.contains_key(field) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "rule `{}` redacts `{source}` keeping unknown field `{field}` of `{schema}`",
                        rule.name.name
                    ),
                    suggestion: Some(format!("keep a field declared on `{schema}`")),
                });
            }
        }
    }
}

/// Registers the typed result of the single `exec "..." -> Schema as binding`
/// form so `after <binding> succeeds as r` resolves `r`'s fields — the same
/// after-binding type flow a named `coerce -> Schema` already gets. The
/// streaming `-> each Schema` form records one fact per element (not a single
/// bound value), so it is skipped here.
fn collect_exec_payload_types(
    statements: &[body::BodyStmt],
    semantic: &SemanticContext,
    payloads: &mut BTreeMap<String, IrType>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                if let body::BodyEffectKind::Exec {
                    parse_target: Some(parse),
                    ..
                } = &effect.kind
                {
                    if !parse.each {
                        if let Some(binding) = &effect.binding {
                            if semantic.schemas.class_exists(&parse.schema) {
                                payloads.insert(binding.clone(), IrType::Ref(parse.schema.clone()));
                            }
                        }
                    }
                }
            }
            body::BodyStmt::After(after) => {
                collect_exec_payload_types(&after.body, semantic, payloads)
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    collect_exec_payload_types(&branch.body, semantic, payloads);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_exec_payload_types(&branch.then_body, semantic, payloads);
                if let Some(else_body) = &branch.else_body {
                    collect_exec_payload_types(else_body, semantic, payloads);
                }
            }
            body::BodyStmt::Handler(handler) => {
                collect_exec_payload_types(&handler.body, semantic, payloads)
            }
            _ => {}
        }
    }
}

/// Collects the schemas an `exec ... -> each` stream records as facts.
fn push_ingest_fact_writes(statements: &[body::BodyStmt], fact_writes: &mut Vec<String>) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                match &effect.kind {
                    body::BodyEffectKind::Exec {
                        parse_target: Some(parse),
                        ..
                    } if parse.each => {
                        fact_writes.push(format!("schema:{}", parse.schema));
                    }
                    // `import <fmt> <Schema>` admits one `<Schema>` fact per row
                    // (spec/std-library/files.md), so a `when <Schema>` rule has a
                    // producer for liveness/effect-graph analysis.
                    body::BodyEffectKind::FileImport { schema, .. } => {
                        fact_writes.push(format!("schema:{schema}"));
                    }
                    _ => {}
                }
            }
            body::BodyStmt::After(after) => push_ingest_fact_writes(&after.body, fact_writes),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    push_ingest_fact_writes(&branch.body, fact_writes);
                }
            }
            body::BodyStmt::Branch(branch) => {
                push_ingest_fact_writes(&branch.then_body, fact_writes);
                if let Some(else_body) = &branch.else_body {
                    push_ingest_fact_writes(else_body, fact_writes);
                }
            }
            body::BodyStmt::Handler(handler) => push_ingest_fact_writes(&handler.body, fact_writes),
            _ => {}
        }
    }
}

/// Body-effect operand checks that need schema knowledge:
/// - `timer until <operand>`: a non-literal operand must be a dotted path
///   resolving to a `time`-typed field (spec/scheduled-time.md). Literals were
///   format-validated by the body parser, so anything that still looks like an
///   instant here is a valid literal and passes.
/// - `exec ... -> Schema` / `-> each Schema`: the parse target must name a
///   declared class (spec/json-ingestion.md).
///
/// The coordination safety model (spec/coordination.md): at most one held
/// lease per progression (hard default), exhaustive outcome handling, and
/// the linear must-release discipline (instance terminals auto-release, so
/// a path that ends in `complete`/`fail` is safe without an explicit
/// `release`).
fn validate_coordination_discipline(
    rule: &RuleDecl,
    statements: &[body::BodyStmt],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut acquires = Vec::new();
    let mut consumes = Vec::new();
    collect_coordination_effects(statements, &mut acquires, &mut consumes);

    if acquires.len() > 1 {
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: acquires[1].2,
            message: format!(
                "rule `{}` acquires more than one lease in a single progression",
                rule.name.name
            ),
            suggestion: Some(
                "the hard default is at most one held lease per progression (it breaks hold-and-wait); restructure into separate rules"
                    .to_owned(),
            ),
        });
    }
    for (binding, until_ttl, span) in &acquires {
        if *until_ttl {
            continue;
        }
        let mut predicates = BTreeSet::new();
        collect_after_predicates(statements, binding, &mut predicates);
        for required in ["held", "contended"] {
            if !predicates.contains(required) {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: *span,
                    message: format!(
                        "rule `{}` does not handle the `{required}` outcome of lease `{binding}`",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "coordination outcomes are exhaustive: add `after {binding} {required} {{ ... }}`"
                    )),
                });
            }
        }
        if let Some(held_body) = find_after_body(statements, binding, body::AfterPredicate::Held) {
            if !releases_or_terminates(held_body, binding) {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: *span,
                    message: format!(
                        "rule `{}` can hold lease `{binding}` forever: the `held` branch neither releases it nor reaches a workflow terminal",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "add `release {binding}` on every non-terminal path, or use `acquire ... until ttl` for fire-and-forget"
                    )),
                });
            }
        }
    }
    for (binding, span) in &consumes {
        let mut predicates = BTreeSet::new();
        collect_after_predicates(statements, binding, &mut predicates);
        for required in ["ok", "over"] {
            if !predicates.contains(required) {
                diagnostics.push(Diagnostic { related: Vec::new(),
                    span: *span,
                    message: format!(
                        "rule `{}` does not handle the `{required}` outcome of counter consume `{binding}`",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "coordination outcomes are exhaustive: add `after {binding} {required} {{ ... }}`"
                    )),
                });
            }
        }
    }
}

fn collect_coordination_effects(
    statements: &[body::BodyStmt],
    acquires: &mut Vec<(String, bool, SourceSpan)>,
    consumes: &mut Vec<(String, SourceSpan)>,
) {
    for_each_body(statements, &mut |stmt| {
        if let body::BodyStmt::Effect(effect) = stmt {
            match &effect.kind {
                body::BodyEffectKind::LeaseAcquire { until_ttl, .. } => {
                    if let Some(binding) = &effect.binding {
                        acquires.push((binding.clone(), *until_ttl, effect.span));
                    }
                }
                body::BodyEffectKind::CounterConsume { .. } => {
                    if let Some(binding) = &effect.binding {
                        consumes.push((binding.clone(), effect.span));
                    }
                }
                _ => {}
            }
        }
    });
}

fn collect_after_predicates(
    statements: &[body::BodyStmt],
    binding: &str,
    predicates: &mut BTreeSet<&'static str>,
) {
    for_each_body(statements, &mut |stmt| {
        if let body::BodyStmt::After(after) = stmt {
            if after.binding == binding {
                predicates.insert(after.predicate.as_str());
            }
        }
    });
}

fn find_after_body<'a>(
    statements: &'a [body::BodyStmt],
    binding: &str,
    predicate: body::AfterPredicate,
) -> Option<&'a [body::BodyStmt]> {
    for statement in statements {
        match statement {
            body::BodyStmt::After(after) => {
                if after.binding == binding && after.predicate == predicate {
                    return Some(&after.body);
                }
                if let Some(found) = find_after_body(&after.body, binding, predicate) {
                    return Some(found);
                }
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    if let Some(found) = find_after_body(&branch.body, binding, predicate) {
                        return Some(found);
                    }
                }
            }
            body::BodyStmt::Branch(branch) => {
                if let Some(found) = find_after_body(&branch.then_body, binding, predicate) {
                    return Some(found);
                }
                if let Some(else_body) = &branch.else_body {
                    if let Some(found) = find_after_body(else_body, binding, predicate) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Linear must-release, prototype form: a statement list is safe if some
/// statement guarantees release — an explicit `release <binding>`, a
/// workflow terminal (instance-terminal auto-release), a nested after-block
/// that is safe, or a branching construct ALL of whose branches are safe.
fn releases_or_terminates(statements: &[body::BodyStmt], binding: &str) -> bool {
    statements.iter().any(|statement| match statement {
        body::BodyStmt::Effect(effect) => matches!(
            &effect.kind,
            body::BodyEffectKind::QueueRelease { item } if item == binding
        ),
        body::BodyStmt::Terminal(_) => true,
        body::BodyStmt::After(after) => releases_or_terminates(&after.body, binding),
        body::BodyStmt::Case(case) => {
            !case.branches.is_empty()
                && case
                    .branches
                    .iter()
                    .all(|branch| releases_or_terminates(&branch.body, binding))
        }
        body::BodyStmt::Branch(branch) => {
            releases_or_terminates(&branch.then_body, binding)
                && branch
                    .else_body
                    .as_ref()
                    .is_some_and(|else_body| releases_or_terminates(else_body, binding))
        }
        _ => false,
    })
}

fn for_each_body(statements: &[body::BodyStmt], visit: &mut impl FnMut(&body::BodyStmt)) {
    for statement in statements {
        visit(statement);
        match statement {
            body::BodyStmt::After(after) => for_each_body(&after.body, visit),
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    for_each_body(&branch.body, visit);
                }
            }
            body::BodyStmt::Branch(branch) => {
                for_each_body(&branch.then_body, visit);
                if let Some(else_body) = &branch.else_body {
                    for_each_body(else_body, visit);
                }
            }
            body::BodyStmt::Handler(handler) => for_each_body(&handler.body, visit),
            _ => {}
        }
    }
}

/// Family B: the `(root, field)` pairs a `case <root>.<disc> { "<lit>" => ... }` arm
/// makes readable — the fields conditioned on `<disc> is "<lit>"`. Empty unless the
/// scrutinee is a single-level `<root>.<disc>` path bound to a schema and the arm
/// pattern is the matching string literal.
fn family_b_arm_allowed(
    scrutinee: &str,
    pattern: &str,
    binding_types: &BTreeMap<String, String>,
    semantic: &SemanticContext,
) -> BTreeSet<(String, String)> {
    let mut allowed = BTreeSet::new();
    let Some((root, disc)) = scrutinee.split_once('.') else {
        return allowed;
    };
    if disc.contains('.') {
        return allowed;
    }
    let trimmed = pattern.trim();
    if trimmed == "_" || trimmed == "default" {
        return allowed;
    }
    let literal = trimmed.trim_matches('"');
    if literal.is_empty() {
        return allowed;
    }
    let Some(schema) = binding_types.get(root) else {
        return allowed;
    };
    if let Some(conditions) = semantic.schemas.presence.get(schema) {
        for (field, (cond_disc, cond_literal)) in conditions {
            if cond_disc == disc && cond_literal == literal {
                allowed.insert((root.to_owned(), field.clone()));
            }
        }
    }
    allowed
}

/// Reject reads of a Family B presence-conditioned field in `text` that are not
/// permitted by `allowed` (the conditioned fields this scope's `case` arm makes
/// present). `text` is any source fragment that may contain dotted field paths.
fn check_conditioned_reads_in_text(
    rule: &RuleDecl,
    text: &str,
    span: SourceSpan,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    allowed: &BTreeSet<(String, String)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (root, path) in dotted_paths(text) {
        let Some(first) = path.first() else {
            continue;
        };
        let Some(schema) = binding_types.get(&root) else {
            continue;
        };
        if let Some((disc, _literal)) = semantic.schemas.field_presence(schema, first) {
            if !allowed.contains(&(root.clone(), first.clone())) {
                diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "rule `{}` reads conditional field `{root}.{first}` outside a matching `case {root}.{disc}` arm",
                        rule.name.name
                    ),
                    suggestion: Some(format!(
                        "read `{root}.{first}` inside `case {root}.{disc} {{ \"...\" => ... }}` — it is present only for a specific `{disc}`"
                    )),
                });
            }
        }
    }
}

fn check_conditioned_reads_in_fields(
    rule: &RuleDecl,
    fields: &[body::FieldAssign],
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    allowed: &BTreeSet<(String, String)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for field in fields {
        match &field.value {
            body::FieldValue::Expr { source, .. } => check_conditioned_reads_in_text(
                rule,
                source,
                field.span,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::FieldValue::Nested { fields, .. } => check_conditioned_reads_in_fields(
                rule,
                fields,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::FieldValue::Shorthand => {}
        }
    }
}

/// Family B read-narrowing (discriminated-families-design.md §5.6/§5.7): walk the
/// rule body and reject a read of a presence-conditioned field that is not inside a
/// matching `case <root>.<disc>` arm. Each `case` arm extends `allowed` with the
/// fields its discriminant=literal makes present. (v1 covers record/terminal/done
/// values, branch conditions, and case guards; effect prompt/argument positions are
/// a documented follow-up.)
fn validate_conditioned_field_reads(
    rule: &RuleDecl,
    statements: &[body::BodyStmt],
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    allowed: &BTreeSet<(String, String)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Record(record) => check_conditioned_reads_in_fields(
                rule,
                &record.fields,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Terminal(terminal) => check_conditioned_reads_in_fields(
                rule,
                &terminal.fields,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Done {
                replacement: Some(record),
                ..
            } => check_conditioned_reads_in_fields(
                rule,
                &record.fields,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Milestone { fields, .. } => check_conditioned_reads_in_fields(
                rule,
                fields,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Done { .. }
            | body::BodyStmt::Cancel { .. }
            | body::BodyStmt::Redact { .. }
            | body::BodyStmt::Effect(_) => {}
            body::BodyStmt::After(after) => validate_conditioned_field_reads(
                rule,
                &after.body,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Handler(handler) => validate_conditioned_field_reads(
                rule,
                &handler.body,
                semantic,
                binding_types,
                allowed,
                diagnostics,
            ),
            body::BodyStmt::Branch(branch) => {
                check_conditioned_reads_in_text(
                    rule,
                    &branch.condition_source,
                    branch.span,
                    semantic,
                    binding_types,
                    allowed,
                    diagnostics,
                );
                validate_conditioned_field_reads(
                    rule,
                    &branch.then_body,
                    semantic,
                    binding_types,
                    allowed,
                    diagnostics,
                );
                if let Some(else_body) = &branch.else_body {
                    validate_conditioned_field_reads(
                        rule,
                        else_body,
                        semantic,
                        binding_types,
                        allowed,
                        diagnostics,
                    );
                }
            }
            body::BodyStmt::Case(case) => {
                for arm in &case.branches {
                    let mut arm_allowed = allowed.clone();
                    arm_allowed.extend(family_b_arm_allowed(
                        &case.scrutinee,
                        &arm.pattern,
                        binding_types,
                        semantic,
                    ));
                    if let Some(guard) = &arm.guard {
                        check_conditioned_reads_in_text(
                            rule,
                            guard,
                            arm.span,
                            semantic,
                            binding_types,
                            &arm_allowed,
                            diagnostics,
                        );
                    }
                    validate_conditioned_field_reads(
                        rule,
                        &arm.body,
                        semantic,
                        binding_types,
                        &arm_allowed,
                        diagnostics,
                    );
                }
            }
        }
    }
}

fn validate_body_effect_operands(
    rule: &RuleDecl,
    statements: &[body::BodyStmt],
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                match &effect.kind {
                    body::BodyEffectKind::LeaseAcquire { resource, .. }
                        if !semantic.leases.contains(resource) =>
                    {
                        diagnostics.push(Diagnostic { related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` acquires undeclared lease `{resource}`",
                                rule.name.name
                            ),
                            suggestion: Some(format!(
                                "declare `lease {resource} {{ key <Type>  slots <N>  ttl <duration> }}`"
                            )),
                        });
                    }
                    body::BodyEffectKind::LedgerAppend { ledger, schema, .. } => {
                        if !semantic.ledgers.contains(ledger) {
                            diagnostics.push(Diagnostic { related: Vec::new(),
                                span: effect.span,
                                message: format!(
                                    "rule `{}` appends to undeclared ledger `{ledger}`",
                                    rule.name.name
                                ),
                                suggestion: Some(format!(
                                    "declare `ledger {ledger} {{ entry <Type>  partition by <field>  retain <duration> }}`"
                                )),
                            });
                        }
                        if !semantic.schemas.class_exists(schema) {
                            diagnostics.push(Diagnostic {
                                related: Vec::new(),
                                span: effect.span,
                                message: format!(
                                    "rule `{}` appends unknown entry class `{schema}`",
                                    rule.name.name
                                ),
                                suggestion: Some(format!("declare `class {schema}` first")),
                            });
                        }
                    }
                    body::BodyEffectKind::CounterConsume { counter, .. }
                        if !semantic.counters.contains(counter) =>
                    {
                        diagnostics.push(Diagnostic { related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` consumes undeclared counter `{counter}`",
                                rule.name.name
                            ),
                            suggestion: Some(format!(
                                "declare `counter {counter} {{ key <Type>  cap <N>  reset <period> }}`"
                            )),
                        });
                    }
                    _ => {}
                }
                if let body::BodyEffectKind::Exec {
                    parse_target: Some(parse),
                    ..
                } = &effect.kind
                {
                    if !semantic.schemas.class_exists(&parse.schema) {
                        let suggestion =
                            match closest_name(&parse.schema, semantic.schemas.classes.keys()) {
                                Some(candidate) => format!(
                                    "did you mean `{candidate}`? otherwise declare `class {}`",
                                    parse.schema
                                ),
                                None => format!(
                                    "declare `class {}` before parsing into it",
                                    parse.schema
                                ),
                            };
                        diagnostics.push(Diagnostic {
                            related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` parses exec output into unknown schema `{}`",
                                rule.name.name, parse.schema
                            ),
                            suggestion: Some(suggestion),
                        });
                    }
                }
                let body::BodyEffectKind::Timer {
                    until: Some(until), ..
                } = &effect.kind
                else {
                    continue;
                };
                if body::is_iso8601_instant(until) {
                    continue;
                }
                let mut segments = until.split('.');
                let root = segments.next().unwrap_or_default();
                let path = segments.map(str::to_owned).collect::<Vec<_>>();
                let Some(schema) = binding_types.get(root) else {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: effect.span,
                        message: format!(
                            "rule `{}` uses unknown binding `{root}` in `timer until {until}`",
                            rule.name.name
                        ),
                        suggestion: Some(
                            "bind a fact in `when` and reference a `time` field on it, or use an ISO-8601 literal"
                                .to_owned(),
                        ),
                    });
                    continue;
                };
                // Dotted runtime fact bindings are untyped; their fields
                // cannot be statically checked.
                if schema.contains('.') {
                    continue;
                }
                let resolved = if path.is_empty() {
                    Err(format!(
                        "`{root}` is a `{schema}` record, not a `time` value"
                    ))
                } else {
                    semantic.schemas.resolve_field_path(schema, &path)
                };
                match resolved {
                    Ok(TypeSyntax::Primitive { ref name, .. }) if name == "time" => {}
                    Ok(_) => {
                        diagnostics.push(Diagnostic { related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` uses non-time operand `{until}` in `timer until`",
                                rule.name.name
                            ),
                            suggestion: Some(format!(
                                "declare the field as `time` on `{schema}` or use an ISO-8601 literal"
                            )),
                        });
                    }
                    Err(message) => {
                        diagnostics.push(Diagnostic { related: Vec::new(),
                            span: effect.span,
                            message: format!(
                                "rule `{}` has invalid `timer until` operand `{until}`: {message}",
                                rule.name.name
                            ),
                            suggestion: Some(
                                "reference a `time`-typed field on a bound fact, or use an ISO-8601 literal"
                                    .to_owned(),
                            ),
                        });
                    }
                }
            }
            body::BodyStmt::After(after) => {
                validate_body_effect_operands(
                    rule,
                    &after.body,
                    semantic,
                    binding_types,
                    diagnostics,
                );
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    validate_body_effect_operands(
                        rule,
                        &branch.body,
                        semantic,
                        binding_types,
                        diagnostics,
                    );
                }
            }
            body::BodyStmt::Branch(branch) => {
                validate_body_effect_operands(
                    rule,
                    &branch.then_body,
                    semantic,
                    binding_types,
                    diagnostics,
                );
                if let Some(else_body) = &branch.else_body {
                    validate_body_effect_operands(
                        rule,
                        else_body,
                        semantic,
                        binding_types,
                        diagnostics,
                    );
                }
            }
            body::BodyStmt::Handler(handler) => {
                validate_body_effect_operands(
                    rule,
                    &handler.body,
                    semantic,
                    binding_types,
                    diagnostics,
                );
            }
            _ => {}
        }
    }
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
                related: Vec::new(),
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

// `claim` stays bindable: `claim item as claim` is an established idiom and
// the trailing binding position is unambiguous.
const RESERVED_BINDING_KEYWORDS: &[&str] = &[
    "after", "askHuman", "call", "case", "coerce", "complete", "consume", "done", "emit", "fail",
    "invoke", "record", "tell", "when", "where",
];

fn validate_binding_name(
    rule: &RuleDecl,
    binding: &str,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if RESERVED_BINDING_KEYWORDS.contains(&binding) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
            span,
            message: format!(
                "rule `{}` binds reserved keyword `{binding}`",
                rule.name.name
            ),
            suggestion: Some(format!(
                "`{binding}` is a rule body keyword; choose another binding name"
            )),
        });
    }
}

fn closest_name<'a>(target: &str, candidates: impl Iterator<Item = &'a String>) -> Option<String> {
    let target_lower = target.to_lowercase();
    candidates
        .map(|candidate| {
            let distance = edit_distance(&target_lower, &candidate.to_lowercase());
            (distance, candidate)
        })
        .filter(|(distance, candidate)| {
            *distance <= 2 && *distance < target.len().min(candidate.len())
        })
        .min_by_key(|(distance, candidate)| (*distance, candidate.as_str().to_owned()))
        .map(|(_, candidate)| candidate.clone())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, a_char) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, b_char) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(a_char != b_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
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
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some((field, expr)) = record_field_assignment(line) else {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
            related: Vec::new(),
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
                diagnostics.push(Diagnostic { related: Vec::new(),
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
        } else if let Some(root) = dangling_value_root(expr, known_roots) {
            // A field access whose root is neither a bound name nor a special
            // root is a dangling reference (the binding does not exist).
            diagnostics.push(Diagnostic { related: Vec::new(),
                span: rule.body.span,
                message: format!(
                    "rule `{}` has unknown binding `{root}` in `record {record_schema}` field `{field}`",
                    rule.name.name
                ),
                suggestion: Some(
                    "reference a binding from a `when ... as name` clause, an effect `as` binding, or a `case` pattern"
                        .to_owned(),
                ),
            });
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

/// Roots valid in value positions without being author bindings: the
/// external-event payload and the coerce prompt context. An explicit allowlist
/// so genuine typos are still caught.
const SPECIAL_VALUE_ROOTS: &[&str] = &["external", "ctx"];

/// Collects every binding NAME a rule body introduces, from the parsed AST so it
/// is robust to multi-line prompts and nesting (the line-based effect collectors
/// only track `coerce`/`claim`, so `tell`/`exec`/etc. bindings are invisible to
/// `binding_types`). Used to reject dangling roots in value positions without
/// false-flagging valid effect results, `after` aliases, or case bindings.
fn collect_all_binding_names(statements: &[body::BodyStmt], out: &mut BTreeSet<String>) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => {
                if let Some(binding) = &effect.binding {
                    out.insert(binding.clone());
                }
            }
            body::BodyStmt::After(after) => {
                if let Some(alias) = &after.alias {
                    out.insert(alias.clone());
                }
                collect_all_binding_names(&after.body, out);
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    if let Some(binding) = &branch.binding {
                        out.insert(binding.clone());
                    }
                    collect_all_binding_names(&branch.body, out);
                }
            }
            body::BodyStmt::Branch(branch) => {
                collect_all_binding_names(&branch.then_body, out);
                if let Some(else_body) = &branch.else_body {
                    collect_all_binding_names(else_body, out);
                }
            }
            body::BodyStmt::Handler(handler) => {
                collect_all_binding_names(&handler.body, out);
            }
            // `redact … as <out>` introduces the projected binding `out`.
            body::BodyStmt::Redact { binding, .. } => {
                out.insert(binding.clone());
            }
            body::BodyStmt::Record(_)
            | body::BodyStmt::Done { .. }
            | body::BodyStmt::Terminal(_)
            | body::BodyStmt::Milestone { .. }
            | body::BodyStmt::Cancel { .. } => {}
        }
    }
}

/// Flags dangling roots in the field payloads of body-AST effects that the
/// line-based validators don't reach: `emit`/`notify` (`Notify`), `file item
/// into` (`QueueFile`), and ledger `append` (`LedgerAppend`). Uses the parsed
/// AST and the same root check as the record/coerce/tell/invoke validators.
fn validate_effect_field_roots(
    rule: &RuleDecl,
    statements: &[body::BodyStmt],
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in statements {
        match statement {
            body::BodyStmt::Effect(effect) => match &effect.kind {
                body::BodyEffectKind::Notify {
                    target_expr,
                    event,
                    fields,
                } => {
                    check_operand_root(
                        rule,
                        &format!("emit `{event}` target"),
                        target_expr,
                        known_roots,
                        diagnostics,
                    );
                    check_field_value_roots(
                        rule,
                        &format!("emit `{event}`"),
                        fields,
                        known_roots,
                        diagnostics,
                    );
                }
                body::BodyEffectKind::QueueFile { queue, fields } => {
                    check_field_value_roots(
                        rule,
                        &format!("file into `{queue}`"),
                        fields,
                        known_roots,
                        diagnostics,
                    );
                }
                body::BodyEffectKind::QueueFinish { item, fields } => {
                    check_operand_root(rule, "finish item", item, known_roots, diagnostics);
                    check_field_value_roots(rule, "finish", fields, known_roots, diagnostics);
                }
                body::BodyEffectKind::LedgerAppend { ledger, fields, .. } => {
                    check_field_value_roots(
                        rule,
                        &format!("append to `{ledger}`"),
                        fields,
                        known_roots,
                        diagnostics,
                    );
                }
                body::BodyEffectKind::LeaseAcquire {
                    resource, key_expr, ..
                } => {
                    check_operand_root(
                        rule,
                        &format!("acquire `{resource}` key"),
                        key_expr,
                        known_roots,
                        diagnostics,
                    );
                }
                body::BodyEffectKind::CounterConsume {
                    counter,
                    key_expr,
                    amount_expr,
                } => {
                    check_operand_root(
                        rule,
                        &format!("consume `{counter}` key"),
                        key_expr,
                        known_roots,
                        diagnostics,
                    );
                    check_operand_root(
                        rule,
                        &format!("consume `{counter}` amount"),
                        amount_expr,
                        known_roots,
                        diagnostics,
                    );
                }
                _ => {}
            },
            body::BodyStmt::After(after) => {
                validate_effect_field_roots(rule, &after.body, known_roots, diagnostics)
            }
            body::BodyStmt::Case(case) => {
                for branch in &case.branches {
                    validate_effect_field_roots(rule, &branch.body, known_roots, diagnostics);
                }
            }
            body::BodyStmt::Branch(branch) => {
                validate_effect_field_roots(rule, &branch.then_body, known_roots, diagnostics);
                if let Some(else_body) = &branch.else_body {
                    validate_effect_field_roots(rule, else_body, known_roots, diagnostics);
                }
            }
            body::BodyStmt::Handler(handler) => {
                validate_effect_field_roots(rule, &handler.body, known_roots, diagnostics)
            }
            _ => {}
        }
    }
}

/// The single source of truth for value-position root validation: returns the
/// dangling root of a single-path value expression — a `root.field…` access whose
/// root is neither a known binding nor a recognized special root — or `None`.
/// Bare atoms (agents, enum variants, literals) have no path and are ignored; the
/// `"`-guard skips values whose "path" was mis-extracted from inside a string
/// literal. Used by every value-position validator (record/terminal/coerce/tell/
/// invoke/effect payloads/operands).
fn dangling_value_root(value: &str, known_roots: &BTreeSet<String>) -> Option<String> {
    let (root, path) = expression_path(value)?;
    if !path.is_empty()
        && !value.contains('"')
        && !known_roots.contains(&root)
        && !SPECIAL_VALUE_ROOTS.contains(&root.as_str())
    {
        Some(root)
    } else {
        None
    }
}

/// Flags a dangling root in a single effect-operand expression (e.g. an
/// `emit ... to <target>` target, a lease/counter `for <key>` key). Same check
/// as the field/record validators.
fn check_operand_root(
    rule: &RuleDecl,
    context: &str,
    operand: &str,
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(root) = dangling_value_root(operand, known_roots) {
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: rule.body.span,
            message: format!(
                "rule `{}` has unknown binding `{root}` in {context} `{operand}`",
                rule.name.name
            ),
            suggestion: Some(
                "reference a binding from a `when ... as name` clause, an effect `as` binding, or a `case` pattern"
                    .to_owned(),
            ),
        });
    }
}

fn check_field_value_roots(
    rule: &RuleDecl,
    context: &str,
    fields: &[body::FieldAssign],
    known_roots: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for field in fields {
        match &field.value {
            body::FieldValue::Expr { source, .. } => {
                if let Some(root) = dangling_value_root(source, known_roots) {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: rule.body.span,
                        message: format!(
                            "rule `{}` has unknown binding `{root}` in {context} field `{}`",
                            rule.name.name, field.name
                        ),
                        suggestion: Some(
                            "reference a binding from a `when ... as name` clause, an effect `as` binding, or a `case` pattern"
                                .to_owned(),
                        ),
                    });
                }
            }
            body::FieldValue::Nested { fields, .. } => {
                check_field_value_roots(rule, context, fields, known_roots, diagnostics)
            }
            body::FieldValue::Shorthand => {}
        }
    }
}

/// The complete set of value-position binding roots for a rule: `when` bindings
/// plus every binding the body introduces, collected from the parsed AST.
fn known_roots_for_rule(rule: &RuleDecl) -> BTreeSet<String> {
    let mut roots: BTreeSet<String> = binding_types_for_rule(rule).into_keys().collect();
    let (body_ast, _) =
        body::parse_rule_body(&rule.body.text, rule.body.span.start, body::BodyMode::Rule);
    collect_all_binding_names(&body_ast.statements, &mut roots);
    roots
}

fn validate_record_blocks(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    known_roots: &BTreeSet<String>,
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
            validate_record_field(
                rule,
                &line,
                &schema,
                semantic,
                binding_types,
                known_roots,
                diagnostics,
            );
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
        // Single-line record `record X { f y }`: opens and closes on one line
        // (brace_delta 0), so the multi-line loop below never collects its fields,
        // leaving them unvalidated. Extract the inner content directly.
        if brace_delta(trimmed) == 0 && trimmed.contains('{') {
            if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
                if close > open {
                    blocks.push((
                        schema,
                        from_binding,
                        trimmed[open + 1..close].trim().to_owned(),
                    ));
                }
            }
            index += 1;
            continue;
        }
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
        if depth == 0 && trimmed.contains('{') {
            // Single-line block: `complete <name> { <fields> }` opens and closes
            // on this line, so its inner content never reaches the multi-line loop
            // below. Capture the content between the braces as the block body.
            if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
                if close > open {
                    let inner = trimmed[open + 1..close].trim();
                    if !inner.is_empty() {
                        terminal_lines.push(inner.to_owned());
                    }
                }
            }
            index += 1;
        } else {
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
                    related: Vec::new(),
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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
                        related: Vec::new(),
                        span: rule.body.span,
                        message: format!("field `{record_schema}.{field}` expects a map literal"),
                        suggestion: Some(format!("record `{field} {{ key value }}`")),
                    });
                    return;
                }
                Err(message) => {
                    diagnostics.push(Diagnostic {
                        related: Vec::new(),
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

#[allow(clippy::too_many_arguments)]
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
        related: Vec::new(),
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

#[allow(clippy::too_many_arguments)]
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
                related: Vec::new(),
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
                related: Vec::new(),
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
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: rule.body.span,
            message: format!(
                "field `{record_schema}.{field}` is missing required object field `{object_schema}.{required}`"
            ),
            suggestion: Some(format!("add `{required}` to the `{field}` object literal")),
        });
    }
}

#[allow(clippy::too_many_arguments)]
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
            related: Vec::new(),
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
                    related: Vec::new(),
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
            related: Vec::new(),
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
            related: Vec::new(),
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects an AgentRef value"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
        while self.consume_op("||") || self.consume_ident("or") {
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
        while self.consume_op("&&") || self.consume_ident("and") {
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
        let mut expr = self.parse_additive()?;
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
            let right = self.parse_additive()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_multiplicative()?;
        loop {
            let op = if self.consume_symbol('+') {
                Some(BinaryOp::Add)
            } else if self.consume_symbol('-') {
                Some(BinaryOp::Sub)
            } else {
                None
            };
            let Some(op) = op else {
                return Ok(expr);
            };
            let right = self.parse_multiplicative()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.consume_symbol('*') {
                Some(BinaryOp::Mul)
            } else if self.consume_symbol('/') {
                Some(BinaryOp::Div)
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
        // Prefix `not` binds looser than comparisons so `not x in y`
        // reads as `not (x in y)`; binary `not in` is handled by
        // parse_comparison before this prefix form is reached.
        if self.consume_ident("not") {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_comparison()?),
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
                if matches!(value.as_str(), "count" | "exists") && self.at_symbol('(') =>
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
            related: Vec::new(),
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects `{primitive}`"),
            suggestion: Some(format!("record a value compatible with `{primitive}`")),
        });
        return;
    }
    match (primitive, literal) {
        ("duration", LiteralExpr::String(value)) if parse_duration_seconds(value).is_none() => {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: rule.body.span,
                message: format!("field `{record_schema}.{field}` has invalid duration literal"),
                suggestion: Some("use an ISO-8601 duration such as `\"PT30M\"`".to_owned()),
            });
        }
        ("time", LiteralExpr::String(value)) if parse_time_epoch_seconds(value).is_none() => {
            diagnostics.push(Diagnostic {
                related: Vec::new(),
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
            related: Vec::new(),
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
            related: Vec::new(),
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
            related: Vec::new(),
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects one of its literal variants"),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            related: Vec::new(),
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
        IrEffectKind::Coerce
    } else if line.starts_with("claim ") {
        IrEffectKind::LoftClaim
    } else if line.starts_with("askHuman") {
        IrEffectKind::HumanAsk
    } else if line.starts_with("call ") || line.starts_with("recall ") {
        IrEffectKind::CapabilityCall
    } else if line.starts_with("emit ") {
        IrEffectKind::EventEmit
    } else if line.starts_with("invoke ") {
        IrEffectKind::WorkflowInvoke
    } else if line.starts_with("read ") {
        IrEffectKind::FileRead
    } else if line.starts_with("write ") {
        IrEffectKind::FileWrite
    } else if line.starts_with("import ") {
        IrEffectKind::FileImport
    } else if line.starts_with("export ") {
        IrEffectKind::FileExport
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
    let first = chars.next()?;
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

fn validate_rule_prompt_content_type_annotation(
    rule: &RuleDecl,
    line: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !(line.starts_with("tell ") || line.starts_with("askHuman") || line.starts_with("coerce ")) {
        return;
    }
    let Some(annotation) = malformed_prompt_content_type_annotation(line) else {
        return;
    };
    diagnostics.push(Diagnostic {
        related: Vec::new(),
        span: rule.body.span,
        message: format!(
            "rule `{}` has malformed multiline prompt content type `{annotation}`",
            rule.name.name
        ),
        suggestion: Some(
            "write a supported token such as `\"\"\"markdown` or put prompt text on the next line"
                .to_owned(),
        ),
    });
}

fn validate_coerce_prompt_content_type_annotations(
    coerce: &CoerceDecl,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for line in coerce.body.text.lines().map(str::trim) {
        if !line.starts_with("prompt ") {
            continue;
        }
        let Some(annotation) = malformed_prompt_content_type_annotation(line) else {
            continue;
        };
        diagnostics.push(Diagnostic { related: Vec::new(),
            span: coerce.body.span,
            message: format!(
                "coerce `{}` has malformed multiline prompt content type `{annotation}`",
                coerce.name.name
            ),
            suggestion: Some(
                "write a supported token such as `\"\"\"markdown` or put prompt text on the next line"
                    .to_owned(),
            ),
        });
    }
}

fn malformed_prompt_content_type_annotation(line: &str) -> Option<String> {
    let (_, tail) = line.split_once("\"\"\"")?;
    let candidate = tail.trim();
    if candidate.is_empty() || candidate.contains("\"\"\"") {
        return None;
    }
    let mut parts = candidate.split_whitespace();
    let first = parts.next()?;
    let has_extra_text = parts.next().is_some();
    let first_is_supported = is_supported_prompt_content_type(first);
    let first_is_annotation_shaped = first_is_supported || first.contains('/');
    if has_extra_text && first_is_annotation_shaped {
        return Some(candidate.to_owned());
    }
    if first.contains('/') && !first_is_supported {
        return Some(first.to_owned());
    }
    None
}

fn is_supported_prompt_content_type(candidate: &str) -> bool {
    if !is_prompt_content_type_token(candidate) {
        return false;
    }
    let normalized = candidate.to_ascii_lowercase();
    normalized.contains('/')
        || matches!(
            normalized.as_str(),
            "markdown" | "json" | "text" | "plain" | "html" | "xml" | "yaml" | "yml"
        )
}

fn is_prompt_content_type_token(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '+' | '-' | '_'))
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
    if rest.contains("=>") {
        return None;
    }
    let before_body = rest.split('{').next().unwrap_or(rest).trim();
    let mut parts = before_body.split_whitespace();
    let binding = parts.next()?.to_owned();
    let predicate = match parts.next()? {
        "succeeds" => DependencyPredicate::Succeeds,
        "fails" => DependencyPredicate::Fails,
        // `times out` / `cancelled` react only to that specific non-success
        // terminal status (spec/expression-kernel.md), mirroring succeeds/fails.
        "cancelled" => DependencyPredicate::Cancelled,
        "times" => {
            if parts.next()? != "out" {
                return None;
            }
            DependencyPredicate::TimedOut
        }
        // Coordination outcomes (spec/coordination.md) are completion-valued;
        // the arm dispatch happens on the outcome variant at lowering.
        "completes" | "held" | "contended" | "ok" | "over" => DependencyPredicate::Completes,
        // `after p reaches "<name>" [as m]` (Family C): consume the quoted
        // milestone name; the IR predicate is completion-shaped (runtime gating
        // keys on the milestone-specific `reached` fact).
        "reaches" => {
            let name = parts.next()?;
            if !(name.starts_with('"') && name.ends_with('"') && name.len() >= 2) {
                return None;
            }
            match (parts.next(), parts.next(), parts.next()) {
                (None, None, None) => {}
                (Some("as"), Some(alias), None) if is_identifier(alias) => {}
                _ => return None,
            }
            return Some((binding, DependencyPredicate::Completes));
        }
        _ => return None,
    };
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => {}
        (Some("as"), Some(alias), None) if is_identifier(alias) => {}
        _ => return None,
    }
    Some((binding, predicate))
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
        format_tags(&program.workflow_tags, &mut formatted);
        format_description(program.workflow_description.as_ref(), &mut formatted);
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
        Item::Queue(queue) => {
            push_line(formatted, format!("queue {} {{", queue.name.name));
            push_line(formatted, format!("  tracker {}", queue.tracker.name));
            push_line(formatted, "}");
        }
        Item::Channel(channel) => {
            push_line(formatted, format!("channel {} {{", channel.name.name));
            push_line(formatted, format!("  provider {}", channel.provider.name));
            if let Some(workspace) = &channel.workspace {
                push_line(formatted, format!("  workspace {}", workspace.name));
            }
            if let Some(destination) = &channel.destination {
                push_line(formatted, format!("  destination {:?}", destination.value));
            }
            push_line(formatted, "}");
        }
        Item::FileStore(file_store) => {
            push_line(formatted, format!("file store {} {{", file_store.name.name));
            push_line(formatted, format!("  root {:?}", file_store.root));
            let format_globs = |formatted: &mut String, direction: &str, globs: &[String]| {
                if !globs.is_empty() {
                    let rendered = globs
                        .iter()
                        .map(|glob| format!("{glob:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    push_line(formatted, format!("  allow {direction} [{rendered}]"));
                }
            };
            format_globs(formatted, "read", &file_store.read_globs);
            format_globs(formatted, "write", &file_store.write_globs);
            push_line(formatted, "}");
        }
        Item::Flow(flow) => format_flow(flow, formatted),
        Item::Action(action) => {
            let params = action
                .params
                .iter()
                .map(|param| format!("{} {}", param.name.name, param.ty.to_source()))
                .collect::<Vec<_>>()
                .join(", ");
            push_line(
                formatted,
                format!("action {}({params}) {{", action.name.name),
            );
            for line in action.body.text.lines() {
                if line.trim().is_empty() {
                    push_line(formatted, "");
                } else {
                    push_line(formatted, line.trim_end());
                }
            }
            push_line(formatted, "}");
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
        Item::Harness(harness) => format_harness(harness, formatted),
        Item::Agent(agent) => format_agent(agent, formatted),
        Item::Enum(enum_decl) => format_enum(enum_decl, formatted),
        Item::Event(event) => format_event(event, formatted),
        Item::Source(source) => format_source(source, formatted),
        Item::Test(test) => format_test(test, formatted),
        Item::Lease(lease) => {
            push_line(formatted, format!("lease {} {{", lease.name.name));
            push_line(formatted, format!("  key {}", lease.key_type.name));
            push_line(formatted, format!("  slots {}", lease.slots));
            push_line(formatted, format!("  ttl {}s", lease.ttl_seconds));
            push_line(formatted, "}");
        }
        Item::Ledger(ledger) => {
            push_line(formatted, format!("ledger {} {{", ledger.name.name));
            push_line(formatted, format!("  entry {}", ledger.entry_schema.name));
            push_line(
                formatted,
                format!("  partition by {}", ledger.partition_field.name),
            );
            push_line(formatted, format!("  retain {}s", ledger.retain_seconds));
            push_line(formatted, "}");
        }
        Item::Counter(counter) => {
            push_line(formatted, format!("counter {} {{", counter.name.name));
            push_line(formatted, format!("  key {}", counter.key_type.name));
            push_line(formatted, format!("  cap {}", counter.cap));
            push_line(formatted, format!("  reset {}", counter.reset));
            push_line(formatted, "}");
        }
        Item::Class(class_decl) => format_class(class_decl, formatted),
        Item::Table(table) => format_table(table, formatted),
        Item::Coerce(coerce) => format_coerce(coerce, formatted),
        Item::Assert(assertion) => {
            format_tags(&assertion.tags, formatted);
            format_description(assertion.description.as_ref(), formatted);
            push_line(formatted, format!("assert {}", assertion.expr));
        }
        Item::Rule(rule) => format_rule(rule, formatted),
    }
}

fn format_tags(tags: &[TagDecl], formatted: &mut String) {
    for tag in tags {
        push_line(formatted, format!("@{}", tag.name));
    }
}

fn format_description(description: Option<&StringLiteral>, formatted: &mut String) {
    if let Some(description) = description {
        push_line(formatted, format!("description {:?}", description.value));
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
    format_block_body(&apply.body.text, formatted);
    push_line(formatted, "}");
}

fn format_workflow(workflow: WorkflowDecl, formatted: &mut String) {
    format_tags(&workflow.tags, formatted);
    format_description(workflow.description.as_ref(), formatted);
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

fn format_harness(harness: HarnessDecl, formatted: &mut String) {
    push_line(
        formatted,
        format!("harness {}: {}", harness.name.name, harness.kind.name),
    );
}

fn format_agent(agent: AgentDecl, formatted: &mut String) {
    let harness = agent
        .harness
        .as_ref()
        .map(|harness| format!(" using {}", harness.name))
        .unwrap_or_default();
    push_line(
        formatted,
        format!("agent {}{} {{", agent.name.name, harness),
    );
    for field in agent.fields {
        match field {
            AgentField::Provider(provider) => {
                push_line(formatted, format!("  provider {}", provider.name));
            }
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
            AgentField::Tools(tools, _) => {
                let tools = tools
                    .into_iter()
                    .map(|tool| tool.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                push_line(formatted, format!("  tools [{tools}]"));
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
        if variant.fields.is_empty() {
            push_line(formatted, format!("  {}", variant.name.name));
            continue;
        }
        push_line(formatted, format!("  {} {{", variant.name.name));
        for field in variant.fields {
            push_line(
                formatted,
                format!("    {} {}", field.name.name, field.ty.to_source()),
            );
        }
        push_line(formatted, "  }");
    }
    push_line(formatted, "}");
}

fn format_time_of_day(time: TimeOfDay) -> String {
    format!("{:02}:{:02}", time.hour, time.minute)
}

fn format_weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

fn format_recurrence(recurrence: &Recurrence) -> String {
    match recurrence {
        Recurrence::At { time, .. } => format!("at {}", format_time_of_day(*time)),
        Recurrence::EveryDuration { source, .. } => format!("every {source}"),
        Recurrence::EveryCalendar { pattern, time, .. } => {
            let pattern = match pattern {
                CalendarPattern::Day => "day".to_owned(),
                CalendarPattern::Weekday => "weekday".to_owned(),
                CalendarPattern::Weekly(day) => format_weekday(*day).to_owned(),
            };
            format!("every {pattern} at {}", format_time_of_day(*time))
        }
    }
}

fn format_source_value(value: &SourceValue) -> String {
    match value {
        SourceValue::Path {
            binding, segments, ..
        } => {
            let mut text = binding.name.clone();
            for segment in segments {
                text.push('.');
                text.push_str(&segment.name);
            }
            text
        }
        SourceValue::String(literal) => format!("{:?}", literal.value),
        SourceValue::Number(number, _) => number.clone(),
    }
}

fn format_test_fields(fields: &[TestField], formatted: &mut String) {
    for field in fields {
        push_line(
            formatted,
            format!("    {} {}", field.name.name, field.value),
        );
    }
}

fn format_test(test: TestDecl, formatted: &mut String) {
    push_line(formatted, format!("test {:?} {{", test.name.value));
    if let Some(workflow) = &test.workflow {
        push_line(formatted, format!("  workflow {}", workflow.name));
    }
    for clause in &test.clauses {
        match clause {
            TestClause::Given(given) => match given {
                GivenClause::Input { fields, .. } => {
                    push_line(formatted, "  given input {");
                    format_test_fields(fields, formatted);
                    push_line(formatted, "  }");
                }
                GivenClause::Fact { ty, fields, .. } => {
                    push_line(formatted, format!("  given fact {} {{", ty.name));
                    format_test_fields(fields, formatted);
                    push_line(formatted, "  }");
                }
                GivenClause::Signal { name, fields, .. } => {
                    push_line(formatted, format!("  given signal {name} {{"));
                    format_test_fields(fields, formatted);
                    push_line(formatted, "  }");
                }
                GivenClause::Clock { at, .. } => {
                    push_line(formatted, format!("  given clock at {:?}", at.value));
                }
                GivenClause::Tracker {
                    tracker, fields, ..
                } => {
                    push_line(formatted, format!("  given tracker {tracker} issue {{"));
                    format_test_fields(fields, formatted);
                    push_line(formatted, "  }");
                }
                GivenClause::File {
                    store,
                    path,
                    content,
                    ..
                } => {
                    push_line(
                        formatted,
                        format!(
                            "  given file {store} at {:?} {:?}",
                            path.value, content.value
                        ),
                    );
                }
            },
            TestClause::Stub(stub) => {
                let surface = stub.surface.join(" ");
                match &stub.payload {
                    Some(StubPayload::Message(message)) => push_line(
                        formatted,
                        format!("  stub {surface} {} {:?}", stub.outcome, message.value),
                    ),
                    Some(StubPayload::Record(fields)) => {
                        push_line(formatted, format!("  stub {surface} {} {{", stub.outcome));
                        format_test_fields(fields, formatted);
                        push_line(formatted, "  }");
                    }
                    None => push_line(formatted, format!("  stub {surface} {}", stub.outcome)),
                }
            }
            TestClause::Run(run) => {
                let text = match &run.kind {
                    RunKind::UntilIdle => "run until idle".to_owned(),
                    RunKind::UntilWorkflowCompleted => "run until workflow completed".to_owned(),
                    RunKind::UntilWorkflowFailed => "run until workflow failed".to_owned(),
                    RunKind::ForSteps(steps) => format!("run for {steps} steps"),
                };
                push_line(formatted, format!("  {text}"));
            }
            TestClause::Expect(expect) => {
                push_line(
                    formatted,
                    format!("  {}", format_expect_target(&expect.target)),
                );
            }
        }
    }
    push_line(formatted, "}");
}

fn format_expect_target(target: &ExpectTarget) -> String {
    match target {
        ExpectTarget::WorkflowCompleted => "expect workflow completed".to_owned(),
        ExpectTarget::WorkflowFailed { failure: None } => "expect workflow failed".to_owned(),
        ExpectTarget::WorkflowFailed {
            failure: Some(failure),
        } => format!("expect workflow failed with {}", failure.name),
        ExpectTarget::Rule { name, status } => {
            let status = match status {
                RuleStatus::Fired => "fired".to_owned(),
                RuleStatus::FiredTimes(count) => format!("fired {count} times"),
                RuleStatus::DidNotFire => "did not fire".to_owned(),
            };
            format!("expect rule {} {status}", name.name)
        }
        ExpectTarget::Effect { name, status } => {
            let status = match status {
                EffectStatus::Requested => "requested",
                EffectStatus::Completed => "completed",
                EffectStatus::Failed => "failed",
            };
            format!("expect effect {name} {status}")
        }
        ExpectTarget::Diagnostic { code } => format!("expect diagnostic {code}"),
        ExpectTarget::NoEffect { name } => format!("expect no {name}"),
        ExpectTarget::Projection(query) => format!("expect {}", format_proj_query(query)),
    }
}

fn format_proj_query(query: &ProjQuery) -> String {
    match &query.kind {
        ProjQueryKind::Exists => format!("{} exists", query.noun),
        ProjQueryKind::Count { predicate, count } => {
            format!("{} count where {predicate} is {count}", query.noun)
        }
        ProjQueryKind::Where { predicate } => {
            format!("{} where {predicate}", query.noun)
        }
    }
}

fn format_source(source: SourceDecl, formatted: &mut String) {
    push_line(
        formatted,
        format!("source {} as {} {{", source.provider.name, source.name.name),
    );
    if let Some(clock) = &source.clock {
        push_line(
            formatted,
            format!("  {}", format_recurrence(&clock.recurrence)),
        );
        if let Some(timezone) = &clock.timezone {
            push_line(formatted, format!("  timezone {:?}", timezone.value));
        }
        match clock.missed {
            Some(MissedPolicy::Skip) => push_line(formatted, "  missed skip"),
            Some(MissedPolicy::Coalesce) => push_line(formatted, "  missed coalesce"),
            Some(MissedPolicy::CatchUp { limit }) => {
                push_line(formatted, format!("  missed catch_up limit {limit}"))
            }
            None => {}
        }
    }
    push_line(
        formatted,
        format!("  observe as {}", source.observe_binding.name),
    );
    push_line(formatted, format!("  emit {} {{", source.emit.signal));
    for field in &source.emit.fields {
        push_line(
            formatted,
            format!(
                "    {} {}",
                field.name.name,
                format_source_value(&field.value)
            ),
        );
    }
    push_line(formatted, "  }");
    push_line(formatted, "}");
}

fn format_event(event: EventDecl, formatted: &mut String) {
    push_line(formatted, format!("signal {} {{", event.name));
    for field in event.fields {
        push_line(
            formatted,
            format!("  {} {}", field.name.name, field.ty.to_source()),
        );
    }
    push_line(formatted, "}");
}

fn format_class(class_decl: ClassDecl, formatted: &mut String) {
    push_line(formatted, format!("class {} {{", class_decl.name.name));
    for field in class_decl.fields {
        let key = if field.is_key { " @key" } else { "" };
        push_line(
            formatted,
            format!("  {} {}{key}", field.name.name, field.ty.to_source()),
        );
    }
    push_line(formatted, "}");
}

fn format_table(table: TableDecl, formatted: &mut String) {
    format_tags(&table.tags, formatted);
    format_description(table.description.as_ref(), formatted);
    push_line(
        formatted,
        format!("table {} as {} [", table.name.name, table.schema.name),
    );
    for row in table.rows {
        push_line(formatted, "  {");
        for line in row.body.text.lines() {
            if line.trim().is_empty() {
                formatted.push('\n');
            } else {
                // `trim()` (not `trim_end()`): normalize the field to a fixed
                // 4-space indent rather than prepending to the row's existing
                // indent, which compounded every pass. Row bodies are flat field
                // lists, so a fixed indent is the canonical form.
                push_line(formatted, format!("    {}", line.trim()));
            }
        }
        push_line(formatted, "  }");
    }
    push_line(formatted, "]");
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
    format_block_body(&coerce.body.text, formatted);
    push_line(formatted, "}");
}

fn format_rule(rule: RuleDecl, formatted: &mut String) {
    format_tags(&rule.tags, formatted);
    format_description(rule.description.as_ref(), formatted);
    push_line(formatted, format!("rule {}", rule.name.name));
    for when in rule.whens {
        push_line(formatted, format!("  when {}", when.text));
    }
    push_line(formatted, "=> {");
    format_block_body(&rule.body.text, formatted);
    push_line(formatted, "}");
}

fn format_flow(flow: FlowDecl, formatted: &mut String) {
    format_tags(&flow.tags, formatted);
    format_description(flow.description.as_ref(), formatted);
    push_line(formatted, format!("flow {}", flow.name.name));
    for when in &flow.whens {
        push_line(formatted, format!("  when {}", when.text));
    }
    // A flow opens its body with a bare `{` on its own line (after the `when`
    // clauses), unlike a rule's `=> {`. The body is the raw source substring, so
    // `format_block_body` re-indents it (string-aware) and carries its comments.
    push_line(formatted, "{");
    format_block_body(&flow.body.text, formatted);
    push_line(formatted, "}");
}

/// Flat-prepend body emitter used where the output feeds IR construction (e.g.
/// a table's synthetic rule body, whose `body_hash` is part of program identity).
/// Kept byte-for-byte stable so the lowered IR / snapshots do not move; the
/// idempotent re-indenter for human formatting is [`format_block_body`].
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

/// Re-indent a rule/apply body to a canonical form derived from brace nesting,
/// so `whip fmt` is idempotent. Two concerns make this non-trivial:
///   - **Bracket nesting:** code lines are indented by their `{`/`[`/`(` depth
///     (string-aware via `scan_braces`), not by a flat prepend that compounds on
///     nested `record`/`complete` blocks.
///   - **Multi-line `"""..."""` strings:** the content is dedented to its common
///     indent and re-indented to the block depth (preserving relative structure).
///     This matches the single-pass canonical form AND is stable across passes,
///     where the old flat prepend grew the string content every time.
fn format_block_body(body: &str, formatted: &mut String) {
    if body.trim().is_empty() {
        return;
    }
    let lines: Vec<&str> = body.lines().collect();
    let mut index = 0;
    let mut depth: i32 = 1;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            formatted.push('\n');
            index += 1;
            continue;
        }
        let opens_with_closer = trimmed
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '}' | ']' | ')'));
        let line_depth = if opens_with_closer {
            (depth - 1).max(0)
        } else {
            depth
        };
        let prefix = "  ".repeat(line_depth as usize);
        let (delta, opens_triple) = scan_braces(trimmed);
        push_line(formatted, format!("{prefix}{trimmed}"));
        if opens_triple {
            // Collect the string content up to the closing `"""`.
            let mut end = index + 1;
            while end < lines.len() && lines[end].matches("\"\"\"").count().is_multiple_of(2) {
                end += 1;
            }
            let content = &lines[index + 1..end];
            let common = content
                .iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);
            for line in content {
                if line.trim().is_empty() {
                    formatted.push('\n');
                } else {
                    push_line(formatted, format!("{prefix}{}", &line[common..]));
                }
            }
            if end < lines.len() {
                // The closing-delimiter line, re-indented to the block depth.
                push_line(formatted, format!("{prefix}{}", lines[end].trim()));
            }
            index = end + 1;
        } else {
            index += 1;
        }
        depth = (depth + delta).max(0);
    }
}

/// Net bracket-depth change for one line, ignoring brackets inside strings.
/// Returns `(delta, opens_unclosed_triple)`: a `true` second element means the
/// line starts a `"""..."""` that does not close on the same line, so following
/// lines are string content. ASCII markers only — UTF-8 string bytes can't
/// false-match.
fn scan_braces(line: &str) -> (i32, bool) {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut delta = 0i32;
    let mut in_string = false;
    while index < bytes.len() {
        if in_string {
            match bytes[index] {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        if line[index..].starts_with("\"\"\"") {
            match line[index + 3..].find("\"\"\"") {
                Some(offset) => index += 3 + offset + 3,
                None => return (delta, true),
            }
            continue;
        }
        match bytes[index] {
            b'"' => in_string = true,
            b'{' | b'[' | b'(' => delta += 1,
            b'}' | b']' | b')' => delta -= 1,
            _ => {}
        }
        index += 1;
    }
    (delta, false)
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
    comments: Vec<Comment>,
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
    let mut comments = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if byte == b'#' {
            let end = skip_line(bytes, index + 1);
            comments.push(Comment {
                marker: CommentMarker::Hash,
                text: source[index + 1..end].trim().to_owned(),
                span: SourceSpan { start: index, end },
            });
            index = end;
            continue;
        }

        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            let end = skip_line(bytes, index + 2);
            comments.push(Comment {
                marker: CommentMarker::Slash,
                text: source[index + 2..end].trim().to_owned(),
                span: SourceSpan { start: index, end },
            });
            index = end;
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

        // Arithmetic operators appear inside guard and field-value
        // expressions, which are re-parsed from raw source slices; the
        // file-level lexer only needs to step over them.
        if matches!(byte, b'*' | b'/' | b'-') {
            index += 1;
            continue;
        }

        if b"{}[]()<>,?|.+!:@".contains(&byte) {
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
            related: Vec::new(),
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
        comments,
    }
}

/// Extract the comments from a source program, in source order. Comments are not
/// part of the token stream or AST; this is the entry point tooling (`whip fmt`,
/// the LSP) uses to preserve them.
pub fn lex_comments(source: &str) -> Vec<Comment> {
    lex(source).comments
}

/// Byte-span regions of string literals and comments in `source`. A tool that
/// edits identifier occurrences (e.g. `whip lsp` rename) consults these to avoid
/// touching text inside a prompt string or a comment — only code identifiers are
/// real references.
pub fn string_and_comment_spans(source: &str) -> Vec<SourceSpan> {
    let lexed = lex(source);
    let mut spans: Vec<SourceSpan> = lexed
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::String(_)))
        .map(|token| token.span)
        .collect();
    spans.extend(lexed.comments.iter().map(|comment| comment.span));
    spans
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
            related: Vec::new(),
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
        let mut workflow_tags = Vec::new();
        let mut workflow_description = None;
        let mut explicit_workflow_body = false;
        let mut workflows = Vec::new();
        let mut patterns = Vec::new();
        let mut items = Vec::new();
        let mut pending_tags = Vec::new();
        let mut pending_description = None;

        while !self.is_at_end() {
            if self.at_symbol('@') {
                if let Some(tag) = self.parse_tag() {
                    pending_tags.push(tag);
                }
            } else if self.at_ident("description") {
                self.parse_pending_description(&mut pending_description);
            } else if self.at_ident("workflow") {
                if let Some(parsed_workflow) = self.parse_workflow(
                    std::mem::take(&mut pending_tags),
                    pending_description.take(),
                ) {
                    if parsed_workflow.explicit_body {
                        workflows.push(parsed_workflow.decl);
                    } else {
                        if workflow.is_some() {
                            self.diagnostics.push(Diagnostic { related: Vec::new(),
                                span: parsed_workflow.decl.name.span,
                                message: "multiple implicit workflow headers are not supported"
                                    .to_owned(),
                                suggestion: Some(
                                    "use explicit `workflow Name { ... }` declarations with `--root`"
                                        .to_owned(),
                                ),
                            });
                        }
                        workflow_tags = parsed_workflow.decl.tags;
                        workflow_description = parsed_workflow.decl.description;
                        // A header-form workflow carries no block, so its only
                        // items are the compact-signature contracts (if any);
                        // those are top-level for a single-workflow program.
                        items.extend(parsed_workflow.decl.items);
                        workflow = Some(parsed_workflow.decl.name);
                        explicit_workflow_body = false;
                    }
                }
            } else if self.at_ident("pattern") {
                self.reject_pending_tags(&mut pending_tags, "pattern");
                self.reject_pending_description(&mut pending_description, "pattern");
                if let Some(pattern) = self.parse_pattern() {
                    patterns.push(pattern);
                }
            } else if let Some(item) =
                self.parse_declaration_item(&mut pending_tags, &mut pending_description)
            {
                items.push(item);
            } else if self.reject_gherkin_misuse() {
                continue;
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
            workflow_tags,
            workflow_description,
            explicit_workflow_body,
            workflows,
            patterns,
            items,
        }
    }

    fn parse_workflow(
        &mut self,
        tags: Vec<TagDecl>,
        description: Option<StringLiteral>,
    ) -> Option<ParsedWorkflow> {
        let start = self.expect_keyword("workflow")?.span.start;
        let name = self.expect_ident("workflow name")?;
        let mut explicit_body = false;
        let mut items = Vec::new();
        let mut end = name.span.end;
        // Optional compact contract signature: `Name(in: T, ...) -> Out [! Fail]`.
        // Desugars to the same `input`/`output`/`failure` contract decls as the
        // keyword form, with the output named `result` and the failure `error`
        // (the conventional names). Both forms are legal; `whip fmt` re-emits the
        // keyword lines (one canonical stored shape).
        if self.at_symbol('(') {
            if let Some((contracts, signature_end)) = self.parse_compact_contract_signature() {
                end = signature_end;
                items.extend(contracts.into_iter().map(Item::WorkflowContract));
            }
        }
        if self.at_symbol('{') {
            explicit_body = true;
            self.expect_symbol('{')?;
            let mut pending_tags = Vec::new();
            let mut pending_description = None;
            while !self.is_at_end() && !self.at_symbol('}') {
                if self.at_symbol('@') {
                    if let Some(tag) = self.parse_tag() {
                        pending_tags.push(tag);
                    }
                    continue;
                }
                if self.at_ident("description") {
                    self.parse_pending_description(&mut pending_description);
                    continue;
                }
                if self.at_ident("workflow") || self.at_ident("pattern") {
                    self.reject_pending_tags(&mut pending_tags, "workflow body declaration");
                    self.reject_pending_description(
                        &mut pending_description,
                        "workflow body declaration",
                    );
                    self.unexpected("workflow body declaration");
                    self.advance();
                    continue;
                }
                if let Some(item) =
                    self.parse_declaration_item(&mut pending_tags, &mut pending_description)
                {
                    items.push(item);
                } else if self.reject_gherkin_misuse() {
                    continue;
                } else {
                    if self.is_at_end() {
                        break;
                    }
                    self.reject_pending_tags(&mut pending_tags, "workflow body declaration");
                    self.reject_pending_description(
                        &mut pending_description,
                        "workflow body declaration",
                    );
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
                tags,
                description,
                items,
                span: SourceSpan { start, end },
            },
            explicit_body,
        })
    }

    /// Parses a compact contract signature `(name: Type, ...) -> Output [! Failure]`
    /// into the same contract decls the keyword form produces. The output binding
    /// is named `result` and the failure `error` — the conventional names used by
    /// `complete result` / `fail error`. Returns the contracts and the signature's
    /// end offset (so the workflow span covers it).
    fn parse_compact_contract_signature(&mut self) -> Option<(Vec<WorkflowContractDecl>, usize)> {
        self.expect_symbol('(')?;
        let mut contracts = Vec::new();
        while !self.is_at_end() && !self.at_symbol(')') {
            let name = self.expect_ident("workflow input name")?;
            self.expect_symbol(':')?;
            let ty = self.parse_type()?;
            let span = name.span.join(ty.span());
            contracts.push(WorkflowContractDecl {
                kind: WorkflowContractKind::Input,
                name,
                ty,
                span,
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
        self.expect_thin_arrow()?;
        let output_ty = self.parse_type()?;
        let output_span = output_ty.span();
        let mut end = output_span.end;
        contracts.push(WorkflowContractDecl {
            kind: WorkflowContractKind::Output,
            name: Ident {
                name: "result".to_owned(),
                span: output_span,
            },
            ty: output_ty,
            span: output_span,
        });
        if self.at_symbol('!') {
            self.advance();
            let failure_ty = self.parse_type()?;
            let failure_span = failure_ty.span();
            end = failure_span.end;
            contracts.push(WorkflowContractDecl {
                kind: WorkflowContractKind::Failure,
                name: Ident {
                    name: "error".to_owned(),
                    span: failure_span,
                },
                ty: failure_ty,
                span: failure_span,
            });
        }
        Some((contracts, end))
    }

    fn parse_tag(&mut self) -> Option<TagDecl> {
        let at = self.expect_symbol('@')?;
        let name_start = at.span.end;
        let mut name_end = name_start;
        for (offset, ch) in self.source[name_start..].char_indices() {
            if ch.is_whitespace() {
                break;
            }
            name_end = name_start + offset + ch.len_utf8();
        }
        let name = self.source[name_start..name_end].to_owned();
        while !self.is_at_end() && self.peek().is_some_and(|token| token.span.start < name_end) {
            self.advance();
        }
        let span = SourceSpan {
            start: at.span.start,
            end: name_end,
        };
        if name.is_empty() {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: "tag is missing a name".to_owned(),
                suggestion: Some("write a tag such as `@fixture`".to_owned()),
            });
            return None;
        }
        if !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
        {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!("tag `@{name}` contains unsupported characters"),
                suggestion: Some(
                    "use letters, digits, `_`, `-`, `.`, or `:` in tag names".to_owned(),
                ),
            });
            return None;
        }
        Some(TagDecl { name, span })
    }

    fn reject_pending_tags(&mut self, pending_tags: &mut Vec<TagDecl>, target: &str) {
        for tag in pending_tags.drain(..) {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: tag.span,
                message: format!("tag `@{}` cannot be attached to {target}", tag.name),
                suggestion: Some(
                    "place tags on workflows, matrices, assertions, or rules".to_owned(),
                ),
            });
        }
    }

    fn parse_pending_description(&mut self, pending_description: &mut Option<StringLiteral>) {
        let Some(description) = self.parse_description() else {
            return;
        };
        if let Some(previous) = pending_description.replace(description) {
            self.diagnostics.push(Diagnostic { related: Vec::new(),
                span: previous.span,
                message: "description is not attached to a declaration".to_owned(),
                suggestion: Some(
                    "place only one `description \"...\"` immediately before the target declaration"
                        .to_owned(),
                ),
            });
        }
    }

    fn parse_description(&mut self) -> Option<StringLiteral> {
        let description = self.expect_keyword("description")?;
        let Some(value) = self.expect_string("description string") else {
            return Some(StringLiteral {
                value: String::new(),
                span: description.span,
            });
        };
        Some(value)
    }

    fn reject_pending_description(
        &mut self,
        pending_description: &mut Option<StringLiteral>,
        target: &str,
    ) {
        if let Some(description) = pending_description.take() {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: description.span,
                message: format!("description cannot be attached to {target}"),
                suggestion: Some(
                    "place descriptions on workflows, matrices, assertions, or rules".to_owned(),
                ),
            });
        }
    }

    fn reject_gherkin_misuse(&mut self) -> bool {
        let Some(token) = self.peek() else {
            return false;
        };
        let TokenKind::Ident(keyword) = &token.kind else {
            return false;
        };
        if !is_gherkin_keyword(keyword) {
            return false;
        }
        let span = token.span;
        self.diagnostics.push(Diagnostic { related: Vec::new(),
            span,
            message: format!(
                "Gherkin keyword `{keyword}` is not WhippleScript workflow syntax"
            ),
            suggestion: Some(
                "use `workflow`, `table`, `rule ... when ... => { ... }`, and `assert` instead of free-text Given/When/Then steps"
                    .to_owned(),
            ),
        });
        self.advance_to_line_end(span.start);
        true
    }

    fn advance_to_line_end(&mut self, line_start: usize) {
        let line_end = self.source[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(self.source.len());
        while self.peek().is_some_and(|token| token.span.start < line_end) {
            self.advance();
        }
    }

    fn parse_declaration_item(
        &mut self,
        pending_tags: &mut Vec<TagDecl>,
        pending_description: &mut Option<StringLiteral>,
    ) -> Option<Item> {
        if self.at_ident("include") {
            self.reject_pending_tags(pending_tags, "include");
            self.reject_pending_description(pending_description, "include");
            self.parse_include().map(Item::Include)
        } else if self.at_ident("use") {
            self.reject_pending_tags(pending_tags, "use");
            self.reject_pending_description(pending_description, "use");
            self.parse_use().map(Item::Use)
        } else if self.at_ident("pattern") {
            self.reject_pending_tags(pending_tags, "pattern");
            self.reject_pending_description(pending_description, "pattern");
            self.parse_pattern().map(Item::Pattern)
        } else if self.at_ident("apply") {
            self.reject_pending_tags(pending_tags, "apply");
            self.reject_pending_description(pending_description, "apply");
            self.parse_apply().map(Item::Apply)
        } else if self.at_ident("input") || self.at_ident("output") || self.at_ident("failure") {
            self.reject_pending_tags(pending_tags, "workflow contract");
            self.reject_pending_description(pending_description, "workflow contract");
            self.parse_workflow_contract().map(Item::WorkflowContract)
        } else if self.at_ident("flow") {
            self.parse_flow(std::mem::take(pending_tags), pending_description.take())
                .map(Item::Flow)
        } else if self.at_ident("action") {
            self.reject_pending_tags(pending_tags, "action");
            self.reject_pending_description(pending_description, "action");
            self.parse_action().map(Item::Action)
        } else if self.at_ident("queue") {
            self.reject_pending_tags(pending_tags, "queue");
            self.reject_pending_description(pending_description, "queue");
            self.parse_queue().map(Item::Queue)
        } else if self.at_ident("channel") {
            self.reject_pending_tags(pending_tags, "channel");
            self.reject_pending_description(pending_description, "channel");
            self.parse_channel().map(Item::Channel)
        } else if self.at_ident("file") {
            self.reject_pending_tags(pending_tags, "file store");
            self.reject_pending_description(pending_description, "file store");
            self.parse_file_store().map(Item::FileStore)
        } else if self.at_ident("harness") {
            self.reject_pending_tags(pending_tags, "harness");
            self.reject_pending_description(pending_description, "harness");
            self.parse_harness().map(Item::Harness)
        } else if self.at_ident("agent") {
            self.reject_pending_tags(pending_tags, "agent");
            self.reject_pending_description(pending_description, "agent");
            self.parse_agent().map(Item::Agent)
        } else if self.at_ident("enum") {
            self.reject_pending_tags(pending_tags, "enum");
            self.reject_pending_description(pending_description, "enum");
            self.parse_enum().map(Item::Enum)
        } else if self.at_ident("signal") {
            self.reject_pending_tags(pending_tags, "signal");
            self.reject_pending_description(pending_description, "signal");
            self.parse_event().map(Item::Event)
        } else if self.at_ident("source") {
            self.reject_pending_tags(pending_tags, "source");
            self.reject_pending_description(pending_description, "source");
            self.parse_source().map(Item::Source)
        } else if self.at_ident("test") {
            self.reject_pending_tags(pending_tags, "test");
            self.reject_pending_description(pending_description, "test");
            self.parse_test().map(Item::Test)
        } else if self.at_ident("lease") {
            self.reject_pending_tags(pending_tags, "lease");
            self.reject_pending_description(pending_description, "lease");
            self.parse_lease().map(Item::Lease)
        } else if self.at_ident("ledger") {
            self.reject_pending_tags(pending_tags, "ledger");
            self.reject_pending_description(pending_description, "ledger");
            self.parse_ledger().map(Item::Ledger)
        } else if self.at_ident("counter") {
            self.reject_pending_tags(pending_tags, "counter");
            self.reject_pending_description(pending_description, "counter");
            self.parse_counter().map(Item::Counter)
        } else if self.at_ident("class") {
            self.reject_pending_tags(pending_tags, "class");
            self.reject_pending_description(pending_description, "class");
            self.parse_class().map(Item::Class)
        } else if self.at_ident("table") {
            self.parse_table(std::mem::take(pending_tags), pending_description.take())
                .map(Item::Table)
        } else if self.at_ident("coerce") {
            self.reject_pending_tags(pending_tags, "coerce");
            self.reject_pending_description(pending_description, "coerce");
            self.parse_coerce().map(Item::Coerce)
        } else if self.at_ident("assert") {
            self.parse_assert(std::mem::take(pending_tags), pending_description.take())
                .map(Item::Assert)
        } else if self.at_ident("rule") {
            self.parse_rule(std::mem::take(pending_tags), pending_description.take())
                .map(Item::Rule)
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
        let mut pending_tags = Vec::new();
        let mut pending_description = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            if self.at_symbol('@') {
                if let Some(tag) = self.parse_tag() {
                    pending_tags.push(tag);
                }
                continue;
            }
            if self.at_ident("description") {
                self.parse_pending_description(&mut pending_description);
                continue;
            }
            if self.at_ident("workflow") || self.at_ident("pattern") {
                self.reject_pending_tags(&mut pending_tags, "pattern body declaration");
                self.reject_pending_description(
                    &mut pending_description,
                    "pattern body declaration",
                );
                self.unexpected("pattern body declaration");
                self.advance();
                continue;
            }
            if let Some(item) =
                self.parse_declaration_item(&mut pending_tags, &mut pending_description)
            {
                items.push(item);
            } else if self.reject_gherkin_misuse() {
                continue;
            } else {
                if self.is_at_end() {
                    break;
                }
                self.reject_pending_tags(&mut pending_tags, "pattern body declaration");
                self.reject_pending_description(
                    &mut pending_description,
                    "pattern body declaration",
                );
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
            let removed_kind = self.advance().clone();
            let removed_label = match &removed_kind.kind {
                TokenKind::Ident(value) => value.as_str(),
                _ => "",
            };
            self.diagnostics.push(Diagnostic { related: Vec::new(),
                span: removed_kind.span,
                message: format!("`use {removed_label}` is no longer supported"),
                suggestion: Some(
                    "write `use memory` for package libraries; attach skills with `agent { skills [...] }`"
                        .to_owned(),
                ),
            });
        }
        Some(UseDecl {
            name: self.expect_use_name("package library name")?,
        })
    }

    /// Parses `<n><unit>` durations at declaration level (`ttl 10m`,
    /// `retain 90d`) — the lexer splits them into a number and a unit ident.
    fn parse_decl_duration_seconds(&mut self, label: &str) -> Option<u64> {
        let (value, span) = self.expect_u32(label)?;
        let unit = self.expect_ident(label)?;
        match body::parse_short_duration_seconds(&format!("{value}{}", unit.name)) {
            Some(seconds) if seconds > 0 => Some(seconds),
            _ => {
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: span.join(unit.span),
                    message: format!("invalid duration `{value}{}`", unit.name),
                    suggestion: Some("use `<n><unit>` with unit s, m, h, or d".to_owned()),
                });
                None
            }
        }
    }

    fn parse_lease(&mut self) -> Option<LeaseDecl> {
        let start = self.expect_keyword("lease")?.span.start;
        let name = self.expect_ident("lease name")?;
        self.expect_symbol('{')?;
        let mut key_type = None;
        let mut slots = 1u32;
        let mut ttl_seconds = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("lease field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "key" => key_type = self.expect_ident("key type"),
                "slots" => {
                    slots = self
                        .expect_u32("slots value")
                        .map(|(value, _)| value)
                        .unwrap_or(1);
                }
                "ttl" => ttl_seconds = self.parse_decl_duration_seconds("ttl duration"),
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown lease field `{other}`"),
                        suggestion: Some("lease fields are `key`, `slots`, and `ttl`".to_owned()),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let span = SourceSpan {
            start,
            end: close.span.end,
        };
        let (Some(key_type), Some(ttl_seconds)) = (key_type, ttl_seconds) else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!(
                    "lease `{}` must declare a `key` type and a `ttl` backstop",
                    name.name
                ),
                suggestion: Some(
                    "every lease is bounded: declare `key <Type>` and `ttl <duration>`".to_owned(),
                ),
            });
            return None;
        };
        Some(LeaseDecl {
            name,
            key_type,
            slots,
            ttl_seconds,
            span,
        })
    }

    fn parse_ledger(&mut self) -> Option<LedgerDecl> {
        let start = self.expect_keyword("ledger")?.span.start;
        let name = self.expect_ident("ledger name")?;
        self.expect_symbol('{')?;
        let mut entry_schema = None;
        let mut partition_field = None;
        let mut retain_seconds = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("ledger field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "entry" => entry_schema = self.expect_ident("entry schema"),
                "partition" => {
                    if self.at_ident("by") {
                        self.advance();
                    } else {
                        self.diagnostics.push(Diagnostic {
                            related: Vec::new(),
                            span: field.span,
                            message: "expected `by` after `partition`".to_owned(),
                            suggestion: Some("write `partition by <field>`".to_owned()),
                        });
                    }
                    partition_field = self.expect_ident("partition field");
                }
                "retain" => retain_seconds = self.parse_decl_duration_seconds("retain duration"),
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown ledger field `{other}`"),
                        suggestion: Some(
                            "ledger fields are `entry`, `partition by`, and `retain`".to_owned(),
                        ),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let span = SourceSpan {
            start,
            end: close.span.end,
        };
        let (Some(entry_schema), Some(partition_field), Some(retain_seconds)) =
            (entry_schema, partition_field, retain_seconds)
        else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!(
                    "ledger `{}` must declare `entry`, `partition by`, and `retain`",
                    name.name
                ),
                suggestion: Some(
                    "every ledger is bounded and partitioned: declare all three fields".to_owned(),
                ),
            });
            return None;
        };
        Some(LedgerDecl {
            name,
            entry_schema,
            partition_field,
            retain_seconds,
            span,
        })
    }

    fn parse_counter(&mut self) -> Option<CounterDecl> {
        let start = self.expect_keyword("counter")?.span.start;
        let name = self.expect_ident("counter name")?;
        self.expect_symbol('{')?;
        let mut key_type = None;
        let mut cap = None;
        let mut reset = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("counter field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "key" => key_type = self.expect_ident("key type"),
                "cap" => {
                    cap = self
                        .expect_u32("cap value")
                        .map(|(value, _)| i64::from(value))
                }
                "reset" => {
                    let period = self.expect_ident("reset period")?;
                    if !matches!(
                        period.name.as_str(),
                        "hourly" | "daily" | "weekly" | "monthly"
                    ) {
                        self.diagnostics.push(Diagnostic {
                            related: Vec::new(),
                            span: period.span,
                            message: format!("unknown reset period `{}`", period.name),
                            suggestion: Some(
                                "use `hourly`, `daily`, `weekly`, or `monthly`".to_owned(),
                            ),
                        });
                    }
                    reset = Some(period.name);
                }
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown counter field `{other}`"),
                        suggestion: Some("counter fields are `key`, `cap`, and `reset`".to_owned()),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let span = SourceSpan {
            start,
            end: close.span.end,
        };
        let (Some(key_type), Some(cap), Some(reset)) = (key_type, cap, reset) else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span,
                message: format!(
                    "counter `{}` must declare `key`, `cap`, and `reset`",
                    name.name
                ),
                suggestion: Some("every counter is bounded: declare all three fields".to_owned()),
            });
            return None;
        };
        Some(CounterDecl {
            name,
            key_type,
            cap,
            reset,
            span,
        })
    }

    fn parse_queue(&mut self) -> Option<QueueDecl> {
        let start = self.expect_keyword("queue")?.span.start;
        let name = self.expect_ident("queue name")?;
        self.expect_symbol('{')?;
        let mut tracker = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("queue field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "tracker" => {
                    tracker = self.expect_ident("tracker kind");
                }
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown queue field `{other}`"),
                        suggestion: Some("the only queue field is `tracker`".to_owned()),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let Some(tracker) = tracker else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start,
                    end: close.span.end,
                },
                message: format!("queue `{}` is missing a tracker", name.name),
                suggestion: Some("add `tracker builtin` inside the queue block".to_owned()),
            });
            return None;
        };
        Some(QueueDecl {
            name,
            tracker,
            span: SourceSpan {
                start,
                end: close.span.end,
            },
        })
    }

    fn parse_channel(&mut self) -> Option<ChannelDecl> {
        let start = self.expect_keyword("channel")?.span.start;
        let name = self.expect_ident("channel name")?;
        self.expect_symbol('{')?;
        let mut provider = None;
        let mut workspace = None;
        let mut destination = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("channel field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "provider" => {
                    provider = self.expect_ident("channel provider");
                }
                "workspace" => {
                    workspace = self.expect_ident("channel workspace");
                }
                "destination" => {
                    destination = self.expect_string("channel destination");
                }
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown channel field `{other}`"),
                        suggestion: Some(
                            "channel fields are `provider`, `workspace`, and `destination`"
                                .to_owned(),
                        ),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let Some(provider) = provider else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start,
                    end: close.span.end,
                },
                message: format!("channel `{}` is missing a provider", name.name),
                suggestion: Some("add `provider <name>` inside the channel block".to_owned()),
            });
            return None;
        };
        Some(ChannelDecl {
            name,
            provider,
            workspace,
            destination,
            span: SourceSpan {
                start,
                end: close.span.end,
            },
        })
    }

    fn parse_file_store(&mut self) -> Option<FileStoreDecl> {
        let start = self.expect_keyword("file")?.span.start;
        if !self.consume_ident("store") {
            self.expected("`store` after `file`");
            return None;
        }
        let name = self.expect_ident("file store name")?;
        self.expect_symbol('{')?;
        let mut root = None;
        let mut read_globs = Vec::new();
        let mut write_globs = Vec::new();
        let mut root_span = None;
        let mut read_span = None;
        let mut write_span = None;
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field) = self.expect_ident("file store field") else {
                self.synchronize_to_block_item();
                continue;
            };
            match field.name.as_str() {
                "root" => {
                    root_span = Some(field.span);
                    root = self
                        .expect_string("file store root")
                        .map(|literal| literal.value);
                }
                // `allow read [...]` / `allow write [...]`: narrow which paths
                // (relative to root) the store permits. Optional — an absent
                // clause means any path inside the root.
                "allow" => {
                    let Some(direction) = self.expect_ident("`read` or `write` after `allow`")
                    else {
                        self.synchronize_to_block_item();
                        continue;
                    };
                    let globs = self
                        .parse_string_list()
                        .map(|(literals, _)| {
                            literals.into_iter().map(|literal| literal.value).collect()
                        })
                        .unwrap_or_default();
                    match direction.name.as_str() {
                        "read" => {
                            read_span = Some(field.span);
                            read_globs = globs;
                        }
                        "write" => {
                            write_span = Some(field.span);
                            write_globs = globs;
                        }
                        other => {
                            self.diagnostics.push(Diagnostic {
                                related: Vec::new(),
                                span: direction.span,
                                message: format!("unknown `allow` direction `{other}`"),
                                suggestion: Some(
                                    "use `allow read [...]` or `allow write [...]`".to_owned(),
                                ),
                            });
                        }
                    }
                }
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: field.span,
                        message: format!("unknown file store field `{other}`"),
                        suggestion: Some(
                            "file store fields are `root`, `allow read [...]`, `allow write [...]`"
                                .to_owned(),
                        ),
                    });
                    self.synchronize_to_block_item();
                }
            }
        }
        let close = self.expect_symbol('}')?;
        let Some(root) = root else {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start,
                    end: close.span.end,
                },
                message: format!("file store `{}` is missing a root", name.name),
                suggestion: Some("add `root \"<dir>\"` inside the file store block".to_owned()),
            });
            return None;
        };
        Some(FileStoreDecl {
            name,
            root,
            read_globs,
            write_globs,
            root_span,
            read_span,
            write_span,
            span: SourceSpan {
                start,
                end: close.span.end,
            },
        })
    }

    fn parse_harness(&mut self) -> Option<HarnessDecl> {
        let start = self.expect_keyword("harness")?.span.start;
        let name = self.expect_ident("harness name")?;
        self.expect_symbol(':')?;
        let kind = self.expect_ident("harness kind")?;
        let span = SourceSpan {
            start,
            end: kind.span.end,
        };
        Some(HarnessDecl { name, kind, span })
    }

    fn parse_agent(&mut self) -> Option<AgentDecl> {
        let start = self.expect_keyword("agent")?.span.start;
        let name = self.expect_ident("agent name")?;
        let harness = if self.at_ident("using") {
            self.advance();
            Some(self.expect_ident("harness name")?)
        } else {
            None
        };
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();

        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field_name) = self.expect_ident("agent field") else {
                self.synchronize_to_block_item();
                continue;
            };

            match field_name.name.as_str() {
                "provider" => {
                    if let Some(provider) = self.expect_ident("provider name") {
                        fields.push(AgentField::Provider(provider));
                    } else {
                        self.synchronize_to_block_item();
                    }
                }
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
                "tools" => {
                    if let Some((tools, span)) = self.parse_ident_list() {
                        fields.push(AgentField::Tools(tools, span));
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
            harness,
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
            let Some(variant) = self.expect_ident("enum variant") else {
                self.synchronize_to_block_item();
                continue;
            };
            // A brace body makes this a data-carrying variant; the body
            // reuses the class field grammar (sum types, spec/sum-types.md).
            let mut fields = Vec::new();
            let mut end = variant.span.end;
            if self.at_symbol('{') {
                self.expect_symbol('{');
                while !self.is_at_end() && !self.at_symbol('}') {
                    let Some(field_name) = self.expect_ident("variant field name") else {
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
                        is_key: false,
                        presence_condition: None,
                    });
                }
                if let Some(close) = self.expect_symbol('}') {
                    end = close.span.end;
                }
            }
            let span = SourceSpan {
                start: variant.span.start,
                end,
            };
            variants.push(EnumVariantDecl {
                name: variant,
                fields,
                span,
            });
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

    fn parse_event(&mut self) -> Option<EventDecl> {
        let start = self.expect_keyword("signal")?.span.start;
        // Dotted lowercase name (`deploy.finished`), matching the `when fact`
        // convention and distinct from PascalCase classes.
        let first = self.expect_ident("signal name")?;
        let mut name = first.name.clone();
        let mut name_span = first.span;
        while self.at_symbol('.') {
            self.expect_symbol('.');
            let segment = self.expect_ident("signal name segment")?;
            name.push('.');
            name.push_str(&segment.name);
            name_span = name_span.join(segment.span);
        }
        if !name.contains('.')
            || name
                .split('.')
                .any(|segment| segment.chars().next().is_some_and(char::is_uppercase))
        {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: name_span,
                message: format!("signal name `{name}` must be dotted lowercase"),
                suggestion: Some(
                    "use a dotted lowercase name such as `deploy.finished`".to_owned(),
                ),
            });
        }
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field_name) = self.expect_ident("signal field name") else {
                self.synchronize_to_block_item();
                continue;
            };
            let Some(ty) = self.parse_type() else {
                self.synchronize_to_block_item();
                continue;
            };
            let presence_condition = self.parse_field_presence_condition();
            fields.push(ClassField {
                span: field_name.span.join(ty.span()),
                name: field_name,
                ty,
                is_key: false,
                presence_condition,
            });
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some(EventDecl {
            name,
            name_span,
            fields,
            span: SourceSpan { start, end },
        })
    }

    fn last_span_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map(|token| token.span.end)
            .unwrap_or(0)
    }

    fn parse_dotted_name(&mut self, label: &str) -> Option<String> {
        let first = self.expect_ident(label)?;
        let mut name = first.name.clone();
        while self.at_symbol('.') {
            self.advance();
            let segment = self.expect_ident(label)?;
            name.push('.');
            name.push_str(&segment.name);
        }
        Some(name)
    }

    /// Capture the source text of an expression from the current token to end of
    /// line (the `assert`/guard idiom), advancing past the consumed tokens.
    fn capture_expr_to_line_end(&mut self) -> (String, SourceSpan) {
        let start = self
            .peek()
            .map(|token| token.span.start)
            .unwrap_or(self.source.len());
        let line_end = self.source[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(self.source.len());
        let mut end = start;
        while !self.is_at_end() {
            let Some(token) = self.peek() else { break };
            if token.span.start >= line_end {
                break;
            }
            let token_end = token.span.end.min(line_end);
            self.advance();
            end = token_end;
        }
        let span = SourceSpan { start, end };
        trimmed_source_text(self.source_text(span), span)
    }

    /// Capture source text up to (but not including) a terminator identifier or a
    /// closing brace — used for a predicate bounded by `is`.
    fn capture_expr_until_ident(&mut self, terminator: &str) -> (String, SourceSpan) {
        let start = self
            .peek()
            .map(|token| token.span.start)
            .unwrap_or(self.source.len());
        let mut end = start;
        while !self.is_at_end() && !self.at_ident(terminator) && !self.at_symbol('}') {
            let Some(token) = self.peek() else { break };
            let token_end = token.span.end;
            self.advance();
            end = token_end;
        }
        let span = SourceSpan { start, end };
        trimmed_source_text(self.source_text(span), span)
    }

    fn parse_test(&mut self) -> Option<TestDecl> {
        let start = self.expect_keyword("test")?.span.start;
        let name = self.expect_string("test name")?;
        let open = self.expect_symbol('{')?;
        let mut workflow = None;
        let mut clauses = Vec::new();
        while !self.is_at_end() && !self.at_symbol('}') {
            if self.at_ident("workflow") {
                self.advance();
                match self.expect_ident("workflow name") {
                    Some(name) => {
                        if workflow.is_some() {
                            self.diagnostics.push(Diagnostic {
                                related: Vec::new(),
                                span: name.span,
                                message: "a test scenario binds at most one `workflow`".to_owned(),
                                suggestion: Some(
                                    "remove the extra `workflow <Name>` header".to_owned(),
                                ),
                            });
                        }
                        workflow = Some(name);
                    }
                    None => self.synchronize_to_block_item(),
                }
            } else if self.at_ident("given") {
                match self.parse_given() {
                    Some(clause) => clauses.push(TestClause::Given(clause)),
                    None => self.synchronize_to_block_item(),
                }
            } else if self.at_ident("stub") {
                match self.parse_stub() {
                    Some(clause) => clauses.push(TestClause::Stub(clause)),
                    None => self.synchronize_to_block_item(),
                }
            } else if self.at_ident("run") {
                match self.parse_run() {
                    Some(clause) => clauses.push(TestClause::Run(clause)),
                    None => self.synchronize_to_block_item(),
                }
            } else if self.at_ident("expect") {
                match self.parse_expect() {
                    Some(clause) => clauses.push(TestClause::Expect(clause)),
                    None => self.synchronize_to_block_item(),
                }
            } else {
                self.unexpected("a test clause (`workflow`, `given`, `stub`, `run`, or `expect`)");
                self.synchronize_to_block_item();
            }
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some(TestDecl {
            name,
            workflow,
            clauses,
            span: SourceSpan { start, end },
        })
    }

    fn parse_test_record(&mut self) -> Option<(Vec<TestField>, usize)> {
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(name) = self.expect_ident("test field name") else {
                self.synchronize_to_block_item();
                continue;
            };
            let (value, value_span) = self.capture_expr_to_line_end();
            fields.push(TestField {
                span: name.span.join(value_span),
                name,
                value,
            });
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some((fields, end))
    }

    fn parse_given(&mut self) -> Option<GivenClause> {
        let start = self.expect_keyword("given")?.span.start;
        if self.consume_ident("input") {
            let (fields, end) = self.parse_test_record()?;
            Some(GivenClause::Input {
                fields,
                span: SourceSpan { start, end },
            })
        } else if self.consume_ident("fact") {
            let ty = self.expect_ident("fact type")?;
            let (fields, end) = self.parse_test_record()?;
            Some(GivenClause::Fact {
                ty,
                fields,
                span: SourceSpan { start, end },
            })
        } else if self.consume_ident("signal") {
            let name = self.parse_dotted_name("signal name")?;
            let (fields, end) = self.parse_test_record()?;
            Some(GivenClause::Signal {
                name,
                fields,
                span: SourceSpan { start, end },
            })
        } else if self.consume_ident("clock") {
            if !self.consume_ident("at") {
                self.expected("`at <timestamp>` after `given clock`");
            }
            let at = self.expect_string("clock timestamp")?;
            let end = at.span.end;
            Some(GivenClause::Clock {
                at,
                span: SourceSpan { start, end },
            })
        } else if self.consume_ident("tracker") {
            let tracker = self.parse_dotted_name("tracker name")?;
            if !self.consume_ident("issue") {
                self.expected("`issue { … }` after `given tracker <name>`");
            }
            let (fields, end) = self.parse_test_record()?;
            Some(GivenClause::Tracker {
                tracker,
                fields,
                span: SourceSpan { start, end },
            })
        } else if self.consume_ident("file") {
            let store = self.parse_dotted_name("file store name")?;
            if !self.consume_ident("at") {
                self.expected("`at <path> \"<content>\"` after `given file <store>`");
            }
            let path = self.expect_string("file path")?;
            let content = self.expect_string("file content")?;
            let end = content.span.end;
            Some(GivenClause::File {
                store,
                path,
                content,
                span: SourceSpan { start, end },
            })
        } else {
            self.unexpected(
                "`input`, `fact`, `signal`, `clock`, `tracker`, or `file` after `given`",
            );
            None
        }
    }

    fn parse_stub(&mut self) -> Option<StubClause> {
        let start = self.expect_keyword("stub")?.span.start;
        // Surface path: dotted-name segments up to the outcome, all on the `stub`
        // line. The trailing segment (before a `{`, string, or end-of-line) is the
        // outcome; the rest is the surface. v0 keeps this lexical: at least one
        // surface segment + one outcome.
        let line_end = self.source[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(self.source.len());
        let mut segments = Vec::new();
        while matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::Ident(_))
        ) && self.peek().is_some_and(|token| token.span.start < line_end)
        {
            match self.parse_dotted_name("stub surface") {
                Some(segment) => segments.push(segment),
                None => break,
            }
        }
        if segments.len() < 2 {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start,
                    end: self.last_span_end(),
                },
                message: "stub needs a surface and an outcome (e.g. `stub agent triager succeeds`)"
                    .to_owned(),
                suggestion: Some("write `stub <surface...> <outcome> [payload]`".to_owned()),
            });
            return None;
        }
        let outcome = segments.pop().expect("outcome present");
        let surface = segments;
        let payload = if self.at_symbol('{') {
            let (fields, _) = self.parse_test_record()?;
            Some(StubPayload::Record(fields))
        } else if matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::String(_))
        ) {
            Some(StubPayload::Message(self.expect_string("stub message")?))
        } else {
            None
        };
        let end = self.last_span_end();
        Some(StubClause {
            surface,
            outcome,
            payload,
            span: SourceSpan { start, end },
        })
    }

    fn parse_run(&mut self) -> Option<RunClause> {
        let start = self.expect_keyword("run")?.span.start;
        let kind = if self.consume_ident("until") {
            if self.consume_ident("idle") {
                RunKind::UntilIdle
            } else if self.consume_ident("workflow") {
                if self.consume_ident("completed") {
                    RunKind::UntilWorkflowCompleted
                } else if self.consume_ident("failed") {
                    RunKind::UntilWorkflowFailed
                } else {
                    self.expected("`completed` or `failed` after `workflow`");
                    return None;
                }
            } else {
                self.expected("`idle` or `workflow completed|failed` after `until`");
                return None;
            }
        } else if self.consume_ident("for") {
            let (steps, _) = self.expect_u32("step count")?;
            if !self.consume_ident("steps") {
                self.expected("`steps` after the step count");
            }
            RunKind::ForSteps(steps)
        } else {
            self.expected("`until ...` or `for <N> steps` after `run`");
            return None;
        };
        let end = self.last_span_end();
        Some(RunClause {
            kind,
            span: SourceSpan { start, end },
        })
    }

    fn parse_expect(&mut self) -> Option<ExpectClause> {
        let start = self.expect_keyword("expect")?.span.start;
        let target = if self.consume_ident("workflow") {
            if self.consume_ident("completed") {
                ExpectTarget::WorkflowCompleted
            } else if self.consume_ident("failed") {
                let failure = if self.consume_ident("with") {
                    self.expect_ident("failure type")
                } else {
                    None
                };
                ExpectTarget::WorkflowFailed { failure }
            } else {
                self.expected("`completed` or `failed` after `workflow`");
                return None;
            }
        } else if self.consume_ident("rule") {
            let name = self.expect_ident("rule name")?;
            let status = if self.consume_ident("fired") {
                if matches!(
                    self.peek().map(|token| &token.kind),
                    Some(TokenKind::Number(_))
                ) {
                    let (count, _) = self.expect_u32("fired count")?;
                    if !self.consume_ident("times") {
                        self.expected("`times` after the fired count");
                    }
                    RuleStatus::FiredTimes(count)
                } else {
                    RuleStatus::Fired
                }
            } else if self.consume_ident("did") {
                if !self.consume_ident("not") {
                    self.expected("`not` in `did not fire`");
                }
                if !self.consume_ident("fire") {
                    self.expected("`fire` in `did not fire`");
                }
                RuleStatus::DidNotFire
            } else {
                self.expected("`fired`, `fired <N> times`, or `did not fire`");
                return None;
            };
            ExpectTarget::Rule { name, status }
        } else if self.consume_ident("effect") {
            let name = self.parse_dotted_name("effect name")?;
            let status = if self.consume_ident("requested") {
                EffectStatus::Requested
            } else if self.consume_ident("completed") {
                EffectStatus::Completed
            } else if self.consume_ident("failed") {
                EffectStatus::Failed
            } else {
                self.expected("`requested`, `completed`, or `failed` after the effect name");
                return None;
            };
            ExpectTarget::Effect { name, status }
        } else if self.consume_ident("diagnostic") {
            let code = self.parse_dotted_name("diagnostic code")?;
            ExpectTarget::Diagnostic { code }
        } else if self.consume_ident("no") {
            let name = self.parse_dotted_name("forbidden effect name")?;
            ExpectTarget::NoEffect { name }
        } else {
            let noun = self.parse_dotted_name("projection noun")?;
            let kind = self.parse_proj_query_kind()?;
            let end = self.last_span_end();
            ExpectTarget::Projection(ProjQuery {
                noun,
                kind,
                span: SourceSpan { start, end },
            })
        };
        let end = self.last_span_end();
        Some(ExpectClause {
            target,
            span: SourceSpan { start, end },
        })
    }

    fn parse_proj_query_kind(&mut self) -> Option<ProjQueryKind> {
        if self.consume_ident("exists") {
            return Some(ProjQueryKind::Exists);
        }
        if self.consume_ident("count") {
            if !self.consume_ident("where") {
                self.expected("`where <predicate> is <N>` after `count`");
                return None;
            }
            let (predicate, _) = self.capture_expr_until_ident("is");
            if !self.consume_ident("is") {
                self.expected("`is <N>` after the count predicate");
                return None;
            }
            let (count, _) = self.expect_u32("count value")?;
            return Some(ProjQueryKind::Count { predicate, count });
        }
        if self.consume_ident("where") {
            let (predicate, _) = self.capture_expr_to_line_end();
            return Some(ProjQueryKind::Where { predicate });
        }
        self.expected("`exists`, `count where ... is <N>`, or `where ...`");
        None
    }

    fn parse_source(&mut self) -> Option<SourceDecl> {
        let start = self.expect_keyword("source")?.span.start;
        let provider = self.expect_ident("source provider")?;
        let is_clock = provider.name == "clock";
        if !self.consume_ident("as") {
            self.expected("`as <name>` after the source provider");
            return None;
        }
        let name = self.expect_ident("source name")?;
        let open = self.expect_symbol('{')?;

        let mut recurrence: Option<Recurrence> = None;
        let mut timezone: Option<StringLiteral> = None;
        let mut missed: Option<MissedPolicy> = None;
        let mut observe_binding: Option<Ident> = None;
        let mut emit: Option<SourceEmit> = None;

        while !self.is_at_end() && !self.at_symbol('}') {
            if self.at_ident("every") || self.at_ident("at") {
                if let Some(parsed) = self.parse_recurrence() {
                    recurrence = Some(parsed);
                } else {
                    self.synchronize_to_block_item();
                }
            } else if self.at_ident("timezone") {
                self.advance();
                timezone = self.expect_string("timezone string");
            } else if self.at_ident("missed") {
                missed = self.parse_missed_policy();
            } else if self.at_ident("observe") {
                self.advance();
                if !self.consume_ident("as") {
                    self.expected("`as <binding>` after `observe`");
                }
                observe_binding = self.expect_ident("observe binding");
            } else if self.at_ident("emit") {
                emit = self.parse_source_emit();
            } else {
                self.unexpected(
                    "a source clause (`every`/`at`, `timezone`, `missed`, `observe`, `emit`)",
                );
                self.synchronize_to_block_item();
            }
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        let span = SourceSpan { start, end };

        let observe_binding = match observe_binding {
            Some(binding) => binding,
            None => {
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!("source `{}` must declare `observe as <binding>`", name.name),
                    suggestion: Some("add `observe as tick`".to_owned()),
                });
                return None;
            }
        };
        let emit = match emit {
            Some(emit) => emit,
            None => {
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "source `{}` must declare `emit <signal> {{ ... }}`",
                        name.name
                    ),
                    suggestion: Some("add `emit triage.tick { ... }`".to_owned()),
                });
                return None;
            }
        };

        let clock = if is_clock {
            let recurrence = match recurrence {
                Some(recurrence) => recurrence,
                None => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span,
                        message: format!("clock source `{}` must declare a recurrence", name.name),
                        suggestion: Some(
                            "add `every weekday at 09:00`, `every 5m`, or `at 09:00`".to_owned(),
                        ),
                    });
                    return None;
                }
            };
            Some(ClockPolicy {
                recurrence,
                timezone,
                missed,
                span,
            })
        } else {
            if recurrence.is_some() || timezone.is_some() || missed.is_some() {
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: format!(
                        "source `{}` uses clock-only clauses but its provider is `{}`, not `clock`",
                        name.name, provider.name
                    ),
                    suggestion: Some(
                        "use `source clock as ...` for recurrence, timezone, or missed clauses"
                            .to_owned(),
                    ),
                });
            }
            None
        };

        Some(SourceDecl {
            name,
            provider,
            clock,
            observe_binding,
            emit,
            span,
        })
    }

    fn parse_recurrence(&mut self) -> Option<Recurrence> {
        if self.at_ident("at") {
            let at = self.expect_keyword("at")?;
            let time = self.parse_time_of_day()?;
            return Some(Recurrence::At {
                span: at.span.join(time.span),
                time,
            });
        }
        let every = self.expect_keyword("every")?;
        if matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::Number(_))
        ) {
            let (value, _) = self.expect_u32("recurrence interval")?;
            let unit = self.expect_ident("duration unit (`s`, `m`, `h`, or `d`)")?;
            let seconds = match unit.name.as_str() {
                "s" => value as u64,
                "m" => value as u64 * 60,
                "h" => value as u64 * 3_600,
                "d" => value as u64 * 86_400,
                other => {
                    self.diagnostics.push(Diagnostic {
                        related: Vec::new(),
                        span: unit.span,
                        message: format!("unknown duration unit `{other}`"),
                        suggestion: Some("use `s`, `m`, `h`, or `d`".to_owned()),
                    });
                    return None;
                }
            };
            return Some(Recurrence::EveryDuration {
                seconds,
                source: format!("{value}{}", unit.name),
                span: every.span.join(unit.span),
            });
        }
        let pattern_ident =
            self.expect_ident("calendar pattern (`day`, `weekday`, or a weekday)")?;
        let pattern = match pattern_ident.name.as_str() {
            "day" => CalendarPattern::Day,
            "weekday" => CalendarPattern::Weekday,
            "monday" => CalendarPattern::Weekly(Weekday::Monday),
            "tuesday" => CalendarPattern::Weekly(Weekday::Tuesday),
            "wednesday" => CalendarPattern::Weekly(Weekday::Wednesday),
            "thursday" => CalendarPattern::Weekly(Weekday::Thursday),
            "friday" => CalendarPattern::Weekly(Weekday::Friday),
            "saturday" => CalendarPattern::Weekly(Weekday::Saturday),
            "sunday" => CalendarPattern::Weekly(Weekday::Sunday),
            other => {
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span: pattern_ident.span,
                    message: format!("unknown calendar pattern `{other}`"),
                    suggestion: Some(
                        "use `day`, `weekday`, or a weekday such as `monday`".to_owned(),
                    ),
                });
                return None;
            }
        };
        if !self.consume_ident("at") {
            self.expected("`at <hh:mm>` after the calendar pattern");
            return None;
        }
        let time = self.parse_time_of_day()?;
        Some(Recurrence::EveryCalendar {
            pattern,
            span: every.span.join(time.span),
            time,
        })
    }

    fn parse_time_of_day(&mut self) -> Option<TimeOfDay> {
        let (hour, hour_span) = self.expect_u32("hour")?;
        self.expect_symbol(':')?;
        let (minute, minute_span) = self.expect_u32("minute")?;
        if hour > 23 || minute > 59 {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: hour_span.join(minute_span),
                message: format!("invalid time of day `{hour:02}:{minute:02}`"),
                suggestion: Some("use a 24-hour `hh:mm` such as `09:00`".to_owned()),
            });
            return None;
        }
        Some(TimeOfDay {
            hour: hour as u8,
            minute: minute as u8,
            span: hour_span.join(minute_span),
        })
    }

    fn parse_missed_policy(&mut self) -> Option<MissedPolicy> {
        self.expect_keyword("missed")?;
        if self.consume_ident("skip") {
            return Some(MissedPolicy::Skip);
        }
        if self.consume_ident("coalesce") {
            return Some(MissedPolicy::Coalesce);
        }
        if self.consume_ident("catch_up") {
            if !self.consume_ident("limit") {
                self.expected("`limit <N>` after `catch_up`");
                return None;
            }
            let (limit, _) = self.expect_u32("catch_up limit")?;
            return Some(MissedPolicy::CatchUp { limit });
        }
        self.expected("`skip`, `coalesce`, or `catch_up limit <N>`");
        None
    }

    fn parse_source_emit(&mut self) -> Option<SourceEmit> {
        let emit = self.expect_keyword("emit")?;
        let first = self.expect_ident("emit signal name")?;
        let mut signal = first.name.clone();
        let mut signal_span = first.span;
        while self.at_symbol('.') {
            self.advance();
            let segment = self.expect_ident("signal name segment")?;
            signal.push('.');
            signal.push_str(&segment.name);
            signal_span = signal_span.join(segment.span);
        }
        let open = self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.is_at_end() && !self.at_symbol('}') {
            let Some(field_name) = self.expect_ident("emit field name") else {
                self.synchronize_to_block_item();
                continue;
            };
            let Some(value) = self.parse_source_value() else {
                self.synchronize_to_block_item();
                continue;
            };
            let value_span = match &value {
                SourceValue::Path { span, .. } => *span,
                SourceValue::String(literal) => literal.span,
                SourceValue::Number(_, span) => *span,
            };
            fields.push(SourceEmitField {
                span: field_name.span.join(value_span),
                name: field_name,
                value,
            });
        }
        let end = self
            .expect_symbol('}')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some(SourceEmit {
            signal,
            signal_span,
            fields,
            span: SourceSpan {
                start: emit.span.start,
                end,
            },
        })
    }

    fn parse_source_value(&mut self) -> Option<SourceValue> {
        match self.peek().map(|token| &token.kind) {
            Some(TokenKind::String(_)) => self.expect_string("value").map(SourceValue::String),
            Some(TokenKind::Number(_)) => {
                let token = self.advance().clone();
                if let TokenKind::Number(value) = token.kind {
                    Some(SourceValue::Number(value, token.span))
                } else {
                    None
                }
            }
            Some(TokenKind::Ident(_)) => {
                let binding = self.expect_ident("value path")?;
                let mut segments = Vec::new();
                let mut span = binding.span;
                while self.at_symbol('.') {
                    self.advance();
                    let segment = self.expect_ident("path segment")?;
                    span = span.join(segment.span);
                    segments.push(segment);
                }
                Some(SourceValue::Path {
                    binding,
                    segments,
                    span,
                })
            }
            _ => {
                self.expected("a value (observation path, string, or number)");
                None
            }
        }
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
            // `@key`: mark this field as the class's natural key (import per-row
            // idempotency, spec/std-library/files.md).
            let mut is_key = false;
            if self.at_symbol('@') {
                if let Some(tag) = self.parse_tag() {
                    if tag.name == "key" {
                        is_key = true;
                    } else {
                        self.diagnostics.push(Diagnostic {
                            related: Vec::new(),
                            span: tag.span,
                            message: format!("unknown field tag `@{}`", tag.name),
                            suggestion: Some(
                                "the only field tag is `@key` (the class natural key)".to_owned(),
                            ),
                        });
                    }
                }
            }
            let presence_condition = self.parse_field_presence_condition();
            let span = field_name.span.join(ty.span());
            fields.push(ClassField {
                span,
                name: field_name,
                ty,
                is_key,
                presence_condition,
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

    fn parse_table(
        &mut self,
        tags: Vec<TagDecl>,
        description: Option<StringLiteral>,
    ) -> Option<TableDecl> {
        let start = self.expect_keyword("table")?.span.start;
        let name = self.expect_ident("table name")?;
        self.expect_keyword("as")?;
        let schema = self.expect_ident("table row class")?;
        let open = self.expect_symbol('[')?;
        let mut rows = Vec::new();

        while !self.is_at_end() && !self.at_symbol(']') {
            if self.at_symbol(',') {
                self.advance();
                continue;
            }
            if !self.at_symbol('{') {
                self.unexpected("table row `{ ... }`");
                self.synchronize_to_table_row();
                continue;
            }
            if let Some(row) = self.parse_table_row() {
                rows.push(row);
            }
            if self.at_symbol(',') {
                self.advance();
            }
        }

        let end = self
            .expect_symbol(']')
            .map(|token| token.span.end)
            .unwrap_or(open.span.end);
        Some(TableDecl {
            name,
            tags,
            description,
            schema,
            rows,
            span: SourceSpan { start, end },
        })
    }

    fn parse_table_row(&mut self) -> Option<TableRow> {
        let open = self.expect_symbol('{')?;
        let body_start = open.span.end;
        let mut depth = 1usize;
        let mut body_end = body_start;
        let mut close_end = open.span.end;

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
                        close_end = token.span.end;
                        break;
                    }
                    body_end = token.span.end;
                }
                _ => body_end = token.span.end,
            }
        }

        if depth != 0 {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start: open.span.start,
                    end: body_end,
                },
                message: "unterminated table row".to_owned(),
                suggestion: Some("close the table row with `}`".to_owned()),
            });
            return None;
        }

        let body_span = SourceSpan {
            start: body_start,
            end: body_end,
        };
        let (text, span) = trimmed_source_text(self.source_text(body_span), body_span);
        Some(TableRow {
            body: BlockSource { text, span },
            span: SourceSpan {
                start: open.span.start,
                end: close_end,
            },
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

    fn parse_flow(
        &mut self,
        tags: Vec<TagDecl>,
        description: Option<StringLiteral>,
    ) -> Option<FlowDecl> {
        let start = self.expect_keyword("flow")?.span.start;
        let name = self.expect_ident("flow name")?;
        let mut whens = Vec::new();
        while !self.is_at_end() && !self.at_symbol('{') {
            if self.at_ident("when") {
                let when = self.expect_keyword("when")?;
                if self.at_symbol('{') {
                    whens.extend(self.parse_grouped_when_clauses(when.span)?);
                } else {
                    whens.push(self.parse_when_clause_with_stop(when.span, true)?);
                }
            } else {
                self.unexpected("`when` clause or `{`");
                self.advance();
            }
        }
        let body = self.parse_block_source()?;
        let span = SourceSpan {
            start,
            end: body.span.end,
        };
        Some(FlowDecl {
            name,
            tags,
            description,
            whens,
            body,
            span,
        })
    }

    /// `action <name>(<param: type>, …) { <effect chain> }` (DR-0023). The body
    /// is captured as a block source; expansion at call sites is a later slice.
    fn parse_action(&mut self) -> Option<ActionDecl> {
        let start = self.expect_keyword("action")?.span.start;
        let name = self.expect_ident("action name")?;
        self.expect_symbol('(')?;
        let mut params = Vec::new();
        while !self.is_at_end() && !self.at_symbol(')') {
            let param_name = self.expect_ident("action parameter name")?;
            let ty = self.parse_type()?;
            let span = param_name.span.join(ty.span());
            params.push(ActionParam {
                name: param_name,
                ty,
                span,
            });
            if self.at_symbol(',') {
                self.advance();
            }
        }
        self.expect_symbol(')')?;
        let body = self.parse_block_source()?;
        let span = SourceSpan {
            start,
            end: body.span.end,
        };
        Some(ActionDecl {
            name,
            params,
            body,
            span,
        })
    }

    fn parse_rule(
        &mut self,
        tags: Vec<TagDecl>,
        description: Option<StringLiteral>,
    ) -> Option<RuleDecl> {
        let start = self.expect_keyword("rule")?.span.start;
        let name = self.expect_ident("rule name")?;
        let mut whens = Vec::new();

        while !self.is_at_end() && !self.at_arrow() {
            if self.at_ident("when") {
                whens.extend(self.parse_when_clauses()?);
            } else if self.at_ident("with") {
                let span = self
                    .peek()
                    .map(|token| token.span)
                    .unwrap_or(SourceSpan { start, end: start });
                self.diagnostics.push(Diagnostic {
                    related: Vec::new(),
                    span,
                    message: "`with` is not a rule readiness clause".to_owned(),
                    suggestion: Some("use `when` for rule conditions".to_owned()),
                });
                self.advance();
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
            tags,
            description,
            whens,
            body,
            span,
        })
    }

    fn parse_when_clauses(&mut self) -> Option<Vec<WhenClause>> {
        let when = self.expect_keyword("when")?;
        if self.at_symbol('{') {
            return self.parse_grouped_when_clauses(when.span);
        }

        Some(vec![self.parse_when_clause_after_keyword(when.span)?])
    }

    fn parse_assert(
        &mut self,
        tags: Vec<TagDecl>,
        description: Option<StringLiteral>,
    ) -> Option<AssertDecl> {
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
        Some(AssertDecl {
            tags,
            description,
            expr,
            span,
        })
    }

    fn parse_when_clause_after_keyword(&mut self, when: SourceSpan) -> Option<WhenClause> {
        self.parse_when_clause_with_stop(when, false)
    }

    /// Flow headers terminate at the body `{`; rule headers at `=>`.
    fn parse_when_clause_with_stop(
        &mut self,
        when: SourceSpan,
        stop_at_brace: bool,
    ) -> Option<WhenClause> {
        let text_start = when.end;
        let mut text_end = text_start;

        while !(self.is_at_end()
            || self.at_arrow()
            || self.at_ident("when")
            || self.at_ident("rule")
            || stop_at_brace && self.at_symbol('{'))
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

    fn parse_grouped_when_clauses(&mut self, when: SourceSpan) -> Option<Vec<WhenClause>> {
        let open = self.expect_symbol('{')?;
        let body_start = open.span.end;
        let mut depth = 1usize;
        let mut body_end = body_start;
        let mut close_end = open.span.end;

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
                        close_end = token.span.end;
                        break;
                    }
                    body_end = token.span.end;
                }
                _ => body_end = token.span.end,
            }
        }

        if depth != 0 {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start: when.start,
                    end: body_end,
                },
                message: "unterminated grouped `when` block".to_owned(),
                suggestion: Some("close the grouped readiness block with `}`".to_owned()),
            });
            return Some(Vec::new());
        }

        let body_span = SourceSpan {
            start: body_start,
            end: body_end,
        };
        let mut clauses = Vec::new();
        let mut offset = 0usize;
        for line in self.source_text(body_span).split_inclusive('\n') {
            let line_without_newline = line.trim_end_matches('\n');
            let line_start = body_span.start + offset;
            offset += line.len();
            let leading = line_without_newline.len() - line_without_newline.trim_start().len();
            let trailing = line_without_newline.len() - line_without_newline.trim_end().len();
            let trimmed_start = line_start + leading;
            let trimmed_end = line_start + line_without_newline.len().saturating_sub(trailing);
            if trimmed_start >= trimmed_end {
                continue;
            }
            clauses.push(WhenClause {
                text: self.source[trimmed_start..trimmed_end].to_owned(),
                span: SourceSpan {
                    start: trimmed_start,
                    end: trimmed_end,
                },
            });
        }

        if clauses.is_empty() {
            self.diagnostics.push(Diagnostic {
                related: Vec::new(),
                span: SourceSpan {
                    start: when.start,
                    end: close_end,
                },
                message: "grouped `when` block has no readiness clauses".to_owned(),
                suggestion: Some(
                    "add one condition per line, such as `started` or `Class as binding`"
                        .to_owned(),
                ),
            });
        }

        Some(clauses)
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
            related: Vec::new(),
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

    /// Parse a bracketed list of identifiers, e.g. `[WordCount, OpenPr]`. Used for
    /// the agent `tools` grant, whose entries reference declared workflows by name.
    fn parse_ident_list(&mut self) -> Option<(Vec<Ident>, SourceSpan)> {
        let open = self.expect_symbol('[')?;
        let mut values = Vec::new();

        while !self.is_at_end() && !self.at_symbol(']') {
            values.push(self.expect_ident("tool workflow name")?);
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

    /// Family B: an optional `when <discriminant> is "<literal>"` suffix on a
    /// schema/signal field — the field is present only when the literal-union
    /// discriminant field equals the literal. `is` is used instead of `==` to stay
    /// within the declaration tokenizer; the meaning is equality
    /// (spec/decision-records/discriminated-families-design.md §5.7).
    fn parse_field_presence_condition(&mut self) -> Option<(String, String)> {
        if !self.at_ident("when") {
            return None;
        }
        self.advance(); // `when`
        let disc = self.expect_ident("discriminant field name after `when`")?;
        if self.at_ident("is") {
            self.advance();
        } else {
            self.expected("`is` after the discriminant field");
            return None;
        }
        let literal = self.expect_string("discriminant literal value")?;
        Some((disc.name, literal.value))
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
            // A package name may be a dotted path (`std.messaging`, `std.coord`)
            // or a bare ident (`memory`); a string literal is also accepted.
            TokenKind::Ident(value) => {
                let mut name = value.clone();
                let mut span = token.span;
                self.advance();
                while self.at_symbol('.') {
                    self.expect_symbol('.');
                    let Some(segment) = self.expect_ident("package name segment") else {
                        break;
                    };
                    name.push('.');
                    name.push_str(&segment.name);
                    span = span.join(segment.span);
                }
                Some(StringLiteral { value: name, span })
            }
            TokenKind::String(value) => {
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
                        related: Vec::new(),
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

    fn consume_ident(&mut self, expected: &str) -> bool {
        if self.at_ident(expected) {
            self.advance();
            true
        } else {
            false
        }
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
            related: Vec::new(),
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
            related: Vec::new(),
            span: token.span,
            message: format!("expected {expected}, found {}", token.kind.label()),
            suggestion: suggestion_for_expected(&expected),
        });
    }

    fn synchronize_to_block_item(&mut self) {
        while !self.is_at_end() {
            if self.at_symbol('}')
                || self.at_ident("profile")
                || self.at_ident("provider")
                || self.at_ident("capacity")
                || self.at_ident("skills")
                || self.at_ident("capabilities")
                || self.at_ident("tools")
            {
                return;
            }
            self.advance();
        }
    }

    fn synchronize_to_table_row(&mut self) {
        while !self.is_at_end() {
            if self.at_symbol('{') || self.at_symbol(']') {
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

fn is_gherkin_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "Feature"
            | "Rule"
            | "Background"
            | "Scenario"
            | "ScenarioOutline"
            | "Scenario-Outline"
            | "Examples"
            | "Given"
            | "When"
            | "Then"
            | "And"
            | "But"
    )
}

fn suggestion_for_expected(expected: &str) -> Option<String> {
    match expected {
        "`{`" => Some("add a `{ ... }` block".to_owned()),
        "`=>`" => Some("add `=> { ... }` after the rule conditions".to_owned()),
        "`->`" => Some("add `-> OutputType` before the coerce prompt block".to_owned()),
        "profile string" => Some("write `profile \"profile-name\"`".to_owned()),
        "capacity value" => Some("write `capacity 1`".to_owned()),
        "package library name" => Some("write a package library name, such as `memory`".to_owned()),
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

    const SEND_PROGRAM: &str = r##"
@service
workflow Notify

class Trigger { id string }

agent worker { provider fixture  profile "r"  capacity 1 }

channel alerts { provider slack  destination "#ops" }

table seed as Trigger [ { id "t" } ]

rule notify
  when Trigger as t
=> {
  send via alerts {
    text "hello"
  } as sent
}
"##;

    #[test]
    fn send_lowers_to_messaging_capability_call_and_registers_builtin() {
        // 1929 OPTION A: `send via <channel>` lowers to a `messaging.send`
        // capability.call and registers a built-in `std.messaging` construct +
        // effect contract (so it is lock-exempt).
        let compiled = compile_program(SEND_PROGRAM);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("lowered IR");
        let uses = ir.construct_uses();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].keyword, "send");
        assert_eq!(uses[0].target_capability, "messaging.send");
        let registry = ir.contract_registry();
        let send_construct = registry
            .constructs
            .iter()
            .find(|form| form.keyword == "send")
            .expect("built-in send construct registration");
        assert_eq!(send_construct.library_id, "std.messaging");
        assert_eq!(
            send_construct.target_capability.as_deref(),
            Some("messaging.send")
        );
        assert!(
            registry
                .libraries
                .iter()
                .any(|lib| lib.id == "std.messaging" && lib.standard),
            "std.messaging must be a standard library"
        );
        assert!(
            registry
                .effect_contracts
                .iter()
                .any(|c| c.id == "messaging.send"
                    && c.effect_kind == "capability.call"
                    && c.library_id == "std.messaging"),
            "built-in messaging.send effect contract must be registered"
        );
    }

    #[test]
    fn send_to_unknown_channel_is_rejected() {
        let source = SEND_PROGRAM.replace("send via alerts", "send via ghost");
        let compiled = compile_program(&source);
        let violations: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("unknown channel"))
            .collect();
        assert_eq!(violations.len(), 1, "{:?}", compiled.diagnostics);
        assert!(violations[0].message.contains("ghost"));
    }

    #[test]
    fn derives_contract_registry_from_imports_and_effects() {
        let source = r#"
workflow RegistrySlice

use memory

class Task {
  title string
}

class Review {
  accepted bool
}

coerce reviewTask(title string) -> Review {
  prompt """
  Review {{ title }}
  """
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start
  when Task as task
=> {
  tell worker as turn """
  Work on {{ task.title }}
  """

  after turn succeeds {
    coerce reviewTask(task.title) as review
  }
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        let registry = ir.contract_registry();
        assert_eq!(registry.validate(), Vec::new());

        assert!(registry
            .libraries
            .iter()
            .any(|library| library.id == "memory" && !library.standard));
        assert!(registry
            .libraries
            .iter()
            .any(|library| library.id == "std.agent" && library.standard));
        assert!(registry
            .libraries
            .iter()
            .any(|library| library.id == "std.coerce" && library.standard));

        let coerce = registry
            .effect_contracts
            .iter()
            .find(|contract| contract.id == "coerce")
            .expect("coerce contract");
        assert_eq!(coerce.library_id, "std.coerce");
        assert_eq!(coerce.validation, TypedOutputValidation::RuntimeBoundary);
        assert!(coerce.source_forms.contains(&"coerce".to_owned()));
        assert!(coerce
            .required_capabilities
            .contains(&"model.invoke".to_owned()));

        let agent = registry
            .effect_contracts
            .iter()
            .find(|contract| contract.id == "agent.tell")
            .expect("agent contract");
        assert_eq!(agent.library_id, "std.agent");
        assert_eq!(agent.output_schema.as_deref(), Some("AgentTurn"));
    }

    #[test]
    fn capability_calls_require_the_target_capability() {
        let source = r#"
workflow PackageCall

use memory

class Task {
  title string
}

rule start
  when Task as task
=> {
  call memory.query for task as context
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        let effect = ir.rules[0]
            .metadata
            .effects
            .iter()
            .find(|effect| effect.kind == IrEffectKind::CapabilityCall)
            .expect("capability call effect");
        assert_eq!(
            effect.required_capabilities,
            vec!["memory.query".to_owned()]
        );

        let registry = ir.contract_registry();
        let contract = registry
            .effect_contracts
            .iter()
            .find(|contract| contract.id == "capability.call")
            .expect("capability call contract");
        assert!(contract
            .required_capabilities
            .contains(&"memory.query".to_owned()));
        assert!(!contract
            .required_capabilities
            .contains(&"capability.call".to_owned()));
    }

    #[test]
    fn package_recall_form_lowers_to_capability_call_marker() {
        let source = r#"
workflow PackageRecall

use memory

class Task {
  title string
}

rule start
  when Task as task
=> {
  recall project_memory for task as context
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        let effect = ir.rules[0]
            .metadata
            .effects
            .iter()
            .find(|effect| effect.kind == IrEffectKind::CapabilityCall)
            .expect("capability call effect");
        assert_eq!(effect.binding.as_deref(), Some("context"));
        assert_eq!(
            effect.required_capabilities,
            vec!["memory.query".to_owned()]
        );
        assert_eq!(
            effect.construct_use,
            Some(IrConstructUse {
                keyword: "recall".to_owned(),
                scope: "rule_body".to_owned(),
                construct_family: "effect_operation".to_owned(),
                lowering_target: "capability_call".to_owned(),
                target_capability: "memory.query".to_owned(),
            })
        );
        assert_eq!(ir.construct_uses().len(), 1);
        assert!(ir.to_snapshot().contains("construct=recall->memory.query"));
    }

    #[test]
    fn parses_schema_agent_and_rule_slice() {
        let source = r#"
workflow QueueWorkerSlice

use memory

queue backlog {
  tracker builtin
}

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
  provider fixture
  profile "repo-writer"
  capacity 1
  skills ["loft-user"]
}

rule start_ready_item
  when backlog has ready item as item
  when worker is available
=> {
  claim item as claim

  after claim succeeds {
    tell worker """
    Implement {{ item.title }}
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
        assert_eq!(workflow, Some("QueueWorkerSlice"));
        assert_eq!(parsed.program.items.len(), 7);

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
        assert_eq!(rule.whens[0].text, "backlog has ready item as item");
        assert!(rule.body.text.contains("after claim succeeds"));
    }

    #[test]
    fn parses_and_lowers_static_table_rows() {
        let source = r#"
workflow TableSeed

agent codex {
  provider codex
  profile "repo-writer"
  capacity 1
}

class Task {
  provider AgentRef<codex>
  title string
  priority int
  status "queued"
}

table tasks as Task [
  {
    provider codex
    title "Review parser"
    priority 1
    status "queued"
  }

  {
    provider codex
    title "Review runtime"
    priority 2
    status "queued"
  }
]
"#;

        let parsed = parse_program(source);
        assert_eq!(parsed.diagnostics, Vec::new());
        let table = parsed
            .program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Table(table) => Some(table),
                _ => None,
            })
            .expect("table item");
        assert_eq!(table.rows.len(), 2);
        let row_spans = table.rows.iter().map(|row| row.span).collect::<Vec<_>>();

        let compiled = compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let table_rule = ir
            .rules
            .iter()
            .find(|rule| rule.name == "table_tasks")
            .expect("table lowers to generated started rule");
        assert_eq!(table_rule.whens[0].pattern, "started");
        assert!(table_rule.body.contains("record Task"));
        assert_eq!(table_rule.metadata.fact_writes, vec!["schema:Task"]);
        assert_eq!(table_rule.metadata.record_sources.len(), 2);
        assert_eq!(
            table_rule
                .metadata
                .record_sources
                .iter()
                .map(|source| (
                    source.schema.as_str(),
                    source.construct.as_str(),
                    source.span
                ))
                .collect::<Vec<_>>(),
            row_spans
                .iter()
                .map(|span| ("Task", "table_row", *span))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_old_matrix_declarations() {
        let source = r#"
workflow MatrixSeed

class Task {
  title string
  status "queued"
}

matrix tasks as Task [
  {
    title "Review parser"
    status "queued"
  }
]
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("expected top-level declaration, found identifier `matrix`")
        }));
    }

    #[test]
    fn rejects_table_rows_that_violate_row_schema() {
        let source = r#"
workflow BadTable

agent codex {
  provider codex
  profile "repo-writer"
  capacity 1
}

class Task {
  provider AgentRef<codex>
  status "queued"
}

table tasks as Task [
  {
    provider "codex"
    status "done"
  }
]
"#;

        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("expects an AgentRef value, not string `codex`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("expects literal string `queued`")));
    }

    #[test]
    fn parses_formats_and_lowers_source_tags_as_metadata() {
        let source = r#"
@fixture
@release-gate
workflow Tagged

class Task {
  status "queued"
}

@seed
table tasks as Task [
  {
    status "queued"
  }
]

@acceptance
assert count(Task where status == "queued") == 1

@dispatch
rule consume_task
  when Task as task
=> {
  done task
}
"#;

        let parsed = parse_program(source);
        assert_eq!(parsed.diagnostics, Vec::new());
        assert_eq!(
            parsed
                .program
                .workflow_tags
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fixture", "release-gate"]
        );

        let formatted = format_program(source).formatted.expect("formats");
        assert!(formatted.contains("@fixture\n@release-gate\nworkflow Tagged"));
        assert!(formatted.contains("@seed\ntable tasks as Task"));
        assert!(formatted.contains("@acceptance\nassert count"));
        assert!(formatted.contains("@dispatch\nrule consume_task"));

        let compiled = compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let tags = ir
            .source_tags
            .iter()
            .map(|tag| {
                (
                    tag.name.as_str(),
                    tag.target_kind.as_str(),
                    tag.target.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert!(tags.contains(&("fixture", "workflow", "Tagged")));
        assert!(tags.contains(&("release-gate", "workflow", "Tagged")));
        assert!(tags.contains(&("seed", "table", "tasks")));
        assert!(tags.contains(&("dispatch", "rule", "consume_task")));
        assert!(ir
            .source_tags
            .iter()
            .any(|tag| tag.name == "acceptance" && tag.target_kind == "assertion"));
    }

    #[test]
    fn parses_formats_and_lowers_source_descriptions_as_metadata() {
        let source = r#"
@fixture
description "Fixture-backed acceptance workflow"
workflow Described

class Task {
  status "queued"
}

description "Static task seed rows"
table tasks as Task [
  {
    status "queued"
  }
]

description "All seed tasks were consumed"
assert count(Task where status == "queued") == 0

description "Consume one queued task"
rule consume_task
  when Task as task
=> {
  done task
}
"#;

        let parsed = parse_program(source);
        assert_eq!(parsed.diagnostics, Vec::new());
        assert_eq!(
            parsed
                .program
                .workflow_description
                .as_ref()
                .map(|description| description.value.as_str()),
            Some("Fixture-backed acceptance workflow")
        );

        let formatted = format_program(source).formatted.expect("formats");
        assert!(formatted.contains(
            "@fixture\ndescription \"Fixture-backed acceptance workflow\"\nworkflow Described"
        ));
        assert!(formatted.contains("description \"Static task seed rows\"\ntable tasks as Task"));
        assert!(formatted.contains("description \"All seed tasks were consumed\"\nassert count"));
        assert!(formatted.contains("description \"Consume one queued task\"\nrule consume_task"));

        let compiled = compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let descriptions = ir
            .source_descriptions
            .iter()
            .map(|description| {
                (
                    description.value.as_str(),
                    description.target_kind.as_str(),
                    description.target.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert!(descriptions.contains(&(
            "Fixture-backed acceptance workflow",
            "workflow",
            "Described"
        )));
        assert!(descriptions.contains(&("Static task seed rows", "table", "tasks")));
        assert!(descriptions.contains(&("Consume one queued task", "rule", "consume_task")));
        assert!(ir
            .source_descriptions
            .iter()
            .any(
                |description| description.value == "All seed tasks were consumed"
                    && description.target_kind == "assertion"
            ));
    }

    #[test]
    fn rejects_descriptions_on_unsupported_declarations_for_now() {
        let source = r#"
workflow BadDescriptions

description "Task schema"
class Task {
  status "queued"
}
"#;

        let parsed = parse_program(source);

        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(
            parsed.diagnostics[0].message,
            "description cannot be attached to class"
        );
    }

    #[test]
    fn rejects_tags_on_unsupported_declarations_for_now() {
        let source = r#"
workflow BadTags

@schema
class Task {
  status "queued"
}
"#;

        let parsed = parse_program(source);

        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(
            parsed.diagnostics[0].message,
            "tag `@schema` cannot be attached to class"
        );
    }

    #[test]
    fn use_short_form_imports_package_libraries_and_rejects_removed_kinds() {
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

        let removed_plugin = parse_program("workflow Imports\n\nuse plugin \"memory\"\n");
        assert_eq!(removed_plugin.diagnostics.len(), 1);
        assert_eq!(
            removed_plugin.diagnostics[0].message,
            "`use plugin` is no longer supported"
        );

        let removed_skill = parse_program("workflow Imports\n\nuse skill \"loft-user\"\n");
        assert_eq!(removed_skill.diagnostics.len(), 1);
        assert_eq!(
            removed_skill.diagnostics[0].message,
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
    fn revision_fixture_bundles_compile_with_expected_contract_shapes() {
        let compatible_v1 =
            compile_program(include_str!("../fixtures/revision-compatible-v1.whip"));
        let compatible_v2 =
            compile_program(include_str!("../fixtures/revision-compatible-v2.whip"));
        let incompatible_v2 =
            compile_program(include_str!("../fixtures/revision-incompatible-v2.whip"));
        for compiled in [&compatible_v1, &compatible_v2, &incompatible_v2] {
            assert_eq!(compiled.diagnostics, Vec::new());
        }
        let compatible_v1 = compatible_v1.ir.expect("compatible v1 compiles");
        let compatible_v2 = compatible_v2.ir.expect("compatible v2 compiles");
        let incompatible_v2 = incompatible_v2.ir.expect("incompatible v2 compiles");

        assert_eq!(compatible_v1.workflow, "RevisionFixture");
        assert_eq!(compatible_v2.workflow, "RevisionFixture");
        assert_eq!(incompatible_v2.workflow, "RevisionFixture");
        assert_eq!(
            compatible_v1
                .workflow_contracts
                .iter()
                .map(|contract| (&contract.kind, contract.name.as_str(), &contract.ty))
                .collect::<Vec<_>>(),
            compatible_v2
                .workflow_contracts
                .iter()
                .map(|contract| (&contract.kind, contract.name.as_str(), &contract.ty))
                .collect::<Vec<_>>()
        );
        assert_ne!(
            compatible_v1
                .workflow_contracts
                .iter()
                .map(|contract| (&contract.kind, contract.name.as_str(), &contract.ty))
                .collect::<Vec<_>>(),
            incompatible_v2
                .workflow_contracts
                .iter()
                .map(|contract| (&contract.kind, contract.name.as_str(), &contract.ty))
                .collect::<Vec<_>>()
        );
        assert!(compatible_v2
            .schemas
            .iter()
            .any(|schema| matches!(schema, IrSchema::Class(class) if class.name == "AuditTrail")));
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
            .expect("dispatch rule lowers");
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
    fn rejects_with_as_rule_readiness_alias() {
        let source = r#"
workflow WithIsNotWhen

rule bad
  with started
=> {
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("`with` is not a rule readiness clause")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .suggestion
            .as_deref()
            .is_some_and(|suggestion| suggestion.contains("use `when` for rule conditions"))));
    }

    #[test]
    fn parses_grouped_when_clauses_as_ordinary_readiness_clauses() {
        let source = r#"
workflow GroupedWhen

class Task {
  status "queued"
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start
  when {
    Task as task where task.status == "queued"
    worker is available
  }
=> {
  tell worker "do it"
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.expect("program compiles");
        let rule = &ir.rules[0];

        assert_eq!(rule.whens.len(), 2);
        assert_eq!(rule.whens[0].pattern, "Task as task");
        assert_eq!(
            rule.whens[0]
                .guard
                .as_ref()
                .map(|guard| guard.expr.to_snapshot()),
            Some("task.status == \"queued\"".to_owned())
        );
        assert_eq!(rule.whens[1].pattern, "worker is available");
        assert!(ir
            .to_snapshot()
            .contains("    when Task as task where task.status == \"queued\""));
        assert!(ir.to_snapshot().contains("    when worker is available"));
    }

    #[test]
    fn accepts_harness_declarations_and_agent_bindings() {
        let source = r#"
workflow HarnessTopology

harness coder: codex
harness reviewer: claude

agent implementer using coder {
  profile "repo-writer"
  capacity 1
}

agent critic using reviewer {
  profile "repo-reader"
  capacity 1
}

rule start
  when started
=> {
  tell implementer as turn "implement"
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        assert_eq!(ir.harnesses.len(), 2);
        assert_eq!(ir.harnesses[0].name, "coder");
        assert_eq!(ir.harnesses[0].kind, "codex");
        assert_eq!(
            ir.agents
                .iter()
                .find(|agent| agent.name == "implementer")
                .and_then(|agent| agent.harness.as_deref()),
            Some("coder")
        );
        let snapshot = ir.to_snapshot();
        assert!(snapshot.contains("harness coder kind=codex"));
        assert!(snapshot.contains("agent implementer harness=coder"));
    }

    #[test]
    fn rejects_agent_binding_to_unknown_harness() {
        let source = r#"
workflow UnknownHarness

agent worker using missing {
  profile "repo-writer"
  capacity 1
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("agent `worker` uses unknown harness `missing`")));
    }

    #[test]
    fn rejects_duplicate_and_unsupported_harness_declarations() {
        let source = r#"
workflow BadHarnesses

harness coder: spaceship
harness coder: codex

agent worker using coder {
  profile "repo-writer"
  capacity 1
}
"#;

        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        let messages = compiled
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("harness `coder` is declared more than once")),
            "{messages:#?}"
        );
        assert!(
            messages.iter().any(|message| {
                message.contains("harness `coder` uses unsupported kind `spaceship`")
            }),
            "{messages:#?}"
        );
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
    fn accepts_workflow_terminal_actions_in_header_style_workflows() {
        let source = r#"
workflow ImplicitTerminal

output result Result

class Result {
  status "ok"
}

rule finish
  when started
=> {
  complete result {
    status "ok"
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("header-style terminals compile");
        assert_eq!(ir.workflow_contracts.len(), 1);
    }

    #[test]
    fn rejects_header_style_terminal_for_undeclared_contract() {
        let source = r#"
workflow ImplicitTerminal

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
            .contains("completes unknown workflow terminal `result`")));
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
  provider fixture
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
    fn lowers_and_formats_agent_tools_grant() {
        // DR-0025: an agent may declare a `tools [...]` grant of workflows it can
        // invoke as typed tools. The grant lowers to `IrAgent.tools`, survives a
        // format round-trip, and de-duplicates entries with a diagnostic.
        let source = r#"
workflow GrantHost

agent worker {
  provider owned
  profile "repo-writer"
  capacity 1
  tools [WordCount, OpenPr]
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("valid ir");
        let agent = ir
            .agents
            .iter()
            .find(|agent| agent.name == "worker")
            .expect("worker agent");
        assert_eq!(
            agent.tools,
            vec!["WordCount".to_owned(), "OpenPr".to_owned()]
        );

        let formatted = format_program(source).formatted.expect("formats");
        assert!(
            formatted.contains("tools [WordCount, OpenPr]"),
            "formatted: {formatted}"
        );

        // A duplicate grant entry is rejected.
        let dup = compile_program(
            "workflow Dup\nagent a {\n  provider owned\n  profile \"p\"\n  tools [X, X]\n}\n",
        );
        assert!(
            dup.diagnostics
                .iter()
                .any(|d| d.message.contains("grants tool `X` more than once")),
            "diagnostics: {:?}",
            dup.diagnostics
        );
    }

    #[test]
    fn accepts_agent_ref_dynamic_tell_targets() {
        let source = r#"
workflow AgentRefRouting

agent codex {
  provider codex
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

agent claude {
  provider claude
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
  provider codex
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell", "repo.write"]
}

agent claude {
  provider claude
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
  provider codex
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
  provider codex
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
  provider codex
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
                "count(task.labels) == 0 || exists(Result where status == \"done\")",
                "(count(task.labels) == 0) || exists(Result where status == \"done\")",
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
  route "pi" | "coerce"
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

class Result {
  title string
  files string[]
}

agent worker {
  provider fixture
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
  class Result
    title string
    files array<string>
agents
  agent worker harness=<fallback> provider=fixture profile=repo-writer capacity=2 skills=[loft-user] capabilities=[] tools=[]
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
                include_str!("../../../examples/queue-worker-with-review.whip"),
                include_str!("../../../examples/queue-worker-with-review.ir"),
            ),
            (
                include_str!("../../../examples/circuit-breaker.whip"),
                include_str!("../../../examples/circuit-breaker.ir"),
            ),
            (
                include_str!("../../../examples/coerce-branch.whip"),
                include_str!("../../../examples/coerce-branch.ir"),
            ),
            (
                include_str!("../../../examples/terminal-output-union.whip"),
                include_str!("../../../examples/terminal-output-union.ir"),
            ),
            (
                include_str!("../../../examples/triage-flow.whip"),
                include_str!("../../../examples/triage-flow.ir"),
            ),
            (
                include_str!("../../../examples/incident-router.whip"),
                include_str!("../../../examples/incident-router.ir"),
            ),
            (
                include_str!("../../../examples/human-review.whip"),
                include_str!("../../../examples/human-review.ir"),
            ),
            (
                include_str!("../../../examples/multi-agent-bounded-concurrency.whip"),
                include_str!("../../../examples/multi-agent-bounded-concurrency.ir"),
            ),
            // `openclaw-lite` now imports the external `memory` package, so it
            // only compiles clean with a `whip.lock`; this parser-level snapshot
            // has no package resolution. Its IR stability is covered by the
            // lock-aware `dev_openclaw_lite_observes_heartbeat_and_files_work`.
            (
                include_str!("../../../examples/scheduled-escalation.whip"),
                include_str!("../../../examples/scheduled-escalation.ir"),
            ),
            (
                include_str!("../../../examples/event-bridge.whip"),
                include_str!("../../../examples/event-bridge.ir"),
            ),
            (
                include_str!("../../../examples/reusable-review-pattern.whip"),
                include_str!("../../../examples/reusable-review-pattern.ir"),
            ),
            (
                include_str!("../../../examples/reusable-action-chain.whip"),
                include_str!("../../../examples/reusable-action-chain.ir"),
            ),
            (
                include_str!("../../../examples/exec-json-ingest.whip"),
                include_str!("../../../examples/exec-json-ingest.ir"),
            ),
            (
                include_str!("../../../examples/deterministic-validation.whip"),
                include_str!("../../../examples/deterministic-validation.ir"),
            ),
            (
                include_str!("../../../examples/autoresearch-lite.whip"),
                include_str!("../../../examples/autoresearch-lite.ir"),
            ),
            (
                include_str!("../../../examples/gastown-lite.whip"),
                include_str!("../../../examples/gastown-lite.ir"),
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
    fn revision_examples_compile() {
        let examples = [
            (
                include_str!("../../../examples/revision-ticket-v1.whip"),
                Some("RevisionTicket"),
            ),
            (
                include_str!("../../../examples/revision-ticket-v2.whip"),
                Some("RevisionTicket"),
            ),
            (
                include_str!("../../../examples/revision-repair-planner.whip"),
                Some("RevisionRepairPlanner"),
            ),
            (
                include_str!("../../../examples/revision-running-cancel.whip"),
                Some("RevisionRunningCancel"),
            ),
            (
                include_str!("../../../examples/revision-parent-child.whip"),
                Some("ParentRevisionExample"),
            ),
            (
                include_str!("../../../examples/revision-validation-approval.whip"),
                Some("RevisionValidation"),
            ),
        ];

        for (source, root) in examples {
            let compiled = compile_program_with_root(source, root);
            assert_eq!(compiled.diagnostics, Vec::new());
            assert!(compiled.ir.is_some());
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

    fn read_codec_program(format: &str) -> String {
        format!(
            r#"
workflow ReadBody

output result Result

class Result {{
  status string
}}

file store project_files {{
  root "./data"
}}

rule pick
  when started
=> {{
  read {format} from project_files at "note.md" as fileResult
  after fileResult succeeds as result {{
    complete result {{
      status "ok"
    }}
  }}
}}
"#
        )
    }

    #[test]
    fn read_accepts_text_and_markdown_body_codecs() {
        for format in ["text", "markdown"] {
            let compiled = compile_program(&read_codec_program(format));
            assert_eq!(
                compiled.diagnostics,
                Vec::new(),
                "`read {format}` compiles clean"
            );
            assert!(compiled.ir.is_some(), "`read {format}` produces IR");
        }
    }

    #[test]
    fn read_rejects_structured_and_binary_codecs() {
        // Structured codecs are the `import` surface; `bytes` is a deferred read
        // codec. `read` decodes only body formats in v0.
        for format in ["json", "jsonl", "csv", "bytes"] {
            let compiled = compile_program(&read_codec_program(format));
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("not supported")),
                "`read {format}` is rejected with a diagnostic; got {:?}",
                compiled.diagnostics
            );
        }
    }

    fn write_program(format: &str, mode_clause: &str) -> String {
        format!(
            r#"
workflow WriteBody

output result Result

class Result {{
  status string
}}

file store out_files {{
  root "./data"
}}

rule pick
  when started
=> {{
  write {format} to out_files at "report.md" {{
    body "hello"
    {mode_clause}
  }} as written
  after written succeeds as result {{
    complete result {{
      status "ok"
    }}
  }}
}}
"#
        )
    }

    #[test]
    fn write_accepts_text_and_markdown_with_explicit_mode() {
        for format in ["text", "markdown"] {
            let compiled = compile_program(&write_program(format, "mode create"));
            assert_eq!(
                compiled.diagnostics,
                Vec::new(),
                "`write {format}` with an explicit mode compiles clean"
            );
            assert!(compiled.ir.is_some(), "`write {format}` produces IR");
        }
    }

    #[test]
    fn write_rejects_structured_codecs() {
        for format in ["json", "csv", "bytes"] {
            let compiled = compile_program(&write_program(format, "mode create"));
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("not supported")),
                "`write {format}` is rejected; got {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn write_requires_an_explicit_mode() {
        // "No silent overwrite": omitting the mode is a check error.
        let compiled = compile_program(&write_program("text", ""));
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("explicit `mode`")),
            "`write` without a mode is rejected; got {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn write_rejects_unknown_mode() {
        let compiled = compile_program(&write_program("text", "mode clobber"));
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unknown write mode")),
            "an unknown write mode is rejected; got {:?}",
            compiled.diagnostics
        );
    }

    fn import_program(format: &str) -> String {
        format!(
            r#"
workflow ImportRows

output result Result

class Result {{
  status string
}}

class IssueRow {{
  title string
  priority string
}}

file store data_files {{
  root "./data"
}}

rule pick
  when started
=> {{
  import {format} IssueRow from data_files at "issues.in" as imported
  after imported succeeds as r {{
    complete result {{
      status "ok"
    }}
  }}
}}
"#
        )
    }

    #[test]
    fn import_accepts_structured_codecs_and_lowers_to_file_import() {
        for format in ["jsonl", "json", "csv"] {
            let compiled = compile_program(&import_program(format));
            assert_eq!(
                compiled.diagnostics,
                Vec::new(),
                "`import {format}` compiles clean"
            );
            let ir = compiled.ir.expect("import produces IR");
            let rule = ir.rules.first().expect("rule");
            assert!(
                rule.metadata
                    .effects
                    .iter()
                    .any(|effect| effect.kind == IrEffectKind::FileImport),
                "`import {format}` lowers to a file.import effect"
            );
        }
    }

    #[test]
    fn import_rejects_unsupported_codecs() {
        // `import` decodes structured row codecs only; body/binary formats are
        // not import surfaces.
        for format in ["xml", "text", "markdown", "bytes"] {
            let compiled = compile_program(&import_program(format));
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("not supported")),
                "`import {format}` is rejected; got {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn class_field_key_annotation_lowers_and_rejects_duplicates() {
        let single = compile_program(
            r#"
workflow Keyed

class Row {
  id string @key
  title string
}
"#,
        );
        assert_eq!(
            single.diagnostics,
            Vec::new(),
            "single `@key` compiles clean"
        );
        let ir = single.ir.expect("ir");
        let class = ir
            .schemas
            .iter()
            .find_map(|schema| match schema {
                IrSchema::Class(class) if class.name == "Row" => Some(class),
                _ => None,
            })
            .expect("Row class");
        let key_fields = class
            .fields
            .iter()
            .filter(|field| field.is_key)
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(key_fields, vec!["id"], "the `@key` field is recorded");

        let dual = compile_program(
            r#"
workflow Keyed

class Row {
  a string @key
  b string @key
}
"#,
        );
        assert!(
            dual.diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("more than one `@key`")),
            "two `@key` fields are rejected; got {:?}",
            dual.diagnostics
        );
    }

    #[test]
    fn single_line_terminal_block_validates_its_fields() {
        // Regression: `complete <name> { <field> }` on one line was reported as
        // missing the field, because the terminal-block extractor only captured
        // subsequent lines (a single-line block has brace-delta 0). Both the
        // single-line and multi-line forms must validate identically.
        for body in [
            "  complete result { status \"ok\" }",
            "  complete result {\n    status \"ok\"\n  }",
        ] {
            let source = format!(
                r#"
workflow S

output result Result

class Result {{
  status string
}}

rule go
  when started
=> {{
{body}
}}
"#
            );
            let compiled = compile_program(&source);
            assert!(
                !compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("missing required field")),
                "terminal block validates its field; got {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn action_declaration_parses_and_is_inert_until_expansion() {
        // DR-0023 slice 1: an `action` template parses (typed params + a block
        // body) and lowers away cleanly (inert until call-site expansion in
        // slice 2), so a program declaring an unused action compiles with no
        // diagnostics.
        let compiled = compile_program(
            r#"
workflow A

output result Result

class Result {
  status string
}

class Task {
  name string
}

action do_it(task Task, label string) {
  record Result {
    status label
  }
}

rule go
  when started
=> {
  complete result {
    status "ok"
  }
}
"#,
        );
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "an unused action declaration compiles clean"
        );
        let ir = compiled.ir.expect("program with an action lowers");
        // The action is a template consumed before lowering — it is not a runtime
        // construct, so it leaves no rule/schema behind beyond the workflow's own.
        assert!(
            ir.rules.iter().any(|rule| rule.name == "go"),
            "the ordinary rule still lowers alongside the action template"
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
      Completed as result => {
        record Routed {
          branch "completed"
          detail result.summary
        }
      }
      Failed as failure => {
        record Routed {
          branch "failed"
          detail failure.reason
        }
      }
      TimedOut as timeout => {
        record Routed {
          branch "timed_out"
          detail timeout.summary
        }
      }
      Cancelled as cancel => {
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
    fn accepts_terminal_output_case_as_binding_form() {
        // `Completed as result` is accepted alongside the space form `Completed result`
        // and binds/narrows identically (Stage 1b surface unification).
        let source = r#"
workflow T

class WorkItem { title string }
class MessageClassification { summary string }
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
      Completed as result => {
        record Routed { branch "completed" detail result.summary }
      }
      Failed as failure => {
        record Routed { branch "failed" detail failure.reason }
      }
      TimedOut as timeout => {
        record Routed { branch "timed_out" detail timeout.summary }
      }
      Cancelled as cancel => {
        record Routed { branch "cancelled" detail cancel.summary }
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
    fn accepts_after_times_out_branch_and_types_payload_alias() {
        let source = r#"
workflow TimedOutBranch

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

  after classification times out as t {
    record Routed {
      branch "timed_out"
      detail t.summary
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn accepts_after_cancelled_branch_and_types_payload_alias() {
        let source = r#"
workflow CancelledBranch

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

  after classification cancelled as c {
    record Routed {
      branch "cancelled"
      detail c.summary
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn rejects_invalid_after_predicate_during_compilation() {
        let source = r#"
workflow BadPredicate

class WorkItem {
  title string
}

class MessageClassification {
  summary string
}

class Routed {
  branch string
}

coerce classifyMessage(title string) -> MessageClassification {
  prompt "Classify"
}

rule classify
  when WorkItem as item
=> {
  coerce classifyMessage(item.title) as classification

  after classification explodes {
    record Routed {
      branch "boom"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.diagnostics.iter().any(|d| d
            .message
            .contains("unsupported `after` predicate `explodes`")));
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
      Completed as result => {
        record TerminalRoute {
          detail result.reason
        }
      }
      Failed as failure => {
        record TerminalRoute {
          detail failure.reason
        }
      }
      TimedOut as timeout => {
        record TerminalRoute {
          detail timeout.summary
        }
      }
      Cancelled as cancel => {
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
      Success as result => {
      }
      Completed as result => {
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

    /// Source for terminal-output case tests: a coerce whose `Completed` payload
    /// is `MessageClassification`, matched in an `after ... completes` case. The
    /// `{cases}` placeholder is filled per test.
    fn terminal_case_program(cases: &str) -> String {
        format!(
            r#"
workflow TerminalCaseMatrix

class WorkItem {{
  title string
}}

class MessageClassification {{
  summary string
}}

class Routed {{
  branch string
}}

coerce classifyMessage(title string) -> MessageClassification {{
  prompt "Classify"
}}

rule classify
  when WorkItem as item
=> {{
  coerce classifyMessage(item.title) as classification

  after classification completes {{
    case classification {{
{cases}
    }}
  }}
}}
"#
        )
    }

    #[test]
    fn accepts_guarded_terminal_case_branch_referencing_refined_payload() {
        // Regression: a `where` guard on a tagged terminal branch must be able to
        // read the tag-refined payload binding (`result.summary`). It was wrongly
        // rejected as an unknown root because `validate_case_blocks` could not
        // bind the terminal payload into the guard scope.
        let source = terminal_case_program(
            "      Completed as result where result.summary == \"ok\" => { record Routed { branch \"ok\" } }\n      _ => { record Routed { branch \"other\" } }",
        );
        let compiled = compile_program(&source);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn rejects_terminal_case_guard_referencing_unknown_payload_field() {
        let source = terminal_case_program(
            "      Completed as result where result.nonexistent == \"ok\" => { record Routed { branch \"ok\" } }\n      _ => { record Routed { branch \"other\" } }",
        );
        let compiled = compile_program(&source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("schema `MessageClassification` has no field `nonexistent`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_non_boolean_terminal_case_guard() {
        let source = terminal_case_program(
            "      Completed as result where result.summary => { record Routed { branch \"ok\" } }\n      _ => { record Routed { branch \"other\" } }",
        );
        let compiled = compile_program(&source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("non-boolean case guard expression")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_duplicate_terminal_output_case_tag() {
        let source = terminal_case_program(
            "      Completed as result => { record Routed { branch \"a\" } }\n      Completed as other => { record Routed { branch \"b\" } }\n      Failed as failure => { record Routed { branch \"f\" } }\n      TimedOut as timeout => { record Routed { branch \"t\" } }\n      Cancelled as cancel => { record Routed { branch \"c\" } }",
        );
        let compiled = compile_program(&source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("duplicate unguarded terminal-output case pattern `Completed`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_terminal_output_case_branch_without_payload_binding() {
        let source = terminal_case_program(
            "      Completed => { record Routed { branch \"a\" } }\n      Failed as failure => { record Routed { branch \"f\" } }\n      TimedOut as timeout => { record Routed { branch \"t\" } }\n      Cancelled as cancel => { record Routed { branch \"c\" } }",
        );
        let compiled = compile_program(&source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("malformed terminal-output case pattern `Completed`")),
            "{:?}",
            compiled.diagnostics
        );
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
    fn rejects_unreachable_case_branch_after_wildcard() {
        // A branch placed after an unguarded `_` can never match (case-family.maude
        // inv c, redundant-postwild).
        let source = r#"
workflow CaseUnreachableGuess

enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class Review {
  status ReviewStatus
}

rule route
  when Review as review
=> {
  case review.status {
    Accept => {
    }
    _ => {
    }
    Revise => {
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("unreachable case branch after the `_` wildcard")),
            "expected unreachable-after-wildcard diagnostic: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn family_b_presence_condition_validates_discriminant() {
        let program = |fields: &str| {
            format!(
                r#"
workflow B
input e Event
output result Done
class Done {{ ok bool }}
class Event {{
{fields}
}}
rule r
  when Event as e
=> {{
  complete result {{ ok true }}
}}
"#
            )
        };
        // Valid: a literal-union discriminant with an in-range `when` literal.
        let ok = compile_program(&program(
            "  kind \"deploy\" | \"rollback\"\n  region string when kind is \"deploy\"",
        ));
        assert_eq!(ok.diagnostics, Vec::new());
        assert!(ok.ir.is_some());
        // Unknown discriminant.
        let bad1 = compile_program(&program(
            "  kind \"deploy\" | \"rollback\"\n  region string when missing is \"deploy\"",
        ));
        assert!(bad1
            .diagnostics
            .iter()
            .any(|d| d.message.contains("unknown discriminant `missing`")));
        // Literal not in the discriminant union.
        let bad2 = compile_program(&program(
            "  kind \"deploy\" | \"rollback\"\n  region string when kind is \"ship\"",
        ));
        assert!(bad2
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a value of `kind`")));
        // Discriminant is not a string-literal union.
        let bad3 = compile_program(&program(
            "  kind string\n  region string when kind is \"deploy\"",
        ));
        assert!(bad3
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a string-literal discriminant")));
    }

    #[test]
    fn case_arm_effect_records_its_selector() {
        // An effect inside a `case <scrutinee> { <pattern> => … }` arm records the
        // selector `(scrutinee, pattern)` so the IFC checker can apply
        // NMIF-on-the-selector to a crossing (DR §7.4).
        let source = r#"
workflow S

input item WorkItem
output result R
class WorkItem { kind "a" | "b" }
class R { ok bool }
class V { ok bool }

coerce f(t string) -> V { prompt "x" }

rule r
  when WorkItem as item
=> {
  case item.kind {
    "a" => {
      coerce f("hi") as v
      after v succeeds {
        complete result { ok v.ok }
      }
    }
    "b" => {
      complete result { ok false }
    }
  }
}
"#;
        let ir = compile_program(source).ir.expect("compiles");
        let rule = ir.rules.iter().find(|r| r.name == "r").expect("rule r");
        let coerce = rule
            .metadata
            .effects
            .iter()
            .find(|e| e.binding.as_deref() == Some("v"))
            .expect("coerce effect v");
        let (scrutinee, pattern) = coerce
            .selected_by
            .as_ref()
            .expect("coerce in a case arm records its selector");
        assert_eq!(scrutinee, "item.kind");
        assert_eq!(pattern, "\"a\"");
        // An effect outside any case (the `complete` is in an arm, but there are no
        // top-level effects here) — sanity: a fresh top-level coerce has no selector.
        // (Covered by every other example whose effects are top-level: selected_by None.)
    }

    #[test]
    fn family_b_read_narrowing_restricts_conditioned_reads() {
        let program = |body: &str| {
            format!(
                r#"
workflow B
input e Event
output result Done
class Done {{ region string }}
class Event {{
  kind "deploy" | "rollback"
  region string when kind is "deploy"
}}
rule r
  when Event as e
=> {{
{body}
}}
"#
            )
        };
        // Outside any case arm: a conditioned read is rejected.
        let outside = compile_program(&program("  complete result { region e.region }"));
        assert!(
            outside
                .diagnostics
                .iter()
                .any(|d| d.message.contains("conditional field `e.region`")),
            "{:?}",
            outside.diagnostics
        );
        // Inside the matching `deploy` arm: allowed.
        let matching = compile_program(&program(
            "  case e.kind {\n    \"deploy\" => { complete result { region e.region } }\n    \"rollback\" => { complete result { region \"none\" } }\n  }",
        ));
        assert_eq!(matching.diagnostics, Vec::new());
        assert!(matching.ir.is_some());
        // Inside the wrong (`rollback`) arm: rejected (region is a deploy-only field).
        let wrong = compile_program(&program(
            "  case e.kind {\n    \"deploy\" => { complete result { region \"x\" } }\n    \"rollback\" => { complete result { region e.region } }\n  }",
        ));
        assert!(
            wrong
                .diagnostics
                .iter()
                .any(|d| d.message.contains("conditional field `e.region`")),
            "{:?}",
            wrong.diagnostics
        );
    }

    #[test]
    fn rejects_conflicting_reused_effect_binding() {
        // Reusing an effect binding for two effects with DIFFERENT result types makes
        // `after <binding> …` ambiguous (§5.5). Same-type reuse is harmless and allowed.
        let source = r#"
workflow D

output result R
class R { x string }
class WorkItem { title string }
class A { a string }
class B { b string }

coerce fa(t string) -> A { prompt "x" }
coerce fb(t string) -> B { prompt "x" }

rule r
  when WorkItem as item
=> {
  coerce fa(item.title) as v
  coerce fb(item.title) as v
  complete result { x "done" }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("reuses effect binding `v`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_transitive_workflow_invocation_cycle() {
        // A invokes B invokes A — a runtime invoke cycle with no compile-time
        // convergence proof (RESOLVED 2026-07-01). Rejected before root selection.
        let source = r#"
workflow A {
  input task TA
  output result RA
  class TA { id string }
  class RA { id string }
  rule go
    when TA as t
  => {
    invoke B { task { id t.id } } as b
    after b succeeds as r { complete result { id r.id } }
  }
}

workflow B {
  input task TB
  output result RB
  class TB { id string }
  class RB { id string }
  rule go
    when TB as t
  => {
    invoke A { task { id t.id } } as a
    after a succeeds as r { complete result { id r.id } }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("A"));
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("graph.unbounded_workflow_invocation_recursion")
                && d.message.contains("A -> B -> A")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn accepts_acyclic_workflow_invocation_chain() {
        // A invokes B invokes C — a finite chain, never flagged (bite: the cycle
        // detector must not over-reject non-recursive nesting).
        let source = r#"
workflow A {
  input task TA
  output result RA
  class TA { id string }
  class RA { id string }
  rule go
    when TA as t
  => {
    invoke B { task { id t.id } } as b
    after b succeeds as r { complete result { id r.id } }
  }
}

workflow B {
  input task TB
  output result RB
  class TB { id string }
  class RB { id string }
  rule go
    when TB as t
  => {
    invoke C { task { id t.id } } as c
    after c succeeds as r { complete result { id r.id } }
  }
}

workflow C {
  input task TC
  output result RC
  class TC { id string }
  class RC { id string }
  rule go
    when TC as t
  => {
    complete result { id t.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("A"));
        assert!(
            !compiled.diagnostics.iter().any(|d| d
                .message
                .contains("graph.unbounded_workflow_invocation_recursion")),
            "acyclic chain wrongly flagged: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn whole_program_validation_catches_a_broken_sibling_under_any_root() {
        // Two workflows; `Good` is well-formed, `Broken` references an undeclared
        // schema in its own scope. Compiling with `--root Good` must still catch
        // `Broken`'s error — the pre-pass validates EVERY workflow, not just the
        // selected root (RESOLVED 2026-07-01). Before this, a broken sibling was
        // silently discarded by root selection and never validated.
        let source = r#"
workflow Good {
  input task TG
  output result RG
  class TG { id string }
  class RG { id string }
  rule go
    when TG as t
  => {
    complete result { id t.id }
  }
}

workflow Broken {
  input task TB
  output result RB
  class TB { id string }
  class RB { id string }
  rule go
    when Nonexistent as t
  => {
    complete result { id t.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Good"));
        assert!(
            compiled.ir.is_none(),
            "a program with a broken sibling must not compile"
        );
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("Nonexistent")),
            "the broken sibling's error was not surfaced: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn cross_workflow_reference_to_sibling_local_is_annotated() {
        // `Consumer` references class `Secret`, which is declared *inside* sibling
        // workflow `Owner` (private to it). The reference is an error (isolation),
        // and it carries a related note pointing at Owner's declaration so the
        // author knows the name exists but is out of scope.
        let source = r#"
workflow Owner {
  input task TO
  output result RO
  class TO { id string }
  class RO { id string }
  class Secret { id string }
  rule go
    when TO as t
  => {
    complete result { id t.id }
  }
}

workflow Consumer {
  input task TC
  output result RC
  class TC { id string }
  class RC { id string }
  rule go
    when Secret as s
  => {
    complete result { id s.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Consumer"));
        assert!(
            compiled.ir.is_none(),
            "sibling-local reference must not compile"
        );
        let leak = compiled
            .diagnostics
            .iter()
            .find(|d| d.message.contains("`Secret`"))
            .expect("an unknown-name diagnostic for Secret");
        assert!(
            leak.related
                .iter()
                .any(|r| r.message.contains("workflow `Owner`")
                    && r.message.contains("private to that workflow")),
            "missing sibling-local leak note: {:?}",
            leak.related
        );
    }

    #[test]
    fn shared_top_level_name_is_not_annotated_as_a_leak() {
        // Bite: a class declared at the TOP LEVEL is global — both workflows may
        // reference it, and no leak note is attached. (Also confirms the program
        // compiles: the shared global resolves in each workflow.)
        let source = r#"
class Shared { id string }

workflow Alpha {
  input task Shared
  output result RA
  class RA { id string }
  rule go
    when Shared as s
  => {
    complete result { id s.id }
  }
}

workflow Beta {
  input task Shared
  output result RB
  class RB { id string }
  rule go
    when Shared as s
  => {
    complete result { id s.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Alpha"));
        assert!(
            compiled.diagnostics.is_empty(),
            "shared top-level global wrongly rejected: {:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn whole_program_validation_accepts_all_well_formed_workflows() {
        // Bite: when every workflow is well-formed, the pre-pass adds no spurious
        // diagnostics and the selected root still compiles to IR.
        let source = r#"
workflow Alpha {
  input task TA
  output result RA
  class TA { id string }
  class RA { id string }
  rule go
    when TA as t
  => {
    complete result { id t.id }
  }
}

workflow Beta {
  input task TB
  output result RB
  class TB { id string }
  class RB { id string }
  rule go
    when TB as t
  => {
    complete result { id t.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Alpha"));
        assert!(
            compiled.diagnostics.is_empty(),
            "well-formed multi-workflow program emitted diagnostics: {:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some(), "selected root failed to compile");
    }

    #[test]
    fn compact_workflow_signature_desugars_to_keyword_contracts() {
        // `Name(in: T) -> Out ! Fail` must produce exactly the contracts the
        // keyword form produces, with output named `result` and failure `error`.
        let compact = r#"
workflow Triage(ticket: Ticket) -> Resolution ! TriageFailed

class Ticket { id string }
class Resolution { id string }
class TriageFailed { reason string }

rule go
  when Ticket as t
=> {
  complete result { id t.id }
}
"#;
        let keyword = r#"
workflow Triage

input ticket Ticket
output result Resolution
failure error TriageFailed

class Ticket { id string }
class Resolution { id string }
class TriageFailed { reason string }

rule go
  when Ticket as t
=> {
  complete result { id t.id }
}
"#;
        let compact_ir = compile_program_with_root(compact, None);
        let keyword_ir = compile_program_with_root(keyword, None);
        assert!(
            compact_ir.diagnostics.is_empty(),
            "compact form did not compile: {:?}",
            compact_ir.diagnostics
        );
        assert!(
            keyword_ir.diagnostics.is_empty(),
            "keyword form did not compile: {:?}",
            keyword_ir.diagnostics
        );
        // Compare the semantic triple (kind, name, type) — spans naturally differ
        // between the two source layouts.
        let project = |ir: &IrProgram| {
            ir.workflow_contracts
                .iter()
                .map(|c| {
                    (
                        format!("{:?}", c.kind),
                        c.name.clone(),
                        format!("{:?}", c.ty),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            project(&compact_ir.ir.expect("compact ir")),
            project(&keyword_ir.ir.expect("keyword ir")),
            "compact signature did not desugar to the same contracts"
        );
    }

    #[test]
    fn compact_signature_supports_multiple_inputs_and_optional_failure() {
        // Multiple comma-separated inputs; failure clause omitted.
        let source = r#"
workflow Merge(left: LeftIn, right: RightIn) -> Merged

class LeftIn { id string }
class RightIn { id string }
class Merged { id string }

rule go
  when {
    LeftIn as l
    RightIn as r
  }
=> {
  complete result { id l.id }
}
"#;
        let compiled = compile_program_with_root(source, None);
        assert!(
            compiled.diagnostics.is_empty(),
            "multi-input compact form did not compile: {:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("ir");
        let inputs = ir
            .workflow_contracts
            .iter()
            .filter(|c| matches!(c.kind, IrWorkflowContractKind::Input))
            .count();
        let failures = ir
            .workflow_contracts
            .iter()
            .filter(|c| matches!(c.kind, IrWorkflowContractKind::Failure))
            .count();
        assert_eq!(inputs, 2, "expected two inputs");
        assert_eq!(
            failures, 0,
            "omitted failure clause must add no failure contract"
        );
    }

    #[test]
    fn rejects_headerless_program_with_no_workflow() {
        // The implicit compatibility root is removed (RESOLVED 2026-07-01): a
        // source with no explicit `workflow` (only shared types/patterns) is a
        // library fragment, not a runnable program, and is rejected.
        let source = r#"
class SharedTicket {
  id string
}

pattern TagReviewed<Input> {
  rule tag
    when Input as item
  => {
    record SharedTicket { id item.id }
  }
}
"#;
        let compiled = compile_program_with_root(source, None);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("program declares no `workflow`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn accepts_single_workflow_header_program() {
        // Bite: the headerless reject must not fire on a program that declares a
        // workflow via the header form (the common single-workflow shape).
        let source = r#"
workflow OnlyOne

input item Job
output result Done

class Job { id string }
class Done { id string }

rule go
  when Job as j
=> {
  complete result { id j.id }
}
"#;
        let compiled = compile_program_with_root(source, None);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("program declares no `workflow`")),
            "header-form program wrongly rejected as headerless: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_recording_observer_only_terminal_schema() {
        // §5.4: the terminal family is `origin = observer` — the kernel projects
        // it, user rules may only eliminate it. A rule that `record`s a terminal
        // schema forges an outcome the kernel never produced, so it is rejected.
        for schema in ["TerminalFailed", "TerminalTimedOut", "TerminalCancelled"] {
            let source = format!(
                r#"
workflow Forge

input item Job
output result Done

class Job {{ id string }}
class Done {{ id string }}

rule sneak
  when Job as q
=> {{
  record {schema} {{ reason "x" summary "y" }}
  complete result {{ id q.id }}
}}
"#
            );
            let compiled = compile_program(&source);
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains(&format!(
                        "cannot record kernel-owned terminal schema `{schema}`"
                    ))),
                "expected rejection for {schema}, got {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn allows_recording_user_writable_builtin_schema() {
        // Regression guard for the fix above: `WorkItem` is a builtin schema ref
        // but user-writable (work-tracking state), so recording it must NOT be
        // rejected as observer-only.
        let source = r#"
workflow WriteWork

input item Job
output result Done

class Job { id string }
class Done { id string }

rule track
  when Job as q
=> {
  record WorkItem { title "t" status "reviewed" }
  complete result { id q.id }
}
"#;
        let compiled = compile_program(source);
        assert!(
            !compiled.diagnostics.iter().any(|d| d
                .message
                .contains("cannot record kernel-owned terminal schema")),
            "WorkItem must remain user-writable, got {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn exhaustive_bool_case_compiles() {
        // `case` over a `bool` field is valid when both `true` and `false` are
        // covered (the finite two-value domain).
        let source = r#"
workflow BoolCaseOk

output result Done

class Done {
  note string
}

class Flag {
  ready bool
}

rule route
  when Flag as f
=> {
  case f.ready {
    true => {
      complete result {
        note "t"
      }
    }
    false => {
      complete result {
        note "f"
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some());
    }

    #[test]
    fn bool_case_rejects_non_exhaustive_and_non_bool_patterns() {
        let source = r#"
workflow BoolCaseBad

class Flag {
  ready bool
}

rule route
  when Flag as f
=> {
  case f.ready {
    true => {
    }
  }

  case f.ready {
    maybe => {
    }
    false => {
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("non-exhaustive case; missing false")),
            "expected non-exhaustive diagnostic: {:?}",
            compiled.diagnostics
        );
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("case pattern `maybe` that is not a `bool` value")),
            "expected non-bool pattern diagnostic: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn exec_schema_result_resolves_typed_fields_for_case() {
        // `exec "..." -> Schema as v` registers its result type, so an
        // `after v succeeds as r` branch can `case` / field-access `r`'s fields —
        // the same after-binding type flow a named `coerce -> Schema` already
        // gets. Before the fix this produced "case scrutinee `r.kind` ... not a
        // typed path".
        let source = r#"
@service
workflow ExecTyped

class Pick { kind "a" | "b" }
class R { choice string }

output result R

signal go.now {
  x string
}

rule j
  when go.now as g
=> {
  exec "echo hi" -> Pick as v

  after v succeeds as r {
    case r.kind {
      "a" => {
        complete result {
          choice "a"
        }
      }
      "b" => {
        complete result {
          choice "b"
        }
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("not a typed path")),
            "exec -> Schema result fields should resolve: {:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some(), "{:?}", compiled.diagnostics);
    }

    #[test]
    fn redact_projection_keeps_only_kept_fields() {
        // `redact c keep [id, status] as safe` synthesizes a projected class
        // holding only the kept fields, so `safe.id` resolves but `safe.ssn`
        // (dropped) is an unknown field — the type-system half of redaction
        // soundness (a dropped field cannot be reached through the projection).
        let kept = r#"
@service
workflow RedactKept

class Customer { id string  ssn string  status string }
class Result { tag string }
output result Result

signal go.now { x string }

coerce read_customer(x string) -> Customer { prompt "x" }

rule r
  when go.now as g
=> {
  coerce read_customer(g.x) as c
  after c succeeds as cust {
    redact cust keep [id, status] as safe
    complete result {
      tag safe.id
    }
  }
}
"#;
        let compiled = compile_program(kept);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown field") || d.message.contains("not a typed")),
            "kept field `safe.id` should resolve: {:?}",
            compiled.diagnostics
        );

        assert!(
            compiled.ir.is_some(),
            "kept program should compile: {compiled:?}"
        );

        let dropped = kept.replace("tag safe.id", "tag safe.ssn");
        let compiled = compile_program(&dropped);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("safe.ssn") || d.message.contains("`ssn`")),
            "dropped field `safe.ssn` should be rejected: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn redact_unknown_kept_field_is_rejected() {
        let source = r#"
@service
workflow RedactBadKeep

class Customer { id string  status string }
class Result { tag string }
output result Result

signal go.now { x string }

coerce read_customer(x string) -> Customer { prompt "x" }

rule r
  when go.now as g
=> {
  coerce read_customer(g.x) as c
  after c succeeds as cust {
    redact cust keep [id, nonexistent] as safe
    complete result {
      tag safe.id
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("keeping unknown field `nonexistent`")),
            "expected unknown-kept-field rejection: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn inline_decide_result_resolves_typed_fields_for_case() {
        // `decide -> { fixed bool } as v` synthesizes a hygienic anonymous result
        // class, so `after v succeeds as r` can `case` / field-access `r`'s
        // fields — the same after-binding type flow a named `coerce -> Schema`
        // gets. Before this, an inline decide result had no type, so `r.fixed`
        // was "not a typed path" and `case r.fixed { true/false }` could not bind.
        let source = r#"
@service
workflow InlineDecideTyped

class R { choice string }
output result R

signal go.now {
  x string
}

rule j
  when go.now as g
=> {
  decide "is it fixed?" -> { fixed bool } as v

  after v succeeds as r {
    case r.fixed {
      true => {
        complete result {
          choice "a"
        }
      }
      false => {
        complete result {
          choice "b"
        }
      }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("not a typed path")),
            "inline decide result fields should resolve: {:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("compiles");
        // The synthesized hygienic class is visible in the IR (so the runtime
        // fixture can generate the anonymous shape).
        assert!(
            ir.schemas.iter().any(|schema| matches!(
                schema,
                IrSchema::Class(class) if class.name == "decide.j.v"
            )),
            "expected synthesized inline-decide class `decide.j.v` in IR schemas"
        );
    }

    #[test]
    fn rejects_flow_statement_after_branch() {
        // A `when/else` branch ends its flow segment. A statement after it used
        // to leak the internal `class FlowAwait_f_<n> has no field <x>` error
        // (the trailing statement's field name fell into a flow-state lookup);
        // now it is rejected with a clear message.
        let source = r#"
@service
workflow AfterBranch

output result R
class R { v string }
class Note { t string }
signal go.now { x string }

flow f
  when go.now as g
{
  askHuman as a choices ["y", "n"] "pick"
  when a.choice == "y" {
    complete result {
      v "yes"
    }
  } else {
    complete result {
      v "no"
    }
  }
  record Note {
    t "trailing"
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("statement after a `when/else` branch")),
            "expected clear post-branch-statement diagnostic: {:?}",
            compiled.diagnostics
        );
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("FlowAwait")),
            "internal FlowAwait error should be suppressed: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn signal_triggered_flow_with_post_ask_branch_compiles() {
        // Regression: a flow triggered by a SIGNAL (dotted schema) with an
        // `askHuman` + post-ask `when/else` used to fail with the internal
        // `class FlowAwait_<flow>_<n> has no field t` — the trigger `t` field is
        // omitted for a dotted/signal schema (no class to `Ref`), but the
        // post-ask segments still read `flowState.t`. Now the trigger is simply
        // not carried for signal triggers, keeping read and write consistent.
        let source = r#"
@service
workflow SigFlow

output result R
class R { v string }
signal go.now { x string }

flow f
  when go.now as g
{
  askHuman as ans choices ["y", "n"] "pick"
  when ans.choice == "y" {
    complete result {
      v "yes"
    }
  } else {
    complete result {
      v "no"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("FlowAwait")),
            "signal-triggered flow should not leak FlowAwait internals: {:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some(), "{:?}", compiled.diagnostics);
    }

    #[test]
    fn rejects_flow_branch_without_preceding_ask() {
        // A flow `when/else` decides on a human answer, so it must follow an
        // `askHuman`. A branch in the initial (pre-ask) segment is rejected with
        // a clear diagnostic rather than silently lowering to seg rules that
        // consume an unestablished `flowState` (a confusing internal error).
        let source = r#"
workflow FlowBranchNoAsk

output result R
failure error E
class R { note string }
class E { reason string }
class Flag { ready bool }

rule seed
  when started
=> {
  record Flag {
    ready true
  }
}

flow only_branch
  when Flag as f
{
  when f.ready {
    complete result {
      note "yes"
    }
  } else {
    fail error {
      reason "no"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("branch that does not directly follow an `askHuman")),
            "expected clear flow-branch diagnostic: {:?}",
            compiled.diagnostics
        );
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("flowState")),
            "internal flowState error should be suppressed: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_malformed_multiline_prompt_content_type_on_rule_prompt() {
        let source = r#"
workflow PromptAnnotationGuess

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule ask
  when started
=> {
  tell worker as turn """markdown extra
  do work
  """
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("malformed multiline prompt content type `markdown extra`")
                && diagnostic.suggestion.as_deref().is_some_and(|suggestion| {
                    suggestion.contains("put prompt text on the next line")
                })
        }));
    }

    #[test]
    fn rejects_malformed_multiline_prompt_content_type_on_coerce_prompt() {
        let source = r#"
workflow CoerceAnnotationGuess

class Review {
  status "ok"
}

coerce review() -> Review {
  prompt """text/markdown extra
  classify the review
  """
}

rule run
  when started
=> {
  coerce review() as result
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(
                "coerce `review` has malformed multiline prompt content type `text/markdown extra`"
            )));
    }

    #[test]
    fn rejects_pasted_top_level_gherkin_with_targeted_diagnostic() {
        let source = r#"
Feature: provider language routing

Scenario: fixture provider reviews every language task
  Given a queued language task
  When the provider turn completes
  Then the language result is reviewed
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Gherkin keyword `Feature` is not WhippleScript workflow syntax")
                && diagnostic.suggestion.as_deref().is_some_and(|suggestion| {
                    suggestion.contains("use `workflow`, `table`, `rule")
                        && suggestion.contains("instead of free-text Given/When/Then steps")
                })
        }));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Gherkin keyword `Given` is not WhippleScript workflow syntax")));
    }

    #[test]
    fn rejects_pasted_gherkin_inside_workflow_body_with_targeted_diagnostic() {
        let source = r#"
workflow PastedGherkin {
  Scenario: fixture provider reviews every language task
  Given a queued language task
  When the provider turn completes
  Then the language result is reviewed
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Gherkin keyword `Scenario` is not WhippleScript workflow syntax")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Gherkin keyword `Then` is not WhippleScript workflow syntax")));
    }

    #[test]
    fn rejects_pasted_gherkin_background_outline_examples_and_continuations() {
        let source = r#"
Feature: provider language routing

Rule: provider execution remains explicit

Background:
  Given a seeded provider table
  And all provider profiles are available

Scenario Outline: provider reviews language task
  When <provider> completes <language>
  But the review is missing
  Then the fixture fails

Examples:
  | provider | language |
  | codex    | French   |
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        for keyword in ["Rule", "Background", "And", "Scenario", "But", "Examples"] {
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(&format!(
                        "Gherkin keyword `{keyword}` is not WhippleScript workflow syntax"
                    ))),
                "missing diagnostic for {keyword}: {:?}",
                compiled
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn explains_multiline_string_binding_position() {
        let source = r#"
workflow BindingGuess

agent worker {
  provider fixture
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
                "bad-terminal-payload",
                include_str!("../../../examples/invalid/bad-terminal-payload.whip"),
            ),
            (
                "recursive-workflow-invocation",
                include_str!("../../../examples/invalid/recursive-workflow-invocation.whip"),
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
                "recursive-pattern",
                include_str!("../../../examples/invalid/recursive-pattern.whip"),
            ),
            (
                "flow-state-access",
                include_str!("../../../examples/invalid/flow-state-access.whip"),
            ),
            (
                "evidence-fact-match",
                include_str!("../../../examples/invalid/evidence-fact-match.whip"),
            ),
            (
                "unknown-schema",
                include_str!("../../../examples/invalid/unknown-schema.whip"),
            ),
            (
                "headerless-library",
                include_str!("../../../examples/invalid/headerless-library.whip"),
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
    fn rejects_dangling_root_in_record_value() {
        // A record value referencing a binding that does not exist (a typo or an
        // unbound name) is a dangling reference that previously compiled silently.
        let source = r#"
@service
workflow DanglingRoot

class Ticket { id string }
class Note { text string }

table seed as Ticket [ { id "1" } ]

rule r
  when Ticket as ticket
=> {
  record Note {
    text tikcet.id
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_single_line_record() {
        // Single-line records (`record X { f y }`) were skipped by the line-based
        // extractor (brace_delta 0) and so were never field-validated; they are
        // now covered.
        let source = r#"
@service
workflow DanglingSingleLine

class Ticket { id string }
class Note { text string }

table seed as Ticket [ { id "1" } ]

rule r
  when Ticket as ticket
=> {
  record Note { text tikcet.id }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_coerce_argument() {
        // Coerce arguments are another value position where a typo'd/unbound root
        // was previously accepted leniently by the type-checker.
        let source = r#"
@service
workflow DanglingCoerceArg

class Ticket { id string  title string }
class Review { summary string }

coerce classify(title string) -> Review { prompt "c" }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "1"  title "t" } ]

rule r
  when Ticket as ticket
  when reviewer is available
=> {
  coerce classify(tikcet.title) as rev
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")
                    && d.message.contains("coerce `classify`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_counter_consume_operand() {
        // Non-field effect operands (lease/counter `for <key>`, `emit ... to
        // <target>`) are also checked via `check_operand_root`.
        let source = r#"
@service
workflow CounterOperandDangling

class CallFailed { service string }
class Service { id string }

counter failure_budget { key Service cap 3 reset daily }

table seed as CallFailed [ { service "x" } ]

rule strike
  when CallFailed as f
=> {
  consume failure_budget for fff.service amount 1 as strike
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `fff`")
                    && d.message.contains("consume")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_queue_file_payload() {
        // Body-AST effect payloads (`file item into`, `emit`, ledger `append`)
        // are validated via `validate_effect_field_roots`, not the line-based
        // validators; their field values were previously unchecked for roots.
        let source = r#"
@service
workflow QueueFieldDangling

class Ticket { id string }

queue backlog { tracker builtin }

table seed as Ticket [ { id "1" } ]

rule r
  when Ticket as ticket
=> {
  file item into backlog {
    title tikcet.id
    body "x"
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")
                    && d.message.contains("file into")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_invoke_input() {
        // Invoke payload inputs are another value position the type-checker
        // accepted leniently for unknown roots.
        let source = r#"
workflow Parent {
  input task Task
  output result Out

  class Task { id string }
  class Out { x string }

  rule r
    when Task as task
  => {
    invoke Child { item tikcet.id } as c
    after c succeeds as cr {
      done task
      complete result { x cr.summary }
    }
  }
}

workflow Child {
  input item string
  output result ChildOut
  class ChildOut { y string }
  rule c
    when item as i
  => {
    complete result { y "done" }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")
                    && d.message.contains("invoke Child")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_dangling_root_in_tell_target() {
        // A dynamic tell target with a typo'd/unbound root was silently accepted
        // (the type lookup returned None and bailed).
        let source = r#"
@service
workflow DanglingTellTarget

class Ticket { id string  provider AgentRef<reviewer> }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "1"  provider reviewer } ]

rule r
  when Ticket as ticket
=> {
  tell tikcet.provider as turn "go"
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown binding `tikcet`")
                    && d.message.contains("tell target")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn accepts_effect_binding_root_in_record_value() {
        // The AST-collected root set must include `tell`/`after` results (which
        // the typed `binding_types` map omits), so reading an effect-result field
        // in a record value compiles. This is the case a naive binding_types-only
        // check wrongly rejected.
        let source = r#"
@service
workflow EffectRoot

class Ticket { id string }
class Note { text string }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "1" } ]

rule r
  when Ticket as ticket
  when reviewer is available
=> {
  tell reviewer as turn "review"
  after turn succeeds {
    record Note {
      text turn.summary
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some());
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
    fn rejects_non_file_operation_on_a_file_store_grant() {
        // A grant on a declared file store may only use file operations; a non-file op
        // (e.g. `recall`) is rejected. A non-file-store resource is left alone.
        let program = |op: &str, resource: &str, store: &str| {
            format!(
                r#"
@service
workflow FileGrant

output result R
class R {{ ok bool }}
class Ticket {{ id string  status "open" }}

agent coder {{ provider fixture  profile "repo-writer"  capacity 1 }}

file store {store} {{ root "./data"  allow read ["docs/**"] }}

table seed as Ticket [ {{ id "T1"  status "open" }} ]

rule work
  when Ticket as ticket where ticket.status == "open"
  when coder is available
=> {{
  tell coder as turn
    with access to {resource} {{
      {op}
    }}
  "go"

  after turn succeeds as outcome {{
    complete result {{ ok true }}
  }}
}}
"#
            )
        };

        // `recall` on a declared file store is not a file operation.
        let bad = compile_program(&program(
            "recall for ticket",
            "project_files",
            "project_files",
        ));
        assert!(
            bad.diagnostics
                .iter()
                .any(|d| d.message.contains("not a file operation")),
            "{:?}",
            bad.diagnostics
        );
        // The same `recall` on a non-file-store resource is left alone (could be a
        // package resource) — no false positive.
        let ok = compile_program(&program(
            "recall for ticket",
            "project_memory",
            "project_files",
        ));
        assert!(
            !ok.diagnostics
                .iter()
                .any(|d| d.message.contains("not a file operation")),
            "{:?}",
            ok.diagnostics
        );
    }

    #[test]
    fn rejects_malformed_turn_access_grants() {
        // An empty grant block and a duplicate resource on one `tell` are both
        // structurally malformed.
        let program = |grant_block: &str| {
            format!(
                r#"
@service
workflow GrantCheck

output result R
class R {{ ok bool }}
class Ticket {{ id string  status "open" }}

agent coder {{ provider fixture  profile "repo-writer"  capacity 1 }}

table seed as Ticket [ {{ id "T1"  status "open" }} ]

rule work
  when Ticket as ticket where ticket.status == "open"
  when coder is available
=> {{
  tell coder as turn
{grant_block}
  "Work it."

  after turn succeeds as outcome {{
    complete result {{ ok true }}
  }}
}}
"#
            )
        };

        let empty = compile_program(&program("    with access to project_memory {\n    }\n"));
        assert!(
            empty
                .diagnostics
                .iter()
                .any(|d| d.message.contains("grants no operations")),
            "{:?}",
            empty.diagnostics
        );

        let duplicate = compile_program(&program(
            "    with access to project_memory {\n      recall for ticket\n    }\n    with access to project_memory {\n      learn for ticket\n    }\n",
        ));
        assert!(
            duplicate
                .diagnostics
                .iter()
                .any(|d| d.message.contains("more than once")),
            "{:?}",
            duplicate.diagnostics
        );
    }

    #[test]
    fn lowers_turn_access_grants_onto_the_agent_tell_effect() {
        // `with access to <resource> { … }` on a tell lowers to `access_grants` on the
        // agent.tell IR effect (Proposal A authority-narrowing metadata).
        let source = r#"
@service
workflow GrantDemo

output result R
class R { ok bool }
class Ticket { id string  status "open" }

agent coder { provider fixture  profile "repo-writer"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
  when coder is available
=> {
  tell coder as turn
    with access to project_memory {
      recall for ticket
      learn for ticket
    }
    with access to project_files {
      read ["docs/**"]
    }
  "Work it."

  after turn succeeds as outcome {
    complete result { ok true }
  }
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.expect("compiles");
        let tell = ir
            .rules
            .iter()
            .flat_map(|rule| rule.metadata.effects.iter())
            .find(|effect| effect.kind == IrEffectKind::AgentTell)
            .expect("agent.tell effect");
        assert_eq!(tell.access_grants.len(), 2);
        let memory = &tell.access_grants[0];
        assert_eq!(memory.resource, "project_memory");
        assert_eq!(memory.operations.len(), 2);
        assert_eq!(memory.operations[0].operation, "recall");
        assert_eq!(memory.operations[0].target.as_deref(), Some("ticket"));
        let files = &tell.access_grants[1];
        assert_eq!(files.resource, "project_files");
        assert_eq!(files.operations[0].operation, "read");
        assert_eq!(files.operations[0].globs, vec!["docs/**".to_owned()]);
    }

    #[test]
    fn rejects_rule_matching_evidence_only_turn_fact() {
        // In-turn observations (streamed/tool_requested/artifact_captured) are
        // evidence, never rule-matchable (spec/agent-harness.md); a `when` on them is
        // an error. The lifecycle facts (completed/failed/…) stay matchable.
        for evidence in [
            "agent.turn.streamed",
            "agent.turn.tool_requested",
            "agent.turn.artifact_captured",
        ] {
            let source = format!(
                "workflow EvidenceMatch\n\noutput result R\nclass R {{ ok bool }}\n\nrule react\n  when fact {evidence} as ev\n=> {{\n  complete result {{ ok true }}\n}}\n"
            );
            let compiled = compile_program(&source);
            assert!(compiled.ir.is_none(), "{evidence} should be rejected");
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("evidence-only fact")
                        && d.message.contains(evidence)),
                "{evidence}: {:?}",
                compiled.diagnostics
            );
        }
        // A matchable lifecycle fact does NOT get the evidence error (it may fail a
        // different check, e.g. needing a producer, but not this one).
        let matchable = "workflow M\n\noutput result R\nclass R {{ ok bool }}\n\nrule react\n  when fact agent.turn.completed as ev\n=> {{\n  complete result {{ ok true }}\n}}\n".replace("{{", "{").replace("}}", "}");
        let compiled = compile_program(&matchable);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("evidence-only fact")),
            "completed must not be flagged as evidence-only: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn rejects_user_rule_accessing_flow_state() {
        // Flow progression state (FlowAwait_*) is owned by the flow's generated
        // rules; a user rule that matches/reads it is rejected.
        let source = include_str!("../../../examples/invalid/flow-state-access.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        let violations: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("may not reference flow-state fact"))
            .collect();
        assert_eq!(violations.len(), 1, "{:?}", compiled.diagnostics);
        assert!(
            violations[0].message.contains("FlowAwait_triage_ticket_1"),
            "names the offending flow-state fact: {}",
            violations[0].message
        );
    }

    #[test]
    fn rejects_user_rule_using_flowfail_terminal() {
        // `flowfail` is the generated-only 503 auto-fail terminal; an author rule
        // that writes it is rejected (use a typed `fail <Failure>` instead).
        let source = r#"
@service
workflow W

class Tick { id string }

table seed as Tick [ { id "T1" } ]

rule r
  when Tick as t
=> {
  flowfail
}
"#;
        let compiled = compile_program(source);
        let violations: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("generated-only `flowfail` terminal"))
            .collect();
        assert_eq!(violations.len(), 1, "{:?}", compiled.diagnostics);
    }

    #[test]
    fn flow_generated_rules_may_access_flow_state() {
        // The legitimate flow itself compiles: its generated `flow.*` rules read
        // the FlowAwait_* state without tripping the namespace lint.
        let compiled = compile_program(include_str!("../../../examples/triage-flow.whip"));
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("may not reference flow-state fact")),
            "generated flow rules must be exempt: {:?}",
            compiled.diagnostics
        );
        assert!(compiled.ir.is_some(), "triage-flow should compile");
    }

    #[test]
    fn rejects_self_recursive_pattern_application() {
        // A pattern whose body applies itself is unbounded recursion: it can never
        // elaborate into a finite first-order program (graph.unbounded_pattern_recursion).
        let source = include_str!("../../../examples/invalid/recursive-pattern.whip");
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        // Exactly the precise recursion diagnostic — the generic "nested apply not
        // supported yet" message must be suppressed for the recursive case.
        assert_eq!(compiled.diagnostics.len(), 1, "{:?}", compiled.diagnostics);
        let diagnostic = &compiled.diagnostics[0];
        assert!(
            diagnostic
                .message
                .contains("graph.unbounded_pattern_recursion"),
            "{}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("expansion cycle Loop -> Loop"),
            "the diagnostic names the cycle: {}",
            diagnostic.message
        );
    }

    #[test]
    fn rejects_mutually_recursive_pattern_application() {
        // A cycle that spans two patterns is rejected once, naming the full cycle.
        let source = r#"
workflow MutualRecursion

class Item {
  id string
}

pattern Ping<T> {
  apply Pong<T> as a {
  }
}

pattern Pong<T> {
  apply Ping<T> as b {
  }
}

apply Ping<Item> as top {
}
"#;
        let compiled = compile_program(source);

        assert!(compiled.ir.is_none());
        let recursion: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("graph.unbounded_pattern_recursion"))
            .collect();
        // One cycle, reported once (members covered are not re-reported).
        assert_eq!(recursion.len(), 1, "{:?}", compiled.diagnostics);
        assert!(
            recursion[0].message.contains("Ping -> Pong -> Ping"),
            "names the full cycle: {}",
            recursion[0].message
        );
    }

    #[test]
    fn allows_non_recursive_nested_apply_without_recursion_error() {
        // A nested apply that does NOT form a cycle is a separate v0 limitation
        // (the generic "not supported yet" message), NOT a recursion error.
        let source = r#"
workflow NonRecursive

class Item {
  id string
}

pattern Inner<T> {
}

pattern Outer<T> {
  apply Inner<T> as x {
  }
}

apply Outer<Item> as top {
}
"#;
        let compiled = compile_program(source);

        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("graph.unbounded_pattern_recursion")),
            "non-recursive nesting must not be flagged as recursion: {:?}",
            compiled.diagnostics
        );
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
    fn rejects_then_sequencing() {
        let source = r#"
workflow NoThen

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
  provider codex
  profile "repo-writer"
  capacity 1
}

assert count(Task where status == "queued") == 0
assert count(Result where status == "done") == 1

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
        assert!(compiled.ir.is_none());
        assert!(compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unsupported `then` sequencing")));
    }

    #[test]
    fn rejects_after_arrow_sequencing() {
        let source = r#"
workflow NoAfterArrow

agent codex {
  provider codex
  profile "repo-writer"
  capacity 1
}

rule finish
  when started
  when codex is available
=> {
  tell codex as turn "write"

  after turn succeeds => {
    record Done {
      status "done"
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(compiled.ir.is_none());
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unsupported `after ... =>` sequencing")));
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
    fn formats_content_typed_multiline_prompts() {
        let source = r#"workflow PromptFormat
class Review {
status "ok"
}
coerce review() -> Review {
prompt """markdown
classify
"""
}
agent worker {
  provider fixture
profile "repo-writer"
capacity 1
}
rule start
when started
=> {tell worker as turn """markdown
write
"""
askHuman """application/json
{"question":"approve?"}
"""}
"#;

        let formatted = format_program(source);
        assert_eq!(formatted.diagnostics, Vec::new());
        let expected = concat!(
            "workflow PromptFormat\n",
            "\n",
            "class Review {\n",
            "  status \"ok\"\n",
            "}\n",
            "\n",
            "coerce review() -> Review {\n",
            "  prompt \"\"\"markdown\n",
            "  classify\n",
            "  \"\"\"\n",
            "}\n",
            "\n",
            "agent worker {\n",
            "  provider fixture\n",
            "  profile \"repo-writer\"\n",
            "  capacity 1\n",
            "}\n",
            "\n",
            "rule start\n",
            "  when started\n",
            "=> {\n",
            "  tell worker as turn \"\"\"markdown\n",
            "  write\n",
            "  \"\"\"\n",
            "  askHuman \"\"\"application/json\n",
            "  {\"question\":\"approve?\"}\n",
            "  \"\"\"\n",
            "}\n",
        );

        assert_eq!(formatted.formatted.as_deref(), Some(expected));
    }

    #[test]
    fn formats_harness_declarations_and_agent_bindings() {
        let source = r#"workflow HarnessFormat
harness coder: codex
agent implementer using coder {
profile "repo-writer"
capacity 1
}
"#;

        let formatted = format_program(source);
        assert_eq!(formatted.diagnostics, Vec::new());
        let expected = concat!(
            "workflow HarnessFormat\n",
            "\n",
            "harness coder: codex\n",
            "\n",
            "agent implementer using coder {\n",
            "  profile \"repo-writer\"\n",
            "  capacity 1\n",
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

    #[test]
    fn lexer_captures_comments_without_affecting_tokens() {
        let source =
            "# top comment\nworkflow Demo\n\nclass Task {\n  title string  // trailing\n}\n";
        let comments = lex_comments(source);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].marker, CommentMarker::Hash);
        assert_eq!(comments[0].text, "top comment");
        assert_eq!(comments[1].marker, CommentMarker::Slash);
        assert_eq!(comments[1].text, "trailing");
        // Spans point back at the original source slice (marker through line end).
        let first = &comments[0];
        assert_eq!(&source[first.span.start..first.span.end], "# top comment");
        // Comments stay out of the parse: the program still compiles cleanly.
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
    }

    #[test]
    fn test_block_parses_given_run_and_expect_clauses() {
        let source = r#"
@service
workflow Demo

test "ci triage" {
  given signal github.workflow_failed {
    run_id "run_123"
  }
  stub agent triager succeeds
  run until idle
  expect issue count where external_id == "run_123" is 1
  expect rule triage_failed_run fired
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        assert_eq!(ir.tests.len(), 1);
        let test = &ir.tests[0];
        assert_eq!(test.name, "ci triage");
        assert_eq!(test.clauses.len(), 5);

        match &test.clauses[0] {
            TestClause::Given(GivenClause::Signal { name, fields, .. }) => {
                assert_eq!(name, "github.workflow_failed");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name.name, "run_id");
                assert_eq!(fields[0].value, "\"run_123\"");
            }
            other => panic!("expected given signal, got {other:?}"),
        }
        match &test.clauses[1] {
            TestClause::Stub(stub) => {
                assert_eq!(stub.surface, vec!["agent".to_owned(), "triager".to_owned()]);
                assert_eq!(stub.outcome, "succeeds");
            }
            other => panic!("expected stub, got {other:?}"),
        }
        assert!(matches!(
            &test.clauses[2],
            TestClause::Run(RunClause {
                kind: RunKind::UntilIdle,
                ..
            })
        ));
        match &test.clauses[3] {
            TestClause::Expect(ExpectClause {
                target: ExpectTarget::Projection(query),
                ..
            }) => {
                assert_eq!(query.noun, "issue");
                match &query.kind {
                    ProjQueryKind::Count { predicate, count } => {
                        assert_eq!(predicate, "external_id == \"run_123\"");
                        assert_eq!(*count, 1);
                    }
                    other => panic!("expected count query, got {other:?}"),
                }
            }
            other => panic!("expected expect projection, got {other:?}"),
        }
        match &test.clauses[4] {
            TestClause::Expect(ExpectClause {
                target: ExpectTarget::Rule { name, status },
                ..
            }) => {
                assert_eq!(name.name, "triage_failed_run");
                assert_eq!(*status, RuleStatus::Fired);
            }
            other => panic!("expected expect rule, got {other:?}"),
        }
    }

    #[test]
    fn test_block_rejects_a_malformed_predicate() {
        let source = r#"
@service
workflow Demo

test "bad predicate" {
  run until idle
  expect issue count where == == is 1
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("predicate on `issue`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn source_clock_block_lowers_to_clock_source() {
        let source = r#"
workflow ClockSource

signal triage.tick {
  scheduled_at time
  observed_at time
  occurrence_id string
  missed_count int
}

source clock as daily_triage {
  every weekday at 09:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
    observed_at tick.observed_at
    occurrence_id tick.occurrence_id
    missed_count tick.missed_count
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        assert_eq!(ir.sources.len(), 1);
        let decl = &ir.sources[0];
        assert_eq!(decl.name, "daily_triage");
        assert_eq!(decl.provider, "clock");
        assert!(decl.is_clock);
        assert_eq!(decl.observe_binding, "tick");
        assert_eq!(decl.emit_signal, "triage.tick");
        assert_eq!(decl.emit_fields.len(), 4);
        assert_eq!(decl.timezone.as_deref(), Some("America/New_York"));
        assert_eq!(decl.missed, Some(MissedPolicy::Coalesce));
        match &decl.recurrence {
            Some(Recurrence::EveryCalendar { pattern, time, .. }) => {
                assert_eq!(*pattern, CalendarPattern::Weekday);
                assert_eq!(time.hour, 9);
                assert_eq!(time.minute, 0);
            }
            other => panic!("expected calendar recurrence, got {other:?}"),
        }
    }

    #[test]
    fn channel_declaration_parses_and_lowers() {
        let source = r##"
@service
workflow ChannelDecl

use std.messaging

channel release_room {
  provider slack
  workspace ops
  destination "#release"
}

output result R
class R { v string }
signal go.now { x string }

rule j
  when go.now as g
=> {
  complete result {
    v "ok"
  }
}
"##;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        assert_eq!(ir.channels.len(), 1);
        let channel = &ir.channels[0];
        assert_eq!(channel.name, "release_room");
        assert_eq!(channel.provider, "slack");
        assert_eq!(channel.workspace.as_deref(), Some("ops"));
        assert_eq!(channel.destination.as_deref(), Some("#release"));
        // The channel construct auto-registers std.messaging in the contract
        // registry (like leases -> std.coord), and `use std.messaging` parses as
        // a dotted package name.
        let registry = ir.contract_registry();
        assert!(registry
            .libraries
            .iter()
            .any(|library| library.id == "std.messaging"));
        // The generic `Message` envelope is a built-in referenceable schema.
        assert!(SchemaIndex::with_builtins().class_exists("Message"));
    }

    #[test]
    fn channel_requires_a_provider() {
        let source = r#"
@service
workflow ChannelNoProvider

channel orphan {
  workspace ops
}

output result R
class R { v string }
signal go.now { x string }
rule j
  when go.now as g
=> { complete result { v "ok" } }
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("missing a provider")),
            "expected missing-provider diagnostic: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn duplicate_channel_is_rejected() {
        let source = r#"
@service
workflow DupChannel

channel room {
  provider slack
}
channel room {
  provider discord
}

output result R
class R { v string }
signal go.now { x string }
rule j
  when go.now as g
=> { complete result { v "ok" } }
"#;
        let compiled = compile_program(source);
        let dup = compiled
            .diagnostics
            .iter()
            .find(|d| d.message.contains("declared more than once"))
            .expect("expected duplicate-channel diagnostic");
        // The diagnostic carries related-information pointing at the first
        // declaration (spec/error-handling.md "Spans And Labels").
        assert_eq!(dup.related.len(), 1, "expected one related-info entry");
        assert_eq!(dup.related[0].message, "first declared here");
        assert!(dup.related[0].span.start < dup.span.start);
    }

    #[test]
    fn when_message_from_binds_message_and_validates_channel() {
        // `when message from <channel> as msg` binds the built-in `Message`
        // envelope and the channel must be declared (spec/messaging.md).
        let ok = compile_program(
            r#"
@service
workflow Inbound

channel release_room {
  provider slack
}

output result Decision
class Decision { note string }

rule react
  when message from release_room as msg
=> {
  complete result { note msg.text }
}
"#,
        );
        assert!(
            ok.diagnostics.is_empty(),
            "expected clean compile, got {:?}",
            ok.diagnostics
        );
        // `msg.text` resolving against the Message schema proves the binding typed.

        let bad = compile_program(
            r#"
@service
workflow Inbound

channel release_room {
  provider slack
}

output result Decision
class Decision { note string }

rule react
  when message from typo_room as msg
=> {
  complete result { note msg.text }
}
"#,
        );
        assert!(
            bad.diagnostics.iter().any(|d| d
                .message
                .contains("`when message from typo_room` names an unknown channel")),
            "expected unknown-channel diagnostic, got {:?}",
            bad.diagnostics
        );
    }

    #[test]
    fn duplicate_schema_diagnostic_points_at_first_declaration() {
        let source = r#"
@service
workflow DupSchema

class Thing { v string }
class Thing { w string }

output result R
class R { v string }
signal go.now { x string }
rule j
  when go.now as g
=> { complete result { v "ok" } }
"#;
        let compiled = compile_program(source);
        let dup = compiled
            .diagnostics
            .iter()
            .find(|d| {
                d.message
                    .contains("schema `Thing` is declared more than once")
            })
            .expect("expected duplicate-schema diagnostic");
        assert_eq!(dup.related.len(), 1);
        assert_eq!(dup.related[0].message, "first declared here");
        assert!(dup.related[0].span.start < dup.span.start);
    }

    #[test]
    fn interval_clock_source_parses_duration() {
        let source = r#"
workflow Interval

signal tick.beat {
  at_time time
}

source clock as heartbeat {
  every 5m
  missed skip

  observe as tick
  emit tick.beat {
    at_time tick.scheduled_at
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        match &ir.sources[0].recurrence {
            Some(Recurrence::EveryDuration { seconds, .. }) => assert_eq!(*seconds, 300),
            other => panic!("expected duration recurrence, got {other:?}"),
        }
        assert_eq!(ir.sources[0].missed, Some(MissedPolicy::Skip));
    }

    #[test]
    fn fails_binding_types_to_effecterror_base() {
        // DR-0032: `after <effect> fails as f` types `f` to the EffectError base
        // (TerminalFailed: reason, summary, effect_id, run_id, kind). The base
        // fields read cleanly.
        let source = r#"
workflow W {
  input task T
  output result R
  failure error E
  class T { x string }
  class R { y string }
  class E { reason string detail string }

  rule go when T as task => {
    exec "true" as e
    after e fails as f {
      fail error { reason f.reason detail f.kind }
    }
    after e succeeds {
      complete result { y task.x }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("invalid field path")),
            "base fields should type-check: {:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn fails_binding_rejects_non_base_field() {
        // The teeth of DR-0032 base typing: a non-base field read on the failure
        // binding is a check error (extras are deferred behind narrowing).
        let source = r#"
workflow W {
  input task T
  output result R
  failure error E
  class T { x string }
  class R { y string }
  class E { reason string }

  rule go when T as task => {
    exec "true" as e
    after e fails as f {
      fail error { reason f.exit_code }
    }
    after e succeeds {
      complete result { y task.x }
    }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("invalid field path `f.exit_code`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn milestone_reaches_rejects_undeclared_milestone() {
        // Family C terminal-only observation invariant: a parent cannot observe a
        // milestone the invoked child never declares.
        let source = r#"
workflow Parent {
  input task Task
  class Task { title string }
  class Saw { note string }

  rule dispatch when Task as task => {
    invoke Child { task { title task.title } } as child
    after child reaches "never_declared" as m {
      record Saw { note m.note }
    }
  }
}

workflow Child {
  input task Task
  output result R
  class Task { title string }
  class R { title string }
  class P { note string }

  rule go when Task as task => {
    emit milestone "actually_declared" of P { note task.title }
    complete result { title task.title }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert!(
            compiled.diagnostics.iter().any(|d| d.message.contains(
                "reaches milestone `never_declared` that workflow `Child` does not declare"
            )),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn emit_milestone_rejects_unknown_payload_class() {
        let source = r#"
workflow Child {
  input task Task
  output result R
  class Task { title string }
  class R { title string }

  rule go when Task as task => {
    emit milestone "m1" of Nonexistent { note task.title }
    complete result { title task.title }
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled.diagnostics.iter().any(|d| d
                .message
                .contains("emits milestone `m1` with unknown payload class `Nonexistent`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn milestone_reaches_accepts_declared_milestone() {
        // The positive control: a declared milestone is accepted (no
        // reject-undeclared / unknown-class diagnostics).
        let source = r#"
workflow Parent {
  input task Task
  class Task { title string }
  class Saw { note string }

  rule dispatch when Task as task => {
    invoke Child { task { title task.title } } as child
    after child reaches "halfway" as m {
      record Saw { note m.note }
    }
  }
}

workflow Child {
  input task Task
  output result R
  class Task { title string }
  class R { title string }
  class P { note string }

  rule go when Task as task => {
    emit milestone "halfway" of P { note task.title }
    complete result { title task.title }
  }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|d| d.message.contains("reaches milestone")
                    || d.message.contains("unknown payload class")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn recurring_clock_source_requires_missed() {
        let source = r#"
workflow NeedsMissed

signal triage.tick {
  scheduled_at time
}

source clock as daily {
  every weekday at 09:00
  timezone "UTC"

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled.diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("must declare a `missed` policy")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn calendar_clock_source_requires_timezone() {
        let source = r#"
workflow NeedsTimezone

signal triage.tick {
  scheduled_at time
}

source clock as daily {
  every weekday at 09:00
  missed skip

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
  }
}
"#;
        let compiled = compile_program(source);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("should declare a `timezone`")),
            "{:?}",
            compiled.diagnostics
        );
    }

    #[test]
    fn generic_source_block_lowers_to_signal_source() {
        let source = r#"
workflow Ingress

signal deploy.finished {
  service string
}

source webhook as deploys {
  observe as obs
  emit deploy.finished {
    service obs.service
  }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("program compiles");
        assert_eq!(ir.sources.len(), 1);
        let decl = &ir.sources[0];
        assert!(!decl.is_clock);
        assert_eq!(decl.provider, "webhook");
        assert!(decl.recurrence.is_none());
        assert_eq!(decl.emit_signal, "deploy.finished");
    }

    #[test]
    fn complete_field_reads_are_collected_per_field() {
        // The per-field flow-signature metadata (DR-0030 X2 v2) keeps each result
        // field's referenced roots SEPARATE (where `egress_payload_reads` joins them):
        // `id` references only `a`, `note` references only `b`.
        let source = r#"
@tool
workflow Producer {
  input request Req
  output result R
  class Req { id string }
  class A { x string }
  class B { y string }
  class R { id string  note string }

  rule combine
    when A as a
    when B as b
  => {
    complete result {
      id a.x
      note b.y
    }
  }
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.expect("program compiles");
        let rule = ir
            .rules
            .iter()
            .find(|r| r.name == "combine")
            .expect("combine rule");
        let per_field = rule
            .metadata
            .complete_field_reads
            .get("result")
            .expect("result has per-field reads");
        assert_eq!(
            per_field.get("id"),
            Some(&BTreeSet::from(["a".to_owned()])),
            "id references only a: {per_field:?}"
        );
        assert_eq!(
            per_field.get("note"),
            Some(&BTreeSet::from(["b".to_owned()])),
            "note references only b: {per_field:?}"
        );
    }
}
