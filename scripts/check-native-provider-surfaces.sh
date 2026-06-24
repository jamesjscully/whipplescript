#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_NATIVE_PROVIDER_SURFACE_REPORT:-$ROOT/target/native-provider-surface.jsonl}"
CODEX_SCHEMA_DIR="${WHIPPLESCRIPT_CODEX_SCHEMA_DIR:-$ROOT/target/native-provider-surface/codex-schema}"
failures=0

mkdir -p "$(dirname "$REPORT")"
: >"$REPORT"

json_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  printf '%s' "$value"
}

record_check() {
  local provider="$1"
  local surface="$2"
  local check="$3"
  local status="$4"
  local message="$5"

  printf '{"provider":"%s","surface":"%s","check":"%s","status":"%s","message":"%s"}\n' \
    "$(json_escape "$provider")" \
    "$(json_escape "$surface")" \
    "$(json_escape "$check")" \
    "$(json_escape "$status")" \
    "$(json_escape "$message")" \
    >>"$REPORT"
}

require_command() {
  local provider="$1"
  local command="$2"

  if ! command -v "$command" >/dev/null 2>&1; then
    record_check "$provider" cli command fail "$command not found on PATH"
    failures=1
    return 1
  fi

  local version
  version="$("$command" --version 2>&1 | head -1 || true)"
  if [[ -z "$version" && "$command" == "pi" ]]; then
    version="$("$command" -v 2>&1 | head -1 || true)"
  fi
  if [[ -z "$version" ]]; then
    version="$(command -v "$command")"
  fi
  record_check "$provider" cli command pass "$version"
}

help_contains() {
  local provider="$1"
  local surface="$2"
  local check="$3"
  local needle="$4"
  shift 4
  local output

  if output="$("$@" --help 2>&1)" && grep -Fq -- "$needle" <<<"$output"; then
    record_check "$provider" "$surface" "$check" pass "help includes $needle"
    return
  fi

  record_check "$provider" "$surface" "$check" fail "help does not include $needle"
  failures=1
}

help_contains_optional() {
  local provider="$1"
  local surface="$2"
  local check="$3"
  local needle="$4"
  shift 4
  local output

  if output="$("$@" --help 2>&1)" && grep -Fq -- "$needle" <<<"$output"; then
    record_check "$provider" "$surface" "$check" pass "help includes $needle"
    return
  fi

  record_check "$provider" "$surface" "$check" skip "optional help does not include $needle"
}

schema_contains() {
  local check="$1"
  local needle="$2"

  if rg -q -- "$needle" "$CODEX_SCHEMA_DIR"; then
    record_check codex app-server-schema "$check" pass "schema includes $needle"
    return
  fi

  record_check codex app-server-schema "$check" fail "schema does not include $needle"
  failures=1
}

check_codex() {
  require_command codex codex || return
  help_contains codex app-server listen-transport "--listen" codex app-server
  help_contains codex app-server schema-generation "generate-json-schema" codex app-server
  help_contains codex mcp-server mcp-server "Start Codex as an MCP server" codex mcp-server

  rm -rf "$CODEX_SCHEMA_DIR"
  mkdir -p "$CODEX_SCHEMA_DIR"
  if codex app-server generate-json-schema --out "$CODEX_SCHEMA_DIR" --experimental >/dev/null 2>&1; then
    record_check codex app-server-schema generate pass "generated local app-server schema"
    schema_contains initialize '"initialize"'
    schema_contains thread-start '"thread/start"'
    schema_contains turn-start '"turn/start"'
    schema_contains turn-started '"turn/started"'
    schema_contains turn-completed '"turn/completed"'
    schema_contains turn-interrupt '"turn/interrupt"'
    schema_contains turn-diff-updated '"turn/diff/updated"'
    schema_contains approvals 'Approval'
  else
    record_check codex app-server-schema generate fail "could not generate local app-server schema"
    failures=1
  fi
}

check_claude() {
  require_command claude claude || return
  help_contains claude cli stream-json "--output-format" claude
  help_contains claude cli session-id "--session-id" claude
  help_contains claude cli allowed-tools "--allowedTools" claude
  help_contains claude cli permission-mode "--permission-mode" claude
  help_contains claude cli hook-events "--include-hook-events" claude
  help_contains_optional claude cli structured-output "--json-schema" claude

  local auth
  auth="$(claude auth status --json 2>/dev/null || true)"
  if grep -Fq '"loggedIn": true' <<<"$auth"; then
    record_check claude auth status pass "logged in; details redacted"
  else
    record_check claude auth status skip "not logged in or auth status unavailable"
  fi

  if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
    record_check claude auth api-key pass "ANTHROPIC_API_KEY is set"
  else
    record_check claude auth api-key skip "ANTHROPIC_API_KEY is not set"
  fi
}

check_pi() {
  require_command pi pi || return
  help_contains pi cli rpc-mode "--mode <mode>" pi
  help_contains pi cli extension-loading "--extension" pi
  help_contains pi cli session-selection "--session" pi
  help_contains pi cli tool-selection "--tools" pi
  help_contains pi cli provider-selection "--provider" pi
  help_contains pi cli model-selection "--model" pi

  if pi list >/dev/null 2>&1; then
    record_check pi extensions list pass "extension list command works"
  else
    record_check pi extensions list fail "extension list command failed"
    failures=1
  fi
}

check_codex
check_claude
check_pi

echo "Wrote native provider surface report: $REPORT" >&2
exit "$failures"
