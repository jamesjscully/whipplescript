# DR-0027: Information-flow control — provable non-interference for agent workflows (founding premise and invariants)

Status: accepted 2026-06-27 (founding premise + invariants). Like
[DR-0024](0024-owned-brokered-agent-harness.md), this record locks **only** the
premise and the invariants every downstream decision must preserve. The surface
syntax, the precise label algebra, the nonmalleable-flow checking and inference
algorithms, the grounding onto concrete whipplescript constructs, and the
implementation slices are deliberately **left open** and sequenced as later steps
(see "What this record does not decide"). The companion record
[DR-0028](0028-information-flow-authority.md) locks the *authority* model — where
policy lives and how it is authored — and is the governance half of this one.

Exploratory formal models gate the design and already exist:
`models/maude/infoflow-integrity.maude`, `infoflow-confidentiality.maude`, and
`infoflow-composition.maude` (each with coverage + bite, registered in
`scripts/check-formal-models.sh`).

## Problem

A single-context agent conflates two things a security system must keep apart:
**data** and **instructions**. Every channel that reaches the model's context can
be interpreted as a command, and everything in that context can influence
everything the model emits. Two consequences, and both are structural:

```text
1. Injection. Untrusted input - an email body, a stranger's message, fetched web
   content - can carry instructions the model obeys. There is no boundary
   between "content to process" and "command to follow".

2. Exfiltration. Sensitive data in the model's context can leave through any
   output channel. There is no boundary between "data cleared for this reader"
   and "data the reader must never see".
```

This is why you cannot safely connect an LLM agent to your bank and a public
inbox at once, nor stand up a surface where multiple parties interact while some
data stays provably protected from some of them. The recommended practice today
is "don't" — there is no enforced boundary to rely on.

The opportunity is specific to whipplescript. Because whip is a **compiled DSL
with typed effects and a formal-model discipline**, the data flowing through a
workflow can carry **information-flow labels** that a static analysis propagates
and checks. Prompt injection becomes a typed **integrity** violation; data
leakage becomes a typed **confidentiality** violation; and the compiler can
**prove**, before a whip runs, that neither can occur except through a small set
of explicit, audited crossings. This is the capability the rest of this record
locks the shape of.

## Decision

Introduce information-flow control as a first-class **compile-time** analysis
over whip programs: a label system, propagated through the dataflow graph and
checked at effect boundaries, that proves non-interference relative to a declared
policy. The analysis builds on the decentralized / asymmetric-delegation label
model (Myers–Liskov DLM; Arden–Liu–Myers flow-limited authorization;
Ren–Acay–Myers asymmetric delegation and polymorphic label inference) and on
nonmalleable information flow (Cecchetti–Myers–Arden).

This record locks eight invariants. Everything downstream must preserve them.

### I-IFC1 — Two axes, both party-relative

> Every value carries a **confidentiality** label and an **integrity** label.
> Neither is binary. Confidentiality is "who may read this" — a set of
> principals. Integrity is "who vouches for this / who may have influenced it" —
> a set of principals. Principals are ordered by an **acts-for** relation, and
> roles are abstract principals.

Prompt injection is then precisely *a value whose integrity does not meet a
control point's requirement* reaching that control point. Leakage is precisely *a
value reaching a party not in its reader set*. "Protected from whom" and "trusted
for whom" are first-class: a binary `secret`/`tainted` is the degenerate
one-principal case. The party-relative form is also the primary *permissiveness*
mechanism — it constrains only the flows a policy names and lets everything else
flow, which is the maximally-permissive reading of a declared protection (see
I-IFC5).

### I-IFC2 — The agent turn is an opaque join box; enforce at boundaries

> An agent turn's output carries the **join** of every label in its context —
> the meet of the inputs' integrity and the join of their confidentiality. We do
> not track flow *through* the model; we enforce only at sources, sinks, and
> crossings.

The model freely mixes its whole context into its output, so any finer claim is
unsound. The join box is conservative in the safe direction: it never
under-approximates a leak or a taint. This reaffirms and specializes
[DR-0024](0024-owned-brokered-agent-harness.md) I2/I3 — the turn was always a
black box, and I3's "no upward control-flow leak" is the structural ancestor of
the integrity invariant. The corollary bounds how we recover precision:

```text
The ONLY sound ways to be more permissive are:
  a. more precise party-relative labels (I-IFC1),
  b. separating contexts so a harmless output never had the secret in scope,
  c. an explicit, audited downgrade (I-IFC3).
NEVER by trusting the model more about what it did with its context.
```

