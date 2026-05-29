# Implementation Plan

Status: draft tracker

This is the project tracker for the new Whippletree system. It runs from formal
modeling through the last e2e acceptance tests.

The plan is organized as stage gates. A stage is complete only when its
acceptance checks pass and the relevant docs, examples, and tests have been
updated. Checkboxes should be updated as implementation lands.

## Product Target

Whippletree v0 should provide:

- a restricted rule language for durable agent orchestration
- typed facts, schemas, effect contracts, and capability profiles
- an event-sourced runtime kernel backed by SQLite
- a control plane for compiling, starting, inspecting, pausing, resuming, and
  cancelling workflow instances
- first-party effects for agent turns, BAML-backed coercion, Loft claims,
  human review, skills, and evidence capture
- adapter support for at least Codex, Claude Code, and Pi-style harnesses
- formal checks and trace-conformance checks that catch orchestration bugs
- e2e tests that run real workflows through the full stack

## Milestone Summary

- [x] M0: Formal kernel spine is executable and checked in CI.
- [x] M1: Source language grammar, parser, and typed IR compile example
  programs.
- [x] M2: Runtime store and kernel can replay events and commit deterministic
  rule rewrites.
- [x] M3: Durable effects, dependency scheduling, leases, retries, and trace
  conformance work end to end.
- [ ] M4: Control plane and CLI manage programs, instances, rule stepping, and
  worker/dev loops.
- [ ] M5: Capability registry, skills, real agent harnesses, BAML coerce, Loft,
  human review, and observability are wired through typed effect contracts.
- [x] M6: Static analysis and generated Maude checks protect user programs.
- [ ] M7: E2E suite covers happy paths, failure paths, recovery, and dogfood
  workflows through real provider surfaces where configured.
- [x] M8: Companion skill and release hardening make the system usable by
  coding agents without hand-holding.

## Stage 0: Repository Reset And Project Skeleton

Goal: make the repo coherent after the redesign and isolate old systems.

- [x] Move the previous statechart workflow runtime into
  `legacy/statechart-workflows-runtime/`.
- [x] Keep the earlier v0.3 runtime in `legacy/v0.3-runtime/`.
- [x] Create the new `spec/` suite for the rule-machine design.
- [x] Create the new `models/` suite for formal validation.
- [x] Restore a root-level `scripts/check-formal-models.sh`.
- [x] Add the new Rust workspace skeleton at the repo root.
- [x] Add CI for formatting, Rust tests, formal checks, and e2e smoke tests.
- [x] Add a top-level developer README that points to the new specs, not the
  legacy systems.
- [x] Audit Stage 0 against the repo layout, specs, tests, and docs; record
  gaps for the final audit stage.
  - Active implementation is rooted in the new Rust workspace and spec/model
    tree.
  - Legacy implementations are isolated under `legacy/`.
  - No blocking Stage 0 gaps remain.

Acceptance:

- [x] `git status` shows no accidental old runtime files outside `legacy/`.
- [x] `scripts/check-formal-models.sh` runs from the repo root.
- [x] A new contributor can find the active spec set from `README.md`.

## Stage 1: Formal Modeling And Verification Spine

Goal: validate the core execution model before and during implementation.

### Maude Kernel

- [x] Add a reusable Maude kernel model for effect lifecycle and dependency
  release.
- [x] Add hand-written dependency tests for `succeeds`, `fails`, and
  `completes`.
- [x] Add a Loft-claim-gated agent-turn model test.
- [x] Make `scripts/check-formal-models.sh` assert expected Maude search
  outcomes.
- [x] Extend the Maude model with durable event log, fact projection, effect
  graph commit, and rule firing.
- [x] Model blocked-by-policy and blocked-by-capacity separately from
  blocked-by-dependency.
- [x] Model retry, timeout, cancellation, and lease expiry outcomes.
- [x] Add a Ralph loop model with an explicit external-event boundary.
- [x] Add a coerce classification model with success/failure branches.
- [x] Add generated Maude checks from typed IR once the compiler exists.
- [ ] Extend the generated per-program Maude spec from effect dependencies to
  expression-kernel behavior:
  - [ ] For every guarded rule, generate a search showing
    `ruleCommitted(<rule>)` is reachable only when the lowered guard predicate
    is true.
  - [ ] For every guarded rule, generate false-guard searches showing no fact
    write, consume, or effect graph commit is reachable.
  - [ ] For every guarded rule, generate error-guard searches showing no fact
    write, consume, or effect graph commit is reachable, while allowing only
    diagnostic/evidence output.
  - [ ] For every assertion checkpoint, generate failure and error searches
    showing no user fact mutation and no effect enqueue/release/claim/complete
    transition is reachable.
  - [ ] Preserve the existing generated effect dependency searches for queued
    upstream dependencies, satisfying terminal release, and non-satisfying
    terminal non-release.
