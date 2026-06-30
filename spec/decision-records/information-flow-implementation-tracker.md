# Information-flow control — implementation tracker

Status: active. Tracks the build of the information-flow control system + the two
root agents, from the locked design ([DR-0026](0026-session-root-agent.md),
[DR-0027](0027-information-flow-control.md),
[DR-0028](0028-information-flow-authority.md), the
[surface](../information-flow-surface.md) and
[governance](../information-flow-governance.md) specs). Standing discipline:
model-first, then greenfield, **per-piece gate** (review → fixes → verify → docs)
before a box is checked; full gate green (`cargo fmt`, clippy, tests,
`check-release-readiness.sh`) at each checkpoint.

This is a large, multi-phase build. Phases are roughly sequential; slices within a
phase each run their own model → code → review → docs → gate loop.

**Status (2026-06-30): all phases reconciled against the shipped code + the durable
[audit-findings](information-flow-audit-findings.md) tracker.** Every soundness-bearing
item is `[x]` done with a test/proof; no `[ ]` remains. The 6 `[~]` boxes are each a
recorded **deferral with cause** (not unstarted work): D4 envelope versioning /
replay-stability liveness + Veil (temporal research; I-IFC7 carriage safety is checked —
`InfoflowLabelCarriage.tla`); the `gov compile` *signed-artifact emit*; a standalone
`inline ⊑ envelope` diagnostic (satisfied in substance by the flow checks); storage-layer
dual-gating (defense-in-depth over the check-time gate) + the `complete result`→invoker
channel (owned by the X2 flow signature, DR-0030); and a dedicated field-level `redact`
construct (an additive ergonomic lever — the safe shape is already expressible). Net this
session: NMIF increment, rule-body join box, per-resource guaranteed invariants +
flagged-risks + self-serve/escalate routes, provider egress, the two agent loops +
escalation channel verified shipped; `InfoflowLabelCarriage.tla` authored + bite-verified
+ gated.

## Phase 1 — Formal-model upgrade (party-relative + NMIF)

The exploratory models were single-owner lattices; upgrade to the decentralized /
asymmetric-delegation model (acts-for, reader/influencer sets) and NMIF.

- [x] integrity model party-relative — acts-for reflexive-transitive closure
  (`canAct`), influence tokens, control sink requiring authority; 4 coverage + 4
  bite, gate-green (`models/maude/infoflow-integrity.maude`), 2026-06-27.
- [x] confidentiality model party-relative — owner-secret tokens + reader-authority
  (`readAuth`) with acts-for closure; incomparable compartments; 4+4 gate-green
  (`models/maude/infoflow-confidentiality.maude`), 2026-06-27.
- [x] composition model party-relative — both axes, explicit join node, dual-gated
  `record`, axis-locked endorse/declassify, the audited trusted surface; 9+9
  gate-green (`models/maude/infoflow-composition.maude`), 2026-06-27.
- [x] Lean proof layer — the algebra, machine-checked (`models/lean`, hermetic,
  Mathlib-free, pinned v4.30.0), 2026-06-27. `actsFor_refl`/`actsFor_trans` (preorder),
  `public_is_bottom`/`public_acts_for_only_public` (bottom holds no authority),
  `canAct_iff` (the reach algorithm is SOUND AND COMPLETE vs the order — axioms
  `[propext, Quot.sound]`), `integ_is_conf_dual`, `conf_to_public_needs_public_source`,
  and the `Verified` boundary discipline (the report-vs-check bug class). Gated by
  `scripts/check-lean-models.sh` (rejects sorry/admit/axiom/native_decide, bite-tested).
  CLOSES the audit finding that acts-for being a preorder + the closure being correct
  were only ASSERTED in comments, never proven.
