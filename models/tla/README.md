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
```

Current model:

```text
ControlPlaneLifecycle.tla
```

It encodes a first-pass runtime lifecycle:

```text
append event
derive projection cursor
claim effect
start run
complete/fail run
expire lease
start/finish recovery from the durable event log
pause/resume/cancel
dependency-gated claimability
```

It names safety invariants for:

```text
run/effect references
claimed/running run consistency
claimability and dependency satisfaction
paused instances not producing new claimable work
terminal effects staying terminal
projection cursor bounds
recovery preserving event-log order
basic type correctness
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

The default script typechecks these formulas with Apalache. It does not treat
full temporal liveness proof as a v0 release gate; the formulas are kept in the
model so future TLC/Apalache temporal-checking work has a stable target.

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

CI policy:

```text
run TLA+/Apalache in default CI
keep generated per-program Maude model search opt-in from the CLI
```

This keeps durable control-plane regressions in the normal gate while avoiding a
formal-tool requirement for ordinary local `whip check` usage.
