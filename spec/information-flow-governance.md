# Information-flow control — governance lifecycle and the signed envelope

Status: design, 2026-06-27. The governance half of the information-flow design:
how the config is authored, compiled, reported on, signed, versioned, and
enforced. Companion to the [source surface](information-flow-surface.md) (which
fixes what grants and labels look like) and to
[DR-0028](decision-records/0028-information-flow-authority.md) (the authority
model). Realizes DR-0028's open steps: the envelope's signed serialization and
the authoring path.

## Two roles, two agents, one lifecycle

Authoring governance and authoring whips are different activities by different
people — and they run on **two separate session-root agents** separated by OS
privilege ([DR-0028](decision-records/0028-information-flow-authority.md) D5):

```text
IT / admin  <-> GOVERNANCE root agent   sudo-gated; edits gov DSL, compiles, signs
user        <-> WHIP root agent         unprivileged; authors whips under the envelope
```

The governance agent is the assisted authoring path: the admin chats with it, it
drafts/edits the DSL and runs `gov compile`, and — with admin authority — signs.
IT may equally hand-author the DSL; both produce the same signed envelope, and both
require admin/sudo to sign. The governor is narrowly bounded (G3): a fixed surface
of edit-DSL / compile / sign, no whip authoring, no data access, no arbitrary
egress; its own setup is admin/install-owned, not self-modifiable. The whip agent
and all untrusted input it handles have **no path** to the governor (G2, enforced
by the privilege boundary). IT sets policy infrequently; users work under it freely.

```text
1. IT writes the governance DSL (the single surface)
2. `gov compile` returns a GUARANTEE REPORT: the data-flow invariants this config
   guarantees, plus flagged risks
3. IT reviews, iterates DSL <-> report, then SIGNS as admin
   -> the envelope is active for that user account
4. user talks to agent; agent drafts a whip
5. `whip check` runs the whip against the active signed envelope
6. violation -> a very helpful error with routes to fix (self-serve, mostly)
7. OK -> compiles; the agent can run it
   IT is NEVER in the per-whip loop
```

IT approves **capabilities** (up front, rarely); users **exercise** them without
supervision. The only time IT re-enters is a policy-level grant change (step 6
escalation, below) — never an individual whip.

## One surface: DSL authored, signed artifact enforced

There is exactly **one governance surface**: the readable DSL
(`grant`/`party`/`delegate` from the [surface spec](information-flow-surface.md)).
It compiles to a signed canonical artifact that **a human never edits** — the same
relationship as whip source to IR, or a package manifest to its lock. IT reads and
writes the DSL and the report; the kernel reads only the signed artifact.

## The signed envelope: one artifact, extending DR-0026

The IFC content is **not** a separate artifact — it extends the
[DR-0026](decision-records/0026-session-root-agent.md) policy envelope. The
"a grant is binding + authorization + label in one statement" unification means
labels ride on the same resource grants DR-0026 already carries, plus a few
top-level sections (roles, parties, the delegation context, downgrade grants).
One signed envelope, one version, one signature, bound to the user account it
governs.

```text
{
  "envelope_version": 7, "epoch": "...",            # monotonic (DR-0028 D4)
  "account": "user-acct-id",                         # the account this governs
  "principals": {
    "roles":   ["Operator","Requester","Approver"],
    "parties": [{"identity":"bob@acme.com","roles":["Requester"]}],
    "providers":["onprem-llm","public-gpt"]
  },
  "delegations": [ {"from":"provider:onprem-llm","to":"Operator","axis":"confidentiality"} ],
  "resources": [
    {"handle":"Ledger","id":"file:/srv/ledger.db","capability":"file_store:rw",
     "confidentiality":{"readers":["Operator"]}, "integrity":{"vouchers":["Operator"]}},
    {"handle":"Intake","id":"imap:ops@acme.com","capability":"channel:inbound",
     "integrity":{"vouchers":["stranger"]}}
  ],
  "downgrade_grants": [
    {"op":"endorse","scope":"Intake/PaymentRequest","to":"Requester"},
    {"op":"declassify","scope":"Ledger","to":"Requester"}
  ],
  "attestation": { "envelope_hash":"...", "approved_by":"...", "at":"..." }
}
```

## `gov compile`: the guarantee report

The report is the centerpiece — it is how IT sees *exactly what is protected and
what is not* before signing. Two parts, both in IT-legible terms.

**Guaranteed invariants** — the realized non-interference, per protected resource:

```text
file:/srv/ledger.db  (Operator-only):
  reaches NO sink except Operator-cleared ones, EXCEPT the declassify->Requester
  grant, bounded to type Receipt (<= |Receipt| released per use)
imap:ops@acme.com  (stranger-integrity):
  cannot drive any control sink above 'stranger' EXCEPT the endorse->Requester
  grant, bounded to type PaymentRequest
```

**Flagged risks** — allowed, but worth IT's eyes:

```text
- provider:public-gpt is cleared for Operator data via delegation — intended?
- channel Intake is open (from stranger) and feeds an endorse grant
- file:/tmp/** is UNLABELED — governed data may flow there freely (coverage gap)
- agent Reviewer is bound to a provider with no Operator clearance — secret-touching
  turns will be redacted or rejected (usability risk)
```

IT iterates the DSL against the report until the guarantees match intent and the
risks are acknowledged, then signs. The report is both the proof (what is provably
protected) and the audit (what is exposed). It is the same realized-protection
artifact the [surface spec](information-flow-surface.md) describes, surfaced to the
governance author.

## Enforcement: `whip check` and the routes-to-fix error

A whip is checked against the active signed envelope. A violation produces a
helpful error whose routes split in two:

```text
SELF-SERVE (no IT) — the three resolutions:
  separate    read the secret in a distinct cleared turn; pass only its result
  cleared     use an already-cleared provider / sink
  downgrade   declassify / endorse using a right IT ALREADY granted

ESCALATE (rare, policy-level):
  "needs a declassify grant for Ledger->Auditor that governance has not issued —
   request it from your admin"
  -> IT signs a new envelope version once; every future whip uses it freely
```

Most fixes are self-serve. Escalation touches the **policy**, never the whip, and
is infrequent. Escalation is the **only** flow from the whip side to the governance
side, and it crosses as **low-integrity data the admin reviews**
([DR-0028](decision-records/0028-information-flow-authority.md) D5): the governance
agent may display the request but never auto-acts on it — the admin decides — so the
channel cannot become an injection path into the privileged governor.

## Versioning, binding to runs, discovery

- **Monotonic version.** `envelope_version` increases on each signed change; the
  attestation covers the canonical form.
- **Bound to runs (D4).** A run records the `envelope_version` it began under
  (alongside `program_version`/`revision_epoch` already in the effect key and run
  metadata). An envelope change applies to **new** runs only; in-flight work
  continues under its start version — non-retroactive, for free.
- **Discovery.** Mirrors the package-lock pattern: workspace discovery plus an
  explicit flag. **No envelope present = ungoverned dev mode** — a plain whip that
  makes no safety claim.

## Trust root: why sign, and how

The signature **binds kernel enforcement to IT's approval**, so the powerful agent
cannot swap in a permissive envelope and self-widen (DR-0026/0028's core
invariant). The kernel honors *only* an approved envelope. Three options:

```text
(A) cryptographic signature, IT holds a key
    +  strong tamper-evidence; cross-machine; agent cannot forge (no key); portable audit
    -  key management / rotation; most infra
(B) OS/filesystem permission (envelope writable only by the admin account)
    +  simplest; no crypto; fine on one box
    -  no cross-machine; trust = "who can write the file"; misconfig or compromised
       admin account = silent bypass; no portable audit
(C) hash + approval record (attestation) — REUSE the package-lock/attestation infra
    +  reuses what already attests package locks; append-only audit; tamper-evident;
       one mechanism, consistent with packages
    -  the approval log itself needs protecting; cross-machine needs it shared
```

**Decision: (C) for v1** — the envelope is attested like a package lock; IT
approval is an attestation entry binding the envelope hash to the IT identity in
the append-only store. Approval authority is the **admin/sudo identity** that runs
the governance agent (DR-0028 D5 G4). **(A) is the hardening upgrade** for
cross-machine custody or a stronger trust root. **(B) is rejected as the trust
root** (too easy to misconfigure into a silent bypass), though "no envelope =
ungoverned dev" is fine because dev makes no safety claim.

## Out of scope: which LLM provider you trust

WhippleScript enforces *declared* provider clearances (DR-0027 provider egress: a
provider is a principal, cleared via delegation, and governed data may not egress to
an un-cleared one). It **cannot verify** that a provider actually protects the data
it receives — retention, logging, and training are the provider's behavior, not
ours. **Which** LLM provider each agent is wired to is therefore the admin's
configuration choice, documented here as a consideration rather than a system
guarantee. This applies with particular force to the governance agent, whose context
is the security policy itself: choosing its provider is the admin's call. We note it;
we do not mandate it.

## Deferred

```text
NL authoring UX (the assisted-authoring detail)
    the governance agent's assisted authoring is IN v1 (DR-0028 D5); what remains
    open is the detailed NL-to-diff rendering and the ratify interaction, not
    whether it exists. The whip agent still drafts only whips.
multi-account / cross-account governance
    one envelope per user account in v1; sharing/inheriting policy across accounts
    is a later concern.
```

## What this spec does not decide

```text
- the guarantee report's exact schema and renderer (its CONTENT is fixed here).
- the attestation record's concrete format (reuses the package-attestation layer;
  shape inherited from there).
- the gov DSL grammar details (the surface spec fixes its shape; a grammar doc
  follows when implementation begins).
```
