# 0020: Blocked-Effect Binding Taxonomy

Status: implemented (v0)

**Implemented 2026-06-16.** Model (TLA+ `BindBlock`/`UnblockEffect` + invariants,
coverage + bite), store (`block_effect_binding`, idempotent + recoverable;
`policy_block_category` column; `blocked` re-claimable), worker (`run_agent_effect`
blocks before provider execution: `provider_health` on sidecar-launch failure,
`credentials` when a present config lacks a required credential ref), and
presentation (`policy_block: {category, detail}` in `whip effects`/`status`,
deriving the category from `blocked_by_*` for scheduling blocks). Tests:
`block_effect_binding_is_idempotent_and_recoverable` (store) and
`dev_native_provider_unavailable_blocks_effect_recoverably` (e2e: binding failure
blocks, recoverable, no failed run / no `agent.turn.failed`). Full gate green.

**Emitted vs defined categories:** scheduling blocks surface `capability`/`profile`/
`capacity`/`dependency` (derived from status); the native binding path emits
`provider_health` and `credentials`. `provider_config` and `enforcement` are defined
in the category enum but not emitted by the native worker path — native providers
default-launch without a binding config, and native-enforcement gaps are not a
distinct pre-execution signal here; both remain available for providers/surfaces
that need them.

**Progress (2026-06-16):** the TLA+ model step is done and verified.
`ControlPlaneLifecycle.tla` now has `BindBlock` (claimed → recoverable `blocked`,
run + lease released, non-terminal) and `UnblockEffect` (blocked → queued), wired
into `Next`, with two new safety invariants `BlockedEffectIsNotTerminal` and
`BlockedEffectHasNoLiveRun` in `SafetyInvariants`. Apalache `check --length=6`
passes (coverage — the new transitions preserve every safety invariant), and a
deliberately-broken `BindBlock` (run not released) is caught at state 2 (bite),
mirroring the `guard-commit-bite` discipline.

**Store/kernel data layer DONE & tested:** `policy_block_category` column +
`EffectView` field; `SqliteStore::block_effect_binding` (queued → recoverable
`blocked` with `{category, detail}`, idempotent — no new event on re-block,
returns the existing one); `blocked` is now re-claimable and `start_run` clears the
category on the recovery claim. Kernel `effect_status` maps `blocked` →
`EffectStatus::Blocked`. Test `block_effect_binding_is_idempotent_and_recoverable`
locks in block/idempotency/recovery. Full gate green.

**Remaining:** worker (`run_agent_effect`) categorizes the binding failure and
calls `block_effect_binding` before provider execution; `whip status`/`effects`
present `{category, detail}` (deriving category from `blocked_by_*` for scheduling
blocks); report-schema update; end-to-end test (binding failure → blocked, not
failed; recovery runs; no secret leak). Leans below apply unless overridden.

## Decision

A blocked effect must explain *why* it is blocked through one categorized reason,
and **provider-binding failures must block the effect (recoverable) rather than
fail it (terminal)**.

Concretely:

1. Worker-time binding failures — missing provider config, missing credentials,
   insufficient native enforcement, or no healthy provider binding — stop
   producing a terminal `failed` effect with a `provider_health_unavailable`
   diagnostic. They instead leave the effect **blocked** (recoverable, retried on
   the next worker pass once the binding prerequisite is satisfied), exactly as
   scheduling-time capability/capacity blocks already behave.
2. Every blocked effect carries a **structured, categorized** `policy_block_reason`
   — `{ category, detail }` — surfaced uniformly by `whip status` and the effect
   JSON. `category` is one closed enum spanning both block origins:

   ```text
   scheduling-time:  capability | profile | capacity | dependency
   binding-time:     provider_config | credentials | enforcement | provider_health
   ```

This is the "block-with-categorized-reason" option (avoid status proliferation):
the fine category lives in the `policy_block_reason` field, not in a combinatorial
set of `blocked_by_*` statuses.

## Context (current state)

- **Scheduling-time blocks already work.** `RuntimeKernel::start_run` maps
  `StoreError::PolicyBlocked` / `CapacityBlocked` to a non-terminal **blocked**
  effect, emits `TraceEvent::EffectBlocked`, and the store records a
  `policy_block_reason`. The runtime status strings `blocked_by_capability`,
  `blocked_by_profile`, `blocked_by_capacity`, `blocked_by_dependency` all collapse
  to `EffectStatus::Blocked` (kernel `lib.rs`). `whip status` already prints
  `policy_block_reason`.
- **Binding-time failures currently *fail* the effect.** In the CLI worker
  (`run_agent_effect`), when a native provider can't be bound (missing config /
  credentials / health), `unavailable_native_provider_adapter` runs a turn that
  resolves through `fail_run`, producing a **terminal `failed`** effect plus a
  `provider_health_unavailable` diagnostic. The provider-readiness layer already
  produces the raw signals we need (`missing_config_refs`,
  `missing_credentials_ref`, `missing_config_keys`) — they just feed a *failure*,
  not a *block*.
