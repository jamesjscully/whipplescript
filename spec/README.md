# WhippleScript Specs

Status: design record and implementation tracker

These specs define the new WhippleScript design from first principles. The current
target is not a statechart language and not a general programming language. It
is a restricted event-sourced rule system for orchestrating agent work.

For user-facing documentation, start in [`../docs/README.md`](../docs/README.md).
The `docs/` directory is authoritative for stable user-facing behavior. The
specs remain design records, acceptance criteria, rationale, and implementation
trackers.

**For trackers, [`TRACKERS.md`](TRACKERS.md) is the single source of truth** —
which trackers exist, their scope, and whether each is active / closed /
archived. It is gate-enforced (`scripts/check-trackers.sh`): an unregistered
tracker fails the build. Register there before starting a new one, and check for
an existing scope first. The per-file links below are convenience pointers; the
registry is authoritative.

## Status Legend

| Status | Meaning |
| --- | --- |
| Normative design | Stable design intent that user docs should summarize for everyday use. |
| Implementation tracker | Checklist/history for building or auditing a feature; not a user contract. |
| Draft | Open design material that may change before it becomes user-facing behavior. |
| Historical | Preserved decision context; do not treat as current behavior without a link from `docs/`. |

When `docs/` and `spec/` appear to disagree, treat `docs/` plus the current
implementation as the user contract and update the stale spec or tracker.

## North Star

WhippleScript should let coding agents and humans write orchestration logic that is:

- explicit about when agent work is requested
- durable across crashes and restarts
- inspectable through an append-only history
- statically analyzable before it runs
- formally modelable with a small operational semantics
- safe to expose in enterprise environments through capability profiles

WhippleScript should not require authors to debug arbitrary distributed systems
control flow. The runtime owns delivery, effect queues, leases, idempotency,
timeouts, and replay. The language owns policy.

## Current Spec Set

- [core-scope.md](core-scope.md): what belongs in the kernel versus packages,
  libraries, and providers
- [construct-grammar.md](construct-grammar.md): draft baseline for controlled
  declarative library lowering and typed interfaces between package-owned
  constructs
- [construct-graph-calculus.md](construct-graph-calculus.md): draft formal
  baseline for package construct graph ports, edges, acceptance, and invariants
- [construct-lowering-preservation.md](construct-lowering-preservation.md):
  draft formal baseline for preserving accepted construct graph meaning during
  platform-owned lowering into ordinary core IR
- [package-management.md](package-management.md): implementation-grade target
  for local `whip.packages.json`, `whip.lock`, `whip package sync`, and package
  lock discovery
- [testing-strategy.md](testing-strategy.md): implementation-grade target for
  language, runtime, package-manager, standard-package, provider-boundary, and
  formal-verification tests
- [workflow-testing.md](workflow-testing.md): implementation-grade target for
  user-authored deterministic workflow scenario tests and package risk utility
  contracts
- [error-handling.md](error-handling.md): implementation-grade target for
  structured, source-local, package-aware, and provider-safe diagnostics
- [editor-tooling.md](editor-tooling.md): implementation-grade target for
  `whip lint`, `whip fmt`, `whip lsp`, package editor metadata, and IDE
  diagnostics
- [construct-interop-examples.md](construct-interop-examples.md): aspirational
  multi-package workflow examples used to pressure-test construct
  interoperability
- [decision-records/](decision-records/): product and language decision records,
  including the moved language ergonomics tracker and active workshop records
  for work tracking, provider compatibility, and package boundaries
