#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCHEMA_DIR="${WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_DIR:-$ROOT/target/codex-app-server-schema}"
REPORT="${WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_REPORT:-$ROOT/target/codex-app-server-schema-report.json}"
PIN="${WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_PIN:-$ROOT/spec/codex-app-server-schema.pin.json}"
UPDATE_PIN="${WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_UPDATE:-0}"

REQUIRED_METHODS=(
  "thread/start"
  "turn/started"
  "turn/completed"
  "turn/interrupt"
  "turn/diff/updated"
)

if ! command -v codex >/dev/null 2>&1; then
  echo "codex not found on PATH" >&2
  exit 2
fi

rm -rf "$SCHEMA_DIR"
mkdir -p "$SCHEMA_DIR" "$(dirname "$REPORT")"

if ! codex app-server generate-json-schema --out "$SCHEMA_DIR" --experimental >/dev/null 2>&1; then
  echo "failed to generate Codex app-server schema" >&2
  exit 1
fi

node - "$ROOT" "$SCHEMA_DIR" "$REPORT" "$PIN" "$UPDATE_PIN" "${REQUIRED_METHODS[@]}" <<'NODE'
const fs = require("fs");
const crypto = require("crypto");
const path = require("path");

const [root, schemaDir, reportPath, pinPath, updatePin, ...requiredMethods] = process.argv.slice(2);

function walk(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return walk(full);
    if (entry.isFile() && entry.name.endsWith(".json")) return [full];
    return [];
  });
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function readText(file) {
  return fs.readFileSync(file, "utf8");
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

const files = walk(schemaDir).sort();
const relativeFiles = files.map((file) => path.relative(schemaDir, file));
const fileHashes = files.map((file, index) => ({
  path: relativeFiles[index],
  sha256: sha256(canonicalJson(JSON.parse(readText(file)))),
}));
const allText = files.map(readText).join("\n");
const schemaHash = sha256(fileHashes.map((file) => `${file.sha256}  ${file.path}`).join("\n"));
const codexVersion = require("child_process")
  .execFileSync("codex", ["--version"], { encoding: "utf8" })
  .trim();
const methods = requiredMethods.map((method) => ({
  method,
  present: allText.includes(JSON.stringify(method)),
}));
const missingMethods = methods.filter((method) => !method.present).map((method) => method.method);
const metadata = {
  schema: "whipplescript.codex_app_server_schema_pin.v1",
  codex_version: codexVersion,
  generated_by: "codex app-server generate-json-schema --experimental",
  schema_file_count: files.length,
  schema_sha256: schemaHash,
  required_methods: methods,
  file_hashes: fileHashes,
};

fs.writeFileSync(reportPath, `${JSON.stringify(metadata, null, 2)}\n`);

if (missingMethods.length > 0) {
  console.error(`missing required Codex app-server schema methods: ${missingMethods.join(", ")}`);
  process.exit(1);
}

if (updatePin === "1") {
  fs.writeFileSync(pinPath, `${JSON.stringify(metadata, null, 2)}\n`);
  console.error(`Updated Codex app-server schema pin: ${path.relative(root, pinPath)}`);
  process.exit(0);
}

if (!fs.existsSync(pinPath)) {
  console.error(`missing Codex app-server schema pin: ${path.relative(root, pinPath)}`);
  console.error("Run with WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_UPDATE=1 to create it.");
  process.exit(1);
}

const pinned = JSON.parse(fs.readFileSync(pinPath, "utf8"));
const comparablePinned = {
  schema: pinned.schema,
  codex_version: pinned.codex_version,
  generated_by: pinned.generated_by,
  schema_file_count: pinned.schema_file_count,
  schema_sha256: pinned.schema_sha256,
  required_methods: pinned.required_methods,
  file_hashes: pinned.file_hashes,
};
if (canonicalJson(comparablePinned) !== canonicalJson(metadata)) {
  console.error("Codex app-server schema metadata changed from pin.");
  console.error(`Report: ${path.relative(root, reportPath)}`);
  console.error(`Pin: ${path.relative(root, pinPath)}`);
  process.exit(1);
}

console.error(`Codex app-server schema pin matches: ${path.relative(root, pinPath)}`);
NODE
