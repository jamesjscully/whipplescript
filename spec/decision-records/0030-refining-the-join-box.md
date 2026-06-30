# DR-0030 — Refining the opaque join box

Status: accepted direction (2026-06-28). Direction **A** (per-tool flow signatures)
is decided and formally modeled; directions **B** and **C** are decided *in
principle* and staged. Refines DR-0027 (I-IFC2 join box, I-IFC3 crossings) and
DR-0029 (X2 per-tool flow signature). Formal model:
`models/maude/infoflow-signature.maude` (in the `scripts/check-formal-models.sh`
gate). Durable tracker: `spec/decision-records/information-flow-audit-findings.md`.

## Problem

The agent turn (and, lifted to packages, an exported `@tool`) is an **opaque join
box** (DR-0027 I-IFC2): an output carries the join of *every* input label. This is
sound but coarse — it collapses "the output depends on *some part* of a
secret-bearing context" into "the output is secret." The canonical pain case is
*read-secret, emit-benign*: a turn reads the whole ledger and emits "looks fine,"
which the join box labels secret because it touched the secret.

DR-0027 already rejected quantitative information flow (QIF / entropy budgets) "out
of scope, not deferred," and DR-0029 fixed X2 at the join box with a reserved,
compiler-verified extension point ("ideas brewing; do not design it in yet"). This
record **takes up that extension** and states the doctrine for *how* the join box
may be refined.

## Decision 0 — The doctrine: refine on the LABEL axis, never the quantity axis

The join box's defect is **granularity, not lack of quantification.** Every sound,
useful generalization recovers *which* secret flowed to *which* party via *which*
port or phase — finer **labels and provenance** — never a *measure* of how much
leaked.

**Why entropy is the wrong tool (sharper than DR-0027's delicacy argument).** Bits
are not the unit the threat model cares about. A confidentiality policy is never
"leak ≤ N bits of the ledger"; it is "party P must not learn *this fact*." Facts are
not fungible with bits:

- "Is the patient HIV-positive?" is **one bit** and is the entire violation.
- A 256-bit public content hash is **256 bits** and is harmless.

An entropy budget treats those as 1 vs 256 units of one currency, so even a
*perfectly correct* bit-counter measures the wrong quantity — it waves through the
catastrophic 1-bit leak and flags the harmless hash. The thing that varies is
*which* bits, not *how many*. (DR-0027's delicacy points — adaptive adversaries,
channel composition, "a subtly-wrong bound is worse than an honest binary one" —
stand, but this fungibility objection is the primary one and is now recorded as
such.)

**The one dial under everything.** Every refinement below collapses to *declared-
and-one-shot* vs *automatic-and-repeatable*. A declared, authored, audited
mechanism is sound because composition is bounded by what the author wrote down
(the audit set enumerates the leaks). Anything automatic and repeatable is an
**oracle**, and oracles defeat both bit-counting and predicate-checking. The QIF
crowd is right that *composition is the whole game*; their error is choosing bits
as the unit. If a pervasive automatic relaxation is ever wanted, the honest unit is
**queries against an oracle** (a per-(secret, party) query/privacy budget), not
Shannon bits of the value.

## Direction A — Per-tool flow signatures (DECIDED + MODELED)

The X2 refinement: an exported tool's result need not carry the join of *all*
inputs; it carries the join of the inputs it **provably depends on**.

### A.1 The producer attests a STRUCTURAL matrix; reader/writer sets are derived

Reader/writer sets are **party-relative** (I-IFC1): "readable by {underwriter,
auditor}" names *consumer* principals. A reusable package author does not know the
consumer's principals, so **the producer cannot attest reader/writer sets** — they
are not even well-defined at package-build time. What the producer *can* prove is
purely **structural and label-agnostic**: which input ports flow to which output
ports (the dependency matrix). The matrix and the reader/writer sets are the
producer side and the consumer side of *one fact*, not competing shapes:

