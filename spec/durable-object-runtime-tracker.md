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

## DO feature-parity sweep — COMPLETE 2026-07-09

Jack directed: bring the DO backend to full feature parity with native, **including
the agent-turn tool executor**. DONE, including Bashkit under DR-0039. The DO
now runs `file.*` effects, exposes `checkpoint`/`restore`
operator commands, and runs a real in-isolate agent tool set — all proven
end-to-end through the real wasm boundary (validate.cjs 8 cases). Remaining DO
asymmetries are intentional (below): explicitly brokered non-bash execution and the
coordination-state checkpoint snapshot (deferred on both hosts).

**Residuals re-scoped to v0.4 (Jack 2026-07-09):** with the parity sweep done and
v0.3 cutting, the deferred `[~]` items below (P7 object-store real backing,
materialization endpoints, image-digest = env-hash, IFC egress default-deny,
per-alarm CPU budget, coordination-state snapshot) are **v0.4-scoped** — none is a
v0.3 blocker. The v0.3 DO deliverable (LIVE runtime + native parity) is complete.

Two threads (both done):

**Thread A — file plane + restore** — DONE 2026-07-09 (P1 3d523ad, P2 31f895b,
P3 e7af877). The DO now runs `file.*` effects over a real `files` table
(`DoSqlStorage` shared via `Rc<DoSql>`), has `FileStore::remove` on the seam, and
exposes `checkpoint`/`restore` operator commands (wasm + an `index.ts` verb
route). validate.cjs cases 6-7 prove file-write + checkpoint→tamper→restore
end-to-end through the real wasm boundary.
- [x] **P1 DO file plane wired end-to-end.**
- [x] **P2 delete on the `FileStore` seam** (promoted to a trait method +
      `DoStorage::delete_file`).
