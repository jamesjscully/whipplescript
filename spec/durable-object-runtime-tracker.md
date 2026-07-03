# Durable-object runtime tracker — sans-IO async + Workers/DO host

**Purpose (open intent):** make whipplescript able to run inside a single-threaded
wasm isolate (Cloudflare Durable Object) by lifting all blocking I/O out of the
core, then building the DO host binding. This file holds only what is *not yet
true in the repo*. The settled design is now **DR-0033**
(`spec/decision-records/0033-durable-object-runtime.md`, accepted 2026-07-03); the
"Decisions" section below is retained as a summary and defers to the DR. Reality
lives in code + git + gates.

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
build on them. **Formalized as DR-0033**
(`spec/decision-records/0033-durable-object-runtime.md`, 2026-07-03) — that record
is the design SSOT; this list is the at-a-glance summary.

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

**Downstream-customer note (2026-07-03, experimentation subsystem):** the
checkpoint substrate this storage plane carries (per
`decision-records/restorable-context.md`, the content-addressed, tiered,
event-referenced file store *is* the checkpoint mechanism) has a second
customer beyond "undo": the experimentation & evaluation subsystem
(`spec/experimentation-subsystem-research-note.md`, pre-ADR) uses a checkpoint
as the frozen prefix of a freeze-and-regenerate experiment. Neither
requirement adds phase work now; both are cheap to preserve while this work is
in flight and expensive to retrofit:

1. The checkpoint's **consistent cut must pin coordination state**
   (counters/leases/queues/ledgers) alongside transcript + event-log index +
   file manifest. The coordination store is workspace-scoped and
   cross-instance, so a cut that omits it makes replay-from-checkpoint
   non-deterministic and hollows out the subsystem's checkpoint-conditional
   evidence semantics (research note §7.2 Tier B, §15). Phase 3 (store behind
   a trait) is the cheap moment to shape this: cut the coordination trait with
   a snapshot/manifest capability in view.
2. **Content-addressing stays canonical and stable** across tiers and hosts
   (Decision 4): the subsystem keys evidence by content hash (kernel identity,
   file manifests), so hash identity must not vary by storage tier, host
   binding, or serialization quirk.

---

## Phase plan (open intent)

### Phase 0 — Formal model + DR-0033 — DONE 2026-07-03
- [x] TLA+ (Apalache) model of the resumable-effect lifecycle: `claim →
      [NeedsIo → io-pending → io-done]* → settle`.
      `models/tla/ResumableEffectLifecycle.tla`, in the `check-tla-models.sh`
      gate (Apalache 0.56.1, length 6, both host modes). Proves — coverage
      (11-invariant `SafetyInvariants` holds) **and** bite (each key invariant
      carries an inline `Bite:` mutation, all six verified to fail-closed):
      exactly-once settle (`NoDuplicateSettle`/`SettledLedgerMatchesSet`);
      at-least-once delivery deduplicated by a durable idempotency key
      (`AtLeastOnceLowerBounds` + `ProviderExecBoundedByRounds` + `IdemKeyStable`,
      Decision 3); no orphaned `io_pending`
      (`NoOrphanedIoPending`/`InflightOnlyWhenIoPending`/`IoPendingHasRoundsLeft`);
      native run-to-completion is a refinement (`NativeExactlyOnce`).
- [x] DR-0033 capturing Decisions 1–7:
      `spec/decision-records/0033-durable-object-runtime.md` (accepted
      2026-07-03); the tracker's Decisions section now defers to it. No
      user-facing surface in Phase 0, so no `docs/` change (formal model + design
      record only).

