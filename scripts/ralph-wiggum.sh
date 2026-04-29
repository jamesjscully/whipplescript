#!/usr/bin/env bash
set -euo pipefail

# Launch a deliberately broad set of Paseo agents against the Armature v0.3 plan.
# This uses the documented Paseo CLI, which is the daemon API surface for scripts.

usage() {
  cat <<'USAGE'
Usage: scripts/ralph-wiggum.sh [--run] [--wait] [--provider PROVIDER]

By default this is a dry run. Pass --run to create detached Paseo agents.

Environment:
  PASEO_BIN          Paseo command to invoke. Default: paseo
  PASEO_PROVIDER     Provider passed to paseo run. Default: codex
  PASEO_RUN_FLAGS    Extra flags appended to every paseo run.

Examples:
  scripts/ralph-wiggum.sh
  scripts/ralph-wiggum.sh --run
  PASEO_RUN_FLAGS="--mode full-access" scripts/ralph-wiggum.sh --run --wait
USAGE
}

run=0
wait_for_agents=0
paseo_bin="${PASEO_BIN:-paseo}"
provider="${PASEO_PROVIDER:-codex}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run)
      run=1
      shift
      ;;
    --dry-run)
      run=0
      shift
      ;;
    --wait)
      wait_for_agents=1
      shift
      ;;
    --provider)
      provider="${2:?missing provider}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

if [[ ! -f "$repo_root/spec/implementation-plan-v0.3.md" ]]; then
  echo "expected Armature repo root at $repo_root" >&2
  exit 1
fi

if ! command -v "$paseo_bin" >/dev/null 2>&1; then
  echo "paseo command not found: $paseo_bin" >&2
  exit 1
fi

common_header="You are implementing Armature v0.3 in this repository.

Read these first:
- spec/armature-v0.3.md
- spec/implementation-plan-v0.3.md

Keep Armature narrow: trigger, launch, monitor, record, supervise, reconcile runtime, inspect. Do not introduce workflow DAGs, durable promises, agent graphs, semantic retries, semantic dedupe, adapters, capabilities, cloud coordination, Windows support, or armature plan.

You are not alone in the codebase. Other agents may be editing adjacent areas. Do not revert work you did not make. Keep your changes inside the ownership scope below unless a small shared change is required, and call that out clearly.

Before finishing, run the relevant tests or checks for your slice. Commit your changes with a direct atomic commit message if the repo instructions allow it."

declare -a names=(
  "armature-foundation"
  "armature-config"
  "armature-store"
  "armature-daemon"
  "armature-triggers"
  "armature-cli"
  "armature-sdk"
  "armature-recipes"
)

declare -a worktrees=(
  "armature-foundation"
  "armature-config"
  "armature-store"
  "armature-daemon"
  "armature-triggers"
  "armature-cli"
  "armature-sdk"
  "armature-recipes"
)

declare -a prompts=(
  "$common_header

Ownership: repository scaffolding, Rust workspace/package layout, shared core types, ID helpers, error conventions, and baseline tests.

Goal: create the initial buildable project structure for the Rust CLI/daemon/core and the TypeScript SDK package. Keep boundaries clean enough for other workers to fill in config, store, daemon runtime, CLI commands, and SDK APIs without reorganizing the repo."

  "$common_header

Ownership: Armature TOML config model, validation, normalized config hashing, workspace discovery, config check behavior, and config fixture tests.

Goal: implement strict boundary validation for tasks, services, triggers, admission, supervision, health checks, resources, and recipes. Workspace discovery must search nearest ancestor upward for .armature/armature.toml and never search downward."

  "$common_header

Ownership: internal state location, SQLite schema/migrations-for-v0 bootstrap, event/run/log persistence interfaces, run directory layout, and store tests.

Goal: put the SQLite database outside the working tree under the XDG state location described in the plan, keyed by a stable hash of canonical workspace path. Keep logs and per-run artifacts inspectable and isolated."

  "$common_header

Ownership: daemon lifecycle, Unix socket transport, runtime reconciliation, service process supervision, process groups, cancellation, hard timeouts, hot config reload, and daemon tests.

Goal: implement the mechanical runtime. Services reconcile automatically. Tasks do not restart by default. Invalid config reloads must keep the prior valid config active. Do not add workflow semantics."

  "$common_header

Ownership: manual, schedule, file-watch, and event trigger sources; event emission path; event-triggered task invocation; file settling; admission policy mechanics.

Goal: implement primitive trigger detection and routing. Supported admission values are allow, reject, restart, queue_one, and queue_all. Rejected/coalesced/superseded triggers must remain inspectable."

  "$common_header

Ownership: armature CLI commands and terminal/JSON output surfaces.

Goal: implement init, dev, up, down, restart, run, emit, status, ps, tasks, services, runs, logs, cancel, config check, doctor, and lock commands against the daemon socket or direct config path where appropriate. Foreground operation must be explicit through dev or a foreground option."

  "$common_header

Ownership: packages/sdk TypeScript API, package metadata, build/test setup, and SDK docs/examples.

Goal: implement the optional thin SDK over env vars, CLI/daemon calls, event parsing, emit, run, status, runs, services, logs, locks, structured logging, readJson, and withLock. The SDK must not introduce a second runtime or workflow helpers."

  "$common_header

Ownership: editable recipe scaffolding and example assets only.

Goal: implement recipes as generated normal files. Include useful starter recipes without external product names: file-watch tests, scheduled status script, generic event source service, event hook task, and explicit named lock example. Recipes must not create hidden daemon behavior."
)

echo "Armature repo: $repo_root"
echo "Paseo command: $paseo_bin"
echo "Provider: $provider"
echo

if [[ "$run" -eq 0 ]]; then
  echo "Dry run. Pass --run to launch these detached agents."
  echo
fi

launched=()

for i in "${!names[@]}"; do
  name="${names[$i]}"
  worktree="${worktrees[$i]}"
  prompt="${prompts[$i]}"

  cmd=("$paseo_bin" run --provider "$provider" --detach --worktree "$worktree" --name "$name")
  if [[ -n "${PASEO_RUN_FLAGS:-}" ]]; then
    # shellcheck disable=SC2206
    extra_flags=($PASEO_RUN_FLAGS)
    cmd+=("${extra_flags[@]}")
  fi
  cmd+=("$prompt")

  if [[ "$run" -eq 0 ]]; then
    printf 'would launch %-22s worktree=%s\n' "$name" "$worktree"
  else
    echo "launching $name..."
    (
      cd "$repo_root"
      "${cmd[@]}"
    )
    launched+=("$name")
  fi
done

if [[ "$run" -eq 0 ]]; then
  exit 0
fi

echo
echo "launched ${#launched[@]} agents"

if [[ "$wait_for_agents" -eq 1 ]]; then
  for name in "${launched[@]}"; do
    echo "waiting for $name..."
    "$paseo_bin" wait "$name"
  done
fi

echo
echo "inspect with:"
echo "  $paseo_bin ls -a"
echo "  $paseo_bin logs <agent-id-or-name> --tail 50"
