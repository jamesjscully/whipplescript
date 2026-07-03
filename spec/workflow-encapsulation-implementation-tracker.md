# Workflow Encapsulation Implementation Tracker

Status: closed (2026-07-02)

Build tracker for the v1 workflow-encapsulation + invocation-authorization theorem.
**Design SSOT is the decision record** `decision-records/workflow-encapsulation-boundary.md`
(all five v1 decisions locked 2026-07-01). All phases shipped 2026-07-02;
reality lives in code, tests, and the gates. The two items that remain open are
re-homed to `owned-harness-tool-surface.md` § Open items (web search tool;
remaining runtime-governance policy extensions) — nothing here is live work.

v1 scope was the FULL theorem (no slice split): the static encapsulation
membrane (E1/E2/E2-DYN/D2b/D3′), the dynamic authority half
(workflow-as-principal + per-instance authority slot + D1 attenuation +
`with access to`), and coordination PARTITION. Method: model-first, per-piece
review gate.

## Decisions (locked — see DR §5)

- [x] Coordination = **partition** (`<pkg>/<name>::X`, fail-closed; `shared` audited opt-in).
- [x] **Invocable-in-package by default + explicit `private` marker** (workflow opt-out of cross-boundary invocation).
- [x] **`invoke B with access to {…}` in v1** — composability primitive; additive; grant ⊆ `declared(B) ∩ effective(A)`.
- [x] Cross-workflow **cancel = operator-only for v1**; language-level cancel + supervisor DEFERRED to a dedicated design pass (§ Deferred).
- [x] **Promote workflow to `workflow:<pkg>/<name>` principal + per-instance authority slot in v1** (forced by `with access to`).

## Sequencing (cross-effort ordering — 2026-07-01 review)

Historical constraints, all satisfied or carried by their canonical homes:

- **Language-ergonomics B1g** (no-silent-no-op sweep) ran alongside Phase 0–1
  and closed 2026-07-02 (canonical home:
  `decision-records/language-ergonomics-tracker.md` B1g).
- **Ordering vs the durable-object effort:** Phase 3's coordination partition
  schema change landed BEFORE the DO tracker's store-trait extraction, and the
  E2-DYN door exists before the DO host multiplies delivery/re-entry seams.
  Mirrored in `durable-object-runtime-tracker.md`.
- **Phase 2 did not outrun Phase 4b on the CLAIM:** 4b (runtime owned-harness
  authority binding) landed with Phase 2, so docs' least-privilege claims are
  backed by enforcement.

## Phase 0 — Models first — SHIPPED 2026-07-02

All five clauses modeled before implementation, registered in
`scripts/check-formal-models.sh` (+ README), gate green:

- [x] **D1 authority attenuation** — `workflow-authority-attenuation.maude`
  (never-widen re-parameterized to `{declared(B), effective(A)}`; 3 Solution /
  3 No-solution).
- [x] **E-COORD (partition)** — `infoflow-coord-carriage.maude` (cross-workflow
  observation is a Solution under `shared`, No-solution under partition;
  4 / 4).
- [x] **D2b invoke-selector NMIF** — extended `infoflow-signal-carriage.maude`
  with `invoke:<B>` substituted for `signal:<name>` (4 Solution /
  5 No-solution).
- [x] **D3′ milestone flow signature** — milestone projection rule in
  `infoflow-field-signature.maude` (6 / 6).
- [x] **E2-DYN** — `workflow-runtime-marker-reresolution.maude` (no-laundering
  guard generalized to a runtime-resolved target identity; 3 / 3).
- [x] **Lean** — `Whipple/ReaderSets.lean`: `leak_safe` ⇏ NMIF among
  clearances, plus `same_principal_flow_trivial` (justifies the |P(R)|≤1
  skip). Gate: `scripts/check-lean-models.sh`.

## Phase 1 — Workflow-as-principal + authority slot + invoke-seam admission — SHIPPED 2026-07-02

