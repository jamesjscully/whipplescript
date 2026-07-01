# Workflow Composition Transition Tracker

Status: active tracker

This tracker covers the transition from the current single-source workflow
implementation to the explicit composition model:

- `workflow` is the deployable and invokable runtime boundary.
- `pattern` is a compile-time reusable building block.
- `rule` is the runtime rewrite unit.
- `include` composes source files.
- `use name` imports package/library surface.
- `apply` expands patterns before runtime.
- `invoke` starts durable child workflows.
- `complete` and `fail` are the only source-level workflow terminal actions.

The target behavior is specified in [language.md](language.md),
[control-plane.md](control-plane.md), and
[execution-contract.md](execution-contract.md). The formal sketch is in
[../models/maude/kernel.maude](../models/maude/kernel.maude) and
[../models/maude/tests/workflow-composition.maude](../models/maude/tests/workflow-composition.maude).

## Current Position

| Area | Status | Notes |
| --- | --- | --- |
| Conceptual spec | [x] | `workflow`, `pattern`, `rule`, `include`, `use`, `apply`, `invoke`, `complete`, and `fail` are described. |
| Maude model | [x] | Pattern elaboration, workflow terminal actions, child invocation resolution, and cancellation are modeled. |
| `use` cleanup | [x] | Source now uses `use memory` for package/library imports; removed `use plugin` and `use skill` forms are rejected. |
| Parser/runtime implementation | [~] | `include` parsing, CLI source-bundle resolution, duplicate-include diagnostics, explicit multi-workflow root selection, workflow contract IR, class-shaped terminal payload validation, keyed workflow input validation/seeding, pattern expansion with type and simple value arguments, static workflow invocation target/input/direct-recursion validation, resumable dev-worker child workflow invocation with source-span records and success/failure/timeout/cancellation projection, status JSON parent/child invocation links, non-class terminal contract rejection, and basic `complete`/`fail` runtime terminal actions are implemented; scalar terminal payload syntax remains. |
| Examples migration | [x] | Done: all 37 `examples/*.whip` begin with an explicit `workflow` declaration (2026-07-01 reconcile). |
| E2E coverage | [~] | Parser/runtime e2e now covers `include`, explicit root selection, keyed workflow inputs, `pattern`/`apply` with simple value arguments, static workflow invocation target/input validation, `workflow.invoke` child success/failure/timeout/cancellation projection, resumable running invocation completion, status JSON invocation links, and `complete`/`fail`. |

## Acceptance Gates

- [x] A source bundle with includes compiles deterministically and records the
  include closure, bundle hash, root workflow, and diagnostics. Include closure +
  per-include `source_hash` (kernel/lib.rs:2571), cycle/dup/root diagnostics, and
  the whole-closure `bundle_hash` (kernel/lib.rs:2571, test
  `analysis_summary_reports_a_stable_whole_closure_bundle_hash`) all recorded in
  `program_analysis_summary_json`.
- [x] A file may contain multiple explicit `workflow` declarations; commands
  require `--root` or equivalent selection when ambiguous.
- [~] `pattern` declarations elaborate into first-order declarations before
  runtime with hygienic generated names and source provenance.
- [~] `apply` cannot create runtime recursion, hidden effects, or declarations
  outside its allowed expansion scope.
- [x] `workflow` declarations have typed `input`, `output`, and `failure`
  contracts. (`WorkflowContractDecl`/`WorkflowContractKind`, parser lib.rs:208;
  `lower_workflow_contract` lib.rs:5280.)
- [x] `complete <output> <payload>` atomically validates output, appends a
  terminal workflow event, stores terminal payload, and marks the instance
  completed. (Typecheck `validate_workflow_terminal_payload` lib.rs:6684; atomic
  commit `rule_commit_with_workflow_terminal_updates_instance_atomically`
  store/lib.rs:8959; payload persist `workflow_terminal_payload` store/lib.rs:7585.)
- [x] `fail <failure> <payload>` atomically validates failure payload, appends a
  terminal workflow event, stores terminal payload, and marks the instance
  failed. (Same paths, `WorkflowTerminalKind::Failed`.)
- [x] Terminal instances cannot commit additional rule effects or user-fact
  mutations after completion/failure/cancellation.
  (`terminal_instance_statuses_are_absorbing` store/lib.rs:9806;
  `duplicate_terminal_completion_rolls_back_event` store/lib.rs:9264.)