- **The gap** is therefore two things: (a) binding failures are terminal, not
  recoverable; (b) the five categories the Stage 7 acceptance lists are not
  surfaced through one uniform field.

## Rationale

- **Recoverable is correct operator semantics.** A missing credential or unhealthy
  provider is an environment problem, not a workflow error. Failing the effect
  forces a manual re-trigger and pollutes the failure path; blocking lets the
  operator fix the binding and have the worker resume — the same contract
  capacity/dependency blocks already give.
- **One categorized field beats N statuses.** The existing `blocked_by_*` status
  strings already encode scheduling categories, but extending that to
  `blocked_by_provider_config`, `blocked_by_credentials`, … multiplies lifecycle
  strings for no machine-readability gain. A single `{category, detail}` on
  `policy_block_reason` is one place for `whip status` to read, one enum to
  validate, and it unifies both origins.
- **Honest by construction.** A binding-blocked effect records the category +
  redacted detail as evidence without leaking secret values (the readiness layer
  already redacts; `policy_block_reason.detail` carries refs/keys, never values).

## Model-first plan (per the model-first methodology)

Formal tool division: this is **control-plane lifecycle**, so the model work is
**TLA+** (`ControlPlaneLifecycle.tla`), not Maude. The Maude rule-system model is
unaffected — binding is a provider/control-plane concern, not rule-commit
semantics. (`policy-capacity-retry.maude` already models capability/policy
block → release; binding blocks are the TLA analog.)

The abstract model already has a generic `"blocked"` effect state; today a claimed
run can only go `running → {completed, failed}`. Add a recoverable binding-block
transition and tighten invariants:

```text
BindBlock(e, r):                      -- new action
  pre:  runs[r] = "claimed", effects[e] = "claimed", bindingUnavailable(e)
  post: effects[e] = "blocked", runs[r] = "none", lease released,
        e NOT added to terminalEffects        -- recoverable, not terminal
```

Invariants to add / extend (coverage **and** bite):
- A binding-blocked effect is never in `terminalEffects` and can be re-claimed
  once `bindingUnavailable(e)` clears (liveness: it eventually runs or terminates).
- No effect reaches a terminal status directly from a binding block without a real
  run (safety: `blocked → completed` is unreachable without `claimed → running`).
- **Bite:** add a deliberately-wrong variant (binding failure → `completed`) and
  confirm the model finds the counterexample — mirroring the `guard-commit-bite`
  discipline so the new invariant isn't vacuous.

## Data-model and implementation changes

1. **`policy_block_reason` becomes structured.** Today it is a free string; make it
   `{ "category": <enum>, "detail": <redacted string> }` (or a category-prefixed
   string if we want zero schema churn — decide below). Update the store column /
   `EffectView`, the status/effect JSON serializers, and the report schemas.
2. **New worker-time block path.** In `run_agent_effect`, when provider selection
   yields an unavailable binding, emit a **block** (effect → `blocked`,
   `policy_block_reason = {binding category, redacted detail}`, run released)
   instead of routing through `unavailable_native_provider_adapter` → `fail_run`.
   Add a kernel entry point analogous to the scheduling block (e.g.
   `block_run(BindBlock { … })`) that emits `TraceEvent::EffectBlocked` and sets
   the reason without a terminal event.
3. **Optional single status string.** Keep the four existing scheduling
   `blocked_by_*` strings; add at most one `blocked_by_binding` umbrella for the
   worker-time origin (or reuse `blocked` and rely solely on the category field).
4. **Uniform presentation.** `whip status` and `whip effects` print
   `category` + `detail` for every blocked effect from the one field.

## Verification plan (per-piece gate)

- **TLA+**: the new `BindBlock` action + invariants typecheck and pass; the bite
  variant is caught. Run via `check-formal-models.sh` / the TLA script.
- **Rust**: (a) a missing-config/credential agent effect ends **blocked**, not
  failed, with the right `category`; (b) `whip status` shows the category+detail;
  (c) once the binding is supplied, the next worker pass runs the effect
  (recovery); (d) secrets never appear in the reason.
- **Gate**: workspace tests + `check-report-schemas.sh` (updated schema) +
  `check-formal-models.sh` + `cargo fmt --all -- --check`.

## Open questions (need your call before coding)

1. **`policy_block_reason` shape:** structured `{category, detail}` object (cleaner,
   schema churn + serializer updates) **or** a `category:detail` prefixed string
   (zero schema churn, less machine-readable)? I lean structured.
2. **Status string:** add one `blocked_by_binding` umbrella status, or keep the
   runtime status as plain `blocked` for binding blocks and rely entirely on the
   category field? I lean plain `blocked` + category (least proliferation).
3. **Enforcement category scope:** "insufficient enforcement" — is that strictly
   native-enforcement-not-available (a binding prerequisite), or does it also cover
   a profile that the endpoint refuses at bind time (which overlaps `profile`)?
   This determines whether `enforcement` and `profile` are distinct categories.
