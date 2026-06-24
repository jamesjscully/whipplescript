#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_REPORT:-$ROOT/target/claude-agent-sdk-surface.json}"

mkdir -p "$(dirname "$REPORT")"

node - "$REPORT" "$ROOT" <<'NODE'
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const reportPath = process.argv[2];
const root = process.argv[3];
const strictRegistry = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_STRICT_REGISTRY === "1";
const probeRegistry = process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_PROBE_REGISTRY !== "0";
const parsedRegistryTimeoutMs = Number(process.env.WHIPPLESCRIPT_CLAUDE_AGENT_SDK_REGISTRY_TIMEOUT_MS || 15000);
const registryTimeoutMs =
  Number.isFinite(parsedRegistryTimeoutMs) && parsedRegistryTimeoutMs > 0
    ? parsedRegistryTimeoutMs
    : 15000;
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

function readJson(relativePath) {
  try {
    return JSON.parse(fs.readFileSync(path.join(root, relativePath), "utf8"));
  } catch {
    return null;
  }
}

const claudeVersion = command("claude", ["--version"]);
const claudeHelp = command("claude", ["--help"]);
const nodeVersion = command("node", ["--version"]);
const npmVersion = command("npm", ["--version"]);
const pythonVersion = command("python3", ["--version"]);
const tsPackage = probeRegistry
  ? command(
      "npm",
      [
        "view",
        "@anthropic-ai/claude-agent-sdk",
        "version",
        "dist-tags",
        "--json",
      ],
      { timeout: registryTimeoutMs },
    )
  : { ok: false, skipped: true };
const pythonPackage = probeRegistry
  ? command(
      "python3",
      [
        "-m",
        "pip",
        "index",
        "versions",
        "claude-agent-sdk",
      ],
      { timeout: registryTimeoutMs },
    )
  : { ok: false, skipped: true };

const missingClaudeFlags = claudeHelp.ok
  ? requiredClaudeFlags.filter((flag) => !claudeHelp.stdout.includes(flag))
  : requiredClaudeFlags;

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const declaredTsSdkVersion =
  packageJson?.dependencies?.["@anthropic-ai/claude-agent-sdk"] ||
  packageJson?.devDependencies?.["@anthropic-ai/claude-agent-sdk"] ||
  null;
const lockedTsSdkVersion =
  packageLock?.packages?.["node_modules/@anthropic-ai/claude-agent-sdk"]?.version ||
  packageLock?.dependencies?.["@anthropic-ai/claude-agent-sdk"]?.version ||
  null;
const tsSdkConfigured = Boolean(declaredTsSdkVersion || lockedTsSdkVersion);

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

const warnings = [];
if (!probeRegistry) {
  warnings.push("registry probe disabled");
} else if (!tsPackage.ok) {
  warnings.push("npm registry metadata unavailable");
}
if (probeRegistry && !pythonPackage.ok) {
  warnings.push("Python SDK registry metadata unavailable");
}

const report = {
  ok:
    claudeVersion.ok &&
    claudeHelp.ok &&
    missingClaudeFlags.length === 0 &&
    nodeVersion.ok &&
    npmVersion.ok &&
    tsSdkConfigured &&
    (!strictRegistry || tsPackage.ok),
  checkedAt: new Date().toISOString(),
  decision: {
    selectedEmbedding: "typescript-sidecar",
    rationale:
      "TypeScript Agent SDK has the current documented package, Node is already available, and the SDK bundles or can target a Claude Code binary; Python remains a fallback/probe surface.",
  },
  strictRegistry,
  registryProbe: {
    enabled: probeRegistry,
    timeoutMs: registryTimeoutMs,
  },
  warnings,
  claudeCli: {
    available: claudeVersion.ok,
    version: claudeVersion.stdout || null,
    requiredFlags: requiredClaudeFlags,
    missingFlags: missingClaudeFlags,
  },
  typescriptSdk: {
    package: "@anthropic-ai/claude-agent-sdk",
    configured: tsSdkConfigured,
    declaredVersion: declaredTsSdkVersion,
    lockedVersion: lockedTsSdkVersion,
    npmRegistryAvailable: tsPackage.ok,
    registryMetadata: tsPackageMetadata,
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
