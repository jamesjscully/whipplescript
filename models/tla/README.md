# TLA+ Models

TLA+ is for durable control-plane lifecycle validation, not per-program user
checks in v0.

Model:

```text
event log
projection cursor
effect outbox
effect dependencies
leases
workers
runs
crash/recovery
pause/resume/cancel
revision cancellation policy
terminal event records
```

Current model:

```text
ControlPlaneLifecycle.tla
NativeProviderLifecycle.tla
```

It encodes a first-pass runtime lifecycle:

```text
append event
derive projection cursor
claim effect
start run
complete/fail/timeout run
cancellation request acknowledgement
cancel run after acknowledgement
expire lease
retry failed or timed-out effect
start/finish recovery from the durable event log
pause/resume/cancel
workflow complete/fail terminal states
dependency-gated claimability
revision activation policies for old queued/running effects
```

It names safety invariants for:

```text
run/effect references
claimed/running run consistency
claimability and dependency satisfaction
paused instances not producing new claimable work
terminal instances not producing new claimable work
cancelled/completed/failed instance states remaining mutually exclusive
current terminal-effect set matching effect status
run-scoped lease consistency
at most one run executing a given effect at once (concurrent-worker safety)
retry removing current terminal status before a new attempt
projection cursor bounds
recovery preserving event-log order
recovery blocking live-state mutation until finish
explicit terminal event records not duplicating a run/effect outcome
revision cancellation policy and requestability gates
basic type correctness
```

`NativeProviderLifecycle.tla` is a focused native-provider fixture for:

```text
cancellation acknowledgement not fabricating terminal cancellation
provider terminal evidence recovery
required artifact-capture failure preventing successful completion
duplicate terminal outcome prevention
terminal event records matching the terminal set
```

It also names weak-fairness and liveness goals:

```text
FairSpec
LivenessGoals
ClaimableEffectEventuallyRunsOrStops
RunningEffectEventuallyTerminalsOrRecovers
ProjectionEventuallyCatchesUp
RecoveryEventuallyFinishes
```

## Information-flow models (temporal layer)

Two models carry the information-flow properties that are about a *sequence* of
steps over time, complementing the single-step Maude bites and the timeless Lean
algebra:

```text
InfoflowReleaseBudget.tla
InfoflowLabelCarriage.tla
```

`InfoflowReleaseBudget.tla` (DR-0030 Direction C) checks, over all traces, that the
checked declassifier never exceeds the public release budget and that no committed
privileged release ever carries a tainted (adaptively-derived) selector — the
no-adaptive-oracle safety surrogate.

`InfoflowLabelCarriage.tla` (I-IFC7, audit-findings W6) checks that a datum's label
is carried across every transport hop — persist, reload, cross-instance handoff,
replay — without being **stripped** (`NoStrip`, confidentiality silently lowered) or
**forged/laundered** (`NoForge`, integrity silently raised). It is the inductive,
all-interleavings complement to `infoflow-carriage.maude`'s single hop. *Bite
(verified to fail):* adding a `LaunderHandoff` transport that rewrites integ to 1
produces an Apalache counterexample at the `NoForge`/`CarriagePreserved` invariant.
Label *crossings* (declassify/endorse) are authorized exceptions modeled in the
crossing models, not here.

The default script typechecks these formulas with Apalache and runs a bounded
safety check over `SafetyInvariants` using `ConstInit`, a small finite harness.
It does not treat full temporal liveness proof as a v0 release gate; the
formulas are kept in the model so future TLC/Apalache temporal-checking work
has a stable target.

Current local workspace status:

```text
java: provided by the repo Nix dev shell
apalache: provided by the repo Nix dev shell
```

Run:

```sh
scripts/check-tla-models.sh
```

If `apalache-mc` is not already on `PATH`, the script enters the repo Nix dev
shell and runs the check there.

The bounded safety depth defaults to `6`. Override it when doing deeper local
validation:

```sh
WHIPPLESCRIPT_TLA_LENGTH=10 scripts/check-tla-models.sh
```

CI policy:

```text
run TLA+/Apalache in default CI
keep generated per-program Maude model search opt-in from the CLI
```

This keeps durable control-plane regressions in the normal gate while avoiding a
formal-tool requirement for ordinary local `whip check` usage.
