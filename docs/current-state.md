# Current state

WhippleScript is pre-1.0. It is good for local experiments with durable agent
orchestration; it is not yet a stable production dependency.

The repository docs track the current checkout of `main`. Published release
artifacts are versioned as `0.1.x`; use docs from the matching Git tag when
pinning exact CLI flags, JSON fields, or provider configuration behavior. The
`stage-*` label printed by `whip --help` is an internal implementation-stage
marker.

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
- Concurrent effect execution: a worker pass runs its ready set on a bounded
  thread pool (sized via `WHIPPLESCRIPT_WORKER_CONCURRENCY`), so a fan-out of
  agent turns or `coerce` calls runs in parallel and `agent { capacity N }` has
  runtime meaning.
- Messaging construct surface: outbound `send via <channel>` and inbound
  `when message from <channel> as msg` (binding the built-in `Message`), driven
  under the fixture provider (`whip message` injects an inbound message); live
  Slack/email delivery is experimental.
- Credential management: `whip auth set/status/login` stores LLM credentials for
  the native `coerce` path (owner-only config) and delegates harness OAuth.
- Lifecycle controls: `pause`, `resume`, `cancel`, `retry`, and workflow
  revision (`whip revise`) for non-terminal instances.
- Acceptance fixtures (`whip accept`) and tag-filtered assertion reports for
  validating workflows in CI.

"Stable enough" means the in-repo implementation and tests hold; it is not a
semver promise. Syntax, CLI flags, and JSON field names may still change
between releases.

## Experimental

- The owned brokered harness (`provider owned`, DR-0024): whip runs the agent
  tool-use loop itself and executes each requested file tool, settling to one
  `agent.turn.<status>` fact. In place: the file tools over the file-store
  boundary and a live model client (OpenAI/Anthropic, `WHIPPLESCRIPT_HARNESS_*`,
  with a credential-free fixture fallback for CI), the enforced envelope (a
  configurable per-turn step budget and a durable workspace lease), and a
  default-deny `bash` tool (allow-list of command prefixes, workspace cwd,
  timeout), and capability-gated tracker tools (`list/add/update_todo` over the
  durable work tracker, refined-I3 shared-state participation), and context
  compaction on long turns (projection only; the durable stream is complete).
  Later slice: resume-from-crash.
- Native provider adapters (Codex, Claude, Pi) and their cancellation,
  artifact, and recovery behavior — live execution against real provider SDKs is
  credential-gated.
- Native `coerce` against real LLMs (OpenAI Responses / Anthropic Messages): the
  request/response logic is built and tested, but live calls are opt-in
  (`WHIPPLESCRIPT_COERCE_PROVIDER`) and credential-gated; the fixture path is the
  default.
- Live messaging providers (Slack/email) producing inbound `Message` facts.
- Package manifests and provider configuration formats.
- Prebuilt release binaries (source install is the reliable fallback).

## Recommended use today

Prototype and validate orchestration locally: route tasks to logical agents,
add review and approval gates, exercise retry and failure branches with the
fixture provider, and inspect the durable record. Treat real-provider runs
as supervised experiments.
