#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_REAL_PROVIDER_REPORT:-$ROOT/target/real-provider-smoke-report.md}"
PREFLIGHT_REPORT="${WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT:-$ROOT/target/real-provider-preflight.jsonl}"
PROVIDER_REPORT_DIR="${WHIPPLESCRIPT_REAL_PROVIDER_REPORT_DIR:-$ROOT/target/real-provider-reports}"
OUTPUT="$(mktemp)"

# shellcheck disable=SC2329
cleanup() {
  rm -f "$OUTPUT"
}
trap cleanup EXIT

env_state() {
  local name="$1"
  if [[ -n "${!name:-}" ]]; then
    echo "set"
  else
    echo "unset"
  fi
}

redact_output() {
  perl -pe 's/(Authorization:\s*Bearer\s+)\S+/${1}[REDACTED]/ig; s/(Bearer\s+)[A-Za-z0-9._~+\/=-]{16,}/${1}[REDACTED]/g; s/sk-[A-Za-z0-9_-]{8,}/sk-REDACTED/g; s/((?:api[_-]?key|token|password|secret)["\x27\s]*[:=]["\x27\s]*)[^"\x27\s,}]+/${1}[REDACTED]/ig'
}

redact_inline() {
  printf '%s' "$1" | redact_output
}

started_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
strict_native="${WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT:-0}"
if [[ "$strict_native" == "1" ]]; then
  selected_providers="${WHIPPLESCRIPT_REAL_PROVIDERS:-codex,claude,pi}"
else
  selected_providers="${WHIPPLESCRIPT_REAL_PROVIDERS:-loft,baml}"
fi

set +e
WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT="$PREFLIGHT_REPORT" \
  "$ROOT/scripts/check-real-providers.sh" >"$OUTPUT" 2>&1
status=$?
set -e

finished_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
mkdir -p "$(dirname "$REPORT")"
mkdir -p "$PROVIDER_REPORT_DIR"

node --input-type=module - "$PREFLIGHT_REPORT" "$PROVIDER_REPORT_DIR" "$REPORT" "$started_at" "$finished_at" "$status" <<'NODE'
import fs from "node:fs";
import path from "node:path";

