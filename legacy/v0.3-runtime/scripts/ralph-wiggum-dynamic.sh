#!/usr/bin/env bash
set -euo pipefail

# Sequential Paseo implementation loop for Armature dynamic management.
# The shared editable plan state is docs/plans/dynamic-management-implementation-state.md.

usage() {
  cat <<'USAGE'
Usage: scripts/ralph-wiggum-dynamic.sh [--run] [--from SLICE] [--only SLICE] [--provider PROVIDER]

By default this is a dry run. Pass --run to start the sequential Paseo loop.
Each agent runs to completion before the next slice starts.

Slices:
  object-cli query-wait adhoc-run locks dynamic-services dynamic-tasks sdk-docs

Environment:
  PASEO_BIN          Paseo command to invoke. Default: paseo
  PASEO_PROVIDER     Provider passed to paseo run. Default: codex/gpt-5.5
  PASEO_RUN_FLAGS    Extra flags appended to every paseo run.
  PASEO_WAIT_TIMEOUT Wait timeout passed to paseo run. Default: 24h.

Examples:
  scripts/ralph-wiggum-dynamic.sh
  scripts/ralph-wiggum-dynamic.sh --run
  scripts/ralph-wiggum-dynamic.sh --run --from adhoc-run
  scripts/ralph-wiggum-dynamic.sh --run --only locks
  PASEO_WAIT_TIMEOUT=4h scripts/ralph-wiggum-dynamic.sh --run
USAGE
}

run=0
from_slice=""
only_slice=""
provider="${PASEO_PROVIDER:-codex/gpt-5.5}"
wait_timeout="${PASEO_WAIT_TIMEOUT:-24h}"

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
state_file="docs/plans/dynamic-management-implementation-state.md"
plan_file="docs/plans/dynamic-management-implementation-plan.md"

if [[ ! -f "$repo_root/spec/dynamic-management-interface.md" ]]; then
  echo "expected Armature repo root at $repo_root" >&2
  exit 1
fi

if [[ ! -f "$repo_root/$state_file" ]]; then
  echo "missing state file: $state_file" >&2
  exit 1
fi

if [[ ! -f "$repo_root/$plan_file" ]]; then
  echo "missing plan file: $plan_file" >&2
  exit 1
fi

if ! command -v "$paseo_bin" >/dev/null 2>&1; then
  echo "paseo command not found: $paseo_bin" >&2
  exit 1
fi

declare -a slices=(
  "object-cli"
  "query-wait"
  "adhoc-run"
  "locks"
  "dynamic-services"
  "dynamic-tasks"
  "sdk-docs"
)

declare -A names=(
  [object-cli]="armature-dynamic-object-cli"
  [query-wait]="armature-dynamic-query-wait"
  [adhoc-run]="armature-dynamic-adhoc-run"
  [locks]="armature-dynamic-locks"
  [dynamic-services]="armature-dynamic-services"
  [dynamic-tasks]="armature-dynamic-tasks"
  [sdk-docs]="armature-dynamic-sdk-docs"
)

declare -A ownership=(
  [object-cli]="canonical object-oriented CLI command groups, alias equivalence, command help, and README examples"
  [query-wait]="record filtering, wait commands, subscribe streams, store/runtime query support, and e2e observation coverage"
  [adhoc-run]="tracked ad hoc process execution through run start / exec, daemon protocol, run model/origin handling, env/cwd/payload/timeout support, logs, cancellation, and e2e coverage"
  [locks]="lock force-release, lock show/list filters, lock with, tokenless owning-run release if implemented, audit records/events, and lock recovery tests"
  [dynamic-services]="ephemeral dynamic service definitions, runtime registry, service inspection, reconciliation, lifecycle commands, and e2e coverage"
  [dynamic-tasks]="ephemeral dynamic task definitions, runtime registry, event/watch/schedule routing integration, task inspection, removal semantics, and e2e coverage"
  [sdk-docs]="TypeScript SDK alignment, README/spec examples, migration notes, and final cross-suite verification"
)

