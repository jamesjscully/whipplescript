#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_PI_RPC_ERROR_REPORT:-$ROOT/target/pi-rpc-error-smoke.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_PI_RPC_ERROR_TIMEOUT_MS:-20000}"
LIVE="${WHIPPLESCRIPT_PI_RPC_ERROR_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

cargo test -p whipplescript-kernel \
  native_adapter_maps_pi_remote_prompt_error_without_raw_message --lib

WHIPPLESCRIPT_PI_RPC_ERROR_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_PI_RPC_ERROR_LIVE="$LIVE" \
node --input-type=module - "$REPORT" <<'NODE'
import fs from "node:fs";
import { spawn } from "node:child_process";

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_PI_RPC_ERROR_TIMEOUT_MS || 20000);
const live = process.env.WHIPPLESCRIPT_PI_RPC_ERROR_LIVE === "1";

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

if (!live) {
  writeReport({
    ok: true,
    checkedAt: new Date().toISOString(),
    live,
    coverage: "deterministic-pi-rpc-remote-error",
    note: "Validates Pi adapter mapping of RPC remote errors to redacted native boundary failures. Set WHIPPLESCRIPT_PI_RPC_ERROR_LIVE=1 to validate the installed Pi RPC error response shape.",
  });
  console.error(`Pi RPC error report wrote ${reportPath}`);
  process.exit(0);
}

const child = spawn("pi", ["--mode", "rpc", "--no-session", "--offline"], {
  cwd: process.cwd(),
  stdio: ["pipe", "pipe", "pipe"],
});
let stdoutBuffer = "";
let stderr = "";
let done = false;
const responses = [];

function stopChild() {
  child.stdin.end();
  child.kill("SIGTERM");
  setTimeout(() => child.kill("SIGKILL"), 2000).unref();
}

function finish(result) {
  if (done) {
    return;
  }
  done = true;
  clearTimeout(timeout);
  stopChild();
  const response = responses.find((message) => message.command === "definitely_invalid_command");
  const ok = Boolean(response) && response.success === false && Boolean(response.errorShape);
  writeReport({
    ok,
    checkedAt: new Date().toISOString(),
    live,
    coverage: "live-pi-rpc-invalid-command-error",
    response,
    stderrBytes: Buffer.byteLength(stderr),
    ...result,
  });
  process.exit(ok ? 0 : 1);
}

const timeout = setTimeout(() => finish({ error: "timeout", timeoutMs }), timeoutMs);
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
      finish({ error: "invalid-json", detail: error.message });
      return;
    }
    if (message.type === "response") {
      responses.push({
        command: message.command || null,
        success: Boolean(message.success),
        errorShape:
          message.error === undefined
            ? null
            : typeof message.error === "string"
              ? { type: "string", chars: [...message.error].length }
              : Array.isArray(message.error)
                ? { type: "array", items: message.error.length }
                : message.error && typeof message.error === "object"
                  ? { type: "object", keys: Object.keys(message.error).length }
                  : { type: typeof message.error },
      });
      finish({});
      return;
    }
  }
});
child.on("exit", (code) => finish({ error: "exited-before-response", status: code }));
child.stdin.write(`${JSON.stringify({ id: "error-1", type: "definitely_invalid_command" })}\n`);
NODE
