# Information-flow audit — findings, status, and fix plan

Opened 2026-06-27 after the **report-vs-check bug**: the guarantee report rendered
against a *tampered* signed envelope instead of refusing, because it was a SECOND
consumer of the attested artifact that did not gate, while the checker did. That
one bug triggered a full audit of the modeling surface and the plan execution.
This document is the **single source of truth** for everything the audit found.
Nothing here is closed until it has a model/proof AND an implementation AND a test.

## The meta-lesson (root cause of the class)

1. **Single-consumer / single-path modeling.** Invariants were phrased about ONE
   consumer ("the whip agent only enforces") and models had ONE consumer of each
   trusted artifact. A second consumer that skips the check is then both
   unmodeled and permitted. Fix discipline: model the **artifact and ALL its
   consumers**; phrase invariants over the **boundary**, not a path.
2. **Bounded tests mistaken for proofs.** "We have formal models" meant Maude
   `search` on 2–3-principal fixtures, not universal proofs. The load-bearing
   algebra was asserted in comments. Fix discipline: **machine-check the algebra**
   (Lean), keep Maude/TLA+ for bite on concrete programs.
3. **Test cross-product gaps.** The `attestation × report` cell was never tested.
   Fix discipline: **a negative bite per consumer**, per trusted artifact.

## Issue inventory

Status key: `DONE` · `PARTIAL` · `OPEN` · `DEFERRED`.

### M — Verification method
- **M1 — Algebra only asserted.** Preorder, closure correctness, label semilattice,
  non-interference, NMIF were comments, not proofs. **PARTIAL** — Lean now proves
  the acts-for preorder, `public` bottom, `canAct` sound+complete (`canAct_iff`,
  axioms `[propext, Quot.sound]`), the conf/integ duality, and the sticky boundary
  (`models/lean/`). OPEN within M1: reader/writer **sets** → semilattice, NMIF,
  non-interference-relative-to-policy as a theorem, and an **agreement** result
  that our `canAct` instantiates the published asymmetric-delegation order.
- **M2 — Maude models are single-consumer.** No model had multiple consumers of a
  trusted artifact; `subworkflow-attestation` modeled only the enforce path.
  **DONE (Wave 1)** — added a second consumer (`publish`, the report analogue) held
  to the same attestation gate; new coverage (genuine serves both) + bite (an
  un-attested tool is never published either); bite-tested (an un-gated rule flips
  the No-solution to a Solution).
- **M3 — No IFC TLA+/Veil.** Durable label carriage (I-IFC7), envelope versioning /
  non-retroactivity (D4), replay-stability are temporal/distributed and unmodeled.
  **OPEN.**
- **M4 — Cross-product test discipline.** Institute "negative bite per consumer per
  trusted artifact" as a standing rule. **OPEN (process).**

### P — Invariant phrasing (path → artifact)
- **P1 — Attestation/G4 phrased per-consumer.** "The whip agent only enforces"
  permitted other consumers to skip verify. **DONE (Wave 1)** — Lean `Verified`
  boundary proves all consumers gate; Rust `ifc::VerifiedEnvelope` is the only
  env→envelope path and `check_with_envelope`/`governance_report` require it (a
  third consumer is now a compile error); DR-0028 gains **G5 (verified-artifact
  boundary)** phrased over the artifact, not a path.
- **P2 — I-IFC3 scoped to downgrades only.** Non-downgrade paths are implicitly
  uncovered by its wording. **OPEN** — review/rephrase for completeness.

### E — Enforcement / implementation (plan ~40% executed)
- **E1 — Refinement check `inline ⊑ envelope` (I-IFC4).** A whip (or package) can
  use data at weaker labels than declared without rejection. No model, no impl.
  **OPEN.**
- **E2 — Five-doors boundary checklist (I-IFC8).** telemetry/logs, session-event
  stream, human.ask not registered as boundaries = unlabeled holes. **OPEN.**
- **E3 — Kernel runtime enforcement (Phase 4).** Envelope load+attestation at the
  kernel, dual-gated stores/`record`, envelope versioning + run binding (D4),
  discovery. Currently check-time only. **OPEN.**
- **E4 — Source crossings in `.whip` grammar.** `endorsed` marker, `declassify`
  construct, role references; the trusted surface is not visible in source for
  review. **OPEN.**