- [x] `invoke Workflow { ... } as name` creates a durable child workflow
  invocation, not an inline expansion. (`run_workflow_invoke_effect` main.rs:25478;
  `workflow_invocations` table, migration 0001:154.)
- [x] Parent workflows observe child terminal state through typed invocation
  completion/failure/timeout/cancellation outputs. (Projection tests
  `dev_projects_{failed,timed_out,cancelled}_child_workflow_invocation`.)
- [x] Provider and harness failures remain effect/run events and evidence; they
  do not automatically fail a workflow unless source rules choose to `fail`. (Done: 503 auto-fail is scoped to UNHANDLED effect failures in self-terminating flows — main.rs ~20105; see [project-503-autofail].)
- [ ] The generated Maude/check path can represent pattern provenance,
  workflow terminal actions, and invocation edges from compiled IR.
- [ ] Examples and docs use one canonical spelling for each concept.

## Phase 1: Source Bundles And Imports

- [x] Define concrete grammar for `include "path.whip"` and allowed path forms.
- [ ] Decide whether coerce imports use `include "types.coerce"`, a separate coerce
  declaration, or generated source bundle members.
- [~] Implement include resolution with cycle detection and stable ordering.
- [~] Preserve per-file source spans through parse, typecheck, diagnostics, and
  formatted output.
- [~] Record include closure and content hashes in typed IR / program metadata.
- [~] Add diagnostics for missing include, duplicate include, include cycle,
  non-file include target, and unsupported extension.
- [~] Add CLI tests for single file, included library file, ambiguous roots, and
  explicit root selection.

## Phase 2: Explicit Workflows

- [x] Add AST/IR nodes for top-level `workflow` declarations.
- [ ] Move current file-level declarations into an implicit compatibility root
  only as a migration bridge, with diagnostics nudging explicit syntax.
- [~] Define allowed top-level declarations inside and outside a workflow.
- [~] Implement root selection for `check`, `dev`, `deploy`, and generated IR
  snapshots.
- [x] Add workflow input binding syntax and runtime start payload validation.
- [~] Add workflow `output` and `failure` contract declarations.
- [ ] Ensure workflow-local names do not leak into sibling workflows.
- [ ] Ensure shared schemas, coerces, patterns, agents, and capabilities have
  explicit local/global scoping rules.

## Phase 3: Patterns And Apply

- [~] Add AST/IR nodes for `pattern` declarations with typed parameters.
- [~] Specify and implement the allowed pattern body surface.
- [~] Implement `apply Pattern { ... }` with typed argument validation.
- [~] Generate hygienic names for expanded rules/effects/facts.
- [~] Attach provenance for every generated declaration back to both the pattern
  definition and application site.