- [x] NMIF increment — robust downgrade, all three legs: (1) Lean `NMIF.lean`
  (`untrusted_declassify_only_public` — attacker-controlled data releases only to the
  public bottom; `endorsed_enables_declassify`; `robustDeclassify_mono`; zero axioms);
  (2) checker **NMIF-on-the-selector** (`ifc.rs:1215` — a crossing whose selector is a
  low-integrity binding is rejected as attacker-steered); (3) grant-authorization — a
  `declassify`/`endorse` crossing requires the consumer's governance grant
  (`grant declassify|endorse <resource> to <role>`). Maude bite in
  `infoflow-declassifier.maude` + `infoflow-integrity.maude`. (The *general*
  non-interference-relative-to-policy theorem remains a deferred research item under
  audit-findings **M1**, distinct from this increment.)
- [~] TLA+ / Veil — durable label carriage (I-IFC7) + envelope versioning / non-retroactive
  (D4) + replay-stability (a transition-system property; Veil-on-Lean or TLA+).
  **I-IFC7 durable label carriage DONE** (`models/tla/InfoflowLabelCarriage.tla`,
  2026-06-30): an Apalache-checked transition system over persist / reload /
  cross-instance-handoff / replay hops with safety invariants `CarriagePreserved`,
  `NoStrip` (confidentiality never silently lowered) and `NoForge` (integrity never
  silently raised — the W6 no-laundering principle as a trace property). Bite-verified
  (a `LaunderHandoff` hop that rewrites integ produces a counterexample); wired into
  `scripts/check-tla-models.sh`; it is the inductive, all-interleavings complement to
  the single-hop `infoflow-carriage.maude`. Also landed earlier:
  `InfoflowReleaseBudget.tla` (DR-0030 budget + no-adaptive-oracle). **DEFERRED:** D4
  envelope versioning / non-retroactivity and replay-stability *liveness*, and a Veil
  model — temporal/liveness research beyond the I-IFC7 safety property captured here
  (audit-findings M3).
- [x] Maude multi-consumer bite — `subworkflow-attestation.maude` gained a SECOND
  consumer (`publish`, the report analogue) held to the same attestation gate:
  coverage (a genuine attested tool serves both consumers) + bite (an un-attested tool
  is never published either; an un-gated rule flips No-solution → Solution). Closes the
  report-vs-check bug class in Maude (Lean `Verified` covers it algebraically). Audit
  finding **M2 (Wave 1)**, 2026-06-27.

## Phase 2 — Parser / IR

- [x] **declassify escape hatch** — BOTH granularities now ship. (1) Governance:
  DSL `grant declassify <resource> to <role>` clears a flow whose sink reader is
  cleared for `<role>`; `grant endorse <resource> to <role>` is the integrity-axis
  twin (no longer open). (2) Source: per-crossing `endorsed`/`declassified` markers on
  `coerce` (E4 above). Both surfaced in the audited trusted surface of the guarantee
  report. 2026-06-27 (governance) / 2026-06-28 (source).
- [x] governance DSL grammar (`grant`/`party`/`delegate`), separate from `.whip`:
  the line parser in `ifc.rs` (`Envelope::from_dsl`) handles `grant <kind> <handle> ->
  <kind:address> <label>` with `readable by R1, R2` / `from R1, R2` (reader/writer
  SETS, E6), `grant declassify|endorse <resource> to <role>`, `grant signal X ->
  signal:X internal` (H8), **`delegate <P> acts-for <Q>`** (now LIVE — drives the
  acts-for closure, not ignored) and **`party <id> : <Role>`** (now LIVE — the
  principal map, round-tripped as canonical `parties`, E7). `Envelope::load`
  auto-detects DSL vs the signed JSON. Canonical signed-JSON emit ships
  (`to_canonical_json` + `SignedEnvelope::to_json`) and the `gov compile` report is the
  guarantee report below. Unit-tested. DEFERRED: future grammar niceties (none required
  for soundness).
