# Information-flow control — source surface and governance grants

Status: design, 2026-06-27. The syntax + data-shapes spec for information-flow
control ([DR-0027](decision-records/0027-information-flow-control.md)) and its
authority model ([DR-0028](decision-records/0028-information-flow-authority.md)).
Defines what governance grants look like, what (little) IFC syntax appears in
source, how real resources are identified, and how the provider-egress boundary
behaves. Grounded by the exploratory Maude models (`models/maude/infoflow-*`).

## Principles

- **Gradual / governance-relative.** A whip with no labels is a normal whip and
  runs as today. IFC is opt-in: when a governance config is set, the compiler
  rejects whips that would violate it. Safety afforded equals the governance
  config provided — transparently (DR-0027 I-IFC5; the refined I-IFC6).
- **Labels protect real resources, not script names.** Governance is authored
  over the environment — paths, endpoints, mailboxes, provider bindings,
  credentials — so protection follows the data no matter what a whip calls it,
  and an agent cannot rename or rebind its way out.
- **Source is almost label-free.** Sources, sinks, and the provider egress carry
  no source annotation; their labels live on the real-resource grants and
  propagate by inference. The only IFC syntax in source is the two explicit
  downgrade crossings.
- **Brokered enforcement.** Because the owned harness runs every effect (DR-0024
  I1), every real-resource access is a visible, governed boundary (DR-0027
  I-IFC8). There is no unbrokered door to the environment.

## Governance grants: binding + authorization + label, in one statement

Governance does not introduce a new resource namespace. A label is a **new
attribute on the resource grant the capability layer already uses** — the
identifier that authorizes access is the identifier that labels. One statement
binds a whip handle to a real resource, authorizes it, and labels it:

```text
grant file_store Ledger  -> file:/srv/ledger.db   readable by Operator
grant channel   Intake   -> imap:ops@acme.com     from stranger
grant channel   Reply    -> smtp:out              audience { Requester }
grant agent     Reviewer -> provider:onprem-llm
```

The whip stays a label-free handle (`file_store Ledger`); governance supplies the
real resource and its label. With no grant, the handle is a local, ungoverned
resource (dev default) and nothing is constrained.

### Resource identity: typed `kind:address`, reusing the grant layer

Real resources are named by a typed identifier `kind:address`, exact or glob,
matching how the capability/provider layer already addresses them:

```text
kind        address          construct family            role in the flow graph
file:       path / glob       file_store                  dual (read tags, write checks; durable)
mail: http: endpoint / URL    channel, web                inbound = source, outbound = sink
exec:       command / glob     exec                        output = source (low integ), args = sink
provider:   binding name       agent + coerce providers    dual (context-in = sink, output = source)
coord:      store / key        queue/ledger/counter/lease  dual (durable)
memory:     pool               std.memory                  dual (durable)
workflow:   name               workflow.invoke             typed crossing
identity:   principal address  human.ask, party bindings   sink (a party) + integrity source
```

Rules:

- **Bindings where they exist, globs where they do not.** A configured connection
  with a credential is labeled by its stable binding name (`provider:bank-api`); a
  raw filesystem or host is labeled by glob (`file:/data/**`), most-specific-wins,
  like the exec allow-list.
- **One label per resource; direction decides its use.** A read tags data with the
  resource label (source); a write checks the data is dominated by it (sink).
  Stores, providers, and memory are **dual-gated** — both — the same shape as the
  modeled `record` sink.

### Parties, roles, delegation

Roles are governance-owned abstract principals; parties bind concrete identities
to roles; the delegation context (asymmetric, per-axis) places principals —
including providers — in the lattice:

```text
party    bob@acme.com : Requester
delegate provider:onprem-llm acts-for Operator for confidentiality
grant    endorse   Intake/PaymentRequest to Requester
grant    declassify Ledger to Requester
```

## Source surface: three IFC touch-points, two of them the trusted surface

```text
coerce <v> into <Type>                  structural validation; integrity PRESERVED; free
coerce <v> into <Type> endorsed [by R]  raise integrity   — explicit, granted, audited
declassify <v> into <Type> [for R]      lower confidentiality — explicit, granted, audited
```

- **`coerce` is structural only.** Untrusted text becomes a typed value at the
  *same* integrity. Coercing a stranger's message yields a stranger-integrity
  value — well-shaped, still untrusted. A plain whip uses this freely. (This
  corrects the earlier "coerce is the endorse" framing, which violated I-IFC3's
  requirement that an integrity raise be explicit and authority-scoped.)
- **`endorse` and `declassify` are the only governed crossings.** Each is an
  explicit marker (so the trusted surface is visible in source), authorized by a
  governance grant, audited (the `endorsedAt`/`declassifiedAt` trail), and carries
  a **mandatory bounded `Type`** as its bandwidth ceiling.
- **Target comes from the grant, not the script.** A bare `endorsed` /
  `declassify` takes its target level / audience from the governance grant; the
  optional `by R` / `for R` only disambiguates when multiple grants could apply.
  Audience is environmental, so the script does not hardcode a party.

Sinks carry **no** source annotation: `send out via Reply` is checked against
`Reply`'s real-resource audience. Likewise the provider egress (below) needs no
source syntax.

## The construct / boundary audit: no ungoverned doors

The guarantee holds iff every environment-touching construct is a **modeled
boundary** visible to the analysis; brokered execution (DR-0024 I1) makes them all
visible, so the audit reduces to completeness of the construct -> boundary map.
The obvious doors (`file_store`, `channel`, `exec`, `signal`, `coord`, `memory`,
web) lower cleanly. The **five non-obvious doors** that must be explicitly modeled:

```text
provider endpoint   an agent turn ships its whole context to a real model API:
                    EGRESS to the provider + a LOW-INTEGRITY source on output.
                    The most important door (see below).
human.ask question  asking shows the party the text: EGRESS to that party; the
                    answer is a low-integrity source attributed to them.
telemetry / logs    std.telemetry OTLP export ships spans to a real collector: a
                    span carrying a data value is an egress.
session-event stream the DR-0026 observability layer is an egress to subscribers;
                    an event carrying governed data reaches whoever tails it.
fact-base / record  durable AND observable; the dual-gated sink already modeled.
```

An *unmodeled* door is the hole; for a *modeled* sink the sticky rule (below)
handles a missing label by rejecting. The anti-evasion deliverable is a checklist
that every construct above (a) lowers to a brokered effect and (b) is registered
as a source/sink/dual boundary of a `kind:`.

## The provider-egress boundary

The model endpoint is treated as a **principal** in the lattice; governance grants
it read-authority via asymmetric confidentiality delegation
(`delegate provider:onprem-llm acts-for Operator for confidentiality`). A public
API gets no delegation, so it is cleared only for public data. The model endpoint's
trust level **becomes policy**.

- **Incremental, brokered check.** Because whip brokers every tool, the context
  grows under its control. The egress check runs at the provider boundary, per
  model call: every value entering the context (initial input + each tool result)
  must be readable by the bound provider-principal.
- **Label-driven redaction.** DR-0024 redaction becomes policy-driven: the harness
  may automatically withhold/mask from the context any data the provider is not
  cleared for — a graceful-degradation path where policy prefers it to rejection.
- **Output is low integrity.** The model is an untrusted generator; turn output is
  the join-box meet of inputs and drives control only through `endorse`.

When a turn would egress uncleared data, the author/agent gets the three universal
IFC resolutions, with separation favored:

```text
(a) RAISE THE SINK   bind the agent to a cleared provider (names which exist, or
                     says none do, forcing b/c).
(b) LOWER THE DATA   declassify the bounded projection the model needs.
(c) SEPARATE         read the secret in a distinct cleared turn; pass only its
                     bounded result into this one (dual-LLM / CaMeL pattern).
```

**Clearance-based routing** is the capability this unlocks: bind an agent to a set
of providers and let the runtime pick the cheapest one cleared for the current
context (public turns to a cheap public model, secret turns to on-prem),
automatically and provably. Powerful but adds runtime/determinism considerations;
flagged as a candidate, v1-or-deferred TBD.

## The sticky boundary, and the realized-protection report

Soundness of the gradual model rests on the governed surface being **sticky**:

> Governed data may flow only to real sinks **cleared for it**. A flow to an
> unlabeled or under-cleared real sink is **rejected**, and the diagnostic names
> the real resource to label: *"file:/srv/ledger.db (Operator-only) reaches
> smtp:out (audience Requester) — release requires a declassify, or relabel the
> sink."*

Transparency ("clear what is protected and what is not") is two parts: the
**declared policy** (the grants) plus a compiler-**derived realized-protection
report** — given (governance config + whips), the enumerable trusted surface:
which resources are protected, and every granted hole with its type-bounded
bandwidth. The report is the part-(b) audit set, surfaced to the governance
author.

## Worked example

```text
# --- source ---
channel Intake
file_store Ledger
agent Reviewer
when message m on Intake {
  coerce m into PaymentRequest endorsed -> req     # endorse: granted, audited
  ask Reviewer to review req against Ledger -> note # turn egress checked vs Reviewer's provider
  declassify note into Receipt -> out               # declassify: granted, audited
  send out via Reply
}

# --- governance (binds + authorizes + labels real resources) ---
grant file_store Ledger  -> file:/srv/ledger.db  readable by Operator
grant channel   Intake   -> imap:ops@acme.com    from stranger
grant channel   Reply    -> smtp:out             audience { Requester }
grant agent     Reviewer -> provider:onprem-llm
grant endorse   Intake/PaymentRequest to Requester
grant declassify Ledger  to Requester
delegate provider:onprem-llm acts-for Operator for confidentiality
party   bob@acme.com : Requester
```

With no governance config this is a normal whip. With it, the compiler: enforces
Operator-only on `Ledger`; treats the `endorsed` as the granted Requester-endorse;
checks the `Reviewer` turn may receive Ledger data (onprem-llm is Operator-cleared);
requires the `declassify` before `send`; and rejects any path that would leak
`Ledger` to `Reply` un-declassified.

## Invariant impact

Two refinements this surface forces, recorded in DR-0027:

- **I-IFC6** is governance-relative, not global totality: soundness is relative to
  the governance config (an empty config protects nothing), and fail-closed
  applies **at the boundary of the governed surface** — governed data may not reach
  an un-cleared or ungoverned sink.
- **I-IFC3** distinguishes structural `coerce` (integrity-preserving, free) from
  the explicit, authority-scoped, audited `endorse`.

## What this spec does not decide

```text
- clearance-based provider routing as v1 vs deferred (runtime/determinism cost).
- role-generics: concrete role names only in v0; parameterized roles for reusable
  patterns deferred.
- the governance config's on-disk/signed form and the NL-to-policy authoring path
  (DR-0028 open steps); this spec fixes the grant SHAPE, not its serialization.
- per-field schema labels beyond the channel/store grain; allowed as opt-in, with
  the join-box caveat (field precision collapses at an agent turn).
```
