#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_REPORT:-$ROOT/target/claude-native-workflow-smoke.json}"
MODEL="${WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL:-sonnet}"

mkdir -p "$(dirname "$REPORT")"

if [[ "${WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE:-}" != "1" ]]; then
  cd "$ROOT"
  WHIPPLESCRIPT_CLAUDE_AGENT_SDK_FAKE=1 \
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
  note: "Set WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1 to run source workflow -> Claude Agent SDK through RuntimeKernel::run_native_agent_turn.",
}, null, 2)}\n`);
console.error(`Claude native workflow report wrote ${reportPath}`);
NODE
  exit 0
fi

cd "$ROOT"

WORKDIR="$(mktemp -d)"
STORE="$WORKDIR/store.sqlite"
WORKFLOW="$WORKDIR/claude-native-workflow.whip"
DEV_JSON="$WORKDIR/dev.json"
RUNS_JSON="$WORKDIR/runs.json"
LOG_JSON="$WORKDIR/log.json"
RECOVER_JSON="$WORKDIR/recover.json"

cleanup() {
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

cat >"$WORKFLOW" <<'WHIP'
workflow ClaudeNativeWorkflowSmoke

agent worker {
  provider claude
  profile "repo-reader"
  capacity 1
}

rule start_claude
  when started
  when worker is available
=> {
  tell worker "Read-only native workflow smoke. Reply exactly: WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_OK"
}
WHIP

WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL="$MODEL" \
WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS="${WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS:-512}" \
cargo run --quiet -p whipplescript -- \
  --store "$STORE" \
  --json \
  dev "$WORKFLOW" \
  --provider claude \
  --until idle >"$DEV_JSON"

INSTANCE_ID="$(node -e 'const fs=require("fs"); const value=JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(value.instance_id || "");' "$DEV_JSON")"
if [[ -z "$INSTANCE_ID" ]]; then
  echo "Claude native workflow dev output did not include instance_id" >&2
  exit 1
fi

cargo run --quiet -p whipplescript -- --store "$STORE" --json runs "$INSTANCE_ID" >"$RUNS_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json log "$INSTANCE_ID" >"$LOG_JSON"
cargo run --quiet -p whipplescript -- --store "$STORE" --json recover "$INSTANCE_ID" >"$RECOVER_JSON"

node - "$REPORT" "$DEV_JSON" "$RUNS_JSON" "$LOG_JSON" "$RECOVER_JSON" "$MODEL" <<'NODE'
const fs = require("fs");
const { spawnSync } = require("child_process");
const [reportPath, devPath, runsPath, logPath, recoverPath, model] = process.argv.slice(2);
const dev = JSON.parse(fs.readFileSync(devPath, "utf8"));
const runs = JSON.parse(fs.readFileSync(runsPath, "utf8"));
const events = JSON.parse(fs.readFileSync(logPath, "utf8"));
const recover = JSON.parse(fs.readFileSync(recoverPath, "utf8"));
const claudeRun = runs.find((run) => run.provider === "claude") || null;
const nativeEvents = events.filter((event) =>
  typeof event.event_type === "string" &&
  event.event_type.startsWith("agent.turn.") &&
  event.payload?.provider === "claude"
);
const nativeEventCounts = nativeEvents.reduce((counts, event) => {
  counts[event.event_type] = (counts[event.event_type] || 0) + 1;
  return counts;
}, {});

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

const ok = Boolean(
  dev.instance_id &&
  claudeRun &&
  claudeRun.status === "completed" &&
  recover.recovered_count === 0 &&
  nativeEventCounts["agent.turn.started"] >= 1 &&
  nativeEventCounts["agent.turn.completed"] >= 1
);
fs.writeFileSync(reportPath, `${JSON.stringify({
  ok,
  skipped: false,
  live: true,
  coverage: "live-source-workflow-claude-agent-sdk-native-bridge",
  model,
  authPosture: {
    localClaude: localClaudeAuthPosture(),
    apiKeySet: Boolean(process.env.ANTHROPIC_API_KEY),
  },
  instanceId: dev.instance_id || null,
  workerRuns: Array.isArray(dev.workers)
    ? dev.workers.map((worker) => ({
        provider: worker.provider || null,
        ranEffects: worker.ran_effects ?? null,
      }))
    : [],
  claudeRun: claudeRun
    ? {
        runId: claudeRun.run_id,
        status: claudeRun.status,
        artifactCount: claudeRun.artifact_count,
        nativeStatus: claudeRun.native_lifecycle?.status || null,
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

echo "Claude native workflow report wrote $REPORT" >&2
