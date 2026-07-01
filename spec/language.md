# Language Sketch

Status: draft

For a shorter author-facing reference, see
[`../docs/language-reference.md`](../docs/language-reference.md).

The language should feel close to rewrite rules:

```text
when facts are present
and guards are true
=> produce facts and durable effects
```

It should avoid table boilerplate, explicit turn schemas, and hidden
coordination nouns. Built-in orchestration facts cover work, agents, turns,
capacity, attempts, and effect lifecycle. The control plane and runtime store
provide those facts; the language defines policy over them.

## Example Shape

```whipplescript
workflow SpecImplementation

agent worker {
  profile "repo-writer"
  capacity 5
  capabilities ["agent.tell", "repo.write"]
}

agent reviewer {
  profile "repo-reader"
  capacity 1
  capabilities ["agent.tell", "repo.read"]
}

rule discover_ready_work
  when work is open as item
  when item.dependencies are done
=> {
  ready item
}

rule implement
  when {
    ready work as item
    worker is available
  }
=> {
  tell worker """markdown
  Implement this work item:

  {{ item.goal }}

  Stay within:
  {{ item.files }}
  """
}

rule review
  when worker completed work as item
=> {
  tell reviewer """markdown
  Review this work item:

  {{ item.goal }}

  Check correctness, tests, and scope.
  """
}

rule accept
  when reviewer accepted work as item
=> {
  done item -> record AcceptedWork {
    id item.id
    status "accepted"
  }
}

rule retry
  when reviewer rejected work as item
  when item.attempts < 3
=> {
  ready item
}

rule escalate
  when worker failed work as item
  when item.attempts >= 3
=> {
  askHuman """markdown
  This work item failed three times:

  {{ item.goal }}

  Please clarify, split, or cancel it.
  """
}
```

`tell worker` lowers to an `agent.tell` effect. It does not synchronously run
the provider. The runtime creates a durable effect, the harness executes it, and
the completion returns as facts/events that other rules can match.

Multiline prompts may declare an optional content type immediately after the
opening delimiter:

```whipplescript
tell worker as turn """markdown
Write a short implementation report.
"""
```

The content type token must be either a supported short name (`markdown`,
`json`, `text`, `plain`, `html`, `xml`, `yaml`, or `yml`) or a MIME-style token
containing `/`, such as `application/json` or `text/markdown`. Tokens begin with
an alphanumeric character. Later characters may be alphanumeric or one of `/`,
`.`, `+`, `-`, or `_`.

For `tell`, `askHuman`, and `coerce` prompts, the CLI lowering preserves this
value as `prompt_content_type` in the durable effect input JSON and removes the
annotation from the prompt body. Coerce effect input also preserves the source
prompt body as `prompt_template`. The annotation is metadata for reports,
rendering, and future validation. It does not invoke a parser, change provider
routing, require JSON validation, or alter rule readiness. If a supported or
MIME-style annotation is followed by extra inline text, `check` reports a
malformed content-type annotation; put prompt text on the next line.

If a rule produces multiple effects, they are unordered unless the source uses
explicit dependency syntax. Source order does not imply execution order.

## Built-In Concepts

The author should not define these in ordinary workflows:

- `work`
- `agent`
- `turn`
- `attempt`
- `available`
- `completed`
- `failed`
- `accepted`
- `rejected`
- `blocked`

The compiler lowers conversational phrases into typed facts and effects.

Core integrations also provide source-level affordances for common workflow
boundaries:

```text
<queue> has ready item
claim item
askHuman
coerce
exec <capability> with <record>
attach skill
```

These must remain visibly causal. A phrase may be friendly, but if it changes
durable state or touches the world, the lowering must be explainable as facts
and effects.

`when` is the only rule-readiness clause introducer. `with` is reserved for
effect input/configuration, such as passing typed stdin in
`exec backup_repo with request -> Report`. It must not be accepted as a synonym
for `when`; doing so would blur state observation with effect routing.

Authors may group readiness clauses to reduce repetition:

```whipplescript
rule implement_ready_issue
  when {
    backlog has ready item as item
    worker is available
  }
=> {
  claim item as work
}
```

This is source sugar for separate `when` clauses. Each non-empty line inside the
block lowers to one ordinary readiness clause; it does not introduce new runtime
semantics.

## Source Composition

WhippleScript has one explicit composition model:

```text
workflow = deployable runtime boundary
pattern  = reusable compile-time building block
rule     = atomic runtime rewrite
include  = source composition
use      = package/library import
apply    = compile-time pattern expansion
invoke   = durable child workflow invocation
complete = successful workflow terminal output
fail     = unsuccessful workflow terminal output
```

Patterns compose behavior into a workflow. Workflows run as durable instances.
The compiler may share internal representation between them, but source
semantics must not blur `apply` and `invoke`.

`use` imports package/library surface by name:

```whip
use memory
```

Packages may register library contracts, capabilities, effect providers, fact
schemas, prompt templates, resources, and optional skills. `use` does not grant
runtime authority, import skills into agent context, or include source files.

Skills are Claude-style agent context bundles. They are assigned to agents:

```whip
agent worker {
  profile "repo-writer"
  skills ["loft-user", "repo-worker"]
}
```

The intended file composition form is `include`, not `use`:

```whip
include "schemas/common.whip"
include "review.coerce"
```

`.whip` includes should splice ordinary WhippleScript declarations before
analysis. `.coerce` includes are schema-coercion backend interop artifacts: they
make coerce functions/classes available to the `std.coercion` backend without
pretending coerce is a skill or workflow-control surface. Include resolution,
cycle detection, and source-map diagnostics are still implementation work.

