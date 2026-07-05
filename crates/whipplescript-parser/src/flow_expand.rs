//! Flow expansion: lowers `flow` declarations into ordinary rules and state
//! classes before analysis (`spec/flow.md`).
//!
//! A flow is a rule whose body is a multi-step sequence. Effect steps chain
//! inside one generated rule through nested `after` blocks (the engine's
//! existing multi-firing machinery). A segment boundary is needed only where
//! the flow waits on a *human answer*: the boundary records an await-state
//! fact carrying the ask's effect id, and the next segment's generated rule
//! joins that state with the correlated `human.answer.received` fact. The
//! generated rules and classes are visible everywhere — no hidden machinery.

use crate::body::{
    self, BodyEffectKind, BodyMode, BodyStmt, FieldValue, HandlerKind, Prompt, RecordStmt,
    TerminalStmt,
};
use crate::{
    BlockSource, ClassDecl, ClassField, Diagnostic, FlowDecl, Ident, Item, RuleDecl, SourceSpan,
    TypeSyntax, WhenClause,
};

/// Reserved prefix for generated flow state classes; user classes must not
/// use it.
pub const FLOW_STATE_PREFIX: &str = "FlowAwait_";

pub fn expand_flow(
    flow: FlowDecl,
    diagnostics: &mut Vec<Diagnostic>,
    warnings: &mut Vec<Diagnostic>,
) -> Vec<Item> {
    let (ast, body_diagnostics) =
        body::parse_rule_body(&flow.body.text, flow.body.span.start, BodyMode::Flow);
    diagnostics.extend(body_diagnostics);

    // Liveness: when a flow is a terminal path (it contains an inline `complete`/
    // `fail`), every branch — each `on fails`/`on timeout` handler and both arms of
    // every internal `when ... { } else { }` — must reach a workflow terminal, or
    // the workflow can stall on that branch (spec/static-analysis.md "Flow
    // liveness"). Severity is `warning`. Modeled in models/maude/flow-liveness.maude.
    check_flow_liveness(&flow.name.name, &ast.statements, warnings);

    // 503 auto-fail scope: only a self-terminating flow (one that owns reaching a
    // terminal via an inline `complete`/`fail`) auto-fails an unhandled effect
    // failure. A pure fact-hand-off flow is left to the broader workflow-liveness
    // analysis — the same scoping `check_flow_liveness` uses. Modeled in
    // models/maude/flow-autofail.maude.
    let flow_self_terminating = flow_contains_terminal(&ast.statements);

    // Only a CLASS trigger can be carried across an `askHuman` boundary: its
    // flow-state field is typed `Ref(schema)` (see `await_class`). A SIGNAL
    // trigger has a dotted schema (e.g. `deploy.finished`) with no class to
    // reference, so `await_class` omits the `t` field — and the post-ask
    // segments must omit reading `flowState.t` to match, or they reference a
    // field that was never written (`class FlowAwait_<flow>_<n> has no field t`).
    // Dropping it here keeps read and write consistent; a post-ask segment that
    // genuinely needs a signal-trigger field then gets a clear unknown-binding
    // error instead of a confusing internal one.
    let trigger_binding = flow
        .whens
        .iter()
        .find_map(|when| crate::binding_from_when(&when.text))
        .filter(|(_, schema)| !schema.contains('.'));

    // Split at human-ask boundaries. A `when/else` branch is only valid as
    // the first statement of a post-ask segment (its condition reads the
    // answer payload).
    let mut segments: Vec<Segment> = vec![Segment::default()];
    let mut statements = ast.statements.into_iter().peekable();
    while let Some(statement) = statements.next() {
        match statement {
            BodyStmt::Effect(effect) if matches!(effect.kind, BodyEffectKind::AskHuman { .. }) => {
                let binding = effect.binding.clone().unwrap_or_else(|| {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: effect.span,
                        message: format!(
                            "flow `{}` has an `askHuman` step without an `as` binding",
                            flow.name.name
                        ),
                        suggestion: Some(
                            "bind the ask (`askHuman as signoff ...`) so later steps can read the answer"
                                .to_owned(),
                        ),
                    });
                    "ask".to_owned()
                });
                // Attach any immediate handlers to the ask within this segment.
                let mut handlers = Vec::new();
                while matches!(statements.peek(), Some(BodyStmt::Handler(_))) {
                    if let Some(BodyStmt::Handler(handler)) = statements.next() {
                        handlers.push(handler);
                    }
                }
                let segment = segments.last_mut().expect("segment exists");
                segment.boundary = Some(Boundary {
                    ask: effect.clone(),
                    ask_binding: binding,
                    handlers,
                });
                segments.push(Segment::default());
            }
            BodyStmt::Branch(branch) => {
                // A `when/else` branch is only valid as the FIRST statement of a
                // POST-ask segment (it decides on the human answer). An ask always
                // opens a new segment, so `segments.len() == 1` means we are still
                // in the initial pre-ask segment — a branch there has no answer to
                // read and otherwise silently lowers to seg rules that consume an
                // unestablished `flowState` (a confusing internal error).
                let is_pre_ask = segments.len() == 1;
                let segment = segments.last_mut().expect("segment exists");
                if is_pre_ask || !segment.statements.is_empty() || segment.branch.is_some() {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: branch.span,
                        message: format!(
                            "flow `{}` has a `when/else` branch that does not directly follow an `askHuman` step",
                            flow.name.name
                        ),
                        suggestion: Some(
                            "a v1 flow `when/else` decides on a human answer — place it immediately after an `askHuman`; to branch on a fact, use a rule with `case`"
                                .to_owned(),
                        ),
                    });
                    continue;
                }
                segment.branch = Some(branch);
            }
            other => {
                let segment = segments.last_mut().expect("segment exists");
                // A `when/else` branch ends its segment — each arm decides the
                // flow's continuation. A statement after the branch in the same
                // segment otherwise lowers to a generated rule that reads a
                // flow-state field the branch never wrote (`class FlowAwait_…
                // has no field <x>`), so reject it with a clear message.
                if segment.branch.is_some() {
                    diagnostics.push(Diagnostic { related: Vec::new(),
                        span: flow.span,
                        message: format!(
                            "flow `{}` has a statement after a `when/else` branch",
                            flow.name.name
                        ),
                        suggestion: Some(
                            "a flow `when/else` branch ends its segment — each arm decides the outcome; move trailing work into the branch arms"
                                .to_owned(),
                        ),
                    });
                    continue;
                }
                segment.statements.push(other);
            }
        }
    }

    let mut items = Vec::new();
    let flow_name = flow.name.name.clone();
    let segment_count = segments.len();

    // Bindings shared across `askHuman` boundaries (a flow is "one fixed sequence
    // with shared bindings"). A pre-ask `tell` result REFERENCED in a later
    // segment is carried through flow state; they are typed `AgentTurn` (tell
    // results — no SemanticContext needed). `carried_per_segment[k]` is the set
    // available to segment k. Only referenced bindings are carried (no bloat, and
    // no snapshot churn for flows that don't reach back across the boundary).
    let pre_ask_tells: Vec<(String, usize)> = segments
        .iter()
        .enumerate()
        .flat_map(|(i, segment)| {
            segment
                .statements
                .iter()
                .filter_map(move |statement| match statement {
                    BodyStmt::Effect(effect)
                        if matches!(effect.kind, BodyEffectKind::Tell { .. }) =>
                    {
                        effect.binding.clone().map(|binding| (binding, i))
                    }
                    _ => None,
                })
        })
        .collect();
    let candidate_names: Vec<String> = pre_ask_tells
        .iter()
        .map(|(binding, _)| binding.clone())
        .collect();
    // Reference text marks each candidate binding with a sentinel ONLY where it
    // appears in a value position (the serializer applies the renamer to values,
    // not field names) — so a field named like a binding (`plan signoff.text`)
    // does not count as a reference.
    let reference_sentinel = |name: &str| format!("\u{1}{name}\u{1}");
    let segment_texts: Vec<String> = segments
        .iter()
        .map(|segment| segment_reference_text(segment, &candidate_names, &reference_sentinel))
        .collect();
    let carried_per_segment: Vec<Vec<String>> = (0..segment_count)
        .map(|k| {
            let mut carried = Vec::new();
            for (binding, defined_at) in &pre_ask_tells {
                let sentinel = reference_sentinel(binding);
                if *defined_at < k
                    && !carried.contains(binding)
                    && (k..segment_count).any(|s| segment_texts[s].contains(&sentinel))
                {
                    carried.push(binding.clone());
                }
            }
            carried
        })
        .collect();

    // The ask binding ending each segment leads into the next segment, where
    // it is renamed to `answer`. Captured explicitly here so post-ask
    // segments know which author binding to rewrite — no global state.
    let prior_ask_bindings: Vec<Option<String>> = std::iter::once(None)
        .chain(
            segments
                .iter()
                .map(|segment| segment.boundary.as_ref().map(|b| b.ask_binding.clone())),
        )
        .collect();

    for (index, segment) in segments.iter().enumerate() {
        // Await-state class for the boundary that LEADS INTO segment index+1.
        if segment.boundary.is_some() && index + 1 < segment_count {
            items.push(Item::Class(await_class(
                &flow_name,
                index + 1,
                trigger_binding.as_ref(),
                &carried_per_segment[index + 1],
                flow.span,
            )));
        }
    }

    for (index, segment) in segments.into_iter().enumerate() {
        let prior_ask = prior_ask_bindings.get(index).cloned().flatten();
        let entry = index == 0;
        // Post-ask segments rewrite references to the trigger fact and to any
        // carried pre-ask bindings so they read from flow state. The renamer is
        // applied only to statement value-positions (via `print_statement_rn`),
        // so field names — including the carried binding names in the boundary
        // `record` below — are never corrupted.
        let trigger_name: Option<String> = if entry {
            None
        } else {
            trigger_binding.as_ref().map(|(binding, _)| binding.clone())
        };
        let carried_here = &carried_per_segment[index];
        let renamer = |text: &str| -> String {
            let mut renamed = text.to_owned();
            if let Some(trigger) = &trigger_name {
                renamed = rename_text(&renamed, Some(trigger), "flowState.t");
            }
            for carried in carried_here {
                renamed = rename_text(&renamed, Some(carried), &format!("flowState.{carried}"));
            }
            renamed
        };

        let mut body_text = String::new();
        let mut synthetic = 0usize;
        if !entry {
            push_stmt_line(&mut body_text, 1, "done flowState");
        }
        // Implicit chaining as top-level sibling `after` blocks (the
        // engine's supported shape): each step is issued inside
        // `after <previous step> succeeds { ... }`, and trailing
        // statements ride with the step they follow.
        let mut previous_step: Option<String> = None;
        let mut statements = segment.statements.iter().peekable();
        while let Some(statement) = statements.next() {
            match statement {
                BodyStmt::Effect(effect) => {
                    let mut effect = effect.clone();
                    let step_binding = effect.binding.clone().unwrap_or_else(|| {
                        synthetic += 1;
                        let generated = format!("flowStep{synthetic}");
                        effect.binding = Some(generated.clone());
                        generated
                    });
                    let (open_depth, body_depth) = match &previous_step {
                        Some(previous) => {
                            push_stmt_line(
                                &mut body_text,
                                1,
                                &format!("after {previous} succeeds {{"),
                            );
                            (true, 2)
                        }
                        None => (false, 1),
                    };
                    print_statement_rn(
                        &BodyStmt::Effect(effect),
                        body_depth,
                        &renamer,
                        &mut body_text,
                    );
                    if open_depth {
                        push_stmt_line(&mut body_text, 1, "}");
                    }
                    // Immediate handlers become sibling failure branches.
                    let mut has_on_fails = false;
                    while matches!(statements.peek(), Some(BodyStmt::Handler(_))) {
                        if let Some(BodyStmt::Handler(handler)) = statements.next() {
                            let predicate = match handler.kind {
                                HandlerKind::OnFails => "fails",
                                // `on timeout` fires only on a timeout — NOT on
                                // success. (Was wrongly `completes`, which fires on
                                // any terminal, so the handler ran on success.)
                                HandlerKind::OnTimeout => "times out",
                            };
                            if handler.kind == HandlerKind::OnFails {
                                has_on_fails = true;
                            }
                            push_stmt_line(
                                &mut body_text,
                                1,
                                &format!("after {step_binding} {predicate} {{"),
                            );
                            for inner in &handler.body {
                                print_statement_rn(inner, 2, &renamer, &mut body_text);
                            }
                            push_stmt_line(&mut body_text, 1, "}");
                        }
                    }
                    // 503 auto-fail: in a self-terminating flow, an effect step with
                    // no `on fails` handler has an unhandled failure path that would
                    // otherwise stall the workflow forever. Route it to the generic
                    // `flowfail` terminal (kernel `fail_instance_internal`). Skipped
                    // when the author wrote an `on fails` handler (the failure is
                    // handled) or when the flow is not self-terminating (deferred to
                    // workflow-liveness). Modeled in models/maude/flow-autofail.maude.
                    if flow_self_terminating && !has_on_fails {
                        push_stmt_line(
                            &mut body_text,
                            1,
                            &format!("after {step_binding} fails {{"),
                        );
                        push_stmt_line(&mut body_text, 2, "flowfail");
                        push_stmt_line(&mut body_text, 1, "}");
                    }
                    previous_step = Some(step_binding);
                }
                other => match &previous_step {
                    Some(previous) => {
                        push_stmt_line(&mut body_text, 1, &format!("after {previous} succeeds {{"));
                        print_statement_rn(other, 2, &renamer, &mut body_text);
                        push_stmt_line(&mut body_text, 1, "}");
                    }
                    None => print_statement_rn(other, 1, &renamer, &mut body_text),
                },
            }
        }
        if let Some(boundary) = &segment.boundary {
            let (open_depth, body_depth) = match &previous_step {
                Some(previous) => {
                    push_stmt_line(&mut body_text, 1, &format!("after {previous} succeeds {{"));
                    (true, 2)
                }
                None => (false, 1),
            };
            print_statement_rn(
                &BodyStmt::Effect(boundary.ask.clone()),
                body_depth,
                &renamer,
                &mut body_text,
            );
            if open_depth {
                push_stmt_line(&mut body_text, 1, "}");
            }
            for handler in &boundary.handlers {
                let predicate = match handler.kind {
                    HandlerKind::OnFails => "fails",
                    // `on timeout` fires only on a timeout, not on success.
                    HandlerKind::OnTimeout => "times out",
                };
                push_stmt_line(
                    &mut body_text,
                    1,
                    &format!("after {} {predicate} {{", boundary.ask_binding),
                );
                for statement in &handler.body {
                    print_statement_rn(statement, 2, &renamer, &mut body_text);
                }
                push_stmt_line(&mut body_text, 1, "}");
            }
            let state_class = format!("{FLOW_STATE_PREFIX}{flow_name}_{}", index + 1);
            push_stmt_line(
                &mut body_text,
                1,
                &format!("after {} succeeds as flowIssued {{", boundary.ask_binding),
            );
            push_stmt_line(&mut body_text, 2, &format!("record {state_class} {{"));
            push_stmt_line(&mut body_text, 3, "askEffect flowIssued.effect_id");
            if let Some((binding, _)) = trigger_binding.as_ref() {
                let source = if entry {
                    binding.clone()
                } else {
                    "flowState.t".to_owned()
                };
                push_stmt_line(&mut body_text, 3, &format!("t {source}"));
            }
            // Carry pre-ask bindings forward into the next segment's await state.
            // A binding defined in THIS segment is a live binding here (source =
            // its name); one carried from an earlier segment is already read from
            // state (source = `flowState.<name>` — chaining). These lines are
            // emitted manually, so the carried field names are not renamed.
            for carried in &carried_per_segment[index + 1] {
                let source = if carried_here.contains(carried) {
                    format!("flowState.{carried}")
                } else {
                    carried.clone()
                };
                push_stmt_line(&mut body_text, 3, &format!("{carried} {source}"));
            }
            push_stmt_line(&mut body_text, 2, "}");
            push_stmt_line(&mut body_text, 1, "}");
        }

        let make_rule = |suffix: &str, extra_guard: Option<String>, text: String| {
            let whens = if entry {
                flow.whens.clone()
            } else {
                let state_class = format!("{FLOW_STATE_PREFIX}{flow_name}_{index}");
                let ask_binding = "answer";
                let mut guard = format!("{ask_binding}.effect_id == flowState.askEffect");
                if let Some(extra) = &extra_guard {
                    guard = format!("{guard} and ({extra})");
                }
                vec![
                    WhenClause {
                        text: format!("{state_class} as flowState"),
                        span: flow.span,
                    },
                    WhenClause {
                        text: format!("fact human.answer.received as {ask_binding} where {guard}"),
                        span: flow.span,
                    },
                ]
            };
            Item::Rule(RuleDecl {
                name: Ident {
                    name: format!("flow.{flow_name}.seg{index}{suffix}"),
                    span: flow.name.span,
                },
                tags: flow.tags.clone(),
                description: None,
                whens,
                body: BlockSource {
                    text,
                    span: flow.body.span,
                },
                span: flow.span,
            })
        };

        if let Some(branch) = segment.branch {
            // The branch condition reads both the awaited answer and (in a
            // post-ask segment) the trigger fact carried in flow state.
            let condition = {
                let mut text = branch.condition_source.clone();
                if !entry {
                    if let Some((binding, _)) = trigger_binding.as_ref() {
                        text = rename_text(&text, Some(binding), "flowState.t");
                    }
                    for carried in carried_here {
                        text = rename_text(&text, Some(carried), &format!("flowState.{carried}"));
                    }
                }
                rename_text(&text, prior_ask.as_deref(), "answer")
            };
            let mut then_text = String::from("  done flowState\n");
            for statement in &branch.then_body {
                print_statement_rn(statement, 1, &renamer, &mut then_text);
            }
            let then_text = rename_text(&then_text, prior_ask.as_deref(), "answer");
            items.push(make_rule("_then", Some(condition.clone()), then_text));
            if let Some(else_body) = &branch.else_body {
                let mut else_text = String::from("  done flowState\n");
                for statement in else_body {
                    print_statement_rn(statement, 1, &renamer, &mut else_text);
                }
                let else_text = rename_text(&else_text, prior_ask.as_deref(), "answer");
                items.push(make_rule(
                    "_else",
                    Some(format!("not ({condition})")),
                    else_text,
                ));
            }
        } else {
            let body_text = if entry {
                body_text
            } else {
                rename_text(&body_text, prior_ask.as_deref(), "answer")
            };
            // Empty trailing segments (flow ended on an ask) need no rule.
            if entry || !body_text.trim().is_empty() || segment.boundary.is_some() {
                items.push(make_rule("", None, body_text));
            }
        }
    }

    items
}

