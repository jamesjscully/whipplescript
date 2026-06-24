# `std.coord`: lease, ledger, counter

Status: spec revised 2026-06-14 from package design
([`0013-coordination-package.md`](decision-records/0013-coordination-package.md)).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**A closed standard package for shared coordination resources, generalizing
coordination the language already hardcodes without becoming arbitrary shared
mutable state.**

WhippleScript already has cross-instance coordination: a workspace-scoped,
shared atomic claim over work items. (The original `items.sqlite`-as-truth
formulation in [`work-queues.md`](work-queues.md) is superseded — see
[`std.tracker`](decision-records/0002-work-tracker-package.md) and the kernel
lease primitive — but the *claim* it proved remains the architectural template.)
The language already special-cases coordination for specific resources:

- agent `capacity N` is a counting semaphore over turns,
- queue `claim` is a lease over a work item,
- a fact is a ledger entry that happens to be instance-local.

`std.coord` owns `lease`, `ledger`, and `counter` as a **closed family** of
shared resources. Closed, not an open "shared cell with arbitrary operations" —
you pick a resource kind from a fixed menu, each with a tiny verb vocabulary.
That closure is what keeps the language from sliding into a transactional
database.

The package state model follows the rest of WhippleScript's durable design:

```text
commands request coordination changes
events record accepted coordination changes
projections answer coordination state
```

`std.coord` is standard and privileged. Its source forms can be package-owned,
but its checks are platform-enforced: release obligations, bounded waits,
multi-lease ordering, counter caps, and ledger retention cannot be validated by
a plain request/response provider call.

