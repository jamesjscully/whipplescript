# Workflow Encapsulation Implementation Tracker

Status: active tracker

Build tracker for the v1 workflow-encapsulation + invocation-authorization theorem.
**Design SSOT is the decision record** `decision-records/workflow-encapsulation-boundary.md`
(PROPOSED, all five v1 decisions locked 2026-07-01) — this tracker holds only the
OPEN BUILD INTENT; the theorem, the clause semantics, and the rationale live in the
DR and are not restated here.

v1 scope = the FULL theorem (no slice split): the static encapsulation membrane
(E1/E2/E2-DYN/D2b/D3′), the dynamic authority half (workflow-as-principal +
per-instance authority slot + D1 attenuation + `with access to`), and coordination
PARTITION. Method: model-first (Maude/Lean the clause before its code); per-piece
review gate (review + verify + docs before a box is checked).

## Decisions (locked — see DR §5)

- [x] Coordination = **partition** (`<pkg>/<name>::X`, fail-closed; `shared` audited opt-in).
- [x] **Invocable-in-package by default + explicit `private` marker** (workflow opt-out of cross-boundary invocation).
- [x] **`invoke B with access to {…}` in v1** — composability primitive; additive; grant ⊆ `declared(B) ∩ effective(A)`.
- [x] Cross-workflow **cancel = operator-only for v1**; language-level cancel + supervisor DEFERRED to a dedicated design pass (§ Deferred).
- [x] **Promote workflow to `workflow:<pkg>/<name>` principal + per-instance authority slot in v1** (forced by `with access to`).

## Phase 0 — Models first (before any implementation)

Each clause is a re-parameterization/extension of an already-proven or bitten primitive.

- [ ] **D1 authority attenuation** — re-parameterize `turn-access-grant.maude`'s
  never-widen (`profile ∩ grant`) to `{declared(B), effective(A)}`; bite: a grant
  naming an authority ∉ the cap is No-solution (never-widen).
- [ ] **E-COORD (partition)** — new `infoflow-coord-carriage.maude`: reuse the signal
  `[inject]` gate + an `[outcome-read]` rule. Negative bite MUST show A's `consume`
  reaching B's observed `remaining`/`holders` is a Solution under `shared` and
  **No-solution under partition** (the fork made mechanical).
- [ ] **D2b invoke-selector NMIF** — extend the shipped H8 selector check in
  `infoflow-signal-carriage.maude` with `invoke:<B>` / the invoke inbound payload
  substituted for `signal:<name>`; bite: a low-integrity input-rooted selector gating
  a higher-integrity branch is rejected.
- [ ] **D3′ milestone flow signature** — `infoflow-field-signature.maude` milestone
  projection rule identical to the complete-result egress rule; the milestone-reached
  occurrence rides the reached-fact read-source.
- [ ] **E2-DYN** — model the runtime marker re-resolution as the no-laundering guard
  generalized from a static name to a runtime-resolved target instance identity.
- [ ] **Lean** — `ReaderSets.lean` lemma: two clearances both `dominates` a label do
  NOT satisfy NMIF w.r.t. a shared cell (`leak_safe` ⇏ NMIF among clearances), plus
  `same_principal_flow_trivial` (self-coordination ⇒ reflexive, justifying the
  |P(R)|≤1 skip). D2b/D3′/E2-DYN reuse existing NMIF/`flow_conf_trans`/no-laundering
  statements source-agnostically (no new lattice).
- [ ] Register every new/changed model in `scripts/check-formal-models.sh` (+ README);
  full formal-models gate green.

## Phase 1 — Workflow-as-principal + authority slot + invoke-seam admission (largest new runtime piece)

- [ ] Promote each workflow to a `workflow:<pkg>/<name>` principal in the acts-for lattice.
- [ ] Per-instance authority slot: `start_child_workflow_instance` (main.rs:~25902,
  attaches no principal today) sets the child slot to `declared(B) ∩ effective(A)`.
- [ ] Enforce at the invoke seam via `ifc_admission` (main.rs:~26214) lifted from the
  root path to every **delegating** invoke edge (Overcaution 1: only on delegation /
  cross-package, not intra-envelope library decomposition).

## Phase 2 — `with access to {…}` explicit start-grant (D1 explicit narrowing)

- [ ] Grammar: `invoke B { … } with access to { <capability grants> } as binding`.
- [ ] Desugar/enforce: the grant narrows the child slot below the automatic cap; the
  named grant must be ⊆ `declared(B) ∩ effective(A)` else a never-widen error.
- [ ] Reuse the turn-access-grant `profile ∩ grant` machinery at the seam.
- [ ] fmt/LSP surface; examples showing least-privilege subagent composition.

## Phase 3 — Coordination partition (E-COORD)

- [ ] Partition coordination rows by owner (`<pkg>/<name>::X`) — the E3
  `UNIQUE(instance_id,…)` isolation lifted to the coordination store (schema change).
- [ ] `shared` opt-in surface: an explicit, audited per-resource declaration that
  accepts the co-tenant channel; under `shared`, the outbound `leaks` check (E-COORD
  part b) still gates the set boundary.
- [ ] Overcaution 2: the check fires only on names contended by DISTINCT
  workflow-principals (self-coordination is unlabeled).

## Phase 4 — Static membrane

