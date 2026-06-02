# Current State

WhippleScript is useful today for local experiments with durable agent
orchestration, but it is not a stable production dependency yet.

## Works Today

- Source install from a checkout or Git URL.
- `whip check` and `whip compile` for checked `.whip` examples.
- Local SQLite stores for runtime experiments.
- `whip dev` with the fixture provider for deterministic local validation.
- `run`, `step`, and `worker` as separate runtime boundaries.
- Inspection commands: `status`, `log`, `facts`, `effects`, `runs`,
  `evidence`, `diagnostics`, and `trace`.
- Human review inbox shape through fixture-backed `human.ask` effects.
- BAML-style typed coercion effects through fixture-backed local validation.
- Workflow revision through `whip revise` for non-terminal running instances.
- Formal and e2e checks used by maintainers.

## Early Or Experimental

- Public language syntax and lowering behavior may change.
- CLI and JSON output fields may change.
- Native provider integration for Codex, Claude, Pi, Loft, and BAML is still
  settling.
- Plugin/provider packaging and configuration are not stable public contracts.
- Prebuilt GitHub Release binaries are the v0.1 binary install path; source
  install remains the fallback for unsigned or platform-specific issues.
- Production automation is not recommended without project-specific review.

## Stability Language

Some spec trackers say an internal subsystem is stable. In those files, stable
means the in-repo implementation and tests for that subsystem are stable enough
for the current v0 work.

That does not mean the public WhippleScript language, CLI, runtime behavior, or
provider/plugin interfaces are stable semver contracts.

## Best Current Use

Use WhippleScript to prototype and inspect agent orchestration:

- route tasks to logical agents
- add review or human approval gates
- test retry/failure branches
- inspect facts, effects, runs, and evidence
- explore provider/plugin boundaries before wiring real credentials

Start with [Quickstart](quickstart.md), then [Tutorial](tutorial.md).
