# Time: effect timeouts, timers, and cancellation

Status: spec drafted 2026-06-09 from decided design
([`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md) A2).
Stage: spec -> modeling -> implementation + testing -> review.

## Surface

```whip
tell worker as turn timeout 10m "Do the task."

askHuman as signoff timeout 24h """markdown
Approve or reject the plan.
"""

timer 24h as deadline

after deadline succeeds {
  cancel signoff
  fail error { reason "no answer within 24h" }
}
```

### `timeout <duration>`

A clause accepted on any effect statement (`tell`, `askHuman`, `coerce`,
`decide`, `call`, `invoke`, `exec`, queue verbs). Grammar position: after
the binding (`as x`), before the prompt/payload.

- The clock anchors at **effect creation** (deadline semantics: predictable,
  anchored to a visible log event, unified with timers). Run-start clocks
  were rejected — they hide capacity stalls, the failure mode timeouts
  exist to catch.
- On expiry the runtime marks the effect `timed_out` (terminal: fires
  `after ... fails`/`completes` branches and flow `on timeout` handlers)
  and **requests** provider cancellation for a running attempt — a request,
  not a result, same discipline as `revise --cancel running`. The
  provider's actual termination is recorded whenever it happens.
- Expiry is recorded as an event plus evidence (deadline, observed time).

### `timer <duration> as <binding>`

Creates a `timer.wait` effect, completed by the runtime when due. Timers
are ordinary effects: rule determinism is untouched, completions fire
`after` branches, everything is inspectable.

### `cancel <binding>`

A rule/flow body operation (v1, by decision):

- Pending (queued/blocked) effect: terminal-cancels it.
- Running effect: requests provider cancellation (request, not result).
- Already-terminal effect: no-op, recorded as evidence.

Primary motivation: the ask + timer escalation race can explicitly cancel
the losing effect (clean inboxes, no zombie provider work). Guard mismatch
remains the documented fallback for un-cancelled losers: a late branch
finds its facts consumed and does nothing.

### Durations

Literals `<integer><unit>` with units `s`, `m`, `h`, `d`. Parsed in the
expression layer (the parser already exposes `parse_duration_seconds`).

## Runtime

No daemon. Due timers and expired timeouts are observed and resolved on
`worker`/`dev` passes; external schedulers own wall-clock wakeups. This
matches the work-queue projection model (state advances when a worker
runs).

- **Idle**: `dev --until idle` treats pending timers/timeouts as idle —
  idle = no immediately runnable work — so dev runs never hang on long
  deadlines. `status` lists pending timers and effect deadlines with due
  times.
- Wall-clock reads live only in the worker/runtime layer, never in rule
  evaluation.

## Static checks

- `timeout` with zero/negative or unparseable duration: check error.
- `cancel x` where `x` is not an effect binding in scope: check error.
- A flow `on timeout` handler on a step without a `timeout` clause: check
  warning (handler can never fire unless a provider times out on its own).

## Modeling notes

- Exactly-once expiry: an effect with a timeout reaches `timed_out` at most
  once, never after another terminal status, and its dependency branches
  fire exactly once (extend the effect-lifecycle property tests; TLA+
  lifecycle model gains a deadline transition if tooling permits).
- Cancel idempotency: `cancel` on any status is safe; double-cancel is a
  recorded no-op.
- Timer/ask race: whichever terminal lands first wins; the loser's branch
  finds consumed facts or is cancelled — no execution path runs both
  branches' terminals (would be caught by the single-terminal guard).
