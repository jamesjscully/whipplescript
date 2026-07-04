# Standard Package Design Tracker

Status: active — ecosystem shape SETTLED 2026-07-04 (see
[std-package-ecosystem-shape.md](../std-package-ecosystem-shape.md), status
feeding-ADR; two forks ⚑-flagged for Jack: M1 meta-grammar deferral, M5
embedded std manifests). Process steps 1–5 satisfied; current gate = step 6
(concrete per-package designs), then step 7 (implementation slices).

## Purpose

Track the pre-implementation design pass for WhippleScript standard packages.
This file is deliberately lightweight. It should help the design discussion stay
ordered without turning provisional thoughts into settled decisions too early.

## Process

1. Review each proposed standard package one by one.
2. For each package, identify the core functionality currently imagined for it.
3. Decide whether that functionality is actually worth building as part of
   WhippleScript.
4. Record the target feature set we feel good about for that package.
5. After all packages are reviewed, step back and reconsider the overall package
   ecosystem shape and organization.
6. Only after the ecosystem shape is settled, design each package concretely.
7. Only after concrete package designs are settled, start implementation work.

Implementation should not drive this pass. Existing code is useful evidence, but
not a commitment to keep a package, feature, name, or boundary.

## Review Checklist

For each package, capture:

```text
core functionality
why this belongs in WhippleScript, if it does
what should not be in the package
target feature set
dependencies on core constructs or other packages
provider/integration expectations
open naming or boundary questions
decision: keep / merge / split / defer / drop
```

## Package Inventory

| Package | Design review status | Current question |
| --- | --- | --- |
| `std.tracker` | Core concept accepted | Use issues as the source of orchestrated work; refine claim/readiness/CLI shape before implementation. |
| `std.agent` | Core concept accepted | Shared feature-report contract; Codex, Claude, and Pi split into provider-specific packages with native semantics. |
| `std.agent.codex` | Core concept accepted | App Server/SDK boundary, Codex-native slash commands/plugins/hooks/subagents reported as features. |
| `std.agent.claude` | Core concept accepted | Claude Agent SDK sidecar boundary, tool/permission policy, hooks/plugins/subagents reported as features. |
| `std.agent.pi` | Core concept accepted | Pi RPC boundary plus `pi_variant` package/extension sets and runtime command/tool discovery. |
| `std.messaging` | Core concept accepted | Communication channels, outbound `send`, generic inbound `Message`; human review is a use case, not a separate package. |
| `std.memory` | Core concept accepted | Memory pools, explicit recall/learn, turn-scoped access grants, and policy-driven curation. |
| `std.time` | Core concept accepted | Clock source provider for recurring typed signals; core keeps one-shot timers/timeouts. |
| `std.ingress` | Core concept accepted | External `source` providers and typed `signal` admission; messaging can feed it explicitly. |
| `std.files` | Core concept accepted | Capability-scoped file stores, deterministic format codecs, read/write/import/export effects, and turn-scoped agent file grants. |
| `std.script` | Core concept accepted | Narrow named, pinned `exec` capabilities; hard-off must be prompt-injection resistant. |
| `std.coord` | Core concept accepted | Closed package family: generic leases, ledgers, counters; no arbitrary shared store. |
| `std.telemetry` | Core concept accepted | Read-only event/evidence-log exporters; operator/provider surface, not workflow syntax. |
| `std.coercion` | Core concept accepted | Core owns `coerce`/`decide`; package owns schema-coercion backend/toolchain support. |

## Meta Questions

ANSWERED 2026-07-04 — each question maps to a decision in
[std-package-ecosystem-shape.md](../std-package-ecosystem-shape.md):
names → "Names" (E1); domains vs catalogs → E2; authoring vs operator-config
→ E3; missing lowering classes → E4; bundled-but-imported → E5;
merge/split/defer/drop → E6 (std.agent.pi deferred name-reserved; std.test
stays dropped; no new merges/splits); package-vs-core line → E7. The
cross-cutting mechanism answers (meta-grammar, provider seam, capability
planes, renames, std-as-manifest, versions, DO plane, static checks) are that
note's M1–M8.

## Current Rule

No new implementation commitments from this pass until:

```text
package-by-package review complete        [x] (inventory rows + Current Notes)
overall ecosystem shape settled           [x] 2026-07-04 → std-package-ecosystem-shape.md
concrete package designs written          [ ] ← current gate
implementation slices chosen from those designs   [ ]
```

## Current Notes

- `std.tracker` owns issue-domain claims. Source examples should bind a claim
  handle such as `active_claim`, not a generic `lease` variable.
- `std.tracker` state is append-only accepted changes plus projections:
  commands request changes, events record accepted changes, and projections
  answer ready/current/history/conflict queries.
- Generic leases belong to `std.coord`; core provider-run leases remain runtime
  internals.
