# Implementation Plan

Status: draft tracker

This is the project tracker for the new Armature system. It runs from formal
modeling through the last e2e acceptance tests.

The plan is organized as stage gates. A stage is complete only when its
acceptance checks pass and the relevant docs, examples, and tests have been
updated. Checkboxes should be updated as implementation lands.

## Product Target

Armature v0 should provide:

- a restricted rule language for durable agent orchestration
- typed facts, schemas, effect contracts, and capability profiles
- an event-sourced runtime kernel backed by SQLite
- a control plane for compiling, starting, inspecting, pausing, resuming, and
  cancelling workflow instances
- first-party effects for agent turns, BAML-backed coercion, Docket claims,
  human review, skills, and evidence capture
- adapter support for at least Codex, Claude Code, and Pi-style harnesses
- formal checks and trace-conformance checks that catch orchestration bugs
- e2e tests that run real workflows through the full stack

## Milestone Summary

- [x] M0: Formal kernel spine is executable and checked in CI.
- [ ] M1: Source language grammar, parser, and typed IR compile example
  programs.
- [ ] M2: Runtime store and kernel can replay events and commit deterministic
  rule rewrites.
- [ ] M3: Durable effects, dependency scheduling, leases, retries, and trace
  conformance work end to end.
- [ ] M4: Control plane and CLI manage programs and instances.
- [ ] M5: Capability registry, skills, agent harnesses, BAML coerce, Docket,
  human review, and observability are wired through typed effect contracts.
- [ ] M6: Static analysis and generated Maude checks protect user programs.
- [ ] M7: E2E suite covers happy paths, failure paths, recovery, and dogfood
  workflows.
- [ ] M8: Companion skill and release hardening make the system usable by
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
- [x] Add a Docket-claim-gated agent-turn model test.
- [x] Make `scripts/check-formal-models.sh` assert expected Maude search
  outcomes.
- [ ] Extend the Maude model with durable event log, fact projection, effect
  graph commit, and rule firing.
- [ ] Model blocked-by-policy and blocked-by-capacity separately from
  blocked-by-dependency.
- [ ] Model retry, timeout, cancellation, and lease expiry outcomes.
- [ ] Add a Ralph loop model with an explicit external-event boundary.
- [ ] Add a coerce classification model with success/failure branches.
- [ ] Add generated Maude checks from typed IR once the compiler exists.

### TLA+/Apalache Runtime Lifecycle

- [ ] Finish the TLA+ lifecycle model for append, projection, claim, lease,
  retry, completion, pause/resume, cancellation, and recovery.
- [ ] Install or document the Apalache runner path used by CI.
- [ ] Add bounded checks for safety invariants:
  - [ ] every run references an existing effect
  - [ ] no effect has more than one terminal completion
  - [ ] no provider run starts unless the effect is claimable
  - [ ] no claimable effect has unsatisfied dependencies
  - [ ] paused instances do not commit new effectful rewrites
  - [ ] recovery does not reorder the per-instance event log
- [ ] Add liveness/fairness checks only after safety checks stabilize.

### Trace Conformance

- [ ] Define the trace event schema consumed by model checkers.
- [ ] Implement a trace checker that validates runtime traces against kernel
  invariants.
- [ ] Add trace checker fixtures for success, dependency violation, duplicate
  terminal completion, stale lease completion, and pause/cancel races.

### Veil/Lean Recheck

- [ ] Reevaluate Veil after the kernel semantics and trace schema stabilize.
- [ ] Decide whether Veil proves stable invariants or remains out of v0.

Acceptance:

- [ ] Formal checks fail on intentionally broken dependency-release behavior.
- [ ] Runtime trace fixtures catch impossible lifecycle transitions.
- [ ] Generated Maude checks can be run optionally from the CLI.

## Stage 2: Source Language, Parser, And Typed IR

Goal: compile `.armature` source into deterministic, typed IR.

- [ ] Finalize v0 grammar for rules, schemas, agents, skills, capabilities,
  effects, `after` blocks, `coerce`, and record construction.
- [ ] Choose and document the parser implementation strategy.
- [ ] Implement lexer/parser with diagnostics that preserve source spans.
- [ ] Build a recoverable parse tree suitable for formatting and helpful
  errors.