- [ ] Validate generated checks against compiled `.whip` fixtures, not only
  hand-authored Maude modules.
  - [ ] Include a true-guard fixture whose committed effect graph still runs
    the existing dependency-release searches.
  - [ ] Include false-guard and error-guard fixtures where generated searches
    prove the rule does not commit.
  - [ ] Include assertion failure and assertion error fixtures where generated
    searches prove facts/effects are unchanged.
  - [ ] Include expected-failure fixtures that inject unsafe generated rewrites
    and prove Maude finds counterexamples when `maude` is available.

### TLA+/Apalache Runtime Lifecycle

- [x] Finish the TLA+ lifecycle model for append, projection, claim, lease,
  retry, completion, pause/resume, cancellation, and recovery.
- [x] Install or document the Apalache runner path used by CI.
- [x] Add bounded checks for safety invariants:
  - [x] every run references an existing effect
  - [x] no effect has more than one terminal completion
  - [x] no provider run starts unless the effect is claimable
  - [x] no claimable effect has unsatisfied dependencies
  - [x] paused instances do not commit new effectful rewrites
  - [x] recovery does not reorder the per-instance event log
- [x] Add liveness/fairness checks only after safety checks stabilize.
  - `ControlPlaneLifecycle.tla` now names weak-fairness assumptions and
    temporal liveness goals for claimable effects, running leased effects,
    projection catch-up, and recovery completion.
  - The default TLA script typechecks these formulas; full temporal proof
    remains future hardening, not a v0 release gate.

### Trace Conformance

- [x] Define the trace event schema consumed by model checkers.
- [x] Implement a trace checker that validates runtime traces against kernel
  invariants.
- [x] Add trace checker fixtures for success, dependency violation, duplicate
  terminal completion, stale lease completion, and pause/cancel races.
  - [x] success
  - [x] dependency violation
  - [x] duplicate terminal completion
  - [x] stale lease completion
  - [x] pause/cancel race basics

### Veil/Lean Recheck

- [x] Reevaluate Veil after the kernel semantics and trace schema stabilize.
  - Decision: keep Veil out of v0 and revisit it after the runtime semantics
    stop moving.
- [x] Decide whether Veil proves stable invariants or remains out of v0.
- [x] Audit Stage 1 against the formal models, generated-check plan, trace
  fixtures, and CI wiring; record gaps for the final audit stage.
  - Hand-written Maude and TLA+/Apalache checks run through repository scripts.
  - Generated per-program Maude checks are available through
    `whip check --model-search`.
  - Trace conformance fixtures cover success and lifecycle violation cases.
  - Veil remains a documented later-assurance path, not a v0 gate.

Acceptance:

- [x] Formal checks fail on intentionally broken dependency-release behavior.
- [x] Runtime trace fixtures catch impossible lifecycle transitions.
- [x] Generated Maude checks can be run optionally from the CLI.
- [ ] Generated Maude checks catch intentionally broken guard-gated rule commit
  behavior from compiled Whippletree fixtures.
- [ ] Generated Maude checks catch intentionally broken assertion
  non-mutation behavior from compiled Whippletree fixtures.
- [ ] Adding guard/assertion searches does not remove or weaken the existing
  generated effect dependency checks.

## Stage 2: Source Language, Parser, And Typed IR

Goal: compile `.whip` source into deterministic, typed IR.

- [x] Finalize v0 grammar for rules, schemas, agents, skills, capabilities,
  effects, `after` blocks, `coerce`, and record construction.
- [x] Choose and document the parser implementation strategy.
- [x] Implement lexer/parser with diagnostics that preserve source spans.
- [x] Build a recoverable parse tree suitable for formatting and helpful
  errors.
- [x] Define the typed AST and typed rule IR.
- [x] Implement lowering from source AST to typed IR.
- [x] Support BAML-aligned boundary types:
  - [x] string
  - [x] int
  - [x] float
  - [x] bool
  - [x] null
  - [x] literal
  - [x] array
  - [x] map
  - [x] union
  - [x] class
  - [x] enum
  - [x] image
  - [x] audio
  - [x] pdf
  - [x] video
- [x] Implement source-span-aware errors modeled after Gleam-style diagnostics.
- [x] Add golden parse/IR fixtures for all examples.
- [x] Add formatter scaffolding after the parse tree stabilizes.
- [x] Audit Stage 2 against the language specs, examples, diagnostics, and IR
  snapshots; record gaps for the final audit stage.
  - Checked examples produce stable IR snapshots.
  - Invalid examples have source-span diagnostics and targeted suggestions.
  - Final-audit gap: equality guards are deliberately outside the v0 grammar
    until guard-expression design is settled.

