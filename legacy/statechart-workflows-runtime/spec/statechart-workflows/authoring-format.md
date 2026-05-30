# Statechart Workflow Authoring Format

Status: design proposal

`.whip` files use a small native statechart DSL. The language borrows BAML's
declaration style for structured data and model coercion, but the workflow
control model is WhippleScript-owned and statechart-first.

The source file defines exactly one machine. The top-level `machine` declaration
names that machine; it does not wrap the file in braces.

## Goals

The source format should let humans and coding agents express:

```text
which machine is running
which durable data exists
which agents and capabilities may be used
which states exist
which events matter
which guards select transitions
which actions and effects occur
which typed model decisions are needed
```

The format must avoid:

- arbitrary imports
- arbitrary host-language functions
- ambient filesystem, network, or process authority
- unbounded loops
- callbacks with unclear lifetime
- hidden mutable state

## Example

```whipplescript
machine implementationLoop
initial running

data {
  seenRuns string[] = []
  lastIdleNudgeAt time? = nil
}

agent director = thread("director")
agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 2
}

agent quality = codingAgent() {
  profile "repo-reader"
  maxActive 1
}

capability plan = adapter("implementationPlan")

enum NextAction {
  StartWorker
  StartQuality
  AskHuman
  Wait
  Done
}

class NextStep {
  action NextAction
  workItemId string?
  reason string
  message string?
}

class RunSummary {
  id string
  name string
  status string
  stdoutTail string
  stderrTail string
  exitCode int?
}

enum RunKind {
  WorkerComplete
  WorkerFailed
  QualityPassed
  QualityFailed
  Irrelevant
}

class RunClassification {
  kind RunKind
  workItemId string?
  reason string
}

coerce chooseNextStep(planText string) -> NextStep {
  model "gpt-4o-mini"

  prompt """
  Choose the next workflow action from the implementation plan.

  {{ planText }}

  Prefer independent worker tasks when available.
  Ask for human help only when blocked or ambiguous.

  {{ ctx.output_format }}
  """
}

coerce classifyRun(run RunSummary) -> RunClassification {
  model "gpt-4o-mini"

  prompt """
  Classify this finished agent run.

  {{ run }}

  Use WorkerComplete when implementation work appears done.
  Use WorkerFailed when the worker failed or became blocked.
  Use QualityPassed when review accepted the work.
  Use QualityFailed when review found a blocking issue.
  Use Irrelevant when this run should not affect the plan.

  {{ ctx.output_format }}
  """
}

state running {
  initial watching

  on finished as run
    guard !(run.id in data.seenRuns)
  {
    case run.name {
      matches "worker-*" -> {
        assign data.seenRuns = data.seenRuns.append(run.id)

        let classification = coerce classifyRun({
          id run.id
          name run.name
          status run.status
          stdoutTail run.stdoutTail
          stderrTail run.stderrTail
          exitCode run.exitCode
        })

        case classification.kind {
          WorkerComplete -> {
            plan.markReadyForQuality(classification.workItemId)

            start quality {
              task classification.workItemId
              message "Review completed worker task."
            }

            stay
          }

          WorkerFailed -> {
            plan.markBlocked(classification.workItemId, classification.reason)

            send director """
            Worker failed: {{ classification.reason }}
            """

            stay
          }

          _ -> {
            stay
          }
        }
      }

      matches "quality-*" -> {
        assign data.seenRuns = data.seenRuns.append(run.id)

        let classification = classifyRun({
          id run.id
          name run.name
          status run.status
          stdoutTail run.stdoutTail
          stderrTail run.stderrTail
          exitCode run.exitCode
        })

        case classification.kind {
          QualityPassed -> {
            plan.markDone(classification.workItemId)
            goto choosing
          }

          QualityFailed -> {
            plan.markBlocked(classification.workItemId, classification.reason)
            askHuman(classification.reason)
            stay
          }

          _ -> {
            stay
          }
        }
      }

      _ -> {
        stay
      }
    }
  }

  state watching {
    on idle
      guard activeRuns() == 0
      guard plan.unfinishedItems() > 0
      guard elapsedSince(data.lastIdleNudgeAt) >= 2m
    {
      assign data.lastIdleNudgeAt = now()
      goto choosing
    }
  }

  state choosing {
    entry {
      let planText = plan.snapshot()
      let next = coerce chooseNextStep(planText)

      case next.action {
        StartWorker -> {
          start worker {
            task next.workItemId
            message next.message
          }

          goto watching
        }

        StartQuality -> {
          start quality {
            task next.workItemId
            message next.message
          }

          goto watching
        }

        AskHuman -> {
          askHuman(next.reason)
          goto watching
        }

        Wait -> {
          send director next.reason
          goto watching
        }

        Done -> {
          goto done
        }
      }
    }
  }

  state done {
    final
  }
}
```

