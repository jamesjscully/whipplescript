# Diagnostics Guide

Use this page when `whip check`, `whip dev`, `whip revise`, or runtime
inspection reports an error. Diagnostics are grouped by where they are
produced and by the repair path. For command syntax and JSON shapes, see
[CLI reference](api-reference.md) and [JSON reference](json-reference.md).

## Parse And Source Shape

### Expected a WhippleScript declaration

Cause: the file starts with free text or pasted Gherkin/Cucumber syntax.

Broken:

```text
Feature: Triage tickets
Scenario: high severity ticket
Given an open ticket
```

Fix:

```whip
workflow TicketTriage

rule start
  when started
=> {
  complete result { ok true }
}
```

WhippleScript `when` clauses are typed readiness patterns, not prose steps.

### Multiple workflow declarations require an explicit root

Cause: the source bundle contains more than one brace-wrapped workflow.

Fix:

```sh
whip check examples/revision-parent-child.whip --root ParentRevisionExample
whip run examples/revision-parent-child.whip --root ParentRevisionExample
```

The same applies to `check`, `run`, `dev`, `step`, and `revise`.

### Binding after a multiline prompt

Cause: the effect binding was placed after the closing triple quote.

Broken:

```whip
tell worker """markdown
Do the work.
""" as turn
```

Fix:

```whip
tell worker as turn """markdown
Do the work.
"""
```

## Type And Schema Checks

### Unknown schema or field

Cause: a rule, assertion, payload, or template references a class or field
that is not declared.

Broken:

```whip
rule bad
  when MissingTask as task
=> { ... }
```

Fix: declare the class, include the file that declares it, or change the rule
to match an existing class. For field errors, use the exact field name from
the class body. The checker usually includes a did-you-mean suggestion when a
nearby name exists.

Invalid fixtures:

- `examples/invalid/unknown-schema.whip`
- `examples/invalid/bad-record.whip`
- `examples/invalid/bad-effect-payload.whip`

### Object literal without an expected type

Cause: an object literal appears where the checker cannot infer a class or map
shape.

Fix: put the literal in a typed context such as `record Class { ... }`,
`complete output { ... }`, `coerce fn(...)`, or a hosted
`exec capability with <record> -> Type`.

### Incompatible expression types

Cause: a guard or assertion compares values from different domains, such as an
enum against a string or a number against a boolean.

Broken:

```whip
when Task as task where task.priority == "high"
```

Fix: use the declared domain:

```whip
when Task as task where task.priority == High
```

Invalid fixtures:

- `examples/invalid/bad-expression-functions.whip`
- `examples/invalid/bad-finite-domain.whip`

## Liveness Checks

### Workflow has no terminal rule

Diagnostic:

```text
error: workflow `X` has no rule that reaches `complete` or `fail`
```

Fix: add a rule that runs `complete <output> { ... }` or
`fail <failure> { ... }`.

For intentionally long-running services, tag the workflow:

```whip
@service
workflow WorkerDaemon
```

### Rule can never fire

Diagnostic:

```text
error: rule `X` can never fire: nothing produces `Y`
```

Fix: make `Y` producible from a workflow `input`, `table`, another rule's
`record`, or a declared external event. If the fact is truly injected by
external infrastructure, tag the rule:

```whip
@external
rule import_ticket
  when Ticket as ticket
=> { ... }
```

## Effect Graph Checks

### Unknown effect binding in `after`

Cause: an `after` block references a binding that was never introduced by an
effect in the rule body.

Broken:

```whip
after review succeeds as result {
  record Done { summary result.summary }
}
```

Fix: bind the effect first:

```whip
coerce reviewWork(item.title) as review

after review succeeds as result {
  record Done { summary result.summary }
}
```

Invalid fixture: `examples/invalid/bad-effect-graph.whip`.

### Effect output is out of scope

Cause: a rule reads an effect terminal payload outside the `after` branch that
proved that terminal status.

Fix: move the read into `after x succeeds/fails/completes`.

Invalid fixture: `examples/invalid/effect-output-scope.whip`.

## Coordination Checks

### More than one lease in one progression

Cause: a rule tries to hold multiple leases at once. The default safety model
allows one held lease per progression to avoid deadlock.

Fix: split the work across rules, or redesign the resource as one lease key.

### Missing lease/counter branch

Cause: `acquire` and `consume` are branchable effects. The checker requires
all outcomes to be handled.

Fix:

```whip
acquire deploy_slot for task.env as slot

after slot held {
  release slot
}

after slot contended {
  askHuman "Deployment slot is busy."
}
```

## Recursion And Namespace Checks

### Recursive pattern application

Diagnostic:

```text
error: recursive pattern application is not allowed (graph.unbounded_pattern_recursion): expansion cycle Loop -> Loop
```

Cause: a pattern's `apply` expands into itself, directly or through a cycle.
Pattern expansion must elaborate into a finite program.

Fix: break the cycle so expansion terminates.

Invalid fixture: `examples/invalid/recursive-pattern.whip`.

### Recursive workflow invocation

Diagnostic:

```text
error: recursive workflow invocation is not allowed (graph.unbounded_workflow_invocation_recursion): invocation cycle Ping -> Pong -> Ping
```

Cause: workflows `invoke` each other in a cycle, which has no compile-time
convergence proof.

Fix: route the recurrence through an external event, clock, or durable
boundary instead of a direct `invoke` cycle.

Invalid fixture: `examples/invalid/recursive-workflow-invocation.whip`.

### Flow-state namespace violation

Diagnostic:

```text
error: rule `X` may not reference flow-state fact `FlowAwait_...`: flow progression state is owned by the flow's generated rules
```

Cause: a user rule matches or reads a flow's internal `FlowAwait_*`
progression fact. That state is owned by the flow's generated rules.

Fix: drive the workflow from your own fact classes; a flow's `FlowAwait_*`
state is internal.

Invalid fixture: `examples/invalid/flow-state-access.whip`.

### Evidence-only fact matched as a fact

Diagnostic:

```text
error: rule `X` matches evidence-only fact `agent.turn.streamed`: in-turn observations are evidence, not rule-matchable facts
```

Cause: a rule's `when` clause matches an in-turn observation
(e.g. `agent.turn.streamed`), which is recorded as evidence rather than a
rule-matchable lifecycle fact.

Fix: match a lifecycle fact (`agent.turn.completed`/`failed`/`timed_out`/
`cancelled`) and read the in-turn detail from its evidence.

Invalid fixture: `examples/invalid/evidence-fact-match.whip`.

## Runtime And Provider Diagnostics

Provider failures do not automatically fail the workflow. They appear as
effect/run state, durable diagnostics, evidence, and trace records.

Inspect in this order:

```sh
whip status <instance>
whip effects <instance>
whip runs <instance>
whip diagnostics <instance>
whip evidence instance <instance-id>
whip trace <instance> --check
```

Common repairs:

| Symptom | Likely cause | Repair |
| --- | --- | --- |
| `blocked_by_capacity` | Agent capacity is full. | Wait, reduce concurrency, or inspect running effects. |
| `blocked_by_capability` | Agent/provider does not expose the required capability. | Fix `capabilities` or provider config. |
| `blocked_by_profile` | Profile policy denied the effect. | Use a narrower effect or bind a profile that permits it. |
| `failed` provider run | Adapter, model, script, or boundary failure. | Read `diagnostics` and `evidence`; write a policy rule to retry, escalate, or `fail`. |
| `timed_out` provider run | Timeout elapsed. | Add an `after x times out`/`after x fails` branch or retry policy. |

## Revision Diagnostics

`whip revise --dry-run` reports compatibility without mutating the store. A
rejected revision keeps the active program version unchanged.

Common failures:

| Diagnostic family | Meaning | Repair |
| --- | --- | --- |
| Root workflow changed | Candidate source changes the instance root. | Use the same root in v0, or start a new instance. |
| Contract changed incompatibly | Input/output/failure contract no longer matches in-flight state. | Preserve the contract or wait for a terminal instance. |
| Removed agent with old work | Existing old-version work still targets an agent removed by the candidate. | Keep the agent, cancel old work, or finish the instance first. |

## Assertion And Fixture Diagnostics

Assertions run after `dev` reaches idle. A failed assertion records a durable
diagnostic and links it to the assertion event. Use `--include-tag` and
`--exclude-tag` to narrow assertion groups while debugging.

Acceptance fixtures validate their own shape before running. Wrong-typed
expectations, unsupported `setup.effects`, unsupported `setup.artifacts`, and
missing assertion-read selectors are rejected as fixture errors rather than
ignored.

## Invalid Fixture Index

The `examples/invalid/` directory is the regression corpus for common
diagnostics:

| Fixture | Covers |
| --- | --- |
| `broken.whip` | Parse/source-shape errors. |
| `headerless-library.whip` | Source with no `workflow` declaration (library fragment). |
| `unknown-schema.whip` | Unknown declarations. |
| `bad-record.whip` | Record payload validation. |
| `bad-agent.whip` | Agent capacity, duplicate skills, unknown fields, missing profile. |
| `bad-effect-graph.whip` | Unknown `after` bindings and unsupported dependency predicates. |
| `bad-effect-payload.whip` | Effect payload type errors. |
| `bad-terminal-payload.whip` | Terminal `complete`/`fail` payload with no declared workflow output. |
| `bad-expression-functions.whip` | Expression function/query arity and type errors. |
| `bad-finite-domain.whip` | Enum/literal-domain misuse. |
| `effect-output-scope.whip` | Effect output visibility errors. |
| `effectful-self-loop.whip` | Effectful liveness/self-loop restrictions. |
| `evidence-fact-match.whip` | Matching an evidence-only fact as a rule-matchable fact. |
| `flow-state-access.whip` | Referencing a flow's internal `FlowAwait_*` state. |
| `recursive-pattern.whip` | Recursive pattern application (expansion cycle). |
| `recursive-workflow-invocation.whip` | Recursive workflow `invoke` cycle. |
