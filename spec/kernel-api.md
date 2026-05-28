# Kernel API

Status: draft

The kernel is the small deterministic core that advances one workflow instance.
It owns the semantics of events, facts, rule commits, effect graphs,
dependencies, and effect lifecycle transitions.

The control plane owns process lifecycle, CLI, recovery loops, workers,
registries, and status surfaces. It calls the kernel; it should not reimplement
kernel semantics.

## Kernel State

Per instance:

```text
R = (L, F, Q, D, C)
```

Where:

- `L` is the append-only event log.
- `F` is the current typed fact projection.
- `Q` is the durable effect outbox.
- `D` is the durable effect-dependency relation.
- `C` is control metadata.

## Operations

Initial kernel API:

```text
append_event(instance, event) -> event_id
derive_projection(instance) -> fact_delta
evaluate_rules(instance, trigger) -> candidate_rule_steps
commit_rule_step(instance, rule_step) -> commit_id
enqueue_effect_graph(instance, graph) -> effect_ids
mark_effect_blocked(instance, effect, reason) -> event_id
claim_effect(instance, effect, worker) -> lease_id
start_run(instance, effect, lease) -> run_id
complete_run(instance, run, output) -> event_id
fail_run(instance, run, error) -> event_id
expire_lease(instance, lease) -> event_id
cancel_effect(instance, effect, reason) -> event_id
```

Names are illustrative; the implementation may expose fewer public functions.
The semantic boundary is what matters.

## Transaction Boundaries

Atomic rule commit:

```text
validate matched facts and guards
consume/update facts
record facts
append derived events, if any
enqueue effect graph nodes
persist dependency edges
record evidence/diagnostics
advance rule cursor
```

Atomic effect claim:

```text
verify effect is claimable
verify dependency predicates are satisfied
verify policy/capability/profile binding
create/renew lease
transition effect to claimed
record evidence
```

Atomic run completion:

```text
verify run belongs to claimed/running effect
validate provider output against effect contract
write artifacts/evidence
transition effect to terminal status
append terminal event
derive standard completion facts
```

Provider execution is never inside a rule commit.

## Kernel Invariants

The kernel enforces:

- per-instance event sequence is append-only and gap-free
- rule commits are serialized per instance
- no effect runs before its dependency predicates are satisfied
- source order never creates effect ordering
- every effect has a stable idempotency key
- every run references exactly one effect
- every terminal run appends exactly one terminal event
- provider output is validated before completion facts are visible
- plugins cannot mutate facts or events outside kernel operations

## Control Plane Boundary

The control plane may:

- compile and deploy programs
- start/pause/resume/cancel instances
- schedule workers
- recover leases
- call providers
- render status
- manage registries

The control plane may not:

- insert facts without a kernel-mediated event or rule commit
- mark effects completed without validation
- claim blocked effects
- infer prompt semantics as workflow facts
- let plugins bypass rule commits or effect events
