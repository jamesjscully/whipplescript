# Scheduled time: absolute deadlines and the `time` type

Status: spec drafted 2026-06-10 from decided design
([`language-ergonomics-tracker.md`](language-ergonomics-tracker.md) C4).
Stage: spec -> modeling -> implementation + testing -> review.

Builds on the relative-time model ([`time.md`](time.md), A2): `timeout`
clauses, the relative `timer <duration>` effect, and the worker-pass time
resolution with no daemon.

## Framing

**The clock is read only at the worker boundary; `now` never exists in a
guard.**

This is the load-bearing rule. Wall-clock time is the canonical impurity:
`now()` in a guard would break "same facts -> same commit" and kill replay.
The existing time model already respects this — a relative deadline is
`creation_time + duration`, both recorded as log events, so firing is
replayable. Absolute scheduling fits *only* if it lives in the same place:
the worker's time pass compares the system clock to a target and records a
firing fact; rules react to the fact, never to the clock.

This adds scheduling (`timer until <datetime>`) without adding impurity to the
deterministic core.

## Surface

### The `time` scalar type

A new scalar `time` (ISO 8601 instants), usable as a schema field type and as
a literal. Literals are written as quoted strings in `time`-typed positions,
so the lexer needs no datetime tokens:

```whip
class Ticket {
  id string
  dueAt time
}

table tickets as Ticket [
  { id "T-1" dueAt "2026-06-15T09:00:00Z" }
]
```

A `time` value is an ordinary recorded value. It **is comparable in guards** —
`where a.dueAt < b.deadline` is pure, because both operands are recorded
facts. What is forbidden is reading the *current* time in a guard: there is no
`now`. The only path by which "now" influences the workflow is a worker firing
a timer.

### Absolute timer

Generalizes the relative `timer <duration> as x`. The operand is a `time`
literal or a `time`-typed path:

```whip
timer until ticket.dueAt as deadline
timer until "2026-06-15T09:00:00Z" as morning

after deadline succeeds {
  ...   // fired when the system clock reached the target; recorded as a fact
}
```

`timer until <time-expr>` and `timer <duration>` are distinguished by the
`until` keyword. Both lower to the same `timer.wait` effect; only the deadline
basis differs (absolute target vs. `created + duration`).

## Mechanics

- The worker's existing time pass (`due_time_effects`) already compares
  elapsed-vs-deadline. An absolute timer carries `target_time` instead of
  `created + duration`; the pass fires it when `system_clock >= target_time`.
- Firing records a `timer.fired` fact (same as relative timers); the rule
  reacts to the fact. Replay re-reads the recorded firing, not the clock.
- No daemon: an absolute timer fires on the **next worker pass at or after**
  the target instant. Precision is bounded by worker cadence, which the
  operator drives — the same property the relative timer already has.
- `time` values serialize as ISO 8601 strings in fact JSON; comparison is on
  the instant, timezone-normalized.

## Out of scope (v1)

- **Recurrence / cron.** Exact "every day at 09:00:00" requires either the
  daemon the design rejects or accepting approximate "at-or-after T, next
  worker pass" semantics. v1 ships one-shot `until` only; recurring needs are
  served by an external scheduler that pokes the workflow via
  [`whip notify`](event-ingress.md) — which keeps the recurrence policy and
  its clock outside the deterministic core. A recurring `timer` may be
  revisited later, explicitly as approximate.
- **Absolute `timeout` on effects.** `timeout` stays relative-only;
  deadlines-on-effects are naturally relative to effect creation. Absolute
  scheduling is `timer until`'s job.
- **Time arithmetic (specced fast-follow).** The first ergonomic gap after
  one-shot `until` is the deadline-*warning* pattern: "escalate one hour
  before `dueAt`" needs `timer until ticket.dueAt - 1h`. Time ± duration over
  recorded values is pure and replay-safe — no clock read; both operands are
  facts — so it threatens nothing this spec defends. v1 ships without it (the
  workaround is precomputing the warning instant upstream); the surface is
  pinned here: `<time-expr> ± <duration>` is itself a `time` expression, legal
  anywhere a `time` path is.

## Static checks

- `now` (or any current-clock reference) in a guard or any rule expression is
  a check error — the determinism boundary.
- A `time` literal must be a valid ISO 8601 instant; an invalid string in a
  `time` position is a check error.
- `timer until <expr>`: `<expr>` must be `time`-typed (literal or path);
  a non-`time` operand is a check error.
- `time` comparisons (`<`, `<=`, `>`, `>=`, `==`) are typed against `time`
  operands only.

## Dependencies

Extends the A2 time pass ([`time.md`](time.md)) — the same `timer.wait`
effect, store columns, and worker resolution, with an absolute deadline basis.
Requires the `time` scalar in the type system (the kernel already carries an
`ExprType::Time`). Pairs with [`event-ingress.md`](event-ingress.md) for
externally-driven recurrence.

## Modeling notes

- Boundary discipline: no expression reads the current clock; the only
  clock read is the worker firing a timer (property: rule evaluation is a
  pure function of facts, unchanged by wall time).
- Firing exactly-once: an absolute timer fires once, on the first worker pass
  at or after the target; re-running the pass does not re-fire (idempotent
  terminal, as for relative timers).
- Replay: a recorded `timer.fired` replays identically regardless of when
  replay runs (golden test: replay at a later wall-clock time yields the same
  trace).
