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
- [~] The generated Maude/check path can represent pattern provenance,
  workflow terminal actions, and invocation edges from compiled IR. **Deferred —
  model-first piece, de-risked.** The kernel model *already* has every rule needed
  (`elaborate-pattern` → `ruleProvenance`; `complete-workflow`/`fail-workflow` →
  `workflowCompletedEvt`/`workflowFailedEvt`; `start-workflow-invocation` +
  `complete/fail/cancel-workflow-invocation` → `invocationOutput/Failure/Cancellation`,
  kernel.maude:585/698/703/757-774). What remains is emitting those searches from
  `generate_maude_model_search` (main.rs:38686) over `ir.pattern_applications`,
  `rule.metadata.terminal_outputs`/`terminal_completes`, and `WorkflowInvoke`
  effects — synthesizing the InstanceId/WorkflowId/OutputId/FactId ops and initial
  configs. Held for a focused model-first pass (getting the generated Maude wrong
  would break the formal-models gate), not a correctness gap.
- [~] Examples and docs use one canonical spelling for each concept. **Partial:**
  examples are canonical (37/37 explicit `workflow`; new include/pattern/parent-child
  examples use the canonical spellings); the cross-doc canonicalization sweep is the
  outstanding half (see Phase 8 docs items). Deferred with that sweep.

## Phase 1: Source Bundles And Imports

- [x] Define concrete grammar for `include "path.whip"` and allowed path forms.
- [ ] Decide whether coerce imports use `include "types.coerce"`, a separate coerce
  declaration, or generated source bundle members. **Open decision** (duplicated in
  Open Decisions below — that is the canonical entry). `include` currently accepts
  only `.whip`; coerce definitions live inline in `.whip` today, so no coerce-import
  mechanism is forced yet.
- [x] Implement include resolution with cycle detection and stable ordering.
  (`SourceBundleResolver` main.rs:37432 — active-stack cycle detection, visited
  dedup, deterministic pre-order concat.)
- [~] Preserve per-file source spans through parse, typecheck, diagnostics, and
  formatted output. **Partial:** the bundle is concatenated into one `source`
  string (main.rs:37549) and spans are re-derived over the combined text, so a
  span's originating *file* is not distinctly preserved. Deferred — no diagnostic
  currently needs per-file attribution; revisit if cross-file diagnostics land.
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
- [~] Move current file-level declarations into an implicit compatibility root
  only as a migration bridge, with diagnostics nudging explicit syntax.
  **Gated on a decision.** The compatibility path exists (`program.workflow` when
  `workflows` is empty, `select_root_workflow` lib.rs:2381) but emits **no**
  nudge/deprecation diagnostic — deliberately, because whether to keep the implicit
  root at all is the open "how much implicit compatibility syntax remains" decision.
  Resolve that first (Open Decisions), then either add the nudge or remove the path.
- [~] Define allowed top-level declarations inside and outside a workflow.
  **Partial/decision:** the `Item` enum admits every decl kind anywhere
  (lib.rs:126); no in-workflow vs out-of-workflow restriction is enforced. Tied
  to the scoping decision (see "workflow-local names" below + Open Decisions).
- [x] Implement root selection for `check`, `dev`, `deploy`, and generated IR
  snapshots. (`select_root_workflow` lib.rs:2377, invoked in the compile path
  lib.rs:1715; `--root` plumbed through the CLI.)
- [x] Add workflow input binding syntax and runtime start payload validation.
- [x] Add workflow `output` and `failure` contract declarations.
  (`WorkflowContractKind::{Output,Failure}` parsed lib.rs:16548.)
- [~] Ensure workflow-local names do not leak into sibling workflows. **Gated on
  the scoping decision.** Today `select_root_workflow` (lib.rs:2449) *discards*
  non-selected workflows and flattens the selected one, so only one workflow ever
  compiles into an instance — there is no active cross-workflow name-collision check
  because siblings never coexist in a compiled program. A real leak/collision check
  only has meaning under a scoping model where siblings share a program; define that
  model first (next item).
- [~] Ensure shared schemas, coerces, patterns, agents, and capabilities have
  explicit local/global scoping rules. **This is the core open scoping decision.**
  Everything is effectively global after flattening. The design question — which
  declarations are workflow-local vs program-global, and how sharing is spelled — is
  the keystone that gates name-leak checks, in/out-of-workflow decl restrictions,
  scoped name resolution (Phase 6), the bundle store schema (Phase 7), and
  diagnostics grouping. Needs a design call before implementation. See Open Decisions.

## Phase 3: Patterns And Apply

- [x] Add AST/IR nodes for `pattern` declarations with typed parameters.
  (`PatternDecl{type_params}` lib.rs:186; `IrPatternApplication` lib.rs:857.)
- [~] Specify and implement the allowed pattern body surface. **Partial:** bodies
  expand via `expand_pattern_item` (lib.rs:4162) and terminal actions are rejected
  in pattern bodies; there is no explicit *allow-list* of permitted body constructs
  beyond that. Deferred — the deny (terminals) is the load-bearing rule.
