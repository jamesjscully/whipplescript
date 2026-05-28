#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLETREE_RELEASE_READINESS_REPORT:-$ROOT/target/release-readiness-report.md}"
FULL="${WHIPPLETREE_RELEASE_READINESS_FULL:-0}"
STRICT_EXTERNAL="${WHIPPLETREE_RELEASE_STRICT_EXTERNAL:-0}"
LOG_DIR="$(mktemp -d)"
overall_status=0
required_failed=0
external_failed=0

cleanup() {
  rm -rf "$LOG_DIR"
}
trap cleanup EXIT

run_check() {
  local kind="$1"
  local name="$2"
  local command="$3"
  local log="$LOG_DIR/${name//[^A-Za-z0-9_.-]/_}.log"
  local status

  echo "== $name"
  echo "\$ $command" >"$log"
  set +e
  bash -c "$command" >>"$log" 2>&1
  status=$?
  set -e
  cat "$log"

  CHECK_NAMES+=("$name")
  CHECK_KINDS+=("$kind")
  CHECK_COMMANDS+=("$command")
  CHECK_STATUSES+=("$status")
  CHECK_LOGS+=("$log")

  if [[ "$status" -ne 0 ]]; then
    case "$kind" in
      required)
        required_failed=1
        overall_status=1
        ;;
      external)
        external_failed=1
        if [[ "$STRICT_EXTERNAL" == "1" ]]; then
          overall_status=1
        fi
        ;;
    esac
  fi
}

CHECK_NAMES=()
CHECK_KINDS=()
CHECK_COMMANDS=()
CHECK_STATUSES=()
CHECK_LOGS=()

run_check required "shell syntax" "cd '$ROOT' && bash -n scripts/*.sh"
run_check required "format check" "cd '$ROOT' && cargo fmt --all -- --check"
run_check required "diff whitespace check" "cd '$ROOT' && git diff --check"
run_check required "Loft compatibility fixtures" "cd '$ROOT' && scripts/check-loft-fixtures.sh"
run_check required "Loft source patch export" \
  "cd '$ROOT' && tmp=\$(mktemp -d) && patch=\$(mktemp) && rm -f \"\$patch\" && git -C \"\$tmp\" init -q && mkdir -p \"\$tmp/spec\" && printf '# Loft v0.1 Specification\n' >\"\$tmp/spec/loft-v0.1.md\" && scripts/export-loft-source-patch.sh \"\$tmp\" \"\$patch\" && grep -q 'fixtures/whippletree/v0.1/manifest.json' \"\$patch\"; status=\$?; rm -rf \"\$tmp\" \"\$patch\"; exit \$status"
run_check required "Loft source repo preflight" \
  "cd '$ROOT' && tmp=\$(mktemp -d) && git -C \"\$tmp\" init -q && mkdir -p \"\$tmp/spec\" && printf '# Loft v0.1 Specification\n' >\"\$tmp/spec/loft-v0.1.md\" && scripts/stage-loft-fixtures.sh \"\$tmp\" >/dev/null && git -C \"\$tmp\" add spec/loft-v0.1.md fixtures/whippletree/v0.1 && git -C \"\$tmp\" -c user.name=Whippletree -c user.email=whippletree@example.invalid commit -q -m 'Add Loft spec fixtures' && scripts/check-loft-source-repo.sh \"\$tmp\"; status=\$?; rm -rf \"\$tmp\"; exit \$status"
run_check required "Loft handoff report" "cd '$ROOT' && scripts/loft-handoff-report.sh"
run_check required "real-provider smoke report" "cd '$ROOT' && scripts/check-real-providers-report.sh"

if [[ "$FULL" == "1" ]]; then
  run_check required "clippy" "cd '$ROOT' && cargo clippy --workspace --all-targets -- -D warnings"
  run_check required "workspace tests" "cd '$ROOT' && cargo test --workspace"
  run_check required "Maude models" "cd '$ROOT' && scripts/check-formal-models.sh"
  run_check required "TLA models" "cd '$ROOT' && scripts/check-tla-models.sh"
  run_check required "e2e suite" "cd '$ROOT' && scripts/check-e2e.sh"
fi

run_check external "strict Loft submodule fixtures" \
  "cd '$ROOT' && WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 scripts/check-loft-fixtures.sh"
run_check external "Loft submodule readiness" \
  "cd '$ROOT' && scripts/check-loft-submodule-readiness.sh"

mkdir -p "$(dirname "$REPORT")"
{
  echo "# Release Readiness Report"
  echo
  echo "- Generated: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  echo "- Full suite: $FULL"
  echo "- Strict external prerequisites: $STRICT_EXTERNAL"
  echo "- Required checks failed: $required_failed"
  echo "- External prerequisite checks failed: $external_failed"
  echo "- Exit code: $overall_status"
  echo
  echo "| Check | Kind | Exit |"
  echo "| --- | --- | --- |"
  for index in "${!CHECK_NAMES[@]}"; do
    echo "| ${CHECK_NAMES[$index]} | ${CHECK_KINDS[$index]} | ${CHECK_STATUSES[$index]} |"
  done
  echo
  for index in "${!CHECK_NAMES[@]}"; do
    echo "## ${CHECK_NAMES[$index]}"
    echo
    echo "- Kind: ${CHECK_KINDS[$index]}"
    echo "- Command: \`${CHECK_COMMANDS[$index]}\`"
    echo "- Exit: ${CHECK_STATUSES[$index]}"
    echo
    echo '```text'
    sed 's/```/` ` `/g' "${CHECK_LOGS[$index]}"
    echo '```'
    echo
  done
} >"$REPORT"

echo "Wrote release readiness report: $REPORT" >&2
exit "$overall_status"