Lever (b) has sound, *construction-based* forms we do rely on for ergonomics:
staged turns (a public phase produces the public-facing output having never seen
the secret), commit-then-fill (the model fixes an output *shape* with the secret
absent, then fills it under the shape's bounded capacity), and trusted **input
coarsening** (the harness shows the model `balance: >$1000` instead of `$4,237`,
lowering the input — and thus the output — bound). These eliminate or shrink the
flow by construction, without trusting the model.

### Quantitative information flow is out of scope (not deferred)

The box is **binary**: a value that touched a secret is secret. A *quantitative*
box — crediting an output's bounded **type** as a leak ceiling (a turn reading the
whole ledger and emitting a 3-valued enum leaks ≤ log₂3 bits) and tracking a
depleting **leak budget** — is strictly richer, sound in principle, and would
dissolve the "read secret, emit benign summary" conservatism. We **deliberately
keep it out of scope**, not merely deferred, because quantitative leak accounting
is delicate (adaptive adversaries, channel composition, correlated values) and a
*subtly wrong* bound is worse than an honest binary one — the binary box's value
is precisely that its soundness is believable. The one place we already bound
*quantity* is the **mandatory bounded type on an explicit, audited `declassify`/
`endorse` crossing** (I-IFC3): quantitative bounding stays confined to those
declared crossings, never a pervasive automatic budget. If the binary box's
rejection rate ever proves to be the real bottleneck, the sound escalation order
is the construction-based levers above *first*, and a quantitative budget only as a
last resort, heavily modeled.

### I-IFC3 — Downgrading is explicit, authority-scoped, and nonmalleable

> The only operations that lower confidentiality (**declassify**) or raise
> integrity (**endorse**) are explicit in the source, require the owning
> principal's authority, are recorded as an enumerable audit fact, and are held
> to **nonmalleable information flow (NMIF)**: a downgrade may not be influenced
> by an attacker.

Structural validation is **not** endorsement: `coerce` / `exec -> Schema` turns
untrusted text into a typed value at the *same* integrity (well-shaped, still
untrusted) and is free and unmarked. `endorse` is the *separate*, explicit,
authority-scoped act that raises integrity — written as an explicit `endorsed`
marker on the validated value, gated by a governance grant and audited (see the
[information-flow surface](../information-flow-surface.md)). `declassify` releases
a bounded typed value to a wider reader set. Both crossings carry a mandatory
bounded type as their bandwidth ceiling, and their target level / audience comes
from the governance grant, not the script. The two hatches are **axis-locked**:
endorse raises integrity
but preserves confidentiality (an endorsed secret is still secret); declassify
lowers confidentiality but preserves integrity (a declassified untrusted value is
still untrusted). The **audit set is the trusted surface** — enumerate the
declassify/endorse crossings and you have the complete list of points a human
reviews; a program with no crossing has an empty trusted surface and the static
checks alone carry the guarantee. NMIF is the rigorous safety property our
exploratory `endorsedAt`/`declassifiedAt` audit gestured at.

### I-IFC4 — Authority is held in the locked governance envelope; programs express usage

> The principal/role hierarchy, the delegation context, data ownership, the
> protected-from relations, and downgrade authority live in the **governance
> envelope**, not in whip programs. A whip expresses how it *uses* data within
> the envelope; the compiler proves **usage refines the envelope**. No program
> can introduce a delegation, relax a protection, or widen its own authority.

This is the information-flow specialization of
[DR-0026](0026-session-root-agent.md)'s "the root cannot widen its own envelope".
The full authority model — where the envelope is authored, how it is locked, and
the two authoring cases — is locked in
[DR-0028](0028-information-flow-authority.md). The dividing line is **authority
vs usage**: anything that grants access or relaxes protection is the envelope's
job; anything that merely uses data within granted bounds is inline and inferred.

### I-IFC5 — The guarantee is policy-relative, and its scope is explicit data flow

> What we prove is **non-interference relative to the declared policy** —
> delimited release: everything flows except what a policy forbids, and
> declassifications are exactly the declared relaxations. We prove explicit and
> implicit *information* flows. We do **not** prove the absence of timing,
> termination, existence, or resource side channels.

Stated as an invariant because honesty about scope is load-bearing: a
declassified one-bit decision *is* a deliberate one-bit release (that is what
declassification means); we bound it, we do not eliminate it. The multi-party
adversarial setting raises the stakes on side channels even though they remain
out of static scope. This is still categorically stronger than "one context,
everything mixes."

### I-IFC6 — Governance-relative soundness, fail-closed at the governed boundary