- **Producer attests the matrix** (no principals — only the tool's own ports). It
  is the native output of the Wave-3 port reach analysis (generalize resource-level
  reach to port-level). The consumer trusts it via the Verified boundary (X5),
  exactly like convergence attestation — it does not re-derive.
- **Consumer derives reader/writer sets** by applying the matrix to *its own* bound
  input labels: `readers(output) = ⋂ { readers(i) : i ∈ deps(output) }`
  (confidentiality); `writers(output) = meet { writers(i) : i ∈ deps(output) }`
  (integrity). One relation yields both axes.

The join box is the degenerate matrix where every output depends on every input. A
refinement only ever *removes* edges, and every removed edge is a proof obligation
(a non-interference claim).

### A.2 Granularity: whole-result v1, per-field v2

- **v1 = whole-result**: one dependency set for the entire tool result (a vector,
  not yet a matrix). Provable with existing port-level reach, **no value-flow
  tracking**. The integrity win stands alone ("does this decision depend on the
  low-integrity inbound note, or only on the high-integrity policy?"); the
  confidentiality win is rarer at whole-result but real.
- **v2 = per-output-field**: needs Direction B's value-flow tracking to attribute
  fields; the full matrix.

The contract is **keyed per-output-port from day one** (a map; v1 uses a single
`result` key) so v2 is non-breaking.

### A.3 Serialize DEPARTURES from the join box (fail-closed)

The signature lists *departures* from the default `direct` edge, never the kept
edges. Listing kept edges (`depends_on`) is unsound: a dropped/garbled contract
entry would silently make an edge `absent` (an unproven independence claim).
Listing departures means a dropped entry reverts the edge to `direct` (full
join-box label) — conservative, fail-closed. **Modeling the conditional case
(A.4) first is what surfaced this**; it would have been locked wrong had v1
shipped `depends_on`.

```jsonc
"flow": { "enum": ["join_box", "signature"] },   // absent / join_box = every output ⟵ every input
"flow_signature": {                               // present iff flow == "signature"
  "<output port>": {                              // v1: single key "result"
    "independent_of": ["<input port>", ...],      // proven non-interference (v1 uses this)
    "mediated":      { "<input port>": "<crossing-id>" }  // v2 conditional; unlisted = direct
  }
}
```

Validation: `independent_of` / `mediated` keys ∈ the input-port universe pinned by
X1 `surface` ∪ `resource_params`; output keys ∈ the tool's result ports; every
`mediated` crossing id ∈ `required_crossings` (X3).

**This shape is now in the schema** (`spec/report-schemas/package_contract_v0.schema.json`,
`information_flow.flow_signature`), as the additive, non-breaking reserved extension:
the `flow` enum gains `"signature"`, `flow_signature` is an optional per-output-port
map, and a conditional enforces `flow == "signature" ⇔ flow_signature present`.
**Only the schema (a spec artifact) changed — no Rust.** The producer at
`crates/whipplescript-cli/src/main.rs:3649` still emits `flow: "join_box"`, so every
emitted contract keeps validating (verified directly with `jsonschema`); computing and
emitting the matrix is the build step. B adds nothing to the contract (it is
intra-package value-flow). C needs nothing beyond the existing `required_crossings`
(a predicate-gated crossing and its release budget are consumer-side governance, not
contract fields, and C is staged).

### A.4 The mediated edge is a CONDITIONAL DISCOUNT (the riskiest part, modeled first)

A per-edge state is one of three:

- **direct** — result carries the input's full label (the default; unlisted).
- **absent** — input provably does not flow (a non-interference claim the producer
  proves on its internals). v1.
- **mediated(C)** — the input flows to the result *only through* an internal
  crossing C (declassify/endorse). A **conditional discount**, valid only if the
  consumer **grants C** (X3); the result then carries C's bounded **target** label,
  not the input's. v2.

Three properties carry all the risk and are pinned by the model:

1. **Mediation is a CUT, not mere reachance.** `mediated(i, C)` is sound only if
   *every* internal flow path from `i` to the result traverses `C`. If any path
   bypasses `C`, `i`'s full label leaks — a bypass is rejected at producer
   attestation.
2. **Fail-closed fallback is to `direct`, never `absent`.** If the consumer does
   **not** grant `C`, the mediated edge reverts to `direct` (full label) — *not* to
   absent. Treating ungranted-mediated as absent is the catastrophic bug.
3. **A granted crossing is not a blank check.** Even granted, the result carries
   the crossing's **target/ceiling** label; if that capped label still exceeds the
   consumer envelope, reject. Plus NMIF (I-IFC3): a crossing whose selector is
   attacker-influenced is rejected at attestation.

**Crossings stay orthogonal to the pure-dataflow matrix in v1.** v1's matrix is
pure dataflow; internal crossings remain declared in `required_crossings` (X3) and
are handled unchanged. The conditional discount (a matrix edge contingent on a
granted crossing) is **v2** — it was *modeled* now (per the decision to de-risk the
riskiest shape before upstream lock-in) but is *built* in v2.

### A.5 Two-sided gate (refines infoflow-package)

- **Producer** attests a signature **only when every declared edge is justified**
  by the internal check: `absent(i)` requires `noReach(i)`; `mediated(i, C)`
  requires `mediated(i, C)` (a cut) **and** `nmifOk(C)`. Under-claimed
  independence, bypassed mediation, or an attacker-steered crossing → never
  attested.
- **Consumer** derives the result label under the signature and grants, releases
  iff it fits the envelope, with ungranted-mediated folding back to `direct`.

### A.6 Formal status

`models/maude/infoflow-signature.maude` + tests are in the gate (3 coverage / 6
bites, 0 warnings): producer bites (under-claimed independence, bypassed mediation,
attacker-steered crossing), consumer bites (ungranted→direct fallback — proven
non-vacuous as a differential against the granted coverage case, granted-but-
target-secret, producer-unsound-never-released).

**The claim the feature rests on is now machine-checked** in
`models/lean/Whipple/FlowSignature.lean` (in the `scripts/check-lean-models.sh`
gate; no `sorry`/`axiom`/`native_decide`), in the reader-set algebra of
`Whipple.ReaderSets`. The result carries the JOIN over paths (`joinReaders` /
`canRead_joinReaders`, lifting `canRead_append`), and:
- **cut⇒bound** (`cut_bound`): a granted mediated edge whose only path is the
  crossing yields result reader-set = the target, so reading is governed by the
  target alone; `mediated_drops_input` shows the input's own reader is dropped (the
  discount is real).
- **bypass loses the bound** (`bypass_requires_input`,
  `mediated_unsound_under_bypass`): any bypass path reintroduces the input reader, so
  mediation is sound only as a cut — pinning the producer obligation.
- **fail-closed fallback** (`ungranted_mediated_eq_direct`, `ungranted_fallback_sound`,
  `absent_admits_illegal_read`): ungranted = direct is sound; treating ungranted as
  `absent` concretely lets the PUBLIC read a non-public input.
- **producer soundness** = set inclusion (`canRead_sound`, `direct_sound`,
  `absent_sound_only_noreach`); **NMIF bridge** (`untrusted_no_discount`): no real
  confidentiality discount on attacker-controlled data.

## Direction B — Phase/provenance-typed turn outputs (DECIDED IN PRINCIPLE, STAGED)

A single model call is irreducibly a join box (we do not track through the model).
Sub-turn precision comes only from **decomposing the turn** — a public phase fixes
an output shape having never seen the secret (commit-then-fill), a private phase
fills specific slots under the secret — and labeling per provenance.

**The discipline that makes this free: the model fills slots, whip assembles the
record.** When composition happens in trusted whip code (a record literal whose
fields are bound from distinct turns), per-field labels fall out of ordinary
dataflow — **zero new IFC surface syntax.** New IFC syntax only ever lives at
*source labels* and *crossings*, both independent of granularity.

The version that *would* swallow the script — per-field annotations *within a single
model output* — is also the **unsound** one (it trusts the model to keep the secret
out of the "public" field), so it is rejected; the discipline is a coincidence of
soundness and ergonomics, not a limitation.

Today the checker is even coarser than per-turn: it is a **per-rule, resource-
granular** join box (`crates/whipplescript-cli/src/ifc.rs:548`). B's first win is
to shrink the box from **rule → turn** and track whip-level value-flow *between*
turns. The honest cost is **not syntactic**: (1) value/binding-level label tracking
(checker internals, new work), and (2) an occasional **extra model call** to get a
label split (a visible runtime cost the author opts into, not a syntactic tax).
Staged; v2 of A (per-field signatures) rides B's value-flow.

**Formal status (soundness argument modeled + proven, build still staged).** Maude
`models/maude/infoflow-staging.maude` (2 coverage / 2 bites) bites the per-turn win
(a field fed only by a public turn egresses to a public sink even with a secret turn
in the same rule) and the laundering bug (a sub-field claim on a single secret turn
output is ignored — the field carries the turn's join). Lean
`models/lean/Whipple/Staging.lean` proves the finding that **B needs no new label
theory**: field independence (`field_independent_of_unread`, via `mem_joinReaders`),
the sound refinement chain true ⊆ per-turn ⊆ per-rule (`soundDecl_trans`,
`perTurn_refines_perRule`, `perRule_admits_implies_field_safe`), and the rejected
per-field-within-one-output version (`single_output_subfield_unsound`) — all reusing
`FlowSignature`'s `joinReaders`/`soundDecl`.

## Direction C — Semantic checked declassifiers (DECIDED IN PRINCIPLE, STAGED)

Generalize I-IFC3's declassify **ceiling from a bounded *type* to a trusted
*predicate***: release only if a trusted classifier confirms a semantic condition
("no account numbers"), reusing the existing `exec "<validator>" -> Schema`
trusted-check primitive. The bounded type becomes the degenerate predicate "fits in
T."

**The oracle problem is inherited, not escaped — and the soundness argument is
whip-native.** A predicate that passes "no account numbers" still leaks `balance`
via adaptively-chosen "is balance > X?" releases that each pass the filter
(repeatable + automatic = oracle). The mitigation literature is clear in two ways:

- **Negative result:** classical query auditing (decide per-query safety from
  history) is computationally hard *and the refusal itself leaks* (a data-dependent
  denial is a signal). So **do not build a smart auditor**; any budget must be
  fixed in advance and public.
- **The right lever is integrity on the SELECTOR.** The adaptive oracle requires
  the *attacker* to steer the questions. whip already tracks integrity and I-IFC3
  already demands NMIF on downgrades, so the soundness argument is: a checked
  declassifier is sound when it is a **declared crossing** *and* its selector
  (which release is requested) is **high-integrity** — no attacker-steered oracle.

So C resolves into: **(1) structural (primary)** — declared one-shot crossing with
NMIF on the selector; **(2) quantitative (backstop)** — a *fixed, public* per-
(secret, party) release budget (DP-style where numeric, a flat query cap where
discrete) for the *self-inflicted* over-querying case only. Never a data-dependent
refusal.

**Open question (carried):** whether C earns its place over A+B. Staging (B) often
removes the need to declassify a model output at all; where staging is impossible,
C is exposed to the oracle precisely in the cases one would reach for it. C's one
distinct capability is releasing a function of a secret to a *narrower-but-nonzero*
audience, which staging cannot. Revisit on real demand.

**Formal status (soundness argument modeled + proven, build still staged).** Maude
`models/maude/infoflow-declassifier.maude` (3 coverage / 5 bites) bites NMIF-on-the-
selector (a low-integrity selector can never drive a privileged release — no adaptive
oracle), the budget backstop (an exhausted budget blocks release), the refusal
channel (a secret-dependent refusal cannot be safely observed by the public), and the
authority + predicate fail-closed gates. Lean
`models/lean/Whipple/CheckedDeclassify.lean` proves the structural argument:
`untrusted_selector_only_public` and its forward form
`privileged_release_needs_trusted_selector` (NMIF aimed at the *selector*, not the
value — the precise content of "no adaptive oracle to a privileged reader"), and
`refusal_channel` (a public-budget refusal is observable, a secret-dependent one is
not — so the budget must be fixed/public).

The **temporal layer** is `models/tla/InfoflowReleaseBudget.tla` (Apalache, in
`scripts/check-tla-models.sh`): over *all* release/refuse interleavings it checks
`BudgetBounded`/`BudgetNonNegative`/`CountMatchesLog` (the all-traces form of the
Maude single-step budget bite) and `NoPrivilegedTaintedRelease` — no privileged
release ever carries an adaptively-derived ("tainted", only available after a prior
release) selector, the operational trace form of the Lean per-step lemma. Both guards
are bite-verified (removing the budget guard breaks `BudgetBounded`; removing the
NMIF-on-selector guard breaks `NoPrivilegedTaintedRelease`). Honest caveat recorded in
the spec: full attacker-independence is a 2-safety hyperproperty that single-trace
model checking cannot decide; the selector invariant is the **safety surrogate** which,
with the Lean lemma, gives the operational "no adaptive oracle to a privileged reader."
So A is covered by Maude + Lean; B by Maude + Lean; C by Maude + Lean + TLA+ (the only
direction with genuine trace-temporal content). No Veil work (the repo's Veil layer is
a planned stub, not a live gate).

## Consequences

- **Supersedes** DR-0029 X2's "ideas brewing; do not design it in yet" — the
  extension is now designed (this record); DR-0029 §X2 points here.
- **Augments** DR-0027's QIF rejection with the fungibility objection as primary.
- The contract `flow` enum gains `"signature"` + an optional `flow_signature`
  (A.3); schema edit is implementation work, not done here.
- Sequencing: Lean cut⇒bound lemma (A.6) → contract schema + DR-0029/tracker
  updates → v1 producer reach-vector + consumer derive/gate → B value-flow → v2
  per-field + conditional discount → C if demanded.

### Implementation note (2026-06-30) — consumer-recompute supersedes producer-attest for v1

Whole-result v1 shipped in `ifc.rs` (`result_dependency_reads`, called from
`check_with_envelope_imports`). It departs from A.1's producer-attest path in one
deliberate way: the **reach matrix is recomputed CONSUMER-side from the pinned tool
source**, not read from a producer-attested `flow_signature` contract field. Rationale:
the structural reach is *label-agnostic* (A.1's reason to attest was that reader/writer
*sets* are party-relative — but the dependency *structure* is not), and the consumer
**already recompiles the pinned tool source** under its own envelope (the H8 cross-package
carriage precedent, DR-0029). So the consumer can derive the matrix directly — sound,
trust-free (no attestation to verify), and consistent with carriage. The `flow_signature`
contract field stays reserved for the case where a consumer does *not* have the source
(a binary/attested-only package); it is not needed while pinned-source recompilation is
the model. The top-level `@service` `complete result` is governed as an egress sink in
the same pass. Per-field v2 (value-flow attribution) remains deferred.

## Non-goals

- QIF / entropy budgets as a pervasive automatic mechanism (Decision 0).
- Producer-attested reader/writer sets (party-relative; not attestable — A.1).
- Per-field labels within a single model output (unsound — B).
- A smart, history-dependent query auditor (refusals leak — C).
- Conditional-discount *implementation* in v1 (modeled now, built in v2 — A.4).
