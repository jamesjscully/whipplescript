//! Formal model generation for workflow IR.
//!
//! The first generated targets are deliberately small TLA+ and Maude
//! overapproximations of validated WorkflowIR. They are not replacements for
//! the hand-written model yet, but they create the product path for
//! `emit-model` and `check`.

use whipplescript_workflow::{expr::Expr, ir::State, schema::Schema, WorkflowIr};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTarget {
    Tla,
    Apalache,
    Maude,
    Veil,
}

#[derive(Debug, Error)]
pub enum ModelgenError {
    #[error("model generation for {0:?} is not implemented yet")]
    NotImplemented(ModelTarget),
    #[error("separate check config for {0:?} is not supported; the generated model contains its check command")]
    SeparateConfigUnsupported(ModelTarget),
    #[error(
        "expression invariant `{name}` cannot be represented by the current generated model abstraction: {reason}"
    )]
    UnsupportedExpressionInvariant { name: String, reason: String },
}

pub fn emit_model(ir: &WorkflowIr, target: ModelTarget) -> Result<String, ModelgenError> {
    match target {
        ModelTarget::Tla => {
            ensure_generated_model_invariants_supported(ir)?;
            Ok(emit_tla_model(ir))
        }
        ModelTarget::Maude => {
            ensure_generated_model_invariants_supported(ir)?;
            Ok(emit_maude_model(ir))
        }
        ModelTarget::Apalache | ModelTarget::Veil => Err(ModelgenError::NotImplemented(target)),
    }
}

pub fn emit_check_config(target: ModelTarget) -> Result<String, ModelgenError> {
    match target {
        ModelTarget::Tla => Ok(emit_tla_check_config()),
        ModelTarget::Maude => Err(ModelgenError::SeparateConfigUnsupported(target)),
        ModelTarget::Apalache | ModelTarget::Veil => Err(ModelgenError::NotImplemented(target)),
    }
}

pub fn emit_tla_check_config() -> String {
    [
        "SPECIFICATION Spec",
        "INVARIANT KnownState",
        "INVARIANT ActiveType",
        "INVARIANT DeclaredEffectType",
        "INVARIANT CoerceType",
        "INVARIANT MaxActivePositive",
        "INVARIANT MaxActiveRespected",
        "",
    ]
    .join("\n")
}

fn ensure_generated_model_invariants_supported(ir: &WorkflowIr) -> Result<(), ModelgenError> {
    for invariant in &ir.invariants {
        if let whipplescript_workflow::ir::Invariant::Expression { name, expr, .. } = invariant {
            return Err(ModelgenError::UnsupportedExpressionInvariant {
                name: name.clone(),
                reason: unsupported_expression_invariant_reason(expr),
            });
        }
    }
    Ok(())
}

fn unsupported_expression_invariant_reason(expr: &Expr) -> String {
    let mut roots = BTreeSet::new();
    collect_path_roots(expr, &mut roots);
    if roots.contains("data") {
        return "workflow data is not included in the generated model yet".to_string();
    }
    if roots.iter().any(|root| root != "data") {
        return format!(
            "path root(s) {} are outside the current generated model",
            roots.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    if contains_call(expr) {
        return "expression calls are not included in generated invariant models yet".to_string();
    }
    "only built-in invariants are currently modeled".to_string()
}

fn collect_path_roots(expr: &Expr, roots: &mut BTreeSet<String>) {
    match expr {
        Expr::Path { path } => {
            let root = path.split_once('.').map(|(root, _)| root).unwrap_or(path);
            roots.insert(root.to_string());
        }
        Expr::Eq { left, right }
        | Expr::Neq { left, right }
        | Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right }
        | Expr::In { left, right } => {
            collect_path_roots(left, roots);
            collect_path_roots(right, roots);
        }
        Expr::And { exprs } | Expr::Or { exprs } => {
            for expr in exprs {
                collect_path_roots(expr, roots);
            }
        }
        Expr::Not { expr } => collect_path_roots(expr, roots),
        Expr::Call { args, .. } => {
            for expr in args {
                collect_path_roots(expr, roots);
            }
        }
        Expr::Object { fields } => {
            for expr in fields.values() {
                collect_path_roots(expr, roots);
            }
        }
        Expr::List { items } => {
            for expr in items {
                collect_path_roots(expr, roots);
            }
        }
        Expr::Literal { .. } => {}
    }
}

fn contains_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { .. } => true,
        Expr::Eq { left, right }
        | Expr::Neq { left, right }
        | Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right }
        | Expr::In { left, right } => contains_call(left) || contains_call(right),
        Expr::And { exprs } | Expr::Or { exprs } => exprs.iter().any(contains_call),
        Expr::Not { expr } => contains_call(expr),
        Expr::Object { fields } => fields.values().any(contains_call),
        Expr::List { items } => items.iter().any(contains_call),
        Expr::Literal { .. } | Expr::Path { .. } => false,
    }
}

