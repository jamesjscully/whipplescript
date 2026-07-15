# `std.ingress`: typed signals from external sources

Status: spec revised 2026-06-14 from package design discussion
([`0007-core-standard-libraries-and-providers.md`](decision-records/0007-core-standard-libraries-and-providers.md)).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**Ingress is the typed boundary where outside observations become durable
WhippleScript facts.**

The runtime event log records everything, so the old source word `event` was
overloaded: it meant both an internal log record and an author-declared outside
input point. The source-level ingress primitive should be called a **signal**:

```text
outside observation -> source provider -> validated signal fact -> rules
```

The shared top-level construct is a **source**:

```text
source <provider> as <name> {
  provider-specific configuration
  observe as <binding>
  emit <declared.signal> { explicit mapping }
}
```

`std.ingress` contributes external source providers such as `cli`, `http`,
`stdio`, `file`, and `grpc`. `std.time` contributes the `clock` source
provider. All of them emit typed signals through the same admission boundary.
No source fires a rule directly.

The construct name behind the `signal`/`source` surface syntax is
`signal_source` (in the `source_declaration` construct family); this is the name
the construct-graph and lowering reports use. `std.time`'s clock source is the
net-new sibling construct `clock_source` in the same family (see
[`std-time.md`](std-time.md)).

This keeps replay clean: replay reads the recorded signal fact, not the network,
filesystem, clock, or process stream.

## Signal Declaration

A `signal` declaration names a typed outside input and its payload schema:

```whip
signal deploy.finished {
  service string
  status "ok" | "failed"
  commit string
}
```

A signal is an **admission contract**. It declares that a workflow may be driven
by this outside fact. The payload schema reuses the class-body field grammar and
may use ordinary WhippleScript boundary types.

Signal names are dotted and lowercase. PascalCase remains reserved for classes,
enums, and sum-type cases.

## Reaction

A declared signal gets the typed bare `when` form:

```whip
rule on_deploy_finished
  when deploy.finished as d
=> {
  tell sre "Deploy of {{ d.service }} finished: {{ d.status }}"
}
```

The binding is typed against the signal schema. Fan-out is the usual per-fact
rule multiplicity.

Undeclared dotted facts may still be matched through the lower-level
`when fact <dotted.name> as x` form, but they are untyped and should not be the
ordinary integration surface.

## Source Surface

### External Source

An external source explicitly binds the provider observation before mapping it
to a signal:

```whip
use std.ingress

signal github.issue_labeled {
  issue_id string
  label string
  title string
  body string
}

source http as github_issues {
  path "/github/issues"
  auth hmac secret github_webhook_secret
  correlate body.issue_id

  observe as delivery
  emit github.issue_labeled {
    issue_id delivery.body.issue_id
    label delivery.body.label
    title delivery.body.title
    body delivery.body.body
  }
}
```

The least-magic rule is: providers expose observation types; authors explicitly
map observation fields into declared signal fields. A provider payload shape is
never assumed to equal a workflow signal shape.

### CLI Admission

The minimal ingress implementation is a CLI admission boundary:

```sh
whip signal <instance> --name deploy.finished \
  --data '{"service":"api","status":"ok","commit":"abc"}'
```

The CLI validates `--data` against the declared signal schema with the JSON
ingestion rules. A malformed payload is rejected before any fact is recorded.

