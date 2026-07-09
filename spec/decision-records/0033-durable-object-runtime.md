# DR-0033 — Sans-IO durable-object runtime

Status: accepted (2026-07-03). Phase 0 of the durable-object-runtime effort.
Formalizes the seven decisions previously scattered in the tracker's "Decisions"
section, which now defers to this record. Formal model:
`models/tla/ResumableEffectLifecycle.tla` (in the `scripts/check-tla-models.sh`
gate). Durable tracker: `spec/durable-object-runtime-tracker.md`. Shares seams
with DR (workflow-encapsulation) — see § Cross-effort ordering.

## Problem

The whip evaluation core — parser, kernel, rule/flow engine, effect ledger — is
already pure, synchronous, and network-free. The only blocking I/O lives inside
the ~15 `run_*_effect` handlers in the worker executor (`ureq` HTTP, `rusqlite`,
`std::fs`, subprocess sidecars). We want the *same core* to run on a second host:
a **Cloudflare Durable Object** — a single-threaded wasm32 isolate whose only
async primitive is `fetch()`, whose storage is synchronous DO SQLite, whose timers
are alarms, and whose config is Worker secrets. The isolate can be **evicted**
mid-request and resumed later against durable state.

The constraint is open-core seam discipline: this must be a **second host binding
behind core-defined seams**, not a fork. The native CLI (blocking `ureq` + OS
threads + `rusqlite` + `std::fs`) must keep behaving exactly as it does today, and
every existing gate must stay green through the native-only refactor phases.

Two design forces are in tension. Wasm isolates want `async`/`await` around every
`fetch`; colouring the Rust executor spine `async` would tax the whole core with
futures, `Send`/`!Send`, and `async_trait` — for a core that is otherwise pure and
synchronous, and that native runs with plain blocking calls. And eviction makes
network delivery unreliable: a request issued just before eviction may or may not
have reached the provider, and the response is lost.

## Decision 1 — Sans-IO: Rust stays synchronous, even on wasm

Each external-I/O effect is a **pure resumable step machine**:

```
step(state, incoming) -> NeedsIo(HttpRequest, state') | Settle(terminal, facts)
```

The *host* drives it. The **native** host runs it to completion in one synchronous
pass (`ureq` blocks on the request, feeds the response straight back into `step`).
The **durable-object** host's TypeScript shell awaits `fetch` on `NeedsIo`, and on
the next isolate wake re-enters the synchronous `step` with the response. No
`async` in Rust, no futures, no `Send`/`!Send` tax, no `async_trait`. The lifecycle
is `claim → [NeedsIo → io_pending → io_done]* → settle`.

**Rejected:** async-colouring the executor spine. The step machine gives the DO its
suspension points without imposing async on the native path or the pure core.

This is not just an ergonomic preference — it is what makes the native path a
**refinement** of the general machine: the same `step` function, run without
suspension. The formal model proves the native run-to-completion path satisfies a
strictly stronger property than the general host (`NativeExactlyOnce`, below).

## Decision 2 — The only async primitive is HTTP (the step machine)

`NeedsIo` carries exactly one thing: an `HttpRequest`. Effects relate to it by
class, and this is a *closed* classification, not an open unification:

- **coerce / agent turns** — always HTTP (a provider call; an agent turn is a
  multi-step machine, each model call one `NeedsIo` — Phase 2).
- **file effects** — HTTP **only on the large tier** (Decision 4); small-file I/O
  is synchronous store access, never on the step machine.
- **coordination / queue / event / notify / human / workflow-invoke / small-file**
  — **never** HTTP. These are store-only effects on the synchronous fast path.

Store-only effects are deliberately **not** put on the step machine — no premature
unification. The step machine exists for one reason: to suspend across an external
`fetch`. An effect that only touches durable storage has nothing to suspend on.

## Decision 3 — Network delivery is at-least-once + idempotency key