fn emit_tla_model(ir: &WorkflowIr) -> String {
    let states = collect_active_state_names(&ir.statechart.states);
    let transitions = collect_transitions(ir, &ir.statechart.states);
    let effect_transitions = collect_effect_transitions(ir, &ir.statechart.states);
    let coerce_transitions = collect_coerce_transitions(ir, &ir.statechart.states);
    let effect_observations = collect_effect_observations(ir, &ir.statechart.states);
    let declared_effects = collect_declared_effect_labels(ir, &ir.statechart.states);
    let initial_state = descend_initial_leaf(&ir.statechart.initial, &ir.statechart.states)
        .unwrap_or_else(|| ir.statechart.initial.clone());
    let module_name = tla_module_name(&ir.workflow.name);

    let mut output = String::new();
    output.push_str(&format!("---- MODULE {module_name} ----\n"));
    emit_tla_builtin_invariant_coverage(ir, &mut output);
    output.push_str("EXTENDS TLC, Naturals\n\n");
    output.push_str("VARIABLES state, active, coerce, effect\n\n");
    output.push_str("Vars == <<state, active, coerce, effect>>\n\n");
    output.push_str("States == {");
    output.push_str(
        &states
            .iter()
            .map(|state| tla_string(state))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("}\n\n");
    emit_tla_declared_effects(&declared_effects, &mut output);
    emit_tla_agent_limits(ir, &mut output);
    emit_tla_coerce_outputs(ir, &mut output);
    output.push_str(&format!(
        "Init ==\n  /\\ state = {}\n  /\\ active = [agent \\in AgentsWithMax |-> 0]\n  /\\ coerce = [function \\in CoerceFunctions |-> CoerceDefault(function)]\n  /\\ effect = \"none\"\n\n",
        tla_string(&initial_state)
    ));
    output.push_str("Next ==\n");
    if transitions.is_empty()
        && effect_transitions.is_empty()
        && coerce_transitions.is_empty()
        && effect_observations.is_empty()
    {
        output.push_str("  FALSE\n\n");
    } else {
        let mut first = true;
        for transition in &transitions {
            push_tla_disjunction_prefix(&mut output, &mut first);
            output.push_str(&format!(
                " /\\ state = {}\n     /\\ state' = {}\n     /\\ active' = active\n     /\\ coerce' = coerce\n     /\\ effect' = effect",
                tla_string(&transition.from),
                tla_string(&transition.to)
            ));
            if let Some(event) = &transition.event {
                output.push_str(&format!(" \\* {}", event));
            }
        }
        for effect in &effect_transitions {
            push_tla_disjunction_prefix(&mut output, &mut first);
            match &effect.kind {
                EffectTransitionKind::Start { agent } => {
                    let label = effect
                        .event
                        .as_ref()
                        .map(|event| format!("{event} start {agent}"))
                        .unwrap_or_else(|| format!("start {agent}"));
                    output.push_str(&format!(
                        " /\\ state = {}\n     /\\ {} \\in AgentsWithMax\n     /\\ active[{}] < MaxActive({})\n     /\\ state' = state\n     /\\ active' = [active EXCEPT ![{}] = @ + 1]\n     /\\ coerce' = coerce\n     /\\ effect' = \"start\" \\* {}",
                        tla_string(&effect.from),
                        tla_string(agent),
                        tla_string(agent),
                        tla_string(agent),
                        tla_string(agent),
                        label
                    ));
                }
                EffectTransitionKind::Finish { agent } => {
                    let label = effect
                        .event
                        .as_ref()
                        .map(|event| format!("{event} finish {agent}"))
                        .unwrap_or_else(|| format!("finish {agent}"));
                    output.push_str(&format!(
                        " /\\ state = {}\n     /\\ {} \\in AgentsWithMax\n     /\\ active[{}] > 0\n     /\\ state' = state\n     /\\ active' = [active EXCEPT ![{}] = @ - 1]\n     /\\ coerce' = coerce\n     /\\ effect' = \"finished\" \\* {}",
                        tla_string(&effect.from),
                        tla_string(agent),
                        tla_string(agent),
                        tla_string(agent),
                        label
                    ));
                }
            }
        }
        for transition in &coerce_transitions {
            push_tla_disjunction_prefix(&mut output, &mut first);
            let label = transition
                .event
                .as_ref()
                .map(|event| format!("{event} coerce {}", transition.function))
                .unwrap_or_else(|| format!("coerce {}", transition.function));
            output.push_str(&format!(
                " /\\ state = {}\n     /\\ \\E output \\in CoerceOutputs({}) :\n          /\\ state' = state\n          /\\ active' = active\n          /\\ coerce' = [coerce EXCEPT ![{}] = output]\n          /\\ effect' = {} \\* {}",
                tla_string(&transition.from),
                tla_string(&transition.function),
                tla_string(&transition.function),
                tla_string(&format!("coerce:{}", transition.function)),
                label
            ));
        }
        for observation in &effect_observations {
            push_tla_disjunction_prefix(&mut output, &mut first);
            let label = observation
                .event
                .as_ref()
                .map(|event| format!("{event} effect {}", observation.effect))
                .unwrap_or_else(|| format!("effect {}", observation.effect));
            output.push_str(&format!(
                " /\\ state = {}\n     /\\ state' = state\n     /\\ active' = active\n     /\\ coerce' = coerce\n     /\\ effect' = {} \\* {}",
                tla_string(&observation.from),
                tla_string(&observation.effect),
                label
            ));
        }
        output.push_str("\n\n");
    }
    output.push_str("Spec == Init /\\ [][Next]_Vars\n\n");
    output.push_str("KnownState == state \\in States\n\n");
    output.push_str("ActiveType == active \\in [AgentsWithMax -> Nat]\n\n");
    output.push_str("DeclaredEffectType == effect \\in DeclaredEffects\n\n");
    output.push_str(
        "CoerceType ==\n  /\\ DOMAIN coerce = CoerceFunctions\n  /\\ \\A function \\in CoerceFunctions : coerce[function] \\in CoerceOutputs(function)\n\n",
    );
    output.push_str("MaxActivePositive == \\A agent \\in AgentsWithMax : MaxActive(agent) > 0\n\n");
    output.push_str(
        "MaxActiveRespected == \\A agent \\in AgentsWithMax : active[agent] <= MaxActive(agent)\n\n",
    );
    output.push_str("====\n");
    output
}

fn emit_tla_builtin_invariant_coverage(ir: &WorkflowIr, output: &mut String) {
    let lines = builtin_invariant_coverage_lines(ir);
    if lines.is_empty() {
        output.push_str("\\* Built-in invariant coverage: none declared\n");
    } else {
        output.push_str("\\* Built-in invariant coverage:\n");
        for line in lines {
            output.push_str(&format!("\\* - {line}\n"));
        }
    }
}

fn push_tla_disjunction_prefix(output: &mut String, first: &mut bool) {
    if *first {
        output.push_str("  \\/");
        *first = false;
    } else {
        output.push_str("\n  \\/");
    }
}

fn emit_tla_declared_effects(effects: &BTreeSet<String>, output: &mut String) {
    output.push_str("DeclaredEffects == {");
    output.push_str(
        &effects
            .iter()
            .map(|effect| tla_string(effect))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("}\n\n");
}

fn emit_tla_agent_limits(ir: &WorkflowIr, output: &mut String) {
    let agents = agents_with_max_active(ir);
    output.push_str("AgentsWithMax == {");
    output.push_str(
        &agents
            .iter()
            .map(|(agent, _)| tla_string(agent))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("}\n\n");
    output.push_str("MaxActive(agent) ==\n");
    if agents.is_empty() {
        output.push_str("  0\n\n");
        return;
    }

    output.push_str("  CASE ");
    for (index, (agent, max_active)) in agents.iter().enumerate() {
        if index > 0 {
            output.push_str("\n    [] ");
        }
        output.push_str(&format!("agent = {} -> {max_active}", tla_string(agent)));
    }
    output.push_str("\n    [] OTHER -> 0\n\n");
}

fn emit_tla_coerce_outputs(ir: &WorkflowIr, output: &mut String) {
    output.push_str("CoerceFunctions == {");
    output.push_str(
        &ir.coerce_functions
            .keys()
            .map(|function| tla_string(function))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("}\n\n");
    output.push_str("CoerceOutputs(function) ==\n");
    if ir.coerce_functions.is_empty() {
        output.push_str("  {\"abstract\"}\n\n");
    } else {
        output.push_str("  CASE ");
        for (index, (function_name, function)) in ir.coerce_functions.iter().enumerate() {
            if index > 0 {
                output.push_str("\n    [] ");
            }
            output.push_str(&format!(
                "function = {} -> {}",
                tla_string(function_name),
                tla_string_set(&schema_output_space(&function.output, &ir.types))
            ));
        }
        output.push_str("\n    [] OTHER -> {\"abstract\"}\n\n");
    }
    output.push_str(
        "CoerceDefault(function) == CHOOSE output \\in CoerceOutputs(function) : TRUE\n\n",
    );
}

fn tla_string_set(values: &[String]) -> String {
    format!(
        "{{{}}}",
        values
            .iter()
            .map(|value| tla_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn schema_output_space(schema: &Schema, types: &BTreeMap<String, Schema>) -> Vec<String> {
    let values = schema_output_space_inner(schema, types, 0);
    if values.is_empty() {
        vec!["abstract".to_string()]
    } else {
        dedupe_preserving_order(values)
            .into_iter()
            .take(32)
            .collect()
    }
}

fn dedupe_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn schema_output_space_inner(
    schema: &Schema,
    types: &BTreeMap<String, Schema>,
    depth: usize,
) -> Vec<String> {
    if depth > 16 {
        return vec!["abstract".to_string()];
    }

    match schema {
        Schema::Boolean => vec!["true".to_string(), "false".to_string()],
        Schema::Null => vec!["null".to_string()],
        Schema::Literal { value } => vec![literal_label(value)],
        Schema::Enum { values } => values.clone(),
        Schema::Optional { inner } => {
            let mut values = vec!["null".to_string()];
            values.extend(schema_output_space_inner(inner, types, depth + 1));
            values
        }
        Schema::Union { variants } => variants
            .iter()
            .flat_map(|variant| schema_output_space_inner(variant, types, depth + 1))
            .collect(),
        Schema::Ref { name } => types
            .get(name)
            .map(|schema| schema_output_space_inner(schema, types, depth + 1))
            .unwrap_or_else(|| vec![format!("ref:{name}")]),
        Schema::Record { fields } => record_output_space(fields, types, depth + 1),
        Schema::String | Schema::Time | Schema::Duration | Schema::Agent => {
            vec!["abstract_string".to_string()]
        }
        Schema::Int => vec!["abstract_int".to_string()],
        Schema::Float => vec!["abstract_float".to_string()],
        Schema::List { .. } | Schema::Set { .. } => vec!["abstract_list".to_string()],
        Schema::Map { .. } | Schema::Json => vec!["abstract_json".to_string()],
    }
}

fn record_output_space(
    fields: &[whipplescript_workflow::schema::Field],
    types: &BTreeMap<String, Schema>,
    depth: usize,
) -> Vec<String> {
    let finite_fields = fields
        .iter()
        .filter_map(|field| {
            finite_discriminant_values(&field.schema, types, depth)
                .map(|values| (field.name.clone(), values))
        })
        .collect::<Vec<_>>();

    if finite_fields.is_empty() {
        return vec!["record".to_string()];
    }

    let mut labels = vec![String::new()];
    for (field_name, values) in finite_fields {
        let mut next = Vec::new();
        for prefix in &labels {
            for value in &values {
                let separator = if prefix.is_empty() { "" } else { ";" };
                next.push(format!("{prefix}{separator}{field_name}={value}"));
                if next.len() >= 32 {
                    return next;
                }
            }
        }
        labels = next;
    }
    labels
}

fn finite_discriminant_values(
    schema: &Schema,
    types: &BTreeMap<String, Schema>,
    depth: usize,
) -> Option<Vec<String>> {
    match schema {
        Schema::Boolean | Schema::Null | Schema::Literal { .. } | Schema::Enum { .. } => {
            Some(schema_output_space_inner(schema, types, depth + 1))
        }
        Schema::Optional { inner } => {
            let mut values = vec!["null".to_string()];
            values.extend(finite_discriminant_values(inner, types, depth + 1)?);
            Some(values)
        }
        Schema::Union { variants } => {
            let mut values = Vec::new();
            for variant in variants {
                values.extend(finite_discriminant_values(variant, types, depth + 1)?);
            }
            Some(values)
        }
        Schema::Ref { name } => types
            .get(name)
            .and_then(|schema| finite_discriminant_values(schema, types, depth + 1)),
        _ => None,
    }
}

fn literal_label(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Null => "null".to_string(),
        _ => "literal".to_string(),
    }
}

fn emit_maude_model(ir: &WorkflowIr) -> String {
    let states = collect_active_state_names(&ir.statechart.states);
    let transitions = collect_transitions(ir, &ir.statechart.states);
    let effect_transitions = collect_effect_transitions(ir, &ir.statechart.states);
    let agents = agents_with_max_active(ir);
    let initial_state = descend_initial_leaf(&ir.statechart.initial, &ir.statechart.states)
        .unwrap_or_else(|| ir.statechart.initial.clone());
    let module_name = maude_module_name(&ir.workflow.name);
    let state_symbols = maude_state_symbols(&states);
    let state_ops = state_symbols.values().cloned().collect::<Vec<_>>();

    let mut output = String::new();
    output.push_str(&format!("mod {module_name} is\n"));
    emit_maude_builtin_invariant_coverage(ir, &mut output);
    output.push_str("  protecting BOOL .\n");
    if !agents.is_empty() {
        output.push_str("  protecting NAT .\n");
    }
    output.push('\n');

    let state_sort = if agents.is_empty() {
        "State"
    } else {
        "ControlState"
    };
    output.push_str(&format!("  sort {state_sort} .\n"));
    output.push_str("  ops ");
    output.push_str(&state_ops.join(" "));
    output.push_str(&format!(" : -> {state_sort} [ctor] .\n"));
    if agents.is_empty() {
        output.push('\n');
        output.push_str("  op init : -> State .\n");
        output.push_str("  op knownState : State -> Bool .\n\n");
    } else {
        output.push_str("  sort Config .\n");
        output.push_str("  op cfg : ControlState");
        for _ in &agents {
            output.push_str(" Nat");
        }
        output.push_str(" -> Config [ctor] .\n\n");
        output.push_str("  op init : -> Config .\n");
        output.push_str("  op knownControlState : ControlState -> Bool .\n");
        output.push_str("  op inv : Config -> Bool .\n");
        output.push_str("  var S : ControlState .\n");
        output.push_str("  vars ");
        output.push_str(&maude_counter_vars(&agents).join(" "));
        output.push_str(" : Nat .\n\n");
    }

    emit_maude_agent_limits(&agents, &mut output);
    emit_maude_coerce_outputs(ir, &mut output);
    for (state, symbol) in &state_symbols {
        output.push_str(&format!("  *** {symbol} = {state}\n"));
    }
    output.push('\n');
    let initial_symbol = state_symbols
        .get(&initial_state)
        .cloned()
        .unwrap_or_else(|| "stUnknown".to_string());
    if agents.is_empty() {
        output.push_str(&format!("  eq init = {initial_symbol} .\n\n"));
    } else {
        output.push_str(&format!(
            "  eq init = {} .\n",
            maude_cfg(&initial_symbol, &vec!["0".to_string(); agents.len()])
        ));
        output.push_str("  eq inv(");
        output.push_str(&maude_cfg("S", &maude_counter_vars(&agents)));
        output.push_str(") = knownControlState(S)");
        for (index, (_, max_active)) in agents.iter().enumerate() {
            output.push_str(&format!(" and A{index} <= {max_active}"));
        }
        output.push_str(" .\n\n");
    }

    for symbol in state_symbols.values() {
        if agents.is_empty() {
            output.push_str(&format!("  eq knownState({symbol}) = true .\n"));
        } else {
            output.push_str(&format!("  eq knownControlState({symbol}) = true .\n"));
        }
    }
    output.push('\n');

    for (index, transition) in transitions.iter().enumerate() {
        let from = state_symbols
            .get(&transition.from)
            .cloned()
            .unwrap_or_else(|| "stUnknown".to_string());
        let to = state_symbols
            .get(&transition.to)
            .cloned()
            .unwrap_or_else(|| "stUnknown".to_string());
        if agents.is_empty() {
            output.push_str(&format!(
                "  rl [tr{index}] : {from} => {to} . *** {}\n",
                maude_transition_label(transition)
            ));
        } else {
            let counters = maude_counter_vars(&agents);
            output.push_str(&format!(
                "  rl [tr{index}] : {} => {} . *** {}\n",
                maude_cfg(&from, &counters),
                maude_cfg(&to, &counters),
                maude_transition_label(transition)
            ));
        }
    }

    if !agents.is_empty() {
        for (index, transition) in effect_transitions.iter().enumerate() {
            let from = state_symbols
                .get(&transition.from)
                .cloned()
                .unwrap_or_else(|| "stUnknown".to_string());
            let counters = maude_counter_vars(&agents);
            match &transition.kind {
                EffectTransitionKind::Start { agent } => {
                    let Some(agent_index) = agents.iter().position(|(name, _)| name == agent)
                    else {
                        continue;
                    };
                    let max_active = agents[agent_index].1;
                    let mut to_counters = counters.clone();
                    to_counters[agent_index] = format!("A{agent_index} + 1");
                    output.push_str(&format!(
                        "  crl [eff{index}] : {} => {} if A{agent_index} < {max_active} . *** {}\n",
                        maude_cfg(&from, &counters),
                        maude_cfg(&from, &to_counters),
                        maude_effect_transition_label(transition)
                    ));
                }
                EffectTransitionKind::Finish { agent } => {
                    let Some(agent_index) = agents.iter().position(|(name, _)| name == agent)
                    else {
                        continue;
                    };
                    let mut from_counters = counters.clone();
                    from_counters[agent_index] = format!("s A{agent_index}");
                    output.push_str(&format!(
                        "  rl [eff{index}] : {} => {} . *** {}\n",
                        maude_cfg(&from, &from_counters),
                        maude_cfg(&from, &counters),
                        maude_effect_transition_label(transition)
                    ));
                }
            }
        }
    }

    if transitions.is_empty() && effect_transitions.is_empty() {
        output.push_str("  *** No state-changing transitions in the generated abstraction.\n");
    }

    output.push_str("endm\n\n");
    if agents.is_empty() {
        output.push_str("search init =>* S:State such that knownState(S:State) == false .\n");
    } else {
        output.push_str("search init =>* C:Config such that inv(C:Config) == false .\n");
    }
    output
}

fn emit_maude_builtin_invariant_coverage(ir: &WorkflowIr, output: &mut String) {
    let lines = builtin_invariant_coverage_lines(ir);
    if lines.is_empty() {
        output.push_str("  *** Built-in invariant coverage: none declared\n");
    } else {
        output.push_str("  *** Built-in invariant coverage:\n");
        for line in lines {
            output.push_str(&format!("  *** - {line}\n"));
        }
    }
}

fn builtin_invariant_coverage_lines(ir: &WorkflowIr) -> Vec<String> {
    let mut names = BTreeSet::new();
    for invariant in &ir.invariants {
        if let whipplescript_workflow::ir::Invariant::Builtin { name, .. } = invariant {
            names.insert(name.as_str());
        }
    }
    names
        .into_iter()
        .map(|name| format!("{name}: {}", builtin_invariant_coverage(name)))
        .collect()
}

fn builtin_invariant_coverage(name: &str) -> &'static str {
    match name {
        "declaredAgentsOnly" => "static validation",
        "declaredEffectsOnly" => "static validation plus generated DeclaredEffectType",
        "agentCapabilitiesRespected" => "adapter/policy validation and runtime dispatch policy",
        "maxActiveRespected" => "runtime dispatch guard plus generated MaxActiveRespected",
        "terminalInvocationsObserved" => "static completion convention plus runtime projection",
        "failedEffectsAreDurable" => "runtime durable event/effect logs",
        "blockedWorkIsVisible" => "runtime status/log projection",
        "noSilentEventDrop" => "runtime event status and diagnostic logs",
        "noUnboundedInternalLoop" => "runtime entry/always transition limits",
        _ => "unknown",
    }
}

fn emit_maude_agent_limits(agents: &[(String, u32)], output: &mut String) {
    if agents.is_empty() {
        return;
    }

    output.push_str("  *** Agent maxActive limits\n");
    for (agent, max_active) in agents {
        output.push_str(&format!("  *** {agent} maxActive {max_active}\n"));
    }
    output.push('\n');
}

fn emit_maude_coerce_outputs(ir: &WorkflowIr, output: &mut String) {
    if ir.coerce_functions.is_empty() {
        return;
    }

    output.push_str("  *** Coerce output spaces\n");
    for (function_name, function) in &ir.coerce_functions {
        output.push_str(&format!(
            "  *** {function_name} outputs {}\n",
            schema_output_space(&function.output, &ir.types).join(", ")
        ));
    }
    output.push('\n');
}

fn agents_with_max_active(ir: &WorkflowIr) -> Vec<(String, u32)> {
    ir.agents
        .iter()
        .filter_map(|(name, agent)| {
            agent
                .max_active
                .map(|max_active| (name.clone(), max_active))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Transition {
    from: String,
    to: String,
    event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EffectTransition {
    from: String,
    event: Option<String>,
    kind: EffectTransitionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CoerceTransition {
    from: String,
    event: Option<String>,
    function: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EffectObservation {
    from: String,
    event: Option<String>,
    effect: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EffectTransitionKind {
    Start { agent: String },
    Finish { agent: String },
}

fn collect_active_state_names(states: &BTreeMap<String, State>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (name, state) in states {
        if state.initial.is_none() {
            names.insert(name.clone());
        }
        names.extend(collect_active_state_names(&state.states));
    }
    names
}

fn collect_transitions(ir: &WorkflowIr, states: &BTreeMap<String, State>) -> BTreeSet<Transition> {
    let mut transitions = BTreeSet::new();
    for (name, state) in states {
        let source_states = active_sources(name, state);
        for handler in &state.on {
            if let Some(target) = &handler.transition {
                insert_transitions(
                    ir,
                    &mut transitions,
                    &source_states,
                    target,
                    Some(handler.event.clone()),
                );
            }
            collect_case_transitions(
                ir,
                &mut transitions,
                &source_states,
                Some(handler.event.clone()),
                &handler.steps,
            );
        }

        for transition in &state.always {
            insert_transitions(
                ir,
                &mut transitions,
                &source_states,
                &transition.transition,
                Some("always".to_string()),
            );
            collect_case_transitions(
                ir,
                &mut transitions,
                &source_states,
                Some("always".to_string()),
                &transition.steps,
            );
        }

        collect_goto_transitions(
            ir,
            &mut transitions,
            &source_states,
            Some("entry".to_string()),
            &state.entry,
        );
        collect_case_transitions(
            ir,
            &mut transitions,
            &source_states,
            Some("entry".to_string()),
            &state.entry,
        );
        transitions.extend(collect_transitions(ir, &state.states));
    }
    transitions
}

fn collect_effect_transitions(
    ir: &WorkflowIr,
    states: &BTreeMap<String, State>,
) -> BTreeSet<EffectTransition> {
    let mut transitions = BTreeSet::new();
    let bounded_agents = agents_with_max_active(ir)
        .into_iter()
        .map(|(agent, _)| agent)
        .collect::<BTreeSet<_>>();

    for (name, state) in states {
        let source_states = active_sources(name, state);
        for handler in &state.on {
            collect_start_effect_transitions(
                &mut transitions,
                &bounded_agents,
                &source_states,
                Some(handler.event.clone()),
                &handler.steps,
            );
            if handler.event == "finished" {
                insert_finish_effect_transitions(
                    &mut transitions,
                    &bounded_agents,
                    &source_states,
                    Some(handler.event.clone()),
                );
            }
        }

        for transition in &state.always {
            collect_start_effect_transitions(
                &mut transitions,
                &bounded_agents,
                &source_states,
                Some("always".to_string()),
                &transition.steps,
            );
        }

        collect_start_effect_transitions(
            &mut transitions,
            &bounded_agents,
            &source_states,
            Some("entry".to_string()),
            &state.entry,
        );
        transitions.extend(collect_effect_transitions(ir, &state.states));
    }

    transitions
}

fn collect_declared_effect_labels(
    ir: &WorkflowIr,
    states: &BTreeMap<String, State>,
) -> BTreeSet<String> {
    let mut effects = BTreeSet::from(["none".to_string(), "finished".to_string()]);
    collect_declared_effects_from_states(ir, states, &mut effects);
    effects
}

fn collect_declared_effects_from_states(
    ir: &WorkflowIr,
    states: &BTreeMap<String, State>,
    effects: &mut BTreeSet<String>,
) {
    for state in states.values() {
        for handler in &state.on {
            collect_declared_effects_from_steps(ir, &handler.steps, effects);
        }
        for transition in &state.always {
            collect_declared_effects_from_steps(ir, &transition.steps, effects);
        }
        collect_declared_effects_from_steps(ir, &state.entry, effects);
        collect_declared_effects_from_states(ir, &state.states, effects);
    }
}

fn collect_declared_effects_from_steps(
    ir: &WorkflowIr,
    steps: &[whipplescript_workflow::ir::Step],
    effects: &mut BTreeSet<String>,
) {
    for step in steps {
        if let Some(effect) = effect_label_for_step(step) {
            effects.insert(effect);
        }

        let mut coerce_functions = BTreeSet::new();
        collect_coerce_calls_from_step(ir, step, &mut coerce_functions);
        for function in coerce_functions {
            effects.insert(format!("coerce:{function}"));
        }

        for arm in &step.case_arms {
            collect_declared_effects_from_steps(ir, &arm.steps, effects);
        }
    }
}

fn collect_effect_observations(
    ir: &WorkflowIr,
    states: &BTreeMap<String, State>,
) -> BTreeSet<EffectObservation> {
    let mut observations = BTreeSet::new();
    let bounded_agents = agents_with_max_active(ir)
        .into_iter()
        .map(|(agent, _)| agent)
        .collect::<BTreeSet<_>>();

    for (name, state) in states {
        let source_states = active_sources(name, state);
        for handler in &state.on {
            collect_effect_observations_from_steps(
                &mut observations,
                &bounded_agents,
                &source_states,
                Some(handler.event.clone()),
                &handler.steps,
            );
        }
        for transition in &state.always {
            collect_effect_observations_from_steps(
                &mut observations,
                &bounded_agents,
                &source_states,
                Some("always".to_string()),
                &transition.steps,
            );
        }
        collect_effect_observations_from_steps(
            &mut observations,
            &bounded_agents,
            &source_states,
            Some("entry".to_string()),
            &state.entry,
        );
        observations.extend(collect_effect_observations(ir, &state.states));
    }

    observations
}

fn collect_effect_observations_from_steps(
    observations: &mut BTreeSet<EffectObservation>,
    bounded_agents: &BTreeSet<String>,
    source_states: &[String],
    event: Option<String>,
    steps: &[whipplescript_workflow::ir::Step],
) {
    for step in steps {
        if let Some(effect) = observable_effect_label_for_step(step, bounded_agents) {
            for from in source_states {
                observations.insert(EffectObservation {
                    from: from.clone(),
                    event: event.clone(),
                    effect: effect.clone(),
                });
            }
        }

        for arm in &step.case_arms {
            collect_effect_observations_from_steps(
                observations,
                bounded_agents,
                source_states,
                event.clone(),
                &arm.steps,
            );
        }
    }
}

fn observable_effect_label_for_step(
    step: &whipplescript_workflow::ir::Step,
    bounded_agents: &BTreeSet<String>,
) -> Option<String> {
    if step.effect == "start" {
        let agent = step.args.get("agent").and_then(|value| value.as_str())?;
        if bounded_agents.contains(agent) {
            return None;
        }
    }
    effect_label_for_step(step)
}

fn effect_label_for_step(step: &whipplescript_workflow::ir::Step) -> Option<String> {
    match step.effect.as_str() {
        "send" | "start" | "askHuman" | "raise" => Some(step.effect.clone()),
        "capability_call" => {
            let capability = step.args.get("capability").and_then(|value| value.as_str());
            let operation = step
                .args
                .get("operation")
                .and_then(|value| value.as_str())
                .unwrap_or("call");
            Some(
                capability
                    .map(|capability| format!("{capability}.{operation}"))
                    .unwrap_or_else(|| format!("capability.{operation}")),
            )
        }
        _ => None,
    }
}

fn collect_coerce_transitions(
    ir: &WorkflowIr,
    states: &BTreeMap<String, State>,
) -> BTreeSet<CoerceTransition> {
    let mut transitions = BTreeSet::new();

    for (name, state) in states {
        let source_states = active_sources(name, state);
        for handler in &state.on {
            collect_coerce_transitions_from_steps(
                ir,
                &mut transitions,
                &source_states,
                Some(handler.event.clone()),
                &handler.steps,
            );
        }

        for transition in &state.always {
            collect_coerce_transitions_from_steps(
                ir,
                &mut transitions,
                &source_states,
                Some("always".to_string()),
                &transition.steps,
            );
        }

        collect_coerce_transitions_from_steps(
            ir,
            &mut transitions,
            &source_states,
            Some("entry".to_string()),
            &state.entry,
        );
        transitions.extend(collect_coerce_transitions(ir, &state.states));
    }

    transitions
}

fn collect_coerce_transitions_from_steps(
    ir: &WorkflowIr,
    transitions: &mut BTreeSet<CoerceTransition>,
    source_states: &[String],
    event: Option<String>,
    steps: &[whipplescript_workflow::ir::Step],
) {
    for step in steps {
        let mut functions = BTreeSet::new();
        collect_coerce_calls_from_step(ir, step, &mut functions);
        for function in functions {
            for from in source_states {
                transitions.insert(CoerceTransition {
                    from: from.clone(),
                    event: event.clone(),
                    function: function.clone(),
                });
            }
        }
    }
}

fn collect_coerce_calls_from_step(
    ir: &WorkflowIr,
    step: &whipplescript_workflow::ir::Step,
    functions: &mut BTreeSet<String>,
) {
    for value in step.args.values() {
        if let Ok(expr) = serde_json::from_value::<Expr>(value.clone()) {
            collect_coerce_calls_from_expr(ir, &expr, functions);
        }
    }
    for arm in &step.case_arms {
        for nested in &arm.steps {
            collect_coerce_calls_from_step(ir, nested, functions);
        }
    }
}

fn collect_coerce_calls_from_expr(ir: &WorkflowIr, expr: &Expr, functions: &mut BTreeSet<String>) {
    match expr {
        Expr::Call { name, args } => {
            if let Some(function_name) = name.strip_prefix("coerce ") {
                if ir.coerce_functions.contains_key(function_name) {
                    functions.insert(function_name.to_string());
                }
            } else if ir.coerce_functions.contains_key(name) {
                functions.insert(name.clone());
            }
            for arg in args {
                collect_coerce_calls_from_expr(ir, arg, functions);
            }
        }
        Expr::Eq { left, right }
        | Expr::Neq { left, right }
        | Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right }
        | Expr::In { left, right } => {
            collect_coerce_calls_from_expr(ir, left, functions);
            collect_coerce_calls_from_expr(ir, right, functions);
        }
        Expr::And { exprs } | Expr::Or { exprs } => {
            for expr in exprs {
                collect_coerce_calls_from_expr(ir, expr, functions);
            }
        }
        Expr::Not { expr } => collect_coerce_calls_from_expr(ir, expr, functions),
        Expr::Object { fields } => {
            for expr in fields.values() {
                collect_coerce_calls_from_expr(ir, expr, functions);
            }
        }
        Expr::List { items } => {
            for expr in items {
                collect_coerce_calls_from_expr(ir, expr, functions);
            }
        }
        Expr::Literal { .. } | Expr::Path { .. } => {}
    }
}

fn collect_start_effect_transitions(
    transitions: &mut BTreeSet<EffectTransition>,
    bounded_agents: &BTreeSet<String>,
    source_states: &[String],
    event: Option<String>,
    steps: &[whipplescript_workflow::ir::Step],
) {
    for step in steps {
        if step.effect == "start" {
            if let Some(agent) = step.args.get("agent").and_then(|value| value.as_str()) {
                if bounded_agents.contains(agent) {
                    for from in source_states {
                        transitions.insert(EffectTransition {
                            from: from.clone(),
                            event: event.clone(),
                            kind: EffectTransitionKind::Start {
                                agent: agent.to_string(),
                            },
                        });
                    }
                }
            }
        }
        if step.effect == "case" {
            for arm in &step.case_arms {
                collect_start_effect_transitions(
                    transitions,
                    bounded_agents,
                    source_states,
                    event.clone(),
                    &arm.steps,
                );
            }
        }
    }
}

fn insert_finish_effect_transitions(
    transitions: &mut BTreeSet<EffectTransition>,
    bounded_agents: &BTreeSet<String>,
    source_states: &[String],
    event: Option<String>,
) {
    for agent in bounded_agents {
        for from in source_states {
            transitions.insert(EffectTransition {
                from: from.clone(),
                event: event.clone(),
                kind: EffectTransitionKind::Finish {
                    agent: agent.clone(),
                },
            });
        }
    }
}

fn collect_case_transitions(
    ir: &WorkflowIr,
    transitions: &mut BTreeSet<Transition>,
    source_states: &[String],
    event: Option<String>,
    steps: &[whipplescript_workflow::ir::Step],
) {
    for step in steps {
        if step.effect == "case" {
            for arm in &step.case_arms {
                if let Some(target) = &arm.transition {
                    insert_transitions(ir, transitions, source_states, target, event.clone());
                }
                collect_goto_transitions(ir, transitions, source_states, event.clone(), &arm.steps);
                collect_case_transitions(ir, transitions, source_states, event.clone(), &arm.steps);
            }
        }
    }
}

fn collect_goto_transitions(
    ir: &WorkflowIr,
    transitions: &mut BTreeSet<Transition>,
    source_states: &[String],
    event: Option<String>,
    steps: &[whipplescript_workflow::ir::Step],
) {
    for step in steps {
        if step.effect == "goto" {
            if let Some(target) = step.args.get("target").and_then(|value| value.as_str()) {
                insert_transitions(ir, transitions, source_states, target, event.clone());
            }
        }
    }
}

fn insert_transitions(
    ir: &WorkflowIr,
    transitions: &mut BTreeSet<Transition>,
    source_states: &[String],
    target: &str,
    event: Option<String>,
) {
    let to = target_leaf_state(target, &ir.statechart.states).unwrap_or_else(|| target.to_string());
    for from in source_states {
        transitions.insert(Transition {
            from: from.clone(),
            to: to.clone(),
            event: event.clone(),
        });
    }
}

fn active_sources(name: &str, state: &State) -> Vec<String> {
    if state.initial.is_none() {
        return vec![name.to_string()];
    }

    let mut sources = Vec::new();
    collect_active_sources(&state.states, &mut sources);
    sources
}

fn collect_active_sources(states: &BTreeMap<String, State>, sources: &mut Vec<String>) {
    for (name, state) in states {
        if state.initial.is_none() {
            sources.push(name.clone());
        }
        collect_active_sources(&state.states, sources);
    }
}

fn descend_initial_leaf(initial: &str, states: &BTreeMap<String, State>) -> Option<String> {
    let mut current_name = initial.to_string();
    let mut current_state = states.get(initial)?;
    while let Some(child_initial) = &current_state.initial {
        current_name = child_initial.clone();
        current_state = current_state.states.get(child_initial)?;
    }
    Some(current_name)
}

fn target_leaf_state(target: &str, states: &BTreeMap<String, State>) -> Option<String> {
    let (_, state) = find_state(states, target)?;
    Some(descend_initial(target, state))
}

fn descend_initial(name: &str, state: &State) -> String {
    let Some(initial) = &state.initial else {
        return name.to_string();
    };

    state
        .states
        .get_key_value(initial)
        .map(|(name, child)| descend_initial(name, child))
        .unwrap_or_else(|| name.to_string())
}

fn find_state<'a>(
    states: &'a BTreeMap<String, State>,
    target: &str,
) -> Option<(&'a str, &'a State)> {
    for (name, state) in states {
        if name == target {
            return Some((name, state));
        }
        if let Some(found) = find_state(&state.states, target) {
            return Some(found);
        }
    }
    None
}

fn tla_module_name(name: &str) -> String {
    let mut module_name = String::from("WhippleScript_");
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            module_name.push(character);
        } else {
            module_name.push('_');
        }
    }
    module_name
}

fn tla_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn maude_module_name(name: &str) -> String {
    let mut module_name = String::from("WHIPPLESCRIPT-");
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            module_name.push(character.to_ascii_uppercase());
        } else {
            module_name.push('-');
        }
    }
    module_name
}

fn maude_state_symbols(states: &BTreeSet<String>) -> BTreeMap<String, String> {
    states
        .iter()
        .enumerate()
        .map(|(index, state)| (state.clone(), format!("st{index}")))
        .collect()
}

fn maude_transition_label(transition: &Transition) -> String {
    let mut label = String::new();
    if let Some(event) = &transition.event {
        label.push_str(&maude_token(event));
        label.push('-');
    }
    label.push_str(&maude_token(&transition.from));
    label.push_str("-to-");
    label.push_str(&maude_token(&transition.to));
    label
}

fn maude_effect_transition_label(transition: &EffectTransition) -> String {
    let mut label = String::new();
    if let Some(event) = &transition.event {
        label.push_str(&maude_token(event));
        label.push('-');
    }
    match &transition.kind {
        EffectTransitionKind::Start { agent } => {
            label.push_str("start-");
            label.push_str(&maude_token(agent));
        }
        EffectTransitionKind::Finish { agent } => {
            label.push_str("finish-");
            label.push_str(&maude_token(agent));
        }
    }
    label.push('-');
    label.push_str(&maude_token(&transition.from));
    label
}

fn maude_counter_vars(agents: &[(String, u32)]) -> Vec<String> {
    (0..agents.len()).map(|index| format!("A{index}")).collect()
}

fn maude_cfg(state: &str, counters: &[String]) -> String {
    let mut args = Vec::with_capacity(counters.len() + 1);
    args.push(state.to_string());
    args.extend(counters.iter().cloned());
    format!("cfg({})", args.join(", "))
}

fn maude_token(value: &str) -> String {
    let mut token = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            token.push(character.to_ascii_lowercase());
        } else if !token.ends_with('-') {
            token.push('-');
        }
    }

    let trimmed = token.trim_matches('-');
    if trimmed.is_empty() {
        "x".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use whipplescript_workflow::schema::{Field, Schema};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn emits_tla_model_for_minimal_workflow() {
        let source = include_str!("../../../examples/workflows/minimal.whip");
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let model = crate::emit_model(&ir, crate::ModelTarget::Tla).expect("model emits");

        assert!(model.contains("---- MODULE WhippleScript_Minimal ----"));
        assert!(model.contains(
            "\\* - declaredEffectsOnly: static validation plus generated DeclaredEffectType"
        ));
        assert!(model.contains(r#"States == {"complete", "waiting"}"#));
        assert!(model.contains(r#"state = "waiting""#));
        assert!(model.contains("active = [agent \\in AgentsWithMax |-> 0]"));
        assert!(model.contains(r#"state = "waiting""#));
        assert!(model.contains(r#"state' = "complete""#));
        assert!(model.contains("KnownState == state \\in States"));
        assert!(model.contains("ActiveType == active \\in [AgentsWithMax -> Nat]"));
        assert!(model.contains(r#"DeclaredEffects == {"finished", "none"}"#));
        assert!(model.contains("DeclaredEffectType == effect \\in DeclaredEffects"));
        assert!(model.contains("CoerceFunctions == {}"));
        assert!(model.contains("CoerceType =="));
        assert!(model.contains("AgentsWithMax == {}"));
        assert!(model
            .contains("MaxActivePositive == \\A agent \\in AgentsWithMax : MaxActive(agent) > 0"));
        assert!(model.contains(
            "MaxActiveRespected == \\A agent \\in AgentsWithMax : active[agent] <= MaxActive(agent)"
        ));
    }

    #[test]
    fn rejects_unimplemented_model_targets() {
        let source = include_str!("../../../examples/workflows/minimal.whip");
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");

        let error =
            crate::emit_model(&ir, crate::ModelTarget::Apalache).expect_err("not implemented");

        assert!(error.to_string().contains("not implemented yet"));
    }

    #[test]
    fn rejects_expression_invariants_until_data_is_modeled() {
        let source = r#"
machine ModelInvariant
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
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let error = crate::emit_model(&ir, crate::ModelTarget::Tla)
            .expect_err("expression invariant is not modelable yet");

        assert!(error
            .to_string()
            .contains("expression invariant `countWithinBound` cannot be represented"));
        assert!(error
            .to_string()
            .contains("workflow data is not included in the generated model yet"));
    }

    #[test]
    fn rejects_expression_invariants_with_calls_until_calls_are_modeled() {
        let source = r#"
machine CallInvariant
initial done

state done {
  final
}

invariant textWorks {
  assert text.contains("abc", "a")
}
"#;
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let error = crate::emit_model(&ir, crate::ModelTarget::Tla)
            .expect_err("expression call invariant is not modelable yet");

        assert!(error
            .to_string()
            .contains("expression invariant `textWorks` cannot be represented"));
        assert!(error
            .to_string()
            .contains("expression calls are not included in generated invariant models yet"));
    }

    #[test]
    fn emits_maude_model_for_minimal_workflow() {
        let source = include_str!("../../../examples/workflows/minimal.whip");
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let model = crate::emit_model(&ir, crate::ModelTarget::Maude).expect("model emits");

        assert!(model.contains("mod WHIPPLESCRIPT-MINIMAL is"));
        assert!(model.contains(
            "*** - declaredEffectsOnly: static validation plus generated DeclaredEffectType"
        ));
        assert!(model.contains("ops st0 st1 : -> State [ctor] ."));
        assert!(model.contains("*** st0 = complete"));
        assert!(model.contains("*** st1 = waiting"));
        assert!(model.contains("eq init = st1 ."));
        assert!(model.contains("rl [tr0] : st1 => st0 . *** start-waiting-to-complete"));
        assert!(model.contains("search init =>* S:State such that knownState(S:State) == false ."));
    }

    #[test]
    fn generated_tla_uses_leaf_sources_for_parent_handlers() {
        let source = include_str!("../../../examples/workflows/spec-implementation.whip");
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let model = crate::emit_model(&ir, crate::ModelTarget::Tla).expect("model emits");

        assert!(model.contains(
            "\\* - agentCapabilitiesRespected: adapter/policy validation and runtime dispatch policy"
        ));
        assert!(model.contains(
            "\\* - maxActiveRespected: runtime dispatch guard plus generated MaxActiveRespected"
        ));
        assert!(model.contains(r#"States == {"choosing", "done", "watching"}"#));
        assert!(!model.contains(r#""running""#));
        assert!(model.contains(r#"state = "watching""#));
        assert!(model.contains(r#"state' = "choosing""#));
        assert!(model.contains(r#"state = "choosing""#));
        assert!(model.contains(r#"state' = "watching""#));
        assert!(model.contains(r#"state' = "done""#));
        assert!(model.contains(r#"AgentsWithMax == {"quality", "worker"}"#));
        assert!(model.contains(r#"agent = "quality" -> 1"#));
        assert!(model.contains(r#"agent = "worker" -> 2"#));
        assert!(model.contains(r#"active["quality"] < MaxActive("quality")"#));
        assert!(model.contains(r#"active' = [active EXCEPT !["quality"] = @ + 1]"#));
        assert!(model.contains(r#"active["worker"] < MaxActive("worker")"#));
        assert!(model.contains(r#"active' = [active EXCEPT !["worker"] = @ + 1]"#));
        assert!(model.contains(r#"active' = [active EXCEPT !["quality"] = @ - 1]"#));
        assert!(model.contains(r#"CoerceFunctions == {"chooseNextStep", "classifyRun"}"#));
        assert!(model.contains(r#""coerce:chooseNextStep""#));
        assert!(model.contains(r#""coerce:classifyRun""#));
        assert!(model.contains(r#""plan.markDone""#));
        assert!(model.contains(r#"effect' = "start""#));
        assert!(model.contains(r#"effect' = "coerce:classifyRun""#));
        assert!(model.contains(r#"effect' = "plan.markReadyForQuality""#));
        assert!(model.contains(r#"effect' = "plan.markBlocked""#));
        assert!(model.contains(r#"effect' = "send""#));
        assert!(model.contains(r#"effect' = "askHuman""#));
        assert!(model.contains(
            r#"function = "classifyRun" -> {"kind=WorkerComplete", "kind=WorkerFailed", "kind=QualityPassed", "kind=QualityFailed", "kind=Irrelevant"}"#
        ));
        assert!(model.contains(
            r#"function = "chooseNextStep" -> {"action=StartWorker", "action=StartQuality", "action=AskHuman", "action=Wait", "action=Done"}"#
        ));
        assert!(model.contains(r#"output \in CoerceOutputs("classifyRun")"#));
        assert!(model.contains(r#"coerce' = [coerce EXCEPT !["classifyRun"] = output]"#));
        assert!(model.contains(r#"output \in CoerceOutputs("chooseNextStep")"#));
        assert!(model.contains(
            "CoerceType ==\n  /\\ DOMAIN coerce = CoerceFunctions\n  /\\ \\A function \\in CoerceFunctions : coerce[function] \\in CoerceOutputs(function)"
        ));
    }

    #[test]
    fn generated_maude_lists_agent_limits() {
        let source = include_str!("../../../examples/workflows/spec-implementation.whip");
        let ir = whipplescript_workflow::parse_source(source).expect("source parses");
        let report = whipplescript_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let model = crate::emit_model(&ir, crate::ModelTarget::Maude).expect("model emits");

        assert!(model.contains("*** - failedEffectsAreDurable: runtime durable event/effect logs"));
        assert!(model.contains("*** quality maxActive 1"));
        assert!(model.contains("*** worker maxActive 2"));
        assert!(model.contains("*** Coerce output spaces"));
        assert!(model.contains(
            "*** classifyRun outputs kind=WorkerComplete, kind=WorkerFailed, kind=QualityPassed, kind=QualityFailed, kind=Irrelevant"
        ));
        assert!(model.contains(
            "*** chooseNextStep outputs action=StartWorker, action=StartQuality, action=AskHuman, action=Wait, action=Done"
        ));
        assert!(model.contains("sort Config ."));
        assert!(model.contains("op cfg : ControlState Nat Nat -> Config [ctor] ."));
        assert!(model.contains("eq init = cfg(st2, 0, 0) ."));
        assert!(model
            .contains("eq inv(cfg(S, A0, A1)) = knownControlState(S) and A0 <= 1 and A1 <= 2 ."));
        assert!(model.contains("crl [eff0]"));
        assert!(model.contains("if A0 < 1"));
        assert!(model.contains("if A1 < 2"));
        assert!(model.contains("=> cfg(st0, A0, A1 + 1)"));
        assert!(model.contains("=> cfg(st0, A0, A1) . *** finished-finish-worker"));
        assert!(model.contains("search init =>* C:Config such that inv(C:Config) == false ."));
    }

    #[test]
    fn schema_output_space_dedupes_and_expands_finite_shapes() {
        let schema = Schema::Record {
            fields: vec![
                Field {
                    name: "mode".to_string(),
                    schema: Schema::Union {
                        variants: vec![
                            Schema::Literal {
                                value: json!("auto"),
                            },
                            Schema::Literal {
                                value: json!("manual"),
                            },
                            Schema::Literal {
                                value: json!("auto"),
                            },
                        ],
                    },
                },
                Field {
                    name: "approved".to_string(),
                    schema: Schema::Optional {
                        inner: Box::new(Schema::Boolean),
                    },
                },
            ],
        };

        let values = crate::schema_output_space(&schema, &BTreeMap::new());

        assert_eq!(
            values,
            vec![
                "mode=auto;approved=null",
                "mode=auto;approved=true",
                "mode=auto;approved=false",
                "mode=manual;approved=null",
                "mode=manual;approved=true",
                "mode=manual;approved=false",
            ]
        );
    }
}
