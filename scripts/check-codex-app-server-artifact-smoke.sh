#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_REPORT:-$ROOT/target/codex-app-server-artifact-smoke.json}"
MODEL="${WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL:-gpt-5.4-mini}"
TIMEOUT_MS="${WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TIMEOUT_MS:-90000}"
EXPECTED_TEXT="${WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TEXT:-Codex native artifact fixture.}"
ACK="${WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK:-}"
TARGET="${WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET:-}"
REQUIRED_ACK="I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE:-}" != "1" ]]; then
  cd "$ROOT"
  cargo test -p whipplescript-kernel \
    native_adapter_streams_codex_notifications_and_diff_artifacts --lib
  node - "$REPORT" <<'NODE'
const fs = require("fs");
const reportPath = process.argv[2];
const report = {
  ok: true,
  skipped: false,
  live: false,
  coverage: "deterministic-codex-app-server-diff-artifact",
  note: "Validates Codex app-server adapter extraction of diff artifact refs without raw diff content. Set WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE=1 with disposable acknowledgement for live provider-generated file-change validation.",
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.error(`Codex app-server artifact report wrote ${reportPath}`);
NODE
  exit 0
fi

if [[ "$ACK" != "$REQUIRED_ACK" || -z "$TARGET" ]]; then
  node - "$REPORT" "$TARGET" "$ACK" <<'NODE'
const fs = require("fs");
const [reportPath, target, ack] = process.argv.slice(2);
const report = {
  ok: false,
  skipped: true,
  live: true,
  error: "missing_live_artifact_prerequisites",
  required: {
    WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET: "non-empty marker for a disposable live-provider target",
    WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK: "I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE",
  },
  received: {
    hasDisposableTarget: Boolean(target),
    hasExpectedAcknowledgement: ack === "I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE",
  },
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.error("missing Codex disposable target acknowledgement");
NODE
  exit 1
fi

cd "$ROOT"

WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL="$MODEL" \
WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TEXT="$EXPECTED_TEXT" \
WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET="$TARGET" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { spawn } = require("child_process");

const reportPath = process.argv[2];
const model = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL;
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TIMEOUT_MS || 90000);
const expectedText = process.env.WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_TEXT;
const disposableTarget = process.env.WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET;
const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "whip-codex-artifact-"));
const artifactName = "whip-codex-artifact.txt";
const artifactPath = path.join(workspace, artifactName);
const prompt = `In this disposable workspace, create a file named ${artifactName} containing exactly ${JSON.stringify(expectedText)}. Do not modify any other file.`;

const child = spawn(
  "codex",
  [
    "app-server",
    "--listen",
    "stdio://",
    "-c",
    `model="${model}"`,
    "-c",
    'sandbox_mode="workspace-write"',
    "-c",
    'approval_policy="never"',
  ],
  { cwd: workspace, stdio: ["pipe", "pipe", "pipe"] },
);

let stdoutBuffer = "";
let stderr = "";
let nextId = 1;
const pending = new Map();
const notifications = [];
const serverRequests = [];

