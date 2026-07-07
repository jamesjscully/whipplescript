# Language reference

The complete reference for `.whip` source. For a guided introduction, start
with the [tutorial](tutorial.md); for authoring guidance, see the
[manual](manual.md).

## Program structure

A program is a root file plus its `include` closure. A file contains
declarations: at most one workflow header (or any number of brace-wrapped
workflows), classes, enums, agents, coerce functions, tables, rules,
assertions, patterns, events, coordination resources, harnesses, and imports.

```whip
include "shared/review.whip"
include "review.coerce"

use memory

workflow Example

input request WorkRequest
output result WorkResult
failure error WorkFailure

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start
  when WorkRequest as request
  when worker is available
=> {
  tell worker as turn """markdown
  Do the work:
  {{ request.title }}
  """

  after turn succeeds as completed {
    complete result {
      id request.id
      summary completed.summary
    }
  }
}
```

A single-workflow file uses the header form above. When one file bundles
several workflows, wrap each in braces and select the root with `--root`:

```whip
workflow Parent {
  input task Task
  ...
}

workflow Child {
  output result ChildResult
  ...
}
```

Both forms support the full language, including contracts and terminal
actions. Library workflows in the same bundle are invokable by name with
`invoke` unless the target workflow is tagged `@private`. A private workflow is
still validated and may be selected as `--root`, but sibling workflows cannot
invoke it.

Contracts may be written as a compact signature on the workflow line instead of
separate `input`/`output`/`failure` lines:

```whip
workflow Triage(ticket: Ticket) -> Resolution ! TriageFailed
```

is exactly equivalent to:

```whip
workflow Triage
  input ticket Ticket
  output result Resolution
  failure error TriageFailed
```

The compact form takes one or more `name: Type` inputs, an output type after
`->` (bound as `result`), and an optional failure type after `!` (bound as
`error`). Both forms are legal to write; `whip fmt` normalizes the compact
signature to the keyword lines.

A terminal payload contract is either a class or a scalar type
(`int`/`float`/`string`/`bool`). A class contract is completed with a field
block; a scalar contract with a bare value:

```whip
workflow Score(ticket: Ticket) -> float

rule score
  when Ticket as ticket
=> {
  complete result 0.9
}
```

Mixing shapes — a block against a scalar contract, or a bare value against a
class contract — is a compile error.

Every program must declare at least one `workflow`. A file that only declares
shared types or patterns is a library — `include` it from a workflow rather than
compiling it on its own; compiling it alone reports `program declares no
`workflow``. Compilation validates **all** workflows in the bundle, not just the
`--root` you select to run, so an error in any workflow is caught in one pass.
Scope is lexical: a top-level declaration (outside every `workflow { ... }`
block) is shared across the whole bundle, while a declaration written inside a
workflow block is private to that workflow. Referencing another workflow's
private name is an error — move the declaration to the top level to share it.
For workflow declarations themselves, `@private` is an invocation membrane marker,
not lexical scoping: it hides the workflow from sibling `invoke` targets while
leaving root selection available for internal runs.

## Lexical Structure

WhippleScript is line-oriented but not indentation-sensitive. Newlines
separate declarations, clauses, and block statements; braces group bodies.
Whitespace otherwise separates tokens and is not meaningful.

Identifiers start with a letter or `_` and continue with letters, digits, `_`,
or `.` where a dotted event/fact name is expected. Type and declaration names
are conventionally `UpperCamelCase`; bindings, agents, queues, resources, and
fields are conventionally `lower_snake_case`.

Line comments start with `#` or `//` outside strings and continue to the end
of the line. String literals use double quotes. Multiline prompts use triple
quotes, optionally annotated with a content type on the opener:

```whip
"""markdown
Prompt body
"""
```

The annotation is metadata; it does not validate the prompt body. Put the
prompt body on the line after the opener. Effect bindings belong on the effect
line, before the prompt opener:

```whip
tell worker as turn """markdown
Do the work.
"""
```

Reserved words cannot be used as `as` binding names. The rejected set includes
operation and control words such as `record`, `done`, `tell`, `coerce`,
`decide`, `askHuman`, `exec`, `call`, `invoke`, `signal`, `source`, `emit`,
`complete`, `fail`, `after`, `case`, `when`, `on`, `timer`, `cancel`,
`acquire`, `release`, `append`, and `consume`.

## Syntax Shape

The reference grammar below is intentionally compact; concrete examples for
each construct follow in the rest of this page.

```text
program       ::= include* use* item*
item          ::= workflow | contract | harness | agent | class | enum | event
                | table | queue | lease | ledger | counter | coerce | rule
                | flow | pattern | apply | action | assert
workflow      ::= tag* "workflow" Ident block?   # header form if block omitted
contract      ::= ("input" | "output" | "failure") Ident Type
class         ::= "class" TypeName "{" field* "}"
enum          ::= "enum" TypeName "{" variant* "}"
event         ::= "event" dotted_name "{" field* "}"
agent         ::= "agent" Ident ("using" Ident)? "{" agent_field* "}"
harness       ::= "harness" Ident ":" Ident
table         ::= "table" Ident "as" TypeName "[" row* "]"
queue         ::= "queue" Ident "{" "tracker" Ident "}"
lease         ::= "lease" Ident "{" "shared"? "key" TypeName "slots" int "ttl" duration "}"
ledger        ::= "ledger" Ident "{" "shared"? "entry" TypeName "partition" "by" Ident "retain" duration "}"
counter       ::= "counter" Ident "{" "shared"? "key" TypeName "cap" int "reset" period "}"
coerce        ::= "coerce" Ident "(" params? ")" "->" Type block
rule          ::= "rule" Ident when* "=>" block
flow          ::= "flow" Ident when* block
pattern       ::= "pattern" Ident ("<" TypeName ("," TypeName)* ">")? block
apply         ::= "apply" Ident type_args? "as" Ident block
action        ::= "action" Ident "(" params? ")" block
assert        ::= "assert" expr
when          ::= "when" readiness
block         ::= "{" statement* "}"
```

The parser is deliberately strict about source forms that look close to valid
WhippleScript but would lower ambiguously. For example, free-text Gherkin
`Given`/`When`/`Then` blocks are rejected with targeted diagnostics instead of
being treated as comments or unknown declarations.

## Declarations

### `workflow`

The runtime boundary. Starting a workflow creates a durable instance with its
own event log, facts, effects, runs, and lifecycle state.

Contracts declare what an instance accepts and produces:

```whip
input phase PhaseReviewRequest
output result PhaseReviewResult
failure error ReviewPhaseFailure
```

Input payloads are keyed by the binding name — for `input phase
PhaseReviewRequest`, start the workflow with
`--input '{"phase": {"id": "phase-1", ...}}'`, and the runtime seeds a
`PhaseReviewRequest` fact. `complete` and `fail` (below) are the only ways a
rule produces the declared outputs; cancellation is an operator action with
no source syntax.

`@private` on a workflow prevents sibling workflow invocation:

```whip
@private
workflow InternalAuditHarness {
  input request AuditRequest
  output result AuditReport
}
```

The private workflow is still a valid root for an internal run. A public wrapper
should expose its own workflow contract or shared pattern body rather than
invoking the private workflow as a sibling.

### `class` and `enum`

Typed shapes for facts and payloads:

```whip
enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class WorkReview {
  status ReviewStatus
  reason string
  confidence float
}
```

Field types follow the coerce-compatible subset: scalars (`string`, `int`,
`float`, `bool`), arrays (`string[]`), classes, enums, optionals, string
literals and literal unions (`status "open" | "done"`), and agent domains
(`AgentRef<codex | claude | pi>`). Literal-typed fields are the idiomatic way
to model small state machines.