> Soundness is **relative to the governance config**: an empty config protects
> nothing (a plain whip runs unconstrained), and the guarantee equals exactly what
> governance declares. Where a protection *is* declared, the analysis is
> fail-closed **at the boundary of the governed surface** — **governed data may not
> flow to an un-cleared or ungoverned sink**, and unlabeled intermediates within a
> governed flow are treated conservatively. Inference never under-approximates a
> label.

This is the gradual form (the
[information-flow surface](../information-flow-surface.md)): IFC is opt-in via the
governance config, so there is no global "every source must be labeled" demand —
many whips need no config at all. The soundness comes instead from the **sticky
boundary**: once a value derives from a protected real source, every one of its
sinks must be governed and cleared, or the whip is rejected (the diagnostic names
the real resource to label). Governed data cannot leak into the ungoverned region.
Confidentiality defaults to maximally secret and integrity to minimally trusted, so
an omission *inside* a governed flow fails safe on both axes. This refines, rather
than contradicts, I-IFC5's policy-relative guarantee: the empty policy is the
default floor.

### I-IFC7 — Labels are preserved across every durable and distributed boundary

> Persisting a value to a store and reading it back, and sending a value across an
> instance boundary, both carry the value's label. Store locations (`file_store`,
> ledger, queue, memory pool, the fact-base) and channels are themselves labeled,
> and no persistence or communication boundary launders a label.

WhippleScript is durable and multi-instance, so most data eventually leaves a
program's live memory — written to a store, read by a later revision, or sent to
another instance. A static label that does not survive these boundaries is no
guarantee at all. A store location's declared label is the label of anything read
from it, and a write into it is a checked crossing (the dual-gated `record` sink
is the canonical case). A cross-instance send carries the channel's label; the
receiver treats incoming data at that label. This is the invariant a stateless IFC
language never needs, and the one multi-party safety most depends on.

### I-IFC8 — The brokered tool surface is fully labeled

> Every tool the owned harness brokers is a labeled boundary. Its outputs carry a
> label and its effects are label-checked; there is no brokered tool that is an
> unlabeled hole.

The agent touches the world only through brokered tools
([DR-0024](0024-owned-brokered-agent-harness.md) I1), so an unlabeled tool is a
laundering channel that defeats every other invariant. The classification is
per-tool: `bash` and web-fetch outputs are low integrity (untrusted origin); a
file read carries its `file_store`'s label; `write`/`record` are checked crossings
gated by the target's label; a `workflow.invoke`
([DR-0025](0025-workflows-as-agent-tools.md)) carries the labels of its typed
input and result. Tool labeling extends the guarantee through the one place the
model reaches the world.

## Why information-flow control, stated once

- Enforced, *provable* multi-party safety is the capability unreachable any other
  way. It cannot be bolted onto a single-context agent — the architecture is the
  vulnerability. Either the data and instructions are separable and checkable, or
  they are not; if they are not, "be careful with prompts" is the only tool, and
  it is not a guarantee.
- The decision hinges on one question, answered yes: do we want to connect agents
  to sensitive data and untrusted/multi-party channels with a compile-time proof
  that injection cannot reach control and protected data cannot reach an
  unauthorized party? If yes, build the label system. If "trust the model and
  review prompts" were enough, we would build nothing.

## Relationship to existing records

```text
DR-0024 owned harness     the agent turn as a brokered leaf is the join box
                          (I-IFC2); I3 "no upward control-flow leak" is the
                          structural ancestor of the integrity axis.
DR-0025 workflows-as-tools a workflow.invoke is a typed crossing; its input
                          contract is a natural label-checked boundary.
DR-0026 session root      the policy envelope now carries IFC content; the two
                          authoring cases are its governance Options A and B.
DR-0028 IFC authority     the governance half of this record: where policy lives
                          and how it is authored, held, and locked.
```

## Verification toolchain

The guarantee spans static, durable, and algebraic obligations, and no single tool
covers them. The division, matching the standing discipline:

```text
Maude            the rule-system / compiler surface: static label propagation,
                 boundary checks, the refinement check (inline subset-of envelope)
                 as a reachability property, and coverage+bite on CONCRETE programs
                 (I-IFC6, the static half of I-IFC8). Bounded search, not proof: it
                 shows a given bad program is rejected, not that all are.

TLA+             the temporal / durable / distributed invariants: I-IFC7 label
                 carriage across persistence and instance boundaries, DR-0028 D4
                 envelope versioning and non-retroactive authorization, and
                 replay-stability of labels.

proof assistant  the algebraic metatheory: the label lattice, NMIF, and inference
(Isabelle/Rocq/  soundness. We LEAN ON the published mechanized results of the
 Lean)           asymmetric-delegation / NMIF / FLAM line and mechanize only OUR
                 deltas (the dual-gated record, durable-store labeling, the
                 envelope refinement), each shown to be a soundness-preserving
                 extension. We do not re-derive the paper.

SMT (Z3)         an IMPLEMENTATION choice, not a verification tool: bounded label
                 inference is constraint-solving over the lattice. Building, not
                 proving.
```

The honest claim this supports is "provable non-interference relative to the
declared policy" — earned by proof for the algebra and model-checking for the
program and lifecycle surface, not "we tested some examples".

## What this record does not decide

Explicitly open, sequenced as later steps (modeled-first per standing
discipline):

```text
- Surface syntax. DRAFTED in the [information-flow surface](../information-flow-surface.md):
  governance grants bind+authorize+label real resources (typed `kind:address`),
  source is label-free except the two explicit crossings (`endorsed` /
  `declassify`, target from grant, bounded type), `coerce` is structural-only,
  sinks and provider egress are implicit. Remaining-open within it: the governance
  config's signed serialization, clearance-based provider routing, role-generics,
  per-field labels beyond the channel/store grain.

- The precise label algebra + checking + inference. Which formalization of
  party-relative labels and acts-for; the join/meet over principal sets; the NMIF
  static check; and bounded label polymorphism with inference so workflows are
  written generic over principals and bound to a finite runtime set. Adopt and
  adapt the asymmetric-delegation line; do not reinvent.

- Construct grounding. DRAFTED in the [information-flow surface](../information-flow-surface.md):
  labels live on governance grants over real resources (typed `kind:address`), not
  in source; the source crossings are the explicit `endorsed` marker (over a
  structural `coerce`) and `declassify`; stores and `record` are dual-gated; sink
  checks on send / spawn / record. The construct/boundary audit (with the five
  non-obvious doors) is there too.

- Formal-model upgrade. The exploratory models are 2-and-multi-point single-owner
  lattices. Upgrade to party-relative labels with acts-for, NMIF, and the
  governance refinement check; preserve coverage AND bite.

- Slices. A per-slice model -> code -> review -> docs -> gate sequence; a checkbox
  tracker spawns when implementation begins. Integrity-first is the agreed order
  (anti-injection is the most visceral and hardens the DR-0026 root power), with
  confidentiality the symmetric follow-on, then party-relative, then NMIF.
```

## Deferred capabilities (recorded hooks, not v1)

```text
decentralized ownership   multiple self-managing data owners who each declassify
                          their own data. v1 is a SINGLE governance authority
                          (DR-0028). Asymmetric delegation already accommodates it
                          additively when mutually-distrusting parties self-manage.

dynamic principals        parties joining/leaving during execution. v1 is
                          open-at-compile-time, finite-at-runtime: programs are
                          principal-polymorphic, the governance config binds a
                          fixed finite party set per deployment.

side-channel control      timing / termination / existence / resource channels.
                          Out of static scope (I-IFC5); a separate concern if ever
                          taken up.
```

## Consequences

- Every value carries party-relative confidentiality and integrity labels;
  injection and leakage are typed violations the compiler rejects (I-IFC1).
- The agent turn is a join box; enforcement is at boundaries; precision is
  recovered only by precise labels, context separation, or audited downgrade —
  never by trusting the model (I-IFC2).
- declassify/endorse are explicit, authority-scoped, axis-locked, NMIF-safe, and
  audited; the audit set is the trusted surface (I-IFC3).
- Authority lives in the governance envelope; programs express usage and are
  proven to refine it; nothing self-widens (I-IFC4, DR-0028).
- The guarantee is policy-relative non-interference over explicit and implicit
  information flows; side channels are out of scope and that is stated, not hidden
  (I-IFC5).
- Every source is labeled and unknowns fail closed; inference never
  under-approximates (I-IFC6).
- Labels survive persistence and cross-instance boundaries; stores and channels
  are labeled; no durable or distributed boundary launders a label (I-IFC7).
- Every brokered tool is a labeled boundary; there is no unlabeled tool hole, and
  context compaction is no exception — a summary carries the join of what it
  summarizes (I-IFC8, I-IFC2).
- Verification is split across tools: Maude for the compiler surface and concrete
  bite, TLA+ for the durable/temporal/distributed invariants, a proof assistant
  scoped to our deltas over the published algebra, SMT inside the inference engine.