- [x] Each workflow is a `workflow:<pkg>/<name>` principal;
  `package:<pkg>` is its acts-for ancestor. Durable instance columns
  `workflow_principal` / `effective_authority` exist for fresh and upgraded
  stores; `instances`/`status` JSON expose the slot.
- [x] Per-instance authority slot: every delegating child start persists
  `declared(B) ∩ effective(A)` (∩ start-grant when present); brokered
  package-exported `@tool` children carry the exporting package identity, and
  rule-issued children derive their package from the parent's persisted
  principal.
- [x] Invoke-seam admission: child IFC admission runs before instance creation
  on every delegating edge (Overcaution 1: delegation/cross-package only, not
  intra-envelope library decomposition).

Evidence: `cargo test -p whipplescript-store` / `-p whipplescript-kernel` /
`--bin whip` — key tests
`attenuated_authority_uses_package_ancestor_for_workflows`,
`delegating_workflow_invoke_refuses_child_ifc_violation`,
`workflow_invoke_preserves_parent_package_identity_for_child_start`,
`package_child_start_persists_package_workflow_principal`.

## Phase 2 — `with access to {…}` explicit start-grant — SHIPPED 2026-07-02

- [x] Grammar: resource-specific `invoke B { … } with access to <resource> { … } as child`
  plus the resource-less grouped shorthand
  `with access to { <resource> { <grant clauses> } ... }`; lowered to
  `IrEffectNode.access_grants`, preserved through flow expansion, carried into
  queued `workflow.invoke` input.
- [x] Enforcement: a grant narrows the child slot below the automatic cap;
  never-widen violations fail closed; no grant ⇒ automatic cap.
- [x] Reuses the turn-access-grant `profile ∩ grant` machinery at the seam.
- [x] fmt preserves invoke start grants; LSP completes the grant surface;
  `examples/least-privilege-subagent.whip`.

Evidence: `cargo test -p whipplescript-parser` +
`--bin whip start_grant_narrows_authority_below_the_automatic_cap`,
`start_grant_rejects_never_widen_violation`,
`formats_invoke_start_access_grants`.

## Phase 3 — Coordination partition (E-COORD) — SHIPPED 2026-07-02

- [x] Lease/ledger/counter rows carry an `owner` key
  (`<pkg>/<name>`), uniqueness constraints include the owner, legacy DBs
  migrate to the compatibility `shared` owner, workers route effects through
  the stored instance principal, and operator JSON exposes the owner.
- [x] `shared` opt-in: a bare `shared` field inside `lease`/`ledger`/`counter`
  declarations; runtime lowers shared coordination to the `shared` owner; IFC
  surfaces shared coordination as `resource:<name>` (outcome = read source,
  mutation = write sink), skips partitioned self-coordination, and flags
  ungoverned shared resources in the guarantee report.
- [x] Overcaution 2: the IFC check fires only on names contended by DISTINCT
  workflow principals (parser-emitted `shared_coordination_usage` metadata).

Evidence: `cargo test -p whipplescript-store
coordination_resources_are_partitioned_by_owner`,
`--test soft_middle shared_coordination_lease_opts_into_cross_workflow_contention`,
`--bin whip partitioned_coordination_is_not_a_cross_principal_ifc_source`.

## Phase 4 — Static membrane — SHIPPED 2026-07-02

- [x] **E1** — cross-package invoke needs an envelope grant on the canonical
  `invoke:<package_id>/<tool>` door (fail-closed); same-package admissible by
  default (Overcaution 3). Shipped for the current cross-package invoke
  surface (`@tool` package exports); the contract attests the door,
  `whip check` rejects ungoverned imported doors, and live owned-turn setup
  refuses to offer an ungoverned cross-package tool. Future ordinary
  package-qualified invoke syntax must reuse this door and check path.
- [x] **E2 + `private`** — source-level `@private` workflows; envelope
  `internal_workflows` set for `invoke:<name>` resources with canonical signed
  JSON round-trip; a root-independent parser pre-pass rejects sibling invokes
  to private workflows.
