#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/loft-fixtures-lib.sh
source "$ROOT/scripts/loft-fixtures-lib.sh"

PREFLIGHT_REPORT="${WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT:-$ROOT/target/real-provider-preflight.jsonl}"
mkdir -p "$(dirname "$PREFLIGHT_REPORT")"
: >"$PREFLIGHT_REPORT"

json_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  printf '%s' "$value"
}

redact_record_value() {
  perl -pe 's/(Authorization:\s*Bearer\s+)\S+/${1}[REDACTED]/ig; s/(Bearer\s+)[A-Za-z0-9._~+\/=-]{16,}/${1}[REDACTED]/g; s/sk-[A-Za-z0-9_-]{8,}/sk-REDACTED/g; s/((?:api[_-]?key|token|password|secret)["\x27\s]*[:=]["\x27\s]*)[^"\x27\s,}]+/${1}[REDACTED]/ig' <<<"$1"
}

record_preflight() {
  local provider="$1"
  local phase="$2"
  local check="$3"
  local status="$4"
  local message="$5"

  printf '{"provider":"%s","phase":"%s","check":"%s","status":"%s","message":"%s"}\n' \
    "$(json_escape "$(redact_record_value "$provider")")" \
    "$(json_escape "$(redact_record_value "$phase")")" \
    "$(json_escape "$(redact_record_value "$check")")" \
    "$(json_escape "$(redact_record_value "$status")")" \
    "$(json_escape "$(redact_record_value "$message")")" \
    >>"$PREFLIGHT_REPORT"
}

if [[ "${WHIPPLESCRIPT_E2E_REAL_PROVIDERS:-}" != "1" ]]; then
  record_preflight all provider.config.missing gate skip "WHIPPLESCRIPT_E2E_REAL_PROVIDERS is not 1"
  echo "Skipping real-provider e2e checks."
  echo "Set WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 after configuring provider credentials."
  exit 0
fi

missing=0
STRICT_NATIVE="${WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT:-0}"
NATIVE_SURFACE="${WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE:-0}"
if [[ "$STRICT_NATIVE" == "1" ]]; then
  SELECTED_PROVIDERS="${WHIPPLESCRIPT_REAL_PROVIDERS:-codex,claude,pi}"
else
  SELECTED_PROVIDERS="${WHIPPLESCRIPT_REAL_PROVIDERS:-loft,coerce}"
fi
CURRENT_PROVIDER="all"

provider_enabled() {
  local provider="$1"
  [[ ",$SELECTED_PROVIDERS," == *",$provider,"* ]]
}

provider_env_prefix() {
  local provider="$1"
  printf '%s' "$provider" | tr '[:lower:]-' '[:upper:]_'
}

destructive_provider_tests_enabled() {
  local provider="$1"
  local prefix
  prefix="$(provider_env_prefix "$provider")"
  local provider_flag="WHIPPLESCRIPT_${prefix}_DESTRUCTIVE_TESTS"
  [[ "${WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS:-0}" == "1" || "${!provider_flag:-0}" == "1" ]]
}

validate_selected_providers() {
  local provider
  local selected_any=0
  IFS=',' read -ra providers <<<"$SELECTED_PROVIDERS"
  for provider in "${providers[@]}"; do
    case "$provider" in
      loft | coerce | codex)
        if [[ "$STRICT_NATIVE" == "1" && ( "$provider" == "loft" || "$provider" == "coerce" ) ]]; then
          record_preflight "$provider" native.strict.failed provider fail "command-wrapper provider is not accepted in native strict mode"
          echo "native strict mode requires Codex, Claude, and Pi native providers; $provider is command-wrapper coverage" >&2
          selected_any=1
          missing=1
          continue
        fi
        record_preflight "$provider" adapter.resolve.selected provider pass "provider selected"
        selected_any=1
        ;;
      claude | pi)
        record_preflight "$provider" adapter.resolve.selected provider pass "provider selected"
        selected_any=1
        ;;
      "")
        ;;
      *)
        record_preflight "$provider" adapter.resolve.failed provider fail "unknown real provider selection"
        echo "unknown real provider selection: $provider" >&2
        missing=1
        ;;
    esac
  done

  if [[ "$selected_any" -ne 1 ]]; then
    record_preflight all adapter.resolve.failed provider fail "no supported real providers selected"
    echo "WHIPPLESCRIPT_REAL_PROVIDERS must include loft, coerce, codex, claude, pi, or a comma-separated subset" >&2
    missing=1
  fi
  if [[ "$STRICT_NATIVE" == "1" ]]; then
    local required
    for required in codex claude pi; do
      if ! provider_enabled "$required"; then
        record_preflight "$required" native.strict.failed provider fail "required native provider is not selected"
        echo "native strict mode requires provider selection: $required" >&2
        missing=1
      fi
    done
  fi
}

