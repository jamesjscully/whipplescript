# Admission, Idempotency, And Replay Contract

Status: normative design baseline

This is the single contract for **how any value becomes a durable typed fact**.
Every subsystem that admits data — external signals, clock occurrences, coerce
and agent-turn outcomes, package effect outputs, peer/CLI signal injection, and
`std.files` row imports — uses this contract. Before this document these rules
were assumed in many specs and defined in none; that gap is the root cause of
several runtime ambiguities.

Three invariants govern every admission:

```text
identity     every admitted fact has a deterministic idempotency key
validation   the value passes runtime-boundary validation against a locked schema
replay       the admitted fact + evidence are durable; replay reads them and
             never re-invokes the source
```

## The Admission Boundary

The only way a value becomes a durable fact is admission through the kernel.
Providers, packages, and agents never write facts directly. An admission is a
single kernel transaction:

```text
candidate value + source identity
  -> runtime-boundary validation against the locked schema
  -> valid:   append fact(s) atomically under the admission idempotency key
  -> invalid: no fact; failed effect / rejected admission + diagnostic
```

Runtime-boundary validation is the **authority** that admits data into facts —
not provider-side validation, not source claims. A streaming source may report
observations as evidence, but a durable typed fact requires a complete validated
value. (`validation: "runtime"` in a package manifest is an input alias for
`runtime_boundary`; the canonical emitted value is `runtime_boundary`.)

## Admission Identity (Idempotency Key)

Every admitted fact carries an idempotency key derived deterministically from its
source. The store enforces a unique index on `(instance_id, fact_identity_key)`
inside the admission transaction, so a re-delivered or retried admission appends
at most once.

```text
source                         idempotency key
----------------------------   -------------------------------------------------
effect terminal -> fact        H(instance_id, program_version,
                                 rule_commit_event_id, effect_node_id)
external signal admission      provider delivery id, if present;
                                 else H(instance_id, source_id,
                                        payload_canonical_hash, source_sequence)
clock occurrence               H(source_id, scheduled_occurrence_instant)
                                 (this value is the occurrence_id)
peer/CLI signal injection      peer: H(origin_instance_id,
                                        origin_effect_idempotency_key,
                                        target, payload_canonical_hash)
                                 CLI: operator-supplied or derived delivery id
typed fact-batch (import)      per row: H(effect_idempotency_key, row_index)
                                 or H(effect_idempotency_key, natural_key) when
                                 the schema declares a natural key
```

Key consequences:

- The **effect idempotency key is stable at effect creation** because it is a
  function of the committing rule and the effect's node position, not of
  not-yet-known dependency outputs. Resolved dependency outputs feed the
  *materialized input* and a separate *execution fingerprint* (below); they do
  not enter the idempotency key. This resolves the prior contradiction where the
  key was required to be both stable-at-creation and derived from resolved
  outputs.
- Payload hashes are computed over the canonical JSON form (sorted keys, no
  insignificant whitespace) so the key is portable across machines.

## Runtime-Boundary Validation

```text
running effect + locked contract + raw source value
  -> valid typed value  -> completed effect + admitted typed fact(s)
  -> invalid value      -> failed effect + validation diagnostic, no fact
```

Validation is against the contract version pinned for the run, so the same bytes
admit the same value under replay. Closed classes reject unknown fields *after*
any backend normalization (e.g. coerce schema-aligned parsing) has run; the
WhippleScript boundary is the final gate.

## Typed Fact-Batch Admission

Some sources admit **N facts from one validated outcome** (e.g. `std.files`
`import`). This is a platform primitive, not a package power:

```text
validated batch of T (size N)
  -> admit all N facts atomically in one transaction
  -> each fact carries its per-row idempotency key (above)
  -> any row invalid -> admit none, fail the effect (all-or-nothing)
  -> replay reconstructs the same N facts from the recorded outcome,
     without re-reading the source
```

A package declares that an effect produces a fact batch of `T`; the kernel owns
the atomic admission, the per-row keys, and replay. Until this primitive is
implemented, surfaces that need it (`std.files` `import`/`export`) are not
accepted.

## Determinism And Replay

Replay is **record-once**: a nondeterministic outcome is recorded as a durable
terminal event with its evidence, and replay reads the recorded outcome — it
never re-invokes a provider, model, clock, or filesystem.

```text
nondeterministic source (LLM turn, coerce, provider effect, clock read)
  -> record terminal outcome + evidence durably, once
  -> replay reads the recorded fact/outcome; the source is never called again
```

To make the recorded outcome the *correct* one to reuse, the idempotency key (or
its companion execution fingerprint) commits to every input that changes the
result: for model-backed effects (coerce, agent turns) that includes the
provider/model id, the prompt or coercion-artifact hash, and the output-schema
hash. Changing any of them is a different key, so a stale recorded outcome is
never reused under a changed contract.

