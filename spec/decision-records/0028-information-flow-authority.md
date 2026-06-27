# DR-0028: Information-flow authority — the governance envelope, and the authority/usage split

Status: accepted 2026-06-27 (authority model). The governance half of
[DR-0027](0027-information-flow-control.md): that record locks *what* is proved
about information flow; this one locks *where policy lives, who authors it, and
how it is held and enforced*. It extends [DR-0026](0026-session-root-agent.md)'s
policy envelope to carry information-flow content and identifies the two
authoring cases with DR-0026's governance Options A and B. Surface syntax and the
checking algorithm are deferred to DR-0027's open steps.

## Problem

[DR-0027](0027-information-flow-control.md) I-IFC4 states that authority lives in
a locked governance envelope and programs express usage. That raises the question
this record answers: **who writes the policy, where does it live, and how is it
held so that a powerful root agent cannot erode it?**

Two concrete use cases drive the answer, and they look different on the surface:

```text
Case 1  An IT data owner and a non-technical user. The user talks to the root
        agent, which writes whips to automate the user's work. The data owner
        wants to hand the user a configured envelope that GUARANTEES safety,
        regardless of what the user asks or what the agent writes.

Case 2  A sophisticated user expresses, to the root agent in natural language, an
        intent to keep certain data protections in place. The agent writes those
        protections into the whips. The user TRUSTS the agent to transcribe the
        intent faithfully.
```

They appear to call for different mechanisms — an external envelope in one, inline
protections in the other. They do not. They are the same mechanism with a
different pen on the envelope.

## Decision

Policy is held in a **two-tier structure**, and the dividing line is **authority
vs usage**:

```text
governance envelope (locked, kernel-enforced)   inline in whips (inferred, checked)
---------------------------------------------   ----------------------------------
AUTHORITY decisions:                            USAGE:
  the principal / role hierarchy                  this workflow reads Ledger
  the delegation context (acts-for)               this turn coerces an Intake msg
  data ownership                                  this send goes to Requester
  the protected-from relations                  expressed freely by the author /
  who holds declassify / endorse authority      agent; never establishes authority
```

The compiler proves **usage refines the envelope** (`inline ⊑ envelope`): a whip
expresses how it uses data within the bounds, and can never introduce a
delegation, relax a protection, or grant itself access the envelope withheld.
There is **always** a locked envelope; what varies between the two cases is only
who drafts it and how it gets locked. Four things this record locks:

### D1 — The governance envelope is the single authority root, always locked

> A single **governance authority** sets all policy in the protected governance
> configuration layer. The envelope holds the principal/role hierarchy, the
> delegation context, ownership, the protected-from relations, and downgrade
> authority. It is signed and kernel-enforced; the root agent cannot modify it.

This is [DR-0026](0026-session-root-agent.md)'s policy envelope, now carrying IFC
content. The delegation context **is** the asymmetric-delegation θ of DR-0027's
label model — the same artifact that prior work calls a trust configuration,
interpretation function, or meta-policy. The unsafe configuration we never permit
is "protections purely inline with no locked ceiling," because then the agent
could silently erode them over time. The minimum viable system still has a locked
envelope (Case 2 below).

### D2 — Roles in source, parties and delegations in governance

> Whip source references **roles** — abstract principals, stable and few. The
> governance config binds **concrete parties** to roles and sets the delegation
> context. Programs are principal-polymorphic; the envelope instantiates a finite
> party set per deployment.

This is what makes "open at compile time, finite at runtime" work (DR-0027's
bounded label polymorphism): a workflow is written once against roles
(`Requester`, `Approver`, `Operator`, `Auditor`) and checked against them; the
governance layer maps who fills each role. It cleanly separates *what the workflow
guarantees* (roles, compile-checked) from *who is who* (parties, in governance,
deployment-specific). Under the gradual surface, boundary labels live on the
**governance grants over real resources** (not in source); source carries a label
only at the explicit `endorsed`/`declassify` crossings, where it references a role,
with everything else inferred (see the
[information-flow surface](../information-flow-surface.md)).

### D3 — Trust required equals authority delegated; the agent acts for its user

> The root agent always operates with the **authority of the user it serves** —
> it acts-for that user, never beyond. The trust a human must place in the agent
> equals the authority delegated to it over the data in question.

This single law resolves both cases and is the heart of the record:

