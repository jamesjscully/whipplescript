# External event ingress: typed events, injection, and hosted webhooks

Status: spec drafted 2026-06-10 from decided design
([`language-ergonomics-tracker.md`](language-ergonomics-tracker.md) C5).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**An authenticated external event becoming a durable typed fact that rules
react to is the event-sourced core, generalized.**

The runtime already has a working instance of this:
`human.answer.received` is an externally-surfaced event — a human submits an
answer, it lands as a durable fact, and rules react. An external event is the
same shape, generalized to any producer. And the reaction surface already
exists: `when fact <dotted.name> as x` (A3) matches arbitrary named facts.

So receiving external events needs almost no new *language*; the cost is the
*ingress mechanism*. The design splits along exactly that seam:

- **3a — language surface (this is core):** a typed event declaration, a
  `when` reaction, and a CLI injection primitive. No server.
- **3b — hosted webhooks (opt-in operations):** a long-running authenticated
  HTTP receiver, configured not coded. No new source syntax.

The discipline matches everything else: receipt of an event is impure, but
once recorded as a fact, replay is preserved (replay re-reads the fact, not
the network).

## 3a — Typed events, reaction, injection

### Declaration

An `event` declaration names an external event (dotted, lowercase, matching
the `when fact` convention and distinct from PascalCase classes) and its
payload schema (reusing the class body grammar):

```whip
event deploy.finished {
  service string
  status string
  commit string
}
```

An `event` declaration is the typed **ingress manifest**: every point at which
the outside world can drive a workflow is one `event` block. It also subsumes
the `@external` liveness escape for these cases — a rule reading a declared
event is automatically exempt from the dead-read lint, because the event *is*
the external producer, typed rather than tagged.

### Reaction

A declared event gets the typed bare `when` form; undeclared dotted facts keep
the untyped `when fact <name>` form:

```whip
rule on_deploy
  when deploy.finished as d
=> {
  tell sre as turn "Deploy of {{ d.service }} finished: {{ d.status }}"
}
```

`d` is typed against the event's schema. Fan-out is the usual per-fact rule
multiplicity.

### Injection

The CLI primitive lands a typed event as a durable fact:

```sh
whip notify <instance> --event deploy.finished \
  --data '{"service":"api","status":"ok","commit":"abc"}'
```

`--data` is parsed against the event's declared schema using the JSON
ingestion primitive ([`json-ingestion.md`](json-ingestion.md)) — events and
JSON ingestion are one mechanism: typed external JSON becomes a durable fact.
A payload that fails schema validation is rejected at the CLI boundary (the
event is not recorded), so a malformed external delivery cannot land an
ill-typed fact.

No server is required: the operator's existing gateway verifies a real webhook
and shells out to `whip notify`. The recorded fact is the source of truth, so
the path is replay-safe.

### In-workflow injection (the `notify` effect)

The same injection, turned inward, is an effect: one instance lands a typed
event in another known instance.

```whip
rule signal_peer
  when SomethingReady as s
=> {
  notify s.target event deploy.finished {
    service s.service
    status "ok"
  }
}
```

`notify <instance> event <name> { payload }` creates an effect that injects a
typed, schema-validated, durable event into the target instance. It is the
directed fire-and-forget primitive of the coordination model
([`coordination.md`](coordination.md#messaging-a-durable-tuple-space)) — still
"inject a durable event," not "open a channel," so it adds no liveness coupling
and stays replay-safe. For point-to-point with a durable, retained log, prefer
a `ledger` partitioned by recipient (a mailbox); use `notify` when a standing
ledger is not wanted.

## 3b — Hosted webhooks (config, not language)

Hosting webhooks is an opt-in runtime mode, configured the way providers are —
**secrets are references, never values**:

```sh
whip serve --webhooks --config hooks.json
```

`hooks.json` maps a declared `event` to an endpoint path and an auth strategy
(`hmac` / `bearer` / `shared-secret`) whose secret is a reference
(`env:DEPLOY_WEBHOOK_SECRET`, a keychain handle, a secret id). The server
verifies the signature, parses the body against the event schema, and lands
the fact exactly-once (delivery dedup by provider-supplied id where present).

This adds **no source syntax** — the `event` declaration in `.whip` is the
contract; exposure and auth are operator config, keeping secrets and the
public-service surface out of source files. It is a separable track that may
ship after 3a.

Deferred to 3b's own design pass, in priority order:

- **Correlation first.** Payload-to-instance correlation (a declared key in
  the event payload routes a delivery to the instance holding the matching
  fact — "ticket T-1 was updated" reaches the instance working T-1),
  generalizing the explicit `<instance>` of `whip notify`. This is the piece
  that removes external *bookkeeping* rather than just external *plumbing*:
  without it, the operator's gateway must maintain its own event→instance
  map. It outranks the hosting concerns below.
- An event that **starts** a new instance. (Initiation already has a front
  door — `whip run --input` seeds a typed fact — so this is sugar over an
  existing path, not a gap.)
- Operational surface: TLS, rate limiting / DoS posture, delivery retry
  semantics.

## Does this remove the need for extensions?

It right-sizes them; it does not remove them.

With external events in (3) + `exec` out + JSON typing
([`json-ingestion.md`](json-ingestion.md)), a large class of integrations that
would otherwise justify a plugin — "react to GitHub events, act via `gh`";
"receive a deploy callback, run a script" — needs **no extension code**, only
workflow source plus config. Integration flows toward the gated escape hatches
(`exec`, events-as-facts).

What extensions remain irreducibly for:

- **Providers** — agent backends (codex/claude/pi) are long-lived
  bidirectional sessions with streaming, cancellation, and evidence; a webhook
  cannot model them.
- **Synchronous in-process capabilities** — `call plugin.cap for x as y`
  returns a typed value into the same logical step; an async event push cannot.
  Many such capabilities collapse into `exec` + JSON-parse, but those needing
  in-process state, pooling, or a non-CLI SDK do not.

So the accurate framing: webhooks + `exec` + JSON make extensions **optional
for integration glue** and reserve the plugin machinery for agent providers
and genuine in-process capabilities — a sharpening of the philosophy, not a
removal of a subsystem.

## Static checks

- An `event` payload schema is a class body; the same field-type rules as
  classes apply.
- A declared event name is dotted and lowercase; a collision with a class
  name or another event is a check error.
- `when <event> as x` requires a declared event; the binding is typed against
  its schema. `when fact <name>` remains available for undeclared dotted
  facts (untyped).
- Reading a declared event satisfies the dead-read lint without `@external`.

## Dependencies

Reuses the `when fact` matcher (A3), the JSON ingestion primitive (C3) for
payload validation, and the `credentials_ref` config model (providers) for 3b
auth. 3a introduces one CLI verb and one declaration; 3b introduces a runtime
mode and a config schema, no source syntax.

## Modeling notes

- Replay safety: an injected event is recorded as a durable fact; replay
  re-reads the fact, never re-receives the delivery (property: trace is
  independent of redelivery).
- Typed ingress: a payload failing schema validation is rejected before any
  fact is recorded; no ill-typed event fact can exist (property over malformed
  payloads).
- Exactly-once (3b): a redelivered webhook with the same provider id lands one
  fact; dedup is observable in the event log.
- Liveness consolidation: a rule reading a declared event is live without
  `@external`; removing the event declaration re-triggers the dead-read lint.
