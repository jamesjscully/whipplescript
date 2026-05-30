# Managed Spec Orchestration Example

This example sketches the target shape for orchestrating coding agents across a
large spec implementation. It assumes a future managed WhippleScript/un-tie adapter
exists. The goal is to show the contracts and control-flow shape, not today's
SDK surface.

The design keeps three things separate:

- Contracts define enforceable repo/workstream/thread boundaries.
- A JSON implementation ledger is durable project state.
- A tiny supervisor script notices completed workers and idle runtime state,
  then messages the director agent.

The supervisor does not schedule work and does not own workflow state. The
director agent reads the ledger, decides what to spawn next, updates state, and
asks the runtime to start bounded workers or quality gates.

## Files

- `contracts/repo.contract.json` defines repo-wide permissions, state files,
  completion semantics, and allowed capabilities.
- `contracts/workstreams/spec-implementation.contract.json` narrows the contract
  for this implementation workstream.
- `contracts/thread-default.contract.json` defines defaults inherited by worker
  threads.
- `builder_only/orchestration/` contains protected prompts and the agent-visible
  contract summary.
- `state/implementation-plan.json` is the dynamic scheduling ledger owned by the
  repo.
- `scripts/supervisor.sh` is a minimal process that nudges the director when
  agent runs complete or when the whole system is idle.

## Intended Runtime Wiring

```toml
[[managed_loop]]
name = "spec-implementation-supervisor"
contract = "contracts/workstreams/spec-implementation.contract.json"
state = "state/implementation-plan.json"
run = "bash scripts/supervisor.sh"
```

Under the hood, the managed loop runs the supervisor with narrow capabilities:
read WhippleScript status/runs, read the ledger, and send a message to the director.
Worker threads still run through the same agent/session sandbox and
`.agent-config.json` policy used by un-tie.

The script is intentionally polling based. It does not depend on in-process
callbacks surviving after the script exits, and it does not try to encode the
implementation workflow. The director agent remains responsible for interpreting
the contract and ledger.

## Runtime Requirements

- `whip`
- `jq`
- Either `untie thread send <thread-id> --stdin` or an WhippleScript task listening
  for `director.message`

Useful environment variables:

```sh
DIRECTOR_THREAD_ID=thread_123
WORKSPACE=/path/to/workspace
PLAN_FILE=state/implementation-plan.json
POLL_SECONDS=15
IDLE_NUDGE_SECONDS=120
```
