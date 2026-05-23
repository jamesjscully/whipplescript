# Dynamic Management Implementation Plan

Status: proposed sequential implementation plan

Primary design document:

- `spec/dynamic-management-interface.md`

Supporting spec:

- `spec/armature-v0.3.md`

The goal of this plan is to align the implementation with the dynamic-management
interface without turning Armature into a workflow engine. The implementation
should keep the v0.3 boundary intact:

**Armature owns invocation truth. User code owns operational meaning.**

## Implementation Strategy

Implement the interface in slices that can land independently and keep the test
suite green. Favor black-box CLI/e2e coverage for agent desire paths.

The implementation order should prioritize:

1. A stable object-oriented CLI vocabulary.
2. Better observation and blocking primitives.
3. Ad hoc tracked process execution.
4. Safe lock recovery/ergonomics.
5. Ephemeral dynamic service/task definitions.
6. SDK/docs once CLI behavior is stable.

## Cross-Slice Rules

Every slice must follow these rules:

1. Keep behavior mechanical.
2. Do not introduce workflow DAGs, agent graphs, semantic retries, semantic dedupe, or domain state machines.
3. Preserve existing top-level aliases where practical.
4. Prefer additive CLI/API changes over breaking changes.
5. Add e2e coverage for realistic agent use.
6. Keep every dynamic object inspectable.
7. Run focused checks and at least one broader integration check before marking done.

## Slice 1: Object CLI Vocabulary

Goal: add canonical object-oriented command groups while preserving existing
top-level aliases.

Canonical forms:

```sh
armature task list
armature task show <name>
armature task run <name>

armature service list
armature service show <name>

armature run list
armature run show <run-id>
armature run logs <run-id>
armature run cancel <run-id>

armature event list
armature event show <event-id>
armature event emit <type>

armature trigger list
armature trigger show <trigger-id>

armature log show <run-id>
armature log tail <run-id> --lines N
armature log follow <run-id>
```

Existing aliases should continue:

```sh
armature tasks
armature services
armature runs
armature logs <run-id>
armature cancel <run-id>
armature events
armature triggers
armature emit <type>
armature run <task-name>
```

Expected implementation work:

- Add command groups in `crates/armature-cli/src/main.rs`.
- Reuse existing command implementations rather than duplicating logic.
- Add CLI help tests and e2e smoke tests for canonical/alias equivalence.
- Update README command examples to introduce the object model.

Acceptance checks:

```sh
cargo test -p armature-cli --bin armature
cargo test -p armature-cli --test e2e object_cli_aliases
cargo test
```

## Slice 2: Query, Wait, Subscribe

Goal: make runtime state easy for agents to observe without custom polling loops.

Query requirements:

```sh
armature event list --type TYPE --source SOURCE --correlation ID --limit N
armature trigger list --task NAME --event EVENT --outcome OUTCOME --correlation ID --limit N
armature run list --name NAME --origin ORIGIN --state STATE --correlation ID --limit N
armature lock list --expired
```

Wait requirements:

```sh
armature wait event <type> --correlation ID --timeout 30s
armature wait run <run-id> --state exited --timeout 30s
armature wait trigger --task NAME --outcome started --timeout 30s
armature wait service <name> --state running --timeout 30s
```

Subscribe requirements:

```sh
armature subscribe events
armature subscribe runs
armature subscribe triggers
```

Initial subscribe may be implemented as polling that emits newline-delimited
JSON when new records appear. It must be documented as an observation stream,
not a broker protocol.

Expected implementation work:

- Extend store filters where needed.
- Add wait loops with timeouts and stable exit behavior.
- Add subscribe streaming in text/NDJSON mode.
- Add e2e tests that avoid brittle timing.

Acceptance checks:

```sh
cargo test -p armature-cli --test e2e wait_and_subscribe_agent_flow
cargo test -p armature-daemon
cargo test
```

## Slice 3: Ad Hoc Tracked Runs

Goal: let agents run arbitrary finite commands under Armature tracking without
creating a task definition.

Canonical command:

```sh
armature run start --name NAME -- <cmd...>
```

Alias:

```sh
armature exec --name NAME -- <cmd...>
```

Useful options:

```sh
--name NAME
--correlation ID
--cwd DIR
--env KEY=VALUE
--timeout DURATION
--payload-file PATH
--stdin
```

Semantics:

- Creates one run record with origin `adhoc` or equivalent.
- Captures logs and run metadata like task/service runs.
- Does not create a task definition.
- Does not register future trigger behavior.
- Propagates provenance/correlation.

Expected implementation work:

- Add protocol request for ad hoc run start if daemon-mediated.
- Decide whether ad hoc runs require a daemon; preferred answer is yes for
  consistent tracking/cancellation.
