# Workflow Composition Transition Tracker

Status: **closed 2026-07-16** — both keystones + all 15 Acceptance Gates shipped;
the coerce-import decision is resolved (defer via `include`), the docs BAML→coerce
sweep is done (0 `baml` in docs). Remaining `[~]` items are correct deferrals
(decl-kind restrictions, invoke authorization, showcase polish) needing a policy
decision, not v0.1 work. Re-surface on demand.

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
| `use` cleanup | [x] | Source now uses `use <package>` (e.g. `use std.memory`) for package/library imports; removed `use plugin` and `use skill` forms are rejected. |
| Parser/runtime implementation | [x] | `include` parsing, CLI source-bundle resolution, duplicate-include diagnostics, explicit multi-workflow root selection, workflow contract IR, class-shaped terminal payload validation, keyed workflow input validation/seeding, pattern expansion with type and simple value arguments, static workflow invocation target/input/direct-recursion validation, resumable dev-worker child workflow invocation with source-span records and success/failure/timeout/cancellation projection, status JSON parent/child invocation links, both class-shaped AND scalar terminal payloads (`complete result 0.9` against a scalar contract), and basic `complete`/`fail` runtime terminal actions are implemented. |
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
  runtime with hygienic generated names and source provenance. **Partial:**
  elaboration + hygienic names shipped (`expand_pattern_applications` lib.rs:3961);
  provenance is name-level, source-span back-links deferred (see Phase 3).
- [~] `apply` cannot create runtime recursion, hidden effects, or declarations
  outside its allowed expansion scope. **Partial:** recursion is fully blocked
  (`detect_pattern_recursion` lib.rs:3871); hidden-effect / out-of-expansion-scope
  containment is not separately enforced (relies on the terminal-in-pattern reject
  + expansion emitting only first-order decls). Deferred — no known escape today.
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
- [x] The generated Maude/check path can represent pattern provenance,
  workflow terminal actions, and invocation edges from compiled IR. **Shipped
  2026-07-01 (commit 5d233c8).** `generate_maude_model_search` now emits searches
  over `ir.pattern_applications` (elaborate-pattern), Output/Failure
  `workflow_contracts` (complete/fail-workflow), and `WorkflowInvoke` effects
  (start/complete/fail-workflow-invocation) against the existing kernel rules —
  `collect_composition_symbols` synthesizes the PatternId/WorkflowId/InstanceId/
  OutputId/FailureId ops and `append_composition_model_searches` emits paired
  coverage + bite (RESIDUAL soup) searches. Tests
  `generates_composition_model_searches_from_ir` +
  `generated_composition_model_search_runs_clean_in_maude` (assert warning-free +
  expected outcomes); verified against real examples.
- [~] Examples and docs use one canonical spelling for each concept. **Partial:**
  examples are canonical (37/37 explicit `workflow`; new include/pattern/parent-child
  examples use the canonical spellings); the cross-doc canonicalization sweep is the
  outstanding half (see Phase 8 docs items). Deferred with that sweep.

## Phase 1: Source Bundles And Imports

- [x] Define concrete grammar for `include "path.whip"` and allowed path forms.
- [x] Decide whether coerce imports use `include "types.coerce"`, a separate coerce
  declaration, or generated source bundle members. **RESOLVED (Jack): defer** — share
  coerces via `include` of the `.whip`; no `.coerce` import built (see the canonical
  entry in Open Decisions below). `include` accepts only `.whip`; coerce definitions
  live inline, so no coerce-import mechanism is forced.
- [x] Implement include resolution with cycle detection and stable ordering.
  (`SourceBundleResolver` main.rs:37432 — active-stack cycle detection, visited
  dedup, deterministic pre-order concat.)