## Core Vocabulary

The source language uses standard statechart terms for control flow:

```text
machine
initial
state
on
guard
entry
always
goto
stay
final
raise
send
assign
```

WhippleScript-specific orchestration terms are reserved for domain effects:

```text
agent
capability
start
askHuman
coerce
snapshot-style adapter calls
```

## Source Unit

A source file defines one machine:

```whipplescript
machine simpleSupervisor
initial watching
```

`initial` names the first active state. Nested compound states may also declare
their own `initial` child state.

The first declared state is not implicitly initial. Reordering source must not
change runtime behavior.

## Durable Data

Workflow-local durable data is declared with `data`:

```whipplescript
data {
  seenRuns string[] = []
  lastIdleNudgeAt time? = nil
}
```

`data` maps to statechart extended state. It is durable, logged, and typed.
Initializers are optional, but when present they must be static literal, list,
or object expressions that match the declared schema. Dynamic expressions such
as `now()` are not valid data initializers in v0.

Durable writes must use `assign`:

```whipplescript
assign data.seenRuns = data.seenRuns.append(run.id)
assign data.lastIdleNudgeAt = now()
```

`let` bindings are ephemeral and exist only for the current event-processing
turn:

```whipplescript
let planText = plan.snapshot()
```

## Agents

Agents are named targets. They are not pattern groups.

```whipplescript
agent director = thread("director")
agent worker = codingAgent() {
  maxActive 2
}

agent quality = codingAgent() {
  maxActive 1
}
```

When an agent with `maxActive` is started, the workflow must also declare and
process the v0 completion convention:

```whip event finished {
  name string
}

state watching {
  on finished as run {
    stay
  }
}
```

The runtime projects active invocations from native `agent_invocations` rows
and processed `finished.name` values. Completion names should be prefixed with
the agent name, such as `worker-01` or `quality-review-01`. When agent names
overlap, the longest matching started-agent prefix wins.

Group-like behavior is expressed in guards or pattern matching over event data:

```whipplescript
case run.name {
  matches "worker-*" -> { ... }
  matches "quality-*" -> { ... }
  _ -> { stay }
}
```

This keeps agent declarations simple and avoids hidden polymorphism.

## Capabilities

Capabilities declare adapter-backed authority:

```whipplescript
capability plan = adapter("implementationPlan")
```

The workflow may call approved operations on the capability:

```whipplescript
let planText = plan.snapshot()
plan.markBlocked(workItemId, reason)
```

The `.whip` language does not gain arbitrary filesystem, database, process,
or network APIs from a capability declaration. A capability is resolved through
the runtime's adapter registry and workspace policy. The adapter must advertise
operation schemas, required authority, idempotency behavior, and failure modes.

This is the extensibility boundary for future operations such as editing files,
updating databases, or running approved scripts.

## BAML-Shaped Types

User-defined structured types are only `enum` and `class`.

```whipplescript
enum RunKind {
  WorkerComplete
  WorkerFailed
}

class RunClassification {
  kind RunKind
  workItemId string?
  reason string
}
```

The spelling intentionally follows BAML:

- no colons in field declarations
- primitive types use BAML names such as `string`, `int`, `float`, and `bool`
- optional fields use `?`
- arrays use `T[]`
- maps use `map<Key, Value>`

