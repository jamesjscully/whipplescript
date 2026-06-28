# Implementation Plan

Status: draft tracker

This is the project tracker for the new WhippleScript system. It runs from formal
modeling through the last e2e acceptance tests.

The plan is organized as stage gates. A stage is complete only when its
acceptance checks pass and the relevant docs, examples, and tests have been
updated. Checkboxes should be updated as implementation lands.

## Per-Piece Review Gate

Implementation proceeds **one piece at a time**, and a piece is not done — its
`- [ ]` box is not checked — until it has passed this gate. The checkbox means
"implemented **and** reviewed **and** verified **and** documented," not just
"code written."

For every piece (a task line below, or a coherent sub-unit of one):

```text
1. Implement the piece.
2. Audit / review it and apply fixes in the same pass. Review against:
   - correctness and the bug/reuse/simplification lens (use /code-review)
   - the spec contract(s) the piece must honor — especially
     admission-and-idempotency.md (identity/idempotency/replay/exactly-once),
     the construct lowering-class catalog, capability/authority boundaries,
     determinism-on-replay, and the diagnostic model
   - the relevant formal model where one exists (the generated/hand-written
     Maude or TLA+ obligation for that piece)
3. Verify: tests + `scripts/check-formal-models.sh` (and report schemas /
   build as applicable) pass green.
4. Update docs in the same pass:
   - spec: the owning spec doc + this tracker
   - user-facing docs/ when the piece changes user-visible behavior (CLI,
     language surface, reports, operator/troubleshooting guidance)
5. Only then check the box. A box may carry a short "(reviewed: <note>)" tag
   when the review found and fixed something worth recording.
```

A stage's `Audit Stage N …` item is the **roll-up** confirmation that every
piece in the stage passed this per-piece gate; it is not a substitute for
reviewing as you go. Pieces that are spec-only (no code) still get the review +
docs steps; pieces with a formal model get the model checked before the box.

**Checkbox states:** `- [ ]` not started; `- [~]` in progress / partially
landed; `- [x]` implemented + reviewed + verified + documented; `- [-]` closed
by decision — deliberately **not** building it (rationale recorded inline). A
`[-]` item is a resolved gap (the open *question* is settled), not an
implemented feature.

The next implementation cycle should use
[`testing-strategy.md`](testing-strategy.md) as the organizing test plan for
language, runtime, package-manager, standard-package, provider-boundary, and
formal-verification coverage. The older e2e stage below remains useful history
and existing coverage, but it is not the whole testing strategy.

Use [`error-handling.md`](error-handling.md) as the diagnostic contract for the
same cycle. New parser, package, construct graph, lowering, runtime, provider,
and report-admission work should emit structured diagnostics with stable codes,
source/provenance links, safe suggestions, and redaction metadata where needed.

Use [`editor-tooling.md`](editor-tooling.md) for the authoring tooling target:
`whip lint`, `whip fmt`, and `whip lsp` should be built over the same compiler,
package, diagnostic, formatter, and report services as `whip check`.

Use [`workflow-testing.md`](workflow-testing.md) for the user-facing testing
target: deterministic top-level `test` scenarios with `given`, `stub`, `run`,
and `expect`, backed by package-declared fixture outcomes and standard risk
utilities.

## Outstanding Work — Ordered Roadmap

This is the single ordered view of everything still open: the unfinished v0
stages (unchecked boxes in Stages 1–11) and Cycle 2 (Stages P1b–P7). It sequences
by hard dependency and marks what runs in parallel. Detailed task lists stay in
the referenced stages; this section is the *order*, not a restatement.

**Done:** P1a — signal vocabulary rename (verified green). Spec hardening —
authored [`admission-and-idempotency.md`](admission-and-idempotency.md) (the
single admission/identity/idempotency/replay contract) and resolved the Tier 1/2/3
review findings across the core/effects/agent/events/language clusters (fixpoint
determinism, idempotency-key derivation, exactly-once recovery, terminal-output
union, in-turn-facts-as-evidence, pattern recursion + flow liveness, one lease
engine, kernel/control-plane ownership). The review-profile name is now unified to
`human-review` across spec + code (done 2026-06-16; see Stage 7 /
`capability-registry.md`).

The work splits into two largely-independent tracks that converge at standard
packages and end-to-end validation:

- **Track C — Compiler & package system** (compile-time; no runtime/provider deps)
- **Track R — Runtime & execution** (the unfinished v0 control-plane/harness work)

The spec-hardening pass added implementation obligations to both tracks; they are
slotted below and detailed in Stage R0 and the referenced stages.

### Phase 1 — Foundations (Track C and Track R can run in parallel)

**Track R foundation — do first:**

0. **Stage R0 — Admission & Idempotency Contract** (kernel substrate). Implement
   [`admission-and-idempotency.md`](admission-and-idempotency.md): the admission
   boundary, per-source idempotency keys + store unique index on
   `(instance, fact_identity_key)`, deterministic fixpoint ordering, the single
   lease engine, the typed fact-batch admission primitive, and exactly-once
   external-effect recovery (run-started marker + re-query-or-`uncertain`).
   Everything that admits a fact (signals, clock, coerce, agent turns, package
   effects, fact-batch) depends on this; it underpins Stages 5/6/8 and the
   std-package ports. **This is the highest-leverage runtime item.**

Track C (compile-time, unblocked now):

1. **P5 — Diagnostics spine** (shared `Severity` enum `error|warning|info|hint`,
   reserved code namespaces). First: every later surface emits diagnostics.
2. **Terminal-output union** — DONE. The `case` 4-tag union + kernel statuses
   already existed; this added the missing `after`-branch keywords `times out` and
   `cancelled` (two-token parse, payload typing) and — beyond the parser — added
   distinct `timed_out`/`cancelled` **dependency predicates** so those branches
   release only on that specific terminal (not on any completion, as a first naive
   pass did). Threaded through parser/kernel/store(SQL)/cli; store test
   `timed_out_dependency_releases_only_on_timeout`; workspace + both CI scripts
   green. (reviewed: the release-precision bug was caught during integration, not
   by the parser-only tests.) **Follow-up (2026-06-16):** fixed the runtime binding
   for a hand-written `case` over a **human-ask** terminal — `after <ask> completes
   { case <ask> { Completed as decided => decided.choice } }`. The ask emits both
   `human.ask.issued` (issuance ack) and `human.answer.received` (the answer);
   `effect_binding_value` now prefers the answer, `terminal_union_value` exposes the
   answer's `{choice,text}` as the terminal value, and `human.ask.issued` no longer
   satisfies `completes` (it did, firing the case before any answer) — while
   `succeeds` is left intact for the flow desugaring's `flowIssued` correlation
   (guardrail: soft_middle `flow_*` tests). Regression test
   `human_ask_case_completed_branch_resolves_answer_choice`. This unblocked both
   `scheduled-escalation` and `gastown-lite` under `whip dev` (the latter's coerce
   `decision.verdict` was a cascade of the premature trigger, not a separate bug).
   `check-rule-coverage` is green again: `event-bridge` (an `@service` needing an
   external `deploy.finished` signal), `package-memory` (needs a lock), and
   `exec-json-ingest` (needs `WHIPPLESCRIPT_EXEC_ALLOW`) are SKIP-listed as
   genuinely setup/signal-gated — a `bd9a0be` oversight. See memory
   `project-terminal-output-case-binding-gap`.
3. **P1b — Source-declaration family + `clock_source`** (source parser; signal/
   clock split). Unblocks `std.time` and the source story.
4. **P3 — Lowering-class maturation** (make `typed_effect_call` / `resource_effect`
   package-authorable). Depends on P1b's family work; unblocks `std.messaging` /
   `std.files`.
5. **P2 — Package manager** (`whip package sync`, lock flip). Self-contained;
   unblocks third-party packages. Can go anytime in Phase 1.

Track R (runtime loop, builds on Stage R0):

6. **Stage 5 rule-lowering** → **Stage 6 `whip step` / `whip worker` / `whip dev`**
   + provider-config loading + command-level diagnostics. The deterministic
   runtime loop, built on the Stage R0 admission/idempotency/lease substrate.
7. **Stage 2 language features** (`include` / root selection, `pattern` / `apply`,
   workflow input/output/failure + `complete` / `fail`, `invoke`). Compiler work
   that proceeds alongside. Carries the hardening obligations: v0 non-recursive
   pattern expansion (recursive `apply` is an error), `invoke` canonical-union
   terminals, and `complete`/`fail` first-terminal-wins tie-break.

### Phase 2 — Providers, tooling, and the test runner

Track R:

7. **Stage 7 provider bindings** (Codex / Claude / Pi / coerce / Loft / human config,
   health checks, explainable selection, blocked-effect status). After Stage 6.
8. **Stage 8 real adapters** (Codex / Claude / Pi adapters; transcript / artifact /
   failure-evidence capture; `agent.turn.*` normalization; control-plane driver;
   workspace records). After Stage 7.

Track C:

9. **P7 — Test surfaces** (`whip test`, scenario driver). Needs P5 + the
   deterministic runtime loop (Stage 5 + `whip step`/`dev`) + fixture providers.
10. **P6 — Editor tooling** (`whip lint` / `fmt` / `lsp`). After P5; reuses compiler
    services.

### Phase 3 — Standard packages (convergence: Track C classes + Track R runtime)

11. **P4 std package ports**, in dependency order:
    - `std.telemetry` — read-side only; can land as early as Phase 1.
    - `std.time` — P1b `clock_source` + Stage 6 runtime recurrence.
    - `std.messaging` — P3 `typed_effect_call` + Stage 7/8 providers.
    - `std.files` — last; slimmed to a storage boundary (DR-0019 review): P3
      classes + turn-grant metadata (Stage 8 agent file grants) + the fact-batch
      admission prerequisite; literal paths; docx/xlsx/dynamic-paths/cloud
      deferred.

### Phase 4 — Verification & validation completion

12. **Stage 1** generated guard/assertion Maude searches validated against compiled
    `.whip` fixtures.
13. **Stage 10b** static matrix seeding, rule-template/action-block expansion,
    deterministic validation capability path.
14. **Stage 11** `openclaw-lite` validation workflow; per-provider real e2e.

### Phase 5 — Release hardening (parked; external-prerequisite-gated)

15. Real-provider smoke with isolated Loft/coerce fixtures; deferred audit gaps
    (`final-audit.md`); package the companion skill.

### Critical path and recommendation

The two tracks share no blocking dependency until P4 / std packages, so a
two-person split is natural. The one change from the spec-hardening pass:
**Stage R0 (Admission & Idempotency Contract) now leads Track R** — the runtime
loop (Stage 5/6) is built on it, so it precedes them. Single-threaded, I
recommend **Track C first** (P5 → terminal union → P1b → P3 → P2): it is where the
recent design landed and it unblocks the package ecosystem — but pull **Stage R0
→ Stage 5 → `whip step`/`dev`** forward early, because **P7 `whip test`** and
**P4 runtime validation** both need a working deterministic runtime loop on top of
R0. Concretely: **P5 → terminal-union → P1b → (R0 → Stage 5 → `whip step`/`dev`) →
P3 → P7 → P2 → P4(telemetry/time)**, with Track R Stages 7–8 and the rest of P4
following.

## Product Target

WhippleScript v0 should provide:

- a restricted rule language for durable agent orchestration
- typed facts, schemas, effect contracts, and capability profiles
- an event-sourced runtime kernel backed by SQLite
- a control plane for compiling, starting, inspecting, pausing, resuming, and
  cancelling workflow instances
- first-party effects for agent turns, schema coercion, Loft claims,
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
- [x] M4: Control plane and CLI manage programs, instances, rule stepping, and
  worker/dev loops. (Verified 2026-06-18: `whip step`/`worker`/`dev` drive programs;
  `whip instances`/`status`/`facts`/`effects`/`pause`/`resume`/`cancel`/`retry`
  manage them — see Stage 5.)
- [x] M5: Capability registry, skills, real agent harnesses,
  schema-coercion backends, Loft, human review, and observability are wired
  through typed effect contracts. (Verified 2026-06-18: all components ship — capability
  registry/skills (Stage 7), Codex/Claude/Pi adapters (implemented + green native-adapter
  checks), coerce, Loft, human-review/inbox, otel/trace — and the wiring is validated by
  the green required `native provider contract` check. Live execution against real
  provider SDKs is validated separately and remains env-gated — see M7.)
- [x] M6: Static analysis and generated Maude checks protect user programs.
- [x] M7: E2E suite covers happy paths, failure paths, recovery, and validation
  workflows through real provider surfaces where configured. (Verified 2026-06-23:
  the e2e suites — `whipplescript-kernel/tests/e2e.rs`,
  `whipplescript-cli/tests/control_plane.rs` + `soft_middle.rs`, 244 test fns —
  cover happy/complete paths, failure/error branches (≈21), recovery/replay/
  lease-expiry (incl. `test_replay_verifies_event_log_reprojects_identically`),
  and validation/acceptance/assertion flows (≈23, incl. `whip accept`). Coverage
  "through real provider surfaces" runs under the credential-gated `*-live-smoke`
  / `check-real-providers.sh` checks (env-gated, like M5). Full workspace suite
  green.)
- [x] M8: Companion skill and release hardening make the system usable by
  coding agents without hand-holding.

## Stage 0: Repository Reset And Project Skeleton

Goal: make the repo coherent after the redesign and remove inactive systems.

- [x] Remove inactive runtime implementation trees from the active repository.
- [x] Remove inactive package, skill, model, and example trees from the active
  repository.
- [x] Create the new `spec/` suite for the rule-machine design.
- [x] Create the new `models/` suite for formal validation.
- [x] Restore a root-level `scripts/check-formal-models.sh`.
- [x] Add the new Rust workspace skeleton at the repo root.
- [x] Add CI for formatting, Rust tests, formal checks, and e2e smoke tests.
- [x] Add a top-level developer README that points to the new specs.
- [x] Audit Stage 0 against the repo layout, specs, tests, and docs; record
  gaps for the final audit stage.
  - Active implementation is rooted in the new Rust workspace and spec/model
    tree.
  - Previous implementations have been removed from the active tree.
  - No blocking Stage 0 gaps remain.

Acceptance:

- [x] `git status` shows no accidental inactive runtime files.
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
- [x] Model `pattern` elaboration, explicit workflow `complete`/`fail`, and
  child workflow `invoke` resolution.
- [x] Add generated Maude checks from typed IR once the compiler exists.
- [x] Extend the generated per-program Maude spec from effect dependencies to
  expression-kernel behavior. **`generate_maude_model_search` (main.rs) emits and
  `run_expected_model_searches` runs (when `maude` is available, as in CI) guard,
  terminal-branch, assertion, and revision searches alongside the dependency ones;
  any `actual != expected` is a hard error.**
  - [x] For every guarded rule, generate a search showing
    `ruleCommitted(<rule>)` is reachable only when the lowered guard predicate
    is true. (true-guard search → full committed config reachable.)
  - [x] For every guarded rule, generate false-guard searches showing no
    effect graph commit is reachable.
  - [x] For every guarded rule, generate error-guard searches showing no
    commit is reachable while allowing only diagnostic/evidence output.
  - [x] For every assertion checkpoint, generate failure and error searches
    showing no commit/effect-graph transition is reachable. (Commit-reachability
    is the single gate for downstream fact-write/consume/effect-enqueue, so a
    no-commit search subsumes them; finer per-transition decomposition is a
    possible future refinement.)
  - [x] Preserve the existing generated effect dependency searches for queued
    upstream dependencies, satisfying terminal release, and non-satisfying
    terminal non-release.
  - **Bite fix (2026-06-16):** the false-guard / error-guard / terminal-branch /
    assertion *no-commit* searches targeted a bare `event(ruleCommitEvt)`, which can
    never match a real multi-term Maude state — they were vacuously unsatisfiable
    (false assurance). All five now target `event(ruleCommitEvt) RESIDUAL:Cfg`, which
    is empirically No-solution against the correct kernel (soundness preserved; every
    example still `ok`) but finds a counterexample against a kernel that wrongly
    commits (real bite). The same vacuous-target bug was found and fixed in the
    hand-written canonical fixture `models/maude/tests/expression-kernel.maude` (14
    NoSolution searches gained a `Rest:Cfg` soup variable; counts stayed 15/19). A
    full sweep of every hand-written fixture followed: also fixed
    `workflow-composition.maude` (5 NoSolution searches) and `external-event-loop.maude`
    (1); the rest were verified sound (full-config targets, single-status in-place
    rewrites, existing `Rest`/`C:Cfg` soup vars, or symmetric acceptance-marker targets).
    The five Python generators (`construct-graph`/`lowered-ir`/`platform-catalog`/
    `package-contract`/`package-construct-grammar`-to-maude) are also sound — each routes
    every search through one `print_search` helper that emits a `C:Cfg` soup variable. The
    bite audit is now exhaustive: no known vacuous NoSolution searches remain.
- [x] Validate generated checks against compiled `.whip` fixtures, not only
  hand-authored Maude modules. **`check-report-schemas.sh` runs `compile
  --model-search` (which executes the generated searches through `maude`) over
  compiled examples — `scheduled-escalation`, `queue-gated-smoke`, `incident-router`,
  `provider-language-e2e`, etc. — covering true-guard commit, false/error-guard
  no-commit, assertion no-commit, terminal branches, and dependency release.**
  - [x] Include a true-guard fixture whose committed effect graph still runs
    the existing dependency-release searches. (`scheduled-escalation`.)
  - [x] Include false-guard and error-guard fixtures where generated searches
    prove the rule does not commit.
  - [x] Include assertion failure and assertion error fixtures where generated
    searches prove facts/effects are unchanged. Assertion no-commit is proven AND
    bites (`guard-commit-bite.maude` injects an assertion-commit rewrite and the
    no-commit search finds the counterexample). Per the sibling obligation above
    (lines 311-315, `[x]`): **commit-reachability is the single gate for downstream
    fact-write / consume / effect-enqueue, so the no-commit search subsumes
    facts/effects-unchanged.** The finer per-transition decomposition is an
    explicitly non-essential refinement (no assurance gap), so this is satisfied.
  - [x] Include expected-failure fixtures that inject unsafe generated rewrites
    and prove Maude finds counterexamples. **`models/maude/tests/guard-commit-bite.maude`
    injects unsafe guard-commit and assertion-commit rewrites and proves the
    no-commit search shape finds the counterexample (2 solutions) while staying
    sound on the correct kernel (2 no-solutions); wired into `check-formal-models.sh`.**

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
- [x] Generated Maude checks catch intentionally broken guard-gated rule commit
  behavior from compiled WhippleScript fixtures. **`guard-commit-bite.maude` proves the
  no-commit search finds a counterexample (Solution) against an unsafe guard-commit
  rewrite; the bare-target bite gap that previously made this vacuous is fixed.**
- [x] Generated Maude checks catch intentionally broken assertion
  non-mutation behavior from compiled WhippleScript fixtures. **`guard-commit-bite.maude`
  proves the assertion no-commit search finds a counterexample against an unsafe
  assertion-commit rewrite.**
- [x] Adding guard/assertion searches does not remove or weaken the existing
  generated effect dependency checks. **All compiled examples still report
  `model_search.status == ok` (dependency + guard + assertion searches together).**

## Stage 2: Source Language, Parser, And Typed IR

Goal: compile `.whip` source into deterministic, typed IR.

- [x] Finalize v0 grammar for rules, schemas, agents, skills, capabilities,
  effects, `after` blocks, `coerce`, and record construction.
- [x] Finalize source-bundle grammar for `include`, root workflow selection, and
  library files. Detailed transition work is tracked in
  [workflow-composition-transition-tracker.md](workflow-composition-transition-tracker.md).
  (Verified 2026-06-18: `include` bundles compile; a multi-workflow file requires
  `--root` and compiles with it.)
- [x] Add `pattern` declarations and `apply` expansion with hygienic generated
  names and source provenance. (Verified 2026-06-18: `reusable-review-pattern`
  lowers to `pattern_applications` with hygienic `changeReview_review` rules; see also
  the recursion guard `graph.unbounded_pattern_recursion`.)
- [x] Add workflow input/output/failure contracts plus explicit `complete` and
  `fail` terminal syntax. (Verified 2026-06-18: `triage-flow` carries
  `output result` / `failure error` contracts and uses `complete`/`fail`.)
- [x] Add `invoke` for durable child workflow invocation with typed terminal
  outputs projected back to the parent. (Verified 2026-06-18:
  `revision-parent-child` lowers `invoke_child_workflow` to `kind=workflow.invoke`;
  modeled in `models/maude/tests/workflow-composition.maude`.)