A `.whip` file contributes declarations to a source bundle. A deployable run
selects one workflow declaration as the root:

```text
root workflow     = selected top-level `workflow` declaration
library file      = included declarations not selected as the root
invokable workflow = top-level `workflow` declaration imported for `invoke`
```

A bundle that declares no `workflow` at all is rejected at compile time
(`program declares no `workflow``): a source with only shared types or patterns
is a library fragment meant to be `include`d, not a runnable program. There is no
implicit compatibility root. If the entrypoint or include closure contains
multiple workflow declarations, the running commands (`dev`, `deploy`) select the
root by name (`--root <name>`); a single-workflow file needs no selection.

Compilation validates **every** workflow in the bundle, not only the selected
root. Each workflow is checked against its own scope — the top-level globals in
the include closure plus that workflow's own local declarations — so a mistake in
any workflow is reported in one compile regardless of which root is chosen, and a
name declared privately inside one workflow cannot satisfy a reference in a
sibling (the reference is an unknown-name error, annotated with where the name
actually lives). Root selection then produces the single entry instance for
`dev`/`deploy`; it never narrows what is validated.

Top-level declarations in the include closure are visible by name. The source
bundle compiles into one versioned program bundle, and a selected root workflow
starts an instance from that bundle.

## Scope And Visibility

Scoping is lexical and intentionally simple:

```text
included top-level declarations -> visible in the including source bundle
workflow-local declarations     -> visible only inside that workflow
pattern-local declarations      -> visible only inside that pattern expansion
rule bindings                   -> visible only inside the rule body
effect outputs                  -> visible only after the dependency predicate
```

Top-level declarations are public within the include closure. Nested
declarations are private to their enclosing workflow or pattern. Imported
workflow internals do not pollute the parent namespace; callers see only the
workflow name and its typed input/output/failure contract.

Pattern expansion is hygienic. Applying a pattern qualifies generated helper
names under the application name so traces can still explain generated rules
without exposing accidental unqualified names:

```text
apply AgentReview<Phase, PhaseReview> as ReviewPlanPhase
```

may generate trace names such as:

```text
ReviewPlanPhase.dispatch
ReviewPlanPhase.Draft
```

but those names are implementation details unless the expansion writes an
explicit caller-visible type supplied by the application.

## Patterns

`pattern` declares a reusable parameterized program fragment. It is not a
runtime instance, not a callback, and not a mini workflow. `apply` specializes a
pattern into ordinary declarations before type checking and lowering.

The intended surface shape is:

```whip
pattern AgentReview<Input, Output> {
  input Input as item

  rule dispatch
    when {
      Input as item
      reviewer is available
    }
  => {
    tell reviewer """markdown
    Review {{ item.title }}.
    """ as turn

    after turn succeeds {
      coerce reviewWork(
        title item.title,
        summary turn.summary
      ) as review

      after review succeeds as result {
        done item -> record Output {
          turn turn
          review result
          status "reviewed"
        }
      }
    }
  }
}
```

and an application is explicit:

```whip
apply AgentReview<PhaseReviewRequest, PhaseReviewResult> as ReviewPlanPhase {
  reviewer codex
}
```

Patterns may define local helper schemas/rules/effects. They produce ordinary
facts/effects in the containing workflow after expansion. In v0, terminal actions
(`complete`/`fail`) are **forbidden in pattern bodies**: a `complete`/`fail`
inside a `pattern` is a compile-time error. Patterns elaborate into rules, and
terminals belong to the workflow that owns the contract. A reusable body that
should reach a terminal does so by recording a result fact that a workflow rule
matches and turns into `complete`/`fail` (see the `ReviewPhaseBody` example
below).

The compiler must elaborate every pattern application into a finite first-order
program before runtime. In v0, pattern expansion is **non-recursive only**: an
`apply` whose expansion (directly or transitively) reaches another `apply` of a
pattern already on the active expansion stack is a compile-time error
(`graph.unbounded_pattern_recursion`, severity `error`). The diagnostic names
the expansion cycle (pattern -> apply -> pattern ...).

Bounded recursive expansion is deferred. If it is added, it will require a
statically-decreasing structural measure over a finite structure (each recursive
`apply` must consume a strictly smaller piece of a finite, compile-time-known
type/value), so that the elaborator can prove a finite expansion before runtime.
Until that analysis exists, all recursive `apply` is rejected.

`apply` arguments are written `name value` inside the application block and bind
the pattern's declared parameters. Each argument is typed by the kind of
parameter it names:

```text
type parameter   (e.g. <Input, Output>)  bound positionally by the <...> list;
                                          each must name a known class/type
agent parameter  (a parameter the pattern uses as a tell/route target)
                                          the argument must name a declared agent
                                          in the applying workflow, e.g.
                                          `reviewer codex`
input/fact parameter (`input Input as x`) the argument must name an in-scope fact
                                          binding or fact class of the pattern's
                                          declared input type, e.g. `item phase`
value parameter  (a scalar/literal slot)  the argument must be a literal of the
                                          parameter's declared type
```

An argument that names no pattern parameter, supplies the wrong kind (e.g. a
literal where an agent is expected, or an agent name where the refined `AgentRef`
domain does not include it), or whose type does not match the parameter's
declared type is a compile-time error before expansion. All `apply` argument
typing happens against the applying workflow's declarations; pattern expansion
does not introduce new agents or facts on its own.