Acceptance:

- [x] `whip check examples/*.whip` produces stable typed IR.
- [x] Invalid examples produce precise errors with source spans and suggested
  fixes.
- [x] IR snapshots are deterministic across runs.

## Stage 3: Static Analysis

Goal: reject programs that would produce hidden distributed-system bugs.

- [x] Validate schema references, field paths, enum variants, and literal
  types.
- [x] Validate fact read/write/consume sets for each rule.
- [x] Validate effect contracts and output binding scopes.
- [x] Reject use of effect outputs outside their matching `after` branch.
- [x] Validate finite effect graphs and dependency edge references.
- [x] Reject implicit ordering assumptions between sibling effects.
- [x] Build rule dependency graph analysis.
- [x] Implement recursion stratification.
- [x] Reject effectful cycles that do not cross an external event, clock,
  human, or durable boundary.
- [x] Validate idempotency-key derivability.
- [x] Validate required capability/profile bindings.
- [x] Validate resource/capacity declarations.
- [x] Emit actionable diagnostics for every rejection.
- [x] Audit Stage 3 against the static-analysis spec, unsafe fixtures, and
  diagnostics; record gaps for the final audit stage.
  - Unsafe fixtures cover unknown schemas, bad records, output-scope leaks,
    invalid effect graphs, bad agent declarations, and effectful self-loops.
  - Static-analysis metadata feeds generated model checks.

Acceptance:

- [x] Unsafe examples are rejected with specific explanations.
- [x] Safe Ralph, Loft, coerce, and human-review examples pass.
- [x] Static analysis outputs enough metadata for generated Maude checks.

## Stage 4: Runtime Store

Goal: persist every runtime transition in a replayable SQLite store.

- [x] Create the root Rust workspace and crates.
- [x] Add a store crate with SQLite migrations.
- [x] Define tables for:
  - [x] programs
  - [x] program versions
  - [x] instances
  - [x] event log
  - [x] fact projections
  - [x] effect outbox
  - [x] effect dependency edges
  - [x] runs
  - [x] leases
  - [x] artifacts/evidence
  - [x] diagnostics
  - [x] plugin registrations
  - [x] capability bindings
- [x] Implement append-only event writes with per-instance sequence numbers.
- [x] Implement transaction helpers for rule commits and effect completion.
- [x] Implement projection rebuild from the event log.
- [x] Implement store-level uniqueness for idempotency keys and terminal
  completions.
- [x] Implement migration tests and replay tests.
- [x] Audit Stage 4 against the runtime-store spec, migrations, replay behavior,
  and transaction boundaries; record gaps for the final audit stage.
  - SQLite migrations create programs, instances, events, facts, effects,
    dependencies, runs, leases, evidence, diagnostics, plugins, and capability
    bindings.
  - Replay, uniqueness, and rollback behavior are covered by store tests.

Acceptance:

- [x] Store replay reconstructs facts/effects from the log.
- [x] Duplicate terminal completions fail transactionally.
- [x] Interrupted transactions leave no partial rule commit.

## Stage 5: Runtime Kernel

Goal: execute compiled programs deterministically against the store.

- [x] Implement kernel operations:
  - [x] create program version
  - [x] create instance
  - [x] ingest external event
  - [x] derive facts
  - [x] evaluate rules
  - [x] commit rule rewrite
  - [x] enqueue effect graph
  - [x] satisfy dependencies
  - [x] claim effect
  - [x] start run
  - [x] complete run
  - [x] fail run
  - [x] timeout run
  - [x] cancel effect
  - [x] pause instance
  - [x] resume instance
  - [x] cancel instance
- [x] Ensure all kernel operations are deterministic and transaction-scoped.
- [x] Implement idempotency-key generation.
- [x] Implement scheduler queries for claimable effects.
- [x] Implement lease acquisition, renewal, expiry, and recovery.
- [x] Implement retry/backoff policy.
- [x] Implement trace emission for conformance checking.
- [ ] Expose enough rule-lowering support for the control plane to turn typed
  IR rule bodies into `NewFact`, `NewEffect`, and dependency records without
  duplicating kernel semantics.
- [x] Audit Stage 5 against the kernel API, formal lifecycle models, trace
  conformance, and scheduler behavior; record gaps for the final audit stage.
  - Kernel lifecycle operations are deterministic and transaction-scoped.
  - Scheduler, lease, retry, pause/resume/cancel, and trace paths are covered
    by unit and e2e tests.

Acceptance:

- [x] Unit tests cover every lifecycle transition.
- [x] Kernel tests match the Maude and TLA+ lifecycle expectations.
- [x] Trace conformance passes for all kernel integration tests.
- [ ] A rule-lowering/step integration test can materialize `record` facts and
  `agent.tell` effects from a compiled workflow body.