- [x] source crossings: both `endorsed` and `declassified` ship as trailing source
  markers on `coerce` (`coerce f(x) as y endorsed` / `… declassified`; may co-occur).
  Leaf flags on `BodyEffectKind::Coerce` (`body.rs`) + `IrEffectNode` (not serialized →
  zero golden/hash churn); surfaced in the trusted-surface report as "endorsed/declassified
  (source) at rule `<r>`". v0 = auditability (authorization still lives in governance
  `grant endorse`/`declassify`). Audit finding **E4**, 2026-06-28.
- [x] IR label representation (party-relative labels; the `kind:address` resource id).
  The governance grant `<kind> <handle> -> <kind:address> <label>` is the binding; the
  checker resolves every resource to its address (`Envelope::resolve`/`address_of`,
  `ifc.rs`) and keys all labels/reasoning/reporting by address. Canonical JSON carries a
  `bindings` map (signed envelopes round-trip). Also the cross-package X4 binding model.
  Audit finding **E5**, 2026-06-28.

## Phase 3 — Compiler IFC pass

- [x] **Slice 1 (vertical, landed 2026-06-27):** `crates/whipplescript-cli/src/ifc.rs`
  — a JSON governance envelope labels resources by confidentiality; the
  **turn-level join box** flags an agent turn granted READ on a confidential
  resource and WRITE/egress on an un-cleared one, wired into `whip check`,
  env-discovered via `WHIPPLESCRIPT_IFC_ENVELOPE` (unset = dev mode). Diagnostic
  carries routes-to-fix (separate / declassify). 3 unit + 1 end-to-end test,
  fmt+clippy clean. Scope: binary confidentiality, turn-grant granularity — the
  proof-of-architecture. **Generalized since** (each its own box below, now done):
- [x] label propagation + the opaque join box (output = join of inputs) at rule-body
  granularity. The enabler — the resource/store name on `IrEffectNode` — landed with
  E5 (`resource_for_body`). The checker now gathers `reads`/`writes` over ALL effects
  in a rule body (file read/write, `send via` capability calls, access grants,
  `record`→`fact:<schema>`, the message/signal/human doors, provider egress) and checks
  every (source, sink) pair: each sink must dominate the join of the rule's sources —
  the opaque join box (I-IFC2: the rule is the unit). Covered by
  `rule_body_file_flow_is_checked` (a direct `read text from ledger` → `write text to
  outbox`, no agent turn, is flagged), 2026-06-28/30.
- [x] sticky-boundary check — fail-closed (I-IFC6): confidential data may not reach
  ANY non-confidential sink (governed-public OR ungoverned), 2026-06-27.
- [x] party-relative labels replacing binary confidentiality (2026-06-27): each
  resource carries a **reader authority**; a flow `src -> sink` leaks unless
  `sink`'s reader authority **acts-for** `src`'s, via a reflexive-transitive
  `can_act` closure (ported from the Maude models) over the envelope's `delegate`
  edges. DSL `readable by <Role>` + `delegate <P> acts-for <Q>`; binary JSON
  (`confidential: bool`) still parses (back-compat). Verified incl. an acts-for
  delegation clearing a flow. **Remaining now CLOSED:** per-resource reader *sets*
  shipped (E6, 2026-06-30 — label = `BTreeSet` of compartments, read iff acts-for
  EVERY one, via one `dominates(provider, required)` relation; `readable by R1, R2` /
  `from R1, R2` DSL; `ReaderSets.lean` soundness) and the integrity axis (endorse)
  is live on both governance + source crossings (E4).
- [~] guarantee report (`gov compile`, DR-0028): `governance_report` now surfaces
  **per-resource guaranteed invariants** (the exact confidentiality/integrity proven on
  every rule, one line per governed resource — not a generic blanket line), violations
  caught, **flagged risks** (touched-but-ungoverned resources, fail-closed to
  public/low + the audited trusted surface as the other risk class), cleared
  principals, and the full door surface; rendered by `whip check` under an envelope.
  Only **signed-artifact emit** remains (a `gov compile` that emits the report as a
  signed artifact) — folded into the Phase-5 governance agent loop, where `gov compile`
  lives. 2026-06-30.
