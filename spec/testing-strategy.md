# Testing Strategy

Status: implementation-grade target for language and platform testing

This document defines how WhippleScript itself should be tested during the next
implementation cycle. It covers language, runtime, package-manager, standard
package, provider-boundary, and formal-verification tests. User-authored
workflow scenario tests are specified separately in
[`workflow-testing.md`](workflow-testing.md).

## Goal

WhippleScript tests should prove that the platform preserves its core promises:

- source programs compile into deterministic typed IR
- package composition is checked through the construct graph and package lock
- accepted constructs lower into ordinary core IR without hidden semantics
- runtime behavior is event-sourced, replayable, and lifecycle-safe
- effects execute only through authorized provider boundaries
- standard packages cannot bypass core capability, lifecycle, or authority
  invariants
- reports and traces are stable enough for CI, artifact admission, and future
  debugging tools

The default test suite should be deterministic and local. Real providers,
destructive operations, external credentials, network services, and model-judged
quality checks are opt-in.

## Non-Goals

This strategy does not add:

- a `std.test` package
- package-defined test syntax inside workflow runtime semantics
- model-judged evals as required CI gates
- live provider credentials for the default test suite
- long-running soak tests as part of normal developer iteration
- compatibility guarantees for pre-release test fixture formats

Testing remains tooling around the language and runtime. It is not a workflow
authoring package.

## Test Layers

The platform test suite is layered. Lower layers should catch local mistakes
quickly; higher layers prove that the full source-to-runtime path still holds.

```text
unit tests
  -> parser/type/golden fixture tests
  -> static-analysis and diagnostic tests
  -> package-manager and package-contract tests
  -> construct graph and lowering artifact admission tests
  -> formal model checks
  -> runtime lifecycle and trace conformance tests
  -> deterministic acceptance fixtures
  -> opt-in real-provider smoke tests
```

Each layer has a different job. A bug should be covered at the lowest layer that
can express it precisely, then covered again at an end-to-end layer only when
integration risk justifies it.

## Unit Tests

Unit tests cover ordinary implementation logic with small, direct inputs.

Required areas:

- lexer and parser tokenization
- typed AST construction
- expression-kernel evaluation
- type unification and boundary-schema validation
- source-span diagnostic rendering
- package manifest and package-set schema parsing
- lock-file hashing, sorting, path normalization, and identity checks
- runtime store transactions
- event-log append and projection helpers
- effect dependency predicates
- capability registry lookup
- provider adapter argument construction
- report serialization helpers

Unit tests should not require Maude, TLA+, provider CLIs, network access, or
credentials.

## Source And IR Golden Tests

Golden tests prove that source-language behavior is deterministic and reviewed.

Required fixture classes:

- valid source parses into stable typed AST and IR snapshots
- invalid source produces stable diagnostics with source spans
- expression fixtures cover guards, assertions, projection queries, map/index
  access, arrays, objects, optional presence, `Missing` versus `null`, and finite
  domain comparisons
- package-backed source forms emit stable construct graph nodes, ports, edges,
  capabilities, and lowering-class requests
- accepted standard-package examples use the target syntax, not historical
  aliases

Golden output may change when the language changes, but changes must be
intentional and reviewed with the corresponding spec update.

## Static-Analysis Tests

Static-analysis tests prove that unsafe source is rejected before runtime.

Required negative cases:

- unknown schemas, fields, enum variants, capabilities, providers, and package
  imports
- guard and assertion type errors
- use of effect outputs outside their matching terminal branch
- implicit sibling-effect ordering assumptions
- effectful cycles without an accepted durable boundary
- unsatisfied package construct ports
- ambiguous construct resolution
- reserved keyword use without platform catalog privilege
- capability calls without declared provider/profile authorization
- package imports without a lock when a lock is required
- stale or identity-mismatched package locks
- package manifests that contradict their asserted contract

Every rejection should have an actionable diagnostic code and a focused test
that prevents the diagnostic from regressing into a generic failure.

## Package-Manager Tests

Package-manager tests cover local package intent, lock generation, and command
integration.

Required cases:

- `whip package sync` writes a deterministic lock from `whip.packages.json`
- `whip package sync --check-only` succeeds only when the lock is byte-identical
- package paths are project-relative and portable
- absolute paths, empty paths, `..`, and symlink escapes are rejected
- duplicate package names, package IDs, and exported library names are rejected
- declared `name`, `package_id`, and `version` constraints are checked against
  the manifest