Current implementation status: `apply Pattern<Type> as alias { ... }` expands
pattern-local declarations with hygienic generated names such as
`alias_dispatch`, substitutes type parameters in types and rule text, applies
simple value arguments written as `name value`, and records application
provenance in IR snapshots.

## Workflow Contracts And Invocation

`workflow` declares a deployable runtime boundary. It may also declare a typed
contract:

```whip
workflow ReviewPhase {
  input phase PhaseReviewRequest
  output result PhaseReviewResult
  failure error ReviewPhaseFailure

  rule dispatch
    when {
      PhaseReviewRequest as phase
      reviewer is available
    }
  => {
    tell reviewer """markdown
    Review {{ phase.title }}.
    """ as turn

    after turn succeeds {
      complete result {
        phaseId phase.phaseId
        turn turn
        status "reviewed"
      }
    }
  }
}
```

The contract may also be written as a compact signature on the workflow line —
`workflow ReviewPhase(phase: PhaseReviewRequest) -> PhaseReviewResult !
ReviewPhaseFailure` — which desugars to the same `input`/`output`/`failure`
contracts, binding the output as `result` and the failure as `error`. The
compact form takes one or more `name: Type` inputs, an output type after `->`,
and an optional failure type after `!`. Both forms are legal; `whip fmt`
normalizes the compact signature to the keyword lines.

A terminal payload contract is either a class (a `{ field ... }` block) or a
scalar type (`int`/`float`/`string`/`bool`). A class contract takes a field
block; a scalar contract takes a bare value:

```whip
workflow Score(ticket: Ticket) -> float ! string

rule score
  when Ticket as ticket
=> {
  complete result 0.9
}
```

Giving a scalar contract a `{ … }` block, or a class contract a bare value, is a
shape mismatch and is rejected. (A parent that `invoke`s such a child observes
the child's projected terminal; typed field/value access on an invoke result is
tracked separately from this contract shape.)

Workflow inputs are initial durable facts/events, not in-memory function
arguments. Workflow outputs are terminal payload contracts, not arbitrary facts
to watch for. The only successful return path is `complete`; the only declared
failure return path is `fail`.

At start time, workflow input payloads are keyed by the declared input binding
name. For example, `input phase PhaseReviewRequest` expects:

```json
{"phase": {"phaseId": "phase-1", "title": "Review parser"}}
```

The runtime validates `phase` against `PhaseReviewRequest` and seeds a durable
`PhaseReviewRequest` fact keyed by `phase`. Rules consume it with ordinary fact
matching, for example `when PhaseReviewRequest as phase`.

```whip
complete result {
  phaseId phase.phaseId
  turn turn
  status "reviewed"
}

fail error {
  phaseId phase.phaseId
  reason failure.reason
}
```

`complete result` must name a declared `output`. `fail error` must name a
declared `failure`. A workflow may produce many intermediate facts, but none of
them complete the workflow unless a rule executes `complete`.

**Terminal tie-break.** Stepping is a deterministic fixpoint (see
[semantics.md](semantics.md)): enabled rules fire in a fixed order (rule
declaration order, then the earliest triggering fact sequence), and each
application is its own sequenced step. If more than one rule would reach
`complete`/`fail` from the same state, the **first committed terminal in that
deterministic order wins**. The moment a terminal commits, the instance is
terminal: no further effectful rule commits, and no second terminal can commit.
A later rule that would also have reached `complete`/`fail` simply does not fire,
because terminal commitment removes the state it would have matched (and the
post-terminal guard rejects it regardless). This makes the winning terminal a
deterministic, replayable function of the program and the input sequence.

Imported workflows compose at runtime through `invoke`, not `apply`:

```whip
invoke ReviewPhase {
  phase PhaseReviewRequest {
    phaseId phase.id
    title phase.title
    status "queued"
  }
} as review

after review succeeds as result {
  record ParentReviewComplete {
    phaseId phase.id
    result result
  }
}

after review fails as failure {
  record ParentReviewBlocked {
    phaseId phase.id
    reason failure.reason
  }
}
```

`invoke` creates a durable **child workflow instance**. It is not a synchronous
call and not a bare invocation effect that merely names a target: the parent's
`workflow.invoke` effect starts a real child instance that runs its own rules to
a workflow terminal (`complete`/`fail`) under its own program version and its own
revision epoch (see
[workflow-revision-transition-tracker.md](workflow-revision-transition-tracker.md);
the child may revise independently of the parent). In `after review succeeds as
result`, `result` is the declared output payload, not an untyped envelope. The
parent does not see or depend on the child's internal rules/facts except through
declared outputs, failures, events, evidence, and artifacts.

The child's lifecycle is the durable workflow lifecycle. Its terminal is
projected back into the parent invocation through the canonical terminal-output
union defined in [expression-kernel.md](expression-kernel.md):
`Completed<O> | Failed<E> | TimedOut | Cancelled`, where `O` is the child's
declared `output` type and `E` is the child's declared `failure` type. The
parent matches it with the canonical branch keywords:

```text
after <child> succeeds as result   -> Completed<O> (child ran `complete`)
after <child> fails as failure     -> Failed<E>    (child ran `fail`)
after <child> times out as timeout -> TimedOut      (status `timed_out`)
after <child> cancelled as cancel  -> Cancelled
after <child> completes as outcome -> the full union (for `case`)
```

`TimedOut` and `Cancelled` are not child workflow outputs; they are
invocation-level terminals and carry the generic union payloads from
[expression-kernel.md](expression-kernel.md): `TimedOut { summary, effect_id,
run_id }` and `Cancelled { summary, effect_id, run_id }`. Only `succeeds`/`fails`
carry child-declared payloads.

