use serde_json::{json, Value};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn whipplescript() -> Command {
    Command::new(env!("CARGO_BIN_EXE_whip"))
}

fn formal_checks_available() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(
            "(command -v tlc >/dev/null 2>&1 && command -v maude >/dev/null 2>&1) || command -v nix >/dev/null 2>&1",
        )
        .status()
        .expect("tool probe runs")
        .success()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under <repo>/crates/whipplescript-cli")
        .to_path_buf()
}

fn fixture(path: &str) -> PathBuf {
    repo_root().join(path)
}

fn run_json(command: &mut Command) -> Value {
    let output = command.output().expect("command runs");
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is json")
}

fn spec_command() -> (PathBuf, PathBuf, PathBuf) {
    (
        fixture("examples/workflows/spec-implementation.whip"),
        fixture("examples/adapters/spec-implementation.fake-adapter.json"),
        fixture("examples/policies/spec-implementation.enterprise-policy.json"),
    )
}

fn active_count(status: &Value, agent: &str) -> u64 {
    status["active_invocations"]
        .as_array()
        .expect("active invocations array")
        .iter()
        .find(|entry| entry["agent"] == agent)
        .and_then(|entry| entry["count"].as_u64())
        .unwrap_or_default()
}

#[test]
fn e2e_native_harness_runs_command_provider_to_finished_event() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("native-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");

    std::fs::write(
        &workflow,
        r#"
machine NativeHarness
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

data {
  done bool = false
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished as run {
    assign data.done = true
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &config,
        format!(
            r#"{{
  "agents": {{
    "worker": {{
      "provider": "command",
      "command": ["sh", "-c", "printf 'completed %s\n' \"$WHIPPLESCRIPT_PROMPT\""],
      "cwd": "{}",
      "timeoutSeconds": 30
    }}
  }}
}}"#,
            dir.path().display()
        ),
    )
    .expect("config writes");

    let started = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );
    assert_eq!(
        started["status"]["active_invocations"][0]["agent"],
        "worker"
    );

    let harness = run_json(
        whipplescript()
            .arg("harness")
            .arg("once")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&config)
            .arg("--json"),
    );
    assert_eq!(harness["completion_event"]["event_type"], "finished");
    assert_eq!(
        harness["completion_event"]["payload"]["status"],
        "succeeded"
    );

    let finished = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(finished["status"]["current_state"], "done");
    assert_eq!(finished["status"]["data"]["done"], true);
    assert_eq!(
        finished["status"]["active_invocations"],
        Value::Array(vec![])
    );
}

#[test]
fn e2e_harness_profile_policy_resolves_agent_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("profile-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");
    let policy = dir.path().join("harness-policy.json");

    std::fs::write(
        &workflow,
        r#"
machine ProfileHarness
initial waiting

agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 1
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(&config, r#"{"agents":{}}"#).expect("config writes");
    std::fs::write(
        &policy,
        format!(
            r#"{{
  "mode": "custom",
  "defaultProfile": "repo-writer",
  "allowCommandProvider": true,
  "profiles": {{
    "repo-writer": {{
      "description": "Use for implementation work in this test repository.",
      "provider": "command",
      "command": ["sh", "-c", "printf 'profiled %s\n' \"$WHIPPLESCRIPT_PROMPT\""],
      "cwd": "{}",
      "timeoutSeconds": 30,
      "filesystem": "workspace_write",
      "network": "denied",
      "allowedEnv": [],
      "allowedTools": ["read", "edit", "test"],
      "enforcement": "best_effort"
    }}
  }}
}}"#,
            dir.path().display()
        ),
    )
    .expect("policy writes");

    run_json(
        whipplescript()
            .arg("validate")
            .arg(&workflow)
            .arg("--profile-policy")
            .arg(&policy)
            .arg("--json"),
    );

    run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );

    let harness = run_json(
        whipplescript()
            .arg("harness")
            .arg("once")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&config)
            .arg("--profile-policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(
        harness["invocation"]["requested_profile"],
        Value::String("repo-writer".to_string())
    );
    assert_eq!(
        harness["completion_event"]["payload"]["summary"],
        "profiled hello"
    );

    let harness_status = run_json(
        whipplescript()
            .arg("harness")
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--profile-policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(
        harness_status["invocations"][0]["resolved_profile"],
        Value::String("repo-writer".to_string())
    );
    assert!(harness_status["harness_events"]
        .as_array()
        .expect("harness events")
        .iter()
        .any(|event| event["kind"] == "provider_started"
            && event["payload"]["resolvedProfile"] == "repo-writer"));
}