- [x] Preserve per-file source spans through parse, typecheck, diagnostics, and
  formatted output. **Shipped 2026-07-01 (commit 7785aec).** The include-bundle
  resolver builds a sorted per-file `SourceSegment{start,path}` table during
  concatenation; `resolve_span_file` maps any combined-source offset back to its
  originating file + in-file offset, and `render_bundle_diagnostic` renders against
  the real file with the file's own line numbering (wired into report_compile_failure,
  check, compile warnings, parse/include-error paths). Tests
  `resolve_span_file_maps_offset_to_originating_file` +
  `included_file_diagnostic_names_the_included_file`.
- [x] Record include closure and content hashes in typed IR / program metadata.
  (`IrInclude{path,source_hash}` main.rs:37531; whole-closure `bundle_hash`
  kernel/lib.rs:2571; surfaced in `include_closure`.)
- [x] Add diagnostics for missing include, duplicate include, include cycle,
  non-file include target, and unsupported extension. (main.rs:37461 cycle,
  :37493 duplicate, :37506 absolute-path, :37520 non-`.whip`, :37530 missing.)
- [x] Add CLI tests for single file, included library file, ambiguous roots, and
  explicit root selection. (`check_resolves_relative_whip_includes`,
  `check_rejects_duplicate_includes_in_one_file`,
  `check_selects_root_from_multiple_explicit_workflows`,
  `check_root_option_validates_current_workflow_name`.)

## Phase 2: Explicit Workflows

- [x] Add AST/IR nodes for top-level `workflow` declarations.
- [x] Move current file-level declarations into an implicit compatibility root
  only as a migration bridge, with diagnostics nudging explicit syntax.
  **Resolved by removal (2026-07-01).** No implicit/compatibility root is kept: a
  bundle that declares no `workflow` at all is rejected at compile time
  (`select_root_workflow` lib.rs, `program declares no `workflow``), so no nudge
  is needed — the header form (`workflow Name`) and the block form
  (`workflow Name { ... }`) are the only entry shapes, both explicit. Tests
  `rejects_headerless_program_with_no_workflow` / `accepts_single_workflow_header_program`;
  fixture `examples/invalid/headerless-library.whip`. See
  `models/maude/workflow-scoping.maude` (headerless reject) and the RESOLVED Open
  Decision below.
- [~] Define allowed top-level declarations inside and outside a workflow.
  **Partial (un-gated 2026-07-01).** Name *scoping* by position now ships
  (top-level = global, workflow-block = private; whole-program validation), but the
  `Item` enum still admits every decl *kind* anywhere (lib.rs:126) — there is no
  "this kind may only appear at top level / only inside a workflow" restriction.
  No longer decision-blocked; a scoped follow-on if a real must-be-here rule
  emerges (e.g. terminals only inside a workflow, already enforced separately).
- [x] Implement root selection for `check`, `dev`, `deploy`, and generated IR
  snapshots. (`select_root_workflow` lib.rs:2377, invoked in the compile path
  lib.rs:1715; `--root` plumbed through the CLI.)
- [x] Add workflow input binding syntax and runtime start payload validation.
- [x] Add workflow `output` and `failure` contract declarations.
  (`WorkflowContractKind::{Output,Failure}` parsed lib.rs:16548.)
- [x] Ensure workflow-local names do not leak into sibling workflows. **Shipped
  2026-07-01.** The whole-program validation pass (`compile_program_with_root`
  lib.rs) lowers **every** workflow against its own scope — top-level globals +
  that workflow's own block-local declarations — so a name declared privately
  inside one workflow cannot satisfy a reference in a sibling; the reference is an
  unknown-name error, and `annotate_cross_workflow_leak` (lib.rs) attaches a
  related note pointing at the sibling's declaration ("`X` is declared inside
  workflow `B`… move it to a top-level declaration to share it"). Tests
  `cross_workflow_reference_to_sibling_local_is_annotated` +
  `shared_top_level_name_is_not_annotated_as_a_leak`. Modeled as the leak/isolation
  property in `models/maude/workflow-scoping.maude`.