### `agent`

An addressable target for agent turns:

```whip
agent codex {
  provider codex
  profile "repo-writer"
  capacity 2
  capabilities ["agent.tell"]
  skills ["whipplescript-author"]
}
```

`provider` (required) names the provider family: `codex`, `claude`, `pi`, or
`fixture`. `profile` (required) names the authority profile. `capacity`
bounds concurrent turns and backs `is available` readiness. `capabilities`
limits what the agent may be asked to do; `skills` attaches context bundles
to its turns.

`settings` (delegated harnesses, DR-0034) selects which of the delegate's own
ambient-config sources it may read when assembling its context: `project`,
`user`, or `none`. Unset means the provider's own default. Ambient config
steers behavior only — the profile/capabilities envelope alone grants
authority, so a delegate reading its own project config can never gain a tool
the program did not authorize.

The context knobs partition by harness class: `compaction` is managed-only
(a delegated harness compacts its own context) and `settings` is
delegated-only (WhippleScript already assembles a managed agent's context).
Declaring either on the wrong class is a compile error, never a silent no-op.

### `table`

Static seed rows, typed against a class and recorded when the instance
starts:

```whip
table language_tasks as LanguageTask [
  {
    provider codex
    language "French"
    status "queued"
  }
  {
    provider claude
    language "Hindi"
    status "queued"
  }
]
```

Use tables for deterministic source data. Anything that depends on provider
output, wall-clock time, or external systems belongs in effects and recorded
facts instead. Table-seeded facts report `provenance_class: "table"` and
carry row source spans in JSON output.

### `coerce`

A typed, coerce-backed model decision. Calling it in a rule creates a durable
`coerce` effect:

```whip
coerce assessIncident(title string, impact string, mitigation string) -> IncidentAssessment {
  prompt """markdown
  Assess the incident response.

  Incident: {{ title }}
  Impact: {{ impact }}
  Proposed mitigation: {{ mitigation }}

  {{ ctx.output_format }}
  """
}
```

`coerce` is an effect, not a function call: the typed output is available
only in an `after ... succeeds` branch.

For a one-off decision that does not warrant a named declaration, use the
inline `decide` form inside a rule body — see
[Inline `decide`](#inline-decide).

### `pattern` and `apply`

Compile-time reuse. `apply` expands a pattern into ordinary declarations
before type checking:

```whip
pattern AgentReview<Input, Output> {
  input Input as item

  rule dispatch
    when Input as item
    when reviewer is available
  => {
    tell reviewer as turn "Review {{ item.title }}."

    after turn succeeds as reviewed {
      done item -> record Output {
        turn reviewed
        status "reviewed"
      }
    }
  }
}

apply AgentReview<PhaseReviewRequest, PhaseReviewResult> as ReviewPlanPhase {
  reviewer codex
}
```

Patterns have no runtime identity. When the reused work needs its own
lifecycle, use a `workflow` and `invoke` it.

### `action`

Compile-time reuse of a repeated *effect chain inside a rule body*. Where
`pattern`/`apply` abstract whole declarations, an `action` abstracts a chain of
statements and is inlined at each call site, fully expanded into the durable
graph:

```whip
action review_change(who AgentRef<reviewer>, item ChangeRequest) {
  tell who as turn """markdown
  Review {{ item.title }}.
  """

  after turn succeeds as reviewed {
    done item -> record ReviewedChange {
      id item.id
      summary reviewed.summary
      status "reviewed"
    }
  }
}

rule review
  when ChangeRequest as item
  when reviewer is available
=> {
  review_change(reviewer, item)
}
```

Semantics:

- **Inline, hygienic expansion.** The call is replaced by the action body with
  arguments substituted for parameters. The action's internal bindings (`turn`,
  `reviewed` above) are uniquified per call site, so two calls in one rule body
  never collide. The compiled rule shows the expanded chain — there is no runtime
  call, frame, or recursion.
- **Fire-and-forget (v0).** A call is a standalone statement; it cannot be bound
  with `as`.
- **Chain shape (v0).** An action body may contain effect statements, `after`
  blocks, `record`, and `done`. `complete`/`fail`/`case`/`branch` and nested
  action calls are not allowed in a body (v0) — keep terminal and branching logic
  in the calling rule.

Like patterns, actions have no runtime identity; they are reuse, not subroutines.

### `redact`

Projects a record-typed binding onto a chosen subset of its fields, producing a
new binding that carries **only** the kept fields:

```whip
rule triage
  when Customer as c
=> {
  redact c keep [id, status] as safe

  complete result {
    who   safe.id
    state safe.status
  }
}
```

`redact <source> keep [<field>, …] as <out>` is a synchronous, pure restructure —
not an effect. It is the explicit information-flow crossing at which the
rule-level analysis is refined: the projection is the deliberate, auditable point
where a record is narrowed before it flows onward (a focused complement to the
audited `declassify` hatch documented in the `examples/infoflow` README).

Semantics:

- **Kept-only type.** `out` has a synthesized type holding just the kept fields,
  so `safe.id` resolves but accessing a dropped field (`safe.ssn`) is a compile
  error. The source schema must be known and every kept field must exist on it.
- **Runtime projection.** At runtime the dropped fields are physically removed
  from the value bound to `out`, so they can never leave through any sink — the
  runtime teeth behind the static drop (proven in
  `models/lean/Whipple/Redaction.lean`: the dropped fields are non-interfering).
- **Information-flow refinement.** Under a governance envelope, a fully-redacted
  egress — `complete result`, `record <Schema>`, or `send via <channel>` that
  references only redacted projections — is additionally checked against the kept
  fields' per-field label join (envelope resources keyed `<Schema>.<field>`):
  keeping a field the sink cannot read is flagged, naming the field. This check is
  **additive** — it does *not* exempt the egress from the rule's ordinary
  read→sink checks. In particular, releasing data derived from a confidential
  **resource read** at a lower label is a declassification and still requires a
  `grant declassify` (dropping a field only narrows the per-field *schema* label,
  not the provenance of a confidential source). The projection narrows
  confidentiality only; the integrity check is unaffected.
- **Source kinds.** The source may be a matched class (`when Class as c`), a
  coerce/decide/exec result, or the alias of an `after … succeeds as <alias>`
  branch (the read-then-redact flow). Redactions may chain: a redaction's output
  can be the source of a later one.
- **Bounded-type projection (no explicit `redact`).** A pure `from` projection —
  `record <T> from <src> { f1  f2 }` or `complete <result> from <src> { f1  f2 }`
  whose fields are all shorthand copies — is auto-governed as a redaction keeping
  `[f1, f2]`: the egress is checked against those fields' per-field labels, so
  projecting only public fields is fine and including a confidential one is flagged
  (naming the field). The target type is the explicit, reviewable bound; the labels
  are the source's. A payload mixing explicit value expressions is not a pure
  projection and stays conservative. (Like the explicit `redact` check, this is
  additive — it does not exempt the egress from the rule's read→sink checks.)

### `include` and `use`

```whip
include "schemas/common.whip"   // contributes declarations to the bundle
include "review.coerce"           // makes coerce classes/functions available to coerce
use memory                      // imports a package/library by name
```

Packages register libraries, capabilities, providers, schemas, and optional
skills. Their capabilities are invoked as explicit `call` effects. A locked
package can also authorize constrained library-owned forms, such as memory
`recall`, which still lower to ordinary core effects.

### Tags and descriptions

Source metadata on workflows, tables, rules, and assertions:

```whip
@private
@acceptance
description "Internal provider x language acceptance workflow"
workflow InternalProviderLanguageE2E
```

Both are preserved in compiled IR for reports. `whip dev --include-tag` /
`--exclude-tag` filter which assertions are evaluated. `@service` and
`@external` carry static meaning for the [liveness checks](#liveness-checks).
`@private` on a workflow is semantic and prevents sibling `invoke`; other tags
do not change readiness, routing, effects, or runtime behavior.

### Signals and sources

A `signal` declares a typed external signal and its payload schema. A `source`
block declares how a signal is admitted as a durable fact. Rules react to the
admitted signal; a source never fires a rule directly.

```whip
signal triage.tick {
  scheduled_at time
  observed_at time
  occurrence_id string
  missed_count int
}

source clock as daily_triage {
  every weekday at 09:00          // recurrence: `at <hh:mm>`, `every <dur>`,
  timezone "America/New_York"     //   or `every <day|weekday|monday…> at <hh:mm>`
  missed coalesce                 // `skip` | `coalesce` | `catch_up limit <N>`

  observe as tick                 // binds the provider observation
  emit triage.tick {              // maps the observation into the declared signal
    scheduled_at tick.scheduled_at
    observed_at tick.observed_at
    occurrence_id tick.occurrence_id
    missed_count tick.missed_count
  }
}
```

`source clock as <name>` is the `clock_source` construct (provided by
[`std.time`](providers.md)); a generic `source <provider> as <name> { observe; emit }`
is the `signal_source` construct. Both belong to the `source_declaration`
construct family and lower to an admission template — they emit a durable signal
fact, never a rule. Static checks: a recurring clock source must declare a
`missed` policy (no silent default), and a calendar schedule should declare a
`timezone` (otherwise it defaults to UTC with a diagnostic). Each clock
occurrence is admitted at most once, keyed by its scheduled instant, so replay
and recovery never double-admit (see the `admission-and-idempotency` and `std-time`
specs). At runtime the worker fires all three recurrence forms — `every <duration>`
(interval), `every <day|weekday|monday…> at <hh:mm>` (calendar), and `at <hh:mm>`
(a single occurrence) — resolving calendar/`at` times in the declared timezone and
honoring daylight-saving transitions (a `09:00` local schedule shifts its UTC
instant across the DST boundary; a nonexistent spring-forward local time is skipped).

## Rules

A rule waits for facts and events, optionally filters them with guards, and
commits a rewrite atomically — either every fact, effect, dependency, and
terminal action in the selected rule persists, or none do.

```whip
rule resolve_incident
  when IncidentTicket as ticket where ticket.status == "open"
  when responder is available
=> {
  tell responder as turn """markdown
  Investigate this incident and propose a mitigation plan.

  {{ ticket.title }}
  Impact: {{ ticket.impact }}
  """

  after turn succeeds as completed {
    coerce assessIncident(ticket.title, ticket.impact, completed.summary) as assessment
  }

  after assessment succeeds as checked {
    done ticket -> record IncidentResolution from ticket {
      mitigation completed.summary
      risk checked.risk
      status "resolved"
    }
  }
}
```

Several `when` lines may also be grouped in a block; the forms are
equivalent:

```whip
rule start
  when {
    WorkRequest as request
    worker is available
  }
=> { ... }
```

### Readiness patterns

| Pattern | Matches |
| --- | --- |
| `when started` | The initial `external.started` event. Use for seed rules. |
| `when <agent> is available` | Free capacity on the agent. |
| `when Class as x [where ...]` | An unconsumed fact of `Class`. |
| `when human answered <label> as x` | A `human.answer.received` fact from an answered inbox item. |
| `when <agent> completed turn ... [as x]` | An `agent.turn.completed` fact. A declared agent name matches only that agent's turns; the generic word `worker` matches any agent. |
| `when <queue> has ready item as x` | An item that is ready to be claimed in a [work queue](#work-queues). |
| `when fact <dotted.name> as x [where ...]` | The general form: any derived fact whose name matches the dotted path. |

All of the readiness patterns above are sugar over `when fact`. `when human
answered signoff as x` is shorthand for matching a `human.answer.received`
fact; `when reviewer completed turn as x` matches `agent.turn.completed`; and
`when backlog has ready item as x` matches a work-queue readiness fact. Reach
for the general `when fact <dotted.name>` form when the event you need does
not yet have a dedicated phrase, for example
`when fact agent.turn.completed as turn`.

The `<label>` in `human answered <label>` is a readability label, not a
binding reference. Discriminate between multiple pending reviews with guards
on the answer payload, which exposes `choice`, `text`, `answered_by`,
`prompt`, `inbox_item_id`, and `effect_id`:

```whip
rule approve
  when human answered signoff as answer where answer.choice == "approve"
=> {
  complete result {
    decision answer.choice
    decidedBy answer.answered_by
  }
}
```

Declare the allowed answer choices in source with a `choices` list on the
`askHuman` action, so the inbox offers exactly that set:

```whip
askHuman as signoff choices ["approve", "reject"] "Approve {{ turn.summary }}?"
```

The declared choices form a string-literal union, so a later `case
answer.choice { ... }` over them is exhaustiveness-checked.

A pending ask is bound to its instance: if the instance reaches a terminal
before the ask is answered — including an operator `whip cancel` — the runtime
retires the ask (it leaves the inbox and can no longer be answered), so an
operator never spends a decision on a dead instance.

### Guards and expressions

`where <expr>` is pure, deterministic filtering over matched facts and
literal values. Supported forms:

```text
field access                    task.review.status
comparison                      ==  !=  <  <=  >  >=
boolean                         and  or  not      (&&, ||, ! accepted)
membership                      x in [...]        x not in [...]
presence                        exists x          empty x
finite queries                  count(Class where ...)   exists(Class where ...)
literals                        strings, numbers, booleans, null, arrays, objects
indexing                        task.metadata["phase"]
enum / finite-domain values     Accept, codex
```

Expression precedence, from tightest to loosest:

| Level | Operators/forms | Notes |
| --- | --- | --- |
| 1 | field access, indexing, function/query call | `task.owner.name`, `metadata["phase"]`, `count(Task where ...)` |
| 2 | unary presence/boolean | `not x`, `!x`, `exists x`, `empty x` |
| 3 | comparison and membership | `==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not in` |
| 4 | conjunction | `and`, `&&` |
| 5 | disjunction | `or`, `||` |

`and` and `or` short-circuit left to right. Comparisons are type-checked:
numbers compare with numbers, strings with strings, booleans with booleans,
finite domains with their own domain, durations with durations, and times with
times. There is no implicit string-to-number, string-to-bool, or enum-to-string
coercion in guards. If a guard cannot evaluate to a boolean, the rule does not
commit and the checker or runtime report points at the expression.

Prefer the word operators in new code:

```whip
when Job as job where job.status == "pending" and job.attempts < 3
```

Guards never perform I/O — no provider queries, coerce calls, file reads,
clocks, or randomness. A decision that needs model judgment or external data
becomes an effect, and a later rule branches on its completion.

### Rule body operations

| Operation | Meaning |
| --- | --- |
| `record Class { ... }` | Create a typed fact. |
| `record Class from binding { ... }` | Create a fact by copying a binding's fields and overriding the listed ones. |
| `done binding` | Consume a matched fact. |
| `done binding -> record ...` | Consume and replace in one atomic commit. |
| `tell agent [requires [...]] [as x] [timeout <dur>] [with access to <resource> { ... }] "..."` | Enqueue an `agent.tell` effect (see [turn-access grants](#turn-access-grants)). |
| `coerce fn(...) as x` | Enqueue a typed `coerce` effect. |
| `decide "..." -> { ... } as x` | Enqueue an inline typed model decision (see [Inline `decide`](#inline-decide)). |
| `askHuman [as x] [choices [...]] "..."` | Enqueue a human review request. |
| `file item into <queue> { ... }` | File a new item into a [work queue](#work-queues). |
| `claim <item> [as x]` | Claim a queue item; already-claimed is a branchable failure. |
| `release <item>` | Return a claimed item to the queue. |
| `finish <item> [{ summary ... }]` | Mark a queue item done. |
| `timer <dur> as x` | Create a [timer effect](#time-and-deadlines) that fires when due. |
| `timer until <time> as x` | Create an absolute [timer effect](#time-and-deadlines) that fires at or after a typed instant. |
| `cancel <binding>` | Cancel a pending or running effect bound earlier. |
| `exec "<command>" as x` | Enqueue a dev-profile gated command effect (see [`exec`](#exec)). |
| `exec <capability> with <record> -> Type as x` | Enqueue a hosted script capability effect with typed stdin/stdout. |
| `call package.capability ... [as x]` | Enqueue a package capability effect. |
| `recall from <pool> for <query> as x` | Package-owned memory form; requires a lock that authorizes lowering to memory recall capability. |
| `emit signal <name> to <instance> { ... } as x` | Enqueue typed signal injection to another instance. |
| `acquire <lease> for <key> as x` | Acquire a workspace-scoped lease; branch on `held` or `contended`. |
| `release <lease-binding>` | Release a lease acquired earlier in the rule progression. |
| `append Type { ... } to <ledger> as x` | Append a typed entry to a partitioned ledger. |
| `consume <counter> for <key> amount <expr> as x` | Consume from a bounded counter; branch on `ok` or `over`. |
| `invoke Workflow { ... } [with access to <resource> { ... } \| with access to { <resource> { ... } ... }] as x` | Start a durable child workflow, optionally narrowing the child start authority. |
| `after x succeeds as y { ... }` | Run when effect `x` completes successfully. |
| `after x fails as y { ... }` | Run when effect `x` fails; `y` binds the failure base (see below). |
| `after x completes { ... }` | Run on any terminal status of `x`. |
| `emit milestone "<name>" of <Class> { ... }` | Project a named, durable milestone mid-flight for an observing parent (Family C). |
| `after p reaches "<name>" as m { ... }` | Run when invoked child `p` projects milestone `<name>`; `m` binds its payload. |
| `case value { Pattern => { ... } }` | Branch over a finite-domain or union value. |
| `complete output { ... }` | Emit the declared workflow output; the instance completes. |
| `fail failure { ... }` | Emit the declared failure payload; the instance fails. |

`consume binding` is a deprecated alias for `done binding`; it compiles with
a warning and will be removed — prefer `done`.

The bare `emit <name>` action has been removed from the language; `emit` must be
followed by `signal` (directed event injection to a peer instance) or `milestone`
(a child-milestone projection — see below). Workflows otherwise append durable
events through ordinary effects and the facts their completions derive.

Binding names introduced with `as` must not shadow operation keywords —
`done`, `record`, `tell`, `complete`, `fail`, and the rest are rejected as
binding names.

### Typed effect failures (`after x fails as f`)

When you bind a failure with `after x fails as f`, `f` carries a uniform **failure
base** — the same shape for every effect kind:

| Field | Meaning |
|---|---|
| `f.reason` | The human-facing failure text. |
| `f.summary` | A short summary (often the same as `reason`). |
| `f.effect_id` / `f.run_id` | Identifiers locating the failed effect run. |
| `f.kind` | The failing effect kind (e.g. `"exec"`, `"coerce"`, `"workflow.invoke"`). |

Reading any other field off `f` is a check error — effect-specific failure detail
(an `exec` exit code, a provider error code) is **not** exposed yet; it is reserved
for a future per-kind refinement and only reachable once that lands. Use `f.reason`
for the failure text regardless of which effect failed. (Design: DR-0032 — effect
failure is the `EffectError` discriminated family; the base is committed, per-kind
extras are deferred behind narrowing.)

### Child-milestone lifecycle (`emit milestone` / `after ... reaches`)

A child workflow can project named, durable **milestones** that an invoking
parent observes mid-flight — generalizing the terminal outcome family
(`succeeds`/`fails`/`completes`) to a lifecycle family over states the child
explicitly declares:

```whip
// in the child workflow
class Progress { detail string }

rule do_work when Task as task => {
  emit milestone "work_started" of Progress { detail task.title }
  tell worker as turn """..."""
  after turn succeeds { complete result { ... } }
}

// in the parent workflow
rule orchestrate when Task as task => {
  invoke Child { task { title task.title } } as child

  after child reaches "work_started" as m {   // m : Progress
    record ParentProgress { note m.detail }
  }
  after child succeeds as r { complete result { ... } }
}
```

Rules:

- A milestone is *declared by emitting it*: `emit milestone "<name>" of <Class>`
  names the projection and types its payload `<Class>`. The `of <Class>` clause
  is optional for a payload-less milestone.
- `after p reaches "<name>" as m` reacts when the invoked child `p` projects that
  milestone; `m` binds the milestone payload. The terminal handlers
  (`after p succeeds` / `fails`) are independent and unchanged — milestone
  observation does not displace terminal observation.
- The name in `reaches` must be one the invoked child actually declares; reaching
  an undeclared milestone is a check error (a parent cannot observe a state the
  child never projects — the *terminal-only observation* invariant).
- Delivery is poll-based and exactly-once: the parent observes each emitted
  milestone on a single derived fact; a milestone the child never emits produces
  no reaction. Observation latency is bounded by the parent's invoke step.

### Turn-access grants

A `tell` may narrow the turn's authority to specific resources with one or more
`with access to <resource> { <grant clauses> }` modifiers, written between the
target and the prompt:

```whip
tell coder as turn
  with access to project_memory {
    recall for issue
    learn for issue
  }
  with access to project_files {
    read ["docs/**"]
  }
"Work the issue."
```

The equivalent grouped shorthand is also accepted and desugars to the same grant
list:

```whip
tell coder as turn
  with access to {
    project_memory {
      recall for issue
      learn for issue
    }
    project_files {
      read ["docs/**"]
    }
  }
"Work the issue."
```

Each grant clause is an operation grant — an operation name with an optional
`for <ref>` target and/or `["glob", …]` path patterns. The grant is
authority-*narrowing* metadata on the `agent.tell` effect (Proposal A): the turn's
effective authority is the intersection of the agent profile and the grant, so a
grant can only restrict, never widen, what the profile already permits. In-turn tool
calls are recorded as evidence, not durable child effects.

A grant must list at least one operation, must not name the same resource twice on
one `tell`, and — for a declared `file store` resource — may only use the file
operations `read`/`write`/`import`/`export`.

For the owned harness, file tools are deny-by-default unless the turn input
carries a file-store grant; ungranted file tools are not offered to the model.
`read`/`import` authorize read-like file tools and `write`/`export` authorize
write-like file tools, with each call checked against the granted globs. `edit`
is offered only with both read and write grants and also requires both at
execution because it reads existing content before rewriting it. Built-in
profiles also narrow the owned harness tool surface: `repo-reader` and
`human-review` are read-only,
`repo-writer`/`permissive`/`release-operator` may mutate subject to grants, and
`no-repo`/`internet-research` receive no filesystem/bash tools. Registered custom
profiles are also consulted: the owned harness maps `repo.read`, `repo.write`,
and `command.run` from `allowed_capabilities` to read, write, and bash tool
authority before intersecting turn grants; tracker mutations use
`tracker.file`, `tracker.claim`, `tracker.finish`, `tracker.release`,
`tracker.update`, or `tracker.write`; curated `@tool` sub-workflow tools use
`workflow.invoke`. If the `tell` uses `requires [...]`, the owned harness also
intersects the tool surface with those known harness capabilities; the store
blocks the turn before provider launch if the target agent did not declare them.
When an IFC governance envelope is active, every
file-store resource named by a turn grant must also be governed by that envelope
before the owned turn is admitted. Bash is offered and executed only when ALL of
the following hold: the profile/registry/required-capability set permits
`command.run`; the turn carries `with access to command { run }`; the command
matches the operator allow-list (`WHIPPLESCRIPT_HARNESS_BASH_ALLOW` — with no
allow-list, every command is refused); the command is a single simple command
(shell control operators, pipes, command substitution, backticks, and
variable/glob/brace/tilde expansion are refused before execution); literal
shell file redirection targets pass the same turn globs as file tools (`<` uses
read globs, `>`/`>>` use write globs; dynamic redirection targets are refused);
path-shaped arguments stay inside the workspace (absolute, `~`, and `..` paths
are rejected); and, when an IFC governance envelope is active, the `command`
resource is governed by that envelope. Command-specific side-effect
classification (per-tool argv operand policies) is deliberately not part of
this surface — the simple-command policy plus the operator allow-list is the
whole enforcement boundary.
Tracker `list_todos` is read-only;
`add_todo` requires
`with access to tracker { file }`, and `update_todo` requires the matching
`claim`/`finish`/`release` grant (or `update`/`write`). When an IFC governance
envelope is active, mutating tracker authority requires the `tracker` resource to
be governed.
Provider configs with `profile_ids` also act as endpoint allow-lists and block
mismatched agent-turn profiles before provider launch. Broader
governance-envelope label/argument policy and future provider/tool capability
mappings remain open implementation work.

### Effect ordering and scope

Source order inside a rule body does not order effects. `after` blocks create
the durable dependency edges, and an effect's output is visible only inside
the `after` branch that proves its terminal status:

```whip
tell worker as turn "Do the task."

after turn succeeds as completed {
  record TurnSummary {
    text completed.summary
  }
}
// `completed` is not in scope here
```

This is what keeps rule lowering deterministic and event causality
explainable.

### Matching and commits

Each worker pass evaluates rules against the current projection for one
instance and active program version. A `when Class as x` clause ranges over
unconsumed facts of that class. Multiple fact clauses form a deterministic
join: the rule is ready for each binding tuple that satisfies all clauses and
guards. A rule that matches three `Ticket` facts can therefore commit three
separate progressions, one per ticket, unless a consumed fact or terminal state
prevents a later progression.

Readiness clauses that are not facts, such as `when worker is available` or
`when backlog has ready item as item`, are projected facts or policy gates
checked at the same boundary. Guards run after bindings are selected and before
the commit is built.

A rule commit is atomic. The runtime records consumed facts, new facts, new
effects, dependency edges, diagnostics/evidence, and workflow terminal actions
in one transaction. If a typed payload, guard, branch, dependency, or terminal
contract cannot be validated, none of that rule's outputs land.

Facts are set-like by class plus key when a stable key is present; facts that
need multiplicity must carry distinct keys. Consuming a fact removes it from
future unconsumed matches, but the historical event/fact record remains
inspectable. Terminal workflow states are absorbing: once a commit reaches
`complete` or `fail`, later rule commits are rejected.

### `case`

Deterministic branching over an enum field or an effect's terminal union:

```whip
after turn completes {
  case turn {
    Completed as completed => {
      record TurnReport { branch "completed" summary completed.summary }
    }
    Failed as failure => {
      record TurnReport { branch "failed" detail failure.reason }
    }
  }
}
```

`case` also branches over a string-literal-union type — a field declared
`status "approve" | "reject"` or an inbox choice set — with the same
exhaustiveness checking:

```whip
case answer.choice {
  "approve" => { complete result { decision answer.choice } }
  "reject"  => { fail error { reason "rejected" } }
}
```

The scrutinee must have a finite-domain type: an enum, a terminal union, a
string-literal union, an optional, or a `bool` (matched with the literals `true`
and `false`). `case` over a plain `string` is rejected — use guarded rules
instead. A `bool` `case` is exhaustive only when it covers both `true` and
`false` (or carries a `_`). See
[`examples/terminal-output-union.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/terminal-output-union.whip)
for exhaustive terminal handling.

### Inline `decide`

`decide` is an anonymous, typed model decision inside a rule body — a coerce
with its schema written in place, for the case where a named `coerce`
declaration would only be used once:

```whip
decide "Is this plan safe to ship? Explain." -> { fixed bool, reason string } as verdict

after verdict succeeds as v {
  record ShipReview {
    fixed v.fixed
    reason v.reason
  }
}
```

It lowers to the same `coerce` effect as a named `coerce`, so the same
rules apply: it is durable, it can fail, and its typed output is available
only in an `after ... succeeds` (or `after verdict succeeds`) branch. Use a
named `coerce` when the decision is reused or deserves a documented prompt;
reach for `decide` for a local, single-use judgment.

An inline `decide`'s anonymous result type flows across the `after ... succeeds`
binding exactly like a named `coerce -> Schema`, so you can field-access *and*
`case` on a `decide` result. To *branch* on a decision, give the result a
`bool`, enum, or string-literal-union field and `case` on it:

```whip
decide "Is this plan safe to ship? Explain." -> { fixed bool, reason string } as verdict
after verdict succeeds as v {
  case v.fixed {
    true => { complete result { decision "ship" } }
    false => { askHuman "Held: {{ v.reason }}" }
  }
}
```

A named `coerce` is still the right choice when the decision shape is reused —
its declared class is a documented, shared contract:

```whip
class ShipVerdict { decision "ship" | "hold"  reason string }

coerce assessShip(plan string) -> ShipVerdict { prompt "{{ plan }}" }

# in a rule body:
coerce assessShip(ticket.plan) as verdict
after verdict succeeds as v {
  case v.decision {
    "ship" => { complete result { decision "ship" } }
    "hold" => { askHuman "Held: {{ v.reason }}" }
  }
}
```

## Flows

A flow is a rule whose body is a multi-step sequence. It reads top to bottom
like a script, while lowering to ordinary rules so the runtime stays the
deterministic rule kernel described above:

```whip
flow triage
  when Ticket as ticket
{
  tell triager as turn "Plan {{ ticket.title }}."
  askHuman as signoff "Approve {{ turn.summary }}?"

  when signoff.choice == "approve" {
    complete result { decision signoff.choice }
  } else {
    fail error { reason "rejected" }
  }
}
```

A flow's `when` clause is the same readiness pattern a rule uses; it
determines when the flow starts and how many times. A flow fans out per
matched fact, so the example above runs once for each `Ticket`.

Steps chain implicitly. Each effect step's output binding (`turn`, `signoff`)
is in scope for every later step, so you write the sequence directly instead
of nesting `after` blocks. Branching uses `when <expr> { ... } else { ... }`
on a prior step's output; in v1 the branch must come directly after an
`askHuman` step.

Per-step handlers attach to the step they follow:

```whip
tell worker as turn timeout 10m "Do the work."
on timeout { fail error { reason "worker timed out" } }
on fails   { askHuman "Worker failed — retry?" }
```

Terminal actions (`complete`, `fail`) end the flow exactly as they end a
rule.

**Branch liveness.** When a flow reaches a terminal on any path (it contains a
`complete`/`fail`), every branch must reach one too, or that path stalls with the
workflow stuck. `whip check` emits a warning when a branch leaves no terminal path:
an `on fails`/`on timeout` handler or a `when`/`else` arm (including a missing
`else`) that neither reaches a terminal nor records a fact a workflow rule completes
from, or an effect that sets a `timeout` but has no `on timeout` handler. Resolve it
by reaching a terminal on the branch (or handing off a fact), or by dropping the
`timeout`.

**Unhandled-failure auto-fail.** Branch liveness is only a warning. At runtime, if
a step in such a self-terminating flow *does* fail with no `on fails` handler, the
workflow does not stall forever — it auto-fails: the instance reaches `failed` with
a generic reason (`unhandled failure of <step> …`) and no typed `failure` payload.
Add an `on fails { fail error { ... } }` handler when you want a typed failure or
custom recovery; otherwise the auto-fail is the safety net that guarantees the flow
always terminates.

### How flows lower

The lowering is fully visible: a flow named `triage` compiles to ordinary
rules named `flow.triage.seg0`, `flow.triage.seg1`, and so on — one segment
per step boundary — plus a reserved `FlowAwait_*` state class that carries
the in-flight bindings between segments. They appear in `whip check` output
like any other rule, so you can audit exactly how a flow sequences its
effects. Nothing about flows is magic; they are a convenience surface over
the rule and fact model.

A pre-ask step's output (for example a `tell` result) that a later segment
reads is carried across the `askHuman` boundary through that `FlowAwait_*`
state class, so `complete result { plan turn.summary }` after the answer reads
the earlier turn even though it ran in a prior segment. Only bindings a later
segment actually references are carried.

The [flow design record](https://github.com/jamesjscully/whipplescript/blob/main/spec/flow.md) documents the lowering in full.

## Work queues

A work queue is vendor-neutral, durable issue tracking declared in source.
Use it when work arrives as a backlog of items to be claimed, worked, and
finished, rather than as facts seeded up front:

```whip
queue backlog {
  tracker builtin
}
```

The `builtin` tracker is workspace-scoped: it stores items in
`.whipplescript/items.sqlite` (override with `WHIPPLESCRIPT_ITEMS_STORE`) and
issues sequential ids `WS-1`, `WS-2`, and so on. Items have a status in one of
four categories — `open`, `in_progress`, `done`, `cancelled`.

Rule and flow bodies act on the queue with these verbs:

```whip
file item into backlog { title "Fix login" body "Users report 500s." }
claim item as work
release work
finish work { summary "patched and verified" }
```

React to ready work with the readiness pattern:

```whip
rule pick_up
  when backlog has ready item as item
  when worker is available
=> {
  claim item as work
  tell worker as turn "Resolve {{ work.title }}."
}
```

`claim` can fail: when another claimant already holds the item, the claim
effect fails normally, and you branch on it like any other failure
(`after work fails as f { ... }`) rather than treating it as an error.

A claimed item is held by the claiming instance for its lifetime: `release` or
`finish` it explicitly, or — if the instance reaches a terminal first,
including an operator `whip cancel` — the runtime returns the still-held item to
`open` so another worker can pick it up. A dead claimant never strands its work.

Operators and agents manage items from the CLI:

```sh
whip items add --queue backlog --title "Fix login" [--body "..."] [--label bug]
whip items list [--queue backlog] [--status open]
whip items show WS-1
```

When an agent files an item mid-turn through the CLI, the runtime stamps it
with run-identity provenance from the `WHIPPLESCRIPT_RUN_ID` environment
variable, so backlog growth is traceable to the run that caused it.

The [work-queues design record](https://github.com/jamesjscully/whipplescript/blob/main/spec/work-queues.md) covers the model in
detail.

## Time and deadlines

Time enters a workflow as effects, never as ambient clock reads in guards.

A `timeout` clause bounds any effect. If the effect has not completed when the
duration elapses, it ends in the `timed_out` status and an
`after ... times out` / `on timeout` branch can react:

```whip
tell worker as turn timeout 10m "Do the work."
```

A `timer` is a standalone effect that completes when its duration is due —
useful for delays, polling intervals, and deadlines that are not tied to
another effect:

```whip
timer 24h as deadline

after deadline succeeds {
  askHuman "24h elapsed — still waiting on review."
}
```

`cancel <binding>` cancels a pending or running effect bound earlier in the
body — for example, cancelling a worker turn once a human rejects the plan.

Durations are written `<n><unit>` with units `s`, `m`, `h`, `d` (for example
`30s`, `10m`, `24h`, `7d`).

Timers and timeouts fire on worker passes; there is no background daemon.
`whip dev --until idle` treats pending timers as idle (it does not block
waiting for wall-clock time), and `whip status` lists pending time effects so
you can see what a paused instance is waiting on. The
[time design record](https://github.com/jamesjscully/whipplescript/blob/main/spec/time.md) documents the semantics.

## `exec`

`exec` has two profiles.

In the dev profile, `exec` runs a local command string as a gated effect:

```whip
exec "scripts/run-tests.sh" as tests

after tests succeeds as result {
  record TestRun { passed result.exit_code == 0 }
}
```

The output binding exposes `tests.exit_code` and `tests.stdout`.

Dev-profile `exec` is gated by operator configuration only — there is no
source syntax that grants it. The worker reads `WHIPPLESCRIPT_EXEC_ALLOW`, a
colon-separated list of glob prefixes (for example `scripts/*:bin/ci-*`). A
command that does not match a grant fails and routes to `after x fails`.
There is no sandbox in the dev profile — a grant is a documented trust
decision the operator makes deliberately, so keep the allow-list as narrow as
the workflow needs.

In the hosted profile, raw command strings are rejected. Source names an
operator-pinned script capability and passes typed input on stdin:

```whip
exec backup_repo with request -> Report as backup

after backup succeeds as report {
  record BackupFinished { summary report.summary }
}
```

The operator supplies a JSON manifest outside the workspace:

```json
{
  "backup_repo": {
    "argv": ["bash", "scripts/backup.sh"],
    "sha256": "9f2c...",
    "env": { "BACKUP_TOKEN": "env:BACKUP_TOKEN" }
  }
}
```

Run hosted checks/workers with `--exec-profile hosted --script-manifest
<path>` or the equivalent `WHIPPLESCRIPT_EXEC_PROFILE=hosted` and
`WHIPPLESCRIPT_SCRIPT_MANIFEST=<path>`. The worker registers
`script.<name>`, verifies the script bytes against `sha256`, stages a
verified copy, and spawns argv-direct. No source text is interpolated into a
shell. A hash mismatch fails before spawn and records the expected/actual
hash in failure evidence.

## Files

A `file store` declares a named, root-scoped directory a workflow may read from
and write to. It is a policy boundary, not an open filesystem handle:

```whip
file store project_files {
  root "./data"
  allow read ["docs/**", "notes/*.md"]
  allow write ["out/**"]
}
```

The optional `allow read`/`allow write` globs narrow which paths (relative to
`root`) each operation may touch; an absent list means any path inside the root.

`import <format> <Schema> from <store> at <path> as <binding>` decodes a
structured file (`jsonl`, `json`, or `csv`) into one typed `<Schema>` fact per
row, which `when <Schema>` rules then react to. Each row is validated against the
schema's required fields and the whole batch is admitted atomically — any invalid
row fails the import and admits nothing. If the schema marks a field `@key`
(e.g. `id string @key`), that field's value identifies each row (so a re-run is
idempotent on it); otherwise rows are keyed by position.

`export <format> <Schema> to <store> at <path> { where <pred> mode <mode> } as
<binding>` is the inverse: it serializes the `<Schema>` facts (the `where` filter
is optional) to a `jsonl`/`json`/`csv` file, in a deterministic order, with the
same `mode` policy as `write`. The `where`-filtered set is a *collection-valued
projection* — the same fact-matching guards use, yielding a collection rather than
a count.

`read` loads one file into a typed binding as a gated effect:

```whip
read text from project_files at "notes.md" as fileResult

after fileResult succeeds as result {
  record Loaded { body result.content }
}
```

The success binding exposes `result.content` (the file body) and `result.bytes`.
A missing file or a refused path routes to `after fileResult fails`.

`write` renders a body to a file. It requires an explicit `mode` — no silent
overwrite:

```whip
write text to project_files at "summary.md" {
  body result.content
  mode create
} as written

after written succeeds as w {
  record Saved { path "summary.md" }
}
```

Modes: `create` (fail if the file exists), `replace` (fail if it does not),
`upsert` (either), `append`. A mode violation (e.g. `create` on an existing file)
is an ordinary failure routed to `after written fails`, leaving the file
untouched. The `body` is an expression resolved when the effect runs, so a write
inside `after <read> succeeds as r { … }` can write `r.content`.

`read` and `write` are effects — they never run in guards or during static
checking. The store's `root` is the scope boundary: the path is taken relative to
`root`, a path that is absolute or uses `..` to climb out of the root is refused
before any disk access, and (when declared) the path must match the store's
`allow read`/`allow write` globs — a denied path fails the operation rather than
touching files outside the policy.

In a `whip test` scenario, seed deterministic file content with `given file
<store> at "<path>" "<content>"` — the harness writes it to a temp dir and
redirects the store root there, so the `read` runs for real against the fixture.

Current limitations (see the `files` spec for the full
package design and roadmap): `read`/`write` handle only the `text` and `markdown`
body codecs (structured `json`/`csv`/`jsonl` are the `import`/`export` surface and
`bytes` is deferred — the parser rejects them), and `import`/`export` and the
`files.read`/`files.write` capability grants are not yet implemented.

## Channels (`std.messaging`)

A `channel` declares a named communication route through a provider — the
boundary for talking through communication platforms (Slack, email, and the
like). It is package-owned: the bare `channel` construct shape is reserved for
`std.messaging`, so third-party packages cannot create channel-like semantics
with weaker guarantees.

```whip
use std.messaging

channel release_room {
  provider slack
  workspace ops
  destination "#release"
}
```

`provider` is required; `workspace` and `destination` are optional provider
config. Secrets and credentials are always references in provider config, never
literal source values. Declaring a channel auto-registers `std.messaging` in the
program's library contract (you do not have to write `use std.messaging`
separately, though it is accepted as a dotted package name).

Send an outbound message with `send via <channel> { ... } as <binding>`:

```whip
rule notify
  when Ticket as ticket where ticket.status == "open"
=> {
  send via release_room {
    text "Ticket {{ ticket.id }} needs triage."
  } as sent

  after sent succeeds {
    complete result { ok "notified" }
  }
}
```

`text` is required; `markdown` and `thread_id` are optional. `send` lowers to a
`messaging.send` capability call. Because `std.messaging` is a standard library
built into the compiler, `send` needs **no** package lock (third-party
constructs still require `whip package sync`). The named channel must be
declared — `send via <unknown>` is a compile error. Under the fixture provider
`send` records a delivery receipt without contacting a real platform; live
Slack/email delivery is provider-configured.

The generic inbound envelope is the built-in `Message` schema (`message_id`,
`channel`, `provider`, `received_at`, `sender`, `sender_claims`, `thread_id`,
`text`, `markdown`, `attachments`, `interaction`, `raw_ref`, `correlation`).
Inbound messaging always produces a generic `Message`, never a domain type —
converting one into a typed fact is explicit (`coerce msg.text -> Decision`, or
a `signal` mapping). React to one with the readiness form:

```whip
rule react
  when message from release_room as msg
=> {
  complete result { note msg.text }
}
```

The channel must be declared (`when message from <unknown>` is a compile error),
and `msg` is typed as `Message`. Under the fixture provider, inject a message
with `whip message <instance> --channel <name> --text "…" --program <file>` so
the rule fires; live Slack/email ingestion is provider-configured. (The
`source interaction` provider mapping still needs a live messaging provider and
remains on the roadmap — see the `messaging` spec.)

## `assert`

Assertions are executable claims about a finished run, evaluated by
`whip dev` (and `whip accept`) after the instance goes idle:

```whip
assert count(Ticket where status == "open") == 0
assert count(IncidentResolution where status == "resolved") >= 1
assert count(effect kind agent.tell where status == completed) == 3
```

Assertions read facts and effect projections with the same expression
language as guards. They never change execution. Tag them to select subsets
per run (`whip dev --include-tag acceptance`).

## Static checks

`whip check` and `whip compile` validate the bundle before anything runs:
types, field paths (including `{{ ... }}` template references inside
prompts), contract payloads, effect targets, binding names, and the liveness
rules below. Unknown names get did-you-mean suggestions.

### Liveness checks

- **Every workflow must be able to end.** At least one rule must reach
  `complete` or `fail`. Tag the workflow `@service` when running forever is
  intended (watchers, recurring harnesses).
- **Every rule must be able to fire.** Each matched class must be produced
  somewhere — a table, another rule, or a workflow input. Tag the rule
  `@external` when its facts arrive from outside the workflow (packages,
  fixtures, external systems).

```whip
@service
workflow RecurringTriage

@external
rule import_ticket
  when ExternalTicket as ticket
=> { ... }
```

### Not Gherkin

`when` clauses are typed readiness patterns, not free-text steps. Pasting
Cucumber/Gherkin `Feature` / `Scenario` / `Given` / `When` / `Then` text into
a `.whip` file produces a targeted diagnostic pointing back to `workflow`,
`table`, `rule`, and `assert`.

## Semantics notes

### Workflow invocation

`invoke` starts a child instance with its own event log and lifecycle. The
parent sees only the child's declared output or failure payload:

```whip
invoke ReviewPhase {
  phase PhaseReviewRequest {
    id phase.id
  }
} as review

after review succeeds as result {
  record ParentReviewComplete { phaseId phase.id result result }
}

after review fails as failure {
  record ParentReviewBlocked { reason failure.reason }
}
```

A provider failure inside the child does not propagate; the parent's `fails`
branch runs only when the child workflow itself executes `fail`.

An invocation may also carry a resource-specific start grant:

```whip
invoke ReviewDocs {
  task task
}
  with access to project_files {
    read ["docs/**"]
  }
  as review
```

The child has its own workflow principal and per-instance effective-authority
slot. Local children use `workflow:local/<name>`; package-exported `@tool`
children invoked through the owned harness use the exporting manifest's package
id. Under a governed envelope, an imported package tool also opens the membrane
door `invoke:<package_id>/<tool>`; the consumer envelope must govern that door
before the tool is accepted or offered to the model. Rule-issued `invoke` starts use the delegating start seam. The default
authority is the child's declared authority narrowed by the parent's effective
authority; a `with access to` clause narrows that cap further. The runtime
rejects a grant that would widen authority beyond either side. The shipped
grammar accepts both `with access to <resource> { ... }` and the grouped
shorthand `with access to { <resource> { ... } ... }` on `invoke`; the grouped
form desugars to the same resource-specific grant metadata before runtime
admission.

### Provider failure vs. workflow failure

A failed provider run is effect/run state plus events and evidence — the
instance stays `running` until a rule reacts or an operator intervenes.
`fail` is the workflow's own decision to end unsuccessfully. Keeping these
distinct is deliberate: which provider failures are fatal is policy, and
policy lives in rules.

### Revision

Changing a running instance's program is a control-plane operation
(`whip revise`), not source syntax. Workflows can *propose* changes with
ordinary effects — `tell` an agent to draft a candidate, `coerce` a review,
`askHuman` for approval — but activation happens outside source. See
[runtime & operations](runtime-operations.md#revising-a-running-instance).

### Prompt content types

`tell`, `askHuman`, and `coerce` prompts may annotate their opening delimiter
with a content type — `"""markdown`, `"""json`, or a MIME value like
`"""application/json`. The annotation is preserved as `prompt_content_type`
in the compiled effect input for reporting; it does not validate the body or
change provider behavior. Put prompt text on the line after the opener.

### Advanced: named harnesses

Binding `provider` on an agent is the normal path and covers almost every
workflow. For the rare case where one provider family needs several distinct
configured endpoints, declare named harnesses and bind an agent to one:

```whip
harness coder: codex
harness reviewer: claude

agent worker using coder { ... }
```

Supported harness kinds: `codex`, `claude`, `pi`, `fixture`,
`native-fixture`, `command`. Reach for this only when a plain `provider`
binding genuinely cannot express the endpoint topology you need.

## Typed data and coordination

### Sum types

An `enum` variant may carry a typed payload (brace body, class field
grammar). The discriminant is synthesized from the variant name and lands in
JSON as a reserved `variant` field (`{"variant": "Approved", "score": 0.9}`);
declaring a field named `variant` is a check error. Each data variant lowers
to a visible `<Enum>.<Variant>` class. `case` over a sum value dispatches on
the variant and binds the payload with `as`; coverage must be exhaustive (or
carry `_`). Fixture runs return the first declared variant; `--variant
<name>` selects another arm.

```whip
enum ReviewOutcome {
  Approved { score float }
  Rejected { reason string }
  Blocked
}

case outcome {
  Approved as a => { complete result { note "{{ a.score }}" } }
  Rejected as r => { fail error { reason r.reason } }
  Blocked => { fail error { reason "blocked" } }
}
```

### JSON ingestion on `exec`

`exec "cmd" -> Report as x` and hosted `exec capability with input -> Report
as x` make success mean exit 0 AND stdout parses as `Report`; `after x
succeeds as r` binds the typed value, and any parse or schema failure routes
to `after x fails`. `-> each WorkItem` parses a JSONL stream or top-level
array and records one typed fact per element (provenance `ingest`),
all-or-nothing — a malformed line fails the whole effect. Rules react with
ordinary `when WorkItem as item` fan-out.

### Deterministic validation

`exec "<validator>" -> Schema` is the deterministic counterpart to `coerce`.
Where `coerce` asks a model to judge an artifact, a deterministic validator is
any non-LLM checker — Unicode script detection, a regex/format pass, a schema
linter — whose output is reproducible: the same input always yields the same
typed result. The workflow runs the checker, ingests its JSON verdict against a
declared `class`, and branches on it exactly like any other typed effect. This
is the path the e2e plan reserves for "exact script/fixture properties" that
should hold in CI without provider access; model-judged `coerce` review and
deterministic validation are meant to run side by side.

```whip
exec "validate-script {{artifact.path}} {{artifact.expectedScript}}" -> ScriptCheck as check

after check succeeds as result {
  complete report { detail result.detail }
}

after check fails as failure {
  fail error { reason failure.message }   # exec failures expose `.message`
}
```

The validator binary is supplied by the operator and granted through
`WHIPPLESCRIPT_EXEC_ALLOW` like any dev-profile `exec`; an ungranted or
malformed validator is a typed effect failure routed to `after check fails`,
never a silent pass. See `examples/deterministic-validation.whip` for a runnable
end-to-end workflow. Note that an `exec` failure binding carries the reason at
`failure.message` (not `failure.reason`, which is the field workflow-invocation
and `coerce` failures expose).

### Scheduled time

`time` is a scalar (ISO-8601 instants, quoted literals). `timer until
<literal-or-time-typed-path> as deadline` fires on the first worker pass at
or after the target; `after deadline succeeds` reacts to the recorded firing.
There is no `now` in guards — the clock is read only at the worker boundary.

### Signals

`signal deploy.finished { service string  status string }` declares typed
external ingress. React with the bare form `when deploy.finished as d`
(typed, no `@external` needed). Inject from outside with `whip signal
<instance> --name deploy.finished --data '{"service":"api","status":"ok"}'
--program <workflow.whip>` — the payload is validated at the boundary. Inject
from inside another workflow with the signal injection effect:

```whip
emit signal deploy.finished to s.target {
  service s.service
  status "ok"
} as sent
```

The target (`s.target`) must be the id of an instance that already exists in
the same store; otherwise the effect fails with `target instance <id> not
found` and routes to `after sent fails`. Because instance ids are generated at
`run` time, a real peer id is normally carried on a fact (e.g. a `PeerInstance`
recorded when the peer registers), not a literal — `examples/event-bridge.whip`
uses a placeholder `peers` table id purely so the file type-checks, so it needs
a real peer to run. A minimal two-instance exercise: `whip run` the peer
workflow and note its id, point the source's peer fact/table at that id (same
`--store`), `whip signal` the source, then `whip step` + `whip worker` the
source — the signal lands as a `deploy.finished` fact on the peer and the source
records its `DeploymentNotice`.

### Coordination resources

A closed family of workflow-scoped coordination resources, each declared with a
typed key and mandatory bounds, mutated only by atomic branchable effects:

```whip
lease deploy_slot { key Environment  slots 1  ttl 10m }
ledger decisions  { entry Decision  partition by area  retain 90d }
counter budget    { key Customer  cap 1000  reset daily }

lease global_deploy_slot { shared  key Environment  slots 1  ttl 10m }

acquire deploy_slot for r.env as slot     # after slot held / after slot contended
release slot                              # or `acquire ... until ttl` (fire-and-forget)
append Decision { area d.area } to decisions as entry
consume budget for t.customer amount t.estTokens as spend   # after spend ok / over
```

The compiler enforces the safety model: at most one held lease per
progression, exhaustive outcome handling (`held`/`contended`, `ok`/`over`),
and must-release on every non-terminal path. Reaching any terminal
auto-releases every lease the instance holds — a rule-driven `complete`/`fail`
or an operator `whip cancel` — so holder lifetime bounds every lease, with TTL
only as the crash net. Counter reset is lazy at the consume boundary. Inspect
shared state with `whip leases`, `whip ledger`, and `whip counters`.

By default, coordination rows are partitioned by workflow owner, so two
workflows using the same source resource name do not contend or communicate
through outcome bits. Adding the bare `shared` field opts the resource into the
shared owner. Under `shared`, branchable outcomes are cross-principal
information-flow read sources and mutations are governed sinks; resources used
by only one workflow principal remain unlabeled self-coordination.

### Observability export

`whip otel-export <instance>` is a cursor-tracked sidecar that tails the
durable log and emits OTLP/HTTP JSON traces — spans named after source
constructs, structural attributes only, emit-once across re-runs. It honors
`OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME`; point it at a local
OpenTelemetry Collector, which owns TLS and backend fan-out. `--dry-run`
prints the payload.

## What WhippleScript is not

Not a general-purpose language: keep data manipulation small and
deterministic, and push computation into providers, coerce functions, packages,
or child workflows behind explicit effects.

Not an implicit lifecycle framework: recurring work, heartbeats, memory,
review, and escalation are ordinary facts, effects, and rules — never hidden
control-flow modes.