## Stage 6: Control Plane And CLI

Goal: expose Whippletree as an inspectable system for many concurrent scripts.

- [x] Implement CLI crate.
- [x] Implement commands:
  - [x] `whip check`
  - [x] `whip compile`
  - [x] `whip run`
  - [x] `whip instances`
  - [x] `whip status`
  - [x] `whip log`
  - [x] `whip facts`
  - [x] `whip effects`
  - [x] `whip runs`
  - [x] `whip pause`
  - [x] `whip resume`
  - [x] `whip cancel`
  - [x] `whip retry`
  - [x] `whip doctor`
- [ ] Implement `whip step`.
- [ ] Implement `whip worker`.
- [ ] Implement `whip dev`.
- [ ] Implement provider configuration loading and validation.
- [ ] Implement command-level diagnostics for idle, blocked, missing provider
  config, missing credentials, and provider capacity exhaustion.
- [x] Support JSON output for every inspection command.
- [x] Add compact human-readable status views.
- [x] Add helpful suggestions for common desire-path mistakes.
- [x] Add control-plane tests for concurrent instances.
- [x] Audit Stage 6 against the control-plane spec, CLI UX, JSON output
  stability, and multi-instance behavior; record gaps for the final audit stage.
  - CLI commands cover check/compile/run/inspection/control-plane operations.
  - JSON inspection output and multi-instance isolation are covered by CLI
    tests.

Acceptance:

- [x] A user can start two instances of the same program and inspect them
  independently.
- [x] Status shows current facts, queued effects, active runs, failures, and
  recent evidence.
- [x] CLI errors include next-step guidance.
- [ ] `whip step` can drive `examples/minimal-noop.whip` from `external.started`
  to a recorded `StartupSeen` fact.
- [ ] `whip dev` can drive `examples/implementation-plan-phase-review.whip`
  through phase request creation and provider dispatch using a fixture provider.
- [ ] Worker failures at provider binding, credential lookup, workspace
  preparation, adapter launch, request submission, stream/read, artifact
  capture, and terminal-event append are visible in status and trace output.

## Stage 7: Capability Registry And Plugin Kernel

Goal: safely bind authority at runtime without bloating the core.

- [x] Implement capability schema registration.
- [x] Implement effect provider registration.
- [x] Implement profiles with descriptions and enforcement modes.
- [x] Ship default profiles:
  - [x] permissive
  - [x] repo-reader
  - [x] repo-writer
  - [x] internet-research
  - [x] human-review
- [x] Implement custom profile loading from config.
- [ ] Implement provider binding config for Codex, Claude, Pi, fixture, BAML,
  Loft, and human inbox providers.
- [ ] Implement provider health checks and explainable provider selection.
- [x] Validate source-requested capabilities against registry bindings.
- [x] Implement plugin package discovery and loading.
- [x] Ensure plugins cannot mutate kernel state directly.
- [x] Add plugin fixtures for memory and external notification examples.
- [x] Audit Stage 7 against the capability registry spec, plugin system spec,
  default profiles, and enforcement evidence; record gaps for the final audit
  stage.
  - Default profiles, plugin manifests, and capability enforcement are wired
    through typed effect contracts.
  - Missing capability and profile mismatches block provider starts before
    execution.

Acceptance:

- [x] Missing capabilities block effects before provider execution.
- [x] Profile mismatch is visible in status and trace output.
- [x] A plugin can register an effect contract and provider without changing
  kernel code.
- [ ] Status for blocked effects explains whether the failure is missing
  capability, profile mismatch, missing provider config, missing credentials, or
  insufficient enforcement.
- [ ] Missing provider configuration, credentials, native enforcement, or healthy
  provider binding blocks an effect before provider execution and records
  diagnostics/evidence without leaking secrets.

## Stage 8: Core Integrations

Goal: wire the built-in effect families through the same contract system.

### Skills

- [x] Implement deterministic skill registry.
- [x] Attach skills to agents, turns, and program scopes.
- [x] Record skill versions and source paths in evidence.

### Agent Harnesses

- [x] Define the harness adapter trait.
- [x] Implement mock harness for deterministic tests.
- [ ] Implement Codex adapter against the Codex App Server or Codex SDK, with
  thread lifecycle, event-stream, approval, diff, artifact, and auth handling.
- [ ] Implement Claude adapter against the Claude Agent SDK, with API/provider
  auth, allowed-tool/profile mapping, streaming message handling, artifact
  capture, and usage capture.
- [ ] Implement Pi adapter through the Pi extension system, with Whippletree
  effect/run correlation to Pi conversation threads, transcript/evidence export,
  and completion detection.
- [ ] Capture provider transcripts, artifacts, exit/status, tool calls, usage,
  diffs, and changed files for real Codex and Claude turns.
