#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_REPORT:-$ROOT/target/claude-agent-sdk-artifact-smoke.json}"
TIMEOUT_MS="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_TIMEOUT_MS:-60000}"
LIVE="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE:-0}"

mkdir -p "$(dirname "$REPORT")"
cd "$ROOT"

cargo test -p whipplescript-kernel \
  native_adapter_captures_claude_artifact_refs_without_raw_content --lib

WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_TIMEOUT_MS="$TIMEOUT_MS" \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE="$LIVE" \
node - "$REPORT" <<'NODE'
const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { spawn, spawnSync } = require("child_process");

const reportPath = process.argv[2];
const timeoutMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_TIMEOUT_MS || 60000);
const live = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE === "1";
const model = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL || "sonnet";
let events = [];
let stderr = "";
let stdoutBuffer = "";

function writeReport(report) {
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

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

function disposablePosture() {
  const expected = "I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE";
  return {
    hasTarget: Boolean(process.env.WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET || process.env.WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET),
    acknowledged:
      (process.env.WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK || process.env.WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK) === expected,
    expectedAck: expected,
  };
}

function removeMatchingRootLeak(expectedText) {
  const leakedPath = path.join(os.tmpdir(), "whip-claude-artifact.txt");
  try {
    if (fs.existsSync(leakedPath) && fs.readFileSync(leakedPath, "utf8") === expectedText) {
      fs.rmSync(leakedPath, { force: true });
      return true;
    }
  } catch {
    return false;
  }
  return false;
}

async function runLiveFixture() {
  const auth = authPosture();
  const disposable = disposablePosture();
  if (!auth.authAvailable || !disposable.hasTarget || !disposable.acknowledged) {
    writeReport({
      ok: false,
      checkedAt: new Date().toISOString(),
      live,
      coverage: "live-claude-agent-sdk-artifact",
      authPosture: auth,
      disposablePosture: disposable,
      missingConfigRefs: [
        ...(!auth.authAvailable ? ["env:ANTHROPIC_API_KEY", "local:claude-auth-status"] : []),
        ...(!disposable.hasTarget ? ["WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET"] : []),
        ...(!disposable.acknowledged ? ["WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK"] : []),
      ],
      error: "missing_live_artifact_prerequisites",
    });
    process.exit(1);
  }

  const expectedText = "WHIPPLESCRIPT_CLAUDE_ARTIFACT_OK\n";
  removeMatchingRootLeak(expectedText);
  const attempts = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_ATTEMPTS || 2);
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "whip-claude-artifact-"));
    const artifactPath = path.join(workspace, "whip-claude-artifact.txt");
    const ok = await runSidecar({
      cwd: workspace,
      fake: false,
      request: {
        run_id: `claude-artifact-live-${attempt}`,
        cwd: workspace,
        prompt: `In this disposable workspace, use the Write tool to create the exact file path ${JSON.stringify(artifactPath)} containing exactly ${JSON.stringify(expectedText)}. Do not modify any other file. Do not reply DONE until after that exact file path exists with that exact content. After the file is written, reply exactly: DONE`,
        model,
        allowed_tools: ["Read", "Glob", "Grep", "Write", "Edit"],
        disallowed_tools: ["Bash"],
        permission_mode: "bypassPermissions",
        allow_dangerously_skip_permissions: true,
        max_turns: 8,
        setting_sources: [],
      },
      artifactPath,
      expectedText,
      coverage: "live-claude-agent-sdk-artifact",
      authPosture: auth,
      disposablePosture: disposable,
      attempt,
      attempts,
    });
    if (ok) {
      process.exit(0);
    }
  }
  process.exit(1);
}

async function runFakeFixture() {
  const ok = await runSidecar({
    cwd: process.cwd(),
    fake: true,
    request: {
      run_id: "claude-artifact-1",
      cwd: process.cwd(),
      prompt: "Emit the fake artifact fixture.",
      artifact_fixture: true,
      allowed_tools: ["Read", "Glob", "Grep"],
      disallowed_tools: ["Bash", "Edit", "Write"],
      permission_mode: "default",
      max_turns: 1,
      setting_sources: [],
    },
    coverage: "fake-sidecar-artifact-metadata",
  });
  process.exit(ok ? 0 : 1);
}