- **E5 — IR party-relative labels + `kind:address` resource ids.** Checker keys on
  handle names, not stable typed resource ids. **PARTIAL/OPEN.**
- **E6 — Reader/writer SETS.** A single role up-set per resource today; real labels
  are sets of principals (a lattice). **OPEN** (paired with M1).
- **E7 — Whip-agent acts-for-user binding (D3).** Only OS-privilege proxy; no
  account scoping enforced. **OPEN.**

### H — Found hands-on (beyond the audit agents)
- **H1 — report-vs-check tamper.** **DONE (Wave 1)** — subsumed by P1: the report
  routes through `VerifiedEnvelope` and refuses a tampered policy structurally; the
  point-fix is gone.
- **H2 — Workflow-result channel is an unmonitored sink.** **PARTIAL (Wave 2)** —
  `record` is now a governed sink `fact:<schema>` (derived from `fact_writes`),
  default public/fail-closed, so a fact derived from confidential data is caught
  unless governance clears the fact (the recordSink of infoflow-composition,
  realized). OPEN: `complete result` (the result channel to the *invoker*) — its
  per-rule form isn't cleanly in the IR and it overlaps the cross-package `@tool`
  result (Wave 3, opaque join box), so it is folded into Wave 3.
- **H3 — Inbound message-trigger is not an integrity source.** `when message from`
  untrusted data isn't modeled as a read; injection only caught from file reads.
  **OPEN.**
- **H4 — `endorse` crossings absent from the trusted-surface report.** **DONE
  (Wave 2)** — the report now audits both axes, tagged `declassify …` / `endorse …`.
- **H5 — Clearing a provider marks it "confidential".** A principal appears as a
  protected resource in the report. **OPEN (cosmetic/semantic).**
- **H6 — Diagnostic span is the whole rule.** Not the offending read/write pair.
  **OPEN (usability).**
- **H7 — Per-field / per-path labels.** Mixed-sensitivity stores must be split;
  labels attach to whole resources. **DEFERRED (recorded).**

### X — Cross-package governance obligations (new; see next section)
- **X1–X8** — what packages must guarantee for governance to compose across them.
  **OPEN (design).** Detailed below.

## Cross-package governance obligations

Governance soundness depends on every boundary being labeled and every consumer
gating. A **package** is imported code that can declare resources/constructs,
broker tools, and run rules — so an unconstrained package is exactly the
"unmodeled door = hole" (I-IFC8) at package granularity. For governance to hold
ACROSS a package boundary, the **package contract** must carry an
`information_flow` obligation block, and both sides must check it:

- **X1 — Effect-surface completeness (no hidden doors).** The contract enumerates
  every resource/effect/egress (`kind:address`) and every brokered tool the package
  can perform. The compiler verifies the package's lowered effects ⊆ its declared
  surface. A package that can open a channel / exec / hit a URL outside its declared
  surface is unsound.
- **X2 — Per-tool flow signature. DECIDED 2026-06-27: opaque join box only.** Every
  exported tool's output carries the join of all inputs (I-IFC2); no per-tool flow
  precision in v1. **Extension point reserved:** finer signatures may be added later
  but ONLY when compiler-verified at package build (the producer runs the IFC check
  on package internals and the attestation carries that machine-checked result) —
  never merely asserted. QIF/entropy-budget precision is explicitly NOT adopted
  (ideas brewing; do not design it in yet). Keep the contract field shaped so a
  `flow_signature` can be added without a breaking change.
- **X3 — No package-asserted authority.** Crossings (`declassify`/`endorse`) and
  resource access require the CONSUMER's governance grant (I-IFC4). The package
  DECLARES required crossings/authority as obligations; undeclared crossings are
  forbidden and attested-absent.