The CLI path has no provider delivery id, so its admission identity follows the
peer/CLI rule in [`admission-and-idempotency.md`](admission-and-idempotency.md):
an operator-supplied delivery id (e.g. `--delivery-id`) is used when present;
otherwise the kernel derives one from the canonical payload hash and the target.
The admission records the operator/CLI origin as provenance. The store's unique
index on `events(instance_id, idempotency_key)` (migrations/0001) is what makes
a re-run of the same CLI admission append at most once; the CLI does not
implement its own dedup. (Amended 2026-07-15 per spec/std-ingress.md "Spec
amendments" 2: the index name here previously read "(instance,
fact_identity_key)" — same mechanism, wrong name.)

### In-Workflow Signal Injection

One workflow can inject a declared signal into another known instance:

```whip
emit signal deploy.finished to peer.id {
  service s.service
  status "ok"
  commit s.commit
} as sent
```

This lowers to a durable effect that validates the payload against the target
program's signal declaration and appends the corresponding signal fact to the
target instance. It is directed fire-and-forget signal injection, not a source
provider, channel session, or synchronous request/reply.

Peer injection also has no provider delivery id. Its admission identity follows
the peer rule in [`admission-and-idempotency.md`](admission-and-idempotency.md):
the key is derived from the origin instance id, the origin effect idempotency
key, the target, and the canonical payload hash. Because the origin effect key is
stable, a retried injection (e.g. after a crash between send and ack) reuses the
same admission key, so the target appends the signal fact at most once. The
admission carries origin-instance/effect provenance so `whip` can attribute an
injected signal to its source workflow.

## Provider Scope

The initial `std.ingress` source providers are:

```text
std.ingress.cli    operator/local CLI signal admission
std.ingress.http   self-hosted HTTP/HTTPS webhook signal source
std.ingress.stdio  development/test JSONL signal source
std.ingress.file   file import/watch signal source
std.ingress.grpc   typed service-boundary signal source
```

Broker/topic adapters are deferred.

### `std.ingress.cli`

The baseline admission path. It validates explicit operator-supplied payloads
and appends typed signal facts. It needs no hosted process and should be the
reference path for tests and manual operations.

### `std.ingress.http`

A self-hosted HTTP/HTTPS source provider. It should support endpoint paths,
secret references, HMAC/bearer/shared-secret auth, deduplication keys,
payload-to-instance correlation, and transport-security configuration.

HTTPS is a mode of this provider, not a separate package. Production deployments
should use TLS directly or run behind a trusted TLS-terminating reverse proxy.
Cleartext HTTP should be limited to localhost development, test fixtures, or
explicitly declared behind-proxy deployments.

HTTP owns transport mechanics only. Product-specific meaning should usually
live in a domain package or explicit signal schema.

### `std.ingress.stdio`

A development/test source provider that reads typed signal deliveries from a
line-oriented process stream. The payloads are validated against declared
signals before admission. Identity is claimed unless wrapped by a stronger
harness.

### `std.ingress.file`

A local file import/watch source provider for workflows that receive outside
data through dropped files, export directories, or test fixtures. It should
deduplicate, validate, and record file evidence before appending a signal fact.

`std.ingress.file` is about outside observations: a file arrived, changed, or
matched a watch pattern. Reading or writing the file's content belongs to
[`std.files`](files.md), usually through a declared `file store`.

### `std.ingress.grpc`

A typed service-boundary source provider. This is useful when operators want a
stronger service contract than generic HTTP/HTTPS webhooks, but it should wait
until the HTTP/HTTPS and CLI paths have hardened the admission contract.

### Deferred

```text
broker/topic adapters
product-specific webhook adapters
chat/comment platform adapters
```

Broker/topic adapters are likely useful later, but they add delivery, offset,
consumer-group, and retention semantics that should not be pulled into the base
design yet.

## Relationship To `std.time`

`std.ingress` and `std.time` share the `source` construct family:

```text
std.ingress  external delivery sources
std.time     clock observation sources
```

Both produce typed signal facts. Neither fires rules directly.

## Relationship To `std.messaging`

`std.ingress` is lower-level and more powerful than messaging. It can produce
any declared typed signal because the author provides a contract.

`std.messaging` is for talking through communication platforms. Its native
inbound value is a generic message envelope: text, sender, thread, attachments,
interactions, and provider metadata. A messaging provider may feed ingress, but
only through explicit configuration, interaction mapping, or schema coercion.

## Relationship To `std.files`

`std.ingress.file` observes file arrivals and changes. `std.files` performs
deliberate content reads, writes, imports, and exports through file-store
resources. A common workflow is:

```text
file source observes path -> typed signal -> std.files import/read effect
```

```text
messaging talks
ingress types outside facts
```

For example, a future interaction-capable messaging provider can be used in two
ways:

```text
std.messaging receives generic Message envelopes from the provider
std.messaging contributes source interaction to admit typed ReleaseDecision signals
```

The first is conversational. The second is a typed integration boundary.

## Provider Contract

Every source provider must report:

```text
provider kind
observation schema
transport kind
transport security mode
supported auth modes
deduplication key source, if any
correlation strategy
payload normalization shape
delivery evidence shape
failure diagnostics
whether it can start instances or only target existing instances
```

The runtime must reject provider configuration that cannot satisfy the declared
signal's validation and correlation requirements.

## Static Checks

- A `signal` payload schema must type-check under the same field rules as
  classes.
- Signal names are dotted lowercase and cannot collide with another signal,
  class, enum, package import, or reserved keyword.
- `when <signal> as x` requires a declared signal and binds `x` to the signal
  payload type.
- A rule that reads a declared signal is live without `@external`; the signal
  declaration is the external producer contract.
- `source <provider> as <name>` splits across the M5 graduated ladder (amended
  2026-07-15 per spec/std-ingress.md "Spec amendments" 1): the provider KIND
  must be contributed by an embedded/locked manifest — a HARD check now that
  embedded manifests are live — while the `use <package>` import line itself
  is an ADVISORY lint (`lint.missing_ingress_import`), with lint→error
  escalation registered, not built.
- `observe as <binding>` binds the provider's declared observation schema.
- `emit <signal> { ... }` must materialize the declared signal payload type
  from the observation binding and other recorded values in scope.
- A source may emit only declared signals through the runtime admission
  boundary.
- Hosted ingress config must use secret references and must declare an
  instance-correlation strategy unless the delivery starts a new instance.

## Non-Goals

- No direct rule firing from sources.
- No provider-owned facts that bypass signal validation.
- No hidden conversion from generic messages into domain types.
- No source-level secrets.
- No synchronous request/reply lifecycle; that belongs to effects or explicit
  outbound messaging plus later signal/message observation.

## Modeling Notes

- **Replay safety:** replay depends only on the recorded signal fact, not on
  external redelivery.
- **Typed admission:** malformed payloads are rejected before any fact is
  recorded.
- **Exactly-once where possible:** admission identity for every signal path
  (external delivery, CLI, peer injection) is defined in
  [`admission-and-idempotency.md`](admission-and-idempotency.md): a provider
  delivery id when present, else the derived per-source key. The store's unique
  index on `events(instance_id, idempotency_key)` enforces append-at-most-once; a
  duplicate is absorbed and recorded as an observable duplicate diagnostic. This
  spec does not define its own dedup mechanism.
- **Correlation soundness:** an observation can target only the instance
  selected by its declared correlation rule.
- **No direct fire:** sources can append signal facts only through the runtime
  admission boundary.

## Implementation Note

The target design uses `signal`, `source`, and `emit signal`. No compatibility
alias is specified for retired terminology.
