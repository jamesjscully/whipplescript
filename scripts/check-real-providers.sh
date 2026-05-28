#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

if [[ "${WHIPPLETREE_E2E_REAL_PROVIDERS:-}" != "1" ]]; then
  echo "Skipping real-provider e2e checks."
  echo "Set WHIPPLETREE_E2E_REAL_PROVIDERS=1 after configuring provider credentials."
  exit 0
fi

missing=0
SELECTED_PROVIDERS="${WHIPPLETREE_REAL_PROVIDERS:-loft,baml}"

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
        selected_any=1
        ;;
      "")
        ;;
      *)
        echo "unknown real provider selection: $provider" >&2
        missing=1
        ;;
    esac
  done

  if [[ "$selected_any" -ne 1 ]]; then
    echo "WHIPPLETREE_REAL_PROVIDERS must include loft, baml, codex, or a comma-separated subset" >&2
    missing=1
  fi
}

require_command() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "missing required tool: $tool" >&2
    missing=1
    return 1
  fi
  echo "$tool: $(command -v "$tool")"
  "$tool" --version 2>/dev/null | head -1 || true
}

require_any_command() {
  local label="$1"
  shift
  local tool
  for tool in "$@"; do
    if command -v "$tool" >/dev/null 2>&1; then
      echo "$label: $(command -v "$tool")"
      "$tool" --version 2>/dev/null | head -1 || true
      return 0
    fi
  done
  echo "missing required tool: $label ($*)" >&2
  missing=1
  return 1
}

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "missing required env: $name" >&2
    missing=1
    return 1
  fi
  echo "$name: set"
}

check_baml_endpoint() {
  local endpoint="$1"
  if [[ ! "$endpoint" =~ ^http://([^/:]+)(:([0-9]+))?(/.*)?$ ]]; then
    echo "WHIPPLETREE_BAML_TEST_ENDPOINT must be an http:// URL" >&2
    missing=1
    return
  fi

  local host="${BASH_REMATCH[1]}"
  local port="${BASH_REMATCH[3]:-80}"
  local path="${BASH_REMATCH[4]:-}"

  if ! timeout 3 bash -c "cat < /dev/null > /dev/tcp/$host/$port" 2>/dev/null; then
    echo "could not connect to BAML endpoint $host:$port" >&2
    missing=1
    return
  fi
  echo "BAML endpoint TCP check ok: $host:$port"

  if [[ -n "${WHIPPLETREE_BAML_HEALTH_PATH:-}" ]]; then
    if ! command -v curl >/dev/null 2>&1; then
      echo "missing required tool for WHIPPLETREE_BAML_HEALTH_PATH: curl" >&2
      missing=1
      return
    fi
    local health_path="$WHIPPLETREE_BAML_HEALTH_PATH"
    if [[ "$health_path" != /* ]]; then
      health_path="/$health_path"
    fi
    curl --fail --silent --show-error --max-time 5 \
      "http://$host:$port$path$health_path" >/dev/null
    echo "BAML endpoint health check ok: $health_path"
  fi
}

check_loft_fixture_repo() {
  local repo="${WHIPPLETREE_LOFT_REPO:-$ROOT/vendor/loft}"
  if [[ ! -d "$repo/.git" ]]; then
    echo "Loft fixture repo not present at $repo"
    echo "Set WHIPPLETREE_LOFT_REPO or add the Loft repo as vendor/loft when fixtures are tracked."
    missing=1
    return
  fi

  if ! "$ROOT/scripts/check-loft-source-repo.sh" "$repo" "Loft fixture repo"; then
    missing=1
  fi
}

echo "Selected real providers: $SELECTED_PROVIDERS"
validate_selected_providers

if provider_enabled loft; then
  require_command "${WHIPPLETREE_LOFT_CLI:-loft}" || true
  require_env WHIPPLETREE_LOFT_TEST_ISSUE || true
  if [[ "${WHIPPLETREE_LOFT_SKIP_REPO_PREFLIGHT:-}" == "1" ]]; then
    echo "Skipping Loft fixture repo preflight because WHIPPLETREE_LOFT_SKIP_REPO_PREFLIGHT=1"
  else
    check_loft_fixture_repo
  fi
fi

if provider_enabled baml; then
  if [[ "${WHIPPLETREE_BAML_SKIP_CLI:-}" == "1" ]]; then
    echo "Skipping BAML CLI probe because WHIPPLETREE_BAML_SKIP_CLI=1"
  else
    require_any_command "baml-cli or baml" baml-cli baml || true
  fi
  require_env WHIPPLETREE_BAML_TEST_ENDPOINT || true
  require_env WHIPPLETREE_BAML_TEST_FUNCTION || true
  require_env WHIPPLETREE_BAML_TEST_ARGUMENTS_JSON || true
  require_env WHIPPLETREE_BAML_TEST_OUTPUT_TYPE || true

  if [[ -n "${WHIPPLETREE_BAML_TEST_ENDPOINT:-}" ]]; then
    check_baml_endpoint "$WHIPPLETREE_BAML_TEST_ENDPOINT"
  fi
fi

if provider_enabled codex; then
  require_command codex || true
fi

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-cli -- doctor
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-cli -- check \
  "$ROOT/examples/loft-worker-with-review.whip" \
  "$ROOT/examples/coerce-branch.whip"

if provider_enabled loft; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-kernel \
    real_loft_show_smoke -- --nocapture
fi

if provider_enabled baml; then
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-kernel \
    real_baml_coerce_endpoint_smoke -- --nocapture
fi

if provider_enabled codex; then
  "$ROOT/scripts/check-codex-message.sh"
fi

echo "Real-provider readiness checks passed."
echo "Provider-destructive flows remain manual until isolated Loft/BAML fixtures are approved."