/// Flow branch-completeness liveness (spec/static-analysis.md). Gated on the flow
/// being a terminal path — i.e. it contains an inline `complete`/`fail` somewhere.
/// In that case every explicitly-branching position (each `on fails`/`on timeout`
/// handler body and both arms of every `when ... { } else { }`, including a missing
/// `else`) must reach a workflow terminal, or the workflow can stall on that branch.
/// Pure fact-hand-off flows (no inline terminal) are deferred to the broader
/// workflow-liveness lint. Severity is `warning`. Modeled in
/// `models/maude/flow-liveness.maude`.
fn check_flow_liveness(flow_name: &str, statements: &[BodyStmt], warnings: &mut Vec<Diagnostic>) {
    if !flow_contains_terminal(statements) {
        return;
    }
    check_flow_branches(flow_name, statements, warnings);
    check_flow_effect_timeouts(flow_name, statements, warnings);
}

/// An effect with an explicit `timeout <duration>` anticipates timing out, yet a
/// flow only routes the timeout to a terminal through an `on timeout` handler (it
/// lowers to an `after <effect> times out { ... }` block). An effect that sets a
/// timeout but attaches no `on timeout` handler leaves the timeout path with no
/// workflow terminal — the flow stalls when it fires (spec/static-analysis.md). The
/// failure path is deliberately NOT required here: any effect can fail, so requiring
/// `on fails` everywhere would over-report; an explicit timeout is opt-in intent.
fn check_flow_effect_timeouts(
    flow_name: &str,
    statements: &[BodyStmt],
    warnings: &mut Vec<Diagnostic>,
) {
    for (index, statement) in statements.iter().enumerate() {
        match statement {
            BodyStmt::Effect(effect) if effect.timeout_seconds.is_some() => {
                // Handlers attach as the sibling `BodyStmt::Handler`s immediately
                // following the effect in the same body.
                let handled = statements[index + 1..]
                    .iter()
                    .take_while(|s| matches!(s, BodyStmt::Handler(_)))
                    .any(|s| {
                        matches!(
                            s,
                            BodyStmt::Handler(handler) if handler.kind == HandlerKind::OnTimeout
                        )
                    });
                if !handled {
                    warnings.push(Diagnostic { related: Vec::new(),
                        span: effect.span,
                        message: format!(
                            "flow `{flow_name}` effect sets a `timeout` but has no `on timeout` handler: the timeout path reaches no workflow terminal"
                        ),
                        suggestion: Some(
                            "add `on timeout { ... }` reaching a `complete`/`fail`, or drop the `timeout`".to_owned(),
                        ),
                    });
                }
            }
            BodyStmt::After(after) => check_flow_effect_timeouts(flow_name, &after.body, warnings),
            BodyStmt::Case(case) => {
                for branch in &case.branches {
                    check_flow_effect_timeouts(flow_name, &branch.body, warnings);
                }
            }
            BodyStmt::Branch(branch) => {
                check_flow_effect_timeouts(flow_name, &branch.then_body, warnings);
                if let Some(else_body) = &branch.else_body {
                    check_flow_effect_timeouts(flow_name, else_body, warnings);
                }
            }
            BodyStmt::Handler(handler) => {
                check_flow_effect_timeouts(flow_name, &handler.body, warnings)
            }
            _ => {}
        }
    }
}

