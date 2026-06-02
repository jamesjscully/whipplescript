#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_PI_RPC_ARTIFACT_REPORT:-$ROOT/target/pi-rpc-artifact-smoke.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_PI_RPC_ARTIFACT_TIMEOUT_MS:-30000}"
LIVE="${WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

cargo test -p whipplescript-kernel \
  native_adapter_captures_pi_artifact_refs_without_raw_content --lib

WHIPPLESCRIPT_PI_RPC_ARTIFACT_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE="$LIVE" \
node --input-type=module - "$REPORT" <<'NODE'
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_PI_RPC_ARTIFACT_TIMEOUT_MS || 30000);
const live = process.env.WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE === "1";
const requestedProvider = process.env.WHIPPLESCRIPT_PI_RPC_PROVIDER || null;
const requestedModel = process.env.WHIPPLESCRIPT_PI_RPC_MODEL || null;

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function disposablePosture() {
  const expected = "I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE";
  return {
    hasTarget: Boolean(process.env.WHIPPLESCRIPT_PI_DISPOSABLE_TARGET || process.env.WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET),
    acknowledged:
      (process.env.WHIPPLESCRIPT_PI_DISPOSABLE_ACK || process.env.WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK) === expected,
    expectedAck: expected,
  };
}

function stopChild(child) {
  child.stdin.end();
  child.kill("SIGTERM");
  setTimeout(() => child.kill("SIGKILL"), 2000).unref();
}

function runLiveFixture() {
  return new Promise((resolve) => {
    const disposable = disposablePosture();
    if (!disposable.hasTarget || !disposable.acknowledged) {
      writeReport({
        ok: false,
        checkedAt: new Date().toISOString(),
        live,
        coverage: "live-pi-rpc-artifact",
        disposablePosture: disposable,
        missingConfigRefs: [
          ...(!disposable.hasTarget ? ["WHIPPLESCRIPT_PI_DISPOSABLE_TARGET"] : []),
          ...(!disposable.acknowledged ? ["WHIPPLESCRIPT_PI_DISPOSABLE_ACK"] : []),
        ],
        error: "missing_live_artifact_prerequisites",
      });
      process.exit(1);
    }

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "whip-pi-artifact-"));
    const artifactPath = path.join(workspace, "whip-pi-artifact.txt");
    const expectedText = "WHIPPLESCRIPT_PI_ARTIFACT_OK\n";
    const child = spawn("pi", ["--mode", "rpc", "--no-session"], {
      cwd: workspace,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdoutBuffer = "";
    let stderr = "";
    let done = false;
    const eventTypes = [];
    let promptAccepted = false;
    let terminalSeen = false;

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
      const fileExists = fs.existsSync(artifactPath);
      const fileBytes = fileExists ? fs.readFileSync(artifactPath) : null;
      const fileMatches = fileBytes?.toString("utf8") === expectedText;
      let workspaceRemoved = false;
      try {
        fs.rmSync(workspace, { recursive: true, force: true });
        workspaceRemoved = !fs.existsSync(workspace);
      } catch {
        workspaceRemoved = false;
      }
      const ok = Boolean(result.ok) && promptAccepted && terminalSeen && fileExists && fileMatches;
      writeReport({
        ok,
        checkedAt: new Date().toISOString(),
        live,
        coverage: "live-pi-rpc-artifact",
        provider: requestedProvider,
        model: requestedModel,
        workspace: {
          kind: "temporary_disposable_workspace",
          basename: path.basename(workspace),
          pathRedacted: true,
          removed: workspaceRemoved,
        },
        promptAccepted,
        terminalSeen,
        eventTypes,
        disposablePosture: disposable,
        artifactFile: {
          exists: fileExists,
          relativePath: path.basename(artifactPath),
          bytes: fileBytes ? fileBytes.length : 0,
          sha256: fileBytes ? `sha256:${crypto.createHash("sha256").update(fileBytes).digest("hex")}` : null,
          matchesExpectedFixture: fileMatches,
        },
        stderrBytes: Buffer.byteLength(stderr),
        ...result,
      });
      process.exit(ok ? 0 : 1);
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
        if (message.type === "turn_end") {
          terminalSeen = true;
        }
        if (message.type === "agent_end") {
          setTimeout(() => finish({ ok: true }), 250);
        }
      }
    });

    child.on("exit", (code) => {
      finish({ ok: false, error: "exited-before-terminal", status: code });
    });

    send({
      id: "prompt-1",
      type: "prompt",
      message: `In this disposable workspace, create a file named whip-pi-artifact.txt containing exactly ${JSON.stringify(expectedText)}. Do not modify any other file.`,
      tools: ["write", "edit", "read"],
      ...(requestedProvider ? { provider: requestedProvider } : {}),
      ...(requestedModel ? { model: requestedModel } : {}),
    });
  });
}

if (live) {
  await runLiveFixture();
} else {
  writeReport({
    ok: true,
    checkedAt: new Date().toISOString(),
    live,
    coverage: "deterministic-pi-rpc-artifact-metadata",
    note:
      "Validates Pi RPC native adapter extraction of explicit artifact metadata refs without raw artifact content. Set WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE=1 with disposable target acknowledgement for live provider-generated artifact validation.",
    artifactShape: {
      hasId: true,
      kind: "file",
      hasUri: true,
      hasHash: true,
      mimeType: "text/plain",
      required: true,
    },
  });
  console.error(`Pi RPC artifact report wrote ${reportPath}`);
}
NODE