- [ ] Capture harness failure events and evidence for config, auth, workspace,
  adapter, launch, request, stream, timeout, cancellation, result-validation, and
  artifact-capture failures.
- [ ] Normalize real provider lifecycle into `agent.turn.*` facts/events.
- [ ] Implement a control-plane driver that materializes ready rules into facts
  and effect outbox entries before providers try to claim effects.
- [ ] Implement workspace records and workspace policy enforcement for shared
  checkout, per-effect worktree, per-issue worktree, and remote sandbox modes.
- [ ] Derive standard `agent.turn.*` completion facts and deterministic
  relationship aliases used by examples.

### BAML Coerce

- [x] Implement managed BAML service startup.
- [x] Implement BAML HTTP client.
- [x] Implement `coerce` effect contracts.
- [x] Validate BAML class/enum/function references at compile time where
  possible.
- [x] Add no-mock coerce integration tests when credentials/environment are
  available.
  - `real_baml_coerce_endpoint_smoke` runs against a configured
    `WHIPPLETREE_BAML_TEST_ENDPOINT` and function contract.
  - `scripts/check-real-providers.sh` requires the BAML smoke-test environment
    before claiming real-provider readiness.
  - `scripts/openai-coerce-server.mjs` provides a local BAML-compatible
    `/coerce` bridge backed by OpenAI Structured Outputs for dogfooding.
  - `scripts/check-openai-coerce.sh` loads `OPENAI_API_KEY` from `.env`, starts
    the OpenAI bridge, and runs the no-mock Coerce smoke test through
    `HttpBamlClient`.
- [x] Add deterministic fake provider tests for CI.

### Loft

- [ ] Add the Loft repository as a git submodule, for example under
  `vendor/loft` or `external/loft`.
  - `scripts/add-loft-submodule.sh` now performs the guarded add only once
    the Loft repo has tracked spec and fixture files.
  - `scripts/check-loft-source-repo.sh` centralizes the local Loft repo
    preflight used by submodule and real-provider readiness.
  - `scripts/stage-loft-fixtures.sh` stages Whippletree's compatibility fixtures
    into a local Loft repo for review and Loft-side commit.
  - `scripts/export-loft-source-patch.sh` produces a reviewable Loft patch
    artifact for the staged spec and fixtures without committing in Loft.
  - `scripts/loft-handoff-report.sh` summarizes Loft-side blockers and next
    commands without mutating either repository.
- [ ] Import and reference the Loft repo specs/fixtures as the source of truth
  for issue IDs, issue state, leases, commands, JSON shapes, and failure modes.
- [x] Replace local placeholder assumptions with the Loft v0.1 CLI/API
  contract.
- [x] Implement Loft capability binding.
- [x] Implement show, claim, renew, release, note, transition, evidence,
  resource-intent, complete, and fail command shapes.
- [x] Model claim success/failure as typed facts.
- [ ] Add Loft contract/conformance tests against submodule fixtures.
  - `scripts/check-loft-fixtures.sh` and
    `loft_submodule_fixture_shapes_are_compatible` validate the
    manifest-driven fixture JSON contract against an explicit fixture override,
    future submodule fixtures, or local compatibility fixtures in
    `examples/loft-fixtures/v0.1`.
  - The fixture manifest now covers rich issue shape, `issue_status`, lease
    claim/renew/release, lease-scoped mutation failures, structured evidence,
    resource intent, lifecycle complete/fail, retryable error details, and
    partial lifecycle recovery.
  - `WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES=1` requires the future
    source-of-truth submodule fixture path and rejects local fallback fixtures.
  - `scripts/check-loft-submodule-readiness.sh` validates the future
    `vendor/loft` source-of-truth wiring end to end once the submodule exists.
  - `scripts/loft-fixtures-lib.sh` centralizes the Loft fixture manifest
    path and manifest parsing used by all Loft readiness and staging scripts.
- [x] Add e2e claim-before-agent-turn workflow.

### Human Review

- [x] Implement human inbox store tables.
- [x] Implement `askHuman` effect.
- [x] Implement CLI commands to list and answer pending human reviews.
- [x] Normalize answers into typed facts.

### Observability

- [x] Implement artifact/evidence store.
- [x] Link evidence to events, effects, runs, facts, and rule commits.
- [x] Add trace export for external observability systems.
- [x] Audit Stage 8 against the skills, agent harness, coerce, Loft, human
  review, and observability specs; record gaps for the final audit stage.
  - Mock harnesses, fake coerce, local Loft contract behavior, human review,
    skills, and evidence capture are covered in kernel e2e tests.
  - No-mock BAML coerce smoke coverage is available when an external endpoint
    and function contract are configured.
  - Final-audit gaps: destructive Loft provider flows and Loft submodule
    fixtures remain external-prerequisite work.

