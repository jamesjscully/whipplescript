# Language reference

The complete reference for `.whip` source. For a guided introduction, start
with the [tutorial](tutorial.md); for authoring guidance, see the
[manual](manual.md).

## Program structure

A program is a root file plus its `include` closure. A file contains
declarations: at most one workflow header (or any number of brace-wrapped
workflows), classes, enums, agents, coerce functions, tables, rules,
assertions, patterns, and imports.

```whip
include "shared/review.whip"
include "review.baml"

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
`invoke`.

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

Field types follow the BAML-compatible subset: scalars (`string`, `int`,
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

A typed, BAML-backed model decision. Calling it in a rule creates a durable
`baml.coerce` effect:

```whip
coerce reviewPoem(language string, summary string) -> PoemReview {
  prompt """markdown
  Review the artifact.

  Language: {{ language }}
  Summary: {{ summary }}

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

### `include` and `use`

```whip
include "schemas/common.whip"   // contributes declarations to the bundle
include "review.baml"           // makes BAML classes/functions available to coerce
use memory                      // imports a plugin by name
```

Plugins register capabilities, providers, schemas, and optional skills; their
capabilities are invoked as explicit `call` effects.

### Tags and descriptions

Source metadata on workflows, tables, rules, and assertions:

```whip
@acceptance
description "Provider x language acceptance workflow"
workflow ProviderLanguageE2E
```

Both are preserved in compiled IR for reports. `whip dev --include-tag` /
`--exclude-tag` filter which assertions are evaluated. Two tags carry static
meaning for the [liveness checks](#liveness-checks): `@service` and
`@external`. No tag changes runtime behavior — readiness, routing, and
effects are unaffected.

## Rules

A rule waits for facts and events, optionally filters them with guards, and
commits a rewrite atomically — either every fact, effect, dependency, and
terminal action in the selected rule persists, or none do.

```whip
rule write_and_review_poem
  when PoemTask as task where task.status == "queued"
  when task.poet is available
=> {
  tell task.poet as turn """markdown
  Write a poem in {{ task.language }}.
  """

  after turn succeeds as completed {
    coerce reviewPoem(task.language, completed.summary) as review
  }

  after review succeeds as checked {
    done task -> record ReviewedPoem from task {
      provider poet
      review checked
      status "reviewed"
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

Prefer the word operators in new code:

```whip
when Job as job where job.status == "pending" and job.attempts < 3
```

Guards never perform I/O — no provider queries, BAML calls, file reads,
clocks, or randomness. A decision that needs model judgment or external data
becomes an effect, and a later rule branches on its completion.

### Rule body operations

| Operation | Meaning |
| --- | --- |
| `record Class { ... }` | Create a typed fact. |
| `record Class from binding { ... }` | Create a fact by copying a binding's fields and overriding the listed ones. |
| `done binding` | Consume a matched fact. |
| `done binding -> record ...` | Consume and replace in one atomic commit. |
| `tell agent [requires [...]] [as x] [timeout <dur>] "..."` | Enqueue an `agent.tell` effect. |
| `coerce fn(...) as x` | Enqueue a typed `baml.coerce` effect. |
| `decide "..." -> { ... } as x` | Enqueue an inline typed model decision (see [Inline `decide`](#inline-decide)). |
| `askHuman [as x] [choices [...]] "..."` | Enqueue a human review request. |
| `file item into <queue> { ... }` | File a new item into a [work queue](#work-queues). |
| `claim <item> [as x]` | Claim a queue item; already-claimed is a branchable failure. |
| `release <item>` | Return a claimed item to the queue. |
| `finish <item> [{ summary ... }]` | Mark a queue item done. |
| `timer <dur> as x` | Create a [timer effect](#time-and-deadlines) that fires when due. |
| `cancel <binding>` | Cancel a pending or running effect bound earlier. |
| `exec "<command>" as x` | Enqueue a dev-profile gated command effect (see [`exec`](#exec)). |
| `exec <capability> with <record> -> Type as x` | Enqueue a hosted script capability effect with typed stdin/stdout. |
| `call plugin.capability ... [as x]` | Enqueue a plugin capability effect. |
| `invoke Workflow { ... } as x` | Start a durable child workflow. |
| `after x succeeds as y { ... }` | Run when effect `x` completes successfully. |
| `after x fails as y { ... }` | Run when effect `x` fails. |
| `after x completes { ... }` | Run on any terminal status of `x`. |
| `case value { Pattern => { ... } }` | Branch over a finite-domain or union value. |
| `complete output { ... }` | Emit the declared workflow output; the instance completes. |
| `fail failure { ... }` | Emit the declared failure payload; the instance fails. |

`consume binding` is a deprecated alias for `done binding`; it compiles with
a warning and will be removed — prefer `done`.

The `emit` action has been removed from the language; using it is now a check
error. Workflows append durable events through ordinary effects and the facts
their completions derive.

Binding names introduced with `as` must not shadow operation keywords —
`done`, `record`, `tell`, `complete`, `fail`, and the rest are rejected as
binding names.

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

### `case`

Deterministic branching over an enum field or an effect's terminal union:

```whip
after turn completes {
  case turn {
    Completed completed => {
      record TurnReport { branch "completed" summary completed.summary }
    }
    Failed failure => {
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

The scrutinee must have a finite-domain type: an enum, a terminal union, or a
string-literal union. `case` over a plain `string` is rejected — use guarded
rules instead. See
[`examples/terminal-output-union.whip`](../examples/terminal-output-union.whip)
for exhaustive terminal handling.

### Inline `decide`

`decide` is an anonymous, typed model decision inside a rule body — a coerce
with its schema written in place, for the case where a named `coerce`
declaration would only be used once:

```whip
decide "Is this plan safe to ship? Explain." -> { fixed bool, reason string } as verdict

after verdict succeeds as v {
  case v.fixed {
    true  => { complete result { decision "ship" } }
    false => { askHuman "Held: {{ v.reason }}" }
  }
}
```

It lowers to the same `baml.coerce` effect as a named `coerce`, so the same
rules apply: it is durable, it can fail, and its typed output is available
only in an `after ... succeeds` (or `after verdict succeeds`) branch. Use a
named `coerce` when the decision is reused or deserves a documented prompt;
reach for `decide` for a local, single-use judgment.

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

### How flows lower

The lowering is fully visible: a flow named `triage` compiles to ordinary
rules named `flow.triage.seg0`, `flow.triage.seg1`, and so on — one segment
per step boundary — plus a reserved `FlowAwait_*` state class that carries
the in-flight bindings between segments. They appear in `whip check` output
like any other rule, so you can audit exactly how a flow sequences its
effects. Nothing about flows is magic; they are a convenience surface over
the rule and fact model.

The [flow design record](../spec/flow.md) documents the lowering in full.

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

Operators and agents manage items from the CLI:

```sh
whip items add --queue backlog --title "Fix login" [--body "..."] [--label bug]
whip items list [--queue backlog] [--status open]
whip items show WS-1
```

When an agent files an item mid-turn through the CLI, the runtime stamps it
with run-identity provenance from the `WHIPPLESCRIPT_RUN_ID` environment
variable, so backlog growth is traceable to the run that caused it.

The [work-queues design record](../spec/work-queues.md) covers the model in
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
[time design record](../spec/time.md) documents the semantics.

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

## `assert`

Assertions are executable claims about a finished run, evaluated by
`whip dev` (and `whip accept`) after the instance goes idle:

```whip
assert count(Ticket where status == "open") == 0
assert count(ReviewedPoem where review.confidence >= 0.5) == 3
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
  `@external` when its facts arrive from outside the workflow (plugins,
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

## Typed data and coordination (Part C)

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

### Scheduled time

`time` is a scalar (ISO-8601 instants, quoted literals). `timer until
<literal-or-time-typed-path> as deadline` fires on the first worker pass at
or after the target; `after deadline succeeds` reacts to the recorded firing.
There is no `now` in guards — the clock is read only at the worker boundary.

### Events

`event deploy.finished { service string  status string }` declares typed
external ingress. React with the bare form `when deploy.finished as d`
(typed, no `@external` needed). Inject from outside with `whip notify
<instance> --event deploy.finished --data '{"service":"api","status":"ok"}'
--program <workflow.whip>` — the payload is validated at the boundary. Inject
from inside another workflow with the `notify` effect:

```whip
notify s.target event deploy.finished {
  service s.service
  status "ok"
} as sent
```

### Coordination resources

A closed family of workspace-scoped shared resources, each declared with a
typed key and mandatory bounds, mutated only by atomic branchable effects:

```whip
lease deploy_slot { key Environment  slots 1  ttl 10m }
ledger decisions  { entry Decision  partition by area  retain 90d }
counter budget    { key Customer  cap 1000  reset daily }

acquire deploy_slot for r.env as slot     # after slot held / after slot contended
release slot                              # or `acquire ... until ttl` (fire-and-forget)
append Decision { area d.area } to decisions as entry
consume budget for t.customer amount t.estTokens as spend   # after spend ok / over
```

The compiler enforces the safety model: at most one held lease per
progression, exhaustive outcome handling (`held`/`contended`, `ok`/`over`),
and must-release on every non-terminal path (reaching `complete`/`fail`
auto-releases — holder lifetime bounds every lease, with TTL as the crash
net). Counter reset is lazy at the consume boundary. Inspect shared state
with `whip leases`, `whip ledger`, and `whip counters`.

### Observability export

`whip otel-export <instance>` is a cursor-tracked sidecar that tails the
durable log and emits OTLP/HTTP JSON traces — spans named after source
constructs, structural attributes only, emit-once across re-runs. It honors
`OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME`; point it at a local
OpenTelemetry Collector, which owns TLS and backend fan-out. `--dry-run`
prints the payload.

## What WhippleScript is not

Not a general-purpose language: keep data manipulation small and
deterministic, and push computation into providers, BAML functions, plugins,
or child workflows behind explicit effects.

Not an implicit lifecycle framework: recurring work, heartbeats, memory,
review, and escalation are ordinary facts, effects, and rules — never hidden
control-flow modes.