### Phase 1 — Pilot: coerce sans-IO on native (no behavior change) — DONE 2026-07-03
- [x] Shared sans-IO vocabulary in `crates/whipplescript-kernel/src/sansio.rs`:
      `HttpRequest`/`HttpResponse`/`TransportError` (moved here from
      `coerce_native`, re-exported for source compat), `IoRequest`
      (`Http(..)` today; sum left open for a future large-object/blob control
      variant so Phase 7 is additive, not a corner), `IoResult`, `Outcome`
      (`NeedsIo`/`Settle`), the `StepMachine` + `HostDriver` traits, and
      `run_to_completion` (the native one-pass driver = the model's
      `NativeExactlyOnce` refinement). A blanket `impl HostDriver for
      T: CoerceTransport` bridges the existing `ureq`/fake transports with no
      change to them.
- [x] Coerce reshaped as a `CoerceStepMachine` (`coerce_native.rs`): `step(None)`
      = prepare (`build_request`) → `NeedsIo(Http)`; `step(Some(resp))` = finish
      (`parse_response` / the identical timeout+transport error branches) →
      `Settle`. `NativeCoerceClient::coerce` now drives it via
      `run_to_completion` over its transport. Byte-for-byte identical behavior:
      all coerce tests green (kernel 199 unit + 17 e2e incl.
      `native_client_drives_transport_and_parses`/`_maps_timeout`/
      `e2e_coerce_success_and_failure_branches_are_deterministic`; CLI
      control_plane 162 + soft_middle 56; full kernel+CLI suite exit 0),
      `cargo fmt --all --check` clean, `clippy -D warnings` clean.
- [x] Model coverage: the general Phase-0 lifecycle model
      (`ResumableEffectLifecycle.tla`) already subsumes coerce as the 1-round
      native instance (both host modes, rounds 0..2). Code-level conformance
      tests demonstrate the actual coerce code follows that shape:
      `coerce_step_machine_is_a_one_round_lifecycle_instance` +
      `coerce_step_machine_maps_transport_failures_to_terminals` (coerce_native)
      and `run_to_completion_drives_zero_one_and_many_rounds` (sansio). No
      user-facing surface, so no `docs/` change.

### Phase 2 — Generalize the seam to agent turns — DONE 2026-07-03
- [x] The single agent model call is a sans-IO step: new `HttpModelClient` trait
      (`harness_loop.rs`) splits an HTTP model client into `build_request`
      (prepare) + `parse_response` (finish); `RealHarnessModelClient`
      (`harness_model.rs`) implements it, and its `HarnessModelClient::next` now
      runs a `ModelCallMachine` via `run_to_completion` over its transport —
      behavior identical (covered by the existing `harness_model` tests:
      `non_success_status_is_a_provider_error`, `timeout_maps_to_timeout`,
      `final_reply_has_no_tool_calls`).
- [x] The whole tool-use turn is a multi-step machine: `BrokeredTurnMachine`
      (`harness_loop.rs`) replicates `run_brokered_loop`'s control flow but
      surfaces each model call as `NeedsIo(Http)` (so a DO isolate can suspend
      across every provider `fetch`); tool calls stay nested effects brokered
      synchronously by the `ToolExecutor`. Native driver `run_brokered_turn_http`
      = `run_to_completion` over the native transport; the DO host (Phase 5)
      drives the same machine across wakes. **Proven byte-identical to
      `run_brokered_loop`** across 5 scenarios (immediate-final, tool-then-final,
      model-error, timeout, step-bound): `brokered_turn_machine_matches_loop_*`
      compare terminal, summary, steps, observation stream, merged usage, tool
      calls, and the checkpoint sequence. Native production stays on
      `run_brokered_loop` (zero behavior change) until Phase 5 flips the DO on.
- [x] Stdio sidecars (codex/claude/pi) confirmed native-only opaque drivers —
      they do not implement `HarnessModelClient` and never touch the step
      machine (DR-0033 Decision 7). Gates: kernel 204 + e2e 17; full kernel+CLI
      suite exit 0; fmt + `clippy -D warnings` clean. No user-facing surface, so
      no `docs/` change.

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
- [ ] Checkpoint cut × coordination store: decide where coordination-state
      snapshot/restore lives in the Phase 3 traits so the restorable-context
      consistent cut can pin it (see Downstream-customer note above).
