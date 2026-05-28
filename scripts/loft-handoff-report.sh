#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_LOFT_REPO="../loft"
LOFT_REPO="${1:-${WHIPPLETREE_LOFT_REPO:-$DEFAULT_LOFT_REPO}}"
REPORT="${2:-${WHIPPLETREE_LOFT_HANDOFF_REPORT:-$ROOT/target/loft-handoff-report.md}}"
LOG_DIR="$(mktemp -d)"
printf -v LOFT_REPO_Q "%q" "$LOFT_REPO"
printf -v ROOT_Q "%q" "$ROOT"

cleanup() {
  rm -rf "$LOG_DIR"
}
trap cleanup EXIT

run_probe() {
  local name="$1"
  local command="$2"
  local log="$LOG_DIR/${name//[^A-Za-z0-9_.-]/_}.log"
  local status

  echo "\$ $command" >"$log"
  set +e
  bash -c "$command" >>"$log" 2>&1
  status=$?
  set -e

  PROBE_NAMES+=("$name")
  PROBE_COMMANDS+=("$command")
  PROBE_STATUSES+=("$status")
  PROBE_LOGS+=("$log")
}

PROBE_NAMES=()
PROBE_COMMANDS=()
PROBE_STATUSES=()
PROBE_LOGS=()

run_probe "Loft repo status" "if [[ -d $LOFT_REPO_Q/.git ]]; then git -C $LOFT_REPO_Q status --short; else echo 'missing local Loft git repo: $LOFT_REPO'; exit 2; fi"
run_probe "Loft source preflight" "cd $ROOT_Q && scripts/check-loft-source-repo.sh $LOFT_REPO_Q"
run_probe "Whippletree submodule readiness" "cd $ROOT_Q && scripts/check-loft-submodule-readiness.sh"
run_probe "Strict submodule fixture conformance" "cd $ROOT_Q && WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 scripts/check-loft-fixtures.sh"

mkdir -p "$(dirname "$REPORT")"
{
  echo "# Loft Handoff Report"
  echo
  echo "- Generated: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  echo "- Loft repo: $LOFT_REPO"
  echo "- Whippletree repo: $ROOT"
  echo
  echo "## Current State"
  echo
  echo "| Probe | Exit |"
  echo "| --- | --- |"
  for index in "${!PROBE_NAMES[@]}"; do
    echo "| ${PROBE_NAMES[$index]} | ${PROBE_STATUSES[$index]} |"
  done
  echo
  echo "## Next Commands"
  echo
  echo '```sh'
  echo "# In Whippletree: export a Loft-side patch for review"
  echo "scripts/export-loft-source-patch.sh $LOFT_REPO_Q"
  echo
  echo "# In Loft: apply/review that patch, then commit spec and fixtures"
  echo "git -C $LOFT_REPO_Q status --short"
  echo "git -C $LOFT_REPO_Q add spec/loft-v0.1.md fixtures/whippletree/v0.1"
  echo "git -C $LOFT_REPO_Q commit -m 'Add Whippletree conformance fixtures'"
  echo
  echo "# In Whippletree: add and verify Loft as source-of-truth submodule"
  echo "scripts/add-loft-submodule.sh $LOFT_REPO_Q vendor/loft"
  echo "scripts/check-loft-submodule-readiness.sh"
  echo "WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 scripts/check-loft-fixtures.sh"
  echo
  echo "# Optional, after provider credentials/tools are configured"
  echo "WHIPPLETREE_E2E_REAL_PROVIDERS=1 WHIPPLETREE_REAL_PROVIDERS=loft scripts/check-real-providers-report.sh"
  echo '```'
  echo
  for index in "${!PROBE_NAMES[@]}"; do
    echo "## ${PROBE_NAMES[$index]}"
    echo
    echo "- Command: \`${PROBE_COMMANDS[$index]}\`"
    echo "- Exit: ${PROBE_STATUSES[$index]}"
    echo
    echo '```text'
    sed 's/```/` ` `/g' "${PROBE_LOGS[$index]}"
    echo '```'
    echo
  done
} >"$REPORT"

cat "$REPORT"
echo "Wrote Loft handoff report: $REPORT" >&2
