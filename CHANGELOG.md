# Changelog

All notable changes to WhippleScript are recorded here. This project aims to
follow [Semantic Versioning](https://semver.org). Dates are UTC.

## [0.1.0] — Unreleased (date set at cut)

The first public release of WhippleScript — a small scripting language for AI to
orchestrate AI, built on a durable, replayable rule/effect kernel with a
scriptable surface. Safe to run by default; explicit, gated escape hatches for
external scripts and agents; LLM-driven control flow goes through coerce-typed
decisions. This release is the complete language, its standard library, native
and cloud runtimes, the owned agent harness, and WhippleScript's own version
control — documented, tested, and formally modeled.

### The language
- Explicit `workflow` declarations with typed `input` / `output` / `failure`
  contracts (and a compact single-line signature form); `include` source
  bundles, `use` package imports, non-recursive `pattern` / `apply` reuse, and
  durable child `invoke` with typed success/failure/timeout/cancellation.
- `flow` — a sequential surface that lowers to rules, with per-step `on fails` /
  `on timeout` handlers and branch-liveness checks.
- A shared, typed expression kernel for guards and assertions: boolean logic,
  ordering, membership, `exists` / `empty` / `count`, optional presence proofs,
  finite-domain (enum / literal-union) checking, and fact/effect projection
  queries — with static diagnostics and generated per-program Maude checks.
- `case` pattern matching over enums, literal unions, optionals, tagged terminal
  outputs, and data-carrying sum-type variants, with exhaustiveness checking.

### Effects
- Agent turns (`tell`) with typed `AgentRef` routing and declared
  capability/profile/capacity enforcement.
- Schema coercion — named `coerce ... -> Type`, inline `decide`, and a bare
  `prompt "..." -> text` free-text effect.
- Deterministic JSON/JSONL ingestion via `exec ... -> Type` / `-> each Type`.
- Capability-gated `exec` (operator allowlist) and content-pinned hosted
  `exec <name> with <record> -> Type` (`std.script`, hard-off without the import).
- Time: `timeout` on any effect, relative and absolute (`timer until`) timers,
  and source-level `cancel`.

### Standard library
Thirteen standard packages, each documented and store/TLA/Maude-tested; they
resolve via signed lockfiles or embedded manifests (a `use`d standard package
works with no lock), a platform capability catalogue, and reserved-word
privileges — no ambient authority.
- **std.coord** — `lease` (incl. N-slot), `ledger`, `counter`; bounded
  `acquire ... wait`, `renew`, at-most-one-lease + lease-order deadlock safety,
  TLA-proven store protocols.
- **std.tracker** — a durable, merge-friendly work tracker (see below).
- **std.messaging** — `channel` + outbound `send` (local mailbox / stdio) and
  inbound `Message` receive.
- **std.ingress** — typed `signal` admission and `source` observers (clock with
  recurrence, file, HTTP), plus `whip signal`.
- **std.memory** — named pools with `learn` / `recall` / `curate` and
  turn-scoped grants, over a real file provider (native and DO planes).
- **std.time** — timers, deadlines, `time` values, the `clock` source.
- **std.files** — `read` / `write` / `import` / `export` with path policy and
  turn-scoped grants.
- **std.telemetry** — cursor-tracked OTLP export (`whip otel-export`),
  structural-by-default, failure-isolated, replay-safe.
- **std.coercion** — the schema-coercion backend contract (native structured
  outputs).
- **std.script** — content-pinned hosted `exec` capabilities.
- **std.agent** / **std.web** — agent-turn and web-tool surfaces.

### Distributed work tracker (std.tracker phase B)
- The tracker's log is a content-addressed **Merkle event DAG** (SHA-256):
  every event is content-hashed over its kind/issue/payload/actor/parents, and
  an issue's identity is the hash of its creation event — tamper-evident and
  merge-stable, with `WS-N` kept as a durable clone-local alias.
- Multi-writer merge is a set-union of the content-addressed log deduped by
  event id; a **per-field conflict engine** surfaces disagreeing concurrent
  edits (a conflicted issue is not ready), resolved by a plain `set`. Optimistic
  concurrency via `--expect-state-token`.
- A full relation taxonomy (blocks / parent-of / related / duplicates /
  supersedes / discovered-from; only `blocks` gates readiness), comments, and
  evidence — each merge-stable.
- Cross-machine transport: the log serializes as content-addressed files, so two
  clones reconcile to a byte-identical frontier by sharing a folder (`whip issue
  export` / `import` / `sync`). The durable-object backend mints identical ids,
  so a DO log and a native log interoperate. Genuine duplicate submissions are
  surfaced; idempotent re-sync stays quiet.

### Native runtime & providers
- Durable SQLite runtime with event-sourced replay, workflow revision, and a
  `worker` / `dev` driver; deterministic fixtures for CI.
- Native **Codex** (app-server) and **Claude** (Agent SDK sidecar) providers,
  live-validated: lifecycle normalization to `agent.turn.*`, provider-native
  cancellation, artifact/evidence capture with redaction, crash/restart
  recovery. Providers are separable crates behind an open, string-keyed registry.

### Cloud runtime — Cloudflare Durable Object
- A sans-IO core (parser, kernel, rule/flow engine, effect ledger) runs inside a
  single-threaded wasm isolate where the only async primitive is `fetch`: every
  HTTP-bearing effect is a resumable step machine that suspends on a request and
  resumes on the response, surviving isolate eviction with no lost work.
- DO host binding over synchronous SQLite (a full port of the runtime /
  coordination / work-item / tracker stores), alarms for timers, secrets for
  credentials. `whip deploy` is a one-command edge deploy.
- Feature parity with native: `file.*` over a DO-owned file plane; `whip
  checkpoint` / `restore` as operator commands on a deployed instance; a real
  in-isolate tool set (read/write/edit/ls/find/grep/recall + tracker todos)
  against DO storage — no filesystem, no subprocess.
- A Class-A compute plane (`whip executor` sidecar over `whip-executor/1`, Bearer
  auth, loopback-only) and a Class-B per-turn container path are built and
  live-proven; production enablement is a follow-on configuration step.

### Owned agent harness
- A context layer: system-prompt assembler, a skills control plane (discover-all
  + model-driven read; skill bodies content-addressed; skills never grant
  authority), deploy-shipped project instructions (`AGENTS.md` / `CLAUDE.md`,
  injected verbatim), and turn-scoped skill pins.
- The `bash` tool runs in an in-isolate virtual shell (Bashkit) over the governed
  workspace VFS — no fork/exec, ambient filesystem, or ambient network — on both
  native and DO.
- Cache-aware conversation compaction: a pluggable `Compactor` (three strategies)
  keeps the assembled prefix append-only between compactions so the provider
  prompt cache is not needlessly busted; the summary is recorded once and reused
  on replay.

### Version control — the versioned workspace
WhippleScript gains its own version control: workspace-as-database with O(1)
branches over a content-addressed store, where an instance's files,
conversation, and effects move as one coherent, provenance-carrying line.
- Branches, cuts, and virtual working sets: O(1) branches, per-instance
  copy-on-write file surfaces, branch-distinct effect keys, materialize-on-exec.
- The mapped 13-operation workspace API (refusals as data), the op log as a
  first-class reflog with `whip branch undo-op`, review-grade Myers diffs,
  handoff bundles (`whipplescript.bundle.v1`) with chunk-granular delta transfer,
  and per-blob erasure discharging `HISTORY_PRESERVED` /
  `EXPORTED_COPY_NOT_RECALLED` by test.
- Selection algebra (`path()` / `by-effect()` / `since()` / `dependents-of()`
  with `| ~ &`) behind selective `undo` / `transport` / `adopt --only` — dry-run
  by default, stranding-checked, no destructive verbs.
- Structured conflicts with rerere-style resolution memory an auto-propagating
  reconciliation daemon; checkout-free `bisect`, `attribution`, `log`; and `whip
  fork` — the chat fork, seeding a new instance from a source's completed turns.

### Experimentation & improve
- `gauge` + `mark` (pin / suppose / settle / evidence / why) for ambient
  experimentation; identification-first quasi-experimental posture; an
  evidence-plane IFC (scope ⊥ clearance); `campaign` declarations; and the
  `improve` loop (holdout-validated, priced spend/park/resume, estimator +
  reopener) over parallel evaluation.

### Web & network access
- `web_search` (SearchProvider trait; Brave first-party, model-provider floor,
  honest absent tier) and `web_fetch` (structurally GET-only behind a central
  SSRF guard with pinned connections and redirect re-entry; HTML→markdown),
  granted via `with access to web { search fetch }`.
- `http source` fetches an external URL GET-only behind an SSRF/egress policy
  (http(s) only, private/loopback blocked, host allowlist).

### Policy plane, IFC, and the store seam
- Static information-flow control: session-root scoping, per-field producer-side
  flow signatures, `redact ... keep [...]`, typed effect failures (`fails as f`),
  and a hermetic Lean proof layer.
- Signed governance envelopes: hosts require an envelope, verify its attestation,
  and bind a policy epoch to the verified canonical hash and signer; a malformed
  envelope fails closed. `whipplescript.host.v1` publishes policy-bound turn
  commands, labeled evidence references, stable event positions, and terminal
  receipts (mixup-rejecting; resources/providers stay references, not copies).
- DR-0036 turn receipts carry a witnessed workspace cut (`workspace_cut_ref`,
  honest-decline when a segment is unwitnessed) and a dynamic guarantee section
  (`writes_within:<scope>` / `no_reads_beyond_grant` / `no_tainted_reads:<class>`)
  evaluated per turn under the cited policy epoch.
- Host-resolved provider profiles (`WHIPPLESCRIPT_PROVIDER_PROFILES`): the policy
  channel hands whip resolved credentials; whip's own auth is the thin fallback.
- The store seam: `whip handles` (stable pointers for external admission logs)
  and `whip checkpoint --external-positions` (position-pair cut for cross-store
  backup/handoff).

### Restorable context
- `whip checkpoint` / `restore`: rewind an agent's files, transcript, and
  event-log position to a prior point as one coherence-checked cut — content-
  addressed, refuses a partial cut, and auto-checkpoints head so the undo is
  itself undoable.

### Reliability
- Every provider request carries a stable per-effect `Idempotency-Key`
  (resume-stable), so an at-least-once retry after an eviction mid-request is
  de-duplicated by providers that honor it.

### Tooling
- `whip check` / `dev` / `worker` / `run` / `status` / `diagnostics` / `doctor`,
  `whip lint` (zero-false-positive analyses), `whip lsp`, `whip fmt`, and the
  `agents` / `providers` / `skills` introspection commands.

### Formal models & distribution
- A gate-registered Maude + TLA+ model suite (rule system, flow, coordination
  protocols, merge/conflict, workspace ops, turn witness, tracker readiness, …)
  with verified bites, plus per-program generated model checks.
- `cargo install` source path and cargo-dist release artifacts for macOS / Linux
  / Windows with shell and PowerShell installers and checksums.