#[test]
fn e2e_harness_run_can_drive_workflow_and_record_timeouts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("drive-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");

    std::fs::write(
        &workflow,
        r#"
machine DriveHarness
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

data {
  finishedStatus string? = nil
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished as run {
    assign data.finishedStatus = run.status
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &config,
        r#"{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "sleep 2"],
      "timeoutSeconds": 1
    }
  }
}"#,
    )
    .expect("config writes");

    run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );

    let harness = run_json(
        whipplescript()
            .arg("harness")
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&config)
            .arg("--drive-workflow")
            .arg("--max-iterations")
            .arg("5")
            .arg("--json"),
    );
    assert_eq!(harness[0]["provider_timed_out"], true);

    let status = run_json(
        whipplescript()
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(status["current_state"], "done");
    assert_eq!(status["data"]["finishedStatus"], "timed_out");

    let harness_status = run_json(
        whipplescript()
            .arg("harness")
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(harness_status["invocations"][0]["status"], "timed_out");
    assert_eq!(harness_status["completions"][0]["status"], "timed_out");
    assert_eq!(harness_status["workflow_status"]["current_state"], "done");
    assert!(harness_status["harness_events"]
        .as_array()
        .expect("harness events")
        .iter()
        .any(|event| event["kind"] == "provider_timed_out"));
    assert!(harness_status["recent_failures"]
        .as_array()
        .expect("recent failures")
        .iter()
        .any(|event| event["kind"] == "provider_timed_out"));
}

#[test]
fn e2e_harness_recovers_expired_leases_and_records_desire_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("lease-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");

    std::fs::write(
        &workflow,
        r#"
machine LeaseHarness
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished {
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &config,
        r#"{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf 'recovered'"],
      "timeoutSeconds": 30
    }
  }
}"#,
    )
    .expect("config writes");

    run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );
    let connection = rusqlite::Connection::open(&store).expect("store opens");
    connection
        .execute(
            r#"
            UPDATE agent_invocations
            SET status = 'claimed', claimed_by = 'dead-worker', claim_expires_at = '0'
            "#,
            [],
        )
        .expect("lease forced expired");

    let harness = run_json(
        whipplescript()
            .arg("harness")
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&config)
            .arg("--drive-workflow")
            .arg("--max-iterations")
            .arg("5")
            .arg("--json"),
    );
    assert_eq!(
        harness[0]["completion_event"]["payload"]["summary"],
        "recovered"
    );

    let harness_status = run_json(
        whipplescript()
            .arg("harness")
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert!(harness_status["harness_events"]
        .as_array()
        .expect("harness events")
        .iter()
        .any(|event| event["kind"] == "lease_expired"));
}

#[test]
fn e2e_harness_records_unknown_agent_and_provider_command_failures() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("desire-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let missing_config = dir.path().join("missing-harness.json");
    let failing_config = dir.path().join("failing-harness.json");

    std::fs::write(
        &workflow,
        r#"
machine DesireHarness
initial waiting

agent worker = codingAgent() {
  maxActive 2
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished {
    stay
  }
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(&missing_config, r#"{"agents":{}}"#).expect("missing config writes");
    std::fs::write(
        &failing_config,
        r#"{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf 'bad command' >&2; exit 7"],
      "timeoutSeconds": 30
    }
  }
}"#,
    )
    .expect("failing config writes");

    run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );
    let output = whipplescript()
        .arg("harness")
        .arg("once")
        .arg(&workflow)
        .arg("--store")
        .arg(&store)
        .arg("--config")
        .arg(&missing_config)
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!output.status.success());

    let missing_status = run_json(
        whipplescript()
            .arg("harness")
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert!(missing_status["recent_failures"]
        .as_array()
        .expect("recent failures")
        .iter()
        .any(|event| event["kind"] == "unknown_agent"));

    run_json(
        whipplescript()
            .arg("harness")
            .arg("once")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&failing_config)
            .arg("--json"),
    );
    let failing_status = run_json(
        whipplescript()
            .arg("harness")
            .arg("status")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert!(failing_status["recent_failures"]
        .as_array()
        .expect("recent failures")
        .iter()
        .any(|event| event["kind"] == "provider_command_failed"));
}

