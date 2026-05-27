//! Workflow language layer for `.armature` files.
//!
//! This crate is intentionally side-effect free. It owns source parsing,
//! normalized IR types, pure expression structures, schemas, diagnostics, and
//! static validation. Runtime execution belongs in `armature-engine`.

pub mod diagnostics {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SourceSpan {
        pub file: String,
        pub start_line: u32,
        pub start_column: u32,
        pub end_line: u32,
        pub end_column: u32,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Severity {
        Error,
        Warning,
        Note,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Diagnostic {
        pub severity: Severity,
        pub message: String,
        pub span: Option<SourceSpan>,
    }
}

pub mod schema {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum Schema {
        String,
        Int,
        Float,
        Boolean,
        Null,
        Time,
        Duration,
        Agent,
        Literal {
            value: serde_json::Value,
        },
        Enum {
            values: Vec<String>,
        },
        Optional {
            inner: Box<Schema>,
        },
        List {
            inner: Box<Schema>,
        },
        Set {
            inner: Box<Schema>,
        },
        Map {
            key: Box<Schema>,
            value: Box<Schema>,
        },
        Union {
            variants: Vec<Schema>,
        },
        Record {
            fields: Vec<Field>,
        },
        Ref {
            name: String,
        },
        Json,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Field {
        pub name: String,
        pub schema: Schema,
    }

    impl Schema {
        pub fn accepts_json(&self, value: &serde_json::Value) -> bool {
            self.accepts_json_inner(value, &BTreeMap::new(), 0)
        }

        pub fn accepts_json_with_types(
            &self,
            value: &serde_json::Value,
            types: &BTreeMap<String, Schema>,
        ) -> bool {
            self.accepts_json_inner(value, types, 0)
        }

        pub fn explain_json_mismatch(
            &self,
            value: &serde_json::Value,
            types: &BTreeMap<String, Schema>,
        ) -> Option<String> {
            self.explain_json_mismatch_inner(value, types, "$", 0)
        }

        fn accepts_json_inner(
            &self,
            value: &serde_json::Value,
            types: &BTreeMap<String, Schema>,
            depth: usize,
        ) -> bool {
            if depth > 16 {
                return false;
            }

            match self {
                Schema::String | Schema::Time | Schema::Duration | Schema::Agent => {
                    value.is_string()
                }
                Schema::Int => value.as_i64().is_some() || value.as_u64().is_some(),
                Schema::Float => value.is_number(),
                Schema::Boolean => value.is_boolean(),
                Schema::Null => value.is_null(),
                Schema::Literal { value: expected } => value == expected,
                Schema::Enum { values } => value
                    .as_str()
                    .is_some_and(|actual| values.iter().any(|expected| expected == actual)),
                Schema::Optional { inner } => {
                    value.is_null() || inner.accepts_json_inner(value, types, depth + 1)
                }
                Schema::List { inner } | Schema::Set { inner } => {
                    value.as_array().is_some_and(|items| {
                        items
                            .iter()
                            .all(|item| inner.accepts_json_inner(item, types, depth + 1))
                    })
                }
                Schema::Map {
                    key: key_schema,
                    value: value_schema,
                } => value.as_object().is_some_and(|entries| {
                    entries.iter().all(|(key, value)| {
                        let key = serde_json::Value::String(key.clone());
                        key_schema.accepts_json_inner(&key, types, depth + 1)
                            && value_schema.accepts_json_inner(value, types, depth + 1)
                    })
                }),
                Schema::Union { variants } => variants
                    .iter()
                    .any(|variant| variant.accepts_json_inner(value, types, depth + 1)),
                Schema::Record { fields } => value.as_object().is_some_and(|object| {
                    object
                        .keys()
                        .all(|key| fields.iter().any(|field| field.name == *key))
                        && fields.iter().all(|field| {
                            object.get(&field.name).map_or_else(
                                || matches!(field.schema, Schema::Optional { .. }),
                                |value| field.schema.accepts_json_inner(value, types, depth + 1),
                            )
                        })
                }),
                Schema::Ref { name } => types
                    .get(name)
                    .is_some_and(|schema| schema.accepts_json_inner(value, types, depth + 1)),
                Schema::Json => true,
            }
        }

        fn explain_json_mismatch_inner(
            &self,
            value: &serde_json::Value,
            types: &BTreeMap<String, Schema>,
            path: &str,
            depth: usize,
        ) -> Option<String> {
            if self.accepts_json_inner(value, types, depth) {
                return None;
            }
            if depth > 16 {
                return Some(format!("{path} exceeds maximum schema nesting depth"));
            }

            match self {
                Schema::Record { fields } => {
                    let Some(object) = value.as_object() else {
                        return Some(format!(
                            "{path} expected record/object, got {}",
                            json_kind(value)
                        ));
                    };
                    for key in object.keys() {
                        if !fields.iter().any(|field| field.name == *key) {
                            return Some(format!("{path}.{key} is not declared in schema"));
                        }
                    }
                    for field in fields {
                        let field_path = format!("{path}.{}", field.name);
                        match object.get(&field.name) {
                            Some(value) => {
                                if let Some(reason) = field.schema.explain_json_mismatch_inner(
                                    value,
                                    types,
                                    &field_path,
                                    depth + 1,
                                ) {
                                    return Some(reason);
                                }
                            }
                            None if !matches!(field.schema, Schema::Optional { .. }) => {
                                return Some(format!("{field_path} is required"));
                            }
                            None => {}
                        }
                    }
                    Some(format!(
                        "{path} does not match record schema; got {}",
                        json_kind(value)
                    ))
                }
                Schema::List { inner } | Schema::Set { inner } => {
                    let Some(items) = value.as_array() else {
                        return Some(format!(
                            "{path} expected list/array, got {}",
                            json_kind(value)
                        ));
                    };
                    for (index, item) in items.iter().enumerate() {
                        let item_path = format!("{path}[{index}]");
                        if let Some(reason) =
                            inner.explain_json_mismatch_inner(item, types, &item_path, depth + 1)
                        {
                            return Some(reason);
                        }
                    }
                    Some(format!("{path} does not match list schema"))
                }
                Schema::Map {
                    key,
                    value: value_schema,
                } => {
                    let Some(entries) = value.as_object() else {
                        return Some(format!(
                            "{path} expected map/object, got {}",
                            json_kind(value)
                        ));
                    };
                    for (entry_key, entry_value) in entries {
                        let key_value = serde_json::Value::String(entry_key.clone());
                        if let Some(reason) = key.explain_json_mismatch_inner(
                            &key_value,
                            types,
                            &format!("{path}.<key:{entry_key}>"),
                            depth + 1,
                        ) {
                            return Some(reason);
                        }
                        if let Some(reason) = value_schema.explain_json_mismatch_inner(
                            entry_value,
                            types,
                            &format!("{path}.{entry_key}"),
                            depth + 1,
                        ) {
                            return Some(reason);
                        }
                    }
                    Some(format!("{path} does not match map schema"))
                }
                Schema::Optional { inner } => {
                    if value.is_null() {
                        None
                    } else {
                        inner.explain_json_mismatch_inner(value, types, path, depth + 1)
                    }
                }
                Schema::Union { variants } => Some(format!(
                    "{path} expected one of {}, got {}",
                    variants
                        .iter()
                        .map(schema_name)
                        .collect::<Vec<_>>()
                        .join(" | "),
                    json_kind(value)
                )),
                Schema::Ref { name } => types
                    .get(name)
                    .map(|schema| schema.explain_json_mismatch_inner(value, types, path, depth + 1))
                    .unwrap_or_else(|| Some(format!("{path} references unknown schema `{name}`"))),
                _ => Some(format!(
                    "{path} expected {}, got {}",
                    schema_name(self),
                    json_kind(value)
                )),
            }
        }
    }

    fn json_kind(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "bool",
            serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "int",
            serde_json::Value::Number(_) => "float",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }

    fn schema_name(schema: &Schema) -> String {
        match schema {
            Schema::String => "string".to_string(),
            Schema::Int => "int".to_string(),
            Schema::Float => "float".to_string(),
            Schema::Boolean => "bool".to_string(),
            Schema::Null => "null".to_string(),
            Schema::Time => "time".to_string(),
            Schema::Duration => "duration".to_string(),
            Schema::Agent => "agent".to_string(),
            Schema::Literal { value } => format!("literal {value}"),
            Schema::Enum { values } => values.join(" | "),
            Schema::Optional { inner } => format!("{}?", schema_name(inner)),
            Schema::List { inner } => format!("{}[]", schema_name(inner)),
            Schema::Set { inner } => format!("set<{}>", schema_name(inner)),
            Schema::Map { key, value } => {
                format!("map<{}, {}>", schema_name(key), schema_name(value))
            }
            Schema::Union { variants } => variants
                .iter()
                .map(schema_name)
                .collect::<Vec<_>>()
                .join(" | "),
            Schema::Record { .. } => "record/object".to_string(),
            Schema::Ref { name } => name.clone(),
            Schema::Json => "json".to_string(),
        }
    }
}

pub mod expr {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    pub enum Expr {
        Literal { value: serde_json::Value },
        Path { path: String },
        Eq { left: Box<Expr>, right: Box<Expr> },
        Neq { left: Box<Expr>, right: Box<Expr> },
        Lt { left: Box<Expr>, right: Box<Expr> },
        Lte { left: Box<Expr>, right: Box<Expr> },
        Gt { left: Box<Expr>, right: Box<Expr> },
        Gte { left: Box<Expr>, right: Box<Expr> },
        In { left: Box<Expr>, right: Box<Expr> },
        And { exprs: Vec<Expr> },
        Or { exprs: Vec<Expr> },
        Not { expr: Box<Expr> },
        Call { name: String, args: Vec<Expr> },
        Object { fields: BTreeMap<String, Expr> },
        List { items: Vec<Expr> },
    }
}

pub mod ir {
    use super::diagnostics::SourceSpan;
    use super::expr::Expr;
    use super::schema::Schema;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub const SCHEMA_VERSION: &str = "statechart-workflow-ir/v0";

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WorkflowIr {
        pub schema_version: String,
        pub workflow: WorkflowMetadata,
        #[serde(default)]
        pub agents: BTreeMap<String, Agent>,
        #[serde(default)]
        pub events: BTreeMap<String, Event>,
        #[serde(default)]
        pub capabilities: BTreeMap<String, Capability>,
        #[serde(default)]
        pub context_schema: BTreeMap<String, Schema>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub context_initializers: BTreeMap<String, Expr>,
        #[serde(default)]
        pub types: BTreeMap<String, Schema>,
        #[serde(default)]
        pub coerce_functions: BTreeMap<String, CoerceFunction>,
        pub statechart: Statechart,
        #[serde(default)]
        pub invariants: Vec<Invariant>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct WorkflowMetadata {
        pub name: String,
        pub source_path: Option<String>,
        pub repo: Option<String>,
        #[serde(default)]
        pub contracts: Vec<String>,
        pub plan: Option<String>,
        pub state_scope: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Agent {
        pub target: AgentTarget,
        #[serde(default)]
        pub profile: Option<String>,
        pub max_active: Option<u32>,
        #[serde(default)]
        pub capabilities: Vec<String>,
        #[serde(default)]
        pub owns: Vec<String>,
        pub contract: Option<String>,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum AgentTarget {
        Thread { name: String },
        CodingAgent,
        Adapter { name: String },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Capability {
        pub adapter: String,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Event {
        pub payload: Schema,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CoerceFunction {
        pub params: Vec<CoerceParam>,
        pub output: Schema,
        pub model: Option<String>,
        pub prompt: Option<String>,
        pub generated_baml_artifact: Option<String>,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CoerceParam {
        pub name: String,
        pub schema: Schema,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Statechart {
        pub initial: String,
        #[serde(default)]
        pub states: BTreeMap<String, State>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct State {
        pub initial: Option<String>,
        #[serde(default)]
        pub on: Vec<EventHandler>,
        #[serde(default)]
        pub entry: Vec<Step>,
        #[serde(default)]
        pub always: Vec<AlwaysTransition>,
        #[serde(default)]
        pub states: BTreeMap<String, State>,
        #[serde(default)]
        pub final_state: bool,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct EventHandler {
        pub event: String,
        pub binding: Option<String>,
        pub guard: Option<Expr>,
        #[serde(default)]
        pub steps: Vec<Step>,
        pub transition: Option<String>,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AlwaysTransition {
        pub guard: Option<Expr>,
        #[serde(default)]
        pub steps: Vec<Step>,
        pub transition: String,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Step {
        pub effect: String,
        #[serde(default)]
        pub args: BTreeMap<String, serde_json::Value>,
        pub assign: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub case_arms: Vec<CaseArm>,
        pub span: Option<SourceSpan>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CaseArm {
        pub pattern: CasePattern,
        #[serde(default)]
        pub steps: Vec<Step>,
        pub transition: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum CasePattern {
        Identifier { name: String },
        Matches { pattern: String },
        Wildcard,
        Literal { value: serde_json::Value },
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum Invariant {
        Builtin {
            name: String,
            span: Option<SourceSpan>,
        },
        Expression {
            name: String,
            expr: Expr,
            span: Option<SourceSpan>,
        },
    }
}

pub mod policy {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PolicyMode {
        Local,
        Team,
        Enterprise,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PolicyOutcome {
        Allow,
        AllowWithWarning,
        Deny,
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PolicyDecision {
        pub outcome: PolicyOutcome,
        pub required_capabilities: Vec<String>,
        pub layers: Vec<PolicyLayerDecision>,
        pub diagnostics: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PolicyLayerDecision {
        pub layer: String,
        pub outcome: PolicyOutcome,
        pub reason: Option<String>,
    }
}

pub mod syntax {
    use logos::Logos;
    use rowan::{GreenNode, GreenNodeBuilder, Language};
    use text_size::{TextRange, TextSize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArmatureLanguage {}

    impl Language for ArmatureLanguage {
        type Kind = SyntaxKind;

        fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
            assert!(raw.0 <= SyntaxKind::Error as u16);
            // SAFETY: SyntaxKind is repr(u16), and the assert keeps raw values
            // within the declared variant range.
            unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
        }

        fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
            rowan::SyntaxKind(kind as u16)
        }
    }

    pub type SyntaxNode = rowan::SyntaxNode<ArmatureLanguage>;

    #[repr(u16)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum SyntaxKind {
        File,
        MachineDecl,
        InitialDecl,
        DataBlock,
        AgentDecl,
        CapabilityDecl,
        EnumDecl,
        ClassDecl,
        CoerceDecl,
        CoerceMember,
        EventDecl,
        StateDecl,
        OnBlock,
        AlwaysBlock,
        EntryBlock,
        ActionBlock,
        AssignStmt,
        GotoStmt,
        FinalMarker,
        InvariantDecl,
        FieldDecl,
        TypeExpr,
        Expr,
        Path,
        MachineKw,
        InitialKw,
        DataKw,
        AgentKw,
        CapabilityKw,
        AdapterKw,
        EnumKw,
        ClassKw,
        CoerceKw,
        ModelKw,
        PromptKw,
        EventKw,
        StateKw,
        OnKw,
        AsKw,
        GuardKw,
        InKw,
        TrueKw,
        FalseKw,
        AlwaysKw,
        EntryKw,
        StayKw,
        LetKw,
        StartKw,
        SendKw,
        AskHumanKw,
        RaiseKw,
        CaseKw,
        AssignKw,
        GotoKw,
        FinalKw,
        InvariantKw,
        NilKw,
        Ident,
        String,
        BlockString,
        Duration,
        Int,
        Float,
        LBrace,
        RBrace,
        LParen,
        RParen,
        LBracket,
        RBracket,
        Dot,
        Comma,
        Question,
        Equals,
        EqEq,
        NotEq,
        Bang,
        Lt,
        LtEq,
        Gt,
        GtEq,
        AndAnd,
        OrOr,
        Pipe,
        Arrow,
        Whitespace,
        Newline,
        Comment,
        Error,
    }

    #[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
    enum LexKind {
        #[token("machine")]
        MachineKw,
        #[token("initial")]
        InitialKw,
        #[token("data")]
        DataKw,
        #[token("agent")]
        AgentKw,
        #[token("capability")]
        CapabilityKw,
        #[token("adapter")]
        AdapterKw,
        #[token("enum")]
        EnumKw,
        #[token("class")]
        ClassKw,
        #[token("coerce")]
        CoerceKw,
        #[token("model")]
        ModelKw,
        #[token("prompt")]
        PromptKw,
        #[token("event")]
        EventKw,
        #[token("state")]
        StateKw,
        #[token("on")]
        OnKw,
        #[token("as")]
        AsKw,
        #[token("guard")]
        GuardKw,
        #[token("in")]
        InKw,
        #[token("true")]
        TrueKw,
        #[token("false")]
        FalseKw,
        #[token("always")]
        AlwaysKw,
        #[token("entry")]
        EntryKw,
        #[token("stay")]
        StayKw,
        #[token("let")]
        LetKw,
        #[token("start")]
        StartKw,
        #[token("send")]
        SendKw,
        #[token("askHuman")]
        AskHumanKw,
        #[token("raise")]
        RaiseKw,
        #[token("case")]
        CaseKw,
        #[token("assign")]
        AssignKw,
        #[token("goto")]
        GotoKw,
        #[token("final")]
        FinalKw,
        #[token("invariant")]
        InvariantKw,
        #[token("nil")]
        NilKw,
        #[regex(r#""{3}([^"]|"[^"]|""[^"])*"{3}"#)]
        BlockString,
        #[regex(r#""([^"\\]|\\.)*""#)]
        String,
        #[regex(r"[0-9]+(ms|s|m|h|d)")]
        Duration,
        #[regex(r"[0-9]+\.[0-9]+")]
        Float,
        #[regex(r"[0-9]+")]
        Int,
        #[regex(r"[A-Za-z_][A-Za-z0-9_-]*")]
        Ident,
        #[token("{")]
        LBrace,
        #[token("}")]
        RBrace,
        #[token("(")]
        LParen,
        #[token(")")]
        RParen,
        #[token("[")]
        LBracket,
        #[token("]")]
        RBracket,
        #[token(".")]
        Dot,
        #[token(",")]
        Comma,
        #[token("?")]
        Question,
        #[token("==")]
        EqEq,
        #[token("!=")]
        NotEq,
        #[token("!")]
        Bang,
        #[token("<=")]
        LtEq,
        #[token("<")]
        Lt,
        #[token(">=")]
        GtEq,
        #[token(">")]
        Gt,
        #[token("&&")]
        AndAnd,
        #[token("||")]
        OrOr,
        #[token("|")]
        Pipe,
        #[token("=")]
        Equals,
        #[token("->")]
        Arrow,
        #[regex(r"[ \t\f]+")]
        Whitespace,
        #[regex(r"\r\n|\n|\r")]
        Newline,
        #[regex(r"//[^\n\r]*")]
        Comment,
    }

    impl From<LexKind> for SyntaxKind {
        fn from(kind: LexKind) -> Self {
            match kind {
                LexKind::MachineKw => SyntaxKind::MachineKw,
                LexKind::InitialKw => SyntaxKind::InitialKw,
                LexKind::DataKw => SyntaxKind::DataKw,
                LexKind::AgentKw => SyntaxKind::AgentKw,
                LexKind::CapabilityKw => SyntaxKind::CapabilityKw,
                LexKind::AdapterKw => SyntaxKind::AdapterKw,
                LexKind::EnumKw => SyntaxKind::EnumKw,
                LexKind::ClassKw => SyntaxKind::ClassKw,
                LexKind::CoerceKw => SyntaxKind::CoerceKw,
                LexKind::ModelKw => SyntaxKind::ModelKw,
                LexKind::PromptKw => SyntaxKind::PromptKw,
                LexKind::EventKw => SyntaxKind::EventKw,
                LexKind::StateKw => SyntaxKind::StateKw,
                LexKind::OnKw => SyntaxKind::OnKw,
                LexKind::AsKw => SyntaxKind::AsKw,
                LexKind::GuardKw => SyntaxKind::GuardKw,
                LexKind::InKw => SyntaxKind::InKw,
                LexKind::TrueKw => SyntaxKind::TrueKw,
                LexKind::FalseKw => SyntaxKind::FalseKw,
                LexKind::AlwaysKw => SyntaxKind::AlwaysKw,
                LexKind::EntryKw => SyntaxKind::EntryKw,
                LexKind::StayKw => SyntaxKind::StayKw,
                LexKind::LetKw => SyntaxKind::LetKw,
                LexKind::StartKw => SyntaxKind::StartKw,
                LexKind::SendKw => SyntaxKind::SendKw,
                LexKind::AskHumanKw => SyntaxKind::AskHumanKw,
                LexKind::RaiseKw => SyntaxKind::RaiseKw,
                LexKind::CaseKw => SyntaxKind::CaseKw,
                LexKind::AssignKw => SyntaxKind::AssignKw,
                LexKind::GotoKw => SyntaxKind::GotoKw,
                LexKind::FinalKw => SyntaxKind::FinalKw,
                LexKind::InvariantKw => SyntaxKind::InvariantKw,
                LexKind::NilKw => SyntaxKind::NilKw,
                LexKind::Ident => SyntaxKind::Ident,
                LexKind::String => SyntaxKind::String,
                LexKind::BlockString => SyntaxKind::BlockString,
                LexKind::Duration => SyntaxKind::Duration,
                LexKind::Int => SyntaxKind::Int,
                LexKind::Float => SyntaxKind::Float,
                LexKind::LBrace => SyntaxKind::LBrace,
                LexKind::RBrace => SyntaxKind::RBrace,
                LexKind::LParen => SyntaxKind::LParen,
                LexKind::RParen => SyntaxKind::RParen,
                LexKind::LBracket => SyntaxKind::LBracket,
                LexKind::RBracket => SyntaxKind::RBracket,
                LexKind::Dot => SyntaxKind::Dot,
                LexKind::Comma => SyntaxKind::Comma,
                LexKind::Question => SyntaxKind::Question,
                LexKind::EqEq => SyntaxKind::EqEq,
                LexKind::NotEq => SyntaxKind::NotEq,
                LexKind::Bang => SyntaxKind::Bang,
                LexKind::Lt => SyntaxKind::Lt,
                LexKind::LtEq => SyntaxKind::LtEq,
                LexKind::Gt => SyntaxKind::Gt,
                LexKind::GtEq => SyntaxKind::GtEq,
                LexKind::AndAnd => SyntaxKind::AndAnd,
                LexKind::OrOr => SyntaxKind::OrOr,
                LexKind::Pipe => SyntaxKind::Pipe,
                LexKind::Equals => SyntaxKind::Equals,
                LexKind::Arrow => SyntaxKind::Arrow,
                LexKind::Whitespace => SyntaxKind::Whitespace,
                LexKind::Newline => SyntaxKind::Newline,
                LexKind::Comment => SyntaxKind::Comment,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Token {
        pub kind: SyntaxKind,
        pub text: String,
        pub range: TextRange,
    }

    impl Token {
        fn is_trivia(&self) -> bool {
            matches!(
                self.kind,
                SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::Comment
            )
        }
    }

    #[derive(Debug, Clone)]
    pub struct ParsedSyntax {
        pub green: GreenNode,
        pub tokens: Vec<Token>,
    }

    pub fn lex(source: &str) -> Vec<Token> {
        let mut lexer = LexKind::lexer(source);
        let mut tokens = Vec::new();

        while let Some(result) = lexer.next() {
            let span = lexer.span();
            let kind = result.map_or(SyntaxKind::Error, SyntaxKind::from);
            tokens.push(Token {
                kind,
                text: source[span.clone()].to_string(),
                range: TextRange::new(
                    TextSize::from(span.start as u32),
                    TextSize::from(span.end as u32),
                ),
            });
        }

        tokens
    }

    pub(crate) struct SyntaxBuilder {
        tokens: Vec<Token>,
        cursor: usize,
        emit_cursor: usize,
        builder: GreenNodeBuilder<'static>,
    }

    impl SyntaxBuilder {
        pub(crate) fn new(tokens: Vec<Token>) -> Self {
            Self {
                tokens,
                cursor: 0,
                emit_cursor: 0,
                builder: GreenNodeBuilder::new(),
            }
        }

        pub(crate) fn start_node(&mut self, kind: SyntaxKind) {
            self.builder.start_node(ArmatureLanguage::kind_to_raw(kind));
        }

        pub(crate) fn finish_node(&mut self) {
            self.builder.finish_node();
        }

        pub(crate) fn peek(&self) -> Option<SyntaxKind> {
            self.peek_token().map(|token| token.kind)
        }

        pub(crate) fn peek_nth(&self, nth: usize) -> Option<SyntaxKind> {
            let mut seen = 0usize;
            let mut index = self.cursor;
            while let Some(token) = self.tokens.get(index) {
                if !token.is_trivia() {
                    if seen == nth {
                        return Some(token.kind);
                    }
                    seen += 1;
                }
                index += 1;
            }
            None
        }

        pub(crate) fn bump(&mut self) -> Option<Token> {
            let index = self.next_non_trivia_index()?;
            self.emit_through(index);
            self.cursor = index + 1;
            Some(self.tokens[index].clone())
        }

        pub(crate) fn eat(&mut self, kind: SyntaxKind) -> Option<Token> {
            if self.peek() == Some(kind) {
                self.bump()
            } else {
                None
            }
        }

        pub(crate) fn at_end(&self) -> bool {
            self.peek().is_none()
        }

        pub(crate) fn finish(self) -> (GreenNode, Vec<Token>) {
            (self.builder.finish(), self.tokens)
        }

        pub(crate) fn peek_token(&self) -> Option<&Token> {
            self.next_non_trivia_index()
                .and_then(|index| self.tokens.get(index))
        }

        fn next_non_trivia_index(&self) -> Option<usize> {
            let mut index = self.cursor;
            while let Some(token) = self.tokens.get(index) {
                if !token.is_trivia() {
                    return Some(index);
                }
                index += 1;
            }
            None
        }

        fn emit_through(&mut self, index: usize) {
            while self.emit_cursor <= index {
                let token = &self.tokens[self.emit_cursor];
                self.builder.token(
                    ArmatureLanguage::kind_to_raw(token.kind),
                    token.text.as_str(),
                );
                self.emit_cursor += 1;
            }
        }

        pub(crate) fn emit_remaining(&mut self) {
            while self.emit_cursor < self.tokens.len() {
                let token = &self.tokens[self.emit_cursor];
                self.builder.token(
                    ArmatureLanguage::kind_to_raw(token.kind),
                    token.text.as_str(),
                );
                self.emit_cursor += 1;
            }
        }
    }

    pub fn syntax_node(green: GreenNode) -> SyntaxNode {
        SyntaxNode::new_root(green)
    }
}

pub mod source {
    use crate::diagnostics::{Diagnostic, Severity, SourceSpan};
    use crate::expr::Expr;
    use crate::ir::{
        Agent, AgentTarget, AlwaysTransition, Capability, CaseArm, CasePattern, CoerceFunction,
        CoerceParam, Event, EventHandler, Invariant, State, Statechart, Step, WorkflowIr,
        WorkflowMetadata, SCHEMA_VERSION,
    };
    use crate::schema::{Field, Schema};
    use crate::syntax::{self, ParsedSyntax, SyntaxBuilder, SyntaxKind};
    use std::collections::{btree_map::Entry, BTreeMap};
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum SourceError {
        #[error("failed to parse Armature source")]
        Parse { diagnostics: Vec<Diagnostic> },
    }

    #[derive(Debug)]
    pub struct ParsedSource {
        pub syntax: ParsedSyntax,
        pub ir: Option<WorkflowIr>,
        pub diagnostics: Vec<Diagnostic>,
    }

    pub fn parse_syntax(source: &str) -> ParsedSource {
        parse_syntax_with_file(source, "<source>")
    }

    pub fn parse_syntax_with_file(source: &str, file: impl Into<String>) -> ParsedSource {
        Parser::new(source, file.into()).parse()
    }

    pub fn parse_source(source: &str) -> Result<WorkflowIr, SourceError> {
        let parsed = parse_syntax(source);
        source_from_parsed(parsed)
    }

    pub fn parse_source_with_file(
        source: &str,
        file: impl Into<String>,
    ) -> Result<WorkflowIr, SourceError> {
        let parsed = parse_syntax_with_file(source, file);
        source_from_parsed(parsed)
    }

    fn source_from_parsed(parsed: ParsedSource) -> Result<WorkflowIr, SourceError> {
        if parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            return Err(SourceError::Parse {
                diagnostics: parsed.diagnostics,
            });
        }

        parsed.ir.ok_or_else(|| SourceError::Parse {
            diagnostics: vec![error("source did not produce workflow IR")],
        })
    }

    struct Parser {
        syntax: SyntaxBuilder,
        diagnostics: Vec<Diagnostic>,
        machine_name: Option<String>,
        initial: Option<String>,
        agents: BTreeMap<String, Agent>,
        capabilities: BTreeMap<String, Capability>,
        context_schema: BTreeMap<String, Schema>,
        context_initializers: BTreeMap<String, Expr>,
        events: BTreeMap<String, Event>,
        types: BTreeMap<String, Schema>,
        coerce_functions: BTreeMap<String, CoerceFunction>,
        states: BTreeMap<String, State>,
        invariants: Vec<Invariant>,
        line_starts: Vec<usize>,
        source_name: String,
    }

    #[derive(Default)]
    struct AgentOptions {
        profile: Option<String>,
        max_active: Option<u32>,
    }

    impl Parser {
        fn new(source: &str, source_name: String) -> Self {
            Self {
                syntax: SyntaxBuilder::new(syntax::lex(source)),
                diagnostics: Vec::new(),
                machine_name: None,
                initial: None,
                agents: BTreeMap::new(),
                capabilities: BTreeMap::new(),
                context_schema: BTreeMap::new(),
                context_initializers: BTreeMap::new(),
                events: BTreeMap::new(),
                types: BTreeMap::new(),
                coerce_functions: BTreeMap::new(),
                states: BTreeMap::new(),
                invariants: Vec::new(),
                line_starts: line_starts(source),
                source_name,
            }
        }

        fn parse(mut self) -> ParsedSource {
            self.syntax.start_node(SyntaxKind::File);

            while !self.syntax.at_end() {
                match self.syntax.peek() {
                    Some(SyntaxKind::MachineKw) => self.parse_machine_decl(),
                    Some(SyntaxKind::InitialKw) => self.parse_initial_decl(),
                    Some(SyntaxKind::DataKw) => self.parse_data_block(),
                    Some(SyntaxKind::AgentKw) => self.parse_agent_decl(),
                    Some(SyntaxKind::CapabilityKw) => self.parse_capability_decl(),
                    Some(SyntaxKind::EnumKw) => self.parse_enum_decl(),
                    Some(SyntaxKind::ClassKw) => self.parse_class_decl(),
                    Some(SyntaxKind::CoerceKw) => self.parse_coerce_decl(),
                    Some(SyntaxKind::EventKw) => self.parse_event_decl(),
                    Some(SyntaxKind::StateKw) => {
                        if let Some((name, state)) = self.parse_state_decl() {
                            match self.states.entry(name) {
                                Entry::Vacant(entry) => {
                                    entry.insert(state);
                                }
                                Entry::Occupied(entry) => {
                                    self.diagnostics.push(error_at(
                                        format!("duplicate state `{}`", entry.key()),
                                        state.span.clone(),
                                    ));
                                }
                            }
                        }
                    }
                    Some(SyntaxKind::InvariantKw) => self.parse_invariant_decl(),
                    Some(kind) => {
                        self.diagnostics.push(
                            self.error_at_current(format!("unexpected top-level token `{kind:?}`")),
                        );
                        self.syntax.bump();
                    }
                    None => break,
                }
            }

            self.syntax.emit_remaining();
            self.syntax.finish_node();
            let source_path = self.workflow_source_path();
            let (green, tokens) = self.syntax.finish();

            let missing_machine = self.machine_name.is_none();
            let missing_initial = self.initial.is_none();
            let ir = match (self.machine_name, self.initial) {
                (Some(name), Some(initial)) => Some(WorkflowIr {
                    schema_version: SCHEMA_VERSION.to_string(),
                    workflow: WorkflowMetadata {
                        name,
                        source_path,
                        repo: None,
                        contracts: Vec::new(),
                        plan: None,
                        state_scope: None,
                    },
                    agents: self.agents,
                    events: self.events,
                    capabilities: self.capabilities,
                    context_schema: self.context_schema,
                    context_initializers: self.context_initializers,
                    types: self.types,
                    coerce_functions: self.coerce_functions,
                    statechart: Statechart {
                        initial,
                        states: self.states,
                    },
                    invariants: self.invariants,
                }),
                _ => {
                    if missing_machine {
                        self.diagnostics
                            .push(error("missing `machine` declaration"));
                    }
                    if missing_initial {
                        self.diagnostics
                            .push(error("missing top-level `initial` declaration"));
                    }
                    None
                }
            };

            ParsedSource {
                syntax: ParsedSyntax { green, tokens },
                ir,
                diagnostics: self.diagnostics,
            }
        }

        fn parse_machine_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::MachineDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::MachineKw);
            let name = self.expect_ident("machine name");
            if self.machine_name.is_some() {
                self.diagnostics
                    .push(self.error_at_span("duplicate `machine` declaration", &decl_span));
            } else {
                self.machine_name = name;
            }
            self.syntax.finish_node();
        }

        fn workflow_source_path(&self) -> Option<String> {
            (self.source_name != "<source>").then(|| self.source_name.clone())
        }

        fn parse_initial_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::InitialDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::InitialKw);
            let initial = self.expect_ident("initial state");
            if self.initial.is_some() {
                self.diagnostics.push(
                    self.error_at_span("duplicate top-level `initial` declaration", &decl_span),
                );
            } else {
                self.initial = initial;
            }
            self.syntax.finish_node();
        }

        fn parse_data_block(&mut self) {
            self.syntax.start_node(SyntaxKind::DataBlock);
            self.expect(SyntaxKind::DataKw);
            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    if let Some((name, schema, initializer)) = self.parse_field_decl(true) {
                        let duplicate_span = self.current_span();
                        match self.context_schema.entry(name) {
                            Entry::Vacant(entry) => {
                                let name = entry.key().clone();
                                entry.insert(schema);
                                if let Some(initializer) = initializer {
                                    self.context_initializers.insert(name, initializer);
                                }
                            }
                            Entry::Occupied(entry) => {
                                self.diagnostics.push(error_at(
                                    format!("duplicate data field `{}`", entry.key()),
                                    duplicate_span,
                                ));
                            }
                        }
                    } else {
                        self.syntax.bump();
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }
            self.syntax.finish_node();
        }

        fn parse_agent_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::AgentDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::AgentKw);
            let name = self.expect_ident("agent name");
            if self.syntax.peek() != Some(SyntaxKind::Equals) {
                self.diagnostics.push(self.error_at_current(
                    "agent declarations use `=` and a constructor, for example `agent worker = codingAgent() { maxActive 1 }` or `agent director = thread(\"director\")`",
                ));
                self.skip_until_top_level_decl();
                self.syntax.finish_node();
                return;
            }
            self.expect(SyntaxKind::Equals);
            let target = self.parse_agent_ctor();
            let options = self.parse_agent_options();

            if let (Some(name), Some(target)) = (name, target) {
                match self.agents.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(Agent {
                            target,
                            profile: options.profile,
                            max_active: options.max_active,
                            capabilities: Vec::new(),
                            owns: Vec::new(),
                            contract: None,
                            span: decl_span,
                        });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics.push(error_at(
                            format!("duplicate agent `{}`", entry.key()),
                            decl_span.clone(),
                        ));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_agent_ctor(&mut self) -> Option<AgentTarget> {
            if self.syntax.peek() == Some(SyntaxKind::AdapterKw) {
                self.syntax.bump();
                self.expect(SyntaxKind::LParen);
                let name = self
                    .expect(SyntaxKind::String)
                    .map(|token| decode_string_literal(&token.text));
                self.expect(SyntaxKind::RParen);
                return name.map(|name| AgentTarget::Adapter { name });
            }

            let ctor = self.expect_ident("agent constructor")?;
            self.expect(SyntaxKind::LParen);
            match ctor.as_str() {
                "thread" => {
                    let name = self
                        .expect(SyntaxKind::String)
                        .map(|token| decode_string_literal(&token.text));
                    self.expect(SyntaxKind::RParen);
                    name.map(|name| AgentTarget::Thread { name })
                }
                "codingAgent" => {
                    self.expect(SyntaxKind::RParen);
                    Some(AgentTarget::CodingAgent)
                }
                _ => {
                    self.diagnostics
                        .push(self.error_at_current(format!("unknown agent constructor `{ctor}`")));
                    self.skip_balanced_parens();
                    None
                }
            }
        }

        fn parse_agent_options(&mut self) -> AgentOptions {
            let mut options = AgentOptions::default();
            if self.syntax.eat(SyntaxKind::LBrace).is_none() {
                return options;
            }

            while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                let Some(option) = self.expect_ident("agent option") else {
                    self.syntax.bump();
                    continue;
                };

                match option.as_str() {
                    "maxActive" => {
                        options.max_active = self
                            .syntax
                            .eat(SyntaxKind::Int)
                            .and_then(|token| token.text.parse::<u32>().ok());
                        if options.max_active.is_none() {
                            self.diagnostics.push(self.error_at_current(
                                "agent option `maxActive` must be followed by an integer",
                            ));
                        }
                    }
                    "profile" => {
                        options.profile = self
                            .syntax
                            .eat(SyntaxKind::String)
                            .map(|token| decode_string_literal(&token.text));
                        if options.profile.is_none() {
                            self.diagnostics.push(self.error_at_current(
                                "agent option `profile` must be followed by a string",
                            ));
                        }
                    }
                    _ => {
                        self.diagnostics.push(
                            self.error_at_current(format!("unknown agent option `{option}`")),
                        );
                        if self.syntax.peek() != Some(SyntaxKind::RBrace) {
                            self.syntax.bump();
                        }
                    }
                }
            }

            self.expect(SyntaxKind::RBrace);
            options
        }

        fn parse_capability_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::CapabilityDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::CapabilityKw);
            let name = self.expect_ident("capability name");
            self.expect(SyntaxKind::Equals);
            self.expect(SyntaxKind::AdapterKw);
            self.expect(SyntaxKind::LParen);
            let adapter = self
                .expect(SyntaxKind::String)
                .map(|token| decode_string_literal(&token.text));
            self.expect(SyntaxKind::RParen);

            if let (Some(name), Some(adapter)) = (name, adapter) {
                match self.capabilities.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(Capability {
                            adapter,
                            span: decl_span,
                        });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics.push(error_at(
                            format!("duplicate capability `{}`", entry.key()),
                            decl_span.clone(),
                        ));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_enum_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::EnumDecl);
            self.expect(SyntaxKind::EnumKw);
            let name = self.expect_ident("enum name");
            let mut values = Vec::new();

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    if let Some(value) = self.expect_ident("enum value") {
                        values.push(value);
                    } else {
                        self.syntax.bump();
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            if let Some(name) = name {
                match self.types.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(Schema::Enum { values });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics
                            .push(error(format!("duplicate type `{}`", entry.key())));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_class_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::ClassDecl);
            self.expect(SyntaxKind::ClassKw);
            let name = self.expect_ident("class name");
            let mut fields = Vec::new();

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    if let Some((name, schema, _)) = self.parse_field_decl(false) {
                        fields.push(Field { name, schema });
                    } else {
                        self.syntax.bump();
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            if let Some(name) = name {
                match self.types.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(Schema::Record { fields });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics
                            .push(error(format!("duplicate type `{}`", entry.key())));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_coerce_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::CoerceDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::CoerceKw);
            let name = self.expect_ident("coerce function name");
            let params = self.parse_param_list();
            self.expect(SyntaxKind::Arrow);
            let output = self.parse_type_expr();
            let mut model = None;
            let mut prompt = None;

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    self.syntax.start_node(SyntaxKind::CoerceMember);
                    match self.syntax.peek() {
                        Some(SyntaxKind::ModelKw) => {
                            self.syntax.bump();
                            model = self
                                .expect(SyntaxKind::String)
                                .map(|token| decode_string_literal(&token.text));
                        }
                        Some(SyntaxKind::PromptKw) => {
                            self.syntax.bump();
                            prompt = self
                                .expect(SyntaxKind::BlockString)
                                .map(|token| decode_block_string_literal(&token.text));
                        }
                        Some(kind) => {
                            self.diagnostics.push(
                                self.error_at_current(format!(
                                    "unexpected coerce member `{kind:?}`"
                                )),
                            );
                            self.syntax.bump();
                        }
                        None => {}
                    }
                    self.syntax.finish_node();
                }
                self.expect(SyntaxKind::RBrace);
            }

            if let (Some(name), Some(output)) = (name, output) {
                match self.coerce_functions.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(CoerceFunction {
                            params,
                            output,
                            model,
                            prompt,
                            generated_baml_artifact: None,
                            span: decl_span,
                        });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics.push(error_at(
                            format!("duplicate coerce function `{}`", entry.key()),
                            decl_span.clone(),
                        ));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_param_list(&mut self) -> Vec<CoerceParam> {
            let mut params = Vec::new();
            if self.expect(SyntaxKind::LParen).is_none() {
                return params;
            }

            while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RParen) {
                let name = self.expect_ident("parameter name");
                let schema = self.parse_type_expr();
                if let (Some(name), Some(schema)) = (name, schema) {
                    params.push(CoerceParam { name, schema });
                }

                if self.syntax.eat(SyntaxKind::Comma).is_none()
                    && self.syntax.peek() != Some(SyntaxKind::RParen)
                {
                    self.diagnostics
                        .push(self.error_at_current("expected `,` or `)` in parameter list"));
                    self.syntax.bump();
                }
            }

            self.expect(SyntaxKind::RParen);
            params
        }

        fn parse_event_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::EventDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::EventKw);
            let name = self.expect_ident("event name");
            let mut fields = Vec::new();

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    if let Some((name, schema, _)) = self.parse_field_decl(false) {
                        fields.push(Field { name, schema });
                    } else {
                        self.syntax.bump();
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            if let Some(name) = name {
                match self.events.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(Event {
                            payload: Schema::Record { fields },
                            span: decl_span,
                        });
                    }
                    Entry::Occupied(entry) => {
                        self.diagnostics.push(error_at(
                            format!("duplicate event `{}`", entry.key()),
                            decl_span.clone(),
                        ));
                    }
                }
            }

            self.syntax.finish_node();
        }

        fn parse_field_decl(
            &mut self,
            allow_initial: bool,
        ) -> Option<(String, Schema, Option<Expr>)> {
            self.syntax.start_node(SyntaxKind::FieldDecl);
            let name = self.expect_ident("field name");
            let schema = self.parse_type_expr();

            let initializer = if allow_initial && self.syntax.eat(SyntaxKind::Equals).is_some() {
                self.parse_expr()
            } else {
                None
            };

            self.syntax.finish_node();
            name.zip(schema)
                .map(|(name, schema)| (name, schema, initializer))
        }

        fn parse_state_decl(&mut self) -> Option<(String, State)> {
            self.syntax.start_node(SyntaxKind::StateDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::StateKw);
            let name = self.expect_ident("state name");
            let mut state = State {
                initial: None,
                on: Vec::new(),
                entry: Vec::new(),
                always: Vec::new(),
                states: BTreeMap::new(),
                final_state: false,
                span: decl_span,
            };

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    match self.syntax.peek() {
                        Some(SyntaxKind::InitialKw) => {
                            self.syntax.start_node(SyntaxKind::InitialDecl);
                            self.syntax.bump();
                            state.initial = self.expect_ident("initial child state");
                            self.syntax.finish_node();
                        }
                        Some(SyntaxKind::OnKw) => {
                            if let Some(handler) = self.parse_on_block() {
                                state.on.push(handler);
                            }
                        }
                        Some(SyntaxKind::EntryKw) => {
                            state.entry = self.parse_entry_block();
                        }
                        Some(SyntaxKind::AlwaysKw) => {
                            if let Some(transition) = self.parse_always_block() {
                                state.always.push(transition);
                            }
                        }
                        Some(SyntaxKind::FinalKw) => {
                            self.syntax.start_node(SyntaxKind::FinalMarker);
                            self.syntax.bump();
                            state.final_state = true;
                            self.syntax.finish_node();
                        }
                        Some(SyntaxKind::StateKw) => {
                            if let Some((name, child)) = self.parse_state_decl() {
                                match state.states.entry(name) {
                                    Entry::Vacant(entry) => {
                                        entry.insert(child);
                                    }
                                    Entry::Occupied(entry) => {
                                        self.diagnostics.push(error_at(
                                            format!("duplicate state `{}`", entry.key()),
                                            child.span.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                        Some(kind) => {
                            self.diagnostics.push(
                                self.error_at_current(format!(
                                    "unexpected state member `{kind:?}`"
                                )),
                            );
                            self.skip_recovery_unit();
                        }
                        None => break,
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            self.syntax.finish_node();
            name.map(|name| (name, state))
        }

        fn parse_always_block(&mut self) -> Option<AlwaysTransition> {
            self.syntax.start_node(SyntaxKind::AlwaysBlock);
            let block_span = self.current_span();
            self.expect(SyntaxKind::AlwaysKw);
            let guard = self.parse_guard_list();
            let (steps, transition) = self.parse_action_block();
            self.syntax.finish_node();

            transition
                .map(|transition| AlwaysTransition {
                    guard,
                    steps,
                    transition,
                    span: block_span.clone(),
                })
                .or_else(|| {
                    self.diagnostics.push(error_at(
                        "always block must end with `goto`",
                        block_span.clone(),
                    ));
                    None
                })
        }

        fn parse_entry_block(&mut self) -> Vec<Step> {
            self.syntax.start_node(SyntaxKind::EntryBlock);
            self.expect(SyntaxKind::EntryKw);
            let (mut steps, transition) = self.parse_action_block();
            if let Some(target) = transition {
                let mut args = BTreeMap::new();
                args.insert("target".to_string(), serde_json::Value::String(target));
                steps.push(Step {
                    effect: "goto".to_string(),
                    args,
                    assign: None,
                    case_arms: Vec::new(),
                    span: None,
                });
            }
            self.syntax.finish_node();
            steps
        }

        fn parse_on_block(&mut self) -> Option<EventHandler> {
            self.syntax.start_node(SyntaxKind::OnBlock);
            let block_span = self.current_span();
            self.expect(SyntaxKind::OnKw);
            let event = self.expect_ident("event name");
            let binding = if self.syntax.eat(SyntaxKind::AsKw).is_some() {
                self.expect_ident("event binding")
            } else {
                None
            };

            if self.current_token_text_is("when") {
                self.diagnostics.push(self.error_at_current(
                    "event handlers use `guard`, not `when`; write `guard condition` before the action block",
                ));
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::LBrace) {
                    self.syntax.bump();
                }
            }

            let guard = self.parse_guard_list();
            let (steps, transition) = self.parse_action_block();
            self.syntax.finish_node();

            event.map(|event| EventHandler {
                event,
                binding,
                guard,
                steps,
                transition,
                span: block_span,
            })
        }

        fn parse_action_block(&mut self) -> (Vec<Step>, Option<String>) {
            self.syntax.start_node(SyntaxKind::ActionBlock);
            let mut steps = Vec::new();
            let mut transition = None;
            let mut outcome_seen = false;

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    if outcome_seen {
                        self.diagnostics.push(self.error_at_current(
                            "explicit outcome must be the last statement in an action block",
                        ));
                        self.skip_unsupported_statement();
                        continue;
                    }
                    match self.syntax.peek() {
                        Some(SyntaxKind::AssignKw) => {
                            if let Some(step) = self.parse_assign_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::LetKw) => {
                            if let Some(step) = self.parse_let_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::StartKw) => {
                            if let Some(step) = self.parse_start_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::SendKw) => {
                            if let Some(step) = self.parse_send_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::AskHumanKw) => {
                            if let Some(step) = self.parse_ask_human_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::RaiseKw) => {
                            if let Some(step) = self.parse_raise_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::Ident) if self.next_is(SyntaxKind::Dot) => {
                            if let Some(step) = self.parse_capability_call_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::CaseKw) => {
                            if let Some(step) = self.parse_case_stmt() {
                                steps.push(step);
                            }
                        }
                        Some(SyntaxKind::GotoKw) => {
                            transition = self.parse_goto_stmt();
                            outcome_seen = true;
                        }
                        Some(SyntaxKind::StayKw) => {
                            self.syntax.bump();
                            outcome_seen = true;
                        }
                        Some(kind) => {
                            self.diagnostics.push(self.error_at_current(format!(
                                "unexpected action statement `{kind:?}`"
                            )));
                            self.skip_unsupported_statement();
                        }
                        None => break,
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            self.syntax.finish_node();
            (steps, transition)
        }

        fn parse_assign_stmt(&mut self) -> Option<Step> {
            self.syntax.start_node(SyntaxKind::AssignStmt);
            let step_span = self.current_span();
            self.expect(SyntaxKind::AssignKw);
            let target = self.parse_path();
            self.expect(SyntaxKind::Equals);
            let value = self.parse_expr();
            self.syntax.finish_node();

            let (Some(target), Some(value)) = (target, value) else {
                return None;
            };

            let mut args = BTreeMap::new();
            args.insert("target".to_string(), serde_json::Value::String(target));
            args.insert(
                "value".to_string(),
                serde_json::to_value(value).expect("expr serializes"),
            );

            Some(Step {
                effect: "assign".to_string(),
                args,
                assign: None,
                case_arms: Vec::new(),
                span: step_span,
            })
        }

        fn parse_let_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::LetKw);
            let name = self.expect_ident("local name");
            self.expect(SyntaxKind::Equals);
            let value = self.parse_expr();

            let (Some(name), Some(value)) = (name, value) else {
                return None;
            };

            let mut args = BTreeMap::new();
            args.insert(
                "value".to_string(),
                serde_json::to_value(value).expect("expr serializes"),
            );

            Some(Step {
                effect: "let".to_string(),
                args,
                assign: Some(name),
                case_arms: Vec::new(),
                span: step_span,
            })
        }

        fn parse_start_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::StartKw);
            let agent = self.expect_ident("agent name");
            if self.syntax.peek() == Some(SyntaxKind::LParen) {
                self.diagnostics.push(self.error_at_current(
                    "start uses a block input, not call syntax; write `start worker { message \"...\" }`",
                ));
                self.skip_balanced_parens();
            }
            let input = if self.syntax.peek() == Some(SyntaxKind::LBrace) {
                Some(Expr::Object {
                    fields: self.parse_object_block_fields(),
                })
            } else {
                None
            };

            agent.map(|agent| {
                let mut args = BTreeMap::new();
                args.insert("agent".to_string(), serde_json::Value::String(agent));
                if let Some(input) = input {
                    args.insert(
                        "input".to_string(),
                        serde_json::to_value(input).expect("expr serializes"),
                    );
                }
                Step {
                    effect: "start".to_string(),
                    args,
                    assign: None,
                    case_arms: Vec::new(),
                    span: step_span,
                }
            })
        }

        fn parse_send_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::SendKw);
            let agent = self.expect_ident("agent name");
            let message = self.parse_expr();

            agent.map(|agent| {
                let mut args = BTreeMap::new();
                args.insert("agent".to_string(), serde_json::Value::String(agent));
                if let Some(message) = message {
                    args.insert(
                        "message".to_string(),
                        serde_json::to_value(message).expect("expr serializes"),
                    );
                }
                Step {
                    effect: "send".to_string(),
                    args,
                    assign: None,
                    case_arms: Vec::new(),
                    span: step_span,
                }
            })
        }

        fn parse_ask_human_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::AskHumanKw);
            let reason = if self.expect(SyntaxKind::LParen).is_some() {
                let reason = self.parse_expr();
                self.expect(SyntaxKind::RParen);
                reason
            } else {
                None
            };

            let mut args = BTreeMap::new();
            if let Some(reason) = reason {
                args.insert(
                    "reason".to_string(),
                    serde_json::to_value(reason).expect("expr serializes"),
                );
            }

            Some(Step {
                effect: "askHuman".to_string(),
                args,
                assign: None,
                case_arms: Vec::new(),
                span: step_span,
            })
        }

        fn parse_raise_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::RaiseKw);
            let event = self.expect_ident("event name");
            let payload = if self.syntax.peek() == Some(SyntaxKind::LBrace) {
                Some(Expr::Object {
                    fields: self.parse_object_block_fields(),
                })
            } else {
                None
            };

            event.map(|event| {
                let mut args = BTreeMap::new();
                args.insert("event".to_string(), serde_json::Value::String(event));
                if let Some(payload) = payload {
                    args.insert(
                        "payload".to_string(),
                        serde_json::to_value(payload).expect("expr serializes"),
                    );
                }
                Step {
                    effect: "raise".to_string(),
                    args,
                    assign: None,
                    case_arms: Vec::new(),
                    span: step_span,
                }
            })
        }

        fn parse_capability_call_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            let capability = self.expect_ident("capability name");
            self.expect(SyntaxKind::Dot);
            let operation = self.expect_ident("capability operation");
            let call_args = if self.syntax.peek() == Some(SyntaxKind::LParen) {
                self.parse_arg_list()
            } else {
                Vec::new()
            };

            match (capability, operation) {
                (Some(capability), Some(operation)) => {
                    let mut args = BTreeMap::new();
                    args.insert(
                        "capability".to_string(),
                        serde_json::Value::String(capability),
                    );
                    args.insert(
                        "operation".to_string(),
                        serde_json::Value::String(operation),
                    );
                    args.insert(
                        "call_args".to_string(),
                        serde_json::to_value(call_args).expect("expr list serializes"),
                    );
                    Some(Step {
                        effect: "capability_call".to_string(),
                        args,
                        assign: None,
                        case_arms: Vec::new(),
                        span: step_span,
                    })
                }
                _ => None,
            }
        }

        fn parse_case_stmt(&mut self) -> Option<Step> {
            let step_span = self.current_span();
            self.expect(SyntaxKind::CaseKw);
            let expr = self.parse_expr();

            let mut case_arms = Vec::new();

            if self.expect(SyntaxKind::LBrace).is_some() {
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    let pattern = self.parse_case_pattern();
                    self.expect(SyntaxKind::Arrow);
                    let (arm_steps, arm_transition) = self.parse_action_block();
                    if let Some(pattern) = pattern {
                        case_arms.push(CaseArm {
                            pattern,
                            steps: arm_steps,
                            transition: arm_transition,
                        });
                    }
                }
                self.expect(SyntaxKind::RBrace);
            }

            let mut args = BTreeMap::new();
            if let Some(expr) = expr {
                args.insert(
                    "expr".to_string(),
                    serde_json::to_value(expr).expect("expr serializes"),
                );
            }

            Some(Step {
                effect: "case".to_string(),
                args,
                assign: None,
                case_arms,
                span: step_span,
            })
        }

        fn parse_case_pattern(&mut self) -> Option<CasePattern> {
            match self.syntax.peek() {
                Some(SyntaxKind::Ident) => {
                    let first = self.syntax.bump()?.text;
                    if first == "_" {
                        return Some(CasePattern::Wildcard);
                    }
                    if first == "matches" {
                        return match self.syntax.bump() {
                            Some(token) if token.kind == SyntaxKind::String => {
                                Some(CasePattern::Matches {
                                    pattern: decode_string_literal(&token.text),
                                })
                            }
                            Some(token) => {
                                let span = Some(self.span_for_offsets(
                                    token.range.start().into(),
                                    token.range.end().into(),
                                ));
                                self.diagnostics.push(self.error_at_span(
                                    format!(
                                        "expected string after `matches`, found `{:?}`",
                                        token.kind
                                    ),
                                    &span,
                                ));
                                None
                            }
                            None => {
                                self.diagnostics
                                    .push(self.error_at_current("expected string after `matches`"));
                                None
                            }
                        };
                    }

                    let mut name = first;
                    while self.syntax.eat(SyntaxKind::Dot).is_some() {
                        if let Some(segment) = self.expect_path_segment("case pattern segment") {
                            name.push('.');
                            name.push_str(&segment);
                        }
                    }
                    Some(CasePattern::Identifier { name })
                }
                Some(SyntaxKind::String) => self.syntax.bump().map(|token| CasePattern::Literal {
                    value: serde_json::Value::String(decode_string_literal(&token.text)),
                }),
                Some(SyntaxKind::Int) => self.syntax.bump().map(|token| CasePattern::Literal {
                    value: serde_json::Value::Number(
                        token
                            .text
                            .parse::<i64>()
                            .map(serde_json::Number::from)
                            .unwrap_or_else(|_| serde_json::Number::from(0)),
                    ),
                }),
                Some(SyntaxKind::NilKw) => {
                    self.syntax.bump();
                    Some(CasePattern::Literal {
                        value: serde_json::Value::Null,
                    })
                }
                Some(kind) => {
                    self.diagnostics.push(
                        self.error_at_current(format!("expected case pattern, found `{kind:?}`")),
                    );
                    self.skip_case_pattern();
                    None
                }
                None => {
                    self.diagnostics
                        .push(self.error_at_current("expected case pattern"));
                    None
                }
            }
        }

        fn parse_goto_stmt(&mut self) -> Option<String> {
            self.syntax.start_node(SyntaxKind::GotoStmt);
            self.expect(SyntaxKind::GotoKw);
            let target = self.expect_ident("target state");
            self.syntax.finish_node();
            target
        }

        fn parse_invariant_decl(&mut self) {
            self.syntax.start_node(SyntaxKind::InvariantDecl);
            let decl_span = self.current_span();
            self.expect(SyntaxKind::InvariantKw);
            let name = self.expect_ident("invariant name");
            if self.syntax.peek() == Some(SyntaxKind::LBrace) {
                self.syntax.bump();
                let assert_token = self.syntax.peek_token().cloned();
                if assert_token
                    .as_ref()
                    .is_some_and(|token| token.kind == SyntaxKind::Ident && token.text == "assert")
                {
                    self.syntax.bump();
                } else {
                    self.diagnostics
                        .push(self.error_at_current("expected `assert` in invariant block"));
                }
                let expr = self.parse_expr();
                while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                    self.diagnostics.push(
                        self.error_at_current("expected end of invariant block after expression"),
                    );
                    self.syntax.bump();
                }
                self.expect(SyntaxKind::RBrace);
                if let (Some(name), Some(expr)) = (name, expr) {
                    self.invariants.push(Invariant::Expression {
                        name,
                        expr,
                        span: decl_span,
                    });
                }
            } else if let Some(name) = name {
                self.invariants.push(Invariant::Builtin {
                    name,
                    span: decl_span,
                });
            }
            self.syntax.finish_node();
        }

        fn parse_type_expr(&mut self) -> Option<Schema> {
            self.syntax.start_node(SyntaxKind::TypeExpr);
            let schema = self.parse_union_type();
            self.syntax.finish_node();
            schema
        }

        fn parse_union_type(&mut self) -> Option<Schema> {
            let mut variants = Vec::new();
            if let Some(schema) = self.parse_optional_type() {
                variants.push(schema);
            }
            while self.syntax.eat(SyntaxKind::Pipe).is_some() {
                if let Some(schema) = self.parse_optional_type() {
                    variants.push(schema);
                }
            }

            match variants.len() {
                0 => None,
                1 => variants.pop(),
                _ => Some(Schema::Union { variants }),
            }
        }

        fn parse_optional_type(&mut self) -> Option<Schema> {
            let mut schema = self.parse_postfix_type();
            if self.syntax.eat(SyntaxKind::Question).is_some() {
                if let Some(inner) = schema {
                    schema = Some(Schema::Optional {
                        inner: Box::new(inner),
                    });
                }
            }
            schema
        }

        fn parse_postfix_type(&mut self) -> Option<Schema> {
            let mut schema = self.parse_primary_type();
            while self.syntax.eat(SyntaxKind::LBracket).is_some() {
                self.expect(SyntaxKind::RBracket);
                if let Some(inner) = schema {
                    schema = Some(Schema::List {
                        inner: Box::new(inner),
                    });
                }
            }
            schema
        }

        fn parse_primary_type(&mut self) -> Option<Schema> {
            match self.syntax.peek()? {
                SyntaxKind::String => self.syntax.bump().map(|token| Schema::Literal {
                    value: serde_json::Value::String(decode_string_literal(&token.text)),
                }),
                SyntaxKind::Int => self.syntax.bump().map(|token| Schema::Literal {
                    value: serde_json::Value::Number(
                        token
                            .text
                            .parse::<i64>()
                            .map(serde_json::Number::from)
                            .unwrap_or_else(|_| serde_json::Number::from(0)),
                    ),
                }),
                SyntaxKind::Float => self.syntax.bump().and_then(|token| {
                    token
                        .text
                        .parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                        .map(|number| Schema::Literal {
                            value: serde_json::Value::Number(number),
                        })
                }),
                SyntaxKind::TrueKw => {
                    self.syntax.bump();
                    Some(Schema::Literal {
                        value: serde_json::Value::Bool(true),
                    })
                }
                SyntaxKind::FalseKw => {
                    self.syntax.bump();
                    Some(Schema::Literal {
                        value: serde_json::Value::Bool(false),
                    })
                }
                SyntaxKind::NilKw => {
                    self.syntax.bump();
                    Some(Schema::Literal {
                        value: serde_json::Value::Null,
                    })
                }
                SyntaxKind::LParen => {
                    self.syntax.bump();
                    let schema = self.parse_union_type();
                    self.expect(SyntaxKind::RParen);
                    schema
                }
                _ => self.expect_ident("type name").and_then(|name| {
                    if name == "map" && self.syntax.eat(SyntaxKind::Lt).is_some() {
                        let key = self.parse_union_type();
                        self.expect(SyntaxKind::Comma);
                        let value = self.parse_union_type();
                        self.expect(SyntaxKind::Gt);
                        match (key, value) {
                            (Some(key), Some(value)) => Some(Schema::Map {
                                key: Box::new(key),
                                value: Box::new(value),
                            }),
                            _ => None,
                        }
                    } else {
                        Some(match name.as_str() {
                            "string" => Schema::String,
                            "int" => Schema::Int,
                            "float" => Schema::Float,
                            "bool" => Schema::Boolean,
                            "null" => Schema::Null,
                            "time" => Schema::Time,
                            "duration" => Schema::Duration,
                            "agent" => Schema::Agent,
                            "json" => Schema::Json,
                            _ => Schema::Ref { name },
                        })
                    }
                }),
            }
        }

        fn parse_guard_list(&mut self) -> Option<Expr> {
            let mut guards = Vec::new();
            while self.syntax.peek() == Some(SyntaxKind::GuardKw) {
                self.syntax.bump();
                if let Some(expr) = self.parse_expr() {
                    guards.push(expr);
                } else {
                    while !self.syntax.at_end()
                        && !matches!(
                            self.syntax.peek(),
                            Some(SyntaxKind::GuardKw | SyntaxKind::LBrace)
                        )
                    {
                        self.syntax.bump();
                    }
                }
            }

            match guards.len() {
                0 => None,
                1 => guards.into_iter().next(),
                _ => Some(Expr::And { exprs: guards }),
            }
        }

        fn parse_expr(&mut self) -> Option<Expr> {
            self.syntax.start_node(SyntaxKind::Expr);
            let expr = self.parse_or_expr();
            self.syntax.finish_node();
            expr
        }

        fn parse_or_expr(&mut self) -> Option<Expr> {
            let mut exprs = Vec::new();
            exprs.push(self.parse_and_expr()?);
            while self.syntax.eat(SyntaxKind::OrOr).is_some() {
                if let Some(expr) = self.parse_and_expr() {
                    exprs.push(expr);
                }
            }

            if exprs.len() == 1 {
                exprs.into_iter().next()
            } else {
                Some(Expr::Or { exprs })
            }
        }

        fn parse_and_expr(&mut self) -> Option<Expr> {
            let mut exprs = Vec::new();
            exprs.push(self.parse_equality_expr()?);
            while self.syntax.eat(SyntaxKind::AndAnd).is_some() {
                if let Some(expr) = self.parse_equality_expr() {
                    exprs.push(expr);
                }
            }

            if exprs.len() == 1 {
                exprs.into_iter().next()
            } else {
                Some(Expr::And { exprs })
            }
        }

        fn parse_equality_expr(&mut self) -> Option<Expr> {
            let mut expr = self.parse_compare_expr()?;
            while matches!(
                self.syntax.peek(),
                Some(SyntaxKind::EqEq | SyntaxKind::NotEq)
            ) {
                let Some(operator) = self.syntax.bump().map(|token| token.kind) else {
                    break;
                };
                let Some(right) = self.parse_compare_expr() else {
                    break;
                };
                expr = match operator {
                    SyntaxKind::EqEq => Expr::Eq {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    SyntaxKind::NotEq => Expr::Neq {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    _ => unreachable!("operator checked above"),
                };
            }
            Some(expr)
        }

        fn parse_compare_expr(&mut self) -> Option<Expr> {
            let mut expr = self.parse_membership_expr()?;
            while matches!(
                self.syntax.peek(),
                Some(SyntaxKind::Lt | SyntaxKind::LtEq | SyntaxKind::Gt | SyntaxKind::GtEq)
            ) {
                let Some(operator) = self.syntax.bump().map(|token| token.kind) else {
                    break;
                };
                let Some(right) = self.parse_membership_expr() else {
                    break;
                };
                expr = match operator {
                    SyntaxKind::Lt => Expr::Lt {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    SyntaxKind::LtEq => Expr::Lte {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    SyntaxKind::Gt => Expr::Gt {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    SyntaxKind::GtEq => Expr::Gte {
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                    _ => unreachable!("operator checked above"),
                };
            }
            Some(expr)
        }

        fn parse_membership_expr(&mut self) -> Option<Expr> {
            let mut expr = self.parse_unary_expr()?;
            while self.syntax.eat(SyntaxKind::InKw).is_some() {
                let Some(right) = self.parse_unary_expr() else {
                    break;
                };
                expr = Expr::In {
                    left: Box::new(expr),
                    right: Box::new(right),
                };
            }
            Some(expr)
        }

        fn parse_unary_expr(&mut self) -> Option<Expr> {
            if self.syntax.eat(SyntaxKind::Bang).is_some() {
                return self.parse_unary_expr().map(|expr| Expr::Not {
                    expr: Box::new(expr),
                });
            }

            self.parse_primary_expr()
        }

        fn parse_primary_expr(&mut self) -> Option<Expr> {
            match self.syntax.peek() {
                Some(SyntaxKind::NilKw) => {
                    self.syntax.bump();
                    Some(Expr::Literal {
                        value: serde_json::Value::Null,
                    })
                }
                Some(SyntaxKind::TrueKw) => {
                    self.syntax.bump();
                    Some(Expr::Literal {
                        value: serde_json::Value::Bool(true),
                    })
                }
                Some(SyntaxKind::FalseKw) => {
                    self.syntax.bump();
                    Some(Expr::Literal {
                        value: serde_json::Value::Bool(false),
                    })
                }
                Some(SyntaxKind::String) => self.syntax.bump().map(|token| Expr::Literal {
                    value: serde_json::Value::String(decode_string_literal(&token.text)),
                }),
                Some(SyntaxKind::BlockString) => self.syntax.bump().map(|token| Expr::Literal {
                    value: serde_json::Value::String(decode_block_string_literal(&token.text)),
                }),
                Some(SyntaxKind::Duration) => self.syntax.bump().map(|token| Expr::Literal {
                    value: serde_json::Value::String(token.text),
                }),
                Some(SyntaxKind::Int) => self.syntax.bump().map(|token| Expr::Literal {
                    value: serde_json::Value::Number(
                        token
                            .text
                            .parse::<i64>()
                            .map(serde_json::Number::from)
                            .unwrap_or_else(|_| serde_json::Number::from(0)),
                    ),
                }),
                Some(SyntaxKind::Float) => self.syntax.bump().and_then(|token| {
                    token
                        .text
                        .parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                        .map(|number| Expr::Literal {
                            value: serde_json::Value::Number(number),
                        })
                }),
                Some(SyntaxKind::LBracket) => {
                    self.syntax.bump();
                    let mut items = Vec::new();
                    while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBracket)
                    {
                        if let Some(item) = self.parse_expr() {
                            items.push(item);
                        }
                        if self.syntax.eat(SyntaxKind::Comma).is_none()
                            && self.syntax.peek() != Some(SyntaxKind::RBracket)
                        {
                            self.syntax.bump();
                        }
                    }
                    self.expect(SyntaxKind::RBracket);
                    Some(Expr::List { items })
                }
                Some(SyntaxKind::LParen) => {
                    self.syntax.bump();
                    let expr = self.parse_expr();
                    self.expect(SyntaxKind::RParen);
                    expr
                }
                Some(SyntaxKind::LBrace) => Some(self.parse_object_expr()),
                Some(SyntaxKind::CoerceKw) => {
                    self.syntax.bump();
                    self.parse_call_expr(true)
                }
                Some(kind) if is_name_token(kind) => self.parse_call_expr(false),
                Some(kind) => {
                    self.diagnostics.push(
                        self.error_at_current(format!("expected expression, found `{kind:?}`")),
                    );
                    self.syntax.bump();
                    None
                }
                None => {
                    self.diagnostics
                        .push(self.error_at_current("expected expression"));
                    None
                }
            }
        }

        fn parse_object_expr(&mut self) -> Expr {
            self.expect(SyntaxKind::LBrace);
            let fields = self.parse_object_fields_until_rbrace();
            self.expect(SyntaxKind::RBrace);
            Expr::Object { fields }
        }

        fn parse_object_block_fields(&mut self) -> BTreeMap<String, Expr> {
            self.expect(SyntaxKind::LBrace);
            let fields = self.parse_object_fields_until_rbrace();
            self.expect(SyntaxKind::RBrace);
            fields
        }

        fn parse_object_fields_until_rbrace(&mut self) -> BTreeMap<String, Expr> {
            let mut fields = BTreeMap::new();
            while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RBrace) {
                let name = self.expect_ident("object field name");
                let value = self.parse_expr();
                if let (Some(name), Some(value)) = (name, value) {
                    match fields.entry(name) {
                        Entry::Vacant(entry) => {
                            entry.insert(value);
                        }
                        Entry::Occupied(entry) => {
                            self.diagnostics.push(self.error_at_current(format!(
                                "duplicate object field `{}`",
                                entry.key()
                            )));
                        }
                    }
                }
                self.syntax.eat(SyntaxKind::Comma);
            }
            fields
        }

        fn parse_call_expr(&mut self, explicit_coerce: bool) -> Option<Expr> {
            let name = self.parse_path()?;
            if self.syntax.peek() != Some(SyntaxKind::LParen) {
                return Some(Expr::Path { path: name });
            }

            let args = self.parse_arg_list();
            Some(Expr::Call {
                name: if explicit_coerce {
                    format!("coerce {name}")
                } else {
                    name
                },
                args,
            })
        }

        fn parse_arg_list(&mut self) -> Vec<Expr> {
            let mut args = Vec::new();
            if self.expect(SyntaxKind::LParen).is_none() {
                return args;
            }

            while !self.syntax.at_end() && self.syntax.peek() != Some(SyntaxKind::RParen) {
                if let Some(arg) = self.parse_expr() {
                    args.push(arg);
                }
                if self.syntax.eat(SyntaxKind::Comma).is_none()
                    && self.syntax.peek() != Some(SyntaxKind::RParen)
                {
                    self.syntax.bump();
                }
            }

            self.expect(SyntaxKind::RParen);
            args
        }

        fn parse_path(&mut self) -> Option<String> {
            self.syntax.start_node(SyntaxKind::Path);
            let mut path = self.expect_path_segment("path");
            if let Some(path_value) = path.as_mut() {
                while self.syntax.eat(SyntaxKind::Dot).is_some() {
                    if let Some(segment) = self.expect_path_segment("path segment") {
                        path_value.push('.');
                        path_value.push_str(&segment);
                    }
                }
            }
            self.syntax.finish_node();
            path
        }

        fn expect(&mut self, kind: SyntaxKind) -> Option<syntax::Token> {
            let found = self.syntax.peek();
            if found == Some(kind) {
                self.syntax.bump()
            } else {
                self.diagnostics.push(self.error_at_current(format!(
                    "expected `{kind:?}`, found `{}`",
                    found
                        .map(|kind| format!("{kind:?}"))
                        .unwrap_or_else(|| "end of file".to_string())
                )));
                None
            }
        }

        fn next_is(&self, kind: SyntaxKind) -> bool {
            self.syntax.peek_nth(1) == Some(kind)
        }

        fn current_token_text_is(&self, text: &str) -> bool {
            self.syntax
                .peek_token()
                .is_some_and(|token| token.text == text)
        }

        fn expect_ident(&mut self, label: &str) -> Option<String> {
            if self.syntax.peek().is_some_and(is_name_token) {
                self.syntax.bump().map(|token| token.text)
            } else {
                self.diagnostics
                    .push(self.error_at_current(format!("expected {label}")));
                None
            }
        }

        fn expect_path_segment(&mut self, label: &str) -> Option<String> {
            match self.syntax.peek() {
                Some(
                    SyntaxKind::Ident
                    | SyntaxKind::DataKw
                    | SyntaxKind::EventKw
                    | SyntaxKind::AgentKw
                    | SyntaxKind::CapabilityKw
                    | SyntaxKind::ClassKw
                    | SyntaxKind::EnumKw
                    | SyntaxKind::CoerceKw
                    | SyntaxKind::StartKw
                    | SyntaxKind::SendKw
                    | SyntaxKind::AskHumanKw
                    | SyntaxKind::RaiseKw
                    | SyntaxKind::StayKw
                    | SyntaxKind::GuardKw
                    | SyntaxKind::AlwaysKw
                    | SyntaxKind::EntryKw
                    | SyntaxKind::CaseKw,
                ) => self.syntax.bump().map(|token| token.text),
                _ => {
                    self.diagnostics
                        .push(self.error_at_current(format!("expected {label}")));
                    None
                }
            }
        }

        fn error_at_current(&self, message: impl Into<String>) -> Diagnostic {
            Diagnostic {
                severity: Severity::Error,
                message: message.into(),
                span: self.current_span(),
            }
        }

        fn error_at_span(
            &self,
            message: impl Into<String>,
            span: &Option<SourceSpan>,
        ) -> Diagnostic {
            Diagnostic {
                severity: Severity::Error,
                message: message.into(),
                span: span.clone(),
            }
        }

        fn current_span(&self) -> Option<SourceSpan> {
            self.syntax.peek_token().map(|token| {
                self.span_for_offsets(token.range.start().into(), token.range.end().into())
            })
        }

        fn span_for_offsets(&self, start: u32, end: u32) -> SourceSpan {
            let (start_line, start_column) = self.line_column(start as usize);
            let (end_line, end_column) = self.line_column(end as usize);
            SourceSpan {
                file: self.source_name.clone(),
                start_line,
                start_column,
                end_line,
                end_column,
            }
        }

        fn line_column(&self, offset: usize) -> (u32, u32) {
            let line_index = match self.line_starts.binary_search(&offset) {
                Ok(index) => index,
                Err(index) => index.saturating_sub(1),
            };
            let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
            (
                line_index as u32 + 1,
                offset.saturating_sub(line_start) as u32 + 1,
            )
        }

        fn skip_balanced_parens(&mut self) {
            if self.syntax.eat(SyntaxKind::LParen).is_none() {
                return;
            }

            let mut depth = 1usize;
            while depth > 0 && !self.syntax.at_end() {
                match self.syntax.bump().map(|token| token.kind) {
                    Some(SyntaxKind::LParen) => depth += 1,
                    Some(SyntaxKind::RParen) => depth -= 1,
                    Some(_) => {}
                    None => break,
                }
            }
        }

        fn skip_recovery_unit(&mut self) {
            match self.syntax.peek() {
                Some(SyntaxKind::LBrace) => self.skip_balanced_braces(),
                Some(SyntaxKind::LParen) => self.skip_balanced_parens(),
                Some(_) => {
                    self.syntax.bump();
                }
                None => {}
            }
        }

        fn skip_unsupported_statement(&mut self) {
            self.skip_recovery_unit();
            while !self.syntax.at_end() {
                match self.syntax.peek() {
                    Some(
                        SyntaxKind::AssignKw
                        | SyntaxKind::GotoKw
                        | SyntaxKind::StayKw
                        | SyntaxKind::CaseKw
                        | SyntaxKind::LetKw
                        | SyntaxKind::StartKw
                        | SyntaxKind::SendKw
                        | SyntaxKind::AskHumanKw
                        | SyntaxKind::RBrace,
                    ) => break,
                    Some(SyntaxKind::LBrace) => self.skip_balanced_braces(),
                    Some(SyntaxKind::LParen) => self.skip_balanced_parens(),
                    Some(_) => {
                        self.syntax.bump();
                    }
                    None => break,
                }
            }
        }

        fn skip_until_top_level_decl(&mut self) {
            while !self.syntax.at_end() {
                match self.syntax.peek() {
                    Some(
                        SyntaxKind::MachineKw
                        | SyntaxKind::InitialKw
                        | SyntaxKind::DataKw
                        | SyntaxKind::AgentKw
                        | SyntaxKind::CapabilityKw
                        | SyntaxKind::EnumKw
                        | SyntaxKind::ClassKw
                        | SyntaxKind::CoerceKw
                        | SyntaxKind::EventKw
                        | SyntaxKind::StateKw
                        | SyntaxKind::InvariantKw,
                    ) => break,
                    Some(SyntaxKind::LBrace) => self.skip_balanced_braces(),
                    Some(SyntaxKind::LParen) => self.skip_balanced_parens(),
                    Some(_) => {
                        self.syntax.bump();
                    }
                    None => break,
                }
            }
        }

        fn skip_case_pattern(&mut self) {
            while !self.syntax.at_end()
                && !matches!(
                    self.syntax.peek(),
                    Some(SyntaxKind::Arrow | SyntaxKind::RBrace)
                )
            {
                match self.syntax.peek() {
                    Some(SyntaxKind::LParen) => self.skip_balanced_parens(),
                    Some(SyntaxKind::LBrace) => self.skip_balanced_braces(),
                    Some(_) => {
                        self.syntax.bump();
                    }
                    None => break,
                }
            }
        }

        fn skip_balanced_braces(&mut self) {
            if self.syntax.eat(SyntaxKind::LBrace).is_none() {
                return;
            }

            let mut depth = 1usize;
            while depth > 0 && !self.syntax.at_end() {
                match self.syntax.bump().map(|token| token.kind) {
                    Some(SyntaxKind::LBrace) => depth += 1,
                    Some(SyntaxKind::RBrace) => depth -= 1,
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    fn decode_string_literal(text: &str) -> String {
        text.strip_prefix('"')
            .and_then(|text| text.strip_suffix('"'))
            .unwrap_or(text)
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
    }

    fn decode_block_string_literal(text: &str) -> String {
        text.strip_prefix("\"\"\"")
            .and_then(|text| text.strip_suffix("\"\"\""))
            .unwrap_or(text)
            .to_string()
    }

    fn is_name_token(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::Ident
                | SyntaxKind::StartKw
                | SyntaxKind::SendKw
                | SyntaxKind::AskHumanKw
                | SyntaxKind::StayKw
                | SyntaxKind::GuardKw
                | SyntaxKind::EntryKw
                | SyntaxKind::CaseKw
                | SyntaxKind::DataKw
                | SyntaxKind::EventKw
                | SyntaxKind::AgentKw
                | SyntaxKind::CapabilityKw
                | SyntaxKind::ClassKw
                | SyntaxKind::EnumKw
                | SyntaxKind::CoerceKw
        )
    }

    fn line_starts(source: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        starts
    }

    fn error(message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span: None,
        }
    }

    fn error_at(message: impl Into<String>, span: Option<SourceSpan>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span,
        }
    }
}

pub mod validate {
    use crate::diagnostics::{Diagnostic, Severity, SourceSpan};
    use crate::expr::Expr;
    use crate::ir::{State, WorkflowIr, SCHEMA_VERSION};
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ValidationReport {
        pub diagnostics: Vec<Diagnostic>,
    }

    impl ValidationReport {
        pub fn is_ok(&self) -> bool {
            !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        }
    }

    #[derive(Debug, Clone, Default)]
    struct ExprScope {
        event_binding: Option<String>,
        event_name: Option<String>,
        locals: BTreeMap<String, crate::schema::Schema>,
        non_null_paths: BTreeSet<String>,
    }

    impl ExprScope {
        fn with_event_binding(event_name: &str, binding: Option<&str>) -> Self {
            Self {
                event_binding: binding.map(str::to_string),
                event_name: Some(event_name.to_string()),
                locals: BTreeMap::new(),
                non_null_paths: BTreeSet::new(),
            }
        }

        fn apply_guard_refinements(&mut self, guard: &Expr) {
            collect_non_null_paths(guard, &mut self.non_null_paths);
        }

        fn apply_case_pattern_refinements(
            &mut self,
            scrutinee: &Expr,
            pattern: &crate::ir::CasePattern,
            nil_previously_matched: bool,
        ) {
            let Expr::Path { path } = scrutinee else {
                return;
            };

            if case_pattern_proves_non_null(pattern, nil_previously_matched) {
                self.non_null_paths.insert(path.clone());
            }
        }
    }

    fn collect_non_null_paths(expr: &Expr, paths: &mut BTreeSet<String>) {
        match expr {
            Expr::And { exprs } => {
                for expr in exprs {
                    collect_non_null_paths(expr, paths);
                }
            }
            Expr::Or { exprs } => {
                if let Some(common_paths) = common_non_null_paths(exprs) {
                    paths.extend(common_paths);
                }
            }
            Expr::Neq { left, right } => {
                if let (Expr::Path { path }, true) = (&**left, is_nil_expr(right)) {
                    paths.insert(path.clone());
                } else if let (true, Expr::Path { path }) = (is_nil_expr(left), &**right) {
                    paths.insert(path.clone());
                }
            }
            Expr::Not { expr } => collect_negated_non_null_paths(expr, paths),
            _ => {}
        }
    }

    fn collect_negated_non_null_paths(expr: &Expr, paths: &mut BTreeSet<String>) {
        match expr {
            Expr::Eq { left, right } => {
                if let (Expr::Path { path }, true) = (&**left, is_nil_expr(right)) {
                    paths.insert(path.clone());
                } else if let (true, Expr::Path { path }) = (is_nil_expr(left), &**right) {
                    paths.insert(path.clone());
                }
            }
            Expr::Or { exprs } => {
                for expr in exprs {
                    collect_negated_non_null_paths(expr, paths);
                }
            }
            Expr::Not { expr } => collect_non_null_paths(expr, paths),
            _ => {}
        }
    }

    fn common_non_null_paths(exprs: &[Expr]) -> Option<BTreeSet<String>> {
        let mut exprs = exprs.iter();
        let first = exprs.next()?;
        let mut common = BTreeSet::new();
        collect_non_null_paths(first, &mut common);

        for expr in exprs {
            let mut paths = BTreeSet::new();
            collect_non_null_paths(expr, &mut paths);
            common = common.intersection(&paths).cloned().collect();
            if common.is_empty() {
                break;
            }
        }

        Some(common)
    }

    fn is_nil_expr(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Literal {
                value
            } if value.is_null()
        )
    }

    fn case_pattern_proves_non_null(
        pattern: &crate::ir::CasePattern,
        nil_previously_matched: bool,
    ) -> bool {
        match pattern {
            crate::ir::CasePattern::Identifier { .. } | crate::ir::CasePattern::Matches { .. } => {
                true
            }
            crate::ir::CasePattern::Literal { value } => !value.is_null(),
            crate::ir::CasePattern::Wildcard => nil_previously_matched,
        }
    }

    fn case_pattern_matches_nil(pattern: &crate::ir::CasePattern) -> bool {
        matches!(
            pattern,
            crate::ir::CasePattern::Literal {
                value
            } if value.is_null()
        )
    }

    pub fn validate_ir(ir: &WorkflowIr) -> ValidationReport {
        let mut diagnostics = Vec::new();

        if ir.schema_version != SCHEMA_VERSION {
            diagnostics.push(error(format!(
                "unsupported workflow IR schema version `{}`",
                ir.schema_version
            )));
        }

        if !ir.statechart.states.contains_key(&ir.statechart.initial) {
            diagnostics.push(error(format!(
                "initial state `{}` is not declared",
                ir.statechart.initial
            )));
        }

        let mut state_names = BTreeSet::new();
        collect_state_names(&mut diagnostics, &mut state_names, &ir.statechart.states);
        let event_names: BTreeSet<_> = ir.events.keys().cloned().collect();
        let type_names: BTreeSet<_> = ir.types.keys().cloned().collect();

        for (state_name, state) in &ir.statechart.states {
            validate_state(
                &mut diagnostics,
                state_name,
                state,
                ir,
                &state_names,
                &event_names,
            );
        }
        validate_bounded_start_completion_convention(&mut diagnostics, ir);

        let mut invariant_names = BTreeSet::new();
        for invariant in &ir.invariants {
            let (name, span) = match invariant {
                crate::ir::Invariant::Builtin { name, span }
                | crate::ir::Invariant::Expression { name, span, .. } => (name, span),
            };
            if !invariant_names.insert(name) {
                diagnostics.push(error_at(
                    format!("invariant `{name}` is declared more than once"),
                    span,
                ));
            }

            if matches!(invariant, crate::ir::Invariant::Builtin { .. })
                && !is_supported_builtin_invariant(name)
            {
                diagnostics.push(error_at(
                    format!("unknown built-in invariant `{name}`"),
                    span,
                ));
            }

            if let crate::ir::Invariant::Expression {
                name, expr, span, ..
            } = invariant
            {
                validate_expr(
                    &mut diagnostics,
                    &format!("invariant `{name}` expression"),
                    ir,
                    expr,
                    &ExprScope::default(),
                    span,
                );
                validate_boolean_expr(
                    &mut diagnostics,
                    &format!("invariant `{name}` expression"),
                    ir,
                    expr,
                    &ExprScope::default(),
                    span,
                );
            }
        }

        for (agent_name, agent) in &ir.agents {
            if agent.max_active == Some(0) {
                diagnostics.push(error_at(
                    format!("agent `{agent_name}` maxActive must be greater than 0"),
                    &agent.span,
                ));
            }
            if let Some(profile) = &agent.profile {
                if profile.trim().is_empty()
                    || profile.chars().any(|character| character.is_control())
                {
                    diagnostics.push(error_at(
                        format!("agent `{agent_name}` profile must be non-empty and contain no control characters"),
                        &agent.span,
                    ));
                }
            }
        }

        for (event_name, event) in &ir.events {
            validate_schema_uniqueness(
                &mut diagnostics,
                &format!("event `{event_name}` payload"),
                &event.payload,
            );
            validate_schema_refs(
                &mut diagnostics,
                &format!("event `{event_name}` payload"),
                &event.payload,
                &type_names,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &format!("event `{event_name}` payload"),
                &event.payload,
                &ir.types,
            );
        }

        for (field_name, schema) in &ir.context_schema {
            validate_schema_uniqueness(
                &mut diagnostics,
                &format!("data field `{field_name}`"),
                schema,
            );
            validate_schema_refs(
                &mut diagnostics,
                &format!("data field `{field_name}`"),
                schema,
                &type_names,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &format!("data field `{field_name}`"),
                schema,
                &ir.types,
            );
        }

        for (field_name, initializer) in &ir.context_initializers {
            validate_data_initializer(&mut diagnostics, field_name, initializer, ir);
        }

        for (type_name, schema) in &ir.types {
            validate_schema_uniqueness(&mut diagnostics, &format!("type `{type_name}`"), schema);
            validate_schema_refs(
                &mut diagnostics,
                &format!("type `{type_name}`"),
                schema,
                &type_names,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &format!("type `{type_name}`"),
                schema,
                &ir.types,
            );
        }
        validate_type_cycles(&mut diagnostics, &ir.types);

        for (function_name, function) in &ir.coerce_functions {
            validate_unique_coerce_params(&mut diagnostics, function_name, &function.params);
            for param in &function.params {
                validate_schema_uniqueness(
                    &mut diagnostics,
                    &format!("coerce `{function_name}` parameter `{}`", param.name),
                    &param.schema,
                );
                validate_schema_refs(
                    &mut diagnostics,
                    &format!("coerce `{function_name}` parameter `{}`", param.name),
                    &param.schema,
                    &type_names,
                );
                validate_baml_boundary_schema(
                    &mut diagnostics,
                    &format!("coerce `{function_name}` parameter `{}`", param.name),
                    &param.schema,
                );
                validate_map_key_schemas(
                    &mut diagnostics,
                    &format!("coerce `{function_name}` parameter `{}`", param.name),
                    &param.schema,
                    &ir.types,
                );
            }
            validate_schema_uniqueness(
                &mut diagnostics,
                &format!("coerce `{function_name}` output"),
                &function.output,
            );
            validate_schema_refs(
                &mut diagnostics,
                &format!("coerce `{function_name}` output"),
                &function.output,
                &type_names,
            );
            validate_baml_boundary_schema(
                &mut diagnostics,
                &format!("coerce `{function_name}` output"),
                &function.output,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &format!("coerce `{function_name}` output"),
                &function.output,
                &ir.types,
            );
        }

        ValidationReport { diagnostics }
    }

    fn is_supported_builtin_invariant(name: &str) -> bool {
        matches!(
            name,
            "agentCapabilitiesRespected"
                | "declaredAgentsOnly"
                | "declaredEffectsOnly"
                | "maxActiveRespected"
                | "terminalInvocationsObserved"
                | "failedEffectsAreDurable"
                | "blockedWorkIsVisible"
                | "noSilentEventDrop"
                | "noUnboundedInternalLoop"
        )
    }

    fn validate_unique_coerce_params(
        diagnostics: &mut Vec<Diagnostic>,
        function_name: &str,
        params: &[crate::ir::CoerceParam],
    ) {
        let mut names = BTreeSet::new();
        for param in params {
            if !names.insert(param.name.clone()) {
                diagnostics.push(error(format!(
                    "coerce `{function_name}` declares duplicate parameter `{}`",
                    param.name
                )));
            }
        }
    }

    fn validate_schema_uniqueness(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        schema: &crate::schema::Schema,
    ) {
        match schema {
            crate::schema::Schema::Enum { values } => {
                let mut seen = BTreeSet::new();
                for value in values {
                    if !seen.insert(value) {
                        diagnostics.push(error(format!(
                            "{owner} declares duplicate enum value `{value}`"
                        )));
                    }
                    if !starts_with_ascii_uppercase(value) {
                        diagnostics.push(error(format!(
                            "{owner} enum value `{value}` must start with an uppercase ASCII letter for BAML compatibility"
                        )));
                    }
                }
            }
            crate::schema::Schema::Record { fields } => {
                let mut seen = BTreeSet::new();
                for field in fields {
                    if !seen.insert(&field.name) {
                        diagnostics.push(error(format!(
                            "{owner} declares duplicate field `{}`",
                            field.name
                        )));
                    }
                    validate_schema_uniqueness(
                        diagnostics,
                        &format!("{owner} field `{}`", field.name),
                        &field.schema,
                    );
                }
            }
            crate::schema::Schema::Optional { inner }
            | crate::schema::Schema::List { inner }
            | crate::schema::Schema::Set { inner } => {
                validate_schema_uniqueness(diagnostics, owner, inner);
            }
            crate::schema::Schema::Map { key, value } => {
                validate_schema_uniqueness(diagnostics, owner, key);
                validate_schema_uniqueness(diagnostics, owner, value);
            }
            crate::schema::Schema::Union { variants } => {
                for variant in variants {
                    validate_schema_uniqueness(diagnostics, owner, variant);
                }
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Ref { .. }
            | crate::schema::Schema::Json => {}
        }
    }

    fn validate_baml_boundary_schema(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        schema: &crate::schema::Schema,
    ) {
        match schema {
            crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Set { .. }
            | crate::schema::Schema::Json => diagnostics.push(error(format!(
                "{owner} uses `{}` which is not supported as a BAML boundary type",
                schema_kind(schema)
            ))),
            crate::schema::Schema::Optional { inner } | crate::schema::Schema::List { inner } => {
                validate_baml_boundary_schema(diagnostics, owner, inner);
            }
            crate::schema::Schema::Map { key, value } => {
                validate_baml_boundary_schema(diagnostics, owner, key);
                validate_baml_boundary_schema(diagnostics, owner, value);
            }
            crate::schema::Schema::Union { variants } => {
                for variant in variants {
                    validate_baml_boundary_schema(diagnostics, owner, variant);
                }
            }
            crate::schema::Schema::Record { fields } => {
                for field in fields {
                    validate_baml_boundary_schema(diagnostics, owner, &field.schema);
                }
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Enum { .. }
            | crate::schema::Schema::Ref { .. } => {}
        }
    }

    fn validate_map_key_schemas(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        schema: &crate::schema::Schema,
        types: &std::collections::BTreeMap<String, crate::schema::Schema>,
    ) {
        validate_map_key_schemas_inner(diagnostics, owner, schema, types, 0);
    }

    fn validate_map_key_schemas_inner(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        schema: &crate::schema::Schema,
        types: &std::collections::BTreeMap<String, crate::schema::Schema>,
        depth: usize,
    ) {
        if depth > 16 {
            return;
        }

        match schema {
            crate::schema::Schema::Optional { inner }
            | crate::schema::Schema::List { inner }
            | crate::schema::Schema::Set { inner } => {
                validate_map_key_schemas_inner(diagnostics, owner, inner, types, depth + 1);
            }
            crate::schema::Schema::Map { key, value } => {
                if !map_key_schema_is_string_compatible(key, types, depth + 1) {
                    diagnostics.push(error(format!(
                        "{owner} declares map key type `{}`; map keys must be string-compatible",
                        schema_kind(key)
                    )));
                }
                validate_map_key_schemas_inner(diagnostics, owner, value, types, depth + 1);
            }
            crate::schema::Schema::Union { variants } => {
                for variant in variants {
                    validate_map_key_schemas_inner(diagnostics, owner, variant, types, depth + 1);
                }
            }
            crate::schema::Schema::Record { fields } => {
                for field in fields {
                    validate_map_key_schemas_inner(
                        diagnostics,
                        owner,
                        &field.schema,
                        types,
                        depth + 1,
                    );
                }
            }
            crate::schema::Schema::Ref { name } => {
                if let Some(schema) = types.get(name) {
                    validate_map_key_schemas_inner(diagnostics, owner, schema, types, depth + 1);
                }
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Enum { .. }
            | crate::schema::Schema::Json => {}
        }
    }

    fn map_key_schema_is_string_compatible(
        schema: &crate::schema::Schema,
        types: &std::collections::BTreeMap<String, crate::schema::Schema>,
        depth: usize,
    ) -> bool {
        if depth > 16 {
            return false;
        }

        match schema {
            crate::schema::Schema::String | crate::schema::Schema::Enum { .. } => true,
            crate::schema::Schema::Literal { value } => value.is_string(),
            crate::schema::Schema::Union { variants } => {
                !variants.is_empty()
                    && variants.iter().all(|variant| {
                        map_key_schema_is_string_compatible(variant, types, depth + 1)
                    })
            }
            crate::schema::Schema::Ref { name } => types
                .get(name)
                .is_none_or(|schema| map_key_schema_is_string_compatible(schema, types, depth + 1)),
            crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Optional { .. }
            | crate::schema::Schema::List { .. }
            | crate::schema::Schema::Set { .. }
            | crate::schema::Schema::Map { .. }
            | crate::schema::Schema::Record { .. }
            | crate::schema::Schema::Json => false,
        }
    }

    fn validate_data_initializer(
        diagnostics: &mut Vec<Diagnostic>,
        field_name: &str,
        initializer: &Expr,
        ir: &WorkflowIr,
    ) {
        let Some(schema) = ir.context_schema.get(field_name) else {
            diagnostics.push(error(format!(
                "data initializer references undeclared data field `{field_name}`"
            )));
            return;
        };

        let Some(value) = static_initializer_value(initializer) else {
            diagnostics.push(error(format!(
                "data field `{field_name}` initializer must be a static literal, list, or object"
            )));
            return;
        };

        if !schema.accepts_json_with_types(&value, &ir.types) {
            diagnostics.push(error(format!(
                "data field `{field_name}` initializer does not match declared `{}` schema",
                schema_kind(schema)
            )));
        }
    }

    fn static_initializer_value(expr: &Expr) -> Option<serde_json::Value> {
        match expr {
            Expr::Literal { value } => Some(value.clone()),
            Expr::List { items } => items
                .iter()
                .map(static_initializer_value)
                .collect::<Option<Vec<_>>>()
                .map(serde_json::Value::Array),
            Expr::Object { fields } => {
                let mut object = serde_json::Map::new();
                for (name, value) in fields {
                    object.insert(name.clone(), static_initializer_value(value)?);
                }
                Some(serde_json::Value::Object(object))
            }
            Expr::Path { .. }
            | Expr::Call { .. }
            | Expr::Eq { .. }
            | Expr::Neq { .. }
            | Expr::Lt { .. }
            | Expr::Lte { .. }
            | Expr::Gt { .. }
            | Expr::Gte { .. }
            | Expr::And { .. }
            | Expr::Or { .. }
            | Expr::Not { .. }
            | Expr::In { .. } => None,
        }
    }

    fn schema_kind(schema: &crate::schema::Schema) -> &'static str {
        match schema {
            crate::schema::Schema::String => "string",
            crate::schema::Schema::Int => "int",
            crate::schema::Schema::Float => "float",
            crate::schema::Schema::Boolean => "bool",
            crate::schema::Schema::Null => "null",
            crate::schema::Schema::Time => "time",
            crate::schema::Schema::Duration => "duration",
            crate::schema::Schema::Agent => "agent",
            crate::schema::Schema::Literal { .. } => "literal",
            crate::schema::Schema::Enum { .. } => "enum",
            crate::schema::Schema::Optional { .. } => "optional",
            crate::schema::Schema::List { .. } => "list",
            crate::schema::Schema::Set { .. } => "set",
            crate::schema::Schema::Map { .. } => "map",
            crate::schema::Schema::Union { .. } => "union",
            crate::schema::Schema::Record { .. } => "record",
            crate::schema::Schema::Ref { .. } => "ref",
            crate::schema::Schema::Json => "json",
        }
    }

    fn starts_with_ascii_uppercase(value: &str) -> bool {
        value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
    }

    fn validate_schema_refs(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        schema: &crate::schema::Schema,
        type_names: &BTreeSet<String>,
    ) {
        match schema {
            crate::schema::Schema::Ref { name } => {
                if !type_names.contains(name) {
                    diagnostics.push(error(format!(
                        "{owner} references undeclared type `{name}`"
                    )));
                }
            }
            crate::schema::Schema::Optional { inner }
            | crate::schema::Schema::List { inner }
            | crate::schema::Schema::Set { inner } => {
                validate_schema_refs(diagnostics, owner, inner, type_names);
            }
            crate::schema::Schema::Map { key, value } => {
                validate_schema_refs(diagnostics, owner, key, type_names);
                validate_schema_refs(diagnostics, owner, value, type_names);
            }
            crate::schema::Schema::Union { variants } => {
                for variant in variants {
                    validate_schema_refs(diagnostics, owner, variant, type_names);
                }
            }
            crate::schema::Schema::Record { fields } => {
                for field in fields {
                    validate_schema_refs(diagnostics, owner, &field.schema, type_names);
                }
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Enum { .. }
            | crate::schema::Schema::Json => {}
        }
    }

    fn validate_type_cycles(
        diagnostics: &mut Vec<Diagnostic>,
        types: &std::collections::BTreeMap<String, crate::schema::Schema>,
    ) {
        let mut visited = BTreeSet::new();
        let mut cyclic = BTreeSet::new();

        for type_name in types.keys() {
            let mut visiting = Vec::new();
            visit_type_refs(type_name, types, &mut visiting, &mut visited, &mut cyclic);
        }

        for type_name in cyclic {
            diagnostics.push(error(format!("type `{type_name}` has a cyclic reference")));
        }
    }

    fn visit_type_refs(
        type_name: &str,
        types: &std::collections::BTreeMap<String, crate::schema::Schema>,
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
        cyclic: &mut BTreeSet<String>,
    ) {
        if let Some(cycle_start) = visiting.iter().position(|name| name == type_name) {
            cyclic.extend(visiting[cycle_start..].iter().cloned());
            return;
        }
        if visited.contains(type_name) {
            return;
        }

        visiting.push(type_name.to_string());
        if let Some(schema) = types.get(type_name) {
            let mut refs = Vec::new();
            collect_schema_refs(schema, &mut refs);
            for ref_name in refs {
                if types.contains_key(ref_name) {
                    visit_type_refs(ref_name, types, visiting, visited, cyclic);
                }
            }
        }
        visiting.pop();
        visited.insert(type_name.to_string());
    }

    fn collect_schema_refs<'a>(schema: &'a crate::schema::Schema, refs: &mut Vec<&'a str>) {
        match schema {
            crate::schema::Schema::Ref { name } => refs.push(name),
            crate::schema::Schema::Optional { inner }
            | crate::schema::Schema::List { inner }
            | crate::schema::Schema::Set { inner } => {
                collect_schema_refs(inner, refs);
            }
            crate::schema::Schema::Map { key, value } => {
                collect_schema_refs(key, refs);
                collect_schema_refs(value, refs);
            }
            crate::schema::Schema::Union { variants } => {
                for variant in variants {
                    collect_schema_refs(variant, refs);
                }
            }
            crate::schema::Schema::Record { fields } => {
                for field in fields {
                    collect_schema_refs(&field.schema, refs);
                }
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Enum { .. }
            | crate::schema::Schema::Json => {}
        }
    }

    fn collect_state_names(
        diagnostics: &mut Vec<Diagnostic>,
        names: &mut BTreeSet<String>,
        states: &std::collections::BTreeMap<String, State>,
    ) {
        for (name, state) in states {
            if !names.insert(name.clone()) {
                diagnostics.push(error(format!(
                    "state name `{name}` is declared more than once; v0 requires globally unique state names"
                )));
            }
            collect_state_names(diagnostics, names, &state.states);
        }
    }

    fn validate_state(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        state: &State,
        ir: &WorkflowIr,
        state_names: &BTreeSet<String>,
        event_names: &BTreeSet<String>,
    ) {
        let mut unguarded_handlers = BTreeSet::new();
        for handler in &state.on {
            let base_scope =
                ExprScope::with_event_binding(&handler.event, handler.binding.as_deref());
            if let Some(guard) = &handler.guard {
                validate_expr(
                    diagnostics,
                    &format!("state `{state_name}` handler `{}` guard", handler.event),
                    ir,
                    guard,
                    &base_scope,
                    &handler.span,
                );
                validate_boolean_expr(
                    diagnostics,
                    &format!("state `{state_name}` handler `{}` guard", handler.event),
                    ir,
                    guard,
                    &base_scope,
                    &handler.span,
                );
            } else if !unguarded_handlers.insert(handler.event.clone()) {
                diagnostics.push(error_at(
                    format!(
                        "state `{state_name}` declares multiple unguarded handlers for event `{}`",
                        handler.event
                    ),
                    &handler.span,
                ));
            }

            if !event_names.contains(&handler.event) {
                diagnostics.push(error_at(
                    format!(
                        "state `{state_name}` handles undeclared event `{}`",
                        handler.event
                    ),
                    &handler.span,
                ));
            }

            if let Some(target) = &handler.transition {
                if !state_names.contains(target) {
                    diagnostics.push(error_at(
                        format!("state `{state_name}` transitions to undeclared state `{target}`"),
                        &handler.span,
                    ));
                }
            }

            let mut scope = base_scope;
            if let Some(guard) = &handler.guard {
                scope.apply_guard_refinements(guard);
            }
            validate_steps(
                diagnostics,
                state_name,
                ir,
                state_names,
                &handler.steps,
                &mut scope,
            );
        }

        for transition in &state.always {
            if let Some(guard) = &transition.guard {
                let scope = ExprScope::default();
                validate_expr(
                    diagnostics,
                    &format!("state `{state_name}` always guard"),
                    ir,
                    guard,
                    &scope,
                    &transition.span,
                );
                validate_boolean_expr(
                    diagnostics,
                    &format!("state `{state_name}` always guard"),
                    ir,
                    guard,
                    &scope,
                    &transition.span,
                );
            }

            if !state_names.contains(&transition.transition) {
                diagnostics.push(error_at(
                    format!(
                        "state `{state_name}` transitions to undeclared state `{}`",
                        transition.transition
                    ),
                    &transition.span,
                ));
            }

            let mut scope = ExprScope::default();
            validate_steps(
                diagnostics,
                state_name,
                ir,
                state_names,
                &transition.steps,
                &mut scope,
            );
        }

        if let Some(initial) = &state.initial {
            if !state.states.contains_key(initial) {
                diagnostics.push(error_at(
                    format!("state `{state_name}` initial child `{initial}` is not declared"),
                    &state.span,
                ));
            }
        }

        let mut scope = ExprScope::default();
        validate_steps(
            diagnostics,
            state_name,
            ir,
            state_names,
            &state.entry,
            &mut scope,
        );

        for (child_name, child) in &state.states {
            validate_state(diagnostics, child_name, child, ir, state_names, event_names);
        }
    }

    fn validate_bounded_start_completion_convention(
        diagnostics: &mut Vec<Diagnostic>,
        ir: &WorkflowIr,
    ) {
        let bounded_started_agents = bounded_started_agents(ir);
        if bounded_started_agents.is_empty() {
            return;
        }

        let has_finished_name = ir
            .events
            .get("finished")
            .is_some_and(|event| schema_has_required_string_field(ir, &event.payload, "name", 0));
        let handled_events = handled_event_names(&ir.statechart.states);
        let handles_finished = handled_events.contains("finished");

        for agent in bounded_started_agents {
            if !has_finished_name {
                diagnostics.push(error(format!(
                    "bounded start of agent `{agent}` requires event `finished` with required string field `name` for active invocation accounting"
                )));
            }
            if !handles_finished {
                diagnostics.push(error(format!(
                    "bounded start of agent `{agent}` requires at least one `finished` handler so completions can be processed"
                )));
            }
        }
    }

    fn bounded_started_agents(ir: &WorkflowIr) -> BTreeSet<String> {
        let mut agents = BTreeSet::new();
        collect_bounded_started_agents_in_states(ir, &ir.statechart.states, &mut agents);
        agents
    }

    fn collect_bounded_started_agents_in_states(
        ir: &WorkflowIr,
        states: &std::collections::BTreeMap<String, State>,
        agents: &mut BTreeSet<String>,
    ) {
        for state in states.values() {
            collect_bounded_started_agents_in_steps(ir, &state.entry, agents);
            for handler in &state.on {
                collect_bounded_started_agents_in_steps(ir, &handler.steps, agents);
            }
            for transition in &state.always {
                collect_bounded_started_agents_in_steps(ir, &transition.steps, agents);
            }
            collect_bounded_started_agents_in_states(ir, &state.states, agents);
        }
    }

    fn collect_bounded_started_agents_in_steps(
        ir: &WorkflowIr,
        steps: &[crate::ir::Step],
        agents: &mut BTreeSet<String>,
    ) {
        for step in steps {
            if step.effect == "start" {
                if let Some(agent) = step.args.get("agent").and_then(|value| value.as_str()) {
                    if ir
                        .agents
                        .get(agent)
                        .is_some_and(|agent| agent.max_active.is_some())
                    {
                        agents.insert(agent.to_string());
                    }
                }
            }
            if step.effect == "case" {
                for arm in &step.case_arms {
                    collect_bounded_started_agents_in_steps(ir, &arm.steps, agents);
                }
            }
        }
    }

    fn handled_event_names(states: &std::collections::BTreeMap<String, State>) -> BTreeSet<String> {
        let mut events = BTreeSet::new();
        collect_handled_event_names(states, &mut events);
        events
    }

    fn collect_handled_event_names(
        states: &std::collections::BTreeMap<String, State>,
        events: &mut BTreeSet<String>,
    ) {
        for state in states.values() {
            for handler in &state.on {
                events.insert(handler.event.clone());
            }
            collect_handled_event_names(&state.states, events);
        }
    }

    fn schema_has_required_string_field(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
        field_name: &str,
        depth: usize,
    ) -> bool {
        if depth > 16 {
            return false;
        }

        match schema {
            crate::schema::Schema::Record { fields } => fields
                .iter()
                .find(|field| field.name == field_name)
                .is_some_and(|field| matches!(field.schema, crate::schema::Schema::String)),
            crate::schema::Schema::Ref { name } => ir.types.get(name).is_some_and(|schema| {
                schema_has_required_string_field(ir, schema, field_name, depth + 1)
            }),
            crate::schema::Schema::Union { variants } => variants.iter().all(|variant| {
                schema_has_required_string_field(ir, variant, field_name, depth + 1)
            }),
            _ => false,
        }
    }

    fn validate_steps(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        state_names: &BTreeSet<String>,
        steps: &[crate::ir::Step],
        scope: &mut ExprScope,
    ) {
        const KNOWN_EFFECTS: &[&str] = &[
            "assign",
            "let",
            "case",
            "goto",
            "send",
            "start",
            "coerce",
            "askHuman",
            "raise",
            "capability_call",
        ];

        for step in steps {
            validate_step_expressions(diagnostics, state_name, ir, step, scope);

            if !KNOWN_EFFECTS.contains(&step.effect.as_str()) {
                diagnostics.push(error_at(
                    format!("state `{state_name}` uses unknown effect `{}`", step.effect),
                    &step.span,
                ));
            }

            if step.effect == "assign" {
                validate_assign_step(diagnostics, state_name, ir, step, scope);
            }

            if matches!(step.effect.as_str(), "start" | "send") {
                validate_agent_step(diagnostics, state_name, ir, step);
            }

            if step.effect == "capability_call" {
                validate_capability_step(diagnostics, state_name, ir, step);
            }

            if step.effect == "case" {
                validate_case_step(diagnostics, state_name, ir, state_names, step, scope);
            }

            if step.effect == "goto" {
                validate_goto_step(diagnostics, state_name, state_names, step);
            }

            if step.effect == "raise" {
                validate_raise_step(diagnostics, state_name, ir, step, scope);
            }

            if step.effect == "let" {
                if let Some(local) = &step.assign {
                    let schema = infer_step_local_schema(ir, scope, step);
                    scope.locals.insert(local.clone(), schema);
                }
            }
        }
    }

    fn validate_step_expressions(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        scope: &ExprScope,
    ) {
        match step.effect.as_str() {
            "assign" | "let" => {
                validate_expr_arg(diagnostics, state_name, ir, step, "value", scope)
            }
            "case" => validate_expr_arg(diagnostics, state_name, ir, step, "expr", scope),
            "send" => {
                validate_expr_arg(diagnostics, state_name, ir, step, "message", scope);
                validate_step_arg_schema(
                    diagnostics,
                    state_name,
                    ir,
                    step,
                    "message",
                    scope,
                    &crate::schema::Schema::String,
                );
            }
            "askHuman" => {
                validate_expr_arg(diagnostics, state_name, ir, step, "reason", scope);
                validate_step_arg_schema(
                    diagnostics,
                    state_name,
                    ir,
                    step,
                    "reason",
                    scope,
                    &crate::schema::Schema::String,
                );
            }
            "start" => validate_expr_arg(diagnostics, state_name, ir, step, "input", scope),
            "raise" => validate_expr_arg(diagnostics, state_name, ir, step, "payload", scope),
            "capability_call" => validate_call_args(diagnostics, state_name, ir, step, scope),
            _ => {}
        }
    }

    fn validate_step_arg_schema(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        arg_name: &str,
        scope: &ExprScope,
        expected: &crate::schema::Schema,
    ) {
        let Some(value) = step.args.get(arg_name) else {
            return;
        };
        let Ok(expr) = serde_json::from_value::<Expr>(value.clone()) else {
            return;
        };
        let Some(actual) = infer_expr_schema(ir, scope, &expr) else {
            return;
        };

        if !schema_accepts_schema(ir, expected, &actual, 0) {
            diagnostics.push(error_at(
                format!(
                    "state `{state_name}` `{}` {arg_name} has `{}` value; expected `{}`",
                    step.effect,
                    schema_kind(&actual),
                    schema_kind(expected)
                ),
                &step.span,
            ));
        }
    }

    fn validate_expr_arg(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        arg_name: &str,
        scope: &ExprScope,
    ) {
        let Some(value) = step.args.get(arg_name) else {
            return;
        };

        match serde_json::from_value::<Expr>(value.clone()) {
            Ok(expr) => validate_expr(
                diagnostics,
                &format!("state `{state_name}` `{}` {arg_name}", step.effect),
                ir,
                &expr,
                scope,
                &step.span,
            ),
            Err(source) => diagnostics.push(error_at(
                format!(
                    "state `{state_name}` `{}` {arg_name} is not a valid expression: {source}",
                    step.effect
                ),
                &step.span,
            )),
        }
    }

    fn validate_call_args(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        scope: &ExprScope,
    ) {
        let Some(value) = step.args.get("call_args") else {
            return;
        };

        let Some(values) = value.as_array() else {
            diagnostics.push(error_at(
                format!(
                    "state `{state_name}` capability call arguments are not a valid expression list"
                ),
                &step.span,
            ));
            return;
        };

        for (index, value) in values.iter().enumerate() {
            match serde_json::from_value::<Expr>(value.clone()) {
                Ok(expr) => validate_expr(
                    diagnostics,
                    &format!("state `{state_name}` capability call argument {index}"),
                    ir,
                    &expr,
                    scope,
                    &step.span,
                ),
                Err(source) => diagnostics.push(error_at(
                    format!(
                        "state `{state_name}` capability call argument {index} is not a valid expression: {source}"
                    ),
                    &step.span,
                )),
            }
        }
    }

    fn validate_goto_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        state_names: &BTreeSet<String>,
        step: &crate::ir::Step,
    ) {
        let Some(target) = step.args.get("target").and_then(|value| value.as_str()) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses goto without target"),
                &step.span,
            ));
            return;
        };

        if !state_names.contains(target) {
            diagnostics.push(error_at(
                format!("state `{state_name}` goto targets undeclared state `{target}`"),
                &step.span,
            ));
        }
    }

    fn validate_case_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        state_names: &BTreeSet<String>,
        step: &crate::ir::Step,
        scope: &ExprScope,
    ) {
        if !step.args.contains_key("expr") {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses case without scrutinee expression"),
                &step.span,
            ));
        }

        let scrutinee = step
            .args
            .get("expr")
            .and_then(|value| serde_json::from_value::<Expr>(value.clone()).ok());
        let mut nil_previously_matched = false;
        for arm in &step.case_arms {
            if let Some(target) = &arm.transition {
                if !state_names.contains(target) {
                    diagnostics.push(error_at(
                        format!(
                            "state `{state_name}` case arm transitions to undeclared state `{target}`"
                        ),
                        &step.span,
                    ));
                }
            }

            let mut arm_scope = scope.clone();
            if let Some(scrutinee) = &scrutinee {
                arm_scope.apply_case_pattern_refinements(
                    scrutinee,
                    &arm.pattern,
                    nil_previously_matched,
                );
            }
            validate_steps(
                diagnostics,
                state_name,
                ir,
                state_names,
                &arm.steps,
                &mut arm_scope,
            );
            nil_previously_matched |= case_pattern_matches_nil(&arm.pattern);
        }
    }

    fn validate_expr(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        expr: &Expr,
        scope: &ExprScope,
        span: &Option<SourceSpan>,
    ) {
        match expr {
            Expr::Literal { .. } => {}
            Expr::Path { path } => validate_path(diagnostics, owner, ir, scope, path, span),
            Expr::Eq { left, right } | Expr::Neq { left, right } => {
                validate_expr(diagnostics, owner, ir, left, scope, span);
                validate_expr(diagnostics, owner, ir, right, scope, span);
                validate_equality_expr(diagnostics, owner, ir, left, right, scope, span);
            }
            Expr::Lt { left, right }
            | Expr::Lte { left, right }
            | Expr::Gt { left, right }
            | Expr::Gte { left, right } => {
                validate_expr(diagnostics, owner, ir, left, scope, span);
                validate_expr(diagnostics, owner, ir, right, scope, span);
            }
            Expr::In { left, right } => {
                validate_expr(diagnostics, owner, ir, left, scope, span);
                validate_expr(diagnostics, owner, ir, right, scope, span);
                validate_in_expr(diagnostics, owner, ir, left, right, scope, span);
            }
            Expr::And { exprs } | Expr::Or { exprs } => {
                for expr in exprs {
                    validate_expr(diagnostics, owner, ir, expr, scope, span);
                    validate_boolean_expr(diagnostics, owner, ir, expr, scope, span);
                }
            }
            Expr::Not { expr } => {
                validate_expr(diagnostics, owner, ir, expr, scope, span);
                validate_boolean_expr(diagnostics, owner, ir, expr, scope, span);
            }
            Expr::Call { name, args } => {
                for arg in args {
                    validate_expr(diagnostics, owner, ir, arg, scope, span);
                }
                validate_call(diagnostics, owner, ir, scope, name, args, span);
            }
            Expr::Object { fields } => {
                for expr in fields.values() {
                    validate_expr(diagnostics, owner, ir, expr, scope, span);
                }
            }
            Expr::List { items } => {
                for expr in items {
                    validate_expr(diagnostics, owner, ir, expr, scope, span);
                }
            }
        }
    }

    fn validate_boolean_expr(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        expr: &Expr,
        scope: &ExprScope,
        span: &Option<SourceSpan>,
    ) {
        let Some(schema) = infer_expr_schema(ir, scope, expr) else {
            return;
        };

        if !schema_accepts_schema(ir, &crate::schema::Schema::Boolean, &schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "{owner} uses `{}` expression where `bool` is required",
                    schema_kind(&schema)
                ),
                span,
            ));
        }
    }

