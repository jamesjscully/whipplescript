#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_REPORT:-$ROOT/target/codex-native-workflow-smoke.json}"
MODEL="${WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL:-gpt-5.4-mini}"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE:-}" != "1" ]]; then
  cd "$ROOT"
  cargo test -p whipplescript --test control_plane \
    dev_native_fixture_records_provider_lifecycle_and_artifacts_from_source_workflow
  node - "$REPORT" <<'NODE'
const fs = require("fs");
const reportPath = process.argv[2];
fs.writeFileSync(reportPath, `${JSON.stringify({
  ok: true,
  skipped: false,
  live: false,
  coverage: "deterministic-source-workflow-native-bridge",
  note: "Set WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE=1 to run source workflow -> Codex app-server through RuntimeKernel::run_native_agent_turn.",
}, null, 2)}\n`);
console.error(`Codex native workflow report wrote ${reportPath}`);
NODE
  exit 0
fi

cd "$ROOT"

WORKDIR="$(mktemp -d)"
STORE="$WORKDIR/store.sqlite"
WORKFLOW="$WORKDIR/codex-native-workflow.whip"
DEV_JSON="$WORKDIR/dev.json"
RUNS_JSON="$WORKDIR/runs.json"
LOG_JSON="$WORKDIR/log.json"
RECOVER_JSON="$WORKDIR/recover.json"

cleanup() {
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

cat >"$WORKFLOW" <<'WHIP'
workflow CodexNativeWorkflowSmoke

agent worker {
  profile "repo-reader"
  capacity 1
}

rule start_codex
  when started
  when worker is available
=> {
  tell worker "Read-only native workflow smoke. Reply exactly: WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_OK"
}
WHIP

WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL="$MODEL" \
WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS="${WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS:-512}" \
cargo run --quiet -p whipplescript -- \
  --store "$STORE" \
  --json \
  dev "$WORKFLOW" \
  --provider codex \
  --until idle >"$DEV_JSON"

INSTANCE_ID="$(node -e 'const fs=require("fs"); const value=JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(value.instance_id || "");' "$DEV_JSON")"
if [[ -z "$INSTANCE_ID" ]]; then
  echo "Codex native workflow dev output did not include instance_id" >&2
  exit 1
fi

cargo run --quiet -p whipplescript -- --store "$STORE" --json runs "$INSTANCE_ID" >"$RUNS_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json log "$INSTANCE_ID" >"$LOG_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json recover "$INSTANCE_ID" >"$RECOVER_JSON"

node - "$REPORT" "$DEV_JSON" "$RUNS_JSON" "$LOG_JSON" "$RECOVER_JSON" "$MODEL" <<'NODE'
const fs = require("fs");
const [reportPath, devPath, runsPath, logPath, recoverPath, model] = process.argv.slice(2);
const dev = JSON.parse(fs.readFileSync(devPath, "utf8"));
const runs = JSON.parse(fs.readFileSync(runsPath, "utf8"));
const events = JSON.parse(fs.readFileSync(logPath, "utf8"));
const recover = JSON.parse(fs.readFileSync(recoverPath, "utf8"));
const codexRun = runs.find((run) => run.provider === "codex") || null;
const nativeEvents = events.filter((event) =>
  typeof event.event_type === "string" &&
  event.event_type.startsWith("agent.turn.") &&
  event.payload?.provider === "codex"
);
const nativeEventCounts = nativeEvents.reduce((counts, event) => {
  counts[event.event_type] = (counts[event.event_type] || 0) + 1;
  return counts;
}, {});
const ok = Boolean(
  dev.instance_id &&
  codexRun &&
  codexRun.status === "completed" &&
  recover.recovered_count === 0 &&
  nativeEventCounts["agent.turn.started"] >= 1 &&
  nativeEventCounts["agent.turn.completed"] >= 1
);
fs.writeFileSync(reportPath, `${JSON.stringify({
  ok,
  skipped: false,
  live: true,
  coverage: "live-source-workflow-codex-app-server-native-bridge",
  model,
  instanceId: dev.instance_id || null,
  workerRuns: Array.isArray(dev.workers)
    ? dev.workers.map((worker) => ({
        provider: worker.provider || null,
        ranEffects: worker.ran_effects ?? null,
      }))
    : [],
  codexRun: codexRun
    ? {
        runId: codexRun.run_id,
        status: codexRun.status,
        artifactCount: codexRun.artifact_count,
        nativeStatus: codexRun.native_lifecycle?.status || null,
      }
    : null,
  nativeEventCounts,
  replayAfterRestart: {
    recoveredCount: recover.recovered_count ?? null,
    recoveredEvents: Array.isArray(recover.recovered_events)
      ? recover.recovered_events.map((event) => event.event_type || null)
      : [],
  },
}, null, 2)}\n`);
if (!ok) {
  process.exit(1);
}
NODE

echo "Codex native workflow report wrote $REPORT" >&2