- [x] **E2-DYN** — the `event.notify` runtime door re-derives the target
  instance's workflow identity from stored principals and re-consults the
  marker; a rejected delivery becomes the branchable `event.notify.failed`
  fact without injecting into the target. (Per-seam discipline: every FUTURE
  deliver-by-id op routes through this check.)
- [x] **D2b** — NMIF-on-selector over the invoke inbound payload: workflow
  input roots read selector integrity from `invoke:<workflow>` (default-low
  unless vouched `grant invoke ... from <Role>`); selected arms may not drive
  a sink whose integrity dominates the selector.
- [x] **D3′** — `milestone_field_reads` metadata re-seeds `reach_reads_from`;
  milestone payloads carry the DR-0030 X2 per-field reader-set signature;
  egress gated by `leak_safe`; exposed in `flow_signature`.

Evidence: `--bin whip ifc_surface_enumerates_every_door`,
`notify_refuses_cross_package_internal_workflow_target`,
`invoke_input_selector_cannot_gate_a_higher_integrity_sink`,
`report_exposes_milestone_per_field_flow_signature`;
`-p whipplescript-parser rejects_invoking_private_sibling_workflow`;
`--test soft_middle ifc_cross_package_rejects_imported_tool_with_ungoverned_surface`.

## Phase 4b — Owned-harness runtime authority enforcement — SHIPPED 2026-07-02

The live owned harness binds tool exposure AND per-call execution to the
intersection of: `with access to` turn grants (file stores; `command { run }`
for bash; `tracker { file | claim | finish | release | update }` for tracker
mutations), the agent/profile capability policy (built-in profiles + the
runtime profile registry mapping `repo.read`/`repo.write`/`command.run`/
`tracker.*`/`workflow.invoke`), known `tell ... requires [...]` capabilities,
and — under a verified IFC envelope — governance of every named resource
(file stores, `command`, `tracker`, cross-package `invoke:<pkg>/<tool>`).
Provider configs with non-empty `profile_ids` block mismatched agent-turn
effects before launch (recoverable `provider_config` block).

Bash policy is a deliberately small, bounded surface: `command.run` capability
+ explicit turn grant + operator allow-list
(`WHIPPLESCRIPT_HARNESS_BASH_ALLOW`; empty = refuse all) + single-simple-command
syntax refusal (control operators, pipes, command/variable/glob/brace/tilde
expansion) + literal-redirection glob checks + workspace-confined path
arguments.

**History, for the record:** a command-specific bash argv classifier sweep
(per-tool operand tables across dozens of toolchains) was built here without
accepted design, then ROLLED BACK from code and docs (2026-07-02 course
correction). It is not part of the v1 surface and not a maintenance
obligation; reintroducing command-specific side-effect policy requires a
model-first design pass with an explicitly bounded surface
(`owned-harness-tool-surface.md` § Open items).

- [x] Turn grants → file/bash/tracker/workflow tool policy, live path
  (`harness_tools.rs`), deny-by-default, argument-sensitive glob checks.
- [x] Profile/registry/required-capability intersection, incl. curated `@tool`
  sub-workflow tools via `workflow.invoke`.
- [x] Governance-envelope resource coverage fails closed in governed mode;
  ungoverned dev mode unchanged.
- [~] **Remaining runtime-governance policy extensions** (envelope
  label/argument policy beyond resource coverage; future provider/tool
  capability mappings) — open DESIGN intent, re-homed to
  `owned-harness-tool-surface.md` § Open items.
- [x] **No PreToolUse-style hooks — DECIDED (Jack).** Per-tool events stay
  evidence, not rule-matchable facts; a rule reacts to the turn terminal,
  never gates a tool mid-loop.
- [x] **No blocking interactive "ask" — DECIDED (Jack): REJECTED.**
  Human-in-the-loop is the async decision-issue model (`human.ask` → inbox →
  rule).

Evidence: `--bin whip harness_tools::tests::*` (grants/profiles/bash policy/
tracker/workflow-tool suites),
`provider_profile_allowlist_blocks_mismatched_effect_profile`,
`--test control_plane dev_owned_harness_completes_turn_with_leaf_invariants`.

