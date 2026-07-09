# Static Analysis

Status: draft

The compiler should reject programs whose orchestration behavior cannot be
bounded or explained by the restricted rule model.

Advisory checks for valid but suspicious programs belong to
[`editor-tooling.md`](editor-tooling.md) as `whip lint` rules. Static analysis
owns correctness and safety rejection; lint owns style, clarity, hygiene, and
operational-risk warnings.

## Required Analyses

### Type Checking

Every fact, event, guard, template reference, and effect payload must be typed.
Unknown fields are errors. Optional fields must be handled before use.

Type checking uses [type-system.md](type-system.md) as the source of truth for
boundary types, [expression-kernel.md](expression-kernel.md) for guard and
assertion expressions, optional/null handling, media references, and coerce
lowerability.

Fact production through `record` must be checked against a known class schema.
Conversational facts supplied by core integrations must lower to typed fact
queries before later compiler phases.

### Expression Kernel Checks

For every guard, assertion, interpolation, table row, and effect argument, the
compiler must compute:

```text
free bindings
field paths
result type
presence requirements
literal/enum domains
projection query reads
dynamic agent target domain, if any
```

The compiler rejects expressions that can evaluate to `Error` in a well-typed
state. It may keep `false` guards as ordinary non-matches, but must report
guards that are statically unsatisfiable over finite enum/literal domains when
that likely indicates a typo.

The expression diagnostic matrix is:

| Category | Rejected shape | Diagnostic should name |
| --- | --- | --- |
| Unknown binding | `task.provider` when `task` is not in scope | the missing binding and the available rule/assertion bindings |
| Unknown field | `task.model` when `LanguageTask` has no `model` field | the receiver type, bad field, and closest known fields |
| Optional misuse | `issue.assignee.name` without `issue.assignee != null`, `exists issue.assignee`, or an equivalent proof | the optional path and the accepted proof forms |
| Presence proof scope | `a || a.field == "x"` or proof only in a non-dominating branch | the proof that failed to dominate the access |
| Non-boolean guard/assertion | `where task.provider` or `assert count(LanguageTask)` | the expression result type and required `bool` result |
| Bad equality | comparing disjoint scalar or finite domains | both operand types or domains |
| Unsatisfiable finite-domain guard | `task.provider == "gpt5"` for provider domain `codex`/`claude`/`pi` | the invalid literal and the valid domain |
| Bad ordering | ordering strings, booleans, records, arrays, maps, or incompatible numeric/time types | the operator and supported ordered types |
| Bad membership | `"codex" in task.provider` or `1 in task.metadata` | collection operand type and required array/map shape |
| Bad array literal | `["codex", 1]` outside a common typed context | the incompatible element positions and expected common type |
| Object literal context | `{ provider "codex" }` where no schema or expected record type is known | the missing expected schema context |
| Invalid enum/literal value | `status == "Done"` outside the declared literal union | the declared finite domain |
| Invalid pattern branch | `case review.status { Completed => ... }` for an enum without `Completed` | the scrutinee domain and invalid pattern |
| Non-exhaustive finite branch | total `case` over enum/literal/optional/tagged union misses a branch and has no wildcard | missing variants/tags |
| Tagged terminal-output misuse | reading `turn.output.artifactPath` without matching `Completed result` first | the required output tag branch |
| Dynamic agent target | `tell task.provider` when `provider` is `string` instead of `AgentRef<...>` | the actual type and valid agent domain |
| Projection query read | `effect kind agent.tell where output.foo == "x"` before a terminal tag proves `output.foo` exists | the projection root and unavailable field |

Diagnostics should be attached to the smallest useful source span: the bad
field, operator, literal, pattern, or dynamic target rather than the whole rule.
Where the compiler has enough type information, help text should include the
source-level repair, for example adding `exists issue.assignee &&` before an
optional access or changing a provider string to an `AgentRef<codex | claude |
pi>` field.

Current checked coverage includes invalid fixtures for unknown schemas, bad
records, effect-output scope leaks, effectful self-loops, bad effect payloads,
and expression-function/query diagnostics. Parser and CLI tests also cover
finite-domain comparison typos, optional presence proofs, duration/time literal
validation, `coerce` argument type validation, `claim` payload
typing, dynamic `AgentRef` target/capability checks, terminal-output branch
tag validation, and branch-binding context safety.

The remaining static-analysis hardening work is mostly diagnostic breadth and
precision: more per-row golden fixtures, exact source spans for every lowered
effect line and branch alternative, formatter-stable expression rendering, and
full typed lowering for rule bodies that currently compile through practical
runtime metadata.

### Read/Write Sets

For each rule, compute:

```text
reads(r)
consumes(r)
produces(r)
effect_nodes(r)
effect_edges(r)
correlates(r)
```

These sets drive conflict checks, cycle analysis, and diagnostics.

### Rule Dependency Graph

Build a graph:

```text
r1 -> r2 when r1 produces a fact that r2 can consume or read
```

Classify strongly connected components:

- pure monotonic recursion: allowed
- recursion through external event or clock: allowed with rate limits
- effectful internal recursion: rejected unless explicitly proven bounded
- negation recursion: rejected unless stratified
- aggregate recursion: rejected unless stratified

### Pattern Expansion Boundedness

Pattern expansion is a compile-time elaboration, analyzed separately from the
runtime rule dependency graph above. The compiler builds the pattern-application
graph (`apply` site -> applied pattern -> nested `apply` sites) and requires that
every application elaborate into a finite first-order program before runtime.