- `std.coord` owns generic `lease`, `ledger`, and `counter` resources. It is a
  privileged standard package because release obligations, bounded waits,
  counter caps, and ledger retention require platform checks.
- `std.coord` should not define a source-facing mailbox concept. A ledger may be
  partitioned by an explicit recipient key for internal tuple-space/barrier
  patterns, but communication mailboxes/channels belong to `std.messaging`.
  Current-instance identity, if needed later, is a core instance-identity design,
  not an incidental `std.coord` feature.
- `std.coercion` is the concept. coerce is a backend/provider implementation for
  schema coercion, not the name of the standard package and not workflow
  decision semantics.
- `std.agent` is a shared boundary package, not a monolithic harness bundle.
  Codex, Claude, and Pi belong in separate provider packages. The portable layer
  is a feature taxonomy and truthful capability report, while native slash
  commands, hooks, plugins, extensions, sessions, and subagents remain
  provider-specific.
- Pi installed extension/package sets should be modeled as `pi_variant`; reserve
  `environment` for a future package-manager or deployment-environment concept.
- Reserved bare words such as `claim`, `renew`, and `release` are platform
  catalog privileges. Package manifests may use them only when the platform
  catalog grants that exact library, construct family, scope, and lowering
  class. (Corrected 2026-07-04: `lease` was listed here but has no privilege
  row — it is a core declaration keyword, not a granted bare word; the shipped
  privilege rows are exactly claim/renew/release → std.tracker,
  core/lib.rs reserved_keyword_privileges.)
- Standard package specs should reuse the shared abstraction vocabulary:
  declared resources, source declarations, effect operations, projections,
  turn access grants, provider capability reports, typed signal admission, and
  no ambient authority. Resource declarations should prefer block-internal
  `provider` clauses over one-off header syntax.
- `std.messaging` replaces the old `std.human` / `std.inbox` conceptual
  package. It owns `channel`, outbound `send`, generic inbound `Message`
  envelopes, and explicit `source interaction` mappings for authenticated
  provider callbacks into typed signals. `send` interaction clauses configure
  provider UI/correlation only; they do not emit typed signals. The package
  should not implicitly parse natural language into domain values or hide a
  request/reply lifecycle. Initial providers are local mailbox, desktop
  notification, and stdio only; Slack, GitHub comments, email, and similar
  integrations are deferred.
- `std.ingress` owns typed `signal` admission. The old source word `event` is
  overloaded with the runtime event log and should be replaced in the target
  design. Ingress is lower-level than messaging and can produce arbitrary
  typed signal payloads only because the workflow declares a validation
  contract. Initial providers are CLI, HTTP/HTTPS, stdio, file, and gRPC;
  broker/topic adapters are deferred.
- `std.time` owns the `clock` source provider. Recurrence is modeled as
  clock observations explicitly mapped into typed signals; core keeps one-shot
  timers, timeouts, `time`, duration, cancellation, and the no-current-clock-in-
  guards invariant.
- `std.script` is a narrow process-execution package, not a plugin system.
  Disabled means truly disabled: no workflow source, agent prompt, provider
  output, messaging payload, or generic capability call may smuggle process
  execution through scripts. Enabled execution requires named manifest
  capabilities, pinned bytes, typed stdin/stdout, capability authorization, and
  evidence.
- Implementation cleanup to keep on the porting list: remove the retired
  `notify` / `event.notify` parser, CLI, runtime, generated IR, and tests once
  `signal`, `source`, and `signal.emit` are implemented. The target docs no
  longer specify a compatibility alias.
- `std.telemetry` owns read-only export of the durable event/evidence log. It is
  surfaced through operator config, environment variables, provider bindings,
  and CLI commands such as `whip otel-export`, not through workflow rule syntax.
  Initial provider target is OTLP/OpenTelemetry. Exporters are cursor-tracked,
  structural-by-default, failure-isolated, and replay-safe. The canonical package
  contract is `spec/std-telemetry.md`; `spec/observability.md` remains the
  substrate/rationale document.
- `std.test` is dropped as a standard package. Workflow testing, acceptance
  fixtures, package conformance, provider fixtures, and future evals remain
  important, but they belong to tooling/design tracks rather than the runtime
  package ecosystem for now.
- `std.memory` owns named memory pools, not ambient model memory. The source
  surface is `memory pool`, `recall from`, `learn from ... into`, turn-scoped
  `with access to <pool> { recall ... learn ... }` grants, `curate`, `keep`,
  and `forget`. Search indexes, embeddings, and distillation mechanics are
  provider/policy details surfaced through evidence and curation projections.
- `std.files` owns deliberate file/document I/O. Source declares `file store`
  resources with read/write path policy, then uses `read`, `write`, `import`,
  and `export` effects. `std.ingress.file` only observes file arrivals/changes;
  `std.files` reads and writes content. Agent file access uses explicit
  turn-scoped `with access to <file-store> { read [...] write [...] }` grants.