- manifest SHA changes make the existing lock stale
- `check`, `compile`, `dev`, `run`, and `worker` discover `whip.lock` by default
- explicit `--package-lock` overrides discovery
- non-standard package imports fail clearly when no lock is available
- package sync and import do not grant provider authority

These tests should use local fixture manifests only. Registry, install, update,
publish, remote source, cache, signing, and environment behavior are deferred.

## Package-Contract Tests

Package-contract tests prove that package manifests describe only declarative
surfaces the platform can verify.

Required cases:

- package manifests validate against `package_manifest_v0`
- package contracts validate against `package_contract_v0`
- package-owned constructs name accepted construct families and lowering classes
- package-owned source forms are namespace-owned unless the platform catalog
  grants a reserved bare form
- required and provided ports use declared cardinality and typed compatibility
- package capability requirements are explicit
- provider bindings are described but not authorized by package import
- package contracts cannot assert checker-owned facts such as graph acceptance,
  lowering preservation, lifecycle acceptance, or capability closure

Negative fixtures should include contradictory contract inputs, not only missing
fields. The artifact validator should reject impossible combinations before the
formal models consume derived facts.

## Construct Graph Tests

Construct graph tests prove package composition before lowering.

Required cases:

- graph identity is deterministic for the same source, package lock, and
  platform catalog
- node and edge acceptance is derived from checker-owned facts
- cardinality is checked for exactly-one, optional-one, many, and named-many
  ports
- optional ports accept zero or one provider, but reject multiple providers
- many ports preserve deterministic ordering
- named-many ports preserve deterministic resource keys
- conflicting resolution facts are rejected even if a package-supplied artifact
  claims uniqueness
- unrelated package nodes do not affect graph meaning except through explicit
  ambiguity
- package composition cannot introduce new control flow, scheduling, lifecycle
  state, or authority

Hand-written Maude fixtures may remain for small edge cases, but generated
construct graph artifacts from real source should be the primary evidence.

## Lowering Tests

Lowering tests prove accepted construct graphs become ordinary core IR without
dropping or inventing semantics.

Required cases:

- every lowered core object has exactly one owner
- duplicate node/node, node/edge, edge/edge, and dependency ownership is rejected
- lowering class lifecycle acceptance is required before lowering preservation
  can succeed
- source declarations such as ingress sources and clocks lower to source or
  schedule policy templates, not emitted event occurrences
- package-backed capability calls lower to ordinary effect operations that still
  require runtime provider authorization
- edge dependencies lower to ordinary core dependency edges
- no lowering emits package-owned runtime lifecycle state
- no lowering emits unchecked provider runs, terminal completions, lease state,
  retry state, cancellation state, or trace evidence

Lowering tests should consume admitted construct graph and lowered IR reports
where possible. Hand-written model facts are acceptable only as scaffolding or
for sharply targeted negative cases.

## Formal Model Tests

Formal checks are part of the platform test suite, not a substitute for normal
tests.

Required Maude coverage:

- rule commit preconditions
- guard true/false/error behavior
- assertion failure/error non-mutation
- effect graph enqueue and dependency release
- package construct graph acceptance
- lowering preservation
- diagnostic adequacy for construct graph and lowering rejection paths
- runtime handoff boundary
- expression-kernel finite abstractions
- negative searches for known impossible states

Required TLA+/Apalache coverage:

- event-log append order
- projection cursor behavior
- claimability
- lease expiry and recovery
- retry and terminal status
- pause, resume, and cancel boundaries
- no provider run without a claimable effect
- no more than one successful terminal completion for an effect
- denied or non-authoritative runtime transitions leave durable diagnostics or
  evidence

Generated model checks should be preferred over hand-maintained fixtures when
the checked property depends on actual compiler output. Hand-maintained models
remain useful for validating the abstraction itself.

## Runtime Lifecycle Tests

Runtime lifecycle tests prove the event-sourced kernel and control plane behave
correctly without real providers.

Required cases:

- event append and replay produce the same projections
- rule commits are atomic
- false and error guards do not mutate workflow state
- assertion failures are reported without hidden state mutation
- effect dependencies block, release, or fail downstream work correctly
- claims and leases prevent duplicate provider starts
- stale completions are rejected or recorded as non-authoritative evidence
- retry policy reuses or creates effect identity according to the runtime
  contract