## Related capability — web search tool

- [~] Re-homed to `owned-harness-tool-surface.md` § Open items (its canonical
  home): shape settled (owned-harness ToolSpec, egress-checked query,
  low-integrity ingress result, `with access to`-grantable), **gated on the
  broader network-tool policy discussion** (open, per Jack).

## Phase 5 — Docs, examples, gate — SHIPPED 2026-07-02

- [x] Language-reference + spec/language.md sections for the membrane,
  `private`, `with access to`, and the coordination partition/`shared` model.
- [x] Examples: `examples/least-privilege-subagent.whip`,
  `examples/private-workflow-wrapper.whip`,
  `examples/coordination-partition-shared.whip` (all pass `whip check`).
- [x] Full release-readiness gate green after the 2026-07-02 course
  correction: parser + bin whip + control_plane + soft_middle + store +
  kernel suites, `cargo fmt`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `scripts/check-formal-models.sh`, `scripts/check-lean-models.sh`,
  `scripts/check-trackers.sh`.

## 2026-07-02 course correction (what "closed the right way" required)

The first implementation pass shipped the substance above but left the tree
dirty; the correction pass fixed:

1. **Prompt-effect scanner regression** — the new `prompt` effect keyword made
   the runtime effect scanner misparse record-field lines named `prompt`
   (fixture-table rows), enqueueing colliding effect idempotency keys
   (`UNIQUE constraint failed` at first step). Fixed by skipping record blocks
   in `parse_effect_statements` and requiring the `as` binding on the prompt
   branch; regression tests
   `parse_effect_statements_skips_record_block_fields` (unit) and the
   `dev_provider_language_*` control_plane pair (e2e).
2. **Clippy debt** — `-D warnings` failures in the new code (parser
   `if_same_then_else`, CLI closure-pattern/unwrap/borrow lints) fixed.
3. **Classifier fossils purged** — `docs/providers.md`,
   `docs/language-reference.md`, `spec/language.md`, and
   `spec/owned-harness-tool-surface.md` no longer describe the rolled-back
   argv classifier as active policy; they now state the actual bounded bash
   policy.
4. **Evidence honesty** — the prior "gate green" claim omitted the failing
   control_plane suite and clippy; the Phase 5 box above was re-checked only
   after the full gate actually passed.
5. **Pre-existing gate breakage found while verifying (not this effort's
   regression):** `check-report-schemas.sh` failed on
   `scheduled-escalation-compile-model-search` — the workflow-terminal
   composition searches (`workflow-complete`/`workflow-fail` ±
   `requires-action`, emitted since the workflow-composition pass) were never
   mirrored into the snapshot-implied expectations of the two report
   validators. Fixed in both mirrors (`IrSnapshotFacts` in `main.rs` and
   `scripts/validate-model-search-report.py`): predicate whitelist,
   `workflow_contracts` snapshot parsing, expected-row generation, and the
   supports-obligation check.

## Deferred (with cause)

- [~] **Language-level cross-workflow `cancel` + supervisor pattern.**
  Operator-only in v1 (DR §5.4). Needs its own design pass BEFORE
  implementation: a directed door gated by parent-`dominates`-child integrity,
  and it must address the cancel-TIMING covert channel (a supervisor observing
  WHEN a child cancels is a data-dependent observe channel — NMIF-on-selector
  treatment).

## Dependencies / notes

- `is_delegating_edge` + the E1 boundary predicate assume `package_of` is
  well-defined per workflow. If the one-program-many-workflows scoping model
  ever blurs package identity, the boundary predicate must be re-derived.
- Under a dev/ungoverned envelope the `internal_workflows` loader returns
  false, so encapsulation is enforced only under a verified envelope —
  consistent with all IFC.
- Every FUTURE runtime op that delivers to / mutates a target instance by id
  must route through the E2-DYN marker check (per-seam door discipline).