- [ ] **E1** — cross-*package* invoke needs an envelope grant (fail-closed); same-package
  admissible by default (Overcaution 3, matches lib.rs:~9257).
- [ ] **E2 + `private` marker** — attested `internal_workflows` marker (sibling of
  `internal_signals`); `private` workflow unnameable as a cross-boundary invoke target.
  (Open sub-nit: surface keyword `private` for workflows vs `internal` for signals —
  align or keep split.)
- [ ] **E2-DYN** — the runtime delivery door (`run_notify_effect` main.rs:~23497,
  invoke-by-handle) re-derives the target instance's workflow identity
  (instance→version_id→program_name) and re-consults the marker; refuse fail-closed
  cross-boundary regardless of how the instance id was obtained.
- [ ] **D2b** — compile-time NMIF-on-selector over the invoke inbound payload (extends
  the shipped signal selector in ifc.rs).
- [ ] **D3′** — `milestone_field_reads` metadata re-seeds `reach_reads_from`; milestone
  payloads carry the same DR-0030 X2 per-field reader-set signature as `complete
  result`; egress gated by `leak_safe`; expose in `flow_signature`.

## Phase 4b — Owned-harness runtime authority enforcement + agent-surface decisions

From the 2026-07-01 v1 surface-hardening design pass. The `with access to` /
capability model is enforced only STATICALLY today; the owned-harness runtime
does not bind tool calls to declared authority (`with_policy`/`with_bash_allow`
are `#[allow(dead_code)]`, test-only — `harness_tools.rs:302-331`; the live
`run_owned_agent_turn` reads bash/file policy from process env, not the decl).

- [ ] **Finish runtime enforcement of declared authority in the owned harness.**
  The owned-harness tool executor's per-tool-call policy = the **governance
  envelope** (IT-set, `gov.rs`) ∩ the agent's declared authority (`with access
  to` / `capabilities`), replacing the env-only path. This makes `with access to`
  actually bind runtime behavior and yields **per-tool-call, argument-sensitive**
  governance (a static grant says "may run bash"; only a runtime gate says "may
  run *this* bash command"). Author boilerplate stays minimal — concrete policy
  (allow-lists, path globs, provider access) lives in the **governance layer
  (IT)**; the author's whole surface is `with access to` (scope a subagent) +
  the agent decl's `capabilities`/`profile`. Enumerate any additional concrete
  author-facing per-turn case before adding surface; none known beyond `with
  access to` today.
- [x] **No PreToolUse-style hooks — DECIDED (Jack): keep the whip model.** Per-tool
  events stay evidence, not rule-matchable facts (turn-is-leaf / I2,
  `harness_loop.rs:14-18`); a rule reacts to the turn terminal, never gates a tool
  mid-loop. Governance is declared-policy (above) + post-turn reactive rules —
  stronger than Claude-Code hooks and deliberately not that shape.
- [x] **No blocking interactive "ask" — DECIDED (Jack): REJECTED.** Human-in-the-loop
  is the **async decision-issue** model (`human.ask` → inbox → a rule reacts when
  the answer lands, `main.rs:23013`); the agent NEVER blocks mid-turn waiting on a
  person — the software adjudicates the decision issue on its own schedule.

## Related capability — web search tool (item ①, gated on the network-tool discussion)

- [ ] **Web search as an owned-harness tool.** A new `ToolSpec` alongside
  `file_tool_specs()` (`harness_tools.rs:99`) with a `ToolExecutor`, kernel-brokered.
  **Owned-harness only** (command-backed Claude/Codex use their native web search).
  IFC: the query is an **egress** (flow-checked like `send`), the result is a
  **low-integrity ingress** (taint source, like an inbound message). Made a
  **capability grantable via `with access to`** — the first real customer of the
  authority model (a subagent gets web search only if delegated it). Settled in
  shape; **gated on the broader network-tool policy discussion** (open, per Jack).

## Phase 5 — Docs, examples, gate

- [ ] Language-reference + spec/language.md sections for the membrane, `private`,
  `with access to`, and the coordination partition/`shared` model.
- [ ] Examples: least-privilege subagent composition (`with access to`); a `private`
  library-internal workflow; a partitioned vs `shared` coordination resource.
- [ ] Full release-readiness gate green.

## Deferred (with cause)

- [~] **Language-level cross-workflow `cancel` + supervisor pattern.** Operator-only in
  v1 (DR §5.4). Needs its own design pass BEFORE implementation: a directed door gated
  by parent-`dominates`-child integrity, and it must address the cancel-TIMING covert
  channel (a supervisor observing WHEN a child cancels is a data-dependent observe
  channel of the same class as the coordination Held/Contended and milestone-reached
  discriminants — NMIF-on-selector treatment). Track that design pass as its own item.

## Dependencies / notes

- `is_delegating_edge` + the E1 boundary predicate assume `package_of` is well-defined
  per workflow. Confirm against the workflow-composition scoping model
  (whole-program validation shipped; one-program-many-workflows package identity).
  See `workflow-composition-transition-tracker.md`.
- Under a dev/ungoverned envelope the `internal_workflows` loader returns false, so
  encapsulation is enforced only under a verified envelope — consistent with all IFC.
- Every FUTURE runtime op that delivers to / mutates a target instance by id must
  route through the E2-DYN marker check (per-seam door discipline).
