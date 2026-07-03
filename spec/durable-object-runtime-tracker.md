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
   evidence semantics (research note, "Evidence identity — the slice hash" +
   "Architecture" sections). Phase 3 (store behind
   a trait) is the cheap moment to shape this: cut the coordination trait with
   a snapshot/manifest capability in view.
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
- [x] Stdio sidecars (codex/claude/pi) confirmed native-only opaque drivers —
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

### Phase 5 — DO host crate + TS shell (the wasm target)
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
- [~] `whipplescript-host-do` crate started — the DO binding's building blocks,
      built for wasm against the wasm-clean core:
      - [x] `FetchClient` (the DO's `fetch`) + `FetchHost` — the sans-IO
            `HostDriver` that fulfills a `NeedsIo(Http)` through the isolate's
            fetch. Any effect step machine (`coerce`, an agent turn) runs on the
            DO through it. Tested (`fetch_host_drives_a_step_machine_over_the_do_fetch`).
      - [x] `DoStorage` (the DO's synchronous SQLite) + `DoFileStore` — the file
            seam (`FileStore`) backed by DO storage (small files inline, flat key
            space). Tested (`do_file_store_round_trips_through_the_file_seam`).
      - Crate builds native (2 tests green, clippy `-D warnings` clean) **and**
        `--no-default-features --target wasm32-unknown-unknown`.
      - [~] TS/Worker shell landed (`worker/`): `src/index.ts` (the
            `WhippleInstance` DO class running the sans-IO drive loop — step the
            synchronous Rust machine, await `fetch` on `NeedsIo(Http)`, re-enter),
            `wrangler.toml` (DO SQLite class + R2 large-object bucket + secrets),
            `package.json`/`tsconfig.json`, and a README mapping each Rust host
            trait to its DO primitive. Deployment scaffold — real code, not built
            here.
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
      - [~] **Instance-level sans-IO scheduler (full lift).** The native top-level
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
                `run_capability_effect` (3a, 2b1b255), `run_loft_effect`,
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
                `&EffectConfig`; event/loft/human/queue are fully WorkerOptions-free
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
                the wasm-bindgen surface below.
      - [ ] The `wasm-bindgen` surface the shell imports (`createInstance`/`step`/
            `snapshot`) wiring `RuntimeKernel<DoSqliteStore>` + `FetchHost` to the
            drive loop; routing every new delivery/re-entry seam through the E2-DYN
            marker door. **Needs a live Cloudflare DO runtime.**
            **Concrete chunk-5 map (found 2026-07-03):** the kernel's
            `InstanceStepMachine` + `InstanceDriver` seam (built + validated in
            chunk 4) is exactly what the DO plugs into. Four concrete pieces, three
            code (writable/wasm-buildable/mock-testable) + one live:
            (5a) **`Coordination` + `WorkItems` on `DoSqliteStore`** — it impl'd
            `RuntimeStore` ONLY; `step_instance_generic` needs all three, verifiable
            against the rusqlite-backed `RusqliteDoSql` mock like the 87-method
            RuntimeStore port. **`WorkItems` DONE (commit 5eb515c):** all 8 methods +
            the `items`/`item_counter` schema ported to DoSql, single-writer
            atomicity (no txn), tested (host-do 28). **REMAINING: `Coordination`**
            (leases/ledgers/counters) — the bigger half (~498-line native impl: slot
            leases + TTL, ledger partitions, counter periods, shared-owner
            partitioning) + its `leases`/`ledger_entries`/`counters` schema; then
            `step_instance_generic` (hence the whole rule pass) runs over
            `DoSqliteStore`;
            (5b) **DO-reachable effect handler cores** — the store-only cores live
            in `main.rs`; the DO's `InstanceDriver::run_effect` needs them in a lib
            (relocate to kernel, or a host-do effect module) + the two HTTP effects
            wired to `FetchHost` (the `NeedsHttp` path the machine already supports);
            (5c) **`DoInstanceDriver` + wasm-bindgen `createInstance/step/snapshot`**
            over `RuntimeKernel<DoSqliteStore>` + `FetchHost`, exported from
            `whipplescript-host-do` for the TS shell;
            (5d) **live validation** on a real Cloudflare DO (the only truly
            infra-gated part — Jack's "plug into infra at the end").

### Phase 6 — Scheduling + config on the DO
- [~] Seams landed in `whipplescript-host-do` (real, tested, native + wasm):
      `Alarms` trait (single-wake-up scheduler — clock-source/timers set the next
      due time here instead of an external poller; the Worker `alarm()` handler
      steps the instance) and `Secrets` trait (config/credentials plane, no
      dotfiles). Tested (`alarms_hold_one_wakeup_and_secrets_resolve_config`).
- [ ] Wire them into the runtime: clock-source + timer effects call `Alarms`;
      provider credential resolution reads `Secrets`; `chrono` std → data-only
      `chrono-tz`; `Instant` → injected virtual clock. *(Needs the live DO alarm
      API + Worker secrets binding.)*

### Phase 7 — Large-object tier (designed now, built later)
- [~] Seam landed in `whipplescript-host-do` (real, tested, native + wasm):
      `ObjectStore` trait + `TieredFileStore` — one `FileStore` surface that
      routes writes by size (small → inline `DoStorage`; ≥ threshold → spilled
      `ObjectStore`), keeping each file in exactly one tier so reads are
      unambiguous (DR-0033 Decision 4). Tested
      (`tiered_file_store_routes_by_size_and_keeps_one_tier`).
- [ ] Back `ObjectStore` with a real platform store (content-addressed keys,
      streamed import/export via a data-plane worker, presigned URLs for
      client↔storage transfer, isolate never buffers bytes). Enterprise-tier
      deliverable; native/OSS backs files with local fs. *(Needs the platform
      object store.)*

### Phase 8 — Sidecar compute plane (registered 2026-07-03; NOT designed)

The pure-DO host solves the **orchestrator + storage plane**; real workflows
also trigger exec/agent compute that cannot live in the isolate (Decision 7:
subprocess effects are HTTP to a container sidecar). This phase registers
that compute plane as open intent — cloud deployment is only partially
solved without it. A design pass is required before any box is checked;
shared design with `versioned-workspace-research-note.md` §9–§10 (the
materialization boundary must be **evidence-grade**: atomic, recorded,
complete imports).

- [ ] Sidecar lifecycle: container-per-DO controller model (Cloudflare
      Containers pairs each container instance with a controlling DO — maps
      1:1 onto the workflow-instance DO; verify current platform state,
      knowledge as of early 2026).
- [ ] Materialization protocol over HTTP: sidecar pulls only missing
      content-addressed blobs (R2/object tier), materializes the branch
      manifest, execs, pushes the diff back keyed by effect id — idempotent
      by Decisions 3/4; just another step-machine effect.
- [ ] **Image digest = environment hash**: the container image digest slots
      into generator-hash ambient config (experimentation note §7) — a
      toolchain bump becomes a visible warm-start, never silent.
- [ ] Economics: cold starts (seconds) + billed-while-running → exec
      batching, warm-pool policy, DO↔sidecar placement affinity.
- [ ] IFC span: egress doors must be enforced where whip cannot see
      syscalls — container network policy (default-deny egress + per-grant
      allowlists) as the backstop; stronger than native exec today, weaker
      than owned-harness — design deliberately, don't inherit accidentally.
- [ ] `whip deploy` packaging surface: DO bindings, R2 bucket, container
      images, secrets, wrangler artifacts — an undesigned product surface.

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
