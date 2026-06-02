#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_PI_RPC_REPORT:-$ROOT/target/pi-rpc-surface.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_PI_RPC_TIMEOUT_MS:-10000}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

WHIPPLESCRIPT_PI_RPC_TIMEOUT_MS="$TIMEOUT_MS" node --input-type=module - "$REPORT" <<'NODE'
import fs from "node:fs";
import { spawn, spawnSync } from "node:child_process";

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_PI_RPC_TIMEOUT_MS || 10000);

function command(name, args) {
  const result = spawnSync(name, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  const stdout = String(result.stdout || "").trim();
  const stderr = String(result.stderr || "").trim();
  return {
    ok: result.status === 0,
    status: result.status ?? null,
    stdout,
    stderr,
    output: `${stdout}\n${stderr}`.trim(),
  };
}

function npmPackage(name) {
  const result = command("npm", ["view", name, "version", "dist-tags", "--json"]);
  if (!result.ok) {
    return { ok: false, error: result.stderr || result.stdout };
  }
  try {
    return { ok: true, metadata: JSON.parse(result.stdout) };
  } catch {
    return { ok: true, metadata: { raw: result.stdout } };
  }
}

function probeRpc() {
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
      child.kill("SIGTERM");
      resolve({
        ...result,
        stderrBytes: Buffer.byteLength(stderr),
      });
    }
    const timeout = setTimeout(() => {
      finish({ ok: false, error: "timeout", timeoutMs });
    }, timeoutMs);
    child.on("error", (error) => {
      finish({
        ok: false,
        error: "spawn-failed",
        code: error.code || null,
      });
    });
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
        if (message.type === "response" && message.command === "get_state") {
          finish({
            ok: Boolean(message.success),
            sessionId: message.data?.sessionId || null,
            modelProvider: message.data?.model?.provider || null,
            modelId: message.data?.model?.id || null,
            isStreaming: message.data?.isStreaming ?? null,
          });
          return;
        }
      }
    });
    child.on("exit", (code) => {
      finish({ ok: false, error: "exited-before-response", status: code });
    });
    child.stdin.write(`${JSON.stringify({ id: "state-1", type: "get_state" })}\n`);
  });
}

const piVersion = command("pi", ["--version"]);
const piHelp = command("pi", ["--help"]);
const rpcProbe = await probeRpc();
const piHelpText = piHelp.output || "";
const report = {
  ok:
    piVersion.ok &&
    piHelp.ok &&
    piHelpText.includes("--mode <mode>") &&
    piHelpText.includes("--tools") &&
    rpcProbe.ok,
  checkedAt: new Date().toISOString(),
  decision: {
    selectedEmbedding: "pi-rpc-subprocess",
    rationale:
      "Pi docs recommend RPC mode for language-agnostic/process-isolated clients; WhippleScript is Rust and can reuse the JSONL adapter boundary.",
  },
  piCli: {
    available: piVersion.ok,
    version: piVersion.output || null,
    hasRpcModeFlag: piHelpText.includes("--mode <mode>"),
    hasToolsFlag: piHelpText.includes("--tools"),
    hasNoSessionFlag: piHelpText.includes("--no-session"),
    helpStream: piHelp.stdout ? "stdout" : "stderr",
    versionStream: piVersion.stdout ? "stdout" : "stderr",
  },
  rpcProbe,
  sdkPackages: {
    "@earendil-works/pi-coding-agent": npmPackage("@earendil-works/pi-coding-agent"),
    "@mariozechner/pi-coding-agent": npmPackage("@mariozechner/pi-coding-agent"),
  },
};

fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
if (!report.ok) {
  console.error(JSON.stringify(report, null, 2));
  process.exit(1);
}
console.error(`Pi RPC surface report wrote ${reportPath}`);
NODE