- [x] Implement `apply Pattern { ... }` with typed argument validation. (Type +
  simple value args; `expand_pattern_applications` lib.rs:3961, test
  `expands_pattern_applications_with_hygienic_names`.)
- [x] Generate hygienic names for expanded rules/effects/facts.
  (`IrPatternApplication.generated`; hygiene tests lib.rs:19789.)
- [~] Attach provenance for every generated declaration back to both the pattern
  definition and application site. **Partial:** name-level provenance is recorded
  (`pattern`+`alias`+`generated`, lib.rs:857, surfaced in the `.ir` snapshot and
  `pattern_applications` report); **source-span** back-links to the definition and
  application site are not yet attached. Deferred — names suffice for the current
  provenance report; spans are an LSP-grade enhancement.
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
  **Partial:** `examples/reusable-review-pattern.ir` carries a
  `pattern_applications` section (emitter lib.rs:2593); provenance is name-level
  only (see the provenance item above) and only one golden covers it. Deferred
  with that provenance enhancement.

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
- [~] Add status/diagnostics output that clearly distinguishes workflow failure
  from provider/effect failure. **Partial:** `flowfail` (503 auto-fail) is
  separated from typed `fail` (lib.rs:6322, `validate_flowfail_generated_only`
  lib.rs:6422) and provider failure surfaces as evidence in the event stream
  (`dev_fixture_failure_reaches_event_stream`); there is no dedicated status-surface
  *field* that labels the two categories side by side. Deferred — a status-UX polish
  item, not a correctness gap.

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
- [x] Ensure child provider failures do not bypass child workflow rules or
  directly complete the parent invocation.
  (`failed_child_invocation_drives_parent_failure_branch` control_plane.rs:5759.)

## Phase 6: Static Analysis And Verification

- [~] Extend name resolution to model source bundles, workflow-local scopes,
  pattern-local scopes, and generated scopes. **Gated on the scoping decision.**
  Only flat/global resolution exists today (post-flatten). A scoped resolver is
  the implementation of that decision (see Phase 2 "local/global scoping rules").
- [~] Extend cycle analysis so compile-time pattern recursion and runtime
  workflow invocation cycles are checked separately. **Partial + gated.**
  Compile-time pattern recursion is fully checked (`detect_pattern_recursion`);
  the *runtime* invocation-cycle half is only a per-rule direct-self-recursion
  reject (lib.rs:8771) — transitive invoke-cycle analysis is gated on the
  "recursive workflow invocation" policy decision (Open Decisions).
- [x] Add termination/boundedness diagnostics for pattern expansion. v0 target:
  emit `graph.unbounded_pattern_recursion` (severity `error`) for any recursive
  `apply`; bounded-recursion analysis is deferred. **Done 2026-06-18** via
  `detect_pattern_recursion` (reachability over the pattern-application graph);
  the compile-time pattern-recursion half of the cycle-analysis line above (runtime
  invocation-cycle analysis is separate, line 169).
- [~] Add invocation graph diagnostics for missing root, ambiguous target,
  unauthorized target, and unsupported recursive invocation. **Partial:**
  missing-root + ambiguous (`select_root_workflow` lib.rs:2438) and unknown-target
  + direct-recursive (lib.rs:8771/8786) are diagnosed; *unauthorized* and
  *transitive* recursive invocation are not (both gated on the recursive-invocation
  + authorization decisions). Deferred to those decisions.
- [~] Generate Maude fixtures from compiled IR for workflow terminal and
  invocation invariants. **Deferred — same model-first piece as the "generated
  Maude/check path" acceptance gate above** (kernel rules exist; emit searches
  from `generate_maude_model_search`). Held for a focused pass.
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
  **Deferred — architectural decision.** Today the bundle is resolved and
  *flattened* into one source string before storage (`resolve_source_bundle`
  main.rs:37432), so `programs`/`program_versions` store one source + its
  `bundle_hash`/`include_closure` in the compiled IR; there is no per-member
  bundle schema. Adding one only matters if we want to store the *unflattened*
  bundle — a model change gated on the "how much implicit compatibility remains"
  decision. See Open Decisions.
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
- [~] Update `whip status` to show parent/child invocation trees. **Partial:**
  one level of `parent` + `children` invocation links is emitted (main.rs:40783);
  a recursive multi-level tree is not yet assembled. Deferred — status-UX polish.
- [~] Update JSON traces to include source bundle, workflow id, pattern
  provenance, invocation id, and terminal payload references. **Partial:** the
  compiled-IR JSON carries `include_closure`/`bundle_hash`/`pattern_applications`
  and status JSON carries invocation links; runtime *event* traces
  (kernel/trace.rs) do not yet uniformly stamp workflow-id/pattern-provenance/
  invocation-id. Deferred — observability enrichment, not a transition blocker.
