# `std.time`: clock sources for recurring signals

Status: spec drafted 2026-06-14 from package design discussion
([`0007-core-standard-libraries-and-providers.md`](decision-records/0007-core-standard-libraries-and-providers.md)).
Stage: spec -> modeling -> implementation + testing -> review.

> **Reserved-class prerequisites:** `clock_source` (in the `source_declaration`
> construct family) — **LANDED (Stage P1b)**: the `source clock as <name> { … }`
> parser, lowering to `clock_source`/`clock_source_template`, static checks, and
> formal models (`clock-source.maude` + `ClockSourceLifecycle.tla`) are in place.
> Remaining work is runtime recurrence (DST/calendar evaluation + durable
> interval-anchor), tracked under the runtime stages, not Cycle 2.

## Framing

**Time is an external observation source.**

`std.time` should not be a mini workflow engine and should not directly run
rules. It contributes the `clock` provider to the shared source construct:

```text
clock observation -> validated signal fact -> rules
```

The construct behind `source clock as <name>` is `clock_source`, a net-new
member of the `source_declaration` construct family (sibling to
[`signal_source`](event-ingress.md)). `clock_source` is the name the
construct-graph and lowering reports use. It coexists with core
[`timer`/`timer.wait`](time.md): `clock_source` owns recurrence and emits typed
signals through admission, while `timer.wait` remains the one-shot rule-body
effect with its own `schedule`/`timer.fired` vocabulary; neither replaces the
other.

This keeps recurrence expressive without weakening the core time rule:

```text
The current clock is read only at worker/provider boundaries, never in guards.
```

Core time owns one-shot mechanics. `std.time` owns recurrence policy.

## Source Surface

The author declares a signal and a clock source that emits it:

```whip
use std.time

signal triage.tick {
  scheduled_at time
  observed_at time
  occurrence_id string
  missed_count int
}

source clock as daily_triage {
  every weekday at 09:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
    observed_at tick.observed_at
    occurrence_id tick.occurrence_id
    missed_count tick.missed_count
  }
}

rule file_daily_triage
  when triage.tick as tick
=> {
  create issue in backlog {
    title "Daily triage"
  } as created
}
```

The explicit `observe as tick` binding avoids magic. The `clock` provider owns
the observation schema; the workflow explicitly maps that observation into the
declared signal schema.

## Clock Observation Schema

The initial `clock` observation should expose:

```text
scheduled_at time      intended occurrence time
observed_at time       worker/provider observation time
occurrence_id string   stable occurrence id for dedupe
missed_count int       number of missed occurrences represented here
schedule_name string   source name
```

`observed_at` is a recorded value. Rules may compare it like any other `time`
fact field, but they still cannot read the current clock.

The `occurrence_id` is **not** invented by the provider. It is the clock-occurrence
admission key defined in
[`admission-and-idempotency.md`](admission-and-idempotency.md):
`H(source_id, scheduled_occurrence_instant)`. The kernel computes it at admission
and the store's unique index on `(instance, fact_identity_key)` enforces that the
same occurrence cannot append a second signal fact. `std.time` does not implement
its own dedup; it provides `scheduled_at` and the source identity, and the kernel
derives the key.

## Recurrence Forms

Initial recurrence should stay small:

```text
at <time>                         one scheduled occurrence
every <duration>                  interval occurrences
every <calendar-pattern> at <time-of-day>  calendar occurrences
```

Calendar pattern syntax should be conservative at first:

```text
every day at 09:00
every weekday at 09:00
every monday at 09:00
```

Cron-compatible syntax can be added later if needed, but the first authoring
surface should be readable without memorizing cron fields.

## Timezone

Calendar schedules must declare a timezone:

```whip
timezone "America/New_York"
```

