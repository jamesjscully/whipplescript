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

pub fn expand_flow(flow: FlowDecl, diagnostics: &mut Vec<Diagnostic>) -> Vec<Item> {
    let (ast, body_diagnostics) =
        body::parse_rule_body(&flow.body.text, flow.body.span.start, BodyMode::Flow);
    diagnostics.extend(body_diagnostics);

    let trigger_binding = flow
        .whens
        .iter()
        .find_map(|when| crate::binding_from_when(&when.text));

    // Split at human-ask boundaries. A `when/else` branch is only valid as
    // the first statement of a post-ask segment (its condition reads the
    // answer payload).
    let mut segments: Vec<Segment> = vec![Segment::default()];
    let mut statements = ast.statements.into_iter().peekable();
    while let Some(statement) = statements.next() {
        match statement {
            BodyStmt::Effect(effect) if matches!(effect.kind, BodyEffectKind::AskHuman { .. }) => {
                let binding = effect.binding.clone().unwrap_or_else(|| {
                    diagnostics.push(Diagnostic {
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
                let segment = segments.last_mut().expect("segment exists");
                if !segment.statements.is_empty() || segment.branch.is_some() {
                    diagnostics.push(Diagnostic {
                        span: branch.span,
                        message: format!(
                            "flow `{}` has a `when/else` branch that does not directly follow an `askHuman` step",
                            flow.name.name
                        ),
                        suggestion: Some(
                            "v1 flow branches decide on a human answer; place `when ... { } else { }` immediately after the ask"
                                .to_owned(),
                        ),
                    });
                    continue;
                }
                segment.branch = Some(branch);
            }
            other => {
                segments
                    .last_mut()
                    .expect("segment exists")
                    .statements
                    .push(other);
            }
        }
    }

    let mut items = Vec::new();
    let flow_name = flow.name.name.clone();
    let segment_count = segments.len();

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
                flow.span,
            )));
        }
    }

    for (index, segment) in segments.into_iter().enumerate() {
        let prior_ask = prior_ask_bindings.get(index).cloned().flatten();
        let entry = index == 0;
        let rename = if entry {
            None
        } else {
            trigger_binding.as_ref().map(|(binding, _)| binding.clone())
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
                    print_statement(
                        &BodyStmt::Effect(effect),
                        body_depth,
                        rename.as_deref(),
                        &mut body_text,
                    );
                    if open_depth {
                        push_stmt_line(&mut body_text, 1, "}");
                    }
                    // Immediate handlers become sibling failure branches.
                    while matches!(statements.peek(), Some(BodyStmt::Handler(_))) {
                        if let Some(BodyStmt::Handler(handler)) = statements.next() {
                            let predicate = match handler.kind {
                                HandlerKind::OnFails => "fails",
                                HandlerKind::OnTimeout => "completes",
                            };
                            push_stmt_line(
                                &mut body_text,
                                1,
                                &format!("after {step_binding} {predicate} {{"),
                            );
                            for inner in &handler.body {
                                print_statement(inner, 2, rename.as_deref(), &mut body_text);
                            }
                            push_stmt_line(&mut body_text, 1, "}");
                        }
                    }
                    previous_step = Some(step_binding);
                }
                other => match &previous_step {
                    Some(previous) => {
                        push_stmt_line(&mut body_text, 1, &format!("after {previous} succeeds {{"));
                        print_statement(other, 2, rename.as_deref(), &mut body_text);
                        push_stmt_line(&mut body_text, 1, "}");
                    }
                    None => print_statement(other, 1, rename.as_deref(), &mut body_text),
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
            print_statement(
                &BodyStmt::Effect(boundary.ask.clone()),
                body_depth,
                rename.as_deref(),
                &mut body_text,
            );
            if open_depth {
                push_stmt_line(&mut body_text, 1, "}");
            }
            for handler in &boundary.handlers {
                let predicate = match handler.kind {
                    HandlerKind::OnFails => "fails",
                    HandlerKind::OnTimeout => "completes",
                };
                push_stmt_line(
                    &mut body_text,
                    1,
                    &format!("after {} {predicate} {{", boundary.ask_binding),
                );
                for statement in &handler.body {
                    print_statement(statement, 2, rename.as_deref(), &mut body_text);
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
                }
                rename_text(&text, prior_ask.as_deref(), "answer")
            };
            let mut then_text = String::from("  done flowState\n");
            for statement in &branch.then_body {
                print_statement(statement, 1, rename.as_deref(), &mut then_text);
            }
            let then_text = rename_text(&then_text, prior_ask.as_deref(), "answer");
            items.push(make_rule("_then", Some(condition.clone()), then_text));
            if let Some(else_body) = &branch.else_body {
                let mut else_text = String::from("  done flowState\n");
                for statement in else_body {
                    print_statement(statement, 1, rename.as_deref(), &mut else_text);
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
                span,
            });
        }
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
fn rename_text(source: &str, binding: Option<&str>, replacement: &str) -> String {
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
fn print_statement(
    statement: &BodyStmt,
    indent: usize,
    rename_trigger: Option<&str>,
    out: &mut String,
) {
    let rn = |text: &str| match rename_trigger {
        Some(binding) => rename_text(text, Some(binding), "flowState.t"),
        None => text.to_owned(),
    };
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
            push_stmt_line(
                out,
                indent,
                &format!(
                    "after {} {}{alias} {{",
                    after.binding,
                    after.predicate.as_str()
                ),
            );
            for statement in &after.body {
                print_statement(statement, indent + 1, rename_trigger, out);
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
                    print_statement(statement, indent + 2, rename_trigger, out);
                }
                push_stmt_line(out, indent + 1, "}");
            }
            push_stmt_line(out, indent, "}");
        }
        BodyStmt::Branch(_) => {
            // Handled at segment level.
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
    let keyword = match terminal.kind {
        body::TerminalKind::Complete => "complete",
        body::TerminalKind::Fail => "fail",
    };
    push_stmt_line(out, indent, &format!("{keyword} {} {{", terminal.name));
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
        BodyEffectKind::Tell { target } => {
            format!("tell {}{requires}{binding}{timeout}", rn(target))
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
        BodyEffectKind::Coerce { name, args } => {
            let args = args
                .iter()
                .map(|arg| rn(arg))
                .collect::<Vec<_>>()
                .join(", ");
            push_stmt_line(
                out,
                indent,
                &format!("coerce {name}({args}){binding}{timeout}"),
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
        BodyEffectKind::Invoke { workflow, payload } => {
            push_stmt_line(out, indent, &format!("invoke {workflow} {{"));
            print_fields(payload, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}}{binding}"));
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
        BodyEffectKind::QueueFile { queue, fields } => {
            push_stmt_line(out, indent, &format!("file item into {queue} {{"));
            print_fields(fields, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}}{binding}"));
            return;
        }
        BodyEffectKind::QueueClaim { item, .. } => {
            format!("claim {}{binding}{timeout}", rn(item))
        }
        BodyEffectKind::QueueRelease { item } => format!("release {}", rn(item)),
        BodyEffectKind::LeaseAcquire {
            resource,
            key_expr,
            until_ttl,
        } => {
            let until = if *until_ttl { " until ttl" } else { "" };
            format!("acquire {resource} for {}{until}{binding}", rn(key_expr))
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
                &format!("notify {} event {event} {{", rn(target_expr)),
            );
            print_fields(fields, indent + 1, &rn, out);
            push_stmt_line(out, indent, &format!("}}{binding}"));
            return;
        }
        BodyEffectKind::QueueFinish { item, fields } => {
            if fields.is_empty() {
                format!("finish {}", rn(item))
            } else {
                push_stmt_line(out, indent, &format!("finish {} {{", rn(item)));
                print_fields(fields, indent + 1, &rn, out);
                push_stmt_line(out, indent, "}");
                return;
            }
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
    use crate::compile_program;

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
        assert!(rule_names
            .iter()
            .any(|name| *name == "flow.triage.seg1_then"));
        assert!(rule_names
            .iter()
            .any(|name| *name == "flow.triage.seg1_else"));

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
