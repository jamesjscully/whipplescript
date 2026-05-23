---
name: armature-statechart
description: Use when coding agents need to author, validate, run, inspect, or repair Armature `.armature` statechart workflows for orchestrating agents through durable events, typed effects, adapter manifests, and formal checks.
---

# Armature Statechart Workflows

Use Armature as a restricted workflow runtime for coordinating agents. The
workflow file describes state, events, typed decisions, and approved effects.
It must not execute TypeScript, shell, Python, or arbitrary host-language code.

Keep this boundary clear:

- Armature owns durable workflow mechanics: parsing, validation, event queue,
  state transitions, effect dispatch records, status projection, and model
  generation.
- Trusted adapters own external authority: starting agents, messaging threads,
  running BAML/coerce calls, asking humans, reading plan files, updating stores,
  or bridging legacy systems.
- The workflow owns orchestration semantics: when work starts, when completions
  matter, when quality gates run, when the system is idle, and when humans are
  asked to decide.

## First Checks

1. Confirm the new CLI surface is available: `armature --help`. From source,
   build with `cargo build -p armature-cli` and use `target/debug/armature`.
2. Find the workflow file, usually `examples/workflows/*.armature` or a repo
   workflow under `.armature/workflows/`.
3. Validate before running:

   ```sh
   armature validate path/to/workflow.armature --json
   ```

4. If the workflow uses adapter-backed effects, validate with every manifest:

   ```sh
   armature validate path/to/workflow.armature \
     --adapter-manifest path/to/adapter.json \
     --json
   ```

5. Start inspection with `overview`, not custom status scripts:

   ```sh
   armature overview path/to/workflow.armature --json
   ```

## Authoring Shape

A workflow should read like one statechart. Keep related behavior near the
state that owns it.

```armature
machine SimpleSupervisor
initial watching

data {
  seenRuns string[]
  lastIdleNudgeAt time? = nil
}

agent director = thread("director")
agent worker = codingAgent() {
  maxActive 2
  capabilities ["edit_code", "run_tests"]
}

event finished {
  name string
  runId string
  ok bool
}

event idle {
  activeRuns int
  unfinishedItems int
}

state watching {
  on finished as run
    guard !(run.runId in data.seenRuns)
  {
    case run.name {
      matches "worker-*" -> {
        assign data.seenRuns = data.seenRuns.append(run.runId)
        send director """
          Agent run {{ run.runId }} completed.
          Inspect logs and update the plan.
        """
        stay
      }

      _ -> {
        stay
      }
    }
  }

  on idle as observation
    guard observation.activeRuns == 0
    guard observation.unfinishedItems > 0
  {
    assign data.lastIdleNudgeAt = now()
    send director """
      The implementation loop appears idle.
      Inspect the plan and restart work or record the blocker.
    """
    stay
  }
}
```

Prefer canonical statechart words already in the language: `machine`,
`initial`, `state`, `on`, `guard`, `entry`, `always`, `goto`, `stay`, and
`final`.

## Coerce

Use `coerce` for structured LLM decisions. Define local classes and enums in
the same file, following BAML's shape. Enum values must start with an uppercase
ASCII letter so generated BAML accepts them. Maps use `map<Key, Value>`, and
v0 map keys must be string-compatible (`string`, enum, string literal, or a
union/ref of those) because runtime values are JSON objects.

```armature
enum NextAction {
  StartWorker
  AskHuman
  Done
}

class NextStep {
  action NextAction
  workItemId string?
  reason string
}

coerce chooseNextStep(planText string) -> NextStep {
  prompt #"
    Choose the next workflow action from the current plan.
    Return a NextStep.

    {{ planText }}
  "#
}

state choosing {
  entry {
    let planText = plan.snapshot()
    let next = coerce chooseNextStep(planText)

    case next.action {
      StartWorker -> {
        start worker {
          workItemId next.workItemId
          reason next.reason
        }
        goto watching
      }
      AskHuman -> {
        askHuman(next.reason)
        goto watching
      }
      Done -> {
        goto done
      }
    }
  }
}
```

