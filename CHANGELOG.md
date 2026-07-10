# Changelog

All notable changes to WhippleScript are recorded here. This project aims to
follow [Semantic Versioning](https://semver.org). Dates are UTC.

> 0.3 adds cloud deployment (the Cloudflare Durable Object runtime) and the owned
> harness. The experimentation/evals + versioned-workspace work (0.4) is tracked
> separately and is **not** part of 0.3. Native provider support is validated for
> **Codex and Claude**; the Pi native provider is deferred.

## [0.4.0] — Unreleased (staged 2026-07-10; date set at cut)

WhippleScript gains its own version control. The versioned workspace replaces
the git seam for agent work: workspace-as-database with O(1) branches over a
content-addressed store, and every instance's files, conversation, and effects
move as one coherent, provenance-carrying line.

### Versioned workspace (the git-replacement surface)
- Branches, cuts, and virtual working sets: O(1) branch creation, per-instance
  copy-on-write file surfaces, branch-distinct effect keys, and
  materialize-on-exec with an O(touched) racy-window-sound import-back.
- The mapped 13-operation workspace API (`workspace_api.rs`, refusals as
  data), the op log as a first-class reflog with `whip branch undo-op`,
  review-grade Myers diffs, handoff bundles (`whipplescript.bundle.v1`) with
  chunk-granular delta transfer and pack objects, and per-blob erasure that
  discharges `HISTORY_PRESERVED` / `EXPORTED_COPY_NOT_RECALLED` by test.
- Selection algebra (`path()`/`by-effect()`/`since()`/`dependents-of()` with
  `| ~ &`) behind selective `undo`/`transport`/`adopt --only` — dry-run by
  default, stranding-checked, no destructive verbs.
- Structured conflicts with rerere-style resolution memory the reconciliation
  daemon auto-propagates; checkout-free `bisect`, `attribution`, and `log`.
- `whip fork` — the chat fork: a new instance seeded from the source's
  completed turns on a fresh branch forked at the source line's head, both
  planes from one quiescent coordinate.

### Policy plane, auth, and the store seam
- DR-0036: turn receipts now carry a witnessed workspace cut
  (`workspace_cut_ref`; honest decline when a native command mutated outside
  the mediated surface) and a dynamic guarantee section
  (`guarantee writes_within:<scope>` / `no_reads_beyond_grant` /
  `no_tainted_reads:<class>` declared in the governance envelope) evaluated
  per turn under the cited policy epoch.
- Host-resolved provider profiles (`WHIPPLESCRIPT_PROVIDER_PROFILES`): the
  policy channel hands whip resolved credentials; whip's own auth is the thin
  standalone fallback.
- The store seam: `whip handles` (stable pointers for external admission
  logs), `whip checkpoint --external-positions` (the position-pair cut for
  cross-store backup/handoff), and the seam-contract draft.

### Owned-harness web tools
- `web_search` (SearchProvider trait; Brave first-party, model-provider floor,
  honest absent tier) and `web_fetch` (structurally GET-only behind a central
  SSRF guard with pinned connections and redirect re-entry; HTML→markdown),
  granted via `with access to web { search fetch }`.

### Formal models
- Six new gate-registered Maude models with verified bites: merge-slice,
  merge-confluence, workstream, branch-effect-key, selective-undo, stat-cache
  (P0), plus op-undo, resolution-memory, chat-fork, turn-witness, and
  seam-crossing, and the ReconciliationDaemonLifecycle TLA+ model.

## [0.3.0] — 2026-07-10

WhippleScript is a small scripting language for AI to orchestrate AI. This release
takes the language onto the edge: the same durable, replayable rule/effect kernel
now runs unchanged in a Cloudflare Durable Object, and the owned agent harness
gains its context layer and a restore-to-a-prior-point capability.

### Cloud runtime — Cloudflare Durable Object
- A sans-IO refactor lets the whole evaluation core (parser, kernel, rule/flow
  engine, effect ledger) run inside a single-threaded wasm isolate, where the
  only async primitive is `fetch`: every HTTP-bearing effect (`coerce`, agent
  turns) is a resumable step machine that suspends on a request and resumes on
  the response, so an instance survives isolate eviction with no lost work.
- Durable-object host binding: the same instance scheduler runs over the DO's
  synchronous SQLite (a full port of the runtime/coordination/work-item stores),
  with alarms for timers/deadlines and secrets for provider credentials.
- `whip deploy` — one-command edge deploy of a workflow to a Worker + DO.
- **Feature parity with the native runtime**: `file.*` effects run over a
  DO-owned file plane; `whip checkpoint` / `whip restore` work as operator
  commands on a deployed instance; and an agent turn runs a real in-isolate tool
  set (read/write/edit/ls/find/grep/recall + the work-tracker todos) against the
  DO's own storage — no filesystem, no subprocess.
- A Class-A compute plane for real toolchains (`whip executor` sidecar over a
  `whip-executor/1` wire) and a Class-B per-turn container path are built and
  live-proven; enabling them in production is a follow-on configuration step.
  In-cluster sidecar calls authenticate with a constant-time-compared `Bearer`
  token (`WHIP_EXECUTOR_TOKEN`), and the sidecars refuse non-loopback calls that
  lack it — the production hardening the compute plane needed.

### Network access
- `http source` fetches an external URL, GET-only, behind an SSRF/egress policy:
  http(s) schemes only, private/loopback IP addresses blocked, and a host
  allowlist (`WHIPPLESCRIPT_HTTP_SOURCE_ALLOW`, with
  `WHIPPLESCRIPT_HTTP_SOURCE_ALLOW_PRIVATE=1` to permit private hosts for local
  development). Web *search* is designed but deferred to a later release.

### Owned agent harness — the context layer
- The owned harness gains a pi-mirrored context layer: a system-prompt assembler,
  a skills control plane (discover-all + model-driven read; skill bodies stored
  content-addressed; skills never grant authority), deploy-shipped project
  instructions (`AGENTS.md` / `CLAUDE.md` discovery, injected verbatim), and
  turn-scoped skill pins.
- Cache-aware conversation compaction: a pluggable `Compactor` with three
  strategies, designed so the assembled prefix stays append-only between
  compactions (the model-provider prompt cache is never needlessly busted) and a
  compaction summary is recorded once and reused on replay.

### Restorable context — checkpoint / restore
- `whip checkpoint` / `whip restore`: rewind an agent's work — its files, its
  transcript, and the instance's event-log position — to a prior point as one
  consistent, coherence-checked cut. File history is captured content-addressed,
  so restore reverts to exact prior bytes; a restore refuses rather than applying
  a partial (dangling) cut, and auto-checkpoints the current head first so the
  undo is itself undoable.

### Reliability
- Every provider request now carries a stable per-effect `Idempotency-Key`
  (resume-stable, not fingerprint-derived), so an at-least-once retry after an
  eviction mid-request is de-duplicated by providers that honor it.

### Embedding and governance
- The `whipplescript` package now publishes its governance and IFC trust boundary
  as a Rust library. Hosts can require a signed envelope, verify its attestation,
  and bind a policy epoch to the verified canonical hash and signer without
  reimplementing WhippleScript's policy parser or security algebra. A malformed
  configured envelope fails closed instead of becoming an ungoverned run.
- The same library publishes `whipplescript.host.v1`: policy-bound turn commands,
  labeled evidence references, stable event positions, and terminal receipts.
  Receipt validation rejects command/run/instance/policy mixups, while resource
  and provider inputs remain references rather than copied bodies or secrets.

## [0.2.0] — 2026-07-06

WhippleScript is a small scripting language for AI to orchestrate AI: a durable,
replayable rule/effect kernel with a scriptable surface. This release is the
language, its standard packages, and the native runtime — documented, tested, and
polished.

### Language & expression kernel
- Explicit `workflow` declarations with typed `input` / `output` / `failure`
  contracts; a compact single-line signature form desugaring to the same.
- Composition model: `include` (source bundles), `use` (package imports),
  `pattern` / `apply` (compile-time reuse, non-recursive), and durable child
  `invoke` with typed success/failure/timeout/cancellation projection.
- `flow` — a sequential surface that lowers to rules, with per-step `on fails` /
  `on timeout` handlers and branch liveness checks.
- A shared, typed expression kernel for guards and assertions: boolean logic,
  ordering, membership, `exists`/`empty`/`count`, presence proofs for optionals,
  finite-domain (enum / literal-union) checking, and fact/effect projection
  queries — with static diagnostics and generated per-program Maude checks.
- `case` pattern matching over enums, literal unions, optionals, tagged terminal
  outputs, and data-carrying sum-type variants, with exhaustiveness checking.

### Effects
- Agent turns (`tell`) with typed `AgentRef` routing and declared agent
  capability/profile/capacity enforcement.
- Schema coercion: named `coerce ... -> Type` and inline `decide`, plus a bare
  `prompt "..." -> text` free-text effect.
- Deterministic JSON/JSONL ingestion via `exec ... -> Type` / `-> each Type`.
- Capability-gated `exec` (operator allowlist) and content-pinned hosted
  `exec <name> with <record> -> Type` (`std.script`, hard-off without the import).
- Time: `timeout` on any effect, relative and absolute (`timer until`) timers,
  and source-level `cancel`.

### Standard packages
- **std.coord** — `lease` (incl. N-slot), `ledger`, `counter`; bounded
  `acquire ... wait`, `renew`, at-most-one-lease + lease-order deadlock safety,
  TLA-proven store protocols.
- **std.tracker** — durable issue/work tracker (queue projection, arbitrated
  claims).
- **std.messaging** — `channel` + outbound `send` (local mailbox and stdio
  providers) and inbound `Message` receive.
- **std.ingress** — typed `signal` admission and `source` observers (clock with
  recurrence, file, HTTP), plus `whip signal`.
- **std.memory** — named pools with `learn` / `recall` / `curate` and turn-scoped
  access grants, backed by a real file provider.
- **std.time** — timers, deadlines, `time` values, and the `clock` source.
- **std.files** — `read` / `write` / `import` / `export` with path policy and
  turn-scoped grants.
- **std.telemetry** — cursor-tracked OTLP export (`whip otel-export`),
  structural-by-default, failure-isolated, replay-safe.
- **std.coercion** — the schema-coercion backend contract (native structured
  outputs).

Packages resolve via signed lockfiles or embedded manifests (a `use`d standard
package works with no lock), a platform capability catalogue, and reserved-word
privileges — with no ambient authority.

### Information-flow control
- Static information-flow control across the workflow: session-root scoping,
  per-field producer-side flow signatures, `redact ... keep [...]`, typed effect
  failures (`fails as f`), and a hermetic Lean proof layer.

### Native providers & runtime
- Native **Codex** (app-server) and **Claude** (Agent SDK sidecar) providers,
  live-validated: lifecycle normalization to `agent.turn.*`, provider-native
  cancellation, artifact/evidence capture with redaction, and crash/restart
  recovery. A Pi (RPC) adapter is present in-tree but deferred (not part of 0.2).
- Durable SQLite runtime with event-sourced replay, workflow revision, and a
  worker/`dev` driver; deterministic fixtures for CI.

### Tooling
- `whip check` / `dev` / `worker` / `run` / `status` / `diagnostics` / `doctor`,
  `whip lint` (zero-false-positive analyses), `whip lsp`, `whip fmt`, and the
  `agents` / `providers` / `skills` introspection commands.

### Distribution
- `cargo install` source path, cargo-dist release artifacts for macOS / Linux /
  Windows with shell and PowerShell installers and checksums, and crates.io-ready
  packaging.

[0.3.0]: https://github.com/jamesjscully/whipplescript/releases/tag/v0.3.0
[0.2.0]: https://github.com/jamesjscully/whipplescript/releases/tag/v0.2.0