    fn validate_in_expr(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        left: &Expr,
        right: &Expr,
        scope: &ExprScope,
        span: &Option<SourceSpan>,
    ) {
        let Some(right_schema) = infer_expr_schema(ir, scope, right) else {
            return;
        };
        let Some(item_schema) = collection_item_schema(ir, &right_schema) else {
            diagnostics.push(error_at(
                format!(
                    "{owner} uses `in` with `{}` collection; expected list, set, or map",
                    schema_kind(&right_schema)
                ),
                span,
            ));
            return;
        };

        let Some(left_schema) = infer_expr_schema(ir, scope, left) else {
            return;
        };
        if !schema_accepts_schema(ir, &item_schema, &left_schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "{owner} uses `in` with `{}` item; expected `{}`",
                    schema_kind(&left_schema),
                    schema_kind(&item_schema)
                ),
                span,
            ));
        }
    }

    fn validate_equality_expr(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        left: &Expr,
        right: &Expr,
        scope: &ExprScope,
        span: &Option<SourceSpan>,
    ) {
        let Some(left_schema) = infer_expr_schema(ir, scope, left) else {
            return;
        };
        let Some(right_schema) = infer_expr_schema(ir, scope, right) else {
            return;
        };

        if schema_accepts_schema(ir, &left_schema, &right_schema, 0)
            || schema_accepts_schema(ir, &right_schema, &left_schema, 0)
        {
            return;
        }

        diagnostics.push(error_at(
            format!(
                "{owner} compares incompatible `{}` and `{}` expressions",
                schema_kind(&left_schema),
                schema_kind(&right_schema)
            ),
            span,
        ));
    }