run_check_script() {
  local provider="$1"
  local phase="$2"
  local check="$3"
  local message="$4"
  shift 4

  echo "$message"
  if "$@"; then
    record_preflight "$provider" "$phase.ok" "$check" pass "$message passed"
    return 0
  fi
  record_preflight "$provider" "$phase.failed" "$check" fail "$message failed"
  missing=1
  return 1
}

check_native_provider_config_gate() {
  if [[ "$STRICT_NATIVE" != "1" ]]; then
    return
  fi
  if [[ -z "${WHIPPLESCRIPT_PROVIDER_CONFIGS:-}" && -z "${WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS:-}" ]]; then
    record_preflight all provider.config.missing native-provider-configs fail "WHIPPLESCRIPT_PROVIDER_CONFIGS is required in native strict mode"
    echo "WHIPPLESCRIPT_PROVIDER_CONFIGS is required in native strict mode" >&2
    missing=1
    return
  fi
  WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_STRICT=1 \
    "$ROOT/scripts/check-native-provider-configs.sh" || {
      record_preflight all provider.config.failed native-provider-configs fail "native provider config validation failed"
      missing=1
      return
    }
  record_preflight all provider.config.ok native-provider-configs pass "native provider config validation passed"
}

check_disposable_fixture_gate() {
  local provider="$1"
  if ! destructive_provider_tests_enabled "$provider"; then
    record_preflight "$provider" destructive.fixture.skip disposable-target skip "destructive provider tests are not enabled"
    return
  fi

  local prefix target_var ack_var target ack expected_ack
  prefix="$(provider_env_prefix "$provider")"
  target_var="WHIPPLESCRIPT_${prefix}_DISPOSABLE_TARGET"
  ack_var="WHIPPLESCRIPT_${prefix}_DISPOSABLE_ACK"
  target="${!target_var:-${WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET:-}}"
  ack="${!ack_var:-${WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK:-}}"
  expected_ack="I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE"

  if [[ -z "$target" ]]; then
    record_preflight "$provider" destructive.fixture.missing disposable-target fail "missing disposable target marker for destructive provider tests"
    echo "destructive $provider tests require $target_var or WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET" >&2
    missing=1
    return
  fi
  if [[ "$ack" != "$expected_ack" ]]; then
    record_preflight "$provider" destructive.fixture.missing disposable-ack fail "missing disposable target acknowledgement for destructive provider tests"
    echo "destructive $provider tests require $ack_var=$expected_ack or WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK=$expected_ack" >&2
    missing=1
    return
  fi

  record_preflight "$provider" destructive.fixture.ok disposable-target pass "disposable target marker acknowledged"
}

require_command() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    record_preflight "$CURRENT_PROVIDER" adapter.resolve.failed "command:$tool" fail "missing required tool"
    echo "missing required tool: $tool" >&2
    missing=1
    return 1
  fi
  record_preflight "$CURRENT_PROVIDER" adapter.resolve.ok "command:$tool" pass "required tool found"
  echo "$tool: $(command -v "$tool")"
  "$tool" --version 2>/dev/null | head -1 || true
}

