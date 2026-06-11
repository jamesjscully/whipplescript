# Current state

WhippleScript is pre-1.0. It is good for local experiments with durable agent
orchestration; it is not yet a stable production dependency.

## Stable enough to rely on

- The authoring loop: `check`, `compile`, `dev` with the fixture provider,
  and the inspection commands (`status`, `log`, `facts`, `effects`, `runs`,
  `evidence`, `diagnostics`, `trace --check`).
- The execution model: durable facts, events, effects, atomic rule commits,
  effect dependencies, leases, and replayable traces.
- Static liveness checks: workflows must reach `complete`/`fail` (escape tag
  `@service`), rule reads must be producible (escape tag `@external`).
- The human review loop: `askHuman` effects (with source-declared `choices`),
  `whip inbox`, answers matched by `when human answered ...` rules.
- Sequential `flow` blocks, which lower to ordinary rules visible in
  `whip check`.
- Work queues with the `builtin` tracker (`queue`, `file`/`claim`/`release`/
  `finish`, `when <queue> has ready item`) and the `whip items` command family.
- Time effects: `timeout` clauses, `timer`, and `cancel`, fired on worker
  passes.
- Inline `decide` decisions, `case` over string-literal unions, the general
  `when fact <dotted.name>` readiness form, dev-profile raw `exec` commands
  (allow-listed via `WHIPPLESCRIPT_EXEC_ALLOW`), and hosted script capability
  `exec <name> with <record> -> Type` backed by a SHA-256-pinned manifest.
- Lifecycle controls: `pause`, `resume`, `cancel`, `retry`, and workflow
  revision (`whip revise`) for non-terminal instances.
- Acceptance fixtures (`whip accept`) and tag-filtered assertion reports for
  validating workflows in CI.

"Stable enough" means the in-repo implementation and tests hold; it is not a
semver promise. Syntax, CLI flags, and JSON field names may still change
between releases.

## Experimental

- Native provider adapters (Codex, Claude, Pi) and their cancellation,
  artifact, and recovery behavior.
- Plugin packaging and provider configuration formats.
- Prebuilt release binaries (source install is the reliable fallback).

## Deprecations and removals

- `consume` is a deprecated alias for `done`; it compiles with a warning and
  will be removed.
- `emit` has been removed from the language; using it is now a check error.
- Loft-specific syntax (`loft has ready issue`, `claim ... with loft`) is gone,
  replaced by work queues. The builtin `AgentTurn` type no longer carries
  `issue` or `changedFiles` fields.

## Recommended use today

Prototype and validate orchestration locally: route tasks to logical agents,
add review and approval gates, exercise retry and failure branches with the
fixture provider, and inspect the durable record. Treat real-provider runs
as supervised experiments.
