# Coordination resources: lease, ledger, counter

Status: spec drafted 2026-06-10 from decided design
([`language-ergonomics-tracker.md`](language-ergonomics-tracker.md) C6).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**A closed family of shared coordination resources, generalizing coordination
the language already hardcodes.**

WhippleScript already has cross-instance mutable state: the work-queue's
`items.sqlite` is workspace-scoped, shared across every instance, and supports
an atomic claim. It is the existence proof and the architectural template. And
the language already special-cases coordination for specific resources:

- agent `capacity N` is a counting semaphore over turns,
- queue `claim` is a lease over a work item,
- a fact is a ledger entry that happens to be instance-local.

`lease`, `ledger`, and `counter` generalize those into a **closed family** of
shared resources alongside `queue`. Closed, not an open "shared cell with
arbitrary operations" — you pick a resource kind from a fixed menu, each with a
tiny verb vocabulary. That closure is what keeps the language from sliding into
a transactional database.

The coordination model is a **durable tuple-space, not message passing**
(see [Messaging](#messaging-a-durable-tuple-space)). Instances never address
each other; they read and write shared durable state, decoupled in identity and
time. This is what preserves inspectability, replayability, and decoupling.

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
reset-scheduled).

## Surface

Each resource is declared like a queue (workspace-scoped builtin), mutated by
atomic branchable effects, and projected to facts. Operation outcomes **are sum
types** ([`sum-types.md`](sum-types.md)) and the compiler enforces exhaustive
handling.

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
  cap 1000             # units (tokens/cents)
  reset daily          # lazy: applied at the next consume (below)
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

All operations carry instance/run provenance (the work-queue mechanism), so
`whip leases` / `whip ledger` / `whip counters` can answer "*who* holds the
prod slot / spent the budget / wrote this entry" — coordination state is fully
attributable and inspectable.

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

## Messaging: a durable tuple-space

WhippleScript coordinates through shared state, not actor-style channels. Every
messaging pattern is expressible as a coordination resource, and authors should
reach for these rather than build channels:

- **Broadcast / pub-sub** -> `ledger` (append; subscribers project).
- **Work distribution / competing consumers** -> `queue` (claim).
- **Spawn / delegate** -> `invoke` (parent -> child).
- **Point-to-point mailbox** -> a `ledger` partitioned by recipient identity:
  "instance B's mailbox" is the ledger partition keyed by B, which B projects
  with `when mailbox has entry for self.id as m`. The pattern requires an
  instance to name itself, which exists nowhere else in the language, so it is
  pinned here: **`self.id`** is the builtin instance id, recorded at instance
  start — a recorded value like any fact field, so expressions over it stay
  pure and replay-safe. A durable, retained, inspectable mailbox with no new
  primitive.
- **Directed fire-and-forget** -> the in-workflow `notify` effect
  ([`event-ingress.md`](event-ingress.md)), which injects a typed durable event
  into a target instance — still "inject a durable event," not "open a
  channel," so no liveness coupling.

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
- `ledger` must declare `retain`; `counter` must declare `cap` and `reset`;
  `lease` must declare `ttl`.
- No coordination resource is readable in a guard — only its projected facts
  and operation outcomes.

## Dependencies

Generalizes the queue (precedent + template). Outcomes reuse sum-type
exhaustiveness (C1); counter reset reuses scheduled time (C4); the `notify`
effect and mailbox pattern compose with event ingress (C5); leases reuse the
worker-lease TTL/instance-terminal runtime; formal models reuse the TLA+/Maude
rig and trace-conformance refinement.

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
