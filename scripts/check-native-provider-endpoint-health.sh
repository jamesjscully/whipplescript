#!/usr/bin/env bash
set -euo pipefail

# Native provider endpoint health: codex + claude only. Pi is deferred from the
# v0.2 native gate (Jack, 2026-07-05) — with the owned/native harness the
# standalone Pi provider has no remaining point for now, and its live RPC probe
# would otherwise block this aggregate. Re-add the pi-surface/pi-interrupt probes
# (see git history) when Pi native support is revived.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_NATIVE_PROVIDER_HEALTH_REPORT:-$ROOT/target/native-provider-endpoint-health.json}"
LIVE="${WHIPPLESCRIPT_NATIVE_PROVIDER_HEALTH_LIVE:-0}"
WORK_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

mkdir -p "$(dirname "$REPORT")"

run_probe() {
  local name="$1"
  shift
  local status_file="$WORK_DIR/$name.status"
  set +e
  "$@"
  local status=$?
  set -e
  printf '%s\n' "$status" >"$status_file"
}

CODEX_REPORT="$WORK_DIR/codex-app-server-live-smoke.json"
CLAUDE_REPORT="$WORK_DIR/claude-agent-sdk-live-smoke.json"

run_probe codex \
  env WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE="$LIVE" \
    WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE_REPORT="$CODEX_REPORT" \
    "$ROOT/scripts/check-codex-app-server-live-smoke.sh"

run_probe claude \
  env WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE="$LIVE" \
    WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE_REPORT="$CLAUDE_REPORT" \
    "$ROOT/scripts/check-claude-agent-sdk-live-smoke.sh"

node --input-type=module - \
  "$REPORT" \
  "$LIVE" \
  "$CODEX_REPORT" "$WORK_DIR/codex.status" \
  "$CLAUDE_REPORT" "$WORK_DIR/claude.status" <<'NODE'
import fs from "node:fs";

const [
  reportPath,
  liveValue,
  codexPath,
  codexStatusPath,
  claudePath,
  claudeStatusPath,
] = process.argv.slice(2);

function readJson(path) {
  if (!fs.existsSync(path)) {
    return { ok: false, missingReport: true };
  }
  return JSON.parse(fs.readFileSync(path, "utf8"));
}

function readStatus(path) {
  if (!fs.existsSync(path)) {
    return 1;
  }
  return Number(fs.readFileSync(path, "utf8").trim() || "1");
}

function shape(value) {
  if (value === null || value === undefined) {
    return null;
  }
  if (Array.isArray(value)) {
    return { type: "array", items: value.length };
  }
  if (typeof value === "object") {
    return { type: "object", keys: Object.keys(value).length };
  }
  if (typeof value === "string") {
    return { type: "string", chars: [...value].length };
  }
  return { type: typeof value };
}

const live = liveValue === "1";
const codex = readJson(codexPath);
const claude = readJson(claudePath);
const checks = [
  {
    provider: "codex",
    surface: "codex_app_server",
    check: "session_turn_notifications",
    status: readStatus(codexStatusPath),
    ok: Boolean(codex.ok),
    skipped: Boolean(codex.skipped),
    coverage: live ? "live_app_server_turn" : "non_live_skipped_probe",
    evidence: {
      model: codex.model || null,
      notificationCounts: codex.notificationCounts || null,
      missingNotifications: codex.missingNotifications || [],
      stderrBytes: codex.stderrBytes ?? null,
    },
  },
  {
    provider: "claude",
    surface: "claude_agent_sdk",
    check: "session_terminal_event",
    status: readStatus(claudeStatusPath),
    ok: Boolean(claude.ok),
    skipped: Boolean(claude.skipped),
    coverage: live ? "live_agent_sdk_session" : "fake_agent_sdk_session",
    evidence: {
      model: claude.model || null,
      sessionIdPresent: typeof claude.sessionId === "string" && claude.sessionId.length > 0,
      terminalType: claude.terminalType || null,
      eventCounts: claude.eventCounts || null,
      authPostureShape: shape(claude.authPosture),
      stderrBytes: claude.stderrBytes ?? null,
    },
  },
];

const ok = checks.every((check) => check.status === 0 && check.ok);
const report = {
  ok,
  live,
  checkedAt: new Date().toISOString(),
  checks,
  sourceReportHandling: "provider-specific temporary reports summarized into this aggregate artifact",
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
if (!ok) {
  console.error(JSON.stringify(report, null, 2));
  process.exit(1);
}
console.error(`Native provider endpoint health report wrote ${reportPath}`);
NODE