- [x] **P3 DO checkpoint/restore entry point** (wasm methods + `index.ts` command
      route; the worker's first verb dispatch).

**Thread B — agent-turn tool executor** — DONE 2026-07-09 (P4 b6b2fcd). The DO
turn now advertises a real in-isolate tool set instead of `tools: Vec::new()`.
- [x] **P4 in-isolate DO tool executor.** `DoToolExecutor` over the shared
      `Rc<DoSql>` for read/write/edit/ls/find/grep/recall + list/add/update_todo,
      all synchronous SQLite rounds against the flat `files` table / `content_blobs`
      / work-item store; schemas mirror native; wired as the default `agent_tools`.
      validate.cjs case: an agent turn calls `write` in-isolate through real wasm.
      plus Bashkit over the same instance-scoped store-backed file plane.

**P5/P6 DROPPED (2026-07-09).** The old plan — reshape the tool seam into a
nested `ToolCallMachine` yielding `NeedsIo(Http)` and broker `bash` to a
`whip-tool/1` sidecar — was the WRONG model. `spec/in-isolate-bash-design-note.md`
scopes the bash solution as an **in-isolate virtual interpreter** (bashkit):
bash builtins run in-process over a store-backed VFS, settling synchronously like
every other tool — no fetch-suspend, no sidecar, no seam reshape. Only *real*
exec (cargo/builds) escalates to the existing Class-A `whip-executor/1` sidecar.
The synchronous `ToolExecutor` trait stays.

**bash-via-bashkit is accepted and implemented in the GaugeDesk DO placement sprint**
(DR-0039, Jack 2026-07-13): Bashkit becomes the default governed `bash` on
native and DO; non-bash external capabilities remain explicit brokered effects.
The same wrapper and script semantics run in the native owned harness, native
governed host, and DO governed host. Tracked by the design note and DR-0039.

**GaugeDesk governed-host local closure — DONE 2026-07-13.** The Worker now
accepts image bodies through a post-admission broker cache; validates
attributable human answers against and resumes the exact suspended command;
exports/imports same-placement forks at an exact durable event coordinate while
re-reading transcript authority from the source DO store; and projects
runtime-owned evidence pointers, terminal receipts, guarantee evidence, and
field-flow signatures. The terminal projection also publishes a deliberately
narrow token-count observation keyed by the opaque runtime `usage_ref`, allowing
an embedding product to meter managed inference without reading runtime SQL or
claiming ownership of the usage evidence body. Caught-up SSE plus hibernatable
WebSockets expose durable progress. Evidence:
`governed_host_images_resolve_only_from_the_admitted_broker_cache`,
`gaugedesk_host_protocol_admits_the_same_package_on_the_do_store`, 72 host-DO
tests, the wasm32 no-default-features build, and Worker `tsc --noEmit`. A live
GaugeDesk-to-deployed-Worker pass remains deployment/secret infrastructure, not
open local host work.

## Secret-free model egress broker (DR-0042 / GaugeDesk LLM-5)

- [x] **Authenticated HTTP broker protocol.** Completed 2026-07-14: added the
  `whipplescript.model-egress.v1` envelope, explicit `model-broker` provider
  realization, exact sentinel/credential-ref checks, auth-header stripping,
  fail-closed broker URL/token/response validation, and hermetic fake-broker
  conformance. The verified turn admission now returns the complete signed
  non-secret provider tuple, so brokered turns realize dynamically with no
  deployment provider map; the static map remains only for an exact explicit
  `worker-secret` transition. Worker TypeScript, host crate tests, wasm build,
  and a full Wrangler dry-run bundle pass. This replaces deployment-wide BYOK
  secrets and broker mapping.
- [ ] **Outbound local broker session.** Bind the same envelope to an authenticated
  client-initiated session so a remote DO development turn can use a locally sealed
  Codex credential without uploading its refresh token. HTTP Home brokers do not
  depend on this transport.

Out of scope (intentional asymmetries): `exec.command` is native-only by design
(DR-0033 Decision 7, re-expressed as the Class-A executor HTTP effect); the
coordination-state snapshot for the checkpoint cut is deferred on *both* hosts
(the `[~]` items below).

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
   stdio agent sidecars (codex/claude) are native-only drivers. On the DO they
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
   evidence semantics (research note, "Evidence identity — the slice hash" +
   "Architecture" sections). Phase 3 (store behind
   a trait) is the cheap moment to shape this: cut the coordination trait with
   a snapshot/manifest capability in view. *Generalized 2026-07-03 by the
   versioned-workspace note's "Scope semantics": the cut spans both planes —
   substance by manifest, workspace-plane stores (evidence ledger, tracker,
   coordination tables) by **position / high-water mark**, which their
   monotonicity makes a per-store integer. Shape the snapshot capability as
   cheap position capture, not deep copy.*
2. **Content-addressing stays canonical and stable** across tiers and hosts
   (Decision 4): the subsystem keys evidence by content hash (kernel identity,
   file manifests), so hash identity must not vary by storage tier, host
   binding, or serialization quirk.

*Third customer (2026-07-03):* the **versioned workspace**
(`spec/versioned-workspace-research-note.md`, pre-ADR) — whip-native version
control generalizing checkpoints into branches (manifest pointers with
children, virtual working sets, certified merge). Same substrate; the
additional cheap-to-preserve requirement is that the manifest/cut design
leaves room for a cut to have **divergent children** (a parent pointer, not a
linear undo chain). No phase work now.

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
- [x] Stdio sidecars (codex/claude) confirmed native-only opaque drivers —
      they do not implement `HarnessModelClient` and never touch the step
      machine (DR-0033 Decision 7). Gates: kernel 204 + e2e 17; full kernel+CLI
      suite exit 0; fmt + `clippy -D warnings` clean. No user-facing surface, so
      no `docs/` change.

### Phase 3 — Store behind a trait — DONE 2026-07-03 (all 3 stores)
- [x] `CoordinationStore` → `Coordination` trait (`coordination.rs`):
      owner-parameterized primitives required, shared-owner convenience forms
      provided; native impl forwards to the inherent methods (delegation, not
      recursion — `unconditional_recursion` guards it). Object-safe (`&mut dyn
      Coordination` drives a boxed backend); `coordination_trait_seam_is_faithful`
      exercises lease/ledger/counter through the trait. Cut around the
      partitioned `<pkg>/<name>::X` owner (encapsulation Phase 3, satisfied).
- [x] `WorkItemStore` → `WorkItems` trait (`items.rs`): same delegating,
      object-safe pattern; `work_items_trait_seam_is_faithful` drives file/claim/
      release/finish/list through `&mut dyn WorkItems`. Store crate 88 tests green;
      fmt + `clippy -D warnings` clean.
- [x] `SqliteStore` → `RuntimeStore` trait (`lib.rs`): all 87 backend-agnostic
      operation methods (event / fact / effect / instance / registry) behind one
      object-safe trait; native impl forwards to the inherent methods (same
      delegating pattern). `runtime_store_trait_seam_is_object_safe_and_faithful`
      drives it through `&dyn RuntimeStore`. Kept the per-store single-trait shape
      (consistent with `Coordination`/`WorkItems`); role/facet splitting can be a
      later refinement. Excluded (inherent-only, native-FS): the constructors
      `open`/`open_in_memory` and `load_package_manifests_from_dir` (takes
      `impl AsRef<Path>` + reads the local FS — not a DO operation). Store crate
      89 tests green; fmt + `clippy -D warnings` clean.
- [~] Snapshot/manifest capability on the store traits (experimentation-subsystem
      downstream requirement) — deferred until the checkpoint mechanism lands so
      it is designed against a real consumer (see the trait doc-comments). This is
      the only Phase-3 remnant, deferred-with-cause.

### Phase 4 — Files route through the store trait (tiering seam) — DONE 2026-07-03
- [x] `FileStore` trait + `NativeFileStore` in
      `crates/whipplescript-store/src/files.rs`: the byte-I/O seam
      (`read_to_string`/`exists`/`create_dir_all`/`write`/`append`) the file
      effects perform, object-safe so a DO backend can be a `&dyn FileStore`
      (small files inline in DO SQLite, large spilled to an object store —
      Phase 7). Native backing = `std::fs`.
      `native_file_store_round_trips_through_the_trait` drives it through the
      trait.
- [x] All four handlers (`file.read`/`file.write`/`file.import`/`file.export`,
      main.rs) route their raw `fs::` I/O through `NativeFileStore` — path
      resolution and the `file store` policy boundary stay in the handler, only
      the bytes cross the seam. Behavior identical: the 15 CLI file-effect tests
      (`dev_file_read/write/import/export/*`, mode enforcement, path-escape
      refusal, policy scoping) pass; fmt + `clippy -D warnings` clean; full
      store+kernel+CLI suite green. The content-hash-handle / tiering model
      (Decision 4) layers on later (Phase 7) behind this same seam; the
      small-file-inline path is the native default.

### Phase 5 — DO host crate + TS shell (the wasm target) — COMPLETE incl. live edge 2026-07-07 (deployed: whipplescript-runtime.jamesjscully.workers.dev; effect-free + coerce-over-mock + timer workflows validated under wrangler dev/workerd, effect-free + timer + AUTONOMOUS ALARM validated on the real edge via wrangler tail). Remaining seams: DO ToolExecutor (async-tool-over-sidecar, = compute-plane Class-B), provider secrets on the edge (`wrangler secret put ANTHROPIC_API_KEY`, mechanism validated via .dev.vars locally)
> **Status (2026-07-03):** every in-repo chunk is built and live-validated —
> 5a store traits on `DoSqliteStore`, 5b all effect families (10 store-only +
> coerce suspend/resume + eviction-safe agent turn), 5c `DoInstanceDriver` +
> `DurableInstance` + the `#[wasm_bindgen]` `WasmDurableInstance` surface, and
> 5d the worker shell + end-to-end validation of the real wasm module over real
> SQLite (commit 56ae999); the DO agent model client landed too (commit 42194cf,
> `MessagesApiClient`), so agent turns run live in-repo alongside coerce. The ONLY
> remainders are provisioning (`wrangler deploy` against a live edge DO +
> `wrangler secret put` for live creds) and one design seam — a DO `ToolExecutor`
> for agent turns that request tools (the async-tool-over-sidecar boundary).
> Details per chunk below.
>
> **Concrete entry point (found 2026-07-03, Phases 0–4 done).** The sans-IO
> seams (`sansio.rs`, `HttpModelClient`/`BrokeredTurnMachine`) and the store
> traits (`RuntimeStore`/`Coordination`/`WorkItems`/`FileStore`) all exist, but
> the core is **not yet wasm-buildable**: `whipplescript-kernel` depends on
> `whipplescript-store`, which pulls `rusqlite` (bundled C, not wasm), and
> `RuntimeKernel` holds a concrete `SqliteStore`. The prerequisite refactor is:
> (1) split the store traits **and the ~40 data types they cross** (`NewEvent`,
> `StoredEvent`, the `*View`/`*Record` structs, `RuntimeStore`/`Coordination`/
> `WorkItems`/`FileStore`) into a wasm-clean crate (e.g. `whipplescript-store-api`)
> that `whipplescript-store` (rusqlite) then implements; (2) make `RuntimeKernel`
> generic over `S: RuntimeStore` (it calls 29 store methods, all already in the
> trait) so the CLI uses `RuntimeKernel<SqliteStore>` and the DO host uses
> `RuntimeKernel<DoSqliteStore>`. Only after that can the wasm host be built.
- [x] `RuntimeKernel<S: RuntimeStore = SqliteStore>` — the kernel is now generic
      over the store trait (commit fc6d14a); native uses `RuntimeKernel<SqliteStore>`
      (inferred), decoupled from the concrete store. Gate-green.
- [x] Store wasm-cleanup (commit 376c15c): `whipplescript-store` feature-gates the
      rusqlite backing behind a default-on `native` feature (`rusqlite` optional;
      all `SqliteStore`/`CoordinationStore`/`WorkItemStore` impls + ~57 helper fns
      + the `StoreError::Sqlite` variant gated). Builds for
      `wasm32-unknown-unknown` with `--no-default-features` (traits + data types
      only), native completely unchanged (90 tests, clippy clean).
- [x] **The sans-IO core builds for wasm** — kernel `native` feature forwards
      to `whipplescript-store/native` (default-on) + `whipplescript-store`
      dependency is `default-features = false`; `RuntimeKernel` has a
      `native`-only `SqliteStore` default (wasm form takes explicit `S`).
      `cargo build -p whipplescript-kernel --no-default-features --target
      wasm32-unknown-unknown` **succeeds** — the whole evaluation core
      (kernel + `sansio` + `harness_loop`/`harness_model` + `coerce_native`) runs
      on wasm. Native unchanged (kernel 204+17, CLI green, clippy `-D warnings`).
      This proves the DR-0033 architecture end-to-end.
- [x] `whipplescript-host-do` crate — the DO binding, built for wasm against the
      wasm-clean core, native tests green + wasm32 build green + live-validated
      through the wasm-bindgen boundary (56ae999). Building blocks:
      - [x] `FetchClient` (the DO's `fetch`) + `FetchHost` — the sans-IO
            `HostDriver` that fulfills a `NeedsIo(Http)` through the isolate's
            fetch. Any effect step machine (`coerce`, an agent turn) runs on the
            DO through it. Tested (`fetch_host_drives_a_step_machine_over_the_do_fetch`).
      - [x] `DoStorage` (the DO's synchronous SQLite) + `DoFileStore` — the file
            seam (`FileStore`) backed by DO storage (small files inline, flat key
            space). Tested (`do_file_store_round_trips_through_the_file_seam`).
      - Crate builds native (2 tests green, clippy `-D warnings` clean) **and**
        `--no-default-features --target wasm32-unknown-unknown`.
      - [x] TS/Worker shell landed (`worker/`): `src/index.ts` (the DO class
            running the sans-IO drive loop — step the synchronous Rust machine,
            await `fetch` on `NeedsIo(Http)`, re-enter), `wrangler.toml`
            (`new_sqlite_classes` DO SQLite + secrets), `package.json`/`tsconfig.json`,
            a README mapping each Rust host trait to its DO primitive, and
            `do_schema.sql` (the 33-table DO schema, coordination tables prefixed
            `coord_*`). **No longer just scaffold: live-validated in-repo** —
            `validate.cjs` (commit 56ae999) backs the JS `DoSqlBridge` with Node 24's
            `node:sqlite` (`DatabaseSync`, in-memory) and drives the REAL wasm-bindgen
            module through the REAL step protocol: `node validate.cjs` reports
            `PASS effect-free workflow -> completed` and `PASS coerce workflow ->
            needs_http -> (fetch) -> terminal`. `pkg/` (the wasm-bindgen output) is
            gitignored — `npm run validate` regenerates it. This exercises the exact
            deployed path minus only Cloudflare's `state.storage.sql` (swapped for
            `node:sqlite`, contract-identical) and `wrangler deploy`.
      - [x] `RuntimeStore` over `DoSql` (`do_store.rs`): the `DoSql` seam (the DO's
            synchronous SQLite as `execute`/`query`) + `DoSqliteStore<Sql: DoSql>`
            implementing the full 87-method `RuntimeStore` trait — **builds for
            wasm** (no rusqlite in non-test code). **All 87 methods are ported and
            verified against real SQLite**: the tests back `DoSql` with rusqlite, so
            every method's SQL runs against an actual engine (27 tests spanning the
            read/query family, registration + manifest fan-out, skills/inbox,
            evidence/diagnostic/artifact records, clock/time + dependency queries,
            leases, fact derivation + batch admission, program-version + the whole
            revision family incl. compatibility analysis, the capability/profile
            policy + capacity engine behind `claimable_effects`, the transactional
            write-path core — `commit_rule`(+guard), the `complete_effect` family,
            `start_run`, `cancel_effect`, `request_effect_cancellation`,
            `activate_revision` — and `rebuild_projections` with its full
            `do_replay_*` suite). Zero `todo!()`; clippy `-D warnings` clean. The DO
            runs the *same* SQL the native `SqliteStore` does; the DO single-writer
            per-invocation model supplies the atomicity the native path gets from a
            rusqlite transaction (methods never yield mid-sequence). What remains is
            *live-DO validation only*: a `DoSql` impl over the real
            `state.storage.sql` in the `worker` crate, exercised end-to-end against
            an actual Durable Object.
      - [~] **Instance-level sans-IO scheduler (full lift).** **STATUS 2026-07-03:
            in-repo COMPLETE — chunks 0–5 all built + validated; cause-deferred
            remainders only** (the optional native-default-executor swap in chunk 4,
            and the live `wrangler deploy` in chunk 5). The narrative below is the
            chunk-by-chunk record. The native top-level
            driver is the `dev` fixpoint (`main.rs`): alternate `step_instance`
            (pure rule pass — reads facts/effects, commits ready rules, may spawn
            effects / reach a workflow terminal) and `run_worker_once` (the effect
            executor, where all external I/O lives) until a full round makes no
            progress (idle/park) or the instance terminates. The refactor
            (scope B — full lift, chosen 2026-07-03) re-expresses this as a
            host-agnostic instance step machine generic over `RuntimeStore`,
            composing the already-sans-IO effect machines
            (`CoerceStepMachine`/`BrokeredTurnMachine`) so a ready HTTP effect
            suspends with `NeedsIo(Http)`; this is the object the wasm-bindgen
            surface wires to. **Model-first landed:**
            `models/tla/InstanceSchedulerLifecycle.tla` (in `check-tla-models.sh`)
            proves the NEW instance-level obligations above the per-effect
            `ResumableEffectLifecycle` — a workflow terminal is recorded at most
            once and is absorbing; the scheduler parks only at a genuine fixpoint;
            an effect is mid-fetch only while running; eviction/resume never loses
            or double-counts instance progress — coverage (6-invariant
            `SafetyInvariants`) **and** bite (5 inline mutations, each verified
            fail-closed). Code lift proceeds in dependency-ordered chunks with
            native gates green at each step.

            **Lift target = `whipplescript-kernel`** (already wasm-clean, already
            depends on `whipplescript-parser` for `IrProgram`/`IrRule` and on the
            `whipplescript-store` traits, already holds
            `RuntimeKernel<S: RuntimeStore>`) — no new crate. **Architectural crux
            the lift must resolve:** natively the runtime store, the coordination
            store, and the work-items store are *three separate SQLite files*, and
            every runtime helper re-opens them by path
            (`SqliteStore::open(store_path)`, `WorkItemStore::open(items_store_path())`).
            On a DO they collapse to one held handle. So the lift's substance is
            changing the store-access idiom from open-by-path to a threaded
            `&mut S: RuntimeStore` (+ `&mut dyn Coordination`/`WorkItems`) handle,
            generically, across the rule engine and all ~15 effect handlers.
            Dependency-ordered chunk plan (each: move + re-import in `main.rs`,
            native gate green, review, commit):
            (0) model-first — **DONE** (`InstanceSchedulerLifecycle.tla`);
            (1a) lift the lowering OUTPUT-TYPE cluster
                (`OwnedFact/Effect/Dependency/WorkflowTerminal/Lowering` +
                `BranchReport/BranchStatus`) into `kernel::lowering` —
                **DONE** (commit a461a41; native+wasm, clippy -D, tests green);
            (1b) lift the pure lowering CLOSURE into `kernel::rule_lowering` —
                **DONE** (commit e5838bf): 104 free functions + 18 support types
                with their impl blocks (`RuleContext`, `EvalValue`/`EvalScope`,
                `ReadyContexts`, `GuardReport`/`GuardStatus`, the parse-block
                structs) + `split_args`, all verified pure — ~4777 lines left
                main.rs; it imports the closure via `rule_lowering::*`. native+wasm,
                clippy -D, fmt idempotent, tests green (unchanged counts). The whole
                pure lowering layer now lives in the kernel;
            (2) lift the rule-pass ORCHESTRATION (`step_instance`/
                `project_queue_items`) generic over held store handles — the crux
                (open-by-path → threaded `&mut S`). **Store-handle fork resolved:
                Option A (unified facade).** (2a) **DONE (commit 97959b1):**
                `NativeStores` (`whipplescript-store`, native feat) presents the 3
                native connections (`SqliteStore`/`CoordinationStore`/
                `WorkItemStore`) as one handle impl'ing
                `RuntimeStore + Coordination + WorkItems` by delegation (104 fwds;
                the 7 shared-owner Coordination defaults inherited) — the native
                counterpart to the DO's one `DoSqliteStore` impl'ing all three;
                tested through all 3 surfaces; store 91 tests green.
                (2b) **DONE (commit 4a15053):** `step_instance_generic<S:
                RuntimeStore + Coordination + WorkItems>` drives the fixpoint over
                ONE held `RuntimeKernel<S>` (no per-op re-open); `project_queue_items`
                / `apply_rule_cancels` / `release_holder_resources_on_terminal` are
                generic too. Native `step_instance` is now a thin wrapper building
                `RuntimeKernel<NativeStores>` — 6 callers unchanged; DO drives the
                same pass over `DoSqliteStore`. Added `RuntimeKernel::store()`/
                `store_mut()`. The behavior-sensitive re-open→held-connection change
                verified equivalent (kernel 204 / bins 374 / control_plane 162 /
                soft_middle 56 green, clippy -D, fmt idempotent). NOTE: the generic
                pass is still physically in `main.rs`; its relocation to the kernel
                rides with the chunk-4 step-machine assembly (the DO host calls it
                from there);
            (3) lift the effect executor: the ~15 `run_*_effect` handlers each →
                thin native wrapper (opens the store) + host-agnostic
                `*_generic<S>(kernel, …)` core the DO step machine calls.
                **IN PROGRESS.** Store-only handlers converted so far (wrapper +
                `*_generic<S: RuntimeStore>` core): `run_event_effect`,
                `run_capability_effect` (3a, 2b1b255),
                `run_human_effect` + the shared read-only helper
                `resolve_effect_input_after_bindings` (3b, 93ffefd), and all four
                file handlers `run_file_effect`/`_write`/`_import`/`_export` via the
                `FileStore` seam (`*_generic<S>(kernel, files: &dyn FileStore, …)`,
                native `NativeFileStore` / DO `DoFileStore`) (3c, 2a1b2f9).
                `run_queue_effect` via `WorkItems` + the facade wrapper
                (`RuntimeKernel<NativeStores>`) (3d, e3b31a5), and
                `run_coordination_effect` via `Coordination` + facade (+ its helper
                `coordination_owner_for_instance` generic over `&S`) (3e, aa016ab).
                and `run_notify_effect` + its nested helpers
                `internal_workflow_delivery_violation` /
                `workflow_identity_for_instance` (now generic over `&S`) (3f,
                dbbf9dc). **11/~15 generic — the store-only executor is fully
                lifted.** REMAINING = the hard tail (best done WITH the chunk-4 step
                machine so the NeedsIo suspension is wired, not just store access):
                `run_coerce_effect`(→`cancel_coerce_effect`/`run_native_coerce_effect`)
                wires the sans-IO `CoerceStepMachine`; `run_agent_effect` is NOT a
                clean store-access conversion — it delegates to
                `harness_tools::run_owned_agent_turn(&mut kernel, …, store_path, …)`
                which RECURSES into sub-workflows (step/worker) just like
                `run_workflow_invoke_effect` (435 ln), so both need the
                recursion/step-machine design, not mechanical threading; `run_exec_effect` stays
                native-only (DR-0033 Decision 7, no DO port). Original note: the
                `file*` handlers need `FileStore`, `coordination`/`queue` need
                `Coordination`/`WorkItems` (audit for inherent-vs-trait methods —
                e.g. `try_acquire_for_owner`); `workflow_invoke` (435 ln) recurses
                into step/worker; `agent`/`coerce` wire the sans-IO
                `BrokeredTurnMachine`/`CoerceStepMachine` for the DO NeedsIo path;
                `exec` stays native-only (DR-0033 Decision 7);
            (4) assemble the `InstanceStepMachine` (the fixpoint as a `StepMachine`
                raising `NeedsIo(Http)`); native `dev`/`worker`/`step` call it.
                **Groundwork DONE (commit d1cb27b):** the rule pass is lifted into
                `whipplescript_kernel::rule_pass` (`step_instance_generic` + generic
                helpers + `StepReport` + `lowering_idempotency_key`, ~455 ln;
                native+wasm) — so the step machine can drive it in the kernel and
                the CLI keeps only the `NativeStores`-building `step_instance`
                wrapper. REMAINING for (4): the store-only effect handler cores
                (all generic, still in `main.rs`) relocate to the kernel too, then
                the `InstanceStepMachine` drives `step_instance_generic` + serial
                effect dispatch, raising `NeedsIo(Http)` for the HTTP effects (which
                lands the agent/coerce sans-IO wiring + the workflow-invoke/agent
                recursion at the same time). **DESIGN FORK (WorkerOptions surface)
                RESOLVED 2026-07-03, commit b84ba6d:** the option-taking cores read
                only `provider` + `outcome.is_failed()`, so the fork collapsed to a
                tiny host-neutral `whipplescript_kernel::effect_config::EffectConfig
                { provider, outcome_failed }` (Option A, projection form — no 155-site
                split). `WorkerOptions::effect_config()` projects it natively; a DO
                builds one from bindings. The 5 store-only cores now take
                `&EffectConfig`; event/human/queue are fully WorkerOptions-free
                (capability still carries `LoadedPackageLock`). REMAINING for (4):
                relocate the store-only cores + their ~2.4k-line pure-helper closure
                (`effect_failure_base` / `file_path_policy_error` /
                `effect_allow_globs` / import-export codecs) into the kernel, resolve
                capability's `LoadedPackageLock` tie. The
                **`InstanceStepMachine` is BUILT (commit c10b07f):**
                `whipplescript_kernel::instance_machine` — the instance fixpoint as
                a resumable sans-IO `StepMachine` (advance rules → terminal?/park? →
                run ready effect → Done loops, NeedsHttp suspends with
                `Outcome::NeedsIo(Http)` and resumes next step; in-flight effect held
                in `self` so a DO eviction loses nothing). Mirrors
                `InstanceSchedulerLifecycle.tla`; 3 mock-driver tests
                (store-only→terminal, HTTP suspend+resume, park) — kernel 207,
                native+wasm, clippy -D. The rule pass + effect execution sit behind
                an `InstanceDriver` seam. **Native `InstanceDriver` binding BUILT
                (commit 8e07bd5):** `NativeInstanceDriver` (main.rs) wires
                advance_rules→`step_instance_generic`+status,
                next_ready_effect→`claimable_effects`, run_effect→the 11 store-only
                handler cores (dispatched by kind, all settle `Done`; HTTP/subproc/
                recursion tail errors clearly). Compiles + clippy -D clean; the
                native handlers run HTTP to completion internally so it never
                suspends (only the DO binding does). **VALIDATED END-TO-END
                (commit 6141c27):** a bin test starts a real store-only workflow and
                `run_instance_via_machine` drives it through the `InstanceStepMachine`
                over `NativeInstanceDriver` to `InstanceOutcome::Terminal`, with the
                durable status confirmed `completed` — the same terminal the dev loop
                reaches. So the instance scheduler is proven on real components (rule
                pass + store + machine), not just the mock-driver unit tests. Chunk 4
                is functionally COMPLETE. The only OPTIONAL remainder: make the
                machine the DEFAULT native executor in `dev`/`worker`/`step` (a
                behavior-neutral swap of a working path, whose real payoff is the DO,
                not native) — deferred as low-value;
            (5) wire the DO host (`RuntimeKernel<DoSqliteStore>` + `FetchHost`) to
                the wasm-bindgen surface below — **DONE.** `DoInstanceDriver`
                (commit e0b68bc) is the DO's `InstanceDriver`; `DurableInstance`
                (9aabc96) holds it across `step()` calls; `WasmDurableInstance`
                (b4724ae) is the `#[wasm_bindgen]` boundary; the whole path is
                live-validated over `node:sqlite` (56ae999). Only `wrangler deploy`
                against a live edge DO remains.
      - [x] The `wasm-bindgen` surface the shell imports — **BUILT + live-validated
            in-repo** (commits 9aabc96 `DurableInstance` create/step/status handle,
            b4724ae `WasmDurableInstance` `#[wasm_bindgen]` create/step/status,
            56ae999 the end-to-end validation). `do_wasm.rs` (`#[cfg(target_arch =
            "wasm32")]`) is the JS↔Rust boundary: `WasmDurableInstance` wraps
            `DurableInstance<JsDoSql>`, `JsDoSql` implements `DoSql` over the JS
            `DoSqlBridge` (`state.storage.sql`), and the step protocol marshals the
            `fetch` request out / response in as JSON. This drives
            `RuntimeKernel<DoSqliteStore>` through the `InstanceStepMachine`, with
            coerce AND agent creds flowing in via `create`'s `coerce_config_json` /
            `agent_config_json` (DO secrets). **The DO agent model client is now
            wired (commit 42194cf):** `kernel::harness_model::MessagesApiClient` is a
            transport-free `HttpModelClient` (config-only, reusing the native
            `build_request`/`parse_response`), so an agent turn suspends on the real
            `/v1/messages` request and resumes to a terminal — validated in-repo over
            `node:sqlite` (validate.cjs's third case) exactly like coerce. What is
            NOT yet wired: an agent turn that requests **tools** needs a DO
            `ToolExecutor` over an HTTP sidecar (the async-tool boundary — genuine
            design work, not wiring), and routing the delivery/re-entry seams through
            the E2-DYN marker door on the deployed surface. **The only truly
            infra-gated remainder is `wrangler deploy` against a real edge DO + live
            creds** (5d below).
            **Concrete chunk-5 map (found 2026-07-03):** the kernel's
            `InstanceStepMachine` + `InstanceDriver` seam (built + validated in
            chunk 4) is exactly what the DO plugs into. Four concrete pieces, three
            code (writable/wasm-buildable/mock-testable) + one live:
            (5a) **`Coordination` + `WorkItems` on `DoSqliteStore` — DONE.**
            `DoSqliteStore` now impls all three store traits, so
            `step_instance_generic` (the whole rule pass) can run over the DO store.
            `WorkItems` (commit 5eb515c: 8 methods + `items`/`item_counter` schema);
            `Coordination` (commit df91ec5: 9 required methods — slot/TTL leases,
            append-commute ledgers, atomic counter consume w/ lazy reset). Both over
            single-writer atomicity (no txn), verified against the rusqlite
            `RusqliteDoSql` mock (host-do 29 tests, clippy -D, native+wasm).
            **DO-schema finding:** the coordination `leases` table name-COLLIDES with
            the runtime store's effect `leases` (separate files natively, one store on
            the DO) → coordination tables prefixed `coord_*` on the DO; the live
            migration must adopt that;
            (5b) **DO-reachable effect handler cores — STARTED (commit 9124f67).**
            Pattern established: lift each store-only core into
            `kernel::effect_handlers` (host-neutral, `EffectConfig`-only) so both
            `InstanceDriver` bindings dispatch it. event+human+queue+coordination+file(read/write/import) cores lifted to kernel::effect_handlers (9124f67,8bdfe2c,36422c7); DO dispatches ALL 8 store-only families incl. file (via FileStore seam, b106095). notify lifted via DeliveryGovernance projection (27b5637); capability lifted via CapabilityContract projection (d88630d) — ALL 10 store-only families execute on the DO. **coerce HTTP effect DONE + PROVEN (6c52884):** DoInstanceDriver dispatches `coerce` — build_coerce_call_parts+build_request→EffectStep::NeedsHttp→(fetch)→parse_response→settle_coerce_result, every piece host-neutral in the kernel (c543428/659c933); test drives a when-started→coerce→complete workflow to Terminal with a fake Anthropic fetch response. The DO SUSPENDS a real provider effect on fetch + RESUMES to terminal — DR-0033's crux, proven in-repo. coerce PROVEN (6c52884) + agent PROVEN (d89089a: snapshot/restore eviction-safe multi-round, DoInstanceDriver dispatch). THE DO EXECUTES ALL EFFECT FAMILIES. **agent turn model client DONE (commit 42194cf):** `MessagesApiClient`
            (transport-free `HttpModelClient`, reuses native build/parse) is wired
            through `create`'s `agent_config_json`; a no-tool agent turn suspends on
            the real `/v1/messages` request + resumes to terminal, live-validated
            over `node:sqlite`. REMAINING HTTP: (a) coerce/agent LIVE creds from the
            DO secrets plane (infra); (b) an agent turn requesting **tools** needs a
            DO `ToolExecutor` over an HTTP sidecar (async-tool boundary — genuine
            design work); workflow_invoke; exec(native-only, DR-0033 Decision 7);
            (5c) **`DoInstanceDriver` DONE (commit e0b68bc):** the DO counterpart to
            `NativeInstanceDriver` over `RuntimeKernel<DoSqliteStore>` — implements the
            `InstanceDriver` seam (advance_rules→`step_instance_generic`,
            next_ready→`claimable_effects`). **PROVEN: the DO drives a real workflow
            (minimal-noop, `when started`→complete) to `completed` through the
            `InstanceStepMachine`, verified against the rusqlite mock** (host-do 30).
            So the DO runs the instance scheduler over its store for effect-free
            workflows. **5c COMPLETE** — the wasm-bindgen `create`/`step`/`status`
            surface is BUILT (`do_wasm.rs`, commits 9aabc96 + b4724ae) and, contrary
            to the earlier "only on a live DO" note, was exercised in-repo by wiring
            the JS `DoSqlBridge`/`fetch` callbacks to `node:sqlite` + a canned
            provider response (commit 56ae999) — the wasm module drives real
            workflows (store-only AND coerce suspend/resume) through the real
            boundary. 5b's effects execute through `run_effect`;
            (5d) **live validation.** In-repo end-to-end validation is **DONE**
            (commits 92fb4bf worker+DO shell + coerce-creds wiring, 56ae999 the
            validation harness): the real wasm-bindgen module runs real workflows —
            effect-free → `completed` in one step, and coerce → `needs_http` →
            (fetch) → terminal across two steps — over real SQLite (`node:sqlite`
            standing in for `state.storage.sql`) in a real JS runtime (Node 24),
            driven by `worker/validate.cjs`. The DR-0033 sans-IO suspend/resume crux
            executes through the deployed code path. **The sole remaining part is
            the literal Cloudflare deployment** — `wrangler deploy` against a real
            edge Durable Object + `wrangler secret put` for provider creds — which is
            provisioning, not code (Jack's "plug into infra at the end"); it deploys
            already-live-validated code with no remaining in-repo engineering.

### Phase 6 — Scheduling + config on the DO
- [~] Seams landed in `whipplescript-host-do` (real, tested, native + wasm):
      `Alarms` trait (single-wake-up scheduler — clock-source/timers set the next
      due time here instead of an external poller; the Worker `alarm()` handler
      steps the instance) and `Secrets` trait (config/credentials plane, no
      dotfiles). Tested (`alarms_hold_one_wakeup_and_secrets_resolve_config`).
- [~] Wire them into the runtime — **timers/deadlines LIVE 2026-07-07**: the
      native dev loop's due-time pass lifted to `kernel::time_pass` (CLI
      delegates; behavior parity via the full control_plane suite);
      `InstanceDriver` gains `advance_time(now)`/`next_due_unix_ms` (injected
      clock — the core never reads wall time); `DoSqliteStore` computes the
      earliest pending due instant; `Parked{next_due_unix_ms}` rides the wasm
      step JSON; the worker shell persists a bootstrap, sets
      `ctx.storage.setAlarm` on park, and its `alarm()` handler re-enters —
      **validated autonomously under wrangler dev/workerd** (timer workflow
      parks → real DO alarm fires untouched → due-time pass + rule pass →
      completed). `DurableInstance::create` is now get-or-create (rehydrates
      THE instance instead of minting a second). Secrets: the Worker
      env/secret binding path feeds provider configs (`.dev.vars` locally,
      `wrangler secret put` live) — the config plane in practice; the
      kernel-side `Secrets`-trait read remains a refinement. **Clock sources LIVE on the DO 2026-07-07**: the
      whole native clock pass (interval + calendar/at, DST-correct, missed
      policies) lifted to `kernel::time_pass::resolve_due_clock_sources`
      (CLI delegates) and wired into the DO's `advance_time`;
      `next_clock_due_unix_ms` computes the next occurrence (interval math +
      forward calendar scan) and joins the effect due-time in the alarm
      wake-up. chrono/chrono-tz with the pinned feature set compile for
      wasm32 (the data-only survey resolved — no clock reads in the lifted
      code). Tested: an `every 30s` source parks with the next tick as the
      wake-up and the alarm re-entry admits the signal fact to a terminal.

### Phase 7 — Large-object tier (designed now, built later)
- [~] Seam landed in `whipplescript-host-do` (real, tested, native + wasm):
      `ObjectStore` trait + `TieredFileStore` — one `FileStore` surface that
      routes writes by size (small → inline `DoStorage`; ≥ threshold → spilled
      `ObjectStore`), keeping each file in exactly one tier so reads are
      unambiguous (DR-0033 Decision 4). Tested
      (`tiered_file_store_routes_by_size_and_keeps_one_tier`).
- [~] Back `ObjectStore` with a real platform store (content-addressed keys,
      streamed import/export via a data-plane worker, presigned URLs for
      client↔storage transfer, isolate never buffers bytes).
      **Deferred-by-design (enterprise-tier / later).** The `ObjectStore` +
      `TieredFileStore` seam is landed and tested (native + wasm), with the v1
      default threshold now wired (see the open-question above). The real backend
      is gated on the platform object store + the versioned workspace
      (un-tie P1) — until those exist there is nothing to bind it to, and native /
      OSS backs files with local fs by design. **When:** enterprise object-store +
      un-tie P1.

### Phase 8 — Sidecar compute plane (designed 2026-07-04; NOT built)

**Design SSOT: `spec/compute-plane-design-note.md`** (design pass
2026-07-04, four forks settled: two service classes with a workspace-wide
delta-kernel result cache; Class-A pool owned by the *workspace* DO —
revising the earlier 1:1-per-instance sketch; Class-B = container-per-turn
with **hibernatable WebSocket from day one**, satisfying Decision 6's
reserved case by decision; one workspace image; default-deny IFC span;
priority classes production > working > counterfactual). Platform state
verified 2026-07-04 (limits 15×'d Feb 2026; autoscaling still unshipped —
fixed-size `getRandom` pools). Open build work:

- [x] Class-A executor pool: workspace-DO-owned, `getRandom`-routed lite/
      basic instances; priority queue (production > working >
      counterfactual); manual size knob w/ working zero-config default.
      BUILD COMPLETE 2026-07-08 (pool + routing + priority queue all in;
      the sole remainder is PRODUCTION ENABLE — an account/billing decision,
      not build work: `whip deploy` ships this exact config when taken;
      containers bill only while running, and getRandom starts them on
      demand, so a deployed-but-idle workspace runs none). Note: pool
      ownership sits at the worker level pending the workspace DO (the
      broker refinement). Build record: (a) `whip executor` = the
      Class-A sidecar (crates/whipplescript-cli/src/exec_server.rs;
      `whip-executor/1` wire — sha-verified inline script bytes, `{script}`
      argv slot, cleaned env, bounded timeout, 512KB stream caps; verified
      over live HTTP); (b) kernel::exec_http = the shared pure halves
      (request build / response parse / content key / typed ingest / settle —
      wasm-clean, byte-identical content keys across hosts); (c) the DO
      `exec.command` arm: store-backed script capabilities
      (`script_capabilities` on RuntimeStore ×3 impls, pin-verified at
      registration), cache-lookup-first (a hit settles with NO HTTP round —
      proven by test), miss → NeedsHttp → settle + cache record; wired
      through DurableEffectPorts/create(exec_config_json, scripts_json) +
      index.ts (WHIP_EXECUTOR_URL) and validated through the real
      wasm-bindgen boundary (validate.cjs exec round green). PRIORITY QUEUE
      v1 DONE 2026-07-08 at the executor level, model-first
      (compute-priority-queue.maude — I1 no-priority-inversion + unguarded-
      mutant bite; the model also caught that a bare AC-soup guard is
      unsound under extension matching, hence the wrapped-state rule):
      bounded EXEC_SLOTS admission gate serving production > working >
      counterfactual, transcribing the verified [serve] guard; the
      whip-executor/1 request carries `priority` (unlabeled = production);
      senders label when postures land. Workspace-DO-brokered placement =
      the production refinement. CONTAINER TIER
      PROVEN LOCALLY 2026-07-08 (b23c155): executor/Dockerfile
      (trixie-slim — glibc 2.39; bookworm exits 1) + [[containers]]
      ExecutorContainer (lite, max 4) + getRandom routing in performFetch
      (executor-sentinel URLs → container stub, not network) — full path
      validated under wrangler dev with real Docker: exec effect →
      whip-executor/1 → container ran the script → settled completed +
      cache entry recorded. Two live-run bugs fixed: bootstrap destructure
      dropped `scripts`; create() now registers script.<name> capability
      schema/binding rows (policy gate) alongside bodies. REMAINING for
      [x]: production container enable (account billing — Jack's call;
      `wrangler deploy` then ships the same config) + priority queue
      (production > working > counterfactual — needs the workspace-DO
      broker; v1 = getRandom only, documented).
- [x] Delta-kernel result cache: content-keyed memoization in the
      effect-ledger discipline (script+env+input hashes); eviction joins
      the versioned-workspace retention policy.
      DONE 2026-07-07: Maude model first (compute-result-cache.maude — I1
      at-most-one-run-per-key, I2 no-stale-serve, stale-mutant bite; in the
      gate). `compute_result_cache` table + `record_compute_result` /
      `lookup_compute_result` on `RuntimeStore` (all 3 impls + 3 schemas;
      first-writer-wins). Opt-in = `"hermetic": true` on the script-manifest
      entry (spec/script-capabilities.md); content key = sha256(script sha +
      argv + resolved env values + `WHIPPLESCRIPT_COMPUTE_ENV_HASH` epoch +
      stdin/parse). Native exec serves hits without spawning (run metadata
      `cache.hit` + content key; entry credits populating effect); successful
      completions only (failures re-run). End-to-end witness test proves the
      script spawns once. Residual: DO-side serve path lands with the Class-A
      executor box (no DO exec exists yet); eviction = retention policy (by
      design); image-digest→env-hash wiring = its own box below.
- [~] Materialization protocol endpoints: pull-missing-blobs → run →
      diff-back keyed by effect id (atomic/recorded/complete; idempotent
      by Decisions 3/4); Class-A batching (several execs per manifest
      request); branch marker + scoped secrets (P6) in the request.
      **v1 shape shipped; full pull/diff-back form deferred-by-design.**
      The remaining full pull-missing-blobs/diff-back form is gated on the
      object tier + versioned workspace (P7 / un-tie P1 — later by design);
      batching is an economics refinement gated on real contention. **When:**
      P7 object tier + un-tie P1.
      v1 SHAPE SHIPPED 2026-07-08 inside whip-executor/1 + whip-turn/1:
      Class-A materialization = sha-pinned script bytes inline, keyed by
      effect id, idempotent (cache first-writer-wins + registry re-attach);
      scoped secrets ride the request (resolved env values / provider
      config). The full pull-missing-blobs/diff-back form needs the object
      tier + versioned workspace (P7 / un-tie P1 — later by design);
      batching is an economics refinement gated on real contention.
- [x] Class-B turn containers: per-turn/per-branch controller DOs;
      hibernatable-WebSocket progress channel (frame format TBD);
      diff-back on completion via the same import.
      v1 DONE 2026-07-08, LIVE-PROVEN under wrangler dev + Docker: an agent
      turn ran WHOLE inside a real per-turn container (fixture model, real
      turn machinery + FileToolExecutor scratch dir) and settled `completed`
      through the workflow DO; the unreachable-provider case settles as a
      clean failed turn (both observed live). Shape: container-holds-the-
      turn — `whip executor` speaks whip-turn/1 (GET /turn WS streaming
      with progress frames + retained finals + {resume} re-attach = DR-0035
      B4 re-query, RFC 6455 hand-rolled; POST /turn = the blocking form the
      DO uses in v1); the DO agent arm with a TurnContainerConfig dispatches
      one round to the per-turn container (idFromName over the turn id,
      1:1) and settles via settle_provider_run_result; dispatch is
      idempotent (duplicate start re-attaches), so eviction mid-await
      recovers. Wired: DurableEffectPorts.turn → create(turn_config_json) →
      index.ts WHIP_TURN_URL sentinel routing. Residuals: the DO consuming
      the WS PROGRESS stream while hibernating (v1 blocks on POST /turn —
      the day-one WS exists container-side); workspace/branch
      materialization + diff-back join the versioned workspace (scratch-dir
      turns are the v1 posture by design).
      v1 BUILD SHAPE (recorded 2026-07-08, per the settled design —
      container-holds-the-turn, NOT DO-drives-tools): the Class-B container
      runs the whole owned agent turn NATIVELY (the image already carries
      the full whip binary — harness loop, tools, provider HTTP all run
      in-container against a scratch dir); the workflow DO opens a
      hibernatable WebSocket to the per-turn container, receives progress
      frames, and settles the final result through the existing
      `settle_provider_run_result` seam (built for exactly this: outcome
      computed elsewhere, settled without re-running). Scoped provider
      secrets ride the turn request (P6). DR-0035 B4 re-query lands here:
      the container outlives any one DO invocation, so reattach = re-query
      turn state over the WS. Frame format to pin during build (JSON lines:
      progress/tool-event/final). Locally validatable the same way as
      Class-A (wrangler dev + Docker). Pieces: container turn server (WS
      endpoint on `whip executor` or a sibling mode), DO WS accept +
      hibernation wiring, settle path, per-turn container class.
      Workspace/branch materialization joins when the versioned workspace
      exists (scratch-dir turns are the v1 posture).
- [~] **Image digest = environment hash** wiring: workspace image
      declaration → digest into generator-hash ambient config; rolling
      redeploy surfaces as a warm-start epoch.
      **Deferred-by-design: gated on the production container build/push.**
      The real image-digest wiring needs the workspace container image to be
      actually built and pushed to a registry (which produces the digest). The
      v1 proxy is in place — `WHIPPLESCRIPT_COMPUTE_ENV_HASH`
      (sha256 of the Dockerfile + staged whip binary) already feeds the
      compute-result-cache content key (P8-1) and stands in for the image
      digest. **When:** production container enable — the digest replaces the
      proxy hash at that point; no in-repo engineering blocks it, only the
      registry push.
- [~] IFC span enforcement: default-deny egress + allowlists derived from
      the exec-grant declarations; verify counterfactual execs are
      network-denied by default on this host.
      **Split: whip-side isolation DONE; platform egress-deny deferred to
      production container enable.** The executor already enforces what whip
      CAN see — cleaned child environment (declared env + PATH only,
      stronger than native exec), sha-pinned bytes, no ambient secrets in
      the container beyond the request's scoped values. Network egress
      default-deny is a PLATFORM property (containers have outbound network
      by default; Cloudflare per-container egress policy is the enforcement
      point when exposed) — the design note's recorded asymmetry stands:
      sidecar network residual contained-but-not-denied in v1. **Whip-side
      allowlist-derivation call (assessed 2026-07-09): NOT built — documented
      only.** Computing the egress allowlist from the exec-grant declarations
      and recording it (so a Cloudflare policy could consume it later) is
      *decorative without a platform enforcement point*: nothing on the DO
      host reads or enforces such an allowlist today, so deriving+storing it now
      would be dead metadata that can drift from the grants before the enforcer
      exists. The honest v1 posture is: whip-side isolation done; the allowlist
      is derived *at* production container enable, from the same exec-grant
      declarations, wired directly into the Cloudflare per-container egress
      policy that will enforce it. **When:** production container enable.
- [x] `whip deploy` v1: one zero-config command (wasm kernel + image +
      DO/bucket/pool provisioning + secrets; wrangler underneath, never
      surfaced).
      DONE 2026-07-07 (v1 = everything currently deployable): `whip deploy`
      orchestrates npm install → wasm build → optional `--set-secrets`
      (forwards ANTHROPIC/OPENAI keys from local env via piped stdin, never
      argv) → wrangler deploy; `--dry-run` validates without publishing;
      worker-dir resolution = `--worker-dir` → `WHIPPLESCRIPT_WORKER_DIR` →
      upward repo discovery. Live-verified: real deploy to
      whipplescript-runtime.jamesjscully.workers.dev + effect-free workflow
      driven to completed on the deployed worker. Fixed en route: spurious
      EPIPE failure when a pinned script exits without reading stdin.
      Residuals (join the container tier when it lands): image build/push,
      pool provisioning, object-tier bucket (P7 by design).

- [ ] DO-plane memory: port `MemoryStore` (whipplescript-store/src/memory.rs,
      the std.memory `local` provider's seam) over `DoSql` so memory pools
      work on the DO — same table shape, FTS5 replaced by the DO's LIKE-based
      lexical match if the platform sqlite lacks FTS5. Registered here per
      spec/std-memory.md MEM-3 (M7: the DO package layer lives in this
      tracker); rides the same cadence as the other `Do*` store ports.

- [x] DO-plane package bootstrap (2026-07-15, post-campaign Wave 1):
      `do_packages::register_embedded_std_packages` seeds the always-embedded
      std manifests (byte-identical to cli's set minus the feature-gated
      codex/claude adapters) into the DO store at `DurableInstance::create`
      AND `attach`, via the DO store's `register_package_manifest` — the
      native `register_locked_packages` counterpart. The
      `do_policy_block_on` exemptions for coordination
      (`lease.`/`ledger.`/`counter.`), `tracker.`, `file.`, and `signal.emit`
      are DELETED; only `timer.wait` stays runtime-resolved, matching native
      exactly. Evidence: `do_package_bootstrap_seeds_admission_rows_for_std_effect_kinds`
      (all seven gated kinds gain a provider row; idempotent re-seed),
      `durable_instance_admits_a_coordination_effect_through_the_real_gate`
      (lease.acquire drives create→step→terminal through the seeded gate),
      and the existing `branch_bound_instance_dispatches_file_effects…` file
      e2e now runs through the real gate. host-do 75 + store 226 green; wasm32
      `--no-default-features --release` build green. **Harness note:** `npm run
      validate` (worker/validate.cjs) is currently red on a node-ESM vs
      wasm-bindgen-CJS glue mismatch (`exports is not defined in ES module
      scope`) — a worker/ config/tooling drift (node 24 + `"type":"module"`),
      independent of this Rust change and in the DR-0042-owned worker/ dir;
      the substantive DO admission logic is proven by the Rust suite above.

---

## Open questions / risks

- [~] Per-alarm CPU budget: large rule evaluations may need chunking across
      alarms. **Mechanism confirmed, threshold deferred to production load.** The
      pass model is already re-entrant and resumable: the DO's `alarm()` handler
      re-enters `WasmDurableInstance::step`, which drives `InstanceStepMachine`
      (`kernel::instance_machine`) forward from durable state; a pass that parks
      at a fixpoint (`InstanceOutcome::Parked{next_due_unix_ms}`) records a
      wake-up and the next alarm continues it (`do_worker.rs` timer re-entry,
      proven by `timer_workflow_parks_with_next_due_then_alarm_reentry_completes`).
      So a rule evaluation that must be split across alarms *can* be — the
      machinery to suspend/resume between alarms exists and is exercised. What is
      NOT settled is the *chunk threshold* (how many rules/effects to run before
      voluntarily parking to stay inside a wall-clock/CPU budget): that is an
      operational tuning value that only real-workload profiling on the edge can
      set, so it is **deferred-with-cause to production load** — no in-repo
      code decision remains.
- [x] Provider idempotency-key coverage matrix (Anthropic vs OpenAI endpoints).
      **Investigated 2026-07-09; header WIRED 2026-07-09 — the tracker open-questions
      list is the home DR-0033 Decision 3 points to** ("stated in the guarantee
      report (see the tracker's open-questions matrix)"). Every provider HTTP path
      now sends a stable per-effect **`Idempotency-Key` request header** derived from
      the effect's durable identity (resume-stable by construction). Per endpoint:

      | Effect | Provider endpoint | Builder | `Idempotency-Key` header | Residual on eviction+resume |
      |---|---|---|---|---|
      | coerce Anthropic | `POST {base}/v1/messages` | `coerce_native::build_anthropic_request` | **Sent** (via `with_idempotency_key`) | Anthropic ignores it — duplicate provider call still possible |
      | coerce OpenAI | `POST {base}/v1/responses` | `coerce_native::build_openai_request` | **Sent + honored** | deduped provider-side (~24h window) |
      | coerce OpenAI/codex | `POST {base}/backend-api/codex/responses` | `coerce_native::build_codex_request` | **Sent + honored** | deduped provider-side (~24h window) |
      | agent Anthropic | `POST {base}/v1/messages` | `harness_model::build_anthropic_request` | **Sent** (from Decision-7 `cache_key`); `cache_control` breakpoint is separate *prompt caching* | Anthropic ignores it — duplicate provider call still possible |
      | agent OpenAI | `POST {base}/v1/responses` | `harness_model::build_openai_request` | **Sent + honored** (from `cache_key`); `prompt_cache_key` in the body is separate *prompt caching* | deduped provider-side (~24h window) |

      The key = `idempotency_key([instance, effect, "coerce"])` for coerce and the
      run/effect id (`cache_key`) for the agent turn — a pure function of durable
      identity, unchanged across suspend/resume/re-dispatch (this is exactly the
      `IdemKeyStable` property in `ResumableEffectLifecycle.tla`). The header is
      skipped when the key is empty (fixture / no-key path), so canned-response tests
      stay byte-identical. **OpenAI + codex honor `Idempotency-Key`** — the resumed
      duplicate after an eviction is absorbed provider-side, closing the
      double-billing residual on those paths (exactly-once provider execution within
      the provider's idempotency window). **Anthropic** does not document an
      idempotency header for `POST /v1/messages`; standard HTTP APIs ignore unknown
      request headers, so sending it is harmless (no rejection, no dedup) — the honest
      **at-least-once external delivery** stated in DR-0033 Decision 3 and proven in
      `AtLeastOnceLowerBounds` still holds for Anthropic until (or unless) it supports
      idempotency. On every path the store-side effect idempotency key
      (`idempotency_key([instance, effect, ...])`, e.g. `do_store.rs`) still bounds the
      outcome to **exactly-once settle** — one terminal in the ledger regardless of
      duplicate dispatches. **Residual is now scoped to Anthropic only; OpenAI/codex
      closed.**
- [x] Threshold + spill policy for file tiering (Decision 4). **Decided +
      wired 2026-07-09.** `TieredFileStore` previously took a caller-supplied
      `threshold_bytes` with no default (only the 8-byte test value ever set it).
      Added `DEFAULT_TIER_THRESHOLD_BYTES = 128 * 1024` and a
      `TieredFileStore::new(storage, objects)` constructor that uses it
      (`crates/whipplescript-host-do/src/lib.rs`). **v1 default = 128 KiB:** keeps
      the common small structured-I/O case (config, transcripts, small JSON) on the
      fast transactional DO-SQLite path, while staying well under DO SQLite's
      practical per-value ceiling (~2 MiB) so large inline blobs don't bloat the
      row cache / write-amplified transaction. **Optional size hint NOT exposed in
      v1** (recommended, documented on the constant): the byte length is known at
      write time, so the writer never needs to pre-declare it; the hint is only
      worth adding if a workload must pre-place a file whose final size the writer
      can't yet see — revisit then. `threshold_bytes` stays a public field so a
      caller can still override. Tested
      (`tiered_file_store_new_uses_the_default_threshold`).
- [x] workflow-invoke is already cross-pass/store-only — **confirmed 2026-07-09:
      needs no step-machine treatment.** Traced `run_workflow_invoke_effect`
      (`crates/whipplescript-cli/src/main.rs:23296`): the handler performs **no
      external I/O** — no `fetch` / `ureq` / `NeedsIo` / `NeedsHttp`. It resolves
      the target from bindings, starts/looks-up the child instance
      (`start_child_workflow_instance_in_package`, `record_workflow_invocation`),
      then drives the child across passes with `step_instance` (rule pass) +
      `run_worker_once` (effect executor) and observes the terminal purely through
      store reads (`get_instance`, `workflow_terminal_summary_from_store`). This
      matches DR-0033 Decision 2, which classifies `workflow-invoke` among the
      store-only "never HTTP" effects. The child's *own* HTTP effects are the
      child instance's effects, driven by the child scheduler's passes — they do
      not make the parent's workflow-invoke effect suspend on a `fetch`. (Distinct,
      separately-tracked concern: how the *child's* HTTP effects get driven on the
      DO — the Phase-5 chunk-3/4 recursion note; that is about child effect
      dispatch, not about workflow-invoke needing a step machine, and does not
      reopen this item.)
- [~] Checkpoint cut × coordination store: decide where coordination-state
      snapshot/restore lives in the Phase 3 traits so the restorable-context
      consistent cut can pin it (see Downstream-customer note above).
      **Deferred-with-cause: design-placement decision, gated on the checkpoint
      mechanism landing.** This is the same remnant as the Phase-3 `[~]`
      snapshot/manifest item (line ~250) and belongs to a separate DR
      (`decision-records/restorable-context.md`, pre-ADR). No checkpoint/undo
      mechanism is built yet, so there is no real consumer to shape the capability
      against — building a snapshot API now would be speculative. **Where it will
      land when the checkpoint mechanism arrives:** a cheap *position-capture*
      capability on the `Coordination`/`WorkItems`/`RuntimeStore` traits (per the
      Downstream-customer note's "cut spans both planes — workspace-plane stores by
      position / high-water mark, which their monotonicity makes a per-store
      integer" — a snapshot capability shaped as high-water-mark capture, not deep
      copy), joining the manifest + event-log-index cut. Do NOT build a checkpoint
      system here.