- [x] Reject recursive pattern application. v0 decision: pattern expansion is
  **non-recursive only**; any `apply` that reaches another `apply` of a pattern
  already on the expansion stack is the compile-time error
  `graph.unbounded_pattern_recursion` (severity `error`). Bounded recursion is
  deferred and, when added, requires a statically-decreasing structural measure
  over a finite structure. See [language.md](language.md#patterns) and
  [static-analysis.md](static-analysis.md). **Implemented 2026-06-18:**
  `detect_pattern_recursion` builds the pattern-application graph and rejects any
  pattern reaching itself (self or transitive cycle), naming the cycle; modeled in
  `models/maude/pattern-recursion.maude`; fixture
  `examples/invalid/recursive-pattern.whip`. Non-recursive *nested* apply
  expansion (transitive elaboration) remains a separate deferred slice (line 159).
- [x] Add formatter support that preserves author-written `pattern` and `apply`
  syntax rather than formatting expanded output.
- [~] Add golden IR snapshots that show generated declarations plus provenance.

## Phase 4: Terminal Workflow Actions

- [x] Add parser support for `complete <output-name> <payload>` and
  `fail <failure-name> <payload>` in rule bodies.
- [~] Typecheck terminal payloads against the current workflow contract.
- [x] Reject `complete`/`fail` outside workflow rule bodies.
- [~] Reject terminal actions in pattern bodies. v0 decision: `complete`/`fail`
  inside a `pattern` body is always a compile-time error (severity `error`);
  patterns elaborate into rules, and workflow terminals belong to the owning
  workflow contract. A reusable body reaches a terminal by recording a result
  fact that a workflow rule turns into `complete`/`fail`. (The earlier
  "unless the pattern contract explicitly allows terminal expansion" option is
  dropped for v0.)
- [x] Make terminal action commits atomic with the rule commit.
- [x] Persist workflow terminal events and terminal payloads in the store.
- [x] Block further effectful rule commits after terminal state.
  (`terminal_instance_statuses_are_absorbing` store/lib.rs:9806.)
- [x] Terminal tie-break: under the deterministic fixpoint (rule declaration
  order, then earliest triggering fact sequence; see
  [semantics.md](semantics.md)), the **first committed terminal wins**. Once a
  `complete`/`fail` commits, the instance is terminal: no further effectful rule
  commits and no second terminal commits. A later rule that would also have
  reached a terminal does not fire (its matched state was consumed by the
  terminal commit, and the post-terminal guard rejects it). See
  [language.md](language.md#workflow-contracts-and-invocation).
  (Absorbing-status + `duplicate_terminal_completion_rolls_back_event`
  store/lib.rs:9264.)
- [~] Add status/diagnostics output that clearly distinguishes workflow failure
  from provider/effect failure.

## Phase 5: Durable Workflow Invocation

- [x] Add parser support for `invoke Workflow { ... } as binding`.
- [x] Typecheck invocation input against the target workflow input contract.
- [~] Validate target workflow visibility and authorization.
- [x] Persist invocation records with parent instance, child instance, target
  workflow, input payload, and source span. (`record_workflow_invocation`
  store/lib.rs:1663; `workflow_invocations` incl. `source_span_json`, 0001:154.)
- [x] Start child instances through the same durable runtime path as root
  instances.
- [x] Project child `complete`, `fail`, timeout, and cancellation into typed
  parent invocation terminal outputs.
- [x] Support `after invocation succeeds`, `after invocation fails`, and
  `after invocation completes` using the existing tagged terminal matching
  model. (Predicate parse body.rs:3036; `invoke_binding_workflow` lib.rs:4903.)
- [x] Ensure child provider failures do not bypass child workflow rules or
  directly complete the parent invocation.
  (`failed_child_invocation_drives_parent_failure_branch` control_plane.rs:5759.)

## Phase 6: Static Analysis And Verification

- [ ] Extend name resolution to model source bundles, workflow-local scopes,
  pattern-local scopes, and generated scopes.
- [ ] Extend cycle analysis so compile-time pattern recursion and runtime
  workflow invocation cycles are checked separately.
- [x] Add termination/boundedness diagnostics for pattern expansion. v0 target:
  emit `graph.unbounded_pattern_recursion` (severity `error`) for any recursive
  `apply`; bounded-recursion analysis is deferred. **Done 2026-06-18** via
  `detect_pattern_recursion` (reachability over the pattern-application graph);
  the compile-time pattern-recursion half of the cycle-analysis line above (runtime
  invocation-cycle analysis is separate, line 169).
- [~] Add invocation graph diagnostics for missing root, ambiguous target,
  unauthorized target, and unsupported recursive invocation.
- [ ] Generate Maude fixtures from compiled IR for workflow terminal and
  invocation invariants.
- [x] Add expected-failure fixtures for broken terminal validation, post-terminal
  mutation, and direct parent completion without child terminal state.
  Broken terminal validation → check-time fixture
  `examples/invalid/bad-terminal-payload.whip` (+`.diagnostics`, in
  `invalid_fixtures_have_actionable_diagnostics`). The other two are *runtime*
  invariants (no check-time fixture applies) and are guarded by store/e2e tests:
  post-terminal mutation → `terminal_instance_statuses_are_absorbing` /
  `duplicate_terminal_completion_rolls_back_event` (store/lib.rs:9806/9264);
  direct parent completion without child terminal → parent only completes via
  child-terminal projection, `failed_child_invocation_drives_parent_failure_branch`
  (control_plane.rs:5759) + the three `dev_projects_*_child_workflow_invocation`.

## Phase 7: Runtime, Store, And CLI

- [ ] Add store schema for programs with source bundles and multiple workflows.
- [ ] Add store schema for workflow terminal payloads and invocation records.
- [ ] Add migration strategy for existing SQLite stores used by tests.
- [x] Update the kernel transaction boundary for terminal workflow commits.
  (Terminal commit is atomic with the rule commit —
  `rule_commit_with_workflow_terminal_updates_instance_atomically` store/lib.rs:8959.)
- [x] Update worker/stepper scheduling to run child workflow instances.
  (`worker_resumes_running_workflow_invocation` control_plane.rs:11496.)
- [~] Update `whip status` to show parent/child invocation trees.
- [ ] Update JSON traces to include source bundle, workflow id, pattern
  provenance, invocation id, and terminal payload references.
- [ ] Update `whip diagnostics` to group errors by file, workflow, pattern
  application, and generated declaration.

## Phase 8: Examples And Docs

- [x] Rewrite core examples with explicit `workflow` declarations. (Done: 37/37 examples workflow-prefixed.)
- [x] Add at least one library file included by multiple workflows.
  (`examples/includes/support-lib.whip` included by both
  `examples/include-triage.whip` and `examples/include-audit.whip`; both in the
  docs-examples gate.)
- [ ] Add at least one reusable `pattern` used in multiple workflows.
- [ ] Add a parent workflow that invokes a child workflow and handles success,
  declared failure, timeout, and cancellation.
- [ ] Update quickstart, language sketch, examples spec, companion skill, and
  troubleshooting docs to use the canonical model.
- [ ] Document the canonical explicit-workflow shape in examples and quickstart.
- [ ] Remove or downgrade examples that imply lifecycle patterns are built into
  the language.

## Phase 9: E2E And validation

- [x] Add deterministic fixture-provider e2e for include plus explicit root
  selection. (`dev_runs_included_bundle_with_explicit_root_selection`
  control_plane.rs — `dev --root Selected` on an include+multi-workflow bundle
  runs to completion.)
- [x] Add deterministic fixture-provider e2e for pattern application provenance.
  (`dev_runs_rule_generated_by_pattern_application` control_plane.rs:11134.)
- [x] Add deterministic fixture-provider e2e for workflow complete/fail.
  (`complete`: `dev_complete_terminal_action_marks_instance_completed`
  control_plane.rs:10811; direct `fail`: `dev_fail_terminal_action_marks_instance_failed`
  control_plane.rs:12753 — asserts failed status + `workflow.failed` event.)
- [x] Add deterministic fixture-provider e2e for parent-child invocation.
  (`dev_creates_workflow_invoke_effect` control_plane.rs:11212; projection tests;
  `worker_resumes_running_workflow_invocation` control_plane.rs:11496.)
- [ ] Add validation workflow that reviews each phase of this tracker using a child
  workflow invocation per phase.
- [ ] Add opt-in real-provider validation that invokes Codex, Claude, and Pi review
  workflows and validates outputs through `coerce`.
- [x] Verify failed provider runs appear in the event stream without directly
  failing workflow instances unless source rules say so.
  (`dev_fixture_failure_reaches_event_stream` control_plane.rs:4965.)

## Open Decisions

- [ ] Exact syntax for workflow contracts:
  `input Name { ... }`, `output Name { ... }`, `failure Name { ... }`, or a
  compact signature form.
- [ ] Whether coerce files are included directly or referenced through a generated
  source-bundle member.
- [x] Whether pattern bodies may contain terminal actions: resolved. v0 forbids
  `complete`/`fail` in pattern bodies entirely (compile-time `error`); no pattern
  capability/contract escape hatch in v0.
- [x] Whether recursive *pattern application* is allowed: resolved. v0 is
  non-recursive-only (`graph.unbounded_pattern_recursion`); bounded recursion is
  deferred pending a statically-decreasing structural measure.
- [ ] Whether recursive *workflow invocation* (runtime `invoke` cycles, distinct
  from compile-time `apply`) is rejected in v0 or allowed only with explicit
  policy limits.
- [ ] How much implicit compatibility syntax remains after examples migrate.

## Next Implementation Slice

1. Implement source bundle parsing and explicit root workflow selection.
2. Add explicit `workflow` AST/IR while preserving current examples through a
   temporary compatibility root.
3. Add terminal `complete`/`fail` syntax and runtime store support before
   implementing `pattern`/`apply`, because invocation needs terminal contracts.