- [x] Ensure shared schemas, coerces, patterns, agents, and capabilities have
  explicit local/global scoping rules. **Shipped 2026-07-01 (RESOLVED Open
  Decision below: remove implicit root, one-program-many-workflows with
  workflow-local scoping).** Scope is lexical by position (spec/language.md "Scope
  And Visibility"): a top-level declaration is global across the include closure;
  a declaration nested in a `workflow { ... }` block is private to that workflow.
  Both parts landed: (1) truly-headerless programs are rejected
  (`select_root_workflow` lib.rs); (2) all workflows compile+validate together
  (whole-program pass) instead of flatten-and-discard, each against globals + its
  own locals. This unblocked the name-leak check (above) and Phase 6 scoped
  resolution. Deferred-with-cause below: in/out-of-workflow decl-*kind* restrictions
  (the `Item` enum still admits every kind anywhere; scoping is enforced but no
  "this kind may only appear at top level" rule), bundle store schema (Phase 7),
  diagnostics grouping. Model: `models/maude/workflow-scoping.maude` (coverage 6 /
  bite 3).

## Phase 3: Patterns And Apply

- [x] Add AST/IR nodes for `pattern` declarations with typed parameters.
  (`PatternDecl{type_params}` lib.rs:186; `IrPatternApplication` lib.rs:857.)
- [x] Specify and implement the allowed pattern body surface. **Shipped 2026-07-01
  (commit 7785aec).** `pattern_body_admission` is the explicit allow-list gate: a
  pattern body may contain rules/effects/records/local schemas/tables/agents/
  coordination resources, but NOT workflow contracts, nested pattern declarations,
  nested applications, or a rule that reaches a workflow terminal (`complete`/`fail`)
  — each rejected with a clear diagnostic. Modeled in
  `models/maude/pattern-body-surface.maude` (6 coverage / 2 bite); tests
  `rejects_terminal_statement_in_pattern_body` +
  `rejects_workflow_contract_in_pattern_body`. (Recursive nested `apply` stays
  rejected per the pattern-recursion decision — line 198.)
- [x] Implement `apply Pattern { ... }` with typed argument validation. (Type +
  simple value args; `expand_pattern_applications` lib.rs:3961, test
  `expands_pattern_applications_with_hygienic_names`.)
- [x] Generate hygienic names for expanded rules/effects/facts.
  (`IrPatternApplication.generated`; hygiene tests lib.rs:19789.)
- [x] Attach provenance for every generated declaration back to both the pattern
  definition and application site. **Shipped 2026-07-01 (commit 7785aec).**
  `IrPatternApplication` now carries `definition_span` (the `pattern` decl) and
  `application_span` (the `apply` site), both surfaced in the `.ir` snapshot as
  `defined-at`/`applied-at` (in addition to the name-level `pattern`+`alias`+
  `generated`). Golden `examples/reusable-review-pattern.ir` regenerated; test
  `pattern_application_records_definition_and_application_spans`.
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
- [x] Add golden IR snapshots that show generated declarations plus provenance.
  **Shipped 2026-07-01 (commit 7785aec + a3f09ac).** `examples/reusable-review-pattern.ir`
  now shows the generated declarations AND source-span provenance
  (`defined-at`/`applied-at`), regenerated against the current compiler; asserted by
  `example_ir_snapshots_are_stable`.

## Phase 4: Terminal Workflow Actions

- [x] Add parser support for `complete <output-name> <payload>` and
  `fail <failure-name> <payload>` in rule bodies.
- [x] Typecheck terminal payloads against the current workflow contract.
  (`validate_workflow_terminal_actions`/`_payload` lib.rs:6605-6716: unknown-terminal
  + non-class-contract + field typecheck. Scalar terminal payloads are a separate
  open item; class-shaped payloads — the v0 contract shape — are fully checked.)
- [x] Reject `complete`/`fail` outside workflow rule bodies.
- [x] Reject terminal actions in pattern bodies. v0 decision: `complete`/`fail`
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
- [x] Add status/diagnostics output that clearly distinguishes workflow failure
  from provider/effect failure. **Shipped 2026-07-01 (commit f878df4).**
  `whip status --json` now carries a `failure_surface` field distinguishing
  workflow-level failure (author `fail` vs generated `flowfail`, via
  `workflow_fail_kind`) from provider/effect failure evidence
  (`provider_failure_count`/present). Test
  `status_failure_surface_separates_workflow_fail_from_provider_evidence`. (The
  author-fail category is fully tested; the `flowfail`/internal branch is
  logic-complete.)

## Phase 5: Durable Workflow Invocation

- [x] Add parser support for `invoke Workflow { ... } as binding`.
- [x] Typecheck invocation input against the target workflow input contract.
- [~] Validate target workflow visibility and authorization. **Partial:** target
  existence + direct self-recursion are validated (`invokes unknown workflow`
  lib.rs:8786, `recursively invokes` lib.rs:8771). No *visibility/authorization
  policy* gate exists — that requires deciding what the policy is (which workflows
  may invoke which; ties to the recursive-invocation Open Decision). Deferred to
  that decision.
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
- [x] **Typed invoke results (success side) — SHIPPED 2026-07-01.** `after child
  succeeds as r` now binds `r` to the child workflow's `output` contract *class*
  so `r.<field>` is statically checked (was opaque `terminal_unknown_payload_type`).
  Static-only: the runtime already carries the child's terminal payload into the
  binding (verified by the pre-existing e2e `result.title == "Invoke smoke"`),
  so this only closed the compile-time typing gap. Resolved in `analyze_rule`'s
  succeeds/completes arm via `invoke_output_class` (`invoke_binding_workflow` →
  the target's sole `output` contract), guarded on `semantic.schemas.class_exists`
  so a child-*local* output class stays opaque (workflow-local scoping preserved);
  the `fails` arm keeps the DR-0032 `TerminalFailed` base (predicate discrimination).
  `WorkflowInputSurface.outputs` added. Model `invoke-result-typing.maude` (4/2);
  tests `typed_invoke_result_checks_field_access_against_child_output` +
  `_accepts_a_valid_child_output_field`; example `typed-invoke-result.whip`.
  Deferred: scalar child outputs (bind `r` to a scalar *value*, not a schema — a
  different mechanism; low value, runtime already exposes it).
- [x] **Typed invoke results (FAILURE side) — SHIPPED 2026-07-01 (commit 91453d9).**
  `after child fails as f` now binds `f` to the child's declared FAILURE contract
  class when it is a sole, shared, parent-scope-resolvable class (via
  `invoke_failure_class` + `WorkflowInputSurface.failures`), so `f.<field>`
  type-checks against the child's failure shape; a non-invoke fails binding
  (coerce/exec) or an unresolvable/child-local/multiple failure falls back to the
  DR-0032 `TerminalFailed` base (the `failed_child_invocation_drives_parent_failure_branch`
  regression with a child-local `CFail` still resolves `f.reason` via the base).
  Model `invoke-result-typing.maude` extended to 7/3; tests
  `typed_invoke_failure_checks_field_access_against_child_failure` +
  `_accepts_a_valid_child_failure_field`.
- [x] Ensure child provider failures do not bypass child workflow rules or
  directly complete the parent invocation.
  (`failed_child_invocation_drives_parent_failure_branch` control_plane.rs:5759.)

## Phase 6: Static Analysis And Verification

- [x] Extend name resolution to model source bundles, workflow-local scopes,
  pattern-local scopes, and generated scopes. **Shipped 2026-07-01 for the
  load-bearing scopes.** Source bundles: the include closure's top-level names are
  global. Workflow-local scopes: each workflow now resolves against globals + its
  own block-local names (whole-program validation pass), and sibling-local names
  do not resolve (leak check above). Pattern-local + generated scopes: hygienic
  qualification (`expand_pattern_applications` lib.rs, `IrPatternApplication.generated`).
  Modeled in `models/maude/workflow-scoping.maude`.
- [x] Extend cycle analysis so compile-time pattern recursion and runtime
  workflow invocation cycles are checked separately. Compile-time pattern
  recursion: `detect_pattern_recursion`. Runtime invocation cycles: direct
  self-invocation rejected per-rule (lib.rs:8766) **and** transitive cycles
  rejected program-wide by `detect_workflow_invoke_recursion` (built over all
  workflows' invoke edges before root selection) —
  `graph.unbounded_workflow_invocation_recursion`, per the 2026-07-01 convergence
  decision. Tests `rejects_transitive_workflow_invocation_cycle` +
  `accepts_acyclic_workflow_invocation_chain`; fixture
  `examples/invalid/recursive-workflow-invocation.whip`; modeled as invoke-graph
  non-convergence in `subworkflow-convergence.maude`.
- [x] Add termination/boundedness diagnostics for pattern expansion. v0 target:
  emit `graph.unbounded_pattern_recursion` (severity `error`) for any recursive
  `apply`; bounded-recursion analysis is deferred. **Done 2026-06-18** via
  `detect_pattern_recursion` (reachability over the pattern-application graph);
  the compile-time pattern-recursion half of the cycle-analysis line above (runtime
  invocation-cycle analysis is separate, line 169).
- [~] Add invocation graph diagnostics for missing root, ambiguous target,
  unauthorized target, and unsupported recursive invocation. **Partial:**
  missing-root + ambiguous (`select_root_workflow` lib.rs:2438), unknown-target
  + direct-recursive (lib.rs:8771/8786), and **transitive** recursive invocation
  (`detect_workflow_invoke_recursion`) are diagnosed; only *unauthorized* target
  remains (gated on the scoping/authorization decision). Deferred to that decision.
- [x] Generate Maude fixtures from compiled IR for workflow terminal and
  invocation invariants. **Shipped 2026-07-01 (commit 5d233c8)** — same piece as
  the "generated Maude/check path" acceptance gate above; `generate_maude_model_search`
  emits terminal (complete/fail-workflow) + invocation (start/complete/fail-workflow-
  invocation) + pattern-elaboration searches from compiled IR, verified running
  clean in Maude by test.
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

- [~] Add store schema for programs with source bundles and multiple workflows.
  **Deferred (un-gated 2026-07-01) — not yet needed.** Today the bundle is resolved
  and *flattened* into one source string before storage (`resolve_source_bundle`
  main.rs:37432), so `programs`/`program_versions` store one source + its
  `bundle_hash`/`include_closure` in the compiled IR; there is no per-member
  bundle schema. The scoping keystone shipped without needing one (whole-program
  validation works over the parsed AST, not a stored per-member schema). A
  per-member schema only matters if we later want to store the *unflattened*
  bundle; author it then. No longer decision-blocked.
- [x] Add store schema for workflow terminal payloads and invocation records.
  Invocation records: `workflow_invocations` table (migration 0001:154, incl.
  parent/child/target/input/`source_span_json`). Terminal payloads: persisted as
  the terminal event `payload_json` referenced by `terminal_event_id`
  (`workflow_terminal_payload` store/lib.rs:7585) — no dedicated table needed.
- [~] Add migration strategy for existing SQLite stores used by tests.
  **Deferred.** Migration infra exists (`apply_migrations`, `schema_migrations`,
  `MIGRATIONS` store/lib.rs:874) but everything sits in a single baseline `0001`
  and tests use fresh stores; no incremental migration has been authored because
  there is no pre-baseline store in the wild yet (pre-release). Author the first
  incremental migration when a schema change must preserve an existing store.
- [x] Update the kernel transaction boundary for terminal workflow commits.
  (Terminal commit is atomic with the rule commit —
  `rule_commit_with_workflow_terminal_updates_instance_atomically` store/lib.rs:8959.)
- [x] Update worker/stepper scheduling to run child workflow instances.
  (`worker_resumes_running_workflow_invocation` control_plane.rs:11496.)
- [x] Update `whip status` to show parent/child invocation trees. **Shipped
  2026-07-01 (commit f878df4).** `whip status --json` emits a recursive
  `invocation_tree` — each child invocation node carries a nested `children` array
  (walks `list_child_workflow_invocations` from each child, depth-guarded at 64);
  test `status_assembles_multi_level_invocation_tree` (3-level Parent→Child→GrandChild).
- [x] Update JSON traces to include source bundle, workflow id, pattern
  provenance, invocation id, and terminal payload references. **Shipped 2026-07-01
  (commit f878df4).** `whip log --json` now stamps each event with `instance_id`,
  `workflow_id`, `workflow_version_id`, and `invocation_id` (spawning invocation for
  child instances, null for roots); compiled-IR JSON already carried
  `include_closure`/`bundle_hash`/`pattern_applications`. Test
  `log_json_stamps_invocation_and_workflow_provenance`.
- [x] Update `whip diagnostics` to group errors by file, workflow, pattern
  application, and generated declaration. **Shipped 2026-07-01 (commit f878df4).**
  `whip diagnostics --grouped --json` returns a `whipplescript.diagnostics_grouped.v0`
  object grouping findings by file / workflow instance / `subject_type` (the latter
  carries pattern-application + generated-declaration provenance); default output
  stays a flat array for back-compat. Test
  `diagnostics_grouped_buckets_findings_by_provenance`.

## Phase 8: Examples And Docs

- [x] Rewrite core examples with explicit `workflow` declarations. (Done: 37/37 examples workflow-prefixed.)
- [x] Add at least one library file included by multiple workflows.
  (`examples/includes/support-lib.whip` included by both
  `examples/include-triage.whip` and `examples/include-audit.whip`; both in the
  docs-examples gate.)
- [x] Add at least one reusable `pattern` used in multiple workflows. The shared
  `TagReviewed` pattern in `examples/includes/review-pattern-lib.whip` is applied
  by both `examples/pattern-consumer-triage.whip` and
  `examples/pattern-consumer-audit.whip`; both in the docs-examples gate.
- [x] Add a parent workflow that invokes a child workflow and handles success,
  declared failure, timeout, and cancellation. (`examples/parent-child-outcomes.whip`
  — one parent rule with `after child succeeds/fails/times out/cancelled` branches;
  in the docs-examples gate with `--root Parent`.)
- [x] Update quickstart, language sketch, examples spec, companion skill, and
  troubleshooting docs to use the canonical model. Done: the
  `include`/`pattern`/`invoke`/`complete`/`fail`/`workflow` spellings were swept
  and found canonical; the BAML→coerce sweep is complete — **`grep -rin baml
  docs/` → zero** (docs use `schema.coerce` throughout; verified 2026-07-16).
- [~] Document the canonical explicit-workflow shape in examples and quickstart.
  **Partial:** the explicit `workflow` shape is present in quickstart; the new
  `include`/`pattern`/parent-child examples (above) document those shapes. Full
  cross-doc canonical-shape section deferred with the sweep above.
- [~] Remove or downgrade examples that imply lifecycle patterns are built into
  the language. **Audited 2026-07-01 (Work Item 5) — corpus is already
  Phase-8-compliant; no rewrite needed.** Nearly every pattern-heavy example carries
  an explicit "this is composition, not a primitive" framing comment (circuit-breaker,
  autoresearch-lite, reusable-review-pattern, scheduled-escalation, gastown-lite,
  openclaw-lite, human-review, queue-worker-with-review, the revision examples). One
  marginal nit left for Jack's judgment (NOT changed): `ralph.whip` lacks the
  framing comment its siblings have and its `@service` + re-tell-on-completion rules
  could read as a built-in "loop"; `docs/examples.md:39` already frames it correctly.
  Suggested optional one-liner: "# Recurrence is composition: `when ralph completed
  turn` re-triggers; there is no loop construct."

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
- [~] Add validation workflow that reviews each phase of this tracker using a child
  workflow invocation per phase. **Deferred — meta/aspirational.** This is a
  self-hosting demo (a whip workflow that invokes a child per tracker phase), not a
  correctness gate; the invocation machinery it would exercise is already covered by
  the deterministic parent-child e2e above. Build as a showcase once the surface is
  frozen. (Belongs with the gaugewright dogfood engine, per open-core notes.)
- [~] Add opt-in real-provider validation that invokes Codex and Claude review
  workflows and validates outputs through `coerce`. **Deferred — external/opt-in.**
  Requires live provider credentials and network; it is the real-provider tier
  tracked canonically in `native-provider-implementation-tracker.md` (NP live
  validation), not deterministic CI. Cross-referenced there.
- [x] Verify failed provider runs appear in the event stream without directly
  failing workflow instances unless source rules say so.
  (`dev_fixture_failure_reaches_event_stream` control_plane.rs:4965.)

## Open Decisions

Each open item carries a recommendation to make it decision-ready. These are the
true remaining gate to "finished" — the implementation items above are done or are
justified deferrals; the calls below unblock the last cluster.

- [x] **Exact syntax for workflow contracts — RESOLVED 2026-07-01 (Jack): add the
  compact single-line signature form.** The keyword form (`input Name Type` /
  `output Name Type` / `failure Name Type`) stays legal; ADD a compact form
  `workflow Name(input: Type[, ...]) -> Output [! Failure]` that desugars to the
  same contract decls (output name defaults to `result`, failure to `error`). Both
  forms are legal to write; `whip fmt` canonicalizes the compact form to keyword
  lines (single stored form, no data loss). **Building now** as its own piece.
- [x] **Coerce file imports — RESOLVED 2026-07-01 (Jack): defer.** Coerce
  definitions live inline in `.whip` today; share them by `include`ing the whole
  `.whip` library file. No dedicated `.coerce` import is built. Revisit when a real
  multi-file coerce need appears; `include` of a `.coerce` member is the smallest
  step then. Closed as deferred (not a transition blocker).
- [x] **Scalar terminal payloads — RESOLVED 2026-07-01 (Jack): allow bare
  scalars — PRODUCER SIDE SHIPPED 2026-07-01.** A scalar output/failure contract
  (`output result float`, note the primitives are `int`/`float`/`string`/`bool` —
  not `number`) is completed with a bare value (`complete result 0.9`); a class
  contract still takes a field block. Shape mismatches (block↔scalar) are rejected.
  End-to-end: parse (`body.rs` `TerminalStmt.scalar`), typecheck
  (`validate_scalar_terminal_payload`), lower (contract `ty` already general),
  runtime store (`append_workflow_terminal` evaluates the scalar to a JSON scalar,
  not an object), IFC whole-value label (`collect_egress_payload_reads` +
  conditioned-reads now fold the scalar's read-roots — fail-closed), flow-expand
  re-print. Model `models/maude/terminal-payload-shape.maude` (6 cov / 2 bite).
  Tests: 4 parser (accept value / reject block / reject class-vs-scalar / value
  typecheck) + `dev_completes_a_scalar_terminal_payload` e2e (scalar stored as a
  scalar). Examples `scalar-terminal.whip`. **CONSUMER SIDE — CLASS outputs now typed (2026-07-01); scalar
  outputs still deferred.** The orthogonal "typed invoke results" follow-on
  shipped for CLASS outputs (see Phase 5): `after child succeeds as r` now binds
  `r` to the child's output contract *class* (shared top-level), so `r.field` is
  statically checked. A SCALAR child output stays opaque — binding `r` directly to
  a scalar *value* (not a schema) is a different mechanism than the schema-map used
  for class typing, and the runtime already exposes the value regardless, so it is
  a low-value remaining slice deferred with cause.
- [x] **Recursive workflow invocation policy — RESOLVED 2026-07-01 (Jack): "as
  permissive as provable convergence at compile time allows."** Interpretation:
  whipplescript cannot prove runtime-`invoke` termination at compile time (it is
  data-dependent), and no decreasing-measure mechanism for runtime invoke exists
  yet — so the most permissive rule that still carries a compile-time convergence
  guarantee is to **reject transitive `invoke` cycles** (no convergence proof admits
  any today), exactly parallel to the bounded-pattern-recursion deferral. A future
  bounded form may admit a cycle that carries a statically-decreasing structural
  measure (or provably crosses an external event/clock boundary, Design Commitment
  7). Direct self-invocation is already rejected (lib.rs:8766); transitive is the
  new work (`graph.unbounded_workflow_invocation_recursion`). Unblocks the
  transitive cycle-analysis item.
- [x] **Implicit compatibility root / scoping keystone — RESOLVED 2026-07-01
  (Jack) + SHIPPED 2026-07-01.** Decision: remove the implicit root, require
  explicit `workflow`; move from flatten-and-discard (only `--root` compiles) to
  one-program-many-workflows with workflow-local scoping. Corpus fully migrated
  (37/37), pre-release, so no back-compat bridge. **Implemented in three pieces,
  all gated (parser 233, CLI 156, kernel/store/parser green, docs-examples green,
  `models/maude/workflow-scoping.maude` 6/3):** (1) headerless reject
  (`select_root_workflow`, `program declares no `workflow``); (2) whole-program
  validation — `compile_program_with_root` lowers every workflow against globals +
  its own locals, aggregating diagnostics, so a broken sibling is caught under any
  `--root` (this surfaced + fixed a latent broken example,
  `examples/revision-parent-child.whip`, whose child agent lacked a provider); (3)
  sibling-local leak note (`annotate_cross_workflow_leak`). Note the blast radius
  was far smaller than feared: because block workflows already segregate their
  items in `WorkflowDecl.items`, scoping was already lexical in the AST — this was
  "loop lowering per workflow with the right scope," not a name-resolution-engine
  rewrite. Root selection still produces the single entry IR for `dev`/`deploy`;
  the pass only widens validation coverage. Unblocked: workflow-local name scoping
  + leak checks (Phase 2), scoped name resolution (Phase 6). Still deferred-with-
  cause: in/out-of-workflow decl-*kind* restrictions, bundle store schema (Phase
  7), diagnostics grouping (Phase 7) — all now un-gated but not yet built.

- [x] Whether pattern bodies may contain terminal actions: resolved. v0 forbids
  `complete`/`fail` in pattern bodies entirely (compile-time `error`); no pattern
  capability/contract escape hatch in v0.
- [x] Whether recursive *pattern application* is allowed: resolved. v0 is
  non-recursive-only (`graph.unbounded_pattern_recursion`); bounded recursion is
  deferred pending a statically-decreasing structural measure.

## Remaining Work (after the 2026-07-01 reconcile + delivery pass)

The transition is substantially shipped. What remains, grouped:

1. **Decision-gated (the real blockers) — CLEARED 2026-07-01.** Both keystone
   decisions are now made *and shipped*: the recursive-invocation policy
   (transitive `invoke` cycles rejected) and the implicit-root / scoping keystone
   (headerless reject + whole-program validation + workflow-local scoping +
   sibling-leak notes; `models/maude/workflow-scoping.maude`). That un-gated the
   downstream cluster; what is left of it is now plain deferred-with-cause work,
   not decision-blocked: in/out-of-workflow decl-*kind* restrictions, the
   unflattened bundle store schema (Phase 7), invoke authorization (needs a policy,
   ties to the invoke Open Decision), and diagnostics grouping (Phase 7). None
   block the transition; each is a scoped follow-on.
2. **Model-first pieces.** Generate Maude searches from compiled IR for pattern
   provenance / terminal actions / invocation edges (kernel rules already exist;
   emit from `generate_maude_model_search`).
3. **Polish / observability (deferred-with-cause).** Recursive status-tree, JSON
   trace enrichment, source-span provenance back-links, workflow-fail-vs-provider-fail
   status field, per-file span preservation, docs canonicalization sweep.
4. **Showcase / external.** Self-hosting validation workflow; opt-in real-provider
   validation (tracked in the native-provider tracker).