Use `coerce choose(...)` or `choose(...)` consistently in a file. Prefer the
explicit `coerce` form when teaching or reviewing a workflow.

## Workflow Design Rules

- Model lifecycle explicitly. Every started bounded agent should have a
  processable `finished` event so active counts can retire.
- Use `maxActive` on agents that can fan out. Static validation and runtime
  enforcement both depend on it.
- Treat `idle` as an observation event, not a loop. The statechart should decide
  what to do when the system is idle.
- Record dedupe facts in `data`, such as `seenRuns`, before messaging or
  starting more work.
- Keep BAML/coerce decisions small and typed. Do not ask a general agent to
  infer workflow control flow from prose when a `class` or `enum` can make the
  decision explicit.
- Keep external authority behind capabilities and adapter manifests. Do not add
  scripting escape hatches to solve one workflow.

## Adapter Manifests

Use adapter manifests to declare what trusted runtime code can do. A workflow
may request `start`, `send`, `askHuman`, or capability calls only when loaded
manifests declare the effect and category.
Manifest `input` schemas describe the runtime request `args` envelope. Include
language routing fields such as `agent`, `capability`, and `operation` when the
effect dispatches them; otherwise static validation may pass less authority
than the runtime actually sends. Optional authored arguments are omitted from
the envelope when absent.
Manifest `output` schemas are used for capability value calls in expressions
when a manifest is loaded, so a non-string `plan.count()` cannot be used as a
`send` message without an explicit coerce step.
When an effect argument expression has a known schema, Armature uses it inside
the request envelope too; for example `start worker { taskId 42 }` can be
rejected by a manifest requiring `input.taskId string`.
Expression-style capability call inputs are function-like: no args expects an
empty record, one arg expects that argument schema, and multiple args expect a
positional list schema. Statement-style capability calls use the dispatch
envelope with `capability`, `operation`, and `call_args`.

```json
{
  "name": "agent-adapter",
  "version": "0.1.0",
  "effects": {
    "start": {
      "category": "async_invocation",
      "required_capabilities": ["adapter.agent.start"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["adapter_failure", "timeout"],
      "model": {"kind": "nondeterministic_outcome", "values": ["accepted", "rejected"]}
    }
  },
  "events": {
    "finished": {
      "type": "record",
      "fields": [
        {"name": "name", "schema": {"type": "string"}},
        {"name": "runId", "schema": {"type": "string"}},
        {"name": "ok", "schema": {"type": "boolean"}}
      ]
    }
  }
}
```

In manifests, `required_capabilities`, `failure_categories`, and
nondeterministic model `values` are exact tokens. Keep them non-empty,
duplicate-free within the local list, and free of whitespace/control
characters.

Validate manifests independently when debugging adapter problems:

```sh
armature validate-adapter path/to/adapter.json --json
```

## Capability Policy

Use `--policy` when a workflow should be checked against an explicit capability
posture. The initial policy format is JSON with exact capability names:

```json
{
  "mode": "enterprise",
  "allowed_capabilities": [
    "adapter.agent.start",
    "message_agents",
    "resource.plan.read",
    "resource.plan.write"
  ],
  "denied_capabilities": []
}
```

Capability names must be non-empty and contain no whitespace/control
characters. Fix manifest or policy spelling instead of compensating for a
near-match in workflow source.

Local mode reports unknown required capabilities as warnings. Enterprise mode
reports unknown required capabilities as errors. Denied capabilities are errors
in every mode. Supplied policy is checked during validation and enforced again
by the manifest dispatcher before runtime effect dispatch.

```sh
armature validate-policy policy.json --json

armature validate workflow.armature \
  --adapter-manifest adapter.json \
  --policy policy.json \
  --json
```

## Running A Workflow

