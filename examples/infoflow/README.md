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

---

## Limitations found hands-on

These are real gaps observed while building the examples, not hypotheticals.

1. **The workflow's own output is an unmonitored sink.** A rule can
   `read text from crm ... { complete result { content customer.content } }` and
   it is **not** flagged — `complete result` / `record` facts are not modeled as
   egress. Confidential data returned as the workflow result flows back to the
   caller (the root/parent agent) unchecked. This is the most significant gap:
   the governed sinks are only the declared *external* resources (files,
   channels, providers), not the result channel to the invoker.

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

5. **Inbound message triggers are not integrity sources.** `when message from
   <channel>` untrusted trigger data is not modeled as a read, so injection is
   only caught when untrusted data comes from a *file* read. (These examples model
   inbound email as files for exactly this reason.)

6. **`endorse` crossings are absent from the trusted-surface report.** Only
   `declassify` grants are surfaced for audit; the integrity hatch (`endorse`,
   raising untrusted → trusted) is invisible in the report, though it is at least
   as risky and arguably deserves more scrutiny.

7. **Clearing a provider marks it "confidential".** Labeling a provider
   `readable by Operator` (so a turn may ship data to it) makes the provider
   itself appear under "protected resources (confidential)" in the report — a
   confusing artifact, since a provider is a principal, not a secret.

8. ~~**The guarantee report does not verify the attestation.**~~ *Fixed while
   writing these examples:* the report now verifies a signed envelope first and
   prints `REFUSED: ...` for a tampered policy instead of rendering a guarantee
   computed from tampered labels.