- pause prevents new effectful commits
- cancel prevents new work and handles in-flight effects according to policy
- crash/restart recovery resumes from the durable log, not in-memory state
- trace conformance catches lifecycle violations from raw store artifacts

These tests should use deterministic fixture providers and local stores.

## CLI And Report Tests

CLI tests prove product commands expose the same semantics as the libraries.

Required commands:

- `whip check`
- `whip compile`
- `whip dev`
- `whip run`
- `whip worker`
- `whip accept`
- `whip package check`
- `whip package sync`
- `whip package lock` while it remains as a low-level escape hatch
- `whip lint`
- `whip fmt`
- `whip lsp` through protocol fixtures
- `whip test` (the user scenario runner; itself a platform-tested command)
- `whip verify-report`
- trace inspection and trace conformance commands

Required report checks:

- every JSON report declares the expected schema identity
- report schemas validate through the repository schema gate
- success and failure reports are distinguishable without scraping text
- diagnostics include stable codes and source spans where source exists
- diagnostics follow [`error-handling.md`](error-handling.md): package-aware
  labels, provenance, safe suggestions, and redaction metadata where needed
- construct graph and lowered IR artifacts carry digest links
- verified artifact bundles are admitted before model searches consume them
- sensitive provider inputs are redacted in opt-in real-provider reports
- lint reports and LSP diagnostic fixtures preserve the same diagnostic codes,
  spans, provenance, suggestions, and fixits as CLI check/lint reports

CLI tests should favor JSON output. Text output tests should be reserved for
high-value user-facing diagnostics.

## Standard Package Conformance Tests

Each standard package needs package-level conformance tests before it is treated
as part of the platform.

Required shared checks for every standard package:

- the package manifest and contract validate
- the package imports through `whip.packages.json` and `whip.lock`
- accepted examples compile and emit expected construct graph/lowered IR
  artifacts
- unsupported source forms fail with package-owned diagnostics
- package-owned diagnostics are declarative metadata rendered through the
  platform diagnostic contract
- required capabilities are explicit
- importing the package does not grant provider authority
- package operations cannot bypass runtime lifecycle ownership

Package-specific minimum checks:

| Package | Required conformance focus |
| --- | --- |
| `std.tracker` | append-only issue events, projections, claim/readiness semantics, conflict diagnostics, external tracker adapter boundaries |
| `std.coord` | generic leases, ledgers, counters, release obligations, bounded waits, no mailbox semantics |
| `std.agent` | portable agent turn contract, feature-report taxonomy, turn-scoped grants, no provider-specific feature lies |
| `std.agent.codex` | Codex feature report across known CLI/app-server versions, slash-command handling, plugin/hook/subagent availability reporting |
| `std.agent.claude` | Claude SDK/CLI feature report, hooks/plugins/tool permissions, cancellation and artifact boundaries |
| `std.agent.pi` | Pi RPC feature report, `pi_variant` resolution, extension/tool availability reporting |
| `std.coercion` | unstructured input to structured output, schema validation, backend failure reporting, no workflow control-flow semantics |
| `std.messaging` | channel declarations, outbound `send`, inbound generic `Message`, one-way local/desktop providers, stdio request/reply transport boundaries |
| `std.ingress` | typed `signal` admission, CLI/HTTP/HTTPS/stdio/file/gRPC source providers, validation before admission |
| `std.time` | clock source declarations, recurring observations mapped to typed signals, no ambient current-clock guards |
| `std.script` | named pinned exec capabilities, hard-off enforcement, typed stdin/stdout, no prompt or payload smuggling |
| `std.memory` | named pools, explicit recall/learn, turn-scoped grants, curation operations, provider-owned indexing/distillation evidence |
| `std.files` | file stores, read/write/import/export, path policy, codecs, turn-scoped agent file grants |
| `std.telemetry` | read-only event/evidence export, cursor tracking, redaction policy, exporter failure isolation |

The table is a minimum. Concrete package specs remain authoritative for each
package's final behavior.

## Provider Boundary Tests

Provider tests are split into deterministic contract tests and opt-in real
smoke tests.

Deterministic provider tests use fixture providers and must cover:

- argument validation
- provider capability reports
- terminal success/failure/timeout projection
- evidence metadata shape
- artifact metadata shape
- cancellation handoff when the provider supports it
- unsupported-feature diagnostics when the provider does not support it
- redaction of secrets and credentials in reports

Opt-in real provider tests must:

- never run in default CI
- require explicit environment opt-in
- default to non-destructive operations
- require disposable-target acknowledgement for destructive checks
- write redacted preflight and result reports
- validate provider feature assumptions before running workflow behavior checks
- avoid treating model quality as a platform correctness signal

Real providers are useful for compatibility drift, not for proving core
orchestration semantics.

## Security And Authority Tests

Security tests are first-class platform tests.

Required cases:

- disabled `std.script` means no process execution through source, prompts,
  provider output, messages, files, memory, package manifests, or generic
  capability calls
- package import cannot grant provider credentials or runtime authority
- memory recall cannot grant capabilities
- message payloads and ingress payloads cannot select arbitrary files or
  providers
- path policies reject traversal, symlink escape, and mismatched store roots
- package manifests cannot smuggle executable hooks
- provider feature reports cannot expand granted capabilities
- stale or spoofed reports are rejected by artifact admission
- prompt text cannot mutate tracker, memory, files, or coordination state except
  through explicit authorized operations

These tests should include malicious fixtures, not only absent-permission
fixtures.

## Acceptance Fixtures

Acceptance fixtures are the deterministic end-to-end test surface for the
platform. They are JSON test inputs consumed by `whip accept`, not workflow
syntax.

Acceptance fixtures should cover:

- one representative happy path for each standard package
- one representative failure path for each standard package
- package interoperability examples that combine multiple standard packages
- package-manager lock discovery in real CLI paths
- replay and trace-conformance behavior for representative workflows
- source assertions, final projections, effect counts, provider runs, artifacts,
  evidence, and diagnostics

Acceptance fixtures should keep agent/provider behavior stubbed unless the
fixture is explicitly marked as a real-provider smoke.

## Evals

Agent evals are not platform correctness tests in v0.

Future evals may measure:

- whether an agent chooses the right workflow structure
- whether a provider preserves required facts in generated text
- whether schema coercion improves or degrades structured extraction
- whether memory recall improves task quality without polluting context
- whether companion skills guide agents toward valid WhippleScript

Those evals should be reported separately from deterministic CI. They may use
rubrics or judge models, but they cannot be the only evidence that core
orchestration, authority, package composition, or runtime lifecycle behavior is
correct.

## CI Profile

The default local and CI profile should run:

```sh
cargo test
scripts/check-formal-models.sh
scripts/check-report-schemas.sh
scripts/check-e2e.sh
```

After package management v0 lands, the default profile should also run:

```sh
whip package sync --check-only
```

Real-provider checks remain separate:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 scripts/check-real-providers.sh
```

CI should publish or retain reports for failures that are hard to debug from
stdout alone:

- acceptance reports
- trace conformance reports
- construct graph and lowered IR reports
- model-search ledgers
- real-provider preflight reports

## Coverage Ledger

The implementation should maintain a small coverage ledger that maps each
platform promise to at least one test or formal check.

Required ledger columns:

```text
promise
spec reference
test layer
fixture or test name
command
status
notes
```

The ledger is not a coverage-percentage system. Its purpose is to prevent major
design promises from existing only in prose.

## Acceptance Criteria

The language/platform testing strategy is implemented when:

- every current standard package spec has at least one deterministic conformance
  fixture
- package sync and lock discovery are covered by CLI tests
- package manifests, package sets, locks, contracts, construct graphs, lowered
  IR reports, acceptance reports, and verified artifact bundles pass schema
  validation in CI
- construct graph and lowering tests consume generated artifacts from real
  source for at least one package-backed workflow
- negative tests cover reserved words, ambiguous construct resolution,
  cardinality mismatch, duplicate lowering ownership, stale locks, authority
  gaps, path escape, and script hard-off enforcement
- runtime lifecycle tests cover dependency release, claims, leases, retries,
  pause, cancel, restart recovery, and trace conformance
- deterministic acceptance fixtures cover representative happy and failure
  paths for core orchestration and standard package interoperability
- lint and LSP fixtures prove editor tooling reuses compiler diagnostics,
  package metadata, formatter edits, and safe code actions without changing
  source validity or runtime authority
- real-provider tests are opt-in, redacted, non-destructive by default, and not
  required for ordinary CI
- model-judged evals, if present, are reported separately from deterministic
  platform correctness

## Deferred User Testing Design

The user-facing test surface is now specified in
[`workflow-testing.md`](workflow-testing.md). The important boundary remains:
platform tests prove runtime/package invariants, while user scenario tests prove
workflow intent against deterministic givens, stubs, runs, and expectations.
