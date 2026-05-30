#!/usr/bin/env bash
set -euo pipefail

# Minimal orchestration supervisor.
#
# This script does not implement workflow logic. It only:
#   1. tells the director when worker/quality agent runs finish
#   2. nudges the director when the runtime appears idle
#
# The director agent owns planning and scheduling. The implementation ledger owns
# durable workflow state. WhippleScript owns run/event/log observation.

WORKSPACE="${WORKSPACE:-.}"
PLAN_FILE="${PLAN_FILE:-state/implementation-plan.json}"
STATE_DIR="${STATE_DIR:-.whipplescript/supervisor}"
POLL_SECONDS="${POLL_SECONDS:-15}"
IDLE_NUDGE_SECONDS="${IDLE_NUDGE_SECONDS:-120}"
RUN_LIMIT="${RUN_LIMIT:-50}"

# In un-tie this should be the existing "send message to thread" operation.
# Keep it behind one function so the policy/sandboxed host can provide the real
# implementation without changing the supervisor.
DIRECTOR_THREAD_ID="${DIRECTOR_THREAD_ID:-director}"

mkdir -p "$STATE_DIR"
SEEN_RUNS="$STATE_DIR/seen-runs"
LAST_IDLE_NUDGE="$STATE_DIR/last-idle-nudge"
touch "$SEEN_RUNS"

send_to_director() {
  local message="$1"

  if command -v untie >/dev/null 2>&1; then
    printf '%s\n' "$message" | untie thread send "$DIRECTOR_THREAD_ID" --stdin
    return
  fi

  # Fallback for local WhippleScript-only experiments. A task can listen on this
  # event and forward the payload to a real director thread.
  whip event emit director.message \
    --source supervisor \
    --json "$(jq -cn --arg message "$message" '{message:$message}')"
}

now_seconds() {
  date +%s
}

seen_run() {
  grep -qxF "$1" "$SEEN_RUNS"
}

mark_seen() {
  printf '%s\n' "$1" >> "$SEEN_RUNS"
}

terminal_runs_json() {
  whip --workspace "$WORKSPACE" --format json run list --limit "$RUN_LIMIT" |
    jq -c '
      .[]
      | select(.state == "exited" or .state == "failed")
      | select(.origin == "task" or .origin == "adhoc")
      | select(.name | test("worker|quality|agent|research|implement|review"))
    '
}

active_count() {
  whip --workspace "$WORKSPACE" --format json overview |
    jq '.active_runs | length'
}

unfinished_ledger_count() {
  jq '
    [.items[]
      | select(.status != "completed"
        and .status != "needs-human-review"
        and .status != "failed")]
    | length
  ' "$PLAN_FILE"
}

notify_completed_runs() {
  terminal_runs_json | while IFS= read -r run; do
    local run_id run_name run_state
    run_id="$(jq -r '.id' <<<"$run")"
    run_name="$(jq -r '.name' <<<"$run")"
    run_state="$(jq -r '.state' <<<"$run")"

    if seen_run "$run_id"; then
      continue
    fi

    mark_seen "$run_id"
    send_to_director "$(cat <<EOF
Agent run completed.

Run id: $run_id
Run name: $run_name
Run state: $run_state

Please inspect the run logs, update $PLAN_FILE, start the next quality check or
worker if appropriate, and create a human-review task if the result is ambiguous.
EOF
)"
  done
}

maybe_nudge_idle() {
  local active unfinished now last
  active="$(active_count)"
  unfinished="$(unfinished_ledger_count)"

  if [ "$active" -ne 0 ] || [ "$unfinished" -eq 0 ]; then
    return
  fi

  now="$(now_seconds)"
  last="0"
  if [ -f "$LAST_IDLE_NUDGE" ]; then
    last="$(cat "$LAST_IDLE_NUDGE")"
  fi

  if [ $((now - last)) -lt "$IDLE_NUDGE_SECONDS" ]; then
    return
  fi

  printf '%s\n' "$now" > "$LAST_IDLE_NUDGE"
  send_to_director "$(cat <<EOF
The implementation loop appears idle.

Active WhippleScript runs: $active
Unfinished ledger items: $unfinished
Ledger: $PLAN_FILE

Please inspect the ledger and runtime overview. If work is ready, start the next
bounded set of workers. If work is blocked, record the blocker or create a
human-review task.
EOF
)"
}

while true; do
  notify_completed_runs
  maybe_nudge_idle
  sleep "$POLL_SECONDS"
done