#[test]
fn e2e_harness_records_workflow_validation_failures() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("bad-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");

    std::fs::write(
        &workflow,
        r#"
machine BadHarness
initial missing

state waiting {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(&config, r#"{"agents":{}}"#).expect("config writes");

    let output = whipplescript()
        .arg("harness")
        .arg("run")
        .arg(&workflow)
        .arg("--store")
        .arg(&store)
        .arg("--config")
        .arg(&config)
        .arg("--drive-workflow")
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!output.status.success());

    let connection = rusqlite::Connection::open(&store).expect("store opens");
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM harness_events WHERE kind = 'workflow_validation_failed'",
            [],
            |row| row.get(0),
        )
        .expect("event count reads");
    assert_eq!(count, 1);
}

#[test]
fn e2e_harness_provider_preset_can_use_command_template() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("preset-harness.whip");
    let store = dir.path().join("workflow.sqlite");
    let config = dir.path().join("harness.json");

    std::fs::write(
        &workflow,
        r#"
machine PresetHarness
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}

state waiting {
  on go as evt {
    start worker {
      message evt.message
    }
    stay
  }

  on finished {
    stay
  }
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &config,
        r#"{
  "agents": {
    "worker": {
      "provider": "claude",
      "command": ["sh", "-c", "printf '%s' \"$1\"", "sh", "{{prompt}}"],
      "timeoutSeconds": 30
    }
  }
}"#,
    )
    .expect("config writes");

    run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello preset"}"#)
            .arg("--json"),
    );

    let harness = run_json(
        whipplescript()
            .arg("harness")
            .arg("once")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--config")
            .arg(&config)
            .arg("--json"),
    );
    assert_eq!(
        harness["completion_event"]["payload"]["status"],
        "succeeded"
    );
    assert_eq!(
        harness["completion_event"]["payload"]["summary"],
        "hello preset"
    );
}