    fn collection_item_schema(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
    ) -> Option<crate::schema::Schema> {
        match schema {
            crate::schema::Schema::Json => Some(crate::schema::Schema::Json),
            crate::schema::Schema::List { inner } | crate::schema::Schema::Set { inner } => {
                Some((**inner).clone())
            }
            crate::schema::Schema::Map { key, .. } => Some((**key).clone()),
            crate::schema::Schema::Optional { inner } => collection_item_schema(ir, inner),
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .and_then(|schema| collection_item_schema(ir, schema)),
            crate::schema::Schema::Union { variants } => {
                let item_schemas = variants
                    .iter()
                    .map(|schema| collection_item_schema(ir, schema))
                    .collect::<Option<Vec<_>>>()?;
                Some(infer_list_item_schema(item_schemas))
            }
            crate::schema::Schema::String
            | crate::schema::Schema::Int
            | crate::schema::Schema::Float
            | crate::schema::Schema::Boolean
            | crate::schema::Schema::Null
            | crate::schema::Schema::Time
            | crate::schema::Schema::Duration
            | crate::schema::Schema::Agent
            | crate::schema::Schema::Literal { .. }
            | crate::schema::Schema::Enum { .. }
            | crate::schema::Schema::Record { .. } => None,
        }
    }

    fn map_key_schema(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
    ) -> Option<crate::schema::Schema> {
        match schema {
            crate::schema::Schema::Json => Some(crate::schema::Schema::Json),
            crate::schema::Schema::Map { key, .. } => Some((**key).clone()),
            crate::schema::Schema::Optional { inner } => map_key_schema(ir, inner),
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .and_then(|schema| map_key_schema(ir, schema)),
            crate::schema::Schema::Union { variants } => variants
                .iter()
                .find_map(|schema| map_key_schema(ir, schema)),
            _ => None,
        }
    }