In v0, pattern expansion is **non-recursive only**. If any `apply` reaches
(directly or transitively) another `apply` of a pattern already on the active
expansion stack, the compiler rejects it with
`graph.unbounded_pattern_recursion` (severity `error`), naming the expansion
cycle. Bounded recursive expansion is deferred and, when added, will require a
statically-decreasing structural measure over a finite structure; until then all
recursive `apply` is rejected. See
[language.md](language.md#patterns).

Terminal actions (`complete`/`fail`) inside a `pattern` body are rejected at this
stage (severity `error`): patterns elaborate into rules, and workflow terminals
belong to the owning workflow contract, not to a reusable building block.

`apply` arguments are typed against the applying workflow's declarations before
expansion (type, agent, input/fact, and value parameters per
[language.md](language.md#patterns)); an unknown, wrong-kind, or wrongly-typed
argument is an `error`.

### Effect Safety

An effectful rule must either:

- consume or advance a unique fact, or
- be keyed to a unique triggering event/correlation, or
- declare an explicit bounded `choose up to N` shape.

The compiler should reject rules that preserve their trigger while emitting new
effects.

### Effect Graph Validation

For each rule, validate the produced effect graph:

- the graph is finite and acyclic
- source order does not imply dependency
- dependency predicates are limited to the canonical terminal-output union
  branches `succeeds`, `fails`, `times out`, `cancelled`, and `completes`
  (see [expression-kernel.md](expression-kernel.md)); `times out`/`cancelled`
  apply to effects that can reach those terminals, notably `invoke` and
  effects carrying a `timeout`
- the compiler rejects `times out`/`cancelled` branches on an effect kind that
  can never reach those terminals
- downstream effects can reference upstream outputs only in scopes where the
  matching dependency predicate guarantees availability
- blocked downstream effects are distinguishable from provider failures
- joins are expressed through rules over completion facts, not hidden graph
  callbacks
- each effect node has its own stable idempotency key

### Flow Lowering And Liveness

A `flow` (see [flow.md](flow.md)) introduces no runtime semantics: it lowers
entirely to ordinary generated rules, a reserved progression-state fact class
`flow.<name>.state`, and ordinary effects. Static analysis treats the **generated
rules and facts as first-class** for every analysis above:

- the generated step rules are subject to the same read/write-set,
  effect-safety, effect-graph, and rule-dependency cycle analyses as
  hand-written rules;
- the generated `flow.`-namespaced facts participate in read/write sets and the
  rule dependency graph like any other fact.

The `flow.` namespace is reserved. The compiler enforces, at `error` severity:

- a user-declared class name may not begin with `flow.`;
- a user (non-generated) rule may not read, match, consume, or `record` any
  `flow.`-namespaced fact. Only the generated rules of the owning flow may touch
  its progression state. Generated step rules and the `flow.`-namespaced facts
  they read are exempt from the dead-rule lint because their producers are
  generated alongside them.

Flow liveness (see also the workflow liveness lint below) must account for
failure and timeout branches. A `flow` does not satisfy a workflow's
no-terminal-path liveness requirement merely by having *some* path to
`complete`/`fail`. When a flow is a workflow's only terminal path, **every**
branch of the flow — including each handled `on fails` and `on timeout` handler,
and both arms of every internal `when ... { } else { }` — must reach a workflow
terminal (`complete` or `fail`). A reachable flow step whose failure/timeout is
unhandled (so the progression stalls on the terminal effect with no terminal
reached) fails the liveness lint, because that branch leaves the workflow with no
terminal path. The diagnostic names the step and the missing handler/branch.

**Unhandled-failure auto-fail.** Liveness is a compile-time *warning*; at runtime,
a self-terminating flow that nonetheless reaches an unhandled effect failure does
not stall forever. Flow expansion generates, for each effect step in a
self-terminating flow that has no `on fails` handler, an auto-fail branch that
routes the step's failure to a generic internal failure terminal — the workflow
transitions to `failed` with a plain reason and **no** author-typed `failure`
payload (kernel `fail_instance_internal`). This applies only to self-terminating
flows (the same scope as the liveness lint; a pure fact-hand-off flow is left to
the broader workflow-liveness analysis) and only to steps without an `on fails`
handler — a handled failure fires the author's typed terminal instead, never the
auto-fail. The generated terminal is spelled `flowfail` and is generated-only:
authoring it in a user rule is rejected (use a typed `fail <Failure> { ... }`).
Modeled in `models/maude/flow-autofail.maude`.

### Workflow Liveness

For each workflow, the compiler runs a no-terminal-path liveness lint: a workflow
that can become non-terminal must have a reachable path to `complete`/`fail`. A
flow contributes to this lint under the branch-complete rule above: it counts as
a terminal path only when every one of its branches reaches a terminal. Severity
is `warning` unless a workflow declares an `output`/`failure` contract and has no
reachable terminal at all, which is an `error`.

### Constraint Preservation

Declared and built-in invariants should be checked against every rule:

- one running turn per work item unless explicitly allowed
- capacity never goes negative
- effects have stable idempotency keys
- completed work cannot become running again without a retry/reopen fact
- work cannot be accepted without a successful review when review is required

The first implementation can combine syntactic checks with generated Maude
searches for bounded counterexamples.

## Diagnostic Standard

Errors should explain:

1. what rule is unsafe
2. what cycle or invariant is involved
3. what effect could repeat or what fact could become inconsistent
4. the smallest source change that would make the intent explicit

The target quality bar is Gleam-style helpfulness: strict where the distinction
matters, generous where the compiler can safely infer intent.

Lint diagnostics should follow the same structured diagnostic contract, but
must not be the only enforcement for correctness, authority, construct graph
acceptance, lowering, or runtime lifecycle invariants.

Every diagnostic carries one severity from the single canonical enum
`error | warning | info | hint` (the same enum used across the specs). Static
analysis owns the `error`-severity correctness/safety rejections; advisory `whip
lint` checks use `warning`/`info`/`hint`.
