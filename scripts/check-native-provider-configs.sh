#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_REPORT:-$ROOT/target/native-provider-config-validation.json}"
CONFIGS="${WHIPPLESCRIPT_PROVIDER_CONFIGS:-}"
if [[ -n "${WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS:-}" ]]; then
  if [[ -n "$CONFIGS" ]]; then
    CONFIGS="$CONFIGS:$WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS"
  else
    CONFIGS="$WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS"
  fi
fi
STRICT="${WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_STRICT:-0}"

mkdir -p "$(dirname "$REPORT")"

if [[ -z "$CONFIGS" ]]; then
  cat >"$REPORT" <<JSON
{
  "status": "skip",
  "message": "WHIPPLESCRIPT_PROVIDER_CONFIGS or WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS is not set"
}
JSON
  echo "Skipping native provider config validation."
  echo "Set WHIPPLESCRIPT_PROVIDER_CONFIGS to a colon-separated list of config files."
  if [[ "$STRICT" == "1" ]]; then
    echo "Native provider config validation is required in strict mode." >&2
    exit 2
  fi
  exit 0
fi

doctor_args=(--json doctor)
IFS=':' read -ra config_paths <<<"$CONFIGS"
for path in "${config_paths[@]}"; do
  if [[ -z "$path" ]]; then
    continue
  fi
  doctor_args+=(--provider-config "$path")
done

if [[ "${#doctor_args[@]}" -eq 2 ]]; then
  echo "provider config variables did not contain any config paths" >&2
  exit 2
fi

tmp_report="$(mktemp)"
cleanup() {
  rm -f "$tmp_report"
}
trap cleanup EXIT

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript -- \
  "${doctor_args[@]}" >"$tmp_report"

node --input-type=module - "$tmp_report" <<'NODE'
import fs from "node:fs";

const reportPath = process.argv[2];
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const checks = report.provider_config_checks ?? [];
const results = checks.flatMap((check) => check.results ?? []);
const failures = results.filter((result) => result.status === "fail");
const required = new Map([
  ["codex-main", "codex_app_server"],
  ["claude-main", "claude_agent_sdk"],
  ["pi-main", "pi_rpc"],
]);

let failed = false;
for (const failure of failures) {
  console.error(
    `native provider config failed: ${failure.provider || "<unknown>"} ` +
      `${failure.code}: ${failure.message}`,
  );
  failed = true;
}

for (const [provider, surface] of required) {
  const supported = results.some(
    (result) =>
      result.provider === provider &&
      result.surface === surface &&
      result.status === "pass" &&
      result.code === "surface_supported",
  );
  if (!supported) {
    console.error(
      `missing required native provider config: ${provider} on ${surface}`,
    );
    failed = true;
  }
}

if (failed) {
  process.exit(2);
}
NODE

mv "$tmp_report" "$REPORT"
trap - EXIT
echo "Wrote native provider config validation report: $REPORT" >&2
