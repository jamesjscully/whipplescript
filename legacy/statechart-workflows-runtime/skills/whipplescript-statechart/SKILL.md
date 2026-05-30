---
name: whipplescript-statechart
description: Use when coding agents need to author, validate, run, inspect, or repair WhippleScript `.whip` statechart workflows for orchestrating agents through durable events, typed effects, adapter manifests, and formal checks.
---

# WhippleScript Statechart Workflows

Use WhippleScript as a restricted workflow runtime for coordinating agents. The
workflow file describes state, events, typed decisions, and approved effects.
It must not execute TypeScript, shell, Python, or arbitrary host-language code.

Keep this boundary clear:

- WhippleScript owns durable workflow mechanics: parsing, validation, event queue,
  state transitions, effect dispatch records, status projection, and model
  generation.
- Trusted adapters own external authority: starting agents, messaging threads,
  running BAML/coerce calls, asking humans, reading plan files, updating stores,
  or bridging legacy systems.
- The workflow owns orchestration semantics: when work starts, when completions
  matter, when quality gates run, when the system is idle, and when humans are
  asked to decide.

## First Checks

1. Confirm the new CLI surface is available: `whip --help`. From source,
   build with `cargo build -p whipplescript-cli` and use `target/debug/whip`.
2. Find the workflow file, usually `examples/workflows/*.whip` or a repo
   workflow under `.whipplescript/workflows/`.
3. For a new local project, scaffold the default files first:

   ```sh
   whip init path/to/project --name MyWorkflow --json
   ```

4. Validate before running:

   ```sh
   whip validate path/to/workflow.whip --json
   ```

5. If the workflow uses adapter-backed effects, validate with every manifest:

   ```sh
   whip validate path/to/workflow.whip \
     --adapter-manifest path/to/adapter.json \
     --json
   ```

   For the built-in JSON file-backed adapters, use the shortcut flags instead
   of writing a manifest by hand:

   ```sh
   whip validate path/to/workflow.whip \
     --plan-file plan.json \
     --review-file reviews.json \
     --json
   ```

6. Start inspection with `overview`, not custom status scripts:

   ```sh
   whip overview path/to/workflow.whip --json
   whip harness status path/to/workflow.whip --json
   ```

## Authoring Shape

A workflow should read like one statechart. Keep related behavior near the
state that owns it.

```whipplescript
machine SimpleSupervisor
initial watching

data {
  seenRuns string[]
  lastIdleNudgeAt time? = nil
}

agent director = thread("director")
agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 2
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

```whipplescript
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
When an effect argument expression has a known schema, WhippleScript uses it inside
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
whip validate-adapter path/to/adapter.json --json
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
whip validate-policy policy.json --json

whip validate workflow.whip \
  --adapter-manifest adapter.json \
  --policy policy.json \
  --json
```

## Running A Workflow

Emit synthetic events during development:

```sh
whip emit workflow.whip \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":3}' \
  --json
```

Process one queued event:

```sh
whip run workflow.whip --json
```

For deterministic development runs, use each `--fake-coerce-output NAME=JSON`
or `--fake-call-output NAME=JSON` name at most once. Duplicate fake output names
are rejected, and names may not contain whitespace or control characters.

Or enqueue and process one event:

```sh
whip run workflow.whip \
  --event finished \
  --payload '{"name":"worker-17","runId":"run_17","ok":true}' \
  --adapter-manifest adapter.json \
  --json
```

Use built-in file-backed adapters for local end-to-end workflows:

```sh
whip run workflow.whip \
  --plan-file plan.json \
  --review-file reviews.json \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":1}' \
  --json
```

Typed adapter-originated events can be enqueued without custom manifests:

```sh
whip harness once workflow.whip \
  --config harness.json \
  --json

whip harness run workflow.whip \
  --config harness.json \
  --drive-workflow \
  --max-iterations 10 \
  --json

whip emit workflow.whip \
  --review-file reviews.json \
  --event humanReview.responded \
  --payload '{"reviewId":"review-1","decision":"approved","response":"ship it"}' \
  --json