- [~] Update `whip diagnostics` to group errors by file, workflow, pattern
  application, and generated declaration. **Deferred.** The `diagnostics` command
  exists (main.rs:178) but emits a flat list; grouping by file/workflow/
  pattern-application/generated-decl is a diagnostics-UX enhancement, most useful
  once workflow-local scoping lands (so grouping keys are meaningful). Sequenced
  after the scoping decision.

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
- [~] Update quickstart, language sketch, examples spec, companion skill, and
  troubleshooting docs to use the canonical model. **Partial:** the explicit
  `workflow` keyword is canonical in docs/quickstart.md, tutorial.md, and
  language-reference.md; a full sweep for the `include`/`pattern`/`invoke`
  canonical spellings across the companion skill + troubleshooting is outstanding.
  Deferred — doc-polish, best done as one sweep once the scoping decision settles
  the surface.
- [~] Document the canonical explicit-workflow shape in examples and quickstart.
  **Partial:** the explicit `workflow` shape is present in quickstart; the new
  `include`/`pattern`/parent-child examples (above) document those shapes. Full
  cross-doc canonical-shape section deferred with the sweep above.
- [~] Remove or downgrade examples that imply lifecycle patterns are built into
  the language. **Deferred — needs a judgment call** on which examples over-promise
  built-in lifecycle (vs demonstrating composition). A per-example audit for Jack's
  review, not a mechanical change. See Open Decisions.

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
- [~] Add opt-in real-provider validation that invokes Codex, Claude, and Pi review
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

- [ ] **Exact syntax for workflow contracts.** `input Name Type` / `output Name
  Type` / `failure Name Type` is implemented and working (lib.rs:16547). Open only:
  whether to *also* add a compact single-line signature form.
  **Recommendation:** keep the current keyword form as canonical, defer the compact
  form (no user demand; it is pure sugar). Low stakes.
- [ ] **Coerce file imports.** Whether coerce definitions can be imported (and how:
  `include "types.coerce"` vs a coerce declaration vs a generated bundle member).
  **Recommendation:** defer — coerce definitions live inline in `.whip` today and
  nothing forces cross-file coerce reuse yet; revisit when a real multi-file coerce
  need appears. If/when needed, `include` of a `.coerce` member is the smallest step.
- [ ] **Scalar terminal payloads.** Whether `complete result 0.9` (a bare scalar)
  is allowed, or terminal payloads must always be class-shaped (`complete result {
  score 0.9 }`). Only class-shaped is implemented. **Recommendation:** make
  class-only the deliberate v0 rule (one payload shape, uniform with `record`;
  scalars can always be a one-field class) and close this as a non-goal rather than
  leave it a lingering "remains."
- [ ] **Recursive workflow invocation policy.** Whether runtime `invoke` cycles
  (distinct from compile-time `apply`) are rejected in v0 or allowed with explicit
  policy limits. Today only *direct* self-invocation is rejected; transitive cycles
  are unanalyzed. **Recommendation:** reject transitive `invoke` cycles in v0
  (symmetry with the pattern-recursion rule; a runtime invoke cycle with no external
  boundary is the effectful-cycle hazard Design Commitment 7 forbids), with a
  documented escape only when a cycle provably crosses an external event/clock.
  Resolving this unblocks the transitive cycle-analysis + invoke-authorization items.
- [ ] **How much implicit compatibility syntax remains.** Whether the implicit
  compatibility root (a file with top-level decls and no explicit `workflow`) is
  kept as a bridge or removed now. **Recommendation (keystone):** since the corpus
  is fully migrated (37/37 explicit `workflow`) and the project is pre-release with
  no back-compat obligation, **remove the implicit root** and require an explicit
  `workflow`. That collapses the flatten-and-discard model into a clean
  one-program-many-workflows model, which is the precondition for the scoping work
  (workflow-local names, scoped resolution, bundle store schema). This is the single
  decision that unblocks the largest remaining cluster.

- [x] Whether pattern bodies may contain terminal actions: resolved. v0 forbids
  `complete`/`fail` in pattern bodies entirely (compile-time `error`); no pattern
  capability/contract escape hatch in v0.
- [x] Whether recursive *pattern application* is allowed: resolved. v0 is
  non-recursive-only (`graph.unbounded_pattern_recursion`); bounded recursion is
  deferred pending a statically-decreasing structural measure.

## Remaining Work (after the 2026-07-01 reconcile + delivery pass)

The transition is substantially shipped. What remains, grouped:

1. **Decision-gated (the real blockers).** The Open Decisions above — chiefly the
   implicit-compatibility-root / scoping keystone and the recursive-invocation
   policy. These gate: workflow-local name scoping + leak checks, scoped name
   resolution, in/out-of-workflow decl restrictions, the bundle store schema,
   transitive invoke-cycle analysis, invoke authorization, and diagnostics grouping.
2. **Model-first pieces.** Generate Maude searches from compiled IR for pattern
   provenance / terminal actions / invocation edges (kernel rules already exist;
   emit from `generate_maude_model_search`).
3. **Polish / observability (deferred-with-cause).** Recursive status-tree, JSON
   trace enrichment, source-span provenance back-links, workflow-fail-vs-provider-fail
   status field, per-file span preservation, docs canonicalization sweep.
4. **Showcase / external.** Self-hosting validation workflow; opt-in real-provider
   validation (tracked in the native-provider tracker).
