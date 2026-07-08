# `std.messaging`: communication channels and generic messages

Status: spec drafted 2026-06-14 from package design discussion
([`0007-core-standard-libraries-and-providers.md`](decision-records/0007-core-standard-libraries-and-providers.md)).
Stage: spec -> modeling -> implementation + testing -> review.

> **Reserved-class prerequisites:** `send` lowers to an ordinary durable effect =
> `capability_call` (**already package-authorable**); `source interaction` uses
> `signal_source` in the `source_declaration` family (**LANDED, Stage P1b**). So
> messaging needs **no new authorable lowering class** (it does *not* require the
> `typed_effect_call` promotion). Outstanding construct-side item: the `channel`
> reserved-keyword grant + the `Message` envelope schema. Runtime providers are
> tracked under the runtime stages.

## Framing

**Messaging is for talking through communication platforms.**

It is narrower than ingress. Messaging sends and receives generic communication
envelopes: text, markdown, sender, thread, attachments, interactions, and provider
metadata. It does not claim that Slack, GitHub comments, email, or desktop
notifications can directly produce arbitrary domain types.

The core shape is:

```text
channel  = configured communication route
send     = outbound durable effect through a channel
message  = inbound durable observation from a channel
```

`std.messaging` should replace the older `std.inbox` / `std.human` framing.
Human review is a use case of messaging plus ingress or coercion; it is not a
separate language primitive.

## Relationship To Ingress

`std.ingress` is the typed outside-input substrate:

```text
outside delivery -> declared signal payload -> typed fact
```

`std.messaging` is the communication-platform layer:

```text
send text/markdown/interactions out
observe generic Message envelopes coming back
```

Inbound messaging may be implemented as an ingress provider internally, but the
author-facing value is still a generic `Message`, not an arbitrary domain type.
Converting a message into a domain fact is explicit:

```whip
when message from release_room as msg
  coerce msg.text -> ReleaseDecision as decision
```

or, for providers with native interactions, by explicit mapping into a
declared `signal`:

```whip
signal release.decision {
  issue_id IssueId
  decision "approve" | "hold"
  actor string
}
```

Messaging talks. Ingress types outside facts.

## Source Surface

### Channel Declaration

A `channel` is a named route through a provider:

```whip
use std.messaging

channel release_room {
  provider slack
  workspace ops
  destination "#release"
}
```

The declaration is package-owned. The platform catalog reserves the bare
`channel` construct shape for `std.messaging` so third-party packages cannot
silently create channel-like semantics with weaker guarantees.

Provider config may fix the destination, allow dynamic recipients, or expose a
threading model. Secrets and credentials are references, never literal source
values.

### Send

**Status: SHIPPED 2026-06-20** (outbound). `send via <channel> { text … [markdown …]
[thread_id …] } as <binding>` parses and lowers to a `messaging.send`
`capability.call`. The construct is provided by the embedded `std.messaging`
package manifest: `use std.messaging` authorizes it with no package lock (the
binary is its own supply chain — not a third-party supply-chain construct; see
`models/maude/std-construct-authorization.maude`). The named
channel must be declared (`send via <unknown>` is a compile error). Under the
fixture provider the effect records a delivery receipt; live providers below are
the roadmap. Inbound `when message from <channel>` still needs a runtime
messaging provider and remains deferred.

`send` creates a durable outbound message effect:

```whip
rule announce_ready
  when issue ready as issue
=> {
  send via release_room {
    text "Issue {{ issue.id }} is ready for review."
  } as sent
}
```

The effect succeeds when the provider accepts the outbound message for delivery
under its contract. It does not mean a person read it, acted on it, or replied.

The minimal content contract is text. Providers may also support markdown,
attachments, threads, mentions, urgency, and native interactions.

### Receive

Inbound provider observations are recorded as generic message facts:

```whip
rule read_release_room
  when message from release_room as msg
=> {
  coerce msg.text -> ReleaseDecision as decision
}
```

The message envelope is intentionally generic:

```text
message_id
channel
provider
received_at
sender
sender_claims
thread_id?
text?
markdown?
attachments[]
interaction?
raw_ref?
correlation?
```

Provider-specific payloads are stored as bounded evidence or `raw_ref`, not as
untyped workflow facts.

### Interactions

Some providers support native buttons, commands, links, reactions, or forms.
Messaging can expose those as presentation affordances, but their semantics
remain provider-bounded:

```whip
send via release_room {
  text "Ship {{ issue.id }}?"
  interactions {
    approve kind button label "Approve" data {
      issue_id issue.id
      decision "approve"
    }
    hold kind reaction label "Hold" data {
      issue_id issue.id
      decision "hold"
    }
  }
} as prompt
```

An interaction is a structured inbound communication event correlated to a
message. A button click, command reply, reaction, link callback, or form submit
is first an inbound `Message` with an `interaction` field and correlation
metadata.

The outbound `send` does **not** emit a typed signal. It only asks the provider to
present affordances and records enough correlation/interaction evidence for a
later callback to be authenticated and interpreted.

Typed mappings are separate source declarations over later callbacks:

```whip
signal release.decision {
  issue_id IssueId
  decision "approve" | "hold"
  actor string
}

source interaction as release_decisions {
  channel release_room
  interactions [approve, hold]

  observe as interaction
  emit release.decision {
    issue_id interaction.data.issue_id
    decision interaction.data.decision
    actor interaction.sender.id
  }
}
```

