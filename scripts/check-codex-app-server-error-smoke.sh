#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_REPORT:-$ROOT/target/codex-app-server-error-smoke.json}"
MODEL="${WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL:-gpt-5.4-mini}"
TIMEOUT_MS="${WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_TIMEOUT_MS:-30000}"
LIVE="${WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

cargo test -p whipplescript-kernel \
  native_adapter_maps_codex_remote_start_error_without_raw_message --lib

if [[ "$LIVE" != "1" ]]; then
  node - "$REPORT" <<'NODE'
const fs = require("fs");
const reportPath = process.argv[2];
const report = {
  ok: true,
  checkedAt: new Date().toISOString(),
  live: false,
  coverage: "deterministic-codex-app-server-remote-error",
  note: "Validates Codex app-server adapter mapping of JSON-RPC remote errors to redacted native boundary failures. Set WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1 to validate the installed Codex app-server error response shape.",
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.error(`Codex app-server error report wrote ${reportPath}`);
NODE
  exit 0
fi

WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL="$MODEL" \
WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_TIMEOUT_MS="$TIMEOUT_MS" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const { spawn } = require("child_process");

const reportPath = process.argv[2];
const model = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_TIMEOUT_MS || 30000);
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
const messages = [];

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function messageSummary(message) {
  return {
    idType: Object.prototype.hasOwnProperty.call(message, "id") ? typeof message.id : null,
    method: message.method || null,
    hasResult: Object.prototype.hasOwnProperty.call(message, "result"),
    hasError: Object.prototype.hasOwnProperty.call(message, "error"),
    errorCode: typeof message.error?.code === "number" ? message.error.code : null,
    errorMessageShape:
      typeof message.error?.message === "string"
        ? { type: "string", chars: [...message.error.message].length }
        : null,
    resultShape: message.result && typeof message.result === "object"
      ? { type: "object", keys: Object.keys(message.result).length }
      : message.result === undefined
        ? null
        : { type: typeof message.result },
  };
}

function finish(code, extra = {}) {
  clearTimeout(timeout);
  const errorResponse = messages.find((message) => message.hasError);
  const ok =
    code === 0 &&
    Boolean(errorResponse) &&
    errorResponse.errorCode === -32600 &&
    errorResponse.errorMessageShape?.type === "string";
  writeReport({
    ok,
    checkedAt: new Date().toISOString(),
    live: true,
    coverage: "live-codex-app-server-invalid-turn-start-error",
    model,
    messageCount: messages.length,
    errorResponse,
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  try {
    child.kill("SIGTERM");
  } catch {}
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
      finish(1, { error: "invalid-json", detail: error.message });
      return;
    }
    const summary = messageSummary(message);
    messages.push(summary);
    if (message.id && pending.has(message.id)) {
      const handlers = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        handlers.reject(message.error);
      } else {
        handlers.resolve(message.result);
      }
    }
  }
});

child.on("exit", (code) => {
  if (!messages.some((message) => message.hasError)) {
    finish(code || 1, { error: "exited-before-error-response" });
  }
});

function request(method, params) {
  const id = nextId++;
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

(async () => {
  try {
    await request("initialize", {
      clientInfo: {
        name: "whipplescript-native-error-smoke",
        version: "0.0.0",
      },
      capabilities: {},
    });
    await request("turn/start", {
      threadId: "definitely-not-a-thread",
      input: [{ type: "text", text: "Say OK" }],
      cwd: process.cwd(),
      model,
      approvalPolicy: "never",
      sandboxPolicy: { type: "readOnly", networkAccess: false },
    });
    finish(3, { error: "turn-start-unexpectedly-succeeded" });
  } catch (error) {
    finish(0, {
      caughtError: {
        code: typeof error.code === "number" ? error.code : null,
        messageShape:
          typeof error.message === "string"
            ? { type: "string", chars: [...error.message].length }
            : null,
      },
    });
  }
})();
NODE
