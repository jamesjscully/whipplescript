#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_RELEASE_READINESS_REPORT:-$ROOT/target/release-readiness-report.md}"
FULL="${WHIPPLESCRIPT_RELEASE_READINESS_FULL:-0}"
STRICT_EXTERNAL="${WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL:-0}"
LOG_DIR="$(mktemp -d)"
overall_status=0
required_failed=0
external_failed=0

# shellcheck disable=SC2329
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
run_check required "tracker discipline" "cd '$ROOT' && scripts/check-trackers.sh"
run_check required "docs snippets" "cd '$ROOT' && scripts/check-docs-snippets.sh"
run_check required "IR goldens" "cd '$ROOT' && scripts/regen-ir-goldens.sh --check"
run_check required "docs site" "cd '$ROOT' && scripts/check-docs-site.sh"
run_check required "report schemas" "cd '$ROOT' && scripts/check-report-schemas.sh"
run_check required "artifact admission differential" "cd '$ROOT' && scripts/check-artifact-admission-differential.sh"
run_check required "format check" "cd '$ROOT' && cargo fmt --all -- --check"
run_check required "diff whitespace check" "cd '$ROOT' && git diff --check"
run_check required "Loft compatibility fixtures" "cd '$ROOT' && scripts/check-loft-fixtures.sh"
run_check required "Loft source patch export" \
  "cd '$ROOT' && tmp=\$(mktemp -d) && patch=\$(mktemp) && rm -f \"\$patch\" && git -C \"\$tmp\" init -q && mkdir -p \"\$tmp/spec\" && printf '# Loft v0.1 Specification\n' >\"\$tmp/spec/loft-v0.1.md\" && scripts/export-loft-source-patch.sh \"\$tmp\" \"\$patch\" && grep -q 'fixtures/whipplescript/v0.1/manifest.json' \"\$patch\"; status=\$?; rm -rf \"\$tmp\" \"\$patch\"; exit \$status"
run_check required "Loft source repo preflight" \
  "cd '$ROOT' && tmp=\$(mktemp -d) && git -C \"\$tmp\" init -q && mkdir -p \"\$tmp/spec\" && printf '# Loft v0.1 Specification\n' >\"\$tmp/spec/loft-v0.1.md\" && scripts/stage-loft-fixtures.sh \"\$tmp\" >/dev/null && git -C \"\$tmp\" add spec/loft-v0.1.md fixtures/whipplescript/v0.1 && git -C \"\$tmp\" -c user.name=WhippleScript -c user.email=whipplescript@example.invalid commit -q -m 'Add Loft spec fixtures' && scripts/check-loft-source-repo.sh \"\$tmp\"; status=\$?; rm -rf \"\$tmp\"; exit \$status"
run_check required "Loft handoff report" "cd '$ROOT' && scripts/loft-handoff-report.sh"
run_check required "real-provider smoke report" "cd '$ROOT' && scripts/check-real-providers-report.sh"
run_check required "real-provider report redaction" "cd '$ROOT' && scripts/check-real-provider-report-redaction.sh"
run_check required "real-provider destructive fixture gate" "cd '$ROOT' && scripts/check-real-provider-destructive-gate.sh"
run_check required "native provider contract" "cd '$ROOT' && scripts/check-native-provider-contract.sh"
run_check required "Codex native adapter" "cd '$ROOT' && scripts/check-codex-native-adapter.sh"
run_check required "Claude native adapter" "cd '$ROOT' && scripts/check-claude-native-adapter.sh"
run_check required "Pi native adapter" "cd '$ROOT' && scripts/check-pi-native-adapter.sh"
run_check required "native provider policy denials" "cd '$ROOT' && scripts/check-native-provider-policy-denials.sh"
run_check required "control-plane driver" "cd '$ROOT' && scripts/check-control-plane-driver.sh"
run_check required "workspace records" "cd '$ROOT' && scripts/check-workspace-records.sh"
run_check required "provider scheduling and capacity" "cd '$ROOT' && scripts/check-provider-scheduling-capacity.sh"
run_check required "expression provider routing" "cd '$ROOT' && scripts/check-expression-provider-routing.sh"
run_check required "operator incident UX" "cd '$ROOT' && scripts/check-operator-incident-ux.sh"
run_check required "cancellation policy matrix" "cd '$ROOT' && scripts/check-cancellation-policy-matrix.sh"
run_check required "store replay conformance" "cd '$ROOT' && scripts/check-store-replay-conformance.sh"
run_check required "provider doctor posture" "cd '$ROOT' && cargo run --quiet -p whipplescript -- --json doctor --providers >/dev/null"
run_check required "artifact metadata redaction" \
  "cd '$ROOT' && cargo test -p whipplescript --test control_plane artifacts_command_lists_metadata_without_raw_content"

