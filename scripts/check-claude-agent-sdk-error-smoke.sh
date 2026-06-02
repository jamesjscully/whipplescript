#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_REPORT:-$ROOT/target/claude-agent-sdk-error-smoke.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_TIMEOUT_MS:-30000}"
LIVE="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

cargo test -p whipplescript-kernel \
  native_adapter_maps_claude_remote_start_error_without_raw_message --lib

WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE="$LIVE" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn } = require("child_process");

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_TIMEOUT_MS || 30000);
const live = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE === "1";

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

if (!live) {
  writeReport({
    ok: true,
    checkedAt: new Date().toISOString(),
    live,
    coverage: "deterministic-claude-agent-sdk-remote-error",
    note: "Validates Claude adapter mapping of sidecar remote errors to redacted native boundary failures. Set WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE=1 to validate a live SDK config-error envelope.",
  });
  console.error(`Claude Agent SDK error report wrote ${reportPath}`);
  process.exit(0);
}

const child = spawn("node", ["scripts/claude-agent-sdk-sidecar.mjs"], {
  cwd: process.cwd(),
  stdio: ["pipe", "pipe", "pipe"],
  env: {
    ...process.env,
    WHIPPLESCRIPT_CLAUDE_AGENT_SDK_FAKE: "",
  },
});
let stdoutBuffer = "";
let stderr = "";
const events = [];

function finish(code, extra = {}) {
  clearTimeout(timeout);
  const terminal = events.find((event) => event.type === "run/error");
  const ok =
    code === 0 &&
    terminal?.payload?.code === "claude_agent_sdk_failed" &&
    typeof terminal?.payload?.message === "string";
  writeReport({
    ok,
    checkedAt: new Date().toISOString(),
    live,
    coverage: "live-claude-agent-sdk-invalid-executable-error",
    terminalType: terminal?.type || null,
    terminalCode: terminal?.payload?.code || null,
    terminalMessageShape:
      typeof terminal?.payload?.message === "string"
        ? { type: "string", chars: [...terminal.payload.message].length }
        : null,
    eventCounts: events.reduce((counts, event) => {
      counts[event.type] = (counts[event.type] || 0) + 1;
      return counts;
    }, {}),
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  child.kill("SIGTERM");
  process.exit(ok ? 0 : code || 1);
}

const timeout = setTimeout(() => finish(2, { error: "timeout", timeoutMs }), timeoutMs);
child.stderr.on("data", (chunk) => {
  stderr += chunk.toString();
});
child.stdout.on("data", (chunk) => {
  stdoutBuffer += chunk.toString();
  for (;;) {
    const newline = stdoutBuffer.indexOf("\n");
    if (newline < 0) {
      break;
    }
    const line = stdoutBuffer.slice(0, newline).trim();
    stdoutBuffer = stdoutBuffer.slice(newline + 1);
    if (!line) {
      continue;
    }
    let event;
    try {
      event = JSON.parse(line);
    } catch (error) {
      finish(1, { error: "invalid-json", detail: error.message });
      return;
    }
    events.push(event);
    if (event.type === "run/error") {
      finish(0);
      return;
    }
  }
});
child.on("exit", (code) => {
  if (!events.some((event) => event.type === "run/error")) {
    finish(code || 1, { error: "sidecar-exited-before-error" });
  }
});
child.stdin.write(
  `${JSON.stringify({
    type: "run/start",
    run_id: "claude-invalid-executable-smoke",
    request: {
      run_id: "claude-invalid-executable-smoke",
      cwd: process.cwd(),
      path_to_claude: "/tmp/whip-missing-claude-executable",
      prompt: "Return OK",
      allowed_tools: ["Read"],
      disallowed_tools: ["Bash", "Edit", "Write"],
      permission_mode: "default",
      max_turns: 1,
      setting_sources: [],
    },
  })}\n`,
);
NODE