- **X4 — Resource parameterization.** The package's abstract resource handles are
  bound by the consumer at import to real governed `kind:address`. A package cannot
  self-bind to an arbitrary real resource; the binding surface is part of the
  contract (so the consumer's governance controls what backs each handle).
- **X5 — Attestation covers IFC.** The producer attests surface-completeness,
  no-undeclared-crossings, and flow-signature accuracy — not just convergence
  (extends `subworkflow-attestation`). The consumer **verifies** the attestation
  (the `Verified` boundary) and **every** consumer (checker, report, kernel) gates.
- **X6 — Transitive composition.** If A uses B, A's surface ⊇ B's (or B is
  encapsulated and re-attested). The transitive closure is explicit (mirrors the
  convergence closure already modeled).
- **X7 — Versioning / non-retroactivity (D4 at package scope).** The contract is
  attested at a hash/version; the consumer's approval binds to that hash. A surface
  change forces re-attestation and re-approval; the package-lock binds the hash.
- **X8 — Fail-closed least authority.** A package gets only consumer-granted
  authority; ungranted access ⇒ import rejected with a routes-to-fix. The sticky
  boundary at package granularity.

Checking is **two-sided**, and is exactly `⊑` (I-IFC4) lifted to packages:
- **Producer side:** proves the package's code stays within its declared surface
  and performs no undeclared crossing (runs the IFC check on package internals
  against the declared surface; the result is what the attestation covers).
- **Consumer side:** proves the declared surface fits the consumer's governance —
  `package-surface ⊑ consumer-envelope` — i.e. the resource bindings land on
  governed resources and every required crossing is granted.

## Decisions (2026-06-27)

- **Flow signatures: opaque join box only** (Fork A), with a reserved
  compiler-verified extension point; QIF/entropy not adopted. (See X2.)
- **Sequencing: keep the wave order below** (Fork B) — local sinks before
  cross-package.

## Carried-forward commitments (do not lose)

The four items agreed in discussion map onto the inventory as follows, so they
are tracked by ID, not by memory:

1. **`VerifiedEnvelope` boundary type in Rust** — realize `Boundary.lean` in code;
   route checker AND report (and any future consumer) through a type that cannot be
   constructed from a signed artifact without verification; subsumes the H1 point-fix
   and makes a future third consumer a compile error. → **P1 / H1, Wave 1.**
2. **Maude multi-consumer bite** — add a second consumer to `subworkflow-attestation`
   so the model bites the bug class concretely (Lean covers it algebraically).
   → **M2, Wave 1.**
3. **Unmonitored sinks** — model `complete result`/`record` (the workflow→invoker
   channel) and the five doors (telemetry, human.ask, session-event) as boundaries.
   → **H2 + H3 + E2, Wave 2.**
4. **Next Lean targets** — NMIF (robust declassification) and reader/writer sets;
   then Veil/TLA+ for durable label carriage (I-IFC7), a transition-system property
   and the natural home for Veil-on-Lean. → **M1/E6, Wave 4; M3, Wave 6.**

## Plan — waves

Sequenced so each wave is model-first, then impl, then test, and so the
highest-leverage corrections (the bug class + the unproven core) go first.

- **Wave 0 (DONE).** Lean foundation: preorder, `canAct_iff`, duality, sticky
  boundary, `Verified` boundary; gate `check-lean-models.sh`.
- **Wave 1 (DONE 2026-06-27) — Closed the bug class end-to-end.** Rust
  `VerifiedEnvelope` boundary type (P1/H1); checker + report require it; Maude
  multi-consumer bite (M2); DR-0028 G5 phrased over the artifact (P1).
- **Wave 2 — Close the unmonitored sinks.** Workflow-result channel (H2), inbound
  message-trigger as integrity source (H3), the five doors (E2); each modeled then
  checked. Small report fixes (H4, H5, H6) ride along.
- **Wave 3 — Cross-package governance.** Ratify X1–X8 as a DR amendment to DR-0025;
  add the `information_flow` block to `package_contract` (v1); two-sided check;
  extend attestation + the Maude attestation model to cover IFC (ties to M2).
- **Wave 4 — Algebra depth (Lean).** Reader/writer sets → semilattice (M1/E6), NMIF
  (M1), non-interference-relative-to-policy (M1), agreement with published order.
- **Wave 5 — Refinement + kernel + source crossings.** `inline ⊑ envelope` (E1),
  kernel runtime enforcement (E3), `.whip` crossing grammar (E4), `kind:address`
  IR labels (E5), whip-agent account binding (E7).
- **Wave 6 — Temporal.** TLA+/Veil for durable carriage + versioning (M3).
- **Deferred (recorded, not lost):** per-field labels (H7); QIF (out of scope).