**Cancellation.** A child instance is cancelled by a control-plane operation on
the child (an explicit cancel, or a revision cancellation policy that
terminal-cancels the parent invocation effect / the child instance — see the
revision tracker), not by an ordinary parent rule reaching into child state. When
the child instance reaches the cancelled terminal, the parent's
`workflow.invoke` effect is projected as `Cancelled` and the parent observes it
through `after <child> cancelled` (or the `Cancelled` arm of `after <child>
completes`).

The compiler validates that the invoked workflow is declared in the source
bundle and that the invocation payload uses the target workflow's declared
`input` names and value types. This is a source-bundle contract check: it does
not inline the child rules into the parent workflow.

Current implementation status: `invoke Workflow { ... } as binding` lowers to a
durable `workflow.invoke` effect with structured input and dependency metadata.
In the local dev/fixture worker, that effect starts a child workflow instance
from the same source bundle, records a parent/child invocation link, and can
either run the child to a terminal state within a bounded local loop or leave
the parent effect running for a later worker pass to resume. Child
`complete`/`fail` payloads are projected back into parent `after review
succeeds`/`after review fails` blocks. If the child does not reach a workflow
terminal state within the bounded loop, the parent invocation effect is marked
`timed_out` and projected as the `TimedOut` terminal, matched by `after review
times out` (or the `TimedOut` arm of `after review completes`). If the child
instance is cancelled, the parent invocation effect is marked `cancelled` and
projected as the `Cancelled` terminal, matched by `after review cancelled` (or
the `Cancelled` arm of `after review completes`). Invocation records include
source-span metadata when the parent effect has it.

If the same logic should be usable inline and as a child workflow, define a
pattern for the reusable body and wrap it in a workflow:

```whip
pattern ReviewPhaseBody<Input, Output> {
  ...
}

workflow ReviewPhase {
  input phase PhaseReviewRequest
  output result PhaseReviewResult
  failure error ReviewPhaseFailure

  apply ReviewPhaseBody<PhaseReviewRequest, PhaseReviewResult> as body {
    item phase
  }

  rule complete_from_body
    when PhaseReviewResult as result
  => {
    complete result {
      phaseId result.phaseId
      review result.review
    }
  }
}
```

This keeps one way to compose each semantic layer:

```text
inline reuse       -> pattern + apply
runtime child work -> workflow + invoke
file composition   -> include
package/library surface -> use
workflow return    -> complete/fail
```

## External Effects

Language constructs that touch the world lower into effect categories defined in
[effects-and-capabilities.md](effects-and-capabilities.md):

```text
tell        -> agent.tell
askHuman    -> human.ask
coerce      -> schema.coerce
emit signal -> signal.emit
call        -> capability.call
```