```text
Case 1, guaranteed safety with zero trust in the agent for protected data.
  The data owner authors and signs the envelope on the separate, sudo-gated
  governance agent (D5).
  The user is a principal in it - say role User - and protected data, say Ledger,
  is readable by Operator with User not in the readers and User holding no
  declassify authority over it. The agent acts-for User. The moment a whip would
  route Ledger to the user's channel, the compiler rejects it: User cannot read
  Ledger and cannot declassify it. Safety holds no matter how the user prompts
  the agent, because the agent was delegated NO authority over the protected data.
  The non-technical user never sees a label; safety is invisible. The confused-
  deputy attack is structurally prevented - the agent is capped at the user's
  authority, not the data owner's.

Case 2, trust-but-verify because the agent holds the user's own authority.
  The user owns their data, so delegating authority to the agent gives it the
  user's authority to declassify the user's own data - which is exactly why this
  case requires trust and Case 1 does not. On the sudo-gated governance agent (the
  user acting as their own admin, D5), the agent DRAFTS the envelope protections
  from the NL intent, surfaces them back in plain language, the user RATIFIES, and
  they LOCK into the envelope. After locking it is enforced identically to Case 1.
  The agent PROPOSES; the
  human DISPOSES; the locked result BINDS future agent actions. The NMIF / trusted
  -surface audit (DR-0027 I-IFC3) is what makes "trust but verify" real - every
  protection and every downgrade is enumerable and shown back for confirmation.
```

The whip agent may author whips freely — including the `endorsed`/`declassify`
crossings — and may **propose** envelope changes, but may **never self-apply** an
envelope change. That is DR-0026's "kernel enforces, agent cannot self-widen", now
carrying IFC content.

### D4 — Envelope changes are versioned and non-retroactive

> A change to the governance envelope is itself a high-authority operation. It
> does **not** retroactively authorize past flows. In-flight work is bound to the
> envelope version it began under, and is revalidated rather than silently
> re-permitted when the envelope changes.

Policy lives over time, so the envelope is versioned — it already is, as the signed
[DR-0026](0026-session-root-agent.md) artifact whose grants append a new version.
Relaxing a protection must not make already-persisted data retroactively readable,
and tightening one must not silently invalidate in-flight work — it is re-checked
against the version it runs under. An envelope change is authored and locked by the
governance authority (D1) and ratified, never self-applied by the agent (D3); D4
adds that its *effect* is forward-only and version-scoped. It is a temporal
property, verified in TLA+ (see
[DR-0027](0027-information-flow-control.md) "Verification toolchain"), and it pairs
with [DR-0027](0027-information-flow-control.md) I-IFC7: persisted data carries the
label it was written under, so a later envelope change cannot relabel it in place.

### D5 — Two root-agent kinds, separated by OS privilege

> Authoring governance and authoring whips are split across **two separate
> session-root agents** ([DR-0026](0026-session-root-agent.md)). The **governance
> root agent** requires admin/sudo to run and is the only thing that may change
> policy; the **whip root agent** is unprivileged and authors only whips under the
> locked envelope. The OS privilege boundary is the enforcement.

This un-defers DR-0026's governance Option B (agent-assisted policy authoring) and
realizes it as a *privileged, isolated* agent rather than a feature of the whip
agent. It also strengthens, not weakens, DR-0026's "the agent cannot self-widen":
the whip agent has no path to the governor, and the governor is gated by sudo.

```text
                     governance root agent          whip root agent
run privilege        admin / sudo REQUIRED          unprivileged
talks to             the admin only                 the user
tool surface         edit gov DSL, gov compile,      author / spawn / run whips
                     read report, sign envelope      (DR-0026 + harness tools)
envelope authority   signs new versions (admin)     read / enforce only
input trust          admin input only                handles untrusted user/env input
its own bounds       fixed narrow surface + sudo     the signed envelope (acts-for user)
```

Four invariants:

```text
G1 privileged governor   running the gov agent requires admin/sudo; the whip agent
                         does not. OS-enforced.
G2 untrusted-input       the gov agent's context holds only admin input + governance
   isolation             artifacts, never untrusted user/environment data. Enforced
                         by process + privilege separation, not convention. This is
                         the structural form of the isolation a standing governance
                         agent requires to be safe.
G3 narrowly bounded      the gov agent's authority is a FIXED tool surface (edit DSL
   governor              / compile / sign): no whip authoring, no protected-data
                         access, no arbitrary egress. It changes policy, never runs
                         workloads. Its own setup (provider, tool surface) is
                         admin/install-owned and sudo-protected, not self-modifiable
                         -- which stops the who-governs-the-governor regress.
G4 single signer         only the gov agent, with admin authority (= the sudo
                         identity = the attestation key, DR-0028 trust root), signs a
                         new envelope version; the whip agent only reads and enforces.
```