Runtime values are JSON, so map keys are JSON object keys. In v0, `Key` must be
string-compatible: `string`, an enum, a string literal, or a union/ref composed
from those. Schemas such as `map<int, string>` are rejected because JSON object
keys arrive as strings.

The DSL may also define native workflow-only types such as `time`, `duration`,
`agent`, and `json`. These are not emitted as BAML boundary types unless an adapter
explicitly supports a conversion.

Type references must be acyclic. Recursive `class` graphs are rejected in v0 so
schema validation, BAML generation, and model generation remain finite and
predictable.

Record/class schemas are closed in v0. A value for `class RunClassification`
must contain the declared required fields, may omit optional fields, and must not
contain undeclared fields. Use `json` or an explicit `map<string, T>` when a
boundary intentionally accepts arbitrary keys.

## Coerce Functions

`coerce` declares a typed model interpretation function:

```whipplescript
coerce classifyRun(run RunSummary) -> RunClassification {
  model "gpt-4o-mini"

  prompt """
  Classify this finished agent run.

  {{ run }}

  {{ ctx.output_format }}
  """
}
```

`coerce` declarations lower to generated BAML source. Runtime `coerce`
execution uses BAML HTTP against that generated source, so input and output
types must be BAML-compatible.

Both call forms are accepted:

```whipplescript
let classification = coerce classifyRun(summary)
let classification = classifyRun(summary)
```

The first form is recommended in examples when model-dependent control flow
should be visually obvious. The second form is allowed when the binding already
makes the intent clear. Diagnostics should still identify the called declaration
as a coerce function.

## Events And Handlers

Event handlers use `on`:

```whipplescript
on finished as run
  guard !(run.id in data.seenRuns)
{
  ...
}
```

`as run` binds the event payload to a local name. Without an alias, the payload
is available as `event`.

Guards are pure expressions. Multiple `guard` lines are ANDed.

## Hierarchical States

States may contain child states:

```whipplescript
state running {
  initial watching

  on finished as run {
    ...
  }

  state watching {
    on idle {
      goto choosing
    }
  }

  state choosing {
    entry {
      ...
    }
  }
}
```

Handler lookup starts at the active leaf state and walks outward through parent
states. If multiple handlers match the same event at the same level, source
order is used only after guards are evaluated; ambiguous unguarded handlers are
a validation error.

## Handler Outcomes

Every `on`, `entry`, and `always` block may use an explicit outcome:

```whipplescript
stay
goto someState
```

`stay` records no state change. `goto` moves to another declared state. In v0,
omitting an explicit outcome is equivalent to staying in the current state after
the block's effects complete. `finish`, `fail`, `exit`, `after`, and `parallel`
are reserved for future revisions and should not be authored yet.
When `stay` or `goto` is written, it must be the last statement in that block.

## Expressions

The expression language is deliberately small:

```text
paths:          run.id, data.seenRuns, next.action
literals:       strings, ints, floats, bools, nil, durations
comparison:     == != < <= > >=
membership:     in, !(x in y)
patterns:       matches "worker-*"
boolean:        && || !
branching:      case
built-ins:      now(), elapsedSince(...), activeRuns()
```

The full v1 primitive boundary is defined in
[expression-primitives.md](expression-primitives.md). Complex computation
belongs behind `coerce` or declared adapter capabilities, not inline workflow
expressions.

No mutation, loops, recursion, imports, reflection, subprocess execution, or
arbitrary host-language callbacks are allowed.

## Effects

Built-in effect-like statements:

```text
assign     update workflow-local durable data
raise      enqueue an internal event
send       send a message to a declared target
start      launch native harness-managed agent work
askHuman   create a human-review obligation
coerce     run typed model interpretation
```

Adapter-backed capability calls are also effects, but they must be declared and
approved:

```whipplescript
plan.markDone(workItemId)
scripts.run("sync-plan")
```

Unknown effect names fail validation.

## Syntax Consistency

The DSL avoids colons across declarations and object literals:

```whipplescript
class RunSummary {
  id string
  exitCode int?
}

let summary = {
  id run.id
  exitCode run.exitCode
}
```

This keeps WhippleScript close to BAML where BAML already has good syntax, while
statechart constructs remain explicit and standard.