An effect may be evicted mid-`fetch`; on resume we retry. Delivery is therefore
**at-least-once**, and the duplicate is bounded by a **per-round idempotency key
derived only from durable identity** — `idempotency_key([instance, version, epoch,
rule, node_id, identity])` — so a retry carries the *same* key and the provider
dedupes it. Where a provider exposes an idempotency header (Anthropic; OpenAI is
uneven) the duplicate is absorbed provider-side; where it does not, the residual
duplicate-on-eviction risk is stated in the guarantee report (see the tracker's
open-questions matrix). Content-addressed writes (Decision 4) are idempotent by
construction and need no key.

**This decision is the one carrying formal risk, so it is the core of the Phase-0
model** (`ResumableEffectLifecycle.tla`, Apalache, coverage + bite). The model
proves, over both host modes, up to bounded depth:

| Property | Invariant(s) | What it guarantees |
|---|---|---|
| Exactly-once **settle** | `NoDuplicateSettle`, `SettledLedgerMatchesSet` | An effect reaches exactly one terminal; the ledger *is* the settled set. |
| At-least-once **delivery** | `AtLeastOnceLowerBounds` | Every started round is dispatched ≥ 1×; after an eviction+resume, dispatches strictly exceed provider executions — the real duplicate. |
| Idempotency-key **stability** | `IdemKeyStable`, `RoundKey` construction | The key is a pure function of durable identity, unchanged across suspend / resume / re-dispatch. |
| **Provider** exactly-once-modulo | `ProviderExecBoundedByRounds` | Despite ≥ 1 dispatches per round, the provider executes each round's request **at most once**, because the stable key deduplicates the retry. |
| No orphaned suspension | `NoOrphanedIoPending`, `InflightOnlyWhenIoPending`, `IoPendingHasRoundsLeft` | A suspended effect always has a live request or (on the DO host) a resume that re-dispatches; no effect strands mid-fetch. |
| Native = refinement | `NativeExactlyOnce` | The eviction-free native pass collapses at-least-once back to exactly-once: one dispatch and one provider execution per round. |

Each invariant carries an inline **Bite:** comment naming the one-line mutation that
makes Apalache report it violated; all six bites are verified to fail-closed. The
sharpest is `ProviderExecBoundedByRounds`: make the key attempt-dependent and the
provider double-executes a round — that is exactly the bug the durable identity
key exists to prevent.

**Recorded residual — provider idempotency-key coverage (verified 2026-07-09).**
The wording above ("Where a provider exposes an idempotency header (Anthropic;
OpenAI is uneven) the duplicate is absorbed provider-side") describes the intended
posture, not the wired state. A code audit of every provider HTTP builder
(`coerce_native::build_{anthropic,openai,codex}_request`,
`harness_model::build_{anthropic,openai}_request`) found that **no provider path
currently sends a provider-level idempotency key** — the sync Messages
(`/v1/messages`) and Responses (`/v1/responses`) APIs do not expose a request-level
idempotency header, and `cache_control` / `prompt_cache_key` are prompt-caching
(cost) controls, not request dedup. The residual is therefore the honest
at-least-once external delivery this decision already states: on eviction after a
`fetch` reaches the provider but before the response is recorded, the resume may
cause a **duplicate provider execution**. The store-side effect idempotency key
bounds this to **exactly-once settle** (one terminal in the ledger), not
exactly-once provider execution. The full per-endpoint matrix is recorded in the
tracker's open-questions list (`spec/durable-object-runtime-tracker.md`), which is
the home this decision points to.

## Decision 4 — One file construct; the runtime owns storage tiering

There is **no user-facing file split.** A file is a **content-hash handle**; its
operations — import→facts, export←facts, hand-to-effect, copy, hash — are
handle/stream-based and size-agnostic. The runtime places bytes automatically:

- **small → inlined in the DO's SQLite** (synchronous, transactional with
  fact-derivation);
- **large → spilled to a platform object store** (streamed out-of-band; the
  isolate touches only handle + metadata, never the bytes).

