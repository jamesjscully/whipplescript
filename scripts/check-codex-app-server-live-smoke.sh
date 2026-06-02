#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE_REPORT:-$ROOT/target/codex-app-server-live-smoke.json}"
MODEL="${WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL:-gpt-5.4-mini}"
TIMEOUT_MS="${WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS:-45000}"
PROMPT="${WHIPPLESCRIPT_CODEX_APP_SERVER_PROMPT:-Read-only smoke test. Reply with exactly: WHIPPLESCRIPT_CODEX_SMOKE_OK}"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE:-}" != "1" ]]; then
  node - "$REPORT" <<'NODE'
const fs = require("fs");
const reportPath = process.argv[2];
const report = {
  ok: true,
  skipped: true,
  reason: "set WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 to run the live Codex app-server smoke",
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.error(report.reason);
NODE
  exit 0
fi

cd "$ROOT"

WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL="$MODEL" \
WHIPPLESCRIPT_CODEX_APP_SERVER_PROMPT="$PROMPT" \
WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS="$TIMEOUT_MS" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn } = require("child_process");

const reportPath = process.argv[2];
const model = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL;
const prompt = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_PROMPT;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_TIMEOUT_MS || 45000);
const requiredNotifications = ["thread/started", "turn/started", "turn/completed"];

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

function summarizeNotifications() {
  const counts = {};
  for (const notification of notifications) {
    if (!notification.method) {
      continue;
    }
    counts[notification.method] = (counts[notification.method] || 0) + 1;
  }
  return counts;
}

function summarizeServerRequests() {
  const counts = {};
  for (const request of serverRequests) {
    if (!request.method) {
      continue;
    }
    counts[request.method] = (counts[request.method] || 0) + 1;
  }
  return counts;
}

function jsonShape(value) {
  if (value === null) {
    return { type: "null" };
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

function evidenceSummary() {
  const approvalRequests = [];
  const toolRequests = [];
  for (const request of serverRequests) {
    const params = request.params || {};
    if (request.method?.includes("/requestApproval")) {
      approvalRequests.push({
        method: request.method,
        requestIdType: typeof request.id,
        threadId: params.threadId || null,
        turnId: params.turnId || null,
        itemId: params.itemId || null,
        approvalId: params.approvalId || null,
        hasReason: typeof params.reason === "string" && params.reason.length > 0,
        commandBytes: typeof params.command === "string" ? Buffer.byteLength(params.command) : null,
      });
    }
    if (request.method === "item/tool/call" || request.method === "item/tool/requestUserInput") {
      toolRequests.push({
        method: request.method,
        requestIdType: typeof request.id,
        threadId: params.threadId || null,
        turnId: params.turnId || null,
        callId: params.callId || null,
        tool: params.tool || null,
        argumentsShape: Object.prototype.hasOwnProperty.call(params, "arguments")
          ? jsonShape(params.arguments)
          : null,
      });
    }
  }
  const diffNotifications = notifications
    .filter(
      (notification) =>
        notification.method === "turn/diff/updated" ||
        notification.method === "item/fileChange/patchUpdated",
    )
    .map((notification) => {
      const params = notification.params || {};
      return {
        method: notification.method,
        threadId: params.threadId || null,
        turnId: params.turnId || null,
        itemId: params.itemId || null,
        diffBytes: typeof params.diff === "string" ? Buffer.byteLength(params.diff) : null,
        changesCount: Array.isArray(params.changes) ? params.changes.length : null,
      };
    });
  const itemNotifications = notifications
    .filter((notification) => notification.method === "item/started" || notification.method === "item/completed")
    .map((notification) => {
      const item = notification.params?.item || {};
      return {
        method: notification.method,
        threadId: notification.params?.threadId || null,
        turnId: notification.params?.turnId || null,
        itemId: item.id || notification.params?.itemId || null,
        itemType: item.type || null,
        status: item.status || null,
      };
    });
  return {
    approvalRequests,
    toolRequests,
    diffNotifications,
    itemNotifications,
  };
}

function finish(code, extra) {
  const counts = summarizeNotifications();
  const requestCounts = summarizeServerRequests();
  const missingNotifications = requiredNotifications.filter((method) => !counts[method]);
  writeReport({
    ok: code === 0 && missingNotifications.length === 0,
    skipped: false,
    model,
    requiredNotifications,
    missingNotifications,
    notificationCounts: counts,
    serverRequestCounts: requestCounts,
    evidence: evidenceSummary(),
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  child.kill("SIGTERM");
  process.exit(code === 0 && missingNotifications.length === 0 ? 0 : code || 1);
}

const timeout = setTimeout(() => {
  finish(2, {
    error: "timeout",
    timeoutMs,
  });
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
      finish(1, {
        error: "invalid-json",
        detail: error.message,
      });
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
  const message = { jsonrpc: "2.0", id, method, params };
  child.stdin.write(`${JSON.stringify(message)}\n`);
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

function respond(id, result) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

function respondError(id, message) {
  child.stdin.write(
    `${JSON.stringify({
      jsonrpc: "2.0",
      id,
      error: { code: -32000, message },
    })}\n`,
  );
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
      respondError(message.id, "unsupported request in WhippleScript Codex smoke");
      break;
  }
}

(async () => {
  try {
    const initialize = await request("initialize", {
      clientInfo: {
        name: "whipplescript-native-smoke",
        version: "0.0.0",
      },
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
    finish(completed ? 0 : 3, {
      codexUserAgent: initialize.userAgent,
      platformOs: initialize.platformOs,
      threadId,
      turnId,
      turnStatus: completed?.params?.turn?.status ?? null,
      durationMs: Date.now() - startedAt,
    });
  } catch (error) {
    clearTimeout(timeout);
    finish(1, {
      error: "request-failed",
      detail: error.message || String(error),
    });
  }
})();
NODE
