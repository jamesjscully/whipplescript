//! Pure rule-lowering engine (DR-0033 instance-scheduler full lift, chunk 1b).
//!
//! The rule pass lowers a ready rule context into an `OwnedLowering` (facts,
//! effects, dependencies, terminal, branch reports) using only the program IR and
//! the current fact/effect snapshot -- no store, no I/O. This entire closure (the
//! `lower_rule`/`ready_contexts` transitive set, verified free of any
//! store/native reference) was lifted out of the CLI so the rule engine can run
//! inside the wasm-clean kernel behind the host-agnostic instance step machine.
//! The native CLI now imports this module; items are `pub` for that cross-crate
//! use.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use std::path::Path;

use serde_json::{json, Value};
use whipplescript_parser::{DependencyPredicate as IrDependencyPredicate, *};
use whipplescript_store::{EffectView, FactView, WorkflowTerminalKind};

use crate::idempotency_key;
use crate::lowering::{
    BranchReport, BranchStatus, OwnedDependency, OwnedEffect, OwnedFact, OwnedLowering,
    OwnedWorkflowTerminal,
};

pub fn insert_json_field(value: &mut Value, key: &str, field: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_owned(), field);
    }
}

/// Self-contained structural schema embedded in a parsing effect's input so
/// the worker can validate ingested bytes without the program IR
/// (spec/json-ingestion.md). The effect carries its own contract, which also
/// keeps replay independent of later source edits.
pub fn ingest_shape_json(ir: &IrProgram, ty: &IrType, depth: usize) -> Value {
    if depth > 8 {
        return json!("json");
    }
    let object_fields = |fields: &[whipplescript_parser::IrClassField]| -> Value {
        Value::Object(
            fields
                .iter()
                .map(|field| {
                    (
                        field.name.clone(),
                        ingest_shape_json(ir, &field.ty, depth + 1),
                    )
                })
                .collect(),
        )
    };
    match ty {
        IrType::Primitive(primitive) => {
            json!(ir_type_name(&IrType::Primitive(primitive.clone())))
        }
        IrType::LiteralString(value) => json!({ "literal": value }),
        IrType::AgentRef(_) => json!("string"),
        IrType::Ref(name) => {
            for schema in &ir.schemas {
                match schema {
                    IrSchema::Class(class) if class.name == *name => {
                        return json!({ "class": name, "fields": object_fields(&class.fields) });
                    }
                    IrSchema::Enum(decl) if decl.name == *name => {
                        return json!({ "enum": decl.variants });
                    }
                    _ => {}
                }
            }
            json!("json")
        }
        IrType::Object(fields) => json!({ "fields": object_fields(fields) }),
        IrType::Optional(inner) => json!({ "optional": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Array(inner) => json!({ "array": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Map(inner) => json!({ "map": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Union(types) => json!({
            "union": types
                .iter()
                .map(|candidate| ingest_shape_json(ir, candidate, depth + 1))
                .collect::<Vec<_>>()
        }),
    }
}

/// Deterministic fixture value for an embedded structural shape: literal
/// fields yield their literal (which is how a variant fixture carries its
/// `variant` tag), enums pick the first variant, scalars get stable
/// placeholders (spec/sum-types.md fixture).
pub fn fixture_value_for_shape(shape: &Value) -> Value {
    match shape {
        Value::String(primitive) => match primitive.as_str() {
            "int" => json!(1),
            "float" => json!(0.5),
            "bool" => json!(true),
            "null" => Value::Null,
            "time" => json!("2026-01-01T00:00:00Z"),
            "json" => json!({}),
            _ => json!("fixture"),
        },
        Value::Object(map) => {
            if let Some(literal) = map.get("literal") {
                return literal.clone();
            }
            if let Some(variants) = map.get("enum").and_then(Value::as_array) {
                return variants
                    .first()
                    .cloned()
                    .unwrap_or_else(|| json!("fixture"));
            }
            if let Some(inner) = map.get("optional") {
                return fixture_value_for_shape(inner);
            }
            if let Some(inner) = map.get("array") {
                return json!([fixture_value_for_shape(inner)]);
            }
            if let Some(inner) = map.get("map") {
                return json!({ "fixture": fixture_value_for_shape(inner) });
            }
            if let Some(options) = map.get("union").and_then(Value::as_array) {
                return options
                    .first()
                    .map(fixture_value_for_shape)
                    .unwrap_or_else(|| json!({}));
            }
            if let Some(fields) = map.get("fields").and_then(Value::as_object) {
                return Value::Object(
                    fields
                        .iter()
                        .map(|(name, field_shape)| {
                            (name.clone(), fixture_value_for_shape(field_shape))
                        })
                        .collect(),
                );
            }
            json!({})
        }
        _ => json!("fixture"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardReport {
    pub rule: String,
    pub when: String,
    pub expr: String,
    pub source_span_json: Option<String>,
    pub status: GuardStatus,
    pub matched: bool,
    pub actual: Value,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardStatus {
    Matched,
    False,
    Error,
}

impl GuardStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::False => "false",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuleContext {
    pub trigger_event_id: Option<String>,
    pub identity: Option<String>,
    pub bindings: Vec<(String, FactView)>,
}

pub fn ready_contexts(
    ir: &IrProgram,
    rule: &IrRule,
    facts: &[FactView],
    effects: &[EffectView],
    started_event_id: Option<&str>,
) -> ReadyContexts {
    let mut contexts = vec![RuleContext {
        trigger_event_id: started_event_id.map(str::to_owned),
        identity: None,
        bindings: Vec::new(),
    }];
    let mut guard_reports = Vec::new();
    for when in &rule.whens {
        let pattern = when.pattern.as_str();
        if pattern == "started" {
            if started_event_id.is_none() {
                return ReadyContexts::empty(guard_reports);
            }
            continue;
        }
        if pattern.ends_with(" is available") {
            continue;
        }
        if let Some((schema, binding)) = pattern.split_once(" as ") {
            let schema = schema.trim();
            let binding = binding.trim();
            let matching = facts
                .iter()
                .filter(|fact| fact.name == schema || fact.name == normalize_pattern_name(schema))
                .filter(|fact| pattern_agent_matches(ir, schema, fact))
                .filter(|fact| pattern_queue_matches(schema, fact))
                .cloned()
                .collect::<Vec<_>>();
            if matching.is_empty() {
                return ReadyContexts::empty(guard_reports);
            }
            let mut expanded = Vec::new();
            for context in contexts {
                for fact in &matching {
                    let mut context = context.clone();
                    context.identity = Some(format!("{binding}:{}", fact.key));
                    context.bindings.push((binding.to_owned(), fact.clone()));
                    match &when.guard {
                        Some(guard) => {
                            let report = eval_guard(
                                &rule.name,
                                &when.source,
                                &guard.source,
                                &guard.expr,
                                &context,
                                facts,
                                effects,
                                ir,
                            );
                            let matched = report.matched;
                            guard_reports.push(report);
                            if matched {
                                expanded.push(context);
                            }
                        }
                        None => expanded.push(context),
                    }
                }
            }
            contexts = expanded;
            continue;
        }
        // Special readiness patterns without an `as` binding (for example
        // `ralph completed turn`) require a matching fact but bind nothing.
        let normalized = normalize_pattern_name(pattern);
        if normalized != pattern {
            let satisfied = facts.iter().any(|fact| {
                fact.name == normalized
                    && pattern_agent_matches(ir, pattern, fact)
                    && pattern_queue_matches(pattern, fact)
            });
            if !satisfied {
                return ReadyContexts::empty(guard_reports);
            }
            continue;
        }
        return ReadyContexts::empty(guard_reports);
    }
    ReadyContexts {
        contexts,
        guard_reports,
    }
}

/// For `<queue> has ready item` patterns, only the named queue's projected
/// items match.
pub fn pattern_queue_matches(pattern: &str, fact: &FactView) -> bool {
    let mut words = pattern.split_whitespace();
    let Some(queue) = words.next() else {
        return true;
    };
    if !(words.next() == Some("has")
        && words.next() == Some("ready")
        && words.next() == Some("item"))
    {
        return true;
    }
    json_from_str(&fact.value_json)
        .get("queue")
        .and_then(Value::as_str)
        .is_none_or(|fact_queue| fact_queue == queue)
}

/// For `<agent> completed turn` patterns where the leading word names a
/// declared agent, only that agent's turns match. The generic `worker` form
/// (or any word that is not a declared agent) matches turns from any agent.
pub fn pattern_agent_matches(ir: &IrProgram, pattern: &str, fact: &FactView) -> bool {
    let Some(agent) = completed_turn_agent(pattern) else {
        return true;
    };
    if !ir.agents.iter().any(|declared| declared.name == agent) {
        return true;
    }
    serde_json::from_str::<Value>(&fact.value_json)
        .ok()
        .and_then(|value| {
            value
                .get("agent")
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .is_none_or(|fact_agent| fact_agent == agent)
}

pub struct ReadyContexts {
    pub contexts: Vec<RuleContext>,
    pub guard_reports: Vec<GuardReport>,
}

impl ReadyContexts {
    pub fn empty(guard_reports: Vec<GuardReport>) -> Self {
        Self {
            contexts: Vec::new(),
            guard_reports,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn eval_guard(
    rule: &str,
    when: &str,
    source: &str,
    guard: &Expr,
    context: &RuleContext,
    facts: &[FactView],
    effects: &[EffectView],
    ir: &IrProgram,
) -> GuardReport {
    let (status, actual, error) = guard_result(eval_expr_value(
        guard,
        &EvalScope::rule(context, facts, effects, ir),
    ));
    let matched = status == GuardStatus::Matched;
    GuardReport {
        rule: rule.to_owned(),
        when: when.to_owned(),
        expr: source.to_owned(),
        source_span_json: None,
        status,
        matched,
        actual,
        error,
    }
}

pub fn eval_guard_source_result(
    guard: &str,
    context: &RuleContext,
) -> (GuardStatus, Value, Option<String>) {
    let Ok(expr) = parse_expression(guard) else {
        return (
            GuardStatus::Error,
            json!({"internal": "ParseError"}),
            Some("case guard could not be parsed".to_owned()),
        );
    };
    let empty_ir = empty_ir_program();
    guard_result(eval_expr_value(
        &expr,
        &EvalScope::rule(context, &[], &[], &empty_ir),
    ))
}

pub fn guard_result(value: EvalValue) -> (GuardStatus, Value, Option<String>) {
    match value {
        EvalValue::Json(Value::Bool(true)) => (GuardStatus::Matched, Value::Bool(true), None),
        EvalValue::Json(Value::Bool(false)) => (GuardStatus::False, Value::Bool(false), None),
        EvalValue::Json(value) => (
            GuardStatus::Error,
            value,
            Some("guard expression did not evaluate to bool".to_owned()),
        ),
        EvalValue::Missing => (
            GuardStatus::Error,
            json!({"internal": "Missing"}),
            Some("guard expression evaluated to Missing".to_owned()),
        ),
        EvalValue::Error(message) => (
            GuardStatus::Error,
            json!({"internal": "Error", "message": message}),
            Some(message),
        ),
    }
}

pub fn parse_guard_literal(expr: &str) -> Value {
    let expr = expr.trim();
    if let Some(unquoted) = expr
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Value::String(unquoted.to_owned());
    }
    if let Ok(number) = expr.parse::<i64>() {
        return Value::Number(number.into());
    }
    match expr {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        value => Value::String(value.to_owned()),
    }
}

pub fn source_span_json(source_path: Option<&Path>, span: SourceSpan, construct: &str) -> String {
    json!({
        "path": source_path.map(|path| path.display().to_string()),
        "start": span.start,
        "end": span.end,
        "construct": construct,
    })
    .to_string()
}

pub struct EvalScope<'a> {
    pub context: Option<&'a RuleContext>,
    pub facts: &'a [FactView],
    pub effects: &'a [EffectView],
    pub ir: &'a IrProgram,
    pub projection: Option<&'a Value>,
    pub projection_schema: Option<&'a str>,
}

impl<'a> EvalScope<'a> {
    fn rule(
        context: &'a RuleContext,
        facts: &'a [FactView],
        effects: &'a [EffectView],
        ir: &'a IrProgram,
    ) -> Self {
        Self {
            context: Some(context),
            facts,
            effects,
            ir,
            projection: None,
            projection_schema: None,
        }
    }

    pub fn assertions(facts: &'a [FactView], effects: &'a [EffectView], ir: &'a IrProgram) -> Self {
        Self {
            context: None,
            facts,
            effects,
            ir,
            projection: None,
            projection_schema: None,
        }
    }

    pub fn projection(&self, projection: &'a Value, schema: Option<&'a str>) -> Self {
        Self {
            context: self.context,
            facts: self.facts,
            effects: self.effects,
            ir: self.ir,
            projection: Some(projection),
            projection_schema: schema,
        }
    }
}

pub fn empty_ir_program() -> IrProgram {
    IrProgram {
        workflow: String::new(),
        source_tags: Vec::new(),
        source_descriptions: Vec::new(),
        shared_coordination_usage: Vec::new(),
        sources: Vec::new(),
        tests: Vec::new(),
        includes: Vec::new(),
        pattern_applications: Vec::new(),
        workflow_contracts: Vec::new(),
        uses: Vec::new(),
        harnesses: Vec::new(),
        queues: Vec::new(),
        channels: Vec::new(),
        file_stores: Vec::new(),
        events: Vec::new(),
        leases: Vec::new(),
        ledgers: Vec::new(),
        counters: Vec::new(),
        schemas: Vec::new(),
        agents: Vec::new(),
        coerces: Vec::new(),
        assertions: Vec::new(),
        rules: Vec::new(),
        rule_dependencies: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvalValue {
    Json(Value),
    Missing,
    Error(String),
}

impl EvalValue {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }

    pub fn into_json(self) -> Value {
        match self {
            Self::Json(value) => value,
            Self::Missing => json!({"internal": "Missing"}),
            Self::Error(message) => json!({"internal": "Error", "message": message}),
        }
    }

    pub fn is_missing_or_null(&self) -> bool {
        matches!(self, Self::Missing | Self::Json(Value::Null))
    }
}

pub fn eval_expr_value(expr: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    match expr {
        Expr::Literal(ExprLiteral::Ident(value)) => eval_ident_literal(value, scope),
        Expr::Literal(literal) => EvalValue::Json(eval_expr_literal(literal)),
        Expr::Path(path) => eval_path(path, scope),
        Expr::Index { target, key } => eval_index(target, key, scope),
        Expr::Array(items) => EvalValue::Json(Value::Array(
            items
                .iter()
                .map(|item| eval_expr_value(item, scope).into_json())
                .collect(),
        )),
        Expr::Object(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.key.clone(),
                    eval_expr_value(&field.value, scope).into_json(),
                );
            }
            EvalValue::Json(Value::Object(object))
        }
        Expr::Unary { op, expr } => match op {
            UnaryOp::Not => EvalValue::Json(Value::Bool(!truthy(&eval_expr_value(expr, scope)))),
        },
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, scope),
        Expr::Call { name, args } => eval_call(name, args, scope),
        Expr::Query { .. } => eval_query_count(expr, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or_else(|value| value),
    }
}

pub fn eval_index(target: &Expr, key: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    let target = eval_expr_value(target, scope);
    let key = eval_expr_value(key, scope);
    let key = match key {
        EvalValue::Json(Value::String(value)) => value,
        EvalValue::Missing => return EvalValue::Missing,
        _ => return EvalValue::error("index expression key did not evaluate to string"),
    };
    match target {
        EvalValue::Json(Value::Object(object)) => object
            .get(&key)
            .cloned()
            .map(EvalValue::Json)
            .unwrap_or(EvalValue::Missing),
        EvalValue::Missing => EvalValue::Missing,
        _ => EvalValue::error("index expression target did not evaluate to object"),
    }
}

pub fn eval_ident_literal(value: &str, scope: &EvalScope<'_>) -> EvalValue {
    if let Some(projection) = scope.projection {
        if let Some(value) = projection.get(value) {
            return EvalValue::Json(value.clone());
        }
    }
    EvalValue::Json(eval_expr_literal(&ExprLiteral::Ident(value.to_owned())))
}

pub fn eval_expr_literal(literal: &ExprLiteral) -> Value {
    match literal {
        ExprLiteral::String(value) | ExprLiteral::Ident(value) => Value::String(value.clone()),
        ExprLiteral::Number(value) => value
            .parse::<i64>()
            .map(|number| Value::Number(number.into()))
            .or_else(|_| {
                value
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .ok_or(())
            })
            .unwrap_or_else(|_| Value::String(value.clone())),
        ExprLiteral::Bool(value) => Value::Bool(*value),
        ExprLiteral::Null => Value::Null,
    }
}

pub fn eval_binary(op: BinaryOp, left: &Expr, right: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    match op {
        BinaryOp::Or => {
            let left = eval_expr_value(left, scope);
            if truthy(&left) {
                EvalValue::Json(Value::Bool(true))
            } else {
                EvalValue::Json(Value::Bool(truthy(&eval_expr_value(right, scope))))
            }
        }
        BinaryOp::And => {
            let left = eval_expr_value(left, scope);
            if !truthy(&left) {
                EvalValue::Json(Value::Bool(false))
            } else {
                EvalValue::Json(Value::Bool(truthy(&eval_expr_value(right, scope))))
            }
        }
        BinaryOp::Eq => EvalValue::Json(Value::Bool(compare_eq(
            &eval_expr_value(left, scope),
            &eval_expr_value(right, scope),
        ))),
        BinaryOp::Ne => EvalValue::Json(Value::Bool(!compare_eq(
            &eval_expr_value(left, scope),
            &eval_expr_value(right, scope),
        ))),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            let left_value = eval_expr_value(left, scope);
            let right_value = eval_expr_value(right, scope);
            let left_ty = expr_runtime_primitive(left, scope);
            let right_ty = expr_runtime_primitive(right, scope);
            match ordered_cmp(
                &left_value,
                &right_value,
                left_ty.as_ref(),
                right_ty.as_ref(),
            ) {
                Ok(ordering) => {
                    let result = ordering
                        .map(|ordering| match op {
                            BinaryOp::Lt => ordering.is_lt(),
                            BinaryOp::Le => ordering.is_le(),
                            BinaryOp::Gt => ordering.is_gt(),
                            BinaryOp::Ge => ordering.is_ge(),
                            _ => false,
                        })
                        .unwrap_or(false);
                    EvalValue::Json(Value::Bool(result))
                }
                Err(message) => EvalValue::error(message),
            }
        }
        BinaryOp::In | BinaryOp::NotIn => {
            let needle = eval_expr_value(left, scope).into_json();
            let haystack = eval_expr_value(right, scope).into_json();
            let contains = match &haystack {
                Value::Array(items) => items.iter().any(|item| item == &needle),
                Value::Object(object) => {
                    needle.as_str().is_some_and(|key| object.contains_key(key))
                }
                _ => false,
            };
            EvalValue::Json(Value::Bool(if op == BinaryOp::In {
                contains
            } else {
                !contains
            }))
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            let left = eval_expr_value(left, scope);
            let right = eval_expr_value(right, scope);
            let (Some(lhs), Some(rhs)) = (number_value(&left), number_value(&right)) else {
                return EvalValue::error("arithmetic requires numeric operands");
            };
            let result = match op {
                BinaryOp::Add => lhs + rhs,
                BinaryOp::Sub => lhs - rhs,
                BinaryOp::Mul => lhs * rhs,
                _ => {
                    if rhs == 0.0 {
                        return EvalValue::error("division by zero");
                    }
                    lhs / rhs
                }
            };
            let integer_operands = matches!(&left, EvalValue::Json(Value::Number(n)) if n.is_i64())
                && matches!(&right, EvalValue::Json(Value::Number(n)) if n.is_i64());
            if integer_operands && result.fract() == 0.0 {
                EvalValue::Json(Value::Number((result as i64).into()))
            } else {
                serde_json::Number::from_f64(result)
                    .map(Value::Number)
                    .map(EvalValue::Json)
                    .unwrap_or_else(|| EvalValue::error("arithmetic produced a non-finite value"))
            }
        }
    }
}

pub fn eval_call(name: &str, args: &[Expr], scope: &EvalScope<'_>) -> EvalValue {
    match (name, args) {
        ("count", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or_else(|value| value),
        ("count", [expr]) => match eval_expr_value(expr, scope) {
            EvalValue::Json(Value::Array(items)) => {
                EvalValue::Json(Value::Number((items.len() as i64).into()))
            }
            EvalValue::Json(Value::Object(items)) => {
                EvalValue::Json(Value::Number((items.len() as i64).into()))
            }
            EvalValue::Json(Value::String(value)) => {
                EvalValue::Json(Value::Number((value.chars().count() as i64).into()))
            }
            EvalValue::Missing => EvalValue::error("missing value for count"),
            EvalValue::Error(message) => EvalValue::Error(message),
            _ => EvalValue::error("unsupported value for count"),
        },
        ("exists", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count > 0)))
            .unwrap_or_else(|value| value),
        ("exists", [expr]) => EvalValue::Json(Value::Bool(
            !eval_expr_value(expr, scope).is_missing_or_null(),
        )),
        _ => EvalValue::error("unknown expression function"),
    }
}

pub fn eval_query_count(query: &Expr, scope: &EvalScope<'_>) -> Result<i64, EvalValue> {
    let Expr::Query { kind, head, guard } = query else {
        return Ok(0);
    };
    match kind {
        QueryKind::Fact => {
            let mut count = 0;
            for fact in scope.facts.iter().filter(|fact| fact.name == head.trim()) {
                if let Some(guard) = guard {
                    let value = json_from_str(&fact.value_json);
                    if guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, Some(head.trim())),
                    ))? {
                        count += 1;
                    }
                } else {
                    count += 1;
                }
            }
            Ok(count)
        }
        QueryKind::Effect => {
            let kind = head
                .trim()
                .strip_prefix("kind ")
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let mut count = 0;
            for effect in scope
                .effects
                .iter()
                .filter(|effect| kind.is_none_or(|kind| effect.kind == kind))
            {
                if let Some(guard) = guard {
                    let value = json!({
                        "kind": effect.kind,
                        "target": effect.target,
                        "status": effect.status,
                        "profile": effect.profile,
                    });
                    if guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, None),
                    ))? {
                        count += 1;
                    }
                } else {
                    count += 1;
                }
            }
            Ok(count)
        }
    }
}

pub fn guard_filter_matches(value: EvalValue) -> Result<bool, EvalValue> {
    match value {
        EvalValue::Json(Value::Bool(value)) => Ok(value),
        value => Err(value),
    }
}

pub fn eval_path(path: &[String], scope: &EvalScope<'_>) -> EvalValue {
    if path.is_empty() {
        return EvalValue::error("empty expression path");
    }
    // The projection (the fact currently examined by a query's inner
    // `where`) is the innermost scope: its fields shadow outer rule
    // bindings. A path whose root is not a projection field falls through
    // to the rule context, so `count(Item where owner == task.owner)`
    // resolves `owner` against each Item and `task` against the binding.
    if let Some(projection) = scope.projection {
        if path
            .first()
            .is_some_and(|first| projection.get(first).is_some())
        {
            let mut current = projection;
            for field in path {
                let Some(next) = current.get(field) else {
                    return EvalValue::Missing;
                };
                current = next;
            }
            return EvalValue::Json(current.clone());
        }
    }
    if let Some(context) = scope.context {
        if let Some(first) = path.first() {
            if let Some(rest) = path.get(1..) {
                if rest.is_empty() {
                    return context
                        .bindings
                        .iter()
                        .find_map(|(binding, fact)| {
                            (binding == first)
                                .then(|| EvalValue::Json(json_from_str(&fact.value_json)))
                        })
                        .unwrap_or(EvalValue::Missing);
                }
                if let Some(value) = context_path_value(context, first, &rest.join(".")) {
                    return EvalValue::Json(value);
                }
                return EvalValue::Missing;
            }
        }
        return EvalValue::Missing;
    }
    if let Some(projection) = scope.projection {
        let mut current = projection;
        for field in path {
            let Some(next) = current.get(field) else {
                return EvalValue::Missing;
            };
            current = next;
        }
        return EvalValue::Json(current.clone());
    }
    EvalValue::Missing
}

pub fn compare_eq(left: &EvalValue, right: &EvalValue) -> bool {
    match (left, right) {
        (EvalValue::Json(left), EvalValue::Json(right)) => left == right,
        (EvalValue::Missing, EvalValue::Missing) => true,
        _ => false,
    }
}

pub fn truthy(value: &EvalValue) -> bool {
    match value {
        EvalValue::Json(Value::Bool(value)) => *value,
        EvalValue::Json(Value::Null) | EvalValue::Missing | EvalValue::Error(_) => false,
        EvalValue::Json(Value::Array(values)) => !values.is_empty(),
        EvalValue::Json(Value::String(value)) => !value.is_empty(),
        EvalValue::Json(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        EvalValue::Json(Value::Object(value)) => !value.is_empty(),
    }
}

pub fn number_value(value: &EvalValue) -> Option<f64> {
    match value {
        EvalValue::Json(value) => value.as_f64(),
        _ => None,
    }
}

pub fn ordered_cmp(
    left: &EvalValue,
    right: &EvalValue,
    left_ty: Option<&IrPrimitiveType>,
    right_ty: Option<&IrPrimitiveType>,
) -> Result<Option<std::cmp::Ordering>, String> {
    let typed = left_ty.or(right_ty);
    match typed {
        Some(IrPrimitiveType::Duration) => {
            let left = typed_string_seconds(left, "duration")?;
            let right = typed_string_seconds(right, "duration")?;
            return Ok(left.partial_cmp(&right));
        }
        Some(IrPrimitiveType::Time) => {
            let left = typed_string_seconds(left, "time")?;
            let right = typed_string_seconds(right, "time")?;
            return Ok(left.partial_cmp(&right));
        }
        _ => {}
    }

    match (number_value(left), number_value(right)) {
        (Some(left), Some(right)) => return Ok(left.partial_cmp(&right)),
        (Some(_), None) | (None, Some(_)) => return Ok(None),
        (None, None) => {}
    }
    let (EvalValue::Json(Value::String(left)), EvalValue::Json(Value::String(right))) =
        (left, right)
    else {
        return Ok(None);
    };
    if let (Some(left), Some(right)) = (parse_duration_seconds(left), parse_duration_seconds(right))
    {
        return Ok(left.partial_cmp(&right));
    }
    if let (Some(left), Some(right)) = (
        parse_time_epoch_seconds(left),
        parse_time_epoch_seconds(right),
    ) {
        return Ok(left.partial_cmp(&right));
    }
    Ok(None)
}

pub fn typed_string_seconds(value: &EvalValue, expected: &str) -> Result<f64, String> {
    let EvalValue::Json(Value::String(value)) = value else {
        return Err(format!("{expected} ordering expected string value"));
    };
    let parsed = match expected {
        "duration" => parse_duration_seconds(value),
        "time" => parse_time_epoch_seconds(value),
        _ => None,
    };
    parsed.ok_or_else(|| format!("invalid {expected} value `{value}`"))
}

pub fn expr_runtime_primitive(expr: &Expr, scope: &EvalScope<'_>) -> Option<IrPrimitiveType> {
    match expr {
        Expr::Path(path) => path_runtime_primitive(path, scope),
        Expr::Literal(ExprLiteral::Ident(field)) if scope.projection_schema.is_some() => {
            let path = [field.clone()];
            path_runtime_primitive(&path, scope)
        }
        _ => None,
    }
}

pub fn path_runtime_primitive(path: &[String], scope: &EvalScope<'_>) -> Option<IrPrimitiveType> {
    if let Some(context) = scope.context {
        let (binding, rest) = path.split_first()?;
        let (_, fact) = context
            .bindings
            .iter()
            .find(|(candidate, _)| candidate == binding)?;
        return ir_path_primitive(scope.ir, &fact.name, rest);
    }
    if let Some(schema) = scope.projection_schema {
        return ir_path_primitive(scope.ir, schema, path);
    }
    None
}

pub fn ir_path_primitive(ir: &IrProgram, schema: &str, path: &[String]) -> Option<IrPrimitiveType> {
    let mut current_schema = schema;
    for (index, field_name) in path.iter().enumerate() {
        let ty = ir.schemas.iter().find_map(|schema| match schema {
            IrSchema::Class(class) if class.name == current_schema => class
                .fields
                .iter()
                .find(|field| field.name == *field_name)
                .map(|field| &field.ty),
            _ => None,
        });
        let next_ty = unwrap_optional_type(ty?)?;
        if index == path.len() - 1 {
            return match next_ty {
                IrType::Primitive(primitive) => Some(primitive.clone()),
                _ => None,
            };
        }
        match next_ty {
            IrType::Ref(next_schema) => current_schema = next_schema,
            _ => return None,
        }
    }
    None
}

pub fn unwrap_optional_type(ty: &IrType) -> Option<&IrType> {
    match ty {
        IrType::Optional(inner) => unwrap_optional_type(inner),
        other => Some(other),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn lower_rule(
    instance_id: &str,
    program_version: &str,
    revision_epoch: &str,
    ir: &IrProgram,
    rule: &IrRule,
    context: &RuleContext,
    facts: &[FactView],
    effects: &[EffectView],
    source_path: Option<&Path>,
) -> OwnedLowering {
    let (body, mut context, branch_reports) = selected_rule_body(&rule.body, context);
    // Materialize top-level `redact … as <out>` projections (sources are when /
    // effect bindings already in scope); after-block redacts materialize below.
    materialize_redactions(&mut context, &rule.metadata.redactions);
    let existing_fact_ids = facts
        .iter()
        .map(|fact| fact.fact_id.as_str())
        .collect::<Vec<_>>();
    let existing_effect_ids = effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect::<Vec<_>>();
    let mut lowering = OwnedLowering::default();
    lowering.branch_reports.extend(branch_reports);
    let pre_terminal_body = strip_after_blocks(&body);
    append_consumed_fact_ids(&mut lowering, &pre_terminal_body, &context, facts);
    append_workflow_terminal(&mut lowering, ir, rule, &pre_terminal_body, &context, None);

    for (record_index, block) in top_level_record_blocks(&pre_terminal_body)
        .into_iter()
        .enumerate()
    {
        let value = parse_record_fields(
            &block.body,
            &context,
            block.from_binding.as_deref(),
            &mut lowering.errors,
        );
        let value_json = Value::Object(value).to_string();
        let fact_key = record_fact_key(&block.schema, &value_json);
        let fact_id = idempotency_key(&[
            instance_id,
            &rule.name,
            &block.schema,
            &fact_key,
            &value_json,
        ]);
        if existing_fact_ids
            .iter()
            .any(|existing| *existing == fact_id)
        {
            continue;
        }
        let record_source = rule
            .metadata
            .record_sources
            .get(record_index)
            .filter(|source| source.schema == block.schema);
        lowering.facts.push(OwnedFact {
            fact_id,
            name: block.schema.clone(),
            key: fact_key,
            value_json,
            schema_id: Some(block.schema),
            provenance_class: record_source
                .map(|source| {
                    if source.construct == "table_row" {
                        "table"
                    } else {
                        "rule"
                    }
                })
                .unwrap_or("rule")
                .to_owned(),
            correlation_id: context.identity.clone(),
            source_span_json: record_source
                .map(|source| source_span_json(source_path, source.span, &source.construct)),
        });
    }

    // Family C: `emit milestone "<name>" of <Class> { ... }` derives a durable
    // `workflow.milestone:<name>` fact in the child's own base — a synchronous
    // projection (NOT an async effect). The observing parent's invoke effect
    // later reads these and re-derives `workflow.invoke.reached:<name>` facts.
    for block in milestone_blocks(&pre_terminal_body) {
        let payload = if block.body.trim().is_empty() {
            serde_json::Map::new()
        } else {
            parse_record_fields(&block.body, &context, None, &mut lowering.errors)
        };
        let fact_name = format!("workflow.milestone:{}", block.name);
        let value_json = json!({
            "milestone": block.name,
            "status": "completed",
            "value": Value::Object(payload),
        })
        .to_string();
        // One fact per (instance, rule, milestone, context) — idempotent across
        // re-evaluations; the milestone name is in the key so distinct
        // milestones never collide.
        let fact_key = idempotency_key(&[instance_id, &rule.name, &fact_name]);
        let fact_id = idempotency_key(&[instance_id, &rule.name, &fact_name, &fact_key]);
        if existing_fact_ids
            .iter()
            .any(|existing| *existing == fact_id)
        {
            continue;
        }
        lowering.facts.push(OwnedFact {
            fact_id,
            name: fact_name,
            key: fact_key,
            value_json,
            schema_id: None,
            provenance_class: "rule".to_owned(),
            correlation_id: context.identity.clone(),
            source_span_json: None,
        });
    }

    let mut parsed_effects = parse_effect_statements(&pre_terminal_body, &context);
    rewrite_lease_releases(&mut parsed_effects, &rule.body);
    let parsed_effects = parsed_effects;
    let mut node_to_effect_id = std::collections::BTreeMap::new();
    let mut binding_to_effect_id = std::collections::BTreeMap::new();
    for (index, parsed) in parsed_effects.iter().enumerate() {
        let effect_node = effect_node_for_parsed(rule, parsed, index);
        let node_id = effect_node
            .map(|effect| effect.id.as_str())
            .unwrap_or(parsed.kind.as_str());
        let effect_id = idempotency_key(&[
            instance_id,
            program_version,
            revision_epoch,
            &rule.name,
            node_id,
            context.identity.as_deref().unwrap_or("started"),
        ]);
        node_to_effect_id.insert(node_id.to_owned(), effect_id.clone());
        if let Some(binding) = effect_node
            .and_then(|effect| effect.binding.as_ref())
            .or(parsed.binding.as_ref())
        {
            binding_to_effect_id.insert(binding.clone(), effect_id);
        }
    }
    for (index, parsed) in parsed_effects.iter().enumerate() {
        let effect_node = effect_node_for_parsed(rule, parsed, index);
        let node_id = effect_node
            .map(|effect| effect.id.as_str())
            .unwrap_or(parsed.kind.as_str());
        let Some(effect_id) = node_to_effect_id.get(node_id).cloned() else {
            continue;
        };
        if existing_effect_ids
            .iter()
            .any(|existing| *existing == effect_id)
        {
            continue;
        }
        if parsed
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("{{"))
        {
            lowering.errors.push(format!(
                "unresolved interpolation in `{}` effect `{node_id}`",
                parsed.kind
            ));
        }
        let input_json = parsed_effect_input_json(
            ir,
            rule,
            parsed,
            &context,
            &binding_to_effect_id,
            &mut lowering.errors,
        );
        let profile = parsed
            .target
            .as_deref()
            .and_then(|target| ir.agents.iter().find(|agent| agent.name == target))
            .and_then(|agent| agent.profile.clone());
        let effect_idempotency_key = idempotency_key(&[&effect_id, "effect"]);
        lowering.effects.push(OwnedEffect {
            effect_id,
            kind: parsed.kind.clone(),
            target: parsed.target.clone(),
            input_json,
            status: "queued".to_owned(),
            idempotency_key: effect_idempotency_key,
            required_capabilities_json: parsed.required_capabilities_json(),
            profile,
            correlation_id: context.identity.clone(),
            source_span_json: effect_node
                .map(|effect| source_span_json(source_path, effect.span, "effect")),
            timeout_seconds: parsed.timeout_seconds,
        });
    }

    for binding in cancel_statements(&pre_terminal_body) {
        if let Some(effect_id) = binding_to_effect_id.get(&binding) {
            lowering.cancels.push(effect_id.clone());
        } else {
            lowering
                .errors
                .push(format!("cancel of unknown effect binding `{binding}`"));
        }
    }

    for dependency in &rule.metadata.dependencies {
        let Some(upstream_effect_id) = node_to_effect_id.get(&dependency.upstream) else {
            continue;
        };
        let Some(downstream_effect_id) = node_to_effect_id.get(&dependency.downstream) else {
            continue;
        };
        if !lowering.effects.iter().any(|effect| {
            effect.effect_id == *upstream_effect_id || effect.effect_id == *downstream_effect_id
        }) {
            continue;
        }
        lowering.dependencies.push(OwnedDependency {
            dependency_id: idempotency_key(&[
                &rule.name,
                upstream_effect_id,
                dependency_predicate_str(&dependency.predicate),
                downstream_effect_id,
            ]),
            upstream_effect_id: upstream_effect_id.clone(),
            downstream_effect_id: downstream_effect_id.clone(),
            predicate: dependency_predicate_str(&dependency.predicate).to_owned(),
        });
    }

    for after in after_blocks(&body) {
        let Some(upstream_effect_id) = binding_to_effect_id.get(&after.binding) else {
            continue;
        };
        let Some(binding_value) = effect_binding_value(facts, upstream_effect_id, &after.predicate)
        else {
            continue;
        };
        let mut after_context = context.clone();
        push_effect_binding(
            &mut after_context,
            &after.binding,
            upstream_effect_id,
            binding_value.clone(),
        );
        if let Some(alias) = &after.alias {
            push_effect_binding(&mut after_context, alias, upstream_effect_id, binding_value);
        }
        for (binding, effect_id) in &binding_to_effect_id {
            if binding == &after.binding {
                continue;
            }
            if let Some(value) = effect_binding_value(facts, effect_id, "succeeds") {
                push_effect_binding(&mut after_context, binding, effect_id, value);
            }
        }
        let (selected_after_body, mut after_context, branch_reports) =
            selected_rule_body(&after.body, &after_context);
        // Materialize `redact … as <out>` projections inside this fired `after`
        // block — its alias (the redaction's source) is now bound.
        materialize_redactions(&mut after_context, &rule.metadata.redactions);
        lowering.branch_reports.extend(branch_reports);
        append_consumed_fact_ids(&mut lowering, &selected_after_body, &after_context, facts);
        append_workflow_terminal(
            &mut lowering,
            ir,
            rule,
            &selected_after_body,
            &after_context,
            Some((&after.binding, &after.predicate)),
        );
        // 503 auto-fail: a generated `flowfail` inside this fired `after <step>
        // fails` block means the step's failure is unhandled in a self-terminating
        // flow. The block only fires when the upstream effect actually failed
        // (`effect_binding_value(.., "fails")` above returned a value), so reaching
        // here is exactly the unhandled-failure case. Route to the kernel generic
        // failed terminal rather than a typed terminal.
        if lowering.internal_fail.is_none() && body_has_top_level_flowfail(&selected_after_body) {
            lowering.internal_fail = Some(format!(
                "unhandled failure of `{}` in flow rule `{}`",
                after.binding, rule.name
            ));
        }
        for record in top_level_record_blocks(&selected_after_body) {
            let value = parse_record_fields(
                &record.body,
                &after_context,
                record.from_binding.as_deref(),
                &mut lowering.errors,
            );
            let value_json = Value::Object(value).to_string();
            let fact_key = record_fact_key(&record.schema, &value_json);
            let fact_id = idempotency_key(&[
                instance_id,
                &rule.name,
                &after.binding,
                &after.predicate,
                &record.schema,
                &fact_key,
                &value_json,
            ]);
            if existing_fact_ids
                .iter()
                .any(|existing| *existing == fact_id)
                || lowering.facts.iter().any(|fact| fact.fact_id == fact_id)
            {
                continue;
            }
            lowering.facts.push(OwnedFact {
                fact_id,
                name: record.schema.clone(),
                key: fact_key,
                value_json,
                schema_id: Some(record.schema),
                provenance_class: "rule".to_owned(),
                correlation_id: context.identity.clone(),
                source_span_json: None,
            });
        }
        let mut selected_effects = parse_effect_statements(&selected_after_body, &after_context);
        rewrite_lease_releases(&mut selected_effects, &rule.body);
        for effect in &mut selected_effects {
            effect.after.get_or_insert_with(|| AfterScope {
                binding: after.binding.clone(),
                predicate: after.predicate.clone(),
            });
        }
        let mut selected_binding_to_effect_id = binding_to_effect_id.clone();
        let mut selected_node_to_effect_id = std::collections::BTreeMap::new();
        for (index, parsed) in selected_effects.iter().enumerate() {
            let effect_node = effect_node_for_parsed(rule, parsed, index);
            let node_id = effect_node
                .map(|effect| effect.id.as_str())
                .unwrap_or(parsed.kind.as_str());
            let effect_id = idempotency_key(&[
                instance_id,
                program_version,
                revision_epoch,
                &rule.name,
                &after.binding,
                &after.predicate,
                node_id,
                after_context.identity.as_deref().unwrap_or("started"),
            ]);
            selected_node_to_effect_id.insert(node_id.to_owned(), effect_id.clone());
            if let Some(binding) = effect_node
                .and_then(|effect| effect.binding.as_ref())
                .or(parsed.binding.as_ref())
            {
                selected_binding_to_effect_id.insert(binding.clone(), effect_id);
            }
        }
        for (binding, effect_id) in &selected_binding_to_effect_id {
            binding_to_effect_id
                .entry(binding.clone())
                .or_insert_with(|| effect_id.clone());
        }
        for binding in cancel_statements(&selected_after_body) {
            if let Some(effect_id) = selected_binding_to_effect_id.get(&binding) {
                lowering.cancels.push(effect_id.clone());
            } else {
                lowering
                    .errors
                    .push(format!("cancel of unknown effect binding `{binding}`"));
            }
        }
        for (index, parsed) in selected_effects.iter().enumerate() {
            let effect_node = effect_node_for_parsed(rule, parsed, index);
            let node_id = effect_node
                .map(|effect| effect.id.as_str())
                .unwrap_or(parsed.kind.as_str());
            let Some(effect_id) = selected_node_to_effect_id.get(node_id).cloned() else {
                continue;
            };
            if existing_effect_ids
                .iter()
                .any(|existing| *existing == effect_id)
                || lowering
                    .effects
                    .iter()
                    .any(|existing| existing.effect_id == effect_id)
            {
                continue;
            }
            if parsed
                .prompt
                .as_deref()
                .is_some_and(|prompt| prompt.contains("{{"))
            {
                lowering.errors.push(format!(
                    "unresolved interpolation in `{}` effect `{node_id}`",
                    parsed.kind
                ));
            }
            let input_json = parsed_effect_input_json(
                ir,
                rule,
                parsed,
                &after_context,
                &selected_binding_to_effect_id,
                &mut lowering.errors,
            );
            let profile = parsed
                .target
                .as_deref()
                .and_then(|target| ir.agents.iter().find(|agent| agent.name == target))
                .and_then(|agent| agent.profile.clone());
            let effect_idempotency_key = idempotency_key(&[&effect_id, "effect"]);
            lowering.effects.push(OwnedEffect {
                effect_id,
                kind: parsed.kind.clone(),
                target: parsed.target.clone(),
                input_json,
                status: "queued".to_owned(),
                idempotency_key: effect_idempotency_key,
                required_capabilities_json: parsed.required_capabilities_json(),
                profile,
                correlation_id: after_context.identity.clone(),
                source_span_json: effect_node
                    .map(|effect| source_span_json(source_path, effect.span, "effect")),
                timeout_seconds: parsed.timeout_seconds,
            });
        }
    }

    lowering
}

pub fn effect_node_for_parsed<'a>(
    rule: &'a IrRule,
    parsed: &ParsedEffect,
    index: usize,
) -> Option<&'a IrEffectNode> {
    parsed
        .binding
        .as_ref()
        .and_then(|binding| {
            rule.metadata
                .effects
                .iter()
                .find(|effect| effect.binding.as_ref() == Some(binding))
        })
        .or_else(|| rule.metadata.effects.get(index))
}

pub fn push_effect_binding(
    context: &mut RuleContext,
    binding: &str,
    effect_id: &str,
    value: Value,
) {
    context
        .bindings
        .retain(|(candidate, _)| candidate != binding);
    context.bindings.push((
        binding.to_owned(),
        FactView {
            fact_id: effect_id.to_owned(),
            program_version_id: None,
            revision_epoch: 0,
            name: binding.to_owned(),
            key: effect_id.to_owned(),
            value_json: value.to_string(),
            provenance_class: "effect".to_owned(),
            source_span_json: None,
        },
    ));
}

/// Projects a record value to a kept field subset (the runtime half of `redact`).
/// A non-object value has no fields to drop and passes through unchanged. This is
/// the concrete twin of `redact (keep)` in models/lean/Whipple/Redaction.lean: the
/// dropped fields are physically removed, so they cannot leave through any sink —
/// the runtime teeth behind the projected type's static drop.
pub fn project_record_value(value: &Value, keep: &[String]) -> Value {
    let Value::Object(map) = value else {
        return value.clone();
    };
    let mut projected = serde_json::Map::new();
    for field in keep {
        if let Some(field_value) = map.get(field) {
            projected.insert(field.clone(), field_value.clone());
        }
    }
    Value::Object(projected)
}

/// Materializes each `redact <source> keep [..] as <out>` of a rule into the
/// evaluation context, so the projected binding `out` resolves like any other
/// binding when payloads render. `redact` is a synchronous pure restructure (not
/// an effect), so it is computed here at lowering time once its source is bound;
/// a redaction whose source is not yet in scope is skipped (it materializes in the
/// scope that does bind it — e.g. inside the `after` block that aliases it).
/// Redactions are visited in body order, so a redaction chained off an earlier
/// one's output resolves.
pub fn materialize_redactions(context: &mut RuleContext, redactions: &[IrRedaction]) {
    for redaction in redactions {
        let Some(source) = context
            .bindings
            .iter()
            .find(|(binding, _)| binding == &redaction.source)
            .map(|(_, fact)| fact.clone())
        else {
            continue;
        };
        let projected = project_record_value(&json_from_str(&source.value_json), &redaction.keep);
        push_effect_binding(context, &redaction.binding, &source.fact_id, projected);
    }
}

pub fn append_consumed_fact_ids(
    lowering: &mut OwnedLowering,
    body: &str,
    context: &RuleContext,
    facts: &[FactView],
) {
    for binding in consume_statements(body) {
        let Some((_, fact)) = context
            .bindings
            .iter()
            .find(|(candidate, _)| candidate == &binding)
        else {
            continue;
        };
        if fact.provenance_class == "effect" {
            continue;
        }
        if !facts.iter().any(|active| active.fact_id == fact.fact_id) {
            continue;
        }
        if !lowering
            .consumed_fact_ids
            .iter()
            .any(|existing| existing == &fact.fact_id)
        {
            lowering.consumed_fact_ids.push(fact.fact_id.clone());
        }
    }
}

pub fn cancel_statements(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let binding = line
                .trim()
                .trim_end_matches(';')
                .strip_prefix("cancel ")?
                .trim();
            is_identifier(binding).then(|| binding.to_owned())
        })
        .collect()
}

pub fn consume_statements(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(';');
            let binding = line
                .strip_prefix("consume ")
                .or_else(|| line.strip_prefix("done "))?
                .split("->")
                .next()
                .unwrap_or_default()
                .trim();
            is_identifier(binding).then(|| binding.to_owned())
        })
        .collect()
}

