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
basic type correctness
```

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