The one flow from the whip side to the governance side is **escalation** (a user's
whip needs an ungranted right). Because that request may be shaped by untrusted
input, it crosses as **low-integrity data the admin reviews** (DR-0027 integrity
applied to the governor itself): the gov agent may display it but never auto-acts on
it; the admin decides. This preserves G2 — the escalation channel is not an
injection path into the privileged agent.

Out of scope (a deployment/documentation concern, not a system guarantee): *which*
LLM provider each agent is wired to. WhippleScript enforces declared provider
clearances (DR-0027 provider egress) but cannot verify a provider actually protects
data — that trust is the admin's configuration choice. For the gov agent, whose
context is the security policy itself, the provider choice is the admin's; we note
the consideration, we do not mandate it.

## Why this split

- It is the only structure that satisfies both cases without two mechanisms. Both
  reduce to "a locked authoritative envelope the kernel enforces, plus inline
  refinements proven to refine it"; the cases differ only in who drafts the
  envelope (an external data owner, or the agent from NL intent then
  human-ratified).
- It places the guarantee where it can be kept. Authority is the one thing an
  agent must not hold over data it serves but does not own; binding authority in a
  locked envelope and proving usage refines it is what makes the guarantee survive
  a confused or injected agent.

## What this record does not decide

```text
- The envelope's concrete schema and on-disk form: DRAFTED in the
  [governance lifecycle spec](../information-flow-governance.md) — one signed
  envelope extending DR-0026, authored via a single DSL surface, compiled to a
  signed artifact, attested like a package lock (trust-root option C), versioned
  and bound to runs (D4). Remaining-open within it: the attestation record's exact
  format and the gov DSL grammar.

- The natural-language-to-policy authoring UX. The assisted-authoring path is IN
  v1 — it runs on the sudo-gated governance agent (D5), with the admin drafting via
  NL or by hand and signing (see the
  [governance lifecycle spec](../information-flow-governance.md)). What remains open
  is the detailed NL-to-diff rendering and the ratify interaction, not whether it
  exists.

- The refinement check itself (inline ⊑ envelope) as a compiler pass: deferred to
  DR-0027's label-algebra and checking step; this record locks that it MUST hold,
  not how it is computed.

- Surface syntax for roles, source labels, and crossings: deferred to DR-0027's
  syntax step.
```

## Deferred capabilities (recorded hooks, not v1)

```text
decentralized ownership   multiple self-managing owners, each with declassify
                          authority over their own data and no authority over
                          others'. v1 is a single governance authority (D1).
                          Asymmetric delegation accommodates it additively.

cross-envelope flow       data crossing between two separately-governed envelopes
                          (two organizations). Needs an inter-envelope trust
                          agreement; out of v1, which is one envelope per session
                          root (DR-0026).
```

## Consequences

- Policy is two tiers: a locked authority envelope plus inline usage proven to
  refine it; there is always a locked envelope (D1).
- Authority decisions (hierarchy, delegation context, ownership, protected-from,
  downgrade rights) live in governance; usage lives inline and inferred; the
  compiler proves usage refines the envelope (D1, DR-0027 I-IFC4).
- Source references roles; governance binds concrete parties and the delegation
  context; programs are principal-polymorphic (D2).
- The agent acts for its user and holds only that user's authority; trust required
  equals authority delegated; Case 1 is guaranteed-safe with no trust over
  protected data, Case 2 is trust-but-verify over the user's own data, and both
  are authored on the sudo-gated governance agent (D3, D5).
- The agent may propose but never self-apply an envelope change; the human or
  external authority ratifies, the kernel enforces (D3).
- Envelope changes are versioned and forward-only: they do not retroactively
  authorize past flows, and in-flight work is bound to and revalidated against its
  envelope version, never silently re-permitted (D4).
- Governance and whip authoring split across two session-root agents separated by
  OS privilege: a sudo-gated governance agent that alone may sign policy, and an
  unprivileged whip agent; untrusted-input isolation is enforced by the privilege
  boundary, escalation crosses as low-integrity data, and the governor is bounded
  by a fixed admin-owned surface (D5, G1–G4). Provider trust is the admin's
  documented config choice, not a system guarantee.