function runSidecar({ cwd, fake, request, artifactPath, expectedText, coverage, authPosture, disposablePosture, attempt, attempts }) {
  return new Promise((resolve) => {
events = [];
stderr = "";
stdoutBuffer = "";
let finished = false;
const child = spawn("node", ["scripts/claude-agent-sdk-sidecar.mjs"], {
  cwd: process.cwd(),
  stdio: ["pipe", "pipe", "pipe"],
  env: {
    ...process.env,
    WHIPPLESCRIPT_CLAUDE_AGENT_SDK_FAKE: fake ? "1" : "",
    WHIPPLESCRIPT_CLAUDE_AGENT_SDK_FAKE_ARTIFACT: fake ? "1" : "",
  },
});

function finish(code, extra = {}) {
  if (finished) {
    return;
  }
  finished = true;
  clearTimeout(timeout);
  const artifactEvents = events.filter((event) => event.type === "claude.artifact.captured");
  const terminalEvents = events.filter((event) =>
    ["claude.turn.completed", "claude.turn.failed", "claude.turn.cancelled", "run/error"].includes(event.type),
  );
  const contentBlockSummary = events.reduce(
    (summary, event) => {
      const blocks = event.payload?.content_blocks;
      if (!blocks) {
        return summary;
      }
      for (const [type, count] of Object.entries(blocks.type_counts || {})) {
        summary.type_counts[type] = (summary.type_counts[type] || 0) + count;
      }
      for (const [toolName, count] of Object.entries(blocks.tool_name_counts || {})) {
        summary.tool_name_counts[toolName] = (summary.tool_name_counts[toolName] || 0) + count;
      }
      return summary;
    },
    { type_counts: {}, tool_name_counts: {} },
  );
  const raw = JSON.stringify(events);
  const firstArtifact = artifactEvents[0]?.payload?.artifacts?.[0] || null;
  const fileExists = artifactPath ? fs.existsSync(artifactPath) : false;
  const fileBytes = fileExists ? fs.readFileSync(artifactPath) : null;
  const fileMatches = expectedText ? fileBytes?.toString("utf8") === expectedText : true;
  const removedRootLeak = expectedText ? removeMatchingRootLeak(expectedText) : false;
  let workspaceRemoved = false;
  if (artifactPath) {
    try {
      fs.rmSync(path.dirname(artifactPath), { recursive: true, force: true });
      workspaceRemoved = !fs.existsSync(path.dirname(artifactPath));
    } catch {
      workspaceRemoved = false;
    }
  }
  const ok =
    code === 0 &&
    terminalEvents.length === 1 &&
    terminalEvents[0].type === "claude.turn.completed" &&
    (live ? fileExists && fileMatches : artifactEvents.length === 1 && firstArtifact?.uri) &&
    !raw.includes("secret fake artifact bytes");
  writeReport({
    ok,
    checkedAt: new Date().toISOString(),
    live,
    coverage,
    model: request.model || model,
    attempt: attempt
      ? {
          index: attempt,
          max: attempts,
        }
      : null,
    workspace: artifactPath
      ? {
          kind: "temporary_disposable_workspace",
          basename: path.basename(path.dirname(artifactPath)),
          pathRedacted: true,
          removed: workspaceRemoved,
          removedRootLeak,
        }
      : null,
    artifactCount: artifactEvents.length,
    terminalType: terminalEvents[0]?.type || null,
    artifactFile: artifactPath
      ? {
          exists: fileExists,
          relativePath: path.basename(artifactPath),
          bytes: fileBytes ? fileBytes.length : 0,
          sha256: fileBytes ? `sha256:${crypto.createHash("sha256").update(fileBytes).digest("hex")}` : null,
          matchesExpectedFixture: fileMatches,
        }
      : null,
    artifactShape: firstArtifact
      ? {
          hasId: Boolean(firstArtifact.id || firstArtifact.artifact_id),
          kind: firstArtifact.kind || null,
          hasUri: Boolean(firstArtifact.uri || firstArtifact.ref || firstArtifact.path),
          hasHash: Boolean(firstArtifact.content_hash || firstArtifact.hash),
          mimeType: firstArtifact.mime_type || firstArtifact.mime || null,
          required: Boolean(firstArtifact.required),
        }
      : null,
    eventCounts: events.reduce((counts, event) => {
      counts[event.type] = (counts[event.type] || 0) + 1;
      return counts;
    }, {}),
    contentBlockSummary,
    authPosture,
    disposablePosture,
    stderrBytes: Buffer.byteLength(stderr),
    ...extra,
  });
  child.kill("SIGTERM");
  resolve(ok);
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
    if (event.payload?.artifacts) {
      event = {
        ...event,
        payload: {
          ...event.payload,
          artifacts: event.payload.artifacts.map((artifact) => {
            const { content, raw_content, data, bytes, ...metadata } = artifact;
            return metadata;
          }),
        },
      };
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
    run_id: request.run_id,
    request,
  })}\n`,
);
  });
}

if (live) {
  void runLiveFixture();
} else {
  void runFakeFixture();
}
NODE
