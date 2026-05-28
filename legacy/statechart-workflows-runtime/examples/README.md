# Examples

This directory contains workflow-first examples for the new `.whip` product
surface.

## Workflows

[`workflows/`](workflows/) contains native `.whip` source sketches:

- `minimal.whip` is a tiny statechart shape for parser/runtime scaffolding.
- `simple-supervisor.whip` is a compact director notification workflow for
  completion and idle observation events.
- `baml-coerce-smoke.whip` is a small opt-in real BAML HTTP smoke workflow.
- `spec-implementation.whip` is the target managed spec implementation
  workflow.

The examples are used by parser, validation, runtime, formal-check, and e2e
tests. Normal CI uses deterministic fake adapter/coerce outputs; the BAML smoke
workflow runs only when a developer opts in with `WHIPPLETREE_RUN_BAML_E2E=1` and
`WHIPPLETREE_BAML_URL`.

## Policies

[`policies/`](policies/) contains JSON capability policy examples:

- `spec-implementation.enterprise-policy.json` allows the fake spec
  implementation adapter's required capabilities in deny-by-default enterprise
  mode.
- `local-file-backed.policy.json` allows the built-in JSON plan, review, and
  agent-file adapter capabilities for local development while keeping BAML
  network disabled.
- `enterprise-baml-http.policy.json` allows only `baml.coerce` against an exact
  local BAML HTTP URL and redacts raw responses.

## Templates

[`templates/`](templates/) contains copyable workflow starting points. Start
with `simple-agent-supervisor.whip` when you need a small worker lifecycle
loop that uses the built-in JSON agent-file adapter.

## Managed Spec Orchestration

[`managed-spec-orchestration/`](managed-spec-orchestration/) contains the earlier
contract/script exploration that motivated the statechart workflow design. Treat
it as background material, not the new implementation center.
