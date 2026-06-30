# Information-flow control — worked examples

These examples exercise the IFC checker (DR-0027 / DR-0028) on a realistic
scenario: a **support-triage assistant** that touches the classic "lethal
trifecta" — confidential data, untrusted input, and an egress channel — and show
both where the system earns its keep and where it currently falls short.

The governance policy labels the **real data sources** in the environment (not
variables in any whip). IT signs it once; authors then write whips freely and the
compiler holds them to it.

```
file store crm          readable by Operator,  from Operator   # confidential PII, trusted
file store inbox         readable by public,    from public     # attacker-controllable email
file store audit_log     readable by public,    from public     # untrusted-OK event log
channel   public_reply   readable by public,    from public     # reply back to the customer
provider  fixture        readable by Operator,  from Operator   # in-house model, in trust domain
```

A label is not limited to one role. `readable by` (and `from`) accept a **set of
compartments**, comma- or space-separated, and a party may read the resource only
if it is cleared for **every** one of them (the intersection — combining secrets
*restricts*, it never widens):

```
file store mixed   readable by Bank, Email   # readable only by a party cleared for BOTH
```

A flow is safe when the sink's reader set **dominates** the source's — every
compartment that gates the source is covered by some compartment of the sink, so
no reader of the sink is un-cleared for the source. A single-compartment label is
just the one-element set, behaving exactly as the role it names. The integrity
axis (`from`) is the dual: a sink requiring `from Sec, Ops` accepts data only from
a source that provides a voucher acting-for each. (DR-0027 E6; the set algebra is
machine-proven in `models/lean/Whipple/ReaderSets.lean` and modeled in
`models/maude/infoflow-reader-sets.maude`.)

`whip check` discovers the policy via `WHIPPLESCRIPT_IFC_ENVELOPE`. With no
envelope set, a whip is **ungoverned** (dev mode) and makes no IFC claim — the
checker imposes nothing.

## Running them

```sh
# Dev mode: no envelope, no IFC constraints.
whip check examples/infoflow/support-triage-unsafe.whip

# Governed: the unsafe whip is REJECTED (2 violations).
WHIPPLESCRIPT_IFC_ENVELOPE=examples/infoflow/governance.policy \
  whip check examples/infoflow/support-triage-unsafe.whip

# The safe whip PASSES the same strict policy (0 violations, no hatches).
WHIPPLESCRIPT_IFC_ENVELOPE=examples/infoflow/governance.policy \
  whip check examples/infoflow/support-triage-safe.whip
```

## What the checker catches (demonstrated)

**1. Confidentiality leak — confidential data to an untrusted recipient.**
`support-triage-unsafe.whip` reads `crm` and emails it out on `public_reply` in
the same rule:

```
information-flow violation in rule `triage`: it may read `crm` (readable by
Operator) and write `public_reply` (readable by public), so data from `crm`
could reach a party not cleared for it
```

**2. Integrity injection — untrusted input into a trusted store.** The same rule
writes attacker-controlled `inbox` content into the `crm`:

```
integrity violation in rule `triage`: it may let untrusted `inbox` (integrity
public) influence `crm` (requires integrity Operator), an injection into a
more-trusted sink
```

**3. Provider egress — the agent silently ships data to a model.**
`agent-egress.whip` has no file write and no channel send, only a `tell` that
reads `crm`. When the agent's provider is **not** cleared, the turn's context
egresses to an uncleared model:

```
provider-egress violation in rule `summarize`: a turn reads `crm` (readable by
Operator) but its provider `fixture` (clearance public) is not cleared, so the
turn's context egresses to an uncleared model
```

When governance clears `fixture` (the in-house model), the same whip passes —
governance, not the author, decides which providers are in the trust domain.

**4. Tamper-evident policy.** `governance.signed.json` is the policy signed by the
governance agent (privileged):

```sh
WHIPPLESCRIPT_GOV_ADMIN=1 whip gov sign examples/infoflow/governance.policy \
  > examples/infoflow/governance.signed.json     # privileged: succeeds
whip gov sign examples/infoflow/governance.policy # unprivileged: REFUSED
```

`whip check` enforces the signed envelope and **rejects a tampered one** (the
SHA-256 attestation no longer matches the content).

## The audited escape hatches (demonstrated)

When a crossing is genuinely intended, governance blesses it explicitly and it
shows up in the guarantee report's **trusted surface** for review.
`governance-with-hatches.policy` adds:

```
grant declassify crm to public      # the data subject is entitled to their own record
grant endorse inbox to Operator     # inbound is sanitized upstream before we trust it
```

Under that policy the *same unsafe code* now passes, and the report lists
`crm -> public` as audited trusted surface. The dangerous flows still exist — they
are now explicitly, auditably accepted rather than silently present.

## The safe shape (demonstrated)

`support-triage-safe.whip` does the same job and passes the **strict** policy with
**no hatches**, by separating contexts:

- untrusted `inbox` → the public-integrity `audit_log` (never the CRM);
- the confidential `crm` read happens in a rule with **no egress sink**;
- the reply carries only public, bounded content (the ticket id).

This is the point: the system permits the maximally-permissive *safe* structure —
it is not just blocking everything.

## The guarantee report (`whip check` under an envelope)

Running `whip check` under a governed envelope prints an IT-legible **guarantee
report** (DR-0028). It states, in order:

- **guaranteed invariants** — one line *per governed resource* with the exact
  property proven on every rule, e.g. `crm: may not flow to a sink not cleared for
  Operator (unless an audited declassify clears it)`. Not a generic blanket line.