require_any_command() {
  local label="$1"
  shift
  local tool
  for tool in "$@"; do
    if command -v "$tool" >/dev/null 2>&1; then
      record_preflight "$CURRENT_PROVIDER" adapter.resolve.ok "command:$tool" pass "$label found"
      echo "$label: $(command -v "$tool")"
      "$tool" --version 2>/dev/null | head -1 || true
      return 0
    fi
  done
  echo "missing required tool: $label ($*)" >&2
  record_preflight "$CURRENT_PROVIDER" adapter.resolve.failed "command:$label" fail "missing required tool"
  missing=1
  return 1
}

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    record_preflight "$CURRENT_PROVIDER" provider.config.missing "env:$name" fail "missing required environment variable"
    echo "missing required env: $name" >&2
    missing=1
    return 1
  fi
  record_preflight "$CURRENT_PROVIDER" provider.config.ok "env:$name" pass "required environment variable is set"
  echo "$name: set"
}

check_coerce_endpoint() {
  local endpoint="$1"
  if [[ ! "$endpoint" =~ ^http://([^/:]+)(:([0-9]+))?(/.*)?$ ]]; then
    record_preflight coerce provider.config.invalid endpoint fail "WHIPPLESCRIPT_COERCE_TEST_ENDPOINT must be an http:// URL"
    echo "WHIPPLESCRIPT_COERCE_TEST_ENDPOINT must be an http:// URL" >&2
    missing=1
    return
  fi

  local host="${BASH_REMATCH[1]}"
  local port="${BASH_REMATCH[3]:-80}"
  local path="${BASH_REMATCH[4]:-}"
  if [[ ! "$host" =~ ^[A-Za-z0-9._-]+$ || ! "$port" =~ ^[0-9]+$ ]]; then
    record_preflight coerce provider.config.invalid endpoint fail "WHIPPLESCRIPT_COERCE_TEST_ENDPOINT contains an invalid host or port"
    echo "WHIPPLESCRIPT_COERCE_TEST_ENDPOINT contains an invalid host or port" >&2
    missing=1
    return
  fi

  if ! command -v curl >/dev/null 2>&1; then
    record_preflight coerce adapter.resolve.failed command:curl fail "curl is required for coerce endpoint checks"
    echo "missing required tool for coerce endpoint checks: curl" >&2
    missing=1
    return
  fi

  local curl_args=(--silent --show-error --max-time 3 --output /dev/null)
  if [[ -n "${WHIPPLESCRIPT_COERCE_AUTH_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $WHIPPLESCRIPT_COERCE_AUTH_TOKEN")
  fi

  if ! curl "${curl_args[@]}" "http://$host:$port$path" >/dev/null; then
    record_preflight coerce provider.launch.failed endpoint fail "could not connect to coerce endpoint"
    echo "could not connect to coerce endpoint $host:$port" >&2
    missing=1
    return
  fi
  record_preflight coerce provider.launch.ok endpoint pass "coerce endpoint TCP check ok"
  echo "coerce endpoint TCP check ok: $host:$port"

  if [[ -n "${WHIPPLESCRIPT_COERCE_HEALTH_PATH:-}" ]]; then
    local health_path="$WHIPPLESCRIPT_COERCE_HEALTH_PATH"
    if [[ "$health_path" != /* ]]; then
      health_path="/$health_path"
    fi
    local health_curl_args=(--fail --silent --show-error --max-time 5)
    if [[ -n "${WHIPPLESCRIPT_COERCE_AUTH_TOKEN:-}" ]]; then
      health_curl_args+=(-H "Authorization: Bearer $WHIPPLESCRIPT_COERCE_AUTH_TOKEN")
    fi
    if ! curl "${health_curl_args[@]}" \
      "http://$host:$port$path$health_path" >/dev/null; then
      record_preflight coerce provider.result.invalid health fail "coerce endpoint health check failed"
      missing=1
      return
    fi
    record_preflight coerce provider.result.valid health pass "coerce endpoint health check ok"
    echo "coerce endpoint health check ok: $health_path"
  fi
}

check_loft_fixture_repo() {
  local repo="${WHIPPLESCRIPT_LOFT_REPO:-$ROOT/vendor/loft}"
  if [[ ! -d "$repo/.git" ]]; then
    record_preflight loft workspace.prepare.failed repo fail "Loft fixture repo not present"
    echo "Loft fixture repo not present at $repo"
    echo "Set WHIPPLESCRIPT_LOFT_REPO or add the Loft repo as vendor/loft when fixtures are tracked."
    missing=1
    return
  fi

  if ! "$ROOT/scripts/check-loft-source-repo.sh" "$repo" "Loft fixture repo"; then
    record_preflight loft workspace.prepare.failed repo fail "Loft fixture repo preflight failed"
    missing=1
  else
    record_preflight loft workspace.prepare.ok repo pass "Loft fixture repo preflight passed"
  fi
}

echo "Selected real providers: $SELECTED_PROVIDERS"
echo "Native strict mode: $STRICT_NATIVE"
validate_selected_providers
check_native_provider_config_gate
IFS=',' read -ra selected_provider_list <<<"$SELECTED_PROVIDERS"
for selected_provider in "${selected_provider_list[@]}"; do
  if [[ -n "$selected_provider" ]]; then
    check_disposable_fixture_gate "$selected_provider"
  fi
done
if [[ "${WHIPPLESCRIPT_REAL_PROVIDER_FIXTURE_GATE_ONLY:-0}" == "1" ]]; then
  if [[ "$missing" -ne 0 ]]; then
    exit 2
  fi
  echo "Real-provider disposable fixture gates passed."
  echo "Wrote real-provider preflight report: $PREFLIGHT_REPORT"
  exit 0
fi

if [[ "$STRICT_NATIVE" != "1" ]] && provider_enabled loft; then
  CURRENT_PROVIDER="loft"
  require_command "${WHIPPLESCRIPT_LOFT_CLI:-loft}" || true
  require_env WHIPPLESCRIPT_LOFT_TEST_ISSUE || true
  if [[ "${WHIPPLESCRIPT_LOFT_SKIP_REPO_PREFLIGHT:-}" == "1" ]]; then
    record_preflight loft workspace.prepare.skip repo skip "WHIPPLESCRIPT_LOFT_SKIP_REPO_PREFLIGHT=1"
    echo "Skipping Loft fixture repo preflight because WHIPPLESCRIPT_LOFT_SKIP_REPO_PREFLIGHT=1"
  else
    check_loft_fixture_repo
  fi
fi

if [[ "$STRICT_NATIVE" != "1" ]] && provider_enabled coerce; then
  CURRENT_PROVIDER="coerce"
  if [[ "${WHIPPLESCRIPT_COERCE_SKIP_CLI:-}" == "1" ]]; then
    record_preflight coerce adapter.resolve.skip coerce-cli skip "WHIPPLESCRIPT_COERCE_SKIP_CLI=1"
    echo "Skipping coerce CLI probe because WHIPPLESCRIPT_COERCE_SKIP_CLI=1"
  else
    require_any_command "coerce-cli or coerce" coerce-cli coerce || true
  fi
  require_env WHIPPLESCRIPT_COERCE_TEST_ENDPOINT || true
  require_env WHIPPLESCRIPT_COERCE_TEST_FUNCTION || true
  require_env WHIPPLESCRIPT_COERCE_TEST_ARGUMENTS_JSON || true
  require_env WHIPPLESCRIPT_COERCE_TEST_OUTPUT_TYPE || true

  if [[ -n "${WHIPPLESCRIPT_COERCE_TEST_ENDPOINT:-}" ]]; then
    check_coerce_endpoint "$WHIPPLESCRIPT_COERCE_TEST_ENDPOINT"
  fi
fi

if provider_enabled codex; then
  CURRENT_PROVIDER="codex"
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    run_check_script codex provider.surface codex-app-server-schema \
      "Checking Codex app-server native schema" \
      "$ROOT/scripts/check-codex-app-server-schema.sh" || true
    run_check_script codex provider.artifact codex-app-server-artifact \
      "Checking Codex app-server diff artifact capture" \
      "$ROOT/scripts/check-codex-app-server-artifact-smoke.sh" || true
    run_check_script codex provider.error codex-app-server-error \
      "Checking Codex app-server error response shape" \
      "$ROOT/scripts/check-codex-app-server-error-smoke.sh" || true
    run_check_script codex provider.workflow codex-native-workflow \
      "Checking source workflow to Codex native adapter bridge" \
      "$ROOT/scripts/check-codex-native-workflow-smoke.sh" || true
  else
    require_command codex || true
  fi
fi

if provider_enabled claude; then
  CURRENT_PROVIDER="claude"
  run_check_script claude provider.surface claude-agent-sdk-surface \
    "Checking Claude Agent SDK native surface" \
    "$ROOT/scripts/check-claude-agent-sdk-surface.sh" || true
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    run_check_script claude provider.artifact claude-agent-sdk-artifact \
      "Checking Claude Agent SDK artifact metadata capture" \
      "$ROOT/scripts/check-claude-agent-sdk-artifact-smoke.sh" || true
    run_check_script claude provider.error claude-agent-sdk-error \
      "Checking Claude Agent SDK error response shape" \
      "$ROOT/scripts/check-claude-agent-sdk-error-smoke.sh" || true
    run_check_script claude provider.workflow claude-native-workflow \
      "Checking source workflow to Claude native adapter bridge" \
      "$ROOT/scripts/check-claude-native-workflow-smoke.sh" || true
  fi
fi

if provider_enabled pi; then
  CURRENT_PROVIDER="pi"
  run_check_script pi provider.surface pi-rpc-surface \
    "Checking Pi RPC native surface" \
    "$ROOT/scripts/check-pi-rpc-surface.sh" || true
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    run_check_script pi provider.artifact pi-rpc-artifact \
      "Checking Pi RPC artifact metadata capture" \
      "$ROOT/scripts/check-pi-rpc-artifact-smoke.sh" || true
    run_check_script pi provider.error pi-rpc-error \
      "Checking Pi RPC error response shape" \
      "$ROOT/scripts/check-pi-rpc-error-smoke.sh" || true
    run_check_script pi provider.workflow pi-native-workflow \
      "Checking source workflow to Pi native adapter bridge" \
      "$ROOT/scripts/check-pi-native-workflow-smoke.sh" || true
  fi
fi
CURRENT_PROVIDER="all"

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi

if [[ "${WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_ONLY:-}" == "1" ]]; then
  echo "Real-provider preflight checks passed."
  echo "Wrote real-provider preflight report: $PREFLIGHT_REPORT"
  exit 0
fi

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript -- doctor
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript -- check \
  "$ROOT/examples/queue-worker-with-review.whip" \
  "$ROOT/examples/coerce-branch.whip"

if provider_enabled loft; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
    real_loft_show_smoke -- --nocapture
fi

if provider_enabled coerce; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
    real_coerce_endpoint_smoke -- --nocapture
fi

if provider_enabled codex; then
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    "$ROOT/scripts/check-codex-app-server-live-smoke.sh"
    "$ROOT/scripts/check-codex-app-server-interrupt-smoke.sh"
    "$ROOT/scripts/check-codex-app-server-artifact-smoke.sh"
    "$ROOT/scripts/check-codex-app-server-error-smoke.sh"
    "$ROOT/scripts/check-codex-native-workflow-smoke.sh"
  else
    "$ROOT/scripts/check-codex-message.sh"
  fi
fi

if provider_enabled claude; then
  "$ROOT/scripts/check-claude-agent-sdk-live-smoke.sh"
  "$ROOT/scripts/check-claude-agent-sdk-interrupt-smoke.sh"
  "$ROOT/scripts/check-claude-agent-sdk-artifact-smoke.sh"
  "$ROOT/scripts/check-claude-agent-sdk-error-smoke.sh"
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    "$ROOT/scripts/check-claude-native-workflow-smoke.sh"
  fi
fi

if provider_enabled pi; then
  "$ROOT/scripts/check-pi-rpc-interrupt-smoke.sh"
  "$ROOT/scripts/check-pi-rpc-artifact-smoke.sh"
  "$ROOT/scripts/check-pi-rpc-error-smoke.sh"
  if [[ "$STRICT_NATIVE" == "1" || "$NATIVE_SURFACE" == "1" ]]; then
    "$ROOT/scripts/check-pi-native-workflow-smoke.sh"
  fi
fi

echo "Real-provider readiness checks passed."
echo "Provider-destructive flows remain manual until isolated Loft/coerce fixtures are approved."
