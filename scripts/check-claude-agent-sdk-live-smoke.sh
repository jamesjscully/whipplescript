#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE_REPORT:-$ROOT/target/claude-agent-sdk-live-smoke.json}"
MODEL="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL:-sonnet}"
TIMEOUT_MS="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS:-60000}"
PROMPT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_PROMPT:-Read-only smoke test. Reply with exactly: WHIPPLESCRIPT_CLAUDE_SMOKE_OK}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL="$MODEL" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_PROMPT="$PROMPT" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS="$TIMEOUT_MS" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn, spawnSync } = require("child_process");

const reportPath = process.argv[2];
const live = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE === "1";
const model = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL;
const prompt = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_PROMPT;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_TIMEOUT_MS || 60000);
const events = [];
let stderr = "";

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

function eventCounts() {
  return events.reduce((counts, event) => {
    counts[event.type] = (counts[event.type] || 0) + 1;
    return counts;
  }, {});
}

const auth = authPosture();
if (live && !auth.authAvailable) {
  writeReport({
    ok: false,
    skipped: false,
    live,
    model,
    authPosture: auth,
    missingConfigRefs: ["env:ANTHROPIC_API_KEY", "local:claude-auth-status"],
    error: "missing_claude_auth",
  });
  process.exit(1);
}

function finish(code, extra = {}) {
  clearTimeout(timeout);
  const terminalEvents = events.filter((event) =>
    ["claude.turn.completed", "claude.turn.failed", "claude.turn.cancelled", "run/error"].includes(event.type),
  );
  const sessionStarted = events.find((event) => event.type === "claude.session.started");
  const ok = code === 0 && Boolean(sessionStarted) && terminalEvents.length === 1 && terminalEvents[0].type === "claude.turn.completed";
  writeReport({
    ok,
    skipped: false,
    live,
    model,
    eventCounts: eventCounts(),
    terminalType: terminalEvents[0]?.type || null,
    terminalPayload: terminalEvents[0]?.payload || null,
    sessionId: sessionStarted?.payload?.session_id || null,
    authPosture: auth,
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  child.kill("SIGTERM");
  process.exit(ok ? 0 : code || 1);
}

const timeout = setTimeout(() => {
  finish(2, { error: "timeout", timeoutMs });
}, timeoutMs);

let stdoutBuffer = "";
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
      finish(event.type === "claude.turn.completed" ? 0 : 1);
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
    run_id: "claude-smoke-1",
    request: {
      run_id: "claude-smoke-1",
      cwd: process.cwd(),
      model,
      prompt,
      allowed_tools: ["Read", "Glob", "Grep"],
      disallowed_tools: ["Bash", "Edit", "Write"],
      permission_mode: "default",
      max_turns: 1,
      setting_sources: [],
    },
  })}\n`,
);
NODE