`source interaction` is a source provider contributed by `std.messaging`. The
construct is a member of the `source_declaration` family (alongside
[`signal_source`](event-ingress.md) and [`clock_source`](std-time.md)), so it
appears in the construct-graph and lowering reports under the same family. It
uses the same source/observe/emit admission discipline as `std.ingress` and
`std.time`: a later provider callback becomes a typed signal only after the
runtime validates provider identity, correlation, interaction name, and the emitted
signal payload, and that admission carries the identity defined in
[`admission-and-idempotency.md`](admission-and-idempotency.md) (the external-signal
key: provider delivery/action token if present, else the derived per-source key).
Providers that cannot report authenticated correlated callbacks cannot satisfy
this source declaration.

### Deferred Request Sugar

`request` may exist later as ergonomic sugar, but it must lower transparently to
`send` plus either generic message observation or declared signal mappings. It
must not introduce a second lifecycle where workflows wait on hidden replies.

No source syntax is accepted for `request` in the current package contract. If it
is added later, the compiled shape must still be visible outbound delivery plus
later inbound `message` or declared `signal` observation.

## Provider Capability Report

Every channel provider must report the features it supports:

```text
direction: outbound_only | inbound_only | bidirectional
content: text | markdown | json | attachments
addressing: fixed_destination | dynamic_recipient | thread_reply
identity: anonymous | claimed_actor | verified_actor
delivery_receipts: accepted | delivered | failed
correlation: none | provider_message_id | thread_id | action_token
interactions: none | links | buttons | commands | forms | reactions
source_mapping: none | message_envelope | interaction_callback
hosting: none | local_cli | local_daemon | hosted_server | provider_webhook
```

The compiler/checker can admit syntax only when the selected provider capability
report supports it. For example, desktop notifications are outbound-only and
cannot satisfy a `when message from channel` rule or `source interaction`
declaration.

## Initial Provider Scope

The initial `std.messaging` provider set should stay deliberately small:

```text
std.messaging.local    bidirectional store-backed local mailbox
std.messaging.desktop  outbound-only desktop notifications
std.messaging.stdio    bidirectional development/test message stream
```

### `std.messaging.local`

The reference provider. It is backed by the WhippleScript store and local CLI,
works in CI, and replaces the current inbox use case without making "human" a
language concept.

It should support:

```text
outbound text/markdown
inbound generic Message envelopes
fixed local mailbox destinations
local actor identity supplied by CLI/config
provider message ids for correlation
accepted/failed delivery status
```

### `std.messaging.desktop`

An outbound-only provider for OS desktop notifications. It exists to prove that
one-way channels are valid first-class channels.

It should support:

```text
outbound text
optional markdown stripped/rendered to text
accepted/failed delivery status
no inbound messages
no interaction-to-signal mapping in the initial scope
```

### `std.messaging.stdio`

A development/test provider that exchanges generic `Message` envelopes over a
line-oriented process boundary. It is useful for fixtures, simple harnesses, and
manual experiments, but it is not a production identity model.

It should support:

```text
outbound text/json message envelopes
inbound JSONL Message envelopes
claimed actor identity only
provider message ids when supplied
accepted/failed delivery status
```

Deferred providers:

```text
Slack
GitHub issue/PR comments
email
Linear/Jira comments
chat systems
HTTP messaging
```

Those providers are useful, but their auth, identity, threading, interaction, and
delivery semantics are too platform-specific for the first pass. HTTP belongs
primarily to `std.ingress` unless a later design finds a distinct messaging
need.

## Static Checks

- A `send` must target a declared channel whose provider supports outbound
  delivery.
- `when message from <channel>` requires a declared channel whose provider
  supports inbound observations.
- Message interaction syntax requires provider support for the requested interaction
  family.
- `source interaction` requires a declared channel, a declared signal, an
  interaction-callback source capability, and enough correlation/auth evidence to
  satisfy the signal contract.
- A provider with claimed-only identity cannot satisfy a rule or mapping that
  requires verified actor identity.
- `request` sugar, if added, must lower to visible `send` plus visible inbound
  message/signal observation.

## Non-Goals

- No generic "human" keyword or `askHuman` semantic primitive.
- No hidden reply wait lifecycle.
- No implicit natural-language parsing into typed domain values.
- No arbitrary provider JSON becoming workflow facts.
- No guarantee that outbound delivery implies reading, acknowledgement, or
  interaction.
- No requirement that every channel provider implement replies, interactions, or
  typed signal mappings.

## Lowering And Runtime Boundary

`send` lowers to an ordinary durable effect. Provider execution records delivery
attempts, provider ids, terminal status, diagnostics, and evidence like any
other effect.

Inbound messages are admitted through the runtime boundary and recorded as
facts with provider evidence. If a message maps to a typed signal, the runtime
performs signal validation before appending the signal fact. A provider may not
directly append arbitrary facts.

## Modeling Notes

- **Delivery/reply separation:** outbound accepted/delivered status is
  independent from later inbound messages.
- **Generic inbound type:** messaging produces `Message`, never a domain type,
  unless an explicit ingress mapping validates a declared signal.
- **Capability soundness:** source forms are accepted only when the channel
  provider reports the necessary feature.
- **No hidden liveness:** no rule is enabled by an unrecorded provider callback;
  all inbound observations are durable facts.