- [architecture.md](architecture.md): system shape and component boundaries
- [kernel-api.md](kernel-api.md): deterministic runtime kernel operations and transaction boundaries
- [control-plane.md](control-plane.md): programs, instances, CLI, concurrent execution
- [runtime-store.md](runtime-store.md): durable store objects and transaction model
- [fact-provenance.md](fact-provenance.md): fact ownership, projection classes, and replay expectations
- [execution-contract.md](execution-contract.md): rule commits, effect graphs, dependencies, and completions
- [admission-and-idempotency.md](admission-and-idempotency.md): the single contract for how any value becomes a durable typed fact — admission identity/idempotency keys, runtime-boundary validation authority, record-once replay, typed fact-batch admission, exactly-once external effects
- [effects-and-capabilities.md](effects-and-capabilities.md): outbox effects, provider bindings, profiles
- [type-system.md](type-system.md): boundary types, schemas, validation, and schema-coercion backend mapping
- [expression-kernel.md](expression-kernel.md): pure guard/assertion expression semantics
- [expression-kernel-tracker.md](expression-kernel-tracker.md): implementation checklist for guard/assertion expression coverage
- [gherkin-lessons-tracker.md](gherkin-lessons-tracker.md): tracker for incorporating useful Gherkin/Cucumber lessons into WhippleScript authoring, reports, and validation without adopting free-text BDD
  execution
- [acceptance-fixtures.md](acceptance-fixtures.md): test-only JSON fixture
  format for running workflows through `dev` and validating final reports
- [reporting.md](reporting.md): current JSON report contracts for source
  metadata, validation summaries, construct/lowered artifacts, model-search
  ledgers, assertion filters, and table provenance
- [report-schemas/](report-schemas/): draft JSON Schema files for report
  envelopes, verified artifact bundles, package sets, package locks, package
  contracts, construct graphs, lowered IR reports, and standalone model-search
  obligation artifacts
- [workflow-composition-transition-tracker.md](workflow-composition-transition-tracker.md): transition checklist for `workflow`, `pattern`, `apply`, `invoke`, `include`, and explicit terminal actions
- [workflow-revision-transition-tracker.md](workflow-revision-transition-tracker.md): transition checklist for in-flight workflow revision, revision epochs, and cancellation policy
- [workflow-revision-followups-tracker.md](workflow-revision-followups-tracker.md): vNext planning tracker for root retargeting, live fact migration, provider cancellation depth, and destructive confirmation policies
- [native-provider-surfaces.md](native-provider-surfaces.md): validated Codex and Claude native integration surface notes
- [native-provider-implementation-tracker.md](native-provider-implementation-tracker.md): execution tracker for native provider capability/config, adapter spikes, cancellation, artifacts, recovery, and validation
- [capability-registry.md](capability-registry.md): runtime authority bindings and enforcement modes
- [plugin-system.md](plugin-system.md): legacy runtime provider-registry notes
  from the retired public plugin-system design
- [skills.md](skills.md): deterministic skill registry and attachment model
- [agent-harness.md](agent-harness.md): provider adapters for real agent turns
- [information-flow-surface.md](information-flow-surface.md): IFC source surface and
  governance grants — gradual labels on real resources, the `endorsed`/`declassify`
  crossings, the construct/boundary audit, and the provider-egress check (DR-0027/0028)
- [information-flow-governance.md](information-flow-governance.md): IFC governance
  lifecycle — IT drafts the DSL once, `gov compile` returns a guarantee report,
  signs (attestation); users author whips freely under enforcement with routes-to-fix
  errors; the signed envelope extends DR-0026 (DR-0028)
- [coerce.md](coerce.md): typed schema-coercion effects and backend/toolchain integration
- [event-ingress.md](event-ingress.md): `std.ingress`, typed `signal`
  admission, and external delivery providers
- [std-time.md](std-time.md): `std.time`, clock sources, recurring
  occurrence policy, and signal emission
- [messaging.md](messaging.md): `std.messaging`, communication channels,
  outbound sends, and generic inbound message envelopes
- [files.md](files.md): `std.files`, capability-scoped file stores,
  deterministic file/document I/O, format codecs, and bounded agent file grants
- [std-telemetry.md](std-telemetry.md): `std.telemetry`, read-side event/evidence
  export, OTLP provider scope, cursor policy, and structural-by-default content
  rules
- [human-review.md](human-review.md): historical/current human-review effect
  notes; target package design moves this use case under `std.messaging`