    fn map_value_schema(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
    ) -> Option<crate::schema::Schema> {
        match schema {
            crate::schema::Schema::Json => Some(crate::schema::Schema::Json),
            crate::schema::Schema::Map { value, .. } => Some((**value).clone()),
            crate::schema::Schema::Optional { inner } => map_value_schema(ir, inner),
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .and_then(|schema| map_value_schema(ir, schema)),
            crate::schema::Schema::Union { variants } => variants
                .iter()
                .find_map(|schema| map_value_schema(ir, schema)),
            _ => None,
        }
    }

    fn validate_path(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        scope: &ExprScope,
        path: &str,
        span: &Option<SourceSpan>,
    ) {
        let Some((root, rest)) = path.split_once('.') else {
            return;
        };

        if root == "data" {
            let Some(field) = rest.split('.').next().filter(|field| !field.is_empty()) else {
                diagnostics.push(error_at(
                    format!("{owner} references invalid data path `{path}`"),
                    span,
                ));
                return;
            };

            let Some(schema) = ir.context_schema.get(field) else {
                diagnostics.push(error_at(
                    format!("{owner} references undeclared data field `{field}` in path `{path}`"),
                    span,
                ));
                return;
            };

            if let Some((_, nested_path)) = rest.split_once('.') {
                let context = SchemaPathValidation {
                    owner,
                    ir,
                    scope,
                    full_path: path,
                    span,
                };
                let current_path = format!("data.{field}");
                validate_schema_path_inner(
                    diagnostics,
                    &context,
                    schema,
                    nested_path,
                    &current_path,
                    0,
                );
            }
            return;
        }

        if scope.event_binding.as_deref() == Some(root) {
            if let Some(event_name) = &scope.event_name {
                if let Some(event) = ir.events.get(event_name) {
                    let context = SchemaPathValidation {
                        owner,
                        ir,
                        scope,
                        span,
                        full_path: path,
                    };
                    validate_schema_path_inner(
                        diagnostics,
                        &context,
                        &event.payload,
                        rest,
                        root,
                        0,
                    );
                }
            }
            return;
        }

        if let Some(schema) = scope.locals.get(root) {
            let context = SchemaPathValidation {
                owner,
                ir,
                scope,
                span,
                full_path: path,
            };
            validate_schema_path_inner(diagnostics, &context, schema, rest, root, 0);
            return;
        }

        diagnostics.push(error_at(
            format!("{owner} references unknown path root `{root}` in path `{path}`"),
            span,
        ));
    }