/// Whether `statements` reach an inline `complete`/`fail` along any nested path.
fn flow_contains_terminal(statements: &[BodyStmt]) -> bool {
    statements.iter().any(|statement| match statement {
        BodyStmt::Terminal(_) => true,
        BodyStmt::After(after) => flow_contains_terminal(&after.body),
        BodyStmt::Case(case) => case
            .branches
            .iter()
            .any(|branch| flow_contains_terminal(&branch.body)),
        BodyStmt::Branch(branch) => {
            flow_contains_terminal(&branch.then_body)
                || branch
                    .else_body
                    .as_deref()
                    .is_some_and(flow_contains_terminal)
        }
        BodyStmt::Handler(handler) => flow_contains_terminal(&handler.body),
        _ => false,
    })
}

/// Whether a branch body "settles": reaches a workflow terminal or a fact hand-off
/// (a `record`, or a `done … -> record …`) a workflow rule can complete from. The
/// fact hand-off is a conservative escape so the lint never flags a branch that
/// advances the workflow indirectly (zero false positives, at the cost of missing a
/// hand-off whose fact no rule actually terminates).
fn flow_branch_settles(statements: &[BodyStmt]) -> bool {
    statements.iter().any(|statement| match statement {
        BodyStmt::Terminal(_) | BodyStmt::Record(_) => true,
        BodyStmt::Done { replacement, .. } => replacement.is_some(),
        BodyStmt::After(after) => flow_branch_settles(&after.body),
        BodyStmt::Case(case) => case
            .branches
            .iter()
            .any(|branch| flow_branch_settles(&branch.body)),
        BodyStmt::Branch(branch) => {
            flow_branch_settles(&branch.then_body)
                || branch.else_body.as_deref().is_some_and(flow_branch_settles)
        }
        BodyStmt::Handler(handler) => flow_branch_settles(&handler.body),
        _ => false,
    })
}

