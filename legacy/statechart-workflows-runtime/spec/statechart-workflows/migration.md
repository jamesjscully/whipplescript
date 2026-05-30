# Migration From Legacy WhippleScript

Legacy WhippleScript centered on scheduled tasks, event triggers, and arbitrary
TypeScript or shell scripts. The statechart workflow track keeps the useful
runtime lessons but changes the product surface.

## What Carries Forward

- durable run/event visibility
- compact status and log inspection
- event-triggered orchestration
- scheduled or external observations such as `idle`
- plain scripts as trusted adapter implementations when explicitly allowed

## What Does Not Carry Forward

- arbitrary TypeScript as normal workflow logic
- unconstrained control flow loops
- workflow-specific state hidden in scripts
- implicit access to shell, filesystem, network, or repo tools
- script output as an untyped control plane

## Mapping Concepts

| Legacy concept | Statechart workflow equivalent |
| --- | --- |
| scheduled task | external observation event such as `idle` |
| event trigger | `on <event>` handler |
| launch-agent script | native `start <agent>` invocation claimed by the harness |
| message script | `send <agent>` effect |
| quality gate script | typed `coerce` decision or bounded adapter effect |
| custom status script | `overview`, `status`, `events`, and `log` |
| script-owned workflow state | `data`, typed events, durable store, or capability-backed resource |
| script-side policy | adapter manifest plus capability policy |

## Migration Steps

1. Identify the workflow lifecycle in statechart terms:
   states, events, guards, effects, and terminal states.
2. Replace script decisions with typed `class`, `enum`, and `coerce`
   declarations where an LLM must classify or choose.
3. Replace script side effects with declared effects:
   `start`, `send`, `askHuman`, `raise`, or a capability call.
4. Move local agent execution into the native harness config; move explicitly
   external authority into an adapter manifest with required capabilities.
5. Move repo or plan state behind a capability such as `plan.snapshot()` or a
   file-backed adapter during local development.
6. Validate with manifests and policies before running.
7. Add formal checks for the lifecycle abstraction before broadening adapter
   authority.

## Script Compatibility

Scripts can still be useful as adapter implementations. They should be treated
as trusted runtime code, not as workflow source.

Acceptable script use:

- a narrow provider command run by the native harness
- a bridge that writes a typed event into WhippleScript
- a repo-specific plan adapter hidden behind a manifest capability

Avoid:

- scripts that own the workflow loop
- scripts that parse logs to decide lifecycle state
- scripts that mutate WhippleScript SQLite state directly
- scripts that bypass capability policy

## Migration Checklist

- The `.whip` file is the primary orchestration artifact.
- Every external effect has a manifest declaration.
- Every required capability is either allowed by policy or intentionally denied.
- Every bounded `start` has a compatible `finished` event path.
- Operators can explain current state using `overview` without reading custom
  scripts.
- Existing scripts, if any, are adapter internals with typed inputs/outputs.