    struct SchemaPathValidation<'a> {
        owner: &'a str,
        ir: &'a WorkflowIr,
        scope: &'a ExprScope,
        full_path: &'a str,
        span: &'a Option<SourceSpan>,
    }

    fn validate_schema_path_inner(
        diagnostics: &mut Vec<Diagnostic>,
        context: &SchemaPathValidation<'_>,
        schema: &crate::schema::Schema,
        nested_path: &str,
        current_path: &str,
        depth: usize,
    ) {
        if nested_path.is_empty() || depth > 16 {
            return;
        }

        match schema {
            crate::schema::Schema::Optional { inner } => {
                if !context.scope.non_null_paths.contains(current_path) {
                    diagnostics.push(error_at(
                        format!(
                            "{} references nested field through optional path `{current_path}` in `{}`; guard `{current_path} != nil` first",
                            context.owner, context.full_path
                        ),
                        context.span,
                    ));
                }
                validate_schema_path_inner(
                    diagnostics,
                    context,
                    inner,
                    nested_path,
                    current_path,
                    depth + 1,
                );
            }
            crate::schema::Schema::Ref { name } => {
                if let Some(resolved) = context.ir.types.get(name) {
                    validate_schema_path_inner(
                        diagnostics,
                        context,
                        resolved,
                        nested_path,
                        current_path,
                        depth + 1,
                    );
                }
            }
            crate::schema::Schema::Json | crate::schema::Schema::Map { .. } => {}
            crate::schema::Schema::Record { fields } => {
                let (segment, rest) = nested_path
                    .split_once('.')
                    .map_or((nested_path, ""), |(segment, rest)| (segment, rest));
                let Some(field) = fields.iter().find(|field| field.name == segment) else {
                    diagnostics.push(error_at(
                        format!(
                            "{} references undeclared field `{segment}` in path `{}`",
                            context.owner, context.full_path
                        ),
                        context.span,
                    ));
                    return;
                };

                let field_path = format!("{current_path}.{segment}");
                validate_schema_path_inner(
                    diagnostics,
                    context,
                    &field.schema,
                    rest,
                    &field_path,
                    depth + 1,
                );
            }
            crate::schema::Schema::Union { variants } => {
                if !variants
                    .iter()
                    .any(|variant| schema_path_accepts(context.ir, variant, nested_path, depth + 1))
                {
                    diagnostics.push(error_at(
                        format!(
                            "{} references field path `{nested_path}` that is not valid for `{}`",
                            context.owner, context.full_path
                        ),
                        context.span,
                    ));
                }
            }
            _ => diagnostics.push(error_at(
                format!(
                    "{} references nested field through non-record path `{}`",
                    context.owner, context.full_path
                ),
                context.span,
            )),
        }
    }

    fn schema_path_accepts(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
        nested_path: &str,
        depth: usize,
    ) -> bool {
        if nested_path.is_empty() || depth > 16 {
            return true;
        }

        match schema {
            crate::schema::Schema::Optional { inner } => {
                schema_path_accepts(ir, inner, nested_path, depth + 1)
            }
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .is_some_and(|schema| schema_path_accepts(ir, schema, nested_path, depth + 1)),
            crate::schema::Schema::Json | crate::schema::Schema::Map { .. } => true,
            crate::schema::Schema::Record { fields } => {
                let (segment, rest) = nested_path
                    .split_once('.')
                    .map_or((nested_path, ""), |(segment, rest)| (segment, rest));
                fields
                    .iter()
                    .find(|field| field.name == segment)
                    .is_some_and(|field| schema_path_accepts(ir, &field.schema, rest, depth + 1))
            }
            crate::schema::Schema::Union { variants } => variants
                .iter()
                .any(|variant| schema_path_accepts(ir, variant, nested_path, depth + 1)),
            _ => false,
        }
    }

    fn infer_step_local_schema(
        ir: &WorkflowIr,
        scope: &ExprScope,
        step: &crate::ir::Step,
    ) -> crate::schema::Schema {
        let Some(value) = step.args.get("value") else {
            return crate::schema::Schema::Json;
        };

        serde_json::from_value::<Expr>(value.clone())
            .ok()
            .and_then(|expr| infer_expr_schema(ir, scope, &expr))
            .unwrap_or(crate::schema::Schema::Json)
    }

    fn infer_expr_schema(
        ir: &WorkflowIr,
        scope: &ExprScope,
        expr: &Expr,
    ) -> Option<crate::schema::Schema> {
        match expr {
            Expr::Literal { value } => Some(infer_literal_schema(value)),
            Expr::Path { path } => resolve_path_schema(ir, scope, path),
            Expr::Call { name, args } => infer_call_schema(ir, scope, name, args),
            Expr::Object { fields } => Some(crate::schema::Schema::Record {
                fields: fields
                    .iter()
                    .map(|(name, expr)| crate::schema::Field {
                        name: name.clone(),
                        schema: infer_expr_schema(ir, scope, expr)
                            .unwrap_or(crate::schema::Schema::Json),
                    })
                    .collect(),
            }),
            Expr::List { items } => Some(crate::schema::Schema::List {
                inner: Box::new(infer_list_item_schema(
                    items
                        .iter()
                        .filter_map(|expr| infer_expr_schema(ir, scope, expr)),
                )),
            }),
            Expr::Eq { .. }
            | Expr::Neq { .. }
            | Expr::Lt { .. }
            | Expr::Lte { .. }
            | Expr::Gt { .. }
            | Expr::Gte { .. }
            | Expr::In { .. }
            | Expr::And { .. }
            | Expr::Or { .. }
            | Expr::Not { .. } => Some(crate::schema::Schema::Boolean),
        }
    }

    fn infer_literal_schema(value: &serde_json::Value) -> crate::schema::Schema {
        match value {
            serde_json::Value::String(_)
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::Null => crate::schema::Schema::Literal {
                value: value.clone(),
            },
            serde_json::Value::Array(items) => crate::schema::Schema::List {
                inner: Box::new(infer_list_item_schema(
                    items.iter().map(infer_literal_schema),
                )),
            },
            serde_json::Value::Object(entries) => crate::schema::Schema::Record {
                fields: entries
                    .iter()
                    .map(|(name, value)| crate::schema::Field {
                        name: name.clone(),
                        schema: infer_literal_schema(value),
                    })
                    .collect(),
            },
        }
    }

    fn infer_list_item_schema(
        schemas: impl IntoIterator<Item = crate::schema::Schema>,
    ) -> crate::schema::Schema {
        let mut schemas = schemas.into_iter();
        let Some(first) = schemas.next() else {
            return crate::schema::Schema::Json;
        };

        let mut variants = vec![first];
        for schema in schemas {
            if !variants.contains(&schema) {
                variants.push(schema);
            }
        }

        if variants.len() == 1 {
            variants.remove(0)
        } else {
            crate::schema::Schema::Union { variants }
        }
    }

    fn infer_call_schema(
        ir: &WorkflowIr,
        scope: &ExprScope,
        name: &str,
        args: &[Expr],
    ) -> Option<crate::schema::Schema> {
        if let Some(function_name) = name.strip_prefix("coerce ") {
            return ir
                .coerce_functions
                .get(function_name.trim())
                .map(|function| function.output.clone());
        }

        if let Some(function) = ir.coerce_functions.get(name) {
            return Some(function.output.clone());
        }

        if name == "now" {
            return Some(crate::schema::Schema::Time);
        }

        if name == "elapsedSince" || name == "time.elapsedSince" {
            return Some(crate::schema::Schema::Duration);
        }

        match name {
            "list.length" => return Some(crate::schema::Schema::Int),
            "list.isEmpty" | "list.contains" => return Some(crate::schema::Schema::Boolean),
            "list.append" | "list.remove" => {
                return args
                    .first()
                    .and_then(|arg| infer_expr_schema(ir, scope, arg))
                    .or(Some(crate::schema::Schema::Json));
            }
            "list.first" => {
                return args
                    .first()
                    .and_then(|arg| infer_expr_schema(ir, scope, arg))
                    .and_then(|schema| collection_item_schema(ir, &schema))
                    .map(|schema| crate::schema::Schema::Optional {
                        inner: Box::new(schema),
                    })
                    .or(Some(crate::schema::Schema::Json));
            }
            "map.get" => {
                return args
                    .first()
                    .and_then(|arg| infer_expr_schema(ir, scope, arg))
                    .and_then(|schema| map_value_schema(ir, &schema))
                    .map(|schema| crate::schema::Schema::Optional {
                        inner: Box::new(schema),
                    })
                    .or(Some(crate::schema::Schema::Json));
            }
            "map.set" | "map.remove" => {
                return args
                    .first()
                    .and_then(|arg| infer_expr_schema(ir, scope, arg))
                    .or(Some(crate::schema::Schema::Json));
            }
            "map.containsKey" => return Some(crate::schema::Schema::Boolean),
            "text.trim" => return Some(crate::schema::Schema::String),
            "text.contains" | "text.startsWith" | "text.endsWith" | "text.matchesGlob" => {
                return Some(crate::schema::Schema::Boolean);
            }
            _ => {}
        }

        if let Some(receiver) = name.strip_suffix(".append") {
            return resolve_path_schema(ir, scope, receiver);
        }

        if let Some(receiver) = name.strip_suffix(".remove") {
            return resolve_path_schema(ir, scope, receiver);
        }

        Some(crate::schema::Schema::Json)
    }

    fn resolve_path_schema(
        ir: &WorkflowIr,
        scope: &ExprScope,
        path: &str,
    ) -> Option<crate::schema::Schema> {
        let (root, rest) = path.split_once('.').unwrap_or((path, ""));

        if root == "data" {
            let (field, nested) = rest.split_once('.').unwrap_or((rest, ""));
            let schema = ir.context_schema.get(field)?;
            let resolved = resolve_schema_path(ir, schema, nested, 0)?;
            return Some(refine_non_null_path(scope, path, resolved));
        }

        if scope.event_binding.as_deref() == Some(root) {
            let event_name = scope.event_name.as_ref()?;
            let schema = &ir.events.get(event_name)?.payload;
            let resolved = resolve_schema_path(ir, schema, rest, 0)?;
            return Some(refine_non_null_path(scope, path, resolved));
        }

        if let Some(schema) = scope.locals.get(root) {
            let resolved = resolve_schema_path(ir, schema, rest, 0)?;
            return Some(refine_non_null_path(scope, path, resolved));
        }

        None
    }

    fn refine_non_null_path(
        scope: &ExprScope,
        path: &str,
        schema: crate::schema::Schema,
    ) -> crate::schema::Schema {
        if scope.non_null_paths.contains(path) {
            strip_outer_optional(schema)
        } else {
            schema
        }
    }

    fn strip_outer_optional(schema: crate::schema::Schema) -> crate::schema::Schema {
        match schema {
            crate::schema::Schema::Optional { inner } => *inner,
            schema => schema,
        }
    }

    fn resolve_schema_path(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
        nested_path: &str,
        depth: usize,
    ) -> Option<crate::schema::Schema> {
        if nested_path.is_empty() {
            return Some(schema.clone());
        }
        if depth > 16 {
            return Some(crate::schema::Schema::Json);
        }

        match schema {
            crate::schema::Schema::Optional { inner } => {
                resolve_schema_path(ir, inner, nested_path, depth + 1)
            }
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .and_then(|schema| resolve_schema_path(ir, schema, nested_path, depth + 1)),
            crate::schema::Schema::Json | crate::schema::Schema::Map { .. } => {
                Some(crate::schema::Schema::Json)
            }
            crate::schema::Schema::Record { fields } => {
                let (segment, rest) = nested_path
                    .split_once('.')
                    .map_or((nested_path, ""), |(segment, rest)| (segment, rest));
                let field = fields.iter().find(|field| field.name == segment)?;
                resolve_schema_path(ir, &field.schema, rest, depth + 1)
            }
            crate::schema::Schema::Union { variants } => {
                let mut resolved = variants
                    .iter()
                    .filter_map(|variant| resolve_schema_path(ir, variant, nested_path, depth + 1))
                    .collect::<Vec<_>>();
                match resolved.len() {
                    0 => None,
                    1 => resolved.pop(),
                    _ => Some(crate::schema::Schema::Union { variants: resolved }),
                }
            }
            _ => None,
        }
    }

    fn validate_call(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        scope: &ExprScope,
        name: &str,
        args: &[Expr],
        span: &Option<SourceSpan>,
    ) {
        let arity = args.len();
        let context = CallValidation {
            owner,
            ir,
            scope,
            span,
        };
        if let Some(function_name) = name.strip_prefix("coerce ") {
            validate_coerce_call(
                diagnostics,
                owner,
                ir,
                scope,
                function_name.trim(),
                args,
                span,
            );
            return;
        }

        if ir.coerce_functions.contains_key(name) {
            validate_coerce_call(diagnostics, owner, ir, scope, name, args, span);
            return;
        }

        match name {
            "now" => {
                if arity != 0 {
                    diagnostics.push(error_at(
                        format!("{owner} calls `now` with {arity} argument(s); expected 0"),
                        span,
                    ));
                }
                return;
            }
            "elapsedSince" | "time.elapsedSince" => {
                if arity != 1 {
                    diagnostics.push(error_at(
                        format!("{owner} calls `{name}` with {arity} argument(s); expected 1"),
                        span,
                    ));
                } else if let Some(schema) = infer_expr_schema(ir, scope, &args[0]) {
                    if !schema_accepts_time(&schema) {
                        diagnostics.push(error_at(
                            format!(
                                "{owner} calls `{name}` with `{}` argument; expected `time` or `time?`",
                                schema_kind(&schema)
                            ),
                            span,
                        ));
                    }
                }
                return;
            }
            "list.length" | "list.isEmpty" | "list.first" => {
                validate_list_helper(diagnostics, &context, name, args, 1, None);
                return;
            }
            "list.contains" | "list.append" | "list.remove" => {
                validate_list_helper(diagnostics, &context, name, args, 2, Some(1));
                return;
            }
            "map.get" | "map.remove" | "map.containsKey" => {
                validate_map_helper(diagnostics, &context, name, args, 2, None);
                return;
            }
            "map.set" => {
                validate_map_helper(diagnostics, &context, name, args, 3, Some(2));
                return;
            }
            "text.trim" => {
                validate_text_helper(diagnostics, &context, name, args, 1);
                return;
            }
            "text.contains" | "text.startsWith" | "text.endsWith" | "text.matchesGlob" => {
                validate_text_helper(diagnostics, &context, name, args, 2);
                return;
            }
            _ => {}
        }

        if name.ends_with(".append") || name.ends_with(".remove") {
            let receiver = name
                .strip_suffix(".append")
                .or_else(|| name.strip_suffix(".remove"))
                .unwrap_or(name);
            validate_path(diagnostics, owner, ir, scope, receiver, span);
            if arity != 1 {
                diagnostics.push(error_at(
                    format!("{owner} calls `{name}` with {arity} argument(s); expected 1"),
                    span,
                ));
            } else if let Some(receiver_schema) = resolve_path_schema(ir, scope, receiver) {
                validate_append_argument(
                    diagnostics,
                    owner,
                    ir,
                    scope,
                    CollectionItemArgument {
                        call_name: name,
                        receiver_schema: &receiver_schema,
                        arg: &args[0],
                    },
                    span,
                );
            }
            return;
        }

        if name
            .split_once('.')
            .is_some_and(|(capability, _)| ir.capabilities.contains_key(capability))
        {
            return;
        }

        diagnostics.push(error_at(
            format!("{owner} calls unknown function `{name}`"),
            span,
        ));
    }

    struct CallValidation<'a> {
        owner: &'a str,
        ir: &'a WorkflowIr,
        scope: &'a ExprScope,
        span: &'a Option<SourceSpan>,
    }

    fn schema_accepts_time(schema: &crate::schema::Schema) -> bool {
        match schema {
            crate::schema::Schema::Time | crate::schema::Schema::Json => true,
            crate::schema::Schema::Optional { inner } => schema_accepts_time(inner),
            crate::schema::Schema::Union { variants } => variants.iter().any(schema_accepts_time),
            _ => false,
        }
    }

    fn validate_list_helper(
        diagnostics: &mut Vec<Diagnostic>,
        context: &CallValidation<'_>,
        name: &str,
        args: &[Expr],
        expected_arity: usize,
        item_arg_index: Option<usize>,
    ) {
        if args.len() != expected_arity {
            diagnostics.push(error_at(
                format!(
                    "{} calls `{name}` with {} argument(s); expected {expected_arity}",
                    context.owner,
                    args.len()
                ),
                context.span,
            ));
            return;
        }

        let Some(receiver_schema) = infer_expr_schema(context.ir, context.scope, &args[0]) else {
            return;
        };
        let Some(item_schema) = append_item_schema(context.ir, &receiver_schema) else {
            if !matches!(receiver_schema, crate::schema::Schema::Json) {
                diagnostics.push(error_at(
                    format!(
                        "{} calls `{name}` with `{}` collection; expected list or set",
                        context.owner,
                        schema_kind(&receiver_schema)
                    ),
                    context.span,
                ));
            }
            return;
        };

        if let Some(index) = item_arg_index {
            let Some(arg_schema) = infer_expr_schema(context.ir, context.scope, &args[index])
            else {
                return;
            };
            if !schema_accepts_schema(context.ir, &item_schema, &arg_schema, 0) {
                diagnostics.push(error_at(
                    format!(
                        "{} calls `{name}` with `{}` item; expected `{}`",
                        context.owner,
                        schema_kind(&arg_schema),
                        schema_kind(&item_schema)
                    ),
                    context.span,
                ));
            }
        }
    }

    fn validate_map_helper(
        diagnostics: &mut Vec<Diagnostic>,
        context: &CallValidation<'_>,
        name: &str,
        args: &[Expr],
        expected_arity: usize,
        value_arg_index: Option<usize>,
    ) {
        if args.len() != expected_arity {
            diagnostics.push(error_at(
                format!(
                    "{} calls `{name}` with {} argument(s); expected {expected_arity}",
                    context.owner,
                    args.len()
                ),
                context.span,
            ));
            return;
        }

        let Some(map_schema) = infer_expr_schema(context.ir, context.scope, &args[0]) else {
            return;
        };
        let Some(key_schema) = map_key_schema(context.ir, &map_schema) else {
            if !matches!(map_schema, crate::schema::Schema::Json) {
                diagnostics.push(error_at(
                    format!(
                        "{} calls `{name}` with `{}` map; expected map",
                        context.owner,
                        schema_kind(&map_schema)
                    ),
                    context.span,
                ));
            }
            return;
        };

        let Some(actual_key_schema) = infer_expr_schema(context.ir, context.scope, &args[1]) else {
            return;
        };
        if !schema_accepts_schema(context.ir, &key_schema, &actual_key_schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "{} calls `{name}` with `{}` key; expected `{}`",
                    context.owner,
                    schema_kind(&actual_key_schema),
                    schema_kind(&key_schema)
                ),
                context.span,
            ));
        }

        if let Some(index) = value_arg_index {
            let Some(value_schema) = map_value_schema(context.ir, &map_schema) else {
                return;
            };
            let Some(actual_value_schema) =
                infer_expr_schema(context.ir, context.scope, &args[index])
            else {
                return;
            };
            if !schema_accepts_schema(context.ir, &value_schema, &actual_value_schema, 0) {
                diagnostics.push(error_at(
                    format!(
                        "{} calls `{name}` with `{}` value; expected `{}`",
                        context.owner,
                        schema_kind(&actual_value_schema),
                        schema_kind(&value_schema)
                    ),
                    context.span,
                ));
            }
        }
    }

    fn validate_text_helper(
        diagnostics: &mut Vec<Diagnostic>,
        context: &CallValidation<'_>,
        name: &str,
        args: &[Expr],
        expected_arity: usize,
    ) {
        if args.len() != expected_arity {
            diagnostics.push(error_at(
                format!(
                    "{} calls `{name}` with {} argument(s); expected {expected_arity}",
                    context.owner,
                    args.len()
                ),
                context.span,
            ));
            return;
        }

        for arg in args {
            let Some(schema) = infer_expr_schema(context.ir, context.scope, arg) else {
                continue;
            };
            if !schema_accepts_schema(context.ir, &crate::schema::Schema::String, &schema, 0) {
                diagnostics.push(error_at(
                    format!(
                        "{} calls `{name}` with `{}` text argument; expected `string`",
                        context.owner,
                        schema_kind(&schema)
                    ),
                    context.span,
                ));
            }
        }
    }

    fn validate_append_argument(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        scope: &ExprScope,
        append: CollectionItemArgument<'_>,
        span: &Option<SourceSpan>,
    ) {
        let Some(item_schema) = append_item_schema(ir, append.receiver_schema) else {
            if !matches!(append.receiver_schema, crate::schema::Schema::Json) {
                diagnostics.push(error_at(
                    format!(
                        "{owner} calls `{}` on `{}`; expected list or set",
                        append.call_name,
                        schema_kind(append.receiver_schema)
                    ),
                    span,
                ));
            }
            return;
        };

        let Some(arg_schema) = infer_expr_schema(ir, scope, append.arg) else {
            return;
        };

        if !schema_accepts_schema(ir, &item_schema, &arg_schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "{owner} calls `{}` with `{}` item; expected `{}`",
                    append.call_name,
                    schema_kind(&arg_schema),
                    schema_kind(&item_schema)
                ),
                span,
            ));
        }
    }

    struct CollectionItemArgument<'a> {
        call_name: &'a str,
        receiver_schema: &'a crate::schema::Schema,
        arg: &'a Expr,
    }

    fn append_item_schema(
        ir: &WorkflowIr,
        schema: &crate::schema::Schema,
    ) -> Option<crate::schema::Schema> {
        match schema {
            crate::schema::Schema::List { inner } | crate::schema::Schema::Set { inner } => {
                Some((**inner).clone())
            }
            crate::schema::Schema::Optional { inner } => append_item_schema(ir, inner),
            crate::schema::Schema::Ref { name } => ir
                .types
                .get(name)
                .and_then(|schema| append_item_schema(ir, schema)),
            crate::schema::Schema::Union { variants } => variants
                .iter()
                .filter_map(|schema| append_item_schema(ir, schema))
                .next(),
            _ => None,
        }
    }

    fn schema_accepts_schema(
        ir: &WorkflowIr,
        expected: &crate::schema::Schema,
        actual: &crate::schema::Schema,
        depth: usize,
    ) -> bool {
        if depth > 16 {
            return true;
        }

        match (expected, actual) {
            (crate::schema::Schema::Json, _) | (_, crate::schema::Schema::Json) => true,
            (
                crate::schema::Schema::Optional {
                    inner: expected_inner,
                },
                crate::schema::Schema::Optional {
                    inner: actual_inner,
                },
            ) => schema_accepts_schema(ir, expected_inner, actual_inner, depth + 1),
            (crate::schema::Schema::Optional { .. }, crate::schema::Schema::Null) => true,
            (crate::schema::Schema::Optional { .. }, crate::schema::Schema::Literal { value })
                if value.is_null() =>
            {
                true
            }
            (crate::schema::Schema::Optional { inner }, _) => {
                schema_accepts_schema(ir, inner, actual, depth + 1)
            }
            (_, crate::schema::Schema::Optional { .. }) => false,
            (crate::schema::Schema::Ref { name }, _) => ir
                .types
                .get(name)
                .is_some_and(|schema| schema_accepts_schema(ir, schema, actual, depth + 1)),
            (_, crate::schema::Schema::Ref { name }) => ir
                .types
                .get(name)
                .is_some_and(|schema| schema_accepts_schema(ir, expected, schema, depth + 1)),
            (crate::schema::Schema::Union { variants }, _) => variants
                .iter()
                .any(|schema| schema_accepts_schema(ir, schema, actual, depth + 1)),
            (_, crate::schema::Schema::Union { variants }) => variants
                .iter()
                .all(|schema| schema_accepts_schema(ir, expected, schema, depth + 1)),
            (
                crate::schema::Schema::Literal { value: expected },
                crate::schema::Schema::Literal { value: actual },
            ) => expected == actual,
            (
                crate::schema::Schema::String
                | crate::schema::Schema::Time
                | crate::schema::Schema::Duration
                | crate::schema::Schema::Agent,
                crate::schema::Schema::Literal { value },
            ) => value.is_string(),
            (crate::schema::Schema::Int, crate::schema::Schema::Literal { value }) => {
                value.as_i64().is_some() || value.as_u64().is_some()
            }
            (crate::schema::Schema::Float, crate::schema::Schema::Literal { value }) => {
                value.is_number()
            }
            (crate::schema::Schema::Boolean, crate::schema::Schema::Literal { value }) => {
                value.is_boolean()
            }
            (crate::schema::Schema::Null, crate::schema::Schema::Literal { value }) => {
                value.is_null()
            }
            (crate::schema::Schema::Enum { values }, crate::schema::Schema::Literal { value }) => {
                value
                    .as_str()
                    .is_some_and(|actual| values.iter().any(|expected| expected == actual))
            }
            (crate::schema::Schema::Float, crate::schema::Schema::Int) => true,
            (
                crate::schema::Schema::List { inner: expected },
                crate::schema::Schema::List { inner: actual },
            )
            | (
                crate::schema::Schema::Set { inner: expected },
                crate::schema::Schema::Set { inner: actual },
            ) => schema_accepts_schema(ir, expected, actual, depth + 1),
            (
                crate::schema::Schema::Map {
                    key: expected_key,
                    value: expected_value,
                },
                crate::schema::Schema::Map {
                    key: actual_key,
                    value: actual_value,
                },
            ) => {
                schema_accepts_schema(ir, expected_key, actual_key, depth + 1)
                    && schema_accepts_schema(ir, expected_value, actual_value, depth + 1)
            }
            (
                crate::schema::Schema::Enum { values: expected },
                crate::schema::Schema::Enum { values: actual },
            ) => actual.iter().all(|value| expected.contains(value)),
            (
                crate::schema::Schema::Record { fields: expected },
                crate::schema::Schema::Record { fields: actual },
            ) => {
                actual.iter().all(|actual_field| {
                    expected
                        .iter()
                        .any(|expected_field| expected_field.name == actual_field.name)
                }) && expected.iter().all(|expected_field| {
                    let Some(actual_field) = actual
                        .iter()
                        .find(|actual_field| actual_field.name == expected_field.name)
                    else {
                        return schema_allows_absent(&expected_field.schema);
                    };
                    schema_accepts_schema(
                        ir,
                        &expected_field.schema,
                        &actual_field.schema,
                        depth + 1,
                    )
                })
            }
            _ => schema_kind(expected) == schema_kind(actual),
        }
    }

    fn schema_allows_absent(schema: &crate::schema::Schema) -> bool {
        match schema {
            crate::schema::Schema::Optional { .. } => true,
            crate::schema::Schema::Union { variants } => variants.iter().any(schema_allows_absent),
            _ => false,
        }
    }

    fn validate_coerce_call(
        diagnostics: &mut Vec<Diagnostic>,
        owner: &str,
        ir: &WorkflowIr,
        scope: &ExprScope,
        function_name: &str,
        args: &[Expr],
        span: &Option<SourceSpan>,
    ) {
        if function_name.is_empty() {
            diagnostics.push(error_at(
                format!("{owner} has empty coerce function name"),
                span,
            ));
            return;
        }

        let Some(function) = ir.coerce_functions.get(function_name) else {
            diagnostics.push(error_at(
                format!("{owner} calls undeclared coerce function `{function_name}`"),
                span,
            ));
            return;
        };

        let arity = args.len();
        if function.params.len() != arity {
            diagnostics.push(error_at(
                format!(
                    "{owner} calls coerce function `{function_name}` with {arity} argument(s); expected {}",
                    function.params.len()
                ),
                span,
            ));
            return;
        }

        for (index, (param, arg)) in function.params.iter().zip(args).enumerate() {
            let Some(actual) = infer_expr_schema(ir, scope, arg) else {
                continue;
            };
            if !schema_accepts_schema(ir, &param.schema, &actual, 0) {
                diagnostics.push(error_at(
                    format!(
                        "{owner} calls coerce function `{function_name}` argument {index} `{}` with `{}` value; expected `{}`",
                        param.name,
                        schema_kind(&actual),
                        schema_kind(&param.schema)
                    ),
                    span,
                ));
            }
        }
    }

    fn validate_agent_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
    ) {
        let Some(agent) = step.args.get("agent").and_then(|value| value.as_str()) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses `{}` without agent", step.effect),
                &step.span,
            ));
            return;
        };

        let Some(agent_decl) = ir.agents.get(agent) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses undeclared agent `{agent}`"),
                &step.span,
            ));
            return;
        };

        if step.effect == "start"
            && matches!(agent_decl.target, crate::ir::AgentTarget::Thread { .. })
        {
            diagnostics.push(error_at(
                format!("state `{state_name}` starts thread agent `{agent}`; use `send` for thread agents or declare a codingAgent/adapter target"),
                &step.span,
            ));
        }
    }

    fn validate_raise_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        scope: &ExprScope,
    ) {
        let Some(event) = step.args.get("event").and_then(|value| value.as_str()) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses raise without event"),
                &step.span,
            ));
            return;
        };

        let Some(event_schema) = ir.events.get(event).map(|event| &event.payload) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` raises undeclared event `{event}`"),
                &step.span,
            ));
            return;
        };

        let payload_schema = step
            .args
            .get("payload")
            .and_then(|value| serde_json::from_value::<Expr>(value.clone()).ok())
            .and_then(|expr| infer_expr_schema(ir, scope, &expr))
            .unwrap_or(crate::schema::Schema::Record { fields: Vec::new() });

        if !schema_accepts_schema(ir, event_schema, &payload_schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "state `{state_name}` raises event `{event}` with `{}` payload; expected `{}`",
                    schema_kind(&payload_schema),
                    schema_kind(event_schema)
                ),
                &step.span,
            ));
        }
    }

    fn validate_capability_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
    ) {
        let Some(capability) = step.args.get("capability").and_then(|value| value.as_str()) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses capability call without capability"),
                &step.span,
            ));
            return;
        };

        if !ir.capabilities.contains_key(capability) {
            diagnostics.push(error_at(
                format!("state `{state_name}` uses undeclared capability `{capability}`"),
                &step.span,
            ));
        }
    }

    fn validate_assign_step(
        diagnostics: &mut Vec<Diagnostic>,
        state_name: &str,
        ir: &WorkflowIr,
        step: &crate::ir::Step,
        scope: &ExprScope,
    ) {
        let Some(target) = step.args.get("target").and_then(|value| value.as_str()) else {
            diagnostics.push(error_at(
                format!("state `{state_name}` has assign step without string target"),
                &step.span,
            ));
            return;
        };

        let Some(data_path) = target.strip_prefix("data.") else {
            diagnostics.push(error_at(
                format!("state `{state_name}` assigns undeclared non-data path `{target}`"),
                &step.span,
            ));
            return;
        };

        let Some(field_name) = data_path.split('.').next() else {
            diagnostics.push(error_at(
                format!("state `{state_name}` assigns invalid data path `{target}`"),
                &step.span,
            ));
            return;
        };

        if !ir.context_schema.contains_key(field_name) {
            diagnostics.push(error_at(
                format!("state `{state_name}` assigns undeclared data field `{field_name}`"),
                &step.span,
            ));
            return;
        }

        let Some(target_schema) = resolve_path_schema(ir, scope, target) else {
            return;
        };
        let Some(value) = step.args.get("value") else {
            return;
        };
        let Ok(expr) = serde_json::from_value::<Expr>(value.clone()) else {
            return;
        };
        let Some(value_schema) = infer_expr_schema(ir, scope, &expr) else {
            return;
        };

        if !schema_accepts_schema(ir, &target_schema, &value_schema, 0) {
            diagnostics.push(error_at(
                format!(
                    "state `{state_name}` assigns `{}` value to `{target}`; expected `{}`",
                    schema_kind(&value_schema),
                    schema_kind(&target_schema)
                ),
                &step.span,
            ));
        }
    }

    fn error(message: String) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message,
            span: None,
        }
    }

    fn error_at(message: String, span: &Option<SourceSpan>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message,
            span: span.clone(),
        }
    }
}

pub use diagnostics::{Diagnostic, Severity, SourceSpan};
pub use ir::WorkflowIr;
pub use source::{
    parse_source, parse_source_with_file, parse_syntax, parse_syntax_with_file, ParsedSource,
    SourceError,
};
pub use validate::{validate_ir, ValidationReport};

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn lexes_minimal_source_with_trivia() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let parsed = crate::parse_syntax(source);

        assert!(parsed.syntax.tokens.iter().any(|token| {
            token.kind == crate::syntax::SyntaxKind::Newline
                || token.kind == crate::syntax::SyntaxKind::Whitespace
        }));
        assert!(parsed.syntax.green.children().next().is_some());
    }

    #[test]
    fn parses_minimal_source_to_golden_ir() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = crate::parse_source(source).expect("minimal source parses");
        let report = crate::validate_ir(&ir);
        let mut actual = serde_json::to_value(&ir).expect("ir serializes");
        null_spans(&mut actual);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(
            actual,
            json!({
                "schema_version": "statechart-workflow-ir/v0",
                "workflow": {
                    "name": "Minimal",
                    "source_path": null,
                    "repo": null,
                    "contracts": [],
                    "plan": null,
                    "state_scope": null
                },
                "agents": {},
                "events": {
                    "start": {
                        "payload": {
                            "type": "record",
                            "fields": [
                                {
                                    "name": "message",
                                    "schema": {"type": "string"}
                                }
                            ]
                        },
                        "span": null
                    }
                },
                "capabilities": {},
                "context_schema": {
                    "lastMessage": {
                        "type": "optional",
                        "inner": {"type": "string"}
                    }
                },
                "context_initializers": {
                    "lastMessage": {
                        "op": "literal",
                        "value": null
                    }
                },
                "types": {},
                "coerce_functions": {},
                "statechart": {
                    "initial": "waiting",
                    "states": {
                        "complete": {
                            "initial": null,
                            "on": [],
                            "entry": [],
                            "always": [],
                            "states": {},
                            "final_state": true,
                            "span": null
                        },
                        "waiting": {
                            "initial": null,
                            "on": [
                                {
                                    "event": "start",
                                    "binding": "evt",
                                    "guard": null,
                                    "steps": [
                                        {
                                            "effect": "assign",
                                            "args": {
                                                "target": "data.lastMessage",
                                                "value": {
                                                    "op": "path",
                                    "path": "evt.message"
                                                }
                                            },
                                            "assign": null,
                                            "span": null
                                        }
                                    ],
                                    "transition": "complete",
                                    "span": null
                                }
                            ],
                            "entry": [],
                            "always": [],
                            "states": {},
                            "final_state": false,
                            "span": null
                        }
                    }
                },
                "invariants": [
                    {
                        "type": "builtin",
                        "name": "declaredEffectsOnly",
                        "span": null
                    }
                ]
            })
        );
    }

    fn null_spans(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if object.contains_key("span") {
                    object.insert("span".to_string(), serde_json::Value::Null);
                }
                for value in object.values_mut() {
                    null_spans(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    null_spans(value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn parses_expression_invariant_blocks() {
        let source = r#"
machine InvariantBlocks
initial done

data {
  count int = 0
}

state done {
  final
}

invariant countWithinBound {
  assert data.count <= 3
}
"#;
        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(ir.invariants.len(), 1);
        match &ir.invariants[0] {
            crate::ir::Invariant::Expression { name, span, .. } => {
                assert_eq!(name, "countWithinBound");
                assert_eq!(span.as_ref().map(|span| span.start_line), Some(13));
            }
            invariant => panic!("expected expression invariant, got {invariant:#?}"),
        }
    }

    #[test]
    fn parses_top_level_agents_capabilities_types_and_coerce() {
        let source = r#"
machine TopLevel
initial done

agent director = thread("director")
agent external = adapter("untie")
agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 4
}

capability plan = adapter("implementationPlan")

enum RunKind {
  WorkerComplete
  WorkerFailed
}

class RunSummary {
  id string
  exitCode int?
}

class RunClassification {
  kind RunKind
  reason string
}

class TypeShapes {
  counts map<string, int>
  decision "yes" | "no"
  optionalDecision ("yes" | "no")?
  enabled true
}

coerce classifyRun(run RunSummary) -> RunClassification {
  model "gpt-4o-mini"
  prompt """
  Classify the run.
  """
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(
            ir.agents["director"].target,
            crate::ir::AgentTarget::Thread {
                name: "director".to_string()
            }
        );
        assert_eq!(
            ir.agents["external"].target,
            crate::ir::AgentTarget::Adapter {
                name: "untie".to_string()
            }
        );
        assert_eq!(
            ir.capabilities["plan"].adapter,
            "implementationPlan".to_string()
        );
        assert_eq!(ir.agents["worker"].max_active, Some(4));
        assert_eq!(ir.agents["worker"].profile.as_deref(), Some("repo-writer"));
        assert!(matches!(
            &ir.types["RunKind"],
            crate::schema::Schema::Enum { values } if values == &vec![
                "WorkerComplete".to_string(),
                "WorkerFailed".to_string()
            ]
        ));
        assert_eq!(ir.coerce_functions["classifyRun"].params.len(), 1);
        assert_eq!(
            ir.coerce_functions["classifyRun"].model.as_deref(),
            Some("gpt-4o-mini")
        );
        let crate::schema::Schema::Record { fields } = &ir.types["TypeShapes"] else {
            panic!("TypeShapes is a record");
        };
        assert!(matches!(
            &fields[0].schema,
            crate::schema::Schema::Map { key, value }
                if matches!(**key, crate::schema::Schema::String)
                    && matches!(**value, crate::schema::Schema::Int)
        ));
        assert!(matches!(
            &fields[1].schema,
            crate::schema::Schema::Union { variants }
                if variants.len() == 2
                    && variants.iter().all(|variant| matches!(variant, crate::schema::Schema::Literal { .. }))
        ));
        assert!(matches!(
            &fields[2].schema,
            crate::schema::Schema::Optional { inner }
                if matches!(**inner, crate::schema::Schema::Union { .. })
        ));
        assert!(matches!(
            &fields[3].schema,
            crate::schema::Schema::Literal { value } if value == &serde_json::json!(true)
        ));
    }

    #[test]
    fn validation_rejects_unknown_type_refs() {
        let source = r#"
machine BadTypes
initial done

coerce classifyRun(run MissingType) -> MissingType {
  model "gpt-4o-mini"
  prompt """
  Classify the run.
  """
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("MissingType")));
    }

    #[test]
    fn parser_rejects_duplicate_top_level_declarations() {
        let source = r#"
machine DuplicateTopLevel
machine DuplicateTopLevelAgain
initial done
initial other

data {
  item string
  item int
}

agent worker = codingAgent()
agent worker = thread("worker")

capability plan = adapter("plan")
capability plan = adapter("other")

enum Result {
  Ok
}

class Result {
  value string
}

event run {
  name string
}

event run {
  id string
}

coerce classify(input string) -> string {
  model "gpt-4o-mini"
}

coerce classify(input string) -> string {
  model "gpt-4o-mini"
}

state done {
  final
}

state done {
  final
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.ir.is_some());
        for expected in [
            "duplicate `machine` declaration",
            "duplicate top-level `initial` declaration",
            "duplicate data field `item`",
            "duplicate agent `worker`",
            "duplicate capability `plan`",
            "duplicate type `Result`",
            "duplicate event `run`",
            "duplicate coerce function `classify`",
            "duplicate state `done`",
        ] {
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected)),
                "missing diagnostic {expected}: {:#?}",
                parsed.diagnostics
            );
        }
    }

    #[test]
    fn parser_rejects_duplicate_object_fields() {
        let source = r#"
machine DuplicateObjectFields
initial waiting

data {
  item json = {}
}

event go {}

state waiting {
  on go {
    assign data.item = { status "open", status "closed" }
  }
}
"#;

        let parsed = crate::parse_syntax(source);
        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("duplicate object field `status`")));
    }

    #[test]
    fn parser_reports_spans_for_current_token_errors() {
        let parsed = crate::parse_syntax(
            r#"machine
initial waiting
"#,
        );

        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("expected machine name"))
            .expect("machine-name diagnostic exists");
        let span = diagnostic.span.as_ref().expect("diagnostic has span");
        assert_eq!(span.file, "<source>");
        assert_eq!(span.start_line, 2);
        assert_eq!(span.start_column, 1);
    }

    #[test]
    fn parse_with_file_records_workflow_source_path() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = crate::parse_source_with_file(source, "examples/workflows/minimal.armature")
            .expect("minimal source parses");

        assert_eq!(
            ir.workflow.source_path.as_deref(),
            Some("examples/workflows/minimal.armature")
        );
    }

    #[test]
    fn parser_rejects_duplicate_nested_state_declarations() {
        let source = r#"
machine DuplicateNestedStates
initial parent

state parent {
  initial child

  state child {
    final
  }

  state child {
    final
  }
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.ir.is_some());
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate state `child`")));
    }

    #[test]
    fn parser_rejects_statements_after_explicit_outcomes() {
        let source = r#"
machine OutcomeOrder
initial waiting

agent director = thread("director")

event go {}

state waiting {
  on go {
    goto done
    send director "unreachable"
  }
}

state done {
  final
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.ir.is_some());
        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("explicit outcome must be the last statement")));
    }

    #[test]
    fn parser_rejects_repeated_explicit_outcomes() {
        let source = r#"
machine RepeatedOutcome
initial waiting

event go {}

state waiting {
  on go {
    stay
    goto done
  }
}

state done {
  final
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.ir.is_some());
        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("explicit outcome must be the last statement")));
    }

    #[test]
    fn parser_suggests_canonical_agent_declaration_for_natural_agent_syntax() {
        let source = r#"
machine BadAgent
initial done

agent worker thread maxActive 1

state done {
  final
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("agent declarations use `=` and a constructor")));
        assert!(!parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unexpected top-level token")));
    }

    #[test]
    fn parser_suggests_guard_instead_of_when() {
        let source = r#"
machine BadWhen
initial waiting

event go {
  ready bool
}

state waiting {
  on go as evt when evt.ready {
    stay
  }
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("event handlers use `guard`, not `when`")));
        assert!(!parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unexpected state member")));
    }

    #[test]
    fn parser_suggests_start_block_instead_of_call_syntax() {
        let source = r#"
machine BadStart
initial waiting

agent worker = codingAgent()

event go {}

state waiting {
  on go {
    start worker("hello")
    stay
  }
}
"#;

        let parsed = crate::parse_syntax(source);

        assert!(parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("start uses a block input, not call syntax")));
    }

    #[test]
    fn validation_rejects_duplicate_schema_members() {
        let source = r#"
machine DuplicateSchemaMembers
initial done

enum Choice {
  One
  One
}

class Payload {
  value string
  value int
}

event run {
  name string
  name int
}

coerce classify(input string, input int) -> Payload {
  model "gpt-4o-mini"
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        for expected in [
            "type `Choice` declares duplicate enum value `One`",
            "type `Payload` declares duplicate field `value`",
            "event `run` payload declares duplicate field `name`",
            "coerce `classify` declares duplicate parameter `input`",
        ] {
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected)),
                "missing diagnostic {expected}: {:#?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn validation_rejects_lowercase_enum_values() {
        let source = r#"
machine BadEnumValue
initial done

enum Choice {
  startWorker
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(
                "type `Choice` enum value `startWorker` must start with an uppercase ASCII letter"
            )));
    }

    #[test]
    fn validation_rejects_zero_agent_max_active() {
        let source = r#"
machine BadAgent
initial done

agent worker = codingAgent() {
  maxActive 0
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("agent `worker` maxActive must be greater than 0")
            })
            .expect("maxActive diagnostic exists");
        let span = diagnostic.span.as_ref().expect("diagnostic has span");
        assert_eq!(span.start_line, 5);
        assert_eq!(span.start_column, 1);
    }

    #[test]
    fn validation_requires_finished_event_for_bounded_starts() {
        let source = r#"
machine MissingFinished
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("requires event `finished` with required string field `name`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("requires at least one `finished` handler")));
    }

    #[test]
    fn validation_requires_finished_handler_for_bounded_starts() {
        let source = r#"
machine MissingFinishedHandler
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("requires at least one `finished` handler")));
        assert!(!report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("requires event `finished` with required string field `name`")));
    }

    #[test]
    fn validation_accepts_finished_convention_for_bounded_starts() {
        let source = r#"
machine BoundedStart
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_cyclic_type_refs() {
        let source = r#"
machine CyclicTypes
initial done

class A {
  b B
}

class B {
  a A
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `A` has a cyclic reference")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `B` has a cyclic reference")));
    }

    #[test]
    fn record_schema_allows_missing_optional_fields_only() {
        let schema = crate::schema::Schema::Record {
            fields: vec![
                crate::schema::Field {
                    name: "required".to_string(),
                    schema: crate::schema::Schema::String,
                },
                crate::schema::Field {
                    name: "optional".to_string(),
                    schema: crate::schema::Schema::Optional {
                        inner: Box::new(crate::schema::Schema::Int),
                    },
                },
            ],
        };

        assert!(schema.accepts_json(&json!({"required": "present"})));
        assert!(schema.accepts_json(&json!({"required": "present", "optional": null})));
        assert!(schema.accepts_json(&json!({"required": "present", "optional": 1})));
        assert!(!schema.accepts_json(&json!({"optional": 1})));
        assert!(!schema.accepts_json(&json!({"required": "present", "optional": "bad"})));
        assert!(!schema.accepts_json(&json!({"required": "present", "extra": true})));
    }

    #[test]
    fn map_schema_validates_values() {
        let schema = crate::schema::Schema::Map {
            key: Box::new(crate::schema::Schema::String),
            value: Box::new(crate::schema::Schema::Int),
        };

        assert!(schema.accepts_json(&json!({"a": 1, "b": 2})));
        assert!(!schema.accepts_json(&json!({"a": "bad"})));
        assert!(!schema.accepts_json(&json!(["a", 1])));
    }

    #[test]
    fn map_schema_validates_literal_keys() {
        let schema = crate::schema::Schema::Map {
            key: Box::new(crate::schema::Schema::Literal {
                value: json!("allowed"),
            }),
            value: Box::new(crate::schema::Schema::Boolean),
        };

        assert!(schema.accepts_json(&json!({"allowed": true})));
        assert!(!schema.accepts_json(&json!({"denied": true})));
        assert!(!schema.accepts_json(&json!({"allowed": "bad"})));
    }

    #[test]
    fn map_schema_rejects_json_object_keys_that_do_not_match_key_schema() {
        let schema = crate::schema::Schema::Map {
            key: Box::new(crate::schema::Schema::Int),
            value: Box::new(crate::schema::Schema::String),
        };

        assert!(!schema.accepts_json(&json!({"1": "value"})));
    }

    #[test]
    fn schema_validation_resolves_named_type_refs() {
        let mut types = std::collections::BTreeMap::new();
        types.insert(
            "Run".to_string(),
            crate::schema::Schema::Record {
                fields: vec![crate::schema::Field {
                    name: "id".to_string(),
                    schema: crate::schema::Schema::String,
                }],
            },
        );
        let schema = crate::schema::Schema::Ref {
            name: "Run".to_string(),
        };

        assert!(schema.accepts_json_with_types(&json!({"id": "run-1"}), &types));
        assert!(!schema.accepts_json_with_types(&json!({"id": 42}), &types));
        assert!(!schema.accepts_json(&json!({"id": "run-1"})));
    }

    #[test]
    fn parses_effect_statements_and_validates_targets() {
        let source = r#"
machine Effects
initial waiting

agent worker = codingAgent()
agent director = thread("director")
capability plan = adapter("implementationPlan")

event go {
  message string
}

event followUp {
  message string
}

state waiting {
  on go as evt {
    plan.markDone(evt.message)
    start worker {
      task evt.message
    }
    send director evt.message
    askHuman(evt.message)
    raise followUp {
      message evt.message
    }
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);
        let steps = &ir.statechart.states["waiting"].on[0].steps;

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(steps[0].effect, "capability_call");
        assert_eq!(steps[1].effect, "start");
        assert_eq!(steps[2].effect, "send");
        assert_eq!(steps[3].effect, "askHuman");
        assert_eq!(steps[4].effect, "raise");
    }

    #[test]
    fn validation_rejects_unknown_effect_targets() {
        let source = r#"
machine BadEffects
initial waiting

event go {
  message string
}

state waiting {
  on go as evt {
    missing.markDone(evt.message)
    start worker {
      task evt.message
    }
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let capability_diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("undeclared capability"))
            .expect("capability diagnostic exists");
        assert_eq!(
            capability_diagnostic
                .span
                .as_ref()
                .expect("capability diagnostic has span")
                .start_line,
            11
        );
        let agent_diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("undeclared agent"))
            .expect("agent diagnostic exists");
        assert_eq!(
            agent_diagnostic
                .span
                .as_ref()
                .expect("agent diagnostic has span")
                .start_line,
            12
        );
    }

    #[test]
    fn validation_rejects_reserved_timer_and_terminal_effects_in_ir() {
        let mut ir = crate::parse_source(
            r#"
machine ReservedEffects
initial waiting

state waiting {
  final
}
"#,
        )
        .expect("source parses");
        ir.statechart.states.get_mut("waiting").unwrap().entry = vec![
            crate::ir::Step {
                effect: "sleep".to_string(),
                args: std::collections::BTreeMap::new(),
                assign: None,
                case_arms: Vec::new(),
                span: None,
            },
            crate::ir::Step {
                effect: "stop".to_string(),
                args: std::collections::BTreeMap::new(),
                assign: None,
                case_arms: Vec::new(),
                span: None,
            },
        ];

        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown effect `sleep`")));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown effect `stop`")));
    }

    #[test]
    fn validation_rejects_starting_thread_agents() {
        let source = r#"
machine BadStartTarget
initial waiting

agent director = thread("director")

event go {}

state waiting {
  on go {
    start director
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("starts thread agent `director`; use `send`")));
    }

    #[test]
    fn validation_rejects_raising_unknown_events() {
        let source = r#"
machine BadRaise
initial waiting

event go {
  message string
}

state waiting {
  on go as evt {
    raise missing {
      message evt.message
    }
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("raises undeclared event `missing`")));
    }

    #[test]
    fn validation_rejects_invalid_raise_payloads() {
        let source = r#"
machine BadRaisePayload
initial waiting

event go {
  message string
}

event followUp {
  message string
  note string?
}

state waiting {
  on go as evt {
    raise followUp {
      message 42
    }
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("raises event `followUp` with `record` payload; expected `record`")));
    }

    #[test]
    fn validation_allows_raise_payloads_missing_optional_fields() {
        let source = r#"
machine OptionalRaisePayload
initial waiting

event go {}

event followUp {
  message string
  note string?
}

state waiting {
  on go {
    raise followUp {
      message "hello"
    }
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_non_string_message_effects() {
        let source = r#"
machine BadMessageEffects
initial waiting

agent director = thread("director")

event go {
  count int
}

state waiting {
  on go as evt {
    send director evt.count
    askHuman(evt.count)
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("`send` message has `int` value; expected `string`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("`askHuman` reason has `int` value; expected `string`")));
    }

    #[test]
    fn validation_rejects_unknown_coerce_expression_calls() {
        let source = r#"
machine BadCoerceCall
initial waiting

event go {
  id string
}

state waiting {
  on go as evt {
    let classification = coerce missingClassifier({
      id evt.id
    })
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("undeclared coerce function `missingClassifier`")));
    }

    #[test]
    fn validation_rejects_coerce_argument_type_mismatches() {
        let source = r#"
machine BadCoerceArgument
initial waiting

event go {
  id int
}

coerce classify(message string) -> string {
  prompt """
  Classify.
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.id)
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains(
                "calls coerce function `classify` argument 0 `message` with `int` value; expected `string`"
            )));
    }

    #[test]
    fn validation_rejects_baml_incompatible_coerce_boundary_types() {
        let source = r#"
machine BadBamlBoundary
initial done

coerce choose(lastSeen time) -> string {
  prompt """
  Choose.
  """
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("not supported as a BAML boundary type")));
    }

    #[test]
    fn validation_rejects_invalid_builtin_call_argument_types() {
        let source = r#"
machine BadBuiltinTypes
initial waiting

data {
  seen string[] = []
  lastIdleNudgeAt int? = nil
  counter int = 0
}

event go {
  id int
}

state waiting {
  on go as evt
    guard elapsedSince(data.lastIdleNudgeAt) >= 2m
  {
    assign data.seen = data.seen.append(evt.id)
    assign data.counter = data.counter.append(evt.id)
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(
                "calls `elapsedSince` with `optional` argument; expected `time` or `time?`"
            )));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `data.seen.append` with `int` item; expected `string`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `data.counter.append` on `int`; expected list or set")));
    }

    #[test]
    fn validation_accepts_documented_collection_map_and_text_helpers() {
        let source = r#"
machine HelperWorkflow
initial waiting

data {
  seen string[] = []
  names map<string, string> = {}
  first string? = nil
  found string? = nil
  hasRun bool = false
  count int = 0
}

event go {
  id string
  message string
}

state waiting {
  on go as evt
    guard text.contains(evt.message, "ready") || list.isEmpty(data.seen)
  {
    assign data.seen = list.append(data.seen, evt.id)
    assign data.seen = data.seen.remove("old")
    assign data.first = list.first(data.seen)
    assign data.names = map.set(data.names, evt.id, text.trim(evt.message))
    assign data.found = map.get(data.names, evt.id)
    assign data.hasRun = list.contains(data.seen, evt.id) && map.containsKey(data.names, evt.id)
    assign data.count = list.length(data.seen)
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_invalid_collection_map_and_text_helpers() {
        let source = r#"
machine BadHelperWorkflow
initial waiting

data {
  seen string[] = []
  counts map<string, int> = {}
  count int = 0
}

event go {
  id string
  numeric int
}

state waiting {
  on go as evt {
    assign data.count = list.length(data.count)
    assign data.seen = list.append(data.seen, evt.numeric)
    assign data.counts = map.set(data.counts, evt.numeric, 1)
    assign data.counts = map.set(data.counts, evt.id, evt.id)
    assign data.count = text.contains(evt.numeric, "x")
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `list.length` with `int` collection; expected list or set")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `list.append` with `int` item; expected `string`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `map.set` with `int` key; expected `string`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `map.set` with `string` value; expected `int`")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("calls `text.contains` with `int` text argument; expected `string`")));
    }

    #[test]
    fn validation_rejects_non_boolean_guards_and_boolean_operands() {
        let source = r#"
machine BadGuardTypes
initial waiting

data {
  count int = 0
  ready bool = false
}

event go {
  id string
}

state waiting {
  on go as evt
    guard data.count
  {
    stay
  }

  on go as evt
    guard data.ready && data.count
  {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let matching = report
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .contains("uses `int` expression where `bool` is required")
            })
            .count();
        assert!(matching >= 2, "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_non_boolean_invariants() {
        let source = r#"
machine BadInvariantType
initial waiting

data {
  count int = 0
}

event go {}

state waiting {
  on go {
    stay
  }
}
"#;

        let mut ir = crate::parse_source(source).expect("source parses");
        ir.invariants.push(crate::ir::Invariant::Expression {
            name: "badInvariant".to_string(),
            expr: crate::expr::Expr::Path {
                path: "data.count".to_string(),
            },
            span: None,
        });
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("invariant `badInvariant` expression uses `int` expression")));
    }

    #[test]
    fn validation_rejects_duplicate_invariant_names() {
        let source = r#"
machine DuplicateInvariants
initial waiting

data {
  count int = 0
}

state waiting {
  final
}

invariant stableCount

invariant stableCount {
  assert data.count == data.count
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("invariant `stableCount` is declared more than once")
            })
            .expect("duplicate invariant diagnostic");
        assert_eq!(
            diagnostic.span.as_ref().map(|span| span.start_line),
            Some(15)
        );
    }

    #[test]
    fn validation_rejects_unknown_builtin_invariant_names() {
        let source = r#"
machine UnknownBuiltinInvariant
initial waiting

state waiting {
  final
}

invariant declaredEffectOnly
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("unknown built-in invariant `declaredEffectOnly`")
            })
            .expect("unknown built-in invariant diagnostic");
        assert_eq!(
            diagnostic.span.as_ref().map(|span| span.start_line),
            Some(9)
        );
    }

    #[test]
    fn validation_rejects_invalid_in_operator_types() {
        let source = r#"
machine BadInTypes
initial waiting

data {
  count int = 0
  labels string[] = []
}

event go {
  id int
}

state waiting {
  on go as evt
    guard evt.id in data.count
  {
    stay
  }

  on go as evt
    guard evt.id in data.labels
  {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("uses `in` with `int` collection; expected list, set, or map")));
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("uses `in` with `int` item; expected `string`")));
    }

    #[test]
    fn validation_rejects_incompatible_equality_operands() {
        let source = r#"
machine BadEqualityTypes
initial waiting

data {
  count int = 0
}

event go {}

state waiting {
  on go
    guard data.count == "one"
  {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("compares incompatible `int` and `literal` expressions")));
    }

    #[test]
    fn validation_rejects_assignment_type_mismatches() {
        let source = r#"
machine BadAssignmentType
initial waiting

data {
  count int = 0
}

event go {
  message string
}

state waiting {
  on go as evt {
    assign data.count = evt.message
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assigns `string` value to `data.count`; expected `int`")));
    }

    #[test]
    fn validation_rejects_optional_assignment_to_required_data() {
        let source = r#"
machine BadOptionalAssignment
initial waiting

data {
  maybeCount int? = nil
  count int = 0
}

event go {}

state waiting {
  on go {
    assign data.count = data.maybeCount
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assigns `optional` value to `data.count`; expected `int`")));
    }

    #[test]
    fn validation_allows_guarded_optional_assignment_to_required_data() {
        let source = r#"
machine GuardedOptionalAssignment
initial waiting

data {
  count int = 0
}

event go {
  maybeCount int?
}

state waiting {
  on go as evt
    guard evt.maybeCount != nil
  {
    assign data.count = evt.maybeCount
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_reversed_guarded_optional_assignment_to_required_data() {
        let source = r#"
machine ReversedGuardedOptionalAssignment
initial waiting

data {
  maybeCount int? = nil
  count int = 0
}

event go {}

state waiting {
  on go
    guard nil != data.maybeCount
  {
    assign data.count = data.maybeCount
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_unguarded_nested_optional_field_access() {
        let source = r#"
machine BadNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("references nested field through optional path `data.user`")));
    }

    #[test]
    fn validation_allows_guarded_nested_optional_field_access() {
        let source = r#"
machine GuardedNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go
    guard data.user != nil
  {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_negated_nil_guarded_nested_optional_field_access() {
        let source = r#"
machine NegatedNilGuardedNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go
    guard !(data.user == nil)
  {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_shared_or_guard_optional_refinement() {
        let source = r#"
machine OrGuardedNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  urgent bool = false
  status string = ""
}

event go {}

state waiting {
  on go
    guard (data.user != nil && data.urgent) || (data.user != nil && !data.urgent)
  {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_demorgan_guard_optional_refinement() {
        let source = r#"
machine DemorganGuardedNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  blocked bool = false
  status string = ""
}

event go {}

state waiting {
  on go
    guard !(data.user == nil || data.blocked)
  {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_double_negated_guard_optional_refinement() {
        let source = r#"
machine DoubleNegatedGuardedNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go
    guard !!(data.user != nil)
  {
    assign data.status = data.user.status
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_case_literal_to_refine_optional_assignment() {
        let source = r#"
machine CaseRefinesOptionalAssignment
initial waiting

data {
  status string = ""
}

event go {
  maybeStatus string?
}

state waiting {
  on go as evt {
    case evt.maybeStatus {
      "ready" -> {
        assign data.status = evt.maybeStatus
        stay
      }

      _ -> {
        stay
      }
    }
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_allows_case_nil_then_wildcard_to_refine_optional_field_access() {
        let source = r#"
machine CaseRefinesNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go {
    case data.user {
      nil -> {
        stay
      }

      _ -> {
        assign data.status = data.user.status
        stay
      }
    }
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_case_wildcard_without_nil_refinement_for_optional_field_access() {
        let source = r#"
machine CaseWildcardDoesNotRefineNestedOptionalAccess
initial waiting

class User {
  status string
}

data {
  user User? = nil
  status string = ""
}

event go {}

state waiting {
  on go {
    case data.user {
      _ -> {
        assign data.status = data.user.status
        stay
      }
    }
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("references nested field through optional path `data.user`")));
    }

    #[test]
    fn validation_allows_nil_assignment_to_optional_data() {
        let source = r#"
machine OptionalNilAssignment
initial waiting

data {
  maybeCount int? = 1
}

event go {}

state waiting {
  on go {
    assign data.maybeCount = nil
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_mixed_list_assignment_to_homogeneous_data() {
        let source = r#"
machine BadMixedListAssignment
initial waiting

data {
  labels string[] = []
}

event go {}

state waiting {
  on go {
    assign data.labels = ["ok", 1]
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assigns `list` value to `data.labels`; expected `list`")));
    }

    #[test]
    fn validation_rejects_data_initializer_type_mismatches() {
        let source = r#"
machine BadDataInitializer
initial waiting

data {
  count int = "bad"
}

event go {
  id string
}

state waiting {
  on go as evt {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("data field `count` initializer does not match declared `int` schema")));
    }

    #[test]
    fn validation_rejects_extra_record_initializer_fields() {
        let source = r#"
machine BadRecordInitializer
initial waiting

class UserState {
  status string
}

data {
  user UserState = { status "todo", typo true }
}

event go {}

state waiting {
  on go {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("data field `user` initializer does not match declared `ref` schema")));
    }

    #[test]
    fn validation_rejects_missing_required_json_record_initializer_fields() {
        let source = r#"
machine BadRecordInitializer
initial waiting

class UserState {
  payload json
}

data {
  user UserState = {}
}

event go {}

state waiting {
  on go {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("data field `user` initializer does not match declared `ref` schema")));
    }

    #[test]
    fn validation_rejects_dynamic_data_initializers() {
        let source = r#"
machine DynamicDataInitializer
initial waiting

data {
  startedAt time = now()
}

event go {
  id string
}

state waiting {
  on go as evt {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("data field `startedAt` initializer must be a static literal")));
    }

    #[test]
    fn validation_checks_literal_type_assignments_precisely() {
        let source = r#"
machine LiteralAssignments
initial waiting

data {
  exact "Ready" = "Ready"
  text string = "initial"
}

event good {}
event bad {}

state waiting {
  on good {
    assign data.exact = "Ready"
    assign data.text = "done"
  }

  on bad {
    assign data.exact = "Done"
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assigns `literal` value to `data.exact`; expected `literal`")));
    }

    #[test]
    fn validation_rejects_map_assignment_type_mismatches() {
        let source = r#"
machine BadMapAssignmentType
initial waiting

data {
  counts map<string, int> = {}
  flags map<string, bool> = {}
}

event go {
  id string
}

state waiting {
  on go as evt {
    assign data.counts = data.flags
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("assigns `map` value to `data.counts`; expected `map`")));
    }

    #[test]
    fn validation_rejects_non_string_compatible_map_keys() {
        let source = r#"
machine BadMapKey
initial waiting

data {
  keyedByInt map<int, string> = {}
}

event go {
  id string
}

state waiting {
  on go as evt {
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("data field `keyedByInt` declares map key type `int`")));
    }

    #[test]
    fn validation_rejects_unknown_expression_calls() {
        let source = r#"
machine BadExpressionCall
initial waiting

event go {
  id string
}

state waiting {
  on go as evt {
    let planText = plna.snapshot()
    send director evt.id
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("unknown function `plna.snapshot`")
            })
            .expect("unknown function diagnostic exists");
        assert_eq!(
            diagnostic
                .span
                .as_ref()
                .expect("diagnostic has statement span")
                .start_line,
            11
        );
    }

    #[test]
    fn validation_allows_declared_capability_expression_calls() {
        let source = r#"
machine CapabilityExpressionCall
initial waiting

capability plan = adapter("implementationPlan")

state waiting {
  entry {
    let planText = plan.snapshot()
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_builtin_expression_call_arity_errors() {
        let source = r#"
machine BadBuiltinArity
initial waiting

event go {
  id string
}

state waiting {
  on go as evt {
    let timestamp = now(evt.id)
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("calls `now` with 1 argument")));
    }

    #[test]
    fn validation_rejects_unknown_data_paths_in_expressions() {
        let source = r#"
machine BadDataPath
initial waiting

data {
  seen string[]
}

event go {
  id string
}

state waiting {
  on go as evt {
    assign data.seen = data.seenRunz.append(evt.id)
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("undeclared data field `seenRunz`")));
    }

    #[test]
    fn validation_rejects_unknown_event_binding_paths() {
        let source = r#"
machine BadEventBindingPath
initial waiting

agent director = thread("director")

event go {
  message string
}

state waiting {
  on go as evt {
    send director evtx.message
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("unknown path root `evtx`"))
            .expect("unknown path diagnostic exists");
        assert_eq!(
            diagnostic
                .span
                .as_ref()
                .expect("diagnostic has statement span")
                .start_line,
            13
        );
    }

    #[test]
    fn validation_rejects_unknown_event_payload_fields() {
        let source = r#"
machine BadEventPayloadPath
initial waiting

agent director = thread("director")

event go {
  message string
}

state waiting {
  on go as evt {
    send director evt.messgae
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("undeclared field `messgae`")));
    }

    #[test]
    fn validation_rejects_unknown_nested_data_fields() {
        let source = r#"
machine BadNestedDataPath
initial waiting

class Run {
  id string
}

data {
  last Run?
}

state waiting {
  always
    guard data.last.missing == "x"
  {
    goto waiting
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("undeclared field `missing`")));
    }

    #[test]
    fn validation_allows_let_local_paths() {
        let source = r#"
machine LetLocalPaths
initial waiting

agent director = thread("director")

event go {
  message string
}

state waiting {
  on go as evt {
    let note = {
      message evt.message
    }
    send director note.message
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn validation_rejects_unknown_let_local_fields() {
        let source = r#"
machine BadLetLocalPath
initial waiting

agent director = thread("director")

event go {
  message string
}

state waiting {
  on go as evt {
    let note = {
      message evt.message
    }
    send director note.mesage
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("undeclared field `mesage`")));
    }

    #[test]
    fn validation_rejects_unknown_coerce_output_fields() {
        let source = r#"
machine BadCoerceOutputPath
initial waiting

agent director = thread("director")

event go {
  message string
}

class Classification {
  reason string
}

coerce classify(message string) -> Classification {
  prompt """
  Classify.
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    send director classification.reasn
    stay
  }
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("undeclared field `reasn`")));
    }

    #[test]
    fn parses_case_as_structured_step() {
        let source = r#"
machine CaseWorkflow
initial waiting

event finished {
  name string
}

state waiting {
  on finished as run {
    case run.name {
      matches "worker-*" -> {
        goto done
      }

      _ -> {
        stay
      }
    }
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);
        let handler = &ir.statechart.states["waiting"].on[0];
        let case_step = &handler.steps[0];

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(handler.transition, None);
        assert_eq!(case_step.effect, "case");
        assert_eq!(case_step.case_arms.len(), 2);
        assert!(matches!(
            &case_step.case_arms[0].pattern,
            crate::ir::CasePattern::Matches { pattern } if pattern == "worker-*"
        ));
        assert_eq!(case_step.case_arms[0].transition.as_deref(), Some("done"));
        assert!(matches!(
            case_step.case_arms[1].pattern,
            crate::ir::CasePattern::Wildcard
        ));
    }

    #[test]
    fn parses_guard_expressions_with_boolean_operators() {
        let source = r#"
machine Guards
initial waiting

data {
  seen string[]
}

event finished {
  id string
  count int
}

state waiting {
  on finished as run
    guard !(run.id in data.seen)
    guard run.count >= 2
  {
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);
        let guard = ir.statechart.states["waiting"].on[0]
            .guard
            .as_ref()
            .expect("guard is lowered");

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert!(matches!(guard, crate::expr::Expr::And { exprs } if exprs.len() == 2));
        let crate::expr::Expr::And { exprs } = guard else {
            unreachable!("checked above");
        };
        assert!(matches!(
            &exprs[0],
            crate::expr::Expr::Not { expr }
                if matches!(expr.as_ref(), crate::expr::Expr::In { .. })
        ));
        assert!(matches!(&exprs[1], crate::expr::Expr::Gte { .. }));
    }

    #[test]
    fn parses_always_transition_blocks() {
        let source = r#"
machine AlwaysWorkflow
initial waiting

data {
  ready bool
}

state waiting {
  always
    guard data.ready == true
  {
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert_eq!(ir.statechart.states["waiting"].always.len(), 1);
        assert_eq!(ir.statechart.states["waiting"].always[0].transition, "done");
        assert!(ir.statechart.states["waiting"].always[0].guard.is_some());
    }

    #[test]
    fn validation_rejects_duplicate_state_names() {
        let source = r#"
machine DuplicateStates
initial parentA

state parentA {
  initial child
  state child {}
}

state parentB {
  initial child
  state child {}
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("declared more than once")));
    }

    #[test]
    fn validation_rejects_duplicate_unguarded_handlers() {
        let source = r#"
machine DuplicateHandlers
initial waiting

event go {
  id string
}

state waiting {
  on go as evt {
    stay
  }

  on go as evt {
    goto done
  }
}

state done {
  final
}
"#;

        let ir = crate::parse_source(source).expect("source parses");
        let report = crate::validate_ir(&ir);

        assert!(!report.is_ok());
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("multiple unguarded handlers for event `go`")));
    }

    #[test]
    fn validates_spec_implementation_fixture_static_surface() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = crate::parse_source(source).expect("spec implementation source parses");
        let report = crate::validate_ir(&ir);

        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        assert!(ir.agents.contains_key("worker"));
        assert!(ir.capabilities.contains_key("plan"));
        assert!(ir.coerce_functions.contains_key("classifyRun"));
        assert!(ir.types.contains_key("RunClassification"));
        assert_eq!(
            ir.statechart.states["running"].initial.as_deref(),
            Some("watching")
        );
        assert!(ir.statechart.states["running"].on[0].guard.is_some());
        assert!(ir.statechart.states["running"].states["watching"].on[0]
            .guard
            .is_some());
    }
}
