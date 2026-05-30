#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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

record_preflight() {
  local provider="$1"
  local phase="$2"
  local check="$3"
  local status="$4"
  local message="$5"

  printf '{"provider":"%s","phase":"%s","check":"%s","status":"%s","message":"%s"}\n' \
    "$(json_escape "$provider")" \
    "$(json_escape "$phase")" \
    "$(json_escape "$check")" \
    "$(json_escape "$status")" \
    "$(json_escape "$message")" \
    >>"$PREFLIGHT_REPORT"
}

if [[ "${WHIPPLESCRIPT_E2E_REAL_PROVIDERS:-}" != "1" ]]; then
  record_preflight all provider.config.missing gate skip "WHIPPLESCRIPT_E2E_REAL_PROVIDERS is not 1"
  echo "Skipping real-provider e2e checks."
  echo "Set WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 after configuring provider credentials."
  exit 0
fi

missing=0
SELECTED_PROVIDERS="${WHIPPLESCRIPT_REAL_PROVIDERS:-loft,baml}"
CURRENT_PROVIDER="all"

provider_enabled() {
  local provider="$1"
  [[ ",$SELECTED_PROVIDERS," == *",$provider,"* ]]
}

validate_selected_providers() {
  local provider
  local selected_any=0
  IFS=',' read -ra providers <<<"$SELECTED_PROVIDERS"
  for provider in "${providers[@]}"; do
    case "$provider" in
      loft | baml | codex)
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
    echo "WHIPPLESCRIPT_REAL_PROVIDERS must include loft, baml, codex, or a comma-separated subset" >&2
    missing=1
  fi
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

check_baml_endpoint() {
  local endpoint="$1"
  if [[ ! "$endpoint" =~ ^http://([^/:]+)(:([0-9]+))?(/.*)?$ ]]; then
    record_preflight baml provider.config.invalid endpoint fail "WHIPPLESCRIPT_BAML_TEST_ENDPOINT must be an http:// URL"
    echo "WHIPPLESCRIPT_BAML_TEST_ENDPOINT must be an http:// URL" >&2
    missing=1
    return
  fi

  local host="${BASH_REMATCH[1]}"
  local port="${BASH_REMATCH[3]:-80}"
  local path="${BASH_REMATCH[4]:-}"

  if ! timeout 3 bash -c "cat < /dev/null > /dev/tcp/$host/$port" 2>/dev/null; then
    record_preflight baml provider.launch.failed endpoint fail "could not connect to BAML endpoint"
    echo "could not connect to BAML endpoint $host:$port" >&2
    missing=1
    return
  fi
  record_preflight baml provider.launch.ok endpoint pass "BAML endpoint TCP check ok"
  echo "BAML endpoint TCP check ok: $host:$port"

  if [[ -n "${WHIPPLESCRIPT_BAML_HEALTH_PATH:-}" ]]; then
    if ! command -v curl >/dev/null 2>&1; then
      record_preflight baml adapter.resolve.failed command:curl fail "curl is required for WHIPPLESCRIPT_BAML_HEALTH_PATH"
      echo "missing required tool for WHIPPLESCRIPT_BAML_HEALTH_PATH: curl" >&2
      missing=1
      return
    fi
    local health_path="$WHIPPLESCRIPT_BAML_HEALTH_PATH"
    if [[ "$health_path" != /* ]]; then
      health_path="/$health_path"
    fi
    if ! curl --fail --silent --show-error --max-time 5 \
      "http://$host:$port$path$health_path" >/dev/null; then
      record_preflight baml provider.result.invalid health fail "BAML endpoint health check failed"
      missing=1
      return
    fi
    record_preflight baml provider.result.valid health pass "BAML endpoint health check ok"
    echo "BAML endpoint health check ok: $health_path"
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
validate_selected_providers

if provider_enabled loft; then
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

if provider_enabled baml; then
  CURRENT_PROVIDER="baml"
  if [[ "${WHIPPLESCRIPT_BAML_SKIP_CLI:-}" == "1" ]]; then
    record_preflight baml adapter.resolve.skip baml-cli skip "WHIPPLESCRIPT_BAML_SKIP_CLI=1"
    echo "Skipping BAML CLI probe because WHIPPLESCRIPT_BAML_SKIP_CLI=1"
  else
    require_any_command "baml-cli or baml" baml-cli baml || true
  fi
  require_env WHIPPLESCRIPT_BAML_TEST_ENDPOINT || true
  require_env WHIPPLESCRIPT_BAML_TEST_FUNCTION || true
  require_env WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON || true
  require_env WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE || true

  if [[ -n "${WHIPPLESCRIPT_BAML_TEST_ENDPOINT:-}" ]]; then
    check_baml_endpoint "$WHIPPLESCRIPT_BAML_TEST_ENDPOINT"
  fi
fi

if provider_enabled codex; then
  CURRENT_PROVIDER="codex"
  require_command codex || true
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

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-cli -- doctor
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-cli -- check \
  "$ROOT/examples/loft-worker-with-review.whip" \
  "$ROOT/examples/coerce-branch.whip"

if provider_enabled loft; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
    real_loft_show_smoke -- --nocapture
fi

if provider_enabled baml; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
    real_baml_coerce_endpoint_smoke -- --nocapture
fi

if provider_enabled codex; then
  "$ROOT/scripts/check-codex-message.sh"
fi

echo "Real-provider readiness checks passed."
echo "Provider-destructive flows remain manual until isolated Loft/BAML fixtures are approved."