The canonical terminal-output union an `after`/`case` sees is defined once in
[`expression-kernel.md`](expression-kernel.md): `Completed<O> | Failed<E> |
TimedOut | Cancelled`. Every effect, agent turn, and coerce terminal uses that
shape; domain-specific success/failure payloads are the `O`/`E` type parameters,
not new tags. Branch keywords: `succeeds` (Completed), `fails` (Failed), `times
out` (TimedOut), `cancelled` (Cancelled), and `completes` (binds the full union
for `case`). The terminal *status* value for timeout is `timed_out`; the source
branch keyword is `times out`.

## Exactly-Once External Effects

External effects (provider runs, file writes, messaging sends) have a real side
effect, so the admission of their terminal must survive a crash without either
losing or duplicating the side effect:

```text
1. worker durably records claim + run-started before invoking the source
2. worker invokes the source
3. worker appends the terminal admission under the effect idempotency key
```

If the worker crashes between 2 and 3, recovery finds a run-started with no
terminal. It must **not** blindly re-invoke the source:

```text
provider supports idempotent re-query (by run/thread/request id)
  -> re-query; admit the discovered terminal
provider has no idempotent re-query
  -> resolve to an explicit `uncertain` terminal (a Failed subkind) surfaced to
     the operator; do not silently re-execute the external side effect
```

A genuine retry (transient failure before the side effect) is safe because it
reuses the same idempotency key, so any duplicate terminal admission is absorbed
by the unique index.

Implemented: `recover_running_provider_runs` resolves a started-without-terminal
run with no recoverable evidence and no idempotent re-query through
`resolve_uncertain_provider_run` → the run records the distinct `uncertain`
status while the effect becomes `failed` (a Failed subkind), carrying the
`runtime.recovery_uncertain` diagnostic so an operator can tell it apart from an
ordinary provider failure. A raced real terminal is skipped, not re-executed.
This realizes the TLA+ `ResolveUncertainRun` action and is covered by kernel
tests.

## Periodic Reset Anchor

Any periodic reset (a `counter`'s `reset daily`, a clock recurrence window) is
nondeterministic without a declared anchor. Every periodic construct must declare
a timezone/anchor; if omitted, the checker defaults to UTC and emits a diagnostic
recommending an explicit anchor. The reset firing is recorded as a fact and
replay re-reads it; it is never recomputed from wall-clock time during replay.

## Formal Coverage

This contract is load-bearing runtime safety — the outbox / exactly-once / replay
territory that formal models exist to check — so it is modeled before
implementation, per the project's formal-models-first discipline (see
[`verification.md`](verification.md)). The split mirrors the tools' strengths:

TLA+/Apalache (`ControlPlaneLifecycle.tla`, extended; Apalache-checked) —
run-lifecycle exactly-once and recovery:

```text
exactly-once external effect (TerminaledRunStaysTerminal): a run that recorded a
  terminal -- including an `uncertain` recovery resolution -- never reverts to an
  executing status, so the external side effect is never silently re-executed
  (StartRun models the side effect; ResolveUncertainRun models recovery resolving
  a started-without-terminal run to a single `uncertain` terminal; a retry is a
  fresh run)
no run records more than one terminal (NoDuplicateTerminalRunEvents, existing)
recovery does not reorder the per-instance event log (existing)
no effect has more than one successful terminal completion (existing)
```

Maude (`admission.maude` + `tests/admission.maude`) — deterministic
rewrite/replay:

```text
the pure-rule fixpoint reaches a canonical fact set under the deterministic
  ordering, independent of interleaving
replay reproduces identical projections; a `re-invoke source` transition is
  forbidden during replay (record-once)
admission validation is a gate: an invalid candidate admits no fact
fact-batch admission is atomic; a partial batch is unreachable
the same admission identity key never produces two distinct facts
```

Where a property depends on real compiler/runtime output (for example the
per-program idempotency key derivation), prefer generated checks from emitted
artifacts over hand-written modules, and run them through
`scripts/check-formal-models.sh`. Stage R0 in
[`implementation-plan.md`](implementation-plan.md) sequences these models first,
then implements the runtime to conform.

## What This Replaces / Where It Is Referenced

The following specs reference this contract instead of restating admission rules:

- `event-ingress.md`, `std-time.md`, `scheduled-time.md` — signal/clock admission
  identity and dedup.
- `coerce.md`, `agent-harness.md` — model-backed effect replay and idempotency
  keys.
- `effects-and-capabilities.md`, `type-system.md` — runtime-boundary validation
  authority.
- `files.md` — typed fact-batch admission for `import`/`export`.
- `coordination.md` — counter reset anchor; lease admission.
- `execution-contract.md`, `control-plane.md`, `semantics.md` — idempotency key,
  exactly-once recovery, fixpoint determinism.
