# Std-package post-campaign tails — sequenced tracker

Status: active — **all buildable waves (0-3) SHIPPED 2026-07-15**; only the
demand-gated (`[~]`, awaiting their documented triggers) and design-heavy
(`[~]`, awaiting a ruling/design) items remain, which is their correct
terminal state. Registered 2026-07-15 at the std-package campaign close-out
(commit 171a9e0; constitution checklist + design-tracker Current Rule both
flipped). This tracker sequences everything the campaign deferred with cause.
The campaign itself is CLOSED — do not reopen per-package v1 scope here; each
item below cites the design doc that owns its definition.

Buildable-wave commits: Wave 0 3e91a1a · Wave 1 91f91dd · Wave 1b 58ed654 ·
Wave 2 699a281 · Wave 3 36660bc.

Sequencing principle: items split by NATURE, not just priority — **buildable
now** (Waves 0-3, in order), **demand-gated** (built when their documented
trigger fires, never speculatively), and **design-heavy** (need a Jack ruling
or a design note before any code).

## Wave 0 — hygiene batch (mechanical, pre-v0.4-cut)

- [x] (2026-07-15, gates: improve_loop 18 + coerce suite) improve.rs re-point: `native_coerce_turn` consumes
      `ResolvedCoercionConfig` directly; delete the documented
      `From<ResolvedCoercionConfig> for NativeCoerceConfig` compat view in
      coerce_runtime.rs (left because improve.rs was another session's file
      during the coercion build — see 484a2b7's deferral note).
- [x] (2026-07-15, both rows removed from 0001; store 226 + messaging suites green) Dead seed row: `binding_messaging_send_builtin` (provider
      `builtin-messaging`, migrations/0001:~467) matches no channel provider
      since binding-driven dispatch (21bb4e4). Pre-release one-way removal.
- [x] (2026-07-15; query fixed to `key` + error no longer swallowed; host-do 73) do_worker.rs vacuous branch test: the "plain plane untouched" assertion
      queries `files WHERE path LIKE …` but the column is `key`, so it passes
      via `unwrap_or(0)` on the SQL error (flagged in the f1876ca build).
- [x] (2026-07-15; Trackers section renamed w/ honest status vocabulary — durable open/closed/canceled + in_progress overlay; anchors updated; flagged spec refs → symbol names; docs gates green) Doc staleness sweep: docs/language-reference.md "Work queues" prose
      (pre-S3 nouns, "done/cancelled" vs shipped closed/canceled),
      docs/api-reference.md "builtin work-queue tracker" line, and stale code
      line-references in spec/std-messaging.md / spec/std-files.md /
      spec/std-agent.md.

## Wave 1 — DO package bootstrap (first real build; cross-cutting unlock)

- [x] (2026-07-15; do_packages.rs + create/attach seeding; exemptions deleted,
      only timer.wait waved through; host-do 75 incl. bootstrap + coordination-
      admit + file e2e, store 226, wasm32 build green) Seed the embedded std
      manifests at DO instance creation (the native `register_locked_packages`
      counterpart), then remove the DO admission-gate exemptions native already
      vacated. Canonical row: durable-object-runtime-tracker.md "DO-plane
      package bootstrap" (flipped there with full evidence). NOTE: `npm run
      validate` (worker/validate.cjs) is red on a pre-existing node-ESM vs
      wasm-bindgen-CJS glue mismatch introduced by commit 302e2c6's
      `"type":"module"` (DR-0042 worker session, off-limits dir) — the
      substantive admission logic is proven by the Rust suite instead.
- [x] (2026-07-15; host-do/do_memory.rs DoMemoryStore + shared kernel
      run_memory_capability + DO dispatch wiring; 4 unit tests + DO learn e2e;
      host-do 80 + kernel 348 + wasm32 green) DO-plane memory (MEM-3 port,
      MemoryStore over DoSql). Canonical row flipped in
      durable-object-runtime-tracker.md "DO-plane memory".

## Wave 2 — tracker T3: renew + claim TTL end-to-end

- [x] (2026-07-15; TrackerRenew IrEffectKind + claim-ttl clause + binding-typed
      renew disambiguation + store claim_item(item,actor,expires) across 3 impls
      + kernel/native/DO dispatch + whip issue --ttl + manifest contract fold;
      store 227, parser 307, kernel 351+18, host-do 80, bin 460, control_plane
      208, wasm32 green) Tracker T3 renew + claim TTL — the contradiction is
      resolved: `renew <claim>` lowers to `tracker.renew`, `renew <acquire>`
      stays `lease.renew`. Built per spec/std-tracker.md "T3 BUILT" note.
      Deferrals (with cause): source `renew` is heartbeat-only (finite extension
      via CLI `--ttl`; the source grammar has no duration), `renew` keeps its
      required `as` binding, DO claim-`ttl` is inert (the DO `"now"` clock stub —
      same as its coordination wait-deadline; renew heartbeat + untimed claims
      work).

## Wave 3 — coercion evidence hashers

- [x] (2026-07-15; CoerceRequest::with_evidence_hashes in kernel coerce.rs,
      used at the native fixture + DO dispatch sites; test
      evidence_hashes_are_content_derived_not_placeholders; kernel 349 + full
      battery green) Replace the placeholder `input_schema_hash` /
      `output_schema_hash` / `generated_coerce_source_hash`
      ("fixture"/"do"/"coerce") on CoerceRequest evidence with real
      content-derived hashes (H(coercion name) / H(named-args JSON) /
      H(output-type identity)). Evidence-plane honesty only — the ADMISSION-key
      commitment to the IR-synthesized schema stays effect_admission_key's.

## Demand-gated (build when the trigger fires; never speculatively)

- [~] ingress I4 HTTP listener + `path`/`auth`/`correlate` clauses. Trigger:
      first webhook demand. Then model-first (IngressDeliveryLifecycle.tla);
      the dedup + admission core (155d436) are the prepared hooks. Owner:
      spec/std-ingress.md "Deferred with cause" (2026-07-15).
- [~] messaging interactions + `source interaction`. Trigger: resolving the
      `signal_source` authorability question; natural rider on the I4 wave
      (both touch source families). Owner: spec/std-messaging.md.
- [~] lint→error import escalation (the M5 ladder ratchet). Trigger: Jack's
      call after the corpus has lived with the advisories. Owner:
      std-package-ecosystem-shape.md M5.
- [~] M6 package version semantics. Trigger: first real second-version
      demand. Owner: std-package-ecosystem-shape.md M6.
- [~] coord `acquire … wait timeout` FIFO queue / `lease order` multi-lease.
      Triggers documented in spec/std-coord.md's deferred table (BoundedWait
      model-first when the queue is demanded).

## Design-heavy (deferred-with-cause: a ruling or design note precedes any code)

These are NOT buildable-now: each is blocked on a design decision that a
hasty unilateral answer would get wrong. Dispositioned 2026-07-15 with the
concrete deliverable and decision-owner so any session (or Jack) can pick one
up; each carries a proposed direction where one exists.

- [~] messaging stdio bidirectional child (spec/std-messaging.md slice 5).
      Blocked on: the child LIFECYCLE surface (the outbound marker-line seam
      ships; a bidirectional child needs a spawn/restart/teardown contract the
      spec never fixed). Deliverable = a design note settling: (a) child
      command config — reuse the script-manifest argv+sha256 pin, or a new
      `channel { command … }` clause?; (b) restart policy on child exit
      (fail-the-effect vs respawn-with-backoff); (c) hosting — a per-workflow
      `local_daemon` vs per-effect spawn. Proposed direction: pin the child
      via the std.script manifest (argv+sha256, same TOCTOU discipline),
      per-effect spawn (no daemon in v1), child-exit = typed effect failure
      (no auto-respawn). Owner: Jack ratifies the clause shape.
- [~] probed agent feature reports (DR-0015 "Next Validation Work"). Blocked
      on: the live-probe harness shape + an evidence-schema decision (Jack).
      The compiled reports + `source` rendering shipped (155d436); this is
      purely additive (`source: "probed"`). Deliverable = decide what a probe
      RUNS (a canned turn per feature class? a capability handshake?) and the
      probe-evidence schema (per-class support + a probe timestamp + the
      provider version probed). Then a `whip doctor --probe` that fills the
      report's `source` from live results. Owner: Jack (needs live provider
      credentials + an evidence-schema ruling).
- [~] ⚑ capability-registry.md grant-plane reconciliation. A naming-boundary
      question (how the grant plane's vocabulary lines up with the capability
      registry) flagged for Jack during the tracker build — spec-only, no
      code. Owner: Jack.
- [~] ⚑ messaging manifest `providers[]` rows. The design said "no provider
      rows"; the manifest validator requires them for bindings, so the shipped
      manifest follows the memory.json precedent (the rows are the
      never-consulted `effect_providers` class, admission-inert for
      capability.call). Decision for Jack: bless the shipped shape (amend the
      design's "no rows" line) OR amend the validator to allow bindings
      without a same-manifest provider row. Owner: Jack.
- [~] ADR-0002 phase B (conflicts/heads/state-tokens, full relation kinds,
      comments/evidence, claim-strength/external sync, DO
      rebuild_projection parity) — the largest unforced item; coord's event
      vocabulary explicitly joins it (one event-sourcing campaign, not two).
      Its own future campaign, sequenced last. Owner: a dedicated campaign.

## Adjacent (other campaigns' rows, slotted here only for sequencing)

- [~] Security-audit follow-ons: native env per-provider key scoping + SSRF
      resolved-IP pinning — near Wave 0/1 if wanted in the same sweep.
      Canonical home: the security-audit record, not this tracker.