- Add run origin/type support if current model lacks it.
- Add CLI/e2e tests for logs, cancellation, correlation, cwd/env, timeout.

Acceptance checks:

```sh
cargo test -p armature-cli --test e2e adhoc_run_is_tracked_and_cancelable
cargo test -p armature-daemon
cargo test
```

## Slice 4: Lock Recovery and Ergonomics

Goal: complete lock semantics from the dynamic interface spec.

Required commands:

```sh
armature lock force-release <name> --reason TEXT
armature lock show <name>
armature lock list --expired
armature lock with <name> --ttl DURATION --reason TEXT -- <cmd...>
```

Existing commands must remain safe:

```sh
armature lock acquire <name> --ttl DURATION --reason TEXT
armature lock renew <name> --token TOKEN --ttl DURATION
armature lock release <name> --token TOKEN
```

Tokenless release may be allowed only for the owning Armature run.

Expected implementation work:

- Add force-release protocol and audit event/record.
- Add lock show/list filters.
- Add `lock with` token capture/release behavior and exit-code propagation.
- Add e2e tests for stale token protection, force release, and `lock with`.

Acceptance checks:

```sh
cargo test -p armature-cli --test e2e lock_recovery_and_with_lock
cargo test -p armature-daemon manual_lock
cargo test
```

## Slice 5: Ephemeral Dynamic Services

Goal: allow runtime-created service definitions without editing user TOML.

Canonical commands:

```sh
armature service add <name> -- <cmd...>
armature service remove <name>
armature service show <name>
armature service list --dynamic
```

Useful options:

```sh
--correlation ID
--cwd DIR
--env KEY=VALUE
--restart never|on_failure|always
--reason TEXT
```

Initial semantics:

- Dynamic services are ephemeral.
- They live until removed, daemon shutdown, or workspace reset.
- They are inspectable and marked `dynamic: true`.
- They do not rewrite `.armature/armature.toml`.

Expected implementation work:

- Add runtime registry for dynamic service definitions.
- Reconcile dynamic services with the same process/log/run machinery as static services.
- Add provenance fields for creator run/correlation where available.
- Add e2e tests for add/start/restart/remove/shutdown cleanup.

Acceptance checks:

```sh
cargo test -p armature-cli --test e2e dynamic_service_lifecycle
cargo test -p armature-daemon
cargo test
```

## Slice 6: Ephemeral Dynamic Tasks

Goal: allow runtime-created trigger handlers without editing user TOML.

Canonical commands:

```sh
armature task add <name> --on EVENT -- <cmd...>
armature task add <name> --watch GLOB --settle 500ms -- <cmd...>
armature task add <name> --schedule CRON -- <cmd...>
armature task remove <name>
armature task show <name>
armature task list --dynamic
```

Initial semantics:

- Dynamic tasks are ephemeral.
- They live until removed, daemon shutdown, or workspace reset.
- They are inspectable and marked `dynamic: true`.
- Event/watch/schedule routing is the same mechanical path as static tasks.
- They do not create workflow state.

Expected implementation work:

- Add runtime registry for dynamic task definitions.
- Merge static and dynamic task views for routing and inspection.
- Add e2e tests for event, watch, and removal behavior.

Acceptance checks:

```sh
cargo test -p armature-cli --test e2e dynamic_task_event_and_watch_lifecycle
cargo test -p armature-daemon
cargo test
```

## Slice 7: SDK and Documentation Alignment

Goal: expose the stable dynamic-management surface through the SDK and docs
without adding a second runtime.

Expected SDK additions:

```ts
armature.task.list()
armature.task.run(name)
armature.run.start(...)
armature.event.emit(...)
armature.wait.event(...)
armature.service.add(...)
armature.task.add(...)
armature.lock.with(...)
```

Rules:

- SDK remains thin over CLI/protocol/env surfaces.
- No workflow helpers.
- No agent graph helpers.
- No semantic retry helpers.

Documentation updates:

- README object model overview.
- Dynamic-management examples.
- Lock recovery examples.
- Agent desire-path examples.
- Migration notes from top-level aliases to canonical object commands.

Acceptance checks:

```sh
npm test --workspace @armature/sdk
cargo test
```

## Release Readiness Checklist

Before declaring the dynamic-management alignment complete:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release
cargo test -p armature-cli --test e2e -- --ignored sustained_stress_many_events_watch_changes_and_services
npm test --workspace @armature/sdk
```

Manual smoke:

```sh
armature up
armature service add github-source -- node sources/github.mjs
armature task add planner --on agent.requested -- node planner.mjs
armature emit agent.requested --correlation req-1 --payload-file request.json
armature wait event work.completed --correlation req-1 --timeout 5m
armature runs --correlation req-1
armature lock with repo:main --ttl 1m -- echo ok
armature down
```