#[test]
fn e2e_spec_implementation_runs_idle_worker_quality_loop_to_done() {
    let (workflow, manifest, policy) = spec_command();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("spec.sqlite");
    let build_out = dir.path().join("build");

    let validation = run_json(
        whipplescript()
            .arg("validate")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(validation["ok"], true);

    let build = run_json(
        whipplescript()
            .arg("build")
            .arg(&workflow)
            .arg("--out")
            .arg(&build_out)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(build["workflow_id"], "specImplementation");
    for key in [
        "ir_json",
        "baml_src",
        "tla_model",
        "tla_config",
        "maude_model",
    ] {
        let path = build[key].as_str().expect("artifact path");
        assert!(Path::new(path).exists(), "{path} should exist");
    }
    let baml_src =
        std::fs::read_to_string(build["baml_src"].as_str().unwrap()).expect("generated baml reads");
    assert!(baml_src.contains("function classifyRun"));
    assert!(baml_src.contains("function chooseNextStep"));

    let idle = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("idle")
            .arg("--payload")
            .arg(r#"{"activeRuns":0,"unfinishedItems":2}"#)
            .arg("--fake-call-output")
            .arg(r#"plan.snapshot="W1 is ready""#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"StartWorker","workItemId":"W1","reason":"ready","message":"Implement W1"}"#)
            .arg("--json"),
    );
    assert_eq!(idle["outcome"]["status"], "processed");
    assert_eq!(idle["status"]["current_state"], "watching");
    assert_eq!(active_count(&idle["status"], "worker"), 1);
    assert_eq!(idle["status"]["recent_effects"][0]["effect"], "start");
    assert_eq!(idle["status"]["recent_effects"][0]["status"], "dispatched");

    let worker_done = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"WorkerComplete","workItemId":"W1","reason":"implementation complete"}"#)
            .arg("--json"),
    );
    assert_eq!(worker_done["outcome"]["status"], "processed");
    assert_eq!(worker_done["status"]["current_state"], "watching");
    assert_eq!(active_count(&worker_done["status"], "worker"), 0);
    assert_eq!(active_count(&worker_done["status"], "quality"), 1);
    assert_eq!(
        worker_done["status"]["recent_effects"][0]["target"],
        "quality"
    );

    let duplicate = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--json"),
    );
    assert_eq!(duplicate["outcome"]["status"], "ignored");
    assert_eq!(active_count(&duplicate["status"], "quality"), 1);

    let quality_done = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-quality-1","name":"quality-1","status":"succeeded","stdoutTail":"accepted","stderrTail":"","exitCode":0}"#)
            .arg("--fake-call-output")
            .arg(r#"plan.snapshot="all done""#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"QualityPassed","workItemId":"W1","reason":"accepted"}"#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"Done","workItemId":null,"reason":"complete","message":null}"#)
            .arg("--json"),
    );
    assert_eq!(quality_done["outcome"]["status"], "processed");
    assert_eq!(quality_done["status"]["current_state"], "done");
    assert_eq!(
        quality_done["status"]["active_invocations"]
            .as_array()
            .expect("active invocations")
            .len(),
        0
    );

    let overview = run_json(
        whipplescript()
            .arg("overview")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "done");

    let events = run_json(
        whipplescript()
            .arg("events")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    let statuses = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["status"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"processed"));
    assert!(statuses.contains(&"ignored"));
}

#[test]
fn e2e_runtime_recovers_processing_event_from_disk() {
    let workflow = fixture("examples/workflows/simple-supervisor.whip");
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("recovery.sqlite");

    run_json(
        whipplescript()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"name":"worker-1","runId":"run-1","ok":true}"#)
            .arg("--json"),
    );

    let connection = rusqlite::Connection::open(&store).expect("store opens");
    let event_json: String = connection
        .query_row(
            "SELECT event_json FROM workflow_events WHERE status = 'queued'",
            [],
            |row| row.get(0),
        )
        .expect("queued event reads");
    let mut event: Value = serde_json::from_str(&event_json).expect("event json parses");
    event["status"] = json!("processing");
    event["attempt_count"] = json!(1);
    connection
        .execute(
            "UPDATE workflow_events SET status = 'processing', event_json = ?1",
            [serde_json::to_string(&event).expect("event serializes")],
        )
        .expect("event is stranded as processing");
    drop(connection);

    let run = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "watching");
    assert_eq!(run["status"]["pending_events"], 0);

    let events = run_json(
        whipplescript()
            .arg("events")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["status"], "processed");
    assert_eq!(events["events"][0]["attempt_count"], 2);
}

#[test]
fn e2e_run_can_use_json_plan_file_adapter() {
    let (workflow, manifest, policy) = spec_command();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("plan-adapter.sqlite");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        serde_json::to_string_pretty(&json!({
            "tasks": [
                {"id": "W1", "status": "todo", "title": "Implement W1"}
            ]
        }))
        .expect("plan serializes"),
    )
    .expect("plan writes");

    let idle = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("idle")
            .arg("--payload")
            .arg(r#"{"activeRuns":0,"unfinishedItems":1}"#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"StartWorker","workItemId":"W1","reason":"ready","message":"Implement W1"}"#)
            .arg("--json"),
    );
    assert_eq!(idle["outcome"]["status"], "processed");
    assert_eq!(active_count(&idle["status"], "worker"), 1);

    let worker_done = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"WorkerComplete","workItemId":"W1","reason":"implementation complete"}"#)
            .arg("--json"),
    );
    assert_eq!(worker_done["outcome"]["status"], "processed");
    assert_eq!(active_count(&worker_done["status"], "quality"), 1);

    let updated_plan: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("updated plan reads"))
            .expect("updated plan parses");
    assert_eq!(updated_plan["tasks"][0]["status"], "ready_for_quality");
}

