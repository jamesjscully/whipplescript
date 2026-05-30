# Source To IR Lowering

Status: design proposal

`.whip` files use a native statechart DSL. The runtime executes only
validated WorkflowIR; it never interprets raw source directly.

This document defines the source-to-IR contract so parser, validator, runtime,
model generator, examples, and diagnostics do not drift.

## Pipeline

The source pipeline is:

```text
read .whip file
lex and parse WhippleScript DSL
build parsed syntax tree with source spans
lower BAML-shaped enum/class/coerce declarations
normalize statechart source into WorkflowIR
validate WorkflowIR
optionally generate BAML artifacts and verification models
```

## Source Unit

Source:

```whipplescript
machine implementationLoop
initial running
```

IR:

```json
{
  "workflow": {
    "name": "implementationLoop",
    "initial": "running"
  }
}
```

A source file defines exactly one machine. `machine` names it. `initial` is
required at machine scope and for every compound state that has child states.

## Declarations

The parser recognizes these top-level declarations:

```text
machine
initial
data
agent
capability
enum
class
coerce
state
invariant
```

Nested `state` declarations may contain:

```text
initial
entry
always
on
state
final
```

## Data

Source:

```whipplescript
data {
  seenRuns string[] = []
  lastIdleNudgeAt time? = nil
}
```

IR:

```json
{
  "context_schema": {
    "seenRuns": {
      "type": "list",
      "inner": {"type": "string"}
    },
    "lastIdleNudgeAt": {
      "type": "optional",
      "inner": {"type": "time"}
    }
  },
  "context_initializers": {
    "seenRuns": {
      "op": "list",
      "items": []
    },
    "lastIdleNudgeAt": {
      "op": "literal",
      "value": null
    }
  }
}
```

The IR may keep the historical name `context_schema`, but the source language
uses `data`.

## Agents

Source:

```whipplescript
agent director = thread("director")
agent external = adapter("untie")
agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 4
}
```

IR:

```json
{
  "agents": {
    "director": {
      "target": {"type": "thread", "name": "director"}
    },
    "external": {
      "target": {"type": "adapter", "name": "untie"}
    },
    "worker": {
      "target": {"type": "coding_agent"},
      "profile": "repo-writer",
      "max_active": 4
    }
  }
}
```

Agents are simple named targets. They do not imply wildcard group behavior.
Pattern matching over event fields handles groups. `profile` is optional source
metadata for native harness resolution; omitted profiles are resolved by
harness profile policy. Thread agents are message targets for `send`; local
`start` targets must be `codingAgent()` and are recorded in the native agent
ledger. Explicit adapter-backed agents may also be started when their adapter
contract is loaded and policy permits it.

## Capabilities

Source:

```whipplescript
capability plan = adapter("implementationPlan")
```

IR:

```json
{
  "capabilities": {
    "plan": {
      "adapter": "implementationPlan"
    }
  }
}
```

Capability operation schemas come from the adapter registry and workspace
policy, not from arbitrary code in the workflow file.

## BAML-Shaped Types

Source:

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

IR:

```json
{
  "types": {
    "RunKind": {
      "type": "enum",
      "values": ["WorkerComplete", "WorkerFailed"]
    },
    "RunClassification": {
      "type": "record",
      "fields": [
        {"name": "kind", "schema": {"type": "ref", "name": "RunKind"}},
        {
          "name": "workItemId",
          "schema": {
            "type": "optional",
            "inner": {"type": "string"}
          }
        },
        {"name": "reason", "schema": {"type": "string"}}
      ]
    }
  }
}
```

These declarations must be accepted by WhippleScript's type checker and lowerable to
BAML when referenced by a `coerce` declaration.

## Coerce Declarations

Source:

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

IR:

```json
{
  "coerce_functions": {
    "classifyRun": {
      "params": [
        {"name": "run", "schema": {"type": "ref", "name": "RunSummary"}}
      ],
      "output": {"type": "ref", "name": "RunClassification"},
      "model": "gpt-4o-mini",
      "prompt_span": "workflow.whip:36:3",
      "generated_baml_artifact": ".whipplescript/build/workflows/implementationLoop/baml_src/classifyRun.baml"
    }
  }
}
```

The compiler generates BAML from WhippleScript declarations. Generated BAML artifacts
are derived build outputs.

## Statechart Handlers

Source:

```whipplescript
state running {
  initial watching

  on finished as run
    guard !(run.id in data.seenRuns)
  {
    assign data.seenRuns = data.seenRuns.append(run.id)
    goto choosing
  }

  state watching {
    on idle {
      stay
    }
  }
}
```

IR:

```json
{
  "statechart": {
    "initial": "running",
    "states": {
      "running": {
        "initial": "watching",
        "on": [
          {
            "event": "finished",
            "binding": "run",
            "guard": {
              "op": "not",
              "expr": {
                "op": "in",
                "left": {"path": "event.run.id"},
                "right": {"path": "context.seenRuns"}
              }
            },
            "steps": [
              {
                "action": "assign",
                "target": {"path": "context.seenRuns"},
                "value": {
                  "op": "call",
                  "name": "append",
                  "receiver": {"path": "context.seenRuns"},
                  "args": [{"path": "event.run.id"}]
                }
              }
            ],
            "outcome": {"type": "goto", "target": "choosing"}
          }
        ],
        "states": {
          "watching": {
            "on": [
              {
                "event": "idle",
                "steps": [],
                "outcome": {"type": "stay"}
              }
            ]
          }
        }
      }
    }
  }
}
```