Interval schedules may omit timezone because they advance by duration from the
recorded start/last occurrence. Calendar schedules carry a periodic anchor in the
sense of [`admission-and-idempotency.md`](admission-and-idempotency.md#periodic-reset-anchor):
a calendar schedule without a timezone defaults to UTC and emits a diagnostic
recommending an explicit anchor, unless the provider config supplies an explicit
default that the report records. The chosen anchor is recorded so replay computes
the same scheduled instants.

## Missed Occurrence Policy

A source may be down or a worker may not run at the intended time. Missed policy
must be explicit for recurring schedules:

```text
missed skip
missed coalesce
missed catch_up limit N
```

Semantics:

```text
skip             emit only the next observed occurrence; record missed diagnostics
coalesce         emit one occurrence representing all missed ticks
catch_up limit N emit up to N missed occurrences in order
```

No silent default. If a recurring source omits `missed`, checking should produce
a diagnostic requiring the author to choose.

**Coalesced occurrence identity.** A coalesced tick still has exactly one
`scheduled_at`, and therefore exactly one occurrence_id, so it interacts cleanly
with the occurrence-id idempotency above. A coalesced tick records the
`scheduled_at` of the **latest (most recent) missed occurrence** it represents;
`missed_count` carries how many earlier occurrences were folded in, and
`observed_at` is the actual observation time. Using the latest scheduled instant
means each scheduled instant maps to at most one occurrence_id across catch-up
modes: a later switch from `coalesce` to `catch_up` would re-admit the earlier,
not-yet-recorded instants under their own keys without colliding with the
coalesced tick, and replaying the coalesced tick re-reads the same recorded fact
rather than recomputing the fold. (`catch_up` and `skip` each record one fact per
emitted occurrence under that occurrence's own `scheduled_at`.)

Realized (2026-06-18): the **interval** form (`every <duration>`) fires at runtime
via `resolve_due_clock_sources` on each worker pass — it enumerates due occurrences
from the cursor (last admitted occurrence, else instance start) to the
worker-boundary clock, applies the missed policy, and admits each as a durable
signal fact keyed by `occurrence_id = H(source, scheduled_instant)`. The cursor is
read from the append-only event log so a consumed occurrence cannot regress it.
Interval time math uses SQLite `strftime` (no date/timezone dependency); the
`at <time>` and calendar (`every <calendar> at <time>`) forms — which need timezone
resolution — lower and are modeled but do not fire at runtime yet.

## Providers

Initial provider scope:

```text
std.time.clock   local/store-backed clock source provider
```

The local provider fires on worker/scheduler passes. It does not require a
hidden daemon; operators may run a scheduler loop, a hosted worker, systemd,
cron, or another process that periodically invokes WhippleScript. The durable
source state records what was observed and emitted.

External schedulers that already exist can also call `whip signal` through
`std.ingress.cli`; they do not need to become schedule providers.

## Core/Package Boundary

Core owns:

```text
time scalar
duration literals
time comparisons over recorded values
time +/- duration over recorded values
timeout <duration> on effects
timer <duration>
timer until <time>
cancel <effect>
no current clock in guards
```

`std.time` owns:

```text
clock source provider
recurrence policy
timezone policy
missed occurrence policy
schedule/source status
clock observation evidence
```

## Static Checks

- `source clock as <name>` (the `clock_source` construct) requires `std.time`.
- `observe as <binding>` binds the `ClockObservation` schema.
- `emit <signal> { ... }` must materialize the declared signal payload type.
- A recurring schedule must declare `missed`.
- A calendar schedule should declare `timezone`; if omitted (and no explicit
  provider default is recorded), the checker defaults to UTC and emits a
  diagnostic recommending an explicit anchor (the periodic-anchor rule of
  [`admission-and-idempotency.md`](admission-and-idempotency.md)).
- A clock source cannot fire a rule directly.
- A clock source cannot emit undeclared signals.
- `now` or any current-clock expression remains forbidden in guards and rule
  expressions.

## Non-Goals

- No direct rule firing.
- No hidden daemon requirement.
- No current clock in guards.
- No cron-provider zoo in the first pass.
- No business-calendar integrations in the first pass.
- No one-shot timer replacement; core `timer` remains the one-shot mechanism.

## Modeling Notes

- **Replay safety:** replay reads recorded signal facts, not the clock.
- **No direct fire:** clock sources emit signal facts only through admission.
- **Occurrence idempotency:** the `occurrence_id` is the kernel-derived
  clock-occurrence key `H(source_id, scheduled_occurrence_instant)`
  ([`admission-and-idempotency.md`](admission-and-idempotency.md)); the store's
  unique index on `(instance, fact_identity_key)` is what prevents a duplicate
  signal fact for the same occurrence. `std.time` asserts no dedup of its own.
- **Missed policy explicitness:** every recurring source has a declared missed
  policy, so catch-up/skip behavior is inspectable.