function sha256(bytes) {
  return `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
}

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function counts(messages) {
  return messages.reduce((result, message) => {
    if (message.method) {
      result[message.method] = (result[message.method] || 0) + 1;
    }
    return result;
  }, {});
}

function summarizeDiffEvidence() {
  function redactedPath(value) {
    if (typeof value !== "string" || value.length === 0 || value === "/dev/null") {
      return null;
    }
    return path.basename(value);
  }
  return notifications
    .filter(
      (notification) =>
        notification.method === "turn/diff/updated" ||
        notification.method === "item/fileChange/patchUpdated",
    )
    .map((notification) => {
      const params = notification.params || {};
      const changes = Array.isArray(params.changes) ? params.changes : [];
      const diff = typeof params.diff === "string" ? params.diff : "";
      const changedFilesFromDiff = [];
      for (const line of diff.split(/\r?\n/)) {
        const gitMatch = line.match(/^diff --git a\/(.+) b\/(.+)$/);
        if (gitMatch) {
          changedFilesFromDiff.push({
            path: redactedPath(gitMatch[2]),
            hasOldPath: true,
            hasNewPath: true,
          });
          continue;
        }
        const plusMatch = line.match(/^\+\+\+ b\/(.+)$/);
        if (plusMatch) {
          changedFilesFromDiff.push({
            path: redactedPath(plusMatch[1]),
            hasOldPath: false,
            hasNewPath: true,
          });
        }
      }
      const changedFiles = changes.length > 0
        ? changes.map((change) => ({
            path: redactedPath(change.path || change.newPath || change.oldPath),
            hasOldPath: typeof change.oldPath === "string",
            hasNewPath: typeof change.newPath === "string",
          }))
        : changedFilesFromDiff;
      return {
        method: notification.method,
        threadId: params.threadId || null,
        turnId: params.turnId || null,
        itemId: params.itemId || null,
        diffBytes: typeof params.diff === "string" ? Buffer.byteLength(params.diff) : null,
        changedFiles,
      };
    });
}

function finish(code, extra) {
  try {
    child.kill("SIGTERM");
  } catch {}
  const fileExists = fs.existsSync(artifactPath);
  const fileBytes = fileExists ? fs.readFileSync(artifactPath) : null;
  const fileText = fileBytes ? fileBytes.toString("utf8") : "";
  const fileMatches = fileText === expectedText || fileText === `${expectedText}\n`;
  const terminal = notifications.find((notification) => notification.method === "turn/completed");
  const diffEvidence = summarizeDiffEvidence();
  const changedFileSeen = diffEvidence.some((entry) =>
    entry.changedFiles.some((change) => change.path === artifactName),
  );
  const rawReport = JSON.stringify({
    diffEvidence,
    stderrBytes: Buffer.byteLength(stderr),
    extra,
  });
  const rawExpectedContentLeaked = rawReport.includes(expectedText);
  const ok =
    code === 0 &&
    terminal?.params?.turn?.status === "completed" &&
    fileExists &&
    fileMatches &&
    !rawExpectedContentLeaked;
  let workspaceRemoved = false;
  try {
    fs.rmSync(workspace, { recursive: true, force: true });
    workspaceRemoved = !fs.existsSync(workspace);
  } catch {
    workspaceRemoved = false;
  }
  writeReport({
    ok,
    skipped: false,
    live: true,
    coverage: "live-codex-app-server-file-artifact",
    model,
    disposableTarget,
    workspace: {
      kind: "temporary_disposable_workspace",
      basename: path.basename(workspace),
      pathRedacted: true,
      removed: workspaceRemoved,
    },
    terminalStatus: terminal?.params?.turn?.status ?? null,
    notificationCounts: counts(notifications),
    serverRequestCounts: counts(serverRequests),
    diffEvidence,
    changedFileEvidence: {
      emittedDiffEvidence: diffEvidence.length > 0,
      changedFileSeen,
      fallback: changedFileSeen ? null : "provider-created file validated from disposable workspace",
    },
    artifactFile: {
      relativePath: artifactName,
      exists: fileExists,
      bytes: fileBytes ? fileBytes.length : null,
      sha256: fileBytes ? sha256(fileBytes) : null,
      matchesExpectedFixture: fileMatches,
    },
    stderrBytes: Buffer.byteLength(stderr),
    rawExpectedContentLeaked,
    ...extra,
  });
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
          error: { code: -32000, message: "unsupported request in WhippleScript Codex artifact smoke" },
        })}\n`,
      );
      break;
  }
}

(async () => {
  try {
    const initialize = await request("initialize", {
      clientInfo: { name: "whipplescript-codex-artifact-smoke", version: "0.0.0" },
      capabilities: {},
    });
    const threadStart = await request("thread/start", {
      cwd: workspace,
      model,
      sandbox: "workspace-write",
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
      cwd: workspace,
      model,
      approvalPolicy: "never",
      sandboxPolicy: { type: "workspaceWrite", networkAccess: false },
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
      durationMs: Date.now() - startedAt,
    });
  } catch (error) {
    clearTimeout(timeout);
    finish(1, { error: "request-failed", detail: error.message || String(error) });
  }
})();
NODE