- [ ] Define the typed AST and typed rule IR.
- [ ] Implement lowering from source AST to typed IR.
- [ ] Support BAML-aligned boundary types:
  - [ ] string
  - [ ] int
  - [ ] float
  - [ ] bool
  - [ ] null
  - [ ] literal
  - [ ] array
  - [ ] map
  - [ ] union
  - [ ] class
  - [ ] enum
  - [ ] image
  - [ ] audio
  - [ ] pdf
  - [ ] video
- [ ] Implement source-span-aware errors modeled after Gleam-style diagnostics.
- [ ] Add golden parse/IR fixtures for all examples.
- [ ] Add formatter scaffolding after the parse tree stabilizes.

Acceptance:

- [ ] `armature check examples/*.armature` produces stable typed IR.
- [ ] Invalid examples produce precise errors with source spans and suggested
  fixes.
- [ ] IR snapshots are deterministic across runs.

## Stage 3: Static Analysis

Goal: reject programs that would produce hidden distributed-system bugs.

- [ ] Validate schema references, field paths, enum variants, and literal
  types.
- [ ] Validate fact read/write/consume sets for each rule.
- [ ] Validate effect contracts and output binding scopes.
- [ ] Reject use of effect outputs outside their matching `after` branch.
- [ ] Validate finite effect graphs and dependency edge references.
- [ ] Reject implicit ordering assumptions between sibling effects.
- [ ] Build rule dependency graph analysis.
- [ ] Implement recursion stratification.
- [ ] Reject effectful cycles that do not cross an external event, clock,
  human, or durable boundary.
- [ ] Validate idempotency-key derivability.
- [ ] Validate required capability/profile bindings.
- [ ] Validate resource/capacity declarations.
- [ ] Emit actionable diagnostics for every rejection.

Acceptance:

- [ ] Unsafe examples are rejected with specific explanations.
- [ ] Safe Ralph, Docket, coerce, and human-review examples pass.
- [ ] Static analysis outputs enough metadata for generated Maude checks.

## Stage 4: Runtime Store

Goal: persist every runtime transition in a replayable SQLite store.

- [ ] Create the root Rust workspace and crates.
- [ ] Add a store crate with SQLite migrations.
- [ ] Define tables for:
  - [ ] programs
  - [ ] program versions
  - [ ] instances
  - [ ] event log
  - [ ] fact projections
  - [ ] effect outbox
  - [ ] effect dependency edges
  - [ ] runs
  - [ ] leases
  - [ ] artifacts/evidence
  - [ ] diagnostics
  - [ ] plugin registrations
  - [ ] capability bindings
- [ ] Implement append-only event writes with per-instance sequence numbers.
- [ ] Implement transaction helpers for rule commits and effect completion.
- [ ] Implement projection rebuild from the event log.
- [ ] Implement store-level uniqueness for idempotency keys and terminal
  completions.
- [ ] Implement migration tests and replay tests.

Acceptance:

- [ ] Store replay reconstructs facts/effects from the log.
- [ ] Duplicate terminal completions fail transactionally.
- [ ] Interrupted transactions leave no partial rule commit.

## Stage 5: Runtime Kernel

Goal: execute compiled programs deterministically against the store.

- [ ] Implement kernel operations:
  - [ ] create program version
  - [ ] create instance
  - [ ] ingest external event
  - [ ] derive facts
  - [ ] evaluate rules
  - [ ] commit rule rewrite
  - [ ] enqueue effect graph
  - [ ] satisfy dependencies
  - [ ] claim effect
  - [ ] start run
  - [ ] complete run
  - [ ] fail run
  - [ ] timeout run
  - [ ] cancel effect
  - [ ] pause instance
  - [ ] resume instance
  - [ ] cancel instance
- [ ] Ensure all kernel operations are deterministic and transaction-scoped.
- [ ] Implement idempotency-key generation.
- [ ] Implement scheduler queries for claimable effects.
- [ ] Implement lease acquisition, renewal, expiry, and recovery.
- [ ] Implement retry/backoff policy.
- [ ] Implement trace emission for conformance checking.

Acceptance:

- [ ] Unit tests cover every lifecycle transition.
- [ ] Kernel tests match the Maude and TLA+ lifecycle expectations.
- [ ] Trace conformance passes for all kernel integration tests.

## Stage 6: Control Plane And CLI

Goal: expose Armature as an inspectable system for many concurrent scripts.