const [preflightPath, reportDir, smokeReportPath, startedAt, finishedAt, exitCode] = process.argv.slice(2);
const envNames = [
  "WHIPPLESCRIPT_E2E_REAL_PROVIDERS",
  "WHIPPLESCRIPT_REAL_PROVIDERS",
  "WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT",
  "WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE",
  "WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS",
  "WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK",
  "WHIPPLESCRIPT_LOFT_TEST_ISSUE",
  "WHIPPLESCRIPT_LOFT_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_LOFT_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_LOFT_DISPOSABLE_ACK",
  "WHIPPLESCRIPT_BAML_TEST_ENDPOINT",
  "WHIPPLESCRIPT_BAML_TEST_FUNCTION",
  "WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON",
  "WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE",
  "WHIPPLESCRIPT_BAML_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_BAML_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_BAML_DISPOSABLE_ACK",
  "WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK",
  "WHIPPLESCRIPT_CLAUDE_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK",
  "WHIPPLESCRIPT_PI_DESTRUCTIVE_TESTS",
  "WHIPPLESCRIPT_PI_DISPOSABLE_TARGET",
  "WHIPPLESCRIPT_PI_DISPOSABLE_ACK",
];
const providerEvidenceRefs = {
  all: [
    preflightPath,
    smokeReportPath,
    process.env.WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_REPORT || "target/native-provider-config-validation.json",
  ],
  codex: [
    process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE_REPORT || "target/codex-app-server-live-smoke.json",
    process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_INTERRUPT_REPORT || "target/codex-app-server-interrupt-smoke.json",
    process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_REPORT || "target/codex-app-server-artifact-smoke.json",
    process.env.WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_REPORT || "target/codex-native-workflow-smoke.json",
    "spec/codex-app-server-schema.pin.json",
  ],
  claude: [
    process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE_REPORT || "target/claude-agent-sdk-live-smoke.json",
    process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_INTERRUPT_REPORT || "target/claude-agent-sdk-interrupt-smoke.json",
    process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_REPORT || "target/claude-agent-sdk-artifact-smoke.json",
    process.env.WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_REPORT || "target/claude-native-workflow-smoke.json",
    "target/claude-agent-sdk-surface.json",
  ],
  pi: [
    process.env.WHIPPLESCRIPT_PI_RPC_INTERRUPT_REPORT || "target/pi-rpc-interrupt-smoke.json",
    process.env.WHIPPLESCRIPT_PI_RPC_ARTIFACT_REPORT || "target/pi-rpc-artifact-smoke.json",
    process.env.WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_REPORT || "target/pi-native-workflow-smoke.json",
    "target/pi-rpc-surface.json",
  ],
};
function redactText(value) {
  return String(value)
    .replace(/(Authorization:\s*Bearer\s+)\S+/gi, "$1[REDACTED]")
    .replace(/\b(Bearer\s+)[A-Za-z0-9._~+/=-]{16,}/g, "$1[REDACTED]")
    .replace(/sk-[A-Za-z0-9_-]{8,}/g, "sk-REDACTED")
    .replace(/((?:api[_-]?key|token|password|secret)["'\s]*[:=]["'\s]*)[^"'\s,}]+/gi, "$1[REDACTED]");
}
function redactValue(value) {
  if (typeof value === "string") {
    return redactText(value);
  }
  if (Array.isArray(value)) {
    return value.map(redactValue);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, redactValue(item)]),
    );
  }
  return value;
}
function providerReportFilename(provider) {
  const redacted = redactText(provider || "provider");
  const filename = redacted.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
  return filename || "provider";
}
const envPosture = Object.fromEntries(
  envNames.map((name) => [name, process.env[name] ? "set" : "unset"]),
);
const records = fs.existsSync(preflightPath)
  ? fs
      .readFileSync(preflightPath, "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => JSON.parse(line))
  : [];
const providers = new Map();
for (const record of records) {
  const provider = record.provider || "all";
  if (!providers.has(provider)) {
    providers.set(provider, []);
  }
  providers.get(provider).push(record);
}
if (providers.size === 0) {
  providers.set("all", []);
}
for (const [provider, checks] of providers) {
  const failed = checks.filter((check) => check.status === "fail").length;
  const skipped = checks.filter((check) => check.status === "skip").length;
  const passed = checks.filter((check) => check.status === "pass").length;
  const report = {
    provider: redactText(provider),
    status: failed > 0 ? "fail" : skipped > 0 && passed === 0 ? "skip" : "pass",
    started_at: startedAt,
    finished_at: finishedAt,
    command_exit_code: Number(exitCode),
    summary: { passed, failed, skipped, total: checks.length },
    environment_posture: envPosture,
    evidence_refs: [
      ...new Set([
        ...(providerEvidenceRefs.all || []),
        ...(providerEvidenceRefs[provider] || []),
      ]),
    ],
    checks: redactValue(checks),
  };
  fs.writeFileSync(
    path.join(reportDir, `${providerReportFilename(provider)}.json`),
    `${JSON.stringify(redactValue(report), null, 2)}\n`,
  );
}
NODE

{
  echo "# Real Provider Smoke Report"
  echo
  echo "- Started: $started_at"
  echo "- Finished: $finished_at"
  echo "- Exit code: $status"
  echo "- Selected providers: $(redact_inline "$selected_providers")"
  echo "- Real-provider gate: $(env_state WHIPPLESCRIPT_E2E_REAL_PROVIDERS)"
  echo "- Native strict mode: $(env_state WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT)"
  echo "- Native surface mode: $(env_state WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE)"
  echo "- Native provider configs: $(env_state WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS)"
  echo "- Destructive provider tests: $(env_state WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS)"
  echo "- Disposable target marker: $(env_state WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET)"
  echo "- Disposable target acknowledgement: $(env_state WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK)"
  echo "- Loft test issue: $(env_state WHIPPLESCRIPT_LOFT_TEST_ISSUE)"
  echo "- Loft destructive tests: $(env_state WHIPPLESCRIPT_LOFT_DESTRUCTIVE_TESTS)"
  echo "- Loft disposable target: $(env_state WHIPPLESCRIPT_LOFT_DISPOSABLE_TARGET)"
  echo "- Loft disposable acknowledgement: $(env_state WHIPPLESCRIPT_LOFT_DISPOSABLE_ACK)"
  echo "- Loft CLI override: $(env_state WHIPPLESCRIPT_LOFT_CLI)"
  echo "- Loft repo override: $(env_state WHIPPLESCRIPT_LOFT_REPO)"
  echo "- Loft repo preflight skip: $(env_state WHIPPLESCRIPT_LOFT_SKIP_REPO_PREFLIGHT)"
  echo "- BAML endpoint: $(env_state WHIPPLESCRIPT_BAML_TEST_ENDPOINT)"
  echo "- BAML destructive tests: $(env_state WHIPPLESCRIPT_BAML_DESTRUCTIVE_TESTS)"
  echo "- BAML disposable target: $(env_state WHIPPLESCRIPT_BAML_DISPOSABLE_TARGET)"
  echo "- BAML disposable acknowledgement: $(env_state WHIPPLESCRIPT_BAML_DISPOSABLE_ACK)"
  echo "- BAML function: $(env_state WHIPPLESCRIPT_BAML_TEST_FUNCTION)"
  echo "- BAML arguments JSON: $(env_state WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON)"
  echo "- BAML output type: $(env_state WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE)"
  echo "- BAML health path: $(env_state WHIPPLESCRIPT_BAML_HEALTH_PATH)"
  echo "- BAML CLI skip: $(env_state WHIPPLESCRIPT_BAML_SKIP_CLI)"
  echo "- Codex smoke prompt: $(env_state WHIPPLESCRIPT_CODEX_SMOKE_PROMPT)"
  echo "- Codex smoke expected response: $(env_state WHIPPLESCRIPT_CODEX_SMOKE_EXPECTED)"
  echo "- Codex smoke model override: $(env_state WHIPPLESCRIPT_CODEX_MODEL)"
  echo "- Codex smoke profile override: $(env_state WHIPPLESCRIPT_CODEX_PROFILE)"
  echo "- Codex smoke report override: $(env_state WHIPPLESCRIPT_CODEX_SMOKE_REPORT)"
  echo "- Codex destructive tests: $(env_state WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS)"
  echo "- Codex disposable target: $(env_state WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET)"
  echo "- Codex disposable acknowledgement: $(env_state WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK)"
  echo "- Claude destructive tests: $(env_state WHIPPLESCRIPT_CLAUDE_DESTRUCTIVE_TESTS)"
  echo "- Claude disposable target: $(env_state WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET)"
  echo "- Claude disposable acknowledgement: $(env_state WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK)"
  echo "- Pi destructive tests: $(env_state WHIPPLESCRIPT_PI_DESTRUCTIVE_TESTS)"
  echo "- Pi disposable target: $(env_state WHIPPLESCRIPT_PI_DISPOSABLE_TARGET)"
  echo "- Pi disposable acknowledgement: $(env_state WHIPPLESCRIPT_PI_DISPOSABLE_ACK)"
  echo "- Preflight JSONL: $PREFLIGHT_REPORT"
  echo "- Per-provider report dir: $PROVIDER_REPORT_DIR"
  echo
  echo "## Per-Provider Reports"
  echo
  if compgen -G "$PROVIDER_REPORT_DIR/*.json" >/dev/null; then
    for provider_report in "$PROVIDER_REPORT_DIR"/*.json; do
      echo "- $provider_report"
    done
  fi
  echo
  echo "## Boundary Preflight"
  echo
  echo '```jsonl'
  if [[ -f "$PREFLIGHT_REPORT" ]]; then
    # shellcheck disable=SC2016
    redact_output <"$PREFLIGHT_REPORT" | sed 's/```/` ` `/g'
  fi
  echo '```'
  echo
  echo "## Output"
  echo
  echo '```text'
  # shellcheck disable=SC2016
  redact_output <"$OUTPUT" | sed 's/```/` ` `/g'
  echo '```'
} >"$REPORT"

redact_output <"$OUTPUT"
echo "Wrote real-provider smoke report: $REPORT" >&2

exit "$status"