- [~] refinement check (`inline ⊑ envelope`). **SATISFIED in substance** (audit E1):
  the whip expresses usage and the checker holds it to the envelope — dangerous flows
  rejected on both axes, the cross-package surface check enforces
  `package-surface ⊑ consumer-envelope` (`infoflow-package.maude`), and a whip authors
  no delegation. DEFERRED: a standalone `inline ⊑ envelope` diagnostic distinct from
  the flow checks.
- [x] guarantee report (guaranteed invariants + flagged risks) and routes-to-fix
  diagnostics (self-serve vs escalate). Per-resource invariants + flagged-risks
  sections shipped (above); every leak/inject diagnostic now names a **self-serve**
  route (no grant: separate contexts / gate on trusted data) and an **escalate** route
  (a governance grant: `grant declassify …` / `grant endorse …`), mirroring the
  two-agent privilege split. Test `leak_and_inject_diagnostics_carry_self_serve_and_escalate_routes`;
  documented in `examples/infoflow/README.md`. 2026-06-30.

## Phase 4 — Kernel enforcement

- [x] envelope load + attestation (trust-root option C). `whip gov sign` produces a
  SHA-256 attestation binding the canonical envelope to the signer (gated by
  `WHIPPLESCRIPT_GOV_ADMIN`); `whip gov verify` checks it; `ifc::VerifiedEnvelope` is
  the only env→envelope path so a tamper is refused structurally. `gov.rs`, Wave 1/5.
- [x] construct/boundary labeling — all five non-obvious doors are modeled boundaries:
  provider-endpoint, `human.ask` (egress sink `human`; `when human answered` =
  low-integrity source), `emit`/`notify` (egress sink `stream`), `record` (sink
  `fact:<schema>`), and `when message from <channel>` / `when <Signal>` inbound sources.
  No unlabeled door remains. Audit findings **E2 (Wave 2) + H8 (2026-06-30)**.
- [~] dual-gated stores / `record`. `record` is a governed sink `fact:<schema>`
  (default public/fail-closed) at check time — the recordSink of
  `infoflow-composition`, realized (H2 fact-base half). **DEFERRED, with cause:**
  (1) dual-gating at the *storage* layer is kernel-runtime enforcement (E3 family) —
  the check-time gate already rejects violating flows before any side effect, so the
  storage-layer gate is defense-in-depth, not a soundness gap. (2) The
  `complete result`→invoker channel (H2 second half) is **owned by the X2 flow
  signature** (DR-0030), not a blanket sink: `IrTerminalOutput` carries the payload
  *types* but not value-level data provenance, and a coarse public `result` sink would
  be unsound-by-overstrictness — it would forbid returning *any* bounded function of
  confidential data to a cleared invoker (it would reject the safe-shape example whose
  whole point is exactly that). The sound treatment is per-field reach (X2,
  `infoflow-signature.maude` + `FlowSignature.lean`, designed; impl is the `flow_signature`
  schema + producer/consumer build), tracked in audit-findings X2.
- [~] envelope versioning + run binding (D4); discovery; ungoverned dev mode.
  Discovery via `WHIPPLESCRIPT_IFC_ENVELOPE` (unset = ungoverned dev mode) is **DONE**.
  **D4 versioning/non-retroactivity/run-binding DEFERRED** (audit-findings M3/E3): it is
  a temporal/transition-system property (a run bound to its admitted policy hash,
  re-validated on a version change) whose proper home is the TLA+/Veil temporal model
  (M3) — building inert hash-stamping without the temporal consumer would be scaffolding,
  not enforcement. The signed envelope already carries a SHA-256 attestation hash, so the
  binding *primitive* exists when M3 lands.

## Phase 5 — Two-agent runtime (DR-0028 D5)