pub fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn selected_rule_body(
    body: &str,
    context: &RuleContext,
) -> (String, RuleContext, Vec<BranchReport>) {
    let lines = body.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut context = context.clone();
    let mut branch_reports = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("after ") {
            let mut depth = brace_delta(trimmed).max(1);
            selected.push(lines[index].to_owned());
            index += 1;
            while index < lines.len() && depth > 0 {
                let line = lines[index];
                depth += brace_delta(line);
                selected.push(line.to_owned());
                index += 1;
            }
            continue;
        }
        if !trimmed.starts_with("case ") {
            selected.push(lines[index].to_owned());
            index += 1;
            continue;
        }
        let Some((case, next_index)) = parse_case_block(&lines, index) else {
            selected.push(lines[index].to_owned());
            index += 1;
            continue;
        };
        let selection = select_case_branch(&case, &mut context);
        if let Some(report) = selection.report {
            branch_reports.push(report);
        }
        if let Some(branch) = selection.branch {
            let (branch_body, branch_context, nested_reports) =
                selected_rule_body(&branch.body.join("\n"), &context);
            context = branch_context;
            branch_reports.extend(nested_reports);
            selected.extend(branch_body.lines().map(str::to_owned));
        }
        index = next_index;
    }
    (selected.join("\n"), context, branch_reports)
}