The `emit signal` surface is the directed signal-injection construct
(`signal_emit`): `emit signal <name> to <target> { <payload> } as <binding>`. It
lowers to a durable `signal.emit` effect that validates the payload against the
target program's declared signal and appends the signal fact to the target
instance. See [event-ingress.md](event-ingress.md). This is the only `emit`
surface; the bare `emit event.name` form is removed (see
[Removed: bare `emit`](#removed-bare-emit) below).

Current implementations may still report `coerce` effects as `coerce`;
that is a backend-specific compatibility name, not the conceptual effect kind.

Registered packages may provide additional namespaced effect contracts, but not
new control-flow semantics. For example, a memory package may provide
`memory.query`; a GitHub package may provide `github.comment`. Rules still
compose those through ordinary durable effects and completion facts.

Every effect has an idempotency key, required capabilities, and a completion
contract.

The runtime enforces effect authority through a durable registry:

- capability schemas define the authority name and input contract
- effect providers bind effect kinds to executable providers
- profiles describe allowed capability sets and whether enforcement is strict
- capability bindings grant a program access to a provider for a capability

The built-in registry ships `permissive`, `repo-reader`, `repo-writer`,
`internet-research`, and `human-review` profiles. If an effect requests a
capability that is not registered, not bound, or not allowed by its profile, the
effect is blocked before a provider run starts. The block reason is written to
the event log and effect projection, so `status` and trace-conformance checks can
explain why no worker was started.

Packages are loaded as manifests that register library/effect contracts,
capability schemas, effect providers, optional profiles, and optional bindings.
They extend the registry but do not receive mutable access to kernel state or
control-flow semantics.

All fact payloads, effect payloads, and `coerce` signatures use the type system
defined in [type-system.md](type-system.md). WhippleScript supports
schema-coercion-compatible boundary types, but only a small pure expression kernel. It should not grow
loops, collection pipelines, numeric libraries, or media manipulation.

## Parser Strategy

The initial compiler uses a hand-written lexer/parser in
`crates/whipplescript-parser`. The grammar is still settling, so this keeps the
parser easy to adjust while preserving the properties the compiler needs:

```text
byte-accurate source spans
recoverable diagnostics
raw rule/effect block preservation
typed top-level syntax nodes for workflow contracts, includes, package imports,
schemas, agents, patterns, applications, coerces, assertions, and rules
```

The parser should only become generator-backed if the grammar stabilizes enough
that generated parse tables are easier to maintain than direct Rust code.

`whip check` renders diagnostics with source excerpts, caret underlines,
and next-step help where the compiler can identify a likely fix. The parser
crate also exposes formatter scaffolding that canonicalizes declarations while
preserving rule and coerce block bodies for later lowering work.

## Facts

Facts are durable workflow memory. Rules match facts in `when` clauses and
produce facts with `record`.

Every fact has provenance: runtime, rule-recorded, effect completion, external
projection, or package/provider projection. See
[fact-provenance.md](fact-provenance.md).

Typed fact declarations use classes:

```whipplescript
class ReviewedWork {
  turn AgentTurn
  review WorkReview
}
```

Producing a fact:

```whipplescript
record ReviewedWork {
  turn turn
  review review
}
```

Matching a fact:

```whipplescript
when ReviewedWork as reviewed
when reviewed.review.status == Accept
```

Rules may also attach pure guards to typed fact matches. Guards are evaluated by
the deterministic stepper before any fact or effect is committed:

```whipplescript
class LanguageTask {
  provider "codex" | "claude" | "pi"
  language string
  expectedScript string
  prompt string
  artifactPath string
  status "queued"
}

rule run_codex_language_task
  when {
    LanguageTask as task where task.provider == "codex"
    codex is available
  }
=> {
  tell codex as turn """markdown
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
}
```

The guard language must stay small and deterministic. It may read matched fact
fields, literals, enums, null, booleans, scalar comparisons, membership, and
presence checks. It must not call providers, `coerce`, package capabilities,
host-language
functions, or string parsers. If a workflow needs semantic judgment, that
judgment belongs in an explicit effect such as `coerce` or `call
validator.checkScript`, and the resulting typed fact can be matched by a later
rule.

Guards are the preferred way to express routing over a shared schema. Authors
should not need one schema per provider when the data shape is identical and
only a literal or enum field selects the target.

### Consumed Facts

Rules can finish a matched work item without mutating it in place:

```whip
rule finish
  when Task as task where task.status == "queued"
=> {
  consume task
  record Done {
    status "done"
  }
}
```

`consume binding` marks the fact bound by `when Class as binding` as consumed in
the same rule commit. `done binding` is accepted as an alias for the same
transition. Consumed facts remain in the append-only event log and replay
history, but they leave the current fact projection: later rule matching,
projection queries, and assertions do not see them by default. This is the
first-class queue-item/done transition for rules that dispatch work, record a
replacement fact, or otherwise need to avoid matching the same fact forever.

Only fact bindings introduced by `when` clauses are consumable. Effect-output
bindings are terminal observations and must be handled with `after` blocks
instead.

### Workflow Sugar

The following forms are syntax sugar over ordinary rule commits, effects,
dependencies, and fact consumption. They do not add lifecycle semantics.

Assertions use explicit expression-kernel functions:

```whip
assert count(Task where status == "queued") == 0
assert count(Result where provider == "codex") == 1
```

Use `count(query) == N` for exact cardinality and `exists(query)` for existence.
The language should not grow assertion aliases for those cases.

`record Class from binding { ... }` lets record fields copy from an in-scope
fact without repeating the binding path:

```whip
record ReviewedPoem from task {
  provider poet
  language
  topic
  artifactPath
  turn poemTurn
  review review
  status "reviewed"
}
```

Bare fields such as `language` copy `task.language`. Field values such as
`provider poet` copy `task.poet` when `poet` is a field on the `from` binding.
Explicit expressions and bindings still work.

`done binding -> record Class { ... }` combines fact consumption with the result
record produced in the same rule commit:

```whip
done task -> record ReviewedPoem from task {
  topic
  artifactPath
  turn poemTurn
  review review
  status "reviewed"
}
```

This lowers exactly like `done task` followed by `record ReviewedPoem ...`.

`after effect succeeds as output { ... }` aliases the terminal output inside
the block while keeping the original effect binding as the effect handle:

```whip
after poemTurn succeeds as turn {
  coerce reviewPoem(task.language, turn.summary) as review
}
```

Follow-up effect work must use explicit `after` blocks. Source order does not
imply effect order, and WhippleScript does not provide `then` sequencing sugar.

### Expression Parser Coverage

The source expression parser covers guards, assertions, projection filters,
table rows, typed effect arguments, interpolation paths, and branch guards with
one deterministic expression kernel. Each surface must parse to the same typed
IR nodes so validation, snapshots, Maude checks, and runtime evaluation do not
grow separate dialects.

| Surface | Accepted expression forms | Notes |
| --- | --- | --- |
| `when Fact as x where <expr>` | paths, literals, `null`, booleans, `!`, `&&`, `||`, comparisons, membership, `exists path`, `empty(expr)`, `count(query)`, arrays, map indexes | Result must be boolean. Guard `false` means non-match; guard `Error` is diagnostic and no commit. |
| Top-level `assert <expr>` | all guard forms plus fact/effect projection queries | Result must be boolean. Assertions are read-only checkpoints over committed facts/effects. |
| `Class where <expr>` | field paths rooted at the projected class alias, comparisons, booleans, membership, presence, map indexes | Projection filters are pure reads and cannot enqueue effects or call providers. |
| `effect kind K where <expr>` | effect status/kind/profile/output paths, comparisons, booleans, membership, presence, map indexes | Output paths must respect completion status and terminal-output union tags. |
| Static table rows | typed literals, arrays, records in schema context, enum/literal values, `AgentRef` values | Table rows are compile-time seed data, not runtime loops. |
| Effect and `record` arguments | typed paths, literals, arrays, records in expected schema context, `AgentRef` values | Arguments must satisfy the declared payload or fact schema before lowering. |
| Interpolation paths | field paths, optional-present paths after proof, map indexes | Interpolation is path-oriented; it does not admit arbitrary provider calls or string parsing. |
| `case expr` scrutinees and branch guards | finite-domain enum/literal/optional/tagged-union values and ordinary boolean branch guards | Branch guards reuse the same parser and evaluator as `where` guards. |

Golden IR fixtures should exercise every row with stable snapshots for source
span preservation, precedence, query reads, dynamic `AgentRef` values, and
runtime-visible `Missing` versus `null` behavior. The fixtures should include
both concise examples and one provider-language validation workflow that routes
through deterministic metadata instead of model judgment. The checked fixtures
now include expression-kernel, provider-language, terminal-output-union, and
companion-skill validation fixtures.

## Pattern Branches

WhippleScript supports only typed finite-domain pattern matching. The feature is
for deterministic branching over workflow data, not for general destructuring or
string parsing.

The v0 pattern surface is:

```text
enum variants
literal-union values
optional Some/None or present/missing branches
tagged terminal-output unions from effect completions
explicit wildcard/default branches
```

Pattern branches may have `where` guards, and those guards use the expression
kernel. Branch guards cannot call providers, invoke `coerce`, read files, parse
strings, or perform I/O.

Example finite enum branch:

```whipplescript
case review.status {
  Accept => record AcceptedWork {
    review review
  }
  Revise => record RevisionNeeded {
    review review
  }
  Blocked => askHuman """markdown
    This review is blocked:
    {{ review.reason }}
  """
}
```

Example optional branch:

```whipplescript
case issue.assignee {
  Some assignee => tell reviewer """markdown
    Review {{ issue.title }} for {{ assignee.name }}.
  """
  None => askHuman """markdown
    Assign an owner before this issue can continue.
  """
}
```

Example tagged terminal-output branch:

```whipplescript
case turn.output {
  Completed as result where exists result.artifactPath => record LanguageArtifact {
    path result.artifactPath
    summary result.summary
  }
  Failed as failure => record ProviderFailure {
    reason failure.reason
  }
  Blocked block => askHuman """markdown
    Provider run was blocked:
    {{ block.reason }}
  """
}
```

The concrete branch syntax may still change as implementation lands. The
semantic requirements are fixed:

- variants must belong to the scrutinee's enum or literal-union domain
- optional `Some` branches bind a proven-present value
- `None` branches cannot read fields through the missing value
- tagged terminal-output branches must match declared completion tags and expose
  only fields valid for that tag
- finite domains are exhaustiveness-checked when a total result is required
- non-matching branches do not commit facts or effects
- exhaustive finite-domain misses produce diagnostics, not hidden fallthrough

Deferred until there is a concrete workflow need:

```text
deep object destructuring
array/list destructuring
user-defined extractors
regex/string pattern matching
provider transcript pattern matching
```

`record` is the source-level marker for durable fact production. It is not
assignment and not an inline local variable. If a rule commits, recorded facts
commit atomically with any effect graph nodes and dependency edges produced by
the same rule.

Fact construction must satisfy the declared class schema. Unknown fields are
errors. Missing required fields are errors. Optional fields may be omitted or
set to `null`.

Conversational fact sugar is allowed for core integrations:

```whipplescript
when {
  backlog has ready item as item
  worker is available
}
```

But sugar must lower to typed fact queries. Source text should not invent hidden
workflow state.

## Correlation

Agent turns and effect outputs must carry enough correlation to avoid relying
on prompt text.

When an effect is created from a typed object, the runtime records correlation
metadata:

```text
effect_id
rule_name
source fact ids
input object ids
dependency outputs used
logical agent
capability/effect kind
```

Examples:

```whipplescript
claim item as work

after work succeeds {
  tell worker """markdown
  Implement {{ item.title }}
  """
}
```

The downstream `agent.tell` effect is correlated with the queue claim output
and the claimed item. Later completion facts can therefore support patterns
like:

```whipplescript
when worker completed turn for loft issue as turn
```

without asking the compiler to infer meaning from prompt text.

## Agent Routing

Agent targets are workflow-owned routing decisions, not model outputs. A rule
must identify the target agent deterministically through either:

- a literal declared agent name, such as `tell codex`
- a typed agent reference whose value is proven to be one of the workflow's
  declared agents
- a registered routing capability that returns a typed, auditable route before
  any provider turn is enqueued

The runtime must never ask a language model to decide which provider is being
tested, which model is active, or which logical agent should receive a turn.
Those values may be copied into result/audit facts by rule literals or typed
metadata, but they should not be fields in schema-coercion output unless the
coercion is explicitly about verifying observed provider evidence.

Dynamic agent routing is typed:

```whipplescript
class LanguageTask {
  provider AgentRef<codex | claude | pi>
  language string
  expectedScript string
  prompt string
  artifactPath string
  status "queued"
}

rule run_language_task
  when {
    LanguageTask as task
    task.provider is available
  }
=> {
  tell task.provider requires ["agent.tell"] as turn """markdown
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
}
```

The compiler rejects `tell` targets that are plain strings or non-`AgentRef`
dynamic fields. Runtime lowering resolves the `AgentRef` value from the matched
fact before enqueuing the `agent.tell` effect, so effect targets and profiles are
chosen deterministically before any provider starts.

`agent` declarations may include a finite `capabilities [...]` list. A `tell`
statement may declare the target capability contract with `requires [...]`; for
dynamic `AgentRef` targets, every possible target in the refined domain must
declare every required capability. The runtime repeats the same check against
the program-version agent metadata before starting a provider run,
so externally inserted or replayed effects cannot bypass source validation.

## Reuse And Matrices

Validation workflows often need a deterministic table: providers x languages,
phases x reviewers, or fixtures x validators. The language should provide a
source-level way to seed small static tables without hiding effects:

```whipplescript
table language_tasks as LanguageTask [
  {
    provider codex
    language "French"
    expectedScript "Latin"
    status "queued"
  }

  {
    provider claude
    language "Hindi"
    expectedScript "Devanagari"
    status "queued"
  }

  {
    provider pi
    language "Japanese"
    expectedScript "Kana and kanji"
    status "queued"
  }
]
```

Table rows lower to ordinary `record` writes during rule evaluation. They must
be fully typed and deterministic; they are not loops over runtime collections,
and they do not hide effects or provider execution. In this implementation
slice, a table declaration compiles to a generated `when started` rule that
records each row as a fact of the declared class. Each row uses the same field
assignment syntax as `record Class { ... }`, including unquoted `AgentRef`
values, scalar literals, arrays, maps, and object literals in typed contexts.

Table declarations are intended for small validation and fixture data sets.
Runtime fan-out over facts still happens through ordinary rules that match the
seeded facts. If a workflow needs to create rows from provider output, external
systems, time, or model judgment, it must use explicit effects and ordinary
`record` operations instead of `table`.

Compiled IR records table row source spans as generated rule
`record_sources`. At runtime, facts seeded from table rows use
`provenance_class: "table"` and report `source_span.construct: "table_row"` in
JSON inspection output. The source span points at the row, not at hidden runtime
logic.

## Tags

Tags are non-semantic source metadata for filtering, documentation, reports, and
future release gates:

```whipplescript
@fixture
@acceptance
workflow ProviderLanguageE2E

@fixture
table language_tasks as LanguageTask [
  {
    provider codex
    language "French"
    status "queued"
  }
]

@acceptance
assert count(LanguageTask where status == "queued") == 6
```

The current implementation accepts tags on workflows, tables, assertions, and
rules. A tag starts with `@` and uses a single non-whitespace name made from
letters, digits, `_`, `-`, `.`, and `:`. Examples: `@fixture`,
`@release-gate`, `@provider:codex`.

Tags are preserved in typed IR as source metadata. They do not change rule
readiness, rule ordering, effect routing, capabilities, provider selection,
table seeding, effects, or runtime state. `whip dev` may include or exclude
assertion evaluation by tag for validation reports, but this filtering is not
workflow execution semantics. Duplicate tags are preserved for now; reporting
may choose to de-duplicate in a future slice.

## Descriptions

Descriptions are non-semantic source metadata for reports and generated
documentation. They may appear immediately before workflows, tables,
assertions, and rules, after any tags:

```whipplescript
@fixture
description "Fixture-backed provider x language acceptance workflow"
workflow ProviderLanguageE2E

description "Static provider x language task rows"
table language_tasks as LanguageTask [
  {
    provider codex
    language "French"
    status "queued"
  }
]

description "Route one queued task to its selected provider"
rule run_language_task
  when LanguageTask as task
=> {
  done task
}
```

Descriptions are preserved in typed IR as source metadata. They do not change
rule readiness, rule ordering, effect routing, capabilities, provider
selection, assertion behavior, or runtime state. A description cannot attach to
schemas, agents, harnesses, coerces, includes, package imports, workflow contracts,
patterns, or applications in this slice.

Repeated effect chains should be reusable without obscuring the durable graph.
A rule template or action block may abstract identical `tell -> coerce ->
record` shapes only if expansion is static and inspectable in the compiled IR:

```whipplescript
action run_language_task(agent AgentRef, task LanguageTask, provider string) {
  tell agent as turn """markdown
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """

  after turn succeeds {
    coerce reviewLanguageArtifact(task.language, task.expectedScript, task.artifactPath, turn.summary) as review
  }

  after review succeeds {
    record LanguageE2EResult {
      provider provider
      language task.language
      artifactPath task.artifactPath
      turn turn
      review review
      status "reviewed"
    }
  }
}
```

This is syntactic reuse, not a general function system. Expansion must preserve
source spans, idempotency keys, dependencies, and effect/fact provenance.

Implemented per [DR-0023](decision-records/0023-action-block-rule-templates.md)
(slices 1–2, 2026-06-17). The example above is illustrative; the v0 surface that
shipped narrows it: a typed agent parameter is written `AgentRef<reviewer>` (the
type grammar requires the angle-bracket form), calls are fire-and-forget (no `as`
binding), and an action body holds the chain shape only — effect statements,
`after` blocks, `record`, and `done`; `complete`/`fail`/`case`/`branch` and
nested action calls stay in the calling rule. See `docs/language-reference.md`
§`action` and `examples/reusable-action-chain.whip`.

## Deterministic Assertions

Workflows and e2e tests need first-class deterministic assertions over facts and
effects so CI can check the intended orchestration without relying on provider
wording:

```whipplescript
assert count(LanguageE2EResult where provider == "codex") == 2
assert exists(LanguageE2EResult where language == "Japanese")
assert count(effect kind agent.tell where status == completed) == 6
assert count(effect kind schema.coerce where status == completed) == 6
```

Assertions are read-only and run after stepping or at named checkpoints. Failed
assertions should produce diagnostics and trace evidence, not partial workflow
state.

## Dependent Effects

Use `after` when one effect must wait for another:

```whipplescript
rule implement_claimed_issue
  when {
    backlog has ready item as item
    worker is available
  }
=> {
  claim item as work

  after work succeeds {
    tell worker """markdown
    Implement {{ item.title }}
    """
  }
}
```

`after` compiles to durable effect dependency edges. It is not a callback, not a
subroutine, and not general control flow.

Allowed v0 predicates correspond to the canonical terminal-output union
(`Completed<O> | Failed<E> | TimedOut | Cancelled`) defined in
[expression-kernel.md](expression-kernel.md):

```text
succeeds   -> Completed<O>
fails      -> Failed<E>
times out  -> TimedOut    (status `timed_out`)
cancelled  -> Cancelled
completes  -> the full union (binds the outcome for case)
```

`times out` and `cancelled` matter mainly for effects that can reach those
terminals, notably `invoke` (see
[Workflow Contracts And Invocation](#workflow-contracts-and-invocation)) and any
effect carrying a `timeout <duration>`. Effect outputs are available only after
the matching dependency predicate is satisfied. The compiler rejects use of
`claim.issue.title` outside the `after claim succeeds` scope.

Joins should be expressed as normal rules over completion facts, not as nested
effect graph syntax.

## Coerce

`coerce` should read like typed schema coercion from messy external output into
a declared shape, but it is semantically asynchronous and durable:

```whipplescript
rule classify
  when worker completed work as item
=> {
  coerce classifyWork(item.summary) as classification
}

rule accept
  when classification.status is Accepted for work as item
=> {
  done item -> record AcceptedWork {
    id item.id
    status "accepted"
  }
}
```

The first rule requests schema coercion. The second rule reacts when the typed
coerce output has arrived. coerce may implement the backend, but it is not the
control-flow concept. See [coerce.md](coerce.md).

## Design Pressure

The syntax must stay honest. If a construct changes durable state or enqueues
an effect, it should be visible. Conversational syntax is good only when it maps
to a small, explainable rewrite.

Bad direction:

```whipplescript
manage team until done
```

Good direction:

```whipplescript
when {
  ready work as item
  worker is available
}
=> tell worker item
```

The second form is friendly but still exposes the causal edge.

## Surface revisions (decided 2026-06-09)

The following supersede earlier statements in this document where they
conflict. Decision records: [`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md).

### Readiness matching: general form and sugar

The general readiness form matches any fact by name, including dotted
runtime facts:

```whip
when fact agent.turn.completed as turn where turn.agent == "triager"
when fact human.answer.received as answer where answer.choice == "approve"
```

Runtime events are stored and matched as facts, so `fact` is the truthful
keyword. Dotted lowercase names cannot collide with capitalized user
classes.

The English readiness phrases are documented sugar over this form — each
has a defined lowering, none is magic:

| Sugar | Lowers to |
| --- | --- |
| `when started` | the initial `external.started` event match |
| `when <agent> is available` | agent capacity readiness |
| `when human answered [<label>] as x` | `when fact human.answer.received as x` (`<label>` is documentary) |
| `when <agent> completed turn ... as x` | `when fact agent.turn.completed as x` (+ agent guard when `<agent>` names a declared agent; the word `worker` is generic) |
| `when <queue> has ready item as x` | ready-item projection match for the queue ([work-queues.md](work-queues.md)) |

`manual review requested` is removed (undocumented, unused). All Loft
phrases are removed with the work-queue design.

### Sequencing sugar: superseded

The earlier stance "WhippleScript does not provide `then` sequencing sugar"
is superseded by the `flow` construct — named flows lower multi-step
sequences to ordinary rules with compiler-managed state. See
[`flow.md`](flow.md). The design-pressure principle is preserved: the
lowering is fully visible (generated rules, reserved `flow.` state facts,
provenance spans).

### Time

`timeout <duration>` on any effect (creation-anchored), `timer <duration>
as x` effects, and the `cancel <binding>` body operation. See
[`time.md`](time.md).

### Work queues

`queue <name> { tracker <kind> }`, the `file`/`claim`/`release`/`finish`
verbs, and the builtin workspace tracker. See [`work-queues.md`](work-queues.md).

### Inline Typed Coercion And Choice Types

`decide "<prompt>" -> { <fields> } as x` is anonymous-coercion sugar: it
lowers to a generated `coerce` function and class with stable names and the
ordinary schema-coercion effect. Current implementations may still report the
legacy `coerce` effect kind. Promotion to a named `coerce` is a mechanical
refactor once a shape is reused.

`case` is permitted over string-literal-union types with exhaustiveness
checking; plain `string` scrutinees remain rejected.

`askHuman as x choices ["approve", "reject"] "..."` declares the choice set
in source: `x.choice` is typed as the literal union (case-able, exhaustive),
and the inbox presents exactly those choices.

### `exec`

Dev profile: `exec "<command>" as x` creates an `exec.command` effect gated by
operator config (`WHIPPLESCRIPT_EXEC_ALLOW`). This is a laptop-loop convenience,
not a security boundary.

Hosted profile: `exec <capability> with <record> -> Type as x` creates an
`exec.command` effect requiring `script.<capability>`. The operator manifest
supplies argv and a SHA-256 pin; the worker verifies bytes before spawn and
passes the typed record on stdin. Raw command strings are rejected in hosted
checks and workers. Evidence records exit code, truncated stdout/stderr, and
the executing hash for hosted capabilities.

### Removed: bare `emit`

The bare `emit event.name` form is removed from the surface: it had no
documented semantics and no usage. Events are the runtime's to append. This does
not remove `emit signal <name> to <target> { ... }` (construct `signal_emit`),
which is the directed signal-injection effect documented in
[External Effects](#external-effects) and
[event-ingress.md](event-ingress.md). Only the undirected bare-event form is
gone.
