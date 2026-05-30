# WhippleScript Language Reference

Status: in progress

This reference describes the canonical `.whip` authoring model. It is meant to
be read by workflow authors before they need the detailed specs.

## Program Shape

A source bundle is a root file plus its `include` closure. A bundle may contain
schemas, agents, coerces, patterns, plugins, and one or more workflows.

```whip
include "shared/review.whip"
include "review.baml"

use memory

workflow Example {
  input request WorkRequest
  output result WorkResult
  failure error WorkFailure

  agent worker {
    profile "repo-writer"
    capacity 1
    skills ["whipplescript-author"]
  }

  rule start
    when WorkRequest as request
    when worker is available
  => {
    tell worker as turn """
    Do the work:
    {{ request.title }}
    """

    after turn succeeds as completed => {
      complete result {
        id request.id
        summary completed.summary
      }
    }
  }
}
```

If a bundle exposes multiple workflows, commands must select the root workflow.
Library workflows remain invokable by name from the same source bundle.

## Declarations

### `workflow`

`workflow` is the runtime boundary. Starting a workflow creates a durable
instance with its own event log, fact projection, effects, runs, leases,
evidence, and terminal state.

```whip
workflow ReviewPhase {
  input phase PhaseReviewRequest
  output result PhaseReviewResult
  failure error ReviewPhaseFailure
  ...
}
```

Workflow input payloads are keyed by the declared binding name. For
`input phase PhaseReviewRequest`, the start payload is shaped like:

```json
{"phase": {"id": "phase-1", "title": "Parser review"}}
```

`complete result { ... }` and `fail error { ... }` are the only source-level
ways for rules to produce declared workflow outputs. Cancellation is a control
plane action, not a rule body operation.

### `class` And `enum`

`class` declares typed fact and boundary payload shapes.

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

Supported boundary types follow the BAML-compatible subset used by the type
system: scalar values, arrays, objects/classes, enums, optionals where
supported, and finite domains such as `AgentRef<codex | claude | pi>`.

### `agent`

`agent` declares an addressable provider target and policy profile.

```whip
agent codex {
  profile "repo-writer"
  capacity 2
  capabilities ["agent.tell"]
  skills ["whipplescript-author", "loft-user"]
}
```

Skills are context bundles assigned to agents or turns. They are not plugins and
they do not extend the language grammar.

### `coerce`

`coerce` declares a typed BAML-backed model decision. Calling it in a rule
creates a durable `baml.coerce` effect.

```whip
coerce reviewPoem(language string, artifactPath string, summary string) -> PoemReview {
  prompt """
  Review the artifact.

  Language: {{ language }}
  Artifact: {{ artifactPath }}
  Summary: {{ summary }}

  {{ ctx.output_format }}
  """
}
```

`coerce` is effectful. It is not a pure local function call. Use `after review
succeeds` to access the typed output.

### `pattern`

`pattern` is compile-time reuse. Applying a pattern expands it into ordinary
declarations before type checking/lowering.

```whip
pattern AgentReview<Input, Output> {
  input Input as item

  rule dispatch
    when Input as item
    when reviewer is available
  => {
    tell reviewer as turn "Review {{ item.title }}."

    after turn succeeds as reviewed => {
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

Patterns are not runtime instances. Use `invoke` when work must run as a child
workflow with its own lifecycle.

### `include`

`include` composes source files.

```whip
include "schemas/common.whip"
include "review.baml"
```

`.whip` includes contribute declarations to the source bundle. `.baml` includes
make BAML classes/functions available to coercion lowering without treating BAML
as a WhippleScript skill.

### `use`

`use` imports a plugin by name.

```whip
use memory
```

Plugins may register capabilities, providers, schemas, resources, prompt
templates, and optional skills. They do not introduce hidden control-flow
semantics.

## Rules

Rules match facts/events and commit deterministic rewrites.

```whip
rule write_and_review_poem
  when PoemTask as task where task.status == "queued"
  when task.poet is available
