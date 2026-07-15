# `std.time`: clock sources for recurring signals

Status: spec drafted 2026-06-14 from package design discussion
([`0007-core-standard-libraries-and-providers.md`](decision-records/0007-core-standard-libraries-and-providers.md));
package design updated 2026-07-04 for the std-package campaign under
[`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md).
Stage: spec -> modeling -> implementation + testing -> review.

> **Reserved-class prerequisites:** `clock_source` (in the `source_declaration`
> construct family) — **LANDED (Stage P1b)**: the `source clock as <name> { … }`
> parser, lowering to `clock_source`/`clock_source_template`, static checks, and
> formal models (`clock-source.maude` + `ClockSourceLifecycle.tla`) are in place.
> Runtime recurrence is ALSO landed — all three recurrence forms fire at worker
> boundaries, tz-aware and DST-correct (see "Realized Runtime Surface"; the
> previous revision of this note understated this). Remaining work is package
> identity (embedded manifest + import advisory per the ecosystem shape's "M5"
> decision) plus the small fidelity slices below.

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

## Package Checklist (design-tracker fields)

- **Core functionality:** recurrence over the shared source construct — the
  `clock` provider, missed-occurrence policy, timezone anchoring, occurrence
  identity, clock-observation evidence.
- **Why it belongs:** recurrence is policy, not kernel invariant. Per the
  ecosystem shape's "Package feature vs core lifecycle invariant" split, core
  keeps one-shot timers/timeouts and no-clock-in-guards; std.time owns the
  recurrence vocabulary, its provider, and its check demands.
- **NOT in the package:** one-shot `timer`/`timer until`/`timeout`/`cancel`
  (core; [`time.md`](time.md), [`scheduled-time.md`](scheduled-time.md)); the
  no-current-clock-in-guards rule (core invariant); external event admission
  and non-clock source providers (std.ingress); DO alarms (DO tracker, per
  "M7"); cron grammar and business calendars (deferred).
- **Target feature set:** the shipped surface below + package identity (slice
  T1) + observation fidelity (T2) + pinned `at`-form semantics (T3).
- **Dependencies:** core `source_declaration` family, signal admission plane,
  and worker pass; std.ingress for the shared source family and signal
  declarations (see "Ingress Boundary"); no other std package.
- **Provider expectations:** exactly one provider, `std.time.clock`, running
  inside the worker pass (see "Providers" for why the M2 seams do not apply).
- **Open naming-boundary questions:** (a) whether family-level source checks
  are documented in [`event-ingress.md`](event-ingress.md) or the std.ingress
  package spec — the std.ingress design owns that resolution; (b) whether the
  `clock` provider row lives in std.time's manifest (chosen here) or a shared
  family manifest. Settled, not open: `timer`/`timer.wait` never migrate into
  std.time (core per "E7").
- **Verdict:** KEEP, separate from std.ingress (per "E6"). The runtime already
  shipped; v1 is a thin identity-and-fidelity tail.

## Surface

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

**Effect operations: NONE.** Clock occurrences are admitted directly as
durable signal facts at the worker boundary; they are never dispatched as
claimable effects (cli/main.rs:20823-20913). Exact effect-kind strings owned
by this package: none. Capability ids: none — the ecosystem shape's "M3"
id==kind rule is vacuously satisfied, and the manifest declares no
capabilities. Adjacent kinds NOT owned here: `timer.wait` (core one-shot,
[`time.md`](time.md)) and `event.notify` → `signal.emit` (std.ingress, rename
slice A of "M4").

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

Reality note (2026-07-04): the shipped payload builder provides
`scheduled_at`/`observed_at`/`occurrence_id`/`missed_count` only —
`schedule_name` is declared here but not yet emitted
(`clock_emit_payload`, cli/main.rs:20877-20884). Slice T2 closes that gap.

The `occurrence_id` is **not** invented by the provider. It is the clock-occurrence
admission key defined in
[`admission-and-idempotency.md`](admission-and-idempotency.md):
`H(source_id, scheduled_occurrence_instant)`. The kernel computes it at
admission and the store's unique at-most-once index — shipped as
`events(instance_id, idempotency_key)` (store migration 0001:75-76;
`fact_identity_key` is this spec family's older name for the same column, see
"Spec Amendments") — enforces that the same occurrence cannot append a second
signal fact. `std.time` does not implement its own dedup; it provides
`scheduled_at` and the source identity, and the kernel derives the key.

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

`at <hh:mm>` semantics (pinned 2026-07-04, matching shipped behavior): the
first upcoming occurrence of that time-of-day fires exactly once, then the
source stays quiet (cli/main.rs:20853-20871). Slice T3 adds the explicit
fire-once regression test.

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

## Realized Runtime Surface

Realized (2026-06-18; staleness corrected 2026-07-04): **all three recurrence
forms fire at runtime** — `every <duration>` (interval math via SQLite
`strftime`), `every <calendar> at <hh:mm>` (tz-aware, DST-correct, hand-rolled
zone handling per the minimal-deps house style), and one-shot `at <hh:mm>` —
via `resolve_due_clock_sources` on each worker pass (cli/main.rs:20823-20913;
tests control_plane.rs:7591, :7691; DST correctness cli/main.rs:43587). The
pass enumerates due occurrences from the cursor (last admitted occurrence,
else instance start) to the worker-boundary clock, applies the missed policy,
and admits each as a durable signal fact keyed by
`occurrence_id = H(source, scheduled_instant)`; the cursor is read from the
append-only event log so a consumed occurrence cannot regress it
(store/lib.rs:4676, :4727). The previous revision of this note — and the
`resolve_due_clock_sources` doc comment (cli/main.rs:20817-20822) — claimed
the calendar and `at` forms do not fire; both claims are stale. Slice T2
purges the code comment.

## Providers

One provider (seam classification per the ecosystem shape's "M2" decision):

```text
std.time.clock   local/store-backed clock source provider (SHIPPED)
```

None of the three designated provider seams (HTTP sans-IO step machine /
subprocess adapter / native adapter trait) applies, because no effect is ever
claimed or dispatched: the provider IS `resolve_due_clock_sources` executing
inside the worker pass over the held store. It is a pre-existing fourth
execution shape — worker-pass scheduler — and stays that way. The fixture
story is the test-harness clause `given clock at "<instant>"`
(parser/lib.rs:18469-18486).

The local provider does not require a hidden daemon; operators may run a
scheduler loop, a hosted worker, systemd, cron, or another process that
periodically invokes WhippleScript. The durable source state records what was
observed and emitted.

DO plane (per "M7"): recurrence resolution lives in cli/main.rs and is NOT
kernel-lifted, so clock sources do not fire on the wasm DO runtime until the
DO tracker's alarms phase
([`durable-object-runtime-tracker.md`](durable-object-runtime-tracker.md), P6
alarms; the chunk-4 tail owns the lift of clock/timer resolution). That is a
registered dependency, not designed here — **DO alarms stay in the DO
tracker**.

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

## Ingress Boundary (mirrored in the std.ingress design)

Per the ecosystem shape's "E2" decision, both package specs state this
boundary:

- **Shared family — std.ingress owns the family.** The
  `source <provider> as <name>` grammar, the `observe as`/`emit` mapping
  shape, and family-level static checks — including the missing
  emit-names-a-declared-signal / payload-materialization check (quick win Q1)
  — are std.ingress's, and apply to clock sources identically.
- **std.time owns:** the `clock` provider kind; recurrence, timezone, and
  missed-policy clauses and their checks; occurrence identity policy; the
  `ClockObservation` schema; the clock occurrence-resolution logic
  (`resolve_due_clock_sources`) that the core-owned worker source pass
  drives.
- **std.ingress owns:** the admission drivers — the `whip signal` operator
  door, signal declarations, source-provider capability reports, and every
  non-clock provider kind (http/stdio/file/…). (Status 2026-07-15: the
  file/http worker-pass pollers and the `whip ingress serve --stdio` driver
  all run through the shared admission core,
  kernel/src/ingress_pass.rs; the earlier "cli/main.rs:20831 skips every
  non-clock source" state is gone.)
- **Core owns:** the source-resolving worker pass and the admission core
  (schema validation, the at-most-once dedup index, the H8 signal-integrity
  gate, fact derivation) — lifecycle invariants per the ecosystem shape's
  "E7"; packages contribute provider kinds the pass drives, and neither
  package claims the pass itself (mirrored from std-ingress.md "Boundary
  with `std.time`").
- **Shared plane:** both admit through the same at-most-once index
  `events(instance_id, idempotency_key)` (store migration 0001:75-76); there
  is no hidden conversion between them.

## Manifest

Embedded manifest `std.time` (per "M5"; rides ecosystem slice S6), validated
by the same pipeline as third-party manifests, catalog-privileged:

- **libraries:** `std.time` v0, standard:true. This closes the shipped
  identity gap: today clock sources register NO library (the
  auto-registration list at parser/lib.rs:2700-2715 lacks std.time) and
  `use std.time` is inert.
- **effect_contracts:** none.
- **constructs:** the `source clock` registration — family
  `source_declaration`, lowering class `clock_source` (core/lib.rs:79-80).
  Per "E4", this registration is what exercises the `clock_source` catalog
  class. Like std.ingress's `signal_source`/`signal_emit` and std.coord's
  `resource_effect`, `clock_source` is `package_authorable: false` today, so
  this manifest is rejected by the shipped validation pipeline until S6 adds
  the catalog-privilege / lowering-authorability extension that std.ingress
  (the shared source-family owner) registers as an S6 obligation; std.time
  contributes only the `clock_source` data, not that mechanism.
- **capabilities:** none (no effect operations — see "Surface").
- **providers:** `std.time.clock` (source provider kind `clock`). (Status
  2026-07-15: the shipped `std/manifests/time.json` carries this as an
  operator-plane provider row `{"id": "clock", "provider_kind": "clock",
  "plane": "operator"}` — added with std.ingress slice I2b so the
  provider-kind-known hard check derives its ENTIRE known-kind set from
  manifest data; the construct row above remains unshipped.)
- **profiles:** none.

Import bite: advisory missing-import lint in v1 (a program declaring a clock
source without `use std.time` lints); lint→hard escalation is a registered
later one-way cleanup, not built (per "M5").

## Static Checks

Tier assignment (per "M8"): every check below is a Tier-2 hand-coded core
check with `std.time` named here as owner — packages bring check DEMANDS,
core implements and enforces. No catalog-flag generic check applies
(rule-of-three not met).

Shipped (parser/lib.rs:18798-18807, :6414-6444; tests :27976, :28005):

- A clock source must declare a recurrence form.
- A recurring schedule must declare `missed` (no silent default).
- A calendar schedule should declare `timezone`; if omitted (and no explicit
  provider default is recorded), the checker defaults to UTC and emits a
  diagnostic recommending an explicit anchor (the periodic-anchor rule of
  [`admission-and-idempotency.md`](admission-and-idempotency.md)).
- A clock source cannot fire a rule directly (holds by construction: sources
  only admit signal facts).
- `now` or any current-clock expression remains forbidden in guards and rule
  expressions (holds by construction: no such grammar exists).

Demanded, not yet shipped:

- `emit <signal> { ... }` must name a declared signal and materialize its
  payload type — VERIFIED missing today (a source emitting an undeclared
  signal passes `whip check` clean). Family-level check owned by std.ingress's
  Q1 quick-win slice, which MUST cover clock sources; std.time contributes the
  clock fixture.
- `source clock as <name>` requires `use std.time` — advisory lint in v1 per
  the "M5" ladder (slice T1), escalation registered.
- The source provider kind (`clock`) must be contributed by an imported
  package's provider row — advisory in v1 (slice T1); today any identifier is
  accepted.
- `observe as <binding>` binds the `ClockObservation` schema (field-set truth
  gated on slice T2).

## Information-Flow Face

Posture per DR-0029 (cross-package information flow): `std.time` adds **no
new membrane door**.

- Clock observations carry no external content: every payload field derives
  from the declared schedule plus the worker-boundary clock read. Integrity is
  internal/worker — a clock source cannot launder external input into a
  signal fact.
- Admission rides the same signal-admission plane as std.ingress, so H8
  signal-integrity carriage applies unchanged (internal-marked signals keep
  emitter integrity; ifc.rs:559, :1063): a clock source emitting an
  internal-governed signal is an internal emitter and needs no exemption.
- No egress, no secrets: the package reads no credentials and writes to no
  external sink. Where the same signal is also externally injectable via
  `whip signal`, each fact carries its recorded source identity and labels
  join at the fact plane.

## Non-Goals

- No direct rule firing.
- No hidden daemon requirement.
- No current clock in guards.
- No cron-provider zoo in the first pass.
- No business-calendar integrations in the first pass.
- No one-shot timer replacement; core `timer` remains the one-shot mechanism.

## v1 Implementation Slices

Each slice is independently gateable under the per-piece review discipline.

- **T1 — package identity.** Embedded `std.time` manifest; register the
  library both on clock-source usage and on `use std.time`; advisory
  missing-import lint; advisory provider-kind-from-package check. Tests:
  manifest validates through the third-party pipeline; contract-registry
  snapshot gains std.time standard:true; lint fixtures (clock source without
  import → advisory, with import → clean). Model: no new property here — the
  std lock-exemption re-key and its
  `models/maude/std-construct-authorization.maude` re-model are owned by
  ecosystem slice S6, which this slice rides.
- **T2 — observation fidelity + staleness purge.** Emit `schedule_name` from
  `clock_emit_payload` (cli/main.rs:20877-20884) so the runtime payload
  matches the declared `ClockObservation` schema; purge the stale
  `resolve_due_clock_sources` doc comment (cli/main.rs:20817-20822). Tests: an
  e2e fixture asserts the full observation field set including
  `schedule_name`. Model: payload-shape only; `clock-source.maude` and
  `ClockSourceLifecycle.tla` properties unchanged.
- **T3 — `at <hh:mm>` semantics pinned.** Spec text pinned under "Recurrence
  Forms" (fire-once); add the explicit regression test (fires exactly once,
  no repeat the next day). Model: verify `ClockSourceLifecycle.tla`'s
  occurrence-uniqueness coverage includes the one-shot form; extend if not.

## Deferred With Cause

- **DO-plane recurrence (alarms).** Cause: "M7" one-concern-one-tracker — DO
  alarms and the kernel lift of `resolve_due_clock_sources` belong to
  [`durable-object-runtime-tracker.md`](durable-object-runtime-tracker.md)
  (P6 alarms / chunk-4 tail). Re-entry: that tracker's phase.
- **Cron-compatible syntax.** Cause: zero demand; readability-first surface.
  Re-entry: author demand → conservative grammar extension over the same
  `clock_source` lowering.
- **Business-calendar integrations.** Cause: provider-zoo risk. Re-entry: a
  concrete calendar-feed demand — likely entering as a std.ingress source
  provider, not std.time grammar.
- **Schedule/source status projection** (a `whip sources` read surface for
  "schedule/source status" in the ownership table). Cause: no operator demand
  yet; durable records already show what fired. Re-entry: operator demand;
  mirrors the `whip leases`/`whip counters` projection pattern.
- **Import lint→hard escalation.** Registered one-way cleanup per "M5"; not
  built in v1.

## Spec Amendments

- **spec/std-time.md, formerly inside "Missed Occurrence Policy" (this
  edit):** the 2026-06-18 "Realized" claim that only the interval form fires
  was stale; replaced by "Realized Runtime Surface" (all three forms fire,
  with test citations).
- **spec/admission-and-idempotency.md, clock-occurrence admission key:** the
  shipped at-most-once index is `events(instance_id, idempotency_key)` (store
  migration 0001:75-76); the spec says `(instance, fact_identity_key)`. Same
  mechanism — align the vocabulary to the shipped column names
  (terminology-only amendment).
- **spec/event-ingress.md, source family / provider contract:** add the
  mirrored boundary statement from "Ingress Boundary" above — family-level
  source checks (including Q1 emit-target validation) are owned by
  std.ingress and must cover clock sources; the `clock` provider kind,
  recurrence grammar, and occurrence identity policy are owned by std.time.
  The std.ingress package design carries the same statement (per "E2").

## Modeling Notes

- **Replay safety:** replay reads recorded signal facts, not the clock.
- **No direct fire:** clock sources emit signal facts only through admission.
- **Occurrence idempotency:** the `occurrence_id` is the kernel-derived
  clock-occurrence key `H(source_id, scheduled_occurrence_instant)`
  ([`admission-and-idempotency.md`](admission-and-idempotency.md)); the store's
  unique index — shipped as `events(instance_id, idempotency_key)` — is what
  prevents a duplicate signal fact for the same occurrence. `std.time` asserts
  no dedup of its own.
- **Missed policy explicitness:** every recurring source has a declared missed
  policy, so catch-up/skip behavior is inspectable.