The coordination model is a **durable tuple-space, not communication-platform
messaging** (see [Coordination vs messaging](#coordination-vs-messaging)).
Instances that use `std.coord` read and write shared durable state, decoupled in
identity and time. Communication through Slack, email, desktop notifications,
or other external channels belongs to `std.messaging`.

## The three architectural principles

Every resource in the family obeys the discipline the queue already proved:

1. **Typed domains, keyed on entities you already model.** Resources are
   declared with a typed key; the key is the identity of a thing the workflow
   already has as a fact — you lease *this `Ticket`*, count *this `Customer`'s*
   budget, log *this `area`'s* decisions. No string namespaces. Kills the
   stringly-typed-collision class.
2. **Atomic-attempt-and-branch, never read-then-act.** There is no operation
   to read shared state into a guard. Every op is one atomic effect with a
   branchable outcome (`acquire -> held | contended`, `consume -> ok | over`).
   The check and the act are the same effect. Kills the TOCTOU race class.
3. **Held resources are bounded by holder-lifetime + TTL.** A held resource
   auto-releases on explicit release, on the holding instance reaching a
   terminal, or on a mandatory TTL backstop. Kills the held-forever leak class.

## The reversible / irreversible split

"Counting" hides two different things; conflating them is a bug factory:

- A **reversible** count (concurrency pool / semaphore): acquire a slot, must
  release, leak-prone. This **folds into `lease`** as an N-slot lease, which
  inherits the leak-safe TTL/instance-terminal release — and is exactly what
  agent `capacity` already is.
- An **irreversible** count (real spend): consume budget, it is gone, reset on
  a schedule. This is **`counter`** — decrement-only, so it cannot leak-hold.

So the family is three non-overlapping shapes: `lease` (reservable, leak-safe,
1-or-N slots), `ledger` (append-only, contention-free), `counter` (consumable,
reset-scheduled). A `lease` with `slots 1` is a mutex; a `lease` with `slots N`
is a semaphore. `mutex` and `semaphore` should not be separate resource kinds
unless the slot model proves insufficient.

## Surface

Each resource is declared like a queue (workspace-scoped builtin), mutated by
atomic branchable effects, and projected to facts. Operation outcomes **are sum
types** ([`sum-types.md`](sum-types.md)) and the compiler enforces exhaustive
handling.

The long-term package lowering should be a modeled `resource_effect` class, not
plain `capability_call`. Construct registration can describe the syntax, but
the platform catalog owns the lowering semantics and reserved bare verbs.

### lease

```whip
lease deploy_slot {
  key Environment      # typed key, not a string
  slots 1              # 1 = mutex; >1 = semaphore
  ttl 10m              # mandatory backstop
}

rule ship
  when ReadyToShip as r
=> {
  acquire deploy_slot for r.env as slot

  after slot held {
    exec deploy_env with r -> DeployResult as d
    after d succeeds { release slot }
    after d fails    { release slot  fail error { reason "deploy failed" } }
  }
  after slot contended { ... }   # someone else holds this env; skip/retry
}
```

`std.coord.lease` does **not** implement leasing itself. It is a *surface* over
the single kernel lease primitive (`acquire`/`renew`/`expire`/`recover`) that the
kernel owns; `queue.claim` ([`work-queues.md`](work-queues.md)) and `std.tracker`
claims are sibling surfaces over the same primitive, sharing its TTL, recovery,
and lease-event vocabulary. The package contributes the typed-key declaration,
the branchable outcomes, and the static safety checks below; the durable lease
lifecycle (hold, renewal, TTL expiry, instance-terminal release, crash recovery)
is kernel-owned. Lease holders and wait queues are ordinary recorded facts; only
the lifecycle is a kernel primitive.

`acquire` is **one atomic attempt**: it completes immediately as `held` or
`contended` — the check and the act are one effect (principle 2), and the
author decides what contention means (skip, downgrade, schedule a retry).

The **waiting form is explicit and bounded**:

```whip
acquire deploy_slot for r.env wait timeout 15m as slot

after slot held       { ... }
after slot times out  { ... }   # turn never came within the bound
```

`wait` enqueues the instance FIFO behind current holders; the effect stays
pending until granted. It has no `contended` outcome — a waiter is by
definition contended — and `timeout` is **mandatory** on `wait` (the same
mandatory-bound discipline as `ttl`/`retain`/`cap`), routing to the existing
`after ... times out` branch. The `BoundedWait` liveness property below is the
wait form's guarantee: FIFO granting plus holder TTLs means a waiter's turn
always comes; the timeout bounds how long the author is willing to let it.
The two forms exist because they answer different questions — `acquire` asks
"is the slot free right now?", `acquire wait` asks "give me my turn" — and
conflating them would make `contended` ambiguous between "failed" and "queued".

`acquire ... until ttl` is the explicit fire-and-forget form (no `release`
obligation; TTL is the sole release). A semaphore is `slots N`.

A fire-and-forget (`until ttl`) lease **does count as held** while its TTL is
unexpired: it occupies a slot, so it counts toward the `slots N` / at-most-one
budget the kernel enforces, and toward the at-most-one-held-lease static check
that breaks hold-and-wait. The only difference from a `release`-obligated lease
is the release trigger (TTL or instance-terminal, never an explicit `release`);
its occupancy of the slot is identical. So acquiring a second lease while holding
an `until ttl` lease still requires a declared `lease order`.

### ledger

```whip
ledger decisions {
  entry ArchitectureDecision   # typed record
  partition by area            # scoped projection
  retain 90d                   # mandatory bounded growth
}

rule record_decision
  when DecisionMade as d
=> { append ArchitectureDecision { area d.area  choice d.choice } to decisions }

rule with_context
  when decisions has entry for "api" as e   # per-entry projection, rhymes with `queue has ready item`
=> { ... }
```

Entries project as per-entry facts (consistent with queue items); aggregation
("recent N", "count for group") reuses the existing fact aggregation
(`count(...)`). Appends commute — there is no contention to resolve.

### counter

```whip
counter model_budget {
  key Customer
  cap 1000               # units (tokens/cents)
  reset daily            # lazy: applied at the next consume (below)
  timezone "UTC"         # required anchor for the reset period boundary
}

rule review
  when ReviewTask as t
=> {
  consume model_budget for t.customer amount t.estTokens as spend

  after spend ok   { coerce review(t.body) as r ... }
  after spend over { tell cheaper_agent "..." }   # downgrade, not crash
}
```

Reset is **lazy, at the consume boundary** — not scheduled. `consume` already
runs at the worker boundary inside one store transaction; that transaction
first compares the current period (derived from the worker's clock read, the
one place the clock is legal) against the period of the counter's last reset,
and zeroes the count if the period has rolled, then applies the consume — one
atomic step. This needs no scheduler, no daemon, and no recurrence machinery —
exactly the mechanisms scheduled time ([`scheduled-time.md`](scheduled-time.md))
declines. A counter nobody consumes is never reset, which is unobservable and
therefore free; the `CapInvariant` model treats reset as part of the consume
action, which makes it smaller, not larger.

A period boundary like `reset daily` is meaningless without an anchor, so
`counter` must declare a `timezone`/anchor — the periodic-reset-anchor rule of
[`admission-and-idempotency.md`](admission-and-idempotency.md#periodic-reset-anchor).
If omitted, the checker defaults to UTC and emits a diagnostic (one
`severity: warning` from the canonical `error | warning | info | hint` enum)
recommending an explicit anchor; without it the period boundary, and therefore
the reset firing, would be non-deterministic on replay. When a consume rolls the
period, the reset firing is recorded as a fact (`counter.period_reset`, below):
replay **re-reads** that recorded reset rather than recomputing the period from
wall-clock time, so the same consume sequence reproduces the same counts. The
anchored period boundary is the only clock-derived input, and it is recorded
once.

All operations carry instance/run provenance (the work-queue mechanism), so
`whip leases` / `whip ledger` / `whip counters` can answer "*who* holds the
prod slot / spent the budget / wrote this entry" — coordination state is fully
attributable and inspectable.

## Event And Projection Vocabulary

Accepted coordination changes are append-only events:

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

The provider answers coordination questions through projections:

```text
active lease holders by resource/key
lease wait queues by resource/key
ledger entries by partition
counter usage by key/period
coordination history/audit
```

No workflow mutates a projection directly. Projections are rebuildable from the
accepted coordination event stream plus runtime clock boundaries recorded by
the provider.

## Safety model

The bug classes are made unrepresentable by static restrictions that stay
**local and predictable** — no environment-dependent compilation needed. Each
maps onto a Coffman deadlock condition or a resource-discipline check.

1. **At-most-one-held-lease per progression (hard default) -> breaks
   hold-and-wait.** You cannot deadlock on locks if you never hold one while
   waiting for another. Trivially, locally checkable; covers the overwhelming
   majority of real use.
2. **Multi-lease requires an explicit declared lease order -> breaks
   circular-wait.** A workspace-level partial order (`lease order: repo_region
   < deploy`); each acquire path is checked locally against it (the proven
   resource-hierarchy technique). The order is the one global object —
   declared once, explicitly — so a file always compiles the same way against
   a given order; a violator fails *its own* check with a precise message. No
   spooky cross-file breakage.
3. **Linear must-release -> eliminates forgotten-release blocking.** An
   acquired lease must be `release`d on every terminal path of its acquiring
   flow, or explicitly marked `until ttl`. Forgetting it is a compile error.
   TTL becomes the *crash* net, not the *forgot-to-write* net.
4. **Exhaustive outcome handling -> eliminates silent-assume-success.**
   Coordination outcomes are sum types (`Held(slot) | Contended`, `Ok | Over`);
   the compiler enforces every arm is handled, exactly like exhaustive `case`
   ([`sum-types.md`](sum-types.md)). You cannot proceed as if you won a lease
   you may have lost, or ignore over-budget.

Together: races (no read-then-act op exists), leaks (linearity + TTL),
deadlock (Coffman-broken statically), and silent failure (exhaustive outcomes)
are all unrepresentable. What remains is liveness, handled below.

## Formal modeling

Tractable because all concurrency funnels through a handful of atomic store
operations — the verifiable surface is four small protocols, not the language.
One TLA+ spec per resource, slotting into the existing TLA+/Maude rig
(`models/tla`, `models/maude`):

- **lease (N-slot semaphore):** `MutualExclusion` (<= N holders per key, ever);
  `NoDeadlock` (under the ordered-acquire discipline, no cyclic-wait state is
  reachable); `BoundedWait` (under FIFO + TTL, every waiter eventually acquires
  — liveness under weak fairness).
- **counter (consumable):** `CapInvariant` (consumed <= cap between resets);
  `NoLostConsume` (concurrent consumes serialize, sum is exact); reset
  monotonicity.
- **ledger (append-log):** `AppendLinearizable` (total order per partition);
  `NoLostEntry`; `PartitionIsolation` (a projection for partition P observes
  exactly P).

The Rust store's transactional ops are checked as a **refinement** of the TLA+
atomic actions via the same trace-conformance mechanism `trace --check` uses.

## Runtime bounds (the undecidable liveness)

Starvation/fairness cannot be decided statically under open contention, so it
is a store guarantee, not a static one: **FIFO-fair granting + mandatory TTL**
gives bounded waiting (the `BoundedWait` property the model verifies), and
instance-terminal release gives crash safety. `BoundedWait` is a property of
the `acquire ... wait` form's queue; the plain `acquire` never waits, so its
liveness is trivial (it always completes immediately, one way or the other).

## Coordination vs messaging

`std.coord` coordinates through shared state, not actor-style channels. These
patterns remain coordination patterns:

- **Broadcast / pub-sub** -> `ledger` (append; subscribers project).
- **Work distribution / competing consumers** -> `queue` (claim).
- **Spawn / delegate** -> `invoke` (parent -> child).
- **Internal per-recipient partitions** -> a `ledger` partitioned by an explicit
  recipient key carried in workflow data. This is useful for durable tuple-space
  patterns such as fan-out/fan-in barriers, but it is not a communication
  channel and should not be described as a mailbox in source-facing docs. If the
  language later needs a builtin current-instance identity value, that belongs in
  a core instance-identity design, not as an incidental `std.coord` feature.
- **Directed typed signal injection** -> the in-workflow signal injection effect
  ([`event-ingress.md`](event-ingress.md)), which injects a typed durable
  signal into a target instance.

`std.messaging` is separate. It is for talking through communication platforms:
local mailboxes, Slack, email, GitHub comments, desktop notifications, stdio,
or similar channels. Messaging may use `std.ingress` for inbound observations,
but it does not replace coordination resources and it does not produce arbitrary
typed domain facts without an explicit signal or schema-coercion boundary.

Synchronous peer request/reply is deliberately awkward: if you need a
synchronous typed answer, you want an effect (`coerce`/`call`/`exec`), not a
round-trip to a peer durable workflow. The parent->children scatter-gather is
the `invoke` + ledger-barrier pattern.

## Static checks

- A resource is declared with a typed key; an undeclared key type or a string
  key is a check error.
- Lease: at most one lease held per progression unless a `lease order` covers
  every multi-acquire path; an acquire outside the declared order is a check
  error. An acquired lease unreleased on a terminal path (and not `until ttl`)
  is a check error.
- All coordination outcomes (`held`/`contended`, `ok`/`over`) must be
  exhaustively handled. For the `wait` form the outcome pair is
  `held`/`times out`; `contended` on a `wait` acquire is a check error.
- `acquire ... wait` without a `timeout` is a check error (every wait is
  author-bounded).
- `ledger` must declare `retain`; `counter` must declare `cap` and `reset`, and
  should declare a `timezone`/anchor for the reset period (default UTC + a
  `severity: warning` diagnostic if omitted, per
  [`admission-and-idempotency.md`](admission-and-idempotency.md#periodic-reset-anchor));
  `lease` must declare `ttl`.
- No coordination resource is readable in a guard — only its projected facts
  and operation outcomes.

## Non-Goals

- No arbitrary shared cells.
- No general SQL/document database surface.
- No unbounded waits.
- No read-then-act guard over coordination state.
- No separate `mutex` or `semaphore` resource until `lease slots` fails.
- No communication-platform channel semantics; those belong to `std.messaging`.

## Dependencies

Generalizes the queue (precedent + template). Outcomes reuse sum-type
exhaustiveness (C1); counter reset reuses core time plus `std.time` and the
periodic-reset anchor of
[`admission-and-idempotency.md`](admission-and-idempotency.md); typed signal
injection and ledger partition/barrier patterns compose with ingress;
`std.coord.lease` is a surface over the **single kernel lease primitive**
(`acquire`/`renew`/`expire`/`recover`) — the same primitive
[`work-queues.md`](work-queues.md)'s `queue.claim` and `std.tracker` claims use,
not a second lease implementation; formal models reuse the TLA+/Maude rig and
trace-conformance refinement. Coordination outcomes are recorded facts; the
lease lifecycle is the kernel primitive (see
[`admission-and-idempotency.md`](admission-and-idempotency.md)).

## Modeling notes

- Safety: mutual exclusion (lease), cap-never-exceeded (counter), and
  append-integrity (ledger) hold under arbitrary interleavings (TLA+ invariants
  model-checked).
- No deadlock: under the one-lease default and the declared-order escape, no
  cyclic-wait state is reachable (model the acquire discipline; check
  unreachability).
- Bounded wait: under FIFO + TTL, every waiter eventually acquires (liveness
  under weak fairness).
- Replay: a coordination outcome (held/contended, ok/over, appended) is
  recorded in the instance log; replay re-reads the recorded outcome, never
  re-runs the shared mutation — runtime non-determinism, replay determinism,
  the same contract as effects and the queue.