#[test]
fn e2e_plan_file_supplies_builtin_plan_manifest_for_plan_only_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("plan-only.whip");
    let store = dir.path().join("plan-only.sqlite");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &workflow,
        r#"
machine PlanOnly
initial waiting

data {
  snapshot string? = nil
  unfinished int = 0
}

capability plan = adapter("implementationPlan")

event go {}

state waiting {
  on go {
    let text = plan.snapshot()
    let count = plan.unfinishedItems()
    assign data.snapshot = text
    assign data.unfinished = count
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &plan,
        r#"{"tasks":[{"id":"W1","status":"todo"},{"id":"W0","status":"done"}]}"#,
    )
    .expect("plan writes");

    let run = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("go")
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "done");
    assert!(run["status"]["data"]["snapshot"]
        .as_str()
        .expect("snapshot")
        .contains("\"W1\""));
    assert_eq!(run["status"]["data"]["unfinished"], 1);
}

#[test]
fn e2e_review_file_supplies_builtin_human_review_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("review-only.whip");
    let store = dir.path().join("review-only.sqlite");
    let reviews = dir.path().join("reviews.json");
    std::fs::write(
        &workflow,
        r#"
machine ReviewOnly
initial waiting

event go {
  reason string
}

state waiting {
  on go as evt {
    askHuman(evt.reason)
    goto waiting
  }
}
"#,
    )
    .expect("workflow writes");

    let run = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"reason":"needs human decision"}"#)
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["recent_effects"][0]["effect"], "askHuman");
    assert_eq!(run["status"]["recent_effects"][0]["status"], "dispatched");

    let reviews: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(reviews["reviews"][0]["status"], "open");
    assert_eq!(reviews["reviews"][0]["reason"], "needs human decision");
}

#[test]
fn e2e_review_file_supplies_builtin_human_response_event_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("review-response.whip");
    let store = dir.path().join("review-response.sqlite");
    let reviews = dir.path().join("reviews.json");
    std::fs::write(
        &workflow,
        r#"
machine ReviewResponse
initial waiting

state waiting {
  final
}
"#,
    )
    .expect("workflow writes");

    let emitted = run_json(
        whipplescript()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("humanReview.responded")
            .arg("--payload")
            .arg(r#"{"reviewId":"review-1","decision":"approved","response":"ship it"}"#)
            .arg("--json"),
    );

    assert_eq!(emitted["event"]["event_type"], "humanReview.responded");
    assert_eq!(emitted["event"]["payload"]["reviewId"], "review-1");
    assert_eq!(emitted["status"]["pending_events"], 1);
    let recorded_response: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(recorded_response["responses"][0]["reviewId"], "review-1");
    assert_eq!(recorded_response["responses"][0]["decision"], "approved");

    let invalid = whipplescript()
        .arg("emit")
        .arg(&workflow)
        .arg("--store")
        .arg(&store)
        .arg("--review-file")
        .arg(&reviews)
        .arg("--event")
        .arg("humanReview.responded")
        .arg("--payload")
        .arg(r#"{"reviewId":"review-2"}"#)
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!invalid.status.success());
}