Object keys are not source syntax. Source constructs lower to explicit IR nodes
with spans.

## Actions And Effects

Source:

```whipplescript
let next = coerce chooseNextStep(planText)
let classification = classifyRun(summary)
assign data.lastIdleNudgeAt = now()
send director "Worker failed"
start worker { task next.workItemId }
askHuman(next.reason)
plan.markDone(next.workItemId)
```

Normalization:

- `coerce chooseNextStep(...)` and `chooseNextStep(...)` both lower to the
  `coerce` effect when the callee resolves to a `coerce` declaration.
- `assign` lowers to a deterministic data update.
- local `send` and `start` lower to native agent ledger effects; adapter-backed
  agents lower to adapter effects.
- `askHuman` lowers to a human-obligation effect backed by the configured
  review adapter.
- `plan.markDone(...)` lowers to an adapter capability operation whose schema is
  supplied by the capability registry.
- `case` lowers to a structured `case` step. Each arm keeps its pattern, nested
  steps, and arm-local transition so branch outcomes cannot leak into the
  parent event handler.

Unknown calls fail validation. There are no arbitrary user-defined functions.

Current `case` IR shape:

```json
{
  "effect": "case",
  "args": {
    "expr": {"op": "path", "path": "run.name"}
  },
  "assign": null,
  "case_arms": [
    {
      "pattern": {"type": "matches", "pattern": "worker-*"},
      "steps": [],
      "transition": "choosing"
    },
    {
      "pattern": {"type": "wildcard"},
      "steps": [],
      "transition": null
    }
  ],
  "span": null
}
```

## Expression Grammar

The first expression grammar is deliberately small and must stay aligned with
[expression-primitives.md](expression-primitives.md):

```text
Expr        := Or
Or          := And ("||" And)*
And         := Equality ("&&" Equality)*
Equality    := Compare (("==" | "!=") Compare)?
Compare     := Membership (("<" | "<=" | ">" | ">=") Membership)?
Membership  := Unary ("in" Unary)?
Unary       := "!" Unary | Primary
Primary     := Path | String | Number | Boolean | Nil | Duration | Call | Object | List | "(" Expr ")"
Call        := Identifier "(" ArgList? ")"
Path        := Identifier ("." Identifier)*
Object      := "{" ObjectField* "}"
ObjectField := Identifier Expr
List        := "[" (Expr ("," Expr)*)? "]"
ArgList     := Expr ("," Expr)*
```

`matches` is a pattern operator available in `case` arms. Guard-level glob
matching should use the explicit `text.matchesGlob(value, pattern)` helper:

```whipplescript
case run.name {
  matches "worker-*" -> { ... }
  _ -> { stay }
}
```

Supported literals:

```text
true
false
nil
integers
floats
durations such as 2m
triple-quoted strings
double-quoted strings
bare enum identifiers when schema context allows them
```

No mutation, loops, recursion, imports, reflection, or host callbacks are
allowed.

## String Interpolation

WhippleScript strings use `{{ path }}` interpolation in v1:

```whipplescript
send director """
Worker failed: {{ classification.reason }}
"""
```

Each interpolation contains a path, not an arbitrary expression. If a string is
exactly one interpolation, the lowered value preserves the path value's type
instead of forcing a string. General expressions inside interpolation are
deferred so message formatting does not become a second expression language.

Prompt blocks inside `coerce` declarations are passed to BAML as prompt
templates. WhippleScript expression interpolation is not active inside prompt blocks;
BAML/Jinja owns that syntax.

## Handler Outcomes

Handler blocks may end with one explicit outcome:

```text
stay
goto <state>
```

`stay` lowers to no state transition. `goto <state>` lowers to a transition to
the named state. If a handler has no explicit outcome, v0 lowers it as no state
transition after effects complete. `finish`, `fail`, `exit`, `after`, and
`parallel` are not implemented in v0 source lowering.
If a handler includes `stay` or `goto`, that outcome must be terminal in the
action block. The parser rejects later statements and repeated outcomes.

## Invariants

Supported built-in invariants may be referenced by name:

```whipplescript
invariant agentCapabilitiesRespected
```

Expression invariants must be named:

```whipplescript
invariant seenRunsShapeStable {
  assert data.seenRuns == data.seenRuns
}
```

The v0 source parser accepts one `assert <expr>` statement per invariant block.
Invariant names must be unique across built-in and expression invariants.
Unknown built-in invariant names and unsupported invariant forms fail
validation.

## Diagnostics

Lowering must preserve source spans for:

- machine metadata
- data declarations
- agents
- capabilities
- types
- coerce declarations
- states
- handlers
- guards
- expressions
- actions
- invariants

Diagnostics should point at source spans, not only generated IR nodes.
