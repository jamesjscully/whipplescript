# Durable-object runtime tracker — sans-IO async + Workers/DO host

**Purpose (open intent):** make whipplescript able to run inside a single-threaded
wasm isolate (Cloudflare Durable Object) by lifting all blocking I/O out of the
core, then building the DO host binding. This file holds only what is *not yet
true in the repo*. Settled design lives in the "Decisions" section below and will
graduate to **DR-0033** in Phase 0; reality lives in code + git + gates.

Registered in `spec/TRACKERS.md` (status: active).

---

## Goal

The whip evaluation core (parser + kernel + rule/flow engine + effect ledger) is
already pure, synchronous, and network-free; the only blocking I/O lives inside
the ~15 `run_*_effect` handlers in the worker executor. Re-home that I/O so the
same core runs on two hosts:

- **native** (today's CLI): blocking `ureq` + OS threads + `rusqlite` + `std::fs`
  — behavior unchanged throughout;
- **durable-object** (new): a wasm32 isolate where the only async primitive is
  `fetch()`, storage is synchronous DO SQLite, timers are alarms, config is
  secrets.

The work is a **second host binding behind core-defined seams**, not a fork
(open-core seam discipline). Phases 0–4 are native-only refactors that keep every
existing gate green; the wasm host arrives only in Phase 5+.

---

## Decisions (settled — the constraints these phases must respect)

These are locked. They constrain the earlier phases even though the later phases
build on them. Formalize as DR-0033 in Phase 0.

1. **Sans-IO, Rust stays synchronous — even on wasm.** Each external-I/O effect
   is a pure resumable step machine `step(state, incoming) -> NeedsIo(HttpRequest,
   state') | Settle(terminal, facts)`. The *host* drives it: native runs it to
   completion synchronously (ureq, one pass); the DO's TS shell awaits `fetch` on
   `NeedsIo` and re-enters the synchronous `step` on the next pass. No
   `async_trait`, no futures, no `Send`/`!Send` tax in Rust. (Rejected: async-
   colouring the executor spine.)

2. **The only async primitive is HTTP (the step machine).** Effects may or may
   not invoke it: coerce/agent turns always; **file effects only on the large
   tier** (below); coordination/queue/event/notify/human/workflow-invoke/small-
   file never (synchronous fast path). Store-only effects are *not* put on the
   step machine (no premature unification).

3. **Network delivery = at-least-once + idempotency key.** An effect may be
   evicted mid-`fetch`; on resume we retry. Provider idempotency keys where they
   exist (Anthropic header; OpenAI uneven) bound the duplicate risk; the residual
   at-least-once semantics is stated in the guarantee report. Content-addressed
   writes (below) are idempotent by construction.

4. **One file construct — the runtime owns storage tiering.** No user-facing
   split. A file is a **content-hash handle**; operations (import→facts,
   export←facts, hand-to-effect, copy, hash) are handle/stream-based and
   size-agnostic. The runtime places bytes automatically: **small → inlined in
   the DO's SQLite** (sync, transactional with fact-derivation); **large → spilled
   to a platform object store** (streamed out-of-band; the isolate touches only
   handle + metadata, never the bytes). Threshold with spill-on-overflow; optional
   hint, never required. The one size-visible edge — "materialize entire content
   as an in-memory value" — is a bounded **runtime limit**, not a language
   construct. (Rejected: two constructs; rejected: wholesale-R2 for the perf/
   atomicity regression on small structured I/O.)

5. **Storage plane is platform-owned and trusted → internal persistence, NOT
   egress.** Writing a file (either tier) is inside the trust boundary, like
   writing DO SQLite; the label rides the content-hash handle. The IFC **egress
   door fires only on an explicit external hand-off** (handing bytes to an outside
   party), not on ordinary storage.

6. **Transport = HTTP everywhere** (streaming bodies, range GETs, multipart PUTs,
   presigned URLs, content-addressed). **No gRPC** (HTTP/2 trailer friction in
   Workers). Hibernatable **WebSocket** reserved for one case — live progress/
   backpressure from a long external data-plane job back to a sleeping DO — built
   only when a workflow needs it. Protobuf-typed sidecar RPC deferred behind the
   same "only if a real typed high-throughput data-plane appears" gate.

7. **Subprocess effects do not exist on the pure-DO host.** `exec.command` and the
   stdio agent sidecars (codex/claude/pi) are native-only drivers. On the DO they
   are either unavailable or re-expressed as HTTP to a container sidecar (which is
   then just another network effect on the step machine).

**Invariants for every phase:** model-first (invariants proven — coverage *and*
bite — before greenfield code); native gates stay green end-to-end (Phases 0–4
change no native behavior); per-piece review gate (review + fixes + verify + docs,
incl. user-facing `docs/`, before a box is checked).

**Cross-effort ordering (2026-07-01):** this effort shares seams with the
workflow-encapsulation build (`workflow-encapsulation-implementation-tracker.md`
§ Sequencing — the mirror of this note). Two constraints:

1. **Phase 3 here (store behind a trait) lands AFTER the encapsulation
   coordination partition** (its Phase 3, the `<pkg>/<name>::X` schema change),
   so the store trait is cut around the partitioned schema and the migration
   happens once. Phases 0–2 here are independent and may run in parallel with
   the encapsulation build.
2. **The DO host (Phase 5+) is built with the membrane already in place, not
   before it.** Every new delivery/re-entry seam the host introduces (alarms,
   `fetch` re-entry, HTTP container sidecar) must route through the
   encapsulation E2-DYN marker door (per-seam door discipline) — the membrane's
   doors must exist before this effort multiplies the seams they guard.

---

## Phase plan (open intent)

### Phase 0 — Formal model + DR-0033
- [ ] TLA+ (Apalache) model of the resumable-effect lifecycle: `claim →
      [NeedsIo → io-pending → io-done]* → settle`. Prove: exactly-once **modulo
      at-least-once-on-eviction** (Decision 3); no orphaned `io-pending`;
      idempotency-key stability across suspend/resume; the native run-to-
      completion path is a **refinement** of the general machine. Bite fixtures.
- [ ] Write DR-0033 capturing Decisions 1–7 (supersede the scattered notes here).

### Phase 1 — Pilot: coerce sans-IO on native (no behavior change)
- [ ] Introduce the shared types (`HttpRequest`/`IoResult`/`Outcome`) + host
      driver trait; the `IoRequest` sum has room for a future large-file/blob
      control variant so it is not a corner.
- [ ] Reshape coerce into the step machine — `build_request` = prepare,
      `parse_response` = finish (already pure); native driver runs it via ureq.
      All coerce tests green; identical behavior.
- [ ] Model coverage: coerce instance of the Phase-0 lifecycle.

### Phase 2 — Generalize the seam to agent turns
- [ ] Express the owned/model agent loop (`HarnessModelClient::next`) as a
      multi-step machine (each model call a `NeedsIo`; tool calls as nested
      effects). Define the HTTP-stepped agent driver (native + the one the DO
      uses). Stdio sidecars (codex/claude/pi) remain native-only opaque drivers.

### Phase 3 — Store behind a trait
- [ ] Extract `SqliteStore`/`WorkItemStore`/`CoordinationStore` APIs into traits;
      native impl = rusqlite. Note where single-writer-per-DO lets the
      `Immediate`-txn / WAL locking machinery fall away on the DO impl.
      *(Ordering: after the encapsulation coordination-partition schema change —
      see Cross-effort ordering above.)*

### Phase 4 — Files route through the store trait (tiering seam)
- [ ] Route `file.read/write/import/export` through the store trait so a second
      physical tier can slot in without touching the language. Native backing =
      `std::fs`. Small-file inline-in-store path defined; large-tier is Phase 7.

### Phase 5 — DO host crate + TS shell (the wasm target)
- [ ] `whipplescript-host-do` (wasm32) + Worker/DO shell: async `fetch` driver for
      `NeedsIo(Http)`, store trait over synchronous DO SQLite, consumes the
      sans-IO core. Subprocess effects unavailable / via HTTP sidecar. Every
      delivery/re-entry seam this host adds routes through the E2-DYN marker
      door (see Cross-effort ordering above).

### Phase 6 — Scheduling + config on the DO
- [ ] Clock-source + timers → DO **alarms** (replaces external polling). Config/
      credentials → Worker **secrets** (no dotfiles). `chrono` std → data-only
      `chrono-tz`; `Instant` → injected virtual clock.

### Phase 7 — Large-object tier (designed now, built later)
- [ ] Platform object-store backing behind the same file store trait: content-
      addressed keys, threshold spill, streamed import/export via a data-plane
      worker, presigned URLs for client↔storage transfer. Isolate never buffers
      bytes. Enterprise-tier deliverable (native/OSS backs files with local fs).

---

## Open questions / risks

- [ ] Per-alarm CPU budget: large rule evaluations may need chunking across
      alarms (the pass model already supports it — confirm on real workloads).
- [ ] Provider idempotency-key coverage matrix (Anthropic vs OpenAI endpoints):
      where a key is unavailable, the residual duplicate-on-eviction risk must be
      documented in the guarantee report (Decision 3).
- [ ] Threshold + spill policy for file tiering: default value, whether the
      optional size hint is worth exposing in v1 (Decision 4).
- [ ] workflow-invoke is already cross-pass/store-only — confirm it needs no step-
      machine treatment (child observed across passes, no external I/O).
