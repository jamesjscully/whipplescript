# DR-0042 — Secret-free model egress broker for hosted placements

Status: accepted (2026-07-14; Jack accepted the GaugeDesk ADR 0085 architecture
and directed implementation after Cloud Home landed). Builds on DR-0024
(WhippleScript owns the brokered agent loop), DR-0027/0028 (provider egress is
IFC-checked authority), DR-0033 (the DO host drives sans-IO HTTP), and DR-0039
(Bashkit is native; non-bash capabilities are explicit brokers).

## Problem

The first live GaugeDesk DO placement maps an admitted provider binding to one
Worker secret. That proves provider admission and edge execution, but it cannot
realize a person's portable linked credential or a project-owned credential
without copying it into deployment-wide Worker configuration. It also conflates
provider identity with credential ownership and funding.

WhippleScript must continue to own provider request construction, exact endpoint,
tool semantics, response parsing, IFC, idempotency, and runtime evidence while a
host-owned account system resolves credentials only after admission.

## Decision

### 1. The DO supports two explicit provider realizations

After WhippleScript admission, the host realizes either:

- `worker-secret` — the transitional/service-managed realization using a named
  Worker secret; or
- `model-broker` — a secret-free realization using an authenticated host broker.

The signed policy epoch is the complete non-secret declaration for
`model-broker`: provider, model, endpoint, credential reference, and placement
are returned by WhippleScript only after admission. No deployment-wide provider
map repeats that tuple. The static Worker map remains only for an explicitly
configured transitional `worker-secret` realization.

The provider binding still must exactly match the WhippleScript-admitted binding
id and credential ref before either realization is selected. The provider id
alone never selects the realization.

### 2. The broker protocol carries the admitted request, not a second model API

Protocol `whipplescript.model-egress.v1` carries:

- the admitted opaque credential/funding reference;
- provider identity;
- the WhippleScript-constructed URL, non-auth headers, and JSON body; and
- the existing stable idempotency key when present.

The broker injects provider authentication immediately before egress and returns
the provider status/body plus an optional opaque reconciliation ref. It does not
construct prompts, choose models/endpoints, parse replies, admit tools, or mint
runtime evidence.

### 3. Brokered requests are provably secret-free

The provider client is instantiated with a fixed non-secret sentinel in every
auth-shaped field. Before contacting the broker, the Worker rejects any
auth/account header whose value is not that exact sentinel, removes all such
headers, and serializes the remaining request. Missing broker URL/token or an
invalid response fails as a transport error; it never falls back to direct
provider fetch.

The broker URL is deployment configuration. Remote brokers require HTTPS;
loopback HTTP remains available only for local conformance. The broker auth token
is used solely on the Worker→broker hop and never enters the model request body,
WhippleScript store, or diagnostics.

### 4. HTTP is the first realization; an outbound session is additive

The first slice is an authenticated HTTP broker endpoint with an injectable fake
transport for hermetic tests. A GaugeDesk Home can implement it behind its KMS and
credential resolver. Local development behind NAT may later bind the same request/
response protocol to an authenticated outbound session; that changes transport,
not runtime semantics or the broker envelope.

## Consequences

- Account/project secrets no longer need to become Worker bindings.
- The existing static map remains usable only when it exactly repeats and
  explicitly declares the transitional `worker-secret` realization.
- Managed upstream gateways and BYOK Home credentials share one runtime protocol
  while keeping different product funding/admission policies.
- The Worker can be tested with a deterministic broker and no live credential.

## Rejected alternatives

- **Put sealed credentials in the DO and let it decrypt.** Rejected: the runtime
  host would become coupled to every product account/KMS system.
- **Send the credential value alongside the turn command.** Rejected: durable
  runtime state and retries would carry secrets.
- **Let the broker choose the provider request.** Rejected: it would fork provider
  semantics and runtime evidence ownership away from WhippleScript.
- **Fallback to direct fetch when the broker is down.** Rejected: availability
  uncertainty cannot widen credential custody or egress.
