# Current state

WhippleScript is pre-1.0. It is good for durable agent orchestration — both
locally and deployed to the edge — but it is not yet a stable production
dependency.

These docs track the `0.1.0` release (the first public release; it collapses the
earlier internal 0.2/0.3/0.4 development lines into one feature-complete cut).
Use docs from the matching Git tag when pinning exact CLI flags, JSON fields, or
provider configuration behavior. `whip --help` prints the version plus a single
`release` implementation-stage label (not a separate compatibility version).

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
- Work queues with the `builtin` tracker (`file`/`claim`/`release`/
  `finish`, `when <tracker> has ready issue`) and the `whip issue` command family.
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
  `when message from <channel> as msg` (binding the built-in `Message`), with
  binding-driven local providers — `local` (file-backed mailbox + inbox poll,
  inspected with `whip mailbox`), `desktop` (outbound-only native
  notification), `stdio`, and `fixture` (`whip message` injects an inbound
  message); live Slack/email delivery stays deferred.
- Credential management: `whip auth status` and `whip auth set <openai|anthropic>
  <key>` store LLM credentials for the native `coerce` path (owner-only config).
- Lifecycle controls: `pause`, `resume`, `cancel`, `retry`, and workflow
  revision (`whip revise`) for non-terminal instances.
- Restorable context: `whip checkpoint` / `whip restore` rewind file state, the
  agent transcript, and the event-log position together as one coherence-checked
  cut (restore does a full reconcile and auto-checkpoints head first). Native
  file I/O only. See [Runtime & operations](runtime-operations.md).
- Acceptance fixtures (`whip accept`) and tag-filtered assertion reports for
  validating workflows in CI.

"Stable enough" means the in-repo implementation and tests hold; it is not a
semver promise. Syntax, CLI flags, and JSON field names may still change
between releases.

## Cloud runtime

The same evaluation kernel runs unchanged inside a Cloudflare Durable Object
wasm isolate (sans-IO: the only async primitive is `fetch`, and HTTP-bearing
effects are resumable step machines that survive isolate eviction). The DO host
runs the same instance scheduler over the DO's synchronous SQLite; timers fire
via DO alarms and provider credentials come from DO secrets. `whip deploy` is a
one-command edge deploy of a workflow to a Worker + DO. A Class-A compute plane
(`whip executor`, a sidecar over a `whip-executor/1` wire) is built but is a
follow-on configuration step — it is not on by default. See
[Runtime & operations](runtime-operations.md) for the full cloud section.

## Experimental

- The owned brokered harness (`provider owned`, DR-0024): whip runs the agent
  tool-use loop itself and executes each requested file tool, settling to one
  `agent.turn.<status>` fact. In place: the file tools over the file-store
  boundary and a live model client (OpenAI/Anthropic, `WHIPPLESCRIPT_HARNESS_*`,
  with a credential-free fixture fallback for CI), the enforced envelope (a
  configurable per-turn step budget and a durable workspace lease), and a
  sandboxed `bash` tool (the in-isolate Bashkit virtual shell over the workspace
  file surface — no OS/network reach; gated by `with access to command { run }`),
  and capability-gated tracker tools (`list/add/update_todo` over the
  durable work tracker; mutating calls use `with access to tracker { ... }`),
  context
  compaction on long turns (projection only; the durable stream is complete), and
  resume-from-crash (the turn transcript is persisted per step and a recovered
  turn resumes from that projection). The delegating Codex/Claude adapters are now
  optional Cargo features, with the owned harness as the built-in path.
- Native provider adapters — Codex and Claude are the validated delegating
  adapters. Their cancellation, artifact, and recovery behavior,
  and live execution against real provider SDKs, are credential-gated.
- Native `coerce` against real LLMs (OpenAI Responses / Anthropic Messages): the
  request/response logic is built and tested, but live calls are opt-in
  (`WHIPPLESCRIPT_COERCE_PROVIDER`) and credential-gated; the fixture path is the
  default.
- Live messaging providers (Slack/email) producing inbound `Message` facts.
- Network access: an `http source` fetches an external URL (GET-only) behind an
  SSRF/egress policy — http(s) only, private/loopback blocked, host allowlist via
  `WHIPPLESCRIPT_HTTP_SOURCE_ALLOW`; the results surface as the source's `emit`.
  Web *search* is designed but deferred (not shipped).
- Package manifests and provider configuration formats.
- Prebuilt release binaries (source install is the reliable fallback).

## Recommended use today

Prototype and validate orchestration locally: route tasks to logical agents,
add review and approval gates, exercise retry and failure branches with the
fixture provider, and inspect the durable record. The same workflow can then be
deployed to the edge with `whip deploy` (see the Cloud runtime section above).
Treat real-provider runs as supervised experiments.