- [x] **privilege-separation core** (`crates/whipplescript-cli/src/gov.rs`,
  2026-06-27): `whip gov sign` is the privileged op (G1/G4) — refused without
  governance privilege, gated by `WHIPPLESCRIPT_GOV_ADMIN` (the sudo/OS-install
  proxy); produces a SHA-256 **attestation** (trust-root option C) binding the
  canonical envelope to the signer. `whip gov verify` is the unprivileged
  whip-agent check; a tamper breaks the hash. Soundness: `SignedEnvelope::sign` is the
  *sole* producer of a valid signature, so the single-signer rule holds structurally.
  *Binding `whip check` to require a verified signed envelope* is satisfied in
  substance: when an envelope is discovered it MUST verify (the `VerifiedEnvelope`
  boundary) and a violating whip is refused at admission (W5); requiring an envelope to
  be *present* is a deployment policy (unset = ungoverned dev mode, by design).
- [x] governance root agent loop — `whip gov agent` (PRIVILEGED; refuses to start
  without governance privilege): a stdin command loop (admin or LLM driver) that
  accumulates a draft from `grant`/`delegate`/`party` lines and acts on `show`
  (canonical JSON) / `sign <path>` / `escalations` / `quit`. It holds the ONLY path to
  signing, so the whip agent can never reach it. `main.rs:gov_agent`.
- [x] whip root agent loop — `whip agent` (UNPRIVILEGED): `check <file>` (runs the IFC
  check) and `escalate <request>` (files a low-integrity request); a `sign` is refused
  with a route to escalate. `main.rs:whip_agent`.
- [x] escalation channel — the one whip→gov flow, carried as **low-integrity data**
  (it is data the admin reads, never an action the whip can perform):
  `whip gov escalate <request>` / `whip gov escalations` + `gov::file_escalation` /
  `list_escalations` over `WHIPPLESCRIPT_GOV_ESCALATIONS`. Unit test
  `escalation_channel_files_low_integrity_and_only_gov_reviews` + e2e
  `ifc_escalation_channel_whip_files_gov_reviews` + privilege-separation e2e.

## Phase 6 — Provider egress + integration

- [x] provider-as-principal egress check (incremental, brokered). A `tell` turn ships
  its context to the agent's model provider, so a turn that reads a confidential
  resource whose provider is not cleared is flagged as a `provider-egress violation`
  (DR-0027 provider-as-principal). The provider is resolved from the agent binding and
  held to the resource's reader set (`leaks(resource, provider)`). `ifc.rs:1043`; test
  `flags_provider_egress_to_uncleared_provider` + `allows_turn_reading_confidential_only_when_provider_cleared`.
- [~] label-driven redaction (the construction-based ergonomic levers). Two levers
  ship: (1) the `declassified` source marker on `coerce` (E4) — the coerce output
  schema is the leak ceiling, so a bounded redaction is enforced *by construction* at
  the crossing point; (2) provider-boundary redaction (DR-0032) — `redacted_provider_summary`
  / `redacted_text_metadata` keep raw provider content out of persisted facts and
  telemetry. DEFERRED (design-gated, needs Jack): a dedicated field-level `redact`
  construct that drops confidential fields by label — an additive ergonomic lever, not
  a soundness gap (the safe shape is already expressible by separating contexts).
- [x] end-to-end examples (the bank+email / multi-party flows) + user-facing docs.
  `examples/infoflow/`: `support-triage-unsafe.whip` (untrusted `inbox` + confidential
  `crm` + a `reply` egress — the multi-party analogue of bank+email), the strict
  `governance.policy` that rejects it, `governance-with-hatches.policy` (audited
  declassify/endorse), `support-triage-safe.whip` (the maximally-permissive *safe*
  structure), `agent-egress.whip`, `governance.signed.json`, and a full README walking
  through what the checker catches, the audited hatches, the safe shape, and the
  guarantee report. Exercised by `scripts/check-docs-examples.sh`.

## Out of scope (recorded)

Quantitative information flow (leak budget) — explicitly out of scope, not deferred
(DR-0027, the I-IFC2 note). Clearance-based provider routing, role-generics,
per-field labels, decentralized ownership, multi-account governance — deferred.