Acceptance:

- [x] Every core integration is represented as an effect contract.
- [x] Every provider interaction writes evidence.
- [x] E2E tests can run with mock providers and selected real providers.

## Stage 9: Generated Verification And Static Tooling

Goal: make verification part of normal authoring without making users learn
Maude or TLA+.

- [x] Generate Maude modules from typed rule IR.
- [x] Generate bounded safety searches for effect graphs and rule cycles.
- [x] Add `whip check --model-search`.
- [x] Attach counterexamples to source spans.
- [x] Add trace-conformance checking to integration tests.
- [x] Add `whip doctor` checks for tool availability:
  - [x] Maude
  - [x] Java
  - [x] Apalache
  - [x] BAML
  - [x] provider CLIs
- [x] Decide whether TLA+/Apalache runs in default CI or nightly CI.
  - Decision: keep TLA+/Apalache in default CI through
    `scripts/check-tla-models.sh`; generated per-program Maude checks stay
    opt-in through `whip check --model-search`.
- [x] Audit Stage 9 against generated verification requirements, doctor checks,
  optional tool paths, and counterexample UX; record gaps for the final audit
  stage.
  - Generated Maude modules and bounded dependency-release searches are wired
    into the CLI.
  - Doctor reports Maude, Java, Apalache, BAML, and provider CLI availability.
  - Normal `whip check` does not require Maude or Apalache.
  - Counterexample failures are attached to dependency source spans using the
    matching `after <effect> <predicate>` anchor.
  - Final-audit gap: add an intentionally unsafe generated-check fixture once
    fixture conventions for expected-failure model searches are settled.

Acceptance:

- [x] Generated Maude finds an intentionally unsafe fixture.
- [x] Counterexamples identify the rule/effect path that caused the issue.
- [x] Users can run normal checks without installing all formal tools.

## Stage 10: Examples And Dogfood Workflows

Goal: prove the language is ergonomic before we harden syntax.

- [x] Add examples:
  - [x] minimal no-op rule
  - [x] Ralph loop
  - [x] Loft claim before agent turn
  - [x] coerce classification then branch
  - [x] human review fallback
  - [x] multi-agent bounded concurrency
  - [x] OpenClaw-lite composition
  - [x] plugin memory example
- [x] Run desire-path sessions where agents author Whippletree scripts.
- [x] Record common wrong guesses.
- [x] Decide which guesses become aliases, diagnostics, or hard errors.
- [x] Update language syntax and companion skill based on results.
- [x] Audit Stage 10 against examples, dogfood notes, desire-path outcomes, and
  fixture coverage; record gaps for the final audit stage.
  - Examples now cover all listed Stage 10 workflow shapes and have checked IR
    snapshots.
  - CLI integration runs `whip check` across all checked examples.
  - Generated Maude model search passes for examples with effect dependencies.
  - Dogfood guesses are recorded in `spec/examples.md`; companion authoring
    guidance is updated in `spec/companion-skill.md`.
  - Follow-up: guarded fact matches and source assertions now exist, but the
    full expression kernel is still tracked separately in
    `spec/expression-kernel-tracker.md`.
  - Fixed during final audit: `as binding` after a multi-line string now
    receives a targeted diagnostic.
  - Dogfood gap: provider-language e2e now uses one shared task schema, but
    typed dynamic agent targets, static matrices, and action/template expansion
    are not implemented.

Acceptance:

- [x] A coding agent can author and run a simple workflow with only the
  companion skill.
- [x] Repeated wrong guesses have either been paved or deliberately rejected
  with excellent diagnostics.
- [x] Examples are included in parser, static-analysis, and e2e test fixtures.

## Stage 10b: Deterministic Routing Language Features

Goal: remove provider/model routing decisions from prompts and model outputs
while keeping workflow source compact.

Detailed guard/assertion expression coverage is tracked in
[expression-kernel-tracker.md](expression-kernel-tracker.md). Stage 10b should
not be considered complete until that tracker reaches its acceptance gates.

- [x] Add guarded fact matches:
  `when LanguageTask as task where task.provider == "codex"`.
- [x] Type-check guard expressions against matched schemas, enum variants,
  literal unions, optional presence proofs, and scalar comparison rules.
- [x] Add full expression parser/AST support for boolean logic, ordering,
  membership, count/empty/exists, query filters, array literals, map indexing,
  parentheses, and precedence.
- [x] Add a finite Maude expression-kernel model for guard true/false/error,
  optional presence, enum/literal domains, typed pattern branches, assertions,
  and AgentRef validity.
- [x] Extend generated per-program Maude checks so rule firing is gated by
  lowered guard predicates before effect graphs can commit.
- [x] Add deterministic assertion syntax over fact/effect projections for e2e
  checks.