declare -A goals=(
  [object-cli]="Add canonical commands such as task list/run, service list/show, run list/show/logs/cancel, event list/show/emit, trigger list/show, and log show/tail/follow while preserving existing v0.3 aliases."
  [query-wait]="Make runtime records easy for agents to query and wait on: filters for event/trigger/run/lock lists, wait commands with timeouts, and subscribe streams as NDJSON observation APIs."
  [adhoc-run]="Let agents launch arbitrary finite commands as tracked runs without creating task definitions. Keep this daemon-mediated, logged, cancelable, correlated, and mechanically distinct from workflows."
  [locks]="Complete safe lock ergonomics and recovery: force-release with reason/audit, lock show, expired filtering, lock with, and release semantics that prevent stale holders from releasing newer leases."
  [dynamic-services]="Implement ephemeral dynamic service definitions with the same supervision/logging/reconciliation machinery as static services. Dynamic services must be inspectable and not rewrite user TOML."
  [dynamic-tasks]="Implement ephemeral dynamic task definitions for event/watch/schedule triggers. Dynamic tasks must be inspectable, removable, and routed through the same trigger/admission path as static tasks."
  [sdk-docs]="Expose the stabilized dynamic-management surface through a thin SDK and documentation. Do not add workflow helpers, agent graph helpers, semantic retry helpers, or a second runtime."
)

declare -A checks=(
  [object-cli]="cargo test -p armature-cli --bin armature && cargo test -p armature-cli --test e2e"
  [query-wait]="cargo test -p armature-cli --test e2e wait_and_subscribe_agent_flow && cargo test"
  [adhoc-run]="cargo test -p armature-cli --test e2e adhoc_run_is_tracked_and_cancelable && cargo test"
  [locks]="cargo test -p armature-cli --test e2e lock_recovery_and_with_lock && cargo test"
  [dynamic-services]="cargo test -p armature-cli --test e2e dynamic_service_lifecycle && cargo test"
  [dynamic-tasks]="cargo test -p armature-cli --test e2e dynamic_task_event_and_watch_lifecycle && cargo test"
  [sdk-docs]="npm test --workspace @armature/sdk && cargo test && cargo clippy --all-targets -- -D warnings"
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
You are implementing the Armature dynamic-management $slice slice in this repository.

Primary checkout for integration:
$repo_root

Your slice branch/worktree name:
$branch

Read these first:
- spec/dynamic-management-interface.md
- spec/armature-v0.3.md
- $plan_file
- $state_file

Shared state:
- $state_file is the editable implementation ledger.
- Mark the $slice row in_progress when you begin.
- Keep your notes compact under "Slice Notes / $slice".
- Mark the row done only after relevant checks have run and your work is integrated.
- If blocked, mark blocked and record the exact blocker.

Boundary:
- Keep Armature narrow: trigger, launch, monitor, record, supervise, reconcile runtime, inspect, lock, wait.
- Do not introduce workflow DAGs, durable promises, semantic retries, semantic dedupe, agent graphs, hidden workflow state, built-in domain adapters, or a second runtime.
- Dynamic definitions are runtime definitions, not workflow state.
- You are not alone in the codebase. Do not revert work you did not make.

Ownership:
${ownership[$slice]}

Goal:
${goals[$slice]}

Expected checks:
${checks[$slice]}

End-of-slice integration:
1. Run relevant tests/checks.
2. Commit your work atomically on branch/worktree $branch.
3. Push branch $branch if remote workflow is available.
4. Integrate into the primary checkout above.
5. Update $state_file with status done and the commit/check summary.
6. If merge or tests fail, leave the slice blocked in $state_file with exact next steps.
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

echo "Armature repo: $repo_root"
echo "State file: $state_file"
echo "Plan file: $plan_file"
echo "Paseo command: $paseo_bin"
echo "Provider: $provider"
echo "Wait timeout: $wait_timeout"
echo

if [[ "$run" -eq 0 ]]; then
  echo "Dry run. Pass --run to execute this sequential loop."
  echo
fi

for slice in "${selected_slices[@]}"; do
  name="${names[$slice]}"
  prompt="$(build_prompt "$slice")"
  cmd=(
    "$paseo_bin" run
    --provider "$provider"
    --mode full-access
    --wait-timeout "$wait_timeout"
    --worktree "$name"
    --name "$name"
  )

  if [[ -n "${PASEO_RUN_FLAGS:-}" ]]; then
    # shellcheck disable=SC2206
    extra_flags=($PASEO_RUN_FLAGS)
    cmd+=("${extra_flags[@]}")
  fi

  cmd+=("$prompt")

  if [[ "$run" -eq 0 ]]; then
    printf 'would run %-17s worktree=%s\n' "$slice" "$name"
    continue
  fi

  echo "starting $slice with $name"
  (
    cd "$repo_root"
    "${cmd[@]}"
  )

  echo "completed $slice; continuing with the current integration checkout"
done

if [[ "$run" -eq 0 ]]; then
  exit 0
fi

echo
echo "dynamic-management implementation loop complete"
echo "review state:"
echo "  $state_file"