=> {
  tell task.poet as turn """
  Write a poem in {{ task.language }}.
  """

  after turn succeeds as completed => {
    coerce reviewPoem(task.language, task.artifactPath, completed.summary) as review
  }

  after review succeeds as checked => {
    done task -> record ReviewedPoem from task {
      provider poet
      review checked
      status "reviewed"
    }
  }
}
```

A rule commit is atomic. Either all facts/effects/dependencies/terminal actions
from the selected rule are persisted, or none are.

## Rule Body Operations

| Operation | Meaning |
| --- | --- |
| `record Class { ... }` | Create a typed fact. |
| `record Class from binding { ... }` | Create a fact by copying fields from a binding and overriding listed fields. |
| `consume binding` | Mark a matched fact consumed. |
| `done binding` | Alias for `consume binding`. |
| `done binding -> record ...` | Consume a fact and create a replacement/result fact in one commit. |
| `tell agent ...` | Enqueue an `agent.tell` effect. |
| `coerce fn(...) as x` | Enqueue a typed `baml.coerce` effect. |
| `claim issue with loft` | Enqueue a Loft claim effect. |
| `askHuman ...` | Enqueue a human review request. |
| `call plugin.capability ...` | Enqueue a namespaced capability effect. |
| `emit event.name` | Enqueue or append a durable event emission. |
| `invoke Workflow { ... } as x` | Start or request a durable child workflow. |
| `after effect succeeds` | Scope that runs after a successful effect completion. |
| `after effect fails` | Scope that runs after a failed effect completion. |
| `after effect completes` | Scope that runs after any terminal completion branch. |
| `complete output { ... }` | Validate and emit successful workflow terminal output. |
| `fail failure { ... }` | Validate and emit workflow failure output. |

Source order does not imply effect order. Use `after` to create durable
dependency edges and scoped effect outputs.

## Guards And Expressions

`when ... where <expr>` is pure deterministic filtering. Guards can inspect
matched facts and pure values only.

Supported expression families include:

```text
field access
equality and inequality
ordering
and / or / not
membership with in
exists / empty / count over finite queries
array and object literals
indexing into present maps/objects
enum and finite-domain literals
```

Guards must not perform I/O, query providers, call BAML, read files, use wall
clock time, or access random sources. If a decision needs model judgment or
external data, represent that as a durable effect and branch on its completion.

## Effects And Completion Scope

Effect outputs are visible only inside the branch that proves the effect
terminal status.

```whip
tell worker as turn "Do the task."

after turn succeeds as completed => {
  record TurnSummary {
    text completed.summary
  }
}
```

Outside that `after` block, `completed` is not in scope. This keeps rule
lowering deterministic and makes event causality explainable.

## Workflow Invocation

Use `invoke` for runtime composition.

```whip
invoke ReviewPhase {
  phase PhaseReviewRequest {
    id phase.id
    title phase.title
  }
} as review

after review succeeds as result => {
  record ParentReviewComplete {
    phaseId phase.id
    result result
  }
}

after review fails as failure => {
  record ParentReviewBlocked {
    phaseId phase.id
    reason failure.reason
  }
}
```

The parent sees declared output/failure payloads, not child-local facts. Child
provider failures remain child effect/run events unless the child workflow
chooses to `fail`.

## Workflow Revision

Workflow revision is a control-plane operation for changing the active program
version of a non-terminal running instance. It is not a source-level rule body
operation, and there is no `.whip` syntax that activates a revision.

Use ordinary workflow effects when a workflow should propose a change:

- `tell` an agent to write a candidate source artifact.
- `coerce` a typed review or classification of the proposal.
- `invoke` a child workflow that prepares or validates a patch.
- `askHuman` for approval before an operator activates the candidate.

The activation path stays outside source:

```sh
whip revise <instance> candidate.whip --root Workflow --dry-run
whip revise <instance> candidate.whip --root Workflow --cancel keep
```

`revise` validates the candidate bundle, records a new revision epoch, and
makes future stepping use the new active program version. Effects that already
exist keep their original `program_version_id` and `revision_epoch`.

Cancellation policy is explicit:

| Policy | Effect |
| --- | --- |
| `--cancel keep` | Keep old-version effects claimable/runnable. |
| `--cancel queued` | Terminal-cancel queued, blocked, and claimable old-version effects. |
| `--cancel running` | Cancel queued effects and request cancellation for running effects. |

Running cancellation requests are not terminal results. A provider must
acknowledge cancellation or complete, fail, or time out through the normal effect
lifecycle.

## Lifecycle

Instances have these durable states:

```text
created
running
paused
blocked
completed
failed
cancelled
```

`paused` prevents new provider starts and rule progress that would create new
effectful work. `completed`, `failed`, and `cancelled` are terminal. Terminal
instances cannot commit further rule effects or user-fact mutations.

Provider failures and workflow failures are different:

- Provider failure: an effect/run terminal event and evidence record.
- Workflow failure: a source rule executes `fail <failure> { ... }`.

This distinction lets workflows inspect, retry, escalate, or ignore provider
failures according to source policy.

## CLI Workflow

Typical local use:

```sh
cargo run -p whipplescript-cli -- check examples/minimal-noop.whip
cargo run -p whipplescript-cli -- compile examples/minimal-noop.whip
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite run examples/minimal-noop.whip --input '{}' --json
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite status <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite trace <instance> --check --json
```

Use `dev` for a local validation loop that composes start, step, and fixture
workers where supported. Use `step` when you want deterministic rule evaluation
without provider execution. Use `worker` to run already-materialized effects.

## What WhippleScript Is Not

WhippleScript is not a general programming language. Keep data manipulation small
and deterministic. Complex computation belongs in providers, BAML functions,
plugins, or external systems with explicit effect boundaries.

WhippleScript is not an implicit lifecycle framework. Recurring work, heartbeat,
memory, review, and escalation are ordinary facts/effects/rules composed by
source code, not built-in control-flow modes.
