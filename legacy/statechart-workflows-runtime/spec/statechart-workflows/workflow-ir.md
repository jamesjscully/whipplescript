# Statechart Workflow IR

Status: design sketch

The workflow IR is the normalized representation shared by parser, static
validator, runtime interpreter, adapters, and verification generators.

The source syntax may evolve. The IR should stay explicit, serializable, and
small enough to inspect in diagnostics.

## Design Requirements

The IR must:

- contain no arbitrary user-authored executable code
- preserve source spans for diagnostics
- make every effect explicit
- make every capability requirement explicit
- expose all coerce input/output schemas needed by validators and model generators
- represent control flow as finite states and transitions
- be lowerable to a transition-system model
- be stable enough for snapshot tests

The IR should be representable as JSON for tests and runtime work. The native
`.whip` DSL lowers to this explicit JSON-shaped IR.

## Top-Level Shape

Illustrative JSON shape:

```json
{
  "schema_version": "statechart-workflow-ir/v0",
  "workflow": {
    "name": "SpecImplementation",
    "source_path": "workflow.whip",
    "repo": ".",
    "contracts": [
      "builder_only/orchestration/contracts/spec-implementation.contract.json"
    ],
    "plan": "state/implementation-plan.json"
  },
  "agents": {},
  "events": {},
  "capabilities": {},
  "context_schema": {},
  "context_initializers": {},
  "types": {},
  "coerce_functions": {},
  "statechart": {},
  "invariants": [],
  "source_spans": {}
}
```

## Workflow Metadata

Workflow metadata defines the coordination boundary:

```json
{
  "name": "SpecImplementation",
  "repo": ".",
  "contracts": [
    "builder_only/orchestration/contracts/spec-implementation.contract.json"
  ],
  "state_scope": ".whipplescript/workflows/spec-implementation",
  "plan": "state/implementation-plan.json"
}
```

The validator resolves paths relative to the workflow source file unless the
workspace configuration says otherwise.

## Agents

Agent declarations normalize source-level thread, coding-agent, and
adapter-backed targets plus optional concurrency:

```json
{
  "director": {
    "target": {
      "type": "thread",
      "name": "director"
    },
    "profile": null,
    "max_active": null,
    "capabilities": [],
    "owns": [],
    "contract": null
  },
  "worker": {
    "target": {
      "type": "coding_agent"
    },
    "profile": "repo-writer",
    "max_active": 4,
    "capabilities": [],
    "owns": [],
    "contract": null
  },
  "external": {
    "target": {
      "type": "adapter",
      "name": "untie"
    },
    "profile": null,
    "max_active": null,
    "capabilities": [],
    "owns": [],
    "contract": null
  }
}
```

Thread agents are message targets. Local `start` targets must be
`coding_agent`; explicitly external starts may target adapter-backed agents.
The runtime never infers additional authority from the target. `profile` is the
requested semantic harness profile for native provider resolution. It does not
grant authority by itself; the harness policy resolves it to provider,
filesystem, network, environment, timeout, and enforcement settings.
`capabilities`, `owns`, and `contract` are reserved IR fields for future
target-specific policy checks; the native harness, adapter manifests, plus
workspace policy remain the implemented authority boundary.

## Events

Events define typed payloads admitted by the workflow:

```json
{
  "finished": {
    "payload": {
      "id": {"type": "string"},
      "name": {"type": "string"},
      "status": {"type": "string"},
      "stdoutTail": {"type": "string"},
      "stderrTail": {"type": "string"},
      "exitCode": {"type": "optional", "inner": {"type": "int"}}
    }
  }
}
```

Every runtime event is validated against the declared event schema before a
transition can consume it.

## Data Schema

The data schema defines durable workflow `data`. The IR field may keep the
internal name `context_schema`, but source diagnostics should use `data`.

```json
{
  "seenRuns": {
    "type": "list",
    "inner": {"type": "string"}
  },
  "runSummary": {
    "type": "optional",
    "inner": {"type": "ref", "name": "RunSummary"}
  },
  "classification": {
    "type": "optional",
    "inner": {"type": "ref", "name": "RunClassification"}
  }
}
```

Data updates must target declared paths. This prevents hidden mutable state
from appearing in guards or actions.

## Capabilities

Capabilities define external project state and approved operations:

```json
{
  "plan": {
    "adapter": "implementationPlan",
    "operations": {
      "snapshot": {
        "input": {"type": "record", "fields": []},
        "output": {"type": "string"}
      },
      "markDone": {
        "input": {
          "type": "record",
          "fields": [
            {"name": "workItemId", "schema": {"type": "string"}}
          ]
        },
        "output": {"type": "null"}
      }
    }
  }
}
```

Capability calls such as `plan.snapshot()` and `plan.markDone(...)` must
reference declared capabilities. Adapter operations must define conflict
behavior, idempotency behavior, and a model abstraction.

## Coerce Functions

The IR records coerce function schemas and the generated BAML artifact location:

```json
{
  "classifyRun": {
    "params": [
      {"name": "run", "schema": {"type": "ref", "name": "RunSummary"}}
    ],
    "output": {"type": "ref", "name": "RunClassification"},
    "model": "gpt-4o-mini",
    "generated_baml_artifact": ".whipplescript/build/workflows/SpecImplementation/baml_src/workflow.baml"
  }
}
```

The model generator treats each coerce call as nondeterministic over the output
schema. The runtime records the concrete result returned by the model provider.

WhippleScript `class`, `enum`, and `coerce` declarations are source of truth.
Generated BAML files are derived artifacts.

## Statechart

The statechart contains states, handlers, transitions, and final states:

```json
{
  "initial": "running",
  "states": {
    "running": {
      "on": [
        {
          "event": "finished",
          "binding": "run",
          "guard": {
            "op": "not",
            "expr": {
              "op": "in",
              "left": {"path": "run.id"},
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
                "args": [{"path": "run.id"}]
              }
            }
          ],
          "transition": "choosing"
        }
      ]
    },
    "choosing": {
      "entry": [
        {
          "action": "let",
          "assign": "planText",
          "value": {
            "op": "call",
            "name": "plan.snapshot",
            "args": []
          }
        },
        {
          "action": "let",
          "assign": "next",
          "value": {
            "op": "call",
            "name": "coerce chooseNextStep",
            "args": [{"path": "planText"}]
          }
        }
      ],
      "always": [
        {
          "guard": {
            "op": "eq",
            "left": {"path": "context.classification.kind"},
            "right": "WorkerComplete"
          },
          "steps": [
            {
              "action": "capability_call",
              "capability": "plan",
              "operation": "markReadyForQuality",
              "args": {
                "workItemId": {"path": "context.classification.workItemId"}
              }
            }
          ],
          "transition": "selecting"
        }
      ]
    },
    "complete": {
      "final": true
    }
  }
}
```

Explicit source initializers are stored separately as pure expression IR:

```json
{
  "context_initializers": {
    "seenRuns": {"op": "list", "items": []},
    "lastIdleNudgeAt": {"op": "literal", "value": null}
  }
}
```

Initializers must be static literal/list/object expressions in v0 and must
match their declared data schema. Runtime initializes durable state from these
values before falling back to schema-derived defaults for missing fields.

Record schemas are closed: declared optional fields may be absent, but
undeclared fields are rejected at runtime and during static assignment,
initializer, adapter, and event payload validation.

## Expressions

Expressions are pure unless they are declared synchronous value calls evaluated
during transition prepare. The initial expression set is the orchestration
kernel defined in [expression-primitives.md](expression-primitives.md):

```text
literal
path
eq
neq
lt/lte/gt/gte
and/or/not
membership
object/list construction
case patterns
string interpolation paths
allowlisted list/map/text/time helpers
coerce calls
capability value calls
```

Expression evaluation cannot dispatch asynchronous effects, mutate workflow
data, call agents, inspect undeclared files, or execute host code. `coerce` and
capability value calls are synchronous value effects with explicit schemas,
policy, logs, and failure behavior.

## Actions

Each action has a normalized name, typed arguments, optional assignment target,
and source span:

```json
{
  "action": "start",
  "args": {
    "agent": "worker",
    "input": {
      "work_item": {"path": "context.next.work_item_id"}
    }
  },
  "assign": null,
  "span": "workflow.whip:92:9"
}
```

The validator resolves each action through the effect registry. Unknown actions
are rejected.

Initial effect names:

```text
send       send a message to an existing agent or thread
start      begin native harness-managed agent work
coerce     call BAML to produce typed data
assign     update workflow-local durable data
askHuman   create a visible human-review obligation
raise      publish a typed event
```

Adapter-backed capability effects such as `plan.snapshot()` and
`plan.markDone(...)` are valid only when declared by an adapter and permitted by
policy.

## Invariants

Invariants are either named built-ins or supported expressions:

```json
[
  {
    "type": "builtin",
    "name": "agentCapabilitiesRespected"
  },
  {
    "type": "expression",
    "name": "max_active_worker",
    "expr": {
      "op": "lte",
      "left": {
        "op": "call",
        "name": "max_active",
        "args": ["worker"]
      },
      "right": 4
    }
  }
]
```

Invariant names must be unique across built-in and expression invariants.
Built-in invariant names must be from WhippleScript's supported built-in invariant
set. Unsupported invariant forms fail validation.

## Source Spans

Every major IR node should carry a source span:

```json
{
  "span": {
    "file": "workflow.whip",
    "start_line": 91,
    "start_column": 9,
    "end_line": 94,
    "end_column": 10
  }
}
```

Diagnostics should reference source spans whenever possible.

## Versioning

The IR must include a schema version. Runtime state should record the workflow
IR version it was created with.

Open questions:

- whether running workflows can migrate across IR versions
- whether source format version and IR version should be separate
- whether enterprise deployments require signed or pinned IR artifacts
