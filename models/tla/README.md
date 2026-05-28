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

Current local workspace status:

```text
java: not found on PATH
apalache: not found on PATH
```

The model skeleton is present, but it is not currently runnable here.