- [ ] Implement CLI crate.
- [ ] Implement commands:
  - [ ] `armature check`
  - [ ] `armature compile`
  - [ ] `armature run`
  - [ ] `armature instances`
  - [ ] `armature status`
  - [ ] `armature log`
  - [ ] `armature facts`
  - [ ] `armature effects`
  - [ ] `armature runs`
  - [ ] `armature pause`
  - [ ] `armature resume`
  - [ ] `armature cancel`
  - [ ] `armature retry`
  - [ ] `armature doctor`
- [ ] Support JSON output for every inspection command.
- [ ] Add compact human-readable status views.
- [ ] Add helpful suggestions for common desire-path mistakes.
- [ ] Add control-plane tests for concurrent instances.

Acceptance:

- [ ] A user can start two instances of the same program and inspect them
  independently.
- [ ] Status shows current facts, queued effects, active runs, failures, and
  recent evidence.
- [ ] CLI errors include next-step guidance.

## Stage 7: Capability Registry And Plugin Kernel

Goal: safely bind authority at runtime without bloating the core.

- [ ] Implement capability schema registration.
- [ ] Implement effect provider registration.
- [ ] Implement profiles with descriptions and enforcement modes.
- [ ] Ship default profiles:
  - [ ] permissive
  - [ ] repo-reader
  - [ ] repo-writer
  - [ ] internet-research
  - [ ] human-review
- [ ] Implement custom profile loading from config.
- [ ] Validate source-requested capabilities against registry bindings.
- [ ] Implement plugin package discovery and loading.
- [ ] Ensure plugins cannot mutate kernel state directly.
- [ ] Add plugin fixtures for memory and external notification examples.

Acceptance:

- [ ] Missing capabilities block effects before provider execution.
- [ ] Profile mismatch is visible in status and trace output.
- [ ] A plugin can register an effect contract and provider without changing
  kernel code.

## Stage 8: Core Integrations

Goal: wire the built-in effect families through the same contract system.

### Skills

- [ ] Implement deterministic skill registry.
- [ ] Attach skills to agents, turns, and program scopes.
- [ ] Record skill versions and source paths in evidence.

### Agent Harnesses

- [ ] Define the harness adapter trait.
- [ ] Implement mock harness for deterministic tests.
- [ ] Implement Codex adapter.
- [ ] Implement Claude Code adapter.
- [ ] Implement Pi-style adapter.
- [ ] Capture stdout/stderr, transcripts, artifacts, exit status, and usage.
- [ ] Normalize provider lifecycle into `agent.turn.*` facts/events.

### BAML Coerce

- [ ] Implement managed BAML service startup.
- [ ] Implement BAML HTTP client.
- [ ] Implement `coerce` effect contracts.
- [ ] Validate BAML class/enum/function references at compile time where
  possible.
- [ ] Add no-mock coerce integration tests when credentials/environment are
  available.
- [ ] Add deterministic fake provider tests for CI.

### Docket

- [ ] Implement Docket capability binding.
- [ ] Implement claim, release, note, transition, and evidence effects.
- [ ] Model claim success/failure as typed facts.
- [ ] Add e2e claim-before-agent-turn workflow.

### Human Review

- [ ] Implement human inbox store tables.
- [ ] Implement `askHuman` effect.
- [ ] Implement CLI commands to list and answer pending human reviews.
- [ ] Normalize answers into typed facts.

### Observability

- [ ] Implement artifact/evidence store.
- [ ] Link evidence to events, effects, runs, facts, and rule commits.
- [ ] Add trace export for external observability systems.

Acceptance:

- [ ] Every core integration is represented as an effect contract.
- [ ] Every provider interaction writes evidence.
- [ ] E2E tests can run with mock providers and selected real providers.

## Stage 9: Generated Verification And Static Tooling

Goal: make verification part of normal authoring without making users learn
Maude or TLA+.

- [ ] Generate Maude modules from typed rule IR.
- [ ] Generate bounded safety searches for effect graphs and rule cycles.
- [ ] Add `armature check --model-search`.
- [ ] Attach counterexamples to source spans.
- [ ] Add trace-conformance checking to integration tests.
- [ ] Add `armature doctor` checks for tool availability:
  - [ ] Maude
  - [ ] Java
  - [ ] Apalache
  - [ ] BAML
  - [ ] provider CLIs
- [ ] Decide whether TLA+/Apalache runs in default CI or nightly CI.

Acceptance:

- [ ] Generated Maude finds an intentionally unsafe fixture.
- [ ] Counterexamples identify the rule/effect path that caused the issue.
- [ ] Users can run normal checks without installing all formal tools.

## Stage 10: Examples And Dogfood Workflows

Goal: prove the language is ergonomic before we harden syntax.

- [ ] Add examples:
  - [ ] minimal no-op rule
  - [ ] Ralph loop
  - [ ] Docket claim before agent turn
  - [ ] coerce classification then branch
  - [ ] human review fallback
  - [ ] multi-agent bounded concurrency
  - [ ] OpenClaw-lite composition
  - [ ] plugin memory example
- [ ] Run desire-path sessions where agents author Armature scripts.
- [ ] Record common wrong guesses.
- [ ] Decide which guesses become aliases, diagnostics, or hard errors.
- [ ] Update language syntax and companion skill based on results.

Acceptance:

- [ ] A coding agent can author and run a simple workflow with only the
  companion skill.
- [ ] Repeated wrong guesses have either been paved or deliberately rejected
  with excellent diagnostics.
- [ ] Examples are included in parser, static-analysis, and e2e test fixtures.

## Stage 11: E2E Test System

Goal: test the real system from source file to provider outcome.

- [ ] Build test harness utilities for isolated temp workspaces and SQLite
  stores.
- [ ] Add deterministic mock providers for CI.
- [ ] Add optional real-provider tests gated by environment variables.
- [ ] Add e2e coverage for:
  - [ ] compile and run minimal workflow
  - [ ] Ralph loop one-turn bounded test mode
  - [ ] Docket claim success before agent turn
  - [ ] Docket claim failure to human review
  - [ ] coerce success branch
  - [ ] coerce failure branch
  - [ ] effect retry after transient failure
  - [ ] lease expiry and recovery
  - [ ] pause prevents new effectful rewrites
  - [ ] resume continues from durable state
  - [ ] cancel prevents new provider starts
  - [ ] restart daemon/control plane and replay state
  - [ ] concurrent instances do not cross-contaminate facts or effects
  - [ ] capability denial blocks execution with useful status
  - [ ] plugin-registered effect runs through the outbox
- [ ] Export trace for every e2e test and run conformance checks.
- [ ] Add flake-stress or repeated-run tests for scheduler races.

Acceptance:

- [ ] E2E suite passes from a clean checkout with mock providers.
- [ ] Optional real-provider suite documents required credentials and tools.
- [ ] A failed e2e run leaves artifacts useful enough to debug without
  rerunning immediately.

## Stage 12: Companion Skill, Docs, And Release Hardening

Goal: make the system usable by coding agents and non-expert operators.

- [ ] Write first-party Armature companion skill.
- [ ] Include:
  - [ ] language overview
  - [ ] common workflow patterns
  - [ ] capability profile selection guidance
  - [ ] examples of good scripts
  - [ ] examples of rejected scripts and why
  - [ ] desire-path notes and aliases
  - [ ] debugging/status workflow
  - [ ] safety guidance for enterprise environments
- [ ] Write CLI quickstart.
- [ ] Write operator guide for stores, profiles, providers, and recovery.
- [ ] Write plugin author guide.
- [ ] Write troubleshooting guide.
- [ ] Add release checklist.
- [ ] Add migration notes explaining why legacy systems were moved aside.

Acceptance:

- [ ] A fresh agent using the companion skill can write a valid Armature script.
- [ ] A human can run the quickstart without reading architecture docs.
- [ ] Release checklist covers tests, formal checks, docs, and known gaps.

## Definition Of Done For v0

- [ ] All M0-M8 milestones are complete.
- [ ] `cargo test --workspace` passes.
- [ ] `scripts/check-formal-models.sh` passes.
- [ ] CLI e2e suite passes with mock providers.
- [ ] Optional real-provider smoke tests have been run and results documented.
- [ ] Trace conformance runs over every e2e test.
- [ ] Companion skill is installed or documented.
- [ ] The repo has no active implementation outside the new root workspace
  except documented legacy folders.

## Immediate Next Slice

The next implementation slice should be:

1. Expand Stage 1 Maude from lifecycle-only effects to event-log/fact/effect
   kernel state.
2. Start Stage 2 with the smallest parser/IR path that compiles one example.
3. Add Stage 11 skeleton tests early so every runtime slice lands with e2e
   pressure.