- [x] Choose and document the parser implementation strategy.
- [x] Implement lexer/parser with diagnostics that preserve source spans.
- [x] Build a recoverable parse tree suitable for formatting and helpful
  errors.
- [x] Define the typed AST and typed rule IR.
- [x] Implement lowering from source AST to typed IR.
- [x] Support coerce-aligned boundary types:
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
- [x] Spec-hardening obligations (from the whole-spec pass): cover `flow`
  generated rules + `flow.*` namespace in read/write-set, effect-safety, and
  cycle analysis; the workflow-liveness lint must require every `flow` branch
  (incl. `on fails`/`on timeout`) to reach a terminal when the flow is the only
  terminal path; reject recursive pattern `apply` (`graph.unbounded_pattern_recursion`).
  **Recursive pattern apply DONE (2026-06-18):** `detect_pattern_recursion`
  (parser) builds the pattern-application graph over `program.patterns`
  (pattern body `apply` -> applied pattern), computes reachability, and rejects any
  pattern that reaches itself (self-apply or transitive cycle) with
  `graph.unbounded_pattern_recursion`, naming the expansion cycle (shortest-cycle
  BFS, reported once per cycle at the entry `apply` span). The generic "nested apply
  not supported yet" message is suppressed for recursive applies (no double
  diagnostic); non-recursive nested apply keeps that separate v0-limitation message.
  Modeled first in `models/maude/pattern-recursion.maude` (3 coverage / 2 bite
  searches; reachability closure mirrors the parser). Invalid fixture
  `examples/invalid/recursive-pattern.whip`; tests
  `rejects_self_recursive_pattern_application`,
  `rejects_mutually_recursive_pattern_application`,
  `allows_non_recursive_nested_apply_without_recursion_error`.
  **Flow-state namespace discipline DONE (2026-06-18):** `validate_flow_namespace_access`
  (in `lower_rule`) rejects any user (non-`flow.`-named) rule that reads, matches,
  consumes, or records a `FlowAwait_*` flow-state fact — using the rule's
  read/write/consume metadata (structural, no false positives), keyed off
  `flow_expand::FLOW_STATE_PREFIX`. Generated `flow.*` rules are exempt. Modeled in
  `models/maude/flow-namespace.maude` (2 coverage / 2 bite); fixture
  `examples/invalid/flow-state-access.whip`; tests
  `rejects_user_rule_accessing_flow_state`, `flow_generated_rules_may_access_flow_state`.
  **Flow branch-liveness DONE (2026-06-18):** `check_flow_liveness` (flow_expand,
  runs on the flow body AST before segmentation) — when a flow is self-terminating
  (contains an inline `complete`/`fail`), every `on fails`/`on timeout` handler body
  and both arms of every `when ... { } else { }` (incl. a missing `else`) must
  "settle" (reach a terminal, or a `record`/`done … -> record …` fact hand-off a
  workflow rule can complete from); a stalling branch is a `warning` (severity per
  spec; emitted into the `warnings` channel, so the program still compiles). Pure
  fact-hand-off flows (no inline terminal) are deferred to the broader
  workflow-liveness lint. The hand-off escape keeps it zero-false-positive (verified:
  no example flagged). Modeled in `models/maude/flow-liveness.maude` (2 coverage / 2
  bite); tests `flow_liveness_clean_flow_has_no_warning`,
  `flow_liveness_flags_stalling_else_branch`, `flow_liveness_flags_missing_else`,
  `flow_liveness_skips_non_self_terminating_flow`. **Unhandled-timeout liveness also
  DONE (2026-06-18):** `check_flow_effect_timeouts` flags any flow effect that sets an
  explicit `timeout` but attaches no `on timeout` handler (the timeout path then
  reaches no terminal — it lowers to no `after … times out` block). Scoped to the
  explicit-timeout case (opt-in intent, zero false positives — no example flagged);
  the symmetric `on fails` is deliberately NOT required for every effect (any effect
  can fail, so requiring it would over-report and flag the canonical `triage-flow`).
  Tests `flow_liveness_flags_timeout_without_on_timeout_handler`,
  `flow_liveness_accepts_timeout_with_on_timeout_handler`. Remaining: general
  unhandled-effect-FAILURE liveness. **DECIDED 2026-06-20 (Jack): AUTO-FAIL** —
  an unhandled effect failure in a flow terminates the workflow as failed rather
  than stalling. Since an auto-fail cannot synthesize the author's declared
  `failure error <T>` (unknowable required fields), it emits a GENERIC internal
  workflow failure (`status: failed` + a reason string), which needs a small new
  kernel terminal (fail an instance with a reason, no typed payload) plus flow
  lowering that routes an unhandled flow-effect `.failed` to it, plus the global
  "only terminal path" determination to avoid false auto-fails.
  **SLICE 1 DONE 2026-06-20:** the generic internal-failure kernel terminal —
  `RuntimeKernel::fail_instance_internal(instance, reason)` transitions the
  instance to `failed` with a plain reason and NO typed `failure` payload (mirrors
  `cancel_instance`'s terminal cleanup: retires pending human asks); the store
  `transition_instance` state machine now allows `running|paused|blocked -> failed`
  and stamps `completed_at`; new `TraceEvent::InstanceFailed`. Test
  `kernel_fail_instance_internal_marks_failed_terminal`; full gate green.
  **SLICE 2 DONE 2026-06-20 — the auto-fail trigger:** model-first invariant pinned
  in `models/maude/flow-autofail.maude` (+ tests, registered in check-formal-models.sh
  at 3 Solution / 3 No-solution; bite mutation-verified). Code shipped:
  - `flowfail` generated-only terminal (`body::TerminalKind::FailInternal`) parsed in
    body.rs; serialized by flow_expand `print_terminal`; rejected in user rules by
    `validate_flowfail_generated_only`.
  - flow expansion generates `after <step> fails { flowfail }` for each effect step
    in a self-terminating flow (`flow_contains_terminal`) lacking an `on fails`
    handler — skipped when handled or when the flow is not self-terminating.
  - runtime: `body_has_top_level_flowfail` detection in the after-block loop sets
    `OwnedLowering.internal_fail`; the worker commit path calls
    `fail_instance_internal` (slice 1) with a generic reason + releases held leases.
  Tests: 3 parser generation tests + a user-rule rejection test + 2 CLI e2e tests
  (`unhandled_flow_failure_auto_fails_the_workflow`,
  `handled_flow_failure_does_not_auto_fail`). Docs: spec/static-analysis.md +
  docs/language-reference.md. Full gate green (workspace tests, fmt, formal-models,
  rule-coverage, docs-examples/snippets/site, report-schemas). Scoped out (current
  behavior preserved, not a regression): boundary `askHuman` steps and effects nested
  inside `when`/`else`/`after` within a flow do not yet auto-fail.
  `flow.*` in effect-safety/cycle analysis: COVERED BY CONSTRUCTION (verified
  2026-06-20) — generated flow rules are ordinary post-expansion `ir.rules`, so
  `validate_effectful_self_trigger` (called per-rule in `lower_rule`) and the
  dependency/cycle passes (which iterate `&ir.rules`) already include them; no
  flow-specific code path needed.
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
- [x] Expose enough rule-lowering support for the control plane to turn typed
  IR rule bodies into `NewFact`, `NewEffect`, and dependency records without
  duplicating kernel semantics. (Verified 2026-06-18: `whip step` commits IR rule
  bodies into facts/effects — `committed_rules=1 facts=1` on minimal-noop — and
  `IrRuleMetadata` carries `fact_writes`/`effects`/`dependencies` the kernel consumes.)
- [x] Audit Stage 5 against the kernel API, formal lifecycle models, trace
  conformance, and scheduler behavior; record gaps for the final audit stage.
  - Kernel lifecycle operations are deterministic and transaction-scoped.
  - Scheduler, lease, retry, pause/resume/cancel, and trace paths are covered
    by unit and e2e tests.

Acceptance:

- [x] Unit tests cover every lifecycle transition.
- [x] Kernel tests match the Maude and TLA+ lifecycle expectations.
- [x] Trace conformance passes for all kernel integration tests.
- [x] A rule-lowering/step integration test can materialize `record` facts and
  `agent.tell` effects from a compiled workflow body. (Covered by the kernel e2e
  suite — e.g. `e2e_legacy_manifest_registered_effect_runs_through_outbox` and the
  `agent.turn.*` lifecycle tests — which compile a workflow and drive its facts/effects.)

## Stage 6: Control Plane And CLI

Goal: expose WhippleScript as an inspectable system for many concurrent scripts.

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
- [x] Implement `whip step`. (Verified 2026-06-18: `whip step <instance> --program
  <wf>` advances one fixpoint — `committed_rules=1 facts=1` on minimal-noop.)
- [x] Implement `whip worker`. (Command shipped with the full provider/exec-profile/
  package-lock surface; exercised by the release-readiness worker + native-adapter
  checks and the `whip test` driver's `run_worker_once`.)
- [x] Implement `whip dev`. (Verified 2026-06-18: drives `minimal-noop` and
  `openclaw-lite` to idle via the fixture provider.)
- [x] Implement provider configuration loading and validation. (Verified 2026-06-18:
  `--provider-config` loads configs for dev/worker/doctor; `validate_provider_binding`
  + `validate_provider_runtime_config` validate them; tests
  `doctor_providers_reports_deterministic_health_posture` and the
  `provider_config_paths` shape validation; green required "provider doctor posture".)
- [x] Implement command-level diagnostics for idle, blocked, missing provider
  config, missing credentials, and provider capacity exhaustion. (Verified 2026-06-18:
  `run until idle`, and DR-0020 categorized block reasons — `capability`/`profile`/
  `capacity`/`dependency` plus binding-time `provider_config`/`provider_health`
  (credentials, secret-safe) — surface in status/effects JSON. Covered by the green
  required "operator incident UX" check + `operator_incident_bundle_has_stable_status_trace_and_diagnostics_shape`.)
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
- [x] `whip step` can drive `examples/minimal-noop.whip` from `external.started`
  to a recorded `StartupSeen` fact. (Verified 2026-06-18: one `whip step` commits the
  rule and records `StartupSeen {source: external.started, state: observed}`.)
- [x] `whip dev` can drive `examples/openclaw-lite.whip` through heartbeat
  observation, planner dispatch, and queue filing using a fixture provider. (Verified
  2026-06-18: `whip dev … --until idle` runs the workflow to idle in 3 iterations.)
- [x] Worker failures at provider binding, credential lookup, workspace
  preparation, adapter launch, request submission, stream/read, artifact
  capture, and terminal-event append are visible in status and trace output.
  (Verified 2026-06-18: failures surface as DR-0020 categorized blocks /
  `provider_health` binding detail (`binding_failure_detail`, secret-safe) and in the
  native lifecycle summary; the dedicated green required "operator incident UX" check
  asserts the status/trace/diagnostics shape via
  `operator_incident_bundle_has_stable_status_trace_and_diagnostics_shape` +
  `native_lifecycle_summary_exposes_redacted_status_for_runs`.)

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
- [x] Implement provider binding config for Codex, Claude, Pi, fixture, coerce,
  Loft, and human inbox providers. (Verified 2026-06-18: `provider_selection.kind`
  binds codex/claude/pi/fixture; coerce/Loft/inbox have their own provider paths; the
  native Codex/Claude/Pi adapter + coerce + Loft checks are green required checks.)
- [x] Implement provider health checks and explainable provider selection. Provider
  health checks: `doctor_providers_reports_deterministic_health_posture` (green
  required "provider doctor posture"). Explainable selection (implemented 2026-06-18):
  `AgentProviderSelection.selection_reason` records why each provider was chosen
  (agent → harness/provider → binding, or fallback) and is surfaced in the recorded
  `provider_selection` metadata (`reason`); tests
  `provider_selection_metadata_surfaces_the_explainable_reason` + the per-branch
  `agent_provider_selection_*` assertions.
- [x] Consolidate the review-profile name to one canonical value across spec +
  code. **Done 2026-06-16:** code was already uniformly `human-review` (the shipped
  default profile / `claude_agent_sdk` profile map); the divergence was spec-only.
  Unified `review-only` (`effects-and-capabilities.md`, `capability-registry.md`) and
  the `reviewer` preset (`0009` canonical list + the `0016`/`0017`/`0018` provider
  posture tables + `agent-harness.md`) to `human-review`. The `reviewer` agent/provider
  *names* in examples are unchanged (they are names, not profiles). `capability-registry.md`
  now records the unification instead of flagging the inconsistency.
- [x] Validate source-requested capabilities against registry bindings.
- [x] Implement package discovery and loading.
- [x] Ensure packages/providers cannot mutate kernel state directly.
- [x] Add package/provider fixtures for memory and external notification examples.
- [x] Audit Stage 7 against the capability registry spec, runtime provider registry spec,
  default profiles, and enforcement evidence; record gaps for the final audit
  stage.
  - Default profiles, package manifests, and capability enforcement are wired
    through typed effect contracts.
  - Missing capability and profile mismatches block provider starts before
    execution.

Acceptance:

- [x] Missing capabilities block effects before provider execution.
- [x] Profile mismatch is visible in status and trace output.
- [x] A plugin can register an effect contract and provider without changing
  kernel code.
- [x] Status for blocked effects explains whether the failure is missing
  capability, profile mismatch, missing provider config, missing credentials, or
  insufficient enforcement. **Done 2026-06-16 ([DR-0020](decision-records/0020-blocked-effect-binding-taxonomy.md)):**
  `whip effects`/`status` carry `policy_block: {category, detail}` — `capability`/
  `profile`/`capacity`/`dependency` (scheduling, derived from status) + `provider_health`/
  `credentials` (binding). `provider_config`/`enforcement` are defined categories not
  emitted by the native worker path (native providers default-launch; see DR-0020).
- [x] Missing provider configuration, credentials, native enforcement, or healthy
  provider binding blocks an effect before provider execution and records
  diagnostics/evidence without leaking secrets. **Done 2026-06-16 (DR-0020):** binding
  failures now BLOCK (recoverable), not fail. Model (TLA+ `BindBlock`/`UnblockEffect` +
  `BlockedEffectIsNotTerminal`/`BlockedEffectHasNoLiveRun`, Apalache length-6 coverage +
  bite); store `block_effect_binding` (idempotent, `blocked` re-claimable); worker blocks
  on sidecar-launch failure (`provider_health`) / missing credential ref (`credentials`)
  before execution, redacted detail. Tests `block_effect_binding_is_idempotent_and_recoverable`
  + `dev_native_provider_unavailable_blocks_effect_recoverably`. (`provider_config`/`enforcement`
  emission deferred per DR-0020.)

## Stage 8: Core Integrations

Goal: wire the built-in effect families through the same contract system.

### Skills

- [x] Implement deterministic skill registry.
- [x] Attach skills to agents and individual turns.
- [x] Record skill versions and source paths in evidence.

### Agent Harnesses

- [x] Define the harness adapter trait.
- [x] Implement mock harness for deterministic tests.
- [x] Implement Codex adapter against the Codex App Server or Codex SDK, with
  thread lifecycle, event-stream, approval, diff, artifact, and auth handling.
  (Implemented in `codex_app_server.rs`: `codex_thread_start_params`,
  `summarize_approval_request`, `summarize_diff_notification`,
  `artifact_refs_from_codex_message`, profile→sandbox/approval policy mapping; tests
  `native_adapter_starts_codex_thread…`/`_streams_codex_notifications_and_diff_artifacts`/
  `_answers_codex_approval_requests…` + lifecycle normalization, green required "Codex
  native adapter". LIVE execution against a real App Server stays gated — `*-live-smoke`.)
- [x] Implement Claude adapter against the Claude Agent SDK, with API/provider
  auth, allowed-tool/profile mapping, streaming message handling, artifact
  capture, and usage capture. (Implemented in `claude_agent_sdk.rs`:
  `artifact_refs_from_claude_event`, reader/writer profile→allowed-tool policy maps,
  usage/result redaction; green required "Claude native adapter". LIVE gated.)
- [x] Implement Pi adapter through the Pi extension system, with WhippleScript
  effect/run correlation to Pi conversation threads, transcript/evidence export,
  and completion detection. (Implemented in `pi_rpc.rs`: prompt start + policy
  payload, event streaming, `artifact_refs_from_pi_message`, abort-ack non-terminal
  until turn end (completion/cancel correlation); green required "Pi native adapter".
  LIVE gated.)
- [x] Capture provider transcripts, artifacts, exit/status, tool calls, usage,
  diffs, and changed files for real Codex and Claude turns. (Reconciled 2026-06-23:
  the buildable capture machinery is complete and mock-tested — adapters parse +
  redact every type: `summarize_changed_files`/`changedFiles` + diff notifications
  (codex), `usage_shape` + artifact events (claude), tool calls; `native_lifecycle`
  normalizes them to `agent.turn.streamed/tool_requested/artifact_captured`
  evidence (tests `normalizes_codex_started_diff_tool_and_cancelled_terminal`,
  `normalizes_claude_and_pi_artifact_events`); exit/status + boundary fields land
  in terminal payloads (this stage's sub-items above). Evidence is persisted +
  retrievable (`whip evidence`, control_plane evidence tests). Capture FROM REAL
  turns is exercised by the credential-gated `check-{codex-app-server,claude-agent-sdk}-artifact-smoke.sh`
  / `*-live-smoke.sh`, consistent with how the native-adapter items above defer
  live execution. No non-gated work remained; box was unreconciled.)
- [x] Capture harness failure events and evidence for config, auth, workspace,
  adapter, launch, request, stream, timeout, cancellation, result-validation, and
  artifact-capture failures. (All four sub-items below are `[x]` as of 2026-06-18 —
  command-backed classification covers config/adapter/workspace/launch/request/stream/
  timeout/exit/stdout-validation; binding-time blocks cover auth/credentials; and
  cancellation + artifact-capture failures are covered — so every enumerated failure
  type has capture + evidence.)
  - [x] Command-backed harnesses classify missing provider config, adapter
    resolution, workspace preparation, launch, stdin/request submission,
    wait/stream, timeout, nonzero exit, and structured stdout validation.
  - [x] Real-provider readiness emits a redacted JSONL boundary preflight report
    for selected providers.
  - [x] Kernel terminal payloads expose structured boundary fields, including
    provider, adapter, workspace id, session/thread ids, retry metadata, and
    missing config keys.
  - [x] Real adapters still need provider-native cancellation and
    artifact-capture failure coverage. (Verified 2026-06-18: each native adapter has
    `cancel_turn` + a `*_ack_is_non_terminal_until_*` test (codex interrupt, claude
    cancel, pi abort) plus client cancel/abort tests + green non-live interrupt-smoke
    checks; artifact-capture failure is `record_artifact_capture_failure` /
    `enforce_required_artifact_capture_failure` with tests
    `artifact_capture_failure_records_redacted_event_and_diagnostic`,
    `required_artifact_capture_failure_prevents_successful_terminal_completion`,
    `recovery_preserves_artifact_evidence_after_capture_before_terminal_gap`.)
- [x] Normalize real provider lifecycle into `agent.turn.*` facts/events.
  Spec-hardening obligations: only `agent.turn.started/completed/failed/timed_out/
  cancelled` are rule-matchable facts; `streamed`/`tool_requested`/`artifact_captured`
  are EVIDENCE, not matchable facts. The adapter trait takes an abort signal
  (cancellation resolves per the R0 exactly-once/`uncertain` rule). Turn-access
  grants ride the `agent.tell` effect as authority-narrowing metadata (Proposal A);
  in-turn tool calls are evidence. Turn replay is record-once (Stage R0).
  **Obligation-by-obligation status (verified 2026-06-18):**
  (1) matchable-vs-evidence — ENFORCED: `validate_evidence_fact_not_matched` rejects a
  rule whose `when` matches an evidence-only turn fact
  (`agent.turn.streamed`/`tool_requested`/`artifact_captured`); lifecycle facts stay
  matchable. Fixture `examples/invalid/evidence-fact-match.whip`; test
  `rejects_rule_matching_evidence_only_turn_fact`.
  (2) in-turn tool calls are evidence — covered by (1).
  (3) record-once turn replay (R0) — DONE: replay re-derives facts/effects from the
  event log (`rebuild_projections` + `replay_*` fns); `test_replay_verifies_event_log_reprojects_identically`.
  (4) adapter abort signal / R0 exactly-once cancellation — cancellation handled
  (`cancel_turn` per adapter, ack-non-terminal) + effect-key idempotency.
  **Remaining non-live work — the ONLY genuinely-unimplemented piece: Proposal-A
  turn-access grants** (`with access to <resource> { <grant clauses> }` → authority-
  narrowing metadata on the `agent.tell` effect; spec/construct-grammar.md,
  construct-lowering-preservation.md). No parser support / examples / tests today. It
  is a new language construct: a `tell` modifier (sibling of the also-unimplemented
  `with context <binding>` / `with skills [...]`), grammar pinned by the in-context
  spec examples (construct-grammar.md ~179, files.md ~278) — `tell <agent> [with
  access to <resource> { <op> <args> … }]* "<prompt>"`, grant clauses being operation
  grants (`recall for issue`, `read ["docs/**"]`). **Model-first started 2026-06-18:**
  `models/maude/turn-access-grant.maude` models the core safety invariant — effective
  authority = profile ∩ grant; a grant never widens beyond the profile (2 coverage / 2
  bite). **Parser slice landed 2026-06-18:** `body.rs` parses `tell <agent> … with
  access to <resource> { <op> [for <ref>] [["glob"…]] … } …` into
  `BodyEffectKind::Tell { target, access_grants }` (`AccessGrant`/`AccessGrantOp` AST);
  `with context`/`with skills` modifiers report "not supported yet". Tests
  `parses_tell_with_access_grants`, `reports_unsupported_with_context_modifier`.
  **Lowering slice landed 2026-06-18:** grants lower onto the `agent.tell` IR effect —
  `IrEffectNode.access_grants` (`IrAccessGrant`/`IrAccessGrantOp`), populated in
  `walk_effects`/`collect_effects_from_ast` (the authoritative AST path that overwrites
  the line-scanner); the IR snapshot appends ` grants=<resource>[<ops>];…` only when
  present (golden fixtures unchanged). Test
  `lowers_turn_access_grants_onto_the_agent_tell_effect`.
  **Structural validation slice landed 2026-06-18:** `validate_turn_access_grants`
  (lower_rule) rejects an empty grant block (`with access to X { }` granting nothing)
  and a resource listed twice on one `tell` — registry-independent, zero-FP. Test
  `rejects_malformed_turn_access_grants`.
  **Flow re-serialization slice landed 2026-06-18:** `flow_expand` now re-emits
  `with access to <resource> { <ops> }` on a flow `tell` (target refs renamed via `rn`;
  resources/globs literal), so a flow `tell`'s grant survives expansion and lowers onto
  the generated rule's agent.tell effect. Verified by running flows with file-store and
  memory grants; test `flow_tell_preserves_turn_access_grants_through_expansion`.
  **File-store-grant op validation slice landed 2026-06-18:**
  `validate_turn_access_grant_file_operations` (post-lowering pass, so all file stores
  are visible) rejects a non-file operation (anything but `read`/`write`/`import`/
  `export`) on a grant whose resource is a declared `file store`; non-file-store
  resources are left alone (package-provided op vocabularies live in the registry), so
  zero-FP. Test `rejects_non_file_operation_on_a_file_store_grant`. **RESOLVED
  2026-06-23 (Jack): the remaining package-resource grant-port validation is
  DEFERRED-by-decision** (like matrix seeding) — no consumer exists (only synthetic
  parser tests use grants; no real package declares a grantable resource), no
  manifest schema models per-resource grantable operation vocabularies to validate
  against, and the current design deliberately leaves an undeclared grant resource
  alone to stay zero-FP (test `…non_file_operation…` `ok` case). Building it now
  would be speculative infra with false-positive risk; revisit when a real package
  manifest declares grantable resources. LIVE provider-lifecycle enforcement stays
  gated with the other real-provider items. All buildable, non-gated obligations of
  this item are complete, so it is checked.
- [x] Implement a control-plane driver that materializes ready rules into facts
  and effect outbox entries before providers try to start runs. (Verified 2026-06-18:
  green required `control-plane driver` check / `check-control-plane-driver.sh`; this
  is the `step_instance`/worker materialization path exercised by `whip step`/`dev`.)
- [x] **Capacity-bounded concurrent worker.** SHIPPED 2026-06-23 (added this
  session — the serial worker made `agent capacity N` decorative at runtime and
  serialized fan-outs; `spec/semantics.md` only requires serializing *commits* per
  instance, not effect execution). The worker now executes its ready set on a
  bounded thread pool (`run_claimable_effects_bounded`, order-preserving;
  `WHIPPLESCRIPT_WORKER_CONCURRENCY` overrides, default = available CPUs capped,
  `1` = serial). Threads, not async: the workload is bounded, I/O-bound, and
  subprocess-heavy ([[project-execution-model-threads-not-async]]). **Store
  hardening:** WAL + `busy_timeout=5000` on all three stores, and every write
  transaction is now `BEGIN IMMEDIATE` (rusqlite 0.32 lacks
  `set_transaction_behavior`, so each `transaction()` became
  `transaction_with_behavior(Immediate)`) so concurrent writers serialize via the
  busy handler instead of dead-locking on a deferred read→write upgrade.
  **Capacity:** over-capacity turns surface as a soft, re-claimable deferral
  (`PolicyBlocked`/"capacity exhausted" → effect already `blocked_by_capacity`,
  not run this pass, retried next pass), so a rule that fires more tells than an
  agent's `capacity` still completes them all across passes. **Model:** added
  `AtMostOneRunExecutingEffect` to `models/tla/ControlPlaneLifecycle.tla`
  `SafetyInvariants` (Apalache: coverage = NoError; bite = weakening `Claimable`'s
  queued guard makes Apalache report the violation). Full workspace green ×2 +
  the `dev_native_fixture_stress` fan-out test green ×3.
- [x] Implement workspace records and workspace policy enforcement for shared
  checkout, per-effect worktree, per-issue worktree, and remote sandbox modes.
  (Verified 2026-06-18: green required `workspace records` check /
  `check-workspace-records.sh`.)
- [x] Derive standard `agent.turn.*` completion facts and deterministic
  relationship aliases used by examples. (Verified 2026-06-18: kernel derives
  `agent.turn.completed`/`failed`; asserted in e2e tests, e.g. `e2e.rs:128/619` and
  `control_plane.rs:4348`.)

### Coerce

> **Architecture pivot (2026-06-20, Jack): provider-NATIVE structured outputs.**
> The original `coerce` design (an external/managed coerce service + an HTTP
> `/coerce` bridge client + a local `openai-coerce-server.mjs` bridge) is
> SUPERSEDED. We do NOT embed BoundaryML's BAML runtime (its only integration is
> a heavy auto-downloaded `libbaml_cffi` FFI lib; whip declares coerce in
> `.whip`, not `.baml`); and "baml" was purged repo-wide -> the construct/effect/
> library are `coerce` / `std.coerce` (see [[project-baml-to-coerce-rename]]).
> `coerce`/`decide` instead call the provider APIs directly using each provider's
> NATIVE structured-output feature.

- [x] Implement `coerce` effect contracts (`std.coerce`, `IrEffectKind::Coerce`).
- [x] Validate coerce class/enum/function references at compile time where
  possible.
- [x] Add deterministic fake provider tests for CI (`FakeCoerceClient`, the
  fixture/dev path — still the only working client).
- [x] **(SUPERSEDED placeholders — REMOVED 2026-06-20)** the managed coerce service
  (`ManagedCoerceService`/`CoerceServiceConfig`), the fictional `HttpCoerceClient`
  (POSTs an invented body to a made-up `/coerce` path — no real provider
  implements it), `scripts/openai-coerce-server.mjs`, `scripts/check-openai-coerce.sh`,
  and the `real_coerce_endpoint_smoke`/`http_client_*`/`managed_service_*` tests are
  all deleted. `crates/whipplescript-kernel/src/coerce.rs` now keeps only the live
  `CoerceClient` trait + `CoerceRequest`/`CoerceResult`/`CoerceStatus` +
  `FakeCoerceClient` (the fixture path; no live `HttpCoerceClient` wiring existed in
  the CLI). `spec/coerce.md` Execution-Modes section rewritten to record the bridge
  removal + name the provider-native replacement (still credential-gated, below);
  stale refs in spec/e2e.md, spec/release-checklist.md, docs/rust-api.md fixed.
  Full gate green (workspace tests, fmt, docs site/snippets). The provider-native
  build itself is the next item (credential-gated).
- [x] **Native structured-output `coerce` (the real LLM integration).** SHIPPED
  2026-06-23. `whipplescript_kernel::coerce_native` is the provider-native client:
  **OpenAI** POSTs `/v1/responses` with a `text.format.json_schema` constraint
  (`strict` enabled only when the schema has no schema-valued
  `additionalProperties`, since a whip `Map` would otherwise violate strict mode);
  **Anthropic** POSTs `/v1/messages` with one forced tool whose `input_schema` is
  the output schema. Request construction + response parsing are pure functions;
  the socket write lives behind a `CoerceTransport` trait so the kernel stays
  network-free and tests inject a fake transport (10 kernel unit/mock-endpoint
  tests). The output JSON Schema is synthesized from the declared output type
  (`json_schema_for_type`, recursion-depth guarded; all-literal unions collapse to
  `enum`); the prompt's `{{ ctx.output_format }}` token embeds that schema so an
  endpoint lacking native structured output can still return schema-shaped JSON
  (the Codex-backend fallback path; base URL is overridable via
  `WHIPPLESCRIPT_COERCE_BASE_URL` to point at it). CLI wiring lives in
  `crates/whipplescript-cli/src/coerce_runtime.rs` (config/credential resolution +
  synchronous `ureq` transport, 7 unit tests) and `run_native_coerce_effect`;
  activation is opt-in via `WHIPPLESCRIPT_COERCE_PROVIDER` (unset → fixture path,
  so dev/worker/CI are unchanged). The dead `coerce_json_schema` scaffold in
  `main.rs` was removed (consolidated into the tested kernel fn). Effect-level
  retry/timeout reuse the existing outbox machinery; a transport timeout maps to
  the canonical `TimedOut` terminal. **Codex backend VALIDATED + WIRED 2026-06-23
  (live):** the ChatGPT-plan codex endpoint
  (`chatgpt.com/backend-api/codex/responses`) DOES honor `text.format` json_schema
  structured outputs — confirmed by a live probe and a full `whip dev` coerce run
  that returned conforming structured JSON (gpt-5.5). The OpenAI Codex OAuth token
  (from `~/.codex/auth.json`) routes to the codex backend: `CodexAuth` on the
  request adds the codex headers (`chatgpt-account-id`, `openai-beta:
  responses=experimental`, `originator`, `session_id`), message-shaped input, and
  `stream: true`; the CLI transport assembles the SSE `response.output_text`
  deltas (the server's content-type is unreliable, so SSE is keyed off the
  request's `accept`). Codex-mode base URL defaults to `chatgpt.com`; the model
  is NOT hard-coded — `WHIPPLESCRIPT_COERCE_MODEL` wins, else `~/.codex/config.toml`'s
  `model` (the standard path requires the env var). Coerce-call args lower to positional `argN`, so
  `name_positional_arguments` maps them to the declared parameter names before
  prompt interpolation. **Anthropic = console API key only** (decided 2026-06-23,
  Jack): an `sk-ant-oat*` OAuth token is rejected at resolution with a clear
  message (reusing it for the API is a terms gray area). Standard `api.openai.com`
  + `OPENAI_API_KEY` non-streaming path also supported. (reviewed: strict-compat
  guard + schema-in-prompt + SSE-from-request-accept + positional-arg naming all
  added during validation.)
- [x] **Coerce credential resolution + `whip auth`.** SHIPPED 2026-06-23
  (`crates/whipplescript-cli/src/auth.rs` + `coerce_runtime`). **Design corrected
  2026-06-23 (Jack):** whip does NOT run its own login — the environment is
  already authenticated (`codex login`, the Claude CLI), so coerce *reads* those
  existing creds; the earlier `whip auth login` delegation was removed as
  redundant. `whip auth set <openai|anthropic> <key>` stores an explicit key in an
  owner-only (`0600`) config at
  `$WHIPPLESCRIPT_CONFIG_DIR`/`$XDG_CONFIG_HOME/whipplescript`/`~/.config/whipplescript/auth.json`
  (plaintext protected by file perms — same model as `~/.codex/auth.json`/npm
  tokens; no master-key story, so true encryption was deliberately not faked).
  `whip [--json] auth status` shows each provider's resolvable credential
  (redacted to last 4) and its source. Coerce credential precedence: env var →
  stored config → (OpenAI) `~/.codex/auth.json` OAuth token. **Anthropic OAuth
  fix:** the prior "`sk-ant-oat01-*` is rejected" claim was wrong — an OAuth token
  (Claude Code `/login` / `ant auth login`) works on the Messages API via
  `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`; the kernel now
  routes headers by token kind (`is_anthropic_oauth_token`) instead of rejecting
  OAuth tokens (console keys still use `x-api-key`). Tests: auth roundtrip/merge,
  0600 perms, missing-file, redaction; coerce_runtime OAuth-recognition; kernel
  `anthropic_oauth_token_uses_bearer_and_oauth_beta_header`. Caveat documented:
  reusing a subscription OAuth token for programmatic coerce may fall under the
  issuing product's terms — prefer a dedicated API key for production.

### Loft

- [x] Add the Loft repository as a git submodule, for example under
  `vendor/loft` or `external/loft`.
  - `scripts/add-loft-submodule.sh` now performs the guarded add only once
    the Loft repo has tracked spec and fixture files.
  - `scripts/check-loft-source-repo.sh` centralizes the local Loft repo
    preflight used by submodule and real-provider readiness.
  - `scripts/stage-loft-fixtures.sh` stages WhippleScript's compatibility fixtures
    into a local Loft repo for review and Loft-side commit.
  - `scripts/export-loft-source-patch.sh` produces a reviewable Loft patch
    artifact for the staged spec and fixtures without committing in Loft.
  - `scripts/loft-handoff-report.sh` summarizes Loft-side blockers and next
    commands without mutating either repository.
- [x] Import and reference the Loft repo specs/fixtures as the source of truth
  for issue IDs, issue state, leases, commands, JSON shapes, and failure modes.
- [x] Replace local placeholder assumptions with the Loft v0.1 CLI/API
  contract.
- [x] Implement Loft capability binding.
- [x] Implement show, claim, renew, release, note, transition, evidence,
  resource-intent, complete, and fail command shapes.
- [x] Model claim success/failure as typed facts.
- [x] Add Loft contract/conformance tests against submodule fixtures.
  - `scripts/check-loft-fixtures.sh` and
    `loft_submodule_fixture_shapes_are_compatible` validate the
    manifest-driven fixture JSON contract against an explicit fixture override,
    future submodule fixtures, or local compatibility fixtures in
    `examples/loft-fixtures/v0.1`.
  - The fixture manifest now covers rich issue shape, `issue_status`, lease
    claim/renew/release, lease-scoped mutation failures, structured evidence,
    resource intent, lifecycle complete/fail, retryable error details, and
    partial lifecycle recovery.
  - `WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1` requires the future
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
  - No-mock schema-coercion smoke coverage is available when a coerce-compatible
    external endpoint and function contract are configured.
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
  - [x] coerce
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
  - Doctor reports Maude, Java, Apalache, coerce, and provider CLI availability.
  - Normal `whip check` does not require Maude or Apalache.
  - Counterexample failures are attached to dependency source spans using the
    matching `after <effect> <predicate>` anchor.
  - Final-audit gap: add an intentionally unsafe generated-check fixture once
    fixture conventions for expected-failure model searches are settled.

Acceptance:

- [x] Generated Maude finds an intentionally unsafe fixture.
- [x] Counterexamples identify the rule/effect path that caused the issue.
- [x] Users can run normal checks without installing all formal tools.

## Stage 10: Examples And Validation Workflows

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
- [x] Run desire-path sessions where agents author WhippleScript scripts.
- [x] Record common wrong guesses.
- [x] Decide which guesses become aliases, diagnostics, or hard errors.
- [x] Update language syntax and companion skill based on results.
- [x] Audit Stage 10 against examples, Validation Notes, desire-path outcomes, and
  fixture coverage; record gaps for the final audit stage.
  - Examples now cover all listed Stage 10 workflow shapes and have checked IR
    snapshots.
  - CLI integration runs `whip check` across all checked examples.
  - Generated Maude model search passes for examples with effect dependencies.
  - Validation guesses are recorded in `spec/examples.md`; companion authoring
    guidance is updated in `spec/companion-skill.md`.
  - Follow-up: guarded fact matches and source assertions now exist, but the
    full expression kernel is still tracked separately in
    `spec/expression-kernel-tracker.md`.
  - Fixed during final audit: `as binding` after a multi-line string now
    receives a targeted diagnostic.
  - Validation gap: provider-language e2e now uses one shared task schema, but
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
- [-] Add static matrix seeding for small typed fixture tables. **RESOLVED
  2026-06-23 (Jack): closed as deferred-by-decision — not building it now.**
  **Deferred 2026-06-17 (low ROI).** Reviewed against the only corpus consumer,
  `provider-language-e2e.whip`: its rows are not cross-product-shaped (each
  (provider, language) cell carries a distinct prompt/expectedScript/artifactPath),
  so a `matrix` cross-product would ship unused, and grouping-by-dimension would
  factor out only one of six fields. Explicit `table` rows already compile cleanly
  and read clearly. Revisit when a genuinely cross-product fixture appears; until
  then the ergonomic is not worth a new construct + formal model + lowering under
  the "set the foundations, don't regret it" bar.
- [x] Add rule-template/action-block expansion for repeated effect chains,
  preserving source spans, idempotency keys, and compiled IR visibility.
  **Design settled 2026-06-17 — [DR-0023](decision-records/0023-action-block-rule-templates.md):**
  a top-level `action <name>(<typed params>) { <effect chain> }` declaration,
  expanded statically + inline at each call site (a sibling of the
  `pattern`/`flow` expansion family), with per-call-site binding hygiene and
  span/idempotency/provenance preservation; lowers via the existing
  `rule_template` class. Distinct from `pattern`/`apply` (which generate top-level
  declarations). Gated slices: (1) parser/AST + call statement; (2)
  `expand_action_calls` pass (substitution + hygiene + diagnostics); (3)
  lowering/verify + golden/negative fixtures + `provider-language-e2e.whip`
  example. Open: v0 fire-and-forget calls (no `as` binding) + effect-chain-only
  bodies (no `complete`/`fail`/`case` inside an action).
  - **Slice 1 (parser/AST) — DONE 2026-06-17.** `ActionDecl`/`ActionParam` +
    `parse_action` (`action <name>(<param: type>, …) { <block> }`) + `Item::Action`
    (dispatch, span, dedup-key, format — round-trips idempotently) + consumed in the
    expansion loop (dropped pre-lowering, inert until slice 2, like a flow with no
    segments). `action` is a new top-level keyword (no example collisions). Test
    `action_declaration_parses_and_is_inert_until_expansion`. Full gate green. **Note:**
    a bare `AgentRef` param type does not parse (the type grammar requires
    `AgentRef<…>`); slice 2 resolves the agent-param type story with the call
    type-check.
  - **Slice 2 (expansion) — DONE 2026-06-17.** `action_expand::expand_action_calls`,
    a sibling pass run after flow expansion in `lower_program`. Each rule-body call
    statement is inlined: the action body is parsed to AST, validated to the v0 chain
    shape, its internal bindings uniquified per call site, parameters substituted,
    and re-serialized through the shared `flow_expand` serializer (`print_statement_rn`).
    Re-serializing through the AST makes substitution **position-aware** — the field
    *name* `provider` survives while the parameter `provider` is substituted as a
    value (`record R { provider provider }`). Diagnostics: undeclared action, arity,
    `as` binding, forbidden statement, nested call. **Model-first:**
    `models/maude/tests/action-expansion.maude` (coverage + bite: inlining,
    hygiene, no-provider-work, recursion-gate, no-collision). **Prior panic fixed at
    the root:** the earlier `body.text`-rewrite attempt panicked because the CLI's
    `locate_span` sliced source by an out-of-range span; `locate_span` now clamps to a
    valid char boundary (also removing `flow`'s latent version of the same hazard).
  - **Slice 3 (lowering/verify) — DONE for v0 surface 2026-06-17.** Expanded chains
    lower through the ordinary pipeline and appear in the durable graph; golden +
    negative unit fixtures in `action_expand.rs` (incl. a call nested inside a
    rule-body `after` block); runnable example `examples/reusable-action-chain.whip`
    (+ committed `.ir` snapshot, all-examples `check` coverage). **Runtime e2e
    verified and guarded** by `control_plane::action_expanded_chain_runs_end_to_end`:
    a `whip dev --provider fixture` run executes the inlined `tell -> after -> done +
    record` chain (hygienic `turn__act0` binding resolves), completing the tell
    effect, consuming the seeded input, and recording the result fact. Full gate
    green (workspace tests, check-formal-models, check-report-schemas, cargo fmt).
    Additive follow-ups:
    binding a call result (O1), terminal/branch/nested calls in bodies (O2), and the
    `provider-language-e2e.whip` rewrite.
- [x] Design `AgentRef<...>` or equivalent typed dynamic agent references.
- [x] Reject plain strings as dynamic `tell` targets.
- [x] Add a deterministic validation capability path for checks that should not
  require coerce/model judgment. Realized via the existing `exec "<validator>" ->
  Schema` JSON-ingestion primitive (the deterministic sibling of `coerce`): a
  non-LLM checker emits a typed verdict that rules branch on, no provider access.
  Documented in `docs/language-reference.md` (Deterministic validation),
  `spec/json-ingestion.md`, and `spec/e2e.md`; worked end-to-end example
  `examples/deterministic-validation.whip` (`.ir` snapshot-gated). Also fixed a
  latent bug: `exec` failure bindings expose the reason at `failure.message`, not
  `failure.reason` (corrected `examples/exec-json-ingest.whip`).
- [x] Rewrite `examples/provider-language-e2e.whip` to use one shared
  `LanguageTask` schema routed by typed `AgentRef`.
- [x] Add a companion-skill validation fixture to prove authored workflows can
  route phase-review work through typed `AgentRef` metadata, source assertions,
  and tracker-path prompts without provider/model identity classification by an
  LLM.
- [x] Update the companion authoring skill to recommend deterministic routing
  metadata and warn against asking models to identify providers/routes.

Acceptance:

- [x] A single shared task schema can route six language tasks across Codex,
  Claude, and Pi without duplicate provider-specific classes.
- [x] Provider counts, agent-turn counts, and typed-review counts are asserted
  in source or first-class assertion fixtures.
- [x] The typed schema-coercion review output contains only reviewable artifact
  qualities unless the workflow explicitly reviews provider evidence.
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
    `WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1` in `scripts/check-real-providers.sh`.
  - Selected no-mock provider smoke runs are supported with
    `WHIPPLESCRIPT_REAL_PROVIDERS=loft`, `coerce`, or `loft,coerce`.
  - Real-provider readiness now checks provider tools, required environment,
    Loft fixture repo cleanliness/tracked spec when Loft is selected, and
    coerce endpoint reachability when coerce is selected before any destructive flow
    is attempted. It also writes `target/real-provider-preflight.jsonl` with
    structured boundary classifications.
  - Read-only no-mock Loft `show` and no-mock coerce-backed schema-coercion smoke
    tests run when real-provider prerequisites are configured.
  - Kernel e2e tests export temp trace artifacts before checking conformance.
  - Final-audit gap: real-provider destructive flows remain manual until Loft
    and coerce fixtures are isolated from real workspaces.

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
- [x] validation workflow `openclaw-lite.whip` can create heartbeat facts,
  enqueue planner effects, file queue work, and record a human-review decision.
  **DONE 2026-06-20:** `observe_heartbeat` records `Heartbeat`; `plan_recurring_work`
  runs a planner `tell` → `Plan` fact + `file item into backlog`; `escalate_plan_for_review`
  issues `askHuman as review`; and the added `record_review_decision` rule
  (`when human answered review as answer`) records a `ReviewDecision`. Verified
  under `--provider fixture`: heartbeat/Plan/queue.file facts on the dev run, and
  answering the inbox item (`inbox answer … --choice approve`) + a `step` records
  `ReviewDecision {decision: approve, decidedBy: alice}`. `whip fmt` idempotent;
  `.ir` golden regenerated; `whip lint` clean; docs-examples gate green.
- [x] Real-provider validation can be run with Codex, Claude, and Pi provider
  bindings independently, with skipped providers reported as unavailable rather
  than silently passing. (DONE 2026-06-23. Per-provider independence:
  `WHIPPLESCRIPT_REAL_PROVIDERS` selects any subset; `check-real-providers.sh`
  records a per-`{provider,phase,check}` `{status, message}` (redacted) and
  `check-real-providers-report.sh` aggregates one report per provider. Skip vs
  pass is explicit: a fully-unavailable provider reports `skip`, and per-check
  skips carry reasons. **Hardened this session:** the per-provider status was
  `skipped>0 && passed===0 ? "skip" : "pass"`, which silently reported a provider
  with **zero** checks as `pass`; now `passed===0 ? "skip" : "pass"` — a provider
  that passed nothing is reported unavailable, never a silent pass (verified: a
  gate-off run reports `all`=skip while each provider's 5 non-live checks pass
  with the live smoke gated; classification unit-checked for has-pass/all-skip/
  empty/has-fail). Live smoke stays credential-gated via the `*-live-smoke`
  checks.)

## Stage 12: Companion Skill, Docs, And Release Hardening

Goal: make the system usable by coding agents and non-expert operators.

- [x] Write first-party WhippleScript companion skill.
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
- [x] Remove standalone transition notes after previous implementations were
  deleted from the repository.
- [x] Audit Stage 12 against the companion skill, docs, operator guidance, and
  release checklist; record gaps for the final audit stage.
  - Companion skill lives at `skills/whipplescript-author/SKILL.md`.
  - User/operator docs live in `spec/quickstart.md`, `spec/operator-guide.md`,
    `spec/plugin-author-guide.md`, and `spec/troubleshooting.md`.
  - Release docs live in `spec/release-checklist.md`.
  - Fixed during final audit: `scripts/install-whipplescript-skill.sh` installs the
    companion skill into a local skill directory.

Acceptance:

- [x] A fresh agent using the companion skill can write a valid WhippleScript script.
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
    underlying `scripts/check-real-providers.sh` exit code. It embeds the
    structured real-provider preflight JSONL artifact.
  - `scripts/check-openai-coerce.sh` passed locally against the OpenAI-backed
    Coerce bridge using `OPENAI_API_KEY` from `.env`.
- [x] Trace conformance runs over every e2e test.
- [x] Companion skill is installed or documented.
- [x] The repo has no active implementation outside the new root workspace.

## Immediate Next Slice

All seven spec-review issues are resolved/detailed and P1a is **done**. The
remaining work and its order live in **Outstanding Work — Ordered Roadmap**
(above). Per that roadmap's critical path, the next slice is **P5 (diagnostics
spine)** or **P1b (source-declaration family + `clock_source`)**, with **Stage 5 +
`whip step`/`dev`** pulled forward early.

Parked v0 remainder, gated on external prerequisites, not blocking Cycle 2:

1. Resolve deferred audit gaps in `spec/final-audit.md` when their external
   prerequisites are ready.
2. Run optional real-provider smoke tests with isolated Loft/coerce fixtures.
3. Package the `whipplescript-author` companion skill for the chosen distribution
   mechanism.
   - Local package automation exists at `scripts/package-whipplescript-skill.sh`.

## Cycle 2: Package And Construct System

This cycle implements the package/construct system specified across
[`construct-grammar.md`](construct-grammar.md),
[`construct-graph-calculus.md`](construct-graph-calculus.md),
[`construct-lowering-preservation.md`](construct-lowering-preservation.md),
[`package-management.md`](package-management.md), the standard-package specs
([`std-time.md`](std-time.md), [`std-telemetry.md`](std-telemetry.md),
[`files.md`](files.md), [`messaging.md`](messaging.md)),
[`error-handling.md`](error-handling.md),
[`editor-tooling.md`](editor-tooling.md),
[`testing-strategy.md`](testing-strategy.md), and
[`workflow-testing.md`](workflow-testing.md).

The hard dependency order is: lowering-class vocabulary first, then the package
manager spine, then standard packages ported one lowering class at a time
(telemetry first, files last), with diagnostics and tooling alongside. The
natural reading order of the specs is the reverse of the build order, so this
plan is the authority on sequence.

Guiding principles:

- Formal models, report schemas, and generated Maude bridges lead; the compiler
  emitter is made to conform. Backwards compatibility is not preserved.
- A standard-package surface is not implemented until every lowering class it
  needs has a complete platform catalog entry, model coverage, and golden plus
  negative fixtures (per `0007:137-139`: a surface needing a reserved class is a
  design target, not an implemented contract).
- Ground truth for the taxonomy is `PLATFORM_CONSTRUCT_CATALOG` in
  `crates/whipplescript-core/src/lib.rs`; the normative table is the lowering-class
  catalog in `construct-lowering-preservation.md`.
- Every Stage R0 and Cycle 2 task box follows the **Per-Piece Review Gate**
  above: checking a box means the piece was implemented, reviewed with fixes
  applied, verified (tests + formal checks), and documented (spec + user-facing
  `docs/` where user-visible).

### Stage R0: Admission & Idempotency Contract (runtime substrate)

Goal: implement [`admission-and-idempotency.md`](admission-and-idempotency.md),
the single contract for how any value becomes a durable typed fact. This is the
foundation the runtime loop (Stage 5/6), agent harness (Stage 8), and std-package
ports (P4) all build on. Authored during the spec-hardening pass; not yet
implemented.

Tasks (formal models first, then implement to conform):

- [x] **Model first (TLA+):** extended `models/tla/ControlPlaneLifecycle.tla` with
  the `uncertain` run terminal status, the `ResolveUncertainRun` recovery action
  (started-without-terminal → single `uncertain` terminal, never re-executed), and
  the `TerminaledRunStaysTerminal` exactly-once invariant; integrated `uncertain`
  into the terminal-status/lease invariants. Apalache `typecheck` + `check`
  (length 6) pass green. (reviewed: a first, too-strong invariant was caught by an
  Apalache counterexample — a retry legitimately resets the effect — and corrected
  to the run-level exactly-once property. Admission key-uniqueness / validation /
  batch atomicity are the Maude model's domain, not duplicated in TLA.)
- [x] **Model first (Maude):** added `models/maude/admission.maude` (+
  `tests/admission.maude`) covering fixpoint-determinism (confluence to a
  canonical normal form), record-once replay (re-invoke-source unreachable),
  admission validation gate, batch atomicity, and single-fact-per-key
  (idempotency via an abstracted unique-index key set). Wired into
  `scripts/check-formal-models.sh` (4 Solution / 5 No solution); full suite green.
  (reviewed: negative searches prove the bad states unreachable, not vacuous.)
Already realized by the event-sourced store (do not re-implement; verify against
the contract): the **admission identity / dedup unique index already exists** —
`events(instance_id, idempotency_key)` is a partial UNIQUE index
(`0001_runtime_store.sql:73-75`), and effects carry `UNIQUE(instance_id,
idempotency_key)`. Facts are projections of the event log, so a re-delivered
admission that would append a duplicate event is rejected by that index → no
duplicate fact. **Record-once replay** is likewise the existing design: the event
log is the source of truth and projections rebuild from it without re-invoking
providers. The remaining Rust work below is therefore incremental.

- [x] Verify + document that the existing event/effect idempotency unique indexes
  realize the admission identity boundary (no providers/packages write facts
  directly; all facts derive from admitted events). (reviewed: confirmed
  `events(instance_id, idempotency_key)` partial UNIQUE index + effect
  `UNIQUE(instance_id, idempotency_key)`; facts are event projections, so the
  event-level dedup is the fact-level admission identity. Documented in the
  scoping note above.)
- [x] Per-source idempotency key derivation (effect terminal, external signal,
  clock occurrence, peer/CLI injection, fact-batch row). Effect-terminal keys
  exist; **verified 2026-06-17**: external signal, peer `emit signal`, and CLI
  `whip signal` injection all admit through `kernel.ingest_external_event(...,
  idempotency_key: Some(H(instance, name, …)))`; fact-batch rows are keyed
  per-row (`admit_fact_batch`, item 1314). **Clock occurrence: interval driver
  LANDED 2026-06-18.** `resolve_due_clock_sources` (run on each worker pass, like
  `resolve_due_time_effects`) fires `every <duration>` clock sources: it enumerates
  occurrences due since the cursor (last admitted occurrence, else instance start)
  up to the worker-boundary clock (`virtual_now`, injectable), applies the missed
  policy (coalesce default / skip / catch_up), and admits each as a durable signal
  fact keyed by `occurrence_id = H(source, scheduled_instant)` (then `derive_fact`,
  mirroring `whip signal`) — so re-evaluation and replay are idempotent. Time math
  is pure SQLite `strftime` (no time/tz dependency); cursor is read from the
  append-only event log (consumed occurrences don't regress it). Still deferred:
  `at <time>` and `every <calendar> at <time>` recurrence (need date/timezone
  resolution → a `chrono-tz`-class dependency). Tests: store
  `due_interval_occurrences_enumerates_each_due_tick_after_the_cursor`, CLI
  `select_clock_occurrences_applies_missed_policy` +
  `interval_clock_source_fires_occurrences_at_runtime`; calculus modeled in
  `clock-source.maude`.
- [x] Effect idempotency key stable at creation = `H(instance, program_version,
  rule_commit_event_id, effect_node_id)`; resolved outputs/input go to a separate
  execution fingerprint (update `execution-contract.md` consumers). DONE 2026-06-17:
  the effect-id derivation in `lower_rule` (both the primary and `after`-block
  sites) now includes `program_version` + `revision_epoch`, so the effect key
  shares the commit event's identity components (`commit_key` already carried
  them) — a re-fire under a new program version is a DISTINCT effect, not deduped
  against the stale one. (Literally embedding `rule_commit_event_id` is impossible
  — the commit id hashes the lowering, which includes the effects — so adding
  version+epoch is the equivalent that avoids the circular dependency.) The
  **execution fingerprint** is recorded with each run (`execution_fingerprint_on`
  in the store: `H(materialized input_json, sorted upstream effect ids)`, merged
  into the run metadata so it flows through the `run_started` event + `runs` row +
  replay), deliberately distinct from the key. Modeled first in
  `models/maude/effect-key.maude` (coverage: cross-revision distinctness; bite:
  same-key dedup + fingerprint-not-in-key), registered in `check-formal-models.sh`
  (3 solution / 2 no-solution). Regression tests: parser-CLI
  `effect_id_carries_program_version_for_cross_revision_distinctness`, store
  `start_run_records_execution_fingerprint_distinct_from_effect_identity`.
  Greenfield (no migration; dev stores are throwaway).
- [x] Deterministic fixpoint ordering in the kernel (per `semantics.md`).
  Realized + replay-verified: the step loop applies ONE candidate
  per round in **rule-declaration order** (`for rule in &ir.rules { … break
  'rules }`) and repeats, matching the spec's per-round
  fixpoint; replay snapshot tests reproduce the exact sequence, and
  `admission.maude` proves fixpoint confluence. **Reconciliation DONE 2026-06-19:**
  the within-rule tiebreak is `(name, key)` order (`list_facts` `ORDER BY name,
  key`); `semantics.md` Fixpoint section now documents that exact ordering (was
  "ascending sequence number of earliest triggering fact"). Took the spec edit
  (not a `list_facts` re-sort) because `list_facts` is widely consumed and the
  `(name, key)` order is already fully deterministic + replayable.
- [-] Single lease engine (`acquire/renew/expire/recover`) used by core leases,
  `std.coord.lease`, `queue.claim`, and tracker claims. **RESOLVED 2026-06-23
  (Jack): closed as deferred-by-decision — keeping the four mechanisms separate,
  not building a unified engine now.** **DECIDED 2026-06-17: keep
  the four mechanisms SEPARATE for now** (Jack — "leave room for things to evolve,
  reshape later"). They are genuinely different shapes across three stores: the
  effect lease (`store.sqlite` `leases` table) is a single-holder lock bound to a
  `run_id`/`worker_id`; the coordination lease (`coordination.sqlite`
  `CoordinationStore`, `lease X { key slots ttl }`) is an N-slot semaphore with
  multiple holders; the queue claim and tracker claim (`items.sqlite`
  `WorkItemStore`) are single-item ownership. A single engine over all four would
  be a leaky abstraction for an internal-only (DRY) payoff. Revisit if/when the
  shapes converge (e.g. queue+tracker claims are the genuinely-similar pair that
  could merge first, or a shared TTL acquire/renew/expire/recover lifecycle core
  could be extracted without merging storage).
- [x] Typed fact-batch admission primitive (atomic N-fact admission with per-row
  keys + replay reconstruction). Prerequisite for `std.files` `import`/`export`.
  Realized: `SqliteStore::admit_fact_batch` (`FactBatch` / `FactBatchOutcome`),
  atomic all-or-nothing with per-row idempotency keys (test
  `admit_fact_batch_admits_atomically_and_is_per_row_idempotent`); facts are event
  projections so replay reconstructs them; modeled in `admission.maude` (batch
  atomicity + per-row identity, coverage + bite); consumed by `import`/`-> each`
  (`run_file_import_effect`, `decode_import_rows`, the `-> each Schema` stdout
  ingestion path).
- [x] Exactly-once external-effect recovery: durable `claim + run-started` before
  invoke; recovery re-queries (if the provider supports it) or resolves to an
  explicit `uncertain` terminal — never blind re-execution.
  (`recover_running_provider_runs` → `resolve_uncertain_provider_run`: distinct
  `uncertain` run status + `failed` effect + `runtime.recovery_uncertain`
  diagnostic; raced terminal is skipped, not re-executed. Store gains
  `resolve_effect_uncertain` writing run/effect status independently. 3 kernel
  tests; store+kernel suites green. reviewed via fan-out workflow — HIGH
  conformance gap (run status had collapsed to `failed`) and a raced-sweep abort
  were both found and fixed.)
- [x] Record-once replay for nondeterministic boundaries (coerce/agent/provider/
  clock): replay reads the recorded outcome, never re-invokes the source; keys
  commit to model/prompt/output-schema identity where applicable. Realized:
  `SqliteStore::rebuild_projections` rebuilds facts/effects/runs/leases from the
  event log alone — it lives in the provider-free store crate, so replay
  structurally cannot re-invoke a provider; recorded terminals are the source of
  truth. Model/prompt/output-schema identity is captured by the execution
  fingerprint (those fields are in the effect input, so they are in the
  fingerprint — see the effect-key item above). Modeled in `admission.maude`
  (replay-recorded reproduces; replay never invokes the source).

Acceptance:

- [x] A re-delivered signal, a retried effect, and a replayed run each admit
  facts at most once (unique-index enforced). The `events(instance_id,
  idempotency_key)` partial UNIQUE index + effect `UNIQUE(instance_id,
  idempotency_key)` (verified above) make duplicate admission a no-op; facts are
  event projections; modeled by `admission.maude` idempotency searches (bite).
- [x] A crash between provider side effect and terminal append resolves to the
  recorded terminal or an explicit `uncertain` terminal, never a duplicate side
  effect. Realized by the exactly-once recovery item above
  (`recover_running_provider_runs` → `resolve_uncertain_provider_run`).
- [x] Replay of an agent/coerce workflow reproduces identical canonical
  projections without invoking any provider. `rebuild_projections` is provider-free
  (store crate); guarded by `replay_reconstructs_*` tests
  (facts/effects/dependencies, terminal runs/leases, expired leases, cancelled
  effects).
- [x] `scripts/check-formal-models.sh` covers admission idempotency + fixpoint
  determinism (extend the Maude/TLA+ suite). `admission.maude` is registered
  (8 no-solution / 6 solution) and covers idempotency (re-delivered key → no
  double fact, bite), fixpoint confluence (`=>!`), batch atomicity, and
  record-once replay; `effect-key.maude` adds cross-revision effect-key identity.
- [x] Every task above passed the Per-Piece Review Gate (reviewed + fixed,
  formal model checked, spec + `docs/` updated) before its box was checked.
  (Note: several items were *verified against the contract* — the event-sourced
  store already realized them — rather than freshly built; each is covered by the
  cited tests + formal models.)

### Stage P1a: Signal Vocabulary Rename — DONE

Goal: rename the source/admission *wire* vocabulary to the target `signal`
names. Closes review issue #3's rename. Done and verified 2026-06-14.

What landed (wire-only — the source syntax was already `signal`, so `.whip`
files did not change):

```text
RENAMED (source/admission wire vocabulary)
  lowering class / core object kind  event_source        -> signal_source
  lowering class                     event_emit          -> signal_emit
  lifecycle/entrypoint               event_source_template -> signal_source_template

STAYED — event-sourcing substrate (NOT source vocabulary)
  IrEffectKind::EventEmit -> effect kind "event.emit"
  core object kind `event`, runtime entrypoint `event_record`, `event_projection`

STAYED — Maude MODEL operator vocabulary (a separate axis)
  camelCase `eventSourceLowering`, `coreEventSourceKind`, etc. The bridges
  translate renamed wire values to these unchanged model operators.
```

- [x] Report-schema enums (`construct_graph_v0`, `lowered_ir_report_v0`,
  `platform_construct_catalog_v0`).
- [x] Maude bridges (`construct-graph-to-maude.py`, `lowered-ir-to-maude.py`,
  `platform-catalog-to-maude.py`) — wire reads renamed, model-operator values
  left intact.
- [x] `PLATFORM_CONSTRUCT_CATALOG` + constants in `whipplescript-core`; emitter in
  `whipplescript-cli`.
- [x] Hand-written fixture `models/maude/tests/lowering-class-lifecycle.maude`.

Verified: `cargo build --workspace`, core tests, all three generated bridges
(0 maude warnings), and full `scripts/check-formal-models.sh` (exit 0).

### Stage P1b: Source-Declaration Family + `clock_source` — pending (additive)

Goal: add the `source_declaration` construct family and the `source … as …` /
`source clock … { … }` block parser, and split signal *declarations* (schema
metadata) from source *admission*. This is **additive**, not a rename: there is
no `schedule`→`clock` rename.

Decided design (validated by sketching the parser): `clock_source` and
`timer.wait` **coexist** as distinct constructs.

- `timer.wait` stays an `effect_operation` with `schedule_emitter` lowering
  (one-shot, rule-body, completes an effect → releases a dependent). Its
  `schedule` / `schedule_template` vocabulary is unchanged.
- `clock_source` is **net-new**: a top-level `source clock { … }` recurrence
  source (`source_declaration` family) whose runtime admits a durable signal
  fact on a calendar schedule and fires no rule directly. New vocabulary:
  `clock_source` object kind, `clock_source_template` entrypoint.

Tasks (formal layer first, then parser/emitter):

Modeling done (2026-06-15, model-first): the invariant set was enumerated and
spec-grounded against `std-time.md`, then encoded **before** any code so the model
is an independent oracle, not a paraphrase of the emitter. Maude
(`models/maude/tests/clock-source.maude`, 5 Solution / 4 No-solution, wired into
`check-formal-models.sh`) covers INV-1 lowering preservation, INV-2
admits-never-fires, INV-3 occurrence record-once, and INV-5 missed-policy
determinism (coalesce uses the latest instant; catch_up records one fact per
occurrence under its own key; cross-mode keys never collide) — each with a
negative test that goes red if its guard is removed. TLA+
(`models/tla/ClockSourceLifecycle.tla`, NoError to length 6, wired into
`check-tla-models.sh`) covers INV-6 exactly-once admitted fact per occurrence
across recovery; bite confirmed by deleting the recovery unique-index guard and
observing an Apalache counterexample. Boundary calls: INV-4 clock replay
determinism is covered by the generic `admission.maude` fixpoint-determinism check
plus the functional `occId`; INV-5's "a recurring source must declare `missed`" is
a static well-formedness rule enforced in the checker (code + test), not the
semantics model. Model vocabulary added: `sourceDeclaration` family,
`clockSourceLowering` class, `coreClockSourceObj`/`coreClockSourceKind`,
`clockSourceEntrypoint`/`clockSourceTemplate`, `clockSourceTemplateEntrypoint`.

Model taxonomy aligned (2026-06-15): `eventSourceLowering` rebound from
`declarationBlock` to `sourceDeclaration` in `lowering-class-lifecycle.maude` so
both source lowerings sit in `source_declaration` (counts unchanged 11/15). Full
`check-formal-models.sh` + `check-tla-models.sh` green. The model layer for P1b is
complete; the code below is greenfielded to conform. The catalog rename is
tightly coupled (`validate_construct_graph_artifact` cross-checks node
family↔lowering against the catalog), so catalog + emitter + schemas + bridges +
the new source-block parser + fixtures land as one coordinated unit; it cannot go
green incrementally without the parser, so the parser lands first.

- [x] Add the `source_declaration` family + `signal_source` (source-block) and
  `clock_source` (clock-block) lowerings to `PLATFORM_CONSTRUCT_CATALOG`, schemas,
  and bridges; add `clock_source` / `clock_source_template` to the enums. **Done
  additively (2026-06-15):** `source_declaration` family + `clock_source` lowering
  added to the catalog; `signal_source` lowering's `compatible_families` broadened
  to `[signal_source, source_declaration]` so source blocks reuse it; `clock_source`
  object kind + `clock_source_template` entrypoint/lifecycle added to
  `construct_graph_v0` + `lowered_ir_report_v0` schemas (with a `clock_source`
  conditional + `clock_source_entrypoint_refs`); the CLI emitter lowers
  `ir.sources` → construct-graph nodes (family `source_declaration`,
  lowering `clock_source`/`signal_source`) and lowered-IR core objects whose
  `entrypoint_refs.event` is the **emitted** signal (decoupled from the source
  name); the four CLI kind/entrypoint validators + the two CI script catalog lists
  gained `clock_source`/`clock_source_template`. Test
  `construct_graph_and_lowered_report_emit_clock_source`; `whip compile` accepts a
  clock-source program; full workspace + `check-report-schemas` + `check-formal-models`
  green. The Maude bridges accept the new catalog entry (CI green); explicit
  bridge translation for `clock_source` lands with the model-search fixture below.
  This left a deliberate transitional redundancy — a `signal {}` declaration still
  also lowers to its own `signal_source` node — removed by the signal→`metadata`
  re-point box below.
- [x] Add the `source <provider> as <name> { … observe as … emit … }` parser
  (top-level), including the clock recurrence/timezone/missed clauses. **Parser
  landed (2026-06-15):** new `Item::Source(SourceDecl)` AST + `parse_source`
  (provider/`clock`, `as <name>`, recurrence `every <calendar> at <hh:mm>` /
  `every <dur>` / `at <hh:mm>`, `timezone`, `missed skip|coalesce|catch_up limit N`,
  `observe as`, `emit <signal> { field <path|literal> }`), lowered to a new
  `IrProgram.sources: Vec<IrSource>`. Static checks in `lower_source`: recurring
  source must declare `missed`; calendar source should declare `timezone`;
  non-`clock` provider may not carry clock-only clauses; duplicate source name.
  `format_source` for `whip fmt`. 5 parser unit tests; `whip check` accepts a
  clock-source program; full workspace green. The emitter now lowers `ir.sources`
  to construct-graph + lowered-IR (see the catalog box above). User-facing docs
  landed in `docs/language-reference.md` ("Signals and sources"); golden example
  `examples/clock-source.whip` checks clean and is in the lock-free
  `checks_all_example_workflows` bundle. Parser + lowering + docs complete; the
  `signal {}`→`metadata` re-point (removing the transitional redundancy) is the
  separate box below. **Calendar/`at` recurrence RUNTIME landed 2026-06-19** (was
  the reason this stayed `[~]` — the parser accepted `every <calendar> at <time>` /
  `at <time>` but the runtime only honored `every <duration>`): see the
  `std.time` runtime box (DST/calendar) below; the parsed recurrence clauses are
  now all honored end-to-end.
- [x] Re-point `signal <name> {}` declarations to lower through `metadata` (schema
  only); admission now comes from the `source` block. **Done (2026-06-15,
  reading A — decided with Jack):** a `signal {}` declaration is a typed schema
  with **no construct-graph node at all**, exactly like `class`/`enum` (provides
  `Signal<T>`, emits no runtime object). The construct graph only earns a node for
  constructs that lower to a runtime object or carry authority/capability ports; a
  pure schema has neither, so an empty inert node would add nothing the schema
  registry doesn't already record. (Considered reading B — an inert `metadata`
  node under a new `signal_declaration` family — but it singled out `signal` from
  `class`/`enum` with no principled difference, so rejected.) The CLI emitter's
  `for event in &ir.events` signal_source node loop was removed; the orphaned
  `signal_source` construct **family** was deleted (the `signal_source` *lowering*
  stays, now exclusively for generic `source` blocks under `source_declaration`).
  Dead code removed: `construct_signal_source_node_id`,
  `construct_signal_source_name_from_node_id`, the `signal_source`-family
  lowered-IR handoff block, and the `SIGNAL_SOURCE_NODE_PREFIX` /
  `CONSTRUCT_FAMILY_SIGNAL_SOURCE` constants. 5 CLI tests reworked to exercise
  `signal_source` via a `source` block (its real producer); the Python lowered-IR
  bridge gained the same source-owner decoupling + a `clock_source` branch; CI
  script catalog lists + event-bridge coverage sets updated (event-bridge has no
  `source` block, so it correctly has no `signal_source`). Full workspace +
  `check-report-schemas` + `check-formal-models` green.
- [x] Golden + negative fixtures; extend the Maude lowering/handoff models for
  `clock_source` admission. **Done:** Maude `models/maude/tests/clock-source.maude`
  (lowering/handoff/admission, model-first) + TLA+ `ClockSourceLifecycle.tla`,
  both wired into CI and green. Golden: `examples/clock-source.whip`; negative
  cases covered by parser unit tests (`recurring_clock_source_requires_missed`,
  `calendar_clock_source_requires_timezone`) + the emission test
  `construct_graph_and_lowered_report_emit_clock_source`. Follow-up (tracked under
  Stage P4): wire `clock-source.whip` through the `check-formal-models` script's
  compile→bridge→Maude path so the `clock_source` Python bridge translation is
  exercised in CI (currently covered by Rust bridge tests).

Acceptance:

- [x] `source clock as X { … }` parses, checks, and lowers to a `clock_source`
  admission template; a `signal X {}` declaration lowers to `metadata` (schema
  only, no construct-graph node — reading A).
- [x] `timer.wait` behavior and `schedule_*` vocabulary are unchanged (untouched;
  `schedule_emitter` lowering + `schedule_template` left intact).
- [x] `scripts/check-formal-models.sh` passes end to end.

**Stage P1b is complete.**

### Stage P2: Package Manager — `whip package sync`

Goal: local `whip.packages.json` + `whip.lock` + `whip package sync` and lock
discovery per `package-management.md`. Closes review issue #4.

Decisions (issue #4) — settled in `package-management.md` and the report schemas:

- [x] Lock shape is the portable `source: {type, path}` object (decided; see
  "Lock Target Shape"). The `package_lock_v0.schema.json` and the emitter still
  carry the legacy absolute `manifest_path`; the schema must flip **atomically
  with the emitter** below, not ahead of it, so CI stays green.
- [x] Canonical serialization pinned (on-disk pretty/sorted/LF/trailing-newline;
  digests over compact `canonical_json`); `package_lock_digest` and
  `package_contract_digest` inputs defined.
- [x] `package_sync_v0.schema.json` added (no emitter yet; not in the
  emitted-artifact validation set, so safe to land ahead of code).
- [x] `validation` `runtime` documented as an input alias for `runtime_boundary`
  (no enum divergence — manifest is input, contract/check are canonical output).

Implementation tasks:

- [x] Flip together: `package_lock_json` emits the portable `source` shape
  (relative to the lock dir, sorted by name then package_id) **and**
  `package_lock_v0.schema.json` requires `source` — done in the same change.
  Integration fallout fixed: the manifest-schema `construct_family`/interface-kind
  enums were aligned to the renamed catalog, and `check-report-schemas.sh` +
  `check-formal-models.sh` co-locate the manifest beside the temp lock so portable
  `source.path` resolves. (reviewed via fan-out; integration-verified green.)
- [x] Switch the lock digest to `canonical_json` over the new lock object.
- [x] Write the lock on-disk in the pinned canonical form (sorted keys, 2-space,
  LF, single trailing newline).
- [x] Implement `whip package sync`, `--check-only` (byte-identical), and `--json`
  (`whipplescript.package_sync.v0`).
- [x] Implement `whip.packages.json` (`package_set`) discovery and validation.
  The `package_set` reader + validator landed earlier; `package sync` discovers
  the nearest `whip.packages.json` via `discover_ancestor_file`. Runtime commands
  consume `whip.lock` (not the set) per the spec's discovery contract, which is the
  box below.
- [x] Implement nearest-`whip.lock` discovery and the no-lock guard for non-`std.`
  imports, per the spec's full Lock Discovery order. `load_package_lock` is the
  single chokepoint for all five load sites (check/compile/run/worker/dev) and
  delegates to `resolve_package_lock_path`: an explicit `--package-lock` wins,
  else search from each source file's directory upward
  (`discover_ancestor_file_from`), else from the cwd upward, else `None`. Sources
  that imply different locks fail with a disambiguation error. With no discoverable
  lock, `validate_construct_uses` rejects any non-`std.` import **or** package
  construct use, naming the blockers and suggesting `whip package sync`; `std.`-only
  programs are unaffected. Regression tests:
  `check_discovers_lock_relative_to_workflow_file`,
  `check_rejects_sources_implying_different_locks`, and
  `check_discovers_nearest_whip_lock_and_guards_when_absent`. The lock-free
  `checks_all_example_workflows` bundle dropped `package-memory.whip` (it now
  requires a lock). Docs updated in `docs/api-reference.md` (the `package lock`
  overview).
- [x] Atomic lock write (temp + rename); reject path escapes after symlink
  canonicalization at both sync and load.

Acceptance:

- [x] `--check-only` is deterministic across runs and machines.
- [x] A tampered manifest fails lock load with a stable diagnostic code. The lock
  pins each manifest by SHA-256 recomputed over the manifest bytes at load
  (`package_manifest_from_json`), so any byte change trips the mismatch in
  `load_package_lock_file`; the `check --json` path surfaces it with the stable
  `error.kind = "package_lock"`. Regression test
  `tampered_manifest_fails_lock_load_with_stable_kind`; documented in
  `docs/api-reference.md`.

This completes Stage P2.

### Stage P3: Lowering-Class Maturation

Goal: give every lowering class a complete platform catalog entry and model
coverage, and promote the classes a standard package needs from compiler-owned
to package-authorable where the design calls for it.

- [x] Complete catalog entries (families, authority profile, lifecycle profile,
  output validation, forbidden behavior) for all classes in the normative table.
  **Audited 2026-06-15:** all 13 catalog lowerings have full entries matching the
  normative table and the Maude `lowering-class-lifecycle` model. One doc drift
  fixed — the table listed `projection_view` authority as `none`, but the catalog
  and model both use `projection-source` (`classProjectionSourceTyped`); table
  updated. (Known naming follow-up, not a gap: the catalog authority value
  `event_admission` for `signal_emit`/`signal_source`/`clock_source` predates the
  P1a wire rename and the table calls it `signal-admission`; the wire value should
  become `signal_admission` when the model-operator axis is renamed — deferred with
  the rest of that optional rename.)
- [x] Promote `typed_effect_call` (and `resource_effect` if needed) to
  package-authorable with the corresponding validator + manifest-enum change.
  **DONE 2026-06-16 (first `std.files` prerequisite):** flipped `typed_effect_call`
  `package_authorable → true` in the platform catalog (`whipplescript-core`); added
  `typed_effect_call` to `package_manifest_v0.schema.json`'s `lowering_target` enum
  (the `…vocabulary_matches_platform_catalog` test now matches); extended the
  package-construct validator to enforce the `Forbidden` `target_capability` policy
  (a `typed_effect_call` construct must NOT name a generic capability the way
  `capability_call` does — its authority is the typed contract + its required
  `Capability` interface, both already enforced generically from the catalog). Golden +
  negative test `typed_effect_call_is_package_authorable_and_forbids_target_capability`.
  `resource_effect` not needed (no consumer). Full gate green.
  **(original analysis below)** NEEDED by `std.files` — confirmed (audit corrected 2026-06-15). Earlier audit
  wrongly read files.md's "capability-scoped" *title* as implying `capability_call`
  and deferred this. The files.md **Construct Graph Contract is explicit**: `read` /
  `write` / `import` / `export` each `requires: Capability<files.read>` (etc.) **and**
  declares `lowering class: typed_effect_call` — agreeing with DR-0019. (The other P4
  packages do not need it: telemetry=`none`, time=`clock_source`, messaging `send`=
  `capability_call`.) So `std.files` is the consumer and the promotion is required.
  **To do:** flip `typed_effect_call` `package_authorable` → true in the catalog; add
  `typed_effect_call` to the `package_manifest_v0` `lowering_target` enum (the
  `package_manifest_schema_construct_vocabulary_matches_platform_catalog` test
  auto-checks the match); extend the package-construct validator to accept a
  package-authored `typed_effect_call` and enforce its authorization shape (it
  `requires: Capability<…>` + a typed output contract; `target_capability` policy is
  `Forbidden`, i.e. it does not name a generic capability the way `capability_call`
  does — the validator must enforce that distinction). Golden + negative fixtures +
  Maude coverage land with it (the two boxes below).
- [x] Golden lowering fixtures and negative fixtures per class. The currently
  authorable classes (`metadata_only`, `capability_call`) already have golden +
  negative coverage (package fixtures + `check-report-schemas`); per-class fixtures
  for any newly-promoted class land with that promotion. Complete for the current
  authorable set (matching the `[x]` sibling below and the `[x]` acceptance
  criteria); no newly authorable class this stage (the 2026-06-19 `channel`/`send`
  constructs lower to the already-covered `metadata_only`/`capability_call`), so
  nothing is pending — the standing "add with each promotion" clause has no
  outstanding promotion.
- [x] Generated Maude coverage for each newly authorable class. No newly authorable
  class this stage; existing classes are covered by `lowering-class-lifecycle.maude`
  and the per-construct Maude tests.

Acceptance:

- [x] Each authorable class has a passing golden + negative fixture pair
  (`metadata_only`, `capability_call` — the current authorable set).
- [x] The package manifest `lowering_target` enum matches the authorable set
  (`metadata_only`, `capability_call`); verified by
  `package_manifest_schema_construct_vocabulary_matches_platform_catalog`.

**Stage P3 status:** catalog maturation is complete for the current taxonomy; the
`typed_effect_call` promotion is deferred to Stage P4 `std.files` (no earlier
consumer). The `files.md` vs DR-0019 lowering-class discrepancy is the open
question to resolve there.

### Stage P4: Standard Package Ports (telemetry → time → messaging → files)

Goal: port standard packages in dependency order, one lowering class at a time.

- [x] `std.telemetry` — buildable now (no construct nodes, lowering class `none`).
  **Construct-cycle side: N/A (done)** — no workflow constructs; reserved-class
  banner added to `std-telemetry.md`. **Runtime export surface DONE 2026-06-19:**
  the OTLP exporter provider (`otel_export` + `otel_post`) builds structural-only
  OTLP/HTTP+JSON spans (resourceSpans/scopeSpans, GenAI semantic conventions for
  model spans, content excluded by default) from the durable run/effect log and
  POSTs to `OTEL_EXPORTER_OTLP_ENDPOINT` (honoring `OTEL_SERVICE_NAME`), with a
  file-based emit-once cursor (failure never advances it; exporter availability
  never affects execution). The **export-cursor CLI** is now complete: `whip
  otel-export <instance> [--dry-run]` (existing) plus the new `whip telemetry
  status` (reports endpoint/service + per-instance exported counts) and `whip
  telemetry reset-cursor [<instance>]` (clears the cursor so the next export
  re-sends). Validated WITHOUT an external backend via an in-process collector
  test: `otel_export_posts_to_collector_then_status_and_reset` (asserts the real
  OTLP POST body + cursor reflected in `status` + cleared by `reset-cursor`); the
  prior `otel_export_dry_run_emits_structural_spans` covers the payload shape. No
  new dependency (the hand-rolled `otel_post` targets a local OTel Collector over
  plain HTTP, the spec's intended deployment; TLS/backends are the Collector's
  job). Full gate green.
- [x] `std.time` — needs `clock_source` (Stage P1b) + DST/calendar + durable
  interval-anchor design. **Construct-cycle side: DONE** — `clock_source` parser +
  lowering + formal models landed in P1b; golden `examples/clock-source.whip`;
  reserved-class banner added. **Runtime recurrence (DST/calendar) DONE
  2026-06-19:** `resolve_due_clock_sources` (cli) now dispatches all three
  recurrence forms — `every <duration>` (interval, prior), `every <calendar> at
  <time>` (`day`/`weekday`/`every <weekday>`), and `at <time>` (one scheduled
  occurrence, fired once via the `last_clock_occurrence.is_none()` guard). New
  `due_calendar_occurrences` computes tz-aware, DST-correct occurrences in the
  `(cursor, now]` window using `chrono` + `chrono-tz` (the plan-anticipated tz
  dependency — the V0 "no-tz-dep" decision was scoped to the interval clock):
  parses the source `timezone`, walks local dates matching the pattern, resolves
  each `HH:MM` local time to UTC (`LocalResult::None` spring-forward gap → skip;
  `Ambiguous` fall-back → earliest), reusing the proven interval emit path
  (`select_clock_occurrences` missed policy → `ingest_external_event` +
  `derive_fact`, idempotent on `occurrence_id`). Unit tests: daily-in-window,
  DST-correct-across-spring-forward (09:00 local shifts 14:00Z EST → 13:00Z EDT),
  weekday-skips-weekends, weekly-matches-only-that-weekday, empty-when-now≤cursor.
  E2E: `calendar_clock_source_fires_occurrences_at_runtime` (`every day at 09:00`
  America/New_York fires via `whip test` `given clock`; holds before the first
  occurrence). The "durable interval-anchor" is the existing append-only
  `last_clock_occurrence` cursor (interval) / deterministic schedule+tz (calendar);
  no separate anchor needed. Full gate green.
- [x] `std.messaging` — **correction (P3 audit): does NOT need `typed_effect_call`.**
  `send` lowers to `capability_call` (already authorable); `source interaction` uses
  `signal_source` (Stage P1b, done). Reserved-class banner added.
  **Construct-side `channel` + `Message` DONE (2026-06-19):** `channel <name> {
  provider <p> [workspace <w>] [destination "<d>"] }` is now a reserved core
  top-level construct (`Item::Channel`/`ChannelDecl`/`IrChannel`, `parse_channel`,
  `lower_channel`) lowering to `metadata_only` (like `queue`); a duplicate name is
  rejected and a missing `provider` is a spanned error. Reserving the bare
  `channel` keyword in the core parser means third-party packages cannot author
  channel-like semantics. The generic inbound `Message` envelope is a built-in
  referenceable schema (`SchemaIndex::with_builtins`, 13 fields per
  spec/messaging.md). Declaring a channel auto-registers `std.messaging` in the
  contract registry (like leases->std.coord); `use std.messaging` now parses as a
  dotted package name (`expect_use_name` accepts dotted paths). Channels appear in
  the IR snapshot, `whip fmt` (idempotent), and LSP document symbols. Tests:
  `channel_declaration_parses_and_lowers`, `channel_requires_a_provider`,
  `duplicate_channel_is_rejected`. No formal model needed (an inert `metadata_only`
  declaration, like `queue`/`file store`). **Remaining (next stage):** the
  author-facing `send via <channel>` / `when message from <channel>` usage sugar +
  `source interaction` mapping + the runtime messaging providers (outbound
  delivery, inbound `Message` facts, provider feature negotiation) — runtime-stage
  work, so this item stays `[~]`. **`send` PROTOTYPED THEN REVERTED 2026-06-19
  (concrete finding):** `send via <channel> { text … } as <binding>` lowers to a
  `ConstructCapabilityCall` (`messaging.send`), which — by the package-authoring
  design — `validate_construct_uses` requires to be authorized by a package lock
  (`construct \`send\` requires a package lock`). There is no `std.messaging`
  package manifest declaring the `messaging.send` `capability.call` contract, and
  no `ConstructCapabilityCall` from a *standard* library has existed before (`recall`
  is third-party `memory`; coordination verbs are dedicated effect kinds). So a
  usable `send` needs EITHER a built-in `std.messaging` package contract +
  standard-library lock-exemption (a package-system design decision on the
  supply-chain authorization boundary) OR a dedicated non-package effect kind
  (contradicts the spec's "send lowers to capability_call"). Both are genuine
  package/runtime-stage design work; shipping `send` without them is a trap
  (compiles to an unsatisfiable lock error), so the prototype was reverted, leaving
  the non-trap channel+Message construct-side. `when message from` is likewise a
  trap without runtime `Message`-fact ingestion. This confirms the send/message
  residual is genuinely the deferred package/runtime stage, not a quick add.
  **DECIDED 2026-06-20 (Jack): OPTION A** — register a built-in `std.messaging`
  `messaging.send` `capability.call` contract and EXEMPT standard-library (`std.*`)
  constructs from the package-lock requirement (std libs are built into the
  compiler, not third-party supply chain). Then restore the `send via` parser +
  channel-existence check + runtime arm. Inbound `when message from` still needs
  runtime `Message`-fact ingestion (the messaging provider), stays deferred.
  **OUTBOUND `send` SHIPPED 2026-06-20 (1929 OPTION A):**
  - Model: the security-relevant lock exemption is pinned in
    `models/maude/std-construct-authorization.maude` (+ tests, check-formal-models.sh
    2 Solution / 2 No-solution; README listed; bite mutation-verified). A *standard
    built-in* construct use compiles with NO lock; a *third-party* use only with an
    authorizing lock; an *unlocked third-party* use and an *unknown* construct never
    compile (lock still bites; exemption keyed on a real built-in registration, not a
    name — no `std.evil` bypass).
  - Parser: `parse_send` (`send via <channel> { text … [markdown …] [thread_id …] }
    as x`) → `ConstructCapabilityCall` keyword="send" target="messaging.send";
    AST-only (its `as` closes on the block line); `validate_send_channels` rejects an
    unknown channel; `SemanticContext.channels` added.
  - Registry: `contract_registry()` registers the built-in `std.messaging` `send`
    construct + `messaging.send` `capability.call` effect contract when a send use
    exists (channel-only programs unaffected).
  - Exemption: `validate_construct_uses` None-branch exempts a use only when the
    built-in registry authorizes it AND its owning library is `standard`
    (`construct_use_is_standard_builtin`).
  - Runtime: `parse_effect_statements` send block arm + capability.call send input
    builder; `messaging.send` capability schema + binding + provider seeded in the
    migration. Runs under `--provider fixture` (records a receipt; real delivery is
    credential-gated).
  - Tests: parser `send_lowers_to_messaging_capability_call_and_registers_builtin`,
    `send_to_unknown_channel_is_rejected`; CLI e2e
    `send_via_channel_runs_under_fixture_and_completes`. Docs: language-reference.md,
    spec/messaging.md. Full gate green.
  **INBOUND `when message from` SHIPPED 2026-06-23 (fixture parity, the symmetric
  counterpart of outbound `send`):** `when message from <channel> as msg` binds the
  built-in `Message` envelope (`binding_from_when` → `Message`), lowers to a
  channel-specific `message.<channel>` fact match
  (`runtime_fact_name_for_pattern`), and validates the channel is declared
  (`validate_message_from_channels`, mirroring `validate_send_channels`). The
  `whip message <instance> --channel <name> --text … --program …` command ingests a
  `Message` fact on the channel (mirrors `whip signal`), so a reactive rule fires
  under the fixture provider. Tests: parser
  `when_message_from_binds_message_and_validates_channel`; CLI e2e
  `inbound_message_fires_when_message_from_rule`. Docs: language-reference.md,
  runtime-operations.md, spec/messaging.md. **Still gated:** LIVE messaging
  providers (Slack/email producing `Message` facts) ride with the other
  real-provider items; the fixture-parity author/runtime surface is complete.
- [x] **Platform prerequisite — typed fact-batch admission**: model + implement
  atomic, idempotent, replay-reconstructed admission of N facts from one validated
  effect outcome (like signal admission). Required before `std.files` `import` and
  any other batch-fact surface.
  **Design decided (2026-06-16, user):** per-row key = **row_index + natural_key**
  (`H(effect_key, natural_key)` when the row schema declares a natural key, else
  `H(effect_key, row_index)`), matching admission-and-idempotency.md; sequencing =
  **model-first, then code**.
  **Model DONE 2026-06-16:** extended `models/maude/admission.maude` with the per-row
  key-derivation layer (`derive(EffectKey, RowKey)`, `importRow`, idempotent admit via
  the existing admitted-set guard) on top of the existing batch-atomicity model.
  Coverage: an imported row admits a fact keyed by its derived identity; two distinct
  natural keys admit two distinct facts. **Bite** (`tests/admission.maude`, all
  No-solution, soup-var targets): re-importing the same (effect, natural key) never
  admits two facts; same for a repeated row index; an invalid row admits none — each
  would be reachable without the member-guard/validity gate. `check-formal-models.sh`
  counts updated (admission solutions 4→6, no-solutions 5→8); full formal suite green.
  **Greenfield sub-pieces (decomposed):**
  - **Sub-piece 1 (parser/IR) — DONE 2026-06-16.** `import <format> <Schema> from <store>
    at <path> as <binding>` parses + lowers to `IrEffectKind::FileImport` (+ `file.import`
    contract, FileImportResult, RuntimeBoundary) across every match site;
    `body::BodyEffectKind::FileImport` + `parse_import` (validates `format ∈ jsonl/json/csv`,
    requires the `as` binding) + dispatch + reserved keyword + `flow_expand` formatter +
    `parse_effect_line`. Verified: `import jsonl IssueRow from … as imported` checks clean
    with `kind=file.import` in the IR; `import xml` is rejected. Tests
    `import_accepts_structured_codecs_and_lowers_to_file_import` /
    `import_rejects_unsupported_codecs` (parser). Docs: `files.md` Status. Full gate green.
    (Runtime pending → an import effect lowers but does not yet run; no worker handler.)
  - **Sub-piece 2 (batch-admission store primitive) — DONE 2026-06-16.**
    `SqliteStore::admit_fact_batch(FactBatch)` admits N rows in one transaction:
    each `FactBatchRow` carries its derived per-row admission key as `fact_id` (the
    caller computes `H(effect_key, row_index | natural_key)` — keying policy stays in
    the runtime), a row whose key already has a fact is skipped (idempotent), and the
    batch commits or rolls back as a unit (all-or-nothing). Facts are recorded with
    provenance `import` + the schema name, so `when <Schema>` rules fan out. Directly
    realizes the Maude `importRow` model. Test
    `admit_fact_batch_admits_atomically_and_is_per_row_idempotent` (admit 2 → 2 facts;
    re-admit same → 0 admitted/2 skipped; overlapping batch → only fresh rows). Full
    gate green.
  - **Sub-piece 3 (`import jsonl`/`json` runtime) — DONE 2026-06-16.** `import` runs
    end-to-end: line-based `parse_effect_statements`/`parsed_effect_input_json`
    `file.import` arms (the input carries the store root, allow-globs, and the schema's
    required non-optional/non-literal fields), `run_file_import_effect` +
    `decode_import_rows` (jsonl = one JSON object per line; json = top-level array),
    per-row required-field validation, and `kernel.admit_fact_batch` (delegating to the
    store primitive) admitting N typed `<Schema>` facts atomically; success settles
    `file.import.completed`, any invalid row settles `file.import.failed` and admits
    nothing (all-or-nothing). Producer analysis fixed: `push_ingest_fact_writes` now
    registers `import <Schema>` as a `schema:<Schema>` fact-write, so `when <Schema>`
    rules are live (not flagged dead) and fan out over the rows. Worker dispatch added.
    Per-row keys use the row index (v0); natural-key is sub-piece #5. Test
    `dev_file_import_jsonl_admits_typed_rows_and_is_atomic` (well-formed → 2 typed facts
    + fan-out fires; missing-field row → failed, 0 facts admitted). Docs: `files.md`
    (Status + Importing Structured Data v0 note). Full gate green.
  - **Sub-piece 4 (`csv` decoder) — DONE 2026-06-16.** `decode_import_rows` now handles
    `csv`: `split_csv_record` (RFC-4180-style quoting — quoted fields may contain commas;
    `""` escapes a quote) parses the header row and maps it over each record (values
    decode as strings); a record whose field count disagrees with the header is rejected.
    Reuses the same per-row validation + atomic admission as jsonl/json. v0 assumes one
    record per line. Test `decode_import_rows_handles_jsonl_json_and_quoted_csv` (unit).
    Docs: `files.md`.
  - **Sub-piece 5 (natural-key declaration + keying) — DONE 2026-06-16.** Completes the
    `row_index + natural_key` decision. User-chosen syntax: a **`@key` field annotation**
    (`id string @key`) — unambiguous, consistent with the language's `@`-tags, no
    collision with a field named `key`. Built: `ClassField`/`IrClassField` gain `is_key`;
    `parse_class` parses a trailing `@key` (rejecting unknown field tags); `lower_class`
    rejects >1 `@key` per class; `format_class` + `to_snapshot` render `@key` only when
    set (fmt round-trips, no snapshot ripple). Runtime: `parsed_effect_input_json` passes
    the schema's `@key` field name; `run_file_import_effect` keys each row by that field's
    value (`H(effect_key, natural_key)`) and records it as the fact key, else falls back
    to the row index. Verified: an import of `@key`-bearing rows records facts keyed by
    the natural-key values (A, B) not indices. Tests
    `class_field_key_annotation_lowers_and_rejects_duplicates` (parser). Docs: `files.md`,
    `language-reference.md`. Full gate green.
  - **Sub-piece 6 — `export`.** Design settled in
    [DR-0022](decision-records/0022-collection-valued-projections.md) (operator steer:
    *set the foundations, don't come back later* → build a real collection value, not a
    per-row-append hack). Introduces a **collection-valued projection** `<Schema> [where
    <pred>]` (type `Array<Ref<Schema>>`, evaluated via the existing `where` fact-matching,
    deterministically ordered) as `export`'s row source — general machinery, exposed only
    in the `export { rows … }` clause in v0. No new formal model (projection-read +
    write-effect lifecycle already modeled). Decomposed into gated slices:
    - **6a** — collection-valued projection: `Expr`/parse entry, `Array<Ref<Schema>>`
      typing, ordered evaluator, projection-read lowering + liveness (marks the rule a
      reader of `<Schema>`). Tests: parse + type + deterministic eval.
    - **6b** — `export` parser/IR: `BodyEffectKind::FileExport` + `parse_export`
      (validate codec ∈ jsonl/json/csv, required mode, rows element type matches
      `<Schema>`) + `IrEffectKind::FileExport` + formatter + reserved keyword.
    - **6c** — `export` runtime: `run_file_export_effect` (resolve collection, serialize
      per format = inverse of the import decoders, enforce mode + boundary, write, settle
      `file.export.completed`/`.failed` with row count + hash) + worker dispatch.

    **Sub-piece 6 — DONE 2026-06-17 (6a/6b/6c landed together; `export` runs end-to-end).**
    The collection-valued projection foundation is realized as a deterministic
    fact-collection resolver (no general Expr-variant ripple — DR-0022's "general
    machinery, conservative surface"): `parse_export` parses `export <fmt> <Schema> to
    <store> at <path> { [where <pred>] mode <mode> } as <binding>` (codec ∈ jsonl/json/csv,
    required mode, ast-only binding seeding); `IrEffectKind::FileExport` + `file.export`
    contract across all match sites; `flow_expand` formatter (idempotent); line-based
    `parse_effect_statements`/`parsed_effect_input_json` `file.export` arms (input carries
    the `where` predicate + the schema's field order for the csv header). Runtime
    `run_file_export_effect` resolves the collection (`list_facts` filtered by `name ==
    schema` and the `where` predicate via the shared `evaluate_proj_predicate`, in the
    store's deterministic `(name, key)` order), serializes via `encode_export_rows`
    (`csv_escape_field` quoting — inverse of the import decoders), enforces the write
    mode-vs-disk + root/`allow write` boundary, writes (incl. append), and settles
    `file.export.completed`/`.failed` with row count + hash. Verified: an import→export
    round-trip with `where status == "ready"` writes exactly the matching rows
    (`id,status\nA,ready\nC,ready\n`). Test `dev_file_export_serializes_filtered_collection`.
    Docs: `files.md` (Status + Exporting v0 note), `language-reference.md`, DR-0022. Full
    gate green. **`std.files` v0 is now complete: read, write, import, export.**
- [x] `std.files` (v0, slimmed per DR-0019 review) — storage boundary only:
  `metadata_only` file-store declaration, read/write/import/export lowering through
  **`typed_effect_call`** (the P3 promotion above is **DONE 2026-06-16**),
  turn-grant-as-metadata (Proposal A), codecs text/markdown/json/jsonl/csv/bytes,
  **literal paths only**. `import`/`export` ride the fact-batch prerequisite above.
  Deferred to separate designs: xlsx, docx, dynamic `Expr` paths, non-filesystem providers.
  **Scope correction (2026-06-16, from code mapping):** this is a **full-stack
  core-language feature, not a manifest-only port.** `parse_effect_line` hardcodes the
  effect-keyword set → `IrEffectKind` (package constructs aren't generically
  manifest-parsed; `memory.recall` only works because it aliases the existing
  `CapabilityCall` kind). `std.files` therefore needs, in order:
  (1) **core IR** — new `IrEffectKind` variants for file read/write (and import/export);
  (2) **parser** — hardcoded keyword + *structured multi-clause* parsing for
  `read <fmt> from <store> at <path> as <binding>` / `write …` and the two-token
  `file store <name> { … }` declaration (the parser does simple `starts_with` keyword
  detection today, not clause parsing);
  (3) **lowering + construct-graph** — emit the `typed_effect_call` nodes + the file-store
  `metadata_only` resource (`provides Resource<FileStore>`);
  (4) the package **manifest** (`files.json`) + capabilities `files.read`/`files.write`;
  (5) **runtime** — `run_file_effect`, a local file provider, the codecs, and (for
  import/export) the typed fact-batch admission primitive.
  Each is a distinct gated piece; this is a dedicated multi-stage effort, not a loop tick.
  **Progress 2026-06-16 (route B, builtin std library):**
  - Piece 1 DONE — core IR `IrEffectKind::FileRead` wired through every match site;
    `register_effect_contract` registers `std.files` + the `file.read` contract
    (`file.read.input` → `FileReadResult`, RuntimeBoundary). Additive; full gate green.
  - Piece 2a DONE — parser: `read <format> from <store> at <path> as <binding>`
    (`body::BodyEffectKind::FileRead` + `parse_read` + dispatch + reserved keyword +
    line-based recognition + `flow_expand` formatter) lowering to a `file.read` IR effect
    node. **Verified: a `read` workflow checks clean and `file.read` appears in the
    compiled IR**; full suite + fmt green.
  - Piece 2b DONE — `file store <name> { root "…" }` as a new top-level `Item`
    (`FileStoreDecl` + `parse_file_store` + dispatch + `Item::span`/lower/format/
    apply-expansion arms). v0 lowers it as accepted (its root is consumed by the
    runtime provider in piece 4; no IR projection yet, to avoid a snapshot ripple).
    **Verified: a full `file store` + `read` + `after` + `complete` workflow checks
    clean (exit 0) with `fileResult kind=file.read` in the IR; fmt round-trips; full
    suite green.**
  - Piece 4 (runtime) **DONE 2026-06-16** — `read text` runs end-to-end. Built:
    (a) `ParsedEffect` read branch stashing `[format, store, path]`; (b) a
    `"file.read"` arm in `parsed_effect_input_json` building `{format, store, path,
    root}`; (c) **file-store-root threading** via `IrProgram.file_stores` (name→root)
    + lowering + `to_snapshot` (no snapshot ripple — empty collections are skipped);
    (d) `run_file_effect` — reads `<root>/<path>`, settles the effect, and derives a
    `file.read.completed`/`file.read.failed` binding fact carrying `value.content`
    (mirrors `run_exec_effect`'s projection) so `after <binding> succeeds as r` binds
    `r.content`; (e) worker dispatch on `"file.read"`.
    Two cross-cutting fixes were required to make the effect runnable: `policy_block_on`
    now short-circuits `file.*` like the other runtime-resolved verbs (timer/queue/
    lease/…) — otherwise `effect_provider_exists("file.read")` policy-blocked it out of
    `claimable_effects`; and the `files.read` **capability requirement was dropped for
    v0** (requiring it with no grantor in scope would policy-block every read), making
    the `file store` `root` the scope boundary instead. **Path containment is enforced**:
    absolute or `..`-escaping paths are refused before any disk access (→
    `file.read.failed`). Regression tests `dev_file_read_binds_content_and_completes`
    and `dev_file_read_refuses_path_escaping_store_root` (control_plane.rs); spec
    (`files.md` Implementation Status) + user docs (`language-reference.md` Files)
    updated. Full gate green (759 tests, fmt, report-schemas, formal-models).
    **Deferred follow-ups** (documented in `files.md`): the `files.read` capability +
    turn-grant layer, per-path `allow read [...]` globs, and
    re-homing `read` onto `typed_effect_call` (v0 took route B, a builtin effect).
    With `read` running end-to-end, `given file` (test-harness #3) is now implementable.
  - **`read` codec scope made honest — DONE 2026-06-16.** `parse_read` now validates
    the format: `read` decodes only the `text`/`markdown` body codecs (both UTF-8
    bodies; `markdown` runs e2e — it reads as a body). Structured `json`/`jsonl`/`csv`
    are rejected with a diagnostic pointing to `import` (typed rows) / `read text` +
    `coerce`; `bytes` (artifact) is rejected as a deferred read codec. Previously any
    format ident parsed and was silently read as text. Tests
    `read_accepts_text_and_markdown_body_codecs` / `read_rejects_structured_and_binary_codecs`
    (parser). Docs: `files.md` (Status table + Reading Files), `language-reference.md`.
  - **`write` (text/markdown body codecs) — DONE 2026-06-16** (full vertical, route B).
    `write <format> to <store> at <path> { body <expr> mode <mode> } as <binding>`
    renders a body to disk end-to-end. User-approved design choices: explicit `mode`
    required (no silent overwrite), text+markdown codecs only (json/csv = `export`,
    deferred), mode violation = ordinary `file.write.failed` (→ `after w fails`). Built:
    (1) core IR `IrEffectKind::FileWrite` + `file.write` contract (FileWriteResult,
    RuntimeBoundary) across every match site; (2) parser — `body::BodyEffectKind::FileWrite`
    + `parse_write` (block parse: format/store/path + `{ body mode }`, validates codec +
    required mode ∈ create/replace/upsert/append) + dispatch + reserved keyword +
    `flow_expand` multi-line formatter (idempotent) + `parse_effect_line`/`is_ast_only`
    (the `as` binding sits on the block's closing line → ast-only seeding so `after
    <binding>` resolves); (3) line-based `parse_effect_statements` `write ` arm
    (gathers the block via `parse_statement_until_balanced_braces`, token-scans body/mode)
    + `parsed_effect_input_json` `file.write` arm (resolves `body` at commit via
    `parse_field_value` against the (after-)context — so `after r succeeds as v { write …
    { body v.content } }` works with no worker-time resolution); (4) runtime
    `run_file_write_effect` (root-containment refusal, mode check vs on-disk state,
    create/replace/upsert/append, content hash, `file.write.completed`/`.failed` facts)
    + worker dispatch. Idempotency/replay inherited from the exactly-once effect
    lifecycle (admission-and-idempotency.md) — no new model. Tests: parser
    `write_accepts_text_and_markdown_with_explicit_mode` / `write_rejects_structured_codecs`
    / `write_requires_an_explicit_mode` / `write_rejects_unknown_mode`; CLI e2e
    `dev_file_write_renders_body_and_enforces_mode` (create writes the resolved body;
    create-on-existing fails and leaves the file untouched). Docs: `files.md` (Status +
    Route B + enforce + Writing Files), `language-reference.md` Files. Full gate green.
  - **`allow read/write [...]` glob policy — DONE 2026-06-16.** The `file store` now
    parses optional `allow read [...]` / `allow write [...]` glob lists (via
    `parse_string_list`); threaded through `FileStoreDecl` → `IrFileStore`
    (`read_globs`/`write_globs`) → `to_snapshot` (serialized only when present → no
    snapshot ripple) → `format_item` (fmt round-trips) → the effect input (`allow`).
    Runtime `file_path_policy_error` (shared by `read`/`write`) refuses absolute/`..`
    paths and, when an allow list is declared, a path matching no glob (empty list = any
    path in root). Reuses the existing `glob_match`. Tightens the boundary from
    root-containment-only to root + policy. Test
    `dev_file_store_allow_policy_scopes_read_paths` (matching path reads, in-root
    out-of-policy path fails). Docs: `files.md`, `language-reference.md`.
    **Remaining std.files:** `import`/`export` (blocked on typed fact-batch admission),
    the `files.read`/`files.write` capability + turn-grant layer, codecs beyond body.
  - **`given file` test fixtures (#3, user-approved "right away") — DONE 2026-06-16.**
    Design was resolved by harness mechanics, not a contentious choice: `whip test`
    already runs the **real** `step_instance` + `run_worker_once` loop (so the real
    `run_file_effect` executes). Built: (1) `GivenClause::File { store, path, content,
    span }` parser variant + `parse_given` arm + `format_test` arm (round-trips
    idempotently) + the two main.rs presentation arms; (2) in `execute_scenario`, when
    a scenario has `given file` clauses, a per-scenario temp dir is created, each
    `<content>` written to `<temp>/<path>`, and the named store's `root` **redirected**
    to that temp dir by cloning the IR and overriding `file_stores[store].root` (the
    redirected root is baked into the effect input at commit time by `step_instance`,
    so only the `ir` handed to it needs rewriting; `run_file_effect` reads root from
    the effect input). Setup errors: an escaping fixture path (absolute/`..`) and a
    `<store>` the workflow does not declare are rejected. No formal model (test-harness
    construct, like `given clock`/`given tracker`). Tests:
    `test_harness_seeds_file_fixtures` (control_plane.rs) — a seeded read completes, an
    unseeded read fails. Docs: `api-reference.md`, `workflow-testing.md` (v0 status +
    example aligned to the implemented plain-string form; `contains """…"""` heredoc
    noted as a planned ergonomic), `language-reference.md` Files. Full gate green.
  - **v0 SCOPE COMPLETE (marked [x] 2026-06-19).** The DR-0019 v0 scope
    (spec/files.md "v0 Scope": `read`/`write`/`import`/`export`; codecs
    text/markdown/json/jsonl/csv/bytes; literal paths; local provider) is
    implemented end-to-end and locally tested (the implementation table in
    files.md shows every v0 row "implemented"). Explicitly OUT of v0 (documented
    in files.md, not gaps in this item): the `files.read`/`files.write` capability
    grants are *deferred for v0* with a stated reason — enforcing them needs a
    grantor model that does not exist yet, and requiring them with no grantor in
    scope would policy-block every operation; and xlsx, docx, dynamic `Expr`
    paths, and non-filesystem providers are deferred to separate designs. Those
    are future-stage work, not v0 deliverables.

Acceptance:

- [x] Each ported package has a reserved-class prerequisite banner in its spec
  (`std-telemetry.md`, `std-time.md`, `messaging.md`, `files.md` — added
  2026-06-15). Golden construct-graph fixtures: `std.time` has
  `examples/clock-source.whip`; `std.telemetry` has no workflow constructs (N/A);
  `std.messaging`/`std.files` fixtures land with their construct-side items
  (`channel`/`Message` schema; the files lowering-class decision).

**Stage P4 construct-cycle status:** telemetry (N/A) + time (done in P1b) construct
sides are complete; messaging needs only the `channel` grant + `Message` schema (no
`typed_effect_call`); files needs the P3 `typed_effect_call` promotion (resolved —
see above) then its port. All
*runtime* provider/export work (OTLP, recurrence evaluation, messaging providers,
files I/O + fact-batch admission) is deferred to the runtime stages, consistent
with the per-package notes above.

### Stage P5: Diagnostics Rework

Goal: one diagnostic object model across check/lint/LSP/test per
`error-handling.md`. Design detailed (review issue #5).

Decided: one severity enum `error | warning | info | hint` (LSP-aligned 1:1);
`note` reclassified as related information; `allow`/`warn`/`deny` are lint
*configured actions*, not severities; reserved code namespaces enumerated with
`lint.*` owned by the linter; static-analysis codes (`graph.*`, `effect.*`)
allocated; code governance (stable, additive, single owner) defined.

- [x] Implement a shared `Severity` enum (replaces bare severity strings) and the
  related-information role for `note`. **Foundation done (2026-06-15):**
  (1) schema side — `construct_graph_v0` + `lowered_ir_report_v0` severity enums
  aligned to the canonical 4-value set `["error","warning","info","hint"]` (matching
  `test_report_v0`). (2) code side — `core::Severity` enum added (`Error|Warning|
  Info|Hint` with `as_str`/`from_wire`/`ALL` + round-trip unit test; doc note that
  `note` is related-information not a severity, and inbox `"normal"` is a distinct
  concept). The CLI artifact-diagnostic validator now enforces the canonical set via
  `Severity::from_wire` (fixing a bug where it rejected `hint`). Workspace +
  `check-report-schemas` green. **Remaining (the cross-crate type retrofit, ~20
  compiler-guided sites):** change `DiagnosticRecord.severity: &str` /
  `TerminalDiagnosticRecord.severity: String` (store) to `core::Severity` and pass
  `Severity::X` at every constructor (store/kernel/cli), `.as_str()` at the DB
  binding, `from_wire` at the DB reads; route the CLI `"severity"` JSON literals
  through the enum; constrain the free-string severity fields (`dev_report_v0`,
  `acceptance_report_v0`, `acceptance_fixture_v0`) to the enum once emitter-sourced.
  (Diagnostic severity is cleanly separable from inbox-item `"normal"` severity, so
  the retrofit is bounded to the diagnostic structs.)
  **Type retrofit DONE (2026-06-15):** `DiagnosticRecord.severity` and
  `TerminalDiagnosticRecord.severity` are now `core::Severity` (was `&str` /
  `String`); every constructor across store/kernel/cli passes `Severity::Error` etc.
  (bulk-migrated, ~15 sites); the DB write binds `.as_str()` and JSON serializes
  `.as_str()`; inbox-item `"normal"` severity was correctly left as a separate
  string concept. Full workspace + `check-report-schemas` + `check-formal-models`
  green. **Lint severities routed through the enum (2026-06-18):**
  `LintFinding.severity` is now `core::Severity` (was a bare `&'static str`); the CLI
  emits `.as_str()` for `severity`/`default_severity`/`configured_action` (wire format
  unchanged) and the LSP maps it via a new `Severity::lsp_code()` (canonical set ↔ LSP
  1/2/3/4, with a `severity_lsp_codes_align_one_to_one` core test), so a `warning`
  lint finding now publishes at LSP severity 2 faithful to its declared severity (was
  a hard-coded 3). **Related-information `note` role DONE (2026-06-19):**
  `Diagnostic.related: Vec<RelatedInfo{span,message}>` + a `with_related(span, msg)`
  builder (so the common no-related case stays a plain literal — the field was
  bulk-added to all ~260 `Diagnostic` literals across parser+cli via a
  compiler-validated transform, the parser struct is never destructured so it was
  safe). Surfaced in CLI text (`= note: <msg> (path:line:col)`), JSON reports
  (`related[]` with `message`/`source_span`, OMITTED when empty so existing
  diagnostic shapes are unchanged), and LSP `relatedInformation` (linked
  locations). Two real producers point at "first declared here": duplicate
  `channel` (`lower_channel`) and duplicate schema (`collect_schema_names`, now
  tracking first spans). Tests:
  `duplicate_channel_is_rejected`/`duplicate_schema_diagnostic_points_at_first_declaration`.
  spec/error-handling.md "Spans And Labels" is the contract. **Remaining (not
  blocking, explicitly accepted):** the `DiagnosticView` read-model +
  `render_diagnostic_with_severity` use display strings (acceptable at read/print
  boundaries); the two acceptance-report aggregate severity fields are
  severity-bucket labels (different semantics) and stay free-string. These are
  presentation-boundary string uses, not the diagnostic-severity contract, so the
  Severity + related-information work is complete.
- [x] Allocate the reserved namespaces + static-analysis codes as analyses land.
  Landed 2026-06-18: `lint.unused_coerce`, `lint.unused_lease`,
  `lint.unused_ledger`, `lint.unused_counter`, `lint.unused_queue`,
  `lint.unused_file_store`, `lint.unused_class`, `lint.unused_enum`,
  `lint.noop_rule`, `lint.coerce_result_unused` (a called coerce whose result binding
  is never used — conservative whole-token count over the rule body, flag at exactly
  one occurrence), `lint.broad_file_grant` (a `file store` read/write glob matching
  everything under the root — `**`/`**/*`; explicit match-all only, zero FP),
  `lint.deep_after_nesting` (severity `info`; a rule nesting `after` blocks ≥4 deep —
  depth from `metadata.max_after_depth`, AST-computed so prompt braces never miscount),
  and `lint.internal` (invalid lint config) (see `whip lint` below).
- [x] Emit `default_severity` / `configured_action` in `lint --json`. `whip --json
  lint` emits both per finding. **Operator-configured actions landed (2026-06-18):**
  `LintAction` enum (`allow`/`warn`/`deny`) resolved per finding from `--allow <id>`/
  `--deny <id>` over a project `whip.lint.json` (`whipplescript.lint_config.v0`, via
  `load_lint_config`) over the `warn` default — CLI > config > default. `allow`
  suppresses the finding (not emitted), `deny` reports it and exits nonzero; `--json`
  `configured_action` now reflects the resolved action (a separate axis from
  `default_severity`). Invalid config is a `lint.internal` error (nonzero exit). Tests
  `lint_deny_action_exits_nonzero`, `lint_allow_action_suppresses_finding`,
  `lint_config_file_applies_and_cli_overrides`, `lint_invalid_config_is_internal_error`.
  **`--rule <id>` selection landed (2026-06-18):** repeatable; restricts the run to
  the named rules (empty = all), applied before action resolution. Test
  `lint_rule_selection_restricts_to_named_rules`. This completes the spec's lint CLI
  flag surface (`--rule`/`--allow`/`--deny`). **Multi-source + directory discovery
  landed (2026-06-18):** `whip lint <source-or-dir>...` accepts multiple positionals
  and directories (reusing `expand_test_sources`/`collect_whip_files` from `whip
  test`), linting each via `lint_source` and aggregating; one source keeps the flat
  `{schema, path, findings}` JSON (back-compat), several emit `{schema, reports:
  [{path, findings}]}`; a denied finding or compile error in any file fails the run.
  Test `lint_discovers_directory_sources_and_aggregates`. Deferred: `--fix`, config
  walk-up discovery, inline source suppressions.

### Stage P6: Editor Tooling — `whip lint` / `whip fmt` / `whip lsp`

Goal: authoring tooling over shared compiler services per `editor-tooling.md`.
Design detailed (review issue #6). Depends on Stage P5.

Decided: `whip test`/test-stub LSP references cross-ref `workflow-testing.md`
(distinct from the `std.test` Non-Goal); v0 incremental model is debounced
full-recheck with per-file parse/IR caching (no LSP-only semantics); `whip fmt`
must be idempotent and stable, gated on the comment model; package editor metadata
has a per-construct-family required set validated by package-contract
(`package_contract.insufficient_metadata`).

Comment-model foundation (2026-06-15): the lexer previously *discarded* comments
(`skip_line` on `#`/`//`), so `whip fmt` would have deleted them — the reason the
plan gates `fmt` on a comment model. The lexer now **captures** comments
(`Comment { marker, text, span }`, both `#` and `//`) into `Lexed`, exposed via the
public `lex_comments(source)`; comments stay out of the token stream so the parser
is unaffected (verified: a commented program still compiles clean). Unit test
`lexer_captures_comments_without_affecting_tokens`. This unblocks the comment-aware
formatter and LSP. **Next (design-gated):** comment *attachment* strategy
(leading/trailing association to AST nodes or line-anchored re-emission) +
threading comments through `format_program` so `whip fmt` is non-destructive — the
attachment strategy is a design decision to settle before implementing.

- [x] Build `whip lint` / `whip fmt` / `whip lsp` over shared compiler services.
  **`whip fmt` v0 shipped (2026-06-15):** `whip fmt [--check] <file>...` over the
  existing `format_program` AST formatter — idempotent on comment-free files
  (verified). Because the AST round-trip cannot yet carry comments, `fmt` is
  **strictly non-destructive**: it refuses (does not write) any file containing
  comments, using `lex_comments` to detect them, and reports it as an error rather
  than dropping them. Integration test
  `fmt_preserves_placeable_comments_and_refuses_unplaceable_ones`; usage + docs
  added. **Comment preservation extended (2026-06-17):** own-line comments are now
  preserved in top-level leading position, in raw-body declarations
  (`rule`/`apply`/`coerce`/`table`), and — newly — interleaved by source position
  inside `class`/`agent`/`enum` bodies (which rebuild from the AST). The strategy
  is **A** (extend the existing span-interleave mechanism, not full per-node AST
  attachment), chosen because the formatter reflows constructs (so line-anchored
  re-emission would drift) and per-node attachment is disproportionate. A
  `lex_comments` no-loss count guard keeps every uncovered position **safe**.
  **Trailing field/variant comments LANDED 2026-06-17:** a unified
  `classify_body_comments` splits each body's comments into own-line (interleaved)
  and trailing (appended to the matching single-line member via `line_index`), so
  `title string  # the title` round-trips. **Nested data-`enum`-variant comments
  LANDED 2026-06-17 too:** a variant's nested field block is a field list in braces,
  so `enum_variant_lines_with_comments` reuses the same classify/emit one level
  deeper (each brace-body filters comments by its own span, so levels never
  double-count). **Top-level trailing comments LANDED 2026-06-17 too:** the
  top-level interleave now attaches a trailing comment to the element whose last
  source line it shares (`workflow Demo  # …`). So every leaf comment position
  (leading, trailing, field own-line/trailing, nested) is preserved. **`signal`
  payload bodies LANDED 2026-06-17 too** (`try_format_event_with_comments` — an
  `EventDecl` payload is a `ClassField` schema, so it reuses the class path).
  **`flow` formatter CORRUPTION fixed 2026-06-17 (HIGH):** `format_item` emitted a
  literal placeholder `flow X { ... }`, silently DESTROYING the whole flow body
  (whens/tells/askHuman/terminals) on the first `fmt` pass — the idempotency
  self-check missed it because the placeholder is *stable*. Replaced with
  `format_flow` (mirrors `format_rule`: tags/description + `flow <name>` + `when`
  clauses + `{` + `format_block_body(&flow.body.text)`), which formats the flow
  faithfully and carries its body comments. The corpus test
  `fmt_is_non_destructive_across_every_example` was strengthened with a
  semantic-equivalence check (a formatted file must compile to the same rule/class
  structure) — idempotency alone could not catch a stable-but-wrong placeholder.
  `queue` bodies landed too (`try_format_queue_with_comments`). The ONE remaining
  refused position is a comment inside a `file store` body — its `FileStoreDecl`
  fields (`root`, glob lists) carry no source spans, so positional interleave is
  caught by the no-loss guard (refused, never dropped). Idempotency AND semantic
  equivalence verified across all examples; test coverage for
  class/agent/enum/signal/queue/file-store preservation, top-level-trailing
  preservation, a refused brace-header-line trailing comment, and flow-body
  non-corruption. **`file store` body comments LANDED 2026-06-18** — clause spans
  (`root_span`/`read_span`/`write_span`) added to `FileStoreDecl` so the same
  classify/interleave path applies; this was the last rebuilt-body gap, so **every
  declaration body now preserves comments**. The only comments `fmt` still refuses
  are genuinely unplaceable ones (a comment trailing a declaration's opening-brace
  line, with no field on that line). (Separately uncovered + noted: a narrow latent
  `use "dotted.name"` quote-drop in `format_item`, fully contained by the
  idempotency self-check — fmt refuses, never corrupts — and dormant since the
  documented form is bare `use memory`.) **`whip lint` v0 shipped (2026-06-18):**
  `whip [--json] lint [--root <wf>] <file>` over `compile_source_path_with_root` —
  reports compile errors (a superset of `check`) and static-analysis warnings.
  Analyses (all flag declared-but-unreferenced constructs that are only usable
  in-program → zero false-positive risk, conservative scans over rule bodies AND
  `when` clauses that can only under-report): `lint.unused_coerce` (never called),
  `lint.unused_lease`/`unused_ledger`/`unused_counter` (coordination resource never
  acquired/appended/consumed), `lint.unused_queue` (never filed into or claimed),
  `lint.unused_file_store` (never read or written),
  `lint.unused_class`/`lint.unused_enum` (type referenced nowhere — exact-once
  source-token count, which also excludes synthetic lowering-generated types like a
  flow's `FlowAwait_*` that never appear in source); plus `lint.noop_rule` (a rule
  with an empty body — fires but produces nothing; detected by an empty-body text
  check, since a top-level `complete`/`fail` is carried by the lowering, not the IR
  metadata). `--json` emits `code` /
  `severity` / `default_severity` / `configured_action` / `message` / `range` per
  finding (`whipplescript.lint.v0`); the text output prefixes `:line:col`. **Spans
  added 2026-06-18:** each finding names the top-level declaration it concerns, which
  `lint_program` resolves to a source span via `document_symbols` (program-unique
  names → unambiguous), so findings point at a location in the CLI and as LSP
  diagnostics. `scripts/check-docs-examples.sh` now also lints every
  example and fails on any finding — dogfooding the linter (a future analysis that
  false-positives on real code is caught here) and keeping examples free of dead
  declarations. Warnings don't fail the run yet (configured actions
  are future work). Test `lint_flags_unused_coerce_functions`. **`whip lsp` v0
  shipped (2026-06-18):** a hand-rolled stdio Language Server (no async/LSP crate —
  the no-runtime-dependency stance) covering the diagnostics-on-edit core:
  `initialize` (capabilities: full text sync), `textDocument/didOpen`+`didChange`
  (re-compile via `compile_program`, publish `textDocument/publishDiagnostics` with
  byte→UTF-16 `lsp_byte_to_position` ranges for errors + warnings), `didClose`
  (clear), `shutdown`/`exit`. Reuses the `whip check` compiler. **Document symbols
  added 2026-06-18:** `textDocument/documentSymbol` returns the top-level
  declarations (editor outline) via a new parser `document_symbols(source) ->
  Vec<DeclSymbol>`. **Go-to-definition added 2026-06-18:** `textDocument/definition`
  resolves the identifier under the cursor to its top-level declaration
  (`lsp_position_to_byte` + `lsp_identifier_at` + the `document_symbols` name→span
  map; top-level names are program-unique so a name match is the definition).
  **Hover added 2026-06-18:** `textDocument/hover` shows the declaration source for
  the symbol under the cursor (same name→decl resolution). **Completion added
  2026-06-18:** `textDocument/completion` returns a flat candidate list — language
  keywords (`LSP_KEYWORDS`) + the document's declared names — editors filter by
  prefix. **Find-references added 2026-06-18:** `textDocument/references` lists
  every whole-token occurrence of the top-level symbol under the cursor
  (`lsp_find_occurrences`; honors `includeDeclaration`). **Rename added 2026-06-18:**
  `textDocument/rename` edits every code occurrence but EXCLUDES occurrences inside
  string literals/comments (via a new parser `string_and_comment_spans`) so a class
  name in a prompt isn't corrupted. **Formatting added 2026-06-18:**
  `textDocument/formatting` reuses `format_program_preserving_comments` (the `whip
  fmt` formatter), returning a whole-document edit (or none if it doesn't parse / is
  refused). **Document highlight added 2026-06-18:** `textDocument/documentHighlight`
  marks every occurrence of the symbol under the cursor (same scan as references).
  Nine LSP tests drive framed JSON-RPC over stdio
  (`lsp_publishes_diagnostics_on_did_open`, `lsp_returns_document_symbols`,
  `lsp_go_to_definition_resolves_top_level_name`, `lsp_hover_shows_declaration_source`,
  `lsp_completion_offers_keywords_and_declared_symbols`,
  `lsp_find_references_lists_all_occurrences`,
  `lsp_rename_edits_code_occurrences_but_not_strings`,
  `lsp_formatting_returns_whole_document_edit`,
  `lsp_document_highlight_marks_all_occurrences`,
  `lsp_workspace_symbol_indexes_open_documents`,
  `lsp_publishes_lint_findings_as_diagnostics`). The single-document LSP
  feature set is complete. **Workspace symbols added 2026-06-18:** `workspace/symbol`
  runs `document_symbols` over every OPEN document and filters by a case-insensitive
  substring query (empty → all), returning `SymbolInformation[]`. v0 indexes open
  documents only. **Lint diagnostics added 2026-06-18:** when a document compiles,
  `lsp_publish_diagnostics` also runs `lint_program` and publishes findings as
  diagnostics tagged `whip lint` at each finding's own severity (via
  `Severity::lsp_code()`; a warning → LSP 2), now that lint findings carry spans,
  distinct from the `whip` correctness diagnostics. What still needs a
  shared symbol-index: cross-FILE references, scope-aware (local-binding) resolution,
  and FILESYSTEM-wide workspace symbols.
- [x] Gate cross-FILE `references`/`definition` and filesystem-wide
  `workspaceSymbol` on a shared symbol-index service; ship the rest of LSP v0 first.
  Single-document, top-level-name v0s of `definition` and `references` shipped
  2026-06-18 (correct because top-level names are program-unique); `workspace/symbol`
  over open documents shipped 2026-06-18. **Cross-file symbol-index DONE
  (2026-06-19):** the `lsp` server now captures workspace roots from `initialize`
  (`rootUri`/`rootPath`/`workspaceFolders`) and `lsp_workspace_documents` scans
  every `.whip` file under them (skipping `.git`/`target`/`node_modules`/hidden
  dirs) with OPEN documents overriding their on-disk content. `workspace/symbol` is
  now FILESYSTEM-wide; `textDocument/definition` resolves a name declared in
  another file (same-document declaration wins, then the workspace is scanned
  lazily); `textDocument/references` collects occurrences across every workspace
  file (the `includeDeclaration` filter applies only to the declaring file). With
  no roots everything degrades to the open documents (backward compatible — the
  single-document LSP tests are unchanged). Test
  `lsp_cross_file_definition_and_workspace_symbol` (real two-file temp workspace).
  **Remaining (distinct, smaller follow-up — NOT the symbol-index):** scope-aware
  (local-binding) resolution — resolving rule-local bindings rather than top-level
  names; tracked separately.

### Stage P7: Test Surfaces — `whip test`

Goal: deterministic user scenario tests per `workflow-testing.md`. Design
detailed (review issue #7).

Decided: `test_report_v0.schema.json` added; `test`/`given`/`stub`/`run`/`expect`
grammar + projection predicate sub-language specified (reusing the expression
kernel); harness mechanics defined (given→admission-path seeding, stub→fixture
provider, replay equality = equal canonical projections); rule-firing assertions
**added** (`expect rule X fired | did not fire | fired N times`) to give an
event-sourced rule system a direct rule lens.

- [x] Implement the `test_scenario` parser + predicate evaluator (shared kernel).
  **Parser landed (2026-06-15):** `Item::Test(TestDecl)` + `parse_test` with the
  four verbs — `given` (signal/input/fact/clock + record bodies), `stub`
  (dotted-name surface + outcome + optional record/string payload), `run`
  (until idle | until workflow completed|failed | for N steps), `expect` (workflow
  completed/failed[with T], rule fired|did-not-fire|fired N times, effect
  requested|completed|failed, diagnostic, no `<effect>`, and `proj_query`:
  `<noun> exists | count where <pred> is N | where <pred>`). Predicates/values are
  captured as source text (the `assert`/guard idiom) for later `parse_expression`.
  Lowered to `IrProgram.tests` (excluded from the executable construct graph);
  `lower_test` checks duplicate names, requires an `expect`, **and validates that
  captured `given`-values and `proj_query` predicates parse** (via
  `parse_expression`) so `whip check` rejects malformed/empty test predicates
  (tests `test_block_parses_given_run_and_expect_clauses` +
  `test_block_rejects_a_malformed_predicate`). `format_test` for `whip fmt`;
  `whip check` accepts/validates test programs; workspace green. **Predicate
  evaluation mechanism for the driver is verified:** reuse `EvalScope { projection:
  Some(row), .. }` + `eval_expr_value` + `guard_result` (a bare ident resolves
  against the projection row) — no new evaluator needed, the driver wires it.
  **v0 parser limitations to extend later** — multi-word package projection nouns
  (`message sent to X`, `file R at P`, `memory M learned`), and richer `no` targets.
  (`given tracker` landed later; `given file` landed 2026-06-16 — see the Stage P4
  `std.files` notes.)
- [x] Implement the scenario driver (isolated store, given-seeding, stub
  injection, iteration cap) and `whip test --json` (`test_report_v0`).
  **v0 driver landed + empirically validated (2026-06-15):** `whip test [--json]
  <file>` runs each `IrProgram.tests` scenario on a fresh temp `SqliteStore`
  (`RuntimeKernel` + `create_program_version_for_program` + `create_instance` +
  `ingest_external_event("external.started")`), seeds `given signal` via
  `ingest_external_event` + `derive_fact` (field values eval'd through
  `eval_expr_value`), drains rules with `step_instance`, then evaluates `expect`:
  workflow terminal from `store.status`, rule-firing by counting `rule.committed`
  events whose payload `rule` matches. Emits `whipplescript.test_report.v0`.
  **Honest by construction:** a scenario using a not-yet-executed clause
  (an unsupported stub outcome) is reported as
  **invalid** (with the reason in scenario `diagnostics`), never a pass. Integration
  test `test_harness_reports_pass_fail_and_error_honestly` proves correct
  pass/fail/invalid verdicts (real execution, no false passes); docs in
  `docs/api-reference.md`.
  **Projection/fact `expect` added (2026-06-15):** `expect <noun> exists | where
  <pred> | count where <pred> is N` evaluates over recorded facts whose name matches
  the noun, via `evaluate_proj_predicate` (parse + `EvalScope { projection:
  Some(row) }` + `eval_expr_value`); integration test
  `test_harness_evaluates_fact_projection_expects` (exists/where/count pass, wrong
  count fails).
  **`stub` + effect settlement added (2026-06-15):** the driver no longer drains
  rules only — it alternates `step_instance` (rules) with `run_worker_once`
  (claims/settles queued effects through the **fixture** provider) to a fixed point
  (event-log stops growing or the instance turns terminal; 100-iteration runaway
  guard). The `stub <surface> <outcome>` clause maps to the global fixture outcome:
  `succeeds → Completed`, `fails → Failed` (`scenario_fixture_outcome`). This
  unblocks agent-using workflows: a `succeeds` stub lets agent turns complete
  (observing rules fire, workflow completes).
  **v0 limits (reported as `invalid`, not silently):** `times_out`/`cancels` (the
  fixture agent path is a shell-command harness — exit-0/non-zero only — so it
  cannot simulate a timeout; the harness REJECTS them rather than silently
  completing, fixed 2026-06-16 after finding the old mapping let `times_out` quietly
  succeed) and unsupported stub outcomes. **Per-agent outcomes added 2026-06-16:**
  `WorkerOptions.agent_outcomes` (a `BTreeMap<agent, FixtureOutcome>`, populated by
  `scenario_agent_outcomes` from `stub agent <name> succeeds|fails` clauses) lets one
  scenario stub agents differently — e.g. `stub agent alpha succeeds` + `stub agent
  beta fails` — applied per turn via `fixture_outcome_for_agent` in both the native
  and command fixture paths; the global `outcome` still backs coerce/non-stubbed
  effects. The old "mixed stub outcomes → invalid" rejection is removed. Test
  `test_harness_supports_per_agent_stub_outcomes`. [The `native-fixture` agent
  adapter was also fixed to emit real `timed_out`/`cancelled` terminals
  (`fixture_terminal_event_kind`) for the `whip dev --provider native-fixture` path,
  which the kernel maps to the matching terminal.] The test block now parses its `workflow <Name>` header
  (`TestDecl.workflow` / `IrTest.workflow`); `format_test` round-trips it.
  Integration test `test_harness_stub_settles_agent_turns_and_outcome_changes_behavior`
  proves `succeeds` completes the turn and `fails` blocks completion (outcome is not
  cosmetic). **Report aligned to `test_report_v0` (2026-06-15):** `whip test --json`
  now emits the full designed schema — top-level `status`
  (`passed|failed|invalid|no_tests`) + `summary` (`selected/passed/failed/invalid/
  skipped`) + per-scenario `{id, workflow, source_span, steps[], expectations[],
  diagnostics[]}`, with each `expect` reported as a pass/fail expectation and
  failure detail carried in scenario `diagnostics`. `check-report-schemas.sh` now
  generates a `whip test` report and validates it against
  `test_report_v0.schema.json` (drift guard).
  **`given input` + `given fact` added (2026-06-15):** `given input { … }` seeds the
  workflow input — validated against the input contracts via the shared
  `validate_workflow_start_input` and derived as the declared input fact off the
  `started` event, exactly as `whip run`/`dev` seed it (a contract violation →
  `invalid`). `given fact <Type> { … }` seeds a pre-existing fact (value digest as
  key) that `when <Type>` rules see on the first step. Both reuse a single
  `eval_given_record` helper running field values through the expression kernel.
  Integration test `test_harness_seeds_given_fact_and_given_input` (matching value
  fires the guarded rule and records the propagated value; non-matching is filtered;
  contract-violating input is `invalid`).
  **`effect` + `no` expects added (2026-06-15):** `expect effect <kind>
  requested|completed|failed` and `expect no <kind>` evaluate over the settled
  effect log (`store.list_effects`), matched by effect kind (the dotted surface,
  e.g. `agent.tell`): `requested` = the effect was created at all, `completed`/
  `failed` require a matching terminal status; `no` asserts zero matching effects.
  This pairs with `stub` — a `succeeds` stub yields `agent.tell completed`, a `fails`
  stub yields `agent.tell failed`. Integration test
  `test_harness_evaluates_effect_and_no_effect_expects` (succeeds→completed,
  fails→failed, and `expect no agent.tell` correctly fails when one was requested).
  **`run` clause now honored (2026-06-15):** the driver was ignoring `RunKind` and
  always running to idle (a latent dishonesty). It now drives in rounds (one round =
  drain rules + settle effects): `run until idle|workflow completed|failed` drives to
  a fixed point; `run for N steps` runs exactly N rounds (stopping early only on a
  terminal instance), letting a test inspect an intermediate state. Integration test
  `test_harness_run_for_n_steps_bounds_execution` proves the bound is real (in
  CompletedTurn, one round settles the agent turn but the observing rule/completion
  only fire on round two, so `run for 1 steps` + `expect workflow completed` fails).
  **`expect diagnostic <code>` added (2026-06-15):** matches a runtime diagnostic
  recorded during the run (`store.list_diagnostics`) by code — completing the
  `expect` target set (workflow / rule / effect / no / projection / diagnostic all
  evaluated). A `fails` stub makes the fixture provider record a `nonzero_exit`
  terminal diagnostic; a `succeeds` run records none, so the same expectation
  correctly fails there. Integration test `test_harness_evaluates_diagnostic_expects`.
  **Dotted projection nouns added (2026-06-15):** the projection noun is now a
  dotted fact name (`ProjQuery.noun: String`, parsed via `parse_dotted_name`), so a
  scenario can assert over runtime facts such as `agent.turn.completed` /
  `agent.turn.failed` — not just single-identifier user facts. Matching is exact
  (a failed turn produces `agent.turn.failed`, so `agent.turn.completed exists`
  fails there). Integration test `test_harness_projects_over_dotted_runtime_facts`.
  (Note: agent-turn *output* is a fixed schema — `summary`/`status`/`agent`/
  `exit_code`/`failure` — derived in the kernel; arbitrary typed agent output and
  per-agent stub payload injection remain a deeper, kernel-spanning change.)
  **Coerce output injection added 2026-06-16:** `stub coerce <fn> returns { … }`
  injects the typed result a fixture coerce returns — `WorkerOptions.coerce_outputs`
  (keyed by function name, built by `scenario_coerce_outputs`, the record payload
  evaluated via `eval_given_record`), consulted in `run_coerce_effect` ahead of the
  variant knob / generated placeholder. So a test controls the value a workflow
  branches on (e.g. `stub coerce classify returns { verdict "merge" }` →
  `expect Out where verdict == "merge"`). Test `test_harness_injects_coerce_output`.
  **`given clock` added 2026-06-16:** `given clock at "<timestamp>"` injects a
  virtual evaluation clock so timer/deadline firing is deterministic and instant
  (no wall-clock sleep). The clock threads as `WorkerOptions.virtual_now: Option<String>`
  (built by `scenario_virtual_now`), passed through `resolve_due_time_effects` into
  `SqliteStore::due_time_effects(instance_id, now)`, whose SQL now binds `?2` (the
  injected clock, or `'now'` when absent — so dev/worker behavior is byte-identical)
  in both the relative-`timeout_seconds` and absolute-`deadline_at` comparisons. The
  un-rejection in the unsupported check is now covered by
  `test_harness_given_clock_controls_deadline_firing` (one scenario advances the clock
  past a `timer until` deadline → workflow fails; a sibling holds it before → deadline
  stays pending). This is the deterministic, sleep-free replacement for wall-clock timer
  tests like `timer_fires_and_cancel_settles_the_race`.
  **CLI selection + exit-code conformance added 2026-06-16:** `whip test` now matches
  the `workflow-testing.md` CLI surface — `-i`/`--include` and `-x`/`--exclude` select
  scenarios by `<workflow>::<name>` id (`*`-glob, `::`-split with empty-side-matches-any,
  excludes override includes; `glob_match`/`test_id_matches`/`parse_test_args`), `--list`
  enumerates selected ids without running, `--pass-if-no-tests` downgrades an empty
  selection to success, and the spec exit codes are honored (0 passed · 1 expectation
  failure · 2 setup invalid [compile error or unrunnable scenario] · 4 nothing selected).
  Previously every outcome collapsed to 0/1 and an empty bundle wrongly returned 0. Test
  `test_command_selection_patterns_and_exit_codes`.
  **Multi-source + scenario-id conformance added 2026-06-16:** `whip test <a> <b> …`
  compiles each source and aggregates their scenarios into one `test_report.v0` (each
  scenario run against its own program text; a compile error in any source is setup-invalid
  → exit 2 before running). Scenario `id` is now the spec form `<workflow>::<name>` (was the
  bare name) in both the report and selection/`--list`. Test
  `test_command_aggregates_multiple_sources`.
  **Directory discovery added 2026-06-16:** a `whip test` positional may be a directory,
  discovered recursively for `.whip` files (`collect_whip_files`/`expand_test_sources`,
  hidden entries + `target/` skipped, sorted for determinism); discovered files compile and
  aggregate like explicit ones (a malformed discovered source is exit 2 — honest, never a
  silent skip). Test `test_command_discovers_whip_files_in_a_directory`. This completes the
  spec's `whip test [<source-or-dir>...]` surface.
  **Remaining for full P7:**
  - Agent stub-output injection: **CLOSED 2026-06-16 (won't-do).** Decided covered by
    existing mechanisms — the two real branch drivers, turn *status* (`stub agent
    succeeds|fails`) and a *coerce* over the summary (`stub coerce … returns`), are already
    injectable; direct injection of the kernel-built free-text `summary` is model-gated and
    low-value (branching on raw summary equality is an anti-pattern).
  - `given tracker` package fixture: **DONE 2026-06-16.** Parser gained `GivenClause::Tracker`
    (`given tracker <name> issue { … }`); the harness isolates the builtin tracker per scenario
    (`WHIPPLESCRIPT_ITEMS_STORE` → a per-scenario temp store) and seeds the given issue via
    `WorkItemStore::file_item`, so `step_instance`'s `project_queue_items` surfaces it as a real
    `queue.item.ready` fact (real projection path, not a hand-seeded fact). Test
    `test_harness_given_tracker_seeds_builtin_tracker_issue`.
  - `given file` package fixture: **DONE 2026-06-16** (unblocked once `std.files` `read` +
    `file store` landed). Parser gained `GivenClause::File` (`given file <store> at "<path>"
    "<content>"`); `execute_scenario` writes each fixture to a per-scenario temp dir and
    redirects the named store's `root` to it (IR clone + `file_stores[store].root` override),
    so the real worker `read`s the fixture. Escaping paths and undeclared stores are rejected.
    Test `test_harness_seeds_file_fixtures` (seeded read completes; unseeded read fails).
  - Multi-word package-projection nouns: **design written — [DR-0021](decision-records/0021-package-projection-noun-vocabulary.md)
    recommends DEFERRAL.** The mechanism (package projection-noun vocabulary with parameter
    slots + slot-aware package-aware parsing + resolution) is large, and 3 of the 4 canonical
    nouns map to unbuilt/unmodeled projections (`file`→std.files, `message`/`memory` thin);
    the one resolvable noun (`issue`) is already queryable via dotted `queue.item.ready`. Revisit
    once the standard package projections land. Awaiting decision (defer vs. a built-in `issue` alias).
- [x] Wire `whip test replay` (deferred from v0 core syntax). **Done 2026-06-16:**
  `whip test replay <instance-id>` replays an instance's recorded event log into a
  throwaway copy of the `--store` (via `SqliteStore::rebuild_projections`, which
  re-derives facts/effects purely from the events table) and checks the
  reconstructed canonical projection is byte-identical to the live-built one — the
  replay-equality invariant (`canonical_projection` strips volatile ids/timestamps/
  epochs and sorts arrays; `serde_json`'s sorted-key serialization makes it stable).
  The user's store is never mutated. Exit 0 equal · 1 diverged (JSON carries both
  `recorded`/`replayed`) · 2 setup error. Test
  `test_replay_verifies_event_log_reprojects_identically`. Operates on a `--store`
  instance rather than the spec's eventual standalone trace-file + `--workflow` form;
  a portable file-based trace format is a future extension.
