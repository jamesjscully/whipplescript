#!/usr/bin/env bash
set -euo pipefail

# Sequential Paseo implementation loop for WhippleScript v0.3.
# The shared editable plan state is docs/plans/implementation-state-v0.3.md.

usage() {
  cat <<'USAGE'
Usage: scripts/ralph-wiggum.sh [--run] [--from SLICE] [--only SLICE] [--provider PROVIDER]

By default this is a dry run. Pass --run to start the sequential Paseo loop.
Each agent runs to completion before the next slice starts.

Slices:
  foundation config store daemon triggers cli sdk recipes

Environment:
  PASEO_BIN          Paseo command to invoke. Default: paseo
  PASEO_PROVIDER     Provider passed to paseo run. Default: codex/gpt-5.4
  PASEO_RUN_FLAGS    Extra flags appended to every paseo run.

Examples:
  scripts/ralph-wiggum.sh
  scripts/ralph-wiggum.sh --run
  scripts/ralph-wiggum.sh --run --from daemon
  scripts/ralph-wiggum.sh --run --only sdk
  PASEO_RUN_FLAGS="--mode full-access" scripts/ralph-wiggum.sh --run
USAGE
}

run=0
from_slice=""
only_slice=""
provider="${PASEO_PROVIDER:-codex/gpt-5.4}"

default_paseo_bin() {
  local candidate resolved bundle_cli
  candidate="$(command -v paseo 2>/dev/null || true)"
  if [[ -z "$candidate" ]]; then
    echo "paseo"
    return
  fi

  resolved="$(readlink -f "$candidate" 2>/dev/null || printf '%s' "$candidate")"
  if [[ "$(basename -- "$resolved")" == "Paseo.bin" ]]; then
    bundle_cli="$(dirname -- "$resolved")/resources/bin/paseo"
    if [[ -x "$bundle_cli" ]]; then
      echo "$bundle_cli"
      return
    fi
  fi

  echo "$candidate"
}

paseo_bin="${PASEO_BIN:-$(default_paseo_bin)}"

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
    --from)
      from_slice="${2:?missing slice}"
      shift 2
      ;;
    --only)
      only_slice="${2:?missing slice}"
      shift 2
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
state_file="docs/plans/implementation-state-v0.3.md"

if [[ ! -f "$repo_root/spec/whipplescript-v0.3.md" ]]; then
  echo "expected WhippleScript repo root at $repo_root" >&2
  exit 1
fi

if [[ ! -f "$repo_root/$state_file" ]]; then
  echo "missing state file: $state_file" >&2
  exit 1
fi

if ! command -v "$paseo_bin" >/dev/null 2>&1; then
  echo "paseo command not found: $paseo_bin" >&2
  exit 1
fi

declare -a slices=(
  "foundation"
  "config"
  "store"
  "daemon"
  "triggers"
  "cli"
  "sdk"
  "recipes"
)

declare -A names=(
  [foundation]="whipplescript-foundation"
  [config]="whipplescript-config"
  [store]="whipplescript-store"
  [daemon]="whipplescript-daemon"
  [triggers]="whipplescript-triggers"
  [cli]="whipplescript-cli"
  [sdk]="whipplescript-sdk"
  [recipes]="whipplescript-recipes"
)

declare -A ownership=(
  [foundation]="repository scaffolding, Rust workspace/package layout, shared core types, ID helpers, error conventions, and baseline tests"
  [config]="WhippleScript TOML config model, validation, normalized config hashing, workspace discovery, config check behavior, and config fixture tests"
  [store]="internal state location, SQLite schema/bootstrap, event/run/log persistence interfaces, run directory layout, and store tests"
  [daemon]="daemon lifecycle, Unix socket transport, runtime reconciliation, service process supervision, process groups, cancellation, hard timeouts, hot config reload, and daemon tests"
  [triggers]="manual, schedule, file-watch, and event trigger sources; event emission path; event-triggered task invocation; file settling; admission policy mechanics"
  [cli]="whipplescript CLI commands and terminal/JSON output surfaces"
  [sdk]="packages/sdk TypeScript API, package metadata, build/test setup, and SDK docs/examples"
  [recipes]="editable recipe scaffolding and example assets only"
)

