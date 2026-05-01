# armature

Armature is a lightweight local daemon and CLI for running ordinary programs from
schedules, file changes, emitted events, and supervised long-running services.

## Install

```sh
cargo build -p armature-cli
alias armature="$PWD/target/debug/armature"
```

## Initialize

```sh
armature init
armature config check
```

Example config:

```toml
[[task]]
name = "test"
watch = ["src/**/*", "tests/**/*"]
run = "cargo test"

[[task]]
name = "on-build-event"
on = "build.completed"
run = "./scripts/on-build-completed.sh"

[[service]]
name = "worker"
run = "./scripts/worker.sh"
```

## Run The Daemon

```sh
armature up
armature dev
armature down
```

`armature up` starts the daemon detached, or reloads config when a daemon is
already running. `armature dev` runs the daemon in the foreground.

## Trigger Work

```sh
armature task run test
armature event emit build.completed --json '{"runId":"run_123","ok":true}'
```

The v0.3 aliases remain available:

```sh
armature run test
armature emit build.completed --json '{"runId":"run_123","ok":true}'
```

## Inspect Runtime Objects

Canonical object-oriented commands:

```sh
armature task list
armature task show test
armature service list
armature service show worker
armature run list
armature run show <run-id>
armature run logs <run-id>
armature run cancel <run-id>
armature event list
armature event show <event-id>
armature trigger list
armature trigger show <trigger-id>
armature log show <run-id>
armature log tail <run-id> --lines 100
```

Existing inspection aliases still work:

```sh
armature tasks
armature services
armature runs
armature logs <run-id>
armature logs --tail 100 <run-id>
armature cancel <run-id>
armature events
armature triggers
```

Use `--format json` globally for machine-readable output:

```sh
armature --format json run list
armature --format json event list
```

## Services And Locks

```sh
armature service start worker
armature service stop worker
armature service restart worker

armature --format json lock acquire branch:main --ttl 10m
armature lock status
armature lock release branch:main
```

Armature owns mechanical invocation truth: triggers, launches, process state,
logs, events, locks, and runtime inspection. User code owns semantic meaning.
