#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CODEX_APP_SERVER_INTERRUPT_REPORT:-$ROOT/target/codex-app-server-interrupt-smoke.json}"
MODEL="${WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL:-gpt-5.4-mini}"
TIMEOUT_MS="${WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS:-60000}"
INTERRUPT_DELAY_MS="${WHIPPLESCRIPT_CODEX_APP_SERVER_INTERRUPT_DELAY_MS:-1200}"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE:-}" != "1" ]]; then
  node - "$REPORT" <<'NODE'
const fs = require("fs");
const reportPath = process.argv[2];
const report = {
  ok: true,
  skipped: true,
  reason: "set WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 to run the live Codex interrupt smoke",
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.error(report.reason);
NODE
  exit 0
fi

cd "$ROOT"

WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL="$MODEL" \
WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_CODEX_APP_SERVER_INTERRUPT_DELAY_MS="$INTERRUPT_DELAY_MS" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn } = require("child_process");

const reportPath = process.argv[2];
const model = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS || 60000);
const interruptDelayMs = Number(process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_INTERRUPT_DELAY_MS || 1200);
const prompt = "Count slowly from 1 to 200, one number per line. Do not use tools.";

const child = spawn(
  "codex",
  [
    "app-server",
    "--listen",
    "stdio://",
    "-c",
    `model="${model}"`,
    "-c",
    'sandbox_mode="read-only"',
    "-c",
    'approval_policy="never"',
  ],
  { cwd: process.cwd(), stdio: ["pipe", "pipe", "pipe"] },
);

let stdoutBuffer = "";
let stderr = "";
let nextId = 1;
const pending = new Map();
const notifications = [];
const serverRequests = [];

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function finish(code, extra) {
  const terminalNotifications = notifications.filter(
    (notification) => notification.method === "turn/completed",
  );
  const terminalStatuses = terminalNotifications.map(
    (notification) => notification.params?.turn?.status ?? null,
  );
  const ok =
    code === 0 &&
    terminalNotifications.length === 1 &&
    terminalStatuses[0] === "interrupted";
  writeReport({
    ok,
    skipped: false,
    model,
    interruptDelayMs,
    terminalCount: terminalNotifications.length,
    terminalStatuses,
    notificationCounts: notifications.reduce((counts, notification) => {
      if (notification.method) {
        counts[notification.method] = (counts[notification.method] || 0) + 1;
      }
      return counts;
    }, {}),
    serverRequestCounts: serverRequests.reduce((counts, request) => {
      if (request.method) {
        counts[request.method] = (counts[request.method] || 0) + 1;
      }
      return counts;
    }, {}),
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  child.kill("SIGTERM");
  process.exit(ok ? 0 : code || 1);
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
    let message;
    try {
      message = JSON.parse(line);
    } catch (error) {
      clearTimeout(timeout);
      finish(1, { error: "invalid-json", detail: error.message });
      return;
    }
    if (message.id && pending.has(message.id)) {
      const handlers = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        handlers.reject(message.error);
      } else {
        handlers.resolve(message.result);
      }
    } else if (message.id && message.method) {
      serverRequests.push(message);
      respondToServerRequest(message);
    } else if (message.method) {
      notifications.push(message);
    }
  }
});

function request(method, params) {
  const id = nextId++;
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

function respond(id, result) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

function respondToServerRequest(message) {
  switch (message.method) {
    case "item/commandExecution/requestApproval":
    case "item/fileChange/requestApproval":
      respond(message.id, { decision: "decline" });
      break;
    case "item/tool/requestUserInput":
      respond(message.id, { answers: {} });
      break;
    case "item/tool/call":
      respond(message.id, { success: false, contentItems: [] });
      break;
    default:
      child.stdin.write(
        `${JSON.stringify({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32000, message: "unsupported request in WhippleScript interrupt smoke" },
        })}\n`,
      );
      break;
  }
}

(async () => {
  try {
    const initialize = await request("initialize", {
      clientInfo: { name: "whipplescript-interrupt-smoke", version: "0.0.0" },
      capabilities: {},
    });
    const threadStart = await request("thread/start", {
      cwd: process.cwd(),
      model,
      sandbox: "read-only",
      approvalPolicy: "never",
      ephemeral: true,
      sessionStartSource: "startup",
    });
    const threadId = threadStart.thread?.id;
    if (!threadId) {
      throw new Error("thread/start response missing thread.id");
    }
    const turnStart = await request("turn/start", {
      threadId,
      input: [{ type: "text", text: prompt }],
      cwd: process.cwd(),
      model,
      approvalPolicy: "never",
      sandboxPolicy: { type: "readOnly", networkAccess: false },
    });
    const turnId = turnStart.turn?.id;
    if (!turnId) {
      throw new Error("turn/start response missing turn.id");
    }
    await new Promise((resolve) => setTimeout(resolve, interruptDelayMs));
    const interruptResult = await request("turn/interrupt", { threadId, turnId });
    const startedAt = Date.now();
    let completed = null;
    while (Date.now() - startedAt < timeoutMs) {
      completed = notifications.find(
        (notification) =>
          notification.method === "turn/completed" &&
          notification.params?.threadId === threadId &&
          notification.params?.turn?.id === turnId,
      );
      if (completed) {
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    clearTimeout(timeout);
    finish(completed?.params?.turn?.status === "interrupted" ? 0 : 3, {
      codexUserAgent: initialize.userAgent,
      platformOs: initialize.platformOs,
      threadId,
      turnId,
      interruptRequestSent: true,
      interruptAcknowledged: true,
      interruptResult,
      completedStatus: completed?.params?.turn?.status ?? null,
      durationMs: Date.now() - startedAt,
    });
  } catch (error) {
    clearTimeout(timeout);
    finish(1, { error: "request-failed", detail: error.message || String(error) });
  }
})();
NODE
