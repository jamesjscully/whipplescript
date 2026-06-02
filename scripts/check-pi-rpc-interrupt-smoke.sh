#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_PI_RPC_INTERRUPT_REPORT:-$ROOT/target/pi-rpc-interrupt-smoke.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_PI_RPC_INTERRUPT_TIMEOUT_MS:-10000}"
LIVE="${WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

WHIPPLESCRIPT_PI_RPC_INTERRUPT_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE="$LIVE" \
node --input-type=module - "$REPORT" <<'NODE'
import fs from "node:fs";
import { spawn } from "node:child_process";

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_PI_RPC_INTERRUPT_TIMEOUT_MS || 10000);
const live = process.env.WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE === "1";

function stopChild(child) {
  child.stdin.end();
  child.kill("SIGTERM");
  setTimeout(() => child.kill("SIGKILL"), 2000).unref();
}

function probeAbort() {
  return new Promise((resolve) => {
    const child = spawn("pi", ["--mode", "rpc", "--no-session", "--offline"], {
      cwd: process.cwd(),
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdoutBuffer = "";
    let stderr = "";
    let done = false;
    function finish(result) {
      if (done) {
        return;
      }
      done = true;
      clearTimeout(timeout);
      stopChild(child);
      resolve({
        ...result,
        stderrBytes: Buffer.byteLength(stderr),
      });
    }
    const timeout = setTimeout(() => {
      finish({ ok: false, error: "timeout", timeoutMs });
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
          finish({ ok: false, error: "invalid-json", detail: error.message });
          return;
        }
        if (message.type === "response" && message.command === "abort") {
          finish({
            ok: Boolean(message.success),
            responseId: message.id || null,
            command: message.command,
            success: Boolean(message.success),
          });
          return;
        }
      }
    });
    child.on("exit", (code) => {
      finish({ ok: false, error: "exited-before-response", status: code });
    });
    child.stdin.write(`${JSON.stringify({ id: "abort-1", type: "abort" })}\n`);
  });
}

function probeLiveAbort() {
  return new Promise((resolve) => {
    const child = spawn("pi", ["--mode", "rpc", "--no-session", "--no-tools"], {
      cwd: process.cwd(),
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdoutBuffer = "";
    let stderr = "";
    let done = false;
    let sentAbort = false;
    let promptAccepted = false;
    let abortAcknowledged = false;
    let turnEnds = 0;
    const eventTypes = [];
    const assistantStopReasons = [];
    function send(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    }
    function finish(result) {
      if (done) {
        return;
      }
      done = true;
      clearTimeout(timeout);
      stopChild(child);
      resolve({
        ...result,
        promptAccepted,
        sentAbort,
        abortAcknowledged,
        turnEnds,
        eventTypes,
        assistantStopReasons,
        stderrBytes: Buffer.byteLength(stderr),
      });
    }
    const timeout = setTimeout(() => {
      finish({ ok: false, error: "timeout", timeoutMs });
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
          finish({ ok: false, error: "invalid-json", detail: error.message });
          return;
        }
        if (message.type) {
          eventTypes.push(message.type);
        }
        if (message.type === "response" && message.command === "prompt") {
          promptAccepted = Boolean(message.success);
        }
        if (message.type === "turn_start" && !sentAbort) {
          sentAbort = true;
          send({ id: "abort-1", type: "abort" });
        }
        if (message.message?.role === "assistant" && message.message?.stopReason) {
          assistantStopReasons.push(message.message.stopReason);
        }
        if (message.type === "turn_end") {
          turnEnds += 1;
        }
        if (message.type === "response" && message.command === "abort") {
          abortAcknowledged = Boolean(message.success);
        }
        if (message.type === "agent_end") {
          setTimeout(() => {
            finish({
              ok:
                promptAccepted &&
                sentAbort &&
                abortAcknowledged &&
                turnEnds === 1 &&
                assistantStopReasons.includes("aborted"),
            });
          }, 250);
        }
      }
    });
    child.on("exit", (code) => {
      finish({ ok: false, error: "exited-before-terminal", status: code });
    });
    send({
      id: "prompt-1",
      type: "prompt",
      message:
        "Write exactly 20000 lines. Each line should contain the line number and the sentence: cancellation validation filler text. Do not use tools.",
    });
  });
}

const abortProbe = live ? await probeLiveAbort() : await probeAbort();
const report = {
  ok: abortProbe.ok,
  checkedAt: new Date().toISOString(),
  coverage: live ? "live-rpc-inflight-abort" : "rpc-abort-command-only",
  note: live
    ? "This validates live Pi RPC prompt acceptance, abort acknowledgement, assistant stopReason=aborted, and single turn_end ordering with tools disabled."
    : "This validates the Pi RPC abort command response shape. Set WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE=1 to validate in-flight turn terminal ordering.",
  abortProbe,
};

fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
if (!report.ok) {
  console.error(JSON.stringify(report, null, 2));
  process.exit(1);
}
console.error(`Pi RPC interrupt report wrote ${reportPath}`);
NODE
