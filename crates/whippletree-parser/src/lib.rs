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
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    Use(UseDecl),
    Agent(AgentDecl),
    Enum(EnumDecl),
    Class(ClassDecl),
    Coerce(CoerceDecl),
    Assert(AssertDecl),
    Rule(RuleDecl),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssertDecl {
    pub expr: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseDecl {
    pub kind: Ident,
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
    pub uses: Vec<IrUse>,
    pub schemas: Vec<IrSchema>,
    pub agents: Vec<IrAgent>,
    pub coerces: Vec<IrCoerce>,
    pub assertions: Vec<IrAssertion>,
    pub rules: Vec<IrRule>,
    pub rule_dependencies: Vec<IrRuleDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrAssertion {
    pub expr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrUse {
    pub kind: IrUseKind,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrUseKind {
    Skill,
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
    pub whens: Vec<String>,
    pub body: String,
    pub metadata: IrRuleMetadata,
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
    pub fact_writes: Vec<String>,
    pub effects: Vec<IrEffectNode>,
    pub dependencies: Vec<IrEffectDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEffectNode {
    pub id: String,
    pub kind: IrEffectKind,
    pub binding: Option<String>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrEffectKind {
    AgentTell,
    BamlCoerce,
    LoftClaim,
    HumanAsk,
    CapabilityCall,
    EventEmit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrEffectDependency {
    pub upstream: String,
    pub predicate: DependencyPredicate,
    pub downstream: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyPredicate {
    Succeeds,
    Fails,
    Completes,
}

#[derive(Clone, Debug)]
struct SemanticContext {
    schemas: SchemaIndex,
    agents: BTreeSet<String>,
    coerce_outputs: BTreeMap<String, TypeSyntax>,
    coerce_param_counts: BTreeMap<String, usize>,
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
    Record {
        schema: String,
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
pub enum Expr {
    Literal(ExprLiteral),
    Path(Vec<String>),
    Array(Vec<Expr>),
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
    let parsed = parse_program(source);
    if !parsed.diagnostics.is_empty() {
        return CompileOutput {
            ir: None,
            diagnostics: parsed.diagnostics,
        };
    }

    lower_program(parsed.program)
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

impl IrProgram {
    pub fn to_snapshot(&self) -> String {
        let mut snapshot = String::new();
        push_line(&mut snapshot, format!("workflow {}", self.workflow));

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
                push_line(
                    &mut snapshot,
                    format!(
                        "  agent {} profile={} capacity={} skills={}",
                        agent.name, profile, capacity, skills
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
                push_line(&mut snapshot, format!("  assert {}", assertion.expr));
            }
        }

        if !self.rules.is_empty() {
            push_line(&mut snapshot, "rules");
            for rule in &self.rules {
                push_line(&mut snapshot, format!("  rule {}", rule.name));
                for when in &rule.whens {
                    push_line(&mut snapshot, format!("    when {}", when));
                }
                if !rule.metadata.fact_reads.is_empty() {
                    push_line(&mut snapshot, "    reads");
                    for read in &rule.metadata.fact_reads {
                        push_line(&mut snapshot, format!("      {}", read));
                    }
                }
                if !rule.metadata.fact_writes.is_empty() {
                    push_line(&mut snapshot, "    writes");
                    for write in &rule.metadata.fact_writes {
                        push_line(&mut snapshot, format!("      {}", write));
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
            Self::Skill => "skill",
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

fn lower_program(program: Program) -> CompileOutput {
    let mut diagnostics = Vec::new();
    let schema_names = collect_schema_names(&program, &mut diagnostics);
    let agent_names = collect_agent_names(&program, &mut diagnostics);
    let semantic = SemanticContext::from_program(&program);
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
            Item::Use(use_decl) => lower_use(use_decl, &mut ir, &mut diagnostics),
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
            Item::Assert(assertion) => lower_assert(assertion, &mut ir, &mut diagnostics),
            Item::Rule(rule) => lower_rule(rule, &semantic, &mut ir, &mut diagnostics),
        }
    }

    ir.rule_dependencies = build_rule_dependencies(&ir.rules);

    CompileOutput {
        ir: diagnostics.is_empty().then_some(ir),
        diagnostics,
    }
}

fn lower_assert(assertion: AssertDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    if let Err(message) = parse_expression(&assertion.expr) {
        diagnostics.push(Diagnostic {
            span: assertion.span,
            message: format!("invalid assertion expression: {message}"),
            suggestion: Some(
                "use a deterministic expression such as `count(Fact) == 1`".to_owned(),
            ),
        });
    }
    ir.assertions.push(IrAssertion {
        expr: assertion.expr,
    });
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

impl SemanticContext {
    fn from_program(program: &Program) -> Self {
        let mut schemas = SchemaIndex::with_builtins();
        let mut agents = BTreeSet::new();
        let mut coerce_outputs = BTreeMap::new();
        let mut coerce_param_counts = BTreeMap::new();

        for item in &program.items {
            match item {
                Item::Enum(enum_decl) => {
                    schemas.enums.insert(
                        enum_decl.name.name.clone(),
                        enum_decl
                            .variants
                            .iter()
                            .map(|variant| variant.name.clone())
                            .collect(),
                    );
                }
                Item::Class(class_decl) => {
                    schemas.classes.insert(
                        class_decl.name.name.clone(),
                        class_decl
                            .fields
                            .iter()
                            .map(|field| (field.name.name.clone(), field.ty.clone()))
                            .collect(),
                    );
                }
                Item::Agent(agent) => {
                    agents.insert(agent.name.name.clone());
                }
                Item::Coerce(coerce) => {
                    coerce_outputs.insert(coerce.name.name.clone(), coerce.output.clone());
                    coerce_param_counts.insert(coerce.name.name.clone(), coerce.params.len());
                }
                _ => {}
            }
        }

        Self {
            schemas,
            agents,
            coerce_outputs,
            coerce_param_counts,
        }
    }
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

fn lower_use(use_decl: UseDecl, ir: &mut IrProgram, diagnostics: &mut Vec<Diagnostic>) {
    let kind = match use_decl.kind.name.as_str() {
        "skill" => IrUseKind::Skill,
        "plugin" => IrUseKind::Plugin,
        _ => {
            diagnostics.push(Diagnostic {
                span: use_decl.kind.span,
                message: "use declaration kind must be `skill` or `plugin`".to_owned(),
                suggestion: Some("write `use skill \"...\"` or `use plugin \"...\"`".to_owned()),
            });
            return;
        }
    };
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
            AgentField::Unknown { name, .. } => {
                diagnostics.push(Diagnostic {
                    span: name.span,
                    message: format!(
                        "unknown agent field `{}` on agent `{}`",
                        name.name, agent.name.name
                    ),
                    suggestion: Some(
                        "supported agent fields are `profile`, `capacity`, and `skills`".to_owned(),
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
        "AgentTurn" | "WorkItem" | "LoftIssue" | "LoftClaim" | "HumanAnswer" | "Evidence"
    )
}

fn lower_rule(
    rule: RuleDecl,
    semantic: &SemanticContext,
    ir: &mut IrProgram,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let metadata = analyze_rule(&rule, semantic, diagnostics);
    validate_effectful_self_trigger(&rule, &metadata, diagnostics);
    ir.rules.push(IrRule {
        name: rule.name.name,
        whens: rule.whens.into_iter().map(|when| when.text).collect(),
        body: rule.body.text,
        metadata,
    });
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
        if metadata.fact_reads.contains(written_fact) {
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
    for when in &rule.whens {
        if let (_, Some(guard)) = split_when_guard(&when.text) {
            validate_expression(rule, guard, semantic, &binding_types, "guard", diagnostics);
            validate_known_field_paths(rule, guard, semantic, &binding_types, diagnostics);
        }
        validate_availability_when(rule, &when.text, semantic, &binding_types, diagnostics);
    }
    validate_case_blocks(rule, semantic, &binding_types, diagnostics);
    let mut block_stack: Vec<BlockFrame> = Vec::new();
    let mut misplaced_effect_bindings = BTreeSet::new();
    let mut anonymous_effects = 0usize;

    for raw_line in rule.body.text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
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

        if line.starts_with("case ") || is_case_branch_start(line) {
            validate_known_field_paths(rule, line, semantic, &binding_types, diagnostics);
            continue;
        }

        if let Some(record_schema) = active_record_schema(&block_stack) {
            validate_record_field(
                rule,
                line,
                record_schema,
                semantic,
                &binding_types,
                diagnostics,
            );
            continue;
        }

        let active_afters = after_scopes(&block_stack);
        validate_binding_uses(rule, line, &seen_bindings, &active_afters, diagnostics);
        validate_known_field_paths(rule, line, semantic, &binding_types, diagnostics);

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

        if let Some(schema) = parse_record_start(line) {
            if !semantic.schemas.class_exists(&schema) {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!("rule `{}` records unknown class `{schema}`", rule.name.name),
                    suggestion: Some(format!("declare `class {schema}` before recording it")),
                });
            }
            metadata.fact_writes.push(format!("schema:{schema}"));
            block_stack.push(BlockFrame::Record { schema });
            continue;
        }

        if let Some((kind, binding)) = parse_effect_line(line) {
            validate_coerce_call(rule, line, &kind, semantic, diagnostics);
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
                idempotency_key,
            });
        }
    }

    metadata.fact_reads.sort();
    metadata.fact_reads.dedup();
    metadata.fact_writes.sort();
    metadata.fact_writes.dedup();
    metadata
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
    kind: &IrEffectKind,
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if kind != &IrEffectKind::BamlCoerce {
        return;
    }
    let Some(function_name) = parse_coerce_call_name(line) else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has malformed coerce call", rule.name.name),
            suggestion: Some("write `coerce functionName(arg, ...) as name`".to_owned()),
        });
        return;
    };
    let Some(expected) = semantic.coerce_param_counts.get(function_name) else {
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
    if let Some(actual) = parse_coerce_arg_count(line) {
        if actual != *expected {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` calls coerce `{function_name}` with {actual} argument(s), expected {expected}",
                    rule.name.name
                ),
                suggestion: Some("pass one argument for each declared coerce parameter".to_owned()),
            });
        }
    }
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
    if target.contains('.') {
        let Some(ty) = expression_type(target, semantic, binding_types) else {
            return;
        };
        if !matches!(ty, TypeSyntax::AgentRef { .. }) {
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
            let presence_proofs = BTreeSet::new();
            validate_expr_node(
                rule,
                &expr,
                semantic,
                binding_types,
                &presence_proofs,
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

fn validate_expr_node(
    rule: &RuleDecl,
    expr: &Expr,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    presence_proofs: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Path(path) => {
            if path.len() < 2 {
                return;
            }
            let root = &path[0];
            let Some(schema) = binding_types.get(root) else {
                return;
            };
            if let Err(message) =
                validate_optional_path_access(schema, &path[1..], semantic, presence_proofs)
            {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has unsafe optional path `{}`: {message}",
                        rule.name.name,
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
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has invalid expression path `{}`: {message}",
                        rule.name.name,
                        path.join(".")
                    ),
                    suggestion: Some("use a field declared on the bound schema".to_owned()),
                });
            }
        }
        Expr::Array(items) => {
            for item in items {
                validate_expr_node(
                    rule,
                    item,
                    semantic,
                    binding_types,
                    presence_proofs,
                    diagnostics,
                );
            }
        }
        Expr::Unary { expr, .. } => validate_expr_node(
            rule,
            expr,
            semantic,
            binding_types,
            presence_proofs,
            diagnostics,
        ),
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            validate_expr_node(
                rule,
                left,
                semantic,
                binding_types,
                presence_proofs,
                diagnostics,
            );
            let mut right_proofs = presence_proofs.clone();
            collect_presence_proofs(left, &mut right_proofs);
            validate_expr_node(
                rule,
                right,
                semantic,
                binding_types,
                &right_proofs,
                diagnostics,
            );
        }
        Expr::Binary { op, left, right } => {
            validate_expr_node(
                rule,
                left,
                semantic,
                binding_types,
                presence_proofs,
                diagnostics,
            );
            validate_expr_node(
                rule,
                right,
                semantic,
                binding_types,
                presence_proofs,
                diagnostics,
            );
            validate_finite_domain_expr(
                rule,
                *op,
                left,
                right,
                semantic,
                binding_types,
                diagnostics,
            );
        }
        Expr::Call { args, .. } => {
            for arg in args {
                validate_expr_node(
                    rule,
                    arg,
                    semantic,
                    binding_types,
                    presence_proofs,
                    diagnostics,
                );
            }
        }
        Expr::Query { guard, .. } => {
            if let Some(guard) = guard {
                validate_expr_node(
                    rule,
                    guard,
                    semantic,
                    binding_types,
                    presence_proofs,
                    diagnostics,
                );
            }
        }
        Expr::Literal(_) => {}
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
        Expr::Path(path) if path.len() >= 2 => Some(path[1..].join(".")),
        _ => None,
    }
}

fn validate_finite_domain_expr(
    rule: &RuleDecl,
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::In | BinaryOp::NotIn
    ) {
        return;
    }
    let Some(domain) = expr_domain(left, semantic, binding_types) else {
        return;
    };
    let literals = match right {
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
    for literal in literals.into_iter().flatten() {
        if !domain.iter().any(|value| value == &literal) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
                message: format!(
                    "rule `{}` compares finite-domain value to unknown `{literal}`",
                    rule.name.name
                ),
                suggestion: Some(format!("use one of: {}", domain.join(", "))),
            });
        }
    }
}

fn expr_domain(
    expr: &Expr,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
) -> Option<Vec<String>> {
    let Expr::Path(path) = expr else {
        return None;
    };
    let root = path.first()?;
    let schema = binding_types.get(root)?;
    let ty = semantic
        .schemas
        .resolve_field_path(schema, path.get(1..)?)
        .ok()?;
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

fn parse_tell_target(line: &str) -> Option<&str> {
    line.strip_prefix("tell ")?
        .split_whitespace()
        .next()
        .filter(|target| !target.is_empty())
}

fn validate_case_blocks(
    rule: &RuleDecl,
    semantic: &SemanticContext,
    binding_types: &BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lines = rule.body.text.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(scrutinee) = case_scrutinee(trimmed) else {
            index += 1;
            continue;
        };
        let scrutinee_ty = expression_type(scrutinee, semantic, binding_types);
        if scrutinee_ty.is_none() {
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
            let line = lines[case_index].trim();
            if depth == 1 {
                if let Some(branch) = parse_case_branch_head(line) {
                    branches.push(branch);
                    validate_case_pattern(
                        rule,
                        branch.pattern,
                        scrutinee_ty.as_ref(),
                        semantic,
                        diagnostics,
                    );
                    if let Some(guard) = branch.guard {
                        validate_expression(
                            rule,
                            guard,
                            semantic,
                            binding_types,
                            "case guard",
                            diagnostics,
                        );
                        validate_known_field_paths(
                            rule,
                            guard,
                            semantic,
                            binding_types,
                            diagnostics,
                        );
                    }
                }
            }
            depth += brace_delta(line);
            case_index += 1;
        }
        validate_case_coverage(
            rule,
            scrutinee_ty.as_ref(),
            &branches,
            semantic,
            diagnostics,
        );
        index += 1;
    }
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
    semantic: &SemanticContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if matches!(pattern, "_" | "default") {
        return;
    }
    if pattern == "None" {
        if !matches!(scrutinee_ty, Some(TypeSyntax::Optional { .. })) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
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
                span: rule.body.span,
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
                    span: rule.body.span,
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
                    span: rule.body.span,
                    message: format!(
                        "rule `{}` has unsupported case pattern `{pattern}`",
                        rule.name.name
                    ),
                    suggestion: Some("use a literal branch value or `_`".to_owned()),
                });
                return;
            };
            validate_union_case_pattern(rule, variants, &literal, diagnostics);
        }
        TypeSyntax::AgentRef { agents, .. } => {
            let Some(literal) = parse_literal_expr(pattern) else {
                diagnostics.push(Diagnostic {
                    span: rule.body.span,
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
            validate_agent_ref_case_pattern(rule, agents, &literal, diagnostics);
        }
        TypeSyntax::Optional { inner, .. } => {
            validate_case_pattern(rule, pattern, Some(inner), semantic, diagnostics);
        }
        _ => {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
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

fn validate_case_coverage(
    rule: &RuleDecl,
    scrutinee_ty: Option<&TypeSyntax>,
    branches: &[CaseBranchHead<'_>],
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
    branches: &[CaseBranchHead<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for branch in branches.iter().filter(|branch| branch.guard.is_none()) {
        let Some(pattern) = normalized_case_pattern(branch.pattern) else {
            continue;
        };
        if !seen.insert(pattern.to_owned()) {
            diagnostics.push(Diagnostic {
                span: rule.body.span,
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

fn is_fallback_pattern(pattern: &str) -> bool {
    matches!(pattern, "_" | "default")
}

fn validate_union_case_pattern(
    rule: &RuleDecl,
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
            span: rule.body.span,
            message: format!("rule `{}` case pattern cannot be `{value}`", rule.name.name),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
    }
}

fn validate_agent_ref_case_pattern(
    rule: &RuleDecl,
    agents: &[Ident],
    literal: &LiteralExpr<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<Vec<_>>();
    let (LiteralExpr::String(value) | LiteralExpr::Ident(value)) = literal else {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("rule `{}` has non-agent case pattern", rule.name.name),
            suggestion: Some(format!("use one of: {}", allowed.join(", "))),
        });
        return;
    };
    if !allowed.contains(value) {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
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
            BlockFrame::Record { .. } => None,
        })
        .collect()
}

fn active_record_schema(block_stack: &[BlockFrame]) -> Option<&str> {
    match block_stack.last() {
        Some(BlockFrame::Record { schema }) => Some(schema),
        _ => None,
    }
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
        IrEffectKind::AgentTell | IrEffectKind::CapabilityCall | IrEffectKind::EventEmit => None,
    }
}

fn parse_coerce_call_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("coerce ")?;
    rest.split_once('(').map(|(name, _)| name.trim())
}

fn parse_coerce_arg_count(line: &str) -> Option<usize> {
    let open = line.find('(')?;
    let after_open = &line[open + 1..];
    let close = after_open.find(')')?;
    let args = after_open[..close].trim();
    if args.is_empty() {
        Some(0)
    } else {
        Some(args.split(',').count())
    }
}

fn validate_known_field_paths(
    rule: &RuleDecl,
    line: &str,
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

fn parse_record_start(line: &str) -> Option<String> {
    line.strip_prefix("record ")
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
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
}

fn record_field_assignment(line: &str) -> Option<(&str, &str)> {
    let field_end = line.find(char::is_whitespace)?;
    let field = &line[..field_end];
    let expr = line[field_end..].trim();
    (!field.is_empty() && !expr.is_empty()).then_some((field, expr))
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
    let (LiteralExpr::String(value) | LiteralExpr::Ident(value)) = literal else {
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
        self.parse_primary()
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
                let arg = match self.parse_primary()? {
                    Expr::Literal(ExprLiteral::Ident(path)) => Expr::Path(vec![path]),
                    expr => expr,
                };
                Ok(Expr::Call {
                    name: value,
                    args: vec![arg],
                })
            }
            Some(ExprTokenKind::Ident(value))
                if matches!(value.as_str(), "count" | "exists" | "empty")
                    && self.at_symbol('(') =>
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
    );
    if !valid {
        diagnostics.push(Diagnostic {
            span: rule.body.span,
            message: format!("field `{record_schema}.{field}` expects `{primitive}`"),
            suggestion: Some(format!("record a value compatible with `{primitive}`")),
        });
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
    } else {
        return None;
    };

    Some((kind, binding_after_as(line)))
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
    let mut parts = rest.split_whitespace();
    let binding = parts.next()?.to_owned();
    let predicate = match parts.next()? {
        "succeeds" => DependencyPredicate::Succeeds,
        "fails" => DependencyPredicate::Fails,
        "completes" => DependencyPredicate::Completes,
        _ => return None,
    };
    Some((binding, predicate))
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

fn format_syntax(program: Program) -> String {
    let mut formatted = String::new();
    if let Some(workflow) = program.workflow {
        push_line(&mut formatted, format!("workflow {}", workflow.name));
        formatted.push('\n');
    }

    let item_count = program.items.len();
    for (index, item) in program.items.into_iter().enumerate() {
        match item {
            Item::Use(use_decl) => {
                push_line(
                    &mut formatted,
                    format!("use {} {:?}", use_decl.kind.name, use_decl.name.value),
                );
            }
            Item::Agent(agent) => format_agent(agent, &mut formatted),
            Item::Enum(enum_decl) => format_enum(enum_decl, &mut formatted),
            Item::Class(class_decl) => format_class(class_decl, &mut formatted),
            Item::Coerce(coerce) => format_coerce(coerce, &mut formatted),
            Item::Assert(assertion) => {
                push_line(&mut formatted, format!("assert {}", assertion.expr))
            }
            Item::Rule(rule) => format_rule(rule, &mut formatted),
        }

        if index + 1 < item_count {
            formatted.push('\n');
        }
    }

    formatted
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
    whippletree_core::IMPLEMENTATION_STAGE
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

        if b"{}[]()<>,?|.+!".contains(&byte) {
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

impl Parser<'_> {
    fn parse_program(&mut self) -> Program {
        let mut workflow = None;
        let mut items = Vec::new();

        while !self.is_at_end() {
            if self.at_ident("workflow") {
                self.advance();
                workflow = self.expect_ident("workflow name");
            } else if self.at_ident("use") {
                if let Some(item) = self.parse_use() {
                    items.push(Item::Use(item));
                }
            } else if self.at_ident("agent") {
                if let Some(item) = self.parse_agent() {
                    items.push(Item::Agent(item));
                }
            } else if self.at_ident("enum") {
                if let Some(item) = self.parse_enum() {
                    items.push(Item::Enum(item));
                }
            } else if self.at_ident("class") {
                if let Some(item) = self.parse_class() {
                    items.push(Item::Class(item));
                }
            } else if self.at_ident("coerce") {
                if let Some(item) = self.parse_coerce() {
                    items.push(Item::Coerce(item));
                }
            } else if self.at_ident("assert") {
                if let Some(item) = self.parse_assert() {
                    items.push(Item::Assert(item));
                }
            } else if self.at_ident("rule") {
                if let Some(item) = self.parse_rule() {
                    items.push(Item::Rule(item));
                }
            } else {
                self.unexpected("top-level declaration");
                self.advance();
            }
        }

        Program { workflow, items }
    }

    fn parse_use(&mut self) -> Option<UseDecl> {
        self.expect_keyword("use")?;
        let kind = self.expect_ident("use declaration kind")?;
        if kind.name != "skill" && kind.name != "plugin" {
            self.diagnostics.push(Diagnostic {
                span: kind.span,
                message: "use declaration kind must be `skill` or `plugin`".to_owned(),
                suggestion: Some("write `use skill \"...\"` or `use plugin \"...\"`".to_owned()),
            });
        }
        Some(UseDecl {
            kind,
            name: self.expect_string("skill name")?,
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
        Some(AssertDecl {
            expr: self.source_text(span).trim().to_owned(),
            span,
        })
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
        Some(WhenClause {
            text: self.source_text(span).trim().to_owned(),
            span,
        })
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
        "skill name" => Some("write a quoted name, such as `\"loft-user\"`".to_owned()),
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

use skill "loft-user"
use plugin "memory"

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
        assert_eq!(rule.whens[0].text, "loft has ready issue as issue");
        assert!(rule.body.text.contains("after claim succeeds"));
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
}

agent claude {
  profile "repo-writer"
  capacity 1
}

class LanguageTask {
  provider AgentRef<codex | claude>
  prompt string
}

rule run_task
  when LanguageTask as task
  when task.provider is available
=> {
  tell task.provider as turn "{{ task.prompt }}"
}
"#;

        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new());
        assert!(compiled.ir.is_some());
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
    provider "claude"
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
    fn lowers_deterministic_ir_snapshot() {
        let source = r#"
workflow Snapshot

use skill "loft-user"

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
uses
  skill loft-user
schemas
  class Work
    title string
    files array<string>
    state union<literal<\"open\"> | literal<\"done\">>
agents
  agent worker profile=repo-writer capacity=2 skills=[loft-user]
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
        assert!(ir.rules.iter().any(|rule| rule
            .whens
            .iter()
            .any(|when| when == "WorkItem as item where item.state == \"ready\"")));
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
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("enum `ReviewStatus` has no variant `Missing`")));
        assert!(compiled.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("uses `Some` for a non-optional case")));
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
    fn formats_top_level_syntax_scaffold() {
        let source = r#"workflow Messy
use skill "loft-user"
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
            "use skill \"loft-user\"\n",
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
}