Emit synthetic events during development:

```sh
armature emit workflow.armature \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":3}' \
  --json
```

Process one queued event:

```sh
armature run workflow.armature --json
```

Or enqueue and process one event:

```sh
armature run workflow.armature \
  --event finished \
  --payload '{"name":"worker-17","runId":"run_17","ok":true}' \
  --adapter-manifest adapter.json \
  --json
```

Inspect durable state:

```sh
armature overview workflow.armature --adapter-manifest adapter.json --json
armature overview workflow.armature --adapter-manifest adapter.json --policy policy.json --json
armature status workflow.armature --json
armature events workflow.armature --json
armature log workflow.armature --json
```

Use a dedicated `--store path/to/workflow.sqlite` when testing multiple runs or
when you need repeatable fixtures.
The human `overview` output includes latest effect status, required
capabilities, and effect errors. Use `--json` when a coding agent needs the
same fields in a machine-readable shape.

## Formal Checks

Generate model artifacts from validated IR:

```sh
armature emit-model workflow.armature --target tla
armature emit-config workflow.armature --target tla
armature emit-model workflow.armature --target maude
armature emit-model workflow.armature --adapter-manifest adapter.json --policy policy.json --target maude
```

Use `emit-config --target tla` when you need to inspect the exact invariant set
that `check --target tla` will run.

Run bounded checks when TLC or Maude is installed, or when the repo Nix flake is
available:

```sh
armature check workflow.armature --target tla --json
armature check workflow.armature --target maude --json
armature check workflow.armature --adapter-manifest adapter.json --policy policy.json --target tla --json
```

Formal models are abstractions. They are useful for lifecycle invariants such
as known states and `maxActive` limits; they do not prove real LLM behavior,
external adapter behavior, or repo-specific quality.
When an adapter manifest or policy is supplied, Armature validates
adapter-backed workflow effects before emitting or checking the abstraction.
`prove` is reserved for stronger future backends. It currently validates the
workflow/contracts and then reports unavailable; use `check` for bounded
verification.

## Debugging

Start with diagnostics, then state:

```sh
armature validate workflow.armature --adapter-manifest adapter.json
armature overview workflow.armature --adapter-manifest adapter.json
armature events workflow.armature
armature log workflow.armature
```

Common repairs:

- `effect ... is not declared`: add or load the right adapter manifest, or
  remove the unsupported effect.
- `expects category`: fix the manifest category to match the language effect.
- `requires denied capability` or `requires capability ... not allowed`: update
  the policy document only if that authority is intended; otherwise remove or
  replace the effect.
- `initial state ... is not declared`: update `initial` or add the state.
- `uses undeclared capability`: add a `capability name = adapter("...")`
  declaration or remove the call.
- `maxActive must be greater than 0`: set a positive limit or omit `maxActive`.
- Active work never retires: ensure started agents eventually emit a `finished`
  event with required `name`, and that the workflow has an `on finished`
  handler reachable from the active state.

Do not repair workflow stalls by adding unbounded event loops, polling scripts,
or arbitrary code execution. First make the lifecycle state, event, and
capability contract explicit.

## Repository References

Read these files when details matter:

- `README.md` for the current command surface and smoke commands.
- `spec/statechart-workflows/grammar.md` for exact DSL syntax.
- `spec/statechart-workflows/source-to-ir.md` for lowering rules.
- `spec/statechart-workflows/runtime-semantics.md` for interpreter behavior.
- `spec/statechart-workflows/effects.md` for effect categories.
- `spec/statechart-workflows/component-contracts.md` for typed boundaries.
- `spec/statechart-workflows/verification.md` for model-checking strategy.
- `examples/workflows/minimal.armature` for the smallest valid workflow.
- `examples/workflows/simple-supervisor.armature` for a compact completion and
  idle-observation workflow.
- `examples/workflows/spec-implementation.armature` for the richer orchestration
  example.
