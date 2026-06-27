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
- [ ] NMIF increment — robust downgrade: an `endorse`/`declassify` may not be
  influenced by an attacker; grant-authorization of the crossing.
- [ ] TLA+ — durable label carriage (I-IFC7) + envelope versioning / non-retroactive
  (D4) + replay-stability.

## Phase 2 — Parser / IR

- [~] governance DSL grammar (`grant`/`party`/`delegate`), separate from `.whip`:
  v0 line parser landed in `ifc.rs` (`Envelope::from_dsl`), `grant <kind> <handle>
  -> <id> <label>` with `readable by` = confidential; `Envelope::load`
  auto-detects DSL vs the JSON signed-artifact. `party`/`delegate` accepted+ignored
  (party-relative content is a later slice). Unit-tested. Remaining: full grammar +
  the canonical signed-JSON emit + the `gov compile` report.
- [ ] source crossings: the `endorsed` marker (over `coerce`), the `declassify`
  construct, role references; everything else label-free.
- [ ] IR label representation (party-relative labels; the `kind:address` resource id).

## Phase 3 — Compiler IFC pass

- [~] **Slice 1 (vertical, landed 2026-06-27):** `crates/whipplescript-cli/src/ifc.rs`
  — a JSON governance envelope labels resources by confidentiality; the
  **turn-level join box** flags an agent turn granted READ on a confidential
  resource and WRITE/egress on an un-cleared one, wired into `whip check`,
  env-discovered via `WHIPPLESCRIPT_IFC_ENVELOPE` (unset = dev mode). Diagnostic
  carries routes-to-fix (separate / declassify). 3 unit + 1 end-to-end test,
  fmt+clippy clean. Scope: binary confidentiality, turn-grant granularity — the
  proof-of-architecture. Remaining within this box:
- [ ] label propagation + the opaque join box (output = join of inputs) at rule-body
  granularity (needs the store name surfaced onto IrEffectNode — a parser change).
- [ ] sticky-boundary check (governed data may not reach an un-cleared/ungoverned sink);
  party-relative labels (reader sets + acts-for) replacing binary confidentiality.
- [ ] refinement check (`inline ⊑ envelope`).
- [ ] guarantee report (guaranteed invariants + flagged risks) and routes-to-fix
  diagnostics (self-serve vs escalate).

## Phase 4 — Kernel enforcement

- [ ] envelope load + attestation (trust-root option C, reuse package-lock attestation).
- [ ] construct/boundary labeling — every env-touching construct a modeled boundary
  (the `kind:` scheme; the five non-obvious doors).
- [ ] dual-gated stores / `record`.
- [ ] envelope versioning + run binding (D4); discovery; ungoverned dev mode.

## Phase 5 — Two-agent runtime (DR-0028 D5)

- [ ] governance root agent — sudo-gated, fixed narrow surface (edit DSL / compile
  / sign), admin-only input.
- [ ] whip root agent — unprivileged, authors whips.
- [ ] escalation channel — the one whip→gov flow, carried as low-integrity data.

## Phase 6 — Provider egress + integration

- [ ] provider-as-principal egress check (incremental, brokered).
- [ ] label-driven redaction (the construction-based ergonomic levers).
- [ ] end-to-end examples (the bank+email / multi-party flows) + user-facing docs.

## Out of scope (recorded)

Quantitative information flow (leak budget) — explicitly out of scope, not deferred
(DR-0027, the I-IFC2 note). Clearance-based provider routing, role-generics,
per-field labels, decentralized ownership, multi-account governance — deferred.
