# 0013: Coordination Package

Status: proposed

## Decision

`std.coord` owns generic shared coordination resources. It is a standard,
privileged package rather than irreducible core syntax, but its invariants are
platform-enforced because ordinary request/response effects cannot prove the
required lifetime and atomicity properties.

The package state model matches the rest of WhippleScript's durable design:

```text
commands request coordination changes
events record accepted coordination changes
projections answer coordination state
```

`std.coord` is not an arbitrary mutable store. It exposes a closed family of
resource kinds with small verb sets:

```text
lease
ledger
counter
```

Mutexes and semaphores should not be separate first-class resources unless the
lease model proves insufficient. A lease with `slots 1` is a mutex; a lease with
`slots N` is a semaphore.

## Boundary

```text
std.tracker
  issue records, issue-domain claims, ready-work projections

std.coord
  generic lease/ledger/counter surfaces, coordination projections

core runtime
  the single kernel lease primitive (acquire/renew/expire/recover),
  terminal status, replay, cancellation, recovery
```

There is **one lease engine**: the kernel owns the lease primitive
(`acquire`/`renew`/`expire`/`recover`). `std.coord`'s `lease`, the work
queue/tracker `claim`, and provider-run/effect leases are all **surfaces** over
that one primitive, sharing its TTL, recovery, and lease-event vocabulary;
packages do **not** implement leases. Coordination resource state (lease holders,
wait queues, ledger entries, counter usage) is ordinary durable facts; only the
lease *lifecycle* is a kernel primitive (see
[`admission-and-idempotency.md`](../admission-and-idempotency.md)). Tracker claims
are issue-domain handles over the same primitive. Importing `std.tracker` should
not grant arbitrary coordination resources, and importing `std.coord` should not
grant issue workflow semantics.

## Resource Kinds

### `lease`

A reversible reservation over a typed key:

```whip
lease deploy_slot {
  key Environment
  slots 1
  ttl 10m
}
```

`acquire` is an atomic branchable effect:

```whip
acquire deploy_slot for r.env as slot

after slot held { ... release slot }
after slot contended { ... }
```

Waiting is explicit and bounded:

```whip
acquire deploy_slot for r.env wait timeout 15m as slot

after slot held { ... }
after slot times out { ... }
```

(The branch keyword is `times out`; the terminal *status* value is `timed_out` —
the canonical terminal union in
[`expression-kernel.md`](../expression-kernel.md).)

There is no unbounded wait and no read-then-act guard. The operation itself is
the atomic boundary.

### `ledger`

An append-only coordination log with bounded retention:

```whip
ledger decisions {
  entry ArchitectureDecision
  partition by area
  retain 90d
}
```

Ledger appends commute. Reads happen through projections, not mutable cells.

### `counter`

An irreversible bounded consumption resource:

```whip
counter model_budget {
  key Customer
  cap 1000
  reset daily
  timezone "UTC"   # required anchor for the reset period boundary
}
```

`consume` atomically applies any lazy period reset and either records the
consumption or returns an over-limit outcome. A periodic `reset` requires a
declared `timezone`/anchor (default UTC + a diagnostic if omitted), and the reset
firing is recorded as a fact (`counter.period_reset`) that replay re-reads — the
periodic-reset-anchor rule of
[`admission-and-idempotency.md`](../admission-and-idempotency.md).

## Event Vocabulary

The package should model accepted coordination changes as append-only events.
Initial vocabulary:

```text
lease.declared
lease.acquire_requested
lease.acquired
lease.contended
lease.wait_enqueued
lease.wait_timed_out
lease.renewed
lease.released
lease.expired

ledger.declared
ledger.entry_appended
ledger.entry_pruned

counter.declared
counter.consumed
counter.over_limit
counter.period_reset
```

Projections answer:

```text
active lease holders by resource/key
lease wait queues by resource/key
ledger entries by partition
counter usage by key/period
coord history/audit
```

## Static Guarantees

`std.coord` requires platform-enforced checks:

```text
leases require TTL
held leases must be released on every terminal path unless declared until ttl
  (an until-ttl lease still counts as held: it occupies a slot and counts toward
   the at-most-one-held-lease / hold-and-wait checks)
waiting must be bounded
multi-lease workflows require a declared lease order
coordination outcomes are exhaustive sum types
counter consume cannot exceed cap inside one period
counter reset requires a declared timezone/anchor (default UTC + diagnostic)
ledger growth must be bounded by retention policy
coordination resources are not readable in guards except through projections
```

Diagnostics use the canonical single severity enum `error | warning | info |
hint`; the missing-reset-anchor case emits a `warning`.

These checks make races, forgotten releases, silent over-budget behavior, and
unbounded shared-state growth unrepresentable in accepted programs.

## Construct And Lowering Needs

The syntax can be package-owned, but the lowering cannot be a plain generic
`capability_call`. The checker/runtime need to understand held-resource
lifetime, release obligations, ordering, bounded waits, counter caps, and
ledger retention.

`std.coord` therefore needs a modeled `resource_effect`-style lowering class
with static evidence in the construct graph and lowering report. Package
manifests can declare the construct instances, but the platform catalog must own
the reserved bare verbs and the lowering class semantics.

## Non-Goals

- No arbitrary shared cells.
- No general SQL/document database surface.
- No unbounded waits.
- No read-then-act guard over coordination state.
- No separate `mutex` or `semaphore` resource until `lease slots` fails.
- No peer-to-peer message channel semantics; durable messaging patterns should
  be expressed through ledgers or a separate messaging package if needed.