- **violations caught** — how many dangerous flows the envelope rejects in this whip.
- **flagged risks** — resources the whip touches that governance has *not* labelled;
  each defaults to public + low-integrity (fail-closed), so the operator must confirm
  it holds nothing confidential and feeds no trusted sink, or add a `grant`.
- **trusted surface** — every audited `declassify` / `endorse` crossing to review.
- **cleared principals** and the full **information-flow surface** (every door).

Each violation diagnostic names **two routes to fix**: a **self-serve** route the
whip author can take alone (separate the contexts / gate the sink on trusted data)
and an **escalate** route that needs a governance grant (`grant declassify …` /
`grant endorse …`) — mirroring the two-agent privilege split.

---

## Limitations found hands-on

These are real gaps observed while building the examples, not hypotheticals.

1. **The workflow's own output is a governed sink.** *`record` is governed (H2):* a
   recorded fact is a sink `fact:<schema>` (the DR-0026 stream and other rules observe
   it), defaulting to public/fail-closed — so reading `crm` and `record`ing a derived
   fact is flagged unless governance clears `fact:<schema>` (see `fact:Reviewed` in
   `governance.policy`). *`complete result` is now governed too (DR-0030 X2,
   top-level):* for a `@service` workflow it is an egress sink at the invoker boundary
   named by the output binding (e.g. `result`), default public/fail-closed — so a rule
   that reads `crm` and `complete`s a result is flagged unless governance clears the
   invoker (`grant … -> result readable by <role>`) or the contexts are separated (the
   safe shape). Whole-result v1: the result conservatively carries the join of the
   completing rule's reads (no per-field value-flow). *The cross-package `@tool` result
   is governed too (DR-0030 X2):* a turn whose agent may call an imported tool folds the
   tool's result reads into the turn — so a tool that reads confidential data the
   consumer never touched, whose result then egresses, is caught. The **Direction A
   reach refinement** keeps only the tool reads that reach a completing rule (inputs the
   result is `independent_of` are dropped); it is computed consumer-side from the pinned
   tool source. **Still deferred:** per-field value-flow (v2) — at whole-result v1 a
   tool that reads confidential data anywhere taints its whole result.

2. **Coarse, rule-level join box (no value tracking).** Any read in a rule is
   assumed to potentially reach any sink in the same rule. The safe refactor must
   physically split rules even when a human can see the value does not actually
   flow. This is the intentional conservative-join-box design (we do not trust the
   agent to be value-precise), but it is a real authoring cost.

3. **Diagnostic span is the whole rule.** Violations point at the rule's `=> {`,
   not the specific read/write lines, because the join box is rule-wide. With many
   effects in a rule, the author must hunt for the offending pair.

4. **Per-resource labels, not per-field/path.** You cannot say "the order-status
   field of the CRM is public but the SSN is confidential" — a label attaches to a
   whole `file store` / `channel`, not a path within it. Mixed-sensitivity stores
   must be physically split.

5. ~~**Inbound message triggers are not integrity sources.**~~ *Fixed (H3):* a rule
   triggered by `when message from <channel>` now treats the channel as a
   low-integrity read source, so attacker-controlled inbound content driving a
   more-trusted sink is caught as an injection — not only file reads.

6. ~~**`endorse` crossings are absent from the trusted-surface report.**~~ *Fixed
   (H4):* the trusted surface now audits BOTH axes — `declassify <r> -> <role>` and
   `endorse <r> -> <role>` — each tagged by axis.

7. ~~**Clearing a provider marks it "confidential".**~~ *Fixed (H5):* `provider`
   (and `human`) grants are tracked as **principals**; the report lists them under
   "cleared principals (providers/humans, not protected data)", not "protected
   resources".

8. ~~**The guarantee report does not verify the attestation.**~~ *Fixed while
   writing these examples:* the report now verifies a signed envelope first and
   prints `REFUSED: ...` for a tampered policy instead of rendering a guarantee
   computed from tampered labels.

9. ~~**Signal triggers are invisible sources (fail-OPEN).**~~ *Fixed (H8):* a rule
   triggered by `when <Signal> as e` now reads the governed resource
   `signal:<name>` — integrity envelope-declared, default `public`/low (fail-closed)
   — so an externally-injected signal driving a more-trusted sink is caught as an
   injection, just like an inbound channel message. Vouch a trusted signal with
   `grant signal <name> -> signal:<name> from <Role>`. Source recognition is now
   *uniform* (channels, human answers, and signals all governed alike); the signal
   also appears in the workflow's information-flow surface.

10. ~~**Internal signals must be hand-vouched.**~~ *Fixed (H8 stage b — emitter-carried
    integrity):* mark a signal an internal channel with
    `grant signal <name> -> signal:<name> internal`, and its integrity is **derived
    from its emitters** instead of defaulting low — an `emit signal X` carries the
    intersection of its emitting rule's read-source vouchers, and `when X` reads that.
    So you only hand-classify the *external entry points*; internal flows propagate
    the emitter's trust automatically (the labeling burden stays `O(external entry)`).
    Carriage spans packages: an imported `@tool`'s `emit signal X` contributes its
    carried integrity to a consumer's `signal:X`, computed under the consumer's own
    envelope from the pinned source. Soundness is preserved two ways — carriage never
    *fabricates* trust (an untrusted emitter yields an untrusted receiver), and
    `whip signal` **refuses** to externally inject an internal signal (no laundering
    untrusted data in under a trusted signal name).