#[test]
fn e2e_file_backed_adapters_cover_agent_plan_and_review_loop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("adapter-loop.whip");
    let store = dir.path().join("adapter-loop.sqlite");
    let plan = dir.path().join("plan.json");
    let reviews = dir.path().join("reviews.json");
    std::fs::write(
        &workflow,
        r#"
machine AdapterLoop
initial watching

agent director = thread("director")
agent worker = codingAgent() {
  maxActive 1
}

capability plan = adapter("implementationPlan")

event idle {}

event finished {
  name string
}

state watching {
  on idle {
    let snapshot = plan.snapshot()
    start worker {
      task "W1"
      message snapshot
    }
    askHuman("worker started")
    goto watching
  }

  on finished as run {
    plan.markDone("W1")
    send director run.name
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(&plan, r#"{"tasks":[{"id":"W1","status":"todo"}]}"#).expect("plan writes");

    let first = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("idle")
            .arg("--json"),
    );
    assert_eq!(first["outcome"]["status"], "processed");
    assert_eq!(first["status"]["current_state"], "watching");
    assert_eq!(active_count(&first["status"], "worker"), 1);

    let review_records: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(review_records["reviews"][0]["status"], "open");
    assert_eq!(review_records["reviews"][0]["reason"], "worker started");

    let review_id = review_records["reviews"][0]["id"]
        .as_str()
        .expect("review id")
        .to_string();

    let review_response = run_json(
        whipplescript()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("humanReview.responded")
            .arg("--payload")
            .arg(format!(
                r#"{{"reviewId":"{review_id}","decision":"approved","response":"continue"}}"#
            ))
            .arg("--json"),
    );
    assert_eq!(
        review_response["event"]["event_type"],
        "humanReview.responded"
    );
    let responded_reviews: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(responded_reviews["reviews"][0]["status"], "responded");
    assert_eq!(responded_reviews["reviews"][0]["decision"], "approved");
    assert_eq!(responded_reviews["reviews"][0]["response"], "continue");

    let ignored_review_response = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--json"),
    );
    assert_eq!(ignored_review_response["outcome"]["status"], "ignored");

    let second = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"name":"worker-01"}"#)
            .arg("--json"),
    );
    assert_eq!(second["outcome"]["status"], "processed");
    assert_eq!(second["status"]["current_state"], "done");
    assert_eq!(active_count(&second["status"], "worker"), 0);

    let updated_plan: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("plan file reads"))
            .expect("plan file parses");
    assert_eq!(updated_plan["tasks"][0]["status"], "done");
}

#[test]
fn e2e_formal_check_commands_run_against_real_fixture_when_tools_exist() {
    let (workflow, manifest, policy) = spec_command();

    if !formal_checks_available() {
        eprintln!("skipping formal e2e because formal tools are unavailable");
        return;
    }

    let tla = run_json(
        whipplescript()
            .arg("check")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--target")
            .arg("tla")
            .arg("--json"),
    );
    assert_eq!(tla["ok"], true);
    assert_eq!(tla["target"], "tla");

    let maude = run_json(
        whipplescript()
            .arg("check")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--target")
            .arg("maude")
            .arg("--json"),
    );
    assert_eq!(maude["ok"], true);
    assert_eq!(maude["target"], "maude");
}

#[test]
fn e2e_real_baml_http_coerce_when_enabled() {
    if env::var("WHIPPLESCRIPT_RUN_BAML_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping real BAML e2e because WHIPPLESCRIPT_RUN_BAML_E2E=1 is not set");
        return;
    }

    let baml_url =
        env::var("WHIPPLESCRIPT_BAML_URL").expect("WHIPPLESCRIPT_BAML_URL is required for real BAML e2e");
    let workflow = fixture("examples/workflows/baml-coerce-smoke.whip");
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("baml.sqlite");
    let build_out = dir.path().join("build");

    let build = run_json(
        whipplescript()
            .arg("build")
            .arg(&workflow)
            .arg("--out")
            .arg(&build_out)
            .arg("--json"),
    );
    let baml_src =
        std::fs::read_to_string(build["baml_src"].as_str().unwrap()).expect("generated baml reads");
    assert!(baml_src.contains("function classifyText"));

    let run = run_json(
        whipplescript()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("classify")
            .arg("--payload")
            .arg(r#"{"text":"The workflow completed successfully."}"#)
            .arg("--baml-url")
            .arg(&baml_url)
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "done");
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["function_name"],
        "classifyText"
    );
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["status"],
        "succeeded"
    );
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_http"
    );
    assert!(run["status"]["data"]["reason"].as_str().is_some());
}

#[test]
fn e2e_no_default_runtime_state_is_tracked_in_examples() {
    let default_store = fixture("examples/workflows/.whipplescript/workflows/Minimal.sqlite");

    let ignored = Command::new("git")
        .arg("check-ignore")
        .arg(&default_store)
        .current_dir(repo_root())
        .output()
        .expect("git check-ignore runs");
    assert!(
        ignored.status.success(),
        "default workflow store should be ignored"
    );
}
