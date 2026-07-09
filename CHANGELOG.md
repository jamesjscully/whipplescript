# Changelog

All notable changes to WhippleScript are recorded here. This project aims to
follow [Semantic Versioning](https://semver.org). Dates are UTC.

> 0.2 is the language, standard packages, and native runtime. Cloud deployment +
> the owned harness (0.3) and the experimentation/evals + versioned-workspace work
> (0.4) are tracked separately and are **not** part of 0.2. Native provider support
> is validated for **Codex and Claude**; the Pi native provider is deferred.

## Unreleased

### Embedding and governance
- The `whipplescript` package now publishes its governance and IFC trust boundary
  as a Rust library. Hosts can require a signed envelope, verify its attestation,
  and bind a policy epoch to the verified canonical hash and signer without
  reimplementing WhippleScript's policy parser or security algebra. A malformed
  configured envelope fails closed instead of becoming an ungoverned run.

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

[0.2.0]: https://github.com/jamesjscully/whipplescript/releases/tag/v0.2.0