if [[ "$FULL" == "1" ]]; then
  run_check required "clippy" "cd '$ROOT' && cargo clippy --workspace --all-targets -- -D warnings"
  run_check required "workspace tests" "cd '$ROOT' && cargo test --workspace"
  run_check required "Maude models" "cd '$ROOT' && scripts/check-formal-models.sh"
  run_check required "TLA models" "cd '$ROOT' && scripts/check-tla-models.sh"
  run_check required "e2e suite" "cd '$ROOT' && scripts/check-e2e.sh"
fi

run_check external "strict Loft submodule fixtures" \
  "cd '$ROOT' && WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 scripts/check-loft-fixtures.sh"
run_check external "Loft submodule readiness" \
  "cd '$ROOT' && scripts/check-loft-submodule-readiness.sh"
run_check external "native provider surface probe" \
  "cd '$ROOT' && scripts/check-native-provider-surfaces.sh"
run_check external "Codex app-server schema pin" \
  "cd '$ROOT' && scripts/check-codex-app-server-schema.sh"
run_check external "native provider endpoint health" \
  "cd '$ROOT' && WHIPPLESCRIPT_NATIVE_PROVIDER_HEALTH_LIVE='$STRICT_EXTERNAL' scripts/check-native-provider-endpoint-health.sh"
run_check external "Codex app-server artifact smoke" \
  "cd '$ROOT' && WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE='$STRICT_EXTERNAL' scripts/check-codex-app-server-artifact-smoke.sh"
run_check external "Codex app-server error smoke" \
  "cd '$ROOT' && WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE='$STRICT_EXTERNAL' scripts/check-codex-app-server-error-smoke.sh"
run_check external "Codex native workflow smoke" \
  "cd '$ROOT' && WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE='$STRICT_EXTERNAL' scripts/check-codex-native-workflow-smoke.sh"
run_check external "Claude Agent SDK surface" \
  "cd '$ROOT' && scripts/check-claude-agent-sdk-surface.sh && node --check scripts/claude-agent-sdk-sidecar.mjs && scripts/check-claude-agent-sdk-live-smoke.sh && scripts/check-claude-agent-sdk-interrupt-smoke.sh && WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE='$STRICT_EXTERNAL' scripts/check-claude-agent-sdk-artifact-smoke.sh && WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE='$STRICT_EXTERNAL' scripts/check-claude-agent-sdk-error-smoke.sh"
run_check external "Claude native workflow smoke" \
  "cd '$ROOT' && WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE='$STRICT_EXTERNAL' scripts/check-claude-native-workflow-smoke.sh"
# Pi native-provider validation is DEFERRED from the v0.2 native gate (Jack,
# 2026-07-05): with the owned/native harness the standalone Pi provider has no
# remaining point for now. The Pi adapter + smokes stay in the tree; re-enable
# these checks if/when Pi native support is revived.
# run_check external "Pi RPC surface" \
#   "cd '$ROOT' && scripts/check-pi-rpc-surface.sh && WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE='$STRICT_EXTERNAL' scripts/check-pi-rpc-interrupt-smoke.sh && WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE='$STRICT_EXTERNAL' scripts/check-pi-rpc-artifact-smoke.sh && WHIPPLESCRIPT_PI_RPC_ERROR_LIVE='$STRICT_EXTERNAL' scripts/check-pi-rpc-error-smoke.sh"
# run_check external "Pi native workflow smoke" \
#   "cd '$ROOT' && WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE='$STRICT_EXTERNAL' scripts/check-pi-native-workflow-smoke.sh"
run_check external "native provider config validation" \
  "cd '$ROOT' && WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_STRICT='$STRICT_EXTERNAL' WHIPPLESCRIPT_PROVIDER_CONFIGS=\"\${WHIPPLESCRIPT_PROVIDER_CONFIGS:-examples/provider-configs/native/native.example.json}\" scripts/check-native-provider-configs.sh"

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
    # shellcheck disable=SC2016
    sed 's/```/` ` `/g' "${CHECK_LOGS[$index]}"
    echo '```'
    echo
  done
} >"$REPORT"

echo "Wrote release readiness report: $REPORT" >&2
exit "$overall_status"