pub fn strip_after_blocks(body: &str) -> String {
    let lines = body.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("after ") {
            let mut depth = brace_delta(trimmed).max(1);
            index += 1;
            while index < lines.len() && depth > 0 {
                depth += brace_delta(lines[index]);
                index += 1;
            }
            continue;
        }
        selected.push(lines[index].to_owned());
        index += 1;
    }
    selected.join("\n")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseBlock {
    pub scrutinee: String,
    pub branches: Vec<CaseBranch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseBranch {
    pub pattern: String,
    pub guard: Option<String>,
    pub body: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseSelection {
    pub branch: Option<CaseBranch>,
    pub report: Option<BranchReport>,
}

pub fn parse_case_block(lines: &[&str], start: usize) -> Option<(CaseBlock, usize)> {
    let header = lines.get(start)?.trim();
    let scrutinee = header
        .strip_prefix("case ")?
        .strip_suffix('{')
        .unwrap_or_else(|| header.strip_prefix("case ").expect("case prefix"))
        .trim()
        .to_owned();
    let mut branches = Vec::new();
    let mut index = start + 1;
    let mut case_depth = brace_delta(header).max(1);
    while index < lines.len() && case_depth > 0 {
        let trimmed = lines[index].trim();
        if let Some((pattern, guard, before_body)) = case_branch_header(trimmed) {
            // `before_body` is the `=>` right-hand side, always starting with
            // `{`. A SINGLE-LINE branch (`pat => { complete ... }`) carries its
            // whole body here and its braces balance on this line; a multi-line
            // branch has just `{` with the body on the following lines. The old
            // logic forced depth to >= 1 and only collected following lines, so
            // a single-line branch's inline body was silently dropped and the
            // branch never materialized at runtime (a check/runtime divergence
            // that `whip fmt` did not fix, since fmt leaves branches single-line).
            let mut body = Vec::new();
            let inline = before_body.strip_prefix('{').unwrap_or(before_body);
            let mut branch_depth = 1 + brace_delta(inline);
            if branch_depth <= 0 {
                // Single-line branch: the body is the inline text minus its
                // closing `}`.
                let inline_body = inline.trim().strip_suffix('}').unwrap_or(inline).trim();
                if !inline_body.is_empty() {
                    body.push(inline_body.to_owned());
                }
                index += 1;
            } else {
                // Multi-line branch: keep any inline lead, then collect the
                // following lines until the branch's braces close.
                let inline_body = inline.trim();
                if !inline_body.is_empty() {
                    body.push(inline_body.to_owned());
                }
                index += 1;
                while index < lines.len() && branch_depth > 0 {
                    let line = lines[index];
                    let next_depth = branch_depth + brace_delta(line);
                    if next_depth >= 1 {
                        body.push(line.to_owned());
                    }
                    branch_depth = next_depth;
                    index += 1;
                }
            }
            branches.push(CaseBranch {
                pattern,
                guard,
                body,
            });
            continue;
        }
        case_depth += brace_delta(trimmed);
        index += 1;
    }
    Some((
        CaseBlock {
            scrutinee,
            branches,
        },
        index,
    ))
}

pub fn case_branch_header(line: &str) -> Option<(String, Option<String>, &str)> {
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
    Some((pattern.to_owned(), guard, body_start))
}

pub fn select_case_branch(case: &CaseBlock, context: &mut RuleContext) -> CaseSelection {
    let value = parse_field_value(&case.scrutinee, context);
    let mut fallback = None;
    for branch in &case.branches {
        if matches!(branch.pattern.as_str(), "_" | "default") {
            fallback = Some(branch.clone());
            continue;
        }
        let mut candidate_context = context.clone();
        if !case_pattern_matches(&branch.pattern, &value, &mut candidate_context) {
            continue;
        }
        if let Some(guard) = branch.guard.as_deref() {
            let (status, actual, error) = eval_guard_source_result(guard, &candidate_context);
            match status {
                GuardStatus::Matched => {}
                GuardStatus::False => continue,
                GuardStatus::Error => {
                    return CaseSelection {
                        branch: None,
                        report: Some(BranchReport {
                            scrutinee: case.scrutinee.clone(),
                            status: BranchStatus::Error,
                            matched: false,
                            tag: terminal_case_tag(&value).map(str::to_owned),
                            actual,
                            error,
                        }),
                    };
                }
            }
        }
        {
            *context = candidate_context;
            return CaseSelection {
                branch: Some(branch.clone()),
                report: Some(BranchReport {
                    scrutinee: case.scrutinee.clone(),
                    status: BranchStatus::Matched,
                    matched: true,
                    tag: terminal_case_tag(&value).map(str::to_owned),
                    actual: value.clone(),
                    error: None,
                }),
            };
        }
    }
    if let Some(branch) = fallback {
        return CaseSelection {
            branch: Some(branch),
            report: Some(BranchReport {
                scrutinee: case.scrutinee.clone(),
                status: BranchStatus::Matched,
                matched: true,
                tag: terminal_case_tag(&value).map(str::to_owned),
                actual: value,
                error: None,
            }),
        };
    }
    CaseSelection {
        branch: None,
        report: terminal_case_tag(&value).map(|tag| BranchReport {
            scrutinee: case.scrutinee.clone(),
            status: BranchStatus::NoMatch,
            matched: false,
            tag: Some(tag.to_owned()),
            actual: value.clone(),
            error: Some("terminal-output case matched no branch".to_owned()),
        }),
    }
}

pub fn case_pattern_matches(pattern: &str, value: &Value, context: &mut RuleContext) -> bool {
    if let Some(tag) = terminal_case_tag(value) {
        let mut parts = pattern.split_whitespace();
        let Some(pattern_tag) = parts.next() else {
            return false;
        };
        if pattern_tag != tag {
            return false;
        }
        // `Tag as binding` binds the payload; bare `Tag` binds nothing (Stage 1b:
        // the legacy space form `Tag binding` is no longer accepted).
        let binding = match (parts.next(), parts.next(), parts.next()) {
            (None, _, _) => None,
            (Some("as"), Some(binding), None) => Some(binding),
            _ => return false,
        };
        if let Some(binding) = binding {
            let payload = terminal_payload_for_tag(value, tag);
            context.bindings.push((
                binding.to_owned(),
                FactView {
                    fact_id: format!("case:{binding}"),
                    program_version_id: None,
                    revision_epoch: 0,
                    name: binding.to_owned(),
                    key: binding.to_owned(),
                    value_json: payload.to_string(),
                    provenance_class: "case".to_owned(),
                    source_span_json: None,
                },
            ));
        }
        return true;
    }
    if pattern == "None" {
        return value.is_null();
    }
    if let Some(binding) = pattern.strip_prefix("Some ").map(str::trim) {
        if value.is_null() || binding.is_empty() {
            return false;
        }
        context.bindings.push((
            binding.to_owned(),
            FactView {
                fact_id: format!("case:{binding}"),
                program_version_id: None,
                revision_epoch: 0,
                name: binding.to_owned(),
                key: binding.to_owned(),
                value_json: value.to_string(),
                provenance_class: "case".to_owned(),
                source_span_json: None,
            },
        ));
        return true;
    }
    // Sum-type value (spec/sum-types.md): an internally-tagged record
    // dispatching on the synthesized `variant` discriminant, compared
    // exactly — coerce already normalized the tag. `Variant as b` binds the
    // matched variant record.
    if let Some(variant) = value.get("variant").and_then(Value::as_str) {
        let mut parts = pattern.split_whitespace();
        if parts.next() != Some(variant) {
            return false;
        }
        let binding = match (parts.next(), parts.next(), parts.next()) {
            (None, _, _) => None,
            (Some("as"), Some(binding), None) => Some(binding),
            _ => return false,
        };
        if let Some(binding) = binding {
            context.bindings.push((
                binding.to_owned(),
                FactView {
                    fact_id: format!("case:{binding}"),
                    program_version_id: None,
                    revision_epoch: 0,
                    name: binding.to_owned(),
                    key: binding.to_owned(),
                    value_json: value.to_string(),
                    provenance_class: "case".to_owned(),
                    source_span_json: None,
                },
            ));
        }
        return true;
    }
    parse_guard_literal(pattern) == *value
}

pub fn terminal_case_tag(value: &Value) -> Option<&str> {
    value.get("tag").and_then(Value::as_str).or_else(|| {
        value
            .get("status")
            .and_then(Value::as_str)
            .and_then(terminal_tag_for_status)
    })
}

pub fn terminal_tag_for_status(status: &str) -> Option<&'static str> {
    match status {
        "completed" | "succeeded" => Some("Completed"),
        "failed" => Some("Failed"),
        "timed_out" | "timeout" => Some("TimedOut"),
        "cancelled" | "canceled" => Some("Cancelled"),
        _ => None,
    }
}

pub fn terminal_payload_for_tag(value: &Value, tag: &str) -> Value {
    match tag {
        "Completed" => value
            .get("value")
            .cloned()
            .or_else(|| value.get("output").cloned())
            .unwrap_or(Value::Null),
        "Failed" | "TimedOut" | "Cancelled" => {
            let mut payload = value
                .get("error")
                .cloned()
                .or_else(|| value.get("failure").cloned())
                .or_else(|| value.pointer("/metadata/error").cloned())
                .or_else(|| value.pointer("/metadata/failure").cloned())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            for field in ["summary", "effect_id", "run_id"] {
                if let Some(field_value) = value.get(field) {
                    payload.insert(field.to_owned(), field_value.clone());
                }
            }
            if !payload.contains_key("reason") {
                if let Some(summary) = value.get("summary") {
                    payload.insert("reason".to_owned(), summary.clone());
                }
            }
            Value::Object(payload)
        }
        _ => value.clone(),
    }
}

pub fn append_workflow_terminal(
    lowering: &mut OwnedLowering,
    ir: &IrProgram,
    rule: &IrRule,
    body: &str,
    context: &RuleContext,
    after: Option<(&str, &str)>,
) {
    if lowering.terminal.is_some() {
        return;
    }
    let Some(terminal) = top_level_terminal_blocks(body).into_iter().next() else {
        return;
    };
    if !workflow_contract_exists(ir, terminal.kind, &terminal.name) {
        return;
    }
    let payload = if let Some(scalar_source) = &terminal.scalar {
        // A bare scalar payload: evaluate the value expression to a JSON scalar
        // (a literal, or a binding path resolved against the rule context).
        parse_record_field_value("", scalar_source, context, None, &mut lowering.errors)
    } else {
        Value::Object(parse_record_fields(
            &terminal.body,
            context,
            terminal.from.as_deref(),
            &mut lowering.errors,
        ))
    };
    let payload_json = payload.to_string();
    let mut key_parts = vec![
        rule.name.as_str(),
        terminal.kind.action(),
        terminal.name.as_str(),
        context.identity.as_deref().unwrap_or("started"),
        payload_json.as_str(),
    ];
    if let Some((binding, predicate)) = after {
        key_parts.push(binding);
        key_parts.push(predicate);
    }
    let idempotency_key = idempotency_key(&key_parts);
    lowering.terminal = Some(OwnedWorkflowTerminal {
        kind: terminal.kind,
        name: terminal.name,
        payload_json,
        idempotency_key,
    });
}

pub fn workflow_contract_exists(ir: &IrProgram, kind: WorkflowTerminalKind, name: &str) -> bool {
    let wanted = match kind {
        WorkflowTerminalKind::Completed => whipplescript_parser::IrWorkflowContractKind::Output,
        WorkflowTerminalKind::Failed => whipplescript_parser::IrWorkflowContractKind::Failure,
    };
    ir.workflow_contracts
        .iter()
        .any(|contract| contract.kind == wanted && contract.name == name)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordBlock {
    pub schema: String,
    pub from_binding: Option<String>,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalBlock {
    pub kind: WorkflowTerminalKind,
    pub name: String,
    /// `complete <T> from <binding>`: bounded-type projection source. The payload
    /// shorthand fields copy this binding's same-named fields. `None` otherwise.
    pub from: Option<String>,
    pub body: String,
    /// A bare scalar payload value source (`complete result 0.9` → `Some("0.9")`).
    /// When set, `body` is empty and the payload is the evaluated scalar value
    /// rather than a field object. `None` for the field-block form.
    pub scalar: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AfterScope {
    pub binding: String,
    pub predicate: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveAfterScope {
    pub scope: AfterScope,
    pub depth: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedEffect {
    pub kind: String,
    pub target: Option<String>,
    pub name: Option<String>,
    pub binding: Option<String>,
    pub args: Vec<String>,
    pub prompt: Option<String>,
    pub prompt_content_type: Option<String>,
    pub required_capabilities: Vec<String>,
    pub after: Option<AfterScope>,
    pub timeout_seconds: Option<i64>,
}

impl ParsedEffect {
    pub fn required_capabilities_json(&self) -> String {
        let mut capabilities = match self.kind.as_str() {
            "coerce" => vec!["coerce".to_owned()],
            "loft.claim" => vec!["loft.claim".to_owned()],
            "human.ask" => vec!["human.ask".to_owned()],
            "capability.call" => Vec::new(),
            "event.emit" => vec!["event.emit".to_owned()],
            "workflow.invoke" => vec!["workflow.invoke".to_owned()],
            "exec.command" if self.name.as_deref() == Some("capability") => self
                .target
                .as_ref()
                .map(|target| vec![format!("script.{target}")])
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        capabilities.extend(self.required_capabilities.iter().cloned());
        capabilities.sort();
        capabilities.dedup();
        serde_json::to_string(&capabilities).unwrap_or_else(|_| "[]".to_owned())
    }
}

/// Extracts a `timeout <duration>` clause from an effect statement line.
pub fn parse_timeout_clause_seconds(line: &str) -> Option<i64> {
    let mut words = line.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if word == "timeout" {
            let value = words.peek()?;
            return whipplescript_parser::body::parse_short_duration_seconds(value)
                .map(|seconds| seconds as i64);
        }
    }
    None
}

/// Extracts a leading double-quoted string, honoring `\"` and `\\` escapes,
/// returning the unescaped content and the text after the closing quote.
pub fn extract_quoted_string(text: &str) -> Option<(String, &str)> {
    let rest = text.strip_prefix('"')?;
    let mut content = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some((_, escaped @ ('"' | '\\'))) => content.push(escaped),
                Some((_, other)) => {
                    content.push('\\');
                    content.push(other);
                }
                None => return None,
            },
            '"' => return Some((content, &rest[index + 1..])),
            _ => content.push(ch),
        }
    }
    None
}

/// Lease/counter keys are entity identities serialized to a stable string.
pub fn coordination_key_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// `release <x>` is queue-or-lease by referent: a binding acquired in this
/// rule body is a lease release (spec/coordination.md); anything else stays
/// the queue verb.
pub fn rewrite_lease_releases(effects: &mut [ParsedEffect], rule_body: &str) {
    let acquires = rule_body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .starts_with("acquire ")
                .then(|| binding_after_as(trimmed))
                .flatten()
        })
        .collect::<std::collections::BTreeSet<_>>();
    for effect in effects {
        if effect.kind == "queue.release"
            && effect
                .args
                .first()
                .is_some_and(|binding| acquires.contains(binding))
        {
            effect.kind = "lease.release".to_owned();
        }
    }
}

pub fn parse_effect_statements(body: &str, context: &RuleContext) -> Vec<ParsedEffect> {
    let mut effects = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut after_scopes: Vec<ActiveAfterScope> = Vec::new();
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let Some(scope) = parse_after_scope(trimmed) {
            after_scopes.push(ActiveAfterScope {
                scope,
                depth: brace_delta(trimmed).max(1),
            });
            index += 1;
            continue;
        }
        // Record blocks are fact writes, not effects; their field lines (`prompt
        // "..."`, `release ...`, ...) must not be scanned as effect statements.
        // The whole block is balanced, so skipping it leaves after-scope brace
        // accounting unchanged (net zero delta).
        if trimmed.starts_with("record ")
            || (trimmed.starts_with("done ") && trimmed.contains("-> record "))
        {
            let (_, next_index) = parse_statement_until_balanced_braces(&lines, index, trimmed);
            index = next_index + 1;
            continue;
        }
        let current_after = after_scopes.last().map(|scope| scope.scope.clone());
        if let Some(rest) = trimmed.strip_prefix("invoke ") {
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let target = rest
                .split_whitespace()
                .next()
                .unwrap_or("workflow")
                .trim_end_matches('{')
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "workflow.invoke".to_owned(),
                target: Some(target),
                name: Some("invoke".to_owned()),
                binding: binding_after_as(&statement),
                args: vec![body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("read ") {
            // read <format> from <store> at <path> as <binding> (std.files): the
            // store/path are stashed in `args` for the input builder.
            let before_as = rest.split(" as ").next().unwrap_or(rest).trim();
            let mut from_split = before_as.splitn(2, " from ");
            let format = from_split.next().unwrap_or("text").trim().to_owned();
            let after_from = from_split.next().unwrap_or("");
            let mut at_split = after_from.splitn(2, " at ");
            let store = at_split.next().unwrap_or("").trim().to_owned();
            let path = at_split
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "file.read".to_owned(),
                target: Some(store.clone()),
                name: Some(format.clone()),
                binding: binding_after_as(trimmed),
                args: vec![format, store, path],
                prompt: None,
                prompt_content_type: None,
                // v0: the `file store` declaration's `root` is the scope boundary;
                // a `files.read` capability-grant layer is a documented follow-up
                // (spec/std-library/files.md). Requiring it here without a grantor
                // would policy-block every read, so v0 reads are store-root scoped.
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if trimmed.strip_prefix("send ").is_some() {
            // send via <channel> { text <expr> [markdown <expr>] [thread_id <expr>] }
            // as <binding> (std.messaging). The block spans lines; the channel and
            // each field expression are carried raw in `args` for the input builder.
            // Lowers to a `messaging.send` capability.call (1929 OPTION A).
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let header = statement.split('{').next().unwrap_or(&statement);
            let after_send = header.trim().strip_prefix("send ").unwrap_or("").trim();
            let channel = after_send
                .strip_prefix("via ")
                .unwrap_or(after_send)
                .trim()
                .to_owned();
            // Extract `text`/`markdown`/`thread_id`, tolerant of order: scan tokens,
            // collecting each field expression up to the next field keyword.
            let inner = invoke_body(&statement).unwrap_or_default();
            let tokens = inner.split_whitespace().collect::<Vec<_>>();
            let field_keywords = ["text", "markdown", "thread_id"];
            let mut text_expr = String::new();
            let mut markdown_expr = String::new();
            let mut thread_expr = String::new();
            let mut cursor = 0;
            while cursor < tokens.len() {
                let field = tokens[cursor];
                if field_keywords.contains(&field) {
                    cursor += 1;
                    let mut value_tokens = Vec::new();
                    while cursor < tokens.len() && !field_keywords.contains(&tokens[cursor]) {
                        value_tokens.push(tokens[cursor]);
                        cursor += 1;
                    }
                    let value = value_tokens.join(" ");
                    match field {
                        "text" => text_expr = value,
                        "markdown" => markdown_expr = value,
                        "thread_id" => thread_expr = value,
                        _ => {}
                    }
                } else {
                    cursor += 1;
                }
            }
            let target = "messaging.send".to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "capability.call".to_owned(),
                target: Some(target.clone()),
                name: Some("send".to_owned()),
                binding: binding_after_as(&statement),
                args: vec![channel, text_expr, markdown_expr, thread_expr],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: vec![target],
                after: current_after,
            });
            index = next_index;
        } else if trimmed.strip_prefix("write ").is_some() {
            // write <format> to <store> at <path> { body <expr> mode <mode> } as
            // <binding> (std.files). The block spans lines, so gather it; the
            // body expression resolves at commit against the (after-)context, so
            // it is carried raw in `args` for the input builder.
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let header = statement.split('{').next().unwrap_or(&statement);
            let after_write = header.trim().strip_prefix("write ").unwrap_or("").trim();
            let mut to_split = after_write.splitn(2, " to ");
            let format = to_split.next().unwrap_or("text").trim().to_owned();
            let after_to = to_split.next().unwrap_or("");
            let mut at_split = after_to.splitn(2, " at ");
            let store = at_split.next().unwrap_or("").trim().to_owned();
            let path = at_split
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_owned();
            // Extract `body` / `mode` from the block, tolerant of either order:
            // scan tokens, collecting the body expression up to `mode`.
            let inner = invoke_body(&statement).unwrap_or_default();
            let tokens = inner.split_whitespace().collect::<Vec<_>>();
            let mut body_tokens = Vec::new();
            let mut mode = String::new();
            let mut cursor = 0;
            while cursor < tokens.len() {
                match tokens[cursor] {
                    "body" => {
                        cursor += 1;
                        while cursor < tokens.len() && tokens[cursor] != "mode" {
                            body_tokens.push(tokens[cursor]);
                            cursor += 1;
                        }
                    }
                    "mode" => {
                        cursor += 1;
                        if cursor < tokens.len() {
                            mode = tokens[cursor].trim_matches('"').to_owned();
                            cursor += 1;
                        }
                    }
                    _ => cursor += 1,
                }
            }
            let body_expr = body_tokens.join(" ");
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "file.write".to_owned(),
                target: Some(store.clone()),
                name: Some(format.clone()),
                binding: binding_after_as(&statement),
                args: vec![format, store, path, body_expr, mode],
                prompt: None,
                prompt_content_type: None,
                // v0: the `file store` `root` is the scope boundary (mirrors
                // `read`); a `files.write` capability-grant layer is a follow-up.
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            // import <format> <Schema> from <store> at <path> as <binding>
            // (std.files): format/schema/store/path are stashed in `args` for the
            // input builder, which also attaches the schema's required fields.
            let before_as = rest.split(" as ").next().unwrap_or(rest).trim();
            let mut from_split = before_as.splitn(2, " from ");
            let head = from_split.next().unwrap_or("").trim();
            let mut head_words = head.split_whitespace();
            let format = head_words.next().unwrap_or("jsonl").to_owned();
            let schema = head_words.next().unwrap_or_default().to_owned();
            let after_from = from_split.next().unwrap_or("");
            let mut at_split = after_from.splitn(2, " at ");
            let store = at_split.next().unwrap_or("").trim().to_owned();
            let path = at_split
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "file.import".to_owned(),
                target: Some(store.clone()),
                name: Some(schema.clone()),
                binding: binding_after_as(trimmed),
                args: vec![format, schema, store, path],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if trimmed.strip_prefix("export ").is_some() {
            // export <format> <Schema> to <store> at <path> { [where <pred>] mode
            // <mode> } as <binding> (std.files). The block spans lines; the where
            // predicate + mode are extracted by a token scan (DR-0022).
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let header = statement.split('{').next().unwrap_or(&statement);
            let after_export = header.trim().strip_prefix("export ").unwrap_or("").trim();
            let mut head_words = after_export.split_whitespace();
            let format = head_words.next().unwrap_or("jsonl").to_owned();
            let schema = head_words.next().unwrap_or_default().to_owned();
            let after_to = after_export.split_once(" to ").map_or("", |(_, rest)| rest);
            let mut at_split = after_to.splitn(2, " at ");
            let store = at_split.next().unwrap_or("").trim().to_owned();
            let path = at_split
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_owned();
            // Block fields: optional `where <pred>` then `mode <mode>`.
            let inner = invoke_body(&statement).unwrap_or_default();
            let tokens = inner.split_whitespace().collect::<Vec<_>>();
            let mut where_tokens = Vec::new();
            let mut mode = String::new();
            let mut cursor = 0;
            while cursor < tokens.len() {
                match tokens[cursor] {
                    "where" => {
                        cursor += 1;
                        while cursor < tokens.len() && tokens[cursor] != "mode" {
                            where_tokens.push(tokens[cursor]);
                            cursor += 1;
                        }
                    }
                    "mode" => {
                        cursor += 1;
                        if cursor < tokens.len() {
                            mode = tokens[cursor].trim_matches('"').to_owned();
                            cursor += 1;
                        }
                    }
                    _ => cursor += 1,
                }
            }
            let predicate = where_tokens.join(" ");
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "file.export".to_owned(),
                target: Some(store.clone()),
                name: Some(schema.clone()),
                binding: binding_after_as(&statement),
                args: vec![format, schema, store, path, predicate, mode],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("timer until ") {
            // Absolute deadline (spec/scheduled-time.md): the operand is a
            // time literal or a time-typed path resolved from context.
            let operand = rest
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                .to_owned();
            let deadline = parse_field_value(&operand, context)
                .as_str()
                .map(str::to_owned)
                .unwrap_or(operand);
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "timer.wait".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![deadline],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("timer ") {
            let duration = rest.split_whitespace().next().unwrap_or_default();
            let duration_seconds =
                whipplescript_parser::body::parse_short_duration_seconds(duration)
                    .map(|seconds| seconds as i64);
            effects.push(ParsedEffect {
                timeout_seconds: duration_seconds,
                kind: "timer.wait".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![duration.to_owned()],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("file ") {
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let queue = rest
                .strip_prefix("item into ")
                .and_then(|tail| tail.split_whitespace().next())
                .unwrap_or_default()
                .trim_end_matches('{')
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "queue.file".to_owned(),
                target: Some(queue),
                name: None,
                binding: binding_after_as(&statement),
                args: vec![body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("claim ") && !trimmed.contains(" with ") {
            let item = trimmed
                .strip_prefix("claim ")
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "queue.claim".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![item],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("release ") {
            let item = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "queue.release".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![item],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("finish ") {
            let (statement, next_index) = if trimmed.contains('{') {
                parse_statement_until_balanced_braces(&lines, index, trimmed)
            } else {
                (trimmed.to_owned(), index)
            };
            let item = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "queue.finish".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(&statement),
                args: vec![item, body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("decide ") {
            // Inline anonymous coercion: decide "<prompt>" -> { fields } as x.
            let prompt = rest
                .strip_prefix('"')
                .and_then(|tail| tail.split_once('"'))
                .map(|(prompt, _)| prompt.to_owned())
                .unwrap_or_default();
            let shape = trimmed
                .split_once("->")
                .and_then(|(_, tail)| tail.split_once('{'))
                .and_then(|(_, tail)| tail.split_once('}'))
                .map(|(shape, _)| shape.trim().to_owned())
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "coerce".to_owned(),
                target: None,
                name: Some("decide".to_owned()),
                binding: binding_after_as(trimmed),
                args: vec![shape],
                prompt: Some(interpolate_prompt(&prompt, context)),
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("exec ") {
            let mut target = None;
            let mut name = None;
            let mut args = Vec::new();
            let parse_spec = if rest.trim_start().starts_with('"') {
                // Escape-aware: a JSON-emitting command (`echo '{\"k\": 1}'`)
                // contains escaped quotes that a naive split would cut at.
                let (command, after_command) = extract_quoted_string(rest).unwrap_or_default();
                args.push(command);
                after_command
                    .trim_start()
                    .strip_prefix("->")
                    .map(str::trim)
                    .map(str::to_owned)
            } else {
                let capability = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let stdin_binding = rest
                    .split_once(" with ")
                    .map(|(_, tail)| {
                        tail.split("->")
                            .next()
                            .unwrap_or_default()
                            .split(" as ")
                            .next()
                            .unwrap_or_default()
                            .split(" timeout ")
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .to_owned()
                    })
                    .unwrap_or_default();
                target = Some(capability);
                name = Some("capability".to_owned());
                args.push(stdin_binding);
                rest.split_once("->")
                    .map(|(_, tail)| tail.trim().to_owned())
            };
            // `-> [each] Schema`: typed stdout ingestion contract
            // (spec/json-ingestion.md), resolved into the effect input.
            if let Some(spec) = parse_spec.as_deref().filter(|spec| !spec.is_empty()) {
                let mut words = spec.split_whitespace();
                match words.next() {
                    Some("each") => {
                        let schema = words.next().unwrap_or_default();
                        args.push(format!("each {schema}"));
                    }
                    Some(schema) => args.push(schema.to_owned()),
                    None => {}
                }
            }
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "exec.command".to_owned(),
                target,
                name,
                binding: binding_after_as(trimmed),
                args,
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("emit signal ") {
            // emit signal <name> to <instance-expr> { payload }
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let event = statement
                .strip_prefix("emit signal ")
                .unwrap_or_default()
                .split_once(" to ")
                .map(|(name, _)| name)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let target_expr = rest
                .split_once(" to ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('{')
                .to_owned();
            let fields = statement
                .split_once('{')
                .and_then(|(_, tail)| tail.rsplit_once('}'))
                .map(|(fields, _)| fields.to_owned())
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "signal.emit".to_owned(),
                target: None,
                name: Some(event.clone()),
                binding: binding_after_as(&statement),
                args: vec![target_expr, event, fields],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("acquire ") {
            // acquire <lease> for <key-expr> [until ttl] as <slot>
            let resource = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let key_expr = rest
                .split_once(" for ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .replace(" until ttl", "")
                .trim()
                .to_owned();
            let until_ttl = rest.contains(" until ttl");
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "lease.acquire".to_owned(),
                target: Some(resource),
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![key_expr, until_ttl.to_string()],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("append ") {
            // append <Schema> { fields } to <ledger> [as x]
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let schema = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let fields = statement
                .split_once('{')
                .and_then(|(_, tail)| tail.rsplit_once('}'))
                .map(|(fields, _)| fields.to_owned())
                .unwrap_or_default();
            let ledger = statement
                .rsplit_once(" to ")
                .map(|(_, tail)| {
                    tail.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "ledger.append".to_owned(),
                target: Some(ledger),
                name: None,
                binding: binding_after_as(&statement),
                args: vec![schema, fields],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("consume ")
            && trimmed.contains(" for ")
            && trimmed.contains(" amount ")
        {
            // consume <counter> for <key-expr> amount <expr> as <binding>
            let rest = trimmed.strip_prefix("consume ").unwrap_or_default();
            let counter = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let key_expr = rest
                .split_once(" for ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" amount ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let amount_expr = rest
                .split_once(" amount ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "counter.consume".to_owned(),
                target: Some(counter),
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![key_expr, amount_expr],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("tell ") {
            let target_expr = rest.split_whitespace().next().unwrap_or("agent");
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "agent.tell".to_owned(),
                target: Some(resolve_tell_target(target_expr, context)),
                name: None,
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: Some(interpolate_prompt(&prompt.text, context)),
                prompt_content_type: prompt.content_type,
                required_capabilities: parse_required_capabilities(trimmed),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("prompt ") {
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            let statement_end = next_index.min(lines.len().saturating_sub(1));
            let statement = lines[index..=statement_end]
                .iter()
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join(" ");
            // `prompt` requires an `as` binding (enforced by the parser); a bare
            // `prompt "..."` line is a payload field, not an effect.
            let binding = binding_after_as(&statement);
            if binding.is_some() {
                effects.push(ParsedEffect {
                    timeout_seconds: parse_timeout_clause_seconds(&statement),
                    kind: "coerce".to_owned(),
                    target: prompt_provider_after_using(&statement),
                    name: Some("prompt".to_owned()),
                    binding,
                    args: Vec::new(),
                    prompt: Some(interpolate_prompt(&prompt.text, context)),
                    prompt_content_type: prompt.content_type,
                    required_capabilities: parse_required_capabilities(&statement),
                    after: current_after,
                });
                index = next_index;
            }
        } else if let Some(rest) = trimmed.strip_prefix("coerce ") {
            let (statement, next_index) =
                parse_statement_until_balanced_parens(&lines, index, trimmed);
            let rest = statement.strip_prefix("coerce ").unwrap_or(rest);
            let call = rest.split(" as ").next().unwrap_or(rest).trim();
            let name = call.split_once('(').map(|(name, _)| name).unwrap_or(call);
            let args = call
                .split_once('(')
                .and_then(|(_, tail)| tail.rsplit_once(')').map(|(args, _)| args))
                .map(split_args)
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "coerce".to_owned(),
                target: Some(name.to_owned()),
                name: Some(name.to_owned()),
                binding: binding_after_as(trimmed),
                args,
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("claim ") && trimmed.contains(" with loft") {
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "loft.claim".to_owned(),
                target: Some("loft".to_owned()),
                name: Some("claim".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if trimmed.starts_with("askHuman ") {
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            // Typed choices declared in source drive the inbox options.
            let choices = trimmed
                .split_once("choices ")
                .and_then(|(_, tail)| tail.split_once('['))
                .and_then(|(_, tail)| tail.split_once(']'))
                .map(|(inner, _)| {
                    inner
                        .split(',')
                        .filter_map(|value| {
                            value
                                .trim()
                                .strip_prefix('"')
                                .and_then(|v| v.strip_suffix('"'))
                        })
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "human.ask".to_owned(),
                target: Some("human".to_owned()),
                name: Some("askHuman".to_owned()),
                binding: binding_after_as(trimmed),
                args: choices,
                prompt: Some(interpolate_prompt(&prompt.text, context)),
                prompt_content_type: prompt.content_type,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some((pool, query)) = parse_recall_statement(trimmed) {
            let target = "memory.query".to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "capability.call".to_owned(),
                target: Some(target.clone()),
                name: Some("recall".to_owned()),
                binding: binding_after_as(trimmed),
                args: vec![pool, query],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: vec![target],
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("call ") {
            let target = rest
                .split_whitespace()
                .next()
                .unwrap_or("plugin")
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "capability.call".to_owned(),
                target: Some(target.clone()),
                name: Some("call".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
                required_capabilities: vec![target],
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("emit ") {
            let event_type = rest
                .split_whitespace()
                .next()
                .unwrap_or("event.emitted")
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "event.emit".to_owned(),
                target: Some(event_type),
                name: Some("emit".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        }
        let delta = brace_delta(trimmed);
        for scope in &mut after_scopes {
            scope.depth += delta;
        }
        after_scopes.retain(|scope| scope.depth > 0);
        index += 1;
    }
    effects
}

pub fn parse_statement_until_balanced_parens(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> (String, usize) {
    let mut statement = trimmed.to_owned();
    let mut depth = paren_delta(trimmed);
    let mut cursor = index;
    while depth > 0 && cursor + 1 < lines.len() {
        cursor += 1;
        let next = lines[cursor].trim();
        statement.push(' ');
        statement.push_str(next);
        depth += paren_delta(next);
    }
    (statement, cursor)
}

pub fn parse_statement_until_balanced_braces(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> (String, usize) {
    let mut statement = trimmed.to_owned();
    let mut depth = brace_delta(trimmed);
    let mut cursor = index;
    while depth > 0 && cursor + 1 < lines.len() {
        cursor += 1;
        let next = lines[cursor].trim();
        statement.push(' ');
        statement.push_str(next);
        depth += brace_delta(next);
    }
    (statement, cursor)
}

pub fn invoke_body(statement: &str) -> Option<String> {
    let open = statement.find('{')?;
    let close = statement.rfind('}')?;
    (close > open).then(|| statement[open + 1..close].trim().to_owned())
}

pub fn paren_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    })
}

pub fn resolve_tell_target(target_expr: &str, context: &RuleContext) -> String {
    parse_field_value(target_expr, context)
        .as_str()
        .unwrap_or(target_expr)
        .to_owned()
}

pub fn parse_required_capabilities(line: &str) -> Vec<String> {
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

pub fn parsed_effect_input_json(
    ir: &IrProgram,
    rule: &IrRule,
    effect: &ParsedEffect,
    context: &RuleContext,
    effect_bindings: &std::collections::BTreeMap<String, String>,
    errors: &mut Vec<String>,
) -> String {
    let mut input = match effect.kind.as_str() {
        "agent.tell" => json!({
            "prompt": effect.prompt.as_deref().unwrap_or_default(),
            "access_grants": effect_access_grants_json(rule, effect, IrEffectKind::AgentTell),
            "rule": rule.name,
            "bindings": context_bindings_json(context),
        }),
        "file.read" => {
            let format = effect
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "text".to_owned());
            let store = effect.args.get(1).cloned().unwrap_or_default();
            let path = effect.args.get(2).cloned().unwrap_or_default();
            let declared = ir
                .file_stores
                .iter()
                .find(|file_store| file_store.name == store);
            let root = declared
                .map(|file_store| file_store.root.clone())
                .unwrap_or_default();
            let allow = declared
                .map(|file_store| file_store.read_globs.clone())
                .unwrap_or_default();
            json!({
                "format": format,
                "store": store,
                "path": path,
                "root": root,
                "allow": allow,
                "rule": rule.name,
            })
        }
        "file.write" => {
            let format = effect
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "text".to_owned());
            let store = effect.args.get(1).cloned().unwrap_or_default();
            let path = effect.args.get(2).cloned().unwrap_or_default();
            let body_expr = effect.args.get(3).cloned().unwrap_or_default();
            let mode = effect.args.get(4).cloned().unwrap_or_default();
            let declared = ir
                .file_stores
                .iter()
                .find(|file_store| file_store.name == store);
            let root = declared
                .map(|file_store| file_store.root.clone())
                .unwrap_or_default();
            let allow = declared
                .map(|file_store| file_store.write_globs.clone())
                .unwrap_or_default();
            // The body resolves at commit against the (after-)context — for a
            // write inside `after X succeeds as r { … }` the binding `r` is
            // already in `context`, so `r.field` evaluates here (no worker-time
            // resolution needed). A string value is the content; anything else is
            // rendered as its JSON text.
            let body_value = parse_field_value(&body_expr, context);
            let body = match &body_value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            json!({
                "format": format,
                "store": store,
                "path": path,
                "root": root,
                "allow": allow,
                "mode": mode,
                "body": body,
                "body_expr": body_expr,
                "rule": rule.name,
            })
        }
        "file.import" => {
            let format = effect
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "jsonl".to_owned());
            let schema = effect.args.get(1).cloned().unwrap_or_default();
            let store = effect.args.get(2).cloned().unwrap_or_default();
            let path = effect.args.get(3).cloned().unwrap_or_default();
            let declared = ir
                .file_stores
                .iter()
                .find(|file_store| file_store.name == store);
            let root = declared
                .map(|file_store| file_store.root.clone())
                .unwrap_or_default();
            let allow = declared
                .map(|file_store| file_store.read_globs.clone())
                .unwrap_or_default();
            // The row schema's required fields (non-optional, non-literal) travel
            // in the input so the worker validates each decoded row without the IR.
            let required_fields = ir
                .schemas
                .iter()
                .find_map(|candidate| match candidate {
                    IrSchema::Class(class) if class.name == schema => Some(class),
                    _ => None,
                })
                .map(|class| {
                    class
                        .fields
                        .iter()
                        .filter(|field| {
                            !matches!(field.ty, IrType::Optional(_) | IrType::LiteralString(_))
                        })
                        .map(|field| field.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            // The `@key` field (if any) keys per-row admission by content
            // (`H(effect_key, natural_key)`) instead of by row index.
            let natural_key_field = ir
                .schemas
                .iter()
                .find_map(|candidate| match candidate {
                    IrSchema::Class(class) if class.name == schema => class
                        .fields
                        .iter()
                        .find(|field| field.is_key)
                        .map(|field| field.name.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            json!({
                "format": format,
                "schema": schema,
                "store": store,
                "path": path,
                "root": root,
                "allow": allow,
                "required_fields": required_fields,
                "natural_key_field": natural_key_field,
                "rule": rule.name,
            })
        }
        "file.export" => {
            let format = effect
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "jsonl".to_owned());
            let schema = effect.args.get(1).cloned().unwrap_or_default();
            let store = effect.args.get(2).cloned().unwrap_or_default();
            let path = effect.args.get(3).cloned().unwrap_or_default();
            let predicate = effect.args.get(4).cloned().unwrap_or_default();
            let mode = effect.args.get(5).cloned().unwrap_or_default();
            let declared = ir
                .file_stores
                .iter()
                .find(|file_store| file_store.name == store);
            let root = declared
                .map(|file_store| file_store.root.clone())
                .unwrap_or_default();
            let allow = declared
                .map(|file_store| file_store.write_globs.clone())
                .unwrap_or_default();
            // The schema's field order (for the csv header / stable column order)
            // travels in the input so the worker serializes without the IR.
            let fields = ir
                .schemas
                .iter()
                .find_map(|candidate| match candidate {
                    IrSchema::Class(class) if class.name == schema => Some(class),
                    _ => None,
                })
                .map(|class| {
                    class
                        .fields
                        .iter()
                        .map(|field| field.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            json!({
                "format": format,
                "schema": schema,
                "store": store,
                "path": path,
                "root": root,
                "allow": allow,
                "mode": mode,
                "predicate": predicate,
                "fields": fields,
                "rule": rule.name,
            })
        }
        "coerce" => {
            let function_name = effect.name.as_deref().unwrap_or("coerce");
            let coerce_prompt = if function_name == "prompt" {
                Some(ParsedPrompt {
                    text: effect.prompt.clone().unwrap_or_default(),
                    content_type: effect.prompt_content_type.clone(),
                })
            } else {
                coerce_prompt_from_ir(ir, function_name)
            };
            let output_type = if function_name == "decide" {
                // Inline `decide -> { … } as <binding>` has no named coerce; its
                // anonymous shape was synthesized at lowering into a hygienic
                // `decide.<rule>.<binding>` class. Deriving the same name here
                // lets the schema-based fixture generate the declared shape so
                // `after <binding> succeeds as r` resolves `r`'s fields.
                whipplescript_parser::inline_decide_schema_name(
                    &rule.name,
                    effect.binding.as_deref().unwrap_or_default(),
                )
            } else if function_name == "prompt" {
                "string".to_owned()
            } else {
                ir.coerces
                    .iter()
                    .find(|coerce| coerce.name == function_name)
                    .map(|coerce| ir_type_name(&coerce.output))
                    .unwrap_or_else(|| "json".to_owned())
            };
            let mut arguments = serde_json::Map::new();
            for (index, arg) in effect.args.iter().enumerate() {
                arguments.insert(format!("arg{index}"), parse_field_value(arg, context));
            }
            let mut input = json!({
                "function_name": function_name,
                "arguments": Value::Object(arguments),
                "argument_exprs": effect.args,
                "output_type": output_type,
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            });
            // Sum-type output (spec/sum-types.md): embed deterministic
            // per-variant fixture values so a fixture run returns a tagged
            // variant (first declared by default; `--variant` selects an
            // arm) without the worker needing the IR.
            if let Some(decl) = ir.schemas.iter().find_map(|schema| match schema {
                IrSchema::Enum(decl) if decl.name == output_type => Some(decl),
                _ => None,
            }) {
                let has_payloads = ir.schemas.iter().any(|schema| {
                    matches!(schema, IrSchema::Class(class)
                        if class.name.starts_with(&format!("{}.", decl.name)))
                });
                if has_payloads {
                    let mut fixtures = serde_json::Map::new();
                    for variant in &decl.variants {
                        let generated = format!("{}.{variant}", decl.name);
                        let value = if ir.schemas.iter().any(|schema| {
                            matches!(schema, IrSchema::Class(class) if class.name == generated)
                        }) {
                            fixture_value_for_shape(&ingest_shape_json(
                                ir,
                                &IrType::Ref(generated),
                                0,
                            ))
                        } else {
                            Value::String(variant.clone())
                        };
                        fixtures.insert(variant.clone(), value);
                    }
                    if let Some(object) = input.as_object_mut() {
                        object.insert("fixture_variants".to_owned(), Value::Object(fixtures));
                        if let Some(first) = decl.variants.first() {
                            object
                                .insert("fixture_default".to_owned(), Value::String(first.clone()));
                        }
                    }
                }
            }
            if let Some(prompt) = coerce_prompt {
                if let Some(object) = input.as_object_mut() {
                    if function_name == "prompt" {
                        object.insert("prompt".to_owned(), Value::String(prompt.text.clone()));
                    }
                    object.insert("prompt_template".to_owned(), Value::String(prompt.text));
                    if let Some(content_type) = prompt.content_type {
                        object.insert(
                            "prompt_content_type".to_owned(),
                            Value::String(content_type),
                        );
                    }
                }
            }
            if function_name == "prompt" {
                if let (Some(provider), Some(object)) = (&effect.target, input.as_object_mut()) {
                    object.insert("provider".to_owned(), Value::String(provider.clone()));
                }
            }
            input
        }
        "loft.claim" => json!({
            "action": "claim",
            "issue": context_bindings_json(context),
            "rule": rule.name,
        }),
        "human.ask" => {
            let choices = if effect.args.is_empty() {
                json!(["accept", "revise", "block"])
            } else {
                json!(effect.args)
            };
            json!({
                "prompt": effect.prompt.as_deref().unwrap_or_default(),
                "choices": choices,
                "severity": "normal",
                "rule": rule.name,
            })
        }
        "queue.file" => {
            let fields = parse_record_fields(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            );
            json!({
                "queue": effect.target,
                "item": Value::Object(fields),
                "rule": rule.name,
            })
        }
        "queue.claim" | "queue.release" | "queue.finish" => {
            let binding = effect.args.first().map(String::as_str).unwrap_or_default();
            let item = parse_field_value(binding, context);
            let mut input = json!({
                "queue": item.get("queue").cloned().unwrap_or(Value::Null),
                "id": item.get("id").cloned().unwrap_or(Value::Null),
                "rule": rule.name,
            });
            if effect.kind == "queue.finish" {
                let fields = parse_record_fields(
                    effect.args.get(1).map(String::as_str).unwrap_or_default(),
                    context,
                    None,
                    errors,
                );
                insert_json_field(&mut input, "payload", Value::Object(fields));
            }
            input
        }
        "signal.emit" => {
            let event_name = effect.args.get(1).cloned().unwrap_or_default();
            let event = ir.events.iter().find(|event| event.name == event_name);
            if event.is_none() {
                errors.push(format!("emit signal of undeclared signal `{event_name}`"));
            }
            let target = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            let payload = Value::Object(parse_record_fields(
                effect.args.get(2).map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            ));
            let shape = event
                .map(|event| ingest_shape_json(ir, &IrType::Object(event.fields.clone()), 0))
                .unwrap_or(Value::Null);
            json!({
                "target_instance": coordination_key_string(&target),
                "event": event_name,
                "payload": payload,
                "shape": shape,
                "rule": rule.name,
            })
        }
        "lease.acquire" => {
            let lease = ir
                .leases
                .iter()
                .find(|lease| Some(&lease.name) == effect.target.as_ref());
            if lease.is_none() {
                errors.push(format!(
                    "acquire of undeclared lease `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let key = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            json!({
                "resource": effect.target,
                "coordination_owner": lease.and_then(|lease| lease.shared.then_some("shared")).unwrap_or_default(),
                "key": coordination_key_string(&key),
                "slots": lease.map(|lease| lease.slots).unwrap_or(1),
                "ttl_seconds": lease.map(|lease| lease.ttl_seconds).unwrap_or(600),
                "until_ttl": effect.args.get(1).map(String::as_str) == Some("true"),
                "rule": rule.name,
            })
        }
        "lease.release" => json!({
            "acquire_effect_id": effect
                .args
                .first()
                .and_then(|binding| effect_bindings.get(binding)),
            "rule": rule.name,
        }),
        "ledger.append" => {
            let ledger = ir
                .ledgers
                .iter()
                .find(|ledger| Some(&ledger.name) == effect.target.as_ref());
            if ledger.is_none() {
                errors.push(format!(
                    "append to undeclared ledger `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let entry = Value::Object(parse_record_fields(
                effect.args.get(1).map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            ));
            let partition = ledger
                .and_then(|ledger| entry.get(&ledger.partition_field))
                .map(coordination_key_string)
                .unwrap_or_default();
            json!({
                "ledger": effect.target,
                "coordination_owner": ledger.and_then(|ledger| ledger.shared.then_some("shared")).unwrap_or_default(),
                "schema": effect.args.first().cloned().unwrap_or_default(),
                "entry": entry,
                "partition": partition,
                "retain_seconds": ledger.map(|ledger| ledger.retain_seconds).unwrap_or(86400),
                "rule": rule.name,
            })
        }
        "counter.consume" => {
            let counter = ir
                .counters
                .iter()
                .find(|counter| Some(&counter.name) == effect.target.as_ref());
            if counter.is_none() {
                errors.push(format!(
                    "consume of undeclared counter `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let key = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            let amount = parse_field_value(
                effect.args.get(1).map(String::as_str).unwrap_or_default(),
                context,
            );
            json!({
                "counter": effect.target,
                "coordination_owner": counter.and_then(|counter| counter.shared.then_some("shared")).unwrap_or_default(),
                "key": coordination_key_string(&key),
                "amount": amount.as_i64().unwrap_or(0),
                "cap": counter.map(|counter| counter.cap).unwrap_or(0),
                "reset": counter.map(|counter| counter.reset.clone()).unwrap_or_else(|| "daily".to_owned()),
                "rule": rule.name,
            })
        }
        "exec.command" => {
            let mut input = if effect.name.as_deref() == Some("capability") {
                let stdin_expr = effect.args.first().map(String::as_str).unwrap_or_default();
                json!({
                    "mode": "capability",
                    "capability": effect.target,
                    "stdin": parse_field_value(stdin_expr, context),
                    "stdin_binding": stdin_expr,
                    "rule": rule.name,
                })
            } else {
                json!({
                    "mode": "raw",
                    "command": effect.args.first().cloned().unwrap_or_default(),
                    "rule": rule.name,
                })
            };
            let parse_spec_index = 1;
            if let Some(spec) = effect.args.get(parse_spec_index) {
                let (each, schema) = match spec.strip_prefix("each ") {
                    Some(schema) => (true, schema),
                    None => (false, spec.as_str()),
                };
                insert_json_field(
                    &mut input,
                    "parse",
                    json!({
                        "schema": schema,
                        "each": each,
                        "shape": ingest_shape_json(ir, &IrType::Ref(schema.to_owned()), 0),
                    }),
                );
            }
            input
        }
        "timer.wait" if effect.timeout_seconds.is_none() => json!({
            "deadline_at": effect.args.first().cloned().unwrap_or_default(),
            "rule": rule.name,
        }),
        "timer.wait" => json!({
            "duration": effect.args.first().cloned().unwrap_or_default(),
            "duration_seconds": effect.timeout_seconds,
            "rule": rule.name,
        }),
        "capability.call" if effect.name.as_deref() == Some("recall") => {
            let pool = effect.args.first().cloned().unwrap_or_default();
            let query_expr = effect.args.get(1).cloned().unwrap_or_default();
            json!({
                "target": effect.target,
                "source_form": "recall",
                "pool": pool,
                "query": parse_field_value(&query_expr, context),
                "query_expr": query_expr,
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            })
        }
        "capability.call" if effect.name.as_deref() == Some("send") => {
            let channel = effect.args.first().cloned().unwrap_or_default();
            let text_expr = effect.args.get(1).cloned().unwrap_or_default();
            let markdown_expr = effect.args.get(2).cloned().unwrap_or_default();
            let thread_expr = effect.args.get(3).cloned().unwrap_or_default();
            let mut message = serde_json::Map::new();
            // Thread the referenced channel's declared provider into the effect
            // input so provider *selection* is visible to the host projection
            // without new plumbing. If the channel isn't declared in the IR, omit.
            if let Some(decl) = ir.channels.iter().find(|c| c.name == channel) {
                message.insert("provider".to_owned(), Value::String(decl.provider.clone()));
            }
            message.insert("channel".to_owned(), Value::String(channel));
            message.insert("text".to_owned(), parse_field_value(&text_expr, context));
            if !markdown_expr.is_empty() {
                message.insert(
                    "markdown".to_owned(),
                    parse_field_value(&markdown_expr, context),
                );
            }
            if !thread_expr.is_empty() {
                message.insert(
                    "thread_id".to_owned(),
                    parse_field_value(&thread_expr, context),
                );
            }
            json!({
                "target": effect.target,
                "source_form": "send",
                "message": Value::Object(message),
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            })
        }
        "capability.call" => json!({
            "target": effect.target,
            "bindings": context_bindings_json(context),
            "rule": rule.name,
        }),
        "event.emit" => json!({
            "event_type": effect.target,
            "payload": {
                "rule": rule.name,
                "bindings": context_bindings_json(context),
            },
            "bindings": context_bindings_json(context),
            "rule": rule.name,
        }),
        "workflow.invoke" => {
            let body = effect.args.first().map(String::as_str).unwrap_or_default();
            json!({
                "target_workflow": effect.target,
                "input": Value::Object(parse_record_fields(body, context, None, errors)),
                "access_grants": effect_access_grants_json(rule, effect, IrEffectKind::WorkflowInvoke),
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            })
        }
        _ => json!({"rule": rule.name}),
    };
    if let Some(after) = &effect.after {
        if let Some(upstream_effect_id) = effect_bindings.get(&after.binding) {
            if let Some(object) = input.as_object_mut() {
                object.insert(
                    "after".to_owned(),
                    json!({
                        "binding": after.binding,
                        "predicate": after.predicate,
                        "upstream_effect_id": upstream_effect_id,
                    }),
                );
            }
        }
    }
    if matches!(effect.kind.as_str(), "agent.tell" | "human.ask" | "coerce") {
        if let Some(content_type) = &effect.prompt_content_type {
            if let Some(object) = input.as_object_mut() {
                object.insert(
                    "prompt_content_type".to_owned(),
                    Value::String(content_type.clone()),
                );
            }
        }
    }
    input.to_string()
}

pub fn effect_access_grants_json(
    rule: &IrRule,
    effect: &ParsedEffect,
    kind: IrEffectKind,
) -> Value {
    let Some(node) = rule.metadata.effects.iter().find(|node| {
        if node.kind != kind {
            return false;
        }
        if let Some(binding) = &effect.binding {
            if node.binding.as_ref() != Some(binding) {
                return false;
            }
        }
        match kind {
            IrEffectKind::AgentTell => node.agent == effect.target,
            IrEffectKind::WorkflowInvoke => node.workflow_target == effect.target,
            _ => true,
        }
    }) else {
        return json!([]);
    };
    Value::Array(
        node.access_grants
            .iter()
            .map(|grant| {
                json!({
                    "resource": grant.resource,
                    "operations": grant
                        .operations
                        .iter()
                        .map(|op| {
                            json!({
                                "operation": op.operation,
                                "target": op.target,
                                "globs": op.globs,
                            })
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

pub fn coerce_prompt_from_ir(ir: &IrProgram, function_name: &str) -> Option<ParsedPrompt> {
    let coerce = ir
        .coerces
        .iter()
        .find(|coerce| coerce.name == function_name)?;
    let lines = coerce.body.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("prompt ") || trimmed == "prompt" {
            return Some(parse_prompt_from_lines(&lines, index, trimmed).0);
        }
    }
    None
}

pub fn parse_after_scope(trimmed: &str) -> Option<AfterScope> {
    let rest = trimmed.strip_prefix("after ")?;
    let (binding, predicate, _) = parse_after_header(rest)?;
    Some(AfterScope { binding, predicate })
}

pub fn binding_after_as(line: &str) -> Option<String> {
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

pub fn prompt_provider_after_using(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "using" {
            return tokens
                .next()
                .map(|provider| {
                    provider.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_')
                })
                .filter(|provider| !provider.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

pub fn parse_recall_statement(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("recall ")?.trim_start();
    let pool = rest.split_whitespace().next()?.to_owned();
    let after_pool = rest.get(pool.len()..)?.trim_start();
    let query_tail = after_pool.strip_prefix("for ")?.trim_start();
    let query_end = [" as ", " timeout ", " requires "]
        .iter()
        .filter_map(|marker| query_tail.find(marker))
        .min()
        .unwrap_or(query_tail.len());
    let query = query_tail[..query_end].trim();
    if pool.is_empty() || query.is_empty() {
        return None;
    }
    Some((pool, query.to_owned()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedPrompt {
    pub text: String,
    pub content_type: Option<String>,
}

pub fn parse_prompt_from_lines(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> (ParsedPrompt, usize) {
    if trimmed.contains("\"\"\"") {
        let mut prompt_lines = Vec::new();
        let after_open = trimmed
            .split_once("\"\"\"")
            .map(|(_, tail)| tail)
            .unwrap_or("");
        let content_type = prompt_content_type_from_opening_tail(after_open);
        if !after_open.is_empty() && content_type.is_none() {
            prompt_lines.push(after_open.to_owned());
        }
        let mut cursor = index + 1;
        while cursor < lines.len() {
            let line = lines[cursor];
            if let Some((head, _tail)) = line.split_once("\"\"\"") {
                prompt_lines.push(head.to_owned());
                return (
                    ParsedPrompt {
                        text: prompt_lines.join("\n").trim().to_owned(),
                        content_type,
                    },
                    cursor,
                );
            }
            prompt_lines.push(line.to_owned());
            cursor += 1;
        }
        return (
            ParsedPrompt {
                text: prompt_lines.join("\n").trim().to_owned(),
                content_type,
            },
            cursor,
        );
    }
    let prompt = trimmed
        .split_once('"')
        .and_then(|(_, tail)| tail.rsplit_once('"').map(|(prompt, _)| prompt))
        .unwrap_or("")
        .to_owned();
    (
        ParsedPrompt {
            text: prompt,
            content_type: None,
        },
        index,
    )
}

pub fn prompt_content_type_from_opening_tail(after_open: &str) -> Option<String> {
    let candidate = after_open.trim();
    if candidate.is_empty() || candidate.contains("\"\"\"") {
        return None;
    }
    is_supported_prompt_content_type(candidate).then(|| candidate.to_ascii_lowercase())
}

pub fn is_supported_prompt_content_type(candidate: &str) -> bool {
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

pub fn is_prompt_content_type_token(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '+' | '-' | '_'))
}

pub fn dependency_predicate_str(predicate: &IrDependencyPredicate) -> &'static str {
    match predicate {
        IrDependencyPredicate::Succeeds => "succeeds",
        IrDependencyPredicate::Fails => "fails",
        IrDependencyPredicate::TimedOut => "timed_out",
        IrDependencyPredicate::Cancelled => "cancelled",
        IrDependencyPredicate::Completes => "completes",
    }
}

pub fn normalize_pattern_name(pattern: &str) -> String {
    whipplescript_parser::runtime_fact_name_for_pattern(pattern)
        .unwrap_or_else(|| pattern.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Matches `<agent-or-worker> completed turn ...` readiness patterns and
/// returns the leading word. `worker` is the generic form (any agent).
pub fn completed_turn_agent(pattern: &str) -> Option<&str> {
    let mut words = pattern.split_whitespace();
    let first = words.next()?;
    if words.next() == Some("completed") && words.next() == Some("turn") {
        Some(first)
    } else {
        None
    }
}

pub fn ir_type_name(ty: &IrType) -> String {
    match ty {
        IrType::Primitive(primitive) => match primitive {
            IrPrimitiveType::String => "string",
            IrPrimitiveType::Int => "int",
            IrPrimitiveType::Float => "float",
            IrPrimitiveType::Bool => "bool",
            IrPrimitiveType::Null => "null",
            IrPrimitiveType::Duration => "duration",
            IrPrimitiveType::Time => "time",
            IrPrimitiveType::Image => "image",
            IrPrimitiveType::Audio => "audio",
            IrPrimitiveType::Pdf => "pdf",
            IrPrimitiveType::Video => "video",
        }
        .to_owned(),
        IrType::LiteralString(value) | IrType::Ref(value) => value.clone(),
        IrType::AgentRef(agents) => format!("AgentRef<{}>", agents.join(" | ")),
        IrType::Optional(inner) => ir_type_name(inner),
        IrType::Array(inner) => format!("{}[]", ir_type_name(inner)),
        IrType::Map(inner) => format!("map<{}>", ir_type_name(inner)),
        IrType::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|field| format!("{} {}", field.name, ir_type_name(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        IrType::Union(types) => types
            .iter()
            .map(ir_type_name)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

pub fn top_level_record_blocks(body: &str) -> Vec<RecordBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut skip_depth = 0i32;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if skip_depth > 0 {
            skip_depth += brace_delta(trimmed);
            index += 1;
            continue;
        }
        if trimmed.starts_with("after ") {
            skip_depth += brace_delta(trimmed).max(1);
            index += 1;
            continue;
        }
        let record_rest = trimmed.strip_prefix("record ").or_else(|| {
            trimmed
                .strip_prefix("done ")
                .and_then(|rest| rest.split_once("->"))
                .map(|(_, record)| record.trim())
                .and_then(|record| record.strip_prefix("record "))
        });
        if let Some(rest) = record_rest {
            let Some((schema, from_binding)) = parse_record_header(rest) else {
                index += 1;
                continue;
            };
            // Inline form: the block opens and closes on the statement line.
            if let Some(body) = inline_block_body(trimmed) {
                blocks.push(RecordBlock {
                    schema,
                    from_binding,
                    body,
                });
                index += 1;
                continue;
            }
            let mut record_lines = Vec::new();
            let mut depth = brace_delta(trimmed);
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
            blocks.push(RecordBlock {
                schema,
                from_binding,
                body: record_lines.join("\n"),
            });
            continue;
        }
        index += 1;
    }
    blocks
}

/// Extracts the inner text of a `{ ... }` block that opens and closes on the
/// same statement line, e.g. `complete result { total 2 }`.
pub fn inline_block_body(line: &str) -> Option<String> {
    let open = line.find('{')?;
    let close = line.rfind('}')?;
    if close <= open || brace_delta(line) != 0 {
        return None;
    }
    Some(line[open + 1..close].trim().to_owned())
}

/// A child-projected milestone (`emit milestone "<name>" [of <Class>] { fields }`,
/// Family C). The runtime derives one durable `workflow.milestone:<name>` fact per
/// block in the child's own base; the parent's invoke effect later observes it.
pub struct MilestoneBlock {
    pub name: String,
    pub body: String,
}

/// Extracts top-level `emit milestone` projections from a rule body (skipping
/// `after` blocks, which are processed in the after-pass). Mirrors
/// `top_level_record_blocks`.
pub fn milestone_blocks(body: &str) -> Vec<MilestoneBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut skip_depth = 0i32;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if skip_depth > 0 {
            skip_depth += brace_delta(trimmed);
            index += 1;
            continue;
        }
        if trimmed.starts_with("after ") {
            skip_depth += brace_delta(trimmed).max(1);
            index += 1;
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("emit milestone ") else {
            index += 1;
            continue;
        };
        let Some((name, _class)) = parse_milestone_header(rest) else {
            index += 1;
            continue;
        };
        // Bare milestone (no `{ }`): a payload-less projection.
        if !trimmed.contains('{') {
            blocks.push(MilestoneBlock {
                name,
                body: String::new(),
            });
            index += 1;
            continue;
        }
        if let Some(inner) = inline_block_body(trimmed) {
            blocks.push(MilestoneBlock { name, body: inner });
            index += 1;
            continue;
        }
        let mut milestone_lines = Vec::new();
        let mut depth = brace_delta(trimmed);
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let before = depth;
            depth += brace_delta(line);
            if !(before == 1 && depth == 0 && line.trim() == "}") {
                milestone_lines.push(line.to_owned());
            }
            index += 1;
        }
        blocks.push(MilestoneBlock {
            name,
            body: milestone_lines.join("\n"),
        });
    }
    blocks
}

/// Parses `"<name>" [of <Class>]` from an `emit milestone ` statement tail,
/// returning (name, optional class).
pub fn parse_milestone_header(rest: &str) -> Option<(String, Option<String>)> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let close = rest.find('"')?;
    let name = rest[..close].to_owned();
    let after = rest[close + 1..].trim_start();
    let class = after.strip_prefix("of ").map(|tail| {
        tail.trim_start()
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("")
            .to_owned()
    });
    Some((name, class))
}

pub fn top_level_terminal_blocks(body: &str) -> Vec<TerminalBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut skip_depth = 0i32;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if skip_depth > 0 {
            skip_depth += brace_delta(trimmed);
            index += 1;
            continue;
        }
        if trimmed.starts_with("after ") {
            skip_depth += brace_delta(trimmed).max(1);
            index += 1;
            continue;
        }
        let terminal = trimmed
            .strip_prefix("complete ")
            .map(|rest| (WorkflowTerminalKind::Completed, rest))
            .or_else(|| {
                trimmed
                    .strip_prefix("fail ")
                    .map(|rest| (WorkflowTerminalKind::Failed, rest))
            });
        let Some((kind, rest)) = terminal else {
            index += 1;
            continue;
        };
        // Header is `<name>`, `<name> from <binding>` (bounded-type projection), or
        // `<name> <scalar-value>` (bare scalar payload, no block).
        let header = rest.split('{').next().unwrap_or("");
        let tokens: Vec<&str> = header.split_whitespace().collect();
        let Some(name) = tokens
            .first()
            .filter(|n| is_identifier(n))
            .map(|n| (*n).to_owned())
        else {
            index += 1;
            continue;
        };
        let has_brace = rest.contains('{');
        let is_from =
            matches!(tokens.as_slice(), [_, "from", ..]) && kind == WorkflowTerminalKind::Completed;
        // Scalar terminal: a bare value after the name, no block, no `from`.
        if !has_brace && tokens.len() >= 2 && !is_from {
            let value = header
                .trim()
                .get(name.len()..)
                .unwrap_or("")
                .trim()
                .to_owned();
            blocks.push(TerminalBlock {
                kind,
                name,
                from: None,
                body: String::new(),
                scalar: Some(value),
            });
            index += 1;
            continue;
        }
        let from = match (tokens.get(1), tokens.get(2), tokens.get(3)) {
            (None, _, _) => None,
            (Some(&"from"), Some(binding), None) if is_identifier(binding) => {
                Some((*binding).to_owned())
            }
            _ => {
                index += 1;
                continue;
            }
        };
        if let Some(body) = inline_block_body(trimmed) {
            blocks.push(TerminalBlock {
                kind,
                name,
                from,
                body,
                scalar: None,
            });
            index += 1;
            continue;
        }
        let mut block_lines = Vec::new();
        let mut depth = brace_delta(trimmed);
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let before = depth;
            depth += brace_delta(line);
            if !(before == 1 && depth == 0 && line.trim() == "}") {
                block_lines.push(line.to_owned());
            }
            index += 1;
        }
        blocks.push(TerminalBlock {
            kind,
            name,
            from,
            body: block_lines.join("\n"),
            scalar: None,
        });
    }
    blocks
}

/// 503 auto-fail: whether `body` contains a top-level generated `flowfail`
/// terminal (a bare keyword line), skipping nested `after` blocks the same way
/// `top_level_terminal_blocks` does. Emitted by flow expansion inside an
/// `after <step> fails { flowfail }` block; routes to `fail_instance_internal`.
pub fn body_has_top_level_flowfail(body: &str) -> bool {
    let mut skip_depth = 0i32;
    for line in body.lines() {
        let trimmed = line.trim();
        if skip_depth > 0 {
            skip_depth += brace_delta(trimmed);
            continue;
        }
        if trimmed.starts_with("after ") {
            skip_depth += brace_delta(trimmed).max(1);
            continue;
        }
        if trimmed == "flowfail" {
            return true;
        }
    }
    false
}

pub fn parse_record_header(rest: &str) -> Option<(String, Option<String>)> {
    let before_brace = rest.split('{').next().unwrap_or(rest).trim();
    let mut parts = before_brace.split_whitespace();
    let schema = parts.next()?.to_owned();
    let from_binding = match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => None,
        (Some("from"), Some(binding), None) if is_identifier(binding) => Some(binding.to_owned()),
        _ => return None,
    };
    Some((schema, from_binding))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AfterBlock {
    pub binding: String,
    pub predicate: String,
    pub alias: Option<String>,
    pub body: String,
}

pub fn after_blocks(body: &str) -> Vec<AfterBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(rest) = trimmed.strip_prefix("after ") else {
            index += 1;
            continue;
        };
        let Some((binding, predicate, alias)) = parse_after_header(rest) else {
            index += 1;
            continue;
        };
        let mut inner = Vec::new();
        if brace_delta(trimmed) == 0 && trimmed.contains('{') {
            // Single-line after block: `after x succeeds { <stmts> }` opens and
            // closes on this line, so its body never reaches the multi-line loop.
            // Capture the content between the braces (mirrors the terminal-block
            // single-line fix).
            if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
                if close > open {
                    let body = trimmed[open + 1..close].trim();
                    if !body.is_empty() {
                        inner.push(body.to_owned());
                    }
                }
            }
            index += 1;
        } else {
            let mut depth = brace_delta(trimmed).max(1);
            index += 1;
            while index < lines.len() && depth > 0 {
                let line = lines[index];
                let next_depth = depth + brace_delta(line);
                if next_depth >= 1 {
                    inner.push(line.to_owned());
                }
                depth = next_depth;
                index += 1;
            }
        }
        blocks.push(AfterBlock {
            binding,
            predicate,
            alias,
            body: inner.join("\n"),
        });
    }
    blocks
}

pub fn parse_after_header(rest: &str) -> Option<(String, String, Option<String>)> {
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
    let predicate = parts.next()?.to_owned();
    // `after p reaches "<name>" [as m]` (Family C): fold the quoted milestone
    // name into the predicate string as `reaches:<name>` so the downstream
    // matcher keys on the milestone-specific `workflow.invoke.reached:<name>`
    // fact. The name lives in the predicate (not a separate slot) to reuse the
    // existing (binding, predicate, alias) plumbing untouched everywhere else.
    if predicate == "reaches" {
        let name = parts.next()?.trim_matches('"').to_owned();
        let alias = match (parts.next(), parts.next(), parts.next()) {
            (None, None, None) => None,
            (Some("as"), Some(alias), None) if is_identifier(alias) => Some(alias.to_owned()),
            _ => return None,
        };
        return Some((binding, format!("reaches:{name}"), alias));
    }
    let alias = match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => None,
        (Some("as"), Some(alias), None) if is_identifier(alias) => Some(alias.to_owned()),
        _ => return None,
    };
    Some((binding, predicate, alias))
}

pub fn effect_binding_value(
    facts: &[FactView],
    upstream_effect_id: &str,
    predicate: &str,
) -> Option<Value> {
    // A human ask emits both `human.ask.issued` (an issuance ack) and
    // `human.answer.received` (the real terminal). Both can satisfy a predicate,
    // so prefer the answer fact when present; otherwise take the first match.
    let matches: Vec<Value> = facts
        .iter()
        .filter_map(|fact| {
            let payload = json_from_str(&fact.value_json);
            if payload.get("effect_id").and_then(Value::as_str) != Some(upstream_effect_id) {
                return None;
            }
            if !fact_matches_after_predicate(&fact.name, &payload, predicate) {
                return None;
            }
            Some(payload)
        })
        .collect();
    let payload = matches
        .iter()
        .find(|payload| payload.get("inbox_item_id").is_some() && payload.get("choice").is_some())
        .or_else(|| matches.first())?
        .clone();
    if predicate == "completes" {
        return Some(terminal_union_value(&payload));
    }
    Some(
        payload
            .get("value")
            .cloned()
            .or_else(|| payload.get("output").cloned())
            .or_else(|| payload.get("error").cloned())
            .unwrap_or(payload),
    )
}

pub fn terminal_union_value(payload: &Value) -> Value {
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let tag = terminal_tag_for_status(status).unwrap_or("Completed");
    let value = payload
        .get("value")
        .cloned()
        .or_else(|| payload.get("output").cloned())
        .or_else(|| {
            // A human answer carries no `value`/`output` field; its structured
            // answer (the chosen option and/or free text) is the terminal value,
            // so `case ask { Completed as decided => decided.choice }` resolves.
            (payload.get("inbox_item_id").is_some()
                && (payload.get("choice").is_some() || payload.get("text").is_some()))
            .then(|| {
                json!({
                    "choice": payload.get("choice").cloned().unwrap_or(Value::Null),
                    "text": payload.get("text").cloned().unwrap_or(Value::Null),
                })
            })
        })
        .unwrap_or(Value::Null);
    json!({
        "tag": tag,
        "status": status,
        "value": value,
        "error": payload.get("error").cloned().or_else(|| payload.get("failure").cloned()).unwrap_or(Value::Null),
        "summary": payload.get("summary").cloned().unwrap_or(Value::Null),
        "effect_id": payload.get("effect_id").cloned().unwrap_or(Value::Null),
        "run_id": payload.get("run_id").cloned().unwrap_or(Value::Null),
    })
}

pub fn fact_matches_after_predicate(name: &str, payload: &Value, predicate: &str) -> bool {
    let status = payload.get("status").and_then(Value::as_str);
    match predicate {
        "succeeds" => {
            // A terminal-marker fact like `workflow.invoke.completed` is emitted
            // for BOTH success and failure (so `after x completes` can fire on
            // either), carrying a `status`. It only satisfies `succeeds` when
            // that status is not a failure — otherwise a failed child's
            // `.completed` fact would wrongly trigger the `after x succeeds`
            // branch and bind its success value to the failure payload.
            // `.succeeded` is emitted only on success; a missing status counts
            // as success.
            !matches!(status, Some("failed" | "timed_out" | "cancelled"))
                && (name.ends_with(".succeeded")
                    || name.ends_with(".completed")
                    || status == Some("completed"))
        }
        "fails" => name.ends_with(".failed") || matches!(status, Some("failed" | "timed_out")),
        // `reaches:<name>` (Family C): the parent's invoke effect derives a
        // `workflow.invoke.reached:<name>` fact for each child milestone it
        // observed. Match exactly that milestone (never the terminal facts), so a
        // milestone the child never emitted produces no reaction (terminal-only
        // observation).
        reaches if reaches.starts_with("reaches:") => {
            let milestone = &reaches["reaches:".len()..];
            name == format!("workflow.invoke.reached:{milestone}")
        }
        // Coordination outcomes (spec/coordination.md): the op completed and
        // its sum-typed value carries the matching variant.
        "held" | "contended" | "ok" | "over" => {
            status == Some("completed")
                && payload
                    .pointer("/value/variant")
                    .and_then(Value::as_str)
                    .is_some_and(|variant| variant.eq_ignore_ascii_case(predicate))
        }
        "completes" => {
            // `human.ask.issued` only acknowledges that the ask was dispatched
            // (its issuing run completed); it is NOT the ask's terminal, so it
            // must not satisfy `completes` — otherwise `after <ask> completes`
            // would fire the moment the ask is issued, before any answer. The
            // ask's terminal is `human.answer.received`. (Flow correlation binds
            // the issuance via `after <ask> succeeds`, which is left intact.)
            if name == "human.ask.issued" {
                return false;
            }
            name == "human.answer.received"
                || name.ends_with(".succeeded")
                || name.ends_with(".failed")
                || name.ends_with(".completed")
                || matches!(
                    status,
                    Some("completed" | "failed" | "timed_out" | "cancelled")
                )
        }
        _ => false,
    }
}

pub fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

pub fn parse_record_fields(
    body: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
    errors: &mut Vec<String>,
) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    for assignment in collect_field_assignments(body) {
        match assignment {
            FieldAssignment::Value { name, value } => {
                object.insert(
                    name.clone(),
                    parse_record_field_value(&name, &value, context, from_binding, errors),
                );
            }
            FieldAssignment::Shorthand { name } => {
                object.insert(
                    name.clone(),
                    parse_record_shorthand_value(&name, context, from_binding, errors),
                );
            }
        }
    }
    object
}

pub fn parse_record_field_value(
    _field: &str,
    value: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
    errors: &mut Vec<String>,
) -> Value {
    if let Some(binding) = from_binding {
        if is_identifier(value)
            && !context
                .bindings
                .iter()
                .any(|(candidate, _)| candidate == value)
        {
            if let Some(copied) = context_field_value(context, binding, value) {
                return copied;
            }
            errors.push(format!("could not resolve `{binding}.{value}`"));
            return Value::Null;
        }
    }
    let is_plain_path = value.contains('.')
        && value
            .split('.')
            .all(|segment| !segment.is_empty() && is_identifier(segment));
    if is_plain_path {
        if let Some((binding, field)) = value.split_once('.') {
            if context
                .bindings
                .iter()
                .any(|(candidate, _)| candidate == binding)
                && context_field_value(context, binding, field).is_none()
            {
                errors.push(format!("could not resolve `{value}`"));
                return Value::Null;
            }
        }
    }
    parse_field_value(value, context)
}

pub fn parse_record_shorthand_value(
    field: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
    errors: &mut Vec<String>,
) -> Value {
    if let Some(binding) = from_binding {
        if let Some(value) = context_field_value(context, binding, field) {
            return value;
        }
        errors.push(format!("could not resolve `{binding}.{field}`"));
        return Value::Null;
    }
    let matches = context
        .bindings
        .iter()
        .filter_map(|(binding, _)| context_field_value(context, binding, field))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next().unwrap_or(Value::Null)
    } else {
        if matches.is_empty() {
            errors.push(format!("could not resolve shorthand field `{field}`"));
        } else {
            errors.push(format!("shorthand field `{field}` is ambiguous"));
        }
        Value::Null
    }
}

pub fn parse_field_value(value: &str, context: &RuleContext) -> Value {
    let value = value.trim();
    if let Some(unquoted) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Value::String(interpolate_prompt(unquoted, context));
    }
    if matches!(value.as_bytes().first(), Some(b'{' | b'[')) {
        if let Ok(parsed) = serde_json::from_str(value) {
            return parsed;
        }
    }
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if value == "null" {
        return Value::Null;
    }
    // Variant construction `Approved { score 0.9 }` builds the
    // internally-tagged record (spec/sum-types.md); the author never writes
    // the discriminant.
    if let Some((head, rest)) = value.split_once('{') {
        let head = head.trim();
        let rest = rest.trim_end();
        if is_identifier(head)
            && head
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
            && rest.ends_with('}')
        {
            let inner = &rest[..rest.len() - 1];
            let mut object = serde_json::Map::new();
            object.insert("variant".to_owned(), Value::String(head.to_owned()));
            let mut nested_errors = Vec::new();
            for (name, field_value) in parse_record_fields(inner, context, None, &mut nested_errors)
            {
                object.insert(name, field_value);
            }
            return Value::Object(object);
        }
    }
    if let Ok(number) = value.parse::<i64>() {
        return Value::Number(number.into());
    }
    if let Some((binding, field)) = value.split_once('.') {
        if let Some(value) = context_field_value(context, binding, field) {
            return value;
        }
    }
    if let Ok(expr) = whipplescript_parser::parse_expression(value) {
        if !matches!(expr, Expr::Literal(ExprLiteral::Ident(_))) {
            let empty_ir = empty_ir_program();
            return eval_expr_value(&expr, &EvalScope::rule(context, &[], &[], &empty_ir))
                .into_json();
        }
    }
    if matches!(value.as_bytes().first(), Some(b'{' | b'[')) {
        if let Some(parsed) = parse_inline_object_literal(value, context) {
            return parsed;
        }
    }
    context
        .bindings
        .iter()
        .find(|(binding, _)| binding == value)
        .map(|(_, fact)| json_from_str(&fact.value_json))
        .unwrap_or_else(|| Value::String(value.to_owned()))
}

pub fn parse_inline_object_literal(value: &str, context: &RuleContext) -> Option<Value> {
    let body = value.strip_prefix('{')?.strip_suffix('}')?.trim();
    let mut object = serde_json::Map::new();
    if body.is_empty() {
        return Some(Value::Object(object));
    }
    for field in body.split(',') {
        let field = field.trim();
        let (name, value) = field.split_once(char::is_whitespace)?;
        object.insert(name.to_owned(), parse_field_value(value.trim(), context));
    }
    Some(Value::Object(object))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldAssignment {
    Value { name: String, value: String },
    Shorthand { name: String },
}

pub fn collect_field_assignments(body: &str) -> Vec<FieldAssignment> {
    // Token-level splitting: structure comes from tokens, never line breaks,
    // so single-line blocks (`{ id "a" status "done" }`) and multi-line
    // blocks behave identically.
    whipplescript_parser::body::split_field_assignments(body)
        .into_iter()
        .map(|assignment| match assignment.value {
            Some(value) => FieldAssignment::Value {
                name: assignment.name,
                value,
            },
            None => FieldAssignment::Shorthand {
                name: assignment.name,
            },
        })
        .collect()
}

pub fn interpolate_prompt(prompt: &str, context: &RuleContext) -> String {
    let mut rendered = prompt.to_owned();
    for (binding, fact) in &context.bindings {
        let value = json_from_str(&fact.value_json);
        if let Some(object) = value.as_object() {
            for (field, field_value) in object {
                let needle = format!("{{{{ {binding}.{field} }}}}");
                rendered = rendered.replace(&needle, &render_interpolation_value(field_value));
            }
        }
    }
    rendered
}

pub fn render_interpolation_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub fn context_field_value(context: &RuleContext, binding: &str, field: &str) -> Option<Value> {
    context_path_value(context, binding, field)
}

pub fn context_path_value(context: &RuleContext, binding: &str, path: &str) -> Option<Value> {
    let fact = context
        .bindings
        .iter()
        .find(|(candidate, _)| candidate == binding)?
        .1
        .value_json
        .clone();
    let mut value = json_from_str(&fact);
    for field in path.split('.') {
        value = value.get(field)?.clone();
    }
    Some(value)
}

pub fn context_bindings_json(context: &RuleContext) -> Value {
    let mut object = serde_json::Map::new();
    for (binding, fact) in &context.bindings {
        object.insert(binding.clone(), json_from_str(&fact.value_json));
    }
    Value::Object(object)
}

pub fn record_fact_key(schema: &str, value_json: &str) -> String {
    let value = json_from_str(value_json);
    if let Some(number) = value.get("number").and_then(Value::as_i64) {
        return format!("{schema}:{number}");
    }
    if let Some(status) = value.get("status").and_then(Value::as_str) {
        return format!("{schema}:{status}:{}", stable_hash_hex(value_json));
    }
    format!("{schema}:{}", stable_hash_hex(value_json))
}

pub fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

pub fn stable_hash_hex(value: &str) -> String {
    format!("{:016x}", stable_hash(value))
}

pub fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn split_args(args: &str) -> Vec<String> {
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
                        values.push(value.to_owned());
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
        values.push(value.to_owned());
    }
    values
}
