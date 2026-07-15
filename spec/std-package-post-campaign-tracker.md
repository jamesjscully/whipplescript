# Std-package post-campaign tails — sequenced tracker

Status: active. Registered 2026-07-15 at the std-package campaign close-out
(commit 171a9e0; constitution checklist + design-tracker Current Rule both
flipped). This tracker sequences everything the campaign deferred with cause.
The campaign itself is CLOSED — do not reopen per-package v1 scope here; each
item below cites the design doc that owns its definition.

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

- [ ] The one tail whose absence is a documented contradiction (`renew` still
      lowers to LeaseRenew). Store plane shipped (WorkItems::renew_claim,
      expiry-aware readiness, tracker-lease.maude); remaining = the surface as
      ONE slice per spec/std-tracker.md "Renew disposition" (2026-07-15 note):
      claim `ttl` clause, `renew <claim>` binding-typed disambiguation
      (mirroring the shipped release split, kernel/rule_lowering.rs),
      `tracker.renew` effect kind (touches every exhaustive IrEffectKind
      match incl. flow_expand re-serialization), `whip issue … --ttl`, and the
      manifest contract row — which also supplies the SHARED renewal contract
      std-coord.md's manifest deliberately awaits (its capability trio ships
      sans contract; drift test pins the absence and flips with this slice).

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

## Design-heavy (a ruling or design note precedes any code)

- [ ] messaging stdio bidirectional child: the spec names the contract but
      not the child lifecycle surface (command config, restart policy,
      `local_daemon` hosting). Deliverable: a short design note, then the
      build. Owner: spec/std-messaging.md slice 5.
- [ ] probed agent feature reports (DR-0015 "Next Validation Work"): needs
      the live-probe harness shape + an evidence-schema decision (Jack).
      Additive once decided (`source: probed`); the report plumbing shipped
      in 155d436.
- [ ] ⚑ capability-registry.md grant-plane reconciliation — naming-boundary
      question flagged for Jack during the tracker build.
- [ ] ⚑ messaging manifest `providers[]` rows — the design said "no provider
      rows"; the validator requires them for bindings; shipped follows the
      memory.json precedent (rows are the never-consulted class). Flagged for
      Jack: bless the shipped shape or amend the validator.
- [ ] ADR-0002 phase B (conflicts/heads/state-tokens, full relation kinds,
      comments/evidence, claim-strength/external sync, DO
      rebuild_projection parity) — the largest unforced item; coord's event
      vocabulary explicitly joins it. Its own future campaign, sequenced last.

## Adjacent (other campaigns' rows, slotted here only for sequencing)

- [~] Security-audit follow-ons: native env per-provider key scoping + SSRF
      resolved-IP pinning — near Wave 0/1 if wanted in the same sweep.
      Canonical home: the security-audit record, not this tracker.