A threshold with spill-on-overflow decides; an optional size *hint* may inform it
but is never required. The one size-visible edge — "materialize the entire content
as an in-memory value" — is a bounded **runtime limit**, not a language construct.

**Rejected:** two file constructs (leaks a storage-engine decision into the
language). **Rejected:** wholesale object-store backing for all files (a perf and
atomicity regression on the common small structured-I/O case).

## Decision 5 — The storage plane is platform-owned and trusted: internal persistence, not egress

Writing a file, on **either** tier, is inside the trust boundary — exactly like
writing DO SQLite. The information-flow label rides the content-hash handle. The
IFC **egress door fires only on an explicit external hand-off** (handing bytes to
an outside party), **not** on ordinary storage. Tiering is invisible to IFC: spill
to the object store is still internal persistence, because the platform object
store is as trusted as the isolate's own SQLite. This keeps the membrane's egress
doors (DR workflow-encapsulation, E2-DYN) meaningful — they mark real boundary
crossings, not storage-engine plumbing.

## Decision 6 — Transport is HTTP everywhere

Streaming bodies, range GETs, multipart PUTs, presigned URLs, content-addressed
keys — all over HTTP. **No gRPC** (HTTP/2 trailer friction in Workers). A
hibernatable **WebSocket** is reserved for exactly one case — live
progress/backpressure from a long external data-plane job back to a sleeping DO —
and is built only when a workflow needs it. Protobuf-typed sidecar RPC is deferred
behind the same "only if a real typed, high-throughput data plane appears" gate.
One transport keeps the `NeedsIo` payload a single `HttpRequest` shape (Decision 2)
and the DO shell a single `fetch` driver.

## Decision 7 — Subprocess effects do not exist on the pure-DO host

`exec.command` and the stdio agent sidecars (codex / claude / pi) are **native-only
drivers**. On the DO they are either unavailable or re-expressed as HTTP to a
container sidecar — which is then just another network effect on the step machine
(Decision 2), not a special case. This keeps the wasm isolate free of any
process-spawn assumption.

## What Phase 0 establishes, and what it does not

**Establishes.** The lifecycle contract above is proven (coverage + bite) *before*
any code moves — the model-first discipline. The properties hold for a general
host that may evict, and the native host is a proven refinement, so the Phase 1–4
native-only refactors have a target to conform to and the Phase 5+ DO host has a
contract to implement.

**Does not.** No Rust code changes in Phase 0. The model abstracts the idempotency
key as an injective `KeyOf` over durable identity and the provider as a dedup set
keyed by `RoundKey`; the concrete key computation, the provider-header matrix, the
file-tier threshold, and the per-alarm CPU budget are Phase 1+ / operational
concerns tracked in the tracker's open-questions list, not re-decided here.

## Cross-effort ordering

This effort shares seams with the workflow-encapsulation build (closed
2026-07-02). Two standing constraints:

1. **Phase 3 (store behind a trait) lands after the encapsulation coordination
   partition** (the `<pkg>/<name>::X` schema change, now shipped), so the store
   trait is cut around the partitioned schema and the migration happens once.
   Phases 0–2 are independent.
2. **The DO host (Phase 5+) is built with the membrane already in place.** Every
   new delivery/re-entry seam the host introduces — alarms, `fetch` re-entry, the
   HTTP container sidecar — routes through the encapsulation **E2-DYN marker door**
   (per-seam door discipline). The membrane's doors must exist before this effort
   multiplies the seams they guard; they do.

## Consequences

- The core stays synchronous and pure; the async lives entirely in the host
  shell. The native binary is unaffected.
- The exactly-once guarantee is honestly stated as **exactly-once settle +
  at-least-once external delivery deduplicated by a durable idempotency key**, with
  a named residual where a provider offers no key.
- One file surface; storage tiering and object-store spill are runtime policy,
  invisible to the language and to IFC egress accounting.
- HTTP is the sole external transport, so `NeedsIo` and the DO `fetch` driver each
  have a single shape.
