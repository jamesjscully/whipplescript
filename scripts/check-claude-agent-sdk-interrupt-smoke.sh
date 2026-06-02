#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_INTERRUPT_REPORT:-$ROOT/target/claude-agent-sdk-interrupt-smoke.json}"
MODEL="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL:-sonnet}"
TIMEOUT_MS="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS:-60000}"
INTERRUPT_DELAY_MS="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_INTERRUPT_DELAY_MS:-1}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL="$MODEL" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_INTERRUPT_DELAY_MS="$INTERRUPT_DELAY_MS" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn, spawnSync } = require("child_process");

const reportPath = process.argv[2];
const live = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE === "1";
const model = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS || 60000);
const interruptDelayMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_INTERRUPT_DELAY_MS || 1);
const events = [];
let stderr = "";
let stdoutBuffer = "";
let cancelSent = false;

function authPosture() {
  const apiKey = Boolean(process.env.ANTHROPIC_API_KEY);
  const bedrock =
    process.env.CLAUDE_CODE_USE_BEDROCK === "1" &&
    (Boolean(process.env.AWS_PROFILE) ||
      Boolean(process.env.AWS_ACCESS_KEY_ID) ||
      Boolean(process.env.AWS_WEB_IDENTITY_TOKEN_FILE));
  const vertex =
    process.env.CLAUDE_CODE_USE_VERTEX === "1" &&
    Boolean(process.env.ANTHROPIC_VERTEX_PROJECT_ID || process.env.GOOGLE_APPLICATION_CREDENTIALS);
  const local = localClaudeAuthPosture();
  return {
    apiKey,
    bedrock,
    vertex,
    localClaudeLogin: local.loggedIn,
    localClaudeAuthMethod: local.authMethod,
    localClaudeApiProvider: local.apiProvider,
    embeddedAuthAvailable: apiKey || bedrock || vertex,
    localAuthAvailable: local.loggedIn,
    authAvailable: apiKey || bedrock || vertex || local.loggedIn,
    acceptedRefs: [
      "env:ANTHROPIC_API_KEY",
      "bedrock:AWS_PROFILE|AWS_ACCESS_KEY_ID|AWS_WEB_IDENTITY_TOKEN_FILE",
      "vertex:ANTHROPIC_VERTEX_PROJECT_ID|GOOGLE_APPLICATION_CREDENTIALS",
      "local:claude-auth-status",
    ],
  };
}

function localClaudeAuthPosture() {
  const result = spawnSync("claude", ["auth", "status"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.status !== 0) {
    return { loggedIn: false, authMethod: null, apiProvider: null };
  }
  try {
    const parsed = JSON.parse(result.stdout || "{}");
    return {
      loggedIn: Boolean(parsed.loggedIn),
      authMethod: parsed.authMethod || null,
      apiProvider: parsed.apiProvider || null,
    };
  } catch {
    return { loggedIn: false, authMethod: null, apiProvider: null };
  }
}

const child = spawn("node", ["scripts/claude-agent-sdk-sidecar.mjs"], {
  cwd: process.cwd(),
  stdio: ["pipe", "pipe", "pipe"],
  env: {
    ...process.env,
    WHIPPLESCRIPT_CLAUDE_AGENT_SDK_FAKE: live ? "" : "1",
  },
});

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function finish(code, extra = {}) {
  clearTimeout(timeout);
  const terminalEvents = events.filter((event) =>
    ["claude.turn.completed", "claude.turn.failed", "claude.turn.cancelled", "run/error"].includes(event.type),
  );
  const ok =
    code === 0 &&
    cancelSent &&
    terminalEvents.length === 1 &&
    terminalEvents[0].type === "claude.turn.cancelled" &&
    Boolean(terminalEvents[0].payload?.acknowledgement);
  writeReport({
    ok,
    skipped: false,
    live,
    model,
    interruptDelayMs,
    cancelSent,
    terminalCount: terminalEvents.length,
    terminalType: terminalEvents[0]?.type || null,
    terminalPayload: terminalEvents[0]?.payload || null,
    cancellationAcknowledgement: terminalEvents[0]?.payload?.acknowledgement || null,
    authPosture: auth,
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

const auth = authPosture();
if (live && !auth.authAvailable) {
  writeReport({
    ok: false,
    skipped: false,
    live,
    model,
    interruptDelayMs,
    authPosture: auth,
    missingConfigRefs: ["env:ANTHROPIC_API_KEY", "local:claude-auth-status"],
    error: "missing_claude_auth",
  });
  process.exit(1);
}

const timeout = setTimeout(() => {
  finish(2, { error: "timeout", timeoutMs });
}, timeoutMs);

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
    if (["claude.turn.completed", "claude.turn.failed", "claude.turn.cancelled", "run/error"].includes(event.type)) {
      finish(event.type === "run/error" ? 1 : 0);
      return;
    }
  }
});

child.on("exit", (code) => {
  if (!events.some((event) => ["claude.turn.completed", "claude.turn.failed", "claude.turn.cancelled", "run/error"].includes(event.type))) {
    finish(code || 1, { error: "sidecar-exited-before-terminal" });
  }
});

child.stdin.write(
  `${JSON.stringify({
    type: "run/start",
    run_id: "claude-interrupt-1",
    request: {
      run_id: "claude-interrupt-1",
      cwd: process.cwd(),
      model,
      prompt: "Count slowly from 1 to 200, one number per line. Do not use tools.",
      allowed_tools: ["Read", "Glob", "Grep"],
      disallowed_tools: ["Bash", "Edit", "Write"],
      permission_mode: "default",
      max_turns: 1,
      setting_sources: [],
    },
  })}\n`,
);

setTimeout(() => {
  cancelSent = true;
  child.stdin.write(
    `${JSON.stringify({
      type: "run/cancel",
      run_id: "claude-interrupt-1",
    })}\n`,
  );
}, interruptDelayMs);
NODE
