#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_REPORT:-$ROOT/target/claude-agent-sdk-surface.json}"

mkdir -p "$(dirname "$REPORT")"

node - "$REPORT" <<'NODE'
const fs = require("fs");
const { execFileSync } = require("child_process");

const reportPath = process.argv[2];
const requiredClaudeFlags = [
  "--output-format",
  "--input-format",
  "--allowedTools",
  "--permission-mode",
  "--include-hook-events",
  "--session-id",
  "--settings",
];

function command(name, args, options = {}) {
  try {
    return {
      ok: true,
      stdout: execFileSync(name, args, {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
        ...options,
      }).trim(),
    };
  } catch (error) {
    return {
      ok: false,
      status: error.status ?? null,
      stdout: String(error.stdout || "").trim(),
      stderr: String(error.stderr || "").trim(),
    };
  }
}

const claudeVersion = command("claude", ["--version"]);
const claudeHelp = command("claude", ["--help"]);
const nodeVersion = command("node", ["--version"]);
const npmVersion = command("npm", ["--version"]);
const tsPackage = command("npm", [
  "view",
  "@anthropic-ai/claude-agent-sdk",
  "version",
  "dist-tags",
  "--json",
]);
const pythonVersion = command("python3", ["--version"]);
const pythonPackage = command("python3", [
  "-m",
  "pip",
  "index",
  "versions",
  "claude-agent-sdk",
]);

const missingClaudeFlags = claudeHelp.ok
  ? requiredClaudeFlags.filter((flag) => !claudeHelp.stdout.includes(flag))
  : requiredClaudeFlags;

let tsPackageMetadata = null;
if (tsPackage.ok) {
  try {
    tsPackageMetadata = JSON.parse(tsPackage.stdout);
  } catch {
    tsPackageMetadata = { raw: tsPackage.stdout };
  }
}

const pythonPackageVersion = pythonPackage.ok
  ? (pythonPackage.stdout.match(/claude-agent-sdk \(([^)]+)\)/) || [])[1] || null
  : null;

const report = {
  ok:
    claudeVersion.ok &&
    claudeHelp.ok &&
    missingClaudeFlags.length === 0 &&
    nodeVersion.ok &&
    npmVersion.ok &&
    tsPackage.ok,
  checkedAt: new Date().toISOString(),
  decision: {
    selectedEmbedding: "typescript-sidecar",
    rationale:
      "TypeScript Agent SDK has the current documented package, Node is already available, and the SDK bundles or can target a Claude Code binary; Python remains a fallback/probe surface.",
  },
  claudeCli: {
    available: claudeVersion.ok,
    version: claudeVersion.stdout || null,
    requiredFlags: requiredClaudeFlags,
    missingFlags: missingClaudeFlags,
  },
  typescriptSdk: {
    package: "@anthropic-ai/claude-agent-sdk",
    npmAvailable: tsPackage.ok,
    metadata: tsPackageMetadata,
    nodeVersion: nodeVersion.stdout || null,
    npmVersion: npmVersion.stdout || null,
  },
  pythonSdk: {
    package: "claude-agent-sdk",
    pipIndexAvailable: pythonPackage.ok,
    latestVersion: pythonPackageVersion,
    pythonVersion: pythonVersion.stdout || null,
  },
};

fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
if (!report.ok) {
  console.error(JSON.stringify(report, null, 2));
  process.exit(1);
}
console.error(`Claude Agent SDK surface report wrote ${reportPath}`);
NODE
