#!/usr/bin/env bash
# Tracker discipline gate. Enforces the single-source-of-truth registry in
# spec/TRACKERS.md so trackers cannot go stale, orphaned, or duplicated silently.
#
# The model (see spec/TRACKERS.md header): a tracker holds only OPEN INTENT;
# reality lives in code + git + gates. Every tracker file must be registered
# exactly once with a scope and a status. This script is the forcing function.
#
# HARD failures (exit 1) — structural, non-retroactive, cheap:
#   - a discovered tracker file not listed in the registry (register it);
#   - a registry row whose file does not exist;
#   - a registry status that is not active | closed | archived;
#   - a file listed twice.
# WARN (exit 0, printed) — staleness signals, the triage worklist:
#   - an `active` tracker whose checkboxes are ALL done (mark it closed);
#   - a `closed` tracker that still has open `[ ]` items (resolve or reopen);
#   - an `archived` tracker still living outside spec/archive/ (informational).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
REGISTRY="spec/TRACKERS.md"

fail=0
warn=0
err() {
  echo "TRACKER GATE (hard): $*" >&2
  fail=1
}
note() {
  echo "tracker gate (warn): $*" >&2
  warn=$((warn + 1))
}

if [[ ! -f "$REGISTRY" ]]; then
  echo "TRACKER GATE (hard): missing registry $REGISTRY" >&2
  exit 1
fi

# --- discovery: what the repo considers a work tracker needing registration ---
# Files whose name marks them a tracker or plan, minus design ADRs that merely
# contain the word, minus anything already parked under spec/archive/.
# Read the FILESYSTEM (not `git ls-files`) so a brand-new, not-yet-committed
# tracker is caught too — the gate must force registration before it can land.
mapfile -t discovered < <(
  find spec -type f \( -name '*tracker*.md' -o -name '*-plan.md' \) \
    | grep -v '^spec/archive/' \
    | grep -v 'spec/decision-records/0002-work-tracker-package.md' \
    | grep -v '^spec/std-tracker.md$' \
    | sort -u
)

# --- parse the registry table: rows are `| `path` | status | scope | date |` ---
declare -A reg_status
declare -A reg_seen
while IFS= read -r line; do
  # a data row starts with `| ` and its first cell is a backticked `.md` path
  # (the `.md` guard excludes the vocabulary/lifecycle tables above the registry)
  [[ "$line" =~ ^\|[[:space:]]*\`([^\`]+\.md)\`[[:space:]]*\|[[:space:]]*([a-z]+)[[:space:]]*\| ]] || continue
  path="${BASH_REMATCH[1]}"
  status="${BASH_REMATCH[2]}"
  if [[ -n "${reg_seen[$path]:-}" ]]; then
    err "duplicate registry row for $path"
    continue
  fi
  reg_seen["$path"]=1
  reg_status["$path"]="$status"

  case "$status" in
    active | closed | archived) ;;
    *) err "$path has invalid status '$status' (use active | closed | archived)" ;;
  esac

  if [[ ! -f "$path" ]]; then
    err "registry lists $path but the file does not exist"
    continue
  fi
done <"$REGISTRY"

# --- every discovered tracker must be registered ---
for f in "${discovered[@]}"; do
  if [[ -z "${reg_seen[$f]:-}" ]]; then
    err "$f is a tracker but is not registered in $REGISTRY (add a row: status + one-line scope)"
  fi
done

# Count checkboxes of a given mark, skipping any "Legend"/"Status Legend" section
# (its `[ ]`/`[x]`/`[~]` entries are keys, not work items — a common false positive).
# $1 = file, $2 = a single char class for the mark (' ', 'x', '~').
count_marks() {
  awk -v mark="$2" '
    /^#{1,6} / { inlegend = (tolower($0) ~ /legend/) ? 1 : 0; next }
    inlegend { next }
    {
      pat = "^[ \t]*[-*] \\[" mark "\\]"
      if ($0 ~ pat) c++
    }
    END { print c + 0 }
  ' "$1"
}

# --- staleness warnings, only where the file actually uses checkboxes ---
for path in "${!reg_status[@]}"; do
  [[ -f "$path" ]] || continue
  status="${reg_status[$path]}"
  open=$(count_marks "$path" ' ')
  part=$(count_marks "$path" '~')
  done_=$(count_marks "$path" '[xX]')

  case "$status" in
    active)
      # has checkboxes, none open or in-progress -> it is done; close it.
      if [[ "$open" -eq 0 && "$part" -eq 0 && "$done_" -gt 0 ]]; then
        note "$path is 'active' but all $done_ checkboxes are done — mark it 'closed'"
      fi
      ;;
    closed)
      if [[ "$open" -gt 0 ]]; then
        note "$path is 'closed' but has $open open '[ ]' item(s) — resolve them, convert to '[~]' with a why, or reopen"
      fi
      ;;
    archived)
      # an archived tracker is intentionally parked; kept in place when it has
      # inbound links, moved under spec/archive/ otherwise. Either is fine — no warn.
      :
      ;;
  esac
done

if [[ "$warn" -gt 0 ]]; then
  echo "tracker gate: $warn warning(s) — the triage worklist (non-blocking)" >&2
fi
if [[ "$fail" -ne 0 ]]; then
  echo "TRACKER GATE: FAILED — registry is out of sync with the repo" >&2
  exit 1
fi
echo "tracker gate: registry in sync (${#reg_seen[@]} trackers registered, ${#discovered[@]} discovered)"
