# DR-0029 — Cross-package information flow

Status: accepted (2026-06-27). Extends DR-0025 (workflows as agent tools) with the
information-flow obligations a package must meet for governance (DR-0027/0028) to
compose across a package boundary. Formal model: `models/maude/infoflow-package.maude`.
Full issue context: `spec/decision-records/information-flow-audit-findings.md` (X1–X8).

## Problem

A package is imported code that can declare resources, broker tools, and run rules.
IFC soundness (I-IFC8) requires every boundary to be labeled and every consumer to
gate. An unconstrained package is therefore the **"unmodeled door"** at package
granularity: if an imported `@tool` can touch a resource or egress to a provider the
consumer's governance cannot see, the guarantee is void at the seam.

## Decision

A package that exports a `@tool` carries an **information-flow surface** in its
contract (`information_flow` in `package_contract_v0`, optional; absent = no IFC
claim, governed by the consumer's fail-closed defaults). The check is **two-sided**,
and is the inline-refines-envelope check (I-IFC4) lifted to packages:

- **Producer** runs the IFC check on the package internals, proving its lowered
  effects stay within the **declared surface** and it performs **no undeclared
  crossing**; the contract then **attests** the IFC surface (`ifc_attested`).
- **Consumer** binds the package's abstract resource handles to governed resources
  and checks the declared **surface refines the consumer envelope**
  (`surface ⊑ envelope`): every required crossing is granted and every bound
  resource lands on a governed resource. Otherwise the import is rejected.

The soundness property (modeled + bite-tested): a cross-package tool grant is
accepted **only when** the package is IFC-attested **and** its declared surface
fits the consumer envelope. An un-attested package, or one whose surface exceeds the
envelope, is **never** accepted (fail-closed).

## The obligations (X1–X8)

1. **X1 — Effect-surface completeness (no hidden doors).** `surface` enumerates every
   resource/effect/egress (`kind:address`) and brokered tool; the producer attests
   lowered effects ⊆ `surface`.
2. **X2 — Per-tool flow signature.** Fixed at the **opaque join box** (output = join
   of all inputs, I-IFC2). Finer signatures are a reserved, **compiler-verified**
   extension — never asserted. QIF/entropy is out of scope.
3. **X3 — No package-asserted authority.** Crossings the tool needs are declared in
   `required_crossings`; the **consumer's** governance must grant each (authority
   lives in the consumer envelope, I-IFC4). Undeclared crossings are forbidden and
   attested-absent.
4. **X4 — Resource parameterization.** `resource_params` are bound by the consumer at
   import to governed `kind:address`; a package cannot self-bind to an arbitrary
   resource.
5. **X5 — Attestation covers IFC.** The producer attests surface-completeness and
   no-undeclared-crossings, verified through the same trust boundary as convergence
   (`VerifiedEnvelope` / DR-0028 G5) — every consumer gates.
6. **X6 — Transitive composition.** If A uses B, A's surface ⊇ B's (or B is
   encapsulated and re-attested); the transitive closure is explicit (mirrors the
   `invokes` convergence closure).
7. **X7 — Versioning / non-retroactivity.** The contract is attested at a hash; the
   consumer's approval binds to it (the package-lock). A surface change forces
   re-attestation and re-approval (D4 at package scope).
8. **X8 — Fail-closed least authority.** A package gets only consumer-granted
   authority; ungranted access ⇒ import rejected with a routes-to-fix.

## Contract shape

`package_contract_v0.workflow_tools[].information_flow`:
`surface` (X1), `resource_params` (X4), `required_crossings` (X3),
`flow: "join_box"` (X2), `ifc_attested` (X5). `invokes` already gives X6; the lock
digest gives X7; the consumer check gives X8.

## Remaining implementation

The data shape and the model are landed. The producer-side surface computation +
attestation and the consumer-side `surface ⊑ envelope` check are the implementation
slices that follow (tracked in the audit-findings doc, Wave 3).
