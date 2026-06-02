#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_REPORT:-$ROOT/target/pi-native-workflow-smoke.json}"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE:-}" != "1" ]]; then
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
  note: "Set WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE=1 to run source workflow -> Pi RPC through RuntimeKernel::run_native_agent_turn.",
}, null, 2)}\n`);
console.error(`Pi native workflow report wrote ${reportPath}`);
NODE
  exit 0
fi

cd "$ROOT"

WORKDIR="$(mktemp -d)"
STORE="$WORKDIR/store.sqlite"
WORKFLOW="$WORKDIR/pi-native-workflow.whip"
DEV_JSON="$WORKDIR/dev.json"
RUNS_JSON="$WORKDIR/runs.json"
LOG_JSON="$WORKDIR/log.json"
RECOVER_JSON="$WORKDIR/recover.json"

cleanup() {
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

cat >"$WORKFLOW" <<'WHIP'
workflow PiNativeWorkflowSmoke

agent worker {
  profile "repo-reader"
  capacity 1
}

rule start_pi
  when started
  when worker is available
=> {
  tell worker "Read-only native workflow smoke. Reply exactly: WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_OK"
}
WHIP

WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS="${WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS:-512}" \
cargo run --quiet -p whipplescript -- \
  --store "$STORE" \
  --json \
  dev "$WORKFLOW" \
  --provider pi \
  --until idle >"$DEV_JSON"

INSTANCE_ID="$(node -e 'const fs=require("fs"); const value=JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(value.instance_id || "");' "$DEV_JSON")"
if [[ -z "$INSTANCE_ID" ]]; then
  echo "Pi native workflow dev output did not include instance_id" >&2
  exit 1
fi

cargo run --quiet -p whipplescript -- --store "$STORE" --json runs "$INSTANCE_ID" >"$RUNS_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json log "$INSTANCE_ID" >"$LOG_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json recover "$INSTANCE_ID" >"$RECOVER_JSON"

node - "$REPORT" "$DEV_JSON" "$RUNS_JSON" "$LOG_JSON" "$RECOVER_JSON" <<'NODE'
const fs = require("fs");
const [reportPath, devPath, runsPath, logPath, recoverPath] = process.argv.slice(2);
const dev = JSON.parse(fs.readFileSync(devPath, "utf8"));
const runs = JSON.parse(fs.readFileSync(runsPath, "utf8"));
const events = JSON.parse(fs.readFileSync(logPath, "utf8"));
const recover = JSON.parse(fs.readFileSync(recoverPath, "utf8"));
const piRun = runs.find((run) => run.provider === "pi") || null;
const nativeEvents = events.filter((event) =>
  typeof event.event_type === "string" &&
  event.event_type.startsWith("agent.turn.") &&
  event.payload?.provider === "pi"
);
const nativeEventCounts = nativeEvents.reduce((counts, event) => {
  counts[event.event_type] = (counts[event.event_type] || 0) + 1;
  return counts;
}, {});
const providerEventTypes = nativeEvents
  .map((event) => event.payload?.provider_event_type)
  .filter(Boolean);
const ok = Boolean(
  dev.instance_id &&
  piRun &&
  piRun.status === "completed" &&
  recover.recovered_count === 0 &&
  nativeEventCounts["agent.turn.started"] >= 1 &&
  nativeEventCounts["agent.turn.completed"] >= 1
);
fs.writeFileSync(reportPath, `${JSON.stringify({
  ok,
  skipped: false,
  live: true,
  coverage: "live-source-workflow-pi-rpc-native-bridge",
  instanceId: dev.instance_id || null,
  workerRuns: Array.isArray(dev.workers)
    ? dev.workers.map((worker) => ({
        provider: worker.provider || null,
        ranEffects: worker.ran_effects ?? null,
      }))
    : [],
  piRun: piRun
    ? {
        runId: piRun.run_id,
        status: piRun.status,
        artifactCount: piRun.artifact_count,
        nativeStatus: piRun.native_lifecycle?.status || null,
      }
    : null,
  nativeEventCounts,
  providerEventTypes,
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

echo "Pi native workflow report wrote $REPORT" >&2