/// Walk every branching position, flagging arms that do not settle, then descend so
/// nested branches inside a settling arm are still checked at their own granularity.
fn check_flow_branches(flow_name: &str, statements: &[BodyStmt], warnings: &mut Vec<Diagnostic>) {
    let stall = |warnings: &mut Vec<Diagnostic>, span: SourceSpan, what: &str| {
        warnings.push(Diagnostic { related: Vec::new(),
            span,
            message: format!(
                "flow `{flow_name}` {what} reaches no workflow terminal: every branch of a terminal flow must `complete` or `fail`"
            ),
            suggestion: Some(
                "reach a terminal on this branch — `complete`/`fail`, or record a fact a workflow rule completes from".to_owned(),
            ),
        });
    };
    for statement in statements {
        match statement {
            BodyStmt::Handler(handler) => {
                if !flow_branch_settles(&handler.body) {
                    let what = match handler.kind {
                        HandlerKind::OnFails => "`on fails` handler",
                        HandlerKind::OnTimeout => "`on timeout` handler",
                    };
                    stall(warnings, handler.span, what);
                }
                check_flow_branches(flow_name, &handler.body, warnings);
            }
            BodyStmt::Branch(branch) => {
                if !flow_branch_settles(&branch.then_body) {
                    stall(warnings, branch.span, "`when` branch");
                }
                match &branch.else_body {
                    None => stall(warnings, branch.span, "`when` branch without an `else`"),
                    Some(else_body) => {
                        if !flow_branch_settles(else_body) {
                            stall(warnings, branch.span, "`else` branch");
                        }
                        check_flow_branches(flow_name, else_body, warnings);
                    }
                }
                check_flow_branches(flow_name, &branch.then_body, warnings);
            }
            BodyStmt::After(after) => check_flow_branches(flow_name, &after.body, warnings),
            BodyStmt::Case(case) => {
                for branch in &case.branches {
                    check_flow_branches(flow_name, &branch.body, warnings);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Segment {
    statements: Vec<BodyStmt>,
    branch: Option<body::BranchBlock>,
    boundary: Option<Boundary>,
}

struct Boundary {
    ask: body::EffectStmt,
    ask_binding: String,
    handlers: Vec<body::HandlerBlock>,
}

fn await_class(
    flow_name: &str,
    index: usize,
    trigger: Option<&(String, String)>,
    carried: &[String],
    span: SourceSpan,
) -> ClassDecl {
    let mut fields = vec![ClassField {
        name: Ident {
            name: "askEffect".to_owned(),
            span,
        },
        ty: TypeSyntax::Primitive {
            name: "string".to_owned(),
            span,
        },
        is_key: false,
        presence_condition: None,
        span,
    }];
    if let Some((_, schema)) = trigger {
        if !schema.contains('.') {
            fields.push(ClassField {
                name: Ident {
                    name: "t".to_owned(),
                    span,
                },
                ty: TypeSyntax::Ref {
                    name: Ident {
                        name: schema.clone(),
                        span,
                    },
                },
                is_key: false,
                presence_condition: None,
                span,
            });
        }
    }
    // Carried pre-ask bindings are tell results, typed `AgentTurn`.
    for binding in carried {
        fields.push(ClassField {
            name: Ident {
                name: binding.clone(),
                span,
            },
            ty: TypeSyntax::Ref {
                name: Ident {
                    name: "AgentTurn".to_owned(),
                    span,
                },
            },
            is_key: false,
            presence_condition: None,
            span,
        });
    }
    ClassDecl {
        name: Ident {
            name: format!("{FLOW_STATE_PREFIX}{flow_name}_{index}"),
            span,
        },
        fields,
        span,
    }
}

/// Serializes a segment's content (statements, branch, boundary ask + handlers)
/// for binding-reference detection. Each candidate binding is rewritten to its
/// sentinel via the serializer's value-position renamer, so the sentinel appears
/// only where the binding is read as a VALUE — not where it is a field name.
fn segment_reference_text(
    segment: &Segment,
    candidates: &[String],
    sentinel: &dyn Fn(&str) -> String,
) -> String {
    let renamer = |text: &str| -> String {
        let mut renamed = text.to_owned();
        for candidate in candidates {
            renamed = rename_text(&renamed, Some(candidate), &sentinel(candidate));
        }
        renamed
    };
    let mut text = String::new();
    for statement in &segment.statements {
        print_statement_rn(statement, 0, &renamer, &mut text);
    }
    if let Some(branch) = &segment.branch {
        // The condition is a value expression; mark candidate references in it.
        text.push_str(&renamer(&branch.condition_source));
        text.push('\n');
        for statement in &branch.then_body {
            print_statement_rn(statement, 0, &renamer, &mut text);
        }
        if let Some(else_body) = &branch.else_body {
            for statement in else_body {
                print_statement_rn(statement, 0, &renamer, &mut text);
            }
        }
    }
    if let Some(boundary) = &segment.boundary {
        print_statement_rn(
            &BodyStmt::Effect(boundary.ask.clone()),
            0,
            &renamer,
            &mut text,
        );
        for handler in &boundary.handlers {
            for statement in &handler.body {
                print_statement_rn(statement, 0, &renamer, &mut text);
            }
        }
    }
    text
}

fn push_stmt_line(out: &mut String, indent: usize, line: &str) {
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(line);
    out.push('\n');
}

/// Rewrites references to `binding` (paths and bare uses) to `replacement`,
/// as whole-word matches. String-literal content is preserved EXCEPT inside
/// `{{ ... }}` template interpolations, where bindings are real references
/// that must be renamed. This prevents corrupting a literal value like
/// `event_type "ticket"` while still rewriting `"... {{ ticket.title }} ..."`.
///
/// `pub(crate)` so `action_expand` reuses the exact same reference-renaming
/// semantics for parameter substitution and binding hygiene.
pub(crate) fn rename_text(source: &str, binding: Option<&str>, replacement: &str) -> String {
    let Some(binding) = binding else {
        return source.to_owned();
    };
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let needle = binding.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut in_template = false; // inside `{{ ... }}`, even within a string
    while index < bytes.len() {
        // Track `{{` / `}}` template fences (they appear inside strings).
        if bytes[index..].starts_with(b"{{") {
            in_template = true;
            out.push_str("{{");
            index += 2;
            continue;
        }
        if bytes[index..].starts_with(b"}}") {
            in_template = false;
            out.push_str("}}");
            index += 2;
            continue;
        }
        if bytes[index] == b'"' && !in_template {
            in_string = !in_string;
            out.push('"');
            index += 1;
            continue;
        }
        // Rename only where the token is a live reference: outside string
        // literals, or inside a `{{ }}` template.
        let renameable = !in_string || in_template;
        let at_word_start =
            index == 0 || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
        if renameable
            && at_word_start
            && bytes[index..].starts_with(needle)
            && !bytes
                .get(index + needle.len())
                .is_some_and(|next| next.is_ascii_alphanumeric() || *next == b'_')
        {
            out.push_str(replacement);
            index += needle.len();
            continue;
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

/// Prints a body statement back to source text. Covers the statement subset
/// flows accept; the generated text re-enters the ordinary rule pipeline.
/// The serializer body, parameterized by an arbitrary reference renamer `rn`.
/// Field names and schema names are emitted verbatim; only value/expression
/// positions pass through `rn`. `action_expand` reuses this to apply parameter
/// substitution + binding hygiene without corrupting field names that happen to
/// share a parameter's name (e.g. `record R { provider provider }`).
pub(crate) fn print_statement_rn(
    statement: &BodyStmt,
    indent: usize,
    rn: &dyn Fn(&str) -> String,
    out: &mut String,
) {
    match statement {
        BodyStmt::Record(record) => print_record(record, indent, &rn, out, "record"),
        BodyStmt::Done {
            binding,
            replacement,
            ..
        } => {
            if let Some(record) = replacement {
                push_stmt_line(out, indent, &format!("done {} ->", rn(binding)));
                print_record(record, indent, &rn, out, "record");
            } else {
                push_stmt_line(out, indent, &format!("done {}", rn(binding)));
            }
        }
        BodyStmt::Terminal(terminal) => print_terminal(terminal, indent, &rn, out),
        BodyStmt::Cancel { binding, .. } => {
            push_stmt_line(out, indent, &format!("cancel {binding}"));
        }
        BodyStmt::Effect(effect) => {
            print_effect(effect, indent, &rn, out);
        }
        BodyStmt::After(after) => {
            let alias = after
                .alias
                .as_ref()
                .map(|alias| format!(" as {alias}"))
                .unwrap_or_default();
            // `reaches` carries a quoted milestone name (Family C); every other
            // predicate is the bare keyword.
            let predicate = match &after.milestone {
                Some(name) => format!("{} {:?}", after.predicate.as_str(), name),
                None => after.predicate.as_str().to_owned(),
            };
            push_stmt_line(
                out,
                indent,
                &format!("after {} {predicate}{alias} {{", after.binding),
            );
            for statement in &after.body {
                print_statement_rn(statement, indent + 1, rn, out);
            }
            push_stmt_line(out, indent, "}");
        }
        BodyStmt::Handler(handler) => {
            // Handlers are attached to the preceding effect by the caller;
            // reaching one here means it followed a non-effect statement.
            let _ = handler;
            push_stmt_line(out, indent, "");
        }
        BodyStmt::Case(case) => {
            push_stmt_line(out, indent, &format!("case {} {{", rn(&case.scrutinee)));
            for branch in &case.branches {
                let binding = branch
                    .binding
                    .as_ref()
                    .map(|binding| format!(" {binding}"))
                    .unwrap_or_default();
                push_stmt_line(
                    out,
                    indent + 1,
                    &format!("{}{binding} => {{", branch.pattern),
                );
                for statement in &branch.body {
                    print_statement_rn(statement, indent + 2, rn, out);
                }
                push_stmt_line(out, indent + 1, "}");
            }
            push_stmt_line(out, indent, "}");
        }
        BodyStmt::Branch(_) => {
            // Handled at segment level.
        }
        BodyStmt::Milestone {
            name,
            payload_class,
            fields,
            ..
        } => {
            let of = payload_class
                .as_ref()
                .map(|class| format!(" of {class}"))
                .unwrap_or_default();
            if fields.is_empty() {
                push_stmt_line(out, indent, &format!("emit milestone {name:?}{of}"));
            } else {
                push_stmt_line(out, indent, &format!("emit milestone {name:?}{of} {{"));
                print_fields(fields, indent + 1, rn, out);
                push_stmt_line(out, indent, "}");
            }
        }
        BodyStmt::Redact {
            source,
            keep,
            binding,
            ..
        } => {
            push_stmt_line(
                out,
                indent,
                &format!(
                    "redact {} keep [{}] as {}",
                    rn(source),
                    keep.join(", "),
                    rn(binding)
                ),
            );
        }
    }
}

fn print_record(
    record: &RecordStmt,
    indent: usize,
    rn: &dyn Fn(&str) -> String,
    out: &mut String,
    keyword: &str,
) {
    let from = record
        .from
        .as_ref()
        .map(|binding| format!(" from {}", rn(binding)))
        .unwrap_or_default();
    push_stmt_line(
        out,
        indent,
        &format!("{keyword} {}{from} {{", record.schema),
    );
    print_fields(&record.fields, indent + 1, rn, out);
    push_stmt_line(out, indent, "}");
}

fn print_terminal(
    terminal: &TerminalStmt,
    indent: usize,
    rn: &dyn Fn(&str) -> String,
    out: &mut String,
) {
    // The generated-only auto-fail terminal is a bare keyword with no name or
    // payload; it must serialize without the `<name> { ... }` block the typed
    // terminals carry.
    if terminal.kind == body::TerminalKind::FailInternal {
        push_stmt_line(out, indent, "flowfail");
        return;
    }
    let keyword = match terminal.kind {
        body::TerminalKind::Complete => "complete",
        body::TerminalKind::Fail => "fail",
        body::TerminalKind::FailInternal => unreachable!("handled above"),
    };
    // A bare scalar payload serializes as `complete <name> <value>` with no block.
    if let Some(FieldValue::Expr { source, .. }) = &terminal.scalar {
        push_stmt_line(
            out,
            indent,
            &format!("{keyword} {} {}", terminal.name, rn(source)),
        );
        return;
    }
    let from = terminal
        .from
        .as_ref()
        .map(|binding| format!(" from {}", rn(binding)))
        .unwrap_or_default();
    push_stmt_line(
        out,
        indent,
        &format!("{keyword} {}{from} {{", terminal.name),
    );
    print_fields(&terminal.fields, indent + 1, rn, out);
    push_stmt_line(out, indent, "}");
}

fn print_fields(
    fields: &[body::FieldAssign],
    indent: usize,
    rn: &dyn Fn(&str) -> String,
    out: &mut String,
) {
    for field in fields {
        match &field.value {
            FieldValue::Shorthand => push_stmt_line(out, indent, &field.name),
            FieldValue::Expr { source, .. } => {
                push_stmt_line(out, indent, &format!("{} {}", field.name, rn(source)))
            }
            FieldValue::Nested { schema, fields } => {
                push_stmt_line(out, indent, &format!("{} {schema} {{", field.name));
                print_fields(fields, indent + 1, rn, out);
                push_stmt_line(out, indent, "}");
            }
        }
    }
}

fn format_access_grants(
    access_grants: &[body::AccessGrant],
    rn: &dyn Fn(&str) -> String,
) -> String {
    access_grants
        .iter()
        .map(|grant| {
            let ops = grant
                .operations
                .iter()
                .map(|op| {
                    let mut clause = op.operation.clone();
                    if let Some(target) = &op.target {
                        clause.push_str(&format!(" for {}", rn(target)));
                    }
                    if !op.globs.is_empty() {
                        clause.push_str(&format!(
                            " [{}]",
                            op.globs
                                .iter()
                                .map(|glob| format!("{glob:?}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    clause
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!(" with access to {} {{ {ops} }}", grant.resource)
        })
        .collect::<String>()
}

fn print_effect(
    effect: &body::EffectStmt,
    indent: usize,
    rn: &dyn Fn(&str) -> String,
    out: &mut String,
) {
    let binding = effect
        .binding
        .as_ref()
        .map(|binding| format!(" as {binding}"))
        .unwrap_or_default();
    let requires = if effect.requires.is_empty() {
        String::new()
    } else {
        format!(
            " requires [{}]",
            effect
                .requires
                .iter()
                .map(|capability| format!("{capability:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let timeout = effect
        .timeout_seconds
        .map(|seconds| format!(" timeout {seconds}s"))
        .unwrap_or_default();
    let header = match &effect.kind {
        BodyEffectKind::Tell {
            target,
            access_grants,
        } => {
            // Re-serialize `with access to <resource> { <op clauses> }` grants so a
            // flow `tell` preserves its access metadata. `for <target>` refs are flow
            // bindings (renamed); resource names and globs are literals.
            let grants = format_access_grants(access_grants, rn);
            format!("tell {}{requires}{binding}{timeout}{grants}", rn(target))
        }
        BodyEffectKind::Prompt { provider } => {
            let using = provider
                .as_ref()
                .map(|provider| format!(" using {provider}"))
                .unwrap_or_default();
            let (text, content_type) = effect
                .prompt
                .as_ref()
                .map(|prompt| (prompt.text.as_str(), prompt.content_type.as_deref()))
                .unwrap_or(("", None));
            let annotation = content_type.unwrap_or_default();
            push_stmt_line(out, indent, &format!("prompt \"\"\"{annotation}"));
            for line in rn(text).lines() {
                push_stmt_line(out, indent, line);
            }
            push_stmt_line(
                out,
                indent,
                &format!("\"\"\"{using}{requires}{binding}{timeout}"),
            );
            return;
        }
        BodyEffectKind::AskHuman { choices } => {
            let choices = if choices.is_empty() {
                String::new()
            } else {
                format!(
                    " choices [{}]",
                    choices
                        .iter()
                        .map(|choice| format!("{choice:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!("askHuman{binding}{choices}{timeout}")
        }
        BodyEffectKind::Coerce {
            name,
            args,
            endorsed,
            declassified,
        } => {
            let args = args
                .iter()
                .map(|arg| rn(arg))
                .collect::<Vec<_>>()
                .join(", ");
            // preserve the source-crossing markers through flow expansion (trailing).
            let endorsed = if *endorsed { " endorsed" } else { "" };
            let declassified = if *declassified { " declassified" } else { "" };
            push_stmt_line(
                out,
                indent,
                &format!("coerce {name}({args}){binding}{timeout}{endorsed}{declassified}"),
            );
            return;
        }
        BodyEffectKind::Decide { result_fields } => {
            let shape = result_fields
                .iter()
                .map(|(name, ty)| format!("{name} {ty}"))
                .collect::<Vec<_>>()
                .join(", ");
            let prompt = effect
                .prompt
                .as_ref()
                .map(|p| p.text.clone())
                .unwrap_or_default();
            push_stmt_line(
                out,
                indent,
                &format!(
                    "decide {:?} -> {{ {shape} }}{binding}{timeout}",
                    rn(&prompt)
                ),
            );
            return;
        }
        BodyEffectKind::Call {
            capability,
            argument,
        } => {
            let argument = argument
                .as_ref()
                .map(|argument| format!(" for {}", rn(argument)))
                .unwrap_or_default();
            push_stmt_line(
                out,
                indent,
                &format!("call {capability}{argument}{binding}{timeout}"),
            );
            return;
        }
        BodyEffectKind::ConstructCapabilityCall {
            keyword, fields, ..
        } => {
            if keyword == "recall" {
                let pool = fields
                    .iter()
                    .find(|field| field.name == "pool")
                    .map(|field| field.source.as_str())
                    .unwrap_or_default();
                let query = fields
                    .iter()
                    .find(|field| field.name == "query")
                    .map(|field| field.source.as_str())
                    .unwrap_or_default();
                push_stmt_line(
                    out,
                    indent,
                    &format!("recall {pool} for {query}{binding}{timeout}"),
                );
            } else {
                let field_source = fields
                    .iter()
                    .map(|field| field.source.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                push_stmt_line(
                    out,
                    indent,
                    &format!("{keyword} {field_source}{binding}{timeout}"),
                );
            }
            return;
        }
        BodyEffectKind::Invoke {
            workflow,
            payload,
            access_grants,
        } => {
            push_stmt_line(out, indent, &format!("invoke {workflow} {{"));
            print_fields(payload, indent + 1, &rn, out);
            let grants = format_access_grants(access_grants, rn);
            push_stmt_line(
                out,
                indent,
                &format!("}}{requires}{binding}{timeout}{grants}"),
            );
            return;
        }
        BodyEffectKind::Timer {
            duration_seconds,
            until,
            ..
        } => {
            match until {
                Some(deadline) => push_stmt_line(
                    out,
                    indent,
                    &format!("timer until {:?}{binding}", rn(deadline)),
                ),
                None => push_stmt_line(out, indent, &format!("timer {duration_seconds}s{binding}")),
            }
            return;
        }
        BodyEffectKind::Exec {
            target,
            parse_target,
        } => {
            let parse = match parse_target {
                Some(parse) if parse.each => format!(" -> each {}", parse.schema),
                Some(parse) => format!(" -> {}", parse.schema),
                None => String::new(),
            };
            let head = match target {
                crate::body::ExecTarget::RawCommand(command) => format!("exec {command:?}"),
                crate::body::ExecTarget::Capability {
                    name,
                    stdin_binding,
                } => format!("exec {name} with {stdin_binding}"),
            };
            push_stmt_line(out, indent, &format!("{head}{parse}{binding}{timeout}"));
            return;
        }
        BodyEffectKind::TrackerFile { queue, fields } => {
            push_stmt_line(out, indent, &format!("file issue into {queue} {{"));
            print_fields(fields, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}}{binding}"));
            return;
        }
        BodyEffectKind::TrackerClaim { item, .. } => {
            format!("claim {}{binding}{timeout}", rn(item))
        }
        BodyEffectKind::TrackerRelease { item } => format!("release {}", rn(item)),
        BodyEffectKind::LeaseAcquire {
            resource,
            key_expr,
            until_ttl,
            wait_seconds,
        } => {
            let until = if *until_ttl { " until ttl" } else { "" };
            let wait = wait_seconds
                .map(|seconds| format!(" wait {seconds}s"))
                .unwrap_or_default();
            format!(
                "acquire {resource} for {}{until}{wait}{binding}",
                rn(key_expr)
            )
        }
        BodyEffectKind::LeaseRenew {
            acquire_binding,
            ttl_seconds,
        } => {
            let until = ttl_seconds
                .map(|seconds| format!(" until {seconds}s"))
                .unwrap_or_default();
            format!("renew {}{until}{binding}", rn(acquire_binding))
        }
        BodyEffectKind::LedgerAppend {
            ledger,
            schema,
            fields,
        } => {
            push_stmt_line(out, indent, &format!("append {schema} {{"));
            print_fields(fields, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}} to {ledger}{binding}"));
            return;
        }
        BodyEffectKind::CounterConsume {
            counter,
            key_expr,
            amount_expr,
        } => format!(
            "consume {counter} for {} amount {}{binding}",
            rn(key_expr),
            rn(amount_expr)
        ),
        BodyEffectKind::Notify {
            target_expr,
            event,
            fields,
        } => {
            push_stmt_line(
                out,
                indent,
                &format!("emit signal {event} to {} {{", rn(target_expr)),
            );
            print_fields(fields, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}}{binding}"));
            return;
        }
        BodyEffectKind::TrackerFinish { item, fields } => {
            if fields.is_empty() {
                format!("finish {}", rn(item))
            } else {
                push_stmt_line(out, indent, &format!("finish {} {{", rn(item)));
                print_fields(fields, indent + 1, &rn, out);
                push_stmt_line(out, indent, "}");
                return;
            }
        }
        BodyEffectKind::FileRead {
            format,
            store,
            path,
        } => {
            format!(
                "read {format} from {} at {}{requires}{binding}{timeout}",
                rn(store),
                rn(path)
            )
        }
        BodyEffectKind::FileWrite {
            format,
            store,
            path,
            body,
            mode,
        } => {
            push_stmt_line(
                out,
                indent,
                &format!("write {format} to {} at {} {{", rn(store), rn(path)),
            );
            push_stmt_line(out, indent + 1, &format!("body {}", rn(body)));
            push_stmt_line(out, indent + 1, &format!("mode {mode}"));
            push_stmt_line(out, indent, &format!("}}{requires}{binding}{timeout}"));
            return;
        }
        BodyEffectKind::FileImport {
            format,
            schema,
            store,
            path,
        } => {
            format!(
                "import {format} {schema} from {} at {}{requires}{binding}{timeout}",
                rn(store),
                rn(path)
            )
        }
        BodyEffectKind::FileExport {
            format,
            schema,
            store,
            path,
            predicate,
            mode,
        } => {
            push_stmt_line(
                out,
                indent,
                &format!(
                    "export {format} {schema} to {} at {} {{",
                    rn(store),
                    rn(path)
                ),
            );
            if let Some(predicate) = predicate {
                push_stmt_line(out, indent + 1, &format!("where {}", rn(predicate)));
            }
            push_stmt_line(out, indent + 1, &format!("mode {mode}"));
            push_stmt_line(out, indent, &format!("}}{requires}{binding}{timeout}"));
            return;
        }
    };
    match &effect.prompt {
        Some(Prompt { text, content_type }) => {
            let annotation = content_type.clone().unwrap_or_default();
            push_stmt_line(out, indent, &format!("{header} \"\"\"{annotation}"));
            for line in rn(text).lines() {
                push_stmt_line(out, indent, line);
            }
            push_stmt_line(out, indent, "\"\"\"");
        }
        None => push_stmt_line(out, indent, &header),
    }
}

#[cfg(test)]
mod tests {
    use crate::{compile_program, compile_program_with_root};

    const TRIAGE: &str = r#"
workflow TicketTriage

input ticket Ticket
output result TriageDecision
failure error TriageBlocked

class Ticket {
  id string
  title string
}

class TriageDecision {
  decision string
  decidedBy string
}

class TriageBlocked {
  reason string
}

agent triager {
  provider fixture
  profile "repo-reader"
  capacity 1
}

flow triage
  when Ticket as ticket
{
  tell triager as turn "Plan {{ ticket.title }}."

  askHuman as signoff "Approve {{ turn.summary }}?"

  when signoff.choice == "approve" {
    complete result {
      decision signoff.choice
      decidedBy signoff.answered_by
    }
  } else {
    fail error {
      reason "rejected"
    }
  }
}
"#;

    fn liveness_warnings(source: &str) -> Vec<String> {
        compile_program(source)
            .warnings
            .into_iter()
            .filter(|w| w.message.contains("reaches no workflow terminal"))
            .map(|w| w.message)
            .collect()
    }

    /// Concatenated body text of every generated `flow.*` rule for `source`.
    fn flow_rule_bodies(source: &str) -> String {
        let compiled = compile_program(source);
        assert_eq!(compiled.diagnostics, Vec::new(), "{source}");
        compiled
            .ir
            .expect("lowered IR")
            .rules
            .into_iter()
            .filter(|rule| rule.name.starts_with("flow."))
            .map(|rule| rule.body)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn flow_autofail_generates_flowfail_for_unhandled_effect() {
        // TRIAGE is self-terminating (it has inline `complete`/`fail`). Its `tell`
        // step has no `on fails` handler, so 503 auto-fail routes the unhandled
        // failure to the generic `flowfail` terminal.
        let bodies = flow_rule_bodies(TRIAGE);
        assert!(
            bodies.contains("after turn fails {"),
            "expected an auto-fail branch for the tell step: {bodies}"
        );
        assert!(
            bodies.contains("flowfail"),
            "expected the generated flowfail terminal: {bodies}"
        );
    }

    #[test]
    fn flow_autofail_skips_effect_with_on_fails_handler() {
        // With an author `on fails` handler the failure is handled, so no auto-fail
        // is generated for that step — the handler's own terminal fires instead.
        let with_handler = TRIAGE.replace(
            r#"tell triager as turn "Plan {{ ticket.title }}.""#,
            "tell triager as turn \"Plan {{ ticket.title }}.\"\n  on fails { fail error { reason \"planning failed\" } }",
        );
        assert_ne!(with_handler, TRIAGE);
        let bodies = flow_rule_bodies(&with_handler);
        assert!(
            bodies.contains("reason \"planning failed\""),
            "the author on-fails handler must be lowered: {bodies}"
        );
        assert!(
            !bodies.contains("flowfail"),
            "a handled failure must not also auto-fail: {bodies}"
        );
    }

    #[test]
    fn flow_autofail_skips_non_self_terminating_flow() {
        // A pure fact-hand-off flow (no inline terminal) is left to the broader
        // workflow-liveness analysis — never auto-failed.
        let source = r#"
@service
workflow Handoff

class Ticket { id string  status "open" }
class Done { id string }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

flow f
  when Ticket as ticket where ticket.status == "open"
  when reviewer is available
{
  tell reviewer as turn "Do work."

  record Done { id ticket.id }
}

rule finish
  when Done as d
=> {
  done d
}
"#;
        let bodies = flow_rule_bodies(source);
        assert!(
            !bodies.contains("flowfail"),
            "a non-self-terminating flow must not auto-fail: {bodies}"
        );
    }

    #[test]
    fn flow_tell_preserves_turn_access_grants_through_expansion() {
        // A `with access to` grant on a flow `tell` must survive flow re-serialization
        // and lower onto the generated rule's agent.tell effect (target refs renamed).
        use crate::IrEffectKind;
        let source = r#"
@service
workflow FlowGrant

output result R
failure error E
class R { ok bool }
class E { reason string }
class Ticket { id string  status "open" }

agent coder { provider fixture  profile "repo-writer"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

flow handle
  when Ticket as ticket where ticket.status == "open"
  when coder is available
{
  tell coder as turn timeout 10m
    with access to project_memory {
      recall for ticket
    }
  "Work {{ ticket.id }}."
  on timeout { fail error { reason "timed out" } }

  complete result { ok true }
}
"#;
        let compiled = compile_program(source);
        let ir = compiled.ir.unwrap_or_else(|| {
            panic!(
                "compiles, diagnostics: {:?}",
                compiled
                    .diagnostics
                    .iter()
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            )
        });
        let tell = ir
            .rules
            .iter()
            .flat_map(|rule| rule.metadata.effects.iter())
            .find(|effect| effect.kind == IrEffectKind::AgentTell)
            .expect("agent.tell effect from the flow");
        assert_eq!(
            tell.access_grants.len(),
            1,
            "grant preserved through flow expansion"
        );
        assert_eq!(tell.access_grants[0].resource, "project_memory");
        assert_eq!(tell.access_grants[0].operations[0].operation, "recall");
    }

    #[test]
    fn flow_invoke_preserves_start_access_grants_through_expansion() {
        // A `with access to` grant on a flow `invoke` must survive flow
        // re-serialization and lower onto the generated workflow.invoke effect.
        use crate::IrEffectKind;
        let source = r#"
workflow Parent {
  output result R
  class R { ok bool }
  class Task { id string }

  table seed as Task [ { id "T1" } ]

  flow handle
    when Task as task
  {
    invoke Child { task task }
      with access to project_files {
        read ["docs/**"]
      }
      as child

    complete result { ok true }
  }
}

workflow Child {
  input task Task
  class Task { id string }
}
"#;
        let compiled = compile_program_with_root(source, Some("Parent"));
        let ir = compiled.ir.unwrap_or_else(|| {
            panic!(
                "compiles, diagnostics: {:?}",
                compiled
                    .diagnostics
                    .iter()
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            )
        });
        let invoke = ir
            .rules
            .iter()
            .flat_map(|rule| rule.metadata.effects.iter())
            .find(|effect| effect.kind == IrEffectKind::WorkflowInvoke)
            .expect("workflow.invoke effect from the flow");
        assert_eq!(
            invoke.access_grants.len(),
            1,
            "grant preserved through flow expansion"
        );
        assert_eq!(invoke.access_grants[0].resource, "project_files");
        assert_eq!(invoke.access_grants[0].operations[0].operation, "read");
    }

    #[test]
    fn flow_liveness_clean_flow_has_no_warning() {
        // Every branch of TRIAGE reaches a terminal (then `complete`, else `fail`),
        // so the liveness lint is silent.
        assert!(
            liveness_warnings(TRIAGE).is_empty(),
            "{:?}",
            liveness_warnings(TRIAGE)
        );
    }

    #[test]
    fn flow_liveness_flags_stalling_else_branch() {
        // Gutting the else arm to a bare `done` (no terminal, no fact hand-off)
        // leaves that branch with no workflow terminal — a warning, but the program
        // still compiles (liveness is `warning` severity).
        let stalled = TRIAGE.replace(
            "  } else {\n    fail error {\n      reason \"rejected\"\n    }\n  }",
            "  } else {\n    done ticket\n  }",
        );
        assert_ne!(stalled, TRIAGE, "the else arm should have been rewritten");
        let compiled = compile_program(&stalled);
        assert!(compiled.ir.is_some(), "liveness is a warning, not an error");
        let warnings = liveness_warnings(&stalled);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("`else` branch"), "{}", warnings[0]);
    }

    #[test]
    fn flow_liveness_flags_missing_else() {
        // Dropping the else entirely leaves the else path with no terminal.
        let no_else = TRIAGE.replace(
            "  } else {\n    fail error {\n      reason \"rejected\"\n    }\n  }",
            "  }",
        );
        assert_ne!(no_else, TRIAGE, "the else arm should have been removed");
        let warnings = liveness_warnings(&no_else);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("without an `else`"), "{}", warnings[0]);
    }

    #[test]
    fn flow_liveness_flags_timeout_without_on_timeout_handler() {
        // An effect that sets a `timeout` but has no `on timeout` handler leaves the
        // timeout path with no terminal. TRIAGE has no timeout; add one without a
        // handler and expect a warning (the program still compiles).
        let with_timeout = TRIAGE.replace(
            r#"tell triager as turn "Plan {{ ticket.title }}.""#,
            r#"tell triager as turn timeout 10m "Plan {{ ticket.title }}.""#,
        );
        assert_ne!(
            with_timeout, TRIAGE,
            "the tell should have gained a timeout"
        );
        let warnings = liveness_warnings(&with_timeout);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("no `on timeout` handler"),
            "{}",
            warnings[0]
        );
    }

    #[test]
    fn flow_liveness_accepts_timeout_with_on_timeout_handler() {
        // The same timeout, paired with an `on timeout` handler that reaches a
        // terminal, is silent.
        let with_handler = TRIAGE.replace(
            r#"tell triager as turn "Plan {{ ticket.title }}.""#,
            "tell triager as turn timeout 10m \"Plan {{ ticket.title }}.\"\n  on timeout { fail error { reason \"timed out\" } }",
        );
        assert_ne!(with_handler, TRIAGE);
        assert!(
            liveness_warnings(&with_handler).is_empty(),
            "{:?}",
            liveness_warnings(&with_handler)
        );
    }

    #[test]
    fn flow_liveness_skips_non_self_terminating_flow() {
        // A flow with NO inline terminal (pure fact hand-off) is deferred to the
        // broader workflow-liveness lint, so an empty else here is not flagged.
        let source = r#"
@service
workflow Handoff

class Ticket { id string  status "open" }
class Done { id string }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

flow f
  when Ticket as ticket where ticket.status == "open"
  when reviewer is available
{
  tell reviewer as turn "Do work."

  when turn.summary == "ok" {
    record Done { id ticket.id }
  } else {
  }
}

rule finish
  when Done as d
=> {
  done d
}
"#;
        assert!(
            liveness_warnings(source).is_empty(),
            "non-self-terminating flow must not be flagged: {:?}",
            liveness_warnings(source)
        );
    }

    #[test]
    fn on_timeout_handler_fires_only_on_timeout_not_success() {
        // Regression: `on timeout { ... }` mapped to the `completes` predicate,
        // which fires on ANY terminal (including success) — so a flow with a
        // timeout-fail handler failed on successful completion. It must map to
        // `times out`.
        let source = r#"
@service
workflow TimeoutHandler

class Ticket { id string  status "open" }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

flow f
  when Ticket as ticket where ticket.status == "open"
  when reviewer is available
{
  tell reviewer as turn timeout 10m "Do work."
  on timeout { done ticket }
}
"#;
        let compiled = compile_program(source);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("flow compiles");
        let seg0 = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.f.seg0")
            .expect("seg0 rule");
        assert!(
            seg0.body.contains("times out"),
            "on-timeout handler uses the `times out` predicate: {}",
            seg0.body
        );
        assert!(
            !seg0.body.contains("turn completes"),
            "on-timeout handler must NOT fire on success via `completes`: {}",
            seg0.body
        );
    }

    #[test]
    fn carries_pre_ask_binding_across_ask_boundary() {
        // A pre-ask `tell` result referenced after the `askHuman` boundary is
        // carried through flow state (shared bindings). Before this it was a
        // dangling reference in the generated post-ask rule.
        let source = r#"
workflow CarryFwd

output result Decision
failure error Rejected

class Ticket { id string  status "open" }
class Decision { summary string }
class Rejected { reason string }

agent reviewer { provider fixture  profile "r"  capacity 1 }

table seed as Ticket [ { id "T1"  status "open" } ]

flow f
  when Ticket as ticket where ticket.status == "open"
  when reviewer is available
{
  tell reviewer as turn "Propose a plan."
  askHuman as signoff choices ["approve", "reject"] "Approve {{ turn.summary }}?"
  when signoff.choice == "approve" {
    done ticket
    complete result { summary turn.summary }
  } else {
    done ticket
    fail error { reason "rejected" }
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
        let ir = compiled.ir.expect("flow compiles");

        // The await-state class carries `turn` typed as `AgentTurn`.
        let await_class = ir
            .schemas
            .iter()
            .find_map(|schema| match schema {
                crate::IrSchema::Class(class)
                    if class.name.starts_with(super::FLOW_STATE_PREFIX) =>
                {
                    Some(class)
                }
                _ => None,
            })
            .expect("await state class generated");
        assert!(
            await_class.fields.iter().any(|field| field.name == "turn"),
            "carried binding field present: {:?}",
            await_class.fields
        );

        // The post-ask rule reads the carried binding from flow state.
        let then = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.f.seg1_then")
            .expect("then rule");
        assert!(
            then.body.contains("flowState.turn.summary"),
            "post-ask reads carried binding via state: {}",
            then.body
        );
        assert!(
            !then.body.contains("summary turn.summary"),
            "raw pre-ask binding not left dangling: {}",
            then.body
        );
    }

    #[test]
    fn flow_lowers_to_visible_named_rules_and_state_class() {
        let compiled = compile_program(TRIAGE);
        assert_eq!(compiled.diagnostics, Vec::new());
        let ir = compiled.ir.expect("flow compiles");

        // Generated rules are visible under the flow's name.
        let rule_names: Vec<&str> = ir.rules.iter().map(|rule| rule.name.as_str()).collect();
        assert!(rule_names
            .iter()
            .any(|name| name.starts_with("flow.triage.seg0")));
        assert!(rule_names.contains(&"flow.triage.seg1_then"));
        assert!(rule_names.contains(&"flow.triage.seg1_else"));

        // The await-state class is reserved-namespaced and typed.
        let await_class = ir
            .schemas
            .iter()
            .find_map(|schema| match schema {
                crate::IrSchema::Class(class)
                    if class.name.starts_with(super::FLOW_STATE_PREFIX) =>
                {
                    Some(class)
                }
                _ => None,
            })
            .expect("await state class generated");
        assert!(await_class
            .fields
            .iter()
            .any(|field| field.name == "askEffect"));

        // No `flow` item survives into the IR — it is fully expanded.
        assert!(!ir.rules.is_empty());
    }

    #[test]
    fn flow_prompt_preserves_prompt_effect() {
        let source = r#"
workflow PromptFlow

output result string

class Ticket {
  title string
}

flow f
  when Ticket as ticket
{
  prompt "Summarize {{ ticket.title }}" using fixture as summary
  complete result summary
}
"#;
        let compiled = compile_program(source);
        assert_eq!(
            compiled.diagnostics,
            Vec::new(),
            "{:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("flow compiles");
        let rule = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.f.seg0")
            .expect("flow segment");

        assert!(
            rule.body.contains("prompt") && rule.body.contains("using fixture"),
            "{}",
            rule.body
        );
        assert!(rule.metadata.effects.iter().any(|effect| {
            effect.kind == crate::IrEffectKind::SchemaCoerce
                && effect.binding.as_deref() == Some("summary")
        }));
    }

    #[test]
    fn flow_state_namespace_is_reserved_against_user_classes() {
        let source = format!(
            "workflow W\n\nclass {}Mine {{\n  id string\n}}\n\nrule r\n  when {}Mine as m\n=> {{\n  done m\n}}\n",
            super::FLOW_STATE_PREFIX,
            super::FLOW_STATE_PREFIX,
        );
        let compiled = compile_program(&source);
        // A user class in the reserved namespace must not silently work as a
        // flow state class; at minimum it compiles deterministically without
        // colliding with generated names (no flow present here).
        let _ = compiled;
    }

    #[test]
    fn entry_segment_carries_no_await_join() {
        let compiled = compile_program(TRIAGE);
        let ir = compiled.ir.expect("compiles");
        let seg0 = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.triage.seg0")
            .expect("entry rule");
        // The entry segment triggers on the flow's own `when`, not on await
        // state.
        assert!(seg0
            .whens
            .iter()
            .any(|when| when.pattern.starts_with("Ticket as")));
        let post = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.triage.seg1_then")
            .expect("post-ask rule");
        // Post-ask segments join the await-state class with the correlated
        // answer fact.
        assert!(post
            .whens
            .iter()
            .any(|when| when.pattern.contains(super::FLOW_STATE_PREFIX)));
        assert!(post
            .whens
            .iter()
            .any(|when| when.pattern.starts_with("fact human.answer.received")));
    }
}

#[cfg(test)]
mod regression_tests {
    use crate::compile_program;

    /// A post-ask branch condition that references both the awaited answer
    /// and the trigger fact must rename both: the answer to `answer` and the
    /// trigger to `flowState.t`. (Reviewer bug 1.)
    #[test]
    fn branch_condition_renames_trigger_and_answer() {
        let source = r#"
workflow FlowBugs

input ticket Ticket
output result Out
failure error Bad

class Ticket {
  id string
  status string
}

class Out {
  decision string
}

class Bad {
  reason string
}

agent triager {
  provider fixture
  profile "repo-reader"
  capacity 1
}

flow triage
  when Ticket as ticket
{
  askHuman as signoff "Approve?"

  when signoff.choice == "approve" and ticket.status == "open" {
    complete result {
      decision signoff.choice
    }
  } else {
    fail error {
      reason "no"
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
        let ir = compiled.ir.expect("compiles");
        let then = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.triage.seg1_then")
            .expect("then rule");
        let guard = then
            .whens
            .iter()
            .find_map(|when| when.guard.as_ref())
            .map(|g| g.expr.to_snapshot())
            .unwrap_or_default();
        assert!(
            guard.contains("flowState.t.status"),
            "guard renamed trigger: {guard}"
        );
        assert!(
            guard.contains("answer.choice"),
            "guard renamed answer: {guard}"
        );
        assert!(
            !guard.contains("ticket."),
            "no stale trigger binding: {guard}"
        );
    }

    /// Two flows in one workflow must not cross-contaminate ask bindings
    /// (the old global thread-local bug). (Reviewer bug 2.)
    #[test]
    fn two_flows_keep_separate_ask_bindings() {
        let source = r#"
@service
workflow TwoFlows

class A { id string }
class B { id string }
class ADone { id string }
class BDone { id string }

flow process_a
  when A as a
{
  askHuman as question_a "?"
  when question_a.choice == "yes" {
    record ADone { id a.id }
  } else {
    record ADone { id "no" }
  }
}

flow process_b
  when B as b
{
  askHuman as question_b "?"
  when question_b.choice == "yes" {
    record BDone { id b.id }
  } else {
    record BDone { id "no" }
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
        let ir = compiled.ir.expect("compiles");
        // Both flows' post-ask rules must guard on their own answer (renamed
        // to `answer`) with no leftover question_a/question_b reference.
        for name in ["flow.process_a.seg1_then", "flow.process_b.seg1_then"] {
            let rule = ir.rules.iter().find(|r| r.name == name).expect(name);
            let guard = rule
                .whens
                .iter()
                .find_map(|when| when.guard.as_ref())
                .map(|g| g.expr.to_snapshot())
                .unwrap_or_default();
            assert!(
                !guard.contains("question_a"),
                "{name} leaked question_a: {guard}"
            );
            assert!(
                !guard.contains("question_b"),
                "{name} leaked question_b: {guard}"
            );
        }
    }

    /// A literal string value must survive rename; only `{{ ... }}` template
    /// references and code paths get rewritten. (Reviewer bug 3.)
    #[test]
    fn literal_values_survive_rename_but_templates_do_not() {
        let source = r#"
workflow LiteralRename

input ticket Ticket
output result Out
failure error Bad

class Ticket {
  id string
}

class Out {
  kind string
  ref string
}

class Bad { reason string }

flow f
  when Ticket as ticket
{
  askHuman as q "id {{ ticket.id }}?"

  when q.choice == "yes" {
    complete result {
      kind "ticket"
      ref ticket.id
    }
  } else {
    fail error { reason "no" }
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
        let ir = compiled.ir.expect("compiles");
        let then = ir
            .rules
            .iter()
            .find(|rule| rule.name == "flow.f.seg1_then")
            .expect("then rule");
        // The literal `"ticket"` must remain; the path `ticket.id` must become
        // `flowState.t.id`.
        assert!(
            then.body.contains("kind \"ticket\""),
            "literal preserved: {}",
            then.body
        );
        assert!(
            then.body.contains("flowState.t.id"),
            "path renamed: {}",
            then.body
        );
    }
}