- [observability.md](observability.md): artifact/evidence store and status UX
- [quickstart.md](quickstart.md): human-facing CLI quickstart
- [operator-guide.md](operator-guide.md): store, profile, provider, and recovery operations
- [plugin-author-guide.md](plugin-author-guide.md): package/library/provider
  authoring; file path retained for old links
- [troubleshooting.md](troubleshooting.md): common diagnostics and operational failures
- [release-checklist.md](release-checklist.md): v0 release gate checklist
- [distribution-tracker.md](distribution-tracker.md): cross-platform install and release artifact tracker
- [documentation-improvement-tracker.md](documentation-improvement-tracker.md): product-oriented docs improvement tracker
- [final-audit.md](final-audit.md): staged audit findings and gap classification
- [memory-plugin.md](memory-plugin.md): historical memory plugin draft; current
  direction is `std.memory`
- [companion-skill.md](companion-skill.md): first-party skill for authoring WhippleScript workflows
- [language.md](language.md): author-facing rule language sketch
- [semantics.md](semantics.md): mathematical runtime model
- [static-analysis.md](static-analysis.md): compiler checks and restrictions
- [verification.md](verification.md): Maude, TLA+/Apalache, Veil, and static-analysis strategy
- [e2e.md](e2e.md): deterministic and opt-in real-provider e2e test guidance
- [implementation-plan.md](implementation-plan.md): staged project tracker from formal verification through e2e testing
- [examples.md](examples.md): early syntax sketches

## User-Facing Docs

- [../docs/README.md](../docs/README.md): documentation map and canonical terms
- [../docs/quickstart.md](../docs/quickstart.md): user-facing local
  quickstart
- [../docs/tutorial.md](../docs/tutorial.md): fixture-backed agent routing and
  review tutorial
- [../docs/concepts.md](../docs/concepts.md): core WhippleScript terms and
  command boundaries
- [../docs/examples.md](../docs/examples.md): checked examples by use case and
  credential requirements
- [../docs/current-state.md](../docs/current-state.md): what works today and
  what remains experimental
- [../docs/manual.md](../docs/manual.md): end-to-end workflow author/operator
  manual
- [../docs/api-reference.md](../docs/api-reference.md): CLI, language,
  runtime, JSON, and Rust API reference
- [../docs/language-reference.md](../docs/language-reference.md): practical
  `.whip` language reference
- [../docs/runtime-operations.md](../docs/runtime-operations.md): runtime
  lifecycle, provider failure capture, and inspection commands
- [../docs/providers.md](../docs/providers.md): fixture provider, experimental
  native providers, and package/provider entry points
- [../docs/troubleshooting.md](../docs/troubleshooting.md): first-10-minute
  setup and runtime fixes
- [../docs/runtime-operations.md](../docs/runtime-operations.md): user-facing operator
  command map
- [../docs/providers.md](../docs/providers.md): user-facing provider and
  package authoring orientation

## Design Commitments

1. Rules are restricted rewrites over typed facts, not arbitrary programs.
2. Effects are durable outbox records. They never execute inline.
3. Agent completions return as events/facts and are correlated by the runtime.
4. Rules may enqueue finite effect graphs with explicit dependency edges.
5. Source order never implies effect ordering.
6. Reusable `pattern` composition elaborates before runtime; recursive pattern
   composition is allowed only under analyzable, structurally bounded strata.
7. Effectful cycles must cross an external event, clock, or explicit durable
   boundary.
8. The compiler should be able to explain why a program is safe or rejected.
9. A source bundle plus selected root workflow compiles into a versioned
   program; each run is a durable instance managed by the control plane.
10. The core stays small: rule runtime, construct/package registries, provider
   harnesses, skills, schema-coercion effects, work tracking, signal admission,
   messaging, and observability only when the kernel must understand their
   lifecycle.
11. Memory, external trackers, browsers, research tools,
   dashboards, and evaluators start as packages/providers unless the kernel must
   understand them.
12. OpenClaw-lite is an example composition, not a product mode or language
    feature.