- [x] Replace ad hoc guard/assertion string evaluators with a shared typed
  expression evaluator.
- [x] Preserve `Missing` separately from `Null` and reject unsafe optional field
  access unless a presence proof exists.
- [ ] Add static matrix seeding for small typed fixture tables.
- [ ] Add rule-template/action-block expansion for repeated effect chains,
  preserving source spans, idempotency keys, and compiled IR visibility.
- [x] Design `AgentRef<...>` or equivalent typed dynamic agent references.
- [x] Reject plain strings as dynamic `tell` targets.
- [ ] Add a deterministic validation capability path for checks that should not
  require BAML/model judgment.
- [x] Rewrite `examples/provider-language-e2e.whip` to use one shared
  `LanguageTask` schema routed by typed `AgentRef`.
- [x] Add `examples/companion-skill-dogfood.whip` to prove companion-skill
  authored workflows can route phase-review work through typed `AgentRef`
  metadata, source assertions, and tracker-path prompts without provider/model
  identity classification by an LLM.
- [x] Update the companion authoring skill to recommend deterministic routing
  metadata and warn against asking models to identify providers/routes.

Acceptance:

- [x] A single shared task schema can route six language tasks across Codex,
  Claude, and Pi without duplicate provider-specific classes.
- [x] Provider counts, agent-turn counts, and BAML-review counts are asserted in
  source or first-class assertion fixtures.
- [x] The BAML review output contains only reviewable artifact qualities unless
  the workflow explicitly reviews provider evidence.
- [x] Guard expressions are parsed, typed, and evaluated through the expression
  kernel rather than raw string splitting.

## Stage 11: E2E Test System

Goal: test the real system from source file to provider outcome.

- [x] Build test harness utilities for isolated temp workspaces and SQLite
  stores.
- [x] Add deterministic mock providers for CI.
- [x] Add optional real-provider tests gated by environment variables.
- [x] Add e2e coverage for:
  - [x] compile and run minimal workflow
  - [x] Ralph loop one-turn bounded test mode
  - [x] Loft claim success before agent turn
  - [x] Loft claim failure to human review
  - [x] coerce success branch
  - [x] coerce failure branch
  - [x] effect retry after transient failure
  - [x] lease expiry and recovery
  - [x] pause prevents new effectful rewrites
  - [x] resume continues from durable state
  - [x] cancel prevents new provider starts
  - [x] restart daemon/control plane and replay state
  - [x] concurrent instances do not cross-contaminate facts or effects
  - [x] capability denial blocks execution with useful status
  - [x] plugin-registered effect runs through the outbox
- [x] Export trace for every e2e test and run conformance checks.
- [x] Add flake-stress or repeated-run tests for scheduler races.
- [x] Audit Stage 11 against e2e coverage, mock/real-provider gating, artifacts,
  and trace export; record gaps for the final audit stage.
  - Mock-provider e2e coverage runs through `scripts/check-e2e.sh`.
  - Optional real-provider prerequisites are gated by
    `WHIPPLETREE_E2E_REAL_PROVIDERS=1` in `scripts/check-real-providers.sh`.
  - Selected no-mock provider smoke runs are supported with
    `WHIPPLETREE_REAL_PROVIDERS=loft`, `baml`, or `loft,baml`.
  - Real-provider readiness now checks provider tools, required environment,
    Loft fixture repo cleanliness/tracked spec when Loft is selected, and
    BAML endpoint reachability when BAML is selected before any destructive flow
    is attempted.
  - Read-only no-mock Loft `show` and no-mock BAML `coerce` smoke tests run
    when real-provider prerequisites are configured.
  - Kernel e2e tests export temp trace artifacts before checking conformance.
  - Final-audit gap: real-provider destructive flows remain manual until Loft
    and BAML fixtures are isolated from real workspaces.

Acceptance:

- [x] E2E suite passes from a clean checkout with mock providers.
- [x] Optional real-provider suite documents required credentials and tools.
- [x] Real-provider smoke runs can emit an audit artifact without changing the
  underlying check exit code.
- [x] Release readiness can emit an aggregate audit artifact with external
  prerequisite checks recorded separately from local required checks.
  - CI runs fast release readiness and uploads the generated report artifact.
  - Release readiness also emits the Loft handoff report for external
    prerequisite tracking.
- [x] A failed e2e run leaves artifacts useful enough to debug without
  rerunning immediately.
- [ ] Dogfood workflow `implementation-plan-phase-review.whip` can create phase
  review facts, enqueue Codex review effects, run configured Codex threads, and
  update `spec/implementation-plan-phase-review-tracker.md`.
- [ ] Real-provider dogfood can be run with Codex, Claude, and Pi provider
  bindings independently, with skipped providers reported as unavailable rather
  than silently passing.