```

Inspect durable state:

```sh
whip overview workflow.whip --adapter-manifest adapter.json --json
whip overview workflow.whip --adapter-manifest adapter.json --policy policy.json --json
whip overview workflow.whip --json
whip status workflow.whip --policy policy.json --json
whip harness status workflow.whip --json
whip events workflow.whip --policy policy.json --json
whip events workflow.whip --status failed --json
whip events workflow.whip --status dead_lettered --json
whip retry-event workflow.whip --event-id evt_cli_... --json
whip log workflow.whip --policy policy.json --json
```

`events --limit` and `log --limit` are bounded inspection controls; keep them
at or below 10,000 records.
Plain `events --status failed` text output includes retry-relevant attempt
counts when nonzero and `last_error` when present; use `--json` when a coding
agent needs the full event payload.
Plain `retry-event` text output confirms `status=queued` and the resulting
`pending_events` count after an administrative retry.

Harness provider config maps workflow agents to `command`, `codex`, `claude`,
or `pi`. `command` requires an explicit command array; the presets provide
thin command templates and may also use command overrides. `timeoutSeconds`
kills a stuck provider and records a typed `finished` event with
`status: "timed_out"` when the workflow declares the standard completion event.
Use placeholders such as `{{prompt}}`, `{{inputJson}}`, `{{invocationId}}`,
`{{agent}}`, and `{{runDir}}` inside configured command arguments.

When authoring workflows for real agents, prefer semantic harness profiles in
the `.whip` source and let the harness policy map those profiles to concrete
providers and sandbox settings:

- `research`: external documentation, package discovery, and web research.
- `repo-reader`: repository inspection without edits.
- `repo-writer`: implementation work after the task is clear.
- `human-review`: approval, decision collection, or structured review.

Read custom profile descriptions before assigning them. Do not combine network
access and repository write access unless the user explicitly requests
permissive mode or supplies a custom profile whose description allows it.
Use provider names such as `codex`, `claude`, `pi`, or `command` in harness
policy/config, not as workflow intent.

`whip harness status --json` includes workflow status, recent invocations,
recent completions, harness events, and recent desire-path failures such as
`unknown_agent`, `provider_command_failed`, `provider_timed_out`,
`completion_schema_mismatch`, `workflow_validation_failed`, and
`lease_expired`.

Use a dedicated `--store path/to/workflow.sqlite` when testing multiple runs or
when you need repeatable fixtures.
The human `overview` and `status` outputs include current state, queued events,
active invocations, latest effects, required capabilities, policy blockers,
current effect failures, current blockers, historical recent failures, latest
coerce calls, the current coerce failure only while its event is unresolved,
historical latest coerce failures, and summarized workflow data. JSON status
also exposes `current_effect_failures`, `current_coerce_failure`,
`current_blockers`, and
`recent_effects[].idempotency_key` for adapter reconciliation and repair. Use
`status --compact` for a short operator view and `--json` when a coding agent
needs the same fields in a machine-readable shape.

## Formal Checks

Generate model artifacts from validated IR:

```sh
whip emit-model workflow.whip --target tla
whip emit-config workflow.whip --target tla
whip emit-model workflow.whip --target maude
whip emit-model workflow.whip --adapter-manifest adapter.json --policy policy.json --target maude
```

Use `emit-config --target tla` when you need to inspect the exact invariant set
that `check --target tla` will run.

Run bounded checks when TLC or Maude is installed, or when the repo Nix flake is
available:

```sh
whip check workflow.whip --target tla --json
whip check workflow.whip --target maude --json
whip check workflow.whip --adapter-manifest adapter.json --policy policy.json --target tla --json
```

Formal models are abstractions. They are useful for lifecycle invariants such
as known states and `maxActive` limits; they do not prove real LLM behavior,
external adapter behavior, or repo-specific quality.
When an adapter manifest or policy is supplied, WhippleScript validates
adapter-backed workflow effects before emitting or checking the abstraction.
`prove` validates the workflow/contracts and runs the current generated
verification bundle, TLA+ plus Maude when those tools are available. Use
`check --target tla` or `check --target maude` when you need one backend.

## Debugging

Start with diagnostics, then state:

```sh
whip validate workflow.whip --adapter-manifest adapter.json
whip overview workflow.whip --adapter-manifest adapter.json
whip events workflow.whip --adapter-manifest adapter.json
whip log workflow.whip --adapter-manifest adapter.json
```

Common repairs:

- `effect ... is not declared`: add or load the right adapter manifest, or
  remove the unsupported effect. For local JSON adapters, prefer
  `--plan-file` or `--review-file` before inventing a custom manifest. Local
  agents use the native harness ledger, not an adapter manifest.
- `expects category`: fix the manifest category to match the language effect.
- `policy document validation failed`: fix duplicate, empty, or invalid policy
  entries before changing the workflow.
- `requires denied capability` or `requires capability ... not allowed`: update
  the policy document only if that authority is intended; otherwise remove or
  replace the effect.
- `payload does not match schema for event`: fix the emitted JSON path named in
  the diagnostic, such as `$.message expected string, got int`, to match the
  workflow event or built-in adapter event. The harness supplies the standard
  `finished` payload shape for native agent completions; `emit --review-file`
  supplies `humanReview.responded`.
- Parser suggestions about `agent`, `when`, or `start worker("...")` are
  canonical syntax hints: use `agent worker = codingAgent() { maxActive 1 }`,
  use one or more `guard` lines before the action block, and pass start input
  with `start worker { message "..." }`.
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
- `spec/statechart-workflows/operations.md` for stuck workflow repair.
- `spec/statechart-workflows/migration.md` for legacy script-runner migration.
- `spec/statechart-workflows/database-migrations.md` for SQLite migration
  rules.
- `spec/statechart-workflows/release-checklist.md` for release and upgrade
  checks.
- `spec/statechart-workflows/verification.md` for model-checking strategy.
- `examples/workflows/minimal.whip` for the smallest valid workflow.
- `examples/workflows/simple-supervisor.whip` for a compact completion and
  idle-observation workflow.
- `examples/workflows/spec-implementation.whip` for the richer orchestration
  example.
- `examples/templates/` for copyable starting points.
