# Static Analysis

Status: draft

The compiler should reject programs whose orchestration behavior cannot be
bounded or explained by the restricted rule model.

## Required Analyses

### Type Checking

Every fact, event, guard, template reference, and effect payload must be typed.
Unknown fields are errors. Optional fields must be handled before use.

Type checking uses [type-system.md](type-system.md) as the source of truth for
boundary types, [expression-kernel.md](expression-kernel.md) for guard and
assertion expressions, optional/null handling, media references, and BAML
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
validation, `coerce` argument type validation, `claim ... with loft` payload
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
- dependency predicates are limited to `succeeds`, `fails`, and `completes`
- downstream effects can reference upstream outputs only in scopes where the
  matching dependency predicate guarantees availability
- blocked downstream effects are distinguishable from provider failures
- joins are expressed through rules over completion facts, not hidden graph
  callbacks
- each effect node has its own stable idempotency key

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