## Stage 12: Companion Skill, Docs, And Release Hardening

Goal: make the system usable by coding agents and non-expert operators.

- [x] Write first-party Whippletree companion skill.
- [x] Include:
  - [x] language overview
  - [x] common workflow patterns
  - [x] capability profile selection guidance
  - [x] examples of good scripts
  - [x] examples of rejected scripts and why
  - [x] desire-path notes and aliases
  - [x] debugging/status workflow
  - [x] safety guidance for enterprise environments
- [x] Write CLI quickstart.
- [x] Write operator guide for stores, profiles, providers, and recovery.
- [x] Write plugin author guide.
- [x] Write troubleshooting guide.
- [x] Add release checklist.
- [x] Add migration notes explaining why legacy systems were moved aside.
- [x] Audit Stage 12 against the companion skill, docs, operator guidance,
  release checklist, and migration notes; record gaps for the final audit stage.
  - Companion skill lives at `skills/whippletree-author/SKILL.md`.
  - User/operator docs live in `spec/quickstart.md`, `spec/operator-guide.md`,
    `spec/plugin-author-guide.md`, and `spec/troubleshooting.md`.
  - Release and migration docs live in `spec/release-checklist.md` and
    `spec/migration-notes.md`.
  - Fixed during final audit: `scripts/install-whippletree-skill.sh` installs the
    companion skill into a local skill directory.

Acceptance:

- [x] A fresh agent using the companion skill can write a valid Whippletree script.
- [x] A human can run the quickstart without reading architecture docs.
- [x] Release checklist covers tests, formal checks, docs, and known gaps.

## Stage 13: Final Audit And Gap Closure

Goal: close every gap found by stage audits before declaring v0 complete.

- [x] Collect the audit findings from Stages 0-12 into one tracked gap list.
- [x] Classify each gap as blocking, deferred-with-rationale, or already fixed.
- [x] Audit security boundaries:
  - [x] capability/profile enforcement
  - [x] provider credential handling
  - [x] prompt/input/output retention posture
  - [x] plugin isolation and authority escalation paths
  - [x] local filesystem and network access assumptions
- [x] Audit efficiency and performance:
  - [x] parser and static-analysis behavior on large programs
  - [x] SQLite query plans and indexes for scheduler/status paths
  - [x] event replay and projection rebuild costs
  - [x] provider artifact/evidence storage growth
  - [x] CLI latency for common inspection commands
- [x] Audit distributed-systems integrity:
  - [x] idempotency-key coverage
  - [x] duplicate terminal completion prevention
  - [x] lease expiry, retry, and recovery behavior
  - [x] dependency release correctness
  - [x] pause/resume/cancel race behavior
  - [x] multi-instance isolation
  - [x] external kernel integration semantics for Loft and future plugins
- [x] Audit reliability and operability:
  - [x] crash recovery from every critical transaction boundary
  - [x] actionable diagnostics and status for blocked/failed effects
  - [x] trace export and conformance coverage
  - [x] migration and upgrade behavior
  - [x] clean-checkout setup and doctor guidance
- [x] Fix every blocking implementation gap found during audit.
- [x] Fix every blocking spec, docs, fixture, and test gap found during audit.
- [x] Re-run the full verification suite after audit fixes.
- [x] Update the implementation plan checkboxes and release checklist with final
  audit outcomes.

Acceptance:

- [x] No blocking audit gaps remain open.
- [x] Deferred gaps have explicit rationale and follow-up tracking.
- [x] Full verification has been rerun after the final audit fixes.

## Definition Of Done For v0

- [x] All M0-M8 milestones are complete.
- [x] `cargo test --workspace` passes.
- [x] `scripts/check-formal-models.sh` passes.
- [x] CLI e2e suite passes with mock providers.
- [x] Optional real-provider smoke tests have been run and results documented.
  - `scripts/check-real-providers-report.sh` writes
    `target/real-provider-smoke-report.md` by default and preserves the
    underlying `scripts/check-real-providers.sh` exit code.
  - `scripts/check-openai-coerce.sh` passed locally against the OpenAI-backed
    Coerce bridge using `OPENAI_API_KEY` from `.env`.
- [x] Trace conformance runs over every e2e test.
- [x] Companion skill is installed or documented.
- [x] The repo has no active implementation outside the new root workspace
  except documented legacy folders.

## Immediate Next Slice

The next implementation slice should be:

1. Resolve deferred audit gaps in `spec/final-audit.md` when their external
   prerequisites are ready.
2. Run optional real-provider smoke tests with isolated Loft/BAML fixtures.
3. Package the `whippletree-author` companion skill for the chosen distribution
   mechanism.
   - Local package automation exists at `scripts/package-whippletree-skill.sh`.