declare -A goals=(
  [foundation]="Create the initial buildable project structure for the Rust CLI/daemon/core and the TypeScript SDK package. Keep boundaries clean enough for later slices to fill in behavior without reorganizing the repo."
  [config]="Implement strict boundary validation for tasks, services, triggers, admission, supervision, health checks, resources, and recipes. Workspace discovery must search nearest ancestor upward for .whipplescript/project.whip and never search downward."
  [store]="Put the SQLite database outside the working tree under the XDG state location described in the plan, keyed by a stable hash of canonical workspace path. Keep logs and per-run artifacts inspectable and isolated."
  [daemon]="Implement the mechanical runtime. Services reconcile automatically. Tasks do not restart by default. Invalid config reloads must keep the prior valid config active. Do not add workflow semantics."
  [triggers]="Implement primitive trigger detection and routing. Supported admission values are allow, reject, restart, queue_one, and queue_all. Rejected/coalesced/superseded triggers must remain inspectable."
  [cli]="Implement init, dev, up, down, restart, run, emit, status, ps, tasks, services, runs, logs, cancel, config check, doctor, and lock commands against the daemon socket or direct config path where appropriate. Foreground operation must be explicit through dev or a foreground option."
  [sdk]="Implement the optional thin SDK over env vars, CLI/daemon calls, event parsing, emit, run, status, runs, services, logs, locks, structured logging, readJson, and withLock. The SDK must not introduce a second runtime or workflow helpers."
  [recipes]="Implement recipes as generated normal files. Include starter recipes without external product names: file-watch tests, scheduled status script, generic event source service, event hook task, and explicit named lock example. Recipes must not create hidden daemon behavior."
)

slice_exists() {
  local needle="$1"
  local slice
  for slice in "${slices[@]}"; do
    [[ "$slice" == "$needle" ]] && return 0
  done
  return 1
}

if [[ -n "$from_slice" ]] && ! slice_exists "$from_slice"; then
  echo "unknown --from slice: $from_slice" >&2
  exit 2
fi

if [[ -n "$only_slice" ]] && ! slice_exists "$only_slice"; then
  echo "unknown --only slice: $only_slice" >&2
  exit 2
fi

build_prompt() {
  local slice="$1"
  local branch="${names[$slice]}"

  cat <<PROMPT
You are implementing the WhippleScript v0.3 $slice slice in this repository.

Primary checkout for integration:
$repo_root

Your slice branch/worktree name:
$branch

Read these first:
- spec/whipplescript-v0.3.md
- spec/implementation-plan-v0.3.md
- $state_file

Shared state:
- $state_file is the editable implementation ledger.
- Mark the $slice row in_progress when you begin.
- Keep your notes compact under "Slice Notes / $slice".
- Mark the row done only after your work is committed, merged to main, pushed, and checks have run.
- If blocked, mark blocked and record the exact blocker.

Boundary:
- Keep WhippleScript narrow: trigger, launch, monitor, record, supervise, reconcile runtime, inspect.
- Do not introduce workflow DAGs, durable promises, agent graphs, semantic retries, semantic dedupe, built-in external adapters, capabilities, Windows support, cloud coordination, or whip plan.
- You are not alone in the codebase. Do not revert work you did not make.

Ownership:
${ownership[$slice]}

Goal:
${goals[$slice]}

End-of-slice integration:
1. Run relevant tests/checks.
2. Commit your work atomically on branch/worktree $branch.
3. Push branch $branch.
4. From the primary checkout above, merge $branch into main and push main.
5. Update $state_file with status done and the commit/check summary.
6. Commit and push the state-file update if it was not already included.
7. If merge or tests fail, leave the slice blocked in $state_file with exact next steps.
PROMPT
}

selected_slices=()
started=0
for slice in "${slices[@]}"; do
  if [[ -n "$only_slice" && "$slice" != "$only_slice" ]]; then
    continue
  fi

  if [[ -n "$from_slice" ]]; then
    if [[ "$slice" == "$from_slice" ]]; then
      started=1
    fi
    [[ "$started" -eq 0 ]] && continue
  fi

  selected_slices+=("$slice")
done

echo "WhippleScript repo: $repo_root"
echo "State file: $state_file"
echo "Paseo command: $paseo_bin"
echo "Provider: $provider"
echo

if [[ "$run" -eq 0 ]]; then
  echo "Dry run. Pass --run to execute this sequential loop."
  echo
fi

for slice in "${selected_slices[@]}"; do
  name="${names[$slice]}"
  prompt="$(build_prompt "$slice")"
  cmd=("$paseo_bin" run --provider "$provider" --worktree "$name" --name "$name")

  if [[ -n "${PASEO_RUN_FLAGS:-}" ]]; then
    # shellcheck disable=SC2206
    extra_flags=($PASEO_RUN_FLAGS)
    cmd+=("${extra_flags[@]}")
  fi

  cmd+=("$prompt")

  if [[ "$run" -eq 0 ]]; then
    printf 'would run %-12s worktree=%s\n' "$slice" "$name"
    continue
  fi

  echo "starting $slice with $name"
  (
    cd "$repo_root"
    "${cmd[@]}"
  )

  echo "completed $slice; syncing local main before next slice"
  (
    cd "$repo_root"
    git checkout main
    git pull --ff-only origin main
  )
done

if [[ "$run" -eq 0 ]]; then
  exit 0
fi

echo
echo "sequential implementation loop complete"
echo "review state:"
echo "  $state_file"
